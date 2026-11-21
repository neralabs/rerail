//! Adapt logical rule witnesses to the shared contribution storage proof.
use super::super::{scalars::Expression as E, storage};
use super::*;
pub(super) fn analyze(v: &Version, r: RuleRef<'_>) -> storage::Projection {
    let mut variables = BTreeMap::new();
    let mut width = 0;
    for literal in r.body() {
        if let Lit::Atom(atom) = literal.view()
            && !atom.neg()
        {
            for (c, arg) in atom.args().enumerate() {
                if let TermRef::Var(s) = arg {
                    variables.entry(s).or_insert(width + c);
                }
            }
            width += atom.arity();
        }
    }
    let term = |t: TermRef<'_>| match t {
        TermRef::Var(s) => variables.get(&s).map_or_else(|| E::Opaque(format!("unbound {}", name(v, r, s))), |&c| E::Column(c)),
        TermRef::Int(n) => E::Integer(n),
        TermRef::Branded(a) => E::Integer(a.value()),
        TermRef::Str(a) => E::Text(v.text(a).to_owned()),
        other => E::Opaque(v.show_term(r, other)),
    };
    let mut predicates = vec![];
    for literal in r.body() {
        match literal.view() {
            Lit::Atom(atom) if !atom.neg() => {
                if atom.args().any(|t| matches!(t, TermRef::Call(_, _))) {
                    predicates.push(E::Opaque("computed atom argument".into()));
                }
            }
            Lit::Cmp { l, op, r: right } => predicates.push(E::Apply(
                match op {
                    CmpOp::Eq => "Equal",
                    CmpOp::Ne => "NotEqual",
                    CmpOp::Lt => "Less",
                    CmpOp::Le => "LessOrEqual",
                    CmpOp::Gt => "Greater",
                    CmpOp::Ge => "GreaterOrEqual",
                }
                .into(),
                vec![term(l), term(right)],
            )),
            _ => predicates.push(E::Opaque(v.show_lit(r, literal))),
        }
    }
    storage::projection(width.max(1), r.head().args().map(term).collect(), predicates)
}
