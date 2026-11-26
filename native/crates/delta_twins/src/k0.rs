//! L1's reading of a K0 tape into K-form rows (decision 0686; Bootstrap from one tape, §4): one fixed walk of a
//! `.rerail` tape, with no lowering. It reads the kernel subset (K0): clauses of one head; bodies of atoms, negated
//! atoms and comparisons, in the order written; arguments that are variables, strings, numbers or `_`; facts. It
//! refuses anything else, naming the node, the first by node order. K0 has no arithmetic.
//!
//! Its symbols are read, never made: a leaf's text and a string's value from the document's symbols block (`intern`'s
//! node → symbol facts), and the names it compares (fields, K-form's step kinds) found in the atoms block.
//!
//! The same reading is written as rules in `bootstrap-l0.rerail` (P2's reader). The self-reading check holds the two
//! to the same rows: P2, run over `l0`'s own tape, gives P2.

use crate::intern as syms;
use crate::tape::Reads;
use rerail_blocks::atoms::find_in;
use rerail_blocks::maps::hash_bytes;
use rerail_rr_syntax::tape::{FIELD_NAMES, Kind};

/// A refusal: the node, and why.
pub type Refusal = (usize, &'static str);

/// Walks document `d`'s tape, giving each K-form row to `put` with its relation's place in K-form's eight (`k_order`
/// is not the pass's: a module's stratum is its file's place in the bootstrap, `boot(path, k)`). Every step is at key
/// `(k, -1)`, and the head at `(-1, -1)`.
pub fn walk(t: &Reads<'_>, d: i64, syms: &[u8], atoms: &[u8], put: &mut dyn FnMut(usize, &[i64])) -> Result<(), Refusal> {
    let name = |s: &str| find_in(atoms, s, hash_bytes(s.as_bytes())).map_or(0, i64::from);
    let field_id = |f: &str| FIELD_NAMES.iter().position(|n| *n == f).expect("an rr field") as u16;
    let field = |p: usize, f: &str| -> Option<usize> {
        let f = field_id(f);
        t.children(p).find(|&c| t.node(c).1 == f)
    };
    let text = |n: usize| i64::from(syms::text(syms, n));
    let (atom, not, cmp) = (name("atom"), name("not"), name("cmp"));
    // A term: what it is, by its node.
    let term = |n: usize, put: &mut dyn FnMut(usize, &[i64])| -> Result<(), Refusal> {
        let (k, _, _) = t.node(n);
        if k == Kind::Variable as u16 {
            put(3, &[d, n as i64, text(n)]);
        } else if k == Kind::String as u16 {
            let v = syms::value(syms, n);
            if v == 0 {
                return Err((n, "a string that does not unescape"));
            }
            put(4, &[d, n as i64, i64::from(v)]);
        } else if k == Kind::Number as u16 {
            put(5, &[d, n as i64, t.number(n).ok_or((n, "a number past an i64"))?]);
        } else if k == Kind::Wildcard as u16 {
            put(6, &[d, n as i64]);
        } else {
            return Err((n, "an argument that is no variable, string, number or `_` (not K0)"));
        }
        Ok(())
    };
    // An argument list's arguments, as step `k`'s of clause `c`.
    let args = |c: usize, k: i64, list: usize, put: &mut dyn FnMut(usize, &[i64])| -> Result<(), Refusal> {
        for (i, a) in t.children(list).filter(|&a| t.counted(a)).enumerate() {
            put(2, &[d, c as i64, k, -1, i as i64, a as i64]);
            term(a, put)?;
        }
        Ok(())
    };
    let relation = |n: usize| -> Result<i64, Refusal> { Ok(text(field(n, "relation").ok_or((n, "no relation"))?)) };
    let head = field_id("head");
    for c in t.children(0).filter(|&c| t.counted(c)) {
        if t.node(c).0 != Kind::Clause as u16 {
            return Err((c, "a statement that is no clause (not K0)"));
        }
        let mut heads = t.children(c).filter(|&x| t.node(x).1 == head);
        let (Some(h), None) = (heads.next(), heads.next()) else {
            return Err((c, "a clause of several heads (not K0)"));
        };
        if field(h, "modal").is_some() {
            return Err((h, "`must` or `may` (not K0)"));
        }
        if field(c, "edit").is_some() {
            return Err((c, "an edit (not K0)"));
        }
        put(0, &[d, c as i64, relation(h)?]);
        if let Some(list) = field(h, "arguments") {
            args(c, -1, list, put)?;
        }
        let Some(b) = field(c, "body") else { continue };
        if t.node(b).0 != Kind::Conjunction as u16 {
            return Err((b, "a body that is no conjunction (not K0)"));
        }
        for (i, l) in t.children(b).filter(|&l| t.counted(l)).enumerate() {
            let i = i as i64;
            let lk = t.node(l).0;
            if lk == Kind::Atom as u16 {
                put(1, &[d, c as i64, i, -1, atom, relation(l)?]);
                if let Some(list) = field(l, "arguments") {
                    args(c, i, list, put)?;
                }
            } else if lk == Kind::Negation as u16 {
                let o = field(l, "operand").ok_or((l, "a negation of nothing"))?;
                if t.node(o).0 != Kind::Atom as u16 {
                    return Err((o, "a negation of no atom (not K0)"));
                }
                put(1, &[d, c as i64, i, -1, not, relation(o)?]);
                if let Some(list) = field(o, "arguments") {
                    args(c, i, list, put)?;
                }
            } else if lk == Kind::Comparison as u16 {
                let op = field(l, "operator").ok_or((l, "a comparison with no operator"))?;
                put(1, &[d, c as i64, i, -1, cmp, text(op)]);
                for (j, side) in ["left", "right"].into_iter().enumerate() {
                    let a = field(l, side).ok_or((l, "a comparison with a side missing"))?;
                    put(2, &[d, c as i64, i, -1, j as i64, a as i64]);
                    term(a, put)?;
                }
            } else {
                return Err((l, "a literal that is no atom, negated atom or comparison (not K0)"));
            }
        }
    }
    Ok(())
}
