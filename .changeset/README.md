# Changesets

Packs under `packs/` are versioned with [changesets](https://github.com/changesets/changesets). A pull request that
changes a pack adds one changeset: run `npx changeset`, pick the pack, and choose the bump from the table in
[docs/pack-format.md](../docs/pack-format.md#versioning-and-release). Do not edit a pack's `version` or `CHANGELOG.md`
by hand; `npx changeset version` writes both when packs are released.

The core package at the repository root keeps its own release: a `v<version>` tag and the root `CHANGELOG.md`.
