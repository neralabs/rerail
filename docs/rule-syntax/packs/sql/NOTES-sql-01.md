# sql-01: AL04, AL05, AL08, RF01, RF05 against their originals

Checked with `translations:check --pack sql --rules <id>` (ptcheck) on the fixtures and samples-sql corpora; ground
truth is sqlfluff 4.5.0 (`uvx sqlfluff lint --dialect <the fixture's dialect directory> --rules <code>`).

| rule | status | fixtures orig-only / trans-only | samples-sql orig-only / trans-only |
|---|---|---|---|
| sqlfluff-al04-aliasing-unique-table | same | 0 / 0 | 0 / 0 |
| sqlfluff-al05-aliasing-unused | same | 0 / 0 | 0 / 0 |
| sqlfluff-al08-aliasing-unique-column | different, translation right | 0 / 1 | 0 / 0 |
| sqlfluff-rf01-references-from | different, translation right | 0 / 4 | 0 / 0 |
| sqlfluff-rf05-references-special-chars | different, translation right | 0 / 2 | 0 / 0 |

## `earlier` is empty on SQL (AL04, AL08)

Both rules picked the later of two equal names with core's `earlier`. The engine fills `visit_index`, which `earlier`
is lowered to, for Python only, so on a `.sql` file `earlier` holds nowhere: a probe on the AL08 fixtures gave no
`visit_index` row and no `earlier` pair between any two identifiers. AL08 reported nothing at all (14 + 39 findings
missed), AL04 missed every repeat inside one FROM clause (`select 1 from t1 as a join t2 as a using (pk)`).
`lib/sql/sql-01.rerail` gives `src_before(m, n)`, start-offset order read off `node`, which is what core documents
`earlier` to be outside Python; both rules use it and are now right on every fixture. The same gap was found for
JavaScript/TypeScript in next-01 (`lib/tsx/next-01.rerail`).

Engine change proposed: fill `visit_index` (start-offset order) for every tree-sitter language, as core.rerail says.
Then `lib/sql/sql-01.rerail` goes and AL04/AL08 say `earlier` again.

## The remaining differences: files the original refuses

Every difference left is in a fixture tree-sitter-sql cannot parse whole (it has an ERROR node). The original's
product path refuses such a file ("source-incomplete: file has syntax errors": 8 fixtures), so it reports nothing
there; `run-policies` does not withhold the file, and the translation reports what it can read. sqlfluff parses
these files and reports what the translation reports:

- AL08, `sqlite/AL05/test_pass_sqlite_different_escaped_quotes.sql:3`: `SELECT "…".id, '…'.id FROM …`. sqlfluff
  (sqlite): `AL08 Reuse of column alias 'id' from line 2` at line 3.
- RF01, `ansi/AL05/test_pass_quoted_case_insensitive.sql:3` and `:6`: `FROM foo.bar a INNER JOIN foo.qux 'B'` with
  `b.bar`, `b.thing`. sqlfluff (ansi) reports RF01 at both lines: it matches names as written, so `b` is not `'B'`.
- RF01, `ansi/RF01/test_fail_redundantly_bracketed_join_target_unknown_table.sql:1`: `SELECT z.x FROM tbl_a AS a INNER
  JOIN ((tbl_b AS b …))`. sqlfluff reports `z.x` (a test_fail case of RF01 itself).
- RF01, `sqlite/AL05/test_pass_sqlite_different_escaped_quotes.sql:2`: sqlfluff (sqlite) reports RF01 at lines 2 and
  3; the translation reports line 2. Line 3's qualifier `` '`''a"' `` is a string with an ERROR after it in
  tree-sitter-sql, so neither side reports line 3.
- RF05, the same file, lines 2 and 4: sqlfluff (sqlite) reports RF05 at 2, 3 and 4; line 3 is the same unparsed
  `'…'` qualifier, so neither side reports it.

## AL05: a statement tree-sitter could not parse is left alone

Two translation-only AL05 findings were wrong, both in statements with an ERROR node: the alias's use sat in the
part tree-sitter lost.
- `ansi/RF01/…_unknown_table.sql:2`: `FROM tbl_a AS a INNER JOIN ((tbl_b AS b INNER JOIN tbl_c AS c ON …)) ON a.id =
  b.id`; tree-sitter-sql makes the join an ERROR and drops `a.id`. sqlfluff (ansi) reports no AL05.
- `sqlite/AL05/test_pass_sqlite_different_escaped_quotes.sql:4`: the alias ```` ```'A"` ```` is used by
  `` '`''a"'.id ``, which tree-sitter reads as a string and an ERROR. sqlfluff (sqlite) reports no AL05.
The rule now skips an alias whose statement has an ERROR node in it or right after it (`unparsed`), as RF04 does.
Comparing backtick-quoted names without case would also have cleared the second one, but sqlfluff does that in
SQLite and BigQuery and not in MySQL (``SELECT `A`.x FROM t AS `a` ``: mysql reports AL05, sqlite does not), and the
engine does not know a file's dialect.

## RF01: a LATERAL join's alias

`samples-sql …/stats_import.sql:915`: `SELECT …, r.* FROM pg_catalog.pg_stats AS s CROSS JOIN LATERAL
pg_catalog.pg_restore_attribute_stats(…) AS r` was reported at `r`. tree-sitter-sql puts the alias of a lateral
function or subquery on the `lateral_cross_join` / `lateral_join` node itself, with no `relation` under it, so the
rule's `table_ref` never saw `r`. sqlfluff (postgres) reports nothing there; `table_ref` now takes that alias.

## Library and engine requests

- New: `lib/sql/sql-01.rerail` (`src_before`), shared by AL04 and AL08, until the engine fills `visit_index` for SQL.
- Engine: `run-policies` reports findings in SQL files with tree-sitter ERROR nodes, though core.rerail says such a
  document is withheld (NOTES-sql.md says so too). The original's product path refuses them, so the suite compares
  translation findings on these files with nothing; the differences above are recorded, not hidden.
