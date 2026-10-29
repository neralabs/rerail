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
data: the test data (`test-data.lock`) and the samples (`samples.lock`), with any `publish/` laid over them. It never
runs upstream. It descends from the V1 validation gates nera-rule-validation ran by hand (its ADR 0006), adapted to
rerail's layout.

It first scores every pack's fixtures and samples, then judges every rule of each gated tool-pack (one with a
`packs/<id>/upstream/<tool>/gates.yaml`), shipped (`packs/<id>/rules/`) or candidate (`packs/<id>/candidates/`). A
shipped rule that fails a blocking check fails the job; a candidate is only reported, and one that passes everything
is "ready to promote". A rule's verdict is the first blocking gate it fails, in the order below, but every check runs,
so the report shows every failure. Besides the rules, `--check` fails on a pack's own scoring errors: an unexplained
false positive, miss, missed label or finding on a clean sample, an engine failure, a stale divergence, or a test data
or samples error.

To run it locally, pull the data first (`npm run fixtures:pull`), then `npm run test:conformance`, or one pack with
`npm run conformance:languages -- --check --pack <id>`. To see every false positive and miss of one rule:
`npm run conformance:languages -- --pack <id> --rule <rule id>`.

### What runs on a pull request

The `tools` job of `ci.yml` runs `npm run conformance:languages:check -- --base <base> --out <dir>` on every push and
every same-repository pull request, with the data the locks name and any `publish/` laid over it. What blocks and what
is reported:

| Blocks the job | Reported only |
| --- | --- |
| a shipped rule of a gated tool-pack that fails a blocking check (`loads`, `compact`, `conformance`) or whose tool-pack fails a tool-pack check | every candidate's checks, and which candidates are ready to promote |
| a pack's scoring errors, above | `upstream-status`, and a shipped rule whose `pack.json` `on` disagrees with it |
| a not-gated or pending list that grows against the base | a shipped rule that passes a check with no evidence behind it (`holds-under-rename` held vacuously) |
| | a report on a labelled sample's unlabelled line, and engine failures on samples' context files |

The job writes `summary.md`, `gates.json` and the pull request comment (`comment.md`) to the `conformance-summary`
artifact, and the `conformance-report` job posts the comment ([The pull request comment](#the-pull-request-comment)).
A fork's pull request runs no CI, and Dependabot's has no OIDC token to fetch the data, so the job and its comment do
not run for either ([docs/upstream.md](../docs/upstream.md#ci)).

### Per-rule checks

| Gate | Checks | Data |
| --- | --- | --- |
| `loads` (blocks) | `json` (no repeated keys), `schema`, `deterministic`, `engine-compile`, `identity`, `bundled-load` (the document alone), `current-contract` (`rerail.policy/v1`, one `mode`, no severity, no `llm` clause), `in-scope` (the scope covers every file type in `gates.yaml`'s `upstream.lints`, or in `rerail.in_scope` when the pack narrows it) | the document, the engine, `gates.yaml` |
| `compact` (blocks) | `size`; a document over 15 KiB fails `no-combinatorial-expansion` or `no-written-out-repetition` when it lists cases instead of expressing the logic | the document |
| `conformance` (blocks) | `scored`; `no-unexplained-false-positives`, `no-unexplained-misses`, `catches-something` (a true positive, or examples in `pack.json`), `engine-reads-fixtures` | the fixtures: upstream's cases and Rerail's probes, after `divergences.json` |
| | `holds-under-rename`: no unexplained FP or FN, and no engine failure (a parse-induced `source-incomplete` is only reported), on the rename variants (units with `source.units[key].renamedFrom`, under `fixtures/renamed/`), scored apart from the originals. It reports, without failing, the variants the renaming took every finding of the rule from (`renamedAway`); when the renaming took every finding and none survives, the check holds vacuously, which is reported as a pass without evidence. "n/a" when no variant covers the rule, and for a tool-pack without variants, whose detail names the held-out evidence left: its judged samples, or none | the rename variants in the test data |
| | `samples-catch-labelled`, `samples-silent-on-clean`, `engine-reads-samples` ([How samples are judged](#how-samples-are-judged)) | the samples labelled for the rule |
| | `stays-silent`: at least one own upstream unit where upstream and the rule both report nothing, or one clean sample in the rule's reach that it is silent on | `source.units` origins and clean samples |
| | `divergences-current`: every declared divergence explains a disagreement ([Divergences](#divergences)) | `divergences.json` |
| `upstream-status` (informational) | `upstream-rule` (`gates.yaml`'s `rules` lists it and the test data holds a unit from its tests or a finding of it), `stable` (not deprecated), `recommended` (an approved configuration turns it on) | `gates.yaml`'s declared facts |

`upstream-status` never stops a rule: it decides only whether a shipped rule starts on (every check holds) or ships
off as experimental, and the report flags a `pack.json` `on` that disagrees.

### How samples are judged

The runner judges each rule by intent, on the samples labelled for it
([docs/upstream.md, "Samples"](../docs/upstream.md#samples)):

- a report on a labelled line is a true positive, and a labelled line with no report is a miss;
- any report on a sample clean for the rule is a false positive;
- an extra report on a sample with labels for the rule is listed for review and fails nothing;
- a divergence with `"source": "samples"`, matched by the sample's path and line, explains a miss or a false positive.

`samples-catch-labelled` fails a shipped rule on an unexplained miss, `samples-silent-on-clean` on an unexplained false
positive, and `engine-reads-samples` on an engine failure on a judged sample (a rule runtime error, a crash, a timeout,
a work budget, an unreadable file) or one naming no file. A sample's context files (the files beside it that give it
manifest facts, configuration and imports) are never judged: an engine failure there is listed in the report
(`contextFailures`, the CLI's `ctx` column) and fails nothing. A parse failure on a judged sample (a syntax error,
`source-partial`, or a `source-incomplete` the engine reports because this or another file was read with errors, which
the runner tells from a read failure by its message: `isParseIncomplete` in `conformance/languages/runner.ts`) is
report-only: `engine-reads-samples` lists it ("N engine failure(s); M file(s) read in part") without failing, and the
sample stays judged, so a label the partial parse makes the rule miss still fails as a miss.

`stays-silent` needs relevant evidence that the rule is silent where it should be: one of the rule's own upstream
units (an original unit whose `source.units` origin names the rule's upstream rule) where upstream reports nothing and
the rule agrees, or one clean sample the rule is silent on. A clean sample counts only where the rule applies and read
it whole: its scope and a clause's `file` condition admit the file, and the rule reports nothing there, reads it in
full, and has no engine failure on it or naming no file, since silence where the rule never looks proves nothing.
Other rules' units, units of unknown origin and Rerail's probes do not count. Its detail reads "N of M own upstream
unit(s), K clean sample(s)". A tool-pack whose test data names no unit origins rests on its clean samples alone; when
a rule has neither, add a clean real-code sample for it.

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

### Divergences

A pack's `divergences.json` declares each place a rule and upstream (or a sample's labels) disagree on purpose, as
`{ rule, source?, path, line, kind, evidence }` (`divergenceSchema` in `conformance/languages/pack.ts`). `source` is
`fixtures` or `samples` (a sample's evaluation path, `<sample-id>/<source.path>`); omitted, it applies to every source.
`evidence` says what was checked (runtime output, upstream source, issue links). A divergence explains every
unmatched report or miss on the line it names, by kind:

- `upstream-wrong`: upstream's answer is wrong there;
- `not-comparable`: the two cannot be compared on that line;
- `rerail-unknown`: Rerail declines to decide. It explains only a miss, never a report Rerail makes.

An explained disagreement fails nothing. A divergence that explains nothing is stale and fails `divergences-current`
(and pack check 4), so a fix that makes the rule agree removes its divergence in the same pull request. Declare a
divergence only for a disagreement you have checked; otherwise fix the rule.

### The pull request comment

The `conformance-report` job keeps one sticky comment on a same-repository pull request, starting with
`<!-- rerail-conformance -->`, edited in place on every run (`conformance/languages/comment.ts`). It leads with the
verdict (every shipped rule passes, or the number of blocking failures, and how many candidates are ready to promote)
and the commit it judged, links the job summary, and has these sections:

- a table per tool-pack: shipped rules passing, candidates ready to promote;
- **Shipped rules**: each blocking failure with the gate it stopped at, the check and why. Fix the rule, or move it to
  `candidates/`;
- **Ready to promote**: each candidate that passes every check, with its upstream rule and whether `upstream-status`
  starts it on or off;
- **On or off**: shipped rules whose `pack.json` `on` disagrees with `upstream-status`;
- **Passing without evidence**: shipped rule checks that hold vacuously, reported;
- **Candidates**: per tool-pack, how many candidates stop first at each gate, then each candidate's first failure,
  grouped by gate.

GitHub caps a comment's length, so the comment drops detail, longest lists first, until it fits; the job summary and
the `conformance-summary` artifact always have everything.

### Candidates and promotion

A port starts as a candidate: its document at `candidates/rules/<id>.json` (named for its final id), its port entry in
`conformance.json` with `policy` naming that document, and no entry in `pack.json`. Candidates are scored and judged by
every check, reported in the comment, never block and never ship. To get one to "ready to promote", work through the
first gate it stops at (the comment's **Candidates** section): fix the rule, add the evidence a check asks for (a clean
real-code sample for `stays-silent`, say), or declare a checked divergence. Passing every check is necessary, not
sufficient: a reviewer still decides.

To promote a candidate, in one pull request:

1. move its document to `rules/<id>.json` and point its `conformance.json` port's `policy` there (pack check 1 and 2);
2. list it in `pack.json` with its `document`, `title` and `group`, `"on": false` when the comment says it starts off,
   and `"mode": "log"` when upstream's recommended configuration only warns
   ([pack-format.md](../docs/pack-format.md#packjson)); an `examples` pair on its `conformance.json` port moves to that
   entry (pack check 3 runs it in either place; `conformance.json` refuses it on a shipped port);
3. move its rule details from `candidates/README.md` to the pack's `README.md`, which publishes only the shipped rules
   (pack check 5);
4. add a changeset for the pack, since `rules/`, `pack.json` and `README.md` are what it publishes
   ([versioning](../docs/pack-format.md#versioning-and-release));
5. give the evidence in the pull request: its conformance results, rename variants and samples included, and any
   divergence declared ([REVIEW.md](../REVIEW.md)).

Once shipped, the rule blocks CI on any failing check. A shipped rule that starts failing is fixed, or moved back to
`candidates/` (its `pack.json` entry removed, its `policy` pointed back, its `examples` moved to its port and its rule
details moved to `candidates/README.md`).

### Packs not gated

A pack whose shipped rules have no upstream adapter is listed in `conformance/languages/ungated-rules.json`
(complexity's three rules); a tool-pack still validated elsewhere would be listed in
`conformance/languages/pending.json`. Both lists may only shrink against the base (`--base`); a new shipped rule in such
a pack fails, so keep it in `candidates/` until its tool-pack is gated.
