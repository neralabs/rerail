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

A rule pack's test data lives in S3, not git: under each `packs/<id>/upstream/<tool>/`, the tool's test cases
(`fixtures/`, `tests/`, the case lists `cases.json` and `*.cases.json`, `fixtures.upstream-check.json`, the
complexity pack's `lizard-*.tsv`) and the tool's recorded answers on them (`*.expected.json`). The patterns are
`PACK_FIXTURE_PATTERNS` in [scripts/s3/pack-fixtures.mjs](scripts/s3/pack-fixtures.mjs); everything else under
`upstream/<tool>/` (`gates.yaml`, the recorders, `UPSTREAM.md`, `goldens.tsv`, `recording.lock`) stays in git. Each
file's key is `rerail/` and its repository path in the bucket `s3://nera-test-fixtures` (us-east-2).

Fetch it once after cloning, and again whenever a merged change touched it:

```sh
aws sso login                # or any AWS credentials with read access to s3://nera-test-fixtures/rerail/
npm run fixtures:pull        # aws s3 sync into packs/; add -- --dry-run to see what it would fetch
```

It needs the [AWS CLI v2](https://aws.amazon.com/cli/) and read access; ask a maintainer for access. `.gitignore`
covers the patterns, so the pulled files never show in `git status`. Files your branch tracks win: a pull puts each
tracked file back to its committed content (or keeps your working-tree edit of it). Without the data, the pack tests
(`npm test`, `npm run test:conformance`, `cargo test`) and the recorders fail with an instruction to run
`npm run fixtures:pull`.

### Adding or changing a pack

Check the pack's test data into your pull request, so review sees it and CI runs on it:

- a pack with a code corpus and ground truth, upstream's answers on it: the corpus and the recordings;
- otherwise, the pack's contract is that it passes its upstream's own test suite: the upstream tests' cases the
  recorder copies and upstream's recorded answers on them.

The recorders and generators write these files and print the command that adds them. Because `.gitignore` covers
them, add them with `git add -f`, e.g. `git add -f -- packs/<id>/upstream/<tool>/fixtures
packs/<id>/upstream/<tool>/fixtures.expected.json`. The usual rules for third-party material apply to them (below).

### Merging (maintainers)

Before merging such a pull request, a maintainer moves its pack test data to S3 and deletes it from the pull request:

```sh
npm run fixtures:push -- --dry-run   # what it would upload
npm run fixtures:push                # upload, verify by downloading back, then git rm --cached
git commit -m "Move pack test data to S3"
```

`fixtures:push` runs with the maintainer's own AWS credentials (write access to the bucket). It refuses a dirty index
and tracked pack test data edited in the working tree; it uploads every tracked file the patterns match to its key,
downloads each back and compares bytes, and only then removes them from the index, printing each, and leaves the files
on disk. It never deletes from S3: a pull request that removes pack test data needs the maintainer to delete those keys
(`aws s3 rm`); the bucket is versioned, so nothing is lost. Structure metric #3 (`npm run check`) fails while any pack
test data is tracked, so a pull request cannot merge before the push.

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
