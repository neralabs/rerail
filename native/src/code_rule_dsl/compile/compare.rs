mod semantic;

use super::ast_rule::AST_RELATIONS;
use super::order::AllOrderConflict;
use super::resolve::{BINDING_KEYS, OCCURRENCE_KEY};
use super::utils::UtilDef;
use crate::code_rule_dsl::json::{invalid, non_empty_str, object, reject_unknown};
use crate::code_rule_dsl::matcher::{
    Collation, CompareOp, CompareText, Comparison, CountBinding, UtilitySummary,
};
#[cfg(test)]
use crate::code_rule_dsl::test_support::{Walk, record_walk};
use napi::Result;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// A `count` object: its one relation (key and value), its bounds, and what it binds (`as`).
pub(super) struct CountSpec<'a> {
    pub(super) key: &'static str,
    pub(super) relation: &'a Value,
    pub(super) min: usize,
    pub(super) max: Option<usize>,
    pub(super) binding: Option<CountBinding>,
}

/// A `count` object's one relation, its `min`/`max` bounds, and its `as`/`offset`.
pub(super) fn count_bounds<'a>(value: &'a Value, path: &str) -> Result<CountSpec<'a>> {
    let count = object(value, path)?;
    reject_unknown(
        count,
        &[
            "inside", "has", "precedes", "follows", "min", "max", "as", "offset",
        ],
        path,
    )?;
    let relations = AST_RELATIONS
        .iter()
        .filter_map(|(key, _)| count.get(*key).map(|relation| (*key, relation)))
        .collect::<Vec<_>>();
    let [(key, relation)] = relations[..] else {
        return Err(invalid(format!(
            "{path} must contain exactly one relation: inside, has, precedes, or follows"
        )));
    };
    let bound = |name: &str| -> Result<Option<usize>> {
        count
            .get(name)
            .map(|value| {
                value
                    .as_u64()
                    .and_then(|value| usize::try_from(value).ok())
                    .ok_or_else(|| invalid(format!("{path}/{name} must be a non-negative integer")))
            })
            .transpose()
    };
    let (min, max) = (bound("min")?, bound("max")?);
    if min.unwrap_or(0) == 0 && max.is_none() {
        return Err(invalid(format!(
            "{path} must set min to at least 1, or set max; otherwise every node passes"
        )));
    }
    let min = min.unwrap_or(0);
    if max.is_some_and(|max| max < min) {
        return Err(invalid(format!("{path}: min must not exceed max")));
    }
    let offset = count
        .get("offset")
        .map(|value| {
            value
                .as_i64()
                .ok_or_else(|| invalid(format!("{path}/offset must be an integer")))
        })
        .transpose()?;
    let binding = match count.get("as") {
        None if offset.is_some() => {
            return Err(invalid(format!(
                "{path}/offset is added to the value `as` binds; set `as`"
            )));
        }
        None => None,
        Some(name) => {
            // A bare name, as `compare` takes it.
            let name = non_empty_str(Some(name), &format!("{path}/as"))?;
            let valid = |c: char| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_';
            if !name.chars().all(valid) {
                return Err(invalid(format!(
                    "{path}/as: `{name}` is not a metavariable name (A-Z, 0-9, _, without `$`)"
                )));
            }
            Some(CountBinding {
                name: name.to_owned(),
                offset: offset.unwrap_or(0),
            })
        }
    };
    Ok(CountSpec {
        key,
        relation,
        min,
        max,
        binding,
    })
}

/// A `compare` object; its names are checked against the whole query once it compiled.
pub(super) fn parse_comparison(value: &Value, path: &str) -> Result<Comparison> {
    let entry = object(value, path)?;
    reject_unknown(
        entry,
        &["left", "op", "right", "collation", "text", "commute"],
        path,
    )?;
    let name = |side: &str| -> Result<String> {
        // Bare names, as `constraints` keys are.
        let name = non_empty_str(entry.get(side), &format!("{path}/{side}"))?;
        let valid = |c: char| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_';
        if !name.chars().all(valid) {
            return Err(invalid(format!(
                "{path}/{side}: `{name}` is not a metavariable name (A-Z, 0-9, _, without `$`)"
            )));
        }
        Ok(name.to_owned())
    };
    let choice = |key: &str, default: Option<&'static str>, allowed: &[&str]| -> Result<String> {
        let value = match (entry.get(key), default) {
            (None, None) => return Err(invalid(format!("{path}/{key} is required"))),
            (None, Some(default)) => default,
            (Some(value), _) => value
                .as_str()
                .ok_or_else(|| invalid(format!("{path}/{key} must be a string")))?,
        };
        if allowed.contains(&value) {
            Ok(value.to_owned())
        } else {
            Err(invalid(format!(
                "{path}/{key}: unknown value `{value}`; expected one of {}",
                allowed.join(", ")
            )))
        }
    };
    let op = match choice("op", None, &["lt", "le", "eq", "ne", "gt", "ge"])?.as_str() {
        "lt" => CompareOp::Lt,
        "le" => CompareOp::Le,
        "eq" => CompareOp::Eq,
        "ne" => CompareOp::Ne,
        "gt" => CompareOp::Gt,
        _ => CompareOp::Ge,
    };
    let collation = match choice(
        "collation",
        Some("ordinal"),
        &[
            "ordinal",
            "fold-lower",
            "fold-upper",
            "lowercase-first",
            "natural",
            "natural-fold-lower",
        ],
    )?
    .as_str()
    {
        "fold-lower" => Collation::FoldLower,
        "natural" => Collation::Natural,
        "natural-fold-lower" => Collation::NaturalFoldLower,
        "fold-upper" => Collation::FoldUpper,
        "lowercase-first" => Collation::LowercaseFirst,
        _ => Collation::Ordinal,
    };
    let text_name = choice(
        "text",
        Some("source"),
        &[
            "source",
            "unquoted",
            "value",
            "key",
            "tokens",
            "structure",
            "structure-hash",
            "structure-parts",
            "node",
            "member",
            "name",
            "reference",
            "reference-no-static-keys",
            "index",
        ],
    )?;
    let text = match text_name.as_str() {
        "unquoted" => CompareText::Unquoted,
        "tokens" => CompareText::Tokens,
        "node" => CompareText::Node,
        "index" => CompareText::Index,
        name => semantic::text(name).map_or(CompareText::Source, CompareText::Semantic),
    };
    // The modes that compare for equality only, exactly, and what they compare.
    let exact = match text {
        CompareText::Tokens => Some("tokens"),
        CompareText::Node => Some("nodes"),
        CompareText::Semantic(text) => semantic::exact(text),
        _ => None,
    };
    if let Some(compared) = exact {
        if !matches!(op, CompareOp::Eq | CompareOp::Ne) {
            return Err(invalid(format!(
                "{path}/op: `{text_name}` compares for equality only; use eq or ne"
            )));
        }
        if collation != Collation::Ordinal {
            return Err(invalid(format!(
                "{path}/collation: `{text_name}` compares {compared} exactly; use ordinal"
            )));
        }
    }
    let commute = match entry.get("commute") {
        None => Vec::new(),
        Some(value) => {
            let malformed = || {
                invalid(format!(
                    "{path}/commute must be a non-empty array of operator texts"
                ))
            };
            let operators = value
                .as_array()
                .filter(|operators| !operators.is_empty())
                .ok_or_else(malformed)?
                .iter()
                .map(|operator| {
                    operator
                        .as_str()
                        .filter(|operator| !operator.trim().is_empty())
                        .map(str::to_owned)
                        .ok_or_else(malformed)
                })
                .collect::<Result<Vec<_>>>()?;
            if text != CompareText::Tokens {
                return Err(invalid(format!(
                    "{path}/commute: commute applies to text tokens only"
                )));
            }
            operators
        }
    };
    if text == CompareText::Index && collation != Collation::Ordinal {
        return Err(invalid(format!(
            "{path}/collation: `index` compares positions exactly; use ordinal"
        )));
    }
    Ok(Comparison {
        left: name("left")?,
        op,
        right: name("right")?,
        collation,
        text,
        reading: None,
        transform: None,
        commute,
    })
}

/// A `compare` in a validated rule: its path, the side, the name it reads, and the scopes it sits
/// in, innermost last. A utility's comparison has a site where the utility stands alone and one
/// per distinct scope a call runs it in.
pub(super) struct CompareSite<'a> {
    pub(super) path: String,
    pub(super) side: &'static str,
    pub(super) name: &'a str,
    scopes: Vec<Scope>,
}

/// A sub-rule that evaluates in an environment of its own: `not` and `count` start from the
/// caller's bindings and keep what they bind to themselves; a stop rule starts empty.
#[derive(Clone)]
pub(super) struct Scope {
    fresh: bool,
    names: BTreeSet<String>,
}

impl CompareSite<'_> {
    /// Whether the compared name can be bound where the comparison runs: by the scopes it sits in
    /// back to the nearest stop rule, or, with no stop rule between, anywhere the query binds.
    pub(super) fn visible(&self, bindable: &BTreeSet<String>) -> bool {
        self.visible_as(self.name, bindable)
    }

    /// Whether `name` can be bound where the comparison runs; see `visible`.
    pub(super) fn visible_as(&self, name: &str, bindable: &BTreeSet<String>) -> bool {
        for scope in self.scopes.iter().rev() {
            if scope.names.contains(name) {
                return true;
            }
            if scope.fresh {
                return false;
            }
        }
        bindable.contains(name)
    }
}

/// What the names a sub-rule's patterns define are read with: the pattern value, compiled for the
/// query's grammars.
pub(super) type PatternNames<'f> = &'f dyn Fn(&Value) -> BTreeSet<String>;

/// Names the terms of one scope bind: patterns in the rule, its composites, the sub-rules of its
/// relations, and the utilities it references (which share the environment); not those of nested
/// `not`, `count` or stop rules, which evaluate in their own. `summaries` holds each utility's own
/// names, which a reference adds and every call of a parameterized one maps to its caller's.
fn scope_names(
    rule: &Value,
    utils: &BTreeMap<String, UtilDef>,
    pattern_names: PatternNames<'_>,
    summaries: &mut UtilitySummary<String, BTreeSet<String>>,
    names: &mut BTreeSet<String>,
) {
    let Some(rule) = rule.as_object() else {
        return;
    };
    if let Some(pattern) = rule.get("pattern") {
        names.extend(pattern_names(pattern));
    }
    if let Some(capture) = rule.get("capture").and_then(Value::as_str) {
        names.insert(capture.to_owned());
    }
    if let Some(name) = rule
        .get("count")
        .and_then(|count| count.get("as"))
        .and_then(Value::as_str)
    {
        names.insert(name.to_owned());
    }
    // A parsed tree's captures are exported to the rule that parses it.
    for parsed in [
        rule.get("parse"),
        rule.get("formatArgs").and_then(|test| test.get("missing")),
    ]
    .into_iter()
    .flatten()
    {
        let mut exported = BTreeSet::new();
        scope_names(parsed, utils, pattern_names, summaries, &mut exported);
        names.extend(exported);
    }
    if let Some(bind) = rule
        .get("arguments")
        .and_then(|arguments| arguments.get("bind"))
        .and_then(Value::as_object)
    {
        names.extend(bind.keys().cloned());
    }
    for key in ["all", "any"] {
        for item in rule
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            scope_names(item, utils, pattern_names, summaries, names);
        }
    }
    for (key, _) in AST_RELATIONS {
        if let Some(relation) = rule.get(key) {
            scope_names(relation, utils, pattern_names, summaries, names);
        }
    }
    for (_, sub) in binding_rules(rule) {
        scope_names(sub, utils, pattern_names, summaries, names);
    }
    if let Some(of_rule) = rule
        .get("nthChild")
        .and_then(Value::as_object)
        .and_then(|nth_child| nth_child.get("ofRule"))
    {
        scope_names(of_rule, utils, pattern_names, summaries, names);
    }
    if let Some(id) = rule.get("matches").and_then(Value::as_str) {
        names.extend(utility_names(id, utils, pattern_names, summaries));
    }
    // A parameterized call binds, by the caller's names, the parameters its utility binds.
    if let Some((id, args)) = rule.get("matches").and_then(json_call) {
        let frame = utility_names(id, utils, pattern_names, summaries);
        for (param, caller) in args {
            if frame.contains(param) {
                names.insert(caller.to_owned());
            }
        }
    }
}

/// The names utility `id`'s rule binds where it runs, in the utility's own names.
fn utility_names(
    id: &str,
    utils: &BTreeMap<String, UtilDef>,
    pattern_names: PatternNames<'_>,
    summaries: &mut UtilitySummary<String, BTreeSet<String>>,
) -> BTreeSet<String> {
    let Some(util) = utils.get(id) else {
        return BTreeSet::new();
    };
    summaries.summarize(id.to_owned(), BTreeSet::new(), |summaries| {
        let mut names = BTreeSet::new();
        scope_names(util.rule, utils, pattern_names, summaries, &mut names);
        names
    })
}

/// A parameterized call's utility and each parameter with the caller's capture it names (without
/// `$`), as a validated `matches` spells them.
pub(super) fn json_call(matches: &Value) -> Option<(&str, Vec<(&str, &str)>)> {
    let (id, args) = matches.as_object()?.iter().next()?;
    let args = args
        .as_object()?
        .iter()
        .filter_map(|(param, caller)| Some((param.as_str(), caller.as_str()?.strip_prefix('$')?)))
        .collect();
    Some((id, args))
}

/// Where a query binds each name, as written: pattern metavariables, `capture`, `arguments.bind`
/// and what parameterized calls return, each with the path of the rule object that binds it.
pub(super) fn binding_site_index(
    rule: &Value,
    rule_path: &str,
    utils: &BTreeMap<String, UtilDef>,
    constraints: &BTreeMap<String, &Value>,
    constraints_path: &str,
    pattern_names: PatternNames<'_>,
) -> Vec<(String, String)> {
    fn collect(
        value: &Value,
        path: &str,
        pattern_names: PatternNames<'_>,
        sites: &mut Vec<(String, String)>,
    ) {
        match value {
            Value::Object(fields) => {
                let mut names = BTreeSet::new();
                if let Some(pattern) = fields.get("pattern") {
                    #[cfg(test)]
                    record_walk(Walk::BindingSitePatterns);
                    names.extend(pattern_names(pattern));
                }
                if let Some(capture) = fields.get("capture").and_then(Value::as_str) {
                    names.insert(capture.to_owned());
                }
                if let Some(name) = fields
                    .get("count")
                    .and_then(|count| count.get("as"))
                    .and_then(Value::as_str)
                {
                    names.insert(name.to_owned());
                }
                if let Some(bind) = fields
                    .get("arguments")
                    .and_then(|arguments| arguments.get("bind"))
                    .and_then(Value::as_object)
                {
                    names.extend(bind.keys().cloned());
                }
                if let Some((_, args)) = fields.get("matches").and_then(json_call) {
                    names.extend(args.into_iter().map(|(_, caller)| caller.to_owned()));
                }
                sites.extend(names.into_iter().map(|name| (path.to_owned(), name)));
                for (key, child) in fields {
                    collect(child, &format!("{path}/{key}"), pattern_names, sites);
                }
            }
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    collect(item, &format!("{path}/{index}"), pattern_names, sites);
                }
            }
            _ => {}
        }
    }

    let mut sites = Vec::new();
    collect(rule, rule_path, pattern_names, &mut sites);
    for util in utils.values() {
        collect(util.rule, &util.rule_path, pattern_names, &mut sites);
    }
    for (name, constraint) in constraints {
        collect(
            constraint,
            &format!("{constraints_path}/{name}"),
            pattern_names,
            &mut sites,
        );
    }
    sites
}

/// Whether `path` is `prefix` or lies under it.
fn under(path: &str, prefix: &str) -> bool {
    path.strip_prefix(prefix)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

/// Whether something outside an `all` conflict's items binds its name where the `all` runs: in the
/// query (its rule, utilities and constraints), or, inside a parameterized utility, in that
/// utility's frame (its parameters, which a caller may pass bound, and its own rule).
pub(super) fn bound_elsewhere(
    conflict: &AllOrderConflict,
    utils: &BTreeMap<String, UtilDef>,
    sites: &[(String, String)],
) -> bool {
    let frame = utils
        .values()
        .find(|util| !util.params.is_empty() && under(&conflict.path, &util.rule_path));
    if frame.is_some_and(|util| util.params.contains(&conflict.name)) {
        return true;
    }
    // What the rule object holding the `all` binds before it was counted where the conflict was
    // found; its other terms bind the name only after the `all` read it.
    sites.iter().any(|(path, name)| {
        name == &conflict.name
            && !under(path, &conflict.path)
            && match frame {
                Some(util) => under(path, &util.rule_path),
                None => !utils
                    .values()
                    .any(|util| !util.params.is_empty() && under(path, &util.rule_path)),
            }
    })
}

/// The scope a parameterized utility's rule runs in: a frame of its own, holding its parameters
/// (whatever a caller passes) and what it binds itself.
pub(super) fn frame_scope(
    util: &UtilDef,
    utils: &BTreeMap<String, UtilDef>,
    pattern_names: PatternNames<'_>,
) -> Vec<Scope> {
    let mut names = util.params.iter().cloned().collect::<BTreeSet<_>>();
    scope_names(
        util.rule,
        utils,
        pattern_names,
        &mut UtilitySummary::new(),
        &mut names,
    );
    vec![Scope { fresh: true, names }]
}

/// The binding objects of a rule that take filters and sub-rules, with their paths under the rule.
fn binding_filters(rule: &serde_json::Map<String, Value>) -> Vec<(&'static str, &Value)> {
    [
        (
            "resolves/binding",
            rule.get("resolves")
                .and_then(|resolves| resolves.get("binding")),
        ),
        ("binds", rule.get("binds")),
        ("formatArgs", rule.get("formatArgs")),
        (
            "binds/redefines",
            rule.get("binds").and_then(|binds| binds.get("redefines")),
        ),
    ]
    .into_iter()
    .filter_map(|(path, owner)| owner.map(|owner| (path, owner)))
    .collect()
}

/// The sub-rules of a rule's `resolves.binding` and `binds`, with their paths under the rule; they
/// share the rule's scope.
fn binding_rules(rule: &serde_json::Map<String, Value>) -> Vec<(String, &Value)> {
    let mut rules = Vec::new();
    for (owner_path, owner) in binding_filters(rule) {
        let keys: Vec<&str> = if owner_path == "formatArgs" {
            vec!["unused"]
        } else {
            BINDING_KEYS
                .iter()
                .copied()
                .chain([OCCURRENCE_KEY])
                .collect()
        };
        for key in keys {
            if let Some(sub) = owner.get(key) {
                rules.push((format!("{owner_path}/{key}"), sub));
            }
        }
    }
    rules
}

/// The utility calls a walk has followed, each with the names visible where it runs.
struct Calls {
    walked: BTreeSet<(String, BTreeSet<String>, bool)>,
}

/// Every `compare` and `within` in a validated rule, with the scopes it sits in. A utility the rule
/// calls shares the caller's environment, so it is walked again from each call, in the caller's
/// scopes: a comparison there may read what the caller binds, under `not` too.
pub(super) fn compare_sites<'a>(
    rule: &'a Value,
    path: &str,
    utils: &BTreeMap<String, UtilDef<'a>>,
    pattern_names: PatternNames<'_>,
    mut scopes: Vec<Scope>,
    sites: &mut Vec<CompareSite<'a>>,
) {
    let mut calls = Calls {
        walked: BTreeSet::new(),
    };
    walk_sites(
        rule,
        path,
        utils,
        pattern_names,
        &mut calls,
        &mut scopes,
        sites,
    );
}

/// What a call's comparisons can see: the names of the scopes back to the nearest stop rule, and
/// whether one ends them (else anything the query binds is visible too).
fn visible_names(scopes: &[Scope]) -> (BTreeSet<String>, bool) {
    let mut names = BTreeSet::new();
    for scope in scopes.iter().rev() {
        names.extend(scope.names.iter().cloned());
        if scope.fresh {
            return (names, true);
        }
    }
    (names, false)
}

fn walk_sites<'a>(
    rule: &'a Value,
    path: &str,
    utils: &BTreeMap<String, UtilDef<'a>>,
    pattern_names: PatternNames<'_>,
    calls: &mut Calls,
    scopes: &mut Vec<Scope>,
    sites: &mut Vec<CompareSite<'a>>,
) {
    let Some(object) = rule.as_object() else {
        return;
    };
    let enter = |sub: &'a Value,
                 sub_path: String,
                 fresh: bool,
                 calls: &mut Calls,
                 scopes: &mut Vec<Scope>,
                 sites: &mut Vec<CompareSite<'a>>| {
        let mut names = BTreeSet::new();
        scope_names(
            sub,
            utils,
            pattern_names,
            &mut UtilitySummary::new(),
            &mut names,
        );
        scopes.push(Scope { fresh, names });
        walk_sites(sub, &sub_path, utils, pattern_names, calls, scopes, sites);
        scopes.pop();
    };
    if let Some(not) = object.get("not") {
        enter(not, format!("{path}/not"), false, calls, scopes, sites);
    }
    // A parsed tree's rule has an environment of its own.
    if let Some(parse) = object.get("parse") {
        enter(parse, format!("{path}/parse"), true, calls, scopes, sites);
    }
    // A `host` rule (inside a parsed tree's rule) runs in the host's environment, with the tree's
    // captures exported into it: the tree's scopes join the host's, and its own names are its own.
    if let Some(host) = object.get("host") {
        let mut hosted = scopes.clone();
        if let Some(tree) = hosted.iter().rposition(|scope| scope.fresh) {
            let names = hosted[tree..]
                .iter()
                .flat_map(|scope| scope.names.iter().cloned())
                .collect();
            hosted.truncate(tree);
            hosted.push(Scope {
                fresh: false,
                names,
            });
        }
        enter(
            host,
            format!("{path}/host"),
            false,
            calls,
            &mut hosted,
            sites,
        );
    }
    if let Some(missing) = object
        .get("formatArgs")
        .and_then(|test| test.get("missing"))
    {
        enter(
            missing,
            format!("{path}/formatArgs/missing"),
            true,
            calls,
            scopes,
            sites,
        );
    }
    let count = object.get("count").and_then(Value::as_object);
    if let Some(count) = count {
        for (key, _) in AST_RELATIONS {
            if let Some(relation) = count.get(key) {
                enter(
                    relation,
                    format!("{path}/count/{key}"),
                    false,
                    calls,
                    scopes,
                    sites,
                );
            }
        }
    }
    for (key, _) in AST_RELATIONS {
        if let Some(relation) = object.get(key) {
            let relation_path = format!("{path}/{key}");
            walk_sites(
                relation,
                &relation_path,
                utils,
                pattern_names,
                calls,
                scopes,
                sites,
            );
        }
    }
    for (sub_path, sub) in binding_rules(object) {
        walk_sites(
            sub,
            &format!("{path}/{sub_path}"),
            utils,
            pattern_names,
            calls,
            scopes,
            sites,
        );
    }
    for (owner, owner_path) in [
        (Some(object), path.to_owned()),
        (count, format!("{path}/count")),
    ] {
        let Some(owner) = owner else { continue };
        for (key, _) in AST_RELATIONS {
            if let Some(stop) = owner.get(key).and_then(|relation| relation.get("stopBy"))
                && stop.is_object()
            {
                enter(
                    stop,
                    format!("{owner_path}/{key}/stopBy"),
                    true,
                    calls,
                    scopes,
                    sites,
                );
            }
        }
    }
    for key in ["all", "any"] {
        for (index, item) in object
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            walk_sites(
                item,
                &format!("{path}/{key}/{index}"),
                utils,
                pattern_names,
                calls,
                scopes,
                sites,
            );
        }
    }
    if let Some(of_rule) = object
        .get("nthChild")
        .and_then(Value::as_object)
        .and_then(|nth_child| nth_child.get("ofRule"))
    {
        walk_sites(
            of_rule,
            &format!("{path}/nthChild/ofRule"),
            utils,
            pattern_names,
            calls,
            scopes,
            sites,
        );
    }
    // A utility runs in its caller's environment: its comparisons see what the caller binds.
    if let Some(id) = object.get("matches").and_then(Value::as_str)
        && let Some(util) = utils.get(id)
    {
        let (names, fresh) = visible_names(scopes);
        if calls.walked.insert((id.to_owned(), names, fresh)) {
            let util_path = util.rule_path.clone();
            walk_sites(
                util.rule,
                &util_path,
                utils,
                pattern_names,
                calls,
                scopes,
                sites,
            );
        }
    }
    for (owner_path, owner) in binding_filters(object) {
        if let Some(name) = owner
            .get("bindings")
            .and_then(|bindings| bindings.get("within"))
            .and_then(Value::as_str)
        {
            sites.push(CompareSite {
                path: format!("{path}/{owner_path}/bindings"),
                side: "within",
                name,
                scopes: scopes.clone(),
            });
        }
    }
    // A gap to a capture reads it where the layout runs, as a comparison does.
    for side in ["before", "after"] {
        if let Some(name) = object
            .get("layout")
            .and_then(|layout| layout.get(side))
            .and_then(|gap| gap.get("to"))
            .and_then(Value::as_str)
            .and_then(|to| to.strip_prefix("capture."))
        {
            sites.push(CompareSite {
                path: format!("{path}/layout/{side}"),
                side: "to",
                name,
                scopes: scopes.clone(),
            });
        }
    }
    if let Some(compare) = object.get("compare") {
        for side in ["left", "right"] {
            if let Some(name) = compare.get(side).and_then(Value::as_str) {
                sites.push(CompareSite {
                    path: format!("{path}/compare"),
                    side,
                    name,
                    scopes: scopes.clone(),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Value, json};
    use std::cell::Cell;
    use std::collections::BTreeSet;

    /// Each call of a parameterized utility binds, under its own caller's name, what the utility
    /// binds: a second call with another mapping makes its capture visible too.
    #[test]
    fn every_call_of_a_parameterized_utility_binds_its_callers_names() {
        let call = |capture: &str| json!({ "bind": { "P": capture } });
        let compiled = compiled(json!({
            "rule": { "kind": "call_expression", "matches": { "same": { "Q": "$F" } } },
            "utils": {
                "bind": { "params": ["P"], "rule": { "pattern": "$P" } },
                "same": { "params": ["Q"], "rule": { "all": [
                    { "has": { "field": "function", "matches": call("$Q") } },
                    { "has": { "field": "arguments", "has": { "kind": "identifier", "matches": call("$X") } } },
                    { "compare": { "left": "X", "op": "eq", "right": "Q", "text": "tokens" } },
                ] } },
            },
        }));
        let source = "f(f);\nf(g);\n";
        assert_eq!(
            matched_texts(
                &evaluate_file(&compiled, "typescript", "a.ts", source),
                source
            ),
            ["f(f) F=f"]
        );
    }

    /// A parameterized utility's frame names come from each utility it calls once per walk, so a
    /// diamond chain compiles each of its patterns a constant number of times.
    #[test]
    fn parameterized_utility_diamonds_scope_names_in_linear_pattern_compilations() {
        let utils = parameterized_utility_diamonds(DIAMOND_DEPTH);
        let utils = super::super::utils::util_map(Some(&utils), "/utils").expect("utilities");
        let compilations = Cell::new(0);
        let pattern_names = |pattern: &Value| {
            compilations.set(compilations.get() + 1);
            pattern
                .as_str()
                .and_then(|pattern| pattern.strip_prefix('$'))
                .map(|name| BTreeSet::from([name.to_owned()]))
                .unwrap_or_default()
        };
        let top = &utils[&format!("u{DIAMOND_DEPTH}")];
        let scopes = super::frame_scope(top, &utils, &pattern_names);
        assert!(scopes[0].names.contains("P"));
        let bound = linear_walk(DIAMOND_DEPTH);
        assert!(
            compilations.get() <= bound,
            "frame names compiled {} patterns (bound {bound})",
            compilations.get()
        );
    }

    /// Every `all` order conflict is settled against one index of the query's binding sites: its
    /// patterns compile once per query, not once per conflict, and not at all without a conflict.
    #[test]
    fn all_order_conflicts_compile_binding_sites_once_per_query() {
        let binders = [
            json!({ "has": { "field": "function", "pattern": "$F" } }),
            json!({ "has": { "field": "arguments", "has": { "pattern": "$A", "kind": "identifier" } } }),
        ];
        let compare =
            json!({ "compare": { "left": "F", "op": "eq", "right": "A", "text": "tokens" } });
        // Each utility reads `$F` and `$A` before binding them, which the query's pattern excuses.
        let utilities = DIAMOND_DEPTH;
        let utils = (0..utilities)
            .map(|index| {
                let rule = json!({ "all": [compare, binders[0], binders[1]] });
                (format!("c{index}"), rule)
            })
            .collect::<serde_json::Map<_, _>>();
        reset_walks();
        compiled(json!({ "rule": { "pattern": "$F($A)" }, "utils": utils }));
        let bound = linear_walk(utilities);
        assert!(
            walks(Walk::BindingSitePatterns) <= bound,
            "settling conflicts compiled {} binding-site patterns (bound {bound})",
            walks(Walk::BindingSitePatterns)
        );
        reset_walks();
        compiled(
            json!({ "rule": { "pattern": "$F($A)", "all": [binders[0], binders[1], compare] } }),
        );
        assert_eq!(walks(Walk::BindingSitePatterns), 0);
    }

    #[test]
    fn compare_names_must_be_single_captures_the_query_binds() {
        let reject = |rule: Value| rejection(compile_typescript_ast(json!({ "rule": rule })));
        let with = |compare: Value| json!({ "pattern": "f($A, $$$REST)", "not": { "pattern": "f($N, $$$)" }, "compare": compare });
        assert_eq!(
            reject(with(json!({ "left": "A", "op": "lt", "right": "N" }))),
            "/clauses/0/forbid/ast/rule/compare/right: `$N` is never bound where this comparison runs (names bound under `not` or `count` are visible only inside it; a stopBy rule sees only the names it binds itself)"
        );
        assert_eq!(
            reject(with(json!({ "left": "REST", "op": "lt", "right": "A" }))),
            "/clauses/0/forbid/ast/rule/compare/left: `$$$REST` captures a node list; compare applies to single-node captures only"
        );
        assert_eq!(
            reject(with(json!({ "left": "$A", "op": "lt", "right": "A" }))),
            "/clauses/0/forbid/ast/rule/compare/left: `$A` is not a metavariable name (A-Z, 0-9, _, without `$`)"
        );
        assert_eq!(
            reject(with(json!({ "left": "A", "right": "A" }))),
            "/clauses/0/forbid/ast/rule/compare/op is required"
        );
        assert_eq!(
            reject(with(
                json!({ "left": "A", "op": "lt", "right": "A", "collation": "locale" })
            )),
            "/clauses/0/forbid/ast/rule/compare/collation: unknown value `locale`; expected one of ordinal, fold-lower, fold-upper, lowercase-first, natural, natural-fold-lower"
        );
        assert_eq!(
            reject(with(
                json!({ "left": "A", "op": "lt", "right": "A", "numeric": true })
            )),
            "unknown field `numeric` at /clauses/0/forbid/ast/rule/compare"
        );
        // An unused utility that binds its own names compiles like any other.
        compiled(json!({
            "rule": { "pattern": "f($A)" },
            "utils": { "unused": { "pattern": "g($B, $C)", "compare": { "left": "B", "op": "lt", "right": "C" } } },
        }));
        // Names a utility or a constraint binds are captures; so is a comparison on its own.
        compiled(json!({
            "rule": { "pattern": "f($A)", "matches": "g" },
            "utils": { "g": { "has": { "pattern": "$G", "kind": "identifier" }, "compare": { "left": "A", "op": "ne", "right": "G" } } },
        }));
        compiled(json!({
            "rule": { "pattern": "f($A)" },
            "constraints": { "A": { "pattern": "$B", "compare": { "left": "B", "op": "eq", "right": "A" } } },
        }));
    }

    #[test]
    fn a_comparison_reads_names_bound_in_its_own_negation_count_or_stop_rule() {
        // ESLint E1: a comparison inside `not`, `count` or a stop rule reads the bindings that
        // sub-rule makes, since each threads one environment through its own terms.
        let later = json!({
            "stopBy": "end",
            "kind": "pair",
            "has": { "kind": "property_identifier", "pattern": "$J" },
            "compare": { "left": "J", "op": "eq", "right": "K" },
        });
        let base = |rest: Value| {
            let mut rule = json!({ "kind": "pair", "has": { "kind": "property_identifier", "pattern": "$K" } });
            rule.as_object_mut()
                .expect("object")
                .extend(rest.as_object().expect("object").clone());
            rule
        };
        let source = "x = { a: 1, b: 2, a: 3, c: 4 };\n";
        // The `not` reads `$K`, so it runs after the relation that binds it.
        let last_of_each = compile_typescript_rule(json!({
            "kind": "pair",
            "all": [base(json!({})), { "not": { "precedes": later.clone() } }],
        }))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_eq!(
            matched_texts(
                &evaluate_file(&last_of_each, "typescript", "a.ts", source),
                source
            ),
            ["b: 2 K=b", "a: 3 K=a", "c: 4 K=c"]
        );
        let repeated = compile_typescript_rule(base(
            json!({ "count": { "follows": later.clone(), "min": 1 } }),
        ))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        assert_eq!(
            matched_texts(
                &evaluate_file(&repeated, "typescript", "a.ts", source),
                source
            ),
            ["a: 3 K=a"]
        );
        // A stop rule runs in a fresh environment: it may compare what it binds itself, but the
        // caller's `$K` is unbound there.
        assert_eq!(
            rejection(compile_typescript_rule(base(json!({
                "inside": { "kind": "program", "stopBy": { "kind": "object", "has": { "kind": "pair", "pattern": "$J" }, "compare": { "left": "J", "op": "eq", "right": "K" } } },
            })))),
            "/clauses/0/forbid/ast/rule/inside/stopBy/compare/right: `$K` is never bound where this comparison runs (names bound under `not` or `count` are visible only inside it; a stopBy rule sees only the names it binds itself)"
        );
        compile_typescript_rule(base(json!({
            "inside": { "kind": "program", "stopBy": { "kind": "object", "has": { "kind": "pair", "has": { "kind": "property_identifier", "pattern": "$J" } }, "not": { "has": { "kind": "pair", "has": { "kind": "property_identifier", "pattern": "$I" }, "compare": { "left": "I", "op": "eq", "right": "J" } } } } },
        })))
        .unwrap_or_else(|error| panic!("{}", error.reason));
        // A name bound in one `not` is still unbound in a sibling `not`, and outside both.
        assert_eq!(
            rejection(compile_typescript_rule(base(json!({
                "all": [
                    { "not": { "has": { "kind": "property_identifier", "pattern": "$J" } } },
                    { "not": { "compare": { "left": "J", "op": "eq", "right": "K" } } },
                ],
            })))),
            "/clauses/0/forbid/ast/rule/all/1/not/compare/left: `$J` is never bound where this comparison runs (names bound under `not` or `count` are visible only inside it; a stopBy rule sees only the names it binds itself)"
        );
        // A utility referenced inside the negation binds into it too.
        compiled(json!({
            "rule": base(json!({ "not": { "precedes": { "stopBy": "end", "matches": "key-j", "compare": { "left": "J", "op": "eq", "right": "K" } } } })),
            "utils": { "key-j": { "kind": "pair", "has": { "kind": "property_identifier", "pattern": "$J" } } },
        }));
    }

    /// The ESLint suite's M10 run 2 repro: a utility's comparison reads what its caller binds
    /// under `not`, `any` or a relation, as the same term written inline does.
    #[test]
    fn a_utility_comparison_reads_what_its_caller_binds() {
        let util = json!({ "u": { "kind": "member_expression",
            "has": { "kind": "property_identifier", "pattern": "$P" },
            "all": [{ "compare": { "left": "N", "op": "eq", "right": "P", "text": "name" } }] } });
        let inline = util["u"].clone();
        let source = "var foo = o.foo; var bar = o.baz;\n";
        let texts = |rule: Value, utils: Option<Value>| {
            let query = match utils {
                Some(utils) => json!({ "rule": rule, "utils": utils }),
                None => json!({ "rule": rule }),
            };
            let compiled =
                compile_typescript_ast(query).unwrap_or_else(|error| panic!("{}", error.reason));
            let result = evaluate_file(&compiled, "typescript", "a.ts", source);
            assert_eq!(result["failures"], json!([]), "{result}");
            matched_texts(&result, source)
        };
        let name = json!({ "kind": "identifier", "pattern": "$N" });
        // Under `not`: a declarator whose name is not the property it reads.
        let not = |call: Value| json!({ "kind": "variable_declarator", "not": { "all": [{ "has": name }, { "has": call }] } });
        assert_eq!(
            texts(not(json!({ "matches": "u" })), Some(util.clone())),
            ["bar = o.baz"]
        );
        assert_eq!(texts(not(inline.clone()), None), ["bar = o.baz"]);
        // Under `any`, and through a relation: declarators that read their own name.
        let any = |call: Value| json!({ "kind": "variable_declarator", "has": name, "any": [{ "has": call }, { "regex": "^never$" }] });
        assert_eq!(
            texts(any(json!({ "matches": "u" })), Some(util.clone())),
            ["foo = o.foo N=foo P=foo"]
        );
        assert_eq!(texts(any(inline), None), ["foo = o.foo N=foo P=foo"]);
        let relation = json!({ "kind": "identifier", "pattern": "$N",
            "inside": { "kind": "variable_declarator", "stopBy": "neighbor", "has": { "matches": "u" } } });
        assert_eq!(texts(relation, Some(util.clone())), ["foo N=foo P=foo"]);
        // A utility called only where the name is unbound is still refused.
        assert_eq!(
            rejection(compile_typescript_ast(json!({
                "rule": { "kind": "variable_declarator", "has": { "matches": "u" }, "not": { "has": name } },
                "utils": util,
            }))),
            "/clauses/0/forbid/ast/utils/u/all/0/compare/left: `$N` is never bound where this comparison runs (names bound under `not` or `count` are visible only inside it; a stopBy rule sees only the names it binds itself)"
        );
    }

    #[test]
    fn count_bounds_are_validated() {
        let reject = |count: Value| {
            rejection(compile_typescript_rule(
                json!({ "kind": "program", "count": count }),
            ))
        };
        let has = json!({ "kind": "if_statement" });
        assert_eq!(
            reject(json!({ "has": has })),
            "/clauses/0/forbid/ast/rule/count must set min to at least 1, or set max; otherwise every node passes"
        );
        assert_eq!(
            reject(json!({ "has": has, "min": 0 })),
            "/clauses/0/forbid/ast/rule/count must set min to at least 1, or set max; otherwise every node passes"
        );
        assert_eq!(
            reject(json!({ "has": has, "min": 3, "max": 2 })),
            "/clauses/0/forbid/ast/rule/count: min must not exceed max"
        );
        assert_eq!(
            reject(json!({ "has": has, "min": -1 })),
            "/clauses/0/forbid/ast/rule/count/min must be a non-negative integer"
        );
        assert_eq!(
            reject(json!({ "has": has, "inside": has, "min": 1 })),
            "/clauses/0/forbid/ast/rule/count must contain exactly one relation: inside, has, precedes, or follows"
        );
        assert_eq!(
            reject(json!({ "has": has, "min": 1, "each": true })),
            "unknown field `each` at /clauses/0/forbid/ast/rule/count"
        );
        assert_eq!(
            reject(json!({ "has": { "kind": "if_statement", "stopBy": "far" }, "min": 1 })),
            "/clauses/0/forbid/ast/rule/count/has/stopBy has unsupported value `far`; expected \"neighbor\", \"end\" or a rule object"
        );
        // `as` names a metavariable; `offset` is an integer, and only shifts what `as` binds.
        assert_eq!(
            reject(json!({ "has": has, "min": 1, "offset": 1 })),
            "/clauses/0/forbid/ast/rule/count/offset is added to the value `as` binds; set `as`"
        );
        for name in ["ccn", "$CCN"] {
            assert_eq!(
                reject(json!({ "has": has, "min": 1, "as": name })),
                format!(
                    "/clauses/0/forbid/ast/rule/count/as: `{name}` is not a metavariable name (A-Z, 0-9, _, without `$`)"
                )
            );
        }
        assert_eq!(
            reject(json!({ "has": has, "min": 1, "as": "CCN", "offset": 0.5 })),
            "/clauses/0/forbid/ast/rule/count/offset must be an integer"
        );
        // A count is a matcher on its own; comparisons inside it are checked like any other.
        compile_typescript_rule(json!({ "count": { "has": has, "min": 1 } })).expect("count alone");
        assert_eq!(
            reject(
                json!({ "has": { "pattern": "$A", "compare": { "left": "A", "op": "lt", "right": "B" } }, "min": 1 })
            ),
            "/clauses/0/forbid/ast/rule/count/has/compare/right: `$B` is never bound where this comparison runs (names bound under `not` or `count` are visible only inside it; a stopBy rule sees only the names it binds itself)"
        );
    }

    #[test]
    fn nth_child_of_rule_comparisons_share_the_containing_scope() {
        let undefined = json!({
            "kind": "identifier",
            "nthChild": {
                "position": 1,
                "ofRule": { "compare": { "left": "MISSING", "op": "eq", "right": "MISSING" } }
            }
        });
        assert_eq!(
            rejection(compile_typescript_rule(undefined)),
            "/clauses/0/forbid/ast/rule/nthChild/ofRule/compare/left: `$MISSING` is never bound where this comparison runs (names bound under `not` or `count` are visible only inside it; a stopBy rule sees only the names it binds itself)"
        );
        compile_typescript_rule(json!({
            "kind": "identifier",
            "not": {
                "all": [
                    { "nthChild": { "position": 1, "ofRule": { "pattern": "$X" } } },
                    { "compare": { "left": "X", "op": "eq", "right": "X" } }
                ]
            }
        }))
        .expect("ofRule binding is visible inside its containing negation");
    }

    /// A comparison in a utility's frame may read what a call there binds through a cycle of
    /// parameterized calls, whichever `any` branch the walk naming the frame meets first.
    #[test]
    fn frame_names_bound_through_recursive_calls_do_not_depend_on_branch_order() {
        for branches in recursive_call_branch_orders() {
            let mut utils = mutually_recursive_calls();
            utils.insert(
                "Z".to_owned(),
                json!({ "params": ["P"], "rule": { "kind": "expression_statement", "any": branches,
                    "compare": { "left": "X", "op": "eq", "right": "P" } } }),
            );
            if let Err(error) = compile_typescript_ast(json!({
                "rule": { "pattern": "$C", "matches": { "Z": { "P": "$C" } } },
                "utils": utils,
            })) {
                panic!("{branches}: {}", error.reason);
            }
        }
    }

    /// The utility walk beside M11a's keys: a utility's comparison reads what its caller's `parse`
    /// exports and what its caller's `formatArgs` binds, under `not` too, as the inline term does.
    #[test]
    fn a_utility_comparison_reads_parse_and_format_args_captures_its_caller_binds() {
        let compile = |rule: Value, utils: Option<Value>| {
            let mut ast = json!({ "language": "python", "rule": rule });
            if let Some(utils) = utils {
                ast["utils"] = utils;
            }
            compile_document_with(
                clause_document(
                    json!({ "id": "ast-match", "forbid": { "ast": ast }, "violation": violation() }),
                ),
                &serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities"),
            )
        };
        let texts = |rule: Value, utils: Option<Value>, source: &str| {
            let compiled = compile(rule, utils).unwrap_or_else(|error| panic!("{}", error.reason));
            let result = evaluate_file(&compiled, "python", "a.py", source);
            assert_eq!(result["failures"], json!([]), "{result}");
            matched_texts(&result, source)
        };
        // A call none of whose direct string arguments spells a mapping key its `%` format reads.
        let reads_key = json!({ "has": { "kind": "argument_list", "has": { "kind": "string", "pattern": "$Q",
            "compare": { "left": "Q", "op": "eq", "right": "K", "text": "unquoted" } } } });
        let keyed = |call: Value| {
            json!({ "kind": "call", "not": { "all": [
                { "has": { "kind": "string", "stopBy": "end", "parse": { "grammar": "percentFormat",
                    "has": { "kind": "mapping_key", "stopBy": "end", "capture": "K" } } } },
                call,
            ] } })
        };
        let source = "print('%(a)s' % d, 'a')\nprint('%(a)s' % d, 'b')\n";
        let utils = json!({ "u": reads_key });
        assert_eq!(
            texts(keyed(json!({ "matches": "u" })), Some(utils), source),
            ["print('%(a)s' % d, 'b')"]
        );
        assert_eq!(
            texts(keyed(reads_key.clone()), None, source),
            ["print('%(a)s' % d, 'b')"]
        );
        // The same with the key a `%` format's mapping leaves unused, which `formatArgs` binds.
        let reads_unused = json!({ "has": { "kind": "argument_list", "has": { "kind": "string", "pattern": "$Q",
            "compare": { "left": "Q", "op": "eq", "right": "U", "text": "source" } } } });
        let unused = |call: Value| {
            json!({ "kind": "call", "not": { "all": [
                { "has": { "stopBy": "end", "pattern": "$S % $R",
                    "formatArgs": { "style": "percent", "string": "S", "args": "R",
                        "unused": { "kind": "pair", "has": { "kind": "string", "pattern": "$U" } } } } },
                call,
            ] } })
        };
        let source =
            "print('%(a)s' % {'a': 1, 'b': 2}, 'b')\nprint('%(a)s' % {'a': 1, 'b': 2}, 'c')\n";
        let utils = json!({ "v": reads_unused });
        let called = texts(unused(json!({ "matches": "v" })), Some(utils), source);
        assert_eq!(called, ["print('%(a)s' % {'a': 1, 'b': 2}, 'c')"]);
        assert_eq!(called, texts(unused(reads_unused.clone()), None, source));
        // Called where neither binds the name, the utility is still refused.
        let refused = |rule: Value, utils: Value| rejection(compile(rule, Some(utils)));
        assert!(
            refused(
                json!({ "kind": "call", "has": { "matches": "u" } }),
                json!({ "u": reads_key })
            )
            .contains("`$K` is never bound where this comparison runs"),
        );
        assert!(
            refused(
                json!({ "kind": "call", "has": { "matches": "v" } }),
                json!({ "v": reads_unused })
            )
            .contains("`$U` is never bound where this comparison runs"),
        );
    }
}
