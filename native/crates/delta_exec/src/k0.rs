//! L1, the K0 pass (decision 0686): one fixed walk of a `.rerail` tape that writes its rules' K-form, with no lowering.
//! It reads the kernel subset (K0): clauses of one head; bodies of atoms, negated atoms and comparisons, in the order
//! written; arguments that are variables, strings, numbers or `_`; facts. It refuses anything else, naming the node.
//!
//! The same reading is written as rules in `bootstrap-l0.rerail` (its first part, P2's reader). The self-reading
//! check holds the two to the same rows: P2, run over `l0`'s own tape, gives P2.
//!
//! As a delta operator it has one tape in, no program, one production (the K-form relations) and no facts in; the
//! file is its grain: an edited file is read again whole, and its rows are written as the change from the last.

use crate::db::{Db, Text};
use crate::kform::{RELATIONS, Refused};
use crate::sym::Symbols;
use crate::tape::{Names, Reads};
use rerail_rr_syntax::tape::Kind;
use rerail_blocks::bistate::Read;
use rustc_hash::FxHashSet;

/// The K-form rows of document `d`'s tape, each relation's set, in `RELATIONS`' order (`k_order`'s empty).
pub fn read(text: &Text, d: i64, sym: &Symbols) -> Result<[FxHashSet<Box<[i64]>>; 8], Refused> {
    let names = Names::new(sym);
    let t = Reads::of(text);
    let mut out: [FxHashSet<Box<[i64]>>; 8] = Default::default();
    let refuse = |n: usize, why: &str| Refused { module: d, rule: n as i64, why: why.to_string() };
    let field = |p: usize, f: &str| -> Option<usize> {
        let f = sym.intern(f);
        t.children(p).find(|&c| names.field(t.node(c).1) == f)
    };
    let fields = |p: usize, f: &str| -> Vec<usize> {
        let f = sym.intern(f);
        t.children(p).filter(|&c| names.field(t.node(c).1) == f).collect()
    };
    let (atom, not, cmp) = (sym.intern("atom"), sym.intern("not"), sym.intern("cmp"));
    let mut put = |rel: usize, row: &[i64]| {
        // copy: derived (a K-form row, written once where the pass computes it)
        out[rel].insert(row.into());
    };
    // A term: what it is, by its node.
    let term = |n: usize, put: &mut dyn FnMut(usize, &[i64])| -> Result<(), Refused> {
        let (k, _, _) = t.node(n);
        if k == Kind::Variable as u16 {
            put(3, &[d, n as i64, sym.intern(t.text(n))]);
        } else if k == Kind::String as u16 {
            put(4, &[d, n as i64, t.value(n, sym).ok_or_else(|| refuse(n, "a string that does not unescape"))?]);
        } else if k == Kind::Number as u16 {
            put(5, &[d, n as i64, t.value(n, sym).ok_or_else(|| refuse(n, "a number past an i64"))?]);
        } else if k == Kind::Wildcard as u16 {
            put(6, &[d, n as i64]);
        } else {
            return Err(refuse(n, "an argument that is no variable, string, number or `_` (not K0)"));
        }
        Ok(())
    };
    // An argument list's arguments, as step `k`'s.
    let args = |c: usize, k: i64, list: usize, put: &mut dyn FnMut(usize, &[i64])| -> Result<(), Refused> {
        for (i, a) in t.children(list).filter(|&a| t.counted(a)).enumerate() {
            put(2, &[d, c as i64, k, i as i64, a as i64]);
            term(a, put)?;
        }
        Ok(())
    };
    let relation = |n: usize| -> Result<i64, Refused> { Ok(sym.intern(t.text(field(n, "relation").ok_or_else(|| refuse(n, "no relation"))?))) };
    for c in t.children(0).filter(|&c| t.counted(c)) {
        let (k, _, _) = t.node(c);
        if k != Kind::Clause as u16 {
            return Err(refuse(c, "a statement that is no clause (not K0)"));
        }
        let heads = fields(c, "head");
        let [h] = heads[..] else { return Err(refuse(c, "a clause of several heads (not K0)")) };
        if field(h, "modal").is_some() {
            return Err(refuse(h, "`must` or `may` (not K0)"));
        }
        put(0, &[d, c as i64, relation(h)?]);
        if let Some(list) = field(h, "arguments") {
            args(c, -1, list, &mut put)?;
        }
        if field(c, "edit").is_some() {
            return Err(refuse(c, "an edit (not K0)"));
        }
        let Some(b) = field(c, "body") else { continue };
        if t.node(b).0 != Kind::Conjunction as u16 {
            return Err(refuse(b, "a body that is no conjunction (not K0)"));
        }
        for (i, l) in t.children(b).filter(|&l| t.counted(l)).enumerate() {
            let i = i as i64;
            let (lk, _, _) = t.node(l);
            if lk == Kind::Atom as u16 {
                put(1, &[d, c as i64, i, atom, relation(l)?]);
                if let Some(list) = field(l, "arguments") {
                    args(c, i, list, &mut put)?;
                }
            } else if lk == Kind::Negation as u16 {
                let o = field(l, "operand").ok_or_else(|| refuse(l, "a negation of nothing"))?;
                if t.node(o).0 != Kind::Atom as u16 {
                    return Err(refuse(o, "a negation of no atom (not K0)"));
                }
                put(1, &[d, c as i64, i, not, relation(o)?]);
                if let Some(list) = field(o, "arguments") {
                    args(c, i, list, &mut put)?;
                }
            } else if lk == Kind::Comparison as u16 {
                let op = field(l, "operator").ok_or_else(|| refuse(l, "a comparison with no operator"))?;
                put(1, &[d, c as i64, i, cmp, sym.intern(t.text(op))]);
                for (j, side) in ["left", "right"].into_iter().enumerate() {
                    let a = field(l, side).ok_or_else(|| refuse(l, "a comparison with a side missing"))?;
                    put(2, &[d, c as i64, i, j as i64, a as i64]);
                    term(a, &mut put)?;
                }
            } else {
                return Err(refuse(l, "a literal that is no atom, negated atom or comparison (not K0)"));
            }
        }
    }
    Ok(out)
}

/// L1: the K-form of documents `docs` (each its module id and text), written into `prog` as the change from what
/// it held for those modules; other modules' rows are kept (the file is the grain).
pub fn pass(docs: &[(i64, &Text)], sym: &Symbols, prog: &mut Db) -> Result<(), Refused> {
    let mut all: [FxHashSet<Box<[i64]>>; 8] = Default::default();
    for (d, t) in docs {
        for (i, set) in read(t, *d, sym)?.into_iter().enumerate() {
            all[i].extend(set);
        }
    }
    let modules: FxHashSet<i64> = docs.iter().map(|d| d.0).collect();
    // `k_order` is not the pass's: a module's stratum is given by the load that asked for it.
    for (i, set) in all.iter().enumerate().take(7) {
        let (name, arity) = (sym.intern(RELATIONS[i].0), RELATIONS[i].1);
        prog.declare(name, arity);
        prog.write(name, |w, version| {
            let mut kept = FxHashSet::default();
            let mut row = vec![0i64; arity];
            for id in version.live_ids() {
                crate::db::read_row(version, id, &mut row);
                if !modules.contains(&row[0]) {
                    continue;
                }
                if set.contains(&row[..]) {
                    kept.insert(row.clone().into_boxed_slice());
                } else {
                    w.kill(id.get());
                }
            }
            for r in set.iter().filter(|r| !kept.contains(*r)) {
                w.push(r);
            }
        });
    }
    Ok(())
}
