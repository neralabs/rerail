# Port readiness

The port readiness gates judge every rule document of a gated tool-pack (one upstream tool in one pack: `python/ruff`,
`typescript/typescript-eslint`) against the upstream linter it ports, from the recordings each tool-pack's committed
`recording.lock` names. They are a layer on top of the pack contract (`test/engine/catalog/packs.test.ts`) and the
conformance check, and a superset of the check: its verdict is the `passes-conformance` gate
(checks/passes-conformance.ts), for candidates as well as shipped rules. Documents under `packs/<pack>/rules/` ship, and
a failing one fails CI. Documents under `packs/<pack>/candidates/**` (except `candidates/probes/**`) go through the same
gates and are only reported.

| V1 gate | Gate id | Blocking |
| --- | --- | --- |
| 1 | `upstream-status` (decides on or off) | no |
| 2 | `loads` | yes |
| 3 | `not-overfit` | yes |
| 4 | `compact` | yes |
| 5 | `matches-upstream-tests` | yes |
| 6 | none: real code runs in NRV only; `matches-upstream-tests` judges the goldens imported from it | — |
| — | `passes-conformance` (the pack's `test:conformance` verdict on the rule, judged after V1's gates; never cached) | yes |

A rule's verdict is the first blocking gate it fails. `matches-upstream-tests` includes `engine-reads-goldens`: a golden
that differs where the engine reported `source-partial`, `source-incomplete` or `budget-exceeded` fails the rule; the
fix belongs in the engine. It also includes `no-real-code-gap`: a rule `gates.yaml` lists under
`real_code_gaps` fails. The goldens and real-code gaps are reviewed data, imported from NRV's goldens or, where NRV pins no tool, found by
rerail's own real-code run (each goldens.tsv's source line says which), and never written by the gates (docs/port-readiness.md, "Goldens").

## Where the code comes from

The gates and the recorder's adapters are adapted, not copied, from nera-rule-validation (NRV,
`https://github.com/neralabs/nera-rule-validation`, `src/validation/` and `src/common/`): NRV `main` at
`e8727c77d0464bd33b1890e56dcf322119911a34`, plus two commits made on its `dbf1a0cdf733b6b6da0ed709b644afbe715ae9b4`,
`690f7223f7d62d23cb2c1cdfc2e910969b18d1e5` (Gate 6 skips type-checked rules, and a file that changed is a new golden)
and `77c266a2545c3fb39c4f9707c0c1c95f20d82cf2` (a golden the engine could not fully read is an engine gap; here the
blocking `engine-reads-goldens`). Both commits are in NRV's GitHub repository.

| NRV `src/validation/` | Here |
| --- | --- |
| `gates/compile.ts`, `generality.ts`, `size.ts`, `upstream-tests.ts`, `provenance.ts` | `checks/loads.ts`, `not-overfit.ts`, `compact.ts`, `matches-upstream-tests.ts`, `upstream-status.ts` |
| `gates/real-code.ts` | none: Gate 6 runs in NRV only, and its goldens are imported |
| `adapters/`, `languages/`, `runtimes.ts` | `conformance/recorders/port-readiness/adapters/` (`eslint_*` files are `adapters/eslint/eslint-*`), `languages/`, `runtimes.ts` |
| `pack.ts`; `cli.ts`, `corpus.ts`, `engine.ts`, `goldens.ts`, `policy-literals.ts`, `report.ts`, `run.ts`, `spans.ts`, `types.ts` | `manifest.ts`; the file of the same name, most of them rewritten |
| `rerail.ts` | none: the gates read this checkout, not a pinned rerail commit |

| NRV `src/common/` | Here |
| --- | --- |
| `process.ts` | `process.ts`, which also catches EPIPE on a child's stdin |
| `digest.ts` | `canonicalJson` in `checks/compact.ts`; `sha256` defined in each file that needs it |
| `manifest.ts` | none: `manifest.ts` validates `gates.yaml` with zod |

Only `policy-literals.ts` and `adapters/ruff-tests.ts` are unchanged. Every import points into this tree, and the checks
read rerail's layout. `upstream-status` is `provenance.ts` without its pack precondition: the kept licence's sha256 is a
tool-pack check here, and the pack contract's checks 5 and 6 cover the rest. The adapters record upstream instead of
answering the gates live, so each also lists the sources of its case texts (`fixtureTextSources`: a file's path and
sha256, or the inline text); their span code moved to `span-units/`, which the gates read without them; their
runtimes come from `conformance/recorders/port-readiness/runtimes/` instead of an NRV checkout; Ruff reads the engine's scope
options from this checkout instead of a pinned engine; Bandit's driver runs under `python -I`; and ESLint's recordings
write `/rerail-gates-cache` for the recorder's cache directory.

## Running

```sh
npm run port-readiness                                   # every gated tool-pack, reports in .rerail/reports/port-readiness/
npm run port-readiness -- --pack python --tool bandit    # one tool-pack
npm run port-readiness -- --no-cache                     # judge every rule again; read and save no results
npm run port-readiness -- --refresh-cache                # judge every rule again and save the fresh results (the nightly)
npm run port-readiness -- --pack javascript --tool eslint --shard 2/4   # one shard: rules with sha256(id) mod 4 = 1
npm run port-readiness -- merge .rerail/reports/port-readiness    # join each tool-pack's shard-<i>of<n>/ reports (shard.ts, merge.ts)
npm run port-readiness -- summary .rerail/reports/port-readiness --base <sha>
npm run port-readiness -- summary .rerail/reports/port-readiness --planned '["python/ruff"]'   # the others: not judged
node conformance/port-readiness/plan.mjs [--diff <from> <to>]   # CI's legs: every one, or the ones a diff selects
```

Exit 1 when a shipped rule fails (or the summary has a blocking failure), 2 on an exception. The run needs the built
native engine; it reads no upstream runtime. It fetches only the pinned corpus files the goldens name (from catalog task
files and the pack's `conformance.json` corpora), cached under `.rerail/port-readiness/corpus/`.

Per-rule results are cached under `.rerail/port-readiness/results/<pack>/<tool>/<key>.json`, a shard's under
`<tool>/shard-<i>of<n>/` (`--cache <dir>` moves the root).
The key covers the gate code a cached verdict can read (`GATE_CODE`), the locked closure of the npm packages the gates
load (`GATE_PACKAGES`) with `package.json`'s `type`, `port-readiness` and `build:native` scripts, the engine (tracked engine
sources; not the native binary, but a run on one older than them saves nothing), the tool-pack's `gates.yaml`, the corpus pins, and
the rule's document, conformance entry, goldens and case sources, and what the rule reads of the recording archive
(recording/tree.ts `ruleTreeDigest`: the config, the whole-tool facts, its own rule directory and corpus answers, the
verdicts on its sources and goldens, and the cases of any rule a known gap names), so recording another rule judges
none of the others again. The tool-pack checks' key takes the archive's sha256 whole. When every rule hits, nothing is fetched or
evaluated. An entry that is not JSON, was written by another cache layout (`CACHE_FORMAT`), lacks the shape of a result,
or names another document is a miss. In CI every run saves its results, pull requests included; GitHub scopes a pull
request's cache to that pull request, so main reads only what main's pushes and the nightly saved.

## Upstream releases

```sh
npm run port-readiness:upstream-releases   # every pin against its upstream's latest stable release
npm run port-readiness:bump -- --pack <pack> --tool <tool> [--version <release>] [--dry-run]   # move a tool-pack's pins
```

`upstream/releases.ts` reads each gated tool-pack's pin of its upstream twice, `upstream.version` in `gates.yaml` and the
package its runtime installs (`runtimes/<upstream.runtime>/`: `package.json`, `pyproject.toml` or `composer.json`), and
the Rust pack's Clippy pin (`TOOLCHAIN` in `packs/rust/upstream/common.py`, with `conformance.json`'s Clippy version). It
asks npm, PyPI, Packagist and Rust's stable channel manifest for each package's latest stable release, prints a table
with each pin's gap (patch, minor, major, or a minor of a `0.x` pin; Rust on major.minor only), and exits 1 only on a
breaking gap (a major release, or a minor one of a `0.x` pin), pins that disagree, or a registry it cannot read; a
minor or patch gap is reported without failing. `--behind <file>` writes every tool-pack behind, breaking or not, as
JSON. `upstream/bump.ts` moves one tool-pack's pins (gates.yaml, the runtime and its directory name, conformance.json,
the generators' constants; Clippy's toolchain) to a release, resolving its tag's commit on GitHub, and lists the prose
still naming the old one. `port-readiness-nightly.yml` runs the check on main and starts `port-readiness-bump.yml` once
on a work branch, with a leg per tool-pack behind, each running the bump; `port-readiness-bump-publish.yml` then
composes the verified legs into one pull request with a commit per tool-pack (docs/port-readiness.md, "Upstream
releases").

## Data

```
packs/<pack>/upstream/<tool>/gates.yaml         the tool-pack's manifest (manifest.ts)
packs/<pack>/upstream/<tool>/recording.lock     which recording this commit is judged against (recording/store.ts)
packs/<pack>/upstream/<tool>/test-data.lock     which archive of pack test data this commit holds (scripts/s3/pack-fixtures.mjs)
packs/<pack>/upstream/<tool>/goldens.tsv        goldens of matches-upstream-tests, by document id, and the real-code run that found them
packs/<pack>/upstream/<tool>/fixtures/          the case sources the recording names: pack test data, from that archive (npm run fixtures:pull)

.rerail/port-readiness/recordings/                       the recordings themselves, recorded or fetched from S3, by digest
  <sha256>.tar.zst                              recording.tar.zst: every answer upstream gave the recorder (recording/archive.ts,
                                                rerail.port-readiness-recording-archive/v1), a canonical tar compressed with zstd
  tree/<sha256>/                                that archive extracted, after its sha256, file count and bytes verify:
    config/<config-id>.json                     the parts the config id hashes
    tool/<config-id>/<field>.json               whole-tool facts: identity, pack checks, licence, the words and texts
                                                not-overfit reads, absent rules, parsers, pins (recording/tree.ts)
    tool/<config-id>/name-sets/<set>.json       names and regexes several rules share
    rules/<config-id>/<rule>/rule.json          a rule's facts, names and case index
    rules/<config-id>/<rule>/cases/<key>.json   upstream's answer on one of its cases
    texts/<sha256>.json                         one inline upstream test text
    answers/<config-id>/<rule>/<task>/<path>@<key>.json      one answer per rule and golden corpus file
    verdicts/<parsers-id>/<sha256><type>[.script].json       independent parsers' verdicts on a case source or golden
```

The recording is not in the tree: the lock names the archive by its tar's sha256, file count and bytes, and it is an
object of its own in S3, `s3://nera-test-fixtures/rerail/recordings/<pack>/<tool>/<sha256>.tar.zst`, written once.
`port-readiness:record` writes it into the cache, and `node scripts/s3/pack-fixtures.mjs recordings` fetches it there
from S3 (`recordings push` puts new ones there); in CI, `port-readiness.yml` restores it from the Actions cache, keyed
by the lock's digest, which its test-data job fills from S3, and a leg that finds it in neither (or whose pull request
changes the lock) checks it with `--check --base`, over the archive its base's lock names, which the test-data job
fetches too. A cached archive that is not the locked one is deleted and fetched again, and the gates extract a cached
archive themselves when its tree is not there; a tree is reused only while the marker its extraction wrote
(`.tar-sha256`) names the locked archive and the canonical tar of the tree's files is that archive, and is extracted
again from the verified archive otherwise. The gates read the recording at the one config the archive holds;
`corpus-answers` fails every rule when it is not this branch's.

Each answer is named by everything it depends on, and only that, so it stays valid while its name does: `<config-id>`
hashes upstream's pins, the runtimes' pin digests the lock records, the gates.yaml sections that say how upstream
lints a file (`upstream`, and ESLint's plugin, release, environment and `.vue` parsers), the recorder's code for the
adapter (`ADAPTER_SOURCES`) and `RECORDING_FORMAT`, the version of the tree the recorder writes (bumped when it writes
a different tree for the same upstream output, never for a refactor or a comment; `recording-format.test.ts` pins that
tree); `<rule>` is the rule with `/` written `~`. A case answer's `<key>` is its source's sha256
and a digest of the other inputs its adapter feeds upstream (recording/tree.ts `caseKey`: the path or file type, the
variant, and for ESLint, Psalm and SQLFluff the case's place in upstream's tests, whose own configuration no recorded
field holds; for PHP_CodeSniffer, tlint, stillat/blade-parser, PHPStan and SQLFluff also the rules recorded with it).
A corpus answer's `<task>` is the golden's task or rerail corpus with
`:` written `~`; `<key>` is the file's sha256, plus, for Ruff, a digest of the `__init__.py`/`__init__.pyi` files above
it that the corpus holds, for PHPStan and Psalm a digest of every golden file of its task, which they analyse as one
project, and for Clippy the corpus's commit; for stillat/blade-parser and PHPStan, whose answer for one rule depends
on the other rules asked with it (`ASKS_EVERY_RULE`), the digest is also of the rules the tool-pack records, so a rule
added or removed asks every answer again. An answer file is `{"findings": [...]}` (`[]`: asked, upstream silent) or
`{"excluded": "<reason>"}`, each rule's on its own. The corpus pins (corpus-tasks.json, a corpus's paths and exclude)
name none of it: a golden whose file changed has a new key, so importing goldens or moving a corpus asks upstream only
about the new files. A verdict is named by the source's sha256, file type and script alone, so a case source and a
golden with the same text share it.

`npm run port-readiness:record -- --pack <id> --tool <tool>` (conformance/recorders/port-readiness/) writes the
recording, asking upstream only for what the locked archive lacks at this branch's config, and nothing when it lacks
nothing: the whole-tool facts when they are missing; the rules the pack names without a complete record (Bandit, Ruff
and ESLint record a rule alone; any other adapter, whose cases or answers depend on the rules recorded with it, records
every rule again once they change); and the corpus answers no file holds. `--corpus` records only the corpus answers
(and refuses when the archive holds nothing at this config); `--full` asks everything again (an incremental record must
equal it); `--prune` drops what no recorded rule and no golden names, which is otherwise kept; `--check` asks
everything afresh and holds the lock when its archive is what a record of it makes (the fresh tree, as `--full` and
`--prune` leave it, or, with `--base <lock>`, the fresh files over that lock's corpus answers, as an incremental record
on it leaves them) or holds each fresh file identically, and otherwise says to record with `--full`, since an
incremental record never asks again about an answer the archive holds. A
tool-pack check that fails fails every rule of the tool-pack: a shipped rule blocks, and a candidate is not ready to
promote. The recording is stale when pins
disagree between `gates.yaml`, `recording.lock`, the recording archive and `conformance.json`, a runtime's pins
(`package.json`, `pyproject.toml`, `.python-version`, `composer.json`, `.php-version`, PHP's `extensions.txt`,
`rust-toolchain.toml`) differ from the ones the lock records, the kept licence (every `LICENSE` or `LICENCE` file under
`licenses/<project>/`, in any letter case and with any extension, read in name order) is not the recorded one, or a rule
is not recorded (the recording checks). `corpus-answers` fails when the archive holds the recording at another config
than this branch's, or lacks an answer for a golden's file and a recorded rule, or the parsers' verdict on it; its
detail names how many, the first, and
`npm run port-readiness:record -- --pack <id> --tool <tool> --corpus`. `corpus-complete` fails when a task or corpus a
golden names was not fetched (a download or git fetch failed); those results are not cached. `bundled-load` loads every
document in the pack's `rules/` together through rerail's bundled-rule loader, on every run, so a conflict between rules
(a duplicate id) fails it.

Offline, `not-overfit` runs regexes over the checkout's case sources only, not every upstream test file; test words stay
complete. The report's method text says so.

## Packs that are not gated

`pending.json` lists the ESLint-family tool-packs not gated yet. It is empty now that every one is recorded; a
tool-pack listed there is still validated in nera-rule-validation until its recordings land, is reported as pending
and is not frozen.

`ungated-rules.json` freezes the shipped rules of every pack that has no gated or pending tool-pack (complexity).
It was derived once, from main `d81f2749`, as the sorted `rules[].id` of
each of those packs' `pack.json`; php and blade left it when main moved all their rules to `candidates/`, node when its
22 rules moved there too, and sql, rust and vue shipped none before their tool-packs were gated. Gating is
decided per (pack, tool), by each shipped rule's
`conformance.json` entry: a rule of a tool with a `gates.yaml` is judged, a rule of a pending tool is pending, and any
other shipped rule, in a gated pack too, must be on the frozen list. A shipped rule that must be and is not listed fails
("keep it in candidates/ until the tool has a gates.yaml"); a listed rule no longer shipped, or whose tool is gated or
pending now, fails ("remove it from the list"). With
`--base <sha>`, both files may only shrink against the base commit's (a base without the file skips the guard; a base
that cannot be fetched or read fails closed).
