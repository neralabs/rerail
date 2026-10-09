mod semantic;

use super::compare::{count_bounds, parse_comparison};
use super::dialect::grammar_gap;
use super::nth_child::{compile_nth_child, validate_nth_child};
use super::order::{all_order_conflicts, order_by_captures};
use super::utils::{UtilIds, UtilScope, is_metavariable_name};
use crate::code_rule_dsl::json::{
    bounded_text, invalid, non_empty_str, object, reject_unknown, required_str,
};
use crate::code_rule_dsl::language::{GrammarDialect, PatternSource, rule_pattern};
use crate::code_rule_dsl::matcher::{
    AstMatcher, CallArgs, CompareText, Count, FieldRelation, Reading, Related, StopBy, UtilCall,
    UtilRef,
};
use crate::code_rule_dsl::model::Limits;
use ast_grep_core::Language;
use ast_grep_core::matcher::{KindMatcher, Pattern, PatternNode};
use napi::Result;
use regex::Regex;
use serde_json::Value;

#[cfg(not(test))]
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
    "calls",
];
/// Tests add `testUnknown`, a leaf whose verdict is unknown where its regex matches the node's text.
#[cfg(test)]
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
    "calls",
    "testUnknown",
];

/// Whether `key` is a rule key somewhere, for messages that tell misplaced keys from unknown ones.
pub(super) fn is_rule_key(key: &str) -> bool {
    AST_RULE_KEYS.contains(&key)
}

type RelationConstructor = fn(Box<Related>, StopBy) -> AstMatcher;

pub(super) const AST_RELATIONS: [(&str, RelationConstructor); 4] = [
    ("inside", AstMatcher::Inside),
    ("has", AstMatcher::Has),
    ("precedes", AstMatcher::Precedes),
    ("follows", AstMatcher::Follows),
];

/// Every check an ast rule must pass regardless of grammar: field names, value types, byte limits,
/// and regex syntax. `compile_ast_rule` relies on these having passed.
pub(super) fn validate_ast_rule(
    value: &Value,
    path: &str,
    limits: &Limits,
    utils: &UtilIds,
) -> Result<()> {
    let rule = object(value, path)?;
    if rule.contains_key("host") {
        return Err(invalid(format!(
            "{path}/host: host tests the literal a parse holds; use it inside parse"
        )));
    }
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
    use crate::code_rule_dsl::language::adapter::{SyntaxRules, with_adapter};
    with_adapter!(
        crate::code_rule_dsl::language::Family::of(language),
        A => <A as SyntaxRules>::DOLLAR_NAMES
    )
    .then(|| find(&pattern.node, language.expando_char(), source))
    .flatten()
}

/// A relation's `stopBy` as written: a rule object is compiled per grammar like any sub-rule.
pub(super) enum StopSpec<'a> {
    Neighbor,
    End,
    Rule(&'a Value),
}

/// A relation's `stopBy`, its `field` and the sub-rule it applies to.
pub(super) fn relation_rule<'a>(
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
pub(crate) fn compile_ast_rule(
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
            Ok((compiled, parsed_as)) if !compiled.has_error() => {
                match literal_dollar_name(&compiled, written, parsed_as) {
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
            // A supertype names a symbol no node carries: ast-grep accepts it and never matches.
            Ok(_) if is_supertype(kind, language) => grammar_gap(
                gaps,
                format!(
                    "{path}/kind: `{kind}` is an abstract kind (a grammar supertype) that no node \
                     carries; list its concrete kinds under `any`"
                ),
            ),
            // Blade's `comment` is the template's comments and its embedded PHP's alike.
            Ok(own) => match (kind, language.kinds().embedded_comment_kind()) {
                ("comment", Some(embedded)) => AstMatcher::Any(vec![
                    AstMatcher::Kind(own),
                    AstMatcher::Kind(KindMatcher::from_id(embedded)),
                ]),
                _ => AstMatcher::Kind(own),
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
            terms.push(
                match compile_relation(relation, language, &relation_path, gaps, utils)? {
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
            );
            group_paths.push(relation_path);
        }
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
        terms.push(AstMatcher::Count(Box::new(Count {
            relation: constructor(Related::new(inner), stop),
            min: spec.min,
            max: spec.max,
            binding: spec.binding,
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
    #[cfg(test)]
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

#[cfg(test)]
mod tests {
    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Value, json};

    /// A call's captures are its utility's, mapped at the call: each utility of a diamond chain is
    /// collected a constant number of times, not once per call path.
    #[test]
    fn parameterized_utility_diamonds_collect_captures_in_linear_visits() {
        let compiled = compiled_parameterized_utility_diamonds(DIAMOND_DEPTH);
        let matcher = &grammar_matchers(&compiled)[0].matcher;
        reset_walks();
        let mut variables = std::collections::BTreeMap::new();
        crate::code_rule_dsl::matcher::collect_variables(matcher, &mut variables);
        assert!(variables.contains_key("X"), "{:?}", variables.keys());
        let bound = linear_walk(DIAMOND_DEPTH);
        assert!(
            walks(Walk::CaptureVariables) <= bound,
            "capture collection expanded {} utilities (bound {bound})",
            walks(Walk::CaptureVariables)
        );
    }

    #[test]
    fn stop_by_must_be_neighbor_end_or_a_rule() {
        let reason = rejection(compile_typescript_rule(
            json!({ "kind": "identifier", "has": { "kind": "identifier", "stopBy": 3 } }),
        ));
        assert_eq!(
            reason,
            "/clauses/0/forbid/ast/rule/has/stopBy must be \"neighbor\", \"end\" or a rule object"
        );
        let reason = rejection(compile_typescript_rule(
            json!({ "kind": "identifier", "has": { "kind": "identifier", "stopBy": "far" } }),
        ));
        assert_eq!(
            reason,
            "/clauses/0/forbid/ast/rule/has/stopBy has unsupported value `far`; expected \"neighbor\", \"end\" or a rule object"
        );
        // A stop rule is an ast rule: its shape is validated like any sub-rule.
        let reason = rejection(compile_typescript_rule(json!({
            "kind": "identifier",
            "inside": { "kind": "call_expression", "stopBy": { "kind": "program", "field": "body" } },
        })));
        assert_eq!(
            reason,
            "unknown field `field` at /clauses/0/forbid/ast/rule/inside/stopBy"
        );
        let reason = rejection(compile_typescript_rule(json!({
            "kind": "identifier",
            "inside": { "kind": "call_expression", "stopBy": { "kind": "no_such_kind" } },
        })));
        assert!(
            reason.starts_with("/clauses/0/forbid/ast/rule/inside/stopBy/kind: "),
            "{reason}"
        );
    }

    #[test]
    fn non_string_kind_and_regex_are_rejected_anywhere() {
        for (rule, path) in [
            (json!({ "kind": ["identifier"] }), "rule/kind"),
            (
                json!({ "pattern": "foo()", "kind": ["call_expression"] }),
                "rule/kind",
            ),
            (
                json!({ "kind": "call_expression", "has": { "kind": 7 } }),
                "rule/has/kind",
            ),
            (json!({ "any": [{ "regex": ["^a"] }] }), "rule/any/0/regex"),
            (
                json!({ "kind": "identifier", "regex": { "source": "^a" } }),
                "rule/regex",
            ),
            (json!({ "kind": "  " }), "rule/kind"),
        ] {
            let reason = rejection(compile_typescript_rule(rule.clone()));
            assert_eq!(
                reason,
                format!("/clauses/0/forbid/ast/{path} must be a non-empty string"),
                "{rule}"
            );
        }
    }

    #[test]
    fn pattern_byte_limits_apply_to_every_pattern_source() {
        let capabilities = capabilities_with(|capabilities| {
            capabilities["limits"]["maxAstPatternBytes"] = json!(8);
            capabilities["limits"]["maxRegexPatternBytes"] = json!(4);
        });
        let ast = |rule: Value| {
            clause_document(json!({
                "id": "c",
                "forbid": { "ast": { "language": "typescript", "rule": rule } },
                "violation": violation(),
            }))
        };
        assert!(
            compile_document_with(ast(json!({ "pattern": "f($A, 1)" })), &capabilities).is_ok()
        );
        assert!(
            compile_document_with(
                ast(json!({ "kind": "identifier", "regex": "^abc" })),
                &capabilities
            )
            .is_ok()
        );
        for (rule, expected) in [
            (
                json!({ "pattern": "f($A, 10)" }),
                "/clauses/0/forbid/ast/rule/pattern is 9 bytes; capabilities.limits.maxAstPatternBytes allows at most 8",
            ),
            (
                json!({ "pattern": { "context": "class A { x = 1 }", "selector": "public_field_definition" } }),
                "/clauses/0/forbid/ast/rule/pattern/context is 17 bytes; capabilities.limits.maxAstPatternBytes allows at most 8",
            ),
            (
                json!({ "kind": "identifier", "not": { "regex": "^abcd" } }),
                "/clauses/0/forbid/ast/rule/not/regex is 5 bytes; capabilities.limits.maxRegexPatternBytes allows at most 4",
            ),
        ] {
            assert_eq!(
                rejection(compile_document_with(ast(rule), &capabilities)),
                expected
            );
        }
        let regex = clause_document(json!({
            "id": "c",
            "forbid": { "regex": { "source": "file.path", "pattern": "é{2}" } },
            "violation": violation(),
        }));
        assert_eq!(
            rejection(compile_document_with(regex, &capabilities)),
            "/clauses/0/forbid/regex/pattern is 5 bytes; capabilities.limits.maxRegexPatternBytes allows at most 4"
        );
        let missing = capabilities_with(|capabilities| {
            capabilities["limits"]
                .as_object_mut()
                .expect("limits")
                .remove("maxAstPatternBytes");
        });
        assert_eq!(
            rejection(compile_document_with(
                ast(json!({ "kind": "identifier" })),
                &missing
            )),
            "capabilities.limits.maxAstPatternBytes must be a positive integer"
        );
    }

    #[test]
    fn kindless_rules_are_accepted_and_test_anonymous_tokens() {
        // ast-grep 0.45.3 rejects a rule with no inferable kinds ("Rule must specify a set of AST
        // kinds to match"). Rerail keeps accepting it: it is the only way to report an anonymous
        // token (`kind` names named kinds only), and TSLint's golden ports anchor on `==`, `,`
        // and `default` that way. Such a rule tests every node, so it can report punctuation.
        let compiled = compile_typescript_rule(json!({
            "inside": { "kind": "parenthesized_expression" },
            "not": { "kind": "number" },
        }))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "(1);\n";
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        assert_eq!(matched_texts(&result, source), ["(", ")"]);
        let operator = compile_typescript_rule(json!({
            "regex": "^==$",
            "inside": { "kind": "binary_expression" },
        }))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "a == b;\n";
        let result = evaluate_file(&operator, "typescript", "a.ts", source);
        assert_eq!(matched_texts(&result, source), ["=="]);
    }

    #[test]
    fn abstract_kinds_are_rejected() {
        // A supertype (`expression`, `statement`, ...) resolves to a symbol no node carries, so
        // ast-grep 0.45.3 accepts it and it never matches.
        for kind in [
            "expression",
            "statement",
            "declaration",
            "primary_expression",
            "pattern",
        ] {
            let reason = rejection(compile_typescript_rule(json!({ "kind": kind })));
            assert_eq!(
                reason,
                format!(
                    "/clauses/0/forbid/ast/rule/kind: `{kind}` is an abstract kind (a grammar supertype) that no node carries; list its concrete kinds under `any` (typescript, tsx, javascript grammars)"
                ),
            );
        }
        let python = clause_document(json!({
            "id": "c",
            "forbid": { "ast": { "language": "python", "rule": { "kind": "identifier", "has": { "kind": "expression" } } } },
            "violation": violation(),
        }));
        let reason = rejection(compile_document_with(
            python,
            &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
        ));
        assert!(
            reason.starts_with(
                "/clauses/0/forbid/ast/rule/has/kind: `expression` is an abstract kind"
            ),
            "{reason}"
        );
        // Concrete kinds, including ones named like a keyword, still compile.
        for kind in ["expression_statement", "await_expression", "identifier"] {
            compile_typescript_rule(json!({ "kind": kind }))
                .unwrap_or_else(|error| panic!("{kind}: {}", error.reason));
        }
    }

    #[test]
    fn python_patterns_reject_dollar_names_that_are_not_metavariables() {
        // Python has no `$`, so ast-grep rewrites `$` before an uppercase letter or `_` to `µ`;
        // `$At` becomes the literal identifier `µAt` (names allow A-Z, 0-9, _ only) and the
        // pattern can never match. ast-grep 0.45.3 accepts it silently.
        let python = |pattern: &str| {
            compile_document_with(
                clause_document(json!({
                    "id": "c",
                    "forbid": { "ast": { "language": "python", "rule": { "pattern": pattern } } },
                    "violation": violation(),
                })),
                &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
            )
        };
        for (pattern, name) in [
            ("time.$At", "$At"),
            ("f($$$Args)", "$$$Args"),
            ("x = '$Name'", "$Name"),
            ("$_x", "$_x"),
        ] {
            assert_eq!(
                rejection(python(pattern)),
                format!(
                    "/clauses/0/forbid/ast/rule/pattern: `{name}` is not a metavariable (names use A-Z, 0-9 and _) and Python source has no `$`, so the pattern never matches (python grammar)"
                ),
                "{pattern}"
            );
        }
        for pattern in ["time.$AT", "f($$$ARGS)", "$_", "f($A1, $$$)"] {
            python(pattern).unwrap_or_else(|error| panic!("{pattern}: {}", error.reason));
        }
        // A `µ` written in the pattern is Python's own identifier character, not a rewritten `$`.
        let compiled = python("µfoo()").unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "µfoo()\nfoo()\n";
        let result = evaluate_file(&compiled, "python", "a.py", source);
        assert_eq!(matched_texts(&result, source), ["µfoo()"]);
        assert!(rejection(python("µfoo($At)")).contains("`$At` is not a metavariable"));
        // `$` is part of TypeScript identifiers: `$At` stays the literal identifier, as upstream.
        let compiled = compile_typescript_rule(json!({ "pattern": "time.$At" }))
            .unwrap_or_else(|error| panic!("{}", error.reason));
        let source = "time.sleep(1);\ntime.$At;\n";
        let result = evaluate_file(&compiled, "typescript", "a.ts", source);
        assert_eq!(matched_texts(&result, source), ["time.$At"]);
    }

    #[test]
    fn shape_errors_are_reported_once_without_a_grammar() {
        let reason = rejection(compile_typescript_rule(
            json!({ "kind": "identifier", "has": { "kinds": "identifier" } }),
        ));
        assert_eq!(
            reason,
            "unknown field `kinds` at /clauses/0/forbid/ast/rule/has"
        );
        let reason = rejection(compile_typescript_rule(
            json!({ "kind": "identifier", "regex": "(" }),
        ));
        assert_eq!(
            reason,
            "/clauses/0/forbid/ast/rule/regex: Parsing text matcher fails."
        );
        // Grammar-dependent failures keep naming the grammar.
        let reason = rejection(compile_typescript_rule(json!({ "pattern": ")(" })));
        assert!(
            reason.ends_with("(typescript, tsx, javascript grammars)"),
            "{reason}"
        );
    }
}
