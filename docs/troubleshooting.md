# Troubleshooting

Status: draft for version 1.

This page maps common symptoms to likely causes and fixes.

## Build and setup

| Symptom | Likely cause | Fix |
| --- | --- | --- |
| `pkg-config` or libarchive errors while compiling `sisyphus` | Missing build dependencies for `compress-tools`. | Install `pkg-config` and `libarchive-dev` (or the equivalent for your distribution). |
| `Failed to execute bwrap` / `bwrap` not found | Bubblewrap is not installed. | Install `bwrap`; it is only needed for real builds, not tests. |
| `Command is running outside of a TTY try passing --yes` | A confirmation prompt was reached without a terminal. | Pass `--yes` / `-y`, or run interactively. |
| `Failed to read SUDO_USER variable` or user lookup failure | Running as root without `SUDO_USER`, or the user has no passwd entry. | Set `SUDO_USER=<user>` or run as the target user. |
| `Failed to build config` / missing config file | `~/.config/sisyphus/config.yaml` does not exist. | Run `sisyphus keygen` or create the config with `priv_key_path` and `pub_key_path`. |
| `Failed to read public key` | Config points to a missing or wrong `pub_key_path`. | Regenerate keys or fix the config path. |
| `Invalid public key: expected 32 bytes` | The public key file is not a raw Ed25519 public key. | Point `pub_key_path` at the `.pub` file produced by `keygen`. |

## Building packages

| Symptom | Likely cause | Fix |
| --- | --- | --- |
| `Package "<name>" already exists, skipping build` | The exact `(name, version, release, architecture)` row exists and the artifact is present. | Bump `version` or `release`, or remove the repository row and artifact deliberately. |
| `Checksum mismatch: expected ..., got ...` | The downloaded source does not match the recipe digest. | Re-run `checksum-gen` and update `source.sha256` only if the new source is expected. |
| `Git sources are not supported in v1` | The source URL path ends in `.git`. | Use a release tarball instead. |
| `Invalid url scheme` | The source uses `file://` or another non-HTTP scheme. | Use `http` or `https`; prefer `https`. |
| `Failed to validate recipe` / invalid name | Name grammar violated. | Use `[a-z0-9][a-z0-9+._-]*`. |
| `Only schema version 1 is supported` | `schema` is missing, `0`, or greater than `1`. | Set `schema: 1`. |
| `Package is not allowed to list itself as a dependency` | `deps` or `makedeps` contains the package name. | Remove the self-entry. |
| `Group name <label> is invalid refer to the package naming documentation` | A `groups` entry violates the package-name grammar. | Use `[a-z0-9][a-z0-9+._-]*`. |
| `Group list must not contain duplicate entries` | The same label appears twice in `groups`. | Remove the duplicate. |
| `Package is not allowed to list itself as a group` | `groups` contains the package's own name. | Remove the self-entry. |
| `dependency `<name>` was not found at ...` | Missing dependency recipe or wrong repository layout. | Add `<recipe-repo>/<dep>/sky.yaml`. |
| `Circular dependency detected` / `dependency cycle` | The dependency graph contains a cycle. | Break the cycle in the recipes. |
| `bash: command not found` inside a phase | The sandbox `/usr` is built only from dependency artifacts. | Add `makedeps` that provide the shell and required tools. |
| Network command fails during a phase | The build sandbox unshares the network. | Declare the download as a recipe source; phases cannot fetch at build time. |
| Files are missing from the package | The `package` phase wrote outside `$pkgdir`. | Stage with `DESTDIR="$pkgdir"` or copy into `$pkgdir`. |

## Repository and install

| Symptom | Likely cause | Fix |
| --- | --- | --- |
| `Package <name> not found` | No repository row for that name and host architecture. | Build/publish the package; check the `architecture` field. |
| `Package <name> not found in installed packages` | `update`/`uninstall` targets a package that is not installed. | Install it first or check the name. |
| `Specify a package or group name to install.` | `atlas install` was called without a package or `--group`. | Pass exactly one of them. |
| `Specify a package or group name to install, not both.` | `atlas install` received both a package and `--group`. | Pass exactly one of them. |
| `Unknow package group <group>` | No repository package belongs to the requested group. Note the current spelling of the install error. | Check the label and publish a package with that `groups` entry. |
| `Unknown package group <group>` | A group update found no members in the repository or install state. | Check the label and the installed membership. |
| `Package <name> from group <group> is not installed` | A package currently in the group is missing from the install database. | Install the group first, or reinstall the missing member. |
| `No package found in group <group>` | A group uninstall found no installed package with that recorded membership. | Check the installed groups with the query in [install-state.md](install-state.md). |
| `Specify either a package or group to update, not both` | `atlas update` received both a package and `--group`. | Pass exactly one; omit both to update everything. |
| `You should specify either a package or group to uninstall, not both` | `atlas uninstall` received both a package and `--group`. | Pass exactly one. |
| `You should specify a package or group to uninstall` | `atlas uninstall` received neither selection. | Pass a package or `--group`. |
| `no such column: groups` | A repository or install database predates group support. | Back up the database, then `ALTER TABLE packages ADD COLUMN groups TEXT NOT NULL DEFAULT '[]'` in `repo.db`, or `ALTER TABLE installed_packages ADD COLUMN groups TEXT NOT NULL DEFAULT '[]'` in `installed.db`; alternatively recreate the database. |
| `Failed to verify sky file signature` | Artifact signed by a different key than the configured public key. | Use the matching config/keypair; republish if needed. |
| `metadata does not match the expected package` | Repository row and artifact identity disagree. | Republish the correct artifact or fix the row. |
| `Dependency `<dep>` of package `<parent>` not found` | Repository `deps` references a package with no row for that architecture. | Publish the dependency for the same architecture. |
| `Failed to install ...: conflicting file ... already exists` | Another package owns the path, or an unowned file exists there. | Remove the conflicting file/package or change the payload paths. |
| `Path ... conflicts with reserved Atlas state directory /var/lib/atlas` | Payload tries to write Atlas state. | Rename the payload path. |
| `Symlink ... potentially trying to escape the installation directory` | Symlink target resolves outside the target root. | Use a relative target that stays inside the root. |
| `Not allowed to downgrade ...` | Repository version is lower than the installed version. | Publish a newer version or intentionally remove the installed package first. |
| `New ... version declares ... as a dependency but it is not yet installed, try reinstalling ...` | The updated package added a runtime dependency that is absent. | Reinstall the parent package so the dependency is resolved and installed. |
| `Failed to exchange files` | Rename/exchange failed, often permissions or an unexpected filesystem boundary. | Check target permissions; staging must share the target filesystem. |
| Cannot find the install database after a `--root` install | `--root` changes only the target filesystem; the database lives in the user's data directory. | Check `~/.local/share/atlas/installed.db` for the correct user. |

## Verification

| Symptom | Likely cause | Fix |
| --- | --- | --- |
| `Provide a package name or a sky file's path to verify` | `atlas verify` was called with no arguments. | Pass a package name. |
| `Either pass a package name or a sky file's path to verify, not both` | Both positionals were supplied. | Pass exactly one. |
| A path passed to `atlas verify` is treated as a package name | The CLI's single positional fills `PACKAGE`. | Use `atlas verify <name>`; direct path verification is a library API. |
| `Checksum mismatch` during verify | Manifest hash does not match the extracted payload. | Rebuild the package; the artifact is corrupt or was modified. |
| `Invalid record, mode mismatch` | Manifest and payload modes disagree. | Rebuild the package. |
| `manifest references path ... not in payload` or `payload contains path ... not in manifest` | Manifest and payload disagree. | Rebuild the package. |

## Expected non-errors

| Output | Meaning |
| --- | --- |
| `Package <name> is already up to date` | Named update found no newer version. |
| `All packages are up to date` | Bulk update found no candidates. |
| No output from `atlas update --group <group>` | Group update found no newer members; it has no dedicated "up to date" message. |
| `No orphaned packages to remove` | `clean` found nothing to remove. |
| `Transaction aborted` | The user declined a confirmation prompt. |
| `Skipping build.` | `sisyphus build --skip-build` was used; the stub does not run the pipeline. |

## When reporting a bug

Include:

- the exact command,
- the full error output (`atlas`/`sisyphus` print the anyhow error chain),
- the relevant recipe or `.sky` path,
- whether the repository and install databases were freshly created,
- the installed Rust version and whether `bwrap` is available.

Do not include private key material.
