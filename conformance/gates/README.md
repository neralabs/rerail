# Rule gates

The rule gates judge every rule document of a gated tool-pack (one upstream tool in one pack: `python/ruff`,
`typescript/typescript-eslint`) against the upstream linter it ports, from recordings committed in the pack. They are a layer on top
of the pack contract (`test/engine/catalog/packs.test.ts`) and the conformance check; they do not replace either.
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
sources and the native binary), the tool-pack's `gates.yaml`, `gates.json` and `gates-corpus.json`, the corpus pins, and
the rule's document, conformance entry, goldens and case sources. When every rule hits, nothing is fetched or
evaluated. An entry that is not JSON, was written by another cache layout (`CACHE_FORMAT`), lacks the shape of a result,
or names another document is a miss. In CI, pull request runs only read the cache; pushes to main and the nightly save
it, so nothing a pull request's code computes is replayed later.

## Data

```
packs/<pack>/upstream/<tool>/gates.yaml         the tool-pack's manifest (manifest.ts)
packs/<pack>/upstream/<tool>/gates.json         upstream's recorded answers (recording.ts, rerail.gates-recording/v1)
packs/<pack>/upstream/<tool>/gates-corpus.json  upstream's findings on the corpus (rerail.gates-corpus/v1)
packs/<pack>/upstream/<tool>/goldens.tsv        goldens of matches-upstream-tests, by document id
```

`npm run gates:record -- --pack <id> --tool <tool>` (conformance/recorders/gates/) writes the recordings. A tool-pack
check that fails (pins disagree between `gates.yaml`, `gates.json` and `conformance.json`; a runtime lockfile differs
from the committed one; the kept LICENSE is not the recorded one; a rule is not recorded) fails every shipped rule of the
tool-pack: the recording is stale.

Offline, `not-overfit` runs regexes over the committed case sources only, not every upstream test file; test words stay
complete. The report's method text says so.

## Packs that are not gated

`pending.json` lists the ESLint-family tool-packs not gated yet (next/eslint-plugin-next): they are still validated in
nera-rule-validation until their recordings land, are reported as pending and are not frozen.

`ungated-rules.json` freezes the shipped rules of every pack that has no gated or pending tool-pack (blade, complexity,
node, php). It was derived once, on branch `ggiangi/ner-1699-rule-gates-in-ci` from main `d81f2749`, as the sorted
`rules[].id` of each of those packs' `pack.json`. A shipped rule there that is not listed fails ("keep it in
candidates/ until the pack has an adapter"); a listed rule no longer shipped fails ("remove it from the list"). With
`--base <sha>`, both files may only shrink against the base commit's (a base without the file skips the guard; a base
that cannot be fetched or read fails closed).
