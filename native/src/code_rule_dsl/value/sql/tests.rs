    use super::*;
    use serde_json::Value as Json;

    fn oracle_root(name: &str) -> Json {
        let path = std::path::Path::new(crate::code_rule_dsl::fixtures::fixtures_dir!()).join(name);
        serde_json::from_str(&std::fs::read_to_string(path).expect("oracle")).expect("json")
    }

    #[test]
    fn sqlite_oracle_values() {
        let root = oracle_root("sql/value/sqlite.expected.json");
        let cases = root["cases"].as_array().unwrap();
        let mut checked = 0;
        for case in cases {
            let text = case["literal"].as_str().unwrap();
            if matches!(text, "\"a\"" | "\"b\"" | "[a]" | "`a`" | "$$x$$") {
                continue;
            }
            let got = literal(text);
            let expected = if let Some(error) = case.get("error") {
                if matches!(text, "0b101" | "0o17" | "N'nat'" | "E'a'" | "'a'\n'b'") {
                    assert!(literal(text).is_known(), "{text}: {error}");
                } else {
                    assert!(matches!(got, Value::Unknown), "{text}: {error}");
                }
                continue;
            } else {
                match case["type"].as_str().unwrap() {
                    "integer" => {
                        Value::int(case["quote"].as_str().unwrap().parse::<i64>().unwrap())
                    }
                    "real" => {
                        let value: f64 = case["quote"].as_str().unwrap().parse().unwrap();
                        if !value.is_finite() {
                            assert!(matches!(got, Value::Unknown), "{text}");
                            continue;
                        }
                        Value::Float(value)
                    }
                    "text" => Value::Str(
                        case["quote"].as_str().unwrap()
                            [1..case["quote"].as_str().unwrap().len() - 1]
                            .replace("''", "'"),
                    ),
                    "null" => Value::None,
                    "blob" => {
                        assert!(matches!(got, Value::Unknown), "{text}");
                        continue;
                    }
                    kind => panic!("unexpected SQLite type {kind}"),
                }
            };
            if matches!(text, "TRUE" | "false" | "FALSE" | "true") {
                assert!(matches!(got, Value::Bool(_)), "{text}");
            } else if matches!(text, "0xFFFFFFFFFFFFFFFF" | "9223372036854775808") {
                assert!(matches!(got, Value::Unknown), "{text}");
            } else {
                assert_eq!(got, expected, "{text}");
            }
            checked += 1;
        }
        assert_eq!(checked, 27);
    }

    /// Byte offset of the 1-based (`line`, `column` in code points) position.
    fn byte_offset(source: &str, line: u64, column: u64) -> usize {
        let mut start = 0;
        for _ in 1..line {
            start += source[start..].find('\n').expect("line exists") + 1;
        }
        let rest = &source[start..];
        let skip = rest
            .char_indices()
            .nth(column as usize - 1)
            .map_or(rest.len(), |(i, _)| i);
        start + skip
    }

    /// The values `decode` may give for an oracle constant: a number written as an
    /// integer may be read as a float (`5.`), as PostgreSQL's own text drops the point.
    fn expected_constant(value: &Json) -> Vec<Value> {
        match value[0].as_str().unwrap() {
            "number" => {
                let text = value[1].as_str().unwrap();
                match text.parse::<BigInt>() {
                    Ok(n) if n.to_i64().is_some() => {
                        let float = text.parse::<f64>().unwrap();
                        vec![Value::Int(n), Value::Float(float)]
                    }
                    Ok(_) => vec![Value::Unknown],
                    Err(_) => vec![Value::Float(text.parse::<f64>().unwrap())],
                }
            }
            "string" => vec![Value::Str(value[1].as_str().unwrap().to_string().into())],
            "bool" => vec![Value::Bool(value[1].as_bool().unwrap())],
            "null" => vec![Value::None],
            "bits" => vec![Value::Unknown],
            kind => panic!("unexpected constant kind {kind}"),
        }
    }

    /// Oracle items the tree cannot match: (line, column, reason).
    const POSTGRES_EXCEPTIONS: &[(u64, u64, &str)] = &[
        (
            6,
            36,
            "the grammar parses `ONLY u` as table `ONLY` aliased `u`",
        ),
        (
            6,
            63,
            "the grammar parses `LATERAL (SELECT z FROM q)` as a call, `z` an alias",
        ),
        (
            18,
            25,
            "PostgreSQL locates `N'nat'` at its string; the literal starts at the `N`",
        ),
        (
            19,
            45,
            "PostgreSQL locates `INTERVAL '1 day'` at its string; the grammar has no literal there",
        ),
        (
            19,
            59,
            "PostgreSQL locates `DATE '2020-01-01'` at its string; the literal starts at `DATE`",
        ),
        (
            19,
            77,
            "PostgreSQL locates `int '3'` at its string; the literal starts at `int`",
        ),
    ];

    #[test]
    fn postgres_oracle_by_position() {
        use crate::code_rule_dsl::language::GrammarDialect;
        use ast_grep_language::LanguageExt;
        let source = std::fs::read_to_string(
            std::path::Path::new(crate::code_rule_dsl::fixtures::fixtures_dir!())
                .join("sql/scope/postgres.sql"),
        )
        .expect("postgres.sql");
        let grep = GrammarDialect::Sql.ast_grep(&source);
        let nodes: Vec<_> = grep.root().dfs().collect();
        let oracle = oracle_root("sql/scope/postgres.expected.json");
        let excepted = |line: u64, column: u64| {
            POSTGRES_EXCEPTIONS
                .iter()
                .any(|&(l, c, _)| l == line && c == column)
        };
        let mut problems: Vec<String> = Vec::new();
        let (mut constants, mut names) = (0, 0);
        let (mut total, mut visited, mut skipped) = (0, 0, 0);
        for statement in oracle["statements"].as_array().unwrap() {
            for item in statement["constants"].as_array().unwrap() {
                let (line, column) = (
                    item["line"].as_u64().unwrap(),
                    item["column"].as_u64().unwrap(),
                );
                total += 1;
                if excepted(line, column) {
                    skipped += 1;
                    continue;
                }
                visited += 1;
                let at = byte_offset(&source, line, column);
                let Some(node) = nodes
                    .iter()
                    .find(|n| n.kind() == "literal" && n.range().start == at)
                else {
                    problems.push(format!("no literal at {line}:{column}"));
                    continue;
                };
                let acceptable = expected_constant(&item["value"]);
                let got = decode(node);
                if acceptable.contains(&got) {
                    constants += 1;
                } else {
                    problems.push(format!(
                        "{line}:{column} {}: {got:?} not in {acceptable:?}",
                        node.text()
                    ));
                }
            }
            for category in ["relations", "columns"] {
                for item in statement[category].as_array().unwrap() {
                    let (line, column) = (
                        item["line"].as_u64().unwrap(),
                        item["column"].as_u64().unwrap(),
                    );
                    total += 1;
                    if excepted(line, column) {
                        skipped += 1;
                        continue;
                    }
                    visited += 1;
                    let at = byte_offset(&source, line, column);
                    let parts: Vec<&str> = item["value"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| p.as_str().unwrap())
                        .filter(|p| *p != "*")
                        .collect();
                    // The reference's identifiers in order: a `field`'s qualifier
                    // (an `object_reference`) followed by its own name.
                    let Some(reference) = nodes.iter().find(|n| {
                        matches!(
                            n.kind().as_ref(),
                            "object_reference" | "field" | "all_fields"
                        ) && n.range().start == at
                    }) else {
                        // `*` alone has no reference node.
                        if !parts.is_empty() {
                            problems.push(format!("no reference at {line}:{column}"));
                        }
                        continue;
                    };
                    let found: Vec<String> = reference
                        .dfs()
                        .filter(|n| n.kind() == "identifier")
                        .map(|n| node_name(&n).expect("identifier name"))
                        .collect();
                    if found == parts {
                        names += parts.len();
                    } else {
                        problems.push(format!(
                            "{category} {line}:{column}: {found:?} != {parts:?}"
                        ));
                    }
                }
            }
        }
        assert!(problems.is_empty(), "{}", problems.join("\n"));
        assert!(constants > 0 && names > 0);
        assert_eq!(skipped, POSTGRES_EXCEPTIONS.len());
        assert_eq!(visited + skipped, total);
    }

    #[test]
    fn equals_sql_three_valued() {
        assert_eq!(equals(&Value::None, &Value::None), None);
        assert_eq!(equals(&Value::int(1), &Value::Float(1.0)), Some(true));
        assert_eq!(equals(&Value::int(1), &Value::Float(1.5)), Some(false));
        assert_eq!(equals(&Value::Bool(true), &Value::Bool(false)), Some(false));
        assert_eq!(
            equals(&Value::Str("A".into()), &Value::Str("a ".into())),
            Some(false)
        );
        assert_eq!(
            equals(&Value::Str("é".into()), &Value::Str("É".into())),
            None
        );
        assert_eq!(equals(&Value::Str("1".into()), &Value::int(1)), None);
    }

    #[test]
    fn names_follow_postgres_rules() {
        assert_eq!(name("Mixed_CASE"), Some("mixed_case".into()));
        assert_eq!(name("\"Mixed_CASE\""), Some("Mixed_CASE".into()));
        assert_eq!(name("\"x\"\"y\""), Some("x\"y".into()));
        assert_eq!(name("`x`"), None);
        assert_eq!(name("\"\""), None);
    }

    /// The literals of `source`, in order, as `decode` reads them; the source parses cleanly.
    fn decoded(source: &str) -> Vec<Value> {
        use crate::code_rule_dsl::language::GrammarDialect;
        use ast_grep_language::LanguageExt;
        let grep = GrammarDialect::Sql.ast_grep(source);
        assert!(!grep.root().get_inner_node().has_error(), "{source}");
        grep.root()
            .dfs()
            .filter(|node| node.kind() == "literal")
            .map(|node| decode(&node))
            .collect()
    }

    #[test]
    fn negative_floats_carry_their_sign_once() {
        assert_eq!(literal("-1.5"), Value::Float(-1.5));
        assert_eq!(literal("-1e2"), Value::Float(-100.0));
        assert_eq!(literal("- 2.25"), Value::Float(-2.25));
        assert_eq!(literal("+1.5"), Value::Float(1.5));
        assert_eq!(literal("--1.5"), Value::Unknown);
        assert_eq!(
            decoded("SELECT -1.5, -1e2, 2.5"),
            [Value::Float(-1.5), Value::Float(-100.0), Value::Float(2.5)]
        );
    }

    /// A literal whose scale (exponent less fraction digits, plus trailing zeros) leaves `i32`'s
    /// bounds is no value, not an overflow.
    #[test]
    fn extreme_exponents_are_unknown() {
        for text in [
            "1.5e-2147483648",
            "10e2147483647",
            "100e2147483646",
            "-1.5e-2147483648",
        ] {
            assert_eq!(literal(text), Value::Unknown, "{text}");
        }
    }

    #[test]
    fn escape_string_octal_and_hex_escapes_are_bytes() {
        assert_eq!(literal(r"E'\xC3\xA9'"), Value::Str("é".into()));
        assert_eq!(literal(r"E'\303\251'"), Value::Str("é".into()));
        assert_eq!(
            literal(r"e'caf\xc3\251 \u00e9!'"),
            Value::Str("café é!".into())
        );
        assert_eq!(literal(r"E'\x41\101'"), Value::Str("AA".into()));
        // PostgreSQL keeps an octal escape's low byte: `\501` is 0x141, the byte 0x41.
        assert_eq!(literal(r"E'\501'"), Value::Str("A".into()));
        // Bytes that are not UTF-8 make no string PostgreSQL accepts in a UTF-8 database.
        assert_eq!(literal(r"E'\xC3'"), Value::Unknown);
        assert_eq!(literal(r"E'\377'"), Value::Unknown);
        assert_eq!(literal(r"E'a\0b'"), Value::Unknown);
    }

    #[test]
    fn lowercase_unicode_prefixes_decode() {
        assert_eq!(name(r#"u&"d\0061t""#), Some("dat".into()));
        assert_eq!(name(r#"U&"d\0061t""#), Some("dat".into()));
        assert_eq!(identifier(Cow::Borrowed(r#"u&"d\0061t""#)), "dat");
        assert_eq!(literal(r"u&'d\0061t'"), Value::Str("dat".into()));
        assert_eq!(literal(r"U&'d\0061t'"), Value::Str("dat".into()));
    }

    #[test]
    fn dollar_quoted_strings_end_at_their_own_tag() {
        for (source, body) in [
            // A candidate tag that does not match leaves its `$` to start the next one.
            ("SELECT $a$foo$bar$a$;", "foo$bar"),
            ("SELECT $a$x$$a$;", "x$"),
            ("SELECT $ab$x$a$ab$;", "x$a"),
            // Tags compare by whole code points: U+0161 is not `a` (0x61).
            ("SELECT $š$before $a$ after$š$;", "before $a$ after"),
            ("SELECT $東$a$q$東$;", "a$q"),
        ] {
            assert_eq!(decoded(source), [Value::Str(body.into())], "{source}");
        }
    }

    /// A dollar-quote tag is empty or an identifier (grammar patch 23), so `$1,$2` is two
    /// positional parameters: the tag `1,` used to open a string that ran to the next `$1,$`.
    #[test]
    fn positional_parameters_are_not_dollar_quote_tags() {
        use crate::code_rule_dsl::language::GrammarDialect;
        use ast_grep_language::LanguageExt;
        let source = "INSERT INTO t VALUES ($1,$2);\nSELECT 1 FROM t WHERE a = NULL;\n\
                      INSERT INTO t VALUES ($1,$2);\nSELECT $_x1$a$1,$_x1$, $é1$b$é1$;\n";
        let grep = GrammarDialect::Sql.ast_grep(source);
        assert!(!grep.root().get_inner_node().has_error(), "{source}");
        let parameters = grep
            .root()
            .dfs()
            .filter(|n| n.kind() == "parameter")
            .count();
        assert_eq!(parameters, 4);
        assert_eq!(
            decoded(source),
            [
                Value::int(1),
                Value::None,
                Value::Str("a$1,".into()),
                Value::Str("b".into())
            ]
        );
    }

    /// The review's repro, through the SQL pack's CV05 port: the file parses clean, so the
    /// `= NULL` on line 2 is reported rather than the file skipped as incomplete.
    #[test]
    fn cv05_reports_beside_positional_parameters() {
        use crate::code_rule_dsl::CodeRuleDslRuntime;
        use crate::code_rule_dsl::test_support::{CAPABILITIES, evaluate_file};
        let rule = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../packs/sql/rules/sqlfluff-cv05-convention-is-null.json");
        let document = std::fs::read_to_string(rule).expect("CV05 rule");
        let compiled =
            CodeRuleDslRuntime::compile(document, "json".to_owned(), CAPABILITIES.to_owned())
                .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "INSERT INTO t (a, b) VALUES ($1,$2);\nSELECT 1 FROM t WHERE a = NULL;\n\
                      INSERT INTO t (a, b) VALUES ($1,$2);\n";
        let result = evaluate_file(&compiled, "sql", "q.sql", source);
        assert_eq!(result["failures"], serde_json::json!([]), "{result}");
        let violations = result["violations"].as_array().expect("violations");
        assert_eq!(violations.len(), 1, "{result}");
        // `startLine` counts from 0: line 2.
        assert_eq!(violations[0]["span"]["startLine"], 1, "{result}");
    }

    /// A `$` that starts no tag fails at once: upstream read `$1,$`, `$2),($` and so on as tags
    /// and scanned each to the end of the file for its close, quadratic in the parameters.
    #[test]
    fn many_positional_parameters_parse_in_linear_time() {
        use crate::code_rule_dsl::language::GrammarDialect;
        use ast_grep_language::LanguageExt;
        let rows: Vec<String> = (0..10_000)
            .map(|i| format!("(${},${})", 2 * i + 1, 2 * i + 2))
            .collect();
        let source = format!("INSERT INTO t VALUES {};\n", rows.join(","));
        let started = std::time::Instant::now();
        let grep = GrammarDialect::Sql.ast_grep(&source);
        let elapsed = started.elapsed();
        assert!(!grep.root().get_inner_node().has_error());
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "{} bytes took {elapsed:?}",
            source.len()
        );
    }
