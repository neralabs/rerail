# SQLFluff 4.4.0: cases Rerail wrote

The fixtures listed below were written by Rerail for the pinned SQLFluff to score, beside the cases copied from upstream (`licenses/sqlfluff/UPSTREAM.md`): inputs the ports once misread. `sqlfluff/generate.py` holds them (`RERAIL_CASES`), writes them here and records SQLFluff's findings on them in `fixtures.expected.json`, checking each case's expected positions against the pinned SQLFluff. They are Rerail's (Apache-2.0, the pack's licence), so nothing goes under `licenses/`. Paths are relative to the pack directory.

| File | Origin |
|---|---|
| `upstream/sqlfluff/fixtures/rerail/postgres/AM05/fail_bare_join_lateral.sql` | written by Rerail |
| `upstream/sqlfluff/fixtures/rerail/postgres/AM05/pass_cross_join_lateral.sql` | written by Rerail |
| `upstream/sqlfluff/fixtures/rerail/postgres/AM05/pass_inner_join_lateral.sql` | written by Rerail |
| `upstream/sqlfluff/fixtures/rerail/postgres/AM05/pass_left_join_lateral.sql` | written by Rerail |
| `upstream/sqlfluff/fixtures/rerail/postgres/RF01/pass_insert_select_on_conflict_excluded.sql` | written by Rerail |
| `upstream/sqlfluff/fixtures/rerail/postgres/RF01/pass_insert_select_on_conflict_excluded_lower.sql` | written by Rerail |
| `upstream/sqlfluff/fixtures/rerail/postgres/RF04/fail_postgres_and_mysql_unreserved_column_names.sql` | written by Rerail |
| `upstream/sqlfluff/fixtures/rerail/postgres/RF04/pass_quoted_keyword_column_name.sql` | written by Rerail |
| `upstream/sqlfluff/fixtures/rerail/postgres/RF06/fail_quoted_index_column.sql` | written by Rerail |
| `upstream/sqlfluff/fixtures/rerail/postgres/RF06/fail_quoted_index_column_with_order.sql` | written by Rerail |
| `upstream/sqlfluff/fixtures/rerail/postgres/ST02/fail_naked_different_case_then.sql` | written by Rerail |
| `upstream/sqlfluff/fixtures/rerail/postgres/ST02/fail_qualified_quoted_part_same_case.sql` | written by Rerail |
| `upstream/sqlfluff/fixtures/rerail/postgres/ST02/fail_quoted_same_case_else.sql` | written by Rerail |
| `upstream/sqlfluff/fixtures/rerail/postgres/ST02/fail_quoted_same_case_then.sql` | written by Rerail |
| `upstream/sqlfluff/fixtures/rerail/postgres/ST02/pass_qualified_quoted_part_different_case.sql` | written by Rerail |
| `upstream/sqlfluff/fixtures/rerail/postgres/ST02/pass_quoted_and_naked_then.sql` | written by Rerail |
| `upstream/sqlfluff/fixtures/rerail/postgres/ST02/pass_quoted_different_case_else.sql` | written by Rerail |
| `upstream/sqlfluff/fixtures/rerail/postgres/ST02/pass_quoted_different_case_then.sql` | written by Rerail |
