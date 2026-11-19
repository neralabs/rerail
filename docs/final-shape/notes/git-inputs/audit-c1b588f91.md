# Git scan audit at `c1b588f91`

The audit used the native Node addon, the headless scan entry point, and the CLI argument parser on macOS. The Git comparison used Git 2.50.1. Each numbered case below produced the stated result. The proposed repairs give the expected behavior for a regression test.

## Findings and source identity

| Case | Fixture and observed result | Proposed repair |
| --- | --- | --- |
| 1. Checkout with a trailing space | Two sibling checkouts had names that differed by one trailing space. A scan of the second checkout reported the first checkout's path and commit. It completed with zero findings. | Read the checkout path from gix, or remove only Git's output terminator. Check the selected commit with both siblings present. |
| 2. Path list with a leading space | A committed file had the name ` lead.py`. With `--fail-on-violations`, its literal `--path` exited 3. The same name in `--paths-from` exited 0. | Preserve each path between line delimiters. Check the selected paths, findings, and exit code through both interfaces. |
| 3. Two entries at one path | A checksum-valid tree contained two different blobs named `same.py`. One contained the violation. The directory API returned both names. The scan completed with zero findings. | Check for duplicate names during tree decode. Report the tree ID and path before storing or reading the ambiguous placement. |
| 4. Malformed selected commit | A checksum-valid commit had a tree header and lacked author and committer headers. Its scan completed with two findings. A rule about the same commit received `git_object_error(..., "malformed")`. | Use the same commit decoder for source selection and metadata. Fail source selection before publishing an invalid commit. |
| 5. Lost findings from another policy | A Python calls policy found two files on its own. A second policy had an unknown `.references` setting. The combined report failed with zero findings in either policy order. | Return an outcome per policy. Keep findings from successful policies and mark the scan failed for the policy with the invalid setting. |

The checkout and path cases enter through the [scan runner](../../../../src/cli/headless-scan.ts) and the [argument parser](../../../../src/cli/audit-cli.ts). The other cases enter through the [snapshot decoder](../../../../native/src/git_snapshot.rs) and the [native policy runner](../../../../native/src/lib.rs).

## Git facts and revision expressions

| Case | Fixture and observed result | Proposed repair |
| --- | --- | --- |
| 6. Concurrent repository predicates | Two repositories differed on whether Git tracked `marked`. Separate runs returned 0 and 1. Seven of twelve overlapping runs returned the wrong value or no row. | Give each evaluation its own repository, index, answers, and observations. Pass that context to its workers and nested phases. |
| 7. Split-index invalidation | A warm rule still returned four tracking answers after the shared index became unavailable. A fresh run returned no answers. The main index file stayed in place. | Observe every file that supplies the effective index. Check its availability before restoring the answers. |
| 8. Literal backslash in a filename | Git tracked a Unix file named `folder\file.py`. Its tree fact preserved the name. The rule `@git_tracked(p)` returned 0 for that fact. | Convert separators only on platforms that use them. Compare index paths as Git path bytes. |
| 9. Directory requirement in a relative selector | Git rejected `HEAD:./a.py/` for a regular file. The rule relation returned the blob ID. | Preserve the final directory requirement during path normalization. Apply it on lookup of the final tree entry. |
| 10. Invalid revision accepted | With no corresponding ref, Git rejected `z-g843173c`. The suffix was an abbreviated tree ID. Rerail selected that tree. | Restrict describe-style fallback to forms Git accepts. Compare plain refs, valid describe output, and the invalid form against Git. |
| 11. Unreadable detached HEAD | A branch held a commit with message `beta`. Detached HEAD named an absent object. Git resolved `:/beta` through the branch. Rerail failed the search. | Handle HEAD like the other starting refs. Keep its read error and return the matching commit from readable history. |
| 12. Directory with a non-UTF-8 name | A valid tree contained a directory with a Latin-1 name. The walk reported the encoding error. The reachable tree IDs contained only the root. | Derive child tree IDs from both entry-name relations. Keep the path encoding error for consumers that require text paths. |

The [legacy predicates](../../../../native/src/code_rule_dsl/logic/strings/git.rs) own the index observations. The [revision resolver](../../../../native/src/git_snapshot_revision.rs) processes selectors. The public [Git library](../../../rule-syntax/lib/git.rr) derives tree membership.

## Scope ownership and invalidation

| Case | Fixture and observed result | Proposed repair |
| --- | --- | --- |
| 13. Older directory snapshot wins | Two runs acquired the same stop generation. The newer run published snapshot 2. The older run then replaced it with snapshot 1. | Give each run a monotonic acquisition sequence. Use the sequence for snapshot ordering and the generation for stop eligibility. |
| 14. Two runs share pending ownership | Two native runs used different scope names and the same caller-supplied run ID. Both staged manifests under one key. Aborting the first removed both. | Allocate a unique staging key inside the store. If the API accepts caller-supplied keys, reject reuse of an active key. |
| 15. Reused view keeps the first owner | Two successive native runs used the same source view, scope, and store. Each ran a different policy. After both finished, the second manifest still had the first run's pending key. | Pass the owner with each evaluation. Keep the reusable Git source independent of the run transaction. |
| 16. Unreadable legacy store appears empty | The ownership query returned true for a store with a directory owner. After access was removed, a new process returned false. Restoring access returned true. | Propagate SQLite open failures to the deferred probe. Remove the probe after a successful query proves that the store has no matching ownership. |
| 17. Repaired directory retains a failure | A scan reported a missing subtree. After repair, the same view listed the directory and still reported it missing. Narrowing the selection also exposed the old failure until another inventory ran. | Associate failures with the selection and tree observation. Retract a failure after a successful read of that tree. |
| 18. Language registry silently ignores an update | The same view accepted two registries for `.py`. After the second call, a rule still returned Python. A fresh view returned Rust with the second registry. | Replace the registry and invalidate its dependent facts. Check the classification against a fresh view. |

The [scope transactions](../../../../native/src/content_store/analysis_scope.rs) publish ownership. The [release worker](../../../../src/engine/native/analysis-scope.ts) consumes the probe result. The [source view](../../../../native/src/git_snapshot.rs) holds the owner, registry, inventory, and failures.

## Cache reuse

| Case | Fixture and observed result | Proposed repair |
| --- | --- | --- |
| 19. Multi-pack index invalidates unrelated trees | A cold scan decoded five trees. A warm scan decoded zero. Adding an unrelated pack still gave zero. Rewriting the multi-pack index then caused five decodes. | Observe the lookup result and supplying pack for each tree. Repeat the measurement after a pack addition and after a change to the supplying pack. |
| 20. Irrelevant decorator setting invalidates files | Three Python files contained no matching decorator. The warm scan rebuilt zero binding contributions. Adding a sibling-link rule for an unused decorator rebuilt all three. | Key each contribution by the decorator evidence it consumes. Test an unused decorator and a matching decorator that changes the result. |

Both profiles used the same source commit and named scope across processes. The [storage observer](../../../../native/src/git_snapshot_storage.rs) computes object stamps. The [reference provider](../../../../native/src/code_rule_dsl/logic/plan/corpus/references.rs) supplies settings for binding contributions.

## Reproduction details

For case 6, both repositories ran this policy through `scanRrPoliciesAsync` with an in-memory store.

```rr
#include "git.rr"
.policy "tracked-marker" revision 1
.violation Marker(at: span, tracked: number) "{tracked}"
Marker(@place("git", 1), v) :- git_root_repo(_), v = @git_tracked("marked").
```

Only the second repository had a staged file named `marked`. The concurrent fixture issued four calls per batch with roots A, B, A, B. Each call used its own output path. The expected findings for each batch were 0, 1, 0, 1.

For case 7, `git update-index --split-index` created the split index. The fixture moved the file named by `git rev-parse --shared-index-path` after the first scan. Both comparisons used the same on-disk Git state.

For case 13, two calls to `enableStorageAsync` used the same directory, store, home, and generation. A new commit separated the calls. Each workspace called `retainAnalysisScopeAsync`. The runs called `finish(true)` in reverse acquisition order. The following query showed the snapshot ID after each finish.

```sql
SELECT workspace, snapshot_id, acquired_at
FROM analysis_scope_workspace_snapshot
WHERE scope GLOB 'cwd:*'
```

For cases 19 and 20, each fresh process wrote a separate `RERAIL_PROFILE` directory. The counters were `git.tree_decodes` and `reference.binding_contributions`. Each report still contained the same reference finding.

## Other implementation and interface gaps

Subsequent measurements used two branches and HEAD at the same tip.
A search for the root commit took four verified object reads in a four-commit history.
It took 64 reads in a 64-commit history. Both searches returned the same root ID as Git.

- The legacy tracking predicate opens the index and hashes its pathname in a separate read. An atomic index replacement between these reads can pair old answers with a new digest. Derive the observation from the index that supplied the answers. This race follows from the code. The concurrency counts above concern case 6. [Index reader](../../../../native/src/code_rule_dsl/logic/strings/git.rs)
- The message search reads and checks each visited commit to enqueue it. It reads and checks it again to match the message. Reuse the verified commit metadata for both operations. Count object reads in a search that reaches the root of a long history. [Commit search](../../../../native/src/git_snapshot_revision.rs)
- Unix and Windows compile Git message patterns with different regular expression engines. The Windows implementation accepts Rust regex syntax. Use a common Git-compatible pattern contract and run the same comparison with Git on both platforms. The audit ran on macOS. [Message patterns](../../../../native/src/git_snapshot_message_pattern.rs)
- Storage validation stats every watched pack and index on each repository check. Object lookup also probes every open index. Measure the cost with many packs and unchanged paths. Share validation across a batch with checks at its boundaries. [Storage observer](../../../../native/src/git_snapshot_storage.rs)
- Directory release receipts accumulate. The owner check searches them by scope and cutoff. The table's only index uses the receipt ID. Index the eligibility query and compact acknowledged receipts with a safe generation cutoff. Keep the ability to reject late publishers and duplicate deliveries. [Owner check](../../../../native/src/content_store/analysis_scope.rs) and [store schema](../../../../native/src/content_store/schema.rs)
- Each inventory constructs and parses the embedded inventory program. The root and source checks also parse their embedded programs. Reuse compiled programs by library bytes and engine version. The warm profile still counted two source-tape parses. The counter does not identify their inputs. Trace those inputs before changing source-tape invalidation. [Git provider](../../../../native/src/code_rule_dsl/logic/plan/corpus/git.rs)
- Inventory recursion calls Rust to split and extend a colon-delimited list of ancestor IDs for each directory placement. This duplicates tree-walk logic in the public library. Derive the bound for the filtered walk in rules. Preserve pruning of excluded subtrees and diagnostics for unreadable trees. [Inventory rules](../../../../native/src/code_rule_dsl/typing/programs/git-inventory.rr) and [host implementation](../../../../native/src/code_rule_dsl/logic/plan/corpus.rs)
- The CLI documents checkout-root semantics for relative revision expressions. From `pkg/`, Git and the native API resolved `HEAD:./inside`. The CLI looked for `inside` at the checkout root. I propose using the invocation directory for Git expressions and keeping report paths relative to the selected tree. This changes a documented interface. [Scan options](../../../headless-scan.md)
- The CLI deliberately skips absent paths. A scan of `absent.py` reported success and an empty path list. Preserve requested and skipped paths in the report so a driver can distinguish this result from an intentional empty selection. [Scan target](../../../../src/cli/headless-scan.ts)
- The headless runner still creates a scratch directory for audit configuration and rule output. Git source reads use the object database. To remove the scratch directory from headless scans, use an in-memory configuration and return diagnostic rows directly. [Scan runner](../../../../src/cli/headless-scan.ts)

Fix cases 1–8 and 13–16 first. These cases can select the wrong source, suppress findings, reuse stale answers, or transfer cache ownership. The acceptance checks above exercise both rule answers and retained ownership where applicable.

## Repairs checked after the audit

| Cases | Commit | Evidence |
| --- | --- | --- |
| 1–2 | `a2d8c22f2` | The report identifies the checkout with the trailing space. Both path interfaces preserve the leading space and report the violation. |
| 3–4, 9–12 | `6bfa331f2` | The scan rejects duplicate entries and malformed commits. Selector tests check Git's answer. A byte-name child remains reachable by ID. |
| 5 | `0a770a110`, `65a745ab2` | Headless and standalone reports keep the successful policy's findings in both orders. [The standalone working-tree ownership fixture](audit-ec9066ed9.md#working-tree-batch-publication-case-58) checks a separate effect of the failed batch. |
| 6–8 | `68ccbe8ac`, `be7e18211` | Overlapping runs keep opposite tracking answers. A small worker pool could still stall. Four Linux runs now return with one, two, three, or four workers. Eight runs on two workers returned the expected forty answers over five batches. Removing and restoring a split index changes warm answers. A literal Unix backslash survives lookup. |
| 13–14, 16 | `a91b1bf7f` | Reverse completion preserves the newer directory snapshot. Aborting one run preserves the other run's rows. A failed ownership query reaches the release worker. |
| 15, 17–18 | `1150e5031` | Two runs share a source view and publish separate manifests. The view retracts a repaired subtree error and applies a replacement language registry. |
| 19 | `b19722b7f` | An unrelated pack or a rewritten multi-pack index causes zero tree decodes. Damaging and restoring the supplying pack changes the answer. |
| 20 | `5fb92bb6e` | A cold scan builds three binding contributions. An unused decorator change builds zero. A matching change builds one and produces the sibling finding. |

Later checks found further problems in policy inspection, worker store selection, scope publication, and release probes.
The [scope and cache notes](analysis-scopes.md) record the repair behavior and the fresh-process measurements.
The [headless regressions](../../../../test/cli/headless-scan.test.ts) exercise the scan interface.
The [scope store tests](../../../../native/src/content_store/corpus_runs/tests.rs) inspect durable ownership after publication, cancellation, and deletion.
The [follow-up audit at `ec9066ed9`](audit-ec9066ed9.md) records additional fixtures and open issues.

### Checks for the other gaps in the original audit

| Requirement | Implementation and evidence |
| --- | --- |
| Observe the effective index that supplied the answers | `68ccbe8ac` hashes the sorted tracking paths from the same decoded index it queries. The split-index fixture changes shared-index availability between warm reads. |
| Read each searched commit once | `3fb6e46ba` keeps the verified commit in the search queue and deduplicates tips before lookup. The four-commit and 64-commit searches made four and 64 verified reads, respectively. |
| Limit checks of unchanged pack layouts | `eaaed594f` shares layout validation across workers. `8a328d0a0` limits unsuccessful lookups in larger indexes. The [measurements](analysis-scopes.md) include probe counts, decode counts and memory cost. |
| Retain the compiled inventory, root and source checks | `6af0a46a1` first reused the embedded programs in one process. `abfab008a` stores them with each consuming scope. Five fresh processes checked reuse and reclamation after the last owner left. `6e3f1618e` retains working-tree policies and their include graphs. Opt-in Git-backed policies still have the separate cost in case 52. [Measurements](analysis-scopes.md) |
| Bound the filtered tree walk in rules | `ae7b6388d` derives candidate ancestry in [git-inventory.rr](../../../../native/src/code_rule_dsl/typing/programs/git-inventory.rr). The filtered walk checks repeated tree IDs along each placement and prunes excluded children before opening them. |
| Index and compact release receipts | `a91b1bf7f` retains the cutoff necessary to reject late publishers and compacts superseded numbered receipts. After four deliveries for a parent and child, the test checks both cutoffs and finds two receipt rows. |
| Resolve relative Git expressions from the invocation directory | The CLI and native fixtures compare a subdirectory's `HEAD:./` with Git. A scan reports requested and skipped paths. The path-list and direct argument tests preserve leading spaces and dashes. |
| Run headless without an extracted source or a scratch output directory | `a42954648` put the headless configuration and diagnostics in memory. Two nested evaluators still wrote output directories. `ba58532ee` put their output in memory too. The regression preserves pre-existing files under both directory names and checks the rule's answer. |
| Compare message selectors with Git on Windows | The package job runs the native selector comparison on x64 Windows. The macOS result and a Windows-target typecheck do not supply the missing Windows runtime result. |
