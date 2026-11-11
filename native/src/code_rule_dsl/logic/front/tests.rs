//! The front end's checks (stratification, lattices, value cycles, prune) on programs in `.rr` (`logic::rr`).

use super::*;
use crate::code_rule_dsl::logic::rr::parse;

fn program(name: &str) -> Program {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/logic/plan/golden/programs").join(name);
    parse(&std::fs::read_to_string(path).unwrap()).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// T's five programs parse, stratify and prune to what T's `rdl.py` reports (its `parse`, `strata` and `prune`,
/// run on the same files): relations, rules, facts, lattices, demand, kindsets, inputs, outputs, components, the
/// largest component, and the rules and components left after pruning.
#[test]
fn t_programs_match_t_front_end() {
    type Expected = (usize, usize, usize, usize, &'static [&'static str], usize, usize, usize, usize, usize, usize, usize);
    let golden: [(&str, Expected); 5] = [
        ("calls.rr", (98, 221, 160, 0, &[], 3, 7, 7, 76, 7, 221, 76)),
        ("calls_rust.rr", (61, 148, 9, 0, &[], 5, 6, 4, 45, 8, 147, 44)),
        ("workload.rr", (42, 63, 35, 1, &[], 1, 7, 3, 29, 2, 63, 29)),
        (
            "workload_demand.rr",
            (43, 63, 35, 1, &["class_ref", "cv", "fn_ref", "has_def", "lookup", "returns", "type_of"], 1, 7, 3, 30, 2, 63, 30),
        ),
        ("workload_perdoc.rr", (45, 66, 35, 1, &[], 1, 10, 3, 29, 2, 66, 29)),
    ];
    for (name, expected) in golden {
        let p = program(name);
        let (sccs, _) = strata(&p).unwrap();
        let mut demand = p.demand.clone();
        demand.sort();
        let pruned = prune(p.clone());
        let (pruned_sccs, _) = strata(&pruned).unwrap();
        let got = (
            p.decls.len(),
            p.rules.len(),
            p.facts.values().map(Vec::len).sum::<usize>(),
            p.lattice.len(),
            demand.iter().map(String::as_str).collect::<Vec<_>>(),
            p.kindsets.len(),
            p.inputs.len(),
            p.outputs.len(),
            sccs.len(),
            sccs.iter().map(BTreeSet::len).max().unwrap_or(0),
            pruned.rules.len(),
            pruned_sccs.len(),
        );
        let (d, r, f, l, dm, k, i, o, s, big, pr, ps) = expected;
        assert_eq!(got, (d, r, f, l, dm.to_vec(), k, i, o, s, big, pr, ps), "{name}");
        // O11: the value cycles, each bounded by the tree.
        let treed: Vec<String> = cycles::check(&p).unwrap().iter().map(|c| c.iter().cloned().collect::<Vec<_>>().join(",")).collect();
        let expected: &[&str] = match name {
            "calls.rr" => &["qn"],
            "calls_rust.rr" => &["modn"],
            _ => &["resolves"],
        };
        for rel in expected {
            assert!(treed.iter().any(|c| c.split(',').any(|r| r == *rel)), "{name}: {rel} not bounded by the tree: {treed:?}");
        }
    }
}

#[test]
fn literals_parse_to_t_ast() {
    let p = parse(
        r#"// a comment
        .decl e(a: node, b: node)
        .input e
        .decl scope(kind: symbol)
        scope("module"). scope("function_definition").
        r(n, @dot(q, a)) :- e(n, o), !g(o), k = count : { e(o, _) }, k >= 2, nearest(n, s, scope), q = "x", a = top.
        e(1, -2).
        "#,
    )
    .unwrap();
    assert_eq!(p.facts["e"], [vec![Value::Int(1), Value::Int(-2)]]);
    let body = &p.rules[0].body;
    assert_eq!(p.rules[0].head.args[1], Term::Call("cat".into(), vec![Term::Var("q".into()), Term::Var("a".into())]));
    assert!(matches!(&body[1], Literal::Atom(a) if a.neg && a.rel == "g"));
    assert!(matches!(&body[2], Literal::Agg { var, op: AggOp::Count, over: None, atom } if var == "k" && atom.args[1] == Term::Wild));
    assert!(matches!(&body[3], Literal::Cmp { op: ">=", r: Term::Int(2), .. }));
    assert!(matches!(&body[4], Literal::Nearest { kindset, .. } if kindset == "scope"));
    assert!(matches!(&body[6], Literal::Cmp { r: Term::Top, .. }));
    assert_eq!(p.kindsets["scope"], ["module", "function_definition"]);
}

#[test]
fn section_12_extensions_parse() {
    let p = parse(
        r#".lattice approx_join<approx> { Lub -> @approx_join }
        .decl type_of(d: number, n: node, t: approx_join<>) widen 8
        .pragma "magic-transform" "*"
        m(g, lo) :- e(g, _), lo = min v : { e(g, v) }.
        t(d, n) :- ast(d, n, { kind: typed_parameter, not: { inside: { kind: lambda } } }).
        "#,
    )
    .unwrap();
    assert_eq!(p.lattice["type_of"], LatticeDecl { keys: 2, ty: "approx".into(), merge: Some("approx_join".into()), widen: Some(8) });
    assert_eq!(p.lattice["type_of"].lattice(), "approx_join");
    assert_eq!(p.demand, ["*"]);
    assert!(matches!(&p.rules[0].body[1], Literal::Agg { op: AggOp::Min, over: Some(v), .. } if v == "v"));
    assert!(
        matches!(&p.rules[1].body[0], Literal::Ast { matcher, .. } if matcher == "{ kind: typed_parameter, not: { inside: { kind: lambda } } }")
    );
}

/// A matcher atom exports metavariables: each binds a variable to its captured node, and prints back as written.
#[test]
fn matcher_atom_exports_captures() {
    let src = r#"f(n, s) :- ast(d, f, {"rule": {"kind": "function_definition", "has": {"field": "name", "pattern": "$FNAME"}}}, FNAME: n), text(n, s)."#;
    let p = parse(src).unwrap();
    let Literal::Ast { captures, .. } = &p.rules[0].body[0] else { panic!("not a matcher atom") };
    assert_eq!(captures, &[("FNAME".to_owned(), "n".to_owned())]);
    assert_eq!(p.rules[0].body[0].vars(), ["d", "f", "n"].map(String::from).into());
    assert!(crate::code_rule_dsl::logic::rr::print(&p).unwrap().contains(src));
    assert!(matches!(parse("f(n) :- ast(d, f, {}, FNAME n)."), Err(Error::Syntax(_))));
}

#[test]
fn named_errors() {
    let unbounded = parse(".decl hi(n: node, v: max<>)\n");
    assert_eq!(unbounded.unwrap_err(), Error::UnboundedLattice { relation: "hi".into() });
    assert!(parse(".decl hi(n: node, v: max<>) widen 4\n").is_ok());

    let negation = parse("a(x) :- e(x), !b(x).\nb(x) :- a(x).\n").unwrap();
    assert!(matches!(strata(&negation), Err(Error::NotStratifiable { negated, .. }) if negated == "b"));

    let counting = "a(x, k) :- e(x), k = count : { b(x, _) }.\nb(x, y) :- a(x, y).\n";
    assert!(matches!(strata(&parse(counting).unwrap()), Err(Error::AggregationInRecursion { head, .. }) if head == "a"));
    // Into a max lattice, a count only rises: monotone, so allowed.
    let monotone = format!(".decl a(x: node, k: max<>) widen 8\n{counting}");
    assert!(strata(&parse(&monotone).unwrap()).is_ok());

    // O11: names made along a tree edge are bounded by the tree; along a graph, they are not.
    let decls = ".decl e(a: node, b: node)\n.decl field(p: node, f: symbol, c: node)\n.decl nm(n: node, s: symbol)\n.decl q(n: node, s: symbol)\n.input e, field, nm\n";
    let down = format!("{decls}q(n, s) :- nm(n, s).\nq(n, @dot(p, s)) :- field(n, \"object\", o), q(o, p), nm(n, s).\n");
    assert!(parse(&down).is_ok());
    let up = format!("{decls}.decl r(n: node, s: symbol)\nr(n, s) :- nm(n, s).\nr(n, @dot(p, s)) :- nearest(n, a, k), r(a, p), nm(n, s).\n.decl k(kind: symbol)\nk(\"module\").\n");
    assert!(parse(&up).is_ok(), "{:?}", parse(&up).err());
    let graph = format!("{decls}q(n, s) :- nm(n, s).\nq(n, @dot(p, s)) :- e(n, o), q(o, p), nm(n, s).\n");
    assert!(matches!(parse(&graph), Err(Error::UnboundedValueCycle { component, .. }) if component.contains("q")));
    // Down one way and up the other: no key moves one way.
    let both = format!("{decls}q(n, s) :- nm(n, s).\nq(n, @dot(p, s)) :- field(n, \"object\", o), q(o, p), nm(n, s).\nq(o, s) :- field(n, \"object\", o), q(n, s).\n");
    assert!(matches!(parse(&both), Err(Error::UnboundedValueCycle { .. })));
    // O111: through constant keys that never come back (`era001`'s grammar), finitely many values; a key that comes
    // back is not bounded.
    let grammar = ".decl g(k: symbol, r: symbol)\ng(\"N\", \"[a-z]+\").\ng(\"E\", @cat(n, \"(?:,\", n, \")*\")) :- g(\"N\", n).\ng(\"S\", @cat(\"(?:\", e, \")\")) :- g(\"E\", e).\n";
    assert!(parse(grammar).is_ok(), "{:?}", parse(grammar).err());
    let back = format!("{grammar}g(\"N\", @cat(s, \"x\")) :- g(\"S\", s).\n");
    assert!(matches!(parse(&back), Err(Error::UnboundedValueCycle { .. })));
    // A count whose next value a lower stratum binds (`era_run` along `era_cline`'s lines) takes its values.
    let lines = ".decl ln(d: number, l: number)\n.input ln\n.decl run(d: number, l0: number, l: number)\nrun(d, l, l) :- ln(d, l).\nrun(d, l0, @add(l, 1)) :- run(d, l0, l), t = @add(l, 1), ln(d, t).\n";
    assert!(parse(lines).is_ok(), "{:?}", parse(lines).err());
    let unbound = ".decl ln(d: number, l: number)\n.input ln\n.decl run(d: number, l0: number, l: number)\nrun(d, l, l) :- ln(d, l).\nrun(d, l0, @add(l, 1)) :- run(d, l0, l), t = @add(l, 2), ln(d, t).\n";
    assert!(matches!(parse(unbound), Err(Error::UnboundedValueCycle { .. })));
    let same = ".decl g(k: symbol, r: symbol)\ng(\"N\", \"a\").\ng(\"N\", @cat(n, \"a\")) :- g(\"N\", n).\n";
    assert!(matches!(parse(same), Err(Error::UnboundedValueCycle { .. })));
    // A value bounded above and below by integers takes finitely many values, as a table of them would bind it
    // (python/lengths' `t >= 0, t <= 100000`); bounded one way only, it does not.
    let product = |bounds: &str| format!(".decl e(a: node, b: node)\n.input e\n.decl len(n: node, k: number)\nlen(n, 1) :- e(n, _).\nlen(n, t) :- e(n, m), len(m, k), len(n, w), t = @mul(k, w){bounds}.\n");
    assert!(parse(&product(", t >= 0, t <= 100000")).is_ok(), "{:?}", parse(&product(", t >= 0, t <= 100000")).err());
    assert!(matches!(parse(&product(", t <= 100000")), Err(Error::UnboundedValueCycle { .. })));
    // A derived relation's guarantee carries the bound (`attr_of` in T's calls.rdl).
    let derived = format!("{decls}.decl at(n: node, o: node)\nat(n, o) :- field(n, \"object\", o).\nq(n, s) :- nm(n, s).\nq(n, @dot(p, s)) :- at(n, o), q(o, p), nm(n, s).\n");
    assert!(parse(&derived).is_ok());

    // Depth bounds what preorder cannot: a parent relation with a decorator's edge (to the definition, a later
    // node: up two edges, down one) still climbs one level each step (`ast_parent` in `python/ast.rr`).
    let tree = ".decl child(p: node, c: node, i: number)\n.decl kind(n: node, k: symbol)\n.decl nm(n: node, s: symbol)\n.input child, kind, nm\n\
        .decl deco(f: node, e: node)\ndeco(f, e) :- kind(dd, \"decorated_definition\"), child(dd, f, 1), child(dd, dc, 0), child(dc, e, 0).\n\
        .decl up(n: node, p: node)\nup(n, p) :- child(p, n, _), !kind(p, \"block\").\nup(e, f) :- deco(f, e).\nup(n, q) :- child(p, n, _), kind(p, \"block\"), up(p, q).\n";
    let climb = format!("{tree}.decl qn(k: node, s: symbol)\nqn(k, s) :- kind(k, \"class\"), nm(k, s).\nqn(k, @dot(p, s)) :- kind(k, \"class\"), up(k, o), qn(o, p), nm(k, s).\n");
    assert!(parse(&climb).is_ok(), "{:?}", parse(&climb).err());
    // A sibling keeps the depth: no bound.
    let sideways = format!("{tree}.decl sib(a: node, b: node)\nsib(a, b) :- child(p, a, _), child(p, b, _).\n.decl qn(k: node, s: symbol)\nqn(k, s) :- nm(k, s).\nqn(k, @dot(p, s)) :- sib(k, o), qn(o, p), nm(k, s).\n");
    assert!(matches!(parse(&sideways), Err(Error::UnboundedValueCycle { .. })));

    // The engine joins on variables: a computed atom argument is refused (`.rr` binds it first, `rr::lower`).
    let computed = super::parse(".decl e(x: sym)\n.decl a(x: sym)\na($x) :- e(@cat($x, $x)).\n");
    assert!(matches!(computed, Err(Error::Syntax(m)) if m.contains("computed term")));
}

/// O11 through derived relations that are themselves recursive, and along a node's children: names built up an
/// attribute chain through `unparen` (at or below its node, by induction over its rules: matching-architecture's
/// `qual`, bootstrap-cache-health's `qualified`), and a node's parts joined in child order. Still refused: a move
/// that may stay at the node, a relation with facts or an input (rows no rule made), a recursive relation over a
/// graph, counting with no child bound, and a position whose node changes.
#[test]
fn value_cycles_bounded_through_derived_relations_and_positions() {
    let decls = "\
.decl field(p: node, f: symbol, c: node)
.decl child(p: node, c: node, i: number)
.decl node(d: number, n: node, kind: symbol, start: number, end: number)
.decl text(n: node, s: symbol)
.decl e(a: node, b: node)
.input field, child, node, text, e
.decl unparen(e: node, m: node)
unparen(e, e) :- node(_, e, k, _, _), k != \"parenthesized_expression\".
unparen(e, m) :- node(_, e, \"parenthesized_expression\", _, _), child(e, i, 0), unparen(i, m).
.decl q(a: node, s: symbol)
q(n, s) :- node(_, n, \"identifier\", _, _), text(n, s).
";
    let attr = ".decl attr(a: node, x: symbol, v: node)\nattr(a, x, v) :- field(a, \"attribute\", i), text(i, x), field(a, \"object\", o), unparen(o, v).\n";
    let qual = "q(a, s) :- attr(a, x, v), q(v, p), s = cat(cat(p, \".\"), x).\n";
    let ok = |src: String| parse(&src).unwrap_or_else(|e| panic!("{e}\n{src}"));
    let refused = |src: String| assert!(matches!(parse(&src), Err(Error::UnboundedValueCycle { .. })), "{src}");
    ok(format!("{decls}{attr}{qual}"));
    // A recursive relation strictly below: every descendant.
    ok(format!("{decls}.decl desc(a: node, d: node)\ndesc(a, d) :- child(a, d, _).\ndesc(a, d) :- child(a, c, _), desc(c, d).\nq(a, s) :- desc(a, v), q(v, p), s = cat(p, \"x\").\n"));
    // unparen alone may stay at the node: a name made there could be made again.
    refused(format!("{decls}q(a, s) :- unparen(a, v), q(v, p), s = cat(p, \"x\").\n"));
    // The facts hole: a fact, or an input, is a row no rule bounds.
    let at = ".decl at(n: node, o: node)\nat(n, o) :- field(n, \"object\", o).\n";
    let along_at = "q(n, s) :- at(n, o), q(o, p), s = cat(p, \"x\").\n";
    ok(format!("{decls}{at}{along_at}"));
    refused(format!("{decls}{at}at(1, 1).\n{along_at}"));
    refused(format!("{decls}{at}.input at\n{along_at}"));
    // A recursive relation over a graph guarantees nothing.
    refused(format!("{decls}.decl reach(a: node, b: node)\nreach(a, b) :- e(a, b).\nreach(a, b) :- e(a, c), reach(c, b).\nq(a, s) :- reach(a, v), q(v, p), s = cat(p, \"x\").\n"));
    // Parts joined along a node's children, index by index.
    let parts = ".decl part(p: node, t: symbol)\npart(p, t) :- text(p, t).\n.decl j(c: node, i: number, s: symbol)\nj(c, 0, s) :- child(c, p, 0), part(p, s).\n";
    ok(format!("{decls}{parts}j(c, i + 1, s2) :- j(c, i, s), child(c, p, i + 1), part(p, t), s2 = cat(s, t).\n"));
    ok(format!("{decls}{parts}j(c, k, s2) :- j(c, i, s), k = i + 1, child(c, p, k), part(p, t), s2 = cat(s, t).\n"));
    refused(format!("{decls}{parts}j(c, i + 1, s2) :- j(c, i, s), part(c, t), s2 = cat(s, t).\n"));
    refused(format!("{decls}{parts}j(c, i + 1, s2) :- j(d, i, s), e(c, d), child(c, p, i + 1), part(p, t), s2 = cat(s, t).\n"));
    refused(format!("{decls}{parts}j(c, i + 1, s2) :- j(c, i, s), child(c, p, i + 1), part(p, t), s2 = cat(s, t).\nj(c, i - 1, s2) :- j(c, i, s), child(c, p, i - 1), part(p, t), s2 = cat(t, s).\n"));
}

/// `nearest(n, _, set)` holds where n has an ancestor in the set (it was "nearest: expected a variable, got Wild"):
/// `inside(n, _, set)`.
#[test]
fn positive_nearest_takes_a_wildcard() {
    let p = parse(".decl k(kind: symbol)\nk(\"module\").\n.decl r(n: node)\nr(n) :- node(_, n, \"identifier\", _, _), nearest(n, _, k).\n").unwrap();
    assert!(matches!(&p.rules[0].body[1], Literal::Atom(a) if a.rel == format!("{AXIS}k") && a.args[1] == Term::Wild && !a.neg), "{}", p.rules[0]);
}

/// `.input` of a relation nothing declares is a named error, not the planner's panic (`plan`'s `prog.decls[rel]`).
#[test]
fn an_undeclared_input_is_named() {
    let undeclared = ".decl w(x: symbol)\nw(\"a\").\n.decl v(x: symbol)\nv(x) :- w(x).\n.input undeclared\n";
    assert_eq!(parse(undeclared).unwrap_err(), Error::UndeclaredInput { relation: "undeclared".into() });
}

#[test]
fn comparisons_bind_like_t() {
    let bound = |vs: &[&str]| vs.iter().map(|v| v.to_string()).collect::<BTreeSet<_>>();
    let x = Term::Var("x".into());
    let cat = Term::Call("cat".into(), vec![Term::Var("q".into()), Term::Var("a".into())]);
    assert_eq!(cmp_binds(&x, "=", &cat, &bound(&["q", "a"])), Some(bound(&["x"])));
    assert_eq!(cmp_binds(&x, "=", &cat, &bound(&["q"])), None);
    // A bound result splits into its unbound parts.
    assert_eq!(cmp_binds(&x, "=", &cat, &bound(&["x", "q"])), Some(bound(&["a"])));
    assert_eq!(cmp_binds(&x, "<", &Term::Int(3), &bound(&[])), None);
}

/// The components come in T's order, which the planner and the native strata follow.
#[test]
fn components_in_t_order() {
    let (sccs, _) = strata(&program("calls_rust.rr")).unwrap();
    let got = sccs.iter().map(|c| c.iter().cloned().collect::<Vec<_>>().join(",")).collect::<Vec<_>>().join(" | ");
    assert_eq!(
        got,
        "kind | nm | impl_fn | iparent | modn | modchild | item | uent | bound_here | glob_ | modpar | inmod | isvis | pubitem | \
         pubuse | mpub | local | tbase | troot | wants | islocal | seg1 | lwalk | lfound | lglob | trait_fn | \
         assoc,globx,impl_of,impl_trait,member,mexport,res,ures | callee | sfield | ttype | call_target,etype | lethasfn | \
         pathk2 | tpname | generic | exttype | rext | call_open | owner | calls | crate_fn_name | macro_open | calls_open | \
         isasync | reaches_open"
    );
}

// ---- recursion ----

/// Recursion that makes values, bounded by the data (`cycles`' rules), is accepted; each unbounded form next to
/// it is refused by name.
#[test]
fn value_cycles_bounded_by_the_data() {
    let decls = ".decl child(p: node, c: node, i: number)\n.decl field(p: node, f: symbol, c: node)\n.decl next_sibling(a: node, b: node)\n.decl kind(n: node, k: symbol)\n.decl text(n: node, s: symbol)\n.decl e(a: node, b: node)\n.decl lim(k: number)\n.input child, field, next_sibling, kind, text, e, lim\n";
    let ok = |src: &str| parse(&format!("{decls}{src}")).unwrap_or_else(|e| panic!("{src}: {e}"));
    let refused = |src: &str| assert!(matches!(parse(&format!("{decls}{src}")), Err(Error::UnboundedValueCycle { .. })), "{src}");
    // A counter below a constant, or below a value of a lower stratum; not one with no bound, or bounded the
    // wrong way, or below a value the recursion itself makes.
    ok(".decl n(x: number)\nn(1).\nn(x + 1) :- n(x), x < 20.\n");
    ok(".decl n(x: number)\nn(1).\nn(y) :- n(x), y = x + 1, y <= k, lim(k).\n");
    ok(".decl n(x: number)\nn(9).\nn(x - 1) :- n(x), x > 0.\n");
    refused(".decl n(x: number)\nn(1).\nn(x + 1) :- n(x).\n");
    refused(".decl n(x: number)\nn(1).\nn(x + 1) :- n(x), x > 0.\n");
    refused(".decl n(x: number)\nn(0).\nn(1).\nn(x + 2) :- n(x), n(y), x < y.\n");
    // A value made from values of lower strata only.
    ok(".decl upto(p: node, i: number)\nupto(p, 0) :- kind(p, \"list\").\nupto(p, i + 1) :- upto(p, i), child(p, _, i).\n");
    // Along siblings, the order of the document, with a count made at each step (`arg_index`).
    ok(".decl arg(t: node, c: node, k: number)\narg(t, c, 0) :- child(t, c, 0).\narg(t, d, k + w) :- arg(t, c, k), next_sibling(c, d), w = count : { child(c, _, _) }.\n");
    ok(".decl arg(t: node, c: node, k: number)\narg(t, c, 0) :- child(t, c, 0).\narg(t, d, k + 1) :- arg(t, c, k), child(t, c, i), child(t, d, i + 1).\n");
    refused(".decl arg(t: node, c: node, k: number)\narg(t, c, 0) :- child(t, c, 0).\narg(t, d, k + 1) :- arg(t, c, k), child(t, c, _), child(t, d, _).\n");
    // A recursive relation's guarantee: unparen(e, m) is m at or below e, so the name moves down (`ruff_qual`);
    // one rule up instead and there is none.
    let unparen = ".decl unparen(e: node, m: node)\nunparen(e, e) :- kind(e, k), k != \"parens\".\nunparen(e, m) :- kind(e, \"parens\"), child(e, i, _), unparen(i, m).\n";
    let qual = ".decl qual(e: node, q: symbol)\nqual(e, x) :- kind(e, \"name\"), text(e, x).\nqual(e, @cat(q, x)) :- unparen(e, m), field(m, \"object\", o), qual(o, q), text(m, x).\n";
    ok(&format!("{unparen}{qual}"));
    refused(&format!("{}{qual}", unparen.replace("child(e, i, _)", "child(i, e, _)")));
    // At or below, and unequal: strictly below (`const_str`'s `deparen(n, e), e != n`).
    ok(&format!("{unparen}.decl cs(n: node, v: symbol)\ncs(n, v) :- text(n, v).\ncs(n, @cat(v, \"!\")) :- unparen(n, e), e != n, cs(e, v).\n"));
    refused(&format!("{unparen}.decl cs(n: node, v: symbol)\ncs(n, v) :- text(n, v).\ncs(n, @cat(v, \"!\")) :- unparen(n, e), cs(e, v).\n"));
    // A regex group is a part of the text: no new value (`gi_path`); and a strictly shorter one when the regex
    // matches something around it, which bounds a count made beside it (`pct_rest`). A group that may be all of
    // the text bounds nothing.
    ok(".decl dirs(d: symbol)\ndirs(s) :- text(_, s).\ndirs(a) :- dirs(p), match(\"(?<a>.+)/[^/]+\", p).\n");
    ok(".decl rest(v: symbol, r: symbol, i: number)\nrest(v, v, 0) :- text(_, v).\nrest(v, r, i + 1) :- rest(v, s, i), match(\"[^%]*%%(?<r>[\\\\s\\\\S]*)\", s).\n");
    refused(".decl rest(v: symbol, r: symbol, i: number)\nrest(v, v, 0) :- text(_, v).\nrest(v, r, i + 1) :- rest(v, s, i), match(\"[^%]*(?<r>[\\\\s\\\\S]*)\", s).\n");
    // A part of a made value is made; a made value through another variable is made; parts split from a bound
    // value carry it.
    refused(".decl r(s: symbol)\nr(s) :- text(_, s).\nr(@substr(@cat(x, \"ab\"), 0, 100)) :- r(x).\n");
    refused(".decl q(n: node, s: symbol)\nq(n, s) :- text(n, s).\nq(n, z) :- q(n, s), y = @cat(s, \"x\"), z = y.\n");
    refused(".decl r(s: symbol)\nr(s) :- text(_, s).\nr(y) :- r(m), m = @cat(q, x), y = @cat(@cat(q, x), x).\n");
    // A split of a made value at one key: a part may be all of it (`"" . m2`), so each round makes a longer one.
    refused(".decl q(n: node, s: symbol)\nq(n, s) :- text(n, s).\nq(n, x) :- q(n, m), m2 = @cat(m, \"a\"), m2 = @cat(p, x).\n");
    // The graph relation and the tree's two ways still bound nothing.
    refused(".decl q(n: node, s: symbol)\nq(n, s) :- text(n, s).\nq(n, @cat(p, s)) :- e(n, o), q(o, p), text(n, s).\n");
    // A rule may make a value where the key stays when the cycle through it moves strictly elsewhere (`pct_rest`
    // and `pct_spec`); not when no move of the cycle is strict.
    let spec = ".decl rest(v: symbol, r: symbol, i: number)\n.decl spec(v: symbol, i: number, r: symbol)\nrest(v, v, 0) :- text(_, v).\nrest(v, r, i + 1) :- spec(v, i, r).\n";
    ok(&format!("{spec}spec(v, i, r) :- rest(v, s, i), match(\"[^%]*%(?<r>.*)\", s).\n"));
    refused(&format!("{spec}spec(v, i, r) :- rest(v, s, i), match(\"[^%]*(?<r>.*)\", s).\n"));
    // A count beside the key, up from a value of a lower stratum while the key stays (`tpl_upto`), with a relation
    // of no count above it (`const_str`); not a count up from its own value, nor one a relation with no count
    // gives back at the same key.
    let tpl = ".decl tpl(t: node, i: number, v: symbol)\n.decl cs(n: node, v: symbol)\ntpl(t, 0, \"\") :- kind(t, \"template\").\ncs(n, v) :- tpl(n, i, v), !child(n, _, i).\n";
    ok(&format!("{tpl}tpl(t, i + 1, @cat(v, w)) :- tpl(t, i, v), child(t, c, i), text(c, w).\ntpl(t, i + 1, @cat(v, w)) :- tpl(t, i, v), child(t, c, i), child(c, e, 0), cs(e, w).\n"));
    refused(&format!("{tpl}tpl(t, i + 1, @cat(v, w)) :- tpl(t, i, v), text(t, w).\n"));
    refused(&format!("{tpl}tpl(t, i + 1, @cat(v, w)) :- tpl(t, i, v), child(t, c, i), text(c, w).\ntpl(n, 0, v) :- cs(n, v).\n"));
}

/// The `cycles` gap of a regex group: the bytes a match has outside the group.
#[test]
fn regex_group_gaps() {
    let gap = |re: &str, g: &str| cycles::gap(re, g);
    assert_eq!(gap("^(?:(?<a>.+)/[^/]+)$", "a"), Some(2));
    assert_eq!(gap("^(?:[^%]*%%(?<r>.*))$", "r"), Some(2));
    assert_eq!(gap("^(?:x*(?<r>.*))$", "r"), Some(0));
    assert_eq!(gap("^(?:(?:ab(?<r>c))+)$", "r"), Some(2));
    assert_eq!(gap("^(?:(?:ab(?<r>c)){3})$", "r"), Some(8));
    assert_eq!(gap("^(?:(?<r>a)b|cde)$", "r"), Some(1));
    assert_eq!(gap("^(?:(?<r>a))$", "s"), None);
}
