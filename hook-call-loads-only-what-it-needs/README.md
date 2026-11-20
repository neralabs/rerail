# A Windows Claude Code hook call reuses the code Node compiled

For [pull request #959](https://github.com/neralabs/rerail/pull/959), head 26d6f38b3. [report.html](report.html) is the same report as one page: download it and open it.

## 1. The decision

**May Rerail's Claude Code hook launcher turn on Node's compile cache, in Node's default folder, so each Windows hook call reuses earlier compiled code?**

**We recommend:** approve it: every hook does what it did, and each Windows hook call starts about 26 ms sooner, 241 to 215 ms at the median on Windows 11.

| Size | Risk and review | Waits on | Not verified |
|---|---|---|---|
| 86 lines in 4 files: 28 ship, 58 are tests and text | Fast. Low risk: one module; every session's hooks answer as before. | Nothing. CI at the first merge: no failure `main` lacks. | 4 |

## 2. Intent, and the gap

| Change | Intended | Found | Now |
|---|---|---|---|
| A compile each call | No script compiled twice _(our reading)_ | Every call compiles it again _(marks, 140 calls)_ | Closes the gap. |

Found in November 2026 on Windows 11 virtual machines, timing each part of a Claude Code hook call.

<details><summary>Sources at main, and causes</summary>

| Change | Where main states the intent | The cause, and what was measured |
|---|---|---|
| A compile each call | `src/harness/hook-launcher.ts` at d54b586a8, lines 1 to 13: the launcher "runs the hook under the node recorded at registration, in this process when that node started it". Nothing in it turns on Node's compile cache. | Each hook event starts a Node, and Node keeps its compile cache off unless `NODE_COMPILE_CACHE` or `module.enableCompileCache` turns it on, so V8 compiles the whole script in every process. A CPU profile of 3 calls on a busy machine put `compileSourceTextModule` at 44.3 ms a call at `main`. With `NODE_COMPILE_CACHE` set by hand, `main`'s calls went from 227 to 201 ms at the median. |

</details>

## 3. Where to look

1. **Read first**, 5 min: `hook-launcher.ts`: `compileCacheEnv`, and the `env` it hands `spawnSync`. Can an old node, a folder Node cannot make, or `NODE_DISABLE_COMPILE_CACHE=1` stop a hook from running?

**Then skim**, 3 min: `hook-launcher.test.ts`: a stand-in hook reports `getCompileCacheDir()`, run through the launcher in its own process and under another node, with the cache on and off.

## 4. Risk against verification

<img src="grid.png" width="544" alt="The changes by risk, high on top, against six checks. Compile cache, low risk: independent review does not hold yet, rules scan: nothing new holds with a limit. CI at this head holds with a limit for every change.">

One change, low risk: one function in the Windows hook launcher; no registered hook command and no hook's code changes.

<details><summary>Each limit, and each check that does not hold yet</summary>

| Change | Check | Its limit, or why it does not hold yet |
|---|---|---|
| Compile cache | independent review | No review by a second reader has run on this change yet. |
| Compile cache | rules scan: nothing new | `npm run check` passed in the commit hook at afc47e2e3. The automated reviewer's rules have not run over the diff. |
| Compile cache | CI at this head | The repository's CI ran at fad788bb3, the first merge of `main`, on GitHub's runners in the fork: no failure `main` lacks there. One Rust job was cancelled; the change reaches no Rust. The second merge changed no file both sides touch. |

</details>

## 5. The changes

**The launcher turns on the cache**, low risk: Windows 11, by the marks: the hook script's compile takes 37.2 ms a call on `main`, 9.2 ms here.

<details><summary><b>6. The decisions</b></summary>

**Remove the compile with Node's cache, or with a smaller hook script?**

**We chose** the compile cache in the launcher: it removes the compile with one call and changes no module the hook loads. **Would change our mind:** A maintainer who prefers the slimmer hook script first, which would also shorten the hook's top-level code.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| Compile cache in the launcher **(chosen)** | The script's compile in a Windows call goes from 37 to 9 ms. | One call in the launcher. | Nothing found. |
| A hook script holding only the hook's own modules | Less to compile and less top-level code to run: that code and the work before the first request take 36.0 ms a call. | Moving the daemon's event client and its runtime out of the hook's imports, in several modules. | Nothing, if built. |
| One request to the daemon per call in place of four | Part of the 30 ms the four requests take beside the daemon's own answers, not measured apart. | A new daemon request. | A daemon of an earlier release would not answer it. |
| The compile cache for Linux and macOS too | The same gain there: on Linux x64 the compile is 32.6 ms of a 167 ms call. | A new registered hook command, rewritten in every person's settings. | Nothing, if built. |

**Evidence.** Marks in a throwaway copy of the installed hook, 140 calls on idle Windows 11, medians: Windows starts the process in 16.9 ms, Node's own start takes 14.1 ms, built-ins and better-sqlite3's JavaScript 22.7 ms, the daemon's answers 30.9 ms.

**Keep the cache in Node's default folder, or under the Rerail home?**

**We chose** Node's own folder, or the one `NODE_COMPILE_CACHE` names: Node's tools already share it, and the launcher works out no path. **Would change our mind:** A maintainer who prefers the cache folder under the Rerail home.

| Option | What a user gets | Cost | What it breaks |
|---|---|---|---|
| Node's default folder, or the one `NODE_COMPILE_CACHE` names **(chosen)** | One cache for every Node program a person runs, where a person who set `NODE_COMPILE_CACHE` expects it. | About 1.2 MB more in the temporary folder. | Nothing found. |
| A folder under the Rerail home | Rerail's files in one place. | A path the launcher works out before it loads the hook. | A person's own `NODE_COMPILE_CACHE` would not be followed. |

**Evidence.** After the Windows 11 run the temporary folder's cache held 596 files, 4,012 KB here against 580 files, 2,856 KB at `main`, where npm's own set-up entries are the rest.

</details>

<details><summary><b>7. How it fits</b></summary>

Nothing has to merge first: the change stands on `main` d54b586a8, and the head holds `main` d93d74b74 through two merges Git made alone. In code it edits `hook-launcher.ts` alone; `main`'s later edit of `docs/windows.md` is in another section of that file.

</details>

<details><summary><b>8. What is not covered</b>: 4 not verified, 3 limits that stay</summary>

1. **Linux and macOS** start the hook from a shell-form entry without the launcher, so they still compile the script at each call.
2. **Codex's and Cursor's Windows entries** start the hook from PowerShell without the launcher, and still compile it at each call.
3. **The slimmer hook script** is the next option, not built here: it would leave less to compile and less top-level code to run.
4. **macOS** never ran.

**Limits that stay**

- A registered command that names the cache would bring the gain to Linux and macOS; it rewrites every person's settings, so it is left for a change of its own.
- The two requests a call makes before its event, a check of the daemon's capability and a read of the repository's state, and the event itself stay as they are.
- Other runs kept the Windows 11 machine busy during the timing, at 100% CPU at its start; `main`, this change and 0.1.5 ran round by round in the same run.

</details>

## 9. What we ask of you

We ask no ruling.

<details><summary><b>10. Evidence</b></summary>

The description on GitHub is the record: "Why it is worth landing" holds the timings, and "How you tested it" the cases and the runs.

| Where | What ran | Result |
|---|---|---|
| Windows 11 and Linux x64 | `test/harness/hook-launcher.test.ts` with the test alone at `main` d54b586a8, then at afc47e2e3 | With the test alone: 2 failed, 13 passed on each system. At afc47e2e3: 15 passed on each. |
| Windows 11, 8 cores | Rerail packed and set up from a local registry, `main` d54b586a8 and this change beside the published 0.1.5; seven hook kinds, 50 calls each a build, round by round | Medians: 241 ms at `main`, 215 ms here, 250 ms for 0.1.5, 50 ms for `node -e 0`; 95th percentiles 394, 376 and 519 ms. Each kind 18 to 31 ms faster here. |
| Linux x64, 16 cores | The same set-up and hook calls, 350 calls a build | Medians: 174 ms at `main`, 176 ms here, 175 ms for 0.1.5, 20 ms for `node -e 0`: unchanged, as expected, since the shell-form entry skips the launcher. |
| Windows 11, 8 cores | The issue's steps, twice: the hook as the launcher leaves it, the hook with the cache off, and `node -e 0` | `main` 254, 259 and 87 ms, then 167, 169 and 46 ms; here 219, 255 and 84 ms, then 127, 149 and 43 ms. Other work kept the machine at 100% CPU in the first run. |
| GitHub's runners, in the fork | The repository's CI at fad788bb3, the first merge of `main` | No failure that `main` does not have there; one Rust job was cancelled, and the change reaches no Rust. |

**Sources.** The branch at 26d6f38b3 and its diff against `main` d93d74b74, read on 20 November 2026; tests at afc47e2e3 and timings on `main` d54b586a8, on Windows 11 virtual machines and Linux x64 machines.

</details>
