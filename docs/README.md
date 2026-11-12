# Atlas Tools documentation

Status: draft for version 1.

This directory documents the user-facing behavior and the on-disk contracts of
Atlas Tools. The code is the source of truth: if a document disagrees with the
implementation, the implementation wins and the document should be fixed.

## Reading order

| Document | What it covers |
| --- | --- |
| [architecture.md](architecture.md) | System overview, crates, pipelines, data flow, and invariants. |
| [recipe-format.md](recipe-format.md) | The `sky.yaml` recipe schema used by Sisyphus. |
| [build-environment.md](build-environment.md) | Fetching, source caching, dependency planning, and the Bubblewrap sandbox. |
| [sky-format.md](sky-format.md) | The signed `.sky` package container and its metadata, payload, and manifest. |
| [package-groups.md](package-groups.md) | Group labels, membership storage, and group install/update/uninstall behavior. |
| [repository.md](repository.md) | The local repository database, publishing, and artifact lookup. |
| [install-state.md](install-state.md) | The Atlas install database, ownership ledger, and transaction behavior. |
| [security.md](security.md) | Trust model, signature verification, path safety, and limitations. |
| [cli.md](cli.md) | Command-line reference for `atlas` and `sisyphus`. |
| [development.md](development.md) | Building, testing, CI, database changes, and commit conventions. |
| [troubleshooting.md](troubleshooting.md) | Common errors and their causes. |

## Version summary

| Contract | Current version |
| --- | --- |
| Recipe schema | `schema: 1` |
| `.sky` container format | `format: 1` |
| Package signature | Ed25519 over three SHA-256 digests |
| Package groups | Recipe `groups` list; repository and install DB columns; not signed |
| Documented architecture | `x86_64` (the code also parses `arm` for tests) |
| Repository | Local SQLite database; no remote mirror yet |

## What exists today

- `sisyphus` builds packages from recipes and publishes them into a local
  repository database, including their group membership.
- `atlas` installs, verifies, updates, and uninstalls packages from that local
  repository. Packages can be selected by name or by group.
- `common` owns the shared `.sky` types, hashing and signature checks, and the
  repository database schema.

Package groups are part of version 1, but they are local database state rather
than part of the signed package: group membership is not stored in `.sky`
metadata and is not authenticated by the package signature. Groups cannot
contain other groups, and a group is not a dependency edge.

Remote repository transport, package deltas, version constraints, Git sources,
and cross-compilation are not implemented. Treat version 1 as the current
contract, not a stable public specification.
