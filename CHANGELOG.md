# Changelog

Every notable change to Rerail's core package, `@neralabs/rerail`, latest first. From 0.1.1 the notes follow [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/): each release lists its changes under Added, Changed, Deprecated, Removed, Fixed and Security. There are two differences: a heading reads `## <version> (<date>)` rather than `## [<version>] - <date>`, and changes waiting for the next release are in [.changelog](.changelog/README.md), so `## Unreleased` holds only that release's opening paragraph.

Versions are numbered as in [Semantic Versioning](https://semver.org/spec/v2.0.0.html), which promises nothing about compatibility below 1.0.0. A release is a patch unless it names a later version, and a patch can change or remove what you use, so read the notes before you update.

## Unreleased

0.2.1 brings Rerail's new rule engine and the first of its new rule language. Rule packs can now ship rules written in the new language, `.rerail`, and the new Django pack is the first: sessions run them once an agent's writes settle, and `rerail scan` includes them. `rerail rules test` runs a `.rerail` rule file from any install, not only from a clone of Rerail's source. Code rules written in JSON still run, and the code they check is now read through the new engine's parsers and content store, so they can read C, C++, C#, Go, HTML, CSS, Markdown and JSON. Rules also get a title in your own words, which a new or edited rule now needs. 0.2.1 also keeps sessions fast while Git worktrees are removed, and stops a session's first edit from holding other sessions' hooks while its rules compile. Before you update: Rerail 0.2.1 does not run rule pack versions published before 17 November 2026. `rerail update` names any you have installed and gives the `rerail packs add` command for a newer version that works. The 17 November versions of the JavaScript, TypeScript, Python and PHP packs do run, but skip about 40 rules; their newer versions run them, and `rerail update` lists those the same way. Your own code rules are skipped too if they test a token the new syntax trees leave out, such as `,`, `=>` or an implied keyword. Rerail says which and why, and they run again once you rewrite them. The first run rebuilds the content store, and the package is about three times larger, about 240 MB to download.

## 0.2.0 (2026-11-20)

0.2.0 brings Rerail's new rule engine and the first of its new rule language. Code rules are still JSON, but the code they check is now read through the new engine's parsers and content store, and they can read C, C++, C#, Go, HTML, CSS, Markdown and JSON. Rule files in the new language, `.rerail`, can be tested with `rerail rules test` from a clone of Rerail's source; sessions, scans and rule packs do not run them yet. Rules also get a title in your own words, which a new or edited rule now needs. Before you update: rule pack versions published before 17 November 2026 accept only a Rerail below 0.2.0, so `rerail update` names the installed packs it leaves out, then lists the newer versions that fit with the `rerail packs add` command that installs them. The 17 November versions of the JavaScript, TypeScript, Python and PHP packs skip about 40 rules until their next version, which `rerail update` lists the same way. Your own code rules that test punctuation or a keyword the grammar implies are skipped too, until you rewrite them. The first run rebuilds the content store, and the package is about three times larger, about 240 MB to download.

### Added

- Code rules can read C, C++, C#, Go, HTML, CSS, Markdown and JSON by syntax: `language: "c"`, `"cpp"`, `"csharp"`, `"go"`, `"html"`, `"css"`, `"markdown"` or `"json"`. `.h` files are C++. As with Java, names, values, arguments, calls and format arguments are not read for them yet; for JSON a rule that uses one is refused.
- History by session marks a steer the coding agent declined with `rerail feedback`: its card reads Declined beside Steered and shows the agent's reason, the session's row and header count declined steers, and a Declined steers chip lists only those sessions. A steer with no reason recorded is not marked either way.
- In internal testing, `rerail engine dump`, `rerail engine asm` and `rerail engine sql` are the rule engine's text tools (`RERAIL_EXPERIMENTAL_ENGINE_TOOLS`), and a code rule can carry a `relational` clause, SQL over the engine's relations (`RERAIL_EXPERIMENTAL_RELATIONAL_RULES`). Without its variable, `rerail engine` refuses, and such a rule never runs and `rerail rules add` refuses it; `rerail rules test` still tests it.
- From a clone of Rerail's source, `rerail rules test` runs a `.rerail` rule over tool calls: a policy with `.scope tool_calls` reads each call's tool and string arguments and reports its finding at the call, and a test gives it calls with ``.tool_call "Bash" `{"command": "…"}` ``. Sessions still check tool calls with JSON tool-call rules, in internal testing.
- A rule now has a `title`, the rule in your own words, which the dashboard lists it by instead of the message the coding agent reads. `rerail rules add` and the library API refuse a new or edited rule without one, and `/rerail-create` fills it from your wording. Rules added before keep showing their first message until edited.
- `rerail rules test` takes a `.rerail` rule file and runs the tests it carries (`.test ... .end` blocks), reporting each as passed or failed; `--json` lists every test with its failures and findings. A file that is missing, does not parse or holds no tests is an error. It runs from a Rerail source clone, or with `RERAIL_SOURCE_ROOT` naming one.
- In internal testing, `rerail scan` runs the rule packs a repository lists in `.rerail/packs.json`, by exact version or by a folder in the repository, optionally only in the `folders` each names and with the rule options its `config` sets, read from the commit its rules come from. `--pack` replaces the listed packs and their options, to try others.

### Changed

- In a coding agent's session, `rerail rules approve <id>` and `rerail rules import` are refused, and `rerail rules add` saves a change to one of your approved rules as a copy that runs in Log, so your approved rule keeps running as it was until you approve the copy on its page in the dashboard.
- The content store's file is now `content-v5.sqlite` in `~/.cache/rerail/` (Windows: `%USERPROFILE%\AppData\Local\rerail`), with the facts it keeps in files in the folders beside it (`.frames`, `.memo`, `.objects`). The old `content-v1.sqlite` (with its `-wal` and `-shm`) is no longer read and stays: delete it yourself, or `rerail install --remove` deletes it with the rest of the content cache.
- The dashboard's banners read in plain text colours, and those about approval show a dashed check; day one's banner names your rule packs and how many rules steer; pack cards are all one size; and the top bar and Settings show the version of Rerail on this computer.
- In History by session, each tool call names its tool beside its badge (Bash beside Run, an MCP call's server and tool beside MCP); a file change still names its file. Rows drop the green ok and the sequence numbers, which move to the time's tooltip, so more of each description shows.
- History by rule counts each violation once, as Log only, Steered or Declined, with a chip for each (Logged is now Log only). Rules show their declined count in red, and a declined violation the coding agent's reason. The rule sheet shows declined in place of last logged, and Overview's Violations steered leaves declined out.
- The dashboard describes rule packs the way set-up does: ready-made rules, mostly the same ones linters use, suggested for what a repo is built with. No page, and no set-up line, says a pack "fits" or is "recommended" any more, and Settings says switching a repo off stops Rerail there.
- History by session's list filters are now All, With violations and Declined steers, on one line; In progress is gone. A filter opens every day of the list, so its matches show. Each row shows its steered and declined counts beside its state, without the event count or session id.
- History by session's header is three quiet lines: the session's title with its Resume or Find button; when it started and ended, or that it is still open; and its counts of events, violations, steered and declined. The session id moves to the resume box: in its command, or its Copy id button.
- The dashboard shows each rule's enforcement, Steer or Log only, beside its switch and on its sheet; your coding agent still changes it. The counts line says how many rules steer, log only and are off, Filter offers Steer and Log only, and the whole-pack switch sits halfway while some rules are on.
- On History by session's violation cards, the rule's title opens the rule, underlined on hover, in place of the Open rule button, with the rule id and the badges on lines of their own under it. Tool calls are labelled Agent and violations Rerail, as your prompts are labelled You.
- Your own code rule that tests a token the syntax trees may leave out, such as `,`, `=>` or an implied keyword, stops running: Rerail skips it and says why, and `rerail rules add` and `rules test` refuse it. Test the named structure or the token as a neighbour (`has`, `follows`, `precedes`), or say why it is stored (`stored`).
- A session quiet for 30 minutes, or 24 hours while it is still owed a check, steer or finding, now stays open while its daemon runs: the dashboard shows it live, `rerail settings` no longer calls it inactive, and `rerail reset` no longer removes it as ended. Its next tool call still waits up to 20 seconds for its rules.
- `/rerail-create` now adds the rules it suggests once they pass their tests, in Log. The dashboard's Overview says how many your coding agent added and what they caught; each is approved on its page, where an edited copy says approving it turns the original off.
- A rule pack with rules that test punctuation or keywords the grammar implies, which this Rerail's syntax trees no longer keep, still loads: each such rule is skipped and reported with the reason. The 17 November 2026 versions of the JavaScript, TypeScript, Python and PHP packs have such rules; their next versions run them again.
- Set-up now says, before it connects your coding agents, that the `/rerail` commands load Rerail's skills (instructions your agent follows to write rules and to work out what went wrong), and the installation guide describes the same.
- The line you see when Rerail steers your coding agent (`◆ Rerail · <rule>  steered  ·  why?`) names the rule by its title, the one you gave a rule of yours or its rule pack lists, rather than by its id. The coding agent's own message still names the id.

### Fixed

- A coding agent reading findings through Rerail's MCP tools (`violations_list`, `audits_list`, `repository_audit_entries_list`) now gets a finding in a file reached through a folder link into a folder holding secrets (`conf` to `.aws`), or by a Windows short name, by rule and place only, as for a file under `.aws/` itself.
- Rerail now checks a Claude Code edit whose call a busy Rerail took up only after the tool wrote the file, once the session's code rules are open, wherever Claude Code's result tells what the file held before; it used to check nothing. A new file a formatter changed before Rerail took up its call is still not checked.
- Claude Code's status line no longer runs every five seconds in every open session, idle ones included: Rerail sets no refresh interval, and removes the 5 it set. On macOS and Linux, without a status line of your own, a refresh starts no Node: the shell prints a file the daemon keeps for the session.
- When Rerail cannot read a file at a check, as when another program holds it open, the next check that can read it now takes what changed: a shell command's write steers the agent, and another program's write to a file the agent edited is recorded. Before, Rerail missed both.
- Files an agent's command wrote and committed while Rerail was asking Git about them, which can happen on a busy machine, could be taken for files Git wrote, and were then not checked. They are now checked as the agent's edits, unless the command commits again each time Rerail asks.
- The daemon's first session in a repository with many worktrees takes about half the memory it took: its walk of the other worktrees no longer opens rule checking in each of them, and sessions share each checkout's Git ignore rules while those stay unchanged. Each later session still adds about as much memory as before.
- `rerail daemon start` no longer says the daemon did not become ready while it goes on to serve. A daemon that has not answered within 15 seconds is named by its pid as one that may still be starting, and left running; `rerail status` shows it running once it answers.
- `rerail daemon start` with an empty `--tls-key`, or one that is not a PEM private key, serves this computer alone whoever can read it, naming the file. Such a key others could read stopped the start; an empty one only its owner read started a daemon that refused every https connection. An empty `--tls-cert` stops the start, naming the file.
- The daemon no longer starts Git on every hook event of a session that works in a folder outside any Git checkout, such as your home folder. Where starting a process is slow, as on a busy Windows machine, each of those starts kept the daemon from answering every session's hooks for up to seconds at a time.
- `rerail daemon stop` no longer fails with `kill ESRCH` when the daemon finishes shutting down just as the stop signals it. That can happen when the daemon spends about as long as the stop waits finishing a busy session's requests. The stop now counts that daemon as stopped and removes its records.
- `rerail daemon stop` ends a daemon that does not answer only when it holds this home's lock as its recorded holder or started no later than the time `daemon.json` records. A killed daemon's record whose process id a later program took is removed, that program left running. Otherwise the stop ends nothing, keeps the record and says how to check.
- On Linux, `rerail daemon stop` says the daemon stopped only once every thread of it has exited, so the daemon no longer holds its files, sockets and locks after the stop answers.
- The daemon no longer exits when another program removes a running session's record from `rerail.sqlite`, as restoring a copy taken before that session began through SQLite's backup API does. It exited within a second, and edits in every open session went unchecked until a daemon started again. That session now keeps recording and writes its record again.
- A dashboard link opened in a browser with no access yet, such as one copied from a coding agent's session, now shows one page asking to connect the browser with `rerail manage`, then continues where the link pointed. It used to look like a repository with no rules or sessions. `/rerail-manage` no longer hands on that link.
- In History by session, a tool call's time and a violation's file path no longer get cut off or run past the card's edge in a narrow window or beside an open rule. Turning off Show all events in a scrolled transcript no longer leaves it empty with stray violation dots.
- Rerail now checks an edit a coding agent made while a session's code rules were still opening, the first time Rerail read a large repository, when the agent edits the same file again before Rerail's first pass over the files. The first edit's lines used to go unchecked: no steer and no record of what they broke.
- In a submodule or nested repository inside one of the repository's other worktrees, Rerail records an agent's shell edit of a file a merge left conflicted and a file written and committed in one command, and no longer records the files that merged cleanly beside a conflict, or that a branch switch removed, as the agent's edits.
- On Windows the engine's in-process Git reads (ignore rules, the code-rule store) now take the config of the Git Rerail runs, such as Git for Windows' `core.autocrlf=true`, and start no Git from `PATH`. On every system they give no answer in a repository Git refuses, and run no filter program a repository configures.
- Rerail now checks an agent's edit to a file `git add -f` has just made tracked when Git wrote the index twice within one step of the file system's clock. Before, Rerail kept the earlier index and treated the file as still ignored, leaving its edits unchecked until Git wrote the index again.
- On Linux, hooks no longer wait while the daemon starts Git. Each Git start copied the whole daemon and paused it for tens to hundreds of milliseconds as it grew, so under load every session's hooks stalled together; the daemon now starts Git without copying itself.
- When several hooks find the daemon down at the same moment, one `rerail daemon start` runs. Before, a hook that looked just as the first one handed the start to its launcher and exited could launch a second start beside it; only one of them ever started a daemon.
- On Linux, a process that ends and is cleaned up at the very moment Rerail checks on it, such as a daemon `rerail daemon stop` has just stopped, now reads as gone. Before, that one check could find it still running, so a stop, a lock or a start waiting on it looked once more.
- Rerail now forgets a worktree once Git no longer lists it, as after `git worktree remove`, with its own settings: the daemon looks at start and hourly. A renamed clone, or one on an unplugged drive or a share that does not answer, stays with all its checkouts. Before, every checkout seen stayed, slowing each read of them.
- Starting Rerail in a repository with many worktrees no longer makes the daemon run Git once per worktree for every session working there. On a busy Windows machine those Git processes kept the daemon from answering hooks for a minute or more, so events went unrecorded.
- `rerail scan` (internal testing) in a copy of a checkout where Rerail is started, made with its `.git`, runs the Rerail home's rules there, as a session started in it would. Before, it ran no rules and failed.
- `rerail scan` (internal testing) that audits nothing, so runs no rules, now says why for each checkout it meant to audit in a `checkout-not-audited` failure, and its report's `rulesSource` is `null` instead of naming a configuration that never ran.
- The session's record now keeps only the file's digest, as for a direct edit, when an edit reaches a file in a folder named like one holding secrets through a folder link (`conf` to `.aws`), a Windows junction or a short folder name, or reaches a file named like one holding secrets by its Windows short name (`WP-CON~1.PHP`).
- A session a daemon left open by ending without its stop (a machine restart, a kill) is ended by the next daemon to start on its Rerail home, 30 minutes after its last event or sooner. For this version's Claude Code, Codex and Cursor sessions, that holds even when another process now has the dead daemon's process id.
- On Windows, a set-up question in the terminal waits for an answer given once it is on screen, as on macOS and Linux. Enter pressed while set-up was busy, such as during the download or the scan, no longer answers the next question with its first answer.
- A file a shell command was still writing, found by a check that ran meanwhile (another command's result, or a quiet session opened again), was checked half-written as the agent's edit, or its change logged as another program's. Code rules now check such a file once its command ends, against the text from before the command.
- `rerail status` keeps counting hook events that could not be recorded after its log of them passes 256 KB and moves aside, and says from when it counts once older ones are gone. Before, the count started over at each move, and hooks failing at the same moment could lose earlier failures.
- A steer a busy Rerail put in its answer to a hook that had already stopped waiting now reaches the coding agent at the session's next hook; before, the agent never got it.
- When an agent made several edits in a row, a rule broken by one was recorded at once but told to the agent only after every edit queued behind it was checked, up to six hooks later. Each tool call's steer now goes out at the next hook after its own check ends.
- A steer Rerail had not yet delivered when the daemon stopped or restarted, or freed the memory of a session quiet for a day, is no longer lost: the session's next event carries it, unless the file it names changed meanwhile. A finding held for the experimental missed-steer digest waits for the next digest the same way.
- Tool calls with many worktrees cost Rerail far less: at 120 worktrees, under 0.2 s of its CPU each instead of about 2 s. One rare case is slower: a file written into a worktree just after many of its files changed, beside another session, can be steered one or two tool calls late. Its finding is recorded when checked.
- On Windows, a URL with `//` and any scheme but `file:` or `smb:` in a tool call (`curl https://example.com/api`) no longer holds the daemon about 21 seconds when its host does not answer. On Linux and macOS, a web URL under `/net` is no longer looked up through the automounter. Doubled backslashes (`"C:\\Users\\me"` in JSON) are checked as single ones.
- TypeScript and JavaScript rules now read a file that uses `using` as a variable name, as in `const using = f(); using.length`. It was a syntax error, so every rule that reads code skipped the file. TypeScript `using` and `await using` declarations now declare their names, so `typescript-eslint-no-unused-vars` reports an unused one.
- On Windows, a file, folder, worktree or submodule of the repository that your account may not read, such as one behind a deny rule, no longer keeps Rerail from checking shell edits to the other files of the session's folder. In 0.1.4 only such a worktree did.
- A turn end's check for shell-written files now records each place it skipped, once, with its reason: a folder or file Rerail cannot read; a file over 1,000,000 bytes that a rule would read; a source or text file holding binary data, such as UTF-16 text; a walk that fails. The dashboard and session API count and list them.
- On Windows, Rerail no longer forgets a busy daemon: once four lookups had found it busy, the next waited 30 s, said it was not running and deleted its record, so later commands found none and a start could launch a second daemon. Lookups now read it as not answering within their usual wait and keep its record.
- On Windows, the daemon now starts when another program holds its engine pipe's name with a pipe that allows one instance: it listens on a pipe of its own with a random suffix, as it already did beside other holders, where every start failed with `listen EBUSY` and no daemon ran.
- On Windows, each of Rerail's own Git calls starts one `git.exe`: Rerail runs Git for Windows' own `git.exe` (in `ucrt64\bin` from 2.56) with the environment its launcher gives it, so with `MSYSTEM` set, Git reads that run a filter such as Git LFS's no longer fail. The Rerail daemon finds Git again after an upgrade removes it.
- On Windows, Rerail no longer starts a program by a drive-root path such as `\tools\git.exe`. In a PATH entry or `CLAUDE_CODE_GIT_BASH_PATH`, it gets the current folder's drive and is passed over inside the current repository; a bash passed over or missing leaves the status line as with the variable unset. In a conflict hook, it gets the checkout's drive.
- On Windows, a checkout's session records outside your profile folder are now private to you even when `.rerail\sessions` was made before Rerail wrote there: by Git from a repository that commits a file in it, by hand, or by Rerail 0.1.4. The daemon fixes the folder when it next opens a session there, your records already in it included.
- On Windows, Log mode and stops saved by Rerail 0.1.4 or earlier apply again in folders opened under another spelling. Set again by hand: a status line hidden through the daemon's API in a folder opened in other letter case, and a rule turned off in a folder outside Git opened through a mapped or subst drive or 8.3 name.
- Rerail now records a file another program writes in a worktree that was removed and added again at the same path as soon as Git lists that worktree again, while the repository has another linked worktree. It used to wait for some unrelated change in the repository when a walk of the worktrees ran while the folder was gone.
- With agent sessions in several worktrees of one repository, Rerail no longer runs Git again to list a worktree's changed files for a session that is still reading an earlier check of the worktrees, one that already listed them; each such repeat was one extra Git process per worktree, and its result went unused.

## 0.1.5 (2026-11-17)

0.1.5 makes Rerail lighter and steadier where it was strained: on Windows, in large repositories, and with many worktrees and sessions open at once. The daemon, the hooks and the `rerail` command use far less memory and start faster, sessions keep their events while the daemon is briefly busy, and Windows gets a long run of fixes. Set-up now matches rule packs to the linters a repository already runs, code rules read Java, and `rerail update` shows what it is doing. Before you update: features in internal testing, `rerail scan` and conversation rules included, are off until their variable is set ([Internal testing](docs/internal-testing.md)). On Windows, see Changed for a TLS key other accounts can read and for going back to an older Rerail.

### Added

- Code rules can read Java: a rule with `language: "java"` matches `.java` files by syntax. Name resolution, values, arguments, calls and format arguments are not read for Java yet; a rule or `transform` that uses one is kept and stays inactive until they are. `suppress`, `parse` and `languageOptions` are refused for Java.
- `rerail packs calibrate` reads a set-up repository's lint config again and updates the rule settings set-up gave, keeping any setting you made yourself; `--dry-run` shows the changes first. Settings set-up wrote before this release count as yours, so calibrate keeps them.
- Set-up runs Claude Code's Rerail hook once after connecting it, and reports the connection as failed when the hook does not run.
- `rerail status` says when hooks run but nothing is recorded, with the failing event and the cause, and names Claude Code hook scripts that are missing.
- `rerail update` ends by listing your installed rule packs that have a newer version for your Rerail, with the `rerail packs add` command that installs them, even when Rerail itself is current. It installs none. A pack is pinned to its version, so `rerail packs add <id>` without a version leaves it.
- `rerail update` and set-up's install name each step as it starts and show that a long one is still running. A step that runs long warns that npm may be stuck and what to check, and a hung npm is stopped.

### Changed

- The `rerail` command starts about three times faster in about half the memory: on a Windows laptop, `rerail version` takes about 0.4 s instead of 1.5 s, and `rerail status` and `rerail session` about 0.6 s instead of 1.7 s.
- Code rules take far less memory once compiled: the on-by-default rules of the JavaScript, TypeScript, Node.js, React and complexity packs together go from about 760 MiB to 150 MiB. The largest, the Node.js pack's `n-no-unsupported-features-node-builtins`, compiles in half the time.
- Each coding agent hook starts about twice as fast and uses about half the memory, peaking near 100 MB instead of 200 MB. Claude Code's status line, which runs every five seconds in each session, takes less CPU time and 16 MB less memory per run.
- An idle daemon no longer rereads each session's settings and audit queue four times a second, which took up to 1.5% of a CPU core per three open sessions on a Windows laptop; it rereads them only after the database changes.
- `rerail packs list`, the daemon's start and `rerail start` in a set-up checkout no longer compile every rule of every pack each time they run, only those of a pack that changed. With the Node, TypeScript, JavaScript, Python and complexity packs, `rerail packs list` takes 2 s instead of 16 s.
- Each line Rerail's daemon writes to `daemon.log` now starts with the UTC time it was written. A crash report from Node or the engine is left untimed, between timed lines.
- Every internal-testing feature is now off until its variable is set: `rerail scan` refuses, and conversation rules, command rules included, are not checked and show as Not running. `RERAIL_EXPERIMENTAL=1` turns them all on, `RERAIL_EXPERIMENTAL_<FEATURE>=1` one, set for the process that runs it; a running daemon must be restarted with it. [Internal testing](docs/internal-testing.md) lists them; a CI `rerail scan` needs `RERAIL_EXPERIMENTAL_HEADLESS_SCAN=1`.
- A coding agent's hook that finds no daemon running now starts one for the next event, so after `rerail daemon stop` the next tool call in a started checkout starts it again. Before, only a session's start did. With `RERAIL_AUTOSTART=0` it starts none, and a session that starts while no daemon runs is told so.
- The daemon closes a session that sends nothing for 30 minutes, or 24 hours while it is still owed a check, steer or finding. Its next event opens it again, and its first tool call waits up to 20 seconds for its rules to open, so that edit is still steered at its own result.
- What a scan or the first repository audit found no longer shows as session violations in the dashboard: no violation card, no logged or steered count. On update this applies to scans already recorded, the whole-repository scan 0.1.0 ran in every checkout included, except a first repository audit that a later scan or retry replaced. Their findings stay at `/api/audits?origin=scan`.
- Set-up now makes rule packs run the same rules as the repository's own linters, turning rules on as well as off. Beside ESLint, oxlint and Ruff it reads Biome, Bandit, Clippy, PHPStan, Psalm, PHP_CodeSniffer and SQLFluff configs, as data, never run. Where it cannot tell what a linter runs, the pack's values stand.
- Set-up's advice for upgrading a Node installed with fnm is now `fnm use --install-if-missing 24`, one command every shell runs, and with nvm-windows it adds `nvm use 24` after `nvm install 24`.
- Set-up explains rule packs in plain words: ready-made rules, mostly the ones linters use, suggested from what a repo is built with rather than from how its code is written. The sessions question says Rerail checks the coding agent's work, not you, and how to stop it.
- On Windows, Rerail supports Claude Code only with Git Bash, from Git for Windows. Without it Claude Code uses PowerShell, where Rerail's status line, `/rerail` commands and set-up's answers fail, as they did in 0.1.4; hooks still record and steer.
- On Windows, where `node` on PATH is not the Node that ran set-up, Claude Code's hooks start that Node through cmd.exe: one Node per event instead of two. Run `rerail install --remove` before going back to an older Rerail: older versions run each hook twice beside these entries, and their removal leaves them, which then fail on every event.
- On Windows, the content cache moves from `~/.cache/rerail` to `%USERPROFILE%\AppData\Local\rerail`, out of a roaming profile's reach. The copy an earlier release left in `~/.cache/rerail` stays until `rerail install --remove` deletes it.
- A daemon whose `--tls-key` other accounts can read (on Windows any account besides you, SYSTEM and Administrators, as under `C:\certs`) now serves this computer alone, over http on loopback. Before, Windows served https with it as asked, and macOS and Linux refused to start. `rerail daemon start` exits 1 if it asked for `--allow-remote` or `--public-host`.

### Fixed

- Rerail's database no longer grows by a copy of which rules ran with each audited edit, about 220 KB with 767 rules on: audits now share one copy per rule set. Audits already recorded move to it a week after the last audit an older Rerail recorded, and `rerail scrub` gives the freed space back.
- `RERAIL_AUTOSTART=false` now keeps coding agent hooks and `rerail setup` from starting the daemon, as `RERAIL_AUTOSTART=0` does. Before, only the background start when a session starts read `false`.
- With `BROWSER=none` set, `rerail manage`, `rerail session --open`, `rerail daemon start --open` and OMP's and Pi's `/rerail-manage` no longer open a browser or an Orca tab: each says no browser opened and gives the link to open yourself, as set-up already did. Outside a terminal, `rerail daemon start --open` says to run `rerail manage` instead.
- Claude Code's assistant text is recorded in a session's events as it streams. Each of those hooks failed with "Claude hook omitted index", because Claude Code sends a reply's `index` as a number and Rerail took only text, so only a turn's final message was kept.
- A hook no longer drops its event when the daemon is busy for a few seconds: it waits up to 12 seconds for the daemon to answer.
- When a running daemon takes more than 2 seconds to answer, the Claude Code status line, the OMP and Pi footer and `/rerail` now say it is not answering. Before, they said Rerail was not running and to start the daemon, which cannot help while it runs.
- With many worktrees, the daemon rereads another worktree when the system reports that something there may have changed, or every 10 minutes, not on each walk. Writes through hard links, memory maps, or a file another Windows program holds open can be recorded up to 10 minutes late.
- A running daemon whose `daemon.json` was removed writes it back within a second, and `rerail daemon start` and `rerail daemon stop` wait for it. Before, they started a second daemon that refused to run, or found none to stop.
- In a repository with many worktrees, the daemon walks the other worktrees once for all the repository's sessions, not once per session, and keeps one copy of what it read, with no text of files unchanged since the session started. Each session in another worktree now adds about half the memory it did.
- The daemon no longer runs out of memory on a large repository. It gives back a checkout's file records and compiled rules 15 minutes after its last session ends or is freed as quiet, shares one record per unchanged file among sessions, and restarts before its heap runs out.
- `rerail daemon stop` no longer waits 10 seconds and then kills the daemon with no clean-up when a coding agent hook's request stopped arriving partway. The daemon cuts such a request off after a second or two, and still answers each event it had taken in that ends within those 10 seconds.
- A worktree made while a session is open, in a folder that session had already written or walked, is now checked by the worktree's own rules. Before, Rerail skipped edits there or judged them by the checkout around it.
- When `git worktree remove` removes a worktree while Rerail is walking it, Rerail no longer records each of its files as deleted.
- With hundreds of worktrees, hooks no longer keep the daemon busy asking for the dashboard link on every event, each answer working out every worktree's state. Only a steer or a session start asks now.
- `rerail install --remove --delete-data` now deletes everything Rerail wrote in its home, then the home, instead of keeping your settings, logs and records. What it leaves, such as agent config backups, is named with the command that deletes it. `/rerail-uninstall` offers this full deletion beside removing with a backup.
- A file Git checks out with CRLF line breaks, under a `text` or `eol=crlf` attribute or `core.autocrlf`, now counts as Git's, so no rule steers a session on a file it never touched.
- When Git takes too long to list the files changed before a session, Rerail now asks again in the background, and a shell edit made after Git answers is steered only on the lines the agent changed. On a busy computer, Rerail no longer ends a Git command that already finished as if it hung.
- A session whose subagents or parallel tool calls run at once no longer loses their events to hooks waiting on each other: a hook lets go of the session's lock while the daemon checks its event, and on Windows waits through a lock file another hook is deleting instead of failing.
- A rule pack made for a newer Rerail is left out saying so (`it needs a newer Rerail … than this one`), not with the schema's refusal of what this Rerail could not read. Set-up keeps offering the version you have installed when the newest does not load.
- A coding agent's tool call that never got its result, such as an edit you refused in Claude Code or one whose result was lost while the daemon restarted, no longer leaves its file unchecked: what the call wrote is checked when the agent stops, and so is any later command that writes the file.
- `/rerail-create`'s guidance names all nine languages a syntax rule can target. It left out Rust, SQL and Vue, which code rules have read since before this release.
- `rerail rules evidence` now counts a session that started before its window but was still active in it, with the prompts, corrections, commands and steers from the window. Before, `--days 1` left out a session that had run since the day before, however much it did that day.
- Set-up run by a coding agent (`/rerail-start`, `rerail setup --relay` or `--json`) no longer opens a browser window; its summary shows the dashboard link instead. Set-up in a terminal still opens the dashboard.
- Installing Rerail again from the terminal or coding agent session that removed it now puts `~/.rerail/bin` back on `PATH`. Before, set-up skipped it because that shell's `PATH` still had the folder, so new terminals could not find `rerail`.
- Set-up, `rerail update` and `rerail packs add` work when your npm configuration sets `allow-scripts`, which npm 11 refuses for the installs Rerail makes for itself.
- `rerail support-bundle` no longer slows down as Rerail records more checkouts: with nearly 900 recorded, it takes under a second instead of over half a minute. It also gives a busy daemon 10 more seconds to answer, and says Rerail runs from its managed install only when it does.
- JavaScript and TypeScript files whose template holds a `\x` or `\u` with no hex digits after it, as ``String.raw`C:\x\users` `` does for a Windows path, now parse. Rules that read names reported "file has syntax errors" for such a file and skipped it whole.
- Just after a process holding one of Rerail's locks was killed, two set-ups, or a set-up and `rerail packs add`, no longer change the rule packs together, and sessions starting together no longer launch a second daemon that fails. On Windows, a process taking a lock as another lets go no longer fails with "EPERM: operation not permitted".
- `rerail install --remove` no longer keeps the shared content cache because some other program runs under the process number of a daemon that ended without its clean-up. On Windows that can still happen while another Rerail home's daemon runs.
- `rerail update` no longer says another update is already running after one was stopped or crashed: for up to 15 minutes after Ctrl-C, or until `update.lock` was deleted by hand when one stopped as it started. Two updates at once no longer fail with "Unexpected end of JSON input" or "EEXIST"; the second says another update is already running.
- When `rerail update` or set-up fails to install a version and then cannot remove the folder it installed into, it now says why the install failed first, then names the folder it left.
- A session's first command in a large checkout waits less while Rerail walks its files: a stopped or not-set-up checkout skips what Git ignores, as a set-up one does, and a walk looks up each folder's checkout once. On Windows, a walk in an 84,000-file checkout takes half a second, not 2 to 28 seconds.
- On Windows, a session's end and `rerail daemon stop` wait for every Git process Rerail started in your checkout to exit, so removing or renaming the folder right after, as `git worktree remove` does, no longer fails because the folder is in use.
- On Windows, Rerail no longer opens a terminal window every few seconds that takes focus: Git, Node and npm, which Rerail starts in the background, now run without a window.
- On Windows, Rerail records Cursor sessions: set-up registers Cursor's hooks in a form PowerShell runs, a workspace Cursor sends in any path form is found, and the Cursor CLI's file tools are kept out of Rerail's data.
- On Windows, set-up, `rerail update` and `rerail install --remove` no longer fail at once, or wait minutes, when a virus scan or another program holds open a file in a folder they remove. They retry for about 6 s, then leave a temporary folder for the system to clear, or name the file held open.
- On Windows, when another program holds open a file Rerail is replacing for longer than Rerail waits, Rerail no longer leaves a `.tmp` file beside it, and the error says another program held the file.
- On Windows, a hook keeps its event and its answer when a virus scan briefly holds its session's lock file, and a lock left by a hook that ended no longer holds up the session's next hooks for 30 seconds when Windows gives its process number to another program.
- On Windows, and in a checkout at `/workspace`, rules that resolve imports, such as the node pack's `n-no-missing-import`, now read the files and installed packages on disk; on Windows they also take an import or a tsconfig `extends` or `paths` entry written as an absolute path (`C:\repo\lib\a.js`). Before, they reported these imports, and installed packages, as missing.
- On Windows, rules run in sessions VS Code opens with a `c:\` path, Codex and Cursor get commands PowerShell runs, set-up's answers included (through the new `rerail setup --powershell`), and `rerail daemon stop` lets the daemon finish its work. On any system, Claude Code's plugin hooks work under a folder with a space in its path.
- On Windows, Rerail's MCP tools find the folder an agent works in when its path is spelt with 8.3 short names (`C:\Users\JOHNSM~1\...`), as a temp folder often is, instead of acting on the checkout's top folder.

### Security

- The daemon's HTTP server, `@hono/node-server`, moves from 2.1.5 to 2.1.8, which closes GHSA-jpvv-r467-5p4w: a backslash in a request's path was read as a path separator, which could get a request past path rules at a reverse proxy in front of the daemon.
- On Windows, Rerail starts every program by its full path, so a program of the same name committed to a repository never runs in its place; the folders it makes outside your profile, where Windows ignores file modes, are private to you; and it no longer follows a link planted where it reads or writes its files (`rerail scrub` included).

## 0.1.4 (2026-11-12)

0.1.4 stops steering an agent for changes it did not make: a file changed by another session, your editor or a background process no longer counts as its edit. A steer found as a turn ends now reaches Claude Code then, not at your next prompt. The dashboard shows each violation under the tool call it followed, and set-up works on Linux images with no compiler. Before you update: Claude Code 2.1.196 or newer is required.

### Added

- `rerail rules show <id>` prints a rule as a draft, and `rerail rules add` stores an edited pack rule as your copy, which turns the original off once approved. Changing where a rule applies, a Rerail pack's rule included, takes a few steps from any coding agent: `/rerail-create` changes only its scope, checks a few paths and shows the change.

### Changed

- Claude Code 2.1.196 or newer is required. Older versions do not tell hooks which prompt they belong to, so Rerail recorded none of their prompts or turn ends and steered nothing as a turn ended.
- Code rules: a file-level gate written once as a shared utility (for example `inside module` holding an import check) now skips a file that fails it as quickly as the same gate written in every clause.
- When no rule pack fits a repository, set-up and `rerail packs recommend` now say why: the files scanned, counted by ending, the folders not scanned, the packs whose excludes left out every file they look for, and what each pack looks for.

### Fixed

- Rerail steers a session only on files changed while one of its tool calls ran, or during its turn in Codex, whose `apply_patch` runs no hook. A file another session, your editor or a background process changed counted as an edit in every session there, so an agent that only read code was told about code it never wrote.
- In the dashboard's Claude Code sessions, a subagent's report and the reminders Claude Code adds to a prompt (`<task-notification>`, `<agent-message>`, `<system-reminder>`) no longer show as what you said, and no longer become the session's title.
- In History by session, a violation's card now sits under the tool call its audit followed, and the findings of one audit sit together under that call, instead of under the first call that mentioned the file or in a block at the end of the timeline.
- The dashboard now shows a violation that an audit in Log mode recorded as "Logged", and steered counts leave it out, even when its rule is on Steer: that audit never told the coding agent. Violations already recorded are corrected when you update.
- Rerail steers an agent for its shell edit of a file a merge or rebase left conflicted, even when the file still holds its side of the conflict, and no longer counts the files that merged cleanly beside a modify/delete conflict as the agent's edits.
- Python rules no longer read a line that opens with `[` after an unpacking assignment, such as `a, b = c, d()`, as part of that assignment. `ruff-f823-undefined-local` reported a variable read on that line as read before the assignment bound it.
- When Stop hooks keep a Claude Code or Codex turn going, Rerail now records the turn ends that follow and checks them against conversation rules, counting those that end on the same final message once. Before, it recorded none of these turn ends after the first one in Claude Code, and none after the second in Codex.
- `rerail rules test` counts a draft's matches only where `rules add` would run the rule: for a rule of yours, the draft's `folder` and where it is added; for your copy of a pack's rule, where the original is. It counted the whole repository before. `--json` lists the folders (`repository.folders`), and why `rules add` would refuse the draft (`refusal`).
- On macOS and Linux, a set-up question in the terminal waits for an answer given once it is on screen. Enter pressed while set-up was busy, such as during the download or the scan, no longer answers the next question with its first answer, which allowed session access, scanned the repository or started Rerail without asking.
- `npx @neralabs/rerail setup` no longer compiles `better-sqlite3` from source, so it works on a Linux image with no compiler, such as `node:24-slim`, where it failed with `not found: make`. It uses the prebuilt binary `better-sqlite3` ships, as `rerail update` already did.
- A shell edit to a file that already had unstaged changes when the session started, or was new and not ignored by Git, is now measured from the text the session first saw, so the agent is steered only on the lines it changed, not on every finding in the file.
- A steer found as a turn ends reaches the agent when you see its line, and counts then. Claude Code gets it as the turn ends and carries on to act on it, not at your next prompt's first tool call; in Codex the line waits for the steer.

## 0.1.3 (2026-11-06)

0.1.3 audits your agent's edits in every worktree of a started repository, not only the one its session started in, and stops counting files it restored from another branch as its own work. While a repository follows `.gitignore`, edits of files Git ignores, and changes that make Git ignore more, are recorded on the session as unchecked. The daemon starts again after a crash left its socket behind, the Claude Code, OMP and Pi footer is gone where Rerail is stopped, and code rules whose `define` shares utilities compile faster and in less memory. Before you update: if your own code imports `createPiConversationRulesExtension`, import it from `@neralabs/rerail/pi`; Pi itself needs no change.

### Added

- While a repository follows `.gitignore`, a tool edit of a file Git ignores is recorded on the session as unchecked, with the ignore file, line and pattern. So is a change that makes Git's ignore rules ignore more, in a `.gitignore` or in `.git/info/exclude`, which no diff shows. The dashboard timeline counts both.

### Changed

- Code rules whose `define` shares utilities with many queries compile faster and in less memory, and report the same. A utility's `all` item that reads a capture an earlier item binds through another utility is no longer refused as reading it before it is bound.
- `createPiConversationRulesExtension` is now imported from `@neralabs/rerail/pi`, not `@neralabs/rerail`, so the main entry no longer needs `typebox`, which Pi supplies. `typebox` is an optional peer dependency. Pi itself loads Rerail's extension as before.

### Fixed

- `rerail daemon start` no longer fails with `EADDRINUSE` after a daemon was killed (`kill -9`, a crash): the engine socket it left behind is removed when no process answers on it, and the start goes on. When a running daemon does answer there, the error says so instead.
- In the dashboard's History by session, a window 901 to 1180px wide no longer squeezes the transcript into a sliver that splits every word: the session list folds into the Session picker, as in a narrower window, and messages wrap between words, breaking only a token too long for its line, such as a file path.
- The Claude Code, OMP and Pi footer is gone where Rerail is stopped, or was never started: a new session there no longer shows `R E R A I L ○ not on here`. `/rerail` and `rerail status` still say Rerail is not on here, and how to start it.
- At session start, including after `/clear`, the footer says `R E R A I L ● starting` until Rerail has recorded the new session, rather than `session unavailable`, which now means only that the session could not be read.
- An agent's edits in a worktree of a started repository are audited and show on the repository's dashboard, whichever folder its session started in. The agent is steered on its file-tool edits and on shell writes in its own folder; a shell write in another worktree is recorded but steers no session. A worktree stopped on its own stays stopped.
- In Pi, a steer sent as its own message opens with a line saying it is an automated note from Rerail, not from you, and advice rather than a new task, so the agent fixes it and goes back to your request instead of treating it as new instructions.
- Pi no longer warns at start-up that Rerail's extension package declares `typebox` in `dependencies`. Rerail now uses the copy Pi provides.
- Rerail no longer steers an agent for files it restored from another branch. `git show origin/main:path > path`, `git checkout main -- path` or `git restore --source main path` counted as the agent writing every line that differed from the checked-out commit, so restoring files to `main` drew findings on code `main` already had.

## 0.1.2 (2026-11-04)

0.1.2 stops counting what Git writes as your agent's work: after a pull, rebase or branch switch, an agent no longer gets findings for code it never touched. It also keeps Rerail's Git calls from fetching from the remote in a partial clone, and corrects what set-up and `rerail install --remove` tell you. Before you update: if you set `RERAIL_STORAGE_ROOT` or `RERAIL_MODEL_PATH`, check the values, since Rerail now reads them everywhere and resolves a relative one against the session's workspace.

### Changed

- `RERAIL_STORAGE_ROOT` and `RERAIL_MODEL_PATH` mean the same everywhere: a relative value resolves against the session's workspace, an empty one counts as unset, and the daemon and Pi through it now read them too. Only the default storage root, `<workspace>/.rerail/sessions`, gets its `.gitignore`; a custom root inside another `.rerail` folder is left alone.

### Fixed

- In a partial clone (`git clone --filter=blob:none`), Rerail's Git calls no longer fetch missing objects from the remote, with Git 2.45 or a 2024-04-19 security release (2.39.4 to 2.44.1): every Git process sets `GIT_NO_LAZY_FETCH=1`, and a git-diff audit lists a renamed file under its new path. The Claude Code set-up check runs Git the same way.
- Rerail no longer steers or records a session for files Git rewrote. A checkout, pull, merge, rebase, reset or new worktree counted as an edit of every file it changed, in every session open in that checkout, so an agent that wrote nothing got findings for code it never touched.
- `rerail` usage now names the version you are running as the release that leaves out features in internal testing, rather than always 0.1.0, and `/rerail-create` says the latest release.
- Set-up's "Not now" screen writes the start command as each coding agent types it: `$rerail-start` where Codex is the only one, and an "In Codex, type $rerail-start." line beside `/rerail-start` where Codex is one of several. The confirm question's "Adds /rerail commands" line follows the same rule.
- `rerail install --remove` counts the daemon's `access-token` as Rerail's own: a Rerail home holding it and nothing of yours is again offered for deletion as a whole, and a shared home lists `access-token` among Rerail's files instead of calling it a file that is not Rerail's.

## 0.1.1 (2026-11-04)

0.1.1 locks the daemon to you, keeps each coding agent's Rerail tools inside its own repository, and loads rules from a checkout's own folder and from third parties' packs, in Log until you approve them. Before you update: open the dashboard with `rerail manage`, since old bookmarks stop working; have scripts that call the daemon send `x-rerail-access`; and give a daemon reachable from other machines a TLS certificate or `--insecure-http`.

### Added

- `/rerail-create` also writes architecture rules: where code belongs and which way dependencies run.
- Code rules gain a `file` query, for files that should not exist, and `importTarget`, for what a JavaScript or TypeScript import points at ([DSL reference](docs/code-rule-dsl-reference.html#importTarget)).
- `rerail install --remove --delete-data` also deletes Rerail's data; add `--dry-run` to list it first.
- The docs are a website, [neralabs.github.io/rerail](https://neralabs.github.io/rerail/), with architecture rule examples from real reviews and a readable DSL reference. Every page is also served as Markdown, with `llms.txt`, for coding agents.
- `rerail support-bundle` writes one diagnostics file to attach to an issue, with no prompts, code or rule text, and `/rerail-bug-report` walks you through it ([support bundles](docs/privacy.md#support-bundles)).
- Third parties' rule packs load with `rerail packs add`, from npm (`rerail-pack-<id>`) or a folder, marked as not reviewed by Rerail ([third parties' packs](docs/rule-packs.md#third-parties-packs)).
- Rule documents in a checkout's `.rerail/rules/` run in that checkout without being installed ([a worktree's own rules](docs/rule-packs.md#a-worktrees-own-rules)).

### Changed

- Rules Rerail did not publish run in Log until you approve them with `rerail rules approve` or the dashboard, and again after each change. Library rules you already have stay approved ([approval](docs/rule-scoping.md#approval)).
- The dashboard's copy is shorter, its top bar and toasts say what to do next, and its tiles open History filtered to them.
- Rerail no longer scans the whole repository when it first sees a checkout, so violations the code already held no longer fill a fresh dashboard.
- Rerail runs the Git on your `PATH`, skipping entries inside the repository or `node_modules`.
- Rerail and its rule packs are public on npm, so installing them needs no npm account.
- Rules with a `regex` read every text file in their scope, `Justfile`, `LICENSE` and lock files included.

### Removed

- `rerail update rollback` is removed. `rerail update` still saves `rerail.sqlite.pre-update`; to go back, stop Rerail and copy it over `rerail.sqlite` ([database compatibility](docs/how-setup-works.md#database-compatibility)).

### Fixed

- JavaScript, TypeScript and Python rules match ESLint, typescript-eslint and Ruff in more cases, and a `rerail-ignore` before a SQL or Rust block covers the whole block.
- `rerail status` and the dashboard report a broken bundled model instead of a healthy daemon.
- A panic in the native engine fails the check that hit it instead of stopping the daemon.
- Installing Rerail from Pi's package gallery no longer loads it twice.
- `rerail rules test` no longer stops on large repositories, and `--json` lists every match.

### Security

- Every request to the daemon needs its access capability, so other users and sandboxed processes on your machine get nothing. Open the dashboard with `rerail manage`, and have scripts send `x-rerail-access` from `daemon.json` ([troubleshooting](docs/troubleshooting.md#the-dashboard-asks-to-connect-this-browser)).
- A coding agent's Rerail tools reach only its own repository, and the transcripts they read leave out tool output and secret-like paths.
- A daemon reachable from other machines needs `--tls-cert` and `--tls-key`, or `--insecure-http` to stay on plain HTTP.
- The daemon limits request size and time, Rerail's files and sockets are private to you, more file names count as secret-like and stay out of `rerail rules test` and stored transcripts, and steers drop control and invisible characters.
- Rerail redacts the secrets it recognizes before writing anything to disk, and `rerail scrub` cleans what earlier versions stored.
- Set-up keeps Claude Code and the Cursor CLI out of Rerail's data, and `rerail status` says what each coding agent can reach ([details](docs/installation.md#what-a-coding-agent-can-reach)).
- `rerail update` checks npm signatures and provenance, ignores a repository's `.npmrc`, runs no install scripts, and installs the dependency versions Rerail was tested with.

## 0.1.0 (2026-10-27)

First release of Rerail, licensed under Apache-2.0. Rerail keeps coding agents on your rules, with checks that live outside the agent: it audits what Claude Code, Codex, Cursor, OMP or Pi writes, steers the agent in the same session, and shows every session, rule and violation in a dashboard.

To start, run this in a repository:

```bash
npx @neralabs/rerail setup
```

### Rules

- Code rules check what an agent writes in Python, JavaScript, TypeScript, PHP, Blade, SQL, Rust and Vue. A native engine runs them on your computer.
- Each rule either logs a violation or steers the agent to fix it. Turn a rule or pack on or off, or change its mode, for a repository, a checkout or a folder.
- An edit hears only about the lines it changed, not problems already in the file.
- A file or rule Rerail could not fully check is reported as not checked, never as passing.
- Comments that silence a linter also silence the rules ported from it, and `rerail-ignore` silences any rule by its id.

### Rule packs

- Rules come in packs, each its own npm package. Set-up recommends the ones that fit your repository, says why, and installs nothing until you answer.
- Packs with rules in this release: complexity, JavaScript, TypeScript, Next.js, Python, PHP, Blade, SQL, Rust and Vue.
- Set-up reads your ESLint, Oxlint and Ruff configs without running them, and offers to start the rules they turn off as off.
- Manage packs with `rerail packs list`, `add`, `remove` and `recommend`.

### Your own rules

- `/rerail-create` writes a rule from your own words: the agent drafts it, tests it on your repository, reads it back, and adds it once you confirm. With no words, it suggests rules from your sessions.
- `/rerail-show-me` shows Rerail catching a problem in your own repository in under a minute, then cleans up.
- Rules belong to the repository, so every clone and worktree shares them. Add one for the whole repository or a single folder.
- Move rules between computers with `rerail rules export` and `rerail rules import`.

### Coding agents

- Every agent gets the same commands: `/rerail`, `/rerail-manage`, `/rerail-create`, `/rerail-show-me`, `/rerail-start`, `/rerail-stop`, `/rerail-update`, `/rerail-uninstall` and `/rerail-help`. Codex writes them as `$rerail`.
- Rerail is off in every repository until you start it there.
- A steer is one line and a link for you, and a block for the agent, which may decline it by telling you why.
- Claude Code, OMP and Pi show a status line with how many rules are on and how often Rerail steered.
- A stopped daemon never stalls a tool call, and a busy one holds it for 25 seconds at most.

### Dashboard

- See rule packs, sessions and violations, with history by rule and by session. Open it with `rerail manage`.
- Files Git ignores are not checked, unless you turn off Follow .gitignore for the repository.
- The daemon listens on this computer only unless you pass `--allow-remote`.
- The daemon also serves an HTTP API and MCP tools.

### Install and update

- Needs Node 24 or newer and Git. Runs on macOS, Linux (glibc 2.28 or newer) and Windows, on arm64 and x64.
- Set-up asks before it reads your coding agent sessions, and again before it reads a repository.
- Updates happen only when you ask: `rerail update` backs up the database first, and `rerail update rollback` goes back.
- `rerail install --remove`, or `/rerail-uninstall`, takes Rerail off the computer and keeps your database in a dated backup.

### For tool builders

- `@neralabs/rerail/rules` validates a code rule as Rerail's own writes do, and `@neralabs/rerail/schemas` exports the rule schemas.

### Upgrading from a preview

- Core no longer ships the complexity pack. If a repository uses it, run `rerail packs add complexity`.
- `/rerail-ui` is now `/rerail-manage`, `rerail ui` is `rerail manage`, and `/rerail-rage-quit` is now `/rerail-uninstall`.
- Rules come only from the repository's configuration: Rerail no longer reads conversation rule files, and `rerail install` no longer takes `--project`.
- On Windows, Codex asks you to trust its hook once more with `/hooks`.

[Every detail of this release](https://github.com/neralabs/rerail/blob/v0.1.0/CHANGELOG.md#010-2026-10-27) is in the changelog at its tag.
