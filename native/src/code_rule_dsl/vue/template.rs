//! Vue template expressions and regions, matching compiler-dom 3.5.43.

#[cfg(test)]
use crate::code_rule_dsl::language::GrammarDialect;
#[cfg(test)]
use ast_grep_language::LanguageExt;
use std::ops::Range;
use tree_sitter::{Node, Parser};

#[derive(Clone)]
pub(crate) enum Arg {
    None,
    Static(Range<usize>),
    Dynamic(Range<usize>),
}
/// A normalized Vue directive.
pub(crate) struct Directive<'t> {
    pub(crate) node: Node<'t>,
    pub(crate) name: &'t str,
    pub(crate) arg: Arg,
    pub(crate) value: Option<Range<usize>>,
}
fn text<'t>(n: Node<'t>, s: &'t str) -> &'t str {
    &s[n.byte_range()]
}
fn unquote(r: Range<usize>, s: &str) -> Range<usize> {
    if r.end - r.start >= 2
        && matches!(
            (&s[r.start..r.start + 1], &s[r.end - 1..r.end]),
            ("\"", "\"") | ("'", "'")
        )
    {
        r.start + 1..r.end - 1
    } else {
        r
    }
}

/// Whether the start tag child `node` follows its tag name with no byte between, and so belongs
/// to it: HTML's tokenizer, and vue-eslint-parser's, read a tag name up to whitespace, `/` or
/// `>`, but tree-sitter-vue ends `tag_name` at a `.` and reads the rest as `.` shorthand
/// attributes (`<ElTable.TableColumn>` as `ElTable` and `.TableColumn`).
fn in_tag_name(node: Node<'_>) -> bool {
    let mut current = node;
    while let Some(previous) = current.prev_named_sibling() {
        if previous.end_byte() != current.start_byte() {
            return false;
        }
        if previous.kind() == "tag_name" {
            return true;
        }
        current = previous;
    }
    false
}

/// The bytes of the tag name `tag_name` starts, as vue-eslint-parser reads it: with the start
/// tag children that belong to it (`in_tag_name`), `ElTable.TableColumn`.
pub(crate) fn tag_name(tag_name: Node<'_>) -> Range<usize> {
    let mut end = tag_name;
    while let Some(next) = end.next_named_sibling()
        && next.start_byte() == end.end_byte()
    {
        end = next;
    }
    tag_name.start_byte()..end.end_byte()
}

impl<'t> Directive<'t> {
    /// Reads a grammar directive attribute, including Vue 2 scope aliases: `slot-scope` on any
    /// element, `scope` on a `<template>` only (vue-eslint-parser's reading; `<th scope="col">`
    /// is HTML's attribute). An attribute that belongs to the tag name (`in_tag_name`) is none.
    pub(crate) fn read(node: Node<'t>, source: &'t str) -> Option<Self> {
        if in_tag_name(node) {
            return None;
        }
        let kind = node.kind();
        if kind == "attribute" {
            let n = node.named_child(0)?;
            let name = text(n, source);
            let on_template = || {
                let start = node.parent()?;
                let mut cursor = start.walk();
                let tag = start
                    .named_children(&mut cursor)
                    .find(|child| child.kind() == "tag_name")?;
                Some(text(tag, source) == "template")
            };
            if name != "slot-scope" && !(name == "scope" && on_template() == Some(true)) {
                return None;
            }
            let value = node.named_child(1).map(|v| unquote(v.byte_range(), source));
            return Some(Self {
                node,
                name,
                arg: Arg::None,
                value,
            });
        }
        if kind != "directive_attribute" {
            return None;
        }
        let mut c = node.walk();
        let children: Vec<_> = node.named_children(&mut c).collect();
        let dn = children
            .iter()
            .find(|n| n.kind() == "directive_name")
            .copied();
        let prefix = dn.map(|n| text(n, source));
        let name = match prefix {
            Some("v-bind") => "bind",
            Some("v-on") => "on",
            Some("v-slot") => "slot",
            Some(v) if v.starts_with("v-") => &v[2..],
            Some(_) => "",
            None => match source.as_bytes().get(node.start_byte()) {
                Some(b':') | Some(b'.') => "bind",
                Some(b'@') => "on",
                Some(b'#') => "slot",
                _ => "",
            },
        };
        if name.is_empty() {
            return None;
        }
        let dyn_node = children
            .iter()
            .find(|n| n.kind() == "dynamic_directive_value")
            .copied();
        let normal = children
            .iter()
            .find(|n| n.kind() == "directive_value")
            .copied();
        let arg = if let Some(d) = dyn_node {
            let r = d.byte_range();
            let inner = d
                .named_child(0)
                .map(|x| x.byte_range())
                .unwrap_or(r.start + 1..r.end.saturating_sub(1));
            Arg::Dynamic(inner)
        } else if let Some(v) = normal {
            Arg::Static(v.byte_range())
        } else {
            Arg::None
        };
        let value = children
            .iter()
            .find(|n| n.kind() == "quoted_attribute_value" || n.kind() == "attribute_value")
            .map(|v| unquote(v.byte_range(), source));
        Some(Self {
            node,
            name,
            arg,
            value,
        })
    }
}

/// Result of compiler-core's `parseFor`.
pub(crate) struct ForParse {
    pub(crate) alias: Range<usize>,
    pub(crate) parenthesized: bool,
    /// The value, key and index, as `parseFor` finds them (the oracle's `for-alias` regions).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) items: Vec<Range<usize>>,
    pub(crate) source: Range<usize>,
}
/// Parses Vue's `in`/`of` syntax with compiler-core's whitespace and iterator rules.
pub(crate) fn parse_for(source: &str, value: Range<usize>) -> Option<ForParse> {
    let v = &source[value.clone()];
    let bytes = v.as_bytes();
    let mut split = None;
    for i in 1..bytes.len().saturating_sub(1) {
        if !bytes[i].is_ascii_alphabetic() {
            continue;
        }
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_alphabetic() {
            j += 1;
        }
        let word = &v[i..j];
        if word != "in" && word != "of" {
            continue;
        }
        if v[..i].chars().last().is_some_and(char::is_whitespace)
            && v[j..].chars().next().is_some_and(char::is_whitespace)
        {
            split = Some((i, j));
            break;
        }
    }
    let (a, b) = split?;
    let alias = value.start..value.start + a;
    let src = value.start + b..value.end;
    if source[src.clone()].trim().is_empty() {
        return None;
    }
    let raw = &source[alias.clone()];
    let t = raw.trim();
    let parenthesized = t.starts_with('(') && t.ends_with(')');
    let inner = if parenthesized { &t[1..t.len() - 1] } else { t };
    let base = alias.start + raw.find(t).unwrap_or(0) + usize::from(parenthesized);
    let mut items = Vec::new();
    let mut last = 0;
    let mut depth = 0;
    for (i, ch) in inner.char_indices() {
        match ch {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => depth -= 1,
            ',' if depth == 0 => {
                let q = inner[last..i].trim();
                if !q.is_empty() {
                    let st = base + last + inner[last..i].find(q).unwrap_or(0);
                    items.push(st..st + q.len());
                }
                last = i + 1;
            }
            _ => {}
        }
    }
    let q = inner[last..].trim();
    if !q.is_empty() {
        let st = base + last + inner[last..].find(q).unwrap_or(0);
        items.push(st..st + q.len());
    }
    Some(ForParse {
        alias,
        parenthesized,
        items,
        source: src,
    })
}
/// How compiler-dom treats a directive expression.
pub(crate) enum Parse {
    /// An expression (the oracle's regions; the view writes expressions without asking).
    #[cfg_attr(not(test), allow(dead_code))]
    Expression,
    /// Parameters (the oracle's regions).
    #[cfg_attr(not(test), allow(dead_code))]
    Params,
    HandlerMember,
    HandlerFunction,
    Statements,
}
/// Classifies a `v-on` value with an argument as compiler-core 3.5.43's `transformOn` does: a
/// member expression or identifier other than `undefined` (`isMemberExpression`) is a handler Vue
/// calls with the event, a function or arrow expression (`isFnExpression`) is the handler itself,
/// anything else (a call, an assignment, several statements, text that is no expression) runs as
/// inline statements. Both checks look through parentheses and TypeScript's `as`, `satisfies`,
/// `!`, `<T>x` and `x<T>` (`unwrapTSNode`).
#[cfg(test)]
pub(crate) fn handler(text: &str, dialect: GrammarDialect) -> Parse {
    let mut parser = Parser::new();
    if parser.set_language(&dialect.get_ts_language()).is_err() {
        return Parse::Statements;
    }
    handler_with(&mut parser, text)
}

/// `handler` with a parser already set to the dialect, for callers classifying many values.
pub(crate) fn handler_with(parser: &mut Parser, text: &str) -> Parse {
    // The newline keeps a trailing `//` comment from swallowing the `)`.
    let wrapped = format!("({text}\n)");
    let Some(tree) = parser.parse(&wrapped, None) else {
        return Parse::Statements;
    };
    let root = tree.root_node();
    if root.has_error() || root.named_child_count() != 1 {
        return Parse::Statements;
    }
    let Some(mut expression) = root
        .named_child(0)
        .filter(|statement| statement.kind() == "expression_statement")
        .and_then(|statement| statement.named_child(0))
    else {
        return Parse::Statements;
    };
    while let Some(inner) = unwrapped(expression) {
        expression = inner;
    }
    match expression.kind() {
        "identifier" if &wrapped[expression.byte_range()] != "undefined" => Parse::HandlerMember,
        "member_expression" | "subscript_expression" => Parse::HandlerMember,
        "arrow_function" | "function_expression" | "function" => Parse::HandlerFunction,
        _ => Parse::Statements,
    }
}

/// The expression a parenthesized expression or a TypeScript wrapper (`unwrapTSNode`) holds.
fn unwrapped(node: Node<'_>) -> Option<Node<'_>> {
    let mut cursor = node.walk();
    let mut operands = node
        .named_children(&mut cursor)
        .filter(|child| child.kind() != "comment");
    match node.kind() {
        "parenthesized_expression"
        | "as_expression"
        | "satisfies_expression"
        | "non_null_expression"
        | "instantiation_expression" => operands.next(),
        // `<T>x`: the type comes first.
        "type_assertion" => operands.last(),
        _ => None,
    }
}

/// A template expression as the view oracle lists it.
#[cfg(test)]
pub(crate) struct Region {
    pub(crate) kind: String,
    pub(crate) range: Range<usize>,
    pub(crate) parse: Parse,
}
/// The template's expressions as the view oracle lists them, sorted.
#[cfg(test)]
pub(crate) fn regions(root: Node<'_>, source: &str, dialect: GrammarDialect) -> Vec<Region> {
    let mut out = Vec::new();
    fn trim(r: Range<usize>, s: &str) -> Option<Range<usize>> {
        let t = s[r.clone()].trim();
        if t.is_empty() {
            None
        } else {
            let a = s[r.clone()].find(t)?;
            Some(r.start + a..r.start + a + t.len())
        }
    }
    fn walk(n: Node<'_>, s: &str, d: GrammarDialect, o: &mut Vec<Region>) {
        if n.kind() == "interpolation"
            && let Some(x) = n.named_child(0)
            && let Some(r) = trim(x.byte_range(), s)
        {
            o.push(Region {
                kind: "interpolation".into(),
                range: r,
                parse: Parse::Expression,
            });
        }
        if (n.kind() == "directive_attribute" || n.kind() == "attribute")
            && let Some(di) = Directive::read(n, s)
        {
            if let Arg::Dynamic(r) = &di.arg {
                o.push(Region {
                    kind: "dynamic-arg".into(),
                    range: r.clone(),
                    parse: Parse::Expression,
                });
            }
            if let Some(v) = di.value.and_then(|r| trim(r, s)) {
                if di.name == "for" {
                    if let Some(f) = parse_for(s, v.clone()) {
                        for r in f.items {
                            o.push(Region {
                                kind: "for-alias".into(),
                                range: r,
                                parse: Parse::Params,
                            });
                        }
                        if let Some(sr) = trim(f.source, s) {
                            o.push(Region {
                                kind: "for-source".into(),
                                range: sr,
                                parse: Parse::Expression,
                            });
                        }
                    }
                } else {
                    let (kind, parse) =
                        if di.name == "slot" || di.name == "slot-scope" || di.name == "scope" {
                            ("slot-params".to_string(), Parse::Params)
                        } else if di.name == "on" && matches!(di.arg, Arg::None) {
                            ("on".to_string(), Parse::Expression)
                        } else if di.name == "on" {
                            ("on".to_string(), handler(&s[v.clone()], d))
                        } else {
                            (di.name.to_string(), Parse::Expression)
                        };
                    o.push(Region {
                        kind,
                        range: v,
                        parse,
                    });
                }
            }
        }
        let mut c = n.walk();
        for x in n.named_children(&mut c) {
            walk(x, s, d, o);
        }
    }
    walk(root, source, dialect, &mut out);
    out.sort_by_key(|r| (r.range.start, r.range.end));
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn for_cases() {
        assert!(parse_for("item in items", 0..13).is_some());
        assert!(parse_for("n in 10", 0..7).is_some());
    }

    #[test]
    fn handlers() {
        use GrammarDialect::{JavaScript, TypeScript};
        for (text, dialect, member, function) in [
            ("foo", JavaScript, true, false),
            ("a.b?.c", JavaScript, true, false),
            ("a[b == c]", JavaScript, true, false),
            ("(foo)", JavaScript, true, false),
            ("foo as any", TypeScript, true, false),
            ("foo!", TypeScript, true, false),
            ("undefined", JavaScript, false, false),
            ("() => x", JavaScript, false, true),
            ("async (e) => go(e)", JavaScript, false, true),
            ("function (e) { go(e) }", JavaScript, false, true),
            ("(e: Event) => go(e)", TypeScript, false, true),
            ("foo()", JavaScript, false, false),
            ("list.map(x => x)", JavaScript, false, false),
            ("a = b", JavaScript, false, false),
            ("a++; b()", JavaScript, false, false),
            ("foo // comment", JavaScript, true, false),
        ] {
            let parse = handler(text, dialect);
            assert_eq!(
                (
                    matches!(parse, Parse::HandlerMember),
                    matches!(parse, Parse::HandlerFunction)
                ),
                (member, function),
                "{text}"
            );
        }
    }
}
