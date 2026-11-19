    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use ast_grep_language::LanguageExt;

    fn read(source: &str) -> OxlintDirectives {
        let root = GrammarDialect::Tsx.ast_grep(source);
        OxlintDirectives::read(&root.root(), source)
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    /// The span of the `nth` (0-based) occurrence of `needle`.
    fn nth(source: &str, needle: &str, nth: usize) -> Range<usize> {
        let (at, _) = source
            .match_indices(needle)
            .nth(nth)
            .unwrap_or_else(|| panic!("{needle} #{nth}"));
        at..at + needle.len()
    }

    /// The `debugger` statements a rule named `rule` keeps reported.
    fn reported(source: &str, rule: &str) -> Vec<usize> {
        let directives = read(source);
        let count = source.matches("debugger;").count();
        (0..count)
            .filter(|&index| {
                !directives.suppresses(&names(&[rule]), nth(source, "debugger;", index))
            })
            .collect()
    }

    /// oxc's `test` (no-debugger's pass and fail cases), for both prefixes.
    #[test]
    fn directives_suppress_as_oxlint_applies_them() {
        for prefix in ["eslint", "oxlint"] {
            for pass in [
                format!("/* {prefix}-disable */\n    debugger;\n/* {prefix}-enable */\n"),
                format!(
                    "/* {prefix}-disable no-debugger, no-console */\n    debugger;\n/* {prefix}-enable no-debugger, no-console */\n"
                ),
                format!("/* {prefix}-disable */\n    debugger;\n"),
                format!("/* {prefix}-disable no-debugger */\n    debugger;\n"),
                format!(
                    "debugger; // {prefix}-disable-line\ndebugger; // {prefix}-disable-line\n\n// {prefix}-disable-next-line\ndebugger;\n\n/* {prefix}-disable-next-line */\ndebugger;\n\ndebugger; /* {prefix}-disable-line */\n"
                ),
                format!(
                    "debugger; // {prefix}-disable-line no-debugger, quotes, semi\n\n/* {prefix}-disable-next-line\n  no-debugger,\n  quotes,\n  semi\n*/\ndebugger;\n"
                ),
                format!(
                    "/* {prefix}-disable */\n    debugger;\n/* {prefix}-disable */\n    debugger;\n"
                ),
                format!(
                    "// {prefix}-disable-next-line no-debugger -- Here's a description.\ndebugger;\n\n/* {prefix}-disable-next-line no-debugger --\n * A long description\n**/\ndebugger;\n"
                ),
                format!(
                    "// {prefix}-disable-next-line no-alert no-debugger - Here's a description.\ndebugger;\n"
                ),
                format!(
                    "/* {prefix}-disable */\n    debugger;\n/* {prefix}-enablefoo */\n    debugger;\n"
                ),
                format!(
                    "debugger; //{prefix}-disable-line\n/*{prefix}-disable-next-line*/\ndebugger;\n"
                ),
                format!(
                    "debugger; // \t\t {prefix}-disable-line\n/*    \t   {prefix}-disable-next-line no-debugger       */\ndebugger;\n"
                ),
                format!("/*{prefix}-disable\nno-debugger\n*/\n    debugger;\n"),
                format!(
                    "debugger; // {prefix}-disable-line ,,no-debugger,,semi,,\n// {prefix}-disable-next-line ,  , ,,no-debugger, , ,\ndebugger;\n"
                ),
                format!("debugger;//{prefix}-disable-line"),
            ] {
                assert_eq!(
                    reported(&pass, "no-debugger"),
                    Vec::<usize>::new(),
                    "{pass}"
                );
            }
            for (fail, expected) in [
                ("debugger;".to_owned(), vec![0]),
                (
                    format!(
                        "debugger; // {prefix}-disable-line no-alert\n\n// {prefix}-disable-next-line no-alert\ndebugger;\n"
                    ),
                    vec![0, 1],
                ),
                (
                    format!(
                        "/* {prefix}-disable-next-line no-debugger --\n * A long description\n**/\ndebugger;\ndebugger;\n"
                    ),
                    vec![1],
                ),
                (
                    format!("// {prefix}-disable-next-line no-debugger\ndebugger;\ndebugger;\n"),
                    vec![1],
                ),
                (
                    format!("// {prefix}-disable-lext-nine no-debugger\ndebugger;\n"),
                    vec![0],
                ),
                (
                    format!("// {prefix}-disabled no-debugger\ndebugger;\n"),
                    vec![0],
                ),
                (
                    format!("debugger; // {prefix}-disable-linefoo no-debugger"),
                    vec![0],
                ),
                (
                    format!(
                        "/* {prefix}-disable */\n    debugger;\n/* {prefix}-enable */\n    debugger;\n"
                    ),
                    vec![1],
                ),
                (
                    format!(
                        "/*{prefix}-disable no-debugger,no-console*/\n    debugger;\n/*{prefix}-enable no-debugger,no-console*/\n    debugger;\n"
                    ),
                    vec![1],
                ),
            ] {
                assert_eq!(reported(&fail, "no-debugger"), expected, "{fail}");
            }
        }
    }

    /// Cases checked against `oxlint` 1.88.0 itself (`no-debugger`): the debugger statements it
    /// still reports.
    #[test]
    fn directive_edges_match_oxlint() {
        let source = "\
debugger; // oxlint-disable-line no-debugger
/* oxlint-disable-line no-debugger */ debugger;
debugger; /* oxlint-disable-line
  no-debugger */
/* oxlint-disable-line
  no-debugger */ debugger;
/* oxlint-disable-next-line no-debugger */ debugger;
debugger;
// oxlint-disable-next-line no-debugger
debugger;
debugger;
/* oxlint-disable */
debugger;
/* oxlint-enable no-debugger */
debugger;
/* oxlint-enable */
debugger;
/* oxlint-disable no-debugger */
debugger;
/* oxlint-enable */
debugger;
/* oxlint-enable no-debugger */
debugger;
// eslint-disable-next-line no-debugger
debugger;
// oxlint-disable-next-line foo/no-debugger
debugger;
// oxlint-disable-next-line no-debugger--reason
debugger;
// oxlint-disable-next-line no-debuggerfoo
debugger;
// oxlint-disable-next-line debugger
debugger;
debugger; // oxlint-disable-line no-console -no-debugger
";
        // 0-based statements: after a `-line` comment on its line (1), after a multi-line
        // `-line` comment's first line (3), the line after a `-next-line`'s next line (7), after
        // `enable` closes `disable` (10, past an `enable no-debugger` that matched nothing, 9),
        // after `enable no-debugger` closes `disable no-debugger` (13, past an `enable` that
        // matched nothing, 12), and names that are not the rule's (17, 18, 19).
        assert_eq!(
            reported(source, "no-debugger"),
            vec![1, 3, 7, 10, 13, 17, 18, 19]
        );
    }

    /// `next_line_span_of_line_comment`: the interval runs from the comment's end over the rest of
    /// its line and the next line, without its line break.
    #[test]
    fn next_line_interval_ends_before_the_next_line_break() {
        for (source, start, stop) in [
            ("// eslint-disable-next-line max-params", 38, 38),
            ("// eslint-disable-next-line max-params\n", 38, 39),
            ("// eslint-disable-next-line max-params\r\n", 38, 40),
            ("// eslint-disable-next-line max-params    \n", 42, 43),
            ("// eslint-disable-next-line max-params    \n ABC", 42, 47),
            (
                "// eslint-disable-next-line max-params    \r\n ABC \r\n",
                42,
                49,
            ),
        ] {
            let directives = read(source);
            assert_eq!(
                directives.intervals,
                [Interval {
                    start,
                    stop,
                    rule: Some("max-params".to_owned()),
                    line: true
                }],
                "{source:?}"
            );
        }
    }

    /// `directive_rule_lists_parse_rules_and_descriptions`.
    #[test]
    fn rule_lists_split_on_commas_and_white_space_up_to_a_description() {
        for (list, expected) in [
            (" no-debugger", vec!["no-debugger"]),
            (
                " no-debugger,no-unused-vars",
                vec!["no-debugger", "no-unused-vars"],
            ),
            (
                " no-debugger no-unused-vars",
                vec!["no-debugger", "no-unused-vars"],
            ),
            (
                " no-debugger  no-unused-vars -- description",
                vec!["no-debugger", "no-unused-vars"],
            ),
            (" no-debugger-- description", vec!["no-debugger"]),
            (
                " typescript/unbound-method - bound later",
                vec!["typescript/unbound-method"],
            ),
            (
                " @scope/plugin/rule-name no-debugger -- description",
                vec!["@scope/plugin/rule-name", "no-debugger"],
            ),
            (
                " no-console -no-debugger",
                vec!["no-console", "-no-debugger"],
            ),
        ] {
            assert_eq!(collect_rule_names(list), expected, "{list:?}");
        }
    }

    /// A listed name matches the rule's own name, whatever plugin prefixes it, and only all of it
    /// (`directive_rule_name_is_matched_on_full_rule_name_not_substring`).
    #[test]
    fn plugin_prefixed_names_match_the_rule_name_after_the_last_slash() {
        let source = "\
// oxlint-disable-next-line react-hooks/rules-of-hooks
useA();
// oxlint-disable-next-line foo/rules-of-hooks
useB();
// oxlint-disable-next-line @scope/plugin/rules-of-hooks
useC();
// eslint-disable-next-line canonical/no-re-export
export * from './foo';
// eslint-disable-next-line import/export
export * from './bar';
";
        let directives = read(source);
        for hook in ["useA()", "useB()", "useC()"] {
            assert!(
                directives.suppresses(&names(&["rules-of-hooks"]), nth(source, hook, 0)),
                "{hook}"
            );
        }
        assert!(!directives.suppresses(&names(&["export"]), nth(source, "export *", 0)));
        assert!(directives.suppresses(&names(&["export"]), nth(source, "export *", 1)));
        // Any of a violation's names may be the one listed.
        assert!(directives.suppresses(&names(&["other", "export"]), nth(source, "export *", 1)));
    }

    /// A block interval covers a problem that overlaps it, even one starting before the `disable`
    /// comment; a `-next-line` one only a problem that starts in it
    /// (`test_disable_next_line_should_not_disable_large_span_diagnostics`). Checked with oxlint
    /// 1.88.0's `max-lines-per-function`.
    #[test]
    fn blocks_cover_overlapping_problems_and_lines_cover_starting_ones() {
        let source = "\
function a() {
  /* oxlint-disable max-lines-per-function */
  /* oxlint-enable max-lines-per-function */
  return 1;
}
function b() {
  // oxlint-disable-next-line max-lines-per-function
  return 1;
}
";
        let directives = read(source);
        let rule = names(&["max-lines-per-function"]);
        let function = |name: &str| {
            let start = source.find(&format!("function {name}")).expect("function");
            start..start + source[start..].find('}').expect("end") + 1
        };
        assert!(directives.suppresses(&rule, function("a")));
        assert!(!directives.suppresses(&rule, function("b")));
        assert!(directives.suppresses(&rule, nth(source, "return 1;", 1)));
        assert!(!directives.suppresses(&rule, nth(source, "return 1;", 0)));
    }

    /// JSX comments are comments, and an empty file holds no directive.
    #[test]
    fn jsx_comments_are_read() {
        let source = "const a = <div>\n  {/* oxlint-disable-next-line rules-of-hooks */}\n  {useA()}\n</div>;\n";
        let directives = read(source);
        assert!(directives.suppresses(&names(&["rules-of-hooks"]), nth(source, "useA()", 0)));
        assert!(read("").intervals.is_empty());
    }

    /// HTML-like comments are comments: cases checked against `oxlint` 1.88.0 itself
    /// (`-A all -D no-debugger`, as `.cjs` files, where oxc lexes them), in every ECMAScript
    /// dialect. Each keeps only its last `debugger` reported.
    #[test]
    fn html_like_comments_are_read() {
        for source in [
            "<!-- oxlint-disable-next-line no-debugger\ndebugger;\ndebugger;\n",
            "--> oxlint-disable-next-line no-debugger\ndebugger;\ndebugger;\n",
            "  --> oxlint-disable-next-line no-debugger\ndebugger;\ndebugger;\n",
            "/*\n*/ --> oxlint-disable-next-line no-debugger\ndebugger;\ndebugger;\n",
            "<!-- oxlint-disable-next-line no-debugger -->\ndebugger;\ndebugger;\n",
            "debugger; <!-- oxlint-disable-line no-debugger\ndebugger;\n",
            "debugger; <!-- oxlint-disable-line\ndebugger;\n",
            "debugger; /* a */ <!-- eslint-disable-line no-debugger\ndebugger;\n",
            "<!-- oxlint-disable no-debugger\ndebugger;\n--> oxlint-enable no-debugger\ndebugger;\n",
        ] {
            for dialect in [
                GrammarDialect::TypeScript,
                GrammarDialect::Tsx,
                GrammarDialect::JavaScript,
            ] {
                let root = dialect.ast_grep(source);
                let directives = OxlintDirectives::read(&root.root(), source);
                let count = source.matches("debugger;").count();
                let kept = (0..count)
                    .filter(|&index| {
                        !directives
                            .suppresses(&names(&["no-debugger"]), nth(source, "debugger;", index))
                    })
                    .collect::<Vec<_>>();
                assert_eq!(kept, [count - 1], "{dialect:?} {source:?}");
            }
        }
    }

    /// The 0-based lines a rule reporting every `useEffect` call, with `suppress`, keeps on
    /// `source` through the engine.
    fn hooks_reported(source: &str, suppress: serde_json::Value) -> Vec<u64> {
        use crate::code_rule_dsl::test_support::*;
        use serde_json::{Value, json};
        let violation = json!({ "anchor": "witness.span", "code": "HOOK", "message": "hook",
            "suppress": suppress });
        let rule = json!({ "kind": "call_expression",
            "has": { "field": "function", "kind": "identifier", "regex": "^useEffect$" } });
        let document = clause_document(json!({ "id": "hooks", "violation": violation,
            "forbid": { "ast": { "language": "typescript", "rule": rule } } }));
        let compiled = compile_document_with(
            document,
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, "tsx", "avatar.tsx", source);
        assert_eq!(result["failures"], json!([]), "{result}");
        result["violations"]
            .as_array()
            .expect("violations")
            .iter()
            .map(|violation| violation["span"]["startLine"].as_u64().expect("line"))
            .collect()
    }

    /// Through the engine: `suppress.oxlint` keeps the hooks `oxlint` 1.88.0's
    /// `react/rules-of-hooks` keeps on the same file, Dot and Plain: `biome-ignore` is no
    /// directive of either linter, and oxlint reads the `eslint-disable` block too.
    /// `suppress.eslint` alone reads only that block.
    #[test]
    fn oxlint_directives_suppress_violations_through_the_engine() {
        use serde_json::json;
        let source = "\
import { useEffect } from \"react\";
// oxlint-disable react-hooks/rules-of-hooks
function Avatar({ x }) { if (x) useEffect(() => {}); return null; }
// oxlint-enable react-hooks/rules-of-hooks
function Badge({ x }) { if (x) useEffect(() => {}); return null; } // oxlint-disable-line react-hooks/rules-of-hooks
// oxlint-disable-next-line react-hooks/rules-of-hooks
function Chip({ x }) { if (x) useEffect(() => {}); return null; }
// biome-ignore lint/correctness/useHookAtTopLevel: intentional
function Dot({ x }) { if (x) useEffect(() => {}); return null; }
/* eslint-disable react-hooks/rules-of-hooks */
function Eslinted({ x }) { if (x) useEffect(() => {}); return null; }
/* eslint-enable react-hooks/rules-of-hooks */
function Plain({ x }) { if (x) useEffect(() => {}); return null; }
";
        let eslint = json!({ "rules": ["react-hooks/rules-of-hooks"] });
        let oxlint = json!({ "rules": ["rules-of-hooks"] });
        assert_eq!(
            hooks_reported(source, json!({ "eslint": eslint, "oxlint": oxlint })),
            [8, 12]
        );
        assert_eq!(hooks_reported(source, json!({ "oxlint": oxlint })), [8, 12]);
        assert_eq!(
            hooks_reported(source, json!({ "eslint": eslint })),
            [2, 4, 6, 8, 12]
        );
    }
