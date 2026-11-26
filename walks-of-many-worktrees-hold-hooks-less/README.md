# Walks of many worktrees hold hooks less, with less CPU and memory

For [pull request #1110](https://github.com/neralabs/rerail/pull/1110), head 87e71afc33. [report.html](report.html) is the same report as one page: download it and open it.

## 1. The decision

**May the daemon take each walk one worktree at a time, keep worktree places until the tree watch reports a move, and share rule copies?**

**We recommend:** approve it: with many worktrees and several sessions, a hook then waits for one worktree's audit, not a whole walk's, and the daemon asks the disk and the database far less.

| Size | Risk and review | Waits on | Not verified |
|---|---|---|---|
| 2,713 lines in 36 files: 1,098 ship, 1,615 are tests and text | Careful. It changes how every walk's findings reach a session. | Nothing. CI has not run at this head. | 4 |

## 2. Intent, and the gap

| Change | Intended | Found | Now |
|---|---|---|---|
| A. Take on the queue | A command does not wait _(docs)_ | 6.7 s hook as first pass ends _(load run, Windows 11)_ | Closes the gap. |
| B. Places on disk | A look-up per folder _(code comment)_ | 46 % of a core after warm-up _(load run, Windows 11)_ | Closes the gap. |
| C. Registry at start | The registry listed once _(code comment)_ | 4.1 s hook in the first minute _(load run, Windows 11)_ | Closes the gap. |
| D. Rule documents | A copy per build _(code comment)_ | Every rule serialized per checkout _(catalog case)_ | Closes the gap. |
| E. Rule copies | A rule placed per folder _(our reading)_ | Heap grew 9.3 MB a minute _(load run, Windows 11)_ | Closes the gap. |

Found 25 November 2026 in a 20-minute load on a Windows 11 machine with 8 processors: 190 worktrees, 17 repositories, 8 sessions.

<details><summary>Sources at main, and causes</summary>

| Change | Where main states the intent | The cause, and what was measured |
|---|---|---|
| A. Take on the queue | `docs/rule-scoping.md`, the worktree walk's paragraph: "One walk of them, shared by the repository's open sessions, runs behind their own folders' walks. A command doesn't wait for this." | `#takeWorktreeWalk` at `main` read the take and applied it in one `#enqueue`: every worktree's audit in one turn of the session's queue, which the session's hooks also wait in; every watching session read the walk at once. |
| B. Places on disk | `src/engine/code-rule/checkout-code-rule-router.ts` at `main`, `walkView`: "A path's full lookup (`#owner`) resolves its real path and looks for `.git` on disk, about 0.4 ms on Windows, and the walk asks two questions of every file." | Each folder's look-up still resolved its real path, every worktree was looked up again at every list, and each look-up read the recorded checkouts' version from the database. |
| C. Registry at start | `src/engine/catalog/repository-identity.ts` at `main`, above `worktreeRegistries`; `repository-registry.ts`, `registryLists`, which calls `worktreeRegistry` and resolves the listed worktrees for each worktree it records. | `worktreeRegistry` reads every entry of the registry before it serves the kept list, and `recordStartedWorktree` asked it for each worktree recorded: n times n reads on the daemon's thread. |
| D. Rule documents | `src/engine/stable-json.ts` at `main`, above `ruleSourceText`: "Worked out at each build: each checkout's rule set hands the catalog a copy of every rule with its mode set, so no two builds share the object." | `#effective` in `rule-configuration.ts` made `{ ...rule, mode }` for each checkout at each build. |
| E. Rule copies | `native/src/code_rule_dsl/model.rs` at `main`: `CompiledCodeRule` holds `canonical: String`, `clauses: Vec<CompiledClause>`, `capabilities: Capabilities`; `CodeRuleScope` holds `GlobSet` by value. | Placing a rule copies the compiled rule to set its placement, which copied its text, clauses, globs and capabilities too. |

</details>

## 3. Where to look

<img src="map.png" width="544" alt="The changed files as bars sized by changed lines, grouped by change and coloured by risk. Code to read: A, A walk taken worktree by worktree, medium risk, 333 lines; B, A folder&#x27;s places resolved once, medium risk, 402 lines; C, The worktree registry read once, low risk, 157 lines; D, The first walk yields, documents shared, low risk, 120 lines; E, A placed rule shares its copies, low risk, 86 lines. Read first: code-rule-bridge.ts, folder-places.ts, checkout-code-rule-router.ts. To skim: 1,614 lines of tests and 1 line of the note for the person upgrading.">

1. **Read first**, 12 min: `code-rule-bridge.ts`: `takeParts`, `splitTake`, `#applyTake`, `#applyTakePart`, `#readTake`. Does each part hold exactly its worktrees' entries, and a failed part leave the rest with their records?
2. **Read first**, 8 min: `folder-places.ts`: `worktreeRoot`, `#keep`, `#collect`, `#watched`. Can a move reach a kept place before the watch's notice ends it?
3. **Read first**, 6 min: `checkout-code-rule-router.ts`: `worktreeRootsInTurns`, `#readEligibility`, `#realBelow`, `#listedWorktreeRoots`. Is a folder taken as its own real path only below a start whose place is its path?
4. 8 min: C and B: `worktree-walks.ts`, `repository-identity.ts`, `repository-registry.ts`, `code-rule-dsl-catalog.ts`, `checkout-owner.ts`, `code-rule.ts`, `worktree-notices.ts`, `runtime.ts`.
5. 6 min: B, native: `inotify.rs`, `notices.rs`, `windows.rs`, `tree_watch/mod.rs`.
6. 5 min: D: `rule-configuration.ts`, `stable-json.ts`.
7. 5 min: E, native: `model.rs`, `code_rule_dsl/mod.rs`, `compile/mod.rs`, `rule_api.rs`, `shared.rs`.

**Then skim**, 10 min: The take cases in `worktree-walks.test.ts` hold a session's audit and count turns; `walk-lookups.test.ts` and `worktree-registry-pass.test.ts` count look-ups and reads.

## 4. Risk against verification

<img src="grid.png" width="544" alt="The changes by risk, high on top, against six checks. A, Take in parts, medium risk: independent review does not hold yet, rules scan: nothing new holds with a limit, Windows 11: no new failure holds with a limit. B, Kept places, medium risk: independent review does not hold yet, rules scan: nothing new holds with a limit, Windows 11: no new failure holds with a limit. C, Registry once, low risk: independent review does not hold yet, rules scan: nothing new holds with a limit, Windows 11: no new failure holds with a limit. D, First walk, low risk: independent review does not hold yet, rules scan: nothing new holds with a limit, Windows 11: no new failure holds with a limit. E, Rule copies, low risk: independent review does not hold yet, rules scan: nothing new holds with a limit. CI at this head holds with a limit for every change.">

Start at A and B, the two medium-risk changes; C, D and E change what is read, not what is decided.

<details><summary>Each limit, and each check that does not hold yet</summary>

| Change | Check | Its limit, or why it does not hold yet |
|---|---|---|
| A. Take in parts | independent review | No review by another reader yet. |
| A. Take in parts | rules scan: nothing new | The repository reviewer's rules at 9a40e368ae against `main` d54013c841, with Rerail built from `main`: no new finding, and no function the change adds or edits is over a complexity of 6. The published 0.2.1 could not load `main`'s own pack (rule dimensions since #1037), so that scan did not run. |
| A. Take in parts | Windows 11: no new failure | At 72b2021a6f the 74 changed and importing test files fail on Windows 11 only where `main` fails on the same machine; GitHub's runners passed the 11 changed files 24 of 24 times on Windows, 27 of 30 on Linux, whose runner stood still. |
| B. Kept places | independent review | No review by another reader yet. |
| B. Kept places | rules scan: nothing new | The repository reviewer's rules at 9a40e368ae against `main` d54013c841, with Rerail built from `main`: no new finding, and no function the change adds or edits is over a complexity of 6. The published 0.2.1 could not load `main`'s own pack (rule dimensions since #1037), so that scan did not run. |
| B. Kept places | Windows 11: no new failure | At 72b2021a6f the places' cases with the engine's watch pass on Windows 11, and a watched folder renamed or deleted is lost at the next take; GitHub's runners passed the 11 changed files 24 of 24 times on Windows, 27 of 30 on Linux, whose runner stood still. |
| C. Registry once | independent review | No review by another reader yet. |
| C. Registry once | rules scan: nothing new | The repository reviewer's rules at 9a40e368ae against `main` d54013c841, with Rerail built from `main`: no new finding, and no function the change adds or edits is over a complexity of 6. The published 0.2.1 could not load `main`'s own pack (rule dimensions since #1037), so that scan did not run. |
| C. Registry once | Windows 11: no new failure | At 72b2021a6f the 74 changed and importing test files fail on Windows 11 only where `main` fails on the same machine; GitHub's runners passed the 11 changed files 24 of 24 times on Windows, 27 of 30 on Linux, whose runner stood still. |
| D. First walk | independent review | No review by another reader yet. |
| D. First walk | rules scan: nothing new | The repository reviewer's rules at 9a40e368ae against `main` d54013c841, with Rerail built from `main`: no new finding, and no function the change adds or edits is over a complexity of 6. The published 0.2.1 could not load `main`'s own pack (rule dimensions since #1037), so that scan did not run. |
| D. First walk | Windows 11: no new failure | At 72b2021a6f the 74 changed and importing test files fail on Windows 11 only where `main` fails on the same machine; GitHub's runners passed the 11 changed files 24 of 24 times on Windows, 27 of 30 on Linux, whose runner stood still. |
| E. Rule copies | independent review | No review by another reader yet. |
| E. Rule copies | rules scan: nothing new | The repository reviewer's rules at 9a40e368ae against `main` d54013c841, with Rerail built from `main`: no new finding, and no function the change adds or edits is over a complexity of 6. The published 0.2.1 could not load `main`'s own pack (rule dimensions since #1037), so that scan did not run. |
| A to E | CI at this head | The repository's CI has not run at this head. The commit hook's checks pass at each commit. |

</details>

<details><summary><b>5. The changes</b></summary>

**A. A walk taken worktree by worktree**, medium risk. A hook at the end of a first pass waits for one worktree's audit, not every worktree's.

- **Problem:** At the end of the daemon's first pass, each watching session read the walk at once with the others and audited every worktree's findings in one turn of its queue, which its hooks waited behind.
- **Who meets it:** Anyone with many worktrees and several sessions, at each daemon start. Load run, Windows 11, 190 worktrees, 8 sessions: a 6.7 s hook as the first pass ended.
- **What it does:** `#takeWorktreeWalk` reads the take in the walk's turn (`ReadTurn`, `WorktreeWalks.#inTurn`); `#applyTake` applies `takeParts`: one part per worktree with something to audit, one for the rest; `#beginTake` lists the worktrees once; the last part keeps the observations once.
- **Risk:** Medium. Every walk's findings go through the split; a part must hold exactly its worktrees' entries, records and answered reads.
- **Check:** Does `splitTake` put every entry of a take in exactly one part? Does a failed part leave the parts after it with their records? Is a closed session's take stopped between parts?
- **Picture:** A walk that finds two of five worktrees changed: at main the session applies it in one turn of its queue, all five worktrees; here in three turns, one for each changed worktree and one for the three quiet ones.
- **Method:** The case 'is applied in one turn for the worktrees with nothing to audit' counts the take turns the walks publish, Linux x64.

**B. A folder's places resolved once**, medium risk. A walk's 12 folders: 12 real-path look-ups at `main`, none here; 50 version reads: 50, then 1.

- **Problem:** Every walk of every session looked each worktree up on disk again, and each folder's owner look-up asked the disk its real path and read every row of the recorded checkouts.
- **Who meets it:** Anyone with many worktrees, after the first pass, at every walk of every session. Load run, Windows 11: the daemon used 46 % of a core on average.
- **What it does:** `FolderPlaces` keeps a worktree top folder's place until the tree watch says its watch ended or lost notices; `walkView(root)` takes folders below the start as their own real paths (`ownerOfReal`); `recordedCheckoutsVersionReader` reads again only after `data_version` or `total_changes()` moves.
- **Risk:** Medium. A kept place is wrong if the watch misses a move: nothing is kept through a link, for a missing folder, on macOS, or below a folder the engine cannot follow.
- **Check:** Can a move reach a kept place before a notice? Does `flush` in the take report the kernel's queue in order? Is a let-go Windows root lost until reopened?
- **Picture:** A walk from the session folder over 12 folders asked the disk for 12 real paths at main and none here; 50 owner look-ups with no write between them read the recorded checkouts 50 times at main and once here.
- **Method:** The two cases of `walk-lookups.test.ts`, run over `main`'s source and here on Linux x64: real paths counted through `node:fs`, statements through SQLite's statement `get`.

**C. The worktree registry read once**, low risk. Registry files read to record 16 worktrees: 256 at `main`, 16 here.

- **Problem:** Recording each worktree asked every recorded checkout's registry again, reading every entry and resolving the listed worktrees: n worktrees cost n times n reads on the daemon's thread.
- **Who meets it:** Anyone with many worktrees, at each session start. Load run, Windows 11: a 4.1 s hook in the first minute as 8 sessions started.
- **What it does:** `WorktreeRegistryPass` reads each registry once for a read that records many worktrees; the router reads eligibility in one `refreshAll`, and a session's first list does it in groups of 16 with a turn between.
- **Risk:** Low. A pass answers what the registry held at its first question; the next pass reads it again.
- **Check:** Can a worktree removed during a pass be recorded? Is the list read before the first turn, in the learn step's run?
- **Picture:** Registry entries read to record a started clone's worktrees: 64 for 8 worktrees and 256 for 16 at main, 8 and 16 here.
- **Method:** The registry pass case, run over `main`'s source and here on Linux x64, counts reads of each registry entry's `gitdir` file.

**D. The first walk yields, documents shared**, low risk. Pack rules serialized again for a second checkout's catalog: every one at `main`, none here.

- **Problem:** Each catalog build made a new running document of every rule for every checkout, so each was serialized and hashed again, and a first walk started a wave's reads, each building a checkout's rules, at once.
- **Who meets it:** Anyone with many worktrees, at the end of the first pass. Load run, Windows 11: a 6.7 s hook as the first pass ended.
- **What it does:** `runningDocument` keeps one document per rule object and mode; `ruleSourceText` keeps a rule's text and digest by the rule; `#startReads` gives the thread back after each first-sight read starts.
- **Risk:** Low. A shared document changed in place would change every checkout's: none is.
- **Check:** Can two checkouts at different modes get one document? Is a configured rule a new object?
- **Picture:** First-sight files whose checkout was found before the walk gave the thread back: all 12 of a wave at main, 1 here.
- **Method:** The case 'gives the thread back after finding each file's checkout', run over `main`'s source and here on Linux x64.

**E. A placed rule shares its copies**, low risk. Workspaces placing one compiled rule: a copy each at `main`, one shared here.

- **Problem:** Each session's checker in another worktree registered every rule by copying the whole compiled rule only to set its placement: about 7 MB of copies per checker with four packs.
- **Who meets it:** Anyone with many worktrees and several sessions, after the first pass. Load run, Windows 11: the heap grew 9.3 MB a minute to 360 MB.
- **What it does:** `CompiledCodeRule` holds its text, clauses (`Clauses`) and capabilities behind `Arc`, and `CodeRuleScope` its globs; a copy changes only its placement.
- **Risk:** Low. Sharing is by type: a change to a copy's clauses copies them first (`Arc::make_mut`).
- **Check:** Does every change to a copy's clauses go through `push` or `DerefMut`?
- **Picture:** Two workspaces that place one compiled rule: at main each holds its own copy of the rule's text and clauses, here they share one.
- **Method:** The engine's placement cases compare the addresses of the text, clauses, globs and capabilities two workspaces hold, over `main`'s source and here on Linux x64.

</details>

<details><summary><b>6. The decisions</b></summary>

**Apply a walk's take whole, or one worktree at a time on the queue?**

**We chose** one part per worktree with something to audit, one for the rest: a hook waits for one audit. **Would change our mind:** A run where the turns themselves cost a hook more than the audits they split.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| The whole take in one turn | Nothing new. | A hook waits for every worktree's audit at the end of a first pass. | Nothing. |
| One part per audited worktree, one for the rest **(chosen)** | A hook waits for at most one worktree's audit; few turns when little changed. | A split of the take's entries, each once. | Nothing a case shows: each part records and steers what the whole take did for its worktrees. |
| One turn per worktree, quiet ones too | The same wait. | A turn per worktree at every walk. | Nothing, but each turn copies the observations unless kept for the last. |

**Evidence.** Cases count the take turns the walks publish (1, 1 and 3 for two changed of five) and hold a session's audit while its tool call begins. A failed part leaves the parts after it, with their records, to the next walk.

**Ask the disk for a worktree's place at each walk, or keep it?**

**We chose** keep it while the tree watch shows nothing moved it, on Linux and Windows only. **Would change our mind:** A move the engine's watch can miss on a supported system, shown by a case.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| Ask the disk at each look-up | No kept answer. | A real path, a stored path and an existence check per worktree per walk of each session. | Nothing. |
| Keep it until the watch's notice **(chosen)** | No disk look-up for a watched worktree's top folder. | A user of the shared watch, flags only. | Nothing kept through a link, for a missing folder, on macOS, or below a folder the engine cannot follow. |
| Keep it for a fixed time | Fewer look-ups. | None. | A moved worktree served stale until the time passes. |

**Evidence.** The places cases move, remove, replace by a link and retarget a worktree folder: each next look-up gives the disk's answer. A walk's own folders are their own real paths for that walk only.

**One running document per checkout, or one per rule and mode?**

**We chose** one per rule object and mode: nothing changes a rule document once read. **Would change our mind:** A code path that changes a rule document in place.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| A new document per checkout per build | No sharing. | Every rule serialized and hashed again per checkout per build. | Nothing. |
| One per rule object and mode **(chosen)** | Text and digest worked out once, in every runtime. | A weak map by rule. | Nothing: a configured rule is a new object with its own document. |
| Cache the text by the rule's content | The same hits. | A serialization to look the cache up. | Nothing, but it pays what it means to save. |

**Evidence.** A case gives a rule a Log setting in one worktree: each checkout runs it at its own mode, and the other rule's document is the same object in both.

</details>

<details><summary><b>7. How it fits</b></summary>

It holds `main` aee140dd91 through a merge of d54013c841 and Git's own of three later ones. #1076, #1089, #1095, #1103 and #1104 merge clean. It closes the automated reviewer's point left on #1078 (9a40e368ae, with a case). #1104 edits the router's `walkView` too and merges clean.

The change holds main at aee140dd91 through four merges. Its trial merges with main and with the open #1076, #1089, #1095, #1103 and #1104 are clean; #1104 shares the router with it.

**Shared files and the merges.** #1074, #1078 and #1051 merged since this change began; it holds them through a merge (1bd9e26497) that stopped at the imports of three files, two comments and a test clean-up, each keeping both sides. A take's parts carry #1074's first-sight listings; the grouping of a walk's changes by checkout is #1078's. Git's own merges of `main` f43850a0a7 and 80cfc21dec touched no file of it. #1089, #1103 (`runtime.ts`) and #1104 (the router) merge clean. It builds on #904, #812, #813, #852, #997, #964, #974, #1062 and #1064.

</details>

<details><summary><b>8. What is not covered</b>: 4 not verified, 2 smaller gaps, 1 limit that stays</summary>

1. **macOS** never run by us beyond the tree watch's macOS job, which passed in our fork at 75bdafb87d and runs in the pull request's CI. The places keep nothing there.
2. **Its own load** the same load of `main` with this change alone, beside `main`, is run by us on one machine alone and is not in yet.
3. **Review** no review by another reader yet.
4. **Test runs** 7 of 7 on our Windows 11 machines; on GitHub's runners 24 of 24 on Windows, 27 of 30 on Linux, three of `main`'s cases failing as that runner stood still.

**Smaller gaps**

- **Kept place within a stretch.** The watch's notices are taken once per stretch of synchronous work, so a move made after that take is answered from the kept place until the stretch ends.
- **Reads in turn.** A session whose read of a walk is slow holds the other sessions' reads of that walk until it ends or fails; it holds no session's queue.

**Limits that stay**

- After this change `main` still opens every file at the first read after a restart on Windows, and asks Git's ignore rules at each walk; both are other changes.

</details>

## 9. What we ask of you

We ask no ruling.

<details><summary><b>10. Evidence</b></summary>

The description on GitHub is the record: "How you tested it" holds the runs at `main` and here.

| Where | What ran | Result |
|---|---|---|
| Windows 11, 8 processors | A 20-minute load, 190 worktrees, 8 sessions: `main` with #1074 and #1076 beside a build holding these changes and three others | Slowest hook in the first minute 4,143 and 2,455 ms; as the first pass ended 6,696 and 962 ms; CPU mean 46 and 26 %; heap growth 9.3 and 1 MB a minute. |
| Linux x64 | The test files of 86a3500626 over `main` 70892dfacf's source, and here | 24 cases fail over `main`'s source and pass here, each on a count or an order; the two engine placement cases fail there. |
| Linux x64 | The 74 changed and importing test files at 9a40e368ae, beside `main` d54013c841 | Here 2,587 passed and at `main` 2,544; the only failures, 956 on both sides, are in `packs.test.ts`, whose fixture archives the machine lacks. Every case `main` runs in a changed file runs here. |
| Windows 11 | The 74 changed and importing test files at 72b2021a6f, beside `main` d54013c841 | Here 2,639 passed and at `main` 2,594; every failure here is one `main` has on that machine: 954 in `packs.test.ts`, 2 fixtures it lacks and one case of a share that does not answer. |
| Windows 11 | The 11 changed test files at 72b2021a6f, with the engine built for its tree | 7 runs on two machines, every one passed; a run on a hard disk and one on a busy workstation failed only at time limits; 30 did not fit. |
| GitHub's runners; Linux x64 | The 11 changed test files at 75bdafb87d, 30 runs on Linux and 24 on Windows; the three cases that failed, alone, 20 runs a side | Windows 24 of 24; Linux 27 of 30, each failure one of `main`'s own cases at its limit while cases of other files stood still too. Alone, medians at `main` and here 163 and 160, 225 and 229, 332 and 346 ms. Stopped 2.8 s early in the case, 18 of 72 failed at `main` and 22 of 72 here. |
| Linux x64 | The case of the reviewer's point on #1078 at 9a40e368ae, with and without its guard | It passes with the guard and fails without it: the walk puts two folders' files in a group of their own ahead of the rest. |
| Linux x64 | The engine's logic crate tests and clippy with -D warnings | At 72b2021a6f the logic crate's 416 tests pass, and clippy with -D warnings passes over the workspace and the native crate. At 9a40e368ae it failed on a test-only function only Windows tests use. |

**Sources.** The branch at 87e71afc33 and its diff against `main` aee140dd91, read on 25 November 2026; runs on Linux x64 and Windows 11 machines and on GitHub's runners in our fork.

</details>
