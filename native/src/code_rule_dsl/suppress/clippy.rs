//! Rustc and Clippy lint-level attributes (`allow`, `expect`, `warn`, `deny`, `forbid`).
//!
//! This is deliberately a syntax-level reader. Lint names are compared textually (after removing
//! whitespace), and are not resolved against the compiler's renamed or removed lint catalogue.

use super::names;
use crate::code_rule_dsl::json::{object, reject_unknown};
use ast_grep_core::{Doc, Node};
use serde_json::Value;

/// `suppress.clippy`: the lint names whose `allow` or `expect` silences the violation, as an
/// attribute spells them (`clippy::double_parens`, a group such as `clippy::complexity` or
/// `clippy::all`, or `warnings`). A port lists its lint and every group it belongs to.
pub(crate) type Clippy = Vec<String>;

/// Parses `suppress.clippy`: `{ "lints": [Name]+ }`.
pub(super) fn spec(clippy: &Value, at: &str) -> napi::Result<Clippy> {
    reject_unknown(object(clippy, at)?, &["lints"], at)?;
    names(
        clippy.get("lints"),
        &format!("{at}/lints"),
        lint_name,
        "lint names",
        "a lint name (`clippy::name`, `warnings`, or a rustc lint or group)",
    )
}

/// A lint path rustc reads in a lint attribute: one or two `::`-joined snake-case identifiers.
fn lint_name(name: &str) -> bool {
    let mut segments = name.split("::");
    let valid = |segment: &str| {
        !segment.is_empty()
            && segment
                .bytes()
                .all(|byte| byte == b'_' || byte.is_ascii_lowercase() || byte.is_ascii_digit())
    };
    match (segments.next(), segments.next(), segments.next()) {
        (Some(first), None, None) => valid(first),
        (Some(tool), Some(lint), None) => valid(tool) && valid(lint),
        _ => false,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Level {
    Allow,
    Expect,
    Warn,
    Deny,
    Forbid,
}

#[derive(Clone, Debug)]
struct Attribute {
    start: usize,
    end: usize,
    level: Level,
    names: Vec<String>,
    order: usize,
}

/// Lint-level attributes of one Rust file.
pub(crate) struct ClippyIndex {
    attributes: Vec<Attribute>,
}

impl ClippyIndex {
    /// Reads active Rust lint-level attributes from `root`.
        pub(crate) fn read<D: Doc>(root: &Node<'_, D>, source: &str) -> Self {
        let mut attributes = Vec::new();
        let mut order = 0;
        for node in root.dfs() {
            let kind = node.kind();
            if kind != "attribute_item" && kind != "inner_attribute_item" {
                continue;
            }
            let text = node.text();
            let parsed = parse_attribute(&text, source);
            if parsed.is_empty() {
                continue;
            }
            let span = if kind == "inner_attribute_item" {
                node.parent()
                    .map(|parent| parent.range())
                    .unwrap_or_else(|| node.range())
            } else {
                let end = node.range().end;
                node.parent()
                    .and_then(|parent| {
                        // An attribute the grammar makes a node's first child (a match arm's, a
                        // closure's) applies to that node; in a list of items, statements,
                        // fields or variants it applies to the next one.
                        let list = matches!(
                            parent.kind().as_ref(),
                            "source_file"
                                | "declaration_list"
                                | "block"
                                | "field_declaration_list"
                                | "ordered_field_declaration_list"
                                | "enum_variant_list"
                                | "match_block"
                                | "arguments"
                                | "parameters"
                                | "field_initializer_list"
                        );
                        if !list && parent.range().start == node.range().start {
                            Some(parent.range())
                        } else {
                            parent
                                .children()
                                .find(|sibling| {
                                    sibling.range().start >= end
                                        && sibling.is_named()
                                        && sibling.kind() != "attribute_item"
                                })
                                .map(|target| target.range())
                        }
                    })
                    .unwrap_or_else(|| node.range())
            };
            for (level, names) in parsed {
                attributes.push(Attribute {
                    start: span.start,
                    end: span.end,
                    level,
                    names,
                    order,
                });
                order += 1;
            }
        }
        Self { attributes }
    }

    /// Whether an `allow`/`expect` naming one of `lints` is the level in force at byte `at`.
        pub(crate) fn suppresses(&self, lints: &[String], at: usize) -> bool {
        let matching = self.attributes.iter().filter(|attribute| {
            attribute.start <= at
                && at < attribute.end
                && attribute
                    .names
                    .iter()
                    .any(|name| lints.iter().any(|lint| lint == name))
        });
        let mut selected: Option<&Attribute> = None;
        for attribute in matching {
            if attribute.level == Level::Forbid {
                return false;
            }
            if selected.is_none_or(|old| {
                let old_width = old.end - old.start;
                let width = attribute.end - attribute.start;
                width < old_width || (width == old_width && attribute.order > old.order)
            }) {
                selected = Some(attribute);
            }
        }
        matches!(
            selected.map(|attribute| attribute.level),
            Some(Level::Allow | Level::Expect)
        )
    }
}

fn parse_attribute(text: &str, source: &str) -> Vec<(Level, Vec<String>)> {
    let text = text.trim();
    let inner = text.starts_with("#!");
    let Some(body) = text
        .strip_prefix(if inner { "#![" } else { "#[" })
        .and_then(|body| body.strip_suffix(']'))
    else {
        return Vec::new();
    };
    let body = body.trim();
    if body.starts_with("cfg_attr(") {
        return parse_cfg_attr(body, source);
    }
    parse_level(body).into_iter().collect()
}

fn parse_cfg_attr(body: &str, source: &str) -> Vec<(Level, Vec<String>)> {
    let Some(inside) = body
        .strip_prefix("cfg_attr(")
        .and_then(|v| v.strip_suffix(')'))
    else {
        return Vec::new();
    };
    let mut parts = split_top_level(inside);
    let Some(predicate) = parts.next().map(str::trim) else {
        return Vec::new();
    };
    if !cfg_enabled(predicate, source) {
        return Vec::new();
    }
    let mut result = Vec::new();
    for part in parts {
        let part = part.trim();
        if let Some(value) = parse_level(part) {
            result.push(value);
        } else if part.starts_with("cfg_attr(") {
            result.extend(parse_cfg_attr(part, source));
        }
    }
    result
}

fn parse_level(body: &str) -> Option<(Level, Vec<String>)> {
    let (name, rest) = body.split_once('(')?;
    let level = match name.trim() {
        "allow" => Level::Allow,
        "expect" => Level::Expect,
        "warn" => Level::Warn,
        "deny" => Level::Deny,
        "forbid" => Level::Forbid,
        _ => return None,
    };
    let list = rest.strip_suffix(')')?;
    let names = split_top_level(list)
        .map(|name| name.split_once('=').map_or(name, |(path, _)| path).trim())
        .filter(|name| !name.is_empty())
        .map(|name| {
            name.chars()
                .filter(|c| !c.is_whitespace())
                .collect::<String>()
        })
        .map(|name| match name.as_str() {
            "clippy::const_static_lifetime" => "clippy::redundant_static_lifetimes".to_owned(),
            "clippy::unused_label" => "unused_labels".to_owned(),
            _ => name,
        })
        .collect();
    Some((level, names))
}

fn split_top_level(text: &str) -> impl Iterator<Item = &str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    let mut quote = false;
    for (at, byte) in text.bytes().enumerate() {
        match byte {
            b'"' => quote = !quote,
            b'(' if !quote => depth += 1,
            b')' if !quote => depth -= 1,
            b',' if !quote && depth == 0 => {
                result.push(&text[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    result.push(&text[start..]);
    result.into_iter()
}

fn cfg_enabled(predicate: &str, source: &str) -> bool {
    let predicate = predicate.trim();
    if predicate == "all()" || predicate == "clippy" || predicate == "all(clippy)" {
        return true;
    }
    if predicate == "any()" || predicate == "not(clippy)" {
        return false;
    }
    if let Some(inner) = predicate
        .strip_prefix("all(")
        .and_then(|v| v.strip_suffix(')'))
    {
        return split_top_level(inner).all(|item| cfg_enabled(item, source));
    }
    if let Some(inner) = predicate
        .strip_prefix("any(")
        .and_then(|v| v.strip_suffix(')'))
    {
        return split_top_level(inner).any(|item| cfg_enabled(item, source));
    }
    if let Some(inner) = predicate
        .strip_prefix("not(")
        .and_then(|v| v.strip_suffix(')'))
    {
        return !cfg_enabled(inner, source);
    }
    let _ = source;
    false
}

#[cfg(test)]
mod tests {
    use super::ClippyIndex;
    use crate::code_rule_dsl::language::GrammarDialect;
    use ast_grep_language::LanguageExt;
    use serde::Deserialize;

    fn suppressed(source: &str, lint: &str, needle: &str) -> bool {
        let at = source.find(needle).expect("needle");
        let root = GrammarDialect::Rust.ast_grep(source);
        let index = ClippyIndex::read(&root.root(), source);
        index.suppresses(&[lint.to_owned()], at)
    }

    #[test]
    fn outer_allow_covers_item() {
        let source = "#[allow(clippy::double_parens)]\nfn f() { ((1)); }\n";
        assert!(suppressed(source, "clippy::double_parens", "((1))"));
    }

    #[test]
    fn later_level_on_same_item_wins() {
        let source =
            "#[allow(clippy::double_parens)]\n#[warn(clippy::double_parens)]\nfn f() { ((1)); }\n";
        assert!(!suppressed(source, "clippy::double_parens", "((1))"));
    }

    #[test]
    fn forbid_cannot_be_lowered() {
        let source = "#![forbid(clippy::double_parens)]\n#[allow(clippy::double_parens)]\nfn f() { ((1)); }\n";
        assert!(!suppressed(source, "clippy::double_parens", "((1))"));
    }

    #[test]
    fn cfg_attr_all_is_active_and_any_is_not() {
        let source = "#[cfg_attr(all(), allow(clippy::double_parens))]\nfn a() { ((1)); }\n#[cfg_attr(any(), allow(clippy::double_parens))]\nfn b() { ((2)); }\n";
        assert!(suppressed(source, "clippy::double_parens", "((1))"));
        assert!(!suppressed(source, "clippy::double_parens", "((2))"));
    }

    #[derive(Deserialize)]
    struct Oracle {
        source: Source,
        files: std::collections::BTreeMap<String, FileStatus>,
        cases: Vec<Case>,
    }

    #[derive(Deserialize)]
    struct Source {
        attribute_rules: Vec<String>,
    }

    #[derive(Deserialize)]
    struct FileStatus {
        #[allow(dead_code)]
        exit_force_warn: i32,
        #[allow(dead_code)]
        exit: i32,
    }

    #[derive(Deserialize)]
    struct Case {
        file: String,
        rule: String,
        line: usize,
        column: usize,
        level: String,
    }

    fn lints_of(rule: &str) -> Vec<String> {
        let names = match rule {
            "clippy::double_parens" => [rule, "clippy::complexity", "clippy::all", "warnings"],
            "clippy::needless_else" | "clippy::redundant_static_lifetimes" => {
                [rule, "clippy::style", "clippy::all", "warnings"]
            }
            "clippy::unreadable_literal" => [rule, "clippy::pedantic", "warnings", ""],
            "unused_variables" | "unused_labels" => [rule, "unused", "warnings", ""],
            _ => ["", "", "", ""],
        };
        names
            .into_iter()
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .collect()
    }

    fn byte_offset(source: &str, line: usize, column: usize) -> usize {
        let line_start = source
            .split_inclusive('\n')
            .take(line.saturating_sub(1))
            .map(str::len)
            .sum();
        line_start
            + source[line_start..]
                .chars()
                .take(column.saturating_sub(1))
                .map(char::len_utf8)
                .sum::<usize>()
    }

    #[test]
    fn clippy_lint_levels_agree_with_clippy_driver() {
        // Sites Clippy suppresses that a syntax-level reader cannot see suppressed.
        const DIVERGENCES: &[(&str, &str)] = &[
            (
                "fixtures/expression.rs:6:8 clippy::double_parens",
                "tree-sitter-rust 0.24.2 parses no attribute on a closure expression (an ERROR \
                 node); rustc rejects it too (E0658), yet still applies it",
            ),
            (
                "fixtures/macros.rs:23:13 clippy::double_parens",
                "the attribute is written inside a macro_rules! body: it applies where the macro \
                 expands",
            ),
            (
                "fixtures/macros.rs:81:19 clippy::double_parens",
                "the attribute is written inside a macro_rules! body: it applies where the macro \
                 expands",
            ),
        ];
        let oracle: Oracle = serde_json::from_str(crate::code_rule_dsl::oracle!(
            "rust/suppress/fixtures.expected.json"
        ))
        .expect("fixture oracle JSON");
        assert_eq!(oracle.files.len(), 10);
        assert!(oracle.cases.len() >= 100);

        let mut mismatches = Vec::new();
        for case in &oracle.cases {
            if case.rule.starts_with('E')
                && case.rule[1..]
                    .chars()
                    .all(|character| character.is_ascii_digit())
            {
                continue;
            }
            if oracle
                .source
                .attribute_rules
                .iter()
                .any(|rule| rule == &case.rule)
            {
                continue;
            }
            let source_path = case.file.strip_prefix("fixtures/").expect("fixture path");
            let source = match source_path {
                "argument.rs" => {
                    crate::code_rule_dsl::oracle!("rust/suppress/fixtures/argument.rs")
                }
                "cfg_attr.rs" => {
                    crate::code_rule_dsl::oracle!("rust/suppress/fixtures/cfg_attr.rs")
                }
                "expect.rs" => crate::code_rule_dsl::oracle!("rust/suppress/fixtures/expect.rs"),
                "expression.rs" => {
                    crate::code_rule_dsl::oracle!("rust/suppress/fixtures/expression.rs")
                }
                "forbid.rs" => crate::code_rule_dsl::oracle!("rust/suppress/fixtures/forbid.rs"),
                "groups.rs" => crate::code_rule_dsl::oracle!("rust/suppress/fixtures/groups.rs"),
                "macros.rs" => crate::code_rule_dsl::oracle!("rust/suppress/fixtures/macros.rs"),
                "names.rs" => crate::code_rule_dsl::oracle!("rust/suppress/fixtures/names.rs"),
                "override.rs" => {
                    crate::code_rule_dsl::oracle!("rust/suppress/fixtures/override.rs")
                }
                "placement.rs" => {
                    crate::code_rule_dsl::oracle!("rust/suppress/fixtures/placement.rs")
                }
                _ => panic!("unknown fixture {source_path}"),
            };
            let root = GrammarDialect::Rust.ast_grep(source);
            let index = ClippyIndex::read(&root.root(), source);
            let at = byte_offset(source, case.line, case.column);
            let got = index.suppresses(&lints_of(&case.rule), at);
            let expected = case.level == "allow";
            if got != expected {
                mismatches.push(format!(
                    "{}:{}:{} {} {} {}",
                    case.file, case.line, case.column, case.rule, expected, got
                ));
            }
        }
        let expected = DIVERGENCES
            .iter()
            .map(|(site, _reason)| format!("{site} true false"))
            .collect::<Vec<_>>();
        assert_eq!(mismatches, expected);
    }
}
