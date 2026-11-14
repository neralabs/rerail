# Changelog

Every notable change to Rerail's core package, `@neralabs/rerail`, latest first. From 0.1.1 the notes follow [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/): each release lists its changes under Added, Changed, Deprecated, Removed, Fixed and Security. There are two differences: a heading reads `## <version> (<date>)` rather than `## [<version>] - <date>`, and changes waiting for the next release are in [.changelog](.changelog/README.md), so `## Unreleased` holds only that release's opening paragraph.

Versions are numbered as in [Semantic Versioning](https://semver.org/spec/v2.0.0.html), which promises nothing about compatibility below 1.0.0. A release is a patch unless it names a later version, and a patch can change or remove what you use, so read the notes before you update.

## 0.1.5 (2026-11-14)

### Added

- Code rules can read Java. A rule with `language: "java"` matches `.java` files by syntax: `kind`, `pattern`, `regex`, the relations, `field`, `nthChild`, `count`, `layout` and `matches`. Name resolution, values, arguments, calls and format arguments are not read for Java yet; a rule or `transform` that uses one is kept and stays inactive until they are.
- Set-up runs Claude Code's Rerail hook once after connecting it, and reports the connection as failed when the hook does not run.
- `rerail status` says when hooks run but nothing is recorded, with the failing event, the coding agent's version and the cause, and names Claude Code hook scripts that are missing. A hook that finds no daemon running starts one for the next event; with `RERAIL_AUTOSTART=0` it starts none, and `rerail status` says so.

### Changed

- Claude Code on Windows needs Git Bash, from Git for Windows. Claude Code runs without it, using PowerShell, but Rerail writes its status line, `/rerail` commands and set-up's answers for Git Bash there; hooks still record and steer.
- The daemon closes a session that sends nothing for 30 minutes, or 24 hours while it is still owed a check, steer or finding. Its next event opens it again, and its first tool call waits up to 20 seconds for its rules to open, so that edit is still steered at its own result.
- Set-up's advice for upgrading a Node installed with fnm is now `fnm use --install-if-missing 24`, one command every shell runs, and with nvm-windows it adds `nvm use 24` after `nvm install 24`. `rerail setup --powershell` writes each answer command for PowerShell, as Codex's and Cursor's `/rerail-start` ask for on Windows.
- An audit that walks folders itself now reads each folder's files in path order, as Git sorts them: a repository audit outside a Git checkout or where its repository storage did not attach, and a touched-files audit. Before, it took each folder's files last-listed first, so when more than 20 files failed, which 20 it listed could differ between machines.
- On Windows, the content cache moves from `~/.cache/rerail` to `%USERPROFILE%\AppData\Local\rerail`, out of a roaming profile's reach. The copy an earlier release left in `~/.cache/rerail` stays until `rerail install --remove` deletes it.
- On Windows, a daemon started with `--tls-key` refuses a key that any account besides you, SYSTEM and Administrators can read, as every local user can a key under `C:\certs`. The error names those accounts and the `icacls` command that makes the key yours alone; run it and start the daemon again.

### Fixed

- Rerail's database no longer grows by a copy of which rules ran, and where, with each audited edit: about 220 KB with 767 rules on. New audits share one copy per rule set; audits recorded before the update keep their own.
- `RERAIL_AUTOSTART=false` now keeps every part of Rerail from starting the daemon, as `RERAIL_AUTOSTART=0` does. Before, a session's start honoured `false`, but a coding agent's hook and `rerail setup` still started the daemon.
- With `BROWSER=none` set, `rerail manage`, `rerail session --open`, `rerail daemon start --open` and OMP's and Pi's `/rerail-manage` no longer open a browser or an Orca tab: each says no browser opened and gives the link to open yourself, as set-up already did.
- When a running daemon takes more than 2 seconds to answer, the Claude Code status line, the OMP and Pi footer and `/rerail` now say it is not answering. Before, they said Rerail was not running and to start the daemon, which cannot help while it runs. In OMP and Pi, `/rerail` says so after 2 seconds, not 4.
- Each turn costs the daemon a little less CPU: looking up which checkout holds a folder, which a walk does for every folder on every turn, reuses one prepared database query, and a walk of a 17,700-file checkout takes about a tenth less time on a Windows laptop.
- Claude Code sessions record the assistant's replies again. Claude Code sends each reply's `index` as a number, and Rerail took only text, so every reply was dropped with "Claude hook omitted index".
- Claude Code replies that hold a blank line, or end on a newline, are recorded whole. Rerail dropped each such batch with "Claude hook omitted delta", so the reply never finished in the session's history and conversation rules never checked it as a whole message.
- The rerail command starts about three times faster and in less than half the memory: `rerail version` takes about 0.4 s instead of 1.5 s, and `rerail status` and `rerail session` about 0.6 s instead of 1.7 s, because the command now ships as one bundled file.
- Code rules take far less memory once compiled. The Node.js pack's `n-no-unsupported-features-node-builtins` held about 500 MiB and now holds under 90 MiB, and it compiles in half the time. The on-by-default rules of the JavaScript, TypeScript, Node.js, React and complexity packs together go from about 760 MiB to 150 MiB.
- In a repository with many worktrees, Rerail's daemon no longer keeps what a session read of the other worktrees once the session ends. It kept a copy for each folder a session had closed in, for 15 minutes after, so its memory grew with each such folder: about 36 MB a folder with 190 worktrees of 500 files.
- The daemon no longer runs out of memory on a large repository or grows with each session. It frees a checkout's file records 15 minutes after the checkout's last session ends, keeps each record a fifth the size, shares one model and one record per unchanged file among sessions, closes idle sessions, and restarts before its heap runs out.
- With hundreds of worktrees, hooks no longer keep Rerail busy asking for the dashboard link: every event asked, steer or not, and each answer worked out every worktree's state. Now only a steer or a session start asks, and the dashboard reads every worktree's state at once.
- `rerail install --remove --delete-data` now deletes everything Rerail wrote in its home, then the home, instead of keeping your settings, logs and device secrets. What it cannot reach, such as agent config backups, is named with the command that deletes it. `/rerail-uninstall` offers this full deletion beside removing with a backup.
- A file Git checks out under a `text` or `eol=crlf` attribute now counts as Git's when its committed copy already held a CRLF, a lone CR or a NUL, so no rule steers a session on a file it never touched.
- Each coding agent hook, the event stream, and the Claude Code status line while the daemon is busy or down start faster and use about a third of the memory: a hook peaked at 285 MB and took half a second to load before it did anything, and now peaks near 85 MB.
- A hook no longer drops its event when the daemon is busy for a few seconds: it waits up to 12 seconds for the daemon to answer as itself, and sends nothing to a daemon that does not. No hook starts a needless Rerail process beside a busy daemon.
- An idle daemon no longer spends about 1.5% of a CPU core per three open sessions rereading each session's settings and audit queue four times a second; it rereads them only after another process changes the database.
- A daemon gives back the memory its compiled rules hold 15 minutes after a repository's last session ends. It kept every rule it had compiled for as long as it ran: about 740 MB with the Node, TypeScript, JavaScript, Python and complexity packs, 512 MB of it the Node pack's builtins rule.
- Rules that resolve imports and requires, such as the node pack's `n-no-missing-import`, read the files and installed packages on disk below the checkout on Windows, and in a checkout at `/workspace`. They read only the files a scan held there, so they reported an import of any other file, or of an installed package, as missing.
- Set-up, `rerail update` and `rerail packs add` work when your npm configuration sets `allow-scripts`, which npm 11 refuses for the installs Rerail makes for itself.
- Installing Rerail again from the terminal or coding agent session that removed it now puts `~/.rerail/bin` back on `PATH`. That shell's `PATH` still had the folder, so set-up wrote nothing, new terminals could not find `rerail`, and `rerail update` never added it.
- In OMP and Pi, an event now waits at most 12 seconds for a busy daemon, as a hook does: it waited up to 14 seconds, and 24 with `RERAIL_AUTOSTART=0`, before the agent could go on.
- Commands that read your rule packs, such as `rerail packs list` and `rerail start`, and the daemon's start no longer compile every rule of every pack: Rerail keeps the engine's verdict on each pack until the pack or Rerail changes. With the Node, TypeScript, JavaScript, Python and complexity packs, `rerail packs list` takes 2 s instead of 16 s.
- A session whose subagents or parallel tool calls run at once no longer loses their events to hooks waiting on each other: its hooks wait for a busy daemon side by side, let go of the session's lock while the daemon checks an event, and on Windows wait through a lock file another hook is deleting instead of failing.
- A coding agent's tool call that never got its result, such as an edit you refused in Claude Code or one whose result was lost while the daemon restarted, no longer leaves its file unchecked: what the call wrote is checked when the agent stops or the session ends, and so is any later command that writes the file.
- When `git worktree remove` removes a worktree while Rerail is walking it, Rerail no longer records each of its files as deleted.
- `rerail rules evidence` now counts a session that started before its window but was still active in it, with the prompts, corrections, commands and steers from the window. Before, `--days 1` left out a session that had run since the day before, however much it did that day.
- Set-up run by a coding agent (`/rerail-start`, `rerail setup --relay` or `--json`) no longer opens a browser window, which could appear on your screen from a sandboxed or unattended run; its summary shows the dashboard link instead. Set-up in a terminal still opens the dashboard.
- Claude Code's status line, which runs every five seconds in each session, now takes about half the CPU time and 16 MB less memory per run: it ships as one bundled file and no longer loads the database schema to read the daemon's answer.
- `rerail support-bundle` takes seconds, not minutes, once Rerail has recorded hundreds of checkouts, gives a busy daemon 10 more seconds to answer, and says Rerail runs from its managed install only when it does. Each line the daemon writes to `daemon.log` now starts with the UTC time it was written.
- JavaScript and TypeScript files whose template holds a `\x` or `\u` with no hex digits after it, as ``String.raw`C:\x\users` `` does for a Windows path, now parse. Rules that read names reported "file has syntax errors" for such a file and skipped it whole.
- Sessions starting the daemon at once no longer launch two daemons for one Rerail home, and two set-ups, or a set-up and `rerail packs add`, no longer change the rule packs together, even just after a process holding the lock was killed: the system now lets go of Rerail's locks when their process ends.
- `rerail install --remove` removes the shared content cache unless a running Rerail daemon holds it. It kept the cache, saying another daemon used it, whenever the process number the cache's record names was in use, which after a daemon ended without its cleanup could be any program, and on Windows whenever another Rerail home's daemon ran.
- `rerail update` no longer says another update is already running after one was stopped or crashed: for up to 15 minutes after Ctrl-C, or until `update.lock` was deleted by hand when one stopped as it started. Two updates at once no longer fail with "Unexpected end of JSON input" or "EEXIST"; the second says another update is already running.
- When `rerail update` or set-up fails to install a version and then cannot remove the folder it installed into, it now says why the install failed first, then names the folder it left, instead of only why that folder could not be removed.
- A session's first command in a large checkout no longer waits up to 10 seconds while Rerail walks its files: a stopped or not-set-up checkout skips what Git ignores, as a set-up one does, and a walk looks up each folder's checkout once. On Windows, a walk of 84,000 files takes half a second instead of 2 to 28 seconds.
- On Windows, an import written as an absolute Windows path (`C:\repo\lib\a.js`), and a tsconfig `extends` or `paths` entry written as one, name that file, as Node's resolvers read them. Rules that resolve imports, such as the node pack's `n-no-missing-import`, reported such an import as missing.
- On Windows, `rerail profile remove` and finding rule packs during set-up no longer fail when a virus scan or the search indexer holds a file in a folder they remove for a moment: they wait for it to let go.
- On Windows, a session's end and `rerail daemon stop` wait for every Git process Rerail started in your checkout to exit, so removing or renaming the folder right after, as `git worktree remove` does, no longer fails with "resource busy or locked".
- On Windows, Rerail no longer opens a terminal window every few seconds that takes focus: Git, Node and npm, which Rerail starts in the background, now run without a window.
- On Windows, Rerail records Cursor sessions: set-up registers Cursor's hooks in a form PowerShell runs, a workspace Cursor sends in any path form is found, and the Cursor CLI's file tools are kept out of Rerail's data. Pi's `rerail_fix` tool starts a Bun-built `pi.exe` without a stray argument.
- On Windows, set-up, `rerail update` and `rerail install --remove` no longer wait minutes or longer when another program holds open a file in a folder they remove. They retry for about 6 s, then leave a temporary folder for the system to clear, or fail naming the file the other program held open.
- On Windows, when another program holds open a file Rerail is replacing for longer than Rerail waits, Rerail no longer leaves a `.tmp` file beside it, in the Rerail home or a repository's `.claude` folder, and the error says that another program held the file.
- On Windows, a hook keeps its event and its answer when a virus scan briefly holds its session's lock file, and a lock left by a hook that ended no longer holds up the session's next hooks for 30 seconds when Windows gives its process number to another program.
- On Windows, Rerail no longer fails with "EPERM: operation not permitted" when one process lets go of a lock as another takes it: two set-ups, or a set-up and `rerail packs add`, and sessions starting the daemon wait their turn, and a second `rerail update` says another update is running.
- On Windows, rules run in sessions VS Code opens with a `c:\` path, plugin hooks work under a profile folder with a space, Codex and Cursor get commands PowerShell runs, files Git rewrote with CRLF line breaks count as Git's, and `rerail daemon stop` lets the daemon finish its work.
- On Windows, Rerail's MCP tools find the folder an agent works in when its path is spelt with 8.3 short names (`C:\Users\JOHNSM~1\...`), as a temp folder often is, instead of acting on the checkout's top folder.
- Rerail's daemon no longer makes a text copy of each file, unchanged since your session started, that it reads the first time it walks a worktree or a folder it could not read before; it keeps the file's digest, and its text once the file changes. Those copies made the daemon's memory climb by up to a gigabyte a minute.

### Security

- On Windows, Rerail starts every program by its full path, so a `git.exe` or `powershell.exe` committed to a repository never runs in its place; the folders it makes outside your profile, where Windows ignores file modes, are private to you; and it no longer follows a link planted where it reads or writes its files (`rerail scrub` included).

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

- Every request to the daemon needs its access capability, so other users and sandboxed processes on your machine get nothing. Open the dashboard with `rerail manage`, and have scripts send `x-rerail-access` from `daemon.json` ([troubleshooting](docs/troubleshooting.md#the-dashboard-says-needs-the-dashboard-link)).
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
