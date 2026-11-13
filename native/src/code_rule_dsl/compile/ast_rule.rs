pub mod semantic;

use super::compare::{count_bounds, parse_comparison};
use super::dialect::grammar_gap;
use super::documents::Side;
use super::nth_child::{compile_nth_child, validate_nth_child};
use super::order::{all_order_conflicts, order_by_captures};
use super::utils::{UtilIds, UtilScope, is_metavariable_name};
use crate::code_rule_dsl::json::{
    bounded_text, invalid, non_empty_str, object, reject_unknown, required_str,
};
use crate::code_rule_dsl::language::{GrammarDialect, PatternSource, rule_pattern};
use crate::code_rule_dsl::matcher::{
    AstMatcher, CallArgs, CompareText, Count, EmbeddedTest, FieldRelation, HostTest, Reading,
    Related, Semantic, StopBy, UtilCall, UtilRef, parse_exported,
};
use crate::code_rule_dsl::model::Limits;
use ast_grep_core::Language;
use ast_grep_core::matcher::{KindMatcher, Pattern, PatternNode};
use napi::Result;
use regex::Regex;
use serde_json::Value;
use std::collections::BTreeSet;
use crate::code_rule_dsl::language::adapter::pack as language_pack;

#[cfg(not(any(test, feature = "test-hooks")))]
const AST_RULE_KEYS: &[&str] = &[
    "pattern",
    "kind",
    "regex",
    "nthChild",
    "all",
    "any",
    "not",
    "matches",
    "inside",
    "has",
    "precedes",
    "follows",
    "compare",
    "count",
    "resolves",
    "value",
    "arguments",
    "binds",
    "parse",
    "capture",
    "formatArgs",
    "layout",
    "filePath",
    "calls",
    "embedded",
    "host",
    "importTarget",
    "stored",
];
/// Tests add `testUnknown`, a leaf whose verdict is unknown where its regex matches the node's text.
#[cfg(any(test, feature = "test-hooks"))]
const AST_RULE_KEYS: &[&str] = &[
    "pattern",
    "kind",
    "regex",
    "nthChild",
    "all",
    "any",
    "not",
    "matches",
    "inside",
    "has",
    "precedes",
    "follows",
    "compare",
    "count",
    "resolves",
    "value",
    "arguments",
    "binds",
    "parse",
    "capture",
    "formatArgs",
    "layout",
    "filePath",
    "calls",
    "embedded",
    "host",
    "importTarget",
    "stored",
    "testUnknown",
];

/// Whether `key` is a rule key somewhere, for messages that tell misplaced keys from unknown ones.
pub(super) fn is_rule_key(key: &str) -> bool {
    AST_RULE_KEYS.contains(&key)
}

type RelationConstructor = fn(Box<Related>, StopBy) -> AstMatcher;

pub const AST_RELATIONS: [(&str, RelationConstructor); 4] = [
    ("inside", AstMatcher::Inside),
    ("has", AstMatcher::Has),
    ("precedes", AstMatcher::Precedes),
    ("follows", AstMatcher::Follows),
];

/// Every check an ast rule must pass regardless of grammar: field names, value types, byte limits,
/// and regex syntax. `compile_ast_rule` relies on these having passed.
pub fn validate_ast_rule(
    value: &Value,
    path: &str,
    limits: &Limits,
    utils: &UtilIds,
) -> Result<()> {
    let rule = object(value, path)?;
    reject_unknown(rule, AST_RULE_KEYS, path)?;
    if !AST_RULE_KEYS.iter().any(|key| rule.contains_key(*key)) {
        return Err(invalid(format!(
            "{path} must contain at least one ast-grep matcher"
        )));
    }
    if let Some(pattern) = rule.get("pattern") {
        let pattern_path = format!("{path}/pattern");
        if pattern.is_string() {
            bounded_text(
                Some(pattern),
                &pattern_path,
                "maxAstPatternBytes",
                limits.max_ast_pattern_bytes,
            )?;
        } else {
            let contextual = pattern.as_object().ok_or_else(|| {
                invalid(format!(
                    "{pattern_path} must be a string or a {{ context, selector }} object"
                ))
            })?;
            reject_unknown(contextual, &["context", "selector"], &pattern_path)?;
            bounded_text(
                contextual.get("context"),
                &format!("{pattern_path}/context"),
                "maxAstPatternBytes",
                limits.max_ast_pattern_bytes,
            )?;
            non_empty_str(
                contextual.get("selector"),
                &format!("{pattern_path}/selector"),
            )?;
        }
    }
    if let Some(kind) = rule.get("kind") {
        non_empty_str(Some(kind), &format!("{path}/kind"))?;
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
    // Why the token a `regex` tests is stored where the rule looks (`compile::token_tests`).
    if let Some(stored) = rule.get("stored") {
        non_empty_str(Some(stored), &format!("{path}/stored"))?;
        if !rule.contains_key("regex") {
            return Err(invalid(format!("{path}/stored says why a token a regex tests is stored: it needs a regex")));
        }
    }
    if let Some(nth) = rule.get("nthChild") {
        validate_nth_child(nth, &format!("{path}/nthChild"), limits, utils)?;
    }
    for (key, _) in AST_RELATIONS {
        if let Some(relation) = rule.get(key) {
            let relation_path = format!("{path}/{key}");
            let field = validate_relation(relation, &relation_path, limits, utils)?;
            // ast-grep-config 0.45.3 `Precedes`/`Follows`: `FieldNotSupported`.
            if field.is_some() && !matches!(key, "inside" | "has") {
                return Err(invalid(format!(
                    "{relation_path}/field: field is only supported in has/inside"
                )));
            }
        }
    }
    if let Some(count) = rule.get("count") {
        let count_path = format!("{path}/count");
        let spec = count_bounds(count, &count_path)?;
        let relation_path = format!("{count_path}/{}", spec.key);
        if validate_relation(spec.relation, &relation_path, limits, utils)?.is_some() {
            return Err(invalid(format!(
                "{relation_path}/field: field is not supported in count"
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
                validate_ast_rule(item, &format!("{path}/{key}/{index}"), limits, utils)?;
            }
        }
    }
    if let Some(not) = rule.get("not") {
        validate_ast_rule(not, &format!("{path}/not"), limits, utils)?;
    }
    // Where `embedded` and `host` may stand is `documents::check_sides`'s.
    for key in ["embedded", "host"] {
        if let Some(inner) = rule.get(key) {
            validate_ast_rule(inner, &format!("{path}/{key}"), limits, utils)?;
        }
    }
    if let Some(compare) = rule.get("compare") {
        parse_comparison(compare, &format!("{path}/compare"))?;
    }
    semantic::validate(rule, path, limits, utils)?;
    if let Some(matches) = rule.get("matches") {
        call_arguments(matches, &format!("{path}/matches"), utils)?;
    }
    Ok(())
}

/// A `matches` value's utility and, for a parameterized call, each parameter with the caller's
/// capture it names (without `$`), in the order the utility lists its parameters.
pub(super) fn call_arguments<'v>(
    matches: &'v Value,
    path: &str,
    utils: &UtilIds,
) -> Result<(&'v str, Option<CallArgs>)> {
    let undefined = |id: &str| {
        invalid(format!(
            "{path}: utility rule `{id}` is not defined in this query's utils or a define of its language"
        ))
    };
    let (id, arguments) = match matches {
        Value::String(id) => {
            let params = utils.get(id.as_str()).ok_or_else(|| undefined(id))?;
            if !params.is_empty() {
                return Err(invalid(format!(
                    "{path}: utility rule `{id}` takes parameters ({}); call it as {{ \"{id}\": {{ \"{}\": \"$NAME\" }} }}",
                    params.join(", "),
                    params[0]
                )));
            }
            return Ok((id, None));
        }
        Value::Object(call) if call.len() == 1 => {
            let (id, arguments) = call.iter().next().expect("one entry");
            (id.as_str(), arguments)
        }
        Value::Object(_) => {
            return Err(invalid(format!(
                "{path} must call one utility: {{ \"id\": {{ \"PARAM\": \"$CAPTURE\" }} }}"
            )));
        }
        _ => {
            return Err(invalid(format!("{path} must be the id of a utility rule")));
        }
    };
    let params = utils.get(id).ok_or_else(|| undefined(id))?;
    if params.is_empty() {
        return Err(invalid(format!(
            "{path}: utility rule `{id}` takes no parameters; name it (\"matches\": \"{id}\")"
        )));
    }
    let call_path = format!("{path}/{id}");
    let arguments = object(arguments, &call_path)?;
    for key in arguments.keys() {
        if !params.contains(key) {
            return Err(invalid(format!(
                "{call_path}/{key}: `{key}` is not a parameter of `{id}` ({})",
                params.join(", ")
            )));
        }
    }
    let mut mapped = Vec::with_capacity(params.len());
    for param in params.iter() {
        let argument = arguments.get(param).ok_or_else(|| {
            invalid(format!(
                "{call_path}: parameter `{param}` is not passed; map it to a capture (\"{param}\": \"$NAME\")"
            ))
        })?;
        let capture = argument
            .as_str()
            .and_then(|text| text.strip_prefix('$'))
            .filter(|name| is_metavariable_name(name))
            .ok_or_else(|| {
                invalid(format!(
                    "{call_path}/{param} must name a capture: `$NAME` (A-Z, 0-9, _)"
                ))
            })?;
        mapped.push((param.clone(), capture.to_owned()));
    }
    Ok((id, Some(mapped)))
}

/// Validates a relation's stop rule and sub-rule; returns its `field`, which the caller admits
/// where upstream does.
fn validate_relation<'a>(
    relation: &'a Value,
    path: &str,
    limits: &Limits,
    utils: &UtilIds,
) -> Result<Option<&'a str>> {
    let (stop, field, nested) = relation_rule(relation, path)?;
    if let StopSpec::Rule(stop) = stop {
        validate_ast_rule(stop, &format!("{path}/stopBy"), limits, utils)?;
    }
    validate_ast_rule(&nested, path, limits, utils)?;
    Ok(field)
}

/// Whether `kind` resolves to a grammar supertype (`expression`, `statement`, ...), which tree-sitter
/// never assigns to a node.
fn is_supertype(kind: &str, language: GrammarDialect) -> bool {
    language.kind_is_supertype(language.kind_to_id(kind))
}

/// In Python and PHP patterns, a leaf that ast-grep's preprocessing turned into the expando
/// character but that is not a metavariable (`$At` becomes the identifier `µAt`), spelled back
/// with `$`. `source` is the pattern as written: a leaf spelled with the expando there (`µfoo`, a
/// valid identifier) was not rewritten.
fn literal_dollar_name(
    pattern: &Pattern,
    source: &str,
    language: GrammarDialect,
) -> Option<String> {
    /// Whether `source` holds `spelling` as a whole name: no further `$` before it, and no
    /// identifier character after it.
    fn written(source: &str, spelling: &str) -> bool {
        source.match_indices(spelling).any(|(at, _)| {
            !source[..at].ends_with('$')
                && !source[at + spelling.len()..]
                    .starts_with(|c: char| c.is_alphanumeric() || c == '_')
        })
    }
    fn find(node: &PatternNode, expando: char, source: &str) -> Option<String> {
        match node {
            PatternNode::Terminal { text, .. } => {
                let name = text.trim_start_matches(expando);
                let sigils = (text.len() - name.len()) / expando.len_utf8();
                if sigils == 0 || name.is_empty() {
                    return None;
                }
                let spelling = format!("{}{name}", "$".repeat(sigils));
                written(source, &spelling).then_some(spelling)
            }
            PatternNode::Internal { children, .. } => children
                .iter()
                .find_map(|child| find(child, expando, source)),
            PatternNode::MetaVar { .. } => None,
        }
    }
    
    { let pack = language_pack::pack(crate::code_rule_dsl::language::Family::of(language)); pack.syntax_dollar_names() }
    .then(|| find(&pattern.node, language.expando_char(), source))
    .flatten()
}

/// A relation's `stopBy` as written: a rule object is compiled per grammar like any sub-rule.
pub enum StopSpec<'a> {
    Neighbor,
    End,
    Rule(&'a Value),
}

/// A relation's `stopBy`, its `field` and the sub-rule it applies to.
pub fn relation_rule<'a>(
    value: &'a Value,
    path: &str,
) -> Result<(StopSpec<'a>, Option<&'a str>, Value)> {
    let relation = object(value, path)?;
    let stop = match relation.get("stopBy") {
        None => StopSpec::Neighbor,
        Some(Value::String(stop)) => match stop.as_str() {
            "neighbor" => StopSpec::Neighbor,
            "end" => StopSpec::End,
            other => {
                return Err(invalid(format!(
                    "{path}/stopBy has unsupported value `{other}`; expected \"neighbor\", \"end\" or a rule object"
                )));
            }
        },
        Some(rule @ Value::Object(_)) => StopSpec::Rule(rule),
        Some(_) => {
            return Err(invalid(format!(
                "{path}/stopBy must be \"neighbor\", \"end\" or a rule object"
            )));
        }
    };
    let field = relation
        .get("field")
        .map(|field| non_empty_str(Some(field), &format!("{path}/field")))
        .transpose()?;
    let mut nested = relation.clone();
    nested.remove("stopBy");
    nested.remove("field");
    Ok((stop, field, Value::Object(nested)))
}

/// Compiles a `validate_ast_rule`-checked rule for one grammar. A pattern or kind the grammar
/// cannot express becomes `Never` and its reason is pushed to `gaps`; `Never` then propagates
/// the way its logic dictates (drops an `any` branch, empties an `all`, satisfies a `not`).
pub fn compile_ast_rule(
    value: &Value,
    language: GrammarDialect,
    path: &str,
    gaps: &mut Vec<String>,
    utils: &UtilScope,
) -> Result<AstMatcher> {
    let rule = object(value, path)?;
    // ast-grep-config's order: atomic, then composite, then relational terms. Bindings thread
    // through the terms in this order, so it decides which node a relation binds first.
    let mut terms = Vec::new();
    if let Some(pattern) = rule.get("pattern") {
        let (written, source) = if let Some(source) = pattern.as_str() {
            (source, PatternSource::Plain(source))
        } else {
            let context = required_str(pattern, "context", &format!("{path}/pattern"))?;
            (
                context,
                PatternSource::Contextual {
                    context,
                    selector: required_str(pattern, "selector", &format!("{path}/pattern"))?,
                },
            )
        };
        terms.push(match rule_pattern(source, language) {
            Ok(compiled) if !compiled.has_error() => {
                match literal_dollar_name(&compiled, written, language) {
                    Some(name) => grammar_gap(
                        gaps,
                        format!(
                            "{path}/pattern: `{name}` is not a metavariable (names use A-Z, 0-9 and _) \
                             and Python source has no `$`, so the pattern never matches"
                        ),
                    ),
                    None => AstMatcher::Pattern(compiled),
                }
            }
            Ok(_) => grammar_gap(
                gaps,
                format!("{path}/pattern parse error: pattern contains an error node"),
            ),
            Err(error) => grammar_gap(gaps, format!("{path}/pattern parse error: {error}")),
        });
    }
    if let Some(kind) = rule.get("kind").and_then(Value::as_str) {
        terms.push(match KindMatcher::try_new(kind, language) {
            // A supertype names a symbol no node carries, so ast-grep's kind test never matches
            // it: it matches the nodes the parser reported as its members (the tape's).
            Ok(_) if is_supertype(kind, language) => AstMatcher::Supertype(crate::code_rule_dsl::matcher::Supertype::new(language.kind_to_id(kind), language)),
            Ok(matcher) => match language.kinds().kind_ids(kind).as_slice() {
                [_] => AstMatcher::Kind(matcher),
                ids => AstMatcher::Any(
                    ids.iter()
                        .copied()
                        .map(KindMatcher::from_id)
                        .map(AstMatcher::Kind)
                        .collect(),
                ),
            },
            Err(error) => grammar_gap(gaps, format!("{path}/kind: {error}")),
        });
    }
    if let Some(regex) = rule.get("regex").and_then(Value::as_str) {
        terms.push(AstMatcher::Regex(Regex::new(regex).map_err(|_| {
            invalid(format!("{path}/regex: Parsing text matcher fails."))
        })?));
    }
    if let Some(capture) = rule.get("capture") {
        terms.push(
            semantic::compile_key(
                "capture",
                capture,
                semantic::Stage::Atomic,
                language,
                &format!("{path}/capture"),
                gaps,
                utils,
            )
            .expect("capture is semantic")?,
        );
    }
    // A parsed tree binds from the node alone: an atomic term.
    if let Some(parse) = rule.get("parse") {
        terms.push(
            semantic::compile_key(
                "parse",
                parse,
                semantic::Stage::Atomic,
                language,
                &format!("{path}/parse"),
                gaps,
                utils,
            )
            .expect("parse is semantic")?,
        );
    }
    // ast-grep-config's last atomic term: the node's position among its siblings. With `ofRule`
    // it binds and may read captures, so it leads the ordered group below.
    let nth_child = rule.get("nthChild");
    let of_rule = nth_child.is_some_and(|nth| nth.get("ofRule").is_some());
    if let Some(nth) = nth_child.filter(|_| !of_rule) {
        terms.push(compile_nth_child(
            nth,
            language,
            &format!("{path}/nthChild"),
            gaps,
            utils,
        )?);
    }
    // Arguments bind like a pattern, from the node alone: an atomic term.
    if let Some(arguments) = rule.get("arguments") {
        terms.push(
            semantic::compile_key(
                "arguments",
                arguments,
                semantic::Stage::Atomic,
                language,
                &format!("{path}/arguments"),
                gaps,
                utils,
            )
            .expect("arguments is semantic")?,
        );
    }
    if let Some(condition) = rule.get("filePath") {
        terms.push(
            semantic::compile_key(
                "filePath",
                condition,
                semantic::Stage::Atomic,
                language,
                &format!("{path}/filePath"),
                gaps,
                utils,
            )
            .expect("filePath is semantic")?,
        );
    }
    // Composites, negations, utilities, relations and binding terms may read captures, and all but
    // `not` may bind them: among them, one that reads what a sibling binds runs after it
    // (`order_by_captures`); otherwise they keep this order.
    let from = terms.len();
    let mut group_paths = Vec::new();
    let mut has_all = false;
    if let Some(nth) = nth_child.filter(|_| of_rule) {
        let nth_path = format!("{path}/nthChild");
        terms.push(compile_nth_child(nth, language, &nth_path, gaps, utils)?);
        group_paths.push(nth_path);
    }
    for key in ["all", "any"] {
        let Some(items) = rule.get(key).and_then(Value::as_array) else {
            continue;
        };
        group_paths.push(format!("{path}/{key}"));
        let mut compiled = items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                compile_ast_rule(
                    item,
                    language,
                    &format!("{path}/{key}/{index}"),
                    gaps,
                    utils,
                )
            })
            .collect::<Result<Vec<_>>>()?;
        let never = |item: &AstMatcher| matches!(item, AstMatcher::Never);
        if key == "all" {
            has_all = true;
        }
        terms.push(if key == "all" {
            if compiled.iter().any(never) {
                AstMatcher::Never
            } else {
                AstMatcher::All(compiled)
            }
        } else {
            compiled.retain(|item| !never(item));
            if compiled.is_empty() {
                AstMatcher::Never
            } else {
                AstMatcher::Any(compiled)
            }
        });
    }
    if let Some(not) = rule.get("not") {
        terms.push(AstMatcher::Not(Box::new(compile_ast_rule(
            not,
            language,
            &format!("{path}/not"),
            gaps,
            utils,
        )?)));
        group_paths.push(format!("{path}/not"));
    }
    if let Some(matches) = rule.get("matches") {
        let matches_path = format!("{path}/matches");
        let (id, arguments) = call_arguments(matches, &matches_path, utils.ids)?;
        let index = utils
            .ids
            .keys()
            .position(|known| *known == id)
            .ok_or_else(|| {
                invalid(format!(
                    "{matches_path}: utility rule `{id}` is not defined"
                ))
            })?;
        let util = UtilRef {
            table: utils.table.clone(),
            index,
        };
        terms.push(match arguments {
            None => AstMatcher::Util(util),
            Some(args) => {
                utils.instance(index, &args, &matches_path)?;
                AstMatcher::Call(Box::new(UtilCall { util, args }))
            }
        });
        group_paths.push(matches_path);
    }
    for (key, constructor) in AST_RELATIONS {
        if let Some(relation) = rule.get(key) {
            let relation_path = format!("{path}/{key}");
            let compiled = compile_relation(relation, language, &relation_path, gaps, utils)?;
            // A bare token regex whose token the tape may leave out is also asked of the gaps (t31).
            let past = match &compiled.1 {
                StopBy::Rule(rule) => match rule.as_ref() {
                    AstMatcher::Not(inner) => Some(inner.clone()),
                    _ => None,
                },
                _ => None,
            };
            let gap = gap_test(key, relation, language, past);
            // A token the tape may leave out on the way to the next stored child (`gap_split`).
            let split = match gap {
                None => gap_split(key, relation, language),
                Some(_) => None,
            };
            let term = with_gap(
                match compiled {
                    (AstMatcher::Never, _, _) => AstMatcher::Never,
                    (inner, stop, None) => constructor(Related::new(inner), stop),
                    // ast-grep-config's `field_name_to_id`: a name the grammar lacks is refused.
                    (inner, stop, Some(name)) => match language.field_to_id(name) {
                        Some(field) => AstMatcher::Field(Box::new(FieldRelation {
                            inside: key == "inside",
                            field,
                            rule: inner,
                            stop,
                            filter: std::sync::OnceLock::new(),
                        })),
                        None => grammar_gap(
                            gaps,
                            format!(
                                "{relation_path}/field: `{name}` is not a field of this grammar"
                            ),
                        ),
                    },
                },
                gap,
            );
            let term = match split {
                None => term,
                Some((test, nested)) => {
                    let (inner, stop, _) = compile_relation(&nested, language, &format!("{relation_path}/{key}"), gaps, utils)?;
                    let walk = AstMatcher::All(vec![
                        AstMatcher::Semantic(crate::code_rule_dsl::matcher::Semantic::Gap(Box::new(test))),
                        constructor(Related::new(inner), stop),
                    ]);
                    match term {
                        AstMatcher::Never => walk,
                        term => AstMatcher::Any(vec![term, walk]),
                    }
                }
            };
            terms.push(term);
            group_paths.push(relation_path);
        }
    }
    // A rule over the file's other document (`embed`): from a host node, its embedded program's
    // nodes standing for bytes the node spans; from a program's node, the host's node spanning the
    // bytes it stands for. Each compiles in that document's grammar, with its utilities.
    if let Some(inner) = rule.get("embedded") {
        let embedded_path = format!("{path}/embedded");
        let documents = utils.documents.ok_or_else(|| {
            invalid(format!(
                "{embedded_path}: this query's language embeds no program"
            ))
        })?;
        let mut rules = Vec::new();
        // The rule must be whole in some dialect the program may be in, as a query's must be in
        // one of its own (`compile_query`): a gap left in every one (a misspelt kind compiles to
        // `Never`, which a `not` reads as true) is the query's.
        let mut whole = false;
        let mut first_gap = None;
        for (dialect, table) in &documents.guests {
            let scope = utils.on(table.clone(), Side::Guest);
            let mut dialect_gaps = Vec::new();
            let compiled =
                compile_ast_rule(inner, *dialect, &embedded_path, &mut dialect_gaps, &scope)?;
            match dialect_gaps.into_iter().next() {
                None => whole = true,
                Some(gap) => {
                    first_gap.get_or_insert(gap);
                }
            }
            if !matches!(compiled, AstMatcher::Never) {
                rules.push((*dialect, compiled));
            }
        }
        if !whole && let Some(gap) = first_gap {
            gaps.push(gap);
        }
        terms.push(if rules.is_empty() {
            grammar_gap(
                gaps,
                format!(
                    "{embedded_path}: the rule matches in none of the embedded program's grammars"
                ),
            )
        } else {
            let mut exports = BTreeSet::new();
            for (_, rule) in &rules {
                parse_exported(rule, &mut exports);
            }
            AstMatcher::Semantic(Semantic::Embedded(Box::new(EmbeddedTest {
                via: documents.via,
                rules,
                exports: exports.into_iter().collect(),
            })))
        });
        group_paths.push(embedded_path);
    }
    if let Some(inner) = rule.get("host") {
        let host_path = format!("{path}/host");
        let documents = utils.documents.ok_or_else(|| {
            invalid(format!(
                "{host_path}: host tests the literal a parse holds; use it inside parse"
            ))
        })?;
        let (dialect, table) = &documents.host;
        let scope = utils.on(table.clone(), Side::Host);
        terms.push(
            match compile_ast_rule(inner, *dialect, &host_path, gaps, &scope)? {
                AstMatcher::Never => AstMatcher::Never,
                compiled => {
                    AstMatcher::Semantic(Semantic::Host(Box::new(HostTest { rule: compiled })))
                }
            },
        );
        group_paths.push(host_path);
    }
    // A binding's sub-rules may bind, like a relation's; comparisons after it read them.
    if let Some(resolves) = rule.get("resolves")
        && let Some(compiled) = semantic::compile_key(
            "resolves",
            resolves,
            semantic::Stage::Ordered,
            language,
            &format!("{path}/resolves/binding"),
            gaps,
            utils,
        )
    {
        terms.push(compiled?);
        group_paths.push(format!("{path}/resolves/binding"));
    }
    // A binding's name node: its sub-rules may bind too.
    if let Some(binds) = rule.get("binds") {
        terms.push(
            semantic::compile_key(
                "binds",
                binds,
                semantic::Stage::Ordered,
                language,
                &format!("{path}/binds"),
                gaps,
                utils,
            )
            .expect("binds is semantic")?,
        );
        group_paths.push(format!("{path}/binds"));
    }
    // Format arguments read the string and argument captures; their `unused` rule may bind, and
    // `missing` exports what the template's fields capture.
    if let Some(format_args) = rule.get("formatArgs") {
        terms.push(
            semantic::compile_key(
                "formatArgs",
                format_args,
                semantic::Stage::Ordered,
                language,
                &format!("{path}/formatArgs"),
                gaps,
                utils,
            )
            .expect("formatArgs is semantic")?,
        );
        group_paths.push(format!("{path}/formatArgs"));
    }
    if group_paths.len() > 1 {
        let mut paired = terms
            .drain(..)
            .zip(std::iter::repeat_n(String::new(), from).chain(group_paths))
            .collect::<Vec<_>>();
        order_by_captures(&mut paired, from, path, &mut utils.captures.borrow_mut())?;
        terms = paired.into_iter().map(|(term, _)| term).collect();
    }
    // An explicit `all` keeps its written order: none of its items may read what only a later one
    // binds, where the terms that run before it (in capture order) do not bind it either.
    if has_all
        && let Some(index) = terms[from..]
            .iter()
            .position(|term| matches!(term, AstMatcher::All(_)))
        && let AstMatcher::All(items) = &terms[from + index]
    {
        let mut captures = utils.captures.borrow_mut();
        let context = terms[..from + index]
            .iter()
            .flat_map(|term| captures.binds(term))
            .collect();
        for conflict in all_order_conflicts(items, &context, path, &mut captures) {
            match utils.conflicts {
                Some(conflicts) => {
                    conflicts.borrow_mut().insert(conflict);
                }
                None => return Err(conflict.error()),
            }
        }
    }
    // Counts and comparisons bind nothing other terms read and come last, so they read what every
    // other term bound. A count rule this grammar cannot express counts no node; whether that
    // passes is up to `min`.
    if let Some(count) = rule.get("count") {
        let count_path = format!("{path}/count");
        let spec = count_bounds(count, &count_path)?;
        let relation_path = format!("{count_path}/{}", spec.key);
        let (inner, stop, _) =
            compile_relation(spec.relation, language, &relation_path, gaps, utils)?;
        let constructor = AST_RELATIONS
            .iter()
            .find_map(|(name, constructor)| (*name == spec.key).then_some(*constructor))
            .expect("count_bounds returns a relation key");
        // Implied tokens a bare token regex counts are counted in the gaps (t31).
        let past = match &stop {
            StopBy::Rule(rule) => match rule.as_ref() {
                AstMatcher::Not(inner) => Some(inner.clone()),
                _ => None,
            },
            _ => None,
        };
        let gap = gap_test(spec.key, spec.relation, language, past);
        terms.push(AstMatcher::Count(Box::new(Count {
            relation: constructor(Related::new(inner), stop),
            min: spec.min,
            max: spec.max,
            binding: spec.binding,
            gap,
        })));
    }
    if let Some(compare) = rule.get("compare") {
        let compare_path = format!("{path}/compare");
        let mut comparison = parse_comparison(compare, &compare_path)?;
        if let CompareText::Semantic(text) = comparison.text {
            semantic::compare_reading(
                &mut comparison,
                text,
                compare,
                &compare_path,
                language,
                utils,
            )?;
        }
        // A tape leaves out the tokens its grammar implies; `tokens` reads them back from it.
        if comparison.text == CompareText::Tokens {
            comparison.implied = crate::code_rule_dsl::tape::tokens::TokenGrammar::of_dialect(language);
        }
        // A transform variable is text: it compares as `source` or `unquoted` text only.
        if let Some(transform) = utils.transform.as_ref().filter(|transform| {
            transform.contains(&comparison.left) || transform.contains(&comparison.right)
        }) {
            if !matches!(comparison.text, CompareText::Source | CompareText::Unquoted) {
                return Err(invalid(format!(
                    "{compare_path}/text: a transform variable is text; compare it as source or unquoted"
                )));
            }
            comparison.transform = Some(transform.clone());
        }
        terms.push(AstMatcher::Compare(Box::new(comparison)));
    }
    // A layout binds nothing and may read a capture, like a comparison.
    if let Some(layout) = rule.get("layout") {
        terms.push(
            semantic::compile_key(
                "layout",
                layout,
                semantic::Stage::Late,
                language,
                &format!("{path}/layout"),
                gaps,
                utils,
            )
            .expect("layout is semantic")?,
        );
    }
    // A value reads only the node, so it binds nothing and may run after the terms that bind.
    if let Some(value) = rule.get("value") {
        terms.push(
            semantic::compile_key(
                "value",
                value,
                semantic::Stage::Late,
                language,
                &format!("{path}/value"),
                gaps,
                utils,
            )
            .expect("value is semantic")?,
        );
    }
    #[cfg(any(test, feature = "test-hooks"))]
    if let Some(regex) = rule.get("testUnknown").and_then(Value::as_str) {
        terms.push(AstMatcher::TestUnknown(Regex::new(regex).map_err(
            |_| invalid(format!("{path}/testUnknown: Parsing text matcher fails.")),
        )?));
    }
    // Name resolution binds and reads nothing, so it may run last, after every cheaper term.
    if let Some(resolves) = rule.get("resolves")
        && let Some(compiled) = semantic::compile_key(
            "resolves",
            resolves,
            semantic::Stage::Late,
            language,
            &format!("{path}/resolves"),
            gaps,
            utils,
        )
    {
        terms.push(compiled?);
    }
    // An import target reads the node's value and the files around it, and binds nothing.
    if let Some(target) = rule.get("importTarget") {
        terms.push(
            semantic::compile_key(
                "importTarget",
                target,
                semantic::Stage::Late,
                language,
                &format!("{path}/importTarget"),
                gaps,
                utils,
            )
            .expect("importTarget is semantic")?,
        );
    }
    // A call's reach reads the workspace closure: the costliest term, and it binds nothing.
    if let Some(calls) = rule.get("calls") {
        terms.push(
            semantic::compile_key(
                "calls",
                calls,
                semantic::Stage::Late,
                language,
                &format!("{path}/calls"),
                gaps,
                utils,
            )
            .expect("calls is semantic")?,
        );
    }
    if terms.iter().any(|term| matches!(term, AstMatcher::Never)) {
        return Ok(AstMatcher::Never);
    }
    match terms.len() {
        0 => Err(invalid(format!(
            "{path} must contain at least one ast-grep matcher"
        ))),
        1 => Ok(terms.pop().expect("one term")),
        _ => Ok(AstMatcher::All(terms)),
    }
}

/// A relation's sub-rule, walk and field name, compiled for one grammar. A stop rule this grammar
/// cannot express never matches, so the walk never stops early; its gap still counts toward "whole
/// in some dialect".
fn compile_relation<'a>(
    relation: &'a Value,
    language: GrammarDialect,
    path: &str,
    gaps: &mut Vec<String>,
    utils: &UtilScope,
) -> Result<(AstMatcher, StopBy, Option<&'a str>)> {
    let (stop, field, nested) = relation_rule(relation, path)?;
    let stop = match stop {
        StopSpec::Neighbor => StopBy::Neighbor,
        StopSpec::End => StopBy::End,
        StopSpec::Rule(stop) => StopBy::Rule(Box::new(compile_ast_rule(
            stop,
            language,
            &format!("{path}/stopBy"),
            gaps,
            utils,
        )?)),
    };
    Ok((
        compile_ast_rule(&nested, language, path, gaps, utils)?,
        stop,
        field,
    ))
}

// tests: a test the engine hosts (it reads above this crate; tools/cratify/test-hosts.tsv)

/// The gap test a relation to a bare token regex also needs (t31): `None` unless the target is
/// a bare regex (or `any` alternatives of one) that matches a token the dialect's grammar implies
/// somewhere, and the relation is `has` (a neighbour, or a field), or `follows`/`precedes`.
/// `has` to the end and `inside` are left as written; the guard refuses what they cannot reach.
pub(in crate::code_rule_dsl) fn gap_test(key: &str, relation: &Value, language: GrammarDialect, past: Option<Box<AstMatcher>>) -> Option<crate::code_rule_dsl::matcher::gap::GapTest> {
    use crate::code_rule_dsl::matcher::gap::{GapTest, Place};
    let object = relation.as_object()?;
    // `stopBy: {not: X}` walks past X; the test holds only where X names kinds and tokens.
    let past_extras = object.get("stopBy").and_then(Value::as_object).is_some_and(|stop| stop.len() == 1 && stop.contains_key("not"));
    let end = match object.get("stopBy").and_then(Value::as_str) {
        None if object.contains_key("stopBy") && !past_extras => return None,
        None | Some("neighbor") => false,
        Some("end") => true,
        Some(_) => return None,
    };
    let field = object.get("field").and_then(Value::as_str);
    let place = match (key, end, field) {
        ("has", false, Some(field)) => Place::Field(field.to_owned()),
        (_, _, Some(_)) => return None,
        ("has", false, None) => Place::Own,
        ("follows", false, None) => Place::Before,
        ("follows", true, None) => Place::Earlier,
        ("precedes", false, None) => Place::After,
        ("precedes", true, None) => Place::Later,
        _ => return None,
    };
    let grammar = crate::code_rule_dsl::tape::tokens::TokenGrammar::of_dialect(language)?;
    // A token regex, alone or beside a `not` that holds on every implied token: a `not` of kinds
    // none of which is the token's own (an implied `accessor` is no `property_identifier`), or of
    // an extra (no implied token is a comment).
    let bare = |rule: &Value| -> Option<String> {
        let rule = rule.as_object()?;
        let regex = rule.get("regex")?.as_str()?;
        let compiled = Regex::new(regex).ok()?;
        fn other_kinds(rule: &Value, token: &Regex) -> bool {
            let Some(rule) = rule.as_object().filter(|rule| rule.len() == 1) else { return false };
            match (rule.get("kind"), rule.get("any")) {
                (Some(kind), _) => kind.as_str().is_some_and(|kind| !token.is_match(kind)),
                (_, Some(any)) => any.as_array().is_some_and(|any| any.iter().all(|rule| other_kinds(rule, token))),
                _ => false,
            }
        }
        let extra = |rule: &Value| rule.as_object().filter(|rule| rule.len() == 1).and_then(|rule| rule.get("kind")?.as_str()).is_some_and(|kind| grammar.is_extra(kind));
        if !rule.iter().all(|(key, value)| key == "regex" || (key == "not" && (other_kinds(value, &compiled) || extra(value)))) {
            return None;
        }
        Some(format!("(?:{regex})"))
    };
    let mut target: serde_json::Map<String, Value> = object.iter().filter(|(key, _)| !matches!(key.as_str(), "stopBy" | "field")).map(|(k, v)| (k.clone(), v.clone())).collect();
    // The token's own relations to the tokens beside it (`not: {follows: {regex: "^,$"}}`): read
    // over the parent's children and implied words (`TokenCondition`).
    let mut token = Vec::new();
    for key in ["follows", "precedes", "not"] {
        let Some(value) = target.get(key) else { continue };
        let condition = match key {
            "not" => value.as_object().filter(|not| not.len() == 1).and_then(|not| not.iter().next()).and_then(|(key, value)| token_condition(key, value, true)),
            _ => token_condition(key, value, false),
        };
        if let Some(condition) = condition {
            token.push(condition);
            target.remove(key);
        }
    }
    // `{not: {kind: comment}}` beside the regex holds on every implied token: none is an extra.
    let not_extra = |rule: &Value| {
        rule.as_object().filter(|rule| rule.len() == 1).and_then(|rule| rule.get("not")?.as_object()).filter(|not| not.len() == 1).and_then(|not| not.get("kind")?.as_str()).is_some_and(|kind| grammar.is_extra(kind))
    };
    let regexes: Vec<String> = match bare(&Value::Object(target.clone())) {
        Some(regex) => vec![regex],
        None if token.is_empty() && target.len() == 1 && target.contains_key("any") => target.get("any")?.as_array()?.iter().filter_map(bare).collect(),
        None if token.is_empty() && target.len() == 1 && target.contains_key("all") => {
            let items = target.get("all")?.as_array()?;
            let regexes: Vec<String> = items.iter().filter_map(bare).collect();
            if regexes.len() != 1 || !items.iter().all(|item| bare(item).is_some() || not_extra(item)) {
                return None;
            }
            regexes
        }
        None => return None,
    };
    if regexes.is_empty() {
        return None;
    }
    let regex = Regex::new(&regexes.join("|")).ok()?;
    let tests = match &place {
        Place::Field(field) => grammar.tests_implied_in(&regex, field),
        _ => grammar.tests_implied(&regex),
    };
    if !tests {
        return None;
    }
    let past = if past_extras { Some(past?) } else { None };
    if past.is_some() && !matches!(place, Place::Before | Place::After) {
        return None;
    }
    // The walk may pass kinds (stored children) and tokens (by a bare regex), as the gaps can; one
    // whose stop rule says more is not a gap's question. A utility's own rule is checked as the
    // test runs (`GapTest::holds`).
    fn passable(rule: &Value) -> bool {
        let Some(rule) = rule.as_object().filter(|rule| rule.len() == 1) else { return false };
        match rule.iter().next() {
            Some((key, value)) if key == "kind" || key == "matches" => value.is_string(),
            Some((key, value)) if key == "regex" => value.as_str().is_some_and(|regex| Regex::new(regex).is_ok()),
            Some((key, value)) if key == "any" => value.as_array().is_some_and(|any| any.iter().all(passable)),
            _ => false,
        }
    }
    if past.is_some() && !object.get("stopBy").and_then(|stop| stop.get("not")).is_some_and(passable) {
        return None;
    }
    Some(GapTest { place, regex, grammar, dialect: language, past, token, stored: false })
}

/// A token's relation to a token regex beside it, under the same parent (`follows: {regex: "^,$",
/// stopBy: end}`), as a `TokenCondition`; `None` where it says more than the regex.
fn token_condition(key: &str, relation: &Value, negated: bool) -> Option<crate::code_rule_dsl::matcher::gap::TokenCondition> {
    use crate::code_rule_dsl::matcher::gap::{Place, TokenCondition};
    let object = relation.as_object()?;
    if !object.keys().all(|key| key == "regex" || key == "stopBy") {
        return None;
    }
    let place = match (key, object.get("stopBy").map(|stop| stop.as_str())) {
        ("follows", None | Some(Some("neighbor"))) => Place::Before,
        ("follows", Some(Some("end"))) => Place::Earlier,
        ("precedes", None | Some(Some("neighbor"))) => Place::After,
        ("precedes", Some(Some("end"))) => Place::Later,
        _ => return None,
    };
    Some(TokenCondition::Token { negated, place, regex: Regex::new(object.get("regex")?.as_str()?).ok()? })
}

/// A neighbour relation to a token the tape may leave out, which goes on to the next stored
/// child the same way (`precedes: {regex: "^=$", precedes: X}`): the gap test of the token,
/// with nothing between it and the child, and the relation's own walk to X, which on a tape
/// without the token is the node's neighbour. `None` unless the relation says that and no more.
pub(in crate::code_rule_dsl) fn gap_split(key: &str, relation: &Value, language: GrammarDialect) -> Option<(crate::code_rule_dsl::matcher::gap::GapTest, Value)> {
    use crate::code_rule_dsl::matcher::gap::{Place, TokenCondition};
    let object = relation.as_object()?;
    let place = match key {
        "follows" => Place::Before,
        "precedes" => Place::After,
        _ => return None,
    };
    if object.get("stopBy").is_some_and(|stop| stop != "neighbor") || object.contains_key("field") {
        return None;
    }
    let nested = object.get(key)?;
    if !object.keys().all(|name| matches!(name.as_str(), "regex" | "stopBy") || name == key) || token_condition(key, nested, false).is_some() {
        return None;
    }
    if nested.as_object()?.get("stopBy").is_some_and(|stop| stop != "neighbor") || nested.get("field").is_some() {
        return None;
    }
    let regex = Regex::new(&format!("(?:{})", object.get("regex")?.as_str()?)).ok()?;
    let grammar = crate::code_rule_dsl::tape::tokens::TokenGrammar::of_dialect(language)?;
    if !grammar.tests_implied(&regex) {
        return None;
    }
    let token = vec![TokenCondition::Stored(place.clone())];
    Some((crate::code_rule_dsl::matcher::gap::GapTest { place, regex, grammar, dialect: language, past: None, token, stored: false }, nested.clone()))
}

/// A relation's term, or it or the gap test.
fn with_gap(term: AstMatcher, gap: Option<crate::code_rule_dsl::matcher::gap::GapTest>) -> AstMatcher {
    use crate::code_rule_dsl::matcher::Semantic;
    match (term, gap) {
        (term, None) => term,
        (AstMatcher::Never, Some(gap)) => AstMatcher::Semantic(Semantic::Gap(Box::new(gap))),
        // A neighbour's own walk reads the next stored node, which on a full tree is the neighbour
        // only where no implied token stands between.
        (term, Some(gap)) if matches!(gap.place, crate::code_rule_dsl::matcher::gap::Place::Before | crate::code_rule_dsl::matcher::gap::Place::After) => {
            let stored = crate::code_rule_dsl::matcher::gap::GapTest { stored: true, ..gap.clone() };
            AstMatcher::Any(vec![AstMatcher::All(vec![term, AstMatcher::Semantic(Semantic::Gap(Box::new(stored)))]), AstMatcher::Semantic(Semantic::Gap(Box::new(gap)))])
        }
        (term, Some(gap)) => AstMatcher::Any(vec![term, AstMatcher::Semantic(Semantic::Gap(Box::new(gap)))]),
    }
}
