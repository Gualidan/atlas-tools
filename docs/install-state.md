# Install state and transactions

Status: draft for version 1.

Atlas keeps installed-package state in a SQLite database under the user's data
directory:

```text
~/.local/share/atlas/installed.db
```

This database is user state. `atlas install --root <dir>` changes only the
target filesystem; it does not move the database under the target root.

## Target root and install directory

Every installed package records an `install_dir`, which is the target root used
at install time—normally `/`.

- `atlas install <name> --root <dir>` records `<dir>`.
- `atlas update` reuses each package's recorded `install_dir`.
- `atlas uninstall` removes files relative to each package's recorded
  `install_dir`.
- `/` and alternate roots are treated the same way; no chroot is involved.

Payload paths are relative to this root. A payload path `usr/bin/hello` is
installed at `<install_dir>/usr/bin/hello`.

`ATLAS_INSTALL_DIR` is a legacy test-only environment variable. Production
installs ignore it; use `--root` or the default `/`.

## Schema

The schema is created lazily with `CREATE TABLE IF NOT EXISTS`.

### `installed_packages`

```sql
CREATE TABLE IF NOT EXISTS installed_packages (
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    release INTEGER NOT NULL,
    architecture TEXT NOT NULL,
    sky_path TEXT NOT NULL,
    groups TEXT NOT NULL,
    install_dir TEXT NOT NULL,
    installed_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    install_reason TEXT NOT NULL,
    PRIMARY KEY (name, architecture)
);
```

`install_reason` is either `explicit` (requested by the user) or `dependency`
(installed automatically). The primary key allows one installed version per
package and architecture.

`groups` is a serialized YAML list of the package's group labels at install or
update time. It is a snapshot of repository membership, not a live view. Package
uninstall does not use it. Group uninstall does. Group update treats it as one of
two membership sources. See [package-groups.md](package-groups.md).

### `installed_packages_update_log`

```sql
CREATE TABLE IF NOT EXISTS installed_packages_update_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    architecture TEXT NOT NULL,
    version TEXT NOT NULL,
    release INTEGER NOT NULL,
    updated_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
```

An `AFTER UPDATE` trigger on `installed_packages` appends a row to this log.
`atlas update` clears the log before resolving updates. The log is not used for
rollback.

### `package_dependencies`

```sql
CREATE TABLE IF NOT EXISTS package_dependencies (
    package_name TEXT NOT NULL,
    architecture TEXT NOT NULL,
    dependency_name TEXT NOT NULL,
    PRIMARY KEY (package_name, architecture, dependency_name),
    FOREIGN KEY (dependency_name, architecture)
        REFERENCES installed_packages(name, architecture),
    FOREIGN KEY (package_name, architecture)
        REFERENCES installed_packages(name, architecture),
    CHECK (package_name != dependency_name)
);
```

These are outgoing dependency edges recorded for each package. They are written
from `.sky` metadata during install/update, while installation-order resolution
reads the repository database. The two sources can disagree; the repository
database wins for resolution.

### `owned_files`

```sql
CREATE TABLE IF NOT EXISTS owned_files (
    path TEXT NOT NULL,
    package_name TEXT NOT NULL,
    architecture TEXT NOT NULL,
    PRIMARY KEY (path, architecture),
    FOREIGN KEY (package_name, architecture)
        REFERENCES installed_packages(name, architecture)
);
```

This is Atlas's ownership ledger. A path can be owned by at most one package per
architecture.

The ledger includes:

- every non-directory manifest path,
- directories the install actually created,
- directories already owned by the same package before an update.

It excludes directories that already existed and were not owned before the
install. This prevents a package from deleting shared directories it did not
create.

## Install

`atlas install <name>` resolves the package and its repository-declared runtime
dependencies for the host architecture, then:

1. Prints the package list and asks for confirmation unless `--yes` is active.
2. Creates the target root and the install database directory if needed.
3. Enables foreign-key enforcement and ensures the schema.
4. For each package in dependency-first order:
   - skips dependencies that are already installed,
   - reinstalls the requested root even if it is already installed,
   - creates a staging directory under `<install_dir>/var/lib/atlas/tmp/`,
   - verifies the signature, metadata identity, archive entries, and payload,
   - extracts the payload into the staging directory,
   - checks ownership and destination conflicts,
   - exchanges files into the target root,
   - records the package, owned files, and dependency edges in one database
     transaction,
   - rolls the filesystem exchange back if recording fails.

Staging must share a filesystem with the target root because Atlas uses atomic
rename/exchange operations.

### Group install

`atlas install --group <group>` resolves every repository package whose group
list contains the exact label, plus their missing runtime dependencies, into one
dependency-first plan. Group members are treated as explicit roots even when
another member depends on them. They are reinstalled if already present;
dependencies that are merely installed are skipped. The combined plan is
confirmed once and then installed package by package.

The install database stores the repository group list for every package in the
plan, including dependencies. A failed later package does not undo packages
already installed by the same group operation.

### Conflict rules

Before mutating destinations, Atlas checks every non-directory manifest path:

- A path owned by another package is a conflict.
- A path owned by the same package is allowed.
- An unowned path that already exists on disk is a conflict.

It also checks destination types:

- a payload directory may reuse an existing directory,
- a payload directory may not replace a non-directory,
- a payload file or symlink may not replace a directory.

Directories may collide safely because Atlas only removes them when they are
empty and owned.

### Reserved state directory

Atlas reserves `/var/lib/atlas` inside the target root. A payload may not contain
that path or anything below it, and a payload file may not be an ancestor of it.
This prevents packages from overwriting Atlas state or its staging area.

### Rollback

The install state machine records each filesystem mutation:

- files swapped with an existing destination,
- newly created files and symlinks,
- newly created directories,
- obsolete files staged during updates.

If the database commit fails, a guard reverses those entries: swapped files are
exchanged back, created entries are removed, and created directories are removed
only when empty. Rollback failures are printed to stderr and otherwise ignored.

The guard is disarmed after a successful database commit.

## Update

`atlas update [name]`:

1. Clears the update log.
2. Resolves candidates by comparing installed and repository versions.
3. Returns before prompting if any bulk-resolution error occurred.
4. Prints the candidate list and asks for confirmation unless `--yes` is active.
5. For each candidate:
   - creates a staging directory under the recorded target root,
   - verifies and stages the new package exactly like an install,
   - exchanges new files into place,
   - moves obsolete owned files and symlinks into the staging directory,
   - updates the installed row, owned-file ledger, and outgoing dependency
     edges in one transaction,
   - restores files if the transaction fails.

Version rules:

- newer repository version → update,
- equal version → no-op, regardless of release,
- older repository version → downgrade error.

Obsolete directories are not removed during update; only files and symlinks are
staged away. Empty owned directories may remain.

Bulk updates stop at the first failure. Updates applied before the failure remain
applied.

### Group update

`atlas update --group <group>` selects the union of repository membership for the
group and installed packages whose recorded `groups` contain it. It resolves the
members and their repository-declared dependencies with the same version rules as
named and bulk updates. A group member missing from the install database, a
missing repository dependency, or a downgrade rejects the plan before prompting.

For each package that is actually updated, the installed `groups` column is
replaced with the repository's current list. Packages already at the repository
version are skipped, so their recorded membership is not refreshed. A dependency
updated because a group member needs it keeps its `install_reason` and receives
its current repository membership.

## Uninstall

`atlas uninstall <name>`:

1. Locates the installed package.
2. Resolves the requested package plus its non-explicit dependencies.
3. Stops traversal at dependencies installed explicitly; they stay installed.
4. Skips any package in the removal set that is required by a package outside
   the set, and skips that package's planned dependencies too.
5. Prints skipped packages with their external required-by package.
6. Asks for confirmation unless `--yes` is active.
7. Removes owned paths in reverse path order:
   - files and symlinks are removed,
   - directories are removed only when empty,
   - missing paths count as success.
8. Deletes owned-file, dependency, and installed-package records in one
   transaction.

Filesystem removal and database deletion are not one transaction. If removal
fails after some paths are deleted, the command returns an error and the
database rows remain. A later uninstall skips the already-missing paths.

### Group uninstall

`atlas uninstall --group <group>` reads membership from `installed_packages`, not
from the repository. Every installed row whose recorded `groups` contain the
label is a removal root, regardless of its recorded `install_reason`. The rest of
the plan follows the same rules as package uninstall:

- non-explicit dependencies join the plan,
- explicit dependencies stop traversal and stay installed,
- packages required from outside the plan, and their planned dependencies, are
  skipped and printed.

This means a package that left the group in the repository can still be removed
by group until it is updated and its recorded membership is refreshed.

## Clean

`atlas clean` removes orphaned packages for the host architecture.

A package is kept when it is:

- installed explicitly, or
- reachable from an explicitly installed package through
  `package_dependencies`.

Everything else is an orphan. Atlas prints the orphan list, asks once for
confirmation (unless `--yes` is active), and uninstalls each orphan without
further prompts. An empty orphan list succeeds without prompting.

## Architecture behavior

- `install` resolves and installs packages for the host architecture. Group
  discovery scans all repository rows for the label, then resolves each selected
  package for the host architecture.
- `update` looks up repository entries for the host architecture, even when an
  installed row records a different architecture. Group update's installed
  membership scan is filtered to the host architecture.
- `uninstall` looks up by name without an architecture filter. Group uninstall
  scans all installed rows for the recorded label.
- `clean` selects orphans for the host architecture.

The code has an `arm` variant used by tests; the documented v1 target is
`x86_64`.

## Database lifecycle

- The database file and schema are created on demand.
- There are no migrations. Schema changes require explicit handling for existing
  install databases. In particular, a database created before group support has
  no `installed_packages.groups` column; `CREATE TABLE IF NOT EXISTS` will not add
  it, so group-aware queries fail against the old file.
- Foreign keys are enabled by the install/update/uninstall write paths.
- `installed_at` is set on insert and preserved across updates.

## Useful inspection queries

```bash
sqlite3 ~/.local/share/atlas/installed.db \
  "SELECT name, version, architecture, install_reason, groups, install_dir FROM installed_packages;"

sqlite3 ~/.local/share/atlas/installed.db \
  "SELECT package_name, dependency_name FROM package_dependencies ORDER BY package_name;"

sqlite3 ~/.local/share/atlas/installed.db \
  "SELECT package_name, path FROM owned_files ORDER BY path;"
```
