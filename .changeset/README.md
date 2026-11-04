# Changesets

Published packs under `packs/` are versioned with [changesets](https://github.com/changesets/changesets). A pull request that
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

A summary under `added-`, `removed-` or `fixed-` is about one rule and opens with its id in backticks
(`` `eslint-no-setter-return` no longer reports `return /* comment */;` in a setter. ``); CI refuses one that opens with
anything else, names a rule the pack does not ship, or under `removed-` names one it still ships. A fix to three rules is
three changesets. A change to no one rule, a README or licence note, is `changed-`.

The Release pull request opens each pack's notes with what changed in its rules since its last release, read from
`pack.json`, one entry per rule: new rules (under Added), with how each starts and its description; rules no longer
shipped (Removed); and rules that start differently (Changed). A summary does not repeat that, or count rules: changesets pile up between releases, and counts written against one `main` contradict
the next. It says what a rule now does differently, or why a rule no longer ships (`` `rule-id`: why it left. ``, which replaces
the rule's generated entry). A change with nothing else to say,
a rule shipped or a README reworded, leaves the summary empty: the changeset still bumps the pack.

Do not edit a pack's `version` or `CHANGELOG.md` by hand: the **Release** pull request (`npm run version:packs` for the
version, `scripts/release/pack-changelog.mjs` for the changelog, in the format of core's) writes both, and merging it
publishes the packs
([release](../docs/pack-format.md#release)). A private pack (`"private": true` in its `package.json`) is never
released: it takes no changeset, and `npm test` and CI refuse a changeset naming it.

The core package at the repository root takes no changeset: the same Release pull request bumps it when what it ships
changes, and its notes are files in `.changelog/` ([.changelog/README.md](../.changelog/README.md)).
