# Rerail on Windows

Windows broke Rerail in ways macOS and Linux never show: a console window for every Git run, renames refused while
another program reads a file, one folder spelled two ways, and shells that read a command differently. This page
states each rule Rerail keeps on Windows, the defect behind it, and the module or check that holds it.
[installation.md](installation.md) and [troubleshooting.md](troubleshooting.md) cover what a user sees;
[CONTRIBUTING.md, "Developing on Windows"](../CONTRIBUTING.md#developing-on-windows) covers working on Rerail there.

## Where Windows is tested

CI runs the unit suite on Windows in one job, `unit-windows` in `.github/workflows/ci.yml`, and every other test job
on Linux or macOS. So each Windows rule is also held by a check, or by a test that fails on Linux when its Windows code
is reverted.

- **Checks.** `npm run check` runs the ones below on every platform: Windows-safe paths, and the built-ins only one
  module may use (child processes, renames, checkout keys and no-follow opens).
- **The platform as a parameter.** Code that branches on Windows takes the platform, or an object standing for the
  machine, with `process.platform` as its default, and its tests pass `"win32"`: `physicalPath(path, platform)`,
  `endProcessTree(child, { platform, killTree })`, `exitWhenParentGone({ platform })`, `DaemonMachine` in
  `src/runtime/daemon-control.ts`, `RerailDaemonOptions.enginePlatform`. `openNoFollow` takes the no-follow flag, and
  its test passes `0` to run the Windows path against a real symlink on Linux.
- **On Windows itself.** `release.yml` builds the native engine for Windows x64 and arm64, refuses one that imports
  the Visual C++ runtime (`scripts/check/check-windows-runtime.mjs`), and installs the package on a Windows x64 runner
  with `scripts/host/windows-install-smoke.ps1`. The [environment matrix](environment-matrix.md) installs it on
  Windows 11 VMs. Neither runs on a pull request.

<details>
<summary>Windows-safe paths</summary>

`npm run check:windows-paths` (`scripts/check/check-windows-paths.mjs`) fails on any tracked path Windows cannot check
out: longer than 200 characters, holding a character Windows forbids in a name, a reserved device name (`aux.js`), a
segment ending in a dot or a space, or two paths that differ only by case.

</details>

Rerail's own test suite runs on Windows too, in CI's `unit-windows` job and on a developer's machine. A test that
needs what Windows lacks, such as a symlink without Developer Mode, skips and says why, or uses what Windows offers in
its place, such as a junction or a named pipe. The helpers sit in `test/support/platform/`: `symlinkOrSkip` and
`linkFolder` (`links.ts`) skip a symlink test where Windows refuses one or make a junction for a folder,
`testEngineEndpoint` (`engine-endpoint.ts`) names a pipe where a Unix socket would go, and `removeFolder`
(`remove-folder.ts`) removes a temp folder once Windows lets go of it.

## Child processes

Every child process in `src/` starts through `src/engine/child-process.ts`, which sets `windowsHide: true`.
`npm run check:owned-builtins` fails on any other way to load `node:child_process` there.

Node starts a child with `windowsHide: false` unless told otherwise. A console program (Git, node, npm, PowerShell)
started by a process that has no console of its own gets a new console with a window, and Windows 11 opens that
console in Windows Terminal, which takes focus. Rerail 0.1.3's daemon had no console and ran Git for every edit it
checked, so a window opened every few seconds while an agent worked. With `windowsHide: true`, libuv starts the child
with `SW_HIDE`, and with `CREATE_NO_WINDOW` when it inherits no stdio. The option does nothing on macOS and Linux.

<details>
<summary>What the helper and the check cover</summary>

- The helper wraps `spawn`, `spawnSync`, `execFile` (its `promisify` form included) and `execFileSync`. It sets
  `windowsHide: true` where Node reads the options, after an argument list passed as `undefined` too
  (`withHiddenWindow`).
- `spawnShown` starts the one program whose window a person is meant to see: the browser opener. A Windows GUI program
  passes the show state it was started with to the window it opens, so `windowsHide` would hide the browser. It takes
  no console program.
- The check (`scripts/check/check-owned-builtins.mjs`) parses every tracked script under `src/` once. Each row of its
  `RULES` table names a built-in only one module may use; this section's is `child-process`.
  - Only the helper loads `node:child_process` as a value, by any import or export, `require`, `import()`, a
    `createRequire` result under any name, `module.require` or `process.getBuiltinModule`. A type-only use passes.
  - `test/engine/child-process.test.ts` checks that every starter the helper exports hides the window, and on Windows
    refuses a program named without its folder.
  - `program-path`: see [Programs by absolute path](#programs-by-absolute-path).
- Scripts outside `src/` (build, check and test scripts) are not in the package and may load `node:child_process`
  themselves.

</details>

## The daemon's console

On Windows the daemon owns a console that has no window, and every console program it starts shares that console. It
never starts without a console.

`DETACHED_PROCESS` leaves a process with no console at all, and Windows ignores `CREATE_NO_WINDOW` beside it. Node's
`detached: true` adds `DETACHED_PROCESS` on Windows, so Rerail starts the daemon through the native `spawn_detached`
(`native/src/process.rs`) with `CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP`. A child started without `windowsHide`,
by a library Rerail does not control for example, then shares the hidden console and opens no window either.

<details>
<summary>The launch, its handles, and its folder</summary>

- `launchDetached` (`src/runtime/daemon-control.ts`) is the one way a detached process starts on Windows: the daemon
  for `rerail daemon start`, the `rerail daemon start` a session's autostart runs (so the SQLite probe, npm and the
  managed-version dispatch it runs open no window either), and the replacement a daemon starts near its heap limit
  (`relaunchDaemon`).
- `spawn_detached` hands the child NUL stdin and the log file and no other handle (`PROC_THREAD_ATTRIBUTE_HANDLE_LIST`).
  Node's spawn hands a child every inheritable handle in the parent, and a daemon started under `just` held the
  caller's output pipe open, so a captured `rerail setup` never finished.
- It keeps the child's process handle. `detachedExitCode(pid)` reads the exit code from it, so a daemon that exits
  during its 15 s start is reported with its code, even once Windows has given its pid to another process.
- The daemon runs in the Rerail home, never in the folder of the session that started it. Windows holds a process's
  current folder open, so a daemon started from a repository kept that checkout, and every folder above it, from
  being deleted or renamed. The repository reaches it as `--workspace-root`, and `searchGitFrom`
  (`src/engine/git.ts`) looks for Git from there, so a `PATH` entry inside the repository is still skipped. The
  `rerail daemon start` a hook's autostart runs (`autostartDaemon`) starts in the Rerail home too, and hands it the
  session's folder as `--workspace-root`. This holds on every platform.
- The Rust test `starts_the_daemon_with_a_hidden_console_not_without_one` checks the flags on every OS, and the Windows
  build checks them against `windows-sys` when it compiles. PR CI builds the native engine on Linux only, so its
  `cfg(windows)` code first compiles in `release.yml`.

</details>

## Programs by absolute path

On Windows Rerail starts every program by its absolute path. `assertProgramPath` in `src/engine/child-process.ts`
refuses any other, unless the call runs a shell command line (`shell: true`).

Windows looks for a program named without a folder in the current folder before `PATH`, and Rerail runs inside
repositories it does not trust. A `powershell.exe` committed to a repository would have received the device secret
that Rerail hands Windows Credential Manager on stdin, and a `rundll32.exe` would have run each time Rerail opened the
dashboard.

<details>
<summary>Where each absolute path comes from</summary>

- **Programs Windows ships**: `windowsSystemProgram(name)` and `windowsPowerShell()` (`src/engine/system-programs.ts`),
  under `%SystemRoot%\System32`: PowerShell, `rundll32.exe` (run from the home folder), `taskkill.exe`, `icacls.exe`,
  `whoami.exe`.
- **Node, npm and Pi**: `programOnPath` (also in `src/engine/system-programs.ts`), the one lookup of a program on
  `PATH`. On Windows it takes only a `.exe` or `.com`, in `PATHEXT`'s order, from an absolute `PATH` entry: Node
  cannot spawn a version manager's `node.cmd` without a shell (`EINVAL`). Under Node, Rerail's children run under
  `process.execPath`. Under Bun (OMP), `nodeExecutable` looks node up this way and fails with a message when no
  `node.exe` is on `PATH`, instead of falling back to a bare `node`. Pi's fix tool finds `pi.exe` the same way.
- **Git**: `gitExecutable`, below.
- **npm**: on Windows it runs as `npm-cli.js` under the chosen node, with that node's folder first on `PATH`, so an
  install script builds better-sqlite3 for that node.
- **Hook commands Rerail registers**: Codex's and Cursor's PowerShell forms fall back to a bare `node`, which
  PowerShell looks up on `PATH` alone, never in the current folder. The cmd.exe form earlier set-ups wrote for Cursor
  found `node.exe` with `%~$PATH:I` for the same reason; registration recognizes it and replaces it.
- **The check**: rule `program-path` of `npm run check:owned-builtins` fails on a call to one of the helper's starters
  in `src/` that names its program by a literal that is not an absolute path.

</details>

## Git for Windows

Rerail runs Git for Windows' own `git.exe`, not its launcher, and reads Git's global files from the home Git uses.

<details>
<summary>The launcher, install locations and Git's home</summary>

- `<Git>\cmd\git.exe` and `<Git>\bin\git.exe` are launchers: they set up the environment and run
  `<Git>\mingw64\bin\git.exe` (`clangarm64` on ARM64, `mingw32` on 32-bit) as their child. Killing a process on
  Windows leaves its children running, so when a Git call timed out the real Git ran on in the repository and kept
  its locks. `gitBehindLauncher` (`src/engine/git.ts`) resolves a launcher to that `git.exe`, which finds its own DLLs,
  helpers and system configuration from its folder.
- Anyone can put a `cmd\git.exe` or `bin\git.exe` on `PATH`, so Rerail goes behind one only into a Git for Windows
  install: `<Git>\cmd\git.exe` runs, and `<Git>\etc\gitconfig` or the build's `share\git` is there. The `git.exe` it
  then runs must pass the checks every `PATH` entry gets (`trustedFolder`): outside every `node_modules`, and outside
  the repository Rerail runs in by its path and by its real path, which a junction into the repository would reach.
- `findGit` tries the candidates one at a time and stops at the first Git that runs, so the `PATH` entries after it
  are never resolved; the status line, a process of its own every few seconds, pays for the entries up to its Git
  only. On Windows a candidate spelled twice in different case is probed once.
- A Git laid out otherwise, such as a scoop shim, runs as found. When a batch reader (`readGitBlobs`, `readGitBlobIds`,
  `gitCrlfPaths`) times out, it ends the shim and the Git it started (`endProcessTree`), and answers 500 ms later
  (`settleWhenDone`) should the pipe still be open.
- After `PATH`, Git is looked for in Program Files on `C:` and on the drive of the account's home (Windows installed on
  another drive), then in the per-user installs under the account's home: `AppData\Local\Programs\Git` and scoop.
  These paths are built from the account's home, never from `LOCALAPPDATA`, which a repository's environment can set
  as easily as `PATH`.
- `gitHome` takes `HOME`, else `HOMEDRIVE` and `HOMEPATH` when that folder exists (a home share), else the profile
  folder, as Git for Windows does. So Rerail reads and watches the `~/.gitconfig` and `core.excludesFile` that Git
  reads.

</details>

## Renames and removals

Every rename in `src/` goes through `src/engine/file-replace.ts`, and the `file-replace` row of
`npm run check:owned-builtins` fails on any other. To make text the whole of a file, Rerail calls `replaceFile` or
`replaceFileSync`; `renameWithRetry` and `renameWithRetrySync` move a file or folder that already exists, or a file a
writer must open itself. A removal that waits out a held file calls `removeWithRetry`, and the `folder-removal` row
fails on an `rm` given `maxRetries` anywhere else.

On Windows, renaming a file onto or from one another process holds open fails with `EPERM`, `EACCES` or `EBUSY`. The
holder is often there for a moment only: a status line reading `daemon.json`, an editor with a source file open,
antivirus or the indexer scanning a file just written. One unretried rename made a daemon start fail, and a rewind
fail while an editor had the file open.

<details>
<summary>What the helpers do</summary>

- On Windows a rename is retried for 2 s while either file is held, then fails with an error that says another program
  held the file. A read-only target is made writable first, since Windows refuses to replace one. A folder at the
  target fails at once, as Windows never renames over one. Elsewhere the first failure stands.
- `replaceFile` writes the text to a new file beside the target, `<file>.<pid>.<uuid>.tmp`, created exclusively with
  the mode given, and renames it over the target. A reader sees the old file or the new one, and when any step fails
  the new file is removed and the old one stays.
- `removeWithRetry` removes a file or folder, trying again 10 times while a file in it is held, 5.5 s of waits in
  all, then fails with an error that names the file and says another program held it open. It counts the tries
  itself, each one `rm` with no retries of its own: `fs.promises.rm` repeats `maxRetries` at every level of the
  folder, so one held file kept it trying for 67 s rather than 5.5 s, and each folder further down multiplies that
  by 11. The updater's staging cleanup, `rerail install --remove`, `rerail profile remove` and the folder lock, for
  the folder it staged, use it.
- `removeScratch` removes a temporary folder Rerail made through `removeWithRetry` and never throws: a folder Windows
  still holds must not replace the result, or the error, of the work that used it. Rule pack discovery's staging
  folder, the code rule spool and the clock probe use it. The system's temp cleanup takes what is left, or, for a
  pack staging folder, the next fetch.
- Rule pack discovery's sweep of staging folders an hour old tries each once: one Windows still holds waits for the
  next fetch, rather than holding up this one.
- Set-up no longer deletes a coding agent's settings file before renaming the new one in, so a failed rename leaves
  the old file, not none.

</details>

## Checkout keys

Every path Rerail keys a checkout's records by is resolved through `checkoutRealpath(path)`
(`src/engine/catalog/checkout-control.ts`). On Windows that is `realpathSync.native`, which spells each name as stored
and the drive letter in upper case, as Git prints a top level. The `checkout-paths` row of
`npm run check:owned-builtins` fails on any other `realpath` in the modules that key catalog rows, audit settings,
presentation preferences or session checkouts (`KEYED` in the check).

Node's JavaScript `realpath` keeps the caller's spelling. VS Code passes its folder as `c:\…`, so the checkout of a
session opened from it never matched the one Git names `C:\…`: the session read as unrecorded and no rule ran in it.
The same mismatch let `c:\users\me` past set-up's refusal of the home folder, and lost audit settings and presentation
preferences saved under the other spelling.

<details>
<summary>Other places that compare Windows paths</summary>

- `physicalPath` and `storedPath` resolve through the same realpath; macOS and Linux keep the JavaScript one.
- The support bundle replaces recorded Windows paths in any case.
- Set-up's folder prompt reads `~\` and the double quotes Explorer's "Copy as path" adds.

</details>

## Links

Rerail never makes a symlink on Windows, and opens its own files without following a link at their name.

- **Making links.** Windows refuses `symlinkSync` without Developer Mode or admin rights. The managed installation's
  `current` is a junction, which needs neither. `rerail scan --rules-ref` writes the ref's `.rerail/rules` symlink
  entry as an empty folder, which the rule loader skips on every platform, as it skips anything that is not a regular
  file.
- **Opening files.** `O_NOFOLLOW` is undefined on Windows, so `flags | O_NOFOLLOW` added nothing there and a planted
  link opened as Rerail's own file. `openNoFollow` and `openNoFollowSync` (`src/engine/no-follow.ts`) use the flag
  where the OS has it. Where it has none they read the name with `lstat` before the open, refuse a link with `ELOOP`,
  and check that the opened file is the one `lstat` saw (device and file id), so a link swapped in between is refused
  too. Session record files, source reads, the repository identity file and the `daemon.log` that `rerail scrub`
  rewrites open this way. A source read that meets a link then reads the file the link leads to, when that file lies
  in the checkout that owns the link, outside its `.git`, and is not named like one holding secrets, opening it the
  same way; an agent's edit through such a link is checked on every system, and a link to `.env` is read on none. The
  rewind's temporary file needs neither: it is a new random name opened with
  `O_CREAT | O_EXCL`, which refuses any name that exists, a link included. The `no-follow` row of
  `npm run check:owned-builtins` fails on a read of `O_NOFOLLOW` anywhere in `src/` but `no-follow.ts`.

## Line endings

Git for Windows' installer sets `core.autocrlf=true` for the whole machine, so a checkout writes each LF of a text
blob as CRLF. Rerail allows for that in the repositories it watches, and its own checkout keeps LF.

<details>
<summary>Files Git writes with CRLF are Git's, not the agent's</summary>

Rerail decides that Git, not the agent, wrote a file by comparing the working-tree bytes with Git's blob, and finds a
file restored from a branch by its blob id. Under `core.autocrlf=true` neither ever matched, so every file an agent's
`git checkout`, `restore`, `stash`, `pull` or `merge` rewrote was audited as the agent's own edit, and a shell edit of
a tracked file lost its earlier text.

`gitCrlfPaths` (`src/engine/git.ts`) asks Git, in one `git check-attr --stdin`, for each file's `text` and `eol`
attributes, and reads `core.autocrlf` and `core.eol`. It names the files a checkout writes with CRLF as Git decides:
an `eol` attribute as it says, never a `-text` file, a file with no `text` attribute only under `core.autocrlf=true`,
and a text file under `core.autocrlf=true`, else by `core.eol`, whose native default is CRLF on Windows alone. For
those files only, the comparisons also try the CRLF form: in the walk's Git verdicts, the recovery of a preimage the
session did not capture, and the search of the branches a file was restored from. `src/engine/code-rule/line-endings.ts`
holds every such comparison: `heldAs` says whether a file holds a blob and in which form, `textAs` gives a blob's text
in that form, and `blobIds` gives the blob ids a file's text can stand for. A text names an LF blob only when a
checkout writes that blob as the text, so a text with an LF and no CR before it, a lone CR, or a NUL names none. A
preimage recovered from Git keeps the CRLF line breaks the file held, so the diff lines up line by line. A shell
command that rewrites an LF file with CRLF where Git would not is still the agent's edit.

</details>

<details>
<summary>Rerail's own checkout</summary>

`.gitattributes` sets `* text=auto eol=lf`, so every checkout writes text files with LF whatever `core.autocrlf` says.
`model/config.json` and `model/tokenizer.json` are `-text`: `model/manifest.json` pins their SHA-256 and the engine
refuses a model whose bytes differ. In a CRLF checkout every hook of a checkout build failed with `model artifact
checksum mismatch for config.json` and nothing was recorded, and the pack READMEs failed the structure checks. A test
checks that every file the manifest pins is `-text`.

</details>

## Each coding agent's shell

Rerail writes every command it hands an agent or a person in the syntax of the shell that runs it. On Windows,
Claude Code runs commands in Git Bash, and Codex and Cursor in PowerShell; each agent runs hooks under a parent of its
own. Which shell an agent runs on Windows is a field of its capability profile, `windowsCommandShell` in
`src/engine/adapter-contract.ts`: `posix` for Claude Code, OMP and Pi, `powershell` for Codex and Cursor. One module,
`src/engine/command-line.ts`, writes a command line for a shell the caller names (`commandLine`, `shellWord`,
`placeholderWord`), never for a platform. Hook commands are the exception: registration recognizes a hook it wrote by
its exact text, so `src/setup/harness-registration.ts` writes and reads those itself.

| Agent | Hook command | Commands it runs |
| --- | --- | --- |
| Claude Code | Exec form, no shell: `node <hook launcher> <recorded node> <hook>` | Git Bash: `'<node>' '<cli.js>'`, and the status line with POSIX quoting |
| Codex | PowerShell: `$LASTEXITCODE = 1; if (Test-Path -LiteralPath '<node>') { & '<node>' '<hook>' } else { node '<hook>' }; exit $LASTEXITCODE` | PowerShell: `& '<node>' '<cli.js>'` |
| Cursor | PowerShell, the event piped in: `Invoke-Command -ScriptBlock { if (Test-Path -LiteralPath '<node>') { $input \| & '<node>' '<hook>' } else { $input \| node '<hook>' } }` | PowerShell: `& '<node>' '<cli.js>'` |

OMP and Pi load Rerail as an extension and run no hook command; a steer writes their commands for Git Bash.

Claude Code on Windows needs Git Bash. Claude Code itself runs without it, running commands in PowerShell instead.
But `windowsCommandShell` is fixed per agent, not per machine, so Rerail's status line, command files and set-up's
answers for Claude Code stay POSIX words that PowerShell cannot run. Its exec-form hooks run without any shell.
Supporting Claude Code without Git Bash is [#558](https://github.com/neralabs/rerail/issues/558).

<details>
<summary>Why each form</summary>

- **Claude Code's exec form.** Claude Code runs a shell-form hook through Git Bash on Windows, which rewrites
  `cmd /d /c` into a drive path; cmd then reads the hook's JSON from stdin as commands.
- **Cursor's pipeline element.** Cursor runs a hook's command in PowerShell as `<event> | & { $input | <command> }`. The
  command is a pipeline element there, so Codex's form, which starts with an assignment, cannot parse, and a `cmd`
  form breaks at its nested quotes and `$PATH`. `Invoke-Command` hands the event to its script block as `$input`, which
  goes on to node's stdin. Registration recognizes the `cmd` form earlier set-ups wrote and replaces it.
- **PowerShell's call operator.** PowerShell takes a line that starts with a quoted path for a string and refuses the
  words after it, so `"<node>" "<cli.js>" start` failed. `& '<path>'` runs it.
- **PowerShell literals.** A single-quoted string keeps spaces, `$`, backticks and `%` as they are. PowerShell ends it
  at `'` and also at `‘`, `’`, `‚` and `‛` (U+2018, U+2019, U+201A, U+201B), so `powershellLiteral`
  (`src/engine/command-line.ts`) writes each of the five twice.
- **Git Bash's single quotes.** Claude Code's command files put each path in single quotes, which Git Bash reads as
  written. Inside double quotes it would expand a `$` or backtick in the path and read a UNC path's `\\` as one `\`.
- **`%`.** Paths keep each `%` single. Only a batch file turns `%%` back into `%`, and no shell these commands reach
  is one.
- **Plugin manifests.** The Claude Code plugin quotes `${CLAUDE_PLUGIN_ROOT}`, which Git Bash would split at a space in
  the profile folder. The Codex plugin uses the brace form `${PLUGIN_ROOT}`, the only form Codex replaces: neither
  PowerShell nor cmd.exe expands `$PLUGIN_ROOT`.
- **Set-up's answers.** On Windows, Codex's and Cursor's `/rerail-start` passes `--powershell`, so every answer command
  set-up prints is `& '<node>' '<cli.js>' setup ...` and carries the flag on: the installed CLI
  (`<rerail home>\current\dist\cli\cli.js`), or the one running set-up until it has installed one. Without the flag,
  Claude Code's case, set-up writes each answer as POSIX words starting with the sh launcher `rerail`, so a Windows path
  goes in single quotes for Git Bash. No answer runs `rerail.cmd`: cmd.exe reads a batch file's arguments again, so a
  `&` in a folder name would end the command there. `CARRIED_FLAGS` in `src/cli/setup-cli.ts` names the on-off flags
  every answer carries.

</details>

<details>
<summary>The feedback command a steer names</summary>

On Windows the agent's line goes on stdin in both shells, each for its own reason. The steer takes the agent's shell
from the session's coding agent's `windowsCommandShell` (`steerShell` in `src/engine/code-rule/rerail-steer.ts`):

- Git Bash: `echo '<that line>' | rerail feedback <rule-id> -`. Git Bash runs `rerail` as the sh launcher, never
  `rerail.cmd`, and as the launcher starts node it rewrites an argument that starts with `/` into a Windows path: the
  line `/v1 is exempt` would arrive as `C:/Program Files/Git/v1 is exempt`. It passes a pipe on as it is.
- PowerShell: three lines, `$OutputEncoding = [Text.UTF8Encoding]::new(); @'`, then a space and the line, then
  `'@ | rerail feedback <rule-id> -`. In PowerShell `rerail` is `rerail.cmd`, whose arguments cmd.exe reads again and
  splits at a `&` or `|`. PowerShell reads nothing inside the here-string, so no quote in the line has to
  be written twice, though PowerShell reads all five quote characters as single quotes, and none can end the string
  early and run the rest as a command. The space keeps a line that starts with a quote and `@` from closing it, and
  `rerail feedback` trims it. The first line has Windows PowerShell 5.1, which every Windows 10 and 11 install ships,
  pipe UTF-8: by default it pipes ASCII and turns every other character into `?`.

</details>

<details>
<summary>The `rerail` launchers and rule evidence</summary>

- Set-up writes `rerail.cmd` for cmd.exe and PowerShell, and an sh launcher `rerail` beside it for Git Bash. In Git
  Bash's own window (mintty) with pseudo consoles off, Windows programs get pipes, not a console, so set-up saw no
  terminal and gave a person the text meant for an agent. The sh launcher starts node through Git for Windows' winpty
  there, as Git for Windows' own `node` alias does.
- `rerail rules evidence` counts PowerShell's and cmd's recursive deletes as it counts `rm -rf`: `Remove-Item` or an
  alias (`rm`, `ri`, `rmdir`, `rd`, `del`, `erase`) with `-Recurse` and `-Force` in any prefix PowerShell accepts, and
  cmd's `rd /s` or `rmdir /s`, in any case.

</details>

## Hooks an agent gave up on

On Windows every hook process exits once the process that started it is gone (`exitWhenParentGone` in
`src/harness/parent-watch.ts`). The Claude Code, Codex and Cursor hook entries each make one call,
`serveHookEntry` (`src/harness/hook-adapter.ts`), which starts that watch and then serves the hook when the module is
the one node started.

A coding agent that times out a hook kills the process it started. On macOS and Linux that is node, since the sh
launcher `exec`s it. On Windows it is the parent node runs under: PowerShell for Codex and Cursor, the hook launcher
for Claude Code. Windows never kills a child with its parent, so the hook ran on holding its session's lock, and the
session's next hook waited up to 30 s for it. The watch checks the parent once a second. Since the watch does nothing
off Windows, `test/harness/hook-entries.test.ts` checks that `serveHookEntry` ends a hook whose parent is gone while
the hook still runs, and that each entry module serves its hook through `serveHookEntry`.

## The engine pipe

On Windows the daemon's engine listens on a named pipe, `\\.\pipe\rerail-<24 hex digits>`, from the SHA-256 of the
database path (`enginePipeName`, which `defaultEngineSocketPath` in `src/daemon/daemon.ts` calls). Any local user can
work out that name and create it first, so the daemon start never trusts whoever holds it. One module,
`src/runtime/engine-pipe.ts`, writes and reads every engine pipe name: the prefix (`PIPE_PREFIX`), the name, the
fallback (`fallbackPipe`), and whether a listed pipe is an engine's (`isEnginePipe`, `pipeListed`,
`enginePipeListed`). The daemon writes the names and the daemon's control reads them.

<details>
<summary>The proof, and the fallback pipe</summary>

- When the name is held (libuv reports `EACCES` or `EADDRINUSE`), the start sends the holder a random challenge on
  `POST /proof` (`engineHoldsCapability` in `src/runtime/engine-proof.ts`). Only an engine holding this home's access
  capability can answer: the answer is an HMAC-SHA256 of the challenge keyed by the capability, which tells the holder
  nothing about it. The capability never leaves the daemon. An answer that is not a proof as the engine writes one
  (another type, another length, a character outside lowercase hex, such as a multibyte one) is refused before the
  comparison, and a comparison that throws counts as a refusal, so such a holder does not hold the capability.
- A holder that proves it is this home's engine stops the start: a second daemon must not run beside it. Any other
  holder is a squatter, and the engine listens on the same name with a random hex suffix, recorded in `daemon.json` as
  `kernelSocketPath`, where hooks read it.
- Rerail 0.1.3 listened on the name with no check and no fallback, so a pipe another user created first made every
  daemon start fail. A probe that sent the capability to learn who holds the pipe would hand it to that user.

</details>

## Daemon liveness and stop

On Windows a recorded daemon counts as alive only while its engine pipe exists, and `rerail daemon stop` asks the
daemon to close itself before it kills anything.

Windows ends the daemon without running its `SIGTERM` handler at every sign-out and restart, and `process.kill` ends a
process at once there, so `daemon.json` and the main-instance record outlive the daemon, and Windows soon gives its pid
to another process. Rerail then took that process for the daemon: it probed the recorded URL with the access
capability, refused every start as "alive but not answering", and `rerail daemon stop` ended the unrelated process.

<details>
<summary>The lookup, elevated daemons, and the stop</summary>

- `lookupDaemon` (`src/runtime/daemon-control.ts`) lists `\\.\pipe\` without connecting to any pipe and treats a live
  pid whose recorded pipe is gone as a stale state file: it removes the file, probes nothing and kills nothing. A
  machine whose pipes cannot be listed counts as holding the pipe.
- `kill(pid, 0)` answers `EPERM` alike for another user's process, a SYSTEM service and this user's own elevated one.
  A pid this user may not signal counts as the recorded daemon only when its engine pipe exists and the engine there
  proves it holds the capability `daemon.json` records, the challenge the engine start uses. Nothing goes to the
  recorded URL before that. macOS and Linux treat `EPERM` as another user's.
- `recordedDaemonAlive` (`src/runtime/daemon-control.ts`) makes both checks above, and every reader of `daemon.json`
  asks it before it sends the capability the file records to the URL it records: `lookupDaemon`, the MCP bridge
  (`src/api/mcp-bridge.ts`), and the status line, the rules link and the start check
  (`src/presentation/session-presentation.ts`). So each of them finds a daemon started elevated, and none sends the
  capability to a process on a reused pid. `processState` (`src/engine/process-liveness.ts`) reads `kill(pid, 0)` once
  for it and for every other pid check.
- The uninstall counts the pid in `main-instance.json` as a daemon that holds the content cache only while a process
  holds `main-instance.lock` beside it (`mainInstanceHolderRuns`): the OS drops that lock however the daemon ends, so
  neither a reused pid nor another Rerail home's daemon keeps the cache. Where the native binding does not load to try
  the lock, `daemonMayHold` counts the pid while it is live and some Rerail engine pipe exists.
- `stopDaemon` sends `POST /native/shutdown` on Windows, with the capability and the native token only `daemon.json`
  holds, and waits up to 10 s for the daemon to close: requests other sessions have in flight finish, and the daemon
  removes its state file and main-instance record. It kills only a daemon still running after that. macOS and Linux
  keep `SIGTERM`. [threat-model.md](threat-model.md) covers the route's access rules.

</details>

## The Claude Code status line

On Windows Rerail's status line runs the person's previous status line through Git Bash, as Claude Code ran it, and
ends its whole process tree when it overruns its 2 s budget.

Node's `shell: true` is cmd.exe on Windows, which reads `~`, single quotes and `$(...)` as plain text, so a bash
status line broke once Rerail wrapped it. `previousStatusSpawn` (`src/hosts/claude/claude-status.ts`) runs it with
Git Bash's `bash.exe`: `CLAUDE_CODE_GIT_BASH_PATH`, as Claude Code reads it, else the one beside the Git Rerail runs
(`gitBash` in `src/engine/git.ts`), else cmd.exe when there is no Git Bash. Finding that Git stops at the first Git
that runs (`findGit`), which the status line pays for on every refresh.

On a timeout, the status line ends the shell's tree with `endProcessTree` (`src/engine/child-process.ts`), the one
helper that ends a child and what it started. On Windows it runs `taskkill /T /F` on the shell first, while it still
runs, since taskkill finds the tree from a live root, and kills the shell alone only when taskkill fails. Killing the
shell first left Git Bash's real `bash.exe`, and everything the status command started, running with its folder open
after every slow refresh. A shell that has already exited, while a process it started still holds its output, is left
alone: no tree is reachable from it, and Windows may have given its pid to an unrelated process, whose tree taskkill
would end. The line settles at the budget, without waiting for the pipe to close. A Git batch read that times out and
the updater's health check end their children the same way.

## Private folders

Windows ignores the `0700` and `0600` modes Rerail gives its folders and files, so a new folder takes its parent's
access control list (ACL). Rerail gives the folders that hold its secrets an owner-only ACL when they are made outside
the profile folder.

Under the profile folder, the inherited ACL already grants the person, SYSTEM and Administrators alone. A folder made
under `C:\` lets Users read, and Authenticated Users change, what is in it. A Rerail home there (`RERAIL_HOME`) holds
the access capability and every prompt in `rerail.sqlite`, and a checkout's `.rerail\sessions` there holds prompts,
preimages and held steers.

<details>
<summary>How the ACL is set and read</summary>

- `src/engine/storage/private-files.ts` is the one interface on every system: `keepFolderPrivate` keeps a folder its
  owner's, and `openToOthers` says who else may read a secret. On macOS and Linux they set and read mode bits; on
  Windows the folder's ACL. Callers make one call and never branch on the platform.
- `src/engine/storage/windows-acl.ts` sets an ACL with `icacls`: protected, granting the person's SID, SYSTEM and
  Administrators full control, inherited by everything in the folder. It reads one with Windows PowerShell's
  `Get-Acl` in SDDL, which names accounts by SID in any language. Both run by their System32 paths.
- The person's SID comes from `whoami /user`, whatever the account: `S-1-5-21-...` for a local or domain account,
  `S-1-12-1-...` for a Microsoft Entra ID (Azure AD) account, as on most company laptops.
- A Rerail home or `.rerail\sessions` folder Rerail makes outside the profile gets the ACL when it is made, and a home
  an earlier release made gets it at the next daemon launch. `rerail remote add` gives the `remote-tokens` folder the
  ACL before it writes a token there, so a remote's token is private before any daemon launch. A failure is reported on
  stderr and never fails the caller.
- `loadDaemonTls` refuses a TLS private key that `openToOthers` finds other accounts can read, naming those accounts
  and the `icacls` command that fixes it, and refuses a key whose ACL it cannot read. The daemon then serves this
  computer alone, on loopback, and says why (`serveRerailDaemon`), so hooks keep recording while the key is fixed.

</details>

## The content cache, updates and removal

On Windows the content cache lives outside the roaming part of the profile, and updates and removals work around
files Windows will not replace while they are in use.

<details>
<summary>Where Windows changes what an update or a removal does</summary>

- **Content cache.** The managed installation keeps the machine-wide content store in `%LOCALAPPDATA%\rerail`
  (`mainContentStoreDirectory` in `src/engine/native/rerail-build.ts`), not `~/.cache/rerail`. A roaming profile
  copies everything in the profile folder but `AppData\Local` at each sign-in and sign-out, so the store slowed every
  sign-in, and of two machines signed in at once the last to sign out overwrote the other's SQLite files.
- **Loaded native files.** Windows refuses to overwrite or delete a DLL a process has loaded, and Node never unloads an
  addon. `rerail update` stops a daemon running from the package it is about to rebuild better-sqlite3 in. The
  uninstall reads the recorded checkouts through `node:sqlite`, which loads no file from the installation, so it can
  delete the versions folder it ran from.
- **The `current` junction.** Windows cannot rename a junction over another, so the swap takes two renames, and a kill
  between them left no `current`. The updater puts back the newest `current.*.old` it finds.

</details>
