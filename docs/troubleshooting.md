# Troubleshooting

Each entry: what you see, why, what to do. Supported systems are under
[Supported and not supported](installation.md#supported-and-not-supported); set-up's files and their removal, under
[Installing Rerail](installation.md).

## Installing

Set-up checks Node, the computer and Git first. A failed check changes nothing: fix it, then rerun the same command.

### Node is too old

```text
✕ Rerail needs Node 24 or newer. You have Node 22.23.4.
  Nothing was changed on this machine.
  Upgrade Node:  nvm install 24
  Then run the same command again.
```

Install Node 24 or newer with the suggested command, then rerun.

<details>
<summary>Upgrade command per install method, and npm's own errors</summary>

The suggested upgrade matches how Node was installed:

| Installed with | Command |
| --- | --- |
| Homebrew `node` | `brew upgrade node` |
| Homebrew versioned formula (`node@22`) | `brew install node` |
| nvm | `nvm install 24` |
| nvm-windows | `nvm install 24, then nvm use 24` |
| fnm | `fnm use --install-if-missing 24` |
| Volta | `volta install node@24` |
| Unknown | `Install Node 24 or newer from https://nodejs.org or your package manager.` |

Ubuntu 24.04's `nodejs` package is Node 18; NodeSource has Node 24. npm prints
`npm warn EBADENGINE Unsupported engine` lines before set-up's message; they mean the same.

Until Rerail's first stable release, npm may stop sooner. Both of these errors mean install Node 24:

- **npm 10** (Node 20 and 22): `npm error notarget No matching version found for @neralabs/rerail@*.`
- **`engine-strict=true`** in `~/.npmrc`: `npm error code EBADENGINE`.

</details>

### Linux: `Rerail can’t run on this computer`

```text
✕ Rerail can’t run on this computer: this Linux has glibc 2.26, and Rerail needs glibc 2.28 or newer.
  Nothing was changed on this machine.
  Rerail runs on macOS and Windows, and on Linux with glibc 2.28 or newer, each on arm64 or x64.
  Use a Linux with glibc 2.28 or newer (`ldd --version` shows yours), such as a newer release or container image.
  Then run the same command again.
```

Move to a newer release or image, such as `node:24-slim`.

<details>
<summary>Which Linux has glibc 2.28, and other reasons</summary>

Rerail's Linux engines need glibc 2.28 or newer, like Node 24's own builds. Every distribution Node 24 runs on has it:
Debian 10+, Ubuntu 20.04+, RHEL 8+ and rebuilds, Amazon Linux 2023, Fedora, and the default `node:24` and
`node:24-slim` images. Amazon Linux 2, Ubuntu 18.04 and CentOS 7 are too old.

Other reasons:

- **musl** (`node:24-alpine`): `this Linux uses musl (Alpine, for example), and Rerail needs glibc`. Rerail ships
  only glibc engines for Linux; use a glibc image such as `node:24-slim`.
- **No engine** (FreeBSD, 32-bit ARM): `Rerail has no engine for <system> on <processor>`.

</details>

### Git is missing

```text
✕ Rerail needs Git, and found none where it looks:
    /home/me/.local/bin/git
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

Rerail reads every repository through Git. Install Git with the suggested command, then rerun.

<details>
<summary>Install commands, and where Rerail looks</summary>

The suggested install command fits the system:

- **Linux**: `apt-get`, `dnf`, `pacman` or `zypper`, without `sudo` as root.
- **macOS**: `xcode-select --install`.
- **Windows**: `winget install --id Git.Git -e`.
- **Elsewhere**: your package manager, or https://git-scm.com.

Rerail tries the first `git` on `PATH` (`git.exe` on Windows), then the locations listed. On Windows it also tries
`C:\Program Files\Git\cmd`, `C:\Program Files (x86)\Git\cmd`, `~\AppData\Local\Programs\Git\cmd` (per-user Git for
Windows) and `~\scoop\apps\git\current\cmd`.

It skips `PATH` entries that are relative (`.`, `bin`), inside the repository (direnv's `PATH_add`) or inside
`node_modules`. So a repository's own `bin/` cannot supply `git`, and a Git only there is not found. A folder named
`git`, or a `git` without its execute bit, is skipped too.

macOS always has `/usr/bin/git`, but only as a stub for Apple's developer tools. Rerail counts it once the Command
Line Tools (`xcode-select --install`), or Xcode with Git, are installed.

</details>

### Windows PowerShell: `npx.ps1 cannot be loaded`

```text
npx : File C:\Program Files\nodejs\npx.ps1 cannot be loaded because running scripts is disabled on this system.
```

PowerShell's execution policy blocks npm's `npx.ps1` before Rerail runs. Any one of these works:

- Run `npx.cmd @neralabs/rerail setup`.
- Run the command in Command Prompt.
- Allow local scripts: `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned`.

The installed `rerail` command is `rerail.cmd`, which the policy does not block.

### Set-up prints nothing for a while

The first `npx @neralabs/rerail setup` downloads and installs about 300 MB; npx prints nothing for 20 to 60 seconds.
Set-up then prints `Downloading @neralabs/rerail@<version> from npm…`, and npm installs it again under `~/.rerail`.

### npx exits with code 1 and prints nothing

```text
PS C:\code\app> npx.cmd @neralabs/rerail setup
PS C:\code\app> $LASTEXITCODE
1
```

npx failed to install Rerail before set-up could start: npm prints nothing for `npx` when the install script of a
package it installs fails. npm's log says which one. Open the newest file in the `_logs` folder of npm's cache
(`npm config get cache` prints the folder: on Windows usually `%LocalAppData%\npm-cache`, on macOS and Linux
`~/.npm`) and search it for `error`. Attach that log when you report the problem.

Rerail 0.1.3 and earlier made npm compile better-sqlite3, which needs Python and C++ build tools, and the log shows
`gyp ERR! find Python`. Later versions install the copy better-sqlite3 ships prebuilt: run
`npx @neralabs/rerail@latest setup`.

### npm cannot reach the registry

Without Rerail in its cache, npx stops before Rerail runs:

```text
npm error code ECONNREFUSED
npm error network If you are behind a proxy, please make sure that the
npm error network 'proxy' config is set properly.  See: 'npm help config'
```

With Rerail cached, set-up asks the registry once and stops within seconds:

```text
rerail: could not reach the npm registry (https://registry.npmjs.org, ECONNREFUSED): check your network, proxy, or registry in ~/.npmrc
```

Check the network, and npm's `registry`, `proxy`, `https-proxy` and `cafile` settings (`npm config list`). Rerail
cannot be installed offline.

<details>
<summary>Mirrors, and `Another Rerail update is already running`</summary>

`rerail update` stops the same way. Both run your own `npm` with your `~/.npmrc`, so a registry mirror set there
works for Rerail and its rule packs.

A failed install removes what it staged. The update lock is free the moment the set-up or `rerail update` holding it
ends, Ctrl-C included, so `Another Rerail update is already running` means one is installing now. A Rerail whose engine
does not load holds the folder `~/.rerail/locks/update-without-engine` instead; if one was cut short and the message
stays with nothing installing, delete that folder and run again.

</details>

### `Couldn’t download Rerail’s rule packs`

```text
Couldn’t download Rerail’s rule packs, so set-up can’t tell which fit app.
  could not reach the npm registry (https://registry.npmjs.org, ECONNREFUSED): check your network, proxy, or registry in ~/.npmrc
Try downloading them again?
```

Fix your network or npm's access to the registry, then choose "Yes, try again". "Not now" stops set-up with nothing
turned on.

<details>
<summary>Why, and when only some packs fail</summary>

npm gave set-up no rule packs: the registry went away, or your npm account has no access.

If only some packs fail to download or load, set-up goes on and lists them under `Rule packs set-up could not use:`,
each with its reason. A pack made for a newer Rerail says so, and that `rerail update` installs the newest.

</details>

### A rule pack I expected is not offered

```text
Node.js and React would fit, but have no rules yet, so set-up does not offer them.
```

These packs are published but have no rules yet, so they would check nothing. Nothing to do: a later version brings
its rules. `rerail packs recommend` lists them too.

### `None of Rerail’s rule packs fit it`

```text
Scanned acme-api. None of Rerail’s rule packs fit it.

  Found           .py 3 · .md 1
                  from Git’s list of files, which leaves out what Git ignores
  Not scanned     fixtures/ (1)
  Closest         Python excludes the 3 files it looks for: tests/ (2),
                  scripts/ (1)
  Packs look for  Python      *.py, *.pyi
                  TypeScript  *.ts, *.mts, *.cts, +1 more,
                              dependency: typescript
```

A pack fits when the repository has a file or dependency it looks for. **Found** counts the files scanned by ending
and says how they were listed. **Not scanned** names the folders set-up never reads, such as `fixtures/` and
`node_modules/`, with the files in each. **Closest** is a pack that looks for files the repository has but excludes
every one of them, by the places it names; a leading `/` means only at the repository's top. **Packs look for** says
what each pack would match. To add a pack anyway, answer "Change rule packs…", or run `rerail packs add <id>`.
`rerail packs recommend` prints the same explanation, with what each pack looks for on that pack's line.

## The `rerail` command

Fixes for `rerail` not being found, finding no repository, or stopping after Node changed.

### `rerail: command not found` right after set-up

Open a new terminal: set-up adds `~/.rerail/bin` to `PATH` for new ones. Until then, run `~/.rerail/bin/rerail`, or
`export PATH="$HOME/.rerail/bin:$PATH"`.

<details>
<summary>Windows, zsh scripts and ssh, other shells, containers</summary>

Set-up says it changed `PATH`: ``You can run `rerail` from any new terminal``, or from a coding agent
`Added ~/.rerail/bin to PATH in ~/.zshrc. Open a new terminal to run rerail.`

- **Windows**: set-up adds the folder to your user `Path`. Terminals in VS Code, Cursor or another editor, and what
  they start (agent shells too), keep the editor's old `Path`: quit the editor completely and reopen it. Meanwhile
  run `%USERPROFILE%\.rerail\bin\rerail.cmd`. Git Bash uses the sh launcher set-up writes beside `rerail.cmd`.
- **zsh scripts and ssh**: set-up writes `~/.zshrc`, which only interactive zsh reads, so `zsh -c '…'` and
  `ssh host rerail …` miss it. Use `~/.rerail/bin/rerail`, or add `export PATH="$HOME/.rerail/bin:$PATH"` to
  `~/.zshenv` (Rerail never writes it).
- **A different shell**: set-up writes the startup file of `$SHELL`, else your login shell. For another shell, or
  with `RERAIL_MODIFY_PATH=0`, add `~/.rerail/bin` to `PATH` yourself.
- **Containers**: `docker exec … bash -c` reads no startup file. Use `bash -l`, or the full path.

Set-up backs up each startup file it changes to `<file>.rerail.bak`, one backup per file.

</details>

### `rerail status` says `No repo found here` or `Rerail is not installed`

```text
No repo found here (/home/me). Run rerail status inside your project.
```

`rerail status` reports on the repository you run it in. Run it inside your project.

```text
Rerail is not installed on this computer. Install it with npx @neralabs/rerail setup.
```

There is no installation in `~/.rerail` (or `RERAIL_HOME`, when set). Run set-up.

### Node was upgraded or removed after set-up

If the Node that ran set-up is gone and the next `node` on `PATH` is older than 24, `rerail` stops with
`Rerail needs Node 24 or newer`, and hooks say why on stderr:

```text
rerail hook: Rerail needs Node 24 or newer and this is Node 22.12.0, so it is off in this session. Put Node 24 or newer first on PATH, then run rerail install --refresh.
```

Put Node 24 or newer first on `PATH` and run `rerail install --refresh` (or `rerail update`). It records that Node in
the launcher, every connected agent's hooks and the status line.

<details>
<summary>How Node is found, and no `node` at all</summary>

The launcher, hooks and Claude Code's status line run the Node that ran set-up, by full path. For Homebrew that is
`/opt/homebrew/bin/node`, a link `brew upgrade` keeps, so upgrading changes nothing.

If that Node is gone (`nvm uninstall`, say), they fall back to the first `node` on `PATH`. Claude Code's status line
on Windows does not.

With that `node` older than 24, the daemon does not start and hooks do nothing. Claude Code's status line reads
`R E R A I L ✕ needs Node 24 or newer (this is 22.12.0)`.

With no `node` on `PATH`, the launcher and hooks fail with exit code 127
(`…/.rerail/bin/rerail: line 5: exec: node: not found`). Install Node 24, then run
`~/.rerail/bin/rerail install --refresh`.

</details>

## Coding agents

Problems connecting Rerail to its five supported coding agents, or with an agent that does not run its hooks.

### Codex: nothing happens

Codex runs only hooks you have trusted. Run `/hooks` in Codex, trust Rerail's hooks, then start a new session.

<details>
<summary>Set-up's hint, skills, and Windows</summary>

After connecting Codex, set-up's summary says `Open Codex and run /hooks once to trust Rerail’s hooks.` Codex asks
again whenever an update changes the hook, as this release does on Windows.

In Codex the commands are skills: `$rerail`, `$rerail-show-me` and so on. Set-up's summary uses that form when Codex
is the only connected agent; with several, it adds `In Codex, type $rerail-show-me.`

On Windows, the hook works from a profile folder with a space or non-ASCII letter (`C:\Users\José García`).

</details>

### Windows: Codex or Cursor cannot run a Rerail command

```text
Unexpected token '"C:\Users\me\.rerail\current\dist\cli\cli.js"' in expression or statement.
```

Codex and Cursor run commands in PowerShell on Windows. Rerail 0.1.3 and earlier wrote their commands as
`"<node>" "<cli.js>" …`, which PowerShell reads as a string followed by a word it refuses. Later releases write
`& '<node>' '<cli.js>' …`. Run `rerail update`, then open a new session.

### `Couldn’t connect …: … is not plain JSON`

```text
  △ Couldn’t connect Claude Code: /home/me/.claude/settings.json is not plain JSON
    (Expected property name or '}' in JSON at position 2 (line 1 column 3)). Rerail left the file as it is.
    Take out its comments and trailing commas, then run rerail setup again to connect Claude Code.
```

Rerail reads and writes agent config files as strict JSON. Remove the comments and trailing commas, then run
`rerail setup` again to connect that agent.

<details>
<summary>What set-up, update and install do meanwhile</summary>

Set-up skips an agent whose file does not parse, connects the others, starts the repository, and names the skipped
agent in its summary. `rerail update` and `rerail install --refresh` skip that agent the same way and say so.

`rerail install` stops instead, with `rerail: Cannot parse JSON in <file>: <reason>`, and changes no config. Fix the
file, or name the other agents: `rerail install --agents cursor`.

</details>

### Set-up says `No coding agent found on this computer`

Set-up connects agents whose config folder exists: `~/.claude`, `~/.codex`, `~/.cursor`, `~/.omp/agent`,
`~/.pi/agent`. With none, the summary names the five it supports. Install one, then run `rerail setup` again.

### A coding agent installed after set-up is not connected

Run the agent once so its config folder exists, then run `rerail setup` again in a set-up repository. Open a new
session in the agent afterwards.

<details>
<summary>What set-up says, and agents you left out</summary>

Set-up connects the new agents and names them (`Now also connected: Codex.`).

It never connects an agent you left out with `--agents` or disconnected with `rerail install --remove --agents <id>`;
`~/.rerail/coding-agents.json` tracks those. Connect one with `rerail install --agents <id>`.

</details>

### `Unknown coding agent`

```text
rerail: Unknown coding agent: copilot. Rerail works with omp, pi, claude-code, codex, cursor.
```

Only those five are supported. GitHub Copilot, Windsurf, Gemini CLI and others are not.

### Rerail does not see Claude Code or Codex

Unset `CLAUDE_CONFIG_DIR` and `CODEX_HOME`, or point them at the defaults: Rerail writes only to `~/.claude` and
`~/.codex`, and with them set elsewhere the agent never reads it.

On Windows, `~` is your profile folder (`USERPROFILE`), where the agents look, even when `HOME` differs, as Git Bash's
may.

### Cursor also runs Claude Code's hooks

With both connected, Cursor's Third-Party Imports setting (on by default) also runs the hooks in
`~/.claude/settings.json`. Rerail's Claude Code hook ignores Cursor's events, so each Cursor chat is recorded once,
as a Cursor session. Nothing to do.

### Claude Code: a command on Rerail's files says `Operation not permitted`

Rerail's own commands leave Claude Code's sandbox only as a plain call. Drop any redirect (`> file`), `cd` or
`$(...)`, or run the command in your own terminal.

<details>
<summary>Why, and after moving Rerail or Node</summary>

With Claude Code's sandbox on, set-up keeps the agent's commands and file tools out of the Rerail home and
`~/.cache/rerail` (see [What a coding agent can reach](installation.md#what-a-coding-agent-can-reach)). With a
redirect, a `cd` or a `$(...)`, Rerail's commands stay sandboxed and fail.

After moving `RERAIL_HOME` or Node, run `rerail setup` again so the entries name the new paths.

</details>

### A session open before set-up shows nothing

Agents load hooks and commands when a session starts. Open a new session after set-up, `rerail install`,
`rerail update` and `rerail install --remove`.

### `rerail status` says `Hooks run but nothing was recorded`

A coding agent ran Rerail's hooks, and Rerail could not record their events. The line names the last event that
failed, the coding agent and its version, the Rerail version and the reason, such as `no Rerail daemon answered`
(the daemon was down; the next hook starts it), `the Rerail daemon (pid …) is running but did not answer within 12 s`
(it was busy, for example opening a large checkout; events are recorded again once it answers),
`the Rerail daemon (pid …) refused the access capability in daemon.json` (`daemon.json` was edited or written by
another daemon; run `rerail daemon stop`, then `rerail daemon start`) or a payload field the coding agent sent in a
shape Rerail did not expect. The failures are kept in `hook-failures.jsonl` in the Rerail home; past 256 KB it moves
to `hook-failures.jsonl.1`, replacing the one there. A hook stopped while it moves the log leaves it as
`hook-failures.jsonl.<pid>.<id>.old`, which the line still counts; a later move removes it once that hook has ended and
the file holds nothing from the last 24 hours. Update Rerail with `rerail update`; if the line stays, report it with
`rerail support-bundle`.

When events were recorded since the first failure, the line reads `Hooks: N hook events could not be recorded in the
last 24 hours` instead. Once the log has moved aside and holds nothing from 24 hours ago, failures from before about its
oldest may be gone, so the line counts from that failure and says when it was: `since <time>, as far back as the log
goes`. Hooks that fail in the moment the log moves can land on either side of that time.

With `RERAIL_AUTOSTART=0` (or `false`) hooks start no daemon, and a hook that finds none logs nothing, since that is
what you asked for. `rerail status` then says the daemon is not running and that `RERAIL_AUTOSTART=0` keeps hooks from
starting it.

If `rerail status` instead names Claude Code hook scripts that are missing, Claude Code's settings point at a Rerail
that is gone, and every hook fails before Rerail runs. Run `rerail install --refresh` and open a new session.

### Windows: a build from a checkout records nothing

```text
Model: checksum mismatch (config.json)
```

`rerail status` says this, and every hook of Rerail built from a Git checkout (`just rerail-dev --install-bridge`, or a
daemon run from the checkout) fails with `model artifact checksum mismatch for config.json`. Git for Windows checks
text files out with CRLF line breaks unless told otherwise, and `model/manifest.json` pins the bytes of
`model/config.json` and `model/tokenizer.json`. The installed package is not affected.

A checkout of a commit whose `.gitattributes` marks both files `-text` writes them as stored. In a checkout made
before, have Git write them again, then stop the daemon; the next session event starts it with the model:

```sh
git rm -q --cached model/config.json model/tokenizer.json
git checkout HEAD -- model/config.json model/tokenizer.json
rerail daemon stop
```

[Developing on Windows](../CONTRIBUTING.md#developing-on-windows) covers the rest of the checkout's line endings.

## Daemon and dashboard

The daemon is the background process every checkout and coding agent on the computer shares; it serves the dashboard.

### Windows: a terminal window keeps opening and takes focus

While a coding agent works, a Windows Terminal window opens every few seconds and takes the keyboard focus. With
several sessions busy it opens almost without pause.

Rerail 0.1.3 and earlier start the daemon with no console. Each Git command the daemon runs then gets a console window
of its own, which Windows 11 opens in Windows Terminal. The release after 0.1.3 starts the daemon with a console that
has no window, and every program the daemon runs shares it. Update while no coding agent session is working, since a
session's next event starts the daemon again:

```sh
rerail daemon stop
rerail update
```

The next session event starts the new daemon.

<details>
<summary>Until you can update</summary>

Start the daemon under a hidden console yourself, in PowerShell. Its Git commands then share that console and open
no window:

```powershell
$rerail = "$env:USERPROFILE\.rerail"
& "$rerail\bin\rerail.cmd" daemon stop
$env:RERAIL_HOME = $rerail
$node = (Get-Command node -CommandType Application | Select-Object -First 1).Source
Start-Process cmd.exe -WindowStyle Hidden -WorkingDirectory $rerail -ArgumentList "/c `"`"$node`" `"$rerail\current\dist\cli\daemon-entry.js`" --state-file `"$rerail\daemon.json`" >> `"$rerail\daemon.log`" 2>&1`""
```

Run it while no coding agent session is working: a hook that finds no daemon starts one the old way. This lists one
daemon when it worked:

```powershell
Get-CimInstance Win32_Process -Filter "Name='node.exe'" | Where-Object CommandLine -like '*daemon-entry.js*'
```

The windows come back after a sign-out, a restart or `rerail daemon start`, until you update.

</details>

### The daemon keeps running after `/rerail-stop`

By design: `/rerail-stop` (or `rerail stop`) stops Rerail only in this checkout; the shared daemon keeps running. To
shut it down, run in a terminal:

```sh
rerail daemon stop
```

<details>
<summary>What stops, and why the daemon comes back</summary>

`/rerail-stop` covers this checkout and its worktrees: no rules run and nothing is logged there. The daemon keeps
running for other checkouts and agents, and the reply says so.

An agent session in a checkout where Rerail is still started restarts the daemon with its next event; stop Rerail
there too. `rerail status` says whether the daemon is running.

</details>

### The dashboard URL changed

`rerail manage --print` prints the current URL; `/rerail-manage` in a session opens it.

<details>
<summary>How the port is chosen</summary>

The daemon reuses the port in `~/.rerail/daemon-port.json`: 4737 at first, or the one `rerail daemon start --port <n>`
named. Only when another program holds that port and 4737 does it take a free port, and keep it, so old links stop
working. A taken `--port` fails the start instead (below).

</details>

### `rerail daemon start` fails

```text
rerail: Rerail daemon exited with code 1 before it was ready:
…
```

The following lines are what this start wrote to `~/.rerail/daemon.log`, and say why. A taken `--port` reads
`listen EADDRINUSE: address already in use 127.0.0.1:<n>`: pick another port, or leave `--port` out.

### `Another main Rerail instance is running`

A second daemon, from another `RERAIL_HOME` or left behind, holds Rerail's machine-wide lock. Stop it
(`rerail daemon stop` from its home, or end its pid), then start again.

### Containers: the dashboard does not open from the host

The daemon listens on the container's loopback only, so a published port (`-p 4737:4737`) gets an empty reply. Set-up
says `Couldn’t open your browser. Copy the link instead.`

Use your editor's port forwarding (VS Code devcontainers forward loopback ports), which keeps the daemon on loopback.
Otherwise, listen on every interface and publish the port on the host's loopback only:

```sh
rerail daemon stop && rerail daemon start --host 0.0.0.0 --allow-remote --insecure-http   # in the container
docker run -p 127.0.0.1:4737:4737 …                                                       # on the host
```

<details>
<summary>Security of this set-up</summary>

`--insecure-http` keeps plain http, fair only because the port is on the host's loopback. Published anywhere else,
pass `--tls-cert <pem> --tls-key <pem>` instead so the capability is not sent in clear (see the daemon section of
[reference.md](reference.md)).

Off loopback:

- The capability changes on every start. After a restart, open the new link `rerail daemon start` prints.
- Anyone holding the link can read and change your rules. Do not paste it anywhere.
- `127.0.0.1` keeps the port off the host's network, but in the container the daemon listens on every interface.
  Other containers on its Docker network (a Compose service, a sidecar) reach it directly, and are refused without
  the capability.

</details>

### The dashboard says "Needs the dashboard link"

Run `rerail manage` (or `rerail daemon start --open`) to open the page with the daemon's access capability, a secret
carried in the dashboard link.

<details>
<summary>Why the link is needed, scripts, and a refused capability</summary>

Every data request needs the capability. The browser keeps it once you open the link `rerail manage` opens or
`rerail daemon start` prints. A bookmark, a new browser or profile, cleared site data, or a link pasted from an
agent's session has none.

- **On loopback** the capability lasts across restarts, as `access-token` in your Rerail home. Delete it to rotate,
  then open the link again.
- **`curl` and scripts** send it as the `x-rerail-access` header, read from `accessToken` in `daemon.json`. Without
  it they get `401 {"error":"access-required"}`.

If `rerail status` says the daemon "refused the access capability in daemon.json", the daemon and its state file
disagree (one was edited or replaced). Run `rerail daemon stop`, then `rerail daemon start`.

</details>

## WSL

Rerail in WSL is a separate Linux install; put it on the same side as your coding agent.

It has its own `~/.rerail`, agent folders and daemon, and shares nothing with Rerail on Windows. Install it in WSL for
an agent in WSL (VS Code or Cursor connected to WSL included), on Windows for an agent on Windows, or on both.

Keep repositories under your WSL home (`~/code/…`) rather than `/mnt/c/…`. For a repository on the Windows drive,
set-up in WSL warns and goes on:

```text
△ This is WSL, and /mnt/c/code/app is on the Windows drive.
  Rerail and your coding agent have to be installed on the same side: both in WSL, or both on Windows.
  Repos on the Windows drive are slow from WSL; a clone in the Linux file system (under ~) is faster.
```

## Working from a clone of Rerail

For changing Rerail itself; setting up is in [CONTRIBUTING.md](../CONTRIBUTING.md#development-setup).

### Windows: checks and pack tests fail in a clone made before LF line ends

In a clone made on Windows before `.gitattributes` set `* text=auto eol=lf`, Git checked text files out with CRLF line
ends. Then the model check fails, `npm run check:structure` fails metrics #8 and #24, and 25 pack tests fail. Commit or
stash your work, then check every file out again under the current `.gitattributes`, once:

```sh
git rm --cached -r . && git reset --hard
```

## Reporting a problem

Run `/rerail-bug-report` in a coding agent; it works out the kind of problem and what to do.

<details>
<summary>What it does per kind of problem</summary>

| The problem | What it does |
| --- | --- |
| Engine bug: Rerail crashed, hung, lost a steer or could not run a rule | Offers to report it with the Bug report form |
| Bug in a rule that ships with Rerail | Offers Log or off in this repository, and to report it with the Rule misfire form |
| Bug in your own or your team's rule | Helps you fix it with `/rerail-create`'s Tune |
| Not a bug: Rerail is off here, the rule is on Log, or it works as documented | Says why, and how to get what you wanted |

</details>

The agent fills in the form for you to finish and submit, or files the issue if you ask. Nothing is sent without you.

### Support bundle

Maintainers can't follow dashboard links, which open only on your computer, so attach a support bundle to
[your issue](https://github.com/neralabs/rerail/issues/new/choose). `/rerail-bug-report` writes it and checks it with
you; in a terminal, inside the repository, run:

```sh
rerail support-bundle
```

It writes `~/rerail-support-bundle.json` and sends nothing. Issues are public and patterns cannot catch everything, so
read it first and replace anything you'd rather not post with `<redacted>`.

<details>
<summary>Options, contents and what Rerail replaces</summary>

- `--output <file>` writes elsewhere. The file is readable by your user only.
- It covers the repository's latest session; `--session <id>` names another (the id is in its dashboard address).
- It works where Rerail won't start or set-up never finished, and says which parts it could not read.

| In the file | Left out |
| --- | --- |
| Rerail, Node and OS versions; daemon and engine health; connected agents | Your prompts and the agent's replies |
| Installed packs and load failures; the repository's state; rule ids and modes | Tool input and output; source code |
| Session timeline: event kinds, tool names, times, what it dropped or could not check | Rule text; a violation's message, captures and fix |
| Steers as the agent got them; violations by rule, file and line | Paths named like secrets |
| The last 200 lines of `daemon.log`, scrubbed | Capabilities and tokens |

In what stays, Rerail replaces the daemon's capabilities, home folders (anyone's `/Users/…`, `/home/…` or
`C:\Users\…`), recorded repositories' paths and names, the session id, your user name and the computer's name. It
also removes common secret shapes (API keys, auth headers, passwords in URLs, private keys) and email addresses.

Other absolute paths, such as `/opt/…`, `/srv/…` or `/Volumes/…`, stay as written. A user name under four characters
is replaced inside paths only. A steer can quote your code, and a file path or your own rule's id can name a project.

A bundle is diagnostic context for maintainers, not a way to show others what Rerail did.

</details>

Report a vulnerability through [SECURITY.md](../SECURITY.md) instead.

## Removing Rerail completely

`rerail install --remove` deletes the installation and keeps your settings, data and backups for you to delete.

It names what it kept; delete those too to remove everything:

- **`~/.rerail`**: Rerail's settings, logs and records, such as `consent.json` and `daemon.log`.
- **`~/.rerail-clean-YYYYMMDD/`**: your rules and history.
- **`<file>.<timestamp>.bak`** copies beside every agent config file it changed.

<details>
<summary>Steps it runs, shared folders, and what it prints</summary>

A `bin`, `versions` or `packs` folder that also holds other files, as when `RERAIL_HOME` is shared with other things,
is kept too; only Rerail's files go.

1. Disconnects every coding agent.
2. Takes `~/.rerail/bin` off `PATH`, and deletes the `<file>.rerail.bak` it kept.
3. Stops the daemon.
4. Moves your database into a backup, `~/.rerail-clean-YYYYMMDD/`.
5. Deletes `~/.rerail/current`, `~/.rerail/bin`, `~/.rerail/versions`, the rule packs and `~/.cache/rerail` (on Windows
   `%LOCALAPPDATA%\rerail`, and the `~/.cache/rerail` an earlier release used there).

It prints each removed path (`Removed <path> (<what>).`) and each kept one:

- `Kept ~/.rerail: … (Rerail's settings, logs and records; delete it if you will not install Rerail again).`
- `Kept <path> (<what>): it holds files that are not Rerail's: <names>.`

A file it cannot remove, such as one Windows locks while a session still runs Rerail, is named with the command that
finishes the job:
`Could not remove <path> (<reason>). Close every coding agent and terminal still running Rerail, then run: …`.

</details>

Choosing "No, cancel set-up" at set-up's sessions question connects and starts nothing, but Rerail is already installed
and on `PATH`. Set-up says so: `Rerail stays installed, and off; rerail install --remove takes it off this computer.`
