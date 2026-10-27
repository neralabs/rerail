# Contributing to Rerail

Thank you for helping. This guide covers trusted contributors, how work starts, how to set up a checkout, what a pull
request needs, and the rules that keep the project open source and its licensing clean.

Everyone taking part follows the [Code of Conduct](CODE_OF_CONDUCT.md). Report a security vulnerability privately, as
[SECURITY.md](SECURITY.md) describes, never in a public issue or pull request.

## Trusted contributors

Anyone can open an issue. Pull requests come from trusted contributors, each from their own fork. That keeps the
project's CI, its secrets and its release workflows away from code nobody on the team has read, without closing the
project to outside work.

### What trust gives you

| | Trusted contributor | Anyone else |
| --- | --- | --- |
| Open and comment on issues | Yes | Yes |
| Claim a `good first issue` or `help wanted` issue | Yes | Ask to become trusted ([below](#becoming-a-trusted-contributor)) |
| Open a pull request from a fork | Yes | Closed with a link to this section |
| Write access, merging, triage, the project's secrets | No | No |

CI on a pull request from a fork runs once a maintainer has approved the run, without the project's secrets; the jobs
that need them skip the pull request and run when the change reaches `main`. A trusted contributor's pull request is
reviewed like anyone else's.

### Becoming a trusted contributor

Ask in the issue you want to work on, or open one: say what you want to change and how you would go about it. For
anything in [Start with an issue](#start-with-an-issue), that is the conversation a maintainer has with you anyway.

Maintainers look at:

- the issue itself: a reproduction, a failing case, or an approach that holds up;
- whether the change fits where Rerail is going, at a size you can finish;
- how you have worked in the issues so far, under the [Code of Conduct](CODE_OF_CONDUCT.md).

A maintainer answers in the issue. If it's a yes, they add you to the list below and assign you the issue. If it's not
yet, they say why and what would change it.

### The list

The list changes only through a maintainer's pull request that links the issue where it was agreed, so every name on
it has a public reason.

Trusted contributors: none yet.

### Leaving the list

- Ask, in any issue or at [opensource@nera.software](mailto:opensource@nera.software), and you are taken off.
- Twelve months without a pull request, issue or comment here takes you off. Ask again any time.
- Breaking the Code of Conduct can take you off, as its enforcement steps describe.

### Pull requests from anyone else

A maintainer closes them with a link to this section. That says nothing about the change itself: ask in an issue,
linking the closed pull request, and once you are trusted, open it again from your fork.

## Start with an issue

Open an issue before you start on:

- a new feature, or a change to how Rerail behaves;
- a new rule, rule pack, language or coding-agent integration;
- a change to the rule-file format, the API, or what a pack publishes;
- anything that spans several parts of the repository.

Agree on the approach with a maintainer in the issue before you write the code. A pull request for work nobody agreed
on may be closed, however good it is.

A trusted contributor can take these straight to a pull request, with no issue:

- typo and documentation fixes;
- a bug fix with a test that fails without it;
- a rule's false positive or false negative, with the code that triggers it added as a test.

### Claiming an issue

Issues labelled `good first issue` or `help wanted` are open to any trusted contributor, and asking for one is a good
way to become one ([Becoming a trusted contributor](#becoming-a-trusted-contributor)). Check that the issue is not
assigned, then comment on it to claim it; a maintainer assigns it to you. If there is no pull request or update from
you within two weeks, the issue is open to others again.

## Contribution flow

1. Find an issue, or open one ([Start with an issue](#start-with-an-issue)).
2. Claim it ([Becoming a trusted contributor](#becoming-a-trusted-contributor) the first time), and agree on the
   approach with a maintainer when the issue needs one.
3. Fork the repository and create a branch from `main`.
4. Set up your checkout ([Development setup](#development-setup)), make the change, and add tests for the behavior it
   changes.
5. Run the checks in [Before you open a pull request](#before-you-open-a-pull-request).
6. Open a pull request against `main` that links the issue (`Fixes #123`) and says how you tested the change.
7. A maintainer reviews it. Push fixes to the same branch; once it is approved and CI passes, it is squash-merged.

## Getting help

Ask in the issue you are working on, or open an issue with your question.

### Response times

Rerail is maintained by a small team. These are the times we aim to reply in, counted in business days; they are
not times to a fix, a merge or a release.

| You sent | We reply within |
| --- | --- |
| A vulnerability report ([SECURITY.md](SECURITY.md)) | 2 business days |
| A conduct report ([CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md#reporting)) | 3 business days |
| A bug report | 5 business days: triaged, labelled, or asked for a reproduction |
| A feature request | 10 business days: accepted, declined, or "not now" with a reason |
| A request to become a trusted contributor | 10 business days |
| A pull request, as a trusted contributor | 10 business days to its first review |
| A question | Best effort |

If an issue or pull request gets no reply within its window, comment on it to bring it back up.

## Development setup

One repository holds everything: the core package at the root, rule packs under [`packs/`](packs/), and the engine's
language conformance fixtures under [`fixtures/`](fixtures/) (test data, never published).

Install [Git LFS](https://git-lfs.com) before you clone, and run `git lfs install` once. The embedding model's weights
(`model/model.safetensors`) and the engine's Unicode names table (`native/src/code_rule_dsl/value/names/names.bin`,
compiled into the engine) are stored in LFS (`.gitattributes`); without it a checkout holds pointer files, and neither
the engine build nor the model works. In a checkout made without it, run `git lfs pull`.

```sh
git clone https://github.com/neralabs/rerail.git
```

`RERAIL_ORACLES_DIR` points the build and tests at another fixtures directory.

Prerequisites: Node.js 24+, npm, Rust through [rustup](https://rustup.rs/), and [`just`](https://github.com/casey/just)
for the dev recipes (`just rerail-dev`, `just nuke-rerail`). `rust-toolchain.toml` pins the toolchain, so `cargo`
installs and selects it on first use.

```sh
npm ci
npm run build:native      # compile the native kernel into native/
npm run build             # clean build: native kernel, runtime, host bundles, UI
npm run check:all         # TypeScript, rustfmt + clippy, UI token and reference checks
npm run test:all          # build, unit, conformance, reference, native, and integration suites
npm run test:conformance  # the language conformance gate, offline
npm run package:check     # build and dry-run the npm pack
```

The pack tests also need the pack test data: run `npm run fixtures:pull` ([Pack test data](#pack-test-data)).

`npm run build:native` needs `cargo` on `PATH`; after a fresh rustup install, `export PATH="$HOME/.cargo/bin:$PATH"`.

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
store is `<checkout>/.rerail/content-v1.sqlite`, private to that checkout or worktree. Only the managed installation
under `~/.rerail/versions/` opens the machine-wide `~/.cache/rerail/content-v1.sqlite`, so a checkout on a newer store
schema cannot break the installed Rerail. `RERAIL_CONTENT_STORE_PATH` overrides the location; the test setup gives
every test file its own store that way. See [Installing Rerail](docs/installation.md#after-setup).

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
the production workspace against the tool's recordings in the pack's `upstream/` and, with `--samples`, on the
tool-packs' labelled real-code samples (`samples.lock`, [docs/port-readiness.md](docs/port-readiness.md#samples));
`--write` saves the report under `.rerail/reports/`. `npm run conformance:languages:check` is the gate over the fixtures
and the samples; it runs offline in `npm run test:conformance`, part of `test:all` in CI.

### Operational commands (`just`)

The root [`Justfile`](Justfile) wraps the day-to-day operations; `just` lists them. Each recipe runs a script under
`scripts/` (or `playground/`, `conformance/`, `benchmark/`), so the same command works without `just`.

| Recipe | Does |
| --- | --- |
| `just rerail [--tag next] [setup args]` | Installs the published package on this machine (`npx @neralabs/rerail setup`). Until launch the package is restricted: `npm login` with an account that has `@neralabs` access first. `RERAIL_TAG` selects the dist-tag; `--restore [dir]` then restores a `nuke-rerail` backup. |
| `just nuke-rerail [--dry-run \| --yes]` | Removes Rerail from this machine (daemon, coding-agent registrations, `~/.rerail`, every checkout's dev daemon), moving it all to `~/.rerail-clean-YYYYMMDD[-HHMMSS]/` for `just rerail --restore`. |
| `just rerail-native` | Installs rustup and the pinned toolchain if needed, then builds the native engine. |
| `just rerail-setup` | Reserves this checkout's dev instance (`.rerail/dev/instance.json`: a stable loopback port in 35000–35999 and the home `.rerail/dev/home`) without starting anything. |
| `just rerail-dev [--install-bridge] [--restore [dir]]` | Rebuilds whatever is stale and (re)starts this checkout's dev daemon on its own home and port, serving `ui/` live. `--install-bridge` registers this checkout's build with every coding agent (`rerail install --root`); hooks always use the default home, so a bridged checkout's dev daemon runs on `~/.rerail` until another `rerail setup` replaces the bridge. `--restore` loads a `nuke-rerail` backup's database. |
| `just rerail-status`, `just rerail-down`, `just rerail-reset` | Shows the dev instance and build freshness; stops its daemon (state kept); deletes this repository's session history from the dev database (rules kept). |
| `just rerail-playground` | The development rule playground beside the dev daemon (attended macOS only; see [playground/README.md](playground/README.md)). |
| `just rerail-smoke` | Packs this checkout, publishes it to a throwaway authenticated local registry, and drives install, update, `nuke-rerail`/`--restore` and `install --remove` in a sandbox `HOME` (`KEEP=1` keeps it). |
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

The command builds the complete package first. Each harness gets a temporary workspace, private profile, resident
daemon, and a probed `sandbox-exec` write boundary. The smoke's conversation rule and code rule are stored in the daemon
database's rule library and added to the workspace's repository, which is set up and started as `rerail start` leaves
it. The smoke uses actual installed CLIs and providers—not scripted models or hook substitutes. It requires a committed
violating edit with a matching native tool result, an executable-policy diagnostic, restoration of the transaction
helper, exactly one cosine delivery containing a private nonce, and an assistant reply quoting that nonce and
diagnostic.

`RERAIL_SMOKE_ACCEPTANCE=transport` narrows a run to the hook transport contract; the default is `full`. Transport mode
remains a real CLI/provider session and still gates sandboxing, one lifecycle session, the completed violating tool
result, in-budget evaluation, the single cosine delivery, and the executable-policy delivery. It deliberately excludes
the model's subsequent file restoration and final prose because those are provider sampling behavior rather than the
hook transport contract. Full mode retains every assertion above.

Prerequisites:

- macOS with `sandbox-exec`, and the selected harness CLIs installed. `RERAIL_CLAUDE_BIN` and `RERAIL_CURSOR_AGENT_BIN`
  can select non-PATH executables.
- Provider credentials for each selected harness. `scripts/host/host-smoke.ts` documents the environment variables it
  reads; a missing credential or endpoint is a failure, never a skipped pass. OAuth tokens copied from a harness login
  must remain valid for at least ten minutes; refresh credentials are not copied.

Each provider-backed session has a 180-second limit by default; `RERAIL_SMOKE_TIMEOUT_MS` can raise that hard limit for a slower provider. `RERAIL_SMOKE_ATTEMPTS` (default 1; CI uses 3) reruns a harness in a fresh sandbox while its attempt is inconclusive: the session timed out or the violation never reached `api/route.py`, and nothing Rerail is accountable for failed. A violation that reached the file without Rerail's record of it or its diagnostic, an extension error, a lost or duplicated delivery, or any other harness exit is a failure on the first attempt. Output is bounded and redacted; disposable profiles and workspaces are removed after each run. The command emits one JSON evidence record per harness attempt (`attempt`, `status` `PASSED`, `FAILED` or `INCONCLUSIVE`) and exits nonzero for any missing prerequisite or failed assertion. The existing user daemon and harness profiles are not replaced.

The Codex session makes its violation through the shell rather than an edit tool, with a workspace helper
(`python3 tools/set_call.py cursor.execute`, later `transaction.fetch_value`) so the command has no quoting for the
model to get wrong. For every harness, whether the violation reached `api/route.py` is judged by reading that file
every 100 ms while the session runs, not from Rerail's records, so a violation Rerail failed to record still counts as
landed. Against Azure, the smoke lowers Codex's stream idle timeout to 60 seconds so a stalled response is retried
inside the attempt.

Claude's internal temporary directory is explicitly kept inside the sandbox with `CLAUDE_CODE_TMPDIR`; API retries are
disabled so authentication failures surface promptly instead of consuming the smoke timeout.

### UI

The browser view lives in `ui/` and has no toolchain: `index.html`, stylesheets under `ui/css/`, and native ES modules
(`app.js`, `shell.js`, `state.js`, `poll.js`, `frame/`, `views/`, `render/`; [ui/README.md](ui/README.md) maps them),
served as bytes. No React, no Vite, no Tailwind, no TypeScript, not even as dev dependencies. Colours come from the
committed `ui/tokens.generated.css` (and the terminal brand colour from `src/presentation/terminal-brand.generated.ts`);
`npm run check:ui-tokens` fails when the UI references a `var(--…)` that neither the token sheet nor the page's own CSS
declares.

The dashboard is a sidebar navigator. Its routes: `#overview` (the default: the repository's tiles, Most violations
logged and Recent sessions), `#pack/<packId>` (one added rule pack's rules, with a switch for the whole pack),
`#created` (rules created by you), `#packs` (the rule pack catalog), `#packs?review=1` (the packs set-up recommended,
reviewed before Rerail starts), `#create` (how to create a rule with your coding agent), `#history` (History by rule;
`?rule=<id>` opens it on one rule), `#sessions` (History by session, with `?sessionId=<id>` for one session),
`#settings`, and `#rule/<id>` (one rule's sheet over the current view, the link a steer carries). Links written before
v8 still resolve: `#rules` opens Overview, `#activity` History by rule, `#rule-packs` the catalog and
`#rule-packs?review=1` the review. Every route carries the `?repository=<key>&checkout=<path>` scope; without one the
dashboard lists the repositories to pick from.

`npm run build:ui` copies `ui/` to `dist/ui`, which the daemon serves. A daemon started with `--dev`, or with
`RERAIL_DEV=1` in its environment, serves `ui/` itself instead, so an edit needs only a browser reload; `rerail status`
and `/healthz` report it as `dev` rather than `bundled`.

## Before you open a pull request

Open a pull request against `main` that links its issue, if it has one. Changes are squash-merged after review.

- `npm run check` passes: the type check, the layer boundaries, the repository structure gate, Windows-safe paths and
  no internal ticket IDs.
- `npm test` passes, with `npm run build` run first (several suites run the built package from `dist/`).
- `npm run check:licenses` and `reuse lint` pass (`uvx --from 'reuse[charset-normalizer]==6.2.0' reuse lint`, as CI
  runs it).
- A change to a pack under `packs/` adds a changeset ([.changeset/README.md](.changeset/README.md)).
- A change to what the core package ships adds an entry under `## Unreleased` in `CHANGELOG.md`
  ([release notes](README.md#release)).
- Tests cover the behavior you changed, and docs that describe it are updated.

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

A rule pack's test data is not in git. Under each `packs/<id>/upstream/<tool>/` it is the tool's test cases
(`fixtures/`, `tests/`, the case lists `cases.json` and `*.cases.json`, `fixtures.upstream-check.json`, the
complexity pack's `lizard-*.tsv`), the tool's recorded answers on them (`*.expected.json`), and `authored/`. The
patterns are `PACK_FIXTURE_PATTERNS` in [scripts/s3/pack-fixtures.mjs](scripts/s3/pack-fixtures.mjs); everything
else under `upstream/<tool>/` (`gates.yaml`, the generators and recorders, `UPSTREAM.md`, `recording.lock`,
`samples.lock`, `SAMPLES.md`, `test-data.lock`) stays in git. The data is of two kinds:

- **Authored**: what Rerail wrote for the tool to score, which nothing can regenerate: the files the tool's own
  `upstream/<tool>/UPSTREAM.md` lists as Rerail's (pack check 6 reads the same list), and everything under
  `upstream/<tool>/authored/`, the place for a contributor's code corpus and its ground truth when the corpus is no
  public repository a pin can name.
- **Derived**: everything else, upstream's tests copied or derived and the tool's recorded answers on them.
  `npm run fixtures:derive` makes it from the tool-pack's pins, running the pack's generator and then writing the case
  sources the port readiness recorder names (recipes in [scripts/pack-data/recipes/](scripts/pack-data/recipes/)). A
  maintainer runs it, and so does the nightly's bump workflow for an upstream release; pull request CI runs it only to
  check that a changed lock's archive is what the pins derive (below).

Both kinds of one tool-pack (one `packs/<pack>/upstream/<tool>/`) are one archive in S3: the canonical tar of its
files (sorted, with fixed modes, owners and times; [scripts/s3/test-data-archive.mjs](scripts/s3/test-data-archive.mjs)),
compressed with zstd, at `s3://nera-test-fixtures/rerail/archives/<pack>/<tool>/<sha256>.tar.zst` (us-east-2). The
tool directory's tracked `test-data.lock` names it by the sha256 of the tar, with its file count and bytes, so a commit
resolves to exactly one archive. An archive is never overwritten (every upload is a conditional write, and the bucket
is versioned): changed data is a new archive and a new lock.

```sh
aws sso login           # or any AWS credentials with read access to s3://nera-test-fixtures/rerail/archives/
npm run fixtures:pull   # every tool-pack; -- --pack <id> [--tool <tool>] for some; -- --dry-run to see what it would fetch
```

Pull again after a checkout or merge that changes a `test-data.lock`. `fixtures:pull` keeps each archive it fetches in
`.rerail/pack-data/archives/` and reuses a copy there that still matches its lock, so it needs the
[AWS CLI v2](https://aws.amazon.com/cli/) and read access to the bucket only for archives it has not fetched before;
ask a maintainer for access. It verifies every archive against its lock (the tar's sha256, its file count and its
bytes) before it extracts any, and refuses an archive with a path that has a control character, is not NFC, or equals
another's ignoring case. It deletes the untracked derived files an archive lacks, which are stale, and keeps
untracked authored ones, saying so. Files your branch tracks win: a pull puts each tracked file back to its committed
content (or keeps your working-tree edit of it). It never writes through a symbolic link: when `packs/`, a tool
directory or a directory an archive's file goes into is one, it stops before writing anything and says to make it a
directory. `.gitignore` covers the patterns, so pulled data does not show in `git status`. Without the data, the pack
tests (`npm test`, `npm run test:conformance`, `cargo test`) and the recorders fail saying to pull.

A tool-pack with a `samples.lock` also has real-code samples: real files from open-source repositories, labelled per
rule with the lines it should report or as clean for it, which the conformance check and the port readiness gates
judge the rules on ([docs/port-readiness.md](docs/port-readiness.md#samples)). They are one archive in S3 too
(`s3://nera-test-fixtures/rerail/archives/samples/<pack>/<tool>/<sha256>.tar.zst`), which `samples.lock` names in
`test-data.lock`'s format and `SAMPLES.md` describes (sources, licences, commits). `fixtures:pull` fetches and
verifies it with the test data and extracts it under `.rerail/pack-data/samples/`, never into the tool directory
(`node scripts/s3/pack-fixtures.mjs samples` fetches the samples alone). Nothing derives them, and no command edits a
set yet: to change one (samples, labels, or a renamed or dropped rule's key), a maintainer's script loads the samples
tree, changes it and re-packs it with `packSamples` and `writeSampleSet` (`scripts/samples/pack.ts`), which write the
archive into `.rerail/pack-data/archives/samples/` and the lock and `SAMPLES.md`; `node scripts/s3/pack-fixtures.mjs
samples push --pack <id> --tool <tool>` uploads it.

CI pulls the same way, in two kinds of job (`.github/actions/pack-test-data`). A fetching job (`fetch: "true"`) only
warms the Actions cache: it looks up the entry keyed by its tool-packs' locks (`node scripts/s3/pack-fixtures.mjs
cache`) without restoring it and, on a miss, downloads the archives from S3 under the repository's read-only OIDC role
and saves them; it runs nothing but this commit's `scripts/s3/pack-fixtures.mjs`. A consuming job (`fetch: "false"`),
which holds no OIDC token, restores that entry, verifies each archive against its lock and extracts it, and fails on a
miss. Every job that reads pack test data consumes, after a fetching job of the same run: a job that can mint a token
never extracts a cache entry, and a job that runs upstream's code (a port readiness gate leg, the nightly's drift
check, the bump, the reproduction check below) has no token to mint. The fetching jobs run for pushes and
same-repository pull requests only: GitHub gives a fork's or Dependabot's pull request no OIDC token.

Two more checks guard the data. **Pack test data is in S3** runs `node scripts/s3/pack-fixtures.mjs check` under the
read role: a `head-object` for every archive a `test-data.lock` names and every recording a `recording.lock` names,
downloading nothing, failing with each missing key, its lock and the command that pushes it. Main's runs read only S3
and never record, so a pull request cannot merge until S3 holds what its locks name. **Test data reproduces from the
pins** derives each tool-pack whose `test-data.lock` the pull request changes again, with no credentials, and compares
it byte for byte with the data the new archive holds (`npm run fixtures:derive -- --pack <id> --tool <tool> --compare
<copy of the pulled data>`, without `--update`, so the tracked files must come out unchanged too). The required check
is **CI result**, which fails when any CI job failed or was cancelled, since GitHub counts a job skipped after a
failed dependency as passing ([docs/port-readiness.md](docs/port-readiness.md#ci)).

Structure metric #3 (`npm run check`) fails while any pack test data is tracked, a tool directory lacks a
`test-data.lock`, or a lock is malformed, so a pull request that checks data in cannot merge until a maintainer has
archived it.

### Adding or changing a pack

Check the authored test data you add or change into your pull request, so review sees it and CI runs on it: the
cases you wrote for the tool (listed in its `UPSTREAM.md`), or a code corpus with its ground truth under
`upstream/<tool>/authored/`. The generators and recorders print the command that adds the authored files they wrote.
Because `.gitignore` covers them, add them with `git add -f`, e.g. `git add -f --
packs/<id>/upstream/<tool>/fixtures/rerail`. Derived data changes with the pins, a generator or a recipe, which your
pull request changes instead; a maintainer derives it before merge. The usual rules for third-party material apply
(below).

### Merging (maintainers)

Before merging a pull request that tracks pack test data, changes a `recording.lock`, or changes what derives the
data, a maintainer archives each tool-pack it changes and puts its recordings in S3. `fixtures:push` does that with
the maintainer's own AWS credentials, which can write to the bucket, so it runs in a trusted worktree of main and
takes the pull request as a git ref: it reads nothing from that commit but its tracked pack test data and lock files
(with git plumbing), never checks it out and never runs its code. The pull request's scripts, `package.json` and
generators never run next to the credentials.

```sh
# In a trusted worktree of main, with AWS credentials that can write to the bucket:
git fetch origin pull/<n>/head:pr-<n>   # or the pull request's branch
gh run download <run-id> -n recordings-<pack>--<tool> -D .rerail/port-readiness/recordings   # when it changes a recording.lock
npm run fixtures:push -- --from pr-<n> [--pack <id> [--tool <tool>]] [--replace] [--into <checkout of the PR branch>]
```

For each tool-pack, by default every one the commit tracks pack test data for, or whose `test-data.lock`,
`recording.lock` or `UPSTREAM.md` differs from its merge base with `HEAD`, `fixtures:push --from`:

- starts from the archive the commit's `test-data.lock` names, fetched and verified as `fixtures:pull` does but never
  into the worktree; drops from it every file the merge base's `UPSTREAM.md` for the tool listed as Rerail's and the
  commit's no longer lists; and lays the commit's tracked pack test data for the tool directory over it, so a file the
  commit tracks is kept. Without a merge base it refuses and says to use `--replace`. With `--replace` there is no base
  archive: the archive is exactly the commit's tracked pack test data of the tool directory, and a selected tool-pack
  that tracks none fails;
- archives the result, lists the files it dropped from the lock's archive, uploads it unless S3 holds it, and reads it
  back and verifies it against the new lock;
- puts the recording archive the commit's `recording.lock` names in S3, as
  `s3://nera-test-fixtures/rerail/recordings/<pack>/<tool>/<sha256>.tar.zst` from
  `.rerail/port-readiness/recordings/<sha256>.tar.zst`: a copy that is what the lock names is uploaded by a
  conditional write, unless S3 holds it, then read back and verified. A recording S3 already holds needs no local copy;
  one in neither stops the push, naming its sha256 and how to get it;
- prints the new `test-data.lock` and the git commands to run in a checkout of the pull request's branch: write the
  lock, `git add` it, and `git rm -r -q --cached --` the tracked data the archive now holds.

An archive path must have no control character, be NFC, and differ from every other path in the archive in more than
case; a symbolic link in a tool directory where pack test data goes is refused. Archiving, pulling and pushing all
check this.

With `--into <dir>`, a checkout of the pull request's branch at the commit `--from` names, it writes the locks there
itself and prints only the git commands. It runs no git there and reads only files: it refuses a checkout whose locks
or tracked pack test data are not byte for byte that commit's, such as the trusted worktree itself. Run the commands in
that checkout, commit and push; nothing there needs npm. The git you run there reads its `.git/config`, so make it one
no code of the pull request has run in, such as a fresh `gh pr checkout <n>` in a clone of its own. On a branch of your
own, `npm run fixtures:push -- --pack <id> --tool <tool>` archives the data as it is on disk (`--dry-run` shows what it
would archive) and prints the same commands.

Removing data: a file the archive holds is not in the pull request's diff, so deleting it from a checkout removes
nothing. An authored file goes with its row: the pull request deletes the file's row from the tool's `UPSTREAM.md` (and
the file, if it tracks it), and `fixtures:push --from` drops it as above. Derived files go with a derivation: the
sandbox commit below tracks all of the tool directory's data, and `--replace` archives exactly that, so a file the
derivation no longer makes is gone. Do not `--replace` a commit that tracks only part of the data: the rest would go
too. The old archive stays in S3 for the commits that name it.

The pull request is then checked by **Pack test data is in S3**, which stays red until S3 holds every archive and
recording its locks name, and, when it changes a `test-data.lock`, by **Test data reproduces from the pins**, which
derives the tool-pack again in CI and must find the new archive's data byte for byte.

When that check fails, its leg uploads what Linux derived as artifact `derived-<pack>--<tool>` (a tar of the tool
directory's pack test data). It is the Linux path for a maintainer without Linux, and the way to correct data that
does not reproduce: review the difference, then commit the derived files to a local branch and archive exactly them
from a trusted main worktree:

```sh
gh run download <run-id> -n derived-<pack>--<tool> -D /tmp/derived
git switch -c derived-<pack>-<tool> <pull request branch> && tar -xf /tmp/derived/derived.tar
git add -f -- packs/<pack>/upstream/<tool> && git commit -m "derived data, not for merge"
# in the main worktree:
npm run fixtures:push -- --from derived-<pack>-<tool> --replace --pack <pack> --tool <tool>
```

A pull request that changes a `recording.lock` needs the recordings it names. A same-repository pull request's port
readiness legs record every tool-pack whose `recording.lock` it changes, never reading the recordings the new lock
names from S3 or the cache, so what they upload is what CI's recorder made from the pinned upstream; a leg also records
an unchanged lock's recordings that neither holds. The legs upload the two as the artifact `recordings-<pack>--<tool>`
of its CI run, kept for 7 days; `<run-id>` is that run's (`gh run list --workflow ci.yml --branch <branch>`), and the
port readiness comment shows the per-rule summary of what upstream answers differently. That is also how
`rust/clippy`'s recordings are made on Linux. The downloaded files are data: `fixtures:push` reads only the ones the
lock names and checks each against its sha256. A fork's pull request has no such run: record it in a sandbox as below.

When a pull request changes a pin, a generator or a recipe, the derived data must be made again, and so must the
recording of a gated tool-pack. `fixtures:derive` and `port-readiness:record` run the pull request's own generators,
recipes and recorder, and the upstream code they fetch, so a maintainer reviews that code first and then runs it in a
sandbox: a clone of its own (not a `git worktree` of the trusted one, which shares its `.git` and hooks), in a
container or VM that can reach no AWS or GitHub credentials (no `AWS_*` variables, no `~/.aws` or `gh` login), on Linux
for `rust/clippy`, whose answers are Linux's (Clippy lints only the code rustc compiles for the host). `fixtures:push --from` takes a commit,
not a directory, so the sandbox commits the derived data on top of the pull request and hands the commit over as a git
bundle, which is data; the commit is amended to hold only the locks before anything is pushed:

```sh
# 1. In the trusted worktree: the archive the pull request's test-data.lock names (main's, unless it changes the lock).
npm run fixtures:pull -- --pack <id> --tool <tool>

# 2. In the sandbox, a clone checked out at the pull request, with that archive (the trusted worktree's
#    .rerail/pack-data/archives/<id>/<tool>/<sha256>.tar.zst, the sha256 its test-data.lock names) copied in, and the
#    samples archive its samples.lock names, when it has one (.rerail/pack-data/archives/samples/<id>/<tool>/):
mkdir -p .rerail/pack-data/archives/<id>/<tool> .rerail/pack-data/archives/samples/<id>/<tool>
cp <path to>/<sha256>.tar.zst .rerail/pack-data/archives/<id>/<tool>/
cp <path to>/<samples sha256>.tar.zst .rerail/pack-data/archives/samples/<id>/<tool>/
npm ci
npm run fixtures:pull -- --pack <id> --tool <tool>               # extracts the copied archive; reads no S3
npm run fixtures:derive -- --pack <id> --tool <tool> --update    # the derived data, from the pins
npm run port-readiness:record -- --pack <id> --tool <tool>       # a gated tool-pack (gates.yaml) only
git add -f -- packs/<id>/upstream/<tool> packs/<id>/licenses
git commit -m "Derive <id>/<tool>"
git bundle create pr-<n>-derived.bundle HEAD
# Copy out pr-<n>-derived.bundle and .rerail/port-readiness/recordings/*.tar.zst.

# 3. In the trusted worktree, with the recordings copied into .rerail/port-readiness/recordings/:
git fetch <path to>/pr-<n>-derived.bundle HEAD:pr-<n>-derived

# 4. In a fresh checkout of the pull request's branch:
git fetch <path to>/pr-<n>-derived.bundle HEAD:pr-<n>-derived
git merge --ff-only pr-<n>-derived

# 5. In the trusted worktree: the commit tracks all of the tool directory's data, so the archive is exactly that.
npm run fixtures:push -- --from pr-<n>-derived --pack <id> --tool <tool> --replace --into <the checkout of step 4>

# 6. In the checkout of step 4: the git commands it printed, then
git commit --amend --no-edit   # the commit now holds the locks and no test data
git push
```

`fixtures:derive` needs the network (the pinned upstreams and their packages), `uv`, and for the PHP and Blade packs
the pinned PHP (`conformance/recorders/port-readiness/runtimes/php/build.sh`) and Composer, for the Rust pack
`rustup`; it installs everything else under `.rerail/pack-data/`. It fails when a derivation changes a tracked file or
an authored one; `--update` (a pin change) keeps the tracked files it changes (a licence table) for the commit instead,
and `--compare <dir>` compares the result with another copy of the data byte for byte.

An upstream release needs nobody to start it: the nightly (`port-readiness-nightly.yml`) goes red when an upstream has
published a breaking release (a major one, or a minor one of a `0.x` pin), reports minor and patch releases without
failing, and each night starts one bump of every tool-pack behind on a work branch (`port-readiness-bump.yml`), a leg
per tool-pack, which derives and records the new test data with no OIDC token or write token and outside main's cache
scope, since it runs upstream's unreviewed code. A workflow on main (`port-readiness-bump-publish.yml`) then checks
each leg's result (it runs the bump itself and refuses a patch that differs from its own anywhere but the locks,
`UPSTREAM.md` and licence files), uploads the archives and recordings, and opens one pull request on
`auto/bump-upstream` with a commit per tool-pack, each carrying its new pins, `test-data.lock` and `recording.lock`
together. That pull request's CI derives each changed tool-pack again and checks its recording against every answer
upstream gives again, so neither the archives nor the answers are taken on the bump run's word. A maintainer reviews
and merges it like any pull request; nothing merges itself. When one tool-pack's gates fail, merge the others without
its commit; closing the pull request unmerged
declines the releases it carried until newer ones appear ([docs/port-readiness.md](docs/port-readiness.md#bump-pull-requests)).
The port readiness recordings are not pack test data and not in its archives: each tool-pack's is an object of its own
in S3, written once — every answer upstream gave the recorder on its own tests, one file per answer, as one canonical archive at `s3://nera-test-fixtures/rerail/recordings/<pack>/<tool>/<sha256>.tar.zst`
— which `recording.lock` names by its tar's sha256, and the Actions cache holds copies for CI.
`node scripts/s3/pack-fixtures.mjs recordings [--pack <id> [--tool <tool>]]` fetches the ones the checkout's locks name
into `.rerail/port-readiness/recordings/` and extracts each archive's tree beside it
([docs/port-readiness.md](docs/port-readiness.md#the-recordings-are-not-in-the-tree)).

An answer is filed under the upstream configuration and what it depends on, so a record asks upstream only for what
the archive lacks, and nothing when it lacks nothing: adding a port records that rule's answers (all the tool-pack's,
for an adapter whose answers depend on the rules recorded together) and leaves every other answer valid.
`npm run port-readiness:record -- --pack <id> --tool <tool>` records what is missing, `--full` everything from nothing,
and `--prune` drops what no recorded rule names ([docs/port-readiness.md](docs/port-readiness.md#the-recording)). A pull request's CI asks upstream for everything again
(`--check`) and accepts whichever of these the lock was recorded with: an incremental record over the base's archive,
`--full` or `--prune`. An answer the archive holds is never asked again by a record, so when the nightly `--check`
finds upstream now answers otherwise, record with `--full`, as it says. A lock from before the one archive (v2 or v3,
naming `recording.json`) is refused: delete it and record the tool-pack again.

A tool-pack's config id moves with its adapter's code (`ADAPTER_SOURCES` in
`conformance/port-readiness/recording/archive.ts`), so a fix to an adapter asks every answer again. The rest of the
recorder (`record.ts`, `recording/tree.ts`, `recording/schema.ts`) moves it only through `RECORDING_FORMAT` in
the same file: a refactor or a comment there asks nothing again, and a change to the tree it writes for the same upstream output fails `test/conformance/recorders/recording-format.test.ts` until you bump
`RECORDING_FORMAT` and update the test's pin, after which every tool-pack records again.

## Licence of contributions

Rerail is licensed under the Apache License 2.0. By submitting a contribution you agree that it is licensed under
the same terms, as described in section 5 of the licence. There is no contributor licence agreement and no
sign-off to add.

## Third-party material

Do not paste code, data or rules from another project without recording it:

1. Add an `UPSTREAM.md` entry beside the material: source project, version or revision, files, and whether each is
   verbatim, derived or an excerpt.
2. Add the upstream licence text (to `LICENSES/` or beside the material under `licenses/<project>/`).
3. Annotate the path in `REUSE.toml`.

`reuse lint` must pass. Only permissively licensed material is accepted, anywhere in the repository: the licence
texts in `LICENSES/` must be on the allowlist in `scripts/check/licence-allowlist.json` (Apache-2.0, MIT, BSD-2-Clause,
BSD-3-Clause, CC0-1.0, ISC, Zlib, 0BSD, Unicode-3.0), so no copyleft file can be committed.

Fixtures (`fixtures/`) and rule packs (`packs/`) follow the same rules; see their READMEs.

## Dependencies

New dependencies must be permissively licensed if they end up in anything Rerail distributes:
`npm run check:licenses` fails on a copyleft production dependency. Copyleft is acceptable only in
development-only dependencies and tools, which are never committed or distributed.
