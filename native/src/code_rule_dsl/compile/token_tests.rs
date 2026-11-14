//! Token tests (RER-131, t31), checked as a rule compiles. A named-only tape leaves out every
//! token the grammar implies, so a test of such a token can no longer hold. A *token test* is a
//! sub-rule with a `regex` and no `kind` or `pattern` on its node, whose regex matches some token
//! of the dialect's grammar. Where it sits says where the token would be:
//!
//! | relation | the token's parent | category if the token is implied there |
//! |---|---|---|
//! | `has` (neighbour) | the node the rule is on | `gap` |
//! | `follows` / `precedes` | that node's parent | `gap` |
//! | `has` (`stopBy: end`) | any node below | `below` |
//! | none: the node matched is the token | | `root` |
//! | `inside` | (a token has no children) | `never` |
//! | `layout` to the next or previous token | the node's parent | `layout` |
//!
//! A test the gap test answers (`ast_rule::gap_test`) is `lowered`. A regex that matches a token
//! without spanning it (`^[rub]*f`) tests a node's text and reached a token only by accident:
//! `incidental`. A test whose token is never implied in its parents is `stored`: it reads a token
//! the tape keeps. A test under a `not` on the token's own node, reached as the node matched, a
//! child or any earlier or later sibling, only kept tokens out of that set, and an implied token
//! is in none: `excluder`. (A neighbour sibling is one node, which is now another one: it stays a
//! `gap`.) `gap?` is a `gap` whose parent the rule does not say. A parent is known from the rule
//! (`kind` on the node, or on its neighbour `inside`); otherwise every kind the grammar has the
//! node under, or every kind.
//!
//! A `root`, `below`, `gap` or `gap?` test is refused: nothing answers it. A test may say why its
//! token is stored where the rule looks (`stored: "<why>"`, beside its `regex`): an optional or
//! alternative token (a hole, a trailing comma, a slice's leading colons, a modifier) is stored,
//! since a gap's tokens are left out only when they are exactly the grammar's implied word. It is
//! then `declared`.

use crate::code_rule_dsl::language::dialect_by_label;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

use crate::code_rule_dsl::tape::token_census::Lang;

/// How the node a sub-rule tests was reached from the node before it.
#[derive(Clone, Debug)]
enum Rel {
    /// The node matched (a rule's root, a constraint's capture, a `site`...): its tag.
    Root(&'static str),
    Has { end: bool, field: Option<String> },
    Sibling { end: bool, field: Option<String> },
    Inside,
}

/// What the rule says about one node: its kind and its parent's.
#[derive(Clone, Debug, Default)]
struct Frame {
    kind: Option<String>,
    parent: Option<String>,
}

/// One token test. `path` is its place in the rule file (`utils/<name>/...` inside a util).
#[derive(Eq, Ord, PartialEq, PartialOrd)]
pub(in crate::code_rule_dsl) struct Test {
    pub category: &'static str,
    pub negated: bool,
    pub relation: String,
    pub regex: String,
    /// (parent, token) where the token is implied.
    pub implied: BTreeSet<(String, String)>,
    pub path: String,
}

struct Walk<'a> {
    lang: &'a Lang,
    utils: &'a Map<String, Value>,
    tests: BTreeSet<Test>,
    active: Vec<String>,
    /// A util already walked in a context (its name and the context, printed): walked once.
    seen: BTreeSet<String>,
    /// Relations the compiler also asks of the gaps (`compile::ast_rule::gap_test`), by path:
    /// a token test that is their target, or an `any` alternative of it, is `lowered`.
    lowered: BTreeSet<String>,
}

const COMPOSITE: [&str; 3] = ["all", "any", "not"];
/// Keys whose values hold the same node's sub-rules (the rule DSL's `$option` cases).
const SAME_NODE: [&str; 6] = ["$option", "cases", "default", "true", "false", "count"];
/// Keys whose values are rules over some other node, matched from scratch.
const FRESH: [&str; 5] = ["site", "rule", "context", "ofRule", "stopBy"];

impl Walk<'_> {
    /// `rule`, a sub-rule over a node reached by `rel` from a node `from` (the node the relation
    /// left); `frame` is what the enclosing same-node rules said about this node.
    fn rule(&mut self, rule: &Value, rel: &Rel, from: &Frame, frame: &Frame, negated: bool, path: &str) {
        match rule {
            Value::Array(items) => {
                for (at, item) in items.iter().enumerate() {
                    self.rule(item, rel, from, frame, negated, &format!("{path}/{at}"));
                }
            }
            Value::Object(map) => self.object(map, rel, from, frame, negated, path),
            _ => {}
        }
    }

    fn object(&mut self, map: &Map<String, Value>, rel: &Rel, from: &Frame, frame: &Frame, negated: bool, path: &str) {
        let mut frame = frame.clone();
        if let Some(kind) = node_kind(map, self.utils, 0) {
            frame.kind = Some(kind);
        }
        if let Some(inside) = map.get("inside").and_then(Value::as_object)
            && inside.get("stopBy").is_none_or(|stop| stop == "neighbor")
            && let Some(kind) = inside.get("kind").and_then(Value::as_str)
        {
            frame.parent = Some(kind.to_owned());
        }
        if let Some(regex) = map.get("regex").and_then(Value::as_str)
            && frame.kind.is_none()
            && !map.contains_key("pattern")
        {
            // The rule may say why the token is stored where it looks.
            let declared = map.get("stored").and_then(Value::as_str).is_some_and(|why| !why.is_empty());
            self.token_test(regex, rel, from, &frame, negated, declared, path);
        }
        if let Some(layout) = map.get("layout").and_then(Value::as_object) {
            for side in ["before", "after"] {
                if layout.get(side).and_then(|s| s.get("to")).and_then(Value::as_str) == Some("token") {
                    self.layout(side, &frame, negated, path);
                }
            }
        }
        for (key, value) in map {
            let here = format!("{path}/{key}");
            match key.as_str() {
                "has" | "follows" | "precedes" | "inside" => {
                    use crate::code_rule_dsl::compile::ast_rule::{gap_split, gap_test};
                    use crate::code_rule_dsl::matcher::AstMatcher;
                    if let Some(test) = gap_test(key, value, self.lang.dialect, Some(Box::new(AstMatcher::Never))) {
                        self.lowered.insert(here.clone());
                        // The token's own relations to the tokens beside it, which the test reads.
                        if !test.token.is_empty() {
                            for nested in ["follows", "precedes", "not/follows", "not/precedes"] {
                                self.lowered.insert(format!("{here}/{nested}"));
                            }
                        }
                    } else if gap_split(key, value, self.lang.dialect).is_some() {
                        self.lowered.insert(here.clone());
                    }
                    let field = value.get("field").and_then(Value::as_str).map(str::to_owned);
                    let end = value.get("stopBy").is_some_and(|stop| stop != "neighbor");
                    let (next, child) = match key.as_str() {
                        "has" if end => (Rel::Has { end, field }, Frame::default()),
                        "has" => (Rel::Has { end, field }, Frame { kind: None, parent: frame.kind.clone() }),
                        "inside" => (Rel::Inside, Frame::default()),
                        _ => (Rel::Sibling { end, field }, Frame { kind: None, parent: frame.parent.clone() }),
                    };
                    self.rule(value, &next, &frame, &child, false, &here);
                }
                "not" => self.rule(value, rel, from, &frame, !negated, &here),
                key if COMPOSITE.contains(&key) || SAME_NODE.contains(&key) => self.rule(value, rel, from, &frame, negated, &here),
                key if FRESH.contains(&key) && value.is_object() => {
                    let tag = FRESH.iter().find(|tag| **tag == key).copied().unwrap_or("fresh");
                    self.rule(value, &Rel::Root(tag), &Frame::default(), &Frame::default(), false, &here);
                }
                "matches" => {
                    let names: Vec<String> = match value {
                        Value::String(name) => vec![name.clone()],
                        Value::Object(call) => call.keys().cloned().collect(),
                        _ => Vec::new(),
                    };
                    for name in names {
                        if self.active.contains(&name) {
                            continue;
                        }
                        let context = format!("{name} {rel:?} {from:?} {frame:?} {negated}");
                        if let Some(util) = self.utils.get(&name).cloned()
                            && self.seen.insert(context)
                        {
                            self.active.push(name.clone());
                            self.rule(&util, rel, from, &frame, negated, &format!("utils/{name}"));
                            self.active.pop();
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn token_test(&mut self, regex: &str, rel: &Rel, from: &Frame, frame: &Frame, negated: bool, declared: bool, path: &str) {
        use crate::code_rule_dsl::regexes::Regex;
        let Ok(compiled) = Regex::new(regex) else { return };
        let Ok(whole) = Regex::new(&format!("^(?:{regex})$")) else { return };
        if !self.lang.tokens.iter().any(|token| compiled.is_match(token)) {
            return;
        }
        // A regex that matches a token without spanning it tests a node's text, and reaches a
        // token only by accident (`.`, `^[rub]*f`): `incidental`.
        let incidental = !self.lang.tokens.iter().any(|token| whole.is_match(token)) || ["x", "foo_bar", "0"].iter().any(|text| compiled.is_match(text));
        let compiled = if incidental { compiled } else { whole };
        let parents_of = |kind: &Option<String>| kind.as_ref().and_then(|kind| self.lang.parents.get(kind)).cloned();
        let (category, relation, implied) = match rel {
            Rel::Inside => ("never", "inside".to_owned(), BTreeSet::new()),
            // The token matched itself: under the neighbour `inside` names, where one does.
            Rel::Root(tag) => ("root", (*tag).to_owned(), self.lang.implied_in(frame.parent.clone().map(|kind| BTreeSet::from([kind])).as_ref(), &compiled, None)),
            Rel::Has { end: true, field } => ("below", "has end".to_owned(), self.lang.implied_in(None, &compiled, field.as_deref())),
            Rel::Has { end: false, field } => {
                let parents = from.kind.clone().map(|kind| BTreeSet::from([kind]));
                (if parents.is_some() { "gap" } else { "gap?" }, "has".to_owned(), self.lang.implied_in(parents.as_ref(), &compiled, field.as_deref()))
            }
            Rel::Sibling { end, field } => {
                let parents = frame.parent.clone().map(|kind| BTreeSet::from([kind])).or_else(|| parents_of(&from.kind));
                let relation = if *end { "sibling end" } else { "sibling" }.to_owned();
                (if parents.is_some() { "gap" } else { "gap?" }, relation, self.lang.implied_in(parents.as_ref(), &compiled, field.as_deref()))
            }
        };
        // A token test that is a lowered relation's target, or an `any` alternative or `all` item
        // of it.
        let target = ["/any/", "/all/"].iter().find_map(|sep| path.rsplit_once(sep).filter(|(_, at)| at.parse::<usize>().is_ok()).map(|(relation, _)| relation)).unwrap_or(path);
        let category = if category != "never" && implied.is_empty() {
            "stored"
        } else if self.lowered.contains(path) || self.lowered.contains(target) {
            "lowered"
        } else if negated && (category == "root" || (category != "never" && !matches!(rel, Rel::Sibling { end: false, .. }))) {
            // A `not` on the token's own node, over a set (the node matched, a node's children,
            // its earlier or later siblings): it kept tokens out of the set, and an implied token
            // is in none. Not a neighbour sibling, which is one node and now another one.
            "excluder"
        } else if incidental && category != "never" {
            "incidental"
        } else if declared && UNANSWERED.contains(&category) {
            "declared"
        } else {
            category
        };
        self.tests.insert(Test { category, negated, relation, regex: regex.to_owned(), implied, path: path.to_owned() });
    }

    fn layout(&mut self, side: &str, frame: &Frame, negated: bool, path: &str) {
        let parents = frame.parent.clone().map(|kind| BTreeSet::from([kind])).or_else(|| frame.kind.as_ref().and_then(|kind| self.lang.parents.get(kind)).cloned());
        let any = crate::code_rule_dsl::regexes::Regex::new(".").unwrap();
        let implied = self.lang.implied_in(parents.as_ref(), &any, None);
        let category = if implied.is_empty() { "stored" } else { "layout" };
        self.tests.insert(Test { category, negated, relation: format!("layout {side}"), regex: String::new(), implied, path: path.to_owned() });
    }
}

/// The node's kind as the rule says it: its own `kind`, one an `all` member or a util it
/// `matches` gives, or the first of an `any` whose every alternative gives one.
fn node_kind(map: &Map<String, Value>, utils: &Map<String, Value>, depth: usize) -> Option<String> {
    if depth > 8 {
        return None;
    }
    if let Some(kind) = map.get("kind").and_then(Value::as_str) {
        return Some(kind.to_owned());
    }
    let of = |value: &Value| value.as_object().and_then(|map| node_kind(map, utils, depth + 1));
    if let Some(kind) = map.get("all").and_then(Value::as_array).and_then(|all| all.iter().find_map(of)) {
        return Some(kind);
    }
    if let Some(kind) = map.get("matches").and_then(Value::as_str).and_then(|name| utils.get(name)).and_then(of) {
        return Some(kind);
    }
    let any = map.get("any")?.as_array()?;
    let kinds: Vec<String> = any.iter().filter_map(of).collect();
    (!any.is_empty() && kinds.len() == any.len()).then(|| kinds[0].clone())
}

/// The worst category of a rule's tests, in the order they cost to fix.
pub(in crate::code_rule_dsl) const ORDER: [&str; 11] = ["root", "below", "layout", "gap", "gap?", "incidental", "excluder", "declared", "lowered", "stored", "never"];

/// The categories nothing answers: a rule with such a test does not compile.
pub(in crate::code_rule_dsl) const UNANSWERED: [&str; 4] = ["root", "below", "gap", "gap?"];


/// Every token test of a policy document, by its language, a test reached in several contexts
/// once, in its worst category.
pub(in crate::code_rule_dsl) fn tests(document: &Value) -> Vec<(String, Test)> {
    let mut langs: BTreeMap<String, Option<&'static Lang>> = BTreeMap::new();
    let clauses = document.get("clauses").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut utils = Map::new();
    for clause in &clauses {
        for found in [clause.pointer("/define/utils"), clause.pointer("/forbid/ast/utils")].into_iter().flatten() {
            if let Some(map) = found.as_object() {
                utils.extend(map.clone());
            }
        }
    }
    let mut found: BTreeMap<(String, String), (String, Test)> = BTreeMap::new();
    for (at, clause) in clauses.iter().enumerate() {
        let Some(ast) = clause.pointer("/forbid/ast").and_then(Value::as_object) else { continue };
        let Some(language) = ast.get("language").and_then(Value::as_str) else { continue };
        let Some(lang) = *langs.entry(language.to_owned()).or_insert_with(|| dialect_by_label(language).and_then(Lang::of)) else { continue };
        let mut walk = Walk { lang, utils: &utils, tests: BTreeSet::new(), active: Vec::new(), seen: BTreeSet::new(), lowered: BTreeSet::new() };
        let base = format!("clauses/{at}");
        if let Some(rule) = ast.get("rule") {
            walk.rule(rule, &Rel::Root("rule"), &Frame::default(), &Frame::default(), false, &format!("{base}/rule"));
        }
        for (key, tag) in [("constraints", "constraint"), ("when", "when")] {
            if let Some(Value::Object(map)) = ast.get(key) {
                for (var, rule) in map {
                    walk.rule(rule, &Rel::Root(tag), &Frame::default(), &Frame::default(), false, &format!("{base}/{key}/{var}"));
                }
            }
        }
        // A test reached in several contexts: its worst category.
        for test in walk.tests {
            let key = (test.path.clone(), test.relation.clone());
            let rank = |t: &Test| ORDER.iter().position(|c| *c == t.category).unwrap();
            if found.get(&key).is_none_or(|(_, old)| rank(&test) < rank(old) || (rank(&test) == rank(old) && test.implied.len() > old.implied.len())) {
                found.insert(key, (language.to_owned(), test));
            }
        }
    }
    found.into_values().collect()
}

/// Refuses a policy with a token test nothing answers (`UNANSWERED`), naming each.
pub(super) fn refuse(document: &Value) -> napi::Result<()> {
    let unanswered: Vec<String> = tests(document)
        .into_iter()
        .filter(|(_, test)| UNANSWERED.contains(&test.category))
        .map(|(_, test)| format!("/{} (`{}`, {})", test.path, test.regex, test.category))
        .collect();
    if unanswered.is_empty() {
        return Ok(());
    }
    Err(crate::code_rule_dsl::json::invalid(format!(
        "a token test reads a token the tape may leave out, which nothing answers: {}; ask the named structure, or the gaps (`has`, `follows`, `precedes` to the token), or say why the token is stored there (`stored`)",
        unanswered.join(", ")
    )))
}
