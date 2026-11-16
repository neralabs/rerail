//! The planner's passes over a version (decision 0173, stage 2 step 3), each stated as an edit: `normalize` is a
//! derivation (every rule rewritten, each recording its source, §9), `stated_apart` an overlay of what it moves. Each is
//! the tree pass it replaces (`plan::sql_kernel::normalize`, `plan::stated_apart`), checked against it (`tests`).
use super::about::About;
use super::build::{Builder, Rewrite, Site, T};
use super::manager::{Cx, Pipeline, ProgramPass, ProgramW, RuleOut, RulePass, Select, Step};
use super::handles::{AtomRef, Lit, RuleRef, TermRef};
use super::version::Version;
use super::{Cell, RelId, Slot};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// `v` normalized for the SQL kernel (`sql_kernel::normalize`): each relation of no columns, or a lattice of no
/// keys, gets a leading presence column (`program::physical`); and when any lattice is not flat, each `top` becomes its
/// lattice's value and each ground rule a fact (`program::tops`). A derivation of `v`: every rule came from one of
/// `v`'s. Also the sentinel relations, those and every declared one of no columns, by id. A rule pass (`Normalize`).
pub fn normalize(v: &Arc<Version>) -> Result<(Arc<Version>, Vec<RelId>), String> {
    let mut pass = Normalize::default();
    let mut p = Pipeline::new("normalize", v.clone());
    p.rules(&mut pass);
    let (out, _) = p.finish()?;
    Ok((out, pass.sentinels))
}

/// `normalize` as a rule pass (decision 0281): what it reads of the whole program is found once (`begin`), each rule
/// is rewritten alone (a ground rule's head is a fact once `top` is its value), and the declarations, lattice keys
/// and facts it changes are written once (`finish`).
#[derive(Default)]
pub struct Normalize {
    /// Whether each relation (by id) gets a presence column.
    sentinel: Vec<bool>,
    /// Each lattice's value column and top, by relation, when any lattice is not flat.
    top_of: Vec<Option<(usize, i64)>>,
    tops: bool,
    /// The facts it states: each sentinel's, with its presence cell, and each ground rule's head.
    facts: BTreeMap<RelId, Vec<Vec<Cell>>>,
    /// The relations given a presence column, and every declared one of no columns, by id.
    pub sentinels: Vec<RelId>,
}

/// The rows of `r`'s stated facts in `v`, a sentinel's with its presence cell.
fn rows_of(v: &Version, sentinel: &[bool], r: RelId) -> Vec<Vec<Cell>> {
    let rows = v.facts(v.rel_atom(r)).map(|f| f.rows().map(|row| row.collect::<Vec<Cell>>()).collect()).unwrap_or_else(Vec::new);
    if sentinel[r.index()] { rows.into_iter().map(|row: Vec<Cell>| std::iter::once(Cell::Int(1)).chain(row).collect()).collect() } else { rows }
}

impl RulePass for Normalize {
    const NAME: &'static str = "normalize";
    fn begin(&mut self, cx: &Cx<'_>) -> Result<Select, String> {
        let v = cx.v();
        let about = cx.get::<About>()?;
        let n = v.rel_end() as usize;
        self.sentinel = vec![false; n];
        let mut declared = vec![false; n];
        for d in v.decls() {
            let r = v.rel_of(d.name()).expect("a declared relation has an id");
            declared[r.index()] = true;
            if d.arity() == 0 || about.lattice_decl(r).is_some_and(|l| l.keys == 0) {
                self.sentinel[r.index()] = true;
            }
        }
        let mut sentinels: Vec<RelId> = (0..n as u32).map(RelId).filter(|r| self.sentinel[r.index()]).collect();
        sentinels.extend(about.nullary.iter().filter(|r| declared[r.index()]));
        sentinels.sort_unstable();
        sentinels.dedup();
        self.sentinels = sentinels;
        // Each lattice's value column (in `v`'s atoms) and its top, when any lattice is not flat.
        self.tops = about.lattice.iter().any(|l| v.text(l.lattice) != "flat");
        self.top_of = vec![None; n];
        if self.tops {
            for l in &about.lattice {
                let lat = crate::code_rule_dsl::logic::native::lattice(v.text(l.lattice)).map_err(|e| e.to_string())?;
                self.top_of[l.rel.index()] = Some((l.keys, lat.top()));
            }
        }
        // Facts: a sentinel's rows get the presence cell; a ground rule's head is one more row (`rule`).
        for f in v.all_facts() {
            let r = v.rel_of(f.name()).expect("a fact relation has an id");
            if self.sentinel[r.index()] {
                self.facts.insert(r, rows_of(v, &self.sentinel, r));
            }
        }
        Ok(Select::All)
    }
    fn rule(&mut self, cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> Result<Step, String> {
        let mut rw = Normal::of(r, &self.sentinel, &self.top_of, self.tops)?;
        if self.tops
            && r.body().next().is_none()
            && let Some(row) = rw.ground_head(r.head())
        {
            let (v, sentinel) = (cx.v(), &self.sentinel);
            self.facts.entry(r.head().rel()).or_insert_with(|| rows_of(v, sentinel, r.head().rel())).push(row);
            return Ok(Step::Drop);
        }
        out.rewrite(&mut rw)
    }
    fn finish(&mut self, cx: &Cx<'_>, b: &mut Builder<'_>) -> Result<(), String> {
        let v = cx.v();
        let about = cx.get::<About>()?;
        let mut empty: Vec<&str> = Vec::new();
        for d in v.decls() {
            if d.arity() == 0 {
                empty.push(v.text(d.name()));
            }
            if self.sentinel[v.rel_of(d.name()).expect("declared").index()] {
                let cols: Vec<(&str, &str)> = std::iter::once(("__presence", "int")).chain(d.cols().map(|(c, t)| (v.text(c), v.text(t)))).collect();
                b.declare(v.text(d.name()), &cols);
            }
        }
        for l in &about.lattice {
            if self.sentinel[l.rel.index()] {
                b.set_lattice_keys(v.rel_name(l.rel), l.keys + 1);
            }
        }
        b.add_nullary(&empty);
        for (r, rows) in std::mem::take(&mut self.facts) {
            b.set_fact_cells(v.rel_name(r), rows);
        }
        Ok(())
    }
}

/// `normalize`'s rewrite of one rule: the presence column, and each `top` by the domain it is in.
struct Normal<'a> {
    sentinel: &'a [bool],
    top_of: &'a [Option<(usize, i64)>],
    tops: bool,
    /// The head's lattice's top, or the flat lattice's.
    default: i64,
    /// Each comparison's top, by its node.
    cmp: HashMap<u32, i64>,
}

fn vars(t: TermRef<'_>, out: &mut Vec<Slot>) {
    match t {
        TermRef::Var(s) => out.push(s),
        TermRef::Call(_, args) => args.for_each(|a| vars(a, out)),
        _ => {}
    }
}

impl<'a> Normal<'a> {
    fn of(r: RuleRef<'_>, sentinel: &'a [bool], top_of: &'a [Option<(usize, i64)>], tops: bool) -> Result<Normal<'a>, String> {
        let default = top_of[r.head().rel().index()].map_or(crate::code_rule_dsl::logic::native::TOP, |(_, t)| t);
        let mut cmp = HashMap::new();
        if tops {
            // The tops each variable is known to hold: a lattice's value column, then through `=`, to a fixpoint.
            let mut known: HashMap<Slot, BTreeSet<i64>> = HashMap::new();
            for a in r.reads().chain([r.head()]) {
                if let Some((col, top)) = top_of[a.rel().index()]
                    && let Some(TermRef::Var(s)) = a.args().nth(col)
                {
                    known.entry(s).or_default().insert(top);
                }
            }
            let mut scratch = Vec::new();
            loop {
                let before: usize = known.values().map(BTreeSet::len).sum();
                for l in r.body() {
                    if let Lit::Cmp { l, op: super::CmpOp::Eq, r: rt } = l.view() {
                        for (target, source) in [(l, rt), (rt, l)] {
                            if let TermRef::Var(target) = target {
                                scratch.clear();
                                vars(source, &mut scratch);
                                let values: BTreeSet<i64> = scratch.iter().filter_map(|s| known.get(s)).flatten().copied().collect();
                                if !values.is_empty() {
                                    known.entry(target).or_default().extend(values);
                                }
                            }
                        }
                    }
                }
                if before == known.values().map(BTreeSet::len).sum::<usize>() {
                    break;
                }
            }
            for lit in r.body() {
                if let Lit::Cmp { l, r: rt, .. } = lit.view() {
                    scratch.clear();
                    vars(l, &mut scratch);
                    vars(rt, &mut scratch);
                    let inferred: BTreeSet<i64> = scratch.iter().filter_map(|s| known.get(s)).flatten().copied().collect();
                    if inferred.len() > 1 && (matches!(l, TermRef::Top) || matches!(rt, TermRef::Top)) {
                        return Err("top is ambiguous between different lattice domains".into());
                    }
                    cmp.insert(lit.node(), inferred.first().copied().unwrap_or(default));
                }
            }
        }
        Ok(Normal { sentinel, top_of, tops, default, cmp })
    }
    fn atom_top(&self, a: AtomRef<'_>) -> i64 {
        self.top_of[a.rel().index()].map_or(self.default, |(_, t)| t)
    }
    /// The head as a fact's row, if every argument is a constant once `top` is its value.
    fn ground_head(&self, head: AtomRef<'_>) -> Option<Vec<Cell>> {
        let top = self.atom_top(head);
        let prefix = self.sentinel[head.rel().index()].then_some(Cell::Int(1));
        prefix.into_iter().map(Some).chain(head.args().map(|t| match t {
            TermRef::Int(i) => Some(Cell::Int(i)),
            TermRef::Branded(b) => Some(Cell::Branded(b)),
            TermRef::Str(a) => Some(Cell::Str(a)),
            TermRef::Top => Some(Cell::Int(top)),
            _ => None,
        })).collect()
    }
}

impl Rewrite for Normal<'_> {
    fn prefix(&mut self, a: AtomRef<'_>, out: &mut Vec<T<'static>>) {
        if self.sentinel[a.rel().index()] {
            out.push(T::Int(1));
        }
    }
    fn term(&mut self, site: Site<'_>, t: TermRef<'_>) -> Option<T<'static>> {
        if !self.tops || !matches!(t, TermRef::Top) {
            return None;
        }
        Some(T::Int(match site {
            Site::Head(a) | Site::Atom(a) => self.atom_top(a),
            Site::Cmp(l) => self.cmp[&l.node()],
        }))
    }
}

/// Each relation of `v` with rules and stated facts, its facts moved apart into `rel__stated`, read by the rule
/// `rel(x…) :- rel__stated(x…)`; a lattice relation keeps its facts (`plan::stated_apart`). An overlay of `v`; and the
/// pairs `(rel, rel__stated)`, in name order. A program pass (`StatedApart`).
pub fn stated_apart(v: &Arc<Version>) -> Result<(Arc<Version>, Vec<(RelId, RelId)>), String> {
    let mut pass = StatedApart::default();
    let mut p = Pipeline::new("stated apart", v.clone());
    p.program(&mut pass);
    let (out, _) = p.finish()?;
    Ok((out, pass.pairs))
}

/// `stated_apart` as a program pass (decision 0281): an edit of the version, its moved facts' relations' fields and
/// one rule each.
#[derive(Default)]
pub struct StatedApart {
    /// The pairs `(rel, rel__stated)`, in name order.
    pub pairs: Vec<(RelId, RelId)>,
}

impl ProgramPass for StatedApart {
    const NAME: &'static str = "stated apart";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let v = cx.v();
        let about = cx.get::<About>()?;
        let mut moved: Vec<RelId> = v.all_facts().into_iter().filter(|f| f.rows().len() > 0).filter_map(|f| v.rel_of(f.name())).filter(|&r| v.heads(r).next().is_some() && about.lattice_decl(r).is_none()).collect();
        moved.sort_unstable_by_key(|&r| v.rel_name(r));
        if moved.is_empty() {
            return Ok(());
        }
        let b = w.on()?;
        for rel in moved {
            let name = v.rel_name(rel);
            let apart = format!("{name}__stated");
            let rows: Vec<Vec<Cell>> = v.facts(v.rel_atom(rel)).expect("stated facts").rows().map(|r| r.collect()).collect();
            let d = v.decl(v.rel_atom(rel)).ok_or_else(|| format!("{name} has stated facts and no declaration"))?;
            let cols: Vec<(&str, &str)> = d.cols().map(|(c, t)| (v.text(c), v.text(t))).collect();
            b.drop_facts(name);
            b.declare(&apart, &cols);
            b.set_fact_cells(&apart, rows);
            let args: Vec<String> = (0..cols.len()).map(|i| format!("rr_s{i}")).collect();
            let args: Vec<T<'_>> = args.iter().map(|a| T::Var(a)).collect();
            let to = b.rel(&apart);
            b.rule().head(rel, &args).atom(to, &args).finish()?;
            self.pairs.push((rel, to));
        }
        cx.decide("moved", || format!("{} relations' stated facts apart", self.pairs.len()));
        Ok(())
    }
}

/// The rules whose head's name the predicate takes, the rest removed: a view, or the version itself where every rule
/// is taken.
pub struct Retain<F>(pub F);

impl<F: Fn(&str) -> bool> ProgramPass for Retain<F> {
    const NAME: &'static str = "retain";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let v = cx.v();
        if v.rules().all(|r| (self.0)(v.rel_name(r.head().rel()))) {
            return Ok(());
        }
        w.keep(|r| (self.0)(v.rel_name(r.head().rel())))
    }
}

/// A part's own fields (decision 0244, `Version::part`): its inputs and outputs, and the natives, lattices and gates
/// of the relations `fields` takes. An overlay, never folded, so the part costs its range list and its fields.
pub struct PartOf<'f, S> {
    pub inputs: &'f [S],
    pub outputs: &'f [S],
    pub fields: super::build::PartFields<'f>,
}

impl<S: AsRef<str>> ProgramPass for PartOf<'_, S> {
    const NAME: &'static str = "part's fields";
    fn run(&mut self, _: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        let b = w.overlay()?;
        b.set_inputs(self.inputs);
        b.set_outputs(self.outputs);
        b.keep_relation_fields(self.fields.natives, self.fields.lattice, self.fields.gates);
        Ok(())
    }
}

/// The program's answers stated as `.0` gives them (`Program::answers`): an overlay, never folded.
pub struct SetAnswers<'a>(pub &'a BTreeMap<String, Vec<String>>);

impl ProgramPass for SetAnswers<'_> {
    const NAME: &'static str = "answers";
    fn run(&mut self, _: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        w.overlay()?.set_answers(self.0);
        Ok(())
    }
}

/// Each relation's qualifiers stated as `.0` gives them, (relation, word) pairs: an overlay, never folded.
pub struct SetQualifiers<'a>(pub &'a [(&'a str, &'a str)]);

impl ProgramPass for SetQualifiers<'_> {
    const NAME: &'static str = "qualifiers";
    fn run(&mut self, _: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<(), String> {
        w.overlay()?.set_qualifiers(self.0);
        Ok(())
    }
}
