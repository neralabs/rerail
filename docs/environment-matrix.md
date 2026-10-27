# Environment matrix

The environment matrix installs a Rerail package in each of a set of environments, throwaway Azure VMs for Linux
and Windows and GitHub-hosted runners for macOS, and checks it there the way a user meets it. CI runs it
(`.github/workflows/env-matrix.yml`) after a push to `main` that changes what core ships, by the rule the Release pull
request uses (`scripts/release/core-paths.mjs`), or that changes the matrix itself; a pack, test, docs or other CI
change leaves the package as the last run tested it. `just rerail-env-matrix` runs the Azure environments by hand.

## Environments

`scripts/host/azure/environments.json` lists them:

| Environment | Image | Expected |
| -- | -- | -- |
| `ubuntu-24.04-x64`, `ubuntu-24.04-arm64` | Ubuntu Server 24.04 LTS | pass |
| `debian-13-x64`, `debian-13-arm64` | Debian 13 | pass |
| `ubuntu-22.04-x64` | Ubuntu Server 22.04 LTS | pass |
| `debian-12-x64` | Debian 12 | pass |
| `rhel-9-x64` | Red Hat Enterprise Linux 9 | pass |
| `windows-11-x64`, `windows-11-arm64` | Windows 11 Pro 25H2 | pass |
| `macos-26-arm64`, `macos-15-arm64` | GitHub-hosted runner `macos-26`, `macos-15` (Apple silicon) | pass |
| `macos-15-x64` | GitHub-hosted runner `macos-15-intel` | pass |

The Linux engines are built against glibc 2.17, older than the glibc 2.28 Node 24 itself needs, so every glibc
distribution Node 24 runs on is expected to pass. An environment where set-up is known to fail is marked
`"expect": "setup-fails"`, with the text the failure prints (`failsWith`) and the reason (`because`): it passes when
set-up fails that way and fails when set-up starts succeeding, and then support has changed and the entry moves to
`"expect": "pass"`.

Azure has no Macs, so the macOS environments (`"os": "macos"`, with the `"runner"` label to run on) run on
GitHub-hosted runners: `matrix.mjs local` runs the Linux guest, `guest.sh`, on the runner itself with sudo. A runner
is not a clean Mac (it carries Xcode, Homebrew and other Nodes), so the guest makes it one the way a developer's is
set up: the account's shell becomes zsh, and Node comes from nodejs.org's installer into `/usr/local/bin`; the
facts record which Node the user's shell runs. `local` refuses to run anywhere but a GitHub-hosted runner, which is
thrown away after the job. Without `--env` it takes the runner it is on as the environment, expected to pass: that is
how the release workflow installs every release on Linux x64 (`ubuntu-24.04`) and macOS arm64 (`macos-15`).

Each environment gets the newest Node release of the major in `environments.json` (`"node"`, overridable per
environment) from nodejs.org, and Git from the distribution, the Command Line Tools on a Mac, or, on Windows, Git
for Windows in its default folder: what a user brings, installed the way they would install it.

## What each environment checks

The checks run as the machine's ordinary user, never as root or SYSTEM: on Linux and macOS through a login shell, on
Windows in a desktop session the user is signed into (automatic sign-in, then a restart), so set-up writes PATH into
the user's own startup files or registry hive as it would on a person's computer. The release workflow runs the same
checks before every publish: `guest.sh` on Linux and macOS (above), and on Windows the registry, set-up, managed
install, hook and uninstall steps of `scripts/host/windows-host.ps1` through `scripts/host/windows-install-smoke.ps1`.

1. **registry**: the package under test and every rule pack in `packs/` are served from a registry on the VM, as
   npmjs will serve them; everything else comes from npmjs.
2. **project**: a Git repository with a `package.json` (React, TypeScript) and one commit, and empty `~/.claude`,
   `~/.codex` and `~/.cursor` folders so set-up connects those agents.
3. **setup**: `npx @neralabs/rerail setup --json` in the project, then each answer through the launcher command set-up
   prints for it (allow sessions, read this repository, start with the packs shown) until set-up says Rerail is on.
4. **status**: `rerail status` reports the daemon running with `Engine: ok`, and `~/.rerail/current` is a symlink (a
   junction on Windows).
5. **PATH**: a new shell finds `rerail`: a login and an interactive bash on Linux; on macOS a login interactive zsh (as
   Terminal.app opens) and an interactive one; on Windows, PATH built from the machine and user values in the
   registry, as a new terminal builds it.
6. **hooks**: the SessionStart command set-up registered for Claude Code and for Codex, run the way each agent runs
   it, records its session with the daemon; Cursor's hooks are registered.
7. **daemon-restart**: `rerail daemon stop` returns within 60 seconds, `status` then reports no daemon, and `rerail
   daemon start` brings it back with the engine.
8. **uninstall**: `rerail install --remove` leaves no Claude Code, Codex or Cursor config pointing into the Rerail
   home, and takes Rerail's directory off PATH.

Facts about each environment (OS, glibc or Windows build, Node and where it lives, npm, Git, and on Windows the
execution policy and what `npx` does in a new Windows PowerShell) are recorded alongside, not judged.

## Results

Each run writes, per environment, `<id>.json` (facts and steps, each with its output's tail) and `<id>.log` (the
whole provisioning and check log), and a `summary.md` table. CI adds the table to the job summary and uploads the
files as the `env-matrix-<id>` artifact; by hand they land in `.rerail/env-matrix/<run id>/`. Each macOS job writes
its own table.

## Running it by hand

Sign in with `az login` to the subscription that holds the resource group, then:

```sh
just rerail-env-matrix run --tarball neralabs-rerail-<version>.tgz                  # every environment
just rerail-env-matrix run --tarball <file> --env windows-11-x64,debian-12-x64      # some
just rerail-env-matrix run --npm next                                               # a published version from npmjs
just rerail-env-matrix run --tarball <file> --env ubuntu-24.04-x64 --keep          # leave the VM for a look
just rerail-env-matrix sweep --run-id <id>                                          # delete what a --keep run left
```

A tarball needs every native engine, so take it from a `rerail-package` artifact (`gh run download <run> -n
rerail-package`, from a Release or Environment matrix run), not from a checkout's `npm pack`. `--npm` installs from
npmjs, which serves `@neralabs` packages only to signed-in members until launch: set `RERAIL_ENV_NPM_TOKEN` to a read
token meanwhile. Environments run in parallel; a run of all of them takes about 20 minutes, the Windows ones longest.

No VM accepts inbound connections. Scripts reach a VM through Azure Run Command, the package and the results move
through blob URLs signed for three hours, and each VM's public IP carries only its outbound traffic. Each environment
deletes its VM, disk, network interface and public IP when it finishes; `sweep` deletes what a cancelled run left and
each run's storage container once nothing has been written to it for three hours (CI sweeps after every run).

In CI the package is staged once per run (`stage` bundles it with the rule packs into the run's container) and each
environment's job runs `run --staged` against it; re-running failed environments reuses that bundle. Every attempt
writes its results into the container, so the bundle lasts three hours past the run's last attempt. A re-run started
later than that fails with `Nothing is staged in run-gh<id>`: re-run all jobs, which stages the package again.

A Windows VM's administrator password is generated for the run and is readable in clear while the VM exists: in the
Run Command resource (`az vm run-command show`, by anyone with Reader on the resource group), in
`C:\rerail-env\guest.ps1`, and in the Winlogon `DefaultPassword` value that signs the user in. The VM admits no inbound
connections and is deleted when its environment finishes, so the password opens nothing reachable and outlives
nothing; keep the resource group's access to the people who run the matrix.

## Azure setup

`scripts/host/azure/bootstrap.sh` sets up, once and idempotently, a resource group (`rerail-envtest-rg` in Canada
Central by default) holding a storage account whose blobs expire after seven days and a virtual network whose
security group admits nothing from outside. For CI it creates the app registration `rerail-env-matrix-github`,
trusted through OIDC for pushes to `main` of this repository, with Contributor on that resource group only, and
`--github` writes the repository variables the workflow reads (`AZURE_CLIENT_ID`, `AZURE_TENANT_ID`,
`AZURE_SUBSCRIPTION_ID`, `RERAIL_AZURE_RESOURCE_GROUP`). `--branch <name>` also trusts one more branch, to try a
change to the workflow before it merges; `--remove-branch <name>` takes that back.

The Windows 11 images run under `--license-type Windows_Client`, which Azure allows for organisations whose Microsoft
365 or Windows subscriptions license Windows 11 for their users.
