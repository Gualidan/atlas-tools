#![allow(dead_code)]

use std::{
    collections::BTreeMap,
    env,
    ffi::OsString,
    fmt::Write as _,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use anyhow::Result;
use atlas::{
    cli::cli::UpdateArgs,
    install::install::install,
    types::runtime_config::RuntimeConfig,
    uninstall::{resolve::UninstallReport, uninstall::uninstall},
    update::update::update,
};
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
use walkdir::WalkDir;
use zstd::Encoder;

pub const ARCH: Architectures = Architectures::X86_64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PayloadEntry {
    File {
        path: String,
        mode: u32,
        contents: Vec<u8>,
    },
    Directory {
        path: String,
        mode: u32,
    },
    Symlink {
        path: String,
        target: String,
    },
}

impl PayloadEntry {
    pub fn file(path: impl Into<String>, contents: impl Into<Vec<u8>>) -> Self {
        Self::file_with_mode(path, 0o644, contents)
    }

    pub fn file_with_mode(
        path: impl Into<String>,
        mode: u32,
        contents: impl Into<Vec<u8>>,
    ) -> Self {
        Self::File {
            path: path.into(),
            mode,
            contents: contents.into(),
        }
    }

    pub fn directory(path: impl Into<String>) -> Self {
        Self::directory_with_mode(path, 0o755)
    }

    pub fn directory_with_mode(path: impl Into<String>, mode: u32) -> Self {
        Self::Directory {
            path: path.into(),
            mode,
        }
    }

    pub fn symlink(path: impl Into<String>, target: impl Into<String>) -> Self {
        Self::Symlink {
            path: path.into(),
            target: target.into(),
        }
    }

    fn path(&self) -> &str {
        match self {
            Self::File { path, .. } | Self::Directory { path, .. } | Self::Symlink { path, .. } => {
                path
            }
        }
    }
}

#[allow(dead_code)]
#[allow(clippy::struct_field_names)]
pub struct Installed {
    pub version: String,
    pub release: u32,
    pub sky_path: String,
    pub install_dir: PathBuf,
    pub installed_at: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct UpdateLogRow {
    pub id: i64,
    pub name: String,
    pub architecture: String,
    pub version: String,
    pub release: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TreeEntry {
    Directory,
    File(Vec<u8>),
    Symlink(PathBuf),
    Other,
}

/// Captures the install database bytes and the full target-root tree.
///
/// Used by "the operation must leave everything unchanged" tests, where
/// comparing one hand-picked file is not enough.
#[derive(Debug, PartialEq, Eq)]
pub struct InstallStateSnapshot {
    pub database: Option<Vec<u8>>,
    pub root: BTreeMap<PathBuf, TreeEntry>,
}

pub fn snapshot_tree(root: &Path) -> Result<BTreeMap<PathBuf, TreeEntry>> {
    let mut snapshot = BTreeMap::new();
    if !root.exists() {
        return Ok(snapshot);
    }

    for entry in WalkDir::new(root).min_depth(1) {
        let entry = entry?;
        let relative_path = entry.path().strip_prefix(root)?.to_path_buf();
        let entry_type = entry.file_type();
        let snapshot_entry = if entry_type.is_dir() {
            TreeEntry::Directory
        } else if entry_type.is_symlink() {
            TreeEntry::Symlink(fs::read_link(entry.path())?)
        } else if entry_type.is_file() {
            TreeEntry::File(fs::read(entry.path())?)
        } else {
            TreeEntry::Other
        };
        snapshot.insert(relative_path, snapshot_entry);
    }
    Ok(snapshot)
}

/// Captures install state for a root that may not exist yet.
pub fn install_state_snapshot(
    config: &RuntimeConfig,
    target_root: &Path,
) -> Result<InstallStateSnapshot> {
    let database = if config.install_db.exists() {
        Some(fs::read(&config.install_db)?)
    } else {
        None
    };

    Ok(InstallStateSnapshot {
        database,
        root: snapshot_tree(target_root)?,
    })
}

/// Asserts that a rejected command created neither an install database nor any
/// target-root content, including staging directories.
pub fn assert_install_state_absent(config: &RuntimeConfig, target_root: &Path) {
    assert!(
        !config.install_db.exists(),
        "install database must not be created by a rejected command"
    );

    let empty = match fs::read_dir(target_root) {
        Ok(mut entries) => entries.next().is_none(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => panic!(
            "failed to inspect target root {}: {error}",
            target_root.display()
        ),
    };
    assert!(
        empty,
        "target root {} must stay empty; staging directories are forbidden",
        target_root.display()
    );
}

#[allow(dead_code)]
pub struct Workspace {
    tmp: TempDir,
    pub config: RuntimeConfig,
    root: PathBuf,
    keypair: Ed25519KeyPair,
    _env: ConfigEnv,
}

#[allow(dead_code)]
impl Workspace {
    pub fn new() -> Result<Self> {
        let tmp = tempdir();
        let config = runtime_config(&tmp)?;
        let (keypair, public_key) = generate_keypair()?;
        let env = ConfigEnv::new(&tmp, &public_key)?;
        let root = tmp.path().join("root");
        fs::create_dir(&root)?;

        Ok(Self {
            tmp,
            config,
            root,
            keypair,
            _env: env,
        })
    }

    pub fn publish(
        &self,
        name: &str,
        version: &str,
        release: u32,
        deps: &[&str],
    ) -> Result<PathBuf> {
        let sky = write_signed_sky_with_file(
            &self.tmp,
            &self.keypair,
            &format!("{name}-{version}-{release}.sky"),
            name,
            version,
            release,
            format!("{name} {version}\\n").as_bytes(),
            deps.iter().map(ToString::to_string).collect(),
            name,
            &format!("{version}.txt"),
        )?;
        seed_repo_db_with_deps(
            &self.config.repo_db,
            &[(name, version, release, sky.to_str().unwrap(), deps)],
        );
        Ok(sky)
    }

    pub fn publish_with_entries(
        &self,
        name: &str,
        version: &str,
        release: u32,
        deps: &[&str],
        entries: &[PayloadEntry],
    ) -> Result<PathBuf> {
        let sky = write_signed_sky_with_entries(
            &self.tmp,
            &self.keypair,
            &format!("{name}-{version}-{release}.sky"),
            name,
            version,
            release,
            deps.iter().map(ToString::to_string).collect(),
            entries,
        )?;
        seed_repo_db_with_deps(
            &self.config.repo_db,
            &[(name, version, release, sky.to_str().unwrap(), deps)],
        );
        Ok(sky)
    }

    /// Like [`Self::publish_with_entries`], but the repository row also belongs
    /// to `groups`. Use this for a group member whose payload must fail
    /// verification or commit after earlier members were installed.
    pub fn publish_with_entries_in_groups(
        &self,
        name: &str,
        version: &str,
        release: u32,
        deps: &[&str],
        entries: &[PayloadEntry],
        groups: &[&str],
    ) -> Result<PathBuf> {
        let sky = write_signed_sky_with_entries(
            &self.tmp,
            &self.keypair,
            &format!("{name}-{version}-{release}.sky"),
            name,
            version,
            release,
            deps.iter().map(ToString::to_string).collect(),
            entries,
        )?;
        seed_repo_db_with_deps_and_groups(
            &self.config.repo_db,
            &[(name, version, release, sky.to_str().unwrap(), deps, groups)],
        );
        Ok(sky)
    }

    /// Publishes a package whose repository row belongs to `groups`.
    ///
    /// Group membership lives in the repository row, not in the `.sky`
    /// metadata, so the payload is the ordinary one-file fixture package.
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
            format!("{name} {version}\\n").as_bytes(),
            deps.iter().map(ToString::to_string).collect(),
            name,
            &format!("{version}.txt"),
        )?;
        seed_repo_db_with_deps_and_groups(
            &self.config.repo_db,
            &[(name, version, release, sky.to_str().unwrap(), deps, groups)],
        );
        Ok(sky)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn target_path(&self, relative_path: impl AsRef<Path>) -> PathBuf {
        self.root.join(relative_path)
    }

    pub fn create_target_root(&self, name: &str) -> Result<PathBuf> {
        let root = self.tmp.path().join(name);
        fs::create_dir_all(&root)?;
        Ok(root)
    }

    pub fn install(&self, name: &str) -> Result<()> {
        install(Some(name), None, &self.config, &self.root)
    }

    pub fn install_into(&self, name: &str, target_root: &Path) -> Result<()> {
        install(Some(name), None, &self.config, &target_root.to_path_buf())
    }

    pub fn install_group(&self, group: &str) -> Result<()> {
        install(None, Some(group), &self.config, &self.root)
    }

    pub fn install_group_into(&self, group: &str, target_root: &Path) -> Result<()> {
        install(None, Some(group), &self.config, &target_root.to_path_buf())
    }

    pub fn uninstall(&self, name: &str) -> Result<UninstallReport> {
        uninstall(&self.config, &name.to_string(), false)
    }

    pub fn update(&self, name: &str) -> Result<()> {
        update(
            &self.config,
            &UpdateArgs {
                package: Some(name.into()),
                group: None,
            },
        )
    }

    pub fn update_groups(&self, group: &str) -> Result<()> {
        update(
            &self.config,
            &UpdateArgs {
                package: None,
                group: Some(group.into()),
            },
        )
    }

    pub fn install_conn(&self) -> Result<Connection> {
        Ok(Connection::open(&self.config.install_db)?)
    }

    pub fn owned_paths(&self, name: &str) -> Result<Vec<String>> {
        let conn = self.install_conn()?;
        owned_paths(&conn, name)
    }

    pub fn repo_conn(&self) -> Result<Connection> {
        Ok(Connection::open(&self.config.repo_db)?)
    }

    pub fn row(&self, name: &str) -> Result<Installed> {
        let conn = self.install_conn()?;
        let (version, release, sky_path, install_dir, installed_at, install_reason): (
            String,
            u32,
            String,
            String,
            String,
            String,
        ) = conn.query_row(
            "SELECT version, release, sky_path, install_dir, installed_at, install_reason
             FROM installed_packages WHERE name = ? AND architecture = ?",
            params![name, ARCH.to_string()],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )?;
        Ok(Installed {
            version,
            release,
            sky_path,
            install_dir: PathBuf::from(install_dir),
            installed_at,
            reason: install_reason,
        })
    }

    pub fn set_metadata(&self, name: &str, installed_at: &str, reason: &str) -> Result<()> {
        self.install_conn()?.execute(
            "UPDATE installed_packages SET installed_at = ?, install_reason = ?
             WHERE name = ? AND architecture = ?",
            params![installed_at, reason, name, ARCH.to_string()],
        )?;
        Ok(())
    }

    pub fn deps(&self, name: &str) -> Result<Vec<String>> {
        let conn = self.install_conn()?;
        let mut stmt = conn.prepare(
            "SELECT dependency_name FROM package_dependencies
             WHERE package_name = ? AND architecture = ? ORDER BY dependency_name",
        )?;
        Ok(stmt
            .query_map(params![name, ARCH.to_string()], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?)
    }

    /// Returns packages in the order their install rows were recorded.
    ///
    /// `installed_packages` is a rowid table, so `rowid` preserves commit
    /// order. Group installs must record dependencies before the members that
    /// need them.
    pub fn install_order(&self) -> Result<Vec<String>> {
        let conn = self.install_conn()?;
        let mut stmt = conn.prepare("SELECT name FROM installed_packages ORDER BY rowid")?;
        Ok(stmt
            .query_map([], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?)
    }

    pub fn assert_installed(&self, name: &str, version: &str, release: u32) -> Result<Installed> {
        let row = self.row(name)?;
        assert_eq!(row.version, version, "{name} version");
        assert_eq!(row.release, release, "{name} release");

        let payload = row.install_dir.join(name);
        let mut files = fs::read_dir(&payload)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<std::io::Result<Vec<OsString>>>()?;
        files.sort();
        assert_eq!(
            files,
            vec![OsString::from(format!("{version}.txt"))],
            "{name} payload files"
        );
        assert_eq!(
            fs::read(payload.join(format!("{version}.txt")))?,
            format!("{name} {version}\\n").as_bytes(),
            "{name} payload contents"
        );

        let conn = self.install_conn()?;
        let mut stmt = conn.prepare(
            "SELECT path FROM owned_files
             WHERE package_name = ? AND architecture = ? ORDER BY path",
        )?;
        let owned = stmt
            .query_map(params![name, ARCH.to_string()], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<String>>>()?;
        let expected_owned = if owned.iter().any(|path| path == name) {
            vec![name.to_string(), format!("{name}/{version}.txt")]
        } else {
            vec![format!("{name}/{version}.txt")]
        };
        assert_eq!(owned, expected_owned, "{name} owned files");
        Ok(row)
    }

    pub fn assert_not_installed(&self, name: &str) {
        assert!(self.row(name).is_err(), "{name} should not be installed");
    }

    pub fn assert_deps(&self, name: &str, expected: &[&str]) -> Result<()> {
        let mut expected = expected.iter().map(ToString::to_string).collect::<Vec<_>>();
        expected.sort();
        assert_eq!(self.deps(name)?, expected, "{name} dependencies");
        Ok(())
    }

    pub fn assert_no_staging_dirs(&self) -> Result<()> {
        let conn = self.install_conn()?;
        let mut stmt = conn.prepare("SELECT name FROM installed_packages")?;
        let mut expected = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(|name| self.root.join(name))
            .collect::<Vec<_>>();
        if self.root.join("var/lib/atlas/tmp").exists() {
            expected.push(self.root.join("var"));
            assert_eq!(
                fs::read_dir(self.root.join("var/lib/atlas/tmp"))?.count(),
                0
            );
        }
        expected.sort();
        let mut actual = fs::read_dir(&self.root)?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<std::io::Result<Vec<_>>>()?;
        actual.sort();
        assert_eq!(
            actual, expected,
            "temporary staging directories must be removed"
        );
        Ok(())
    }

    /// Asserts the command under test left no install state and an empty root.
    pub fn assert_install_state_absent(&self) {
        assert_install_state_absent(&self.config, &self.root);
    }

    /// Asserts the command under test left no install state at `target_root`.
    pub fn assert_install_state_absent_at(&self, target_root: &Path) {
        assert_install_state_absent(&self.config, target_root);
    }

    pub fn install_state_snapshot(&self) -> Result<InstallStateSnapshot> {
        install_state_snapshot(&self.config, &self.root)
    }

    /// Asserts every recorded dependency edge still points at an installed
    /// package. Partial group installs must not leave orphaned edges behind.
    pub fn assert_no_dangling_dependencies(&self) -> Result<()> {
        let conn = self.install_conn()?;
        let mut stmt = conn.prepare(
            "SELECT d.package_name, d.dependency_name, d.architecture
             FROM package_dependencies d
             LEFT JOIN installed_packages p
               ON p.name = d.dependency_name AND p.architecture = d.architecture
             WHERE p.name IS NULL
             ORDER BY d.package_name, d.dependency_name",
        )?;
        let dangling = stmt
            .query_map([], |row| {
                Ok(format!(
                    "{} -> {} ({})",
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        assert!(
            dangling.is_empty(),
            "dangling dependency edges: {dangling:?}"
        );
        Ok(())
    }

    pub fn get_update_log(&self) -> Result<Vec<UpdateLogRow>> {
        let conn = self.install_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, name, architecture, version, release
             FROM installed_packages_update_log ORDER BY id ASC",
        )?;
        Ok(stmt
            .query_map([], |row| {
                Ok(UpdateLogRow {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    architecture: row.get(2)?,
                    version: row.get(3)?,
                    release: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

/// Builds a temporary directory in the platform's configured temporary area.
/// Install staging directories are created beside their target directory, so
/// atomic exchanges remain on the target's filesystem.
pub fn tempdir() -> TempDir {
    TempBuilder::new()
        .prefix("atlas_test_")
        .tempdir()
        .expect("failed to create test temporary directory")
}

pub fn runtime_config(tmp: &TempDir) -> Result<RuntimeConfig> {
    RuntimeConfig::build_at(tmp.path())
}

pub fn count(conn: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) -> i64 {
    conn.query_row(sql, params, |row| row.get(0)).unwrap()
}

pub fn owned_paths(conn: &Connection, package_name: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT path FROM owned_files WHERE package_name = ? AND architecture = ? ORDER BY path",
    )?;
    Ok(stmt
        .query_map(params![package_name, ARCH.to_string()], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn read_target_file(root: &Path, relative_path: impl AsRef<Path>) -> Result<Vec<u8>> {
    Ok(fs::read(root.join(relative_path))?)
}

/// One repository row for seeding. `deps` and `groups` are serialized exactly
/// the way Sisyphus publishes them.
pub struct RepoRow<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub release: u32,
    pub architecture: Architectures,
    pub sky_path: &'a str,
    pub deps: &'a [&'a str],
    pub groups: &'a [&'a str],
}

pub fn seed_repo_db_rows(path: &Path, packages: &[RepoRow<'_>]) {
    let mut conn = Connection::open(path).unwrap();
    create_packages_table(&mut conn).unwrap();

    for package in packages {
        let deps = serialize_saphyr_list(package.deps);
        let groups = serialize_saphyr_list(package.groups);
        let architecture = package.architecture.to_string();

        conn.execute(
            "INSERT OR REPLACE INTO packages (name, version, release, architecture, sky_path, recipe_sha256, built_at, builder, deps, makedeps, groups) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![
                package.name,
                package.version,
                package.release,
                architecture,
                package.sky_path,
                "deadbeef",
                "2026-01-01T00:00:00Z",
                "tester",
                &deps,
                "[]",
                &groups
            ],
        )
        .unwrap();

        // Read back the arrange so a wrong column order or architecture cannot
        // turn a group test into a vacuous assertion about an empty plan.
        let (stored_deps, stored_groups): (String, String) = conn
            .query_row(
                "SELECT deps, groups FROM packages WHERE name = ? AND architecture = ?",
                params![package.name, architecture],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(stored_deps, deps, "{} repository deps", package.name);
        assert_eq!(stored_groups, groups, "{} repository groups", package.name);
    }
}

fn serialize_saphyr_list(values: &[&str]) -> String {
    let values = values.iter().map(ToString::to_string).collect::<Vec<_>>();
    serde_saphyr::to_string(&values).expect("failed to serialize test list to YAML")
}

pub fn seed_repo_db(path: &Path, packages: &[(&str, &str, u32, &str)]) {
    let rows = packages
        .iter()
        .map(|&(name, version, release, sky_path)| RepoRow {
            name,
            version,
            release,
            architecture: ARCH,
            sky_path,
            deps: &[],
            groups: &[],
        })
        .collect::<Vec<_>>();

    seed_repo_db_rows(path, &rows);
}

/// Seeds the repository database with runtime dependency lists exactly the way
/// Sisyphus publishes them: the `deps` column is serialized with
/// `serde_saphyr`, never hand-written YAML.
pub fn seed_repo_db_with_deps(path: &Path, packages: &[(&str, &str, u32, &str, &[&str])]) {
    let rows = packages
        .iter()
        .map(|&(name, version, release, sky_path, deps)| RepoRow {
            name,
            version,
            release,
            architecture: ARCH,
            sky_path,
            deps,
            groups: &[],
        })
        .collect::<Vec<_>>();

    seed_repo_db_rows(path, &rows);
}

/// Tuple shape for grouped repository fixtures:
/// `(name, version, release, sky_path, deps, groups)`.
pub type GroupedRepoPackage<'a> = (&'a str, &'a str, u32, &'a str, &'a [&'a str], &'a [&'a str]);

/// Seeds host-architecture packages with explicit group membership.
///
/// Tuple shape: `(name, version, release, sky_path, deps, groups)`.
pub fn seed_repo_db_with_deps_and_groups(path: &Path, packages: &[GroupedRepoPackage<'_>]) {
    seed_repo_db_with_deps_and_groups_for_arch(path, ARCH, packages);
}

/// Seeds repository rows for an arbitrary architecture.
///
/// Group selection must use the requested architecture, so tests need rows for
/// a different architecture without a host-installable `.sky`. Keep `sky_path`
/// unique: it is the repository table's primary key.
pub fn seed_repo_db_with_deps_and_groups_for_arch(
    path: &Path,
    architecture: Architectures,
    packages: &[GroupedRepoPackage<'_>],
) {
    let rows = packages
        .iter()
        .map(
            |&(name, version, release, sky_path, deps, groups)| RepoRow {
                name,
                version,
                release,
                architecture,
                sky_path,
                deps,
                groups,
            },
        )
        .collect::<Vec<_>>();

    seed_repo_db_rows(path, &rows);
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
    let data = data_to_sign(&metadata_bytes, &payload, files.as_bytes())?;
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

/// Builds a payload archive and sorted manifest from arbitrary regular files,
/// directories, and symlinks. Parent directories are added automatically with
/// mode 0755 unless an explicit directory entry supplies another mode.
pub fn build_payload_and_manifest(entries: &[PayloadEntry]) -> Result<(Vec<u8>, String)> {
    let mut by_path = BTreeMap::<String, PayloadEntry>::new();
    for entry in entries {
        let path = entry.path().to_owned();
        if by_path.insert(path.clone(), entry.clone()).is_some() {
            anyhow::bail!("duplicate test payload path {path}");
        }
    }

    let paths = by_path.keys().cloned().collect::<Vec<_>>();
    for path in paths {
        let mut parent = Path::new(&path).parent();
        while let Some(parent_path) = parent {
            let parent_name = parent_path.to_string_lossy().into_owned();
            if parent_name.is_empty() || parent_name == "." {
                break;
            }

            match by_path.get(&parent_name) {
                Some(PayloadEntry::Directory { .. }) => {}
                Some(_) => {
                    anyhow::bail!("test payload parent {parent_name} is not a directory");
                }
                None => {
                    by_path.insert(parent_name.clone(), PayloadEntry::directory(&parent_name));
                }
            }
            parent = parent_path.parent();
        }
    }

    let mut payload = Vec::new();
    let encoder = Encoder::new(&mut payload, 3)?;
    let mut archive = Builder::new(encoder);
    let mut manifest = String::new();

    for (path, entry) in by_path {
        match entry {
            PayloadEntry::File { mode, contents, .. } => {
                let mut header = Header::new_gnu();
                header.set_mode(mode);
                header.set_size(contents.len() as u64);
                archive.append_data(&mut header, &path, Cursor::new(&contents))?;
                let file_hash = hex::encode(hash(Cursor::new(&contents))?);
                writeln!(&mut manifest, "f\t{mode:04o}\t{file_hash}\t{path}")?;
            }
            PayloadEntry::Directory { mode, .. } => {
                let mut header = Header::new_gnu();
                header.set_mode(mode);
                header.set_size(0);
                header.set_entry_type(EntryType::dir());
                archive.append_data(&mut header, &path, std::io::empty())?;
                writeln!(&mut manifest, "d\t{mode:04o}\t-\t{path}")?;
            }
            PayloadEntry::Symlink { target, .. } => {
                let mut header = Header::new_gnu();
                header.set_mode(0o777);
                header.set_size(0);
                header.set_entry_type(EntryType::symlink());
                archive.append_link(&mut header, &path, &target)?;
                writeln!(&mut manifest, "l\t0777\t{target}\t{path}")?;
            }
        }
    }

    archive.into_inner()?.finish()?;
    Ok((payload, manifest))
}

/// Builds a signed package with arbitrary regular files, directories, and
/// symlinks. Parent directories are synthesized by
/// [`build_payload_and_manifest`].
#[allow(clippy::too_many_arguments)]
pub fn write_signed_sky_with_entries(
    tmp: &TempDir,
    keypair: &Ed25519KeyPair,
    sky_name: &str,
    package_name: &str,
    version: &str,
    release: u32,
    deps: Vec<String>,
    entries: &[PayloadEntry],
) -> Result<PathBuf> {
    let (payload, files) = build_payload_and_manifest(entries)?;
    let metadata_bytes =
        serde_saphyr::to_string(&metadata_for(package_name, version, release, deps))?.into_bytes();
    let data = data_to_sign(&metadata_bytes, &payload, files.as_bytes())?;
    let signature = keypair.sign(&data).as_ref().to_vec();
    let sky_path = tmp.path().join(sky_name);
    write_sky(&sky_path, &payload, &metadata_bytes, &files, &signature)?;
    Ok(sky_path)
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
        "f\t0644\t{file_hash}\t{dir}/{file_name}\nd\t0755\t-\t{dir}\n"
    ))
}

/// Builds the manifest matching [`build_payload_with_symlink`].
pub fn manifest_for_symlink(dir: &str, link_name: &str, target: &str) -> String {
    format!("l\t0777\t{target}\t{dir}/{link_name}\nd\t0755\t-\t{dir}\n")
}

#[allow(clippy::too_many_arguments)]
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
    let data = data_to_sign(&metadata_bytes, &payload, files.as_bytes())?;
    let signature = keypair.sign(&data).as_ref().to_vec();

    let sky_path = tmp.path().join(sky_name);
    write_sky(&sky_path, &payload, &metadata_bytes, &files, &signature)?;
    Ok(sky_path)
}

/// Like [`write_signed_sky`] but with a custom payload directory name, so two
/// packages installed together don't conflict on the same paths.
#[allow(clippy::too_many_arguments)]
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
    let data = data_to_sign(&metadata_bytes, &payload, files.as_bytes())?;
    let signature = keypair.sign(&data).as_ref().to_vec();

    let sky_path = tmp.path().join(sky_name);
    write_sky(&sky_path, &payload, &metadata_bytes, &files, &signature)?;
    Ok(sky_path)
}

/// Like [`write_signed_sky`] but with a custom payload directory and file
/// name, so two packages can share a directory without colliding on a file.
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

/// Like [`write_signed_sky`] but shipping a single symlink instead of a
/// regular file.
#[allow(clippy::too_many_arguments)]
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
    let files = manifest_for_symlink(dir, link_name, target);
    let metadata_bytes =
        serde_saphyr::to_string(&metadata_for(package_name, version, release, deps))?.into_bytes();
    let data = data_to_sign(&metadata_bytes, &payload, files.as_bytes())?;
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
        let guard = LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        // Integration tests run without a terminal; auto-accept confirmation
        // prompts so install/uninstall/update can be exercised directly.
        atlas::cli::confirmation::set_assume_yes(true);

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
