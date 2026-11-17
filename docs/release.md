# Releasing Rerail

How core is built, versioned and published. Rule packs release separately; see [pack-format.md](pack-format.md#release).

## Release

Merging the Release pull request tags a version; the tag builds, checks and publishes core.

### Build

`.github/workflows/release.yml` builds the package the updater installs, via `.github/workflows/package.yml`, and
test-installs it on each platform before publishing.

<details>
<summary>Build and check steps</summary>

1. Compile the native kernel for macOS, Linux glibc and Windows MSVC, on arm64 and x64.
   - Linux builds against glibc 2.17 with napi-rs's cross toolchain; a kernel needing glibc newer than 2.28 is rejected.
   - A Windows kernel importing the Visual C++ runtime is rejected. The kernels link it statically, so a clean Windows
     install needs no redistributable.
2. Smoke-test native loading and device storage on each host.
3. Build the runtime and bundled UI; pack only if all six `native/*.node` files are present.
4. Install the tarball into a scratch prefix; check `rerail --help`, the bundled UI and `nativeHealth()`.
5. On Windows x64, Linux x64 and macOS arm64, install it as a user would and exercise the managed runtime:
   `scripts/host/windows-install-smoke.ps1` on Windows, the environment matrix's `scripts/host/azure/guest.sh` through
   `matrix.mjs local` on Linux and macOS. Nothing publishes unless all three pass.

</details>

### Release pull request

`.github/workflows/release-pr.yml` refreshes the **Release** pull request on every push to `main`, unless the release
is frozen ([Freezing a release](#freezing-a-release)). It also carries the rule pack bumps
([docs/pack-format.md](pack-format.md#release)).

- **Bump:** if anything core ships changed since the last `v<version>` tag, it bumps to the next patch (`0.1.0` →
  `0.1.1`) and writes its `CHANGELOG.md` section.
- **Core ranges:** a pack it releases that an older core cannot load, because it holds a rule off or ships a rule in
  a language added since, has its core range raised to leave those cores out, once core is at the release that first
  loads it. The description lists each.
- **Merge:** tags the released commit `v<version>`, which starts `release.yml`, and `release-<commit>`, which starts
  the packs' release. The released commit is the pull request's head, or the candidate when the release is frozen;
  never the merge commit on `main`, which can hold later work.

<details>
<summary>Exact bump and tag details</summary>

- The bump changes the root entries of `package.json` and `package-lock.json`. The `CHANGELOG.md` section comes from
  the unreleased lead and pending `.changelog/` entries.
- The core range is raised in the pack's `package.json` and in `package-lock.json`, by the rule the pack validation
  judges it by (`scripts/release/core.mjs`). A pack that needs it and has no changeset is not released, so it is
  named in a warning and left alone: add a changeset for it. Only a plain `>=<version>` range is rewritten, since a
  raise replaces the whole range: a released pack with an upper bound or alternatives in its range stops the run, to be
  raised by hand, and one that is not released is named in the same warning.
- A pack released while core stays older than the release that first loads it, because core is left out of the pull
  request, is named in a warning not to merge: published so, no released Rerail could load it.
- The tags are pushed with the `neralabs-ci` app's token, by the run the merge starts
  (`scripts/release/release-tag.mjs`). It reads the last merged Release pull request, so a run cancelled by a newer
  push loses nothing: the next one tags the same commit.
- **Anything core ships:** every path except packs, documentation, CI, changesets and pending changelog entries, tests,
  fixtures, corpora, benchmarks, development and release scripts, and repository files npm does not pack.
  `package-lock.json` counts only when more than its pack entries moved (`NOT_SHIPPED` in
  `scripts/release/core-paths.mjs`).

</details>

### Before merging

The Release pull request's description ends with a checklist. Whoever merges it ticks each box first:

- **End-user testing.** One hour using the staged release the way a user does. Freeze the release first
  ([Freezing a release](#freezing-a-release)), so merges to `main` neither change what you test nor clear the boxes.
  - **Core:** frozen, the **Release** run on `changeset-release/candidate` holds it as the `rerail-package` run
    artifact, unpublished. Not frozen, `gh workflow run release.yml --ref changeset-release/main` builds it
    ([Manual runs and put-back](#manual-runs-and-put-back)).
  - **Packs:** the **Release packs** run, on `changeset-release/candidate` when frozen and on the pull request
    otherwise (frozen, the pull request's own run uploads none), uploads the tarballs it would publish as the
    `rule-packs` artifact; `rerail packs add <tarball>`
    installs one. A pack from a tarball counts as unpublished, so its rules run in Log until approved
    ([rule packs](rule-packs.md)).
- **Release notes.** Accurate, complete, and written for the person upgrading ([Release notes](#release-notes)).
- **Documentation.** `README.md` and `docs/` are accurate for what the release ships.
- **`context7.json`.** Its folders, exclusions and rules match the docs and commands, so AI tools pull current Rerail
  docs.
- **[Internal testing](internal-testing.md).** The page lists exactly the features in internal testing, and marks each
  one graduated or removed.
- **Scrub.** No internal name, customer name, internal URL or old product name in the code, the docs, or the commit
  messages and pull request text since the last release. `npm run check` catches only internal ticket IDs and a few
  stale paths in tracked files.
- **npm package pages.** `README.md`, and `keywords` and `repository` in `package.json`, are current for core and
  every pack.

Each pull request's author confirms the same of their own change
([CONTRIBUTING.md](../CONTRIBUTING.md#before-you-open-a-pull-request)); this is the check of the release as a whole.

Whatever changes what merging releases rewrites the description and clears the boxes: a push to `main`, or, for a
frozen release, a cherry-pick, freezing a different commit or unfreezing. Nothing in CI reads them: the checklist is
the merger's word (`BEFORE_MERGING` in `scripts/release/release-version.mjs`).

### Freezing a release

Freezing holds the release at one commit while `main` stays open: a release candidate pinned to a commit, with no
release branch to maintain. Run the actions from the terminal, or from Actions → Release pull request → Run workflow.

1. **Freeze.** `gh workflow run release-pr.yml -f action=freeze` holds the release at `main`'s tip; add
   `-f commit=<commit>` for an earlier commit on `main`. CI, the core build and the pack build start on the candidate.
2. **Test.** Use the candidate's builds ([Before merging](#before-merging)). Merges to `main` change nothing.
3. **Fix.** Merge the fix to `main` as any pull request, then add it to the release:
   `gh workflow run release-pr.yml -f action=cherry-pick -f commit=<commit or pull request number>`.
4. **Release.** Tick the checklist and merge the Release pull request. It releases the candidate, and brings `main` the
   versions and changelogs.

`gh workflow run release-pr.yml -f action=unfreeze` follows `main` again. Each action comments on the pull request
with what the release now is.

To ship a fix on top of the latest release without the rest of `main`, freeze that release's tag with the fix:
`gh workflow run release-pr.yml -f action=freeze -f commit=v<version> -f fixes="<commit or pull request number> ..."`.

<details>
<summary>What is guarded</summary>

Every action changes the release in one push or refuses, and a refusal says what to run instead.

- **Only tested commits merge.** The pull request's **Release candidate** check, part of the required **CI result**,
  passes once CI has passed on the candidate itself. Each cherry-pick makes a new candidate and runs CI on it again.
- **Fixes reach `main` first.** A cherry-pick takes only a commit on `main`, so `main` never lacks a fix a release has.
- **Nothing untested rides along.** Freezing takes a commit on `main` or the latest release's tag, and nothing else.
  Freezing a different commit keeps the fixes it does not hold.
- **The branch can't be changed by hand.** A head that `release-pr.yml` did not build, such as one from GitHub's
  **Update branch**, fails the check: it would release all of `main`.
- **The candidate holds only reviewed commits.** The pull request shows the Release commit alone, so the check
  rebuilds the candidate: it must be its base, cherry-picks that are commits on `main`, and a Release commit that does
  exactly what the pull request shows.
- **A fix that doesn't apply changes nothing.** One that builds on later work is refused, naming it: cherry-pick what
  it builds on first, or freeze a newer commit.
- **Old commits are refused.** A tag's run uses the tagged commit's own workflows, so a commit from before this flow
  can't be frozen.
- **Nothing publishes early.** A candidate's builds are unpublished. Packs publish only on a `release-*` tag's push,
  core only on a `v*` tag's, and both wait for the `release` environment's approval.
- **A release tag pushed by hand releases nothing.** `release-packs.yml` holds a `release-*` tag to its own commit's
  name and to a commit a merged Release pull request released. That stops a mistake; a tag's run uses the tagged
  commit's own workflow, so who may create `release-*` and `v*` tags is the tag ruleset's to limit.

</details>

<details>
<summary>How a frozen release is built</summary>

- **Two commits.** The candidate (`changeset-release/candidate`) is the frozen commit, the cherry-picks in `main`'s
  order, and the Release commit. It is what is built, tested, tagged and published. The pull request's branch holds
  the same Release commit on `main`'s tip, so merging stays a clean squash; merging the candidate itself would conflict
  on every release note `main` reworded since.
- **Pushes to `main`** leave both alone, so nothing reruns and the checklist stands. Only if `main` changes a file the
  Release commit changes is that commit applied to `main`'s tip again; the candidate is the same.
- **A release note `main` changed after the freeze** ships as it was frozen unless its commit is cherry-picked; the
  pull request gets a comment naming it. If `main` changes any other line the Release commit rewrites, such as the
  lead in `CHANGELOG.md`, the pull request can't merge until that commit is cherry-picked or the release is unfrozen;
  a comment names the files and the command.
- **Entries added to `main` after the freeze** stay pending for the next release. A cherry-picked fix's entry ships in
  this one and leaves `main` with the merge.
- **State** is the pull request's head commit, which names the candidate; the candidate's message names its base and
  cherry-picks (`scripts/release/candidate/candidate.mjs`).
- **After the merge**, the tags hold the released commit and the candidate's branch is removed.

</details>

### Installer plugins

The same bump re-pins the installer plugins (`plugins/claude`, `plugins/codex`, `plugins/cursor`) to the version just
released. Publish a new plugin version only once the release is on npm.

<details>
<summary>Per-store publishing, pins and stamping</summary>

- **Claude:** developer portal; follows `main`, live when a reviewer publishes it.
- **Codex:** upload a new ZIP of `plugins/codex` at platform.openai.com/plugins.
- **Cursor:** Cursor's team reviews each repository update by hand.
- The bump updates each manifest's `version` and the `npx @neralabs/rerail@<version>` its `install` skill runs
  (`PLUGIN_PINS` in `scripts/release/core.mjs`).
- Checked-in manifests carry no version. `npm pack` stamps `package.json`'s version in (`prepack`) and restores the
  source form (`postpack`). After a failed pack, run `node scripts/build/stamp-plugin-versions.mjs restore`.

</details>

### Tags and channels

A `v<version>` tag, equal to `package.json`'s version, publishes `@neralabs/rerail` to npmjs on the channel it names,
after a reviewer approves.

| Tag | Channel |
| --- | --- |
| `v0.2.0` | `latest` (stable) |
| `v0.2.0-beta.1` | `beta` |
| `v0.2.0-next.1` | `next` |

- `beta` and `next` are opt-in: `rerail update --channel <tag>`.
- The Release pull request makes only plain versions, for `latest`. For a `beta` or `next` preview, set the version
  and push the tag by hand.

<details>
<summary>Publishing rules and checks</summary>

- Publishing uses npm trusted publishing from the protected `release` environment. The channel is the npm dist-tag.
- Other prerelease suffixes are rejected. The run refuses a preview older than core's `latest` before building.
- `next` never trails `latest`: a `latest` release raises `next` to it (`scripts/release/raise-next.mjs`) unless `next`
  holds a newer preview. Whoever follows `next` gets every stable release. A `beta` or `next` release leaves other tags
  alone.
- Re-running the publish job is safe: npm refusing to publish over an existing version counts as published.
- The run's summary lists core's dist-tags. It fails if the channel's tag is not the new version, or, for `latest` and
  `next` releases, `next` is unset or older than `latest`.

</details>

### Manual runs and put-back

`workflow_dispatch` builds a trial tarball, retries a failed release, or puts `latest` back to an earlier version.

- **No inputs:** uploads the tarball as a run artifact, unpublished. Try it with `npm install <tarball>`.
- **`publish` set to a version:** retries that version's release from `main`; see below.
- **`latest` set to an earlier released version:** builds nothing and points `latest` back to it.

<details>
<summary>Retrying a failed release</summary>

Release tags can't be moved or deleted, and re-running a failed run reuses the tag's scripts. So when a release fails
before publishing because of something that doesn't ship, such as a workflow or a release script, fix that on `main`
and run:

```bash
gh workflow run release.yml --ref main -f publish=<version>
```

The run builds `main`, runs every check and publishes behind the same approval. The tag stays where it is, and npm's
provenance names the `main` commit. It is refused unless all of these hold, so core's own code is the tag's:

- it runs from `main` with `latest` left empty, and `main`'s `package.json` names that version;
- the `v<version>` tag exists;
- nothing core ships has changed since the tag (`publishRetryRefusal` in `scripts/release/core-paths.mjs`).

How the package is built and its changelog can still differ from the tag, so that a fix to the build can be retried
too. The run summary lists every path changed since the tag, starting with those that can change the tarball (`PACKAGE_INPUTS`): the build
workflow and its actions, the shrinkwrap step, `.gitattributes` and `CHANGELOG.md`. Check that list before approving.

If the fix changes what ships, it takes the next version: the Release pull request offers one as soon as the fix is on
`main`. Say in that version's lead that the failed version was never published, since its notes won't reach anyone
updating past it.

A retry builds `main`, so it is also refused when `main` gained shipped changes while the release was frozen. Release
the next version then, frozen at a commit you have tested.

</details>

<details>
<summary>What a put-back also does</summary>

The put-back uses trusted publishing behind the same approval as a release, so no one needs an npm login to put core
back. If `next` is older than that version, it raises `next` to it too. The run's **GitHub Release** job then makes
that version's Release the repository's latest.

</details>

### Build cache

`.github/workflows/release-cache.yml` keeps the native builds' cargo caches warm. After changing `Cargo.lock`, the Cargo
manifests, `.cargo/config.toml` or `rust-toolchain.toml`, let it finish before merging the Release pull request.
Otherwise the release builds cold.

<details>
<summary>How the caches are used</summary>

- A tag's run restores only caches saved on `main`; `package.yml` restores only the exact match.
- On `main` it runs nightly and after changes to those files. Each target lacking a matching cache builds from scratch
  and saves one.

</details>

### Environment matrix

`.github/workflows/env-matrix.yml` builds the same package after each push to `main` that can change what ships. It
installs it as a user would; see [docs/environment-matrix.md](environment-matrix.md) for environments, checks and manual
runs.

<details>
<summary>Machines and user path</summary>

- **Throwaway Azure VMs:** Ubuntu and Debian on x64 and arm64, RHEL on x64, Windows 11 on x64 and arm64.
- **GitHub-hosted macOS runners:** macOS 26 and 15 on Apple silicon, macOS 15 on Intel.

Each follows a user's path: `npx @neralabs/rerail setup`, the managed runtime, PATH in a new shell, the coding-agent
hooks, daemon restart and removal.

</details>

### Release notes

Release notes pitch the release to people who opt into each update; the GitHub Release publishes them unchanged.

- **Entries.** Each pull request changing what core ships adds one file in `.changelog/`, its name opening with its
  change type (`added-`, `changed-`, `deprecated-`, `removed-`, `fixed-`, `security-`;
  [.changelog/README.md](../.changelog/README.md)): one or two plain sentences for the upgrader, at most 60 words.
- **Lead.** Whoever readies the release writes a paragraph under `## Unreleased` at the top of `CHANGELOG.md` on
  `main`: what the release gives and why to take it.
- **Format.** From 0.1.1 the notes follow [Keep a Changelog 1.1.0](https://keepachangelog.com/en/1.1.0/), with the two
  differences `CHANGELOG.md`'s header states: headings read `## <version> (<date>)`, and pending entries wait in
  `.changelog/` rather than under `## Unreleased`. A pack's notes take the same format and types, its lead and rule
  entries written from `pack.json` ([pack-format.md](pack-format.md#release)).
- **Versions.** Below 1.0.0 a release is a patch unless it names another, and a patch can change or remove behavior,
  which Semantic Versioning allows below 1.0.0. To release a minor or major, name it in the heading:
  `## 0.2.0 (unreleased)`. What core promises from 1.0.0 is not set yet.

<details>
<summary>Why, assembly and CI checks</summary>

Entry files mean pull requests never edit the same lines of `CHANGELOG.md`. Entries leave detail to the docs they link,
and leave out commit hashes, ticket ids, pull request numbers and repository-internal maintenance. The lead goes before
the first entry; add the `## Unreleased` heading if the last release took it. It belongs on `main` because every push
rewrites the Release pull request.

**Assembly.** The Release pull request renames the heading `## <version> (<date>)`, dated the day it last updated from
`main`. It puts pending entries under the lead, under a `### <type>` heading for each change type that has one (Added,
Changed, Deprecated, Removed, Fixed, Security, in that order, file-name order within a type), and deletes their files.
With neither lead nor entry, the section says no entries were written. Should an entry with no type reach `main`, core
is left out of the Release pull request, and its description says why, until the file is renamed; the packs still
release.

**CI** (`scripts/release/core-changelog-check.mjs`, in `ci.yml`'s lint job) fails:

- a pull request with a pending `.changelog/` entry that has no change type, or whose added or changed entry runs
  long, names a ticket, pull request or commit, or starts with its own `- `;
- a pull request writing an entry into `CHANGELOG.md`'s first section, under `## Unreleased` or the last release's
  dated heading;
- the Release pull request until the dated section has its lead. One pull request releases core and packs, so this
  also holds back the packs' release.

</details>

### GitHub Releases

Every released version, core's and each pack's, gets a GitHub Release on its tag, with its `CHANGELOG.md` section as
notes.

**Backfill** creates Releases for tags without one, such as older releases or runs that failed before their Release:

1. Sync tags: `git fetch --tags --prune --prune-tags` (add `--force` for a tag rejected as changed).
2. Preview: `node scripts/release/github-release.mjs --backfill --dry-run`.
3. Run it without `--dry-run`; your gh login needs repository write access.

<details>
<summary>How Releases are made, and which tags backfill skips</summary>

- `scripts/release/github-release.mjs` builds them, so notes are written once.
- `release.yml`'s **GitHub Release** job makes core's, titled `v<version>`: a pre-release for `next` and `beta`, the
  repository's latest release for `latest`. It runs once the tarball passes the shrinkwrap check and the three managed
  installs, beside the npm publish, and waits on nothing npm does: a slow publish or a failed dist-tag read-back
  cannot hold back or skip it. If the publish then fails, the Release stands, and the retry (`publish` input) runs the
  job again.
- A version without its own changelog section says no changelog entries were written.
- Re-runs rewrite an existing Release's notes, pre-release and latest flags. After a put-back, the restored version
  becomes the latest release.
- `release-packs.yml` makes a pack's Release ([pack-format.md](pack-format.md#release)).
- Step 1 makes the checkout's tags the remote's. The dry run prints each Release it would create, with notes from the
  changelog at the tag's commit.
- Backfill reads only the remote's tags, and names each tag it skips: local-only, local commit differing from the
  remote's, or no changelog section for its version. So every Release it makes has notes.
- Only the highest core version on `latest` becomes the latest release.

</details>

## Shrinkwrap

The tarball ships an `npm-shrinkwrap.json`, so installs get exactly the dependencies `package-lock.json` pinned.

- **Write failures.** Run `npm install` and commit the lockfile.
- **Failed pack.** Remove the leftover with `node scripts/release/shrinkwrap-pack.mjs remove`, or npm would use it
  instead of `package-lock.json`.

<details>
<summary>How the shrinkwrap is written and checked</summary>

- The checkout's only lockfile is `package-lock.json`; it never commits a shrinkwrap.
- npm honors a published shrinkwrap instead of resolving versions and integrity hashes fresh; the registry flags such
  versions (`_hasShrinkwrap`).
- **Writing.** `npm pack` writes it (`prepack`: `node scripts/release/shrinkwrap-pack.mjs write`) and deletes it after
  (`postpack`); `package.json`'s `files` lists it.
- **Contents.** The root and every package its dependencies reach in `package-lock.json`: no development dependency,
  workspace pack, or optional peer nothing else needs. Packages reached only through optional dependencies are marked
  `optional`.
- **No source builds.** Every dependency's entry sets `gypfile: false`. npm reads a shrinkwrapped dependency from its
  entry, not its own `package.json`, so without it npm compiles `better-sqlite3` with `node-gyp` during
  `npx @neralabs/rerail setup`, and the install fails on an image with no compiler, such as `node:24-slim`.
- **Write failures.** Writing fails if `package-lock.json`'s root dependencies differ from `package.json`'s, or the
  lockfile lacks a package a dependency needs.
- **Release check.** Before publishing, `release.yml` runs `node scripts/release/shrinkwrap-pack.mjs check <tarball>`.
  It fails on a missing shrinkwrap or one differing from the tag's `package-lock.json`, naming the packages that differ.
- **Layout.** npm installs a shrinkwrapped package's dependencies inside it, under
  `node_modules/@neralabs/rerail/node_modules`, not beside it.

</details>

The managed install (`src/setup/updater.ts`) runs no install scripts (`--ignore-scripts`). A new dependency needing an
install script would break after `rerail update`; check before adding one.

<details>
<summary>Install-time verification</summary>

- No production dependency needs an install script; `better-sqlite3` ships prebuilt binaries.
- Before running staged code, it verifies registry signatures and provenance attestations of everything installed with
  `npm audit signatures`, stopping on any failure.
- A registry without signing keys, such as some private mirrors, gets a warning and the install continues.

</details>
