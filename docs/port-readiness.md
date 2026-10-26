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
readiness". It was called the rule gates while this was built. What it keeps outside the tree is named for port
readiness too: the recording (`recording.tar.zst`, `rerail.port-readiness-recording-archive/v1`), the lock that names
it (`rerail.port-readiness-recording-lock/v4`), its Actions cache entries (`port-readiness-recording-<pack>-<tool>-<sha256>`)
and the marker on the recording
summaries. The file that configures the gates themselves (`gates.yaml`) is named for the gates, as it should be.

## The gates

A rule's verdict is the first blocking gate it fails, in this order. Every gate still runs on every rule, so the
report shows every failure, not only the first.

| Gate | What it checks | Blocks |
| --- | --- | --- |
| `loads` | The document is written for the current rule contract (`rerail.policy/v1`, one `mode`, no severity), passes the duplicate-key check, the policy schema and the engine compile (as written and as stable JSON), loads through the bundled-rule loader, has no `llm` clause, and its scope covers every file type upstream lints, or the ones the tool-pack's `gates.yaml` narrows it to (`rerail.in_scope`: typescript-eslint's ports cover TypeScript files only, since JavaScript files are the javascript pack's, and eslint-plugin-vue's cover `.vue` files only). The narrowing holds for the other gates too: `matches-upstream-tests` does not score an upstream test case of a type `in_scope` leaves out (eslint-plugin-vue's `.js` cases), reports it as not scored, "outside the tool-pack's rerail.in_scope (.js)", counts it in the identical-results detail and lists it under the rule's `outOfScope`, and `fires` and `stays-silent` must be met by in-scope cases alone. | yes |
| `not-overfit` | No word the rule hardcodes (patterns, fixes, regexes, names, static values) appears in upstream's tests unless upstream's own code explains it (the rule's implementation, its imports and helpers, the names upstream's regexes match) or it is a language keyword. Catches ports that pass upstream's tests by naming their fixtures. The regexes are not run over stress tests of upstream's own engine (a test text over 20,000 characters with fewer than one identifier per 1,000, such as ESLint's 1,000,000-backslash strings), which Rerail does not run. | yes |
| `compact` | A document over 15 KiB is inspected: it fails when a third or more of its clauses is the same structure written out over and over, or one node kind nests three or more times along one path. Catches rules that list cases instead of expressing the logic. | yes |
| `matches-upstream-tests` | Upstream, configured as upstream recommends, is the expected answer on its own tests for the rule. The rule must report the same findings at the same start and end positions, in both directions (`identical-results`), leaving out only an end part upstream does not report on every finding of the rule (when any finding of an ESLint rule on its own tests has no end, the rule is compared on starts alone; the report lists every rule compared without an end); and reproduce at least one upstream finding (`fires`). | yes |
| `passes-conformance` | The pack's own conformance check (`test:conformance`), which already blocks every shipped rule: on the pack's fixtures (upstream's cases and Rerail's probes, after `divergences.json`) the rule has no unexplained false positive or miss, catches something (or `pack.json` gives it examples), runs without engine failures and has no stale divergence; on the tool-pack's real-code samples it catches every labelled violation (`samples-catch-labelled`), reports nothing on a sample clean for it (`samples-silent-on-clean`) and runs without engine failures (`engine-reads-samples`); and it is silent somewhere it should be (`stays-silent`). See "Samples" below. Candidates are held to the same bar, so the gates are a superset of the check and "ready to promote" means CI accepts the rule once it ships. It comes after V1's gates, so V1's verdicts read the same. | yes |
| `upstream-status` | The rule exists upstream with an implementation and tests, is not deprecated, and one of upstream's recommended configurations turns it on. It decides only whether a shipped rule starts on; the report flags a pack.json `on` that disagrees. | no |

Tool-pack checks come before any gate. When one fails, every rule of the tool-pack fails with it: a shipped rule blocks
and a candidate is not ready to promote. They fail when the recording is stale (the recording checks), when the
recording archive is filed under another config than this branch's (record the tool-pack again), when a case source in
the checkout is not the bytes upstream's answers were recorded on (`case-sources`), or when the pack's shipped rules
do not load together through rerail's bundled-rule loader (`bundled-load`, for example two documents with one id). A
tool-pack whose samples tree was not restored fails only that tool's rules.

Known parser gaps (`known_gaps` in `gates.yaml`) excuse an upstream test file that differs where the engine reported a
parse failure on it, and nothing else. A gap holds when its probe occurs in every file it names, the engine still fails
to parse the probe, and the recorded independent parsers read the probe as valid source. A listed gap that does not
hold, or excuses no file, is a blocking failure until it is removed or corrected.

Each check's detail says why it failed, in a sentence: the first differing upstream test file and positions, the words
`not-overfit` found, the size metrics.

From V1's numbers: Gate 1 is `upstream-status`, 2 `loads`, 3 `not-overfit`, 4 `compact`, 5 `matches-upstream-tests`.
Gate 6, real code, is the tool-pack's real-code samples, which `passes-conformance` judges.

### Samples

A sample is a real file from an open-source repository, labelled for the pack's rules: per rule, the lines the rule
should report, or `[]` when the file is clean for it; a rule the labels do not name is not judged on the file. Labels
are line-only (a column, when present, is provenance). The format is `scripts/samples/format.ts`: each sample is
`<sample-id>/<source path>` beside its `<source path>.expected.json` (the source repository, commit, path, sha256 and
SPDX licence, and the labels), with `LICENSES.md` at the archive's root. No copyleft source is included.

A tool-pack's samples are one canonical archive in S3,
`s3://nera-test-fixtures/rerail/archives/samples/<pack>/<tool>/<sha256>.tar.zst`, written once, which the tracked
`packs/<id>/upstream/<tool>/samples.lock` names by its tar's sha256, file count and bytes, in `test-data.lock`'s
format. `SAMPLES.md` beside it lists the repositories, licences, commits and file counts. `npm run fixtures:pull`
fetches and extracts them with the test data (`node scripts/s3/pack-fixtures.mjs samples` alone), into
`.rerail/pack-data/samples/<pack>/<tool>/<sha256>/`, and CI's pack-test-data action carries them in the same cache
entry. Nothing derives or records them, and no command edits a set yet. To add, relabel or drop samples, or to rename
or drop a rule key (a label naming no rule of the tool-pack fails its whole pack's conformance check, which catches
typos), a maintainer's script loads the samples tree, changes it, and re-packs it with `packSamples` and
`writeSampleSet` (`scripts/samples/pack.ts`), which write the archive into `.rerail/pack-data/archives/samples/` and
the tool-pack's `samples.lock` and `SAMPLES.md`. `node scripts/s3/pack-fixtures.mjs samples push --pack <id> --tool
<tool>` then puts the archive in S3.

The conformance runner judges each rule by intent, on the samples labelled for it:

- a report on a labelled line is a true positive, and a labelled line with no report is a miss;
- any report on a sample clean for the rule is a false positive;
- an extra report on a sample with labels for the rule is listed for review and fails nothing;
- a divergence with `"source": "samples"`, matched by the sample's path and line, explains a miss or a false positive.

`passes-conformance` reads four checks from it. `samples-catch-labelled` fails a shipped rule on an unexplained miss,
`samples-silent-on-clean` on an unexplained false positive, and `engine-reads-samples` on an engine failure on a
judged sample (a rule runtime error, a crash, a timeout, a work budget, an unreadable file) or one naming no file.
A sample's context files (the files beside it that give it manifest facts, configuration and imports) are never
judged: an engine failure there is listed in the report (`contextFailures`, the CLI's `ctx` column) and fails nothing.
A parse failure on a judged sample (a syntax error, `source-partial`, or a `source-incomplete` the engine reports
because this or another file was read with errors, which the runner tells from a read failure by its message:
`isParseIncomplete` in `conformance/languages/runner.ts`) is report-only: `engine-reads-samples` lists it ("N engine
failure(s); M file(s) read in part") without failing, and the sample stays judged, so a label the partial parse makes
the rule miss still fails as a miss. `stays-silent` needs relevant evidence that the rule is silent where it should be:
one of the rule's own upstream test cases where upstream reports nothing and the rule agrees, or one clean sample the
rule is silent on, read in full and finished: no parse failure and no engine failure on it, and none naming no file.
Other rules' fixture units and Rerail's probes do not count. Its detail reads "N own upstream case(s), M clean
sample(s)". Candidates are scored and reported but fail nothing.

### The recording

**One file per answer.** A tool-pack's recording is one tree, one file per answer of upstream on its own tests,
packed as one canonical `recording.tar.zst` (`conformance/port-readiness/recording/archive.ts`):

```text
config/<config-id>.json                                   the parts the config id hashes
verdicts/<parsers-id>/<sha256><type>[.script].json        independent parsers' verdicts on a case source ([] when all were silent)
```

An answer is named by everything it depends on, so it holds exactly while its name does. What invalidates one:

- **the config id**: upstream's pins (repository, commit, version, runtime), the runtimes' pin digests `recording.lock`
  records, every `gates.yaml` setting an adapter reads (its `upstream` section, the whole `eslint` section, and
  `rerail.in_scope`), the "Not copied" prefixes of the pack's `UPSTREAM.md` files, a digest of the recorder's code
  for the tool-pack's adapter (`ADAPTER_SOURCES` in `archive.ts`: the adapter, its drivers, the language toolkits and
  span units it reads, and `runtimes.ts`), and `RECORDING_FORMAT` (`archive.ts`), the version of the tree the recorder
  writes from upstream's answers (`record.ts`, `recording/tree.ts`, `recording/schema.ts`). A bump moves it, and so
  does a fix to an adapter, and every answer is asked again. A change to the rest of the recorder moves it only through
  `RECORDING_FORMAT`, which is bumped whenever the recorder writes a different tree for the same upstream output (its
  keys, file layout, encodings, span handling or what is recorded), never for a refactor or a comment:
  `test/conformance/recorders/recording-format.test.ts` pins the tree it writes for a fixture tool-pack, and fails
  until the format is bumped and the pin updated;
- **the case key** (below);
- **the rule**: a rule the recording gains has no answers yet.

An answer no recorded rule names any more stays in the archive until `--prune`, and a new config id drops the old
ones. `--full` records everything from nothing, which an incremental record must equal; `port-readiness:record
--check`, which the nightly and a pull request's recording leg run, asks everything again and fails when the locked
archive lacks a file it made or holds another. An incremental record never asks again about an answer the archive
holds, so it cannot fix one upstream now gives otherwise: `--check` then says to record with `--full`.

**Upstream's own tests are answered the same way.** The rest of the tree holds what upstream says about its tests and
itself (`recording/tree.ts`), under the same config id:

```text
tool/<config-id>/<field>.json                  whole-tool facts: identity, pack checks, licence, the test words and
                                               texts not-overfit reads, absent rules, parsers, pins and runtimes
tool/<config-id>/name-sets/<set>.json          names and regexes several rules share
rules/<config-id>/<rule>/rule.json             a rule's facts, names and case index: each case's identity, committed
                                               source, upstream's written expectation and the key of its answer
rules/<config-id>/<rule>/cases/<key>.json      upstream's answer on the case: {"expected": [...], "skipped": null | "<reason>"}
texts/<sha256>.json                            one inline upstream test text
```

A case's key is its source's sha256 and a digest of the other inputs its adapter feeds upstream: the path or file type
it is read at, its variant (options, test method, owner test), and, for PHP_CodeSniffer, tlint, stillat/blade-parser,
PHPStan and SQLFluff, the rules recorded with it. An ESLint, Psalm or SQLFluff case's answer also depends on the
configuration it carries in upstream's test file, which nothing recorded holds, so its key takes the case's place in
the test file too: positions cannot move while the config id pins upstream's commit. ESLint compares a rule on its
starts alone when one of its own findings has no end; that is read from the rule's answers, not recorded apart.
A verdict file is shared by case sources with the same text, file type and script.

So a record asks upstream only about what the tree lacks: nothing at all when it lacks nothing; the whole-tool facts
when they are missing; for Bandit, Ruff and ESLint, whose cases and answers do not depend on which other rules are
recorded, only a rule the pack names that has no complete record; for every other adapter, every rule again once the
rules the pack names change. A rule's cached gate result is keyed by its own part of the tree, so recording another
rule judges no other rule again.

## Which packs are gated

A **tool-pack** is one upstream tool in one pack, such as `python/ruff`. It is gated when it has
`packs/<id>/upstream/<tool>/gates.yaml`.

- **Gated:** `python/ruff`, `python/bandit`, `javascript/eslint`, `javascript/eslint-plugin-unicorn`,
  `react/eslint-plugin-react`, `react/eslint-plugin-react-hooks`, `typescript/typescript-eslint`,
  `typescript/eslint-plugin-vitest`, `next/eslint-plugin-next`, `php/phpcs`, `php/phpstan`, `php/psalm`,
  `blade/bladestan`, `blade/tlint`, `blade/blade-parser`, `sql/sqlfluff`, `rust/clippy` and `vue/eslint-plugin-vue`.
  The slowest run in shards: `SHARDS` in `conformance/port-readiness/plan.mjs` says which and how many (see
  "Sharding" below).
- **Pending:** none. A tool-pack listed in `conformance/port-readiness/pending.json` is still validated in
  nera-rule-validation until its recording lands, and the gates report it as pending without blocking.
- **Not gated:** packs with no upstream adapter (complexity ships rules; node ships only
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

The engine is this checkout's (`npm run build:native` first). The first run fetches upstream's pinned test files into
`.rerail/port-readiness/` and reads the tool-pack's samples from the tree `npm run fixtures:pull` extracted (or
fetches it); later runs reuse them and the results of every rule whose inputs did not change.
`--no-cache` judges every rule again. The command exits 1 when a shipped rule fails, as CI does.

### What a cached result is keyed by

A rule's result key covers everything its verdict can depend on: the gate code a cached verdict can read, the npm
packages the gate process loads, the tool-pack's manifest and recording, the rule's own document, conformance entry
and upstream case sources, and the engine, though only the part of the engine the rule can be judged by.
`passes-conformance` is not cached: the pack's conformance check, the samples included, runs on every run.

The tool-pack checks (the known gaps) are cached under a key of their own (`toolPackKey`). They read every rule of the
tool-pack, whichever shard runs them, so the key covers what they read of every rule: each rule's upstream name and
case sources, and the engine over all the rules' families. It also covers the shared inputs, and the judged rules'
keys, whose results the gap counts come from. An edit to one rule's case source therefore moves every shard's key.
Otherwise one shard would replay the old checks, another would compute new ones, and the merge would refuse them.

The gate code is `conformance/port-readiness/` without what no cached verdict depends on: what only the report job or
the recorder runs (the summary, the pull request comment, the shard merge, the not-gated and pending lists, the
recording diff), report rendering (a leg renders `report.md` from a finished report), the leg plan and the README:
`GATE_CODE` in `cache.ts`, and `cache.test.ts` fails if `run.ts`'s imports reach an excluded file.
The npm packages are the locked closure (`package-lock.json`) of `GATE_PACKAGES` (better-sqlite3, diff, drizzle-orm,
semver, tsx, yaml, zod), plus `package.json`'s `type`, the `port-readiness` script with its pre and post
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
that leg saves neither its results nor upstream's test files. The PHP build and the LFS objects are saved by an explicit step
straight after they are made, before any recorder runs, and the nightly's recording check restores the recorder's
cache without saving it. GitHub scopes a pull request's cache to that pull request,
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
3. **Not gated and pending** packs, and any new or stale entry.
4. **Candidates:** where candidates stop, gate by gate, and why each one stopped. Candidates that pass every gate are
   listed as promotable: passing the gates is necessary to ship, not sufficient; reviewers still decide.
5. **On or off:** shipped rules whose `on` in pack.json disagrees with `upstream-status`.
6. Per tool-pack details: each gate on its own, run integrity notes.

A rule that fails: fix the rule (the check's detail names the first difference), or move it to `candidates/` (out of
`pack.json`, its `conformance.json` path updated).

## What the gates read

The gates read what a recorder wrote, the tool-pack's samples, and upstream's pinned test files, which they fetch:

```text
packs/<id>/upstream/<tool>/
├── gates.yaml          upstream pins, recommended configurations, known parser gaps, and the team's decisions for the tool
├── recording.lock      the manifest: which recording this commit's rules are judged against (conformance/port-readiness/recording/store.ts)
├── test-data.lock      which archive of pack test data this commit's tool-pack holds, by sha256 (scripts/s3/pack-fixtures.mjs)
├── samples.lock        which archive of real-code samples the tool-pack's rules are judged on, by sha256 (scripts/s3/pack-fixtures.mjs)
├── SAMPLES.md          the samples' source repositories, licences, commits and file counts
└── fixtures/           upstream's test files, copied (listed in licenses/<project>/UPSTREAM.md); pack test data, not in git
```

`fixtures/` is pack test data, not in git ([CONTRIBUTING.md](../CONTRIBUTING.md#pack-test-data)): upstream's copies,
the case sources the recorder names and the cases Rerail wrote are one archive per tool-pack in S3, which
`test-data.lock` names and `npm run fixtures:pull` fetches. Without it the `case-sources` check fails, and the recorder
refuses to record a tool-pack that has a lock, both saying to pull. In CI a test-data job, which runs nothing but this
commit's `scripts/s3/pack-fixtures.mjs`, warms the Actions cache with each planned tool-pack's archive
(`.github/actions/pack-test-data` with `fetch: "true"`): it looks up the entry keyed by the lock without restoring it
and, on a miss, downloads the archive from S3 under the repository's read-only OIDC role and saves it. The gate legs
can run upstream's code (the recorder), so they hold no OIDC token and consume the entry (`fetch: "false"`): they
restore it, verify the archive against the lock and extract it, and fail on a miss. So the gates run for pushes and
same-repository pull requests only: GitHub gives a fork's or Dependabot's pull request no OIDC token. Pull request CI
never runs on data it derived: a pull request that changes the pins, a generator or the recorder runs on the archive
its lock names until a maintainer derives the data again, away from any credential, and pushes it from a trusted
worktree of main (`npm run fixtures:push -- --from <ref>`), which writes the new lock for the pull request to commit.
CI derives a tool-pack only to check a changed lock's archive against its pins (**Test data reproduces from the pins**,
[below](#ci)). A pull request that adds or changes authored cases checks them in, and the pull keeps every file the
commit tracks; the maintainer who merges it archives them the same way, and the pull request commits the new lock and
untracks them ([CONTRIBUTING.md](../CONTRIBUTING.md#merging-maintainers)). CI picks the legs from the git diff
(`ci.yml`'s filter and `plan.mjs`), and the lock is in the diff, so no change to the data goes unjudged.

### The recordings are not in the tree

A recording — upstream's facts and its expected findings per rule on its own tests — is generated and large. The
sixteen of
them were 71 MB as JSON, and git keeps every version of each forever, so a pack re-recorded after an upstream bump cost
another few megabytes of history nobody can read. Each tool-pack commits a `recording.lock` of a few hundred bytes
instead, naming exactly the archive its rules are judged against: its tar's sha256, file count, bytes and format, and
the pins and runtimes it was made with. The bytes live in `.rerail/port-readiness/recordings/` (`<sha256>.tar.zst`,
and its tree extracted to `tree/<sha256>/`), where the recorder writes them, and every read checks them against the
lock: bytes that are not the ones the lock names read as absent.

Their durable store is S3: each archive is an object of its own at
`s3://nera-test-fixtures/rerail/recordings/<pack>/<tool>/<sha256>.tar.zst`, named by its tar's sha256 as the lock's
`files["recording.tar.zst"].sha256` names it, and written once, by a conditional write that never overwrites. In CI the
Actions cache is the fast path: an entry keyed by the tool-pack and the lock's digest
(`port-readiness-recording-<pack>-<tool>-<sha256>`) holds the archive at
`.rerail/port-readiness/recordings/`. Before any leg starts, the test-data job looks that entry up without restoring
it; on a miss it downloads the archive from S3 under the read role (`node scripts/s3/pack-fixtures.mjs recordings --pack
<id> --tool <tool>`, which verifies it against the lock) and saves the entry when S3 held it. A leg restores the
entry, so a commit whose recording is in the cache or in S3 records nothing, unless its pull request changes the
tool-pack's `recording.lock` (below). S3 and the cache, not Git LFS, because every public clone and every fork would
draw on the repository's LFS bandwidth quota and going over it blocks LFS for the whole repository. A cache entry is a
copy, never the source: GitHub evicts one unused for a week, and the next run reads S3 again.

Locally, `node scripts/s3/pack-fixtures.mjs recordings [--pack <id> [--tool <tool>]]` does the same for the
checkout's locks: it keeps a copy in `.rerail/port-readiness/recordings/` that matches its sha256, downloads the rest
with your AWS credentials, and names, without failing, any S3 does not hold yet, which you record.

**A pull request's leg records what neither the cache nor S3 holds, and every tool-pack whose `recording.lock` the
pull request changes.** The plan job marks a tool-pack `rerecord` when its `recording.lock` differs from the base's
(the merge commit's first parent, or the event's base commit when the base has moved); then the test-data job fetches
the archive the base's lock names instead of the new lock's, and the legs restore none, so the rules are judged
against what CI's recorder made from the pinned upstream, never against a recording the pull request, or the bump run
that produced it, put in S3. A leg that records installs the pinned runtimes, runs `port-readiness:record` with this
commit's recorder over the archive the base's lock names (which the test-data job fetches from S3 into
the cache; a base from before the one archive gives none, and everything is asked), and fails unless the recorder wrote
exactly the lock and `UPSTREAM.md` the commit has and the case sources the archive its `test-data.lock` names holds. It
compares every file under the tool-pack and the pack's `licenses/`, tracked or not, since pack test data is git-ignored
and `git status` would not show a case source the recorder added; such a file is one `npm run fixtures:derive` does not
make, and the derivation is what needs the fix. The lock names no recorder version, so a release of rerail leaves it
as it was. Only then does the leg save the archive (a `rerecord` leg saves none: no run restores it), and
nothing else: the recorder ran upstream's code and the Composer packages it resolved, so the leg saves neither
upstream's test files nor its results. The recorder runs without the
job's token. A new tool-pack, a new port or a pin bump therefore goes green on its own pull request, with nobody to
start a recording: the commit's lock is checked against upstream run in CI, and what records the answers (the
recorder, its adapters and runtimes) is in the pull request's diff, as the gate code that judges them is. A lock
pointed at a recording of the pull request's own making fails the same way, since CI's recording does not match it.
Whether it restored it or recorded it, each tool-pack's legs upload the archive as the artifact
`recordings-<pack>--<tool>` (the `<sha256>.tar.zst` file, kept for 7 days; a sharded tool-pack's first shard uploads
it), which is how the maintainer who merges the pull request gets it into S3:

```sh
gh run download <run-id> -n recordings-<pack>--<tool> -D .rerail/port-readiness/recordings
npm run fixtures:push -- --from pr-<n>   # in a trusted worktree of main (CONTRIBUTING.md, "Merging (maintainers)")
```

A leg's recording is never written to S3 from CI: the leg ran upstream's code, and the only CI credential that can
write to the bucket is the `test-data-upload` environment's role, which only the bump's publish workflow on main
assumes. A push to main or a nightly never records: its leg fails when neither the cache nor S3 holds the recordings
the lock names, saying to push them. So the pull request must not merge before they are in S3, and `ci.yml`'s check
**Pack test data is in S3** (`node scripts/s3/pack-fixtures.mjs check`) holds it: it fails, naming each missing
recording, test data archive and samples archive with its lock and how to push it, until S3 holds every one the locks
name.

GitHub scopes a pull request's cache entries to that pull request, so main reads only what main's own pushes and the
nightly saved; an entry a pull request saved holds the bytes its own recorder made, which the pull request could as
well have judged by editing the gate code. And because the recordings are not in the diff, the leg writes a per-rule
summary (`--changes`, below) against the recording the base's lock names (which the test-data job fetched from S3 for
a `rerecord` tool-pack; a first recording when the base has no lock) into its job summary and its report artifact,
and the report job puts each in the pull request's port readiness comment under "Recorded by this run": CI computed
it, so it is the summary to trust, not one a pull request body carries.

### Recording

`port-readiness:record` writes the recording into `.rerail/port-readiness/recordings/`, its `recording.lock` into the
tool-pack's directory, and any case source the recording names that the tool-pack's `fixtures/` lacks, with its row in
`UPSTREAM.md`. The lock and `UPSTREAM.md` are committed; the case sources are derived pack test data, which
`npm run fixtures:derive` writes from the pins after the generator, as the recorder would, and which go into the
tool-pack's archive, never into git; the recordings are neither, and reach S3 when a maintainer pushes them or the
bump's publish workflow uploads them. Record again after adding a port or changing a tool's pins: a record asks
upstream only for what the archive lacks, so a new port records that rule alone (Bandit, Ruff, ESLint) and a record
with nothing missing runs no upstream. It needs `uv`, the network, the pinned upstream runtimes
(`conformance/recorders/port-readiness/runtimes/`) and the pack test data (`npm run fixtures:pull`, then
`npm run fixtures:derive` after a pin or generator change):

```sh
npm run port-readiness:record -- --pack python --tool ruff            # whatever the archive lacks: tool facts, rules
npm run port-readiness:record -- --pack python --tool ruff --full     # everything again, from nothing
npm run port-readiness:record -- --pack python --tool ruff --prune    # and drop what no recorded rule names
```

The `php` and `blade` tool-packs also need the pinned PHP first on `PATH`, and Composer. That PHP is built by
`conformance/recorders/port-readiness/runtimes/php/build.sh`, the script CI's gate and nightly legs build it with
(`.github/actions/php`), into the prefix it is given, on Linux (with the development packages the action installs) or
on macOS (with Xcode's command line tools and the Homebrew formulae the script names when one is missing):

```sh
conformance/recorders/port-readiness/runtimes/php/build.sh "$PWD/.rerail/php/8.5.11"
PATH="$PWD/.rerail/php/8.5.11/bin:$PATH" npm run port-readiness:record -- --pack php --tool phpcs
```

`rust/clippy`'s derived data and recordings are Linux's, made on `x86_64-unknown-linux-gnu` as CI's legs and the bump
workflow make them: Clippy lints only the code rustc compiles for the host. Derive and record it on Linux, or take
what CI's leg records: a leg whose check fails leaves the lock and the archive a record makes in its report artifact
(`recorded-rust--clippy/`), and once the commit has that lock the leg's `recordings-rust--clippy` artifact holds it
for the maintainer to push.

Locally that is all: the lock names what was just recorded and the gates read it from
`.rerail/port-readiness/recordings/`, so iterating on a port needs no network. The flow for a pull request that
changes a tool-pack:

1. Pull the tool-pack's test data and recordings (`npm run fixtures:pull -- --pack <id> --tool <tool>`, then
   `node scripts/s3/pack-fixtures.mjs recordings --pack <id> --tool <tool>`) and, after a pin or generator change,
   derive the data again (`npm run fixtures:derive -- --pack <id> --tool <tool> --update`); record, run the gates, and
   commit `recording.lock` and `UPSTREAM.md`, with any authored cases (`git add -f`). A case source the recorder wrote
   that the derivation did not is a bug in the derivation: fix it rather than commit the file. Until a maintainer
   pushes newly derived data, CI runs on the archive the old `test-data.lock` names.
2. CI's test-data job looks the archive the commit's lock names up in the cache, then in S3, unless the pull
   request changes the lock: then it fetches the base lock's, and every leg of the tool-pack runs
   `port-readiness:record --check --base <the base's lock>` from the pull request's commit. It asks upstream for
   everything again and fails unless the commit's lock is what a record of it makes: an incremental
   record over the base's archive, or a `--full` or `--prune` one, so each mode the maintainer chose holds, and
   answers no recorded rule names are no failure; the case sources must be what it made too. A leg that
   finds an unchanged lock's archive in neither also checks, and saves it in the pull request's cache scope.
   Either way the legs upload it as the artifact `recordings-<pack>--<tool>`, and the comment carries the
   per-rule summary of each tool-pack checked.
   **Pack test data is in S3** stays red until S3 holds what the locks name.
3. Before merging, a maintainer derives any data the pull request changed away from every credential and pushes, from
   a trusted worktree of main, the archive and the recordings (the CI run's artifact, or the sandbox's own):
   `npm run fixtures:push -- --from pr-<n>` ([CONTRIBUTING.md](../CONTRIBUTING.md#merging-maintainers)). The pull
   request commits the new `test-data.lock` and untracks its data; **Test data reproduces from the pins** then derives
   the tool-pack again in CI and compares it with the archive.
4. After the merge, the first push to `main` finds the recordings in S3 and saves them into `main`'s cache scope, and
   every later run reads them from there. Nothing on `main` records: a push whose recordings S3 lacks fails.

`port-readiness-nightly.yml` runs the same recorder with `--check` in a read-only job of its own, so it reports when a
locked recording no longer matches the pinned upstream and saves nothing, then judges every rule again
(`--refresh-cache`) and saves the fresh results. Its `record-check-test-data` job warms the locked recordings from S3
as the gates' test-data job does, since `--check` compares the fresh answers with the locked archive's.

Because the recordings are no longer in the diff, `port-readiness:record` writes a per-rule summary of what moved instead —
expected findings gained and lost on upstream's own tests (compared
whatever config id they are filed under), facts changed
(`--changes <file>`, `conformance/port-readiness/recording/diff.ts`) — and a gate leg that records puts it in its job
summary and report artifact, from which the report job copies it into the pull request comment.
When the committed lock names a recording that cannot be read (neither the cache nor S3 held it, or the lock or the
recording fails its schema), the summary warns that it was not compared rather than calling it a first recording.

The lock pins every runtime the recorder installs for the tool-pack: upstream's own (`upstream.runtime`), the adapter's
(`parsers` for ESLint, `python` for Ruff and Bandit, `php` for PHP_CodeSniffer, PHPStan and Psalm, `blade` and `php` for tlint, `php` for stillat/blade-parser, whose own runtime is the Blade toolkit's `blade`) and the environment the pack is linted in
(`eslint.environment.runtime`).
`pins-agree` fails when the lock's runtimes are not exactly those (`recordedRuntimes` in `manifest.ts`), so a
`gates.yaml` that names a new runtime, or moves to another runtime directory, needs the tool-pack recorded again.

Each runtime is pinned by its `package.json`, `pyproject.toml` or `composer.json` alone: every direct dependency at an
exact version and in `overrides` (the recorder refuses a range), and no lockfile is committed (they are git-ignored).
A Python runtime also pins its exact interpreter in `.python-version`, which uv installs; a PHP runtime pins its exact
interpreter in `.php-version`, which Composer cannot install, so the recorder refuses to run on any other `php`
(`pinnedPhp` in `runtimes.ts`) and the gate legs that record and the nightly build that release. Every PHP process runs with no
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
again; the pin moves the runtime pin digests, so the config id, and every answer is asked again. Left unpinned, the
package is caught up with as `--check` says, with `--full`: an incremental record never asks
again about an answer the archive holds. A recording's per-rule summary (`--changes`, which a gate leg that records uploads in its artifact) lists every package each Composer runtime resolved (`name@version`, from `vendor/composer/installed.json`),
so a transitive release shows between two recordings; the list is in neither the archive nor the lock, so a
release that moves no answer fails nothing. Composer audits what it resolved with blocking off (`--no-blocking`): an
advisory is written to the recorder's log and does not stop the install.

`not-overfit` runs a rule's regexes over every upstream test text, as V1 did. Upstream's test files are not copied into
the pack: the recording lists each one with its sha256, and the gates fetch them from upstream at the pinned commit
(sparse, cached by their pins), so Ruff's GPL-derived fixtures are read in place and never committed. Test text that
is not a file of its own (Ruff's test-module strings, the `code` of each ESLint RuleTester case) is kept verbatim in
the archive (`texts/`, which the whole-tool `fixtureTexts` names). That makes the recording partly upstream's material,
so it carries upstream's licence text (`licenseText`, whose sha256 is `licenseSha256`) wherever the recording is copied. The
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
the pinned PHP and nikic/php-parser for PHP; libpg_query for SQL; the pinned rustc for Rust) read every case source,
so a file the engine cannot parse can be told from a file that is not valid source. A `.sql` file is
a psql script, so libpg_query reads the SQL psql would send for it: psql's meta-commands (`\echo`, `\gset`, `\if`, ...)
and the rows of `COPY ... FROM STDIN` become whitespace, a meta-command that sends the query buffer becomes `;`, and a
variable psql has defined becomes a stand-in of the same length (`sqlfluff_driver.py`'s `psql_sent_sql`), so the error
positions are the file's.
Most case sources are source every parser accepts, so a verdict file every parser was silent on is `[]`, which
reads back as each of the recording's parsers (named once, in the whole-tool `parsers`) silent. The file is there
either way: `[]` means every parser was silent, and no file at all means the source was never recorded, on which the
gates have no verdict and draw no conclusion. A verdict is named by the source's sha256, file type and script, so
case sources with the same text share one.

### Upstream releases

A pin says which release the rules were ported and recorded against; nothing above notices when upstream publishes a
newer one. `port-readiness-nightly.yml` therefore runs a job on main that does:

```sh
npm run port-readiness:upstream-releases   # conformance/port-readiness/upstream/releases.ts
```

For every gated tool-pack it reads the release `gates.yaml` pins (`upstream.version`) and the one its runtime installs
(`runtimes/<upstream.runtime>/package.json`, `pyproject.toml` or `composer.json`), and fails when they differ. The
package is the one the runtime installs upstream as: an ESLint-family upstream's comes from `eslint.release.installed`,
and every other adapter's is listed in `ADAPTER_PACKAGES`, since an upstream's repository is not always the name it
publishes under (PHPCSStandards/PHP_CodeSniffer is `squizlabs/php_codesniffer`, tighten/tlint is `tightenco/tlint`).
The Rust pack's Clippy ships with Rust, so its release is a toolchain's: `gates.yaml` pins its version and commit and
names the `clippy-<release>` runtime, whose `rust-toolchain.toml` pins the channel; `packs/rust/upstream/common.py`
refuses any toolchain other than `TOOLCHAIN`, and `conformance.json` names Clippy as `0.1.<minor>`. The check fails
unless they all agree. None of them is the repository's `rust-toolchain.toml`, which pins the toolchain Rerail is built
with.

It then asks each registry for the latest stable release: npm's `latest` dist-tag, the highest final PyPI release with
a file that is not yanked, the highest Packagist version without a `dev`, alpha, beta or RC suffix, and `[pkg.rust]` in
Rust's stable channel manifest (what rustup installs as `stable`; GitHub's releases of rust-lang/rust are marked latest
by hand and rate-limited without a token). Versions compare by their numeric components, a leading `v` dropped, and the
first component that differs is the gap: `patch`, `minor` or `major`, and `breaking-minor` for a minor release ahead of
a `0.x` pin. Rust and Clippy compare on major.minor only: a Rust patch release is not behind at all, and is not bumped.
A gap is breaking when it is `major` or `breaking-minor`. The job prints one table row per tool-pack (`Tool-pack`,
`Ecosystem`, `Package`, `Pinned`, `Latest stable`, `Gap`, `Status`; the gap reads `minor (breaking)`, say, when it is
breaking), appends the table to the job summary (`GITHUB_STEP_SUMMARY`), and lists below it the failing tool-packs'
problems, then the ones behind by a release that is not breaking. It annotates each failing tool-pack with `::error::`
and each other one behind with `::warning::`, and fails only on a breaking gap, pins that disagree, or a registry that
cannot be read: a check that cannot see upstream's releases does not pass. A minor or patch release ahead of the pin is
reported and bumped, but does not turn the nightly red. The pins stay exact, with nothing in `gates.yaml` allowing a
gap: until a bump merges the gates keep judging the rules against the pinned release, and the job stays red while any
pin is a breaking release behind. `--behind <file>` writes every tool-pack behind, breaking or not, sorted by
tool-pack, as JSON (`{pack, tool, toolPack, pinned, latest, gap, breaking}`), for the night's bump.

#### Bump pull requests

An upstream release is a pin change, and the pack test data derived from the new pins, archived. Pull request CI
derives that data only to check an archive against its pins ([CONTRIBUTING.md](../CONTRIBUTING.md#pack-test-data)),
so a bump workflow produces it. Each night the nightly starts one bump of every tool-pack behind, breaking or not,
which ends in one pull request on `auto/bump-upstream` with a commit per tool-pack, each the bump of one tool-pack:

```sh
npm run port-readiness:bump -- --pack python --tool ruff [--version 0.17.1] [--dry-run]  # upstream/bump.ts
```

The bump moves every pin of the release together, and reads each file back to check that only the pins moved:

- `gates.yaml`: `upstream.version`, and `upstream.commit` as the commit the release's GitHub tag names (an annotated tag
  is followed to its commit), plus an ESLint upstream's `eslint.release.tag`. How an upstream tags its releases is
  learnt from the pin: the first of `eslint.release.tag`'s form, `v<version>` and `<version>` whose tag of the pinned
  release names the pinned commit. A release without that tag stops the bump.
- The runtime's pin file (`package.json`, `pyproject.toml` or `composer.json`): upstream's package, and on npm every
  package of its scope pinned at the same release (typescript-eslint's packages are released together). A runtime
  directory named `<name>-<version>` is renamed for the new release, and every `gates.yaml` naming it follows; the other
  test-time dependencies stay where they are, and the recording shows whether they still fit.
- `conformance.json`'s `tools[].version`.
- The generators' pins (`packs/<pack>/upstream/<tool>/*` and the pack's shared `upstream/*` scripts): the commit, npm's
  `gitHead` of the release where a generator pins that instead (eslint-plugin-react-hooks), a `...VERSION = "<release>"`
  constant, and an entry keyed by upstream's package (`"eslint-plugin-react": "7.37.5"`). A line naming the commit names
  its release too, and moves with it.
- Clippy: `TOOLCHAIN`, `RUSTC_VERSION` and `CLIPPY_VERSION` in `packs/rust/upstream/common.py`, read from the release's
  channel manifest (`channel-rust-<release>.toml`), and `CLIPPY_COMMIT`, rust-lang/rust-clippy's `rust-<release>` tag.
  rust-clippy tags `.0` releases only: a point release whose `src/tools/clippy` in rust-lang/rust is the `.0` release's
  tree keeps the `.0` tag's commit, and one that changed it stops the bump.

Deriving and recording run upstream's code at the new release, which nobody has reviewed yet, with its unlocked
transitive packages and Composer. Such code can do anything a step of its job can: read the runner's cache token and
write any cache key in the run's ref scope, or mint the job's OIDC token and assume the roles it is trusted for. So it
never runs in main's cache scope, which every pull request and push to main reads (the dependency and native build
caches, the port readiness verdicts), and never in a job that holds an OIDC token or a token that can write. The work
is split over three workflows:

1. The nightly's `bump-dispatch` job (`port-readiness-nightly.yml`) runs no code from the tree. It takes every
   tool-pack `port-readiness:upstream-releases --behind` lists, as `[{pack, tool, version}]` sorted by pack then tool,
   and leaves out each `<pack>/<tool>@<version>` a maintainer has already declined: one a closed, unmerged bump pull
   request carried (below). With `GITHUB_TOKEN` it then points the work branch `auto/bump-run/upstream` at the night's
   commit of main (deleting a branch an earlier run left and creating it again) and dispatches
   `port-readiness-bump.yml` there once, with that list. With nothing left, it starts nothing.
2. `port-readiness-bump.yml`, the producer, runs on the work branch and refuses any other ref, and any list that is not
   1 to 64 distinct `{pack, tool, version}` sorted by pack then tool, each a lower-case name. Its `scope` job deletes
   every cache entry in the branch's scope, which an earlier run's upstream code may have written. Its `test-data` job
   and its `recordings` legs, one per listed tool-pack, run only the tree's own code: they pull every tool-pack's
   archive, and the recordings each listed tool-pack's current lock names (which its new recording is compared with),
   under S3's read role into the branch's cache. The `bump` legs, one per listed tool-pack and each failing alone
   (`fail-fast: false`), hold no OIDC token and only a read-only `GITHUB_TOKEN` for the bump step. Each restores those
   archives (`fetch: "false"`) and recordings from the cache, bumps its tool-pack's pins, and derives
   (`npm run fixtures:derive -- --update`), records again from no answers (`port-readiness:record --full`: the bump
   moves every config id it records, so no locked answer would be reused) and archives
   (`node scripts/s3/pack-fixtures.mjs pack`, which writes the archive and rewrites `test-data.lock` without the
   network) every tool-pack whose runtime the bump changed: its own, and every tool-pack that installs a shared runtime such as `blade`. It runs on Linux, with the
   runtimes installed as the record legs install them, so `rust/clippy`'s answers are Linux's. What it changed in the
   tree (the pins, `recording.lock`, `test-data.lock` and the `UPSTREAM.md` rows of new copies) becomes a patch,
   uploaded as the leg's `bump-<pack>--<tool>` artifact with the new archives (`archives/<pack>/<tool>/<sha256>.tar.zst`),
   the archive every changed `recording.lock` names (`recordings/<sha256>.tar.zst`), the bump's summary and the
   per-rule summaries. Whatever the upstream code does with the run's cache token lands in the work branch's scope,
   which nothing trusted reads.
3. `port-readiness-bump-publish.yml` runs on main when a bump run ends (`workflow_run`), succeeded or failed: a leg that
   fails leaves the others standing. It runs with main's workflow and main's code, and treats every artifact as data:
   it runs nothing from one and restores no cache.
   - `run` accepts a dispatch on `auto/bump-run/upstream` whose commit is main's head or an ancestor of it and that
     started at most 24 hours ago. When main has moved, it skips the whole run with a notice (the next nightly bumps
     again) if anything the bump ran differs between the run's commit and main's head (the bump workflow,
     `.github/actions`, `.npmrc`, the package and TypeScript files, `conformance/`, `scripts/`, `src/engine/`, the Ruff
     adapter's `native/src/code_rule_dsl/scope/options.rs`, and the packs' upstream directories, licences and
     `conformance.json` files). Everything after runs at main's head, where the pull request's branch starts. It lists
     the run's legs (its `Bump <pack>/<tool> to <version>` jobs and how each ended) and its artifacts: the legs to
     verify are those that succeeded and left a `bump-<pack>--<tool>` artifact; an artifact of any other name is
     ignored.
   - `verify`, one leg per such artifact and each failing alone, holds only a read-only `GITHUB_TOKEN`: no OIDC token,
     no environment. Its artifact's `bump.json` must name its tool-pack and release; a bump that changed nothing
     (`changed` is false, or the patch is empty), or whose patch no longer applies to main's head, has nothing to
     compose. Otherwise it installs main's dependencies from main's lockfile with no cache, runs `port-readiness:bump`
     itself, and applies the artifact's patch to a separate index. `bump.json` must be the one it computed (apart from
     `leftovers`), and the two trees, staged as the producer stages them (the packs and the recorder runtimes), must be
     identical except, for the bumped tool-pack and each tool-pack `bump.json` records again, in `test-data.lock`,
     `recording.lock`, the tool's `UPSTREAM.md` and the files directly in `packs/<pack>/licenses/<project>/`
     (`<project>` is `gates.yaml`'s `rerail.project`): what packing, recording and the generators write. Any other
     difference fails, listing its paths, so the run cannot rewrite a generator, a recorder, `gates.yaml` or a runtime
     manifest. Its verdict, the artifact `verified-<pack>--<tool>`, holds the patch's sha256, the tree it makes, the
     release it moves from and whether that is breaking (from its own `bump.json`), and the tool-packs it derived
     again; or why there is nothing to compose. A failed leg leaves none.
   - `compose`, with the same read-only token, starts from main's head and takes the requested tool-packs in order. It
     leaves out, as "not bumped tonight" with the reason, a tool-pack whose bump leg failed, whose verify leg failed or
     found nothing to compose, whose patch does not apply after the ones before it, or that derives again a tool-pack
     an earlier one bumped or derived again: the first in order stays. Each other one's patch, checked against the
     sha256 its verify leg recorded, becomes a commit of its own, `Bump <pack>/<tool> to <version>`
     (`git apply --3way --index`), so no two commits touch the same tool-pack's locks, and any one can be dropped.
     The commits together are confined again: nothing outside the packs and
     `conformance/recorders/port-readiness/runtimes/` (a pack's `licenses/<project>/` one level deep), no symbolic link
     or submodule. It then builds and checks the pull request's body before anything is uploaded or pushed: a hidden
     marker per commit, `<!-- bump: <pack>/<tool>@<version> -->`, first; a summary table of every leg (tool-pack, from
     → to, breaking or not, bumped or why not); per commit the pin diff, the locks and what they name, and how to
     regenerate locally, sharing the one-commit caps (the file list at most 100 files and 12,000 bytes, the pin diff
     20,000, the test data section 8,000, the regeneration section 4,000) between the commits; what checks the test
     data; and each composed leg's own report in what room is left. The whole must be valid UTF-8 and at most 65,000
     bytes, or the job fails. Its outputs name each commit's tool-pack, release and tree; its `composed` artifact holds
     the commits (a git bundle) and the body. When nothing is composed, nothing is published.
   - `upload`, after `compose` and only when it composed something, runs in the `test-data-upload` environment, whose
     upload-only role is the one credential that can write to the bucket: conditional writes under `rerail/archives/`
     and `rerail/recordings/` (never an overwrite) and reads. It requires the composed commits to make the tree
     `compose` named, and takes only the `test-data.lock` and `recording.lock` files they change, the archives the
     former name from the composed legs' artifacts, and the artifacts' `recordings/<sha256>.tar.zst` files the latter
     name, copied into `.rerail/port-readiness/recordings/`. `node scripts/s3/pack-fixtures.mjs upload` then puts each
     changed tool-pack's archive and recording in S3 (`rerail/recordings/<pack>/<tool>/<sha256>.tar.zst`, write-once
     like the archives), verifying each against its lock before the upload and
     reading it back after. The archives and recordings are still the runs': only their digests
     are checked here, and the pull request's CI checks their content.
   - `pull-request`, after the upload, requires each composed commit to have the tree `compose` named, on main's head
     one after another, confines them again, commits the same trees as the `neralabs-ci` app's bot user on
     `auto/bump-upstream`, and opens or updates the one pull request with the app's token (CI runs on it, as on the
     Version packs pull request, and pulls the archives the new locks name). Its title is
     `Upstream releases: <N> tool-packs`, or the one commit's `Bump <pack>/<tool> to <version>`. It commits no lock
     whose archive or recordings `upload` did not put in S3, and counts only the app's own pull requests from this
     repository's branch. A branch already holds the bumps only when its last commits are the app's, on that commit of
     main, with the same messages and trees; a branch whose tip the app did not commit is left alone. The body puts
     first what the publish workflow established itself, then each bump run leg's own report as "Reported by the bump
     run, unverified", folded and fenced as plain text with mentions and hiding HTML removed: upstream's release links
     and changes since the pin, the tool-packs recorded again, each one's per-rule summary
     (`port-readiness:record --changes`), and the lines that still name the old release: prose (READMEs, comments,
     `UPSTREAM.md` headers, rule evidence) the bump does not rewrite, for the reviewer to settle.
   - `work-branch` then deletes `auto/bump-run/upstream` once `run` passed and there was nothing to publish (no leg to
     verify, or nothing composed) or the pull request was opened, unless a newer run has moved it.

Nothing merges itself: a maintainer reviews the pull request and merges it like any other. Each night rewrites the
branch from main only when the bumps it makes changed (a newer release, a tool-pack more or less, or a different
recording), and leaves alone a branch someone else pushed to. Closing the pull request without merging it declines
exactly the bumps it carried, its markers: the next nightlies leave each of those `<pack>/<tool>@<version>` out until a
newer release of it appears, and propose the rest. A bump that cannot be made (pins that already disagree, a release
without a tag) fails its leg with the reason, the other tool-packs are still proposed, and the pin stays behind until a
maintainer bumps it by hand.

When a tool-pack's gates fail on the pull request, merge the others without its commit. Each commit is one tool-pack's
bump, and no two derive the same tool-pack again, so dropping one leaves every other commit's locks as they are (a
script two bumps both edit, such as a pack's `upstream/common.py`, keeps the other's lines). From a branch of your own:

```sh
git fetch origin main auto/bump-upstream
git switch -c bump-upstream-<date> origin/auto/bump-upstream
git rebase --onto <its commit>~1 <its commit>   # drop that tool-pack's commit
git push -u origin bump-upstream-<date>
```

then open a pull request from that branch, which CI checks like any other (its archives and recordings are already
in S3), and close the nightly's: closing it declines the dropped tool-pack's release, so the nightly does not propose it
again until upstream publishes a newer one, while the tool-packs you merged are no longer behind. To keep the dropped
release instead, bump it by hand (`npm run port-readiness:bump`, then derive, record and push as the pull request's
"Regenerate locally" section says).

## CI

`ci.yml` calls `port-readiness.yml` after its build job when a pull request or a push to main touches `packs/`,
`conformance/`, the engine, the root package files or the workflows: a plan job, a test-data job per tool-pack it
plans, one job per leg, then a report job that writes the summary and fails when anything blocking failed. A push to
main is compared with the previous head only when CI finished green there; after a red, cancelled or missing run it
runs port readiness whatever it touches, so a failing shipped rule, or a change whose queued run was replaced, is never
skipped past.

Port readiness runs for pushes and same-repository pull requests only, because its test-data job warms the cache with
each planned tool-pack's test data archive (`.github/actions/pack-test-data`, `fetch: "true"`) and the recording archive
its `recording.lock` names, from S3 under an OIDC token on a cache miss (`id-token: write`, which `ci.yml` and the
nightly grant the called workflow), and GitHub gives a fork's or Dependabot's pull request none. The legs, which can
run upstream's code, hold no OIDC token: any of their steps could mint one and assume the read role, so they only
restore what the test-data job saved (`fetch: "false"`). The nightly's drift check (`record-check`) gets its archives
and recordings the same way, from `record-check-test-data`. The plan sees only the git diff, and every change to the
data is in it: the tool-pack's `test-data.lock` changes with its archive, its `recording.lock` with its recordings, and
a pull request that checks data in tracks it until the maintainer archives it. The plan job also marks each tool-pack
whose `recording.lock` a pull request changes (`rerecord`), whose legs record it instead of reading the recordings the
lock names ([above](#the-recordings-are-not-in-the-tree)); only a pull request's leg ever records.

Three more `ci.yml` jobs guard the data itself, outside port readiness:

- **Pack test data is in S3** (`test-data-in-s3`) runs `node scripts/s3/pack-fixtures.mjs check` under the read role on
  every push and same-repository pull request: a `head-object` per archive every `test-data.lock` names and per
  recording every `recording.lock` names, downloading nothing. It fails listing each missing key with its lock and the
  command that pushes it, so a pull request cannot merge with data only its own cache scope holds.
- **Test data reproduces from the pins** (`test-data-reproduces`) has one leg per tool-pack whose `test-data.lock` a
  pull request changes against its base. With no credentials it restores the archives (`fetch: "false"`), installs the
  runtimes the bump job installs (uv, the pinned PHP for `php` and `blade`, Clippy's toolchain for `rust/clippy`),
  copies the pulled data to `$RUNNER_TEMP/oracle/` under the same paths, and runs `npm run fixtures:derive -- --pack
  <id> --tool <tool> --compare <that copy>` without `--update`: the archive must be byte for byte what the pins derive,
  and the tracked files must come out unchanged.
- **CI result** (`ci-result`) needs every other job, runs whatever happened to them, prints each one's result and fails
  when any failed or was cancelled. It is the check the ruleset requires: GitHub reports a job skipped because a job it
  needs failed as skipped, and a skipped required check passes. A job skipped by its own condition (forks, Dependabot,
  the Release pull request, the path filters) does not fail it.

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
