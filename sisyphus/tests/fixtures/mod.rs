//! Shared repository-DB fixtures for sisyphus integration tests.
//!
//! This is a module, not a test target: `repo_db.rs` and `package_groups.rs`
//! both `mod fixtures;`, so they share these builders without compiling or
//! running each other's `#[test]` functions.

#![allow(dead_code)]

use common::types::{
    metadata::{Build, Metadata},
    package::{Architectures, Source},
};
use sisyphus::types::package::Package;

/// Builds a package fixture with an unreachable source.
///
/// `groups` is explicit so callers cannot accidentally test the grouped and
/// ungrouped cases with the same arrange.
#[must_use]
pub fn package(
    name: &str,
    version: &str,
    architecture: Architectures,
    groups: Vec<String>,
) -> Package {
    Package {
        schema: 1,
        name: name.to_string(),
        version: version.to_string(),
        release: 1,
        architecture,
        source: Source {
            url: "https://example.invalid/src.tar.gz".to_string(),
            sha256: "unused".to_string(),
        },
        makedeps: vec![],
        deps: vec![],
        groups,
        prepare: None,
        build: None,
        check: None,
        package: "true".to_string(),
    }
}

/// Builds identity-neutral metadata.
///
/// `insert_package` only reads `build.*`, so the mismatched identity is
/// deliberate: this helper is for tests that do not read the row back. Use
/// `package_groups::metadata_for` when the row identity matters.
#[must_use]
pub fn metadata() -> Metadata {
    Metadata {
        format: 1,
        name: "unused".to_string(),
        version: "0.0.0".to_string(),
        release: 1,
        architecture: Architectures::X86_64,
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
