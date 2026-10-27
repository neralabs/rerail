# Conformance

Test support that scores rule packs against the tools they port. It is never published (it is outside package.json
`files`), and `src/` never imports it.

- `pack-validation.ts`: the pack validation contract, checks 1 to 8 of [docs/pack-format.md](../docs/pack-format.md).
  `test/engine/catalog/packs.test.ts` runs it for every `packs/<id>/`.
- `languages/`: the common conformance runner and the gate. It scores each ported rule TP/FP/FN through the
  production workspace, against the tool-pack's expected findings (`packs/<id>/upstream/<tool>/fixtures.expected.json`,
  pack test data that `npm run fixtures:pull` fetches from the archive the tool-pack's `test-data.lock` names), its
  rename variants, the real-code samples and the pack's declared divergences. The CLI is `npm run conformance:languages`,
  with `-- --check` as CI's gate (`npm run test:conformance`). See [The gate](#the-gate).
- `t1/`: the TypeScript grammar census (tier T1 of the language conformance standard).
- `engine.ts`: what every harness shares about the engine it scores (the capabilities, and which engine failure codes
  are not failures).

The generators that write the expected findings are outside this directory, on the authoring path:
`packs/<id>/upstream/<tool>/` and `scripts/pack-data/` (the ESLint helper is `scripts/pack-data/eslint-expected.mjs`,
the renamer `scripts/pack-data/rename/`). How the data is snapshotted and published is
[docs/upstream.md](../docs/upstream.md).

Adding a pack or a port: [docs/language-packs.md](../docs/language-packs.md#start-a-pack).

## The gate

`npm run test:conformance` (`conformance/languages/cli.ts --check`) is the one gate a ported rule passes to ship. It
sits on top of the [pack validation contract](../docs/pack-format.md#the-validation-contract) and reads only pinned
data: the test data (`test-data.lock`) and the samples (`samples.lock`). It never runs upstream. It descends from the
V1 validation gates nera-rule-validation ran by hand (its ADR 0006), adapted to rerail's layout.

It first scores every pack's fixtures and samples, then judges every rule of each gated tool-pack (one with a
`packs/<id>/upstream/<tool>/gates.yaml`), shipped (`packs/<id>/rules/`) or candidate (`packs/<id>/candidates/`). A
shipped rule that fails a blocking check fails the job; a candidate is only reported, and one that passes everything
is "ready to promote". A rule's verdict is the first blocking gate it fails, in the order below, but every check runs,
so the report shows every failure. CI writes `summary.md`, `gates.json` and the pull request comment
(`comment.md`) with `--out` (the `conformance-summary` artifact).

### Per-rule checks

| Gate | Checks | Data |
| --- | --- | --- |
| `loads` (blocks) | `json` (no repeated keys), `schema`, `deterministic`, `engine-compile`, `identity`, `bundled-load` (the document alone), `current-contract` (`rerail.policy/v1`, one `mode`, no severity, no `llm` clause), `in-scope` (the scope covers every file type in `gates.yaml`'s `upstream.lints`, or in `rerail.in_scope` when the pack narrows it) | the document, the engine, `gates.yaml` |
| `compact` (blocks) | `size`; a document over 15 KiB fails `no-combinatorial-expansion` or `no-written-out-repetition` when it lists cases instead of expressing the logic | the document |
| `conformance` (blocks) | `scored`; `no-unexplained-false-positives`, `no-unexplained-misses`, `catches-something` (a true positive, or examples in `pack.json`), `engine-reads-fixtures` | the fixtures: upstream's cases and Rerail's probes, after `divergences.json` |
| | `holds-under-rename`: no unexplained FP or FN on the rename variants (units with `source.units[key].renamedFrom`, under `fixtures/renamed/`), scored apart from the originals; "n/a" for a tool-pack without variants, whose overfit evidence is its samples | the rename variants in the test data |
| | `samples-catch-labelled`, `samples-silent-on-clean`, `engine-reads-samples` | the samples labelled for the rule |
| | `stays-silent`: at least one own upstream unit where upstream and the rule both report nothing, or one clean sample the rule is silent on | `source.units` origins and clean samples |
| | `divergences-current`: every declared divergence explains a disagreement | `divergences.json` |
| `upstream-status` (informational) | `upstream-rule` (`gates.yaml`'s `rules` lists it and the test data holds a unit from its tests or a finding of it), `stable` (not deprecated), `recommended` (an approved configuration turns it on) | `gates.yaml`'s declared facts |

`upstream-status` never stops a rule: it decides only whether a shipped rule starts on (every check holds) or ships
off as experimental, and the report flags a `pack.json` `on` that disagrees.

The samples' judgement (by line and by intent, context files never judged, parse failures report-only) and the
`stays-silent` evidence are described in [docs/upstream.md, "Samples"](../docs/upstream.md#samples).

### Tool-pack checks

Before any rule, each gated tool-pack must pass `bundled-load` (the pack's shipped rules load together through
rerail's bundled-rule loader), `declared-pins` (`conformance.json`'s `tools[].version` equals `gates.yaml`'s
`upstream.version`) and `conformance-runs` (the pack's conformance check ran without errors). When one fails, every
rule of the tool-pack fails with it.

### Declared upstream facts (`gates.yaml`)

`gates.yaml` is the tool-pack's upstream manifest (schema in `conformance/languages/manifest.ts`):

- `upstream`: the pins (`version`, `commit`, `runtime`), `recommended_configs` (in order: the first that turns a rule on
  is its `recommended`), and `lints`, the file types upstream lints at the pins.
- `rerail.in_scope` (optional): the file types the pack's ports cover when it deliberately covers fewer than upstream
  lints; each must be one of `upstream.lints`.
- `rules`, keyed by upstream rule name: `{ deprecated: <upstream's message> | null, recommended: { config, options } |
  null }`. A rule not listed is not a rule of the pinned upstream. The facts are declared by hand and reviewed at each
  bump, whose pull request shows them.

### Own upstream units and rename variants

Each generator writes, in `fixtures.expected.json`, `source.units`: per unit (by its path, or the first path of a
multi-file unit), `rules`, the upstream rules whose tests it came from. A rule's own units are the original units whose
origin names its upstream rule; they count for `stays-silent` and `upstream-rule`. A unit without an entry (Rerail's
probes, or one whose origin is unknown) is scored but is no rule's own evidence. A rename variant's entry also carries
`renamedFrom`, the key of the original unit, which must itself be an original with an origin.

### Packs not gated

A pack whose shipped rules have no upstream adapter is listed in `conformance/languages/ungated-rules.json`
(complexity's three rules); a tool-pack still validated elsewhere would be listed in
`conformance/languages/pending.json`. Both lists may only shrink against the base (`--base`); a new shipped rule in such
a pack fails, so keep it in `candidates/` until its tool-pack is gated.
