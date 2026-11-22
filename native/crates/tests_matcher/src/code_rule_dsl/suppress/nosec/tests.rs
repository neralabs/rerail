    use super::*;
    use crate::code_rule_dsl::lines::Terminators;

    #[test]
    fn nosec_comments_parse_as_bandit_parses_them() {
        assert_eq!(parse("# a comment"), None);
        assert_eq!(parse("# NOSEC"), None, "case-sensitive");
        assert_eq!(parse("# nosec"), Some(vec![]));
        assert_eq!(parse("#nosec"), Some(vec![]));
        assert_eq!(parse("# nosec:"), Some(vec![]));
        assert_eq!(parse("# nosec B101"), Some(vec!["B101"]));
        assert_eq!(parse("# nosec: B101"), Some(vec!["B101"]));
        assert_eq!(parse("# nosec B101, B102"), Some(vec!["B101", "B102"]));
        assert_eq!(parse("# nosec B101 B102"), Some(vec!["B101", "B102"]));
        // One repetition of `(?:(...),?)+` without a space keeps only its last test.
        assert_eq!(parse("# nosec B101,B102"), Some(vec!["B102"]));
        assert_eq!(parse("# nosec assert_used"), Some(vec!["B101"]));
        assert_eq!(parse("# nosec random, B602"), Some(vec!["B311", "B602"]));
        // Unknown words name no test; a list of them only is a blanket nosec.
        assert_eq!(parse("# nosec TODO"), Some(vec![]));
        assert_eq!(
            parse("# nosec b101"),
            Some(vec![]),
            "ids are case-sensitive"
        );
        assert_eq!(parse("# nosec B101 TODO"), Some(vec!["B101"]));
        assert_eq!(parse("# nosec B999"), Some(vec![]));
        assert_eq!(parse("# nosec B001"), Some(vec!["B001"]));
        // The list ends at the next `#`; `# nosec` may follow another comment.
        assert_eq!(parse("# nosec B101 # B102"), Some(vec!["B101"]));
        assert_eq!(parse("# reason # nosec B602"), Some(vec!["B602"]));
        assert_eq!(parse("# nosecB101"), Some(vec!["B101"]));
    }

    fn read(source: &str) -> (NosecIndex, LineTable) {
        let lines = LineTable::new(
            source,
            Terminators::of(crate::code_rule_dsl::language::GrammarDialect::Python),
        );
        let mut index = NosecIndex::default();
        for (at, _) in source.match_indices('#') {
            let line = lines.line(at);
            if index.lines.contains_key(&line) {
                continue;
            }
            let end = lines.end(line);
            index.lines.insert(line, parse(&source[at..end]));
        }
        (index, lines)
    }

    fn codes(codes: &[&str]) -> Vec<String> {
        codes.iter().map(|code| (*code).to_owned()).collect()
    }

    #[test]
    fn nosec_suppresses_on_the_reported_line_and_the_nodes_lines() {
        let source = "a = 1  # nosec\nb = 2  # nosec B101\nc = 3  # nosec B102\nd = 4  # comment\ncall(\n    x,  # nosec B602\n)\ne = 5\n";
        let (index, lines) = read(source);
        let line = |n: usize| (lines.start(n), lines.end(n));
        let at = |n: usize| lines.start(n);
        // Bare: every test.
        assert!(index.suppresses(&codes(&["B101"]), at(0), line(0), &lines));
        // Coded: only the named tests.
        assert!(index.suppresses(&codes(&["B101"]), at(1), line(1), &lines));
        assert!(!index.suppresses(&codes(&["B101"]), at(2), line(2), &lines));
        // Another comment, or none, suppresses nothing.
        assert!(!index.suppresses(&codes(&["B101"]), at(3), line(3), &lines));
        assert!(!index.suppresses(&codes(&["B101"]), at(7), line(7), &lines));
        // A node spanning lines 4 to 6: the first `# nosec` among its lines counts.
        let call = (lines.start(4), lines.end(6));
        assert!(index.suppresses(&codes(&["B602"]), at(4), call, &lines));
        assert!(!index.suppresses(&codes(&["B603"]), at(4), call, &lines));
        // The reported line's tests join the node's.
        let both = "x = f(  # nosec B101\n    y)  # nosec B102\n";
        let (index, lines) = read(both);
        let node = (lines.start(0), lines.end(1));
        assert!(index.suppresses(&codes(&["B101"]), lines.start(1), node, &lines));
        assert!(index.suppresses(&codes(&["B102"]), lines.start(1), node, &lines));
        assert!(!index.suppresses(&codes(&["B103"]), lines.start(1), node, &lines));
        // A blanket nosec on the reported line joined with a coded one on the node's first line
        // names tests, so it is no longer blanket.
        let mixed = "x = f(  # nosec B101\n    y)  # nosec\n";
        let (index, lines) = read(mixed);
        let node = (lines.start(0), lines.end(1));
        assert!(!index.suppresses(&codes(&["B102"]), lines.start(1), node, &lines));
    }

    /// Through the engine: a rule reporting every `assert`, with `suppress.nosec` naming B101,
    /// keeps exactly the asserts Bandit 1.9.4 keeps (checked with `bandit -t B101`).
    #[test]
    fn nosec_suppresses_violations_through_the_engine() {
        use crate::code_rule_dsl::test_support::*;
        use serde_json::{Value, json};
        let violation = json!({ "anchor": "witness.span", "code": "B101", "message": "assert",
            "suppress": { "nosec": { "codes": ["B101"] } } });
        let document = clause_document(json!({ "id": "assert", "violation": violation,
            "forbid": { "ast": { "language": "python", "rule": { "kind": "assert_statement" } } } }));
        let compiled = compile_document_with(
            document,
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "\
assert a  # nosec
assert b  # nosec B101
assert c  # nosec assert_used
assert d  # nosec B102
assert e  # nosec TODO
assert f  # noqa
assert g
assert (h,
        i)  # nosec
x = '# nosec'; assert j
";
        let result = evaluate_file(&compiled, "python", "a.py", source);
        assert_eq!(result["failures"], json!([]), "{result}");
        let lines = result["violations"]
            .as_array()
            .expect("violations")
            .iter()
            .map(|violation| violation["span"]["startLine"].as_u64().expect("line"))
            .collect::<Vec<_>>();
        // 0-based: lines 4 (another test), 6 (`# noqa`), 7 (no comment) and 10 (`# nosec` in a
        // string), as Bandit reports them.
        assert_eq!(lines, [3, 5, 6, 9]);
    }
