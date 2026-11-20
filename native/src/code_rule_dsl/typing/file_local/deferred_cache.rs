use crate::code_rule_dsl::logic::front::{AggOp, Atom, Literal, Name, Program, Rule, Term};

/// Cache a reused binary product only while the exact sizes of both factors fit its allowance.
/// The guard depends only on the factors. Maintenance can cross the threshold in either direction.
pub(super) fn install(
    p: &mut Program,
    rule: &Rule,
    factors: &[Atom],
    readers: usize,
    limit: usize,
) -> (Name, Atom) {
    let base = super::helper_name(
        "rr_pair_cache_",
        rule.head.rel.as_str(),
        &rule.to_string(),
        |n| {
            ["small", "pairs"]
                .iter()
                .any(|s| p.decls.contains_key(format!("{n}_{s}").as_str()))
        },
        &mut Default::default(),
    );
    let named = |suffix: &str, args: &[Term]| Atom {
        rel: format!("{base}_{suffix}").into(),
        args: args.to_vec().into(),
        neg: false,
        gated: false,
    };
    // This pass runs after rr::lower::lower_boolean_relations. Use its physical unit encoding.
    let small = named("small", &[Term::Int(1)]);
    let pairs = named("pairs", &rule.head.args);
    let pair_cols = p.decls[&rule.head.rel].clone();
    p.nullary.insert(small.rel.to_string());
    p.decls.insert(small.rel.clone(), [("rr_unit".into(), "int".into())].into());
    p.decls.insert(pairs.rel.clone(), pair_cols);
    for atom in [&small, &pairs] {
        p.qualifiers
            .entry(atom.rel.to_string())
            .or_default()
            .insert("no_inline".into());
    }
    let mut guard = Vec::new();
    for (i, factor) in factors.iter().enumerate() {
        let var = format!("rr_count_{i}");
        let mut all = factor.clone();
        all.args.make_mut().fill(Term::Wild);
        guard.push(Literal::Agg {
            var,
            op: AggOp::Count,
            over: None,
            atom: all,
        });
    }
    let admit = Term::Call(
        "rr_admit_pairs".into(),
        [
            Term::Var("rr_count_0".into()),
            Term::Var("rr_count_1".into()),
            Term::Int(readers as i64),
            Term::Int(limit as i64),
        ]
        .into(),
    );
    guard.push(Literal::Cmp {
        l: admit,
        op: "=",
        r: Term::Int(1),
    });
    p.rules.extend([
        Rule {
            head: small.clone(),
            body: guard.into(),
        },
        Rule {
            head: pairs.clone(),
            body: [
                Literal::Atom(small.clone()),
                Literal::Atom(factors[0].clone()),
                Literal::Atom(factors[1].clone()),
            ]
            .into(),
        },
    ]);
    (pairs.rel, small)
}
