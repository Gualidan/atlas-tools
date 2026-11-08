//! Fixtures for the package-group publishing scenarios.
//!
//! This file intentionally contains **no `#[test]` functions**. It is the
//! shared arranger for the group tests: recipe construction, publishing,
//! repository read-back, and the invariants used after a failed publish.
//!
//! Facts these fixtures encode (and keep honest):
//!
//! * Group membership is repository-DB state (`packages.groups`), scoped by
//!   `(name, architecture)`. It is deliberately absent from `.sky` metadata
//!   (`docs/sky-format.md`), so a "signed membership" test must read the DB for
//!   membership and separately verify the `.sky` signature.
//! * A group is never a dependency: `deps` and `makedeps` are separate columns
//!   and separate recipe lists.
//! * Sky paths are derived from `name-version-release-architecture`, so two
//!   fixtures can never silently share an artifact path.
//! * Publishing writes the `.sky` file before the repository DB row. A failed
//!   DB write therefore leaves the archive on disk; the snapshots below keep
//!   "DB unchanged" and "sky file remains" as separate observations.

#![allow(dead_code)]

use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, ensure};
use common::{
    db::db::create_packages_table,
    functions::sky_verify::{data_to_sign, read_sky, verify_metadata, verify_signature},
    types::{
        config::SisyphusConfig,
        metadata::Metadata,
        package::{Architectures, PackageIdentity},
    },
};

use rusqlite::{Connection, OptionalExtension, params};
use sisyphus::{
    package::{
        db::insert_package, metadata::metadata_gen, payload::payload_gen, sign::sign_artifacts,
        sky::sky_gen,
    },
    signing::signing::keygen,
    types::{package::Package, recipe::Recipe, runtime_config::RuntimeConfig},
};

mod fixtures;

use fixtures::{metadata, package};
use tempfile::tempdir;

/// The architecture used by most group scenarios.
pub const X86_64: Architectures = Architectures::X86_64;
/// The second architecture used by the "same group, two architectures" case.
pub const ARM: Architectures = Architectures::ARM;

/// A valid, simple group label.
pub const GROUP: &str = "foo_group";
/// Another valid label, used for multi-group and republish cases.
pub const OTHER_GROUP: &str = "bar_group";

/// Duplicate group labels, which validation must reject before any side effect.
#[must_use]
pub fn duplicate_groups() -> Vec<String> {
    vec![GROUP.to_string(), GROUP.to_string()]
}

/// Group labels that are invalid by the package-name grammar.
///
/// Kept as a list so the validation test can exercise every bad shape without
/// re-inventing the literal set in the test body.
#[must_use]
pub fn invalid_group_names() -> Vec<String> {
    vec![
        String::new(),           // empty
        "foo group".to_string(), // whitespace
        "FooGroup".to_string(),  // uppercase
        "foo/group".to_string(), // path separator
        "-foo".to_string(),      // must start with [a-z0-9]
        "foo\0bar".to_string(),  // control character
    ]
}

/// Builds a package fixture with runtime and build dependencies in addition to
/// group labels.
#[must_use]
pub fn package_with(
    name: &str,
    version: &str,
    architecture: Architectures,
    groups: Vec<String>,
    deps: Vec<String>,
    makedeps: Vec<String>,
) -> Package {
    let mut package = package(name, version, architecture, groups);
    package.deps = deps;
    package.makedeps = makedeps;
    package
}

/// Builds metadata whose identity matches `package`.
///
/// `fixtures::metadata()` is deliberately identity-neutral ("unused", 0.0.0),
/// which is fine for `insert_package` (it only reads `build.*`), but it is the
/// wrong arrange for any test that reads the row back and expects the package
/// it published. Use this helper instead.
#[must_use]
pub fn metadata_for(package: &Package) -> Metadata {
    let mut metadata = metadata();
    metadata.name.clone_from(&package.name);
    metadata.version.clone_from(&package.version);
    metadata.release = package.release;
    metadata.architecture = package.architecture;
    metadata.deps.clone_from(&package.deps);
    metadata.source.clone_from(&package.source);
    metadata
}

/// Serializes the metadata a `.sky` file would carry for `package`.
///
/// Useful for asserting that group membership is *not* part of the signed
/// metadata contract.
///
/// # Errors
///
/// Returns an error if the metadata cannot be serialized.
pub fn metadata_yaml(package: &Package) -> Result<String> {
    serde_saphyr::to_string(&metadata_for(package)).context("Failed to serialize fixture metadata")
}

/// Asserts that membership is not smuggled into `.sky` metadata.
///
/// # Errors
///
/// Returns an error if a `groups` key appears in the metadata YAML.
pub fn assert_metadata_omits_groups(package: &Package) -> Result<()> {
    let yaml = metadata_yaml(package)?;
    ensure!(
        !yaml.lines().any(|line| line.starts_with("groups:")),
        "package metadata unexpectedly encodes group membership:\n{yaml}"
    );
    Ok(())
}

/// A recipe file fixture.
///
/// `groups: None` means the key is omitted entirely, which is the backward
/// compatibility case. `groups: Some(vec![])` writes `groups: []` and is a
/// different arrange.
#[derive(Clone, Debug)]
pub struct RecipeFixture {
    pub name: String,
    pub version: String,
    pub release: u32,
    pub architecture: Architectures,
    pub groups: Option<Vec<String>>,
    pub deps: Vec<String>,
    pub makedeps: Vec<String>,
    pub source_url: String,
    pub source_sha256: String,
    pub package_script: String,
}

impl RecipeFixture {
    /// Starts a valid recipe with no `groups` key.
    #[must_use]
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            release: 1,
            architecture: Architectures::X86_64,
            groups: None,
            deps: Vec::new(),
            makedeps: Vec::new(),
            // Never resolvable: validation tests must fail before a fetch.
            source_url: "https://example.invalid/src.tar.gz".to_string(),
            source_sha256: "unused".to_string(),
            package_script: "true".to_string(),
        }
    }

    /// Mirrors an existing package into a recipe, including its groups.
    #[must_use]
    pub fn for_package(package: &Package) -> Self {
        let mut fixture = Self::new(package.name.clone(), package.version.clone())
            .with_release(package.release)
            .with_architecture(package.architecture);
        fixture.groups = if package.groups.is_empty() {
            None
        } else {
            Some(package.groups.clone())
        };
        fixture.deps.clone_from(&package.deps);
        fixture.makedeps.clone_from(&package.makedeps);
        fixture.source_url.clone_from(&package.source.url);
        fixture.source_sha256.clone_from(&package.source.sha256);
        fixture
    }

    #[must_use]
    pub fn with_release(mut self, release: u32) -> Self {
        self.release = release;
        self
    }

    #[must_use]
    pub fn with_architecture(mut self, architecture: Architectures) -> Self {
        self.architecture = architecture;
        self
    }

    /// Writes an explicit `groups:` list.
    #[must_use]
    pub fn with_groups<I, S>(mut self, groups: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.groups = Some(groups.into_iter().map(Into::into).collect());
        self
    }

    /// Removes the `groups` key completely.
    #[must_use]
    pub fn without_groups_field(mut self) -> Self {
        self.groups = None;
        self
    }

    #[must_use]
    pub fn with_deps<I, S>(mut self, deps: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.deps = deps.into_iter().map(Into::into).collect();
        self
    }

    #[must_use]
    pub fn with_makedeps<I, S>(mut self, makedeps: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.makedeps = makedeps.into_iter().map(Into::into).collect();
        self
    }

    /// Renders the recipe as YAML.
    #[must_use]
    pub fn to_yaml(&self) -> String {
        let mut yaml = String::new();

        writeln!(yaml, "schema: 1").expect("writing to a String cannot fail");
        push_scalar(&mut yaml, "name", &self.name);
        push_scalar(&mut yaml, "version", &self.version);
        writeln!(yaml, "release: {}", self.release).expect("writing to a String cannot fail");
        push_scalar(&mut yaml, "architecture", &self.architecture.to_string());
        writeln!(yaml, "source:").expect("writing to a String cannot fail");
        push_scalar(&mut yaml, "  url", &self.source_url);
        push_scalar(&mut yaml, "  sha256", &self.source_sha256);

        if let Some(groups) = &self.groups {
            push_list(&mut yaml, "groups", groups);
        }
        push_list(&mut yaml, "deps", &self.deps);
        push_list(&mut yaml, "makedeps", &self.makedeps);

        writeln!(yaml, "package: |").expect("writing to a String cannot fail");
        for line in self.package_script.lines() {
            writeln!(yaml, "  {line}").expect("writing to a String cannot fail");
        }

        yaml
    }

    /// Writes the recipe to `<recipe_repo>/<name>/sky.yaml` and returns the path.
    ///
    /// # Errors
    ///
    /// Returns an error if the recipe directory or file cannot be written.
    pub fn write(&self, config: &RuntimeConfig) -> Result<PathBuf> {
        let path = config.recipe_repo.join(&self.name).join("sky.yaml");
        let parent = path
            .parent()
            .context("Fixture recipe path has no parent directory")?;
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create recipe directory `{}`", parent.display()))?;
        fs::write(&path, self.to_yaml())
            .with_context(|| format!("Failed to write fixture recipe `{}`", path.display()))?;
        Ok(path)
    }

    /// Parses a recipe through the production reader so validation is exercised.
    ///
    /// # Errors
    ///
    /// Returns an error if the recipe cannot be read, parsed, or validated.
    pub fn parse_path(path: &Path) -> Result<Package> {
        let path = path.to_path_buf();
        Recipe { path: &path }.parse()
    }

    /// Writes, then parses through the production reader.
    ///
    /// # Errors
    ///
    /// Returns an error if the recipe cannot be written, read, parsed, or
    /// validated.
    pub fn parse(&self, config: &RuntimeConfig) -> Result<Package> {
        let path = self.write(config)?;
        Self::parse_path(&path)
    }
}

fn yaml_scalar(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn push_scalar(out: &mut String, key: &str, value: &str) {
    writeln!(out, "{key}: {}", yaml_scalar(value)).expect("writing to a String cannot fail");
}

fn push_list(out: &mut String, key: &str, values: &[String]) {
    if values.is_empty() {
        writeln!(out, "{key}: []").expect("writing to a String cannot fail");
        return;
    }
    writeln!(out, "{key}:").expect("writing to a String cannot fail");
    for value in values {
        writeln!(out, "  - {}", yaml_scalar(value)).expect("writing to a String cannot fail");
    }
}

/// The on-disk location `sky_gen`/`package_gen` would use for this package.
#[must_use]
pub fn sky_path_for(config: &RuntimeConfig, package: &Package) -> PathBuf {
    config
        .sky_repo
        .join(package.architecture.to_string())
        .join(format!(
            "{}-{}-{}-{}.sky",
            package.name, package.version, package.release, package.architecture
        ))
}

/// The staging directory used by the signed-publish fixture.
#[must_use]
pub fn package_stage_dir(config: &RuntimeConfig, package: &Package) -> PathBuf {
    config.srcdir.join(format!(
        "{}-{}-{}-{}",
        package.name, package.version, package.release, package.architecture
    ))
}

/// Records a package row without creating a real `.sky` archive.
///
/// Use this for DB-only scenarios. The caller supplies the path, so a test that
/// cares about the artifact can point at a stub it wrote first.
///
/// # Errors
///
/// Returns an error if the row cannot be inserted or the arrange cannot be read
/// back as exactly one row.
pub fn publish(config: &RuntimeConfig, package: &Package, sky_path: &Path) -> Result<()> {
    let sky_path = sky_path
        .to_str()
        .context("Fixture sky path is not valid UTF-8")?;
    insert_package(config, package, sky_path, &metadata_for(package))?;

    let rows = row_count(config, &package.name, package.architecture)?;
    ensure!(
        rows == 1,
        "arrange failed: expected one repository row for `{}` ({}), found {rows}",
        package.name,
        package.architecture
    );
    Ok(())
}

/// Writes a placeholder `.sky` at the package's canonical path, then inserts the
/// repository row.
///
/// This is the fast arrange for scenarios that treat the artifact as opaque. It
/// intentionally keeps the write-before-insert ordering of the real pipeline so
/// the rollback scenario sees the same leftover file.
///
/// # Errors
///
/// Returns an error if the stub or row cannot be written.
pub fn publish_stub(config: &RuntimeConfig, package: &Package) -> Result<PathBuf> {
    let sky_path = sky_path_for(config, package);
    let parent = sky_path
        .parent()
        .context("Fixture sky path has no parent directory")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("Failed to create sky directory `{}`", parent.display()))?;
    fs::write(
        &sky_path,
        format!("stub sky for {} {}\n", package.name, package.version),
    )
    .with_context(|| format!("Failed to write sky stub `{}`", sky_path.display()))?;

    publish(config, package, &sky_path)?;
    Ok(sky_path)
}

/// The result of publishing a real, signed `.sky` file.
pub struct PublishedPackage {
    pub package: Package,
    pub metadata: Metadata,
    pub sky_path: PathBuf,
    pub pub_key_path: PathBuf,
}

/// Writes and parses a recipe, then publishes it as a real signed `.sky`.
///
/// This path uses payload generation, metadata generation, signing, and the sky
/// writer, but not fetch/extract/build, so it does not need libarchive or
/// bubblewrap. It does exercise `insert_package` through `sky_gen`.
///
/// # Errors
///
/// Returns an error if the recipe, payload, metadata, key, signature, archive,
/// or repository row cannot be written.
pub fn publish_recipe(config: &RuntimeConfig, recipe: &RecipeFixture) -> Result<PublishedPackage> {
    let recipe_path = recipe.write(config)?;
    let package = RecipeFixture::parse_path(&recipe_path)?;
    publish_signed(config, &package, &recipe_path)
}

/// Publishes `package` as a real signed `.sky`, using `recipe_path` for the
/// recipe hash that ends up in metadata.
///
/// The payload path is unique per package/version so two fixtures can never
/// collide, and a rerun starts from a clean staging directory.
///
/// # Errors
///
/// Returns an error if the staging tree, payload, metadata, key, signature,
/// archive, or repository row cannot be written.
pub fn publish_signed(
    config: &RuntimeConfig,
    package: &Package,
    recipe_path: &Path,
) -> Result<PublishedPackage> {
    let stage = package_stage_dir(config, package);
    if stage.exists() {
        fs::remove_dir_all(&stage).with_context(|| {
            format!(
                "Failed to clear previous staging directory `{}`",
                stage.display()
            )
        })?;
    }

    let payload_path = stage
        .join("usr")
        .join("share")
        .join(&package.name)
        .join(format!("{}.txt", package.version));
    let payload_parent = payload_path
        .parent()
        .context("Fixture payload path has no parent directory")?;
    fs::create_dir_all(payload_parent).with_context(|| {
        format!(
            "Failed to create fixture payload directory `{}`",
            payload_parent.display()
        )
    })?;
    fs::write(
        &payload_path,
        format!("{} {}\n", package.name, package.version),
    )
    .with_context(|| {
        format!(
            "Failed to write fixture payload `{}`",
            payload_path.display()
        )
    })?;

    let (manifest, payload) = payload_gen(&stage).context("Failed to generate fixture payload")?;
    let metadata =
        metadata_gen(&recipe_path.to_path_buf(), package).context("Failed to generate metadata")?;

    let (priv_key_path, pub_key_path) =
        ensure_keypair(config).context("Failed to prepare fixture signing key")?;
    let signature = sign_artifacts(&priv_key_path, &metadata, &payload, &manifest)
        .context("Failed to sign fixture artifacts")?;

    let output_dir = config.sky_repo.join(package.architecture.to_string());
    fs::create_dir_all(&output_dir).with_context(|| {
        format!(
            "Failed to create fixture sky output directory `{}`",
            output_dir.display()
        )
    })?;
    let sky_path = sky_gen(
        &metadata,
        &payload,
        &manifest,
        &signature,
        package,
        &output_dir,
        config,
    )
    .context("Failed to write fixture sky file")?;

    let rows = row_count(config, &package.name, package.architecture)?;
    ensure!(
        rows == 1,
        "arrange failed: expected one repository row for `{}` ({}), found {rows}",
        package.name,
        package.architecture
    );

    Ok(PublishedPackage {
        package: package.clone(),
        metadata,
        sky_path,
        pub_key_path,
    })
}

fn ensure_keypair(config: &RuntimeConfig) -> Result<(PathBuf, PathBuf)> {
    let key_path = config.recipe_repo.join(".test-keys").join("signer.key");
    if key_path.exists() {
        let pub_key_path = key_path.with_extension("pub");
        return Ok((key_path, pub_key_path));
    }
    keygen(&Some(key_path)).context("Failed to generate fixture signing key")
}

/// Verifies a published fixture archive with the fixture's own public key.
///
/// # Errors
///
/// Returns an error if the archive cannot be read, its metadata does not match
/// the package identity, or the signature does not verify.
pub fn verify_published_sky(published: &PublishedPackage) -> Result<Metadata> {
    verify_sky(
        &published.sky_path,
        &published.pub_key_path,
        &published.package,
    )
}

/// Verifies an arbitrary signed `.sky` against `pub_key_path`.
///
/// # Errors
///
/// Returns an error if the archive cannot be read, its metadata does not match
/// `package`, or the signature does not verify.
pub fn verify_sky(sky_path: &Path, pub_key_path: &Path, package: &Package) -> Result<Metadata> {
    let (metadata_bytes, payload_bytes, files, signature) = read_sky(sky_path)?;
    let identity = PackageIdentity {
        name: package.name.clone(),
        version: package.version.clone(),
        release: package.release,
        architecture: package.architecture,
    };
    let metadata = verify_metadata(&metadata_bytes, &identity)?;
    let data = data_to_sign(&metadata_bytes, &payload_bytes, files.as_bytes())?;
    let settings = SisyphusConfig {
        priv_key_path: PathBuf::new(),
        pub_key_path: pub_key_path.to_path_buf(),
    };
    verify_signature(&data, &settings, &signature)?;
    Ok(metadata)
}

/// One repository row, with the three YAML-list columns decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageRow {
    pub name: String,
    pub version: String,
    pub release: u32,
    pub architecture: String,
    pub sky_path: String,
    pub deps: Vec<String>,
    pub makedeps: Vec<String>,
    pub groups: Vec<String>,
}

type RawRow = (String, String, u32, String, String, String, String, String);

const SELECT_ROW: &str = "SELECT name, version, release, architecture, sky_path, deps, makedeps, groups \
     FROM packages WHERE name = ?1 AND architecture = ?2";

fn open_repo(config: &RuntimeConfig) -> Result<Option<Connection>> {
    if !config.repo_db_path.exists() {
        return Ok(None);
    }
    let conn = Connection::open(&config.repo_db_path)
        .context("Failed to open fixture repository database")?;
    Ok(Some(conn))
}

fn has_packages_table(conn: &Connection) -> Result<bool> {
    let exists: Option<()> = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'packages'",
            [],
            |_| Ok(()),
        )
        .optional()
        .context("Failed to inspect fixture repository schema")?;
    Ok(exists.is_some())
}

fn yaml_list(value: &str) -> Result<Vec<String>> {
    serde_saphyr::from_str(value).context("Failed to decode YAML list from repository row")
}

fn package_row_from_raw(raw: RawRow) -> Result<PackageRow> {
    Ok(PackageRow {
        name: raw.0,
        version: raw.1,
        release: raw.2,
        architecture: raw.3,
        sky_path: raw.4,
        deps: yaml_list(&raw.5)?,
        makedeps: yaml_list(&raw.6)?,
        groups: yaml_list(&raw.7)?,
    })
}

fn raw_query_row(
    conn: &Connection,
    name: &str,
    architecture: Architectures,
) -> Result<Option<RawRow>> {
    conn.query_row(SELECT_ROW, params![name, architecture.to_string()], |row| {
        Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
            row.get(6)?,
            row.get(7)?,
        ))
    })
    .optional()
    .context("Failed to read fixture package row")
}

/// Reads one repository row, or `None` when the DB/table/row does not exist.
///
/// # Errors
///
/// Returns an error if the database cannot be queried or the list columns do not
/// contain valid YAML.
pub fn package_row(
    config: &RuntimeConfig,
    name: &str,
    architecture: Architectures,
) -> Result<Option<PackageRow>> {
    let Some(conn) = open_repo(config)? else {
        return Ok(None);
    };
    if !has_packages_table(&conn)? {
        return Ok(None);
    }
    raw_query_row(&conn, name, architecture)?
        .map(package_row_from_raw)
        .transpose()
}

/// Reads every repository row in a stable order.
///
/// # Errors
///
/// Returns an error if the database cannot be queried or a row cannot be
/// decoded.
pub fn package_rows(config: &RuntimeConfig) -> Result<Vec<PackageRow>> {
    let Some(conn) = open_repo(config)? else {
        return Ok(Vec::new());
    };
    if !has_packages_table(&conn)? {
        return Ok(Vec::new());
    }
    let mut statement = conn
        .prepare(
            "SELECT name, version, release, architecture, sky_path, deps, makedeps, groups \
             FROM packages ORDER BY name, architecture",
        )
        .context("Failed to prepare fixture package query")?;
    let raw = statement
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
            ))
        })
        .context("Failed to query fixture package rows")?
        .collect::<rusqlite::Result<Vec<RawRow>>>()
        .context("Failed to collect fixture package rows")?;

    raw.into_iter().map(package_row_from_raw).collect()
}

/// Counts repository rows for `(name, architecture)`; a missing DB/table is 0.
///
/// # Errors
///
/// Returns an error if an existing database cannot be queried.
pub fn row_count(config: &RuntimeConfig, name: &str, architecture: Architectures) -> Result<i64> {
    let Some(conn) = open_repo(config)? else {
        return Ok(0);
    };
    if !has_packages_table(&conn)? {
        return Ok(0);
    }
    conn.query_row(
        "SELECT COUNT(*) FROM packages WHERE name = ?1 AND architecture = ?2",
        params![name, architecture.to_string()],
        |row| row.get(0),
    )
    .context("Failed to count fixture package rows")
}

/// The decoded group labels for one `(name, architecture)`.
///
/// # Errors
///
/// Returns an error if the row cannot be read or decoded.
pub fn groups_for(
    config: &RuntimeConfig,
    name: &str,
    architecture: Architectures,
) -> Result<Vec<String>> {
    Ok(package_row(config, name, architecture)?
        .map(|row| row.groups)
        .unwrap_or_default())
}

/// The decoded runtime dependencies for one `(name, architecture)`.
///
/// # Errors
///
/// Returns an error if the row cannot be read or decoded.
pub fn deps_for(
    config: &RuntimeConfig,
    name: &str,
    architecture: Architectures,
) -> Result<Vec<String>> {
    Ok(package_row(config, name, architecture)?
        .map(|row| row.deps)
        .unwrap_or_default())
}

/// The decoded build dependencies for one `(name, architecture)`.
///
/// # Errors
///
/// Returns an error if the row cannot be read or decoded.
pub fn makedeps_for(
    config: &RuntimeConfig,
    name: &str,
    architecture: Architectures,
) -> Result<Vec<String>> {
    Ok(package_row(config, name, architecture)?
        .map(|row| row.makedeps)
        .unwrap_or_default())
}

/// Asserts exact group membership for one `(name, architecture)`.
///
/// # Errors
///
/// Returns an error if the observed labels differ, including order.
pub fn assert_groups(
    config: &RuntimeConfig,
    name: &str,
    architecture: Architectures,
    expected: &[&str],
) -> Result<()> {
    let actual = groups_for(config, name, architecture)?;
    let expected = expected
        .iter()
        .map(|group| (*group).to_string())
        .collect::<Vec<_>>();
    ensure!(
        actual == expected,
        "group membership mismatch for `{name}` ({architecture}): expected {expected:?}, got {actual:?}"
    );
    Ok(())
}

/// Asserts the row carries no group labels.
///
/// # Errors
///
/// Returns an error if any label is present.
pub fn assert_no_groups(
    config: &RuntimeConfig,
    name: &str,
    architecture: Architectures,
) -> Result<()> {
    let actual = groups_for(config, name, architecture)?;
    ensure!(
        actual.is_empty(),
        "expected no groups for `{name}` ({architecture}), got {actual:?}"
    );
    Ok(())
}

/// Asserts a label appears exactly once in the row.
///
/// # Errors
///
/// Returns an error if it appears zero or more than once.
pub fn assert_group_recorded_once(
    config: &RuntimeConfig,
    name: &str,
    architecture: Architectures,
    group: &str,
) -> Result<()> {
    let groups = groups_for(config, name, architecture)?;
    let occurrences = groups
        .iter()
        .filter(|candidate| *candidate == group)
        .count();
    ensure!(
        occurrences == 1,
        "group `{group}` must appear exactly once for `{name}` ({architecture}), appeared {occurrences} times in {groups:?}"
    );
    Ok(())
}

/// Asserts a label was not treated as a runtime or build dependency.
///
/// # Errors
///
/// Returns an error if the label appears in `deps` or `makedeps`.
pub fn assert_group_is_not_a_dependency(
    config: &RuntimeConfig,
    name: &str,
    architecture: Architectures,
    group: &str,
) -> Result<()> {
    let row = package_row(config, name, architecture)?
        .with_context(|| format!("package `{name}` ({architecture}) was not published"))?;
    ensure!(
        !row.deps.iter().any(|dep| dep == group) && !row.makedeps.iter().any(|dep| dep == group),
        "group `{group}` leaked into dependency lists for `{name}` ({architecture}): deps={:?}, makedeps={:?}",
        row.deps,
        row.makedeps
    );
    Ok(())
}

/// All `.sky` files currently under the fixture sky repository, sorted.
///
/// # Errors
///
/// Returns an error if the directory tree cannot be walked.
pub fn sky_files(config: &RuntimeConfig) -> Result<Vec<PathBuf>> {
    if !config.sky_repo.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(&config.sky_repo) {
        let entry = entry.context("Failed to walk fixture sky repository")?;
        if entry.file_type().is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "sky")
        {
            files.push(entry.into_path());
        }
    }
    files.sort();
    Ok(files)
}

/// Snapshot of the repository DB rows plus the sky artifacts on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoSnapshot {
    pub rows: Vec<PackageRow>,
    pub sky_files: Vec<PathBuf>,
}

/// Captures the current repository state.
///
/// # Errors
///
/// Returns an error if the rows or sky files cannot be read.
pub fn repo_snapshot(config: &RuntimeConfig) -> Result<RepoSnapshot> {
    Ok(RepoSnapshot {
        rows: package_rows(config)?,
        sky_files: sky_files(config)?,
    })
}

/// Asserts the repository *rows* are byte-for-byte equivalent to `before`.
///
/// Sky files are deliberately excluded: publishing writes the archive before
/// the DB insert, so a failed insert can leave a new artifact behind while the
/// DB correctly rolls back.
///
/// # Errors
///
/// Returns an error if the rows differ.
pub fn assert_repo_db_unchanged(config: &RuntimeConfig, before: &RepoSnapshot) -> Result<()> {
    let after = repo_snapshot(config)?;
    ensure!(
        after.rows == before.rows,
        "repository DB changed after a failed publish\nbefore: {:?}\nafter: {:?}",
        before.rows,
        after.rows
    );
    Ok(())
}

/// Asserts a directory does not exist or has no entries.
///
/// # Errors
///
/// Returns an error if the directory cannot be read.
pub fn directory_is_empty(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(true);
    }
    Ok(fs::read_dir(path)
        .with_context(|| format!("Failed to read directory `{}`", path.display()))?
        .next()
        .is_none())
}

/// Asserts validation failed before fetch, extraction, artifact creation, or a
/// repository write.
///
/// The recipe directory is intentionally not checked: writing the recipe is the
/// test's arrange, not a publish side effect.
///
/// # Errors
///
/// Returns an error if any publish-side path was touched.
pub fn assert_no_publish_side_effects(config: &RuntimeConfig) -> Result<()> {
    ensure!(
        !config.repo_db_path.exists(),
        "repository DB was created at `{}` before validation completed",
        config.repo_db_path.display()
    );
    let skies = sky_files(config)?;
    ensure!(
        skies.is_empty(),
        "sky artifacts were written before validation completed: {skies:?}"
    );
    ensure!(
        directory_is_empty(&config.srcdir)?,
        "source staging directory `{}` was touched before validation completed",
        config.srcdir.display()
    );
    ensure!(
        directory_is_empty(&config.fetch_cache)?,
        "fetch cache `{}` was touched before validation completed",
        config.fetch_cache.display()
    );
    ensure!(
        !config.cache_db_path.exists(),
        "cache DB was created at `{}` before validation completed",
        config.cache_db_path.display()
    );
    Ok(())
}

/// Installs a SQLite trigger that aborts every `INSERT` into `table`.
///
/// The trigger name is derived from the table so multiple tables can be failed
/// in one fixture directory.
///
/// # Errors
///
/// Returns an error if the table does not exist or the trigger cannot be
/// created.
pub fn fail_inserts_into(config: &RuntimeConfig, table: &str) -> Result<()> {
    let mut conn = Connection::open(&config.repo_db_path)
        .context("Failed to open fixture repository database")?;
    if table == "packages" {
        create_packages_table(&mut conn)?;
    }

    let trigger = format!("package_groups_test_fail_{table}_insert");
    conn.execute_batch(&format!(
        "CREATE TRIGGER IF NOT EXISTS {trigger} BEFORE INSERT ON {table} \
         BEGIN SELECT RAISE(ABORT, 'forced group membership write failure'); END"
    ))
    .with_context(|| format!("Failed to install failure trigger on `{table}`"))?;
    Ok(())
}

/// Forces the next `packages` insert to fail.
///
/// The current writer stores groups inline in `packages.groups`, so the group
/// membership write *is* the package-row insert. If a separate membership table
/// is introduced later, call `fail_inserts_into` with that table instead.
///
/// # Errors
///
/// Returns an error if the repository DB or trigger cannot be written.
pub fn fail_package_row_inserts(config: &RuntimeConfig) -> Result<()> {
    fail_inserts_into(config, "packages")
}

#[test]
fn a_package_with_a_group_produces_a_db_row_with_the_group() -> Result<()> {
    let tmp = tempdir()?;
    let config = RuntimeConfig::build_at(tmp.path())?;

    let recipe = RecipeFixture::new("foo", "1.0.0").with_groups([GROUP]);

    let published = publish_recipe(&config, &recipe)?;

    assert_groups(&config, "foo", Architectures::X86_64, &[GROUP])?;
    assert_group_recorded_once(&config, "foo", Architectures::X86_64, GROUP)?;

    assert_group_is_not_a_dependency(&config, "foo", Architectures::X86_64, GROUP)?;

    let metadata = verify_published_sky(&published)?;
    assert_eq!(metadata.name, "foo");
    assert_eq!(metadata.version, "1.0.0");
    assert_eq!(metadata.release, 1);
    assert_eq!(metadata.architecture, Architectures::X86_64);

    assert_metadata_omits_groups(&published.package)?;

    Ok(())
}

#[test]
fn group_membership_does_not_change_runtime_or_build_dependencies() -> Result<()> {
    let tmp = tempdir()?;
    let config = RuntimeConfig::build_at(tmp.path())?;

    let recipe = RecipeFixture::new("foo", "1.0.0")
        .with_groups([GROUP])
        .with_deps(["hello"])
        .with_makedeps(["makehello"]);
    let published = publish_recipe(&config, &recipe)?;

    assert_group_is_not_a_dependency(&config, "foo", Architectures::X86_64, GROUP)?;

    let metadata = verify_published_sky(&published)?;
    assert_eq!(metadata.name, "foo");
    assert_eq!(metadata.version, "1.0.0");
    assert_eq!(metadata.release, 1);
    assert_eq!(metadata.architecture, Architectures::X86_64);
    assert_eq!(metadata.deps, vec!["hello"]);

    // The build plan and the repository row must read dependencies from the
    // dependency fields, never from `groups`.
    assert_eq!(
        deps_for(&config, "foo", Architectures::X86_64)?,
        vec!["hello".to_string()],
        "runtime dependencies must come from `deps`, not `groups`"
    );
    assert_eq!(
        makedeps_for(&config, "foo", Architectures::X86_64)?,
        vec!["makehello".to_string()],
        "build dependencies must come from `makedeps`, not `groups`"
    );

    Ok(())
}

#[test]
fn republishing_a_package_replaces_its_old_group_memberships() -> Result<()> {
    let tmp = tempdir()?;
    let config = RuntimeConfig::build_at(tmp.path())?;

    let recipe = RecipeFixture::new("foo", "1.0.0")
        .with_groups([GROUP])
        .with_deps(["hello"])
        .with_makedeps(["makehello"]);
    let published = publish_recipe(&config, &recipe)?;

    assert_group_is_not_a_dependency(&config, "foo", Architectures::X86_64, GROUP)?;

    let metadata = verify_published_sky(&published)?;
    assert_eq!(metadata.name, "foo");
    assert_eq!(metadata.version, "1.0.0");
    assert_eq!(metadata.release, 1);
    assert_eq!(metadata.architecture, Architectures::X86_64);
    assert_eq!(metadata.deps, vec!["hello"]);
    assert_eq!(recipe.makedeps, vec!["makehello"]);

    let updated = RecipeFixture::new("foo", "2.0.0")
        .with_groups([OTHER_GROUP])
        .with_deps(["hello"])
        .with_makedeps(["makehello"]);
    publish_recipe(&config, &updated)?;

    assert_group_is_not_a_dependency(&config, "foo", Architectures::X86_64, OTHER_GROUP)?;
    assert_groups(&config, "foo", Architectures::X86_64, &[OTHER_GROUP])?;
    assert!(assert_groups(&config, "foo", Architectures::X86_64, &[GROUP]).is_err());

    Ok(())
}

#[test]
fn a_recipe_with_multiple_groups_publishes_each_membership_once() -> Result<()> {
    let tmp = tempdir()?;
    let config = RuntimeConfig::build_at(tmp.path())?;

    let recipe = RecipeFixture::new("foo", "1.0.0").with_groups([GROUP, OTHER_GROUP]);
    publish_recipe(&config, &recipe)?;

    assert_groups(&config, "foo", X86_64, &[GROUP, OTHER_GROUP])?;
    for group in [GROUP, OTHER_GROUP] {
        assert_group_recorded_once(&config, "foo", X86_64, group)?;
        assert_group_is_not_a_dependency(&config, "foo", X86_64, group)?;
    }

    Ok(())
}

#[test]
fn a_recipe_without_groups_remains_a_valid_ungrouped_package() -> Result<()> {
    let tmp = tempdir()?;
    let config = RuntimeConfig::build_at(tmp.path())?;

    let recipe = RecipeFixture::new("foo", "1.0.0");
    assert!(
        !recipe.to_yaml().contains("groups:"),
        "this fixture must omit the `groups` key entirely"
    );

    let parsed = recipe.parse(&config)?;
    assert!(
        parsed.groups.is_empty(),
        "a missing `groups` key must default to an empty list, got {:?}",
        parsed.groups
    );

    let published = publish_recipe(&config, &recipe)?;
    assert_no_groups(&config, "foo", X86_64)?;
    assert_eq!(published.metadata.version, "1.0.0");

    Ok(())
}

#[test]
fn invalid_or_duplicate_group_names_are_rejected_before_build_side_effects() -> Result<()> {
    let tmp = tempdir()?;
    let config = RuntimeConfig::build_at(tmp.path())?;

    let cases = invalid_group_names()
        .into_iter()
        .map(|group| (format!("invalid group {group:?}"), vec![group]))
        .chain(std::iter::once((
            "duplicate groups".to_string(),
            duplicate_groups(),
        )));

    for (label, groups) in cases {
        let recipe = RecipeFixture::new("foo", "1.0.0").with_groups(groups);
        let error = publish_recipe(&config, &recipe)
            .err()
            .with_context(|| format!("recipe with {label} must be rejected"))?;
        let message = format!("{error:#}");
        assert!(
            message.to_lowercase().contains("group"),
            "case `{label}` must fail as a group-validation error, got: {message}"
        );

        assert_no_publish_side_effects(&config)
            .with_context(|| format!("case `{label}` touched publish state before failing"))?;
    }

    Ok(())
}

#[test]
fn publishing_the_same_group_for_two_architectures_keeps_memberships_separate() -> Result<()> {
    let tmp = tempdir()?;
    let config = RuntimeConfig::build_at(tmp.path())?;

    // Both architectures carry GROUP; x86_64 additionally carries
    // OTHER_GROUP so a lookup that merged both rows would be visible.
    let x86 = package(
        "foo",
        "1.0.0",
        X86_64,
        vec![GROUP.to_string(), OTHER_GROUP.to_string()],
    );
    let arm = package("foo", "1.0.0", ARM, vec![GROUP.to_string()]);

    publish_stub(&config, &x86)?;
    publish_stub(&config, &arm)?;

    assert_groups(&config, "foo", X86_64, &[GROUP, OTHER_GROUP])?;
    assert_groups(&config, "foo", ARM, &[GROUP])?;
    assert_eq!(row_count(&config, "foo", X86_64)?, 1);
    assert_eq!(row_count(&config, "foo", ARM)?, 1);

    Ok(())
}

#[test]
fn failed_group_index_insertion_rolls_back_package_and_membership_rows() -> Result<()> {
    let tmp = tempdir()?;
    let config = RuntimeConfig::build_at(tmp.path())?;

    let old = package("foo", "1.0.0", X86_64, vec![GROUP.to_string()]);
    let old_sky = publish_stub(&config, &old)?;
    let before = repo_snapshot(&config)?;
    assert_eq!(before.rows.len(), 1);
    assert_eq!(before.sky_files, vec![old_sky.clone()]);

    // Current schema stores membership inline in the package row, so making
    // that row's INSERT fail is exactly the membership-write failure.
    fail_package_row_inserts(&config)?;

    let new = package("foo", "2.0.0", X86_64, vec![OTHER_GROUP.to_string()]);
    let new_sky = sky_path_for(&config, &new);
    let error = publish_stub(&config, &new)
        .expect_err("the forced repository insert failure must abort the publish");
    let message = format!("{error:#}");
    assert!(
        message.contains("forced group membership write failure"),
        "the abort must come from the forced membership write, got: {message}"
    );

    // The package row and its old membership must survive the rollback.
    assert_repo_db_unchanged(&config, &before)?;
    assert_eq!(
        package_row(&config, "foo", X86_64)?
            .expect("the previous package row must survive rollback")
            .sky_path,
        old_sky.to_str().expect("fixture sky path must be UTF-8")
    );
    assert_groups(&config, "foo", X86_64, &[GROUP])?;

    // The new .sky is written before the DB insert, so it is expected to stay.
    let after = repo_snapshot(&config)?;
    assert_eq!(
        after.sky_files.len(),
        2,
        "the pre-insert .sky must remain alongside the old one: {:?}",
        after.sky_files
    );
    assert!(after.sky_files.contains(&new_sky));
    assert!(after.sky_files.contains(&old_sky));

    Ok(())
}
