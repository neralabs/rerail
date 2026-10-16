//! `parse`: a rule over the tree a sub-grammar parses from a string literal, and `capture`, which
//! binds a node of that tree.

use super::ast_rule::{
    AST_RELATIONS, StopSpec, compile_ast_rule, relation_rule, validate_ast_rule,
};
use super::compare::{count_bounds, parse_comparison};
use super::order::order_by_captures;
use super::resolve::{compile_resolves, validate_resolves};
use super::utils::{UtilIds, UtilScope};
use super::value::{compile_value_with, validate_value};
use crate::code_rule_dsl::json::{bounded_text, invalid, non_empty_str, object};
use crate::code_rule_dsl::language::adapter::{SyntaxFeature, SyntaxRules, gate, with_adapter};
use crate::code_rule_dsl::language::kind_ids;
use crate::code_rule_dsl::language::{Family, GrammarDialect};
use crate::code_rule_dsl::literal::{Grammar, LiteralHost};
use crate::code_rule_dsl::matcher::Semantic;
use crate::code_rule_dsl::matcher::{
    AstMatcher, CaptureAnalysis, CompareText, Count, HostTest, ParseTest, Reading, Related,
    SemanticText, StopBy, Transform, parse_exported, table_reading,
};
use crate::code_rule_dsl::model::Limits;
use crate::code_rule_dsl::soundness::sound_under_errors;
use ast_grep_core::matcher::KindMatcher;
use napi::Result;
use regex::Regex;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::sync::Arc;

/// Keys a rule inside `parse` may use, whatever the grammar.
const SUB_RULE_KEYS: &[&str] = &[
    "kind", "regex", "all", "any", "not", "inside", "has", "precedes", "follows", "count",
    "compare", "value", "capture",
];

/// Keys a rule over a parsed annotation, whose nodes are Python's, may use besides.
const ANNOTATION_KEYS: &[&str] = &["pattern", "matches", "resolves", "parse"];

fn allowed(grammar: Grammar, key: &str) -> bool {
    SUB_RULE_KEYS.contains(&key)
        || (grammar == Grammar::Annotation && ANNOTATION_KEYS.contains(&key))
        // A text grammar's tree reaches its host node through `host`; a directive's item names a
        // global `resolves` reads.
        || (grammar != Grammar::Annotation && key == "host")
        || (grammar == Grammar::EslintDirective && key == "resolves")
}

/// The grammars' names, as `parse.grammar` takes them.
fn grammar_names() -> Vec<&'static str> {
    Grammar::ALL.iter().map(|grammar| grammar.name()).collect()
}

/// Validates a `parse` object: its grammar and the rule of that grammar it holds.
pub(super) fn validate_parse(
    value: &Value,
    path: &str,
    limits: &Limits,
    utils: &UtilIds,
) -> Result<Grammar> {
    let parse = object(value, path)?;
    let name = non_empty_str(parse.get("grammar"), &format!("{path}/grammar"))?;
    let grammar = Grammar::named(name).ok_or_else(|| {
        let reason = if name == "ecmaRegex" {
            "is not supported yet".to_owned()
        } else {
            format!(
                "is not a grammar; expected one of {}",
                grammar_names().join(", ")
            )
        };
        invalid(format!("{path}/grammar: `{name}` {reason}"))
    })?;
    if parse.contains_key("flags") {
        return Err(invalid(format!(
            "{path}/flags: `{}` has no flags",
            grammar.name()
        )));
    }
    let rule = sub_rule(parse);
    validate_sub_rule(&rule, path, grammar, true, limits, utils)?;
    Ok(grammar)
}

/// The rule a `parse` object holds: every key but `grammar` and `flags`.
fn sub_rule(parse: &Map<String, Value>) -> Value {
    let mut rule = parse.clone();
    rule.remove("grammar");
    rule.remove("flags");
    Value::Object(rule)
}

/// `hosted`: the tree has a host node, which `host` tests (a `parse`'s literal; formatArgs' fields
/// have none).
pub(super) fn validate_sub_rule(
    value: &Value,
    path: &str,
    grammar: Grammar,
    hosted: bool,
    limits: &Limits,
    utils: &UtilIds,
) -> Result<()> {
    let rule = object(value, path)?;
    for key in rule.keys() {
        if key == "host" && !hosted && grammar != Grammar::Annotation {
            return Err(invalid(format!(
                "{path}/host: host tests the literal a parse holds; formatArgs' fields have none"
            )));
        }
        if !allowed(grammar, key) {
            let known = super::ast_rule::is_rule_key(key) || key == "host";
            return Err(invalid(if known {
                format!(
                    "{path}/{key}: `{key}` is not supported inside parse ({} trees)",
                    grammar.name()
                )
            } else {
                format!("unknown field `{key}` at {path}")
            }));
        }
    }
    if rule.is_empty() {
        return Err(invalid(format!(
            "{path} must contain at least one matcher of the parsed tree"
        )));
    }
    if let Some(kind) = rule.get("kind") {
        let kind = non_empty_str(Some(kind), &format!("{path}/kind"))?;
        // An annotation's kinds are Python's, which its rule's compilation checks.
        if grammar != Grammar::Annotation && !grammar.kinds().contains(&kind) {
            return Err(invalid(format!(
                "{path}/kind: `{kind}` is not a {} kind; expected one of {}",
                grammar.name(),
                grammar.kinds().join(", ")
            )));
        }
    }
    if let Some(regex) = rule.get("regex") {
        let source = bounded_text(
            Some(regex),
            &format!("{path}/regex"),
            "maxRegexPatternBytes",
            limits.max_regex_pattern_bytes,
        )?;
        Regex::new(source)
            .map_err(|_| invalid(format!("{path}/regex: Parsing text matcher fails.")))?;
    }
    if let Some(capture) = rule.get("capture") {
        capture_name(capture, &format!("{path}/capture"))?;
    }
    if let Some(value) = rule.get("value") {
        validate_value(value, &format!("{path}/value"), limits)?;
        if value.get("follow").is_some() {
            return Err(invalid(format!(
                "{path}/value/follow: a parsed node has no binding to follow"
            )));
        }
    }
    if let Some(compare) = rule.get("compare") {
        let comparison = parse_comparison(compare, &format!("{path}/compare"))?;
        if grammar != Grammar::Annotation
            && !matches!(
                comparison.text,
                CompareText::Source
                    | CompareText::Unquoted
                    | CompareText::Semantic(SemanticText::Value)
                    | CompareText::Node
            )
        {
            return Err(invalid(format!(
                "{path}/compare/text: a parsed tree compares by source, unquoted, value or node"
            )));
        }
    }
    // An annotation's patterns, utilities and names are checked as the host's are.
    for key in ["pattern", "matches"] {
        if let Some(value) = rule.get(key) {
            let mut single = Map::new();
            single.insert(key.to_owned(), value.clone());
            validate_ast_rule(&Value::Object(single), path, limits, utils)?;
        }
    }
    if let Some(host) = rule.get("host") {
        validate_ast_rule(host, &format!("{path}/host"), limits, utils)?;
    }
    if let Some(resolves) = rule.get("resolves") {
        validate_resolves(resolves, &format!("{path}/resolves"))?;
        if grammar == Grammar::EslintDirective
            && let Some(other) = object(resolves, &format!("{path}/resolves"))?
                .keys()
                .find(|key| *key != "global")
        {
            return Err(invalid(format!(
                "{path}/resolves/{other}: a directive's node resolves by `global` only: its text as \
                 a declared global of the query's languageOptions"
            )));
        }
        if resolves.get("binding").is_some() {
            return Err(invalid(format!(
                "{path}/resolves/binding: a parsed annotation's names are read, but their bindings' nodes are the host's"
            )));
        }
    }
    if let Some(parse) = rule.get("parse") {
        let nested = validate_parse(parse, &format!("{path}/parse"), limits, utils)?;
        if nested != Grammar::Annotation {
            return Err(invalid(format!(
                "{path}/parse/grammar: a string inside an annotation parses as an annotation"
            )));
        }
    }
    for key in ["all", "any"] {
        if let Some(items) = rule.get(key) {
            let items = items
                .as_array()
                .ok_or_else(|| invalid(format!("{path}/{key} must be an array")))?;
            if items.is_empty() {
                return Err(invalid(format!("{path}/{key} must not be empty")));
            }
            for (index, item) in items.iter().enumerate() {
                validate_sub_rule(
                    item,
                    &format!("{path}/{key}/{index}"),
                    grammar,
                    hosted,
                    limits,
                    utils,
                )?;
            }
        }
    }
    if let Some(not) = rule.get("not") {
        validate_sub_rule(not, &format!("{path}/not"), grammar, hosted, limits, utils)?;
    }
    for (key, _) in AST_RELATIONS {
        if let Some(relation) = rule.get(key) {
            validate_sub_relation(
                relation,
                &format!("{path}/{key}"),
                grammar,
                hosted,
                limits,
                utils,
            )?;
        }
    }
    if let Some(count) = rule.get("count") {
        let count_path = format!("{path}/count");
        let spec = count_bounds(count, &count_path)?;
        if spec.binding.is_some() {
            return Err(invalid(format!(
                "{count_path}/as: a parsed tree's count binds nothing; count in the ast query's \
                 own rule to report its value"
            )));
        }
        validate_sub_relation(
            spec.relation,
            &format!("{count_path}/{}", spec.key),
            grammar,
            hosted,
            limits,
            utils,
        )?;
    }
    Ok(())
}

fn validate_sub_relation(
    relation: &Value,
    path: &str,
    grammar: Grammar,
    hosted: bool,
    limits: &Limits,
    utils: &UtilIds,
) -> Result<()> {
    let (stop, field, nested) = relation_rule(relation, path)?;
    if field.is_some() {
        return Err(invalid(format!(
            "{path}/field: field is not supported in a parsed tree's rule"
        )));
    }
    if let StopSpec::Rule(stop) = stop {
        validate_sub_rule(
            stop,
            &format!("{path}/stopBy"),
            grammar,
            hosted,
            limits,
            utils,
        )?;
    }
    validate_sub_rule(&nested, path, grammar, hosted, limits, utils)
}

/// A capture's name: a metavariable name without `$`.
pub(super) fn capture_name<'a>(value: &'a Value, path: &str) -> Result<&'a str> {
    let name = non_empty_str(Some(value), path)?;
    let valid = |c: char| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_';
    if !name.chars().all(valid) {
        return Err(invalid(format!(
            "{path}: `{name}` is not a capture name (A-Z, 0-9, _, without `$`)"
        )));
    }
    Ok(name)
}

/// Compiles a validated `parse` for one host grammar.
pub(super) fn compile_parse(
    value: &Value,
    language: GrammarDialect,
    path: &str,
    gaps: &mut Vec<String>,
    utils: &UtilScope,
) -> Result<AstMatcher> {
    let parse = object(value, path)?;
    let name = parse
        .get("grammar")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let grammar = Grammar::named(name)
        .ok_or_else(|| invalid(format!("{path}/grammar: `{name}` is not a grammar")))?;
    let family = Family::of(language);
    gate(
        &format!("{path}/grammar"),
        with_adapter!(family, A => <A as SyntaxRules>::feature(SyntaxFeature::Parse(grammar))),
    )?;
    let rule = if grammar == Grammar::Annotation {
        // An annotation's nodes are Python's: its rule compiles as the host's does.
        compile_ast_rule(&sub_rule(parse), grammar.host_language(), path, gaps, utils)?
    } else {
        let host = Hosting { language, utils };
        compile_sub_rule(
            &sub_rule(parse),
            grammar,
            path,
            utils.transform.as_ref(),
            Some(&host),
            gaps,
        )?
    };
    if matches!(rule, AstMatcher::Never) {
        return Ok(AstMatcher::Never);
    }
    let mut exports = BTreeSet::new();
    parse_exported(&rule, &mut exports);
    let mut hosts = Vec::new();
    host_rules(&rule, false, &mut hosts);
    Ok(AstMatcher::Semantic(Semantic::Parse(Box::new(ParseTest {
        grammar,
        host_sound: (!hosts.is_empty()).then(|| {
            hosts
                .iter()
                .all(|(rule, negated)| !negated && sound_under_errors(rule))
        }),
        rule,
        exports: exports.into_iter().collect(),
        host_kinds: Arc::new(match grammar.literal_host() {
            LiteralHost::PythonString => kind_ids(language, &["string", "concatenated_string"]),
            LiteralHost::EcmaComment => kind_ids(language, &["comment"]),
            LiteralHost::PhpString => kind_ids(
                language,
                &["string", "encapsed_string", "heredoc", "nowdoc"],
            ),
            LiteralHost::PhpComment => kind_ids(language, &["comment"]),
        }),
        reading: table_reading(utils.options.as_deref(), utils.python.as_deref()),
    }))))
}

/// The host a parsed tree's rule reaches: the language and scope its `host` rules compile with,
/// and the options an item's `resolves` reads.
pub(super) struct Hosting<'s, 'u> {
    pub(super) language: GrammarDialect,
    pub(super) utils: &'s UtilScope<'u>,
}

/// The `host` rules of a parsed tree's rule, each with whether a negation or a count holds it
/// (whose verdict is then an absence claim over the host).
fn host_rules<'r>(matcher: &'r AstMatcher, negated: bool, found: &mut Vec<(&'r AstMatcher, bool)>) {
    match matcher {
        AstMatcher::Semantic(Semantic::Host(test)) => found.push((&test.rule, negated)),
        AstMatcher::All(items) | AstMatcher::Any(items) => {
            for item in items {
                host_rules(item, negated, found);
            }
        }
        AstMatcher::Not(item) => host_rules(item, true, found),
        AstMatcher::Count(count) => host_rules(&count.relation, true, found),
        AstMatcher::Inside(item, stop)
        | AstMatcher::Has(item, stop)
        | AstMatcher::Precedes(item, stop)
        | AstMatcher::Follows(item, stop) => {
            host_rules(item, negated, found);
            if let StopBy::Rule(stop) = stop {
                host_rules(stop, true, found);
            }
        }
        _ => {}
    }
}

/// Compiles a validated rule of a text grammar. `host` is where it reaches its host node (`None`
/// for formatArgs' fields, which have none).
pub(super) fn compile_sub_rule(
    value: &Value,
    grammar: Grammar,
    path: &str,
    transform: Option<&Arc<Transform>>,
    host: Option<&Hosting>,
    gaps: &mut Vec<String>,
) -> Result<AstMatcher> {
    let rule = object(value, path)?;
    let mut terms = Vec::new();
    let lang = grammar.lang();
    if let Some(kind) = rule.get("kind").and_then(Value::as_str) {
        terms.push(AstMatcher::Kind(
            KindMatcher::try_new(kind, lang)
                .map_err(|error| invalid(format!("{path}/kind: {error}")))?,
        ));
    }
    if let Some(regex) = rule.get("regex").and_then(Value::as_str) {
        terms.push(AstMatcher::Regex(Regex::new(regex).map_err(|_| {
            invalid(format!("{path}/regex: Parsing text matcher fails."))
        })?));
    }
    if let Some(capture) = rule.get("capture") {
        terms.push(AstMatcher::Semantic(Semantic::Capture(
            capture_name(capture, &format!("{path}/capture"))?.to_owned(),
        )));
    }
    // Composites, the negation and relations may read captures, and all but `not` may bind them:
    // among them, one that reads what a sibling binds runs after it (`order_by_captures`), as a
    // host rule's do; otherwise they keep this order.
    let from = terms.len();
    let mut group_paths = Vec::new();
    for key in ["all", "any"] {
        let Some(items) = rule.get(key).and_then(Value::as_array) else {
            continue;
        };
        group_paths.push(format!("{path}/{key}"));
        let mut compiled = items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                compile_sub_rule(
                    item,
                    grammar,
                    &format!("{path}/{key}/{index}"),
                    transform,
                    host,
                    gaps,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        if key == "all" {
            let mut ordered = compiled
                .drain(..)
                .enumerate()
                .map(|(index, matcher)| (matcher, format!("{path}/{key}/{index}")))
                .collect::<Vec<_>>();
            order_by_captures(&mut ordered, 0, path, &mut CaptureAnalysis::default())?;
            compiled = ordered.into_iter().map(|(matcher, _)| matcher).collect();
        }
        terms.push(if key == "all" {
            AstMatcher::All(compiled)
        } else {
            AstMatcher::Any(compiled)
        });
    }
    if let Some(not) = rule.get("not") {
        terms.push(AstMatcher::Not(Box::new(compile_sub_rule(
            not,
            grammar,
            &format!("{path}/not"),
            transform,
            host,
            gaps,
        )?)));
        group_paths.push(format!("{path}/not"));
    }
    for (key, constructor) in AST_RELATIONS {
        if let Some(relation) = rule.get(key) {
            let (inner, stop) = compile_sub_relation(
                relation,
                grammar,
                &format!("{path}/{key}"),
                transform,
                host,
                gaps,
            )?;
            terms.push(constructor(Related::new(inner), stop));
            group_paths.push(format!("{path}/{key}"));
        }
    }
    if group_paths.len() > 1 {
        let mut paired = terms
            .drain(..)
            .zip(std::iter::repeat_n(String::new(), from).chain(group_paths))
            .collect::<Vec<_>>();
        order_by_captures(&mut paired, from, path, &mut CaptureAnalysis::default())?;
        terms = paired.into_iter().map(|(term, _)| term).collect();
    }
    if let Some(count) = rule.get("count") {
        let count_path = format!("{path}/count");
        let spec = count_bounds(count, &count_path)?;
        let (inner, stop) = compile_sub_relation(
            spec.relation,
            grammar,
            &format!("{count_path}/{}", spec.key),
            transform,
            host,
            gaps,
        )?;
        let constructor = AST_RELATIONS
            .iter()
            .find_map(|(name, constructor)| (*name == spec.key).then_some(*constructor))
            .expect("count_bounds returns a relation key");
        terms.push(AstMatcher::Count(Box::new(Count {
            relation: constructor(Related::new(inner), stop),
            min: spec.min,
            max: spec.max,
            binding: None,
        })));
    }
    if let Some(compare) = rule.get("compare") {
        let mut comparison = parse_comparison(compare, &format!("{path}/compare"))?;
        if comparison.text == CompareText::Semantic(SemanticText::Value) {
            comparison.reading = Some(Reading::literal(grammar.host_language()));
        }
        // A transform variable is text: it compares as `source` or `unquoted` text only.
        if let Some(transform) = transform.filter(|transform| {
            transform.contains(&comparison.left) || transform.contains(&comparison.right)
        }) {
            if !matches!(comparison.text, CompareText::Source | CompareText::Unquoted) {
                return Err(invalid(format!(
                    "{path}/compare/text: a transform variable is text; compare it as source or unquoted"
                )));
            }
            comparison.transform = Some(Arc::clone(transform));
        }
        terms.push(AstMatcher::Compare(Box::new(comparison)));
    }
    if let Some(value) = rule.get("value") {
        terms.push(compile_value_with(
            value,
            grammar.host_language(),
            &format!("{path}/value"),
            Reading::literal(grammar.host_language()),
        )?);
    }
    // A directive's node names a global: its text, read as the query's `languageOptions`
    // declare globals (`matcher::Resolves::item`).
    if let Some(resolves) = rule.get("resolves") {
        let options = host.and_then(|host| host.utils.options.as_ref());
        let mut compiled = compile_resolves(
            resolves,
            grammar.host_language(),
            &format!("{path}/resolves"),
            options,
            None,
        )?;
        if let AstMatcher::Semantic(Semantic::Resolves(resolves)) = &mut compiled {
            resolves.item = true;
        }
        terms.push(compiled);
    }
    // Last: the host rule reads the captures the terms before it bind.
    if let Some(rule) = rule.get("host") {
        let host = host.ok_or_else(|| {
            invalid(format!(
                "{path}/host: host tests the literal a parse holds; formatArgs' fields have none"
            ))
        })?;
        let compiled = compile_ast_rule(
            rule,
            host.language,
            &format!("{path}/host"),
            gaps,
            host.utils,
        )?;
        terms.push(AstMatcher::Semantic(Semantic::Host(Box::new(HostTest {
            rule: compiled,
        }))));
    }
    match terms.len() {
        0 => Err(invalid(format!(
            "{path} must contain at least one matcher of the parsed tree"
        ))),
        1 => Ok(terms.pop().expect("one term")),
        _ => Ok(AstMatcher::All(terms)),
    }
}

fn compile_sub_relation(
    value: &Value,
    grammar: Grammar,
    path: &str,
    transform: Option<&Arc<Transform>>,
    host: Option<&Hosting>,
    gaps: &mut Vec<String>,
) -> Result<(AstMatcher, StopBy)> {
    let (stop, _, nested) = relation_rule(value, path)?;
    let stop = match stop {
        StopSpec::Neighbor => StopBy::Neighbor,
        StopSpec::End => StopBy::End,
        StopSpec::Rule(stop) => StopBy::Rule(Box::new(compile_sub_rule(
            stop,
            grammar,
            &format!("{path}/stopBy"),
            transform,
            host,
            gaps,
        )?)),
    };
    Ok((
        compile_sub_rule(&nested, grammar, path, transform, host, gaps)?,
        stop,
    ))
}

#[cfg(test)]
mod annotation_tests;
#[cfg(test)]
mod host_tests;
#[cfg(test)]
mod m12b_tests;
#[cfg(test)]
mod ruff;
#[cfg(test)]
mod tests;
