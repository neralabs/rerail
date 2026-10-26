# Troubleshooting

Each entry starts with what you see, then why, then what to do. What Rerail runs on is listed under
[Supported and not supported](installation.md#supported-and-not-supported); what set-up writes, and how to remove
it, is in [Installing Rerail](installation.md).

## Installing

### Node is too old

```
✕ Rerail needs Node 24 or newer. You have Node 22.23.4.
  Nothing was changed on this machine.
  Upgrade Node, for example:  nvm install 24
```

Rerail runs on Node 24 or newer. Set-up checks before it writes anything. Install Node 24 the way you installed Node
(`brew install node`, `nvm install 24`, `fnm install 24`, `volta install node@24`, nodejs.org, or NodeSource on
Linux; the `nodejs` package of Ubuntu 24.04 is Node 18), then run the same command again. npm prints
`npm warn EBADENGINE Unsupported engine` lines before this message; they say the same thing.

Until Rerail's first stable release, npm 10 (Node 20 and 22) stops earlier with
`npm error notarget No matching version found for @neralabs/rerail@*.`, and with `engine-strict=true` in `~/.npmrc`
npm stops with `npm error code EBADENGINE`. Both mean the same: install Node 24.

### Linux: `GLIBC_2.39' not found`

```
rerail: Installed native runtime health check failed: …
Error: /lib/x86_64-linux-gnu/libc.so.6: version `GLIBC_2.39' not found (required by …/native/rerail_engine.linux-x64-gnu.node)
```

Rerail's Linux engine needs glibc 2.39 or newer. Debian 12, Ubuntu 22.04, RHEL 9 and its rebuilds, and Amazon Linux
2023 have an older one, and so do the default `node:24` and `node:24-slim` container images (Debian 12). Check with
`ldd --version`. Use Ubuntu 24.04 or newer, Debian 13 or newer, or Fedora 40 or newer; in containers, an image such
as `node:24-trixie-slim`.

### Linux: `Error loading shared library ld-linux-…`

```
Error: Error loading shared library ld-linux-aarch64.so.1: No such file or directory (needed by …/native/rerail_engine.linux-arm64-gnu.node)
```

This is a musl distribution, such as Alpine (`node:24-alpine`). Rerail ships only glibc engines for Linux. Use a glibc
distribution or image (see above).

After either Linux failure, set-up leaves an empty `~/.rerail/versions/`. If Rerail was never installed on this
machine, delete `~/.rerail`.

### Git is missing

```
rerail: Installed CLI could not start a health-check repository: … rerail: Rerail has not recorded /tmp/rerail-update-health-…/workspace.
```

Rerail needs Git, and it does not search `PATH` for it: it looks only at `/usr/bin/git`, `/usr/local/bin/git` and
`/opt/homebrew/bin/git`, or on Windows at `C:\Program Files\Git\cmd\git.exe` and
`C:\Program Files (x86)\Git\cmd\git.exe`. Install Git from your distribution (`apt-get install git`,
`dnf install git`), Homebrew, or the Xcode Command Line Tools (`xcode-select --install`), or on Windows Git for Windows
for all users in its default folder. A Git that lives only elsewhere (Nix, Linuxbrew, scoop, a per-user Git for
Windows) is not found. Then run set-up again.

### Windows PowerShell: `npx.ps1 cannot be loaded`

```
npx : File C:\Program Files\nodejs\npx.ps1 cannot be loaded because running scripts is disabled on this system.
```

PowerShell's execution policy blocks npm's `npx.ps1` before Rerail runs. Run `npx.cmd @neralabs/rerail setup`
instead, run the command in Command Prompt, or allow local scripts for your account:
`Set-ExecutionPolicy -Scope CurrentUser RemoteSigned`. The `rerail` command set-up installs is `rerail.cmd`, which the
policy does not block.

### Set-up prints nothing for a long time

The first `npx @neralabs/rerail setup` downloads and installs about 300 MB, and prints nothing for 20 to 60 seconds
before Rerail's first line. Much longer than that usually means npm cannot reach the registry (below).

### npm cannot reach the registry

```
npm error code ECONNREFUSED
npm error network If you are behind a proxy, please make sure that the
npm error network 'proxy' config is set properly.  See: 'npm help config'
```

Set-up and `rerail update` run your own `npm`, with your `~/.npmrc`. Check the network, and npm's `registry`,
`proxy`, `https-proxy` and `cafile` settings (`npm config list`). A registry mirror set in `~/.npmrc` works for Rerail
and its rule packs. Rerail cannot be installed offline.

When npx already has Rerail in its cache, set-up can start and then sit silent for many minutes while npm retries
each package. Press Ctrl-C, fix the network, and run it again. If set-up then says
`Another Rerail update is already running` and no other set-up is running, delete `~/.rerail/update.lock`.

If the registry goes away during the scan, set-up names each pack it could not fetch, and the headline says none
fit, which here means they could not be downloaded:

```
Scanned app. None of Rerail’s rule packs fit it.
  Rule packs set-up could not use:
    @neralabs/rerail-pack-javascript: npm could not reach the registry (ECONNREFUSED)
```

Fix the network and run `rerail setup` again rather than starting with no rule packs.

## The `rerail` command

### `rerail: command not found` right after set-up

Set-up puts `~/.rerail/bin` on `PATH` for terminals opened after it ran, and says so (`You can run rerail from any
new terminal`, or from a coding agent `Added ~/.rerail/bin to PATH in ~/.zshrc. Open a new terminal to run rerail.`).
Open a new terminal. Until then, run `~/.rerail/bin/rerail`, or `export PATH="$HOME/.rerail/bin:$PATH"`.

- **Windows**: set-up adds the folder to your user `Path`. A terminal inside VS Code, Cursor or another editor, and
  anything started from it (a coding agent's shell included), keeps the `Path` the editor started with: quit the
  editor completely and open it again. Meanwhile run `%USERPROFILE%\.rerail\bin\rerail.cmd`. In Git Bash, type
  `rerail.cmd`.
- **zsh scripts and ssh**: the line goes into `~/.zshrc`, which only interactive zsh reads, so `zsh -c '…'` and
  `ssh host rerail …` do not find `rerail`. Use `~/.rerail/bin/rerail` there, or add
  `export PATH="$HOME/.rerail/bin:$PATH"` to `~/.zshenv`.
- **A different shell**: set-up writes the startup file of `$SHELL`, else of your account's login shell. If you use
  another shell, or set `RERAIL_MODIFY_PATH=0`, add `~/.rerail/bin` to `PATH` yourself.
- **Containers**: `docker exec … bash -c` reads no startup file. Use `bash -l`, or the full path.

### Node was upgraded or removed after set-up

```
sh: /opt/homebrew/Cellar/node/26.5.0/bin/node: No such file or directory
```

or `…/.rerail/bin/rerail: line 5: exec: node: not found`, or a coding agent's hooks failing with
`exec: node: not found`.

Set-up records the full path of the Node that ran it, for the launcher, the hooks and Claude Code's status line.
Upgrading or uninstalling that Node (`brew upgrade node`, `nvm uninstall`) removes the path. The launcher and the
hooks then use the first `node` on `PATH`, whatever its version; the Claude Code status line has no such fallback.
Make sure Node 24 or newer is on `PATH`, then run `rerail install --refresh`: it records the new Node in every
connected coding agent's hooks and status line. The launcher keeps the old path and goes on using `node` from `PATH`.

## Coding agents

### Codex: nothing happens

Codex runs only hooks you have trusted, and set-up does not say so (`rerail install` does). Open Codex, run `/hooks`,
trust the `rerail` hook, then start a new session. In Codex the commands are skills, typed `$rerail`,
`$rerail-show-me` and so on; set-up's summary spells them `/rerail-…`.

### `Cannot parse JSON in …/settings.json`

```
rerail: Cannot parse JSON in /home/me/.claude/settings.json: Expected property name or '}' in JSON at position 2 (line 1 column 3)
```

Rerail reads and writes coding agent config files as strict JSON. When one does not parse, set-up and
`rerail install` stop before changing any of them: no coding agent is connected and the repository stays off.
(`rerail update` and `rerail install --refresh` leave that one agent alone and say so.) Remove the comments and
trailing commas from the file named, then run the same command again. To go ahead without that agent, name the
others: `rerail setup --agents cursor`.

### A coding agent installed after set-up is not connected

Set-up connects the coding agents whose config folder exists (`~/.claude`, `~/.codex`, `~/.cursor`, `~/.omp/agent`,
`~/.pi/agent`). Running set-up again in a repository that is set up connects no new one. Run the agent once so its
folder exists, then run `rerail install` (every agent found) or `rerail install --agents codex`. Open a new session
in that agent afterwards.

### `Unknown coding agent`

```
rerail: Unknown coding agent: copilot. Rerail works with omp, pi, claude-code, codex, cursor.
```

Rerail connects only those five. GitHub Copilot, Windsurf, Gemini CLI and the others are not supported.

### Rerail does not see Claude Code or Codex

Rerail writes only to `~/.claude` and `~/.codex`. With `CLAUDE_CONFIG_DIR` or `CODEX_HOME` set elsewhere, the agent
never reads what Rerail wrote. Unset them, or point them at the default folders.

### Cursor also runs Claude Code's hooks

With Claude Code and Cursor both connected, Cursor's Third-Party Imports setting (on by default) makes Cursor run
the hooks in `~/.claude/settings.json` too. Rerail's Claude Code hook ignores the events Cursor sends it, so each
Cursor chat is recorded once, as a Cursor session. Nothing to do.

### A session open before set-up shows nothing

Coding agents load hooks and commands when a session starts. Open a new session after set-up, `rerail install`,
`rerail update` and `rerail install --remove`.

## Daemon and dashboard

### The dashboard URL changed

Rerail's daemon listens on `127.0.0.1:4737`. When another program holds that port, it takes a free port instead, a
different one each time it starts, so an old dashboard link stops working. `rerail manage --print` prints the
current URL (`/rerail-manage` in a session opens it). For a URL that stays, free port 4737. `rerail daemon start
--port <n>` picks a port for that run only; when that port is taken too, it fails with
`listen EADDRINUSE: address already in use 127.0.0.1:<n>`.

### `Another main Rerail instance is running`

A second daemon, from another `RERAIL_HOME` or one left behind, holds Rerail's machine-wide lock. Stop it
(`rerail daemon stop` from its home, or end that process by its pid), then start again.

### Containers: `rerail daemon stop` never returns

In a container without an init process (the usual `sleep infinity` devcontainer or compose service), the stopped
daemon is never reaped, and `rerail daemon stop` waits for it forever. The daemon is already down: press Ctrl-C, and
`rerail status` says it is not running. Run the container with an init: `docker run --init`, `"init": true` in
`devcontainer.json`, `init: true` in compose.

### Containers: the dashboard does not open from the host

The daemon listens on the container's loopback only, so a published port (`-p 4737:4737`) gets an empty reply, and
set-up says `Couldn’t open a browser from here`. Use your editor's port forwarding (VS Code devcontainers forward
loopback ports), or restart the daemon on every interface and publish the port on the host's loopback only:

```sh
rerail daemon stop && rerail daemon start --host 0.0.0.0 --allow-remote   # in the container
docker run -p 127.0.0.1:4737:4737 …                                       # on the host
```

`--allow-remote` lets anything that reaches that port read and change your rules, which is why the port should be
published on `127.0.0.1` only.

## WSL

Rerail in WSL is the Linux build, with its own `~/.rerail`, coding agent folders and daemon, separate from Rerail on
Windows. Install it on the side where your coding agent runs: in WSL for an agent running in WSL (including VS Code
or Cursor connected to WSL), and on Windows for an agent running on Windows. Use both if you use both; they share
nothing. Keep repositories on the same side as the agent, under your WSL home (`~/code/…`) rather than `/mnt/c/…`.

## Removing Rerail completely

`rerail install --remove` disconnects every coding agent, takes `~/.rerail/bin` off `PATH`, stops the daemon and
moves your database into a backup (`~/.rerail-clean-YYYYMMDD/`). It leaves:

- `~/.rerail`: the installed versions, the launcher, the rule packs and the install's state (about 300 MB);
- `~/.cache/rerail`: the content store;
- `~/.rerail-clean-YYYYMMDD/`: your rules and history;
- `<file>.<timestamp>.bak` copies beside every config and shell startup file it changed;
- empty `~/.codex/skills/rerail*` folders and an empty `~/.omp/agent/extensions`.

To remove everything, run `rerail install --remove`, then delete `~/.rerail` and `~/.cache/rerail`, and the backup
and `.bak` files if you do not want them. Declining set-up's sessions question ("No, stop set-up") also leaves
`~/.rerail` and the `PATH` entry, because set-up installs before it asks; remove them the same way.
