# Git and scope audit at `ec9066ed9`

The original [twenty cases and their regression tests](audit-c1b588f91.md) cover specific inputs. This audit checks additional inputs, public entry points, and the collector's choice of owners.

Seventeen of the 28 follow-up findings have verified results. Eleven remain open. The separate Windows checks for cases 21 and 22 passed at `65e24db1d`.

## Paths that share a display label

Case 75 put two entries at the root and below `nested`. One name had the bytes `ff 2e 70 79`. The other was the text `[hex:ff2e7079]`.
The earlier headless report used the same display path for the finding and failure and provided no second key.
The new headless and native reports give the complete path with an explicit encoding. Each identity matched exactly one entry from the native directory listing.
The diff and policy-ref checks also preserved byte names. Six related tests passed together in 16 seconds with a 30-second per-test timeout.
Two earlier runs exceeded the default five seconds in different tests. [Report format](../../../headless-scan.md) · [Public regressions](../../../../test/cli/headless-scan.test.ts)

## Cost of inspecting a policy in a fresh catalog

Case 53 compared 50 warm calls to `LoadedRrPolicies.load` in each mode. Every call returned the same policy metadata and include graph.
On an M3, median wall time was 0.243 ms in one catalog and 3.434 ms with a fresh catalog per call.
A fresh catalog spent 1.508 ms of CPU per call opening the reading store and 0.956 ms reading its recorded include graph.
It spent 0.053 ms checking inputs, 0.043 ms taking the collection lease, 0.003 ms settling queued writes and 0.038 ms checking image availability.
The existing catalog spent 0.031 ms checking inputs. All 100 calls reused the process's program. Both modes decoded zero compiled programs.
These measurements retain the store and collection checks. They explain the cost for a local policy with `git.rr` and the default image format.
Run the regression with `RERAIL_PROF=1` to print the timings and check the call counts. [Catalog regression](../../../../native/tests/git_revision_cli.rs)

## Repository and worktree locations

The standalone CLI rejected `git_worktree` as undeclared before the repair.
The main checkout and two linked checkouts now report one repository ID and common directory. Each reports its own view ID and working directory.
Rules in both working-tree and revision runs returned the linked checkout's real path. A bare repository returned its common directory and zero working directories.
Renaming the bare repository preserved both IDs. A fresh invocation using the same scope and store returned its new path.
One native relation supplies the locations. The two public relations are rules in `git.rr`.
[Rule reference](../../../rule-syntax/git.md) · [Standalone CLI regression](../../../../native/tests/git_revision_cli.rs)

## Decoded directory cache

Case 18 used one revision view for successive disjoint selections. With a 1,964-byte test budget, the root and two small subtrees fit.
Revisiting a recent subtree required zero decodes. Reading a third subtree evicted the oldest leaf. Returning to that leaf required one decode.
The oversized directory returned all 128 entries on both reads. Each read required one decode and left 1,964 bytes in the cache.

With the production 8 MiB limit, a second fixture selected twelve directories with 4,000 entries each. It then read a 40,000-entry directory twice.
The native allocator reported 8.2 MiB above baseline after the twelve selections, down from 12.5 MiB with the previous addon.
After both oversized reads, the respective increases were 15.6 MiB and 29.3 MiB. These figures include native allocations outside the decoded directory cache.
[Cache contract](analysis-scopes.md) · [Snapshot regression](../../../../native/src/git_snapshot.rs)

## Skipped revision paths

For case 47, one CLI invocation selected a tracked symlink, a gitlink, an absent path, and a readable file.
Before the repair, the report gave only the names it skipped. It now gives each reason and the mode of the entry that prevented selection.
The same report covered lookups below a symlink, gitlink, and regular file. These used the ancestor's mode.
Only the absent path had no mode. The readable file still produced its finding.
A separate fixture with a missing tree object produced a scan failure for the affected path.
[Headless CLI regressions](../../../../test/cli/headless-scan.test.ts)

## Working-tree path spelling

For case 46, the CLI previously listed both `SRC/A.PY` and `src/a.py` as selected paths on a case-insensitive volume.
After the repair, selecting either spelling returned one finding at `src/a.py`. The report preserved each supplied spelling in `requestedPaths`.
On a case-sensitive APFS volume, the CLI reported separate findings for `src/A.py` and `src/a.py`. It skipped the absent `SRC/A.PY`.
Revision scans required the exact Git tree spelling on both volumes. The Windows CLI also passed the spelling check on NTFS at `209ce53ce`.
That first repair skipped working-tree symlinks. Running the earlier CLI showed that it accepted both a file link and a path below a directory link.
The corrected regression preserves both selected link names and the single finding at their shared target. It passed on both APFS volumes and on NTFS.
At `b66ec0203`, the Windows guest ran the four selected tests together in 20.28 seconds. All four passed.
They covered path spelling, Git message selection, tree invalidation on NTFS, and checked source reads.
[Headless CLI regression](../../../../test/cli/headless-scan.test.ts)

## Declared outputs with constant rows

Case 100 affected programs whose declared outputs contained only facts after constant folding. A revision scan exported an empty CSV for a single literal fact.
Adding a document-dependent output to the same program made the missing rows appear.
After the repair, the standalone `logic` and `csv` commands exported the literal fact and both rows of a relation that also read a Git blob.
Over an empty tree, both relations still exported their literal facts.
[Standalone regression](../../../../native/tests/git_revision_cli.rs)

## Native source reads

Case 28 now has a checked read. One revision view returned distinct results for an absent path, an oversized blob, and an empty file.
Removing a tracked blob or its containing tree raised an error with the missing object ID. Moving the Git directory also raised an error.
Restoring each item made the file readable again on the same view.
The headless rule loader uses the checked read for JSON policies at a Git ref.
[Read contract](../../../headless-scan.md#reading-a-file-from-a-native-revision-view) · [Regression tests](../../../../test/cli/headless-scan.test.ts)

## Live checkout ownership

Cases 95 and 96 reproduced on the same router before the repair. Moving a registered child's `.git` left the child in charge.
Adding a gitlink for an unregistered child left the parent in charge.
After the repair, moving and restoring `.git` changed ownership on the next lookup. An unregistered nested repository stayed under its parent.
Adding or removing either the configured submodule path or the gitlink changed ownership.
A configured ancestor directory did not claim a descendant gitlink. Removing and restoring the shared index also changed ownership.
[Router regressions](../../../../test/engine/code-rule/checkout-code-rule-router.test.ts)

Case 97 passed with two registered directories outside Git. Moving the first and pruning its registration released its files in the same resolver.
The second directory kept its files. Restoring the first marker alone left the path unowned.
Recording it again restored ownership. [Owner regression](../../../../test/engine/catalog/checkout-owner.test.ts)

## Windows results at `65e24db1d`

Both checks ran under Parallels on Windows 11 Pro ARM64, build 26200. The fixtures used NTFS.
Node 24.14.1 loaded the ARM64 addon built with MSVC and Rust 1.99.1. Git for Windows was 2.52.0.windows.2.

| Check | Result |
| --- | --- |
| Message selectors | Git resolved 22 expressions and rejected 16. The addon returned the same 22 commit IDs and rejected the same 16 expressions. |
| In-place tree rewrite | The damaged object retained its size, modification time, creation time, and file ID. Both the open view and a new view rejected it. Warm and cold CLI scans reported identical failures. After restoration, the open view read the directory and the warm scan reported the original finding again. |

The first build at `250c76fd4` failed. Windows rejected a slash appended to a verbatim grammar path. MSVC also rejected verbatim scanner and header paths.
`01f5ff02c` passes ordinary native paths to the compiler. All eight vendored grammars built.
`0f0ca72bf` supplies MSVC intrinsics for the JSON kernels. The Windows ARM64 parser passed its tape and digest regression, and MSVC compiled the x64 translation unit.
`74e7a153d` reads process CPU time from the platform profiler. On Windows, a `repeat` invocation returned 6,487 grammar rows and reported user and kernel time.

The initial NTFS test reached teardown, where Windows refused to delete the open stores.
`65e24db1d` runs each scan in a separate CLI process. Each process releases its store handles on exit.
The native view stays open across the rewrite and restoration. Both Windows tests passed together on the committed checkout. The CLI rewrite test also passed on macOS.
[Regression tests](../../../../test/cli/headless-scan.test.ts)

The Parallels guest rebuilt the Windows ARM64 addon at `61d8073a5`. On NTFS, all three selected tests passed in 24.96 seconds.
These covered Git message selection, invalidation and restoration of a damaged tree, and checked source reads on a single revision view.
The read test distinguished a missing path, an empty file, a byte limit, and an unavailable blob or subtree.
It also moved the Git directory and restored it before reading again.

## Earlier repairs to the catalog and store

The headless regression for case 102 reproduced the permission change before the repair.
After the repair, each scan reported the expected violation and published its scope. The existing parents kept modes `0755` and `1777`.
Both database files had mode `0600`. The native tests also check the journal mode and repair a managed cache directory on reopen.

The catalog regressions cover cases 88–91 and 94. Pruning kept a main checkout with a separate Git directory, its linked checkout, and their settings.
It kept settings through an access denial and collected them after an actual removal.
It also kept settings when the original checkout returned or a second connection registered a replacement before the prune transaction.
An ordinary replacement directory had no checkout identity. Both catalogs adopted a later remote, including after a forced transaction rollback.

## Checks repeated at `e7e4bf393`

At that commit, the review had 28 open items. That count excludes the two Windows runtime checks and the optional language features below.
Six fixtures ran again on macOS with the current TypeScript code and the native addon from `6507e1a65`. The intervening commit changed tests and notes.

| Case | Input and observation |
| --- | --- |
| Bound commit metadata | Git reported zero parents for the initial commit and one for its child. The [root policy](#a-direct-rule-can-produce-a-false-finding) reported both. Adding the explicit object request made it report only the initial commit. |
| 88 | Git read HEAD from a main checkout with a separate Git directory. Pruning removed the registration and its only rule setting. |
| 89 | An unprivileged process lost search permission on the checkout's parent. Pruning removed the only setting before the parent became readable again. |
| 90 | An ordinary directory replaced a Git checkout at the same path. Lookup returned the former repository key together with `git: false`. |
| 91 | Two catalogs recorded the same checkout before it gained an origin. After registration ran again in each catalog, the first returned a remote key and the second returned a clone key. |
| 102 | Opening a store changed an existing parent from `0755` to `0700`. It also changed a separate parent from `1777` to `0700`. Both new databases had mode `0600`. |

The [checkout and cache fixtures](#six-more-failures), [remaining cases and rule interfaces](#the-other-22-open-items), and [permission fixture](#102-opening-a-scope-store-changes-permissions-on-its-parent) give the repair checks.
Case 68 has since passed for writers that record their control generation. The next section describes the remaining limitation for older writers.

## Directory generation cleanup after `9fd0a64ff`

Case 68 now passes for directory acquisitions whose control generations the store observes.
A parent stop previously removed 30 completed runs and left 30 directory-generation rows.
With the same inputs, the previous addon delivered the stop against the upgraded store. The store kept zero runs and zero generation rows.
A delayed acquisition recreated zero rows. A writer in another home and a writer above the cutoff both kept their runs.
The regression repeats publication after reopening the store and checks the persisted scan IDs.

An older writer can acquire a directory without recording its control generation. The store then lacks a safe cutoff for removing that generation row.
A mixed-version run kept the older writer alive across a lower stop and published its run.
In a store created by the previous addon, an unconditional release of the same scope removed its legacy row.
The previous addon then reopened the directory and published. A writer it acquired before that release received the `released` result.
The other 28 items from the review below remain open.
[Regression tests](../../../../native/src/content_store/corpus_runs/tests.rs) · [Scope contract](analysis-scopes.md)

## Findings after `be7e18211`

That review tracked 29 open items. The [previous review](#findings-at-ba58532ee-on-macos) gives the evidence and acceptance checks for 28 carried forward.
They include a performance measurement with an unknown cause and proposed rule APIs.
Case 102 adds a filesystem side effect. The [Linux verification](#linux-verification-after-8ea34a1a1) records the repairs and their regression results.

Fix the operations that can change unrelated permissions, delete live settings, or return the wrong rule answer first.
The corresponding regressions check the parent directory's mode, the surviving catalog rows, and the exact exported or negated relation.
For cache reuse, case 101 measures derivations across runs and scopes. Case 61 measures unrelated history reads.

### 102. Opening a scope store changes permissions on its parent

An unprivileged Linux user created two existing directories under a disposable root.
One directory had mode `1777`. The other had mode `0755`.
The user opened `scope.sqlite` in each directory with `AnalysisScopeRun` and called `finish(false)`.
Both directories changed to `0700`. Both databases had mode `0600`.
A user who only had access through the group or other permission bits would lose access to unrelated files there.

The headless scan reproduced this with `RERAIL_CONTENT_STORE_PATH` pointing to a new database in an existing `1777` directory.
A rule reported the expected HEAD. The report had zero failures and published the named scope. The directory ended at `0700`.

The native store applies the same permission change to every existing parent that the caller owns.
The repair needs to distinguish a Rerail-managed directory from a file path that the caller places in an existing directory.
Preserve the mode and special bits on the caller's existing parent.
Keep the protection for Rerail-managed directories, database files, and sidecars.
Test both shared modes, a fresh managed directory, an older managed directory with loose permissions, and a reopened store.
[Database preparation](../../../../native/src/content_store/private_files.rs) · [Existing store permission tests](../../../../native/src/content_store/tests.rs)

### Linux verification after `8ea34a1a1`

The byte-directory fixture ran on Linux arm64, on overlayfs, with Node 24.22.3 and Git 2.39.5.
Two readable symlinks pointed to physical directories named with byte `ff` and with the UTF-8 replacement character.
Stopping the first directory previously removed three runs. The three belonged to the raw directory, its Unicode sibling, and an older ambiguous scope.
The headless CLI also completed a scan against the sibling's commit when given the first symlink.

`7ef059534` gives each directory a distinct scope. An individual stop preserves the sibling and the old record.
A stop at the common parent collects the old record. Changing the first symlink after queuing its stop preserves the second registration.
The CLI reports an encoding error for the unsupported checkout path before selecting a commit.
[Scope contract](analysis-scopes.md) · [Store regression](../../../../native/src/content_store/corpus_runs/tests.rs) · [Queued release and CLI regression](../../../../test/cli/headless-scan.test.ts)

The same environment exposed a hang when Node loaded the addon. A separate process timed out inside `require`.
`98c76de86` starts workers after the library loads. Fresh Node processes now return the kernel version.
Historical scans also return with `RERAIL_THREADS=1`. [Native health checks](../../../../test/engine/native/native-health.test.ts)

The original concurrent tracking test then timed out with four evaluations and three CPU workers.
The debugger showed all three workers waiting for answers. The callers needed queued document work to produce those answers.
`be7e18211` lets a waiting worker run queued work after its receive times out.
Four simultaneous evaluations returned the correct tracking answers in three batches at each pool size from one through four.
Eight simultaneous evaluations on two workers also returned all forty expected answers across five batches.
The Linux package check runs the opposite-repository test with two workers. [Answer handling](../../../../native/src/code_rule_dsl/logic/plan/corpus.rs)

The Linux selector comparisons, the Linux same-length rewrite, and all six standalone revision command tests passed.
The package job checks the test volume format. The [Windows results](#windows-results-at-65e24db1d) came from a subsequent run.
[Package checks](../../../../.github/workflows/package.yml)

The Linux headless and native health suites passed 142 tests as root. The two skipped permission tests both passed as an ordinary user.
On macOS, two tests exceeded five seconds. Teardown found a remaining `content.sqlite.upkeep` file.
Both tests passed in isolation. With two engine workers, the full suites passed 143 tests and skipped the Linux byte-directory test.
The content store suite passed 257 tests on Linux and 264 on macOS, with four skipped on each platform.
The Datalog suite passed 399 tests. The one-worker SQL example returned the three expected counts from one submission.
`b04243db4` removed eight import-boundary violations in the schema generator and headless tests.
The full structural check still reported 15 failing categories. The same check reported 16 at `8ea34a1a1`.

## Findings at `ba58532ee` on macOS

This review recorded 28 open items. Cases 92 and 93 passed their acceptance checks.
The 22 carried forward included six rule-interface gaps and one unexplained timing difference.
Four more failures affect checkout pruning and long-running filesystem watches. One drops a rule's exported rows. One prevents reuse across identical revision scans.
At that point, the Git-for-Windows, NTFS, and raw Unix directory checks lacked results.

### Six more failures

| Case | Observed result | Repair and acceptance check |
| --- | --- | --- |
| 94. Pruning deletes a newly registered checkout | A prune saw an absent checkout. A second connection registered a new clone and added a rule setting at the same path before the delete transaction. The prune removed both new rows. Git could still read the new HEAD. | Carry the observed registration into the write transaction. Recheck the checkout and its identity under the writer lock. Delete settings only for confirmed removals. Repeat with a new clone and a restored original. |
| 95. A watch routes a path to a removed nested checkout | A router first returned the registered inner checkout for `inner/a.py`. After moving its `.git` directory away, the same router still returned the inner checkout. A new router returned the parent. | Validate cached owning directories against the observed `.git` entries. Exercise removal, creation and restoration during the same session, including an unregistered nested repository. |
| 96. A watch misses a new submodule | An unregistered nested checkout initially belonged to the parent for rule routing. Adding its entry to `.gitmodules` and its gitlink to the index made a fresh router choose the child. The existing router still chose the parent. | Observe the enclosing checkout's effective index and its `.gitmodules` content when caching the submodule decision. Test adding and removing each input under a live router. |
| 97. A plain directory keeps its old owner after pruning | The catalog removed a moved non-Git checkout. A new ordinary directory took its old path. The existing owner resolver still claimed a file there. A new resolver had no owner. | Rebuild or invalidate the set of plain roots from committed registrations when they change. Keep a second, still registered plain root in the regression. |
| 100. A declared output loses constant rows | A revision contained a Python file and a file named `marker` with the line `selected`. Directly exporting a relation with a literal fact and a row derived from `file_line` wrote an empty CSV. Joining that relation to an identifier in the same tree returned both rows. Even a program exporting only its literal fact wrote an empty CSV. | Trace requested outputs through planning and export. Emit the full relation when its rows come from program facts or constant folding. Compare direct and document-dependent outputs, including a run over an empty tree. |
| 101. Repeated scans of one commit recompute a constant relation | Three scans of the same commit read the same line through `file_line`. Each returned `selected`. Each derived the relation again. The observer store held one, two, then three distinct constants records. The first two scans even used the same scope. | Key the derivation by its program, inputs, and observations. Validate the requested path and blob against the current view. Record scope ownership separately. Test repeated scans, a second scope, an unrelated edit, an edit to the requested blob, and deletion of the last owner. |

Cases 95 and 96 use the router's `sourceRoot` method. Case 97 uses its underlying owner resolver.
Case 94 used two actual SQLite connections. The fixture scheduled the registration between enumeration and the delete transaction.
[Router](../../../../src/engine/code-rule/checkout-code-rule-router.ts) · [Owner resolver](../../../../src/engine/catalog/checkout-owner.ts) · [Catalog pruning](../../../../src/engine/catalog/repository-registry.ts)

For case 101, create a commit with `x = 1` in `a.py` and one line, `selected`, in `marker`.
Define `marker(s) :- file_line("marker", _, s).` in a Python policy. Report `s` where `marker(s)` joins `kind(n,"identifier")`.
In one process, scan HEAD three times using one store and scopes `one`, `one`, and `two`.
The profile recorded `derived first` in all three runs. The observer database confirmed the extra records independently.
Two filesystem scans of the same checkout kept one constants record. The next three revision scans added one record each.
The constants key includes the canonical root. A scoped Git view puts the scope hash and invocation number in that root.
[Constants key](../../../../native/src/code_rule_dsl/logic/plan/corpus/constants.rs) · [Git view](../../../../native/src/git_snapshot.rs)

### The other 22 open items

| Case | Evidence | Repair and acceptance check |
| --- | --- | --- |
| 23. A stop can be lost after restore | Restoring the control database before delivery left three published memberships. | Journal the committed decision, acquisition cutoff, and store identities independently of the restored file. Test rollback, restore, and crashes before and after acknowledgement. |
| 68. Directory generations accumulate | Stopping a parent removed 30 child runs but left 30 generation rows. | Compact child rows under a parent cutoff. After restart, reject a delayed older writer and admit a new watch. |
| 42. A removed store stays registered | Delivery can recover from an outage. Permanent removal leaves a route with no retirement operation. | Retire by store identity and retain the outstanding release decision. Bring back the old store, then put a different store at its former path. |
| 75. Two names become one report path | An invalid-byte Git name and the literal name `[hex:ff2e7079]` have the same JSON path. | Carry an explicit encoding and lossless path through findings and failures. Join both report rows to the correct tree entries. |
| 46. A valid filesystem spelling misses a finding | On a case-insensitive volume, selecting `SRC/A.PY` omitted a finding in `src/a.py`. | Match selections against actual filesystem names. Test case-sensitive siblings and exact Git tree names. |
| 11. A large binary fails the revision scan | A 32,000,001-byte binary produced a budget failure. The filesystem scan skipped it. | Check a bounded blob prefix before applying the text limit. Test a binary, large text, and an unreadable blob with the same policy. |
| 28. Source reads hide failures | The native convenience method `readSource` returns the same nullable result for a missing path, an unavailable blob, and the size limit. | Route callers through a checked API. The existing `readFile(path, limit)` throws on read errors and returns null at the size limit. Verify each outcome and retry the same view after the blob returns. |
| 47. Skipped paths lose their reasons | A selected symlink, gitlink, and absent path all appear as skipped. | Keep the entry kind and reason through report generation. Select all three in one invocation. |
| 61. A history lookup depends on unrelated scopes | After 64 unrelated publications, a query about one run read and hashed 65 rows. | Request history by scope and observe only those rows. Check read counts and cache hits after related and unrelated changes. |
| 18. One view retains every decoded subtree | Changing the selection leaves previously visited entries in the view's native map. | Bound decoded bytes. Measure eviction and re-decoding on repeated disjoint selections and on one tree larger than the limit. |
| 52. Git-backed policies are lowered again | Reinspection follows and compiles the include graph again. | Persist compilation by policy blob, resolved includes, and engine inputs. Count lowerings after restart and after editing one include. |
| 53. Fresh catalog overhead is unexplained | Median inspection took 1.677 ms in a new catalog and 0.073 ms in the existing one. | Time store opening, observation checks, and decoding separately. Establish the cause before changing retention. |
| 88. Pruning deletes a valid main checkout | With `git init --separate-git-dir`, Git discovery found the working directory. The worktree list named the metadata directory. Pruning deleted the checkout and its setting. | Recognize a previously recorded main checkout from its current top level, clone identity, and Git directory. Keep registry membership checks for linked worktrees. Repeat with both kinds in one catalog. |
| 89. Access denial deletes settings | Removing search permission from a checkout's parent made pruning delete its row and setting. Restoring permission made the checkout readable again. | Use a filesystem operation that distinguishes absence from access denial. Preserve the row on uncertain reads. Test denial, recovery, and actual removal. |
| 90. A replacement directory inherits the former Git checkout | After moving a recorded repository and creating an ordinary directory at its old path, public lookup returned the old repository key with `git=false`. | Require identity evidence before matching a recorded Git checkout at a reused path. Test non-Git replacement, temporary absence, and restoration. |
| 91. Remote adoption can stop permanently in a process | Two databases recorded a clone before it gained an origin. Only the first adopted the remote. In a separate fixture, a failed adoption still suppressed the retry after the database recovered. | Use the committed catalog state to decide whether adoption is due. Test both catalogs in each order and retry an aborted transaction in the same process. |
| Bound metadata demand | Joining a bound selected commit to `git_message` returns no row until the policy explicitly requests the object. | Propagate the repository and object ID through the library to the existing provider. Compare direct and explicit requests, missing objects, and read counts. |
| Commit trailers | A policy reading `git_trailer` fails to compile. | Derive ordered trailers in the library. Compare duplicate keys, continuations, and ordinary message text with Git. |
| Commit diagnostic locations | A metadata violation at `@place("git",1)` reports a file called `git`. The finding has no commit location. | Add a repository, commit, and optional field to the finding location. Test a message violation alongside a real file named `git`. |
| Repository location facts | `git_repository` compiled. Both proposed path relations, `git_repository_dir` and `git_worktree`, failed as undeclared. | Expose the common directory by repository and the working directory by view. Test two linked worktrees and a bare repository. Paths would be outputs. |
| Content handles | The proposed `git_object` and `git_blob_content` both failed as undeclared. Rules can obtain kind and blob digest. | Specify how rules consume a content handle. Reuse the content store, then verify equal bytes across revisions and explicit missing-object outcomes. |
| Selected document provenance | The proposed `git_snapshot_request`, `git_source_view`, `git_source_file`, and `source_document` all failed as undeclared. Tree entries and selected tree facts already exist. | Expose the selected path, blob, grammar, and document through joins over the existing source facts. Test the same blob at two paths and exclude one from the analysis. |

The last three rows are proposed rule capabilities. A missing proposed name alone does not establish incorrect scan output.
The [implemented Git reference](../../../rule-syntax/git.md), [proposed library](library.md), [catalog lookup](../../../../src/engine/catalog/repository-registry.ts), and [scope contracts](analysis-scopes.md) give their respective interfaces.

### A direct rule can produce a false finding

The existing bound-demand issue also affects negation.
In a two-commit repository, the following clause reported both commits as roots. Both scans completed without an error.

```rr
Root(@place("git", 1)) :- git_source_commit(r, c), !git_parent(r, c, _, _).
```

Adding `git_object_request(r,c) :- git_source_commit(r,c).` made the policy report only the actual root.
A fix must establish a complete parent family before evaluating absence. Test a root, a child, and an unavailable commit.
The unavailable commit must produce an explicit incomplete outcome. [Rule library](../../../rule-syntax/lib/git.rr)

### Repairs verified during this review

| Case | Before | After |
| --- | --- | --- |
| 92. Scope publication and table accounting | The storage architecture check rejected seven tables. | `e52252593` identifies their writers and lifetimes. An injected failure rolled back a partial publication. A retry published one run. Deletion removed its history and ancestor rows. |
| 93. Process-local retention | The registry digest and embedded programs used process-local storage. | `f1b4873ed` generates the digest from the actual suffix registry. `abfab008a` retains each program in the store under the consuming phase. Across five processes, a second scope reused three programs. After deletion of its last owner, a fresh process compiled all three again. [Counts](analysis-scopes.md) |
| 98. A policy change could produce a clean revision report | Two committed configurations each reported a violation. A scan that read the old attachment and the new library table completed with zero findings and zero failures. | `d2da2ef3b` reads eligibility and all home policy tables in one transaction. With the same interleaving, the scan reported the old policy's finding and the next scan reported the new one. [Headless regression](../../../../test/cli/headless-scan.test.ts) |
| 99. A nested evaluation deleted existing files | A headless revision scan deleted a sentinel in the invocation directory's `constants/`. In a standalone run, constant folding and the typing bridge removed sentinels in the output directory's `constants/` and `typing-calls/`. | `ba58532ee` evaluates both phases in memory. The standalone run returned the same row and kept both files. The headless run preserved its file. Three concurrent cache tests completed three repetitions after failing one test in each repetition before the fix. [Standalone regression](../../../../native/tests/git_revision_cli.rs) |

The Datalog suite passed 399 tests and skipped 13. All six standalone Git revision tests passed.
The three concurrent engine tests passed each of three repetitions. TypeScript passed its type check.
Two integration tests exceeded five seconds in a parallel run. Both passed on retry with the same timeout.
All 200 tests in the five headless and checkout suites then passed with one worker.

### Language coverage and open design choices

The shared classifier uses exact names and suffixes. Two executables had identical bytes, including a Python shebang.
The same Python rule found a call in `check.py` and skipped `check` in both filesystem and Git scans.
The cache design proposes shebang detection and configurable parser profiles. Those are additional capabilities.
For shebang support, request a bounded prefix only for unresolved names and key the decision by that content and classifier version.
For parser profiles, first specify which entry points and settings a policy can select. Test the same bytes under two effective profiles.
[Classifier](../../../rule-syntax/lib/source-language.rr) · [Cache contract](cache-contracts.md#4-language-selection)

## Earlier catalog fixtures and verification at `33d7acd50`

### Reproduce the new catalog findings

Use disposable working directories and a fresh catalog opened with `openRerailDatabase`.
Cases 88 and 89 use the same checkout setting insert as the [newline-worktree fixture](../../../../test/engine/catalog/registered-worktrees.test.ts).

* For case 88, run `git init --separate-git-dir=metadata checkout` and make an empty commit.
  Call `recordCheckout` on the working directory, insert one setting, and call `pruneCheckouts`.
  The returned array includes the live working directory. The count of checkout settings changes from one to zero.
* For case 89, record a checkout below a directory you own and insert one setting.
  Remove search permission from the parent, run pruning as an unprivileged user, and restore the parent's permissions.
  Pruning returns the checkout path. The restored checkout has zero settings.
* For case 90, record a Git checkout, rename it, and create an empty directory at its previous location.
  Call `lookupCheckout` before registering the new directory. Its result contains the previous repository key and `git: false`.
* For case 91, record the same checkout in two catalogs before it has a remote. Add an HTTPS origin.
  Call `recordCheckout` on both catalogs again in the same process. Only the first returns a `remote:` key.
  In a fresh fixture with one catalog, install a SQLite trigger that aborts checkout repository-key updates.
  Attempt adoption once, remove the trigger, and repeat. The second call returns the old `clone:` key.

### Repaired while testing the rule examples

A HEAD-only policy used a shared store. A second policy used the same HEAD answer to request the commit and check its empty message.
The old engine skipped the object request and returned zero findings. A fresh store returned one.
The retained native answer had a digest for one row and a payload with zero rows.

The engine now retains the produced rows even if the first program has no consumer inside its request evaluation.
It versions the native-answer observation so the next read can repair an old store.
The old executable missed the finding. The repaired executable returned one finding on the same database, again on a warm rerun, and on a fresh database.
The [standalone regression](../../../../native/tests/git_revision_cli.rs) executes both published examples. Two linked worktrees share a clone and have different HEADs.
It checks their view keys, tips, empty-message result, and the view on each lookup error.
The public view relations are defined in [git.rr](../../../rule-syntax/lib/git.rr).

### Verification limits

Five standalone CLI tests and eight targeted Datalog cache tests passed.
The store gate passed 263 tests with four skipped.
The first 399-test logic run found both facts-only failures above and a fixture that failed while making a Git commit.
Commit `33d7acd50` isolates the fixture from machine Git config. The rerun passed 397 tests and failed the two facts-only checks. The quick tier skipped 13.
Of three additional engine cache tests, one first failed with a missing file. It passed on its own. Case 99 above describes the subsequent stress test and repair.
Of 260 integration tests, three exceeded the five-second timeout on the first run. All 260 passed with two workers and the original timeout.
The timeouts affected the first headless scan, the first repository startup, and the read-only database migration fixture.

Three platform checks still need execution. Compare message selectors with Git for Windows, and test a same-length object rewrite with restored modification time on NTFS.
On a byte-preserving Unix filesystem, delete one directory scope and check ownership of the other directory whose name differs only in its raw bytes.

## Checkout lookup and pruning after `f15ea8b95`

Cases 81, 82, 86 and 87 now have regression tests. Registration and pruning kept the newline-named worktree and its setting with Git 2.50.1 and 2.35.8. Both catalog suites passed all 28 tests with Git 2.35.8. The five catalog and runtime suites passed 55 tests with the system Git.

The long-lived resolver reads the current checkout on each call. It follows linked-worktree replacement and restoration. An unrecorded clone sees a changed or removed remote, including a remote loaded from an updated include file. The registry reuses a worktree list only after checking the common directory and its registration records.

Fresh discovery has a cost. In 30 warm resolver calls per checkout, the median was 8.69 ms for a recorded checkout and 22.83 ms for an unrecorded clone. This used one open catalog connection on macOS with a one-minute load average of 8.87. The latter lookup also asks Git for the effective remote.

### Deferred ownership probes

Case 17 now keeps each probe's last failure, attempt count, and due time in the control database.
The retry test reads the row through 13 failures and checks the five-second initial delay, doubling, and one-hour cap.
Recovery uses the original release ID and preserves a later directory owner. The migration keeps the due time of a pending probe from an older database.

## Status at `4b4c8754e`

Seventeen implementation issues or performance questions and five rule-interface gaps remain in this review.
The preceding review listed 25 implementation items. Ten now have committed repairs. Two additional cases affect the long-lived checkout resolver.
The rule-interface count includes two further discrepancies described below. The platform checks have separate status.

### What can still go wrong

| Case | Evidence | Repair and acceptance check |
| --- | --- | --- |
| 23. A committed stop can disappear | Restoring an older control database before delivery left three memberships alive. | Make a journal outside the restored control file authoritative for the stop decision, its cutoff and its target stores. Apply the decision idempotently to the control file and stores. Test rollback, restore, and crashes on both sides of acknowledgement. |
| 68. Stopped directories leave generation rows | Stopping a parent removed 30 child runs and retained 30 generation rows. | Compact covered generations with a fence for prior writers. Attempt late publication before and after restart, then start a new watch. |
| 17. An offline store is polled every five seconds | A failed legacy ownership probe loses its error and repeats the same interval. | Record the failure and attempt count. Increase the delay up to a cap. Reconnect and deliver the original cutoff. |
| 42. A permanently removed store stays registered | Temporary outages are recoverable, but the registry has no way to retire a store. | Add retirement of an identified store with outstanding releases accounted for. Test an offline store returning and a different database taking its pathname. |
| 75. Two Git names have the same report path | The invalid-byte filename and the literal string `[hex:ff2e7079]` both display as that string. | Add an unambiguous machine-readable path representation to failures and findings. Keep byte identity through rendering and deduplication. |
| 46. An accepted filesystem spelling can lose a finding | On a case-insensitive volume, selecting `SRC/A.PY` missed the finding in `src/a.py`. | Resolve the selection against actual inventory names on that volume. Test case-sensitive siblings and the exact-name Git tree separately. |
| 11. A binary can fail a historical scan | A 32,000,001-byte binary failed in Git and was skipped on the filesystem. | Check a bounded prefix for binary content before applying the text limit. Also exercise large text and a missing blob. |
| 28. Native source reads hide the cause of failure | The same nullable result represents an absent path, an unavailable tracked object, and the text limit. | Add a checked read with distinct outcomes, and migrate callers that need diagnostics. Remove and restore the blob on the same view. |
| 47. An excluded entry looks absent | A selected symlink, gitlink and missing path all land in the skipped list without a reason. | Carry the entry mode and skip reason into the report. Exercise all three in one scan. |
| 61. One scope's history observes unrelated scopes | A query for one run read and hashed 65 rows after 64 unrelated publications. | Request and observe rows by scope. Count rows and cache hits after changing the queried scope and an unrelated one. |
| 18. A long-lived view retains decoded subtrees | Disjoint selections leave the old decoded entries resident. | Budget decoded bytes and evict old entries. Measure retained bytes and re-decodes for disjoint paths and a tree larger than the budget. |
| 52. Git-backed policies repeat compilation work | Reinspection follows and lowers the include graph again. | Cache the compiled graph by policy blob, resolved includes and engine inputs. Count lowerings after restart and after replacing one include. |
| 53. Fresh-catalog inspection costs more | Warm inspection took a median 1.677 ms in a new catalog and 0.073 ms in the existing one. The cause remains unmeasured. | Separate store opening, validation and decoding. Make any optimization preserve correct answers across stores and eviction. |
| 81. A newline in a worktree name breaks registration and pruning | In this review Git's NUL-delimited list contained the complete name. The catalog missed it. `pruneCheckouts` removed the recorded checkout while Git still listed it. | Share one structured reader between registration and pruning. Use NUL-delimited Git output or exact administrative records on an older Git. Check that pruning preserves that checkout's settings. |
| 82. The worktree registry retains the old common directory | Replacing A's linked worktree with B's at the same path produced B's list labelled with A's common directory. | Revalidate the checkout's Git directory, common directory and registration data before reusing the list. Test moving a linked worktree. |
| 86. The daemon can return A's settings for a linked checkout from B | I retained `createCheckoutResolver`, moved A's worktree, and put B's there. The retained resolver returned A's repository key. A fresh lookup returned B. | Invalidate cached discovery when the checkout or its Git directory changes. Compare the public resolver with fresh lookup after replacement, during absence, and after restoration. |
| 87. Changing the remote leaves a fresh clone using the old remote's settings | A previously unrecorded clone used a recorded remote A. After changing its origin to recorded remote B, the same resolver still returned A. A fresh lookup returned B. | Revalidate remote-derived lookups against the effective Git configuration. Test a URL change, a removed remote and an included config file with the same resolver. |

Cases 82 and 86 use different entry points and caches. Case 87 requires no move. The checkout keeps both its directory and its Git directory throughout.
[Registry of worktrees](../../../../src/engine/catalog/repository-identity.ts) · [Checkout resolver and pruning](../../../../src/engine/catalog/repository-registry.ts)

### What a rule author still cannot express as designed

| Gap | Observable behavior | Repair and acceptance check |
| --- | --- | --- |
| A bound commit does not request its message | Joining `git_source_commit` to `git_message` yielded no row until the policy added an object request. | Propagate bound keys through the library. Compare direct and explicit queries, check missing objects, and count object reads. |
| The documented HEAD example does not compile | The design uses `git_ref`. The public library declares a revision lookup and no ref-only lookup. | Make the example executable. Specify whether exact ref names and revision expressions require different relations. Test a branch, a symbolic HEAD and an expression. |
| Ordered trailers have no public relation | A rule asking for the signed-off-by trailer failed to compile. | Implement ordered trailer facts, including repeated keys and continuations. Compare the examples with Git. |
| A mutable lookup's public result drops the worktree key | Across two linked worktrees, the same repository key and expression `HEAD` gave different correct tips. The `git_root_view` values differed. The convenience answer `git_revision(repo, expr, oid)` cannot identify its worktree by itself. | Expose a public view-keyed lookup and error relation over the existing view-keyed provider. Document the invocation-scoped convenience relation and correct the design's definition of repository identity. Test both worktrees under one store. |
| A finding about a commit has only a file-style location | A metadata-only policy anchored at `@place("git",1)` produced a finding at path `git`, line 1, in an empty tree. The report names the scanned commit. The finding has no structured commit location of its own. | Provide a commit location carrying repository, object ID and optional field. Preserve source spans for actual files. Exercise a commit message and a real file named `git` in the same report. |

The rules for tree placement already live in `.rr`. The two-file revision fixture reused language classification and parser tapes with zero warm parses. That evidence concerns source processing. It does not close the Git-backed policy compilation or scope-history cases above.
[Design contract](library.md) · [Current rule API](../../../rule-syntax/git.md) · [Measured cache behavior](analysis-scopes.md)

### Repairs and evidence on this branch

| Cases | Commit | What passed |
| --- | --- | --- |
| 44, 65, 78, 79, 80, 85 | `8fea135fb` | An existing view follows A's ref updates, refuses B at A's old location, and resumes after A returns. A moved read-only clone recovers its history and settings. The catalog rejects an oversized ID and a FIFO. A copy carrying the marker shares clone history and has a separate worktree view. |
| 74 | `73689678b` | The optional-handle addon batch left six memberships before the repair and zero after a failed batch. Both policy orders retain prior published ownership. A subsequent metadata-only evaluation succeeds after a missing subtree. |
| 83, 84 | `e0b83379d` | All 128 extended selector comparisons agree with Git, including the previous 40 mismatches. The earlier 60 named-remote comparisons still agree. Reusing a view observes configuration changes. |
| 67 | `5074154dd`, `4b4c8754e` | Invalid revisions create no store or route. Tests cover failing policies, simultaneous acquisitions with the same scan ID, interruption on both sides of publication, a killed writer, and pre-upgrade registrations. An offline store remains a candidate until it can answer. |

Git for Windows message selectors, same-length object rewrites on NTFS, and directory-scope release with invalid filename bytes on a byte-preserving Unix volume still need runtime evidence.
The six-suite run after the addon rebuild had one timeout in the first filesystem scan. Its other 205 tests passed.
Five fresh Node processes using the same addon and separate empty stores measured 4 ms for native binding load, 227–245 ms for runner import, and 234–255 ms for the first metadata-only revision scan. The repeat scan took 69–80 ms. Those figures cover a different fixture and do not explain the filesystem test's 8.4 seconds.
I also timed the filesystem test's policy and source outside Vitest, in five fresh processes. Native binding load took 6–7 ms. Runner import took 222–236 ms. The first filesystem scan took 207–319 ms. The one-minute host load average at the start was 8.57. These runs reused the installed addon and the operating system's file cache.
The final run of all six suites passed 207 tests at the original timeout. The store gate passed 263 tests with four skipped. Four standalone revision CLI tests, six cache architecture tests, and TypeScript checking passed. The intermittent first-test delay remains unexplained.

## Status at `87d18d609` and in the identity draft

The previous review listed 20 follow-up cases. Three have committed repairs. Seventeen still lack a committed repair, including one performance measurement.
This review adds eight reproduced problems and three differences between the library design and the public rules.
Case numbers below belong to this follow-up audit. The original audit has separate numbers.

### Eight new reproductions

| Case | How to see it | Observable result | Repair and acceptance check |
| --- | --- | --- | --- |
| 78. An open view reads another checkout's HEAD | Open a named `GitRevisionSource` on A. Rename A and put an independently initialized checkout B at its old path. Query both `git_source_commit` and `git_revision` for HEAD in the original view. | One policy reported A's pinned commit and B's HEAD. The same result occurred with the preceding native addon and with the draft. | Verify the clone and worktree identities before reading mutable refs or reopening storage. Return a source error if the view can no longer open its own repository. Check a real ref update in A as a positive control. |
| 79. The catalog accepts part of an oversized ID | Write `short-id`, pad it with spaces to 128 bytes, then append `trailing-content` to the repository marker. Call `resolveCloneId`. | The 144-byte file identifies the clone as `short-id`. The draft native reader rejects the same file. | Check the file size and read one byte past the limit. Apply the same acceptance rules in both readers. Test 128 bytes, 129 bytes, and a valid prefix with a suffix. |
| 80. A repository identity lookup hangs | On Unix, use a named pipe for the common directory's ID file. Call `resolveCloneId` in a child with no pipe writer. | The child entered the call and was still blocked when the 2.2-second deadline expired. | Open nonblocking on Unix and check the open descriptor for a regular file. Check that an ordinary file still works and a pipe returns an error without a writer. |
| 81. A worktree name loses its final component after a newline | Register a linked worktree with a literal newline inside its directory name. Compare `worktreeRegistry(main)` with `git worktree list --porcelain -z`. | Git returned the full path. The catalog returned only the characters before the newline. | Read NUL-delimited paths on supported Git versions. On older versions, use structured worktree data or read the administrative path records. Exercise the automatic registration entry point on both paths. |
| 82. The registry associates worktrees with the wrong common directory | Read A's registry from a linked checkout. Move it with Git. Add a linked checkout from B at the vacated path. Read its registry in the same process. | The returned list belonged to B, but its reported common directory still belonged to A. | Revalidate the checkout's Git directory and common directory before reusing the registry. Repeat the replacement with separate registries and an unchanged-checkout control. |
| 83. A local upstream disappears | Set `branch.main.remote=.` and `branch.main.merge=refs/heads/local`. Compare `@{u}` and the applicable `@{push}` modes with Git. | Git returned the local branch. Rerail rejected it. The extended matrix had 32 such mismatches. | Resolve the local merge ref in the current repository. Check both names, two distinct tips, and local and named push remotes. |
| 84. Invalid push configuration still produces an upstream answer | Configure a named upstream and set `push.default` to `CURRENT` or `SIMPLE`. Resolve `@{u}`. | Git 2.50.1 rejected the configuration. Rerail returned an object ID in eight comparisons. | Validate the configuration that Git loads for this selector. Compare valid values, invalid values, and ordinary ref lookups before setting the validation boundary. |
| 85. Passive catalog lookup gives another checkout the old repository key | Record read-only A with no marker. Move A, create unrelated read-only B at the old path, and call `lookupCheckout` at both locations. | In the draft, lookup lost A and gave B the recorded key for A. Active identity resolution distinguished B. | Use the same read-only identity evidence for passive catalog lookup and recording. Check that A finds its settings after the move and B finds none of A's settings. |

Cases 78 and 65 exercise different lifetimes. Case 65 starts a new scan after a move. Case 78 keeps the old view alive through replacement.
Case 85 reads the settings catalog. Its result can differ even when completed-run facts name the correct repository.
[Snapshot and mutable repository access](../../../../native/src/git_snapshot.rs) · [Catalog identity and registry](../../../../src/engine/catalog/repository-identity.ts) · [Passive checkout lookup](../../../../src/engine/catalog/repository-registry.ts)

The selector comparison used two branch tips and local refs only. It varied fetch remote, push remote, merge target, push mode, and expression.
Of 128 comparisons, 88 agreed. Cases 83 and 84 account for all 40 differences. The previous 60 comparisons used named remotes and valid modes.
[Revision resolver](../../../../native/src/git_snapshot_revision.rs)

### What a rule author encounters

| Design expectation | Result from an actual policy | Proposed change and check |
| --- | --- | --- |
| A bound commit can supply its message | Joining `git_source_commit(r,c)` to `git_message(r,c,m)` completed with zero findings. The commit had a UTF-8 message. Adding `git_object_request(r,c) :- git_source_commit(r,c).` returned the full text. | Propagate a bound repository and object ID through the library lookup. Keep explicit requests for batch enumeration. Check direct and explicit queries, missing objects, and the number of objects read. |
| The design's HEAD example uses `git_ref` | The compiler reported an undeclared relation. The implemented API uses an explicit revision request. | Make the contract example executable. Use the existing revision API if the example requires revision semantics. If it requires a ref name, define that narrower operation and test a ref, a tag, and a revision expression. |
| A rule can ask for commit trailers | A commit contained a `Signed-off-by` trailer. A policy reading `git_trailer` failed at compilation. | Define ordered trailer facts in the library. Specify continuations, duplicate names, and the end of the trailer block. Compare the fixture with Git's trailer parser. |

These are design gaps. The implemented reference already shows an explicit object request. The two other relation names come from the design document.
[Design contract](library.md) · [Implemented reference](../../../rule-syntax/git.md) · [Git rule library](../../../rule-syntax/lib/git.rr)

### Seventeen earlier cases without a committed repair

| Case | Last observed result | Repair and check |
| --- | --- | --- |
| 23 | Restoring an older control database lost a committed stop. Three memberships survived. | Keep durable evidence of the commit, routes and acquisition cutoff through control-database restore. Test rollback, committed stop, crash before delivery and replay after delivery. |
| 74 | One failed optional-handle native batch published six memberships and zero runs. An explicit aborted run published neither. | Acquire one run for the whole durable batch. Publish after all policies and required reads succeed. Check both policy orders and a scope with earlier owners. |
| 65 | Committed history uses the Git directory pathname. A linked worktree or rename loses the history. A replacement at the old path can inherit it. | Persist a clone ID for history and a worktree ID for HEAD and index. Test independent scans and case 78. Keep older path-only history enumerable by scope. Its original clone cannot be inferred from the pathname. |
| 44 | First use of a read-only Git directory without an ID fails in the committed code. | Use stable directory identity when available. The draft passed the restart check below. Specify behavior when the filesystem supplies neither a marker nor a stable directory identity. |
| 67 | Three invalid revision scans left three routes and three generations. Later deletion of the name was ambiguous. | Resolve the revision before acquisition. Remove unused routes after abort. Test simultaneous successful and failed acquisitions. |
| 68 | Stopping a parent removed 30 child runs and left 30 generation rows. | Retire generations once the stop cutoff covers older writers. Check delayed publication, restart, and a new watch. |
| 17 | A failed legacy-store probe retries every five seconds without retaining its failure reason. | Store the reason and retry count. Cap increasing delays and deliver the original cutoff after recovery. |
| 42 | The store registry has no operation for retiring a permanently removed store. | Add explicit retirement after resolving outstanding ownership and deliveries. Check that a temporarily offline store can still return. |
| 75 | A byte path and the literal name `[hex:ff2e7079]` have the same report path. | Preserve raw path bytes in machine-readable findings and failures. Check both names at the root and below a directory. |
| 46 | On a case-insensitive volume, selecting `SRC/A.PY` can miss a finding in `src/a.py`. | Match filesystem requests to inventory spelling. Test both volume types and keep exact Git tree selection. |
| 11 | A 32,000,001-byte binary failed a revision scan. The filesystem scan skipped it. | Determine binary status from a bounded prefix before applying the text limit. Check large text and absent objects as controls. |
| 28 | The nullable native source reader conceals absence, an unavailable tracked blob, and the byte limit. | Return distinct read outcomes and preserve the compatibility wrapper as needed. Remove and restore a blob in the same view. |
| 47 | Historical reports group excluded symlinks and gitlinks with absent entries. | Preserve the Git mode and skip reason. Select a link, a gitlink, and a missing path together. |
| 61 | Asking for one scope read and hashed 65 records after 64 unrelated publications. | Add a scope-keyed request and observation. An unrelated publication must preserve the hit. Keep whole-store enumeration available. |
| 18 | A long-lived view retains decoded entries for every subtree it visited. | Bound decoded bytes. Measure retained memory and decodes for disjoint selections and for one oversized tree. |
| 52 | Repeated inspection of a policy from Git reads and lowers its include graph again. | Retain the compiled program by policy blob, resolved includes, and engine inputs. Count lowerings after restart and after one included file changes. |
| 53 | Median warm inspection took 1.677 ms with a fresh catalog and 0.073 ms with the existing catalog. | Measure store opening, validation and decoding separately. Check alternating stores and eviction before changing validation. The timing alone does not identify redundant work. |

### What passed and what still needs a host

* `a4d536b8b` fixed equal-time message selection in case 77. All 15 branch, tag and detached-HEAD comparisons agreed with Git.
* `da05181a9` fixed the named-remote push destination in case 76. All 60 original comparisons agreed. Cases 83 and 84 use different configurations.
* `87d18d609` fixed bare and linked discovery in case 45. The catalog tests exercised both discovery orders and preserved earlier bare-checkout settings.
* In the identity draft, fresh scans found the original run after an on-volume move and in a linked worktree. A fresh clone at the old path found no such run. A filesystem copy with the marker shared history and had a different worktree ID. This is the draft's copy behavior.
* The draft published a first scan without writing to its read-only Git directory. After the directory became writable, a fresh process published its marker and found the first commit under the same clone ID. Case 85 still fails.
* The six headless, CLI, Git, and catalog suites passed 196 tests. All four standalone revision CLI tests passed. The store gate passed 263 tests with four skipped. Six cache and combined-program tests passed.
* NTFS still needs the same-length object rewrite with restored modification time. Git for Windows still needs the message-selector comparison. A Unix volume that accepts invalid filename bytes still needs the independent directory-scope release test.
* Earlier runs sometimes exceeded the first scan's five-second test timeout after an addon rebuild. The 196-test run passed. Native load time and first scan time still need separate cold-process measurements.

## Status at `f0a2f43c9`

Twenty implementation findings remain. Five of the previous 21 have verified repairs. Four more have new reproductions below.
The case numbers in this report refer to the follow-up audit. The original audit has its own numbering.

### Scope ownership after a failed native batch (74)

I ran the same two local policies against the same Git commit with the Node addon's `scanRrPoliciesAsync`.
The first policy reported one Python identifier. The second returned `.references: unknown key unknownParameter`.
I queried the store after each call.

| Native call | Published manifest memberships | Completed runs |
| --- | ---: | ---: |
| Supply an `AnalysisScopeRun` and finish it with `false` | 0 | 0 |
| Supply a named `GitRevisionSource` and omit the optional run | 6 | 0 |

The second call makes the failed batch's name visible to later rules that ask which scopes own cache entries.
Stage a durable native batch under one acquisition even when the caller omits the run handle.
Publish after all policies and required source reads succeed. An explicit handle must still let a caller group evaluations.
Test a new name, a name with earlier successful memberships, both policy orders, and a successful batch.
[Native batch entry point](../../../../native/src/lib.rs) · [Scope publication](../../../../native/src/content_store/analysis_scope.rs)

### One report path identifies two different Git entries (75)

I used `git mktree -z` to put the four filename bytes `ff 2e 70 79` and the literal text filename `[hex:ff2e7079]` in one tree.
The native directory listing distinguished them. Reading the text filename returned its blob.
A metadata policy reported a finding on the text filename. A Python policy in the same batch required the source inventory.
The JSON report used `[hex:ff2e7079]` for both the finding's path and the invalid-UTF-8 failure's path.
A consumer that joins the two arrays by path associates the failure with the readable file.

Give byte paths a distinct representation in the machine-readable report. Keep the hex label for display.
Check that a consumer can join each entry back to the raw directory listing. Cover an invalid component below a valid parent as well.
[Inventory path reporting](../../../../native/src/git_snapshot.rs) · [Scan report](../../../../src/cli/headless-scan.ts)

### Two revision expressions give different answers from Git (76 and 77)

The comparisons used Git 2.50.1, the native source constructor, and a policy reading `git_revision` and `git_revision_error`.
All refs and objects were local. I created two commits with the same committer timestamp and different IDs.
Branch `aaa-match` pointed at commit A with message `equal audit alpha`. The checked-out `bbb-match` pointed at commit B with message `equal audit beta`.
I configured its upstream as `origin/bbb-match` (A), its push remote as `fork`, and `fork/bbb-match` as B.

| Expression | Git | Rerail rule |
| --- | --- | --- |
| `@{u}` | A | A |
| `@{push}` with `push.default=simple` | Exit 128, cannot resolve a single destination | B |
| `:/equal audit` | B | A |
| `:/equal audit beta` | B | B |
| `HEAD^{/equal audit}` | B | B |

* **76. Push destination.** Apply Git's single-destination check to the configured push mode and tracking refs before returning a commit.
  Exercise `simple` in triangular and single-remote configurations. Compare `current`, `upstream`, `matching` and `nothing` with Git as controls.
* **77. Message search order.** Match Git's ordering of starting refs, HEAD, and equal-time candidates. Test identical timestamps with matching branch and tag tips, attached and detached HEAD, and a uniquely matching message.

[Revision resolver](../../../../native/src/git_snapshot_revision.rs)

### Earlier findings that remain open

| Case | Observable problem | Repair and check |
| --- | --- | --- |
| 65 | A history join found the seed commit in the original checkout. It lost the row after a rename. An unrelated checkout at the old path returned the seed commit. A linked worktree at **the very same commit** also returned zero rows. | Use a durable clone identity for stored object history and retain a separate identity for worktree-local HEAD and index requests. Repeat all four queries. Old path-only records need evidence before assigning them to a clone. |
| 23 | Restoring an older control database lost a committed directory stop. Three memberships survived. | Preserve the commit decision, store routes and acquisition cutoff through restore. Test a rollback and crashes before and after delivery. |
| 46 | On a case-insensitive volume the filesystem accepted a selected path, then the policy missed the file under its stored spelling. | Match filesystem selections to inventory spelling. Test wrong-case names on both kinds of volume and preserve Git's exact tree names. |
| 11 | A 32,000,001-byte binary failed a revision scan. The same working-tree scan skipped it. | Classify from a bounded blob prefix. Exercise large binary, large text and missing Git objects with the same policy. |
| 28 | The native `readSource` returned the same empty result for an absent path and an unavailable tracked blob. It also drops the size-limit error. | Give consumers distinct read outcomes. Remove and restore a blob on the same view and check the byte limit. |
| 47 | A historical path report groups excluded entry modes with missing names. | Preserve mode and reason. Request a symlink, a gitlink and a missing path together. |
| 44 | A first scan cannot assign an identity to a readable clone whose Git directory is read-only. | Record an initial identity in the Rerail home when needed. Check a restart and a later writable scan. |
| 45 | A bare repository and its linked checkout receive different clone IDs. | Resolve Git's common directory in both cases. Test discovery in both orders and reconcile earlier IDs. |
| 67 | Three invalid revisions created three routes and three generations. Reusing one name in another store made deletion ambiguous. | Resolve the revision before acquiring the scope. Retire an aborted acquisition's unused route while preserving concurrent acquisitions. |
| 68 | One parent stop removed 30 directory runs and left 30 generation rows. | Compact generations once the stop cutoff fences old writers. Test a pending writer, a new watch and repeated one-time directories. |
| 61 | A query for one scope read and hashed 65 records after 64 unrelated publications. | Add a scope-keyed request and cache observation. Keep enumeration available. Check a cache hit after an unrelated scope changes. |
| 17 | A failed probe of a legacy store retries after five seconds without a stored reason. | Record the cause and retry count. Use capped backoff. After recovery, apply the original cutoff. |
| 42 | A permanently removed store has no maintenance retirement operation. | Retire its route after resolving ownership and outstanding deliveries. Check that an offline store can return. |
| 18 | A long-lived view keeps decoded entries from every visited subtree. | Bound retained decoded bytes and measure repeated disjoint selections, including one tree larger than the bound. |
| 52 | Repeated inspection of a Git-backed policy reads and lowers its include graph again. | Persist the compiled program by policy blob, resolved includes and engine version. Measure an unchanged restart and a changed include. |
| 53 | In the paired measurement, the median inspection took 1.677 ms with a fresh catalog and 0.073 ms with the existing catalog. | Measure store opening, validation and query time separately. Test alternating stores and collection before changing the reuse path. |

### Repairs checked in this pass

| Case | Commit | Evidence |
| --- | --- | --- |
| 66 | `769ee8124` | Switching rule libraries now selects the requested include and diagnostic in both orders, including a fresh process with a warm store. An already inspected policy retains its original include. |
| 73 | `bee6fbc44` | A collector rejects a missing or replacement lock after its directory moves. Restoring the original directory still prevents collection until its reader releases the shared lease. |
| 70 | `323273c75` | A warning before publication still prevents publication. A late worker warning cannot change the committed result returned by a second finish. |
| 69 | `cd7eac26a` | With zero and 1,000 unrelated scopes, the stop trigger executed 208 and 209 SQLite VM instructions. Before indexing it took 141 and 5,142 for the same deletion. |
| 71 | `f0a2f43c9` | A store produced by the older addon had five manifest memberships and zero recorded runs. The upgraded rule found its owner. A second fixture tested directory and named owners, pending writes, and retraction on deletion. |

### Verification gaps

* The same-size rewrite with restored modification time still needs an NTFS run. Compare warm and cold diagnostics for the damaged and restored object.
* Git message selectors still need a comparison with the installed Git for Windows. Compare the returned IDs and rejections.
* The macOS volume rejected the invalid-byte directory name during setup. On a byte-preserving Unix volume, publish through two directory aliases and check that deleting one leaves the other's ownership intact.
* The first checkout scan again exceeded its five-second test timeout after rebuilding the addon. Measure native loading and first scan execution separately in fresh processes.

The repeated headless, CLI usage and Git suites passed all 160 tests with the same addon and timeout. The first run passed 159.
The store gate passed 263 tests and skipped four. Six cache architecture and combined-program tests and four standalone revision CLI tests passed. TypeScript checking passed.

## Status at `183e7f6b1`

Twenty-one implementation findings remain. Eight have new reproductions below. Thirteen have earlier reproductions or measurements and still have open repairs.
Two Windows checks and one Unix filename check need runtime evidence.

### Eight more open findings

| Case | Observed result | Repair and acceptance check |
| --- | --- | --- |
| 65. History identifies a repository by its Git directory path | A rule joined `git_root_repo` to the commit of a completed run. Moving the checkout removed the result. An unrelated checkout at its old path returned the original commit. | Give stored history a durable clone identity. Keep a separate worktree identity for requests that depend on HEAD. Repeat both moves and check the commit in each result. |
| 66. Changing the rule library can select yesterday's policy | I inspected one local policy with two different `RERAIL_RR_LIB` directories. Each provided a different `audit-lib.rr`. The second inspection still reported the first directory. A fresh process with the warm store did the same. An empty store selected the second directory. | Include the library search root in the process and durable program keys. Check rule diagnostics and input paths in both orders. Keep the first directory readable throughout the test. |
| 67. Failed revisions register named scopes | Three invalid revision scans left three routes and three generations, with zero completed runs. After a successful scan in another store, deleting its name required choosing between the two stores. | Resolve the revision before acquisition. Retire a failed scan's new route when no published owner or other pending acquisition needs it. Repeat with a concurrent successful scan. |
| 68. Directory generations survive a stop | I published one run under each of 30 directory scopes. A single parent stop removed all 30 runs. It left 30 generation rows and one receipt. | Retire unused directory generations once the receipt covers earlier writers. Check an old pending writer, a later watch, and repeated use of one-time directories. |
| 69. A directory stop scans unrelated history | I executed the history deletion from the stop trigger against its two input tables. With zero or 1,000 unrelated named scopes, it deleted the same one run. The SQLite VM executed 47 and 5,048 instructions respectively. The plan scanned `analysis_history_scope` in both cases. | Index the mapping from stopped directory to owned scopes, or use indexed directory ranges. Check descendants, home boundaries and the query plan with unrelated names. |
| 70. A completed native run can change its warning | `finish(true)` returned null and the publication was `published`. After `warn("late warning")`, a second finish returned that warning for the same published run. | Seal the warning together with the publication result. Repeated finishes must return the first result. A warning recorded before publication must still prevent it. |
| 71. Existing cache ownership can be invisible to rules | An older addon evaluated a Python policy under `legacy`. After upgrade, SQLite had five manifest memberships. A current policy querying `analysis_named_scope("legacy", _)` returned no rows. The old store had no completed run records. | Derive scope presence from live ownership as well as completed runs. Keep unavailable scan IDs absent. Check rule-visible presence before and after deletion. |
| 73. A collector can acquire the wrong lock after a directory moves | A reader held the original store's shared lease. I opened a collector, moved the containing directory, and put another database at the old path. The collector obtained an exclusive lease while the original reader still held its lease. The fixture checked lock acquisition only. | Bind the lock file and verified database to the same directory identity. Abort or defer collection if the pair changes during acquisition. Check the active reader before and after the move, then release it and retry. |

[Repository keys](../../../../native/src/git_snapshot.rs) · [Policy reader](../../../../native/src/code_rule_dsl/logic/rr.rs) · [Scope acquisition](../../../../src/cli/headless-scan.ts) · [Store publication](../../../../native/src/content_store/analysis_scope.rs) · [Stop trigger](../../../../native/src/content_store/schema.rs) · [Public scope facts](../../../rule-syntax/lib/analysis.rr)

For case 65, publish a scan of checkout A under the name `old`. Use this policy for three subsequent scans in the same store.
Keep the query policy outside both checkouts. Give its own scans a different scope.

```rr
#include "analysis.rr"
#include "git.rr"
.policy "history" revision 1
.violation Seen(at: span, id: symbol) "{id}"
Seen(@place("x", 1), c) :- git_root_repo(repo), analysis_named_scope("old", s),
    analysis_scope_run(s, r), analysis_run_commit(r, repo, c).
```

| Location of the query | Finding |
| --- | --- |
| Original checkout A | A's commit |
| Same checkout after a rename | Zero findings |
| Unrelated checkout B at A's former location | A's commit |

For case 66, put different definitions of the same include file in two copies of the rule library.
Keep the policy file and both library copies in place. Inspect the policy through `LoadedRrPolicies.load` and read the included path from `programInputs`.

| Library requested | Process and store | Include reported |
| --- | --- | --- |
| A | First inspection | A |
| B | Existing process and warm store | A |
| B | Fresh process and warm store | A |
| B | Fresh process and empty store | B |

For case 73, extend the Unix store fixture `collection_keeps_the_database_it_verified_after_its_path_changes`.
Keep the reader and shared lease alive before opening the collector.

```rust
let reader = ContentStore::open(&file).unwrap();
let _reading = reader.hold_collection().unwrap();
let id = reader.identity();
```

After the fixture creates the replacement, check `collector.try_collect().unwrap().is_none()`.
At this commit the assertion fails. The existing fixture confirms that the database connection still points to the original database.
The exclusive lock uses the current pathname. [Collection lease](../../../../native/src/content_store/collection_guard.rs) · [Store fixture](../../../../native/src/content_store/corpus_runs/tests.rs)

### Thirteen earlier findings still open

| Case | Current gap | Repair and acceptance check |
| --- | --- | --- |
| 23 | Restoring an older control database can lose a committed stop. Three memberships survived the recorded fixture. | Make committed release routes and cutoffs recoverable. Restore on both sides of commit and delivery. Check that rollback releases nothing. |
| 44 | A readable checkout with no clone ID fails its first scan when its Git directory is read-only. | Store the initial identity in the Rerail home when necessary. Reopen the checkout and make it writable. Check that its identity stays stable. |
| 45 | A bare repository and its linked worktree receive different clone IDs. | Identify the common Git directory, reconcile existing IDs, and check both discovery orders. |
| 46 | A filesystem scan accepts a case variant such as `SRC/A.PY` and can return zero findings for a file with a violation. | Resolve selected source paths to inventory spelling on the host volume. Compare findings on case-sensitive and case-insensitive volumes. |
| 11 | A revision scan fails on a 32,000,001-byte NUL-filled file selected by a broad policy. The filesystem scan skips it. | Classify from a bounded Git blob prefix. Compare large text, large binary and unreadable inputs across providers. |
| 28 | The nullable native source read conflates absence, size limit and object failure. | Publish the checked outcomes to source consumers. Remove and restore a tracked blob on the same view. |
| 47 | The revision report groups excluded symlinks with absent paths. | Preserve entry mode and exclusion reason. Select a symlink, gitlink and missing path together. |
| 61 | A query for one scope read and hashed 65 history records after 64 unrelated publications. | Add a scope-keyed history request and observation. Verify a cache hit after another scope publishes or disappears. Preserve explicit enumeration. |
| 17 | A failing legacy-store probe retries after five seconds without a stored diagnosis. | Persist its reason and retry count. Use capped backoff and keep the original stop cutoff through recovery. |
| 42 | Maintenance has no way to retire a permanently removed store path. | Retire the route after resolving owners and deliveries. Keep temporarily offline stores eligible. |
| 18 | A long-lived Git view retains decoded entries from each subtree it visited. | Bound the decoded buffer. Move through disjoint selections and measure retained bytes and decodes. |
| 52 | Inspecting a Git-backed policy repeatedly reads and lowers its include graph. | Cache the lowered program by blob, resolved includes and engine version. Count compilations after restart and after one include changes. |
| 53 | A fresh catalog still pays to open and check the selected store. A repeated inspection in one catalog took 0.073 ms median. A fresh catalog took 1.677 ms in the paired run. | Profile store opening and query costs. Compare same-store and alternating-store loads, including after collection. |

The [earlier fixtures](#status-at-6e3f1618e) and [history query](#an-unrelated-scope-changes-the-input-to-a-history-query-case-61) give the original test conditions.

### Repairs verified since the run history audit

| Cases | Change and evidence |
| --- | --- |
| 13, 59, 62 | `61e46aa9e` binds the evaluated source to its publication. `d9bc3616f` fences earlier legacy writers. `2e307808c` delivers a stop to its recorded directory after replacement or disappearance. Existing stores lack the original acquisition cutoff for older receipts. Upgrade fences outstanding legacy writers at the acquisition count during upgrade. |
| 14, 15, 41 | `2e25c3719` routes named deletion to the registered store. `91a8fe16e` registers named stores for collection. `0136dcf40` retired 100 deleted names' generations and rejected an old permit after name reuse. Case 67 covers acquisitions that never published. |
| 29, 30, 60 | `ee40d56d5` reports the scan scope and separates publication, prior release and collection outcomes. |
| Cache architecture | The guard found two independent catalog freshness checks and an unclassified lease table. `3d37f7262` uses the shared program reader and identifies pending runs as leases. All six architecture and combined-program checks passed. |
| 63. Collection of a replacement file | A scheduled collector added 94 tables to an unrelated SQLite file and changed its mode from 0644 to 0600. `31d317e69` verifies the existing database and pins that connection during collection. The regression checks the unrelated file's bytes and mode, rejects a different Rerail store, and restores the original. Older path-only registrations can validate a store schema but cannot identify the original database. |
| 64. History without a readable store | A readable store supplied one history finding. When access failed, the history policy silently returned zero and the scan completed. `448b10170` reports a policy failure, preserves independent findings, and reads the original history after access returns. |
| 72. Case of the reported scope directory | On a case-insensitive volume the JSON scope was `../PROJECT/PKG`; SQLite owned `project/pkg`. `183e7f6b1` reports the stored spelling. Case 46 concerns selected source files. |

Windows execution of the NTFS rewrite and Git message-selector comparisons is still pending.
The filesystem for the macOS tests rejected the invalid-byte directory fixture during setup. It needs a byte-preserving Unix filesystem.

The final headless, Git and CLI usage suites passed all 158 tests. The store gate passed 262 tests with four skipped.
All six cache architecture and combined-program checks passed. The four standalone revision command tests and TypeScript checking passed.
Some earlier runs exceeded the first native scan's five-second timeout after relinking. They passed on repeat with the same timeout.
Measure native loading and scan execution separately on a cold process to locate that variance.

## Run history audit from `757d01c18`

The fixtures used the macOS addon and executable built from the checkout with the unfinished run history changes.
The history reader and its rule library were local changes during this audit.

There are 22 known implementation findings. Eighteen come from the [previous audit](#status-at-c1093d9a5), including case 13 below.
Cases 59–62 are additional findings. Two Windows checks still need execution results. A separate directory-name check needs a filesystem that accepts arbitrary filename bytes.

### What a rule can recover about a completed run (case 13)

I created two commits and evaluated this policy three times against the second commit.
Each native call used the same store and named scope. Each run had a distinct scan ID.

```rr
#include "git.rr"
.policy "source" revision 1
.violation Seen(at: span, oid: symbol) "{oid}"
Seen(@place("git", 1), id) :- git_source_commit(_, id).
```

| Source declared on the run | Finding from the policy | Commit in SQLite | Result from a second policy joining the scan ID to its commit |
| --- | --- | --- | --- |
| Omitted, as existing native callers do | Second commit | NULL | No row |
| First commit | Second commit | First commit | First commit |
| Second commit | Second commit | Second commit | Second commit |

All three calls to `finish(true)` returned `null`. The second policy used `analysis_named_scope`, `analysis_scope_run`, `analysis_run_scan_id` and `analysis_run_commit`.
The binding accepts a run and a revision view separately. It checks which store owns the run.
It can then evaluate a different revision from the one the run records.
Bind the actual source when evaluation acquires its owner. Reject a conflicting source before evaluation. Check both workspace attachment methods and the explicit policy API.

The standalone `logic` command also differs by source. A rule reading the three scan IDs returned three rows in both modes.
The filesystem command exited 0 with three records remaining. The revision command exited 0 with four records remaining.
Acquire the directory owner for working-tree `logic` and use the same publication boundary as a revision invocation.

The draft records a Git tree or a directory path. A directory path cannot identify which stored workspace snapshot supplied a run.
Record the run-to-snapshot relation when publishing staged snapshots. Check the join after reopening the store and after deleting one of two scopes with shared data.
Keep existing owners on schema upgrade. An earlier store has no completed scan IDs to backfill.
[Native policy evaluation](../../../../native/src/lib.rs) · [Workspace attachment](../../../../native/src/code_rule_dsl/workspace/api.rs) · [Standalone invocation](../../../../native/src/code_rule_dsl/tools.rs)

### A stopped directory can regain an owner (cases 59 and 60)

Two directory runs evaluated the same policy against the same store. Each found the repository and staged a manifest.
The first used the older native constructor without a home. The second specified home `fixture` and generation 5.
I delivered the numbered stop `10:receipt:fixture` before either call to `finish(true)`.

| Point in the test | Completed runs | Published manifest memberships | Return from finish |
| --- | --- | --- | --- |
| Immediately after the stop | 0 | 0 | — |
| After finishing the run with a home | 0 | 0 | `null` |
| After finishing the run without a home | 1 | 1 | `null` |

A control test first published a run without a home. The same stop removed it. A new run at generation 11 with the matching home could publish.
I repeated the manifest check with the addon from the previous selector tests. It also returned `null` for both calls and republished one manifest.

- **59. Late publication after a stop.** The release removes existing directory owners without a home. Its eligibility check misses an earlier pending owner with the same directory. Record which store acquisitions the release covers. Apply the same cutoff to published owners and writers that finish later. Check descendants, a different home, a new acquisition and duplicate delivery.
- **60. A caller cannot tell whether publication occurred.** Both calls returned the same value despite different durable outcomes. Return a publication outcome from the transaction and carry it into the report. Distinguish a successful analysis whose scope was released from a failure to write the store. Test a stop after evaluation and before publication.

Case 59 also affects the manifest that the collector uses. Case 60 concerns publication. The older case 30 concerns a separate failure during subsequent collection.
[Publication permit and scope release](../../../../native/src/content_store/analysis_scope.rs) · [Binding result](../../../../native/src/code_rule_dsl/workspace/api.rs)

### An unrelated scope changes the input to a history query (case 61)

I published one run in `release`, followed by 64 runs in `other`. The query for `release` still needs one row.
The draft history reader's SQL returned one row before the additions and 65 afterward.
`EXPLAIN QUERY PLAN` showed a scan of the run table and a primary-key lookup of each scope.
The reader hashes all returned rows. A change under `other` therefore changes the observation used to validate the `release` query.
This fixture measured input rows. It did not profile source parsing.

Give rules a scope-keyed history request and observe the rows that answer that request.
Keep an explicit way to enumerate all scopes. Check that a second scope can publish or disappear without invalidating the first scope's query.
A change inside the requested scope must still invalidate it. Check both answers and the cache hit.

### A queued stop deletes another checkout's ownership (case 62)

This fixture used the previous selector addon and the existing JavaScript release worker.
Two separate checkouts, `A` and `B`, each registered the same store and published one manifest membership.
I committed a stop for `A` while it was an ordinary directory. Before delivery, I renamed `A` and put a symlink to `B` at its former path.

| | Memberships under A | Memberships under B | Queued releases |
| --- | ---: | ---: | ---: |
| After committing the stop | 1 | 1 | 1 |
| After delivery | 1 | 0 | 0 |
| Expected after delivery | 0 | 1 | 0 |

The worker resolves the queued path against the live filesystem before looking up store routes.
The native release also canonicalizes the path before choosing ownership rows.
Capture the directory scope key when queuing the stop. Use that exact key for route lookup, delivery and acknowledgement.
Repeat the test with the path missing, replaced by a symlink and reused for a new watch. The new watch must keep its later ownership.
[Release worker](../../../../src/engine/native/analysis-scope.ts) · [Directory release](../../../../native/src/content_store/analysis_scope.rs)

### Directory names that require byte identity

The native directory key uses `to_string_lossy()` on the canonical path. An invalid UTF-8 byte and a real replacement character can then produce the same key.
I attempted the two-directory fixture on macOS. The volume rejected the invalid byte with `EILSEQ` during setup, before Rerail ran.
Repeat on Linux with valid UTF-8 symlink names that point to the two byte paths. Publish one run through each link and delete one scope.
Use a lossless scope key or reject an unsupported canonical name before accepting ownership. [Scope key](../../../../native/src/content_store/analysis_scope.rs)

Five existing headless regressions passed against the rebuilt addon during this audit. They cover shared snapshot ownership, collection warnings, cross-revision reuse, language classification and parser version changes.

## Status at `c1093d9a5`

Three more cases have repairs. Eighteen implementation findings remain, including the partial performance repair for case 53.
Cases 21 and 22 also need Windows runtime results.

| Case | Change | Evidence |
| --- | --- | --- |
| 58 | `963d9473f` gives a complete working-tree batch one directory owner. Both packed and individual policies use it. | Two packed policies found four identifiers. After an edit, the batch found six. Adding a broken policy at either end returned all six and preserved every previous directory membership, including the acquisition and run numbers. Removing the broken policy published the new memberships. The nested batch test also aborted the outer owner and found zero published manifests. |
| 34 | `c1093d9a5` resolves the previous branch before applying its reflog suffix. | Git, the native source API and the rule agree on `@{-1}@{0}`, `@{-1}@{1}`, and `@{-1}@{0}^`. Switching back from a detached checkout makes those selectors fail on all three paths. |
| 35 | `c1093d9a5` checks whether both range endpoints resolve before selecting a tree path that contains two dots. | From the fixture root, Git and Rerail reject `HEAD:./src/..` and select the parent tree for `HEAD:./src/../`. From inside `src`, both accept the first spelling. A directory named `two..dots` still resolves. |

The remaining cases are 11, 13–15, 17–18, 23, 28–30, 41–42, 44–47 and 52–53.
The [scope fixture](#scope-membership-at-publication) shows why public run membership still needs a change to the store.
The [repair list](#repairs-and-remaining-work) gives the other actions and their affected operations.

The 31 targeted Rust store and Git tests passed. The final executable and engine run passed eight tests, including all three executable regressions.
The full TypeScript typecheck passed after `e4915918f` made the restore test check that the release table contains a receipt.
The old addon failed four comparisons in the new selector fixture. The corrected addon passed it in isolation and in the headless suite.
The first full run passed 146 of 147 tests. The cold checkout test exceeded its five-second timeout.
The repeat passed all 147 with the same addon and timeout.

## Status at `680694436`

Twenty-one implementation findings remain open, including one partial performance repair. Two additional checks need Windows runtime results.
The numbers in this document identify follow-up cases. The original audit has its own numbering.

### Working-tree batch publication (case 58)

I ran `rerail-engine run-policies` with a Python identifier policy and a second policy with no violation declaration.
Each comparison started with a fresh store. The working tree and its commit contained the same assignment.

| Source and policy order | Exit | Findings | Published manifest memberships | Pending runs |
| --- | --- | --- | --- | --- |
| Working tree, valid then invalid | 1 | 1 | 2 under the directory | 0 |
| Working tree, invalid then valid | 1 | 1 | 2 under the directory | 0 |
| Commit, valid then invalid | 1 | 1 | 0 | 0 |
| Working tree, valid policy alone | 0 | 1 | 2 under the directory | 0 |

I also started with a successful working-tree run. After I edited its source, a failed batch added two more manifests under the same directory.
The report kept the two findings from the edited file. The existing test checks published membership only for the named revision scope. [Executable regression](../../../../native/tests/git_revision_cli.rs)

Use the [policy fixture below](#three-new-reproductions) and a fresh store to repeat the working-tree check.

```sh
RERAIL_TAPE_CACHE="$out/store.sqlite" nice -n 19 rerail-engine run-policies "$repo" "$out/batch" good.rr bad.rr
sqlite3 "$out/store.sqlite" "SELECT scope, count(*) FROM analysis_scope_manifest GROUP BY scope"
```

The standalone working-tree batch lets its individual corpus evaluations acquire directory ownership.
Stage the packed and individual evaluations under one directory run. Publish once the batch succeeds.
In both policy orders, verify that failure preserves the earlier memberships exactly. Then remove the invalid policy and verify that the new memberships appear. [Batch runner](../../../../native/src/code_rule_dsl/tools.rs) · [Run publication](../../../../native/src/content_store/analysis_scope.rs)

### Repairs and remaining work

| Cases | Result checked |
| --- | --- |
| 55–56 | `65a745ab2` keeps findings and reports each failing policy. Both policy orders pass in working-tree and revision mode. The revision commands return exit 1 on incomplete file coverage and recover after the missing subtree returns. A metadata-only policy can still succeed. |
| 57 | `37af0408e` looks up the same inspected Git policy through equivalent path spellings. Tests cover two policy revisions in both load orders, directory suffixes and a root escape. |
| 53 (partial) | `680694436` checks the selected store's recorded inputs and compiled image on a process-cache hit. The collector test retains the input record, evicts the compiled program, then verifies that another read restores a loadable program. |

The paired macOS measurements used ten warmups and forty measured reads per case.
A fresh catalog in the same store went from 2.225 ms to 1.872 ms median per policy inspection.
A reused catalog took 0.053 ms before and 0.052 ms after. A fresh catalog still opens the store outside a held reading session.
In a separate run, fresh catalogs alternated between two warm stores and returned identical inspection data on all 100 reads.
The medians were 6.242 ms and 5.958 ms. The headless tests were running during that measurement.
Profile that open and its queries before changing the store handle lifetime. Test concurrent stores and collection when measuring a change.

| Work remaining | Cases |
| --- | --- |
| Persist completed run IDs and expose rule-visible scope and source relations | 13 |
| Recover committed stops, route deletion to its owners and register named-run stores for collection | 23, 14, 41 |
| Retire obsolete stores, compact generation rows and record failed probe attempts | 42, 15, 17 |
| Publish a whole working-tree batch atomically | 58 |
| Report which scope owns a scan and distinguish publication from later collection | 29, 30 |
| Support first use in a read-only repository and use one clone ID across linked worktrees | 44, 45 |
| Match Git for previous-checkout reflog suffixes and relative selectors ending in `..` | 34, 35 |
| Resolve case-insensitive selections, classify large binaries, and distinguish absent, unreadable and excluded inputs | 46, 11, 28, 47 |
| Bound decoded trees, reuse Git-backed policy compilations and reduce working-tree catalog cost | 18, 52, 53 |

The [scope publication fixture](#scope-membership-at-publication) compares distinct scan IDs with the memberships that survived publication in case 13.
For case 13, publish the scope, run ID and selected source in the same transaction as their cache ownership.
Expose those records through explicitly keyed `.rr` relations. A tree-only run supplies a tree ID and zero commit rows.
Preserve existing manifest owners on upgrade. Earlier databases discarded completed scan IDs, so their historical run membership cannot be reconstructed.
The [case descriptions and acceptance checks](#status-at-6e3f1618e) specify a repair for each older finding.
Cases 21 and 22 still need an NTFS rewrite and a selector comparison with the installed Git for Windows.

Run the standalone regressions from the checkout root with `nice -n 19 scripts/dev/test-remote -- cargo nextest run -p rerail-engine --features test-hooks --profile gate --test git_revision_cli`.
Both executable regressions passed. Eight targeted Rust program-cache tests passed.
The first run of the headless and Git helper suites passed 145 of 146 tests.
The first cold checkout scan exceeded its five-second timeout. The repeat passed all 146 with the same addon and timeout.

## Standalone command findings at `00ca25ab7`

At `00ca25ab7`, 23 implementation findings remained. Three came from new fixtures against the rebuilt standalone CLI and native addon.
Two additional checks require Windows. The full headless and Git helper suites passed 146 tests on this build.

### Three new reproductions

| Case | What the fixture returned | Repair and acceptance check |
| --- | --- | --- |
| 55. One failed policy discards a standalone batch report | The standalone `run-policies` command found two Python identifiers at `HEAD`. Adding a second policy without a violation declaration gave exit 1 and empty stdout in both orders. | Keep a result for every policy. Emit the two findings and the failing policy's path in the report. Return a failure status and abort scope publication. Test both orders through the executable. |
| 56. Partial source coverage gives a successful exit | A repository contained `a.py` and `sub/b.py`. I removed the loose tree for `sub` from the fixture. The `logic` command processed one file, reported the missing tree on stderr and exited 0. The `run-rr` command returned one finding and a `sourceFailures` entry for `sub`. It also exited 0. | Carry the evaluation outcome to the executable separately from its output. Preserve partial findings. Require a failing exit for incomplete source coverage. Restore the subtree and check both the exit status and the recovered results. A metadata-only policy that reads no files must still succeed. |
| 57. A loaded policy depends on path spelling | A native catalog loaded `src/words.rr`. Evaluating `src//words.rr` through that catalog reported “policy was not loaded”. Direct evaluation of both spellings and catalog evaluation of the original spelling each returned two findings. | Use the same normalized repository-relative key for insertion and lookup. Keep the repository and revision in the key. Test both spellings, two policy revisions and an attempted escape from the root. |

To reproduce case 55, use a fixture checkout with two Python files. Give each file one assignment, such as `x = 1`.
Save this policy as `good.rr`. In `bad.rr`, put only the line `.policy "broken" revision 1`.

```rr
#include "core.rr"
.policy "working" revision 1
.language python
.violation Seen(at: node) "identifier"
Seen(n) :- kind(n, "identifier").
```

```sh
nice -n 19 rerail-engine run-policies "$repo" "$out/control" good.rr --revision HEAD --analysis-scope review
nice -n 19 rerail-engine run-policies "$repo" "$out/batch" good.rr bad.rr --revision HEAD --analysis-scope review
```

The first command returned a JSON report with both diagnostics. The second returned zero bytes on stdout.
The batch runner returns at the first error. The executable writes a report to stdout only for a successful result. [Executable](../../../../native/src/bin/rerail-engine.rs) · [Batch and revision runners](../../../../native/src/code_rule_dsl/tools.rs)
The native catalog indexes policy versions by the caller's path string. [Native policy catalog](../../../../native/src/lib.rs)

### Scope membership at publication

Case 13 requires a durable run ID as well as public relations.
I ran an explicit Python policy three times against the same commit. Each report had a different scan ID and one finding.
The first two scans used the named scope `release`. Each left the same five manifest memberships and zero pending rows.
The third scan used `comparison` and added five memberships under that name. Deleting `release` preserved all five under `comparison`.
The named scope snapshot table stayed empty throughout this fixture.

Publication stores membership by scope and manifest. It removes the staging row that contains the scan ID.
A reader cannot recover both completed runs from those five memberships. [Store schema](../../../../native/src/content_store/schema.rs) · [Publication](../../../../native/src/content_store/analysis_scope.rs)

The repair should publish the run ID, scope and selected source in the transaction that publishes its cache ownership.
For a revision run, record the repository, tree and optional commit even when the policy only needs metadata.
Expose explicitly keyed scope-to-run and run-to-source relations in the rule library.
Resolve both scan IDs after reopening the store. Delete one scope and verify that the other still resolves its run and retains shared cache entries.

### Remaining cases by affected operation

| Operation | Open case numbers and effects |
| --- | --- |
| Rule-visible scope ownership | 13. A policy cannot join a completed run to its scope and source. |
| Recovery and deletion | 23. A control database restore can lose a committed stop. 14. Deletion can succeed against the wrong store. 41. Maintenance misses a store used only by named scans. |
| Long-lived stores | 42. Retired store paths remain registered. 15. Deleted scope names retain generation rows. 17. Failed store probes retry every five seconds without recording a reason. |
| Scan reports | 29. The report omits the retention scope. 30. Text output describes collection failure as failed retention. |
| Repository and revision selection | 44. First use fails with a read-only Git directory. 45. A bare clone and its linked worktree get different clone IDs. 34–35. Two single-revision expressions disagree with Git. |
| File selection and reading | 46. A case-insensitive filesystem accepts a spelling that the policy filter misses. 11. A large binary can fail a revision scan. 28. The nullable reader hides a missing object. 47. Excluded symlinks appear absent. |
| Cache cost | 18. A view retains decoded trees across disjoint selections. 52. Repeated Git policy inspection lowers the program again. 53. A new catalog in the same store decodes a program already in memory. |
| Standalone commands and native catalog | 55–57. Batch findings, exit status and path lookup have the failures reproduced above. |

I repeated cases 34, 35, 46, 47 and 29 with the current addon.
Git resolved `@{-1}@{0}` after a branch switch. Rerail rejected it.
Git rejected `HEAD:./src/..` as a single revision. Both tools resolved `HEAD:./src/../` to the same tree.
On the case-insensitive test volume, `SRC/A.PY` gave zero findings and `src/a.py` gave one. Both reports listed their path as selected.
One revision report put a tracked symlink and an absent file in the same skipped list. It omitted the supplied analysis scope.
The [earlier fixtures and repair checks](#status-at-6e3f1618e) supply the evidence for the other cases.
The [Windows checks](#status-at-00ca25ab7) still need an execution result for NTFS invalidation and Git message selection.

## Status at `00ca25ab7`

Four more follow-up cases had repairs at this checkpoint. The open count was twenty before the standalone command tests above. Two Windows runtime checks were pending.
The [original repair list](audit-c1b588f91.md#checks-for-the-other-gaps-in-the-original-audit) covers the numbered cases and the additional requirements separately.

| Case | Commit | Verification |
| --- | --- | --- |
| 50 | `250abffc6`, `00ca25ab7` | From a foreign Git environment, the CLI reports the selected checkout's clone ID, branch and commit. The rule resolves that checkout's HEAD and index entry. Changing the pack directory forces an existing view to reopen the repository. The rule still resolves the selected index. The async Git helpers read its blob bytes. |
| 54 | `58aadfcbd` | Both catalog load orders return the requested policy version. An equivalent new view selects it. A second checkout with the same commit ID cannot select it. A request without a source reports ambiguity when the catalog holds two Git versions. |
| 48 | `66a234496` | The report preserves a packed ref ending in byte `ff`. Passing the reported bytes to Git returns the selected commit. Its replacement-character sibling returns a different commit. A detached HEAD has neither a ref name nor raw ref bytes. |
| 49 | `30a283c16` | `--path=--alpha.py` and a path list both select the same file, report its violation and exit 3. Missing argument values still produce a usage error. |

For case 21, `6bedf57a1` adds the Windows file identity and change time to Git storage observations.
Windows exposes the change time separately from the last write time. [FILE_BASIC_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_basic_info)
The test rewrites a loose tree in place with the same length and restores its modification time.
On macOS, a reused view and a fresh view detect the damage. Separate warm and cold content stores report the same failure.
The warm scan returns its finding again after the original bytes are restored.
The Windows reader type-checked for `x86_64-pc-windows-msvc`. A runtime result on NTFS is pending.
The [package job](../../../../.github/workflows/package.yml) now records the Windows test volume type, Git build options and Node architecture.
The same job runs the Git message-selector comparison for case 22.

On macOS, the four targeted CLI and native-binding regressions passed after the view-refresh change.
The full headless and Git helper suites passed all 146 tests with the rebuilt addon.
On earlier cold starts, the first headless test exceeded its five-second timeout.
The Git workspace suite passed all 22 selected tests on the build farm.
The local store suite passed 255 tests and skipped four under its normal filter.
On the farm, full store runs failed in filesystem journal replay tests, including a serial run.
The TypeScript build typecheck passed.

## Status at `6e3f1618e`

There are 24 open implementation findings and two missing Windows results. Case numbers refer to this follow-up audit.
The twenty cases in the original audit have their own numbers and regression tests.

### Repository selection and Git facts

| Case | Evidence | Repair and acceptance check |
| --- | --- | --- |
| 50 | Reproduced again. With explicit checkout A and `GIT_DIR` pointing to B, the report contains A's source commit and B's checkout HEAD. Adding B's worktree gives “prefix not found”. | Bind discovery, rule evaluation and report metadata to the same repository. Test both environment settings with distinct HEADs and branches. |
| 48 | A packed ref with byte `ff` in its name appears as its replacement-character sibling in the report. The two branches have different tips. | Preserve ref bytes through discovery. Emit a raw name when UTF-8 decoding fails. Resolve the reported name back to its commit. |
| 44 | The first scan of a readable checkout fails when its Git directory is read-only and has no Rerail clone ID. | Resolve an existing ID first. Keep a fallback ID in the Rerail home. Check identity across restarts and when the checkout becomes writable. |
| 45 | A bare repository and its linked worktree have different Rerail IDs. | Use the common Git directory for both. Test both scan orders and reconcile existing IDs. |
| 34 | Reproduced again. After a branch switch, Git resolves `@{-1}@{0}`. Rerail rejects it. | Carry the previous checkout's ref through the reflog suffix. Compare the source and rule APIs with Git. |
| 35 | Reproduced again. With a parent commit, Git rejects `HEAD:./src/..` as a single revision. Rerail returns the root tree. | Apply Git's revision interpretation before path normalization. Compare the trailing-slash and subdirectory variants. |
| 54 | New native API fixture. Two policy commits use the same path and emit “one” and “two”. Loading both into one catalog, then explicitly requesting the first policy source, emits “two”. The same request without the catalog emits “one”. | Key a loaded policy by source identity and relative path, or reject a conflicting source. Test both load orders. The headless CLI normally creates a catalog per scan. |

### File selection

| Case | Evidence | Repair and acceptance check |
| --- | --- | --- |
| 46 | Reproduced again on a case-insensitive volume. The filesystem report selects `SRC/A.PY` and returns zero findings. The stored spelling returns one. | Resolve the name from the inventory and use that spelling for evaluation and reporting. Test both volume types. |
| 11 | A broad policy scans a 32,000,001-byte NUL-filled file alongside a text violation. The revision scan fails on the binary. The filesystem scan completes. | Classify from a bounded blob prefix. Compare large binary, text and unavailable objects across providers. |
| 28 | The native nullable source reader returns the same result for a missing path and an unavailable tracked blob. The checked file reader reports the object failure. | Provide distinct absence, oversize and object-error results to source consumers. Test removal and restoration on the same view. |
| 47 | A tracked symlink appears in the revision report's list of absent paths. | Report the exclusion reason and Git entry mode. Test an absent path, symlink and gitlink together. |
| 49 | The CLI accepts the file `--alpha.py` from a path list. Passing it to `--path` fails. | Accept `--path=--alpha.py`. Compare path, finding and exit code for both input forms. |

### Scope ownership and collection

| Case | Evidence | Repair and acceptance check |
| --- | --- | --- |
| 13 | Scope ownership exists in SQLite. An ordinary policy cannot join a run to its scope and snapshot. | Expose committed memberships through explicitly keyed relations. Test two scopes sharing a snapshot, then delete one and repeat the join. |
| 23 | Restoring the control database after a committed stop can lose the release queue. Three memberships survived the fixture. | Persist the stop's routes and cutoff with recoverable commit evidence. Test commit, rollback, crash and restore around delivery. |
| 14 | Named deletion succeeds against the currently configured store even when a different store still owns the scope. | Locate the registered owners and report which stores acknowledged deletion. Test an override change and a repeated delete. |
| 41 | A custom store used only by named scans disappears from maintenance when the override changes. | Register named-run stores on acquisition. Verify collection after restart and after deleting the last owner. |
| 42 | The maintenance registry retains a path for every recorded directory store. It has no retirement operation. | Retire a permanently removed store once its owners and deliveries are resolved. Keep temporarily offline stores eligible. |
| 15 | Deleted one-use scope names keep a generation row indefinitely. | Compact generations after older writers lose their permits. Test delayed publication, name reuse and restart. |
| 17 | A permanently failing legacy-store probe retries after five seconds. Its row omits the failure reason. | Keep the error and retry count, use capped backoff, and retain the stop cutoff through recovery. |
| 29 | A saved revision report identifies the selection scope as `repository`. It omits the named analysis scope that owns the results. | Add the retention scope kind and name. Test the same revision in two scopes and check exported directory reports for local path disclosure. |
| 30 | After a collector failure, text output says the cache was not retained. The named manifest is still present. | Report publication and collection separately. Inject each failure and compare the message with actual ownership. |

### Cache cost and platform evidence

| Case | Evidence | Repair and acceptance check |
| --- | --- | --- |
| 18 | A long-lived native view keeps previously decoded trees as its path selection moves. | Bound the decoded tree buffer. Walk disjoint subtrees and measure retained bytes and decode counts. |
| 52 | New. A Git-backed policy takes the native include reader and lowering path on every inspection. Thirty warm loads of one policy took a median 2.39 ms through Git and 0.045 ms from the worktree in one catalog. | Retain the lowered policy by its blob, resolved includes and engine inputs. Reopen the store and count compilations. Change one include and verify the new diagnostic. |
| 53 | New. A new catalog reading an unchanged local policy in the same store took a median 2.76 ms, up from 0.057 ms before the cross-store repair. The hit validates and decodes a durable program the process already holds. | Validate the input record and check that the selected store has the program without decoding a second copy. Measure both same-store and cross-store loads, including after collection. |
| 21 | Windows verification pending. A same-size NTFS rewrite with restored modification time can preserve all fields currently used for the object stamp. | Run the mutation on NTFS. Compare warm and cold diagnostics for the damaged object, then restore its bytes and repeat. |
| 22 | Windows verification pending. The Git message-selector suite has a macOS result and a Windows workflow step. | Run the packaged addon against Git for Windows. Record versions, accepted object IDs and rejected expressions. |

The timings measure inspection only. Each path had three warmup reads and thirty measured reads of the same policy on macOS.
The performance fixtures returned identical inspection data on each repeat.
The Windows cases describe missing evidence. Neither has a reproduced Windows failure.

### What changed in this pass

Commit `6e3f1618e` closes cases 43 and 51 for working-tree policies in the per-project store configuration.
A policy inspected in store A ran against store B. A separate process reopened B and returned findings with zero source-tape parses and zero Git tree decodes.
The control scope owned at least one memo row exclusively. The row survived scope deletion and disappeared during collection. Rows for shared plans survived.
A separate test edits an include between policy inspection and evaluation. The pinned evaluation returns the original message. A new inspection returns the edited message.

All 140 headless and Git helper tests passed. Seven targeted Rust tests passed under Cargo. The TypeScript build typecheck passed.
The collection test also passed with explicit checks for shared rows before and after collection.
The package workflow and the full native gate were outside this run.
An older run hit `ENOTEMPTY` during fixture cleanup. It did not recur in these runs and its cause remains unknown.

## Open findings at `0c5fb730a`

Sixteen of the previous 25 findings remain open. Their original fixtures and repair checks appear under the case numbers below.

| Cases | What remains |
| --- | --- |
| 11, 18, 28 | A large binary fails a revision scan. A view retains decoded trees without a bound. The nullable source reader hides object failures. |
| 13–15, 17, 23 | Rules lack scope membership. Named deletion depends on the configured store. Generation rows accumulate. Failed probes retry without a reason. Restoring the control database can lose a committed stop. |
| 29–30, 41–42 | Reports omit retention scope. Text output misreports a collector failure. Maintenance misses named-only custom stores and retains obsolete store paths. |
| 34–35 | Previous-checkout reflog suffixes and a terminal `..` in a relative tree selector disagree with Git. Both differences recurred in this audit. |
| 21–22 | A same-size rewrite on NTFS and the selector comparison with Git for Windows still need Windows results. |

Cases 19 and 36–37 have repairs in `944998813`. Native selection, membership, listing, and checked file reads now accept the same relative path spellings.
The store tests check equivalent selections against the same inventory key and generation.
Four targeted headless tests passed with the rebuilt addon. The reference-edge comparison also passed under Cargo after the fixture stopped requesting a signing key.

Eight additional implementation and interface findings follow. Case 51 concerns the evidence for collection.

| Case | What the check showed | Repair and acceptance check |
| --- | --- | --- |
| 43. Cross-store policy parsing | After store A warmed the process, a scan published its plans to store B. A fresh process reopening B parsed seven unchanged rule inputs, including the policy itself and two reads of `core.rr`. There were zero Python parses and zero Git tree decodes. | On a process-cache hit, make the compiled policy and its include graph available to the consuming store. Check the graph and program independently. Repeat the A-to-B test after restarting and count parses by input digest. |
| 44. Read-only Git directory | Git resolved `HEAD` in a readable checkout with no clone-ID file. With a read-only `.git`, a revision scan failed creating `rerail-repository-id.*.tmp`. | Look up the existing clone ID first. When writing is unavailable, keep a local identity in the Rerail home. Test the first scan and restart with a read-only repository. |
| 45. Bare clone identity | A bare repository and its linked worktree produced different repository keys for the same commit. The bare root acquired both `.rerail/repository-id` and `rerail-repository-id`. | Discover the Git common directory independently of the worktree top level. Reconcile existing IDs and check both scan orders in a repository without a remote. |
| 46. Case-insensitive path selection | On a case-insensitive volume, selecting `SRC/A.PY` listed it as selected in the filesystem report but found zero identifiers. Selecting the stored spelling `src/a.py` found one. The Git report skipped the uppercase spelling. | Resolve requested names against the source inventory. Pass the selected spelling to both the filter and the report. Repeat on case-sensitive and case-insensitive volumes. |
| 47. Reason for a skipped path | With a tracked symlink `alias.py`, a revision report put the link in `skippedPaths`, the documented list of absent paths. The filesystem scan returned the identifier from its target. | Report an excluded Git entry with its mode and a reason. Preserve Git's rule for symlinks and gitlinks. Check a symlink, a gitlink and a genuinely absent path in one report. |
| 48. Non-UTF-8 branch name | A packed ref ended in byte `ff`. The report named its U+FFFD sibling. Resolving the reported ref with Git returned a different commit from the reported HEAD. | Read the reference name as bytes. Emit text after strict decoding or preserve the raw name in a byte field. Compare both refs and their different commits. |
| 49. Path starting with two dashes | For the tracked file `--alpha.py`, literal `--path --alpha.py` failed with “requires a value”. A path list selected it successfully. | Accept an unambiguous value form such as `--path=--alpha.py`. Compare the selected path, finding, and exit status with the path-list form. |
| 50. Mixed repository provenance | A scan explicitly selected checkout A. With `GIT_DIR` set to B's Git directory, the completed report identified A's source commit and B's checkout HEAD. Setting `GIT_WORK_TREE` to B as well failed with “prefix not found”. | Resolve the explicitly selected repository once. Use that identity for source selection, discovery and checkout metadata. Test both environment settings through the CLI and rule interface. |
| 51. Collection evidence | The reused-view test requests collection and checks the surviving plans. The collector can skip or bound a pass. The test has no discarded control that proves eviction happened. | Force collection pressure. Check that a released control plan disappears and a shared plan survives. Then reopen the store and check parse counts. |

Two full headless and Git-helper runs each passed 138 of 139 tests. Case 43 failed both times.
The first run parsed five rule inputs. After priming another store, the full run and the isolated test each reported seven parses.
The five inputs in the first run were `core.rr`, `source-language.rr`, `python/env.rr`, `python/modules.rr`, and `sources.rr`.
The phase trace on reopening the store showed no saved include graph for either the policy or `sources.rr`.
The store suite passed 255 tests and skipped four. These results use the macOS development addon with test hooks.
Product builds can use a shared rule-image store. This audit measured the per-project store configuration.

The [scan report](../../../../src/cli/headless-scan.ts), [argument reader](../../../../src/cli/audit-cli.ts), and [repository identity](../../../../src/engine/catalog/repository-identity.ts) supply cases 44–50.
The [cache regression](../../../../test/cli/headless-scan.test.ts) supplies cases 43 and 51.

### Repair order and choices

1. Bind each scan to one repository. Check the source ID, checkout HEAD, branch bytes, and clone identity in the same fixture.
2. Define which committed stop records survive a restore. A generation alone cannot identify the stores and directories that a lost stop covered.
3. Publish the include graph and compiled policy to each consuming store. Force eviction of a released control before measuring reuse.
4. Expose committed run membership through relations such as `analysis_scope_run(scope, run)` and `analysis_run_snapshot(run, snapshot)`. These are proposed names.
   A rule can join those rows on explicit keys. Keep pending work private until publication unless a policy needs its own pending run.
5. Make selection and reporting agree on each analyzed file. Keep the excluded Git mode available as a skip reason.

## Repairs after `8a328d0a0`

Case 40 now has a regression that compares each retained policy and plan in the same store.
The old addon lost the root and inventory program memberships after scope deletion. The corrected addon passes with a reused view and a fresh view.
The test covers abort and reuse in a second store. The isolated run found the Python identifiers with zero parses and zero tree decodes.
Cases 43 and 51 above describe the limits of that result.

Cases 38 and 39 now pass for both signature roles, including a message search through the commit.
The fixture compares the subject and dates with Git and checks both header positions through the rule library.
A second experiment populated a store with decoder version 3. Version 4 returned the typed date and the raw signature from that checkout.

Cases 31 and 32 now reject a direct or tagged commit with a local root of the wrong kind.
The walk reports missing trees and wrong-kind targets. Only verified trees produce directory placements, including both paths to one shared subtree.

For case 33, filesystem and revision scans now return the same finding for `src/../src/a.py`.
The report preserves the requested and skipped spellings. Both providers reject absolute paths and escapes.

## Status at `8a328d0a0`

Of the twenty cases open at `8a490c35e`, seven now have repairs.

| Cases | Commit | Verification |
| --- | --- | --- |
| 4, 16 | `9488e0fe2`, `95f74b5b6` | Stop delivery waits for the registered database identity. Tests replace the database at its address and register a newer owner during delivery. A directory with 4,096 unrelated registrations releases only the matching routes. Collection can still visit the released store. |
| 12 | `8a328d0a0` | Twelve 257-object packs required 149–152 warm index probes, down from 780. Findings and tree decodes stayed identical. A separate benchmark measured construction and lookup over 252,499 index entries. [Timings and memory](analysis-scopes.md) |
| 24, 25 | `46e310793` | Malformed releases preserve both owner and receipt rows. Both native generation entry points reject fractions, nonfinite values, and integers outside the supported range. |
| 26, 27 | `bf45851a5` | A selector that follows a malformed edge rejects the target kind. Both scan providers skip a regular file selected with a trailing slash or slash-dot. Direct commit selection and native reads have separate gaps in cases 31 and 37. |

Commit `e2b3f68a9` also lets Git message searches continue past a malformed loose ref. This extends case 11 of the *original* audit.
At `8a328d0a0`, 135 headless and Git helper tests passed. The store suite passed 254 tests and skipped four.
The selector comparisons used Apple Git 2.50.1. The earlier intermittent `ENOTEMPTY` teardown failure did not recur in this run. Its cause remains unverified.

Thirteen earlier cases remain open. Their fixtures and acceptance checks appear below.

| Cases | Remaining work |
| --- | --- |
| 11, 18, 19 | Match binary detection across providers. Bound decoded trees retained by a view. Canonicalize the native path-set cache key. |
| 13–15, 17, 23 | Expose run and scope membership to rules. Locate a named scope for deletion. Retire old generations. Record failed probes and back off. Recover committed stops after restoring the control database. |
| 28–30 | Distinguish native read failures from absence. Report the retention scope. Identify collection failures correctly in text output. |
| 21–22 | Check object invalidation on NTFS and run the message-selector comparisons against Git for Windows. Both lack a Windows result. |

The additional cases below bring the open count to 25. Case 42 follows from the implementation. Cases 31–41 have macOS fixtures.
The two Windows items above are verification gaps. The audit has no observed stale NTFS result or Git for Windows mismatch.

### Additional results at `8a328d0a0`

| # | What happened | Repair and acceptance check |
| --- | --- | --- |
| 31 | A commit recorded another commit as its root tree. A metadata policy completed with one finding. The report's `source.tree` was a commit ID. Peeling the same input with `^{tree}` reported the wrong kind. | Check the kind of a locally available root when selecting a commit directly or through a tag. Keep metadata-only scans working when the root object is absent. Test both cases. |
| 32 | A rule requested a tree walk on the selected **commit**. It received `git_tree_directory(repo, commit, "", commit)`. The library seeds directory placement from the request before establishing the object's kind. | Keep a separate traversal frontier for error reporting. Emit public directory placements for verified trees. Test a commit, a missing child, and two placements of one valid tree. |
| 33 | `--path src/a.py` found a Python call on both providers. With `src/../src/a.py`, the filesystem report listed the path as selected and found zero calls. The revision report skipped it. An absolute spelling also appeared as selected in the filesystem report. | Normalize repository-relative inputs once before lookup and filtering. Reject an absolute path or an escape. Keep original spellings in the report. Assert the call finding through both providers. |
| 34 | Git resolved `@{-1}@{0}` and `@{-1}@{1}` after a branch switch. Rerail rejected the second suffix as unconsumed input. | Carry the previous checkout's reference into the reflog lookup. Compare IDs from the rule relation and source selector with Git on attached and detached checkouts. |
| 35 | With a parent commit, Git rejected `HEAD:./src/..` as a single revision. Rerail selected the root tree. Git accepted `HEAD:./src/../`, and both returned the root tree. | Check Git's expression interpretation before path normalization. Compare accepted and rejected spellings with `git rev-parse --verify` from the root and a subdirectory. |
| 36 | The native `readFile("src/a.py", -1)` returned all five bytes. A limit of 4,294,967,296 returned null on that file. JavaScript values reach an unsigned 32-bit conversion before validation. | Validate the public byte limit before conversion. Test zero, the maximum, fractions, both out-of-range directions, and nonfinite values. |
| 37 | `containsPath("src/a.py/.")` returned false. On the same view, `readFile("src/a.py/.", 10)` returned the file. | Preserve the final directory requirement in the common source lookup. Test membership, reading and listing through all native entry points. |
| 38 | A commit had an extra header whose name contained byte `ff`. In an isolated repository, `git fsck --strict` succeeded and `git show -s --format=%s` returned its subject. Rerail rejected the entire commit as malformed. | Preserve header name bytes in the object decoder. Expose a byte relation alongside text headers. Test reading the subject, tree, and raw header independently. |
| 39 | With byte `fc` in the author's name, Git still returned author time 123 and offset +0230. The rule received the raw signature and the committer, but lost the typed author time. | Emit timestamp and offset facts independently of name and email decoding. Compare both author and committer cases with Git. Keep the raw signature. |
| 40 | After scope deletion and reuse, the same Git view published three manifests for a Python rule. A fresh view published five. The same difference occurred across independent stores. All runs returned one finding and published successfully. | Attach the cached inventory's dependencies to each run and store. After deletion, compare retained plan identities and fresh-process reuse with a fresh view. Exercise an abort as well. |
| 41 | A named revision scan used a custom store. It completed without creating a maintenance registration. After removing the override, `collectAnalysisCaches` never visited that store. | Register stores used by named runs for maintenance. Test discovery after restart with the override removed, including collection after the last scope is deleted. |
| 42 | The maintenance registry retains one path per directory store. Maintenance visits every recorded path. The code has no retirement operation for a permanently removed store. Directory release routing now prunes its own rows. | Add an explicit way to retire a store after resolving its owners and pending releases. Test permanent retirement, temporarily offline stores, and repeated rotation of store paths. |

The [source view](../../../../native/src/git_snapshot.rs), [revision resolver](../../../../native/src/git_snapshot_revision.rs), and [native read interface](../../../../native/src/code_rule_dsl/workspace/api.rs) supply cases 31 and 34–37.
Cases 32 and 38–40 use the [Git rule library](../../../rule-syntax/lib/git.rr) and the [Git fact provider](../../../../native/src/code_rule_dsl/logic/plan/corpus/git.rs).
The [scan target](../../../../src/cli/headless-scan.ts) and [store registry](../../../../src/engine/native/analysis-scope.ts) supply cases 33, 41, and 42.

For case 32, run this policy with `--revision HEAD --analysis-scope walk-check` in a repository with a commit.
It reports the commit ID as the root directory.

```rr
#include "git.rr"
.policy "walk-check" revision 1
git_tree_walk_request(r, c) :- git_source_commit(r, c).
.violation Directory(at: span, id: symbol) "Directory {id}"
Directory(@place("git", 1), c) :- git_source_commit(r, c), git_tree_directory(r, c, "", c).
```

For case 40, the fixture contained one Python identifier. Each run used a new native `AnalysisScopeRun` and the same named scope.
After each successful `finish(true)`, a separate read-only SQLite process counted the retained plans.

```sql
SELECT hex(m.policy), count(*)
FROM analysis_scope_manifest a JOIN corpus_manifests m USING(manifest_id)
WHERE a.scope = 'named:repeat'
GROUP BY m.policy ORDER BY m.policy;
```

Two policy digests disappeared only on the reused view. The experiment established missing ownership. It did not measure reparsing after collection.
The first repair priorities are cases 23, 31–33, 38, and 40. They affect release delivery, selected source, rule facts, findings, and cache ownership.

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

At `8a490c35e`, the first correctness priorities were cases 4 and 23–26. The status above records their subsequent results.
