use std::path::PathBuf;
use std::process::Command;
use std::{fs::set_permissions, os::unix::fs::PermissionsExt};

use anyhow::{Context, Result};
use common::types::package::{Architectures, Source};
use rusqlite::Connection;
use sisyphus::extract::extract::extract;
use sisyphus::fetch::fetch::fetch;
use sisyphus::types::fetcher::FetchedSource;
use sisyphus::types::package::Package;
use sisyphus::types::runtime_config::RuntimeConfig;
use tempfile::{Builder as TempBuilder, TempDir};

/// Builds a local git repository acting as the fetch remote: one commit
/// containing a file, an executable and a symlink, whose path is returned for
/// use as the package source URL.
fn seed_remote(keypair_dir: &TempDir) -> Result<PathBuf> {
    let remote = TempBuilder::new()
        .prefix("sisyphus_remote_")
        // The fetcher picks git by checking that the URL ends in `git`, so the
        // remote directory name carries that suffix.
        .suffix("git")
        .tempdir_in(keypair_dir.path())?;

    let run = |args: &[&str]| -> Result<()> {
        let status = Command::new("git")
            .args(args)
            .current_dir(remote.path())
            .env("GIT_AUTHOR_NAME", "tester")
            .env("GIT_AUTHOR_EMAIL", "tester@example.invalid")
            .env("GIT_COMMITTER_NAME", "tester")
            .env("GIT_COMMITTER_EMAIL", "tester@example.invalid")
            .status()
            .context("failed to run git")?;
        anyhow::ensure!(status.success(), "git {args:?} failed");
        Ok(())
    };

    run(&["init", "--initial-branch", "main"])?;
    std::fs::write(remote.path().join("file.txt"), "hello\n")?;
    run(&["add", "file.txt"])?;

    // Real recipes ship executables and symlinks; exercising both here pins
    // that extraction preserves their permissions and types.
    std::fs::write(remote.path().join("build.sh"), "#!/bin/sh\necho built\n")?;
    set_permissions(
        remote.path().join("build.sh"),
        PermissionsExt::from_mode(0o755),
    )?;
    run(&["add", "build.sh"])?;

    std::os::unix::fs::symlink("file.txt", remote.path().join("link.txt"))?;
    run(&["add", "link.txt"])?;

    run(&["commit", "-m", "seed"])?;

    Ok(remote.keep())
}

fn package(url: &PathBuf) -> Package {
    Package {
        schema: 1,
        name: "foo".to_string(),
        version: "1.0.0".to_string(),
        release: 1,
        architecture: Architectures::X86_64,
        source: Source {
            url: url.to_string_lossy().into_owned(),
            sha256: None,
        },
        makedeps: vec![],
        deps: vec![],
        prepare: None,
        build: None,
        check: None,
        package: "true".to_string(),
    }
}

fn cache_paths(config: &RuntimeConfig, url: &str) -> Result<Option<String>> {
    let conn = Connection::open(&config.cache_db_path)?;
    let path = conn
        .query_row("SELECT path FROM sources WHERE url = ?1", [url], |row| {
            row.get(0)
        })
        .ok();
    Ok(path)
}

#[test]
fn the_fetch_cache_preserves_the_cloned_git_source() -> Result<()> {
    let tmp = TempBuilder::new()
        .prefix("sisyphus_test_")
        .tempdir_in("/var/tmp")?;
    let config = RuntimeConfig::build_at(&tmp.path().to_path_buf())?;

    let remote = seed_remote(&tmp)?;
    let pkg = package(&remote);

    // Cache miss: no cache db, no cloned source yet.
    assert!(
        !config.cache_db_path.exists(),
        "cache db must not exist yet"
    );
    let fetched = fetch(&pkg, false, &config).expect("the first fetch of foo must be a miss");
    let cached = match &fetched {
        FetchedSource::Dir(path) => path.clone(),
        other => panic!("a git source must be fetched as Dir, got {other:?}"),
    };

    let cloned = config.fetch_cache.join("foo");
    assert!(cloned.exists(), "the clone must exist in the fetch cache");
    let recorded = cache_paths(&config, &pkg.source.url)?
        .expect("the fetched source must be recorded in the cache");
    assert_eq!(
        recorded,
        cloned.to_string_lossy(),
        "the cache row must point at the cloned source"
    );

    // The extract stage must not move the cache entry away: extract copies.
    let (_extract_dir, source_root) = extract(fetched).expect("extracting the git source");
    assert!(source_root.exists(), "the extracted source root must exist");
    assert!(
        cloned.exists(),
        "extracting the source must leave the cache copy intact"
    );
    assert_eq!(
        cached, cloned,
        "the fetched Dir must point at the clone in the cache, not at the remote"
    );

    // Extraction must faithfully reproduce the source tree: the executable's
    // mode and the symlink's type and target must survive the copy.
    use std::os::unix::fs::PermissionsExt;
    let build_sh = std::fs::metadata(source_root.join("build.sh"))
        .expect("the extracted tree must contain build.sh");
    assert_eq!(
        build_sh.permissions().mode() & 0o777,
        0o755,
        "build.sh must keep its executable bit after extraction"
    );
    let link_txt = source_root
        .join("link.txt")
        .symlink_metadata()
        .expect("the extracted tree must contain link.txt");
    assert!(
        link_txt.file_type().is_symlink(),
        "link.txt must still be a symlink after extraction"
    );
    assert_eq!(
        std::fs::read_link(source_root.join("link.txt"))?,
        std::path::Path::new("file.txt"),
        "the extracted symlink must still point at file.txt"
    );

    // Cache hit: the second fetch serves the same path from the cache.
    let fetched_second =
        fetch(&pkg, false, &config).expect("the second fetch of foo must be a cache hit");
    match &fetched_second {
        FetchedSource::Dir(path) => assert_eq!(
            path, &cloned,
            "the cached source must be served at the same path"
        ),
        other => panic!("a cached git source must be a Dir, got {other:?}"),
    }

    Ok(())
}
