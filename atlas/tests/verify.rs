use std::{
    io::Cursor,
    path::{Path, PathBuf},
};

use anyhow::Result;
use atlas::cli::cli::VerifyArgs;
use atlas::types::runtime_config::RuntimeConfig;
use atlas::verify::verify::verify;
use common::types::{
    config::SisyphusConfig,
    metadata::{Build, Metadata},
    package::{Architectures, Source},
};
use common::{
    db::db::create_packages_table,
    functions::{hash::hash, sky_verify::data_to_sign},
};
use ring::signature::{Ed25519KeyPair, KeyPair};
use rusqlite::{Connection, params};
use tar::{Builder, EntryType, Header};
use tempfile::TempDir;
use zstd::Encoder;

const ARCH: Architectures = Architectures::X86_64;

fn runtime_config(tmp: &TempDir) -> Result<RuntimeConfig> {
    RuntimeConfig::build_at(tmp.path())
}

fn seed_repo_db(path: &Path, packages: &[(&str, &str, u32, &str)]) {
    let mut conn = Connection::open(path).unwrap();
    create_packages_table(&mut conn).unwrap();

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

fn raw_header(name: &str, mode: u32, size: u64) -> Header {
    raw_entry_header(name, mode, size, EntryType::file())
}

fn raw_entry_header(name: &str, mode: u32, size: u64, entry_type: EntryType) -> Header {
    let mut header = Header::new_gnu();
    header.set_mode(mode);
    header.set_size(size);
    header.set_entry_type(entry_type);
    let bytes = name.as_bytes();
    header.as_old_mut().name[..bytes.len()].copy_from_slice(bytes);
    header.set_cksum();
    header
}

fn write_raw_sky(tmp: &TempDir, name: &str, members: &[(&str, &[u8])]) -> Result<PathBuf> {
    let path = tmp.path().join(name);
    let file = std::fs::File::create(&path)?;
    let encoder = Encoder::new(file, 3)?;
    let mut builder = Builder::new(encoder);

    for (member_name, contents) in members {
        let header = raw_header(member_name, 0o644, contents.len() as u64);
        builder.append(&header, *contents)?;
    }

    builder.into_inner()?.finish()?;
    Ok(path)
}

fn payload_tar_with_entries(entries: &[(&str, &[u8])]) -> Result<Vec<u8>> {
    let mut payload = Vec::new();
    let encoder = Encoder::new(&mut payload, 3)?;
    let mut builder = Builder::new(encoder);

    for (entry_name, contents) in entries {
        let header = raw_header(entry_name, 0o644, contents.len() as u64);
        builder.append(&header, *contents)?;
    }

    builder.into_inner()?.finish()?;
    Ok(payload)
}

fn build_payload_with_path(path: &str, contents: &[u8]) -> Result<Vec<u8>> {
    payload_tar_with_entries(&[(path, contents)])
}

/// Like [`payload_tar_with_entries`], but each entry = (name, mode, type, contents),
/// so mode bits and forbidden entry types reach the reader.
fn payload_tar_with_typed_entries(entries: &[(&str, u32, EntryType, &[u8])]) -> Result<Vec<u8>> {
    let mut payload = Vec::new();
    let encoder = Encoder::new(&mut payload, 3)?;
    let mut builder = Builder::new(encoder);

    for (entry_name, mode, entry_type, contents) in entries {
        let header = raw_entry_header(entry_name, *mode, contents.len() as u64, *entry_type);
        builder.append(&header, *contents)?;
    }

    builder.into_inner()?.finish()?;
    Ok(payload)
}

fn build_payload_with_typed_entry(
    path: &str,
    mode: u32,
    entry_type: EntryType,
    contents: &[u8],
) -> Result<Vec<u8>> {
    payload_tar_with_typed_entries(&[(path, mode, entry_type, contents)])
}

fn build_payload_with_duplicate(path: &str, contents1: &[u8], contents2: &[u8]) -> Result<Vec<u8>> {
    payload_tar_with_entries(&[(path, contents1), (path, contents2)])
}

fn canonical_metadata_bytes() -> Result<Vec<u8>> {
    Ok(serde_saphyr::to_string(&test_metadata())?.into_bytes())
}

fn write_sky_with_payload(tmp: &TempDir, name: &str, payload: &[u8]) -> Result<PathBuf> {
    let metadata_bytes = canonical_metadata_bytes()?;
    let sky_path = tmp.path().join(name);
    write_sky(&sky_path, payload, &metadata_bytes, "", &[0u8; 64])?;
    Ok(sky_path)
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

fn build_payload_with_extra_file(
    contents: &[u8],
    file_mode: u32,
    extra_path: &str,
    extra_contents: &[u8],
    extra_mode: u32,
) -> Result<Vec<u8>> {
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

    let mut extra_header = Header::new_gnu();
    extra_header.set_mode(extra_mode);
    extra_header.set_size(extra_contents.len() as u64);
    builder.append_data(&mut extra_header, extra_path, extra_contents)?;

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
            sha256: "unused".to_string(),
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
        "f\t0644\t{file_hash}\tdir/file.txt\nd\t0755\t-\tdir\nl\t0777\tfile.txt\tdir/lnk\n",
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
        "f\t0644\t{file_hash}\tdir/file.txt\nd\t0755\t-\tdir\nl\t0777\tfile.txt\tdir/lnk\n",
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
        "f\t0755\t{file_hash}\tdir/file.txt\nd\t0755\t-\tdir\nl\t0777\tfile.txt\tdir/lnk\n",
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

#[test]
fn manifest_references_path_not_in_payload() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let config = runtime_config(&tmp)?;

    let contents = b"hello, world!\n";
    let payload = build_payload(contents, 0o644)?;

    let file_hash = hex::encode(hash(Cursor::new(contents))?);
    let files = format!(
        "f\t0644\t{}\tdir/file.txt\nd\t0755\t-\tdir\nl\t0777\tfile.txt\tdir/lnk\nf\t0644\t{}\tdir/missing.txt\n",
        file_hash,
        "0".repeat(64)
    );

    let (sky_path, pub_key_path) =
        write_signed_sky(&payload, &files, &tmp, "manifest_missing.sky")?;
    let settings = SisyphusConfig {
        priv_key_path: tmp.path().join("unused.key"),
        pub_key_path,
    };

    let result = verify(mode2_args(&sky_path), &config, settings);
    assert_chain(&result, "manifest references path");
    assert_chain(&result, "not in payload");
    assert_chain(&result, "dir/missing.txt");
    Ok(())
}

#[test]
fn payload_contains_path_not_in_manifest() -> Result<()> {
    let tmp = TempDir::new().unwrap();
    let config = runtime_config(&tmp)?;

    let contents = b"hello, world!\n";
    let extra = b"secret\n";
    let payload = build_payload_with_extra_file(contents, 0o644, "dir/secret.txt", extra, 0o644)?;

    let file_hash = hex::encode(hash(Cursor::new(contents))?);
    let files = format!(
        "f\t0644\t{file_hash}\tdir/file.txt\nd\t0755\t-\tdir\nl\t0777\tfile.txt\tdir/lnk\n",
    );

    let (sky_path, pub_key_path) = write_signed_sky(&payload, &files, &tmp, "payload_extra.sky")?;
    let settings = SisyphusConfig {
        priv_key_path: tmp.path().join("unused.key"),
        pub_key_path,
    };

    let result = verify(mode2_args(&sky_path), &config, settings);
    assert_chain(&result, "payload contains path");
    assert_chain(&result, "not in manifest");
    assert_chain(&result, "dir/secret.txt");
    Ok(())
}

mod outer_archive_validation {
    use atlas::verify::validate_archive::validate_archive_paths;

    use super::*;

    fn canonical_members<'a>(
        metadata: &'a [u8],
        payload: &'a [u8],
        signature: &'a [u8],
    ) -> Vec<(&'a str, &'a [u8])> {
        vec![
            ("metadata.yaml", metadata),
            ("payload.tar.zst", payload),
            ("files", b""),
            ("signature.ed25519", signature),
        ]
    }

    fn valid_payload() -> Result<Vec<u8>> {
        build_payload(b"hello\n", 0o644)
    }

    #[test]
    fn absolute_path_in_outer_archive() -> Result<()> {
        let tmp = TempDir::new().unwrap();
        let metadata = canonical_metadata_bytes()?;
        let payload = valid_payload()?;
        let signature = [0u8; 64];

        let mut members = canonical_members(&metadata, &payload, &signature);
        members[0].0 = "/metadata.yaml";
        let sky_path = write_raw_sky(&tmp, "absolute.sky", &members)?;

        let result = validate_archive_paths(&sky_path);
        assert_chain(&result, "absolute path");
        assert_chain(&result, "/metadata.yaml");
        Ok(())
    }

    #[test]
    fn parent_dir_in_outer_archive() -> Result<()> {
        let tmp = TempDir::new().unwrap();
        let metadata = canonical_metadata_bytes()?;
        let payload = valid_payload()?;
        let signature = [0u8; 64];

        let mut members = canonical_members(&metadata, &payload, &signature);
        members[0].0 = "../metadata.yaml";
        let sky_path = write_raw_sky(&tmp, "parentdir.sky", &members)?;

        let result = validate_archive_paths(&sky_path);
        assert_chain(&result, "ParentDir");
        assert_chain(&result, "../metadata.yaml");
        Ok(())
    }

    #[test]
    fn duplicate_outer_archive_member() -> Result<()> {
        let tmp = TempDir::new().unwrap();
        let metadata = canonical_metadata_bytes()?;
        let payload = valid_payload()?;
        let signature = [0u8; 64];

        let sky_path = write_raw_sky(
            &tmp,
            "duplicate.sky",
            &[
                ("metadata.yaml", &metadata),
                ("metadata.yaml", b"duplicate"),
                ("payload.tar.zst", &payload),
                ("files", b""),
                ("signature.ed25519", &signature),
            ],
        )?;

        let result = validate_archive_paths(&sky_path);
        assert_chain(&result, "Duplicate");
        assert_chain(&result, "metadata.yaml");
        Ok(())
    }

    #[test]
    fn unknown_outer_archive_member() -> Result<()> {
        let tmp = TempDir::new().unwrap();
        let metadata = canonical_metadata_bytes()?;
        let payload = valid_payload()?;
        let signature = [0u8; 64];

        let sky_path = write_raw_sky(
            &tmp,
            "unknown.sky",
            &[
                ("metadata.yaml", &metadata),
                ("payload.tar.zst", &payload),
                ("files", b""),
                ("signature.ed25519", &signature),
                ("hax.txt", b"malicious"),
            ],
        )?;

        let result = validate_archive_paths(&sky_path);
        assert_chain(&result, "Unexpected");
        assert_chain(&result, "hax.txt");
        Ok(())
    }

    #[test]
    fn missing_canonical_member() -> Result<()> {
        let tmp = TempDir::new().unwrap();
        let metadata = canonical_metadata_bytes()?;
        let payload = valid_payload()?;

        let sky_path = write_raw_sky(
            &tmp,
            "missing.sky",
            &[
                ("metadata.yaml", &metadata),
                ("payload.tar.zst", &payload),
                ("files", b""),
            ],
        )?;

        let result = validate_archive_paths(&sky_path);
        assert_chain(&result, "missing");
        Ok(())
    }
}

mod payload_archive_validation {
    use atlas::verify::validate_payload::validate_payload_paths;

    use super::*;

    #[test]
    fn missing_payload_is_rejected_even_when_other_members_are_present() -> Result<()> {
        let tmp = TempDir::new()?;
        let sky = write_raw_sky(
            &tmp,
            "missing-payload.sky",
            &[("metadata.yaml", b"metadata"), ("files", b"manifest")],
        )?;
        let result = validate_payload_paths(&sky);
        assert_chain(&result, "Payload not found in sky file");
        Ok(())
    }

    #[test]
    fn valid_payload_is_found_between_unrelated_archive_members() -> Result<()> {
        let tmp = TempDir::new()?;
        let payload =
            build_payload_with_typed_entry("dir/file.txt", 0o644, EntryType::file(), b"safe\n")?;
        let sky = write_raw_sky(
            &tmp,
            "valid-payload.sky",
            &[
                ("metadata.yaml", b"metadata"),
                ("payload.tar.zst", &payload),
                ("files", b"manifest"),
            ],
        )?;
        validate_payload_paths(&sky)
    }

    #[test]
    fn absolute_path_in_payload() -> Result<()> {
        let tmp = TempDir::new().unwrap();
        let payload = build_payload_with_path("/etc/passwd", b"malicious\n")?;
        let sky_path = write_sky_with_payload(&tmp, "payload_absolute.sky", &payload)?;

        let result = validate_payload_paths(&sky_path);
        assert_chain(&result, "absolute");
        assert_chain(&result, "/etc/passwd");
        Ok(())
    }

    #[test]
    fn parent_dir_in_payload() -> Result<()> {
        let tmp = TempDir::new().unwrap();
        let payload = build_payload_with_path("dir/../etc/passwd", b"malicious\n")?;
        let sky_path = write_sky_with_payload(&tmp, "payload_parentdir.sky", &payload)?;

        let result = validate_payload_paths(&sky_path);
        assert_chain(&result, "ParentDir");
        assert_chain(&result, "dir/../etc/passwd");
        Ok(())
    }

    #[test]
    fn duplicate_payload_member() -> Result<()> {
        let tmp = TempDir::new().unwrap();
        let payload = build_payload_with_duplicate("dir/file.txt", b"first\n", b"second\n")?;
        let sky_path = write_sky_with_payload(&tmp, "payload_duplicate.sky", &payload)?;

        let result = validate_payload_paths(&sky_path);
        assert_chain(&result, "Duplicate");
        assert_chain(&result, "dir/file.txt");
        Ok(())
    }

    #[test]
    fn setuid_bit_in_payload() -> Result<()> {
        let tmp = TempDir::new().unwrap();
        let payload =
            build_payload_with_typed_entry("dir/evil.bin", 0o4755, EntryType::file(), b"x\n")?;
        let sky_path = write_sky_with_payload(&tmp, "payload_setuid.sky", &payload)?;

        let result = validate_payload_paths(&sky_path);
        assert_chain(&result, "setuid");
        assert_chain(&result, "4755");
        assert_chain(&result, "dir/evil.bin");
        Ok(())
    }

    #[test]
    fn setgid_bit_in_payload() -> Result<()> {
        let tmp = TempDir::new().unwrap();
        let payload =
            build_payload_with_typed_entry("dir/evil.bin", 0o2755, EntryType::file(), b"x\n")?;
        let sky_path = write_sky_with_payload(&tmp, "payload_setgid.sky", &payload)?;

        let result = validate_payload_paths(&sky_path);
        assert_chain(&result, "setgid");
        assert_chain(&result, "2755");
        assert_chain(&result, "dir/evil.bin");
        Ok(())
    }

    #[test]
    fn fifo_entry_in_payload() -> Result<()> {
        let tmp = TempDir::new().unwrap();
        let payload = build_payload_with_typed_entry("dir/pipe", 0o644, EntryType::fifo(), b"")?;
        let sky_path = write_sky_with_payload(&tmp, "payload_fifo.sky", &payload)?;

        let result = validate_payload_paths(&sky_path);
        assert_chain(&result, "invalid type");
        assert_chain(&result, "Fifo");
        assert_chain(&result, "dir/pipe");
        Ok(())
    }

    #[test]
    fn device_entry_in_payload() -> Result<()> {
        let tmp = TempDir::new().unwrap();
        let payload =
            build_payload_with_typed_entry("dir/dev", 0o644, EntryType::block_special(), b"")?;
        let sky_path = write_sky_with_payload(&tmp, "payload_device.sky", &payload)?;

        let result = validate_payload_paths(&sky_path);
        assert_chain(&result, "invalid type");
        assert_chain(&result, "Block");
        assert_chain(&result, "dir/dev");
        Ok(())
    }
}

#[test]
fn symlink_target_mismatch_is_rejected_even_when_the_signature_is_valid() -> Result<()> {
    let tmp = TempDir::new()?;
    let config = runtime_config(&tmp)?;
    let contents = b"hello\n";
    let payload = build_payload(contents, 0o644)?;
    let file_hash = hex::encode(hash(Cursor::new(contents))?);
    let files = format!(
        "f\t0644\t{file_hash}\tdir/file.txt\nd\t0755\t-\tdir\nl\t0777\twrong.txt\tdir/lnk\n"
    );
    let (sky, pub_key_path) = write_signed_sky(&payload, &files, &tmp, "wrong-target.sky")?;
    let settings = SisyphusConfig {
        priv_key_path: tmp.path().join("unused.key"),
        pub_key_path,
    };
    let result = verify(mode2_args(&sky), &config, settings);
    assert_chain(&result, "symlink is pointing to the wrong file");
    Ok(())
}
