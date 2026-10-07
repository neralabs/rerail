# Installing Rerail

This guide tracks `main`; read it from `main` when the checkout is on a feature
branch.

Rerail lands on a machine through one command, `rerail setup`, delivered by
`npx` from the private Azure Artifacts feed. `just rerail` gets the machine the
registry credentials that needs and then runs it. Everything below explains
what that command does, what it needs before it can run, and what is still
undecided about installing without an account. Until the first `rerail-v*` tag
is published, the from-source route under "Developing from a checkout" is the
one that works.

No Rerail path reads the `nera-data-layer` submodule, so `git submodule update`
is not part of installing Rerail and a submodule that will not sync is no reason
to wait.

## One line

From a nera-landing checkout:

```sh
just rerail
```

`just` is the repository's command runner; see "Prerequisites" below.

`just rerail` is a guided installer. It makes sure npm can read
`@neralabs/rerail` from the Azure Artifacts feed (see "Registry access" below),
then runs `npx @neralabs/rerail@latest setup`. Run it again at any time: with
working credentials it goes straight to setup, and with expired ones it
replaces them.

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
   - OMP: symlink `~/.omp/agent/extensions/rerail → current`
   - Pi: `packages` entry in `~/.pi/agent/settings.json`
   - Claude Code: `hooks` in `~/.claude/settings.json` running `current/dist/claude-hook.js` (on Windows as exec-form entries that Claude Code 2.1.139+ spawns without Git Bash: `node` from `PATH` starts `dist/hook-launcher.js`, which prefers the `node.exe` recorded at setup and falls back to the `PATH` node once that binary is gone), the general `/rerail` command, and exact `/rerail-start` / `/rerail-stop` / `/rerail-ui` / `/rerail-session` commands under `~/.claude/commands/` (`/rerail-ui` and `/rerail-session` open the page as a tab in the Orca or Cursor window before falling back to the system browser)
   - Codex: `~/.codex/hooks.json` running `current/dist/codex-hook.js` (Codex asks you to trust it once via `/hooks`; it cannot expose an exact third-party slash-command name, so use the terminal CLI)
   - Cursor: `~/.cursor/hooks.json` running `current/dist/cursor-hook.js`, plus exact `/rerail-start` / `/rerail-stop` / `/rerail-ui` commands under `~/.cursor/commands/` (`/rerail-ui` opens the rules page as a tab in the Cursor window before falling back to the system browser)

   Existing entries from other tools are preserved byte-for-byte; a backup
   `<file>.<timestamp>.bak` is written only when a file changes. Re-running is
   a no-op. `--harness <list>` limits it; `--project <dir>` also seeds empty
   `<dir>/.<harness>/conversation-rules.json` files.

3. **Start** — the daemon comes up (`rerail daemon start`) unless `--skip-start`.
   Adapters also autostart it on the first session, so this is a convenience.
4. **Report** — prints what changed and reminds you to add `~/.rerail/bin` to
   `PATH` (the Windows launcher is `rerail.cmd`) and restart the harnesses.
   An older install's generated `bin/rerail` launcher is deleted; there is no
   `rerail` alias.

Rerail stays off in every repository until you start it there (see "Starting
and stopping Rerail" below).

`--dry-run` prints all of the above without writing anything. Without
`--harness`, registration touches every harness it detects, so make the first
run a dry run and read what it would change; the real run writes a timestamped
`<file>.<timestamp>.bak` beside every file it edits.

## Prerequisites

- [`just`](https://github.com/casey/just), the repository's command runner, for
  every `just` recipe below. Install it with `brew install just` (macOS),
  `cargo install just`, or your distro's package, and run recipes from the repo
  root where the `Justfile` lives.
- Node.js 24 or newer on `PATH`. The launcher remembers the Node that ran setup
  and falls back to `PATH`, so version-manager switches do not break hooks.
- Membership in the `nera-software` Azure DevOps organization. Access comes
  from the `Nera Developers` Entra group (Stakeholder access through a group
  rule); ask Jeremy (jbarnes@nera.software) to add you.
- The [Azure CLI](https://learn.microsoft.com/cli/azure/install-azure-cli)
  (`az`), which `just rerail` uses to sign you in and mint your token.
- macOS, Linux glibc, or Windows MSVC on arm64/x64. The package carries
  prebuilt native kernels for those; nothing is compiled on the user's machine.

## Registry access

`@neralabs/rerail` is published to a private Azure Artifacts feed:

```
https://pkgs.dev.azure.com/nera-software/_packaging/nera-software/npm/registry/
```

`just rerail` settles credentials in this order:

1. If `npm whoami` already succeeds against the feed, it keeps what you have.
2. If `NERA_NPM_TOKEN` is set, it uses that token.
3. Otherwise it runs `az login --tenant nera.software` and mints your own
   organization-scoped, read-only personal access token (Packaging: Read),
   named `rerail-npm-read-<hostname>`, valid for a year or the tenant's
   maximum, whichever is shorter.

If Azure DevOps refuses the request outright (HTTP 401 or 403), you are not
in the organization: it stops and tells you to ask Jeremy
(jbarnes@nera.software) to add you to the `Nera Developers` group. If Azure
DevOps answers but issues no token, it says so, names both causes (not in the
organization yet, so ask Jeremy; or anything else), and continues to the
manual path. The same manual path follows when the Azure CLI is missing, the
sign-in fails, or `az` does not answer within a minute:

1. Open https://dev.azure.com/nera-software/_usersSettings/tokens.
2. Create a token for the `nera-software` organization with the
   **Packaging: Read** scope.
3. Paste it at the hidden prompt `just rerail` shows.

The token is written to `~/.npmrc` (or the file `NPM_CONFIG_USERCONFIG` names)
as one block under a `managed by just rerail` comment: an `@neralabs:registry=`
scope mapping plus a username and base64 `_password` for the feed. Re-running
replaces that block and leaves every other line alone, and the file is made
readable by you only. `just rerail` verifies the credential before it runs setup,
and replaces it only when the registry rejects it; if the registry cannot be
reached, it stops and leaves the saved credential alone.
`rerail update` shells out to `npm`, so the same `~/.npmrc` serves the updater.
When the token expires, `rerail update` and other npm commands report that the
registry credentials are missing or expired; re-run `just rerail` to refresh
them.

`just rerail` needs a terminal for the sign-in and paste steps. Without one,
and without `NERA_NPM_TOKEN`, it prints the steps above and exits non-zero.
For non-interactive use such as CI, set `NERA_NPM_TOKEN` to a Packaging: Read
token; it is used only when the existing credential does not work.
`RERAIL_REGISTRY` points the installer at another registry, for testing against
a local one (as `just rerail-smoke` does); it must be an `https://` URL, or
`http://` on `127.0.0.1`, `localhost`, or `[::1]`. For any registry other than
the Nera feed, the installer never signs in to Azure or mints a token: it uses
`NERA_NPM_TOKEN`, or exits non-zero if that is not set.

`npx @neralabs/rerail setup` is plain Node and runs anywhere once `~/.npmrc` is
in place. On Windows, `just rerail` needs `just`, Node.js, and the Azure CLI on
`PATH`; there is no Windows-native installer yet.

## Anonymous install

Installing without an Azure DevOps account (public npmjs, or any other anonymous
read) is a launch-time decision that has not been made. The options and why
each is on hold are in `docs/distribution-model.md`. Until then, the private
route above is the only install path.

## After setup

The following commands describe the managed installation, once `~/.rerail/bin`
is on `PATH`. Container development uses `just rerail-command <args>` instead;
it does not replace the managed launcher.

```sh
rerail start             # start Rerail in this repository (first time: also sets it up)
rerail stop              # stop Rerail in this repository; the daemon keeps serving others
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
rerail rules export      # dump global and this project's rules to ~/.rerail/rules.json
rerail install --remove  # take Rerail out of every coding agent, stop the daemon, back up the database
```

Inside OMP and Pi, `/rerail-settings` opens an editable settings menu and
`/rerail-scan` chooses full-repository or Git-diff scope; one-shot scans are always log-only.
Both target the current session automatically. Slash completion also lists the
native OMP/Pi `/rerail-status`, `/rerail-check`, `/rerail-update`,
`/rerail-rollback`, and `/rerail-reset` commands. These commands take no
arguments.
Restart the coding agent after installing or rebuilding to load the commands.
Claude Code also retains general `/rerail` guidance at
`~/.claude/commands/rerail.md`; its scan invocation must supply the exact session
identity. See the [scan and settings contract](../README.md#scan-and-settings-controls)
for scope, mode, persistence, diagnostics, and background versus next-event
delivery.

Updates only happen when you ask for one. Nothing in Rerail polls the registry
or installs a version in the background; `rerail update` (and `just rerail` from
a checkout, which re-runs setup against the registry) are the only paths.

Rerail keeps one database, `~/.rerail/rerail.sqlite` (`$RERAIL_HOME/rerail.sqlite`),
holding the rules and the sessions, events, and work links of every repository
on the machine; each session is attributed to its repository (`rerail status`
names the file). The daemon's pages list the repository it was started in.
Every runtime start and `rerail daemon start` seed the rules bundled with the package
without overwriting a rule you edited or created (bootstrap records the digest
each row was seeded from and only refreshes rows that still match it).
`rerail update` first copies the database to `~/.rerail/rerail.sqlite.pre-update`;
`rerail rules export [--output <file>]` and `rerail rules import <file>` move
rules by hand. `rerail seed` fills the database with mock rules to work
against. JSON rule files in
`<project>/.omp|.pi|.claude|.codex|.cursor/conversation-rules.json` (as
`{"version": 1, "rules": []}` plus rules) are still read, but a database row
with the same id wins.

Policy evaluation caches parsed files and results in a content store. Which
one a Rerail uses depends on where its package lives, not on a flag: the
managed installation (a package under `~/.rerail/versions/<version>/`, or
`$RERAIL_HOME/versions/`, normally loaded through `current`) owns the
machine-wide `~/.cache/nera/rerail/content-v3.sqlite`. Any build running from a
Git checkout — source, `dist/`, tests, benchmarks, every worktree, and the
`just rerail-dev` container — uses `<checkout>/.rerail/content-v3.sqlite`
instead (`.rerail/` is gitignored), so a checkout on a newer store schema never
migrates the store the installed version reads. The file name follows the
store's compatibility floor (`v3` today) and changes whenever the floor rises.
`RERAIL_CONTENT_STORE_PATH` overrides the location for either. A package that
is neither (for example an `npx` copy) has no store and reports that on its
first scan.

### Starting and stopping Rerail

Rerail is off in every repository until it is started there. Set-up and rules
belong to the repository; on and off belong to each checkout, stored in
`~/.rerail/rerail.sqlite`. A linked worktree follows the nearest checkout of its
repository that contains it and was turned on or off, at any depth, and
otherwise the main checkout, until it is started or stopped itself.

Starting or stopping a checkout also reaches every worktree that follows it:
the worktrees inside it without their own setting, and, from the main
checkout, every linked worktree without its own setting, wherever it is on
disk. Worktrees with their own setting, and checkouts that contain this one,
keep their state.

- `/rerail-start` (OMP, Pi, Claude Code, Cursor) or `rerail start` (terminal,
  and the only spelling in Codex, which cannot expose an exact third-party
  slash-command name) starts Rerail in the current checkout. In a
  repository that has never been set up, it also does the first-time set-up
  and seeds the default rules.
- `/rerail-stop` or `rerail stop` stops Rerail in the current checkout: no
  recording, evaluation, steering, or daemon autostart from its sessions.
  Recorded history, rules, and the shared daemon stay, and other repositories
  are unaffected.

`rerail daemon start` and `rerail daemon stop` control the background daemon
itself, which serves every started repository on the machine.

Upgrading keeps everyone where they were: a repository with recorded history
counts as started, and a checkout that carried the old `.rerail/disabled` marker
counts as stopped (in a linked worktree, that worktree only). The marker is not
read after the upgrade; delete it whenever you like.

### One main instance

At most one main Rerail instance runs on a machine: the managed
installation's daemon, the only process that opens the machine-wide content
store. It holds an exclusive OS lock (`flock` / `LockFileEx`) on
`~/.cache/nera/rerail/main-instance.lock` for its whole life and records its
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

### Upgrading from two databases

Releases before the single database kept rules in `~/.rerail/rules.sqlite` and
each checkout's history in `<checkout>/.rerail/transcripts.sqlite`. The first
start of this release creates `~/.rerail/rerail.sqlite` as a consistent copy of
`rules.sqlite`, so every rule and project assignment carries over, and leaves
`rules.sqlite` in place without reading it again. Checkout
`transcripts.sqlite` files are left untouched and their history is not
imported. The custom rules a checkout file holds are imported only when Rerail
starts in that checkout, so start Rerail once in each checkout (or keep a backup
of the file) before you delete it; after that, delete it yourself once you no
longer need its history.

### Resetting session history

To clear a repository's recorded sessions, run from any of its checkouts
(safe to repeat):

```sh
rerail reset             # managed installation
just rerail-reset        # running checkout-local development container
```

It deletes that repository's ended sessions and every session's events, work
links, audits, and audit requests in one transaction. Rules, settings, and
other repositories are untouched; the file is never unlinked, so open daemons
and harness sessions keep working, and running sessions keep recording.

### Removing Rerail and restoring its databases

`rerail install --remove` takes Rerail out of every coding agent, stops the
daemon, and moves `rerail.sqlite` (with its `-wal`/`-shm`, and a legacy
`rules.sqlite`) into `~/.rerail-clean-YYYYMMDD/rerail-home/`
(`-YYYYMMDD-HHMMSS` when one from today exists), so your rules and history
are kept in the same layout `just rerail --restore` reads.

To take Rerail off a machine completely (the `just rerail` install and any
`just rerail-dev --install-bridge` bridge), keeping its data in a backup:

```sh
just nuke-rerail --dry-run    # print what would be removed; change nothing
just nuke-rerail              # asks y/N first; --yes skips the prompt
just rerail --restore         # reinstall, then restore the newest backup's databases
just rerail-dev --restore     # or run a checkout's container with the backed-up rule catalog
```

`just nuke-rerail` stops the daemon and every running Rerail dev container on
the machine (the `rerail` containers `just rerail-dev` started in any checkout;
Docker absent or not running is skipped). Containers are only stopped, never
removed, and their volumes stay, so the rule catalog in each container's
`rerail_home` volume survives and `just rerail-dev` in that checkout starts it
again. It then backs up and edits every harness
config Rerail writes: hooks pointing into `~/.rerail` (or `$RERAIL_HOME`) leave
`~/.claude/settings.json`, `~/.cursor/hooks.json`, and `~/.codex/hooks.json`;
the Claude status line reverts to the one recorded before Rerail registered
(Claude project settings that Rerail composed a status line into are handled
the same way); the `rerail@rerail-dev` plugin and `rerail-dev` marketplace leave
Claude settings; the `rerail` MCP server leaves `~/.omp/agent/mcp.json`; Rerail
packages leave `~/.pi/agent/settings.json`. It moves the Rerail slash commands,
the Codex `rerail` skill, and the whole Rerail home into the backup, removes the
OMP extension symlink, and deletes the `@neralabs` scope mapping and
`just rerail` credential block from `~/.npmrc` (not backed up), so the next
`just rerail` signs you in again. Restart open harness sessions afterwards:
their loaded hooks point at files that are gone.

The backup is `~/.rerail-clean-YYYYMMDD` (`-YYYYMMDD-HHMMSS` when one from
today exists):

| Path | Holds |
|------|-------|
| `configs/` | Each harness config as it was, named by its path under your home with `/` as `_` (`.claude_settings.json`) |
| `claude-commands/`, `cursor-commands/` | The moved `/rerail*` slash commands |
| `codex-skill-rerail/` | The moved Codex skill |
| `rerail-home/` | The whole Rerail home: databases, versions, bridges, logs |

`--restore [dir]` uses the named backup, or the newest `~/.rerail-clean-*`
holding a `rerail-home/`. After the normal install succeeds it stops the
daemon, moves the database it is about to replace (with its `-wal`/`-shm`)
aside to `<name>.pre-restore-<timestamp>`, copies in the backup's database
with its write-ahead log already folded in, starts the daemon again on the
same port, and checks `/healthz`. `just rerail --restore` restores
`rerail.sqlite` (every rule and every repository's history), `repository-id`
(the home directory's rule scope), and `sessions/` (that directory's session
ledgers). `just rerail-dev --restore` restores only `rerail.sqlite`, into the
container's Rerail home volume. A backup made before the single database holds
`rules.sqlite` instead: both commands restore it with no `rerail.sqlite` beside
it, so the restarted daemon copies it in as on any upgrade; that backup's
`transcripts.sqlite` is not restored. Neither restores `harness.sqlite`,
`versions/`, `bridges/`, or `daemon.json`; the fresh install owns those.

## Developing from a checkout

`just rerail` still installs the published package. `just rerail-dev` instead
builds and runs this checkout in its own Linux container:

```sh
just rerail-dev                    # provision, build, start, and print this checkout's URL
just rerail-dev --install-bridge   # explicitly install the portable host bridge once
just rerail-status
just rerail-command settings --session ID --json
just rerail-reset                  # delete this repository's session history (all worktrees) in its container; rules stay
just rerail-down                   # stop only this Rerail; preserve its state
```

The host needs Docker with Compose v2, Git, Bash, Python 3, Node.js 24+, and
`just`. On Windows, run these development commands from a Linux checkout in
WSL2 with Docker Desktop integration. Rust and the Rerail kernel build stay in
the container. No initialized data-layer submodule or registry token is
needed.

`just rerail-setup` provisions without starting. It records explicit development
activation, the worktree identity, one distinct reserved `NERA_RERAIL_HOST_PORT`,
and the matching loopback `NERA_RERAIL_PUBLIC_URL` in `.nera/instance.env`.
Compose maps that public host port to private container port 4737. Existing Rerail
routing fields are reconciled without changing application ports or persisted
`.rerail` state.
Installed or otherwise non-worktree Rerail continues to default to
`http://127.0.0.1:4737`; reserved workspace origins do not change that default.
Ordinary application setup does not activate Rerail development routing. Once
opted in, `just up` also starts Rerail; `just status` and `just down` include it.
`just clean` requires confirmation and removes this checkout's containers, build
volumes, and `.rerail` state, not sibling resources.

Each checkout owns its container home, rule catalogs, transcripts, content
store, native build cache, dependencies, and generated files. Project data,
including the checkout's content store, remains in `<checkout>/.rerail`;
container home data is separate from the real `~/.rerail`. Git common metadata is
mounted read-only, and named volumes hide host dependency/output directories.

`--install-bridge` copies the container's portable bridge to
`~/.rerail/bridges/<hash>`, points the `~/.rerail/dev-bridge` symlink at it, and
replaces the managed `~/.rerail/current` harness hooks with hooks that name the
bridge's `dist/` directory. There is no `--remove-bridge`. To undo it, remove
the bridge's hooks and register the managed install again:

```sh
rerail install --root "$(readlink ~/.rerail/dev-bridge)" --remove
rerail install
```

The first line strips every hook that names the bridge and leaves the bridge
files in place; the second writes hooks for `~/.rerail/current` again. Restart
harness sessions afterwards.

Rerun `just rerail-dev` after source edits: it rebuilds only this checkout's
container and continues serving its reserved public origin. Other enabled
workspaces may remain running on their own reserved origins. A launcher may stop
a managed daemon only when its persisted state and health prove the exact same
workspace and database; foreign or unverified owners are refused, and unrelated
workspaces are never stopped.
Restart this worktree's native harness sessions after a rebuild: their native
registries and cached semantic selections live in the replaced container.

Stop the container with `just rerail-down` before running a host-side Rerail
build or `npm --prefix rerail run test:all`. Docker Desktop holds the output
mountpoints while the container runs, so a host `build:clean` cannot remove
`rerail/dist` until it stops. This does not affect builds in sibling worktrees.

The explicit `--install-bridge` step copies JavaScript-only, content-addressed
artifacts under `~/.rerail/bridges/`, updates `~/.rerail/dev-bridge`, and registers
OMP, Pi, Claude Code, Codex, and Cursor. Restart harness sessions afterwards.
The bridge routes by the session/command working directory to the nearest
explicitly enabled checkout. Native operations run in that container. A stopped
development container produces an error; it never falls back to the shared daemon.
Ordinary checkouts continue using the managed installation, when present.

Normal startup, reset, and rebuild never rewrite the host bridge, harness
registrations, or the managed installation. If you change bridge protocol or
host adapter code, explicitly reinstall the bridge and restart the affected
harness sessions. There is no implicit global registration switch during the
worktree development loop.

`python3 scripts/rerail-container-smoke.py` exercises two real sibling workspaces
concurrently through their distinct reserved public Rerail origins. It checks
distinct builds and catalogs, native OMP/Pi operations, all three command hooks,
MCP, maintenance, reset, persistence, rebuild, exclusive same-database ownership,
refusal to stop foreign or unverified daemons, and scoped cleanup. It uses a
sandbox home, verifies host build artifacts remain untouched, and writes
diagnostics under `.nera/artifacts/rerail-container-smoke/`. The PR workflow runs
it as **Rerail worktree isolation**.

`just rerail-smoke` remains the separate offline managed-installer check: it
publishes to a throwaway local registry and exercises setup/update/removal in
a sandbox home.

`just rerail-dev` runs the checkout's daemon in its per-worktree container with
`RERAIL_DEV=1`, so it serves the committed page straight from the bind-mounted
`rerail/ui`: edit a file, reload the browser, no rebuild and no restart. API/MCP
and the database stay in the container, and private container port 4737 remains
hidden and is allocated so concurrent workspaces do not collide. Open the
printed workspace URL at `/#rules`. See
[the README](../README.md#container-development-loop).

## What is not automated yet

- Publishing the first version (push a `rerail-v*` tag once the release
  workflow is on `main`; tracked in Linear).
- Activation (`rerail activate`) is optional until the control plane exists.
- A service unit (launchd/systemd) so the daemon survives reboots; today
  adapters restart it on the next session.
