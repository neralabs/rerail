# Git and scope audit at `ec9066ed9`

The original [twenty cases and their regression tests](audit-c1b588f91.md) cover specific inputs. This audit checks additional inputs, public entry points, and the collector's choice of owners. The experiments ran on macOS.

## Status at `00ca25ab7`

Four more follow-up cases have repairs. Twenty implementation findings and two Windows runtime checks remain open.
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
