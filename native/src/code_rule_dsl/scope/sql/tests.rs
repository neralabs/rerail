    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::test_support::pack_test_data;
    use ast_grep_language::LanguageExt;
    use serde_json::Value as Json;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    /// SQLFluff's recording on its own fixtures, or one of them: `packs/sql/upstream/sqlfluff/<name>`.
    fn sqlfluff(name: &str) -> PathBuf {
        pack_test_data(&format!("packs/sql/upstream/sqlfluff/{name}"))
    }

    fn json(name: &str) -> Json {
        let text = std::fs::read_to_string(sqlfluff(name)).expect("recording");
        serde_json::from_str(&text).expect("json")
    }

    /// A finding as the oracle states it: the 1-based line and 1-based code-point column.
    type Finding = (String, String, u64, u64);

    fn position(source: &str, offset: usize) -> (u64, u64) {
        let before = &source[..offset];
        let line = before.matches('\n').count() as u64 + 1;
        let column = before.rsplit('\n').next().unwrap().chars().count() as u64 + 1;
        (line, column)
    }

    #[derive(Debug)]
    enum Mismatch {
        /// SQLFluff recorded it; the scope does not report it.
        Missing,
        /// The scope reports it; SQLFluff did not record it.
        Extra,
    }

    const QUOTED_CASE: &str = "recorded under ansi, where a naked identifier folds to upper case so `a` matches `\"A\"`; the scope folds as PostgreSQL does (lower case), and SQLFluff run with dialect postgres reports this AL05 too (checked); `sameFindings` is false in fixtures.cases.json";

    const SQLITE_TRIGGER: &str = "sqlite-only: a SQLite trigger body (`CREATE TRIGGER x AFTER DELETE ON y BEGIN SELECT ... END`) is no PostgreSQL statement, which runs a trigger's function; the grammar reads the trigger head as an ERROR and the body's `SELECT` as a statement of its own, so the `old`/`new` row that SQLFluff's sqlite RF01 adds as an implicit target (`_get_implicit_targets`) is unresolved there";

    /// Every difference between the recording and the scope: (path, rule, line, column, kind, reason).
    const MISMATCHES: &[(&str, &str, u64, u64, Mismatch, &str)] = &[
        (
            "ansi/AL05/test_ansi_function_not_table_parameter.sql",
            "AL05",
            2,
            18,
            Mismatch::Missing,
            "ansi has no row references, so `TO_JSON_STRING(t)` does not use `t`; PostgreSQL has (AL05 `allow_row_references`, issue #7039) and SQLFluff with dialect postgres reports nothing here (checked); `sameFindings` is false in fixtures.cases.json",
        ),
        (
            "ansi/AL05/test_fail_naked_select_single_quoted_table_wrong_case.sql",
            "AL05",
            2,
            14,
            Mismatch::Missing,
            "ansi-only: `tab1 as 'a'` is not ansi or PostgreSQL (`parses` is false in fixtures.cases.json; dialect postgres fails to parse it); the string alias keeps its case under ansi folding (`a` is `A`, `'a'` stays) and so is unused, the scope reads it as the name `a`",
        ),
        (
            "ansi/AL05/test_pass_naked_select_quoted_table.sql",
            "AL05",
            2,
            14,
            Mismatch::Extra,
            QUOTED_CASE,
        ),
        (
            "ansi/AL05/test_pass_naked_select_quoted_table_subquery.sql",
            "AL05",
            2,
            17,
            Mismatch::Extra,
            QUOTED_CASE,
        ),
        (
            "ansi/AL05/test_pass_naked_select_single_quoted_table.sql",
            "AL05",
            2,
            14,
            Mismatch::Extra,
            "ansi-only: `tab1 as 'A'` is not ansi or PostgreSQL (`parses` is false in fixtures.cases.json); `a.col1` folds to lower case and `'A'` stays upper, as in QUOTED_CASE",
        ),
        (
            "ansi/AL05/test_pass_quoted_select_naked_table.sql",
            "AL05",
            2,
            14,
            Mismatch::Extra,
            QUOTED_CASE,
        ),
        (
            "ansi/AL05/test_pass_quoted_select_naked_table_subquery.sql",
            "AL05",
            2,
            17,
            Mismatch::Extra,
            QUOTED_CASE,
        ),
        (
            "ansi/RF01/test_fail_redundantly_bracketed_join_target_unknown_table.sql",
            "AL05",
            2,
            15,
            Mismatch::Extra,
            "grammar gap: the grammar reads no redundantly bracketed join target (`INNER JOIN ((b JOIN c ON b.id = c.id)) ON a.id = b.id`), which PostgreSQL accepts; the join from its `INNER` to the outer `ON` is an ERROR outside the statement, so the outer condition's `a.id` is no reference and `a` looks unused",
        ),
        (
            "sqlite/AL05/test_pass_sqlite_different_escaped_quotes.sql",
            "RF01",
            3,
            1,
            Mismatch::Missing,
            "sqlite-only: `'`''a\"'.id` qualifies a column with a string literal, which sqlite reads as an identifier; the grammar reads a literal, not an object_reference",
        ),
        (
            "sqlite/AL05/test_pass_sqlite_different_escaped_quotes.sql",
            "AL05",
            4,
            17,
            Mismatch::Extra,
            "sqlite-only: the alias is used by the string-literal qualifier above, which the grammar does not read",
        ),
        (
            "sqlite/RF01/test_sqlite_create_trigger_after_delete_reference.sql",
            "RF01",
            5,
            16,
            Mismatch::Extra,
            SQLITE_TRIGGER,
        ),
        (
            "sqlite/RF01/test_sqlite_create_trigger_after_insert_reference.sql",
            "RF01",
            5,
            16,
            Mismatch::Extra,
            SQLITE_TRIGGER,
        ),
        (
            "sqlite/RF01/test_sqlite_create_trigger_after_update_reference.sql",
            "RF01",
            5,
            16,
            Mismatch::Extra,
            SQLITE_TRIGGER,
        ),
        (
            "sqlite/RF01/test_sqlite_create_trigger_after_update_reference.sql",
            "RF01",
            5,
            27,
            Mismatch::Extra,
            SQLITE_TRIGGER,
        ),
    ];

    fn read_scopes(source: &str) -> SqlScopes {
        SqlScopes::read(&GrammarDialect::Sql.ast_grep(source).root())
    }

    /// The `nth` whole-word occurrence of `needle` in `source`.
    fn at(source: &str, needle: &str, nth: usize) -> Range<usize> {
        let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        let start = source
            .match_indices(needle)
            .filter(|(start, _)| {
                !word(source[..*start].chars().next_back())
                    && !word(source[start + needle.len()..].chars().next())
            })
            .nth(nth)
            .unwrap()
            .0;
        start..start + needle.len()
    }

    #[test]
    fn qualifiers_resolve_to_their_binding() {
        let source = "SELECT t.a, s.u.b FROM s.u JOIN x AS t ON true";
        let scopes = read_scopes(source);
        assert!(scopes.unresolved().is_empty());
        assert!(scopes.unused_aliases().is_empty());
        assert_eq!(
            scopes.binding(&at(source, "t", 0)),
            Some(at(source, "t", 1))
        );
        // `s.u.b` anchors at its table part and resolves to the unaliased `s.u`.
        assert_eq!(
            scopes.binding(&at(source, "u", 0)),
            Some(at(source, "u", 1))
        );
    }

    #[test]
    fn unknown_and_case_different_qualifiers_are_unresolved() {
        let source = "SELECT z.a, B.c FROM x AS b";
        assert_eq!(
            read_scopes(source).unresolved(),
            vec![at(source, "z", 0), at(source, "B", 0)]
        );
        let source = "SELECT * FROM s.x WHERE t.x.a = 1 AND o.x.a = 1";
        assert_eq!(
            read_scopes(source).unresolved(),
            vec![at(source, "x", 1), at(source, "x", 2)]
        );
    }

    #[test]
    fn nested_blocks_see_their_parents() {
        let source = "WITH c AS (SELECT 1 FROM x) SELECT c.a FROM c JOIN y AS t ON true \
                      WHERE EXISTS (SELECT 1 FROM z WHERE z.k = t.k)";
        let scopes = read_scopes(source);
        assert!(scopes.unresolved().is_empty());
        assert!(scopes.unused_aliases().is_empty());
        // `c.a` resolves to the relation that selects from the CTE.
        assert_eq!(
            scopes.binding(&at(source, "c", 1)),
            Some(at(source, "c", 2))
        );
        let source = "SELECT 1 FROM (SELECT t.a FROM x) AS d WHERE d.a = 1";
        assert_eq!(read_scopes(source).unresolved(), vec![at(source, "t", 0)]);
    }

    /// SQLFluff 4.4.0 RF01 (`sqlfluff lint --dialect postgres --rules RF01`) resolves a qualifier
    /// against `FROM` relations only: a CTE the query does not select from is no target.
    #[test]
    fn ctes_are_targets_only_through_from() {
        let source = "WITH c AS (SELECT 1 AS a) SELECT c.a FROM other_table";
        assert_eq!(read_scopes(source).unresolved(), vec![at(source, "c", 1)]);
        let source = "WITH c AS (SELECT 1 AS a) SELECT c.a FROM c";
        assert!(read_scopes(source).unresolved().is_empty());
        let source = "WITH c AS (SELECT 1 AS a) SELECT x FROM c WHERE EXISTS (SELECT c.a FROM d)";
        assert!(read_scopes(source).unresolved().is_empty());
    }

    /// SQLFluff 4.4.0 RF01 compares the unquoted text case sensitively, on PostgreSQL too: it
    /// reports `T` in `SELECT T.a FROM t` and `t` in `SELECT t.a FROM "T"`, not `T` in
    /// `SELECT T.a FROM "T"` nor `"t"` in `SELECT "t".a FROM t`.
    #[test]
    fn qualifiers_compare_unquoted_text_case_sensitively() {
        for (source, unresolved) in [
            ("SELECT T.a FROM t", Some("T")),
            ("SELECT t.a FROM T", Some("t")),
            ("SELECT t.a FROM \"T\"", Some("t")),
            ("SELECT \"T\".a FROM t", Some("\"T\"")),
            ("SELECT S.T.a FROM s.t", Some("T")),
            ("SELECT T.a FROM \"T\"", None),
            ("SELECT \"t\".a FROM t", None),
        ] {
            let expected: Vec<_> = unresolved
                .map(|needle| {
                    let start = source.find(&format!("{needle}.a")).unwrap();
                    start..start + needle.len()
                })
                .into_iter()
                .collect();
            assert_eq!(read_scopes(source).unresolved(), expected, "{source}");
        }
    }

    #[test]
    fn dml_targets_bind_and_only_selects_report() {
        let source = "UPDATE t SET a = (SELECT 1 FROM b WHERE b.k = t.k) WHERE q.a = 1";
        assert!(read_scopes(source).unresolved().is_empty());
        let source =
            "DELETE FROM s.t AS d WHERE EXISTS (SELECT 1 FROM u WHERE u.k = d.k AND w.k = 1)";
        assert_eq!(read_scopes(source).unresolved(), vec![at(source, "w", 0)]);
    }

    /// SQLFluff 4.4.0 RF01 (`_get_implicit_targets`, dialect postgres) resolves a qualifier in a
    /// query nested in a `CREATE POLICY` or `ALTER POLICY` statement against the policy's table,
    /// and still reports an unknown one.
    #[test]
    fn policies_bind_their_table() {
        for source in [
            "CREATE POLICY p ON t USING (EXISTS (SELECT 1 FROM u WHERE u.k = t.k));",
            "CREATE POLICY p ON s.t FOR UPDATE TO PUBLIC USING (true) \
             WITH CHECK (EXISTS (SELECT 1 FROM u WHERE u.k = t.k));",
            "ALTER POLICY p ON t USING (EXISTS (SELECT 1 FROM u WHERE u.k = t.k));",
        ] {
            assert!(read_scopes(source).unresolved().is_empty(), "{source}");
        }
        let source = "CREATE POLICY p ON t USING (EXISTS (SELECT 1 FROM u WHERE u.k = w.k));";
        assert_eq!(read_scopes(source).unresolved(), vec![at(source, "w", 0)]);
        let source = "ALTER POLICY p ON t WITH CHECK (EXISTS (SELECT w.k FROM u));";
        assert_eq!(read_scopes(source).unresolved(), vec![at(source, "w", 0)]);
    }

    /// SQLFluff 4.4.0 RF01 (`sqlfluff lint --dialect postgres --rules RF01`) reports nothing in an
    /// `INSERT`'s conflict action, after a `SELECT` as after `VALUES`, and still reports the
    /// `SELECT`'s own unknown qualifier.
    #[test]
    fn insert_conflict_actions_are_not_the_selects() {
        for source in [
            "INSERT INTO t (x, y) SELECT s.a, s.b FROM s ON CONFLICT (x) DO UPDATE SET y = EXCLUDED.y;",
            "INSERT INTO t (x, y) SELECT s.a, s.b FROM s ON CONFLICT (x) DO UPDATE SET y = excluded.y;",
            "INSERT INTO t (x, y) VALUES (1, 2) ON CONFLICT (x) DO UPDATE SET y = EXCLUDED.y;",
            "INSERT INTO t (x, y) VALUES (1, 2) ON CONFLICT (x) DO UPDATE SET y = excluded.y;",
            "INSERT INTO t (x, y) SELECT s.a, s.b FROM s ON CONFLICT (x) DO UPDATE SET y = zz.y \
             WHERE qq.y = 1;",
        ] {
            assert!(read_scopes(source).unresolved().is_empty(), "{source}");
        }
        let source = "INSERT INTO t (x, y) SELECT s.a, zz.b FROM s ON CONFLICT (x) DO NOTHING;";
        assert_eq!(read_scopes(source).unresolved(), vec![at(source, "zz", 0)]);
    }

    #[test]
    fn unused_aliases_follow_al05() {
        let source = "SELECT 1 FROM x AS a JOIN x AS b ON true";
        assert!(
            read_scopes(source).unused_aliases().is_empty(),
            "a repeated table is exempt"
        );
        let source = "SELECT 1 FROM x AS a JOIN (SELECT 1) AS q ON true JOIN y r ON r.k = 1";
        assert_eq!(
            read_scopes(source).unused_aliases(),
            vec![at(source, "a", 0)]
        );
        let source = "SELECT to_json(t) FROM x AS t";
        assert!(
            read_scopes(source).unused_aliases().is_empty(),
            "a row reference uses the alias"
        );
    }

    /// Each binding of `source`, in source order, with its form and the `used` fact `Usage::sql`
    /// gives it in the file's `ScopeTable`.
    fn used(source: &str) -> Vec<(&str, Form, Option<bool>)> {
        use crate::code_rule_dsl::scope::ScopeTable;
        use crate::code_rule_dsl::scope::usage::{BindingId, Usage};
        let root = GrammarDialect::Sql.ast_grep(source);
        let table = ScopeTable::build(root.root(), GrammarDialect::Sql, usize::MAX, &|| false)
            .expect("table builds");
        let usage = Usage::build(&table, &root.root(), usize::MAX, &|| false).expect("usage");
        let mut bindings = Vec::new();
        for (scope, entry) in table.scopes.iter().enumerate() {
            for (name, chain) in &entry.bindings {
                for (index, binding) in chain.iter().enumerate() {
                    let node = binding.name.expect("a SQL binding has a name");
                    let facts = usage.facts(BindingId {
                        scope: scope as u32,
                        name,
                        index,
                    });
                    bindings.push((
                        node.start as usize..node.end as usize,
                        binding.site.form,
                        facts.used,
                    ));
                }
            }
        }
        bindings.sort_by_key(|(range, ..)| range.start);
        bindings
            .into_iter()
            .map(|(range, form, used)| (&source[range], form, used))
            .collect()
    }

    /// `Usage::sql` puts AL05's walk on the table's bindings: each block's scope, name and index.
    #[test]
    fn usage_marks_the_bindings_references_use() {
        assert_eq!(
            used("SELECT 1 FROM x AS a JOIN y r ON r.k = 1"),
            vec![
                ("a", Form::Relation, Some(false)),
                ("r", Form::Relation, Some(true)),
            ]
        );
        // `o.k` uses the outer `o` from the subquery; `s.k` uses the subquery's own `s`, which
        // shadows the outer `s`.
        assert_eq!(
            used(
                "SELECT 1 FROM x AS o JOIN y AS s ON true \
                 WHERE EXISTS (SELECT 1 FROM z AS s WHERE s.k = o.k)"
            ),
            vec![
                ("o", Form::Relation, Some(true)),
                ("s", Form::Relation, Some(false)),
                ("s", Form::Relation, Some(true)),
            ]
        );
    }

    /// PostgreSQL 18.6 looks an unqualified `FROM` name up among the visible `WITH` queries first
    /// (`parse_clause.c`, `getNSItemForSpecialRelationTypes`: "schema-qualified names are never
    /// CTE ... refs"): a relation over the CTE's name uses it, as does a qualifier naming it. A
    /// non-recursive WITH query sees only the ones before it, not itself (`parse_cte.c`,
    /// `transformWithClause`); a recursive one sees them all. An inner `WITH` query of the same
    /// name shadows an outer one.
    #[test]
    fn ctes_are_used_by_the_relations_that_select_from_them() {
        for (source, cte) in [
            ("WITH c AS (SELECT 1 AS a) SELECT a FROM c", Some(true)),
            ("WITH C AS (SELECT 1 AS a) SELECT a FROM c AS d", Some(true)),
            ("WITH c AS (SELECT 1 AS a) SELECT c.a FROM d", Some(true)),
            ("WITH c AS (SELECT 1 AS a) SELECT 1 FROM d", Some(false)),
            ("WITH c AS (SELECT 1 AS a) SELECT 1 FROM s.c", Some(false)),
            ("WITH \"C\" AS (SELECT 1 AS a) SELECT 1 FROM c", Some(false)),
            ("WITH c AS (SELECT 1 FROM c) SELECT 1", Some(false)),
            ("WITH c AS (SELECT 1) UPDATE t SET a = 1 FROM c", Some(true)),
            // `UPDATE c` writes the table `c`, never the WITH query.
            ("WITH c AS (SELECT 1) UPDATE c SET a = 1", Some(false)),
            (
                "WITH RECURSIVE c AS (SELECT 1 UNION ALL SELECT 1 FROM c) SELECT 1",
                Some(true),
            ),
        ] {
            let ctes: Vec<_> = used(source)
                .into_iter()
                .filter(|(_, form, _)| *form == Form::Cte)
                .map(|(_, _, used)| used)
                .collect();
            assert_eq!(ctes, vec![cte], "{source}");
        }
        // `b`'s body cannot see the later `c`; the main query uses `b`.
        let source = "WITH b AS (SELECT 1 FROM c), c AS (SELECT 1) SELECT 1 FROM b";
        assert_eq!(
            used(source)
                .into_iter()
                .filter(|(_, form, _)| *form == Form::Cte)
                .collect::<Vec<_>>(),
            vec![("b", Form::Cte, Some(true)), ("c", Form::Cte, Some(false))]
        );
        let source =
            "WITH c AS (SELECT 1) SELECT 1 FROM (WITH c AS (SELECT 2) SELECT 1 FROM c) AS s";
        assert_eq!(
            used(source)
                .into_iter()
                .filter(|(_, form, _)| *form == Form::Cte)
                .collect::<Vec<_>>(),
            vec![("c", Form::Cte, Some(false)), ("c", Form::Cte, Some(true))]
        );
    }

    /// A DML target (the `target` form: `DELETE FROM` and `INSERT INTO` name theirs outside any
    /// relation) is used by the statement that writes it, whether or not anything qualifies it.
    #[test]
    fn dml_targets_are_used_by_their_statement() {
        for (source, target) in [
            ("DELETE FROM t WHERE true", "t"),
            ("DELETE FROM s.t AS d WHERE true", "d"),
            ("INSERT INTO t (a) VALUES (1)", "t"),
        ] {
            assert_eq!(
                used(source),
                vec![(target, Form::Target, Some(true))],
                "{source}"
            );
        }
    }

    #[test]
    fn reproduces_sqlfluff_rf01_and_al05() {
        let recording = json("fixtures.expected.json");
        let mut recorded: BTreeSet<Finding> = BTreeSet::new();
        for finding in recording["findings"].as_array().unwrap() {
            let rule = finding["rule"].as_str().unwrap();
            if rule == "RF01" || rule == "AL05" {
                recorded.insert((
                    rule.to_owned(),
                    finding["path"].as_str().unwrap().to_owned(),
                    finding["line"].as_u64().unwrap(),
                    finding["column"].as_u64().unwrap(),
                ));
            }
        }
        let mut reproduced: BTreeSet<Finding> = BTreeSet::new();
        for path in recording["source"]["files"].as_array().unwrap() {
            let path = path.as_str().unwrap();
            let source =
                std::fs::read_to_string(sqlfluff(&format!("fixtures/{path}"))).expect("fixture");
            let grep = GrammarDialect::Sql.ast_grep(&source);
            let scopes = SqlScopes::read(&grep.root());
            for (rule, ranges) in [
                ("RF01", scopes.unresolved()),
                ("AL05", scopes.unused_aliases()),
            ] {
                for range in ranges {
                    let (line, column) = position(&source, range.start);
                    reproduced.insert((rule.to_owned(), path.to_owned(), line, column));
                }
            }
        }
        let mut missing: Vec<Finding> = recorded.difference(&reproduced).cloned().collect();
        let mut extra: Vec<Finding> = reproduced.difference(&recorded).cloned().collect();
        for (path, rule, line, column, kind, _reason) in MISMATCHES {
            let finding = (rule.to_string(), path.to_string(), *line, *column);
            let unexplained = match kind {
                Mismatch::Missing => &mut missing,
                Mismatch::Extra => &mut extra,
            };
            let Some(position) = unexplained.iter().position(|f| *f == finding) else {
                panic!("stale mismatch entry {finding:?} ({kind:?})");
            };
            unexplained.remove(position);
        }
        assert!(missing.is_empty(), "recorded, not reproduced: {missing:#?}");
        assert!(extra.is_empty(), "reproduced, not recorded: {extra:#?}");
        assert_eq!(recorded.len(), 44, "RF01 32 + AL05 12 recorded");
        // The recorded findings less the 3 `Missing` entries, plus the 11 `Extra` ones.
        assert_eq!(reproduced.len(), 52);
    }
