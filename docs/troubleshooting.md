# Troubleshooting

Each entry starts with what you see, then why, then what to do. What Rerail runs on is listed under
[Supported and not supported](installation.md#supported-and-not-supported); what set-up writes, and how to remove
it, is in [Installing Rerail](installation.md).

## Installing

Set-up checks Node, the computer and Git before it installs anything. When one of them stops it, it says why and
what to do, and that nothing was changed on this machine; fix that, then run the same command again.

### Node is too old

```
✕ Rerail needs Node 24 or newer. You have Node 22.23.4.
  Nothing was changed on this machine.
  Upgrade Node:  nvm install 24
  Then run the same command again.
```

Rerail runs on Node 24 or newer. The upgrade command follows how this Node was installed: `brew upgrade node` for
Homebrew's `node` (`brew install node` for a versioned formula such as `node@22`), `nvm install 24`,
`fnm install 24 && fnm use 24` or `volta install node@24`. When set-up cannot tell, it says
`Install Node 24 or newer from https://nodejs.org or your package manager.` (the `nodejs` package of Ubuntu 24.04 is
Node 18; NodeSource has Node 24). npm prints `npm warn EBADENGINE Unsupported engine` lines before this message; they
say the same thing.

Until Rerail's first stable release, npm 10 (Node 20 and 22) stops earlier with
`npm error notarget No matching version found for @neralabs/rerail@*.`, and with `engine-strict=true` in `~/.npmrc`
npm stops with `npm error code EBADENGINE`. Both mean the same: install Node 24.

### Linux: `Rerail can’t run on this computer`

```
✕ Rerail can’t run on this computer: this Linux has glibc 2.26, and Rerail needs glibc 2.28 or newer.
  Nothing was changed on this machine.
  Rerail runs on macOS and Windows, and on Linux with glibc 2.28 or newer, each on arm64 or x64.
  Use a Linux with glibc 2.28 or newer (`ldd --version` shows yours), such as a newer release or container image.
  Then run the same command again.
```

Rerail's Linux engines need glibc 2.28 or newer, as Node 24's own Linux builds do. Every distribution Node 24 runs
on has it: Debian 10 and later, Ubuntu 20.04 and later, RHEL 8 and later and their rebuilds, Amazon Linux 2023,
Fedora, and the default `node:24` and `node:24-slim` container images. Amazon Linux 2, Ubuntu 18.04 and CentOS 7 are
older. Move to a newer release or image.

On Alpine and other musl distributions (`node:24-alpine`) the reason reads
`this Linux uses musl (Alpine, for example), and Rerail needs glibc`: Rerail ships only glibc engines for Linux. Use
a glibc image, such as `node:24-slim`.

On a system or processor Rerail has no engine for (FreeBSD, 32-bit ARM), it reads
`Rerail has no engine for <system> on <processor>`.

### Git is missing

```
✕ Rerail needs Git, and found none where it looks:
    /usr/bin/git
    /usr/local/bin/git
    /opt/homebrew/bin/git
    /home/linuxbrew/.linuxbrew/bin/git
    /run/current-system/sw/bin/git
    /home/me/.nix-profile/bin/git
  Nothing was changed on this machine.
  Install Git:  sudo apt-get install git
  Then run the same command again.
```

Rerail reads every repository through Git, and runs it only from the places listed, never from `PATH`, so a
repository cannot put its own `git` first. On Windows those are `C:\Program Files\Git\cmd`,
`C:\Program Files (x86)\Git\cmd`, `~\AppData\Local\Programs\Git\cmd` (Git for Windows installed for one user) and
`~\scoop\apps\git\current\cmd`. The install command fits the system: `apt-get`, `dnf`, `pacman` or `zypper` (without
`sudo` when you are root), `xcode-select --install` on macOS, `winget install --id Git.Git -e` on Windows; elsewhere
it says to use your package manager or https://git-scm.com. A Git that lives anywhere else is not found: install it
in one of these places. On macOS `/usr/bin/git` is always there, but only as a stub for Apple's developer tools:
Rerail counts it only once the Command Line Tools are installed (`xcode-select --install`), or Xcode with Git.

### Windows PowerShell: `npx.ps1 cannot be loaded`

```
npx : File C:\Program Files\nodejs\npx.ps1 cannot be loaded because running scripts is disabled on this system.
```

PowerShell's execution policy blocks npm's `npx.ps1` before Rerail runs. Run `npx.cmd @neralabs/rerail setup`
instead, run the command in Command Prompt, or allow local scripts for your account:
`Set-ExecutionPolicy -Scope CurrentUser RemoteSigned`. The `rerail` command set-up installs is `rerail.cmd`, which the
policy does not block.

### Set-up prints nothing for a while

The first `npx @neralabs/rerail setup` downloads and installs about 300 MB, and prints nothing for 20 to 60 seconds
before Rerail's first line: that is npx fetching the package. Set-up then says what it installs,
`Downloading @neralabs/rerail@<version> from npm…`, before npm installs it again under `~/.rerail`. When npm cannot
reach the registry, see below.

### npm cannot reach the registry

When npx cannot fetch Rerail at all, npm stops before Rerail runs:

```
npm error code ECONNREFUSED
npm error network If you are behind a proxy, please make sure that the
npm error network 'proxy' config is set properly.  See: 'npm help config'
```

When npx already has Rerail in its cache, set-up starts, asks the registry once, and stops within seconds, naming it:

```
rerail: could not reach the npm registry (https://registry.npmjs.org, ECONNREFUSED): check your network, proxy, or registry in ~/.npmrc
```

`rerail update` stops the same way. Both run your own `npm`, with your `~/.npmrc`. Check the network, and npm's
`registry`, `proxy`, `https-proxy` and `cafile` settings (`npm config list`). A registry mirror set in `~/.npmrc`
works for Rerail and its rule packs. Rerail cannot be installed offline.

A failed install removes what it staged and its lock. If you press Ctrl-C while set-up or `rerail update` is
installing, the lock stays: the next run then says `Another Rerail update is already running` for up to 15 minutes.
When no other set-up or update is running, delete `~/.rerail/update.lock` and run it again.

### `Couldn’t download Rerail’s rule packs`

```
Couldn’t download Rerail’s rule packs, so set-up can’t tell which fit app.
  could not reach the npm registry (https://registry.npmjs.org, ECONNREFUSED): check your network, proxy, or registry in ~/.npmrc
Try downloading them again?
```

npm gave set-up none of the rule packs: the registry went away, or your npm account has no access to them. Fix that,
then choose "Yes, try again"; "Not now" stops set-up with nothing turned on. When only some packs could not be
downloaded or used, set-up goes on with the others and lists the rest under `Rule packs set-up could not use:`, each
with its reason. A pack made for a newer Rerail says so, and that `rerail update` installs the newest.

### A rule pack I expected is not offered

```
Node.js and React would fit, but have no rules yet, so set-up does not offer them.
```

These packs are published, but have no rules yet, so adding one would check nothing. `rerail packs recommend` lists
them too. Nothing to do: a later version of the pack brings its rules.

## The `rerail` command

### `rerail: command not found` right after set-up

Set-up puts `~/.rerail/bin` on `PATH` for terminals opened after it ran, and says so
(``You can run `rerail` from any new terminal``, or from a coding agent
`Added ~/.rerail/bin to PATH in ~/.zshrc. Open a new terminal to run rerail.`).
Open a new terminal. Until then, run `~/.rerail/bin/rerail`, or `export PATH="$HOME/.rerail/bin:$PATH"`.

- **Windows**: set-up adds the folder to your user `Path`. A terminal inside VS Code, Cursor or another editor, and
  anything started from it (a coding agent's shell included), keeps the `Path` the editor started with: quit the
  editor completely and open it again. Meanwhile run `%USERPROFILE%\.rerail\bin\rerail.cmd`. Git Bash runs
  `rerail` too, through the sh launcher set-up writes beside `rerail.cmd`.
- **zsh scripts and ssh**: the line goes into `~/.zshrc`, which only interactive zsh reads, so `zsh -c '…'` and
  `ssh host rerail …` do not find `rerail`. Use `~/.rerail/bin/rerail` there, or add
  `export PATH="$HOME/.rerail/bin:$PATH"` to `~/.zshenv`. Rerail does not write `~/.zshenv` itself.
- **A different shell**: set-up writes the startup file of `$SHELL`, else of your account's login shell. If you use
  another shell, or set `RERAIL_MODIFY_PATH=0`, add `~/.rerail/bin` to `PATH` yourself.
- **Containers**: `docker exec … bash -c` reads no startup file. Use `bash -l`, or the full path.

The startup file set-up changed keeps the file as it was in `<file>.rerail.bak`, one backup per file.

### `rerail status` says `No repo found here` or `Rerail is not installed`

```
No repo found here (/home/me). Run rerail status inside your project.
```

`rerail status` reports on the repository you run it in. Run it inside your project.

```
Rerail is not installed on this computer. Install it with npx @neralabs/rerail setup.
```

This computer has no Rerail installation in `~/.rerail` (or in `RERAIL_HOME`, when that is set). Run set-up.

### Node was upgraded or removed after set-up

Set-up records the full path of the Node that ran it for the launcher, the hooks and Claude Code's status line;
Homebrew's by the link `brew upgrade` keeps (`/opt/homebrew/bin/node`), so upgrading Homebrew's Node changes
nothing. When the recorded Node is gone (`nvm uninstall`, say), each falls back to the first `node` on `PATH`, except
Claude Code's status line on Windows.

When that `node` is older than 24, `rerail` stops with set-up's `Rerail needs Node 24 or newer` message, the daemon
does not start, the coding agents' hooks do nothing and say why in one line on stderr:

```
rerail hook: Rerail needs Node 24 or newer and this is Node 22.12.0, so it is off in this session. Put Node 24 or newer first on PATH, then run rerail install --refresh.
```

and Claude Code's status line reads `R E R A I L ✕ needs Node 24 or newer (this is 22.12.0)`. Put Node 24 or newer
first on `PATH`, then run `rerail install --refresh`: it records that Node in the launcher, every connected coding
agent's hooks and the status line. `rerail update` does the same.

With no `node` on `PATH` at all, the launcher fails with exit code 127
(`…/.rerail/bin/rerail: line 5: exec: node: not found`), and so do the hooks. Install Node 24, then run
`~/.rerail/bin/rerail install --refresh`.

## Coding agents

### Codex: nothing happens

Codex runs only hooks you have trusted. Set-up's summary says so after it connects Codex:
`Open Codex and run /hooks once to trust Rerail’s hooks.` Open Codex, run `/hooks`, trust Rerail's hooks, then
start a new session. Codex asks again whenever an update changes the hook, as this release does on Windows. In Codex the
commands are skills, typed `$rerail`, `$rerail-show-me` and so on; set-up's summary writes them that way when Codex is
the only coding agent connected, and adds `In Codex, type $rerail-show-me.` when it is one of several.

On Windows, Codex's hook works from a profile folder whose name has a space or a non-ASCII letter
(`C:\Users\José García`).

### `Couldn’t connect …: … is not plain JSON`

```
  △ Couldn’t connect Claude Code: /home/me/.claude/settings.json is not plain JSON
    (Expected property name or '}' in JSON at position 2 (line 1 column 3)). Rerail left the file as it is.
    Take out its comments and trailing commas, then run rerail setup again to connect Claude Code.
```

Rerail reads and writes coding agent config files as strict JSON. Set-up leaves an agent whose file does not parse
alone, connects the others, starts the repository, and names that one in its summary. Take out the comments and
trailing commas from the file named, then run `rerail setup` again: it connects that agent. `rerail update` and
`rerail install --refresh` likewise leave that one agent alone and say so.

`rerail install` stops instead, with `rerail: Cannot parse JSON in <file>: <reason>`, and changes no agent's config.
Fix the file, or name the others: `rerail install --agents cursor`.

### Set-up says `No coding agent found on this computer`

Set-up connects the coding agents whose config folder exists (`~/.claude`, `~/.codex`, `~/.cursor`, `~/.omp/agent`,
`~/.pi/agent`). With none, its summary says so, names the five it works with, and says to install one, then run
`rerail setup` again to connect it.

### A coding agent installed after set-up is not connected

Run `rerail setup` again in a repository that is set up: it connects the coding agents installed since and names them
(`Now also connected: Codex.`). Run the agent once first, so its config folder exists, and open a new session in it
afterwards. Set-up run again never connects an agent you left out with `--agents` or disconnected with
`rerail install --remove --agents <id>` (it keeps track in `~/.rerail/coding-agents.json`); connect one of those with
`rerail install --agents <id>`.

### `Unknown coding agent`

```
rerail: Unknown coding agent: copilot. Rerail works with omp, pi, claude-code, codex, cursor.
```

Rerail connects only those five. GitHub Copilot, Windsurf, Gemini CLI and the others are not supported.

### Rerail does not see Claude Code or Codex

Rerail writes only to `~/.claude` and `~/.codex`. With `CLAUDE_CONFIG_DIR` or `CODEX_HOME` set elsewhere, the agent
never reads what Rerail wrote. Unset them, or point them at the default folders. On Windows, `~` is your profile
folder (`USERPROFILE`), where the agents look, even where `HOME` names another folder, as Git Bash's may.

### Cursor also runs Claude Code's hooks

With Claude Code and Cursor both connected, Cursor's Third-Party Imports setting (on by default) makes Cursor run
the hooks in `~/.claude/settings.json` too. Rerail's Claude Code hook ignores the events Cursor sends it, so each
Cursor chat is recorded once, as a Cursor session. Nothing to do.

### Claude Code: a command on Rerail's files says `Operation not permitted`

With Claude Code's sandbox on, set-up keeps the agent's commands and file tools out of the Rerail home and
`~/.cache/rerail` (see [What a coding agent can reach](installation.md#what-a-coding-agent-can-reach)). Rerail's own
commands run outside the sandbox, but only as a plain call: one with a redirect (`> file`), a `cd` or a `$(...)`
stays sandboxed and fails. Run the command without them, or run it yourself in a terminal. Run `rerail setup` again
after moving `RERAIL_HOME` or Node, so the entries name the new paths.

### A session open before set-up shows nothing

Coding agents load hooks and commands when a session starts. Open a new session after set-up, `rerail install`,
`rerail update` and `rerail install --remove`.

## Daemon and dashboard

### The daemon keeps running after `/rerail-stop`

That is on purpose. `/rerail-stop` (and `rerail stop`) stops Rerail in this checkout and the worktrees inside it: no
rules run there and nothing is logged. The daemon is shared by every checkout and coding agent on the computer, so it
keeps running for the others, and the reply says so. To shut it down, run this in a terminal:

```sh
rerail daemon stop
```

A coding agent's session in a checkout where Rerail is still started starts the daemon again with its next event; stop
Rerail there too to keep it down. `rerail status` says whether it is running.

### The dashboard URL changed

Rerail's daemon comes back on the port it last listened on, which it keeps in `~/.rerail/daemon-port.json`: 4737 the
first time, or the one `rerail daemon start --port <n>` named. Only when another program holds that port and 4737
does it take a free port, and keep that one from then on, so an old dashboard link stops working. `rerail manage
--print` prints the current URL (`/rerail-manage` in a session opens it). A port named with `--port` that is taken
fails the start (below).

### `rerail daemon start` fails

```
rerail: Rerail daemon exited with code 1 before it was ready:
…
```

The lines after it are what this start wrote to `~/.rerail/daemon.log`, and say why. A taken `--port` reads
`listen EADDRINUSE: address already in use 127.0.0.1:<n>`: pick another port, or leave `--port` out.

### `Another main Rerail instance is running`

A second daemon, from another `RERAIL_HOME` or one left behind, holds Rerail's machine-wide lock. Stop it
(`rerail daemon stop` from its home, or end that process by its pid), then start again.

### Containers: the dashboard does not open from the host

The daemon listens on the container's loopback only, so a published port (`-p 4737:4737`) gets an empty reply, and
set-up says `Couldn’t open your browser. Copy the link instead.` Use your editor's port forwarding first (VS Code
devcontainers forward loopback ports): the daemon stays on loopback. Only where that is not available, restart the
daemon on every interface and publish the port on the host's loopback only:

```sh
rerail daemon stop && rerail daemon start --host 0.0.0.0 --allow-remote --insecure-http   # in the container
docker run -p 127.0.0.1:4737:4737 …                                                       # on the host
```

`--insecure-http` keeps plain http, which is fair here only because the port is published on the host's loopback;
publish it anywhere else and pass `--tls-cert <pem> --tls-key <pem>` instead, so the capability is not sent in clear
(see the daemon section of [reference.md](reference.md)).

Off loopback the capability changes on every start, so open the new link (`rerail daemon start` prints it) after a
restart. Anyone holding the link can read and change your rules, so do not paste it anywhere. Publishing on
`127.0.0.1` keeps the port off the host's network, but inside the container the daemon now listens on every
interface: any other container on the same Docker network (a Compose service, a sidecar) reaches it directly, and is
refused without the capability.

### The dashboard says "Needs the dashboard link"

Every request for your data needs the daemon's access capability, which the page keeps in your browser once it has
been opened from the link `rerail manage` opens or `rerail daemon start` prints. A bookmark, a new browser or
profile, cleared site data, or a link pasted from a coding agent's session has none: run `rerail manage` (or
`rerail daemon start --open`) and the page opens with it. On loopback the capability lasts across restarts
(`access-token` in your Rerail home; delete it to rotate, then open the link again). `curl` and scripts send it as the
`x-rerail-access` header, reading `accessToken` from `daemon.json`; without it they get
`401 {"error":"access-required"}`.

If `rerail status` says the daemon "refused the access capability in daemon.json", the running daemon and its state
file disagree (one was edited or replaced): `rerail daemon stop`, then `rerail daemon start`.

## WSL

Rerail in WSL is the Linux build, with its own `~/.rerail`, coding agent folders and daemon, separate from Rerail on
Windows. Install it on the side where your coding agent runs: in WSL for an agent running in WSL (including VS Code
or Cursor connected to WSL), and on Windows for an agent running on Windows. Use both if you use both; they share
nothing. Keep repositories on the same side as the agent, under your WSL home (`~/code/…`) rather than `/mnt/c/…`.
Set-up run in WSL for a repository on the Windows drive says so, and goes on:

```
△ This is WSL, and /mnt/c/code/app is on the Windows drive.
  Rerail and your coding agent have to be installed on the same side: both in WSL, or both on Windows.
  Repos on the Windows drive are slow from WSL; a clone in the Linux file system (under ~) is faster.
```

## Removing Rerail completely

`rerail install --remove` disconnects every coding agent, takes `~/.rerail/bin` off `PATH` (and deletes the
`<file>.rerail.bak` it kept), stops the daemon, moves your database into a backup (`~/.rerail-clean-YYYYMMDD/`), and
then deletes the installation: `~/.rerail/current`, `~/.rerail/bin`, `~/.rerail/versions`, the rule packs and
`~/.cache/rerail`. It prints each path it removed (`Removed <path> (<what>).`), and names what it kept:

- `~/.rerail` itself, with Rerail's settings, logs and records, such as `consent.json` and `daemon.log`
  (`Kept ~/.rerail: … (Rerail's settings, logs and records; delete it if you will not install Rerail again).`);
- a `bin`, `versions` or `packs` folder that also holds files that are not Rerail's, as when `RERAIL_HOME` is a
  folder you use for other things: only Rerail's files go
  (`Kept <path> (<what>): it holds files that are not Rerail's: <names>.`);
- `~/.rerail-clean-YYYYMMDD/`: your rules and history;
- `<file>.<timestamp>.bak` copies beside every coding agent config file it changed.

To remove everything, delete those too. A file it cannot remove, such as one Windows keeps locked while a session
still runs Rerail, is named with the command that finishes the job:
`Could not remove <path> (<reason>). Close every coding agent and terminal still running Rerail, then run: …`.

Choosing "No, cancel set-up" at set-up's sessions question connects and starts nothing, but set-up has installed
Rerail and put it on `PATH` before it asks. It says so:
`Rerail stays installed, and off; rerail install --remove takes it off this computer.`
