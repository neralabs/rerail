# Environment matrix

The environment matrix installs a Rerail package and checks it as a user would meet it: Linux and Windows on
throwaway Azure VMs, macOS on GitHub-hosted runners.

CI (`.github/workflows/env-matrix.yml`) runs it after a push to `main` that changes the matrix, or what core ships per
`scripts/release/core-paths.mjs` (the Release pull request's rule). Pack, test, docs or other CI changes leave the
package as the last run tested it. `just rerail-env-matrix` runs the Azure environments by hand.

## Environments

`scripts/host/azure/environments.json` lists the environments and what each is expected to do.

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

<details>
<summary>Glibc support and expected failures</summary>

Linux engines target glibc 2.17, below the 2.28 Node 24 itself needs, so any glibc distribution running Node 24
should pass.

A known set-up failure is marked `"expect": "setup-fails"`, with the printed text (`failsWith`) and reason
(`because`). It passes while set-up fails that way. Once set-up succeeds it fails: support has changed, so switch the
entry to `"expect": "pass"`.

</details>

### macOS on GitHub-hosted runners

Azure has no Macs, so macOS environments run the Linux guest, `guest.sh`, on a GitHub-hosted runner via
`matrix.mjs local`.

<details>
<summary>How the runner becomes a clean Mac</summary>

A macOS environment has `"os": "macos"` plus a `"runner"` label to run on; the guest runs on it with sudo.

A runner is not a clean Mac: it carries Xcode, Homebrew and other Nodes. The guest makes zsh the account's shell and
installs Node from nodejs.org's installer into `/usr/local/bin`, as a developer would. The facts record which Node the
user's shell runs.

`local` refuses to run anywhere but a GitHub-hosted runner, which is discarded after the job. Without `--env`, the
runner it is on is the environment, expected to pass. The release workflow installs every release this way on
`ubuntu-24.04` (Linux x64) and `macos-15` (macOS arm64).

</details>

### Node and Git

Each environment installs Node and Git the way a user would.

- **Node**: newest release of the `environments.json` major (`"node"`, overridable per environment), from nodejs.org.
- **Git**: the distribution's; the Command Line Tools on a Mac; Git for Windows, default folder.

## What each environment checks

Eight checks run as the ordinary user, never root or SYSTEM, so set-up writes PATH as on a person's computer.

1. **registry**: a registry on the VM serves the package and every published rule pack in `packs/`; all else comes
   from npmjs.
2. **project**: a Git repository with a `package.json` and one commit, plus empty agent folders.
3. **setup**: `npx @neralabs/rerail setup --json`, answered until Rerail is on.
4. **status**: `rerail status` shows the daemon running with `Engine: ok`.
5. **PATH**: a new shell finds `rerail`.
6. **hooks**: Claude Code's and Codex's SessionStart commands record their sessions with the daemon.
7. **daemon-restart**: `rerail daemon stop` and `rerail daemon start` work.
8. **uninstall**: `rerail install --remove` unhooks every agent and takes Rerail off PATH.

<details>
<summary>Exact pass conditions</summary>

- **registry**: the VM's registry serves them as npmjs will.
- **project**: the `package.json` is React and TypeScript; empty `~/.claude`, `~/.codex` and `~/.cursor` make set-up
  connect those agents.
- **setup**: runs in the project, then sends each answer through the launcher command it prints (allow sessions, read
  this repository, start with the packs shown) until set-up says Rerail is on.
- **status**: `~/.rerail/current` is a symlink (junction on Windows).
- **hooks**: Claude Code's and Codex's SessionStart commands run as each agent runs them. Cursor's hooks are registered.
- **daemon-restart**: `rerail daemon stop` returns within 60 seconds, `status` shows no daemon, and `rerail daemon
  start` restores it with the engine.
- **uninstall**: no Claude Code, Codex or Cursor config points into the Rerail home, and Rerail's directory is off PATH.

</details>

<details>
<summary>Sessions, shells and recorded facts</summary>

Set-up writes PATH into the user's own startup files or registry hive. Linux and macOS use a login shell; Windows a
signed-in desktop session (automatic sign-in, then a restart).

The PATH check uses login and interactive bash on Linux; login interactive zsh (as Terminal.app opens) and
interactive zsh on macOS. On Windows, PATH is built from machine and user registry values, as a new terminal builds it.

The release workflow runs the same checks before every publish: `guest.sh` on Linux and macOS, and on Windows the
registry, set-up, managed install, hook and uninstall steps of `scripts/host/windows-host.ps1` via
`scripts/host/windows-install-smoke.ps1`.

Facts are recorded, not judged: OS, glibc or Windows build, Node and its location, npm, Git; on Windows, also the
execution policy and what `npx` does in a new Windows PowerShell.

</details>

## Results

Each run writes `<id>.json` and `<id>.log` per environment, plus a `summary.md` table.

- **`<id>.json`**: facts and steps, each with its output's tail.
- **`<id>.log`**: the full provisioning and check log.

CI adds the table to the job summary (one per macOS job) and uploads the files as the `env-matrix-<id>` artifact. By
hand, they land in `.rerail/env-matrix/<run id>/`.

## Running it by hand

Run the Azure environments locally with `just rerail-env-matrix`, after `az login` to the subscription that holds the
resource group:

```sh
just rerail-env-matrix run --tarball neralabs-rerail-<version>.tgz                  # every environment
just rerail-env-matrix run --tarball <file> --env windows-11-x64,debian-12-x64      # some
just rerail-env-matrix run --npm next                                               # a published version from npmjs
just rerail-env-matrix run --tarball <file> --env ubuntu-24.04-x64 --keep           # leave the VM for a look
just rerail-env-matrix sweep --run-id <id>                                          # delete what a --keep run left
```

- **Tarball**: needs every native engine, so use a `rerail-package` artifact from a Release or Environment matrix run
  (`gh run download <run> -n rerail-package`), not a checkout's `npm pack`.
- **`--npm`**: until launch, npmjs serves `@neralabs` only to signed-in members; set `RERAIL_ENV_NPM_TOKEN` to a read
  token.
- **Duration**: environments run in parallel; a full run takes about 20 minutes, Windows longest.

### Network and clean-up

VMs accept no inbound connections and are deleted when done.

<details>
<summary>How VMs are reached and removed</summary>

Scripts reach a VM through Azure Run Command; the package and results move through blob URLs signed for three hours.
Each VM's public IP carries only outbound traffic.

Each environment deletes its VM, disk, network interface and public IP when it finishes. `sweep`, run by CI after every
run, deletes what a cancelled run left and any run container unwritten for three hours.

</details>

### Staging in CI

CI stages the package once per run; each environment's job runs `run --staged` against it.

<details>
<summary>Re-runs and expiry</summary>

`stage` bundles the package and rule packs into the run's container, and re-runs of failed environments reuse it.
Every attempt writes its results there, so the bundle lasts three hours past the last attempt.

Later re-runs fail with `Nothing is staged in run-gh<id>`: re-run all jobs instead, which stages the package again.

</details>

### Windows passwords

Each Windows VM's administrator password is generated per run and readable in clear while the VM exists.

<details>
<summary>Where it is readable and why that is acceptable</summary>

- the Run Command resource (`az vm run-command show`, for anyone with Reader on the resource group);
- `C:\rerail-env\guest.ps1`;
- the Winlogon `DefaultPassword` value that signs the user in.

With no inbound access and the VM deleted after its run, the password opens nothing. Still, limit resource group
access to the people who run the matrix.

</details>

## Azure setup

`scripts/host/azure/bootstrap.sh` creates the Azure resources once, idempotently.

<details>
<summary>Flags, resources and licensing</summary>

- **`--github`** writes the workflow's repository variables: `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`,
  `AZURE_SUBSCRIPTION_ID`, `RERAIL_AZURE_RESOURCE_GROUP`.
- **`--branch <name>`** trusts one more branch, to test a workflow change before merge; **`--remove-branch <name>`**
  undoes it.

The script creates a resource group (`rerail-envtest-rg` in Canada Central by default) with a storage account whose
blobs expire after seven days and a virtual network whose security group admits nothing from outside.

For CI, it creates the app registration `rerail-env-matrix-github`, trusted through OIDC for pushes to this
repository's `main`, with Contributor on that resource group only.

Windows 11 images run under `--license-type Windows_Client`, which Azure allows when Microsoft 365 or Windows
subscriptions license Windows 11 for the organisation's users.

</details>
