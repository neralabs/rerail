# Contributing to Rerail

Thank you for helping. A few rules keep the project open source and its licensing clean.

## Getting the code

Install [Git LFS](https://git-lfs.com) before you clone, and run `git lfs install` once. The embedding model's weights
(`model/model.safetensors`) and the engine's Unicode names table (`native/src/code_rule_dsl/value/names/names.bin`,
compiled into the engine) are stored in LFS (`.gitattributes`); without it a checkout holds pointer files, and neither
the engine build nor the model works. In a checkout made without it, run `git lfs pull`. Then `npm ci`,
`npm run build` and, for the pack tests, `npm run fixtures:pull` and `npm run fixtures:derive`
([Pack test data](#pack-test-data)).

Generated files are listed, with the command that rewrites each, in
[scripts/generate/README.md](scripts/generate/README.md); edit the generator, not its output.

## Pack test data

A rule pack's test data is not in git. Under each `packs/<id>/upstream/<tool>/` it is the tool's test cases
(`fixtures/`, `tests/`, the case lists `cases.json` and `*.cases.json`, `fixtures.upstream-check.json`, the
complexity pack's `lizard-*.tsv`), the tool's recorded answers on them (`*.expected.json`), and `authored/`. The
patterns are `PACK_FIXTURE_PATTERNS` in [scripts/s3/pack-fixtures.mjs](scripts/s3/pack-fixtures.mjs); everything
else under `upstream/<tool>/` (`gates.yaml`, the generators and recorders, `UPSTREAM.md`, `goldens.tsv`,
`recording.lock`) stays in git. The data is of two kinds:

- **Authored**: what Rerail wrote for the tool to score, which nothing can regenerate: the files the tool's own
  `upstream/<tool>/UPSTREAM.md` lists as Rerail's (pack check 6 reads the same list), and everything under
  `upstream/<tool>/authored/`, the place for a contributor's code corpus and its ground truth when the corpus is no
  public repository a pin can name. It lives in S3: each file's key is `rerail/` and its repository path in the bucket
  `s3://nera-test-fixtures` (us-east-2).
- **Derived**: everything else, upstream's tests copied or derived and the tool's recorded answers on them.
  `npm run fixtures:derive` makes it from the tool-pack's pins, running the pack's generator and then writing the case
  sources the port readiness recorder names (recipes in [scripts/pack-data/recipes/](scripts/pack-data/recipes/)). It
  is never committed and never uploaded: CI derives it and keeps it in its Actions cache, keyed by every input of the
  derivation (`.github/workflows/pack-test-data.yml`).

Fetch the authored part once after cloning, and again whenever a merged change touched it; derive the rest after
either, and after a change to a pin or a generator:

```sh
aws sso login                # or any AWS credentials with read access to s3://nera-test-fixtures/rerail/
npm run fixtures:pull        # the authored files, into packs/; add -- --dry-run to see what it would fetch
npm run fixtures:derive      # every tool-pack; -- --pack <id> [--tool <tool>] for some
```

`fixtures:pull` needs the [AWS CLI v2](https://aws.amazon.com/cli/) and read access; ask a maintainer for access.
`fixtures:derive` needs the network (the pinned upstreams and their packages), `uv`, and for the PHP and Blade packs
the pinned PHP (`conformance/recorders/port-readiness/runtimes/php/build.sh`) and Composer, for the Rust pack
`rustup`; it installs everything else under `.rerail/pack-data/`. It fails when a derivation changes a tracked file
or an authored one; `--compare <dir>` compares the result with another copy of the data byte for byte. Clippy's
answers are Linux's in CI (`#[cfg(target_os = "linux")]` code in the tokio corpus): derived on macOS they differ
there. `.gitignore` covers the patterns, so neither kind shows in `git status`. Files your branch tracks win: a
pull puts each tracked file back to its committed content (or keeps your working-tree edit of it). Without the data,
the pack tests (`npm test`, `npm run test:conformance`, `cargo test`) and the recorders fail with an instruction to
pull and derive.

An upstream release is a pin change and nothing else: change the pins (the generator's constants, `gates.yaml`, the
runtime under `conformance/recorders/port-readiness/runtimes/`), run `npm run fixtures:derive -- --pack <id> --tool
<tool> --update`, which rewrites the derived data and keeps the tracked files it changes (a licence table) for the
commit, and record the tool-pack again (`npm run port-readiness:record`). Nothing is uploaded; CI derives the new
data on the pull request.

### Adding or changing a pack

Check the authored test data you add or change into your pull request, so review sees it and CI runs on it: the
cases you wrote for the tool (listed in its `UPSTREAM.md`), or a code corpus with its ground truth under
`upstream/<tool>/authored/`. Never check in derived data: a pull request that changes upstream's tests or answers
changes the pins or the generator that makes them, and CI derives them (structure metric #3 fails on any tracked
derived file). The generators and recorders print the command that adds the authored files they wrote. Because
`.gitignore` covers them, add them with `git add -f`, e.g. `git add -f --
packs/<id>/upstream/<tool>/fixtures/rerail`. The usual rules for third-party material apply (below).

### Merging (maintainers)

Before merging such a pull request, a maintainer moves its authored test data to S3 and deletes it from the pull
request:

```sh
npm run fixtures:push -- --dry-run   # what it would upload
npm run fixtures:push                # upload, verify by downloading back, then git rm --cached
git commit -m "Move pack test data to S3"
```

`fixtures:push` runs with the maintainer's own AWS credentials (write access to the bucket). It refuses a dirty index,
tracked pack test data edited in the working tree, and tracked derived data; it uploads every tracked authored file to
its key, downloads each back and compares bytes, and only then removes them from the index, printing each, and leaves
the files on disk. It never deletes from S3: a pull request that removes authored data needs the maintainer to delete
those keys (`aws s3 rm`); the bucket is versioned, so nothing is lost. Structure metric #3 (`npm run check`) fails while
any authored data is tracked, so a pull request cannot merge before the push.

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
