# Changelog

Every notable change to Rerail's core package, `@neralabs/rerail`, latest first. From 0.1.1 the notes follow [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/): each release lists its changes under Added, Changed, Deprecated, Removed, Fixed and Security. There are two differences: a heading reads `## <version> (<date>)` rather than `## [<version>] - <date>`, and changes waiting for the next release are in [.changelog](.changelog/README.md), so `## Unreleased` holds only that release's opening paragraph.

Versions are numbered as in [Semantic Versioning](https://semver.org/spec/v2.0.0.html), which promises nothing about compatibility below 1.0.0. A release is a patch unless it names a later version, and a patch can change or remove what you use, so read the notes before you update.

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
