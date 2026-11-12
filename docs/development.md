# Development

Status: draft for version 1.

## Prerequisites

| Requirement | Why |
| --- | --- |
| Stable Rust with edition 2024 support | The workspace is edition 2024, resolver 3. |
| `pkg-config` | `compress-tools` build detection. |
| libarchive headers (`libarchive-dev` on Debian/Ubuntu) | Required to compile Sisyphus. |
| `bwrap` (Bubblewrap) | Required only to run real builds; tests do not invoke it. |
| Git | Normal repository workflow. |

## Build and test

```bash
cargo build --release
cargo test --workspace

cargo test -p atlas <filter>
cargo test -p sisyphus <filter>
cargo test -p common
```

Run a single integration target with output:

```bash
cargo test -p atlas --test install <filter> -- --nocapture
```

CI lives in `.github/workflows/rust.yml`. It installs `pkg-config` and
`libarchive-dev` on Ubuntu and runs `cargo test`. It does not run `cargo fmt`,
`clippy`, or builds that invoke Bubblewrap.

## Workspace layout

| Path | Contents |
| --- | --- |
| `atlas/src/` | CLI, install/update/uninstall/verify pipelines, state machine, rollback guards. |
| `atlas/tests/` | Install, update, uninstall, verify, confirmation, and package-state integration tests. |
| `sisyphus/src/` | CLI, fetch, extract, dependency planning, sandbox, packaging, signing. |
| `sisyphus/tests/` | Cache/fetch, keygen, repository DB, and source-root tests. |
| `crates/common/src/` | `.sky` helpers, metadata/manifest types, directory lookup, repository schema. |
| `crates/common/tests/` | Package-format round-trip tests. |
| `docs/` | Version 1 contracts and operational documentation. |

Dependency versions are declared once in the root `Cargo.toml`
`[workspace.dependencies]`; crate manifests use `dep.workspace = true`.

## Test conventions

Integration tests are self-contained:

- temporary directories,
- `RuntimeConfig::build_at` for isolated state paths,
- shared fixtures in `atlas/tests/fixtures/mod.rs` and
  `sisyphus/tests/fixtures/mod.rs`,
- no external services.

`sisyphus/tests/fetch_archive_cache.rs` runs a local loopback HTTP server. No
test requires internet access or `bwrap`.

Some Atlas tests re-exec the test binary through environment variables
(`ATLAS_CONFIRMATION_TEST_CHILD`, `ATLAS_TERMINAL_TEST_CHILD`,
`ATLAS_RESOLUTION_TEST_CHILD`) to isolate TTY and confirmation behavior. Preserve
that pattern when changing prompt code.

Atlas fixtures set `SISYPHUS_CONFIG_DIR` under a process-wide mutex because
environment variables are global and edition 2024 makes `set_var` unsafe.

Sisyphus's `libc` dev-dependency exists only so keygen tests can force a `umask`;
do not add it to production dependencies.

Place new tests in the matching target:

| Behavior | Target |
| --- | --- |
| Install | `atlas/tests/install*.rs` |
| Update | `atlas/tests/update*.rs` |
| Uninstall | `atlas/tests/uninstall*.rs` |
| Package groups | `atlas/tests/install_group.rs`, `atlas/tests/update_group.rs`, `sisyphus/tests/package_groups.rs` |
| Verification | `atlas/tests/verify.rs` |
| State machine | `atlas/tests/package_state.rs` |
| Prompts | `atlas/tests/confirmation*.rs` |

## Changing a format

`docs/recipe-format.md` and `docs/sky-format.md` define the version 1 contracts.
Keep code and docs in sync. For a `.sky` change, expect edits in:

1. `crates/common/src/types/` — metadata, manifest, identity types,
2. `crates/common/src/functions/sky_verify.rs` — reading and signature data,
3. `sisyphus/src/package/` — metadata, payload, manifest, signing, archive,
4. `atlas/src/verify/` — archive and payload validation, manifest parsing,
5. `atlas/src/types/package_state.rs` — install-time checks,
6. tests in both crates,
7. `docs/sky-format.md`.

For a recipe change, expect edits in:

1. `sisyphus/src/types/package.rs` — parsing and validation,
2. `sisyphus/src/build/` — dependency and phase behavior,
3. `sisyphus/src/package/metadata.rs` — metadata fields,
4. `common` if the metadata contract changes,
5. `docs/recipe-format.md`.

Group labels are deliberately not metadata: they are written to the repository
row by `sisyphus/src/package/db.rs` and read by Atlas from the repository or
install database. A change to group behavior also needs
`docs/package-groups.md`.

## Changing a database

There are no migrations.

- Repository schema: `crates/common/src/db/db.rs`
  (`create_packages_table`).
- Install schema: `atlas/src/install/db.rs` (`ensure_install_schema`).

Both use `CREATE TABLE IF NOT EXISTS`. A schema change must either preserve
compatibility with existing databases or include explicit handling for old
databases. Check every query that uses the changed columns, including:

- `sisyphus/src/package/db.rs` and `sisyphus/src/pipeline/build.rs`,
- `atlas/src/install/resolve.rs`,
- `atlas/src/update/resolve.rs` and `atlas/src/update/db.rs`,
- `atlas/src/uninstall/resolve.rs` and `atlas/src/uninstall/db.rs`,
- `atlas/src/uninstall/orphans.rs`,
- `atlas/src/verify/verify.rs`.

## Documentation policy

- `docs/` describes version 1 behavior and contracts.
- Where documents and code disagree, trust the code and update the document.
- Record implementation-only caveats explicitly so the contract does not
  silently drift from behavior.
- Update the root `README.md` when user-facing commands, prerequisites, or state
  paths change.

## Commits

History uses Conventional Commits with a scope:

```text
feat(atlas): ...
fix(sisyphus): ...
test(atlas): ...
docs: ...
chore(deps): ...
```

Keep commits atomic: one logical change per commit, with tests in the same commit
as the behavior they cover.
