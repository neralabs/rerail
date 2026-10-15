# Rule gates

The rule gates judge every rule document of a gated tool-pack (one upstream tool in one pack: `python/ruff`,
`typescript/typescript-eslint`) against the upstream linter it ports, from recordings committed in the pack. They are a layer on top
of the pack contract (`test/engine/catalog/packs.test.ts`) and the conformance check, and a superset of the check: its
verdict is the `passes-conformance` gate (checks/passes-conformance.ts), for candidates as well as shipped rules.
Documents under `packs/<pack>/rules/` ship, and a failing one fails CI. Documents under `packs/<pack>/candidates/**`
(except `candidates/probes/**`) go through the same gates and are only reported.

| V1 gate | Gate id | Blocking |
| --- | --- | --- |
| 1 | `upstream-status` (decides on or off) | no |
| 2 | `loads` | yes |
| 3 | `not-overfit` | yes |
| 4 | `compact` | yes |
| 5 | `matches-upstream-tests` | yes |
| 6 | `matches-real-code` (its disagreements become goldens of `matches-upstream-tests`) | no |
| — | `passes-conformance` (the pack's `test:conformance` verdict on the rule, judged after V1's gates; never cached) | yes |

A rule's verdict is the first blocking gate it fails. `matches-upstream-tests` includes `engine-reads-goldens`: a golden
that differs where the engine reported `source-partial`, `source-incomplete` or `budget-exceeded` fails the rule; the
fix belongs in the engine (NER-1764).

## Running

```sh
npm run gates                                   # every gated tool-pack, reports in .rerail/reports/gates/
npm run gates -- --pack python --tool bandit    # one tool-pack
npm run gates -- --no-cache                     # judge every rule again; read and save no results
npm run gates -- --refresh-cache                # judge every rule again and save the fresh results (the nightly)
npm run gates -- --write                        # also write this run's golden promotions to goldens.tsv
npm run gates -- --pack typescript --tool typescript-eslint --shard 2/6   # one shard: rules with sha256(id) mod 6 = 1
npm run gates -- merge .rerail/reports/gates    # join each tool-pack's shard-<i>of<n>/ reports (shard.ts, merge.ts)
npm run gates -- summary .rerail/reports/gates --base <sha>
```

Exit 1 when a shipped rule fails (or the summary has a blocking failure), 2 on an exception. The run needs the built
native engine; it reads no upstream runtime. It fetches only the pinned corpora (catalog task files and the pack's
`conformance.json` corpora), cached under `.rerail/gates/corpus/`.

Per-rule results are cached under `.rerail/gates/results/<pack>/<tool>/<key>.json`, a shard's under
`<tool>/shard-<i>of<n>/` (`--cache <dir>` moves the root).
The key covers the gate code and its dependencies (`package.json`, `package-lock.json`), the engine (tracked engine
sources and the native binary), the tool-pack's `gates.yaml` and the two recording digests its `recording.lock` names, the corpus pins, and
the rule's document, conformance entry, goldens and case sources. When every rule hits, nothing is fetched or
evaluated. An entry that is not JSON, was written by another cache layout (`CACHE_FORMAT`), lacks the shape of a result,
or names another document is a miss. In CI, pull request runs only read the cache; pushes to main and the nightly save
it, so nothing a pull request's code computes is replayed later.

## Data

```
packs/<pack>/upstream/<tool>/gates.yaml         the tool-pack's manifest (manifest.ts)
packs/<pack>/upstream/<tool>/recording.lock     which recording this commit is judged against (recording/store.ts)
packs/<pack>/upstream/<tool>/goldens.tsv        goldens of matches-upstream-tests, by document id

.rerail/gates/recordings/<sha256>.json          the recordings themselves, fetched or recorded, cached by digest
  gates.json                                    upstream's recorded answers (recording/schema.ts, rerail.gates-recording/v2)
  gates-corpus.json                             upstream's findings on the corpus (rerail.gates-corpus/v2)
```

The recordings are not in the tree: the lock names them and a GitHub release holds them, so `npm run gates -- fetch
--pack <id> --tool <tool>` must have run (or `gates:record`, which writes them straight into the cache). A download
whose sha256 is not the one the lock names fails the run.

`npm run gates:record -- --pack <id> --tool <tool>` (conformance/recorders/gates/) writes the recordings. A tool-pack
check that fails fails every rule of the tool-pack: a shipped rule blocks, and a candidate is not ready to promote. The
recording is stale when pins disagree between `gates.yaml`, `recording.lock`, `gates.json` and `conformance.json`, a
runtime's pins (`package.json`, `pyproject.toml`, `.python-version`) differ from the ones the lock records, the kept
LICENSE is not the recorded one, or a rule is not recorded (`corpus-pins` and the recording checks). `corpus-complete` fails when a file `gates-corpus.json` answers for was not fetched with its
recorded sha256 (a download or git fetch failed); those results are not cached. `bundled-load` loads every document in
the pack's `rules/` together through rerail's bundled-rule loader, on every run, so a conflict between rules (a
duplicate id) fails it.

Offline, `not-overfit` runs regexes over the committed case sources only, not every upstream test file; test words stay
complete. The report's method text says so.

## Packs that are not gated

`pending.json` lists the ESLint-family tool-packs not gated yet. It is empty now that every one is recorded; a
tool-pack listed there is still validated in nera-rule-validation until its recordings land, is reported as pending
and is not frozen.

`ungated-rules.json` freezes the shipped rules of every pack that has no gated or pending tool-pack (complexity).
It was derived once, on branch `ggiangi/ner-1699-rule-gates-in-ci` from main `d81f2749`, as the sorted `rules[].id` of
each of those packs' `pack.json`; php and blade left it when main moved all their rules to `candidates/`, node when its
22 rules moved there too (NER-1897), and sql and vue
ship none. Gating is decided per (pack, tool), by each shipped rule's
`conformance.json` entry: a rule of a tool with a `gates.yaml` is judged, a rule of a pending tool is pending, and any
other shipped rule, in a gated pack too, must be on the frozen list. A shipped rule that must be and is not listed fails
("keep it in candidates/ until the tool has a gates.yaml"); a listed rule no longer shipped, or whose tool is gated or
pending now, fails ("remove it from the list"). With
`--base <sha>`, both files may only shrink against the base commit's (a base without the file skips the guard; a base
that cannot be fetched or read fails closed).
