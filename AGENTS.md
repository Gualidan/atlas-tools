# AGENTS.md

## Workspace

Cargo workspace, edition 2024, resolver 3. Three crates:

- `atlas/` — the package manager CLI/library. Entrypoint: `atlas/src/main.rs` → `atlas/src/cli/cli.rs::run`.
- `sisyphus/` — the builder/publisher CLI/library. Entrypoint: `sisyphus/src/main.rs` → `sisyphus/src/cli/cli.rs::run`; full pipeline is `sisyphus/src/pipeline/build.rs`.
- `crates/common/` — shared `.sky` format, metadata/manifest types, hashing/signature verification, and the repository DB schema. It is the contract between Sisyphus (writer) and Atlas (reader); format changes usually need edits in both binaries.

Add dependency versions once in the root `Cargo.toml` `[workspace.dependencies]` and use `dep.workspace = true` in crate manifests.

`docs/recipe-format.md` and `docs/sky-format.md` define the v1 wire contracts. Keep code and docs in sync; where they disagree, trust the code.

## Commands

```bash
cargo test --workspace                              # what CI runs
cargo test -p atlas <filter>                        # or -p sisyphus / -p common
cargo test -p atlas --test install <filter> -- --nocapture
cargo build --release

cargo run --bin sisyphus -- build <recipe-repo>/<name>/sky.yaml
cargo run --bin sisyphus -- checksum-gen <recipe>
cargo run --bin sisyphus -- keygen [-k <path>]

cargo run --bin atlas -- install <name> [--root <dir>]
cargo run --bin atlas -- verify (--package <name> | <sky-path>)
cargo run --bin atlas -- update [<name>]
cargo run --bin atlas -- uninstall <name>
cargo run --bin atlas -- clean
```

- Both binaries accept global `--yes`. Prompts use `inquire` and fail outside a TTY without it.
- CI is `.github/workflows/rust.yml`; it only runs `cargo test` on Ubuntu after installing `pkg-config` and `libarchive-dev`.

## Prerequisites and runtime state

- Stable Rust with edition 2024 support.
- `pkg-config` + libarchive headers (`libarchive-dev`) are required to compile `sisyphus` (`compress-tools`).
- `bwrap` (bubblewrap) is required only to run real builds; the test suite does not invoke it.
- `load_config` requires `$SISYPHUS_CONFIG_DIR/config.yaml`, otherwise `~/.config/sisyphus/config.yaml`, with `priv_key_path` and `pub_key_path`. The YAML file is not auto-created.
- Data/cache/config paths come from the passwd home directory, not `$HOME`. When running as root, `SUDO_USER` must be set or these lookups fail.
- Default state: repo DB `~/.local/share/sisyphus/repo/repo.db`; artifacts `~/.local/share/sisyphus/repo/<arch>/`; source cache `~/.cache/sisyphus/sources` + `cache.db`; install DB `~/.local/share/atlas/installed.db`.
- `atlas install --root <dir>` changes only the target filesystem; the install DB stays in the user data dir. `ATLAS_INSTALL_DIR` is only a legacy test env var and is ignored by production install.
- There is no remote repository/mirror yet. Atlas reads a local SQLite repo DB that Sisyphus (or tests) populates.
- `docs/sky-format.md` still describes the install DB under `/var/lib/atlas/local/...`; current code uses the user data path above.

## Architecture notes

- Atlas flow: resolve from repo DB → verify `.sky` (signature, metadata identity, archive paths, payload manifest) → stage → swap into the target root → commit the install DB. Install/uninstall/update use rollback guards.
- Atlas resolves dependencies from the repo DB `deps` column, **not** from `.sky` metadata. Do not "fix" this; tests deliberately make the two disagree.
- Sisyphus flow: parse/validate recipe → fetch + checksum-verify source → extract → resolve `makedeps` build plan → run phases in Bubblewrap → package/sign → write `.sky` → insert repo row.
- A build is skipped when the same `(name, version, release, arch)` row exists and its sky file still exists. Publishing replaces the single repo row per `(name, arch)`.
- Recipes must live at `<recipe_repo>/<name>/sky.yaml`. The repo root is inferred as the recipe path's second ancestor; dependency recipes are looked up at `<recipe_repo>/<dep>/sky.yaml`.
- Bubblewrap exposes a temporary `/usr` assembled from dependency artifacts, not the host `/usr`, and unshares the network. Phase scripts receive `srcdir`, `builddir`, `pkgdir`, `name`, `version`, `release`, `ARCH`; `package` must stage under `$pkgdir`.
- v1 docs restrict builds/installs to `x86_64`, although the `Architectures` enum also has an `arm` variant used in tests. Atlas install/verify-by-name use `std::env::consts::ARCH`.

## Format and DB invariants

- `.sky` is a zstd-compressed tar with exactly `metadata.yaml`, `payload.tar.zst`, `files`, `signature.ed25519`; no extra top-level entries.
- Signature is Ed25519 over raw `SHA256(metadata.yaml) || SHA256(payload.tar.zst) || SHA256(files)`; `signature.ed25519` is 64 raw bytes.
- `files` is sorted bytewise, tab-separated: `type\tmode\tsha256-or-link-target\tpath`. Payload ownership is normalized to `root:root`.
- Reject absolute paths, `..`, duplicate members, device/FIFO/socket entries, and setuid/setgid files.
- DB schemas are created lazily with `CREATE TABLE IF NOT EXISTS` (`crates/common/src/db/db.rs` for the repo, `atlas/src/install/db.rs` for install state). There are no migrations, so schema changes need explicit handling for existing DBs.

## Tests

- Integration tests are self-contained: temp dirs, `RuntimeConfig::build_at`, and fixtures in `atlas/tests/fixtures/mod.rs` / `sisyphus/tests/fixtures/mod.rs`. No external services; `sisyphus/tests/fetch_archive_cache.rs` uses a local loopback HTTP server.
- Some Atlas tests re-exec themselves through env vars (`ATLAS_CONFIRMATION_TEST_CHILD`, `ATLAS_TERMINAL_TEST_CHILD`, `ATLAS_RESOLUTION_TEST_CHILD`) to isolate TTY/confirmation behavior. Preserve that pattern.
- Atlas fixtures set `SISYPHUS_CONFIG_DIR` under a process-wide mutex because env vars are global; edition 2024 makes `set_var` unsafe.
- Sisyphus's `libc` dev-dependency exists only so keygen tests can force `umask`; do not add it to production dependencies.
- Put new tests in the matching existing target (e.g. install behavior in `atlas/tests/install*.rs`, update behavior in `atlas/tests/update*.rs`) and reuse the shared fixtures.

## Commits

History uses Conventional Commits with a scope: `feat(atlas): ...`, `fix(sisyphus): ...`, `test(atlas): ...`, `docs: ...`, `chore(deps): ...`. Keep commits atomic.
