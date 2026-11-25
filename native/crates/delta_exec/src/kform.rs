//! K-form: a program as rows of nine relations (decision 0686), each row's first column its module (the document its
//! rules were read from; rule and term ids are that document's node ids, L9):
//!
//! - `k_rule(m, rule, head)`: a rule of module `m`, writing relation `head`;
//! - `k_step(m, rule, k, kind, rel)`: step `k` of its body: kind `atom`, `not` or `cmp` (`rel` a `cmp`'s operator);
//! - `k_arg(m, rule, k, i, term)`: step `k`'s `i`-th argument; `k = -1` is the head;
//! - `k_var(m, term, name)`, `k_str(m, term, s)`, `k_num(m, term, n)`, `k_wild(m, term)`: what a term is;
//! - `k_fn(m, term, op, left, right)`: a term that is an arithmetic expression (`+`, `-`, `*`) of two terms;
//! - `k_order(m, head, k)`: the stratum of the rules of module `m` that write `head`; a head with none is stratum 0.
//!
//! What a program means is fixed here, and nothing else is (decision 0686): a rule's steps run in `k` order; a
//! variable is bound at its first occurrence and tested at every later one; an `atom` step joins on its bound
//! columns, a `not` step is an anti-join, a `cmp` a filter; an expression is computed in a head; strata run in the
//! order the program states. The executor never reorders, rewrites or stratifies a rule. It refuses one it cannot run as written: a `not` or `cmp` with an unbound variable, a
//! head variable no step binds, `_` in a head or a comparison, an expression in a body.
//!
//! A `Plan` is a rule read off its rows, with what a run needs of it that the rows do not hold where they lie: its
//! variables numbered, whether it reads one document only, and the columns it probes each relation by. Plans are the
//! level's cache (`Db::plans`, L5), read again only for the modules whose rows changed.

use crate::db::{Db, each_row};
use crate::sym::Symbols;
use rerail_blocks::change::{Cmp, Term};
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::BTreeSet;

/// The K-form relations, by name, with their arities.
pub const RELATIONS: [(&str, usize); 9] = [
    ("k_rule", 3),
    ("k_step", 5),
    ("k_arg", 5),
    ("k_var", 3),
    ("k_str", 3),
    ("k_num", 3),
    ("k_wild", 2),
    ("k_fn", 5),
    ("k_order", 3),
];

/// The symbols the executor reads K-form by. A level's program lies in the relations under one prefix: `k_` as
/// read, `k1_` as the first lowering writes them (one form, a name a level).
pub struct Words {
    pub k: [i64; 9],
    pub atom: i64,
    pub not: i64,
    pub cmp: i64,
    pub ops: [(i64, Cmp); 6],
    pub fns: [(i64, Fun); 3],
}

impl Words {
    pub fn new(sym: &Symbols) -> Words {
        Words::with_prefix(sym, "k_")
    }

    pub fn with_prefix(sym: &Symbols, prefix: &str) -> Words {
        let k = RELATIONS
            .map(|(n, _)| sym.intern(&format!("{prefix}{}", n.strip_prefix("k_").expect("k_"))));
        let ops = [
            ("=", Cmp::Eq),
            ("!=", Cmp::Ne),
            ("<", Cmp::Lt),
            ("<=", Cmp::Le),
            (">", Cmp::Gt),
            (">=", Cmp::Ge),
        ]
        .map(|(s, o)| (sym.intern(s), o));
        let fns =
            [("+", Fun::Add), ("-", Fun::Sub), ("*", Fun::Mul)].map(|(s, f)| (sym.intern(s), f));
        Words {
            k,
            atom: sym.intern("atom"),
            not: sym.intern("not"),
            cmp: sym.intern("cmp"),
            ops,
            fns,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fun {
    Add,
    Sub,
    Mul,
}

/// A head's argument: a term, or an expression of two.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Val {
    T(Term),
    Fn(Fun, Box<Val>, Box<Val>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartKind {
    Join,
    Not,
    Test(Cmp),
}

#[derive(Clone, Debug)]
pub struct PartPlan {
    pub kind: PartKind,
    pub rel: i64,
    pub terms: Vec<Term>,
}

/// One rule, read off its K-form rows.
#[derive(Clone, Debug)]
pub struct Plan {
    pub module: i64,
    pub rule: i64,
    pub stratum: i64,
    pub head: i64,
    pub head_args: Vec<Val>,
    pub parts: Vec<PartPlan>,
    pub vars: usize,
    /// Every join and negation reads a view, all of one document (their first terms one variable): its rows are one
    /// document's, so it runs in that document's single-file wave.
    pub single_file: bool,
    /// The columns each relation it reads is probed by, in its cold run and its terms.
    pub probes: Vec<(i64, Vec<u16>)>,
}

/// Why a program cannot run as written: its module and rule (node ids), and what.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub module: i64,
    pub rule: i64,
    pub why: String,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "module {} rule {}: {}", self.module, self.rule, self.why)
    }
}

/// Each row of K-form relation `i` of `db` on one side that agrees with `bound`.
fn each(
    db: &Db,
    w: &Words,
    after: bool,
    i: usize,
    bound: &[Option<i64>],
    f: &mut dyn FnMut(&[i64]),
) {
    if db.has(w.k[i]) {
        let v = if after {
            db.after(w.k[i])
        } else {
            db.before(w.k[i])
        };
        each_row(&v, bound, f);
    }
}

/// The modules with rules in `db`'s program on one side.
pub fn modules(db: &Db, w: &Words, after: bool) -> BTreeSet<i64> {
    let mut m = BTreeSet::new();
    each(db, w, after, 0, &[None; 3], &mut |r| {
        m.insert(r[0]);
    });
    m
}

/// The modules whose rows changed in the run of `db`'s program.
pub fn changed_modules(db: &Db, w: &Words) -> BTreeSet<i64> {
    let mut m = BTreeSet::new();
    for &k in &w.k {
        if db.has(k) && db.changed(k) {
            let b = db.bistate(k);
            let before = b.before().expect("a version");
            for id in b.gone() {
                m.insert(rerail_blocks::bistate::Read::get(
                    before,
                    id,
                    rerail_blocks::bistate::ColumnIndex::new(0),
                ));
            }
            for id in b.added() {
                m.insert(rerail_blocks::bistate::Read::get(
                    b.after(),
                    id,
                    rerail_blocks::bistate::ColumnIndex::new(0),
                ));
            }
        }
    }
    m
}

#[derive(Clone, Debug)]
enum Read {
    Var(i64),
    Const(i64),
    Wild,
    Fn(Fun, Box<Read>, Box<Read>),
}

/// Module `m`'s rules in `db`'s program on one side, read into plans by probes of its rows where they lie.
/// `is_view` says which relations are tape views.
pub fn read_module(
    db: &Db,
    w: &Words,
    after: bool,
    sym: &Symbols,
    is_view: &dyn Fn(i64) -> bool,
    m: i64,
) -> Result<Vec<Plan>, Refused> {
    let mut rules: Vec<[i64; 2]> = Vec::new();
    each(db, w, after, 0, &[Some(m), None, None], &mut |r| {
        rules.push([r[1], r[2]])
    });
    rules.sort_unstable();
    let mut plans = Vec::new();
    for [rule, head] in rules {
        let refuse = |why: String| Refused {
            module: m,
            rule,
            why,
        };
        let mut strata: Vec<i64> = Vec::new();
        each(db, w, after, 8, &[Some(m), Some(head), None], &mut |r| {
            strata.push(r[2])
        });
        let stratum = match strata[..] {
            [] => 0,
            [k] => k,
            _ => {
                return Err(refuse(format!(
                    "{} is given {} strata",
                    sym.name(head),
                    strata.len()
                )));
            }
        };
        // What term `t` is.
        let term = |t: i64| -> Result<Read, Refused> {
            fn go(
                db: &Db,
                w: &Words,
                after: bool,
                m: i64,
                t: i64,
                depth: usize,
            ) -> Result<Read, String> {
                let mut found: Vec<Read> = Vec::new();
                let b = [Some(m), Some(t), None];
                each(db, w, after, 3, &b, &mut |r| found.push(Read::Var(r[2])));
                each(db, w, after, 4, &b, &mut |r| found.push(Read::Const(r[2])));
                each(db, w, after, 5, &b, &mut |r| found.push(Read::Const(r[2])));
                each(db, w, after, 6, &[Some(m), Some(t)], &mut |_| {
                    found.push(Read::Wild)
                });
                let mut fns: Vec<[i64; 3]> = Vec::new();
                each(
                    db,
                    w,
                    after,
                    7,
                    &[Some(m), Some(t), None, None, None],
                    &mut |r| fns.push([r[2], r[3], r[4]]),
                );
                for [op, l, r] in fns {
                    let f = w
                        .fns
                        .iter()
                        .find(|f| f.0 == op)
                        .ok_or(format!("term {t}: no operator {op}"))?
                        .1;
                    if depth > 64 {
                        return Err(format!("term {t}: an expression too deep"));
                    }
                    found.push(Read::Fn(
                        f,
                        Box::new(go(db, w, after, m, l, depth + 1)?),
                        Box::new(go(db, w, after, m, r, depth + 1)?),
                    ));
                }
                match found.len() {
                    1 => Ok(found.pop().expect("one")),
                    0 => Err(format!(
                        "term {t} is no variable, constant, _ or expression"
                    )),
                    n => Err(format!("term {t} is {n} things")),
                }
            }
            go(db, w, after, m, t, 0).map_err(&refuse)
        };
        // Step `k`'s terms, in order.
        let args = |k: i64| -> Result<Vec<Read>, Refused> {
            let mut a: Vec<(i64, i64)> = Vec::new();
            each(
                db,
                w,
                after,
                2,
                &[Some(m), Some(rule), Some(k), None, None],
                &mut |r| a.push((r[3], r[4])),
            );
            a.sort_unstable();
            if a.iter().enumerate().any(|(i, x)| x.0 != i as i64) {
                return Err(refuse(format!(
                    "step {k}'s arguments are not 0 .. {}, one each",
                    a.len()
                )));
            }
            a.iter().map(|x| term(x.1)).collect()
        };
        let mut steps: Vec<(i64, i64, i64)> = Vec::new();
        each(
            db,
            w,
            after,
            1,
            &[Some(m), Some(rule), None, None, None],
            &mut |r| steps.push((r[2], r[3], r[4])),
        );
        steps.sort_unstable();
        if steps.windows(2).any(|s| s[0].0 == s[1].0) {
            return Err(refuse("two steps at one place".into()));
        }
        // Variables numbered by their first occurrence; `bound` those bound so far.
        let mut names: Vec<i64> = Vec::new();
        let mut bound: Vec<bool> = Vec::new();
        let mut parts = Vec::new();
        for (k, kind, rel) in steps {
            let reads = args(k)?;
            let kind = if kind == w.atom {
                PartKind::Join
            } else if kind == w.not {
                PartKind::Not
            } else if kind == w.cmp {
                let op = w
                    .ops
                    .iter()
                    .find(|o| o.0 == rel)
                    .ok_or_else(|| refuse(format!("step {k}: no comparison {}", sym.name(rel))))?
                    .1;
                if reads.len() != 2 {
                    return Err(refuse(format!(
                        "step {k}: a comparison of {} terms",
                        reads.len()
                    )));
                }
                PartKind::Test(op)
            } else {
                return Err(refuse(format!("step {k}: no step kind {}", sym.name(kind))));
            };
            let mut terms = Vec::with_capacity(reads.len());
            let mut binds = Vec::new();
            for r in reads {
                terms.push(match r {
                    Read::Const(c) => Term::Const(c),
                    Read::Wild if matches!(kind, PartKind::Test(_)) => {
                        return Err(refuse(format!("step {k}: `_` in a comparison")));
                    }
                    Read::Wild => Term::Any,
                    Read::Fn(..) => {
                        return Err(refuse(format!("step {k}: an expression in a body")));
                    }
                    Read::Var(v) => {
                        let i = names.iter().position(|&n| n == v).unwrap_or_else(|| {
                            names.push(v);
                            bound.push(false);
                            names.len() - 1
                        });
                        if !bound[i] {
                            if kind != PartKind::Join {
                                return Err(refuse(format!(
                                    "step {k}: variable {} is not bound before it",
                                    sym.name(v)
                                )));
                            }
                            binds.push(i);
                        }
                        Term::Var(i)
                    }
                });
            }
            for i in binds {
                bound[i] = true;
            }
            parts.push(PartPlan { kind, rel, terms });
        }
        let head_val = |r: Read| -> Result<Val, Refused> {
            fn go(r: Read, names: &[i64], sym: &Symbols) -> Result<Val, String> {
                Ok(match r {
                    Read::Const(c) => Val::T(Term::Const(c)),
                    Read::Wild => return Err("`_` in the head".into()),
                    Read::Var(v) => {
                        Val::T(Term::Var(names.iter().position(|&n| n == v).ok_or_else(
                            || format!("head variable {} is bound by no step", sym.name(v)),
                        )?))
                    }
                    Read::Fn(f, l, r) => Val::Fn(
                        f,
                        Box::new(go(*l, names, sym)?),
                        Box::new(go(*r, names, sym)?),
                    ),
                })
            }
            go(r, &names, sym).map_err(&refuse)
        };
        let head_args = args(-1)?
            .into_iter()
            .map(head_val)
            .collect::<Result<Vec<_>, _>>()?;
        let reads: Vec<&PartPlan> = parts
            .iter()
            .filter(|p| !matches!(p.kind, PartKind::Test(_)))
            .collect();
        let single_file = !reads.is_empty()
            && reads.iter().all(|p| is_view(p.rel))
            && reads.iter().all(|p| {
                matches!(p.terms.first(), Some(Term::Var(_)))
                    && p.terms.first() == reads[0].terms.first()
            });
        let probes = probes(&parts, names.len(), is_view);
        plans.push(Plan {
            module: m,
            rule,
            stratum,
            head,
            head_args,
            parts,
            vars: names.len(),
            single_file,
            probes,
        });
    }
    Ok(plans)
}

/// The columns each relation of `parts` is probed by in its cold run and in each part's term (`change::body`'s
/// orders: the parts in order; part `i` first, then the rest in order).
fn probes(parts: &[PartPlan], vars: usize, is_view: &dyn Fn(i64) -> bool) -> Vec<(i64, Vec<u16>)> {
    let mut out: Vec<(i64, Vec<u16>)> = Vec::new();
    let orders = std::iter::once(None).chain((0..parts.len()).map(Some));
    for first in orders {
        let order: Vec<usize> = match first {
            None => (0..parts.len()).collect(),
            Some(i) => std::iter::once(i)
                .chain((0..parts.len()).filter(|&j| j != i))
                .collect(),
        };
        let mut bound = vec![false; vars];
        for p in order {
            let part = &parts[p];
            if matches!(part.kind, PartKind::Test(_)) {
                continue;
            }
            let changed = first == Some(p);
            let cols: Vec<u16> = part
                .terms
                .iter()
                .enumerate()
                .filter(|(_, t)| match t {
                    Term::Const(_) => true,
                    Term::Var(v) => bound[*v] || (part.kind == PartKind::Not && changed),
                    Term::Any => false,
                })
                .map(|(c, _)| c as u16)
                .collect();
            // A changed join's rows are read from its change, not probed.
            if !(changed && part.kind == PartKind::Join)
                && !cols.is_empty()
                && !is_view(part.rel)
                && !out.iter().any(|o| o.0 == part.rel && o.1 == cols)
            {
                out.push((part.rel, cols));
            }
            if part.kind == PartKind::Join || changed {
                for t in &part.terms {
                    if let Term::Var(v) = t {
                        bound[*v] = true;
                    }
                }
            }
        }
    }
    out
}

/// The columns the executor reads a program's K-form relations by (`read_module`, and a module's rows killed by
/// `k0::pass`): asked for in `db`, which holds a program in the relations `w` names.
pub fn indexes(db: &mut Db, w: &Words) {
    const BY: [&[u16]; 9] = [
        &[0],
        &[0, 1],
        &[0, 1, 2],
        &[0, 1],
        &[0, 1],
        &[0, 1],
        &[0, 1],
        &[0, 1],
        &[0, 1],
    ];
    for (i, &(_, arity)) in RELATIONS.iter().enumerate() {
        db.declare(w.k[i], arity);
        db.want(w.k[i], &[0]);
        db.want(w.k[i], BY[i]);
    }
}

/// Every plan of `db`'s program on one side, for checks and tests.
pub fn read_all(
    db: &Db,
    w: &Words,
    after: bool,
    sym: &Symbols,
    is_view: &dyn Fn(i64) -> bool,
) -> Result<Vec<Plan>, Refused> {
    let mut all = Vec::new();
    for m in modules(db, w, after) {
        all.extend(read_module(db, w, after, sym, is_view, m)?);
    }
    Ok(all)
}

/// A stratum of a program, in the order it runs: its rules (module, place in the module's plans), its heads, and
/// what its rules read of it.
#[derive(Clone, Debug)]
pub struct Stratum {
    pub k: i64,
    pub plans: Vec<(i64, usize)>,
    pub heads: FxHashSet<i64>,
    /// A rule reads a head of its own stratum: it runs as a fixpoint.
    pub recursive: bool,
    /// It writes `load`: its loads are answered before its other rules run.
    pub asks: bool,
}

/// A level's cache of the program it runs (L5): each module's plans, and the strata they make, as the program
/// states them. Made again only for the modules whose rows changed.
#[derive(Clone, Debug, Default)]
pub struct Program {
    pub modules: FxHashMap<i64, Vec<Plan>>,
    pub strata: Vec<Stratum>,
}

impl Program {
    pub fn plan(&self, at: (i64, usize)) -> &Plan {
        &self.modules[&at.0][at.1]
    }

    /// The strata the program states (`k_order`), checked: a head is written in one stratum; a rule reads no head of
    /// a later stratum, negates none of its own, and reads views no earlier than `load`'s stratum (a view's rows are
    /// the documents the loads answer). The executor computes no strata: a program that is not stratified as it
    /// states is refused.
    pub fn order(
        &mut self,
        sym: &Symbols,
        is_view: &dyn Fn(i64) -> bool,
        load: i64,
    ) -> Result<(), Refused> {
        let mut at: Vec<(i64, usize)> = self
            .modules
            .iter()
            .flat_map(|(&m, ps)| (0..ps.len()).map(move |i| (m, i)))
            .collect();
        at.sort_unstable();
        let mut stratum_of: FxHashMap<i64, i64> = FxHashMap::default();
        let mut strata: std::collections::BTreeMap<i64, Stratum> = Default::default();
        for &a in &at {
            let p = self.plan(a);
            let refuse = |why: String| Refused {
                module: p.module,
                rule: p.rule,
                why,
            };
            if is_view(p.head) {
                return Err(refuse(format!("{} is a view", sym.name(p.head))));
            }
            if let Some(&j) = stratum_of.get(&p.head).filter(|&&j| j != p.stratum) {
                return Err(refuse(format!(
                    "{} is written in strata {j} and {}",
                    sym.name(p.head),
                    p.stratum
                )));
            }
            stratum_of.insert(p.head, p.stratum);
            let s = strata.entry(p.stratum).or_insert_with(|| Stratum {
                k: p.stratum,
                plans: Vec::new(),
                heads: FxHashSet::default(),
                recursive: false,
                asks: false,
            });
            s.plans.push(a);
            s.heads.insert(p.head);
            s.asks |= p.head == load;
        }
        for s in strata.values_mut() {
            for &a in &s.plans {
                let p = &self.modules[&a.0][a.1];
                let refuse = |why: String| Refused {
                    module: p.module,
                    rule: p.rule,
                    why,
                };
                for part in p
                    .parts
                    .iter()
                    .filter(|x| !matches!(x.kind, PartKind::Test(_)))
                {
                    let reads = if is_view(part.rel) { load } else { part.rel };
                    match stratum_of.get(&reads) {
                        Some(&j) if j > s.k => {
                            return Err(refuse(format!(
                                "{} is read in stratum {} before its stratum {j}",
                                sym.name(part.rel),
                                s.k
                            )));
                        }
                        Some(&j)
                            if j == s.k && part.kind == PartKind::Not && !is_view(part.rel) =>
                        {
                            return Err(refuse(format!(
                                "{} is negated in its own stratum {j}",
                                sym.name(part.rel)
                            )));
                        }
                        // Views and `load` are read once the stratum's loads are answered, except by the loads.
                        Some(&j) if j == s.k && (reads != load || p.head == load) => {
                            s.recursive = true
                        }
                        _ => {}
                    }
                }
            }
        }
        self.strata = strata.into_values().collect();
        Ok(())
    }
}
