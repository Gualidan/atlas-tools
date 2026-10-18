use std::{
    io::Cursor,
    path::{Path, PathBuf},
};

use anyhow::Result;
use atlas::cli::cli::VerifyArgs;
use atlas::cli::verify::verify;
use atlas::types::runtime_config::RuntimeConfig;
use common::functions::{hash::hash, sky_verify::data_to_sign};
use common::types::{
    config::SisyphusConfig,
    metadata::{Build, Metadata},
    package::{Architectures, Source},
};
use ring::signature::{Ed25519KeyPair, KeyPair};
use rusqlite::{Connection, params};
use tar::{Builder, EntryType, Header};
use tempfile::TempDir;
use zstd::Encoder;

const ARCH: Architectures = Architectures::X86_64;

fn runtime_config(tmp: &TempDir) -> Result<RuntimeConfig> {
    RuntimeConfig::build_at(&tmp.path().to_path_buf())
}

fn seed_repo_db(path: &Path, packages: &[(&str, &str, u32, &str)]) {
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

fn build_payload(contents: &[u8], file_mode: u32) -> Result<Vec<u8>> {
    let mut payload = Vec::new();
    let encoder = Encoder::new(&mut payload, 3)?;
    let mut builder = Builder::new(encoder);

    let mut dir_header = Header::new_gnu();
    dir_header.set_mode(0o755);
    dir_header.set_size(0);
    dir_header.set_entry_type(EntryType::dir());
    builder.append_data(&mut dir_header, "dir", std::io::empty())?;

    let mut file_header = Header::new_gnu();
    file_header.set_mode(file_mode);
    file_header.set_size(contents.len() as u64);
    builder.append_data(&mut file_header, "dir/file.txt", contents)?;

    let mut lnk_header = Header::new_gnu();
    lnk_header.set_mode(0o777);
    lnk_header.set_size(0);
    lnk_header.set_entry_type(EntryType::symlink());
    builder.append_link(&mut lnk_header, "dir/lnk", "file.txt")?;

    builder.into_inner()?.finish()?;
    Ok(payload)
}

fn write_sky(
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

fn tamper_payload(payload: &[u8]) -> Vec<u8> {
    let mut tampered = payload.to_vec();
    let middle = tampered.len() / 2;
    tampered[middle] ^= 0xFF;
    tampered
}

fn generate_keypair() -> Result<(Ed25519KeyPair, Vec<u8>)> {
    let rng = &ring::rand::SystemRandom::new();
    let doc = Ed25519KeyPair::generate_pkcs8(rng)?;
    let keypair = Ed25519KeyPair::from_pkcs8(doc.as_ref())?;
    let pub_key = keypair.public_key().as_ref().to_vec();
    Ok((keypair, pub_key))
}

fn test_metadata() -> Metadata {
    Metadata {
        format: 1,
        name: "hello".to_string(),
        version: "1.0.0".to_string(),
        release: 1,
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

fn write_signed_sky(
    payload: &[u8],
    files: &str,
    tmp: &TempDir,
    name: &str,
) -> Result<(PathBuf, PathBuf)> {
    let metadata_bytes = serde_saphyr::to_string(&test_metadata())?.into_bytes();
    let (keypair, pub_key) = generate_keypair()?;
    let data = data_to_sign(&metadata_bytes, payload.to_vec(), files.as_bytes())?;
    let signature = keypair.sign(&data).as_ref().to_vec();

    let sky_path = tmp.path().join(name);
    write_sky(&sky_path, payload, &metadata_bytes, files, &signature)?;
    let pub_key_path = tmp.path().join("hello.pub");
    std::fs::write(&pub_key_path, &pub_key)?;
    Ok((sky_path, pub_key_path))
}

fn setup() -> Result<(TempDir, RuntimeConfig, SisyphusConfig, PathBuf, PathBuf)> {
    let tmp = TempDir::new().unwrap();
    let config = runtime_config(&tmp)?;

    let (keypair, pub_key) = generate_keypair()?;

    let contents = b"hello, world!\n";
    let payload = build_payload(contents, 0o644)?;
    let tampered_payload = tamper_payload(&payload);

    let file_hash = hex::encode(hash(Cursor::new(contents))?);
    let files = format!(
        "f\t0644\t{}\tdir/file.txt\nd\t0755\t-\tdir\nl\t0777\tfile.txt\tdir/lnk\n",
        file_hash
    );
    let metadata_bytes = serde_saphyr::to_string(&test_metadata())?.into_bytes();

    let data = data_to_sign(&metadata_bytes, payload.clone(), files.as_bytes())?;
    let signature = keypair.sign(&data).as_ref().to_vec();

    let sky_path = tmp.path().join("hello.sky");
    write_sky(&sky_path, &payload, &metadata_bytes, &files, &signature)?;

    let tampered_path = tmp.path().join("tampered.sky");
    write_sky(
        &tampered_path,
        &tampered_payload,
        &metadata_bytes,
        &files,
        &signature,
    )?;

    let settings = SisyphusConfig {
        priv_key_path: tmp.path().join("unused.key"),
        pub_key_path: tmp.path().join("hello.pub"),
    };
    std::fs::write(&settings.pub_key_path, &pub_key)?;

    Ok((tmp, config, settings, sky_path, tampered_path))
}

fn assert_chain(result: &Result<()>, expected: &str) {
    let Err(err) = result else {
        panic!("expected verification failure containing `{expected}`, got Ok(())");
    };
    let chain = format!("{err:#}");
    assert!(chain.contains(expected), "error chain was: {chain}");
}

fn mode1_args(package: &str) -> VerifyArgs {
    VerifyArgs {
        package: Some(package.into()),
        sky_path: None,
    }
}

fn mode2_args(sky_path: &Path) -> VerifyArgs {
    VerifyArgs {
        package: None,
        sky_path: Some(sky_path.to_path_buf()),
    }
}

#[test]
fn mode1_happy_path() -> Result<()> {
    let (_tmp, config, settings, sky_path, _tampered_path) = setup()?;
    seed_repo_db(
        &config.repo_db,
        &[("hello", "1.0.0", 1, sky_path.to_str().unwrap())],
    );

    let result = verify(mode1_args("hello"), &config, settings);
    assert!(
        result.is_ok(),
        "unexpected verification failure: {result:?}"
    );
    Ok(())
}

#[test]
fn mode1_metadata_mismatch() -> Result<()> {
    let (_tmp, config, settings, sky_path, _tampered_path) = setup()?;
    seed_repo_db(
        &config.repo_db,
        &[("hello", "2.0.0", 1, sky_path.to_str().unwrap())],
    );

    let result = verify(mode1_args("hello"), &config, settings);
    let err = result.expect_err("verification should fail on metadata mismatch");
    let chain = format!("{err:#}");
    assert!(
        chain.contains("metadata does not match the expected package"),
        "chain was: {chain}"
    );
    Ok(())
}

#[test]
fn mode1_signature_failure() -> Result<()> {
    let (_tmp, config, settings, _sky_path, tampered_path) = setup()?;
    seed_repo_db(
        &config.repo_db,
        &[("hello", "1.0.0", 1, tampered_path.to_str().unwrap())],
    );

    let result = verify(mode1_args("hello"), &config, settings);
    assert_chain(&result, "Failed to verify sky file signature");
    Ok(())
}

#[test]
fn mode2_happy_path() -> Result<()> {
    let (_tmp, config, settings, sky_path, _tampered_path) = setup()?;
    seed_repo_db(
        &config.repo_db,
        &[("hello", "1.0.0", 1, sky_path.to_str().unwrap())],
    );

    let result = verify(mode2_args(&sky_path), &config, settings);
    assert!(
        result.is_ok(),
        "unexpected verification failure: {result:?}"
    );
    Ok(())
}

#[test]
fn mode2_signature_failure() -> Result<()> {
    let (_tmp, config, settings, _sky_path, tampered_path) = setup()?;
    seed_repo_db(
        &config.repo_db,
        &[("hello", "1.0.0", 1, tampered_path.to_str().unwrap())],
    );

    let result = verify(mode2_args(&tampered_path), &config, settings);
    assert_chain(&result, "Failed to verify sky file signature");
    Ok(())
}

#[test]
fn mode_match_happy_path() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let config = runtime_config(&tmp)?;

    let contents = b"hello, world!\n";
    let payload = build_payload(contents, 0o644)?;

    let file_hash = hex::encode(hash(Cursor::new(contents))?);
    let files = format!(
        "f\t0644\t{}\tdir/file.txt\nd\t0755\t-\tdir\nl\t0777\tfile.txt\tdir/lnk\n",
        file_hash
    );

    let (sky_path, pub_key_path) = write_signed_sky(&payload, &files, &tmp, "modes.sky")?;
    let settings = SisyphusConfig {
        priv_key_path: tmp.path().join("unused.key"),
        pub_key_path,
    };

    let result = verify(mode2_args(&sky_path), &config, settings);
    assert!(
        result.is_ok(),
        "unexpected verification failure: {result:?}"
    );
    Ok(())
}

#[test]
fn mode_mismatch_failure() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let config = runtime_config(&tmp)?;

    let contents = b"hello, world!\n";
    let payload = build_payload(contents, 0o644)?;

    let file_hash = hex::encode(hash(Cursor::new(contents))?);
    let files = format!(
        "f\t0755\t{}\tdir/file.txt\nd\t0755\t-\tdir\nl\t0777\tfile.txt\tdir/lnk\n",
        file_hash
    );

    let (sky_path, pub_key_path) = write_signed_sky(&payload, &files, &tmp, "mismatch.sky")?;
    let settings = SisyphusConfig {
        priv_key_path: tmp.path().join("unused.key"),
        pub_key_path,
    };

    let result = verify(mode2_args(&sky_path), &config, settings);
    assert_chain(&result, "Invalid record, mode mismatch");
    Ok(())
}
