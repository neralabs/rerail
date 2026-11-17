# A folder keeps its settings under the spelling an earlier version stored

For [pull request #676](https://github.com/neralabs/rerail/pull/676), head 67f59eed4. [report.html](report.html) is the same report as one page: download it and open it.

## 1. The decision

**Do you want an earlier version's rows copied to the spelling Windows stores, with a table of spellings, or moved with no table?**

1. Log and a silent status line 0.1.4 saved apply; a folder outside Git lists once.
2. 0.1.4's own repository list shows each copied folder outside Git twice.
3. On Windows a start-state check first copies rows waiting on its drive. **Changes the intent.**

**We recommend:** answer on #666 first, since it can change the copy rule; this head answers the 17 November review; in 220 Windows 11 readings beside 0.1.4 none is below `main`'s.

| Size | Risk and review | Waits on | Not verified |
|---|---|---|---|
| 2,845 lines in 28 files: 1,215 ship, 8 run in every test run, 1,622 are tests and text | Careful. A code owner approves. | #666's answer. Open since 14 November. | 4 |

## 2. Intent, and the gap

| Change | Intended | Found | Now |
|---|---|---|---|
| A. Saved settings | Settings hold under either spelling _(docs)_ | Log reads Steer _(real daemons)_ | Closes the gap, changes answer 1. |
| A. 0.1.4's own rows | Older builds open the file _(code comment)_ | 0.1.4 still reads its rows _(code reading)_ | Closes the gap, changes answer 2. |
| B. Later changes | A later change by 0.1.4 is read _(our reading)_ | A later Steer read Log _(automated review)_ | Closes the gap. |
| C. The start-state check | A check without side effects _(code comment)_ | A gone share: 21 s _(one lookup)_ | **CHANGES THE INTENT: ANSWER 3.** The check copies waiting rows. A maintainer must agree. |
| D. Lists | Recorded checkouts, listed _(code comment)_ | One folder listed twice _(real daemons)_ | Closes the gap. |

Found 13 to 17 November 2026, on Windows 11 beside 0.1.4.

<details><summary>Sources at main, and causes</summary>

| Change | Where main states the intent | The cause, and what was measured |
|---|---|---|
| A. Saved settings | `docs/windows.md` lines 236 and 237, under "Checkout keys", say the mismatch that #503 ended "lost audit settings and presentation preferences saved under the other spelling." Line 228 says every key goes through `checkoutRealpath`. | Since #503 every store looks a folder up by the spelling the disk stores. 0.1.3 and 0.1.4 kept the session's spelling for audit settings and presentation preferences, and a folder outside Git as typed. No step moves those rows. Read through real daemons and Claude Code's hooks against `main` at 71e0822ce. 0.1.5, published on 17 November, has this, as `main` does. |
| A. 0.1.4's own rows | `src/engine/storage/migrations.ts` lines 8 to 10, the module comment: "Older builds read a new nullable column, table or index correctly, because Drizzle names the columns it reads and writes, so such a step sets no \`floor\` and an older build keeps opening the file." | A coding agent's hooks from an older install, or a downgrade, share the database. The branch's first commits moved the rows: by a read of the code, not run, 0.1.4 then loses its presentation preference and repository audit for that folder. 6809d7af2 replaced the move with a copy. The doubled line: two folders outside Git that 0.1.4 started through `subst` letters, at fe4da2b09. |
| B. Later changes | Nothing at main says what a build reads of an earlier build's later writes. The nearest words, `src/engine/storage/migrations.ts` lines 8 to 10, speak of an older build opening the file. At main the two versions read each other's changes only where they spell a folder alike. | The step copies once. What 0.1.4 wrote later under a spelling the step copied stayed in its own rows: Windows 11, 0.1.4 from npm, a Git folder reached through a `subst` letter, at 46e3eab93. `main` reads none of it, before or after. Only a trigger runs inside an earlier version's own write. |
| C. The start-state check | `src/runtime/repository-start.ts` lines 52 and 53, above `workspaceStartState`: "\`workspace\`'s start state without side effects: no identity file, database, or schema is created and nothing is recorded." Lines 67 and 68 say "without side effects" of `workspaceSetUp` too, and the branch rewrites both. A read-only open at main already runs pending schema steps through a writable open (`src/engine/storage/database.ts` lines 173 to 175). | A drive mapped to a share whose host does not answer holds a read about 21 s, and the process waits that long at its exit. So an open reads the system drive and its caller's drive only, and keys elsewhere wait. A first reader on such a drive is the start-state check, which a hook that no daemon answers and `rerail state` make. The lookup was timed on Windows 11 at 8ad35c3d2. |
| D. Lists | `src/engine/catalog/repository-registry.ts` line 440, above `listRepositories`: "Every repository Rerail knows, with its recorded checkouts, ordered by name." Line 379, above `parentCheckouts`: "The recorded checkouts of the same repository whose folder contains \`checkout\`, nearest first; never itself." | 0.1.4 records a folder outside Git as the session typed it, and `main` records it again as the disk spells it: two `checkouts` rows for one folder, and every list reads both. At 67f59eed4 the dashboard lists 16 folders where `main` lists 17. At de8a6b3b5 a start state that left the earlier rows out read a folder 0.1.4 had stopped as on. |

</details>

## 3. Where to look

<img src="map.png" width="544" alt="The changed files as bars sized by changed lines, grouped by change and coloured by risk. Code to read: A, The step copies and keeps, high risk, 632 lines; B, Triggers carry later changes, high risk, 156 lines; C, Drives and the write lock, high risk, 253 lines; D, Lists and the start state, high risk, 182 lines. Read first: path-key-spelling.ts, path-key-spelling-triggers.ts, database.ts. To skim: 1,543 lines of tests and 79 lines of `docs/windows.md` and one release note.">

1. **Read first**, 15 min: `path-key-spelling.ts`: `lookUpPathKeys`, `respellPathKeys`, `moveKey`, `copyKeyedRows`, `noteFailedLookup`. Can a copy replace or drop a saved row?
2. **Read first**, 10 min: `path-key-spelling-triggers.ts`: `CARRIED_TABLES`, `copyOfOld`, `carriedTableTriggers`. Can a trigger fail a write, or change a row nobody copied?
3. **Read first**, 10 min: `database.ts`: `openRerailDatabaseIn`, `copyKeysWaitingIn`, `copyCanWait`, `applySchema`. Can a reader wait on a writer, or fail, where `main` answers?
4. 10 min: A: `migrations.ts`, `schema.ts`.
5. 4 min: B: `schema-sql.generated.ts`, `engine/index.ts`, `schema-sql.ts`.
6. 10 min: C: `repository-start.ts`, `operation.ts`, `stores.ts`, `session-presentation.ts`, `operations.ts`, `hook-host.ts`; D's `support-bundle.ts`.
7. 15 min: D: `repository-registry.ts`, `presentation-preference-store.ts`, `audit-store.ts`, `code-rule-dsl-catalog.ts`, `headless-scan.ts`, `runtime/index.ts`, `repository-operations.ts`, `rule-evidence.ts`.

**Then skim**, 10 min: No assertion leaves: one new test file, 43 cases; one case added to the support bundle's; one step id added.

## 4. Risk against verification

<img src="grid.png" width="544" alt="The changes by risk, high on top, against six checks. A, Copy and keep, high risk: users lose nothing holds with a limit, Windows 11: no new failure holds with a limit. B, Triggers, high risk: independent review holds with a limit, users lose nothing holds with a limit, Windows 11: no new failure holds with a limit. C, Drives, lock, high risk: fails without the fix holds with a limit, users lose nothing holds with a limit, Windows 11: no new failure holds with a limit. D, Lists, start, high risk: fails without the fix holds with a limit, users lose nothing holds with a limit, Windows 11: no new failure holds with a limit. CI at this head does not hold yet for every change.">

No CI result at this head yet. Every Windows mark is a half. Start at A.

<details><summary>Each limit, and each check that does not hold yet</summary>

| Change | Check | Its limit, or why it does not hold yet |
|---|---|---|
| A. Copy and keep | users lose nothing | Windows 11 beside 0.1.4 from npm, real daemons and Claude Code's hooks: 262 kinds of reading at fe4da2b09, none of this build's below `main`'s; 220 readings at 67f59eed4, 178 equal and 35 that keep what 0.1.4 saved. Linux x64: 93 kinds, all equal. One reading of 0.1.4's is below `main`'s: answer 2. |
| A. Copy and keep | Windows 11: no new failure | The change's test file is among the 146 run on Windows 11 at 67f59eed4. Of the 146 test files that import a changed module, 995 cases fail there: 956 as at `main`; 38 time-outs beside other runs, which pass 3 of 3 runs alone at both; 1 missing pack test data. |
| B. Triggers | independent review | The automated reviewer's review of cc6afc0b2 names no finding in the triggers and reads its earlier ones as fixed; it has not read 67f59eed4, and no maintainer has reviewed. Our own checks found a row under the stored spelling taking 0.1.4's later change (c63861204) and a silent status line drawn (fe4da2b09). |
| B. Triggers | users lose nothing | Through real daemons, a copy nobody changed took 0.1.4's later stop, audit setting, status line and added preference. Unit tests alone show: a start it makes later, its changes to a checkout's rule settings and to repository audits, the rows it deletes, and a row saved under the stored spelling while its key waited. |
| B. Triggers | Windows 11: no new failure | The change's test file is among the 146 run on Windows 11 at 67f59eed4. Of the 146 test files that import a changed module, 995 cases fail there: 956 as at `main`; 38 time-outs beside other runs, which pass 3 of 3 runs alone at both; 1 missing pack test data. |
| C. Drives, lock | fails without the fix | At 0ed1ad013, with keys waiting and the write lock held, a start-state check waited 5,467 ms and failed with `SQLITE_BUSY`, and the daemon answered 500 after 5.1 s: the new cases fail over that source. CI's 4 failed cases: 20 of 20 runs before 356c11b05, 0 of 20 after. The folder `readLiveResponse` sends has no test that fails without it. |
| C. Drives, lock | users lose nothing | Folders behind seven drive letters read Log here and Steer at `main`. A hook, `rerail start`, `stop`, `status` and `state` each copy a drive's keys as the first contact there. `rerail settings` alone on another drive reads Steer until one of those ran. A first session under a held write lock reads Steer, as at `main`. |
| C. Drives, lock | Windows 11: no new failure | The change's test file, with its drive and lock cases, is among the 146 run on Windows 11 at 67f59eed4. Of the 146 test files that import a changed module, 995 cases fail there: 956 as at `main`; 38 time-outs beside other runs, which pass 3 of 3 runs alone at both; 1 missing pack test data. |
| D. Lists, start | fails without the fix | The support bundle's case fails over cc6afc0b2's source, and the two cases of the pass that forgets checkouts each fail over another resolution of the merge, 20 of 20 runs on Windows 11 for one. The stopped folder's case fails 20 of 20 without 8ad35c3d2. Four readers have no test that fails without them. |
| D. Lists, start | users lose nothing | Windows 11, 0.1.4 from npm: a folder recorded through a `subst` letter stays in the support bundle at cc6afc0b2 and is replaced at bdf38a7b5 and at `main`. The dashboard lists each folder once at 67f59eed4. The pass keeps and forgets the same rows at `main` and here. The list of repository audits is shown by a unit test only. |
| D. Lists, start | Windows 11: no new failure | The change's test file and the support bundle's are among the 146 run on Windows 11 at 67f59eed4. Of the 146 test files that import a changed module, 995 cases fail there: 956 as at `main`; 38 time-outs beside other runs, which pass 3 of 3 runs alone at both; 1 missing pack test data. |
| A to D | CI at this head | No run has reported at 67f59eed4 yet. Run 44064143141 at cc6afc0b2: 31 checks pass, 2 are skipped, none fails. |

</details>

<details><summary><b>5. The changes</b></summary>

**A. The step copies and keeps**, high risk. 0.1.4's Log, read back: 0 of 26 at `main`, 26 of 26 here.

- **Problem:** After the update a folder a person set to Log is steered again, and a status line they turned off is drawn. Their settings sit under the spelling the session gave, where this version no longer looks.
- **Who meets it:** A person on Windows who updates from 0.1.3 or 0.1.4 to 0.1.5 or later with settings saved in a folder reached as `c:\…`, through a mapped or `subst` letter, or outside Git as typed. How many is not known.
- **What it does:** `lookUpPathKeys` looks every key up before the write lock. Under it, `respellPathKeys` records each moved spelling, and `copyKeyedRows` copies a folder's rows in five tables to the stored spelling where that key holds none. The earlier rows stay. A gone folder keeps its key, as does one whose lookups fail for an hour. Elsewhere the step reads nothing.
- **Risk:** High. A database step under `src/engine/storage/`, a high-risk path in `REVIEW.md`. A wrong copy changes whether the agent is told about a broken rule, and no later open repairs it.
- **Check:** Can `copyKeyedRows` overwrite a row under the stored spelling? Its `NOT EXISTS` reads the other primary key columns from `PATH_KEYED_TABLES`: is each list whole? Does `keyOrStored` leave a key waiting for every error that can pass, and keep it only for a folder that is gone or unreadable? Under the lock `respellPathKeys` reads the audit history only past the lookups' last row: can a key written in between be missed? No lookup on disk may run under the lock.
- **Picture:** Readings of rerail settings where 0.1.4 saved Log under one spelling only: 0 of 26 are Log at main, 26 of 26 with this change. Status lines where 0.1.4 saved silent: 0 of 8 stay silent at main, 8 of 8 with this change.
- **Method:** Windows 11, 0.1.4 from npm sharing one database, real daemons and Claude Code's hooks started with its JSON, at fe4da2b09 against `main` at 71e0822ce; no Claude Code process ran. 26 readings of `rerail settings` in 12 folders where 0.1.4 saved Log under one spelling only, and 8 status lines where it saved silent.

**B. Triggers carry later changes**, high risk. A scope 0.1.4 sets later, read back: 0 of 5 at `main`, 5 of 5 here.

- **Problem:** An older install's hooks keep writing under their own spelling. Without the triggers this version goes on reading the copy as the step made it: a later stop, Steer or compact set by 0.1.4 does not reach it.
- **Who meets it:** A person who runs two versions on one database: after a downgrade, or with a coding agent whose hooks come from an older install. How many do is not known.
- **What it does:** 13 triggers on five tables run inside an earlier version's own writes. `copyOfOld` matches this version's row only while it still holds every value the earlier row held; for a preference, which keeps no time, also only a row made after `preferences_before`. A checkout carries only its start state. A write under any other key costs one primary key lookup.
- **Risk:** High. The triggers are part of the schema applied at every writable open and run in every write to five tables, an earlier version's included. A trigger that failed would fail that write.
- **Check:** Can any trigger statement fail the write that runs it? They use no conflict clause and set only values the earlier row holds. Does each `values` list in `CARRIED_TABLES` name every column that tells a copy from a row saved apart? `CREATE TRIGGER IF NOT EXISTS` keeps an old body, so a build that changes a trigger must drop it in a database step. Does `setCheckoutStarted` still set its value, now that the nested rows go first?
- **Picture:** Readings of rerail settings after 0.1.4 set the touched-files scope later: 0 of 5 show that scope at main, 5 of 5 with this change.
- **Method:** The same runs through real daemons at fe4da2b09. Five readings of `rerail settings` after 0.1.4 set the touched-files scope later: `main` reads `ai-output/steer`; this build reads `touched-files/log`, and `touched-files/steer` once this build set Steer there.

**C. Drives and the write lock**, high risk. Start-state check, write lock held: 25.8 ms at `main`, 30.8 ms here.

- **Problem:** A key on another disk, a mapped drive or a share cannot be looked up from anywhere: a share whose host is gone holds a read about 21 s. And a reader that copies must not wait on a writer.
- **Who meets it:** A person whose folders sit on a drive other than the system drive, or behind a mapped or `subst` letter. A hook that no daemon answers and `rerail state` make the check.
- **What it does:** `readableRoots` lets an open look keys up on the system drive and its caller's drive only, 1 s per drive. The others wait. `openRerailDatabaseIn` copies them when a caller first works on their drive, and `copyKeysWaitingIn` takes the write lock only when free. `copyCanWait` lets only the lock or the disk leave keys waiting; a fault fails the read.
- **Risk:** High. Every writable open on every platform now passes `readBeforeLock` and `migrateLater`, and the check a hook makes before anything else can write. A reader that waited or failed here would hold a coding agent up.
- **Check:** On macOS and Linux, does `applySchema` do what `main`'s does? Can `openRerailDatabaseIn` return a closed connection, or leave one open, on any path through `keysWaitHere`? `copyKeysWaitingIn` repeats a writable open's three steps with a lock wait of 0: does it stay in step with `openChecked`? Can a coding agent make the daemon look up a drive it does not work on (`copyWaitingKeysFor`)? `requestedFolder` sends the session's spelling on Windows only.
- **Picture:** A start-state check with keys waiting while another process holds the write lock: 25.8 ms at main, 30.8 ms with this change.
- **Method:** Windows 11 at 46e3eab93: the median of 21 start-state checks with keys waiting while another process holds the write lock; 6 to 29 ms more than `main`'s for 1 to 200 waiting keys. A writable open there takes 60 to 140 µs more than `main`'s (medians, two runs each, 0ed1ad013). Neither was timed again at this head.

**D. Lists and the start state**, high risk. A folder outside Git, listed: twice at `main`, once here.

- **Problem:** With the earlier rows kept, every list that reads a keyed table would show a copied folder twice. And a start state must still follow a stop 0.1.4 makes in the same folder under another letter case.
- **Who meets it:** Anyone who opens the repository list, the repository audits or the presentation preferences after the step. The stopped folder: a person who stops Rerail with 0.1.4 after this version first ran there.
- **What it does:** `listRepositories` gives every recorded row in `checkouts`, which the support bundle replaces, and `listedCheckouts` the list a person reads. `notAnEarlierSpelling` leaves the kept rows out of `parentCheckouts`, `recordedCheckoutsWithin` and the preference list. `startStateParents` counts them. `pruneCheckouts` reads a kept row as the folder the disk stores. `listRepositoryAudits` reads where an audit ran under its folder's key.
- **Risk:** High. `checkoutStatus` decides whether Rerail runs in a checkout: a wrong parent turns Rerail on where a person stopped it, as de8a6b3b5 did before 8ad35c3d2.
- **Check:** Is each reader of `checkouts` either filtered or meant to read the kept rows, as the support bundle must? `listedCheckoutStates` counts them for the status and not for the parent it names: is that the split `checkoutStatus` and `parentCheckouts` make? `remove` deletes this version's row first, since a trigger would otherwise delete it uncounted. The list of repository audits goes past #666's bug: 0.1.4 already shows the doubled entry there.
- **Picture:** Entries for one folder outside Git in the dashboard's repository list: 2 at main, 1 with this change.
- **Method:** Through real daemons on Windows 11 at 67f59eed4: the dashboard's repository list shows each folder once, 16 entries, where `main`'s shows one folder outside Git twice, 17. The list of repository audits is shown by a unit test; no run through real daemons recorded a repository audit.

</details>

<details><summary><b>6. The decisions</b></summary>

**May the start-state check write?** (ask 3)

**We chose** the copy at the check: a drive's first hook reads 0.1.4's settings. **Would change our mind:** a need for a check that never writes.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| The check copies first, when the write lock is free **(chosen)** | The first hook, `rerail status` or `rerail state` on a drive reads what 0.1.4 saved there: folders behind seven drive letters read Log, where `main` reads Steer. | A check that `main`'s comment calls free of side effects can write: one writable open per drive with waiting keys. Under a held lock it takes 30.8 ms against 25.8 ms. | While another process holds the write lock the keys wait, and that session reads Steer, as at `main`. `rerail settings` alone copies nothing. |
| Copy only at `rerail start`, `rerail stop` and a daemon's request | The check stays a read, as `main`'s comment says. | Not built, so no run shows it. | By a read of the code: a hook that no daemon answers and `rerail state` on another drive read as at `main` until one of those commands ran there. |

Before: At main a hook's start-state check opens the database read-only and reads the folder's start state. After: With this change on Windows the check opens the database read-only and asks whether keys wait on the folder's drive. If none does, nearly always, it reads the start state. If some do, it copies their rows when the write lock is free, then reads.

**Evidence.** Through real daemons on Windows 11: a hook, `rerail start`, `stop`, `status` and `state` each copy a drive's keys as the first contact there; with no daemon the status line copies nothing and the first hook copies. Under a held lock: the median of 21 checks at 46e3eab93, with `main`'s answer. At 0ed1ad013 the same check waited 5,467 ms and failed.

**Copy with a table, move, or read both spellings?** (ask 1)

**We chose** the copy: both versions then read what 0.1.4 saved. **Would change our mind:** a maintainer's answer on #666.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| Copy and keep, with `path_key_spellings` **(chosen)** | This version reads what 0.1.4 saved: Log, a silent status line, a checkout's rule setting, a stop. 0.1.4 sharing the file reads each row where it wrote it. | 1,215 changed source lines in 22 files, a table and 13 triggers, on the storage path. | 0.1.4's own repository list doubles each folder outside Git the step copied. |
| Move the rows, with no table | This version reads what 0.1.4 saved. | No table, no triggers and no filter in the lists. | An earlier version sharing the database loses its presentation preference and repository audit for that folder. Read from the code, not run. |
| Read the earlier spelling at lookup, with no database step | The audit mode and a stop come back, and 0.1.4's later changes are read with no copy. | 137 changed source lines in 5 files, at 3e7b3ebec. Per event and per status line refresh, no more than `main`. | A silent status line and a rule turned off outside Git through a mapped letter stay lost. A session spelt as the disk stores the folder reads Steer, as it did with 0.1.4. |

Before: At main, 0.1.4 saved Log under the session's spelling, this build looks the folder up under the stored spelling, finds no row and reads Steer. After: With this change the step copies the row to the stored spelling and records the earlier spelling in path_key_spellings. This build reads Log, and 0.1.4 still reads its own row.

**Evidence.** The copy: Windows 11 beside 0.1.4 from npm, real daemons and Claude Code's hooks, at fe4da2b09, 262 kinds of reading, and 220 readings at 67f59eed4. The read at lookup: the same set-up against `main` at c89f068ed, in #666's comment of 14 November. The move was read from the code and not run.

**Outside Git: copy every `checkouts` row, or only rows with a start state?** (ask 2)

**We chose** every row for now: #666's answer may change the copy rule. **Would change our mind:** an answer on #666 that keeps the copy.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| Copy the `checkouts` row of every folder outside Git **(chosen)** | Every folder outside Git that 0.1.4 recorded is recorded under the stored spelling at the first open from its drive. | One doubled line per such folder in an older version's repository list. | 0.1.4's own list shows each copied folder twice, also where this build never recorded that folder. After `main` it shows twice only the folders `main` records itself. |
| Copy only rows whose own start state is set | 0.1.4's repository list stays as it is after `main` for folders whose start state follows their repository. | Not built, so no run shows it. | It still copies, and 0.1.4 still doubles, a folder whose start state 0.1.4 set on its own, with `rerail stop` there. |

**Evidence.** Through real daemons on Windows 11 at fe4da2b09: two folders outside Git that 0.1.4 started through `subst` letters hold no start state of their own. After `main`, 0.1.4's list shows each once. After this build's first `rerail status` and `rerail state` there, it shows each twice. A third folder, which `main` records itself, is doubled after both.

</details>

<details><summary><b>7. How it fits</b></summary>

Nothing has to merge first: this head holds `main` at 44576f751, with #708 in it. #674 has merge conflicts with it: whichever lands second keeps both sides.

Pull request #676 is based on main at 44576f751, which this head has merged, and no other change stands on it. In trial merges it has merge conflicts with the draft #674 in three shared files, and merges clean in every file it shares with four other open pull requests.

**Shared files, trial merges, the smaller fix.** #708 merged first: its pass in `pruneCheckouts` is kept whole and reads a row kept under an earlier spelling as the folder the disk stores (092f30742). #674, a draft, meets it in `session-presentation.ts`, `repository-start.ts` and `hook-host.ts`. Trial merges with #677 and #739 are clean; with #517 and #508 the shared file merges. A smaller fix, which reads the earlier spelling at lookup, stands at 3e7b3ebec: one of the two lands. #674 was not run merged with this. The description has the commits under "Trial merges".

</details>

<details><summary><b>8. What is not covered</b>: 4 not verified, 6 smaller gaps, 8 limits that stay</summary>

1. **0.1.4's own repository list** doubles each copied folder outside Git. The narrower copy rule waits for #666's answer.
2. **Shown by unit tests only:** a later start by 0.1.4, its changes to rule settings and repository audits, and its deletions.
3. **Five lines** have no test that fails without them.
4. **Not run:** macOS, the whole suite, the 262 kinds of reading at this head (220 readings ran), 0.1.5 between 0.1.4 and this build, a merge with #674.

**Smaller gaps**

- **`rerail settings` alone on another drive.** On a drive other than the system drive it reads Steer until a hook or `rerail start`, `stop`, `status` or `state` has run there. Shown at 8ad35c3d2, on a path no later commit touches.
- **A first session under a held write lock.** It waits about 5.9 s and is not told Rerail runs, at `main` and here: 5,858 and 5,870 ms on Windows 11, 5,256 and 5,261 ms on Linux x64. It reads Steer; the next session copies and reads Log.
- **A case-only rename.** After a folder is renamed in letter case only, both versions read Steer for it and list two rows, at `main` and here.
- **A link to a gone share.** A key on the system drive whose folder became such a link would be looked up at each writable open from that drive for an hour, then keep its spelling: not measured. One such lookup took 21,030 ms. 0.1.4 stores a link's target.
- **Row ids across `rerail scrub`.** The preference rule relies on SQLite keeping the row ids of `presentation_preferences` through `VACUUM`. If that changed, copies would stop following 0.1.4's later changes; no row that stood would start following.
- **The five lines, named.** The filter in `parentCheckouts`, which no longer serves the start state; `path_key_spellings` among a headless scan's configuration tables; the folder `readLiveResponse` sends; rule evidence's checkout count; and the checkout a scan picks (`checkoutOnDisk`).

**Limits that stay**

- A later change by an earlier version carries only while this version's copy still holds what the step copied. Once this version changes the copy, it keeps its own values, as at `main`.
- What an earlier version first saves after the step under a spelling the step never saw is not read here, as at `main`.
- Where 0.1.4 saved a folder under both spellings, the row under the stored spelling stays for both versions, and this version reads it under every spelling, as `main` does: Steer where only the other spelling held Log.
- A presentation preference removed here is removed for the earlier version too. Its status line through a drive letter applies what it saved under the letter, where at `main` it is drawn.
- Cost on Windows 11: a writable open takes 60 to 140 µs more than `main`'s (0ed1ad013), and a session start through real hooks 16 ms more at the median of pairs, from 47 less to 67 more (8ad35c3d2).
- Writes to five tables run a trigger, and the pass that forgets checkouts reads one more join: neither timed on its own. `rerail scrub` on Windows counts a copied folder as 2 checkouts.
- The audit history is never rewritten, no floor is raised, and an earlier version still opens the file. On macOS and Linux the step reads and copies nothing.
- Two sentences of `docs/windows.md` rest on unit tests only: this version reads a start an earlier version makes later, and a row saved under the stored spelling before the step copied the folder keeps its values.

</details>

## 9. What we ask of you

Asks 1 to 3 rule on answers 1 to 3.

1. **The copy with its table of spellings, or a move with no table?** We recommend the copy, as built; rule on #666.
2. **May 0.1.4's list double a copied folder until #666 is answered?** We recommend yes, for now.
3. **May a start-state check write on Windows?** We recommend yes.
4. **May a `Migration` read before the write lock, and run at later opens?** We recommend yes.
5. **Where 0.1.4 saved both spellings, does the stored spelling's row stay?** We recommend yes, as at `main`.

<details><summary><b>10. Evidence</b></summary>

The description on GitHub is the record: "How you tested it" holds the runs and the costs, "The decisions" each option.

| Where | What ran | Result |
|---|---|---|
| Windows 11, real daemons | `main` at 44576f751 and this build at 67f59eed4, each after 0.1.4 from npm wrote the database, then 0.1.4 again; 20 folders, Claude Code's hooks started with its JSON, the dashboard's list | 220 readings. 178 equal. 35 where this build keeps what 0.1.4 saved and `main` does not. 4 of the dashboard's list: each folder once here, one twice at `main`. 3 audit ids each run makes. At fe4da2b09: 262 kinds of reading, none of this build's below `main`'s. |
| Linux x64, real daemons | The same order at fe4da2b09, with two folders that differ only by letter case, a folder outside Git, a stop by 0.1.4 and a held write lock | 93 kinds of reading, all equal, 0.1.4's included. |
| Windows 11 and Linux x64 | The built CLI beside 0.1.4 from npm: the support bundle at `main`, cc6afc0b2 and bdf38a7b5 | Windows 11: a folder 0.1.4 recorded through a `subst` letter stays in the bundle at cc6afc0b2; `main` and bdf38a7b5 replace it. Linux x64: equal at the three. |
| Linux x64 and Windows 11 | The 146 test files that import a changed module, at 67f59eed4 | Linux x64: 2,934 passed, 29 skipped, 956 failed: 955 in `packs.test.ts`, as at `main` on that machine, and 1 time-out under load. Windows 11: 995 failed: 956 as at `main`; 38 time-outs and timings beside other runs, which pass 3 of 3 runs alone here and at `main`; 1 missing pack test data. |
| Linux x64 | The answers to the review of 17 November, at bdf38a7b5 | Each new case fails over the source before its fix; 19 one-line removals each turn a case red; the 7 new cases pass 20 of 20 runs. |
| Linux x64 and Windows 11 | The two cases of the pass that forgets checkouts, at 67f59eed4 | Both pass 20 of 20 runs on each system. With `main`'s pass taken as it is, one fails 20 of 20 runs on Windows 11; with the merge alone, the other fails on Linux x64. |
| Linux x64 | The automated reviewer's rules over the diff against `main` at 44576f751, with published 0.1.1 and with `main`'s build, at 67f59eed4 | No new finding; 28 existing ones elsewhere in the changed files. Published 0.1.1 cannot parse `support-bundle.test.ts`, at `main` too; `main`'s build scans it and agrees. |
| CI on GitHub | The pull request's own run | No run has reported at 67f59eed4 yet. Run 44064143141 at cc6afc0b2: 31 checks pass, 2 are skipped, none fails. |

**What this report adds to the description**

- **What `main` says of the start-state check.** The description lists the copy at a drive's first contact among its changes. This report adds that `main`'s comment above `workspaceStartState` calls the check free of side effects, and that this branch rewrites the comment: a change of intent, with ask 3 and the first decision.
- **Two rulings the description does not ask for.** The description names what a `Migration` gains (`read`, `readLater`, `later`) in a fold under "Where it lands", and states the row that stays as its decision 3. This report asks for a ruling on each: asks 4 and 5.
- **Sizes as changed lines.** The description counts added lines: 1,142 of source in 22 files. This report counts added and removed lines together: 1,215 that ship in 22 files, 8 in the schema generator, 1,543 in 3 test files and 79 in the docs and the release note.

**Sources.** The branch at 67f59eed4 and its diff against `main` at 44576f751, read on 17 November 2026. Rerail at `main` for each quoted intent, and 0.1.5 as published for what it holds. Issue #666 and its comments of 14 November. The automated review of 17 November. Our run records and the decision records of this change and of the smaller fix.

</details>
