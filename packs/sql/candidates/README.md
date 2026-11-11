# Rerail rule pack: SQL, candidates

The ports in `rules/` here are still being evaluated: scored against the tools' expected findings and reported, never
gated and never shipped. This file is not published; the pack's [README](../README.md) covers the rules it ships.

## Candidates

`sql/sqlfluff` is gated (`upstream/sqlfluff/gates.yaml`). SQLFluff's own YAML cases, run as `dialect: postgres`,
`templater: raw` with the port's rule configuration (the engine reads every `.sql` file as PostgreSQL; a case SQLFluff's
postgres dialect cannot read whole, a Jinja case and a case configured otherwise left out), show why 1 port stays
a candidate:

| Port | Why |
|---|---|
| `sqlfluff-am02-ambiguous-union` | SQLFluff skips AM02 for PostgreSQL, so it reports nothing on its tests: the port's 7 findings are extra and it reproduces no upstream finding |

PG01 and PG02 ship off because they are PostgreSQL-only while Rerail reads every `.sql` file in one dialect. AM02 is the
reverse: SQLFluff skips it for PostgreSQL, so on a PostgreSQL file a bare `UNION` would be a false positive; it should
start off if it ever ships, to be turned on for MySQL or ANSI SQL.

## Scores

TP/FP/FN of each candidate against SQLFluff 4.4.0 on the fixtures, as the pack README's scores:

| Port | SQLFluff rule | Status | Fixtures | Skips files with syntax errors |
|---|---|---|---|---|
| `sqlfluff-am02-ambiguous-union` | AM02 | candidate | 5/0/0 (1 div.) | yes |

Over every port, shipped and candidate: of the 55 divergences, 54 are `not-comparable` (dialect readings, MySQL-only
and SQLite-only syntax, files PostgreSQL rejects, and SQLFluff's dialect gates for PG01 and AM02) and 1 is
`rerail-unknown` (a grammar gap in RF01: a redundantly bracketed join); the candidate holds 1 of them, AM02's.

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `sqlfluff-am02-ambiguous-union`

Port of SQLFluff 4.4.0 AM02 (ambiguous.union). Flags a UNION not immediately followed by DISTINCT or ALL. SQLFluff skips AM02 in its postgres dialect (it runs in ANSI, MySQL and other dialects with UNION DISTINCT; not SQLite), so the rule would start off and be turned on for MySQL/ANSI repositories (README already covers the start-off part).
