# Rerail rule packs

Rule packs for Rerail. Each pack is published to npm as its own package, versioned and released separately from
the core package at the repository root.

> Status: pre-release. No pack is published yet; the TSLint pack still ships inside the core package
> (`policies/rule-packs/tslint.json`) until core loads packs installed from npm.

## Pack layout

The full specification (layout, manifest, validation data, `rerail pack check`, versioning, licensing) is
[docs/pack-format.md](../docs/pack-format.md). In short, each pack lives in `packs/<id>/`:

| Path | Contents | Published |
| --- | --- | --- |
| `package.json` | npm package `@neralabs/rerail-pack-<id>`, marked by `"rerail": { "pack": "./rerail-pack.json" }`; no dependencies | yes |
| `rerail-pack.json` | Manifest: id, match, `coreRange`, `languages`, `license`, `upstream[]`, and each rule's title, group, mode and examples | yes |
| `rules/*.json` | Rule documents | yes |
| `LICENSE`, `NOTICE`, `README.md` | The pack's licence, notice and description | yes |
| `licenses/<project>/` | Upstream licence, verbatim, and `UPSTREAM.md` listing every copied or adapted file | yes |
| `validation/` | Upstream tools' recorded findings, their copied tests, declared divergences, candidate rules | no |

A pack is data only: it never imports the core engine. Core loads it, and core validates it with
`rerail pack check`.

## Third-party material

Packs derived from upstream rule sets keep the upstream licence under `packs/<name>/licenses/<project>/`. The
repository's `REUSE.toml` maps the copied files to their licence, `reuse lint` must pass, and the licence check
rejects any licence outside the repository's allowlist.
