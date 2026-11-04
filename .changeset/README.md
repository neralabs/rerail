# Changesets

Packs under `packs/` are versioned with [changesets](https://github.com/changesets/changesets). A pull request that
changes what a pack publishes (its `package.json`, `pack.json`, `rules/`, `licenses/`, `LICENSE`, `NOTICE` or
`README.md`) adds one changeset, and CI fails it otherwise: run `npx changeset`, pick the pack, and choose `patch`
while the pack is below 1.0.0, else the bump from the table in
[docs/pack-format.md](../docs/pack-format.md#versioning-and-release) (`npm test` refuses anything else). The summary is
the pack's release note and goes out as it is on the pack's GitHub Release: say what changed for the person upgrading,
in plain language, in a sentence or two (at most 60 words), and leave out commit hashes, ticket ids, pull request
numbers and repository-internal maintenance; CI fails a pull request whose changeset says otherwise.

`npx changeset` gives the file a random name: rename it so it starts with its change type and says what it changes
(`fixed-vue-no-unused-vars-filters.md`). The types are core's, from
[.changelog/README.md](../.changelog/README.md#changelog-entries): `added-`, `changed-`, `deprecated-`, `removed-`,
`fixed-` and `security-`. An entry that does more than one takes the type that matters most to the person upgrading.
CI fails a pull request while any pending changeset has no type, an empty one included.

The Release pull request opens each pack's notes with what changed in its rules since its last release, read from
`pack.json`: new rules (under Added) and how each starts, rules no longer shipped (Removed), and rules that start
differently (Changed). A summary does not repeat that, or count rules: changesets pile up between releases, and counts written against one `main` contradict
the next. It says what a rule now does differently, or why a rule no longer ships. A change with nothing else to say,
a rule shipped or a README reworded, leaves the summary empty: the changeset still bumps the pack.

Do not edit a pack's `version` or `CHANGELOG.md` by hand: the **Release** pull request (`npm run version:packs` for the
version, `scripts/release/pack-changelog.mjs` for the changelog, in the format of core's) writes both, and merging it
publishes the packs
([release](../docs/pack-format.md#release)).

The core package at the repository root takes no changeset: the same Release pull request bumps it when what it ships
changes, and its notes are files in `.changelog/` ([.changelog/README.md](../.changelog/README.md)).
