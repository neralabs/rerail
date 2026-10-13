# Rerail rule pack: Rust

30 code rules for Rust, ported from Clippy 0.1.99. Each is scored against Clippy's recorded findings on its UI tests, through the engine's production workspace path, with every disagreement declared in `divergences.json` (none today).

This pack follows the [pack structure and validation contract](../../docs/pack-format.md) and is checked by `npx vitest run test/engine/catalog/packs.test.ts`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title and group |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `conformance.json` | Each ported lint's upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned Clippy tool and corpora |
| `divergences.json` | Declared disagreements with Clippy |
| `upstream/clippy/` | Clippy's copied UI tests, recordings and recorder (`generate.py`, sharing `upstream/common.py`) |
| `licenses/clippy/` | Clippy's licence and `UPSTREAM.md`, listing every copied file |

Re-recording runs offline against Clippy 0.1.99 and Rust 1.99.0:

```sh
python3 upstream/clippy/generate.py fixtures <rust-clippy checkout at tag rust-1.99.0> <scratch directory>
python3 upstream/clippy/generate.py corpus <cargo target root> ripgrep=<ripgrep checkout> tokio=<tokio checkout>
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack rust --rule clippy-precedence --fixtures
```
