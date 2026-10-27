# Upstream snapshots and publishing data

A pack's ported rules are judged against upstream: a linter at the release its tool-pack pins. Upstream is
provenance at a point in time. What it said is captured once, put in S3 and pinned by sha256 in a tracked lock, and
from then on CI only verifies the pinned bytes and scores the rules on them. CI never runs upstream, and never checks
whether upstream would still answer the same: the snapshot goes stale only when a maintainer decides to take a new one.

| Artifact | Lock | Made by |
| --- | --- | --- |
| Pack test data: upstream's tests, the expected findings (`fixtures.expected.json`), Rerail's authored cases | `packs/<id>/upstream/<tool>/test-data.lock` | the generators, through `npm run fixtures:derive` (a maintainer, or the bump workflow) |
| Real-code samples | `packs/<id>/upstream/<tool>/samples.lock` | the sample tooling (`scripts/samples/`) |

Both are published the same way ([Publishing pack data](#publishing-pack-data)): the data goes into the pull request
under the tool-pack's `publish/` directory, and a workflow on main puts it in S3 once a code owner approves.

What the rules must pass is the conformance check (`npm run test:conformance`), described with each of its checks in
[conformance/README.md](../conformance/README.md#the-gate). Each gated tool-pack's `gates.yaml` is its upstream
manifest: the pins (`upstream.version`, `upstream.commit`, the runtime), the file types upstream lints
(`upstream.lints`), and the per-rule facts declared at the pins (`rules`), which a maintainer reviews at each bump.

## Samples

A sample is a real file from an open-source repository, labelled for the pack's rules: per rule, the lines the rule
should report, or `[]` when the file is clean for it; a rule the labels do not name is not judged on the file. Labels
are line-only (a column, when present, is provenance). The format is `scripts/samples/format.ts`: each sample is
`<sample-id>/<source path>` beside its `<source path>.expected.json` (the source repository, commit, path, sha256 and
SPDX licence, and the labels), with `LICENSES.md` at the archive's root. No copyleft source is included.

A tool-pack's samples are one canonical archive in S3,
`s3://nera-test-fixtures/rerail/archives/samples/<pack>/<tool>/<sha256>.tar.zst`, written once, which the tracked
`samples.lock` names by its tar's sha256, file count and bytes, in `test-data.lock`'s format. `SAMPLES.md` beside it
lists the repositories, licences, commits and file counts. `npm run fixtures:pull` fetches and extracts them with the
test data (`node scripts/s3/pack-fixtures.mjs samples` alone), into `.rerail/pack-data/samples/<pack>/<tool>/<sha256>/`,
and CI's pack-test-data action carries them in the same cache entry. Samples staged under the tool-pack's
`publish/samples/` are laid over the locked set, so a pull request's CI judges them before they are published. To add,
relabel or drop samples, or to rename or drop a rule key (a label naming no rule of the tool-pack fails its whole
pack's conformance check, which catches typos), the sample tooling (`scripts/samples/pack.ts`) writes the set into
`publish/samples/`, and the set is published like any pack data.

The conformance runner judges each rule by intent, on the samples labelled for it:

- a report on a labelled line is a true positive, and a labelled line with no report is a miss;
- any report on a sample clean for the rule is a false positive;
- an extra report on a sample with labels for the rule is listed for review and fails nothing;
- a divergence with `"source": "samples"`, matched by the sample's path and line, explains a miss or a false positive.

Three checks read it. `samples-catch-labelled` fails a shipped rule on an unexplained miss, `samples-silent-on-clean`
on an unexplained false positive, and `engine-reads-samples` on an engine failure on a judged sample (a rule runtime
error, a crash, a timeout, a work budget, an unreadable file) or one naming no file. A sample's context files (the
files beside it that give it manifest facts, configuration and imports) are never judged: an engine failure there is
listed in the report (`contextFailures`, the CLI's `ctx` column) and fails nothing. A parse failure on a judged sample
(a syntax error, `source-partial`, or a `source-incomplete` the engine reports because this or another file was read
with errors, which the runner tells from a read failure by its message: `isParseIncomplete` in
`conformance/languages/runner.ts`) is report-only: `engine-reads-samples` lists it ("N engine failure(s); M file(s)
read in part") without failing, and the sample stays judged, so a label the partial parse makes the rule miss still
fails as a miss.

`stays-silent` needs relevant evidence that the rule is silent where it should be: one of the rule's own upstream
units (an original unit whose `source.units` origin names the rule's upstream rule) where upstream reports nothing and
the rule agrees, or one clean sample the rule is silent on. Other rules' units, units of unknown origin and Rerail's
probes do not count. Its detail reads "N of M own upstream unit(s), K clean sample(s)". A tool-pack whose test data
names no unit origins rests on its clean samples alone. Candidates are scored and reported but fail nothing.

## Take a new snapshot

A snapshot changes only on purpose, in one of three ways, each ending in a pull request that carries the new data in
`publish/`:

- **A bump** moves the pins to a newer upstream release and derives the data again
  ([Bump pull requests](#bump-pull-requests)); the nightly starts one for every tool-pack behind.
- **A re-snapshot** derives the data again at the release already pinned: a generator or recipe change, a runtime
  pin's, or data only Linux makes. It is the bump workflow's `resnapshot` input.
- **A local derive**: `npm run fixtures:derive -- --pack <id> --tool <tool> [--update]` (or the pack's generator), then
  `npm run fixtures:stage -- --pack <id> --tool <tool> [--all]`, and the publish
  ([CONTRIBUTING.md, "Pack test data"](../CONTRIBUTING.md#pack-test-data)).

The generators run upstream's code, so they run on the authoring path only: a maintainer's machine or the bump
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
own (`gates.yaml`'s `upstream.runtime`) and the adapter's (`parsers` for ESLint, `python` for Ruff and Bandit, `php` for
PHP_CodeSniffer, PHPStan and Psalm, `blade` and `php` for tlint and stillat/blade-parser). Each runtime is pinned by its
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

## Publishing pack data

Nobody needs S3 credentials to change pack data. A tool-pack's `packs/<pack>/upstream/<tool>/publish/` is a tracked
directory, absent on main, and anything in it is a publish request:

| Path in the pull request | Becomes |
| --- | --- |
| `publish/<path>` | test data archive entry `<path>` (`publish/fixtures/AL04/x.sql` → `fixtures/AL04/x.sql`) |
| `publish/samples/<sample-id>/<path>`, `publish/samples/LICENSES.md` | samples archive entry `<sample-id>/<path>`, `LICENSES.md` |
| `publish/samples/REUSE.toml` | nothing: licensing for the tracked copy only |

1. Make the data in the pulled layout (derive, hand-written cases, or the sample tooling, which writes
   `publish/samples/` itself), then `npm run fixtures:stage -- --pack <id> --tool <tool>` copies every file the locked
   archive lacks or holds differently into `publish/` (`--all`: the whole set, for a replace publish; an overlay cannot
   delete a file).
2. `git add` the `publish/` directory, push and open the pull request. CI lays `publish/` over the pulled data and
   judges it; `pack-data-publish-request.yml` keeps a sticky comment with the publish steps, by hand and with the gh
   CLI; structure metric #3 blocks the merge while any `publish/` is tracked.
3. A maintainer (or an agent with write access) dispatches `pack-data-publish.yml` on main with the pull request's
   number, and `replace` on to make each tool-pack's archive exactly what `publish/` holds (off: lay `publish/` over the
   locked archive).
4. **prep** packs every `publish/` from the pull request head's blobs with main's code
   (`scripts/s3/pack-fixtures.mjs pack --from`), runs nothing of the pull request, checks every file's source licence
   against `scripts/check/licence-allowlist.json`, and writes the summary: per archive its sha256, files, bytes and
   what was added, changed and dropped.
5. A code owner reads that summary and approves the `test-data-upload` deployment.
6. **upload** re-verifies the archives against prep's outputs, puts each create-only under the write role
   (`s3://nera-test-fixtures/rerail/archives/…`, tagged with the commit, pull request, run and approver), then confirms
   every object the new locks name by downloading and verifying it again.
7. **finish**, only once every object is confirmed, builds one commit with git plumbing (the new locks, `publish/`
   removed) and pushes it to the pull request's branch with the `neralabs-ci` app's token, never forcing, so CI runs
   again on it. On a fork, or when the branch moved and the push is rejected, the sticky comment carries each lock's
   text to commit by hand instead. A failed or unapproved upload commits nothing; re-running is safe.

A maintainer who holds credentials may still push with `npm run fixtures:push`; the workflow is the normal path.

## Replay a commit

Every commit names its data by sha256, and S3 keeps every object ever published (writes are create-only). To see
what a commit's rules were judged on, check it out and run `npm run fixtures:pull`: it fetches the archives its locks
name, verifies them, and extracts them as that commit's CI did. Commits from before the snapshot model also name
recordings under `rerail/recordings/`, which stay in S3.

## Derive and bumps are paused

Pack test data derivation and upstream bumps are paused until the generators write per-repo provenance (a follow-up
to the per-repo provenance change). Each `packs/<id>/licenses/<project>/UPSTREAM.md` now records its repository per
repo: the Source row, the pack-path patterns of the files taken from it, and the `Not copied` lines. The per-file
record is no longer in git: each tool-pack's test data archive holds `upstream/<tool>/provenance.json` (every file, its
upstream path and how it was made, a snapshot taken when the tables moved) and `upstream/<tool>/LICENSES.md` (the
licence texts), for audit. Nothing in the repository reads `provenance.json`; pack check 6, the publish licence check
and structure metric 7 work from the tracked patterns.

The generators and `fixtures:derive` still write per-file rows, so until the follow-up:

- an ECMA tool-pack's derive (javascript, typescript) fails, since its rename variants look up their originals' rows;
- every other row-writing derive, run with `--update`, rewrites `UPSTREAM.md` back to per-file rows and must not be
  committed;
- `upstream-releases.yml`'s `bump-dispatch` job does not start bumps. The release check still runs and reports which
  tool-packs are behind.

Do not derive, re-snapshot or bump a tool-pack until the follow-up lands.

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

An upstream release is a pin change plus the test data derived from the new pins. Each night `bump-dispatch` points
the work branch `auto/bump-run/upstream` at main's commit and dispatches `upstream-bump.yml` there once, with every
tool-pack behind (`[{pack, tool, version}]`, sorted by pack then tool), leaving out each `<pack>/<tool>@<version>` a
closed, unmerged bump pull request already declined.

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
  `gitHead` where a generator pins that instead, a `...VERSION = "<release>"` constant, and an entry keyed by upstream's
  package.
- Clippy: `TOOLCHAIN`, `RUSTC_VERSION`, `CLIPPY_VERSION` and `CLIPPY_COMMIT` in `packs/rust/upstream/common.py`.

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
  `.github/actions/pack-test-data` (`fetch: "true"`, from S3 under the read role's OIDC token on a miss). The jobs that
  read the data hold no token and restore it (`fetch: "false"`).
- **Pack test data is in S3** (`test-data-in-s3`) runs `node scripts/s3/pack-fixtures.mjs check` under the read role: a
  `head-object` per archive every `test-data.lock` and `samples.lock` names, downloading nothing. A pull request cannot
  merge with data S3 lacks.
- **Scripts, reference examples, conformance, and rule playground** (`tools`) runs the conformance check
  (`conformance:languages -- --check`) over every pack, with `publish/` laid over the pulled data, and uploads its
  summary and pull request comment as the `conformance-summary` artifact, which the `conformance-report` job posts on
  the pull request. This is the gate
  ([conformance/README.md](../conformance/README.md#the-gate)).
- **Lint and type-check** (`lint`) runs structure metric #3, which fails while any `publish/` is tracked or pack test
  data is tracked in the pulled layout.
- **CI result** (`ci-result`) needs every other job and fails when any failed or was cancelled; it is the check the
  ruleset requires.

Forks and Dependabot get no OIDC token, so the jobs that pull from S3 run for pushes and same-repository pull
requests.
