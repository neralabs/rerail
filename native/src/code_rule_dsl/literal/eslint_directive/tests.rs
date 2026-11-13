    use super::*;

    /// A parsed directive: its label, value, description, and items with their settings.
    type Parsed = (
        String,
        String,
        Option<String>,
        Vec<(String, Option<String>)>,
    );

    fn parsed(comment: &str) -> Option<Parsed> {
        let directive = Directive::parse(comment)?;
        let items = directive
            .items(comment)
            .into_iter()
            .map(|item| {
                assert_eq!(&comment[item.range.clone()], item.name);
                (
                    item.name,
                    item.setting.map(|(setting, range)| {
                        assert_eq!(&comment[range], setting);
                        setting
                    }),
                )
            })
            .collect();
        Some((
            directive.label.clone(),
            comment[directive.value_range.clone()].to_owned(),
            directive.description.map(|range| comment[range].to_owned()),
            items,
        ))
    }

    #[test]
    fn directives_parse_as_eslint_parses_them() {
        let item = |name: &str| (name.to_owned(), None);
        assert_eq!(
            parsed("/* eslint-disable a, 'b' , \"c\" -- because */"),
            Some((
                "eslint-disable".into(),
                "a, 'b' , \"c\"".into(),
                Some("because".into()),
                vec![item("a"), item("b"), item("c")]
            ))
        );
        assert_eq!(
            parsed("// eslint-disable-next-line"),
            Some((
                "eslint-disable-next-line".into(),
                String::new(),
                None,
                vec![]
            ))
        );
        // A `//` comment holds only the line directives.
        assert_eq!(parsed("// eslint-disable a"), None);
        assert_eq!(parsed("// global a"), None);
        // The label ends at white space; `eslint-disablex` is no directive.
        assert_eq!(parsed("/* eslint-disablex */"), None);
        assert_eq!(
            parsed("/*eslint-enable*/").map(|parsed| parsed.0),
            Some("eslint-enable".into())
        );
        assert_eq!(parsed("/* Eslint-disable */"), None);
        // The description needs white space on both sides of its dashes.
        assert_eq!(
            parsed("/* eslint-disable a--b -- c -- d */").map(|parsed| (parsed.1, parsed.2)),
            Some(("a--b".into(), Some("c -- d".into())))
        );
        assert_eq!(
            parsed("/* global a:off, b : readonly c\u{a0}d,,e:writable:x a:true */"),
            Some((
                "global".into(),
                "a:off, b : readonly c\u{a0}d,,e:writable:x a:true".into(),
                None,
                vec![
                    ("a".into(), Some("true".into())),
                    ("b".into(), Some("readonly".into())),
                    item("c"),
                    item("d"),
                    ("e".into(), Some("writable".into())),
                ]
            ))
        );
        assert_eq!(
            parsed("/*globals*/"),
            Some(("globals".into(), String::new(), None, vec![]))
        );
        assert_eq!(
            parsed("/* exported a, a */").map(|parsed| parsed.3),
            Some(vec![item("a")])
        );
        assert_eq!(
            parsed("/* eslint no-alert: 2 */").map(|parsed| parsed.3),
            Some(vec![])
        );
        assert_eq!(parsed("/* just a comment */"), None);
        assert_eq!(parsed("/**/"), None);
    }

    fn found(
        rule: serde_json::Value,
        anchor: &str,
        each: Option<&str>,
        source: &str,
    ) -> serde_json::Value {
        use crate::code_rule_dsl::test_support::*;
        let mut violation = serde_json::json!({ "anchor": anchor, "code": "X", "message": if anchor == "witness.span" { "x" } else { "$N" } });
        if let Some(each) = each {
            violation["each"] = serde_json::json!(each);
        }
        let compiled = compile_document_with(
            clause_document(serde_json::json!({ "id": "c", "violation": violation,
                "forbid": { "ast": { "language": "typescript", "rule": rule } } })),
            &serde_json::from_str::<serde_json::Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        evaluate_file(&compiled, "typescript", "a.ts", source)
    }

    fn texts(result: &serde_json::Value, source: &str) -> Vec<String> {
        result["violations"]
            .as_array()
            .expect("findings")
            .iter()
            .map(|violation| {
                let span = &violation["span"];
                let (start, end) = (
                    span["startByte"].as_u64().unwrap() as usize,
                    span["endByte"].as_u64().unwrap() as usize,
                );
                format!(
                    "{}={}",
                    &source[start..end],
                    violation["message"].as_str().unwrap()
                )
            })
            .collect()
    }

    #[test]
    fn a_directive_comment_parses_into_a_tree_anchored_in_the_comment() {
        use serde_json::json;
        let source = "/* global a, b:off -- why */ x;\n// eslint-disable-next-line no-alert, 'semi'\n// eslint-disable a\n/* not one */\nconst s = '/* global z */';\n";
        let items = json!({ "kind": "comment", "parse": { "grammar": "eslintDirective", "has": { "kind": "item", "capture": "N" } } });
        // Only comments ESLint reads as directives parse: the line comment's block directive does not.
        assert_eq!(
            texts(&found(items.clone(), "capture.N", None, source), source),
            ["a=a", "no-alert=no-alert"]
        );
        assert_eq!(
            texts(&found(items, "capture.N", Some("N"), source), source),
            ["a=a", "b=b", "no-alert=no-alert", "semi=semi"]
        );
        let settings = json!({ "kind": "comment", "parse": { "grammar": "eslintDirective",
            "has": { "kind": "item", "has": { "kind": "setting", "value": { "eq": "off" }, "capture": "N" } } } });
        assert_eq!(
            texts(&found(settings, "capture.N", None, source), source),
            ["off=off"]
        );
        let described = json!({ "kind": "comment", "parse": { "grammar": "eslintDirective",
            "value": { "eq": "global" }, "has": { "kind": "description", "capture": "N" } } });
        assert_eq!(
            texts(&found(described, "capture.N", None, source), source),
            ["why=why"]
        );
        // Reading the comment alone, it holds in a file with syntax errors, and negated there too.
        let broken = "/* global a */\nlet = ;\n";
        let not_directive = json!({ "kind": "comment", "not": { "parse": { "grammar": "eslintDirective", "kind": "directive" } } });
        let result = found(not_directive, "witness.span", None, broken);
        assert_eq!(result["failures"][0]["code"], "source-partial", "{result}");
        assert_eq!(texts(&result, broken), Vec::<String>::new());
    }
