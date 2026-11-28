# Measurement notes: prisma

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
| one-client | 30/31, five apps; 26 in one app | on, log |
| request-body-as-data | 8/8, four apps | on, log |
| side-effect-in-transaction | 5/5, two apps | on, log |
| query-in-loop | 48/67 judged; 57 additional findings unjudged | off, log |
| raw-unsafe-interpolation | 0/3 after fixes | off, log |
| global-client-in-transaction | 3/3, one app | candidate, sparse |
| read-modify-write | 2/2, one app | candidate, sparse |
| sensitive-fields-to-client | 4/7 judged; two additional findings unjudged | candidate |
| find-first-without-order-by | 5/49 judged; additional findings unjudged | candidate |
| bulk-write-without-where | 0/1 | candidate |
| dev-migration-in-deploy | 0/2 | candidate |

## Test set #3

Client reuse produced one false match in an independent sidecar sharing a Next.js package. Request-data checking
produced three real matches; transaction side effects produced none. Query-in-loop had 35/48 judged findings real
and one additional unjudged finding; set #2 had 13/19 real and 56 additional unjudged findings. The combined 48/67 is
a judged subset, not a complete precision estimate. Raw SQL checking produced three false matches on set #2 and
none on set #3: the values were trusted database-derived integer IDs, including one validated by a typed lookup.

## Open fixes

Loop findings include startup restoration, independent partial-success handling, external-operation checkpoints,
fixed lists behind transformations, ordered writes, reads after locks and nested transaction-batch construction.
Secret-management UIs can intentionally return credentials supplied by their users. Unordered lookups can implement
idempotency, partial uniqueness or alternate-identifier resolution. A Next.js dependency does not make every process
in that package hot-reloaded. Raw-SQL syntax does not establish a value's provenance or runtime type.

## Fixes after the first test set

Query-in-loop recognizes offline-only imports, compare-and-set writes, sorted transactional writes and checked
per-row updates. Raw SQL recognizes generated question-mark placeholders. Neither fix met the unseen-app bar.

## The promoted rules run per file

Cross-file client and schema facts are keyed by documents and symbols. Fixtures and published-pack split checks
exercise the promoted rules. Separate re-runs on the matching apps reproduce the five request-data and five
transaction-side-effect findings from test set #2.

## Generalization round (held-out apps)

The held-out apps informed generalization, so their later findings are not independent acceptance evidence.

## Frozen test set (2026-11-26)

The first frozen set became tuning data for the two off rules after its false matches informed their fixes.
