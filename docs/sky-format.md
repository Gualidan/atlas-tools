# Atlas `.sky` package format

Status: draft for version 1.

A `.sky` file is the signed, installable package artifact built by Sisyphus and
installed by Atlas. It contains a filesystem payload, the metadata Atlas needs
to identify and track the package, an exhaustive file manifest, and an Ed25519
signature.

## Filename

```text
<name>-<version>-<release>-<architecture>.sky
```

For example: `hello-2.12.1-1-x86_64.sky`.

The identity in the filename must match the metadata identity. Atlas installs
only when the repository row and the metadata identity agree exactly on name,
version, release, and architecture.

## Container layout

A version 1 package is a Zstandard-compressed tar archive containing exactly
these four top-level entries:

```text
metadata.yaml
payload.tar.zst
files
signature.ed25519
```

No other top-level entry is permitted. Archive member paths must be relative and
must not contain `.` or `..` components. Atlas rejects absolute paths, duplicate
members, unexpected members, and missing required members before installing any
payload files.

| Entry | Purpose |
| --- | --- |
| `metadata.yaml` | Package identity, runtime dependencies, source/build provenance, and format version. |
| `payload.tar.zst` | Zstandard-compressed tar archive of the staged `$pkgdir` filesystem tree. |
| `files` | Canonical manifest of every path in `payload.tar.zst`. |
| `signature.ed25519` | 64-byte Ed25519 signature over the canonical signed data. |

The current archive validator keys on each member's filename rather than
requiring depth one. The writer always emits the four files at the archive root;
producers should not rely on the looser behavior.

## `metadata.yaml`

`metadata.yaml` is UTF-8 YAML. A version 1 package has this structure:

```yaml
format: 1
name: hello
version: 2.12.1
release: 1
architecture: x86_64
deps:
  - glibc

source:
  url: https://ftp.gnu.org/gnu/hello/hello-2.12.1.tar.gz
  sha256: "<source SHA-256>"

build:
  recipe_sha256: "<SHA-256 of the input sky.yaml bytes>"
  built_at: "2026-08-28T12:00:00Z"
  builder: "sisyphus 0.2.0"
```

| Field | Meaning |
| --- | --- |
| `format` | Container format version. The v1 contract requires `1`. |
| `name` | Package name. |
| `version` | Upstream version. |
| `release` | Atlas packaging revision. |
| `architecture` | Target architecture. |
| `deps` | Runtime dependency names. |
| `source.url` | Verified source URL. |
| `source.sha256` | Verified source SHA-256. |
| `build.recipe_sha256` | Lowercase SHA-256 of the original recipe bytes. |
| `build.built_at` | RFC 3339 UTC build timestamp. Informational only. |
| `build.builder` | Builder identity string, currently `sisyphus <version>`. |

`built_at` is not a reproducibility claim. Reproducibility would require
controlling build timestamps and environments and is not implemented.

The current verifier checks metadata identity and signature but does not
explicitly reject `format` values other than `1`. Producers must still write
`1`.

`groups` is intentionally absent from `metadata.yaml`. Group membership is
per-package repository state stored in the `groups` column of the repository
database, and Atlas records a copy in its install database. It is therefore not
covered by the package signature. A validly signed package can be republished
with different group membership without changing the `.sky` bytes.

## Payload rules

`payload.tar.zst` contains the complete staged `$pkgdir` tree. Paths are relative
to the target filesystem root:

```text
usr/bin/hello
usr/share/man/man1/hello.1
```

Allowed entry types in version 1:

- regular files,
- directories,
- symbolic links,
- hard links (accepted by the path validator; see implementation notes).

Rejected entry types and states:

- device nodes,
- FIFOs,
- sockets,
- setuid or setgid modes,
- absolute paths,
- paths containing `.` or `..` components,
- duplicate paths.

Payload ownership is normalized at package creation to `root:root`. Atlas
installs paths below its selected target root and does not trust archive-supplied
ownership outside those rules.

The current writer emits regular files, directories, and symbolic links only.
It does not emit hard-link entries.

### Symlinks

A symbolic link's target is recorded verbatim in the manifest. At install time,
Atlas rejects a symlink whose target would escape the install root or whose path
contains `..`. Standalone `verify` compares the target with the manifest but does
not perform that escape check.

## `files` manifest

`files` is UTF-8 text with one record per payload path, in ascending bytewise
path order. Each line is tab-separated with this structure:

```text
<type>\t<mode-octal>\t<sha256-or-link-target>\t<path>\n
```

| Type | Meaning | Third field |
| --- | --- | --- |
| `d` | Directory | Literal `-` |
| `f` | Regular file | Lowercase SHA-256 hex digest |
| `l` | Symbolic link | Unmodified link target |

Modes are written as four octal digits, masked to `0o7777`.

Examples:

```text
d\t0755\t-\tusr
d\t0755\t-\tusr/bin
f\t0755\t7f83b1657ff1fc53b92dc18148a1d65dfa135014\tusr/bin/hello
l\t0777\t../lib/libhello.so.1\tusr/lib/libhello.so
```

The manifest must match the payload exactly. Atlas verifies both directions:

- every manifest path must exist in the payload,
- every payload path must appear in the manifest,
- regular-file hashes and modes must match,
- directory modes must match,
- symlink targets and modes must match.

There is no manifest record type for hard links. A package that includes hard
links will fail the bidirectional manifest check even though the payload path
validator accepts the entry type.

## Signing

Version 1 uses Ed25519.

The signed message is the exact byte concatenation:

```text
SHA-256(metadata.yaml) || SHA-256(payload.tar.zst) || SHA-256(files)
```

Each hash is the 32 raw digest bytes, in that order. `signature.ed25519`
contains the 64 raw signature bytes.

Sisyphus holds the private signing key in a file referenced by
`priv_key_path` in the Sisyphus config. Atlas verifies with the single public key
referenced by `pub_key_path` in the same config. There is no key ID, keyring,
rotation, or revocation mechanism in version 1.

## Verification

Atlas verifies a package before staging any payload files.

### By package name

`atlas verify <name>` (and every install/update) performs:

- repository lookup for the host architecture,
- outer archive entry validation,
- signature verification with the configured public key,
- metadata identity comparison against the repository row,
- payload path validation,
- extraction into a temporary directory,
- bidirectional manifest/payload checks including modes, hashes, and symlink
  targets.

### By direct path

The library verifier can also accept a `.sky` path through
`VerifyArgs.sky_path`. In that mode there is no expected repository identity, so
signature and structural checks run without a metadata identity comparison. The
current CLI exposes two optional positionals (`[PACKAGE] [SKY_PATH]`) but exactly
one must be supplied; a single positional argument fills `PACKAGE`. See
[cli.md](cli.md#atlas-verify).

### What verification does not cover

- It does not enforce `metadata.format == 1`.
- It does not check manifest sort order.
- It does not authenticate the repository database, its `sky_path` values, or
  package group membership.
- It does not resolve symlink targets for escape during standalone `verify`;
  install-time commit performs that check.
- It does not verify build reproducibility or the truth of `source`/`build`
  provenance fields beyond the signature.

## Installation state

Package files are installed below a target root—normally `/`—and ownership is
tracked in the Atlas install database under the user's data directory:

```text
~/.local/share/atlas/installed.db
```

The previous `/var/lib/atlas/local/...` layout described by earlier revisions of
this document is obsolete. See [install-state.md](install-state.md) for the
current schema and transaction behavior.

## Version 1 boundaries

Version 1 has:

- one documented target architecture (`x86_64`),
- one signature algorithm (Ed25519),
- one trusted-key model (a single configured public key),
- no package deltas,
- no rollback archives,
- no package group metadata,
- no repository index format,
- no remote transport.

Repository metadata and mirror transport are separate concerns to be specified
when remote installation is implemented.
