# Rerail rule packs

Rule packs for Rerail. Each pack is published to npm as its own package, versioned and released separately from
the core package at the repository root.

> Status: pre-release (below 1.0.0). Every pack here is published to npm, public like core, and core ships none
> of them: set-up and `rerail packs add` install them into `$RERAIL_HOME/packs`. The packs
> here: `python`, `php`, `blade`, `rust`, `typescript`, `javascript`, `react`, `vue`, `node`, `next`, `sql` and
> `complexity`. Every rule document Rerail ships lives in a pack; core has no rules of its own, and lists the packs
> by name only (`src/engine/catalog/pack-list.generated.ts`).

- [docs/pack-format.md](../docs/pack-format.md): the layout and the validation contract every pack here passes.
- [conformance/README.md](../conformance/README.md#the-gate): the conformance check (`npm run test:conformance`), the gate every
  shipped ported rule must pass; candidates run through it and are only reported.
- [docs/upstream.md](../docs/upstream.md): upstream snapshots, the packs' test data and samples, and how new data is
  published; [docs/pack-format.md](../docs/pack-format.md#pack-data) for the files and locks.
- [docs/language-packs.md](../docs/language-packs.md#start-a-pack): how to start a pack, for a supported language, a
  planned one, or one that ports another tool's rules.

```sh
npm run fixtures:pull                                               # the packs' test data and samples, from S3
npx vitest run test/engine/catalog/packs.test.ts                    # every pack: checks 1 to 8 and each rule's status
npx vitest run test/engine/catalog/packs.test.ts -t "pack python"   # one pack
npm run test:conformance                                            # the conformance check
```

A pull request that changes a pack adds a changeset (`npx changeset`); see
[versioning](../docs/pack-format.md#versioning-and-release). Pack versions and changelogs are never edited by hand.
A pull request that adds or changes pack data stages it into the tool-pack's tracked `upstream/<tool>/publish/`
directory (`npm run fixtures:stage -- --pack <id> --tool <tool>`); a maintainer runs the publish workflow
(`pack-data-publish.yml`) for it, a code owner approves the upload, and the workflow commits the new locks and removes
`publish/` ([CONTRIBUTING.md](../CONTRIBUTING.md#pack-test-data)). Deriving test data from the pins is paused for now
([docs/upstream.md](../docs/upstream.md#derive-and-bumps-are-paused)).

## Pack layout

Each pack lives in `packs/<id>/`:

| Path | Contents | Published |
| --- | --- | --- |
| `package.json` | npm package `@neralabs/rerail-pack-<id>`; core version range as a peer dependency; no dependencies | yes |
| `pack.json` | The pack (`rerail.rule-pack/v1`): match, and each rule's document, title, group, mode and examples | yes |
| `rules/<id>.json` | Code-rule documents (`rerail.policy/v1`), each named for its rule id ([rule ids](../docs/pack-format.md#packjson)) | yes |
| `LICENSE`, `NOTICE`, `README.md` | The pack's licence, notice and description | yes |
| `licenses/<project>/` | Upstream licence, verbatim, and `UPSTREAM.md`: per upstream repository, its source, the patterns of the files taken and how, and what is not copied ([format](../docs/pack-format.md#upstreammd)) | yes |
| `conformance.json`, `divergences.json`, `upstream/`, `candidates/` | For ported rules: the ports, declared divergences, ports under evaluation (scored and reported, never gated), and per tool `upstream/<tool>/` with its generator, `gates.yaml` and the locks of its pack data. The tool's copied tests, expected findings and samples are not in git: one archive each in S3, which `test-data.lock` and `samples.lock` name and `npm run fixtures:pull` fetches ([pack data](../docs/pack-format.md#pack-data)) | no |

A pack is data only: it never imports the core engine. Core runs its rules, and the repository's tests validate
every pack against the contract. A pack may hold rules for a language core lists as `planned`: they are stored
inactive and reported as `not supported yet` until core reads the language.

## Third-party material

Packs derived from upstream rule sets keep the upstream licence under `packs/<id>/licenses/<project>/`, with an
`UPSTREAM.md` per upstream repository; pack check 6 checks the pulled test data against its patterns, and the publish
refuses any file whose licence the repository's allowlist (`scripts/check/licence-allowlist.json`) does not take. The
rule documents are annotated in `REUSE.toml`, and `reuse lint` must pass ([licensing](../docs/pack-format.md#licensing)).
