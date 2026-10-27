# Changesets

Packs under `packs/` are versioned with [changesets](https://github.com/changesets/changesets). A pull request that
changes what a pack publishes (its `package.json`, `pack.json`, `rules/`, `licenses/`, `LICENSE`, `NOTICE` or
`README.md`) adds one changeset, and CI fails it otherwise: run `npx changeset`, pick the pack, and choose `patch`
while the pack is below 1.0.0, else the bump from the table in
[docs/pack-format.md](../docs/pack-format.md#versioning-and-release) (`npm test` refuses anything else). Do not edit a
pack's `version` or `CHANGELOG.md` by hand: the **Release** pull request (`npm run version:packs`) writes both, and
merging it publishes the packs
([release](../docs/pack-format.md#release)).

The core package at the repository root takes no changeset: the same Release pull request bumps it when what it ships
changes, and its notes go under `## Unreleased` in the root `CHANGELOG.md` ([release notes](../docs/release.md#release)).
