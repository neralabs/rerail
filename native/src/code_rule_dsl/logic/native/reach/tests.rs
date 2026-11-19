//! The reach index's tests (decision 0119): the engine hosts them (`test-hosts.tsv`).
use super::Closed;
use crate::code_rule_dsl::facts::vtab::Source as _;

/// Every pair, by the definition: each start's seeds, then the edges walked to a fixpoint.
fn naive(seed: &[(i64, i64)], edges: &[(i64, i64)]) -> Vec<(i64, i64)> {
    let mut r: std::collections::BTreeSet<(i64, i64)> = seed.iter().copied().collect();
    loop {
        let more: Vec<(i64, i64)> = r.iter().flat_map(|&(x, y)| edges.iter().filter(move |e| e.0 == y).map(move |e| (x, e.1))).filter(|p| !r.contains(p)).collect();
        if more.is_empty() {
            return r.into_iter().collect();
        }
        r.extend(more);
    }
}

fn rows(c: &Closed) -> Vec<(i64, i64)> {
    (0..c.len()).map(|r| (c.value(r, 0), c.value(r, 1))).collect()
}

#[test]
fn the_index_holds_the_closure_and_answers_its_probes() {
    // A cycle (2, 3, 4), a chain out of it, a self loop, and starts with one seed and with two.
    let edges = [(1, 2), (2, 3), (3, 4), (4, 2), (4, 5), (5, 6), (7, 7), (8, 1)];
    let seed = [(10, 1), (11, 5), (12, 7), (13, 6), (13, 7), (14, 9)];
    let c = Closed::reach(&seed, &edges);
    let want = naive(&seed, &edges);
    assert_eq!(rows(&c), want);
    for (r, &(x, z)) in want.iter().enumerate() {
        assert_eq!(c.row_of(x, z), Some(r));
        assert_eq!(c.lookup_all(&[(0, x), (1, z)]), Some(vec![r]));
    }
    assert_eq!(c.row_of(11, 2), None);
    assert_eq!(c.lookup(0, &[13]), Some(want.iter().enumerate().filter(|(_, p)| p.0 == 13).map(|(r, _)| r).collect()));
    assert_eq!(c.lookup(1, &[6]), Some(want.iter().enumerate().filter(|(_, p)| p.1 == 6).map(|(r, _)| r).collect()));
    let stats = c.statistics().unwrap();
    assert_eq!((stats[0].1, stats[0].2), (want.len() as i64, 5));
}

#[test]
fn a_relation_through_the_closure_maps_what_each_start_reaches() {
    let edges = [(1, 2), (2, 3), (3, 1), (3, 4)];
    let seed = [(10, 1), (11, 4)];
    let map = [(2, 100), (4, 200), (4, 300)];
    let through = Closed::reach(&seed, &edges).through(&map);
    let reach = naive(&seed, &edges);
    let want: std::collections::BTreeSet<(i64, i64)> = reach.iter().flat_map(|&(x, e)| map.iter().filter(move |m| m.0 == e).map(move |m| (x, m.1))).collect();
    assert_eq!(rows(&through), want.into_iter().collect::<Vec<_>>());
    assert!(through.row_of(11, 100).is_none());
}

#[test]
fn the_shape_names_the_cycles_and_the_connected_parts_largest_first() {
    // Two parts: a cycle 1 2 3 with a tail to 4, and a self loop 7 joined to 8; 9 alone (a seed's node, no edge).
    let edges = [(1, 2), (2, 3), (3, 1), (3, 4), (7, 7), (7, 8)];
    let c = Closed::reach(&[(10, 1), (11, 7), (12, 9)], &edges);
    let s = c.shape();
    assert_eq!((s.nodes, s.edges, s.components, s.cyclic, s.self_loops), (7, 6, 5, 1, 1));
    assert_eq!(s.largest_cycles, vec![(3, vec![1, 2, 3])]);
    assert_eq!(s.connected, 3);
    assert_eq!(s.largest_connected, vec![(4, vec![1, 2, 3, 4]), (2, vec![7, 8]), (1, vec![9])]);
    assert_eq!(c.spread(), (3, 3, 4));
}

mod form {
    use crate::code_rule_dsl::logic::front::Native;
    use crate::code_rule_dsl::logic::reach_form::find;
    use crate::code_rule_dsl::logic::rr::parse_own;

    const CLOSURE: &str = ".decl e(a: number, b: number)\n.input e\n.decl own(a: number)\n.input own\n\
        .decl r(a: number, b: number){q}\nr(d, d) :- own(d).\nr(d, f) :- r(d, x), e(x, f).\n\
        .decl m(a: number, b: number)\n.input m\n.decl s(a: number, q: number)\ns(d, q) :- r(d, x), m(x, q).\n\
        .decl out(a: number, b: number)\n.output out\nout(a, b) :- s(a, b), r(a, b).\n";

    fn program(qualifier: &str, extra: &str) -> crate::code_rule_dsl::logic::front::Program {
        parse_own(&(CLOSURE.replace("{q}", qualifier) + extra)).unwrap()
    }

    #[test]
    fn a_closure_and_a_relation_through_it_are_found() {
        let mut prog = program("", "");
        assert_eq!(find(&mut prog).unwrap(), (1, 1));
        assert_eq!(prog.natives.get("r"), Some(&Native::Reach { seed: "rr_reach_seed_r".into(), edge: "e".into() }));
        assert_eq!(prog.natives.get("s"), Some(&Native::ReachThrough { reach: "r".into(), map: "m".into() }));
        // The seeds are their own relation's rules; `r`'s one rule orders it after them and the edges.
        assert_eq!(prog.rules.iter().filter(|r| r.head.rel == "rr_reach_seed_r").count(), 1);
        let own: Vec<_> = prog.rules.iter().filter(|r| r.head.rel == "r").collect();
        assert_eq!(own.len(), 1);
        assert!(!own[0].body.iter().any(|l| l.atom().is_some_and(|a| a.rel == "r")));
    }

    #[test]
    fn a_closure_inside_a_larger_cycle_is_left_as_rows() {
        // The seeds read `r` back through `back`: `r` is not its own component.
        let mut prog = program("", ".decl back(a: number)\nback(a) :- r(a, _).\nr(d, d) :- back(d).\n");
        assert_eq!(find(&mut prog).unwrap(), (0, 0));
        assert!(prog.natives.is_empty());
    }

    #[test]
    fn the_directive_asks_for_the_form_and_no_reach_refuses_it() {
        let mut prog = program(" no_reach", "");
        assert_eq!(find(&mut prog).unwrap(), (0, 0));
        let mut prog = program(" reach", "");
        assert_eq!(find(&mut prog).unwrap(), (1, 1));
        // A second recursive rule breaks the shape: declared `reach`, that is an error naming why.
        let mut prog = program(" reach", "r(d, f) :- r(x, d), e(x, f).\n");
        let err = format!("{:?}", find(&mut prog).unwrap_err());
        assert!(err.contains("more than one rule reads it"), "{err}");
    }
    /// A rewrite after `find` (magic sets' demand, decision 0145) can read a closure inside its edges' or seeds'
    /// component: then it is rows again, with its rules as written, and so is a relation through it. At O125 the
    /// demand put `anc` in a component of 44 relations, which had no maintained form and was derived at every edit.
    #[test]
    fn a_closure_a_later_rewrite_puts_in_a_cycle_is_rows_again() {
        let mut prog = program("", ".decl w(a: number)\n.input w\nr(d, d) :- w(d).\n");
        assert_eq!(find(&mut prog).unwrap(), (1, 1));
        assert!(crate::code_rule_dsl::logic::reach_form::undo_in_cycles(&mut prog).unwrap().is_empty(), "no cycle yet");
        // As a demand rewrite would: `w` is made from `r`, which a seed reads.
        let rule = parse_own(".decl r(a: number, b: number)\n.decl w(a: number)\nw(a) :- r(a, _).\n").unwrap().rules.remove(0);
        prog.rules.push(rule);
        prog.inputs.retain(|i| i != "w");
        let mut undone = crate::code_rule_dsl::logic::reach_form::undo_in_cycles(&mut prog).unwrap();
        undone.sort();
        assert_eq!(undone, vec!["r".to_owned(), "s".to_owned()]);
        assert!(prog.natives.is_empty());
        assert!(!prog.decls.contains_key("rr_reach_seed_r") && prog.rules.iter().all(|r| !r.to_string().contains("rr_reach_seed_r")));
        let own: Vec<String> = prog.rules.iter().filter(|r| r.head.rel == "r").map(|r| r.to_string()).collect();
        assert_eq!(own.len(), 3, "{own:#?}");
        assert!(own.iter().any(|r| r.contains(":- r(") && r.contains("e(")), "its recursive rule again: {own:#?}");
    }
}

