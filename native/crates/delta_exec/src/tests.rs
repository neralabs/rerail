//! The first cut's narrow tests (decision 0686).

use crate::db::{Db, Text};
use crate::kform::{Program, RELATIONS, Words};
use crate::run::{Exec, NoWorld};
use crate::sym::Symbols;
use crate::{Boot, Files, L0, k0, text};
use std::collections::BTreeSet;
use std::sync::Arc;

type Rows = BTreeSet<Vec<i64>>;

/// A program of modules (id, source), read by the K0 pass, run once over `facts` (relation, arity, rows): the
/// reference strategy's cold run.
fn cold(sym: &Symbols, modules: &[(i64, &str)], order: Option<&Rows>, facts: &[(&str, usize, &Rows)], heads: &[&str]) -> Vec<Rows> {
    let exec = Exec::new(sym);
    let texts: Vec<(i64, Arc<Text>)> = modules.iter().map(|(m, s)| (*m, text(s))).collect();
    let mut prog = Db::new();
    k0::pass(&texts.iter().map(|(m, t)| (*m, &**t)).collect::<Vec<_>>(), sym, &mut prog).expect("K0");
    let mut db = Db::new();
    for (n, a, rows) in facts {
        set(&mut db, sym, n, *a, rows);
    }
    if let Some(o) = order {
        set(&mut prog, sym, "k_order", 2, o);
    }
    exec.phase(&prog, &Words::new(sym), &mut db, &NoWorld).expect("runs");
    heads.iter().map(|h| sym.intern(h)).map(|h| if db.has(h) { db.rows(h, true) } else { Rows::new() }).collect()
}

fn set(db: &mut Db, sym: &Symbols, name: &str, arity: usize, rows: &Rows) {
    let n = sym.intern(name);
    db.declare(n, arity);
    db.write_set(n, &rows.iter().map(|r| r.clone().into_boxed_slice()).collect());
}

fn names(sym: &Symbols, rows: &Rows, col: usize) -> BTreeSet<String> {
    rows.iter().map(|r| sym.name(r[col])).collect()
}

/// One rule over a three-line `.rerail` tape: the views read where the tape lies.
#[test]
fn views_over_a_three_line_tape() {
    let sym = Symbols::new();
    let exec = Exec::new(&sym);
    let rules = text(
        "name(d, n) :- kind(d, c, \"clause\"), field(d, c, \"head\", h), field(d, h, \"relation\", r), text(d, r, n).\n\
         num(d, v) :- kind(d, t, \"number\"), value(d, t, v).\n\
         second(d, a) :- kind(d, c, \"clause\"), field(d, c, \"head\", h), field(d, h, \"arguments\", l), child(d, l, a, 1).\n",
    );
    let mut prog = Db::new();
    k0::pass(&[(1, &rules)], &sym, &mut prog).expect("K0");
    let mut db = Db::new();
    let d = db.give(sym.intern("three.rerail"), text("a(1, x).\n// a comment\nb(2) :- a(2, _).\nc(\"x\", 3).\n"));
    exec.phase(&prog, &Words::new(&sym), &mut db, &NoWorld).expect("runs");
    let rows = db.rows(sym.intern("name"), true);
    assert!(rows.iter().all(|r| r[0] == d));
    assert_eq!(names(&sym, &rows, 1), ["a", "b", "c"].map(String::from).into());
    let nums: BTreeSet<i64> = db.rows(sym.intern("num"), true).iter().map(|r| r[1]).collect();
    assert_eq!(nums, [1, 2, 3].into());
    // `child` counts named children that are no comment: the second argument of `a` and of `c`.
    let second = db.rows(sym.intern("second"), true);
    assert_eq!(second.len(), 2);
}

/// The K0 pass's rows for a five-line file, against rows written by hand (node ids read off the tape's dump).
#[test]
fn k0_pass_rows_by_hand() {
    let sym = Symbols::new();
    let src = "e(1, 2).\n\
               e(2, 3).\n\
               // a comment\n\
               p(x, z) :- e(x, y), e(y, z), !q(y), x != z.\n\
               q(\"s\") :- e(_, 3).\n";
    let t = text(src);
    let rows = k0::read(&t, 7, &sym).expect("K0");
    let r = crate::tape::Reads::of(&t);
    let mut dump = String::new();
    for i in 0..r.len() {
        let (k, f, x) = r.node(i);
        dump += &format!("{i}: {} {} {x} {:?}\n", rerail_rr_syntax::tape::KIND_NAMES.get(k as usize).unwrap_or(&"?"), rerail_rr_syntax::tape::FIELD_NAMES[f as usize], r.text(i));
    }
    let s = |x: &str| sym.intern(x);
    let got = |i: usize| -> Rows { rows[i].iter().map(|r| r.to_vec()).collect() };
    let want: [Rows; 7] = HAND(&s);
    for (i, w) in want.iter().enumerate() {
        assert_eq!(&got(i), w, "{}\n{dump}", RELATIONS[i].0);
    }
    assert!(got(7).is_empty());
}

/// The hand-written rows of `k0_pass_rows_by_hand`'s file, module 7.
#[allow(non_snake_case)]
fn HAND(s: &dyn Fn(&str) -> i64) -> [Rows; 7] {
    let m = 7;
    let (atom, not, cmp) = (s("atom"), s("not"), s("cmp"));
    [
        // k_rule
        [vec![m, 1, s("e")], vec![m, 7, s("e")], vec![m, 14, s("p")], vec![m, 40, s("q")]].into(),
        // k_step
        [
            vec![m, 14, 0, atom, s("e")],
            vec![m, 14, 1, atom, s("e")],
            vec![m, 14, 2, not, s("q")],
            vec![m, 14, 3, cmp, s("!=")],
            vec![m, 40, 0, atom, s("e")],
        ]
        .into(),
        // k_arg
        [
            vec![m, 1, -1, 0, 5],
            vec![m, 1, -1, 1, 6],
            vec![m, 7, -1, 0, 11],
            vec![m, 7, -1, 1, 12],
            vec![m, 14, -1, 0, 18],
            vec![m, 14, -1, 1, 19],
            vec![m, 14, 0, 0, 24],
            vec![m, 14, 0, 1, 25],
            vec![m, 14, 1, 0, 29],
            vec![m, 14, 1, 1, 30],
            vec![m, 14, 2, 0, 35],
            vec![m, 14, 3, 0, 37],
            vec![m, 14, 3, 1, 39],
            vec![m, 40, -1, 0, 44],
            vec![m, 40, 0, 0, 49],
            vec![m, 40, 0, 1, 50],
        ]
        .into(),
        // k_var
        [
            vec![m, 18, s("x")],
            vec![m, 19, s("z")],
            vec![m, 24, s("x")],
            vec![m, 25, s("y")],
            vec![m, 29, s("y")],
            vec![m, 30, s("z")],
            vec![m, 35, s("y")],
            vec![m, 37, s("x")],
            vec![m, 39, s("z")],
        ]
        .into(),
        // k_str
        [vec![m, 44, s("s")]].into(),
        // k_num
        [vec![m, 5, 1], vec![m, 6, 2], vec![m, 11, 2], vec![m, 12, 3], vec![m, 50, 3]].into(),
        // k_wild
        [vec![m, 49]].into(),
    ]
}

/// The K0 pass refuses what is not K0: an aggregate, a disjunction.
#[test]
fn k0_pass_refuses_an_aggregate() {
    let sym = Symbols::new();
    let t = text("n(c) :- c = count : { e(_, _) }.\n");
    let e = k0::read(&t, 1, &sym).expect_err("an aggregate is not K0");
    assert!(e.why.contains("not K0"), "{e}");
    let t = text("n(x) :- e(x, _) ; e(_, x).\n");
    assert!(k0::read(&t, 1, &sym).is_err());
}

/// A deterministic edit chain: each step's change run by the rules' terms, against a cold run of the same facts.
#[test]
fn delta_equals_reference_over_an_edit_chain() {
    let sym = Symbols::new();
    let exec = Exec::new(&sym);
    let two = "two(x, z) :- e(x, y), e(y, z), !b(y), x != z.\nloop(x) :- e(x, y), e(y, x).\nany(x) :- e(x, _).\n";
    let reach = "reach(x, y) :- e(x, y).\nreach(x, z) :- reach(x, y), e(y, z), !b(z).\n";
    let modules = [(1, two), (2, reach)];
    let heads = ["two", "loop", "any", "reach"];
    let texts: Vec<(i64, Arc<Text>)> = modules.iter().map(|(m, s)| (*m, text(s))).collect();
    let mut prog = Db::new();
    k0::pass(&texts.iter().map(|(m, t)| (*m, &**t)).collect::<Vec<_>>(), &sym, &mut prog).expect("K0");
    // Two strata: the non-recursive module first.
    let order: Rows = [vec![1, 0], vec![2, 1]].into();
    set(&mut prog, &sym, "k_order", 2, &order);
    let mut db = Db::new();
    let (mut e, mut b): (Rows, Rows) = (Rows::new(), Rows::new());
    let mut seed = 0x2545_f491_u64;
    let mut next = |n: u64| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (seed >> 33) % n
    };
    for step in 0..24 {
        if step > 0 {
            for _ in 0..1 + next(3) {
                let r = vec![next(6) as i64, next(6) as i64];
                if !e.remove(&r) {
                    e.insert(r);
                }
            }
            if next(3) == 0 {
                let r = vec![next(6) as i64];
                if !b.remove(&r) {
                    b.insert(r);
                }
            }
        }
        set(&mut db, &sym, "e", 2, &e);
        set(&mut db, &sym, "b", 1, &b);
        exec.phase(&prog, &Words::new(&sym), &mut db, &NoWorld).expect("runs");
        let want = cold(&sym, &modules, Some(&order), &[("e", 2, &e), ("b", 1, &b)], &heads);
        for (h, w) in heads.iter().zip(&want) {
            assert_eq!(&db.rows(sym.intern(h), true), w, "step {step}: {h}");
        }
        prog.settle();
        db.settle();
    }
    // The non-recursive module ran by its terms after the first step: its heads' counts are kept.
    assert!(db.counted.contains(&sym.intern("two")));
}

/// A program's change: an added rule runs, a removed rule's rows go, a rule added to a head adds its rows.
#[test]
fn program_delta() {
    let sym = Symbols::new();
    let exec = Exec::new(&sym);
    let e: Rows = [vec![1], vec![2], vec![3]].into();
    let f: Rows = [vec![7]].into();
    let turns = [
        "a(x) :- e(x).\nb(x) :- e(x), x != 2.\n",
        "a(x) :- e(x).\na(x) :- f(x).\nb(x) :- e(x), x != 2.\n",
        "a(x) :- e(x).\nc(x) :- e(x), x > 1.\n",
    ];
    let heads = ["a", "b", "c"];
    let mut prog = Db::new();
    let mut db = Db::new();
    for (i, src) in turns.iter().enumerate() {
        let t = text(src);
        k0::pass(&[(1, &t)], &sym, &mut prog).expect("K0");
        set(&mut db, &sym, "e", 1, &e);
        set(&mut db, &sym, "f", 1, &f);
        exec.phase(&prog, &Words::new(&sym), &mut db, &NoWorld).expect("runs");
        let want = cold(&sym, &[(1, src)], None, &[("e", 1, &e), ("f", 1, &f)], &heads);
        for (h, w) in heads.iter().zip(&want) {
            let n = sym.intern(h);
            let got = if db.has(n) { db.rows(n, true) } else { Rows::new() };
            assert_eq!(&got, w, "turn {i}: {h}");
        }
        prog.settle();
        db.settle();
    }
}

/// The loop's first turn: L1 → L2 → L3 → L4.
fn first_turn<'s>(sym: &'s Symbols, user: &str) -> Boot<'s> {
    let mut boot = Boot::new(sym);
    boot.turn(&text(L0), &[("user.rerail", text(user))], &Files::kernel()).expect("the turn runs");
    boot
}

/// The self-reading check: P2, run over `l0`'s own tape, gives P2. And each loaded kernel file's rows at L2 are the
/// K0 pass's rows of it.
#[test]
fn p2_reads_itself_and_the_kernel() {
    let sym = Symbols::new();
    let boot = first_turn(&sym, "p(1).\n");
    let mut total = 0;
    for (n, _) in &RELATIONS[..7] {
        let r = sym.intern(n);
        let at_l2: Rows = boot.l2.rows(r, true).into_iter().filter(|row| row[0] == 1).collect();
        assert_eq!(at_l2, boot.p2.rows(r, true), "{n}");
        total += at_l2.len();
    }
    assert!(total > 100, "P2 holds its rules");
    for (path, src) in crate::KERNEL {
        let d = boot.l2.doc_of(sym.intern(path)).unwrap_or_else(|| panic!("{path} loaded"));
        let want = k0::read(&text(src), d, &sym).expect("K0");
        for (i, (n, _)) in RELATIONS[..7].iter().enumerate() {
            let at_l2: Rows = boot.l2.rows(sym.intern(n), true).into_iter().filter(|row| row[0] == d).collect();
            let w: Rows = want[i].iter().map(|r| r.to_vec()).collect();
            assert_eq!(at_l2, w, "{path}: {n}");
        }
    }
    // The kernel's modules run in the order its loads give.
    let order: Rows = boot.l2.rows(sym.intern("k_order"), true);
    assert_eq!(order.iter().map(|r| r[1]).collect::<BTreeSet<_>>(), [1, 2, 3].into());
}

/// The first lowering: a rule with its filter first is refused as written, and once lowered gives the rows of the
/// rule written in order.
#[test]
fn first_lowering_places_filters() {
    let sym = Symbols::new();
    let user = "p(\"a\"). p(\"b\"). p(\"c\").\nq(\"a\", 1). q(\"c\", 2).\n\
                ok(x, n) :- p(x), q(x, n), x != \"b\", n > 1.\n\
                low(x, n) :- n > 1, x != \"b\", p(x), q(x, n).\n";
    let boot = first_turn(&sym, user);
    // As read, `low` cannot run: its filters come before their variables are bound.
    let e = Program::read(&boot.l3, true, &boot.k, &sym).err().expect("refused as written");
    assert!(e.why.contains("not bound"), "{e}");
    let ok = boot.l4.rows(sym.intern("ok"), true);
    let low = boot.l4.rows(sym.intern("low"), true);
    assert_eq!(ok.len(), 1);
    assert_eq!(ok, low);
    assert_eq!(names(&sym, &low, 0), ["c".to_string()].into());
}

/// The loop's next turn: the user's file edited, each level run across its input's change.
#[test]
fn next_turn_follows_an_edit() {
    let sym = Symbols::new();
    let mut boot = first_turn(&sym, "p(\"a\").\nr(x) :- x != \"z\", p(x).\n");
    boot.settle();
    boot.turn(&text(L0), &[("user.rerail", text("p(\"a\"). p(\"z\"). p(\"y\").\nr(x) :- x != \"z\", p(x).\n"))], &Files::kernel()).expect("runs");
    assert_eq!(names(&sym, &boot.l4.rows(sym.intern("r"), true), 0), ["a", "y"].map(String::from).into());
}
