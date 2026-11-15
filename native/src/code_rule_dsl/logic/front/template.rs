//! A rule's template: its variables numbered in order of first appearance, its relations named by a label. Two
//! relations whose rules have equal templates are one relation (`simplify`'s `congruence`), and a helper whose
//! template one made before has is that helper (`rr::lower`).

use super::{AggOp, Atom, Literal, Name, Rule, Term};

/// `rule` with its variables numbered in order of first appearance and its head relation `@`: two rules of equal
/// templates are one rule of two relations (`rr::lower`'s helpers, made once by theirs; `simplify::project_existentials`' parts).
pub fn template(rule: &Rule, names: &CanonicalNames) -> String {
    canonical(rule, &|r| Name::new(r), names)
}

/// A rule with every relation named by `label` and its variables numbered in order of first appearance.
///
/// Its variables are numbered by a scan of the few names seen so far, each `v<k>` one of `names`, made once for the
/// caller's rules: a map and a `format!` per variable, and each name interned again (a lock the pool's threads share),
/// were most of `congruence`'s templates, 9% of a cold batch's main thread.
pub fn canonical(rule: &Rule, label: &dyn Fn(&str) -> Name, names_made: &CanonicalNames) -> String {
    let mut names: Vec<String> = Vec::new();
    let mut name = |v: &str| -> Name {
        let k = names.iter().position(|n| n == v).unwrap_or_else(|| {
            names.push(v.to_owned());
            names.len() - 1
        });
        names_made.var(k)
    };
    fn term(t: &Term, name: &mut dyn FnMut(&str) -> Name) -> Term {
        match t {
            Term::Var(v) => Term::Var(name(v)),
            Term::Call(f, args) => Term::Call(f.clone().into(), args.iter().map(|a| term(a, name)).collect()),
            t => t.clone(),
        }
    }
    let atom = |a: &Atom, name: &mut dyn FnMut(&str) -> Name| Atom { rel: label(&a.rel), args: a.args.iter().map(|t| term(t, name)).collect(), ..a.clone() };
    // The head is the relation the signature is of: `@`, so two relations' rules compare by their bodies.
    let head = Atom { rel: names_made.head.clone(), ..atom(&rule.head, &mut name) };
    let body: Vec<Literal> = rule
        .body
        .iter()
        .map(|l| match l {
            Literal::Atom(a) => Literal::Atom(atom(a, &mut name)),
            Literal::Cmp { l, op, r } => Literal::Cmp { l: term(l, &mut name), op, r: term(r, &mut name) },
            Literal::Agg { var, op, over, atom: a } => {
                let var = name(var).as_str().to_owned();
                let over = over.as_ref().map(|o| name(o).as_str().to_owned());
                let op = match op {
                    AggOp::Join { by, sep } => AggOp::Join { by: by.iter().map(|k| name(k).as_str().to_owned()).collect(), sep: sep.clone() },
                    op => op.clone(),
                };
                Literal::Agg { var, op, over, atom: atom(a, &mut name) }
            }
            Literal::Nearest { n, a, kindset } => Literal::Nearest { n: name(n).as_str().to_owned(), a: name(a).as_str().to_owned(), kindset: kindset.clone() },
            Literal::Ast { document, node, matcher, captures } => Literal::Ast {
                document: name(document).as_str().to_owned(),
                node: name(node).as_str().to_owned(),
                matcher: matcher.clone(),
                captures: captures.iter().map(|(c, v)| (c.clone(), name(v).as_str().to_owned())).collect(),
            },
        })
        .collect();
    Rule { head, body: body.into() }.to_string()
}

/// The names `canonical` writes, made once by its caller: the variables `v0`, `v1`, …, the head's `@`, and the
/// placeholder of the relations a template reads.
pub struct CanonicalNames {
    vars: Vec<Name>,
    head: Name,
    pub placeholder: Name,
}

impl CanonicalNames {
    pub fn new() -> Self {
        CanonicalNames { vars: (0..64).map(|k| Name::new(&format!("v{k}"))).collect(), head: Name::new("@"), placeholder: Name::new("\u{1}") }
    }

    /// The variable name `v<k>`.
    pub fn var(&self, k: usize) -> Name {
        self.vars.get(k).cloned().unwrap_or_else(|| Name::new(&format!("v{k}")))
    }
}
