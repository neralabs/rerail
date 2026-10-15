//! Rerail's own suppression comment, `rerail-ignore RULE-ID[, RULE-ID ...] [-- reason]`: always
//! honoured, for every code rule, whatever the repository's `suppressions` setting (which governs
//! only other linters' comments, `suppress`). A directive covers a node of the file's tree:
//!
//! - before the file's first code (only comments, white space, a shebang and PHP's `<?php` before
//!   it): the whole file;
//! - on its own line (leading): the next named sibling after it in its parent, comments passed
//!   over (before a Python body's first statement, that statement); nothing where none follows
//!   (the end of a block);
//! - after code on the line where it starts (trailing): the nearest preceding named sibling, when
//!   only comments stand between them and it ends on that line; past a `,` or `;` ending a list
//!   element or class field on that line, the elements before it on the line, from the one
//!   holding the line's first code byte; otherwise the smallest ancestor of the comment holding
//!   that byte (never the file's root).
//!
//! A violation is silenced when the byte its anchor names falls in a range covered for its rule's
//! id: the anchor's start, or for an end anchor the witness's last byte (the point where a witness
//! ends is where the next node may start).

use crate::code_rule_dsl::lines::LineTable;
use ast_grep_core::{Doc, Node};
use std::ops::Range;

/// The directive's name.
const DIRECTIVE: &str = "rerail-ignore";

/// A byte range covered for the rule ids a directive names.
#[derive(Debug, PartialEq, Eq)]
struct Cover {
    /// Exclusive of its end, as the covered node's; the whole file's runs to `usize::MAX`, past an
    /// anchor at the end of the file.
    range: Range<usize>,
    rules: Vec<String>,
}

/// A file's `rerail-ignore` directives, as the ranges they cover.
#[derive(Debug, Default)]
pub(crate) struct RerailIgnores {
    covers: Vec<Cover>,
}

impl RerailIgnores {
    /// Reads every comment of the file `root` is the root of; `source` is its whole text.
    pub(crate) fn read<D: Doc>(root: &Node<'_, D>, source: &str, lines: &LineTable) -> Self {
        let mut covers = Vec::new();
        // A file that never spells the directive holds none: no walk.
        if !source.contains(DIRECTIVE) {
            return Self { covers };
        }
        let mut first = None;
        for comment in root.dfs().filter(is_comment) {
            let range = comment.range();
            let rules = rule_ids(&source[range.clone()]);
            if rules.is_empty() {
                continue;
            }
            let first = *first.get_or_insert_with(|| first_code(root, source));
            let covered = if range.end <= first {
                Some(0..usize::MAX)
            } else {
                covered(&comment, source, lines)
            };
            if let Some(range) = covered {
                covers.push(Cover {
                    range,
                    rules: rules.into_iter().map(str::to_owned).collect(),
                });
            }
        }
        Self { covers }
    }

    /// Whether any directive of the file names `rule`.
    pub(crate) fn names(&self, rule: &str) -> bool {
        self.covers
            .iter()
            .any(|cover| cover.rules.iter().any(|name| name == rule))
    }

    /// Whether a violation of `rule` whose anchor names byte `at` is silenced.
    pub(crate) fn ignores(&self, rule: &str, at: usize) -> bool {
        self.covers
            .iter()
            .any(|cover| cover.range.contains(&at) && cover.rules.iter().any(|name| name == rule))
    }
}

/// Comment kinds across Rerail's grammars: every grammar's `comment` (a Blade template's
/// `<!-- -->` and its embedded PHP's), a script's `html_comment`, Blade's `{{-- --}}` and Rust's
/// `line_comment` and `block_comment`.
fn is_comment<D: Doc>(node: &Node<'_, D>) -> bool {
    matches!(
        node.kind().as_ref(),
        "comment" | "html_comment" | "blade_comment" | "line_comment" | "block_comment"
    )
}

/// The rule ids a comment's directive names, from the whole comment text (delimiters included):
/// after the comment's opener and any `*` and white space, `rerail-ignore`, then white space and
/// ids separated by commas or white space, up to the end of the line or a ` -- ` reason. None
/// where the comment is no directive, or one naming no rule.
fn rule_ids(comment: &str) -> Vec<&str> {
    let mut text = comment.trim_end();
    if let Some(inner) = ["--}}", "-->", "*/"]
        .into_iter()
        .find_map(|closer| text.strip_suffix(closer))
    {
        text = inner;
    }
    // A script's HTML-like comment opens with `<!--`, or with `-->` at the start of a line.
    let Some(inner) = ["{{--", "<!--", "-->", "//", "/*", "#", "--"]
        .into_iter()
        .find_map(|opener| text.strip_prefix(opener))
    else {
        return Vec::new();
    };
    let text = inner.trim_start_matches(|c: char| c == '*' || c.is_whitespace());
    let Some(rest) = text.strip_prefix(DIRECTIVE) else {
        return Vec::new();
    };
    // `rerail-ignored` or `rerail-ignore-next` is another word, no directive.
    if !rest.starts_with(|c: char| c.is_whitespace()) {
        return Vec::new();
    }
    let line = rest.split(['\n', '\r']).next().unwrap_or_default();
    before_reason(line)
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|id| !id.is_empty())
        .collect()
}

/// `list` up to a reason: two or more dashes with white space before them and white space (or the
/// end) after, as an ESLint directive's description starts.
fn before_reason(list: &str) -> &str {
    let bytes = list.as_bytes();
    for (at, byte) in bytes.iter().enumerate() {
        if !byte.is_ascii_whitespace() {
            continue;
        }
        let dashes = bytes[at + 1..]
            .iter()
            .take_while(|byte| **byte == b'-')
            .count();
        let after = at + 1 + dashes;
        if dashes >= 2 && bytes.get(after).is_none_or(u8::is_ascii_whitespace) {
            return &list[..at];
        }
    }
    list
}

/// The first byte of the file's code: the earliest leaf outside comments that is no shebang (`#!`
/// line), PHP open tag or white space. The file's length where it has none.
fn first_code<D: Doc>(root: &Node<'_, D>, source: &str) -> usize {
    let mut first = source.len();
    let mut stack = vec![root.clone()];
    while let Some(node) = stack.pop() {
        let range = node.range();
        // A comment's tokens (Blade's `{{--` and its text) are no code; nor is anything after
        // code already found.
        if is_comment(&node) || range.start >= first {
            continue;
        }
        if !node.is_leaf() {
            stack.extend(node.children());
            continue;
        }
        let text = &source[range.clone()];
        let shebang = range.start == 0 && text.starts_with("#!") && !text.trim_end().contains('\n');
        if !text.trim().is_empty()
            && !shebang
            && !matches!(node.kind().as_ref(), "hash_bang_line" | "php_tag")
        {
            first = range.start;
        }
    }
    first
}

/// Whether a node is code a directive can cover: named, no comment, not white space alone.
fn is_code<D: Doc>(node: &Node<'_, D>, source: &str) -> bool {
    node.is_named() && !is_comment(node) && !source[node.range()].trim().is_empty()
}

/// The range a directive comment past the file's first code covers.
fn covered<D: Doc>(comment: &Node<'_, D>, source: &str, lines: &LineTable) -> Option<Range<usize>> {
    let start = comment.range().start;
    let line = lines.line(start);
    let line_start = lines.start(line);
    let before = &source[line_start..start];
    if before.trim().is_empty() {
        // Leading: the next code among its siblings. Python places a comment before a body's
        // first statement beside the body (`block`, which starts at that statement), not in it:
        // the comment leads that statement.
        let mut next = comment.next();
        while let Some(node) = next {
            if is_code(&node, source) {
                let first = (node.kind() == "block")
                    .then(|| node.children().find(|child| is_code(child, source)))
                    .flatten()
                    .filter(|child| child.range().start == node.range().start);
                return Some(first.unwrap_or(node).range());
            }
            next = node.next();
        }
        return None;
    }
    // Trailing: the code it directly follows, when that ends on its line. Past a `,` or `;` that
    // ends there (a list element's or class field's), the elements before it on the line, from
    // the one holding the line's first code byte: in `m(n, //` that is no element but the call
    // opened on the line.
    let code = line_start + (before.len() - before.trim_start().len());
    // Whether the walk passed a separator, and the end of the element that separator closes.
    let mut separated = false;
    let mut end = None;
    let mut previous = comment.prev();
    while let Some(node) = previous {
        previous = node.prev();
        if is_comment(&node) || source[node.range()].trim().is_empty() {
            continue;
        }
        let range = node.range();
        if lines.line(range.end.saturating_sub(1).max(range.start)) != line {
            break;
        }
        if !node.is_named() {
            if matches!(node.kind().as_ref(), "," | ";") {
                separated = true;
                continue;
            }
            break;
        }
        if !separated {
            return Some(range);
        }
        let end = *end.get_or_insert(range.end);
        if range.start <= code {
            return Some(range.start..end);
        }
    }
    // Otherwise the smallest construct holding the line's code: the comment sits inside it.
    let mut ancestor = comment.parent();
    while let Some(node) = ancestor {
        let parent = node.parent();
        parent.as_ref()?;
        if node.range().start <= code {
            return Some(node.range());
        }
        ancestor = parent;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::lines::Terminators;
    use crate::code_rule_dsl::test_support::*;
    use ast_grep_language::LanguageExt;
    use serde_json::{Value, json};

    #[test]
    fn reads_rule_ids_from_a_directive_comment() {
        for (comment, ids) in [
            ("// rerail-ignore a", vec!["a"]),
            ("// rerail-ignore a, b c,d", vec!["a", "b", "c", "d"]),
            (
                "//rerail-ignore acme:no-print -- generated code",
                vec!["acme:no-print"],
            ),
            ("// rerail-ignore a --- b", vec!["a"]),
            ("// rerail-ignore a-b --x", vec!["a-b", "--x"]),
            (
                "# rerail-ignore complexity-cyclomatic-python",
                vec!["complexity-cyclomatic-python"],
            ),
            ("/* rerail-ignore x.y_z/w */", vec!["x.y_z/w"]),
            ("/**\n * rerail-ignore a\n * b\n */", vec!["a"]),
            ("/** rerail-ignore a */", vec!["a"]),
            ("{{-- rerail-ignore a --}}", vec!["a"]),
            ("<!-- rerail-ignore a -->", vec!["a"]),
            ("<!-- rerail-ignore a", vec!["a"]),
            ("--> rerail-ignore a -- generated", vec!["a"]),
            ("# rerail-ignore a\n", vec!["a"]),
            // No id: nothing.
            ("// rerail-ignore", vec![]),
            ("// rerail-ignore -- reason", vec![]),
            ("// rerail-ignore\n", vec![]),
            // Not the directive.
            ("// rerail-ignored a", vec![]),
            ("// rerail-ignore-next a", vec![]),
            ("// Rerail-Ignore a", vec![]),
            ("// see rerail-ignore a", vec![]),
            ("## rerail-ignore a", vec![]),
        ] {
            assert_eq!(rule_ids(comment), ids, "{comment:?}");
        }
    }

    /// The covered ranges of a TypeScript, Python or PHP source, as the texts they cover.
    fn covered_texts(language: GrammarDialect, source: &str) -> Vec<(String, Vec<String>)> {
        let root = language.ast_grep(source);
        let lines = LineTable::new(source, Terminators::of(language));
        RerailIgnores::read(&root.root(), source, &lines)
            .covers
            .into_iter()
            .map(|cover| {
                let end = cover.range.end.min(source.len());
                (source[cover.range.start..end].to_owned(), cover.rules)
            })
            .collect()
    }

    fn cover(text: &str, rules: &[&str]) -> (String, Vec<String>) {
        (
            text.to_owned(),
            rules.iter().map(|rule| (*rule).to_owned()).collect(),
        )
    }

    #[test]
    fn a_directive_covers_its_node() {
        let source = "#!/usr/bin/env node\n// header\n// rerail-ignore whole\nconst a = 1;\n// rerail-ignore lead\nfunction f() {\n  g();\n}\nfunction h() {}\nb(); // rerail-ignore trail\nc(); d(); // rerail-ignore last\nconst e = k( // rerail-ignore open\n  1,\n);\nm(n, // rerail-ignore inner\n  o);\nfunction p() {\n  q();\n  // rerail-ignore end\n}\nconst s = \"// rerail-ignore string\";\n";
        assert_eq!(
            covered_texts(GrammarDialect::TypeScript, source),
            [
                cover(source, &["whole"]),
                cover("function f() {\n  g();\n}", &["lead"]),
                cover("b();", &["trail"]),
                cover("d();", &["last"]),
                cover("const e = k( // rerail-ignore open\n  1,\n);", &["open"]),
                cover("m(n, // rerail-ignore inner\n  o)", &["inner"]),
            ]
        );
    }

    #[test]
    fn a_directive_covers_its_node_in_python_and_php() {
        let python = "# rerail-ignore whole\nimport os\n# rerail-ignore lead\ndef f():\n    a = 1\n    return a\n\ndef g():\n    pass\nx = h(  # rerail-ignore open\n    1)\nif y:  # rerail-ignore cond\n    z()\ndef k():\n    # rerail-ignore body\n    a()\n    b()\n";
        assert_eq!(
            covered_texts(GrammarDialect::Python, python),
            [
                cover(python, &["whole"]),
                cover("def f():\n    a = 1\n    return a", &["lead"]),
                cover("x = h(  # rerail-ignore open\n    1)", &["open"]),
                cover("if y:  # rerail-ignore cond\n    z()", &["cond"]),
                cover("a()", &["body"]),
            ]
        );
        let php = "<?php\n/** rerail-ignore whole */\n$a = 1;\n# rerail-ignore lead\nfunction f() {\n    return 1;\n}\n$b = g(); // rerail-ignore trail\n$c = \"# rerail-ignore string\";\n";
        assert_eq!(
            covered_texts(GrammarDialect::Php, php),
            [
                cover(php, &["whole"]),
                cover("function f() {\n    return 1;\n}", &["lead"]),
                cover("$b = g();", &["trail"]),
            ]
        );
    }

    /// One code rule, `id`, reporting each node of `kind` in `language`, with `suppress` where
    /// given.
    fn rule(id: &str, language: &str, kind: &str, suppress: Option<Value>) -> Value {
        let mut violation = violation();
        if let Some(suppress) = suppress {
            violation["suppress"] = suppress;
        }
        json!({ "format": "rerail.policy/v1", "id": id, "revision": 1, "mode": "block",
            "clauses": [{ "id": "c", "violation": violation,
                "forbid": { "ast": { "language": language, "rule": { "kind": kind } } } }] })
    }

    fn reported(document: Value, language: &str, path: &str, source: &str) -> Vec<String> {
        let compiled = compile_document_with(
            document,
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        )
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let result = evaluate_file(&compiled, language, path, source);
        assert_eq!(result["failures"], json!([]), "{result}");
        matched_texts(&result, source)
    }

    /// Through the engine, with no `suppress`: the file-top directive covers the file, a leading
    /// one the next node (a violation on its third line too, none in the next function), a
    /// trailing one its statement, and another rule's id nothing.
    #[test]
    fn the_engine_drops_ignored_witnesses_in_every_language() {
        let source = "function f() {\n  a();\n  b();\n}\n// rerail-ignore calls\nfunction g() {\n  c();\n  d();\n}\nfunction h() {\n  e();\n}\nk( // rerail-ignore calls\n  l(),\n);\nm(); // rerail-ignore other\n";
        let document = rule("calls", "typescript", "call_expression", None);
        assert_eq!(
            reported(document.clone(), "typescript", "a.ts", source),
            ["a()", "b()", "e()", "m()"]
        );
        let top = format!("// rerail-ignore other, calls -- generated\n{source}");
        assert!(reported(document, "typescript", "a.ts", &top).is_empty());

        let python = "def f():\n    a()\n\n# rerail-ignore calls\ndef g():\n    b()\n    c()\n\nd()  # rerail-ignore calls\ne()  # rerail-ignore other\ns = \"# rerail-ignore calls\"\n";
        assert_eq!(
            reported(
                rule("calls", "python", "call", None),
                "python",
                "a.py",
                python
            ),
            ["a()", "e()"]
        );

        let php = "<?php\nfunction f() {\n    a();\n}\n// rerail-ignore calls\nfunction g() {\n    b();\n    c();\n}\nd(); # rerail-ignore calls\ne(); /* rerail-ignore other */\n";
        let php_rule = rule("calls", "php", "function_call_expression", None);
        assert_eq!(
            reported(php_rule.clone(), "php", "a.php", php),
            ["a()", "e()"]
        );
        let top = php.replacen("<?php\n", "<?php\n/* rerail-ignore calls */\n", 1);
        assert!(reported(php_rule, "php", "a.php", &top).is_empty());

        let blade = "<div>{{ a() }}</div>\n{{-- rerail-ignore calls --}}\n<p>{{ b() }}</p>\n@php\n    // rerail-ignore calls\n    c();\n    d();\n@endphp\n<span>{{ e() }}</span>\n";
        let blade_rule = rule("calls", "blade", "function_call_expression", None);
        assert_eq!(
            reported(blade_rule.clone(), "blade", "a.blade.php", blade),
            ["a()", "d()", "e()"]
        );
        let top = format!("{{{{-- rerail-ignore calls --}}}}\n{blade}");
        assert!(reported(blade_rule, "blade", "a.blade.php", &top).is_empty());

        let rust = "fn f() {\n    a();\n}\n// rerail-ignore calls\nfn g() {\n    b();\n    c();\n}\nfn h() {\n    d(); /* rerail-ignore calls */\n    e(); // rerail-ignore other\n    let s = \"// rerail-ignore calls\";\n}\n";
        let rust_rule = rule("calls", "rust", "call_expression", None);
        assert_eq!(
            reported(rust_rule.clone(), "rust", "a.rs", rust),
            ["a()", "e()"]
        );
        let top = format!("// rerail-ignore calls\n{rust}");
        assert!(reported(rust_rule, "rust", "a.rs", &top).is_empty());
    }

    /// A trailing directive after a list element's `,` or a class field's `;` covers that element
    /// or field (the ones before it on its line too), not the argument list, object or class body
    /// holding it: the later elements and members stay reported.
    #[test]
    fn a_trailing_directive_past_a_separator_covers_the_element_before_it() {
        let call = "foo(\n  a(), // rerail-ignore calls\n  b(),\n  c(),\n);\n";
        let pair = "foo(\n  a(), b(), // rerail-ignore calls\n  c(),\n);\n";
        let object = "const o = {\n  a: 1, // rerail-ignore keys\n  b: 2,\n};\n";
        let class = "class A {\n  x = f(); // rerail-ignore members\n  m() {}\n  n() {}\n}\n";
        let python = "foo(\n    a(),  # rerail-ignore calls\n    b(),\n)\n";
        for (language, source, expected) in [
            (GrammarDialect::TypeScript, call, cover("a()", &["calls"])),
            (
                GrammarDialect::TypeScript,
                pair,
                cover("a(), b()", &["calls"]),
            ),
            (GrammarDialect::TypeScript, object, cover("a: 1", &["keys"])),
            (
                GrammarDialect::TypeScript,
                class,
                cover("x = f()", &["members"]),
            ),
            (GrammarDialect::Python, python, cover("a()", &["calls"])),
        ] {
            assert_eq!(covered_texts(language, source), [expected], "{source:?}");
        }

        let calls = rule("calls", "typescript", "call_expression", None);
        assert_eq!(
            reported(calls, "typescript", "a.ts", call),
            [call.trim_end().trim_end_matches(';'), "b()", "c()"]
        );
        let mut members = rule("members", "typescript", "public_field_definition", None);
        members["clauses"][0]["forbid"]["ast"]["rule"] = json!({ "any": [
            { "kind": "public_field_definition" }, { "kind": "method_definition" }
        ] });
        assert_eq!(
            reported(members, "typescript", "a.ts", class),
            ["m() {}", "n() {}"]
        );
    }

    /// A covered node's range excludes its end: a leading directive over `a();` leaves `b();`,
    /// which starts where `a();` ends, reported, and a trailing one covers `b();` only. An end
    /// anchor names its witness's last byte, so `a();`'s, at `b();`'s start, stays covered by the
    /// directive over `a();` and out of the one over `b();`.
    #[test]
    fn a_directive_covers_no_sibling_starting_where_its_node_ends() {
        let leading = "function f() {\n// rerail-ignore probe\na();b();\n}\n";
        let spaced = "function f() {\n// rerail-ignore probe\na(); b();\n}\n";
        let trailing = "a();b(); // rerail-ignore probe\n";
        for (source, expected) in [(leading, "b();"), (spaced, "b();"), (trailing, "a();")] {
            for (anchor, at) in [
                ("witness.span", source.find(expected).expect("statement")),
                ("witness.start", source.find(expected).expect("statement")),
                (
                    "witness.end",
                    source.find(expected).expect("statement") + expected.len(),
                ),
            ] {
                let mut document = rule("probe", "typescript", "expression_statement", None);
                document["clauses"][0]["violation"]["anchor"] = json!(anchor);
                let compiled = compile_document_with(
                    document,
                    &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
                )
                .unwrap_or_else(|error| panic!("{}", error.reason));
                let result = evaluate_file(&compiled, "typescript", "a.ts", source);
                assert_eq!(result["failures"], json!([]), "{result}");
                let starts = result["violations"]
                    .as_array()
                    .expect("violations")
                    .iter()
                    .map(|violation| violation["span"]["startByte"].as_u64().expect("start"))
                    .collect::<Vec<_>>();
                assert_eq!(starts, [at as u64], "{anchor} in {source:?}");
            }
        }
    }
}
