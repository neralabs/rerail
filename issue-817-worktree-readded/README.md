# A worktree made again is read as soon as Git lists it

For [pull request #909](https://github.com/neralabs/rerail/pull/909), head 91af14122. [report.html](report.html) is the same report as one page: download it and open it.

## 1. The decision

**May the walk of a repository's other worktrees watch the folder where Git registers them, so that a change Git makes there starts a walk?**

**We recommend:** approve it: a file written in a worktree made again reaches the Log within seconds where it waited for an unrelated change, and the watch starts no Git process of its own.

| Size | Risk and review | Waits on | Not verified |
|---|---|---|---|
| 908 lines in 16 files: 412 ship, 496 are tests and text | Careful. It adds one watched folder per repository. | Nothing. No CI run on GitHub yet: not pushed. | 5 |

## 2. Intent, and the gap

| Change | Intended | Found | Now |
|---|---|---|---|
| Made again, read again | A write reaches the Log within seconds _(docs)_ | Waited for an unrelated walk _(a traced daemon)_ | Closes the gap. |

Found in November 2026 on Linux x64, tracing a daemon's walks across a worktree removed and added again.

<details><summary>Sources at main, and causes</summary>

| Change | Where main states the intent | The cause, and what was measured |
|---|---|---|
| Made again, read again | `docs/rule-scoping.md` at 6b3990ba8, the paragraph on walking the other worktrees. `src/engine/code-rule/worktree-walks.ts` at 6b3990ba8, `#forget`: `this.#notices.retain(roots)`. | The walk's list leaves out a path with no folder, so the walk lets go of the worktree; nothing watches Git's registry, so the add asks for no walk. With no walk between, the walk after the add read the new folder but would not watch it within 5 s of the old watch ending, and asked for no later walk. |

</details>

## 3. Where to look

1. **Read first**, 10 min: `worktree-notices.ts`: `watchRegistry`, `registryNoticeCounts`, `#registryNotice`, `folderAt`, `#endedAt`. Can Git's own work in a worktree ask for walks, or a folder renamed away and back be chased?
2. 5 min: `worktree-walks.ts`, `checkout-code-rule-router.ts`, `code-rule.ts`, `code-rule-bridge.ts`, `tree_watch/inotify.rs`, `tree_watch/mod.rs`, `tree-watch.ts`, `native.ts`: where the registry is named, watched and gives way.

**Then skim**, 5 min: The tests: a scripted watch for the notices, and a real repository through the bridge for the walks.

## 4. Risk against verification

<img src="grid.png" width="544" alt="The changes by risk, high on top, against six checks. Made again, low risk: users lose nothing holds with a limit, Windows 11: no new failure holds with a limit. CI at this head holds with a limit for every change.">

One change, low risk: one more watched folder per repository; what a walk reads and records stays as on `main`.

<details><summary>Each half mark's limit</summary>

| Change | Check | It holds with this limit |
|---|---|---|
| Made again | users lose nothing | Every worktree is watched as on `main` near the Linux watch budget, shown at the engine's seam only: the registry watch takes only spare room and gives it up. On Windows 11 a lock-file cycle in a registry entry takes about 0.25 ms more. |
| Made again | Windows 11: no new failure | Windows 11: the 24 changed and importing test files pass at this head (749) and at `main` 6b3990ba8 (736), none failed. Five states, 5 runs at 1e7110240: here 5 of 5 in every state. One other run's 35 s hook stall is unexplained. |
| Made again | CI at this head | No CI run on GitHub yet: the branch is not pushed. On Linux x64 CI's lint and type-check job passes at this head; the build and integration job (23 files, 91 tests) at 1e7110240. |

</details>

## 5. The changes

**Worktrees made again, read again**, low risk: Written after a walk while the folder was gone: `main` 6b3990ba8 0 of 3 runs, here 3 of 3.

<details><summary><b>6. The decisions</b></summary>

**What tells the walks that Git lists a worktree again?**

**We chose** a watch of the folder where Git registers the worktrees: any add, remove, move, repair or prune changes it. **Would change our mind:** A Git that rewrites an entry's `gitdir` or `locked` file on ordinary commands: each would ask for a walk.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| Watch Git's registry folder **(chosen)** | A worktree read and watched again as soon as Git lists it, with or without a walk while it was gone. | One watch per repository; on Linux one per entry. | A repository's first linked worktree, added while a session runs: no registry folder yet, as on `main`. |
| Watch the folder above a gone worktree | The same, while one is gone. | On Windows the engine watches that folder's whole tree, every other worktree with it. | Every write in every worktree comes as a notice of it. |
| Read the registry at each tool call | The same at the next tool call. | A read of every entry per tool call. | A file written while the session is idle waits. |

**Evidence.** Linux x64, five states, `main` and here side by side: every state records both files here in every run (3 runs at this head, 5 at 1e7110240). The watch gave 26 notices in one traced run, one per add and one for `git worktree prune`, none from the walks' own Git processes.

**When is a worktree whose watch ended watched again?**

**We chose** at once when another folder stands at its path (device, file number, birth time); the same folder waits as before. **Would change our mind:** A file system that gives a folder made again the same file number and birth time.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| Folder identity **(chosen)** | A folder made again is watched as the next walk reads it. | One `lstat` per watch start. | Nothing found. |
| End every wait when Git's registry changes | The same, mostly. | None. | Notices of two roots come in no fixed order, so the wait may begin after the registry's notice. |
| Drop the 5 s wait | Every ended watch watched at once. | None. | A folder being renamed back and forth is chased at every walk. |

**Evidence.** With no walk between, `main` 6b3990ba8 recorded a file written 3 s after the add in 0 of 3 runs on Linux x64; here 3 of 3. On ext4 a folder made at once after another often takes its file number: the birth time tells them apart, and taking it out fails a case on Linux x64.

**Where does the registry watch take its room on Linux?**

**We chose** only room the watch budget has left, folders above it included, given up to a worktree that needs it. **Would change our mind:** A user whose engine runs at its budget and who would rather have the registry watched than the budget kept.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| The budget's spare room **(chosen)** | Every worktree watched as on `main`; the registry while room is left. | A room check per watch call, the same comparison as before. | At the budget's edge the registry is not watched, as on `main`. |
| Outside the budget | The registry watched at the edge too. | Up to one inotify watch per registered worktree past the engine's share. | Another program of the user may find no inotify watch left. |
| After the walk's worktrees | The same at the first walk. | None. | A worktree that grows at a later walk finds the registry in its room and is not watched. |

**Evidence.** Cases with a budget sized so the last worktree just fits without the registry: it fits here too and the registry's watch ends; with room for the registry but not the folder above it, the registry is not watched; a spare root ends no other. Each fails with its line taken out.

</details>

<details><summary><b>7. How it fits</b></summary>

Nothing has to merge first: it stands alone on `main` 6b3990ba8, which it has merged. It shares files with #812, whose trial merge is clean, and with #904, where one file and its test file conflict.

**The files it shares.** #812 edits `checkout-code-rule-router.ts`, its test file and `native.ts`; the trial merge is clean. #883 shares no file. #904 edits `code-rule-bridge.ts`, the router and `worktree-walks.ts`, which merge clean, and `worktree-notices.ts` and its test file, which conflict: the one that lands second merges `main` and keeps both. #865, #879, #881 and #887, merged today, edit files this one edits; Git merged them cleanly and they compose.

</details>

<details><summary><b>8. What is not covered</b>: 5 not verified, 1 limit that stays</summary>

1. **One Windows 11 run** at 1e7110240: two Read tool calls took about 35 s and the late file was missed. The cause is unknown; four more such runs recorded every file.
2. **macOS** never ran.
3. **A repository's first linked worktree** added while a session runs waits for the next walk, as on `main`: no registry folder exists yet.
4. **A folder put back by another program** moved away and back with `mv`: Git's list does not change, so a write there waits for the 10-minute reread, as on `main`.
5. **The Linux watch budget's edge** an engine at its budget does not watch the registry, as on `main`; the budget is shown at the engine's seam only.

**Limits that stay**

- The walk this change asks for still waits for the 5 s gap after the session's last walk.

</details>

## 9. What we ask of you

We ask no ruling.

<details><summary><b>10. Evidence</b></summary>

The description on GitHub is the record: "How you tested it" holds the runs on `main` and here.

| Where | What ran | Result |
|---|---|---|
| Linux x64 | The built Claude Code hook against a daemon of `main` and of this branch, five states, one machine | `main` 6b3990ba8 beside this head, 3 runs each: `main` walked 0 of 3, the folder above 1 of 3, another branch 1 of 3; no walk between, late file 0 of 3. Here 3 of 3 in every state. Earlier, 5 runs each: `main` cccb25a8d and ba0545900 2 or 3 of 5 in the walk-between states; 1e7110240 5 of 5. |
| Windows 11 | The same probe at 1e7110240 and `main` cccb25a8d on one machine, 5 runs each | `main` cccb25a8d: 1 to 4 of 5 in the walk-between states, late file 3 or 4 of 5 with no walk between. Here 5 of 5 in every state, each with the engine built for its own commit. One other run stalled 35 s, cause unknown. |
| Linux x64 and Windows 11 | The 24 changed and importing test files, this head and `main` 6b3990ba8 on one machine | Linux: here 745 passed and one `audit-history.test.ts` case at its 5 s limit; `main` 733 passed. That file alone, five runs each: the first two fail at both. Windows, each with its own engine: here 749 passed, `main` 736, none failed. |
| Linux x64 | The spare root's cases, 10 runs in a row, at 1e7110240 | 10 of 10 runs passed: the spare root's nine cases, each run with its build. |
| Linux x64 | Each line of the change taken out alone | 20 of 20 fail a case on Linux x64; an earlier head's 28 each failed one, and 12 of the 28 ran as Windows 11 builds, 11 failing their case (birth time aside). |
| Linux x64 and Windows 11 | Daemon CPU per Bash tool call at 48 worktrees; Git's lock-file cycle in a registry entry | Linux, 12 alternating pairs: medians 402 ms at `main` cccb25a8d and 408 at 1e7110240, higher in 6 and lower in 6. Windows, 6 pairs of 300 cycles: median 153 ms unwatched, 229 ms watched. |
| Linux x64 | CI's lint and type-check job, the engine's format check, clippy and tests | At this head: lint and type-check, `check-rustfmt.mjs`, clippy with `-D warnings`, the executors guard test and the tree watch's 30 cases, each exit 0. The build and integration job at 1e7110240: 23 files, 91 tests passed. |
| Linux x64 | The automated reviewer's rules at `main` 6b3990ba8 over the diff against it | No new finding and no new complexity, with the published 0.1.5 and with Rerail built from `main`. |

**Sources.** The branch at 91af14122 and its diff against `main` 6b3990ba8, read on 20 November 2026; runs on Linux x64 machines and Windows 11 machines.

</details>
