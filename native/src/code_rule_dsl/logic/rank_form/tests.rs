//! The rank form is found by the rules' form alone, whatever their relations are called.
use super::*;
use crate::code_rule_dsl::logic::rr::parse;

/// The reader's literal after the form, as text.
fn reader(prog: &Program, head: &str) -> String {
    prog.rules.iter().filter(|r| r.head.rel == head).map(|r| r.to_string()).collect::<Vec<_>>().join("\n")
}

/// A count of earlier rows of a group, over relations of any names: made by rank, its reader a sum over the rank.
#[test]
fn a_count_below_a_bound_is_a_rank_whatever_its_relations() {
    let mut p = parse(concat!(
        ".decl edge(p: number, c: number, i: number)\n.decl flag(n: number)\n.decl top(b: number)\n.input edge, flag, top\n",
        ".decl place(b: number, s: number, i: number)\n.output place\n",
        "place(b, s, i) :- top(b), edge(b, s, _), !flag(s), i = count : { edge(b, s2, _), !flag(s2), s2 < s }.\n",
    ))
    .unwrap();
    assert_eq!(find(&mut p), 1, "{}", reader(&p, "place"));
    let (rel, native) = p.natives.iter().find(|(_, n)| matches!(n, Native::Rank { .. })).expect("a rank");
    let Native::Rank { group, op, .. } = native else { unreachable!() };
    assert_eq!((*group, op.as_str()), (1, "<"), "grouped by b, counting below");
    let place = reader(&p, "place");
    assert!(place.contains(&format!("sum")) && place.contains(rel.as_str()), "{place}");
}

/// Written the other way round (`x > y`), and counting above (`y > x`): the op as the counted key sees it.
#[test]
fn a_rank_takes_its_op_from_the_counted_side() {
    for (cmp, want) in [("s > s2", "<"), ("s2 > s", ">"), ("s2 <= s", "<="), ("s >= s2", "<=")] {
        let mut p = parse(&format!(concat!(
            ".decl edge(p: number, c: number, i: number)\n.decl top(b: number)\n.input edge, top\n",
            ".decl place(b: number, s: number, i: number)\n.output place\n",
            "place(b, s, i) :- top(b), edge(b, s, _), i = count : {{ edge(b, s2, _), {} }}.\n",
        ), cmp))
        .unwrap();
        assert_eq!(find(&mut p), 1, "{cmp}");
        let op = p.natives.values().find_map(|n| if let Native::Rank { op, .. } = n { Some(op.clone()) } else { None });
        assert_eq!(op.as_deref(), Some(want), "{cmp}");
    }
}

/// The counted rows read an asked variable (`near(s2, s)`): it is in the group, so each asked row is a group of its
/// own, counted as exactly as before.
#[test]
fn a_count_tied_beyond_its_comparison_groups_by_what_ties_it() {
    let mut p = parse(concat!(
        ".decl edge(p: number, c: number, i: number)\n.decl near(a: number, b: number)\n.decl top(b: number)\n.input edge, near, top\n",
        ".decl place(b: number, s: number, i: number)\n.output place\n",
        "place(b, s, i) :- top(b), edge(b, s, _), i = count : { edge(b, s2, _), near(s2, s), s2 < s }.\n",
    ))
    .unwrap();
    assert_eq!(find(&mut p), 1, "{}", reader(&p, "place"));
    let group = p.natives.values().find_map(|n| if let Native::Rank { group, .. } = n { Some(*group) } else { None });
    assert_eq!(group, Some(2), "grouped by b and s");
}

/// Not the form: both sides of the comparison are the counted rows' own (`j < s2`), so no bound comes from the
/// asked rows.
#[test]
fn a_comparison_within_the_counted_rows_is_left_as_rows() {
    let mut p = parse(concat!(
        ".decl edge(p: number, c: number, i: number)\n.decl top(b: number)\n.input edge, top\n",
        ".decl place(b: number, i: number)\n.output place\n",
        "place(b, i) :- top(b), i = count : { edge(b, s2, j), j < s2 }.\n",
    ))
    .unwrap();
    assert_eq!(find(&mut p), 0, "{}", reader(&p, "place"));
    assert!(p.natives.values().all(|n| !matches!(n, Native::Rank { .. })));
}
