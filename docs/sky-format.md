# Atlas `.sky` package format

Status: draft for version 1.

A `.sky` file is the signed, installable package artifact built by Sisyphus and
installed by Atlas. It contains a filesystem payload, the metadata Atlas needs
to resolve and track it, an exhaustive file manifest, and an Ed25519 signature.

The v1 container is a Zstandard-compressed tar archive. Its filename is:

```text
<name>-<version>-<release>-<architecture>.sky
```

For example: `hello-2.12.1-1-x86_64.sky`.

## Archive layout

Every v1 package contains exactly these top-level entries:

```text
metadata.yaml
payload.tar.zst
files
signature.ed25519
```

No other top-level entry is permitted. Every archive member path must be
relative, normalized, and free of `..` components. Atlas rejects absolute
paths, duplicate names, and malformed archives before any file is installed.

| Entry | Purpose |
| --- | --- |
| `metadata.yaml` | Package identity, dependencies, source/build provenance, and format version. |
| `payload.tar.zst` | Zstandard-compressed tar archive of the staged `$pkgdir` filesystem tree. |
| `files` | Canonical manifest of every path in `payload.tar.zst`. |
| `signature.ed25519` | Ed25519 signature over the canonical signed data. |

## `metadata.yaml`

`metadata.yaml` is UTF-8 YAML with this required v1 structure:

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
  builder: "sisyphus 0.1.0"
```

`format` must be `1`. `name`, `version`, `release`, `architecture`, and `deps`
follow the rules in the recipe specification. `source` records the verified
source used for provenance. `build.recipe_sha256` is the lowercase SHA-256 of
the original recipe bytes; it lets users identify exactly which recipe produced
the package.

`built_at` is informational and uses RFC 3339 UTC. It is not a reproducibility
claim; reproducibility will be addressed separately by controlling build
timestamps and environments.

## Payload rules

`payload.tar.zst` contains the complete staged `$pkgdir` tree. Its paths are
relative to the target filesystem root:

```text
usr/bin/hello
usr/share/man/man1/hello.1
```

It must never contain leading `/`, `..`, or an entry that resolves outside the
installation root. Regular files, directories, symbolic links, and hard links
are allowed. Device nodes, FIFOs, sockets, and setuid/setgid files are rejected
in version 1.

Payload ownership is normalized at package creation to `root:root`. Atlas
installs payload paths below its selected target root—normally `/`—and never
uses archive-supplied ownership outside these rules.

## `files` manifest

`files` is UTF-8 text, one record per payload path, in ascending bytewise path
order. Each line has this tab-separated structure:

```text
<type>\t<mode-octal>\t<sha256-or-link-target>\t<path>\n
```

Examples:

```text
d\t0755\t-\tusr
f\t0755\t7f83b1657ff1fc53b92dc18148a1d65dfa135014\tusr/bin/hello
l\t0777\t../lib/libhello.so.1\tusr/lib/libhello.so
```

Types are `d` for a directory, `f` for a regular file, and `l` for a symbolic
link. Regular-file hashes are lowercase SHA-256 values. Directory hashes are
`-`. A symbolic-link value is its unmodified link target. The manifest must
match the payload exactly; Atlas verifies it before installing.

## Signing

Atlas v1 uses Ed25519 signatures and a trusted distribution public-key ring.
Sisyphus holds the matching private signing key in protected local storage; it
does not embed private key material in a recipe or package.

The signed message is the exact byte concatenation:

```text
SHA-256(metadata.yaml) || SHA-256(payload.tar.zst) || SHA-256(files)
```

where each hash is its 32 raw bytes in the listed order. `signature.ed25519`
contains the 64 raw signature bytes. Future package formats may add a key ID or
signature envelope; version 1 identifies the signer by attempting verification
against the trusted keyring.

Atlas verifies the signature, metadata constraints, manifest, and archive paths
before staging or installing any payload files.

## Atlas installation database

After installation, Atlas stores the package records at:

```text
/var/lib/atlas/local/<name>-<version>-<release>/
```

Each record retains at least the verified `metadata.yaml`, `files`, and package
signature. Atlas uses these records to detect file conflicts, identify file
owners, verify installations, and remove or upgrade packages.

Installation is transactional: Atlas validates the entire package, stages any
required changes, updates the filesystem, then atomically commits its database
record. On failure it must restore the prior filesystem and database state.

## Version 1 boundaries

Version 1 has one architecture (`x86_64`), one signature algorithm (Ed25519),
one trusted-key model (distribution keys), and no package deltas, rollbacks,
or repository index format. Repository metadata and mirror transport are
separate formats to be specified when remote installation is implemented.
