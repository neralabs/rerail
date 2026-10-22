# Contributing to Rerail

Thank you for helping. A few rules keep the project open source and its licensing clean.

## Getting the code

Install [Git LFS](https://git-lfs.com) before you clone, and run `git lfs install` once. The embedding model's weights
(`model/model.safetensors`) and the engine's Unicode names table (`native/src/code_rule_dsl/value/names/names.bin`,
compiled into the engine) are stored in LFS (`.gitattributes`); without it a checkout holds pointer files, and neither
the engine build nor the model works. In a checkout made without it, run `git lfs pull`. Then `npm ci`,
`npm run build` and, for the pack tests, `npm run fixtures:pull` ([Pack test data](#pack-test-data)).

Generated files are listed, with the command that rewrites each, in
[scripts/generate/README.md](scripts/generate/README.md); edit the generator, not its output.

## Pack test data

A rule pack's test data is not in git. Under each `packs/<id>/upstream/<tool>/` it is the tool's test cases
(`fixtures/`, `tests/`, the case lists `cases.json` and `*.cases.json`, `fixtures.upstream-check.json`, the
complexity pack's `lizard-*.tsv`), the tool's recorded answers on them (`*.expected.json`), and `authored/`. The
patterns are `PACK_FIXTURE_PATTERNS` in [scripts/s3/pack-fixtures.mjs](scripts/s3/pack-fixtures.mjs); everything
else under `upstream/<tool>/` (`gates.yaml`, the generators and recorders, `UPSTREAM.md`, `goldens.tsv`,
`recording.lock`, `test-data.lock`) stays in git. The data is of two kinds:

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
- puts every recording the commit's `recording.lock` names in S3 as
  `s3://nera-test-fixtures/rerail/recordings/<pack>/<tool>/<sha256>.json`, from
  `.rerail/port-readiness/recordings/<sha256>.json`: a copy whose sha256 is the one the lock names is uploaded by a
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
for `rust/clippy`, whose tokio corpus has `#[cfg(target_os = "linux")]` code. `fixtures:push --from` takes a commit,
not a directory, so the sandbox commits the derived data on top of the pull request and hands the commit over as a git
bundle, which is data; the commit is amended to hold only the locks before anything is pushed:

```sh
# 1. In the trusted worktree: the archive the pull request's test-data.lock names (main's, unless it changes the lock).
npm run fixtures:pull -- --pack <id> --tool <tool>

# 2. In the sandbox, a clone checked out at the pull request, with that archive (the trusted worktree's
#    .rerail/pack-data/archives/<id>/<tool>/<sha256>.tar.zst, the sha256 its test-data.lock names) copied in:
mkdir -p .rerail/pack-data/archives/<id>/<tool>
cp <path to>/<sha256>.tar.zst .rerail/pack-data/archives/<id>/<tool>/
npm ci
npm run fixtures:pull -- --pack <id> --tool <tool>               # extracts the copied archive; reads no S3
npm run fixtures:derive -- --pack <id> --tool <tool> --update    # the derived data, from the pins
npm run port-readiness:record -- --pack <id> --tool <tool>       # a gated tool-pack (gates.yaml) only
git add -f -- packs/<id>/upstream/<tool> packs/<id>/licenses
git commit -m "Derive <id>/<tool>"
git bundle create pr-<n>-derived.bundle HEAD
# Copy out pr-<n>-derived.bundle and .rerail/port-readiness/recordings/*.json.

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
together. That pull request's CI derives each changed tool-pack again and records it again, so neither the archives
nor the answers are taken on the bump run's word. A maintainer reviews and merges it like any pull request; nothing
merges itself. When one tool-pack's gates fail, merge the others without its commit; closing the pull request unmerged
declines the releases it carried until newer ones appear ([docs/port-readiness.md](docs/port-readiness.md#bump-pull-requests)).
The port readiness recordings are not pack test data and not in its archives: each is an object of its own in S3,
written once at
`s3://nera-test-fixtures/rerail/recordings/<pack>/<tool>/<sha256>.json`, which `recording.lock` names by sha256, and
the Actions cache holds copies for CI. `node scripts/s3/pack-fixtures.mjs recordings [--pack <id> [--tool <tool>]]`
fetches the ones the checkout's locks name into `.rerail/port-readiness/recordings/`
([docs/port-readiness.md](docs/port-readiness.md#the-recordings-are-not-in-the-tree)).

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
BSD-3-Clause, ISC, Zlib, 0BSD, CC0-1.0, Unicode-3.0), so no copyleft file can be committed.

Fixtures (`fixtures/`) and rule packs (`packs/`) follow the same rules; see their READMEs.

## Dependencies

New dependencies must be permissively licensed if they end up in anything Rerail distributes:
`npm run check:licenses` fails on a copyleft production dependency. Copyleft is acceptable only in
development-only dependencies and tools, which are never committed or distributed.

## Pull requests

Open a pull request against `main`. Changes are squash-merged after review.
