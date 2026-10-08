# Rerail rule packs

Rule packs for Rerail. Each pack is published to npm as its own package, versioned and released separately from
the core package at the repository root.

> Status: pre-release. No pack is published yet; the TSLint pack still ships inside the core package
> (`policies/rule-packs/tslint.json`) until core loads packs installed from npm.

## Pack layout

Each pack lives in `packs/<name>/`:

| Path | Contents |
| --- | --- |
| `package.json` | npm package, named `@neralabs/rerail-pack-<name>`; no dependency on the core package |
| `rerail-pack.json` | Pack manifest: `name`, `coreRange`, `languages`, `license`, `upstream[]` |
| `LICENSE`, `NOTICE` | The pack's licence and notice, shipped in the package |
| `licenses/<project>/LICENSE` | Upstream licence, copied verbatim at the pinned version |
| `licenses/<project>/UPSTREAM.md` | Every file copied from the project, and whether it is verbatim, derived or an excerpt |
| `rules/*.json` | Rule definitions |

`coreRange` is the range of Rerail core versions the pack supports. `upstream` lists the projects the pack copies
or adapts rules from; each one has a `licenses/<project>/` directory.

A pack is data only: it never imports the core engine. Core loads it, and core validates it.

## Third-party material

Packs derived from upstream rule sets keep the upstream licence under `packs/<name>/licenses/<project>/`. The
repository's `REUSE.toml` maps the copied files to their licence, `reuse lint` must pass, and the licence check
rejects any licence outside the repository's allowlist.
