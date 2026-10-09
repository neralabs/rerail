# Rerail rule pack: Python

22 code rules for Python, ported from Ruff 0.16.9 (17 rules) and Bandit 1.8.6 (5 tests). Each is scored against
the tool's own findings on the tool's test fixtures, through the engine's production workspace path, with every
disagreement declared in `divergences.json` (none today).

This is the reference pack for the [pack structure and validation contract](../../docs/pack-format.md): it is
checked by `npx vitest run test/engine/catalog/packs.test.ts`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title and group; two rules carry examples |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `conformance.json` | Each ported rule's upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tools and the pinned corpus (django) |
| `divergences.json` | Declared disagreements with the tools |
| `upstream/<tool>/` | The tool's copied tests (`fixtures/`), its recordings (`fixtures.expected.json`, `corpus-django.expected.json`) and the recorder (`generate.py`, sharing `upstream/common.py`) |
| `licenses/<tool>/` | The tool's licence and `UPSTREAM.md`, listing every copied file |

Re-recording runs offline against the pinned tool versions, under CPython 3.14:

```sh
python3.14 upstream/ruff/generate.py fixtures <ruff checkout at 0.16.9> <ruff 0.16.9 executable>
python3.14 upstream/ruff/generate.py corpus <ruff 0.16.9 executable> django=<django checkout>
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack python --rule ruff-f841-unused-variable --fixtures
```
