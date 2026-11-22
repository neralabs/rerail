//! Bind independent node ranges to the tuples their readers can actually use.
//!
//! An intermediate product is safe to restrict to a superset of every read's keys. Build that superset from
//! each reader's connected positive context. When that context would itself form a product, require keys from
//! the reader's callers first. Reject a product with a reader that cannot supply a connected demand.

use crate::code_rule_dsl::logic::front::{LatticeDecl, Native, Value};
use crate::code_rule_dsl::logic::front::name::List;
use crate::code_rule_dsl::logic::plan::body::{VarSet, atom_vars, lit_vars, term_vars};
use crate::code_rule_dsl::logic::program_image::fields::ProgramFields as F;
use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{AtomRef, Builder, CmpOp, Lit, RuleBuilder, RuleId, RuleRef, Slot, T, TermRef, Version, t_of};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub mod deferred;
pub mod nullable;
pub mod upstream;

/// The fields the proofs read, of the program's version as it is now.
#[derive(Clone)]
pub(super) struct Fields {
    pub(super) inputs: Vec<String>,
    pub(super) outputs: Vec<String>,
    pub(super) facts: BTreeSet<String>,
    pub(super) natives: BTreeMap<String, Native>,
    pub(super) answers: BTreeMap<String, Vec<String>>,
    pub(super) asked_by: BTreeMap<String, Vec<BTreeSet<String>>>,
    pub(super) demand: Vec<String>,
    pub(super) lattice: BTreeMap<String, LatticeDecl>,
    pub(super) qualifiers: BTreeMap<String, BTreeSet<String>>,
    pub(super) gates: Vec<(String, crate::code_rule_dsl::logic::front::ast::Gate)>,
    pub(super) layouts: BTreeMap<String, Vec<crate::code_rule_dsl::logic::front::LayoutFact>>,
}


/// The demand relations' prefix, and the deferred pairs' (the planner reads them in a rule's body, `body::own_demand`).
pub(in crate::code_rule_dsl::logic::plan) use crate::code_rule_dsl::logic::plan::body::{DEFERRED_PAIRS, PRODUCT_DEMAND as PREFIX};

impl Fields {
    fn of(v: &Version) -> Fields {
        Fields {
            inputs: F::input_names(v),
            outputs: F::output_names(v),
            facts: F::fact_names(v).into_iter().collect(),
            natives: F::natives(v),
            answers: F::answers(v),
            asked_by: F::asked(v),
            demand: F::demand(v),
            lattice: F::lattice(v),
            qualifiers: F::qualifiers(v),
            gates: F::gates(v),
            layouts: F::layouts(v),
        }
    }
}

/// The program the pass rewrites: its input, and the version its rewrites so far made (`step`), a derivation of the
/// input and then of each step's, whose rules are read in place. A rule the input has is recorded as each
/// derivation's rule's source (`origin`); a refused step is a version dropped.
#[derive(Clone)]
pub struct Program<'i> {
    input: &'i Version,
    pub made: Option<Arc<Version>>,
    pub(super) f: Fields,
}

/// A part of a rule's body a demand reads (`Context`): a guard the rule is given (`guard`), or its literal `i`.
#[derive(Clone, Copy, PartialEq)]
pub enum Part {
    Guard,
    Lit(usize),
}

/// The input's id of rule `r` of a version of the program: `r`'s own in the input (`first`), else its source.
fn origin(first: bool, r: RuleRef<'_>) -> Option<RuleId> {
    if first { Some(r.id()) } else { r.source() }
}

/// A rule rewritten from `v`'s rule `r` into `b`, its source recorded (`origin`): its head, then what the caller
/// writes.
pub fn rewritten<'b, 'p>(first: bool, v: &Version, r: RuleRef<'_>, b: &'b mut Builder<'p>) -> RuleBuilder<'b, 'p> {
    let rb = b.rule();
    let rb = match origin(first, r) {
        Some(id) => rb.source(id),
        None => rb,
    };
    rb.head_from(v, r, r.head())
}

impl<'i> Program<'i> {
    /// The pass's input, not rewritten yet.
    pub fn of(input: &'i Version) -> Program<'i> {
        Program { input, made: None, f: Fields::of(input) }
    }

    /// The version as the rewrites have made it.
    pub fn v(&self) -> &Version {
        self.made.as_deref().unwrap_or(self.input)
    }

    /// Whether the version is the input's (no step made yet).
    pub fn first(&self) -> bool {
        self.made.is_none()
    }

    /// The relation rule `r`'s head writes.
    pub(super) fn head_rel(&self, r: RuleRef<'_>) -> &str {
        self.v().rel_name(r.head().rel())
    }

    /// `rel`'s columns, where it is declared.
    pub fn columns(&self, rel: &str) -> Option<Vec<(String, String)>> {
        F::columns_of(self.v(), rel)
    }

    /// Whether `.decl rel(…)` carries the qualifier `word`.
    pub(super) fn qualified(&self, rel: &str, word: &str) -> bool {
        self.f.qualifiers.get(rel).is_some_and(|q| q.contains(word))
    }

    /// A derivation of the version: each rule `rewrite` writes itself (it says whether it did), else kept as it is;
    /// then what `tail` writes (rules after them, fields).
    fn derive(
        &self,
        rewrite: &mut dyn FnMut(bool, &Version, RuleRef<'_>, &mut Builder<'_>) -> Result<bool, String>,
        tail: &mut dyn FnMut(&Version, &mut Builder<'_>) -> Result<(), String>,
    ) -> Result<Arc<Version>, String> {
        let (v, first) = (self.v(), self.first());
        let mut b = Builder::derive(v);
        for r in v.rules() {
            if !rewrite(first, v, r, &mut b)? {
                if first {
                    b.keep(v, r);
                } else {
                    b.copy_as_is(v, r);
                }
            }
        }
        tail(v, &mut b)?;
        Ok(b.finish()?.0)
    }

    /// The version `v` a step made, now the program's.
    fn set(&mut self, v: Arc<Version>) {
        self.f = Fields::of(&v);
        self.made = Some(v);
    }

    /// Each rule, as `show_rule` writes it, of relation `rel` (all of them: `None`): what a test reads.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn shown(&self, rel: Option<&str>) -> Vec<String> {
        let v = self.v();
        v.rules().filter(|r| rel.is_none_or(|rel| self.head_rel(*r) == rel)).map(|r| v.show_rule(r)).collect()
    }

    /// Whether a step changed the program.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn changed(&self) -> bool {
        !self.first()
    }

    /// The components, dependencies first, of version `v`'s rules (`front::graph::components_by_name`).
    fn components(v: &Version) -> Result<Vec<BTreeSet<String>>, crate::code_rule_dsl::logic::front::Error> {
        crate::code_rule_dsl::logic::front::graph::components_by_name(v, &crate::code_rule_dsl::logic::program_image::about::About::of(v))
    }
}


/// Check the image for a possible product before decoding rules for the demand analysis.
fn may_have_products(version: &Version) -> bool {
    fn vars(term: TermRef<'_>, out: &mut BTreeSet<Slot>) {
        match term {
            TermRef::Var(slot) => { out.insert(slot); }
            TermRef::Call(_, args) => args.for_each(|arg| vars(arg, out)),
            _ => {}
        }
    }
    version.rules().any(|rule| {
        let head = rule.head();
        let Some(decl) = version.decl_of(version.rel_name(head.rel())) else { return false };
        let nodes: BTreeSet<Slot> = head.args().zip(decl.cols())
            .filter_map(|(term, (_, ty))| matches!(version.text(ty), "node" | "loc").then(|| term.var()).flatten())
            .collect();
        if nodes.len() < 2 {
            return false;
        }
        let mut groups: Vec<BTreeSet<Slot>> = Vec::new();
        for literal in rule.body() {
            let mut group = BTreeSet::new();
            match literal.view() {
                Lit::Atom(atom) if !atom.neg() => atom.args().for_each(|term| vars(term, &mut group)),
                Lit::Cmp { l, op: CmpOp::Eq, r } => {
                    vars(l, &mut group);
                    vars(r, &mut group);
                }
                Lit::Agg { var, op, over, atom } => {
                    group.insert(var);
                    group.extend(over);
                    group.extend(op.bound());
                    atom.args().for_each(|term| vars(term, &mut group));
                }
                Lit::Nearest { n, a, .. } => group.extend([n, a]),
                Lit::Ast { document, node, captures, .. } => {
                    group.extend([document, node]);
                    group.extend(captures.map(|(_, slot)| slot));
                }
                _ => continue,
            }
            groups.retain_mut(|other| {
                if group.is_disjoint(other) { return true; }
                group.append(other);
                false
            });
            if nodes.is_subset(&group) {
                return false;
            }
            groups.push(group);
        }
        groups.iter().filter(|group| !group.is_disjoint(&nodes)).take(2).count() == 2
    })
}

/// The analysis reads simplified rules when the image contains a possible product.
/// Publish an edit of the version with the source of every replaced rule.
pub(super) enum BoundProducts {
    Whole,
    Linked,
}

impl ProgramPass for BoundProducts {
    const NAME: &'static str = "bound node products by their readers";

    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let version: &Version = cx.v();
        if matches!(self, Self::Whole) && !may_have_products(version) {
            return Ok(());
        }
        let mut program = Program::of(version);
        let rewritten = rewrite(&mut program, true)?;
        if rewritten.products.is_empty() {
            return Ok(());
        }
        publish(version, program.v(), w)?;
        crate::code_rule_dsl::logic::profile::decision("node Cartesian products", || {
            format!("restricted to connected reader demands: {}", rewritten.products.into_iter().collect::<Vec<_>>().join(", "))
        });
        if !rewritten.readers.is_empty() {
            crate::code_rule_dsl::logic::profile::decision("projected reader demand", || {
                format!("{} ({} ordered node and {} present location witnesses)", rewritten.readers.into_iter().collect::<Vec<_>>().join(", "), rewritten.order_witnesses, rewritten.present_witnesses)
            });
        }
        Ok(())
    }
}

/// `made`, a derivation of `version` by the steps (`Program`), published as an edit of it: the fields the steps
/// changed, through the fields API (decision 0276 §3): declarations made and dropped, qualifiers, nullary relations,
/// facts. A rule kept as it was stays; each other input rule is replaced by what was made from it (its source
/// recorded), or removed where nothing was; the rules the steps added follow.
fn publish(version: &Version, made: &Version, w: &mut ProgramW<'_>) -> Result<(), String> {
    use crate::code_rule_dsl::logic::program_image::fields::FieldsMut as _;
    let builder = w.on()?;
    for rel in F::declared_names(made) {
        let cols = F::columns_of(made, &rel).unwrap_or_default();
        if F::columns_of(version, &rel).as_ref() != Some(&cols) {
            let pairs: Vec<(&str, &str)> = cols.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
            builder.fields().declare(&rel, &pairs);
        }
    }
    for rel in F::declared_names(version) {
        if F::columns_of(made, &rel).is_none() {
            builder.fields().undeclare(&rel);
        }
    }
    let qualifiers = F::qualifiers(made);
    if qualifiers != F::qualifiers(version) {
        builder.fields().set_field(crate::code_rule_dsl::logic::program_facts::Field::Qualifiers(&qualifiers));
    }
    let (was, now) = (version.nullary_names()?, made.nullary_names()?);
    let added: Vec<&String> = now.difference(&was).collect();
    if !added.is_empty() {
        builder.fields().add_nullary(&added);
    }
    for rel in F::fact_names(made) {
        let rows = F::fact_rows(made, &rel);
        if !F::states_facts(version, &rel) || F::fact_rows(version, &rel) != rows {
            builder.fields().set_facts(&rel, &rows);
        }
    }
    let mut from: BTreeMap<RuleId, Vec<RuleRef<'_>>> = BTreeMap::new();
    let mut added = Vec::new();
    for r in made.rules() {
        match r.source().filter(|id| version.rule(*id).is_some()) {
            Some(id) => from.entry(id).or_default().push(r),
            None => added.push(r),
        }
    }
    for was in version.rules() {
        match from.get(&was.id()).map(Vec::as_slice) {
            Some([r]) if r.hash() == was.hash() => {}
            Some(rules) => {
                builder.remove(was.id());
                for r in rules {
                    builder.copy_from(made, *r, was.id());
                }
            }
            None => builder.remove(was.id()),
        }
    }
    for r in added {
        builder.copy(made, r);
    }
    Ok(())
}

/// The parts of a rule's body that share variables (`connect`): their variables, and whether one reads the corpus.
#[derive(Default)]
pub struct Context {
    vars: VarSet,
    body: Vec<Part>,
    reads_input: bool,
}

pub fn connect(groups: &mut Vec<Context>, part: Part, vars: VarSet, reads_input: bool) {
    if vars.is_empty() {
        return;
    }
    let mut joined = Context { vars, body: vec![part], reads_input };
    groups.retain_mut(|other| {
        if joined.vars.is_disjoint(&other.vars) {
            return true;
        }
        joined.vars.extend(&other.vars);
        joined.body.append(&mut other.body);
        joined.reads_input |= other.reads_input;
        false
    });
    groups.push(joined);
}

/// Relations that depend on the corpus, as opposed to constants defined by the program.
fn corpus_relations<'v>(program: &'v Program<'_>) -> BTreeSet<&'v str> {
    let v = program.v();
    let mut readers: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut pending: Vec<&str> = program.f.inputs.iter().map(String::as_str).collect();
    for rule in v.rules() {
        let head = v.rel_name(rule.head().rel());
        for literal in rule.body() {
            match literal.view() {
                Lit::Atom(a) if !a.neg() => readers.entry(v.rel_name(a.rel())).or_default().push(head),
                Lit::Ast { .. } | Lit::Nearest { .. } => pending.push(head),
                _ => {}
            }
        }
    }
    let mut varies = BTreeSet::new();
    while let Some(rel) = pending.pop() {
        if varies.insert(rel) {
            pending.extend(readers.get(rel).into_iter().flatten().copied());
        }
    }
    varies
}

pub fn plain(term: TermRef<'_>) -> bool {
    matches!(term, TermRef::Var(_) | TermRef::Int(_) | TermRef::Str(_) | TermRef::Branded(_))
}

/// Whether plain terms `a` and `b` of one rule are the same term.
pub fn same(a: TermRef<'_>, b: TermRef<'_>) -> bool {
    match (a, b) {
        (TermRef::Var(x), TermRef::Var(y)) => x == y,
        (TermRef::Int(x), TermRef::Int(y)) => x == y,
        (TermRef::Str(x), TermRef::Str(y)) => x == y,
        (TermRef::Branded(x), TermRef::Branded(y)) => x == y,
        (TermRef::Wild, TermRef::Wild) | (TermRef::Top, TermRef::Top) => true,
        _ => false,
    }
}

/// Constant term `t` of `v` as a fact's value.
pub fn value(v: &Version, t: TermRef<'_>) -> Value {
    match t {
        TermRef::Int(n) => Value::Int(n),
        TermRef::Str(s) => Value::Str(v.text(s).into()),
        TermRef::Branded(n) => Value::Branded(n),
        _ => unreachable!("a constant key"),
    }
}

/// The variables of `rule` that hold few values whatever the corpus: set to a constant, bound only by lookups on
/// constants, or reached from constants (and such variables) through a relation's declared key or few-row fan-out
/// (`.layout`, decision 0361). A node variable fixed so is no side of a product: `qv("builtins.object", "cls", c)`
/// gives the few class nodes of one name, and a rule pairing them with every reference to `object` is as large as
/// the references (Q249).
fn fixed_vars(rule: RuleRef<'_>, program: &Program<'_>) -> BTreeSet<Slot> {
    use crate::code_rule_dsl::logic::front::{FEW, Fan, LayoutFact};
    let v = program.v();
    let mut fixed: BTreeSet<Slot> = rule
        .body()
        .filter_map(|literal| match literal.view() {
            Lit::Cmp { l: TermRef::Var(x), op: CmpOp::Eq, r } if plain(r) && !matches!(r, TermRef::Var(_)) => Some(x),
            _ => None,
        })
        .collect();
    // A variable bound only by lookups on constants (`qv("builtins.object", "cls", $c)`): every positive atom that
    // reads it reads no other variable and no wildcard, and at least one does. Fixed as `$c = constant` is: a
    // product of it is one side times that lookup's rows (356f1b516, `calls.rerail`).
    let positive: Vec<AtomRef<'_>> = rule.body().filter_map(|l| match l.view() {
        Lit::Atom(a) if !a.neg() => Some(a),
        _ => None,
    }).collect();
    let constant = |t: TermRef<'_>| plain(t) && !matches!(t, TermRef::Var(_));
    for a in &positive {
        let Some(v) = a.args().find_map(|t| match t {
            TermRef::Var(x) => Some(x),
            _ => None,
        }) else {
            continue;
        };
        let lookup = |a: &AtomRef<'_>| a.args().all(|t| matches!(t, TermRef::Var(x) if x == v) || constant(t)) && a.args().any(constant);
        let reads = |a: &&AtomRef<'_>| a.args().any(|t| matches!(t, TermRef::Var(x) if x == v));
        if lookup(a) && positive.iter().filter(reads).all(lookup) {
            fixed.insert(v);
        }
    }
    // Reached through a declared key or a few-row fan-out, to a fixed point.
    loop {
        let before = fixed.len();
        for a in &positive {
            let Some(facts) = program.f.layouts.get(v.rel_name(a.rel())) else { continue };
            let args: Vec<TermRef<'_>> = a.args().collect();
            let bound = |i: &usize| match args.get(*i) {
                Some(TermRef::Var(x)) => fixed.contains(x),
                Some(TermRef::Wild) | None => false,
                Some(t) => plain(*t),
            };
            let mut reached: Vec<usize> = Vec::new();
            for fact in facts {
                match fact {
                    LayoutFact::Key(cols) if cols.iter().all(bound) => reached.extend(0..args.len()),
                    LayoutFact::Fanout { from, to, fan } if from.iter().all(bound) && (matches!(fan, Fan::One | Fan::Few) || matches!(fan, Fan::About(n) if *n <= FEW)) => reached.extend(to),
                    _ => {}
                }
            }
            for i in reached {
                if let Some(TermRef::Var(x)) = args.get(i) {
                    fixed.insert(*x);
                }
            }
        }
        if fixed.len() == before {
            return fixed;
        }
    }
}

fn disconnected_nodes(rule: RuleRef<'_>, program: &Program<'_>, varies: &BTreeSet<&str>) -> bool {
    let v = program.v();
    let fixed = fixed_vars(rule, program);
    let cols = program.columns(program.head_rel(rule)).unwrap_or_default();
    let node_vars: BTreeSet<Slot> = rule
        .head()
        .args()
        .zip(&cols)
        .filter_map(|(term, (_, ty))| match term {
            TermRef::Var(x) if matches!(ty.as_str(), "node" | "loc") && !fixed.contains(&x) => Some(x),
            _ => None,
        })
        .collect();
    if node_vars.len() < 2 {
        return false;
    }
    let mut groups = Vec::new();
    for (i, literal) in rule.body().enumerate() {
        let reads_input = match literal.view() {
            Lit::Atom(a) if !a.neg() => varies.contains(v.rel_name(a.rel())),
            Lit::Agg { atom, .. } => varies.contains(v.rel_name(atom.rel())),
            Lit::Nearest { .. } | Lit::Ast { .. } => true,
            Lit::Cmp { op: CmpOp::Eq, .. } => false,
            _ => continue,
        };
        connect(&mut groups, Part::Lit(i), lit_vars(literal), reads_input);
    }
    groups.iter().filter(|group| group.reads_input && group.vars.iter().any(|x| node_vars.contains(&x))).take(2).count() == 2
}

fn refusal(relation: &str, reason: impl std::fmt::Display) -> String {
    let source = crate::code_rule_dsl::logic::profile::author_joins::source_name(relation, None);
    format!("unbounded node Cartesian product in `{source}`: {reason}")
}

/// The connected parts of `rule`'s body a demand may read: its positive atoms of no demanded relation and no computed
/// argument, and its guard (`guard`: the guard's variables) where it has one.
fn contexts(program: &Program<'_>, rule: RuleRef<'_>, guard: Option<VarSet>, demanded: &BTreeSet<String>) -> Vec<Context> {
    let v = program.v();
    let mut groups = Vec::new();
    if let Some(vars) = guard {
        connect(&mut groups, Part::Guard, vars, false);
    }
    for (i, literal) in rule.body().enumerate() {
        // A demand can only use independent evidence. Ignore filters, aggregate results, and the relations
        // whose answers are being restricted. Dropping these conditions only adds keys.
        if let Lit::Atom(a) = literal.view()
            && !a.neg()
            && !demanded.contains(v.rel_name(a.rel()))
            && a.args().all(|t| !matches!(t, TermRef::Call(..)))
        {
            connect(&mut groups, Part::Lit(i), atom_vars(a), false);
        }
    }
    groups
}

fn check_target(program: &Program<'_>, rel: &str) -> Result<(), String> {
    let f = &program.f;
    if f.outputs.iter().any(|r| r == rel)
        || f.inputs.iter().any(|r| r == rel)
        || f.facts.contains(rel)
        || f.lattice.contains_key(rel)
        || f.natives.contains_key(rel)
        || f.asked_by.contains_key(rel)
        || program.qualified(rel, "no_magic")
    {
        return Err(refusal(rel, "the relation must be computed in full"));
    }
    if program.columns(&format!("{PREFIX}{rel}")).is_some() {
        return Err(refusal(rel, "the demand relation's name is already in use"));
    }
    let v = program.v();
    if v.rules().filter(|r| program.head_rel(*r) == rel).any(|r| !r.head().args().all(plain)) {
        return Err(refusal(rel, "a computed head cannot be bounded by a tuple demand"));
    }
    Ok(())
}

/// The terms of `a`'s arguments in rule `r`, those at `cols` (all of them: `None`).
fn args_at<'v>(r: RuleRef<'v>, a: AtomRef<'v>, cols: Option<&[usize]>) -> Vec<T<'v>> {
    let args: Vec<TermRef<'_>> = a.args().collect();
    match cols {
        Some(cols) => cols.iter().map(|&c| t_of(r, args[c])).collect(),
        None => args.into_iter().map(|t| t_of(r, t)).collect(),
    }
}

/// A demand's rule, of a read in rule `r` of `v`: head `rel(head)`, its body `parts` of `r` (`Part::Guard` written
/// as `guard`), in order.
pub fn write_demand(v: &Version, r: RuleRef<'_>, b: &mut Builder<'_>, rel: &str, head: &[T<'_>], guard: (&str, &[T<'_>]), parts: &[Part]) -> Result<(), String> {
    let body: Vec<_> = r.body().collect();
    let mut rb = b.rule().head(rel, head);
    for part in parts {
        rb = match *part {
            Part::Guard => rb.atom(guard.0, guard.1),
            Part::Lit(i) => rb.lit(v, r, body[i]),
        };
    }
    rb.finish().map(drop)
}

/// Guard the products before splitting the program by document or planning its joins.
pub fn rewrite(program: &mut Program<'_>, narrow_readers: bool) -> Result<Rewritten, String> {
    let varies = corpus_relations(program);
    let products: BTreeSet<String> = {
        let v = program.v();
        v.rules()
            .filter(|rule| program.columns(program.head_rel(*rule)).is_some() && disconnected_nodes(*rule, program, &varies))
            .map(|rule| program.head_rel(rule).to_owned())
            .collect()
    };
    // A refusal names the rules that make the products: a relation demanded for one may be far from it.
    let named = |e: String| -> String {
        let v = program.v();
        let rules: Vec<String> = v.rules().filter(|r| products.contains(program.head_rel(*r)) && disconnected_nodes(*r, program, &varies)).map(|r| v.show_rule(r)).collect();
        format!("{e}; the products: {}", rules.join("; "))
    };
    if narrow_readers && !products.is_empty() {
        let mut strict = program.clone();
        let products = rewrite_products(&mut strict, products.clone()).map_err(named)?;
        let mut trial = program.clone();
        let mut readers = upstream::start(&mut trial, &products)?;
        if !readers.is_empty()
            && let Ok(products) = rewrite_products(&mut trial, products.clone()) {
                upstream::propagate(&mut trial, &products, &mut readers)?;
                let order_witnesses = upstream::restore_order_witnesses(&mut trial, &readers)?;
                let present_witnesses = nullable::restore(&mut trial)?;
                *program = trial;
                return Ok(Rewritten { products, readers, order_witnesses, present_witnesses });
            }
        *program = strict;
        return Ok(Rewritten { products, readers: BTreeSet::new(), order_witnesses: 0, present_witnesses: 0 });
    }
    let refused = products.clone();
    let named = |e: String| -> String {
        let v = program.v();
        let rules: Vec<String> = v.rules().filter(|r| refused.contains(program.head_rel(*r)) && disconnected_nodes(*r, program, &varies)).map(|r| v.show_rule(r)).collect();
        format!("{e}; the products: {}", rules.join("; "))
    };
    let mut changed = program.clone();
    let products = rewrite_products(&mut changed, products).map_err(named)?;
    *program = changed;
    Ok(Rewritten { products, readers: BTreeSet::new(), order_witnesses: 0, present_witnesses: 0 })
}

pub struct Rewritten {
    pub products: BTreeSet<String>,
    pub readers: BTreeSet<String>,
    pub order_witnesses: usize,
    pub present_witnesses: usize,
}

fn rewrite_products(program: &mut Program<'_>, mut demanded: BTreeSet<String>) -> Result<BTreeSet<String>, String> {
    if demanded.is_empty() {
        return Ok(demanded);
    }
    for rel in &demanded {
        check_target(program, rel)?;
    }

    loop {
        let v = program.v();
        let mut more = BTreeSet::new();
        // Each demand's rule: its reader, the read, and the parts of the reader it reads.
        let mut demands: Vec<(RuleId, usize, Vec<Part>)> = Vec::new();
        let mut facts = BTreeMap::<String, Vec<List<Value>>>::new();
        for reader in v.rules() {
            let reads: Vec<(usize, AtomRef<'_>, bool)> = reader
                .body()
                .enumerate()
                .filter_map(|(i, l)| {
                    let a = l.atom()?;
                    demanded.contains(v.rel_name(a.rel())).then_some((i, a, matches!(l.view(), Lit::Atom(_))))
                })
                .collect();
            if reads.is_empty() {
                continue;
            }
            let head_rel = program.head_rel(reader);
            let own = demanded.contains(head_rel);
            let guard_vars = atom_vars(reader.head());
            let available = contexts(program, reader, own.then(|| guard_vars.clone()), &demanded);
            for (i, a, is_atom) in reads {
                let rel = v.rel_name(a.rel());
                let from_reader = || {
                    let source = crate::code_rule_dsl::logic::profile::author_joins::source_name(head_rel, Some(rel));
                    let columns = program.columns(rel).unwrap_or_default().into_iter().map(|(name, _)| name).collect::<Vec<_>>().join(", ");
                    format!("the read in `{source}` has no connected demand for ({columns})")
                };
                if !is_atom || a.gated() || !a.args().all(plain) {
                    return Err(refusal(rel, from_reader()));
                }
                let name = format!("{PREFIX}{rel}");
                let keys = atom_vars(a);
                if keys.is_empty() {
                    let row = a.args().map(|t| value(v, t)).collect();
                    facts.entry(name).or_default().push(row);
                } else if let Some(source) = available.iter().find(|c| keys.is_subset(&c.vars)) {
                    demands.push((reader.id(), i, source.body.clone()));
                } else if !own && contexts(program, reader, Some(guard_vars.clone()), &demanded).iter().any(|c| keys.is_subset(&c.vars)) {
                    // Its own demand must key the read: it is bounded too, where it may be.
                    check_target(program, head_rel).map_err(|e| format!("{e}, to key its read of `{rel}` (in {})", v.show_rule(reader)))?;
                    more.insert(head_rel.to_owned());
                } else {
                    return Err(refusal(rel, from_reader()));
                }
            }
        }
        if !more.is_empty() {
            demanded.extend(more);
            continue;
        }

        let mut qualifiers = program.f.qualifiers.clone();
        for rel in &demanded {
            qualifiers.entry(format!("{PREFIX}{rel}")).or_default().extend(["no_inline".to_owned(), "no_magic".to_owned()]);
        }
        let decls: Vec<(String, Vec<(String, String)>)> = demanded.iter().map(|rel| (format!("{PREFIX}{rel}"), program.columns(rel).unwrap_or_default())).collect();
        let made = program.derive(
            &mut |first, v, r, b| {
                let rel = v.rel_name(r.head().rel());
                if !demanded.contains(rel) {
                    return Ok(false);
                }
                // The product's rule reads its demand first.
                let (name, guard) = (format!("{PREFIX}{rel}"), args_at(r, r.head(), None));
                let mut rb = rewritten(first, v, r, b).atom(name.as_str(), &guard);
                for l in r.body() {
                    rb = rb.lit(v, r, l);
                }
                rb.finish().map(|_| true)
            },
            &mut |v, b| {
                use crate::code_rule_dsl::logic::program_image::fields::FieldsMut as _;
                for (rel, cols) in &decls {
                    let pairs: Vec<(&str, &str)> = cols.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
                    b.fields().declare(rel, &pairs);
                }
                b.fields().set_field(crate::code_rule_dsl::logic::program_facts::Field::Qualifiers(&qualifiers));
                for (reader, i, parts) in &demands {
                    let r = v.rule(*reader).expect("a reader of the version");
                    let a = r.body().nth(*i).and_then(|l| l.atom()).expect("the read");
                    let rel = format!("{PREFIX}{}", v.rel_name(a.rel()));
                    let guard_rel = format!("{PREFIX}{}", v.rel_name(r.head().rel()));
                    let (head, guard) = (args_at(r, a, None), args_at(r, r.head(), None));
                    write_demand(v, r, b, &rel, &head, (&guard_rel, &guard), parts)?;
                }
                for (rel, rows) in &facts {
                    b.fields().set_facts(rel, rows);
                }
                Ok(())
            },
        )?;
        let components = Program::components(&made).map_err(|e| refusal(demanded.first().expect("at least one product"), e))?;
        for component in components.iter().filter(|c| c.len() > 1) {
            if let Some(rel) = component.iter().find(|r| demanded.contains(*r) && component.contains(&format!("{PREFIX}{r}"))) {
                return Err(refusal(rel, "the demand would depend on the product it bounds"));
            }
        }
        program.set(made);
        return Ok(demanded);
    }
}

// cratify: `tests` is rerail-logic's (it mounts it: it reads the language, `rr::parse_version`, or the corpus's root)
