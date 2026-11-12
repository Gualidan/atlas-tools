# Command-line reference

Status: draft for version 1.

Two binaries are produced by the workspace:

- `atlas` — installs and manages packages.
- `sisyphus` — builds, signs, and publishes packages.

Both accept the global `--yes` / `-y` flag. Both print runtime errors to stderr
and exit with a nonzero status on failure. Clap argument errors exit separately
with the usual usage error status.

## Common behavior

| Behavior | Details |
| --- | --- |
| Global flag | `-y`, `--yes` enables non-interactive confirmation. |
| Prompts | Install, update, uninstall, and clean use `inquire`. Outside a TTY, a prompt fails unless `--yes` is passed. |
| Config | Commands that verify packages load the Sisyphus config YAML and its `pub_key_path`. |
| Paths | State paths come from the passwd home directory; when running as root, `SUDO_USER` must be set. |
| Repository | Local only. There is no remote repository or mirror. |

## `atlas`

```text
Usage: atlas [OPTIONS] <COMMAND>

Commands:
  install
  verify
  uninstall
  update
  clean
```

### `atlas install`

```text
atlas install <PACKAGE> [--root <DIR>]
```

Installs a package and its missing runtime dependencies for the host
architecture.

- `<PACKAGE>` is required.
- `--root <DIR>` selects the target root. The default is `/`.
- The install database stays at `~/.local/share/atlas/installed.db`; it does not
  move under the target root.
- Already-installed dependencies are skipped; the requested package is
  reinstalled.
- The command prints the dependency-ordered package list and asks for
  confirmation unless `--yes` is active.
- Each package must pass signature, metadata, archive, and manifest validation.
- Requires the Sisyphus config with a public key path.

Examples:

```bash
atlas install hello
atlas install hello --root /tmp/test-root
atlas -y install hello
```

Installing into `/` needs write access to the target paths. Running as root
requires `SUDO_USER` to be set so Atlas can find the user state directory.

### `atlas verify`

```text
atlas verify [PACKAGE] [SKY_PATH]
```

Verifies a package without installing it. At least one of `PACKAGE` or
`SKY_PATH` must be supplied, and supplying both is an error.

- With `PACKAGE`, Atlas looks up the host-architecture repository row and
  verifies the artifact, including metadata identity.
- With `SKY_PATH`, the library verifier checks signature and structure without
  an expected package identity.

Current CLI rough edge: both arguments are optional positionals, and a single
positional argument fills `PACKAGE`. As a result, the practical CLI form is:

```bash
atlas verify hello
```

Passing a path as the only argument is treated as a package name. Direct
sky-path verification is available through the library API
(`VerifyArgs.sky_path`).

### `atlas update`

```text
atlas update [PACKAGE]
```

Updates one installed package, or every installed package when `PACKAGE` is
omitted.

- Versions are compared with SemVer.
- A newer repository version is an update.
- An equal version is a no-op, even if release numbers differ.
- A lower repository version is a downgrade error.
- Named updates include the named package's repository dependencies.
- Bulk updates resolve all packages first and abort before prompting if
  resolution reports an error.
- The package list is shown and confirmation is requested unless `--yes` is
  active.
- An empty update set succeeds without prompting.
- Requires the Sisyphus config when there are candidates to verify and apply.

Examples:

```bash
atlas update
atlas update hello
atlas -y update
```

### `atlas uninstall`

```text
atlas uninstall <PACKAGE>
```

Removes an installed package and its non-explicit dependencies.

- Explicitly installed dependencies remain installed and stop traversal.
- Packages required by packages outside the removal set are skipped and listed
  with the external required-by package.
- Owned files and symlinks are removed; owned directories are removed only when
  empty.
- The package list is shown and confirmation is requested unless `--yes` is
  active.
- Does not require the Sisyphus config.
- Filesystem removal is not transactional: a partial failure can leave some
  paths deleted and database rows in place.

### `atlas clean`

```text
atlas clean
```

Removes orphaned packages for the host architecture.

An orphan is an installed package that is neither explicit nor reachable from an
explicit package through recorded dependency edges. Atlas prints the list, asks
once for confirmation unless `--yes` is active, and uninstalls each orphan
without further prompts. An empty orphan list succeeds without prompting.

See [install-state.md](install-state.md) for the exact queries and semantics.

## `sisyphus`

```text
Usage: sisyphus [OPTIONS] <COMMAND>

Commands:
  build
  keygen
  checksum-gen
```

### `sisyphus build`

```text
sisyphus build <RECIPE> [-s|--skip-build]
```

Runs the full pipeline for a recipe:

1. parse and validate the recipe,
2. skip if the exact identity is already published and its artifact exists,
3. fetch and verify the source,
4. extract it,
5. plan and build dependencies,
6. run the recipe phases in Bubblewrap,
7. sign and write the `.sky` artifact,
8. publish the repository row.

`<RECIPE>` must be `<recipe-repo>/<name>/sky.yaml`. The repository root is
derived from the recipe path.

`--skip-build` currently prints `Skipping build.` and exits successfully without
fetching, extracting, or building. It is a stub, not a partial pipeline.

Requires:

- a valid Sisyphus config with `priv_key_path`,
- `bwrap` on `PATH` for actual builds,
- a dependency set that provides `bash` and the tools used by phases.

Example:

```bash
sisyphus build ./recipes/hello/sky.yaml
```

### `sisyphus keygen`

```text
sisyphus keygen [-k|--keypair-path <PATH>]
```

Generates an Ed25519 keypair and writes the Sisyphus config.

- Without `-k`, keys go under `~/.config/sisyphus/keys/` with a
  user-hostname-timestamp filename.
- With `-k`, the private key is written at `<PATH>` and the public key at the
  same path with a `.pub` extension.
- Existing key files are never overwritten.
- The config is written to `~/.config/sisyphus/config.yaml` and records both
  paths. Unlike config loading, `keygen` does not honor
  `$SISYPHUS_CONFIG_DIR`.
- The config file is rewritten after successful key generation.

Current implementation quirk: when a config already exists, `keygen` prompts
only if `--yes` is active. Without `--yes`, it rewrites the config without
confirmation. Existing key files still cause an error rather than being
overwritten.

Example:

```bash
sisyphus keygen
sisyphus keygen -k /secure/signing.key
```

### `sisyphus checksum-gen`

```text
sisyphus checksum-gen <RECIPE>
```

Fetches the recipe's declared source and prints its SHA-256:

```text
Checksum: <hex>, paste this into your recipe
```

Despite the command's help text, it does **not** modify the YAML file. Copy the
printed digest into `source.sha256` manually.

This command downloads the source but does not update the source cache database.
It does not require signing keys.

## End-to-end example

```bash
# 1. Build the workspace
cargo build --release

# 2. Create keys and ~/.config/sisyphus/config.yaml
cargo run --bin sisyphus -- keygen

# 3. Fill in the recipe's source.sha256
cargo run --bin sisyphus -- checksum-gen ./recipes/hello/sky.yaml

# 4. Build and publish
cargo run --bin sisyphus -- build ./recipes/hello/sky.yaml

# 5. Verify and install
cargo run --bin atlas -- verify hello
cargo run --bin atlas -- install hello

# 6. Manage installed packages
cargo run --bin atlas -- update hello
cargo run --bin atlas -- uninstall hello
cargo run --bin atlas -- clean
```

## Current CLI limitations

- No package version selection; the repository has one row per
  `(name, architecture)`.
- No remote repository or download command.
- No dry-run mode.
- `verify` accepts two optional positionals but only one may be supplied; see
  the rough edge above.
- `build --skip-build` is a stub.
- `checksum-gen` does not edit recipes.
