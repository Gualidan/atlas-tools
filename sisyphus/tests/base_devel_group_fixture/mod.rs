#![allow(dead_code)]

//! Test fixture for the sandbox `/usr` assembly tests, trimmed and ported
//! from `crates/sandbox-install/tests/fixtures/mod.rs`. It builds signed
//! `.sky` packages beside a temporary repository database (at
//! `<repo_db.parent()>/<arch>/<name>-<version>-<release>.sky`, the layout
//! `install_group_into_root` derives), seeds the repo DB the way Sisyphus
//! publishes rows, and points `load_config` at a test-specific config
//! directory via `SISYPHUS_CONFIG_DIR`.

use std::{
    env,
    ffi::OsString,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use anyhow::Result;
use common::{
    db::db::create_packages_table,
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

/// A repository seed row: `(name, version, release, sky_path, deps, groups)`.
type SeedRow<'a> = (
    &'a str,
    &'a str,
    u32,
    &'a str,
    &'a [&'a str],
    &'a [&'a str],
);

/// Self-contained per-test environment. The process-wide lock serializes
/// tests because `SISYPHUS_CONFIG_DIR` is a global; each test holds it for
/// its duration.
pub struct Fixture {
    tmp: TempDir,
    keypair: Ed25519KeyPair,
    previous_config_dir: Option<OsString>,
    _lock: MutexGuard<'static, ()>,
}

impl Fixture {
    pub fn new() -> Result<Self> {
        static LOCK: Mutex<()> = Mutex::new(());
        let lock = LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let tmp = TempBuilder::new()
            .prefix("base_devel_group_test_")
            .tempdir()?;
        let (keypair, public_key) = generate_keypair()?;

        let config_dir = tmp.path().join("sisyphus-config");
        fs::create_dir_all(&config_dir)?;
        let pub_key_path = config_dir.join("pub.key");
        fs::write(&pub_key_path, &public_key)?;
        fs::write(
            config_dir.join("config.yaml"),
            format!(
                "priv_key_path: {}\npub_key_path: {}\n",
                config_dir.join("unused.key").display(),
                pub_key_path.display()
            ),
        )?;

        let previous_config_dir = env::var_os("SISYPHUS_CONFIG_DIR");
        unsafe { env::set_var("SISYPHUS_CONFIG_DIR", &config_dir) };

        Ok(Self {
            tmp,
            keypair,
            previous_config_dir,
            _lock: lock,
        })
    }

    /// The repository database path. `install_group_into_root` derives the
    /// sky repository as `repo_db.parent()`, which is `tmp/repo` here.
    pub fn repo_db(&self) -> PathBuf {
        self.tmp.path().join("repo/repo.db")
    }

    /// Publishes a one-file package (`<name>/<version>.txt` containing
    /// `<name> <version>\n`) whose repository row belongs to `groups`.
    /// The `.sky` lands at `<repo_db.parent()>/<arch>/<name>-<version>-<release>.sky`.
    pub fn publish_in_groups(
        &self,
        name: &str,
        version: &str,
        release: u32,
        deps: &[&str],
        groups: &[&str],
    ) -> Result<PathBuf> {
        let sky = write_signed_sky_with_file(
            &self.tmp,
            &self.keypair,
            &format!("{name}-{version}-{release}.sky"),
            name,
            version,
            release,
            format!("{name} {version}\n").as_bytes(),
            deps.iter().map(ToString::to_string).collect(),
            name,
            &format!("{version}.txt"),
        )?;

        // Relocate the `.sky` into the arch directory of the repo layout
        // that `install_group_into_root` resolves against.
        let repo_dir = self.repo_db().parent().unwrap().join(ARCH.to_string());
        fs::create_dir_all(&repo_dir)?;
        let sky_path = repo_dir.join(sky.file_name().unwrap());
        fs::rename(&sky, &sky_path)?;

        self.seed_repo_db(&[(
            name,
            version,
            release,
            sky_path.to_str().unwrap(),
            deps,
            groups,
        )]);
        Ok(sky_path)
    }

    /// Seeds repository rows with runtime dependency and group lists exactly
    /// the way Sisyphus publishes them: serialized with `serde_saphyr`, never
    /// hand-written YAML.
    fn seed_repo_db(&self, packages: &[SeedRow<'_>]) {
        let mut conn = Connection::open(self.repo_db()).unwrap();
        create_packages_table(&mut conn).unwrap();

        for (name, version, release, sky_path, deps, groups) in packages {
            let deps: Vec<String> = deps.iter().map(ToString::to_string).collect();
            let deps = serde_saphyr::to_string(&deps).expect("failed to serialize deps");
            let groups: Vec<String> = groups.iter().map(ToString::to_string).collect();
            let groups = serde_saphyr::to_string(&groups).expect("failed to serialize groups");

            conn.execute(
                "INSERT OR REPLACE INTO packages (name, version, release, architecture, sky_path, recipe_sha256, built_at, builder, deps, makedeps, groups) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    name,
                    version,
                    release,
                    ARCH.to_string(),
                    sky_path,
                    "deadbeef",
                    "2026-01-01T00:00:00Z",
                    "tester",
                    &deps,
                    "[]",
                    &groups
                ],
            )
            .unwrap();

            // Read back the arrange so a wrong column order or architecture
            // cannot turn a group test into a vacuous assertion about an
            // empty plan.
            let (stored_deps, stored_groups): (String, String) = conn
                .query_row(
                    "SELECT deps, groups FROM packages WHERE name = ? AND architecture = ?",
                    params![name, ARCH.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(stored_deps, deps, "{name} repository deps");
            assert_eq!(stored_groups, groups, "{name} repository groups");
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        match self.previous_config_dir.take() {
            Some(value) => unsafe { env::set_var("SISYPHUS_CONFIG_DIR", value) },
            None => unsafe { env::remove_var("SISYPHUS_CONFIG_DIR") },
        }
    }
}

pub fn generate_keypair() -> Result<(Ed25519KeyPair, Vec<u8>)> {
    let rng = &ring::rand::SystemRandom::new();
    let doc = Ed25519KeyPair::generate_pkcs8(rng)?;
    let keypair = Ed25519KeyPair::from_pkcs8(doc.as_ref())?;
    let pub_key = keypair.public_key().as_ref().to_vec();
    Ok((keypair, pub_key))
}

pub fn metadata_for(name: &str, version: &str, release: u32, deps: Vec<String>) -> Metadata {
    Metadata {
        format: 1,
        name: name.to_string(),
        version: version.to_string(),
        release,
        architecture: ARCH,
        deps,
        source: Source {
            url: "https://example.invalid/src.tar.gz".to_string(),
            sha256: "unused".to_string(),
        },
        build: Build {
            recipe_sha256: "deadbeef".to_string(),
            built_at: "2026-01-01T00:00:00Z".to_string(),
            builder: "tester".to_string(),
        },
    }
}

/// Builds a payload containing `<dir>/` and `<dir>/<file_name>`.
pub fn build_payload_with_file(dir: &str, file_name: &str, contents: &[u8]) -> Result<Vec<u8>> {
    let mut payload = Vec::new();
    let encoder = Encoder::new(&mut payload, 3)?;
    let mut builder = Builder::new(encoder);

    let mut dir_header = Header::new_gnu();
    dir_header.set_mode(0o755);
    dir_header.set_size(0);
    dir_header.set_entry_type(EntryType::dir());
    builder.append_data(&mut dir_header, dir, std::io::empty())?;

    let file_path = format!("{dir}/{file_name}");
    let mut file_header = Header::new_gnu();
    file_header.set_mode(0o644);
    file_header.set_size(contents.len() as u64);
    builder.append_data(&mut file_header, file_path, contents)?;

    builder.into_inner()?.finish()?;
    Ok(payload)
}

/// Builds the manifest matching [`build_payload_with_file`].
pub fn manifest_for_file(dir: &str, file_name: &str, contents: &[u8]) -> Result<String> {
    let file_hash = hex::encode(hash(Cursor::new(contents))?);
    Ok(format!(
        "f\t0644\t{file_hash}\t{dir}/{file_name}\nd\t0755\t-\t{dir}\n"
    ))
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

#[allow(clippy::too_many_arguments)]
pub fn write_signed_sky_with_file(
    tmp: &TempDir,
    keypair: &Ed25519KeyPair,
    sky_name: &str,
    package_name: &str,
    version: &str,
    release: u32,
    contents: &[u8],
    deps: Vec<String>,
    dir: &str,
    file_name: &str,
) -> Result<PathBuf> {
    let payload = build_payload_with_file(dir, file_name, contents)?;
    let files = manifest_for_file(dir, file_name, contents)?;
    let metadata_bytes =
        serde_saphyr::to_string(&metadata_for(package_name, version, release, deps))?.into_bytes();
    let data = data_to_sign(&metadata_bytes, &payload, files.as_bytes())?;
    let signature = keypair.sign(&data).as_ref().to_vec();

    let sky_path = tmp.path().join(sky_name);
    write_sky(&sky_path, &payload, &metadata_bytes, &files, &signature)?;
    Ok(sky_path)
}
