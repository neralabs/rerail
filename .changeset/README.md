# Changesets

Published packs under `packs/` are versioned with [changesets](https://github.com/changesets/changesets). A pull request that
changes what a pack publishes (its `package.json`, `pack.json`, `rules/`, `licenses/`, `LICENSE`, `NOTICE` or
`README.md`) adds one changeset, and CI fails it otherwise: run `npx changeset`, pick the pack, and choose `patch`
while the pack is below 1.0.0, else the bump from the table in
[docs/pack-format.md](../docs/pack-format.md#versioning-and-release) (`npm test` refuses anything else). The summary is
the pack's release note and goes out as it is on the pack's GitHub Release: say what changed for the person upgrading,
in plain language, in a sentence or two (at most 60 words), and leave out commit hashes, ticket ids, pull request
numbers and repository-internal maintenance; CI fails a pull request whose changeset says otherwise.

The Release pull request opens each pack's notes with what changed in its rules since its last release, read from
`pack.json`: new rules and how each starts, rules no longer shipped, and rules that start differently. A summary does
not repeat that, or count rules: changesets pile up between releases, and counts written against one `main` contradict
the next. It says what a rule now does differently, or why a rule no longer ships. A change with nothing else to say,
a rule shipped or a README reworded, leaves the summary empty: the changeset still bumps the pack.

Do not edit a pack's `version` or `CHANGELOG.md` by hand: the **Release** pull request (`npm run version:packs`, then
`scripts/release/pack-changelog.mjs`) writes both, and merging it publishes the packs
([release](../docs/pack-format.md#release)). A private pack (`"private": true` in its `package.json`) is never
released: it takes no changeset, and `npm test` and CI refuse a changeset naming it.

The core package at the repository root takes no changeset: the same Release pull request bumps it when what it ships
changes, and its notes are files in `.changelog/` ([.changelog/README.md](../.changelog/README.md)).
