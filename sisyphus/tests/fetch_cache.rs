use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};
use common::types::package::{Architectures, Source};
use rusqlite::Connection;
use sisyphus::extract::extract::extract;
use sisyphus::fetch::fetch::fetch;
use sisyphus::types::fetcher::FetchedSource;
use sisyphus::types::package::Package;
use sisyphus::types::runtime_config::RuntimeConfig;
use tempfile::{Builder as TempBuilder, TempDir};

/// Builds a local git repository acting as the fetch remote: one commit with a
/// single file, whose path is returned for use as the package source URL.
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
