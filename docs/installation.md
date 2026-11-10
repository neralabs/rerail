# Installing Rerail

One command in your repository installs Rerail under your home folder, connects your coding agents and, once you say
yes, starts Rerail there.

Details: [How set-up works](how-setup-works.md). Problems: [Troubleshooting](troubleshooting.md). From source:
[Developing from a checkout](how-setup-works.md#developing-from-a-checkout).

## Requirements

Set-up checks these first and changes nothing if one is missing.

- **Node.js 24** or newer.
- **Git** on your `PATH`; Rerail reads every repository through it.
- **macOS, Linux with glibc 2.28 or newer, or Windows**, on arm64 or x64 (no Alpine or other musl Linux).
- **npmjs** access, with no npm account.

<details>
<summary>Platform and registry notes</summary>

When a requirement is missing, set-up stops and says what to do. Native engines ship prebuilt, so nothing is compiled
on your machine. [Supported and not supported](#supported-and-not-supported) has the full platform list.
[Registry access](#registry-access) covers npmjs, the only registry Rerail is published to, and mirrors.

</details>

## Install

Run set-up in your repository; it turns nothing on until you say yes.

```sh
npx @neralabs/rerail setup
```

It installs Rerail under `~/.rerail`, without admin rights, and asks:

1. **Read sessions?** "No" stops set-up with Rerail installed and off.
2. **Read this repository?** Or skip the scan and pick rule packs yourself.
3. **Start?** Accept the recommended rule packs, change them, or answer "Not now".

<details>
<summary>What each question covers</summary>

1. Asked once per computer: may Rerail read your coding agents' sessions (what you ask, what they do, the files they
   edit) to check them against your rules?
2. Outside a repository it asks which folder to start with, or lets you skip; `--repo <folder>` names one. If you skip
   the scan and pick rule packs yourself, no code is read.
3. It shows the recommended rule packs, each with why it matched, or why none fits, and the coding agents it will
   connect. "Not now" installs, connects and turns on nothing.

</details>

On yes, it connects your coding agents and starts Rerail in this repository only. Start a new coding agent session
there; `rerail manage` opens the dashboard.

<details>
<summary>Agents, answering and relay mode</summary>

Set-up connects the coding agents it finds (Claude Code, Codex, Cursor, OMP and Pi), or only those named with
`--agents`. It ends with a summary and a dashboard link. Open `rerail manage` in a new terminal.

In a terminal, answer with ↑/↓ and Enter, or type the answer's number. To have a coding agent install Rerail, ask it:
it runs `setup --relay` and asks you each question in the session.

</details>

### Running set-up again

Set-up is idempotent, so rerunning it doubles as an upgrade.

<details>
<summary>What a rerun connects</summary>

In a repository already set up, it changes nothing there. It connects only coding agents installed since set-up or
`rerail install` last looked.

It skips agents you left out with `--agents` or removed with `rerail install --remove --agents <id>`. Name one to connect
it anyway, for example `npx @neralabs/rerail setup --agents codex`.

</details>

## What it changes on your machine

Set-up writes to three places outside the repository, and to nothing before you say yes.

- **`~/.rerail`**: the package, the `rerail` launcher and your rules and history.
- **Your shell's startup file**, so `rerail` runs in new terminals.
- **Your coding agents' user settings**: hooks or an extension, commands and denies.

<details>
<summary>Exact files and entries</summary>

- **`~/.rerail`** (or `RERAIL_HOME`): the package, the `rerail` launcher in `~/.rerail/bin`, the database of your rules
  and session history (`rerail.sqlite`), and the daemon's files. The cache is `~/.cache/rerail`.
- **Shell startup file**: a block between `# >>> rerail >>>` and `# <<< rerail <<<` on macOS and Linux, or the user
  `Path` on Windows. `RERAIL_MODIFY_PATH=0` leaves `PATH` to you.
- **Coding agent settings** in `~/.claude`, `~/.codex`, `~/.cursor`, `~/.omp/agent` and `~/.pi/agent`. The denies keep
  the agent out of Rerail's data. Other tools' entries stay; each changed file gets a `<file>.<timestamp>.bak` backup
  beside it.

Nothing in the repository, its Git metadata or the database changes before you say yes.

</details>

[How set-up works](how-setup-works.md) lists every step, file and entry.

## Other ways to install

Pi and OMP can install Rerail through their own package managers; set-up then takes over.

### From Pi's package gallery

`pi install npm:@neralabs/rerail` works; Rerail is in Pi's [package gallery](https://pi.dev/packages). The next Pi
session loads Rerail and says to run `/rerail-start`, which finishes set-up.

<details>
<summary>How the managed install takes over</summary>

`/rerail-start` also replaces the `npm:@neralabs/rerail` entry in `~/.pi/agent/settings.json`, in place, with the managed
install, `~/.rerail/current`. Pi then loads one copy, and `rerail update`, not `pi update`, governs the version. A gallery
install made after set-up is swapped the same way, by `/rerail-start` again or the next `rerail update`.

</details>

### From OMP's plugins

After set-up, run `omp plugin uninstall @neralabs/rerail` if you used `omp plugin install @neralabs/rerail`, then
restart OMP. Set-up cannot replace OMP's copy, so both would load; set-up and `rerail update` remind you while it stays.

## Update

Rerail updates only when you ask; nothing polls the registry or installs a version in the background.

```sh
rerail update check   # is a newer version on the channel?
rerail update         # install it; /rerail-update in a coding agent ($rerail-update in Codex)
```

Rerunning `npx @neralabs/rerail setup` also updates.

<details>
<summary>Channels and what an update does</summary>

- `rerail update --channel <tag>` selects the npm dist-tag: `latest`, `beta` or `next`.
- An update first copies the database to `~/.rerail/rerail.sqlite.pre-update`.
- The new version applies to coding agent sessions started afterwards; connected coding agents get its commands.

</details>

Details: [Updates](how-setup-works.md#updates).

## Start and stop

Rerail is off in every repository until started there; stopping keeps your rules and history.

- **`/rerail-start` or `rerail start`** (`rerail start` in Codex) starts it in this checkout.
- **`/rerail-stop` or `rerail stop`** stops it here: nothing is recorded or checked.

<details>
<summary>Unset repositories, all checkouts and the daemon</summary>

- In a repository not set up, starting asks set-up's questions first.
- **`--repository`** on either command acts on every checkout of the repository.
- **`rerail daemon start` / `rerail daemon stop`** control the one background daemon serving every repository.

</details>

How checkouts and worktrees inherit this: [Rule scoping](rule-scoping.md#start-and-stop).

## Remove

Removal keeps your rules and history unless you ask to delete them.

```sh
rerail rules export --output ~/rerail-rules.json   # optional: a portable copy of your rules
rerail install --remove --dry-run                  # print what would change
rerail install --remove                            # or /rerail-uninstall in a coding agent
```

Restart open coding agent sessions afterwards. `--delete-data` also deletes your database; that cannot be undone.

<details>
<summary>What removal keeps and deletes</summary>

`rerail install --remove` takes Rerail out of every coding agent and off `PATH`, backing up each file it changes and
leaving other tools' entries alone. It stops the daemon and moves your database into `~/.rerail-clean-YYYYMMDD/`.

It then deletes, naming each path, `~/.rerail/current`, the launcher, every version, the rule packs and the cache
`~/.cache/rerail`.

`rerail install --remove --delete-data` deletes the database, its backups, the cache and the checkouts' session ledgers
instead of keeping them. Nothing it deletes can be restored.

</details>

See [Removing Rerail in detail](how-setup-works.md#removing-rerail-in-detail).

## Supported and not supported

In short: macOS, glibc Linux and Windows 11 on arm64 or x64, with Node.js 24+, for Claude Code, Codex, Cursor, OMP and Pi.

<details>
<summary>Supported</summary>

- **macOS** on arm64 and x64, including x64 Node under Rosetta.
- **Linux** on arm64 and x64 with glibc 2.28 or newer: every distribution Node 24 runs on (Debian 10+, Ubuntu 20.04+,
  RHEL 8+ and rebuilds, Amazon Linux 2023, Fedora).
- **Windows 11** on x64 and arm64, including profile folder names with a space or non-ASCII letter.
- **Node.js 24** or newer, however installed (nodejs.org, Homebrew, nvm, fnm, Volta, NodeSource).
- **Git** on `PATH`, or in the usual install locations when `PATH` has none.
- **Shells** set-up puts `rerail` on `PATH` for: zsh, bash and fish (interactive; `zsh -c` and ssh need the full path
  or `~/.zshenv`); any other shell via `~/.profile`; on Windows, PowerShell, Command Prompt and Git Bash via the user
  `Path`.
- **Coding agents**, each found by its config folder in your home folder (`~/.claude`, `~/.codex`, `~/.cursor`,
  `~/.omp/agent`, `~/.pi/agent`; on Windows, the profile folder, `USERPROFILE`). Run `rerail setup` again to connect
  one installed later.
  - Claude Code: the CLI, the VS Code extension (its panel shows no status line) and the JetBrains plugin; 2.1.139 or
    newer on Windows.
  - Codex CLI; trust the hook once with `/hooks`.
  - Cursor's IDE agent and its CLI (`cursor-agent`).
  - OMP and Pi.
- **Containers and devcontainers** with a glibc image, including the default `node:24` and `node:24-slim`, with or
  without an init process.
- **WSL 2**, as Linux, with the coding agent running in WSL too: install Rerail on the same side as the coding agent. A
  repository on the Windows drive (`/mnt/c/…`) works but is slow from WSL; set-up says so.
- **A registry mirror or proxy** set in `~/.npmrc`.

</details>

<details>
<summary>Not supported</summary>

- Linux with glibc older than 2.28 (Amazon Linux 2, Ubuntu 18.04, CentOS 7), where Node 24 does not run either. Alpine
  and other musl distributions (`node:24-alpine`). Other operating systems and processors.
- Node.js 23 or older.
- No Git, or Git only inside the repository or a `node_modules` folder.
- Other coding agents and editors: GitHub Copilot, Windsurf, Gemini CLI, Cline, Roo Code, Zed's agent, Aider, Continue,
  OpenCode, Amp and the rest. Cursor's cloud agents, which run no user hooks.
- Agent config folders moved with `CLAUDE_CONFIG_DIR` or `CODEX_HOME`. A config file that is not strict JSON (comments,
  trailing commas) is left alone; that coding agent stays unconnected until the file is fixed.
- A coding agent on Windows with Rerail in WSL, or the other way round.
- Installing offline: set-up, `rerail update` and rule packs need the npm registry.

</details>

## What a coding agent can reach

A coding agent runs commands as you, so only its own sandbox can keep it out of Rerail's data.

Set-up adds denies where it can and never turns a sandbox on or off. `rerail status` shows whether each connected
agent's tools can read Rerail's data.

<details>
<summary>Per-agent coverage</summary>

Rerail's data is `~/.rerail` (the dashboard's access token, your rules and history) and `~/.cache/rerail`. Set-up adds
denies for both where the agent's settings have a place for them.

- **Claude Code**: file tools always kept out; commands only with its sandbox on (`/sandbox`).
- **Cursor CLI**: file tools kept out.
- **Codex and Cursor**: default sandboxes stop writes, not reads.
- **OMP and Pi**: no sandbox.

Settings → Coding agents in the dashboard shows the same as `rerail status`, for this computer, from each agent's
current settings. For Claude Code, that is whether `sandbox.enabled` is on in your user or managed settings.

</details>

[Sandbox and permission entries](how-setup-works.md#sandbox-and-permission-entries) lists every entry.

## Registry access

Rerail is published only to npmjs, as a public package, so no account or sign-in is needed.

Set-up and `rerail update` use your npm configuration, so a mirror or proxy in `~/.npmrc` works; a repository's own
`.npmrc` does not apply to updates. [Updates](how-setup-works.md#updates) covers expired tokens and signature checks.
