# Installing Rerail

Rerail installs with one command, run in the repository you want it in. Set-up installs Rerail under your home folder,
connects your coding agents, and starts Rerail in that repository once you say yes. This page covers what you need
first, what set-up asks and changes, and how to update, stop and remove Rerail. [How set-up works](how-setup-works.md)
has the details behind each step, and [Troubleshooting](troubleshooting.md) answers what goes wrong. To run Rerail from
a source checkout instead, see [Developing from a checkout](how-setup-works.md#developing-from-a-checkout).

## Requirements

- **Node.js 24 or newer.**
- **Git on your `PATH`**, since Rerail reads every repository through it.
- **macOS, Linux with glibc 2.28 or newer, or Windows**, on arm64 or x64. The package carries prebuilt native engines,
  so nothing is compiled on your machine. Alpine and other musl Linux are not supported;
  [Supported and not supported](#supported-and-not-supported) has the full list.
- **npmjs**, the only registry Rerail is published to. The package is public, so no npm account is needed.

Set-up checks Node, the platform and Git before it installs anything. When one is missing it stops, says what to do,
and changes nothing.

## Install

In the repository you want Rerail in:

```sh
npx @neralabs/rerail setup
```

Set-up installs Rerail under `~/.rerail`, with no admin rights, and turns nothing on. Then it asks:

1. Once per computer, whether Rerail may read the sessions of the coding agents it connects to (what you ask, what they
   do, the files they edit) to check them against your rules. "No" stops set-up with Rerail installed and off.
2. Whether it may read this repository. Outside a repository it asks which folder to start with, or you can skip for
   now; `--repo <folder>` names one. You can also skip the scan and pick rule packs yourself, and then no code is read.
3. Whether to start with the rule packs it recommends, each shown with why it matched, and the coding agents it will
   connect. You can change the rule packs, or answer "Not now", which installs, connects and turns on nothing.

Once you say yes, it connects the coding agents it finds (Claude Code, Codex, Cursor, OMP and Pi), or only the ones you
name with `--agents`, starts Rerail in this repository only, and ends with a summary and a link to the dashboard. Start
a new coding agent session in the repository; Rerail runs there. In a new terminal, `rerail manage` opens the dashboard.

In a terminal, move between answers with ↑/↓ and press Enter, or type an answer's number. To have a coding agent
install Rerail, ask it to: it runs `setup --relay` and asks you each question in the session.

Run set-up again at any time: it is idempotent, so it doubles as an upgrade. In a repository that is set up already it
changes nothing there and connects only the coding agents installed since set-up or `rerail install` last looked,
never one you left out with `--agents` or removed with `rerail install --remove --agents <id>`. Name one to connect it
anyway, for example `npx @neralabs/rerail setup --agents codex`.

## What it changes on your machine

- **`~/.rerail`** (or `RERAIL_HOME`): the installed package, the `rerail` launcher in `~/.rerail/bin`, the database
  of your rules and session history (`rerail.sqlite`), and the daemon's files. Its cache is `~/.cache/rerail`.
- **Your shell's startup file**, so `rerail` runs in a new terminal: a block between `# >>> rerail >>>` and
  `# <<< rerail <<<` on macOS and Linux, or the user `Path` on Windows. `RERAIL_MODIFY_PATH=0` leaves `PATH` to you.
- **Your coding agents' user settings** in `~/.claude`, `~/.codex`, `~/.cursor`, `~/.omp/agent` and `~/.pi/agent`:
  hooks or an extension, the Rerail commands, and denies that keep the agent out of Rerail's data. Other tools'
  entries stay, and a file that changes gets a `<file>.<timestamp>.bak` backup beside it.
- **Nothing in the repository**, its Git metadata or the database, before you say yes.

[How set-up works](how-setup-works.md) lists every step, file and entry.

## Other ways to install

### From Pi's package gallery

Pi's [package gallery](https://pi.dev/packages) lists Rerail, and `pi install npm:@neralabs/rerail` works: the next Pi
session loads Rerail and says to start it with `/rerail-start`. `/rerail-start` then finishes set-up, and in
`~/.pi/agent/settings.json` it swaps that `npm:@neralabs/rerail` entry for the managed install, `~/.rerail/current`,
in the same place. Pi then loads one copy, and `rerail update` governs the version, not `pi update`. A gallery install
made after set-up is swapped the same way, by `/rerail-start` run again or by the next `rerail update`.

### From OMP's plugins

Every coding agent gets the same commands: `/rerail`, `/rerail-manage`,
`/rerail-create`, `/rerail-start`, `/rerail-stop`, `/rerail-update`,
`/rerail-uninstall`, `/rerail-help` and `/rerail-show-me` (`$rerail`,
`$rerail-manage`, and so on in Codex). `rerail install` also removes the
`rerail-ui` and `rerail-rage-quit` files an earlier build wrote. `rerail update`
then runs `rerail install --refresh` from the version it activated: every coding
agent already connected gets that version's commands, skills and hooks, and loses
the ones it retired; a coding agent never connected is left alone. Command files
and skills Rerail generated are replaced without a `.bak` copy. Updating from
0.1.0-next.2 or earlier runs that version's update, which does not do this: run
`rerail install --refresh` once afterwards.
Settings, scans, status, update checks, rollback and reset are the CLI
commands above.
Restart the coding agent after installing or rebuilding to load the commands.
A scan or settings invocation must supply the exact session
identity. See the [scan and settings contract](../README.md#scan-and-settings-controls)
for scope, mode, persistence, diagnostics, and background versus next-event
delivery.

Updates only happen when you ask for one. Nothing in Rerail polls the registry
or installs a version in the background; `rerail update` and re-running
`npx @neralabs/rerail setup` are the only paths. `rerail update --channel <tag>`
selects the npm dist-tag (`latest`, `beta`, or `next`).

Rerail keeps one database, `~/.rerail/rerail.sqlite` (`$RERAIL_HOME/rerail.sqlite`),
holding each repository's rule configuration, the rules you created, and the
sessions, events, and work links of every repository on the machine; each
session is attributed to its repository (`rerail status` names the file). The
daemon serves every repository; its pages are scoped to one with
`?repository=<key>&checkout=<path>`. The rules of the rule packs you install
(`$RERAIL_HOME/packs`, see [Rule packs](rule-packs.md#installing-packs)) run only
in repositories that add them (see [Rule scoping](rule-scoping.md)).
`rerail update` first copies the database to `~/.rerail/rerail.sqlite.pre-update`
(kept only when the update changes the version, and recorded with that change), and
`rerail update rollback` puts that copy back when rolling that change back and the
update changed the database in a way the older version cannot read (see
[Database compatibility](#database-compatibility));
`rerail rules export [--output <file>]` and `rerail rules import <file>` move
your rules and each repository's configuration by hand. `rerail seed` fills the
database with mock rules to work against.

Policy evaluation caches parsed files and results in a content store. Which
one a Rerail uses depends on where its package lives, not on a flag: the
managed installation (a package under `~/.rerail/versions/<version>/`, or
`$RERAIL_HOME/versions/`, normally loaded through `current`) owns the
machine-wide `~/.cache/rerail/content-v3.sqlite`. Any build running from a
Git checkout — source, `dist/`, tests, benchmarks, every worktree — uses
`<checkout>/.rerail/content-v3.sqlite` instead (`.rerail/` is gitignored), so a
checkout on a newer store schema never changes the store the installed version
reads. The file name follows the store's compatibility floor (`v2` today) and
changes whenever the floor rises. `RERAIL_CONTENT_STORE_PATH` overrides the
location for either. A package that is neither (for example an `npx` copy) has
no store and reports that on its first scan.

### Starting and stopping Rerail

Rerail is off in every repository until it is started there. Rules belong to
the repository; on and off belong to each checkout, stored in
`~/.rerail/rerail.sqlite`. A checkout without its own value follows the nearest
checkout of the same repository that contains it and has one, and otherwise
the repository. Two clones of one remote are one repository.

- `/rerail-start` (OMP, Pi, Claude Code, Cursor) or `rerail start` (terminal,
  and the only spelling in Codex, which cannot expose an exact third-party
  slash-command name) starts Rerail in the current checkout and the checkouts
  inside it. In a repository that is not set up, it runs set-up's steps 2 to 6
  there first: it asks, recommends rule packs, and starts once you confirm
  ("Skip, I’ll pick packs" lists every rule pack set-up fetched, installs
  the ones you pick and adds each for the whole repository, without reading
  it; "None for now" sets it up with none). Until a repository is set up, its sessions are
  recorded but no rules run.
- `/rerail-stop` or `rerail stop` stops Rerail in the current checkout and the
  checkouts inside it: no recording, evaluation, steering, or daemon autostart
  from their sessions. Recorded history, rules, and the shared daemon stay,
  and other repositories are unaffected.
- `rerail start --repository` and `rerail stop --repository` act on every
  checkout of the repository, clearing their own values.

`rerail daemon start` and `rerail daemon stop` control the background daemon
itself, which serves every started repository on the machine.

### One main instance

At most one main Rerail instance runs on a machine: the managed
installation's daemon, the only process that opens the machine-wide content
store. It holds an exclusive OS lock (`flock` / `LockFileEx`) on
`~/.cache/rerail/main-instance.lock` for its whole life and records its
pid, version, package root, and start time beside it in `main-instance.json`.
The OS drops the lock however the daemon ends, `kill -9` included, so there is
never a stale lock to clear. A second main daemon — another `$RERAIL_HOME`, or
an orphan its `daemon.json` no longer names — exits at startup with
`Another main Rerail instance is running: pid …, version …`, which `rerail daemon start`
prints from `daemon.log`; stop that process (`rerail daemon stop`, or end it by pid)
and start again.

Everything else in the managed installation goes through that daemon rather
than evaluating in-process: the Claude Code, Codex, and Cursor hooks, OMP and
Pi sessions, and `rerail events` streams (in internal testing) send their events
to it, starting it when it is not running (unless `RERAIL_AUTOSTART=0`), and CLI
commands such as `rerail settings` and `rerail scan` (in internal testing) use its API. Any process other than the lock holder that tries to
open the machine-wide store gets an error and a failed policy run instead of a
second writer. Development builds never take the lock and never open that
store.

Updates hand over without overlap: `rerail update` only activates the new
version, and the next hook or session from it stops the old daemon, waits
until that process has exited and the OS has released its lock, then starts
the new daemon, which takes the lock. `rerail setup` replaces a stale daemon
the same way. A `rerail events` stream (in internal testing) that outlives an update keeps sending to
whichever daemon is running and never brings its own version back.

### Database compatibility

`rerail.sqlite` records its compatibility floor in `PRAGMA user_version`: the
oldest schema version a Rerail must read to open it, 3 in this release. Which
migration steps the file has run is recorded in its `schema_migrations` table.

When a newer Rerail first opens the file (the daemon, `rerail start`, set-up,
or a command, hook or status line that only reads), it runs the steps the file
has not run, in one transaction: a step that fails leaves the file as it was.
A process that only reads and cannot write the file (its permissions) gets an
error naming those steps instead, and the file, its mode included, is left as
it was.
Most steps add a column or a table, which older versions read correctly, so
they keep the floor and an older Rerail still opens the file; a hook or the
status line runs them before reading. A step an older version would misread
raises the floor and waits for the new version's daemon or a command that
writes (until then a hook or the status line says to start Rerail, `rerail
daemon start`); from then on older versions refuse the file, and `rerail update
rollback` puts back the copy `rerail update` took before that version change
when the floor is now above what the version rolled back to opens. It writes the
copy into the database file itself, so anything that has the file open goes on
with the restored database, after keeping what the database held as
`rerail.sqlite.before-rollback` (readable only by you; a later rollback keeps its
own as `.before-rollback.2`, `.3`, …, never over an earlier one). A copy's
record is used once: once the copy is put back, or you are told how to put it
back yourself, no later rollback puts it back again. While it writes,
`rerail.sqlite.restoring` (holding the rollback's process id) sits beside the
database and every Rerail process refuses to open it; try again in a moment. A
rollback that was killed leaves the file behind, but once its process is gone
Rerail opens the database again and the next rollback takes the file over. While
a coding agent session still has the database open, or another rollback is
restoring, the copy is not put back: copy it over yourself, as the message says
(it also says when it stopped the daemon). An update that finds nothing
newer, or fails, keeps the copy of the last change; one that changes the version
but cannot keep a copy removes the last one, which no longer belongs to the
change a rollback would undo.

A database written by 0.1.0-next.1 (stamped 1, before the migration path) is
adopted: the first Rerail process that opens it records the baseline and stamps
it 3, keeping everything in it. 0.1.0-next.1 then no longer opens it, and its
`rerail update` kept a copy with no record of the change, so `rerail update
rollback` to it does not put the copy back itself: it prints how to (copy
`rerail.sqlite.pre-update` over `rerail.sqlite` with Rerail stopped, keeping
the current file if you need what changed since). A database file written by a newer Rerail (a
floor above this release's), by other software, by 0.1.0-next.0 (stamped 1, in
an older shape) or by a development build (stamped 2) is refused with an error
and left untouched. Move such a file aside (or point `RERAIL_HOME` elsewhere)
and start again, then re-import rules with `rerail rules import`.

### Resetting session history

To clear a repository's recorded sessions, run `rerail reset` from any of its
checkouts (safe to repeat).

It deletes that repository's ended sessions and every session's events, work
links, audits, and audit requests in one transaction. Rules, settings, and
other repositories are untouched; the file is never unlinked, so open daemons
and harness sessions keep working, and running sessions keep recording.

### Removing Rerail

To take Rerail off a machine:

```sh
rerail update check   # is a newer version on the channel?
rerail update         # install it; /rerail-update in a coding agent ($rerail-update in Codex)
```

Running `npx @neralabs/rerail setup` again updates too. `rerail update --channel <tag>` selects the npm dist-tag
(`latest`, `beta` or `next`). An update first copies the database to `~/.rerail/rerail.sqlite.pre-update`, then
activates the new version for coding agent sessions started after it, and gives the coding agents already connected
its commands. See [Updates](how-setup-works.md#updates) for the details.

## Start and stop

Rerail is off in every repository until it is started there. Stopping keeps your rules and history.

- `/rerail-start` or `rerail start` (`rerail start` in Codex) starts it in this checkout. In a repository that is not
  set up, it asks set-up's questions first.
- `/rerail-stop` or `rerail stop` stops it in this checkout: nothing is recorded or checked there.
- `--repository` on either acts on every checkout of the repository.
- `rerail daemon start` and `rerail daemon stop` control the one background daemon that serves every repository.

How checkouts and worktrees inherit this is in [Rule scoping](rule-scoping.md#start-and-stop).

## Remove

```sh
rerail rules export --output ~/rerail-rules.json   # optional: a portable copy of your rules
rerail install --remove --dry-run                  # print what would change
rerail install --remove                            # or /rerail-uninstall in a coding agent
```

`rerail install --remove` takes Rerail out of every coding agent and off `PATH`, leaving other tools' entries alone and
backing up each file it changes. It stops the daemon, moves your database into `~/.rerail-clean-YYYYMMDD/` so your
rules and history are kept, and deletes the installation: `~/.rerail/current`, the launcher, every version, the rule
packs and the cache `~/.cache/rerail`, naming each path. Restart open coding agent sessions afterwards.
`rerail install --remove --delete-data` deletes the database, its backups, the cache and the checkouts' session
ledgers instead of keeping them; nothing it deletes can be restored. See
[Removing Rerail in detail](how-setup-works.md#removing-rerail-in-detail).

## Supported and not supported

Supported:

- **Operating systems**: macOS on arm64 and x64 (x64 Node under Rosetta works too); Linux on arm64 and x64 with
  glibc 2.28 or newer, which every distribution Node 24 runs on has (Debian 10 and later, Ubuntu 20.04 and later,
  RHEL 8 and later and their rebuilds, Amazon Linux 2023, Fedora); Windows 11 on x64 and arm64, profile folders whose
  name has a space or a non-ASCII letter included.
- **Node.js** 24 or newer, however it is installed (nodejs.org, Homebrew, nvm, fnm, Volta, NodeSource).
- **Git** on `PATH`, or in the usual install locations when `PATH` has none.
- **Shells** set-up puts `rerail` on `PATH` for: zsh, bash and fish (interactive shells; `zsh -c` and ssh need the
  full path or `~/.zshenv`), any other shell through `~/.profile`, and on Windows PowerShell, Command Prompt and Git
  Bash through the user `Path`.
- **Coding agents**: Claude Code as the CLI, in the VS Code extension (whose panel shows no status line) and in the
  JetBrains plugin; on Windows, Claude Code 2.1.139 or newer. Codex CLI (trust the hook once with `/hooks`). Cursor's
  IDE agent and its CLI (`cursor-agent`). OMP and Pi. Each is found by its config folder in your home folder
  (`~/.claude`, `~/.codex`, `~/.cursor`, `~/.omp/agent`, `~/.pi/agent`; on Windows the profile folder, `USERPROFILE`),
  and one installed after set-up is connected by running `rerail setup` again.
- **Containers and devcontainers** with a glibc image, the default `node:24` and `node:24-slim` included, with or
  without an init process.
- **WSL 2**, as Linux, with the coding agent running in WSL too: install Rerail on the same side as the coding agent. A
  repository on the Windows drive (`/mnt/c/…`) works but is slow from WSL; set-up says so.
- **A registry mirror or proxy** set in `~/.npmrc`.

Not supported:

- Linux with glibc older than 2.28 (Amazon Linux 2, Ubuntu 18.04, CentOS 7), where Node 24 does not run either.
  Alpine and other musl distributions (`node:24-alpine`). Other operating systems and processors.
- Node.js 23 or older.
- No Git, or a Git only inside the repository or a `node_modules` folder.
- Other coding agents and editors: GitHub Copilot, Windsurf, Gemini CLI, Cline, Roo Code, Zed's agent, Aider,
  Continue, OpenCode, Amp and the rest. Cursor's cloud agents, which run no user hooks.
- Agent config folders moved with `CLAUDE_CONFIG_DIR` or `CODEX_HOME`. An agent config file that is not strict JSON
  (comments, trailing commas) is left as it is, and that coding agent stays unconnected until the file is fixed.
- A coding agent on Windows with Rerail in WSL, or the other way round.
- Installing offline: set-up, `rerail update` and rule packs need the npm registry.

## What a coding agent can reach

A coding agent runs commands as you, so only its own sandbox can keep it out of Rerail's data: `~/.rerail` (the
dashboard's access token, your rules and history) and `~/.cache/rerail`. Set-up adds denies for both where the agent's
settings have a place for them, and never turns a sandbox on or off. Claude Code's file tools are always kept out, its
commands only with its sandbox on (`/sandbox`); the Cursor CLI's file tools are kept out. Codex's and Cursor's default
sandboxes stop writes but not reads, and OMP and Pi have no sandbox.
[Sandbox and permission entries](how-setup-works.md#sandbox-and-permission-entries) lists every entry.

Settings → Coding agents in the dashboard and `rerail status` show, for each connected coding agent, whether its
tools can read Rerail's data on this computer, from its settings as they are now (for Claude Code, whether
`sandbox.enabled` is on in your user settings or in managed settings).

## Registry access

Rerail is published only to npmjs, as a public package, so no account or sign-in is needed. Set-up and
`rerail update` use your normal npm configuration, so a mirror or proxy in `~/.npmrc` works; a repository's own
`.npmrc` does not apply to updates. [Updates](how-setup-works.md#updates) covers expired tokens and signature checks.
