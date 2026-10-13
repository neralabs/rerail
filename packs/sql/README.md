# Rerail rule pack: SQL

22 code rules for SQL, ported from SQLFluff 4.4.0. Each is scored against SQLFluff's own findings on SQLFluff's tests
and on pinned public corpora (postgres, mattermost, anki, application-services), with every disagreement declared in
`divergences.json`.

It follows the [pack structure and validation contract](../../docs/pack-format.md): it is checked by
`npx vitest run test/engine/catalog/packs.test.ts -t "pack sql"`. The engine does not read SQL yet, so the rules are
reported as not supported yet and are not active until core reads SQL.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title and group |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `conformance.json` | Each ported rule's upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tool and the pinned corpora |
| `divergences.json` | Declared disagreements with SQLFluff |
| `upstream/sqlfluff/` | SQLFluff's copied tests (`fixtures/`), its recordings (`fixtures.expected.json`, `fixtures.cases.json`, `corpus-<name>.expected.json`) and the recorder (`generate.py`, sharing `fixtures/sql/common.py`) |
| `licenses/sqlfluff/` | SQLFluff's licence and `UPSTREAM.md`, listing every copied file |

Re-recording runs offline against the pinned SQLFluff 4.4.0, in the venv that `fixtures/sql/common.py` documents:

```sh
.rerail/tools/sql/venv/bin/python packs/sql/upstream/sqlfluff/generate.py fixtures
.rerail/tools/sql/venv/bin/python packs/sql/upstream/sqlfluff/generate.py corpus
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack sql --rule sqlfluff-al04-aliasing-unique-table --fixtures
```
