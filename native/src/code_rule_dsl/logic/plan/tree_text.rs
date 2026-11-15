//! The tree's `Debug` text of a rule and of a relation's fields (`front::Rule`, `front::Native`, …), written from a
//! version's handles. A relation's identity (`plan::identities`, decision 0103) is a digest of that text, and it keys
//! the strata a store keeps, so the text must be the tree's byte for byte. Each type here mirrors a tree type field
//! by field, under the same names, so its derived `Debug` writes what the tree's derived `Debug` writes (a `Name` or
//! a `List` writes as its `str` or slice does). Until a version's own digest keys them (decision 0173, step 6).
use super::super::program_image::{AggRef, AtomRef, Cell, Lit, TermRef};
use super::body::Body;
use super::fields::Fields;

#[derive(Debug)]
enum Term<'v> {
    Var(&'v str),
    Wild,
    Int(i64),
    Str(&'v str),
    Top,
    Call(&'v str, Vec<Term<'v>>),
}

#[derive(Debug)]
struct Atom<'v> {
    rel: &'v str,
    args: Vec<Term<'v>>,
    neg: bool,
    gated: bool,
}

#[derive(Debug)]
enum AggOp<'v> {
    Count,
    Min,
    Max,
    Sum,
    Product,
    Join { by: Vec<&'v str>, sep: &'v str },
}

#[derive(Debug)]
enum Literal<'v> {
    Atom(Atom<'v>),
    Cmp { l: Term<'v>, op: &'static str, r: Term<'v> },
    Agg { var: &'v str, op: AggOp<'v>, over: Option<&'v str>, atom: Atom<'v> },
    Nearest { n: &'v str, a: &'v str, kindset: &'v str },
    Ast { document: &'v str, node: &'v str, matcher: &'v str, captures: Vec<(&'v str, &'v str)> },
}

#[derive(Debug)]
struct Rule<'v> {
    head: Atom<'v>,
    body: Vec<Literal<'v>>,
}

fn term<'v>(b: &Body<'v>, t: TermRef<'_>) -> Term<'v> {
    match t {
        TermRef::Var(s) => Term::Var(b.var(s)),
        TermRef::Wild => Term::Wild,
        TermRef::Int(i) => Term::Int(i),
        TermRef::Str(a) => Term::Str(b.v.text(a)),
        TermRef::Top => Term::Top,
        TermRef::Call(f, args) => Term::Call(b.v.text(f), args.map(|a| term(b, a)).collect()),
    }
}

fn atom<'v>(b: &Body<'v>, a: AtomRef<'_>) -> Atom<'v> {
    Atom { rel: b.rel(a), args: a.args().map(|t| term(b, t)).collect(), neg: a.neg(), gated: a.gated() }
}

/// Rule `b` as the tree's `Debug` writes it (`format!("{rule:?}")`).
pub fn rule(b: &Body<'_>) -> String {
    let body = (0..b.len())
        .map(|i| match b.view(i) {
            Lit::Atom(a) => Literal::Atom(atom(b, a)),
            Lit::Cmp { l, op, r } => Literal::Cmp { l: term(b, l), op: op.as_str(), r: term(b, r) },
            Lit::Agg { var, op, over, atom: a } => Literal::Agg {
                var: b.var(var),
                op: match op {
                    AggRef::Count => AggOp::Count,
                    AggRef::Min => AggOp::Min,
                    AggRef::Max => AggOp::Max,
                    AggRef::Sum => AggOp::Sum,
                    AggRef::Product => AggOp::Product,
                    AggRef::Join { by, sep } => AggOp::Join { by: by.map(|s| b.var(s)).collect(), sep: b.v.text(sep) },
                },
                over: over.map(|o| b.var(o)),
                atom: atom(b, a),
            },
            Lit::Nearest { n, a, kindset } => Literal::Nearest { n: b.var(n), a: b.var(a), kindset: b.v.text(kindset) },
            Lit::Ast { document, node, matcher, captures } => Literal::Ast {
                document: b.var(document),
                node: b.var(node),
                matcher: b.v.text(matcher),
                captures: captures.map(|(k, v)| (b.v.text(k), b.var(v))).collect(),
            },
        })
        .collect();
    format!("{:?}", Rule { head: atom(b, b.head()), body })
}

#[derive(Debug)]
enum Value<'v> {
    Int(i64),
    Str(&'v str),
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug)]
enum Native<'v> {
    DfsFirst { edge: &'v str, target: &'v str, roots: Option<&'v str> },
    MatchAll { input: &'v str, pattern: &'v str, groups: Vec<&'v str>, positions: bool },
    Components { edge: &'v str, own: &'v str },
    Reach { seed: &'v str, edge: &'v str },
    ReachThrough { reach: &'v str, map: &'v str },
    Python { kind: &'v str, input: &'v str, targets: Option<&'v str> },
    PythonCandidates { input: &'v str, targets: &'v str, context: &'v str, member: &'v str },
}

#[derive(Debug)]
struct LatticeDecl<'v> {
    keys: usize,
    ty: &'v str,
    merge: Option<&'v str>,
    widen: Option<u32>,
}

fn decl<'v>(f: &Fields<'v>, rel: &str) -> Option<Vec<(&'v str, &'v str)>> {
    f.columns(rel)
}

fn facts<'v>(f: &Fields<'v>, rel: &str) -> Option<Vec<Vec<Value<'v>>>> {
    let v = f.v;
    f.facts(rel).map(|fs| fs.rows().map(|row| row.map(|c| match c {
        Cell::Int(i) => Value::Int(i),
        Cell::Str(a) => Value::Str(v.text(a)),
    }).collect()).collect())
}

fn native<'v>(f: &Fields<'v>, rel: &str) -> Option<Native<'v>> {
    use super::super::program_image::NativeSpec as S;
    let (r, t) = (|x| f.rel_name(x), |x| f.text(x));
    f.native(rel).map(|n| match &n.spec {
        S::DfsFirst { edge, target, roots } => Native::DfsFirst { edge: r(*edge), target: r(*target), roots: roots.map(r) },
        S::MatchAll { input, pattern, groups, positions } => Native::MatchAll { input: r(*input), pattern: t(*pattern), groups: groups.iter().map(|&g| t(g)).collect(), positions: *positions },
        S::Components { edge, own } => Native::Components { edge: r(*edge), own: r(*own) },
        S::Reach { seed, edge } => Native::Reach { seed: r(*seed), edge: r(*edge) },
        S::ReachThrough { reach, map } => Native::ReachThrough { reach: r(*reach), map: r(*map) },
        S::Python { kind, input, targets } => Native::Python { kind: t(*kind), input: r(*input), targets: targets.map(r) },
        S::PythonCandidates { input, targets, context, member } => Native::PythonCandidates { input: r(*input), targets: r(*targets), context: t(*context), member: t(*member) },
    })
}

fn lattice<'v>(f: &Fields<'v>, rel: &str) -> Option<LatticeDecl<'v>> {
    f.lattice_decl(rel).map(|l| LatticeDecl { keys: l.keys, ty: f.text(l.ty), merge: l.merge.map(|m| f.text(m)), widen: l.widen })
}

/// A relation's fields as `identities` writes them, the tree's `Debug` of (its declaration, its facts, its native,
/// its lattice, whether it is nullary).
pub fn shape(f: &Fields<'_>, rel: &str) -> String {
    format!("{:?}", (decl(f, rel), facts(f, rel), native(f, rel), lattice(f, rel), f.nullary(rel)))
}

/// A relation no rule derives as `leaf_identity` writes it: the tree's `Debug` of its declaration, then of its facts.
pub fn leaf(f: &Fields<'_>, rel: &str) -> String {
    format!("{:?}\n{:?}", decl(f, rel), facts(f, rel))
}
