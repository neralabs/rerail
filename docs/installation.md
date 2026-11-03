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

If you installed it with `omp plugin install @neralabs/rerail`, run `omp plugin uninstall @neralabs/rerail` after
set-up and restart OMP. Set-up cannot replace OMP's own plugin copy, so both would load; set-up and `rerail update`
remind you while it stays.

## Update

Updates happen only when you ask for one: nothing in Rerail polls the registry or installs a version in the background.

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
