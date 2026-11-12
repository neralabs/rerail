pub mod semantic;

use super::ast_rule::AST_RELATIONS;
use super::order::AllOrderConflict;
use super::resolve::{BINDING_KEYS, OCCURRENCE_KEY};
use super::utils::UtilDef;
use crate::code_rule_dsl::json::{invalid, non_empty_str, object, reject_unknown};
use crate::code_rule_dsl::matcher::{
    Collation, CompareOp, CompareText, Comparison, CountBinding, UtilitySummary,
};
#[cfg(any(test, feature = "test-hooks"))]
use crate::code_rule_dsl::walks::{Walk, record_walk};
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
    // A parsed tree's captures are exported to the rule that parses it, an embedded program's to
    // the host rule that reaches it.
    for parsed in [
        rule.get("parse"),
        rule.get("embedded"),
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
                    #[cfg(any(test, feature = "test-hooks"))]
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
    // A parsed tree's rule has an environment of its own, as an embedded program's has.
    for key in ["parse", "embedded"] {
        if let Some(tree) = object.get(key) {
            enter(tree, format!("{path}/{key}"), true, calls, scopes, sites);
        }
    }
    // A `host` rule (inside a parsed tree's or an embedded program's rule) runs in the host's
    // environment, with the tree's captures exported into it: the tree's scopes join the host's,
    // and its own names are its own.
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
mod tests;
