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
the production workspace against the tool's expected findings in the pack's `upstream/` and, with `--samples`, on the
tool-packs' labelled real-code samples (`samples.lock`, [docs/upstream.md](docs/upstream.md#samples));
`--write` saves the report under `.rerail/reports/`. `npm run conformance:languages:check` is the gate over the fixtures,
their rename variants and the samples ([conformance/README.md](conformance/README.md#the-gate)); it runs offline in
`npm run test:conformance`, part of `test:all` in CI.

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
complexity pack's `lizard-*.tsv`), the expected findings on them (`*.expected.json`), and `authored/`. The
patterns are `PACK_FIXTURE_PATTERNS` in [scripts/s3/pack-fixtures.mjs](scripts/s3/pack-fixtures.mjs); everything
else under `upstream/<tool>/` (`gates.yaml`, the generators, `UPSTREAM.md`, `samples.lock`, `SAMPLES.md`,
`test-data.lock`) stays in git. The data is of two kinds:

- **Authored**: what Rerail wrote for the tool to score, which nothing can regenerate: the files the tool's own
  `upstream/<tool>/UPSTREAM.md` lists as Rerail's (pack check 6 reads the same list), and everything under
  `upstream/<tool>/authored/`, the place for a contributor's code corpus and its ground truth when the corpus is no
  public repository a pin can name.
- **Derived**: everything else, upstream's tests copied or derived (with the ECMA tool-packs' rename variants under
  `fixtures/renamed/`) and the expected findings on them. `npm run fixtures:derive` makes it from the tool-pack's
  pins, running the pack's generator (recipes in [scripts/pack-data/recipes/](scripts/pack-data/recipes/)). A
  maintainer runs it, and so does the bump workflow for an upstream release or a re-snapshot; pull request CI never
  runs it ([docs/upstream.md](docs/upstream.md#take-a-new-snapshot)).

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
another's ignoring case. Then it lays the tool-pack's `publish/` (below) over what it extracted, so the checkout holds
what publishing it would make. It deletes the derived files on disk that neither the archive nor `publish/` holds,
which are stale, and keeps authored ones, saying so. It never writes or reads through a symbolic link: when `packs/`,
a tool directory, its `publish/` or a directory an archive's file goes into is one, it stops before writing anything
and says to make it a directory. `.gitignore` covers the patterns, so pulled data does not show in `git status`.
Without the data, the pack tests (`npm test`, `npm run test:conformance`, `cargo test`) and the generators fail saying
to pull. To replay any commit, check it out and pull: its locks name the exact archives its CI judged.

A tool-pack with a `samples.lock` also has real-code samples: real files from open-source repositories, labelled per
rule with the lines it should report or as clean for it, which the conformance check judges the rules on
([docs/upstream.md](docs/upstream.md#samples)). They are one archive in S3 too
(`s3://nera-test-fixtures/rerail/archives/samples/<pack>/<tool>/<sha256>.tar.zst`), which `samples.lock` names in
`test-data.lock`'s format and `SAMPLES.md` describes (sources, licences, commits). `fixtures:pull` fetches and
verifies it with the test data and extracts it under `.rerail/pack-data/samples/`, never into the tool directory
(`node scripts/s3/pack-fixtures.mjs samples` fetches the samples alone). Nothing derives them: to change a set
(samples, labels, or a renamed or dropped rule's key), a maintainer's script loads the samples tree, changes it and
stages it with `writeSampleSet` (`scripts/samples/pack.ts`) into the tool-pack's `publish/samples/`, which is
published like test data (below).

CI pulls the same way, in two kinds of job (`.github/actions/pack-test-data`). A fetching job (`fetch: "true"`) only
warms the Actions cache: it looks up the entry keyed by its tool-packs' locks (`node scripts/s3/pack-fixtures.mjs
cache`) without restoring it and, on a miss, downloads the archives from S3 under the repository's read-only OIDC role
and saves them; it runs nothing but this commit's `scripts/s3/pack-fixtures.mjs`. A consuming job (`fetch: "false"`),
which holds no OIDC token, restores that entry, verifies each archive against its lock and extracts it, and fails on a
miss. Every job that reads pack test data consumes, after a fetching job of the same run: a job that can mint a token
never extracts a cache entry, and a job that runs upstream's code (the bump) has no token to mint. The fetching jobs
run for pushes and same-repository pull requests only: GitHub gives a fork's or Dependabot's pull request no OIDC
token.

**Pack test data is in S3** runs `node scripts/s3/pack-fixtures.mjs check` under the read role: a `head-object` for
every archive a `test-data.lock` or `samples.lock` names, downloading nothing, failing with each missing key, its lock
and its remedy. Main's runs read only S3, so a pull request cannot merge until S3 holds what its locks name. CI
verifies the data by sha256 only: it never derives it again or asks upstream. That is deliberate: pack test data is a
point-in-time artifact, and no CI job checks that it reproduces from the pins. What a pull request publishes is
checked by the code owner who approves its upload, reading prep's summary against the pull request's pin, generator
and recipe changes, and by the conformance check on the new data ([Merging](#merging-maintainers)). The required check
is **CI result**, which fails when any CI job failed or was cancelled, since GitHub counts a job skipped after a failed
dependency as passing ([docs/upstream.md](docs/upstream.md#ci)).

Structure metric #3 (`npm run check`) fails while any pack test data is tracked in the pulled layout, a tool directory
lacks a `test-data.lock`, a lock is malformed, or a `publish/` is tracked ("Pack data waiting to publish"), so a pull
request cannot merge until its data is in S3 and its locks name it.

### Adding or changing a pack

New or changed pack test data reaches S3 through the tool-pack's **`publish/`** directory,
`packs/<pack>/upstream/<tool>/publish/`: tracked, never ignored, and normally absent. Anything in it is a publish
request. `publish/<path>` becomes the test data archive's file `<path>` (`publish/fixtures/AL04/x.sql` →
`fixtures/AL04/x.sql`), and `publish/samples/<path>` the samples archive's; anything else there fails the publish.

1. Make the data in the usual, ignored layout: `npm run fixtures:derive -- --pack <id> --tool <tool> [--update]` or
   the pack's generator for derived data, your own cases for authored data (listed in the tool's `UPSTREAM.md`, or a
   code corpus with its ground truth under `upstream/<tool>/authored/`). The generators say so when they
   write data.
2. Stage it: `npm run fixtures:stage -- --pack <id> --tool <tool>` copies every file on disk that the locked archive
   lacks or holds differently into `publish/`. A file deleted on disk cannot be expressed that way: `--all` copies
   the whole set, for a publish with **replace**, which makes the archive exactly `publish/`.
3. `git add packs/<id>/upstream/<tool>/publish`, push, and open the pull request. Review sees the bytes in the diff
   (`*.expected.json` stays collapsed), and its CI judges the data as it would be published: `fixtures:pull` lays
   `publish/` over the locked archive, and the samples the runner reads are the locked set overlaid with
   `publish/samples/`.

The usual rules for third-party material apply (below), and every file going to S3 must be under a licence on
[scripts/check/licence-allowlist.json](scripts/check/licence-allowlist.json): test data by the
`packs/<id>/licenses/<project>/UPSTREAM.md` that lists it (Rerail's own files are Apache-2.0), samples by their labels'
`source.license`. An SPDX `A OR B` passes when either side is on the list, `A AND B` only when both are.

A comment on the pull request ("Pack data waiting to publish") lists the tool-packs and how to run the publish, by hand
and with the `gh` CLI.

Removing data: a file the archive holds is not in the pull request's diff. An authored file goes with its row: the
pull request deletes the file's row from the tool's `UPSTREAM.md`, and an overlay publish of that tool-pack drops it.
Anything else goes with a `replace` publish of the whole set (`fixtures:stage --all`). The old archive stays in S3 for
the commits that name it.

### Merging (maintainers)

A pull request that tracks a `publish/` is published by **Publish pack data**
([.github/workflows/pack-data-publish.yml](.github/workflows/pack-data-publish.yml)), which only runs on `main` and is
dispatched with the pull request's number (`replace` off by default):

```sh
gh workflow run pack-data-publish.yml -R neralabs/rerail -f pr=<n> [-f replace=true]
```

- **prep** (main's code; the read role only) packs every `publish/` of the pull request's head from its git blobs,
  never checking it out or running its code: per tool-pack and kind, the locked archive less the files its
  `UPSTREAM.md` stopped listing, overlaid with `publish/` (with replace, `publish/` alone). It refuses any other path
  under `publish/` and any file whose licence the allowlist does not take, before any approval is asked, and writes a
  summary: each archive's sha256, files, bytes and licences, and what it adds, changes and drops.
- **upload** waits for a code owner to approve the `test-data-upload` deployment, after reading that summary. Only
  then can it assume the write role. It verifies each archive against prep's outputs, puts it create-only (an object
  is never overwritten), tagged `rerail:commit`, `rerail:pr`, `rerail:run-id` and `rerail:uploader` (the approver),
  and then confirms every object: S3's head, and a fresh download verified against its lock.
- **finish**, only after every object is confirmed, commits the new locks and deletes `publish/` in one commit on the
  pull request's branch, pushed with the `neralabs-ci` app's token so CI runs on it, never forced. A fork's branch, or
  a push GitHub rejects because the branch moved, gets the locks in the comment instead: commit them, delete
  `publish/` and push, or pull and publish again. Re-running is safe: an object S3 already holds is accepted once it
  verifies, and nothing is put twice.
- **failed**, when an object was not put and confirmed (or the approval was rejected), says which in the comment;
  nothing is committed and no lock changes.

An archive path must have no control character, be NFC, and differ from every other path in the archive in more than
case; a symbolic link in a tool directory where pack test data goes is refused. Archiving, pulling, staging and
publishing all check this.

A maintainer with write credentials may still archive a tool-pack's data as it is on disk and upload it, tagged with
their `gh` login, with `npm run fixtures:push -- --pack <id> --tool <tool>` (`--dry-run` shows what it would archive);
the publish workflow is the routine path.

The pull request is then checked by **Pack test data is in S3**, which stays red until S3 holds every archive its
locks name, and by the conformance check on the published data.

No CI job derives the data again: the approving code owner is the check that it is what the pull request's pins,
generators and recipes make. prep's summary lists what each archive adds, changes and drops (up to 50 paths per list;
the dropped ones, which a replace publish removes, appear nowhere else) and its licences. Approve only when every change
follows from the pull request: data that changes with no pin, generator or recipe change to explain it is a reason to
reject the upload. To check a bump's data yourself, derive it again in the sandbox described below, at the pull
request's head with its `publish/`: `npm run fixtures:pull` (which lays `publish/` over the locked archive), then
`npm run fixtures:derive -- --pack <id> --tool <tool> --update` and
`npm run fixtures:stage -- --pack <id> --tool <tool> --all`; `git status -- packs/<id>/upstream/<tool>/publish` then
shows every file where the derivation differs from what the pull request publishes, and is clean when they agree.

When a pull request changes a pin, a generator or a recipe, the derived data must be made again. `fixtures:derive`
runs the pull request's own generators and recipes, and the upstream code they fetch, so a maintainer reviews that
code first and then runs it in a sandbox: a clone of its own, in a container or VM that can reach no AWS or GitHub
credentials (no `AWS_*` variables, no `~/.aws` or `gh` login), on Linux for `rust/clippy`, whose answers are Linux's
(Clippy lints only the code rustc compiles for the host). For a change already on main, the bump workflow's
`resnapshot` input does this in CI ([docs/upstream.md](docs/upstream.md#take-a-new-snapshot)). The sandbox stages the
whole derived set into `publish/` and hands the commit over as a git bundle, which is data:

```sh
# 1. In a checkout with read access: the archive the pull request's test-data.lock names (main's, unless it changes it).
npm run fixtures:pull -- --pack <id> --tool <tool>

# 2. In the sandbox, a clone checked out at the pull request, with that archive (the
#    .rerail/pack-data/archives/<id>/<tool>/<sha256>.tar.zst its test-data.lock names) copied in, and the samples
#    archive its samples.lock names, when it has one (.rerail/pack-data/archives/samples/<id>/<tool>/):
mkdir -p .rerail/pack-data/archives/<id>/<tool> .rerail/pack-data/archives/samples/<id>/<tool>
cp <path to>/<sha256>.tar.zst .rerail/pack-data/archives/<id>/<tool>/
cp <path to>/<samples sha256>.tar.zst .rerail/pack-data/archives/samples/<id>/<tool>/
npm ci
npm run fixtures:pull -- --pack <id> --tool <tool>               # extracts the copied archive; reads no S3
npm run fixtures:derive -- --pack <id> --tool <tool> --update    # the derived data, from the pins
npm run fixtures:stage -- --pack <id> --tool <tool> --all        # the whole set, for a replace publish
git add -- packs/<id>/upstream/<tool>/publish packs/<id>/licenses
git commit -m "Derive <id>/<tool>"
git bundle create pr-<n>-derived.bundle HEAD

# 3. In a fresh checkout of the pull request's branch:
git fetch <path to>/pr-<n>-derived.bundle HEAD:pr-<n>-derived
git merge --ff-only pr-<n>-derived
git push

# 4. Publish it with replace:
gh workflow run pack-data-publish.yml -R neralabs/rerail -f pr=<n> -f replace=true
```

`fixtures:derive` needs the network (the pinned upstreams and their packages), `uv`, and for the PHP and Blade packs
the pinned PHP (`scripts/pack-data/runtimes/php/build.sh`) and Composer, for the Rust pack
`rustup`; it installs everything else under `.rerail/pack-data/`. It fails when a derivation changes a tracked file or
an authored one; `--update` (a pin change) keeps the tracked files it changes (a licence table) for the commit instead.

An upstream release needs nobody to start it: each night `upstream-releases.yml` reports every tool-pack whose upstream
has published a newer stable release (without failing) and starts one bump of all of them on a work branch
(`upstream-bump.yml`), a leg per tool-pack, which moves the pins and derives the new test data into each stale
tool-pack's `publish/` with no OIDC token or write token and outside main's cache scope, since it runs upstream's
unreviewed code. A workflow on main (`upstream-bump-publish.yml`) then checks each leg's result (it runs the pin edits
itself and refuses a patch that differs from its own anywhere but the files deriving and staging write), opens one
pull request on `auto/bump-upstream` with a commit per tool-pack, and dispatches **Publish pack data** for it with
replace, whose upload a code owner approves like any other. A maintainer reviews the pin diffs, the data and the
`gates.yaml` `rules` facts, and merges it like any pull request; nothing merges itself. When one tool-pack fails the
conformance check, merge the others without its commit; closing the pull request unmerged declines the releases it
carried until newer ones appear ([docs/upstream.md](docs/upstream.md#bump-pull-requests)). To derive tool-packs again
at their current pins, point `auto/bump-run/upstream` at main's head and dispatch `upstream-bump.yml` there with
`resnapshot` on.

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
BSD-3-Clause, CC0-1.0, ISC, Zlib, 0BSD, Unicode-3.0, PostgreSQL), so no copyleft file can be committed.

Fixtures (`fixtures/`) and rule packs (`packs/`) follow the same rules; see their READMEs.

## Dependencies

New dependencies must be permissively licensed if they end up in anything Rerail distributes:
`npm run check:licenses` fails on a copyleft production dependency. Copyleft is acceptable only in
development-only dependencies and tools, which are never committed or distributed.
