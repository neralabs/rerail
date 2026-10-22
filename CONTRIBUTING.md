# Contributing to Rerail

Thank you for helping. A few rules keep the project open source and its licensing clean.

Everyone taking part follows the [Code of Conduct](CODE_OF_CONDUCT.md). Report a security vulnerability privately, as
[SECURITY.md](SECURITY.md) describes, never in a public issue or pull request.

## Getting the code

Install [Git LFS](https://git-lfs.com) before you clone, and run `git lfs install` once. The embedding model's weights
(`model/model.safetensors`) and the engine's Unicode names table (`native/src/code_rule_dsl/value/names/names.bin`,
compiled into the engine) are stored in LFS (`.gitattributes`); without it a checkout holds pointer files, and neither
the engine build nor the model works. In a checkout made without it, run `git lfs pull`. Then `npm ci` and
`npm run build`.

Generated files are listed, with the command that rewrites each, in
[scripts/generate/README.md](scripts/generate/README.md); edit the generator, not its output.

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
BSD-3-Clause, CC-BY-4.0, CC0-1.0, ISC, Zlib, 0BSD, Unicode-3.0), so no copyleft file can be committed.

Fixtures (`fixtures/`) and rule packs (`packs/`) follow the same rules; see their READMEs.

## Dependencies

New dependencies must be permissively licensed if they end up in anything Rerail distributes:
`npm run check:licenses` fails on a copyleft production dependency. Copyleft is acceptable only in
development-only dependencies and tools, which are never committed or distributed.

## Pull requests

Open a pull request against `main`. Changes are squash-merged after review.

Before you open it:

- `npm run check` passes: the type check, the layer boundaries, the repository structure gate, Windows-safe paths and
  no internal ticket IDs.
- `npm test` passes, with `npm run build` run first (several suites run the built package from `dist/`).
- `npm run check:licenses` and `reuse lint` pass (`uvx --from 'reuse[charset-normalizer]==6.2.0' reuse lint`, as CI
  runs it).
- A change to a pack under `packs/` adds a changeset ([.changeset/README.md](.changeset/README.md)).
- A change to what the core package ships adds an entry under `## Unreleased` in `CHANGELOG.md`
  ([release notes](README.md#release)).
- Tests cover the behavior you changed, and docs that describe it are updated.
