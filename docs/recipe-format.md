# Sisyphus recipe format

Status: draft for version 1 (`schema: 1`).

A recipe describes how to build one Atlas package. It is declarative metadata
plus small Bash scripts for the build phases. Sisyphus validates a recipe before
it fetches sources or starts a sandbox.

## Location and discovery

Recipes must be stored at:

```text
<recipe-repository>/<package-name>/sky.yaml
```

Sisyphus derives the recipe repository root as the second ancestor of the
canonicalized recipe path. Dependency recipes are looked up at:

```text
<recipe-repository>/<dependency-name>/sky.yaml
```

The directory name is expected to match the `name` field. The current validator
checks the name grammar but does not compare it with the directory name.

## Complete example

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

groups:
  - desktop

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

## Top-level fields

| Field | Required | Type | Meaning |
| --- | --- | --- | --- |
| `schema` | yes | positive integer | Recipe-format version. Version 1 must be `1`. |
| `name` | yes | string | Package identifier. Must match `[a-z0-9][a-z0-9+._-]*`. |
| `version` | yes | string | Upstream package version. Must be non-empty and contain no `/`, whitespace, or control characters. |
| `release` | yes | positive integer | Atlas packaging revision for this upstream version. |
| `architecture` | yes | string | Target CPU architecture. Version 1 documents `x86_64`. |
| `source` | yes | object | Source with `url` and `sha256`. See [Source entries](#source-entries). |
| `makedeps` | no | list of strings | Build-time dependencies. Defaults to `[]`. |
| `deps` | no | list of strings | Runtime dependencies embedded in the resulting `.sky` metadata. Defaults to `[]`. |
| `groups` | no | list of strings | Group labels this package belongs to. Stored in the repository database, not in `.sky` metadata. Defaults to `[]`. |
| `prepare` | no | multiline string | Preparation script. Omitted means no preparation phase. |
| `build` | no | multiline string | Compilation script. Omitted means no build phase. |
| `check` | no | multiline string | Test script. Omitted means no test phase. |
| `package` | yes | multiline string | Script that stages payload files under `$pkgdir`. Must be non-empty. |

## Field details

### `schema`

Only version `1` is accepted. A missing `schema` or any value other than `1`
fails validation.

### `name`

The first character must be an ASCII lowercase letter or digit. The remaining
characters may be ASCII lowercase letters, digits, `+`, `.`, `_`, or `-`.

Valid examples: `hello`, `libc++`, `python3.13`, `foo_bar`, `gcc-14`.

### `version`

The version is recorded verbatim. It must be non-empty and must not contain `/`,
whitespace, or control characters. It is not required to be valid SemVer; Atlas
only parses installed/repository versions as SemVer during `update`.

### `release`

The packaging revision for the upstream version. The v1 contract treats this as
a positive integer. The resulting package identity uses it verbatim.

### `architecture`

Version 1 documents `x86_64`. The shared `Architectures` type also parses
`arm`, which is used by tests; do not treat `arm` as a supported v1 target.

### Source entries

Version 1 has exactly one source object:

```yaml
source:
  url: https://example.org/project-1.0.tar.gz
  sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
```

| Field | Required | Meaning |
| --- | --- | --- |
| `url` | yes | HTTP(S) URL. The contract requires HTTPS; the fetcher also accepts HTTP for loopback tests. Must not be empty. |
| `sha256` | yes | Lowercase hexadecimal SHA-256 of the downloaded bytes. The v1 contract requires exactly 64 characters. |

Sources are fetched and checksum-verified outside the build sandbox. The
verified archive is cached and then extracted or copied into the writable
`$srcdir` working tree before phases run. This preserves the verified cache while
allowing `prepare` to patch or generate files.

Git sources are not supported. A URL whose path ends in `.git` is rejected with
an explicit error. `file://` and other non-HTTP schemes are rejected.

### `makedeps`

Packages needed only to build or test this package.

- They are resolved recursively into a dependency-first build plan.
- Missing dependencies are built before the root package.
- Already-published, exact-identity artifacts are reused after signature and
  metadata verification.
- `makedeps` are not written to the resulting `.sky` metadata as runtime
  dependencies.
- A package may not list itself.

### `deps`

Runtime dependencies.

- They are written into the resulting `.sky` metadata.
- Atlas resolves installation order from the repository database's `deps`
  column, not from `.sky` metadata.
- A package may not list itself.

Version constraints are intentionally absent in version 1.

### `groups`

Optional labels that let Atlas operate on a set of packages at once. Defaults to
`[]`.

- Group names use the same grammar as package names:
  `[a-z0-9][a-z0-9+._-]*`.
- The list must not contain duplicates, and a package may not list its own name
  as a group.
- Labels do not need to be declared anywhere else; a group exists implicitly as
  long as at least one repository row uses it.
- A group is not a dependency. Group labels do not affect the build plan,
  `deps`, `makedeps`, or the resulting `.sky` metadata.
- Membership is written to the repository database's `groups` column for the
  package's `(name, architecture)` row. Republishing replaces that row and
  therefore replaces the package's complete membership list.
- Atlas records the membership in its install database at install time and uses
  that snapshot for group uninstall operations.

See [package-groups.md](package-groups.md) for install, update, and uninstall
semantics.

### Phases

`prepare`, `build`, and `check` are optional. `package` is required.

Phases run in this order:

1. `prepare`
2. `build`
3. `check`
4. `package`

Each present phase is executed as:

```text
bash -euo pipefail -c <phase-script>
```

inside a Bubblewrap sandbox. A nonzero exit status ends the build and no package
is produced. `$srcdir` is not the working directory; recipes must `cd`
explicitly.

The following variables are set:

| Variable | Meaning |
| --- | --- |
| `srcdir` | Writable working tree containing the extracted, verified source. |
| `builddir` | Writable directory for generated build files. |
| `pkgdir` | Empty writable staging root for package payload files. |
| `name` | Recipe package name. |
| `version` | Recipe upstream version. |
| `release` | Recipe package release. |
| `ARCH` | Target architecture. |

`package` must write all payload files below `$pkgdir`. A normal installation to
`/usr/bin/example` is staged as `$pkgdir/usr/bin/example`, commonly via
`DESTDIR="$pkgdir"`. Files written outside `$pkgdir` are not packaged.

See [build-environment.md](build-environment.md) for sandbox mounts, dependency
artifacts, networking, and filesystem details.

## Dependency rules

Dependency names use the same grammar as `name`. Version constraints, optional
dependencies, and alternative providers are not part of version 1. Group labels
are not dependency names and are never resolved as dependencies.

The resolver:

- traverses `makedeps` depth-first and emits dependencies before dependents,
- traverses the runtime dependencies of dependencies that must be built,
- detects cycles on the active traversal path,
- reuses a local artifact only when name, version, release, and architecture
  match exactly,
- errors when a dependency recipe or repository record is missing.

Current parser behavior:

- Duplicate entries are tolerated. `makedeps` are sorted and deduplicated before
  traversal.
- `groups` is the exception: duplicate group labels are rejected during recipe
  validation, and a missing `groups` key defaults to an empty list.
- Unknown top-level fields are currently ignored by the YAML deserializer.
  They are not part of the v1 contract and may be rejected by a later version.
- `release: 0` is currently accepted because there is no explicit zero check.
- `architecture: arm` is currently accepted because the shared enum has an
  `arm` variant used by tests.

## Result identity

The resulting package identity is:

```text
<name>-<version>-<release>-<architecture>
```

For the example, it is `hello-2.12.1-1-x86_64`. The generated artifact uses that
identity as its filename with a `.sky` suffix.

## Version 1 boundaries

Version 1 intentionally does not define:

- split packages,
- optional dependencies,
- version constraints,
- patches,
- build options or feature flags,
- Git sources,
- cross-compilation,
- non-`x86_64` targets as supported targets,
- nested groups or group dependencies,
- signed group membership.

Such features require a schema-version increment or a backwards-compatible
extension with explicit validation rules.
