use super::*;
use crate::code_rule_dsl::logic::front;

const PROGRAM: &str = r#"
.decl base(c: node, b: node)
.decl bound_in(c: node, a: symbol)
.input base, bound_in
.decl found(c: node, a: symbol, at: node)
found(c, a, c) :- bound_in(c, a).
found(c, a, x) :- base(c, b), !bound_in(c, a), found(b, a, x).
.decl reach(c: node, x: node)
reach(c, x) :- found(c, _, x).
reach(c, y) :- reach(c, x), reach(x, y).
.output found, reach
"#;

/// The hop `found($b, …)` splits into a same-document rule and one reading the previous round; `reach`'s
/// recursive atoms are hops too (the second's node is `$x`), and the first is tied to the head.
#[test]
fn hops_read_the_previous_round_across_documents() {
    let prog = crate::code_rule_dsl::logic::rr::parse(PROGRAM).unwrap();
    let b = bound(&prog, &BTreeMap::new()).unwrap();
    assert_eq!(b.hops, BTreeSet::from(["found".to_owned(), "reach".to_owned()]));
    let rules: Vec<String> = b.program.rules.iter().map(ToString::to_string).collect();
    assert!(rules.contains(&"found($c, $a, $x) :- base($c, $b), !bound_in($c, $a), found($b, $a, $x), @doc($b) = @doc($c)".to_owned()), "{rules:#?}");
    assert!(rules.contains(&"found($c, $a, $x) :- base($c, $b), !bound_in($c, $a), found__prev($b, $a, $x), @doc($b) != @doc($c)".to_owned()), "{rules:#?}");
    assert!(rules.contains(&"reach($c, $y) :- reach($c, $x), reach__prev($x, $y), @doc($x) != @doc($c)".to_owned()), "{rules:#?}");
    // Non-recursive rules are unchanged; the split rules give one rule per choice.
    assert_eq!(b.program.rules.len(), 1 + 2 + 1 + 2);
    assert!(b.program.inputs.contains(&"found__prev".to_owned()));
    assert_eq!(b.program.decls["found__prev"], prog.decls["found"]);
    assert!(b.unbounded.is_empty());
    front::strata(&b.program).unwrap();
}

/// A recursive atom whose node a same-document atom joins to the head's is not a hop: `look` (file-local) puts `$s` in
/// `$n`'s document, so only the rule within the document is kept. `base` crosses files and still splits.
#[test]
fn same_document_joins_are_not_hops() {
    let prog = crate::code_rule_dsl::logic::rr::parse(
        r#"
.decl look(n: node, s: node)
.decl base(c: node, b: node)
.input look, base
.decl pv(n: node, v: node)
pv(n, n) :- base(n, _).
pv(n, v) :- look(n, s), pv(s, v).
pv(n, v) :- base(n, b), pv(b, v).
.output pv
"#,
    )
    .unwrap();
    let b = bound(&prog, &BTreeMap::from([("look".to_owned(), vec![0, 1])])).unwrap();
    let rules: Vec<String> = b.program.rules.iter().map(ToString::to_string).collect();
    assert!(rules.contains(&"pv($n, $v) :- look($n, $s), pv($s, $v)".to_owned()), "{rules:#?}");
    assert!(rules.contains(&"pv($n, $v) :- base($n, $b), pv__prev($b, $v), @doc($b) != @doc($n)".to_owned()), "{rules:#?}");
    assert_eq!(b.program.rules.len(), 1 + 1 + 2);
}

/// The rounds run the hops' cone (`found` and what it reads); `reach`, which no hop reads, runs once after them and
/// reads `found`'s final rows as `found__done`.
#[test]
fn the_rest_reads_the_cone_once() {
    let prog = crate::code_rule_dsl::logic::rr::parse(
        r#"
.decl base(c: node, b: node)
.decl bound_in(c: node, a: symbol)
.input base, bound_in
.decl found(c: node, a: symbol, at: node)
found(c, a, c) :- bound_in(c, a).
found(c, a, x) :- base(c, b), !bound_in(c, a), found(b, a, x).
.decl names(c: node, a: symbol)
names(c, a) :- found(c, a, _).
.output found, names
"#,
    )
    .unwrap();
    let b = bound(&prog, &BTreeMap::new()).unwrap();
    let split = cone_split(&b.program, &b.hops).expect("names is outside the cone");
    assert!(split.cone.rules.iter().all(|r| r.head.rel == "found"));
    assert_eq!(split.cone.outputs, vec!["found".to_owned()]);
    assert_eq!(split.reads, BTreeSet::from(["found".to_owned()]));
    let rules: Vec<String> = split.rest.rules.iter().map(ToString::to_string).collect();
    assert_eq!(rules, vec!["names($c, $a) :- found__done($c, $a, _)".to_owned()]);
    assert!(split.rest.inputs.contains(&"found__done".to_owned()));
    assert_eq!(split.rest.outputs, vec!["names".to_owned()]);
    // A program that is all cone is not split.
    let only = crate::code_rule_dsl::logic::rr::parse(PROGRAM).unwrap();
    let b = bound(&only, &BTreeMap::new()).unwrap();
    assert!(cone_split(&b.program, &b.hops).is_none());
}

/// What the rounds read and no round changes (`own`, from base relations only) runs once before them, and the
/// rounds read its rows as `own__pre`; `found`, which reads the previous round, stays in the rounds.
#[test]
fn what_no_round_changes_runs_before_the_rounds() {
    let prog = crate::code_rule_dsl::logic::rr::parse(
        r#"
.decl base(c: node, b: node)
.decl bound_in(c: node, a: symbol)
.input base, bound_in
.decl own(c: node, a: symbol)
own(c, a) :- bound_in(c, a), base(c, _).
.decl found(c: node, a: symbol, at: node)
found(c, a, c) :- own(c, a).
found(c, a, x) :- base(c, b), !own(c, a), found(b, a, x).
.output found
"#,
    )
    .unwrap();
    let b = bound(&prog, &BTreeMap::new()).unwrap();
    let ps = pre_split(&b.program).expect("own reads no previous round");
    assert!(ps.pre.rules.iter().all(|r| r.head.rel == "own"));
    assert_eq!(ps.reads, BTreeSet::from(["own".to_owned()]));
    assert!(ps.rounds.rules.iter().all(|r| r.head.rel == "found"));
    assert!(ps.rounds.inputs.contains(&"own__pre".to_owned()));
    let rules: Vec<String> = ps.rounds.rules.iter().map(ToString::to_string).collect();
    assert!(rules.iter().all(|r| !r.contains("own(")), "{rules:#?}");
    assert_eq!(ps.rounds.outputs, vec!["found".to_owned()]);
    // A program whose every relation reads the previous round is not split.
    let only = crate::code_rule_dsl::logic::rr::parse(PROGRAM).unwrap();
    let b = bound(&only, &BTreeMap::new()).unwrap();
    assert!(pre_split(&b.program).is_none());
}

#[test]
fn the_digest_ignores_row_order() {
    let a = BTreeMap::from([("r".to_owned(), vec![vec![1, 2], vec![3, 4]])]);
    let b = BTreeMap::from([("r".to_owned(), vec![vec![3, 4], vec![1, 2], vec![1, 2]])]);
    assert_eq!(digest(&a), digest(&b));
    assert_ne!(digest(&a), digest(&BTreeMap::from([("r".to_owned(), vec![vec![1, 2]])])));
}
