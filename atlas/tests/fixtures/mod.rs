#![allow(dead_code)]

use std::{
    env,
    ffi::OsString,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use anyhow::Result;
use atlas::types::runtime_config::RuntimeConfig;
use common::{
    functions::{hash::hash, sky_verify::data_to_sign},
    types::{
        metadata::{Build, Metadata},
        package::{Architectures, Source},
    },
};
use ring::signature::{Ed25519KeyPair, KeyPair};
use rusqlite::{Connection, params};
use tar::{Builder, EntryType, Header};
use tempfile::{Builder as TempBuilder, TempDir};
use zstd::Encoder;

pub const ARCH: Architectures = Architectures::X86_64;

/// Builds a temporary directory on the same filesystem as the install staging
/// directory (`/var/tmp`), so that the atomic `renameat_with` exchange used by
/// the install state machine works.
pub fn tempdir() -> TempDir {
    TempBuilder::new()
        .prefix("atlas_test_")
        .tempdir_in("/var/tmp")
        .expect("failed to create test temporary directory")
}

pub fn runtime_config(tmp: &TempDir) -> Result<RuntimeConfig> {
    RuntimeConfig::build_at(&tmp.path().to_path_buf())
}

pub fn seed_repo_db(path: &Path, packages: &[(&str, &str, u32, &str)]) {
    let conn = Connection::open(path).unwrap();
    conn.execute(
        "CREATE TABLE IF NOT EXISTS packages (name TEXT NOT NULL, version TEXT NOT NULL, release INTEGER NOT NULL, architecture TEXT NOT NULL, sky_path TEXT PRIMARY KEY, recipe_sha256 TEXT NOT NULL, built_at TEXT NOT NULL, builder TEXT NOT NULL, deps TEXT NOT NULL, makedeps TEXT NOT NULL)",
        [],
    )
    .unwrap();

    for (name, version, release, sky_path) in packages {
        conn.execute(
            "INSERT OR REPLACE INTO packages (name, version, release, architecture, sky_path, recipe_sha256, built_at, builder, deps, makedeps) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                *name,
                *version,
                *release,
                ARCH.to_string(),
                *sky_path,
                "deadbeef",
                "2026-01-01T00:00:00Z",
                "tester",
                "[]",
                "[]"
            ],
        )
        .unwrap();
    }
}

pub fn raw_header(name: &str, mode: u32, size: u64) -> Header {
    let mut header = Header::new_gnu();
    header.set_mode(mode);
    header.set_size(size);
    header.set_entry_type(EntryType::file());
    let bytes = name.as_bytes();
    header.as_old_mut().name[..bytes.len()].copy_from_slice(bytes);
    header.set_cksum();
    header
}

pub fn write_sky(
    path: &Path,
    payload: &[u8],
    metadata_bytes: &[u8],
    files: &str,
    signature: &[u8],
) -> Result<()> {
    let file = std::fs::File::create(path)?;
    let encoder = Encoder::new(file, 3)?;
    let mut builder = Builder::new(encoder);

    let mut header = Header::new_gnu();
    header.set_mode(0o777);
    header.set_size(metadata_bytes.len() as u64);
    builder.append_data(&mut header, "metadata.yaml", metadata_bytes)?;
    header.set_size(payload.len() as u64);
    builder.append_data(&mut header, "payload.tar.zst", payload)?;
    header.set_size(files.len() as u64);
    builder.append_data(&mut header, "files", files.as_bytes())?;
    header.set_size(signature.len() as u64);
    builder.append_data(&mut header, "signature.ed25519", signature)?;

    builder.into_inner()?.finish()?;
    Ok(())
}

pub fn generate_keypair() -> Result<(Ed25519KeyPair, Vec<u8>)> {
    let rng = &ring::rand::SystemRandom::new();
    let doc = Ed25519KeyPair::generate_pkcs8(rng)?;
    let keypair = Ed25519KeyPair::from_pkcs8(doc.as_ref())?;
    let pub_key = keypair.public_key().as_ref().to_vec();
    Ok((keypair, pub_key))
}

pub fn metadata_for(name: &str, version: &str, release: u32) -> Metadata {
    Metadata {
        format: 1,
        name: name.to_string(),
        version: version.to_string(),
        release,
        architecture: ARCH,
        deps: vec![],
        source: Source {
            url: "https://example.invalid/src.tar.gz".to_string(),
            sha256: None,
        },
        build: Build {
            recipe_sha256: "deadbeef".to_string(),
            built_at: "2026-01-01T00:00:00Z".to_string(),
            builder: "tester".to_string(),
        },
    }
}

/// Builds a payload containing `dir/` and `dir/file.txt`.
pub fn build_payload(contents: &[u8]) -> Result<Vec<u8>> {
    let mut payload = Vec::new();
    let encoder = Encoder::new(&mut payload, 3)?;
    let mut builder = Builder::new(encoder);

    let mut dir_header = Header::new_gnu();
    dir_header.set_mode(0o755);
    dir_header.set_size(0);
    dir_header.set_entry_type(EntryType::dir());
    builder.append_data(&mut dir_header, "dir", std::io::empty())?;

    let mut file_header = Header::new_gnu();
    file_header.set_mode(0o644);
    file_header.set_size(contents.len() as u64);
    builder.append_data(&mut file_header, "dir/file.txt", contents)?;

    builder.into_inner()?.finish()?;
    Ok(payload)
}

/// Builds the manifest matching [`build_payload`].
pub fn manifest_for(contents: &[u8]) -> Result<String> {
    let file_hash = hex::encode(hash(Cursor::new(contents))?);
    Ok(format!(
        "f\t0644\t{}\tdir/file.txt\nd\t0755\t-\tdir\n",
        file_hash
    ))
}

pub fn write_signed_sky(
    tmp: &TempDir,
    keypair: &Ed25519KeyPair,
    sky_name: &str,
    package_name: &str,
    version: &str,
    release: u32,
    contents: &[u8],
) -> Result<PathBuf> {
    let payload = build_payload(contents)?;
    let files = manifest_for(contents)?;
    let metadata_bytes =
        serde_saphyr::to_string(&metadata_for(package_name, version, release))?.into_bytes();
    let data = data_to_sign(&metadata_bytes, payload.clone(), files.as_bytes())?;
    let signature = keypair.sign(&data).as_ref().to_vec();

    let sky_path = tmp.path().join(sky_name);
    write_sky(&sky_path, &payload, &metadata_bytes, &files, &signature)?;
    Ok(sky_path)
}

/// Holds the process-wide environment lock while pointing `load_config` at a
/// test-specific directory.
pub struct ConfigEnv {
    _guard: MutexGuard<'static, ()>,
    previous: Option<OsString>,
}

impl ConfigEnv {
    pub fn new(tmp: &TempDir, pub_key: &[u8]) -> Result<Self> {
        static LOCK: Mutex<()> = Mutex::new(());
        let guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

        let config_dir = tmp.path().join("sisyphus-config");
        std::fs::create_dir_all(&config_dir)?;
        let pub_key_path = config_dir.join("pub.key");
        std::fs::write(&pub_key_path, pub_key)?;
        let priv_key_path = config_dir.join("unused.key");
        std::fs::write(
            config_dir.join("config.yaml"),
            format!(
                "priv_key_path: {}\npub_key_path: {}\n",
                priv_key_path.display(),
                pub_key_path.display()
            ),
        )?;

        let previous = env::var_os("SISYPHUS_CONFIG_DIR");
        unsafe { env::set_var("SISYPHUS_CONFIG_DIR", &config_dir) };

        Ok(Self {
            _guard: guard,
            previous,
        })
    }
}

impl Drop for ConfigEnv {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => unsafe { env::set_var("SISYPHUS_CONFIG_DIR", value) },
            None => unsafe { env::remove_var("SISYPHUS_CONFIG_DIR") },
        }
    }
}
