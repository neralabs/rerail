//! A program's K-form rows read as rules (decision 0686; B§3, B§7): what the executor runs, built as data from the
//! rows the program trace holds (`kform`), a chain of steps a rule, so that a plan is made from it and nothing else.
//!
//! B§3's five sentences are its whole meaning, and this reading keeps them as written:
//!
//! 1. a rule's steps run in their key order `(k, j)`; a variable is bound at its first occurrence and tested at every
//!    later one;
//! 2. an atom step joins, a `not` step is an anti-join, a `cmp` step filters;
//! 3. a relation is a tape view, an answer (`load` → `doc`), a base input, or the head of rules;
//! 4. a module's rules form one stratum, and strata run in `k_order`;
//! 5. a rule whose steps read one document's views and relations runs in that document's single-file wave
//!    (`Rule::single_file`), any other in the cross-file wave.
//!
//! What the executor refuses rather than fixes (B§7), each naming the module and the rule's node: a `not` or `cmp`
//! step with a variable no earlier step binds, a head variable no step binds, a stratum that negates its own head, a
//! step that reads the head of a stratum that does not run before its own, a relation two modules write, and a
//! program of several modules one of which has no `k_order` row. Placing a step correctly is the lowerer's job, in
//! rules.

use crate::kform::{RELATIONS, Refused};
use crate::sym::find;
use crate::tape::View;
use std::collections::{BTreeMap, BTreeSet};

/// A term: a variable (its name's symbol), a constant (a string's symbol or a number's value) or `_`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Term {
    Var(i64),
    Const(i64),
    Wild,
}

/// A comparison's operator.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Cmp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Cmp {
    pub const ALL: [(Cmp, &'static str); 6] = [(Cmp::Eq, "="), (Cmp::Ne, "!="), (Cmp::Lt, "<"), (Cmp::Le, "<="), (Cmp::Gt, ">"), (Cmp::Ge, ">=")];

    pub fn test(self, a: i64, b: i64) -> bool {
        match self {
            Cmp::Eq => a == b,
            Cmp::Ne => a != b,
            Cmp::Lt => a < b,
            Cmp::Le => a <= b,
            Cmp::Gt => a > b,
            Cmp::Ge => a >= b,
        }
    }

    pub fn code(self) -> i64 {
        Cmp::ALL.iter().position(|c| c.0 == self).expect("an operator") as i64
    }

    pub fn of_code(c: i64) -> Option<Cmp> {
        Cmp::ALL.get(usize::try_from(c).ok()?).map(|c| c.0)
    }
}

/// What a step does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Atom,
    Not,
    Cmp(Cmp),
}

/// A step of a rule's body: its key, what it does, the relation it reads (a `cmp`'s none) and its arguments.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Step {
    pub key: (i64, i64),
    pub kind: Kind,
    pub rel: i64,
    pub args: Vec<Term>,
}

/// A rule: its module, its node, its head's relation and arguments, and its steps in key order.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Rule {
    pub module: i64,
    pub id: i64,
    pub head: i64,
    pub args: Vec<Term>,
    pub steps: Vec<Step>,
}

/// The relations the executor knows by name (B§3, sentence 3): the tape views, and `load`, the answer whose rows the
/// world answers with documents. Every other relation is a base input or the head of rules.
#[derive(Clone, Debug, Default)]
pub struct Known {
    pub views: BTreeMap<i64, View>,
    pub load: Option<i64>,
    /// The declarations' relations (decision 0361), in `shapes::DECLS`' order: `key`, `fanout`, `tree`, `order`; and
    /// the classes `one` and `few`.
    pub decls: [Option<i64>; 4],
    pub one: Option<i64>,
    pub few: Option<i64>,
    /// A named algorithm (0361 item 4's override): `by(rel, wave)` runs `rel`'s recursion in one wave.
    pub by: Option<i64>,
    pub wave: Option<i64>,
}

impl Known {
    /// The views' and `load`'s symbols in the atoms block `atoms`, where they have one.
    pub fn new(atoms: &[u8]) -> Known {
        let views = View::ALL.iter().filter_map(|(v, name, _)| find(atoms, name).map(|s| (s, *v))).collect();
        Known { views, load: find(atoms, "load"), decls: crate::shapes::DECLS.map(|d| find(atoms, d)), by: find(atoms, "by"), wave: find(atoms, "wave"), one: find(atoms, "one"), few: find(atoms, "few") }
    }

    pub fn view(&self, rel: i64) -> Option<View> {
        self.views.get(&rel).copied()
    }
}

impl Rule {
    /// Sentence 5: whether its steps read one document's views and relations. In the kernel this follows from the
    /// steps alone: it reads a view, and every relation step's and its head's first argument is that view's document
    /// variable.
    pub fn single_file(&self, known: &Known) -> bool {
        let d = self.steps.iter().find(|s| s.kind == Kind::Atom && known.view(s.rel).is_some()).and_then(|s| s.args.first().copied());
        let Some(Term::Var(d)) = d else { return false };
        self.args.first() == Some(&Term::Var(d)) && self.steps.iter().filter(|s| !matches!(s.kind, Kind::Cmp(_))).all(|s| s.args.first() == Some(&Term::Var(d)))
    }

    /// The relations its steps read (a `cmp`'s none).
    pub fn reads(&self) -> impl Iterator<Item = (Kind, i64)> + '_ {
        self.steps.iter().filter(|s| !matches!(s.kind, Kind::Cmp(_))).map(|s| (s.kind, s.rel))
    }

    fn refuse(&self, why: String) -> Refused {
        Refused { module: self.module, rule: self.id, why }
    }
}

/// A module: its stratum's place (`k_order`; `None`: no row) and its rules.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Module {
    pub k: Option<i64>,
    pub rules: Vec<Rule>,
}

impl Module {
    /// The relations its rules write.
    pub fn heads(&self) -> BTreeSet<i64> {
        self.rules.iter().map(|r| r.head).collect()
    }

    /// Whether its rules read their own heads (sentence 4: a Δfixpoint, else one wave).
    pub fn recursive(&self) -> bool {
        let heads = self.heads();
        self.rules.iter().any(|r| r.reads().any(|(_, rel)| heads.contains(&rel)))
    }
}

/// A program: its modules, by module id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Program {
    pub modules: BTreeMap<i64, Module>,
}

/// A step's place: its module, its rule, and its `(k, j)` (`(-1, -1)`: the head).
type StepKey = (i64, i64, i64, i64);

/// K-form's rows, a set a relation in `RELATIONS`' order, each row its module first.
pub type Rows = [BTreeSet<Vec<i64>>];

impl Program {
    /// The program K-form's rows `rows` hold, its symbols in `atoms`. Refused (B§7) as the module doc says.
    pub fn read(rows: &Rows, atoms: &[u8]) -> Result<Program, Refused> {
        assert_eq!(rows.len(), RELATIONS.len(), "a set a K-form relation");
        let at = |name: &str| RELATIONS.iter().position(|r| r.0 == name).expect("a K-form relation");
        let sym = |s: &str| find(atoms, s);
        let (atom, not, cmp) = (sym("atom"), sym("not"), sym("cmp"));
        let ops: BTreeMap<i64, Cmp> = Cmp::ALL.iter().filter_map(|(c, s)| sym(s).map(|i| (i, *c))).collect();
        // What each term is, by (module, term).
        let mut terms: BTreeMap<(i64, i64), Term> = BTreeMap::new();
        for r in &rows[at("k_var")] {
            terms.insert((r[0], r[1]), Term::Var(r[2]));
        }
        for name in ["k_str", "k_num"] {
            for r in &rows[at(name)] {
                terms.insert((r[0], r[1]), Term::Const(r[2]));
            }
        }
        for r in &rows[at("k_wild")] {
            terms.insert((r[0], r[1]), Term::Wild);
        }
        // Each rule's arguments by (module, rule, k, j), in their order.
        let mut args: BTreeMap<StepKey, Vec<(i64, i64)>> = BTreeMap::new();
        for r in &rows[at("k_arg")] {
            args.entry((r[0], r[1], r[2], r[3])).or_default().push((r[4], r[5]));
        }
        let term_list = |m: i64, rule: i64, key: (i64, i64)| -> Result<Vec<Term>, Refused> {
            let refuse = |why: String| Refused { module: m, rule, why };
            let mut list = args.get(&(m, rule, key.0, key.1)).cloned().unwrap_or_default();
            list.sort_unstable();
            list.iter()
                .enumerate()
                .map(|(i, &(n, t))| {
                    if n != i as i64 {
                        return Err(refuse(format!("step {key:?}: argument {i} missing")));
                    }
                    terms.get(&(m, t)).copied().ok_or_else(|| refuse(format!("step {key:?}: argument {i}, term {t}, is no variable, constant or _")))
                })
                .collect()
        };
        let mut modules: BTreeMap<i64, Module> = BTreeMap::new();
        for r in &rows[at("k_rule")] {
            let (m, id) = (r[0], r[1]);
            let rule = Rule { module: m, id, head: r[2], args: term_list(m, id, (-1, -1))?, steps: Vec::new() };
            modules.entry(m).or_default().rules.push(rule);
        }
        for r in &rows[at("k_step")] {
            let (m, id, key) = (r[0], r[1], (r[2], r[3]));
            let refuse = |why: String| Refused { module: m, rule: id, why };
            let kind = match Some(r[4]) {
                k if k == atom => Kind::Atom,
                k if k == not => Kind::Not,
                k if k == cmp => Kind::Cmp(*ops.get(&r[5]).ok_or_else(|| refuse(format!("step {key:?}: an unknown comparison")))?),
                _ => return Err(refuse(format!("step {key:?}: a kind that is not atom, not or cmp"))),
            };
            let rule = modules.get_mut(&m).and_then(|md| md.rules.iter_mut().find(|x| x.id == id)).ok_or_else(|| refuse(format!("step {key:?} of no rule")))?;
            rule.steps.push(Step { key, kind, rel: if matches!(kind, Kind::Cmp(_)) { 0 } else { r[5] }, args: Vec::new() });
        }
        for md in modules.values_mut() {
            for rule in &mut md.rules {
                rule.steps.sort_by_key(|s| s.key);
                for k in 0..rule.steps.len() {
                    let key = rule.steps[k].key;
                    rule.steps[k].args = term_list(rule.module, rule.id, key)?;
                }
            }
            md.rules.sort_by_key(|r| r.id);
        }
        for r in &rows[at("k_order")] {
            if let Some(md) = modules.get_mut(&r[0]) {
                md.k = Some(r[1]);
            }
        }
        let p = Program { modules };
        p.check(atoms)?;
        Ok(p)
    }

    /// Its modules in stratum order: `k_order`, then module id.
    pub fn strata(&self) -> Vec<i64> {
        let mut m: Vec<(Option<i64>, i64)> = self.modules.iter().map(|(&id, md)| (md.k, id)).collect();
        m.sort_unstable();
        m.into_iter().map(|(_, id)| id).collect()
    }

    /// The module writing each relation.
    pub fn writers(&self) -> BTreeMap<i64, i64> {
        self.modules.iter().flat_map(|(&m, md)| md.heads().into_iter().map(move |h| (h, m))).collect()
    }

    /// B§7's checks, each a refusal naming the module and the rule.
    fn check(&self, atoms: &[u8]) -> Result<(), Refused> {
        let n = |s: i64| crate::sym::name_of(atoms, s);
        let mut writers: BTreeMap<i64, i64> = BTreeMap::new();
        for (&m, md) in &self.modules {
            for h in md.heads() {
                if let Some(&other) = writers.get(&h) {
                    let rule = md.rules.iter().find(|r| r.head == h).map_or(-1, |r| r.id);
                    return Err(Refused { module: m, rule, why: format!("relation {} is written by module {} too: a relation is one stratum's", n(h), n(other)) });
                }
                writers.insert(h, m);
            }
        }
        if self.modules.len() > 1
            && let Some((&m, _)) = self.modules.iter().find(|(_, md)| md.k.is_none())
        {
            return Err(Refused { module: m, rule: -1, why: "no k_order row, in a program of several modules: strata run in k_order".into() });
        }
        let order: BTreeMap<i64, usize> = self.strata().into_iter().enumerate().map(|(i, m)| (m, i)).collect();
        for (&m, md) in &self.modules {
            for rule in &md.rules {
                let mut bound: BTreeSet<i64> = BTreeSet::new();
                for s in &rule.steps {
                    let vars = s.args.iter().filter_map(|t| if let Term::Var(v) = t { Some(*v) } else { None });
                    match s.kind {
                        Kind::Atom => bound.extend(vars),
                        Kind::Not | Kind::Cmp(_) => {
                            if let Some(v) = vars.clone().find(|v| !bound.contains(v)) {
                                return Err(rule.refuse(format!("step {:?}: variable {} is not bound by an earlier step", s.key, n(v))));
                            }
                            if matches!(s.kind, Kind::Cmp(_)) && s.args.len() != 2 {
                                return Err(rule.refuse(format!("step {:?}: a comparison of {} terms", s.key, s.args.len())));
                            }
                        }
                    }
                    if !matches!(s.kind, Kind::Cmp(_))
                        && let Some(&w) = writers.get(&s.rel)
                    {
                        let neg = s.kind == Kind::Not;
                        if w == m && neg {
                            return Err(rule.refuse(format!("step {:?} negates relation {}, its own stratum's head", s.key, n(s.rel))));
                        }
                        if w != m && order[&w] > order[&m] {
                            return Err(rule.refuse(format!("step {:?} reads relation {}, the head of module {}, a later stratum", s.key, n(s.rel), n(w))));
                        }
                        if w != m && md.k.is_some() && self.modules[&w].k == md.k {
                            return Err(rule.refuse(format!("step {:?} reads relation {}, the head of module {}, of the same stratum place", s.key, n(s.rel), n(w))));
                        }
                    }
                }
                for t in &rule.args {
                    match t {
                        Term::Var(v) if !bound.contains(v) => return Err(rule.refuse(format!("head variable {} is bound by no step", n(*v)))),
                        Term::Wild => return Err(rule.refuse("a head argument is _".into())),
                        _ => {}
                    }
                }
            }
        }
        Ok(())
    }
}
