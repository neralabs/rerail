use super::*;

fn program(name: &str) -> Program {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/code_rule_dsl/logic/front/rdl").join(name);
    parse(&std::fs::read_to_string(path).unwrap()).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// T's five programs parse, stratify and prune to what T's `rdl.py` reports (its `parse`, `strata` and `prune`,
/// run on the same files): relations, rules, facts, lattices, demand, kindsets, inputs, outputs, components, the
/// largest component, and the rules and components left after pruning.
#[test]
fn t_programs_match_t_front_end() {
    type Expected = (usize, usize, usize, usize, &'static [&'static str], usize, usize, usize, usize, usize, usize, usize);
    let golden: [(&str, Expected); 5] = [
        ("calls.rdl", (98, 221, 160, 0, &[], 3, 7, 7, 76, 7, 221, 76)),
        ("calls_rust.rdl", (61, 148, 9, 0, &[], 5, 6, 4, 45, 8, 147, 44)),
        ("workload.rdl", (42, 63, 35, 1, &[], 1, 7, 3, 29, 2, 63, 29)),
        (
            "workload_demand.rdl",
            (43, 63, 35, 1, &["class_ref", "cv", "fn_ref", "has_def", "lookup", "returns", "type_of"], 1, 7, 3, 30, 2, 63, 30),
        ),
        ("workload_perdoc.rdl", (45, 66, 35, 1, &[], 1, 10, 3, 29, 2, 66, 29)),
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
    }
}

#[test]
fn literals_parse_to_t_ast() {
    let p = parse(
        r#"// a comment
        .decl e(a: node, b: node)
        .input e
        .kindset scope = "module" | "function_definition"
        r($n, @cat($q, $a)) :- e($n, $o), !g($o), $k = count { e($o, _) }, $k >= 2, nearest($n, $s, scope), $q = "x", $a = top.
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
        r#".decl type_of(d: int, n: node; t: approx) merge @approx_join widen 8
        .demand *
        m($g, $lo) :- e($g, _), $lo = min $v : { e($g, $v) }.
        t($d, $n) :- ast($d, $n, { kind: typed_parameter, not: { inside: { kind: lambda } } }).
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

#[test]
fn named_errors() {
    let unbounded = parse(".decl hi(n: node; v: max)\n");
    assert_eq!(unbounded.unwrap_err(), Error::UnboundedLattice { relation: "hi".into() });
    assert!(parse(".decl hi(n: node; v: max) widen 4\n").is_ok());

    let negation = parse("a($x) :- e($x), !b($x).\nb($x) :- a($x).\n").unwrap();
    assert!(matches!(strata(&negation), Err(Error::NotStratifiable { negated, .. }) if negated == "b"));

    let counting = "a($x, $k) :- e($x), $k = count { b($x, _) }.\nb($x, $y) :- a($x, $y).\n";
    assert!(matches!(strata(&parse(counting).unwrap()), Err(Error::AggregationInRecursion { head, .. }) if head == "a"));
    // Into a max lattice, a count only rises: monotone, so allowed.
    let monotone = format!(".decl a(x: node; k: max) widen 8\n{counting}");
    assert!(strata(&parse(&monotone).unwrap()).is_ok());

    let computed = parse("a($x) :- e(@cat($x, $x)).\n");
    assert!(matches!(computed, Err(Error::Syntax(m)) if m.contains("computed term")));
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
    let (sccs, _) = strata(&program("calls_rust.rdl")).unwrap();
    let got = sccs.iter().map(|c| c.iter().cloned().collect::<Vec<_>>().join(",")).collect::<Vec<_>>().join(" | ");
    assert_eq!(
        got,
        "kind | nm | impl_fn | iparent | modn | modchild | item | uent | bound_here | glob | modpar | inmod | isvis | pubitem | \
         pubuse | mpub | local | tbase | troot | wants | islocal | seg1 | lwalk | lfound | lglob | trait_fn | \
         assoc,globx,impl_of,impl_trait,member,mexport,res,ures | callee | sfield | ttype | call_target,etype | lethasfn | \
         pathk2 | tpname | generic | exttype | rext | call_open | owner | calls | crate_fn_name | macro_open | calls_open | \
         isasync | reaches_open"
    );
}
