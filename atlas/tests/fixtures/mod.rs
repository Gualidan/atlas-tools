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

pub fn count(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> i64 {
    conn.query_row(sql, params, |row| row.get(0)).unwrap()
}

pub fn seed_repo_db(path: &Path, packages: &[(&str, &str, u32, &str)]) {
    let packages_without_deps: Vec<(&str, &str, u32, &str, &[&str])> = packages
        .iter()
        .map(|(name, version, release, sky_path)| (*name, *version, *release, *sky_path, &[][..]))
        .collect();

    seed_repo_db_with_deps(path, &packages_without_deps);
}

/// Seeds the repository database with runtime dependency lists exactly the way
/// Sisyphus publishes them: the `deps` column is serialized with
/// `serde_saphyr`, never hand-written YAML.
pub fn seed_repo_db_with_deps(path: &Path, packages: &[(&str, &str, u32, &str, &[&str])]) {
    let mut conn = Connection::open(path).unwrap();
    create_packages_table(&mut conn).unwrap();

    for (name, version, release, sky_path, deps) in packages {
        let deps: Vec<String> = deps.iter().map(|dep| dep.to_string()).collect();
        let deps = serde_saphyr::to_string(&deps).expect("failed to serialize deps to YAML");

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
                deps,
                "[]"
            ],
        )
        .unwrap();
    }
}

pub fn raw_header(name: &str, mode: u32, size: u64) -> Header {
    raw_entry_header(name, mode, size, EntryType::file())
}

pub fn raw_entry_header(name: &str, mode: u32, size: u64, entry_type: EntryType) -> Header {
    let mut header = Header::new_gnu();
    header.set_mode(mode);
    header.set_size(size);
    header.set_entry_type(entry_type);
    let bytes = name.as_bytes();
    header.as_old_mut().name[..bytes.len()].copy_from_slice(bytes);
    header.set_cksum();
    header
}

/// Builds a zstd'd tar payload containing exactly one entry whose name, mode
/// and type are all copied into the raw header, bypassing tar's path
/// sanitization. This is what lets evil paths (`/etc/evil`, `dir/../..`) and
/// forbidden entry types into a payload at all.
pub fn build_payload_with_raw_entry(
    path: &str,
    mode: u32,
    entry_type: EntryType,
    contents: &[u8],
) -> Result<Vec<u8>> {
    let mut payload = Vec::new();
    let encoder = Encoder::new(&mut payload, 3)?;
    let mut builder = Builder::new(encoder);

    let header = raw_entry_header(path, mode, contents.len() as u64, entry_type);
    builder.append(&header, contents)?;
    builder.append_data(
        &mut raw_entry_header("dir", 0o755, 0, EntryType::dir()),
        "dir",
        std::io::empty(),
    )?;

    builder.into_inner()?.finish()?;
    Ok(payload)
}

/// Like [`write_signed_sky`] but the payload holds one fully-controlled raw
/// entry instead of the sanitized `dir/file.txt` pair. The manifest is kept to
/// a single uniform `dir` line: the payload gates in `NotInstalled::verify`
/// fire before manifest parsing, so its content is unconstrained, and every
/// sky stays valid-if-ever-parsed while each test commits exactly one crime.
#[allow(clippy::too_many_arguments)]
pub fn write_signed_sky_with_raw_entry(
    tmp: &TempDir,
    keypair: &Ed25519KeyPair,
    sky_name: &str,
    package_name: &str,
    version: &str,
    release: u32,
    path: &str,
    mode: u32,
    entry_type: EntryType,
    contents: &[u8],
) -> Result<PathBuf> {
    let payload = build_payload_with_raw_entry(path, mode, entry_type, contents)?;
    let files = "d\t0755\t-\tdir\n".to_string();
    let metadata_bytes =
        serde_saphyr::to_string(&metadata_for(package_name, version, release, vec![]))?
            .into_bytes();
    let data = data_to_sign(&metadata_bytes, payload.clone(), files.as_bytes())?;
    let signature = keypair.sign(&data).as_ref().to_vec();

    let sky_path = tmp.path().join(sky_name);
    write_sky(&sky_path, &payload, &metadata_bytes, &files, &signature)?;
    Ok(sky_path)
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

/// Builds a payload containing `dir/` and `dir/file.txt`.
pub fn build_payload(contents: &[u8]) -> Result<Vec<u8>> {
    build_payload_in_dir("dir", contents)
}

/// Builds a payload containing `<dir>/` and `<dir>/file.txt`.
pub fn build_payload_in_dir(dir: &str, contents: &[u8]) -> Result<Vec<u8>> {
    build_payload_with_file(dir, "file.txt", contents)
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

/// Builds a payload containing `<dir>/` and the symlink `<dir>/<link_name>`
/// pointing at `target`.
pub fn build_payload_with_symlink(dir: &str, link_name: &str, target: &str) -> Result<Vec<u8>> {
    let mut payload = Vec::new();
    let encoder = Encoder::new(&mut payload, 3)?;
    let mut builder = Builder::new(encoder);

    let mut dir_header = Header::new_gnu();
    dir_header.set_mode(0o755);
    dir_header.set_size(0);
    dir_header.set_entry_type(EntryType::dir());
    builder.append_data(&mut dir_header, dir, std::io::empty())?;

    let link_path = format!("{dir}/{link_name}");
    let mut link_header = Header::new_gnu();
    link_header.set_mode(0o777);
    link_header.set_size(0);
    link_header.set_entry_type(EntryType::symlink());
    builder.append_link(&mut link_header, &link_path, target)?;

    builder.into_inner()?.finish()?;
    Ok(payload)
}

/// Builds the manifest matching [`build_payload`].
pub fn manifest_for(contents: &[u8]) -> Result<String> {
    manifest_for_dir("dir", contents)
}

/// Builds the manifest matching [`build_payload_in_dir`].
pub fn manifest_for_dir(dir: &str, contents: &[u8]) -> Result<String> {
    manifest_for_file(dir, "file.txt", contents)
}

/// Builds the manifest matching [`build_payload_with_file`].
pub fn manifest_for_file(dir: &str, file_name: &str, contents: &[u8]) -> Result<String> {
    let file_hash = hex::encode(hash(Cursor::new(contents))?);
    Ok(format!(
        "f\t0644\t{}\t{dir}/{file_name}\nd\t0755\t-\t{dir}\n",
        file_hash
    ))
}

/// Builds the manifest matching [`build_payload_with_symlink`].
pub fn manifest_for_symlink(dir: &str, link_name: &str, target: &str) -> Result<String> {
    Ok(format!(
        "l\t0777\t{target}\t{dir}/{link_name}\nd\t0755\t-\t{dir}\n"
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
    deps: Vec<String>,
) -> Result<PathBuf> {
    let payload = build_payload(contents)?;
    let files = manifest_for(contents)?;
    let metadata_bytes =
        serde_saphyr::to_string(&metadata_for(package_name, version, release, deps))?.into_bytes();
    let data = data_to_sign(&metadata_bytes, payload.clone(), files.as_bytes())?;
    let signature = keypair.sign(&data).as_ref().to_vec();

    let sky_path = tmp.path().join(sky_name);
    write_sky(&sky_path, &payload, &metadata_bytes, &files, &signature)?;
    Ok(sky_path)
}

/// Like [`write_signed_sky`] but with a custom payload directory name, so two
/// packages installed together don't conflict on the same paths.
pub fn write_signed_sky_in_dir(
    tmp: &TempDir,
    keypair: &Ed25519KeyPair,
    sky_name: &str,
    package_name: &str,
    version: &str,
    release: u32,
    contents: &[u8],
    deps: Vec<String>,
    dir: &str,
) -> Result<PathBuf> {
    let payload = build_payload_in_dir(dir, contents)?;
    let files = manifest_for_dir(dir, contents)?;
    let metadata_bytes =
        serde_saphyr::to_string(&metadata_for(package_name, version, release, deps))?.into_bytes();
    let data = data_to_sign(&metadata_bytes, payload.clone(), files.as_bytes())?;
    let signature = keypair.sign(&data).as_ref().to_vec();

    let sky_path = tmp.path().join(sky_name);
    write_sky(&sky_path, &payload, &metadata_bytes, &files, &signature)?;
    Ok(sky_path)
}

/// Like [`write_signed_sky`] but with a custom payload directory and file
/// name, so two packages can share a directory without colliding on a file.
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
    let data = data_to_sign(&metadata_bytes, payload.clone(), files.as_bytes())?;
    let signature = keypair.sign(&data).as_ref().to_vec();

    let sky_path = tmp.path().join(sky_name);
    write_sky(&sky_path, &payload, &metadata_bytes, &files, &signature)?;
    Ok(sky_path)
}

/// Like [`write_signed_sky`] but shipping a single symlink instead of a
/// regular file.
pub fn write_signed_sky_with_symlink(
    tmp: &TempDir,
    keypair: &Ed25519KeyPair,
    sky_name: &str,
    package_name: &str,
    version: &str,
    release: u32,
    link_name: &str,
    target: &str,
    deps: Vec<String>,
    dir: &str,
) -> Result<PathBuf> {
    let payload = build_payload_with_symlink(dir, link_name, target)?;
    let files = manifest_for_symlink(dir, link_name, target)?;
    let metadata_bytes =
        serde_saphyr::to_string(&metadata_for(package_name, version, release, deps))?.into_bytes();
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
