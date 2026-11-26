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

/// An arithmetic operator: a relation the executor knows by its symbol (0686, K1-a; B§3, sentence 3: "an answer"),
/// its operands then its answer, read with its operands bound. A Pure answer: a step that reads one binds its last
/// column from the others, and it never changes. A division by zero and an overflow have no row.
///
/// `-` of two terms is `Neg`, `0 − a`. `Id` is `=` binding a variable no earlier step binds (`y = x + 1`): its answer is
/// its operand. Neither has a symbol of its own: `Program::read` makes them where the terms say.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Arith {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Neg,
    Id,
}

impl Arith {
    /// The operators a K-form step names by symbol; `Neg` and `Id` follow, by code only.
    pub const ALL: [(Arith, &'static str); 5] = [(Arith::Add, "+"), (Arith::Sub, "-"), (Arith::Mul, "*"), (Arith::Div, "/"), (Arith::Rem, "%")];
    const CODES: [Arith; 7] = [Arith::Add, Arith::Sub, Arith::Mul, Arith::Div, Arith::Rem, Arith::Neg, Arith::Id];

    /// Its operands' count.
    pub fn operands(self) -> usize {
        match self {
            Arith::Neg | Arith::Id => 1,
            _ => 2,
        }
    }

    /// Its answer for operands `x` (`operands()` of them), `None` where it has no row.
    pub fn apply(self, x: &[i64]) -> Option<i64> {
        match (self, x) {
            (Arith::Add, &[a, b]) => a.checked_add(b),
            (Arith::Sub, &[a, b]) => a.checked_sub(b),
            (Arith::Mul, &[a, b]) => a.checked_mul(b),
            (Arith::Div, &[a, b]) => a.checked_div(b),
            (Arith::Rem, &[a, b]) => a.checked_rem(b),
            (Arith::Neg, &[a]) => 0i64.checked_sub(a),
            (Arith::Id, &[a]) => Some(a),
            _ => unreachable!("an operator of {} operands, given {}", self.operands(), x.len()),
        }
    }

    pub fn code(self) -> i64 {
        Arith::CODES.iter().position(|&c| c == self).expect("an operator") as i64
    }

    pub fn of_code(c: i64) -> Option<Arith> {
        Arith::CODES.get(usize::try_from(c).ok()?).copied()
    }
}

/// An aggregate (step 9, L4-f): a delta operator the executor knows by its symbol, `count:` and the like, which no
/// relation can be named (B§3, sentence 3: an answer). A step of one folds the rows of one relation that match the
/// keys bound before it (P§10: maintained by changed groups), and binds its answer. `count` and `sum` of no rows are 0;
/// `min`, `max` and `mean` (a truncated quotient) of none have no row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AggOp {
    Count,
    Sum,
    Min,
    Max,
    Mean,
}

impl AggOp {
    pub const ALL: [(AggOp, &'static str); 5] = [(AggOp::Count, "count:"), (AggOp::Sum, "sum:"), (AggOp::Min, "min:"), (AggOp::Max, "max:"), (AggOp::Mean, "mean:")];

    pub fn code(self) -> i64 {
        AggOp::ALL.iter().position(|c| c.0 == self).expect("an aggregate") as i64
    }

    pub fn of_code(c: i64) -> Option<AggOp> {
        AggOp::ALL.get(usize::try_from(c).ok()?).map(|c| c.0)
    }

    /// Its answer over a group's folded values, `None` where it has no row.
    pub fn answer(self, f: &Fold) -> Option<i64> {
        match self {
            AggOp::Count => Some(f.n),
            AggOp::Sum => Some(f.sum),
            AggOp::Min => (f.n > 0).then_some(f.lo),
            AggOp::Max => (f.n > 0).then_some(f.hi),
            AggOp::Mean => (f.n > 0).then(|| f.sum / f.n),
        }
    }
}

/// A group's values folded as they are read: their count, sum, least and greatest.
#[derive(Clone, Copy, Debug)]
pub struct Fold {
    n: i64,
    sum: i64,
    lo: i64,
    hi: i64,
}

impl Default for Fold {
    fn default() -> Fold {
        Fold { n: 0, sum: 0, lo: i64::MAX, hi: i64::MIN }
    }
}

impl Fold {
    pub fn add(&mut self, v: i64) {
        self.n += 1;
        self.sum = self.sum.wrapping_add(v);
        self.lo = self.lo.min(v);
        self.hi = self.hi.max(v);
    }
}

/// What a step does: an atom of a relation, its negation, a comparison, an atom of an arithmetic operator (`Fn`), a
/// call of a function (`Call`, its place in `funcs::FUNCS`: its arguments, then its answer), or an aggregate (`Agg`)
/// of a relation: its arguments the relation's pattern, then its value's term and its answer's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Atom,
    Not,
    Cmp(Cmp),
    Fn(Arith),
    Call(u16),
    Agg(AggOp),
}

impl Kind {
    /// Whether it reads a relation's rows: a comparison and an operator read none.
    pub fn reads(self) -> bool {
        matches!(self, Kind::Atom | Kind::Not | Kind::Agg(_))
    }
}

/// A step of a rule's body: its key, what it does, the relation it reads (a `cmp`'s none) and its arguments.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Step {
    pub key: (i64, i64),
    pub kind: Kind,
    pub rel: i64,
    pub args: Vec<Term>,
}

impl Step {
    /// The columns of the relation it reads: an aggregate's pattern, its arguments but its value and its answer.
    pub fn arity(&self) -> usize {
        if matches!(self.kind, Kind::Agg(_)) { self.args.len() - 2 } else { self.args.len() }
    }
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
    /// `missing(path)`: the paths `load` asked that the world has no file at (a miss is a row, D3), written by the
    /// level's answer, read by rules (an include's resolution: the first candidate that is there).
    pub missing: Option<i64>,
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
        Known { views, load: find(atoms, "load"), missing: find(atoms, "missing"), decls: crate::shapes::DECLS.map(|d| find(atoms, d)), by: find(atoms, "by"), wave: find(atoms, "wave"), one: find(atoms, "one"), few: find(atoms, "few") }
    }

    pub fn view(&self, rel: i64) -> Option<View> {
        self.views.get(&rel).copied()
    }
}

impl Rule {
    /// The operators its terms make (`Arith`), in step order: a `-` of two terms is `Neg`, and an `=` of a variable no
    /// earlier step binds and a bound term or a constant is `Id`, its answer that variable (Soufflé's `y = x + 1`).
    fn operators(&mut self) {
        let mut bound: BTreeSet<i64> = BTreeSet::new();
        let var = |t: &Term| if let Term::Var(v) = *t { Some(v) } else { None };
        for s in &mut self.steps {
            match s.kind {
                Kind::Fn(Arith::Sub) if s.args.len() == 2 => s.kind = Kind::Fn(Arith::Neg),
                Kind::Cmp(Cmp::Eq) if s.args.len() == 2 => {
                    let free = |t: &Term| var(t).is_some_and(|v| !bound.contains(&v));
                    let given = |t: &Term| matches!(t, Term::Const(_)) || var(t).is_some_and(|v| bound.contains(&v));
                    for (a, b) in [(0, 1), (1, 0)] {
                        if free(&s.args[a]) && given(&s.args[b]) {
                            s.kind = Kind::Fn(Arith::Id);
                            s.args = vec![s.args[b], s.args[a]];
                            break;
                        }
                    }
                }
                _ => {}
            }
            match s.kind {
                Kind::Atom => bound.extend(s.args.iter().filter_map(var)),
                Kind::Fn(_) | Kind::Call(_) | Kind::Agg(_) => bound.extend(s.args.last().and_then(var)),
                _ => {}
            }
        }
    }

    /// Sentence 5: whether its steps read one document's views and relations. In the kernel this follows from the
    /// steps alone: it reads a view, and every relation step's and its head's first argument is that view's document
    /// variable.
    pub fn single_file(&self, known: &Known) -> bool {
        let d = self.steps.iter().find(|s| s.kind == Kind::Atom && known.view(s.rel).is_some()).and_then(|s| s.args.first().copied());
        let Some(Term::Var(d)) = d else { return false };
        self.args.first() == Some(&Term::Var(d)) && self.steps.iter().filter(|s| s.kind.reads()).all(|s| s.args.first() == Some(&Term::Var(d)))
    }

    /// The relations its steps read (a `cmp`'s and an operator's none).
    pub fn reads(&self) -> impl Iterator<Item = (Kind, i64)> + '_ {
        self.steps.iter().filter(|s| s.kind.reads()).map(|s| (s.kind, s.rel))
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
    /// The declared relations (`k_decl`, K1-d): each one's columns' types, in column order.
    pub decls: BTreeMap<i64, Vec<i64>>,
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
        let (atom, not, cmp, call) = (sym("atom"), sym("not"), sym("cmp"), sym("call"));
        let ops: BTreeMap<i64, Cmp> = Cmp::ALL.iter().filter_map(|(c, s)| sym(s).map(|i| (i, *c))).collect();
        let ariths: BTreeMap<i64, Arith> = Arith::ALL.iter().filter_map(|(c, s)| sym(s).map(|i| (i, *c))).collect();
        let aggs: BTreeMap<i64, AggOp> = AggOp::ALL.iter().filter_map(|(c, s)| sym(s).map(|i| (i, *c))).collect();
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
                k if k == atom => match (ariths.get(&r[5]), aggs.get(&r[5])) {
                    (Some(&op), _) => Kind::Fn(op),
                    (_, Some(&op)) => Kind::Agg(op),
                    _ => Kind::Atom,
                },
                k if k == not => Kind::Not,
                k if k == cmp => Kind::Cmp(*ops.get(&r[5]).ok_or_else(|| refuse(format!("step {key:?}: an unknown comparison")))?),
                k if k == call => {
                    let name = crate::sym::name_of(atoms, r[5]);
                    Kind::Call(crate::funcs::find(&name).ok_or_else(|| refuse(format!("step {key:?}: a call of {name}, which is no function")))? as u16)
                }
                _ => return Err(refuse(format!("step {key:?}: a kind that is not atom, not, cmp or call"))),
            };
            let rule = modules.get_mut(&m).and_then(|md| md.rules.iter_mut().find(|x| x.id == id)).ok_or_else(|| refuse(format!("step {key:?} of no rule")))?;
            rule.steps.push(Step { key, kind, rel: if kind.reads() { r[5] } else { 0 }, args: Vec::new() });
        }
        for md in modules.values_mut() {
            for rule in &mut md.rules {
                rule.steps.sort_by_key(|s| s.key);
                for k in 0..rule.steps.len() {
                    let key = rule.steps[k].key;
                    rule.steps[k].args = term_list(rule.module, rule.id, key)?;
                    // An aggregate as K1 writes it: the relation it folds, its answer, its value, then the pattern. As
                    // the executor reads it: the relation the step's, the pattern, the value and the answer.
                    // A call as K1 writes it: its answer, then its arguments. As the executor reads it: its arguments,
                    // then its answer, as an operator's.
                    if let Kind::Call(f) = rule.steps[k].kind {
                        let s = &mut rule.steps[k];
                        let n = s.args.len().saturating_sub(1);
                        if !crate::funcs::takes(usize::from(f), n) {
                            let name = crate::funcs::FUNCS[usize::from(f)].0;
                            return Err(Refused { module: rule.module, rule: rule.id, why: format!("step {key:?}: {name} of {n} arguments") });
                        }
                        s.args.rotate_left(1);
                    }
                    if let Kind::Agg(op) = rule.steps[k].kind {
                        let s = &mut rule.steps[k];
                        let refuse = |why: String| Refused { module: rule.module, rule: rule.id, why };
                        let (Some(&Term::Const(rel)), true) = (s.args.first(), s.args.len() >= 3) else {
                            return Err(refuse(format!("step {key:?}: an aggregate {op:?} that names no relation")));
                        };
                        if rel < 0 {
                            return Err(refuse(format!("step {key:?}: aggregate {op:?} of a body that is not one atom: write the body as a relation of an earlier file, and aggregate that")));
                        }
                        let (answer, value) = (s.args[1], s.args[2]);
                        s.rel = rel;
                        s.args = s.args[3..].iter().copied().chain([value, answer]).collect();
                    }
                }
                rule.operators();
            }
            md.rules.sort_by_key(|r| r.id);
        }
        for r in &rows[at("k_order")] {
            if let Some(md) = modules.get_mut(&r[0]) {
                if let Some(k) = md.k.filter(|&k| k != r[1]) {
                    return Err(Refused { module: r[0], rule: -1, why: format!("two k_order rows, strata {k} and {}: a module is one stratum", r[1]) });
                }
                md.k = Some(r[1]);
            }
        }
        // The declarations: each module's columns of a relation, dense from 0, one type a column (a module's
        // declarations of one relation are the union of their columns); two modules' that differ are refused.
        let mut by: BTreeMap<(i64, i64), BTreeMap<i64, i64>> = BTreeMap::new();
        for r in &rows[at("k_decl")] {
            if let Some(t) = by.entry((r[0], r[1])).or_default().insert(r[2], r[3]).filter(|&t| t != r[3]) {
                let (a, b) = (crate::sym::name_of(atoms, t), crate::sym::name_of(atoms, r[3]));
                return Err(Refused { module: r[0], rule: -1, why: format!("relation {}'s column {} is declared of type {a} and of {b}", crate::sym::name_of(atoms, r[1]), r[2]) });
            }
        }
        let mut decls: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
        for ((m, rel), cols) in by {
            let name = crate::sym::name_of(atoms, rel);
            if cols.keys().copied().ne(0..cols.len() as i64) {
                return Err(Refused { module: m, rule: -1, why: format!("relation {name} is declared with its columns not 0 to {}", cols.len() - 1) });
            }
            let types: Vec<i64> = cols.into_values().collect();
            if let Some(other) = decls.get(&rel).filter(|o| **o != types) {
                return Err(Refused { module: m, rule: -1, why: format!("relation {name} is declared with {} columns and with {}, or of other types", types.len(), other.len()) });
            }
            decls.insert(rel, types);
        }
        let p = Program { modules, decls };
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
                        Kind::Fn(op) => {
                            if s.args.len() != op.operands() + 1 {
                                return Err(rule.refuse(format!("step {:?}: operator {op:?} of {} terms, not {}", s.key, s.args.len(), op.operands() + 1)));
                            }
                            let ins = s.args[..op.operands()].iter().filter_map(|t| if let Term::Var(v) = t { Some(*v) } else { None });
                            if let Some(v) = ins.clone().find(|v| !bound.contains(v)) {
                                return Err(rule.refuse(format!("step {:?}: operand {} is not bound by an earlier step", s.key, n(v))));
                            }
                            bound.extend(vars);
                        }
                        Kind::Call(_) => {
                            let ins = s.args[..s.args.len() - 1].iter().filter_map(|t| if let Term::Var(v) = t { Some(*v) } else { None });
                            if let Some(v) = ins.clone().find(|v| !bound.contains(v)) {
                                return Err(rule.refuse(format!("step {:?}: argument {} is not bound by an earlier step", s.key, n(v))));
                            }
                            bound.extend(vars);
                        }
                        // An aggregate: its pattern's bound variables are its keys, the rest its own; its value one
                        // of the pattern's (any for `count`); it binds its answer only.
                        Kind::Agg(op) => {
                            let n = s.arity();
                            let var = |t: &Term| if let Term::Var(v) = *t { Some(v) } else { None };
                            // A variable of its pattern found elsewhere in the rule is a key: bound before it.
                            let elsewhere = rule.steps.iter().filter(|o| o.key != s.key).flat_map(|o| &o.args).chain(&rule.args).filter_map(var).collect::<BTreeSet<i64>>();
                            if let Some(v) = s.args[..n].iter().filter_map(var).find(|v| elsewhere.contains(v) && !bound.contains(v)) {
                                return Err(rule.refuse(format!("step {:?}: aggregate {op:?}'s key {} is not bound by an earlier step", s.key, crate::sym::name_of(atoms, v))));
                            }
                            if op != AggOp::Count && !matches!(s.args[n], Term::Var(v) if s.args[..n].contains(&Term::Var(v))) {
                                return Err(rule.refuse(format!("step {:?}: aggregate {op:?}'s value is not a variable of its body's atom", s.key)));
                            }
                            bound.extend(var(&s.args[n + 1]));
                        }
                        Kind::Not | Kind::Cmp(_) => {
                            if let Some(v) = vars.clone().find(|v| !bound.contains(v)) {
                                return Err(rule.refuse(format!("step {:?}: variable {} is not bound by an earlier step", s.key, n(v))));
                            }
                            if matches!(s.kind, Kind::Cmp(_)) && s.args.len() != 2 {
                                return Err(rule.refuse(format!("step {:?}: a comparison of {} terms", s.key, s.args.len())));
                            }
                        }
                    }
                    if s.kind.reads()
                        && let Some(&w) = writers.get(&s.rel)
                    {
                        let neg = s.kind == Kind::Not;
                        if w == m && neg {
                            return Err(rule.refuse(format!("step {:?} negates relation {}, its own stratum's head", s.key, n(s.rel))));
                        }
                        if w == m && matches!(s.kind, Kind::Agg(_)) {
                            return Err(rule.refuse(format!("step {:?} aggregates relation {}, its own stratum's head: an aggregate reads an earlier stratum", s.key, n(s.rel))));
                        }
                        if w != m && order[&w] > order[&m] {
                            return Err(rule.refuse(format!("step {:?} reads relation {}, the head of module {}, a later stratum", s.key, n(s.rel), n(w))));
                        }
                        if w != m && md.k.is_some() && self.modules[&w].k == md.k {
                            return Err(rule.refuse(format!("step {:?} reads relation {}, the head of module {}, of the same stratum place", s.key, n(s.rel), n(w))));
                        }
                    }
                }
                // A declared relation's arity, in its head and its steps.
                let arity = |rel: i64, n: usize, at: &str| -> Result<(), Refused> {
                    match self.decls.get(&rel) {
                        Some(cols) if cols.len() != n => Err(rule.refuse(format!("{at} {} of {n} columns, declared with {}", crate::sym::name_of(atoms, rel), cols.len()))),
                        _ => Ok(()),
                    }
                };
                arity(rule.head, rule.args.len(), "head")?;
                for s in rule.steps.iter().filter(|s| s.kind.reads()) {
                    arity(s.rel, s.arity(), &format!("step {:?} reads", s.key))?;
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
