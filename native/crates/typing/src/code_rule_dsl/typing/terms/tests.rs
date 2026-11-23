use super::*;

// Name atoms: any numbers stand for the interner's.
const INT: Sym = 1;
const STR: Sym = 2;
const LIST: Sym = 3;
const DICT: Sym = 4;
const T: Sym = 10;
const U: Sym = 11;
const HASHABLE: u32 = 0;
const AWAITABLE: u32 = 1;

#[test]
fn equal_terms_are_one_term() {
    let mut t = Terms::default();
    let a = t.named(INT, &[]);
    let la = t.named(LIST, &[a]);
    let b = t.named(INT, &[]);
    assert_eq!(a, b);
    assert_eq!(la, t.named(LIST, &[b]));
    let top = t.top();
    assert_ne!(la, t.named(LIST, &[top]));
}

#[test]
fn unions_flatten_deduplicate_and_widen() {
    let mut t = Terms::default();
    let (i, s) = (t.named(INT, &[]), t.named(STR, &[]));
    let is = t.union(&[i, s], false);
    assert_eq!(is, t.union(&[s, i, i], false), "order and repeats do not matter");
    assert_eq!(t.union(&[is, i], false), is, "nested unions flatten");
    assert_eq!(t.union(&[i], false), i, "one closed member is itself");
    assert_ne!(t.union(&[i], true), i, "an open union keeps its flag");
    let open_i = t.union(&[i], true);
    let opened = t.union(&[is, open_i], false);
    assert_eq!(t.get(opened).ctor, Ctor::OpenUnion, "an open member opens it");
    let top = t.top();
    assert_eq!(t.union(&[i, top], false), top, "⊤ absorbs");
    let many: Vec<Tid> = (100..100 + MEMBERS as Sym + 1).map(|n| t.named(n, &[])).collect();
    assert_eq!(t.union(&many, false), top, "past the member limit");
}

#[test]
fn deep_terms_widen_to_top() {
    let mut t = Terms::default();
    let mut x = t.named(INT, &[]);
    for _ in 0..10 {
        x = t.named(LIST, &[x]);
    }
    assert_eq!(t.depth(x), DEPTH);
    // list[list[list[⊤]]]
    let mut y = t.top();
    for _ in 0..DEPTH - 1 {
        y = t.named(LIST, &[y]);
    }
    assert_eq!(x, y);
}

#[test]
fn proven_and_possible_are_three_valued() {
    let mut t = Terms::default();
    let (i, s) = (t.named(INT, &[]), t.named(STR, &[]));
    t.declare_traits(Ctor::Named, INT, 1 << HASHABLE, 1 << HASHABLE);
    t.declare_traits(Ctor::Named, STR, 1 << HASHABLE, 1 << HASHABLE);
    let l = t.named(LIST, &[i]);
    t.declare_traits(Ctor::Named, LIST, 0, 0);
    assert_eq!(t.check(i, HASHABLE), Truth::True);
    assert_eq!(t.check(l, HASHABLE), Truth::False);
    assert_eq!(t.check(i, AWAITABLE), Truth::False);
    let is = t.union(&[i, s], false);
    assert!(t.check(is, HASHABLE).proven());
    let il = t.union(&[i, l], false);
    assert_eq!(t.check(il, HASHABLE), Truth::Unknown, "one member may not be");
    let open = t.union(&[i, s], true);
    assert_eq!(t.check(open, HASHABLE), Truth::Unknown, "an open union proves nothing");
    assert!(t.check(open, AWAITABLE).possible());
    assert_eq!(t.check(t.top(), HASHABLE), Truth::Unknown);
    // A variable has its bound's traits (bounds are must-traits).
    let v = t.var(T, Some(i));
    assert!(t.check(v, HASHABLE).proven());
    // some / every over members.
    let hashable = |m: Tid| t.check(m, HASHABLE);
    assert_eq!(t.every(is, &hashable), Truth::True);
    assert_eq!(t.every(il, &hashable), Truth::False);
    assert_eq!(t.some(il, &hashable), Truth::True);
    assert_eq!(t.every(open, &hashable), Truth::Unknown, "more members may exist");
    let open_l = t.union(&[l], true);
    let hashable = |m: Tid| t.check(m, HASHABLE);
    assert_eq!(t.some(open_l, &hashable), Truth::Unknown);
}

#[test]
fn instantiation_follows_the_order() {
    let mut t = Terms::default();
    let (i, s) = (t.named(INT, &[]), t.named(STR, &[]));
    let (vt, vu) = (t.var(T, None), t.var(U, None));
    let list_t = t.named(LIST, &[vt]);
    // 1. Receiver: list[T].pop(self) -> T on list[int].
    let pop = t.callable(&[list_t], vt);
    let list_i = t.named(LIST, &[i]);
    assert_eq!(t.instantiate(pop, Some(list_i), &[], &[]), i);
    // 2. Explicit type arguments: cast[T](x: object) -> T with [str].
    let obj = t.top();
    let cast = t.callable(&[obj], vt);
    assert_eq!(t.instantiate(cast, None, &[s], &[i]), s);
    // 3. One-way matching: first(xs: list[T]) -> T on list[str].
    let first = t.callable(&[list_t], vt);
    let list_s = t.named(LIST, &[s]);
    assert_eq!(t.instantiate(first, None, &[], &[list_s]), s);
    // No unification: a mismatch binds nothing, and T falls back to its bound (⊤).
    let d = t.named(DICT, &[i, s]);
    assert_eq!(t.instantiate(first, None, &[], &[d]), t.top());
    // 4. One callback step: map(f: (T) -> U, xs: list[T]) -> list[U] with f: (int) -> str.
    let f = t.callable(&[vt], vu);
    let list_u = t.named(LIST, &[vu]);
    let map = t.callable(&[f, list_t], list_u);
    let int_to_str = t.callable(&[i], s);
    assert_eq!(t.instantiate(map, None, &[], &[int_to_str, list_i]), list_s);
    // 5. Unbound variables fall back to their bounds.
    let hashable_t = t.var(T, Some(i));
    let make = t.callable(&[], hashable_t);
    assert_eq!(t.instantiate(make, None, &[], &[]), i);
}

#[test]
fn terms_are_written_as_type_term_and_type_arg() {
    let mut t = Terms::default();
    let i = t.named(INT, &[]);
    let l = t.named(LIST, &[i]);
    let db = rusqlite::Connection::open_in_memory().unwrap();
    t.write(&db, "main").unwrap();
    let n: i64 = db.query_row("SELECT count(*) FROM type_term", [], |r| r.get(0)).unwrap();
    assert_eq!(n as usize, t.len());
    let (ctor, name, arity): (i64, i64, i64) =
        db.query_row("SELECT ctor, name_atom, arity FROM type_term WHERE tid = ?1", [l], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
    assert_eq!((ctor, name, arity), (Ctor::Named as i64, i64::from(LIST), 1));
    let arg: i64 = db.query_row("SELECT arg FROM type_arg WHERE tid = ?1 AND i = 0", [l], |r| r.get(0)).unwrap();
    assert_eq!(arg, i64::from(i));
}
