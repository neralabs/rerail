# A tool call's checkout reads cost the same at any worktree count

For the pull request of branch `JeanFrancoisGagne/main-lookup-cost`, not opened yet, head 1b8da5638. [report.html](report.html) is the same report as one page: download it and open it.

## 1. The decision

**May a steer, in two rare states, arrive one or two tool calls late for this gain?**

**We recommend:** approve it: the late steer comes at the next tool call or the one after, its finding in Log once checked, and a tool call at 120 worktrees costs about 0.1 s, not 2 s.

| Size | Risk and review | Waits on | Not verified |
|---|---|---|---|
| 4,172 lines in 28 files: 2,039 ship, 2,133 are tests and text | Careful. A code owner: two rare states steer later. | Nothing. No pull request needs to merge first. | 5 |

## 2. Intent, and the gap

| Change | Intended | Found | Now |
|---|---|---|---|
| A. Rules current per event | Rules current at every event _(our reading)_ | 11,474 statements per tool call _(a statement count)_ | Closes the gap. |
| B. A worktree's HEAD | A commit anywhere seen next walk _(our reading)_ | One Git start per worktree per walk _(a CPU profile)_ | Closes the gap. |
| C. A change's line endings | Git's line endings per change _(our reading)_ | Three Git starts per pass _(a CPU profile)_ | Closes the gap. |
| D. Steer under its edit | A steer under its edit _(code comment)_ | 0 of 5 Writes on time _(a probe count)_ | Closes the gap. |
| E. A remade worktree watched | Changes in a remade worktree seen _(code comment)_ | One file in five recorded _(a probe count)_ | Closes the gap. |

Found on 11 November 2026 on a Windows 11 laptop with 190 worktrees; counted on 18 November on Linux x64.

<details><summary>Sources at main, and causes</summary>

| Change | Where main states the intent | The cause, and what was measured |
|---|---|---|
| A. Rules current per event | `src/engine/code-rule/checkout-code-rule-router.ts` lines 111 to 113 at `main`: `refreshCodeRules` calls `catalogs.refresh(root)` for every checkout the session knows. `src/engine/code-rule/code-rule-dsl-catalog.ts` line 390 and lines 471 to 496: each refresh reads the row, `checkoutStatus`, `parentCheckouts` and the signature. | `parentCheckouts` (`repository-registry.ts` lines 398 to 403 at `main`) loads every recorded checkout of the repository and filters them in JavaScript, once per checkout the session knows, at each of the several events of a tool call. |
| B. A worktree's HEAD | `src/engine/code-rule/worktree-walks.ts` at `main`: `#readCheckouts` runs `gitHead(root)` for each watched worktree at each walk; `src/engine/code-rule/code-rule-bridge.ts` does the same for the session's checkouts at each pass. | Each `git rev-parse HEAD` is a fork of the whole daemon process, whose cost grows with its memory: one per watched worktree every walk. |
| C. A change's line endings | `src/engine/code-rule/code-rule-bridge.ts` at `main`: `readGitView` and `#recoverPreimages` call `gitCrlfPaths`, which starts `git config` twice and `git check-attr` for every pass that finds a change. | `gitCrlfPaths` keeps nothing: the settings and the attributes are asked again at each pass, though no configuration or attributes file changed. |
| D. Steer under its edit | `src/runtime/runtime.ts`, the comment on `HOOK_EDIT_CHECK_MS`; the evaluator's pump in `code-rule-enforcer.ts` took queued edits in the order they came. | The walk's 46 `log` findings were queued first, and the pump checked them first. |
| E. A remade worktree watched | `src/engine/code-rule/worktree-notices.ts` at `main`, the comment on `REWATCH_MS` and `#open`. | No walk is asked for while the worktree has no watch: read calls ask for none, no notice comes, and an unwatched read gets no reread timer. |

</details>

## 3. Where to look

<img src="map.png" width="544" alt="The changed files as bars sized by changed lines, grouped by change and coloured by risk. Code to read: A, One read for a session&#x27;s checkouts, medium risk, 758 lines; B, HEAD read from Git&#x27;s files, medium risk, 561 lines; C, Line endings from Git&#x27;s files, medium risk, 290 lines; D, Tool call&#x27;s edit checked first, low risk, 316 lines; E, Worktree made again watched at once, low risk, 114 lines. Read first: repository-registry.ts, checkout-code-rule-router.ts, git-head-reader.ts. To skim: 2,130 lines of tests and 3 lines of one release note and one docs paragraph.">

1. **Read first**, 10 min: `repository-registry.ts`: `recordedCheckoutsReader`, `CheckoutsReader.read`, `#question`, `stateOf`. Can a recorded checkout move without the recorded checkouts' version moving?
2. **Read first**, 8 min: `checkout-code-rule-router.ts`: `refreshCodeRules`, `#rulesChanged`, `#lane`, `#openLane`, `interrupt`. Can a checkout's auditor audit with rules older than its inputs?
3. **Read first**, 10 min: `git-head-reader.ts`: `gitHeadReader`, `vouchedBy`, `keptAnswer`, `configReader`. Can the files name a commit Git would not name?
4. 6 min: `git-crlf-reader.ts`, `git.ts`.
5. 5 min: `code-rule-dsl-catalog.ts`, `rule-configuration.ts`, `stable-json.ts`.
6. 3 min: `code-rule-bridge.ts`, `worktree-walks.ts`, `runtime.ts`, `code-rule-enforcer.ts`, `code-rule.ts` (`PendingEdits`), `worktree-notices.ts` (`#waits`, `#remade`).

**Then skim**, 10 min: Eight test files: the cost case counts statements per tool call, the contract case compares the reader with the per-checkout readers, the reader cases count Git starts.

## 4. Risk against verification

<img src="grid.png" width="544" alt="The changes by risk, high on top, against six checks. A, One read, medium risk: independent review does not hold yet, users lose nothing holds with a limit, rules scan: nothing new holds with a limit. B, HEAD reader, medium risk: independent review does not hold yet, users lose nothing holds with a limit. C, Line endings, medium risk: independent review does not hold yet, users lose nothing holds with a limit. D, Order, low risk: independent review holds with a limit. E, Rewatch, low risk: users lose nothing holds with a limit, rules scan: nothing new holds with a limit. CI at this head does not hold yet for every change.">

Start at change A: a stale answer there applies the wrong rules to an edit.

<details><summary>Each limit, and each check that does not hold yet</summary>

| Change | Check | Its limit, or why it does not hold yet |
|---|---|---|
| A. One read | independent review | No second reading of this head from a separate checkout. |
| A. One read | users lose nothing | 45 Write tool calls through a real daemon and real hooks gave `main`'s answers and records on Linux x64 and Windows 11, one coverage record sooner. The limit: at 6bb4ae3d6, before `main` was merged in. |
| A. One read | rules scan: nothing new | On Windows 11 the cost case passes 5 runs of 5 at the head and fails 5 of 5 with the fix's first line out. The registry's lines ran on Linux x64 only on the engine. |
| B. HEAD reader | independent review | No second reading of this head from a separate checkout. |
| B. HEAD reader | users lose nothing | The same 45 Write tool calls gave `main`'s answers with and without the reader. The limit: at 6bb4ae3d6. |
| C. Line endings | independent review | No second reading of this head from a separate checkout. |
| C. Line endings | users lose nothing | The same 45 Write tool calls gave `main`'s answers. The limit: at 6bb4ae3d6. |
| D. Order | independent review | On the 0.2.0 engine, beside another session, the Write was steered in its own answer 19 of 20 times here: 4 of 5 on Linux x64 after a nested checkout moved, 5 of 5 after a worktree was deleted and added again, 10 of 10 on Windows 11; `main` 20 of 20. The late one came a call later. |
| E. Rewatch | users lose nothing | A folder made again that the file system gives the device, file number and birth time of the one removed waits 5 s, as on `main`. |
| E. Rewatch | rules scan: nothing new | Each new line taken out fails a case in 10 runs of 10 on Linux x64, and in groups on Windows 11; one survives: the birth time in a folder's identity. |
| A to E | CI at this head | CI runs on the pull request. |

</details>

<details><summary><b>5. The changes</b></summary>

**A. One read for a session's checkouts**, medium risk. One tool call at 120 worktrees: 1,542 to 2,125 ms of daemon CPU on `main`, 42 to 186 ms here.

- **Problem:** At every hook event the daemon re-read every checkout the session knows, each with every recorded checkout of its repository, so one event cost the worktrees known times the checkouts recorded.
- **Who meets it:** Everyone with many worktrees, at every hook event of every session there, on `main` with the 0.2.0 engine as before it.
- **What it does:** `recordedCheckoutsReader` reads every checkout a session asks about, with its parents and start state, in one prepared statement. `refreshCodeRules` has each checkout's auditor read its rules when its inputs moved: the session checkout's at each event, every auditor's when a signature or setting changed, an auditor's at its opening and at the session's end.
- **Risk:** Medium: a version that misses a change would leave an auditor on old rules until its next input moves.
- **Check:** Whether any write to `checkouts` or `repositories` leaves the recorded checkouts' version as it was, and whether an auditor can audit before it read its rules after an input moved.
- **Picture:** Daemon CPU per Bash tool call at 120 worktrees, the median of five runs, on main with the 0.2.0 engine and with this change: Windows 11 2,006 ms against 109 ms; Linux x64 1,626 ms against 88 ms.
- **Method:** A real daemon from the built CLI and the built Claude Code hooks, one repository of 120 worktrees set up first from a home of its own, 5 runs of 5 Bash tool calls, `main` 7efcac9c6 and the head in one probe per machine.

**B. HEAD read from Git's files**, medium risk. Three walks of the other worktrees: 10 Git starts for their `HEAD` through `gitHead`, 0 through this reader.

- **Problem:** With the lookups gone, the walks' `git rev-parse HEAD`, one per watched worktree per walk, held the daemon's thread: each a fork of a process of several gigabytes.
- **Who meets it:** Everyone with many worktrees: 17.8% of the daemon's thread at 190 worktrees in a CPU profile, Linux x64.
- **What it does:** `gitHeadReader` reads `HEAD`, the branch's ref and `packed-refs` from Git's files. Git is asked at a checkout's first read and whenever its `.git` entry, a configuration file Git reads, an owner Git checks or the object and ref folders changed; between, the files' answer stands where Git's last answer named the same commit.
- **Risk:** Medium: an answer from the files where Git names another commit would attribute a change to the wrong commit.
- **Check:** Whether a file Git reads for `HEAD` is missing from the key, and whether any path makes the reader answer without Git having vouched.
- **Picture:** Git processes started for HEAD over three walks of the other worktrees in the walk case, Linux x64: 10 with the walk's read through gitHead, 0 through this reader.
- **Method:** The walk case counts `gitHead` calls and `rev-parse HEAD` processes over three walks after a first one, at the head and with the walk's call put back to `gitHead`, on Linux x64.

**C. Line endings from Git's files**, medium risk. A pass that finds a change: 3 line-ending Git starts through `gitCrlfPaths`, 0 through this reader.

- **Problem:** Each pass that found a change asked Git for the line-ending settings and attributes again: `git config` twice and `git check-attr`, though no file they come from changed.
- **Who meets it:** Everyone with many worktrees and busy sessions: about 15% of the daemon's thread once `HEAD` came from files, Linux x64.
- **What it does:** `gitCrlfReader` keeps the settings by the configuration files' stamps and each path's attributes by every attributes file Git reads, asking `check-attr` only for paths whose key moved, and none where no attributes file exists.
- **Risk:** Medium: a kept answer that misses an attributes file would compare a change under the wrong line endings.
- **Check:** Whether every file Git reads attributes from is in a path's key, `attr.tree` and `GIT_ATTR_SOURCE` included.
- **Picture:** Git processes started for line endings at each pass that finds a change in the bridge case, Linux x64: 3 through gitCrlfPaths, 0 through this reader.
- **Method:** The bridge case counts `check-attr`, `core.autocrlf` and `core.eol` Git processes at the second and third passes, at the head and with the call put back to `gitCrlfPaths`, on Linux x64.

**D. Tool call's edit checked first**, low risk. After a nested checkout moved, beside another session: on time 4 or 5 in 5 on the engine; `main` always.

- **Problem:** A walk of the other worktrees queues what it found as `log` edits, which steer no session. A nested checkout moved inside a worktree is about 46 of them, and the agent's next Write was answered after all.
- **Who meets it:** A session whose agent writes in a worktree just after a folder of files changed there, now that walks reach a change about a second sooner.
- **What it does:** Where no rule of the checkout reads other files, the edit that may steer is checked first. Where one does, it is checked with the log edits ahead of it in one engine call. Each revision publishes once its edits are checked, and a tool call's result waits for the edits that may steer.
- **Risk:** Low: every edit is checked and recorded, and the Log lists audits in the order edits came; where a rule reads other files, a log edit is checked with the batch's texts held.
- **Check:** Whether a rule the cross-file flag misses reads another file the workspace holds.
- **Picture:** A Write into a nested checkout just moved inside a worktree, beside another session, through a real daemon and the built Claude Code hooks on the 0.2.0 engine: steered in its own answer 5 of 5 times at main and 4 of 5 here on Linux x64; 5 of 5 and 5 of 5 on Windows 11.
- **Method:** A probe through a real daemon and the built Claude Code hooks, five fresh worktrees, each with a nested checkout renamed and then a Write into it while a second session is open: `main` beside the head on one Linux x64 machine at once, in turn on one Windows 11 machine.

**E. Worktree made again watched at once**, low risk. A file written in a worktree deleted and added again: recorded 1 of 5 on `main`, 5 of 5 now.

- **Problem:** A worktree deleted and added again lost its watch with the old folder, and `main` waited 5 s to watch it again. A file written after the walk read the new folder waited for some later tool call's walk.
- **Who meets it:** A session whose repository has a worktree removed and added again at the same place while the daemon runs.
- **What it does:** Each watch keeps the identity of the folder it watches; when it ended and another folder stands at the path, the next walk watches the worktree before it reads it.
- **Risk:** Low: one awaited `lstat` as a watch opens, and one per walk while an ended watch waits its 5 s; the same folder, or none, still waits 5 s.
- **Check:** Whether a folder being moved back and forth could now have its watch end on every walk.
- **Picture:** A file written straight to disk in a worktree deleted and added again, through a real daemon and the built Claude Code hooks on Linux x64: recorded by the walk 1 of 5 times on main, 5 of 5 at this change.
- **Method:** A probe through a real daemon and the built Claude Code hooks, five fresh worktrees per state, each deleted (or a folder above it removed) and added again, then a file written straight to disk, on Linux x64, at `main` and at the head side by side on one host.

</details>

<details><summary><b>6. The decisions</b></summary>

**When does a checkout's auditor read its rules again?**

**We chose** when an input moved: the recorded checkouts' version, a signature row, or its own rules folder at each event. **Would change our mind:** a change Rerail records that moves neither the recorded checkouts' version nor a signature row.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| One statement per event, rules read when their inputs moved **(chosen)** | `main`'s answers at 6 statements an event. | A version read per event. | Nothing measured. |
| Keep `main`'s reads, each made cheaper | Less per read. | An index. | Still grows with the worktrees known. |
| Read only the checkouts a tool call touches | Fewer reads. | Nothing. | A setting made elsewhere reaches another worktree's auditor only at its next use: a rule told later. |

**Evidence.** The contract case compares every answer of the one-statement reader with `checkoutStatus` and `parentCheckouts` over 200 random trees and 30 steps of records, starts and stops, on Linux x64 and Windows 11.

**When may a reader answer from Git's files?**

**We chose** only where Git's last answer named the same commit under the same inputs, all at least 2 s old. **Would change our mind:** a file Git reads for a checkout's `HEAD` or attributes that the key leaves out.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| Git's answer vouches for the files, kept by their stamps **(chosen)** | Git's answer, from the files while they hold. | A `stat` of each file Git reads. | Nothing measured. |
| Ask Git each walk, as `main` does | Git's answer. | One Git start per worktree per walk. | The daemon's thread, at 190 worktrees. |
| Read the files alone | No Git start. | Nothing. | A file Git cannot parse, `safe.directory` or a refused branch name gives a commit Git would not name. |

**Evidence.** Each case of `git-head-reader.test.ts` and `git-crlf-reader.test.ts` compares the reader with Git in the same folder after each kind of change, and counts Git starts.

</details>

<details><summary><b>7. How it fits</b></summary>

Nothing has to merge first: it holds `main` at 88446abff (#865 and #880 in), the 0.2.0 engine. #812 conflicts in the router and its test, clean with `main` alone; #881 and #894 merge clean; #883 and #884 share no file.

This pull request holds main at 88446abff, the 0.2.0 engine, and needs nothing else. #881 and #894 share files with it and merge clean; #883 and #884 share none; #812 conflicts with it in the router and its test, and merges clean with main alone.

**Trial merges with this head.** Read from a local clone on 19 November. `main` 88446abff merged in with no conflict and no edit: #865 keeps the calls of a file the session folder's walk could not read, a walk this change leaves as it was. #812 at da345bfc3: the router, the evaluator, `runtime.ts` and two tests; conflicts in the router and its test. #881 at 78a38570b: the bridge and its test, clean. #894 at 5596af86d: `runtime.ts`, clean. #883 and #884: no shared file.

</details>

<details><summary><b>8. What is not covered</b>: 5 not verified, 4 limits that stay</summary>

1. **A worktree's files changed at once** beside another session, a Write there was on time 19 of 20 times on the 0.2.0 engine, `main` 20 of 20; the late one a call later.
2. **One session, a worktree removed** late on `main` too: its walk's first sight of the worktree (#907). One probe: 1 of 152 late here, 3 of 152 on `main`.
3. **A worktree walked while gone, then added again** a file written there waits for an unrelated walk, here as on `main`.
4. **The 10-minute load at this head** did not run. At 0.1.5's code against this change's code then: CPU 43% against 96 to 98% of a core.
5. **macOS** never ran.

**Limits that stay**

- A rule file written into a worktree's own `.rerail/rules`, then a Write there, beside another session: on time 19 of 20 here, 20 of 20 on `main`; the late one a call later, its finding logged at once.
- A rule that another worktree's rules file leaves unrunnable is recorded as skipped at that worktree's next walk or tool call, or at the session's end, where `main` records it at the next event. Every check runs with current rules.
- Each catalog build still serializes every rule with its mode, as on `main`: each checkout's rule set hands it a copy of its own.
- After a failed `git var`, the HEAD reader asks Git for 60 s before it tries Git's file list again.

</details>

## 9. What we ask of you

We ask no ruling.

<details><summary><b>10. Evidence</b></summary>

The description is the record: "How you tested it" holds the failing cases, the counts and the runs.

| Where | What ran | Result |
|---|---|---|
| Linux x64 | 120 worktrees, 5 runs of 5 Bash tool calls through a real daemon, at `main` and here, on the 0.2.0 engine | Daemon CPU per tool call 1,542 to 2,028 ms against 42 to 186 ms; hook median 1,182 ms against 370 ms. |
| Windows 11 | The same | 1,931 to 2,125 ms against 94 to 125 ms; hook median 1,188 against 283 ms. |
| Linux x64 | The same at b95422be7 beside `main` 90075faa3, five probes on three machines | 1,072 to 2,422 ms against 24 to 242 ms; rule-breaking Writes steered in their own answer 27 of 28 here, 28 of 28 on `main`. |
| Linux x64 and Windows 11 | The 78 test files that import a changed module at this head, against the 75 of them at `main` 88446abff | The 78 test files that import a changed module fail on Linux x64 only as `main`'s do, none here only, and on Windows 11 only as `main`'s do once the 7 files that held 23 cases failing on a loaded machine ran again; #865's 16 cases pass at both; CI's lint and type-check pass. |
| Linux x64 and Windows 11 | The changed cases at the head, and with the fix's lines out | Head: every case passes. Lines out: each of three fails in every run; the partway check's case 20 of 20. |
| Linux x64 | One session: 75 runs of a worktree added, written to and removed, then a Write in another, beside `main` 88446abff | Late 1 of 152 rule-breaking Writes here, after 96 log checks of the removed worktree's files; 3 of 152 on `main`, two of them after 109 such checks. |
| Linux x64 and Windows 11 | A Write after a worktree's files changed at once, beside another session | On time 19 of 20 here, 20 of 20 on `main`; the late one a tool call later. |
| Linux x64 and Windows 11 | A Write after a rule file was written into the worktree's own rules, beside another session | On time 19 of 20 here, 20 of 20 on `main`; the late one a tool call later, its finding in Log at once. |

**Sources.** The branch at 1cef26483, read on 19 November 2026; `main` at 88446abff, cccb25a8d, 90075faa3, 7efcac9c6 and ba0545900.

</details>
