    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use ast_grep_language::LanguageExt;

    fn read(source: &str) -> BiomeSuppressions {
        let root = GrammarDialect::Tsx.ast_grep(source);
        BiomeSuppressions::read(&root.root(), source)
    }

    fn rules(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    /// The starts of `needle`'s occurrences a problem of `rule` spanning the occurrence keeps
    /// reported.
    fn reported(source: &str, needle: &str, rule: &str) -> Vec<usize> {
        let suppressions = read(source);
        source
            .match_indices(needle)
            .map(|(at, _)| at)
            .filter(|&at| !suppressions.suppresses(&rules(&[rule]), at..at + needle.len()))
            .collect()
    }

    /// The 0-based lines of the `useEffect(` calls `rule` keeps reported.
    fn hooks(source: &str, rule: &str) -> Vec<usize> {
        let suppressions = read(source);
        source
            .match_indices("useEffect(")
            .filter(|(at, _)| !suppressions.suppresses(&rules(&[rule]), *at..at + 12))
            .map(|(at, _)| source[..at].matches('\n').count())
            .collect()
    }

    const HOOK: &str = "lint/correctness/useHookAtTopLevel";

    /// Biome's `suppression` test (`crates/biome_js_analyze/src/suppressions.tests.rs`
    /// l.146-255): the `==` operators `noDoubleEquals` still reports, at Biome's own offsets.
    /// `lint(suspicious/noDoubleEquals)` is the category `lint` with a value: every lint rule.
    #[test]
    fn biome_suppression_test() {
        const SOURCE: &str = "
            function checkSuppressions1(a, b) {
                a == b;
                // biome-ignore lint/suspicious:whole group
                p == f;
                // biome-ignore lint/suspicious/noDoubleEquals: single rule
                j == k;
                /* biome-ignore lint/complexity/useWhile: multiple block comments */ /* biome-ignore lint/suspicious/noDoubleEquals: multiple block comments */
                o == m;
                // biome-ignore lint/complexity/useWhile: multiple line comments
                // biome-ignore lint/suspicious/noDoubleEquals: multiple line comments
                d == x;
                z == v;
            }

            // biome-ignore lint/suspicious/noDoubleEquals: do not suppress warning for the whole function
            function checkSuppressions2(a, b) {
                a == b;
            }

            function checkSuppressions3(a, b) {
                a == b;
                // biome-ignore lint/suspicious: whole group
                a == b;
                // biome-ignore lint/suspicious/noDoubleEquals: single rule
                a == b;
                /* biome-ignore lint/complexity/useWhile: multiple block comments */ /* biome-ignore lint(suspicious/noDoubleEquals): multiple block comments */
                a == b;
                // biome-ignore lint/complexity/useWhile: multiple line comments
                // biome-ignore lint/suspicious/noDoubleEquals: multiple line comments
                a == b;
                a == b;
            }

            // biome-ignore lint(suspicious/noDoubleEquals): do not suppress warning for the whole function
            function checkSuppressions4(a, b) {
                a == b;
            }

            function checkSuppressions5() {
                // biome-ignore format explanation
                // biome-ignore format(:
                // biome-ignore (value): explanation
                // biome-ignore unknown: explanation
            }
        ";
        assert_eq!(
            reported(SOURCE, "==", "lint/suspicious/noDoubleEquals"),
            [67, 651, 845, 932, 1518, 1713]
        );
    }

    /// Biome's `suppression_issue_5562` and
    /// `suppression_range_should_report_when_contains_inner_comment` (l.828-865, l.953-990): a
    /// `biome-ignore` reaches the line after it only, and one inside the function it precedes
    /// leaves it covered.
    #[test]
    fn biome_ignore_covers_the_next_line() {
        let source = "console.log(\"should error\");\n\n// biome-ignore lint/suspicious/noConsole: foo\nconsole.log(\"should be suppressed\");";
        assert_eq!(
            reported(source, "console.log", "lint/suspicious/noConsole"),
            [0]
        );
        let source = "
// biome-ignore lint/complexity/useArrowFunction: single rule
const foo0 = function (bar: string) {
  // biome-ignore lint/style/noParameterAssign: single rule
  bar = 'baz';
};";
        assert!(reported(source, "function (bar: string) {\n  // biome-ignore lint/style/noParameterAssign: single rule\n  bar = 'baz';\n}", "lint/complexity/useArrowFunction").is_empty());
        assert!(reported(source, "bar = 'baz'", "lint/style/noParameterAssign").is_empty());
    }

    /// What a `biome-ignore` covers: from the comment to the end of the next line, so code after
    /// it on its own line too; not the line after a blank line or a plain comment; consecutive
    /// suppression comments (a trailing one included) move on to the line after the last.
    #[test]
    fn biome_ignore_ranges() {
        let source = "\
// biome-ignore lint: x
useEffect(a); useEffect(b);
useEffect(c);
// biome-ignore lint: x

useEffect(d);
// biome-ignore lint: x
// a note
useEffect(e);
/* biome-ignore lint: x */ useEffect(f);
useEffect(g);
useEffect(h);
useEffect(i); // biome-ignore lint: x
useEffect(j);
// biome-ignore lint/correctness: x
useEffect(k); // biome-ignore lint/suspicious: x
useEffect(l);
useEffect(m);
";
        assert_eq!(hooks(source, HOOK), [2, 5, 8, 11, 12, 17]);
    }

    /// A block comment over several lines covers the line after its last; one on a line of its
    /// own covers the next; JSX comments are comments (as `biome lint` 2.5.15 reports both).
    #[test]
    fn block_and_jsx_comments() {
        let source = "\
/**
 * A note.
 * biome-ignore lint/correctness/useHookAtTopLevel: x
 */
useEffect(a);
useEffect(b);
const c = (
  <div>
    {/* biome-ignore lint/correctness/useHookAtTopLevel: x */}
    {useEffect(c)}
    {useEffect(d)}
  </div>
);
";
        assert_eq!(hooks(source, HOOK), [5, 10]);
    }

    /// `lint`, the rule's group and the rule itself suppress it; another group or rule, an
    /// instance (Rerail's violations report none) or a non-lint category does not.
    #[test]
    fn category_granularity() {
        for (category, suppressed) in [
            ("lint", true),
            ("lint/correctness", true),
            ("lint/correctness/useHookAtTopLevel", true),
            (
                "lint/correctness/noUnusedVariables lint/correctness/useHookAtTopLevel",
                true,
            ),
            ("lint/suspicious", false),
            ("lint/correctness/noUnusedVariables", false),
            ("lint/correctness/useHookAtTopLevel(useEffect)", false),
            ("format", false),
            ("assist", false),
            ("lint/plugin", false),
            ("lint/plugin/useEffect", false),
        ] {
            let source = format!("// biome-ignore {category}: x\nuseEffect(a);\n");
            assert_eq!(hooks(&source, HOOK).is_empty(), suppressed, "{category}");
        }
    }

    /// A comment Biome refuses suppresses nothing: an unknown category (with a known one beside
    /// it too), no reason, no colon, an unclosed `(`, a value without category, `biome-ignore`
    /// not spelled in lowercase anywhere in the comment. A category the JavaScript registry lacks
    /// (a CSS rule, a group) is refused alone.
    #[test]
    fn refused_comments() {
        for comment in [
            "// biome-ignore lint/correctness/useHookAtTopLevell: x",
            "// biome-ignore lint/correctness/useHookAtTopLevel linx: x",
            "// biome-ignore lint/correctness/useHookAtTopLevel:",
            "// biome-ignore lint/correctness/useHookAtTopLevel:   ",
            "// biome-ignore lint/correctness/useHookAtTopLevel",
            "// biome-ignore lint/correctness/useHookAtTopLevel x",
            "// biome-ignore lint/correctness/useHookAtTopLevel(x: y",
            "// biome-ignore (value): x",
            "// biome-ignore lint/correctness/ useHookAtTopLevel: x",
            "// BIOME-IGNORE lint: x",
            "// Biome_Ignore lint: x",
            "// biome-ignore-all-start lint: x",
            "// biome-ignore: x",
            "// biome ignore lint: x",
            "// note: biome-ignore lint: x",
        ] {
            let source = format!("{comment}\nuseEffect(a);\n");
            assert_eq!(hooks(&source, HOOK), [1], "{comment}");
        }
        for comment in [
            "// biome-ignore lint/a11y/useGenericFontNames lint/correctness/useHookAtTopLevel: x",
            "// biome-ignore lint/correctness/useHookAtTopLevel: x biome-ignore",
            "/** biome-ignore lint/correctness/useHookAtTopLevel: x */",
            "//biome-ignore lint/correctness/useHookAtTopLevel:x",
            "// biome-ignorelint: x",
            "// biome-ignore\tlint/correctness/useHookAtTopLevel\t: x",
            "// biome-ignore format lint: x",
            "// biome-ignore lint/correctness/useHookAtTopLevel(x)lint: y",
        ] {
            let source = format!("{comment}\nuseEffect(a);\n");
            assert!(hooks(&source, HOOK).is_empty(), "{comment}");
        }
    }

    /// Biome's top-level tests (l.295-673): `biome-ignore-all` before the first token, after a
    /// shebang or other comments, covers the whole file; several in one comment each count; after
    /// code it covers nothing. An instance counts as its rule there.
    #[test]
    fn biome_ignore_all() {
        for prefix in [
            "",
            "#!/usr/bin/env bun\n",
            "/*\n* Top level comment here. It could be a banner or a license comment\n* MIT\n*/\n",
            "#!/usr/bin/env bun\n\n/* Arbitrary comment here\n */\n",
        ] {
            let source = format!(
                "{prefix}/**\n* biome-ignore-all lint/style/useConst: reason\n*/\n\nuseEffect(a);\nlet foo = 2;\n"
            );
            assert!(
                reported(&source, "let foo", "lint/style/useConst").is_empty(),
                "{prefix}"
            );
            assert_eq!(hooks(&source, HOOK).len(), 1, "{prefix}");
        }
        let source = "/**\n* biome-ignore-all lint/style/useConst: reason\n* biome-ignore-all lint/correctness/useHookAtTopLevel(useEffect): reason2\n*/\n\nuseEffect(a);\nlet foo = 2;\n";
        assert!(hooks(source, HOOK).is_empty());
        assert!(reported(source, "let foo", "lint/style/useConst").is_empty());
        let source = "// biome-ignore-all lint: reason\nuseEffect(a);\nuseEffect(b);\n";
        assert!(hooks(source, HOOK).is_empty());
        // A leading byte order mark is a token to Biome: the suppression after it is not at the top.
        let source = "\u{feff}// biome-ignore-all lint: reason\nuseEffect(a);\nuseEffect(b);\n";
        assert_eq!(hooks(source, HOOK), [1, 2]);
        let source = "useEffect(a);\n// biome-ignore-all lint: reason\nuseEffect(b);\n";
        assert_eq!(hooks(source, HOOK), [0, 2]);
        let source = "\"use strict\";\n// biome-ignore-all lint: reason\nuseEffect(b);\n";
        assert_eq!(hooks(source, HOOK), [2]);
    }

    /// Biome's range tests (l.675-826, l.867-912): `biome-ignore-start` covers what it holds
    /// whole until the `-end` of the same category; an `-end` without its start closes nothing,
    /// an unclosed start runs to the end of the file, and the latest open start is the one closed.
    #[test]
    fn biome_ignore_ranges_start_end() {
        let source = "
// biome-ignore-start lint/suspicious/noDoubleEquals: single rule
// biome-ignore-start lint/suspicious/noDebugger: single rule
a == b;
debugger;
// biome-ignore-end lint/suspicious/noDoubleEquals: single rule
a === b;
debugger;
// biome-ignore-end lint/suspicious/noDebugger: single rule
debugger;

        ";
        assert_eq!(
            reported(source, "debugger", "lint/suspicious/noDebugger"),
            [source.rfind("debugger").unwrap()]
        );
        assert!(reported(source, "a == b", "lint/suspicious/noDoubleEquals").is_empty());
        let source = "
// biome-ignore-start lint/suspicious/noDoubleEquals: single rule
// biome-ignore-start lint/style/useConst: single rule
a == b;
let c;
// biome-ignore-end lint/suspicious/noDoubleEquals: single rule
a == b;
let c;
// biome-ignore-end lint/style/useConst: single rule
        ";
        assert_eq!(
            reported(source, "a == b", "lint/suspicious/noDoubleEquals"),
            [source.rfind("a == b").unwrap()]
        );
        assert!(reported(source, "let c", "lint/style/useConst").is_empty());
        for (source, kept) in [
            // A group start is closed by a group end only.
            (
                "// biome-ignore-start lint/correctness: x\nuseEffect(a);\n// biome-ignore-end lint/correctness/useHookAtTopLevel: x\nuseEffect(b);\n",
                vec![],
            ),
            (
                "// biome-ignore-start lint/correctness: x\nuseEffect(a);\n// biome-ignore-end lint/correctness: x\nuseEffect(b);\n",
                vec![3],
            ),
            (
                "// biome-ignore-start lint: x\nuseEffect(a);\n// biome-ignore-end lint: x\nuseEffect(b);\n",
                vec![3],
            ),
            ("// biome-ignore-end lint: x\nuseEffect(a);\n", vec![1]),
            (
                "useEffect(a);\n// biome-ignore-start lint/correctness/useHookAtTopLevel: x\nuseEffect(b);\n",
                vec![0],
            ),
        ] {
            assert_eq!(hooks(source, HOOK), kept, "{source}");
        }
        // A problem the range holds only in part is not covered.
        let source =
            "useEffect(a,\n// biome-ignore-start lint: x\nb);\n// biome-ignore-end lint: x\n";
        let suppressions = read(source);
        let inside = source.find("b)").expect("b");
        assert!(!suppressions.suppresses(&rules(&[HOOK]), 0..inside + 2));
        assert!(suppressions.suppresses(&rules(&[HOOK]), inside..inside + 2));
    }

    /// Cases checked against `biome lint` 2.5.15 itself (`noDebugger` alone on): the debugger
    /// statements it still reports, as 0-based lines.
    #[test]
    fn cases_match_biome() {
        let debuggers = |source: &str| -> Vec<usize> {
            let suppressions = read(source);
            source
                .match_indices("debugger;")
                .filter(|(at, _)| {
                    !suppressions.suppresses(&rules(&["lint/suspicious/noDebugger"]), *at..at + 9)
                })
                .map(|(at, _)| source[..at].matches('\n').count())
                .collect()
        };
        let source = "\
// biome-ignore lint: x
debugger; debugger;
debugger;
// biome-ignore lint: x

debugger;
// biome-ignore lint: x
// a note
debugger;
/* biome-ignore lint: x */ debugger;
debugger;
debugger;
debugger; // biome-ignore lint: x
debugger;
// biome-ignore lint/suspicious: x
debugger; // biome-ignore lint/style: x
debugger;
debugger;
";
        assert_eq!(debuggers(source), [2, 5, 8, 11, 12, 17]);
        for (comment, reported) in [
            ("// biome-ignore lint/suspicious/noDebuggerr: x", true),
            ("// biome-ignore lint/suspicious/noDebugger linx: x", true),
            ("// biome-ignore lint/suspicious/noDebugger:", true),
            ("// biome-ignore lint/suspicious/noDebugger", true),
            ("// biome-ignore lint/suspicious/noDebugger x", true),
            ("// biome-ignore lint/suspicious/noDebugger(x: y", true),
            ("// biome-ignore (value): x", true),
            ("// BIOME-IGNORE lint: x", true),
            ("// Biome_Ignore lint: x", true),
            ("// biome-ignore-all-start lint: x", true),
            ("// biome-ignore: x", true),
            ("// note: biome-ignore lint: x", true),
            ("// biome-ignore lint/suspicious/noDebugger(x): y", true),
            ("// biome-ignore lint/plugin: x", true),
            ("// biome-ignore assist: x", true),
            (
                "// biome-ignore lint/a11y/useGenericFontNames lint/suspicious/noDebugger: x",
                false,
            ),
            ("/** biome-ignore lint/suspicious/noDebugger: x */", false),
            ("//biome-ignore lint/suspicious/noDebugger:x", false),
            ("// biome-ignorelint: x", false),
            ("// biome-ignore\tlint/suspicious/noDebugger\t: x", false),
            ("// biome-ignore format lint: x", false),
            (
                "// biome-ignore lint/suspicious/noDebugger(x)lint: y",
                false,
            ),
            ("// biome-ignore lint/correctness lint/suspicious: x", false),
        ] {
            let source = format!("x();\n{comment}\ndebugger;\n");
            let expected: &[usize] = if reported { &[2] } else { &[] };
            assert_eq!(debuggers(&source), expected, "{comment}");
        }
        for (source, reported) in [
            (
                "#!/usr/bin/env node\n\n/* Arbitrary comment here\n */\n/**\n* biome-ignore-all lint/suspicious/noDebugger(x): reason\n*/\ndebugger;\n",
                vec![],
            ),
            (
                "\"use strict\";\n// biome-ignore-all lint: reason\ndebugger;\n",
                vec![2],
            ),
            (
                "// biome-ignore-start lint/suspicious: x\ndebugger;\n// biome-ignore-end lint/suspicious/noDebugger: x\ndebugger;\n",
                vec![],
            ),
            (
                "// biome-ignore-end lint: x\ndebugger;\n// biome-ignore-start lint/suspicious/noDebugger: x\ndebugger;\n// biome-ignore-start lint: x\ndebugger;\n// biome-ignore-end lint: x\ndebugger;\n",
                vec![1],
            ),
            // Biome's lexer has no HTML-like comments: `<!--` and `-->` are tokens (a parse
            // error), and both statements stay reported.
            (
                "<!-- biome-ignore lint/suspicious/noDebugger: x\ndebugger;\ndebugger;\n",
                vec![1, 2],
            ),
            (
                "--> biome-ignore lint/suspicious/noDebugger: x\ndebugger;\ndebugger;\n",
                vec![1, 2],
            ),
        ] {
            assert_eq!(debuggers(source), reported, "{source}");
        }
    }

    /// Only lint rules of the JavaScript analyzer are rule names `suppress.biome` takes.
    #[test]
    fn lint_rules() {
        assert!(is_lint_rule("lint/correctness/useHookAtTopLevel"));
        assert!(is_lint_rule("lint/nursery/noReactStringRefs"));
        assert!(!is_lint_rule("lint/correctness/useHookAtTopLevell"));
        assert!(!is_lint_rule("lint/correctness"));
        assert!(!is_lint_rule("lint/a11y/useGenericFontNames"));
        assert!(!is_lint_rule("assist/source/organizeImports"));
        assert!(!is_lint_rule("source/organizeImports"));
        assert!(!is_lint_rule("syntax/correctness/noSuperWithoutExtends"));
        // Sorted and without repeats, as the lookups need.
        assert!(
            registry::CATEGORIES
                .windows(2)
                .all(|pair| pair[0] < pair[1])
        );
        assert!(
            registry::JS_RULES
                .windows(2)
                .all(|pair| (pair[0].0, pair[0].1) < (pair[1].0, pair[1].1))
        );
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

    /// Through the engine: `suppress.biome` drops Dot, under its `biome-ignore`, and keeps Eh,
    /// whose comment has no reason: `biome lint` 2.5.15 reports lines 2, 6 and 7 of this file.
    /// With `oxlint` beside it, Chip goes too.
    #[test]
    fn biome_ignore_suppresses_violations_through_the_engine() {
        use serde_json::json;
        let source = "\
import { useEffect } from \"react\";
// oxlint-disable-next-line react-hooks/rules-of-hooks
function Chip({ x }) { if (x) useEffect(() => {}); return null; }
// biome-ignore lint/correctness/useHookAtTopLevel: intentional
function Dot({ x }) { if (x) useEffect(() => {}); return null; }
// biome-ignore lint/correctness/useHookAtTopLevel
function Eh({ x }) { if (x) useEffect(() => {}); return null; }
function Plain({ x }) { if (x) useEffect(() => {}); return null; }
";
        let biome = json!({ "rules": [HOOK] });
        let oxlint = json!({ "rules": ["rules-of-hooks"] });
        assert_eq!(hooks_reported(source, json!({ "biome": biome })), [2, 6, 7]);
        assert_eq!(
            hooks_reported(source, json!({ "oxlint": oxlint, "biome": biome })),
            [6, 7]
        );
    }
