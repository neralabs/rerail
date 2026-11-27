# Troubleshooting

Each entry: what you see, why, what to do. Supported systems are under
[Supported and not supported](installation.md#supported-and-not-supported); set-up's files and their removal, under
[Installing Rerail](installation.md).

## Installing

Set-up checks Node, the computer and Git first. A failed check changes nothing: fix it, then rerun the same command.

### `npx: command not found`

```text
zsh: command not found: npx
```

Node.js is not installed (`npx` comes with it), so set-up never starts. Bash says `npx: command not found`;
PowerShell says `The term 'npx' is not recognized`. Install Node 24 or newer from [nodejs.org](https://nodejs.org) or
with Homebrew (`brew install node`), nvm, fnm or Volta, open a new terminal, check that `node --version` prints `v24`
or later, and rerun.

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

A Rerail that npm installed without set-up, as a dependency or with `npm install -g`, makes no such check. Where it
then finds no engine package for the system, it gives the reason itself:

```text
Rerail has no engine for Linux with musl (Alpine, for example): on Linux it needs glibc.
Rerail has no engine for freebsd on x64: it runs on macOS, Windows, and Linux with glibc, each on arm64 or x64.
```

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

The first `npx @neralabs/rerail setup` downloads about 240 MB, which unpacks to over 700 MB: npm 11.6.2 and the later
npm 11 releases fetch Rerail's engines for all six platforms into npx's cache. npx prints nothing meanwhile, for a
minute or more on a slow connection.
Set-up then prints `Downloading @neralabs/rerail@<version> from npm…`, and npm installs it again under `~/.rerail`,
this time with the one engine for this computer, about 50 MB.
From there each step prints a line as it starts, and a step still running prints `still running after 30 s`, then
again every 30 seconds.

### Set-up or `rerail update` says npm may be stuck

```text
Downloading @neralabs/rerail@<version> from npm has taken 3 min. npm may be stuck: check your network or proxy, and the registry npm uses (`npm config get registry`). Rerail stops it at 20 min.
```

npm prints nothing while it downloads, so Rerail times each step. On a slow line the download can take this long; wait
or press Ctrl-C. A download still running at 20 minutes, or an npm request at 3, is stopped: the update fails, its
staged files are removed and the version that was active stays active. Check the settings in
[npm cannot reach the registry](#npm-cannot-reach-the-registry), then run it again.

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
works for Rerail and its rule packs, as long as it serves Rerail's six engine packages as well as `@neralabs/rerail`
([Registry access](installation.md#registry-access)).

A failed install removes what it staged. The update lock is free the moment the set-up or `rerail update` holding it
ends, Ctrl-C included, so `Another Rerail update is already running` means one is installing now. A Rerail whose engine
does not load holds the folder `~/.rerail/locks/update-without-engine` instead; if one was cut short and the message
stays with nothing installing, delete that folder and run again.

</details>

### `npm could not install …, the engine … runs on here`

```text
npm could not install @neralabs/rerail-engine-linux-x64-gnu@0.2.1, the engine @neralabs/rerail@0.2.1 runs on here (E404): the registry npm installs from (https://npm.example.com) does not have it. A mirror, proxy or allowlist must carry @neralabs/rerail-engine-linux-x64-gnu as well as @neralabs/rerail: check the registry in your npm configuration. Nothing was installed.
```

Set-up and `rerail update` install Rerail and this computer's engine package in one npm command, and your registry
served the first and not the second. Have it serve the six engine packages
([Registry access](installation.md#registry-access)), then run the same command again. Nothing was installed, and a
version already active stays active.

<details>
<summary>The codes, and an engine that is not the released one</summary>

- **`E404`:** the registry does not know the package; the message names the registry when npm did. **`ETARGET`:** it
  knows the package and lacks that version, as a mirror that has not caught up does.
- **`EBADPLATFORM`:** the message reads `npm takes it for another system's engine`. The `npm` on your PATH runs on a
  Node built for another processor than the Node Rerail runs on, which the message names: put that Node first on PATH
  and run the command again.
- **Another file under the same name:**

  ```text
  The @neralabs/rerail-engine-linux-x64-gnu@0.2.1 npm installed is not the one @neralabs/rerail@0.2.1 was released with: its integrity is sha512-…, and @neralabs/rerail@0.2.1 pins sha512-…. The registry npm installs from serves another file under that name. Nothing of it was run.
  ```

  Each Rerail version pins the sha512 of the engine package released with it, and what your registry served has
  another. Compare `npm view @neralabs/rerail-engine-<suffix>@<version> dist.integrity` through your registry and with
  `--registry https://registry.npmjs.org`: a mirror must pass npmjs's tarball on unchanged.
- **A registry that gives npm no sha512:** where a registry describes a version with a SHA-1 alone, Rerail has npm
  hash the tarball it installed and compares that, so the install goes on as usual. Where npm cannot hash it, the
  message reads `npm recorded no SHA-512 for the … it installed (its integrity is sha1-…), and its tarball could not be
  hashed …`, with npm's reason, and nothing is installed: run the command again once npm reaches the registry.

</details>

### `Couldn’t download Rerail’s rule packs`

```text
Couldn’t download Rerail’s rule packs, so set-up can’t suggest rule packs for app.
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
Node.js and React would be suggested, but have no rules yet, so set-up does not offer them.
```

These packs are published but have no rules yet, so they would check nothing. Nothing to do: a later version brings
its rules. `rerail packs recommend` lists them too.

### A rule pack is left out, or its rules show as not running, after an update

Rerail 0.2.1 and later leave out a pack version published before 17 November 2026, and mark as not running a rule of
the 17 November versions that tests a token the new syntax trees leave out. Install the newest version of each pack
that works with this Rerail:

```sh
rerail packs update
```

### `No rule pack matches what it’s built with`

```text
Scanned acme-api. No rule pack matches what it’s built with.

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

## The engine

Rerail's native engine is not inside `@neralabs/rerail`: each system and processor has an engine package, installed
with Rerail at the same version ([Registry access](installation.md#registry-access)). A command, hook or daemon that
cannot load it stops with one of the messages below. Set-up and `rerail update` print it after
`Installed native runtime health check failed:` and keep the version that was active.

### Rerail's engine is not installed

```text
Rerail's engine for Linux on x64 is not installed: the Rerail at /home/you/.npm/_npx/0f1e2d3c4b5a6978/node_modules/@neralabs/rerail has no @neralabs/rerail-engine-linux-x64-gnu@0.2.1, an optional dependency of @neralabs/rerail. npm leaves optional dependencies out under --omit=optional (omit=optional or optional=false in an .npmrc), and when the registry or mirror it installs from lacks the package or serves another file for it. Install it with `npm install --prefix "/home/you/.npm/_npx/0f1e2d3c4b5a6978" @neralabs/rerail-engine-linux-x64-gnu@0.2.1`. Or set RERAIL_ENGINE_PATH to an engine file for this system (rerail_engine.linux-x64-gnu.node).
```

npm installed Rerail without its engine and reported no error: it skips an optional dependency it is told to leave out,
or cannot fetch, and still exits with 0. Run the command the message gives. Naming the package installs it whatever
the setting, and where the registry lacks it npm now fails and says so.

When `rerail update` printed the message, it gives no command: the update staged that Rerail in a folder it removed
as it failed, and the version that was active stays active. Fix the cause below, then run `rerail update` again.

<details>
<summary>Finding the cause, and other shapes of the message</summary>

- **npm leaves optional dependencies out.** `npm config list` shows `omit = ["optional"]` or `optional = false`, from
  an `.npmrc` or an `NPM_CONFIG_` variable. Remove it, or keep running the message's command after each install you
  make with npm yourself.
- **The registry lacks the package.** `npm view @neralabs/rerail-engine-<suffix>@<version> version` prints the version
  when your registry serves it. A private registry, mirror or allowlist must serve all six engine packages
  ([Registry access](installation.md#registry-access)).
- **The registry serves another file for the package.** npm checks each engine package against the sha512 Rerail pins
  for it and leaves out one that differs, with no error. Compare
  `npm view @neralabs/rerail-engine-<suffix>@<version> dist.integrity` through your registry and with
  `--registry https://registry.npmjs.org`.
- **An update from Rerail 0.1.5 or earlier:** in place of a command, the message says

  ```text
  An update or set-up staged this Rerail, and removes its folder when it fails: run the update or set-up again once npm can install the package. Where npm's settings leave optional dependencies out, set the environment variable npm_config_include=optional for that run.
  ```

  Those versions install `@neralabs/rerail` alone and leave its engine to npm's optional dependencies, so a setting
  that leaves them out stops the update. Set the variable in the terminal that runs the update, not in an `.npmrc`:
  `npm_config_include=optional rerail update` in sh, bash or zsh; `$env:npm_config_include = "optional"; rerail update`
  in PowerShell; `set npm_config_include=optional` and then `rerail update` in cmd. It does not help where the
  registry lacks the package. Later versions install the engine package by name and need no variable.
- **A global install, or any folder npm did not install Rerail into as a dependency:** the message gives the command
  without a folder:

  ```text
  Install it with `npm install @neralabs/rerail-engine-linux-x64-gnu@0.2.1`, run where @neralabs/rerail is installed (with `-g` for a global install).
  ```
- **`RERAIL_ENGINE_PATH`** names an engine file for Rerail to load instead of looking for the package. Use the engine
  built for this Rerail's version; a missing file is `RERAIL_ENGINE_PATH does not exist: <path>`.

</details>

### Rerail was installed for another system

```text
The Rerail at /Users/you/.rerail/versions/0.2.1/node_modules/@neralabs/rerail was installed for macOS on arm64, and this is macOS on x64: it has @neralabs/rerail-engine-darwin-arm64 and no @neralabs/rerail-engine-darwin-x64@0.2.1. Its folder came from another computer, or this Node is built for another processor than the Node that installed it. Run Rerail with that Node, or add this system's engine. Install it with `npm install --prefix "/Users/you/.rerail/versions/0.2.1" --force @neralabs/rerail-engine-darwin-x64@0.2.1`. `--force` has npm keep the other system's engine. Or set RERAIL_ENGINE_PATH to an engine file for this system (rerail_engine.darwin-x64.node).
```

An install holds the engine for the system and processor of the Node that installed it, and this Node is another.
Run Rerail with the Node that installed it, or run the command the message gives, after which the install holds this
system's engine too. npm 11.6.2 and later also install all six inside Rerail then, about 240 MB.

<details>
<summary>How an install meets another system</summary>

- **Two Nodes on one computer, built for different processors:** on a Mac, an x64 Node under Rosetta beside an arm64
  one, from two version managers, say; on Windows on Arm, an x64 Node beside an arm64 one.
- **A folder from another computer:** a `~/.rerail` copied, restored from a backup, taken from a CI cache keyed without
  the system, or mounted from the host into a container. Run `npx @neralabs/rerail setup` on the system that runs
  Rerail, with a Rerail home of its own (`RERAIL_HOME`) where the folder is shared.

</details>

### The engine package is another version than Rerail

```text
@neralabs/rerail-engine-linux-x64-gnu at /home/you/app/node_modules/@neralabs/rerail-engine-linux-x64-gnu is version 0.2.2, and the Rerail at /home/you/app/node_modules/@neralabs/rerail is version 0.2.1: Rerail loads only the engine released with it. Install it with `npm install --prefix "/home/you/app" @neralabs/rerail-engine-linux-x64-gnu@0.2.1`.
```

Rerail found its system's engine package at another version than its own, and loads only the engine released with
it. Run the command the message gives.

This happens when an engine package was installed or updated apart from Rerail: named in a `package.json` of its own,
or left beside Rerail in its `node_modules/@neralabs` folder by an earlier install. Depend on `@neralabs/rerail`
alone; it names the engine package of its version.

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

### `rerail status` says `No Git repository here` or `Rerail is not installed`

```text
No Git repository here (/home/me). Rerail works in a Git repository: run rerail status inside one, or start Rerail in one with rerail start --repo <folder>.
```

Rerail works per Git repository, and `rerail status` reports on the one you run it in. Run it inside your project, or
start Rerail in your project from where you are with `rerail start --repo <folder>`, `<folder>` being the project's
folder.

```text
Rerail is not installed on this computer. Install it with npx @neralabs/rerail setup.
```

There is no installation in `~/.rerail` (or `RERAIL_HOME`, when set). Run set-up.

### `Git could not say where the repository of … is`

```text
Git could not say where the repository of C:\code\app is: exit status 3221225794 (0xC0000142), signal none, stderr: nothing
```

Rerail asked Git where the folder's repository is, and that Git gave no answer: Windows could not start it
(0xC0000142, often while the machine runs out of resources for new processes), it ran past its time limit of 10 seconds
(`no exit status`, with the time limit named), something ended it (a signal, or on Windows exit status 1 with no
output), or it failed with the message after `stderr:`. For that one ask the folder reads as a folder outside Git, as
it did before, and Rerail keeps nothing from the answer, so the next command or hook asks Git again. Where Rerail would
record the checkout from it, the record fails with the line instead and nothing is written: no `.rerail/repository-id`
in the checkout, no move to another repository. The same line ending `which clone id the config of the repository of …
holds` comes from the read of a clone id kept in Git's config (`rerail.repository-id`): that record fails too, and no
new id is made in its place. The daemon writes each line to `daemon.log` once for each folder and reason. When it keeps
coming, run `git -C <folder> rev-parse --show-toplevel` in a terminal: Git's own error says what to mend.

### Node was upgraded or removed after set-up

If the Node that ran set-up is gone and the next `node` on `PATH` is older than 24, `rerail` stops with
`Rerail needs Node 24 or newer`, and hooks say why on stderr:

```text
rerail hook: Rerail needs Node 24 or newer and this is Node 22.12.0, so it is off in this session. Put Node 24 or newer first on PATH, then run rerail install --refresh.
```

Put Node 24 or newer first on `PATH` and run `rerail install --refresh` (or `rerail update`). It records that Node in
the launcher, every connected agent's hooks and the status line.

If an older Node took the place of the recorded one at the same path (a package downgraded, Homebrew's `node` linked
to an older formula), `rerail` still runs that path whatever `PATH` holds, and the hooks' line names the launcher's
own command with `node` from `PATH` in its place:

```text
rerail hook: Rerail needs Node 24 or newer and this is Node 22.12.0, so it is off in this session. Put Node 24 or newer first on PATH, then run node /home/me/.rerail/current/dist/cli/cli.js install --refresh: the rerail command runs /usr/local/bin/node while that file is there, whatever PATH holds.
```

Run that with Node 24 or newer first on `PATH`; it records that Node as `rerail install --refresh` does. Putting Node
24 or newer back at the recorded path works too. On Windows the line puts the path in double quotes, which `cmd.exe`
and PowerShell both take; a path holding `%`, `!`, `$`, a backtick or a typographic double quote gets PowerShell's
single quotes instead, and the line says to run it in PowerShell.

<details>
<summary>How Node is found, and no `node` at all</summary>

The launcher, hooks and, where it starts Node, Claude Code's status line run the Node that ran set-up, by full path.
For Homebrew that is `/opt/homebrew/bin/node`, a link `brew upgrade` keeps, so upgrading changes nothing. The status
line starts Node on Windows, and where you had a status line of your own
([how Claude Code draws it](in-session-surfaces.md#how-claude-code-draws-it)).

If that Node is gone (`nvm uninstall`, say), they fall back to the first `node` on `PATH`, Claude Code's status line on
Windows included.

With that `node` older than 24, the daemon does not start and hooks do nothing. A Claude Code status line that starts
Node reads `R E R A I L ✕ needs Node 24 or newer (this is 22.12.0)`.

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
again whenever an update changes the hook.

In Codex the commands are skills: `$rerail`, `$rerail-show-me` and so on. Set-up's summary uses that form when Codex
is the only connected agent; with several, it adds `In Codex, type $rerail-show-me.`

On Windows, the hook works from a profile folder with a space or non-ASCII letter (`C:\Users\José García`).

</details>

### Windows: Codex or Cursor cannot run a Rerail command

```text
Unexpected token '"C:\Users\me\.rerail\current\dist\cli\cli.js"' in expression or statement.
```

Codex and Cursor run commands in PowerShell on Windows. Rerail 0.1.4 and earlier wrote their commands as
`"<node>" "<cli.js>" …`, which PowerShell reads as a string followed by a word it refuses. Later releases write
`& '<node>' '<cli.js>' …`. Run `rerail update`, then open a new session.

### `Couldn’t connect Claude Code: Claude Code's Rerail hook …`

```text
  △ Couldn’t connect Claude Code: Claude Code's Rerail hook did not answer within 30 s
    Rerail’s hooks are written, but they fail. Fix that, then run rerail setup again to connect Claude Code.
```

After connecting Claude Code, set-up runs the hook it registered once, as Claude Code will. A hook that cannot run
records nothing and says nothing in a session, so set-up reports the connection as failed, though the hooks stay in
Claude Code's `settings.json`. The message ends with what happened: `did not answer within 30 s`, `exited <code>: <its
output>`, `failed: rerail hook: <reason>`, `answered with no JSON object` or `could not run: <reason>`. Fix what it
names, then run `rerail setup` again.

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

A coding agent ran Rerail's hooks, and Rerail could not record their events (or the `rerail events` stream's, under
its own names for them, such as `user-prompt`). The line names the last event that
failed, the coding agent and its version, the Rerail version and the reason, such as `no Rerail daemon answered`
(the daemon was down; the next hook starts it), `the Rerail daemon (pid …) is running but did not answer within 12 s`
(it was busy, for example opening a large checkout, and the hook could not leave the event for it: the daemon is of an
older version, or could not watch `untaken-events`; events are recorded again once it answers), `the Rerail daemon
(pid …) is running but not reading, and 256 events already wait for it, the most Rerail keeps` or `… would pass the
16 MiB Rerail keeps` (see below), `… and
the event is larger than the 1 MiB one event may hold` (a tool call that large, which a daemon that reads refuses
too), `… and the event could not be left for it` with the error that kept it out of `untaken-events`,
`the Rerail daemon (pid …) refused the access capability in daemon.json` (`daemon.json` was edited or written by
another daemon; run `rerail daemon stop`, then `rerail daemon start`) or a payload field the coding agent sent in a
shape Rerail did not expect. The failures are kept in `hook-failures.jsonl` in the Rerail home; past 256 KB one hook
moves it to `hook-failures.jsonl.1`, replacing the one there, and the line counts both. While it cannot be moved, as
when another program holds `hook-failures.jsonl.1`, it stops taking failures at 1 MB. Update Rerail with
`rerail update`; if the line stays, report it with `rerail support-bundle`.

The line leads with the first hook that failed after the last event Rerail recorded. When hooks also failed before
that event, it goes on with the count of all of them: `K hook events failed, of N that could not be recorded in the
last 24 hours`. When events were recorded after the last failure, the line reads `Hooks: N hook events could not be
recorded in the last 24 hours` instead. Once the log has moved aside and holds nothing from 24 hours ago, failures from
before `hook-failures.jsonl.1` began may be gone, so the line counts the failures since it began and says when that
was: `since <time>, as far back as the log goes`; when another program keeps `hook-failures.jsonl.1` from being read,
it counts from when `hook-failures.jsonl` began. When every failure it still holds came after the last recorded
event, hooks may have failed before the time it leads with, and the line says `as far back as the log goes` too.

A daemon that runs but does not answer costs only the hooks that start in its first 12 s their wait: each leaves its
event in `untaken-events` in the Rerail home and answers, and while such an event waits there each later hook leaves
its own behind it at once without asking the daemon. A session's start still waits. As soon as the daemon reads again
it checks those events as it checks any event, in the order their hooks met them and before any later event; what a
check finds reaches the agent with the session's next answer, not with the hook that left the event. Rerail never saw
such a tool call's files before it ran, so it tells the call its own lines only where the file before the call is
certain, and checks the whole file the call left everywhere else. The file before the call is certain only when the
call is an Edit or MultiEdit (not a Write), its edits put back (each new text, found exactly once, replaced by its old
text, at every place for an edit of every place) give a text on which the tool would make them again and leave
exactly the file as it is, and that text is the file as the session last saw it. Then the call is told exactly its
own lines, as when Rerail sees it run, but for one case: an edit of every place is also told a line holding its new
text that another program wrote over an older line since the session last looked, one line more, never fewer.
Anything else is checked whole, and a finding can name lines the call did not write, but none of its own is missed: a
Write; an Edit that created its file; an Edit whose new text the file no
longer holds once; a file another program or another such call changed since the session last saw it, or that the
session first saw after the call ran, as in a busy spell that starts with the session. A tool's output is not kept
with them. The folder keeps at most 256 events and 16 MiB, so what a daemon that reads again has to check
is bounded; past that a hook's event is lost and the line above counts it. An entry there the daemon cannot read (a
folder under an event's name, a file another user or program left, one another program holds open) counts once as
an event it could not check, and is removed where it can be. While events wait, `rerail status` says
`Hooks: N hook events wait in untaken-events for the Rerail daemon, which has not read since <time>`; once the daemon
has checked them it says how many it checked late, and how many it could not check. In Cursor, prompts and answers
read from the transcript stay unread until the next hook the daemon answers, which checks them.

With `RERAIL_AUTOSTART=0` (or `false`) hooks start no daemon, and a hook that finds none logs nothing, since that is
what you asked for. A session that starts with no daemon running says so as it starts: Claude Code and Codex
show the line under the session's start, and Cursor's agent is asked to begin its first reply with it. The line, which
`rerail status` prints too, says the daemon is not running, that `RERAIL_AUTOSTART=0` keeps hooks from starting it, so
nothing is recorded until it runs, and to start it with `rerail daemon start`. A daemon that runs but that hooks may
not use, one of another Rerail package or, on Windows, one of Rerail 0.1.4 or earlier, is not replaced either: the
session's start says that nothing is recorded until a daemon from the active package runs, and to run
`rerail daemon stop`, then `rerail daemon start`. Each event lost beside it goes to the hook failure log above.

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

Rerail 0.1.4 and earlier start the daemon with no console. Each Git command the daemon runs then gets a console window
of its own, which Windows 11 opens in Windows Terminal. The release after 0.1.4 starts the daemon with a console that
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

### `Engine: unavailable: no code rule can run: …`

`rerail status` says this, and exits 5, when the daemon's kernel loads but its code rules can open no
[content store](how-setup-works.md#the-content-store); the session footer says `engine unavailable`, and `daemon.log`
and the support bundle give the same reason. The daemon reads it once, as it starts. Conversation rules, steers and
session history still run; code rules and audits do not.

- `… is neither the managed installation under … nor in a Git checkout, so it has no content store`: Rerail runs from a
  package outside `~/.rerail/versions` and outside any Git checkout, such as one `npm install` put in a folder of its
  own. Install a published version with `npx @neralabs/rerail setup`, run a checkout's build
  ([Developing from a checkout](how-setup-works.md#developing-from-a-checkout)), or start the daemon with
  `RERAIL_CONTENT_STORE_PATH` naming a file outside the machine-wide store directory (`~/.cache/rerail/`, on Windows
  `%USERPROFILE%\AppData\Local\rerail\`). Then restart the daemon.
- `The machine-wide Rerail content store … belongs to the main Rerail daemon`: `RERAIL_CONTENT_STORE_PATH` names a file
  in the machine-wide store directory, which only the managed installation's daemon opens. Point it elsewhere, or unset
  it, then restart the daemon.

### `Another main Rerail instance is running`

A second daemon, from another `RERAIL_HOME` or left behind, holds Rerail's machine-wide lock. Stop it
(`rerail daemon stop` from its home, or end its pid), then start again.

### `Another Rerail daemon (pid …) runs in …`

Another daemon holds this Rerail home's lock (`locks/daemon.lock`). A daemon started there writes this line to the
home's `daemon.log` and exits as soon as the holder answers as the home's daemon, or after waiting a second for the lock
when the holder does not answer. A `rerail daemon start` that launched it prints the line under
`Rerail daemon exited with code 1 before it was ready`. While the daemon it names answers, the next
`rerail daemon start` finds it. If it does not, `rerail daemon stop` stops it once `daemon.json` names it, or gives the
pid to end; if the stop finds none and a start prints this line again, end the process it names and start again.

### `this daemon (pid …) no longer holds the Rerail home at …`

The Rerail home was moved or removed while its daemon ran. The daemon writes this line to `daemon.log` and stops
writing `daemon.json`, so a home made again at that path never names it, but it still runs, recording into the
database of the home it lost. End the pid it names, then run `rerail daemon start`.

### `not-on-disk: the change was made, but Rerail could not confirm it is on the disk …`

The daemon made a change you asked for in the dashboard, with `rerail settings` or `rerail scan` (in internal testing),
or through an MCP tool, then waited for the disk to keep it, and the disk gave the error the message ends with. The
dashboard shows the same message. Rerail says a change is made only once it is on the disk, so a power cut cannot undo
it; this one is made, but a power cut or an operating system crash may still undo it. Do not make it again unless you
see it is gone: making it again can make it twice, such as a second audit request.

The daemon's background sync of the history syncs the same file. While that sync fails, `daemon.log` says
`rerail daemon: syncing the history to the disk failed, trying again every 5 s: …` once, and
`rerail daemon: syncing the history to the disk works again` once a sync succeeds. Until then a power cut or an
operating system crash can take more than the last 5 s of history ([Rerail database](reference.md#rerail-database)).

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

### The dashboard asks to connect this browser

The page says "Connect this browser to Rerail" in place of every view. Run `rerail manage` (or
`rerail daemon start --open`) in a terminal to open the page with the daemon's access capability, a secret carried in
the dashboard link. The page that asked then continues on its own. If it opens in a browser other than the one you
want, paste the link it prints into that one.

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

### `Rerail's engine … is not built`

```text
Rerail's engine for macOS on arm64 is not built: /Users/you/rerail/native has no rerail_engine.darwin-arm64.node. Build it with `npm run build:native`, or fetch a released one with `npm run engine:pull`. Or set RERAIL_ENGINE_PATH to an engine file for this system (rerail_engine.darwin-arm64.node).
```

A clone loads the engine in its own `native/` folder, and a new clone has none. `npm run build:native` builds this
machine's from source, which needs Rust; `npm run engine:pull` fetches the one a release published.

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
- A daemon that misses the first 2-second health check gets 10 more seconds to answer before the bundle calls it
  unresponsive; one that answers is recorded with how long it took. The file is written before that wait, calling the
  daemon unresponsive, and again after it, so a command stopped while it waits still leaves a bundle.

| In the file | Left out |
| --- | --- |
| Rerail, Node and OS versions; whether Rerail runs from its managed install; daemon and engine health; connected agents | Your prompts and the agent's replies |
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
`rerail install --remove --delete-data` deletes those too, and `~/.rerail` itself; it cannot be undone.

What `--remove` keeps, and names:

- **`~/.rerail`**: Rerail's settings, logs and records, such as `consent.json` and `daemon.log`.
- **`~/.rerail-clean-YYYYMMDD/`**: your rules and history.
- **`<file>.<timestamp>.bak`** copies beside every agent config file it changed.

`--delete-data` keeps only what it cannot reach, such as those `.bak` copies, which hold your own settings, and
names each with the command that deletes it. Both lists in full:
[What each removal leaves behind](how-setup-works.md#what-each-removal-leaves-behind).

<details>
<summary>Steps it runs, shared folders, and what it prints</summary>

A `bin`, `versions` or `packs` folder that also holds other files, as when `RERAIL_HOME` is shared with other things,
is kept too; only Rerail's files go.

1. Disconnects every coding agent.
2. Stops the daemon.
3. Moves your database into a backup, `~/.rerail-clean-YYYYMMDD/`.
4. Takes `~/.rerail/bin` off `PATH`, and deletes the `<file>.rerail.bak` it kept.
5. Deletes `~/.rerail/current`, `~/.rerail/bin`, `~/.rerail/versions`, the rule packs and `~/.cache/rerail` (on Windows
   `%USERPROFILE%\AppData\Local\rerail`, and the `~/.cache/rerail` an earlier release used there).

It prints each removed path (`Removed <path> (<what>).`) and each kept one:

- `Kept ~/.rerail: … (Rerail's settings, logs and records; delete it if you will not install Rerail again).`
- `Kept <path> (<what>): it holds files that are not Rerail's: <names>.`

A file it cannot remove, such as one Windows locks while a session still runs Rerail, is named with the command that
finishes the job:
`Could not remove <path> (<reason>). Close every coding agent and terminal still running Rerail, then run: …`.

</details>

Choosing "Not now, cancel set-up" at set-up's sessions question connects and starts nothing, but Rerail is already installed
and on `PATH`. Set-up says so: `Rerail stays installed, and off; rerail install --remove takes it off this computer.`
