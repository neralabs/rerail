//! The simplifier's passes (decision 0038) on versions: each program read from `.rr` text, run through `passes`, and
//! its rules compared as `show_rule` prints them.

use super::{Report, passes};
use crate::code_rule_dsl::logic::program_image::Version;
use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
use crate::code_rule_dsl::logic::program_image::manager::Pipeline;
use std::sync::Arc;

/// `text`'s version simplified, and what the passes did.
fn simplified(text: &str) -> (Report, Arc<Version>) {
    let v = crate::code_rule_dsl::logic::rr::parse_version(text).unwrap();
    let mut p = Pipeline::new("test", v);
    let report = passes(&mut p).unwrap();
    (report, p.finish().unwrap().0)
}

/// Every rule of `v`, a line each.
fn rules(v: &Version) -> Vec<String> {
    v.rules().map(|r| v.show_rule(r)).collect()
}

/// The rules of `v` whose head is `head`.
fn rules_of(v: &Version, head: &str) -> Vec<String> {
    v.rules().filter(|r| v.rel_name(r.head().rel()) == head).map(|r| v.show_rule(r)).collect()
}

/// The greatest congruence merges recursive relations equal only to each other, and the relations over them: `p1` and
/// `p2` are one, so `q1` and `q2` (each with `!p3`), and the counts over them; `p3` reads `f`, not `e`, and `q3`
/// negates `p1`, so both stay.
#[test]
fn recursive_relations_equal_to_each_other_merge_with_their_readers() {
    let (report, v) = simplified(
        r#"
        .decl e(a: number, b: number)
        .input e
        .decl f(a: number, b: number)
        .input f
        .decl p1(a: number, b: number)
        .decl p2(a: number, b: number)
        .decl p3(a: number, b: number)
        .decl q1(a: number)
        .decl q2(a: number)
        .decl q3(a: number)
        .decl c1(a: number, n: number)
        .decl c2(a: number, n: number)
        .decl out(a: number)
        .output out
        p1(x, y) :- e(x, y).
        p1(x, z) :- p1(x, y), e(y, z).
        p2(u, v) :- e(u, v).
        p2(u, w) :- p2(u, v), e(v, w).
        p3(x, y) :- f(x, y).
        p3(x, z) :- p3(x, y), f(y, z).
        q1(x) :- p1(x, _), !p3(x, x).
        q2(x) :- p2(x, _), !p3(x, x).
        q3(x) :- p3(x, _), !p1(x, x).
        c1(a, n) :- e(a, _), n = count : { p1(a, _) }.
        c2(a, n) :- e(a, _), n = count : { p2(a, _) }.
        out(x) :- q1(x), q2(x), q3(x), c1(x, _), c2(x, _).
        "#,
    );
    assert_eq!(report.merged, 3, "{:?}", rules(&v));
    for pair in [["p1", "p2"], ["q1", "q2"], ["c1", "c2"]] {
        assert_eq!(pair.iter().filter(|r| v.declares(r)).count(), 1, "{pair:?} one relation: {:?}", rules(&v));
    }
    assert!(v.declares("p3") && v.declares("q3"), "{:?}", rules(&v));
}

#[test]
fn implied_atoms_kind_tests_and_identical_relations_go() {
    let (report, v) = simplified(
        r#"
        .decl node(d: number, n: node, kind: symbol, start: number, end: number)
        .decl doc(d: number, path: symbol, module: symbol)
        .input node
        .input doc
        .decl k0(kind: symbol)
        k0("function_definition").
        .decl b1(l: node, h: node)
        .decl b2(l: node, h: node)
        .decl m1(n: node)
        .decl m2(n: node)
        .decl out(n: node, p: symbol)
        .output out
        b1(low, high) :- inside(low, s, k0), node(_, s, "function_definition", _, _), inside(s, high, k0).
        b2(x, y) :- inside(x, z, k0), node(_, z, "function_definition", _, _), inside(z, y, k0).
        m1(n) :- inside(n, v, k0), !b1(n, v).
        m2(q) :- inside(q, w, k0), !b2(q, w).
        out(n, p) :- node(d, n, "call", _, _), !m1(n), !m2(n), doc(d, p, m), doc(d, p, _).
        "#,
    );
    // The second `!m1($n)` the merge made. `doc($d, $p, _)` beside `doc($d, $p, $m)` is one row of `doc` (its key
    // `$d`): merged, and `$m`, named once, is `_`.
    assert_eq!(report.implied, 1);
    assert_eq!(report.keyed, 1);
    // The kind test on a member of k0 = {function_definition}, in b1 and b2.
    assert_eq!(report.kind_tests, 2);
    // b2 merges into b1, then m2 (now equal to m1) into m1.
    assert_eq!(report.merged, 2);
    let out = rules_of(&v, "out");
    assert_eq!(out.len(), 1);
    assert!(out[0].contains("!m1($n), doc($d, $p, _)") && !out[0].contains("m2") && out[0].matches("doc(").count() == 1, "{}", out[0]);
    assert!(!v.declares("b2") && !v.declares("m2"));
}

/// A relation nothing reads keeps its name, though it equals a read one: the engine may read it by name (`env`'s
/// settings). A class is named after its shortest member: a copy's name is its relation's with a prefix.
#[test]
fn unread_relations_stay_and_a_class_takes_its_shortest_name() {
    let (report, v) = simplified(
        r#"
        .decl base(n: number)
        .input base
        .decl env(k: symbol, v: symbol)
        env("load.budget", "9").
        .decl cfg(k: symbol, v: symbol)
        cfg("load.budget", "9").
        .decl q(n: number)
        .decl rr_wide_node_text_q(n: number)
        q(n) :- base(n).
        rr_wide_node_text_q(n) :- base(n).
        .decl out(n: number, v: symbol)
        .output out
        out(n, v) :- q(n), rr_wide_node_text_q(n), cfg(_, v).
        "#,
    );
    assert_eq!(report.merged, 1);
    assert!(!v.declares("rr_wide_node_text_q"), "{:?}", rules(&v));
    assert!(v.states_facts("env") && v.states_facts("cfg"), "{:?}", v.fact_names());
    // The class keeps one member's rules: its namesake's, not its first's as well.
    assert_eq!(rules_of(&v, "q").len(), 1, "{:?}", rules(&v));
}

/// A relation nothing reads joins the class of its own widened copy and names it (all its readers read the copy);
/// one whose name only ends like a copy's relation (`same_q` against `rr_wide_node_text_cmp_same_q`) stays apart.
#[test]
fn an_unread_relation_names_its_own_copys_class_only() {
    let (report, v) = simplified(
        r#"
        .decl base(n: number)
        .input base
        .decl q(n: number)
        .decl rr_wide_node_text_q(n: number)
        q(n) :- base(n).
        rr_wide_node_text_q(n) :- base(n).
        .decl other(n: number)
        .input other
        .decl same_q(n: number)
        .decl cmp_same_q(n: number)
        .decl rr_wide_node_text_cmp_same_q(n: number)
        same_q(n) :- other(n).
        cmp_same_q(n) :- other(n).
        rr_wide_node_text_cmp_same_q(n) :- other(n).
        .decl out(n: number)
        .output out
        out(n) :- rr_wide_node_text_q(n), rr_wide_node_text_cmp_same_q(n), cmp_same_q(n).
        "#,
    );
    assert_eq!(report.merged, 2, "{:?}", rules(&v));
    assert!(!v.declares("rr_wide_node_text_q") && !v.declares("rr_wide_node_text_cmp_same_q"), "{:?}", rules(&v));
    // `same_q` neither joined nor named the class: the reached rule reads `cmp_same_q` still. (Its own rules go with
    // the other unread relations' rules, the dead-rule pass.)
    let out = &rules_of(&v, "out")[0];
    assert!(out.contains("cmp_same_q(") && !out.contains(" same_q("), "{out}");
}

/// Two atoms of one keyed row merge by renaming the later's variable, in the head's computed terms too: the head
/// `@lower($t)` reads the kept atom's variable, not a `$t` no atom binds any more.
#[test]
fn a_merged_variable_is_renamed_inside_the_head() {
    let (report, v) = simplified(
        r#"
        .decl text(n: node, s: symbol)
        .input text
        .decl rn(n: node, s: symbol)
        .output rn
        rn(n, @lower(t)) :- text(n, u), text(n, t).
        "#,
    );
    assert_eq!(report.keyed, 1, "{:?}", rules(&v));
    let rn = &rules_of(&v, "rn")[0];
    assert_eq!(rn.matches("text(").count(), 1, "{rn}");
    let kept = regex::Regex::new(r"text\(\$n, \$(\w+)\)").unwrap().captures(rn).map(|c| c[1].to_owned()).unwrap_or_else(|| panic!("{rn}"));
    assert!(rn.contains(&format!("@lower(${kept})")), "the head reads the kept atom's variable: {rn}");
}

/// Two programs' copies of a recursive library relation (each reads its own copy) are one relation in the shared
/// namespace, with their equal facts; the programs' own rules, which differ, keep their namespaces.
#[test]
fn recursive_copies_merge_into_the_shared_namespace() {
    // Unprofiled: held so no profile test turns the (process-wide) profile on meanwhile.
    let _run = crate::code_rule_dsl::logic::profile::one_run();
    let (report, v) = simplified(
        r#"
        .decl edge(a: number, b: number)
        .input edge
        .decl p0_seed(a: number)
        p0_seed(1).
        .decl p1_seed(a: number)
        p1_seed(1).
        .decl p0_reach(a: number)
        .decl p1_reach(a: number)
        p0_reach(a) :- p0_seed(a).
        p0_reach(b) :- p0_reach(a), edge(a, b).
        p1_reach(a) :- p1_seed(a).
        p1_reach(b) :- p1_reach(a), edge(a, b).
        .decl p0_mine(a: number)
        .decl p1_mine(a: number)
        p0_mine(a) :- p0_reach(a), a > 2.
        p1_mine(a) :- p1_reach(a), a > 3.
        .decl out(a: number)
        .output out
        out(a) :- p0_mine(a).
        out(a) :- p1_mine(a).
        "#,
    );
    assert_eq!(report.merged, 4);
    assert_eq!(report.shared, 2);
    assert!(["p0_reach", "p1_reach", "p0_seed", "p1_seed"].iter().all(|r| !v.declares(r)), "{:?}", rules(&v));
    assert_eq!(rules_of(&v, "s_reach").len(), 2);
    assert_eq!(v.fact_rows("s_seed").len(), 1);
    assert!(v.declares("p0_mine") && v.declares("p1_mine"));
    // p0_mine and p1_mine differ only in their constant: reported in a profiled run only (below).
    assert_eq!((report.similar.0, report.similar.1), (0, 0));
}

/// Relations equal up to their constants are a profile's report (`relations equal up to their constants`), not a
/// part of the merge: an unprofiled simplify makes no second congruence for them (it was about half of each merge's
/// cost in a pack, decision 0168), and a profiled one reports them as before.
#[test]
fn relations_alike_but_for_their_constants_are_found_only_when_profiled() {
    let text = r#"
        .decl edge(a: number, b: number)
        .input edge
        .decl p0_reach(a: number)
        p0_reach(b) :- edge(1, b).
        .decl p1_reach(a: number)
        p1_reach(b) :- edge(1, b).
        .decl p0_mine(a: number)
        p0_mine(b) :- edge(2, b), p0_reach(b).
        .decl p1_mine(a: number)
        p1_mine(b) :- edge(3, b), p1_reach(b).
        .output p0_mine, p1_mine
    "#;
    // Held from the unprofiled run on: no other profile test turns the (process-wide) profile on meanwhile.
    let _run = crate::code_rule_dsl::logic::profile::one_run();
    let (quiet, _) = simplified(text);
    assert_eq!(quiet.merged, 2);
    assert_eq!((quiet.similar.0, quiet.similar.1), (0, 0));
    crate::code_rule_dsl::logic::profile::set(true);
    let (profiled, _) = simplified(text);
    crate::code_rule_dsl::logic::profile::set(false);
    assert_eq!(profiled.merged, quiet.merged);
    assert_eq!((profiled.similar.0, profiled.similar.1), (2, 1));
}

/// The converter's `has` with no stop: a node holding a match among its descendants, climbed one `child` edge a
/// wave. Made `within`: the matches, and the nodes they lie within.
#[test]
fn a_climb_through_child_is_the_within_axis() {
    let (report, v) = simplified(
        r#"
        .decl node(d: number, n: node, kind: symbol, start: number, end: number)
        .input node
        .decl child(p: node, c: node, i: number)
        .input child
        .decl m23(n: node)
        .decl m25(n: node)
        .decl u19(n: node)
        .decl out(n: node)
        .output out
        m23(n) :- node(_, n, "identifier", _, _).
        m25(n) :- m23(n).
        m25(n) :- child(n, v24, _), u19(v24).
        u19(n) :- m25(n).
        out(n) :- m25(n), node(_, n, "call", _, _).
        "#,
    );
    assert_eq!(report.climbs, vec!["m25/u19".to_owned()]);
    let climbed: Vec<String> = rules_of(&v, "m25").into_iter().chain(rules_of(&v, "u19")).collect();
    assert!(climbed.iter().all(|r| !r.contains("child(")), "{climbed:?}");
    assert!(climbed.iter().any(|r| r.contains("within($n, $n_axis)")), "{climbed:?}");
    assert!(v.input_names().iter().any(|i| i == "within"));
}

/// A step with a test of the node it reaches is no plain closure: left as it is.
#[test]
fn a_climb_that_tests_each_step_stays_recursive() {
    let (report, _) = simplified(
        r#"
        .decl node(d: number, n: node, kind: symbol, start: number, end: number)
        .input node
        .decl child(p: node, c: node, i: number)
        .input child
        .decl r(n: node)
        .decl out(n: node)
        .output out
        r(n) :- node(_, n, "identifier", _, _).
        r(n) :- child(n, c, _), r(c), node(_, n, "attribute", _, _).
        out(n) :- r(n).
        "#,
    );
    assert!(report.climbs.is_empty());
}

#[test]
fn a_variable_equal_to_a_constant_is_the_constant() {
    let (report, v) = simplified(
        r#"
        .decl node(d: number, n: node, kind: symbol, start: number, end: number)
        .decl text(n: node, s: symbol)
        .input node
        .input text
        .decl m(n: node)
        .output m
        m(n) :- node(_, n, "identifier", _, _), text(n, v), v = "get".
        m(n) :- node(_, n, "identifier", _, _), text(n, v), "set" = v.
        .decl t(s: symbol)
        .decl doc(d: number, path: symbol, module: symbol)
        .input doc
        .output t
        t(s) :- doc(_, s, _), s = "a.py".
        "#,
    );
    assert_eq!(report.constants, 2);
    let m = rules_of(&v, "m");
    assert!(m.iter().any(|r| r.contains(r#"text($n, "get")"#) && !r.contains("$v")), "{m:?}");
    assert!(m.iter().any(|r| r.contains(r#"text($n, "set")"#) && !r.contains("$v")), "{m:?}");
    // A head variable stays: the head is filled from a text column.
    let t = &rules_of(&v, "t")[0];
    assert!(t.contains(r#"doc(_, $s, _), $s = "a.py""#), "{t}");
}

#[test]
fn singletons_keyed_rows_and_unread_relations() {
    let (report, v) = simplified(
        r#"
        .decl node(d: number, n: node, kind: symbol, start: number, end: number)
        .decl text(n: node, s: symbol)
        .input node
        .input text
        .decl named(n: node)
        .output named
        named(n) :- node(d, n, _, _, _), node(_, n, "identifier", s, _), text(n, v), text(w, v), s > 3.
        .decl two(n: node, m: node)
        .output two
        two(n, m) :- node(_, n, "call", _, _), node(_, m, "call", _, _), node(_, n, _, a, _), node(_, m, _, a, _).
        .decl dead(n: node)
        .decl deader(n: node)
        dead(n) :- node(_, n, "call", _, _), !deader(n).
        deader(n) :- node(_, n, "name", _, _).
        "#,
    );
    // One row of `node` for `$n` (its key), `$d` named once; `text($w, $v)`, `$w` named once, is implied.
    let named = &rules_of(&v, "named")[0];
    assert!(named.contains(r#"node(_, $n, "identifier", $s, _)"#) && named.matches("node(").count() == 1, "{named}");
    assert!(named.matches("text(").count() == 1, "{named}");
    // Atoms with different keys stay apart: `$n` and `$m` are two nodes with one start (each its two atoms merged).
    let two = &rules_of(&v, "two")[0];
    assert_eq!(two.matches("node(").count(), 2, "{two}");
    // `dead`, a test of one node that nothing reads, is inlined into its (no) readers; `deader`, a filter (a node
    // lookup and a test), is inlined into `dead` first.
    assert!(report.unread.is_empty() && report.inlined.contains(&"deader".to_owned()), "{report:?}");
    assert!(rules_of(&v, "dead").is_empty() && rules_of(&v, "deader").is_empty(), "{:?}", rules(&v));
}

#[test]
fn a_part_meeting_the_rest_only_at_a_document_is_a_relation_of_documents() {
    let (report, v) = simplified(
        r#"
        .decl node(d: number, n: node, kind: symbol, start: number, end: number)
        .decl name(n: node, s: symbol)
        .input node
        .input name
        .decl cached(n: node)
        .output cached
        cached(n) :- node(_, n, "module", _, _), node(d, n, _, _, _), node(d, y, _, _, _), name(y, s), contains("cache", s).
        cached(n) :- node(_, n, "class", _, _), node(e, n, _, _, _), node(e, z, _, _, _), name(z, t), contains("cache", t).
        .decl near(n: node)
        .output near
        near(n) :- node(d, n, "call", _, _), node(d, y, _, _, _), name(y, s), name(n, s).
        "#,
    );
    // One relation: the second rule's part is the first's once renamed, so it reads that one; nothing to merge.
    assert_eq!(report.existentials, vec!["exists_0".to_owned()]);
    assert_eq!(report.merged, 0);
    let exists = rules_of(&v, "exists_0");
    assert_eq!(exists.len(), 1);
    assert!(exists[0].contains("name(") && exists[0].contains("contains") && !exists[0].contains("module"), "{}", exists[0]);
    for cached in rules_of(&v, "cached") {
        assert!(cached.contains("exists_0(") && !cached.contains("name("), "{cached}");
    }
    // `$s` joins the part to the head's node: it stays.
    assert!(rules_of(&v, "near")[0].contains("name($y, $s)"), "{:?}", rules_of(&v, "near"));
}

#[test]
fn two_nodes_of_a_document_compared_by_span_are_within() {
    let (report, v) = simplified(
        r#"
        .decl node(d: number, n: node, kind: symbol, start: number, end: number)
        .input node
        .decl inside(n: node, b: node)
        .output inside
        inside(n, b) :- node(d, n, "call", s, e), node(d, b, "try_statement", bs, be), bs <= s, be >= e.
        .decl other(n: node)
        .output other
        other(n) :- node(d, n, "call", s, e), node(d2, b, "try_statement", bs, be), bs <= s, e <= be.
        "#,
    );
    assert_eq!(report.within, 1);
    assert!(v.input_names().iter().any(|i| i == "span_within") && v.declares("span_within"));
    let inside = &rules_of(&v, "inside")[0];
    assert!(inside.contains("span_within($n, $b)") && !inside.contains("<="), "{inside}");
    // Two documents: not a containment of one document's nodes.
    assert!(!rules_of(&v, "other")[0].contains("within"), "{:?}", rules_of(&v, "other"));
}

/// eager-resolve's `er_wrapped`: tests of derived relations on the head's variables and a base lookup, with nothing
/// joining f to v. Built whole it is every def with every call; inlined, its reader tests its own pair.
#[test]
fn a_filter_may_test_derived_relations_of_its_head() {
    let (report, v) = simplified(
        r#"
        .decl node(d: number, n: node, kind: symbol, start: number, end: number)
        .decl child(p: node, c: node, i: number)
        .input node, child
        .decl sym(f: node, s: symbol)
        sym(f, "x") :- node(_, f, "function_definition", _, _).
        .decl async(f: node)
        async(f) :- child(f, a, 0), node(_, a, "async", _, _).
        .decl wrapped(f: node, v: node, c: node)
        wrapped(f, v, v) :- sym(f, _), !async(f), node(_, v, "call", _, _).
        .decl alias(f: node)
        alias(f) :- sym(f, _).
        .decl out(f: node)
        .output out
        out(f) :- sym(f, _), child(f, v, 0), wrapped(f, v, c), alias(f).
        "#,
    );
    assert!(report.inlined.iter().any(|r| r == "wrapped"), "{:?}", report.inlined);
    assert!(!report.inlined.iter().any(|r| r == "alias"), "{:?}", report.inlined);
    let out = rules_of(&v, "out");
    assert!(out.iter().all(|r| !r.contains("wrapped(") && r.contains("!async(")), "{out:?}");
}

#[test]
fn a_computed_view_is_inlined_where_its_reader_reads_its_domain() {
    let view = r#"
        .decl split_line(d: number, l: number, t: symbol)
        .input split_line
        .decl picked(d: number)
        .input picked
        .decl trim(raw: symbol, s: symbol)
        trim(raw, @replace_all(raw, "^ +", "")) :- split_line(_, _, raw).
        .decl line(d: number, l: number, s: symbol)
        .output line
        line(d, l, s) :- picked(d), split_line(d, l, raw), trim(raw, s).
    "#;
    // `other` does not read `trim`'s domain: inlined there, it would read every line; so `trim` stays built.
    let (report, _) = simplified(&format!("{view}\n.decl other(s: symbol)\n.output other\nother(s) :- picked(r), trim(r, s).\n"));
    assert!(!report.inlined.contains(&"trim".to_owned()), "{report:?}");
    let (report, v) = simplified(view);
    assert!(report.inlined.contains(&"trim".to_owned()), "{report:?}");
    // The function of the reader's own line; the domain's second `split_line` read is implied and dropped.
    assert_eq!(rules(&v), vec![r#"line($d, $l, $s) :- picked($d), split_line($d, $l, $raw), $s = @replace_all($raw, "^ +", "")"#]);
    // The reader's own `$raw` (here its result, not the definition's `raw`) stays the reader's.
    let (report, v) = simplified(
        r#"
        .decl split_line(d: number, l: number, t: symbol)
        .input split_line
        .decl picked(d: number)
        .input picked
        .decl trim(raw: symbol, s: symbol)
        trim(raw, @replace_all(raw, "^ +", "")) :- split_line(_, _, raw).
        .decl line(d: number, l: number, s: symbol)
        .output line
        line(d, l, raw) :- picked(d), split_line(d, l, x), trim(x, raw).
        "#,
    );
    assert!(report.inlined.contains(&"trim".to_owned()));
    assert_eq!(rules(&v), vec![r#"line($d, $l, $raw) :- picked($d), split_line($d, $l, $x), $raw = @replace_all($x, "^ +", "")"#]);
}

/// An attribute's request is read by name (`syntax::VALUE_REQ`): a request over one node is a test of one node, but
/// stays, or `value` answers nothing.
#[test]
fn an_attributes_request_is_not_inlined() {
    let (report, v) = simplified(
        r#"
        .decl node(d: number, n: node, kind: symbol, start: number, end: number)
        .input node
        .decl rr_value_req(n: node)
        .decl value(n: node, v: symbol)
        .decl out(v: symbol)
        .output out
        rr_value_req(n) :- node(_, n, "number", _, _).
        value(n, v) :- rr_value_req(n), v = @rr_node_value(n).
        out(v) :- node(_, n, "number", _, _), value(n, v).
        "#,
    );
    assert!(!report.inlined.iter().any(|r| r == "rr_value_req"), "{report:?}");
    assert!(!rules_of(&v, "rr_value_req").is_empty());
}

#[test]
fn a_test_of_one_node_read_positively_is_inlined() {
    let (report, v) = simplified(
        r#"
        .decl node(d: number, n: node, kind: symbol, start: number, end: number)
        .decl text(n: node, s: symbol)
        .input node
        .input text
        .decl k(n: node)
        .decl not_k(n: node)
        .decl named(n: node)
        .decl read_negated(n: node)
        .decl j(n: node)
        .decl not_both(n: node)
        .decl out(n: node)
        .output out
        k(n) :- text(n, "x").
        j(n) :- text(n, "z").
        not_both(n) :- node(_, n, _, _, _), !k(n).
        not_both(n) :- node(_, n, _, _, _), !j(n).
        not_k(m) :- node(_, m, _, _, _), !k(m).
        named(n) :- node(_, n, "identifier", _, _), not_k(n).
        read_negated(n) :- node(_, n, _, _, _), !k(n), text(n, "y").
        out(a) :- node(_, a, "call", _, _), named(a), !read_negated(a), not_both(a).
        "#,
    );
    // `not_k`, then `named` (now `node(...), !k($n)` over `$n` alone); `read_negated` is read negated, and `k` has
    // no node test. `not_both` is first one rule, `!not_both_all($n)` with `not_both_all($n) :- k($n), j($n)` (De
    // Morgan). Then `j`, a filter read once positively, is inlined into `not_both_all`.
    assert_eq!(report.inlined, vec!["named".to_owned(), "not_both".to_owned(), "not_k".to_owned(), "j".to_owned()]);
    let out = &rules_of(&v, "out")[0];
    assert!(out.contains("!k($a)") && out.contains("!not_both_all($a)") && !out.contains("named") && !out.contains("not_k"), "{out}");
    assert!(rules(&v).iter().any(|r| r == r#"not_both_all($n) :- k($n), text($n, "z")"#), "{:?}", rules(&v));
    assert!(!v.declares("not_k") && !v.declares("named") && v.declares("read_negated"));
}

/// A relation read only by `count { … } = 0` loses the columns the count does not name, in the count's atom as in its
/// rules' heads (both arities change together).
#[test]
fn an_existence_count_reads_the_projection() {
    let (report, v) = simplified(
        r#"
        .decl node(d: number, n: node, kind: symbol, start: number, end: number)
        .input node
        .decl pair(a: node, b: node, k: symbol)
        .decl out(n: node)
        .output out
        pair(a, b, k) :- node(_, a, k, _, _), node(_, b, k, _, _), a != b.
        out(n) :- node(_, n, "call", _, _), count : { pair(n, _, _) } = 0.
        "#,
    );
    assert_eq!(report.projected, vec![("pair".to_owned(), vec![1, 2])]);
    assert_eq!(v.columns_of("pair").map(|c| c.len()), Some(1));
    let text = rules(&v);
    assert!(text.iter().any(|r| r.starts_with("out(") && r.contains("pair($n)")), "{text:?}");
    assert!(text.iter().any(|r| r.starts_with("pair($a) :-")), "{text:?}");
}

/// A relation whose readers read no location column keeps its first one: the split partitions a file-local
/// relation's rows by it, and a load hands on a document's rows by it (python/modules.rr's `py_docmod`).
#[test]
fn a_projection_keeps_a_location_column() {
    let (report, v) = simplified(
        r#"
        .decl doc(d: number, p: symbol, q: symbol)
        .input doc
        .decl docmod(d: number, p: symbol, q: symbol)
        .decl out(q: symbol)
        .output out
        docmod(d, p, q) :- doc(d, p, q), q != "".
        out(q) :- docmod(_, _, q).
        "#,
    );
    assert_eq!(report.projected, vec![("docmod".to_owned(), vec![1])]);
    let text = rules(&v);
    assert!(text.iter().any(|r| r.starts_with("docmod($d, $q) :-")), "{text:?}");
}

/// A test of stated facts alone is decided before the run: `!setting("guests", "off")` drops its rule when the fact
/// is stated, and a positive test that holds leaves its body.
#[test]
fn tests_of_stated_facts_are_decided() {
    let (_, v) = simplified(
        r#"
        .decl node(d: number, n: node, kind: symbol, start: number, end: number)
        .input node
        .decl setting(k: symbol, v: symbol)
        setting("guests", "off").
        .decl gated(n: node)
        .decl kept(n: node)
        .decl lost(n: node)
        .output gated
        .output kept
        .output lost
        gated(n) :- !setting("guests", "off"), node(_, n, "call", _, _).
        kept(n) :- setting("guests", "off"), node(_, n, "call", _, _).
        lost(n) :- setting("guests", "on"), node(_, n, "call", _, _).
        "#,
    );
    let text = rules(&v);
    assert!(!text.iter().any(|r| r.starts_with("gated(") || r.starts_with("lost(")), "{text:?}");
    assert!(text.iter().any(|r| r.starts_with("kept(") && !r.contains("setting")), "{text:?}");
}

/// `count : { r(…) } = 0` is `!r(…)`, and `> 0` is `r(…)`: a plain read for demand, inlining and projection.
#[test]
fn an_existence_count_is_a_negation() {
    let text = r#"
        .decl node(d: number, n: node, kind: symbol, start: number, end: number)
        .input node
        .decl pair(a: node, b: node, k: symbol)
        .decl none(n: node)
        .decl some(n: node)
        .decl many(n: node)
        .output none
        .output some
        .output many
        pair(a, b, k) :- node(_, a, k, _, _), node(_, b, k, _, _), a != b.
        none(n) :- node(_, n, "call", _, _), count : { pair(n, _, _) } = 0.
        some(n) :- node(_, n, "call", _, _), count : { pair(n, _, _) } > 0.
        many(n) :- node(_, n, "call", _, _), count : { pair(n, _, _) } > 1.
    "#;
    // The `.rr` reader reads `count : {…} = 0` as the negation already (`none`'s rule): only `> 0` is left.
    let read = crate::code_rule_dsl::logic::rr::parse_version(text).unwrap();
    assert!(rules_of(&read, "none").iter().all(|r| !r.contains("count")), "{:?}", rules(&read));
    let (report, v) = simplified(text);
    assert_eq!(report.existence, 1);
    let text = rules(&v);
    assert!(text.iter().any(|r| r.starts_with("none(") && r.contains("!pair($n") && !r.contains("count")), "{text:?}");
    assert!(text.iter().any(|r| r.starts_with("some(") && r.contains(", pair($n") && !r.contains("count")), "{text:?}");
    assert!(text.iter().any(|r| r.starts_with("many(") && r.contains("count")), "{text:?}");
}

/// A relation's rules are a set: `p1` has its rule twice and `p2` once, and they are one class, so one merge finds it
/// (a list comparison kept them apart until the repeat was dropped, which took a second merge).
#[test]
fn a_repeated_rule_does_not_keep_a_relation_from_its_twin() {
    let (report, v) = simplified(
        ".decl e(a: number, b: number)\n.input e\n.decl p1(a: number, b: number)\n.decl p2(a: number, b: number)\n\
         .decl out(a: number)\n.output out\np1(a, b) :- e(a, b).\np1(a, b) :- e(a, b).\np2(a, b) :- e(a, b).\n\
         out(a) :- p1(a, _), p2(_, a).\n",
    );
    assert_eq!(report.merged, 1, "{:?}", rules(&v));
    assert_eq!(["p1", "p2"].iter().filter(|r| v.declares(r)).count(), 1, "{:?}", rules(&v));
}

/// A request never merges, but its widened copy left with the request's own rules is read as the request: once
/// pruned, `eslint-yoda`'s `rr_wide_node_text_rr_token_req` held `rr_token_req`'s rules and rows. A copy whose rules
/// differ stays. PR #508's eb4a92d28 (t47), on the tree's pass there; stated here on the version's.
#[test]
fn a_requests_copy_with_its_own_rules_is_read_as_the_request() {
    let (_, v) = simplified(
        r#"
        .decl base(n: number)
        .input base
        .decl other(n: number)
        .input other
        .decl rr_token_req(n: number)
        .decl rr_wide_node_text_rr_token_req(n: number)
        rr_token_req(n) :- base(n).
        rr_wide_node_text_rr_token_req(n) :- base(n).
        .decl rr_value_req(n: number)
        .decl rr_wide_node_text_rr_value_req(n: number)
        rr_value_req(n) :- base(n).
        rr_wide_node_text_rr_value_req(n) :- other(n).
        .decl out(n: number)
        .output out
        out(n) :- rr_wide_node_text_rr_token_req(n), rr_token_req(n), rr_wide_node_text_rr_value_req(n), rr_value_req(n).
        "#,
    );
    let text = rules(&v);
    assert!(!text.iter().any(|r| r.contains("rr_wide_node_text_rr_token_req")), "{text:#?}");
    assert_eq!(rules_of(&v, "rr_token_req").len(), 1, "{text:#?}");
    assert_eq!(rules_of(&v, "rr_wide_node_text_rr_value_req").len(), 1, "{text:#?}");
}

/// A relation whose every rule a stated fact decided against has no rows, so an atom over it is decided too: a
/// positive one drops its rule (`selected`, then `asked` in turn), a negated one leaves its body (`unselected`). Left
/// read, `entry_file` was a relation of no rule, fact or input, which the split reads as another file's: `asked` went
/// to the cross-file part, and the run was refused (decision 0174 §6). PR #508's eb4a92d28 (t47).
#[test]
fn reads_of_a_relation_the_stated_facts_emptied_are_decided() {
    let (_, v) = simplified(
        r#"
        .decl node(d: number, n: node, kind: symbol, start: number, end: number)
        .input node
        .decl doc(d: number, path: symbol, module: symbol)
        .input doc
        .decl option_value(k: symbol, v: symbol)
        option_value("entryPoint", "one").
        .decl entry_file(d: number)
        .decl selected(n: node)
        .decl asked(n: node)
        .decl unselected(n: node)
        .output asked
        .output unselected
        entry_file(d) :- doc(d, _, _), option_value("entryPoint", "all").
        entry_file(d) :- doc(d, _, _), option_value("entryPoint", "two").
        selected(n) :- node(d, n, "call", _, _), entry_file(d).
        asked(n) :- selected(n).
        asked(n) :- node(_, n, "string", _, _).
        unselected(n) :- node(d, n, "call", _, _), !entry_file(d).
        "#,
    );
    let text = rules(&v);
    assert!(!text.iter().any(|r| r.contains("entry_file") || r.contains("selected(") && !r.contains("unselected(")), "{text:#?}");
    assert!(rules_of(&v, "asked").iter().any(|r| r.contains("\"string\"")), "{text:#?}");
    assert_eq!(rules_of(&v, "unselected").len(), 1, "{text:#?}");
    let split = crate::code_rule_dsl::logic::program_image::manager::analysis::<crate::code_rule_dsl::typing::file_local::FileLocal>(&v).unwrap();
    assert!(split.local.contains("asked"), "{:?}\n{text:#?}", split.causes);
}

/// A relation whose every rule a fact test decides away is empty: a rule reading it goes, a negated read of it is
/// dropped, to a fixpoint (a file scope the run's options rule out, read by a guest's copies). The tree simplify's test
/// of 9a9b270ce, on the version's passes.
#[test]
fn a_relation_its_fact_tests_empty_takes_its_readers() {
    let (_, v) = simplified(
        r#"
        .decl opt(k: symbol)
        opt("a").
        .decl base(n: number)
        .input base
        .decl scope(n: number)
        scope(n) :- base(n), opt("b").
        .decl reads(n: number)
        reads(n) :- base(n), scope(n).
        .decl again(n: number)
        again(n) :- reads(n).
        .decl apart(n: number)
        apart(n) :- base(n), !scope(n).
        .decl out(n: number)
        .output out
        out(n) :- again(n).
        out(n) :- apart(n).
        "#,
    );
    let heads: std::collections::BTreeSet<String> = v.rules().map(|r| v.rel_name(r.head().rel()).to_owned()).collect();
    assert!(!heads.contains("scope") && !heads.contains("reads") && !heads.contains("again"), "{heads:?}");
    assert!(rules(&v).iter().any(|r| r.contains("base(") && !r.contains("scope")), "{:?}", rules(&v));
}
