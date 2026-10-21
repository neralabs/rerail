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
readiness too: the recordings (`recording.json`, `rerail.port-readiness-recording/v2`, and `corpus.json`,
`rerail.port-readiness-corpus/v2`), the lock that names them (`rerail.port-readiness-recording-lock/v2`), their
Actions cache entries (`port-readiness-recording-<pack>-<tool>-<sha256>-<sha256>`) and the marker on the recording
summaries. The files that configure the gates themselves (`gates.yaml`, `goldens.tsv`) are named for the gates, as they
should be.

## The gates

A rule's verdict is the first blocking gate it fails, in this order. Every gate still runs on every rule, so the
report shows every failure, not only the first.

| Gate | What it checks | Blocks |
| --- | --- | --- |
| `loads` | The document is written for the current rule contract (`rerail.policy/v1`, one `mode`, no severity), passes the duplicate-key check, the policy schema and the engine compile (as written and as stable JSON), loads through the bundled-rule loader, has no `llm` clause, and its scope covers every file type upstream lints, or the ones the tool-pack's `gates.yaml` narrows it to (`rerail.in_scope`: typescript-eslint's ports cover TypeScript files only, since JavaScript files are the javascript pack's). | yes |
| `not-overfit` | No word the rule hardcodes (patterns, fixes, regexes, names, static values) appears in upstream's tests unless upstream's own code explains it (the rule's implementation, its imports and helpers, the names upstream's regexes match) or it is a language keyword. Catches ports that pass upstream's tests by naming their fixtures. The regexes are not run over stress tests of upstream's own engine (a test text over 20,000 characters with fewer than one identifier per 1,000, such as ESLint's 1,000,000-backslash strings), which Rerail does not run. | yes |
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
  goldens and remove the entry. Nine typescript-eslint candidates are listed today; the next largest difference
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
  `typescript/eslint-plugin-vitest`, `next/eslint-plugin-next`, `php/phpcs`, `php/phpstan`, `php/psalm`,
  `blade/bladestan`, `blade/tlint`, `blade/blade-parser`. The slowest run in shards: `SHARDS` in
  `conformance/port-readiness/plan.mjs` says which and how many (see "Sharding" below).
- **Pending:** none. A tool-pack listed in `conformance/port-readiness/pending.json` is still validated in
  nera-rule-validation until its recording lands, and the gates report it as pending without blocking.
- **Not gated:** packs with no upstream adapter (complexity ships rules; node, sql and vue ship only
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

The tool-pack checks (the corpus, `corpus-complete` and the known gaps) are cached under a key of their own
(`toolPackKey`). They read every rule of the tool-pack, whichever shard runs them, so the key covers what they read of
every rule: each rule's upstream name, goldens and case sources, and the engine over all the rules' families. It also
covers the shared inputs, and the judged rules' keys, whose results the gap counts come from. An edit to one rule's
golden or case source therefore moves every shard's key. Otherwise one shard would replay the old checks, another
would compute new ones, and the merge would refuse them.

The gate code is `conformance/port-readiness/` without what no cached verdict depends on: what only the report job or
the recorder runs (the summary, the pull request comment, the shard merge, the not-gated and pending lists, the
recording diff), report rendering (a leg renders `report.md` from a finished report), the leg plan and the README:
`GATE_CODE` in `cache.ts`, and `cache.test.ts` fails if `run.ts`'s imports reach an excluded file.
The npm packages are the locked closure (`package-lock.json`) of `GATE_PACKAGES` (apache-arrow, better-sqlite3, diff,
drizzle-orm, tsx, yaml, zod), plus `package.json`'s `type`, the `port-readiness` script with its pre and post
scripts, and `build:native`. Any other script, field or locked package is out, so a dependency bump the gates never load re-judges nothing;
`cache.test.ts` bundles `run.ts` and fails if it loads a package outside that closure.

That part of the engine is its shared code plus the files of the rule's own **language family** (`languages/*.json`'s
`family`: `ecma` for JavaScript and TypeScript, `python` for Python, `rust` for Rust, `php` for PHP, `blade` for
Blade). `conformance/port-readiness/engine.ts`
lists each family's files explicitly: its vendored grammars and registry entries, the scope and value modules named for
it, its builtin tables, and the suppression readers only it honours. Rust is listed though no Rust tool-pack is gated,
so that its files leave the key of every ecma and python rule. A family whose files embed another language's program
keys on that family too (the registry's `embeds`): a Blade template's semantics are the PHP of its compiled view, so a
Blade rule keys on Blade's files and PHP's, while a PHP rule never reaches Blade's parser and keys on PHP's alone.
Everything else is shared and so part of every rule's
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

In CI every run saves its results, pull requests included, except a gate leg that recorded its tool-pack itself: the
recorder ran upstream's code and unlocked Composer packages, which could have rewritten anything under `.rerail/`, so
that leg saves neither its results nor the corpus. The PHP build and the LFS objects are saved by an explicit step
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
3. **Blocked by engine gaps:** shipped rules that fail only because the engine could not read a golden.
4. **Not gated and pending** packs, and any new or stale entry.
5. **Candidates:** where candidates stop, gate by gate, and why each one stopped. Candidates that pass every gate are
   listed as promotable: passing the gates is necessary to ship, not sufficient; reviewers still decide.
6. **On or off:** shipped rules whose `on` in pack.json disagrees with `upstream-status`.
7. Per tool-pack details: each gate on its own, run integrity notes.

A rule that fails: fix the rule (the check's detail names the first difference), or move it to `candidates/` (out of
`pack.json`, its `conformance.json` path updated).

## What the gates read

The gates read what a recorder wrote, and fetch only the pinned real-code files the goldens name and upstream's
pinned test files:

```text
packs/<id>/upstream/<tool>/
├── gates.yaml          upstream pins, recommended configurations, known parser gaps, real-code gaps, and the team's decisions for the tool
├── recording.lock      the manifest: which recording this commit's rules are judged against (conformance/port-readiness/recording/store.ts)
├── goldens.tsv         the rules' goldens, imported from NRV
└── fixtures/           upstream's test files, copied (listed in licenses/<project>/UPSTREAM.md); pack test data, in S3
```

`fixtures/` is pack test data: it is not in git but in `s3://nera-test-fixtures/rerail/`, at the paths it has here, and
`npm run fixtures:pull` puts it in the checkout ([CONTRIBUTING.md](../CONTRIBUTING.md#pack-test-data)). Without it the
`case-sources` check fails, and the recorder refuses to record a tool-pack that has a lock, both saying to pull it.
Every CI job that runs the gates or the recorder pulls it first, under the repository's OIDC role
(`.github/actions/pack-fixtures`), so the gates run for pushes and same-repository pull requests only: GitHub gives a
fork's or Dependabot's pull request no OIDC token. A pull request that adds or changes case sources checks them in,
and the pull wins over S3 for every file the commit tracks; the maintainer who merges it moves them to S3 and deletes
them from git in the same pull request. CI picks the legs from the git diff (`ci.yml`'s filter and `plan.mjs`), so that
deletion is what selects them; no change reaches S3 alone.

### The recordings are not in the tree

A recording — `recording.json`, upstream's facts and expected findings per rule, and `corpus.json`, its findings on
the real-code files the goldens name — is generated, large, and rewritten whole every time a tool-pack is recorded. The sixteen of
them were 71 MB, and git keeps every version of each forever, so a pack re-recorded after an upstream bump cost
another few megabytes of history nobody can read. Each tool-pack commits a `recording.lock` of a few hundred bytes
instead, naming exactly the one its rules are judged against: its sha256, its size, its format, and the pins and
runtimes it was made with. The bytes live in `.rerail/port-readiness/recordings/<sha256>.json`, where the recorder
writes them, and every read checks them against the digest: bytes that are not the ones the lock names read as absent.

In CI that directory is a GitHub Actions cache entry, keyed by the tool-pack and the lock's two digests
(`port-readiness-recording-<pack>-<tool>-<recording sha256>-<corpus sha256>`), so a commit that did not re-record hits
the entry an earlier run saved and records nothing. The cache, not Git LFS, because every public clone and every fork
would draw on the repository's LFS bandwidth quota and going over it blocks LFS for the whole repository. An entry is
a copy, never the source: GitHub evicts one unused for a week, and the leg then records again.

**A leg records what the cache does not hold.** On a miss the leg records the tool-pack itself: it installs the pinned
runtimes, runs `port-readiness:record` with this commit's recorder, and fails unless the recorder wrote exactly the
lock, case sources and `UPSTREAM.md` the commit has. It compares every file under the tool-pack and the pack's
`licenses/`, tracked or not, since pulled test data is git-ignored and `git status` would not show a case source the
recorder added. The lock names no recorder version, so a release of rerail leaves it as it was. Only then does the leg
save the two recordings, and nothing else: the recorder ran upstream's code and the Composer packages it resolved, so
the leg saves neither the corpus nor its results. The recorder runs without the job's token. A new tool-pack, a new
port or a pin bump therefore goes green on its own pull request, with nobody to start a recording: the commit's lock is
checked against upstream run in CI, and what records the answers (the recorder, its adapters and runtimes) is in the
pull request's diff, as the gate code that judges them is. A lock pointed at a recording of the pull request's own
making fails the same way, since CI's recording does not match it.

GitHub scopes a pull request's cache entries to that pull request, so main reads only what main's own pushes and the
nightly saved; an entry a pull request saved holds the bytes its own recorder made, which the pull request could as
well have judged by editing the gate code. And because the recordings are not in the diff, the leg writes a per-rule
summary (`--changes`, below) against the recording the base's lock names (restored from the cache when an earlier run
saved it; a first recording when the base has no lock) into its job summary and its report artifact.

### Recording

`port-readiness:record` writes the recording into the local cache, the `recording.lock` beside it, and any case source
the recording names that the tool-pack's `fixtures/` lacks, with its row in `UPSTREAM.md`. The lock and `UPSTREAM.md`
are committed; the case sources are pack test data, checked in with the pull request (`git add -f`, which the recorder
prints: pulled test data is git-ignored) and moved to S3 before it merges; the recordings are neither. Record again
after adding a port, changing a tool's pins, adding a corpus or importing a golden whose file `corpus.json` does not
answer; it needs `uv`, the network, the pinned upstream runtimes (`conformance/recorders/port-readiness/runtimes/`) and
the pack test data (`npm run fixtures:pull`):

```sh
npm run port-readiness:record -- --pack python --tool ruff
```

The `php` and `blade` tool-packs also need the pinned PHP first on `PATH`, and Composer. That PHP is built by
`conformance/recorders/port-readiness/runtimes/php/build.sh`, the script CI's gate and nightly legs build it with
(`.github/actions/php`), into the prefix it is given, on Linux (with the development packages the action installs) or
on macOS (with Xcode's command line tools and the Homebrew formulae the script names when one is missing):

```sh
conformance/recorders/port-readiness/runtimes/php/build.sh "$PWD/.rerail/php/8.5.11"
PATH="$PWD/.rerail/php/8.5.11/bin:$PATH" npm run port-readiness:record -- --pack php --tool phpcs
```

Locally that is all: the lock names what was just recorded and the gates read it out of the cache, so iterating on a
port needs no network. A maintainer's flow for a pull request that changes a tool-pack:

1. Record locally, run the gates, and commit the lock, `UPSTREAM.md` and any new case sources (`git add -f`).
2. CI's gate leg misses the cache, records the tool-pack from the pull request's commit, fails unless it wrote the same
   files, and saves the recordings in the pull request's cache scope.
3. Before merging, a maintainer runs `npm run fixtures:push`, which uploads the pull request's pack test data to S3 and
   untracks it, and pushes that commit; CI then pulls the data from S3 instead.
4. The first push to `main` after the merge records the tool-pack once more, in `main`'s scope, and every later run
   and pull request reads it from there.

`port-readiness-nightly.yml` runs the same recorder with `--check` in a read-only job of its own, so it reports when a
committed recording no longer matches the pinned upstream and saves nothing, then judges every rule again
(`--refresh-cache`) and saves the fresh results.

Because the recordings are no longer in the diff, `port-readiness:record` writes a per-rule summary of what moved instead —
expected findings gained and lost on upstream's own tests, corpus files answered differently, facts changed
(`--changes <file>`, `conformance/port-readiness/recording/diff.ts`) — and a gate leg that records puts it in its job
summary and report artifact.
When the committed lock names a recording that cannot be read (it is not in the cache, or the lock or the cached
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
again. A recording's per-rule summary (`--changes`, which a gate leg that records uploads in its artifact) lists every package each Composer runtime resolved (`name@version`, from `vendor/composer/installed.json`),
so a transitive release shows between two recordings; the list is in neither `recording.json` nor the lock, so a
release that moves no answer fails nothing. Composer audits what it resolved with blocking off (`--no-blocking`): an
advisory is written to the recorder's log and does not stop the install.

`not-overfit` runs a rule's regexes over every upstream test text, as V1 did. Upstream's test files are not copied into
the pack: the recording lists each one with its sha256, and the gates fetch them from upstream at the pinned commit
(sparse, cached like the corpus), so Ruff's GPL-derived fixtures are read in place and never committed. Test text that
is not a file of its own (Ruff's test-module strings, the `code` of each ESLint RuleTester case) is kept verbatim in
`recording.json` (`fixtureTexts.inline`). That makes the recording partly upstream's material, so it carries
upstream's licence text (`licenseText`, whose sha256 is `licenseSha256`) wherever the recording is copied. The
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
the pinned PHP and nikic/php-parser for PHP) read every
case source and every corpus file, so a file the engine cannot parse can be told from a file that is not valid source.
Nine of ten corpus files are source every parser accepts, and writing the same silent verdict list on each of them was
most of both recordings, so a recording names its parsers once in `parsers` and leaves `verdicts` off an entry no
parser objected to. The entry itself stays, with its path and its sha256: an entry with no `verdicts` means every
parser was silent, and no entry at all means the source was never recorded — a golden whose text changed, say — on
which the gates have no verdict and draw no conclusion.

### Upstream releases

A pin says which release the rules were ported and recorded against; nothing above notices when upstream publishes a
newer one. `port-readiness-nightly.yml` therefore runs a job on main that does:

```sh
npm run port-readiness:upstream-releases   # conformance/port-readiness/upstream-releases.ts
```

For every gated tool-pack it reads the release `gates.yaml` pins (`upstream.version`) and the one its runtime installs
(`runtimes/<upstream.runtime>/package.json`, `pyproject.toml` or `composer.json`), and fails when they differ. The
package is the one the runtime installs upstream as: an ESLint-family upstream's comes from `eslint.release.installed`,
and every other adapter's is listed in `ADAPTER_PACKAGES`, since an upstream's repository is not always the name it
publishes under (PHPCSStandards/PHP_CodeSniffer is `squizlabs/php_codesniffer`, tighten/tlint is `tightenco/tlint`).
The Rust pack's Clippy is not gated; it ships with Rust, so its pin is the toolchain `packs/rust/upstream/common.py`
refuses any other than (`TOOLCHAIN`, whose Clippy `conformance.json` must name as `0.1.<minor>`), not
`rust-toolchain.toml`, which pins the toolchain Rerail is built with.

It then asks each registry for the latest stable release: npm's `latest` dist-tag, the highest final PyPI release with
a file that is not yanked, the highest Packagist version without a `dev`, alpha, beta or RC suffix, and `[pkg.rust]` in
Rust's stable channel manifest (what rustup installs as `stable`; GitHub's releases of rust-lang/rust are marked latest
by hand and rate-limited without a token). Versions compare by their numeric components, a leading `v` dropped. The job
prints one table row per tool-pack, appends the table to the job summary (`GITHUB_STEP_SUMMARY`), annotates each failing
tool-pack with `::error::`, and fails when a pin is behind, the pins disagree, or a registry cannot be read: a check that
cannot see upstream's releases does not pass. A failure is a prompt to bump the pins and record the tool-pack again
("Recording" above), not a broken build; until then the gates keep judging the rules against the pinned release.

## CI

`ci.yml` calls `port-readiness.yml` after its build job when a pull request or a push to main touches `packs/`,
`conformance/`, the engine, the root package files or the workflows: a plan job, one job per leg it plans, then a
report job that writes the summary and fails when anything blocking failed. A push to main is compared with the
previous head only when CI finished green there; after a red, cancelled or missing run it runs port readiness
whatever it touches, so a failing shipped rule, or a change whose queued run was replaced, is never skipped past.

Port readiness runs for pushes and same-repository pull requests only, because every leg pulls the packs' test data
from S3 under an OIDC token (`id-token: write`, which `ci.yml` and the nightly grant the called workflow), and GitHub
gives a fork's or Dependabot's pull request none. The plan sees only the git diff: a change to pack test data alone
would plan no leg, so none is made alone (the maintainer who moves a pull request's data to S3 deletes it from git in
that pull request, which plans its tool-packs).

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
