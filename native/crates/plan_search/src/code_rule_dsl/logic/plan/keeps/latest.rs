//! The design's S5 ("latest in an ordered group"), the planner's side. A relation `a` whose rules each take a min or
//! max over one relation `g`'s group, `a(k, u) :- …, u = max t : { g(…, t), t < r }`, read by one rule only, which
//! joins `g` back at it (`c(…) :- …, a(k, u), g(…, u)`), is unfolded into that reader (`inline::inline`): no aggregate
//! relation is made. Then two positive reads of one relation that a key the layouts proved (decision 0361) binds to
//! the same terms are one read (`merged`): the reader's `rf_fn_read(i, s, x, _)` and the unfolded
//! `rf_fn_read(i, s1, x1, r)`, keyed by `i`, become `rf_fn_read(i, s, x, r)`, so the max's group and the join back
//! read the same columns, and the engine runs the two as one step (setfix's `back_at_extreme`). Structure only: never
//! a relation's name. `RERAIL_SHAPE_LATEST=off` turns it off.

use super::inline;
use super::super::body::Placed;
use super::super::fields::Fields;
use super::super::super::program_image::{AggRef, AtomId, AtomRef, Builder, Lit, LitRef, RelId, Rewrite, RuleId, RuleRef, Site, Slot, T, TermRef, Version};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// Whether relation `rel` has S5's shape: every rule a min or max (bounded or not) over one atom of one relation `g`,
/// at one column, its value the head's last column, the head's columns plain variables; and one reader, positive,
/// whose rule joins `g` back at that value (an atom of `g` with the read's last variable at that column).
pub fn has_shape(v: &Version, rel: RelId) -> bool {
    let defs: Vec<RuleRef<'_>> = v.rules().filter(|r| r.head().rel() == rel).collect();
    if defs.is_empty() {
        return false;
    }
    // Each rule's aggregate: its relation and the column of the value it folds.
    let mut over: Option<(RelId, usize)> = None;
    for d in &defs {
        let head: Vec<TermRef<'_>> = d.head().args().collect();
        let Some(TermRef::Var(last)) = head.last().copied() else { return false };
        if head.iter().any(|t| t.var().is_none()) {
            return false;
        }
        let aggs: Vec<(Slot, AggRef<'_>, Option<Slot>, AtomRef<'_>)> = d.body().filter_map(|l| match l.view() { Lit::Agg { var, op, over, atom } => Some((var, op, over, atom)), _ => None }).collect();
        let [(var, op, Some(t), atom)] = aggs.as_slice() else { return false };
        if *var != last || !matches!(op, AggRef::Max | AggRef::Min | AggRef::Bounded { .. }) || atom.neg() {
            return false;
        }
        let Some(col) = atom.args().position(|a| a.var() == Some(*t)) else { return false };
        match over {
            None => over = Some((atom.rel(), col)),
            Some(o) if o == (atom.rel(), col) => {}
            Some(_) => return false,
        }
    }
    let Some((g, col)) = over else { return false };
    let readers: Vec<(RuleRef<'_>, AtomRef<'_>)> = v.readers(rel).collect();
    let [(reader, read)] = readers.as_slice() else { return false };
    if read.neg() || reader.head().rel() == rel {
        return false;
    }
    let Some(TermRef::Var(u)) = read.args().last() else { return false };
    reader.body().any(|l| matches!(l.view(), Lit::Atom(a) if !a.neg() && a.rel() == g && a.args().nth(col).and_then(|x| x.var()) == Some(u)))
}

/// The program's relations of S5's shape unfolded into their readers, the reads a key makes one merged (`merged`):
/// the relations and the version, an overlay of `base`'s; `None` where none has the shape. `keys(rel)` gives the
/// column sets the layouts proved to be `rel`'s keys.
pub fn latest(base: &Placed, keys: &dyn Fn(&str) -> Vec<Vec<usize>>) -> Result<Option<(Vec<String>, Placed)>, String> {
    if std::env::var("RERAIL_SHAPE_LATEST").is_ok_and(|v| v == "off") {
        return Ok(None);
    }
    let Some(f) = Fields::of(base) else { return Ok(None) };
    let v: &Version = &base.v;
    let heads: BTreeSet<RelId> = v.rules().map(|r| r.head().rel()).collect();
    let chosen: BTreeSet<String> = heads
        .into_iter()
        .filter(|&r| {
            let name = v.rel_name(r);
            !f.is_output(name) && !f.qualified(name, "no_inline") && !name.contains("__") && has_shape(v, r)
        })
        .map(|r| v.rel_name(r).to_owned())
        .collect();
    if chosen.is_empty() {
        return Ok(None);
    }
    let got = inline::inline(&base.v, &base.order(), &chosen)?;
    let done: Vec<String> = chosen.iter().filter(|r| !got.unfinished.contains(*r)).cloned().collect();
    if done.is_empty() {
        return Ok(None);
    }
    // The copies, each read a key makes one merged.
    let copies: BTreeSet<RuleId> = got.rules.iter().filter(|(_, _, into)| into.iter().any(|r| chosen.contains(r))).map(|(id, _, _)| *id).collect();
    let mut order: Vec<RuleId> = got.rules.iter().map(|(id, _, _)| *id).collect();
    let mut cur = Arc::clone(&got.v);
    let mut todo: BTreeSet<RuleId> = copies;
    for _ in 0..4 {
        let mut b = Builder::on(&cur);
        let mut moved: BTreeMap<RuleId, RuleId> = BTreeMap::new();
        for &id in &todo {
            let Some(r) = cur.rule(id) else { continue };
            if let Some(new) = merged(&mut b, &cur, r, keys)? {
                b.remove(id);
                moved.insert(id, new);
            }
        }
        if moved.is_empty() {
            break;
        }
        let (v2, _, placing) = b.finish_placing()?;
        let place = |id: RuleId| placing.as_ref().map_or(id, |p| p[&id]);
        for id in order.iter_mut() {
            *id = place(moved.get(id).copied().unwrap_or(*id));
        }
        todo = moved.values().map(|&id| place(id)).collect();
        cur = v2;
    }
    super::super::super::profile::decision("latest in a group (the design's S5)", || format!("{} min or max relations made in their readers: {}", done.len(), done.join(", ")));
    Ok(Some((done, Placed::new(cur, order, base.whole))))
}

/// Rule `r` with two positive reads of one relation merged, where one of `keys(rel)` binds both to the same terms: the
/// first kept, its wildcards filled from the second's variables, the second's variables written as the first's
/// terms there. `None` where no pair merges (or a merge would need a comparison: a constant against a variable).
pub fn merged(b: &mut Builder<'_>, v: &Version, r: RuleRef<'_>, keys: &dyn Fn(&str) -> Vec<Vec<usize>>) -> Result<Option<RuleId>, String> {
    let atoms: Vec<(LitRef<'_>, AtomRef<'_>)> = r.body().filter_map(|l| match l.view() { Lit::Atom(a) if !a.neg() => Some((l, a)), _ => None }).collect();
    let same = |x: TermRef<'_>, y: TermRef<'_>| match (x, y) {
        (TermRef::Var(a), TermRef::Var(b)) => a == b,
        (TermRef::Int(a), TermRef::Int(b)) => a == b,
        (TermRef::Str(a), TermRef::Str(b)) => a == b,
        (TermRef::Branded(a), TermRef::Branded(b)) => a == b,
        _ => false,
    };
    for (i, (_, p)) in atoms.iter().enumerate() {
        for (ql, q) in &atoms[i + 1..] {
            if p.rel() != q.rel() {
                continue;
            }
            let (pa, qa): (Vec<TermRef<'_>>, Vec<TermRef<'_>>) = (p.args().collect(), q.args().collect());
            if pa.len() != qa.len() || !keys(v.rel_name(p.rel())).iter().any(|k| !k.is_empty() && k.iter().all(|&c| c < pa.len() && same(pa[c], qa[c]))) {
                continue;
            }
            // Column by column: the first's wildcard takes the second's variable; the second's variable is the
            // first's term.
            let mut fill: BTreeMap<usize, AtomId> = BTreeMap::new();
            let mut sub: BTreeMap<Slot, Sub> = BTreeMap::new();
            let mut ok = true;
            for (c, (&x, &y)) in pa.iter().zip(&qa).enumerate() {
                let to = match (x, y) {
                    (_, TermRef::Wild) => continue,
                    (TermRef::Wild, TermRef::Var(s)) => {
                        fill.insert(c, b.atom(v.text(r.var(s))));
                        continue;
                    }
                    (TermRef::Var(ps), TermRef::Var(s)) if ps == s => continue,
                    (TermRef::Var(ps), TermRef::Var(s)) => (s, Sub::Var(b.atom(v.text(r.var(ps))))),
                    (TermRef::Int(n), TermRef::Var(s)) => (s, Sub::Int(n)),
                    (TermRef::Str(a), TermRef::Var(s)) => (s, Sub::Str(b.atom(v.text(a)))),
                    (TermRef::Branded(a), TermRef::Var(s)) => (s, Sub::Branded(a)),
                    (x, y) if same(x, y) => continue,
                    _ => {
                        ok = false;
                        break;
                    }
                };
                if sub.get(&to.0).is_some_and(|t| *t != to.1) {
                    ok = false;
                    break;
                }
                sub.insert(to.0, to.1);
            }
            if !ok {
                continue;
            }
            let mut rw = Merge { drop: ql.node(), keep: p.node(), fill, sub };
            // The merge makes the second read's other literals the first's where they test the merged variables
            // (`!kind(s1, "module")` beside `!kind(s, "module")`): written once.
            return Ok(Some(b.rewrite(v, r, &mut rw).each_once().finish()?));
        }
    }
    Ok(None)
}

/// What a merged read's variable is written as (`merged`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sub {
    Var(AtomId),
    Int(i64),
    Str(AtomId),
    Branded(crate::code_rule_dsl::logic::front::ast::Branded),
}

/// `merged`'s rewrite: the read `drop` left out, the read `keep`'s wildcards at `fill` filled, and `sub`'s variables
/// written as their terms.
struct Merge {
    drop: u32,
    keep: u32,
    fill: BTreeMap<usize, AtomId>,
    sub: BTreeMap<Slot, Sub>,
}

impl Rewrite for Merge {
    fn keep(&mut self, l: LitRef<'_>) -> bool {
        l.node() != self.drop
    }
    fn arg(&mut self, a: AtomRef<'_>, c: usize, _t: TermRef<'_>) -> Option<T<'static>> {
        (a.node() == self.keep).then(|| self.fill.get(&c).map(|&x| T::VarAtom(x))).flatten()
    }
    fn term(&mut self, _site: Site<'_>, t: TermRef<'_>) -> Option<T<'static>> {
        t.var().and_then(|s| self.sub.get(&s)).map(|s| match *s {
            Sub::Var(a) => T::VarAtom(a),
            Sub::Int(n) => T::Int(n),
            Sub::Str(a) => T::StrAtom(a),
            Sub::Branded(a) => T::Branded(a),
        })
    }
    fn var(&mut self, s: Slot) -> Option<AtomId> {
        match self.sub.get(&s) {
            Some(Sub::Var(a)) => Some(*a),
            _ => None,
        }
    }
}
