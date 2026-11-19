# An unread write is checked at the next walk that reads it

For the pull request of branch `JeanFrancoisGagne/walk-unreadable-own-write`, not opened yet, head 661802b80. [report.html](report.html) is the same report as one page: download it and open it.

## 1. The decision

**Should a walk that could not read a file keep the session's calls, so the next walk judges the write by its stamp?**

**We recommend:** approve it: one condition at the end of a walk, the one the shared worktree walk already uses, and a held write is checked at the agent's next command.

| Size | Risk and review | Waits on | Not verified |
|---|---|---|---|
| 320 lines in 5 files: 21 ship, 299 are tests and text | Quick. Two source files, 14 lines added and 7 removed. | Nothing. GitHub's CI runs at 661802b80, with the engine merged. | 4 |

## 2. Intent, and the gap

| Change | Intended | Found | Now |
|---|---|---|---|
| A write never read | The next walk judges it _(the line's comment)_ | No walk took it _(built hook, five sessions)_ | Closes the gap. |

Found on 19 November 2026 (UTC) in a bridge case of another change, then shown through the built hook on Linux x64 and Windows 11.

<details><summary>Sources at main, and causes</summary>

| Change | Where main states the intent | The cause, and what was measured |
|---|---|---|
| A write never read | `src/engine/code-rule/code-rule-bridge.ts` at cf5d520e4, `#finishSessionWalk`: `if (scan.failedRoots.size === 0) discovery.toolCalls.answered(walk.callsEnded)`. | Only a folder the walk could not list held the calls back. A file whose read failed left no stamp, so the walk forgot the command; the next walk found no call over the file's stamp and took the write for another program's. |

</details>

## 3. Where to look

1. **Read first**, 2 min: `code-rule-bridge.ts`: `#finishSessionWalk` and `#auditEdits`. Can a walk that missed a read still let a call go, or drop the session's text?
2. 1 min: `worktree-walks.ts`: `missedReads`.

**Then skim**, 3 min: Seven bridge cases: held command and edit tool writes, a folder, a later write, a file that stays held, another program's write in Log; one through the hook.

## 4. Risk against verification

<img src="grid.png" width="544" alt="The changes by risk, high on top, against six checks. Missed read, low risk: the first five checks hold. CI at this head holds with a limit for every change.">

One change, low risk: when a walk lets the session's calls go.

<details><summary>Each half mark's limit</summary>

| Change | Check | It holds with this limit |
|---|---|---|
| Missed read | CI at this head | GitHub's CI runs at 661802b80; its result is not in this report. |

</details>

## 5. The changes

**Calls kept while reads are missed**, low risk: A held write: `main` cf5d520e4 steered 0 of 5 sessions; this change 5 of 5.

<details><summary><b>6. The decisions</b></summary>

**Keep the calls until a walk reads everything, or claim the unread file?**

**We chose** keep the calls: the next walk judges the write by its stamp, as for any file. **Would change our mind:** A real program that holds files and moves their times as it lets go, often enough that the stamps miss the command.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| Keep the calls until a walk has read everything it met **(chosen)** | The write is checked at the next walk that reads the file. | The calls of one walk kept while a file stays unreadable. | Nothing found; a holder that moves the file's times reads as the writer. |
| Claim the unread file for the command | The same write checked. | A set of paths. | Another program's later write is audited as the agent's. |

**Evidence.** With the claim built, the case of a later write by another program fails: that write is audited as the agent's. With the chosen form it passes, as at `main` cf5d520e4.

</details>

<details><summary><b>7. How it fits</b></summary>

Nothing has to merge first: it stands on `main` cf5d520e4. Two waiting changes edit `code-rule-bridge.ts` and its test file; both trial merges are clean.

**The files it shares.** A change that records each place a walk could not check edits the lines above; a change that leaves a file a command is emptying edits `stampOfRead`. Any order works.

</details>

<details><summary><b>8. What is not covered</b>: 4 not verified, 1 limit that stays</summary>

1. **The run through the product** (a daemon, the built hook, 5 sessions a side) was not made again on the 0.2.0 engine.
2. **A holder that moves the times** of the file as it lets go makes the write read as that holder's, as at `main` for any such write.
3. **How often** a person's machine holds a file as a command ends was not measured.
4. **macOS** never ran.

**Limits that stay**

- On Windows 11 at `main` cf5d520e4, the first session after a daemon start was once not steered at the result about a write with no holder: its command came while the session's rules were still opening, a separate effect.

</details>

## 9. What we ask of you

We ask no ruling.

<details><summary><b>10. Evidence</b></summary>

The description on GitHub is the record: "How you tested it" holds the runs at `main` and here.

| Where | What ran | Result |
|---|---|---|
| Linux x64 and Windows 11 | On the 0.2.0 engine: the changed modules' test files and the hook file at 8d16bd8c8 and `main` c21947263, and the new cases over the sources of `main` c21947263 | On each system the 15 test files pass at this head (415) and at `main` c21947263 (408), the hook file too; over the sources of `main` c21947263 the four bridge cases and the hook case fail. |
| The maintainers' machine | Their own run on #825, the release candidate (`main` 7efcac9c6) | The bug shows, as on `main` cf5d520e4. |
| Linux x64 and Windows 11 | A daemon of this build and the built Claude Code hook, a held write, 5 sessions a side | `main` cf5d520e4 steered 0 of 5 and recorded 0 of 5 on both; this change 5 of 5 on both. |
| Linux x64 and Windows 11 | The same steps through 0.1.5 from npm | Steered 0 of 5, recorded 0 of 5, as `main` cf5d520e4, on both. |
| Linux x64 and Windows 11 | The seven bridge cases and the hook case, 20 runs at this head | 20 of 20 passed on each system, the hook case also 10 of 10 beside a CPU load on Linux x64; with the sources of `main` cf5d520e4 four bridge cases and the hook case fail. |
| Linux x64 and Windows 11 | The issue's steps as written, with 0.1.5 from npm | The held write lost 3 of 3 times on Linux x64 and 1 of 1 on Windows 11. |
| Linux x64 and Windows 11 | Five mutants of the two changed lines and the claim form | Each fails a case. |
| Linux x64 and Windows 11 | The 15 test files of the changed modules and those importing them, against `main` | 466 passed on each; `main` cf5d520e4's known quiet-session flake failed once on Linux x64, then 10 of 10 here and at `main` cf5d520e4. |

**Sources.** The branch at 8d16bd8c8 (5c9ac6453 with `main` c21947263 merged in, no merge conflict) and its diff against `main` c21947263, read on 19 November 2026.

</details>
