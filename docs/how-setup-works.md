# How set-up works

What `rerail setup` does step by step, what it writes, and how the managed installation, updates, the database and
removal work underneath. [Installing Rerail](installation.md) is the short guide; this page is for when you need the
details. [Troubleshooting](troubleshooting.md) answers what goes wrong.

## The managed installation

`npx` fetches the package into npm's cache and runs its `rerail` binary from
there. That copy is ephemeral, so `setup`'s first job is to make a durable one:

```
~/.rerail/
├── versions/<version>/node_modules/@neralabs/rerail/   the installed package
├── current  →  versions/<version>/node_modules/@neralabs/rerail
├── bin/rerail                                          launcher shim → current/dist/cli/cli.js
├── update-state.json                                  active + previous version, channel
├── daemon.json / daemon.log                           the running daemon
├── daemon-port.json                                   the port the daemon comes back on after a restart
├── coding-agents.json                                 the coding agents set-up and `rerail install` have seen
├── packs/node_modules/@neralabs/rerail-pack-<id>/     Rerail's rule packs you installed (`rerail packs`)
├── packs/node_modules/[@<scope>/]rerail-pack-<id>/    third parties' rule packs you installed
└── versions/<older>/…                                 the versions `rerail update` replaced
```

## Set-up step by step

Set-up follows the same steps in a terminal and from a coding agent. Nothing
in a repository, its Git metadata, or the database is written before the
person says yes.

0. **Checks** — on a Node major below 24, on a computer Rerail has no engine
   for (musl Linux such as Alpine, glibc older than 2.28, another system or
   processor), or without Git, set-up stops before changing anything and says
   what to do. Which Git and Node set-up looks for is under
   [Which Git and Node Rerail runs](#which-git-and-node-rerail-runs).
1. **Install** — set-up runs `npm install @neralabs/rerail@<this version>`
   into `versions/<version>`, a staging folder until it passes validation:
   the package identity, the native kernel loading, and a real event-stream
   round-trip. It then re-points `current` atomically and (re)writes
   `bin/rerail`; the npx copy is never used again. Installing turns nothing
   on, connects no coding agent and starts no daemon. Set-up skips this step
   when it runs from the version `current` already points at. Once any
   version is installed, an answer run (`--answer`, `--repo`) and
   `/rerail-start` skip it too: OMP and Pi run those from the session's own
   package, which may be older than `current`.
   - **May Rerail read your sessions** — once per computer, before set-up
     reads a repository or connects a coding agent, it asks whether Rerail may
     read the sessions of the coding agents it connects to (what you ask, what
     they do, the files they edit), to check them against your rules and show
     their events in the dashboard. "Yes" (`--answer allow`) is kept in
     `~/.rerail/consent.json`, with the folder the run was for (`--repo`), and
     never asked again; set-up still asks before reading that folder. "No"
     (`--answer decline`) stops set-up with nothing new connected or started,
     and removes an earlier yes so the next set-up asks again. Coding agents
     connected before stay connected until `rerail install --remove`. The
     install above has already happened, so Rerail stays installed and off,
     with `rerail` on `PATH`, and the screen says `rerail install --remove`
     takes it off the computer. `rerail start` in a repository that is not set
     up asks this too; an update's health check does not.
2. **Ask before reading** — set-up names the Git repository it ran in and asks
   whether it may read it. Outside a repository (or in your home folder) it
   asks which folder to start with instead (type the folder, or skip for now),
   numbered as this step; `--repo <folder>` names one, which is the go-ahead
   to read it.
3. **Recommend** — the repository's files and `package.json` dependencies are
   matched against the rule packs; each pack that fits is shown with its rule
   count, why it matched and the folder it would be added in, and the coding
   agents it will connect ("Will connect to: …"; `--agents` names others). A
   repository that matches no rule pack is offered none; it can still start,
   and add rule packs in the dashboard later. The screen also lists the rules
   of those packs that will start off or with options taken from the
   repository's own lint config (see
   [The repository's own lint config](rule-scoping.md#the-repositorys-own-lint-config)).
4. **Confirm** — "Yes, start Rerail" (`--answer start`) installs the packs
   shown and starts; "Change rule packs…" (`--answer choose`) picks from every
   pack set-up fetched instead; "Not now" (`--answer not-now`) installs,
   connects and turns on nothing. Picking rule packs yourself without a scan
   is the question before ("Skip, I'll pick packs", `--answer pick-myself`:
   no code is read). See [Rule scoping](rule-scoping.md#set-up) for how each
   answer attaches packs.
5. **Start, in this repository** — set-up connects the chosen coding agents (all it found, unless `--agents` names
   others), attaches the recommended or chosen packs to the repository, and starts it: every checkout of it that has
   not been stopped on its own, the one set-up ran in included. Other repositories stay off. Every registration points
   at `~/.rerail/current`, so hooks follow an update without being rewritten; `rerail update` registers again only
   with the coding agents already connected, to bring the new version's commands and skills. Per coding agent:
   - OMP: the link `~/.omp/agent/extensions/rerail` → `current`, and a `rerail` MCP server in `~/.omp/agent/mcp.json`.
     An OMP plugin install of Rerail is left alone; the summary says to remove it.
   - Pi: a `packages` entry in `~/.pi/agent/settings.json`, which takes the place of any gallery
     `npm:@neralabs/rerail` entry, keeping its filters.
   - Claude Code: hooks, the status line (wrapping any you had) and the
     [sandbox and permission denies](#sandbox-and-permission-entries) in `~/.claude/settings.json`, and the Rerail
     commands in `~/.claude/commands/`. On Windows the hooks run without Git Bash.
   - Codex: hooks in `~/.codex/hooks.json` (trusted once with `/hooks` in Codex), and one skill per Rerail command in
     `~/.codex/skills/<name>/SKILL.md`, invoked as `$rerail`, `$rerail-manage` and so on.
   - Cursor: hooks in `~/.cursor/hooks.json`, the Rerail commands in `~/.cursor/commands/`, and permission denies in
     `~/.cursor/cli-config.json` when the Cursor CLI has written that file.

   Other tools' entries are kept as they are, though the file is written back as JSON indented by two spaces.
   Rerail's own entries, from this install, another install of this package, or one whose install is gone, are
   replaced, not duplicated. A config that is not plain JSON (a comment, a trailing comma) is left alone: set-up
   connects the other coding agents and starts the repository, and its summary names the file, the reason and what
   to do. A `<file>.<timestamp>.bak` is written only when a file changes. Re-running is a no-op.
6. **Summary** — the rule packs added (by name, "TSLint in web" when added in
   one folder), the rules on, the other repositories Rerail is on in, the
   dashboard link (`#rules`, its Overview, scoped to the repository and
   checkout), and each coding agent that could not be connected. Commands are
   shown as `$rerail-show-me` when Codex is the only coding agent connected,
   with Codex's form beside the others' when it is one of several, and after
   Codex's hooks are written a line says to trust them once with `/hooks` in
   Codex. With no coding agent found it says so, names the ones Rerail works
   with, and says to run `rerail setup` again once one is installed. The daemon used is the active install's, which
   serves every repository; one from another version or on another port is
   replaced (`--port` picks the port). The browser opens unless
   `BROWSER=none`, the session is over SSH, or Linux has no display.

### Sandbox and permission entries

Rerail's data sits in two folders: the Rerail home (`~/.rerail`, or `RERAIL_HOME`), with the access token, the
database of every repository's history and the installed package, and the content cache (`~/.cache/rerail`). A coding
agent that can run commands runs them as you, so only the agent's own sandbox can keep it out of these. Set-up adds
denies for both folders where the agent's user settings have a documented place for them. It never turns a sandbox on
or off, and changes no other sandbox or permission setting. A Rerail home that is your home folder, or a folder above
it, is left out, since denying it would deny every project; the cache is still denied. `rerail install --remove` takes
out exactly what it added; an entry you had already stays. Running set-up again with another `RERAIL_HOME` moves the
denies to it.

Set-up writes, per coding agent:

- **Claude Code** (`~/.claude/settings.json`):
  - The sandbox's `filesystem.denyRead` and `filesystem.denyWrite` name both folders. `filesystem.allowRead` opens
    `current` and `versions` in the Rerail home again, so commands can read the installed package but no data.
  - `permissions.deny` covers the file tools, which the sandbox does not: `Edit(...)` for both folders, `Read(...)`
    for the whole cache and for the Rerail home's data files (`*.json*`, `*.sqlite*`, `*.log*`, `*.lock`,
    `access-token*`, `active-profile*`, `device-secrets/`, `profiles/`). The installed package, `bin/` and `packs/`
    stay readable: `/rerail-create` reads its skill there, and a deny cannot be reopened for one folder.
  - `sandbox.excludedCommands` holds Rerail's CLI as the Rerail commands run it (`<node> <root>/dist/cli/cli.js *`),
    the launcher (`~/.rerail/bin/rerail *`) and `rerail *`. These run outside the sandbox and go through your normal
    permission rules there: what a Rerail command pre-approves runs, and anything else asks you. A Rerail command
    with a redirect, a `cd` or a `$(...)` stays sandboxed and cannot read Rerail's files.
  - Hooks, the status line and MCP servers run outside the sandbox, so Rerail's hooks are not affected.
  - The sandbox is off unless you turn it on (`/sandbox`). With it off, only the file-tool denies apply, and the
    agent's shell commands can read and change Rerail's data. On native Windows Claude Code has no sandbox, and set-up
    writes only the file-tool denies.
- **Codex**: no stable user setting denies a path. Permission profiles can, but they are in beta and replace the
  `sandbox_mode` settings, so set-up does not write them. The default `workspace-write` sandbox lets commands write
  only to the workspace and temporary folders and turns network access off, so a sandboxed command cannot change
  Rerail's data. It can still read it, the access token included.
- **Cursor**: `sandbox.json` can only add paths, and the editor's agent has no path deny, so the editor gets no
  entries. By default its sandboxed commands write only to the workspace and temporary folders, and loopback
  addresses are blocked, but they can read Rerail's data. "Read Access: Workspace" in Cursor's settings makes the
  agent ask before it reads outside the workspace; it applies to every path, so set-up does not set it. The Cursor
  CLI (`cursor-agent`) gets `Write(...)` denies for both folders and the same `Read(...)` denies as Claude Code in
  `~/.cursor/cli-config.json`, for its file tools only. Set-up adds them only when the CLI has written that file, and
  not on Windows.
- **OMP and Pi** have no sandbox. Their agents' commands and tools can read and change all of Rerail's data.

## Running set-up again

"Skip for now" (no repository) connects the coding agents found and starts
Rerail nowhere. Set-up run again in a repository that is set up never asks
again: it connects the coding agents found since (named in the summary as "Now
also connected"), then shows the summary, or says Rerail is stopped there.
`~/.rerail/coding-agents.json` lists every coding agent set-up or `rerail
install` has seen, except one that could not be connected, so the next set-up
tries it again; without the file every coding agent found counts as new.
Rerail stays off in every other repository until you start it there (see
[Start and stop](installation.md#start-and-stop)); starting a repository that
is not set up runs steps 2 to 6 for it.

## `rerail` on your PATH

Every set-up run also puts `~/.rerail/bin` (the Windows launcher is
`rerail.cmd`, with an sh launcher `rerail` beside it for Git Bash, which runs
neither `.cmd` nor `.bat` files by bare name) on `PATH`, so `rerail` runs
from a new terminal. Set-up records that it handled `PATH` in
`~/.rerail/launcher-path.json`, and from then on `PATH` is yours: a block you
take out stays out, and only `rerail setup` run again adds it back.
`rerail update` adds the entry only when that record is missing. On macOS and
Linux it appends a block between `# >>> rerail >>>` and `# <<< rerail <<<` to
the startup file `$SHELL` (else the account's login shell) reads: `$ZDOTDIR/.zshrc` (else `~/.zshrc`) for zsh;
`~/.bashrc` and the first of `~/.bash_profile`, `~/.bash_login`,
`~/.profile` that exists (else `~/.profile`) for bash; its own
`~/.config/fish/conf.d/rerail.fish` for fish; `~/.profile` otherwise. The block
adds the directory only when `PATH` lacks it, so a file sourced twice adds it
once. On Windows it appends the directory to the user `Path`
(`HKCU\Environment`), keeping that value's registry kind. Set-up says what it
changed, and changes nothing when the running `PATH` has the directory
already, when the entry is in place, or with `RERAIL_MODIFY_PATH=0`; a block
written for another Rerail home is replaced. A startup file it changes keeps
one backup beside it, `<file>.rerail.bak`: the file as it was before Rerail
last changed it. Each change replaces that backup, and `rerail install
--remove` deletes it once it has taken the block out.

The block is in the file an interactive shell reads, so a shell that is not
interactive misses it: `zsh -c`, `ssh host rerail …` to a zsh account, and a
script. There, call `~/.rerail/bin/rerail` by its full path, or add the
directory to `PATH` yourself in a file every such shell reads (`~/.zshenv`
for zsh). Rerail does not write `~/.zshenv`: every zsh reads it, scripts
included, and on macOS `/etc/zprofile` reorders `PATH` after it. Coding agents
run Rerail by its full path, so they need neither.

## How set-up asks

**In a terminal** (stdin and stdout are TTYs, no `--relay`), each question is
a menu: ↑/↓ (or k/j) move the `❯` from answer to answer, wrapping around, a
digit moves it to that answer, and Enter takes the answer it is on, so typing
`2` then Enter answers as before. Ctrl-C puts the terminal back and stops
set-up; Ctrl-D cancels it. The folder and rule-pack prompts read a typed line.

**Without a terminal** (stdin or stdout is not a TTY, as when a coding agent
runs it), or with `--relay`, each run prints the next question for the coding
agent to relay, as written, with one command per answer, and exits 0. The
coding agent runs set-up again with the person's answer: `--answer <allow|decline|yes|other-folder|pick-myself|start|choose|not-now|folder|skip>`,
`--repo <folder>`, `--agents <ids>` (`omp,pi,claude-code,codex,cursor`; it
works on its own, with any answer, and no question asks for it) and
`--packs <id@version,…>` (the exact versions an answer installs, a third
party's pack by its package name, `@acme/rerail-pack-sql@1.0.0`; `none` for
none).
Each answer run starts from what it is told, so running one twice repeats no
earlier step's side effect. Claude Code's, Cursor's and Codex's `/rerail-start`
runs `rerail start --relay`, and every answer command carries `--relay`, so a
coding agent that runs commands in a terminal still gets these questions
instead of a prompt waiting on input.

**In a coding agent's own UI**, `--json` on `rerail setup` and on `rerail
start` prints one JSON object per run and nothing else on stdout:
`{ step, screen, question }`. `step` is
`sessions`, `read-repo`, `confirm-packs`, `pick-packs` or `no-repo` at a question, else `done`,
`stopped`, or `error` (exit 1); `screen` is plain text for the person; each of
the question's options carries its answer key, label, `detail`, the arguments
that give it (`next.args`, starting with `setup` and carrying `--json`), and,
where the answer needs a folder or coding agent ids, the value to ask for first
(`input`: append `[flag, value]`). `rerail start --json` in the home folder
asks which project to start with. The
side effects are the agent lane's. `--database <file>` names the database an
extension was configured with.

In OMP and Pi, `/rerail-start` always runs this picker through the launcher's
`rerail start --json`: each question's text shows as a notice, then the
question as a select (a text box for a folder or coding agent ids), and the
summary shows when set-up ends. In a repository that is set up the first step
just starts Rerail and says so. Closing a picker stops set-up and turns
nothing on. Without a UI (print mode) it shows `rerail start`'s text. Claude
Code, Codex and Cursor relay the questions as text.

`rerail install` (see [Commands after set-up](#commands-after-set-up)) still connects coding agents on its own, with
`--agents <list>`, `--root` and `--dry-run`.

## Which Git and Node Rerail runs

Rerail reads every repository through Git. It runs the first `git` it finds on `PATH` (`git.exe` on Windows),
skipping entries that are relative, inside the repository Rerail runs in (the nearest folder above it with a `.git`),
or inside a `node_modules` folder, so a repository cannot put its own `git` first. When `PATH` has none, it falls back to the usual
install locations: Homebrew, Linuxbrew and Nix on macOS and Linux, Git for Windows and scoop on Windows. On macOS the
`/usr/bin/git` stub counts only with the Command Line Tools or Xcode installed.

Set-up records the Node that ran it for the launcher, the hooks and the Claude Code status line, Homebrew's by the
link `brew upgrade` keeps (`/opt/homebrew/bin/node`, not its versioned `Cellar` folder). Each falls back to `node` on
`PATH` once the recorded Node is gone (after `nvm uninstall`, say), except the status line on Windows. On a Node older
than 24 the CLI and the daemon stop with set-up's message, the hooks do nothing, and the status line says
`needs Node 24 or newer`. After moving to another Node, run `rerail install --refresh` with it first on `PATH` to
record it for all of them.

Set-up checks Node, the platform and Git before it installs anything. When one is missing it stops, says what to do,
and changes nothing; an install that fails later removes the empty folders it made.

## Commands after set-up

The following commands describe the managed installation, from a terminal
opened after set-up put `~/.rerail/bin` on `PATH`:

```sh
rerail start             # start Rerail in this checkout (first time: runs set-up's questions for the repository)
rerail stop              # stop Rerail in this checkout; the daemon keeps serving others
rerail daemon start      # start the background daemon (--port, --host, --open, --dev, ...)
rerail daemon stop       # stop the background daemon
rerail status            # whether Rerail is started here, otherwise daemon/database/kernel
rerail version           # the installed version
rerail manage            # opens the dashboard's Overview (--print only prints the URL)
rerail session claude-code ID --open  # this session's status line and page URL; --open opens it
rerail scan --session ID --scope repository --wait  # internal testing: inspect findings in Sessions
rerail settings --session ID --mode log             # save this workspace's default
rerail update check      # is a newer version on the channel?
rerail update            # install it
rerail reset             # delete this repository's session history, across all worktrees; rules stay
rerail scrub             # redact secrets from history earlier versions stored (database, ledgers, held steers, daemon.log)
rerail rules approve     # list the rules and packs that run in Log, waiting for your approval
rerail rules approve ID  # print a waiting rule's content and approve it so it steers
rerail rules export      # dump your rules and every repository's configuration to ~/.rerail/rules.json
rerail support-bundle    # write a file of diagnostics to attach to an issue, to ~/rerail-support-bundle.json
rerail install --remove  # take Rerail out of every coding agent, stop the daemon, back up the database, delete the install
rerail install --remove --delete-data  # the same, deleting the database, backups, cache and checkouts' session ledgers instead
```

Every coding agent gets the same commands: `/rerail`, `/rerail-manage`,
`/rerail-create`, `/rerail-start`, `/rerail-stop`, `/rerail-update`,
`/rerail-uninstall`, `/rerail-help`, `/rerail-show-me` and `/rerail-bug-report` (`$rerail`,
`$rerail-manage`, and so on in Codex). In Claude Code and Cursor, `/rerail-manage`
opens the dashboard as a tab in the Orca or Cursor window it runs in, else in
the system browser. After `rerail update` activates a version, it runs that
version's `rerail install --refresh`: every coding agent already connected
gets its commands, skills and hooks and loses the ones it retired; a coding
agent never connected is left alone. Command files and skills Rerail generated
are replaced without a `.bak` copy.
Restart the coding agent after installing or rebuilding to load the commands.
A scan or settings invocation must supply the exact session
identity. See the [scan and settings contract](reference.md#scan-and-settings-controls)
for scope, mode, persistence, diagnostics, and background versus next-event
delivery.

## Updates

Updates only happen when you ask for one. Nothing in Rerail polls the registry
or installs a version in the background; `rerail update` and re-running
`npx @neralabs/rerail setup` are the only paths. `rerail update --channel <tag>`
selects the npm dist-tag (`latest`, `beta`, or `next`).

npmjs (`https://registry.npmjs.org/`) is the only registry `@neralabs/rerail` is published to. Rerail adds no registry
configuration of its own: `setup` and `rerail update` run the `npm` next to the Node that runs them, with your normal
npm configuration. When a token your npm configuration holds for npmjs is invalid or has expired, npm-backed commands
such as `rerail update` report that the npm registry credentials are missing or expired; sign in again with
`npm login`, or remove the token. Rerail redacts `_authToken` and `_password` values from any npm output it prints.

`rerail update` asks the registry for the latest version from an empty temporary folder, so a repository's own
`.npmrc` cannot choose the registry; your `~/.npmrc` still applies. It installs without running any package's install
scripts, then checks the registry signatures and provenance attestations of everything it installed with
`npm audit signatures` before running any of it. A signature that does not verify stops the update, and nothing is
installed. A registry that publishes no signing keys, as some private mirrors do, gets a warning naming it, and the
update goes on.

## The database

Rerail keeps one database for every repository on the computer, `~/.rerail/rerail.sqlite`
(`$RERAIL_HOME/rerail.sqlite`); `rerail status` names the file. [Rerail database](reference.md#rerail-database)
says what it holds and what `rerail reset` and `rerail scrub` do to it, and
[Deleting your data](privacy.md#deleting-your-data) how to remove it. `rerail update` first copies it to
`rerail.sqlite.pre-update` beside it, kept only when the update changes the version: the database as it was before
the version now active, to put back by hand (see [Database compatibility](#database-compatibility)).

## The content store

Policy evaluation caches parsed files and results in a content store. Which
one a Rerail uses depends on where its package lives, not on a flag: the
managed installation (a package under `~/.rerail/versions/<version>/`, or
`$RERAIL_HOME/versions/`, normally loaded through `current`) owns the
machine-wide `~/.cache/rerail/content-v1.sqlite`. Any build running from a
Git checkout — source, `dist/`, tests, benchmarks, every worktree — uses
`<checkout>/.rerail/content-v1.sqlite` instead (`.rerail/` is gitignored), so a
checkout on a newer store schema never changes the store the installed version
reads. The file name follows the store's compatibility floor (`v1` today) and
changes whenever the floor rises. `RERAIL_CONTENT_STORE_PATH` overrides the
location for either. A package that is neither (for example an `npx` copy) has
no store and reports that on its first scan.

## One main instance

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
to it, starting it when it is not running, after a reboot too (unless `RERAIL_AUTOSTART=0`), and CLI
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

## Database compatibility

`rerail.sqlite` records its compatibility floor in `PRAGMA user_version`: the oldest schema version a Rerail must be
able to read to open it.

When a newer Rerail first opens the file (the daemon, `rerail start`, set-up, or a command, hook or status line that
only reads), it runs the migration steps the file has not run, in one transaction: a step that fails leaves the file
as it was. A process that cannot write the file gets an error naming those steps, and the file is left as it was.

Most steps add a column or a table, which older versions read correctly, so the floor stays and an older Rerail still
opens the file; a hook or the status line runs such steps itself. A step an older version would misread raises the
floor, and waits for the new version's daemon or a command that writes; until then a hook or the status line says to
run `rerail daemon start`. From then on older versions refuse the file. To run an older version against it anyway,
put back the copy `rerail update` took before that version change:

1. Stop Rerail (`rerail daemon stop`) and close the coding agent sessions that have it open.
2. Keep `rerail.sqlite` elsewhere if you need what changed since the update.
3. Copy `rerail.sqlite.pre-update` over `rerail.sqlite`.

An update that finds nothing newer, or fails, keeps the copy from the last version change. One that changes the
version but cannot take a copy removes the old one, which no longer holds the database before the version now active.

A file stamped by a newer Rerail, or written by other software, is refused with an error and left untouched. Move it
aside (or point `RERAIL_HOME` elsewhere), start again, then re-import your rules with `rerail rules import`.

## Removing Rerail in detail

To take Rerail off a machine:

```sh
rerail rules export --output ~/rerail-rules.json   # optional: a portable copy of your rules
rerail install --remove --dry-run                  # print what would change
rerail install --remove                            # or: --remove --delete-data, keeping nothing
```

`rerail install --remove` takes out every registration that names this
installation — harness hooks, the Claude Code status line and commands, the
sandbox and permission denies set-up added to `~/.claude/settings.json` and
`~/.cursor/cli-config.json`, the Cursor commands, the Codex skill, the OMP
extension symlink and MCP entry, and the Pi package entry (an
`npm:@neralabs/rerail` one too, when it removes the managed install) — and leaves other tools' entries alone, writing a
`<file>.<timestamp>.bak` beside every file it changes. A folder that held only
what it removed goes too: each `~/.codex/skills/rerail*` (and `~/.codex/skills`
when those were all it held), and `~/.omp/agent/extensions` when the link was
all it held. It then stops the
daemon and moves `rerail.sqlite` (with its `-wal`/`-shm`) into
`~/.rerail-clean-YYYYMMDD/rerail-home/` (`-YYYYMMDD-HHMMSS` when one from today
exists), so your rules and history are kept. Restart open harness sessions
afterwards: their loaded hooks point at files that are about to go.

It also takes the `PATH` entry set-up added out of every shell startup file
(or the Windows user `Path`). Last, it deletes the installation itself:
`~/.rerail/current`, the launcher's `~/.rerail/bin`, every version under
`~/.rerail/versions` with `update-state.json`, the rule packs in
`~/.rerail/packs`, and the cache `~/.cache/rerail` (left alone, and said so,
while another Rerail home's daemon holds it). In `bin`, `versions` and `packs`
it deletes only what Rerail put there: a folder that also holds other files
(a Rerail home shared with them, as with `RERAIL_HOME=$HOME`) is kept with
those files, and named. It prints each path it removed,
and what it kept in the Rerail home (settings such as `consent.json`, logs,
records), which you can delete if you will not install Rerail again; a home
that also holds other files is never offered for deletion as a whole, only
Rerail's files in it are named. A file it
cannot remove, such as one Windows keeps locked while a session still runs
Rerail, is named with the command that finishes the job once that session is
closed (`Remove-Item -LiteralPath '<path>' -Recurse -Force` in PowerShell, and
`[System.IO.Directory]::Delete('<path>')` for the `current` link, which removes the link and not
what it points at; `rm -rf <path>` elsewhere); the rest of the uninstall has
happened, so it still exits 0.

### Removing everything, data included

`rerail install --remove --delete-data` does all of the above, but deletes
instead of backing up, and prints each path it deleted:

- `rerail.sqlite` and `harness.sqlite` in the Rerail home, with every file named
  after them (`-wal`, `-shm`, journals);
- every earlier backup, `~/.rerail-clean-*`;
- the session ledgers, `<checkout>/.rerail/sessions`, of every checkout the
  database recorded (read before the database goes); the checkout's `.rerail`
  goes too when the ledgers were all it held. A checkout that no longer exists
  is skipped. A link is never followed: a `.rerail` that is a link is left alone,
  and a `sessions` that is a link is removed as a link, never what it points
  at. Ledgers written elsewhere with `RERAIL_STORAGE_ROOT` are not found;
- the content cache: `~/.cache/rerail` (as above, kept while another Rerail
  home's daemon holds it), and the file `RERAIL_CONTENT_STORE_PATH` names, with
  its sidecars, when it is set.

`rerail install --remove --delete-data --dry-run` lists what it would delete.
`--delete-data` only goes with the whole uninstall: with `--agents`, or with
`--root` naming a checkout other than the active install, it refuses before
changing anything. Nothing it deletes can be restored, so export your rules
first if you may want them back.

## Developing from a checkout

A source checkout builds and tests with Node.js 24+, npm, Rust, and
[`just`](https://github.com/casey/just); see
[Development setup](../CONTRIBUTING.md#development-setup) in the contributing guide.

```sh
just rerail-dev --install-bridge
```

This builds the checkout, activates it as `~/.rerail/current` the way `setup`
activates a published version, writes the launcher, registers every harness,
and starts the checkout's dev daemon on the default home; its hooks name
`~/.rerail/current`, never the checkout path. `just nuke-rerail` removes it
again (with a backup). Run
`npx @neralabs/rerail setup` to return to the published package. Restart
harness sessions after every rebuild.
