# Build environment

Status: draft for version 1.

This document describes what Sisyphus does between reading a recipe and writing
a `.sky` file: source fetching, caching, extraction, dependency planning, the
Bubblewrap sandbox, and packaging.

## Source fetching

Sisyphus fetches sources with a blocking HTTP client.

- Only `http` and `https` URLs are accepted.
- A URL whose path ends in `.git` is rejected with an explicit error. Git
  sources are not supported in version 1.
- The downloaded filename comes from the final response URL's last path segment,
  or `tmp.bin` when the response URL has no filename.
- Files are written under:

  ```text
  ~/.cache/sisyphus/sources/<package-name>/<filename>
  ```

- The existing file at that path is replaced. A failure after file creation can
  leave an empty or partial file behind.

### Source cache

The build path uses a SQLite cache at `~/.cache/sisyphus/cache.db` with this
lazily created table:

```sql
CREATE TABLE IF NOT EXISTS sources (
    url TEXT PRIMARY KEY,
    sha256 TEXT NOT NULL,
    path TEXT NOT NULL,
    fetched_at TEXT NOT NULL
);
```

On a build:

1. If the cache database exists, look up the recipe's `url`.
2. A hit is reused only when the stored `sha256` matches the recipe and the
   stored file still exists.
3. A reused file is hashed again and must match the recipe.
4. On a miss, Sisyphus downloads the source, verifies its SHA-256, and replaces
   the cache row.

`checksum-gen` uses a separate fetch mode: it downloads the source and prints the
digest, but does not read or write the cache database. It can overwrite the
cached file for that package.

### Checksum verification

The computed SHA-256 is lowercase hex and must match `source.sha256` exactly.
Mismatches fail with:

```text
Checksum mismatch: expected <expected>, got <computed>
```

## Extraction

Sisyphus uses `compress-tools` (libarchive) to extract the verified archive into
a temporary directory with ownership ignored.

It then chooses the source root:

- if the extraction directory contains exactly one readable top-level entry and
  that entry is a directory, that directory becomes the source root,
- otherwise the extraction directory itself is the source root.

The selected root is mounted at `/srcdir` and exposed as `$srcdir`. `prepare` can
modify this tree; the verified cache is not modified.

## Dependency planning

Only `makedeps` start the build dependency traversal. The root package's `deps`
are recorded in the final metadata and are not built for the root unless they are
also reached as a dependency of something that must be built.

The resolver performs a depth-first traversal:

1. Read the dependency's recipe from `<recipe-repo>/<dep>/sky.yaml`.
2. Look up an exact `(name, version, release, architecture)` artifact in the
   local repository database.
3. If a matching row exists, verify its signature and metadata, add the artifact
   to the reusable set, and recursively resolve the artifact's recorded runtime
   dependencies.
4. Otherwise, recursively resolve the dependency's `makedeps` and `deps`, then
   append its recipe to the build order.
5. Reject cycles on the active traversal path and missing recipes.

The build plan is ordered so dependencies are built before dependents. Builds
execute in that order, and each newly built artifact is added to the artifact set
available to later builds.

Artifact reuse is exact: there is no version-range or compatibility solver. A
different version or release is rebuilt from its recipe.

## Sandbox

Phase scripts run with Bubblewrap. `bwrap` must be on `PATH`.

The sandbox is constructed as follows:

| Mount / setting | Value |
| --- | --- |
| Read-only `/usr` | Temporary directory populated from dependency artifacts |
| `/bin` | Symlink to `/usr/bin` |
| `/lib` | Symlink to `/usr/lib` |
| `/lib64` | Symlink to `/usr/lib64` |
| `/sbin` | Symlink to `/usr/sbin` |
| Writable `/srcdir` | Extracted source tree |
| Writable `/builddir` | Temporary build directory |
| Writable `/pkgdir` | Empty temporary package staging directory |
| Network | Unshared with `--unshare-net` |

The host `/usr`, home directory, and live filesystem are not exposed. The
sandbox does not explicitly mount the host's `/proc`, `/dev`, or `/tmp`; a
dependency artifact can provide those paths if needed.

Because the host `/usr` is hidden, commands used by phases—including `bash`
itself—must come from the dependency artifact set. In practice a recipe needs
`makedeps` that provide a shell and the tools its phases invoke.

### Environment

| Variable | Value |
| --- | --- |
| `srcdir` | `/srcdir` |
| `builddir` | `/builddir` |
| `pkgdir` | `/pkgdir` |
| `name` | Recipe name |
| `version` | Recipe version |
| `release` | Recipe release |
| `ARCH` | Target architecture |

### Phase execution

Each present phase is executed as:

```text
bash -euo pipefail -c <phase-script>
```

Phases run in `prepare`, `build`, `check`, `package` order. A nonzero exit status
aborts the build. Temporary directories are owned by `tempfile` and are removed
when they go out of scope.

`package` must stage files below `$pkgdir`. Writing to absolute paths such as
`/usr/bin/foo` writes inside the sandbox and will not be included in the
payload; use `DESTDIR="$pkgdir"` or an equivalent staging prefix.

## Packaging

After the root package's phases succeed:

1. `payload_gen` walks `$pkgdir` and builds `payload.tar.zst`.
   - Regular files, directories, and symlinks are supported.
   - Payload ownership is normalized to `root:root`.
   - Manifest modes are masked to `0o7777`.
2. `metadata_gen` records `format: 1`, identity, runtime `deps`, source
   provenance, the recipe's SHA-256, an RFC 3339 timestamp, and the builder
   string.
3. `sign_artifacts` hashes the serialized metadata, payload, and manifest and
   signs their raw digests with the configured private key.
4. `sky_gen` writes the four-entry archive to:

   ```text
   ~/.local/share/sisyphus/repo/<architecture>/<name>-<version>-<release>-<architecture>.sky
   ```

5. The package is published into the repository database. Publishing replaces
   the existing row for `(name, architecture)`.

## Build skipping

Before fetching anything, `run_build` checks the repository database. If a row
exists for the exact `(name, version, release, architecture)` and its `sky_path`
still exists on disk, Sisyphus prints:

```text
Package "<name>" already exists, skipping build
```

and exits successfully. If the row exists but the artifact is gone, the build
proceeds and publishing replaces the row.

## Current limitations

- No Git sources, patches, or multiple sources.
- No network access during phases; all source fetching happens beforehand.
- No implicit host toolchain. Dependencies must provide the shell and tools.
- Artifact reuse requires an exact identity match.
- No sandbox for package installation; Atlas validates paths and writes as the
  invoking user.
- No build reproducibility guarantee.
