# A first session's walk of many worktrees takes about half the memory

For [pull request #812](https://github.com/neralabs/rerail/pull/812), head da345bfc3. [report.html](report.html) is the same report as one page: download it and open it.

## 1. The decision

**May a walk answer for a worktree by another's rule engine holding the same rules, and sessions share each checkout's Git ignore reader?**

**We recommend:** approve it: on the 0.2.0 engine the first session's walk of 61 checkouts takes 377 to 516 MB where `main` takes 903 to 1,045 MB; answers are `main`'s or Git's (network shares: stand-in cases).

| Size | Risk and review | Waits on | Not verified |
|---|---|---|---|
| 3,166 lines in 15 files: 1,232 ship, 1,934 are tests and text | Careful. It changes which engine answers a walk's questions. | Nothing. No CI result at this head yet. | 4 |

## 2. Intent, and the gap

| Change | Intended | Found | Now |
|---|---|---|---|
| A. A walk's questions | Each worktree answered by its rules _(our reading)_ | An engine opened per worktree _(heap snapshot)_ | Closes the gap. |
| B. Git ignore readers | Ignore rules read once _(code comment)_ | Read once per session _(one reading)_ | Closes the gap. |
| B. The index read again | Index read again on change _(code comment)_ | A same-step rewrite was missed _(engine case, #809)_ | Closes the gap. |

Found in November 2026 on Linux x64 and Windows 11 machines, measuring #726's steps in a repository of 153 checkouts.

<details><summary>Sources at main, and causes</summary>

| Change | Where main states the intent | The cause, and what was measured |
|---|---|---|
| A. A walk's questions | `src/engine/code-rule/checkout-code-rule-router.ts` at 7efcac9c6: `#accepts` and `#ignores` answer through `#lane(owner.checkoutRoot)`, which opens that checkout's rule engine on first use. | `#accepts` and `#ignores` call `#lane`, which opens a `NativeCodeRuleEnforcer` and its workspace (the checkout's rules, a content store, its index) in every worktree the first walk reaches. A heap snapshot after one session held one per checkout. |
| B. Git ignore readers | `src/engine/code-rule/checkout-code-rule-router.ts` at 7efcac9c6, `#gitIgnores`: "Whether Git ignores `owner`'s path in its checkout, while the session follows `.gitignore`; its rules open once." Once per router: each session opens its own. | `openGitIgnoreMatcher` is called per router and per engine, so sessions in a repository of many worktrees each hold a copy of every asked worktree's Git index. |
| B. The index read again | `native/src/git_ignore.rs` at 7efcac9c6, the module comment: "The index is re-read whenever it changes on disk, so `git add` of an ignored file brings it back". `ignored_by` reads it through gix's shared index (`index_or_empty`). | gix reads its shared index again only once the file's modification time is later. Git writes the index to a new file and renames it over the old one, and a second write within one step of the file system's clock keeps the first one's time, to the nanosecond. `main`'s own case fails 3 of 200 runs on a machine whose file times step every 4 ms. |

</details>

## 3. Where to look

<img src="map.png" width="544" alt="The changed files as bars sized by changed lines, grouped by change and coloured by risk. Code to read: A, Walk answered without new engines, medium risk, 219 lines; B, One Git ignore reader per checkout, medium risk, 1,013 lines. Read first: checkout-code-rule-router.ts, git_ignore.rs, git-ignore-matchers.ts. To skim: 1,932 lines of tests and 2 lines of half the memory; same-step force-adds checked..">

1. **Read first**, 8 min: `checkout-code-rule-router.ts`: `#accepts`, `#readsIn`, `#laneWithRulesOf`, `#refreshAnsweredCatalogs`. Can a worktree be answered by an engine whose rules read paths its own would not?
2. **Read first**, 10 min: `git_ignore.rs`: `sources_unchanged`, `Sources`, `refresh`, `FileStamp`. Which ignore change or index write would a new reader see that this one misses?
3. **Read first**, 4 min: `git-ignore-matchers.ts`: `matcher`, `release`. Is a reader ever kept after every router that took it has closed?
4. 3 min: `code-rule-enforcer.ts`, `native.ts`, `engine-service.ts` and `runtime.ts`: the version and path questions, the binding and the wiring.

**Then skim**, 8 min: The router cases count engines and readers opened; the reader cases change one Git source at a time and ask a later router.

## 4. Risk against verification

<img src="grid.png" width="544" alt="The changes by risk, high on top, against six checks. A, Walk answers, medium risk: the first five checks hold. B, Ignore reader, medium risk: the first five checks hold. CI at this head does not hold yet for every change.">

Start at change A: it decides which engine answers. Change B decides what a later session ignores.

<details><summary>Each limit, and each check that does not hold yet</summary>

| Change | Check | Its limit, or why it does not hold yet |
|---|---|---|
| A and B | CI at this head | CI ran green on the pull request at 44907c8b2; it runs again on the pushed head. |

</details>

<details><summary><b>5. The changes</b></summary>

**A. Walk answered without new engines**, medium risk. First session, 190 checkouts, open-file limit 92: 1,898 MB at `main`, 578 MB here (Linux x64, 0.2.0 engine).

- **Problem:** A session's first walk asks of every file of every worktree whether a rule reads it. `main` opens a rule engine in each worktree to answer, with the checkout's rules, a content store and its Git index.
- **Who meets it:** Everyone who opens a session in a repository with many worktrees, once per daemon start. #726 measured 846 MB with 110 worktrees on 0.1.5.
- **What it does:** `#accepts` answers for a worktree with no engine by an open engine at the same rules version (`#laneWithRulesOf`, `rulesVersion`, `readsPath`), through that worktree's own Git ignore rules; its catalog is read again with the engines' own.
- **Risk:** Medium. The answer comes from another engine: a rule whose reach depended on more than its catalog version would read differently. None does at `main`.
- **Check:** Is a catalog version enough to say two worktrees read the same paths, and is every worktree answered by another read again when the rules are?
- **Picture:** After the first session in a repository of 190 checkouts of 1,000 files with the four suggested rule packs, the daemon holds 1,898 MB at main 7efcac9c6 and 578 MB with this change, on Linux x64, under an open-file limit of 92 (medians of 10 pairs).
- **Method:** A generated repository of 190 checkouts of 1,000 files with the four suggested rule packs; a fresh daemon under an open-file limit of 92, then three sessions through real Claude Code hooks; resident memory once the daemon settles. 10 pairs, alternating order, on one Linux x64 machine; the median of each side.

**B. One Git ignore reader per checkout**, medium risk. Sessions share each checkout's ignore reader: the engine 96 MB less a later session, at 153 checkouts.

- **Problem:** A Git ignore reader holds its checkout's Git index. At `main` each session opens one in every worktree it asks about.
- **Who meets it:** Every session after the first in a repository with many worktrees.
- **What it does:** `GitIgnoreMatchers` hands a checkout's reader to a router while `sourcesUnchanged` holds: the engine reads Git's configuration again and compares three files' bytes. The reader keeps the index's stamp only where it tells every later write (NTFS, APFS, HFS+ at once; other local volumes once the clock leaves its times; network shares never), else compares the index's checksum.
- **Risk:** Medium. A stale reader would hide files from the rules or check ignored ones; the engine reads the configuration again as its own open does and compares what the rules come from.
- **Check:** Does `Sources` hold everything a reader opened now would read differently, and does `refresh` see every write of the index? It compares the worktree, the common Git folder, the excludes file, the pattern case, precious parsing and three files' bytes; the index by its stamp where the volume lets a stamp tell, else by its checksum.
- **Picture:** With a file changed in every one of 153 checkouts before each later session, each later session adds 1,364.5 MB of engine memory at main and 1,268.5 MB with this change, on Linux x64, at an earlier head with the same reader sharing.
- **Method:** 153 checkouts of 7,500 files, a file changed in every checkout before each of three later sessions, through real hooks; the engine's allocated bytes; one run each on one Linux x64 machine, at an earlier head with the same reader sharing.

</details>

<details><summary><b>6. The decisions</b></summary>

**Which engine answers a walk's questions about a worktree with none open?**

**We chose** an open engine at the same rules version, which reads the same paths, so no engine opens to answer. **Would change our mind:** A rule whose reach depends on the worktree beyond its catalog version; none exists at `main`.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| An open engine at the same rules version answers **(chosen)** | The same answers, about half the first session's memory. | A version check per question about a worktree with no engine. | Nothing found: records equal `main`'s. |
| An engine in every worktree the walk reaches | The same answers. | Twice the memory after one session at 153 checkouts. | Memory. |
| A table of the paths each rule reads, per rules version | The same answers. | A second reader of rule placements beside the engine's. | Two places that must agree on what a rule reads. |

**Evidence.** A heap snapshot after one session at an earlier `main` held one engine per checkout the walk reached. Router cases count the engines a walk opens, at `main` and here.

**Who says a shared Git ignore reader still reads what a new one would?**

**We chose** the engine, reading Git's configuration again through gix as its open does and comparing what the rules come from. **Would change our mind:** A source of ignore rules gix reads that `Sources` misses.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| The engine reads the configuration again and compares three files' bytes **(chosen)** | Each later session follows the rules a new reader would read. | One configuration read and three small file reads per checkout per later session. | Nothing found through real hooks on two systems. |
| A list of Git's files kept in TypeScript | The same, until Git reads a file the list lacks. | A second list that trails Git. | Missed includes, conditional includes and a commented value in an earlier version. |
| One reader per session | The same answers. | A copy of each checkout's index per session. | Memory. |

**Evidence.** Through real Claude Code hooks, sessions opened after each change of ignore sources answered as `main` on Linux x64 and Windows 11; in one state the shared reader answered as Git where `main` did not.

</details>

<details><summary><b>7. How it fits</b></summary>

Nothing has to merge first: it stands on `main` at a81b07ac3, with the 0.2.0 engine; `main` cccb25a8d edits none of its files.

This pull request is based on main at a81b07ac3, with the 0.2.0 engine, and needs no other change.

**Open pull requests that share a file.** #884 also edits `engine-service.ts`; a trial merge with this head has no conflict.

</details>

<details><summary><b>8. What is not covered</b>: 4 not verified, 3 limits that stay</summary>

1. **Later sessions** still grow the daemon about as at `main`: this is part of #726.
2. **Daemon CPU beside busy worktrees** read within 3% of `main`'s either way; the job's readings fell 25% from its first case to its last.
3. **Windows 11, the first session's walk** ended later: 85 against 70 s at `main` (medians of seven pairs, before the engine change), on no more CPU.
4. **macOS, FAT, exFAT, ReFS and network shares** never ran on such a volume; the cases stand in for each. #726 was measured on macOS.

**Limits that stay**

- An engine that has opened keeps the ignore rules it read for the rest of the session, as at `main`.
- Where no stamp is kept (a read within its file's clock step, an index time ahead of the clock, a share), a question reads the index's 20-byte checksum, or under `index.skipHash` the whole index.
- The volume is found once per reader by its file system's name: a driver that reports NTFS, APFS or HFS+ while giving a new file an earlier one's identity would keep a stamp that cannot tell them.

</details>

## 9. What we ask of you

We ask no ruling.

<details><summary><b>10. Evidence</b></summary>

The description on GitHub is the record: "How you tested it" holds the runs at `main` and here.

| Where | What ran | Result |
|---|---|---|
| Linux x64 | 190 checkouts of 1,000 files, three sessions under an open-file limit of 92, at `main` 7efcac9c6 and this change, alternating pairs | First session 1,776 to 2,074 MB at `main`, 436 to 661 MB here; no hook failed in 10 runs here nor in 10 at `main`; most files open 92 to 92 at `main`, 87 to 91 here. |
| Linux x64 | Through real hooks at `main` 7efcac9c6, ba0545900 and this head: an edit after a same-step `git add -f`; 61 checkouts of 3,000 files | 0 of 5 checked at both mains, 5 of 5 here. First session 1,045 MB at 7efcac9c6, 818 at ba0545900, 507 and 377 here. |
| Linux x64 and Windows 11 | The reader's cases at the merge with `main` (cases added) and here; each new line taken out | Red at the merge on both systems; here 3 runs on Linux x64, 5 on Windows 11, every case but `main`'s own path-form case on Windows; every mutant fails a case. |
| Linux x64 | 61 checkouts of 3,000 files, a session in the main checkout and one in a worktree, through real hooks, at `main` e021bf5c0 and here, two runs a side (before the engine change) | First session settled at 822 and 937 MB at `main`, 481 and 355 MB here; engine 539 and 92 to 93 MB. |
| Linux x64 | 153 checkouts of 7,500 files, a session in the main checkout and three worktrees, at `main` cf5d520e4 and this head on one machine; 11 checkouts under strace for the daemon's index reads (before the engine change) | First session 2,380 MB at `main` (engine 1,469 MB), 1,016 MB here (engine 204 MB); earlier pairs 2,212 against 1,024 and 2,193 against 1,054 MB. A later session's reader reads no index here, one per checkout at `main`. |
| Linux x64 and Windows 11 | Through real hooks at `main` cf5d520e4 and this head: an edit after a same-step `git add -f` (one session, five rounds); an earlier session after a later one took the reader (three runs a side) (before the engine change) | One session: 0 of 5 checked at `main`, 5 of 5 here, on both systems. Earlier session: 3 of 3 at both in every run, on both systems. |
| Linux x64 | CI's integration job at this head; `main`'s own index re-read case, 200 runs a build, on a machine whose file times step every 4 ms; each reader line taken out (before the engine change) | 90 tests passed, 4 skipped. The case failed 3 times at `main` and none here. Each of ten mutants of the reader fails a case. |
| Windows 11 | 35 test files reaching the reader, with this head's engine; the new cases 20 runs in a row; the engine's tests at `main` and here (before the engine change) | 729 passed, 5 skipped; 8 failures outside `main`'s record (time-outs, two walk checks) passed 3 of 3 runs again, where `main` failed one bound once. The new cases pass 20 runs. The engine fails the same 14 cases as `main` on one machine. |

**Sources.** The branch at da345bfc3 and its diff against `main` cccb25a8d (merge base a81b07ac3), read on 19 November 2026; runs on Linux x64 and Windows 11 machines of ours.

</details>
