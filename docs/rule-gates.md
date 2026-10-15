# Rule gates

The rule gates decide whether a ported rule is good enough to ship. Every rule a pack ships (`packs/<id>/rules/`)
must pass them, and CI fails when one does not. Every candidate (`packs/<id>/candidates/`) runs through the same gates,
but its results are only reported, so CI shows how far each candidate got and why it stopped.

The gates sit on top of the [pack validation contract](pack-format.md#the-validation-contract) (checks 1 to 6) and the
conformance check (`npm run test:conformance`), which stay as they are. They are the V1 validation gates that
nera-rule-validation ran by hand (its ADR 0006), brought into rerail so that nobody needs a second repository to
validate a rule. The gates' code is NRV's, ported as it was; what changed is where it runs and what it reads.

## The gates

A rule's verdict is the first blocking gate it fails, in this order. Every gate still runs on every rule, so the
report shows every failure, not only the first.

| Gate | What it checks | Blocks |
| --- | --- | --- |
| `loads` | The document is written for the current rule contract (`rerail.policy/v1`, one `mode`, no severity), passes the duplicate-key check, the policy schema and the engine compile (as written and as stable JSON), loads through the bundled-rule loader, has no `llm` clause, and its scope covers every file type upstream lints. | yes |
| `not-overfit` | No word the rule hardcodes (patterns, fixes, regexes, names, static values) appears in upstream's tests unless upstream's own code explains it (the rule's implementation, its imports and helpers, the names upstream's regexes match) or it is a language keyword. Catches ports that pass upstream's tests by naming their fixtures. | yes |
| `compact` | A document over 15 KiB is inspected: it fails when a third or more of its clauses is the same structure written out over and over, or one node kind nests three or more times along one path. Catches rules that list cases instead of expressing the logic. | yes |
| `matches-upstream-tests` | Upstream, configured as upstream recommends, is the expected answer on its own tests for the rule. The rule must report the same findings at the same start and end positions, in both directions (`identical-results`); reproduce at least one upstream finding (`fires`); agree on at least one case where upstream reports nothing (`stays-silent`); and agree with upstream on every golden (`identical-on-goldens`, `engine-reads-goldens`). | yes |
| `passes-conformance` | The pack's own conformance check (`test:conformance`), which already blocks every shipped rule: on the pack's fixtures (upstream's cases and Rerail's probes, after `divergences.json`) the rule has no unexplained false positive or miss, catches something (or `pack.json` gives it examples), runs without engine failures and has no stale divergence; when the pack pins corpora, their recordings cover it. Candidates are held to the same bar, so the gates are a superset of the check and "ready to promote" means CI accepts the rule once it ships. It comes after V1's gates, so V1's verdicts read the same. | yes |
| `matches-real-code` | The rule and upstream run on real code: the SWE Atlas and SWE-Evo tasks' reference solutions and the corpora the pack pins in `conformance.json`. It never stops a rule by itself; every file where they differ becomes a golden that `matches-upstream-tests` judges in the same run. | through goldens |
| `upstream-status` | The rule exists upstream with an implementation and tests, is not deprecated, and one of upstream's recommended configurations turns it on. It decides only whether a shipped rule starts on; the report flags a pack.json `on` that disagrees. | no |

Tool-pack checks come before any gate. When one fails, every rule of the tool-pack fails with it: a shipped rule blocks
and a candidate is not ready to promote. They fail when the recording is stale (`corpus-pins` and the recording checks),
when a corpus file the recording answers for was not fetched with its recorded bytes (`corpus-complete`, usually a
failed download: run again), or when the pack's shipped rules do not load together through rerail's bundled-rule
loader (`bundled-load`, for example two documents with one id).

Goldens: a golden is a real-code file where the rule and upstream once differed (`packs/<id>/upstream/<tool>/goldens.tsv`:
the rule, the task or corpus, the path and the file's sha256). Goldens are never removed; they stay as regression
tests. When the engine could not fully read a golden (a parse failure, or its work budget ran out), the difference is
an engine gap, not the rule's: the rule still fails, through `engine-reads-goldens`, and the report lists it under
"Blocked by engine gaps" so the fix goes to the engine (NER-1764), not the rule.

Each check's detail says why it failed, in a sentence: the first differing upstream test file and positions, the words
`not-overfit` found, the size metrics.

From V1's numbers: Gate 1 is `upstream-status`, 2 `loads`, 3 `not-overfit`, 4 `compact`, 5 `matches-upstream-tests`,
6 `matches-real-code`.

## Which packs are gated

A **tool-pack** is one upstream tool in one pack, such as `python/ruff`. It is gated when it has
`packs/<id>/upstream/<tool>/gates.yaml`.

- **Gated:** `python/ruff` (in 2 shards, see "Sharding" below), `python/bandit`, `javascript/eslint` (in 4 shards),
  `javascript/eslint-plugin-unicorn`, `react/eslint-plugin-react`, `react/eslint-plugin-react-hooks`,
  `typescript/typescript-eslint` (in 8 shards), `typescript/eslint-plugin-vitest`, `next/eslint-plugin-next`.
- **Pending:** none. A tool-pack listed in `conformance/gates/pending.json` is still validated in
  nera-rule-validation until its recording lands, and the gates report it as pending without blocking.
- **Not gated:** packs with no upstream adapter (node and complexity ship rules; php, blade, sql and vue ship only
  candidates, which run nowhere yet). Their current shipped rules are frozen in
  `conformance/gates/ungated-rules.json`. A new shipped rule in such a pack fails: keep it in `candidates/` until the
  pack has an adapter. This is decided per tool, not per pack: in a gated pack, a shipped rule whose upstream tool has no
  `gates.yaml` and is not pending is frozen the same way. Both lists may only shrink: CI compares them with the pull
  request's base.

Each rule's upstream rule comes from the pack's `conformance.json`. A shipped rule missing from it fails.

## Run them

```sh
npm run gates -- --pack python --tool bandit   # one tool-pack; reports in .rerail/reports/gates/
npm run gates -- --pack python                 # every gated tool-pack of a pack
npm run gates -- summary .rerail/reports/gates # the merged summary CI writes
```

The engine is this checkout's (`npm run build:native` first). The first run fetches the real-code corpus into
`.rerail/gates/` (pinned, about 400 MB); later runs reuse it and the results of every rule whose inputs did not change.
`--no-cache` judges every rule again. The command exits 1 when a shipped rule fails, as CI does. In CI, pull request
runs read the cached results that pushes to main and the nightly saved, and save none of their own.

## Read the report

On a pull request, CI posts the results as a comment: each tool-pack's shipped and candidate counts, every shipped
rule that blocks and why, the candidates **ready to promote** (they pass every gate: the signal to move them into
`rules/`, with the `on` that upstream-status sets), and the other candidates grouped by the gate they stop at, collapsed. A new run collapses
the earlier comments as outdated, so the pull request keeps its history. The comment stays under GitHub's size limit by
shortening long lists; the job summary and the uploaded reports have everything. The job summary reads top to bottom:

1. The headline: shipped rules passing, blocking failures, candidates passing every gate.
2. **Blocking failures:** each shipped rule that fails, the gate it stopped at, the check and why.
3. **Blocked by engine gaps:** shipped rules that fail only because the engine could not read a golden.
4. **Not gated and pending** packs, and any new or stale entry.
5. **Candidates:** where candidates stop, gate by gate, and why each one stopped. Candidates that pass every gate are
   listed as promotable: passing the gates is necessary to ship, not sufficient; reviewers still decide.
6. **On or off:** shipped rules whose `on` in pack.json disagrees with `upstream-status`.
7. Per tool-pack details: each gate on its own, run integrity notes.

A rule that fails: fix the rule (the check's detail names the first difference), or move it to `candidates/` (out of
`pack.json`, its `conformance.json` path updated).

## What the gates read

Upstream linters never run in pull request CI. The gates read what a recorder wrote into the pack, and fetch only the
pinned real-code corpus and upstream's pinned test files:

```text
packs/<id>/upstream/<tool>/
├── gates.yaml          upstream pins, recommended configurations, known parser gaps, and the team's decisions for the tool
├── recording.lock      the manifest: which recording this commit's rules are judged against (recording-store.ts)
├── goldens.tsv         the rule's goldens
└── fixtures/           upstream's test files, copied (listed in licenses/<project>/UPSTREAM.md)
```

### The recordings are not in the tree

A recording — `gates.json`, upstream's facts and expected findings per rule, and `gates-corpus.json`, its findings on
the real-code corpus — is generated, large, and rewritten whole every time a tool-pack is recorded. The sixteen of
them were 71 MB, and git keeps every version of each forever, so a pack re-recorded after an upstream bump cost
another few megabytes of history nobody can read. They live in a GitHub release instead, and each tool-pack commits a
`recording.lock` of a few hundred bytes naming exactly the one its rules are judged against: its sha256, its size, its
format, the pins it was made at and the release that holds it.

Release assets rather than Git LFS, because every public clone and every fork would draw on the repository's LFS
bandwidth quota and going over it blocks LFS for the whole repository; rather than Actions artifacts, because those
expire after 90 days and a commit must stay judgeable. An asset is named `<pack>-<tool>-<stem>-<sha256>.json.gz`, so
it is write-once: the same recording is the same asset, and nothing a commit already names is ever replaced.

```sh
npm run gates -- fetch --pack python --tool ruff   # download what this commit's lock names, into .rerail/gates/recordings/
```

Each gate leg fetches its own tool-pack and nothing else, and a job that runs no gate fetches nothing. A recording is
cached by its own digest, so a re-run on the same commit downloads nothing, and a download whose sha256 is not the
one the lock names fails the leg — which is how a pull request that points its lock at a recording of its own fails.

### Recording

`gates:record` writes the recording into the local cache, the `recording.lock` beside it, and the gzipped assets to
upload under `.rerail/gates/recordings/upload/`. Only the lock is committed. Record again after adding a port,
changing a tool's pins or adding a corpus; it needs `uv`, the network, and the pinned upstream runtimes
(`conformance/recorders/gates/runtimes/`):

```sh
npm run gates:record -- --pack python --tool ruff
```

Locally that is all: the lock names what was just recorded and the gates read it out of the cache, so iterating on a
port needs no release and no network. In CI only `rule-gates-record.yml` records, and it is the only job that
uploads. Pull request CI never runs an upstream linter, so it cannot tell a recording that is what upstream says from
one written to make a rule pass; in a public repository that would be a way past the gates. A maintainer starts a
recording with the `record-gates` label or by hand, and the job runs the recorder and the runtimes **from the base
branch**, reading only the pull request's pins and rule list. For a branch in this repository it pushes the manifest
commit to that branch; for a fork, whose branch the default token cannot write to, it pushes
`gate-recordings/pr-<n>` here and comments to say so.

`rule-gates-nightly.yml` calls the same job with `check: true`, so it reports when a committed recording no longer
matches the pinned upstream and uploads nothing, then judges every rule again (`--refresh-cache`) and saves the fresh
results.

Because the recordings are no longer in the diff, `gates:record` writes a per-rule summary of what moved instead —
expected findings gained and lost on upstream's own tests, corpus files answered differently, facts changed
(`--changes <file>`, `conformance/gates/recording-diff.ts`) — and the recording job posts it on the pull request.

Each runtime is pinned by its `package.json` or `pyproject.toml` alone: every direct dependency at an exact version
and in `overrides` (the recorder refuses a range), and no lockfile is committed (they are git-ignored). Only those direct
pins are in GitHub's dependency graph: Dependabot's version updates cover the root package alone, and a security update
against a runtime's pin changes it and fails `runtime-pins` until the tool-pack is recorded again. Transitive packages
resolve when a runtime is installed. When one of them changes what upstream
answers, the nightly `--check` fails and names the difference: pin that package too (an `overrides` entry in
`package.json`, or a direct `==` dependency in `pyproject.toml`) and record again.

`not-overfit` runs a rule's regexes over every upstream test text, as V1 did. Upstream's test files are not copied into
the pack: the recording lists each one with its sha256, and the gates fetch them from upstream at the pinned commit
(sparse, cached like the corpus), so Ruff's GPL-derived fixtures are read in place and never committed. Test text that
is not a file of its own (Ruff's test-module strings, the `code` of each ESLint RuleTester case) is kept verbatim in
`gates.json` (`fixtureTexts.inline`). That makes the recording partly upstream's material: the recorder lists it in
the upstream's `UPSTREAM.md` as excerpts, and `REUSE.toml` annotates it under upstream's licence and Apache-2.0.

Independent parsers (two TypeScript versions and Babel for the ESLint family; CPython and libcst for Python) read every
case source and every corpus file, so a file the engine cannot parse can be told from a file that is not valid source.
Nine of ten corpus files are source every parser accepts, and writing the same silent verdict list on each of them was
most of both recordings, so a recording names its parsers once in `parsers` and leaves `verdicts` off an entry no
parser objected to. The entry itself stays, with its path and its sha256: an entry with no `verdicts` means every
parser was silent, and no entry at all means the source was never recorded — a golden whose text changed, say — on
which the gates have no verdict and draw no conclusion.

## CI

`ci.yml` calls `rule-gates.yml` after its build job when a pull request touches `packs/`, `conformance/`, the engine
or the workflows, and on every push to main: one job per gated tool-pack, then a report job that writes the summary
and fails when anything blocking failed.

### Sharding

A tool-pack too slow for one 60-minute job runs in several, each judging a share of its rules. The matrix gives such a
tool-pack's legs `shard` (1 to `shards`) and `shards`; each runs

```sh
npm run gates -- --pack typescript --tool typescript-eslint --shard 2/6   # rules whose sha256(id) mod 6 is 1
```

A rule's shard depends only on its document id and the shard count, so adding or removing rules moves no other rule.
Every shard still checks the whole tool-pack (pins, recordings, the bundled-rule load), writes its report under
`shard-<i>of<n>/` and caches its results under `.rerail/gates/results/<pack>/<tool>/shard-<i>of<n>/`. The report job
then merges each tool-pack's shards into one report before the summary:

```sh
npm run gates -- merge .rerail/reports/gates           # shard-*of*/<pack>--<tool>.json -> <pack>--<tool>.json
npm run gates -- merge .rerail/reports/gates --write   # and the shards' golden promotions into goldens.tsv
```

A tool-pack whose shards are missing, of another count, or disagree on what they share gets no merged report, so the
summary fails it. `--write` does not take `--shard` (shards run side by side); merge the shards with `--write` instead.
