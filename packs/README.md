# Rerail rule packs

Rule packs for Rerail. Each pack is published to npm as its own package, versioned and released separately from
the core package at the repository root.

> Status: pre-release (below 1.0.0). Every pack here is published to npm, restricted to the same people who can read
> core, and core ships none of them: set-up and `rerail packs add` install them into `$RERAIL_HOME/packs`. The packs
> here: `python`, `php`, `blade`, `rust`, `typescript`, `javascript`, `react`, `vue`, `node`, `next`, `sql` and
> `complexity`. Every rule document Rerail ships lives in a pack; core has no rules of its own, and lists the packs
> by name only (`src/engine/catalog/pack-list.generated.ts`).

- [docs/pack-format.md](../docs/pack-format.md): the layout and the validation contract every pack here passes.
- [docs/port-readiness.md](../docs/port-readiness.md): the port readiness gates every shipped ported rule must pass (`npm run port-readiness`);
  candidates run through them and are only reported.
- [docs/language-packs.md](../docs/language-packs.md#start-a-pack): how to start a pack, for a supported language, a
  planned one, or one that ports another tool's rules.

```sh
npx vitest run test/engine/catalog/packs.test.ts                    # every pack: checks 1 to 6 and each rule's status
npx vitest run test/engine/catalog/packs.test.ts -t "pack python"   # one pack
```

A pull request that changes a pack adds a changeset (`npx changeset`); see
[versioning](../docs/pack-format.md#versioning-and-release). Pack versions and changelogs are never edited by hand.

## Pack layout

Each pack lives in `packs/<id>/`:

| Path | Contents | Published |
| --- | --- | --- |
| `package.json` | npm package `@neralabs/rerail-pack-<id>`; core version range as a peer dependency; no dependencies | yes |
| `pack.json` | The pack (`rerail.rule-pack/v1`): match, and each rule's document, title, group, mode and examples | yes |
| `rules/<id>.json` | Code-rule documents (`rerail.policy/v1`), each named for its rule id ([rule ids](../docs/pack-format.md#packjson)) | yes |
| `LICENSE`, `NOTICE`, `README.md` | The pack's licence, notice and description | yes |
| `licenses/<project>/` | Upstream licence, verbatim, and `UPSTREAM.md` listing every copied or adapted file | yes |
| `conformance.json`, `divergences.json`, `upstream/`, `candidates/` | For ported rules: the tools' recorded findings, their copied tests, declared divergences, ports under evaluation (scored and reported, never gated) | no |

A pack is data only: it never imports the core engine. Core runs its rules, and the repository's tests validate
every pack against the contract. A pack may hold rules for a language core lists as `planned`: they are stored
inactive and reported as `not supported yet` until core reads the language.

## Message wording

[`pack-message-wording.draft.json`](pack-message-wording.draft.json) is a rule for working in this repository, not
part of any pack. It steers the coding agent while it writes a rule's violation message:

```sh
rerail rules add packs/pack-message-wording.draft.json   # into your library, on, for this repository
```

It reads every violation message in `packs/*/rules/*.json` and `packs/*/candidates/rules/*.json`, each case of one an
option chooses included (but not the option's name), and steers when a message names an upstream tool (as a pasted
rule id does when it starts with the tool's name), names Rerail, or writes NEVER, MUST or ALWAYS in capitals before a
lower-case word (so a SQL keyword such as `GENERATED ALWAYS AS` passes). A word in backticks or single quotes is code
or a configured value, and passes; double quotes don't count. Each violation is reported once, at its first such
word. The rest of the [message criteria](../docs/pack-format.md#message) are left to review.

What it gets wrong:

- **A message holding its own rule id is caught only when the id names a tool**: 523 of the 595 ids do; the `n-`,
  `next-`, `react-`, `vitest-` and `complexity-` ones don't. A rule reads JSON as text only, and a regex cannot compare
  the document's `id` with its message.
- **An apostrophe that pairs with no quote**, such as a Rust lifetime (`'a`) or `print()'s`, ends what it reads of a
  message, unless a quote later in the message pairs with it: then what that quote opened counts as unquoted.
- **Backticks hide any word**, so backticking a tool name or a rule id silences it without fixing the message.

Test a change with `rerail rules test packs/pack-message-wording.draft.json`, raise its `revision`, and add it again;
`test/cli/pack-message-wording.test.ts` runs its cases.

## Third-party material

Packs derived from upstream rule sets keep the upstream licence under `packs/<id>/licenses/<project>/`. The
repository's `REUSE.toml` maps the copied files to their licence, `reuse lint` must pass, and the licence check
rejects any licence outside the repository's allowlist (`scripts/check/licence-allowlist.json`).
