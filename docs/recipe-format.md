# Sisyphus recipe format

Status: draft for version 1 (`schema: 1`).

A Sisyphus recipe describes how to build one Atlas package. Recipes are stored
in the recipe repository at:

```text
core/<package-name>/sky.yaml
```

The directory name and the `name` field must match. A recipe is declarative
metadata plus small Bash scripts for the build phases. Sisyphus validates a
recipe before it fetches sources or starts a sandbox.

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
| `name` | yes | string | Package identifier. It must match `[a-z0-9][a-z0-9+._-]*`. |
| `version` | yes | string | Upstream package version. It must be non-empty and must not contain `/`, whitespace, or control characters. |
| `release` | yes | positive integer | Atlas packaging revision for this upstream version. |
| `architecture` | yes | string | Target CPU architecture. Version 1 accepts only `x86_64`. |
| `source` | yes | object containing `url` and `sha256` | Sources fetched before the sandbox starts. |
| `makedeps` | no | list of strings | Packages needed only to build or test this package. Defaults to `[]`. |
| `deps` | no | list of strings | Runtime package dependencies embedded in the resulting `.sky` metadata. Defaults to `[]`. |
| `prepare` | no | multiline string | Preparation script. Omitted means no preparation phase. |
| `build` | yes | multiline string | Compilation script. |
| `check` | no | multiline string | Test script. Omitted means no test phase. |
| `package` | yes | multiline string | Script that stages files into `$pkgdir`. |

Unknown fields are rejected in version 1. This prevents misspellings such as
`makedep` from silently changing the package contract.

## Source entries

Each source entry has this version-1 form:

```yaml
sources:
  - url: https://example.org/project-1.0.tar.gz
    sha256: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
```

`url` must use `https`. HTTP, local files, and Git sources are intentionally
out of scope for the first recipe schema; they can be added by a later schema
version with explicit source types and immutable Git revisions.

`sha256` is the lowercase hexadecimal SHA-256 digest of the downloaded bytes.
It must be exactly 64 characters. Sisyphus verifies it before extraction.

Sources are downloaded and checksum-verified outside the build sandbox into an
immutable fetch cache. Sisyphus extracts or copies them into the writable
`$srcdir` working tree before phase scripts run. This preserves the verified
source cache while allowing `prepare` to apply patches or generate files. A
later version may add a `name` field to control the local source filename and
support patches cleanly.

## Dependencies

Dependency entries are package names and use the same grammar as `name`.
Version constraints are deliberately not part of version 1.

- `makedeps` are resolved and installed in the isolated build root before any
  phase executes. They are not included as runtime dependencies.
- `deps` are runtime dependencies. They are written into the `.sky` package
  metadata and must be satisfied by Atlas before installation.

All dependencies are resolved as a graph. Cycles are errors. Duplicate entries
within either list are errors. A dependency may not name the package being
built.

## Build environment and phases

Sisyphus invokes each present phase as a Bash script using:

```text
bash -euo pipefail -c <phase-script>
```

The scripts execute in a fresh Bubblewrap sandbox. The sandbox has no network
access after Sisyphus has fetched and verified the declared sources. It does
not expose the builder's home directory or the live host filesystem.

The following variables are provided:

| Variable | Meaning |
| --- | --- |
| `srcdir` | Writable working tree containing extracted verified sources. |
| `builddir` | Writable directory for generated build files. |
| `pkgdir` | Empty writable staging root for package payload files. |
| `name` | Recipe package name. |
| `version` | Recipe upstream version. |
| `release` | Recipe package release. |
| `ARCH` | Target architecture (`x86_64` in version 1). |

Phases run in order: `prepare`, `build`, `check`, then `package`. A phase
failure ends the build and no package is produced. `srcdir` is not required to
be the working directory; recipes should `cd` explicitly.

`package` must write all payload files below `$pkgdir`. A normal upstream
installation to `/usr/bin/example` is staged as
`$pkgdir/usr/bin/example`, commonly using `DESTDIR="$pkgdir"`. Sisyphus
rejects attempts to package absolute paths, parent-directory traversal, or
paths outside `$pkgdir`.

## Result identity

The resulting package identity is:

```text
<name>-<version>-<release>-<architecture>
```

For the example, it is `hello-2.12.1-1-x86_64`. The generated artifact uses
that identity as its filename with a `.sky` suffix.

## Version 1 boundaries

Version 1 intentionally does not define split packages, optional dependencies,
version constraints, Git sources, patches, build options, cross-compilation,
or non-x86_64 targets. Such features require a schema-version increment or a
backwards-compatible extension with explicit validation rules.
