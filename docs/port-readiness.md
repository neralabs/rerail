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
readiness". It was called the rule gates while this was built. What it publishes outside the repository is named for
port readiness too: the recordings (`recording.json`, `rerail.port-readiness-recording/v2`, and `corpus.json`,
`rerail.port-readiness-corpus/v2`) and the lock that names them (`rerail.port-readiness-recording-lock/v1`), the
`port-readiness-recordings` release and its assets, the branches the record workflow pushes, the marker on its comments
and the `record-port-readiness` label. The files that configure the gates themselves (`gates.yaml`, `goldens.tsv`) are
named for the gates, as they should be.

## The gates

A rule's verdict is the first blocking gate it fails, in this order. Every gate still runs on every rule, so the
report shows every failure, not only the first.

| Gate | What it checks | Blocks |
| --- | --- | --- |
| `loads` | The document is written for the current rule contract (`rerail.policy/v1`, one `mode`, no severity), passes the duplicate-key check, the policy schema and the engine compile (as written and as stable JSON), loads through the bundled-rule loader, has no `llm` clause, and its scope covers every file type upstream lints, or the ones the tool-pack's `gates.yaml` narrows it to (`rerail.in_scope`: typescript-eslint's ports cover TypeScript files only, since JavaScript files are the javascript pack's). | yes |
| `not-overfit` | No word the rule hardcodes (patterns, fixes, regexes, names, static values) appears in upstream's tests unless upstream's own code explains it (the rule's implementation, its imports and helpers, the names upstream's regexes match) or it is a language keyword. Catches ports that pass upstream's tests by naming their fixtures. | yes |
| `compact` | A document over 15 KiB is inspected: it fails when a third or more of its clauses is the same structure written out over and over, or one node kind nests three or more times along one path. Catches rules that list cases instead of expressing the logic. | yes |
| `matches-upstream-tests` | Upstream, configured as upstream recommends, is the expected answer on its own tests for the rule. The rule must report the same findings at the same start and end positions, in both directions (`identical-results`), leaving out only an end part upstream does not report on every finding of the rule (when any finding of an ESLint rule on its own tests has no end, the rule is compared on starts alone; the report lists every rule compared without an end); reproduce at least one upstream finding (`fires`); agree on at least one case where upstream reports nothing, in its tests or its goldens (`stays-silent`); agree with upstream on every golden (`identical-on-goldens`, `engine-reads-goldens`); and not be listed as a real-code gap (`no-real-code-gap`). See "Goldens" below. | yes |
| `passes-conformance` | The pack's own conformance check (`test:conformance`), which already blocks every shipped rule: on the pack's fixtures (upstream's cases and Rerail's probes, after `divergences.json`) the rule has no unexplained false positive or miss, catches something (or `pack.json` gives it examples), runs without engine failures and has no stale divergence; when the pack pins corpora, their recordings cover it. Candidates are held to the same bar, so the gates are a superset of the check and "ready to promote" means CI accepts the rule once it ships. It comes after V1's gates, so V1's verdicts read the same. | yes |
| `upstream-status` | The rule exists upstream with an implementation and tests, is not deprecated, and one of upstream's recommended configurations turns it on. It decides only whether a shipped rule starts on; the report flags a pack.json `on` that disagrees. | no |

Tool-pack checks come before any gate. When one fails, every rule of the tool-pack fails with it: a shipped rule blocks
and a candidate is not ready to promote. They fail when the recording is stale (`corpus-pins` and the recording checks),
when a committed case source is not the bytes upstream's answers were recorded on (`case-sources`), when a task or
corpus a golden names could not be fetched (`corpus-complete`, usually a failed download: run again), or when the pack's shipped rules do not load together through rerail's bundled-rule
loader (`bundled-load`, for example two documents with one id).

Known parser gaps (`known_gaps` in `gates.yaml`) excuse an upstream test file that differs where the engine reported a
parse failure on it, and nothing else. A gap holds when its probe occurs in every file it names, the engine still fails
to parse the probe, and the recorded independent parsers read the probe as valid source. A listed gap that does not
hold, or excuses no file, is a blocking failure until it is removed or corrected.

Each check's detail says why it failed, in a sentence: the first differing upstream test file and positions, the words
`not-overfit` found, the size metrics.

From V1's numbers: Gate 1 is `upstream-status`, 2 `loads`, 3 `not-overfit`, 4 `compact`, 5 `matches-upstream-tests`.
Gate 6, real code, runs in NRV only; rerail judges the goldens it found, in `matches-upstream-tests`.

### Goldens

A golden is a real-code file a rule must agree with upstream on. `packs/<id>/upstream/<tool>/goldens.tsv` lists each
by rule, task or corpus, path and the file's sha256; its expected answer is upstream's findings on that file in
`corpus.json`. Rerail does not run the rules over the corpus to find goldens, and never writes `goldens.tsv`. Finding
them is V1's Gate 6, which stays in nera-rule-validation (NRV): NRV writes every file where a rule and upstream differ
to its `validation/goldens/<pack>.tsv`, and a tool-pack's `goldens.tsv` is a reviewed list imported from there.

- **Every rule's goldens are imported, candidates' too,** filed under NRV's id for the rule (the document id at NRV's
  pinned rerail commit, or `<tool>:<upstream rule>`) and mapped to the rerail document by its upstream rule. A
  candidate held back by a golden stays held until it agrees there, and the gates see the moment it does.
- **A candidate that differs throughout takes one line instead.** A candidate that differs on more than 500 of its
  goldens (`SYSTEMATIC_FILES` in `goldens.ts`) is wrong on real code as a whole, not on cases: its goldens would only
  restate that, at the cost of judging thousands of files. It is listed once under `real_code_gaps` in the tool-pack's
  `gates.yaml`, with how many of its goldens it differs on, in their place. A listed rule fails `no-real-code-gap`, so
  it is never ready to promote, and a shipped rule cannot be listed. Once it agrees with upstream there, import its
  goldens and remove the entry. Eight typescript-eslint candidates are listed today; the next largest difference
  anywhere is 335 files.
- **A negative where the tests have none.** `stays-silent` needs a case where upstream reports nothing and the rule
  agrees. Ruff's upstream test files all have findings, so each shipped Ruff rule without another such case has one
  golden where upstream reports nothing: the best-ranked negative file of the last real-code run in rerail, the file
  richest in the rule's own constructs. NRV is to write these itself; then they are imported like the rest.

Run integrity names a golden or `real_code_gaps` entry for a rule the tool-pack does not have, a rule with both, and a
candidate that differs on more than 500 of its goldens. When the engine could not fully read a golden (a parse
failure, or its work budget ran out), the difference is an engine gap, not the rule's: the rule still fails, through
`engine-reads-goldens`, and the report lists it under "Blocked by engine gaps" so the fix goes to the engine
(NER-1764), not the rule.

**Upstream's answers cover the goldens' files only.** The recorder fetches the files `goldens.tsv` names and asks
upstream about those alone, and the gates fetch the same files; nothing in rerail reads the rest of the corpus. Where
upstream's answer on a file depends on files around it, the recorder hands upstream those too and records nothing for
them: Ruff takes a file's package, and so its module name (N999), from the `__init__.py` files above it
(`CONTEXT_FILES` in the recorder), and without them it flags 19 of django's locale packages it does not flag in the
whole corpus. So an
imported golden whose file `corpus.json` does not answer fails its rule ("corpus.json has no answer for it") until the
tool-pack is recorded again, as a new port needs a recording before its upstream tests can be judged.
`port-readiness:record --check`, which the nightly runs, accepts a locked `corpus.json` recorded over more files (the
whole corpus, before the recorder narrowed) as long as it answers every golden's file as a fresh recording does; the
next re-record drops the rest.

## Which packs are gated

A **tool-pack** is one upstream tool in one pack, such as `python/ruff`. It is gated when it has
`packs/<id>/upstream/<tool>/gates.yaml`.

- **Gated:** `python/ruff`, `python/bandit`, `javascript/eslint`, `javascript/eslint-plugin-unicorn`,
  `react/eslint-plugin-react`, `react/eslint-plugin-react-hooks`, `typescript/typescript-eslint`,
  `typescript/eslint-plugin-vitest`, `next/eslint-plugin-next`. The slowest run in shards: `SHARDS` in
  `conformance/port-readiness/plan.mjs` says which and how many (see "Sharding" below).
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

The engine is this checkout's (`npm run build:native` first). The first run fetches the real-code files the goldens
name into `.rerail/port-readiness/` (pinned); later runs reuse them and the results of every rule whose inputs did not change.
`--no-cache` judges every rule again. The command exits 1 when a shipped rule fails, as CI does.

### What a cached result is keyed by

A rule's result key covers everything its verdict can depend on: the gate code a cached verdict can read, the npm
packages the gate process loads, the tool-pack's manifest, recordings and corpus pins, the rule's own document,
conformance entry, goldens and upstream case sources, and the engine, though only the part of the engine the rule can be
judged by.

The gate code is `conformance/port-readiness/` without what no cached verdict depends on: what only the report job or
the recorder runs (the summary, the pull request comment, the shard merge, the not-gated and pending lists, the
recording diff), report rendering (a leg renders `report.md` from a finished report), the leg plan and the README:
`GATE_CODE` in `cache.ts`, and `cache.test.ts` fails if `run.ts`'s imports reach an excluded file.
The npm packages are the locked closure (`package-lock.json`) of `GATE_PACKAGES` (apache-arrow, better-sqlite3, diff,
drizzle-orm, tsx, yaml, zod), plus `package.json`'s `type`, the `port-readiness` script with its pre and post
scripts, and `build:native`. Any other script, field or locked package is out, so a dependency bump the gates never load re-judges nothing;
`cache.test.ts` bundles `run.ts` and fails if it loads a package outside that closure.

That part of the engine is its shared code plus the files of the rule's own **language family** (`languages/*.json`'s
`family`: `ecma` for JavaScript and TypeScript, `python` for Python, `rust` for Rust). `conformance/port-readiness/engine.ts`
lists each family's files explicitly: its vendored grammars and registry entries, the scope and value modules named for
it, its builtin tables, and the suppression readers only it honours. Rust is listed though no Rust tool-pack is gated,
so that its files leave the key of every ecma and python rule. Everything else is shared and so part of every rule's
key: `matcher/`, `compile/`, `workspace/` and `language/adapter/` (apart from three Rust files: the adapter
`language/adapter/rust.rs`, and `matcher/arguments/rust.rs` and `matcher/format_args/rust.rs`, which only that adapter
and style `rust` call), the lockfile, the host's TypeScript engine, and any engine file nobody listed. That last one is the direction the list fails in: a
missing entry costs one cold re-judge of everything, never a stale verdict. A rule whose document names no language, or
one the language registry does not know, keys on the whole engine for the same reason.

A Python-only engine change therefore judges the Python legs cold, and the unicorn leg, whose `unicorn-filename-case`
names no language, and leaves every other leg reading main's results. The cost is the mirror image: a file listed as
one family's that another family in fact reads would leave that family's verdicts stale until the nightly's uncached
run. `test/conformance/port-readiness/engine.test.ts` fails when a listed path matches no engine source, so the list
cannot rot silently.

Some things are deliberately out of the key. The Node.js version (CI pins only the major), `@napi-rs/cli` and
`tsconfig.json` could in principle move a verdict; the nightly's uncached run is what catches that. The engine sources
only `cargo test` compiles cannot. And the native binary: it is not bit-reproducible, so hashing it made a
build-cache miss on unchanged sources re-judge every rule, and the sources it was built from say the same thing
reproducibly. That holds only while the binary is their build, so a binary older than an engine source it is built from
(an edit not yet rebuilt) saves no result. A shard report does not record which binary made it, so shards run on a stale binary and merged after a rebuild are not caught: rerun them.
The test-only sources are every file a `#[cfg(test)] mod` declaration reaches, which is how the engine keeps its oracle
tests beside the code they check.

In CI every run saves its results, pull requests included. GitHub scopes a pull request's cache to that pull request,
so main never reads one: a push to main reads only what main's own pushes and the nightly saved. A pull request can
hand itself verdicts its own code computed, which it could already do by editing the gate code that decides them.

## Read the report

On a pull request, CI posts the results as a comment: each tool-pack's shipped and candidate counts, every shipped
rule that blocks and why, the candidates **ready to promote** (they pass every gate: the signal to move them into
`rules/`, with the `on` that upstream-status sets), and the other candidates grouped by the gate they stop at, collapsed. A new run collapses
the earlier comments as outdated, so the pull request keeps its history. The comment stays under GitHub's size limit by
shortening long lists; the job summary and the uploaded reports have everything. When the pull request judged only some
tool-packs, the comment and the job summary say how many of them the counts cover, and a line names the rest: "Not
judged on this change", with main's verdict standing for them. A pull request that judged none gets no new comment
unless a not-gated check fails, but its earlier comments are still collapsed as outdated. The job summary reads top to
bottom:

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
pinned real-code files the goldens name and upstream's pinned test files:

```text
packs/<id>/upstream/<tool>/
├── gates.yaml          upstream pins, recommended configurations, known parser gaps, real-code gaps, and the team's decisions for the tool
├── recording.lock      the manifest: which recording this commit's rules are judged against (conformance/port-readiness/recording/store.ts)
├── goldens.tsv         the rules' goldens, imported from NRV
└── fixtures/           upstream's test files, copied (listed in licenses/<project>/UPSTREAM.md)
```

### The recordings are not in the tree

A recording — `recording.json`, upstream's facts and expected findings per rule, and `corpus.json`, its findings on
the real-code files the goldens name — is generated, large, and rewritten whole every time a tool-pack is recorded. The sixteen of
them were 71 MB, and git keeps every version of each forever, so a pack re-recorded after an upstream bump cost
another few megabytes of history nobody can read. They live in a GitHub release instead, and each tool-pack commits a
`recording.lock` of a few hundred bytes naming exactly the one its rules are judged against: its sha256, its size, its
format, the pins it was made at and the release that holds it.

Release assets rather than Git LFS, because every public clone and every fork would draw on the repository's LFS
bandwidth quota and going over it blocks LFS for the whole repository; rather than Actions artifacts, because those
expire after 90 days and a commit must stay judgeable. An asset is named `<pack>-<tool>-recording-<sha256>.json.gz` or
`<pack>-<tool>-corpus-<sha256>.json.gz`, so it is write-once: the same recording is the same asset, and nothing a
commit already names is ever replaced. A lock naming its asset any other way is refused, so it cannot point at another
tool-pack's recording.

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
one are those recorded on a maintainer's machine and uploaded by hand before that workflow could run from `main`;
`LIFTED_ASSETS` in `recording/store.ts` lists them, nothing joins the list once the workflow runs from `main`, and an
entry goes when its tool-pack gets an attested recording. A hand-uploaded recording made before each recorded case
carried the sha256 of its committed source has its case sources held by `case-sources` to one digest over all of them
instead, the one they had when it was recorded (`LIFTED_CASE_SOURCES`), and an edited or missing case source fails it
the same way.

### Recording

`port-readiness:record` writes the recording into the local cache, the `recording.lock` beside it, the gzipped assets to
upload under `.rerail/port-readiness/recordings/upload/`, and any case source the recording names that the tool-pack's
`fixtures/` lacks, with its row in `UPSTREAM.md`. The lock, the case sources and `UPSTREAM.md` are committed; the
recordings are not. Record again after adding a port,
changing a tool's pins, adding a corpus or importing a golden whose file `corpus.json` does not answer; it needs `uv`, the network, and the pinned upstream runtimes
(`conformance/recorders/port-readiness/runtimes/`):

```sh
npm run port-readiness:record -- --pack python --tool ruff
```

Locally that is all: the lock names what was just recorded and the gates read it out of the cache, so iterating on a
port needs no release and no network. In CI only `port-readiness-record.yml` records, and it is the only workflow that
uploads. Pull request CI never runs an upstream linter, so it cannot tell a recording that is what upstream says from
one written to make a rule pass; in a public repository that would be a way past the gates. A maintainer starts a
recording with the `record-port-readiness` label, or by hand with a pull request and the head `sha` they reviewed, and
the run records **only that commit**: if the pull request has moved on since, it fails rather than record the newer
push. The recorder and the runtimes come **from the base branch**, and only the pull request's `packs/` and rule list
from its head (so the goldens it records answers for are the head's); a fork may not change the upstream its
`gates.yaml` pins (repository, commit, runtime, tests), only `popularity`, `known_gaps` and `real_code_gaps`. Only a run from `main`, for `main` or a pull request into `main`, is attested, and the
gates take no other recording, so `authorize` refuses any other before a leg records: one started by hand from another
branch, and a stacked pull request, which is recorded once it targets `main`.

The run has three jobs, so nothing a pull request chose runs where a write token is. `authorize` checks who started
it and fixes the commits, the tool-packs and the release. One read-only `record` leg per tool-pack runs the recorder
and uploads what it wrote as an artifact. `publish`, the only job with a write token, runs no recorder, node or npm: it
checks every path and digest in those artifacts, attests the recordings it checked, uploads them, and commits every
leg's lock, case sources and `UPSTREAM.md` rows as one commit on the recorded head, pushed once:

- a pull request from a branch here gets the commit on its branch, as a fast-forward;
- a fork's goes to `port-readiness-recordings/pr-<n>` here, since the default token cannot write to a fork, and a
  comment says to merge it;
- a run started by hand from `main` with no pull request records `main`, pushes `port-readiness-recordings/run-<id>`,
  and opens a pull request into `main`: a recording never reaches a protected branch without review. (GitHub must let
  Actions create pull requests, under Settings → Actions → General.)

A push or pull request made with the workflow token starts no workflow, so CI runs on the recording commit once
someone pushes to the branch or closes and reopens the pull request.

GitHub holds at most 1,000 assets in a release. `authorize` uses the newest of `port-readiness-recordings`,
`port-readiness-recordings-2`, `-3` and so on, and starts the next once the newest holds 900; `publish` refuses an
upload that would pass 1,000. A lock names the release that holds its recordings, so an older commit keeps fetching
from the older one.

`port-readiness-nightly.yml` runs the same recorder with `--check` in a read-only job of its own, so it reports when a
committed recording no longer matches the pinned upstream and uploads nothing, then judges every rule again
(`--refresh-cache`) and saves the fresh results.

Because the recordings are no longer in the diff, `port-readiness:record` writes a per-rule summary of what moved instead —
expected findings gained and lost on upstream's own tests, corpus files answered differently, facts changed
(`--changes <file>`, `conformance/port-readiness/recording/diff.ts`) — and the publish job posts it on the pull request,
one comment per tool-pack, collapsing that tool-pack's earlier ones.
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
`recording.json` (`fixtureTexts.inline`). That makes the recording partly upstream's material, so it carries
upstream's LICENSE text (`licenseText`, whose sha256 is `licenseSha256`) wherever the release asset is downloaded, and
the upstream's `UPSTREAM.md` records it as excerpts under upstream's licence and Apache-2.0 — in the note under the
file table, not in the table itself, which pack check 6 reads as the files the pack carries and the recording is no
longer one of them. `REUSE.toml` cannot annotate a file that is not here and says so where the annotations were. No
case source or text comes from an upstream path a pack's `licenses/<project>/UPSTREAM.md` lists as "Not copied"
(`- Not copied: \`<prefix>\``): the recorder reads those lines, skips such a case as `not-copied` and refuses a
recording that would hold such a file's text inline.

Independent parsers (two TypeScript versions and Babel for the ESLint family; CPython and libcst for Python) read every
case source and every corpus file, so a file the engine cannot parse can be told from a file that is not valid source.
Nine of ten corpus files are source every parser accepts, and writing the same silent verdict list on each of them was
most of both recordings, so a recording names its parsers once in `parsers` and leaves `verdicts` off an entry no
parser objected to. The entry itself stays, with its path and its sha256: an entry with no `verdicts` means every
parser was silent, and no entry at all means the source was never recorded — a golden whose text changed, say — on
which the gates have no verdict and draw no conclusion.

## CI

`ci.yml` calls `port-readiness.yml` after its build job when a pull request or a push to main touches `packs/`,
`conformance/`, the engine, the root package files or the workflows: a plan job, one job per leg it plans, then a
report job that writes the summary and fails when anything blocking failed. A push to main is compared with the
previous head only when CI finished green there; after a red, cancelled or missing run it runs port readiness
whatever it touches, so a failing shipped rule, or a change whose queued run was replaced, is never skipped past.

The plan (`conformance/port-readiness/plan.mjs`) gives each gated tool-pack one leg, or one per shard. A pull request
judges only the tool-packs of the packs it touches: a change under `packs/<pack>/` plans that pack's tool-packs, while
a change to the engine, the gate code, the root package files or the workflows still plans every leg, and the
not-gated and pending lists, which the report job checks on every run, plan none. A tool-pack left out keeps main's
verdict. Pushes to main and the nightly judge every leg.

```sh
node conformance/port-readiness/plan.mjs                                                  # every leg
node conformance/port-readiness/plan.mjs --diff "$(git merge-base origin/main HEAD)" HEAD  # the legs this branch plans
```

### Sharding

A tool-pack too slow for one 60-minute job runs in several, each judging a share of its rules. `SHARDS` in
`conformance/port-readiness/plan.mjs` is the only place the counts live; the plan gives such a tool-pack's legs `shard`
(1 to `shards`) and `shards`, and each runs

```sh
npm run port-readiness -- --pack javascript --tool eslint --shard 2/4   # rules whose sha256(id) mod 4 is 1
```

A rule's shard depends only on its document id and the shard count, so adding or removing rules moves no other rule.
Every shard still checks the whole tool-pack (pins, recordings, the bundled-rule load), writes its report under
`shard-<i>of<n>/` and caches its results under `.rerail/port-readiness/results/<pack>/<tool>/shard-<i>of<n>/`. The report job
then merges each tool-pack's shards into one report before the summary:

```sh
npm run port-readiness -- merge .rerail/reports/port-readiness   # shard-*of*/<pack>--<tool>.json -> <pack>--<tool>.json
```

A tool-pack whose shards are missing, of another count, or disagree on what they share gets no merged report, so the
summary fails it.
