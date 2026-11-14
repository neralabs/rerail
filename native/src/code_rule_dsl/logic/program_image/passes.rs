//! The planner's passes over a version (decision 0173, stage 2 step 3), each stated as an edit: `normalize` is a
//! derivation (every rule rewritten, each recording its source, §9), `stated_apart` an overlay of what it moves. Each is
//! the tree pass it replaces (`plan::sql_kernel::normalize`, `plan::stated_apart`), checked against it (`tests`).
use super::about::About;
use super::build::{Builder, Rewrite, Site, T};
use super::handles::{AtomRef, Lit, RuleRef, TermRef};
use super::version::Version;
use super::{Cell, RelId, Slot};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// `v` normalized for the SQL kernel (`sql_kernel::normalize`): each relation of no columns, or a lattice of no
/// keys, gets a leading presence column (`program::physical`); and when any lattice is not flat, each `top` becomes its
/// lattice's value and each ground rule a fact (`program::tops`). A derivation of `v`: every rule came from one of
/// `v`'s. Also the sentinel relations, those and every declared one of no columns, by id.
pub fn normalize(v: &Version) -> Result<(Arc<Version>, Vec<RelId>), String> {
    let about = About::of(v);
    let n = v.rel_end() as usize;
    let mut sentinel = vec![false; n];
    let mut empty: Vec<&str> = Vec::new();
    let mut declared = vec![false; n];
    for d in v.decls() {
        let r = v.rel_of(d.name()).expect("a declared relation has an id");
        declared[r.index()] = true;
        if d.arity() == 0 {
            empty.push(v.text(d.name()));
        }
        if d.arity() == 0 || about.lattice_decl(r).is_some_and(|l| l.keys == 0) {
            sentinel[r.index()] = true;
        }
    }
    let mut sentinels: Vec<RelId> = (0..n as u32).map(RelId).filter(|r| sentinel[r.index()]).collect();
    sentinels.extend(about.nullary.iter().filter(|r| declared[r.index()]));
    sentinels.sort_unstable();
    sentinels.dedup();
    // Each lattice's value column (in `v`'s atoms) and its top, when any lattice is not flat.
    let tops = about.lattice.iter().any(|l| v.text(l.lattice) != "flat");
    let mut top_of: Vec<Option<(usize, i64)>> = vec![None; n];
    if tops {
        for l in &about.lattice {
            let lat = crate::code_rule_dsl::logic::native::lattice(v.text(l.lattice)).map_err(|e| e.to_string())?;
            top_of[l.rel.index()] = Some((l.keys, lat.top()));
        }
    }
    let mut b = Builder::derive(v);
    for d in v.decls() {
        if sentinel[v.rel_of(d.name()).expect("declared").index()] {
            let cols: Vec<(&str, &str)> = std::iter::once(("__presence", "int")).chain(d.cols().map(|(c, t)| (v.text(c), v.text(t)))).collect();
            b.declare(v.text(d.name()), &cols);
        }
    }
    for l in &about.lattice {
        if sentinel[l.rel.index()] {
            b.set_lattice_keys(v.rel_name(l.rel), l.keys + 1);
        }
    }
    b.add_nullary(&empty);
    // Facts: a sentinel's rows get the presence cell; a ground rule's head is one more row.
    let rows_of = |r: RelId| -> Vec<Vec<Cell>> {
        let rows = v.facts(v.rel_atom(r)).map(|f| f.rows().map(|row| row.collect::<Vec<Cell>>()).collect()).unwrap_or_else(Vec::new);
        if sentinel[r.index()] { rows.into_iter().map(|row: Vec<Cell>| std::iter::once(Cell::Int(1)).chain(row).collect()).collect() } else { rows }
    };
    let mut facts: BTreeMap<RelId, Vec<Vec<Cell>>> = BTreeMap::new();
    for f in v.all_facts() {
        let r = v.rel_of(f.name()).expect("a fact relation has an id");
        if sentinel[r.index()] {
            facts.insert(r, rows_of(r));
        }
    }
    for r in v.rules() {
        let mut rw = Normal::of(r, &sentinel, &top_of, tops)?;
        if tops
            && r.body().next().is_none()
            && let Some(row) = rw.ground_head(r.head())
        {
            facts.entry(r.head().rel()).or_insert_with(|| rows_of(r.head().rel())).push(row);
            continue;
        }
        b.rewrite(v, r, &mut rw).finish()?;
    }
    for (r, rows) in facts {
        b.set_fact_cells(v.rel_name(r), rows);
    }
    let (out, _) = b.finish()?;
    Ok((out, sentinels))
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
/// pairs `(rel, rel__stated)`, in name order.
pub fn stated_apart(v: &Arc<Version>) -> Result<(Arc<Version>, Vec<(RelId, RelId)>), String> {
    let about = About::of(v);
    let mut moved: Vec<RelId> = v.all_facts().into_iter().filter(|f| f.rows().len() > 0).filter_map(|f| v.rel_of(f.name())).filter(|&r| v.heads(r).next().is_some() && about.lattice_decl(r).is_none()).collect();
    moved.sort_unstable_by_key(|&r| v.rel_name(r));
    if moved.is_empty() {
        return Ok((v.clone(), Vec::new()));
    }
    let mut b = Builder::on(v);
    let mut out = Vec::new();
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
        out.push((rel, to));
    }
    let (w, _) = b.finish()?;
    Ok((w, out))
}
