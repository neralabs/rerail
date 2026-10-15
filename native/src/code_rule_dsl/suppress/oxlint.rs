//! Oxlint 1.88.0's disable directives (`crates/oxc_linter/src/disable_directives.rs`'s
//! `DisableDirectivesBuilder` and `DisableDirectives::contains`, which
//! `crates/oxc_linter/src/context/mod.rs`'s `add_diagnostic` asks with the reporting rule's own
//! name, without its plugin), at oxc tag `oxlint_v1.88.0`
//! (`6941b4ed45ead21c45bc2164cb80a180eed85b43`), with `respectEslintDisableDirectives` at its
//! default, `true`.
//!
//! The comments are oxc's (`Semantic::comments`), HTML-like ones (Annex B.1.1) included:
//! `crates/oxc_parser/src/lexer/punctuation.rs`'s `read_left_angle` and `read_minus` leave a
//! `<!--` anywhere, and a `-->` first on its line, to `skip_single_line_comment` as
//! `CommentKind::HtmlOpen` and `HtmlClose` (`byte_handlers.rs`), whose `content_span`
//! (`crates/oxc_ast/src/ast/comment.rs`) starts after the 4- or 3-byte opener, in a script: a
//! `.cjs` or `.cts` file, or a `.js`, `.jsx`, `.ts` or `.tsx` one without module syntax
//! (`ModuleKind::Unambiguous`). Elsewhere (`.mjs`, `.mts`, module syntax) oxc does not parse
//! the file and oxlint reports nothing, so every `html_comment` tree-sitter-javascript's and
//! tree-sitter-typescript's scanners yield is read as one.
//!
//! Ported from oxc (MIT, VoidZero Inc. & Contributors, and Boshen; see THIRD_PARTY_NOTICES)
//! without its unused-directive bookkeeping and fixes.

use ast_grep_core::{Doc, Node};
use std::collections::HashMap;
use std::ops::Range;

/// Where a directive leaves a rule (or, `rule` none, every rule) off: `DisabledRule` over its
/// interval.
#[derive(Debug, PartialEq, Eq)]
struct Interval {
    start: usize,
    stop: usize,
    /// The name the directive lists, plugin prefix included; `None` disables every rule.
    rule: Option<String>,
    /// A `-line` or `-next-line` directive (`is_next_line`): it covers a problem that starts in
    /// the interval, where a block covers one that overlaps it.
    line: bool,
}

/// A directive's kind (`DirectiveKind`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Disable,
    DisableNextLine,
    DisableLine,
    Enable,
}

/// A file's oxlint (and ESLint-spelled) disable directives, as the intervals oxlint builds from
/// them.
#[derive(Debug, Default)]
pub(crate) struct OxlintDirectives {
    intervals: Vec<Interval>,
}

impl OxlintDirectives {
    /// Reads every comment of the file `root` is the root of, HTML-like ones included.
    pub(crate) fn read<D: Doc>(root: &Node<'_, D>, source: &str) -> Self {
        Self::build(
            source,
            root.dfs()
                .filter(|node| matches!(node.kind().as_ref(), "comment" | "html_comment"))
                .map(|comment| comment.range()),
        )
    }

    /// `build_impl` over comments given by their outer ranges (`//`, `/* */`, `<!--` or `-->`
    /// included), in source order. Only pairs build a block interval: `disable` with `enable`,
    /// `disable rule` with `enable rule`. An `enable rule` inside a `disable` (or an `enable`
    /// inside a `disable rule`) matches nothing, and the `disable` runs on to the end of the file.
    fn build(source: &str, comments: impl IntoIterator<Item = Range<usize>>) -> Self {
        let mut intervals = Vec::new();
        // The first unmatched `disable`, and each rule's first unmatched `disable rule`: where
        // their comment's content ends.
        let mut disable_all_start: Option<usize> = None;
        let mut disable_start_map: HashMap<String, usize> = HashMap::new();
        for range in comments {
            let Some(content) = content_span(source, range) else {
                continue;
            };
            let text = source[content.clone()].trim_start();
            let Some((kind, rule_list)) = match_directive(text) else {
                continue;
            };
            let rules = collect_rule_names(rule_list);
            // A `-line` or `-next-line` directive's interval.
            let line = match kind {
                Kind::Disable => {
                    if rules.is_empty() {
                        disable_all_start.get_or_insert(content.end);
                    }
                    for rule in &rules {
                        disable_start_map
                            .entry((*rule).to_owned())
                            .or_insert(content.end);
                    }
                    None
                }
                Kind::DisableNextLine => {
                    // The rest of the comment's line, and the next line up to its line break.
                    let mut stop = content.end;
                    let mut after = source[content.end..].split_inclusive('\n');
                    if let Some(rest_of_line) = after.next() {
                        stop += rest_of_line.len();
                    }
                    if let Some(next_line) = after.next() {
                        stop += next_line.trim_end_matches(['\n', '\r']).len();
                    }
                    Some(content.end..stop)
                }
                Kind::DisableLine => {
                    // From the start of the line the comment starts on to the comment: a block
                    // comment over several lines covers its first line.
                    let start = source[..content.start]
                        .lines()
                        .next_back()
                        .map_or(0, |line| content.start - line.len());
                    Some(start..content.start)
                }
                Kind::Enable => {
                    if rules.is_empty()
                        && let Some(start) = disable_all_start.take()
                    {
                        intervals.push(Interval {
                            start,
                            stop: content.start,
                            rule: None,
                            line: false,
                        });
                    }
                    for rule in &rules {
                        if let Some(start) = disable_start_map.remove(*rule) {
                            intervals.push(Interval {
                                start,
                                stop: content.start,
                                rule: Some((*rule).to_owned()),
                                line: false,
                            });
                        }
                    }
                    None
                }
            };
            if let Some(line) = line {
                let rules = if rules.is_empty() {
                    vec![None]
                } else {
                    rules.iter().map(|rule| Some((*rule).to_owned())).collect()
                };
                intervals.extend(rules.into_iter().map(|rule| Interval {
                    start: line.start,
                    stop: line.end,
                    rule,
                    line: true,
                }));
            }
        }
        // A lone `disable` (or `disable rule`) runs to the end of the file.
        if let Some(start) = disable_all_start {
            intervals.push(Interval {
                start,
                stop: source.len(),
                rule: None,
                line: false,
            });
        }
        for (rule, start) in disable_start_map {
            intervals.push(Interval {
                start,
                stop: source.len(),
                rule: Some(rule),
                line: false,
            });
        }
        Self { intervals }
    }

    /// Whether a problem of a rule named any of `rules` (oxlint's names, without a plugin),
    /// spanning bytes `span`, is suppressed (`contains`): an interval that overlaps the span
    /// (`Lapper::find`) disables every rule or a listed name whose part after its last `/` is the
    /// rule's name; a `-line` or `-next-line` interval must also hold the span's start.
    pub(crate) fn suppresses(&self, rules: &[String], span: Range<usize>) -> bool {
        self.intervals.iter().any(|interval| {
            let found = interval.start < span.end && interval.stop > span.start;
            let names = interval.rule.as_deref().is_none_or(|name| {
                let bare = name.rsplit_once('/').map_or(name, |(_, rule)| rule);
                rules.iter().any(|rule| rule == bare)
            });
            let covered = if interval.line {
                span.start >= interval.start && span.start < interval.stop
            } else {
                span.start < interval.stop && span.end > interval.start
            };
            found && names && covered
        })
    }
}

/// A comment's `content_span`: inside its `//` or `/* */`, or after its `<!--` or `-->`.
fn content_span(source: &str, range: Range<usize>) -> Option<Range<usize>> {
    let text = &source[range.clone()];
    if text.starts_with("//") {
        Some(range.start + 2..range.end)
    } else if text.starts_with("/*") && text.ends_with("*/") && text.len() >= 4 {
        Some(range.start + 2..range.end - 2)
    } else if text.starts_with("<!--") {
        Some(range.start + 4..range.end)
    } else if text.starts_with("-->") {
        Some(range.start + 3..range.end)
    } else {
        None
    }
}

/// `match_directive`: an `eslint-` or `oxlint-` `disable`, `disable-next-line`, `disable-line` or
/// `enable` at the start of the (start-trimmed) comment text, ending the text or followed by
/// white space, and the text after it.
fn match_directive(text: &str) -> Option<(Kind, &str)> {
    fn is_directive_end(text: &str) -> bool {
        text.is_empty() || text.starts_with(char::is_whitespace)
    }
    let rest = text
        .strip_prefix("eslint-")
        .or_else(|| text.strip_prefix("oxlint-"))?;
    if let Some(rest) = rest.strip_prefix("disable") {
        if let Some(rest) = rest.strip_prefix("-next-line")
            && is_directive_end(rest)
        {
            return Some((Kind::DisableNextLine, rest));
        } else if let Some(rest) = rest.strip_prefix("-line")
            && is_directive_end(rest)
        {
            return Some((Kind::DisableLine, rest));
        } else if is_directive_end(rest) {
            return Some((Kind::Disable, rest));
        }
    } else if let Some(rest) = rest.strip_prefix("enable")
        && is_directive_end(rest)
    {
        return Some((Kind::Enable, rest));
    }
    None
}

/// `collect_rule_names`: names split on commas and white space, up to a description: `--`, or a
/// `-` between white space.
fn collect_rule_names(text: &str) -> Vec<&str> {
    let mut rules = Vec::new();
    let mut rule_start = None;
    let mut rule_end = text.len();
    let mut chars = text.char_indices().peekable();
    let mut previous = None;
    while let Some((index, ch)) = chars.next() {
        let is_description_start = ch == '-'
            && chars.peek().map(|(_, next)| *next).is_some_and(|next| {
                next == '-' || (previous.is_some_and(char::is_whitespace) && next.is_whitespace())
            });
        if is_description_start {
            rule_end = index;
            break;
        }
        if ch == ',' || ch.is_whitespace() {
            if let Some(start) = rule_start.take() {
                rules.push(&text[start..index]);
            }
        } else if rule_start.is_none() {
            rule_start = Some(index);
        }
        previous = Some(ch);
    }
    if let Some(start) = rule_start {
        rules.push(&text[start..rule_end]);
    }
    rules
}

#[cfg(test)]
mod tests {
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
}
