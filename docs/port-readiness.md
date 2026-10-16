# Port readiness

The port readiness gates decide whether a ported rule is good enough to ship. Every rule a pack ships (`packs/<id>/rules/`)
must pass them, and CI fails when one does not. Every candidate (`packs/<id>/candidates/`) runs through the same gates,
but its results are only reported, so CI shows how far each candidate got and why it stopped.

The gates sit on top of the [pack validation contract](pack-format.md#the-validation-contract) (checks 1 to 6) and the
conformance check (`npm run test:conformance`), which stay as they are. They are the V1 validation gates that
nera-rule-validation ran by hand (its ADR 0006), brought into rerail so that nobody needs a second repository to
validate a rule. The gates' code and the recorder's adapters are adapted from NRV's, not copied: they read rerail's
layout, and the gates answer from recordings instead of running upstream. The NRV commits they come from, and what
changed, are in [`conformance/port-readiness/README.md`](../conformance/port-readiness/README.md#where-the-code-comes-from).

**Names.** The system is *port readiness*, its steps are *gates*, and GitHub shows its workflows and checks as "Port
readiness". It was called the rule gates while this was built. A few names published outside the repository keep the
older word until the first re-record after it merges, because the committed `recording.lock` files and the attestation
check match them exactly: the recording formats (`rerail.gates-recording/v2`, `rerail.gates-corpus/v1`), the
`gate-recordings` release and its asset names, the recording comment marker and the `record-gates` label. The files that
configure the gates themselves (`gates.yaml`, `goldens.tsv`) are named for the gates, as they should be.

## The gates

A rule's verdict is the first blocking gate it fails, in this order. Every gate still runs on every rule, so the
report shows every failure, not only the first.

| Gate | What it checks | Blocks |
| --- | --- | --- |
| `loads` | The document is written for the current rule contract (`rerail.policy/v1`, one `mode`, no severity), passes the duplicate-key check, the policy schema and the engine compile (as written and as stable JSON), loads through the bundled-rule loader, has no `llm` clause, and its scope covers every file type upstream lints. | yes |
| `not-overfit` | No word the rule hardcodes (patterns, fixes, regexes, names, static values) appears in upstream's tests unless upstream's own code explains it (the rule's implementation, its imports and helpers, the names upstream's regexes match) or it is a language keyword. Catches ports that pass upstream's tests by naming their fixtures. | yes |
| `compact` | A document over 15 KiB is inspected: it fails when a third or more of its clauses is the same structure written out over and over, or one node kind nests three or more times along one path. Catches rules that list cases instead of expressing the logic. | yes |
| `matches-upstream-tests` | Upstream, configured as upstream recommends, is the expected answer on its own tests for the rule. The rule must report the same findings at the same start and end positions, in both directions (`identical-results`), leaving out only an end part upstream does not report on every finding of the rule (when any finding of an ESLint rule on its own tests has no end, the rule is compared on starts alone; the report lists every rule compared without an end); reproduce at least one upstream finding (`fires`); agree on at least one case where upstream reports nothing (`stays-silent`); and agree with upstream on every golden (`identical-on-goldens`, `engine-reads-goldens`). | yes |
| `passes-conformance` | The pack's own conformance check (`test:conformance`), which already blocks every shipped rule: on the pack's fixtures (upstream's cases and Rerail's probes, after `divergences.json`) the rule has no unexplained false positive or miss, catches something (or `pack.json` gives it examples), runs without engine failures and has no stale divergence; when the pack pins corpora, their recordings cover it. Candidates are held to the same bar, so the gates are a superset of the check and "ready to promote" means CI accepts the rule once it ships. It comes after V1's gates, so V1's verdicts read the same. | yes |
| `matches-real-code` | The rule and upstream run on real code: the SWE Atlas and SWE-Evo tasks' reference solutions and the corpora the pack pins in `conformance.json`. It never stops a rule by itself; every file where they differ becomes a golden that `matches-upstream-tests` judges in the same run. A rule with no eligible file (none it applies to that upstream reads) is judged on nothing and reported `not-run` with "0 eligible files", never as agreeing: today that is every rule of `javascript/eslint` and `javascript/eslint-plugin-unicorn`, since the javascript pack pins no corpus and no task's solution changes a .js, .mjs or .cjs file, and of `typescript/eslint-plugin-vitest`, whose ports apply only to TypeScript test files while upstream lints only .js, .mjs and .cjs. | through goldens |
| `upstream-status` | The rule exists upstream with an implementation and tests, is not deprecated, and one of upstream's recommended configurations turns it on. It decides only whether a shipped rule starts on; the report flags a pack.json `on` that disagrees. | no |

Tool-pack checks come before any gate. When one fails, every rule of the tool-pack fails with it: a shipped rule blocks
and a candidate is not ready to promote. They fail when the recording is stale (`corpus-pins` and the recording checks),
when a committed case source is not the bytes upstream's answers were recorded on (`case-sources`), when a corpus
file the recording answers for was not fetched with its recorded bytes (`corpus-complete`, usually a failed download:
run again), or when the pack's shipped rules do not load together through rerail's bundled-rule
loader (`bundled-load`, for example two documents with one id).

Goldens: a golden is a real-code file where the rule and upstream once differed (`packs/<id>/upstream/<tool>/goldens.tsv`:
the rule, the task or corpus, the path and the file's sha256). Goldens are never removed; they stay as regression
tests. When the engine could not fully read a golden (a parse failure, or its work budget ran out), the difference is
an engine gap, not the rule's: the rule still fails, through `engine-reads-goldens`, and the report lists it under
"Blocked by engine gaps" so the fix goes to the engine (NER-1764), not the rule.

Known parser gaps (`known_gaps` in `gates.yaml`) excuse an upstream test file that differs where the engine reported a
parse failure on it, and nothing else. A gap holds when its probe occurs in every file it names, the engine still fails
to parse the probe, and the recorded independent parsers read the probe as valid source. A listed gap that does not
hold, or excuses no file, is a blocking failure until it is removed or corrected.

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
- **Pending:** none. A tool-pack listed in `conformance/port-readiness/pending.json` is still validated in
  nera-rule-validation until its recording lands, and the gates report it as pending without blocking.
- **Not gated:** packs with no upstream adapter (complexity ships rules; node, php, blade, sql and vue ship only
  candidates, which run nowhere yet). Their current shipped rules are frozen in
  `conformance/port-readiness/ungated-rules.json`. A new shipped rule in such a pack fails: keep it in `candidates/` until the
  pack has an adapter. This is decided per tool, not per pack: in a gated pack, a shipped rule whose upstream tool has no
  `gates.yaml` and is not pending is frozen the same way. Both lists may only shrink: CI compares them with the pull
  request's base.

Each rule's upstream rule comes from the pack's `conformance.json`. A shipped rule missing from it fails.

## Run them

```sh
npm run port-readiness -- --pack python --tool bandit   # one tool-pack; reports in .rerail/reports/port-readiness/
npm run port-readiness -- --pack python                 # every gated tool-pack of a pack
npm run port-readiness -- summary .rerail/reports/port-readiness # the merged summary CI writes
```

The engine is this checkout's (`npm run build:native` first). The first run fetches the real-code corpus into
`.rerail/port-readiness/` (pinned, about 400 MB); later runs reuse it and the results of every rule whose inputs did not change.
`--no-cache` judges every rule again. The command exits 1 when a shipped rule fails, as CI does.

### What a cached result is keyed by

A rule's result key covers everything its verdict can depend on: the gate code and its dependencies, the tool-pack's
manifest, recordings and corpus pins, the rule's own document, conformance entry, goldens and upstream case sources,
and the engine, though only the part of the engine the rule can be judged by.

That part is the engine's shared code plus the files of the rule's own **language family** (`languages/*.json`'s
`family`: `ecma` for JavaScript and TypeScript, `python` for Python). `conformance/port-readiness/engine.ts` lists each family's
files explicitly: its vendored grammars and registry entries, the scope and value modules named for it, its builtin
tables, and the suppression readers only it honours. Everything else is shared and so part of every rule's key:
`matcher/`, `compile/`, `workspace/`, `language/adapter/`, the lockfile, the host's TypeScript engine, and any engine
file nobody listed. That last one is the direction the list fails in: a missing entry costs one cold re-judge of
everything, never a stale verdict. A rule whose document names no language, or one the language registry does not
know, keys on the whole engine for the same reason.

A Python-only engine change therefore judges the Python legs cold and leaves the other 17 reading main's results. The
cost is the mirror image: a file listed as one family's that another family in fact reads would leave that family's
verdicts stale until the nightly's uncached run. `test/conformance/port-readiness/engine.test.ts` fails when a listed path
matches no engine source, so the list cannot rot silently.

Two things are deliberately out of the key. The native binary: it is not bit-reproducible, so hashing it made a
build-cache miss on unchanged sources re-judge every rule, and the sources it was built from say the same thing
reproducibly. That holds only while the binary is their build, so a binary older than an engine source it is built from
(an edit not yet rebuilt) saves no result, and neither `--write` nor `merge --write` runs on it. A shard report does
not record which binary made it, so shards run on a stale binary and merged after a rebuild are not caught: rerun them.
And the engine sources only `cargo test` compiles: every file a `#[cfg(test)] mod` declaration reaches, which is how the
engine keeps its oracle tests beside the code they check.

In CI every run saves its results, pull requests included. GitHub scopes a pull request's cache to that pull request,
so main never reads one: a push to main reads only what main's own pushes and the nightly saved. A pull request can
hand itself verdicts its own code computed, which it could already do by editing the gate code that decides them.

## Read the report

On a pull request, CI posts the results as a comment: each tool-pack's shipped and candidate counts, every shipped
rule that blocks and why, the candidates **ready to promote** (they pass every gate: the signal to move them into
`rules/`, with the `on` that upstream-status sets), and the other candidates grouped by the gate they stop at, collapsed. A new run collapses
the earlier comments as outdated, so the pull request keeps its history. The comment stays under GitHub's size limit by
shortening long lists; the job summary and the uploaded reports have everything. The job summary reads top to bottom:

1. The headline: shipped rules passing, blocking failures, candidates passing every gate.
2. **Blocking failures:** each shipped rule that fails, the gate it stopped at, the check and why, and each listed
   known parser gap that does not hold or excuses nothing.
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
├── recording.lock      the manifest: which recording this commit's rules are judged against (conformance/port-readiness/recording/store.ts)
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
it is write-once: the same recording is the same asset, and nothing a commit already names is ever replaced. A lock
naming its asset any other way is refused, so it cannot point at another tool-pack's recording.

```sh
npm run port-readiness -- fetch --pack python --tool ruff   # download what this commit's lock names, into .rerail/port-readiness/recordings/
```

Each gate leg fetches its own tool-pack and nothing else, and a job that runs no gate fetches nothing. A recording is
cached by its own digest, so a re-run on the same commit downloads nothing, and a download whose sha256 is not the
one the lock names fails the leg — which is how a pull request that points its lock at a recording of its own fails.

A matching digest only says the asset is the one the lock names. Anyone who can write to the repository, and any
workflow with a write token, can upload to the release, so the fetch also requires the GitHub artifact attestation
that `port-readiness-record.yml` makes when it records from `main` (`gh attestation verify`, so fetching needs the GitHub
CLI, signed in or given `GH_TOKEN`/`GITHUB_TOKEN`, even once the repository is public). The only assets taken without
one are those uploaded by hand when the recordings left the tree, before that workflow could run; `LIFTED_ASSETS` in
`recording/store.ts` lists them, and an entry goes when its tool-pack is next recorded. Those recordings predate the
sha256 each recorded case now carries for its committed source, so `case-sources` holds their case sources to one
digest over all of them instead, the one they had when they were lifted (`LIFTED_CASE_SOURCES`), and an edited or
missing case source fails it the same way.

### Recording

`port-readiness:record` writes the recording into the local cache, the `recording.lock` beside it, the gzipped assets to
upload under `.rerail/port-readiness/recordings/upload/`, and any case source the recording names that the tool-pack's
`fixtures/` lacks, with its row in `UPSTREAM.md`. The lock, the case sources and `UPSTREAM.md` are committed; the
recordings are not. Record again after adding a port,
changing a tool's pins or adding a corpus; it needs `uv`, the network, and the pinned upstream runtimes
(`conformance/recorders/port-readiness/runtimes/`):

```sh
npm run port-readiness:record -- --pack python --tool ruff
```

Locally that is all: the lock names what was just recorded and the gates read it out of the cache, so iterating on a
port needs no release and no network. In CI only `port-readiness-record.yml` records, and it is the only workflow that
uploads. Pull request CI never runs an upstream linter, so it cannot tell a recording that is what upstream says from
one written to make a rule pass; in a public repository that would be a way past the gates. A maintainer starts a
recording with the `record-gates` label, or by hand with a pull request and the head `sha` they reviewed, and the run
records **only that commit**: if the pull request has moved on since, it fails rather than record the newer push. The
recorder and the runtimes come **from the base branch**, and only the pull request's `packs/` and rule list from its
head; a fork may not change the upstream its `gates.yaml` pins (repository, commit, runtime, tests), only
`popularity` and `known_gaps`. Only a run from `main`, for `main` or a pull request into `main`, is attested, and the
gates take no other recording, so `authorize` refuses any other before a leg records: one started by hand from another
branch, and a stacked pull request, which is recorded once it targets `main`.

The run has three jobs, so nothing a pull request chose runs where a write token is. `authorize` checks who started
it and fixes the commits, the tool-packs and the release. One read-only `record` leg per tool-pack runs the recorder
and uploads what it wrote as an artifact. `publish`, the only job with a write token, runs no recorder, node or npm: it
checks every path and digest in those artifacts, attests the recordings it checked, uploads them, and commits every
leg's lock, case sources and `UPSTREAM.md` rows as one commit on the recorded head, pushed once:

- a pull request from a branch here gets the commit on its branch, as a fast-forward;
- a fork's goes to `gate-recordings/pr-<n>` here, since the default token cannot write to a fork, and a comment says
  to merge it;
- a run started by hand from `main` with no pull request records `main`, pushes `gate-recordings/run-<id>`, and opens
  a pull request into `main`: a recording never reaches a protected branch without review. (GitHub must let Actions
  create pull requests, under Settings → Actions → General.)

A push or pull request made with the workflow token starts no workflow, so CI runs on the recording commit once
someone pushes to the branch or closes and reopens the pull request.

GitHub holds at most 1,000 assets in a release. `authorize` uses the newest of `gate-recordings`,
`gate-recordings-2`, `-3` and so on, and starts the next once the newest holds 900; `publish` refuses an upload that
would pass 1,000. A lock names the release that holds its recordings, so an older commit keeps fetching from the older
one.

`port-readiness-nightly.yml` runs the same recorder with `--check` in a read-only job of its own, so it reports when a
committed recording no longer matches the pinned upstream and uploads nothing, then judges every rule again
(`--refresh-cache`) and saves the fresh results.

Because the recordings are no longer in the diff, `port-readiness:record` writes a per-rule summary of what moved instead —
expected findings gained and lost on upstream's own tests, corpus files answered differently, facts changed
(`--changes <file>`, `conformance/port-readiness/recording/diff.ts`) — and the publish job posts it on the pull request.
When the committed lock names a recording that cannot be read (its fetch failed, or the lock or the cached recording
fails its schema), the summary warns that it was not compared rather than calling it a first recording.

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
`gates.json` (`fixtureTexts.inline`). That makes the recording partly upstream's material, so the upstream's
`UPSTREAM.md` records it as excerpts under upstream's licence and Apache-2.0 — in the note under the file table, not
in the table itself, which pack check 6 reads as the files the pack carries and the recording is no longer one of
them. `REUSE.toml` cannot annotate a file that is not here and says so where the annotations were.

Independent parsers (two TypeScript versions and Babel for the ESLint family; CPython and libcst for Python) read every
case source and every corpus file, so a file the engine cannot parse can be told from a file that is not valid source.
Nine of ten corpus files are source every parser accepts, and writing the same silent verdict list on each of them was
most of both recordings, so a recording names its parsers once in `parsers` and leaves `verdicts` off an entry no
parser objected to. The entry itself stays, with its path and its sha256: an entry with no `verdicts` means every
parser was silent, and no entry at all means the source was never recorded — a golden whose text changed, say — on
which the gates have no verdict and draw no conclusion.

## CI

`ci.yml` calls `port-readiness.yml` after its build job when a pull request or a push to main touches `packs/`,
`conformance/`, the engine, the root package files or the workflows: one job per gated tool-pack, then a report job
that writes the summary and fails when anything blocking failed.

### Sharding

A tool-pack too slow for one 60-minute job runs in several, each judging a share of its rules. The matrix gives such a
tool-pack's legs `shard` (1 to `shards`) and `shards`; each runs

```sh
npm run port-readiness -- --pack typescript --tool typescript-eslint --shard 2/6   # rules whose sha256(id) mod 6 is 1
```

A rule's shard depends only on its document id and the shard count, so adding or removing rules moves no other rule.
Every shard still checks the whole tool-pack (pins, recordings, the bundled-rule load), writes its report under
`shard-<i>of<n>/` and caches its results under `.rerail/port-readiness/results/<pack>/<tool>/shard-<i>of<n>/`. The report job
then merges each tool-pack's shards into one report before the summary:

```sh
npm run port-readiness -- merge .rerail/reports/port-readiness           # shard-*of*/<pack>--<tool>.json -> <pack>--<tool>.json
npm run port-readiness -- merge .rerail/reports/port-readiness --write   # and the shards' golden promotions into goldens.tsv
```

A tool-pack whose shards are missing, of another count, or disagree on what they share gets no merged report, so the
summary fails it. `--write` does not take `--shard` (shards run side by side); merge the shards with `--write` instead.
