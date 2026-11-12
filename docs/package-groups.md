# Package groups

Status: draft for version 1.

A package group is a label, not a package. Recipes tag packages with zero or
more group labels, and Atlas can install, update, or uninstall all packages
carrying a label in one operation. Groups have no version, no dependencies, and
no artifact of their own.

```yaml
groups:
  - desktop
```

## Declaring membership

Sisyphus reads the optional `groups` list from `sky.yaml`. Each entry must:

- use the package-name grammar `[a-z0-9][a-z0-9+._-]*`,
- be unique within the list,
- differ from the package's own `name`.

A missing `groups` key defaults to an empty list. A group label does not need to
be declared separately; a group exists while at least one repository row or
install record uses it.

Validation failures are reported before the build starts:

| Condition | Error |
| --- | --- |
| Empty or malformed label | `Group name <label> is invalid refer to the package naming documentation` |
| Duplicate label | `Group list must not contain duplicate entries` |
| Package lists itself | `Package is not allowed to list itself as a group` |

Groups do not affect the build plan. `deps` and `makedeps` remain the only
dependency edges, and group labels are never resolved as packages.

## Where membership is stored

Membership is local database state, not package content:

| Database | Column | Meaning |
| --- | --- | --- |
| Repository (`repo.db`) | `packages.groups` | Current membership published by Sisyphus, scoped to `(name, architecture)`. |
| Install state (`installed.db`) | `installed_packages.groups` | Membership snapshot recorded when the package was installed or last updated. |

Both columns contain a serialized YAML list. Publishing a package replaces its
repository row, so republishing replaces the package's complete group list for
that architecture.

`.sky` metadata does **not** contain `groups`, and the package signature does not
cover membership. Anyone who can write the local repository database can change
which groups a package belongs to without invalidating its signature. See
[security.md](security.md).

## Installing a group

```bash
atlas install --group <group>
atlas install -g <group>
atlas install --group <group> --root <dir>
```

- Exactly one of a package name or `--group` must be supplied.
- Atlas scans repository rows for exact group-label membership, collects every
  member, and resolves each member plus its missing runtime dependencies using
  the repository's `deps` column.
- The combined plan is dependency-first. Each package appears once, even when it
  is a dependency of several members or is both a member and a dependency.
- The entire plan is resolved before the confirmation prompt. A missing member,
  missing dependency, or cycle rejects the plan before anything is installed.
- Group members are recorded as `explicit` installs and are installed even if
  they were already present. Dependencies already installed by another package
  are skipped; a package that is both an installed dependency and a selected
  member is reinstalled as explicit.
- Installation remains sequential per package. If a later package fails to
  verify or install, earlier successfully installed packages remain installed.
- One confirmation prompt lists the combined plan unless `--yes` is active.

Unknown group:

```text
Unknow package group `<group>`
```

## Updating a group

```bash
atlas update --group <group>
atlas update -g <group>
```

A group update selects the union of:

- packages currently in the group in the repository database, and
- installed packages whose recorded install-state membership contains the group.

The selected members and their repository-declared dependencies are resolved
with the normal update rules:

- newer semantic version → update,
- equal version → skip, even when release numbers differ,
- older repository version → downgrade error that rejects the plan,
- a selected member missing from the install database → error,
- a missing repository dependency → error.

All candidates are resolved before the prompt. If resolution reports an error,
nothing is updated. If you decline the prompt, nothing is updated. Updates are
then applied in dependency-first order and stop at the first failure; earlier
updates remain applied.

Membership is refreshed from the repository only when a package is actually
updated. A package that is already at the repository version is skipped, so its
install-state membership keeps the previous snapshot.

Notable selection behavior:

- A member whose newer version leaves the group is updated once, then stops
  matching the group on later updates.
- A dependency-only package whose newer version joins the group is selected by
  the repository scan and updated while keeping its installed reason.
- If every selected member is already current, the group update succeeds without
  changing anything and without an "up to date" message.

Unknown group:

```text
Unknown package group `<group>`
```

## Uninstalling a group

```bash
atlas uninstall --group <group>
atlas uninstall -g <group>
```

Group uninstall reads membership from the **install database**, not from the
repository. This means it operates on the membership snapshot recorded at
install/update time, even if the package has since changed groups in the
repository.

- Exactly one of a package name or `--group` must be supplied.
- Every installed package whose recorded groups contain the label is a removal
  root, regardless of its recorded install reason.
- Each member's non-explicit dependencies join the removal plan. Explicit
  dependencies stop traversal and stay installed.
- Packages in the plan that are required by packages outside the plan, and their
  planned dependencies, are skipped and printed with the external required-by
  package.
- Owned files and symlinks are removed; directories are removed only when empty.
- One confirmation prompt lists the combined plan unless `--yes` is active.

Unknown or empty group:

```text
No package found in group <group>
```

## Architecture behavior

Group membership is stored per `(name, architecture)` in the repository. Atlas
group discovery currently scans repository rows without an architecture filter,
then resolves each selected package's metadata and version for the host
architecture. Consequently, membership recorded only on another architecture's
row can select the host-architecture package of the same name. Installed
membership used by group updates is filtered to the host architecture; group
uninstall looks up installed rows by name without an architecture filter. In
practice these details are only observable when the same group label is used
across architectures.

The documented v1 target is `x86_64`; the `arm` variant exists for tests.

## Limitations

- Group membership is not signed or otherwise authenticated.
- Groups cannot contain other groups and are not dependency edges.
- There is no group object, group package, or repository index of groups.
- Group metadata is not present in `.sky` files.
- Install-state membership is refreshed only when a package is installed or
  updated.
- Group update has no dedicated "nothing to do" message.
- Existing databases created before group support have no `groups` column and
  are not migrated automatically; see [troubleshooting.md](troubleshooting.md).
