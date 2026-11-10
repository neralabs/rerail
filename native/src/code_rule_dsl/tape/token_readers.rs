//! D (RER-131, t31): every pack rule's tests of a token, sorted by what named-only tapes do to
//! them. A test whose token the grammar implies where the rule looks can no longer hold: the token
//! is in no tape.
//!
//! A *token test* is a sub-rule with a `regex` and no `kind` or `pattern` on its node, whose regex
//! matches some token of the dialect's grammar. Where it sits says where the token would be:
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
//! A regex that matches a token without spanning it (`^[rub]*f`), or matches any name (`.`),
//! tests a node's text and reached a token only by accident: `incidental`. A test whose token is never implied in its
//! parents is `stored`: it reads a token the tape
//! keeps, and needs nothing. A test under a `not` on the token's own node, reached as the node
//! matched, a child or any earlier or later sibling, only kept tokens out of that set, and an
//! implied token is in none: `excluder`, which needs nothing either. (A neighbour sibling is one
//! node, which is now another one: it stays a `gap`.) `gap?` is a `gap` whose parent the rule does not say: its token is
//! implied in some kind, maybe not the one the rule is under. A parent is known from the rule (`kind` on the node, or on its
//! neighbour `inside`); otherwise every kind the grammar has the node under, or every kind.

use super::Grammar;
use super::tokens::TokenGrammar;
use crate::code_rule_dsl::language::{GrammarDialect, dialect_by_label};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// One dialect's automata, as the census asks them.
struct Lang {
    implied: BTreeMap<String, BTreeSet<(String, String)>>,
    parents: BTreeMap<String, BTreeSet<String>>,
    tokens: BTreeSet<String>,
}

impl Lang {
    fn of(lang: GrammarDialect) -> Option<Self> {
        let language = Grammar::new(lang).ok()?.kinds.grammar()?.clone();
        let grammar = TokenGrammar::of(lang, &language)?;
        Some(Self { implied: grammar.implied_tokens(), parents: grammar.parents(), tokens: grammar.token_texts() })
    }

    /// The parents among `parents` (all kinds where `None`) where a token `regex` matches is
    /// implied, under `field` where one is given: (parent, token) pairs.
    fn implied_in(&self, parents: Option<&BTreeSet<String>>, regex: &regex::Regex, field: Option<&str>) -> BTreeSet<(String, String)> {
        let mut out = BTreeSet::new();
        let kinds: Box<dyn Iterator<Item = &String>> = match parents {
            Some(parents) => Box::new(parents.iter()),
            None => Box::new(self.implied.keys()),
        };
        for kind in kinds {
            for (token, token_field) in self.implied.get(kind).into_iter().flatten() {
                if regex.is_match(token) && field.is_none_or(|field| field == token_field) {
                    out.insert((kind.clone(), token.clone()));
                }
            }
        }
        out
    }
}

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
struct Test {
    category: &'static str,
    negated: bool,
    relation: String,
    regex: String,
    /// (parent, token) where the token is implied.
    implied: BTreeSet<(String, String)>,
    path: String,
}

struct Walk<'a> {
    lang: &'a Lang,
    utils: &'a Map<String, Value>,
    tests: BTreeSet<Test>,
    active: Vec<String>,
    /// A util already walked in a context (its name and the context, printed): walked once.
    seen: BTreeSet<String>,
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
            self.token_test(regex, rel, from, &frame, negated, path);
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

    fn token_test(&mut self, regex: &str, rel: &Rel, from: &Frame, frame: &Frame, negated: bool, path: &str) {
        let Ok(compiled) = regex::Regex::new(regex) else { return };
        let Ok(whole) = regex::Regex::new(&format!("^(?:{regex})$")) else { return };
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
            Rel::Root(tag) => ("root", (*tag).to_owned(), self.lang.implied_in(None, &compiled, None)),
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
        let category = if category != "never" && implied.is_empty() {
            "stored"
        } else if negated && (category == "root" || (category != "never" && !matches!(rel, Rel::Sibling { end: false, .. }))) {
            // A `not` on the token's own node, over a set (the node matched, a node's children,
            // its earlier or later siblings): it kept tokens out of the set, and an implied token
            // is in none. Not a neighbour sibling, which is one node and now another one.
            "excluder"
        } else if incidental && category != "never" {
            "incidental"
        } else {
            category
        };
        self.tests.insert(Test { category, negated, relation, regex: regex.to_owned(), implied, path: path.to_owned() });
    }

    fn layout(&mut self, side: &str, frame: &Frame, negated: bool, path: &str) {
        let parents = frame.parent.clone().map(|kind| BTreeSet::from([kind])).or_else(|| frame.kind.as_ref().and_then(|kind| self.lang.parents.get(kind)).cloned());
        let any = regex::Regex::new(".").unwrap();
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
const ORDER: [&str; 9] = ["root", "below", "layout", "gap", "gap?", "incidental", "excluder", "stored", "never"];

/// RER-131 D (ignored): every token test of every pack rule, its category, and where its token is
/// implied, to `RERAIL_TOKEN_READERS` (a TSV); a summary per pack and category on stderr.
#[test]
#[ignore = "census: RERAIL_TOKEN_READERS=<tsv>"]
fn token_readers() {
    let out = std::env::var("RERAIL_TOKEN_READERS").expect("RERAIL_TOKEN_READERS");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../packs");
    let mut langs: BTreeMap<String, Option<Lang>> = BTreeMap::new();
    let mut tsv = String::from("pack\trule\tlanguage\tcategory\tnegated\trelation\tregex\timplied (parent token)\tpath\n");
    // (pack, category) -> (tests, rules by worst category)
    let mut tests_by: BTreeMap<(String, &str), usize> = BTreeMap::new();
    let mut rules_by: BTreeMap<(String, &str), usize> = BTreeMap::new();
    let mut packs: Vec<_> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().path()).filter(|p| p.is_dir()).collect();
    packs.sort();
    for pack in packs {
        let pack_name = pack.file_name().unwrap().to_string_lossy().into_owned();
        for sub in ["rules", "candidates/rules"] {
            let Ok(entries) = std::fs::read_dir(pack.join(sub)) else { continue };
            let mut files: Vec<_> = entries.map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
            files.sort();
            let label = if sub == "rules" { pack_name.clone() } else { format!("{pack_name}/candidates") };
            for file in files {
                let document: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
                let name = file.file_stem().unwrap().to_string_lossy().into_owned();
                let clauses = document.get("clauses").and_then(Value::as_array).cloned().unwrap_or_default();
                let mut utils = Map::new();
                for clause in &clauses {
                    for found in [clause.pointer("/define/utils"), clause.pointer("/forbid/ast/utils")].into_iter().flatten() {
                        if let Some(map) = found.as_object() {
                            utils.extend(map.clone());
                        }
                    }
                }
                let mut worst: Option<usize> = None;
                let mut found: BTreeMap<(String, String), (String, Test)> = BTreeMap::new();
                for (at, clause) in clauses.iter().enumerate() {
                    let Some(ast) = clause.pointer("/forbid/ast").and_then(Value::as_object) else { continue };
                    let Some(language) = ast.get("language").and_then(Value::as_str) else { continue };
                    let Some(lang) = langs.entry(language.to_owned()).or_insert_with(|| dialect_by_label(language).and_then(Lang::of)).as_ref() else { continue };
                    let mut walk = Walk { lang, utils: &utils, tests: BTreeSet::new(), active: Vec::new(), seen: BTreeSet::new() };
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
                for (language, test) in found.into_values() {
                    let implied: Vec<String> = test.implied.iter().take(8).map(|(parent, token)| format!("{parent} {token}")).collect();
                    let more = if test.implied.len() > 8 { format!(" +{}", test.implied.len() - 8) } else { String::new() };
                    writeln!(tsv, "{label}\t{name}\t{language}\t{}\t{}\t{}\t{}\t{}{more}\t{}", test.category, test.negated, test.relation, test.regex, implied.join(", "), test.path).unwrap();
                    *tests_by.entry((label.clone(), test.category)).or_default() += 1;
                    let rank = ORDER.iter().position(|c| *c == test.category).unwrap();
                    worst = Some(worst.map_or(rank, |w| w.min(rank)));
                }
                if let Some(rank) = worst {
                    *rules_by.entry((label.clone(), ORDER[rank])).or_default() += 1;
                }
            }
        }
    }
    std::fs::write(&out, tsv).unwrap();
    let labels: BTreeSet<&String> = rules_by.keys().map(|(pack, _)| pack).collect();
    eprintln!("rules by worst category (tests in brackets):");
    for label in labels {
        let row: Vec<String> = ORDER
            .iter()
            .map(|c| format!("{c} {} ({})", rules_by.get(&(label.clone(), *c)).unwrap_or(&0), tests_by.get(&(label.clone(), *c)).unwrap_or(&0)))
            .collect();
        eprintln!("  {label}: {}", row.join(", "));
    }
    let total = |c: &str, map: &BTreeMap<(String, &str), usize>| map.iter().filter(|((_, k), _)| *k == c).map(|(_, n)| n).sum::<usize>();
    let row: Vec<String> = ORDER.iter().map(|c| format!("{c} {} ({})", total(c, &rules_by), total(c, &tests_by))).collect();
    eprintln!("  all: {}", row.join(", "));
}
