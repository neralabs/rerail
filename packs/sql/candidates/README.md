# Rerail rule pack: SQL, candidates

The ports in `rules/` here are still being evaluated: scored and reported against the tools' recordings, never gated
and never shipped. This file is not published; the pack's [README](../README.md) covers the rules it ships.

PG01 and PG02 ship off because they are PostgreSQL-only while Rerail reads every `.sql` file in one dialect. AM02 is the
reverse: SQLFluff skips it for PostgreSQL, so on a PostgreSQL file a bare `UNION` would be a false positive; it should
start off if it ever ships, to be turned on for MySQL or ANSI SQL.

## Scores

TP/FP/FN of each candidate against SQLFluff 4.4.0, as the pack README's scores. Each has no false positive and no false
negative on the fixtures outside its declared divergences; AM02 has one false positive on postgres.

Over every port, shipped and candidate: of the 59 divergences, 58 are `not-comparable` (dialect readings, MySQL-only
and SQLite-only syntax, files PostgreSQL rejects, and SQLFluff's dialect gates for PG01 and AM02) and 1 is
`rerail-unknown` (a grammar gap in RF01: a redundantly bracketed join); the candidates hold 13 of them, AL05's 7, RF01's
5 and AM02's 1. Of the 63 corpus false negatives the report lists (at most 50 per port and corpus), RF01's 7 among
them, 59 are in a file with a syntax error.

| Port | SQLFluff rule | Status | Fixtures | postgres | mattermost | anki | application-services | Skips files with syntax errors |
|---|---|---|---|---|---|---|---|---|
| `sqlfluff-al05-aliasing-unused` | AL05 | candidate | 9/0/0 (7 div.) | 61/0/0 | 0/0/0 | 1/0/0 | 0/0/0 | yes |
| `sqlfluff-am02-ambiguous-union` | AM02 | candidate | 5/0/0 (1 div.) | 0/1/0 | 0/0/0 | 0/0/0 | 0/0/0 | yes |
| `sqlfluff-rf01-references-from` | RF01 | candidate | 27/0/0 (5 div.) | 0/0/0 | 0/0/0 | 0/0/0 | 0/0/7 | yes |

## Port readiness

Where each candidate stops, from `npm run port-readiness -- --pack sql --tool sqlfluff`:

| Port | Stops at | Why |
|---|---|---|
| `sqlfluff-al05-aliasing-unused` | matches-upstream-tests | 6 of its cases differ, each a file the engine cannot read whole and libpg_query rejects (BigQuery, Snowflake and T-SQL syntax); its 27 findings on the rest are all reproduced |
| `sqlfluff-am02-ambiguous-union` | matches-upstream-tests | SQLFluff skips AM02 for PostgreSQL, so it reports nothing on its tests: the port's 7 findings are extra, it reproduces no upstream finding (`fires`), and 1 golden differs (`functional_deps.sql`, 181:1 extra) |
| `sqlfluff-rf01-references-from` | matches-upstream-tests | 9 of its cases differ (55 of 70 findings reproduced): 8 the engine cannot read whole (DuckDB, BigQuery `UNNEST ... WITH OFFSET`, Snowflake `SEMANTIC_VIEW`, and the redundantly bracketed join `JOIN ((b JOIN c ON ...))`, which PostgreSQL accepts), and a Trino row reference, missed at 5:47 |

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `sqlfluff-al05-aliasing-unused`

Port of SQLFluff 4.4.0 AL05 (aliasing.unused) with default settings. Not reported where the alias is required (a derived table or VALUES list) or where the same table is referenced more than once.

### `sqlfluff-am02-ambiguous-union`

Port of SQLFluff 4.4.0 AM02 (ambiguous.union). Flags a UNION not immediately followed by DISTINCT or ALL. SQLFluff skips AM02 in its postgres dialect (it runs in ANSI, MySQL and other dialects with UNION DISTINCT; not SQLite), so the rule would start off and be turned on for MySQL/ANSI repositories (README already covers the start-off part).

### `sqlfluff-rf01-references-from`

Port of SQLFluff 4.4.0 RF01 (references.from), default settings. Flags the table qualifier of a column reference that names no table or alias in the FROM clause of its query or of an enclosing query.
