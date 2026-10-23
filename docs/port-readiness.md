# Port readiness

The port readiness gates decide whether a ported rule is good enough to ship. Every rule a pack ships (`packs/<id>/rules/`)
must pass them, and CI fails when one does not. Every candidate (`packs/<id>/candidates/`) runs through the same gates,
but its results are only reported, so CI shows how far each candidate got and why it stopped.

The gates sit on top of the [pack validation contract](pack-format.md#the-validation-contract) (checks 1 to 8) and the
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

The gates come in two stages. **Conformance** (`loads`, `compact`, `passes-conformance`) checks every rule document of
every pack. **Port readiness** checks ports, once they pass conformance. Port readiness (`not-overfit`,
`matches-upstream-tests`, and `upstream-status`, which decides only `on`) makes sure a port is true to the upstream
rule it comes from.

Every rule document declares its origin, one of two
([pack format](pack-format.md#conformancejson-divergencesjson-originsjson-upstream)):

- **A port** comes from an upstream linter's rule, such as Ruff's B905 or ESLint's `no-alert`. Its `conformance.json`
  entry declares it, naming that rule. A port the conformance runner cannot score yet, since no recording covers its
  upstream rule, is declared in the pack's `origins.json` instead, with its upstream rule and why no entry maps it.
- **An original** is Rerail's own rule (complexity's three, modelled on lizard): `origins.json` declares it, saying
  what it is. It is done after conformance, and a candidate that passes it is ready to promote.

The declaration is per document, not per pack: an original can sit in a pack of ports. A document that declares
neither fails conformance, at its first step, and the pack checks (check 1) fail it too.

A gated tool-pack's run judges its own rules on `loads`, `compact` and port readiness; the conformance job
(`conformance/port-readiness/conformance-gates.ts`) judges every other rule on conformance. `passes-conformance` is the
pack's conformance check, which scores every rule of the pack at once, so in CI it runs once per pack, in the
conformance job, for every rule the pack's `conformance.json` maps: the tool-pack runs and their shards skip it
(`--conformance-in-own-job`), and the summary takes each rule's result from the conformance job's report. A tool-pack run
started by hand runs the check itself. A rule's verdict is the first
step it fails, conformance first, so a port that fails conformance stops there, whatever port readiness would say.
Every gate still runs on every rule it applies to, so the report shows every failure, not only the first. The summary
holds one verdict per rule document (`STEPS` in `conformance/port-readiness/summary.ts`), so each rule is counted once.
Besides the gates, conformance starts with **Declares its origin: a port or an original**, and port readiness has these
steps, in this order:

1. **Mapped to an upstream rule:** the port is declared in `origins.json`, and no `conformance.json` entry maps it yet,
   so no port readiness gate can run on it. The verdict gives the `unmapped` reason.
2. **Set up for its upstream linter:** its upstream tool has no `gates.yaml` yet (node's `eslint-plugin-n`), so no port
   readiness gate can run on it. A shipped one fails: keep it in `candidates/` until the tool has a `gates.yaml`.
3. **Upstream linter checks:** the tool-pack checks below.
4. **Applies to the files upstream lints:** `loads`'s `in-scope` check, which judges the port, not the document.

Then `not-overfit` and `matches-upstream-tests`.

| Gate | Section | What it checks | Blocks |
| --- | --- | --- | --- |
| `loads` | conformance | The document is written for the current rule contract (`rerail.policy/v1`, one `mode`, no severity), passes the duplicate-key check, the policy schema and the engine compile (as written and as stable JSON), loads through the bundled-rule loader, has no `llm` clause, and, for a ported rule, its scope covers every file type upstream lints (`in-scope`, which judges the port, so the summary and the comment count it as a port step; a rule no gated tool-pack judges has no upstream to say which, so the check is not made), or the ones the tool-pack's `gates.yaml` narrows it to (`rerail.in_scope`: typescript-eslint's ports cover TypeScript files only, since JavaScript files are the javascript pack's, and eslint-plugin-vue's cover `.vue` files only). The narrowing holds for the other gates too: `matches-upstream-tests` does not score an upstream test case of a type `in_scope` leaves out (eslint-plugin-vue's `.js` cases), reports it as not scored, "outside the tool-pack's rerail.in_scope (.js)", counts it in the identical-results detail and lists it under the rule's `outOfScope`, and `fires` and `stays-silent` must be met by in-scope cases alone. | yes |
| `compact` | conformance | A document over 15 KiB is inspected: it fails when a third or more of its clauses is the same structure written out over and over, or one node kind nests three or more times along one path. Catches rules that list cases instead of expressing the logic. | yes |
| `passes-conformance` | conformance | The pack's own conformance check (`test:conformance`), which already blocks every shipped rule: on the pack's fixtures (upstream's cases and Rerail's probes, after `divergences.json`) the rule has no unexplained false positive or miss, catches something (or `pack.json` gives it examples), runs without engine failures and has no stale divergence; when the pack pins corpora, their recordings cover it. Candidates are held to the same bar, so the gates are a superset of the check and "ready to promote" means CI accepts the rule once it ships. It runs with the other conformance gates, before the port gates, so a rule that fails a conformance gate stops there whatever the port gates say. It does not apply to a rule no `conformance.json` entry maps, since the check has nothing to score it against: complexity has no `conformance.json` at all. | yes |
| `not-overfit` | port | No word the rule hardcodes (patterns, fixes, regexes, names, static values) appears in upstream's tests unless upstream's own code explains it (the rule's implementation, its imports and helpers, the names upstream's regexes match) or it is a language keyword. Catches ports that pass upstream's tests by naming their fixtures. The regexes are not run over stress tests of upstream's own engine (a test text over 20,000 characters with fewer than one identifier per 1,000, such as ESLint's 1,000,000-backslash strings), which Rerail does not run. | yes |
| `matches-upstream-tests` | port | Upstream, configured as upstream recommends, is the expected answer on its own tests for the rule. The rule must report the same findings at the same start and end positions, in both directions (`identical-results`), leaving out only an end part upstream does not report on every finding of the rule (when any finding of an ESLint rule on its own tests has no end, the rule is compared on starts alone; the report lists every rule compared without an end); reproduce at least one upstream finding (`fires`); agree on at least one case where upstream reports nothing, in its tests or its goldens (`stays-silent`); agree with upstream on every golden (`identical-on-goldens`, `engine-reads-goldens`); and not be listed as a real-code gap (`no-real-code-gap`). See "Goldens" below. | yes |
| `upstream-status` | port | The rule exists upstream with an implementation and tests, is not deprecated, and one of upstream's recommended configurations turns it on. It decides only whether a shipped rule starts on; the report flags a pack.json `on` that disagrees. | no |

Tool-pack checks judge the tool-pack, not a rule. When one fails, every rule of the tool-pack fails with it: a shipped
rule blocks and a candidate is not ready to promote. In a verdict they come after conformance and before the other
port readiness gates. They fail when the recording is stale (`corpus-pins` and the recording checks),
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
Gate 6, real code, does not run in rerail's CI; rerail judges the goldens a real-code run found, in
`matches-upstream-tests`. V1 ran `not-overfit` before `compact`; here every conformance gate comes first, so a rule that
fails both stops at `compact`.

### Goldens

A golden is a real-code file a rule must agree with upstream on. `packs/<id>/upstream/<tool>/goldens.tsv` lists each
by rule, task or corpus, path and the file's sha256; its expected answer is upstream's findings on that file in
`corpus.json`. Rerail's CI does not run the rules over the corpus to find goldens, and the gates never write
`goldens.tsv`. Finding them is V1's Gate 6, and each `goldens.tsv` names the run that found its goldens on its second
line (`# Source: ...`, which the gates require):

- **NRV's Gate 6** for the tool-packs nera-rule-validation pins (python, javascript, react, typescript, next): NRV writes
  every file where a rule and upstream differ to its `validation/goldens/<pack>.tsv`, and the tool-pack's `goldens.tsv`
  is a reviewed list imported from there.
- **Rerail's own Gate 6** for sql, rust and vue, which NRV pins no tool of: #141 ran #85's `matches-real-code` (at
  `f7d18209`, before #85 moved Gate 6 to NRV) over each pack's pinned corpora and the SWE tasks' solutions, with
  `--write`. Nothing finds new real-code disagreements for these three until real-code discovery runs again: in rerail,
  as a nightly job, is the follow-up the expected-answers plan lists.

For the NRV-imported tool-packs:

- **Every rule's goldens are imported, candidates' too,** filed under NRV's id for the rule (the document id at NRV's
  pinned rerail commit, or `<tool>:<upstream rule>`) and mapped to the rerail document by its upstream rule. A
  candidate held back by a golden stays held until it agrees there, and the gates see the moment it does.
- **A candidate that differs throughout takes one line instead.** A candidate that differs on more than 500 of its
  goldens (`SYSTEMATIC_FILES` in `goldens.ts`) is wrong on real code as a whole, not on cases: its goldens would only
  restate that, at the cost of judging thousands of files. It is listed once under `real_code_gaps` in the tool-pack's
  `gates.yaml`, with how many of its goldens it differs on, in their place. A listed rule fails `no-real-code-gap`, so
  it is never ready to promote, and a shipped rule cannot be listed. Once it agrees with upstream there, import its
  goldens and remove the entry. Nine typescript-eslint candidates are listed today; the next largest difference
  anywhere is 335 files.
- **A negative where the tests have none.** `stays-silent` needs a case where upstream reports nothing and the rule
  agrees. Ruff's upstream test files all have findings, so each shipped Ruff rule without another such case has one
  golden where upstream reports nothing: the best-ranked negative file of the last real-code run in rerail, the file
  richest in the rule's own constructs. NRV is to write these itself; then they are imported like the rest.

Run integrity names a golden or `real_code_gaps` entry for a rule the tool-pack does not have, a rule with both, and a
candidate that differs on more than 500 of its goldens. When the engine could not fully read a golden (a parse
failure, or its work budget ran out), the difference is an engine gap, not the rule's: the rule still fails, through
`engine-reads-goldens`, and the report lists it under "Blocked by engine gaps" so the fix goes to the engine,
not the rule.

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
  `typescript/eslint-plugin-vitest`, `next/eslint-plugin-next`, `php/phpcs`, `php/phpstan`, `php/psalm`,
  `blade/bladestan`, `blade/tlint`, `blade/blade-parser`, `sql/sqlfluff`, `rust/clippy` and `vue/eslint-plugin-vue`.
  The slowest run in shards: `SHARDS` in `conformance/port-readiness/plan.mjs` says which and how many (see
  "Sharding" below).
- **Not gated:** node's `eslint-plugin-n`, whose ports ship only as candidates. Conformance judges them; port readiness
  cannot run until the tool has a `gates.yaml`, so they stop at "Set up for its upstream linter", and a shipped one
  fails. complexity's rules are originals (its `origins.json`): conformance is all they need.

A port's upstream rule comes from its `conformance.json` entry, or from `origins.json` for a port no entry maps yet.

## Run them

```sh
npm run port-readiness -- --pack python --tool bandit   # one tool-pack; reports in .rerail/reports/port-readiness/
npm run port-readiness -- --pack python                 # every gated tool-pack of a pack
npm run port-readiness -- conformance                  # the conformance gates on every rule no tool-pack judges
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

The tool-pack checks (the corpus, `corpus-complete` and the known gaps) are cached under a key of their own
(`toolPackKey`). They read every rule of the tool-pack, whichever shard runs them, so the key covers what they read of
every rule: each rule's upstream name, goldens and case sources, and the engine over all the rules' families. It also
covers the shared inputs, and the judged rules' keys, whose results the gap counts come from. An edit to one rule's
golden or case source therefore moves every shard's key. Otherwise one shard would replay the old checks, another
would compute new ones, and the merge would refuse them.

The gate code is `conformance/port-readiness/` without what no cached verdict depends on: what only the report job or
the recorder runs (the summary, the pull request comment, the shard merge, the recording diff), report rendering (a leg renders `report.md` from a finished report), the leg plan and the README:
`GATE_CODE` in `cache.ts`, and `cache.test.ts` fails if `run.ts`'s imports reach an excluded file.
The npm packages are the locked closure (`package-lock.json`) of `GATE_PACKAGES` (apache-arrow, better-sqlite3, diff,
drizzle-orm, tsx, yaml, zod), plus `package.json`'s `type`, the `port-readiness` script with its pre and post
scripts, and `build:native`. Any other script, field or locked package is out, so a dependency bump the gates never load re-judges nothing;
`cache.test.ts` bundles `run.ts` and fails if it loads a package outside that closure.

That part of the engine is its shared code plus the files of the rule's own **language family** (`languages/*.json`'s
`family`: `ecma` for JavaScript and TypeScript, `python` for Python, `rust` for Rust, `php` for PHP, `blade` for
Blade, `sql` for SQL). `conformance/port-readiness/engine.ts` lists each gated family's files explicitly: its vendored
grammars and registry entries, the scope and value modules named for it, its builtin tables, and the suppression
readers only it honours. A family whose files embed another language's program keys on that family too (the
registry's `embeds`): a Blade template's semantics are the PHP of its compiled view, so a Blade rule keys on Blade's
files and PHP's, while a PHP rule never reaches Blade's parser and keys on PHP's alone. Vue has no list, so a Vue rule
keys on the whole engine. Everything else is shared and so part of every rule's key: `matcher/`, `compile/`,
`workspace/` and `language/adapter/` (apart from five files: Rust's adapter `language/adapter/rust.rs`, and
`matcher/arguments/rust.rs` and `matcher/format_args/rust.rs`, which only that adapter and style `rust` call; SQL's
adapter `language/adapter/sql.rs` and `matcher/arguments_sql.rs`, the call binding only it reads), the lockfile, the
host's TypeScript engine, and any engine file nobody listed. That last one is the direction the list fails in: a
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

In CI every run saves its results, pull requests included, except a gate leg that recorded its tool-pack itself: the
recorder ran upstream's code and unlocked Composer packages, which could have rewritten anything under `.rerail/`, so
that leg saves neither its results nor the corpus. The PHP build and the LFS objects are saved by an explicit step
straight after they are made, before any recorder runs, and the nightly's recording check restores the recorder's
cache without saving it. GitHub scopes a pull request's cache to that pull request,
so main never reads one: a push to main reads only what main's own pushes and the nightly saved. A pull request can
hand itself verdicts its own code computed, which it could already do by editing the gate code that decides them.

## Read the report

On a pull request, CI posts the results as one comment. It says only what a merge or a promotion needs:

1. **The heading:** ✅ or ❌, how many shipped rules pass the gates they must (conformance, and port readiness for a
   port), and how many candidates are ready to promote. Under it, one line on each stage.
2. **Shipped Rule Summary:** each blocker first, one line per shipped rule that fails (the pack and upstream rule it
   comes from, the stage and gate it stopped at, and why), with any blocking failure no rule owns (a known parser gap
   that does not hold or excuses nothing, a pack check, a missing report). Then one collapsed block per pack, `python:
   136`, holding its shipped rules by their declared origin (the upstream linter for a port, `original`, or `not
   declared` for a document that declares neither): passing, failing, total. A pack with a blocker opens by default.
3. **Candidate Rule Summary:** each candidate ready to promote first, the signal to move it into `rules/` (a reviewer
   still decides), with whether upstream recommends a port on by default. Then one collapsed block per pack, the same
   shape: its candidates by origin (ready, failing conformance, failing port readiness, total), and the gates they fail
   at, one row per gate and one column per origin.

No table is wider than its origins, so nothing scrolls sideways. A new run collapses the earlier comments as outdated,
so the pull request keeps its history. The comment stays under GitHub's size limit by capping its lists; the job
summary is the same page with every list in full and each failing candidate's gate and why, and `summary.json` and
the uploaded reports have everything else (each tool-pack's `report.md` has each gate on its own, `on` against upstream-status, and run integrity notes).
When the pull request judged only some tool-packs, a line names the rest: "Not re-run on this change", with main's
results standing for them; the counts leave them out. A pull request that judged no tool-pack and touched nothing the
conformance gates read gets no new comment unless a check fails, but its earlier comments are still collapsed as
outdated.

A rule that fails: fix the rule (the check's detail names the first difference), or move it to `candidates/` (out of
`pack.json`, its `conformance.json` path updated).

## What the gates read

The gates read what a recorder wrote, and fetch only the pinned real-code files the goldens name and upstream's
pinned test files:

```text
packs/<id>/upstream/<tool>/
├── gates.yaml          upstream pins, recommended configurations, known parser gaps, real-code gaps, and the team's decisions for the tool
├── recording.lock      the manifest: which recording this commit's rules are judged against (conformance/port-readiness/recording/store.ts)
├── goldens.tsv         the rules' goldens, and on its second line the real-code run that found them
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
one the lock names is not taken. A cached recording is read without checking its attestation again, so CI's
recordings cache is saved only after a fetch that found both recordings, and never by a leg that recorded them itself:
a later push with the same lock asks the release again rather than reading what an earlier push's recorder made.

**A leg records what the release does not hold.** When the lock names a recording the release does not have (the
release exists and has no asset of that name; the fetch exits 3), the leg records the tool-pack itself: it installs the
pinned runtimes, runs `port-readiness:record` with this commit's recorder and the release the lock names, and fails
unless the recorder wrote exactly the lock, case sources and `UPSTREAM.md` the commit has. The lock names no recorder
version, so a release of rerail leaves it as it was. Any other fetch failure (a release the lookup cannot find, which
GitHub answers alike for a missing tag, a repository the token cannot read and a wrong repository; any other releases
API error, a download, a digest, an attestation) fails the leg: only a recording that was never uploaded is recorded
instead. The recorder runs upstream's code and the Composer packages it resolves, so it runs without the job's token. A
new tool-pack, a new port or a pin bump therefore goes green on its own pull request, with nobody to start a recording:
the commit's lock is checked against upstream run in CI, and what records the answers (the recorder, its adapters and
runtimes) is in the pull request's diff, as the gate code that judges them is. A lock pointed at a recording of the pull
request's own making fails the same way, since CI's recording does not match it. The attested release stays the fast
path: a leg that fetches records nothing.

A fork's pull request records against the upstream its base pins, as `port-readiness-record.yml` does: the leg
refuses a fork's change to `gates.yaml`'s repository, commit, runtime or tests (only `popularity`, `known_gaps`,
`real_code_gaps` and comments may differ), since CI would otherwise run the upstream the fork chose and match the lock
the fork committed. It refuses a fork's change to the code the recorder runs for the same reason:
`conformance/recorders/port-readiness`, `conformance/port-readiness`, `src/engine` (which the recorder imports for
`PACKAGE_ROOT`), `package.json` and `package-lock.json` (the script that starts it, and the npm packages it imports)
and `.github/actions/php`. `cache.test.ts` checks that list against what a bundle of the recorder loads. A maintainer
records such a pull request with `port-readiness-record.yml`, which runs the base branch's recorder; a fork that touches
only `packs/` still records itself. The guard is no defence against a pull request that edits the workflow, since a
`pull_request` run takes the workflow from the pull request: reviewing a change to `.github/` is. And because the
recordings are not in the diff, the leg writes the per-rule summary the record
workflow posts (`--changes`, below), against the recording the base's lock names (a first recording when the base has
none), into its job summary and its report artifact.

A matching digest only says the asset is the one the lock names. Anyone who can write to the repository, and any
workflow with a write token, can upload to the release, so the fetch also requires the GitHub artifact attestation
that `port-readiness-record.yml` makes when it records from `main` (`gh attestation verify`, so fetching needs the GitHub
CLI, signed in or given `GH_TOKEN`/`GITHUB_TOKEN`, even once the repository is public), and takes no asset without it.
The 18 assets (each tool-pack's `recording.json` and `corpus.json`) recorded on a maintainer's machine and uploaded by
hand before that workflow could run from `main` were taken without an attestation (`LIFTED_ASSETS` in
`recording/store.ts`) until run 39319650531 recorded every tool-pack from `main`; the list is empty now and nothing
joins it. The two lists that went with it are empty too. `LIFTED_CASE_SOURCES`, which held a hand-uploaded recording's
case sources to one digest over all of them, was already empty before that run: the hand-uploaded recordings had gained
a sha256 per case. `LIFTED_RUNTIMES`, which pinned `eslint-config-next-16.3.8` for next/eslint-plugin-next,
react/eslint-plugin-react and react/eslint-plugin-react-hooks before their locks named it, emptied with that run.

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

The `php` and `blade` tool-packs also need the pinned PHP first on `PATH`, and Composer. That PHP is built by
`conformance/recorders/port-readiness/runtimes/php/build.sh`, the script CI's record and nightly legs build it with
(`.github/actions/php`), into the prefix it is given, on Linux (with the development packages the action installs) or
on macOS (with Xcode's command line tools and the Homebrew formulae the script names when one is missing):

```sh
conformance/recorders/port-readiness/runtimes/php/build.sh "$PWD/.rerail/php/8.5.11"
PATH="$PWD/.rerail/php/8.5.11/bin:$PATH" npm run port-readiness:record -- --pack php --tool phpcs
```

`rust/clippy`'s corpus answers depend on the host: Clippy lints only the code rustc compiles, and a `#[cfg(target_os =
"linux")]` item is compiled on Linux alone. Its recordings are CI's, recorded on `x86_64-unknown-linux-gnu`; recorded on
macOS, `corpus.json` lacks the findings in Linux-only code (tokio's `examples/prewarm-fd-table.rs`), so record it on
Linux or take the lock CI's leg writes (its report artifact holds the recording).

Locally that is all: the lock names what was just recorded and the gates read it out of the cache, so iterating on a
port needs no release and no network. In CI, a gate leg records a tool-pack whose recording the release does not hold
(above), and `port-readiness-record.yml` is the only workflow that uploads, so later runs fetch instead of recording.
A maintainer uploads one with the `record-port-readiness` label, or by hand with a pull request and the head `sha` they reviewed, and
the run records **only that commit**: if the pull request has moved on since, it fails rather than record the newer
push. The recorder and the runtimes come **from the base branch**, and only the pull request's `packs/` and rule list
from its head (so the goldens it records answers for are the head's); a fork may not change the upstream its
`gates.yaml` pins (repository, commit, runtime, tests), only `popularity`, `known_gaps` and `real_code_gaps`. Only a run from `main`, for `main` or a pull request into `main`, is attested, and the
gates fetch no other recording (an asset without that attestation fails the leg; only one the release lacks is
recorded in the leg instead), so `authorize` refuses any other before a leg records: one started by hand from another
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

The lock pins every runtime the recorder installs for the tool-pack: upstream's own (`upstream.runtime`), the adapter's
(`parsers` for ESLint, `python` for Ruff and Bandit, `php` for PHP_CodeSniffer, PHPStan and Psalm, `blade` and `php` for tlint, `php` for stillat/blade-parser, whose own runtime is the Blade toolkit's `blade`) and the environment the pack is linted in
(`eslint.environment.runtime`).
`pins-agree` fails when the lock's runtimes are not exactly those (`recordedRuntimes` in `manifest.ts`), so a
`gates.yaml` that names a new runtime, or moves to another runtime directory, needs the tool-pack recorded again.

Each runtime is pinned by its `package.json`, `pyproject.toml` or `composer.json` alone: every direct dependency at an
exact version and in `overrides` (the recorder refuses a range), and no lockfile is committed (they are git-ignored).
A Python runtime also pins its exact interpreter in `.python-version`, which uv installs; a PHP runtime pins its exact
interpreter in `.php-version`, which Composer cannot install, so the recorder refuses to run on any other `php`
(`pinnedPhp` in `runtimes.ts`) and the record and nightly workflows build that release. Every PHP process runs with no
php.ini (`php -n`, `PHP_INI` in `runtimes.ts`, which sets the few settings the gates depend on, UTF-8 as the default
charset among them), so the machine's settings never reach an answer, and the extensions a PHP loads do:
PHP_CodeSniffer's `Generic.PHP.DeprecatedFunctions` lists the deprecated functions `get_defined_functions()` finds.
So the build is pinned too: `runtimes/php/extensions.txt` lists the extensions `build.sh` compiles in (`php -n -m`,
sorted), the recorder refuses a `php` that loads any other set and names what is missing or extra, and the file is a
pin of the `php` runtime, which every PHP tool-pack's lock records. Only the manifests' direct
pins are in GitHub's dependency graph: Dependabot's version updates cover the root package alone, and a security update
against a runtime's pin changes it and fails `runtime-pins` until the tool-pack is recorded again. Transitive packages
resolve when a runtime is installed. When one of them changes what upstream
answers, the nightly `--check` fails and names the difference: pin that package too (an `overrides` entry in
`package.json`, a direct `==` dependency in `pyproject.toml`, or a direct requirement in `composer.json`) and record
again. A recording's per-rule summary (`--changes`, which both the record workflow's legs and a gate leg that records
upload in their artifact) lists every package each Composer runtime resolved (`name@version`, from `vendor/composer/installed.json`),
so a transitive release shows between two recordings; the list is in neither `recording.json` nor the lock, so a
release that moves no answer fails nothing. Composer audits what it resolved with blocking off (`--no-blocking`): an
advisory is written to the recorder's log and does not stop the install.

`not-overfit` runs a rule's regexes over every upstream test text, as V1 did. Upstream's test files are not copied into
the pack: the recording lists each one with its sha256, and the gates fetch them from upstream at the pinned commit
(sparse, cached like the corpus), so Ruff's GPL-derived fixtures are read in place and never committed. Test text that
is not a file of its own (Ruff's test-module strings, the `code` of each ESLint RuleTester case) is kept verbatim in
`recording.json` (`fixtureTexts.inline`). That makes the recording partly upstream's material, so it carries
upstream's licence text (`licenseText`, whose sha256 is `licenseSha256`) wherever the release asset is downloaded. The
licence is every file at upstream's root named `LICENSE` or `LICENCE`, in any letter case and with any extension or
suffix (`licence.txt`, `license.md`, `LICENSE-MIT`), their bytes one after another in name order; the pack keeps the
same files under `licenses/<project>/`, and `license-sha256` holds them to the recorded digest. The
upstream's `UPSTREAM.md` records the recording as excerpts under upstream's licence and Apache-2.0 — in the note under the
file table, not in the table itself, which pack check 6 reads as the files the pack carries and the recording is no
longer one of them. `REUSE.toml` cannot annotate a file that is not here and says so where the annotations were. No
case source or text comes from an upstream path a pack's `licenses/<project>/UPSTREAM.md` lists as "Not copied"
(`- Not copied: \`<prefix>\``): the recorder reads those lines, skips such a case as `not-copied` and refuses a
recording that would hold such a file's text inline.

Independent parsers (two TypeScript versions and Babel for the ESLint family; CPython and libcst for Python; `php -l` on
the pinned PHP and nikic/php-parser for PHP; libpg_query for SQL; the pinned rustc for Rust) read every case source and
every corpus file, so a file the engine cannot parse can be told from a file that is not valid source. A `.sql` file is
a psql script, so libpg_query reads the SQL psql would send for it: psql's meta-commands (`\echo`, `\gset`, `\if`, ...)
and the rows of `COPY ... FROM STDIN` become whitespace, a meta-command that sends the query buffer becomes `;`, and a
variable psql has defined becomes a stand-in of the same length (`sqlfluff_driver.py`'s `psql_sent_sql`), so the error
positions are the file's.
Nine of ten corpus files are source every parser accepts, and writing the same silent verdict list on each of them was
most of both recordings, so a recording names its parsers once in `parsers` and leaves `verdicts` off an entry no
parser objected to. The entry itself stays, with its path and its sha256: an entry with no `verdicts` means every
parser was silent, and no entry at all means the source was never recorded — a golden whose text changed, say — on
which the gates have no verdict and draw no conclusion.

## CI

`ci.yml` calls `port-readiness.yml` after its build job when a pull request or a push to main touches `packs/`,
`conformance/`, the engine, the root package files or the workflows: a plan job, one job per leg it plans, the
conformance job, which runs the conformance gates on every rule no tool-pack judges on every run (it reads no
recording and takes about a minute, most of it the javascript pack's conformance check), then a report job that writes the summary and fails when anything blocking failed. A push to main is compared with the
previous head only when CI finished green there; after a red, cancelled or missing run it runs port readiness
whatever it touches, so a failing shipped rule, or a change whose queued run was replaced, is never skipped past.

The plan (`conformance/port-readiness/plan.mjs`) gives each gated tool-pack one leg, or one per shard. A pull request
judges only the tool-packs of the packs it touches: a change under `packs/<pack>/` plans that pack's tool-packs, while
a change to the engine, the gate code, the root package files or the workflows still plans every leg. A tool-pack left
out keeps main's verdict. Pushes to main and the nightly judge every leg.

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
