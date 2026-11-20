# Thirteen waits end on the fact that decides each, not a clock

For [pull request #964](https://github.com/neralabs/rerail/pull/964), head 22d3ffbd1. [report.html](report.html) is the same report as one page: download it and open it.

## 1. The decision

**May thirteen of Rerail's waits end on the fact that decides each, in place of its clock, in one pull request?**

**We recommend:** approve it: each wait shows on today's `main` and not here, through the product; two rare states that end later on purpose are named under the decisions and what is not covered.

| Size | Risk and review | Waits on | Not verified |
|---|---|---|---|
| 18,926 lines in 156 files: 8,363 ship, 9 run in every test run, 10,554 are tests and text | Careful. A code owner: when hooks answer and records land. | Nothing. No pull request needs to merge first. | 6 |

## 2. Intent, and the gap

| Change | Intended | Found | Now |
|---|---|---|---|
| A. A hook's daemon wait | A bounded daemon wait _(code comment)_ | Hooks wait, events lost _(product run)_ | Closes the gap. |
| B. Records, lock held | Writes wait for the lock _(code comment)_ | Hooks wait, records lost _(product run)_ | Closes the gap. |
| C. A turn's end | Turn's end carries steers _(code comment)_ | Next prompt gives up _(product run)_ | Closes the gap. |
| D. A command's result | A walk per result _(code comment)_ | Seconds per result _(product run)_ | Closes the gap. |
| E. A restart's successor | Successor takes the lock _(code comment)_ | No daemon left _(product run)_ | Closes the gap. |
| F. A reopened session | First edit checked on time _(code comment)_ | Every call waits _(product run)_ | Closes the gap. |
| G. The note's reads | Note read after answer _(our reading)_ | Late note, no count _(product run)_ | Closes the gap. |
| H. The bridge's check | One bounded check _(code comment)_ | Busy daemon called stopped _(product run)_ | Closes the gap. |
| I. The dashboard's reads | Reads on a clock _(code comment)_ | Late changes, idle reads _(product run)_ | Closes the gap. |
| J. The sign-in reading | Sign-in kept five minutes _(code comment)_ | Status waits for gh _(product run)_ | Closes the gap. |

Each wait shown on today's `main` on 20 November 2026, through the product, on 16-core and 32-core Linux x64 machines.

<details><summary>Sources at main, and causes</summary>

| Change | Where main states the intent | The cause, and what was measured |
|---|---|---|
| A. A hook's daemon wait | `src/runtime/daemon/daemon-waits.ts` lines 12 to 17 at `main`: `HOOK_DAEMON_WAIT_MS = PROBE_TIMEOUT_MS + BUSY_DAEMON_WAIT_MS`, "How long, in all, a hook waits for a busy daemon from the event's arrival", 2 s and 10 s; `hook-host.ts` line 94 sets each hook's deadline from it. | Every hook spends the whole bound on a daemon that has not read since the last hook's bound ran out, then drops its event: nothing records that the daemon left an earlier event untaken. |
| B. Records, lock held | `src/engine/storage/database.ts` at `main`: line 44 `PRAGMA busy_timeout = 5000`; line 81, "a write would wait for it up to `busy_timeout` (5 s) on the caller's thread". A steered hook writes its own receipt through the same wait. | The daemon's record writes run on the thread that answers hooks, so each waits SQLite's 5 s there, and a write that outlasts it is lost; a steered hook waits the same for its receipt. |
| C. A turn's end | `src/runtime/runtime.ts` lines 1306 to 1324 at `main`, `#ingestTurnEnd`: "A hook host's stop hook carries the turn's steers, so it waits for all of them". The hook gives the daemon 30 s (line 143). | The turn's end goes on waiting after its hook gave up, and the session's later events queue behind it; the steers it finds go into an answer no one reads. |
| D. A command's result | `src/engine/code-rule/code-rule-bridge.ts` lines 1343 to 1349 at `main`: `BOUNDARY_DISCOVERY_WAIT_MS = 10_000`, "How long a turn checkpoint or a command result waits for a checkout-wide source walk ... before answering with what it has". | Every shell command's result walks the whole session folder to find what the command wrote, though the folder's watch already reports each change. |
| E. A restart's successor | `src/engine/native/home-daemon-lock.ts` lines 20 to 26 at `main`: `ACQUIRE_WAIT_MS = 1_000`, `ACQUIRE_POLL_MS = 50`, "a daemon restarting itself lets go as it exits, just after it launches the next one". | The successor looks at the lock every 50 ms and gives up after 1 s; an old process slower to exit outlasts it, and the successor exits too. |
| F. A reopened session | `src/runtime/runtime.ts` lines 138 to 145 at `main`: `REOPENED_RULES_WAIT_MS = 20_000`, "Its first edit went unchecked at its own result while they did, and its steer arrived a turn later." | Every tool call of a reopened session, a read or a search too, waits for the rules, though only a call that can write needs them. |
| G. The note's reads | `src/presentation/session-presentation.ts` at `main`: line 80 `READ_TIMEOUT_MS = 2_000`, the bound of each read `get` makes (line 174), the note's, the steer link's and `/rerail`'s rules page among them, all after the event's answer; lines 146 to 151: past it the daemon "is busy, not gone". | The hook reads twice more after the event's answer; a daemon busy with the work it just answered misses both bounds, and the note goes out without its count. |
| H. The bridge's check | `src/api/mcp-bridge.ts` lines 104 to 111 at `main`: `#confirm` checks the daemon's health and throws `DAEMON_NOT_RUNNING` when that check fails; `src/runtime/daemon/daemon-waits.ts` line 13: `PROBE_TIMEOUT_MS = 2_000`. | A busy daemon misses the health check's bound, and the bridge reads the missed check as no daemon. |
| I. The dashboard's reads | `ui/poll.js` lines 1 to 8 and 18 at `main`: "one conditional GET per resource every 5 s (`If-None-Match`; 304 means nothing changed), backing off to 30 s while the daemon is unreachable"; `INTERVAL = 5_000`. | The page cannot know when anything changed, so it reads on the clock whether or not anything moved. |
| J. The sign-in reading | `src/runtime/merge-assist/service.ts` lines 131 to 134 at `main`: `GITHUB_READINESS_MS = 5 * 60_000`, "How long `gh`'s readiness is reused before `gh auth status` runs again"; `GITHUB_UNAVAILABLE_MS = 60_000` for a `gh` that could not tell. | Past the age a status runs `gh` in its own answer, and a sign-in changed outside Rerail stays unseen until the age runs out. |

</details>

## 3. Where to look

<img src="map.png" width="544" alt="The changed files as bars sized by changed lines, grouped by change and coloured by risk. Code to read: A, Hooks beside a silent daemon, high risk, 1,176 lines; B, Records off the hook thread, high risk, 2,033 lines; C, A turn&#x27;s end that gave up, medium risk, 457 lines; D, Shell results by change notices, medium risk, 1,022 lines; E, The daemon&#x27;s lock and restart, medium risk, 837 lines; F, A reopened session and kept rules, medium risk, 325 lines; G, Note and link in the answer, low risk, 560 lines; H, The MCP bridge&#x27;s one bound, low risk, 202 lines; I, The dashboard&#x27;s change stream, low risk, 1,225 lines; J, Merge Assist&#x27;s sign-in read, low risk, 535 lines. Read first: engine-service.ts, untaken-events.ts, record-writes.ts. To skim: 10,420 lines of tests and 134 lines of fourteen release notes and seven docs pages.">

1. **Read first**, 15 min: `engine-service.ts`: `hookRequestSignal`, `answerFactsReader`, `recordHandedReceipt`, `untakenTaker`. Can an answer's facts or a kept receipt disagree with the session's records?
2. **Read first**, 10 min: `untaken-events.ts`: `leaveUntaken`, `untakenFor`, the taken mark. Can a hook skip its wait for a daemon that reads?
3. **Read first**, 10 min: `record-writes.ts`: `openRecordWriter`, `#keep`, `#awaitLock`, `#lockFreed`. Can a read miss a write still waiting for the lock?
4. 25 min: `hook-host.ts`, `hook-failures.ts`, `daemon.ts`, `daemon-control.ts`, `cursor-hook-adapter.ts`, `cursor-transcript.ts`, `daemon-entry.ts`, `runtime/index.ts`, `write-lock-wait.ts`, `write-lock-wait-worker.ts`, `session-store.ts`, `audit-store.ts`, `database.ts`, `audit-control.ts`, `runtime.ts`.
5. 25 min: `code-rule-bridge.ts`, `worktree-notices.ts`, `worktree-walks.ts`, `inotify.rs`, `main_instance.rs`, `home-daemon-lock.ts`, `file-lock.ts`, `native.ts`, `customer-profile.ts`, `packs-lock.ts`, `ci.yml`, `kept-runtimes.ts`, `memory-room.ts`, `heap-ceiling.ts`.
6. 15 min: `session-presentation.ts`, `cli.ts`, `event-answer.ts`, `adapter-contract.ts`, `claude-hook-adapter.ts`, `event-protocol.ts`, `host-commands.ts`, `code-rule-dsl-catalog.ts`, `event-stream.ts`, `codex-hook-adapter.ts`, `rerail-steer.ts`, `engine/index.ts`, `mcp-bridge.ts`.
7. 20 min: `poll.js`, `change-feed.ts`, `change-stream.ts`, `topbar.js`, `api.js`, `stores.ts`, `worktree-pack.ts`, `branch-git.ts`, `app.js`, `state.js`, `operation.ts`, `http.ts`, `sign-in.ts`, `service.ts`, `gh-config.ts`, `merge-assist-cli.ts`, `merge.js`, `model.ts`, `github.ts`.

**Then skim**, 30 min: Seventy test files: each wait has a case that fails over `main`'s source or with its line out.

## 4. Risk against verification

<img src="grid.png" width="544" alt="The changes by risk, high on top, against six checks. A, Silent daemon, high risk: users lose nothing holds with a limit. B, Records, lock, high risk: users lose nothing holds with a limit. C, Turn&#x27;s end, medium risk: the first five checks hold. D, Shell results, medium risk: users lose nothing holds with a limit. E, Lock, restart, medium risk: the first five checks hold. F, Reopen, rules, medium risk: users lose nothing holds with a limit. G, Event answer, low risk: the first five checks hold. H, MCP bridge, low risk: users lose nothing holds with a limit. I, Dashboard, low risk: the first five checks hold. J, Merge Assist, low risk: the first five checks hold. CI at this head does not hold yet for every change.">

Start at A and B: when a hook answers and when its records land.

<details><summary>Each limit, and each check that does not hold yet</summary>

| Change | Check | Its limit, or why it does not hold yet |
|---|---|---|
| A. Silent daemon | users lose nothing | Every event of a silence was kept and checked, 8 of 8 against none at `main`. The limit: a late check of an edit to every place can tell one line another program wrote, one more than `main`, never fewer. |
| B. Records, lock | users lose nothing | Records land 253 to 757 ms after the release where `main` lost them. The limit: behind a holder that changed nothing, a record lands at `main`'s time, 60.8 against 63.4 ms at the median. |
| D. Shell results | users lose nothing | A rule-breaking write is still steered at its own result, 10 of 10 at both builds. The limit: a write no notice tells of is checked at the turn's end or the ten-minute walk. |
| F. Reopen, rules | users lose nothing | The common hooks match `main` within 5 ms. The limit: compiled rules hold memory between sessions while the daemon has room, 33 to 43 MB for a first runtime with these packs. |
| H. MCP bridge | users lose nothing | 200 common calls take 1 ms at the median at both builds. The limit: a daemon that passes its checks late and never answers holds a call about 62 s, against 34 s. |
| A to J | CI at this head | CI runs once the pull request opens. |

</details>

<details><summary><b>5. The changes</b></summary>

**A. Hooks beside a silent daemon**, high risk. Hooks beside a silent daemon: 12.2 s each on `main`, 0.15 to 0.34 s after the first here.

- **Problem:** While the daemon's process lives but reads nothing, every hook waits its whole bound, then drops its event: the agent waits at each tool call, and nothing it does is recorded or checked.
- **Who meets it:** Every session whose daemon stops reading for a while, at each tool call of that silence. Shown through the product on Linux x64 and Windows 11.
- **What it does:** `leaveUntaken` writes a hook's event, after its bounded wait, into a folder of the Rerail home; while an event waits there, `untakenFor` lets each later hook leave its own at once. The daemon takes the folder in arrival order when it reads again, before any later event, and what it finds reaches the agent with the next answer.
- **Risk:** High: it decides which hook waits and when an event is checked; a wrong mark would skip a daemon that reads, or check events out of order.
- **Check:** Whether a hook can skip its wait for a daemon that reads (the taken mark, the folder's identity in the state file), whether every left event is checked before any later one, and whether a late check of an edit tells only lines the edit made.
- **Picture:** Eight hooks while the daemon's thread was held, through the built Claude Code hook on Linux x64: on main each waited up to 12,200 ms and none of the 8 events was kept; here the first waited 12,131 ms, the next 7 at most 340 ms, and all 8 were kept and checked.
- **Method:** The built Claude Code hook beside a real daemon of each build: 30 common tool calls, the daemon's thread held through 4 tool calls (8 hooks), then 2 more; a 16-core Linux x64 machine at load 13 to 28, `main` then this change; again this change first at load 2 to 6.

**B. Records off the hook thread**, high risk. A hook while another process holds the database: 5.2 s on `main`, 0.2 to 0.3 s here, steered.

- **Problem:** While another Rerail process holds the database's write lock (`rerail reset`, a scrub), each record waits SQLite's 5 s on the thread that answers hooks: hooks wait, no steer comes, and a record that outlasts it is lost.
- **Who meets it:** Every session while another Rerail process holds the database, at each hook. Shown through the product on Linux x64 and Windows 11.
- **What it does:** `openRecordWriter` keeps a record write in memory while the lock is held and makes it once a worker thread finds the lock free, trying at SQLite's own times and when the write-ahead log changes. Reads of this process's records see waiting writes. A steered hook hands its receipt to the daemon, which keeps it until the lock is free.
- **Risk:** High: it changes when every record is written; a lost or reordered waiting write would leave the Log without an audit.
- **Check:** Whether a read of what this process recorded can miss a waiting write, whether a waiting write keeps its 5 s bound, whether the worker can try later than SQLite's own busy wait would, and whether a receipt the daemon keeps is recorded once, after the release or at the next start.
- **Picture:** Hooks while another process held the database's write lock, through the built Claude Code hook and a real daemon on Linux x64, 3 runs a side: PreToolUse up to 5,369 ms on main and 262 ms here; the turn's end up to 15,464 ms on main and 311 ms here.
- **Method:** The built Claude Code hook and a real daemon; another process takes the database's write lock and keeps it; 3 runs a side, alternating, on a 16-core Linux x64 machine at load 16 to 27. The receipt: the time from a steered hook's answer to its exit, `main` first.

**C. A turn's end that gave up**, medium risk. The next prompt after a turn's end gave up: 30.2 s, then failing, on `main`; 0.2 to 0.5 s here.

- **Problem:** A turn's end whose hook gave up at its 30 s went on waiting for every check; the next prompt and tool calls queued behind it, each giving up, and its steers went into an answer no one read.
- **Who meets it:** A session whose turn ends while many checks still run, as after a formatter rewrites thousands of files. Shown through the product on Linux x64 and Windows 11.
- **What it does:** `#ingestTurnEnd` waits for the turn's checks through `settledBefore`, which ends when the hook's request closes. A turn's end no one waits for takes nothing the session holds (`#boundaryDeliveries`), so its steers go with the session's next event. A turn's end whose hook still waits is unchanged.
- **Risk:** Medium: a steer kept for the next event could reach the agent twice or never; it reached it once in every run.
- **Check:** Whether a turn's end can mark a steer delivered into an answer no hook reads, and whether a kept steer reaches the agent once, with the next event.
- **Picture:** After a formatter-like command rewrote 2,000 modules and the turn ended while checks ran, Linux x64, 2 runs a side: on main the next prompt, a Read and its result each gave up at up to 30,248 ms; here every prompt and tool call answered in at most 519 ms.
- **Method:** A real daemon and the built Claude Code hook in a checkout of 2,000 modules; a formatter-like shell command rewrites them, four with a rule break, and the turn ends while the checks run; then turn after turn. 2 runs a side, the order turned, on a 32-core Linux x64 machine at load 21 to 27.

**D. Shell results by change notices**, medium risk. A shell command's result at 100,000 files: 1.33 s on `main`, 0.19 s here, still steered.

- **Problem:** Every shell command's result walked the whole session folder to find what the command wrote, and the hook waited for that walk up to 10 s.
- **Who meets it:** Every session in a large checkout, at each shell command's result. Shown through the product on Linux x64 at 100,000 files.
- **What it does:** `SessionNotices` keeps the paths the folder's watch reported since the last pass, and `#finishSessionWalk` reads only those at a command's result. A turn's end walks the whole folder, and so does the first result ten minutes after the last whole walk began: those catch the writes no notice tells of. No timer runs.
- **Risk:** Medium: a write no notice tells of is steered at the turn's end or the ten-minute walk, not at its own result.
- **Check:** Whether a write a common tool makes can leave no notice, whether a notice that spells a path otherwise than the folder holds it leads to a whole walk, and whether a file a command was still writing is read at the next result.
- **Picture:** A read-only shell command's result in a checkout of 100,000 Python files, 10 runs on a 32-core Linux x64 machine: 1,330 ms at the median on main against 190 ms here; the daemon's CPU 1,630 ms against 50 ms.
- **Method:** A real daemon and the built Claude Code hook in a checkout of 100,000 Python files, `main` with its own engine, after each build's first whole walk ended; 10 runs of a read-only command and 10 of one that writes a rule-breaking file; a 32-core Linux x64 machine at load 8 to 12.

**E. The daemon's lock and restart**, medium risk. Old daemon slow to exit: a daemon still runs 0 of 5 times on `main`, 5 of 5 here.

- **Problem:** A restarting daemon launches its successor, which tried for the home's lock every 50 ms for 1 s; an old process slower to exit left no daemon running. A daemon started beside a running one waited a second to stop.
- **Who meets it:** A person whose daemon restarts through its memory watch while its old process is slow to exit. Shown through the CLI on Linux x64 and Windows 11.
- **What it does:** The engine (`main_instance.rs`) hands a waiter the lock at its release; `home-daemon-lock.ts` and `file-lock.ts` wait on that release instead of a 50 ms poll. A daemon started beside a running one stops as soon as that daemon answers as the home's own; only a holder that does not answer gets the one-second wait.
- **Risk:** Medium: a wrong hand-over could leave two daemons in one home, or none.
- **Check:** Whether two waiters can both take the lock at one release, whether a waiter's bound ends its wait on Windows (an overlapped `LockFileEx` cancelled), and whether a holder that does not answer still gets the one-second wait.
- **Picture:** A restart through the real CLI whose old process takes 3 s to exit, Linux x64, 5 runs: a daemon left running 0 of 5 times on main, 5 of 5 here. A daemon started beside a running one stops in up to 1,485 ms on main, 772 ms here.
- **Method:** The CLI starts a real daemon in a fresh home; a preload has it restart through its own memory watch and take a set time to exit after it launched its successor; 5 runs a case, `main` with its own engine; a 16-core Linux x64 machine at load 2 to 5.

**F. A reopened session and kept rules**, medium risk. First read after a reopen, the content store held: 10.7 to 10.8 s on `main`, 0.6 s here.

- **Problem:** A session opening again after a daemon restart held every tool call, a read too, until its rules opened, up to 20 s each. A session back 15 minutes after the last one compiled its rules again.
- **Who meets it:** A session reopened after a daemon restart, an idle close or a resume, and a session started where the last one left over 15 minutes before. Shown on Linux x64 and Windows 11.
- **What it does:** A reopened session holds only its first call that can write (an edit, a shell command) for its rules, once; a read goes through, and each turn's end still waits. `KeptRuntimes` keeps a checkout's compiled rules while `memory-room.ts` finds room; short of room they close 15 minutes after the last session, and before the heap ceiling restarts the daemon.
- **Risk:** Medium: kept rules hold memory between sessions; a wrong reading of room would keep it on a machine that needs it.
- **Check:** Whether a call that can write can pass before its rules open, whether the room check reads the heap, the resident size and the machine's free memory on each system, and whether kept runtimes close before the heap ceiling restarts the daemon.
- **Picture:** Linux x64. A session reopened after a daemon restart while another process held the content store: its first read waited 10,817 ms on main and took 621 ms here. A new session 1,000 s after the last left: SessionStart 549 ms on main, 210 ms here.
- **Method:** The built Claude Code hook beside a real daemon of each build, 3,000 TypeScript files; another process holds the content store's write lock 45 s as the session reopens, 2 runs, at load 4 to 9. The later session: both builds at once at load 1 to 6, one sample a build. 16-core Linux x64 machines.

**G. Note and link in the answer**, low risk. A session start beside a busy daemon: 4.2 s with no rule count on `main`; 0.2 s, counted, here.

- **Problem:** After the event's answer, the hook read the daemon again for the note's rules and link, and a steer for its link, each with 2 s; beside a daemon busy with what it answered, the note came late, uncounted.
- **Who meets it:** Every session start and steer while the daemon is busy, and `/rerail` beside a daemon that does not answer. Shown on Linux x64 and Windows 11.
- **What it does:** The runtime that takes an event says in its answer what the note and the links need (`EventAnswerFacts`), and the Claude Code, Codex and Cursor hooks read it there. `/rerail` takes its rules page from its one read of the daemon; `--open` still asks again. An older daemon's answer lacks the fields, and the hook reads after it, as before.
- **Risk:** Low: a wrong field would give the note a wrong link or count; rules and steers do not change.
- **Check:** Whether the answer's rule count is the one the note read before, and whether an older hook or daemon still works with the other's answer.
- **Picture:** Linux x64. A session start beside a daemon busy after it answered: up to 4,212 ms with no rule count on main, 205 ms counted here. rerail session for Cursor beside a daemon that never answers: 4,191 ms on main, 2,193 ms here.
- **Method:** A real daemon per build; the Claude Code, Codex and Cursor session-start hooks and a Claude Code steer, 3 runs each, the daemon healthy, busy after it answered, or its rule configuration's read held; `rerail session` beside one stopped daemon of 190 worktrees; 16-core Linux x64 machines at load 2 to 5.

**H. The MCP bridge's one bound**, low risk. A call beside a daemon busy 5 s: "not running" at 2.1 s on `main`; answered at 5.1 s here.

- **Problem:** The MCP bridge gave the daemon 2 s to answer its health check and told the agent the daemon was not running when a busy one missed it, then checked again on a timer between calls.
- **Who meets it:** An agent using Rerail's tools while the daemon is busy. Shown through the built bridge on Linux x64 and Windows 11.
- **What it does:** A call has two bounds, each with its own signal: 30 s for the checks before it is sent, from the moment the bridge takes it, then 30 s for the daemon's answer. A wait that starts with under 2 s of its part left gets the 2 s it had before. No timer runs between calls.
- **Risk:** Low: a call can wait longer, about 62 s at most where a daemon passes its checks late and never answers.
- **Check:** Whether every wait ends, and whether any part of a call gets less time than on `main`.
- **Picture:** The built MCP bridge on Linux x64: an idle bridge made 222 context switches a minute on main and 10 here. Beside a daemon busy 5 s, main said the daemon was not running at 2,125 ms; this change answered at 5,103 ms.
- **Method:** The built MCP bridge beside a real daemon whose thread a preload holds 5 s or 33 s; 200 common calls; an idle minute's context switches; a 16-core Linux x64 machine at load 2.

**I. The dashboard's change stream**, low risk. A change on an open dashboard: 3.7 s at the median on `main`, 6 ms here.

- **Problem:** The dashboard read everything again every 5 s while its tab showed, three to five daemon handlers a read, whether or not anything changed; a change reached the page seconds after it was written.
- **Who meets it:** Everyone with the dashboard open. Shown through the page's own `ui/poll.js` on Linux x64 and Windows 11.
- **What it does:** The page follows one stream from the daemon (`GET /api/changes`): a token at once, again when any process's commit becomes readable, a pack, a checkout's rules or its branch moves, and a message every 5 s. The page reads when the token moves, at most every 5 s. A page with no stream reads every 5 s.
- **Risk:** Low: a missed token would leave the page on old data until the next message; agents see nothing of it.
- **Check:** Whether every change a page shows moves the token (another process's commit, a branch Git writes with no commit), and whether a page without a stream falls back to its 5 s reads.
- **Picture:** The page's own poll.js against the built daemon on Linux x64: a write after a quiet spell reached the page in 3,715 ms at the median on main and 6 ms here; an idle minute made 24 page requests on main and 2 here.
- **Method:** The page's own `ui/poll.js` against the built daemon of each build; another process writes a repository's record after a quiet spell, 8 times; an idle minute; then one write a second for a minute; a 16-core Linux x64 machine at load 2 to 6.

**J. Merge Assist's sign-in read**, low risk. A Merge Assist status past five minutes: 1.3 s on `main`, 20.3 s if `gh` stalls; 0.2 s here.

- **Problem:** Merge Assist kept its reading of `gh`'s sign-in five minutes; a status past that ran `gh` in its own answer, up to 20 s when it stalled, and a sign-in changed outside Rerail stayed unseen until then.
- **Who meets it:** People trying Merge Assist, which is in internal testing, at each status past five minutes from the last reading. Shown on Linux x64 and Windows 11.
- **What it does:** `sign-in.ts` compares the stamp of `gh`'s `hosts.yml` (its identity, size and times, one stat off the daemon's thread) with the one kept beside the reading, so a login, logout or switch shows at the next status. Past an age the daemon reads `gh` in the background; the CLI, which answers once, reads it in its answer.
- **Risk:** Low: a missed stamp would show an old sign-in until the age runs out, as on `main`; the feature is in internal testing.
- **Check:** Whether a stat that answers late can write over a newer reading, and whether a listing whose `hosts.yml` moved lists again with the new reading.
- **Picture:** Merge Assist's status through the built CLI against a real daemon on Linux x64: with a sign-in reading 6 minutes old, 1,269 ms at the median on main and 195 ms here; with the GitHub CLI stalled, 20,251 ms on main and 201 ms here.
- **Method:** The built CLI and the built MCP bridge against a real daemon, a stand-in GitHub CLI on the path that answers after a set delay; a 16-core Linux x64 machine at load 2, `main` first.

</details>

<details><summary><b>6. The decisions</b></summary>

**After the daemon left an event untaken, does each later hook wait?**

**We chose** the first hook of a silence waits; later ones leave their events in order and answer at once. **Would change our mind:** a case where an event's check at its own hook matters more than 12 s a hook.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| The first waits; later ones answer at once, checked in order later **(chosen)** | The first hook 12.1 s; the next 7 0.15 to 0.34 s; all 8 recorded and checked. | A steer for such an event comes at a later tool call. | Nothing measured. |
| Each waits 12 s and drops its event (`main`'s) | A steer at its own hook when the daemon answers in time. | 12.1 to 12.2 s each. | The events: none of the 8 recorded. |
| A shorter wait | Not built. | A busy daemon taken for a dead one sooner. | More events lost. |

**Evidence.** The built Claude Code hook beside a real daemon of each build: 30 common tool calls, then the daemon's thread held through 4 tool calls, then 2 more; a 16-core Linux x64 machine at load 13 to 28. Again with this change first at load 2 to 6: the common hook path a median of 169 ms at both builds.

**What does a shell command's result read?**

**We chose** the noticed files; the whole folder at a turn's end and ten minutes after the last whole walk. **Would change our mind:** a common editor or tool whose writes the watch misses, seen as an edit checked only at a turn's end.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| Noticed files; whole walks at a turn's end and every ten minutes **(chosen)** | 0.19 s at 100,000 files, the daemon's CPU 0.05 s. | A write no notice tells of (a hard link from outside the folder, a held memory map, on Windows a same-size write to an open file) waits for the turn's end or the whole walk. | Nothing measured. |
| The whole folder at every result, up to 10 s (`main`'s) | Every write checked at its own result. | 1.33 s at the median, the daemon's CPU 1.63 s. | Seconds at each result in a large checkout. |
| The notices only, the whole folder at a turn's end alone | As chosen. | Nothing more. | Such a write waits for the turn's end, however long the turn. |

**Evidence.** A real daemon and the built Claude Code hook in a checkout of 100,000 Python files, 10 runs of a read-only command and 10 of one that writes a rule-breaking file; a 32-core Linux x64 machine at load 8 to 12. In 11-minute turns, a write no notice tells of was steered at 600.3 s here and at its own result on `main`.

**Do a repository's compiled rules stay in memory after its last session?**

**We chose** kept while the daemon has room; short of room, freed 15 minutes after the last session, as on `main`. **Would change our mind:** a machine whose other work misses the memory a kept repository holds.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| Kept while the daemon has room; short of room, freed after 15 minutes **(chosen)** | SessionStart 210 ms, 1,000 s after the last session left. | The memory the rules hold between sessions: 33 to 43 MB for a first runtime with these packs. | Nothing measured. |
| Freed 15 minutes after the last session (`main`'s) | The memory back after 15 minutes. | 549 ms, the rules compiled again. | Nothing; a slower start. |
| Keep a fixed number of repositories | Not built. | A count to choose. | A count that fits no machine. |

**Evidence.** Both builds at once on a 16-core Linux x64 machine, a real daemon each, 3,000 TypeScript files; a new session 1,000 s after the last: SessionStart 549 against 210 ms, the daemon's CPU over that turn 1,210 against 690 ms; one sample a build. Short of room, the runtime was freed between minutes 14 and 16 at both builds.

</details>

<details><summary><b>7. How it fits</b></summary>

Nothing has to merge first: it holds `main` at 06a1a1ce8 and needs no other pull request. Part of #737, as its second question asked. Left out: the store's writes taking turns, to be redone on the new store; arrival-order checks, whose wait the new engine no longer shows.

This pull request holds main at 06a1a1ce8 and needs nothing else. #921 merges clean.

**Trial merges with this head.** `main` 06a1a1ce8 is merged in. Of what merged today, #883, #912 and #903 merged clean beside it; #904, #812 and #909 met it in `worktree-notices.ts`, `runtime.ts` and `engine-service.ts`, #922 in `code-rule-bridge.ts`, `runtime.ts`, `session-store.ts` and a doc, #924 in eight files, #920 in `runtime.ts`, two docs and a test file, and #915's invented names in 13 test and source files; each file keeps both sides. Open: #921 shares no file and merges clean.

</details>

<details><summary><b>8. What is not covered</b>: 6 not verified, 5 limits that stay</summary>

1. **A write no notice tells of** is checked at the turn's end or the next whole walk, not at its own result, as the release note says.
2. **A record behind a quiet holder** lands at `main`'s time: a holder that changed nothing leaves no sign of its release.
3. **Codex and Cursor** ran their answers' new fields in unit cases only; every product run drove Claude Code's hooks.
4. **The dashboard under one write a second** was measured before its last fix only: 3.0 s against `main`'s 2.6 s at the median.
5. **The change stream's last reads** stay in the daemon's memory for its life: one short entry per part for each checkout Rerail recorded that a stream named.
6. **macOS** ran in none of our runs; CI runs the tree watch's tests there.

**Limits that stay**

- On Windows the file system reports an open file's growth only once it is flushed, so a record waiting for another process's lock keeps SQLite's own times there, as on `main`.
- The first edit of a session's first start can be steered at the next hook while the rules still compile, on `main` as here: 5 of 19 and 4 of 19.
- Left out, a check's turn on the engine in arrival order: on the 0.2.0 engine `main`'s check beside a large scan no longer waits (153 to 200 ms, against 1.4 to 3.1 s before).
- Each change was measured through the product on Windows 11 when it was built, at older `main` commits; today's figures are Linux x64.
- Behind a proxy other than nginx that holds a streamed response, the page reads every 5 s from when it opens, as `main` does: a change shows at the next read.

</details>

## 9. What we ask of you

We ask no ruling.

<details><summary><b>10. Evidence</b></summary>

The description is the record: "How you tested it" holds the runs and each wait's measure in its folds, and "What is not covered" the limits.

| Where | What ran | Result |
|---|---|---|
| Linux x64 | CI's lint and type-check at the merge of `main` 06a1a1ce8 (fe17eb24f); the engine's library tests that read the source tree (`rerail-logic`) at cc2828e54, `main` beside them | Pass; `rerail-logic` 411 pass and 7 are ignored, as at `main`; the merges since bring no change to the Rust this change edits. |
| Linux x64 | The 308 test files the change adds or edits, and those importing a changed module (4b57f6301), at load 20 to 25 | Only `packs.test.ts` and `complexity-rules.test.ts` fail, as on `main`, and nine 5 s limits; beside `main` bf24c778f, three runs a side, the same two cases fail at both builds and the rest pass at both. |
| Windows 11 | The same 286 unit files (f2b7ce23a), four machines | Only those two files and 30 to 60 s limits on machines running other work; the 19 files that held them, `main` 6ec384ad9 then this change on one machine: none failed at either (541 and 523 cases passed). |
| Linux x64 | CI's integration step (4b57f6301), at load 19 | 23 files passed, 1 skipped; 95 tests passed. |
| Linux x64 | `claude-hook-rules` and `event-adapter-parity` alternating with `main` df9358d3f, 10 runs a side, at load 16 to 27 | Both fail at both builds: the session's first edit not steered at its own result 3 of 10 here and 1 of 10 on `main` (the first start's rules still compiling, `main`'s design), and the 5 s limit 5 of 10 at each. |
| Linux x64 | The repository's reviewer rules over the changed files (fe17eb24f), `main` 06a1a1ce8 built beside it | No new finding; 1,017 functions added or edited, none over complexity 6. |
| Linux x64 | The test files of each file both this change and `main` since d54b586a8 edit, and of #920's, with those importing them (43), at fe17eb24f, beside `main` 06a1a1ce8; CI's browser tests | Linux x64, on a loaded machine: 845 pass and 13 fail at their 5 s limits, against 651 and 77 at `main`; the 3 files holding the 13 pass on a quieter machine at all three refs. Windows 11: 855 pass here and 725 at `main`, none fails. The browser tests: 86 here, 82 at `main`. |
| Linux x64 | Each wait through the product: `main` df9358d3f beside this change at 022469d1f on one machine, its load given | Each wait in the description's table shows at `main` and not here. |

**Sources.** The branch at fe17eb24f, read on 20 November 2026; `main` at 06a1a1ce8, d54b586a8, 9b93ecd80, e3b4112ce, d5f941195, d69d7ab68, 1328a4e38, 95ad00618, 1e915f661, 6ec384ad9, bf24c778f, df9358d3f, a5ed5ccb8 and 2221b5ec9. Each wait's figures come from runs through the product on 20 November, on Linux x64; earlier checks of each change from its own runs on Linux x64 and Windows 11.

</details>
