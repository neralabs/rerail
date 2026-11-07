# sqlfluff, hand-written (batch next-sql)

21 rules, 539 lines of rules, plus `lib/sql/syntax.rr` (143 lines, new) and two generated keyword tables.

Scores are from `tools/rrlab/rrlab score sql <id> --all`, at (path, line, code). rrlab's own-fixture subset is empty
for this pack, because sqlfluff's fixtures sit under dialect directories (`ansi/AL04/…`, `postgres/PG01/…`). `--all`
scores every fixture, which is a superset. "ex" counts the differences that `divergences.json` already explains.

| rule | lines | TP / FP / FN | ex | deviations |
|---|---|---|---|---|
| sqlfluff-al04-aliasing-unique-table | 24 | 9 / 0 / 0 | | |
| sqlfluff-al05-aliasing-unused | 40 | 9 / 2* / 1 | 6 | 1, 2, 3 |
| sqlfluff-al08-aliasing-unique-column | 22 | 15 / 0 / 0 | | |
| sqlfluff-al09-aliasing-self-alias-column | 22 | 18 / 0 / 0 | | |
| sqlfluff-am01-ambiguous-distinct | 14 | 1 / 0 / 0 | | |
| sqlfluff-am05-ambiguous-join | 15 | 14 / 0 / 0 | | |
| sqlfluff-am09-ambiguous-order-by-limit | 26 | 3 / 0 / 0 | | |
| sqlfluff-cv02-convention-coalesce | 17 | 2 / 0 / 0 | | |
| sqlfluff-cv04-convention-count-rows | 19 | 3 / 0 / 0 | | |
| sqlfluff-cv05-convention-is-null | 15 | 7 / 0 / 0 | | |
| sqlfluff-cv08-convention-left-join | 12 | 9 / 0 / 0 | | |
| sqlfluff-pg01-postgres-excessive-locks | 25 | 10 / 0 / 0 | 1 | 4 |
| sqlfluff-pg02-postgres-not-valid-foreign-key | 18 | 1 / 0 / 0 | | 4 |
| sqlfluff-rf01-references-from | 46 | 31 / 0 / 0 | 1 | |
| sqlfluff-rf04-references-keywords | 33 | 4 / 0 / 0 | 3 | |
| sqlfluff-rf05-references-special-chars | 35 | 0 / 0 / 10 (ASCII: 12 / 0 / 0) | 4 | 5 |
| sqlfluff-rf06-references-quoting | 24 | 0 / 0 / 19 (ASCII: 21 / 0 / 0) | 34 | 4, 5 |
| sqlfluff-st01-structure-else-null | 15 | 4 / 0 / 0 | | |
| sqlfluff-st02-structure-simple-case | 47 | 15 / 0 / 0 | | |
| sqlfluff-st08-structure-distinct | 27 | 7 / 0 / 0 | | 7 |
| sqlfluff-st10-structure-constant-expression | 43 | 10 / 0 / 0 | | 6 |

`*` marks files with tree-sitter ERROR nodes. The engine withholds findings in those files; rrlab still scores them.

Data tables:
- `gen-keywords.py` writes `sqlfluff-rf04-references-keywords.facts.rr` (836 `sql_keyword` rows) and
  `sqlfluff-rf06-references-quoting.facts.rr` (102 `pg_reserved` rows).
- `check-keywords.py` round-trips both tables against the JSON.

## Deviations

1. **AL05, UNNEST outside PostgreSQL.** The FN is `ansi/ST08/test_pass_distinct_subquery_inside_count.sql:5`.
   sqlfluff accepts an unused alias on `UNNEST(…)` / `generate_series(…)` only in PostgreSQL, so in this ANSI file
   sqlfluff reports the alias and the rule does not. The rule exempts table functions in every dialect, as the JSON does, because one grammar serves
   every dialect and the engine does not know a file's dialect.
2. **AL05, how names compare.** Names compare as PostgreSQL resolves them (`ident_pg`). The exception is an alias
   in `'…'`, which parses only in MySQL and SQLite; those compare case-insensitively (`alias_key`). With that, the
   case-sensitive ANSI fixtures (`test_pass_naked_select_quoted_table` and the others) stay as `divergences.json`
   already explains them. The two FP* are parse-error files: `ansi/RF01/…_unknown_table.sql` and
   `sqlite/AL05/test_pass_sqlite_different_escaped_quotes.sql`.
3. **AL05, WINDOW.** `SELECT count(*) FROM t AS a WINDOW w AS (PARTITION BY a.x)`: sqlfluff does not look in a
   named WINDOW clause and reports `a`; the rule counts `a.x` there as a use and does not report. Checked against
   sqlfluff 4.4.0.
4. **Dialect.** PG01, PG02 and RF06 are PostgreSQL rules, and the engine reads every `.sql` file the same way. The
   explained differences are the non-PostgreSQL fixtures:
   - PG01: `ansi/PG01/test_pass_non_postgres_dialect_ignored.sql`;
   - RF06: the ANSI and MySQL fixtures, where folding goes to upper case or quoting uses backticks.
   RF06 follows PostgreSQL folding throughout.
5. **`\p{…}` classes.** RF05 and RF06 test Unicode letters with `\p{L}`, `\p{Ll}`, `\p{N}` and so on, which is
   what sqlfluff's regexes mean. rrlab's Soufflé uses std::regex, which rejects `\p`, so both rules score 0 TP
   there. ASCII copies of the two rules (the classes replaced by `[A-Za-z]` and similar; kept out of the tree)
   score RF05 12 / 0 / 0 and RF06 21 / 0 / 0. The rules keep `\p`, because the engine's matcher supports it.
6. **ST10, case.** `a = A` is reported: sqlfluff compares unquoted column references without case. Function
   calls and other expressions compare as written, so `f(a) = F(a)` is not reported, and `"a" = "A"` is not either.
   Checked against sqlfluff 4.4.0, as are the review fixes (RF01 `q.*`, RF05 `INSERT INTO t ("a b")`, CV04
   `x.count(1)`, ST01 `ELSE /* c */ NULL`, AL09 `t.a AS a`).
7. **ST08, regex flags.** `(?is:…)` is not accepted by rrlab, so the rule uses `(?i)` with `[\s\S]*`.

## Library and core change requests

- None for core or the existing libraries. `lib/sql/syntax.rr` is new; after review it also has `next_code`
  (from ST02, now used by ST01 too) and `table_function` (from AL04, now used by AL05 too).

## Harness gaps (rrlab)

- **Own fixtures.** Proposal: match a rule's code against the fixture directory under a dialect directory
  (`*/AL04/…`), not only the whole rule name.
- **Regex.** Proposal: translate `\p{L}`, `\p{Ll}`, `\p{Lu}`, `\p{Lm}`, `\p{Lo}` and `\p{N}` to a Unicode-aware
  matcher or to their ASCII subsets, and accept scoped flags `(?is:…)`, so that rules written for the engine score
  as written.
