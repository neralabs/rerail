//! Rules through the logic language (list step 21; 03 §12, 05 Phase 6 rows 3–4).
//!
//! A pack's policies become one logic program: each document goes through `logic::front::yaml`, so a forbid
//! clause is a rule `violation("<policy>", "<clause>", $D, $N) :- ast($D, $N, {…})`, and the documents the
//! program gives back must equal the ones read. That is the round trip, and it runs on the path. A document
//! that does not survive it is refused, not lowered beside it.
//!
//! The wave runs the program's rules, not a list of clauses:
//! - each `ast` atom whose Python matcher lowers is a clause of the wave (its index `i`), and the atom reads
//!   `matched(i, D, N)`, the wave's terminal matches;
//! - the rewritten rules join the wave's source (`Logic::rules`), so `violation` is derived as the rules say
//!   and the findings layer reads it;
//! - a rule with an `ast` atom that does not lower is left out (its clause is not covered, as before).
//!
//! **Demand per wave (C10; magic sets).** Each lowered atom is its own relation `ast_i(d, n)`, keyed by
//! document, and `.demand` names every one. Each chunk's documents seed the demand: each rule reading an atom
//! starts with `wave($D)`, the chunk's documents (`temp.wave_document`). `logic::magic::magic_sets` gives the
//! magic rules, so `magic_ast_i(d)` is the documents clause `i` is needed on. What the magic rules read that
//! the wave cannot have before its batch (another atom, or a relation derived from one) is dropped from them,
//! which only widens demand. The rest is planned into one statement (`emit::single`) deriving
//! `demanded(i, d)`. Before each chunk's batch is registered, that statement is run, and a clause's root
//! programs start only on the documents it names (`Wave::demand`). A demand program that needs recursion or
//! setup is not planned: every clause is demanded on every document, counted as `wave.demand.all`.
//!
//! **Rules of one shape are one rule (`merged`).** A pack's forbid rules differ mostly in their constants:
//! `violation("p", "c", $D, $N) :- match(3, $D, $N)` and the next clause's. Each rule's constants in its head
//! and positive atoms become variables, read from a constant table of the shape, so the rules that are equal
//! once their variables are renamed in order are one rule
//! `violation(K0, K1, V0, V1) :- violation_shape_0(K0, K1, K2), matched(K2, V0, V1)` and one row
//! each. The wave plans one statement per shape and one for the table, not one per rule (about 1,300 per
//! chunk on the data-layer pack before). A shape of one rule is kept as written.

use super::findings::Shape;
use super::matches::Clause;
use crate::code_rule_dsl::CompiledCodeRuleDsl;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::logic::front::{self, Atom, Literal, Program, Rule, Term, Value, yaml};
use crate::code_rule_dsl::logic::magic;
use crate::code_rule_dsl::logic::plan::emit;
use crate::code_rule_dsl::logic::sql::select::Source;
use crate::code_rule_dsl::matcher::gated_match_tree;
use crate::code_rule_dsl::model::{ClauseKind, Query};
use crate::code_rule_dsl::transducer::packed::LoweredMatcher;
use crate::content_store::{ContentStore, StoreError, StoreResult};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// The relation the wave's matches are in (`matches::SOURCE`).
const MATCH: &str = "matched";
/// The chunk's documents in the demand program, and what it derives.
const WAVE: &str = "wave";
const DEMANDED: &str = "demanded";

fn invalid(error: impl std::fmt::Display) -> StoreError {
    StoreError::Invalid(error.to_string())
}

/// The policies' rules as the wave runs them.
#[derive(Clone, Debug, Default)]
pub(in crate::code_rule_dsl) struct Logic {
    /// Datalog appended to the wave's source: the relations' declarations, `.output`, and the rules over
    /// `match`.
    pub(in crate::code_rule_dsl) rules: String,
    /// The demand program: the magic rules seeded by `wave(d)`, deriving `demanded(i, d)`. `None`: every
    /// clause is demanded on every document.
    pub(in crate::code_rule_dsl) demand: Option<Program>,
    /// The relations the rules read as bound with the chunk (`is_bound`), with their columns: the
    /// wave reads them as the chunk's `main` tables.
    pub(in crate::code_rule_dsl) bound: Vec<(String, &'static [&'static str])>,
}

impl Logic {
    /// Each clause as its own forbid rule, `violation(<policy>, <clause>, d, n) :- matched(i, d, n)`,
    /// demanded everywhere: the program of clauses given without their documents (tests, `rir` dumps of
    /// one clause).
    pub(in crate::code_rule_dsl) fn per_clause(clauses: &[Clause]) -> Self {
        let mut prog = Program::default();
        for (i, clause) in clauses.iter().enumerate() {
            let (policy, id) = &clause.key;
            let head = Atom { rel: "violation".to_owned(), args: vec![Term::Str(policy.clone()), Term::Str(id.clone()), var("D"), var("N")], neg: false, gated: false };
            prog.rules.push(Rule { head, body: vec![matched(i, "D", "N")] });
        }
        declare_violation(&mut prog);
        Self { rules: wave_text(&merged(prog)).expect("a clause's forbid rule prints as .rr"), demand: None, bound: Vec::new() }
    }

    /// The demand statement on `store` (its atoms interned there): a SELECT of `(i, d)` rows over
    /// `temp.wave_document`, or `None` where every clause is demanded.
    pub(in crate::code_rule_dsl) fn demand_sql(&self, store: &ContentStore) -> StoreResult<Option<String>> {
        let statement = self.demand_statement(store).ok().flatten();
        if self.demand.is_some() && statement.is_none() {
            crate::code_rule_dsl::prof::count("wave.demand.all", 1);
        }
        Ok(statement)
    }

    /// `demand_sql`'s statement, or why it has none.
    fn demand_statement(&self, store: &ContentStore) -> Result<Option<String>, String> {
        let Some(prog) = &self.demand else { return Ok(None) };
        let mut batch = HashMap::from([(WAVE.to_owned(), (Source::positional("temp.wave_document", &["d"]), 100.0))]);
        // The `py.*` families are read where they are bound (`main`), as the chunk's base relations are,
        // each priced at a thousand rows: no statistics exist before the chunk's first read (decision 0029).
        for rel in prog.inputs.iter().filter(|rel| is_bound(rel)) {
            let columns = crate::code_rule_dsl::logic::sql::select::input_columns(rel).expect("a bound relation's columns");
            batch.insert(rel.clone(), (Source::named(&format!("main.{rel}"), columns), 1000.0));
        }
        let intern = &mut |names: &[&str]| store.intern_atoms(names).map_err(|e| crate::code_rule_dsl::logic::plan::Error::Plan(e.to_string()));
        let emitted = emit::emit_program(prog, "demand", &batch, intern).and_then(|emitted| emit::single(emitted, DEMANDED)).map_err(|e| e.to_string())?;
        let sql = &emitted.program.steps[0].sql;
        // The emitter always sets up `rdl_int`; a statement that reads no setup needs none.
        let named = |statement: &str| statement.split(['(', ' ']).nth(3).map(str::to_owned).unwrap_or_default();
        if let Some(needed) = emitted.setup.iter().find(|statement| sql.contains(&named(statement))) {
            return Err(format!("the demand program needs setup: {needed}"));
        }
        sql.strip_prefix(&format!("INSERT OR IGNORE INTO new_{DEMANDED} ")).map(|sql| Some(sql.to_owned())).ok_or_else(|| format!("not an insert: {sql}"))
    }
}

fn var(name: &str) -> Term {
    Term::Var(name.to_owned())
}

fn atom(rel: &str, args: Vec<Term>) -> Literal {
    Literal::Atom(Atom { rel: rel.to_owned(), args, neg: false, gated: false })
}

/// `matched(i, d, n)`: clause `i`'s matches.
fn matched(i: usize, document: &str, node: &str) -> Literal {
    atom(MATCH, vec![Term::Int(i as i64), var(document), var(node)])
}

/// Whether `rel` is one of the `py.*` families (`facts::vtab::syntax::PY`).
fn is_py(rel: &str) -> bool {
    crate::code_rule_dsl::facts::vtab::syntax::PY.contains(&rel)
}

/// Declares each `py.*` family the rules read and do not declare: integer columns, as bound.
fn declare_py(prog: &mut Program) {
    use crate::code_rule_dsl::facts::vtab::syntax::{PY, PY_COLUMNS};
    let read: BTreeSet<String> = prog.rules.iter().flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.clone())).filter(|rel| is_py(rel)).collect();
    for rel in read {
        let columns = PY_COLUMNS[PY.iter().position(|p| *p == rel).expect("a py family")];
        prog.decls.entry(rel).or_insert_with(|| columns.iter().map(|c| ((*c).to_owned(), "int".to_owned())).collect());
    }
}

/// Declares each base relation the rules read and do not declare (`BASE`), as bound.
fn declare_base(prog: &mut Program) {
    let read: BTreeSet<String> = prog.rules.iter().flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.clone())).collect();
    for (rel, columns) in BASE.iter().filter(|(rel, _)| read.contains(*rel)) {
        prog.decls.entry((*rel).to_owned()).or_insert_with(|| columns.iter().map(|(c, t)| ((*c).to_owned(), (*t).to_owned())).collect());
    }
}

fn declare_violation(prog: &mut Program) {
    prog.decls.entry("violation".to_owned()).or_insert_with(|| {
        [("policy", "sym"), ("clause", "sym"), ("d", "int"), ("n", "int")].map(|(c, t)| (c.to_owned(), t.to_owned())).to_vec()
    });
    if !prog.outputs.iter().any(|o| o == "violation") {
        prog.outputs.push("violation".to_owned());
    }
}

/// The rules' text for the wave, in `.rr`: declarations (nodes are the wave's integers), outputs, rules, facts.
fn wave_text(prog: &Program) -> Result<String, crate::code_rule_dsl::logic::front::Error> {
    let decls = prog.decls.iter().map(|(rel, cols)| (rel.clone(), cols.iter().map(|(c, t)| (c.clone(), if t == "node" { "int".to_owned() } else { t.clone() })).collect())).collect();
    let printed = Program { decls, outputs: prog.outputs.clone(), rules: prog.rules.clone(), facts: prog.facts.clone(), ..Program::default() };
    crate::code_rule_dsl::logic::rr::print(&printed)
}

/// A rule with its variables renamed `V0, V1, …` in order of appearance, and its constants in the head and in
/// positive atoms replaced by `K0, K1, …`: the rule's shape, and the constants it was written with.
fn shape(rule: &Rule) -> (Rule, Vec<Value>) {
    let mut names = HashMap::<String, String>::new();
    let mut rename = |v: &mut String| {
        let next = names.len();
        *v = names.entry(v.clone()).or_insert_with(|| format!("V{next}")).clone();
    };
    fn term(t: &mut Term, rename: &mut dyn FnMut(&mut String)) {
        match t {
            Term::Var(v) => rename(v),
            Term::Call(_, args) => args.iter_mut().for_each(|a| term(a, rename)),
            _ => {}
        }
    }
    let mut constants = Vec::new();
    let abstracted = |args: &mut Vec<Term>, rename: &mut dyn FnMut(&mut String), constants: &mut Vec<Value>| {
        for arg in args {
            let value = match arg {
                Term::Int(i) => Value::Int(*i),
                Term::Str(s) => Value::Str(s.clone()),
                other => {
                    term(other, rename);
                    continue;
                }
            };
            *arg = Term::Var(format!("K{}", constants.len()));
            constants.push(value);
        }
    };
    let mut out = rule.clone();
    abstracted(&mut out.head.args, &mut rename, &mut constants);
    for literal in &mut out.body {
        match literal {
            Literal::Atom(a) if !a.neg && !a.gated => abstracted(&mut a.args, &mut rename, &mut constants),
            Literal::Atom(a) => a.args.iter_mut().for_each(|t| term(t, &mut rename)),
            Literal::Cmp { l, r, .. } => {
                term(l, &mut rename);
                term(r, &mut rename);
            }
            Literal::Agg { var, over, atom, .. } => {
                atom.args.iter_mut().for_each(|t| term(t, &mut rename));
                rename(var);
                if let Some(over) = over {
                    rename(over);
                }
            }
            Literal::Nearest { n, a, .. } => {
                rename(n);
                rename(a);
            }
            Literal::Ast { document, node, .. } => {
                rename(document);
                rename(node);
            }
        }
    }
    (out, constants)
}

/// `prog`'s rules of one shape (`shape`) as one rule over the shape's constant table, `<head>_shape_<k>`,
/// with a row per rule. A shape of one rule, or of none of its constants, is kept as written.
fn merged(mut prog: Program) -> Program {
    let kind = |v: &Value| if matches!(v, Value::Int(_)) { "int" } else { "sym" };
    let mut groups: Vec<(Rule, Vec<Vec<Value>>, Vec<Rule>)> = Vec::new();
    let mut by_key = HashMap::<String, usize>::new();
    for rule in std::mem::take(&mut prog.rules) {
        let (shaped, constants) = shape(&rule);
        let key = format!("{shaped} / {}", constants.iter().map(kind).collect::<Vec<_>>().join(","));
        let at = *by_key.entry(key).or_insert_with(|| {
            groups.push((shaped, Vec::new(), Vec::new()));
            groups.len() - 1
        });
        groups[at].1.push(constants);
        groups[at].2.push(rule);
    }
    let mut tables = 0;
    for (shaped, rows, rules) in groups {
        if rules.len() < 2 || rows[0].is_empty() {
            prog.rules.extend(rules);
            continue;
        }
        let rel = loop {
            let rel = format!("{}_shape_{tables}", shaped.head.rel);
            tables += 1;
            if !prog.decls.contains_key(&rel) {
                break rel;
            }
        };
        let columns = rows[0].iter().enumerate().map(|(j, v)| (format!("k{j}"), kind(v).to_owned())).collect();
        prog.decls.insert(rel.clone(), columns);
        let args = (0..rows[0].len()).map(|j| var(&format!("K{j}"))).collect();
        prog.facts.insert(rel.clone(), rows);
        let body = std::iter::once(atom(&rel, args)).chain(shaped.body).collect();
        prog.rules.push(Rule { head: shaped.head, body });
    }
    prog
}

/// The lowered policies: the wave's clauses and their shapes (aligned), and the rules over them.
pub(in crate::code_rule_dsl) struct Lowered {
    pub(in crate::code_rule_dsl) clauses: Vec<Clause>,
    pub(in crate::code_rule_dsl) shapes: Vec<Shape>,
    pub(in crate::code_rule_dsl) logic: Logic,
    /// The policies as one logic program, as the round trip gave it.
    pub(in crate::code_rule_dsl) program: Program,
}

/// `policies` (each by its file stem) through the logic language: their documents round-tripped, their
/// `ast` atoms whose Python matcher lowers as the wave's clauses, their rules rewritten over them, and the
/// demand program.
pub(in crate::code_rule_dsl) fn lower(policies: &[(String, CompiledCodeRuleDsl)]) -> StoreResult<Lowered> {
    lower_with(policies, &Program::default())
}

/// The policy of an authored program's helper `ast` atoms has its policy's id and this suffix.
const HELPERS: &str = "~helpers";

/// A matcher's text as a compiled policy holds it (`by_matcher`'s keys): authored text may order or
/// space its keys otherwise.
fn normalized(matcher: &str) -> String {
    serde_json::from_str::<serde_json::Value>(matcher).map_or_else(|_| matcher.to_owned(), |value| crate::code_rule_dsl::json::canonicalize_value(value).to_string())
}

/// The base relations a rule may read besides the `py.*` families: T's syntax relations and
/// `core_node`, bound with each chunk before its batch (`facts::vtab::syntax::sources`).
const BASE: [(&str, &[(&str, &str)]); 10] = [
    ("node", &[("d", "int"), ("n", "int"), ("kind", "int"), ("start", "int"), ("end_", "int")]),
    ("doc", &[("d", "int"), ("path", "sym"), ("module", "sym")]),
    ("pkg", &[("d", "int"), ("p", "sym")]),
    ("child", &[("p", "int"), ("c", "int"), ("idx", "int")]),
    ("field", &[("p", "int"), ("f", "int"), ("c", "int")]),
    ("text", &[("n", "int"), ("s", "sym")]),
    ("intval", &[("n", "int"), ("v", "int")]),
    ("op", &[("n", "int"), ("s", "sym")]),
    ("tape", &[("n", "int"), ("par", "int"), ("size", "int"), ("depth", "int")]),
    ("core_node", &[("d", "int"), ("t", "int"), ("n", "int")]),
];

/// Whether `rel` is bound with the chunk before its batch: a base relation or a `py.*` family.
fn is_bound(rel: &str) -> bool {
    is_py(rel) || BASE.iter().any(|(base, _)| *base == rel)
}

/// `policies` and an authored program in Rerail logic beside them (`authored`; empty for none):
/// - its `.policy` documents are policies of their own, whose forbid clauses are the anchors of its
///   violations: their codes, messages and shapes;
/// - its rules join the policies', and a rule deriving `violation("<policy>", "<clause>", …)` replaces
///   the rules the round trip gave that clause;
/// - an `ast` atom of its rules that is no clause's matcher is a helper clause of the wave, of a
///   policy of its own (`HELPERS`, its first `.policy`'s `define` clauses kept so utils resolve) that
///   derives no violation;
/// - its rules may read the base relations (`BASE`) and the `py.*` families as they are bound.
pub(in crate::code_rule_dsl) fn lower_with(policies: &[(String, CompiledCodeRuleDsl)], authored: &Program) -> StoreResult<Lowered> {
    let compile = |document: &serde_json::Value| {
        crate::code_rule_dsl::CodeRuleDslRuntime::compile(document.to_string(), "json".to_owned(), crate::code_rule_dsl::tools::CAPABILITIES.to_owned()).map_err(|e| invalid(e.reason))
    };
    let mut owned: Vec<(String, CompiledCodeRuleDsl)> = policies.iter().map(|(name, compiled)| (name.clone(), CompiledCodeRuleDsl { inner: Arc::clone(&compiled.inner) })).collect();
    for document in &authored.policies {
        let id = document.get("id").and_then(|id| id.as_str()).ok_or_else(|| invalid("an authored .policy has no id"))?;
        owned.push((id.to_owned(), compile(document)?));
    }
    // The authored atoms' matchers that no clause has: the helper clauses, `h0`, `h1`, … in order.
    let known: BTreeSet<String> = owned
        .iter()
        .filter_map(|(_, compiled)| serde_json::from_str::<serde_json::Value>(&compiled.inner.canonical).ok())
        .flat_map(|canonical| canonical.get("clauses").and_then(|c| c.as_array()).cloned().unwrap_or_default())
        .filter_map(|clause| clause.get("forbid").and_then(|f| f.get("ast")).map(|m| m.to_string()))
        .collect();
    let mut helpers = Vec::<String>::new();
    for literal in authored.rules.iter().flat_map(|rule| &rule.body) {
        if let Literal::Ast { matcher, .. } = literal {
            let matcher = normalized(matcher);
            if !known.contains(&matcher) && !helpers.contains(&matcher) {
                helpers.push(matcher);
            }
        }
    }
    if !helpers.is_empty() {
        let base = authored.policies.first().ok_or_else(|| invalid("authored rules with helper ast atoms need a .policy"))?;
        let id = format!("{}{HELPERS}", base.get("id").and_then(|id| id.as_str()).unwrap_or_default());
        let mut document = base.clone();
        document["id"] = serde_json::Value::String(id.clone());
        let mut clauses: Vec<serde_json::Value> = base.get("clauses").and_then(|c| c.as_array()).into_iter().flatten().filter(|c| c.get("define").is_some()).cloned().collect();
        for (i, matcher) in helpers.iter().enumerate() {
            let matcher: serde_json::Value = serde_json::from_str(matcher).map_err(invalid)?;
            clauses.push(serde_json::json!({"id": format!("h{i}"), "forbid": {"ast": matcher}, "violation": {"code": "helper", "message": "helper", "anchor": "witness.span"}}));
        }
        document["clauses"] = serde_json::Value::Array(clauses);
        owned.push((id, compile(&document)?));
    }
    // The clauses whose rules the authored ones replace.
    let replaced: BTreeSet<(String, String)> = authored
        .rules
        .iter()
        .filter(|rule| rule.head.rel == "violation")
        .filter_map(|rule| match rule.head.args.as_slice() {
            [Term::Str(policy), Term::Str(clause), ..] => Some((policy.clone(), clause.clone())),
            _ => None,
        })
        .collect();
    let policies = &owned[..];
    let mut program = Program::default();
    let mut documents = Vec::new();
    for (name, compiled) in policies {
        let document: serde_json::Value = serde_json::from_str(&compiled.inner.canonical).map_err(|e| invalid(format!("{name}: {e}")))?;
        yaml::add(&mut program, &document).map_err(|e| invalid(format!("{name}: {e}")))?;
        documents.push(document);
    }
    let back = yaml::documents(&program).map_err(invalid)?;
    if let Some(at) = (0..documents.len()).find(|&at| back.get(at) != Some(&documents[at])) {
        return Err(invalid(format!("{}: the policy does not round-trip through the logic language", policies[at].0)));
    }

    // Each forbid clause's matcher, by its text, as the rules' `ast` atoms hold it.
    let mut by_matcher = HashMap::<String, Vec<(usize, usize)>>::new();
    for (p, (_, compiled)) in policies.iter().enumerate() {
        let canonical: serde_json::Value = serde_json::from_str(&compiled.inner.canonical).map_err(invalid)?;
        for clause in canonical.get("clauses").and_then(|c| c.as_array()).into_iter().flatten() {
            let (Some(id), Some(matcher)) = (clause.get("id").and_then(|i| i.as_str()), clause.get("forbid").and_then(|f| f.get("ast"))) else { continue };
            if let Some(at) = compiled.inner.clauses.iter().position(|c| c.id == id) {
                by_matcher.entry(normalized(&matcher.to_string())).or_default().push((p, at));
            }
        }
    }
    let (mut clauses, mut shapes) = (Vec::new(), Vec::new());
    let mut index = HashMap::<(usize, usize), Option<usize>>::new();
    let mut lower_clause = |p: usize, at: usize| -> Option<usize> {
        *index.entry((p, at)).or_insert_with(|| {
            let (policy, compiled) = &policies[p];
            let clause = &compiled.inner.clauses[at];
            let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &clause.kind else { return None };
            let mut python = matchers.iter().enumerate().filter(|(_, m)| m.language == GrammarDialect::Python);
            let (grammar_at, grammar) = python.next()?;
            let (tree, paths) = gated_match_tree(grammar).ok()?;
            let lowered = LoweredMatcher::lower(&tree).ok()?;
            clauses.push(Clause {
                paths,
                name: format!("{policy}/{}", clause.id),
                key: (compiled.inner.id.clone(), clause.id.clone()),
                plan: Arc::clone(&grammar.anchors),
                matcher: lowered,
            });
            shapes.push(Shape { rule: Arc::clone(&compiled.inner), clause: at, grammar: grammar_at });
            Some(clauses.len() - 1)
        })
    };

    // The rules over the wave's clauses; a rule whose atom does not lower is left out.
    let mut wave = Program { decls: program.decls.clone(), outputs: program.outputs.clone(), facts: authored.facts.clone(), ..Program::default() };
    for (rel, columns) in &authored.decls {
        wave.decls.entry(rel.clone()).or_insert_with(|| columns.clone());
    }
    for rel in &authored.outputs {
        if !wave.outputs.contains(rel) {
            wave.outputs.push(rel.clone());
        }
    }
    let mut demand = Program::default();
    let key_of = |rule: &Rule| match rule.head.args.as_slice() {
        [Term::Str(policy), Term::Str(clause), ..] if rule.head.rel == "violation" => Some((policy.clone(), clause.clone())),
        _ => None,
    };
    let generated = program.rules.iter().filter(|rule| {
        key_of(rule).is_none_or(|key| !replaced.contains(&key) && !key.0.ends_with(HELPERS))
    });
    for rule in generated.chain(&authored.rules) {
        let head_key = match rule.head.args.as_slice() {
            [Term::Str(policy), Term::Str(clause), ..] => Some((policy.as_str(), clause.as_str())),
            _ => None,
        };
        let mut lowered_body = Vec::new();
        let mut demand_body = Vec::new();
        let mut seeds = Vec::new();
        let mut covered = true;
        for literal in &rule.body {
            let Literal::Ast { document, node, matcher, captures } = literal else {
                lowered_body.push(literal.clone());
                demand_body.push(literal.clone());
                continue;
            };
            // The wave's `matched(i, D, N)` carries no captures (O60): a rule reading them stays out.
            if !captures.is_empty() {
                covered = false;
                break;
            }
            let found = by_matcher.get(&normalized(matcher)).and_then(|found| {
                found
                    .iter()
                    .find(|&&(p, at)| head_key == Some((policies[p].1.inner.id.as_str(), policies[p].1.inner.clauses[at].id.as_str())))
                    .or_else(|| found.first())
                    .copied()
            });
            let Some(i) = found.and_then(|(p, at)| lower_clause(p, at)) else {
                covered = false;
                break;
            };
            lowered_body.push(matched(i, document, node));
            demand_body.push(atom(&format!("ast_{i}"), vec![var(document), var(node)]));
            seeds.push(document.clone());
        }
        if !covered {
            continue;
        }
        wave.rules.push(Rule { head: rule.head.clone(), body: lowered_body });
        // The chunk's documents seed each atom's demand.
        seeds.sort();
        seeds.dedup();
        let body = seeds.iter().map(|d| atom(WAVE, vec![var(d)])).chain(demand_body).collect();
        demand.rules.push(Rule { head: rule.head.clone(), body });
    }
    // An authored violation's clause is the wave's even where no rule reads its matcher: its shape
    // is the layer's (code, message, anchor).
    for (policy, clause) in &replaced {
        let at = policies.iter().enumerate().find_map(|(p, (_, compiled))| {
            (compiled.inner.id == *policy).then(|| compiled.inner.clauses.iter().position(|c| c.id == *clause).map(|at| (p, at))).flatten()
        });
        let Some((p, at)) = at else {
            return Err(invalid(format!("authored violations of {policy}/{clause}: no such forbid clause")));
        };
        if lower_clause(p, at).is_none() {
            return Err(invalid(format!("authored violations of {policy}/{clause}: its matcher does not lower")));
        }
    }
    declare_violation(&mut wave);
    declare_py(&mut wave);
    declare_base(&mut wave);
    let shared: Vec<(String, String)> = clauses.iter().map(|clause| clause.key.clone()).collect();
    if let Some(key) = shared.iter().enumerate().find_map(|(at, key)| shared[..at].contains(key).then_some(key)) {
        return Err(invalid(format!("clause {}/{} lowers twice: its violations would not tell its matchers apart", key.0, key.1)));
    }
    let demand = demand_program(demand, &wave, clauses.len())?;
    let read: BTreeSet<&str> = wave.rules.iter().flat_map(|r| r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.as_str())).collect();
    let bound = read
        .into_iter()
        .filter(|rel| is_bound(rel))
        .filter_map(|rel| crate::code_rule_dsl::logic::sql::select::input_columns(rel).map(|columns| (rel.to_owned(), columns)))
        .collect();
    Ok(Lowered { clauses, shapes, logic: Logic { rules: wave_text(&merged(wave)).map_err(invalid)?, demand, bound }, program })
}

/// The demand program of `rules` (each reading `ast_i` atoms after `wave` seeds): `.demand` on every atom,
/// T's magic sets, then the magic rules alone, less what they cannot read before the batch, and
/// `demanded(i, d) :- magic_ast_i(d)`.
fn demand_program(mut prog: Program, wave: &Program, atoms: usize) -> StoreResult<Option<Program>> {
    if atoms == 0 {
        return Ok(None);
    }
    let int = |name: &str| (name.to_owned(), "int".to_owned());
    prog.decls = wave.decls.iter().map(|(rel, cols)| (rel.clone(), cols.iter().map(|(c, t)| (c.clone(), if t == "node" { "int".to_owned() } else { t.clone() })).collect())).collect();
    prog.decls.insert(WAVE.to_owned(), vec![int("d")]);
    prog.inputs.push(WAVE.to_owned());
    for i in 0..atoms {
        prog.decls.insert(format!("ast_{i}"), vec![int("d"), int("n")]);
        prog.inputs.push(format!("ast_{i}"));
        prog.demand.push(format!("ast_{i}"));
    }
    let magic = magic::magic_sets(prog, &magic::Options::default()).map_err(|e| invalid(format!("the policies' demand: {e}")))?;
    // What the batch has not derived yet: the atoms, every input but the chunk's documents and the
    // `py.*` families (bound with the chunk's base relations, before the batch), and every relation
    // that reads one.
    let mut unavailable: BTreeSet<String> = (0..atoms).map(|i| format!("ast_{i}")).collect();
    unavailable.extend(magic.inputs.iter().filter(|rel| *rel != WAVE && !rel.starts_with("ast_") && !is_bound(rel)).cloned());
    loop {
        let more = magic
            .rules
            .iter()
            .filter(|rule| !rule.head.rel.starts_with("magic_") && !unavailable.contains(&rule.head.rel))
            .filter(|rule| rule.body.iter().any(|l| l.atom().is_some_and(|a| unavailable.contains(&a.rel)) || matches!(l, Literal::Ast { .. })))
            .map(|rule| rule.head.rel.clone())
            .collect::<Vec<_>>();
        if more.is_empty() {
            break;
        }
        unavailable.extend(more);
    }
    // The magic rules of the atoms, widened, and the available rules they read.
    let mut rules = Vec::new();
    let mut needed = BTreeSet::new();
    for rule in magic.rules.iter().filter(|rule| rule.head.rel.starts_with("magic_ast_")) {
        let body = widened(&rule.head, &rule.body, &unavailable);
        needed.extend(body.iter().filter_map(|l| l.atom()).map(|a| a.rel.clone()));
        rules.push(Rule { head: rule.head.clone(), body });
    }
    let mut seen = BTreeSet::new();
    while let Some(rel) = needed.iter().find(|rel| !seen.contains(*rel)).cloned() {
        seen.insert(rel.clone());
        for rule in magic.rules.iter().filter(|rule| rule.head.rel == rel) {
            needed.extend(rule.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.clone()));
            rules.push(rule.clone());
        }
    }
    for i in 0..atoms {
        let head = Atom { rel: DEMANDED.to_owned(), args: vec![Term::Int(i as i64), var("D")], neg: false, gated: false };
        rules.push(Rule { head, body: vec![atom(&format!("magic_ast_{i}"), vec![var("D")])] });
    }
    let referenced: BTreeSet<String> = rules.iter().flat_map(|r| std::iter::once(r.head.rel.clone()).chain(r.body.iter().filter_map(|l| l.atom()).map(|a| a.rel.clone()))).collect();
    let mut decls: BTreeMap<String, Vec<(String, String)>> = magic.decls.iter().filter(|(rel, _)| referenced.contains(*rel)).map(|(r, c)| (r.clone(), c.clone())).collect();
    decls.insert(DEMANDED.to_owned(), vec![int("c"), int("d")]);
    decls.insert(WAVE.to_owned(), vec![int("d")]);
    for i in 0..atoms {
        decls.entry(format!("magic_ast_{i}")).or_insert_with(|| vec![int("d")]);
    }
    let facts = magic.facts.iter().filter(|(rel, _)| referenced.contains(*rel)).map(|(r, f)| (r.clone(), f.clone())).collect();
    // The chunk's documents and the `py.*` families the magic rules read are its inputs.
    let inputs = std::iter::once(WAVE.to_owned()).chain(referenced.iter().filter(|rel| is_bound(rel)).cloned()).collect();
    let demand = Program { decls, facts, rules, inputs, outputs: vec![DEMANDED.to_owned()], ..Program::default() };
    front::strata(&demand).map_err(|e| invalid(format!("the policies' demand: {e}")))?;
    Ok(Some(demand))
}

/// `body` less the literals the batch has not derived (`unavailable`), and less those left without their
/// variables bound: leaving a literal out of a magic rule only widens demand.
fn widened(head: &Atom, body: &[Literal], unavailable: &BTreeSet<String>) -> Vec<Literal> {
    let mut kept: Vec<Literal> = body.iter().filter(|l| !l.atom().is_some_and(|a| unavailable.contains(&a.rel))).cloned().collect();
    loop {
        let bound: BTreeSet<String> = kept.iter().filter_map(|l| match l {
            Literal::Atom(a) if !a.neg => Some(a.vars()),
            _ => None,
        }).flatten().collect();
        let before = kept.len();
        kept.retain(|l| match l {
            Literal::Atom(a) if !a.neg => true,
            Literal::Nearest { n, a, .. } => bound.contains(n) || bound.contains(a),
            other => other.vars().is_subset(&bound),
        });
        if kept.len() == before {
            break;
        }
    }
    let bound: BTreeSet<String> = kept.iter().filter_map(|l| l.atom().filter(|a| !a.neg)).flat_map(|a| a.vars()).collect();
    if !head.vars().is_subset(&bound) {
        // The seed binds every atom's document: a key the widening unbound is demanded everywhere.
        return vec![atom(WAVE, head.args.clone())];
    }
    kept
}

#[cfg(test)]
mod tests;
