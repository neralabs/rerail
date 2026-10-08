# Rerail rule packs

Rule packs for Rerail. Each pack is published to npm as its own package, versioned and released separately from
the core package at the repository root.

> Status: pre-release. No pack is published yet; the TSLint pack still ships inside the core package
> (`policies/rule-packs/tslint.json`) until core loads packs installed from npm.

## Pack layout

The structure and the validation contract are specified in [docs/pack-format.md](../docs/pack-format.md). In short,
each pack lives in `packs/<id>/`:

| Path | Contents | Published |
| --- | --- | --- |
| `package.json` | npm package `@neralabs/rerail-pack-<id>`; core version range as a peer dependency; no dependencies | yes |
| `pack.json` | The pack (`rerail.rule-pack/v1`): match, and each rule's document, title, group, mode and examples | yes |
| `rules/*.json` | Code-rule documents | yes |
| `LICENSE`, `NOTICE`, `README.md` | The pack's licence, notice and description | yes |
| `licenses/<project>/` | Upstream licence, verbatim, and `UPSTREAM.md` listing every copied or adapted file | yes |
| `conformance.json`, `divergences.json`, `upstream/`, `candidates/` | For ported rules: the tools' recorded findings, their copied tests, declared divergences, ports under evaluation | no |

A pack is data only: it never imports the core engine. Core runs its rules, and the repository's tests validate
every pack against the contract. A pack may hold rules for a language core lists as `planned`: they are stored
inactive and reported as `not supported yet` until core reads the language
([language packs](../docs/language-packs.md)).

## Third-party material

Packs derived from upstream rule sets keep the upstream licence under `packs/<name>/licenses/<project>/`. The
repository's `REUSE.toml` maps the copied files to their licence, `reuse lint` must pass, and the licence check
rejects any licence outside the repository's allowlist.
