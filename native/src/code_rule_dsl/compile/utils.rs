use super::order::AllOrderConflict;
use crate::code_rule_dsl::json::{invalid, object, reject_unknown};
use crate::code_rule_dsl::matcher::{CaptureAnalysis, MAX_UTIL_INSTANCES, Transform, UtilityNode};
use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
use napi::Result;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Weak};

/// A `utils` or `constraints` object: names to ast rules, in name order.
pub(super) fn rule_map<'a>(
    value: Option<&'a Value>,
    path: &str,
) -> Result<BTreeMap<String, &'a Value>> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let map = object(value, path)?;
    let mut rules = BTreeMap::new();
    for (name, rule) in map {
        if name.is_empty() {
            return Err(invalid(format!("{path}: names must not be empty")));
        }
        rules.insert(name.clone(), rule);
    }
    Ok(rules)
}

/// A utility as written: its rule, its parameters and the path its errors name (a query's `utils`,
/// or a `define` clause's for a utility the document shares). A utility with parameters runs in a
/// frame of its own (`UtilCall`).
#[derive(Clone)]
pub(super) struct UtilDef<'a> {
    pub(super) rule: &'a Value,
    pub(super) params: Vec<String>,
    /// The utility's path; its rule's is `rule_path`.
    pub(super) path: String,
    pub(super) rule_path: String,
}

/// The utilities a rule may name, by id, with their parameters.
pub(crate) type UtilIds<'a> = BTreeMap<&'a str, &'a [String]>;

/// Whether a name is a metavariable's (without `$`), as a pattern spells it.
pub(super) fn is_metavariable_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// A `utils` object as utility definitions, ids checked. A utility is a rule, or `{ params, rule }`:
/// capture names its callers bind to their own (`{ "matches": { "id": { "P": "$C" } } }`).
pub(super) fn util_map<'a>(
    value: Option<&'a Value>,
    path: &str,
) -> Result<BTreeMap<String, UtilDef<'a>>> {
    let mut utils = BTreeMap::new();
    for (id, rule) in rule_map(value, path)? {
        if id.contains(['(', ')', ',', '=']) {
            return Err(invalid(format!("{path}: invalid utility rule id `{id}`")));
        }
        let util_path = format!("{path}/{id}");
        let (rule, params, rule_path) = match rule.as_object() {
            Some(fields) if fields.contains_key("params") => {
                reject_unknown(fields, &["params", "rule"], &util_path)?;
                let params_path = format!("{util_path}/params");
                let names = fields
                    .get("params")
                    .and_then(Value::as_array)
                    .filter(|names| !names.is_empty())
                    .ok_or_else(|| {
                        invalid(format!(
                            "{params_path} must be a non-empty array of capture names"
                        ))
                    })?;
                let mut params = Vec::with_capacity(names.len());
                for (index, name) in names.iter().enumerate() {
                    let name = name
                        .as_str()
                        .filter(|name| is_metavariable_name(name))
                        .ok_or_else(|| {
                            invalid(format!(
                                "{params_path}/{index} must be a capture name without `$` (A-Z, 0-9, _)"
                            ))
                        })?;
                    if params.iter().any(|param| param == name) {
                        return Err(invalid(format!(
                            "{params_path}/{index}: `{name}` is listed twice"
                        )));
                    }
                    params.push(name.to_owned());
                }
                let rule = fields
                    .get("rule")
                    .ok_or_else(|| invalid(format!("{util_path}/rule is required")))?;
                (rule, params, format!("{util_path}/rule"))
            }
            _ => (rule, Vec::new(), util_path.clone()),
        };
        utils.insert(
            id,
            UtilDef {
                rule,
                params,
                path: util_path,
                rule_path,
            },
        );
    }
    Ok(utils)
}

/// The utility a `matches` value names: an id, or a parameterized call's one key.
pub(super) fn call_target(matches: &Value) -> Option<&str> {
    match matches {
        Value::String(id) => Some(id),
        Value::Object(call) if call.len() == 1 => call.keys().next().map(String::as_str),
        _ => None,
    }
}

/// The ids a rule names with `matches`, anywhere in it.
fn called_ids<'a>(value: &'a Value, ids: &mut Vec<&'a str>) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                if key == "matches"
                    && let Some(id) = call_target(child)
                {
                    ids.push(id);
                }
                called_ids(child, ids);
            }
        }
        Value::Array(items) => {
            for item in items {
                called_ids(item, ids);
            }
        }
        _ => {}
    }
}

/// Utility ids named directly by a rule.
pub(super) fn called_utilities(rule: &Value) -> BTreeSet<String> {
    let mut ids = Vec::new();
    called_ids(rule, &mut ids);
    ids.into_iter().map(str::to_owned).collect()
}

/// Strongly connected utility components in dependency-first order. Members are sorted by id.
/// An acyclic utility is a component of one. A recursive component is finalized as one unit.
pub(super) fn utility_components(utils: &BTreeMap<String, UtilDef<'_>>) -> Vec<Vec<String>> {
    struct Tarjan<'a> {
        utils: &'a BTreeMap<String, UtilDef<'a>>,
        next: usize,
        indices: BTreeMap<String, usize>,
        low: BTreeMap<String, usize>,
        stack: Vec<String>,
        active: BTreeSet<String>,
        components: Vec<Vec<String>>,
    }

    impl Tarjan<'_> {
        fn visit(&mut self, id: &str) {
            let index = self.next;
            self.next += 1;
            self.indices.insert(id.to_owned(), index);
            self.low.insert(id.to_owned(), index);
            self.stack.push(id.to_owned());
            self.active.insert(id.to_owned());

            let mut dependencies = Vec::new();
            called_ids(self.utils[id].rule, &mut dependencies);
            dependencies.sort_unstable();
            dependencies.dedup();
            for dependency in dependencies {
                if !self.utils.contains_key(dependency) {
                    continue;
                }
                if !self.indices.contains_key(dependency) {
                    self.visit(dependency);
                    let dependency_low = self.low[dependency];
                    self.low
                        .entry(id.to_owned())
                        .and_modify(|low| *low = (*low).min(dependency_low));
                } else if self.active.contains(dependency) {
                    let dependency_index = self.indices[dependency];
                    self.low
                        .entry(id.to_owned())
                        .and_modify(|low| *low = (*low).min(dependency_index));
                }
            }

            if self.low[id] == self.indices[id] {
                let mut component = Vec::new();
                loop {
                    let member = self.stack.pop().expect("an active utility is on the stack");
                    self.active.remove(&member);
                    let complete = member == id;
                    component.push(member);
                    if complete {
                        break;
                    }
                }
                component.sort();
                self.components.push(component);
            }
        }
    }

    let mut tarjan = Tarjan {
        utils,
        next: 0,
        indices: BTreeMap::new(),
        low: BTreeMap::new(),
        stack: Vec::new(),
        active: BTreeSet::new(),
        components: Vec::new(),
    };
    for id in utils.keys() {
        if !tarjan.indices.contains_key(id) {
            tarjan.visit(id);
        }
    }
    tarjan.components
}

/// The shared utilities `roots` reach through `matches`, directly or through each other.
pub(super) fn reachable<'a>(
    roots: impl IntoIterator<Item = &'a Value>,
    shared: &BTreeMap<String, UtilDef<'a>>,
) -> BTreeSet<String> {
    let mut pending = Vec::new();
    for root in roots {
        called_ids(root, &mut pending);
    }
    let mut reached = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if let Some(util) = shared.get(id)
            && reached.insert(id.to_owned())
        {
            called_ids(util.rule, &mut pending);
        }
    }
    reached
}

/// ast-grep-config's cycle check: utilities may recurse only through a relation, which always moves
/// to another node. A reference reached through `all`/`any`/`not`/`matches` alone would evaluate the
/// same utility on the same node forever, so it is rejected, as upstream does.
pub(super) fn reject_util_cycles(utils: &BTreeMap<String, UtilDef>) -> Result<()> {
    fn same_node_refs<'a>(rule: &'a Value, refs: &mut Vec<&'a str>) {
        if let Some(id) = rule.get("matches").and_then(call_target) {
            refs.push(id);
        }
        for key in ["all", "any"] {
            for item in rule
                .get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                same_node_refs(item, refs);
            }
        }
        if let Some(not) = rule.get("not") {
            same_node_refs(not, refs);
        }
    }
    fn visit<'a>(
        id: &'a str,
        utils: &'a BTreeMap<String, UtilDef>,
        done: &mut BTreeMap<&'a str, bool>,
    ) -> Result<()> {
        match done.get(id) {
            Some(true) => return Ok(()),
            Some(false) => {
                let path = utils.get(id).map_or(id, |util| util.path.as_str());
                return Err(invalid(format!(
                    "{path}: utility rule `{id}` has a cyclic dependency in its `matches` sub-rule"
                )));
            }
            None => {}
        }
        let Some((id, util)) = utils.get_key_value(id) else {
            return Ok(());
        };
        done.insert(id, false);
        let mut refs = Vec::new();
        same_node_refs(util.rule, &mut refs);
        for next in refs {
            visit(next, utils, done)?;
        }
        done.insert(id, true);
        Ok(())
    }
    let mut done = BTreeMap::new();
    for id in utils.keys() {
        visit(id, utils, &mut done)?;
    }
    Ok(())
}

impl UtilScope<'_> {
    /// Records a parameterized call of utility `index`: a query makes at most
    /// `MAX_UTIL_INSTANCES` distinct ones (a utility and a mapping) per grammar.
    pub(super) fn instance(
        &self,
        index: usize,
        args: &[(String, String)],
        path: &str,
    ) -> Result<()> {
        let mut instances = self.instances.borrow_mut();
        instances.insert((index, args.to_vec()));
        if instances.len() > MAX_UTIL_INSTANCES {
            return Err(invalid(format!(
                "{path}: more than {MAX_UTIL_INSTANCES} distinct parameterized utility calls in one query"
            )));
        }
        Ok(())
    }

    pub(super) fn instances(&self) -> BTreeSet<(usize, crate::code_rule_dsl::matcher::CallArgs)> {
        self.instances.borrow().clone()
    }
}

/// The utilities a rule's `matches` may name while compiling it for one grammar.
pub(crate) struct UtilScope<'a> {
    pub(crate) artifacts: &'a super::artifacts::CompilerArtifacts,
    /// Direct compiled-component targets, with the query-order index used only for call limits.
    pub(crate) targets: &'a BTreeMap<&'a str, (usize, Weak<UtilityNode>)>,
    pub(crate) ids: &'a UtilIds<'a>,
    /// The query's `languageOptions`, which its `resolves` terms read.
    pub(crate) options: Option<Arc<LanguageOptions>>,
    /// A Python query's `languageOptions`.
    pub(crate) python: Option<Arc<PythonOptions>>,
    /// The query's `transform`, whose variables comparisons may read.
    pub(crate) transform: Option<Arc<Transform>>,
    /// Where the query collects `all` items that read what a later item binds, to settle once it
    /// knows what else binds each name (`compare::settle_all_order`); `None` refuses them at once.
    pub(crate) conflicts: Option<&'a RefCell<BTreeSet<AllOrderConflict>>>,
    /// What the rule's terms bind and read, each utility summarized once per compilation of the
    /// table, for capture ordering and `all` order checks.
    pub(in crate::code_rule_dsl) captures: &'a RefCell<CaptureAnalysis>,
    /// Distinct parameterized calls belong to one query, not its reusable utility table.
    pub(crate) instances: RefCell<BTreeSet<(usize, crate::code_rule_dsl::matcher::CallArgs)>>,
    /// Where the query collects its `calls` terms; `None` where `calls` is not supported.
    pub(crate) calls: Option<&'a crate::code_rule_dsl::calls::CallsCollector>,
}

#[cfg(test)]
mod tests {
    use crate::code_rule_dsl::test_support::*;
    use serde_json::json;

    #[test]
    fn same_node_util_cycles_are_rejected_and_relation_recursion_is_not() {
        for (utils, cycle) in [
            (json!({ "a": { "matches": "a" } }), "a"),
            (
                json!({ "a": { "all": [{ "kind": "number" }, { "matches": "b" }] }, "b": { "any": [{ "matches": "a" }] } }),
                "a",
            ),
            (json!({ "a": { "not": { "matches": "a" } } }), "a"),
        ] {
            let reason = rejection(compile_typescript_ast(json!({
                "rule": { "matches": "a" },
                "utils": utils,
            })));
            assert_eq!(
                reason,
                format!(
                    "/clauses/0/forbid/ast/utils/{cycle}: utility rule `{cycle}` has a cyclic dependency in its `matches` sub-rule"
                )
            );
        }
        for relation in ["has", "inside", "precedes", "follows"] {
            compiled(json!({
                "rule": { "matches": "a" },
                "utils": { "a": { "kind": "number", relation: { "matches": "a" } } },
            }));
        }
    }

    #[test]
    fn matches_must_name_a_defined_utility() {
        assert_eq!(
            rejection(compile_typescript_ast(
                json!({ "rule": { "kind": "number", "not": { "matches": "missing" } } })
            )),
            "/clauses/0/forbid/ast/rule/not/matches: utility rule `missing` is not defined in this query's utils or a define of its language"
        );
        assert_eq!(
            rejection(compile_typescript_ast(json!({
                "rule": { "matches": { "wrap": { "INNER": { "kind": "number" } } } },
                "utils": { "wrap": { "kind": "number" } },
            }))),
            "/clauses/0/forbid/ast/rule/matches: utility rule `wrap` takes no parameters; name it (\"matches\": \"wrap\")"
        );
        assert_eq!(
            rejection(compile_typescript_ast(json!({ "rule": { "matches": 3 } }))),
            "/clauses/0/forbid/ast/rule/matches must be the id of a utility rule"
        );
        assert_eq!(
            rejection(compile_typescript_ast(
                json!({ "rule": { "kind": "number" }, "utils": { "a(b)": { "kind": "number" } } })
            )),
            "/clauses/0/forbid/ast/utils: invalid utility rule id `a(b)`"
        );
        // Unused utilities are still validated, as upstream compiles every one.
        assert_eq!(
            rejection(compile_typescript_ast(
                json!({ "rule": { "kind": "number" }, "utils": { "a": { "stopBy": "end" } } })
            )),
            "unknown field `stopBy` at /clauses/0/forbid/ast/utils/a"
        );
    }

    #[test]
    fn constraints_must_name_a_single_node_capture() {
        assert_eq!(
            rejection(compile_typescript_ast(
                json!({ "rule": { "pattern": "$F()" }, "constraints": { "G": { "kind": "identifier" } } })
            )),
            "/clauses/0/forbid/ast/constraints/G: metavariable `$G` is not defined by the rule, its utils, or its constraints"
        );
        assert_eq!(
            rejection(compile_typescript_ast(
                json!({ "rule": { "pattern": "f($$$ARGS)" }, "constraints": { "ARGS": { "kind": "number" } } })
            )),
            "/clauses/0/forbid/ast/constraints/ARGS: `$$$ARGS` captures a node list; constraints apply to single-node captures only"
        );
        // Defined by a utility or by another constraint, even under `not`, is defined.
        compiled(json!({
            "rule": { "kind": "call_expression", "matches": "callee" },
            "utils": { "callee": { "has": { "kind": "identifier", "pattern": "$F" } } },
            "constraints": { "F": { "pattern": "$G" }, "G": { "not": { "regex": "^_" } } },
        }));
    }

    #[test]
    fn utils_and_constraints_compile_per_dialect() {
        // A JSX-only utility never matches in plain TypeScript and the rest of the rule stays live.
        let rule = compiled(json!({
            "rule": { "any": [{ "matches": "jsx" }, { "pattern": "g($A)" }] },
            "utils": { "jsx": { "kind": "jsx_self_closing_element" } },
        }));
        assert_violations(&evaluate_file(&rule, "typescript", "a.ts", "g(1);\n"), 1);
        assert_violations(
            &evaluate_file(&rule, "tsx", "a.tsx", "const v = <br />;\ng(1);\n"),
            2,
        );

        // A utility or constraint no dialect can express is an error, even when it is unused.
        assert_eq!(
            rejection(compile_typescript_ast(json!({
                "rule": { "kind": "number" },
                "utils": { "bad": { "kind": "no_such_kind" } },
            }))),
            "/clauses/0/forbid/ast/utils/bad/kind: Kind `no_such_kind` is invalid. (typescript, tsx, javascript grammars)"
        );
        assert_eq!(
            rejection(compile_typescript_ast(json!({
                "rule": { "pattern": "$F()" },
                "constraints": { "F": { "kind": "no_such_kind" } },
            }))),
            "/clauses/0/forbid/ast/constraints/F/kind: Kind `no_such_kind` is invalid. (typescript, tsx, javascript grammars)"
        );
    }
}
