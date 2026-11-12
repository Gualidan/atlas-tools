# Security model

Status: draft for version 1.

This document describes what Atlas Tools authenticates, what it validates, and
what it does not protect against. It is a description of the current
implementation, not a formal security audit.

## Trust model

There are three separate trust decisions:

1. **Recipe trust** — Sisyphus executes recipe scripts inside Bubblewrap. Anyone
   who can change a recipe can change what the build runs.
2. **Package authenticity** — Sisyphus signs the package; Atlas verifies the
   signature with a configured public key.
3. **Local state trust** — Atlas trusts its local repository and install
   databases. Neither database is signed.

The distribution model is currently single-key and local:

- Sisyphus signs with the private key at `priv_key_path`.
- Atlas verifies with the public key at `pub_key_path`.
- Both paths come from the same Sisyphus config YAML.
- There is no key ID, keyring, rotation, revocation, or threshold signing.

If the signing key is compromised, an attacker can produce packages that Atlas
accepts.

## Key management

`sisyphus keygen` generates an Ed25519 keypair.

- The private key is written with mode `0600`.
- The public key is written next to it with a `.pub` extension and mode `0644`.
- Existing key files are never overwritten.
- The config file records `priv_key_path` and `pub_key_path`.

The default key directory is `~/.config/sisyphus/keys/`. The config is loaded from
`$SISYPHUS_CONFIG_DIR/config.yaml` when set, otherwise
`~/.config/sisyphus/config.yaml`.

The config file itself is plain YAML. Protect it if the private key path points
to a sensitive location.

## What is signed

The signed message is:

```text
SHA-256(metadata.yaml) || SHA-256(payload.tar.zst) || SHA-256(files)
```

Each digest is 32 raw bytes. `signature.ed25519` is the 64-byte Ed25519
signature.

This authenticates the metadata bytes, the compressed payload bytes, and the
manifest bytes as a unit. It does not sign the outer tar headers or the filename.

Atlas additionally checks that the metadata identity matches the repository row
when verifying by package name. That check prevents a validly signed package for
one identity from being installed under another identity in the local
repository.

## Source integrity

Sisyphus verifies the downloaded source against the recipe's SHA-256 before
extraction. Cached sources are hashed again before reuse.

Limitations:

- The recipe's expected digest is trusted once the recipe is trusted.
- The contract requires HTTPS, but the fetcher also accepts HTTP. Plain HTTP is
  useful only for loopback tests and should not be used for distribution.
- The source cache database is not signed. A local attacker who can replace both
  the cached file and the cache database still has to produce bytes matching the
  recipe's digest.
- There is no signature on the source archive itself; integrity depends on the
  recipe digest.

## Build isolation

Phase scripts execute inside Bubblewrap with:

- an unshared network namespace,
- a read-only `/usr` assembled from dependency artifacts,
- writable `/srcdir`, `/builddir`, and `/pkgdir`,
- symlinks from `/bin`, `/lib`, `/lib64`, and `/sbin` into `/usr`,
- no host home directory or live host root.

This limits what a recipe can read and write during the build, but it is not a
general-purpose sandbox guarantee:

- The recipe author controls the scripts that run.
- Dependency artifacts are trusted inputs and can provide arbitrary tools.
- `bwrap` configuration and kernel namespace support determine the actual
  isolation boundary.
- Builds are not reproducible; `built_at` is informational.

## Package validation

Before installing payload files, Atlas validates the container and payload.

### Outer archive

- Member paths must be relative.
- Member paths must not contain `.` or `..` components.
- Only `metadata.yaml`, `payload.tar.zst`, `files`, and `signature.ed25519` are
  allowed as member filenames.
- The four expected members must be present exactly once.

### Payload archive

- Entry paths must be relative.
- Entry paths must not contain `.` or `..` components.
- Duplicate paths are rejected.
- Allowed types are regular files, directories, symlinks, and hard links.
- Device nodes, FIFOs, sockets, and other unexpected types are rejected.
- Setuid and setgid modes are rejected.

### Manifest and metadata

- The signature is verified before installation.
- Metadata identity must match the expected package row.
- Every manifest path must exist in the payload.
- Every payload path must appear in the manifest.
- File hashes, modes, directory modes, and symlink targets are compared.

The current verifier does not explicitly enforce `metadata.format == 1`, does
not check manifest sort order, and does not check symlink escapes during
standalone `verify`.

## Install-time filesystem safety

Install and update commit add further checks:

- Every manifest path is checked for absolute paths and unsafe components.
- Writes are rejected if any parent component is a symlink.
- Symlink targets are rejected when they would escape the install root.
- Destination type conflicts are rejected before mutation.
- A non-directory path owned by another package is a conflict.
- An unowned existing non-directory is a conflict.
- `/var/lib/atlas` is reserved and cannot be written by a payload.
- Swapped and created entries are recorded for rollback if database recording
  fails.

Installation itself is **not** sandboxed. Atlas writes files as the invoking
user. Installing into `/` requires appropriate privileges, and running Atlas as
root means a malicious or buggy package can affect the host within the limits of
the checks above.

`--root <dir>` is a convenience for alternate roots and testing, not a security
boundary. The install database remains in the user's home directory, and the
target root is not chrooted.

## Known limitations

- A single signing key protects all packages; there is no revocation.
- No key ID or trust ring; key selection is by config path.
- The repository database is unsigned. A local attacker who can edit it can
  change which artifact path Atlas opens, although the artifact still needs a
  valid signature.
- `sky_path` is not constrained to the repository directory.
- The source cache and install database are not signed.
- Uninstall removes owned paths without a rollback log.
- There is no remote transport, mirror authentication, or index signature.
- No formal audit or hardened privilege separation exists.

## Operational recommendations

- Keep the private signing key offline when not building, and restrict access to
  `priv_key_path`.
- Use HTTPS sources for real packages.
- Review recipes before building them; the recipe controls the build scripts.
- Use `atlas verify <name>` before installing unfamiliar packages.
- Test packages with `atlas install --root <dir>` before installing to `/`.
- Do not run Atlas as root unless the package and its signing key are trusted.
