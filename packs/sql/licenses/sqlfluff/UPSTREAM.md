# SQLFluff 4.4.0

Upstream: [sqlfluff/sqlfluff](https://github.com/sqlfluff/sqlfluff/tree/0348222a3bcdb0226563f7349c5a39beb0641955) at tag `4.4.0` (commit `0348222a3bcdb0226563f7349c5a39beb0641955`). License: MIT (Copyright (c) 2018-2026 Alan Cruickshank); [`LICENSE.md`](LICENSE.md) is the upstream license file, copied verbatim at that tag.

The files listed below are SQL strings taken from the YAML test cases of the SQLFluff rules the SQL pack ports (the pack's `conformance.json`) and remain under SQLFluff's license. Each file is one case's `pass_str` or `fail_str`, written byte for byte; the case's name is the file name. Every other file under `upstream/sqlfluff/`, including the cases Rerail wrote itself (`fixtures/rerail/`), is Rerail's, under the Apache License 2.0 (the pack's `LICENSE`): `generate.py` and the recordings it writes by running SQLFluff (`fixtures.expected.json`, `fixtures.cases.json`, `corpus-*.expected.json`).

Paths are relative to the pack directory, `packs/sql/`.

| File | Upstream path | Copy |
|---|---|---|
| `upstream/sqlfluff/fixtures/ansi/AL04/test_fail_exactly_once_duplicated_aliases.sql` | `test/fixtures/rules/std_rule_cases/AL04.yml`, case `test_fail_exactly_once_duplicated_aliases` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL04/test_fail_two_duplicated_aliases.sql` | `test/fixtures/rules/std_rule_cases/AL04.yml`, case `test_fail_two_duplicated_aliases` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL04/test_fail_subquery.sql` | `test/fixtures/rules/std_rule_cases/AL04.yml`, case `test_fail_subquery` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL04/test_pass_subquery.sql` | `test/fixtures/rules/std_rule_cases/AL04.yml`, case `test_pass_subquery` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL04/test_fail_subquery_same_alias.sql` | `test/fixtures/rules/std_rule_cases/AL04.yml`, case `test_fail_subquery_same_alias` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL04/test_fail_subquery_alias_matches_base_table.sql` | `test/fixtures/rules/std_rule_cases/AL04.yml`, case `test_fail_subquery_alias_matches_base_table` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL04/test_fail_subquery_table_matches_base_table.sql` | `test/fixtures/rules/std_rule_cases/AL04.yml`, case `test_fail_subquery_table_matches_base_table` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL04/test_fail_subquery_join_matches_base_table.sql` | `test/fixtures/rules/std_rule_cases/AL04.yml`, case `test_fail_subquery_join_matches_base_table` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_fail_table_alias_not_referenced_1.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_fail_table_alias_not_referenced_1` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_fail_table_alias_not_referenced_1_subquery.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_fail_table_alias_not_referenced_1_subquery` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_table_alias_referenced_subquery.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_table_alias_referenced_subquery` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_table_alias_referenced.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_table_alias_referenced` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_unaliased_table_referenced.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_unaliased_table_referenced` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/AL05/test_ignore_postgres_value_table_functions.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_ignore_postgres_value_table_functions` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/AL05/test_ignore_postgres_value_table_functions_generate_series.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_ignore_postgres_value_table_functions_generate_series` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/AL05/test_postgres_row_reference_in_function.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_postgres_row_reference_in_function` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/AL05/test_postgres_whole_row_reference.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_postgres_whole_row_reference` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/AL05/test_postgres_row_reference_in_subquery.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_postgres_row_reference_in_subquery` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_fail_table_alias_not_referenced_2.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_fail_table_alias_not_referenced_2` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_fail_table_alias_not_referenced_2_subquery.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_fail_table_alias_not_referenced_2_subquery` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_subquery_alias_not_referenced.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_subquery_alias_not_referenced` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_subselect_uses_alias_1.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_subselect_uses_alias_1` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_subselect_uses_alias_2.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_subselect_uses_alias_2` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_subselect_uses_alias_3.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_subselect_uses_alias_3` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_ansi_function_not_table_parameter.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_ansi_function_not_table_parameter` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_join_on_expression_in_parentheses.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_join_on_expression_in_parentheses` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_derived_query_requires_alias_1.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_derived_query_requires_alias_1` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_derived_query_requires_alias_2.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_derived_query_requires_alias_2` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_derived_query_requires_alias_3.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_derived_query_requires_alias_3` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/AL05/test_pass_postgres_values_clause.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_postgres_values_clause` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_naked_select_single_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_naked_select_single_quoted_table` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_fail_naked_select_single_quoted_table_wrong_case.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_fail_naked_select_single_quoted_table_wrong_case` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_naked_select_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_naked_select_quoted_table` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_quoted_select_naked_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_quoted_select_naked_table` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_naked_select_quoted_table_subquery.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_naked_select_quoted_table_subquery` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_quoted_select_naked_table_subquery.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_quoted_select_naked_table_subquery` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/mysql/AL05/test_pass_mysql_quoted_select_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_mysql_quoted_select_quoted_table` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/mysql/AL05/test_pass_mysql_naked_select_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_mysql_naked_select_quoted_table` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/mysql/AL05/test_pass_mysql_quoted_select_naked_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_mysql_quoted_select_naked_table` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/AL05/test_pass_postgres_naked_select_double_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_postgres_naked_select_double_quoted_table` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/AL05/test_fail_postgres_naked_select_double_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_fail_postgres_naked_select_double_quoted_table` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/AL05/test_pass_postgres_quoted_select_naked_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_postgres_quoted_select_naked_table` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/AL05/test_pass_postgres_quoted_select_single_quote_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_postgres_quoted_select_single_quote_table` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/sqlite/AL05/test_pass_sqlite_naked_select_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_sqlite_naked_select_quoted_table` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/sqlite/AL05/test_pass_sqlite_quoted_select_naked_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_sqlite_quoted_select_naked_table` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/sqlite/AL05/test_pass_sqlite_naked_select_quoted_table_subquery.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_sqlite_naked_select_quoted_table_subquery` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/sqlite/AL05/test_pass_sqlite_quoted_select_naked_table_subquery.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_sqlite_quoted_select_naked_table_subquery` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/sqlite/AL05/test_pass_sqlite_different_escaped_quotes.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_sqlite_different_escaped_quotes` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_repeat_referenced_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_repeat_referenced_table` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_case_insensitive.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_case_insensitive` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_quoted_case_insensitive.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_quoted_case_insensitive` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_fail_naked_select_double_quoted_table_wrong_case_cs.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_fail_naked_select_double_quoted_table_wrong_case_qcs_nlow.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_fail_naked_select_double_quoted_table_wrong_case_qcs_nup.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_fail_naked_select_single_quoted_table_wrong_case_cs.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_fail_naked_select_single_quoted_table_wrong_case_qcs_nlow.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_fail_naked_select_single_quoted_table_wrong_case_qcs_nup.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_naked_select_double_quoted_table_ci.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_naked_select_double_quoted_table_lower_cs.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_naked_select_double_quoted_table_qcs_nlow.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_naked_select_double_quoted_table_qcs_nup.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_naked_select_double_quoted_table_upper_cs.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_naked_select_single_quoted_table_ci.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_naked_select_single_quoted_table_lower_cs.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_naked_select_single_quoted_table_qcs_nlow.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_naked_select_single_quoted_table_qcs_nup.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL05/test_pass_naked_select_single_quoted_table_upper_cs.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/postgres/AL05/test_pass_postgres_different_quoted_case_insensitive.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_postgres_different_quoted_case_insensitive` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/AL05/test_pass_postgres_jsonb_array_elements_text.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_postgres_jsonb_array_elements_text` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/AL05/test_pass_postgres_json_array_elements.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_postgres_json_array_elements` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/AL05/test_pass_postgres_wildcard_row_passed_to_function.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml`, case `test_pass_postgres_wildcard_row_passed_to_function` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL08/test_fail_references.sql` | `test/fixtures/rules/std_rule_cases/AL08.yml`, case `test_fail_references` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL08/test_fail_aliases.sql` | `test/fixtures/rules/std_rule_cases/AL08.yml`, case `test_fail_aliases` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL08/test_fail_alias_refs.sql` | `test/fixtures/rules/std_rule_cases/AL08.yml`, case `test_fail_alias_refs` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL08/test_fail_locs.sql` | `test/fixtures/rules/std_rule_cases/AL08.yml`, case `test_fail_locs` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL08/test_fail_alias_case.sql` | `test/fixtures/rules/std_rule_cases/AL08.yml`, case `test_fail_alias_case` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL08/test_fail_qualified.sql` | `test/fixtures/rules/std_rule_cases/AL08.yml`, case `test_fail_qualified` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL08/test_pass_table_names.sql` | `test/fixtures/rules/std_rule_cases/AL08.yml`, case `test_pass_table_names` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL09/test_pass_no_self_alias.sql` | `test/fixtures/rules/std_rule_cases/AL09.yml`, case `test_pass_no_self_alias` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL09/test_fail_no_quotes_same_case.sql` | `test/fixtures/rules/std_rule_cases/AL09.yml`, case `test_fail_no_quotes_same_case` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL09/test_fail_with_quotes_same_case.sql` | `test/fixtures/rules/std_rule_cases/AL09.yml`, case `test_fail_with_quotes_same_case` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL09/test_fail_different_case.sql` | `test/fixtures/rules/std_rule_cases/AL09.yml`, case `test_fail_different_case` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AL09/test_pass_different_quotes.sql` | `test/fixtures/rules/std_rule_cases/AL09.yml`, case `test_pass_different_quotes` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/mysql/AL09/test_pass_mysql_quoted_identifiers.sql` | `test/fixtures/rules/std_rule_cases/AL09.yml`, case `test_pass_mysql_quoted_identifiers` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM01/test_pass_only_group_by.sql` | `test/fixtures/rules/std_rule_cases/AM01.yml`, case `test_pass_only_group_by` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM01/test_fail_distinct_and_group_by.sql` | `test/fixtures/rules/std_rule_cases/AM01.yml`, case `test_fail_distinct_and_group_by` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM02/test_pass_union_all.sql` | `test/fixtures/rules/std_rule_cases/AM02.yml`, case `test_pass_union_all` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM02/test_fail_bare_union.sql` | `test/fixtures/rules/std_rule_cases/AM02.yml`, case `test_fail_bare_union` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM02/test_pass_union_distinct.sql` | `test/fixtures/rules/std_rule_cases/AM02.yml`, case `test_pass_union_distinct` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM02/test_pass_union_distinct_with_comment.sql` | `test/fixtures/rules/std_rule_cases/AM02.yml`, case `test_pass_union_distinct_with_comment` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM02/test_fail_triple_join_with_one_bad.sql` | `test/fixtures/rules/std_rule_cases/AM02.yml`, case `test_fail_triple_join_with_one_bad` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM02/test_fail_triple_join_with_one_bad_lowercase.sql` | `test/fixtures/rules/std_rule_cases/AM02.yml`, case `test_fail_triple_join_with_one_bad_lowercase` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/AM02/test_postgres.sql` | `test/fixtures/rules/std_rule_cases/AM02.yml`, case `test_postgres` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM02/test_fail_mixed_case_union_preserves_case.sql` | `test/fixtures/rules/std_rule_cases/AM02.yml`, case `test_fail_mixed_case_union_preserves_case` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_fail_lone_join_default.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml`, case `test_fail_lone_join_default` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_fail_lone_join_lowercase_default.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml`, case `test_fail_lone_join_lowercase_default` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_inner_join_default.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml`, case `test_pass_inner_join_default` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_left_join_default.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml`, case `test_pass_left_join_default` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_right_join_default.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml`, case `test_pass_right_join_default` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_full_join_default.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml`, case `test_pass_full_join_default` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_left_outer_join_default.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml`, case `test_pass_left_outer_join_default` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_right_outer_join_default.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml`, case `test_pass_right_outer_join_default` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_full_outer_join_default.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml`, case `test_pass_full_outer_join_default` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_cross_join_default.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml`, case `test_pass_cross_join_default` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_fail_full_join_both.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_fail_full_join_lowercase_both.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_fail_full_join_lowercase_outer.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_fail_full_join_outer.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_fail_left_join_both.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_fail_left_join_outer.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_fail_lone_join_both.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_fail_lone_join_inner.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_fail_lone_join_lowercase_both.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_fail_lone_join_lowercase_inner.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_fail_right_join_both.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_fail_right_join_outer.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_full_join_inner.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_full_outer_join_both.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_full_outer_join_inner.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_full_outer_join_outer.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_inner_join_both.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_inner_join_inner.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_inner_join_outer.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_left_join_inner.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_left_outer_join_both.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_left_outer_join_inner.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_left_outer_join_outer.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_lone_join_outer.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_right_join_inner.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_right_outer_join_both.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_right_outer_join_inner.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM05/test_pass_right_outer_join_outer.sql` | `test/fixtures/rules/std_rule_cases/AM05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM09/test_fail_limit_without_order_by.sql` | `test/fixtures/rules/std_rule_cases/AM09.yml`, case `test_fail_limit_without_order_by` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM09/test_fail_limit_and_offset_without_order_by.sql` | `test/fixtures/rules/std_rule_cases/AM09.yml`, case `test_fail_limit_and_offset_without_order_by` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM09/test_pass_limit_with_order_by.sql` | `test/fixtures/rules/std_rule_cases/AM09.yml`, case `test_pass_limit_with_order_by` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/AM09/test_pass_limit_and_offset_with_order_by.sql` | `test/fixtures/rules/std_rule_cases/AM09.yml`, case `test_pass_limit_and_offset_with_order_by` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM09/test_pass_no_limit_or_offset.sql` | `test/fixtures/rules/std_rule_cases/AM09.yml`, case `test_pass_no_limit_or_offset` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM09/test_fail_limit_with_order_by_in_subquery.sql` | `test/fixtures/rules/std_rule_cases/AM09.yml`, case `test_fail_limit_with_order_by_in_subquery` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/AM09/test_pass_limit_with_order_by_in_subquery.sql` | `test/fixtures/rules/std_rule_cases/AM09.yml`, case `test_pass_limit_with_order_by_in_subquery` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV02/test_pass_coalesce.sql` | `test/fixtures/rules/std_rule_cases/CV02.yml`, case `test_pass_coalesce` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV02/test_pass_qualified_udf.sql` | `test/fixtures/rules/std_rule_cases/CV02.yml`, case `test_pass_qualified_udf` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV02/test_fail_ifnull.sql` | `test/fixtures/rules/std_rule_cases/CV02.yml`, case `test_fail_ifnull` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV02/test_fail_nvl.sql` | `test/fixtures/rules/std_rule_cases/CV02.yml`, case `test_fail_nvl` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV04/passes_on_count_star.sql` | `test/fixtures/rules/std_rule_cases/CV04.yml`, case `passes_on_count_star` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV04/changes_count_0_to_count_star.sql` | `test/fixtures/rules/std_rule_cases/CV04.yml`, case `changes_count_0_to_count_star` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV04/changes_count_1_to_count_star.sql` | `test/fixtures/rules/std_rule_cases/CV04.yml`, case `changes_count_1_to_count_star` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV04/handles_whitespaces.sql` | `test/fixtures/rules/std_rule_cases/CV04.yml`, case `handles_whitespaces` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV04/no_false_positive_on_count_col.sql` | `test/fixtures/rules/std_rule_cases/CV04.yml`, case `no_false_positive_on_count_col` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV04/no_false_positive_on_expression.sql` | `test/fixtures/rules/std_rule_cases/CV04.yml`, case `no_false_positive_on_expression` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV04/changes_count_0_to_count_1.sql` | `test/fixtures/rules/std_rule_cases/CV04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV04/changes_count_1_to_count_0.sql` | `test/fixtures/rules/std_rule_cases/CV04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV04/changes_count_star_to_count_0.sql` | `test/fixtures/rules/std_rule_cases/CV04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV04/changes_count_star_to_count_1.sql` | `test/fixtures/rules/std_rule_cases/CV04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV04/changes_count_star_to_count_1_handle_new_line.sql` | `test/fixtures/rules/std_rule_cases/CV04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV04/changes_to_count_1_if_both_present.sql` | `test/fixtures/rules/std_rule_cases/CV04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV04/passes_on_count_0.sql` | `test/fixtures/rules/std_rule_cases/CV04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV04/passes_on_count_1.sql` | `test/fixtures/rules/std_rule_cases/CV04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV04/passes_on_count_1_if_both_present.sql` | `test/fixtures/rules/std_rule_cases/CV04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV05/test_is_null.sql` | `test/fixtures/rules/std_rule_cases/CV05.yml`, case `test_is_null` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV05/test_is_not_null.sql` | `test/fixtures/rules/std_rule_cases/CV05.yml`, case `test_is_not_null` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV05/test_not_equals_null_upper.sql` | `test/fixtures/rules/std_rule_cases/CV05.yml`, case `test_not_equals_null_upper` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV05/test_not_equals_null_multi_nulls.sql` | `test/fixtures/rules/std_rule_cases/CV05.yml`, case `test_not_equals_null_multi_nulls` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV05/test_not_equals_null_lower.sql` | `test/fixtures/rules/std_rule_cases/CV05.yml`, case `test_not_equals_null_lower` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV05/test_equals_null_spaces.sql` | `test/fixtures/rules/std_rule_cases/CV05.yml`, case `test_equals_null_spaces` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV05/test_equals_null_no_spaces.sql` | `test/fixtures/rules/std_rule_cases/CV05.yml`, case `test_equals_null_no_spaces` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV05/test_complex_case_1.sql` | `test/fixtures/rules/std_rule_cases/CV05.yml`, case `test_complex_case_1` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV05/test_set_clause.sql` | `test/fixtures/rules/std_rule_cases/CV05.yml`, case `test_set_clause` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/CV05/test_exclude_constraint.sql` | `test/fixtures/rules/std_rule_cases/CV05.yml`, case `test_exclude_constraint` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/mysql/CV05/test_mysql_system_variable.sql` | `test/fixtures/rules/std_rule_cases/CV05.yml`, case `test_mysql_system_variable` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV08/test_fail_right_join.sql` | `test/fixtures/rules/std_rule_cases/CV08.yml`, case `test_fail_right_join` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV08/test_pass_left_join.sql` | `test/fixtures/rules/std_rule_cases/CV08.yml`, case `test_pass_left_join` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV08/test_pass_inner_join.sql` | `test/fixtures/rules/std_rule_cases/CV08.yml`, case `test_pass_inner_join` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV08/test_fail_right_and_right_join.sql` | `test/fixtures/rules/std_rule_cases/CV08.yml`, case `test_fail_right_and_right_join` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV08/test_fail_right_and_left_join.sql` | `test/fixtures/rules/std_rule_cases/CV08.yml`, case `test_fail_right_and_left_join` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV08/test_fail_right_and_inner_join.sql` | `test/fixtures/rules/std_rule_cases/CV08.yml`, case `test_fail_right_and_inner_join` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV08/test_pass_left_inner_join.sql` | `test/fixtures/rules/std_rule_cases/CV08.yml`, case `test_pass_left_inner_join` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV08/test_fail_subquery_right_join.sql` | `test/fixtures/rules/std_rule_cases/CV08.yml`, case `test_fail_subquery_right_join` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV08/test_pass_subquery_left_join.sql` | `test/fixtures/rules/std_rule_cases/CV08.yml`, case `test_pass_subquery_left_join` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV08/test_pass_subquery_inner_join.sql` | `test/fixtures/rules/std_rule_cases/CV08.yml`, case `test_pass_subquery_inner_join` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV08/test_fail_with_right_join.sql` | `test/fixtures/rules/std_rule_cases/CV08.yml`, case `test_fail_with_right_join` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV08/test_pass_with_left_join.sql` | `test/fixtures/rules/std_rule_cases/CV08.yml`, case `test_pass_with_left_join` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/CV08/test_pass_with_inner_join.sql` | `test/fixtures/rules/std_rule_cases/CV08.yml`, case `test_pass_with_inner_join` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/PG01/test_pass_create_index_concurrently.sql` | `test/fixtures/rules/std_rule_cases/PG01.yml`, case `test_pass_create_index_concurrently` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/PG01/test_pass_create_index_concurrently_if_not_exists.sql` | `test/fixtures/rules/std_rule_cases/PG01.yml`, case `test_pass_create_index_concurrently_if_not_exists` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/PG01/test_pass_unique_index_without_concurrently.sql` | `test/fixtures/rules/std_rule_cases/PG01.yml`, case `test_pass_unique_index_without_concurrently` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/PG01/test_fail_create_index_without_concurrently.sql` | `test/fixtures/rules/std_rule_cases/PG01.yml`, case `test_fail_create_index_without_concurrently` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/PG01/test_fail_create_index_if_not_exists_without_concurrently.sql` | `test/fixtures/rules/std_rule_cases/PG01.yml`, case `test_fail_create_index_if_not_exists_without_concurrently` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/PG01/test_fail_create_index_on_jsonb_without_concurrently.sql` | `test/fixtures/rules/std_rule_cases/PG01.yml`, case `test_fail_create_index_on_jsonb_without_concurrently` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/PG01/test_pass_create_index_on_jsonb_with_concurrently.sql` | `test/fixtures/rules/std_rule_cases/PG01.yml`, case `test_pass_create_index_on_jsonb_with_concurrently` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/PG01/test_pass_drop_index_concurrently.sql` | `test/fixtures/rules/std_rule_cases/PG01.yml`, case `test_pass_drop_index_concurrently` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/PG01/test_fail_drop_index_without_concurrently.sql` | `test/fixtures/rules/std_rule_cases/PG01.yml`, case `test_fail_drop_index_without_concurrently` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/PG01/test_fail_drop_index_if_exists_without_concurrently.sql` | `test/fixtures/rules/std_rule_cases/PG01.yml`, case `test_fail_drop_index_if_exists_without_concurrently` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/PG01/test_pass_reindex_concurrently.sql` | `test/fixtures/rules/std_rule_cases/PG01.yml`, case `test_pass_reindex_concurrently` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/PG01/test_fail_reindex_without_concurrently.sql` | `test/fixtures/rules/std_rule_cases/PG01.yml`, case `test_fail_reindex_without_concurrently` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/PG01/test_fail_reindex_table_without_concurrently.sql` | `test/fixtures/rules/std_rule_cases/PG01.yml`, case `test_fail_reindex_table_without_concurrently` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/PG01/test_pass_refresh_materialized_view_concurrently.sql` | `test/fixtures/rules/std_rule_cases/PG01.yml`, case `test_pass_refresh_materialized_view_concurrently` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/PG01/test_fail_refresh_materialized_view_without_concurrently.sql` | `test/fixtures/rules/std_rule_cases/PG01.yml`, case `test_fail_refresh_materialized_view_without_concurrently` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/PG01/test_pass_non_postgres_dialect_ignored.sql` | `test/fixtures/rules/std_rule_cases/PG01.yml`, case `test_pass_non_postgres_dialect_ignored` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/PG02/test_pass_alter_table_add_column.sql` | `test/fixtures/rules/std_rule_cases/PG02.yml`, case `test_pass_alter_table_add_column` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/PG02/test_pass_alter_table_drop_column.sql` | `test/fixtures/rules/std_rule_cases/PG02.yml`, case `test_pass_alter_table_drop_column` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/PG02/test_pass_add_foreign_key_not_valid.sql` | `test/fixtures/rules/std_rule_cases/PG02.yml`, case `test_pass_add_foreign_key_not_valid` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/PG02/test_pass_add_foreign_key_not_valid_then_validate.sql` | `test/fixtures/rules/std_rule_cases/PG02.yml`, case `test_pass_add_foreign_key_not_valid_then_validate` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/PG02/test_fail_add_foreign_key_without_not_valid.sql` | `test/fixtures/rules/std_rule_cases/PG02.yml`, case `test_fail_add_foreign_key_without_not_valid` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/PG02/test_pass_add_non_fk_constraint_without_not_valid.sql` | `test/fixtures/rules/std_rule_cases/PG02.yml`, case `test_pass_add_non_fk_constraint_without_not_valid` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF01/test_fail_object_not_referenced_2.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_fail_object_not_referenced_2` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF01/test_pass_object_referenced_2.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_pass_object_referenced_2` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF01/test_pass_object_referenced_3.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_pass_object_referenced_3` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF01/test_pass_object_referenced_5b.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_pass_object_referenced_5b` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF01/test_pass_object_referenced_6.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_pass_object_referenced_6` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF01/test_pass_object_referenced_7.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_pass_object_referenced_7` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF01/test_fail_object_referenced_7.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_fail_object_referenced_7` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF01/test_pass_object_referenced_8.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_pass_object_referenced_8` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF01/test_pass_two_part_reference_8.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_pass_two_part_reference_8` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF01/test_pass_two_part_reference_9.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_pass_two_part_reference_9` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF01/test_fail_two_part_reference_10.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_fail_two_part_reference_10` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF01/test_fail_two_part_reference_11.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_fail_two_part_reference_11` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF01/test_pass_two_part_reference_11.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_pass_two_part_reference_11` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF01/test_pass_simple_delete.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_pass_simple_delete` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/mysql/RF01/test_mysql_select_no_from_should_not_except.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_mysql_select_no_from_should_not_except` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/RF01/test_postgres_value_table_alias.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_postgres_value_table_alias` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF01/test_pass_update_with_alias.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_pass_update_with_alias` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/sqlite/RF01/test_sqlite_create_trigger_after_insert_reference.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_sqlite_create_trigger_after_insert_reference` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/sqlite/RF01/test_sqlite_create_trigger_after_delete_reference.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_sqlite_create_trigger_after_delete_reference` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/sqlite/RF01/test_sqlite_create_trigger_after_update_reference.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_sqlite_create_trigger_after_update_reference` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/sqlite/RF01/test_sqlite_outside_of_trigger.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_sqlite_outside_of_trigger` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/mysql/RF01/test_mysql_identifier_with_backticks_should_not_except.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_mysql_identifier_with_backticks_should_not_except` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/RF01/test_postgres_pass_dot_inside_quotes.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_postgres_pass_dot_inside_quotes` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/RF01/test_pass_postgres_create_policy_using_target_table.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_pass_postgres_create_policy_using_target_table` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/RF01/test_pass_postgres_create_policy_with_check_target_table.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_pass_postgres_create_policy_with_check_target_table` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/RF01/test_pass_postgres_alter_policy_target_table.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_pass_postgres_alter_policy_target_table` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/RF01/test_fail_postgres_create_policy_unreferenced_table.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_fail_postgres_create_policy_unreferenced_table` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/RF01/test_pass_postgres_create_policy_no_table_reference.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_pass_postgres_create_policy_no_table_reference` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/mysql/RF01/test_pass_mysql_update_alias_correlated_subquery.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_pass_mysql_update_alias_correlated_subquery` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/mysql/RF01/test_pass_mysql_multi_table_update_alias_correlated_subquery.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_pass_mysql_multi_table_update_alias_correlated_subquery` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/mysql/RF01/test_pass_mysql_comma_multi_table_update_alias_correlated_subquery.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_pass_mysql_comma_multi_table_update_alias_correlated_subquery` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF01/test_pass_redundantly_bracketed_join_target_alias.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_pass_redundantly_bracketed_join_target_alias` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF01/test_fail_redundantly_bracketed_join_target_unknown_table.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml`, case `test_fail_redundantly_bracketed_join_target_unknown_table` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF04/test_pass_valid_identifier.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml`, case `test_pass_valid_identifier` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF04/test_fail_keyword_as_identifier_column.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml`, case `test_fail_keyword_as_identifier_column` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF04/test_fail_keyword_as_identifier_column_alias.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml`, case `test_fail_keyword_as_identifier_column_alias` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF04/test_fail_keyword_as_identifier_table_alias.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml`, case `test_fail_keyword_as_identifier_table_alias` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF04/test_pass_valid_identifier_not_alias.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml`, case `test_pass_valid_identifier_not_alias` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF04/test_fail_keyword_as_identifier_column_alias_config.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF04/test_fail_keyword_as_identifier_not_alias_all.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF04/test_fail_keyword_as_quoted_identifier_column.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF04/test_pass_ignore_word1.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF04/test_pass_ignore_word2.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF04/test_pass_ignore_words_regex1.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF04/test_pass_ignore_words_regex2.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF04/test_pass_keyword_as_quoted_identifier_column_none_policy.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF04/test_pass_valid_identifier_table_alias_column_alias_config.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/postgres/RF04/test_fail_keyword_as_column_name_postgres.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml`, case `test_fail_keyword_as_column_name_postgres` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF05/test_fail_special_chars_create_table_space.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml`, case `test_fail_special_chars_create_table_space` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF05/test_fail_special_chars_create_table_gt.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml`, case `test_fail_special_chars_create_table_gt` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF05/test_fail_special_chars_create_table_lt.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml`, case `test_fail_special_chars_create_table_lt` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF05/test_pass_special_chars_create_table_none.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml`, case `test_pass_special_chars_create_table_none` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF05/test_pass_ignore_lists_quoted_fail.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml`, case `test_pass_ignore_lists_quoted_fail` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF05/test_fail_ignore_words_regex_quoted_mixed_case.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF05/test_fail_special_chars_quoted_policy_alias.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF05/test_fail_special_chars_quoted_policy_column_alias.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF05/test_fail_table_aliases.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF05/test_pass_ignore_lists_quoted.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF05/test_pass_ignore_lists_quoted_mixed_case.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF05/test_pass_ignore_words_regex_quoted.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF05/test_pass_special_chars_create_table_space_allowed.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF05/test_pass_special_chars_quoted_policy_aliases.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF05/test_pass_special_chars_quoted_policy_column_aliases.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF05/test_pass_special_chars_quoted_policy_none.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF05/test_pass_table_aliases.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/postgres/RF05/test_pass_ignore_lists_unquoted_fail.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml`, case `test_pass_ignore_lists_unquoted_fail` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/RF05/test_pass_create_extension_reference_quoted.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml`, case `test_pass_create_extension_reference_quoted` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/RF05/test_pass_drop_extension_reference_quoted.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml`, case `test_pass_drop_extension_reference_quoted` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/RF05/test_pass_underscore_only_identifier_postgres.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml`, case `test_pass_underscore_only_identifier_postgres` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/RF05/test_pass_ignore_lists_unquoted.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/postgres/RF05/test_pass_ignore_lists_unquoted_mixed_case.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/postgres/RF05/test_pass_ignore_words_regex_unquoted.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF06/test_pass_default.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml`, case `test_pass_default` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/RF06/test_fail_default_lowercase.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml`, case `test_fail_default_lowercase` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/RF06/test_fail_quoted_column_postgres_case_insensitive.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/postgres/RF06/test_fail_quoted_column_postgres_case_sensitive.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF06/test_fail_quoted_column_ansi.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml`, case `test_fail_quoted_column_ansi` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF06/test_fail_column_reference_prefer_quoted_ansi.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF06/test_fail_multiple_references_prefer_quoted_ansi.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF06/test_fail_table_reference_prefer_quoted_ansi.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF06/test_pass_column_reference_prefer_quoted_ansi.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF06/test_pass_ignore_if.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF06/test_pass_ignore_lists.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF06/test_pass_ignore_lists_mixed_case.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF06/test_pass_ignore_words_regex.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF06/test_pass_ignore_words_regex_mixed_case.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF06/test_pass_multiple_references_prefer_quoted_ansi.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF06/test_pass_reserved_keyword_prefer_quoted_ansi.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF06/test_pass_special_symbols_prefer_quoted_ansi.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF06/test_pass_table_reference_prefer_quoted_ansi.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/RF06/test_pass_whitespace_prefer_quoted_ansi.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST01/no_redundant_else_null.sql` | `test/fixtures/rules/std_rule_cases/ST01.yml`, case `no_redundant_else_null` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST01/redundant_else_null.sql` | `test/fixtures/rules/std_rule_cases/ST01.yml`, case `redundant_else_null` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST01/alternate_case_when_syntax.sql` | `test/fixtures/rules/std_rule_cases/ST01.yml`, case `alternate_case_when_syntax` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST01/alternate_case_when_syntax_boolean.sql` | `test/fixtures/rules/std_rule_cases/ST01.yml`, case `alternate_case_when_syntax_boolean` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST01/else_expression.sql` | `test/fixtures/rules/std_rule_cases/ST01.yml`, case `else_expression` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_pass_case_cannot_be_reduced_1.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_pass_case_cannot_be_reduced_1` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_pass_case_cannot_be_reduced_2.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_pass_case_cannot_be_reduced_2` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_pass_case_cannot_be_reduced_3.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_pass_case_cannot_be_reduced_3` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_pass_case_cannot_be_reduced_4.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_pass_case_cannot_be_reduced_4` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_pass_case_cannot_be_reduced_5.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_pass_case_cannot_be_reduced_5` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_pass_case_cannot_be_reduced_6.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_pass_case_cannot_be_reduced_6` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_pass_case_cannot_be_reduced_7.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_pass_case_cannot_be_reduced_7` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_pass_case_cannot_be_reduced_8.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_pass_case_cannot_be_reduced_8` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_pass_case_cannot_be_reduced_9.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_pass_case_cannot_be_reduced_9` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_pass_case_cannot_be_reduced_10.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_pass_case_cannot_be_reduced_10` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_pass_case_cannot_be_reduced_11.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_pass_case_cannot_be_reduced_11` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_pass_case_cannot_be_reduced_12.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_pass_case_cannot_be_reduced_12` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/ST02/test_pass_case_cannot_be_reduced_13.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_pass_case_cannot_be_reduced_13` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_fail_unnecessary_case_1.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_fail_unnecessary_case_1` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_fail_unnecessary_case_2.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_fail_unnecessary_case_2` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_fail_unnecessary_case_3.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_fail_unnecessary_case_3` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_fail_unnecessary_case_4.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_fail_unnecessary_case_4` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_fail_unnecessary_case_5.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_fail_unnecessary_case_5` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_fail_unnecessary_case_6.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_fail_unnecessary_case_6` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_fail_unnecessary_case_7.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_fail_unnecessary_case_7` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_fail_unnecessary_case_8.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_fail_unnecessary_case_8` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_fail_unnecessary_case_9.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_fail_unnecessary_case_9` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_fail_unnecessary_case_10.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_fail_unnecessary_case_10` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_fail_unnecessary_case_11.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_fail_unnecessary_case_11` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_pass_distinct_from_null_does_not_change_expression.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_pass_distinct_from_null_does_not_change_expression` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_pass_not_distinct_from_null_does_not_change_expression.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml`, case `test_pass_not_distinct_from_null_does_not_change_expression` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST02/test_fail_no_copy_code_out_of_template.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml` | verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST08/test_fail_distinct_with_parenthesis_1.sql` | `test/fixtures/rules/std_rule_cases/ST08.yml`, case `test_fail_distinct_with_parenthesis_1` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST08/test_fail_distinct_with_parenthesis_2.sql` | `test/fixtures/rules/std_rule_cases/ST08.yml`, case `test_fail_distinct_with_parenthesis_2` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST08/test_fail_distinct_with_parenthesis_3.sql` | `test/fixtures/rules/std_rule_cases/ST08.yml`, case `test_fail_distinct_with_parenthesis_3` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST08/test_fail_distinct_with_parenthesis_4.sql` | `test/fixtures/rules/std_rule_cases/ST08.yml`, case `test_fail_distinct_with_parenthesis_4` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST08/test_fail_distinct_with_parenthesis_5.sql` | `test/fixtures/rules/std_rule_cases/ST08.yml`, case `test_fail_distinct_with_parenthesis_5` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST08/test_fail_distinct_with_parenthesis_6.sql` | `test/fixtures/rules/std_rule_cases/ST08.yml`, case `test_fail_distinct_with_parenthesis_6` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/postgres/ST08/test_fail_distinct_with_parenthesis_7.sql` | `test/fixtures/rules/std_rule_cases/ST08.yml`, case `test_fail_distinct_with_parenthesis_7` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST08/test_pass_no_distinct.sql` | `test/fixtures/rules/std_rule_cases/ST08.yml`, case `test_pass_no_distinct` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST08/test_fail_distinct_column_inside_count.sql` | `test/fixtures/rules/std_rule_cases/ST08.yml`, case `test_fail_distinct_column_inside_count` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST08/test_fail_distinct_concat_inside_count.sql` | `test/fixtures/rules/std_rule_cases/ST08.yml`, case `test_fail_distinct_concat_inside_count` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST08/test_pass_distinct_subquery_inside_count.sql` | `test/fixtures/rules/std_rule_cases/ST08.yml`, case `test_pass_distinct_subquery_inside_count` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST10/test_pass_where.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml`, case `test_pass_where` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST10/test_pass_select.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml`, case `test_pass_select` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST10/test_pass_join.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml`, case `test_pass_join` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST10/test_fail_where.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml`, case `test_fail_where` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST10/test_fail_select.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml`, case `test_fail_select` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST10/test_fail_join.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml`, case `test_fail_join` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST10/test_pass_allowed_literal_true.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml`, case `test_pass_allowed_literal_true` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST10/test_pass_allowed_literal_false.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml`, case `test_pass_allowed_literal_false` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST10/test_fail_disallowed_literal_true.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml`, case `test_fail_disallowed_literal_true` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST10/test_fail_disallowed_literal_false.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml`, case `test_fail_disallowed_literal_false` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST10/test_fail_disallowed_literal_false2.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml`, case `test_fail_disallowed_literal_false2` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST10/test_fail_disallowed_literal_false3.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml`, case `test_fail_disallowed_literal_false3` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST10/test_fail_bracketed.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml`, case `test_fail_bracketed` | `fail_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST10/test_pass_bracketed.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml`, case `test_pass_bracketed` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST10/test_noop_true_literal.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml`, case `test_noop_true_literal` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST10/test_noop_false_literal.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml`, case `test_noop_false_literal` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST10/test_noop_between.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml`, case `test_noop_between` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST10/test_other_operators_with_unknown_precedence.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml`, case `test_other_operators_with_unknown_precedence` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST10/test_other_operators_with_bracketed_precedence.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml`, case `test_other_operators_with_bracketed_precedence` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST10/test_other_operators_on_rhs.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml`, case `test_other_operators_on_rhs` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST10/test_pass_array_access.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml`, case `test_pass_array_access` | `pass_str`, verbatim |
| `upstream/sqlfluff/fixtures/ansi/ST10/test_pass_templated_queries.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml` | verbatim |
| `upstream/sqlfluff/fixtures/athena/AL05/test_pass_athena_back_quoted_select_naked_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/athena/AL05/test_pass_athena_naked_select_back_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/athena/AL05/test_pass_athena_naked_select_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/athena/AL05/test_pass_athena_quoted_select_naked_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/athena/RF01/test_athena_apply_rule_if_multiple_tables.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/athena/RF01/test_athena_apply_rule_if_one_aliased_table.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/athena/RF01/test_athena_apply_rule_if_single_table_and_force_enable.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/athena/RF01/test_athena_ignore_rule_if_single_table.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/athena/RF06/test_fail_prefer_quoted_keywords_athena.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/athena/RF06/test_pass_prefer_quoted_keywords_athena.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/AL04/test_pass_bigquery_function.sql` | `test/fixtures/rules/std_rule_cases/AL04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/AL05/test_bigquery_function_takes_tablealias_column_parameter.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/AL05/test_bigquery_function_takes_tablealias_column_struct_parameter.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/AL05/test_bigquery_function_takes_tablealias_parameter.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/AL05/test_ignore_bigquery_value_table_functions.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/AL05/test_pass_bigquery_aliased_table_with_ticks_referenced.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/AL05/test_pass_bigquery_cross_join_array.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/AL05/test_pass_bigquery_cross_join_unnest.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/AL05/test_pass_bigquery_naked_select_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/AL05/test_pass_bigquery_nested_inner_join.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/AL05/test_pass_bigquery_qualify_clause.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/AL05/test_pass_bigquery_quoted_escapes.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/AL05/test_pass_bigquery_quoted_select_naked_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/AL05/test_pass_bigquery_unaliased_table_with_hyphens.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/AL05/test_pass_bigquery_unnest.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/AL09/test_fail_bigquery_quoted_column_no_space_with_as.sql` | `test/fixtures/rules/std_rule_cases/AL09.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/AL09/test_fail_bigquery_quoted_column_no_space_without_as.sql` | `test/fixtures/rules/std_rule_cases/AL09.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/AL09/test_pass_bigquery_quoted_column_no_space_with_as.sql` | `test/fixtures/rules/std_rule_cases/AL09.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/AL09/test_pass_bigquery_quoted_column_no_space_without_as.sql` | `test/fixtures/rules/std_rule_cases/AL09.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/CV05/test_bigquery_set_options.sql` | `test/fixtures/rules/std_rule_cases/CV05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF01/test_bigquery_unnest_with_offset_pass.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF01/test_fail_object_not_referenced_1.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF01/test_pass_bigquery_dash.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF01/test_pass_bigquery_single_column_unpivot_output_aliases.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF01/test_pass_object_referenced_1.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF01/test_pass_object_referenced_4a.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF01/test_pass_object_referenced_5a.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF01/test_pass_object_referenced_5c.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF01/test_pass_unnest_aliased_bigquery.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF01/test_pass_unnest_bigquery.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF05/test_fail_single_quote_bigquery.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF05/test_fail_special_chars_create_table_lt_bigquery.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF05/test_fail_star_bigquery.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF05/test_pass_dot_bigquery.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF05/test_pass_dots_bigquery.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF05/test_pass_hyphen_bigquery_no_quotes.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF05/test_pass_hyphen_bigquery_quotes_1.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF05/test_pass_hyphen_bigquery_quotes_2.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF05/test_pass_single_quote2_bigquery.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF05/test_pass_single_quote_bigquery.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF05/test_pass_special_chars_create_table_none_quoted_biquery.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF05/test_pass_star_bigquery.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF06/test_fail_column_reference_prefer_quoted_backticks.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF06/test_fail_multiple_references_prefer_quoted_backticks.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF06/test_fail_table_reference_prefer_quoted_backticks.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF06/test_pass_column_reference_prefer_quoted_backticks.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF06/test_pass_multiple_references_prefer_quoted_backticks.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF06/test_pass_reserved_keyword_prefer_quoted_backticks.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF06/test_pass_special_symbols_prefer_quoted_backticks.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF06/test_pass_table_reference_prefer_quoted_backticks.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF06/test_pass_uppivot_bigquery.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/bigquery/RF06/test_pass_whitespace_prefer_quoted_backticks.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/clickhouse/AL05/test_fail_clickhouse_naked_select_quoted_table_cs.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/clickhouse/AL05/test_pass_clickhouse_naked_select_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/clickhouse/AL05/test_pass_clickhouse_quoted_escapes.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/clickhouse/AL05/test_pass_clickhouse_quoted_select_naked_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/clickhouse/AL05/test_pass_clickhouse_quoted_select_naked_table_cs.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/clickhouse/AL09/test_pass_different_case_clickhouse.sql` | `test/fixtures/rules/std_rule_cases/AL09.yml` | verbatim |
| `upstream/sqlfluff/fixtures/clickhouse/ST02/test_pass_differently_cased_clickhouse_indexed_columns_no_else.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml` | verbatim |
| `upstream/sqlfluff/fixtures/clickhouse/ST02/test_pass_differently_cased_clickhouse_indexed_columns_with_else_null.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml` | verbatim |
| `upstream/sqlfluff/fixtures/databricks/RF01/test_fail_databricks_non_metadata_reference.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/databricks/RF01/test_pass_databricks_metadata_file_path.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/databricks/RF01/test_pass_databricks_metadata_in_where.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/databricks/RF01/test_pass_databricks_metadata_multiple_fields.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/db2/AL05/test_fail_db2_naked_select_double_quoted_table_wrong_case.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/db2/AL05/test_pass_db2_naked_select_double_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/duckdb/AL05/test_fail_duckdb_single_quote_select_naked_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/duckdb/AL05/test_pass_duckdb_naked_select_double_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/duckdb/AL05/test_pass_duckdb_quoted_select_naked_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/duckdb/AL05/test_pass_duckdb_quoted_select_single_quote_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/duckdb/RF01/test_pass_duckdb_single_key_unpivot_output_aliases.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/duckdb/RF01/test_pass_object_referenced_4b.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/duckdb/RF06/test_fail_default_insensitive.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/exasol/AL05/test_fail_exasol_quoted_select_naked_table_wrong_case.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/exasol/AL05/test_fail_exasol_values_clause.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/exasol/AL05/test_pass_exasol_naked_select_double_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/exasol/AL05/test_pass_exasol_quoted_select_naked_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/exasol/AL05/test_pass_exasol_quoted_select_single_quote_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/exasol/AL05/test_pass_exasol_values_clause.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/exasol/AM02/test_exasol.sql` | `test/fixtures/rules/std_rule_cases/AM02.yml` | verbatim |
| `upstream/sqlfluff/fixtures/exasol/AM02/test_exasol_union_all.sql` | `test/fixtures/rules/std_rule_cases/AM02.yml` | verbatim |
| `upstream/sqlfluff/fixtures/exasol/RF01/test_exasol_invalid_foreign_key_from.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/exasol/RF01/test_pass_exasol_select_into.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/exasol/RF06/test_fail_create_quoted_user_exasol.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/exasol/RF06/test_pass_create_user_quoted_password_exasol.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/exasol/RF06/test_pass_exasol_password_literals.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/hive/AL05/test_pass_hive_naked_select_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/hive/AL05/test_pass_hive_quoted_select_naked_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/hive/RF01/test_pass_object_referenced_5d.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/mariadb/RF04/test_pass_mariadb_column_charset_keyword_name.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/mysql/RF04/test_fail_mysql_keyword_column_name_still_flagged.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/mysql/RF04/test_pass_mysql_alter_convert_charset_keyword_name.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/mysql/RF04/test_pass_mysql_alter_default_charset_keyword_name.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/mysql/RF04/test_pass_mysql_column_charset_keyword_name.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/oracle/AL05/test_fail_oracle_naked_select_double_quoted_table_wrong_case.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/oracle/AL05/test_pass_oracle_naked_select_double_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/oracle/RF01/test_pass_oracle_schema_sequence_currval.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/oracle/RF01/test_pass_oracle_sequence_currval.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/oracle/RF01/test_pass_oracle_sequence_nextval.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/redshift/AL05/test_fail_redshift_naked_select_double_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/redshift/AL05/test_fail_redshift_qualify_does_not_follows_from.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/redshift/AL05/test_fail_redshift_qualify_follows_from.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/redshift/AL05/test_pass_redshift_array_value.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/redshift/AL05/test_pass_redshift_naked_select_double_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/redshift/AL05/test_pass_redshift_qualify_follows_from.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/redshift/AL05/test_pass_redshift_quoted_select_naked_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/redshift/AL05/test_pass_redshift_quoted_select_single_quote_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/redshift/AL05/test_pass_redshift_semi_structured_op.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/redshift/AL05/test_redshift_whole_row_reference.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/redshift/AM02/test_redshift.sql` | `test/fixtures/rules/std_rule_cases/AM02.yml` | verbatim |
| `upstream/sqlfluff/fixtures/redshift/PG02/test_pass_non_postgres_dialect_ignored.sql` | `test/fixtures/rules/std_rule_cases/PG02.yml` | verbatim |
| `upstream/sqlfluff/fixtures/redshift/RF01/test_pass_object_referenced_5e.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/redshift/RF01/test_pass_redshift_array_unnest.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/redshift/RF01/test_pass_redshift_from_unpivot_output_aliases.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/redshift/RF01/test_pass_redshift_object_unpivot.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/redshift/RF05/test_fail_special_chars_redshift_hash_column.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/redshift/RF05/test_pass_special_chars_redshift_hash_table.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/redshift/RF05/test_pass_underscore_only_identifier_redshift.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/redshift/RF06/test_pass_datetime_redshift.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/redshift/ST02/test_pass_simple_case_ignored.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/AL05/test_fail_snowflake_flatten_function.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/AL05/test_fail_snowflake_flatten_lateral.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/AL05/test_fail_snowflake_naked_select_double_quoted_table_wrong_case.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/AL05/test_pass_snowflake_flatten_lateral.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/AL05/test_pass_snowflake_naked_select_double_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/AL05/test_pass_snowflake_values.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/AL05/test_snowflake_delete_cte.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/AL08/test_fail_alias_quoted.sql` | `test/fixtures/rules/std_rule_cases/AL08.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/RF01/test_fail_redundantly_bracketed_join_target_unknown_table_snowflake.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/RF01/test_fail_snowflake_ordinary_outer_reference.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/RF01/test_fail_snowflake_semantic_view_outer_join_reference.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/RF01/test_fail_snowflake_semantic_view_outer_select_reference.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/RF01/test_fail_snowflake_semantic_view_outer_where_reference.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/RF01/test_pass_redundantly_bracketed_join_target_alias_snowflake.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/RF01/test_pass_snowflake_from_unpivot_output_aliases.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/RF01/test_pass_snowflake_schema_sequence_nextval.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/RF01/test_pass_snowflake_semantic_view_facts.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/RF01/test_pass_snowflake_semantic_view_references.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/RF01/test_pass_snowflake_sequence_currval.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/RF01/test_pass_snowflake_sequence_nextval.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/RF01/test_snowflake_pass_dot_inside_quotes.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/RF04/test_pass_one_character_identifier.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/RF05/test_pass_pivot_snowflake.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/RF05/test_pass_pivot_snowflake_complex.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/RF05/test_pass_special_chars_snowflake_stage_metadata.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/RF06/test_fail_default_uppercase.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/RF06/test_fail_quoted_column_snowflake_case_insensitive.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/RF06/test_pass_quoted_column_snowflake.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/RF06/test_snowflake_default_keyword.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/ST02/test_fail_identical_quoted_indexed_expression_when_not_null_no_else.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/ST02/test_fail_indexed_expression_when_not_null_no_else.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/ST02/test_fail_indexed_expression_when_null_else_null.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/ST02/test_fail_mixed_case_indexed_expression_when_not_null_no_else.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/ST02/test_fail_multiple_indexed_expression_when_not_null_no_else.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/ST02/test_fail_qualified_indexed_expression_when_not_null_no_else.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/ST02/test_pass_array_accessors.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/ST02/test_pass_composite_condition_does_not_change_indexed_then_expression.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/ST02/test_pass_composite_condition_with_else_null_does_not_change_indexed_then_expression.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/ST02/test_pass_differently_cased_quoted_indexed_columns.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/ST02/test_pass_dynamic_index_does_not_change_expression.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/ST02/test_pass_indexed_condition_does_not_change_bare_then_expression.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/ST02/test_pass_null_condition_does_not_change_bare_then_expression.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml` | verbatim |
| `upstream/sqlfluff/fixtures/snowflake/ST02/test_pass_null_condition_does_not_change_indexed_then_expression.sql` | `test/fixtures/rules/std_rule_cases/ST02.yml` | verbatim |
| `upstream/sqlfluff/fixtures/soql/RF01/test_soql_ignore_rule.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sparksql/AL05/test_fail_sparksql_values_clause.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sparksql/AL05/test_pass_sparksql_naked_select_double_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sparksql/AL05/test_pass_sparksql_quoted_select_naked_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sparksql/AL05/test_pass_sparksql_values_clause.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sparksql/RF01/test_fail_explode_nested_field_sparksql.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sparksql/RF01/test_pass_explode_nested_field_sparksql.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sparksql/RF01/test_pass_sparksql_metadata_file_path.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sparksql/RF05/test_fail_special_chars_show_tblproperties.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sparksql/RF05/test_pass_alteration_identifier_in_file_reference_sparksql.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sparksql/RF05/test_pass_character_class_identifier_in_file_reference_sparksql.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sparksql/RF05/test_pass_character_range_identifier_in_file_reference_sparksql.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sparksql/RF05/test_pass_dot_slash_identifier_in_file_reference_sparksql.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sparksql/RF05/test_pass_negated_character_class_identifier_in_file_reference_sparksql.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sparksql/RF05/test_pass_negated_character_range_identifier_in_file_reference_sparksql.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sparksql/RF05/test_pass_question_mark_identifier_in_file_reference_sparksql.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sparksql/RF05/test_pass_special_chars_show_tblproperties.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sparksql/RF05/test_pass_star_identifier_in_file_reference_sparksql.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sparksql/RF06/test_fail_insert_overwrite_directory.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sparksql/RF06/test_pass_insert_overwrite_directory.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sqlite/RF06/test_fail_prefer_quoted_identifiers_sqlite_no_fix.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sqlite/RF06/test_fail_prefer_quoted_keywords_sqlite_backticks.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sqlite/RF06/test_fail_prefer_quoted_keywords_sqlite_backticks_to_double_quotes.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sqlite/RF06/test_fail_prefer_quoted_keywords_sqlite_double_quotes.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sqlite/RF06/test_fail_prefer_quoted_keywords_sqlite_double_quotes_to_backticks.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sqlite/RF06/test_pass_prefer_quoted_identifiers_sqlite_existing_quotes.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sqlite/RF06/test_pass_prefer_quoted_keywords_sqlite_backticks.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/sqlite/RF06/test_pass_prefer_quoted_keywords_sqlite_double_quotes.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/teradata/AL05/test_pass_teradata_naked_select_double_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/teradata/AL05/test_pass_teradata_quoted_select_naked_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/AL05/test_pass_trino_naked_select_double_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/AL05/test_pass_trino_quoted_select_naked_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/RF01/test_pass_trino_lambda_expression.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/RF01/test_trino_aliased_row_fields_force_enable.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/RF01/test_trino_correlated_nested_row_reference_quoted_alias.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/RF01/test_trino_correlated_reference_in_joined_subquery.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/RF01/test_trino_correlated_row_reference_force_enable.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/RF01/test_trino_correlated_row_reference_in_joined_subquery.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/RF01/test_trino_correlated_row_reference_qualified_table.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/RF01/test_trino_row_fields_force_enable.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/RF01/test_trino_row_fields_from_single_aliased_table.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/RF01/test_trino_row_fields_from_single_quoted_aliased_table.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/RF01/test_trino_row_fields_from_single_table.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/RF01/test_trino_row_fields_in_bracketed_union.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/RF01/test_trino_row_fields_in_union.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/RF01/test_trino_row_reference_with_multiple_tables.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/RF01/test_trino_unknown_correlated_row_reference.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/RF01/test_trino_unknown_reference_in_joined_subquery.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/RF01/test_trino_unknown_reference_in_joined_subquery_unaliased_parent.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/RF01/test_trino_unknown_reference_in_union_force_enable.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/RF01/test_trino_unknown_reference_in_union_join.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/RF01/test_trino_unknown_reference_with_multiple_tables.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/trino/RF01/test_trino_unknown_row_qualifier_containing_parent_alias.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/AL04/test_pass_tsql_table_variable.sql` | `test/fixtures/rules/std_rule_cases/AL04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/AL05/test_pass_tsql_naked_select_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/AL05/test_pass_tsql_nodes_method_requires_alias.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/AL05/test_pass_tsql_nodes_method_requires_alias_unreferenced.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/AL05/test_pass_tsql_object_reference_override.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/AL05/test_pass_tsql_openrowset_bulk_requires_correlation_name_1.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/AL05/test_pass_tsql_openrowset_bulk_requires_correlation_name_2.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/AL05/test_pass_tsql_quoted_select_naked_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/AL05/test_pass_tsql_values_clause_in_parentheses.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/AL09/test_fail_no_quotes_same_case_tsql.sql` | `test/fixtures/rules/std_rule_cases/AL09.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/AL09/test_pass_no_self_alias_tsql.sql` | `test/fixtures/rules/std_rule_cases/AL09.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/AL09/test_pass_no_self_alias_tsql_no_spaces.sql` | `test/fixtures/rules/std_rule_cases/AL09.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/AL09/test_pass_tsql_quoted_column_no_space_with_as.sql` | `test/fixtures/rules/std_rule_cases/AL09.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/CV05/test_tsql_alternate_alias_syntax.sql` | `test/fixtures/rules/std_rule_cases/CV05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/CV05/test_tsql_exec_clause.sql` | `test/fixtures/rules/std_rule_cases/CV05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/CV05/test_tsql_variable_assignment.sql` | `test/fixtures/rules/std_rule_cases/CV05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF01/test_fail_redundantly_bracketed_join_target_unknown_table_tsql.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF01/test_nested_join_clause_does_not_flag.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF01/test_nested_join_clauses_do_not_flag.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF01/test_parenthesized_join_clauses_do_not_flag.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF01/test_pass_postgres_merge_with_alias.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF01/test_pass_redundantly_bracketed_join_target_alias_tsql.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF01/test_pass_tsql_containstable_arguments.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF01/test_pass_tsql_containstable_join_arguments.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF01/test_pass_tsql_nested_join_alias.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF01/test_pass_tsql_pivot_table_alias.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF01/test_tsql_bracketed_identifiers_pass.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF01/test_tsql_select_system_as_identifier.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF04/test_fail_keyword_as_quoted_identifier_column_alias.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF04/test_fail_keyword_as_quoted_identifier_column_alias_config.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF04/test_fail_keyword_as_quoted_identifier_not_alias_all.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF04/test_fail_keyword_as_quoted_identifier_table_alias.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF04/test_pass_valid_quoted_identifier.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF04/test_pass_valid_quoted_identifier_not_alias.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF04/test_pass_valid_quoted_identifier_table_alias_column_alias_config.sql` | `test/fixtures/rules/std_rule_cases/RF04.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF05/test_fail_special_chars_create_table_hash_unquoted.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF05/test_fail_special_chars_unquoted_policy_aliases.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF05/test_fail_special_chars_unquoted_policy_column_aliases.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF05/test_fail_tsql_single_quoted_alias_special_chars.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF05/test_pass_special_chars_create_table_none_quoted_tsql.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF05/test_pass_special_chars_unquoted_policy_aliases.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF05/test_pass_special_chars_unquoted_policy_column_aliases.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF05/test_pass_tsql_single_quoted_alias.sql` | `test/fixtures/rules/std_rule_cases/RF05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF06/test_fail_multiple_references_prefer_quoted_tsql.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF06/test_fail_quoted_column_tsql.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF06/test_fail_quoted_column_tsql_case_insensitive.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF06/test_fail_reference_tsql.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF06/test_fail_table_reference_prefer_quoted_tsql.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF06/test_pass_column_reference_prefer_quoted_tsql.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF06/test_pass_ignore_if_tsql.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF06/test_pass_ignore_lists_mixed_case_tsql.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF06/test_pass_ignore_lists_tsql.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF06/test_pass_ignore_words_regex_mixed_case_tsql.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF06/test_pass_ignore_words_regex_tsql.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF06/test_pass_multiple_references_prefer_quoted_tsql.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF06/test_pass_quoted_identifier_keyword_tsql.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF06/test_pass_reference_tsql.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF06/test_pass_reserved_keyword_prefer_quoted_tsql.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF06/test_pass_special_symbols_prefer_quoted_tsql.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF06/test_pass_table_reference_prefer_quoted_tsql.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF06/test_pass_tsql_execute_as.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/RF06/test_pass_whitespace_prefer_quoted_tsql.sql` | `test/fixtures/rules/std_rule_cases/RF06.yml` | verbatim |
| `upstream/sqlfluff/fixtures/tsql/ST10/test_bitwise_operator_on_rhs_tsql.sql` | `test/fixtures/rules/std_rule_cases/ST10.yml` | verbatim |
| `upstream/sqlfluff/fixtures/vertica/AL05/test_pass_vertica_naked_select_double_quoted_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/vertica/AL05/test_pass_vertica_quoted_select_naked_table.sql` | `test/fixtures/rules/std_rule_cases/AL05.yml` | verbatim |
| `upstream/sqlfluff/fixtures/vertica/RF01/test_pass_vertica_lambda_expression.sql` | `test/fixtures/rules/std_rule_cases/RF01.yml` | verbatim |

> `recording.json` is a port readiness recording, and no file of this pack, so it is recorded here rather than in the
> table above: the tool-pack's `recording.lock` names it and a GitHub release holds the bytes (docs/port-readiness.md).
> It carries excerpts of this upstream's tests (`test/fixtures/rules/std_rule_cases/*.yml`: the `fail_str` and
> `pass_str` of each case), redistributed under the licence above, together with Rerail's Apache-2.0; the LICENSE beside
> this file is this upstream's own, at the pinned commit.
