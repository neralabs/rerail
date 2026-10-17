//! Randomly generated rules over the in-scope operators, evaluated by both engines over fixture
//! sources in Python, TypeScript and PHP: every case's findings (roots, captures, messages) and
//! failures must be identical.
//!
//! The generator is deterministic: `RERAIL_SQL_DIFF_SEED` (default `0x5e7_c0de`) seeds it and
//! `RERAIL_SQL_DIFF_CASES` (default 500) sets how many compiled cases are compared. Rules the
//! front end rejects (an invalid pattern or kind, an order conflict) are generated but not
//! counted.

use super::snapshot;
use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::code_rule_dsl::language::{GrammarDialect, dialect_by_label};
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::matcher::{UtilLimit, node_at};
use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
use crate::code_rule_dsl::test_support::{CAPABILITIES, clause_document, compile_document_with};
use ast_grep_core::AstGrep;
use rayon::prelude::*;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::Arc;
use std::time::Instant;

/// SplitMix64: small, fast and fixed, so a seed names the same cases everywhere.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound.max(1) as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

/// A short source fragment a pattern is built from: its text, the ranges of its identifier-like
/// leaves and of its argument list's contents.
struct Snippet {
    text: String,
    leaves: Vec<Range<usize>>,
    arguments: Option<Range<usize>>,
}

/// What rules over one language's fixtures are built from.
struct Vocabulary {
    label: &'static str,
    files: Vec<(String, String)>,
    kinds: Vec<String>,
    fields: Vec<String>,
    snippets: Vec<Snippet>,
    regexes: Vec<String>,
}

const FIXTURES: &[(&str, &[&str])] = &[
    (
        "python",
        &[
            "python/scope/usage/F841.py",
            "python/value/ruff/SIM116.py",
            "python/calls/project/pkg/models.py",
        ],
    ),
    (
        "typescript",
        &[
            "ecma/scope/typescript/bindings.ts",
            "ecma/scope/typescript/constants.ts",
            "ecma/value/eslint/no-dupe-keys.ts",
        ],
    ),
    (
        "php",
        &[
            "php/calls/php/trace.php",
            "php/calls/php/project/more.php",
            "php/scope/php/unanalysed.php",
        ],
    ),
];

pub(super) fn tape(dialect: GrammarDialect, source: &str) -> TapeDoc {
    let never = || false;
    let grammar = Grammar::new(dialect).expect("grammar");
    let image = TapeImage::parse(&grammar, source, &never)
        .expect("parsed")
        .expect("tape");
    let lines = Arc::new(LineTable::new(source, Terminators::of(dialect)));
    TapeDoc::restore_with_lines(Arc::new(source.to_owned()), grammar, &image, lines, &never)
        .expect("restored")
}

const LEAF_KINDS: &[&str] = &[
    "identifier",
    "property_identifier",
    "shorthand_property_identifier",
    "type_identifier",
    "variable_name",
    "name",
    "integer",
    "number",
    "true",
    "false",
    "none",
];

const ARGUMENT_KINDS: &[&str] = &[
    "arguments",
    "argument_list",
    "formal_parameters",
    "parameters",
    "array",
    "list",
];

impl Vocabulary {
    fn of(label: &'static str, paths: &[&str]) -> Self {
        let dialect = dialect_by_label(label).expect("dialect");
        let catalog = dialect.kinds();
        let mut kinds = BTreeSet::new();
        let mut fields = BTreeSet::new();
        let mut snippets = Vec::new();
        let mut words = BTreeSet::new();
        let mut files = Vec::new();
        for path in paths {
            let source = std::fs::read_to_string(format!(
                "{}/{path}",
                crate::code_rule_dsl::fixtures_dir!()
            ))
            .unwrap_or_else(|error| panic!("{path}: {error}"));
            let doc = tape(dialect, &source);
            let root = AstGrep::doc(doc.clone());
            for id in 0..doc.transducer_len() as u32 {
                let node = node_at(&root, id);
                if node.is_named() && !node.is_error() {
                    kinds.insert(node.kind().into_owned());
                }
                let field = doc.transducer_field(id);
                if field != 0
                    && let Some(name) = catalog.field_name(field)
                {
                    fields.insert(name.to_owned());
                }
                let text = node.text();
                if node.is_named()
                    && LEAF_KINDS.contains(&node.kind().as_ref())
                    && text.len() <= 12
                    && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                {
                    words.insert(text.to_string());
                }
                let start = node.range().start;
                if node.is_named()
                    && !node.is_error()
                    && (3..=48).contains(&text.len())
                    && !text.contains('\n')
                    && !text.contains('$')
                {
                    let mut leaves = Vec::new();
                    let mut arguments = None;
                    for inner in node.dfs().skip(1) {
                        let range = inner.range();
                        let relative = range.start - start..range.end - start;
                        if inner.is_named()
                            && inner.is_named_leaf()
                            && LEAF_KINDS.contains(&inner.kind().as_ref())
                        {
                            leaves.push(relative.clone());
                        }
                        if arguments.is_none()
                            && ARGUMENT_KINDS.contains(&inner.kind().as_ref())
                            && relative.len() >= 2
                        {
                            arguments = Some(relative.start + 1..relative.end - 1);
                        }
                    }
                    if !leaves.is_empty() || arguments.is_some() {
                        snippets.push(Snippet {
                            text: text.to_string(),
                            leaves,
                            arguments,
                        });
                    }
                }
            }
            files.push((path.to_string(), source));
        }
        let mut regexes: Vec<String> = words
            .iter()
            .step_by(3)
            .take(24)
            .map(|word| format!("^{word}"))
            .collect();
        regexes.extend(
            [
                "^[a-z]", "[0-9]", "^_", "\\(", "^.$", "^\"", "[A-Z]", "\\.", "^$", "=", ",",
            ]
            .map(str::to_owned),
        );
        // PHP sources hold `$`-names: a metavariable written there is a variable, not a capture.
        if label == "php" {
            snippets.retain(|snippet| !snippet.text.contains("$"));
        }
        Self {
            label,
            files,
            kinds: kinds.into_iter().collect(),
            fields: fields.into_iter().collect(),
            snippets,
            regexes,
        }
    }
}

const SINGLE: &[&str] = &["A", "B", "C"];
const LISTS: &[&str] = &["L", "M"];

struct Generator<'v> {
    rng: Rng,
    vocabulary: &'v Vocabulary,
    /// Parameterized utilities and their parameters, as generated for the case.
    calls: Vec<(String, Vec<String>)>,
    plain: Vec<String>,
    /// Captures a parameterized utility's rule may use: its parameters.
    params: Option<Vec<String>>,
    counted: bool,
    /// Whether the case's rule reaches a recursive utility.
    recursive: bool,
}

/// The relations a recursive utility recurses through, one direction per cycle.
const DIRECTIONS: &[&str] = &["has", "inside", "precedes", "follows"];

impl Generator<'_> {
    fn single(&mut self) -> String {
        match &self.params {
            Some(params) if !params.is_empty() => self.rng.pick(params).clone(),
            _ => (*self.rng.pick(SINGLE)).to_owned(),
        }
    }

    fn pattern(&mut self) -> String {
        let snippet = self.rng.pick(&self.vocabulary.snippets);
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();
        if let Some(arguments) = &snippet.arguments
            && self.rng.chance(40)
        {
            let list = if self.rng.chance(30) {
                "$$$".to_owned()
            } else {
                format!("$$${}", self.rng.pick(LISTS))
            };
            edits.push((arguments.clone(), list));
        }
        for leaf in &snippet.leaves {
            if edits
                .iter()
                .any(|(range, _)| range.start < leaf.end && leaf.start < range.end)
            {
                continue;
            }
            if self.rng.chance(45) {
                let name = if self.rng.chance(10) {
                    "_".to_owned()
                } else {
                    self.single()
                };
                edits.push((leaf.clone(), format!("${name}")));
            }
        }
        edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
        let mut text = snippet.text.clone();
        for (range, replacement) in edits {
            text.replace_range(range, &replacement);
        }
        if self.rng.chance(8) {
            format!("${}", self.single())
        } else {
            text
        }
    }

    fn stop(&mut self, depth: usize) -> Value {
        match self.rng.below(10) {
            0..=3 => json!("end"),
            4..=6 => json!("neighbor"),
            _ => self.simple(depth + 1),
        }
    }

    /// A small capture-free rule (stop rules, `ofRule`).
    fn simple(&mut self, _depth: usize) -> Value {
        match self.rng.below(4) {
            0 | 1 => json!({ "kind": self.rng.pick(&self.vocabulary.kinds) }),
            2 => json!({ "regex": self.rng.pick(&self.vocabulary.regexes) }),
            _ => json!({ "any": [
                { "kind": self.rng.pick(&self.vocabulary.kinds) },
                { "kind": self.rng.pick(&self.vocabulary.kinds) },
            ] }),
        }
    }

    fn relation(&mut self, key: &str, depth: usize) -> Value {
        let mut inner = self.rule(depth + 1);
        if self.rng.chance(70) {
            inner["stopBy"] = self.stop(depth);
        }
        if (key == "inside" || key == "has")
            && self.rng.chance(15)
            && !self.vocabulary.fields.is_empty()
        {
            inner["field"] = json!(self.rng.pick(&self.vocabulary.fields));
        }
        inner
    }

    fn rule(&mut self, depth: usize) -> Value {
        let mut rule = Map::new();
        let terms = 1 + self.rng.below(if depth == 0 { 3 } else { 2 });
        for _ in 0..terms {
            let deep = depth < 3;
            let choice = self.rng.below(if deep { 22 } else { 8 });
            match choice {
                0..=2 => {
                    let pattern = self.pattern();
                    rule.insert("pattern".into(), json!(pattern));
                }
                3..=5 => {
                    rule.insert("kind".into(), json!(self.rng.pick(&self.vocabulary.kinds)));
                }
                6 => {
                    rule.insert(
                        "regex".into(),
                        json!(self.rng.pick(&self.vocabulary.regexes)),
                    );
                }
                7 => {
                    let position = match self.rng.below(5) {
                        0 => json!(1 + self.rng.below(3)),
                        1 => json!("2n+1"),
                        2 => json!("-n+2"),
                        3 => json!({ "position": 1 + self.rng.below(2), "reverse": true }),
                        _ => {
                            let of_rule = if self.rng.chance(50) {
                                self.simple(depth + 1)
                            } else {
                                self.rule(depth + 2)
                            };
                            json!({ "position": self.rng.pick(&["1", "2", "n+1", "2n"]), "ofRule": of_rule, "reverse": self.rng.chance(30) })
                        }
                    };
                    rule.insert("nthChild".into(), position);
                }
                8 => {
                    rule.insert(
                        "all".into(),
                        json!([self.rule(depth + 1), self.rule(depth + 1)]),
                    );
                }
                9 | 10 => {
                    let branches = 2 + self.rng.below(2);
                    let any: Vec<Value> = (0..branches).map(|_| self.rule(depth + 1)).collect();
                    rule.insert("any".into(), json!(any));
                }
                11 => {
                    rule.insert("not".into(), self.rule(depth + 1));
                }
                12 => {
                    if !self.calls.is_empty() && self.rng.chance(50) {
                        let (name, params) = self.rng.pick(&self.calls).clone();
                        let args: Map<String, Value> = params
                            .iter()
                            .map(|param| (param.clone(), json!(format!("${}", self.single()))))
                            .collect();
                        rule.insert("matches".into(), json!({ name: args }));
                    } else if !self.plain.is_empty() {
                        rule.insert("matches".into(), json!(self.rng.pick(&self.plain)));
                    }
                }
                13..=18 => {
                    let key = *self.rng.pick(&["inside", "has", "precedes", "follows"]);
                    let relation = self.relation(key, depth);
                    rule.insert(key.into(), relation);
                }
                _ => {
                    let key = *self.rng.pick(&["inside", "has", "precedes", "follows"]);
                    let mut relation = self.rule(depth + 2);
                    if self.rng.chance(70) {
                        relation["stopBy"] = self.stop(depth);
                    }
                    let mut count = Map::new();
                    count.insert(key.into(), relation);
                    let min = self.rng.below(3);
                    if min > 0 {
                        count.insert("min".into(), json!(min));
                    }
                    if min == 0 || self.rng.chance(40) {
                        count.insert("max".into(), json!(min + self.rng.below(3)));
                    }
                    if self.params.is_none() && self.rng.chance(30) {
                        count.insert("as".into(), json!("N"));
                        count.insert("offset".into(), json!(self.rng.below(3) as i64 - 1));
                        self.counted = true;
                    }
                    rule.insert("count".into(), Value::Object(count));
                }
            }
        }
        if rule.is_empty() {
            rule.insert("kind".into(), json!(self.rng.pick(&self.vocabulary.kinds)));
        }
        Value::Object(rule)
    }

    /// A recursive step: `direction`'s relation to a node `reference` matches, perhaps under a
    /// stop, a field or an extra term, perhaps negated or counted.
    fn recursive_step(&mut self, direction: &str, reference: Value) -> Value {
        let mut step = Map::new();
        step.insert("matches".into(), reference);
        if self.rng.chance(30) {
            step.insert("kind".into(), json!(self.rng.pick(&self.vocabulary.kinds)));
        }
        match self.rng.below(10) {
            0..=3 => {
                step.insert("stopBy".into(), json!("end"));
            }
            4..=5 => {
                step.insert("stopBy".into(), json!("neighbor"));
            }
            _ => {}
        }
        if (direction == "has" || direction == "inside")
            && self.rng.chance(15)
            && !self.vocabulary.fields.is_empty()
        {
            step.insert("field".into(), json!(self.rng.pick(&self.vocabulary.fields)));
        }
        let relation = json!({ direction: Value::Object(step.clone()) });
        match self.rng.below(10) {
            0 => json!({ "not": relation }),
            1 => {
                let mut count = Map::new();
                count.insert(direction.into(), Value::Object(step));
                count.insert("min".into(), json!(1 + self.rng.below(2)));
                json!({ "count": count })
            }
            _ => relation,
        }
    }

    /// A utility that recurses through relations in one direction (now and then two, whose reads
    /// may loop back): its own rule, or a call's with parameters.
    fn recursive_util(&mut self, params: Option<Vec<String>>, reference: Value) -> Value {
        let direction = *self.rng.pick(DIRECTIONS);
        self.params = params.clone();
        let mut base = self.rule(2);
        if let Some(params) = &params
            && self.rng.chance(70)
        {
            base["pattern"] = json!(format!("${}", params[0]));
        }
        let mut branches = vec![base, self.recursive_step(direction, reference.clone())];
        if self.rng.chance(6) {
            let other = *self.rng.pick(DIRECTIONS);
            branches.push(self.recursive_step(other, reference));
        }
        let body = match self.rng.below(10) {
            0 | 1 => {
                // A conjunction: the recursion holds only beside another term.
                let mut all = Map::new();
                all.insert("all".into(), json!(branches));
                Value::Object(all)
            }
            _ => json!({ "any": branches }),
        };
        self.params = None;
        match params {
            Some(params) => json!({ "params": params, "rule": body }),
            None => body,
        }
    }

    /// One case: an `ast` query (rule, utilities, constraints) and its violation.
    fn case(&mut self) -> (Value, Value) {
        self.calls.clear();
        self.plain.clear();
        self.counted = false;
        self.recursive = false;
        let mut utils = Map::new();
        let mut recursive = None;
        if self.rng.chance(35) {
            match self.rng.below(4) {
                0 => {
                    // A parameterized utility calling itself.
                    let params = vec!["P".to_owned()];
                    let reference = json!({ "r0": { "P": "$P" } });
                    let util = self.recursive_util(Some(params.clone()), reference);
                    utils.insert("r0".into(), util);
                    self.calls.push(("r0".into(), params));
                    recursive = Some(json!({ "r0": { "P": format!("${}", self.single()) } }));
                }
                1 => {
                    // Two utilities: `r1` meets `r0` at its own node, `r0` reaches `r1` through
                    // a relation.
                    let util = self.recursive_util(None, json!("r1"));
                    utils.insert("r0".into(), util);
                    let base = self.rule(2);
                    utils.insert(
                        "r1".into(),
                        json!({ "any": [base, { "matches": "r0", "kind": self.rng.pick(&self.vocabulary.kinds) }] }),
                    );
                    self.plain.push("r0".into());
                    recursive = Some(json!("r0"));
                }
                _ => {
                    let util = self.recursive_util(None, json!("r0"));
                    utils.insert("r0".into(), util);
                    self.plain.push("r0".into());
                    recursive = Some(json!("r0"));
                }
            }
        }
        for index in 0..self.rng.below(3) {
            let name = format!("u{index}");
            if self.rng.chance(50) {
                let params: Vec<String> = if self.rng.chance(70) {
                    vec!["P".into()]
                } else {
                    vec!["P".into(), "Q".into()]
                };
                self.params = Some(params.clone());
                let mut rule = self.rule(2);
                if self.rng.chance(70) {
                    rule["pattern"] = json!(format!("${}", params[0]));
                }
                self.params = None;
                utils.insert(name.clone(), json!({ "params": params, "rule": rule }));
                self.calls.push((name, params));
            } else {
                let rule = self.rule(2);
                utils.insert(name.clone(), rule);
                self.plain.push(name);
            }
        }
        let mut ast = Map::new();
        ast.insert("language".into(), json!(self.vocabulary.label));
        let mut rule = self.rule(0);
        if let Some(reference) = recursive
            && self.rng.chance(85)
        {
            rule["matches"] = reference;
        }
        self.recursive = serde_json::to_string(&rule)
            .is_ok_and(|text| text.contains("\"r0\""));
        ast.insert("rule".into(), rule);
        if !utils.is_empty() {
            ast.insert("utils".into(), Value::Object(utils));
        }
        if self.rng.chance(15) {
            let name = self.single();
            ast.insert("constraints".into(), json!({ name: self.rule(2) }));
        }
        let message = if self.counted { "n=$N" } else { "x" };
        let mut violation = json!({ "anchor": "witness.span", "code": "X", "message": message });
        // `violation.each` over a capture the rule names, now and then.
        let text = serde_json::to_string(&ast).unwrap_or_default();
        let named: Vec<&str> = SINGLE
            .iter()
            .copied()
            .filter(|name| {
                text.match_indices(&format!("${name}"))
                    .any(|(at, _)| !text[..at].ends_with('$'))
            })
            .collect();
        if !named.is_empty() && self.rng.chance(30) {
            violation["each"] = json!(*self.rng.pick(&named));
        }
        (Value::Object(ast), violation)
    }
}

enum Outcome {
    /// The front end refused the rule.
    Rejected,
    /// The native matcher hit a utility limit: its depth limit (256 levels or 1 MiB of stack,
    /// which depends on the build) or its work budget, limits the SQL engine does not emulate.
    NativeLimit,
    /// The SQL engine refused an operator.
    Unsupported(String),
    /// Both engines answered alike, with this many violations.
    Same(usize),
    Differs {
        native: String,
        sql: String,
    },
}

fn run_case(ast: &Value, violation: &Value, input: &Value, capabilities: &Value) -> Outcome {
    // A case known to take hours on the SQL engine (`RERAIL_SQL_DIFF_SKIP`: a fragment of its
    // rule's JSON) is skipped, and counted.
    if let Ok(fragment) = std::env::var("RERAIL_SQL_DIFF_SKIP")
        && !fragment.is_empty()
        && ast.to_string().contains(&fragment)
    {
        eprintln!("skipped case: {ast}");
        return Outcome::Rejected;
    }
    let document = clause_document(
        json!({ "id": "ast-match", "forbid": { "ast": ast }, "violation": violation }),
    );
    let Ok(compiled): napi::Result<CompiledCodeRuleDsl> =
        compile_document_with(document, capabilities)
    else {
        return Outcome::Rejected;
    };
    let evaluate = |engine: &str| {
        compiled
            .evaluate(
                input.to_string(),
                "report".to_owned(),
                Some(engine.to_owned()),
            )
            .map_err(|error| error.reason)
    };
    let started = Instant::now();
    let native = evaluate("native");
    let native_time = started.elapsed().as_secs_f64();
    let sql = evaluate("sql");
    let sql_time = started.elapsed().as_secs_f64() - native_time;
    if native_time + sql_time > 10.0 {
        eprintln!("slow case: native {native_time:.1}s, sql {sql_time:.1}s: {}", ast.to_string().chars().take(800).collect::<String>());
    }
    if native.as_ref().is_ok_and(|output| {
        output.contains(UtilLimit::Depth.message()) || output.contains(UtilLimit::Work.message())
    }) {
        return Outcome::NativeLimit;
    }
    match (&native, &sql) {
        (_, Err(reason)) if reason.contains("Unsupported(") => Outcome::Unsupported(reason.clone()),
        _ if native == sql => Outcome::Same(
            native
                .ok()
                .and_then(|output| serde_json::from_str::<Value>(&output).ok())
                .and_then(|result| result["violations"].as_array().map(Vec::len))
                .unwrap_or(0),
        ),
        _ => Outcome::Differs {
            native: format!("{native:?}"),
            sql: format!("{sql:?}"),
        },
    }
}

/// Counts the operators a rule uses, by key (`stopBy` by its kind, `matches` by its form).
fn census(value: &Value, counts: &mut std::collections::BTreeMap<String, usize>) {
    match value {
        Value::Object(object) => {
            for (key, item) in object {
                let name = match (key.as_str(), item) {
                    ("stopBy", Value::String(stop)) => format!("stopBy:{stop}"),
                    ("stopBy", _) => "stopBy:rule".to_owned(),
                    ("matches", Value::Object(_)) => "matches:call".to_owned(),
                    ("matches", _) => "matches:shared".to_owned(),
                    ("nthChild", Value::Object(nth)) if nth.contains_key("ofRule") => {
                        "nthChild:ofRule".to_owned()
                    }
                    (
                        "language" | "params" | "position" | "reverse" | "min" | "max" | "offset",
                        _,
                    ) => {
                        continue;
                    }
                    _ => key.clone(),
                };
                *counts.entry(name).or_default() += 1;
                if key != "utils" && key != "constraints" {
                    census(item, counts);
                } else if let Value::Object(items) = item {
                    for rule in items.values() {
                        census(rule, counts);
                    }
                }
            }
        }
        Value::Array(items) => items.iter().for_each(|item| census(item, counts)),
        _ => {}
    }
}

fn env_number(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .map(|value| {
            let value = value.trim();
            match value.strip_prefix("0x") {
                Some(hex) => u64::from_str_radix(hex, 16),
                None => value.parse(),
            }
            .unwrap_or_else(|_| panic!("{name} must be a number"))
        })
        .unwrap_or(default)
}

#[test]
fn generated_rules_agree_with_the_native_matcher() {
    let seed = env_number("RERAIL_SQL_DIFF_SEED", 0x5e7_c0de);
    let wanted = env_number("RERAIL_SQL_DIFF_CASES", 500) as usize;
    let started = Instant::now();
    let vocabularies: Vec<Vocabulary> = FIXTURES
        .iter()
        .map(|(label, paths)| Vocabulary::of(label, paths))
        .collect();
    let inputs: Vec<Value> = vocabularies
        .iter()
        .map(|vocabulary| {
            let files: Vec<(&str, &str, &str)> = vocabulary
                .files
                .iter()
                .map(|(path, source)| (vocabulary.label, path.as_str(), source.as_str()))
                .collect();
            snapshot(&files)
        })
        .collect();
    let mut capabilities: Value = serde_json::from_str(CAPABILITIES).expect("capabilities");
    capabilities["limits"]["maxWitnesses"] = json!(1_000_000);
    capabilities["limits"]["maxVisitedNodes"] = json!(1_000_000_000);
    let mut generators: Vec<Generator<'_>> = vocabularies
        .iter()
        .enumerate()
        .map(|(index, vocabulary)| Generator {
            rng: Rng(seed ^ (index as u64).wrapping_mul(0x2545_f491_4f6c_dd1d)),
            vocabulary,
            calls: Vec::new(),
            plain: Vec::new(),
            params: None,
            counted: false,
            recursive: false,
        })
        .collect();
    let (mut compared, mut rejected, mut generated) = (0usize, 0usize, 0usize);
    let (mut with_violations, mut violations) = (0usize, 0usize);
    let mut operators = std::collections::BTreeMap::new();
    let (mut with_recursion, mut with_each, mut with_either, mut native_limits) =
        (0usize, 0usize, 0usize, 0usize);
    let mut unsupported = Vec::new();
    let mut differences = Vec::new();
    while compared < wanted {
        let batch: Vec<(usize, Value, Value, bool)> = (0..256)
            .map(|index| {
                let language = (generated + index) % generators.len();
                let (ast, violation) = generators[language].case();
                (language, ast, violation, generators[language].recursive)
            })
            .collect();
        generated += batch.len();
        let outcomes: Vec<Outcome> = batch
            .par_iter()
            .map(|(language, ast, violation, _)| {
                run_case(ast, violation, &inputs[*language], &capabilities)
            })
            .collect();
        for ((_, ast, violation, recursive), outcome) in batch.iter().zip(outcomes) {
            if compared >= wanted {
                break;
            }
            if !matches!(outcome, Outcome::Rejected | Outcome::NativeLimit) {
                census(ast, &mut operators);
                let each = violation.get("each").is_some();
                with_recursion += usize::from(*recursive);
                with_each += usize::from(each);
                with_either += usize::from(*recursive || each);
            }
            match outcome {
                Outcome::Rejected => rejected += 1,
                Outcome::NativeLimit => native_limits += 1,
                Outcome::Unsupported(reason) => {
                    compared += 1;
                    unsupported.push((ast.clone(), reason));
                }
                Outcome::Same(found) => {
                    compared += 1;
                    violations += found;
                    with_violations += usize::from(found > 0);
                }
                Outcome::Differs { native, sql } => {
                    compared += 1;
                    differences.push((ast.clone(), native, sql));
                }
            }
        }
    }
    eprintln!(
        "sql differential: seed {seed:#x}, {compared} cases compared ({generated} generated, {rejected} rejected by the front end), {} differences, {} unsupported, {with_violations} cases with violations ({violations} violations), {:.1}s",
        differences.len(),
        unsupported.len(),
        started.elapsed().as_secs_f64()
    );
    eprintln!(
        "recursion or each: {with_either} compared cases ({with_recursion} reaching a recursive utility, {with_each} under violation.each); {native_limits} cases not compared: the native matcher hit a utility limit (depth or work)"
    );
    eprintln!("operators in compared cases: {operators:?}");
    let mut refusals = std::collections::BTreeMap::<String, usize>::new();
    for (_, reason) in &unsupported {
        let name = reason
            .split("Unsupported(")
            .nth(1)
            .and_then(|rest| rest.split(')').next())
            .unwrap_or(reason);
        *refusals.entry(name.to_owned()).or_default() += 1;
    }
    eprintln!("refusals: {refusals:?}");
    for (ast, reason) in unsupported.iter().take(5) {
        eprintln!("unsupported: {reason}\n  {ast}");
    }
    for (ast, native, sql) in differences.iter().take(5) {
        let cut = |text: &str| text.chars().take(1500).collect::<String>();
        eprintln!(
            "difference:\n  rule: {ast}\n  native: {}\n  sql: {}",
            cut(native),
            cut(sql)
        );
    }
    // The generator covers in-scope operators only; the shapes the engine refuses within them
    // are `lower::walk_leak`'s, a stop rule that recurses, and a recursion whose reads loop in
    // the fixtures (where the matcher would recurse until its depth limit, or around the loop
    // by stopping at a first match), and it must refuse them rather than answer.
    let refused = |reason: &String| {
        [
            "Unsupported(nthChild with a binding ofRule as a relation's rule)",
            "Unsupported(recursion whose reads loop back)",
            "Unsupported(recursion deeper than 256 levels)",
            "Unsupported(recursive utility in a stop rule)",
        ]
        .iter()
        .any(|refusal| reason.contains(refusal))
    };
    assert!(
        differences.is_empty() && unsupported.iter().all(|(_, reason)| refused(reason)),
        "{} differences, {} unsupported",
        differences.len(),
        unsupported.len()
    );
}
