# Local repository

Status: draft for version 1.

Atlas Tools has no remote repository or mirror yet. Sisyphus publishes into a
local directory that contains a SQLite database and the built artifacts. Atlas
reads that same directory directly.

## Layout

```text
~/.local/share/sisyphus/repo/
├── repo.db
├── x86_64/
│   └── hello-2.12.1-1-x86_64.sky
└── arm/
    └── ...
```

| Path | Purpose |
| --- | --- |
| `repo.db` | SQLite database with one row per publishable package. |
| `<architecture>/` | Built `.sky` artifacts named by package identity. |

Both binaries derive these paths from the invoking user's passwd home directory,
not `$HOME`. When running as root, `SUDO_USER` must be set.

## Schema

The table is created lazily with `CREATE TABLE IF NOT EXISTS`:

```sql
CREATE TABLE IF NOT EXISTS packages (
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    release INTEGER NOT NULL,
    architecture TEXT NOT NULL,
    sky_path TEXT PRIMARY KEY,
    recipe_sha256 TEXT NOT NULL,
    built_at TEXT NOT NULL,
    builder TEXT NOT NULL,
    deps TEXT NOT NULL,
    makedeps TEXT NOT NULL,
    groups TEXT NOT NULL,
    UNIQUE (name, architecture)
);
```

| Column | Meaning |
| --- | --- |
| `name` | Package name. |
| `version` | Upstream version string. |
| `release` | Atlas packaging revision. |
| `architecture` | `x86_64` or `arm`. |
| `sky_path` | Absolute path to the `.sky` artifact, stored as text. |
| `recipe_sha256` | SHA-256 of the recipe bytes that produced the artifact. |
| `built_at` | RFC 3339 build timestamp. |
| `builder` | Builder identity string. |
| `deps` | Runtime dependencies as a serialized YAML list. |
| `makedeps` | Build dependencies as a serialized YAML list. |
| `groups` | Package group labels as a serialized YAML list. |

`deps`, `makedeps`, and `groups` are YAML strings, not JSON. Atlas parses `deps`
when resolving install order and `groups` when selecting packages by group.

Group membership is scoped to the row's `(name, architecture)`. It is not part of
the `.sky` artifact and is not covered by the package signature.

There is no migration system. Schema changes need explicit handling for existing
databases. A repository database created before group support has no `groups`
column, and the current `CREATE TABLE IF NOT EXISTS` schema creation does not add
it.

## Publishing

`sisyphus build` writes the `.sky` file first, then publishes the row. Publishing
is a delete-then-insert transaction:

```sql
DELETE FROM packages WHERE name = ? AND architecture = ?;
INSERT INTO packages (...) VALUES (...);
```

Consequences:

- The repository holds **one row per `(name, architecture)`**, regardless of
  version or release.
- Publishing a newer version replaces the row pointer; the previous `.sky` file
  remains on disk but is no longer referenced.
- Publishing also replaces the package's group membership for that architecture.
  Omitting a group from the new recipe removes it from the repository.
- The database does not track multiple versions of the same package for one
  architecture.
- If writing the `.sky` succeeds and the database insert fails, an unreferenced
  artifact can remain on disk.

The row's `sky_path` is the artifact path Atlas later opens. Atlas does not
verify that the path is inside the repository directory.

## How Atlas uses the repository

### Install

`atlas install <name>`:

1. Looks up `name` for the host architecture.
2. Errors with `Package <name> not found` when no row exists.
3. Reads `deps` from the row and resolves the runtime dependency graph.
4. Skips dependencies that are already installed; reinstalls the requested
   package.
5. Verifies each `.sky` artifact before staging.

Resolution uses the repository's `deps` column, **not** the `.sky` metadata.
Tests deliberately publish rows whose dependencies disagree with the artifact to
ensure Atlas does not trust metadata for this decision.

### Group install

`atlas install --group <group>`:

1. Scans repository rows for exact membership in the requested group.
2. Resolves every member plus its missing runtime dependencies.
3. Produces one dependency-first plan with each package once.
4. Rejects the plan before installing anything if a member, dependency, or cycle
   is invalid.
5. Installs group members as `explicit`; already-installed dependencies are
   skipped.

Group installation uses repository membership and writes the resulting groups to
the install database along with each package record. See
[package-groups.md](package-groups.md).

### Verify

`atlas verify <name>` looks up the host-architecture row and verifies the
artifact at `sky_path`, including metadata identity comparison against the row.

### Update

`atlas update` compares installed versions with repository versions using
SemVer:

- a newer repository version produces an update candidate,
- an equal version is skipped even when release numbers differ,
- a lower repository version is a downgrade error,
- named updates also include the named package's repository dependencies,
- bulk updates visit installed packages in name order,
- repository lookups use the host architecture, even for installed rows recorded
  for another architecture.

If a new version declares a dependency that is not installed, the update fails
with a message advising a reinstall of the parent package.

Group updates select the union of current repository membership and installed
membership recorded for the host architecture. They then use the same version
rules and dependency resolution as named updates. Membership is refreshed only
for packages that are actually updated.

### Build skipping

Before building, Sisyphus checks for an exact
`(name, version, release, architecture)` row. The build is skipped only when the
row exists and its `sky_path` exists on disk.

## Source cache is separate

The repository database is not the source cache. The source cache lives at:

```text
~/.cache/sisyphus/cache.db
~/.cache/sisyphus/sources/
```

It is keyed by URL and stores the expected digest, file path, and fetch time. See
[build-environment.md](build-environment.md) for its schema and lookup rules.

## Current limitations

- No remote repository, mirror, index signature, or transport.
- No multiple versions per `(name, architecture)`.
- No repository-level metadata signing. Group membership is unsigned local
  database state.
- No separate group table or group index; membership is a YAML list on each
  package row.
- No garbage collection for unreferenced artifacts.
- No transaction spanning the artifact file write and the database insert.
- No migration support for schema changes.
