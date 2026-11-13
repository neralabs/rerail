# Audits: the directory walk reads files in path order

For [pull request #635](https://github.com/neralabs/rerail/pull/635), head 83e627c0f. [report.html](report.html) is the same report as one page: download it and open it.

## 1. The decision

**Does the walk still read every file it read before, with only the order changed, including when a folder's listing fails partway?**

1. A walked repository audit lists the same failures, in the same order, on every machine.
2. The file and byte budgets stop a walk after the same files on every machine.

**We recommend:** approve it once CI is green, which needs #634 on main, and you have read the [one place marked read first](#3-where-to-look).

| Size | Lane | Waits on | Not verified |
|---|---|---|---|
| 191 lines in 5 files: 39 ship, 152 are tests and text | Fast. The automated reviewer can approve. | #634. 23 of 26 CI jobs pass until it merges. | 4 |

## 2. Intent, and the gap

| Change | Intended | Found | Now |
|---|---|---|---|
| Failure list order | Failures in path order _(a test's assertion)_ | 22 of 29 newer-image jobs failed _(CI logs, one day)_ | Closes the gap, changes answer 1. |
| Where budgets stop | Same stop point on every machine _(our reading)_ | Stop point followed the filesystem _(code, then a test)_ | Closes the gap, changes answer 2. |

Found on 12 November 2026 in CI's unit test jobs, then reproduced on Linux and on Windows 11.

<details><summary>Sources at main, and causes</summary>

| Change | Where main states the intent | The cause, and what was measured |
|---|---|---|
| Failure list order | `test/engine/code-rule/unlisted-text-files.test.ts` lines 130 to 141: the test "skip an oversize binary one through the host's directory walk, and keep the budget failure for an oversize text or listed one" expects `failures: ["data/dump.raw budget-exceeded", "web/big.ts budget-exceeded"]`. `test/engine/code-rule/code-rule-enforcer-mechanical.test.ts` line 1220, "audits a Git checkout holding unreadable untracked directories as the walk does", expects the checkout's result to equal the walk's (line 1243). | `#auditFiles` pops paths off a stack (`src/engine/code-rule/code-rule-enforcer.ts` line 1629), and `#enqueueAuditDirectory` pushed each entry as `opendir` listed it (line 1691). ext4 lists a folder by a hash seeded per filesystem, tmpfs on recent Linux kernels newest first, NTFS by name. Of the "Unit tests (1/2)" jobs of 12 November 2026 (UTC), the test failed in none of the 52 on the older runner image. Load and parallel runs changed nothing. |
| Where budgets stop | Nothing at main states it. The nearest text is a code comment on `#indexRepository`, `src/engine/code-rule/code-rule-enforcer.ts` lines 1428 and 1429: "the walk's file, byte and five-minute budgets stop the index as they stop the walk". It says which budgets apply, not which files a stopped walk has read. `docs/reference.md` line 874 names the budgets and promises no order. | `#auditFiles` counts each file against both budgets in one `if`, as the stack gives it (lines 1634 to 1637). The new test passes 512 MiB with 17 hard links to one file of 31,999,000 bytes: with main's walk, the 16 files audited follow the listing. No test or run reaches 100,000 files. The native index stops the same way at 100,000 files (`native/src/code_rule_dsl/workspace/index.rs` lines 440 to 442) and sorts only what it listed (line 453). It is left alone. |

</details>

## 3. Where to look

1. **Read first**, 5 min: `code-rule-enforcer.ts`: `#enqueueAuditDirectory`, `pushInPathOrder`, `walkOrderKey`. Its `finally` pushes what a failed listing gave. Does every entry main pushed still reach the stack?

**Then skim**, 5 min: No existing assertion changes. Two new tests mock `opendir`. The reference promises the same failures only within budgets.

## 4. Risk against verification

<img src="grid.png" width="544" alt="The changes by risk, high on top, against six checks. Path order, medium risk: users lose nothing holds with a limit, Windows 11: no new failure holds with a limit. CI at this head holds with a limit for every change.">

Three half marks: no walk past the file budget, one Windows failure main has too, and CI.

<details><summary>Each half mark's limit</summary>

| Change | Check | It holds with this limit |
|---|---|---|
| Path order | users lose nothing | At the head against main, over 1,741 files on tmpfs and ext4: status, violations and unlisted counts are unchanged, and the walk reads the same files. Under the 20-failure cap the failures are the same set. Over it a plain folder lists what a Git checkout lists, in the same order. No walk went past its 100,000-file budget. |
| Path order | Windows 11: no new failure | `code-rule-enforcer-mechanical.test.ts` ran twice at the head on Windows 11 with Node 24.21.0: 55 of 56 passed each time. The one failure, an existing test on unreadable untracked directories, fails at main on Windows too. Both changed test files ran there at e3f866f48: 70 of 71, the same failure. The flaky test fails at main there and passes with this change. |
| Path order | CI at this head | 23 of 26 jobs pass at the head, both unit test jobs and the licence gate among them. Dependency audit fails until #634 merges, and this pull request changes no dependency. CI result sums them up. In the run's first attempt `Unit tests (2/2)` hit one timeout in `test/cli/rule-draft.test.ts`, which #636 fixes. |

</details>

## 5. The changes

**Walk in path order**, medium risk: The flaky test on tmpfs: 200 of 200 runs fail at main, none with this change.

<details><summary><b>6. The decisions</b></summary>

**Where is the order fixed: the walk, the test, or the finished list?**

**We chose** the walk: the list and the budgets both stop depending on the filesystem. **Would change our mind:** folders of hundreds of thousands of entries outside Git checkouts.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| Sort each folder's entries in the walk **(chosen)** | The same failures in the same order on every machine, as a Git checkout gives. Budgets stop after the same files | One sort per folder. Peak memory 2% to 6% higher on 50,000 to 60,000 files. No measurable time | Nothing found. A failure list's order moves. Past 20 failures, which 20 it lists moves too |
| Make the test accept either order | A green CI. Results still differ by machine | A changed assertion | It hides a difference users see |
| Sort the failure list when the audit ends | The finished list in path order, when nothing stops the walk | Every failure is held until the end, where a log audit now writes its failures page by page | A budget still stops after different files. Failures written before the end keep the filesystem's order |

Before: At main opendir lists a folder in the filesystem's order, each walkable entry goes on the stack as listed, and the walk takes the last one listed first. After: With this change the walkable entries are collected with a sort key each, sorted by UTF-8 bytes with a folder as its name and a slash, and pushed in a finally with the last in path order first. The walk takes them in path order.

**Evidence.** The byte-budget test audits the same 16 files for a folder listed in path order and in reverse. Cost, at the first commit: one repository audit outside Git, on a 32-core Linux machine, on tmpfs, main and this change taking turns, 3 runs each. 50,000 files in one folder: 16.7 to 19.7 s at main and 13.7 to 14.2 s here, peak memory 422 to 427 MB and 449 to 453 MB. The description's cost table holds two more rows.

**Which order: UTF-8 bytes, JavaScript strings, or the locale?**

**We chose** UTF-8 bytes, a folder as `name/`: Git's order and the native index's. **Would change our mind:** the native index changing its order.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| UTF-8 bytes (`Buffer.compare`), a folder as `name/` **(chosen)** | Git's order: `B/x.ts`, `a-b.ts`, `a.ts`, `a/b.ts`, `a0.ts` | A small buffer per entry | Nothing found |
| JavaScript string comparison | Git's order for most names | None | Characters above U+FFFF sort before U+E000 to U+FFFF, by UTF-16 code units |
| `localeCompare` | An order that reads well | None | It changes with the machine's locale, the problem this change removes |
| Plain names, no `/` for folders | Almost Git's order | None | Folder `a` sorts before `a-b.ts` and `a.ts`, so `a/b.ts` is read before them, where Git puts it after |

**Evidence.** The native index sorts by Rust `String` comparison, which compares UTF-8 bytes (`native/src/code_rule_dsl/workspace/index.rs` line 453). The new path-order test's 22 names cover upper case before lower case, `-` and `.` before `/`, and `0` after it. It runs through the directory walk and through a Git checkout's index and expects one list from both.

**Is a folder still audited when its listing fails partway?**

**We chose** yes, as at main: a `finally` pushes the listed entries, and the folder is reported. **Would change our mind:** a ruling that an audit stops at the first listing error.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| Sort and push what was listed, in a `finally` **(chosen)** | The entries listed before the error are audited, in path order, and the folder is reported, as at main | A `try` and `finally` around the listing | Nothing found |
| Push only after a complete listing | Only the folder's failure | The fewest lines | Files main audits go unchecked on a network or FUSE filesystem whose listing fails partway |
| Push as listed, then sort that folder's part of the stack | The same files as main | Still a key per entry, and a sort of a slice of the stack in place | Nothing. More code for the same files |

Before: At main opendir gives 2 of a folder's 5 entries, then fails with EIO. Both are already on the stack, both are audited, and the folder is reported. After: With this change the finally sorts and pushes the 2 entries the listing gave before its error. Both are audited in path order, and the folder is reported.

**Evidence.** The new test "audits the entries a folder listed before its listing failed partway, and reports the folder" mocks `opendir` so one folder's listing throws EIO after two of its five entries. It passes with main's walk on Linux, and at the head on Linux and on Windows 11. With a walk that pushes only after a complete listing it fails: the audit reads one file and leaves out the two listed ones.

</details>

<details><summary><b>7. How it fits</b></summary>

It stands on main alone. A green CI needs #634 on main. #503 edits four of its five files: one conflict, in a test's imports. Three later pull requests of ours edit `code-rule-enforcer.ts`.

**Shared files, and the pull requests to come.** #503 edits `code-rule-enforcer.ts`, both test files and `docs/reference.md`. A trial merge with #503 at 78867d0e gave one conflict, in the import lines of `code-rule-enforcer-mechanical.test.ts`. The second to land resolves it. On [tracking issue #632](https://github.com/neralabs/rerail/issues/632), orders 5, 6 and 10 hold the four waiting changes that edit `code-rule-enforcer.ts`. Orders 5 and 6 take main after this lands; order 10 follows #503. #636, opened with this one, fixes the other test that fails main's CI at random and shares no file. The description's "How it fits" has a row for each.

</details>

<details><summary><b>8. What is not covered</b>: 4 not verified, 4 smaller gaps, 4 limits that stay</summary>

1. **The 100,000-file budget's stop** has no test and no run: [ask 2](#9-what-we-ask-of-you).
2. **CI is not green** until #634 merges.
3. **Windows.** Only the two changed test files ran.
4. **macOS** never ran.

**Smaller gaps**

- **Cost at the head.** Audit time and peak memory were measured at the first commit, a0fd11bde. The third commit moves the same sort and push into a `finally` and adds no work, and the fourth changes one test file. It takes a rerun of one probe.
- **A test of the file budget's stop.** It needs 100,000 files, or the budgets passed in through the enforcer's options, which is a source change. This pull request adds neither: [ask 2](#9-what-we-ask-of-you).
- **Windows at the head.** `unlisted-text-files.test.ts` did not change in 83e627c0f and was not rerun on Windows 11. At e3f866f48 both changed test files ran there: 70 of 71.
- **The merge with #503.** One conflict, in a test file's import lines. The second to land resolves it.

**Limits that stay**

- The five-minute budget still stops a walk after different files on machines of different speed. `docs/reference.md` promises the same failures only for a scan that stays within its budgets.
- The native index still stops listing at 100,000 files in the filesystem's order, and sorts only what it listed. [Part 9](#9-what-we-ask-of-you) asks about it.
- A touched-files audit still takes the paths it is given last first, as at main. A folder among them is walked in path order.
- A folder's entries wait with a sort key each until the listing ends: about half a kilobyte per entry in a folder of 50,000, where peak memory rose 6% (3 runs each, a 32-core Linux machine).

</details>

## 9. What we ask of you

1. **Should the native index list in path order before it stops at 100,000 files?** We recommend yes, as its own pull request with a Rust test.
2. **Should a test pin the 100,000-file stop? It needs the budgets as enforcer options.** We recommend no: one `if` holds both budgets. If yes, its own pull request.

Answer 1 asks no ruling. Main's flaky test already expects path order.

Answer 2 asks no ruling. A test pins the byte budget's stop only: [ask 2](#9-what-we-ask-of-you).

<details><summary><b>10. Evidence</b></summary>

The description on GitHub is the record: "How you tested it" for the runs and the rules scan, "Decisions" for the options and the cost table.

| Where | What ran | Result |
|---|---|---|
| CI on GitHub | The pull request's run at 83e627c0f, read on 13 November 2026 | Second attempt: 23 of 26 jobs passed, both unit test jobs and the licence gate among them. 1 skipped: the Rust engine. 2 failed: `Dependency audit`, and the `CI result` that sums up. `Unit tests (1/2)` ran both changed test files, the byte-budget test in 6.5 s. The first attempt's `Unit tests (2/2)` failed on one timeout in `test/cli/rule-draft.test.ts`, which #636 fixes. |
| Linux, 32 cores | The flaky test at main, eight at a time, on tmpfs, an ext4 root and ten new ext4 filesystems, with and without load | Failed 200 of 200 on tmpfs and 24 of 24 on each of the 7 new filesystems that list `data` first. Failed 0 wherever `web` is listed first. Load changed neither result. With this change, at the first commit, the whole file ran 1,240 times on the same filesystems: 0 failed. |
| Linux, 32 cores | The new path-order test with main's walk | Failed 160 of 160 runs through the directory walk. Passed 160 of 160 through the Git checkout's index. |
| Linux, 32 cores | The new byte-budget test at 83e627c0f, then with the sort line removed, then with main's walk | Passed at 83e627c0f. Failed with the sort removed and with main's walk: with the folder listed in path order, the walk reads the last of the 17 files first and audits the last 16, where the test expects the first 16. |
| Linux, 32 cores | Three corpora of up to 1,741 files, each as a plain folder and as a Git checkout, audited at main, at 83e627c0f and at main again, on tmpfs and on ext4 | Status, violations and unlisted counts are unchanged. The walk reads the same files as main, in path order only at 83e627c0f. Under the 20-failure cap the failures are the same set. Over it a plain folder lists the same failures, in the same order, as the Git checkout. Main run twice gave the same results. |
| Linux, 32 cores | Both changed test files, at 83e627c0f, 1 run | 72 of 72. |
| Windows 11, Node 24.21.0 | `code-rule-enforcer-mechanical.test.ts` at 83e627c0f, 2 runs. Both changed test files at e3f866f48, 1 run | 55 of 56 in each run at 83e627c0f and 70 of 71 at e3f866f48. The one failure each time, "audits a Git checkout holding unreadable untracked directories as the walk does", fails at main on Windows too. |
| Linux | The rules scan as the automated reviewer runs it, at 83e627c0f against the merge base | 15 findings over the ceiling at the base and the same 15 at 83e627c0f, none new. |

**What this report adds to the description**

- **What the byte-budget test costs a test run.** It reads 17 files of 31,999,000 bytes each, hard links to one file, so it takes 32 MB of disk. At 83e627c0f it took 2.9 to 4.2 s in 22 runs on a 32-core Linux machine, 3.6 and 4.9 s in 2 runs on Windows 11, and 6.5 s in CI's run. Its time limit is 120 s. The description does not give its cost.
- **#503 moved after the trial merge.** The trial merge used #503 at 78867d0e. #503's head is now baf7a5369, one commit later. That commit edits six files and none of the four this pull request shares with #503 (its file list on GitHub, read on 13 November 2026).

**Sources.** The pull request, its diff and its CI on GitHub, read on 13 November 2026. Rerail at main (feef6b5f0) for each quoted intent. The CI logs of 12 November 2026 for how the gap was found. Our run records and [tracking issue #632](https://github.com/neralabs/rerail/issues/632).

</details>
