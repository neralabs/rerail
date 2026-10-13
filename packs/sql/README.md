# Rerail rule pack: SQL

22 code rules for SQL, ported from SQLFluff 4.4.0. Each is scored against SQLFluff's own findings on SQLFluff's tests
and on pinned public corpora (postgres, mattermost, anki, application-services), with every disagreement declared in
`divergences.json`.

It follows the [pack structure and validation contract](../../docs/pack-format.md): it is checked by
`npx vitest run test/engine/catalog/packs.test.ts -t "pack sql"` and by
`node --import tsx conformance/languages/cli.ts --check --pack sql`. The engine reads SQL through its own adapter
([the SQL adapter](../../docs/language-packs.md#the-sql-adapter)).

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title and group |
| `rules/` | The code-rule documents (`rerail.policy/v1`) |
| `conformance.json` | Each ported rule's upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tool and the pinned corpora |
| `divergences.json` | Declared disagreements with SQLFluff |
| `upstream/sqlfluff/` | SQLFluff's copied tests (`fixtures/`), its recordings (`fixtures.expected.json`, `fixtures.cases.json`, `corpus-<name>.expected.json`) and the recorder (`generate.py`, sharing `fixtures/sql/common.py`) |
| `licenses/sqlfluff/` | SQLFluff's licence and `UPSTREAM.md`, listing every copied file |

## How the rules read SQL

The engine parses every `.sql` file with one generic grammar and reads it as PostgreSQL does: a naked identifier folds
to lower case, a quoted one is exact, and a whole-row reference (`to_json(t)`) uses its relation. SQLFluff's own tests
are written for ANSI, PostgreSQL, MySQL and SQLite (the four dialects copied). The recorder lints each fixture with its
own dialect and each ANSI one again with `--dialect postgres` (`fixtures.cases.json`, `postgresDialect.sameFindings`):
167 ANSI cases find the same either way and 26 do not. Where a port follows PostgreSQL and the fixture's dialect reads
otherwise, the case is a `not-comparable` divergence, its evidence the PostgreSQL reading. Of the 59 divergences, 57
are `not-comparable` (dialect readings, MySQL-only and SQLite-only syntax, files PostgreSQL rejects, and SQLFluff's
dialect gates for PG01 and AM02) and 2 are `rerail-unknown` (grammar gaps in RF01: a redundantly bracketed join and
`CREATE POLICY`).

## Scores

TP/FP/FN against SQLFluff 4.4.0, from `node --import tsx conformance/languages/cli.ts --pack sql`; `div.` counts the
findings a declared divergence explains. Every port has no false positive anywhere and no false negative on the
fixtures outside its divergences. Corpus false negatives do not fail the gate.

| Port | SQLFluff rule | Fixtures | postgres | mattermost | anki | application-services | Skips files with syntax errors |
|---|---|---|---|---|---|---|---|
| `sqlfluff-al04-aliasing-unique-table` | AL04 | 9/0/0 | 0/0/3 | 0/0/0 | 0/0/0 | 0/0/0 | yes |
| `sqlfluff-al05-aliasing-unused` | AL05 | 10/0/0 (6 div.) | 4/0/57 | 0/0/0 | 1/0/0 | 0/0/0 | yes |
| `sqlfluff-al08-aliasing-unique-column` | AL08 | 14/0/0 (1 div.) | 1/0/52 | 0/0/0 | 0/0/0 | 0/0/1 | yes |
| `sqlfluff-al09-aliasing-self-alias-column` | AL09 | 18/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | no |
| `sqlfluff-am01-ambiguous-distinct` | AM01 | 1/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | no |
| `sqlfluff-am02-ambiguous-union` | AM02 | 5/0/0 (1 div.) | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | yes |
| `sqlfluff-am05-ambiguous-join` | AM05 | 13/0/0 | 26/0/8 | 4/0/1 | 2/0/0 | 0/0/2 | no |
| `sqlfluff-am09-ambiguous-order-by-limit` | AM09 | 3/0/0 | 1/0/2 | 0/0/0 | 2/0/0 | 0/0/0 | yes |
| `sqlfluff-cv02-convention-coalesce` | CV02 | 2/0/0 | 0/0/0 | 3/0/0 | 0/0/0 | 0/0/1 | no |
| `sqlfluff-cv04-convention-count-rows` | CV04 | 3/0/0 | 0/0/0 | 0/0/0 | 1/0/0 | 0/0/0 | no |
| `sqlfluff-cv05-convention-is-null` | CV05 | 7/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | no |
| `sqlfluff-cv08-convention-left-join` | CV08 | 9/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | no |
| `sqlfluff-pg01-postgres-excessive-locks` | PG01 | 8/0/0 (1 div.) | 55/0/56 | 148/0/118 | 0/0/0 (3 div.) | 0/0/0 | yes |
| `sqlfluff-pg02-postgres-not-valid-foreign-key` | PG02 | 1/0/0 | 0/0/1 | 0/0/0 | 0/0/0 | 0/0/0 | yes |
| `sqlfluff-rf01-references-from` | RF01 | 22/0/0 (6 div.) | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/7 | yes |
| `sqlfluff-rf04-references-keywords` | RF04 | 2/0/0 (3 div.) | 28/0/78 | 54/0/88 | 0/0/1 (1 div.) | 0/0/0 | yes |
| `sqlfluff-rf05-references-special-chars` | RF05 | 10/0/0 (4 div.) | 0/0/9 | 0/0/0 | 10/0/0 | 0/0/0 | yes |
| `sqlfluff-rf06-references-quoting` | RF06 | 8/0/0 (47 div.) | 22/0/14 | 2/0/3 | 0/0/0 | 0/0/0 | yes |
| `sqlfluff-st01-structure-else-null` | ST01 | 4/0/0 | 0/0/0 | 0/0/0 | 1/0/1 | 0/0/0 | no |
| `sqlfluff-st02-structure-simple-case` | ST02 | 11/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | yes |
| `sqlfluff-st08-structure-distinct` | ST08 | 7/0/0 | 0/0/1 | 0/0/0 | 0/0/0 | 0/0/0 | yes |
| `sqlfluff-st10-structure-constant-expression` | ST10 | 10/0/0 | 4/0/1 | 0/0/0 | 0/0/0 | 0/0/0 | no |

RF06's 47 fixture divergences are ANSI and MySQL case folding, each line checked with `sqlfluff lint --dialect
postgres`.

## Parse coverage

A port with a clause that is unsound under syntax errors (`source-incomplete` in the
[reference](../../docs/code-rule-dsl-reference.html)) skips every file in which the grammar finds an `ERROR` node; the
others run around the errors (`source-partial`). Per source:

| Source | Files | Left out (SQLFluff could not analyse them) | Scored | With a syntax error, skipped | Parsed cleanly |
|---|---|---|---|---|---|
| fixtures | 265 | 0 | 265 | 13 | 252 (95%) |
| postgres | 629 | 342 | 287 | 202 | 85 (30%) |
| mattermost | 560 | 0 | 560 | 262 | 298 (53%) |
| anki | 112 | 15 | 97 | 33 | 64 (66%) |
| application-services | 21 | 15 | 6 | 4 | 2 (33%) |

The corpus false negatives are mostly these files: of the 356 the report lists (at most 50 per port and corpus), 347
are in a file with a syntax error. `--rule <id>` prints each syntax-error file at its first error (`SYNTAX
path:line:column`).

## Re-recording and scoring

Re-recording runs offline against the pinned SQLFluff 4.4.0, in the venv that `fixtures/sql/common.py` documents:

```sh
.rerail/tools/sql/venv/bin/python packs/sql/upstream/sqlfluff/generate.py fixtures
.rerail/tools/sql/venv/bin/python packs/sql/upstream/sqlfluff/generate.py corpus
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack sql --rule sqlfluff-al04-aliasing-unique-table --fixtures
```
