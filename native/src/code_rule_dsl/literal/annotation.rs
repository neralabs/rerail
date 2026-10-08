//! `annotation`: a string type definition parsed as a Python expression, as Ruff 0.16.9's
//! `parse_type_annotation` reads it. A simple string (one part whose source is its value) is read
//! in place: a triple-quoted one as a parenthesized expression, others as an expression; its nodes
//! map to the exact source bytes. Any other string's value is read as an expression and every node
//! maps to the whole string. A string Ruff refuses is an `ERROR` root.
//!
//! The scope table parses each string it classifies as a type definition (see `scope::python`) and
//! reads the names inside; `parse` reads the table's trees.

use super::offsets::{self, Decode};
use super::{Grammar, HostMap, PNode, SubTree, semantic};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::value::Value;
use ast_grep_language::LanguageExt;
use std::ops::Range;
use std::sync::OnceLock;

/// tree-sitter's ERROR symbol, which `kind: "ERROR"` names.
pub(crate) const ERROR_KIND: u16 = u16::MAX;

/// Python's hard keywords that tree-sitter-python may read as identifiers.
const KEYWORDS: &[&str] = &["async", "await"];

pub(crate) fn python() -> &'static tree_sitter::Language {
    static PYTHON: OnceLock<tree_sitter::Language> = OnceLock::new();
    PYTHON.get_or_init(|| GrammarDialect::Python.get_ts_language())
}

/// The kind name of a Python symbol id, `ERROR` included.
pub(crate) fn kind_name(id: u16) -> &'static str {
    if id == ERROR_KIND {
        return "ERROR";
    }
    python().node_kind_for_id(id).unwrap_or("")
}

/// Parses a plain string literal (no `f`, `b` or `t` prefix) as Ruff parses a string annotation.
/// `offset` shifts the tree's node ranges (the table's synthetic ranges). `None` for a literal
/// whose value is not known.
///
/// A string Ruff's parser refuses is one `ERROR` root. Otherwise the semantic syntax errors Ruff's
/// checker reports in the expression where `host` places the string (`semantic`) are `ERROR` nodes
/// after the expression, children of the root, each over the offending node (the whole string
/// for a complex one) and valued by Ruff's message.
pub(crate) fn parse<D: ast_grep_core::Doc>(
    node: &ast_grep_core::Node<'_, D>,
    offset: usize,
    host: &semantic::Host<'_>,
) -> Option<SubTree> {
    let decoded = match offsets::decode_python(node) {
        Decode::Known(decoded) => decoded,
        Decode::Unknown | Decode::NotString => return None,
    };
    let whole = decoded.host.clone();
    let text = decoded.text.clone();
    let (root, map) = match decoded.simple {
        Some(simple) => (
            expression(&text, simple.triple),
            HostMap::Exact(simple.content as isize),
        ),
        None => (expression(&text, false), HostMap::Whole(whole.clone())),
    };
    let root = match root {
        Some(mut root) => {
            let source_start = |at: usize| match &map {
                HostMap::Exact(delta) => (at as isize + delta).max(0) as usize,
                _ => whole.start,
            };
            for found in semantic::check(&root, &text, host, &source_start) {
                let mut marker = error(found.start, found.end);
                marker.extra = true;
                marker.value = Some(Value::Str(found.message));
                root.children.push(marker);
            }
            root
        }
        None => error(0, text.len()),
    };
    Some(SubTree::new(Grammar::Annotation, text, offset, root, map))
}

fn error(start: usize, end: usize) -> PNode {
    PNode {
        kind: ERROR_KIND,
        start,
        end,
        named: true,
        error: true,
        extra: false,
        field: 0,
        value: None,
        children: Vec::new(),
    }
}

/// The expression `text` holds, as Ruff's expression mode (or, `parenthesized`, its parenthesized
/// expression mode) reads it; `None` where Ruff reports a syntax error.
pub(crate) fn expression(text: &str, parenthesized: bool) -> Option<PNode> {
    // Nothing to read, or (outside parentheses) a first line that is indented.
    if text.trim().is_empty() {
        return None;
    }
    if !parenthesized {
        let first = text
            .split('\n')
            .find(|line| !line.trim().is_empty())
            .unwrap_or_default();
        if first.starts_with([' ', '\t', '\x0c']) {
            return None;
        }
    }
    // The closing parenthesis on a line of its own, so a trailing comment does not hide it.
    let source = if parenthesized {
        format!("({text}\n)")
    } else {
        text.to_owned()
    };
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(python()).ok()?;
    let tree = parser.parse(&source, None)?;
    let module = tree.root_node();
    if module.has_error() {
        return None;
    }
    let mut cursor = module.walk();
    let children: Vec<tree_sitter::Node<'_>> = module
        .children(&mut cursor)
        .filter(|child| child.kind() != "comment")
        .collect();
    // One statement, no `;`.
    let [statement] = children.as_slice() else {
        return None;
    };
    if statement.kind() != "expression_statement" {
        return None;
    }
    let mut cursor = statement.walk();
    let expressions: Vec<tree_sitter::Node<'_>> = statement
        .named_children(&mut cursor)
        .filter(|child| child.kind() != "comment")
        .collect();
    let mut root = match expressions.as_slice() {
        [single] => *single,
        // `a, b` is a tuple.
        [_, _, ..] => *statement,
        [] => return None,
    };
    let shift = usize::from(parenthesized);
    if parenthesized {
        // The parentheses are the mode's, not the text's: what they hold is the expression.
        if root.kind() != "parenthesized_expression" {
            // `(a, b)` reads as a tuple over the added parentheses: Ruff's tuple. Without a comma
            // it holds nothing (only a comment) or a starred expression, which Ruff refuses.
            let mut cursor = root.walk();
            let comma = root.children(&mut cursor).any(|child| child.kind() == ",");
            if root.kind() != "tuple" || !comma {
                return None;
            }
        } else {
            let mut cursor = root.walk();
            let inner: Vec<tree_sitter::Node<'_>> = root
                .named_children(&mut cursor)
                .filter(|child| child.kind() != "comment")
                .collect();
            match inner.as_slice() {
                [single] => root = *single,
                _ => return None,
            }
        }
    }
    if !valid(&root, &source) || (!parenthesized && breaks_line(&root, &source)) {
        return None;
    }
    let clamp = |range: Range<usize>| {
        let start = range.start.saturating_sub(shift).min(text.len());
        let end = range.end.saturating_sub(shift).clamp(start, text.len());
        start..end
    };
    // The root is an `expression_statement` over the whole text holding the expression (Ruff's
    // `ModExpression`), so a rule on the root can reach every node of the expression with `has`.
    if root.kind() == "expression_statement" {
        let mut tuple = convert(root, &clamp);
        tuple.start = 0;
        tuple.end = text.len();
        return Some(tuple);
    }
    Some(PNode {
        kind: python().id_for_node_kind("expression_statement", true),
        start: 0,
        end: text.len(),
        named: true,
        error: false,
        extra: false,
        field: 0,
        value: None,
        children: vec![convert(root, &clamp)],
    })
}

/// Ruff's parser refuses what tree-sitter-python accepts here: a hard keyword as a name, a
/// starred expression, an unparenthesized walrus or assignment expression at the top, a `yield`
/// anywhere but alone in parentheses, `yield from` a starred expression, and `await` of an
/// `await`. (`await` and a parenthesized `yield` parse; Ruff's semantic checks report them.)
fn valid(root: &tree_sitter::Node<'_>, source: &str) -> bool {
    if matches!(
        root.kind(),
        "named_expression" | "list_splat" | "yield" | "assignment" | "augmented_assignment"
    ) {
        return false;
    }
    let mut stack = vec![(*root, 0)];
    while let Some((node, depth)) = stack.pop() {
        // Deeper than a decoded value may nest (`value::MAX_DEPTH`): not read.
        if depth > crate::code_rule_dsl::value::MAX_DEPTH {
            return false;
        }
        let mut cursor = node.walk();
        let named: Vec<tree_sitter::Node<'_>> = node
            .named_children(&mut cursor)
            .filter(|child| child.kind() != "comment")
            .collect();
        match node.kind() {
            "yield" if node.is_named() => {
                let alone = node.parent().is_some_and(|parent| {
                    parent.kind() == "parenthesized_expression" && parent.named_child_count() == 1
                });
                let mut cursor = node.walk();
                let from = node
                    .children(&mut cursor)
                    .any(|token| token.kind() == "from");
                if !alone || (from && named.iter().any(|value| value.kind() == "list_splat")) {
                    return false;
                }
            }
            "await" if named.iter().any(|operand| operand.kind() == "await") => return false,
            "identifier" if KEYWORDS.contains(&&source[node.byte_range()]) => return false,
            _ => {}
        }
        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor).map(|child| (child, depth + 1)));
    }
    true
}

/// Whether a line ends inside the expression outside any bracket: Ruff's expression mode ends the
/// expression there, where tree-sitter-python may read on. A backslash continues the line.
fn breaks_line(root: &tree_sitter::Node<'_>, source: &str) -> bool {
    let mut leaves = Vec::new();
    let mut stack = vec![*root];
    while let Some(node) = stack.pop() {
        if node.child_count() == 0 || node.kind() == "string" {
            leaves.push(node);
            continue;
        }
        let mut cursor = node.walk();
        let children: Vec<_> = node.children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    let mut depth = 0usize;
    for pair in leaves.windows(2) {
        let (left, right) = (pair[0], pair[1]);
        match left.kind() {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => depth = depth.saturating_sub(1),
            _ => {}
        }
        let gap = &source[left.end_byte()..right.start_byte()];
        if depth == 0 && gap.contains('\n') && !gap.contains("\\\n") && !gap.contains("\\\r") {
            return true;
        }
    }
    false
}

/// A tree-sitter subtree as a sub-tree node, its ranges passed through `clamp`.
fn convert(root: tree_sitter::Node<'_>, clamp: &dyn Fn(Range<usize>) -> Range<usize>) -> PNode {
    fn node(
        cursor: &mut tree_sitter::TreeCursor<'_>,
        clamp: &dyn Fn(Range<usize>) -> Range<usize>,
    ) -> PNode {
        let current = cursor.node();
        let range = clamp(current.byte_range());
        let mut built = PNode {
            kind: current.kind_id(),
            start: range.start,
            end: range.end,
            named: current.is_named(),
            error: false,
            extra: current.is_extra(),
            field: cursor.field_id().map_or(0, |id| id.get()),
            value: None,
            children: Vec::new(),
        };
        if cursor.goto_first_child() {
            loop {
                built.children.push(node(cursor, clamp));
                if !cursor.goto_next_sibling() {
                    break;
                }
            }
            cursor.goto_parent();
        }
        built
    }
    let mut cursor = root.walk();
    node(&mut cursor, clamp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::oracle;

    fn kinds(text: &str, parenthesized: bool) -> Option<String> {
        fn describe(node: &PNode, text: &str) -> String {
            let children: Vec<String> = node
                .children
                .iter()
                .filter(|child| child.named)
                .map(|child| describe(child, text))
                .collect();
            if children.is_empty() {
                format!("{}:{}", kind_name(node.kind), &text[node.start..node.end])
            } else {
                format!("{}({})", kind_name(node.kind), children.join(" "))
            }
        }
        expression(text, parenthesized).map(|root| describe(&root, text))
    }

    #[test]
    fn annotations_are_expressions_as_ruff_reads_them() {
        let statement = |inner: &str| format!("expression_statement({inner})");
        assert_eq!(
            kinds("List[int]", false),
            Some(statement("subscript(identifier:List identifier:int)"))
        );
        assert_eq!(kinds("int ", false), Some(statement("identifier:int")));
        assert_eq!(kinds("int # x", false), Some(statement("identifier:int")));
        assert_eq!(
            kinds("a, b", false).as_deref(),
            Some("expression_statement(identifier:a identifier:b)")
        );
        assert_eq!(
            kinds("\nint |\n  str", true),
            Some(statement("binary_operator(identifier:int identifier:str)"))
        );
        assert_eq!(kinds(" int", true), Some(statement("identifier:int")));
        assert_eq!(
            kinds("(int)", true),
            Some(statement("parenthesized_expression(identifier:int)"))
        );
        for refused in [
            " int",
            "async",
            "print x",
            "a b",
            "(int",
            "int\\",
            "x := 1",
            "*a",
            "yield x",
            "await",
            "await await x",
            "f(yield x)",
            "",
            " ",
            "1 +",
            "a if b",
            "(a) or (b",
            "int |\nstr",
            "a = 1",
        ] {
            assert_eq!(kinds(refused, false), None, "{refused:?}");
        }
        assert_eq!(
            kinds("lambda: 1", false),
            Some(statement("lambda(integer:1)"))
        );
        assert_eq!(kinds("match", false), Some(statement("identifier:match")));
        assert_eq!(
            kinds("await x", false),
            Some(statement("await(identifier:x)"))
        );
        assert_eq!(
            kinds("(yield)", false),
            Some(statement("parenthesized_expression(yield:yield)"))
        );
    }

    /// Every case of `fixtures/python/literal/annotation.expected.json`, written in its host and read through the
    /// scope table: a refused literal where Ruff 0.16.9's parser reports F722 on the whole
    /// literal, and otherwise an `ERROR` node after the expression for each semantic syntax error
    /// its checker reports, at the same range with the same message. In a complex (escaped)
    /// string Ruff relocates every expression to the whole literal but leaves a lambda
    /// parameter's, a keyword argument's and a comprehension clause's range at its offset in the
    /// value, which it then reads as a file offset; the tree maps every node to the whole
    /// literal, so the escaped cases' reports are read there.
    #[test]
    fn annotations_report_what_ruff_reports() {
        use super::super::SubDoc;
        use crate::code_rule_dsl::scope::{Reading, ScopeTable};
        use ast_grep_core::AstGrep;
        use std::sync::Arc;
        let oracle: serde_json::Value =
            serde_json::from_str(oracle!("python/literal/annotation.expected.json"))
                .expect("oracle");
        let cases = oracle["cases"].as_array().expect("cases");
        assert!(cases.len() > 2_000);
        let hosts: std::collections::HashMap<&str, &str> = [
            ("module", "x: {}\n"),
            ("function", "def f():\n    x: {}\n"),
            ("async", "async def f():\n    x: {}\n"),
            ("class", "class C:\n    x: {}\n"),
            ("parameter", "async def f(p: {}): pass\n"),
            ("global", "def f():\n    x: {}\n    global a, b\n"),
        ]
        .into_iter()
        .collect();
        let mut mismatches = Vec::new();
        for case in cases {
            let value = case[0].as_str().expect("value");
            let style = case[1].as_str().expect("style");
            let literal = case[2].as_str().expect("literal");
            let host = case[3].as_str().expect("host");
            let template = hosts[host];
            let source = template.replace("{}", literal);
            let at = template.find("{}").expect("placeholder");
            let whole = literal.len() as i64;
            let mut theirs: Vec<String> = case[4]
                .as_array()
                .expect("reports")
                .iter()
                .map(|report| {
                    let (mut start, mut end) =
                        (report[0].as_i64().unwrap(), report[1].as_i64().unwrap());
                    if style == "escaped" {
                        (start, end) = (0, whole);
                    }
                    format!("{start}..{end} {}", report[2].as_str().unwrap())
                })
                .collect();
            let root = GrammarDialect::Python.ast_grep(&source);
            let table = ScopeTable::build_with(
                root.root(),
                GrammarDialect::Python,
                usize::MAX,
                &|| false,
                Reading::default(),
            )
            .expect("table builds");
            let Some(annotation) = table.annotations.iter().find(|found| found.start == at) else {
                mismatches.push(format!("{host} {style} {value:?}: no type definition"));
                continue;
            };
            let tree = Arc::clone(&annotation.tree);
            let mut ours: Vec<String> = if tree.is_error() {
                vec![format!("0..{whole} syntax")]
            } else {
                let document = AstGrep::doc(SubDoc {
                    tree: Arc::clone(&tree),
                });
                document
                    .root()
                    .children()
                    .filter(|child| child.kind() == "ERROR")
                    .map(|child| {
                        let range = tree.host_range(child.node_id()).expect("range");
                        let message = match tree.value(child.node_id()) {
                            Some(Value::Str(message)) => message.clone(),
                            other => format!("{other:?}"),
                        };
                        format!("{}..{} {message}", range.start - at, range.end - at)
                    })
                    .collect()
            };
            // Ruff's parser reports once, on the whole literal, with its own message.
            if tree.is_error()
                && theirs.len() == 1
                && theirs[0].starts_with(&format!("0..{whole} "))
            {
                theirs = vec![format!("0..{whole} syntax")];
            }
            ours.sort();
            theirs.sort();
            if ours != theirs {
                mismatches.push(format!(
                    "{host} {style} {value:?}: Ruff {theirs:?}, Rerail {ours:?}"
                ));
            }
        }
        assert_eq!(
            mismatches,
            Vec::<String>::new(),
            "\n{}",
            mismatches.join("\n")
        );
    }
}
