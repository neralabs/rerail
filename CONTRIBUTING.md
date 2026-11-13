# Contributing to Rerail

Thank you for helping. This guide covers how contribution works, how work starts, how to set up a checkout, what a
pull request needs, and the rules that keep the project open source and its licensing clean.

Everyone taking part follows the [Code of Conduct](CODE_OF_CONDUCT.md). Report a security vulnerability privately, as
[SECURITY.md](SECURITY.md) describes, never in a public issue or pull request. Who the maintainers are and how
decisions are made is in [docs/GOVERNANCE.md](docs/GOVERNANCE.md).

## How contribution works

Rerail is open source with gated contribution. Anyone can propose a change, from their fork. Code reaches this
repository only through a pull request from a branch here, which contributors and the project's own automation open.
Nera Software employees are contributors by being on the team; anyone else becomes one by approval.

| | Anyone | Contributors |
| --- | --- | --- |
| Report a bug or a rule misfire, request a feature or an integration, ask a question | Yes, in an issue | Yes |
| Propose a change from a fork: a pull request, or a branch linked from an issue ([below](#proposing-a-change-from-your-fork)) | Yes, with no CI | Not needed |
| Push branches here and open pull requests from them, with CI | [Become a contributor](#becoming-a-contributor) | Yes |
| Claim a `good first issue` or `help wanted` issue | Become a contributor | Yes |

The automation is Dependabot, for dependency updates, and the GitHub Actions workflows that open the **Release** pull
request (`release-pr.yml`) and upstream bump pull requests (`upstream-bump-publish.yml`) as the `neralabs-ci` app.
Their pull requests are reviewed and merged like any other.

### Proposing a change from your fork

Anyone can write the change themselves:

1. Open an issue for it, or find the one that covers it.
2. Fork the repository and make the change on a branch in your fork, with tests, as
   [Before you open a pull request](#before-you-open-a-pull-request) describes.
3. Open a pull request from your fork against `main`, or link the branch in the issue.

No CI runs on a fork's pull request. GitHub holds every fork run until a maintainer approves it, and maintainers do not
approve them, since CI is costly to run. Run the checks yourself and say in the pull request what you ran.

A maintainer reviews it there. If it goes in, the maintainer ports it onto a branch here in their own pull request,
credits you with a `Co-authored-by:` trailer on the commit, and closes your pull request with a link to the new one.
Pack data your change stages in `publish/` comes in with the ported pull request and is published from it. What you
submit is licensed under Apache-2.0 ([Licence of contributions](#licence-of-contributions)), and
[Third-party material](#third-party-material) applies to it like any other change.

### AI tools and automated accounts

You can use AI tools, coding agents included, to write a change. A person still stands behind it: they answer the
review, can explain any part of it, and ran the checks the pull request reports.

A maintainer closes a pull request from an account that looks automated when no person behind it can be reached: for
example, a new account opening pull requests across many unrelated repositories. Maintainers don't have the capacity to
review automated contributions at that volume. Closing it is not a judgement on the code, and a maintainer may fix the
issue in their own pull request. If you are the person behind such an account, email
[opensource@nera.software](mailto:opensource@nera.software) to talk about the change.

## Contributors

Contributors push branches to this repository and open pull requests from them, so CI runs on their changes. Nera
Software employees are contributors by being on the team. Anyone else applies, and the maintainers approve them. Being
a contributor is not a path to maintainer: Nera Software appoints maintainers
([docs/GOVERNANCE.md](docs/GOVERNANCE.md#who-the-maintainers-are)).

### Becoming a contributor

Ask in the issue you want to work on, or open one with the
[contributor form](https://github.com/neralabs/rerail/issues/new?template=5-contributor.yml): say what you want to
change and how you would go about it. For anything in [Start with an issue](#start-with-an-issue), that is the
conversation a maintainer has with you anyway.

Maintainers look at:

- the issue itself: a reproduction, a failing case, or an approach that holds up;
- changes you proposed from your fork, and how they went;
- whether the change fits where Rerail is going, at a size you can finish;
- how you have worked in the issues so far, under the [Code of Conduct](CODE_OF_CONDUCT.md).

A maintainer answers in the issue. If it's a yes, they add you to the list below and assign you the issue. If it's not
yet, they say why and what would change it.

Push access for contributors outside Nera Software comes once the repository is set up for it: secrets and OIDC
scoped to branches, and a role narrower than write. Until then, an approved contributor proposes from their fork like
anyone else
([Proposing a change from your fork](#proposing-a-change-from-your-fork)).

### The list

Contributors from outside Nera Software are listed here. The list changes only through a maintainer's pull request
that links the issue where it was agreed, so every name on it has a public reason.

Contributors outside Nera Software: none yet.

### Leaving the list

- Ask, in any issue or at [opensource@nera.software](mailto:opensource@nera.software), and you are taken off.
- Twelve months without a pull request, issue or comment here takes you off. Ask again any time.
- Breaking the Code of Conduct can take you off, as its enforcement steps describe.

Leaving the list removes your push access.

## Start with an issue

Open an issue before you start on:

- a new feature, or a change to how Rerail behaves;
- a new rule, rule pack, language or coding-agent integration;
- a change to the rule-file format, the API, or what a pack publishes;
- anything that spans several parts of the repository.

Agree on the approach with a maintainer in the issue before you write the code. A pull request for work nobody agreed
on may be closed, however good it is.

Contributors take these straight to a pull request, with no issue; from a fork, they are a
proposal like any other:

- typo and documentation fixes;
- a bug fix with a test that fails without it;
- a rule's false positive or false negative, with the code that triggers it added as a test.

### Claiming an issue

Issues labelled `good first issue` or `help wanted` are open to any contributor, and asking for one is a good way to
become one ([Becoming a contributor](#becoming-a-contributor)). Check that the issue is not assigned, then comment on it
to claim it; a maintainer assigns it to you. If there is no pull request or update from you within two weeks, the issue
is open to others again.

## Contribution flow

**From a branch here** (contributors):

1. Find an issue, or open one ([Start with an issue](#start-with-an-issue)).
2. Claim it or assign it to yourself, and agree on the approach with a maintainer when the issue needs one.
3. Create a branch from `main` in this repository.
4. Set up your checkout ([Development setup](#development-setup)), make the change, and add tests for the behavior it
   changes.
5. Run the checks in [Before you open a pull request](#before-you-open-a-pull-request).
6. Open a pull request against `main` that links the issue (`Fixes #123`) and says how you tested the change.
7. A code owner reviews it, and the `neralabs-reviewer` bot may too (never on a pull request from a fork). The bot
   approves when it finds nothing blocking and the pull request touches no high-risk path in
   [REVIEW.md](REVIEW.md#high-risk-areas); that approval counts as the one main requires. Push fixes to the same
   branch. Pack data the pull request stages is published by a maintainer first
   ([docs/upstream.md, "Publishing pack data"](docs/upstream.md#publishing-pack-data)); once it is approved and CI
   passes, it is squash-merged.

**From a fork** (anyone): steps 1, 4 and 5 as above, on a branch in your fork, then open a pull request from it or link
the branch in the issue ([Proposing a change from your fork](#proposing-a-change-from-your-fork)).

## Getting help

Ask in the issue you are working on, or open an issue with your question.

### Response times

These are the times we aim for, counted in business days: a first response within 5, and a decision within 10. For a
bug or a rule misfire we also aim to have the fix merged to `main` within the 10 days; when it ships depends on the
next release ([docs/release.md](docs/release.md)).

| You sent | First response | Decision |
| --- | --- | --- |
| A vulnerability report ([SECURITY.md](SECURITY.md)) | 2 business days | As [SECURITY.md](SECURITY.md) describes |
| A conduct report ([CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md#reporting)) | 3 business days | As the [Code of Conduct](CODE_OF_CONDUCT.md#what-happens-next) describes |
| A bug report or rule misfire | 5 business days: triaged, labelled, or asked for a reproduction | 10 business days: what we will do, with a reason; the fix is the aim |
| A feature or integration request | 5 business days | 10 business days: accepted, declined, or "not now" with a reason |
| A change from your fork ([Proposing a change from your fork](#proposing-a-change-from-your-fork)) | 5 business days | 10 business days: ported, or declined with a reason |
| A request to become a contributor | 5 business days | 10 business days: yes, or not yet with what would change it |
| A question | 5 business days | 10 business days: answered |

If an issue or pull request gets no reply within its window, comment on it to bring it back up.

## Development setup

One repository holds everything: the core package at the root, rule packs under [`packs/`](packs/), and the engine's
language conformance fixtures under [`fixtures/`](fixtures/) (test data, never published).

Install [Git LFS](https://git-lfs.com) before you clone, and run `git lfs install` once. The embedding model's weights
(`model/model.safetensors`) and the engine's Unicode names table (`native/src/code_rule_dsl/value/names/names.bin`,
compiled into the engine) are stored in LFS (`.gitattributes`); without it a checkout holds pointer files, and neither
the engine build nor the model works: `npm run build:native` and `rerail install --root` stop on a pointer, naming the
file. In a checkout made without it, run `git lfs install && git lfs pull`.

```sh
git clone https://github.com/neralabs/rerail.git
```

`RERAIL_ORACLES_DIR` points the build and tests at another fixtures directory.

Prerequisites:

- Node.js 24.18 or later, for its npm (11.16 or later), which reads `allowScripts` in `package.json`. That denies
  better-sqlite3's install script, so `npm ci` uses the addon better-sqlite3 ships prebuilt instead of compiling it.
  An older npm runs the script, which fails without Python and C++ build tools. `npm run check:install-scripts` fails
  when a bump leaves the deny on the old version.
- Rust through [rustup](https://rustup.rs/), to build the native engine. `rust-toolchain.toml` pins the toolchain, so
  `cargo` installs and selects it on first use. Without Rust, use a released engine ([Without Rust](#without-rust)).
- [`just`](https://github.com/casey/just) for the dev recipes (`just rerail-dev`, `just nuke-rerail`), or the command
  each recipe runs, which the [`Justfile`](Justfile) shows.
- The AWS CLI v2, signed in with read access to the pack test data, for the pack tests and the conformance gate
  ([Pack test data](#pack-test-data)). Without access, `npm run check`, the builds and `npm run test:scripts` still
  run, and so do the tests that read no pack data; the ones that do fail, each saying to pull it.

On Windows, see also [Windows](#windows).

```sh
npm ci                    # also installs the pre-commit hook (below)
npm run build:native      # compile the native kernel into native/
npm run build             # clean build: native kernel, runtime, host bundles, UI
npm run check:all         # TypeScript, rustfmt + clippy, UI token and reference checks
npm run test:all          # build, unit, conformance, reference, native, and integration suites
npm run test:conformance  # the language conformance gate, offline
npm run package:check     # build and dry-run the npm pack
```

`npm ci` and `npm install` (`prepare`) install a Git pre-commit hook that runs `scripts/check/pre-commit.sh`, the
checks of CI's "Lint and type-check" job: the Justfile's format, the release notes your branch adds against your local
`origin/main`, `npm run check`, `check:ui-tokens` and `check:reference`. It checks the working tree, not only what is
staged; `git commit --no-verify` skips it. A `pre-commit` hook of your own is left in place.

The pack tests also need the pack test data: run `npm run fixtures:pull` ([Pack test data](#pack-test-data)).

`npm run build:native` needs `cargo` on `PATH`; after a fresh rustup install, `export PATH="$HOME/.cargo/bin:$PATH"`.
On Windows, see [Developing on Windows](#developing-on-windows).

### Without Rust

`npm run engine:pull` puts a released engine in `native/` instead of building one: it runs
`npm pack @neralabs/rerail@<version>` for the version `package.json` names (`-- --version <version>` for another) and
copies its `native/rerail_engine.<platform>.node`. When `native/`, `Cargo.toml`, `Cargo.lock` or `languages/` changed
since that release's tag (`git diff --stat v<version> -- native/ Cargo.toml Cargo.lock languages/`), it warns and
lists the changes: the engine is older than the checkout, and tests of those changes need `npm run build:native`.
`RERAIL_ENGINE_PATH` makes the runtime load an engine from another path instead.

### Windows

- **Rust.** The toolchain targets MSVC, so it needs Visual Studio's C++ tools: Visual Studio or its Build Tools with
  the "Desktop development with C++" workload, or just the "MSVC v143 build tools" and a "Windows 11 SDK". Install
  rustup with `winget install Rustlang.Rustup` (or `rustup-init.exe` from [rustup.rs](https://rustup.rs/)), then
  open a new terminal. In a PowerShell already open, `$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"` puts
  `cargo` on `PATH`.
- **`just`.** `winget install Casey.Just`. It runs each recipe with `sh`, so use it from Git Bash, or put Git's `bin`
  folder (`C:\Program Files\Git\bin`) on `PATH`. Without `just`, run the command the recipe runs.
- **Line ends.** A clone made before `.gitattributes` set `* text=auto eol=lf` has files with CRLF line ends, and the
  model check, structure metrics #8 and #24 and 25 pack tests fail in it. After committing or stashing your work,
  check every file out again once:
  `git rm --cached -r . && git reset --hard`
  ([Troubleshooting](docs/troubleshooting.md#windows-checks-and-pack-tests-fail-in-a-clone-made-before-lf-line-ends)).

### Each component

| Component | Where | Build | Check and test |
| --- | --- | --- | --- |
| Runtime, CLI and coding-agent hosts | `src/` | `npm run build` | `npm run check`, `npm test` |
| Native engine (Rust) | `native/` | `npm run build:native` | `npm run check:native`, `npm run test:native` |
| Rule packs | `packs/` | none | `npm test` (the pack validation, `test/engine/catalog/packs.test.ts`), `npm run conformance:languages -- --pack <id>` |
| Dashboard | `ui/` | `npm run build:ui` | `npm run check:ui-tokens` |
| Integration suites | `integration/` | `npm run build` | `npm run test:integration:built` (needs a real OMP, below) |
| Repository scripts | `scripts/` | none | `npm run test:scripts` |

Generated files are listed, with the command that rewrites each, in
[scripts/generate/README.md](scripts/generate/README.md); edit the generator, not its output.

### Development builds

Everything run from a checkout (tests, benchmarks, `node dist/cli/cli.js`) is a development build: its policy content
store is `<checkout>/.rerail/content-v4.sqlite`, private to that checkout or worktree. Only the managed installation
under `~/.rerail/versions/` opens the machine-wide `~/.cache/rerail/content-v4.sqlite`, so a checkout on a newer store
schema cannot break the installed Rerail. `RERAIL_CONTENT_STORE_PATH` overrides the location; the test setup gives
every test file its own store that way. See [The content store](docs/how-setup-works.md#the-content-store).

To run a checkout's build in your harnesses instead of the published package, run `just rerail-dev --install-bridge`.
It builds the checkout, activates the checkout as `~/.rerail/current`, the way `setup` activates a published version,
writes the launcher, and registers the harnesses; run `npx @neralabs/rerail setup` again to return to the published
package.

Rerail's own toolchain is Node only. OMP remains an external Bun-hosted runtime; its integration suites
(`integration/omp-*.test.ts`, `integration/host-auth.test.ts`) use Vitest to drive a real standalone OMP executable.
Set `RERAIL_OMP_EXECUTABLE` to its absolute path, or provision the release pinned to the SDK dependency with
`node scripts/host/install-omp-host.mjs` (checksum-verified; uses a private local cache and prints the executable path).
CI uses the same host. `npm run test:integration:built` skips the build step when `dist/` is current.

`scripts/generate/model/prepare_model.py` reproduces the bundled INT8 artifact from the revision and checksums recorded
in `model/manifest.json`.

`npm run conformance:languages -- --pack <pack>` scores a language pack's ported linter rules (`packs/<pack>/`) through
the production workspace against the tool's expected findings in the pack's `upstream/` and, with `--samples`, on the
tool-packs' labelled real-code samples (`samples.lock`, [docs/upstream.md](docs/upstream.md#samples));
`--write` saves the report under `.rerail/reports/`. `npm run conformance:languages:check` is the gate over the fixtures,
their rename variants and the samples ([conformance/README.md](conformance/README.md#the-gate)); it runs offline in
`npm run test:conformance`, part of `test:all` in CI.

### Operational commands (`just`)

The root [`Justfile`](Justfile) wraps the day-to-day operations; `just` lists them. Each recipe runs a script under
`scripts/` (or `conformance/`, `benchmark/`), so the same command works without `just`.

| Recipe | Does |
| --- | --- |
| `just rerail [--tag next] [setup args]` | Installs the published package on this machine (`npx @neralabs/rerail setup`). `RERAIL_TAG` selects the dist-tag; `--restore [dir]` then restores a `nuke-rerail` backup. |
| `just nuke-rerail [--dry-run \| --yes]` | Removes Rerail from this machine (daemon, coding-agent registrations, `~/.rerail`, every checkout's dev daemon), moving it all to `~/.rerail-clean-YYYYMMDD[-HHMMSS]/` for `just rerail --restore`. |
| `just rerail-native` | Installs rustup and the pinned toolchain if needed, then builds the native engine. |
| `just rerail-setup` | Reserves this checkout's dev instance (`.rerail/dev/instance.json`: a stable loopback port in 35000–35999 and the home `.rerail/dev/home`) without starting anything. |
| `just rerail-dev [--install-bridge] [--restore [dir]]` | Rebuilds whatever is stale and (re)starts this checkout's dev daemon on its own home and port, serving `ui/` live. `--install-bridge` registers this checkout's build with every coding agent (`rerail install --root`); hooks always use the default home, so a bridged checkout's dev daemon runs on `~/.rerail` until another `rerail setup` replaces the bridge. `--restore` loads a `nuke-rerail` backup's database. |
| `just rerail-status`, `just rerail-down`, `just rerail-reset` | Shows the dev instance and build freshness; stops its daemon (state kept); deletes this repository's session history from the dev database (rules kept). |
| `just rerail-smoke` | Packs this checkout, publishes it to a throwaway authenticated local registry, and drives install, update, `nuke-rerail`/`--restore`, `install --remove` and `install --remove --delete-data` in a sandbox `HOME` (`KEEP=1` keeps it). |
| `just rerail-env-matrix run --tarball <file> [--env <ids>]` | Installs a package on throwaway Azure VMs (Linux and Windows, x64 and arm64) and checks it there as a user meets it; `--npm <spec>` installs from npmjs instead, `list` names the environments, `sweep` deletes leftovers. Needs `az login` ([docs/environment-matrix.md](docs/environment-matrix.md)). |
| `just rerail-language-conformance` | Language-pack conformance (`--pack <id>`, `--check`). |
| `just rerail-query-parity --corpus <dir> --manifest <paths.json>` | Compares a reference native addon (`RERAIL_QUERY_PARITY_REFERENCE_ADDON`) with this checkout's on a corpus. |

`npm run test:scripts` runs the recipes' tests.

### Live harness smoke (macOS)

Run the real-provider smoke as the final acceptance step, after the deterministic regression suite:

```sh
npm run test:e2e:host -- --host all
npm run test:e2e:host -- --host codex
```

It builds the package, then runs each harness's installed CLI against its real provider in a temporary workspace with
a private profile, a resident daemon and a `sandbox-exec` write boundary. It passes when a violating edit reaches the
file and Rerail records it, the executable-policy diagnostic arrives, exactly one conversation-rule delivery carries a
private nonce, and the model restores the file and quotes the nonce and the diagnostic.

Prerequisites:

- macOS with `sandbox-exec`, and the selected harness CLIs installed. `RERAIL_CLAUDE_BIN` and `RERAIL_CURSOR_AGENT_BIN`
  can select non-PATH executables.
- Provider credentials for each selected harness, which `configureHostAuth` in `scripts/host/host-harness.ts` reads; a
  missing credential or endpoint is a failure, never a skipped pass. An OAuth token copied from a harness login must
  outlast the session limit by a minute; refresh credentials are not copied.

| Variable | Default | Does |
| --- | --- | --- |
| `RERAIL_SMOKE_TIMEOUT_MS` | `180000` | Each provider session's hard limit. |
| `RERAIL_SMOKE_ATTEMPTS` | `1` (CI: `3`) | Reruns a harness in a fresh sandbox while its attempt is inconclusive. |
| `RERAIL_SMOKE_ACCEPTANCE` | `full` | `transport` (Codex and Cursor only) drops the checks on the model's restoration and final reply. |

The command prints one JSON evidence record per harness attempt (`status` `PASSED`, `FAILED` or `INCONCLUSIVE`) and
exits nonzero for any missing prerequisite or failed assertion. It leaves your daemon and harness profiles alone, and
removes its profiles and workspaces after each run. When an attempt counts as inconclusive, and how each harness is set
up, is in `scripts/host/host-smoke.ts` and `scripts/host/host-harness.ts`.

### Developing on Windows

Pull request CI runs the unit suite on Windows in one job (`unit-windows`), and every other test job on Linux or macOS.
Test a change to how Rerail starts processes, resolves paths, replaces files or writes commands on Windows yourself,
and keep to the rules in [docs/windows.md](docs/windows.md).

**Git, Git LFS and line endings.** Git for Windows includes Git LFS; run `git lfs install` once before you clone. Its
installer sets `core.autocrlf=true` for the whole machine, which checks text files out with CRLF line breaks.
`.gitattributes` sets `* text=auto eol=lf`, so a checkout writes LF whatever that setting says, and marks
`model/config.json` and `model/tokenizer.json` `-text`, so they keep their bytes: `model/manifest.json` pins their
SHA-256, and a CRLF copy made every hook of a checkout build fail, so nothing was recorded. A checkout made before the
`eol=lf` line keeps its CRLF files until Git writes them again. Commit or stash your work, then have Git write every
tracked file again:

```sh
git rm -r --cached -q . && git reset --hard
```

**Node and the native engine.** Install Node.js 24 or newer. `npm run build:native` compiles with Microsoft's C++
toolchain, so install the Visual Studio Build Tools with the C++ workload, and rustup:

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
winget install --id Rustlang.Rustup -e
```

Then open a new terminal so `%USERPROFILE%\.cargo\bin` is on `PATH`; a terminal inside VS Code or Cursor keeps the
editor's `PATH` until the editor restarts. rustup's default host on Windows is the MSVC one, and `rust-toolchain.toml`
picks the toolchain. `.cargo/config.toml` links the C runtime into the engine (`+crt-static`), so it loads on a
machine with no Visual C++ runtime. The first build writes `native/rerail_engine.win32-<arch>-msvc.node`; it took 23
minutes on a Windows 11 laptop. Install rustup this way before you run `just rerail-native`, which then only installs
the pinned toolchain and builds. Run `just` recipes from Git Bash: `just` runs each recipe with `sh`, which Git for
Windows puts on `PATH` there.

**Tests and your profile.** The vitest setup file (`test/support/setup-rerail-home.ts`) gives every test file its own
user home, Rerail home and content store. It sets both `HOME` and `USERPROFILE`, since `os.homedir()` reads
`USERPROFILE` on Windows. Git in a test reads none of your settings or the machine's: the setup file writes a fresh
`.gitconfig` in that home holding only a test identity, points `GIT_CONFIG_GLOBAL` at it and sets
`GIT_CONFIG_NOSYSTEM=1`, since Git for Windows' installer sets `core.autocrlf=true` for the whole machine, which once
wrote a test repository's LF files with CRLF. A test that moves the home calls `stubUserHome`
(`test/support/user-home.ts`), never `vi.stubEnv("HOME")` alone: on Windows that leaves `os.homedir()` on your real
profile, and a registration test once pointed every hook in a developer's `~/.claude/settings.json` at a temp folder
the run then deleted, so their machine recorded nothing. The setup file fails a test file that writes a temp path into
your real coding agent settings, naming each file; `rerail install --refresh` restores them.

Never run a probe script or a built CLI against your real profile either. Outside vitest, move the home in the shell
first, as vitest does (Git Bash), and remove `$T` afterwards:

```sh
T=$(cygpath -w "$(mktemp -d)")
export HOME="$T" USERPROFILE="$T" APPDATA="$T\\AppData\\Roaming" LOCALAPPDATA="$T\\AppData\\Local"
export RERAIL_AUTOSTART=0 RERAIL_MODIFY_PATH=0 RERAIL_DEVICE_SECRET_STORE=file
```

`RERAIL_MODIFY_PATH=0` keeps set-up out of your user `Path`, which lives in the registry, where no environment variable
redirects it. `RERAIL_DEVICE_SECRET_STORE=file` keeps activation out of Windows Credential Manager.

**Symlinks.** Windows makes a symlink only with Developer Mode on or from an elevated process. Rerail itself never
makes one there ([docs/windows.md, "Links"](docs/windows.md#links)), so run the suite without Developer Mode, as most
users' machines are: a test that needs a symlink skips and says why, or makes a junction where a junction tests the
same thing. Tests do either through `symlinkOrSkip` or `linkFolder` (`test/support/platform/links.ts`); the other
helpers for what differs on Windows (named pipes, permission bits, removing a folder Windows still holds) sit beside
them in `test/support/platform/`. CI's `unit-windows` job runs the whole unit suite on Windows on every push to main and
every pull request from a branch of this repository, so a test that fails there fails CI.

**PowerShell scripts.** `scripts/host/windows-install-smoke.ps1`, the release gate on Windows, runs under PowerShell 7
in `release.yml`. It dot-sources `scripts/host/windows-host.ps1`, which the environment matrix's Windows VMs also run
under Windows PowerShell 5.1, so keep that file working in both. Install PowerShell 7 with
`winget install --id Microsoft.PowerShell -e`, then run the smoke on a package you packed:

```sh
npm run build && npm pack
pwsh -NoProfile -File scripts/host/windows-install-smoke.ps1 -Tarball neralabs-rerail-<version>.tgz
```

Delete the tarball afterwards; git does not ignore it. The smoke installs into a sandbox profile whose folder name has
an accent and a space. It puts back your user `Path`, which set-up writes and the sandbox cannot redirect, and reads
every child's output as UTF-8, so it passes on a console whose code page is 437, an en-US install's default.

**Console windows.** On Windows, a console program started by a process with no console opens a window, which
Windows 11 hands to Windows Terminal, and it takes focus. When you change how Rerail starts a process, start it
through `src/engine/child-process.ts`, and only the browser opener through `spawnShown`; `npm run check` holds the
first. Then watch it on Windows 11 with Windows Terminal as the default terminal application, as it is out of the
box: run your build as the daemon your coding agents use (`just rerail-dev --install-bridge`), work a session in a
repository where Rerail is started, and look for any window that opens. A window that shows for a moment is easy to
miss. The tracer posted with the
[report of this problem](https://github.com/neralabs/rerail/issues/507#issuecomment-6754235764) logs every console
window shown, every focus change and every process start with its parent, and exits 1 when a console window appears:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File popups.ps1 -Seconds 60
```

Run `npx @neralabs/rerail setup` afterwards to return to the published package.

### UI

The browser view lives in `ui/` and has no toolchain: `index.html`, stylesheets under `ui/css/`, and native ES modules,
served as bytes. No React, no Vite, no Tailwind, no TypeScript, not even as dev dependencies.
[ui/README.md](ui/README.md) maps the modules and lists the routes; every route keeps the
`?repository=<key>&checkout=<path>` scope, and without one the dashboard lists the repositories to pick from. Colours
come from the committed `ui/tokens.generated.css` (and the terminal brand colour from
`src/presentation/terminal-brand.generated.ts`); `npm run check:ui-tokens` fails when the UI references a `var(--…)`
that neither the token sheet nor the page's own CSS declares.

`npm run build:ui` copies `ui/` to `dist/ui`, which the daemon serves. A daemon started with `--dev`, or with
`RERAIL_DEV=1` in its environment, serves `ui/` itself instead, so an edit needs only a browser reload; `rerail status`
and `/healthz` report it as `dev` rather than `bundled`.

## Before you open a pull request

Open a pull request against `main` that links its issue, if it has one. Changes are squash-merged after review.

- `npm run check` passes: the type check, the layer boundaries, the repository structure gate, Windows-safe paths,
  child processes started only through `src/engine/child-process.ts`, renames only through `src/engine/file-replace.ts`,
  test folders removed only through `test/support/platform/remove-folder.ts`, checkout paths resolved only through
  `src/engine/catalog/checkout-control.ts`, opens that refuse a symlink only through `src/engine/no-follow.ts`, no
  internal references (ticket IDs, `nera-` names outside the public list), and `allowScripts` denying the install
  script of every installed package that ships its addon prebuilt. While the pull request stages pack data in a
  `publish/`, structure metric #3 fails for each such tool-pack until the data is published.
- `npm test` passes, with `npm run build` run first (several suites run the built package from `dist/`).
- `npm run check:licenses` and `reuse lint` pass (`uvx --from 'reuse[charset-normalizer]==6.2.0' reuse lint`, as CI
  runs it).
- A change to a published pack under `packs/` adds a changeset, its name opening with its change type
  ([.changeset/README.md](.changeset/README.md)); a private pack takes none. It also stages the
  pack data it adds, changes or removes ([Adding or changing a pack](#adding-or-changing-a-pack)).
- A change to what the core package ships adds an entry as a file of its own in `.changelog/`, its name opening with
  its change type ([.changelog/README.md](.changelog/README.md)), never under `## Unreleased` in `CHANGELOG.md`.
- A changeset summary or changelog entry is a release note, and goes out as it is on the GitHub Release: it says what
  changed for the person upgrading, in plain language, in a sentence or two (at most 60 words), and leaves out commit
  hashes, ticket ids, pull request numbers and repository-internal maintenance; CI fails a pull request whose entry or
  summary says otherwise ([release notes](docs/release.md#release)).
- Tests cover the behavior you changed.
- Docs are updated in the same pull request, not a later one. A change to commands, coding agents or setup updates
  `README.md` and the pages under `docs/` that describe it, and `context7.json` where its rules or folders name what
  changed.
- A feature that enters, graduates from or leaves internal testing updates
  [docs/internal-testing.md](docs/internal-testing.md), which lists exactly what is in
  ([governance](docs/GOVERNANCE.md#internal-testing-features) says who approves a graduation or removal).
- It carries only approved Rerail context ([Keeping private context out](#keeping-private-context-out)).
- A change to what a package does keeps its npm page current: its `README.md`, and `keywords` and `repository` in its
  `package.json`, for core and for a pack.

Whoever merges the Release pull request checks the same of the release as a whole
([docs/release.md](docs/release.md#before-merging)).

### Keeping private context out

The repository, its pull requests and its CI logs are public. A contribution carries only context approved for Rerail:
what the change does and the public sources it rests on. Before you publish a branch or a pull request, check its
fixtures, test data, code comments, commit messages, pull request title, body and comments, and any agent session
traces or transcripts you attach or paste from, for:

- the names of Nera's private repositories and projects, or text quoted from them;
- customer names, and old product names (such as "Tempo");
- internal ticket IDs, internal URLs and links to internal tools: Notion, Slack, Orca shares, signed attachment URLs;
- personal paths and usernames (`/Users/<you>/…`, a home directory in a stack trace or a scratch path);
- anything about research participants or the people whose prompts or sessions a study read.

Replace each with a neutral description or a generic example (`/Users/me`, "a private web application"), keeping what
a test case tests. `npm run check:internal-refs` fails on internal ticket IDs and on any `nera-` identifier outside
its public list (`PUBLIC_NAMES` in `scripts/check/check-internal-refs.mjs`), in tracked paths and text. It sees only
that hyphenated form: the same name written with an underscore, a slash or as a scoped package passes, and so do
pull request text, links and personal details, so those are yours to check. A squash-merged commit message can't be
changed afterwards.

## Reporting a bug

Open an issue with:

- your Rerail version: the `Current:` line of `rerail update status`;
- the coding agent and its version (Claude Code, Codex, Cursor, OMP or Pi);
- your operating system and architecture, and whether you run it in WSL;
- what you did, what happened, and what you expected;
- for a rule: its id, its pack and the pack's version (`rerail packs list` prints `<pack>@<version>`), its mode (`log`
  or `steer`), and the steer text or the dashboard entry;
- the smallest code or steps that reproduce it.

Leave out secrets and code you cannot share; a smaller reproduction is better than a redacted one.

## Requesting a feature

Open an issue with the problem you have, the solution you would like, the alternatives you considered, and your
context: the coding agent, the language and the kind of repository.

## Large changes

For a change that spans components, changes the rule-file format or the API, or adds a language or a coding agent:

- write a short design in the issue first: the problem, the approach, the alternatives, and what it changes for
  existing rules, packs and installations (compatibility and upgrade);
- once it is agreed, split it into pull requests that each build and pass on their own;
- keep refactoring and behavior changes in separate pull requests.

## Pack test data

A pack that ports another tool's rules is judged on that tool's test cases and on labelled real-code samples, and none
of that data is in git. Each tool-pack (`packs/<pack>/upstream/<tool>/`) has a test data archive in S3, named by
sha256 in the tracked `test-data.lock`, and a samples archive when it has a `samples.lock`. Without the data, the pack
tests (`npm test`, `npm run test:conformance`, `cargo test`) and the generators fail, saying to pull:

```sh
npm run fixtures:pull   # every tool-pack; -- --pack <id> [--tool <tool>] for some; -- --dry-run to see what it would fetch
```

`fixtures:pull` keeps every archive it fetches in `.rerail/pack-data/archives/`, so it needs read access to the
bucket only for an archive it has not fetched before; ask a maintainer for access. It verifies each archive against
its lock before it extracts anything, and lays any staged `publish/` (below) over what it extracted. Pulled data is
ignored by git. Pull again after a checkout or merge that changes a lock.

What the archives hold, how data is published and how to replay a CI run are in [docs/upstream.md](docs/upstream.md);
the files and the lock format are in [docs/pack-format.md, "Pack data"](docs/pack-format.md#pack-data).

### Adding or changing a pack

Each change is one pull request, with a changeset for the pack unless it is private ([.changeset/README.md](.changeset/README.md)).
The steps for adding a rule, written for Rerail or ported from a tool, with what each has needed in practice, are
[docs/adding-rules-to-a-pack.md](docs/adding-rules-to-a-pack.md).

**A rule.** A pack ships the rules in `packs/<id>/rules/` that `pack.json` lists; `candidates/` holds ports under
evaluation, which are scored and reported but never shipped. Start a new port as a candidate
([docs/language-packs.md, "Start a pack"](docs/language-packs.md#start-a-pack)). For a false positive or a false
negative, add the code that triggers it with the fix, as a sample or an authored case (below). Score the pack with
`npm run conformance:languages -- --pack <id>` (`--samples` adds the samples) and run the gate with
`npm run test:conformance`. Declare each intended disagreement with the tool in the pack's `divergences.json`, with the
evidence you checked ([conformance/README.md, "Divergences"](conformance/README.md#divergences)).

On a pull request from a branch here that changes what the check reads (any change under `packs/` does), CI keeps a
**Conformance** comment. A shipped rule that fails a blocking check fails CI, and everything else in the comment is
reported only ([conformance/README.md, "The pull request comment"](conformance/README.md#the-pull-request-comment)). A
pull request from a fork runs no CI: run `npm run test:conformance` yourself and say so in the pull request. "Ready to
promote" is necessary, not sufficient, and the reviewer decides; the steps are in
[conformance/README.md, "Candidates and promotion"](conformance/README.md#candidates-and-promotion).

**Test data.** You need no S3 credentials to change pack data. Pull, then write the cases in the pulled layout: a code
corpus with its ground truth under `upstream/<tool>/authored/`, or a case file under the tool directory with its row
in the tool's `UPSTREAM.md`. Then derive, stage what changed, and commit `publish/`:

```sh
npm run fixtures:derive -- --pack <id> --tool <tool>   # the case files' expected findings, and provenance.json
npm run fixtures:stage -- --pack <id> --tool <tool>    # every file the locked archive lacks or holds differently
git add packs/<id>/upstream/<tool>/publish
```

Every file must be under a licence the allowlist takes ([Third-party material](#third-party-material)). CI judges the
staged data, and the pull request cannot merge until a maintainer publishes it. Changing samples, removing a file,
derived data and the publish itself are in
[docs/upstream.md, "Publishing pack data"](docs/upstream.md#publishing-pack-data).

## Licence of contributions

Rerail is licensed under the Apache License 2.0. By submitting a contribution you agree that it is licensed under
the same terms, as described in section 5 of the licence. There is no contributor licence agreement and no
sign-off to add.

## Third-party material

Do not paste code, data or rules from another project without recording it. Only permissively licensed material is
accepted, anywhere in the repository: every licence must be on the allowlist in
[scripts/check/licence-allowlist.json](scripts/check/licence-allowlist.json) (Apache-2.0, MIT, BSD-2-Clause,
BSD-3-Clause, CC0-1.0, ISC, Zlib, 0BSD, Unicode-3.0, PostgreSQL, PSF-2.0), so no copyleft file can be committed or published.
An SPDX licence expression passes when some way to satisfy it uses only listed licences: `A OR B` when either side is
on the list, `A AND B` only when both are, with `AND` binding tighter than `OR`; a malformed one is refused.

Anywhere outside a pack:

1. Add an `UPSTREAM.md` beside the material, per upstream repository: the source project, its version or revision
   and commit, the paths taken, and whether each is verbatim, derived or an excerpt.
2. Add the upstream licence text, to `LICENSES/` or beside the material.
3. Annotate the path in `REUSE.toml`. `reuse lint` must pass.

In a pack (`packs/<id>/`), the record is per upstream repository and covers the pack test data that is not in git:

- `licenses/<project>/` holds the repository's licence text, verbatim, and its `UPSTREAM.md`: a `License:` line with
  the SPDX expression, a `Source` table (repository, tag, commit and licence), a `Taken from it` table mapping each
  pack-path glob of copied files to its upstream path and how it was copied, and a `Not copied` line for each part of
  the repository that must not be copied, with the pack-path glob it would land under.
- The tool's own `upstream/<tool>/UPSTREAM.md` lists, one row per file, the files in the tool directory that Rerail
  wrote. They are Apache-2.0, like everything else Rerail writes.
- Each test data archive holds `provenance.json` (every file, its upstream path and how it was made) and `LICENSES.md`
  (the licence texts), for audit. No licence check reads them; publish prep reads the file list in `provenance.json`
  and refuses an archive it does not describe.
- `REUSE.toml` annotates the rule documents with the pack's licence expression, and the pulled and staged copies of
  the copied files with the upstream licence.

Pack check 6 (`npm test`) and the publish decide each file's licence the same way
([docs/pack-format.md, "Licence of each file"](docs/pack-format.md#licence-of-each-file)); the records' formats are in
[docs/pack-format.md, "UPSTREAM.md"](docs/pack-format.md#upstreammd) and
[docs/pack-format.md, "provenance.json and LICENSES.md"](docs/pack-format.md#provenancejson-and-licensesmd).

Real-code samples are whole files of other repositories. Each sample's labels declare its repository's licence, and
publish takes it only when `scripts/samples/licences.ts` records that licence for the repository at the sample's
commit and the licence files it records there read as that licence: a repository with no entry, a declared licence
that is not its entry, a commit with no licence, and a copyleft repository are refused. A samples archive's
`LICENSES.md` lists every file with its repository's licence, and holds a licence text only where one was read at the
samples' commits (a set converted from a test set with only the licence it recorded, as the ESLint test set's were,
carries that licence and no text); `SAMPLES.md` says which
([docs/upstream.md, "Sample licences"](docs/upstream.md#sample-licences)).

The checks confirm that licences are declared, permitted and cover every file; they cannot confirm that an
`UPSTREAM.md` or a `licences.ts` entry is true. That is the reviewer's job ([REVIEW.md](REVIEW.md)). Fixtures
(`fixtures/`) follow the same rules; see [fixtures/README.md](fixtures/README.md).

## Dependencies

New dependencies must be permissively licensed if they end up in anything Rerail distributes:
`npm run check:licenses` fails on a copyleft production dependency. Copyleft is acceptable only in
development-only dependencies and tools, which are never committed or distributed.
