# Rerail rule pack: SQL

22 code rules for SQL, ported from SQLFluff 4.4.0. 19 pass the [conformance check](../../conformance/README.md#the-gate)
and ship: they live in `rules/` and `pack.json` lists them, 17 on and PG01 and PG02 off. The other 3 (AL05, AM02
and RF01) are candidates: they live in `candidates/rules/` and move to `rules/` and `pack.json` once they are ready
(see "Candidates" below). Each port is scored against SQLFluff's own findings on SQLFluff's tests,
and against the labelled lines of SQLFluff's real-code samples
([docs/upstream.md](../../docs/upstream.md#samples)), with every disagreement declared in
`divergences.json`. The conformance check (`--check`, run in CI) scores the tests and the samples: it gates the shipped
rules and reports the candidates. SQL has no rename variants (`holds-under-rename` covers the ECMAScript packs), so the
real-code samples are the pack's only evidence that a port is not fitted to SQLFluff's tests.

It follows the [pack structure and validation contract](../../docs/pack-format.md): it is checked by
`npx vitest run test/engine/catalog/packs.test.ts -t "pack sql"` and by
`node --import tsx conformance/languages/cli.ts --check --pack sql`. The engine reads SQL through its own adapter
([the SQL adapter](../../docs/language-packs.md#the-sql-adapter)).

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): the 19 shipped rules, with PG01 and PG02 `"on": false` |
| `rules/` | The 19 shipped code-rule documents (`rerail.policy/v1`), one per port, named for its id |
| `candidates/rules/` | The 3 candidate code-rule documents: AL05, AM02 and RF01 |
| `conformance.json` | Every port (shipped and candidate): its upstream tool and rule, how findings compare, and the capabilities it exercises; the pinned tool |
| `divergences.json` | Declared disagreements with SQLFluff |
| `upstream/sqlfluff/` | SQLFluff's copied tests (`fixtures/`, and Rerail's own PostgreSQL cases beside them in `fixtures/rerail/`, listed as `RERAIL_CASES` in `generate.py`), its expected findings and case list (`fixtures.expected.json`, `fixtures.cases.json`) and the generator (`generate.py`, sharing `fixtures/sql/common.py`); the lock naming its real-code samples (`samples.lock`) with their sources and licences (`SAMPLES.md`); the upstream manifest `gates.yaml` (its pins, approved configuration, the file types it lints and the facts of every rule the pack ports). The tests, Rerail's cases and the expected findings are pack test data, not in git: one archive in S3, which `test-data.lock` names and `npm run fixtures:pull` fetches with the samples |
| `licenses/sqlfluff/` | SQLFluff's licence and `UPSTREAM.md`, listing every copied file |

## Rules that start off

PG01 and PG02 ship off (`"on": false` in `pack.json`), to be turned on per repository or checkout: SQLFluff's default
configuration leaves them off (`force_enable = False`, the conformance check's `upstream-status`), and they are
PostgreSQL-only, while Rerail reads every `.sql` file in one dialect: on a MySQL or SQLite file they would advise
`CONCURRENTLY` or `NOT VALID`, which those databases lack. AM02, a candidate, is the reverse: SQLFluff skips it for
PostgreSQL, so on a PostgreSQL file a bare `UNION` would be a false positive; it should start off if it ever ships, to
be turned on for MySQL or ANSI SQL. The other 17 shipped rules start on. The scores below cover all 22.

## How the rules read SQL

The engine parses every `.sql` file with one generic grammar and reads it as PostgreSQL does: a naked identifier folds
to lower case, a quoted one is exact, and a whole-row reference (`to_json(t)`) uses its relation. SQLFluff's own tests
are written for ANSI, PostgreSQL, MySQL and SQLite (the four dialects copied). The generator lints each fixture with its
own dialect and each ANSI one again with `--dialect postgres` (`fixtures.cases.json`, `postgresDialect.sameFindings`):
167 ANSI cases find the same either way and 26 do not. Where a port follows PostgreSQL and the fixture's dialect reads
otherwise, the case is a `not-comparable` divergence, its evidence the PostgreSQL reading. Of the 59 divergences, 58
are `not-comparable` (dialect readings, MySQL-only and SQLite-only syntax, files PostgreSQL rejects, and SQLFluff's
dialect gates for PG01 and AM02) and 1 is `rerail-unknown` (a grammar gap in RF01: a redundantly bracketed join).

## Scores

TP/FP/FN of each port against SQLFluff 4.4.0 on the fixtures, from `node --import tsx conformance/languages/cli.ts
--pack sql --fixtures`; `div.` counts the findings a declared divergence explains. Every port has no false positive and
no false negative on the fixtures outside its declared divergences; that says nothing about other SQL, and a port's
reading is PostgreSQL's wherever the dialects differ.

| Port | SQLFluff rule | Status | Fixtures | Skips files with syntax errors |
|---|---|---|---|---|
| `sqlfluff-al04-aliasing-unique-table` | AL04 | shipped | 9/0/0 | yes |
| `sqlfluff-al05-aliasing-unused` | AL05 | candidate | 9/0/0 (7 div.) | yes |
| `sqlfluff-al08-aliasing-unique-column` | AL08 | shipped | 14/0/0 (1 div.) | yes |
| `sqlfluff-al09-aliasing-self-alias-column` | AL09 | shipped | 18/0/0 | no |
| `sqlfluff-am01-ambiguous-distinct` | AM01 | shipped | 1/0/0 | no |
| `sqlfluff-am02-ambiguous-union` | AM02 | candidate | 5/0/0 (1 div.) | yes |
| `sqlfluff-am05-ambiguous-join` | AM05 | shipped | 14/0/0 | no |
| `sqlfluff-am09-ambiguous-order-by-limit` | AM09 | shipped | 3/0/0 | yes |
| `sqlfluff-cv02-convention-coalesce` | CV02 | shipped | 2/0/0 | no |
| `sqlfluff-cv04-convention-count-rows` | CV04 | shipped | 3/0/0 | no |
| `sqlfluff-cv05-convention-is-null` | CV05 | shipped | 7/0/0 | no |
| `sqlfluff-cv08-convention-left-join` | CV08 | shipped | 9/0/0 | no |
| `sqlfluff-pg01-postgres-excessive-locks` | PG01 | shipped, off | 10/0/0 (1 div.) | yes |
| `sqlfluff-pg02-postgres-not-valid-foreign-key` | PG02 | shipped, off | 1/0/0 | yes |
| `sqlfluff-rf01-references-from` | RF01 | candidate | 27/0/0 (5 div.) | yes |
| `sqlfluff-rf04-references-keywords` | RF04 | shipped | 4/0/0 (3 div.) | yes |
| `sqlfluff-rf05-references-special-chars` | RF05 | shipped | 10/0/0 (4 div.) | yes |
| `sqlfluff-rf06-references-quoting` | RF06 | shipped | 19/0/0 (47 div.) | yes |
| `sqlfluff-st01-structure-else-null` | ST01 | shipped | 4/0/0 | no |
| `sqlfluff-st02-structure-simple-case` | ST02 | shipped | 15/0/0 | yes |
| `sqlfluff-st08-structure-distinct` | ST08 | shipped | 7/0/0 | yes |
| `sqlfluff-st10-structure-constant-expression` | ST10 | shipped | 10/0/0 | no |

RF06's 47 fixture divergences are ANSI and MySQL case folding, each line checked with `sqlfluff lint --dialect
postgres`.

## Parse coverage

A port with a clause that is unsound under syntax errors (`source-incomplete` in the
[reference](../../docs/code-rule-dsl-reference.html)) skips every file in which the grammar finds an `ERROR` node; the
others run around the errors (`source-partial`). Of the 283 fixtures, 8 have a syntax error and 275 (97%) parse
cleanly. `--rule <id>` prints each syntax-error file at its first error (`SYNTAX path:line:column`).

## Candidates

`sql/sqlfluff` is gated (`upstream/sqlfluff/gates.yaml`). SQLFluff's own YAML cases, run as `dialect: postgres`,
`templater: raw` with the port's rule configuration (the engine reads every `.sql` file as PostgreSQL; a case SQLFluff's
postgres dialect cannot read whole, a Jinja case and a case configured otherwise left out), show why 3 ports stay
candidates:

| Port | Why |
|---|---|
| `sqlfluff-al05-aliasing-unused` | 6 of its cases differ, each a file the engine cannot read whole and libpg_query rejects (BigQuery, Snowflake and T-SQL syntax); its 27 findings on the rest are all reproduced |
| `sqlfluff-am02-ambiguous-union` | SQLFluff skips AM02 for PostgreSQL, so it reports nothing on its tests: the port's 7 findings are extra and it reproduces no upstream finding |
| `sqlfluff-rf01-references-from` | 9 of its cases differ (55 of 70 findings reproduced): 8 the engine cannot read whole (DuckDB, BigQuery `UNNEST ... WITH OFFSET`, Snowflake `SEMANTIC_VIEW`, and the redundantly bracketed join `JOIN ((b JOIN c ON ...))`, which PostgreSQL accepts), and a Trino row reference, missed at 5:47 |

The `CREATE POLICY` and `ALTER POLICY` known gaps are gone: the grammar reads both statements, and the scope binds the
policy's table for the queries in its `USING` and `WITH CHECK`, as SQLFluff's RF01 does on PostgreSQL. PG01 and PG02
are off in SQLFluff's default configuration (`force_enable = False`), so they ship off.

## Deriving and scoring

Taking a new snapshot is a maintainer deriving the pack's test data from its pins and publishing it
([docs/upstream.md](../../docs/upstream.md#take-a-new-snapshot)); a pull request carries it only as a publish request
under `upstream/sqlfluff/publish/`. The recipe (`scripts/pack-data/recipes/sql.ts`) runs `upstream/sqlfluff/generate.py`
over the fixtures, against the pinned SQLFluff 4.4.0, in the venv that `fixtures/sql/common.py` pins:

```sh
npm run fixtures:derive -- --pack sql
npm run fixtures:stage -- --pack sql    # copy what changed into upstream/sqlfluff/publish/
```

Scoring one rule with every false positive and false negative printed:

```sh
node --import tsx conformance/languages/cli.ts --pack sql --rule sqlfluff-al04-aliasing-unique-table --fixtures
```

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `sqlfluff-al04-aliasing-unique-table`

Port of SQLFluff 4.4.0 AL04, which reads SELECT statements: report a table relation whose alias (its explicit alias, else its table name) repeats the alias of an earlier relation in the same FROM clause, or of a relation in a subquery of the same SELECT, other than the subquery the relation aliases; a relation that calls one of the postgres dialect's value table functions (unnest, generate_series) has no table alias; names compare as SQLFluff's raw_normalized does, with quotes removed and case kept.

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

Port of SQLFluff 4.4.0 RF04 (references.keywords) with its defaults (unquoted_identifiers_policy = aliases, quoted_identifiers_policy = none): report a naked identifier of more than one character that names a table or column alias or a CREATE TABLE column definition and is a keyword. SQLFluff reads the naked identifier against its dialect's unreserved keywords; the engine reads every .sql file as PostgreSQL, so the regex holds exactly SQLFluff's postgres unreserved keywords (dialect_selector('postgres').sets('unreserved_keywords'), 836 words). A column definition is one of a CREATE TABLE column list, with its type, as in SQLFluff's postgres dialect: an ALTER TABLE ... ADD COLUMN name, a typed or partition table's `name WITH OPTIONS`, a foreign table's column and a composite type's attribute are column references or plain words there, and a column definition list after a table function's alias is parameters, so they are skipped.

### `sqlfluff-rf05-references-special-chars`

Port of SQLFluff 4.4.0's RF05 (references.special_chars) with its defaults (both identifier policies all, no space or additional characters allowed): report a naked or quoted identifier whose name, without its quotes and underscores, holds a character other than a Unicode letter or digit (a doubled quote included). A name in a SELECT element that has an alias is not read (only the alias is), and neither is a PostgreSQL extension name or a routine's parameter name (a parameter to SQLFluff, not an identifier).

### `sqlfluff-rf06-references-quoting`

Port of SQLFluff 4.4.0's RF06 (references.quoting) with its defaults (prefer_quoted_identifiers and prefer_quoted_keywords false, case_sensitive true), under SQLFluff's postgres dialect, the reading the engine gives every .sql file: report a double-quoted identifier that would mean the same unquoted, because its name has no upper-case letter (PostgreSQL folds unquoted names to lower case), matches the postgres dialect's naked-identifier template [\p{L}_][\p{L}\p{N}_$]* and is none of the dialect's reserved words (its anti-template, embedded here). PostgreSQL lexes every double-quoted token as a quoted identifier; the engine reads one as an identifier, or as a literal where its grammar takes a name as a string (a column list, an index or extension name). A routine body (dollar-quoted, a string to SQLFluff) is not read.

### `sqlfluff-st01-structure-else-null`

Port of SQLFluff 4.4.0 ST01 (structure.else_null). Reports a CASE whose ELSE is the bare NULL literal, at the CASE start.

### `sqlfluff-st02-structure-simple-case`

Port of SQLFluff 4.4.0 ST02 (structure.simple_case). Reports a searched CASE with one WHEN that is boolean (THEN TRUE ELSE FALSE, or the reverse), or whose condition is `column IS NULL` with the column as its ELSE, or `column IS NOT NULL` with the column as its THEN; anchored at the WHEN condition. A simple CASE, several WHENs, a compound condition and IS DISTINCT FROM are not reported.

### `sqlfluff-st08-structure-distinct`

Port of SQLFluff 4.4.0 ST08 (structure.distinct). Three cases: in a SELECT DISTINCT whose first selected expression is parenthesized alone, reports the parentheses; where the parentheses start an arithmetic expression right after DISTINCT with no space, reports DISTINCT; and reports DISTINCT written as a function, DISTINCT(x), as an aggregate's lone argument (not around a subquery).

### `sqlfluff-st10-structure-constant-expression`

Port of SQLFluff 4.4.0's ST10 (structure.constant_expression): report =, != or <> whose operands are the same non-literal written identically, or two literals other than the allowed 1 = 1 and 1 = 0 (compared without a string's quotes, so '1' = 1 is allowed too), anchored at the operator. SQLFluff's operands are the pieces adjacent to the operator, as its postgres dialect reads them: a positional parameter ($1) is a literal; a date, time, timestamp or interval literal is one operand and no literal; any other typed string (name 'x') is a type then a string, so on the left its string is the operand and on the right the type, which matches nothing; a right operand followed by COLLATE is the collated expression, a left one is the collation name, which matches nothing; an operand that is itself an arithmetic, bitwise or other binary expression is not adjacent, so such comparisons are skipped.
