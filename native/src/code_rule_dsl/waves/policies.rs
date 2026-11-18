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
//! starts with `wave($D)`, the chunk's documents (`temp.wave_document`). T's magic sets (`KeyDemand`) give the
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
use crate::code_rule_dsl::logic::front::{List, Value, yaml};
use crate::code_rule_dsl::logic::plan::corpus::preparation::magic::KeyDemand;
use crate::code_rule_dsl::logic::plan::emit;
use crate::code_rule_dsl::logic::program_facts::Field;
use crate::code_rule_dsl::logic::program_image::analyses::Components;
use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
use crate::code_rule_dsl::logic::program_image::manager::{Pipeline, analysis};
use crate::code_rule_dsl::logic::program_image::{AtomId, AtomRef, Builder, Lit, LitRef, Rewrite, RuleRef, Site, Slot, T, TermRef, Version, t_of};
use crate::code_rule_dsl::logic::sql::select::Source;
use crate::code_rule_dsl::matcher::gated_match_tree;
use crate::code_rule_dsl::model::{ClauseKind, Query};
use crate::code_rule_dsl::transducer::packed::LoweredMatcher;
use crate::content_store::{ContentStore, StoreError, StoreResult};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

/// The relation the wave's matches are in (`matches::SOURCE`).
const MATCH: &str = "matched";
/// The chunk's documents in the demand program, and what it derives.
const WAVE: &str = "wave";
const DEMANDED: &str = "demanded";
const VIOLATION: &str = "violation";

fn invalid(error: impl std::fmt::Display) -> StoreError {
    StoreError::Invalid(error.to_string())
}

/// The policies' rules as the wave runs them.
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct Logic {
    /// The program added to the wave's source (`matches::SOURCE`): the relations' declarations, outputs, rules
    /// over `match` and their facts (decision 0249).
    pub(in crate::code_rule_dsl) rules: Arc<Version>,
    /// The demand program: the magic rules seeded by `wave(d)`, deriving `demanded(i, d)`. `None`: every
    /// clause is demanded on every document.
    pub(in crate::code_rule_dsl) demand: Option<Arc<Version>>,
    /// The relations the rules read as bound with the chunk (`is_bound`), with their columns: the
    /// wave reads them as the chunk's `main` tables.
    pub(in crate::code_rule_dsl) bound: Vec<(String, &'static [&'static str])>,
}

impl Logic {
    /// Each clause as its own forbid rule, `violation(<policy>, <clause>, d, n) :- matched(i, d, n)`,
    /// demanded everywhere: the program of clauses given without their documents (tests, `rir` dumps of
    /// one clause).
    pub(in crate::code_rule_dsl) fn per_clause(clauses: &[Clause]) -> StoreResult<Self> {
        let mut b = Builder::base();
        for (i, clause) in clauses.iter().enumerate() {
            let (policy, id) = &clause.key;
            let head = [T::Str(policy), T::Str(id), T::Var("D"), T::Var("N")];
            b.rule().head(VIOLATION, &head).atom(MATCH, &[T::Int(i as i64), T::Var("D"), T::Var("N")]).finish().map_err(invalid)?;
        }
        declare_violation(&mut b);
        output_violation(&mut b);
        Ok(Self { rules: merged(&finished(b)?)?, demand: None, bound: Vec::new() })
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
        for rel in prog.input_names().into_iter().filter(|rel| is_bound(rel)) {
            let columns = crate::code_rule_dsl::logic::sql::select::input_columns(&rel).expect("a bound relation's columns");
            batch.insert(rel.clone(), (Source::named(&format!("main.{rel}"), columns), 1000.0));
        }
        let intern = &mut |names: &[&str]| store.intern_atoms(names).map_err(|e| crate::code_rule_dsl::logic::plan::Error::Plan(e.to_string()));
        let emitted = emit::emit_program(Arc::clone(prog), "demand", &batch, intern).and_then(|emitted| emit::single(emitted, DEMANDED)).map_err(|e| e.to_string())?;
        let sql = &emitted.program.steps[0].sql;
        // The emitter always sets up `rdl_int`; a statement that reads no setup needs none.
        let named = |statement: &str| statement.split(['(', ' ']).nth(3).map(str::to_owned).unwrap_or_default();
        if let Some(needed) = emitted.setup.iter().find(|statement| sql.contains(&named(statement))) {
            return Err(format!("the demand program needs setup: {needed}"));
        }
        sql.strip_prefix(&format!("INSERT OR IGNORE INTO new_{DEMANDED} ")).map(|sql| Some(sql.to_owned())).ok_or_else(|| format!("not an insert: {sql}"))
    }
}

/// `b`'s version.
fn finished(b: Builder<'_>) -> StoreResult<Arc<Version>> {
    b.finish().map(|(v, _)| v).map_err(invalid)
}

/// The program with nothing in it: no authored program beside the policies.
pub(in crate::code_rule_dsl) fn nothing() -> StoreResult<Arc<Version>> {
    finished(Builder::base())
}

/// Whether `rel` is one of a family's lazy relations (`facts::vtab::syntax::Families`: Python's `py.*`).
fn is_py(rel: &str) -> bool {
    crate::code_rule_dsl::facts::vtab::syntax::is_family(rel)
}

/// Declares each relation of `read` that `b` does not declare and that is a `py.*` family (integer columns) or a
/// base relation (`BASE`), as bound.
fn declare_read(b: &mut Builder<'_>, read: &BTreeSet<String>) {
    for rel in read {
        if b.declares(rel) {
            continue;
        }
        if is_py(rel) {
            let columns = crate::code_rule_dsl::facts::vtab::syntax::family_columns(rel).expect("a family's relation");
            b.declare(rel, &columns.iter().map(|c| (*c, "int")).collect::<Vec<_>>());
        } else if let Some((_, columns)) = BASE.iter().find(|(base, _)| *base == rel) {
            b.declare(rel, columns);
        }
    }
}

fn declare_violation(b: &mut Builder<'_>) {
    if !b.declares(VIOLATION) {
        b.declare(VIOLATION, &[("policy", "sym"), ("clause", "sym"), ("d", "int"), ("n", "int")]);
    }
}

/// `violation` among `b`'s outputs.
fn output_violation(b: &mut Builder<'_>) {
    let mut outputs = b.output_names();
    if !outputs.iter().any(|o| o == VIOLATION) {
        outputs.push(VIOLATION.to_owned());
        b.set_outputs(&outputs);
    }
}

/// Declares each of `from`'s relations that `b` does not, its node columns the wave's integers.
fn declare_from(b: &mut Builder<'_>, from: &Version) {
    for rel in from.declared_names() {
        if b.declares(&rel) {
            continue;
        }
        let columns = from.columns_of(&rel).unwrap_or_default();
        b.declare(&rel, &columns.iter().map(|(c, t)| (c.as_str(), if t == "node" { "int" } else { t.as_str() })).collect::<Vec<_>>());
    }
}

/// The relations a rule's body reads: its atoms', an aggregate's included.
fn reads<'v>(v: &'v Version, r: RuleRef<'v>) -> impl Iterator<Item = &'v str> + 'v {
    r.body().filter_map(|l| l.atom()).map(|a| v.rel_name(a.rel()))
}

/// The variables of term `t`, into `out`.
fn term_vars(t: TermRef<'_>, out: &mut BTreeSet<Slot>) {
    match t {
        TermRef::Var(s) => {
            out.insert(s);
        }
        TermRef::Call(_, args) => args.for_each(|a| term_vars(a, out)),
        _ => {}
    }
}

fn atom_vars(a: AtomRef<'_>) -> BTreeSet<Slot> {
    let mut out = BTreeSet::new();
    a.args().for_each(|t| term_vars(t, &mut out));
    out
}

/// The variables literal `l` names.
fn lit_vars(l: LitRef<'_>) -> BTreeSet<Slot> {
    match l.view() {
        Lit::Atom(a) => atom_vars(a),
        Lit::Cmp { l, r, .. } => {
            let mut out = BTreeSet::new();
            term_vars(l, &mut out);
            term_vars(r, &mut out);
            out
        }
        Lit::Agg { var, over, atom, .. } => {
            let mut out = atom_vars(atom);
            out.insert(var);
            out.extend(over);
            out
        }
        Lit::Nearest { n, a, .. } => [n, a].into(),
        Lit::Ast { document, node, captures, .. } => [document, node].into_iter().chain(captures.map(|(_, s)| s)).collect(),
    }
}

/// A rule's shape: its text with each constant in its head and in its positive atoms (and their calls) taken out as
/// `K`, in order, and the constants it was written with. Variables are their slots, numbered as they first appear, so
/// rules equal but for their variables' names and those constants have one shape.
fn shape(v: &Version, r: RuleRef<'_>) -> (String, Vec<Value>) {
    use std::fmt::Write;
    fn term(v: &Version, t: TermRef<'_>, abstracted: bool, out: &mut String, constants: &mut Vec<Value>) {
        match t {
            TermRef::Int(i) if abstracted => constants.push(Value::Int(i)),
            TermRef::Str(a) if abstracted => constants.push(Value::Str(v.text(a).to_owned().into())),
            _ => {}
        }
        match t {
            TermRef::Int(_) | TermRef::Str(_) if abstracted => out.push('K'),
            TermRef::Var(s) => _ = write!(out, "V{}", s.0),
            TermRef::Wild => out.push('_'),
            TermRef::Top => out.push('T'),
            TermRef::Int(i) => _ = write!(out, "{i}"),
            TermRef::Str(a) => _ = write!(out, "{:?}", v.text(a)),
            TermRef::Branded(b) => _ = write!(out, "{b:?}"),
            TermRef::Call(f, args) => {
                _ = write!(out, "{}(", v.text(f));
                for a in args {
                    term(v, a, abstracted, out, constants);
                    out.push(',');
                }
                out.push(')');
            }
        }
    }
    let atom = |a: AtomRef<'_>, abstracted: bool, out: &mut String, constants: &mut Vec<Value>| {
        _ = write!(out, "{}{}{}(", if a.neg() { "!" } else { "" }, if a.gated() { "?" } else { "" }, v.rel_name(a.rel()));
        for t in a.args() {
            term(v, t, abstracted, out, constants);
            out.push(',');
        }
        out.push(')');
    };
    let (mut out, mut constants) = (String::new(), Vec::new());
    atom(r.head(), true, &mut out, &mut constants);
    out.push_str(" :-");
    for l in r.body() {
        out.push(' ');
        match l.view() {
            Lit::Atom(a) => atom(a, !a.neg() && !a.gated(), &mut out, &mut constants),
            Lit::Cmp { l, op, r } => {
                term(v, l, false, &mut out, &mut constants);
                _ = write!(out, " {op:?} ");
                term(v, r, false, &mut out, &mut constants);
            }
            Lit::Agg { var, op, over, atom: a } => {
                let op = match op {
                    crate::code_rule_dsl::logic::program_image::AggRef::Join { by, sep } => format!("join[{:?}]({})", v.text(sep), by.map(|s| s.0.to_string()).collect::<Vec<_>>().join(",")),
                    other => format!("{:?}", std::mem::discriminant(&other)),
                };
                _ = write!(out, "V{} = {op} V{:?} : ", var.0, over.map(|s| s.0));
                atom(a, false, &mut out, &mut constants);
            }
            Lit::Nearest { n, a, kindset } => _ = write!(out, "nearest(V{}, V{}, {})", n.0, a.0, v.text(kindset)),
            Lit::Ast { document, node, matcher, captures } => {
                _ = write!(out, "ast(V{}, V{}, {}", document.0, node.0, v.text(matcher));
                for (name, s) in captures {
                    _ = write!(out, ", {}: V{}", v.text(name), s.0);
                }
                out.push(')');
            }
        }
        out.push(',');
    }
    (out, constants)
}

/// Writes a rule's constants (in its head and positive atoms, as `shape` takes them out) as the variables `ks`, in
/// order: `on` while the walk is in such an atom.
struct Abstracting {
    ks: Vec<AtomId>,
    next: usize,
    on: bool,
}

impl Rewrite for Abstracting {
    fn term(&mut self, _site: Site<'_>, t: TermRef<'_>) -> Option<T<'static>> {
        if !self.on || !matches!(t, TermRef::Int(_) | TermRef::Str(_)) {
            return None;
        }
        self.next += 1;
        Some(T::VarAtom(self.ks[self.next - 1]))
    }
}

/// `v`'s rules of one shape (`shape`) as one rule over the shape's constant table, `<head>_shape_<k>`, with a row
/// per rule. A shape of one rule, or of none of its constants, is kept as written.
fn merged(v: &Arc<Version>) -> StoreResult<Arc<Version>> {
    let kind = |c: &Value| if matches!(c, Value::Int(_)) { "int" } else { "sym" };
    let mut groups: Vec<(Vec<RuleRef<'_>>, Vec<Vec<Value>>)> = Vec::new();
    let mut by_key = HashMap::<String, usize>::new();
    for r in v.rules() {
        let (shaped, constants) = shape(v, r);
        let key = format!("{shaped} / {}", constants.iter().map(kind).collect::<Vec<_>>().join(","));
        let at = *by_key.entry(key).or_insert_with(|| {
            groups.push((Vec::new(), Vec::new()));
            groups.len() - 1
        });
        groups[at].0.push(r);
        groups[at].1.push(constants);
    }
    let mut b = Builder::derive(v);
    let mut tables = 0;
    for (rules, rows) in groups {
        if rules.len() < 2 || rows[0].is_empty() {
            for r in rules {
                b.copy(v, r);
            }
            continue;
        }
        let r = rules[0];
        let head = v.rel_name(r.head().rel());
        let rel = loop {
            let rel = format!("{head}_shape_{tables}");
            tables += 1;
            if !v.declares(&rel) {
                break rel;
            }
        };
        let columns: Vec<(String, &str)> = rows[0].iter().enumerate().map(|(j, c)| (format!("k{j}"), kind(c))).collect();
        b.declare(&rel, &columns.iter().map(|(c, t)| (c.as_str(), *t)).collect::<Vec<_>>());
        b.set_facts(&rel, &rows.iter().map(|row| List::from(row.clone())).collect::<Vec<_>>());
        // The constants' variables, `K0, K1, …`, each named apart from the rule's own.
        let names: BTreeSet<&str> = r.vars().map(|a| v.text(a)).collect();
        let ks: Vec<AtomId> = (0..rows[0].len())
            .map(|j| {
                let mut k = format!("K{j}");
                while names.contains(k.as_str()) {
                    k.push('_');
                }
                b.atom(&k)
            })
            .collect();
        let table: Vec<T<'_>> = ks.iter().map(|&k| T::VarAtom(k)).collect();
        let rel = b.rel(&rel);
        let mut rw = Abstracting { ks: ks.clone(), next: 0, on: true };
        let mut rule = b.rule().head_with(v, r, r.head(), &mut rw).atom(rel, &table);
        for l in r.body() {
            rw.on = matches!(l.view(), Lit::Atom(a) if !a.neg() && !a.gated());
            rule = rule.lit_with(v, r, l, &mut rw);
        }
        rule.finish().map_err(invalid)?;
    }
    finished(b)
}

/// The lowered policies: the wave's clauses and their shapes (aligned), and the rules over them.
pub(in crate::code_rule_dsl) struct Lowered {
    pub(in crate::code_rule_dsl) clauses: Vec<Clause>,
    pub(in crate::code_rule_dsl) shapes: Vec<Shape>,
    pub(in crate::code_rule_dsl) logic: Logic,
    /// The policies as one logic program, as the round trip gave it.
    pub(in crate::code_rule_dsl) program: Arc<Version>,
}

/// `policies` (each by its file stem) through the logic language: their documents round-tripped, their
/// `ast` atoms whose Python matcher lowers as the wave's clauses, their rules rewritten over them, and the
/// demand program.
pub(in crate::code_rule_dsl) fn lower(policies: &[(String, CompiledCodeRuleDsl)]) -> StoreResult<Lowered> {
    lower_with(policies, &nothing()?)
}

/// What reads an `ast` atom's captures (decision 0107): W's matcher of the atom's rule, matched again at each root the
/// transducer found, and the visits its policy allows a file. `verify`: the transducer runs a relaxed matcher (a
/// superset of the roots), so each root is W's only where W's matcher matches there too.
pub(in crate::code_rule_dsl) struct Captor {
    pub(in crate::code_rule_dsl) grammar: crate::code_rule_dsl::matcher::GrammarMatcher,
    pub(in crate::code_rule_dsl) limits: crate::code_rule_dsl::model::Limits,
    pub(in crate::code_rule_dsl) verify: bool,
}

/// An `ast` atom's matcher lowered for the transducer (decision 0106), as `lower_with` lowers a helper clause: the
/// forbid clause of a policy of its own (`base`'s `define` clauses kept so utils resolve), its Python grammar's match
/// tree, and the gates on a document's path and text. A matcher the transducer does not lower runs as its relaxed
/// form ([`relaxed`]) where that lowers, its roots confirmed by W's matcher (`Captor::verify`, decision 0107). `Err`
/// says why it does not lower.
pub(in crate::code_rule_dsl) fn lower_matcher(matcher: &str, base: Option<&serde_json::Value>) -> Result<(LoweredMatcher, Vec<crate::code_rule_dsl::matcher::DocumentGate>, Captor), String> {
    let value: serde_json::Value = serde_json::from_str(matcher).map_err(|e| format!("the matcher is not JSON: {e}"))?;
    let (grammar, limits) = helper_grammar(&value, base)?;
    let lower = |grammar: &crate::code_rule_dsl::matcher::GrammarMatcher| -> Result<_, String> {
        let (tree, paths) = gated_match_tree(grammar).map_err(|e| format!("{e:?}"))?;
        Ok((LoweredMatcher::lower(&tree).map_err(|e| format!("{e:?}"))?, paths))
    };
    // A `transform` makes text for the captures and matches nothing: the transducer lowers the rule without it, and
    // W's matcher (the captor, which keeps it) makes the text at each root.
    let (value, shape) = match value.as_object().filter(|o| o.contains_key("transform")) {
        Some(o) => {
            let mut bare = o.clone();
            bare.remove("transform");
            let bare = serde_json::Value::Object(bare);
            let (shape, _) = helper_grammar(&bare, base)?;
            (bare, shape)
        }
        None => (value, grammar.clone()),
    };
    match lower(&shape) {
        Ok((lowered, paths)) => Ok((lowered, paths, Captor { grammar, limits, verify: false })),
        Err(why) => {
            // Comparisons dropped first; then every negation in a positive place too.
            let mut last = why.clone();
            for every_not in [false, true] {
                let loose_base = base.and_then(|b| relaxed_utils(b, &value, every_not));
                let base = loose_base.as_ref().or(base);
                let Some(loose) = relaxed(&value, every_not).or_else(|| loose_base.as_ref().map(|_| value.clone())) else { continue };
                let (loose, _) = helper_grammar(&loose, base)?;
                match lower(&loose) {
                    Ok((lowered, paths)) => return Ok((lowered, paths, Captor { grammar, limits, verify: true })),
                    Err(again) => last = again,
                }
            }
            Err(if last == why { why } else { format!("{why}; relaxed: {last}") })
        }
    }
}

/// `value`'s Python grammar matcher, compiled as a helper policy's forbid clause, and its policy's visit limits.
fn helper_grammar(value: &serde_json::Value, base: Option<&serde_json::Value>) -> Result<(crate::code_rule_dsl::matcher::GrammarMatcher, crate::code_rule_dsl::model::Limits), String> {
    let defines: Vec<serde_json::Value> = base.and_then(|b| b.get("clauses")).and_then(|c| c.as_array()).into_iter().flatten().filter(|c| c.get("define").is_some()).cloned().collect();
    let mut document = base.cloned().unwrap_or_else(|| serde_json::json!({"format": "rerail.policy/v1", "revision": 1, "mode": "block"}));
    document["id"] = serde_json::Value::String(format!("rr{HELPERS}"));
    let mut clauses = defines;
    clauses.push(serde_json::json!({"id": "h0", "forbid": {"ast": value}, "violation": {"code": "helper", "message": "helper", "anchor": "witness.span"}}));
    document["clauses"] = serde_json::Value::Array(clauses);
    let compiled = crate::code_rule_dsl::CodeRuleDslRuntime::compile(document.to_string(), "json".to_owned(), crate::code_rule_dsl::tools::CAPABILITIES.to_owned()).map_err(|e| e.reason.to_string())?;
    let clause = compiled.inner.clauses.iter().find(|c| c.id == "h0").ok_or("the helper clause is missing")?;
    let ClauseKind::Forbid(Query::Ast { matchers, .. }) = &clause.kind else { return Err("not an ast query".to_owned()) };
    let grammar = matchers.iter().find(|m| m.language == GrammarDialect::Python).ok_or("no Python matcher")?;
    Ok((grammar.clone(), compiled.inner.capabilities.limits.clone()))
}

/// `base`'s policy with its utils (`define.utils`) relaxed as `relaxed` relaxes a rule, where that only widens what
/// a rule using them matches: no `not` in a util reaches a widened one (through `matches`, transitively). `None`
/// where no util changes, or one would be widened under a `not`.
fn relaxed_utils(base: &serde_json::Value, rule: &serde_json::Value, every_not: bool) -> Option<serde_json::Value> {
    use serde_json::Value;
    use std::collections::{BTreeMap, BTreeSet};
    fn refs(v: &Value, out: &mut BTreeSet<String>) {
        match v {
            Value::Object(o) => o.iter().for_each(|(k, v)| match (k.as_str(), v) {
                ("matches", Value::String(name)) => {
                    out.insert(name.clone());
                }
                _ => refs(v, out),
            }),
            Value::Array(a) => a.iter().for_each(|v| refs(v, out)),
            _ => {}
        }
    }
    /// The utils `v` reaches under a `not`.
    fn negated(v: &Value, under: bool, out: &mut BTreeSet<String>) {
        match v {
            Value::Object(o) => o.iter().for_each(|(k, v)| match (k.as_str(), v) {
                ("matches", Value::String(name)) if under => {
                    out.insert(name.clone());
                }
                _ => negated(v, under || k == "not", out),
            }),
            Value::Array(a) => a.iter().for_each(|v| negated(v, under, out)),
            _ => {}
        }
    }
    let mut out = base.clone();
    let mut changed: BTreeSet<String> = BTreeSet::new();
    let mut graph: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut under_not: BTreeSet<String> = BTreeSet::new();
    negated(rule, false, &mut under_not);
    for clause in out.get_mut("clauses")?.as_array_mut()? {
        let Some(utils) = clause.get_mut("define").and_then(|d| d.get_mut("utils")).and_then(Value::as_object_mut) else { continue };
        for (name, util) in utils.iter_mut() {
            negated(util, false, &mut under_not);
            refs(util, graph.entry(name.clone()).or_default());
            // A util is a bare rule; `{"regex": ""}` (every node) stands in where relaxing leaves it nothing.
            let wrapped = serde_json::json!({"rule": {"all": [util.clone(), {"regex": ""}]}});
            if let Some(mut loose) = relaxed(&wrapped, every_not) {
                let mut rule = loose["rule"].take();
                if let Some(items) = rule.get_mut("all").and_then(Value::as_array_mut)
                    && items.len() > 1
                {
                    items.retain(|item| item != &serde_json::json!({"regex": ""}));
                }
                *util = rule;
                changed.insert(name.clone());
            }
        }
    }
    if changed.is_empty() {
        return None;
    }
    // A util reaching a widened one widens too.
    let mut widened = changed;
    loop {
        let more: Vec<String> = graph.iter().filter(|(n, r)| !widened.contains(*n) && r.iter().any(|x| widened.contains(x))).map(|(n, _)| n.clone()).collect();
        if more.is_empty() {
            break;
        }
        widened.extend(more);
    }
    let mut reached = BTreeSet::new();
    for n in &under_not {
        reached.insert(n.clone());
        let mut work = vec![n.clone()];
        while let Some(x) = work.pop() {
            for y in graph.get(&x).into_iter().flatten() {
                if reached.insert(y.clone()) {
                    work.push(y.clone());
                }
            }
        }
    }
    reached.is_disjoint(&widened).then_some(out)
}

/// A matcher's rule with the terms the transducer's labels cannot carry dropped where dropping only widens it: a
/// comparison of captures (`compare`) or a test of the file's path (`filePath`, W's to confirm) as a conjunct, and a `not` holding one, in a positive place (an `all`'s item,
/// a key of a rule, the rule of a `has` or `inside`); with `every_not`, every `not` in a positive place (a fact that
/// may be unknown under one, say). An `any` with an arm left matching every node goes too. Every node the matcher
/// matches, the relaxed one does. `None` where nothing is dropped, or a term would have to be dropped where that
/// narrows (under a `not`).
pub(in crate::code_rule_dsl) fn relaxed(value: &serde_json::Value, every_not: bool) -> Option<serde_json::Value> {
    use serde_json::Value;
    enum Relax {
        Same,
        Changed,
        /// Nothing positive left: the term matches every node, so its parent drops it.
        Remove,
        Fail,
    }
    fn holds_unsupported(v: &Value) -> bool {
        match v {
            Value::Object(o) => o.iter().any(|(k, v)| k == "compare" || k == "filePath" || holds_unsupported(v)),
            Value::Array(a) => a.iter().any(holds_unsupported),
            _ => false,
        }
    }
    /// The keys that are not a rule's terms but how a relational rule applies.
    const HOW: [&str; 2] = ["stopBy", "field"];
    fn relax(rule: &mut Value, every_not: bool) -> Relax {
        let Some(obj) = rule.as_object_mut() else { return Relax::Same };
        let mut changed = false;
        for key in obj.keys().cloned().collect::<Vec<_>>() {
            match key.as_str() {
                "compare" | "filePath" => {
                    obj.remove(&key);
                    changed = true;
                }
                "not" if every_not || holds_unsupported(&obj[&key]) => {
                    obj.remove(&key);
                    changed = true;
                }
                "all" => {
                    let Some(items) = obj[&key].as_array_mut() else { continue };
                    let mut kept = Vec::new();
                    for mut item in std::mem::take(items) {
                        match relax(&mut item, every_not) {
                            Relax::Same => kept.push(item),
                            Relax::Changed => {
                                changed = true;
                                kept.push(item);
                            }
                            Relax::Remove => changed = true,
                            Relax::Fail => return Relax::Fail,
                        }
                    }
                    if kept.is_empty() {
                        obj.remove(&key);
                    } else {
                        obj.insert(key, Value::Array(kept));
                    }
                }
                "any" => {
                    let Some(arms) = obj[&key].as_array_mut() else { continue };
                    let mut every = false;
                    for arm in arms {
                        match relax(arm, every_not) {
                            Relax::Same => {}
                            Relax::Changed => changed = true,
                            // An arm left matching every node: so does the `any`, which goes.
                            Relax::Remove => every = true,
                            Relax::Fail => return Relax::Fail,
                        }
                    }
                    if every {
                        obj.remove(&key);
                        changed = true;
                    }
                }
                "has" | "inside" | "precedes" | "follows" => match relax(&mut obj[&key], every_not) {
                    Relax::Same => {}
                    Relax::Changed => changed = true,
                    Relax::Remove => {
                        obj.remove(&key);
                        changed = true;
                    }
                    Relax::Fail => return Relax::Fail,
                },
                _ => {}
            }
        }
        if changed && obj.keys().all(|k| HOW.contains(&k.as_str())) {
            return Relax::Remove;
        }
        if changed { Relax::Changed } else { Relax::Same }
    }
    let mut value = value.clone();
    match relax(value.get_mut("rule")?, every_not) {
        Relax::Changed => Some(value),
        Relax::Same | Relax::Remove | Relax::Fail => None,
    }
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
pub(in crate::code_rule_dsl) fn lower_with(policies: &[(String, CompiledCodeRuleDsl)], authored: &Arc<Version>) -> StoreResult<Lowered> {
    let compile = |document: &serde_json::Value| {
        crate::code_rule_dsl::CodeRuleDslRuntime::compile(document.to_string(), "json".to_owned(), crate::code_rule_dsl::tools::CAPABILITIES.to_owned()).map_err(|e| invalid(e.reason))
    };
    let authored_policies = authored.policies().map_err(invalid)?;
    let mut owned: Vec<(String, CompiledCodeRuleDsl)> = policies.iter().map(|(name, compiled)| (name.clone(), CompiledCodeRuleDsl { inner: Arc::clone(&compiled.inner) })).collect();
    for document in &authored_policies {
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
    for literal in authored.rules().flat_map(|rule| rule.body()) {
        if let Lit::Ast { matcher, .. } = literal.view() {
            let matcher = normalized(authored.text(matcher));
            if !known.contains(&matcher) && !helpers.contains(&matcher) {
                helpers.push(matcher);
            }
        }
    }
    if !helpers.is_empty() {
        let base = authored_policies.first().ok_or_else(|| invalid("authored rules with helper ast atoms need a .policy"))?;
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
    let replaced: BTreeSet<(String, String)> = authored.rules().filter_map(|rule| violation_key(authored, rule)).collect();
    let policies = &owned[..];
    let mut documents = Vec::new();
    for (name, compiled) in policies {
        let document: serde_json::Value = serde_json::from_str(&compiled.inner.canonical).map_err(|e| invalid(format!("{name}: {e}")))?;
        documents.push(document);
    }
    let program = yaml::program(&documents).map_err(invalid)?;
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
            let lowered = match LoweredMatcher::lower(&tree) {
                Ok(lowered) => lowered,
                Err(e) => {
                    // A clause the IR refuses runs on W's matcher, natively (decision 0202's `ir-form` list).
                    let name = format!("{policy}/{}", clause.id);
                    rerail_accel::forms::missing("ir-form", &name, &name, &format!("{e:?}"), 1);
                    return None;
                }
            };
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

    // The rules over the wave's clauses (`wave`), and the same rules reading `ast_i` after the chunk's
    // documents (`demand`); a rule whose atom does not lower is left out.
    let (mut wave, mut demand) = (Builder::base(), Builder::base());
    let mut read = BTreeSet::<String>::new();
    let generated = program.rules().filter(|&rule| violation_key(&program, rule).is_none_or(|key| !replaced.contains(&key) && !key.0.ends_with(HELPERS)));
    for (from, rule) in generated.map(|r| (&*program, r)).chain(authored.rules().map(|r| (&**authored, r))) {
        let head_key = match rule.head().args().take(2).map(|t| if let TermRef::Str(a) = t { Some(from.text(a)) } else { None }).collect::<Option<Vec<_>>>() {
            Some(key) if key.len() == 2 => Some((key[0], key[1])),
            _ => None,
        };
        // Each `ast` literal's clause; `None` where one does not lower.
        let mut atoms = HashMap::<u32, usize>::new();
        let mut seeds = BTreeSet::<&str>::new();
        let covered = rule.body().all(|l| {
            let Lit::Ast { document, matcher, captures, .. } = l.view() else { return true };
            // The wave's `matched(i, D, N)` carries no captures (O60): a rule reading them stays out.
            if captures.count() > 0 {
                return false;
            }
            let found = by_matcher.get(&normalized(from.text(matcher))).and_then(|found| {
                found
                    .iter()
                    .find(|&&(p, at)| head_key == Some((policies[p].1.inner.id.as_str(), policies[p].1.inner.clauses[at].id.as_str())))
                    .or_else(|| found.first())
                    .copied()
            });
            let Some(i) = found.and_then(|(p, at)| lower_clause(p, at)) else { return false };
            atoms.insert(l.node(), i);
            seeds.insert(from.text(rule.var(document)));
            true
        });
        if !covered {
            continue;
        }
        let mut lowered = wave.rule().head_from(from, rule, rule.head());
        let mut demanded = demand.rule().head_from(from, rule, rule.head());
        // The chunk's documents seed each atom's demand.
        for d in &seeds {
            demanded = demanded.atom(WAVE, &[T::Var(*d)]);
        }
        for l in rule.body() {
            match (l.view(), atoms.get(&l.node())) {
                (Lit::Ast { document, node, .. }, Some(&i)) => {
                    let (d, n) = (T::Var(from.text(rule.var(document))), T::Var(from.text(rule.var(node))));
                    lowered = lowered.atom(MATCH, &[T::Int(i as i64), d.clone(), n.clone()]);
                    let ast = demanded.builder().rel(&format!("ast_{i}"));
                    demanded = demanded.atom(ast, &[d, n]);
                }
                _ => {
                    lowered = lowered.lit(from, rule, l);
                    demanded = demanded.lit(from, rule, l);
                }
            }
        }
        lowered.finish().map_err(invalid)?;
        demanded.finish().map_err(invalid)?;
        read.extend(reads(from, rule).map(str::to_owned));
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
    // The wave's header: the round trip's and the authored program's declarations and outputs, the authored
    // facts, and the relations the rules read as bound. The demand program's declarations are the wave's.
    for b in [&mut wave, &mut demand] {
        declare_from(b, &program);
        declare_from(b, authored);
        declare_violation(b);
        declare_read(b, &read);
    }
    let mut outputs = program.output_names();
    outputs.extend(authored.output_names().into_iter().filter(|o| !outputs.contains(o)).collect::<Vec<_>>());
    wave.set_outputs(&outputs);
    output_violation(&mut wave);
    for rel in authored.fact_names() {
        wave.set_facts(&rel, &authored.fact_rows(&rel));
    }
    let shared: Vec<(String, String)> = clauses.iter().map(|clause| clause.key.clone()).collect();
    if let Some(key) = shared.iter().enumerate().find_map(|(at, key)| shared[..at].contains(key).then_some(key)) {
        return Err(invalid(format!("clause {}/{} lowers twice: its violations would not tell its matchers apart", key.0, key.1)));
    }
    let demand = demand_program(demand, clauses.len())?;
    let bound = read.iter().filter(|rel| is_bound(rel)).filter_map(|rel| crate::code_rule_dsl::logic::sql::select::input_columns(rel).map(|columns| (rel.clone(), columns))).collect();
    Ok(Lowered { clauses, shapes, logic: Logic { rules: merged(&finished(wave)?)?, demand, bound }, program })
}

/// `violation("<policy>", "<clause>", …)`'s policy and clause, where `rule` derives one.
fn violation_key(v: &Version, rule: RuleRef<'_>) -> Option<(String, String)> {
    if v.rel_name(rule.head().rel()) != VIOLATION {
        return None;
    }
    let mut args = rule.head().args();
    match (args.next(), args.next()) {
        (Some(TermRef::Str(policy)), Some(TermRef::Str(clause))) => Some((v.text(policy).to_owned(), v.text(clause).to_owned())),
        _ => None,
    }
}

/// The demand program of the rules `rules` holds (each reading `ast_i` atoms after `wave` seeds): `.demand` on every
/// atom, T's magic sets (`KeyDemand`), then the magic rules alone, less what they cannot read before the batch, and
/// `demanded(i, d) :- magic_ast_i(d)`.
fn demand_program(mut rules: Builder<'_>, atoms: usize) -> StoreResult<Option<Arc<Version>>> {
    if atoms == 0 {
        return Ok(None);
    }
    let names: Vec<String> = (0..atoms).map(|i| format!("ast_{i}")).collect();
    rules.declare(WAVE, &[("d", "int")]);
    for name in &names {
        rules.declare(name, &[("d", "int"), ("n", "int")]);
    }
    rules.set_inputs(&std::iter::once(WAVE.to_owned()).chain(names.iter().cloned()).collect::<Vec<_>>());
    rules.set_field(Field::Demand(&names));
    let mut p = Pipeline::new("the policies' demand", finished(rules)?);
    p.program(&mut KeyDemand::default());
    let magic = p.finish().map_err(|e| invalid(format!("the policies' demand: {e}")))?.0;
    let head = |r: RuleRef<'_>| magic.rel_name(r.head().rel());
    // What the batch has not derived yet: the atoms, every input but the chunk's documents and the
    // `py.*` families (bound with the chunk's base relations, before the batch), and every relation
    // that reads one.
    let mut unavailable: BTreeSet<String> = names.iter().cloned().collect();
    unavailable.extend(magic.input_names().into_iter().filter(|rel| rel != WAVE && !rel.starts_with("ast_") && !is_bound(rel)));
    loop {
        let more = magic
            .rules()
            .filter(|&r| !head(r).starts_with("magic_") && !unavailable.contains(head(r)))
            .filter(|&r| r.body().any(|l| l.atom().is_some_and(|a| unavailable.contains(magic.rel_name(a.rel()))) || matches!(l.view(), Lit::Ast { .. })))
            .map(|r| head(r).to_owned())
            .collect::<Vec<_>>();
        if more.is_empty() {
            break;
        }
        unavailable.extend(more);
    }
    // The magic rules of the atoms, widened, and the available rules they read.
    let mut b = Builder::under(&magic);
    let mut referenced = BTreeSet::<String>::new();
    let mut needed = BTreeSet::<String>::new();
    for r in magic.rules().filter(|&r| head(r).starts_with("magic_ast_")) {
        referenced.insert(head(r).to_owned());
        let mut rule = b.rule().head_from(&magic, r, r.head());
        match widened(&magic, r, &unavailable) {
            Some(kept) => {
                for l in kept {
                    needed.extend(l.atom().map(|a| magic.rel_name(a.rel()).to_owned()));
                    rule = rule.lit(&magic, r, l);
                }
            }
            // The seed binds every atom's document: a key the widening unbound is demanded everywhere.
            None => {
                needed.insert(WAVE.to_owned());
                rule = rule.atom(WAVE, &r.head().args().map(|t| t_of(&magic, r, t)).collect::<Vec<_>>());
            }
        }
        rule.finish().map_err(invalid)?;
    }
    let mut seen = BTreeSet::new();
    while let Some(rel) = needed.iter().find(|rel| !seen.contains(*rel)).cloned() {
        seen.insert(rel.clone());
        for r in magic.rules().filter(|&r| head(r) == rel) {
            needed.extend(reads(&magic, r).map(str::to_owned));
            b.copy(&magic, r);
        }
    }
    referenced.extend(needed);
    for (i, name) in names.iter().enumerate() {
        let magic_ast = b.rel(&format!("magic_{name}"));
        b.rule().head(DEMANDED, &[T::Int(i as i64), T::Var("D")]).atom(magic_ast, &[T::Var("D")]).finish().map_err(invalid)?;
    }
    for rel in referenced.iter().filter(|rel| magic.declares(rel)) {
        let columns = magic.columns_of(rel).unwrap_or_default();
        b.declare(rel, &columns.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect::<Vec<_>>());
    }
    b.declare(DEMANDED, &[("c", "int"), ("d", "int")]);
    b.declare(WAVE, &[("d", "int")]);
    for name in &names {
        if !b.declares(&format!("magic_{name}")) {
            b.declare(&format!("magic_{name}"), &[("d", "int")]);
        }
    }
    for rel in magic.fact_names().into_iter().filter(|rel| referenced.contains(rel)) {
        b.set_facts(&rel, &magic.fact_rows(&rel));
    }
    // The chunk's documents and the `py.*` families the magic rules read are its inputs.
    b.set_inputs(&std::iter::once(WAVE.to_owned()).chain(referenced.iter().filter(|rel| is_bound(rel)).cloned()).collect::<Vec<_>>());
    b.set_outputs(&[DEMANDED]);
    let demand = finished(b)?;
    analysis::<Components>(&demand).map_err(|e| invalid(format!("the policies' demand: {e}")))?;
    Ok(Some(demand))
}

/// Magic rule `r`'s body less the literals the batch has not derived (`unavailable`), and less those left without
/// their variables bound: leaving a literal out of a magic rule only widens demand. `None` where the head's key is
/// left unbound.
fn widened<'v>(v: &'v Version, r: RuleRef<'v>, unavailable: &BTreeSet<String>) -> Option<Vec<LitRef<'v>>> {
    let positive = |l: LitRef<'_>| matches!(l.view(), Lit::Atom(a) if !a.neg());
    let mut kept: Vec<LitRef<'v>> = r.body().filter(|l| !l.atom().is_some_and(|a| unavailable.contains(v.rel_name(a.rel())))).collect();
    loop {
        let bound: BTreeSet<Slot> = kept.iter().filter(|&&l| positive(l)).flat_map(|&l| lit_vars(l)).collect();
        let before = kept.len();
        kept.retain(|&l| match l.view() {
            Lit::Atom(a) if !a.neg() => true,
            Lit::Nearest { n, a, .. } => bound.contains(&n) || bound.contains(&a),
            _ => lit_vars(l).is_subset(&bound),
        });
        if kept.len() == before {
            break;
        }
    }
    let bound: BTreeSet<Slot> = kept.iter().filter(|&&l| positive(l)).flat_map(|&l| lit_vars(l)).collect();
    atom_vars(r.head()).is_subset(&bound).then_some(kept)
}

#[cfg(test)]
mod tests;
