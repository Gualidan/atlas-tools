use anyhow::Result;
use common::{
    functions::{sky_verify::verify_metadata, unpack_payload::unpack_payload},
    types::{
        manifest::{FileType, ManifestLine},
        package::{Architectures, PackageIdentity},
    },
};
use std::{fs, io::Cursor, path::PathBuf};
use tar::{Builder, Header};

const METADATA: &[u8] = br"format: 1
name: example
version: 1.2.3
release: 4
architecture: x86_64
deps: [runtime]
source:
  url: https://example.invalid/source.tar.gz
  sha256: unused
build:
  recipe_sha256: deadbeef
  built_at: '2026-01-01T00:00:00Z'
  builder: tester
";

#[test]
fn metadata_verification_accepts_a_slice_and_identity_borrowed_from_owned_strings() -> Result<()> {
    let name = String::from("example");
    let version = String::from("1.2.3");
    let identity = PackageIdentity {
        name: name.clone(),
        version: version.clone(),
        release: 4,
        architecture: Architectures::X86_64,
    };
    let metadata = verify_metadata(METADATA, &identity)?;
    assert_eq!(metadata.name, name);
    assert_eq!(metadata.version, version);
    assert_eq!(metadata.release, 4);
    assert_eq!(metadata.architecture, Architectures::X86_64);
    assert_eq!(metadata.deps, vec!["runtime"]);
    Ok(())
}

#[test]
fn package_identity_owns_escaped_strings_after_the_yaml_buffer_is_dropped() -> Result<()> {
    let identity: PackageIdentity = {
        let yaml = String::from(
            "name: \"ex\\u0061mple\"\nversion: \"1.2.\\u0033\"\nrelease: 4\narchitecture: x86_64\n",
        );
        serde_saphyr::from_str(&yaml)?
    };

    let metadata = verify_metadata(METADATA, &identity)?;

    assert_eq!(identity.name, "example");
    assert_eq!(identity.version, "1.2.3");
    assert_eq!(metadata.name, identity.name);
    assert_eq!(metadata.version, identity.version);
    Ok(())
}

#[test]
fn cloning_a_package_identity_keeps_independent_strings_and_verification() -> Result<()> {
    let original: PackageIdentity = serde_saphyr::from_str(
        "name: example\nversion: 1.2.3\nrelease: 4\narchitecture: x86_64\n",
    )?;
    let mut cloned = original.clone();
    cloned.name.push_str("-other");
    cloned.version.push_str("-rc.1");

    verify_metadata(METADATA, &original)?;
    assert!(verify_metadata(METADATA, &cloned).is_err());
    assert_eq!(original.name, "example");
    assert_eq!(original.version, "1.2.3");
    assert_eq!(cloned.release, original.release);
    assert_eq!(cloned.architecture, original.architecture);
    drop(original);
    assert_eq!(cloned.name, "example-other");
    assert_eq!(cloned.version, "1.2.3-rc.1");
    Ok(())
}

#[test]
fn each_identity_field_must_match_the_metadata() {
    let identity = PackageIdentity {
        name: "example".to_string(),
        version: "1.2.3".to_string(),
        release: 4,
        architecture: Architectures::X86_64,
    };
    for (field, mismatch) in [
        (
            "name",
            PackageIdentity {
                name: "different".to_string(),
                ..identity.clone()
            },
        ),
        (
            "version",
            PackageIdentity {
                version: "1.2.4".to_string(),
                ..identity.clone()
            },
        ),
        (
            "release",
            PackageIdentity {
                release: 5,
                ..identity.clone()
            },
        ),
        (
            "architecture",
            PackageIdentity {
                architecture: Architectures::ARM,
                ..identity
            },
        ),
    ] {
        let err =
            verify_metadata(METADATA, &mismatch).expect_err("identity mismatch must be rejected");
        assert!(
            err.to_string().contains("metadata does not match"),
            "{field}: {err:#}"
        );
        let message = err.to_string();
        for identity_value in ["example", "1.2.3", &mismatch.name, &mismatch.version] {
            assert!(message.contains(identity_value), "{field}: {message}");
        }
        for non_identity_value in [
            "source",
            "https://example.invalid/source.tar.gz",
            "unused",
            "runtime",
            "deadbeef",
            "2026-01-01T00:00:00Z",
            "tester",
        ] {
            assert!(!message.contains(non_identity_value), "{field}: {message}");
        }
    }
}

#[test]
fn malformed_or_incomplete_metadata_is_rejected() {
    let identity = PackageIdentity {
        name: "example".to_string(),
        version: "1.2.3".to_string(),
        release: 4,
        architecture: Architectures::X86_64,
    };
    for bytes in [
        b"format: [".as_slice(),
        b"name: example\n".as_slice(),
        b"".as_slice(),
    ] {
        let err = verify_metadata(bytes, &identity).expect_err("invalid metadata must be rejected");
        assert!(
            format!("{err:#}").contains("Failed to deserialize metadata"),
            "{err:#}"
        );
    }
}

#[test]
fn display_and_manifest_format_preserve_the_file_type_wire_values() {
    for (kind, marker, mode, target) in [
        (FileType::File, "f", 0o100_644, "deadbeef"),
        (FileType::Dir, "d", 0o40_755, "-"),
        (FileType::Symlink, "l", 0o120_777, "../lib/example"),
    ] {
        assert_eq!(format!("{kind}"), marker);
        assert_eq!(kind.to_string(), marker);
        let line = ManifestLine {
            file_type: kind,
            mode,
            target_or_hash: target.into(),
            path: PathBuf::from("path with spaces"),
        };
        let expected_mode = match marker {
            "f" => "0644",
            "d" => "0755",
            "l" => "0777",
            _ => unreachable!(),
        };
        assert_eq!(
            line.format(),
            format!("{marker}\t{expected_mode}\t{target}\tpath with spaces\n")
        );
    }
}

fn append(builder: &mut Builder<Vec<u8>>, name: &str, contents: &[u8]) -> Result<()> {
    let mut header = Header::new_gnu();
    header.set_mode(0o644);
    header.set_size(contents.len() as u64);
    builder.append_data(&mut header, name, contents)?;
    Ok(())
}

fn sky_with_payload(payload: &[u8]) -> Result<(tempfile::TempDir, PathBuf)> {
    let tmp = tempfile::tempdir()?;
    let mut sky = Builder::new(Vec::new());
    append(&mut sky, "metadata.yaml", METADATA)?;
    append(&mut sky, "payload.tar.zst", payload)?;
    append(&mut sky, "files", b"not payload contents")?;
    append(&mut sky, "signature.ed25519", b"not payload contents")?;
    let path = tmp.path().join("example.sky");
    fs::write(&path, zstd::encode_all(Cursor::new(sky.into_inner()?), 1)?)?;
    Ok((tmp, path))
}

#[test]
fn unpack_extracts_the_payload_and_skips_surrounding_archive_members() -> Result<()> {
    let mut payload = Builder::new(Vec::new());
    append(&mut payload, "dir/file.txt", b"payload contents\n")?;
    let payload = zstd::encode_all(Cursor::new(payload.into_inner()?), 1)?;
    let (_tmp, sky) = sky_with_payload(&payload)?;
    let destination = tempfile::tempdir()?;

    unpack_payload(&sky, &destination)?;

    assert_eq!(
        fs::read(destination.path().join("dir/file.txt"))?,
        b"payload contents\n"
    );
    for member in [
        "metadata.yaml",
        "payload.tar.zst",
        "files",
        "signature.ed25519",
    ] {
        assert!(
            !destination.path().join(member).exists(),
            "wrapper member {member} must not be extracted"
        );
    }
    Ok(())
}

#[test]
fn unpack_reports_corrupt_payload_compression() -> Result<()> {
    let (_tmp, sky) = sky_with_payload(b"not a zstd stream")?;
    let destination = tempfile::tempdir()?;
    let err = unpack_payload(&sky, &destination).expect_err("invalid payload must fail");
    assert!(
        format!("{err:#}").contains("Failed to unpack archive to destination"),
        "{err:#}"
    );
    assert!(fs::read_dir(destination.path())?.next().is_none());
    Ok(())
}
