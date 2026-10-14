# Installing Rerail

Rerail lands on a machine through one command, `rerail setup`, delivered by
`npx` from npmjs. Everything below explains what that command does, what it
needs before it can run, and how to update, reset, and remove an installation.
To run Rerail from a source checkout instead, see "Developing from a checkout".

## One line

```sh
npx @neralabs/rerail setup
```

Run it again at any time: setup is idempotent, so it doubles as "upgrade". In a
repository that is set up already it changes nothing there and connects no
coding agent installed since; name one to connect it, for example
`npx @neralabs/rerail setup --agents codex`.

## What setup does

`npx` fetches the package into npm's cache and runs its `rerail` binary from
there. That copy is ephemeral, so `setup`'s first job is to make a durable one:

```
~/.rerail/
├── versions/<version>/node_modules/@neralabs/rerail/   the installed package
├── current  →  versions/<version>/node_modules/@neralabs/rerail
├── bin/rerail                                          launcher shim → current/dist/cli/cli.js
├── update-state.json                                  active + previous version, channel
├── daemon.json / daemon.log                           the running daemon
└── versions/<older>/…                                 kept for `rerail update rollback`
```

Set-up follows the same steps in a terminal and from a coding agent. Nothing
in a repository, its Git metadata, or the database is written before the
person says yes.

0. **Node check** — on a Node major below 24 set-up stops before changing
   anything and says how to upgrade.
1. **Install** — `ManagedUpdater.install()` runs `npm install @neralabs/rerail@<this version>`
   into `versions/<version>` (a staging directory until validation passes),
   checks the package identity, loads the native kernel and runs a real
   event-stream round-trip, then activates it. `current` is re-pointed
   atomically and `bin/rerail` is (re)written. The npx copy is never referenced
   again. Installing turns nothing on, connects no coding agent and starts no
   daemon. A run from the version `current` already points at skips this step.
   So does an answer (`--answer`, `--repo`) or a `/rerail-start` step once any
   version is installed: OMP and Pi run those from the session's own package,
   which may be older than `current`, and installing it would undo
   `/rerail-update`.
2. **Ask before reading** — set-up names the Git repository it ran in and asks
   whether it may read it. Outside a repository (or in your home folder) it
   asks which folder to start with instead; `--repo <folder>` names one, which
   is the go-ahead to read it.
3. **Recommend** — the repository's files and `package.json` dependencies are
   matched against the rule packs; each pack that fits is shown with its rule
   count, why it matched and the folder it would be added in, with the coding
   agents found and not found. A repository that matches no rule pack is
   offered none; it can still start, and add rule packs in the dashboard later.
4. **Confirm** — start, review the packs in the dashboard first (the review
   link opens `#rule-packs?review=1` scoped to the repository and checkout,
   and set-up asks again rather than waiting on the browser), name the coding
   agents to connect, or pick rule packs yourself (the repository is set up
   with no packs and no code is read). See [Rule scoping](rule-scoping.md#set-up)
   for how each answer attaches packs.
5. **Start, in this repository** — the chosen coding agents (all found,
   unless named) are connected, the recommended packs (or the ones kept on in
   the review) are attached to the repository, and the repository is started:
   every checkout of it, the one set-up ran in included, that has not been
   stopped on its own. Other repositories stay off. Connecting writes persistent registrations, all pointing at
   `~/.rerail/current` so future updates never touch coding agent config:
   - OMP: symlink `~/.omp/agent/extensions/rerail → current`, plus a `rerail` MCP server entry in `~/.omp/agent/mcp.json`
   - Pi: `packages` entry in `~/.pi/agent/settings.json`
   - Claude Code: `hooks` in `~/.claude/settings.json` running `current/dist/hosts/claude/claude-hook.js` (on Windows as exec-form entries that Claude Code 2.1.139+ spawns without Git Bash: `node` from `PATH` starts `dist/harness/hook-launcher.js`, which prefers the `node.exe` recorded at setup and falls back to the `PATH` node once that binary is gone), and the Rerail commands (`/rerail`, `/rerail-ui`, `/rerail-create`, `/rerail-start`, `/rerail-stop`, `/rerail-update`, `/rerail-rage-quit`) under `~/.claude/commands/` (`/rerail-ui` opens the dashboard as a tab in the Orca or Cursor window before falling back to the system browser)
   - Codex: `~/.codex/hooks.json` running `current/dist/hosts/codex/codex-hook.js` (Codex asks you to trust it once via `/hooks`), plus one skill per Rerail command under `~/.codex/skills/<name>/SKILL.md`, invoked as `$rerail`, `$rerail-ui`, and so on
   - Cursor: `~/.cursor/hooks.json` running `current/dist/hosts/cursor/cursor-hook.js`, plus the same Rerail commands under `~/.cursor/commands/` (`/rerail-ui` opens the dashboard as a tab in the Cursor window before falling back to the system browser)

   Existing entries from other tools are preserved byte-for-byte. Entries
   another install of this package wrote (identified by the `package.json`
   beside the script they run) are replaced, not duplicated; that includes a
   Claude status line such an install composed, and because what it wrapped is
   recorded only in that install's home, removing Rerail then leaves no status
   line there. A hook, status line
   or MCP bridge entry of the shape setup writes is also replaced when its
   install is gone, with neither its script nor a `package.json` beside it on
   disk, since it can never run again. So is an OMP extension link or Pi
   package that names a gone root of the shape setup registers (a Rerail
   home's `current`, such as `~/.rerail/current` or `$RERAIL_HOME/current`, or
   `node_modules/@neralabs/rerail`) that no longer resolves; a path that still
   resolves stays unless it holds this package, and so does a missing path of
   any other shape, such as a gone checkout. Setup's report names the target of each link it removes. The retired `/rerail-session`, `/rerail-disable` and
   `/rerail-enable` command files are removed when they run this install,
   another install of this package or one that is gone; anyone else's stay, and
   removing Rerail removes only this install's. A backup
   `<file>.<timestamp>.bak` is written only when a file changes. Re-running is
   a no-op.
6. **Summary** — the rule packs added (by name, "TSLint in web" when added in
   one folder), the rules on, the other repositories Rerail is on in, the
   coding agents connected, and the dashboard link (`#rules`, scoped to the
   repository and checkout). The daemon used is the active install's, which
   serves every repository; one from another version or on another port is
   replaced (`--port` picks the port). The browser opens unless
   `BROWSER=none`, the session is over SSH, or Linux has no display. Add
   `~/.rerail/bin` to `PATH` (the Windows launcher is `rerail.cmd`).

"Skip for now" (no repository) connects the coding agents found and starts
Rerail nowhere. Set-up run again in a repository that is set up never asks
again: it shows the summary, or says Rerail is stopped there. Rerail stays off
in every other repository until you start it there (see "Starting and stopping
Rerail" below); starting a repository that is not set up runs steps 2 to 6
for it.

**Without a terminal** (stdin or stdout is not a TTY, as when a coding agent
runs it), or with `--relay`, each run prints the next question for the coding
agent to relay, as written, with one command per answer, and exits 0. The
coding agent runs set-up again with the person's answer: `--answer <yes|other-folder|pick-myself|start|review|other|skip>`,
`--repo <folder>` and `--agents <ids>` (`omp,pi,claude-code,codex,cursor`).
Each answer run starts from what it is told, so running one twice repeats no
earlier step's side effect. Claude Code's, Cursor's and Codex's `/rerail-start`
runs `rerail start --relay`, and every answer command carries `--relay`, so a
coding agent that runs commands in a terminal still gets these questions
instead of a prompt waiting on input.

**In a coding agent's own UI**, `--json` on `rerail setup` and on `rerail
start` prints one JSON object per run and nothing else on stdout:
`{ step, screen, question }` (`SetupStep` in `src/setup-step.ts`). `step` is
`read-repo`, `confirm-packs` or `no-repo` at a question, else `done`,
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

`rerail install` (below) still connects coding agents on its own, with
`--agents <list>`, `--root` and `--dry-run`.

## Prerequisites

- Node.js 24 or newer on `PATH`. The launcher remembers the Node that ran setup
  and falls back to `PATH`, so version-manager switches do not break hooks.
- macOS, Linux glibc, or Windows MSVC on arm64/x64. The package carries
  prebuilt native kernels for those; nothing is compiled on the user's machine.
- Read access to `@neralabs/rerail` on npmjs (see "Registry access").

## Registry access

npmjs (`https://registry.npmjs.org/`) is the only registry `@neralabs/rerail`
is published to. Rerail adds no registry configuration of its own: `setup` and
`rerail update` run the `npm` next to the Node that runs them, with your normal
npm configuration.

Until launch the package is restricted. Sign in once with `npm login` as a
member of the `neralabs` npm organization, or put a read-only granular access
token for the package in `~/.npmrc`. After launch the package is public and no
sign-in is needed.

When the credential is missing or has expired, npm-backed commands such as
`rerail update` report that the npm registry credentials are missing or
expired; sign in again with `npm login`. Rerail redacts `_authToken` and
`_password` values from any npm output it prints.

## After setup

The following commands describe the managed installation, once `~/.rerail/bin`
is on `PATH`:

```sh
rerail start             # start Rerail in this checkout (first time: runs set-up's questions for the repository)
rerail stop              # stop Rerail in this checkout; the daemon keeps serving others
rerail daemon start      # start the background daemon (--port, --host, --open, --dev, ...)
rerail daemon stop       # stop the background daemon
rerail status            # whether Rerail is started here, otherwise daemon/database/kernel
rerail ui                # opens the rules page (--print only prints the URL)
rerail session claude-code ID --open  # this session's status line and page URL; --open opens it
rerail scan --session ID --scope repository --wait  # inspect findings in Sessions
rerail settings --session ID --mode log             # save this workspace's default
rerail update check      # is a newer version on the channel?
rerail update            # install it; `rerail update rollback` undoes it
rerail reset             # delete this repository's session history, across all worktrees; rules stay
rerail rules export      # dump your rules and every repository's configuration to ~/.rerail/rules.json
rerail install --remove  # take Rerail out of every coding agent, stop the daemon, back up the database
```

Every coding agent gets the same commands: `/rerail`, `/rerail-ui`,
`/rerail-create`, `/rerail-start`, `/rerail-stop`, `/rerail-update` and
`/rerail-rage-quit` (`$rerail`, `$rerail-ui`, and so on in Codex). Settings,
scans, status, update checks, rollback and reset are the CLI commands above.
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
`?repository=<key>&checkout=<path>`. The rules that ship with Rerail are read
from the package and run only in repositories that add them (see
[Rule scoping](rule-scoping.md)).
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
machine-wide `~/.cache/rerail/content-v1.sqlite`. Any build running from a
Git checkout — source, `dist/`, tests, benchmarks, every worktree — uses
`<checkout>/.rerail/content-v1.sqlite` instead (`.rerail/` is gitignored), so a
checkout on a newer store schema never changes the store the installed version
reads. The file name follows the store's compatibility floor (`v1` today) and
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
  ("I'll pick rule packs myself" sets it up with none, and you pick them on
  the Rule packs page). Until a repository is set up, its sessions are
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
Pi sessions, and `rerail events` streams send their events to it, starting it
when it is not running (unless `RERAIL_AUTOSTART=0`), and CLI commands such as
`rerail scan` use its API. Any process other than the lock holder that tries to
open the machine-wide store gets an error and a failed policy run instead of a
second writer. Development builds never take the lock and never open that
store.

Updates hand over without overlap: `rerail update` only activates the new
version, and the next hook or session from it stops the old daemon, waits
until that process has exited and the OS has released its lock, then starts
the new daemon, which takes the lock. `rerail setup` replaces a stale daemon
the same way. A `rerail events` stream that outlives an update keeps sending to
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
`rerail.sqlite.restoring` (holding the rollback's process id and when the system
booted) sits beside the database and every Rerail process refuses to open it; try
again in a moment. A rollback that was killed leaves the file behind, but once its
process is gone, or the computer has restarted since, Rerail opens the database
again and the next rollback takes the file over. If Rerail still says a copy is
being put back when no rollback runs, delete the file. While
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
rerail rules export --output ~/rerail-rules.json   # optional: a portable copy of your rules
rerail install --remove --dry-run                  # print what would change
rerail install --remove
```

`rerail install --remove` takes out every registration that names this
installation — harness hooks, the Claude Code status line and commands, the
Cursor commands, the Codex skill, the OMP extension symlink and MCP entry, and
the Pi package entry — and leaves other tools' entries alone, writing a
`<file>.<timestamp>.bak` beside every file it changes. It then stops the
daemon and moves `rerail.sqlite` (with its `-wal`/`-shm`) into
`~/.rerail-clean-YYYYMMDD/rerail-home/` (`-YYYYMMDD-HHMMSS` when one from today
exists), so your rules and history are kept. Restart open harness sessions
afterwards: their loaded hooks point at files that are about to go.

Then delete the Rerail home (`~/.rerail`, or `$RERAIL_HOME`) and the content
store directory (`~/.cache/rerail`), and remove `~/.rerail/bin` from `PATH`.

## Developing from a checkout

A source checkout builds and tests with Node.js 24+, npm, and Rust; see
[Develop](../README.md#develop) in the README. After `npm run build`,

```sh
node dist/cli/cli.js install --root "$PWD"
```

activates the checkout as `~/.rerail/current` the way `setup` activates a
published version, writes the launcher, and registers every harness; its hooks
name `~/.rerail/current`, never the checkout path. Run
`npx @neralabs/rerail setup` to return to the published package. Restart
harness sessions after every rebuild.

## What is not automated yet

- Activation (`rerail activate`) is optional until a control plane exists.
- A service unit (launchd/systemd) so the daemon survives reboots; today
  adapters restart it on the next session.
