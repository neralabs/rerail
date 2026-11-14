//! The inlining `choose` decided, as an overlay of the run's version (t36 item 6, decision 0173 §7): each read of an
//! inlined relation unfolded into its reader's copies (the module's "Unfolding"), the inlined relations' rules removed
//! where no reader reads them still. A pass is an overlay of the one before (a rule read by another pass's copies is
//! one of that pass's), so the result is the base, less the readers and the inlined rules, plus the copies: what it
//! costs is the change.

use super::COPIES;
use crate::code_rule_dsl::logic::program_image::{AtomId, AtomRef, Builder, CmpOp, Lit, LitRef, RelId, Rewrite, RuleRef, Site, T, TermRef, Version};
use crate::code_rule_dsl::logic::program_image::version::Made;
use crate::code_rule_dsl::logic::program_image::{RuleId, Slot};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// The inlined version, and each of its rules' origin: the base rule it was unfolded from, and the relations unfolded
/// into it, for every rule of the result (`unfold_reads`'s `origin`), in the tree pass's order.
pub(super) struct Inlined {
    pub v: Arc<Version>,
    /// The result's rules in the tree pass's order: each rule's id in `v`, its base rule and the relations unfolded
    /// into it.
    pub rules: Vec<(RuleId, RuleId, BTreeSet<String>)>,
    /// The inlined relations some reader still reads (a chain deeper than the passes): their rules are kept.
    pub unfinished: BTreeSet<String>,
}

/// `base` with the reads of the `inline` relations unfolded into their readers, as `unfold_reads` unfolds them (at
/// most eight passes, a rule's first such read a pass), and the inlined relations' rules removed but where a reader
/// still reads one.
pub(super) fn inline(base: &Arc<Version>, inline: &BTreeSet<String>) -> Result<Inlined, String> {
    let inline_ids: BTreeSet<RelId> = inline.iter().filter_map(|r| base.rel(r)).collect();
    // The program's rules in the tree pass's order (copies where their reader was), with their origins.
    let mut seq: Vec<(RuleId, RuleId, BTreeSet<String>)> = base.rules().map(|r| (r.id(), r.id(), BTreeSet::new())).collect();
    let mut cur = Arc::clone(base);
    let first_read = |v: &Version, id: RuleId| first_read(v, id, &inline_ids).map(|(i, _)| i);
    for pass in 0..8 {
        if !seq.iter().any(|(id, _, _)| first_read(&cur, *id).is_some()) {
            break;
        }
        // Each inlined relation's rules, in the pass's order, as the tree pass gathers them at its start.
        let defs: BTreeMap<RelId, Vec<RuleId>> = inline_ids.iter().map(|&rel| (rel, seq.iter().map(|e| e.0).filter(|&id| cur.rule(id).is_some_and(|r| r.head().rel() == rel)).collect())).collect();
        let mut b = Builder::on(&cur);
        let mut next = Vec::with_capacity(seq.len());
        let mut changed = false;
        let (mut removed, mut added) = (Vec::new(), Vec::new());
        for (id, origin, into) in std::mem::take(&mut seq) {
            let Some((at, read)) = self::first_read(&cur, id, &inline_ids) else {
                next.push((id, origin, into));
                continue;
            };
            let reader = cur.rule(id).expect("a rule of the pass");
            let ds: Vec<RuleRef<'_>> = defs[&read.rel()].iter().map(|&d| cur.rule(d).expect("a def")).collect();
            match unfold(&mut b, &cur, reader, at, read, &ds, pass)? {
                Some(copies) => {
                    b.remove(id);
                    removed.push(id);
                    added.extend(copies.iter().copied());
                    let mut into = into;
                    into.insert(cur.rel_name(read.rel()).to_owned());
                    next.extend(copies.into_iter().map(|c| (c, origin, into.clone())));
                    changed = true;
                }
                None => next.push((id, origin, into)),
            }
        }
        let (v, made) = b.finish()?;
        renumber(&cur, &removed, &added, made, &mut next);
        cur = v;
        seq = next;
        if !changed {
            break;
        }
    }
    // An inlined relation some reader still reads (a chain deeper than the passes) keeps its rules.
    let unfinished: BTreeSet<String> = seq
        .iter()
        .filter_map(|(id, _, _)| cur.rule(*id))
        .filter(|r| !inline_ids.contains(&r.head().rel()))
        .flat_map(|r| r.body().filter_map(|l| l.atom()).collect::<Vec<_>>())
        .filter(|a| inline_ids.contains(&a.rel()))
        .map(|a| cur.rel_name(a.rel()).to_owned())
        .collect();
    let gone = |r: RuleRef<'_>| {
        let head = cur.rel_name(r.head().rel());
        inline.contains(head) && !unfinished.contains(head)
    };
    let drop_ids: Vec<RuleId> = seq.iter().filter(|(id, _, _)| cur.rule(*id).is_some_and(gone)).map(|e| e.0).collect();
    if !drop_ids.is_empty() {
        let mut b = Builder::on(&cur);
        for &id in &drop_ids {
            b.remove(id);
        }
        let (v, made) = b.finish()?;
        seq.retain(|e| !drop_ids.contains(&e.0));
        renumber(&cur, &drop_ids, &[], made, &mut seq);
        cur = v;
    }
    Ok(Inlined { v: cur, rules: seq, unfinished })
}

/// The ids of `seq`'s rules after an edit of `parent` (`removed` gone, `added` added) was folded (`Builder::finish`
/// folds an edit of half its parent or more): a fold numbers the rules from 0, in the overlay's order (the parent's
/// left, then the added).
fn renumber(parent: &Version, removed: &[RuleId], added: &[RuleId], made: Made, seq: &mut [(RuleId, RuleId, BTreeSet<String>)]) {
    if !matches!(made, Made::Folded) {
        return;
    }
    let gone: BTreeSet<RuleId> = removed.iter().copied().collect();
    let mut added = added.to_vec();
    added.sort_unstable();
    let order: BTreeMap<RuleId, RuleId> =
        parent.rules().map(|r| r.id()).filter(|id| !gone.contains(id)).chain(added).enumerate().map(|(k, id)| (id, RuleId(k as u32))).collect();
    for e in seq.iter_mut() {
        e.0 = order[&e.0];
    }
}

/// Rule `id`'s first read of an inlined relation: its place in the body, and the atom.
fn first_read<'v>(v: &'v Version, id: RuleId, inline: &BTreeSet<RelId>) -> Option<(usize, AtomRef<'v>)> {
    let r = v.rule(id)?;
    r.body().enumerate().find_map(|(i, l)| l.atom().filter(|a| inline.contains(&a.rel())).map(|a| (i, a)))
}

/// `reader` with its read at `at` (`read`) of a relation whose rules are `defs` unfolded into `b`: the copies' ids,
/// or `None` where a negated read is not a negation of plain literals (`keeps::unfold`). `fresh` names the copies'
/// variables apart.
fn unfold(b: &mut Builder<'_>, v: &Version, reader: RuleRef<'_>, at: usize, read: AtomRef<'_>, defs: &[RuleRef<'_>], fresh: usize) -> Result<Option<Vec<RuleId>>, String> {
    let read_args: Vec<TermRef<'_>> = read.args().collect();
    let mut out = Vec::new();
    if !read.neg() {
        for (n, def) in defs.iter().enumerate() {
            let n = n + 1;
            // Each of the def's variables named apart, but a head variable read as a variable: the reader's, once.
            let mut to: BTreeMap<Slot, String> = (0..def.var_count() as u32).map(|s| (Slot(s), format!("{}_i{fresh}_{n}", v.text(def.var(Slot(s)))))).collect();
            let mut eqs: Vec<(TermRef<'_>, TermRef<'_>)> = Vec::new();
            let mut matched: BTreeSet<Slot> = BTreeSet::new();
            for (h, r) in def.head().args().zip(&read_args) {
                match (h, *r) {
                    (_, TermRef::Wild) => {}
                    (TermRef::Var(s), TermRef::Var(x)) if matched.insert(s) => {
                        to.insert(s, v.text(reader.var(x)).to_owned());
                    }
                    _ => eqs.push((h, *r)),
                }
            }
            let names: BTreeMap<Slot, AtomId> = to.iter().map(|(s, name)| (*s, b.atom(name))).collect();
            let reader_vars: Vec<AtomId> = (0..reader.var_count() as u32).map(|s| b.atom(v.text(reader.var(Slot(s))))).collect();
            let eq_terms: Vec<(T<'_>, T<'_>)> = eqs.iter().map(|&(h, r)| (term(v, h, &|s| names[&s]), term(v, r, &|s| reader_vars[s.0 as usize]))).collect();
            let mut rn = Renamed { to: &names };
            let mut keep = Until { at, k: 0 };
            let mut rb = b.rewrite(v, reader, &mut keep);
            for l in def.body() {
                rb = rb.lit_with(v, *def, l, &mut rn);
            }
            for (h, r) in eq_terms {
                rb = rb.cmp(h, CmpOp::Eq, r);
            }
            for l in reader.body().skip(at + 1) {
                rb = rb.lit(v, reader, l);
            }
            out.push(rb.finish()?);
        }
        return Ok(Some(out));
    }
    let Some(alternatives) = negated(b, v, reader, read, defs) else { return Ok(None) };
    let mut copies: Vec<Vec<usize>> = vec![Vec::new()];
    for alts in &alternatives {
        copies = copies.into_iter().flat_map(|c| (0..alts.len()).map(move |k| {
            let mut c = c.clone();
            c.push(k);
            c
        })).collect();
        if copies.len() > COPIES * 4 {
            return Ok(None);
        }
    }
    for pick in copies {
        let mut keep = Until { at, k: 0 };
        let mut rb = b.rewrite(v, reader, &mut keep);
        for (alts, &k) in alternatives.iter().zip(&pick) {
            rb = match &alts[k] {
                Alt::Atom { rel, neg: true, args } => rb.not(*rel, args),
                Alt::Atom { rel, neg: false, args } => rb.atom(*rel, args),
                Alt::Cmp { l, op, r } => rb.cmp(l.clone(), *op, r.clone()),
            };
        }
        for l in reader.body().skip(at + 1) {
            rb = rb.lit(v, reader, l);
        }
        out.push(rb.finish()?);
    }
    Ok(Some(out))
}

/// A negated read's alternative literal (`keeps::negated`), its terms the builder's.
enum Alt<'v> {
    Atom { rel: RelId, neg: bool, args: Vec<T<'v>> },
    Cmp { l: T<'v>, op: CmpOp, r: T<'v> },
}

/// `keeps::negated` over the version: each def negated, a set of alternatives whose product is the copies.
fn negated<'v>(b: &mut Builder<'_>, v: &'v Version, reader: RuleRef<'_>, read: AtomRef<'_>, defs: &[RuleRef<'_>]) -> Option<Vec<Vec<Alt<'v>>>> {
    let read_args: Vec<TermRef<'_>> = read.args().collect();
    let read_vars: BTreeSet<Slot> = read_args.iter().filter_map(|t| t.var()).collect();
    let reader_vars: Vec<AtomId> = (0..reader.var_count() as u32).map(|s| b.atom(v.text(reader.var(Slot(s))))).collect();
    let mut alternatives = Vec::new();
    for def in defs {
        let heads: Vec<Slot> = def.head().args().map(|t| t.var()).collect::<Option<_>>()?;
        if heads.iter().collect::<BTreeSet<_>>().len() != heads.len() {
            return None;
        }
        // A def variable's substitute: the read's term (the reader's), or `_`.
        let mut to: BTreeMap<Slot, Sub<'_>> = heads.iter().zip(&read_args).map(|(h, t)| (*h, Sub::Read(*t))).collect();
        let mut count: BTreeMap<Slot, usize> = BTreeMap::new();
        for l in def.body() {
            if let Lit::Atom(a) = l.view() {
                for t in a.args() {
                    if let TermRef::Var(s) = t {
                        *count.entry(s).or_default() += 1;
                    }
                }
            }
        }
        let body: Vec<LitRef<'_>> = def.body().collect();
        let sub = |t: TermRef<'_>, to: &BTreeMap<Slot, Sub<'_>>| subst(v, t, to, &reader_vars);
        match body.as_slice() {
            [l] if matches!(l.view(), Lit::Atom(a) if !a.neg()) => {
                let Lit::Atom(a) = l.view() else { unreachable!() };
                for t in a.args() {
                    if let TermRef::Var(s) = t {
                        let free = !matches!(to.get(&s), Some(Sub::Read(TermRef::Var(_) | TermRef::Int(_) | TermRef::Str(_))));
                        if free && count[&s] != 1 {
                            return None;
                        }
                        if free {
                            to.insert(s, Sub::Wild);
                        }
                    }
                }
                let args = a.args().map(|t| sub(t, &to)).collect::<Option<_>>()?;
                alternatives.push(vec![Alt::Atom { rel: a.rel(), neg: true, args }]);
            }
            body => {
                let bound = |s: Slot| matches!(to.get(&s), Some(Sub::Read(TermRef::Var(x))) if read_vars.contains(x)) || matches!(to.get(&s), Some(Sub::Read(TermRef::Int(_) | TermRef::Str(_))));
                let mut alts = Vec::new();
                for l in body {
                    let mut vars = Vec::new();
                    lit_vars(l.view(), &mut vars);
                    if !vars.into_iter().all(bound) {
                        return None;
                    }
                    alts.push(match l.view() {
                        Lit::Atom(a) => Alt::Atom { rel: a.rel(), neg: !a.neg(), args: a.args().map(|t| sub(t, &to)).collect::<Option<_>>()? },
                        Lit::Cmp { l, op, r } => Alt::Cmp { l: sub(l, &to)?, op: negate(op), r: sub(r, &to)? },
                        _ => return None,
                    });
                }
                alternatives.push(alts);
            }
        }
    }
    Some(alternatives)
}

/// What a negated def's variable becomes.
#[derive(Clone, Copy)]
enum Sub<'r> {
    Read(TermRef<'r>),
    Wild,
}

fn lit_vars(l: Lit<'_>, out: &mut Vec<Slot>) {
    fn term(t: TermRef<'_>, out: &mut Vec<Slot>) {
        match t {
            TermRef::Var(s) => out.push(s),
            TermRef::Call(_, args) => args.for_each(|a| term(a, out)),
            _ => {}
        }
    }
    match l {
        Lit::Atom(a) => a.args().for_each(|t| term(t, out)),
        Lit::Cmp { l, r, .. } => {
            term(l, out);
            term(r, out);
        }
        _ => {}
    }
}

/// The comparison that holds where `op` does not (`keeps::negate`).
fn negate(op: CmpOp) -> CmpOp {
    match op {
        CmpOp::Eq => CmpOp::Ne,
        CmpOp::Ne => CmpOp::Eq,
        CmpOp::Lt => CmpOp::Ge,
        CmpOp::Ge => CmpOp::Lt,
        CmpOp::Gt => CmpOp::Le,
        CmpOp::Le => CmpOp::Gt,
    }
}

/// Term `t` of a def, each variable `to` names replaced (`keeps::subst`). `None` for a variable it does not name,
/// which `negated`'s checks leave none of (every variable is the read's, or `_`).
fn subst<'v>(v: &'v Version, t: TermRef<'_>, to: &BTreeMap<Slot, Sub<'_>>, reader_vars: &[AtomId]) -> Option<T<'v>> {
    Some(match t {
        TermRef::Var(s) => match to.get(&s)? {
            Sub::Wild => T::Wild,
            Sub::Read(r) => term(v, *r, &|x| reader_vars[x.0 as usize]),
        },
        TermRef::Call(f, args) => T::Call(v.text(f), args.map(|a| subst(v, a, to, reader_vars)).collect::<Option<_>>()?),
        other => term(v, other, &|_| unreachable!("no variable")),
    })
}

/// Term `t` as the builder writes it, each variable by the atom `var` names.
fn term<'v>(v: &'v Version, t: TermRef<'_>, var: &dyn Fn(Slot) -> AtomId) -> T<'v> {
    match t {
        TermRef::Var(s) => T::VarAtom(var(s)),
        TermRef::Wild => T::Wild,
        TermRef::Top => T::Top,
        TermRef::Int(i) => T::Int(i),
        TermRef::Str(a) => T::Str(v.text(a)),
        TermRef::Call(f, args) => T::Call(v.text(f), args.map(|a| term(v, a, var)).collect()),
    }
}

/// A reader's literals before `at` (the copy's literals follow).
struct Until {
    at: usize,
    k: usize,
}

impl Rewrite for Until {
    fn keep(&mut self, _l: LitRef<'_>) -> bool {
        self.k += 1;
        self.k <= self.at
    }
}

/// A def's literal written into a reader's copy: each of its variables renamed (`keeps::rename_all`).
struct Renamed<'a> {
    to: &'a BTreeMap<Slot, AtomId>,
}

impl Rewrite for Renamed<'_> {
    fn term(&mut self, _site: Site<'_>, t: TermRef<'_>) -> Option<T<'static>> {
        t.var().map(|s| T::VarAtom(self.to[&s]))
    }
    fn var(&mut self, s: Slot) -> Option<AtomId> {
        self.to.get(&s).copied()
    }
}
