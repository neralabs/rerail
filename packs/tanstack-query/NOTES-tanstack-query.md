# Measurement notes: tanstack-query

## How the rules were measured

Tuned on a 33-app corpus, generalized against 8 held-out apps, then measured on frozen sets of 20, 37 and 41 apps.
An app becomes tuning data for a rule once its findings inform a fix. Enabled rules require observed precision above
90% on applicable unseen apps. Promotion also requires five judged matches in two apps. Small and concentrated
samples are not statistical guarantees. Historical migration status is not inferred from repository contents.

## Corpus runs

The corpus is tuning evidence, not acceptance evidence. App names, commits, source paths, individual verdicts and
retracted findings are preserved privately for coordinated disclosure. They are intentionally absent from public
documentation. Some runs hit a 5 GB memory or 50-minute guard; missing runs are not zero findings.

## Matches read

Independent readers judged the applicable findings. Counts below distinguish judged samples from total findings.
Private records retain the per-finding evidence; no live exploitation or upstream remediation was established.

## Dropped candidates

Unmeasured rules and rules below the acceptance bar remain outside pack.json in candidates/, with their fixtures.
A candidate's passing fixtures establish its documented shapes, not its precision on unfamiliar applications.

## Library candidates

Reuse the engine's existing libraries. Cross-file facts must carry stable document paths and symbols, not node IDs.

## Decisions after test set #3

| Rule | Unseen real / judged | State |
|---|---:|---|
| query-data-copied-to-state | 6/6 | on, steer |
| fetch-without-ok-check | 6/6 | on, log |
| server-action-as-query-fn | 6/6, three apps | on, steer |
| same-key-different-query-fn | no frozen-set match after fixes | candidate |
| cache-key-matches-no-query | 2/7 | candidate |
| set-query-data-mutates-cache | 1/1 | candidate, sparse |

## Test sets #2 and #3

The Server Action rule's six real findings came from set #2. The unchecked-fetch rule gained one real finding on
set #3. Five false cache-key findings belonged to an abandoned hook with no callers. A cache-mutation finding on
set #3 was real but provides too little evidence for promotion. Other candidates remain unmeasured.

## Open fixes

An unused hook can look like a broken invalidation path. Query-factory identity and hydration-prefetch exemptions
are bounded approximations. Cross-dialect AST visibility and helper resolution remain documented engine limits.

## Fixes after the first test set

Conflicting-query-function checking recognizes library key factories, factory identity and hydration prefetches.
The fixed rule remains a candidate: its remaining held-out matches were false, and those apps informed its tuning.
