//! "The first of a group by an order" (t50 step 2, decision 0515): a relation written as a group's rows that no other
//! row of the group precedes,
//!
//! ```text
//! F(k…, v) :- X(k…, v), !L(k…, v), <literals not reading v>.
//! L(k…, v) :- X(k…, v), X(k…, w), w < v.                                   // min (`w > v`: max)
//! L(k…, v) :- X(k…, v), X(k…, w), @strlen(w) < @strlen(v).                 // shortlex min: shorter first,
//! L(k…, v) :- X(k…, v), X(k…, w), @strlen(w) = @strlen(v), w < v.          // then the least text
//! ```
//!
//! is read as a lattice relation: `F`'s value column joins its key's rows under the order (`min<>`, `max<>`,
//! `shortlex_min<>`), and `F`'s rule is `X` and its other literals. The self-join `L` made a row for every pair of a
//! group's rows; the lattice keeps one a key, in one pass. `L` is dropped when nothing else reads it. Nothing changes
//! in `.rerail`. `RERAIL_FIRST=off` leaves the shape as written, for measuring.
//!
//! The statements are their tapes' nodes, read in place (decision 0276, stage 6 A): what is recognized is said as
//! positions, which the lowering reads past (`Firsts`): `L`'s statements are left out, `F`'s value column is read as
//! its lattice, and `F`'s `!L` literal is not read.
use super::super::check::clause_body;
use super::super::clause::{At, HeadAt, L, LitAt, Reads, T};
use super::super::from_tape::{RrFile, S, Stmt};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// The order a later-relation states, and the lattice that keeps its first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    Min,
    Max,
    Shortlex,
}

impl Order {
    /// The lattice column type `F`'s value column becomes.
    pub fn column(self) -> &'static str {
        match self {
            Order::Min => "min<>",
            Order::Max => "max<>",
            Order::Shortlex => "shortlex_min<>",
        }
    }
}

/// A recognized relation: `first` keeps the group's first of `of` by `order`; `later`, its later-relation, is dropped
/// when `drop_later`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct First {
    pub first: String,
    pub later: String,
    pub of: String,
    pub order: Order,
    pub drop_later: bool,
}

/// A module's first-of-a-group relations, and the positions the lowering reads past for them.
#[derive(Clone, Debug, Default)]
pub struct Firsts {
    pub found: Vec<First>,
    /// Each `F`'s `!L` literal: its file and node.
    pub unread: Vec<(Arc<RrFile>, u32)>,
}

impl Firsts {
    /// The lattice column type `rel`'s value column is read as, when `rel` is a recognized `F`.
    pub fn column(&self, rel: &str) -> Option<&'static str> {
        self.found.iter().find(|f| f.first == rel).map(|f| f.order.column())
    }

    /// Whether statement `st` is a dropped `L`'s: its declaration or one of its rules.
    pub fn drops(&self, st: &Stmt) -> bool {
        let dropped = |rel: &str| self.found.iter().any(|f| f.drop_later && f.later == rel);
        match st.what() {
            Some(S::Decl(name)) => dropped(name),
            Some(S::Clause) => matches!(st.heads().as_slice(), [h] if dropped(h)),
            _ => false,
        }
    }

    /// Whether literal node `node` of `file` is one the lowering reads past.
    pub fn unread(&self, file: &Arc<RrFile>, node: u32) -> bool {
        self.unread.iter().any(|(f, n)| *n == node && Arc::ptr_eq(f, file))
    }
}

/// Term `a` as a variable, if it is one.
fn var(f: &RrFile, a: &At) -> Option<String> {
    if let T::Var(v) = f.tv(a) { Some(v) } else { None }
}

/// Arguments that are distinct variables.
fn vars(f: &RrFile, args: &[At]) -> Option<Vec<String>> {
    let vs: Vec<String> = args.iter().map(|a| var(f, a)).collect::<Option<_>>()?;
    let distinct: BTreeSet<&String> = vs.iter().collect();
    (distinct.len() == vs.len()).then_some(vs)
}

/// An atom's arguments as variables (`Some`) and wildcards (`None`); None for anything else.
fn pattern(f: &RrFile, args: &[At]) -> Option<Vec<Option<String>>> {
    args.iter()
        .map(|a| match f.tv(a) {
            T::Var(v) => Some(Some(v)),
            T::Wild => Some(None),
            _ => None,
        })
        .collect()
}

/// Whether `args` are the head's variables `h`, each once, in order, with wildcards between them anywhere.
fn is_head_with_wilds(f: &RrFile, args: &[At], h: &[String]) -> bool {
    pattern(f, args).is_some_and(|p| p.iter().flatten().eq(h.iter()))
}

fn strlen_of(f: &RrFile, a: &At) -> Option<String> {
    match f.tv(a) {
        T::Call { name, args, .. } if name == "strlen" && args.len() == 1 => var(f, &args[0]),
        _ => None,
    }
}

/// A comparison of a later-rule: "w before v" in the text order (`"<"`), in length (`"len<"`), as long (`"len="`);
/// or `">"`, w after v. None for anything else.
fn comparison(f: &RrFile, l: &L, v: &str, w: &str) -> Option<&'static str> {
    let L::Cmp { l, op, r } = l else { return None };
    // `a op b` as "w op' v".
    let oriented = |a: &str, b: &str| -> Option<&'static str> {
        let flipped = match *op {
            "<" => ">",
            ">" => "<",
            "=" => "=",
            _ => return None,
        };
        if (a, b) == (w, v) {
            Some(match *op {
                "<" => "<",
                ">" => ">",
                _ => "=",
            })
        } else if (a, b) == (v, w) {
            Some(flipped)
        } else {
            None
        }
    };
    if let (Some(a), Some(b)) = (var(f, l), var(f, r)) {
        return oriented(&a, &b).filter(|o| *o != "=");
    }
    if let (Some(a), Some(b)) = (strlen_of(f, l), strlen_of(f, r)) {
        return match oriented(&a, &b)? {
            "<" => Some("len<"),
            "=" => Some("len="),
            _ => None,
        };
    }
    None
}

/// The later-rule's shape: `L(h…) :- X(h…), X(h… with w for the value), comparisons`. Its `X`, and its comparisons
/// as `comparison` names them, sorted.
fn later_rule(f: &RrFile, head: &HeadAt, body: &[LitAt]) -> Option<(String, Vec<&'static str>)> {
    let h = vars(f, &head.args)?;
    let (v, keys) = h.split_last()?;
    let mut atoms = Vec::new();
    let mut cmps = Vec::new();
    for l in body {
        match f.lit(l) {
            L::Atom { rel, args, neg: false } => atoms.push((rel, args)),
            c @ L::Cmp { .. } => cmps.push(c),
            _ => return None,
        }
    }
    let [(x1, a1), (x2, a2)] = atoms.as_slice() else { return None };
    if x1 != x2 {
        return None;
    }
    let (own, other) = if is_head_with_wilds(f, a1, &h) {
        (a1, a2)
    } else if is_head_with_wilds(f, a2, &h) {
        (a2, a1)
    } else {
        return None;
    };
    // The other atom: the same wildcards, the keys in their places, and a fresh variable w in the value's.
    let (own, other) = (pattern(f, own)?, pattern(f, other)?);
    let at = own.iter().position(|a| a.as_ref() == Some(v))?;
    let Some(Some(w)) = other.get(at).cloned() else { return None };
    let same_elsewhere = own.len() == other.len() && own.iter().zip(&other).enumerate().all(|(i, (a, b))| i == at || a == b);
    if !same_elsewhere || h.contains(&w) || !keys.iter().all(|k| own.contains(&Some(k.clone()))) {
        return None;
    }
    let mut cs: Vec<&'static str> = cmps.iter().map(|c| comparison(f, c, v, &w)).collect::<Option<_>>()?;
    cs.sort_unstable();
    Some((x1.clone(), cs))
}

/// The order a later-relation's rules state together: each rule's comparisons, sorted, as a set.
fn order_of(rules: &[Vec<&'static str>]) -> Option<Order> {
    let mut rs: Vec<&Vec<&'static str>> = rules.iter().collect();
    rs.sort();
    match rs.as_slice() {
        [r] if r.as_slice() == ["<"] => Some(Order::Min),
        [r] if r.as_slice() == [">"] => Some(Order::Max),
        [a, b] if a.as_slice() == ["<", "len="] && b.as_slice() == ["len<"] => Some(Order::Shortlex),
        _ => None,
    }
}

/// The column types the order needs: numbers for min and max, symbols for shortlex.
fn typed_for(order: Order, ty: &str) -> bool {
    match order {
        Order::Min | Order::Max => matches!(ty, "number" | "unsigned"),
        Order::Shortlex => ty == "symbol",
    }
}

/// How many atoms of `rel` literal `l` reads, in its terms' aggregates and its group's alternatives too.
fn atoms_of(f: &RrFile, l: &LitAt, rel: &str) -> usize {
    fn term(f: &RrFile, a: &At, rel: &str) -> usize {
        match f.tv(a) {
            T::Agg { over, by, body, .. } => over.iter().chain(&by).map(|a| term(f, a, rel)).sum::<usize>() + body.iter().map(|l| atoms_of(f, l, rel)).sum::<usize>(),
            T::Call { args, .. } => args.iter().map(|a| term(f, a, rel)).sum(),
            T::Bin { l, r, .. } => term(f, &l, rel) + term(f, &r, rel),
            T::Neg(x) | T::Named(_, x) => term(f, &x, rel),
            _ => 0,
        }
    }
    match f.lit(l) {
        L::Atom { rel: r, args, .. } => usize::from(r == rel) + args.iter().map(|a| term(f, a, rel)).sum::<usize>(),
        L::Cmp { l, r, .. } => term(f, &l, rel) + term(f, &r, rel),
        L::Pattern { subject, .. } => term(f, &subject, rel),
        L::Group { alts, .. } => alts.iter().flatten().map(|l| atoms_of(f, l, rel)).sum(),
    }
}

/// A rule: its statement's place among the module's, its head and its body.
struct Rule<'s> {
    at: usize,
    st: &'s Stmt,
    head: HeadAt,
    body: Vec<LitAt>,
}

/// The module's first-of-a-group relations.
pub fn recognize(statements: &[Stmt]) -> Firsts {
    let mut out = Firsts::default();
    if std::env::var("RERAIL_FIRST").is_ok_and(|v| v == "off") {
        return out;
    }
    let mut decls: BTreeMap<String, (Vec<(String, String)>, Vec<String>)> = BTreeMap::new();
    let mut rules: BTreeMap<String, Vec<Rule<'_>>> = BTreeMap::new();
    let mut multi_head: BTreeSet<String> = BTreeSet::new();
    for (at, st) in statements.iter().enumerate() {
        let (t, node) = st.tape();
        match st.what() {
            Some(S::Decl(_)) => {
                if let Ok((name, cols, tail)) = t.decl(node) {
                    decls.insert(name, (cols, tail));
                }
            }
            Some(S::Clause) => {
                let f = st.file();
                let heads = f.heads_at(node);
                if heads.len() != 1 || st.has_edits() {
                    multi_head.extend(heads.into_iter().map(|h| h.rel));
                    continue;
                }
                let body = clause_body(f, node);
                let head = heads.into_iter().next().expect("one head");
                rules.entry(head.rel.clone()).or_default().push(Rule { at, st, head, body });
            }
            // A test or fixture reads the module's relations too.
            Some(S::Test | S::Fixtures) => return out,
            _ => {}
        }
    }
    for (first, frules) in &rules {
        let ([fr], false) = (frules.as_slice(), multi_head.contains(first)) else { continue };
        let f = fr.st.file();
        if fr.head.modal.is_some() {
            continue;
        }
        let Some((cols, tail)) = decls.get(first) else { continue };
        if !tail.is_empty() || cols.iter().any(|(_, t)| t.ends_with("<>")) {
            continue;
        }
        let Some(h) = vars(f, &fr.head.args) else { continue };
        let Some(v) = h.last() else { continue };
        // `X(h…)` and `!L(h…)`, with other literals that do not read the value.
        let (mut of, mut later, mut negation) = (None, None, None);
        let mut rest_reads_v = false;
        for l in &fr.body {
            match f.lit(l) {
                L::Atom { rel, args, neg } if (!neg && is_head_with_wilds(f, &args, &h)) || (neg && vars(f, &args).as_ref() == Some(&h)) => match (neg, &of, &later) {
                    (false, None, _) => of = Some(rel),
                    (true, _, None) => {
                        later = Some(rel);
                        negation = Some(l);
                    }
                    _ => rest_reads_v = true,
                },
                _ => {
                    let mut vs = Vec::new();
                    f.lit_vars(l, &mut vs);
                    rest_reads_v |= vs.contains(v);
                }
            }
        }
        let (Some(of), Some(later), Some(&LitAt::Node(negation, _)), false) = (of, later, negation, rest_reads_v) else { continue };
        let Some(lrules) = rules.get(&later) else { continue };
        if multi_head.contains(&later) || lrules.iter().any(|r| r.head.modal.is_some()) {
            continue;
        }
        let Some(shapes) = lrules.iter().map(|r| later_rule(r.st.file(), &r.head, &r.body)).collect::<Option<Vec<_>>>() else { continue };
        if shapes.iter().any(|(x, _)| *x != of) {
            continue;
        }
        let Some(order) = order_of(&shapes.iter().map(|(_, c)| c.clone()).collect::<Vec<_>>()) else { continue };
        // F's head copies the value from X, so F's column is its type.
        if !decls.get(first).and_then(|(c, _)| c.last()).is_some_and(|(_, t)| typed_for(order, t)) {
            continue;
        }
        // L is dropped when F's rule reads it once, its own rules never, and no other statement names it (an
        // `.output`, a relation given as an argument, an aggregate's body).
        let own: BTreeSet<usize> = lrules.iter().map(|r| r.at).chain([fr.at]).collect();
        let named_elsewhere = statements.iter().enumerate().filter(|(i, st)| !own.contains(i) && !matches!(st.what(), Some(S::Decl(_)))).any(|(_, st)| {
            let (t, node) = st.tape();
            if let Some(S::Io { .. }) = st.what() {
                return t.io(node).is_ok_and(|(_, names, _)| names.contains(&later));
            }
            let mut names = BTreeSet::new();
            t.names_in(node, &mut names);
            names.contains(&later)
        });
        let read_by_own = lrules.iter().any(|r| r.body.iter().any(|l| atoms_of(r.st.file(), l, &later) > 0));
        let drop_later = !named_elsewhere && !read_by_own && fr.body.iter().map(|l| atoms_of(f, l, &later)).sum::<usize>() == 1;
        out.unread.push((Arc::clone(f), negation));
        out.found.push(First { first: first.clone(), later, of, order, drop_later });
    }
    out
}
