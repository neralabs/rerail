# Measurement notes: next

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
| redirect-in-try | 8/8, two apps | on |
| revalidate-unknown-target | 19/19, two apps | on |
| server-action-without-auth | 8/13 after fixes | off |
| middleware-not-applied | 1/2 | off |
| revalidate-unknown-tag | 25/31 | candidate |
| public-env-secret | 0/2 | candidate |
| server-action-unvalidated-input | 1/1 after fixes | candidate, sparse |
| unstable-cache-closure-key | no unseen match after fixes | candidate |

## Open fixes

Auth detection still reports deliberate anonymous writes, public registration and bearer-token-authorized actions.
A legacy middleware can coexist with a correctly located replacement proxy. Tag assignments in another ECMAScript
dialect can be invisible. Unused deployment variables can resemble publicly exposed secrets. Cache-wrapper
exclusions are bounded heuristics, not proofs of request invariance.

## Fixes after the first test set

Auth helpers in another dialect are read as text; auth-result dataflow and counter-only Prisma writes are recognized.
Unchecked-filter analysis recognizes schema unique keys, later unique writes, transaction siblings and strict guards.
Closure analysis recognizes forwarding wrappers; an environment-fallback exemption was rejected because fallback
syntax does not prove a captured value irrelevant to the cache key. These fixes did not justify enabling the auth rule.

## Generalization

Framework-dependency gates and import resolution replace app-specific names. Engine limitations and bounded helper
resolution remain documented in ENGINE-GAPS.md and each rule's header.
