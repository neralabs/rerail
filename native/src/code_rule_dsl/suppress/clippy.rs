//! Rustc and Clippy lint-level attributes (`allow`, `expect`, `warn`, `deny`, `forbid`), and
//! Clippy's `#[clippy::msrv]`.
//!
//! This is deliberately a syntax-level reader. Lint names are compared textually (after removing
//! whitespace), and are not resolved against the compiler's renamed or removed lint catalogue.

use super::names;
use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
use ast_grep_core::{Doc, Node};
use serde_json::Value;

/// `suppress.clippy`: what silences the violation as Clippy silences its lint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Clippy {
    /// The lint names whose `allow` or `expect` silences it, as an attribute spells them
    /// (`clippy::double_parens`, a group such as `clippy::complexity` or `clippy::all`, or
    /// `warnings`). A port lists its lint and every group it belongs to.
    pub(crate) lints: Vec<String>,
    /// The Rust version the lint needs (Clippy's `msrvs::*`): the lint is off where the
    /// `#[clippy::msrv = ".."]` in force names an older one.
    pub(crate) msrv: Option<RustVersion>,
}

/// A Rust version, `major.minor.patch`; `1.53` is `1.53.0`.
pub(crate) type RustVersion = (u32, u32, u32);

/// Parses `suppress.clippy`: `{ "lints": [Name]+, "msrv"?: "1.53" }`.
pub(super) fn spec(clippy: &Value, at: &str) -> napi::Result<Clippy> {
    reject_unknown(object(clippy, at)?, &["lints", "msrv"], at)?;
    let lints = names(
        clippy.get("lints"),
        &format!("{at}/lints"),
        lint_name,
        "lint names",
        "a lint name (`clippy::name`, `warnings`, or a rustc lint or group)",
    )?;
    let msrv = clippy
        .get("msrv")
        .map(|msrv| {
            msrv.as_str().and_then(rust_version).ok_or_else(|| {
                invalid(format!(
                    "{at}/msrv must be a Rust version, `major.minor` or `major.minor.patch`"
                ))
            })
        })
        .transpose()?;
    Ok(Clippy { lints, msrv })
}

/// `1.53` or `1.53.0`, as `#[clippy::msrv]` and Clippy's `msrv` option spell a version.
fn rust_version(text: &str) -> Option<RustVersion> {
    let mut parts = text.split('.').map(|part| {
        (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| part.parse::<u32>().ok())
            .flatten()
    });
    let major = parts.next()??;
    let minor = parts.next()??;
    let patch = parts.next().unwrap_or(Some(0))?;
    parts.next().is_none().then_some((major, minor, patch))
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

/// What one attribute sets: a lint level for some lints, or Clippy's msrv.
#[derive(Clone, Debug)]
enum Setting {
    Level(Level, Vec<String>),
    Msrv(RustVersion),
}

#[derive(Clone, Debug)]
struct Attribute {
    start: usize,
    end: usize,
    setting: Setting,
    order: usize,
}

impl Attribute {
    /// Whether `self` is in force over `other` where both cover a byte: the narrower one, or the
    /// later of two on the same item.
    fn overrides(&self, other: &Self) -> bool {
        let (width, other_width) = (self.end - self.start, other.end - other.start);
        width < other_width || (width == other_width && self.order > other.order)
    }
}

/// Lint-level and `#[clippy::msrv]` attributes of one Rust file.
pub(crate) struct ClippyIndex {
    attributes: Vec<Attribute>,
}

impl ClippyIndex {
    /// Reads active Rust lint-level and `#[clippy::msrv]` attributes from `root`.
    pub(crate) fn read<D: Doc>(root: &Node<'_, D>) -> Self {
        let mut attributes = Vec::new();
        let mut order = 0;
        for node in root.dfs() {
            let kind = node.kind();
            if kind != "attribute_item" && kind != "inner_attribute_item" {
                continue;
            }
            let text = node.text();
            let parsed = parse_attribute(&text);
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
                        // fields or variants it applies to the next one, and to that one's other
                        // attributes (rustc sets an item's levels before checking its attributes).
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
                            let mut start = node.range().start;
                            let mut previous = node.prev();
                            while let Some(sibling) = previous {
                                match sibling.kind().as_ref() {
                                    "attribute_item" => start = sibling.range().start,
                                    "line_comment" | "block_comment" => {}
                                    _ => break,
                                }
                                previous = sibling.prev();
                            }
                            parent
                                .children()
                                .find(|sibling| {
                                    sibling.range().start >= end
                                        && sibling.is_named()
                                        && !matches!(
                                            sibling.kind().as_ref(),
                                            "attribute_item" | "line_comment" | "block_comment"
                                        )
                                })
                                .map(|target| start..target.range().end)
                        }
                    })
                    .unwrap_or_else(|| node.range())
            };
            for setting in parsed {
                attributes.push(Attribute {
                    start: span.start,
                    end: span.end,
                    setting,
                    order,
                });
                order += 1;
            }
        }
        Self { attributes }
    }

    /// Whether Clippy is silent at byte `at`: an `allow`/`expect` naming one of `clippy.lints` is
    /// the level in force there, or the `#[clippy::msrv]` in force names a Rust older than
    /// `clippy.msrv`.
    pub(crate) fn suppresses(&self, clippy: &Clippy, at: usize) -> bool {
        self.allowed(&clippy.lints, at)
            || clippy
                .msrv
                .is_some_and(|needed| self.msrv(at).is_some_and(|msrv| msrv < needed))
    }

    /// Whether an `allow`/`expect` naming one of `lints` is the level in force at byte `at`.
    fn allowed(&self, lints: &[String], at: usize) -> bool {
        let mut selected: Option<(&Attribute, Level)> = None;
        for attribute in self.covering(at) {
            let Setting::Level(level, names) = &attribute.setting else {
                continue;
            };
            if !names.iter().any(|name| lints.contains(name)) {
                continue;
            }
            if *level == Level::Forbid {
                return false;
            }
            if selected.is_none_or(|(old, _)| attribute.overrides(old)) {
                selected = Some((attribute, *level));
            }
        }
        matches!(selected, Some((_, Level::Allow | Level::Expect)))
    }

    /// The version the `#[clippy::msrv]` in force at byte `at` names, if any.
    fn msrv(&self, at: usize) -> Option<RustVersion> {
        let mut selected: Option<(&Attribute, RustVersion)> = None;
        for attribute in self.covering(at) {
            if let Setting::Msrv(version) = attribute.setting
                && selected.is_none_or(|(old, _)| attribute.overrides(old))
            {
                selected = Some((attribute, version));
            }
        }
        selected.map(|(_, version)| version)
    }

    fn covering(&self, at: usize) -> impl Iterator<Item = &Attribute> {
        self.attributes
            .iter()
            .filter(move |attribute| attribute.start <= at && at < attribute.end)
    }
}

fn parse_attribute(text: &str) -> Vec<Setting> {
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
        return parse_cfg_attr(body);
    }
    parse_setting(body).into_iter().collect()
}

fn parse_cfg_attr(body: &str) -> Vec<Setting> {
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
    if !cfg_enabled(predicate) {
        return Vec::new();
    }
    let mut result = Vec::new();
    for part in parts {
        let part = part.trim();
        if let Some(value) = parse_setting(part) {
            result.push(value);
        } else if part.starts_with("cfg_attr(") {
            result.extend(parse_cfg_attr(part));
        }
    }
    result
}

/// A lint level (`allow(..)`) or `clippy::msrv = ".."`; Clippy ignores an msrv it cannot parse.
fn parse_setting(body: &str) -> Option<Setting> {
    if let Some((path, value)) = body.split_once('=')
        && path
            .chars()
            .filter(|c| !c.is_whitespace())
            .eq("clippy::msrv".chars())
    {
        return value
            .trim()
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .and_then(rust_version)
            .map(Setting::Msrv);
    }
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
    Some(Setting::Level(level, names))
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

fn cfg_enabled(predicate: &str) -> bool {
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
        return split_top_level(inner).all(cfg_enabled);
    }
    if let Some(inner) = predicate
        .strip_prefix("any(")
        .and_then(|v| v.strip_suffix(')'))
    {
        return split_top_level(inner).any(cfg_enabled);
    }
    if let Some(inner) = predicate
        .strip_prefix("not(")
        .and_then(|v| v.strip_suffix(')'))
    {
        return !cfg_enabled(inner);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{Clippy, ClippyIndex, RustVersion, rust_version};
    use crate::code_rule_dsl::language::GrammarDialect;
    use ast_grep_language::LanguageExt;
    use serde::Deserialize;

    fn silenced(source: &str, clippy: &Clippy, needle: &str) -> bool {
        let at = source.find(needle).expect("needle");
        let root = GrammarDialect::Rust.ast_grep(source);
        ClippyIndex::read(&root.root()).suppresses(clippy, at)
    }

    fn suppressed(source: &str, lint: &str, needle: &str) -> bool {
        let lints = vec![lint.to_owned()];
        silenced(source, &Clippy { lints, msrv: None }, needle)
    }

    #[test]
    fn an_older_msrv_in_force_silences_a_lint_that_needs_a_newer_rust() {
        let source = "#[clippy::msrv = \"1.52\"]\nfn a() { ((1)); }\n#[clippy::msrv = \"1.53.0\"]\nfn b() { ((2)); }\nfn c() { ((3)); }\n#[cfg_attr(all(), clippy::msrv = \"1.1\")]\nfn d() { ((4)); }\n#[clippy :: msrv = \"x\"]\nfn e() { ((5)); }\nmod m {\n    #![clippy::msrv = \"1.40\"]\n    fn f() { ((6)); }\n}\n";
        let needs = |msrv: Option<RustVersion>| Clippy {
            lints: vec!["clippy::double_parens".to_owned()],
            msrv,
        };
        assert!(silenced(source, &needs(Some((1, 53, 0))), "((1))"));
        assert!(!silenced(source, &needs(None), "((1))"));
        assert!(!silenced(source, &needs(Some((1, 52, 0))), "((1))"));
        assert!(!silenced(source, &needs(Some((1, 53, 0))), "((2))"));
        assert!(!silenced(source, &needs(Some((1, 53, 0))), "((3))"));
        assert!(silenced(source, &needs(Some((1, 2, 0))), "((4))"));
        assert!(!silenced(source, &needs(Some((1, 53, 0))), "((5))"));
        assert!(silenced(source, &needs(Some((1, 53, 0))), "((6))"));
        assert_eq!(rust_version("1.53"), Some((1, 53, 0)));
        assert_eq!(rust_version("1.53.2"), Some((1, 53, 2)));
        for bad in ["1", "1.", "1.x", "1.2.3.4", " 1.2", "+1.2"] {
            assert_eq!(rust_version(bad), None, "{bad}");
        }
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

    #[test]
    fn an_items_levels_cover_its_own_attributes() {
        let source = "#[expect(clippy::allow_attributes)] // why\n#[allow(unused)]\nstruct A;\n#[allow(unused)]\n#[expect(clippy::allow_attributes)]\nstruct B;\n#[allow(dead_code)]\nstruct C;\n";
        assert!(suppressed(
            source,
            "clippy::allow_attributes",
            "allow(unused)]\nstruct A"
        ));
        assert!(suppressed(
            source,
            "clippy::allow_attributes",
            "allow(unused)]\n#[expect"
        ));
        assert!(!suppressed(
            source,
            "clippy::allow_attributes",
            "allow(dead_code)"
        ));
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
            let index = ClippyIndex::read(&root.root());
            let at = byte_offset(source, case.line, case.column);
            let clippy = Clippy {
                lints: lints_of(&case.rule),
                msrv: None,
            };
            let got = index.suppresses(&clippy, at);
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
