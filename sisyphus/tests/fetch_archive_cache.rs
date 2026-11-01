use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::{Context, Result};
use common::functions::hash::hash;
use common::types::package::{Architectures, Source};
use rusqlite::Connection;
use sisyphus::checksum::checksum::checksum_gen;
use sisyphus::fetch::fetch::{FetchMode, fetch};
use sisyphus::types::package::Package;
use sisyphus::types::runtime_config::RuntimeConfig;
use tempfile::{Builder as TempBuilder, TempDir};

/// Serves a queue of byte payloads, one per HTTP request: the queue's first
/// response goes to the first GET, the second to the second GET, and the last
/// payload repeats forever. This lets a single URL change its content between
/// fetches, the way a re-released tarball does.
fn start_payload_server(payloads: Vec<Vec<u8>>) -> Result<String> {
    let listener = TcpListener::bind("127.0.0.1:0").context("failed to bind test server")?;
    let port = listener.local_addr()?.port();
    let url = format!("http://127.0.0.1:{port}/pkg.tar.gz");

    let queue: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(payloads));
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut drain = [0u8; 4096];
            let _ = stream.read(&mut drain);

            let payload: Vec<u8> = {
                let mut queue = queue.lock().unwrap();
                if queue.len() > 1 {
                    queue.remove(0)
                } else {
                    queue[0].clone()
                }
            };

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                payload.len()
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(&payload);
        }
    });

    Ok(url)
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(hash(bytes).unwrap().as_ref())
}

fn package(url: &str, sha256: &str) -> Package {
    Package {
        schema: 1,
        name: "foo".to_string(),
        version: "1.0.0".to_string(),
        release: 1,
        architecture: Architectures::X86_64,
        source: Source {
            url: url.to_string(),
            sha256: sha256.to_string(),
        },
        makedeps: vec![],
        deps: vec![],
        prepare: None,
        build: None,
        check: None,
        package: "true".to_string(),
    }
}

fn config(tmp: &TempDir) -> Result<RuntimeConfig> {
    RuntimeConfig::build_at(&tmp.path().to_path_buf())
}

fn source_rows(config: &RuntimeConfig) -> i64 {
    // Checksum mode never touches the cache db, so the db or table may not
    // exist: a missing table counts as zero rows.
    if !config.cache_db_path.exists() {
        return 0;
    }
    let conn = Connection::open(&config.cache_db_path).unwrap();
    conn.query_row("SELECT COUNT(*) FROM sources", [], |row| row.get(0))
        .unwrap_or(0)
}

fn cached_sha(config: &RuntimeConfig, url: &str) -> Option<String> {
    let conn = Connection::open(&config.cache_db_path).unwrap();
    conn.query_row("SELECT sha256 FROM sources WHERE url = ?1", [url], |row| {
        row.get(0)
    })
    .ok()
}

/// Arc 1: a miss downloads the payload, verifies it and records it; a second
/// build then hits the cache and serves the same file without the row count
/// ever changing.
#[test]
fn a_miss_downloads_and_a_second_build_hits_the_cache() -> Result<()> {
    let tmp = TempBuilder::new()
        .prefix("sisyphus_test_")
        .tempdir_in("/var/tmp")?;
    let runtime_config = config(&tmp)?;

    let contents = b"first archive\n";
    let url = start_payload_server(vec![contents.to_vec()])?;
    let pkg = package(&url, &sha256(contents));

    let fetched = fetch(&pkg, &FetchMode::Build, &runtime_config)
        .expect("a cache miss must download and verify the source");
    assert!(fetched.exists(), "the downloaded file must exist");
    assert_eq!(
        std::fs::read(&fetched)?,
        contents,
        "the file must be the served payload"
    );
    assert_eq!(
        source_rows(&runtime_config),
        1,
        "the miss must record one row"
    );

    let hit = fetch(&pkg, &FetchMode::Build, &runtime_config)
        .expect("the second build must hit the cache");
    assert_eq!(hit, fetched, "the cache hit must serve the same file");
    assert_eq!(
        source_rows(&runtime_config),
        1,
        "a cache hit must not add rows"
    );

    Ok(())
}

/// Arc 2: `checksum_gen` end-to-end — it fetches without touching the cache db
/// and without verification, leaving no row behind.
#[test]
fn checksum_gen_end_to_end() -> Result<()> {
    let tmp = TempBuilder::new()
        .prefix("sisyphus_test_")
        .tempdir_in("/var/tmp")?;
    let runtime_config = config(&tmp)?;

    let contents = b"archive for checksum\n";
    let url = start_payload_server(vec![contents.to_vec()])?;
    let pkg = package(&url, "");

    checksum_gen(&pkg, &runtime_config).expect("checksum_gen must fetch and hash the archive");
    // checksum_gen uses FetchMode::Checksum, which never touches the cache db.
    assert_eq!(
        source_rows(&runtime_config),
        0,
        "checksum_gen must not insert the source into a row"
    );

    Ok(())
}

/// Arc 3: `checksum_gen` run twice — the regression pin. A second run must not
/// crash and must not touch the cache db: Checksum mode only downloads and
/// hashes, so the table stays empty.
#[test]
fn checksum_gen_twice() -> Result<()> {
    let tmp = TempBuilder::new()
        .prefix("sisyphus_test_")
        .tempdir_in("/var/tmp")?;
    let runtime_config = config(&tmp)?;

    let contents = b"archive for checksum\n";
    let url = start_payload_server(vec![contents.to_vec()])?;
    let pkg = package(&url, "");

    checksum_gen(&pkg, &runtime_config).expect("the first checksum_gen must succeed");
    checksum_gen(&pkg, &runtime_config)
        .expect("the second checksum_gen must succeed over the existing row");

    assert_eq!(
        source_rows(&runtime_config),
        0,
        "no row is inserted during checksum"
    );

    Ok(())
}

/// Arc 4: same URL, new sha — the old entry must be replaced, not duplicated
/// or stale (INSERT OR REPLACE semantics on the url primary key).
#[test]
fn a_new_sha_for_the_same_url_replaces_the_cached_row() -> Result<()> {
    let tmp = TempBuilder::new()
        .prefix("sisyphus_test_")
        .tempdir_in("/var/tmp")?;
    let runtime_config = config(&tmp)?;

    let first = b"first release\n";
    let second = b"second release\n";
    let url = start_payload_server(vec![first.to_vec(), second.to_vec()])?;

    let pkg_one = package(&url, &sha256(first));
    fetch(&pkg_one, &FetchMode::Build, &runtime_config).expect("the first fetch must succeed");
    assert_eq!(
        cached_sha(&runtime_config, &url).as_deref(),
        Some(sha256(first).as_str()),
        "the first sha must be recorded"
    );

    let pkg_two = package(&url, &sha256(second));
    let fetched = fetch(&pkg_two, &FetchMode::Build, &runtime_config)
        .expect("a new sha for the same url must miss the cache and re-download");
    assert_eq!(
        std::fs::read(&fetched)?,
        second,
        "the fetch must serve the new payload"
    );
    assert_eq!(
        cached_sha(&runtime_config, &url).as_deref(),
        Some(sha256(second).as_str()),
        "the new sha must replace the old cache row"
    );

    // The entry now serves the second release, not the first.
    let hit = fetch(&pkg_two, &FetchMode::Build, &runtime_config)
        .expect("the replaced row must serve the new file");
    assert_eq!(
        std::fs::read(&hit)?,
        second,
        "the cache must hold the new release"
    );

    Ok(())
}
