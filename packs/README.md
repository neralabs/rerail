# Rerail rule packs

Rule packs for Rerail. Each pack is published to npm as its own package, versioned and released separately from
the core package at the repository root.

> Status: pre-release. No pack is published yet, and core does not load installed packs yet: the TSLint pack still
> ships inside the core package (`policies/rule-packs/tslint.json`). The packs here: `python`, `php`, `blade`,
> `typescript`, `react`, `node` and `next`.

- [docs/pack-format.md](../docs/pack-format.md): the layout and the validation contract every pack here passes.
- [docs/language-packs.md](../docs/language-packs.md#start-a-pack): how to start a pack, for a supported language, a
  planned one, or one that ports another tool's rules.

```sh
npx vitest run test/packs.test.ts                    # every pack: checks 1 to 6 and each rule's status
npx vitest run test/packs.test.ts -t "pack python"   # one pack
```

A pull request that changes a pack adds a changeset (`npx changeset`); see
[versioning](../docs/pack-format.md#versioning-and-release). Pack versions and changelogs are never edited by hand.

## Pack layout

Each pack lives in `packs/<id>/`:

| Path | Contents | Published |
| --- | --- | --- |
| `package.json` | npm package `@neralabs/rerail-pack-<id>`; core version range as a peer dependency; no dependencies | yes |
| `pack.json` | The pack (`rerail.rule-pack/v1`): match, and each rule's document, title, group, mode and examples | yes |
| `rules/*.json` | Code-rule documents (`rerail.policy/v1`) | yes |
| `LICENSE`, `NOTICE`, `README.md` | The pack's licence, notice and description | yes |
| `licenses/<project>/` | Upstream licence, verbatim, and `UPSTREAM.md` listing every copied or adapted file | yes |
| `conformance.json`, `divergences.json`, `upstream/`, `candidates/` | For ported rules: the tools' recorded findings, their copied tests, declared divergences, ports under evaluation | no |

A pack is data only: it never imports the core engine. Core runs its rules, and the repository's tests validate
every pack against the contract. A pack may hold rules for a language core lists as `planned`: they are stored
inactive and reported as `not supported yet` until core reads the language.

## Third-party material

Packs derived from upstream rule sets keep the upstream licence under `packs/<id>/licenses/<project>/`. The
repository's `REUSE.toml` maps the copied files to their licence, `reuse lint` must pass, and the licence check
rejects any licence outside the repository's allowlist (`scripts/licence-allowlist.json`).
