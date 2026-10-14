# tree-sitter-sql: vendored with patches

Upstream: https://github.com/DerekStride/tree-sitter-sql, version 0.3.11 (commit
`7b51ecda191d36b92f5a90a8d1bc3faef1c7b8b8`), MIT. Rerail reads every `.sql` file with this one grammar
(`languages/sql.json`): PostgreSQL, SQLite, MySQL and ANSI files alike. The patches make it read what the
SQL pack's oracles hold: SQLFluff 4.4.0's rule fixtures (`packs/sql/upstream/sqlfluff/fixtures/`) and
PostgreSQL 18.6's names, values and arguments oracles (`fixtures/sql/{scope,matcher/arguments}/postgres.sql`).
The oracle inputs all parse without an `ERROR` node; 251 of the 265 SQLFluff fixtures do. The 14 that do not:

- a bare `[1]` array literal (BigQuery syntax): `ansi/ST10/test_pass_array_access.sql`;
- a redundantly bracketed join target: `ansi/RF01/test_{fail_redundantly_bracketed_join_target_unknown_table,pass_redundantly_bracketed_join_target_alias}.sql`;
- `CREATE POLICY` / `ALTER POLICY`: the five `postgres/RF01/test_*_postgres_{create,alter}_policy_*.sql`;
- `CREATE TRIGGER … BEGIN … END`: the three `sqlite/RF01/test_sqlite_create_trigger_after_*_reference.sql`;
- quoting the grammar reads differently: `ansi/AL05/test_pass_quoted_case_insensitive.sql`,
  `ansi/AL08/test_fail_locs.sql` and `sqlite/AL05/test_pass_sqlite_different_escaped_quotes.sql`.

A port that skips files with syntax errors skips these (`packs/sql/README.md`); each finding they affect is a
declared divergence or scored as a syntax-error file.

## Files

| File | Origin |
| --- | --- |
| `LICENSE` | upstream, verbatim |
| `src/grammar.json` | generated from the patched `grammar.js` |
| `src/scanner.c` | upstream, with patches 11 and 22 |
| `src/tree_sitter/{alloc,array,parser}.h` | written by `tree-sitter generate` 0.27.0 |
| `rerail.patch` | the patches: `git diff` of `grammar.js`, `src/scanner.c` and `test/corpus/` |

Upstream ships no `src/grammar.json` or `src/parser.c`; `native/build.rs` generates the parser from
`grammar.json` with tree-sitter-generate 0.27.0, as it does the other vendored grammars.

## Regenerating

```sh
git clone https://github.com/DerekStride/tree-sitter-sql
cd tree-sitter-sql
git checkout 7b51ecda191d36b92f5a90a8d1bc3faef1c7b8b8
git apply <rerail>/native/grammars/tree-sitter-sql/rerail.patch
tree-sitter generate   # tree-sitter-cli 0.27.0
tree-sitter test       # 429 of 429 (3 error-recovery cases skipped, below)
```

then copy `src/grammar.json`, `src/scanner.c` and `src/tree_sitter/*.h` here.

## Patches

Each patch has a case in `test/corpus/rerail.txt`, a file `rerail.patch` adds; `rerail.patch` holds
them all.

1. **One comment kind.** Block comments were `marginalia`; they are `comment`, as `--` comments are, so
   the engine's comment handling (`kind: "comment"`, the SQLFluff `noqa` index) reads both. Upstream's
   five marginalia cases now expect `(comment)`.
2. **A join without `ON` or `USING`** (`FROM foo JOIN bar`), which SQLFluff's ANSI dialect reads:
   `ansi/AM05/*_default.sql`, `ansi/AL04/test_fail_subquery_join_matches_base_table.sql`.
3. **`UNION | INTERSECT | EXCEPT DISTINCT`**: `ansi/AM02/*`.
4. **`REINDEX` and `REFRESH MATERIALIZED VIEW [CONCURRENTLY]`** (`reindex_statement`,
   `refresh_statement`): `postgres/PG01/*reindex*`, `*refresh*`.
5. **`NOT VALID`** after an added constraint: `postgres/PG02/*not_valid*`.
6. **Doubled quotes inside quoted identifiers** (`"x""y"`, `` `p``q` ``):
   `mysql/AL09/test_pass_mysql_quoted_identifiers.sql`.
7. **`$` inside an identifier** after its first character (PostgreSQL):
   `postgres/RF05/test_pass_ignore_lists_unquoted_fail.sql`.
8. **A string alias after `AS`** (`a AS 'alias'`, MySQL and SQLite):
   `ansi/RF05/test_pass_ignore_lists_quoted_fail.sql`. Without `AS` a string after a term stays a string
   continuation (`U&'..'\n'..'`).
9. **`EXCLUDE [USING method] (element WITH operator, ...)`** table constraints:
   `postgres/CV05/test_exclude_constraint.sql`.
10. **MySQL `SET a = 1, b = @@var`**: several assignments, `@@` system variables (`system_variable`):
    `mysql/CV05/test_mysql_system_variable.sql`.
11. **Nested block comments**, as PostgreSQL's scanner and the SQL standard read them: `comment` is an
    external token scanned in `scanner.c` (`scan_comment`), so `/* a /* b */ c */` is one comment
    (`fixtures/sql/suppress/sqlfluff/postgres_bodies.sql` line 8).
12. **Named arguments** `f(x => 1)`, `f(x := 1)` (`named_argument`, fields `name` and `value`); upstream
    read `x =` then `> 1`.
13. **A double-quoted token in an expression is an identifier** (ANSI SQL, PostgreSQL, SQLite), never a
    string: `literal` takes `_expression_string` (single-quoted, dollar-quoted and `E''` strings);
    statement slots that take a string (`COMMENT ... IS`, `SEPARATOR`, options) keep accepting `"..."`.
    Upstream cases that read MySQL's double-quoted strings in expressions now expect `field`
    (`Mixed Predicate types`, `Disjunctive Predicate`, `GROUP CONCAT Impala`, the INSERT cases,
    `JSON traversal`, `Simple select with where and string`, `SELECT AS UNSIGNED`).
14. **Exponents in either case** (`4664.E+5`, `1E3`).
15. **`ON CONFLICT (target) [WHERE ...]` and `ON CONFLICT ON CONSTRAINT name`** (PostgreSQL).
16. **`DELETE FROM t [alias] USING from_item, ...`** (PostgreSQL).
17. (none: a `LATERAL` subquery in a comma-separated `FROM` list still fails; upstream reads only
    `CROSS JOIN LATERAL` and `JOIN LATERAL`.)
18. **`E''` strings with any backslash escape and `''`, and `UESCAPE`** after `U&'..'` and `U&".."`.
19. **PostgreSQL's SQL-syntax calls** `substring(s FROM a FOR b)`, `substring(s FOR b)`,
    `overlay(s PLACING t FROM a FOR b)`, `position(a IN s)`, `trim(BOTH | LEADING | TRAILING [x] FROM s)`:
    their operands are `parameter`s. MySQL's `EXTRACT(unit FROM x)` form keeps priority.
20. **`ALTER TABLE ... VALIDATE CONSTRAINT name`** (`validate_constraint`):
    `postgres/PG02/test_pass_add_foreign_key_not_valid_then_validate.sql`.
21. **Any non-ASCII character in an identifier**, as PostgreSQL's scanner reads them (`scan.l`'s
    `ident_start` and `ident_cont` take every byte from `\200` to `\377`, so every UTF-8 encoded
    non-ASCII character); upstream took `U+00C0` to `U+017F` only. A code rule's pattern metavariable
    (`$A`, read as `µA`, the expando ast-grep and Rerail's patterns share) is then an identifier.
22. **Dollar-quoted strings close at their own tag** (`scanner.c`). Upstream stored a tag's code points
    as `char`, so `$š$` (U+0161) read as `$a$` and closed at an embedded `$a$`; and it scanned each
    `$...$` candidate whole, so a candidate that failed consumed the `$` opening the real closing tag
    (`$a$foo$bar$a$`, the string `foo$bar`, never closed). Tags are now whole code points, at most 255
    of them (a longer one is not read as a tag, so the start tag a function body leaves open always
    serializes), compared by length and content; the closing tag is matched one code point at a time,
    a failed candidate leaving a `$` it stopped at to start the next. The scanner state is a fixed
    struct, serialized as a flag byte and the tag's code points.

Three upstream error-recovery cases (`errors.txt`: `Function body with a $$ string literal with the same
name`, `... a unbalanced $$`, `... a tag name contains whitespace`) are marked `:skip`: they assert the
shape of the `ERROR` nodes upstream's parser recovers with, which the patched grammar recovers differently;
each still parses with an `ERROR`.
