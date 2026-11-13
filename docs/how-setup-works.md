# How set-up works

What `rerail setup` does and writes, and how installation, updates, the database and removal work underneath. Short
guide: [Installing Rerail](installation.md). Problems: [Troubleshooting](troubleshooting.md).

## The managed installation

Set-up copies Rerail out of npm's ephemeral `npx` cache into a durable install in `~/.rerail`.

```text
~/.rerail/
├── versions/<version>/node_modules/@neralabs/rerail/  the installed package
├── current  →  versions/<version>/node_modules/@neralabs/rerail
├── bin/rerail                                         launcher shim → current/dist/cli/cli.js
├── update-state.json                                  active + previous version, channel
├── daemon.json / daemon.log                           the running daemon
├── daemon-port.json                                   the port the daemon comes back on after a restart
├── coding-agents.json                                 the coding agents set-up and rerail install have seen
├── packs/node_modules/@neralabs/rerail-pack-<id>/     Rerail's rule packs you installed (rerail packs)
├── packs/node_modules/[@<scope>/]rerail-pack-<id>/    third parties' rule packs you installed
└── versions/<older>/…                                 the versions rerail update replaced
```

## Set-up step by step

The steps match in a terminal and a coding agent; nothing reaches a repository, Git or the database before you say yes.

**0. Checks.** Set-up stops, says what to do and changes nothing on Node below 24, without Git, or on a computer
Rerail has no engine for. Those are musl Linux such as Alpine, glibc older than 2.28, and other systems or processors.
See [Which Git and Node Rerail runs](#which-git-and-node-rerail-runs).

**1. Install.** Set-up runs `npm install @neralabs/rerail@<this version>` into `versions/<version>`, points `current`
at it and writes `bin/rerail`. Nothing is turned on, no agent connected, no daemon started.

<details>
<summary>How the install works</summary>

- `versions/<version>` is a staging folder until it passes validation: package identity, native kernel loading, a
  real event-stream round-trip.
- Then `current` is re-pointed atomically and `bin/rerail` (re)written; the npx copy is never used again.
- An install that fails later removes the empty folders it made.
- Skipped when set-up runs from the version `current` points at. Once any version is installed, answer runs
  (`--answer`, `--repo`) and `/rerail-start` skip it too: OMP and Pi run those from the session's own package, which
  may be older than `current`.

</details>

**May Rerail read your sessions.** Asked once per computer: may Rerail read connected agents' sessions (your prompts,
their actions, the files they edit) to check them against your rules?

- **Yes** (`--answer allow`): kept in `~/.rerail/consent.json`, never asked again.
- **No** (`--answer decline`): stops set-up. Rerail stays installed and off.

<details>
<summary>Consent details</summary>

- The question comes before set-up reads a repository or connects an agent. A yes also lets Rerail show the
  sessions' events in the dashboard.
- A yes is kept with the run's folder (`--repo`). Reading that folder is still asked.
- A no connects and starts nothing new, and clears an earlier yes so the next set-up asks again. Agents connected
  before stay until `rerail install --remove`. `rerail` stays on `PATH`; the screen says `rerail install --remove`
  takes it off the computer.
- `rerail start` in a repository not set up asks too; an update's health check does not.

</details>

**2. Ask before reading.** Set-up names its Git repository and asks to read it. Outside a repository, or in your home
folder, it asks which folder to start with (type one, or skip for now). `--repo <folder>` names one and is the
go-ahead.

**3. Recommend.** The repository's files and `package.json` dependencies are matched against the rule packs, and each
pack that fits is shown.

<details>
<summary>What the recommendation shows</summary>

- Each fit shows its rule count, why it matched and the folder it would be added in.
- The agents to connect ("Will connect to: …"; `--agents` names others).
- Pack rules that start off or take options from the repository's own lint config
  ([The repository's own lint config](rule-scoping.md#the-repositorys-own-lint-config)).
- With no match, no pack is offered; the repository can still start and add packs in the dashboard later.

</details>

**4. Confirm.** [Rule scoping](rule-scoping.md#set-up) explains how each answer attaches packs.

- **Yes, start Rerail** (`--answer start`): installs the packs shown and starts.
- **Change rule packs…** (`--answer choose`): pick from every pack set-up fetched instead.
- **Not now** (`--answer not-now`): installs, connects and turns on nothing.
- To pick packs without a scan, answer the question before with **Skip, I'll pick packs** (`--answer pick-myself`);
  no code is read.

**5. Start, in this repository.** Set-up connects the chosen coding agents, attaches the packs and starts this
repository. Other repositories stay off.

<details>
<summary>What each coding agent gets</summary>

Set-up connects all agents found, unless `--agents` names others. It starts every checkout of the repository not
stopped on its own, this one included.

Registrations point at `~/.rerail/current`, so hooks follow an update unchanged. `rerail update` re-registers only
agents already connected, to bring the new version's commands and skills.

- **OMP:** link `~/.omp/agent/extensions/rerail` → `current`; `rerail` MCP server in `~/.omp/agent/mcp.json`. An
  OMP plugin install of Rerail is left alone; the summary says to remove it.
- **Pi:** `packages` entry in `~/.pi/agent/settings.json`, replacing any gallery `npm:@neralabs/rerail` entry but
  keeping its filters.
- **Claude Code:** hooks, status line (wrapping any you had) and
  [sandbox and permission denies](#sandbox-and-permission-entries) in `~/.claude/settings.json`; commands in
  `~/.claude/commands/`. Windows hooks run without Git Bash.
- **Codex:** hooks in `~/.codex/hooks.json` (trusted once with `/hooks` in Codex); one skill per command in
  `~/.codex/skills/<name>/SKILL.md`, invoked as `$rerail`, `$rerail-manage` and so on.
- **Cursor:** hooks in `~/.cursor/hooks.json`, commands in `~/.cursor/commands/`, permission denies in
  `~/.cursor/cli-config.json` if the Cursor CLI has written that file.

</details>

<details>
<summary>How config files are written</summary>

- Other tools' entries are kept, though the file is rewritten as JSON indented by two spaces.
- Rerail's own entries, from any install of this package (even one that is gone), are replaced, not duplicated.
- A config that is not plain JSON (a comment, a trailing comma) is left alone. Set-up still connects the other agents
  and starts the repository; the summary names the file, the reason and what to do.
- A `<file>.<timestamp>.bak` is written only on change. Re-running is a no-op.

</details>

**6. Summary.** Set-up lists the packs and rules added, the dashboard link, and agents it could not connect. In a
terminal it then opens the browser; run by a coding agent (`--relay`, `--json`), it shows the link alone.

<details>
<summary>What the summary shows</summary>

- The packs added by name ("TSLint in web" when added in one folder), rules on, other repositories Rerail is on in,
  the dashboard link (`#rules`, its Overview, scoped to repository and checkout), and agents that could not be
  connected.
- Commands show as `$rerail-show-me` when Codex is the only agent, and in Codex's form beside the others' when it is
  one of several. Once Codex's hooks are written, a line says to trust them with `/hooks` in Codex.
- With no agent found, it says so, names the supported ones, and says to rerun `rerail setup` once one is installed.
- The daemon is the active install's and serves every repository; one from another version or port is replaced
  (`--port` picks the port).
- In a terminal the browser opens unless `BROWSER=none`, the session is over SSH, or Linux has no display. A coding
  agent's run opens none: it may run where no one watches the screen, or in a sandbox.

</details>

### Sandbox and permission entries

Set-up denies coding agents access to Rerail's data wherever the agent's user settings document a place for it.

- **Rerail home** (`~/.rerail`, or `RERAIL_HOME`): access token, every repository's history database, installed
  package.
- **Content cache** (`~/.cache/rerail`, on Windows `%USERPROFILE%\AppData\Local\rerail`): parsed files and results.

Agents run commands as you, so only their own sandbox can keep them out.

| Coding agent | What set-up writes |
| --- | --- |
| Claude Code | Sandbox and permission entries in `~/.claude/settings.json` |
| Codex | Nothing |
| Cursor editor | Nothing |
| Cursor CLI | File-tool denies in `~/.cursor/cli-config.json` |
| OMP and Pi | Nothing: no sandbox |

<details>
<summary>General rules</summary>

- Set-up never turns a sandbox on or off, or changes other sandbox or permission settings.
- A Rerail home that is your home folder, or above it, is skipped, since denying it would deny every project. The
  cache is still denied.
- `rerail install --remove` takes out exactly what it added; entries you already had stay.
- Re-running set-up with another `RERAIL_HOME` moves the denies to it.

</details>

<details>
<summary>Claude Code entries</summary>

- Sandbox `filesystem.denyRead` and `filesystem.denyWrite` name both folders. `filesystem.allowRead` reopens
  `current` and `versions` in the Rerail home, so commands can read the installed package but no data.
- `permissions.deny` covers the file tools, which the sandbox does not: `Edit(...)` for both folders; `Read(...)` for
  the whole cache and the Rerail home's data files (`*.json*`, `*.sqlite*`, `*.log*`, `*.lock`, `access-token*`,
  `active-profile*`, `device-secrets/`, `profiles/`). The installed package, `bin/` and `packs/` stay readable:
  `/rerail-create` reads its skill there, and a deny cannot be reopened for one folder.
- `sandbox.excludedCommands` holds Rerail's CLI as Rerail commands run it (`<node> <root>/dist/cli/cli.js *`), the
  launcher (`~/.rerail/bin/rerail *`) and `rerail *`. These run unsandboxed under your normal permission rules: what a
  Rerail command pre-approves runs, anything else asks. A Rerail command with a redirect, a `cd` or a `$(...)` stays
  sandboxed and cannot read Rerail's files.
- Hooks, the status line and MCP servers run outside the sandbox, so Rerail's hooks are unaffected.
- The sandbox is off until you turn it on (`/sandbox`). While off, only the file-tool denies apply, and shell commands
  can read and change Rerail's data. On native Windows Claude Code has no sandbox; only the file-tool denies are
  written.

</details>

<details>
<summary>Codex, Cursor, OMP and Pi</summary>

- **Codex:** no stable user setting denies a path; permission profiles can, but are in beta and replace the
  `sandbox_mode` settings. The default `workspace-write` sandbox lets commands write only to the workspace and
  temporary folders, network off: they cannot change Rerail's data but can read it, access token included.
- **Cursor editor:** `sandbox.json` can only add paths, and the editor's agent has no path deny. Sandboxed commands by
  default write only to the workspace and temporary folders, loopback blocked, but can read Rerail's data. "Read
  Access: Workspace" in Cursor's settings makes the agent ask before reading outside the workspace; it covers every
  path, so set-up leaves it alone.
- **Cursor CLI** (`cursor-agent`): `Write(...)` denies for both folders and Claude Code's `Read(...)` denies, for its
  file tools only. Written only when the CLI has written that file; on Windows as native paths
  (`C:\Users\me\.rerail\**`), the only form the CLI matches.
- **OMP and Pi** have no sandbox: their agents can read and change all of Rerail's data.

</details>

## Running set-up again

Re-running set-up connects newly found coding agents, and never asks again in a repository already set up.

Other repositories stay off until you start Rerail there (see [Start and stop](installation.md#start-and-stop)).
Starting one that is not set up runs steps 2 to 6 for it.

<details>
<summary>Details</summary>

- **"Skip for now"** (no repository) connects the agents found and starts Rerail nowhere.
- **In a set-up repository**, it connects the agents found since ("Now also connected" in the summary), then shows the
  summary, or says Rerail is stopped there.
- **`~/.rerail/coding-agents.json`** lists every agent set-up or `rerail install` has seen, except one that could not
  be connected, so the next set-up retries it. Without the file, every agent found counts as new.

</details>

## `rerail` on your PATH

Every set-up run puts `~/.rerail/bin` on `PATH`, so `rerail` runs from a new terminal.

Shells that are not interactive (`zsh -c`, scripts) miss it: call `~/.rerail/bin/rerail` by its full path there.
`RERAIL_MODIFY_PATH=0` stops set-up changing `PATH`.

<details>
<summary>Per-platform files</summary>

On macOS and Linux, a block between `# >>> rerail >>>` and `# <<< rerail <<<` goes into the startup file `$SHELL`
(else the account's login shell) reads:

| Shell | Startup file |
| --- | --- |
| zsh | `$ZDOTDIR/.zshrc` (else `~/.zshrc`) |
| bash | `~/.bashrc`, and the first of `~/.bash_profile`, `~/.bash_login`, `~/.profile` that exists (else `~/.profile`) |
| fish | its own `~/.config/fish/conf.d/rerail.fish` |
| other | `~/.profile` |

The block adds the directory only when `PATH` lacks it, so a file sourced twice adds it once. On Windows the
directory is appended to the user `Path` (`HKCU\Environment`), keeping that value's registry kind. The Windows
launcher is `rerail.cmd`, with an sh launcher `rerail` beside it for Git Bash, which runs neither `.cmd` nor `.bat`
files by bare name. In Git Bash's own window (mintty), which gives Windows programs pipes instead of a console, that
launcher starts node through winpty, so set-up shows a person its menu.

</details>

<details>
<summary>Ownership, backups and skipped changes</summary>

- **Ownership.** Set-up records that it handled `PATH` in `~/.rerail/launcher-path.json`; from then on `PATH` is
  yours. A block you take out stays out until `rerail setup` runs again. `rerail update` adds the entry only when that
  record is missing.
- **When nothing changes.** Set-up says what it changed. It changes nothing when the running `PATH` already has the
  directory, the entry is in place, or `RERAIL_MODIFY_PATH=0`. A block written for another Rerail home is replaced.
- **Backup.** A changed startup file keeps one `<file>.rerail.bak` beside it: the file before Rerail last changed it.
  Each change replaces it; `rerail install --remove` deletes it once the block is out.

</details>

<details>
<summary>Non-interactive shells</summary>

The block is in the file an interactive shell reads, so `zsh -c`, `ssh host rerail …` to a zsh account, and scripts
miss it. There, call `~/.rerail/bin/rerail` by its full path, or add the directory to `PATH` yourself in a file every
such shell reads (`~/.zshenv` for zsh).

Rerail does not write `~/.zshenv`: every zsh reads it, scripts included, and on macOS `/etc/zprofile` reorders `PATH`
after it. Coding agents run Rerail by its full path, so they need neither.

</details>

## How set-up asks

Set-up asks as a terminal menu, as text a coding agent relays, or as JSON for a coding agent's own UI.

- **Terminal menu:** when stdin and stdout are TTYs and there is no `--relay`. Move with the arrow keys or a digit;
  Enter answers.
- **Relay:** otherwise, or with `--relay`, each run prints the next question and exits 0. The agent reruns set-up
  with the person's answer as flags.
- **JSON:** `--json` on `rerail setup` and `rerail start` prints one `{ step, screen, question }` object per run.

<details>
<summary>Menu keys and relay flags</summary>

In the menu, ↑/↓ (or k/j) move the `❯`, wrapping around; a digit moves it to that answer; Enter takes it, so `2` then
Enter still works. Ctrl-C restores the terminal and stops set-up; Ctrl-D cancels it. The folder and rule-pack prompts
read a typed line.

Without a terminal (stdin or stdout is not a TTY, as under a coding agent), the agent relays each question as written,
with one command per answer. Answer flags:

- `--answer <allow|decline|yes|other-folder|pick-myself|start|choose|not-now|folder|skip>`
- `--repo <folder>`
- `--agents <ids>` (`omp,pi,claude-code,codex,cursor`): works on its own or with any answer; no question asks for it.
- `--packs <id@version,…>`: the exact versions an answer installs; a third party's pack by its package name
  (`@acme/rerail-pack-sql@1.0.0`); `none` for none.

Each answer run starts from what it is told, so running one twice repeats no earlier step's side effect. Claude
Code's, Cursor's and Codex's `/rerail-start` runs `rerail start --relay`, and every answer command carries `--relay`,
so an agent running commands in a terminal gets these questions, not a prompt waiting on input. On Windows Codex's and
Cursor's `/rerail-start` runs `rerail start --relay --powershell`, since they run PowerShell there: each answer
command is then written for PowerShell (`& '<node>' '<rerail home>\current\dist\cli\cli.js' setup ...`, or the CLI
running set-up until set-up has installed one) and carries `--powershell` too. Every value PowerShell would read goes in single quotes, as
`--packs 'typescript@0.1.1,complexity@0.1.1'`, since PowerShell passes a bare `a,b` to a program as two arguments; so
does the value the agent writes in (`--packs '<packs>'`). PowerShell ends a single-quoted value at any of `'`, `‘`, `’`,
`‚` and `‛`, so the answer for a folder the person names tells the agent to write each of them twice in it (a folder
named `O'Neil` goes in as `'O''Neil'`). Claude Code runs Git Bash on Windows, so its answer commands
are written for a POSIX shell there too, and start with the sh launcher `rerail`. No answer runs `rerail.cmd`: cmd.exe
reads a batch file's arguments again, so a folder named `C:\code\R&D` would end the command at its `&` and run the
rest, and a `%NAME%` in one would expand.

</details>

<details>
<summary>JSON fields</summary>

The JSON object is the only thing on stdout.

- **`step`**: `sessions`, `read-repo`, `confirm-packs`, `pick-packs` or `no-repo` at a question, else `done`,
  `stopped`, or `error` (exit 1).
- **`screen`**: plain text for the person.
- **`question`**: each option carries its answer key, label, `detail`, and the arguments that give it (`next.args`,
  starting with `setup` and carrying `--json`). An answer that needs a folder or agent ids also carries `input`, the
  value to ask for first (append `[flag, value]`).

`rerail start --json` in the home folder asks which project to start with. The side effects are the agent lane's.
`--database <file>` names the database an extension was configured with.

</details>

<details>
<summary>The picker in OMP and Pi</summary>

In OMP and Pi, `/rerail-start` always runs this picker through the launcher's `rerail start --json`. Each question's
text shows as a notice, then the question as a select (a text box for a folder or agent ids); the summary shows when
set-up ends. In a set-up repository the first step starts Rerail and says so. Closing a picker stops set-up and turns
nothing on. Without a UI (print mode) it shows `rerail start`'s text. Claude Code, Codex and Cursor relay the
questions as text.

</details>

`rerail install` (see [Commands after set-up](#commands-after-set-up)) still connects coding agents on its own, with
`--agents <list>`, `--root` and `--dry-run`.

## Which Git and Node Rerail runs

Rerail runs the first safe `git` it finds, and the Node that ran set-up.

After moving to another Node, run `rerail install --refresh` with it first on `PATH` to record it for the launcher,
hooks and status line.

<details>
<summary>How Git is found</summary>

Rerail reads every repository through the first `git` on `PATH` (`git.exe` on Windows). It skips relative entries,
and entries inside the repository it runs in (the nearest folder above with a `.git`) or a `node_modules` folder, so a
repository's own `bin/` cannot put its `git` first. As a shell does, it passes over a folder named `git` or a `git`
without its execute bit.

With none on `PATH`, it falls back to the usual install locations: Homebrew, Linuxbrew and Nix on macOS and Linux;
Git for Windows and scoop on Windows. On macOS the `/usr/bin/git` stub counts only with the Command Line Tools or
Xcode installed.

</details>

<details>
<summary>How Node is recorded</summary>

Set-up records the Node that ran it for the launcher, the hooks and the Claude Code status line. Homebrew's is
recorded by the link `brew upgrade` keeps (`/opt/homebrew/bin/node`), not its versioned `Cellar` folder. Once the
recorded Node is gone (after `nvm uninstall`, say), each falls back to `node` on `PATH`. The status line does so on
Windows too: Claude Code runs it through Git Bash there, so its `sh -c` fallback works as on macOS and Linux.

On a Node older than 24, the CLI and the daemon stop with set-up's message, the hooks do nothing, and the status line
says `needs Node 24 or newer`.

</details>

## Commands after set-up

Run these from any terminal opened after set-up put `~/.rerail/bin` on `PATH`.

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

Every coding agent also gets slash commands, from `/rerail` to `/rerail-bug-report`. Restart the coding agent after
installing or rebuilding to load them.

<details>
<summary>Coding agent commands</summary>

The commands are `/rerail`, `/rerail-manage`, `/rerail-create`, `/rerail-start`, `/rerail-stop`, `/rerail-update`,
`/rerail-uninstall`, `/rerail-help`, `/rerail-show-me` and `/rerail-bug-report` (`$rerail`, `$rerail-manage`, and so
on in Codex). In Claude Code and Cursor, `/rerail-manage` opens the dashboard as a tab in the Orca or Cursor window it
runs in, else in the system browser.

After activating a version, `rerail update` runs that version's `rerail install --refresh`: each connected agent gets
its commands, skills and hooks and loses retired ones; an agent never connected is left alone. Generated command
files and skills are replaced without a `.bak` copy.

</details>

A scan or settings invocation must supply the exact session identity. See the
[scan and settings contract](reference.md#scan-and-settings-controls) for scope, mode, persistence, diagnostics, and
background versus next-event delivery.

## Updates

Rerail updates only when you ask; nothing polls the registry or installs a version in the background.

The only paths are `rerail update` and re-running `npx @neralabs/rerail setup`. `rerail update --channel <tag>` picks
the npm dist-tag (`latest`, `beta`, or `next`).

<details>
<summary>Registry and credentials</summary>

`@neralabs/rerail` is published only to npmjs (`https://registry.npmjs.org/`). Rerail adds no registry
configuration: `setup` and `rerail update` run the `npm` next to their Node, with your normal npm configuration.

If your npm token for npmjs is invalid or expired, npm-backed commands such as `rerail update` report that the npm
registry credentials are missing or expired; sign in again with `npm login`, or remove the token. Rerail redacts
`_authToken` and `_password` values from any npm output it prints.

</details>

<details>
<summary>How updates are verified</summary>

`rerail update` asks for the latest version from an empty temporary folder, so a repository's own `.npmrc` cannot
choose the registry; your `~/.npmrc` still applies. It installs without running any install scripts, then checks
registry signatures and provenance attestations of everything installed with `npm audit signatures`, before running
any of it.

A signature that does not verify stops the update with nothing installed. A registry that publishes no signing keys,
as some private mirrors do, gets a warning naming it, and the update goes on.

</details>

## The database

One database, `~/.rerail/rerail.sqlite` (`$RERAIL_HOME/rerail.sqlite`), holds every repository on the computer; `rerail status` names it.

[Rerail database](reference.md#rerail-database) covers what it holds and what `rerail reset` and `rerail scrub` do to
it; [Deleting your data](privacy.md#deleting-your-data) covers removing it.

`rerail update` first copies it to `rerail.sqlite.pre-update` beside it, kept only when the version changes: the
database before the now-active version, to put back by hand (see [Database compatibility](#database-compatibility)).

## The content store

Policy evaluation caches parsed files and results in a content store; where the package lives, not a flag, picks which one.

The managed installation uses `~/.cache/rerail/content-v1.sqlite`, and on Windows `%USERPROFILE%\AppData\Local\rerail\content-v1.sqlite`, out of
a roaming profile's reach (earlier releases kept it in `~/.cache/rerail` there too; the uninstall removes that copy); a
build from a Git checkout uses `<checkout>/.rerail/content-v1.sqlite`. `RERAIL_CONTENT_STORE_PATH` overrides either location.

<details>
<summary>Which store a build uses</summary>

- **Managed installation** (a package under `~/.rerail/versions/<version>/` or `$RERAIL_HOME/versions/`, normally
  loaded through `current`): owns the machine-wide `~/.cache/rerail/content-v1.sqlite`.
- **Any build from a Git checkout** (source, `dist/`, tests, benchmarks, every worktree):
  `<checkout>/.rerail/content-v1.sqlite` (`.rerail/` is gitignored), so a checkout on a newer store schema never
  changes the store the installed version reads.
- **Neither** (say, an `npx` copy): no store, reported on its first scan.

The file name follows the store's compatibility floor (`v1` today) and changes when the floor rises.

</details>

## One main instance

At most one main Rerail instance runs per machine: the managed installation's daemon, the only process that opens the machine-wide content store.

If a second main daemon exits at startup with `Another main Rerail instance is running: pid …, version …`, stop that
process (`rerail daemon stop`, or end it by pid) and start again.

<details>
<summary>The lock</summary>

The daemon holds an exclusive OS lock (`flock` / `LockFileEx`) on `~/.cache/rerail/main-instance.lock` (on Windows
`%USERPROFILE%\AppData\Local\rerail\main-instance.lock`) for its whole life, and records its pid, version, package root and start time
beside it in `main-instance.json`. The OS drops the
lock however the daemon ends, `kill -9` included, so there is never a stale lock to clear.

A second main daemon can come from another `$RERAIL_HOME`, or be an orphan its `daemon.json` no longer names.
`rerail daemon start` prints its exit message from `daemon.log`.

</details>

<details>
<summary>What goes through the daemon</summary>

Everything else goes through the daemon, not in-process evaluation. The Claude Code, Codex and Cursor hooks, OMP and
Pi sessions, and `rerail events` streams (in internal testing) send events to it, starting it when it is not running,
after a reboot too (unless `RERAIL_AUTOSTART=0`). CLI commands such as `rerail settings` and `rerail scan` (in
internal testing) use its API.

Any other process opening the machine-wide store gets an error and a failed policy run, not a second writer.
Development builds never take the lock and never open that store.

</details>

<details>
<summary>How updates hand over</summary>

Updates hand over without overlap. `rerail update` only activates the new version. The next hook or session from it
stops the old daemon, waits until that process has exited and the OS has released its lock, then starts the new
daemon, which takes the lock. `rerail setup` replaces a stale daemon the same way. A `rerail events` stream (in
internal testing) that outlives an update sends to whichever daemon is running and never brings its own version back.

</details>

## Database compatibility

`rerail.sqlite` records its compatibility floor in `PRAGMA user_version`: the oldest schema version a Rerail must read to open it.

Once a newer Rerail raises the floor, older versions refuse the file. To run an older version against it anyway, put
back the copy `rerail update` took before that version change:

1. Stop Rerail (`rerail daemon stop`) and close the coding agent sessions that have it open.
2. Keep `rerail.sqlite` elsewhere if you need what changed since the update.
3. Copy `rerail.sqlite.pre-update` over `rerail.sqlite`.

A file stamped by a newer Rerail, or written by other software, is refused with an error and left untouched. Move it
aside (or point `RERAIL_HOME` elsewhere), start again, then re-import your rules with `rerail rules import`.

<details>
<summary>How migrations run, and which copy is kept</summary>

A newer Rerail that first opens the file (the daemon, `rerail start`, set-up, or a command, hook or status line that
only reads) runs the pending migration steps in one transaction. A failing step leaves the file as it was. A process
that cannot write the file gets an error naming those steps, and the file is left as it was.

Most steps add a column or a table, which older versions read correctly: the floor stays, an older Rerail still opens
the file, and a hook or the status line runs such steps itself. A step an older version would misread raises the
floor. It waits for the new version's daemon or a command that writes; until then a hook or the status line says to
run `rerail daemon start`.

An update that finds nothing newer, or fails, keeps the copy from the last version change. One that changes the
version but cannot take a copy removes the old one, which no longer holds the database before the now-active version.

</details>

## Removing Rerail in detail

`rerail install --remove` takes Rerail off a machine and keeps your rules and history in a backup.

```sh
rerail rules export --output ~/rerail-rules.json   # optional: a portable copy of your rules
rerail install --remove --dry-run                  # print what would change
rerail install --remove                            # or: --remove --delete-data, keeping nothing
```

The database goes to `~/.rerail-clean-YYYYMMDD/rerail-home/`. Restart open harness sessions afterwards: their loaded
hooks point at files that are about to go.

<details>
<summary>What it removes</summary>

- **Unregisters** everything naming this installation, leaving other tools' entries alone. Each changed file gets a
  `<file>.<timestamp>.bak` beside it. Removed:
  - harness hooks, and the Claude Code status line and commands;
  - the sandbox and permission denies set-up added to `~/.claude/settings.json` and `~/.cursor/cli-config.json`;
  - the Cursor commands, the Codex skill, and the OMP extension symlink and MCP entry;
  - the Pi package entry (an `npm:@neralabs/rerail` one too, when it removes the managed install);
  - a folder that held only what it removed: each `~/.codex/skills/rerail*` (and `~/.codex/skills` when those were
    all it held), and `~/.omp/agent/extensions` when the link was all it held.
- **Backs up the database:** stops the daemon and moves `rerail.sqlite` (with its `-wal`/`-shm`) into
  `~/.rerail-clean-YYYYMMDD/rerail-home/` (`-YYYYMMDD-HHMMSS` when one from today exists).
- **Removes the `PATH` entry** set-up added from every shell startup file (or the Windows user `Path`).
- **Deletes the installation**, last: `~/.rerail/current`, the launcher's `~/.rerail/bin`, every version under
  `~/.rerail/versions` with `update-state.json`, the rule packs in `~/.rerail/packs`, and the cache `~/.cache/rerail`
  (on Windows `%USERPROFILE%\AppData\Local\rerail`, and the `~/.cache/rerail` an earlier release used there; left alone, and said so, while
  another Rerail home's daemon holds it).

</details>

<details>
<summary>What it keeps</summary>

In `bin`, `versions` and `packs` it deletes only what Rerail put there. A folder that also holds other files (a Rerail
home shared with them, as with `RERAIL_HOME=$HOME`) is kept with those files, and named.

It prints each path it removed, and what it kept in the Rerail home (settings such as `consent.json`, logs, records),
which you can delete if you will not install Rerail again. A home that also holds other files is never offered for
deletion whole; only Rerail's files in it are named.

</details>

<details>
<summary>Locked files</summary>

A file it cannot remove, such as one Windows holds while a session still runs Rerail, is named with the command that
finishes the job once that session is closed. The rest of the uninstall has happened, so it still exits 0.

- PowerShell: `Remove-Item -LiteralPath '<path>' -Recurse -Force`; for the `current` link,
  `[System.IO.Directory]::Delete('<path>')`, which removes the link, not what it points at.
- Elsewhere: `rm -rf <path>`.

</details>

### Removing everything, data included

`rerail install --remove --delete-data` does all of the above but deletes instead of backing up, printing each path it deletes.

Nothing it deletes can be restored, so export your rules first if you may want them back.
`rerail install --remove --delete-data --dry-run` lists what it would delete.

<details>
<summary>What it deletes</summary>

- **Databases:** `rerail.sqlite` and `harness.sqlite` in the Rerail home, with every file named after them (`-wal`,
  `-shm`, journals).
- **Backups:** every earlier backup, `~/.rerail-clean-*`.
- **Session ledgers:** `<checkout>/.rerail/sessions` of every checkout the database recorded (read before the
  database goes), and the checkout's `.rerail` when the ledgers were all it held. A checkout that no longer exists is
  skipped. Links are never followed: a `.rerail` that is a link is left alone, and a `sessions` link is removed as a
  link, never its target. Ledgers written elsewhere with `RERAIL_STORAGE_ROOT` are not found.
- **Content cache:** `~/.cache/rerail`, on Windows `%USERPROFILE%\AppData\Local\rerail` (kept while another Rerail home's daemon holds it, as above), and the file
  `RERAIL_CONTENT_STORE_PATH` names, with its sidecars, when it is set.

`--delete-data` only goes with the whole uninstall: with `--agents`, or with `--root` naming a checkout other than the
active install, it refuses before changing anything.

</details>

## Developing from a checkout

A source checkout can stand in for the published package as your managed installation.

It builds and tests with Node.js 24+, npm, Rust, and [`just`](https://github.com/casey/just); see
[Development setup](../CONTRIBUTING.md#development-setup) in the contributing guide.

```sh
just rerail-dev --install-bridge
```

This builds the checkout, activates it as `~/.rerail/current` the way `setup` activates a published version, writes
the launcher, registers every harness, and starts the checkout's dev daemon on the default home. Its hooks name
`~/.rerail/current`, never the checkout path.

- `just nuke-rerail` removes it again (with a backup).
- `npx @neralabs/rerail setup` returns to the published package.
- Restart harness sessions after every rebuild.
