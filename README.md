# Atlas Tools

>[!WARNING]
> This is a work in progress and is not yet ready for use.

Atlas Tools is a Rust workspace implementing a source-based package management
system. **Sisyphus** builds signed `.sky` packages from declarative recipes, and
**Atlas** installs, verifies, updates, and removes them.

There is no remote repository or mirror yet. Both tools work against a local
SQLite repository database populated by Sisyphus.

## Workspace layout

| Path | Crate | Role |
| --- | --- | --- |
| `atlas/` | `atlas` | Package manager CLI/library: install, verify, update, uninstall, and orphan cleanup. |
| `sisyphus/` | `sisyphus` | Builder/publisher CLI/library: fetch sources, build in a sandbox, sign, package, and publish. |
| `crates/common/` | `common` | Shared `.sky` format, metadata and manifest types, hashing/signature verification, and database schemas. |
| `docs/` | — | Version 1 contracts and guides. Start at [`docs/README.md`](docs/README.md). |

`common` is the contract between the writer (Sisyphus) and the reader (Atlas),
so format changes usually touch both binaries.

## Features

- **Declarative recipes** — one `sky.yaml` per package with optional shell
  phases for preparation, build, and checks.
- **Hermetic builds** — phase scripts run in Bubblewrap with a temporary `/usr`
  assembled from dependency artifacts, writable
  `$srcdir`/`$builddir`/`$pkgdir`, and no network access.
- **Source caching and checksum verification** — downloaded sources are cached
  and verified against the recipe's SHA-256 before extraction.
- **Dependency resolution** — `makedeps` are resolved recursively and built in
  dependency-first order; cycles, self-dependencies, and missing recipes are
  rejected, and existing verified artifacts are reused.
- **Signed packages** — every `.sky` artifact carries an Ed25519 signature over
  its metadata, payload, and manifest.
- **Transactional installs** — packages are verified and staged before being
  committed, with rollback guards for failed installs and updates.
- **Target-root installs** — `--root <dir>` installs into an alternate
  filesystem root without moving the install database.
- **Shared dependency handling** — uninstall keeps packages still required by
  retained packages; `clean` removes packages that are no longer reachable.
- **Local repository** — a SQLite database stores one publishable row per
  `(name, architecture)` and points Atlas at each `.sky` artifact.

## How it works

### Build pipeline (Sisyphus)

1. Parse and validate the recipe.
2. Fetch the source over HTTP(S), reusing the cache when possible, and verify
   its SHA-256 digest.
3. Extract the source into a writable working tree.
4. Resolve `makedeps` into a dependency-first build plan, reusing existing
   verified artifacts and building anything missing.
5. Run `prepare`, `build`, `check`, then the required `package` phase inside
   Bubblewrap.
6. Generate `metadata.yaml`, `payload.tar.zst`, `files`, and a signature, then
   write `<name>-<version>-<release>-<architecture>.sky`.
7. Insert or replace the package's repository row.

A build is skipped when the same `(name, version, release, architecture)` row
exists and its `.sky` file is still present. Publishing replaces the single
repository row per `(name, architecture)`.

### Install pipeline (Atlas)

1. Resolve the package and its runtime dependencies from the repository
   database's `deps` column (not from `.sky` metadata).
2. Verify the signature against the configured public key, the metadata
   identity, archive paths, and the payload against the `files` manifest.
3. Stage the payload, swap it into the target root, and record ownership,
   dependency edges, and install reason.
4. Roll back filesystem changes if recording fails.

## Package formats

### Recipe format

Recipes are versioned YAML files. Sisyphus looks for a package's recipe at
`<recipe-repo>/<name>/sky.yaml`, and the directory name is expected to match
`name`.

```yaml
schema: 1
name: hello
version: 2.12.1
release: 1
architecture: x86_64

source:
  url: https://ftp.gnu.org/gnu/hello/hello-2.12.1.tar.gz
  sha256: "<64 lowercase hexadecimal SHA-256 characters>"

makedeps:
  - gcc
  - make

deps:
  - glibc

prepare: |
  cd "$srcdir/hello-2.12.1"

build: |
  cd "$srcdir/hello-2.12.1"
  ./configure --prefix=/usr
  make

check: |
  cd "$srcdir/hello-2.12.1"
  make check

package: |
  cd "$srcdir/hello-2.12.1"
  make DESTDIR="$pkgdir" install
```

| Field | Required | Meaning |
| --- | --- | --- |
| `schema` | yes | Recipe-format version. Version 1 must be `1`. |
| `name` | yes | Package identifier matching `[a-z0-9][a-z0-9+._-]*`. It is expected to match the recipe directory. |
| `version` | yes | Upstream version. Whitespace, control characters, and `/` are rejected. |
| `release` | yes | Atlas packaging revision for this upstream version. |
| `architecture` | yes | Target CPU architecture. Version 1 documents `x86_64`; an `arm` variant exists for tests. |
| `source` | yes | Object with `url` (HTTP/S) and lowercase 64-character `sha256`. |
| `makedeps` | no | Build-time dependencies resolved and installed in the build root. Defaults to `[]`. |
| `deps` | no | Runtime dependencies embedded in `.sky` metadata and checked by Atlas. Defaults to `[]`. |
| `prepare` | no | Preparation script. |
| `build` | no | Compilation script. |
| `check` | no | Test script. |
| `package` | yes | Script that stages files under `$pkgdir`. |

Phases run in order with `bash -euo pipefail -c <script>` and receive `srcdir`,
`builddir`, `pkgdir`, `name`, `version`, `release`, and `ARCH`. The `package`
phase must not write outside `$pkgdir`.

Version 1 intentionally has no Git sources, patches, split packages, optional
dependencies, version constraints, build options, or cross-compilation. See
[`docs/recipe-format.md`](docs/recipe-format.md) for the full contract.

### `.sky` package format

A `.sky` file is a Zstandard-compressed tar archive named
`<name>-<version>-<release>-<architecture>.sky`. It contains exactly four
top-level entries:

```text
metadata.yaml
payload.tar.zst
files
signature.ed25519
```

- `metadata.yaml` records the format version, package identity, runtime
  dependencies, source provenance, and the recipe's SHA-256.
- `payload.tar.zst` contains the staged filesystem tree. Paths are relative to
  the target root and may be regular files, directories, or symlinks.
  Ownership is normalized to `root:root`; setuid/setgid files, device nodes,
  FIFOs, sockets, absolute paths, and `..` components are rejected.
- `files` is a bytewise-sorted, tab-separated manifest:
  `type\tmode\tsha256-or-link-target\tpath`.
- `signature.ed25519` is a 64-byte Ed25519 signature over
  `SHA256(metadata.yaml) || SHA256(payload.tar.zst) || SHA256(files)`.

See [`docs/sky-format.md`](docs/sky-format.md) for the complete format.

## Getting started

### Prerequisites

- Linux with a stable Rust toolchain supporting edition 2024.
- Git.
- `pkg-config` and libarchive headers (`libarchive-dev` on Debian/Ubuntu) to
  compile Sisyphus through `compress-tools`.
- `bwrap` (Bubblewrap) to run real builds. The test suite does not invoke it.

### Build

```bash
cargo build --release
```

### Configure signing keys

A Sisyphus config file is required by `build` and by Atlas `install`/`verify`.
Generate an Ed25519 keypair and the config in one step:

```bash
cargo run --bin sisyphus -- keygen
# or choose the private key path:
cargo run --bin sisyphus -- keygen -k /path/to/signing.key
```

By default, keys are written under `~/.config/sisyphus/keys/` and the config to
`~/.config/sisyphus/config.yaml`. Later commands read
`$SISYPHUS_CONFIG_DIR/config.yaml` when that variable is set, otherwise
`~/.config/sisyphus/config.yaml`. The config contains `priv_key_path` and
`pub_key_path`; the public key is written next to the private key with a `.pub`
extension. Existing key files are never overwritten.

### Build your first package

1. Create a recipe at `<recipe-repo>/<name>/sky.yaml`.
2. Fill in `source.sha256`. `checksum-gen` downloads the source and prints the
   digest to paste into the recipe:

   ```bash
   cargo run --bin sisyphus -- checksum-gen ./recipes/hello/sky.yaml
   ```

3. Build and publish:

   ```bash
   cargo run --bin sisyphus -- build ./recipes/hello/sky.yaml
   ```

The artifact is written to
`~/.local/share/sisyphus/repo/<architecture>/<name>-<version>-<release>-<architecture>.sky`,
and its row is inserted into `~/.local/share/sisyphus/repo/repo.db`.

### Install and manage packages

```bash
# Install into / (requires write access to the target paths)
cargo run --bin atlas -- install hello

# Install into an alternate root (the install DB stays in your data dir)
cargo run --bin atlas -- install hello --root /tmp/test-root

# Verify the repository package for the host architecture
cargo run --bin atlas -- verify hello

# Update one package, or every installed package
cargo run --bin atlas -- update hello
cargo run --bin atlas -- update

# Remove a package (dependencies still required by other packages are kept)
cargo run --bin atlas -- uninstall hello

# Remove installed packages that are no longer reachable
cargo run --bin atlas -- clean
```

Atlas currently resolves packages for the host architecture. The verifier also
supports checking a `.sky` file directly through the library's `VerifyArgs`
(`sky_path`).

## CLI reference

Both binaries accept the global `--yes` / `-y` flag. Confirmation prompts use
`inquire` and fail outside a TTY unless `--yes` is passed.

### `sisyphus`

| Command | Description |
| --- | --- |
| `sisyphus build <recipe>` | Run the full fetch, dependency, build, package, and publish pipeline. |
| `sisyphus checksum-gen <recipe>` | Fetch the declared source and print its SHA-256 for the recipe. |
| `sisyphus keygen [-k <path>]` | Generate an Ed25519 keypair and write the Sisyphus config. |

### `atlas`

| Command | Description |
| --- | --- |
| `atlas install <name> [--root <dir>]` | Install a package and missing runtime dependencies. Defaults to `/`. |
| `atlas verify <name>` | Verify signature, metadata identity, archive paths, and manifest/payload agreement without installing. |
| `atlas update [<name>]` | Upgrade one package or all installed packages to newer semantic versions. Equal versions are skipped; downgrades are rejected. |
| `atlas uninstall <name>` | Remove a package, keeping shared dependencies that are still required. |
| `atlas clean` | Remove orphaned packages that are not reachable from explicitly installed packages. |

## Runtime state

Default paths are derived from the invoking user's passwd home directory, not
`$HOME`. When running as root, `SUDO_USER` must be set or lookups fail.

| State | Default path |
| --- | --- |
| Sisyphus config (includes key paths) | `~/.config/sisyphus/config.yaml` (or `$SISYPHUS_CONFIG_DIR/config.yaml`) |
| Repository database | `~/.local/share/sisyphus/repo/repo.db` |
| Built artifacts | `~/.local/share/sisyphus/repo/<architecture>/` |
| Source cache | `~/.cache/sisyphus/sources/` and `~/.cache/sisyphus/cache.db` |
| Atlas install database | `~/.local/share/atlas/installed.db` |
| Staging directories | `<target-root>/var/lib/atlas/tmp/` |

The repository database and install database are SQLite files created lazily.
There are no migrations yet, so schema changes need explicit handling for
existing databases.

Atlas reserves `/var/lib/atlas` in package payloads and rejects packages that
try to write there.

## Development

```bash
cargo test --workspace                 # what CI runs
cargo test -p atlas <filter>           # or -p sisyphus / -p common
cargo test -p atlas --test install <filter> -- --nocapture
```

CI lives in `.github/workflows/rust.yml` and runs `cargo test` on Ubuntu after
installing `pkg-config` and `libarchive-dev`.

The v1 contracts in `docs/` must stay in sync with the code; where they
disagree, trust the code. Integration tests are self-contained and use temporary
directories, `RuntimeConfig::build_at`, and shared fixtures. No external
services are required.

Changes are organized as atomic commits using Conventional Commits with a
scope, for example `feat(atlas): ...`, `fix(sisyphus): ...`,
`test(atlas): ...`, `docs: ...`, or `chore(deps): ...`.

## Contributing

Contributions are welcome:

1. Fork the repository.
2. Create a feature branch.
3. Add or update tests for your change. I recommend using the
   `tdd-test-writer` skill; it is already included in this repo.
   For further info on how to use it see the [README File](Skills-README.md).

4. Keep commits atomic and use Conventional Commit messages. I recommend the `atomic-commits` skill; install it with:

   ```sh
   npx skills add https://github.com/nullswan/atomic-commits --skill atomic-commits
   ```
>[!WARNING]
> Your changes will be rejected if they do not follow the Atomic Commits format.

5. Submit a pull request with a clear description.

## License

This project is licensed under the GNU General Public License v3.0 or later
(`GPL-3.0-or-later`). See the [LICENSE](LICENSE) file for details.
