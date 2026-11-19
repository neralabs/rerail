# Git and scope audit at `ec9066ed9`

The original [twenty cases and their regression tests](audit-c1b588f91.md) cover specific inputs. This audit checks additional inputs, public entry points, and the collector's choice of owners. The experiments ran on macOS.

## Status at `8a490c35e`

Ten cases from this follow-up audit have regression coverage.

| Cases | Commit | Check |
| --- | --- | --- |
| 1 | `ef0b83be5` | Separate headless processes render the current policy wording. A message edit and revert work with the same filesystem or revision store. |
| 2 | `8690c224b` | Under collection pressure, the latest successful directory acquisition keeps its manifest. The tests cover abort, reverse completion, cross-scope reuse, and schema migration. |
| 3 | `8a490c35e` | The directory owner count goes from one to zero on stop and returns to one after a control reset. The regression also covers a rolled-back stop, recovery from an ancestor receipt and a stop after restoring an older control backup. |
| 5–8 | `ecef2c02e` | Selection and rules reject the tested malformed commit, bad tag target kind and unsorted tree. Message search finds the matching readable branch past a missing parent. Case 26 below covers other object links. |
| 9–10 | `12ea6378e` | An invalid UTF-8 list line fails before selection. A Git diff keeps the valid replacement-character name and reports the distinct byte name separately. |
| 20 | `bc726bc5f` | A collector failure appears in the JSON report alongside the successful scan and its findings. Case 30 covers the text formatter. |

Cases 4, 11–19, 21 and 22 remain open. The eight fixtures below bring the open count to 20.
Cases 12, 15–19 concern cost or operational behavior. Case 13 needs a rule visibility contract.
Cases 21 and 22 need Windows verification. The tested inputs and proposed checks for each appear in the tables.

The rebuilt native addon passed all 129 tests across the headless and Git helper suites.
The store suite passed 254 tests with four excluded by its filter. The TypeScript build typecheck passed.
An earlier run also hit `ENOTEMPTY` during test store cleanup. The clean rerun does not establish the cause of that intermittent teardown failure.

## Further observations at `8a490c35e`

| # | Observed result | Repair and acceptance check |
| --- | --- | --- |
| 23 | A directory had three manifest memberships. Its stop committed one queued release. Recreating the home before delivery dropped the queue. After settling releases, all three memberships remained. Generation recovery in case 3 lets new runs publish but cannot reconstruct the missing stop. | Persist committed stop intents with their routes and cutoffs in recoverable storage. Define what backup and restore must preserve. Kill a worker on either side of commit and restore an older home with both delivered and undelivered stops. Only committed stops may take effect. |
| 24 | Delivering `invalid:receipt:<home>` through the native API removed all three directory memberships. A failed integer parse selected the unconditional release branch. | Parse numbered receipts before opening a writer. Reject malformed cutoffs and preserve the explicitly supported legacy receipt form. Assert the owner and receipt tables remain identical on error. |
| 25 | The native `AnalysisScopeRun` constructor accepted JavaScript `NaN`, `Infinity`, and `1.9` for the acquisition generation. The binding casts each input to an integer. | Require a finite, nonnegative safe integer at every public entry point that accepts a generation. Check rejection before staging. Include zero and the largest supported token as valid controls. |
| 26 | Each of three malformed selectors opened the original tree. A commit's tree header referenced a commit. A parent header referenced a tree. A regular-file entry referenced a tree. The selectors were `^{tree}`, `^1`, and `:fake`, respectively. | Check the immediate target kind required by each edge before another peel or source selection. Preserve Git's special handling of gitlinks. Exercise the source constructor and the rule API with valid and invalid links. |
| 27 | For a tracked regular file, the filesystem scan listed `a.py/` in `skippedPaths`. The revision scan accepted it, and the native view's `containsPath("a.py/")` returned true. | Preserve the directory requirement for explicit scan paths as well as for Git revision expressions. Compare both providers on files, directories and the selected root. |
| 28 | With a tracked blob moved offline, the native `readSource("a.py")` returned null, and `sourceFailures()` was empty. The same view's `readFile("a.py", 1024)` reported the missing object. | Give native callers a checked read with distinct outcomes for absence, size limit and an unavailable object. Migrate callers that need diagnostics and document the legacy nullable shorthand. Test removal, restoration and oversize. |
| 29 | A scan under named scope `fixture` returned the revision, tree and commit. Its only scope field contained `repository`. The saved report could not identify the retention scope. | Add analysis scope kind and name to the report and its schema. Retain the existing path scope field. Test two named runs of the same revision and a directory run. Decide which local identifiers belong in a report sent to a remote. |
| 30 | With an injected collector failure, the text formatter printed “Cache was not retained.” SQLite still held one manifest for the named scope. The JSON report preserved the collector error. | Distinguish a publication failure from a collection failure when formatting the report. Test both messages against the actual memberships after the scan. |

Cases 23–25 use the [control queue](../../../../src/engine/native/analysis-scope.ts), the [native run constructor](../../../../native/src/code_rule_dsl/workspace/api.rs), and the [release transaction](../../../../native/src/content_store/analysis_scope.rs).
Cases 26–28 use [revision traversal](../../../../native/src/git_snapshot_revision.rs) and the [source view](../../../../native/src/git_snapshot.rs).
Cases 29–30 use the [headless report and formatter](../../../../src/cli/headless-scan.ts).

## Results from fixtures

| # | Inputs and observed result | Repair and acceptance check |
| --- | --- | --- |
| 1 | Change only the literal message in a `.violation` from `first` to `second`. In separate headless processes, both a revision scan and a filesystem scan returned `first` with the warm store. Both returned `second` with a fresh store. A separate edit to the diagnostic code worked. | Render saved finding values with the policy template inspected for this run. Test a message-only edit and revert with zero new source parses. [Message contract](../../decisions/0225-preparation-costs-the-change.md) |
| 2 | Publish directory manifests A and B for the same root and policy set. Evaluate A again and finish that run with `false`. Its existing manifest advanced from `last_run=2` to 5. The last successful manifest B stayed at 4. The collector's directory protection query now prefers A. | Record the acquisition sequence on the scope's published manifest membership. Advance that sequence only on successful publication. Under collection pressure, check the plan that survives an abort, reverse completion, and reuse in a different scope. [Collector](../../../../native/src/content_store/corpus_runs.rs) |
| 3 | Scan, stop, then recreate the home control database at the same path while keeping the content store. The first scan published one directory manifest. The stop removed it. The next scan completed without a cache warning and published zero. Its acquisition generation equaled the retained stop cutoff. | Recover a generation above prior cutoffs before accepting new runs, or give a recreated home an independent durable identity. Exercise restoration of an older control backup as well. [Generation acquisition](../../../../src/engine/native/analysis-scope.ts) |
| 4 | Move a fixture store offline and deliver a numbered directory release through the native API. It created a new store at the original path, wrote the receipt, and returned success. The offline database still had its owner. The worker checks existence before delivery. The interval between that check and open is a race. | Deliver against an existing store with a verified identity. Keep the release pending if the store vanishes or changes identity. Test removal between the check and delivery, then restore the original store. [Release entry point](../../../../native/src/lib.rs) |
| 5 | A commit with a valid tree header and no signatures failed direct source selection. Both `<commit>^{tree}` and `<commit>:dir` selected source through it. | Apply the same commit decoder at each navigation step that consumes a commit. Check direct selection, peeling, path selection, and their rule-query counterparts. [Revision navigation](../../../../native/src/git_snapshot_revision.rs) |
| 6 | An annotated tag declared its target a commit but pointed to a tree. Git's integrity check reported a bad tag pointer. Rerail followed it and opened the tree. | Decode the tag and compare its declared target kind with the acquired object's kind before following it. Test nested tags and a missing target. [Snapshot selection](../../../../native/src/git_snapshot.rs) |
| 7 | A literal Git tree contained `z` before `a`. Git's integrity check reported `treeNotSorted`. The source API sorted it and returned both entries as a successful read. | Validate Git's original entry order, including its directory comparison convention, before sorting entries for lookup. Test a valid mix of directory and file names and an invalid ordering. [Tree decoder](../../../../native/src/git_snapshot.rs) |
| 8 | A newer branch tip had a missing parent. A readable branch had the matching message. Git resolved both `:/both` and `:/base`. Rerail stopped at the missing parent for both. The existing test covers an unreadable starting ref. | Continue searching the other queued commits after an unreadable edge. Retain its error if no readable commit matches. Compare both outcomes with Git. [Message search](../../../../native/src/git_snapshot_revision.rs) |
| 9 | A `--paths-from` file contained the bytes `66 ff 2e 70 79` and a line delimiter. The repository contained the different UTF-8 name `f�.py`. The scan completed, selected that file, and reported its finding. | Decode the text list strictly before selection and report the invalid line. Test that an invalid list cannot select a valid replacement-character name. [Argument reader](../../../../src/cli/audit-cli.ts) |
| 10 | A Git index contained distinct names with `ff` and `ef bf bd` in the same position. With both staged and marked skip-worktree, Git's NUL-delimited diff returned two names. The JSON scan report contained one path. In this fixture it reported a read failure at that path. | Split Git output as bytes, then decode each name. Preserve the valid path and identify the unsupported one by its bytes. Check that deduplication preserves both identities. [Diff selection](../../../../src/cli/headless-scan.ts) |
| 11 | A repository contained a 32,000,001-byte NUL-filled `unlisted.dat` and a text file with a violation. With the same broad JSON policy, the filesystem scan completed. The revision scan reported the text violation and failed on the binary with `budget-exceeded`. | Use a Git-backed binary-prefix check with a bounded memory cost. Test a large binary, large text, and an unreadable object through both providers. [Repository index](../../../../native/src/code_rule_dsl/workspace/index.rs) |
| 12 | With 12 unrelated one-object packs, the 64-directory warm fixture went from 780 to three index probes in `ec9066ed9`. With 257 objects in each of the 12 packs, the same scan still made 780 probes. Both returned 64 findings and decoded zero trees. | Measure index construction and lookup together on representative large packs. Evaluate a shared supplier index while preserving duplicate suppliers and alternate object stores. Compare cold cost, warm cost, and memory before choosing the representation. [Storage observer](../../../../native/src/git_snapshot_storage.rs) |

## Scope, cache, and interface gaps found in the implementation

| # | Current behavior and consequence | Repair or decision |
| --- | --- | --- |
| 13 | Scope ownership exists as SQLite rows. A policy cannot join a run, its scope, and its snapshot through the public `.rr` library. | Expose read-only, explicitly keyed membership facts. Specify whether a running policy sees committed memberships, its own pending membership, or both. Test two scopes sharing a snapshot and deleting one. [Ownership schema and visibility](analysis-scopes.md) |
| 14 | Named scope deletion uses the store selected at deletion time. The CLI returns success when the name or store is absent. A change to the store override can leave the intended ownership untouched. | Report the target store and deletion outcome. Make the store explicit or register the stores used by named scopes. Preserve idempotence for repeated deletion. [Scope command](../../../../src/cli/cli.ts) |
| 15 | The store retains a generation row for each deleted named scope. Repeated use of one-time names therefore grows this table. | Retire generation rows once all older staging permits are closed. Test late writers, name reuse, restart, and bounded row counts after repeated deletion. [Generation and staging checks](../../../../native/src/content_store/analysis_scope.rs) |
| 16 | Directory-to-store registrations survive release. Each delivery batch loads the entire registered directory list and filters it in JavaScript. Old directories continue to add work to unrelated stops. | Track acquisition generations on registrations and prune acknowledged registrations when safe. Query matching directory routes in SQLite. Test a new registration concurrent with a stop and many unrelated directories. [Directory registry](../../../../src/engine/native/analysis-scope.ts) |
| 17 | A permanently inaccessible or invalid legacy candidate stays due for another probe after five seconds. Its queue row records the next time but omits the failure reason. | Persist the last error and retry count, use capped backoff, and expose pending probes. Preserve the original cutoff across repair and retry. [Probe worker](../../../../src/engine/native/analysis-scope.ts) |
| 18 | A native view retains decoded tree entries in `observed_trees` and a full saved inventory. Changing the path selection drops the saved inventory but leaves previously visited trees in the map for the life of the view. | Keep observation facts and the Datalog result as the reusable representation. Bound any decoded read buffer. Measure retained bytes while one view moves through disjoint subtrees. [View storage](../../../../native/src/git_snapshot.rs) |
| 19 | The native path-selection cache key includes the caller's ordered vector. Reordering or duplicating the same paths changes the key and clears the saved inventory. Headless CLI sorting masks this for usual CLI calls. | Canonicalize the set once at the native boundary. Test equivalent selections in a reused view and count inventory executions. Preserve the caller's original spelling separately in the report. [Selection and inventory key](../../../../native/src/git_snapshot.rs) |
| 20 | If post-scan collection fails, the headless runner writes the error to stderr. A caller consuming only the returned report cannot tell that collection failed. | Add the collection error to the returned cache warnings while preserving findings and the primary scan outcome. Test a failing collector with a successful scan. [Report finalization](../../../../src/cli/headless-scan.ts) |
| 21 | On Windows, the object stamp uses length, creation time, and modification time. An in-place, same-length rewrite with restored modification time can preserve those inputs. This audit has no observed stale Windows result. | Exercise that mutation on NTFS against both warm and cold scans. Use a reliable change signal or verify bytes when storage cannot establish freshness. [Object observation](../../../../native/src/git_snapshot_storage.rs) |
| 22 | The package workflow contains a Git selector comparison on Windows x64. This branch has no recorded Windows run of that comparison. The portable matcher was exercised on macOS. | Run the packaged addon against the installed Git for Windows, record both versions and compare resolved IDs and rejected expressions. [Package job](../../../../.github/workflows/package.yml) |

## Reproduce the stale report

Use a checkout with a commit. Save this local policy as `audit.rr`.

```rr
#include "git.rr"
.policy "same" revision 1
.violation Seen(at: span) "first"
Seen(@place("git", 1)) :- git_root_repo(_).
```

Run this command in a fresh process each time. Use one content store for the first two runs.

```sh
rerail scan --revision HEAD --analysis-scope audit --rr ./audit.rr --json --no-send
```

Change `"first"` to `"second"` before the second run. Compare its violation message with a third run using a fresh `RERAIL_CONTENT_STORE_PATH`. The same comparison works for a filesystem scan when you omit the revision and named scope.

The outstanding correctness work starts with cases 4 and 23–26. Each can change the owner set or the selected source.
