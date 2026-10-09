# Rerail rule pack: Python

156 code rules for Python, ported from Ruff 0.16.9 (115 rules) and Bandit 1.9.4 (41 tests). Each is scored against the
tool's own findings on the tool's test fixtures, through the engine's production workspace path, with every
disagreement declared in `divergences.json` (none today). Every port honours its tool's suppression comment, as the
tool does: Ruff's `# noqa` (`suppress.noqa`) and Bandit's `# nosec` (`suppress.nosec`). 31 Ruff rules that Ruff's
default selection leaves off ship off (`"on": false` in `pack.json`).

This is the reference pack for the [pack structure and validation contract](../../docs/pack-format.md): it is
checked by `npx vitest run test/engine/catalog/packs.test.ts`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title, group and whether it is on; two rules carry examples |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `conformance.json` | Each ported rule's upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tools and the pinned corpus (django) |
| `divergences.json` | Declared disagreements with the tools |
| `upstream/<tool>/` | The tool's copied tests (`fixtures/`), the cases Rerail wrote for it (`fixtures/rerail/`, listed in `UPSTREAM.md`: every shipped rule on lines carrying `# noqa` or `# nosec`, and probes of inputs the ports once misread), its recordings (`fixtures.expected.json`, `corpus-django.expected.json`) and the recorder (`generate.py`, sharing `upstream/common.py`) |
| `licenses/<tool>/` | The tool's licence and `UPSTREAM.md`, listing every copied file |
| `candidates/rules/` | Ports still being evaluated (78 Ruff and Bandit ports): never shipped or listed in `pack.json`. 76 have a `conformance.json` port and are scored against the recordings without gating; A005 (GPL-derived fixtures only) and INP001 (tested only with a `namespace-packages` option) are not recorded |

Re-recording runs offline against the pinned tool versions, under CPython 3.14 (Bandit itself runs from its own
install, on CPython 3.13):

```sh
python3.14 upstream/ruff/generate.py fixtures <ruff checkout at 0.16.9> <ruff 0.16.9 executable>
python3.14 upstream/ruff/generate.py corpus <ruff 0.16.9 executable> django=<django checkout>
python3.14 upstream/bandit/generate.py fixtures <bandit checkout at 1.9.4> <bandit 1.9.4 executable>
python3.14 upstream/bandit/generate.py corpus <bandit 1.9.4 executable> django=<django checkout>
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack python --rule ruff-f823-undefined-local --fixtures
```
