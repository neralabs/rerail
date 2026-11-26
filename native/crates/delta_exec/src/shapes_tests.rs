use super::*;
use crate::body::FEW;
use crate::program::Step;

const E: i64 = 1;
const F: i64 = 2;
const G: i64 = 3;
const M: i64 = 4;
const P: i64 = 9;
const X: Term = Term::Var(100);
const Y: Term = Term::Var(101);
const Z: Term = Term::Var(102);
const W: Term = Term::Var(103);

fn step(kind: Kind, rel: i64, args: &[Term]) -> Step {
    Step { key: (0, 0), kind, rel, args: args.to_vec() }
}

fn atom(rel: i64, args: &[Term]) -> Step {
    step(Kind::Atom, rel, args)
}

fn rule(args: &[Term], steps: Vec<Step>) -> Rule {
    Rule { module: 0, id: 0, head: P, args: args.to_vec(), steps }
}

/// e(x, y): key x, fanout y few; f(y, z): key y; g(z): key z.
fn base(r: i64) -> Option<Facts> {
    match r {
        E => Some(Facts { keys: vec![0b01], caps: vec![(1, FEW)] }),
        F => Some(Facts { keys: vec![0b01], caps: Vec::new() }),
        G => Some(Facts { keys: vec![0b1], caps: Vec::new() }),
        _ => None,
    }
}

fn of(r: &Rule) -> Facts {
    infer(P, &[r], false, &base)
}

/// 0361's propagation: a projection keeps a key if it keeps the key's columns; a join along a key keeps the other
/// side's key; a filter keeps everything; an equality carries what it binds; a cross product keeps no key.
#[test]
fn a_rules_head_keeps_the_keys_its_body_determines() {
    // A projection keeping e's key, and one dropping it.
    assert_eq!(of(&rule(&[X], vec![atom(E, &[X, Y])])).keys, vec![0b1]);
    assert_eq!(of(&rule(&[Y], vec![atom(E, &[X, Y])])).keys, Vec::<u64>::new());
    // A join along f's key: x determines y, y determines z.
    assert_eq!(of(&rule(&[X, Z], vec![atom(E, &[X, Y]), atom(F, &[Y, Z])])).keys, vec![0b01]);
    // A cross product: neither side's key determines the other's columns.
    assert_eq!(of(&rule(&[X, Z], vec![atom(E, &[X, Term::Wild]), atom(G, &[Z])])).keys, Vec::<u64>::new());
    // Filters keep it, a negation binding nothing.
    let filtered = rule(&[X, Y], vec![atom(E, &[X, Y]), step(Kind::Cmp(Cmp::Lt), 0, &[X, Term::Const(3)]), step(Kind::Not, M, &[X])]);
    assert_eq!(of(&filtered).keys, vec![0b01]);
    // An equality: w is y.
    assert_eq!(of(&rule(&[X, W], vec![atom(E, &[X, Y]), step(Kind::Cmp(Cmp::Eq), 0, &[W, Y])])).keys, vec![0b01]);
    // A head column the body leaves free (a wildcard) determines nothing.
    assert!(of(&rule(&[X, Z], vec![atom(E, &[X, Y]), atom(M, &[Y, Z])])).keys.is_empty());
}

/// A fanout holds of a head column where one atom's row determines the head's row; a union and a recursion keep
/// only what they declare.
#[test]
fn a_fanout_carries_where_an_atoms_row_determines_the_heads_and_unions_keep_nothing() {
    let copy = rule(&[X, Y], vec![atom(E, &[X, Y])]);
    assert_eq!(of(&copy).caps, vec![(1, FEW)]);
    // Joined along f's key, e's row still determines the head's.
    let joined = rule(&[Y, Z], vec![atom(E, &[X, Y]), atom(F, &[Y, Z])]);
    assert_eq!(of(&joined).caps, vec![(0, FEW)]);
    // Joined to m, of no facts: e's row does not determine z.
    assert!(of(&rule(&[Y, Z], vec![atom(E, &[X, Y]), atom(M, &[Y, Z])])).caps.is_empty());
    // A union, and a recursion.
    let other = rule(&[X, Y], vec![atom(F, &[X, Y])]);
    assert!(infer(P, &[&copy, &other], false, &base).is_empty());
    assert!(infer(P, &[&copy], true, &base).is_empty());
}

/// The declarations' rows as facts: a key's columns by its number, a fanout's class; another class bounds nothing.
#[test]
fn declared_rows_give_each_relations_facts() {
    let (one, few, many) = (70, 71, 72);
    let keys = BTreeSet::from([vec![E, 0, 0], vec![E, 1, 1], vec![E, 1, 2], vec![F, 0, 0]]);
    let fanouts = BTreeSet::from([vec![E, 0, few], vec![F, 1, one], vec![G, 0, many]]);
    let s = declared(&keys, &fanouts, Some(one), Some(few));
    assert_eq!(s[&E], Facts { keys: vec![0b001, 0b110], caps: vec![(0, FEW)] });
    assert_eq!(s[&F], Facts { keys: vec![0b1], caps: vec![(1, 1.0)] });
    assert!(!s.contains_key(&G));
    assert_eq!(s[&E].cap(0b110), 1.0);
    assert_eq!(s[&E].cap(0b011), 1.0);
    assert_eq!(s[&E].cap(0b010), f64::INFINITY);
}
