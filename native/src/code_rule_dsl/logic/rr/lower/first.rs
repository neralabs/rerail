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
use rerail_rr_syntax::syntax::{Head, Item, Lit, Statement, Term};
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

fn term_reads(t: &Term, v: &str) -> bool {
    match t {
        Term::Var(x) => x == v,
        Term::Call { args, .. } => args.iter().any(|a| term_reads(a, v)),
        Term::Bin { l, r, .. } => term_reads(l, v) || term_reads(r, v),
        _ => false,
    }
}

fn lit_reads(l: &Lit, v: &str) -> bool {
    match l {
        Lit::Atom { args, .. } => args.iter().any(|a| term_reads(a, v)),
        Lit::Cmp { l, r, .. } => term_reads(l, v) || term_reads(r, v),
        Lit::Pattern { subject, .. } => term_reads(subject, v),
        Lit::Group { alts, .. } => alts.iter().flatten().any(|l| lit_reads(l, v)),
    }
}

fn var(t: &Term) -> Option<&str> {
    if let Term::Var(v) = t { Some(v) } else { None }
}

fn strlen_of(t: &Term) -> Option<&str> {
    match t {
        Term::Call { name, args, .. } if name == "strlen" && args.len() == 1 => var(&args[0]),
        _ => None,
    }
}

/// A comparison of a later-rule, as `(what, before)`: "w before v" in the text order (`"<"`), in length (`"len<"`), as
/// long (`"len="`); or `">"`, w after v. None for anything else.
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
    if let (Some(a), Some(b)) = (var(l), var(r)) {
        return oriented(a, b).filter(|o| *o != "=");
    }
    if let (Some(a), Some(b)) = (strlen_of(l), strlen_of(r)) {
        return match oriented(a, b)? {
            "<" => Some("len<"),
            "=" => Some("len="),
            _ => None,
        };
    }
    None
}

/// The later-rule's shape: `L(h…) :- X(h…), X(h… with w for the value), comparisons`. Its `X`, and its comparisons
/// as `comparison` names them, sorted.
fn later_rule(head: &Head, body: &[Lit]) -> Option<(String, Vec<&'static str>)> {
    let h = vars(&head.args)?;
    let (v, keys) = h.split_last()?;
    let mut atoms = Vec::new();
    let mut cmps = Vec::new();
    for l in body {
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

/// The module's first-of-a-group relations.
pub fn recognize(statements: &[Statement]) -> Vec<First> {
    if std::env::var("RERAIL_FIRST").is_ok_and(|v| v == "off") {
        return Vec::new();
    }
    let mut decls: BTreeMap<&str, &Vec<(String, String)>> = BTreeMap::new();
    let mut tails: BTreeMap<&str, &Vec<String>> = BTreeMap::new();
    let mut rules: BTreeMap<&str, Vec<(&Head, &Vec<Lit>)>> = BTreeMap::new();
    // How many literals (and `.output`s) read each relation.
    let mut reads: BTreeMap<&str, usize> = BTreeMap::new();
    let mut multi_head: BTreeSet<&str> = BTreeSet::new();
    fn count<'a>(l: &'a Lit, reads: &mut BTreeMap<&'a str, usize>) {
        match l {
            Lit::Atom { rel, .. } => *reads.entry(rel).or_default() += 1,
            Lit::Group { alts, .. } => alts.iter().flatten().for_each(|l| count(l, reads)),
            _ => {}
        }
    }
    for (item, _) in statements {
        match item {
            Item::Decl { name, cols, tail } => {
                decls.insert(name, cols);
                tails.insert(name, tail);
            }
            Item::Io { names, .. } => names.iter().for_each(|n| *reads.entry(n).or_default() += 1),
            Item::Clause { heads, body, edits } => {
                body.iter().for_each(|l| count(l, &mut reads));
                if heads.len() != 1 || !edits.is_empty() {
                    heads.iter().for_each(|h| {
                        multi_head.insert(&h.rel);
                    });
                    continue;
                }
                rules.entry(&heads[0].rel).or_default().push((&heads[0], body));
            }
            // A test or fixture reads the module's relations too.
            Item::Test { .. } | Item::Fixtures { .. } => return Vec::new(),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for (&first, frules) in &rules {
        let ([(head, body)], false) = (frules.as_slice(), multi_head.contains(first)) else { continue };
        if head.modal.is_some() {
            continue;
        }
        let Some(cols) = decls.get(first) else { continue };
        if !tails.get(first).is_some_and(|t| t.is_empty()) || cols.iter().any(|(_, t)| t.ends_with("<>")) {
            continue;
        }
        let Some(h) = vars(&head.args) else { continue };
        let Some((v, _)) = h.split_last() else { continue };
        // `X(h…)` and `!L(h…)`, with other literals that do not read the value.
        let mut of = None;
        let mut later = None;
        let mut rest_reads_v = false;
        for l in body.iter() {
            match l {
                Lit::Atom { rel, args, neg } if (!neg && is_head_with_wilds(args, &h)) || (*neg && vars(args).as_ref() == Some(&h)) => match (neg, &of, &later) {
                    (false, None, _) => of = Some(rel),
                    (true, _, None) => later = Some(rel),
                    _ => rest_reads_v = true,
                },
                l => rest_reads_v |= lit_reads(l, v),
            }
        }
        let (Some(of), Some(later), false) = (of, later, rest_reads_v) else { continue };
        let Some(lrules) = rules.get(later.as_str()) else { continue };
        if multi_head.contains(later.as_str()) || lrules.iter().any(|(h, _)| h.modal.is_some()) {
            continue;
        }
        let Some(shapes) = lrules.iter().map(|(h, b)| later_rule(h, b)).collect::<Option<Vec<_>>>() else { continue };
        if shapes.iter().any(|(x, _)| x != of) {
            continue;
        }
        let Some(order) = order_of(&shapes.iter().map(|(_, c)| c.clone()).collect::<Vec<_>>()) else { continue };
        let typed = |r: &str| decls.get(r).and_then(|c| c.last()).is_some_and(|(_, t)| typed_for(order, t));
        // F's head copies the value from X, so F's column is its type.
        if !typed(first) {
            continue;
        }
        // F's rule reads L once; L's own rules read it never. Anything else keeps it.
        let drop_later = reads.get(later.as_str()).copied().unwrap_or(0) == 1;
        found.push(First { first: first.to_owned(), later: later.clone(), of: of.clone(), order, drop_later });
    }
    found
}

/// Rewrites the module's statements for `found`: each `F`'s value column is its lattice, its rule loses `!L`, and a
/// dropped `L` loses its declaration and rules.
pub fn rewrite(statements: &mut Vec<Statement>, found: &[First]) {
    if found.is_empty() {
        return;
    }
    let by_first: BTreeMap<&str, &First> = found.iter().map(|f| (f.first.as_str(), f)).collect();
    let dropped: BTreeSet<&str> = found.iter().filter(|f| f.drop_later).map(|f| f.later.as_str()).collect();
    statements.retain(|(item, _)| match item {
        Item::Decl { name, .. } => !dropped.contains(name.as_str()),
        Item::Clause { heads, .. } => !(heads.len() == 1 && dropped.contains(heads[0].rel.as_str())),
        _ => true,
    });
    for (item, _) in statements.iter_mut() {
        match item {
            Item::Decl { name, cols, .. } => {
                if let (Some(f), Some(last)) = (by_first.get(name.as_str()), cols.last_mut()) {
                    last.1 = f.order.column().to_owned();
                }
            }
            Item::Clause { heads, body, .. } if heads.len() == 1 => {
                if let Some(f) = by_first.get(heads[0].rel.as_str()) {
                    body.retain(|l| !matches!(l, Lit::Atom { rel, neg: true, .. } if *rel == f.later));
                }
            }
            _ => {}
        }
    }
}
