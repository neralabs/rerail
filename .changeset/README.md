# Changesets

Packs under `packs/` are versioned with [changesets](https://github.com/changesets/changesets). A pull request that
changes a pack adds one changeset: run `npx changeset`, pick the pack, and choose `patch` while the pack is below
1.0.0, else the bump from the table in [docs/pack-format.md](../docs/pack-format.md#versioning-and-release) (`npm test`
refuses anything else). Do not edit a pack's `version` or `CHANGELOG.md`
by hand: the **Version packs** pull request (`npm run version:packs`) writes both, and merging it publishes the packs
([release](../docs/pack-format.md#release)).

The core package at the repository root keeps its own release: a `v<version>` tag and the root `CHANGELOG.md`.
