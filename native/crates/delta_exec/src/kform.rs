//! K-form: a program as rows of eight relations (decision 0686), each row's first column its module (the document its
//! rules were read from; rule and term ids are that document's node ids, L9):
//!
//! - `k_rule(m, rule, head)`: a rule of module `m`, writing relation `head`;
//! - `k_step(m, rule, k, kind, rel)`: step `k` of its body: kind `atom`, `not` or `cmp` (`rel` a `cmp`'s operator);
//! - `k_arg(m, rule, k, i, term)`: step `k`'s `i`-th argument; `k = -1` is the head;
//! - `k_var(m, term, name)`, `k_str(m, term, s)`, `k_num(m, term, n)`, `k_wild(m, term)`: what a term is;
//! - `k_order(m, k)`: module `m`'s stratum; a module with none is stratum 0.
//!
//! What a program means is fixed here, and nothing else is (decision 0686): a rule's steps run in `k` order; a
//! variable is bound at its first occurrence and tested at every later one; an `atom` step joins on its bound
//! columns, a `not` step is an anti-join, a `cmp` a filter. The executor never reorders or rewrites a rule. It
//! refuses one it cannot run as written: a `not` or `cmp` with an unbound variable, a head variable or `_` no step
//! binds, a step with no argument where its relation has some.
//!
//! `Plan` is a rule read off its rows for one run: plan state, made from the rows each run and kept by nothing.

use crate::db::Db;
use crate::sym::Symbols;
use rustc_hash::FxHashMap;

/// The K-form relations, by name, with their arities.
pub const RELATIONS: [(&str, usize); 8] =
    [("k_rule", 3), ("k_step", 5), ("k_arg", 5), ("k_var", 3), ("k_str", 3), ("k_num", 3), ("k_wild", 2), ("k_order", 2)];

/// The symbols the executor reads K-form by. A level's program lies in the eight relations under one prefix: `k_`
/// as read, `k1_` as the first lowering writes them (decision 0686: one form, a name a level).
pub struct Words {
    pub k: [i64; 8],
    pub atom: i64,
    pub not: i64,
    pub cmp: i64,
    pub ops: [(i64, Op); 6],
}

impl Words {
    pub fn new(sym: &Symbols) -> Words {
        Words::with_prefix(sym, "k_")
    }

    pub fn with_prefix(sym: &Symbols, prefix: &str) -> Words {
        let k = RELATIONS.map(|(n, _)| sym.intern(&format!("{prefix}{}", n.strip_prefix("k_").expect("k_"))));
        let ops = [("=", Op::Eq), ("!=", Op::Ne), ("<", Op::Lt), ("<=", Op::Le), (">", Op::Gt), (">=", Op::Ge)].map(|(s, o)| (sym.intern(s), o));
        Words { k, atom: sym.intern("atom"), not: sym.intern("not"), cmp: sym.intern("cmp"), ops }
    }

    pub fn rule(&self) -> i64 {
        self.k[0]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Op {
    pub fn holds(self, a: i64, b: i64) -> bool {
        match self {
            Op::Eq => a == b,
            Op::Ne => a != b,
            Op::Lt => a < b,
            Op::Le => a <= b,
            Op::Gt => a > b,
            Op::Ge => a >= b,
        }
    }
}

/// An argument as the run reads it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arg {
    /// A constant: the column must equal it.
    Const(i64),
    /// A variable's first occurrence: the column binds register `r`.
    Bind(usize),
    /// A later occurrence: the column must equal register `r`.
    Test(usize),
    /// A later occurrence in the atom that binds `r`: the column must equal the one that bound it.
    Again(usize),
    /// `_`: any value.
    Wild,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepKind {
    Atom,
    Not,
    Cmp(Op),
}

#[derive(Clone, Debug)]
pub struct Step {
    pub kind: StepKind,
    pub rel: i64,
    pub args: Vec<Arg>,
}

/// One rule, read off its K-form rows.
#[derive(Clone, Debug)]
pub struct Plan {
    pub module: i64,
    pub rule: i64,
    pub head: i64,
    pub head_args: Vec<Arg>,
    pub steps: Vec<Step>,
    pub regs: usize,
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

/// A program's rules on one side of its run, by module, with each module's stratum.
pub struct Program {
    pub plans: Vec<Plan>,
    pub order: FxHashMap<i64, i64>,
}

impl Program {
    /// The program held in `db`'s K-form relations on one side (`after`), read into plans by probes of its rows
    /// where they lie. A database with no K-form relations holds the empty program.
    pub fn read(db: &Db, after: bool, w: &Words, sym: &Symbols) -> Result<Program, Refused> {
        // Each row of K-form relation `i` agreeing with `bound`.
        let each = |i: usize, bound: &[Option<i64>], f: &mut dyn FnMut(&[i64])| {
            if db.has(w.k[i]) {
                let v = if after { db.after(w.k[i]) } else { db.before(w.k[i]) };
                crate::run::scan_table(&v, bound, f);
            }
        };
        // The rules, in (module, rule) order: plan state.
        let mut rules: Vec<[i64; 3]> = Vec::new();
        each(0, &[None; 3], &mut |r| rules.push([r[0], r[1], r[2]]));
        rules.sort_unstable();
        let mut order = FxHashMap::default();
        each(7, &[None; 2], &mut |r| {
            order.insert(r[0], r[1]);
        });
        let mut plans = Vec::new();
        for [module, rule, head] in rules {
            let refuse = |why: String| Refused { module, rule, why };
            // What term `t` is.
            let term = |t: i64| -> Result<Term, Refused> {
                let mut found: Vec<Term> = Vec::new();
                each(3, &[Some(module), Some(t), None], &mut |r| found.push(Term::Var(r[2])));
                each(4, &[Some(module), Some(t), None], &mut |r| found.push(Term::Const(r[2])));
                each(5, &[Some(module), Some(t), None], &mut |r| found.push(Term::Const(r[2])));
                each(6, &[Some(module), Some(t)], &mut |_| found.push(Term::Wild));
                match found[..] {
                    [one] => Ok(one),
                    [] => Err(refuse(format!("term {t} is no variable, constant or _"))),
                    _ => Err(refuse(format!("term {t} is {} things", found.len()))),
                }
            };
            // Step `k`'s terms, in order.
            let read = |k: i64| -> Result<Vec<Term>, Refused> {
                let mut args: Vec<(i64, i64)> = Vec::new();
                each(2, &[Some(module), Some(rule), Some(k), None, None], &mut |r| args.push((r[3], r[4])));
                args.sort_unstable();
                if args.iter().enumerate().any(|(i, a)| a.0 != i as i64) {
                    return Err(refuse(format!("step {k}'s arguments are not 0 .. {}, one each", args.len())));
                }
                args.iter().map(|a| term(a.1)).collect()
            };
            let mut steps: Vec<(i64, i64, i64)> = Vec::new();
            each(1, &[Some(module), Some(rule), None, None, None], &mut |r| steps.push((r[2], r[3], r[4])));
            steps.sort_unstable();
            if steps.windows(2).any(|s| s[0].0 == s[1].0) {
                return Err(refuse("two steps at one place".into()));
            }
            let mut regs: FxHashMap<i64, usize> = FxHashMap::default();
            let mut plan_steps = Vec::new();
            for (k, kind, rel) in steps {
                let terms = read(k)?;
                let (kind, binds) = if kind == w.atom {
                    (StepKind::Atom, true)
                } else if kind == w.not {
                    (StepKind::Not, false)
                } else if kind == w.cmp {
                    let op = w.ops.iter().find(|o| o.0 == rel).ok_or_else(|| refuse(format!("step {k}: no comparison {}", sym.name(rel))))?.1;
                    if terms.len() != 2 {
                        return Err(refuse(format!("step {k}: a comparison of {} terms", terms.len())));
                    }
                    (StepKind::Cmp(op), false)
                } else {
                    return Err(refuse(format!("step {k}: no step kind {}", sym.name(kind))));
                };
                let mut a = Vec::with_capacity(terms.len());
                for t in terms {
                    a.push(match t {
                        Term::Const(c) => Arg::Const(c),
                        Term::Wild if matches!(kind, StepKind::Cmp(_)) => return Err(refuse(format!("step {k}: `_` in a comparison"))),
                        Term::Wild => Arg::Wild,
                        Term::Var(v) => match regs.get(&v) {
                            Some(&r) => Arg::Test(r),
                            None if binds => {
                                let r = regs.len();
                                regs.insert(v, r);
                                Arg::Bind(r)
                            }
                            None => return Err(refuse(format!("step {k}: variable {} is not bound before it", sym.name(v)))),
                        },
                    });
                }
                // A variable twice in one atom: its second occurrence tests the first.
                let mut seen = Vec::new();
                for x in &mut a {
                    if let Arg::Bind(r) = *x {
                        if seen.contains(&r) {
                            *x = Arg::Again(r);
                        }
                        seen.push(r);
                    }
                }
                plan_steps.push(Step { kind, rel, args: a });
            }
            let mut head_args = Vec::new();
            for t in read(-1)? {
                head_args.push(match t {
                    Term::Const(c) => Arg::Const(c),
                    Term::Wild => return Err(refuse("`_` in the head".into())),
                    Term::Var(v) => Arg::Test(*regs.get(&v).ok_or_else(|| refuse(format!("head variable {} is bound by no step", sym.name(v))))?),
                });
            }
            plans.push(Plan { module, rule, head, head_args, steps: plan_steps, regs: regs.len() });
        }
        Ok(Program { plans, order })
    }

    /// Module `m`'s stratum.
    pub fn stratum(&self, m: i64) -> i64 {
        self.order.get(&m).copied().unwrap_or(0)
    }

    /// Whether it has no rules.
    pub fn is_empty(&self) -> bool {
        self.plans.is_empty()
    }
}

#[derive(Clone, Copy, Debug)]
enum Term {
    Var(i64),
    Const(i64),
    Wild,
}
