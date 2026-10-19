# Rerail rule pack: SQL

22 code rules for SQL, ported from SQLFluff 4.4.0. Every one is a candidate: it lives in `candidates/rules/` and moves
to `rules/` and `pack.json` once it passes every blocking [port readiness](../../docs/port-readiness.md) gate (see
"Port readiness" below). While `pack.json` lists no rules, the pack ships none and is never recommended. Each port is
scored against SQLFluff's own findings on SQLFluff's tests and on pinned public corpora (postgres, mattermost, anki,
application-services), with every disagreement declared in `divergences.json`. The conformance check (`--check`, run in
CI) reports the candidates without gating them.

It follows the [pack structure and validation contract](../../docs/pack-format.md): it is checked by
`npx vitest run test/engine/catalog/packs.test.ts -t "pack sql"` and by
`node --import tsx conformance/languages/cli.ts --check --pack sql`. The engine reads SQL through its own adapter
([the SQL adapter](../../docs/language-packs.md#the-sql-adapter)).

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`); it lists no rules while every port is a candidate |
| `candidates/rules/` | The 22 candidate code-rule documents (`rerail.policy/v1`), one per port, named for its id |
| `conformance.json` | Every port (all candidates): its upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tool and the pinned corpora |
| `divergences.json` | Declared disagreements with SQLFluff |
| `upstream/sqlfluff/` | SQLFluff's copied tests (`fixtures/`, and Rerail's own PostgreSQL cases beside them in `fixtures/rerail/`, listed as `RERAIL_CASES` in `generate.py`), its recordings (`fixtures.expected.json`, `fixtures.cases.json`, `corpus-<name>.expected.json`) and the recorder (`generate.py`, sharing `fixtures/sql/common.py`); the port readiness gates' `gates.yaml`, `recording.lock` and `goldens.tsv` |
| `licenses/sqlfluff/` | SQLFluff's licence and `UPSTREAM.md`, listing every copied file |

## Rules that would start off

PG01, PG02 and AM02 depend on SQLFluff's dialect, and Rerail reads every `.sql` file in one dialect, so each should
start off (`"on": false` in `pack.json`) if it ever ships, to be turned on per repository or checkout. PG01 and PG02
are PostgreSQL-only: on a MySQL or SQLite file they would advise `CONCURRENTLY` or `NOT VALID`, which those databases
lack. AM02 is the reverse: SQLFluff skips it for PostgreSQL, so on a PostgreSQL file a bare `UNION` would be a false
positive; it would be turned on for MySQL or ANSI SQL. The other 19 would start on. The scores below cover all 22.

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

TP/FP/FN of each candidate against SQLFluff 4.4.0, from `node --import tsx conformance/languages/cli.ts --pack sql`; `div.` counts the
findings a declared divergence explains. On the recorded fixtures and the four pinned corpora, every candidate has no
false positive and no false negative on the fixtures outside its declared divergences; that says nothing about other
SQL, and a port's reading is PostgreSQL's wherever the dialects differ. Corpus false negatives do not fail the gate.

| Port | SQLFluff rule | Fixtures | postgres | mattermost | anki | application-services | Skips files with syntax errors |
|---|---|---|---|---|---|---|---|
| `sqlfluff-al04-aliasing-unique-table` | AL04 | 9/0/0 | 0/0/3 | 0/0/0 | 0/0/0 | 0/0/0 | yes |
| `sqlfluff-al05-aliasing-unused` | AL05 | 10/0/0 (6 div.) | 4/0/57 | 0/0/0 | 1/0/0 | 0/0/0 | yes |
| `sqlfluff-al08-aliasing-unique-column` | AL08 | 14/0/0 (1 div.) | 1/0/52 | 0/0/0 | 0/0/0 | 0/0/1 | yes |
| `sqlfluff-al09-aliasing-self-alias-column` | AL09 | 18/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | no |
| `sqlfluff-am01-ambiguous-distinct` | AM01 | 1/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | no |
| `sqlfluff-am02-ambiguous-union` | AM02 | 5/0/0 (1 div.) | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | yes |
| `sqlfluff-am05-ambiguous-join` | AM05 | 14/0/0 | 26/0/8 | 4/0/1 | 2/0/0 | 0/0/2 | no |
| `sqlfluff-am09-ambiguous-order-by-limit` | AM09 | 3/0/0 | 1/0/2 | 0/0/0 | 2/0/0 | 0/0/0 | yes |
| `sqlfluff-cv02-convention-coalesce` | CV02 | 2/0/0 | 0/0/0 | 3/0/0 | 0/0/0 | 0/0/1 | no |
| `sqlfluff-cv04-convention-count-rows` | CV04 | 3/0/0 | 0/0/0 | 0/0/0 | 1/0/0 | 0/0/0 | no |
| `sqlfluff-cv05-convention-is-null` | CV05 | 7/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | no |
| `sqlfluff-cv08-convention-left-join` | CV08 | 9/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | no |
| `sqlfluff-pg01-postgres-excessive-locks` | PG01 | 10/0/0 (1 div.) | 55/0/56 | 148/0/118 | 0/0/0 (3 div.) | 0/0/0 | yes |
| `sqlfluff-pg02-postgres-not-valid-foreign-key` | PG02 | 1/0/0 | 0/0/1 | 0/0/0 | 0/0/0 | 0/0/0 | yes |
| `sqlfluff-rf01-references-from` | RF01 | 26/0/0 (6 div.) | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/7 | yes |
| `sqlfluff-rf04-references-keywords` | RF04 | 4/0/0 (3 div.) | 28/0/78 | 55/0/87 | 0/0/1 (1 div.) | 0/0/0 | yes |
| `sqlfluff-rf05-references-special-chars` | RF05 | 10/0/0 (4 div.) | 0/0/9 | 0/0/0 | 10/0/0 | 0/0/0 | yes |
| `sqlfluff-rf06-references-quoting` | RF06 | 19/0/0 (47 div.) | 23/0/13 | 4/0/1 | 0/0/0 | 0/0/0 | yes |
| `sqlfluff-st01-structure-else-null` | ST01 | 4/0/0 | 0/0/0 | 0/0/0 | 1/0/1 | 0/0/0 | no |
| `sqlfluff-st02-structure-simple-case` | ST02 | 15/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/0 | yes |
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
| fixtures | 283 | 0 | 283 | 13 | 270 (95%) |
| postgres | 629 | 342 | 287 | 202 | 85 (30%) |
| mattermost | 560 | 0 | 560 | 262 | 298 (53%) |
| anki | 112 | 15 | 97 | 33 | 64 (66%) |
| application-services | 21 | 15 | 6 | 4 | 2 (33%) |

The corpus false negatives are mostly these files: of the 356 the report lists (at most 50 per port and corpus), 347
are in a file with a syntax error. `--rule <id>` prints each syntax-error file at its first error (`SYNTAX
path:line:column`).

## Port readiness

`sql/sqlfluff` is gated (`upstream/sqlfluff/gates.yaml`). Upstream runs as `dialect: postgres`, `templater: raw` with
the port's rule configuration on every SQLFluff YAML case and corpus file, because the engine reads every `.sql` file
as PostgreSQL; a case SQLFluff's postgres dialect cannot read whole, a Jinja case and a case configured otherwise are
skipped. From `npm run port-readiness -- --pack sql --tool sqlfluff`, no candidate passes every blocking gate yet:

| Port | Stops at | Why |
|---|---|---|
| `sqlfluff-al04-aliasing-unique-table` | compact | 57.1% of its clauses is structure written out over and over (limit 33.3%) |
| `sqlfluff-rf06-references-quoting` | not-overfit | the gate cannot run its class-subtraction regex (`[\p{L}_--\p{Lu}--\p{Lt}]`); one golden also differs (`contrib/amcheck/sql/check_gin.sql`, 2 missing) |
| `sqlfluff-am02-ambiguous-union` | matches-upstream-tests | SQLFluff skips AM02 for PostgreSQL, so it reports nothing on its tests and the port's 7 findings are extra |
| `sqlfluff-al05-aliasing-unused` | matches-upstream-tests | 9 of its cases differ: 8 the engine cannot read whole (BigQuery, Snowflake and T-SQL syntax), 1 finding missed and 1 extra |
| `sqlfluff-rf01-references-from` | matches-upstream-tests | 21 of its cases differ (50 of 70 findings reproduced): bracketed joins and dialect syntax the engine cannot read; `CREATE POLICY` and `ALTER POLICY` are listed known gaps |
| `sqlfluff-st02-structure-simple-case` | matches-upstream-tests | misses `test_fail_multiple_indexed_expression_when_not_null_no_else` (1:18) |
| the other 16 | matches-upstream-tests | `engine-reads-goldens` alone |

Every candidate also fails `engine-reads-goldens`: between 73 and 277 corpus files SQLFluff reads whole are files the
engine's grammar cannot (postgres 202, mattermost 71, anki 4). libpg_query reads 121 of them as valid PostgreSQL
(`DO $$ … $$` blocks, among others); the other 156 hold what only SQLFluff accepts (`:name` parameters, psql
meta-commands). Those are engine gaps (NER-1764), recorded in `goldens.tsv`. PG01 and PG02 are off in SQLFluff's
default configuration (`force_enable = False`), so they would ship off.

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

The port readiness recording (`upstream/sqlfluff/recording.lock`) comes from the recorder's own runtime
(`conformance/recorders/port-readiness/runtimes/sqlfluff-4.4.0`):

```sh
npm run port-readiness:record -- --pack sql --tool sqlfluff
npm run port-readiness -- --pack sql --tool sqlfluff
```

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `sqlfluff-al04-aliasing-unique-table`

Port of SQLFluff 4.4.0 AL04. Reports each later table relation whose explicit alias (or, unaliased, its table name) matches an earlier one in the same FROM clause, compared under PostgreSQL name folding.

### `sqlfluff-al05-aliasing-unused`

Port of SQLFluff 4.4.0 AL05 (aliasing.unused) with default settings. Not reported where the alias is required (a derived table or VALUES list) or where the same table is referenced more than once.

### `sqlfluff-al08-aliasing-unique-column`

Port of SQLFluff 4.4.0 AL08. In each SELECT list, reports each later reused explicit alias or column-reference name, case-insensitively, with identifier quote delimiters ignored.

### `sqlfluff-al09-aliasing-self-alias-column`

Port of SQLFluff 4.4.0 AL09 (aliasing.self_alias.column). Reports a column reference aliased to its own last name part, either written identically (quotes and case included) or, both unquoted, differing in case only (the "ambiguous self alias" message). Anchored at the select element's start.

### `sqlfluff-am01-ambiguous-distinct`

Port of SQLFluff 4.4.0 AM01 (ambiguous.distinct). Implementation note: the grammar puts GROUP BY in the FROM node after the SELECT node.

### `sqlfluff-am02-ambiguous-union`

Port of SQLFluff 4.4.0 AM02 (ambiguous.union). Flags a UNION not immediately followed by DISTINCT or ALL. SQLFluff skips AM02 in its postgres dialect (it runs in ANSI, MySQL and other dialects with UNION DISTINCT; not SQLite), so the rule would start off and be turned on for MySQL/ANSI repositories (README already covers the start-off part).

### `sqlfluff-am05-ambiguous-join`

Port of SQLFluff 4.4.0 AM05 (ambiguous.join) with default fully_qualify_join_types=inner: reports a join whose first join keyword is bare JOIN, anchored at the JOIN clause start.

### `sqlfluff-am09-ambiguous-order-by-limit`

Port of SQLFluff 4.4.0 AM09 (ambiguous.order_by_limit). Anchored at LIMIT. A LIMIT that ends a UNION, INTERSECT or EXCEPT belongs to the set expression, which SQLFluff's AM09 does not read. The grammar parses no OFFSET without LIMIT.

### `sqlfluff-cv02-convention-coalesce`

Port of SQLFluff 4.4.0 CV02 (convention.coalesce), default settings. Only unqualified IFNULL/NVL are reported; schema-qualified names are user-defined functions and are excluded.

### `sqlfluff-cv04-convention-count-rows`

Port of SQLFluff 4.4.0 CV04 (convention.count_rows) with prefer_count_1 and prefer_count_0 false. An unqualified COUNT whose only argument is the literal 0 or 1 is reported at the function's start; COUNT(column), COUNT(DISTINCT ...) and other expressions are not.

### `sqlfluff-cv05-convention-is-null`

Port of SQLFluff 4.4.0 CV05 (convention.is_null). Only =, != and <> whose right operand is the bare NULL literal (not parenthesised), anchored at the operator. A SET clause's assignment and an exclusion constraint's operator are not comparisons in the grammar, as SQLFluff skips them.

### `sqlfluff-cv08-convention-left-join`

Port of SQLFluff 4.4.0 CV08 (convention.left_join). Reports RIGHT JOIN and RIGHT OUTER JOIN at the start of the join clause.

### `sqlfluff-pg01-postgres-excessive-locks`

Port of SQLFluff 4.4.0 PG01 (postgres.excessive_locks), force_enable=true. CREATE INDEX (except UNIQUE), DROP INDEX, REINDEX and REFRESH MATERIALIZED VIEW must use CONCURRENTLY; each anchored at the statement start. Statements inside a dollar-quoted routine or DO body (a string to SQLFluff) or an error region are not read.

### `sqlfluff-pg02-postgres-not-valid-foreign-key`

Port of SQLFluff 4.4.0 PG02 (postgres.not_valid_foreign_key), force_enable=true. ALTER TABLE ... ADD CONSTRAINT ... FOREIGN KEY must include NOT VALID; anchored at the ALTER TABLE start. Statements inside a dollar-quoted routine or DO body or an error region are not read.

### `sqlfluff-rf01-references-from`

Port of SQLFluff 4.4.0 RF01 (references.from), default settings. Flags the table qualifier of a column reference that names no table or alias in the FROM clause of its query or of an enclosing query.

### `sqlfluff-rf04-references-keywords`

Port of SQLFluff 4.4.0 RF04 (references.keywords) with defaults (unquoted_identifiers_policy = aliases, quoted_identifiers_policy = none). Reports a naked identifier of more than one character that names a table or column alias or a CREATE TABLE column definition and is a keyword. SQLFluff reads the file's dialect's unreserved keywords; the engine has no dialect setting, so the regex holds the keywords that SQLFluff's postgres and mysql dialects both list as unreserved (a reserved word cannot be a naked alias there). ALTER TABLE ... ADD COLUMN names are skipped, as SQLFluff's postgres dialect reads them as column references.

### `sqlfluff-rf05-references-special-chars`

Port of SQLFluff 4.4.0 RF05 (references.special_chars) with defaults (both identifier policies all, no space or additional characters allowed). Reports a naked or quoted identifier whose name, without quotes and underscores, holds a character other than a Unicode letter or digit (a doubled quote included). A name in a SELECT element that has an alias is not read (only the alias is), nor is a PostgreSQL extension name.

### `sqlfluff-rf06-references-quoting`

Port of SQLFluff 4.4.0 RF06 (references.quoting) with defaults (prefer_quoted_identifiers and prefer_quoted_keywords false, case_sensitive true), under SQLFluff's postgres dialect. Reports a double-quoted identifier whose name has no upper-case letter, matches the postgres naked-identifier template [\p{L}_][\p{L}\p{N}_$]* and is not one of the dialect's reserved words (its anti-template, embedded in the rule). A dollar-quoted routine body is not read.

### `sqlfluff-st01-structure-else-null`

Port of SQLFluff 4.4.0 ST01 (structure.else_null). Reports a CASE whose ELSE is the bare NULL literal, at the CASE start.

### `sqlfluff-st02-structure-simple-case`

Port of SQLFluff 4.4.0 ST02 (structure.simple_case). Reports a searched CASE with one WHEN that is boolean (THEN TRUE ELSE FALSE, or the reverse), or whose condition is `column IS NULL` with the column as its ELSE, or `column IS NOT NULL` with the column as its THEN; anchored at the WHEN condition. A simple CASE, several WHENs, a compound condition and IS DISTINCT FROM are not reported.

### `sqlfluff-st08-structure-distinct`

Port of SQLFluff 4.4.0 ST08 (structure.distinct). Three cases: in a SELECT DISTINCT whose first selected expression is parenthesized alone, reports the parentheses; where the parentheses start an arithmetic expression right after DISTINCT with no space, reports DISTINCT; and reports DISTINCT written as a function, DISTINCT(x), as an aggregate's lone argument (not around a subquery).

### `sqlfluff-st10-structure-constant-expression`

Port of SQLFluff 4.4.0 ST10 (structure.constant_expression). Reports =, != or <> whose operands are the same non-literal written identically, or two literals other than the allowed 1 = 1 and 1 = 0, anchored at the operator. A typed literal (timestamp '...') is not a literal there. An operand that is itself an arithmetic, bitwise or other binary expression is not one of SQLFluff's adjacent operands, so such comparisons are skipped.
