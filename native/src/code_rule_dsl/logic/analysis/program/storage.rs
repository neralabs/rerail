//! Adapt logical rule witnesses to the shared contribution storage proof.
use super::super::{scalars::Expression as E, storage};
use super::*;
pub(super) fn analyze(rule: &front::Rule) -> storage::Projection {
    let mut variables = BTreeMap::new();
    let mut width = 0;
    for literal in &rule.body {
        if let Literal::Atom(atom) = literal
            && !atom.neg
        {
            for (c, arg) in atom.args.iter().enumerate() {
                if let Term::Var(v) = arg {
                    variables.entry(v.clone()).or_insert(width + c);
                }
            }
            width += atom.args.len();
        }
    }
    let term = |t: &Term| match t {
        Term::Var(v) => variables
            .get(v)
            .map_or_else(|| E::Opaque(format!("unbound {v}")), |&c| E::Column(c)),
        Term::Int(n) => E::Integer(*n),
        Term::Str(s) => E::Text(s.to_string()),
        _ => E::Opaque(format!("{t:?}")),
    };
    let mut predicates = vec![];
    for literal in &rule.body {
        match literal {
            Literal::Atom(atom) if !atom.neg => {
                if atom.args.iter().any(|t| matches!(t, Term::Call(_, _))) {
                    predicates.push(E::Opaque("computed atom argument".into()));
                }
            }
            Literal::Cmp { l, op, r } => predicates.push(E::Apply(
                match *op {
                    "=" => "Equal",
                    "!=" => "NotEqual",
                    "<" => "Less",
                    "<=" => "LessOrEqual",
                    ">" => "Greater",
                    ">=" => "GreaterOrEqual",
                    other => other,
                }
                .into(),
                vec![term(l), term(r)],
            )),
            _ => predicates.push(E::Opaque(format!("{literal:?}"))),
        }
    }
    storage::projection(
        width.max(1),
        rule.head.args.iter().map(term).collect(),
        predicates,
    )
}
