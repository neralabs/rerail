# Upstream snapshots and publishing data

A pack's ported rules are judged against upstream: a linter at the release its tool-pack pins. Upstream is
provenance at a point in time. What it said is captured once, put in S3 and pinned by sha256 in a tracked lock, and
from then on CI only verifies the pinned bytes and scores the rules on them. CI never runs upstream, and never checks
whether upstream would still answer the same: the snapshot goes stale only when a maintainer decides to take a new one.

A tool-pack (`packs/<pack>/upstream/<tool>/`) has two kinds of pack data, each one archive in S3 that a tracked lock
names:

| Pack data | Lock | What it holds | How it changes |
| --- | --- | --- | --- |
| Test data | `test-data.lock` | upstream's tests, the expected findings (`fixtures.expected.json`) and the cases Rerail wrote for the tool to score | authored cases by hand; derived data only by derive, which is paused ([Derive and bumps are paused](#derive-and-bumps-are-paused)) |
| Real-code samples | `samples.lock` (optional) | real files from open-source repositories, labelled per rule | the sample builders in `scripts/samples/`, a library API ([Samples](#samples)) |

Either kind changes through a pull request that stages the new data, and reaches S3 through one workflow on main once
a code owner approves ([Publishing pack data](#publishing-pack-data)). The files, the locks and the archives are
described in [pack-format.md, "Pack data"](pack-format.md#pack-data), and how each copied file's licence is decided in
[pack-format.md, "Licensing"](pack-format.md#licensing). The step-by-step for each actor (contributor, maintainer,
approver) is in [CONTRIBUTING.md, "Pack test data"](../CONTRIBUTING.md#pack-test-data) and [REVIEW.md](../REVIEW.md);
this page is the reference.

What the rules must pass is the conformance check (`npm run test:conformance`), described with each of its checks in
[conformance/README.md](../conformance/README.md#the-gate). Each gated tool-pack's `gates.yaml` is its upstream
manifest: the pins (`upstream.version`, `upstream.commit`, the runtime), the file types upstream lints
(`upstream.lints`), and the per-rule facts declared at the pins (`rules`), which a maintainer reviews at each bump.

## Samples

A sample is a real file from an open-source repository, labelled for the pack's rules: per rule, the lines the rule
should report, or `[]` when the file is clean for it; a rule the labels do not name is not judged on the file. Labels
mark intent, by line (a column, when present, is provenance). How the conformance check judges a rule on them is in
[conformance/README.md, "How samples are judged"](../conformance/README.md#how-samples-are-judged); the archive's
layout, `samples.lock` and `SAMPLES.md` are in [pack-format.md, "Pack data"](pack-format.md#pack-data).

`npm run fixtures:pull` fetches each tool-pack's samples archive with its test data
(`node scripts/s3/pack-fixtures.mjs samples` fetches the samples alone) and extracts it under
`.rerail/pack-data/samples/<pack>/<tool>/<sha256>/`, never into the tool directory. Samples staged under the
tool-pack's `publish/samples/` are laid over the locked set, so a pull request's CI judges them before they are
published.

To add, relabel or drop samples, or to rename or drop a rule key in the labels (a label naming no rule of the
tool-pack fails its whole pack's conformance check, which catches typos), a maintainer's one-off script calls the
sample builders: it loads the set, changes it, and writes the samples to add or replace into `publish/samples/` with
`writeSampleSet` (`scripts/samples/pack.ts`), which checks them as the publish does and writes a `REUSE.toml` that
licenses the staged copy. The same pull request updates `SAMPLES.md`, and the set is then published like any pack data.
The builders are a library API that tests and such scripts call. There is no command for them yet, and no standing
policy for trimming a set: a `SAMPLES.md` says how its set was trimmed, where it was, when it was built.

### Sample licences

A sample is a whole file of another repository, so its licence comes from a recorded table, not from the sample's own
labels. `RECORDED_LICENCES` in `scripts/samples/licences.ts` names every repository samples may be taken from
(`owner/name`) with its SPDX licence: the repositories whose licence files the builders read at the samples' commits
(`SOURCE_LICENCES`, checked against the text on every build), and the ESLint test set's sources, with the licence the
test set recorded. `COPYLEFT_SOURCES` lists the repositories found copyleft, with the evidence.

When samples are published, `publishedSamples` (`scripts/samples/publish.ts`) refuses:

- a sample whose repository has no entry, or whose labels declare a `source.license` other than the entry's;
- a sample from a copyleft source;
- a path that is neither `LICENSES.md` nor `<sample-id>/<path>`, or a sample not staged whole: its file beside its
  labels, the file's sha256 equal to the labels' `source.sha256`.

The publish's licence check then judges each sample's licence, which is now the recorded one, against the allowlist
(`scripts/check/licence-allowlist.json`), and the approver's summary lists every repository the archive takes files
from with that licence.

The archive's `LICENSES.md` lists each repository, its licence and the files taken from it, with the licence texts
the builders read at the samples' commits. The ESLint test set's sources carry only the licence id the test set
recorded, with no text, so an archive holds licence texts only for the repositories whose licence files were read, and
`SAMPLES.md` says which: all of them, none, or the ones it names. A pull request that takes samples from a new
repository adds its entry, and the reviewer reads the repository's licence file at the samples' commits.

## Take a new snapshot

A snapshot of the derived test data changes only on purpose, in one of three ways, each ending in a pull request that
carries the new data in `publish/`. All three are paused for now
([Derive and bumps are paused](#derive-and-bumps-are-paused)); this section describes them as they run once the pause
lifts:

- **A bump** moves the pins to a newer upstream release and derives the data again
  ([Bump pull requests](#bump-pull-requests)); the nightly starts one for every tool-pack behind.
- **A re-snapshot** derives the data again at the release already pinned: a generator or recipe change, a runtime
  pin's, or data only Linux makes. It is the bump workflow's `resnapshot` input.
- **A local derive**: `npm run fixtures:derive -- --pack <id> --tool <tool> [--update]`, then
  `npm run fixtures:stage -- --pack <id> --tool <tool> --all`, and the publish with `replace`. A maintainer runs
  it on a pull request's own pins, generators and recipes in a sandbox ([Derive in a sandbox](#derive-in-a-sandbox)).

The generators run upstream's code, so they run on the authoring path only: a maintainer's sandbox or the bump
workflow, never pull request CI.

### What the generators write

Each generator writes the tool-pack's test data in the pulled layout under `packs/<id>/upstream/<tool>/`: upstream's
test cases under `fixtures/`, and `fixtures.expected.json`, the expected findings (what upstream reports on them,
`expectedFindingsSchema` in `conformance/languages/pack.ts`). Its `source.units` gives each unit's upstream origin:
`rules`, the upstream rules whose tests the unit was written from, which is what makes a unit one of a rule's own for
`stays-silent` and `upstream-rule`. A unit with no entry (Rerail's probes, or a unit whose origin is unknown) is
scored but is nobody's own evidence. The ESLint-based generators share `scripts/pack-data/eslint-expected.mjs`.

For the ECMA tool-packs the generators also write a rename variant of every upstream unit under `fixtures/renamed/`
(`fixtures/renamed/<the original's path>`): a copy whose identifiers and string and number literal contents are
renamed by `scripts/pack-data/rename/ecmascript.ts` (same length and character shape, one bijective map per file, the
language's keywords, standard library and common host names protected). Upstream lints the copy like any unit, so its
expected findings are upstream's own answer on it, and its `source.units` entry carries `renamedFrom`, the key of the
original. The conformance runner scores variants apart from the originals, as `holds-under-rename`: a port that
matches the test cases' names or literals instead of the construct fails it. A family without variants yet takes its
overfit evidence from the samples alone.

### Runtimes

`fixtures:derive` installs the runtimes a generator needs from `scripts/pack-data/runtimes/` (`runtimes.ts`): upstream's
own (`gates.yaml`'s `upstream.runtime`) and the adapter's (`python` for Ruff and Bandit, `php` for PHP_CodeSniffer,
PHPStan and Psalm, `blade` and `php` for tlint and stillat/blade-parser). Each runtime is pinned by its
`package.json`, `pyproject.toml` or `composer.json` alone: every direct dependency at an exact version and in
`overrides`, and no lockfile is committed. A Python runtime also pins its exact interpreter in `.python-version`, which
uv installs; a PHP runtime pins its interpreter in `.php-version`, which Composer cannot install, so derive refuses any
other `php`. Every PHP process runs with no php.ini (`php -n`, `PHP_INI` in `runtimes.ts`), and
`runtimes/php/extensions.txt` lists the extensions `runtimes/php/build.sh` compiles in; a `php` that loads any other
set is refused. Transitive packages resolve when a runtime is installed: when one changes what upstream answers, pin it
too and take a re-snapshot.

```sh
scripts/pack-data/runtimes/php/build.sh "$PWD/.rerail/php/8.5.11"
PATH="$PWD/.rerail/php/8.5.11/bin:$PATH" npm run fixtures:derive -- --pack php --tool phpcs
```

### Derive in a sandbox

Paused, like every derive. When a pull request changes a pin, a generator or a recipe, the derived data must be made
again. `fixtures:derive` runs the pull request's own generators and recipes, and the upstream code they fetch, so a
maintainer reviews that code first and then runs it in a sandbox: a clone of its own, in a container or VM that can
reach no AWS or GitHub credentials (no `AWS_*` variables, no `~/.aws` or `gh` login), on Linux for `rust/clippy`, whose
answers are Linux's (Clippy lints only the code rustc compiles for the host). For a change already on main, the bump
workflow's `resnapshot` input does this in CI. The sandbox stages the whole derived set into `publish/` and hands the
commit over as a git bundle, which is data:

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
the pinned PHP (`scripts/pack-data/runtimes/php/build.sh`) and Composer, for the Rust pack `rustup`; it installs
everything else under `.rerail/pack-data/`. It fails when a derivation changes a tracked file or an authored one;
`--update` (a pin change) keeps the tracked files it changes (a licence record) for the commit instead.

To check a pull request's derived data the same way, run steps 1 and 2 at its head with its `publish/` (step 2's
`fixtures:pull` lays `publish/` over the locked archive): `git status -- packs/<id>/upstream/<tool>/publish` then shows
every file where the derivation differs from what the pull request publishes, and is clean when they agree.

## Publishing pack data

Nobody needs S3 credentials to change pack data. A pull request is a publish request for a tool-pack when it does
either of these:

- it tracks the tool-pack's `packs/<pack>/upstream/<tool>/publish/`, a directory that is absent on main;
- it removes rows from the tool's own `upstream/<tool>/UPSTREAM.md`, the list of the files Rerail wrote
  ([Removing data](#removing-data)).

| Path in the pull request | Becomes |
| --- | --- |
| `publish/<path>` | test data archive entry `<path>` (`publish/fixtures/AL04/x.sql` → `fixtures/AL04/x.sql`) |
| `publish/samples/<sample-id>/<path>`, `publish/samples/LICENSES.md` | samples archive entry `<sample-id>/<path>`, `LICENSES.md` |
| `publish/samples/REUSE.toml` | nothing: licensing for the tracked copy only |

Anything else under `publish/` fails the publish. To make the request, make the data in the pulled layout (cases
written by hand, the sample builders, or derive once it resumes), then `npm run fixtures:stage -- --pack <id> --tool
<tool>` copies every file the locked archive lacks or holds differently into `publish/` (`--all`: the whole set, for a
`replace` publish; an overlay cannot delete a file). Commit `publish/` and open the pull request. Its CI lays
`publish/` over the pulled data and judges it, and structure metric #3 blocks the merge while any `publish/` is tracked.

### The publish workflow

Two workflows carry a request. Neither runs any code of the pull request.

1. **The request comment.** `pack-data-publish-request.yml` runs on `pull_request_target`, so for forks too, with the
   base branch's workflow: it checks out nothing and holds only a token that writes pull request comments. While the
   pull request has anything to publish, it keeps one sticky comment, "Pack data waiting to publish": a row per
   tool-pack with its files in `publish/` and its authored rows removed from `UPSTREAM.md`, and the publish steps by
   hand and with the `gh` CLI. It deletes that comment, while it is still waiting, once nothing is left to publish.
   The comment blocks nothing; structure metric #3 does.
2. **The dispatch.** A maintainer, or an agent with write access, dispatches **Publish pack data**
   (`pack-data-publish.yml`) on main with the pull request's number. With `replace` off (the default) each tool-pack's
   archive is the locked one, less the files of removed authored rows, with `publish/` laid over it; with `replace`
   on it is exactly what `publish/` holds. The run is titled "Publish pack data for #<n>".

   ```sh
   gh workflow run pack-data-publish.yml -R neralabs/rerail -f pr=<n> [-f replace=true]
   ```

3. **prep** runs only on main, with main's code and the read role. It refuses a pull request that is not open, a head
   that is no commit on a valid branch, and, for a branch of this repository, a head on the default branch or a
   protected one. It fetches `refs/pull/<n>/head` as objects, which must be the head GitHub names, and packs every
   `publish/` from those git blobs (`scripts/s3/pack-fixtures.mjs pack --from`), never checking the pull request out.
   Staged samples go through `publishedSamples` ([Sample licences](#sample-licences)). Every file going to S3 must be
   under a licence the allowlist takes: test data as pack check 6 decides it, through the same classifier
   ([pack-format.md, "Licensing"](pack-format.md#licensing)), samples by their recorded licence. A refusal fails prep,
   and no approval is asked. prep then writes the approver's summary: per archive its kind, tool-pack, sha256, files,
   bytes, its licences with their file counts and a Sources column (a samples archive's repositories with their
   licences; for test data, the `licenses/*/UPSTREAM.md` records), then what each archive adds, changes and drops (up
   to 50 paths per list; a replace publish's dropped files appear nowhere else), and how many `publish/` files the
   commit will delete.
4. **upload** waits for the `test-data-upload` deployment, whose required reviewer is a code owner who has read the
   summary ([REVIEW.md](../REVIEW.md)). It fails, putting nothing, when no approval is on record: an environment
   without a required reviewer would gate nothing. Only then does it assume the write role. It verifies each archive
   against prep's job outputs (not the artifact's own manifest), puts it create-only (S3 refuses a key it already
   holds), tagged `rerail:commit` (the pull request's head), `rerail:pr`, `rerail:run-id` and `rerail:uploader` (the
   approver), and then confirms every object, even after a failed put: S3's head, and a fresh download verified
   against its lock.
5. **finish** runs only once every object is confirmed. It builds one commit on the pull request's head with git
   plumbing alone (no checkout, no hooks): the new `test-data.lock` and `samples.lock` files written, and every
   `publish/` of the published tool-packs deleted. It pushes it as the `neralabs-ci` app, with the app's token so CI
   runs again, as a plain push that GitHub accepts only as a fast-forward of the head prep packed; it never forces. The
   comment then reads "published".

When finish cannot push, the run is red and the comment reads "action needed", with each lock's text and the reason:
the branch is on a fork ([Forks](#forks)), or someone pushed to it since prep. Commit the locks, delete `publish/` and
push, or pull and dispatch again. When an object was not put and confirmed (a failed put or confirm, a rejected
approval, a cancelled job), the **failed** job names each object's failure in the comment; nothing is committed and
no lock changes. Re-running is safe: puts are create-only and content-addressed, and confirm accepts an object S3
already holds once it verifies.

Once the locks are committed, **Pack test data is in S3** passes and the conformance check runs on the published data
([CI](#ci)).

### Forks

prep reads a fork's head through `refs/pull/<n>/head`, so a fork's data is packed, approved and uploaded like any
other, but finish cannot push to a fork's branch: the comment carries the locks, and the contributor commits them and
deletes `publish/`. CI's jobs that read S3, the conformance check among them, run only for pushes and same-repository
pull requests ([CI](#ci)), so a fork's data is judged by the approver and, after merge, by main's CI.

### Removing data

A file the archive holds is not in the pull request's diff, so an overlay cannot delete it. An authored file goes with
its row: the pull request deletes the file's row from the tool's `UPSTREAM.md`, and an overlay publish of that
tool-pack drops it. The removed row alone makes the tool-pack a publish request, with nothing staged in `publish/`:
the request comment counts it, and prep drops every authored file the record listed at the merge base with main and no
longer lists. Anything else goes with a `replace` publish of the whole set (`fixtures:stage --all`). The old archive
stays in S3 for the commits that name it.

### `fixtures:push`

`npm run fixtures:push -- --pack <id> --tool <tool>` (`--dry-run` shows what it would archive) archives a tool-pack's
data as it is on disk, uploads it under the caller's own write credentials, tagged with their `gh` login, and rewrites
the lock; `node scripts/s3/pack-fixtures.mjs samples push` does the same for a samples archive already in the local
cache. Neither runs prep's licence check or asks a code owner's approval, so a pull request's data goes through the
workflow. The commands remain for a maintainer who holds write credentials and must put an archive in S3 outside a
pull request, such as one a lock already names that S3 lacks, the case `fixtures:pull` and
**Pack test data is in S3** name it for.

## Replay a commit

Every commit names its data by sha256, and S3 keeps every object ever published (writes are create-only). To see
what a commit's rules were judged on, check it out and run `npm run fixtures:pull`: it fetches the archives its locks
name, verifies them, and extracts them as that commit's CI did.

A CI run also lists the archives it used. Each `.github/actions/pack-test-data` step sets an `archives` output, one
`<pack> <tool> <kind> <sha256>` line per archive (kind `test-data` or `samples`), and its pull adds the same lines as a
table to the job's summary; a fetching job that hits the cache runs no pull, so its output lists the locks its cache key
names. To replay a run from that output, check out the run's commit and `npm run fixtures:pull`: its locks name those
sha256s. To fetch one archive alone, take the key `rerail/archives/<pack>/<tool>/<sha256>.tar.zst` (a samples archive:
`rerail/archives/samples/<pack>/<tool>/<sha256>.tar.zst`) from `s3://nera-test-fixtures/`; the sha256 is its tar's, so
check it after `zstd -d`.

## Derive and bumps are paused

Pack test data derivation and upstream bumps are paused until the generators write per-repo provenance. Each
`packs/<id>/licenses/<project>/UPSTREAM.md` records its repository, not each file
([pack-format.md, "UPSTREAM.md"](pack-format.md#upstreammd)), and the per-file record is `provenance.json` in each
test data archive, for audit. The generators still write per-file rows: an ECMA tool-pack's generator (javascript,
typescript) would fail, and every other one would rewrite `UPSTREAM.md` back to per-file rows, which pack check 6
rejects. So every path that derives refuses, with the message `derive and bumps are paused until the generators write
per-repo provenance; see docs/upstream.md#derive-and-bumps-are-paused`:

- `npm run fixtures:derive` refuses every tool-pack before it removes a file or runs a generator, and exits 1
  (`scripts/pack-data/derive.ts`, `scripts/pack-data/paused.ts`);
- `npm run upstream:bump` only plans a bump (`--dry-run`); asked to apply one, it refuses, writes nothing and exits 1.
  Its summary's "Regenerate locally" section points here instead of giving derive commands;
- `upstream-bump.yml`, dispatched by hand, bumps and re-snapshots (`resnapshot`) alike, fails at its `scope` job's first
  step, so no other job starts;
- `upstream-releases.yml`'s `bump-dispatch` job does not start bumps (`if: false && ...`). The release check still runs
  and reports which tool-packs are behind.

### What to do meanwhile

The locked data stands, and everything that does not need a generator goes ahead as usual:

- rule changes, scored on the locked data by the conformance check;
- samples ([Samples](#samples));
- a code corpus with its ground truth under `upstream/<tool>/authored/`, staged and published;
- removing an authored file by removing its row ([Removing data](#removing-data)).

Anything a generator makes waits: new upstream cases, expected findings (including those on the cases Rerail wrote,
which the generator records by running the tool), a pin, generator or recipe change, and a new ported tool-pack's
data. Running a pack's generator directly is no way round the pause, since it writes the per-file rows check 6
rejects. Open the pull request or issue without the data and say it waits on the pause.

### What resumes

The generators are changed to write per-repo provenance: `licenses/<project>/UPSTREAM.md` records with patterns, the
tool's own `UPSTREAM.md` listing what Rerail wrote, and each file's provenance in the archive's `provenance.json`. Then
`refuseWhilePaused` is dropped from `deriveToolPack`, the CLI's refusal and the summary's pointer from
`scripts/upstream/bump.ts`, the refusing step from `upstream-bump.yml` and the leading `false &&` from `bump-dispatch`.
Local derives, re-snapshots and the nightly bumps then run as [Take a new snapshot](#take-a-new-snapshot) describes,
and this section goes.

## Upstream releases

A pin says which release the rules were ported against; `upstream-releases.yml` notices when upstream publishes a newer
one. Once a day on main it runs:

```sh
npm run upstream:releases -- --behind behind.json   # scripts/upstream/releases.ts
```

For every gated tool-pack it reads the release `gates.yaml` pins (`upstream.version`) and the one its runtime installs
(`scripts/pack-data/runtimes/<upstream.runtime>/package.json`, `pyproject.toml` or `composer.json`). The package is the
one the runtime installs upstream as: an ESLint-family upstream's comes from `eslint.release.installed`, and every other
adapter's is listed in `ADAPTER_PACKAGES`, since an upstream's repository is not always the name it publishes under
(PHPCSStandards/PHP_CodeSniffer is `squizlabs/php_codesniffer`, tighten/tlint is `tightenco/tlint`). The Rust pack's
Clippy ships with Rust, so its release is a toolchain's: `gates.yaml` pins its version and commit and names the
`clippy-<release>` runtime, whose `rust-toolchain.toml` pins the channel; `packs/rust/upstream/common.py` refuses any
toolchain other than `TOOLCHAIN`, and `conformance.json` names Clippy as `0.1.<minor>`. None of them is the repository's
`rust-toolchain.toml`, which pins the toolchain Rerail is built with.

It then asks each registry for the latest stable release: npm's `latest` dist-tag, the highest final PyPI release with
a file that is not yanked, the highest Packagist version without a `dev`, alpha, beta or RC suffix, and `[pkg.rust]` in
Rust's stable channel manifest. Versions compare by their numeric components, a leading `v` dropped, and the first
component that differs is the gap: `patch`, `minor` or `major`, and `breaking-minor` for a minor release ahead of a
`0.x` pin. Rust and Clippy compare on major.minor only: a Rust patch release is not behind. The job prints one table
row per tool-pack (`Tool-pack`, `Ecosystem`, `Package`, `Pinned`, `Latest stable`, `Gap`, `Status`) into the job
summary and annotates each tool-pack behind, pins that disagree or a registry it cannot read with a warning. It never
fails on what upstream published: the bump is the fix. `--behind <file>` writes every tool-pack behind, breaking or not,
as JSON (`{pack, tool, toolPack, pinned, latest, gap, breaking}`), and its `bump-dispatch` job starts the night's bump
with that list.

### Bump pull requests

An upstream release is a pin change plus the test data derived from the new pins. Bumps are paused for now
([Derive and bumps are paused](#derive-and-bumps-are-paused)); this section describes them as they run once the pause
lifts. Each night `bump-dispatch` points the work branch `auto/bump-run/upstream` at main's commit and dispatches
`upstream-bump.yml` there once, with every tool-pack behind (`[{pack, tool, version}]`, sorted by pack then tool),
leaving out each `<pack>/<tool>@<version>` a closed, unmerged bump pull request already declined.

```sh
npm run upstream:bump -- --pack python --tool ruff [--version 0.17.1] [--dry-run]  # scripts/upstream/bump.ts
```

The bump moves every pin of the release together, and reads each file back to check that only the pins moved:

- `gates.yaml`: `upstream.version`, and `upstream.commit` as the commit the release's GitHub tag names (an annotated tag
  is followed to its commit), plus an ESLint upstream's `eslint.release.tag`. A release without a tag of the pinned
  form stops the bump.
- The runtime's pin file: upstream's package, and on npm every package of its scope pinned at the same release. A
  runtime directory named `<name>-<version>` is renamed for the new release, and every `gates.yaml` naming it follows.
- `conformance.json`'s `tools[].version`.
- The generators' pins (`packs/<pack>/upstream/<tool>/*` and the pack's shared `upstream/*` scripts): the commit, npm's
  `gitHead` where a generator pins that instead, a `...VERSION = "<release>"` and `...TAG = "<tag>"` constant, and an
  entry keyed by upstream's package.
- The reference tools a pack pins by hand (`fixtures/vue/tools.mjs`, `fixtures/sql/common.py`): upstream's package
  entry, and the commit and the tag beside it.
- `packs/<pack>/licenses/<project>/UPSTREAM.md`: its Source row's tag and commit, and the release in its heading. A row
  with no tag (`—`) naming npm's `gitHead`, as react-hooks' does, moves to the new release's `gitHead` and keeps `—`.
- Clippy: `TOOLCHAIN`, `RUSTC_VERSION`, `CLIPPY_VERSION` and `CLIPPY_COMMIT` in `packs/rust/upstream/common.py`, and
  `licenses/clippy/UPSTREAM.md`'s Source row and the Rust release in its heading.

Whatever still names the old release afterwards (prose in a README, a comment, the rest of `UPSTREAM.md`, a derive
recipe under `scripts/pack-data/recipes/`, or a renamed runtime's old name) is listed in the summary for the reviewer.

The bump does not touch `gates.yaml`'s `rules`: the pull request shows them, and the reviewer checks them against the
new release (a rule deprecated, added to a recommended configuration, or gone).

Deriving runs upstream's code at a release nobody has reviewed, with unlocked transitive packages and Composer. So it
never runs in main's cache scope or next to a credential, and the work is split over three workflows:

1. **`upstream-releases.yml`** (`bump-dispatch`) runs no code from the tree beyond the release check, and dispatches
   the bump on the work branch.
2. **`upstream-bump.yml`**, the producer, runs on the work branch and refuses any other ref. Its `scope` job empties
   the branch's cache scope; its `test-data` job pulls every tool-pack's archive under the read role with main's code;
   its `bump` legs, one per listed tool-pack and each failing alone, hold no OIDC token. Each moves its tool-pack's pins
   (`upstream:bump`), derives on Linux every tool-pack the bump made stale (`fixtures:derive -- --update`), stages each
   one's whole derived set into its `publish/` (`fixtures:stage --all`), and uploads the patch, the bump's summary and
   `bump.json` as the `bump-<pack>--<tool>` artifact. With the `resnapshot` input, each listed `version` must be the
   release already pinned, and the tool-pack is derived again at it; a maintainer starts one by pointing
   `auto/bump-run/upstream` at main's head and dispatching the workflow there.
3. **`upstream-bump-publish.yml`** runs on main when a bump run ends, with main's code, treating every artifact as
   data. `run` checks the bump run (its branch, age, and that main's code has not changed since its commit). `verify`,
   one leg per artifact, runs the pin edits again with main's code and admits the patch only if it makes exactly those
   edits plus the files deriving and staging write, and changes no lock. `compose` turns the verified patches into one
   commit per tool-pack, `Bump <pack>/<tool> to <version>`, on main's head, and builds the pull request's body (pin
   diffs, upstream facts, what still names the old release, each leg's own report marked unverified). `pull-request`
   pushes the commits to `auto/bump-upstream` (`auto/resnapshot-upstream` for a re-snapshot) with the app's token, opens
   or updates the pull request, and dispatches `pack-data-publish.yml` for it with `replace`, so its data reaches S3 by
   the one publish path, after a code owner approves. `work-branch` then deletes the work branch.

Nothing merges itself. Closing a bump pull request without merging it declines exactly the bumps it carried: the
nightlies leave each of those `<pack>/<tool>@<version>` out until a newer release appears. A bump that cannot be made
(pins that already disagree, a release without a tag) fails its leg with the reason, and the other tool-packs are
still proposed.

When one tool-pack fails the conformance check on the pull request, merge the others without its commit. Each commit is
one tool-pack's bump, so dropping one leaves the others as they are. From a branch of your own:

```sh
git fetch origin main auto/bump-upstream
git switch -c bump-upstream-<date> origin/auto/bump-upstream
git rebase --onto <its commit>~1 <its commit>   # drop that tool-pack's commit
git push -u origin bump-upstream-<date>
```

then open a pull request from it and close the nightly's: closing declines the dropped release until upstream
publishes a newer one. A branch built from the bump's commits before its publish still carries `publish/`; dispatch
the publish for the new pull request. To keep the dropped release instead, bump it by hand (`npm run upstream:bump`,
then derive, stage and publish).

## CI

`ci.yml`'s jobs that touch pack data:

- **Pack test data** (`test-data`) fills the Actions cache with every archive the locks name, through
  `.github/actions/pack-test-data` with `fetch: "true"`: it looks the cache key (from the locked sha256s alone) up
  without restoring it and, on a miss, downloads the archives from S3 under the read role's OIDC token; it runs
  nothing but this commit's `scripts/s3/pack-fixtures.mjs`. The jobs that read the data hold no token: they restore
  that entry with `fetch: "false"`, verify each archive against its lock, extract it, and fail on a miss. So a job that
  can mint a token never extracts a cache entry, and a job that runs upstream's code (the bump) has no token to mint.
- **Pack test data is in S3** (`test-data-in-s3`) runs `node scripts/s3/pack-fixtures.mjs check` under the read role: a
  `head-object` per archive every `test-data.lock` and `samples.lock` names, downloading nothing, failing with each
  missing key, its lock and its remedy. A pull request cannot merge with data S3 lacks.
- **Scripts, reference examples, conformance, and rule playground** (`tools`) runs the conformance check
  (`conformance:languages -- --check`) over every pack, with `publish/` laid over the pulled data, and uploads its
  summary and pull request comment as the `conformance-summary` artifact, which the `conformance-report` job posts on
  the pull request ([conformance/README.md](../conformance/README.md#what-runs-on-a-pull-request)).
- **Lint and type-check** (`lint`) runs structure metric #3, which fails while any `publish/` is tracked, a tool
  directory lacks its `test-data.lock`, a lock is malformed, or pack test data is tracked in the pulled layout
  ([repository-structure.md](repository-structure.md#3-fixture-data-outside-fixtures-and-packsupstream-pack-test-data-in-git-or-unpinned-or-pack-data-waiting-to-publish--target-0)).
- **CI result** (`ci-result`) needs every other job and fails when any failed or was cancelled; it is the check the
  ruleset requires, since GitHub counts a job skipped after a failed dependency as passing.

CI verifies the data by sha256 only: it never derives it again or asks upstream. Pack test data is a point-in-time
artifact, and no CI job checks that it reproduces from the pins; the approving code owner checks that what a pull
request publishes follows from it, and the conformance check judges the rules on it.

Forks and Dependabot get no OIDC token, so `test-data`, `test-data-in-s3`, `tools` (and with it the conformance
comment) and the other jobs that read pack data run only for pushes and same-repository pull requests. On a fork's
pull request, structure metric #3 is what holds `publish/` back from merging.
