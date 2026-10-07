# Installing Rerail

Rerail lands on a machine through one command, `rerail setup`, delivered by
`npx` from npmjs. Everything below explains what that command does, what it
needs before it can run, and how to update, reset, and remove an installation.
To run Rerail from a source checkout instead, see "Developing from a checkout".

## One line

```sh
npx @neralabs/rerail setup
```

Run it again at any time: setup is idempotent, so it doubles as "upgrade and
re-register".

## What setup does

`npx` fetches the package into npm's cache and runs its `rerail` binary from
there. That copy is ephemeral, so `setup`'s first job is to make a durable one:

```
~/.rerail/
├── versions/<version>/node_modules/@neralabs/rerail/   the installed package
├── current  →  versions/<version>/node_modules/@neralabs/rerail
├── bin/rerail                                          launcher shim → current/dist/cli.js
├── update-state.json                                  active + previous version, channel
├── daemon.json / daemon.log                           the running daemon
└── versions/<older>/…                                 kept for `rerail update rollback`
```

1. **Install** — `ManagedUpdater.install()` runs `npm install @neralabs/rerail@<this version>`
   into `versions/<version>` (a staging directory until validation passes),
   checks the package identity, loads the native kernel and runs a real
   event-stream round-trip, then activates it. `current` is re-pointed
   atomically and `bin/rerail` is (re)written. The npx copy is never referenced
   again.
2. **Register** — `rerail install` writes persistent registrations for every
   harness found on the machine, all pointing at `~/.rerail/current` so future
   updates never touch harness config:
   - OMP: symlink `~/.omp/agent/extensions/rerail → current`, plus a `rerail` MCP server entry in `~/.omp/agent/mcp.json`
   - Pi: `packages` entry in `~/.pi/agent/settings.json`
   - Claude Code: `hooks` in `~/.claude/settings.json` running `current/dist/claude-hook.js` (on Windows as exec-form entries that Claude Code 2.1.139+ spawns without Git Bash: `node` from `PATH` starts `dist/hook-launcher.js`, which prefers the `node.exe` recorded at setup and falls back to the `PATH` node once that binary is gone), the general `/rerail` command, and exact `/rerail-disable` / `/rerail-enable` / `/rerail-ui` / `/rerail-session` commands under `~/.claude/commands/` (`/rerail-ui` and `/rerail-session` open the page as a tab in the Orca or Cursor window before falling back to the system browser)
   - Codex: `~/.codex/hooks.json` running `current/dist/codex-hook.js` (Codex asks you to trust it once via `/hooks`; it cannot expose an exact third-party slash-command name, so use the terminal CLI)
   - Cursor: `~/.cursor/hooks.json` running `current/dist/cursor-hook.js`, plus exact `/rerail-disable` / `/rerail-enable` / `/rerail-ui` commands under `~/.cursor/commands/` (`/rerail-ui` opens the rules page as a tab in the Cursor window before falling back to the system browser)

   Existing entries from other tools are preserved byte-for-byte; a backup
   `<file>.<timestamp>.bak` is written only when a file changes. Re-running is
   a no-op. `--harness <list>` limits it; `--project <dir>` also seeds empty
   `<dir>/.<harness>/cosine-rules.json` files.

3. **Start** — the daemon comes up (`rerail start`) unless `--skip-start`.
   Adapters also autostart it on the first session, so this is a convenience.
4. **Report** — prints what changed and reminds you to add `~/.rerail/bin` to
   `PATH` (the Windows launcher is `rerail.cmd`) and restart the harnesses.

`--dry-run` prints all of the above without writing anything. Without
`--harness`, registration touches every harness it detects, so make the first
run a dry run and read what it would change; the real run writes a timestamped
`<file>.<timestamp>.bak` beside every file it edits.

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
rerail disable           # stop Rerail in this checkout without stopping the shared daemon
rerail enable            # resume Rerail in this checkout
rerail status            # checkout disablement, otherwise daemon/database/kernel
rerail ui                # opens the rules page (--print only prints the URL)
rerail session claude-code ID --open  # this session's status line and page URL; --open opens it
rerail scan --session ID --scope repository --wait  # inspect findings in Sessions
rerail settings --session ID --mode log             # save this workspace's default
rerail update check      # is a newer version on the channel?
rerail update            # install it; `rerail update rollback` undoes it
rerail reset             # delete this repository's session history, across all worktrees; rules stay
rerail rules export      # dump global and this project's rules to ~/.rerail/rules.json
rerail install --remove  # take Rerail back out of every harness
```

Inside OMP and Pi, `/rerail-settings` opens an editable settings menu and
`/rerail-scan` chooses full-repository or Git-diff scope; one-shot scans are always log-only.
Both target the current session automatically. OMP, Pi, Claude
Code, and Cursor expose the exact
`/rerail-disable` and `/rerail-enable` names. Slash completion also lists the
native OMP/Pi `/rerail-status`, `/rerail-check`, `/rerail-update`,
`/rerail-rollback`, and `/rerail-reset` commands. These commands take no
arguments. Disablement persists as `.rerail/disabled`; it suppresses recording,
evaluation, steering, and daemon autostart only for that checkout. Existing
data and the shared daemon are left intact. Codex cannot expose an exact
third-party slash-command name; use `rerail disable` / `rerail enable` in its
terminal.
Restart the harness process after installing or rebuilding to load the commands.
Claude Code also retains general `/rerail` guidance at
`~/.claude/commands/rerail.md`; its scan invocation must supply the exact session
identity. See the [scan and settings contract](../README.md#scan-and-settings-controls)
for scope, mode, persistence, diagnostics, and background versus next-event
delivery.

Updates only happen when you ask for one. Nothing in Rerail polls the registry
or installs a version in the background; `rerail update` and re-running
`npx @neralabs/rerail setup` are the only paths. `rerail update --channel <tag>`
selects the npm dist-tag (`latest`, `beta`, or `next`).

Rerail keeps one database, `~/.rerail/rerail.sqlite` (`$RERAIL_HOME/rerail.sqlite`),
holding the rules and the sessions, events, and work links of every repository
on the machine; each session is attributed to its repository (`rerail status`
names the file). The daemon's pages list the repository it was started in.
Every runtime start and `rerail start` seed the rules bundled with the package
without overwriting a rule you edited or created (bootstrap records the digest
each row was seeded from and only refreshes rows that still match it).
`rerail update` first copies the database to `~/.rerail/rerail.sqlite.pre-update`;
`rerail rules export [--output <file>]` and `rerail rules import <file>` move
rules by hand. `rerail seed` fills the database with mock rules to work
against. JSON rule files in
`<project>/.omp|.pi|.claude|.codex|.cursor/cosine-rules.json` (as
`{"version": 1, "rules": []}` plus rules) are also read, but a database row
with the same id wins.

Policy evaluation caches parsed files and results in a content store. Which
one a Rerail uses depends on where its package lives, not on a flag: the
managed installation (a package under `~/.rerail/versions/<version>/`, or
`$RERAIL_HOME/versions/`, normally loaded through `current`) owns the
machine-wide `~/.cache/rerail/content-v3.sqlite`. Any build running from a
Git checkout — source, `dist/`, tests, benchmarks, every worktree — uses
`<checkout>/.rerail/content-v3.sqlite` instead (`.rerail/` is gitignored), so a
checkout on a newer store schema never changes the store the installed version
reads. The file name follows the store's compatibility floor (`v3` today) and
changes whenever the floor rises. `RERAIL_CONTENT_STORE_PATH` overrides the
location for either. A package that is neither (for example an `npx` copy) has
no store and reports that on its first scan.

### One main instance

At most one main Rerail instance runs on a machine: the managed
installation's daemon, the only process that opens the machine-wide content
store. It holds an exclusive OS lock (`flock` / `LockFileEx`) on
`~/.cache/rerail/main-instance.lock` for its whole life and records its
pid, version, package root, and start time beside it in `main-instance.json`.
The OS drops the lock however the daemon ends, `kill -9` included, so there is
never a stale lock to clear. A second main daemon — another `$RERAIL_HOME`, or
an orphan its `daemon.json` no longer names — exits at startup with
`Another main Rerail instance is running: pid …, version …`, which `rerail start`
prints from `daemon.log`; stop that process (`rerail stop`, or end it by pid)
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

### Resetting session history

To clear a repository's recorded sessions, run `rerail reset` from any of its
checkouts (safe to repeat).

It deletes that repository's ended sessions and every session's events, work
links, policy runs, and scan requests in one transaction. Rules, settings, and
other repositories are untouched; the file is never unlinked, so open daemons
and harness sessions keep working, and running sessions keep recording.

### Removing Rerail

To take Rerail off a machine:

```sh
rerail rules export --output ~/rerail-rules.json   # optional: keep your rules
rerail stop
rerail install --remove --dry-run                  # print what would change
rerail install --remove
```

`rerail install --remove` takes out every registration that names this
installation — harness hooks, the Claude Code status line and commands, the
Cursor commands, the Codex skill, the OMP extension symlink and MCP entry, and
the Pi package entry — and leaves other tools' entries alone, writing a
`<file>.<timestamp>.bak` beside every file it changes. Restart open harness
sessions afterwards: their loaded hooks point at files that are about to go.

Then delete the Rerail home (`~/.rerail`, or `$RERAIL_HOME`) and the content
store directory (`~/.cache/rerail`), and remove `~/.rerail/bin` from `PATH`.
The home holds the database, so copy `rerail.sqlite` somewhere first if you
want to keep your rules and history.

## Developing from a checkout

A source checkout builds and tests with Node.js 24+, npm, and Rust; see
[Develop](../README.md#develop) in the README. After `npm run build`,

```sh
node dist/cli.js install --root "$PWD"
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
