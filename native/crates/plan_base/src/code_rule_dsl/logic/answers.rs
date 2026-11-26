//! Requests answered inside the fixpoint (decision 0065 §2a).
//!
//! A program's `answers` name request relations (`rr_dir_req`, `load`, …) and what an answer adds rows to
//! (`dir_entry`, `loaded`, a loaded document's per-file rows). `analyses::strata` counts each produced relation as
//! reading its request, so a request that depends on its own answers is one recursive component with them, and the
//! native fixpoint pauses for the runner to answer (`setfix`'s `request` and `produced`).
//!
//! A negation inside such a component is not stratified: its relation grows as answers come in. [`gate`] makes it
//! wait, per key: `!N(t)` becomes `done_N_ad(t), !N(t)`, where `ask_N_ad(t)` holds the keys the rule asks about
//! (derived from the literals before the negation) and the engine copies a key into `done_N_ad` only in a wave after
//! which nothing N reads changed and no request is unanswered (the gate demand uses, `magic::gate_negations`). This
//! is sound when `N(t)` reads only answers that `t` determines: an answered request is whole, and later answers add
//! new keys rather than rows under an old one. Import resolution is of that kind.
//!
//! An aggregate inside the component waits the same way (the user, 2026-11-20: "Gate aggregates per key", Q409): its
//! key is the columns of its atom that the literals before it bind, the group it aggregates. `join x : { child(e, s,
//! i), value(s, x) }` over a document's nodes is whole once the document is: a parsed string's (decision 0431 §8) is
//! answered whole, as a loaded file is.
//!
//! Both passes are made on the program's version (decision 0241's form), reading its rules by their handles and its
//! fields, never a tree.

use rerail_blocks::vector::Vector;
use super::front::{Edges, path, tarjan};
use super::front::ast::Gate;
use super::plan::body::Placed;
use super::plan::fields::{Fields, FieldsData};
use super::program_image::manager::{Cx, Pipeline, ProgramPass, ProgramW, RuleOut, RulePass, Select, Step};
use super::program_image::passes::{PartOf, Retain, SetAnswers};
use super::program_image::{AtomRef, Builder, Lit, LitRef, PartFields, Rewrite, RuleBuilder, RuleRef, T, TermRef, Version};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use crate::code_rule_dsl::logic::program_image::passes::Parts as _;

/// An answer's rows under `HELD_BACK` + a request's name are request rows the answerer holds back: asked again once
/// the stratum is otherwise quiet (`setfix::defer`; the load budget's levels, decision 0145).
pub const HELD_BACK: &str = "rr_held_back:";

/// Whether `rel` is a gate's `done` relation (`gate`): a test of keys its rule's literals bind, which the planner
/// joins only once they are bound.
pub fn is_gate(rel: &str) -> bool {
    rel.starts_with("done_")
}

/// A term of `v`'s rule `r`, to write into a builder under `v`'s ids.
fn t_of<'v>(v: &'v Version, r: RuleRef<'v>, t: TermRef<'v>) -> T<'v> {
    match t {
        TermRef::Var(s) => T::VarAtom(r.var(s)),
        TermRef::Wild => T::Wild,
        TermRef::Top => T::Top,
        TermRef::Int(i) => T::Int(i),
        TermRef::Branded(b) => T::Branded(b),
        TermRef::Str(a) => T::StrAtom(a),
        TermRef::Call(f, args) => T::Call(v.text(f), args.map(|a| t_of(v, r, a)).collect()),
    }
}

/// A negated atom, or an aggregate's, copied gated.
struct Gated<'v>(&'v Version);

impl Rewrite for Gated<'_> {
    fn atom_as(&mut self, a: AtomRef<'_>) -> Option<(String, bool, bool)> {
        Some((self.0.rel_name(a.rel()).to_owned(), a.neg(), true))
    }
}

/// A literal of a gated rule (`gate`): one of the rule's, copied; a gate's test (`done_N(0)`, or `done_N_ad` over
/// the key, the negated literal's arguments at those places); or the negated literal, gated.
enum Out {
    Copy(usize),
    DoneAll(String),
    Done(String, Vector<usize>, usize),
    Gated(usize),
}

/// A gate `gate` makes: its `done` relation, its `ask` (none: the whole relation's gate, `done_N(0)`), the negated
/// relation, and the key's columns.
type Made = (String, Option<String>, String, Vec<(String, String)>);

/// Literal `o` of a gated rule, written into `rb` (`GateRules::rule`): its rule's body, or an `ask`'s.
fn write<'b, 'p>(v: &Version, r: RuleRef<'_>, lits: &[LitRef<'_>], rb: RuleBuilder<'b, 'p>, o: &'b Out) -> RuleBuilder<'b, 'p> {
    match o {
        Out::Copy(i) => rb.lit(v, r, lits[*i]),
        Out::DoneAll(done) => rb.atom(done.as_str(), &[T::Int(0)]),
        Out::Done(done, at, i) => {
            let args: Vec<TermRef<'_>> = lits[*i].atom().expect("a negated or aggregated atom").args().collect();
            let key: Vec<T<'_>> = at.iter().map(|&k| t_of(v, r, args[k])).collect();
            rb.atom(done.as_str(), &key)
        }
        Out::Gated(i) => rb.lit_with(v, r, lits[*i], &mut Gated(v)),
    }
}

/// `p`'s version (a whole program's: its rules and fields) with `answers` stated, and the negations inside each
/// component that holds a request gated (module docs). Two passes (decision 0281): `GateRules`, a derivation of it
/// (decision 0241's form), each rule it leaves as it was kept by its handle and each rewritten or made written from
/// it, in order, with the declarations it adds; then `GateFields`, an overlay of the gates and the answers. Where no
/// negation is gated, an overlay of the answers alone (`SetAnswers`).
pub fn gate(p: &Placed, answers: &BTreeMap<String, Vec<String>>) -> Result<Arc<Version>, String> {
    let mut pipe = Pipeline::new("gate", p.v.clone());
    let mut rules = GateRules { answers, produced: BTreeMap::new(), sccs: Vec::new(), comp: BTreeMap::new(), answered: BTreeSet::new(), edges: Edges::new(), fields: None, made: Vec::new(), shown: BTreeSet::new() };
    if !answers.is_empty() {
        if !p.whole {
            return Err("the requests of a version of rules alone".into());
        }
        pipe.rules(&mut rules);
    }
    if rules.answered.is_empty() {
        pipe.program(&mut SetAnswers(answers));
    } else {
        pipe.program(&mut GateFields { rules: &rules });
    }
    Ok(pipe.finish()?.0)
}

/// The gate's rewrite of the rules (`gate`): the components found once (`begin`), each rule inside an answered one
/// rewritten alone, its negations of the component gated, and the gates' rules written once (`finish`).
struct GateRules<'a> {
    answers: &'a BTreeMap<String, Vec<String>>,
    /// Each relation an answer produces, and its request.
    produced: BTreeMap<String, String>,
    sccs: Vec<BTreeSet<String>>,
    /// Each relation's component.
    comp: BTreeMap<String, usize>,
    /// The components that hold a request and what answers it.
    answered: BTreeSet<usize>,
    /// What each relation reads (and each produced relation's request): the graph the components are of.
    edges: Edges,
    fields: Option<Arc<FieldsData>>,
    made: Vec<Made>,
    /// The (head, negated relation) pairs whose cycle the profile was given.
    shown: BTreeSet<(String, String)>,
}

impl GateRules<'_> {
    /// Why `rel` is in component `c`, which loads: its reads to the component's request and the request's back to
    /// it, then each step with a rule that makes it.
    fn cycle(&self, v: &Version, rel: &str, c: usize) -> String {
        let name = |r: super::program_image::RelId| v.rel_name(r);
        let request = self.answers.keys().find(|q| self.comp.get(q.as_str()) == Some(&c)).map_or("?", String::as_str);
        let (out_path, back_path) = (path(&self.edges, rel, request), path(&self.edges, request, rel));
        let mut steps = Vec::new();
        for p in [&out_path, &back_path] {
            for w in p.windows(2) {
                let (x, y) = (w[0].trim_start_matches('!'), w[1].trim_start_matches('!'));
                let rule = v.rules().find(|rr| name(rr.head().rel()) == x && rr.body().any(|l| l.atom().is_some_and(|a| name(a.rel()) == y)));
                steps.push(format!("\n  {x} -> {y}: {}", rule.map_or_else(|| format!("{x} is {y}'s answer"), |rr| v.show_rule(rr))));
            }
        }
        format!("the component of {} relations: {} ... {}{}", self.sccs[c].len(), out_path.join(" -> "), back_path.join(" -> "), steps.concat())
    }
}

impl RulePass for GateRules<'_> {
    const NAME: &'static str = "gated negations";
    fn begin(&mut self, cx: &Cx<'_>) -> Result<Select, String> {
        let v: &Version = cx.v();
        let name = |r: super::program_image::RelId| v.rel_name(r);
        self.produced = self.answers.iter().flat_map(|(req, ps)| ps.iter().map(move |q| (q.clone(), req.clone()))).collect();
        let mut nodes: BTreeSet<String> = v.rules().map(|r| name(r.head().rel()).to_owned()).collect();
        nodes.extend(self.produced.keys().cloned());
        let mut edges: Edges = BTreeMap::new();
        for r in v.rules() {
            for l in r.body() {
                if let Some(a) = l.atom().filter(|a| nodes.contains(name(a.rel()))) {
                    let negative = (a.neg() || matches!(l.view(), Lit::Agg { .. })) && !a.gated();
                    edges.entry(name(r.head().rel()).to_owned()).or_default().insert((name(a.rel()).to_owned(), negative));
                }
            }
        }
        for (q, req) in &self.produced {
            edges.entry(q.clone()).or_default().insert((req.clone(), false));
        }
        self.sccs = tarjan(&nodes, &edges);
        self.edges = edges.clone();
        self.comp = self.sccs.iter().enumerate().flat_map(|(i, s)| s.iter().map(move |r| (r.clone(), i))).collect();
        let (comp, sccs, produced) = (&self.comp, &self.sccs, &self.produced);
        self.answered = self.answers.keys().filter_map(|r| comp.get(r.as_str()).copied()).filter(|&c| sccs[c].iter().any(|r| produced.contains_key(r.as_str()))).collect();
        if self.answered.is_empty() {
            return Ok(Select::Rules(Default::default()));
        }
        self.fields = Some(cx.get::<FieldsData>()?);
        Ok(Select::All)
    }
    fn rule(&mut self, cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> Result<Step, String> {
        let v: &Version = cx.v();
        let f = Fields::over(v, self.fields.as_ref().expect("begun"));
        let name = |r: super::program_image::RelId| v.rel_name(r);
        let Some(&c) = self.comp.get(name(r.head().rel())).filter(|c| self.answered.contains(c)) else { return Ok(Step::Keep) };
        let comp = &self.comp;
        let lits: Vec<LitRef<'_>> = r.body().collect();
        let in_comp = |l: &LitRef<'_>| l.atom().is_some_and(|a| comp.get(name(a.rel())) == Some(&c));
        let mut outs = Vec::new();
        // The variables the literals before each one bind: its positive atoms' (an aggregate's own are its alone).
        let mut bound: BTreeSet<super::program_image::Slot> = BTreeSet::new();
        for (i, l) in lits.iter().enumerate() {
            let inside = in_comp(l);
            let agg = matches!(l.view(), Lit::Agg { .. });
            let at_bound = bound.clone();
            if let Lit::Atom(a) = l.view()
                && !a.neg()
            {
                bound.extend(a.args().filter_map(|t| match t {
                    TermRef::Var(s) => Some(s),
                    _ => None,
                }));
            }
            let Some(a) = l.atom().filter(|a| inside && (a.neg() || agg) && !a.gated()) else {
                outs.push(Out::Copy(i));
                continue;
            };
            let rel = name(a.rel());
            // Why it is gated: the cycle through the request that puts its relation in the component.
            if self.shown.insert((name(r.head().rel()).to_owned(), rel.to_owned())) {
                crate::code_rule_dsl::logic::profile::decision("gated negations (decision 0065 §2a)", || format!("{} {} {rel} inside a component that loads: {}", name(r.head().rel()), if agg { "aggregates" } else { "negates" }, self.cycle(v, rel, c)));
            }
            // The key: a negated atom's columns that are not wild, all bound by the literals before it; an
            // aggregate's that they bind (a constant's too), its group.
            let at: Vector<usize> = a
                .args()
                .enumerate()
                .filter(|(_, t)| match t {
                    TermRef::Wild => false,
                    TermRef::Var(s) if agg => at_bound.contains(s),
                    _ => true,
                })
                .map(|(k, _)| k)
                .collect();
            let ad: String = (0..a.arity()).map(|k| if at.contains(&k) { 'b' } else { 'f' }).collect();
            let cols: Vec<(String, String)> = at.iter().map(|&k| (format!("c{k}"), f.column_type(rel, k).unwrap_or("int").to_owned())).collect();
            // Keyed when the literals before the negation read the component, so its keys grow with it; otherwise the
            // keys are fixed before it runs, and the whole relation's gate holds them all (as demand's does).
            let asks_inside = lits[..i].iter().any(&in_comp);
            if at.is_empty() || !asks_inside {
                let done = format!("done_{rel}");
                if !self.made.iter().any(|(d, ..)| *d == done) {
                    self.made.push((done.clone(), None, rel.to_owned(), vec![("k".to_owned(), "int".to_owned())]));
                }
                outs.push(Out::DoneAll(done));
            } else {
                let (ask, done) = (format!("ask_{rel}_{ad}"), format!("done_{rel}_{ad}"));
                // The keys asked about: the rule's literals before the negation, as this rewrite writes them (an
                // earlier negation or aggregate of the component gated, not read raw).
                let args: Vec<TermRef<'_>> = a.args().collect();
                let key: Vec<T<'_>> = at.iter().map(|&k| t_of(v, r, args[k])).collect();
                // A key variable the literals before bind none of, a later positive one binds (the rule is safe): the
                // ask reads the later positive atoms and comparisons too, never a negation or an aggregate. Each holds
                // wherever the rule does, so the keys asked are the rule's own.
                let unbound = at.iter().any(|&k| matches!(args[k], TermRef::Var(s) if !at_bound.contains(&s)));
                let late: Vec<Out> = if unbound {
                    (i + 1..lits.len())
                        .filter(|&j| match lits[j].view() {
                            Lit::Atom(b) => !b.neg(),
                            Lit::Cmp { .. } | Lit::Nearest { .. } => true,
                            _ => false,
                        })
                        .map(Out::Copy)
                        .collect()
                } else {
                    Vec::new()
                };
                let mut rb = out.rule().head(ask.as_str(), &key);
                for o in outs.iter().chain(&late) {
                    rb = write(v, r, &lits, rb, o);
                }
                rb.finish()?;
                if !self.made.iter().any(|(d, ..)| *d == done) {
                    self.made.push((done.clone(), Some(ask), rel.to_owned(), cols));
                }
                outs.push(Out::Done(done, at, i));
            }
            outs.push(Out::Gated(i));
        }
        if outs.iter().all(|o| matches!(o, Out::Copy(_))) {
            return Ok(Step::Keep);
        }
        let mut rb = out.rule().head_from(v, r, r.head());
        for o in &outs {
            rb = write(v, r, &lits, rb, o);
        }
        rb.finish()?;
        Ok(Step::Written)
    }
    /// The gates' relations, and each `done` fed from its `ask` once what its relation reads inside the component is
    /// quiet. Placement only: the rule puts `done` in the component (`native_stratum` drops a gate's rule).
    fn finish(&mut self, cx: &Cx<'_>, b: &mut Builder<'_>) -> Result<(), String> {
        let Some(d) = &self.fields else { return Ok(()) };
        let f = Fields::over(cx.v(), d);
        for (done, ask, rel, cols) in &self.made {
            let decl: Vec<(&str, &str)> = cols.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
            b.fields().declare(done, &decl);
            match ask {
                Some(ask) => {
                    b.fields().declare(ask, &decl);
                    let vs: Vec<String> = (0..cols.len()).map(|i| format!("_g{i}")).collect();
                    let vs: Vec<T<'_>> = vs.iter().map(|x| T::Var(x.as_str())).collect();
                    b.rule().head(done.as_str(), &vs).atom(ask.as_str(), &vs).finish()?;
                }
                None => {
                    let wild = vec![T::Wild; f.arity(rel).unwrap_or(0)];
                    b.rule().head(done.as_str(), &[T::Int(0)]).atom(rel.as_str(), &wild).finish()?;
                }
            }
        }
        Ok(())
    }
}

/// The gates of `GateRules`' rewrite, and the answers, over it (`gate`): each gate quiet once what its relation reads
/// inside its component, with the answers' edges but not through a gate, is; a gate whose relation reads another's
/// after it. An overlay, never folded.
struct GateFields<'r, 'a> {
    rules: &'r GateRules<'a>,
}

impl ProgramPass for GateFields<'_, '_> {
    const NAME: &'static str = "gates";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let g = self.rules;
        let w_: &Version = cx.v();
        let names: BTreeSet<String> = g.made.iter().map(|(d, ..)| d.clone()).collect();
        let mut reads: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for r in w_.rules() {
            for a in r.reads().filter(|a| !(a.gated() || names.contains(w_.rel_name(a.rel())))) {
                reads.entry(w_.rel_name(r.head().rel()).to_owned()).or_default().insert(w_.rel_name(a.rel()).to_owned());
            }
        }
        for (q, req) in &g.produced {
            reads.entry(q.clone()).or_default().insert(req.clone());
        }
        let mut gates = Vec::new();
        for (done, ask, rel, _) in &g.made {
            let c = g.comp[rel.as_str()];
            let (mut reach, mut todo) = (BTreeSet::new(), vec![rel.clone()]);
            while let Some(x) = todo.pop() {
                if (g.comp.get(x.as_str()) == Some(&c) || x.starts_with("ask_")) && reach.insert(x.clone()) {
                    todo.extend(reads.get(&x).into_iter().flatten().cloned());
                }
            }
            let deps: Vec<String> = w_
                .rules()
                .filter(|r| reach.contains(w_.rel_name(r.head().rel())))
                .flat_map(|r| r.reads().map(|a| w_.rel_name(a.rel()).to_owned()).collect::<Vec<_>>())
                .filter(|x| names.contains(x.as_str()) && x != done)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            gates.push((done.clone(), Gate { src: ask.clone(), rel: rel.clone(), level: 0, quiet: reach.into_iter().collect(), deps }));
        }
        // Levels: a gate whose relation reads another's comes after it.
        let level_edges: Edges = gates.iter().map(|(d, g)| (d.clone(), g.deps.iter().map(|x| (x.clone(), false)).collect())).collect();
        let order = tarjan(&names, &level_edges);
        for (d, gt) in &mut gates {
            gt.level = order.iter().position(|s| s.contains(d.as_str())).unwrap_or(0);
        }
        cx.decide("gated", || format!("{} gates in {} answered components", gates.len(), g.answered.len()));
        let b = w.overlay()?;
        b.fields().add_tree_gates(&gates);
        b.fields().set_answers(g.answers);
        Ok(())
    }
}

/// A program split at its requests (`split`): `cone`, the relations every request depends on (the load part's
/// rules, with its gates and answers), and the rest, which reads what it needs of the cone as inputs (`reads`). The
/// cone runs first, pausing for its answers; the rest runs once over every document it loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loading {
    pub(in crate::code_rule_dsl) cone: BTreeSet<String>,
    pub reads: BTreeSet<String>,
    pub load_inputs: Vec<String>,
    pub(in crate::code_rule_dsl) load_outputs: Vec<String>,
    /// The rest's inputs: the program's, and what it reads of the cone; a caller adds what else it binds.
    pub rest_inputs: Vec<String>,
    pub(in crate::code_rule_dsl) rest_outputs: Vec<String>,
}

/// The relations `requests` depend on, among those `p`'s rules derive (a request no rule derives is none): the
/// cone of requests (`split`).
pub fn cone<'r>(p: &Placed, requests: impl IntoIterator<Item = &'r str>) -> BTreeSet<String> {
    let v: &Version = &p.v;
    let rules: Vec<RuleRef<'_>> = p.rules().collect();
    let derived: BTreeSet<&str> = rules.iter().map(|r| v.rel_name(r.head().rel())).collect();
    let mut cone: BTreeSet<String> = requests.into_iter().filter(|r| derived.contains(r)).map(str::to_owned).collect();
    let mut work: Vec<String> = cone.iter().cloned().collect();
    while let Some(rel) = work.pop() {
        for r in rules.iter().filter(|r| v.rel_name(r.head().rel()) == rel) {
            for a in r.reads() {
                let read = v.rel_name(a.rel());
                if derived.contains(read) && cone.insert(read.to_owned()) {
                    work.push(read.to_owned());
                }
            }
        }
    }
    cone
}

/// [`Loading`] of `p` (a whole program's version, its answers stated), or `None` for a program without answers:
/// the version's (`Split`, decision 0281 §2).
pub fn split(p: &Placed) -> Option<Loading> {
    if !p.whole {
        return None;
    }
    super::program_image::manager::analysis::<Split>(&p.v).ok()?.0.clone()
}

/// The split at the requests of a whole version (`split`), as an analysis: what 0225 §5's split rules compute.
#[derive(Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Split(pub(in crate::code_rule_dsl) Option<Loading>);

impl super::program_image::manager::Analysis for Split {
    const NAME: &'static str = "split";
    const READS: super::program_image::manager::Reads = {
        use super::program_image::manager::Reads;
        Reads::RULES.or(Reads::ABOUT).or(Reads::INPUTS).or(Reads::OUTPUTS)
    };
    fn compute(cx: &Cx<'_>) -> Result<Split, String> {
        Ok(Split(split_of(&Placed::whole(cx.v().clone()))))
    }
}

/// [`Loading`] of `p`, computed.
fn split_of(p: &Placed) -> Option<Loading> {
    let f = Fields::of(p)?;
    let v: &Version = &p.v;
    if f.answers().is_empty() {
        return None;
    }
    let rules: Vec<RuleRef<'_>> = p.rules().collect();
    let cone = cone(p, f.answers().iter().map(|&(q, _)| v.rel_name(q)));
    let outputs: Vec<String> = f.outputs().map(str::to_owned).collect();
    let reads: BTreeSet<String> = rules
        .iter()
        .filter(|r| !cone.contains(v.rel_name(r.head().rel())))
        .flat_map(|r| r.reads())
        .map(|a| v.rel_name(a.rel()))
        .filter(|a| cone.contains(*a))
        .map(str::to_owned)
        // An output made in the cone is handed on too, and written by the rest as its other outputs are.
        .chain(outputs.iter().filter(|o| cone.contains(*o)).cloned())
        .collect();
    let inputs: Vec<String> = f.inputs().map(str::to_owned).collect();
    let mut load_outputs: Vec<String> = outputs.iter().filter(|o| cone.contains(*o) || reads.contains(*o)).cloned().collect();
    let mut rest_inputs = inputs.clone();
    for r in &reads {
        if !load_outputs.contains(r) {
            load_outputs.push(r.clone());
        }
        if !rest_inputs.contains(r) {
            rest_inputs.push(r.clone());
        }
    }
    let rest_outputs = outputs.iter().filter(|o| !cone.contains(*o) || reads.contains(*o)).cloned().collect();
    Some(Loading { cone, reads, load_inputs: inputs, load_outputs, rest_inputs, rest_outputs })
}

impl Loading {
    /// The load part and the rest, of `v` (the version split): each a view of the rules its side of the cone derives,
    /// with its own inputs and outputs, and its side's lattices and gates; both keep every native, and the rest no
    /// answers.
    pub fn parts(&self, v: &Arc<Version>) -> Result<(Arc<Version>, Arc<Version>), String> {
        let cone = &self.cone;
        let (inside, outside, all) = (|rel: &str| cone.contains(rel), |rel: &str| !cone.contains(rel), |_: &str| true);
        let load = v.part(inside, &self.load_inputs, &self.load_outputs, PartFields { natives: &all, lattice: &inside, gates: &inside })?;
        let none = BTreeMap::new();
        let mut rest = Pipeline::new("rest part", v.clone());
        rest.program(&mut Retain(outside))
            .program(&mut PartOf { inputs: &self.rest_inputs, outputs: &self.rest_outputs, fields: PartFields { natives: &all, lattice: &outside, gates: &outside } })
            .program(&mut SetAnswers(&none));
        Ok((load, rest.finish()?.0))
    }
}

// cratify: `tests` is rerail-logic's (it mounts it: it reads the language, `rr::parse_version`, or the corpus)
