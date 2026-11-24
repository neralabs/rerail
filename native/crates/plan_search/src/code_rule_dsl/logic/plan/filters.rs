//! A rule read as a filter of one relation (decision 0611, t52 step 3): where every row of its source passes, the
//! filter is read as its source. The search's keeps (`keeps`) and the run (`plan.rs`) read it alike.
use super::fields::Fields;
use super::super::program_image::{CmpOp, Lit, RuleRef, Slot as VarSlot, TermRef, Version};

/// Relation `rel`'s one rule `rule` as a filter its source's rows can stand for (decision 0611): `filter_of`, its head
/// a plain relation. A lattice's rows are its groups' merges, a gate's and a native operator's are made otherwise.
pub fn plain_filter(f: &Fields<'_>, rel: &str, v: &Version, rule: RuleRef<'_>) -> Option<(String, Vec<(usize, i64)>)> {
    if f.lattice(rel) || f.gate(rel) || f.native(rel).is_some() {
        return None;
    }
    filter_of(v, rule)
}

/// Rule `rule` as a filter of one relation (t52 step 3): `h(v0, …) :- x(v0, …), $vi < b, …`, the head the atom's
/// distinct variables in order, every other literal one of them compared under a constant. `x`, and each compared
/// column with its bound.
pub fn filter_of(v: &Version, rule: RuleRef<'_>) -> Option<(String, Vec<(usize, i64)>)> {
    let head: Vec<VarSlot> = rule.head().args().map(|t| t.var()).collect::<Option<_>>()?;
    if head.iter().enumerate().any(|(i, x)| head[..i].contains(x)) {
        return None;
    }
    let mut atom = None;
    let mut cmps = Vec::new();
    for l in rule.body() {
        match l.view() {
            Lit::Atom(a) if !a.neg() && atom.is_none() => atom = Some(a),
            Lit::Cmp { op: CmpOp::Lt, l: TermRef::Var(x), r: TermRef::Int(b) } => cmps.push((x, b)),
            _ => return None,
        }
    }
    let a = atom?;
    let args: Vec<VarSlot> = a.args().map(|t| t.var()).collect::<Option<_>>()?;
    if args != head {
        return None;
    }
    let bounds = cmps.into_iter().map(|(x, b)| args.iter().position(|y| *y == x).map(|c| (c, b))).collect::<Option<Vec<_>>>()?;
    Some((v.rel_name(a.rel()).to_owned(), bounds))
}
