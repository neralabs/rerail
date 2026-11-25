//! L1, the K0 pass (decision 0686): one fixed walk of a `.rerail` tape that writes its rules' K-form, with no lowering.
//! It reads the kernel subset (K0): clauses of one head; bodies of atoms, negated atoms and comparisons, in the order
//! written; arguments that are variables, strings, numbers, `_` or arithmetic of them; facts. It refuses anything
//! else, naming the node.
//!
//! The same reading is written as rules in `bootstrap-l0.rerail` (P2's reader). The self-reading check holds the two
//! to the same rows: P2, run over `l0`'s own tape, gives P2.
//!
//! As a delta operator it has tapes in, no program, one production (the K-form relations) and no facts in. The file
//! is its grain: a changed file's module has every old row gone and every new row added; an unchanged one is not read.
//! Its rows are written where the walk finds them.

use crate::db::{Db, Text};
use crate::kform::{RELATIONS, Refused};
use crate::sym::Symbols;
use crate::tape::{Names, Reads};
use rerail_blocks::bistate::ColumnIndex;
use rerail_blocks::table::Table;
use rerail_rr_syntax::tape::Kind;

/// Walks document `d`'s tape, giving each K-form row to `put` with its relation's place in `RELATIONS` (`k_order` is
/// not the pass's: a module's stratum is given by the load that asked for it).
pub fn walk(
    text: &Text,
    d: i64,
    sym: &Symbols,
    put: &mut dyn FnMut(usize, &[i64]),
) -> Result<(), Refused> {
    let names = Names::new(sym);
    let t = Reads::of(text);
    let refuse = |n: usize, why: &str| Refused {
        module: d,
        rule: n as i64,
        why: why.to_string(),
    };
    let field = |p: usize, f: &str| -> Option<usize> {
        let f = sym.intern(f);
        t.children(p).find(|&c| names.field(t.node(c).1) == f)
    };
    let (atom, not, cmp) = (sym.intern("atom"), sym.intern("not"), sym.intern("cmp"));
    // A term: what it is, by its node.
    fn term(
        t: &Reads<'_>,
        n: usize,
        d: i64,
        sym: &Symbols,
        field: &dyn Fn(usize, &str) -> Option<usize>,
        put: &mut dyn FnMut(usize, &[i64]),
    ) -> Result<(), Refused> {
        let refuse = |n: usize, why: &str| Refused {
            module: d,
            rule: n as i64,
            why: why.to_string(),
        };
        let (k, _, _) = t.node(n);
        if k == Kind::Variable as u16 {
            put(3, &[d, n as i64, sym.intern(t.text(n))]);
        } else if k == Kind::String as u16 {
            put(
                4,
                &[
                    d,
                    n as i64,
                    t.value(n, sym)
                        .ok_or_else(|| refuse(n, "a string that does not unescape"))?,
                ],
            );
        } else if k == Kind::Number as u16 {
            put(
                5,
                &[
                    d,
                    n as i64,
                    t.value(n, sym)
                        .ok_or_else(|| refuse(n, "a number past an i64"))?,
                ],
            );
        } else if k == Kind::Wildcard as u16 {
            put(6, &[d, n as i64]);
        } else if k == Kind::BinaryExpression as u16 {
            let side =
                |f: &str| field(n, f).ok_or_else(|| refuse(n, "an expression with a part missing"));
            let (l, o, r) = (side("left")?, side("operator")?, side("right")?);
            put(7, &[d, n as i64, sym.intern(t.text(o)), l as i64, r as i64]);
            term(t, l, d, sym, field, put)?;
            term(t, r, d, sym, field, put)?;
        } else {
            return Err(refuse(
                n,
                "an argument that is no variable, string, number, `_` or arithmetic (not K0)",
            ));
        }
        Ok(())
    }
    // An argument list's arguments, as step `k`'s of clause `c`.
    let args = |c: usize,
                k: i64,
                list: usize,
                put: &mut dyn FnMut(usize, &[i64])|
     -> Result<(), Refused> {
        for (i, a) in t.children(list).filter(|&a| t.counted(a)).enumerate() {
            put(2, &[d, c as i64, k, i as i64, a as i64]);
            term(&t, a, d, sym, &field, put)?;
        }
        Ok(())
    };
    let relation = |n: usize| -> Result<i64, Refused> {
        Ok(sym.intern(t.text(field(n, "relation").ok_or_else(|| refuse(n, "no relation"))?)))
    };
    for c in t.children(0).filter(|&c| t.counted(c)) {
        if t.node(c).0 != Kind::Clause as u16 {
            return Err(refuse(c, "a statement that is no clause (not K0)"));
        }
        let head = sym.intern("head");
        let heads: Vec<usize> = t
            .children(c)
            .filter(|&x| names.field(t.node(x).1) == head)
            .collect();
        let [h] = heads[..] else {
            return Err(refuse(c, "a clause of several heads (not K0)"));
        };
        if field(h, "modal").is_some() {
            return Err(refuse(h, "`must` or `may` (not K0)"));
        }
        if field(c, "edit").is_some() {
            return Err(refuse(c, "an edit (not K0)"));
        }
        put(0, &[d, c as i64, relation(h)?]);
        if let Some(list) = field(h, "arguments") {
            args(c, -1, list, put)?;
        }
        let Some(b) = field(c, "body") else { continue };
        if t.node(b).0 != Kind::Conjunction as u16 {
            return Err(refuse(b, "a body that is no conjunction (not K0)"));
        }
        for (i, l) in t.children(b).filter(|&l| t.counted(l)).enumerate() {
            let i = i as i64;
            let lk = t.node(l).0;
            if lk == Kind::Atom as u16 {
                put(1, &[d, c as i64, i, atom, relation(l)?]);
                if let Some(list) = field(l, "arguments") {
                    args(c, i, list, put)?;
                }
            } else if lk == Kind::Negation as u16 {
                let o = field(l, "operand").ok_or_else(|| refuse(l, "a negation of nothing"))?;
                if t.node(o).0 != Kind::Atom as u16 {
                    return Err(refuse(o, "a negation of no atom (not K0)"));
                }
                put(1, &[d, c as i64, i, not, relation(o)?]);
                if let Some(list) = field(o, "arguments") {
                    args(c, i, list, put)?;
                }
            } else if lk == Kind::Comparison as u16 {
                let op = field(l, "operator")
                    .ok_or_else(|| refuse(l, "a comparison with no operator"))?;
                put(1, &[d, c as i64, i, cmp, sym.intern(t.text(op))]);
                for (j, side) in ["left", "right"].into_iter().enumerate() {
                    let a = field(l, side)
                        .ok_or_else(|| refuse(l, "a comparison with a side missing"))?;
                    put(2, &[d, c as i64, i, j as i64, a as i64]);
                    term(&t, a, d, sym, &field, put)?;
                }
            } else {
                return Err(refuse(
                    l,
                    "a literal that is no atom, negated atom or comparison (not K0)",
                ));
            }
        }
    }
    Ok(())
}

/// L1: the K-form of documents `docs` (each its module id, its text before when it had one, and its text now),
/// written into `prog`: a changed document's module rows gone and its new rows added, an unchanged one's left.
pub fn pass(
    docs: &[(i64, Option<&Text>, &Text)],
    sym: &Symbols,
    prog: &mut Db,
) -> Result<(), Refused> {
    let changed: Vec<(i64, &Text)> = docs
        .iter()
        .filter(|(_, b, a)| b.is_none_or(|b| b.src != a.src))
        .map(|&(d, _, a)| (d, a))
        .collect();
    // Each file is read whole before any row is written: a refused file writes nothing.
    for &(d, t) in &changed {
        walk(t, d, sym, &mut |_, _| {})?;
    }
    crate::kform::indexes(prog, &crate::kform::Words::new(sym));
    for (i, &(n, _)) in RELATIONS.iter().enumerate().take(8) {
        let name = sym.intern(n);
        prog.write(name, |w, version| {
            for &(d, _) in &changed {
                // Its rows are sorted on column 0, the module: a range of each run.
                Table::probe(version, &[ColumnIndex::new(0)], &[d], &mut |id| {
                    w.kill(id.get());
                    true
                });
            }
            for &(d, t) in &changed {
                walk(t, d, sym, &mut |r, row| {
                    if r == i {
                        w.push(row);
                    }
                })
                .expect("read once already");
            }
        });
    }
    Ok(())
}

/// The K-form rows of one document, by relation, for checks and tests.
pub fn rows(
    text: &Text,
    d: i64,
    sym: &Symbols,
) -> Result<[std::collections::BTreeSet<Vec<i64>>; 9], Refused> {
    let mut out: [std::collections::BTreeSet<Vec<i64>>; 9] = Default::default();
    // copy: not block data (a check's rows)
    walk(text, d, sym, &mut |r, row| {
        out[r].insert(row.to_vec());
    })?;
    Ok(out)
}
