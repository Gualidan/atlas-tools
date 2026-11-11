# Architecture

Status: draft for version 1.

Atlas Tools is a Cargo workspace with three crates that form a writer/reader
pair around a shared package format.

```text
recipe (sky.yaml)
      │
      ▼
┌─────────────┐    fetch + verify     ┌──────────────┐
│  Sisyphus   │ ────────────────────► │ source cache │
│  (builder)  │                       └──────────────┘
│             │    build in bwrap
│             │    sign + package
└──────┬──────┘
       │ writes .sky + repo row
       ▼
┌─────────────────────┐
│ local repository DB │
│ + repo/<arch>/*.sky │
└──────────┬──────────┘
           │ reads
           ▼
┌─────────────┐   verify + stage + commit
│    Atlas    │ ─────────────────────────► target root
│ (installer) │ ─────────────────────────► install DB
└─────────────┘
```

## Crates

### `atlas`

The package manager CLI and library.

| Area | Location | Responsibility |
| --- | --- | --- |
| CLI | `atlas/src/cli/` | Argument parsing, global `--yes`, confirmation prompts. |
| Resolution | `atlas/src/install/resolve.rs`, `atlas/src/update/resolve.rs`, `atlas/src/uninstall/resolve.rs` | Dependency and update planning. |
| Verification | `atlas/src/verify/` | Archive entry checks, payload checks, manifest checks, signature verification. |
| Install state | `atlas/src/types/package_state.rs`, `atlas/src/install/db.rs` | Verify → stage → commit state machine and install records. |
| Rollback | `atlas/src/types/guards.rs` | Restore files if the database commit fails. |
| Uninstall | `atlas/src/uninstall/` | Owned-path removal, shared-dependency preservation, orphan cleanup. |
| Update | `atlas/src/update/` | Semantic-version comparison, staged replacement, database updates. |

The entrypoint is `atlas/src/main.rs` → `atlas/src/cli/cli.rs::run`.

### `sisyphus`

The builder and publisher.

| Area | Location | Responsibility |
| --- | --- | --- |
| CLI | `sisyphus/src/cli/` | `build`, `checksum-gen`, `keygen`. |
| Fetch | `sisyphus/src/fetch/` | HTTP(S) download, source cache lookup, cache database writes. |
| Checksum | `sisyphus/src/checksum/` | SHA-256 verification and `checksum-gen` output. |
| Extract | `sisyphus/src/extract/` | libarchive extraction and source-root detection. |
| Dependencies | `sisyphus/src/build/dependencies.rs` | Dependency-first build planning and artifact reuse. |
| Build | `sisyphus/src/build/build.rs` | Bubblewrap phase execution. |
| Package | `sisyphus/src/package/` | Metadata, payload, manifest, signature, `.sky` writing, repo insertion. |
| Signing | `sisyphus/src/signing/` | Ed25519 key generation and signing. |

The entrypoint is `sisyphus/src/main.rs` → `sisyphus/src/cli/cli.rs::run`; the
full pipeline is `sisyphus/src/pipeline/build.rs::run_build`.

### `common`

The shared contract between Sisyphus and Atlas:

- `.sky` reading and signature helpers (`functions/sky_verify.rs`),
- SHA-256 helpers (`functions/hash.rs`),
- payload unpacking (`functions/unpack_payload.rs`),
- metadata, manifest, package identity, and architecture types,
- repository schema creation (`db/db.rs`),
- user data/cache/config directory lookup.

Format changes usually require edits in both binaries.

## Build pipeline

`sisyphus build <recipe>` performs these steps:

1. **Parse and validate** the recipe (`Recipe::parse`, `Package::validate`).
2. **Skip if already published**: if the repository has a row for the exact
   `(name, version, release, architecture)` and its `sky_path` still exists, the
   build exits successfully without rebuilding.
3. **Fetch** the source over HTTP(S) into the source cache, or reuse a verified
   cached copy.
4. **Verify** the source's SHA-256 against the recipe.
5. **Extract** the archive into a temporary directory and locate the source root.
6. **Plan dependencies** from `makedeps` (see
   [build-environment.md](build-environment.md)). Missing dependency recipes are
   built first, in dependency-first order. Verified local artifacts are reused.
7. **Build** each planned package, then the root package, in Bubblewrap.
8. **Package**: generate the payload, manifest, metadata, and signature; write
   `<name>-<version>-<release>-<architecture>.sky`.
9. **Publish**: replace the repository row for `(name, architecture)`.

A failure at any step aborts the command. Temporary build directories are
cleaned up by `tempfile`. A failed build does not publish a repository row; a
failed database insert after writing the artifact can leave an unreferenced
`.sky` file on disk.

## Install pipeline

`atlas install <name>` performs these steps:

1. Look up the package in the repository for the host architecture.
2. Resolve the package and its runtime dependencies from the repository's
   `deps` column. Already-installed dependencies are skipped; the requested
   package is reinstalled.
3. Ask for confirmation unless `--yes` is active.
4. For each package in dependency-first order:
   1. verify the signature against the configured public key,
   2. verify metadata identity against the repository row,
   3. validate the outer archive and payload paths,
   4. extract the payload into a staging directory,
   5. check file ownership and destination conflicts,
   6. exchange files into the target root,
   7. record the install in the install database,
   8. roll back the filesystem exchange if recording fails.

`update` follows the same verify/stage/commit path, then removes owned paths
that are no longer present in the new payload. `uninstall` removes owned paths
and then deletes database records; it does not restore files if removal fails
partway through.

## Key invariants

- **One repository row per `(name, architecture)`.** Publishing replaces that
  row. Atlas cannot install two versions of the same package for one
  architecture from the local repository.
- **Atlas resolves runtime dependencies from the repository database**, not
  from `.sky` metadata. Tests deliberately make the two sources disagree.
- **Package identity is exact**: name, version, release, and architecture must
  match the repository row during install.
- **`/var/lib/atlas` is reserved.** Payloads may not write there.
- **Owned files are tracked per package and architecture** in `owned_files`.
  Directories are claimed only when the install created them or they were
  already owned by the same package.
- **All paths are relative to the target root** and are validated before any
  destination is mutated.
- **The install database is user state**, even for `--root` installs. It lives
  under the invoking user's data directory, not under the target root.

## State locations

| State | Default |
| --- | --- |
| Repository database | `~/.local/share/sisyphus/repo/repo.db` |
| Repository artifacts | `~/.local/share/sisyphus/repo/<architecture>/` |
| Source cache | `~/.cache/sisyphus/sources/` |
| Source cache database | `~/.cache/sisyphus/cache.db` |
| Sisyphus config | `~/.config/sisyphus/config.yaml` (or `$SISYPHUS_CONFIG_DIR/config.yaml`) |
| Atlas install database | `~/.local/share/atlas/installed.db` |
| Install staging | `<target-root>/var/lib/atlas/tmp/` |

Paths are derived from the passwd home directory, not `$HOME`. When running as
root, `SUDO_USER` must identify the target user.

## Current limitations

- No remote repository, mirror, or transport protocol.
- No version constraints, optional dependencies, split packages, patches, or
  package deltas.
- Recipe schema 1 documents `x86_64`; the architecture enum also accepts `arm`
  for tests.
- No reproducible-build guarantee. `built_at` is informational.
- No database migrations. Schemas are created with `CREATE TABLE IF NOT EXISTS`;
  changes to existing databases need explicit handling.
- Uninstall is not transactional across the filesystem: a failure can leave
  some owned paths deleted while database rows remain.
