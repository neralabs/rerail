# A stopped checkout's hook answers without waiting for a busy daemon

For [pull request #674](https://github.com/neralabs/rerail/pull/674), head 5d7404689. [report.html](report.html) is the same report as one page: download it and open it.

## 1. The decision

**May a hook repeat the daemon's last answer of stopped for its folder, for up to 30 minutes, when the daemon is 75 ms late?**

1. A stopped checkout's hook repeats a late daemon's last answer. **Changes the intent.**
2. The reference drops "never contact the daemon". **Changes the intent.**

**We recommend:** answer #670 first, then read "What is not covered": it names the states where a hook repeats stopped for up to 30 minutes while the daemon reads started.

| Size | Risk and review | Waits on | Not verified |
|---|---|---|---|
| 6,218 lines in 35 files: 1,670 ship, 4,548 are tests and text | Careful. A code owner approves: it touches `src/engine/storage/`. | #670's answer. No CI result at this head yet. | 6 |

## 2. Intent, and the gap

| Change | Intended | Found | Now |
|---|---|---|---|
| A. The start check | A running daemon answers first _(code comment)_ | About 2 s per hook _(timings)_ | **CHANGES THE INTENT: ANSWER 1.** The hook repeats stopped. A maintainer must agree. |
| A. The reference | Hooks never contact the daemon _(docs)_ | One request per hook _(counts)_ | **CHANGES THE INTENT: ANSWER 2.** The reference says it asks. A maintainer must agree. |
| B. Its own answer | The daemon's own answer _(code comment)_ | No read gives it _(earlier heads)_ | Closes the gap. |
| C. A start since | Its open database answers _(code comment)_ | A copy hides a start _(runs)_ | Closes the gap. |
| D. The open | An open that changes nothing _(code comment)_ | It can wait 5 s _(tests)_ | Closes the gap. |
| E. The queue | A 12 s wait _(code comment)_ | Given-up checks stay queued _(bursts)_ | Closes the gap. |
| F. The client | Plain fetch _(code comment)_ | Given-up reads leave empty connections _(counts)_ | Closes the gap. |
| G. The request | One request with a limit _(code comment)_ | It keeps the process alive _(mutation)_ | Closes the gap. |

Reported in #670 on 13 November 2026; timed on Linux x64 and Windows 11.

<details><summary>Sources at main, and causes</summary>

| Change | Where main states the intent | The cause, and what was measured |
|---|---|---|
| A. The start check | `src/harness/hook-host.ts` lines 61 and 62 at `main`, above `hookRerailStarted`: "The check one-shot hooks make. A running daemon answers first, from its own environment and open database; without one, the hook reads the database itself, which never starts a daemon." | `hookRerailStarted` awaits `readRepositoryState` (`src/harness/hook-host.ts` line 67 at `main`), whose read ends after `READ_TIMEOUT_MS`, 2 s (`src/presentation/session-presentation.ts` line 79). A request to a busy daemon waits behind other sessions' work. Measured with Claude Code, Codex and Cursor hooks through a real daemon held busy in 900 ms turns, `main` at e8a6b0b5a: 1,548 to 1,806 ms on Linux x64, 2,470 to 3,470 ms on Windows 11. |
| A. The reference | `docs/reference.md` line 849 at `main`: "A stopped checkout records nothing, runs no rules, delivers no guidance, and its hooks never autostart or contact the daemon." | A stand-in daemon counted one request per hook in a stopped checkout at `main`, on Linux x64 and Windows 11. `hookRerailStarted` asks before it reads anything, in every checkout. An earlier head of this branch read first and kept the reference's words; a started checkout's start check then took 20 against 11 ms on Linux x64 and 25 against 15 ms on Windows 11. |
| B. Its own answer | `src/harness/hook-host.ts` lines 61 and 62 at `main`: a running daemon answers "from its own environment and open database". `src/engine/catalog/repository-registry.ts` line 219, above `lookupCheckout`: a fresh clone "is still found by its remote, with \`cloneId\` null". | At 3977f531f the hook answered from Git in its own environment, and `GIT_CEILING_DIRECTORIES` set otherwise than the daemon's made it read a started checkout as stopped. At 6993ee854 it read the folders on disk with no Git, and differed from the daemon in states such as a WAL deleted under the daemon. The daemon reads the clone id file and its open database on every answer. |
| C. A start since | `src/harness/hook-host.ts` lines 61 and 62 at `main`: a running daemon answers "from its own environment and open database". At `main` the hook asks that daemon every time, so a copy written over the file changes no answer. | The daemon's connection holds the pages it read. `cp` over `rerail.sqlite` keeps the file's id, so every check of the kept answer agreed at d5d45c2e3 and the hook repeated stopped: through a real daemon, on both systems, that build sent 0 of 3 events where `main` sends 3 of 3. The file's size or time changed in 60 of 60 windows from checkpoints alone, so neither tells. |
| D. The open | `src/engine/storage/database.ts` line 226 at `main`, above `openRerailDatabaseAsIs`: "A read-only open that never changes the file". In `connectSqlite`, line 44: `PRAGMA busy_timeout = 5000`, for every connection. | `connectSqlite` gives every connection a 5 s busy timeout, and SQLite's open follows a link and waits for a writer on a named pipe. On Windows SQLite retries a refused read for about 1.4 s, which no busy timeout bounds: the four lock tests took 1,527 to 1,579 ms at 2a88553c5. |
| E. The queue | `src/runtime/daemon-control.ts` line 1003 at `main`, above `HOOK_DAEMON_WAIT_MS`, the sum of a 2 s first wait and 10 s more: "How long, in all, a hook waits for a busy daemon from the event's arrival: the first probe and the busy wait after it." | A request its hook ended stays queued at a busy daemon, which serves it though nobody waits; on Linux the daemon admits one connection per turn of its loop. At 667ab68a3, 8 stopped-checkout hooks that repeated stopped left 8 start checks queued, and a started checkout's hook gave up at 12 s with 0 of 4 events. At `main` e8a6b0b5a the same follows 6 groups of 4 hooks at once: 12,075 to 12,138 ms, 0 of 4 (Linux x64). |
| F. The client | `src/runtime/daemon-control.ts` line 201 at `main`, above `daemonFetch`: "How a Rerail process reaches the daemon \`state\` records: plain \`fetch\` for an \`http:\` daemon". Line 206 returns `fetch` for every daemon that is not `https:`. | Node's bundled fetch opens a new connection once a request's signal aborts, and sends nothing on it. A test daemon counts two connections for one read that gives up at `main`, and one here. Through a real busy daemon at 4662bd526, the first hook after a burst of 24 took 2,068 to 2,144 ms, past the 2 s read limit, and each such hook left one more empty connection (Linux x64). |
| G. The request | `src/presentation/session-presentation.ts` lines 427 and 428 at `main`, above `readRepositoryState`: "Whether Rerail is started in \`cwd\`'s repository, from the live host daemon, or null when none answers. One request: the recorded pid must be alive and the reply must parse as a presentation." In `get`: `AbortSignal.timeout(READ_TIMEOUT_MS)`. | `get` ends its read only on its 2 s limit, and on Windows `engineCapabilityProof` waits up to 2 s for the engine. With the request left open at 0c9ad801e, 14 tests fail on Linux x64. Beside a suspended daemon, `main`'s hook took 2,134 ms and this change's 199 ms (Linux x64, 9c4f9298d). |

</details>

## 3. Where to look

<img src="map.png" width="544" alt="The changed files as bars sized by changed lines, grouped by change and coloured by risk. Code to read: A, Ask first, repeat after a grace, medium risk, 117 lines; B, The kept answer and its checks, medium risk, 613 lines; C, The start mark, medium risk, 223 lines; D, The check&#x27;s open of the database, medium risk, 116 lines; E, Marks of given-up start checks, medium risk, 371 lines; F, One kept connection per process, medium risk, 226 lines; G, The hook ends its request, low risk, 4 lines. Read first: hook-host.ts, stopped-answers.ts, waiting-asks.ts. To skim: 4,511 lines of tests and 37 lines of the reference, the threat model, two release notes.">

1. **Read first**, 6 min: `hook-host.ts`: `hookRerailStarted`, `daemonAnswer`. Can it answer stopped, or throw, where the daemon says started?
2. **Read first**, 15 min: `stopped-answers.ts`: `repeatsStoppedAnswer`, `stillHolds`. Does any check pass where the daemon reads started?
3. **Read first**, 8 min: `waiting-asks.ts`: `HookAsk`, `ServedAsks`. Can a waiting mark outlive its folder's requests?
4. 6 min: C: `start-mark.ts`: `renewStartMark`, `startMarkHolds`.
5. 10 min: E to G: `session-presentation.ts`, `daemon.ts`, `transport.ts`, `daemon-control.ts`, `engine-proof.ts`.
6. 5 min: D: `database.ts`.
7. 6 min: B, C: `repository-identity.ts`, `setup-cli.ts`, `runtime/index.ts`, `repository-registry.ts`, `repository-start.ts`, `repository-setup.ts`, `engine/index.ts`.

**Then skim**, 12 min: Read `generatedTrees` in `hook-host.test.ts` and the reference's stopped-checkout paragraph.

## 4. Risk against verification

<img src="grid.png" width="544" alt="The changes by risk, high on top, against six checks. A, Ask first, medium risk: independent review holds with a limit, users lose nothing holds with a limit, Windows 11: no new failure holds with a limit. B, Kept answer, medium risk: independent review holds with a limit, users lose nothing holds with a limit, Windows 11: no new failure holds with a limit. C, Start mark, medium risk: independent review holds with a limit, users lose nothing does not hold yet, Windows 11: no new failure holds with a limit. D, Database open, medium risk: independent review holds with a limit, users lose nothing holds with a limit, Windows 11: no new failure holds with a limit. E, Waiting marks, medium risk: independent review holds with a limit, users lose nothing holds with a limit, Windows 11: no new failure holds with a limit. F, One connection, medium risk: independent review holds with a limit, users lose nothing holds with a limit. G, Request ends, low risk: independent review holds with a limit. CI at this head does not hold yet for every change.">

Start at C's third mark: a start that renews no mark.

<details><summary>Each limit, and each check that does not hold yet</summary>

| Change | Check | Its limit, or why it does not hold yet |
|---|---|---|
| A. Ask first | independent review | A second run from a separate checkout repeated the timings at 6f0152ac8 and found the same gain. The automated reviewer approved 6993ee854, an earlier design, and has not reviewed this head. The limit: no person on the team has reviewed it. |
| A. Ask first | users lose nothing | Beside an idle daemon each hook asks once and takes the daemon's answer, as at `main`. The limit: it then cost +0.7 to +4.6 ms more than `main`'s, paired medians at 6f0152ac8, before a hook made a pending mark at each ask. The timings are hook processes beside a real daemon, not agent sessions. |
| A. Ask first | Windows 11: no new failure | On Windows 11, of the 30 test files at the head's source, one old case failed on timing on a loaded machine, as at 2b2192f9f there (3 of 5 runs). The limit: the whole suite did not run at this head, and the Windows timings ran beside other test runs. |
| B. Kept answer | independent review | Second runs from a separate checkout found a clone id the check did not read, a named pipe that held the hook and a database moved aside under the daemon; a9bbc7a07, d9dc055aa and 7153a6ce8 fix them. The limit: no person on the team has reviewed it, and the automated reviewer has not seen this design. |
| B. Kept answer | users lose nothing | A test asks two hooks and the daemon about each of 40 folders: 0 where the hook repeats stopped and the daemon reads otherwise, on both systems. The limit: the 57 states that record what `main` records through a real daemon ran at 6f0152ac8 or its source, and What is not covered names where one is stale. |
| B. Kept answer | Windows 11: no new failure | On Windows 11 the link, share, held-file and lock tests of `hook-host.test.ts` pass at the head's source. The limit: the named-pipe tests cannot run there, since Windows makes no named pipe in a folder, and at 948a7e06f one case, of a sync tool's copy, failed 1 of 10 runs on a rename Windows refused, at 4662bd526 alike. |
| C. Start mark | independent review | A second run from a separate checkout at 9c4f9298d found a 2 s wait on a held mark and a folder that takes no file; 47786743e and 6f0152ac8 fix both. At 6f0152ac8 it found two starts that renew no mark; the reference names them. The limit: no person on the team has reviewed it. |
| C. Start mark | users lose nothing | Does not hold in the first states What is not covered names. Where a start renews no mark and an older copy of the database is then written in place, hooks send 0 of 3 events where `main` sends 3 of 3: run at 6f0152ac8 for a linked and a refusing answers folder. Every other state run matches `main`. |
| C. Start mark | Windows 11: no new failure | On Windows 11 at 6f0152ac8, with another program holding the mark, a start or stop through the API takes 4.4 to 7.6 ms (`main`: 3.4 to 5.6) and no hook waits longer than at `main`. The limit: Windows checks no folder permissions here, so a folder that refuses new files goes unseen. |
| D. Database open | independent review | A second run from a separate checkout found that a byte-range lock on the database cost each Windows hook 1.1 s more than `main`'s; 2db93395b reads the first page and the WAL's header first. The limit: no person on the team has reviewed it, on a high-risk path. |
| D. Database open | users lose nothing | Every other caller opens as before: `besideDaemon` defaults to false and `openChecked` is unchanged. The limit: on Windows a lock past the database's first page still costs a hook about 1.1 s more than `main`'s, with the same events (1,767.5 against 648.6 ms, 9c4f9298d). |
| D. Database open | Windows 11: no new failure | The Windows lock tests, in `hook-host.test.ts`, pass at the head's source. With the whole file, the header, the first page or the WAL locked, a hook takes -15.4 to +11.0 ms against `main`'s, paired (9c4f9298d). The limit: a lock past the first page, and `database-baseline.test.ts` is not among the 29 test files run at 2b2192f9f. |
| E. Waiting marks | independent review | A second run from a separate checkout at 4662bd526 found that a pending mark with no request behind it, left by a killed hook or a start check that outlived its hook's answer, stopped the folder's hooks from marking; 948a7e06f fixes it. The limit: neither a person on the team nor the automated reviewer has reviewed it. |
| E. Waiting marks | users lose nothing | On Linux x64 a started checkout's hook keeps 4 of 4 events after bursts of 8 and 24 hooks, a planted mark and a killed hook, five rounds each (948a7e06f). The limit: hooks that start together each leave their check queued. On Windows 11, 1 of 35 started-checkout hooks counted 2 of 4 events; its cause was not traced. |
| E. Waiting marks | Windows 11: no new failure | On Windows 11 both changed test files pass whole at the head's source, and the five new cases fail at 2b2192f9f. The limit: on a machine running other tests, 4 of 20 runs of the new and nearby cases failed on timing, old cases among them. |
| F. One connection | independent review | A second run from a separate checkout at 4662bd526 found that the first one or two hooks after a burst passed the 2 s read limit, each leaving one more empty connection; a88b29ed6 and 6a31a35b0 fix it. The limit: neither a person on the team nor the automated reviewer has reviewed it. |
| F. One connection | users lose nothing | Every request to an `http:` daemon goes over Node's http client and one keep-alive agent; a GET whose kept connection closed is sent once more, a POST never. The 30 test files run at 5d7404689 pass on Linux x64. The limit: a daemon serving https gets no gain, and the whole suite did not run at this head. |
| G. Request ends | independent review | The automated reviewer raised nothing on the stop signal in `session-presentation.ts`, `daemon-control.ts` and `engine-proof.ts` at 6993ee854, an earlier design. Since then `readStartAnswer` carries the signal beside the hook's ask. The limit: no person on the team has reviewed it. |
| A to G | CI at this head | No CI result at 5d7404689 yet. On GitHub, CI last ran at 6993ee854, an earlier design (run 43622328319, read on 14 November 2026): 28 checks passed and none failed, on the run's second attempt. CI's jobs that need no secret ran at 32e88850e on Linux x64: the release-note check, the build and the integration job each exit 0. |

</details>

<details><summary><b>5. The changes</b></summary>

**A. Ask first, repeat after a grace**, medium risk. Hook beside a busy daemon: 1,548 to 1,806 ms at `main`, 86 to 196 ms here (Linux x64).

- **Problem:** In a checkout where Rerail is stopped, every Claude Code, Codex and Cursor hook asks the daemon whether Rerail runs there and waits up to 2 s. A daemon busy with other checkouts holds each one, for an answer of `{}`.
- **Who meets it:** A person who stopped Rerail in some checkouts while sessions in others keep the daemon busy: every hook there. How many people work this way is not known. #670 reports one laptop with about 40 such sessions.
- **What it does:** `hookRerailStarted` asks the daemon first. `daemonAnswer` finds the start mark (`markBeforeAsking`), asks through the hook's `HookAsk`, and waits the grace out (`settlesWithin`). An answer in time is kept (`keptAnswer`). Past the grace, `stoppedEndingRequest` asks `knownStopped` whether the last answer may be repeated; if so, the hook answers stopped, marks its request (`abandoned`) and ends it. Elsewhere it waits.
- **Risk:** Medium. This check decides whether a hook does anything, for three coding agents. By itself it only repeats stopped or throws on its own fault, once the daemon is late or a check of the folder waits there.
- **Check:** Is `stopped` the only answer `daemonAnswer` gives by itself? When the check throws anything but SQLite's error, `stoppedEndingRequest` ends the request and the hook answers `{}` with a line on stderr, in a started checkout too; at `main` it waits for the daemon there. The mark is found before the ask, so a start that commits while the daemon answers leaves nothing to repeat. A hook that does not ask (`unasked`) answers stopped only where `knownStopped` held.
- **Picture:** Time of a stopped-checkout hook beside a busy daemon, lowest to highest; each bar ends at the highest. Linux x64: 1,548 to 1,806 ms at main, 86 to 196 ms with this change. Windows 11, beside other test runs: 2,470 to 3,470 ms at main, 353 to 777 ms with this change.
- **Method:** Claude Code, Codex and Cursor hooks in a stopped checkout, each started as its agent starts it, through the build's own daemon held busy in 900 ms turns of its loop: `main` at e8a6b0b5a against 948a7e06f, each build in a Rerail home of its own. Linux x64 on one machine; Windows 11 on one desktop machine beside other test runs.

**B. The kept answer and its checks**, medium risk. 30 minutes of ordinary running: 60 of 60 later hooks repeated, 163.3 ms against `main`'s 533.7 (Linux x64).

- **Problem:** A hook cannot place a folder as Git does from the disk alone. Earlier heads of this branch decided stopped from the folders and the database, and each differed from the daemon in some state.
- **Who meets it:** Every hook in a stopped checkout beside a late daemon takes the kept answer. A wrong stopped leaves a started checkout unrecorded and unchecked for up to 30 minutes. Source: our reading of the code.
- **What it does:** `keepStartAnswer` writes the answer of stopped whole, per folder, with file ids, clone id and start mark. `repeatsStoppedAnswer` repeats it only while `stillHolds`: under 30 minutes old, the same folder, top and `.git`, clone id (`cloneIdAt`) and database files (`sameDatabaseFiles`), and a database that reads stopped (`stillStopped`). A starting daemon makes the folder (`makeAnswersFolder`) and removes earlier answers (`removeEarlierAnswers`).
- **Risk:** Medium. A wrong stopped leaves a started checkout unchecked. Each check can only turn a kept stopped into not known, and the hook then waits as on `main`. Nothing is read or written through a link.
- **Check:** Does `stillHolds` miss anything the daemon reads again on every answer (`lookupAt`)? `cloneIdAt` finds the clone id file as Git does, from `.git`, a `gitdir:` line and `commondir`, never through a link or on a server. `smallFileText` opens without waiting, so a named pipe reads as no answer. On Linux an answer is kept only where the daemon holds the database's three files open (`daemonHolds`). Name a state the tests lack and it gets a test.
- **Picture:** Median time of a later hook in a stopped checkout over 30 minutes of ordinary running on Linux x64, at 9c4f9298d: 533.7 ms at main, 163.3 ms with this change. All 60 of this change's later hooks repeated the daemon's answer.
- **Method:** 30 minutes of hooks, checkpoints, `rerail status`, `rerail scan` and readers beside the build's own daemon on Linux x64 at 9c4f9298d, with a later hook in a stopped checkout every 30 s: 60 for each build, medians. The start mark changed in 0 of 60 windows. A second run beside other runs: 60 of 60, 203.0 against 692.3 ms.

**C. The start mark**, medium risk. A database copy written in place after a start: no event sent without the mark, every event here.

- **Problem:** The daemon answers from what its connection read. An older copy of the database written over the live file, after a start the daemon read, leaves the file saying stopped while the daemon answers started. Nothing in the file tells.
- **Who meets it:** Anyone who restores or copies `rerail.sqlite` in place while the daemon runs, after a start of Rerail. How often that happens is not known. Without the mark, hooks there sent no event for up to 30 minutes.
- **What it does:** `renewStartMark` renames a new file over `stopped-answers/start-mark` in one try without waiting, only where a daemon made that folder. Refused, it writes `start-mark-stuck` and tries again for 2 s. Eight writers call it after their commit. `startMarkFound` gives its device, file id and times. `startMarkHolds` compares them and refuses beside a stuck mark or a folder that takes no file.
- **Risk:** Medium. Every start, stop and move of a checkout now calls it, the daemon's among them. A failure is never the writer's: its change has committed. A mark that cannot be renewed makes hooks wait, as on `main`.
- **Check:** Does every writer that can turn an answer from stopped renew after its commit, never inside the transaction? A test of six writers checks it through a proxy of the connection. A renewal that returns with no mark and no stuck mark leaves kept answers holding: `renewStartMark` returns so at a link in place of the folder, and where the folder refuses both files. On Windows `access` reads no folder permissions.
- **Picture:** With another program holding the start mark open on Windows 11, at 6f0152ac8. A start or stop through the API: 3.4 to 5.6 ms at main, 4.4 to 7.6 ms with this change. A hook asking the daemon meanwhile: 244.2 to 261.4 ms at main, 246.4 to 252.5 ms with this change.
- **Method:** the build's own daemon, CLI and Claude Code hooks on Windows 11, while another program holds `start-mark` open. 6 rounds a row, `main` at 71e0822ce and 6f0152ac8 in turn, two runs of each; lowest and highest medians. At 9c4f9298d, which retried the rename for 2 s, the same took 2,024.7 and 2,137.3 ms.

**D. The check's open of the database**, medium risk. Windows 11, a byte-range lock on the database: 1,527 to 1,579 ms at 2a88553c5, within 16 ms of `main` here.

- **Problem:** The hook's check reads the database beside a running daemon. An ordinary open would run schema steps, wait up to 5 s on a lock, wait on a named pipe, or follow a link to another file.
- **Who meets it:** Every repeat opens the database once. A wait there would hold a hook that `main` answers after the daemon's delay alone. Source: our runs with a held lock, a pipe and a link at each name.
- **What it does:** `openRerailDatabaseAsIs` takes `besideDaemon`. With it, `wholeNow` refuses a missing `-wal` or `-shm` and any of the three files that is not a plain file, and on Windows reads the first page and the WAL's header first (`readsNowOnWindows`). `openReadOnlyAtOnce` then connects with a busy timeout of 0 and runs `openChecked`'s read-only steps. `connectSqlite` gains that timeout as a parameter.
- **Risk:** Medium. `src/engine/storage/` is a high-risk path. No schema, migration or stored data changes, and the defaults leave every other caller as on `main`. A refusal only sends the hook back to the daemon.
- **Check:** Does `openChecked` stay untouched, and does any caller but the hook's check pass `besideDaemon`? `wholeNow` closes no descriptor of a database file on POSIX, where that would drop SQLite's locks in the process; it opens the files itself only on Windows. A lock past the first page still meets SQLite's own retry, about 1.4 s.
- **Picture:** The one lock that still costs more. A hook in a stopped checkout on Windows 11 beside a late daemon, while another program locks pages of the database past the first, at 9c4f9298d: 648.6 ms at main, 1,767.5 ms with this change. Both record the same events.
- **Method:** 6 pairs a row on Windows 11 through the build's own daemon, at 9c4f9298d: another program locks a byte range of `rerail.sqlite` while the daemon answers late from what it read. With the whole file, the header, the first page or the WAL locked: -15.4 to +11.0 ms, paired.

**E. Marks of given-up start checks**, medium risk. After a killed hook, a started checkout's hook kept 0 of 4 events at 4662bd526, all here (Linux x64).

- **Problem:** A hook that repeats stopped ends its request, which stays in a busy daemon's queue and is served though nobody waits. On Linux the daemon admits one connection per turn, so a burst queues ahead of every other checkout's hooks.
- **Who meets it:** Sessions in started checkouts beside a busy daemon, once stopped-checkout hooks gave up. At 667ab68a3, before the marks, 8 such hooks made a started checkout's next hook give up at 12 s and lose its 4 events (Linux x64).
- **What it does:** `HookAsk.beforeAsking` makes the hook's pending mark, `<name>.<hook>.asking`, or answers that a check of the folder waits (`.waiting`). `abandoned` renames it to `.waiting` once `sent` said the request reached the OS on a new connection; a sent check with no mark is never ended (`mayEnd`). `ServedAsks.served` removes the folder's marks as the daemon serves a start check there.
- **Risk:** Medium. A mark that outlives its request lets the hooks of its folder skip asking. They then repeat the kept answer only while it holds, so a mark alone never reads a started checkout as stopped.
- **Check:** Can `.waiting` exist with no request of the folder queued at the daemon? `abandoned` renames only after `sent`, which Node's `finish` gives for a new connection; `served` removes pending marks before `.waiting`, so a rename after it finds nothing. An ask its hook ended (`abandoned`, `done`) marks and sends nothing. Is `servingStartCheck` right to count only a `GET /api/presentation` with `repository` and no `sessionId`? This change's lines in `daemon.ts` are 39 of 50, and in `session-presentation.ts` 40 of 73.
- **Picture:** A started checkout's hook after stopped-checkout hooks gave up, Linux x64, five rounds. After a killed hook and 8 more: 32.7 to 32.8 s at main, 32.5 to 32.7 s here, 4 of 4 events both. After 6 groups of 4 at once: main gave up at 12.1 s with no event; here 36.2 to 36.3 s with all 4.
- **Method:** The build's own daemon held busy in 900 ms turns and real hook processes on one Linux x64 machine, five rounds a side: 948a7e06f, then `main` at e8a6b0b5a. A started checkout's hook sends 4 events. At 4662bd526 the killed-hook rows gave 12,081 to 12,091 ms and 0 of 4. Lowest and highest of the rounds.

**F. One kept connection per process**, medium risk. A read that gives up leaves the daemon two connections at `main`, one here (a test on both systems).

- **Problem:** Node's bundled `fetch` opens a second connection when a request gives up, and sends nothing on it. A busy daemon admits each empty connection in a turn of its loop, ahead of later hooks' requests.
- **Who meets it:** Every hook and command that gives up waiting on a busy `http:` daemon. After a burst of 24, the first hook at 4662bd526 took 2,068 to 2,144 ms, past the 2 s read limit (Linux x64).
- **What it does:** `daemonFetch` sends requests to an `http:` daemon through `agentFetch` over Node's http client and one keep-alive agent per process (`KEPT_CONNECTIONS`), and the pinned https client through the same function. `sendOnce` tells `sent` at Node's `finish`, only on a new connection. A GET or HEAD whose kept connection closed before any answer is sent once more; a POST never.
- **Risk:** Medium. Every client of an `http:` daemon sends through it. A request the daemon takes twice would be the harm: only a GET or HEAD is sent again, and only where its kept connection closed before any answer.
- **Check:** Does anything that reads `fetch`'s behaviour differ: bodies (a string or bytes only), headers that repeat, statuses with no body, a signal's own reason on abort? Is 4 s (`KEPT_CONNECTION_MS`) under the daemon's keep-alive of 5 s? Node unrefs a kept connection, so it holds no process open. This commit, a88b29ed6, is also a change of its own, not open yet; this branch adds `sent` to it. Of `daemon-control.ts`'s 86 lines, 61 are this change's and 25 the stop signal's.
- **Picture:** Connections a daemon admits for one read that gives up, by a test's count on Linux x64 and Windows 11: two at main, where Node's fetch opens a second and sends nothing on it; one with this change.
- **Method:** `transport.test.ts`: a test daemon counts the connections it admits while a read to it gives up, at 2b2192f9f on Linux x64 and Windows 11. Through a real busy daemon on Linux x64, the daemon's connection log shows no empty connection after a burst of 24 at 948a7e06f; at 4662bd526 each hook past the read limit left one.

**G. The hook ends its request**, low risk. A suspended daemon: 2,134 ms per hook at `main`, 199 ms here (Linux x64).

- **Problem:** Node keeps a process alive while a socket or a pipe is open. A hook that answered first would still exit only when the daemon answered or the read's 2 s limit ended its request.
- **Who meets it:** Every hook that repeats stopped under change A. Without this change it would have its answer early and its process would end no sooner. Source: our reading of Node, and the mutation that leaves the request open.
- **What it does:** `readStartAnswer` is new: the daemon's answer with the folder, the checkout and the daemon it rests on. It takes an optional stop signal and passes it to `readableDaemon` and `get`, where `endedBy` joins it to the 2 s limit. `recordedDaemonStatus` passes it to Windows' question to the engine, and `engineCapabilityProof` hands it to the request, which then ends as `refused`.
- **Risk:** Low. Each new parameter is optional and only the hook's start check passes one. The status line, the dashboard and every other reader take `main`'s path.
- **Check:** On Windows a stopped proof answers `refused`, which `recordedDaemonStatus` reads as gone. Only `readableDaemon` passes a stop, and it then returns false, so the read returns null and no state file is touched. In `get`, a stop is not the time limit, so it is not reported as `NotAnswering`. Only `engine-proof.ts` is counted here: this change's 33 lines in `session-presentation.ts` are under E and its 25 in `daemon-control.ts` under F.
- **Picture:** Time of one Claude Code hook in a stopped checkout on Linux x64 whose daemon is suspended and never answers, at 9c4f9298d: 2,134 ms at main, which leaves at the read's time limit, and 199 ms with this change, which ends its request.
- **Method:** a Claude Code hook in a stopped checkout whose daemon is suspended once it has answered stopped there, through the build's own daemon on Linux x64: `main` at 71e0822ce against 9c4f9298d. `main`'s hook leaves when the read's limit ends its request.

</details>

<details><summary><b>6. The decisions</b></summary>

**Ask the daemon first and repeat after a grace, or read first?** (ask 1)

**We chose** ask first: beside a prompt daemon a hook does what `main`'s does. **Would change our mind:** an answer of no on #670: the fix then moves into the daemon.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| Ask the daemon first; past the grace, repeat its last answer of stopped **(chosen)** | Beside a prompt daemon, `main`'s behaviour. Beside a busy one, later hooks in a stopped folder take 86 to 196 ms on Linux x64 (`main`: 1,548 to 1,806) and 353 to 777 ms on Windows 11 (`main`: 2,470 to 3,470). | The grace before the kept answer's check. Looks at file metadata before each ask. Per folder a file of 358 to 458 bytes; one start mark; an empty mark file per ask in a folder with a kept answer. | The states and costs under What is not covered. The reference's sentence changes: a stopped checkout's hook still asks a running daemon. |
| Read the database first; ask the daemon only where the read cannot tell | No request in a stopped checkout: 82 ms beside a busy daemon on Linux x64, at 60bbe3e51. The reference's words stay true. | The read in every checkout. A started checkout's start check took 20 against 11 ms on Linux x64 and 25 against 15 ms on Windows 11, idle daemon. | The most common case, a started checkout, gets slower than `main`. |
| The daemon answers start checks ahead of its queue | Shorter waits in every checkout, and no kept answer to go stale. | A second listener and database connection in the daemon, or memory shared between processes. Not built. | The hook still waits on the daemon. |
| Do nothing | Nothing new. | 1.5 to 1.8 s per stopped-checkout hook beside a busy daemon on Linux x64 and 2.5 to 3.5 s on Windows 11 (`main` at e8a6b0b5a). | The reference stays untrue. |

Before: At main the hook asks the daemon, waits up to 2 s and takes the daemon's answer. After: The hook looks whether a check of its folder waits at the daemon: if so, and every check agrees, it repeats stopped unasked. If not it makes a pending mark and asks. Past the grace, where every check agrees, it repeats stopped and marks its request; else it waits.

**Evidence.** With a daemon that answers at once, the asks-first tests expect no database opened and no kept answer checked, and fail with the check put before the ask. Paired, real daemon, idle, this change less `main`, at 6f0152ac8, before a hook made a pending mark: +3.1 ms in a started checkout and +0.7 ms in a stopped one on Linux x64, +4.6 and +3.0 ms on Windows 11.

**How long may the daemon take before the hook repeats?** (ask 3)

**We chose** 75 ms: on Windows 11, 25 of 2,100 idle asks miss it, against 145 at 50 ms. **Would change our mind:** Windows machines whose idle daemon often answers later.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| 75 ms **(chosen)** | Beside a busy daemon the hook repeats right after the grace. | 25 of 2,100 idle asks on a Windows 11 virtual machine miss it and pay the check of the kept answer, and 0 of 2,100 on Linux x64. | Nothing measured. |
| 50 ms | A repeat 25 ms sooner beside a busy daemon. | 145 of 2,100 idle asks on Windows 11 miss it, 6.9%. | Idle hooks on Windows 11 pay the check six times as often. |
| 100 ms | 8 of 2,100 idle asks on Windows 11 miss it. | 25 ms more for every repeat beside a busy daemon. | Nothing measured. |
| 150 ms | 0 of 2,100 idle asks on Windows 11 miss it. | 75 ms more for every repeat beside a busy daemon. | Nothing measured. |

**Evidence.** A real daemon (`rerail daemon start`), each ask in a process of its own, 2,100 idle asks per system, at 60bbe3e51. Linux x64, 16 cores: median 16.5 ms, 99th percentile 20.0 ms, longest 25.9 ms. A Windows 11 virtual machine with no other run: median 23.2 ms, 90th percentile 46.1 ms, 99th 76.5 ms, longest 139.1 ms. At 40 ms, 364 of 2,100 miss it there.

**May the hook decide stopped by itself, or only repeat the daemon?** (ask 4)

**We chose** repeat only: every version that decided from the disk differed from the daemon somewhere. **Would change our mind:** a state where the daemon reads started while every check agrees, beyond those named.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| Repeat the daemon's last answer, checked against clone id, database files and start mark **(chosen)** | Every recorded checkout and worktree keeps the gain in ordinary running: over 30 minutes, 60 of 60 later hooks repeated (Linux x64, 9c4f9298d). | 577 lines for the kept answer and 164 for the start mark, at 5d7404689. A session's first hook in a folder still waits, and so does the first after any start or stop of Rerail. | A worktree with no record of its own, a new clone and a folder outside Git wait as on `main`. A start that renews no mark goes unseen (What is not covered). |
| The folders on disk and the database, no Git | An answer in a session's first hook too. This branch at 6993ee854. | 211 lines that follow Git's search. | It differed from the daemon in some state at every rebuild: a nested clone's `.git` turning into a `gitdir:` file, a WAL deleted under the daemon, links to a server that does not answer. |
| The daemon says what its answer rests on | Exact in every state but a copy of the database written in place, which the daemon itself does not see. | A field in `/api/presentation` and an edit of a handler at complexity 17 on `main`. Not built. | Nothing known. |
| Drop a kept answer when the database file's size or time changes | It would see a copy written in place. | Not built. | The gain: the file changed in 60 of 60 thirty-second windows from the daemon's own checkpoints (Linux x64, 9c4f9298d). |

**Evidence.** A test asks two hooks and the daemon about each of 40 folders, with and without a ceiling right above the folder in the daemon's environment: 0 where the hook repeats stopped and the daemon reads otherwise, on Linux x64 and Windows 11. Through a real daemon an older copy of the database written in place sent 0 of 3 events before the start mark and 3 of 3 with it, as `main`.

</details>

<details><summary><b>7. How it fits</b></summary>

It stands on `main` alone, merged in at e8a6b0b5a: it needs no open pull request, and none needs it. #677 and #676 have merge conflicts with it; so has a change of ours not open yet, whose commit this branch holds.

Pull request #674 is based on main at e8a6b0b5a, which is merged in. Its merge waits for a maintainer's answer on #670. No pull request needs it. In trial merges it has merge conflicts with #677 in one file and #676 in four, beyond their own with main, and none with #665 beyond its own. A change of ours not open yet, which sends requests over one kept connection, has merge conflicts in three files; this branch holds its commit.

**Shared files and trial merges.** `git merge-tree` of each head with 5d7404689; a merge conflict counts only beyond that pull request's own with `main`. #677 at 62205c1c3: `engine-proof.ts`, where this adds a stop signal. #676 at 67f59eed4: `repository-registry.ts`, `database.ts`, `hook-host.ts` and `session-presentation.ts`. #665 at 5ba35449e: none beyond its own four. The change that sends requests over one kept connection, not open yet: `daemon-control.ts`, `transport.ts` and `transport.test.ts`, where this branch holds its commit plus `sent`; the second to land keeps this branch's side. #548 reports the busy daemon itself, which this does not fix.

</details>

<details><summary><b>8. What is not covered</b>: 6 not verified, 6 smaller gaps, 8 limits that stay</summary>

1. **A start that renews no mark:** an older release's, another program's, one cut off before its renewal, or one behind a linked or refusing answers folder. A database copy written in place then goes unseen.
2. **A database swapped before the answer was kept** goes unseen on macOS, and on Linux where the daemon also opened the new file. Read, not run.
3. **A Git layout changed after the daemon placed the folder** is read by the daemon through the placement it kept. Read, not run.
4. **Hooks that start together** each leave their check queued: after 6 groups of 4, a started checkout's hook took 36.2 to 36.3 s, where `main`'s gave up at 12.1 s (Linux x64).
5. **On Windows 11,** 1 of 35 started-checkout hooks at 948a7e06f counted 2 of its 4 events, beside other test runs. A second run of 10 counted every event. Its cause was not traced.
6. **Not run:** macOS, a real case-insensitive Linux folder, the event stream beside a late daemon, a real session of an agent, and the whole suite at this head.

**Smaller gaps**

- **After a start or stop, hooks wait for one daemon answer.** On Linux x64 beside a daemon that stays busy, 8 of 8 later hooks took `main`'s time, 2,120 to 2,130 ms. On Windows 11 the first hook took 2,517 ms and the next repeated again (6f0152ac8).
- **After a burst, a first hook can wait one more daemon turn.** For the one check left queued: up to 0.15 s more in a stopped checkout, which its 2 s read limit caps, and 0.67 s in a started one (Linux x64), with the same answer and events.
- **A Windows lock past the database's first page.** A hook then takes about 1.1 s more than `main`'s (1,767.5 against 648.6 ms, 9c4f9298d) and records the same events.
- **A mark the daemon cannot remove.** A `.waiting` whose removal the system refuses stays while refused: its folder's hooks repeat the kept answer without asking, only while that answer holds. A killed hook's pending mark claims nothing; the daemon removes it at the folder's next start check.
- **The first 46 commits hold earlier designs.** They read the database and the folders on disk in the hook. c9f94cf1b and ae2d8584d replaced that code. Read the diff against `main`, not commit by commit.
- **A process of the same user can rewrite a kept answer.** One that makes it name the mark now there, and writes an older database copy in place, makes hooks skip events. On `main` such a process gets the same by keeping the daemon from answering for 2 s.

**Limits that stay**

- The first hook in a folder, a hook where the daemon last answered started, a started checkout beside a busy daemon, and a stale `daemon.json` or none: all as on `main`.
- A repository never recorded, a new clone, a worktree with no record of its own, a folder outside Git, a Windows network share and a Git folder behind a link wait as on `main`: only the daemon's lookup places them.
- A daemon serving https gets no gain: a hook waits for the TLS handshake the busy daemon delays, as on `main`, and makes no mark.
- On Windows 11 no mark forms beside a busy daemon: the hooks end during the engine's proof, before they send. The gain there comes from the 75 ms grace, and Windows' daemon admits several connections per turn of its loop.
- A refused renewal leaves `start-mark-stuck`, and every stopped folder's hooks wait as on `main` until a renewal succeeds. The daemon tries again for 2 s; a one-shot command leaves it to the next renewal.
- Beside an idle daemon a hook cost +0.7 to +4.6 ms more than `main`'s, paired medians at 6f0152ac8. With the pending mark, at 948a7e06f, a later stopped-checkout hook took a median 87.5 ms against `main`'s 90.0 (Linux x64, 52 each).
- An answer is repeated for at most 30 minutes, and only a daemon answer in time renews it. A start or stop of Rerail anywhere ends the gain until the daemon answers one hook in time.
- A fault in the check of the kept answer that is not SQLite's fails the hook: it answers `{}` with one line on stderr, in a started checkout too, where `main` waits for the daemon.

</details>

## 9. What we ask of you

Asks 1 and 2 rule on answers 1 and 2.

1. **On [#670](https://github.com/neralabs/rerail/issues/670): may a hook repeat the daemon's last answer of stopped?** We recommend yes: only what the daemon said.
2. **May the reference drop "never contact the daemon"?** We recommend yes: `main` already asks.
3. **Is the grace right for the Windows machines you see?** We recommend keep it until one shows otherwise.
4. **Are the starts that renew no mark acceptable?** We recommend yes: the reference names them.

<details><summary><b>10. Evidence</b></summary>

The description is the record: "How you tested it" holds the timings and the runs, and "What is not covered" what remains.

| Where | What ran | Result |
|---|---|---|
| CI on GitHub | No run at 5d7404689 yet. The last run, 43622328319, was at 6993ee854, an earlier design | 28 checks passed, none failed and the others were skipped, on the run's second attempt (read on 14 November 2026). |
| Linux x64; Windows 11 | The 30 test files the description names as touched by the change or importing a changed module, at 5d7404689 (the head's source on Windows 11) | Linux x64: 705 passed, 27 skipped, 0 failed. Windows 11: 696 passed, 37 skipped, 5 failed; those 5 fail at `main` e8a6b0b5a the same way, in 2 of 2 runs. At 948a7e06f a sixth, a rename Windows refused in the case of a sync tool's copy, failed in 1 of 10 runs there and at 4662bd526 alike. |
| Linux x64; Windows 11 | The 13 cases added or changed for the marks and `sent`, 20 runs in a row, at 2b2192f9f (c93ca0fcd on Windows 11); and each of the 16 lines 948a7e06f, 6a31a35b0 and 2b2192f9f add or change, taken out alone | 0 of 20 runs failed on each system. Each line taken out fails a case on both. At 4662bd526 on Linux x64, 7 new or changed cases fail. |
| Linux x64 | CI's release-note check, build and integration job, at the head's source; the automated reviewer's rules over the diff against `main` | Each exits 0; integration: 22 files, 90 passed, 4 skipped. The rules: no new finding; 161 functions added or edited, the highest at 6. |
| Linux x64; Windows 11 | Hook time beside a busy daemon: Claude Code, Codex and Cursor, through a real daemon, 948a7e06f against `main` at e8a6b0b5a | Linux x64: 86 to 196 ms against `main`'s 1,548 to 1,806. Windows 11, beside other test runs: 353 to 777 ms against 2,470 to 3,470. |
| Linux x64 | Bursts beside a busy daemon, five rounds a side, through a real daemon, at 948a7e06f: a started checkout's hook, which sends 4 events, after hooks in a stopped checkout | After 8 hooks: 32,561 to 32,645 ms, 4 of 4 (`main`: 32,731 to 32,785, 4 of 4). After a killed hook and 8: 32,503 to 32,660 ms, 4 of 4 (4662bd526: 12,081 to 12,091, 0 of 4). After 6 groups of 4 at once: 36,216 to 36,313 ms, 4 of 4 (`main`: 12,075 to 12,138, 0 of 4). |
| Windows 11 | The same bursts on one desktop machine beside other test runs, at 948a7e06f | After 8 hooks: 36,682 to 37,820 ms, 4 of 4 in four rounds and 2 of 4 counted in one; a second run that counts again 10 s later: 36,689 to 37,695 ms, 4 of 4 in each of 5 (`main`: 37,243 to 37,395, 4 of 4). After 6 groups of 4 at once: 37,232 to 37,434 ms, 4 of 4, as at `main`. No mark forms there. |
| Linux x64; Windows 11 | Earlier runs, each at its commit: the states through a real daemon (7680b1f4d's source and 6f0152ac8), a held start mark (6f0152ac8), 30 minutes of ordinary running (9c4f9298d) | 57 states on each system record what `main` records; two more on Linux x64 and one on Windows 11 record less: the first line of What is not covered. Held mark, Windows 11: a start or stop through the API 4.4 to 7.6 ms (`main`: 3.4 to 5.6). 60 of 60 later hooks repeated, 163.3 ms against `main`'s 533.7 (Linux x64). |

**What this report adds to the description**

- **The two intents at `main`, quoted.** The description quotes neither. The reference at `main` says a stopped checkout's hooks "never autostart or contact the daemon"; a code comment says "A running daemon answers first". This pull request keeps the comment's order and rewrites the reference. [Intent, and the gap](#2-intent-and-the-gap) gives both quotes, and its folds their lines.
- **A second answer changed on purpose: the reference's sentence.** The description lists the reference under docs, at low risk. After this change a hook in a stopped checkout still asks a running daemon first. Ask 2 puts it to a maintainer.
- **The event stream makes the same start check.** The description names Claude Code, Codex and Cursor hooks. `src/harness/event-stream.ts` line 133 calls `hookRerailStarted` for each event too, so a stream in a stopped checkout beside a late daemon now repeats the kept answer. Read, not run.
- **A pending mark is made per ask, not per given-up check.** The description counts an empty mark file per start check given up. `HookAsk.beforeAsking` makes the hook's pending mark before each ask in a folder with a kept answer, and `done` removes it once the daemon answers. The idle timings, at 6f0152ac8, are from before the marks. Read in the diff, not timed.
- **Our answers to the three read-first questions.** Read in the diff. `daemonAnswer` returns the daemon's answer or the literal `stopped`, and only where `knownStopped` held. Each check of `stillHolds` can only turn a kept stopped into not known. A `.waiting` is made only by a rename after `sent`, and `served` removes it; one whose removal the system refuses outlives its request, as What is not covered says.

**Sources.** The description of pull request #674 on 18 November 2026 for the runs, timings, trial merges, CI and reviews, each at the commit it names. The branch at 5d7404689 and `main` at e8a6b0b5a for the diff, counts and quoted intents. A file two changes edit is counted whole under the one with most of its lines. The test lines' split among the changes is our count by test.

</details>
