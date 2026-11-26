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
//! in `.rerail`: the statements are read off their tape (decision 0276) and lowered so: `F`'s declaration with the
//! lattice column, its rule without `!L`, and a dropped `L` left out (`lower_module_seeded_written`). `RERAIL_FIRST=off` leaves the shape as written, for measuring.
use super::super::check::{clause_body, clause_reads};
use super::super::clause::{At, L, LitAt, Reads, T};
use super::super::from_tape::{RrFile, S, Statements, Stmt};
use std::collections::{BTreeMap, BTreeSet};

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

/// A term as recognition reads it: a variable, a wildcard, `@strlen` of a variable, or anything else.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Term {
    Var(String),
    Wild,
    Strlen(String),
    Other,
}

/// A literal as recognition reads it, with the variables it reads.
#[derive(Debug)]
enum Lit {
    Atom { rel: String, args: Vec<Term>, neg: bool },
    Cmp { l: Term, op: &'static str, r: Term },
    Other,
}

/// A clause of one head and no edits, read off its tape.
struct Clause {
    rel: String,
    modal: bool,
    args: Vec<Term>,
    body: Vec<(Lit, Vec<String>)>,
}

fn term(f: &RrFile, a: &At) -> Term {
    match f.tv(a) {
        T::Var(v) => Term::Var(v),
        T::Wild => Term::Wild,
        T::Call { name, args, .. } if name == "strlen" && args.len() == 1 => match f.tv(&args[0]) {
            T::Var(v) => Term::Strlen(v),
            _ => Term::Other,
        },
        _ => Term::Other,
    }
}

fn lit(f: &RrFile, l: &LitAt) -> (Lit, Vec<String>) {
    let mut reads = Vec::new();
    f.lit_vars(l, &mut reads);
    let lit = match f.lit(l) {
        L::Atom { rel, args, neg } => Lit::Atom { rel, args: args.iter().map(|a| term(f, a)).collect(), neg },
        L::Cmp { l, op, r } => Lit::Cmp { l: term(f, &l), op, r: term(f, &r) },
        _ => Lit::Other,
    };
    (lit, reads)
}

/// An atom's arguments as variables, each once; None for anything else.
fn vars(args: &[Term]) -> Option<Vec<&str>> {
    let vs: Vec<&str> = args.iter().map(|a| if let Term::Var(v) = a { Some(v.as_str()) } else { None }).collect::<Option<_>>()?;
    let distinct: BTreeSet<&&str> = vs.iter().collect();
    (distinct.len() == vs.len()).then_some(vs)
}

/// An atom's arguments as variables (`Some`) and wildcards (`None`); None for anything else.
fn pattern(args: &[Term]) -> Option<Vec<Option<&str>>> {
    args.iter()
        .map(|a| match a {
            Term::Var(v) => Some(Some(v.as_str())),
            Term::Wild => Some(None),
            _ => None,
        })
        .collect()
}

/// Whether `args` are the head's variables `h`, each once, in order, with wildcards between them anywhere.
fn is_head_with_wilds(args: &[Term], h: &[&str]) -> bool {
    pattern(args).is_some_and(|p| p.iter().flatten().copied().eq(h.iter().copied()))
}

/// A comparison of a later-rule, as "w before v" in the text order (`"<"`), in length (`"len<"`), as long
/// (`"len="`); or `">"`, w after v. None for anything else.
fn comparison(l: &Lit, v: &str, w: &str) -> Option<&'static str> {
    let Lit::Cmp { l, op, r } = l else { return None };
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
    match (l, r) {
        (Term::Var(a), Term::Var(b)) => oriented(a, b).filter(|o| *o != "="),
        (Term::Strlen(a), Term::Strlen(b)) => match oriented(a, b)? {
            "<" => Some("len<"),
            "=" => Some("len="),
            _ => None,
        },
        _ => None,
    }
}

/// The later-rule's shape: `L(h…) :- X(h…), X(h… with w for the value), comparisons`. Its `X`, and its comparisons
/// as `comparison` names them, sorted.
fn later_rule(c: &Clause) -> Option<(String, Vec<&'static str>)> {
    let h = vars(&c.args)?;
    let (v, keys) = h.split_last()?;
    let mut atoms = Vec::new();
    let mut cmps = Vec::new();
    for (l, _) in &c.body {
        match l {
            Lit::Atom { rel, args, neg: false } => atoms.push((rel, args)),
            Lit::Cmp { .. } => cmps.push(l),
            _ => return None,
        }
    }
    let [(x1, a1), (x2, a2)] = atoms.as_slice() else { return None };
    if x1 != x2 {
        return None;
    }
    let (own, other) = if is_head_with_wilds(a1, &h) { (a1, a2) } else if is_head_with_wilds(a2, &h) { (a2, a1) } else { return None };
    // The other atom: the same wildcards, the keys in their places, and a fresh variable w in the value's.
    let (own, other) = (pattern(own)?, pattern(other)?);
    let at = own.iter().position(|a| *a == Some(*v))?;
    let Some(Some(w)) = other.get(at).copied() else { return None };
    let same_elsewhere = own.len() == other.len() && own.iter().zip(&other).enumerate().all(|(i, (a, b))| i == at || a == b);
    if !same_elsewhere || h.contains(&w) || !keys.iter().all(|k| own.contains(&Some(*k))) {
        return None;
    }
    let mut cs: Vec<&'static str> = cmps.iter().map(|c| comparison(c, v, w)).collect::<Option<_>>()?;
    cs.sort_unstable();
    Some(((*x1).clone(), cs))
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

/// The module's first-of-a-group relations, read off its statements' tapes.
pub fn recognize(statements: &Statements) -> Vec<First> {
    if std::env::var("RERAIL_FIRST").is_ok_and(|v| v == "off") {
        return Vec::new();
    }
    let mut decls: BTreeMap<String, (Vec<(String, String)>, Vec<String>)> = BTreeMap::new();
    let mut rules: BTreeMap<String, Vec<Clause>> = BTreeMap::new();
    // How many literals (and `.output`s) read each relation.
    let mut reads: BTreeMap<String, usize> = BTreeMap::new();
    let mut multi_head: BTreeSet<String> = BTreeSet::new();
    for st in statements {
        let (t, node) = st.tape();
        match st.what() {
            Some(S::Decl(_)) => {
                if let Ok((name, cols, tail)) = t.decl(node) {
                    decls.insert(name, (cols, tail));
                }
            }
            Some(S::Io { .. }) => {
                if let Ok((_, names, _)) = t.io(node) {
                    names.into_iter().for_each(|n| *reads.entry(n).or_default() += 1);
                }
            }
            Some(S::Clause) => {
                let f = st.file();
                let body = clause_body(f, node);
                let mut read = Vec::new();
                clause_reads(f, &body, &mut read);
                read.into_iter().for_each(|r| *reads.entry(r).or_default() += 1);
                let heads = f.heads_at(node);
                if heads.len() != 1 || st.has_edits() {
                    multi_head.extend(heads.into_iter().map(|h| h.rel));
                    continue;
                }
                let [h] = heads.as_slice() else { unreachable!("one head") };
                let clause = Clause { rel: h.rel.clone(), modal: h.modal.is_some(), args: h.args.iter().map(|a| term(f, a)).collect(), body: body.iter().map(|l| lit(f, l)).collect() };
                rules.entry(h.rel.clone()).or_default().push(clause);
            }
            // A test or fixture reads the module's relations too.
            Some(S::Test | S::Fixtures) => return Vec::new(),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for (first, frules) in &rules {
        let ([c], false) = (frules.as_slice(), multi_head.contains(first)) else { continue };
        if c.modal {
            continue;
        }
        let Some((cols, tail)) = decls.get(first) else { continue };
        if !tail.is_empty() || cols.iter().any(|(_, t)| t.ends_with("<>")) {
            continue;
        }
        let Some(h) = vars(&c.args) else { continue };
        let Some((v, _)) = h.split_last() else { continue };
        // `X(h…)` and `!L(h…)`, with other literals that do not read the value.
        let mut of = None;
        let mut later = None;
        let mut rest_reads_v = false;
        for (l, read) in &c.body {
            match l {
                Lit::Atom { rel, args, neg } if (!neg && is_head_with_wilds(args, &h)) || (*neg && vars(args).as_ref() == Some(&h)) => match (neg, &of, &later) {
                    (false, None, _) => of = Some(rel),
                    (true, _, None) => later = Some(rel),
                    _ => rest_reads_v = true,
                },
                _ => rest_reads_v |= read.iter().any(|r| r == v),
            }
        }
        let (Some(of), Some(later), false) = (of, later, rest_reads_v) else { continue };
        let Some(lrules) = rules.get(later.as_str()) else { continue };
        if multi_head.contains(later.as_str()) || lrules.iter().any(|c| c.modal) {
            continue;
        }
        let Some(shapes) = lrules.iter().map(later_rule).collect::<Option<Vec<_>>>() else { continue };
        if shapes.iter().any(|(x, _)| x != of) {
            continue;
        }
        let Some(order) = order_of(&shapes.iter().map(|(_, c)| c.clone()).collect::<Vec<_>>()) else { continue };
        let typed = |r: &str| decls.get(r).and_then(|(c, _)| c.last()).is_some_and(|(_, t)| typed_for(order, t));
        // F's head copies the value from X, so F's column is its type.
        if !typed(first) {
            continue;
        }
        // F's rule reads L once; L's own rules read it never. Anything else keeps it.
        let drop_later = reads.get(later.as_str()).copied().unwrap_or(0) == 1;
        found.push(First { first: first.clone(), later: later.clone(), of: of.clone(), order, drop_later });
    }
    found
}

/// How the lowering reads the module for `found` (`lower_module_seeded_written`): each `F`'s value column is its
/// lattice, its rule is read without `!L`, and a dropped `L`'s declaration and rules are left out.
#[derive(Clone, Debug, Default)]
pub struct Firsts {
    /// `F`'s lattice column, by `F`.
    pub column: BTreeMap<String, &'static str>,
    /// `L`, by `F`: the negation `F`'s rule is read without.
    pub without: BTreeMap<String, String>,
    /// The later-relations left out.
    pub dropped: BTreeSet<String>,
}

impl Firsts {
    pub fn of(found: &[First]) -> Firsts {
        Firsts {
            column: found.iter().map(|f| (f.first.clone(), f.order.column())).collect(),
            without: found.iter().map(|f| (f.first.clone(), f.later.clone())).collect(),
            dropped: found.iter().filter(|f| f.drop_later).map(|f| f.later.clone()).collect(),
        }
    }

    /// Declaration `name`'s columns, as the module is read: `None` for a dropped later-relation.
    pub fn columns(&self, name: &str, mut cols: Vec<(String, String)>) -> Option<Vec<(String, String)>> {
        if self.dropped.contains(name) {
            return None;
        }
        if let (Some(c), Some(last)) = (self.column.get(name), cols.last_mut()) {
            last.1 = (*c).to_owned();
        }
        Some(cols)
    }
}
