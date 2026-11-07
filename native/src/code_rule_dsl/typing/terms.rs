//! Type terms (03 §13 "Type terms", "Generics are first-class", "Rule predicates"; I12, I13, I14; list step 29).
//!
//! Every term lives in one space of interned terms: a term is a number (`Tid`), and two equal terms are the same
//! number. A term is one of:
//!
//! ```text
//! named      list[int]          a named type with arguments (`name` is an atom of the single interner)
//! union      int | str          members flattened, sorted, deduplicated; `open`: more members may exist
//! top        ⊤                  nothing is known
//! var        T <: Hashable      a type variable with its bound (⊤ when it has none)
//! callable   (int, str) -> bool parameters, then the return
//! tuple      tuple[int, str]
//! literal    Literal["a"]       `name` is the literal's atom
//! reference  "Foo"              a forward reference by name, not yet resolved
//! ```
//!
//! Widening keeps the space finite: a union of more than `MEMBERS` members is ⊤, and a subterm deeper than
//! `DEPTH` is ⊤.
//!
//! Traits (hashable, awaitable, iterable, …) are bits. Each term has a `must` mask (the trait certainly holds)
//! and a `may` mask (it may hold). ⊤ and open unions must nothing and may everything. A rule asks `proven: X`
//! (X is true: in `must`) or `possible: X` (X is not false: in `may`); `check` gives the three values.
//!
//! Instantiation (`instantiate`) is local and one-way, in 03 §13's order: the receiver, the explicit type
//! arguments, one-way matching of parameters against arguments (no unification), one callback step, and then
//! each unbound variable falls back to its bound.
//!
//! Stored as `type_term(tid, ctor, name_atom, arity)` and `type_arg(tid, i, arg)` (`write`).

use std::collections::HashMap;

use crate::content_store::atoms::Sym;

pub(crate) type Tid = u32;

/// Widening: a union of more members is ⊤.
pub(crate) const MEMBERS: usize = 8;
/// Widening: a subterm below this depth is ⊤.
pub(crate) const DEPTH: usize = 4;
/// No name: the `name` of a term without one.
pub(crate) const NO_NAME: Sym = Sym::MAX;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Ord, PartialOrd)]
pub(crate) enum Ctor {
    Named = 0,
    Union = 1,
    OpenUnion = 2,
    Top = 3,
    Var = 4,
    Callable = 5,
    Tuple = 6,
    Literal = 7,
    Reference = 8,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct Term {
    pub(crate) ctor: Ctor,
    pub(crate) name: Sym,
    pub(crate) args: Vec<Tid>,
}

/// Three-valued: a check through unknown is neither true nor false.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Truth {
    True,
    False,
    Unknown,
}

impl Truth {
    pub(crate) fn not(self) -> Truth {
        match self {
            Truth::True => Truth::False,
            Truth::False => Truth::True,
            Truth::Unknown => Truth::Unknown,
        }
    }
    /// `proven:` this.
    pub(crate) fn proven(self) -> bool {
        self == Truth::True
    }
    /// `possible:` this.
    pub(crate) fn possible(self) -> bool {
        self != Truth::False
    }
}

pub(crate) struct Terms {
    terms: Vec<Term>,
    index: HashMap<Term, Tid>,
    /// Declared traits of named and literal terms, by (ctor, name): (must, may).
    traits: HashMap<(Ctor, Sym), (u64, u64)>,
    top: Tid,
}

impl Default for Terms {
    fn default() -> Self {
        let mut t = Terms { terms: Vec::new(), index: HashMap::new(), traits: HashMap::new(), top: 0 };
        t.top = t.intern(Term { ctor: Ctor::Top, name: NO_NAME, args: Vec::new() });
        t
    }
}

impl Terms {
    fn intern(&mut self, term: Term) -> Tid {
        if let Some(&t) = self.index.get(&term) {
            return t;
        }
        let t = self.terms.len() as Tid;
        self.terms.push(term.clone());
        self.index.insert(term, t);
        t
    }

    /// A term, widened below `DEPTH`.
    fn make(&mut self, ctor: Ctor, name: Sym, args: Vec<Tid>) -> Tid {
        let args = args.into_iter().map(|a| self.cut(a, DEPTH - 1)).collect();
        self.intern(Term { ctor, name, args })
    }

    /// `t` with every subterm deeper than `depth` widened to ⊤.
    fn cut(&mut self, t: Tid, depth: usize) -> Tid {
        if self.depth(t) <= depth {
            return t;
        }
        // A term with arguments does not fit in one level.
        if depth <= 1 {
            return self.top;
        }
        let term = self.terms[t as usize].clone();
        let args = term.args.iter().map(|&a| self.cut(a, depth - 1)).collect();
        match term.ctor {
            Ctor::Union | Ctor::OpenUnion => self.union_of(args, term.ctor == Ctor::OpenUnion),
            _ => self.intern(Term { ctor: term.ctor, name: term.name, args }),
        }
    }

    pub(crate) fn get(&self, t: Tid) -> &Term {
        &self.terms[t as usize]
    }
    pub(crate) fn len(&self) -> usize {
        self.terms.len()
    }
    pub(crate) fn top(&self) -> Tid {
        self.top
    }
    pub(crate) fn named(&mut self, name: Sym, args: &[Tid]) -> Tid {
        self.make(Ctor::Named, name, args.to_vec())
    }
    pub(crate) fn var(&mut self, name: Sym, bound: Option<Tid>) -> Tid {
        let bound = bound.unwrap_or(self.top);
        self.make(Ctor::Var, name, vec![bound])
    }
    pub(crate) fn callable(&mut self, params: &[Tid], ret: Tid) -> Tid {
        let mut args = params.to_vec();
        args.push(ret);
        self.make(Ctor::Callable, NO_NAME, args)
    }
    pub(crate) fn tuple(&mut self, items: &[Tid]) -> Tid {
        self.make(Ctor::Tuple, NO_NAME, items.to_vec())
    }
    pub(crate) fn literal(&mut self, value: Sym) -> Tid {
        self.make(Ctor::Literal, value, Vec::new())
    }
    pub(crate) fn reference(&mut self, name: Sym) -> Tid {
        self.make(Ctor::Reference, name, Vec::new())
    }

    /// The union of `members`: nested unions flattened (an open one opens it), ⊤ absorbing, sorted and
    /// deduplicated; one closed member is itself; past `MEMBERS` members, ⊤.
    pub(crate) fn union(&mut self, members: &[Tid], open: bool) -> Tid {
        self.union_of(members.to_vec(), open)
    }
    fn union_of(&mut self, members: Vec<Tid>, mut open: bool) -> Tid {
        let mut flat = Vec::new();
        for m in members {
            match self.terms[m as usize].ctor {
                Ctor::Top => return self.top,
                Ctor::Union => flat.extend(self.terms[m as usize].args.clone()),
                Ctor::OpenUnion => {
                    open = true;
                    flat.extend(self.terms[m as usize].args.clone());
                }
                _ => flat.push(m),
            }
        }
        flat.sort_unstable();
        flat.dedup();
        if flat.len() > MEMBERS {
            return self.top;
        }
        if flat.len() == 1 && !open {
            return flat[0];
        }
        let ctor = if open { Ctor::OpenUnion } else { Ctor::Union };
        self.intern(Term { ctor, name: NO_NAME, args: flat })
    }

    pub(crate) fn depth(&self, t: Tid) -> usize {
        let term = &self.terms[t as usize];
        match term.ctor {
            // A union's members are alternatives at its own level.
            Ctor::Union | Ctor::OpenUnion => term.args.iter().map(|&a| self.depth(a)).max().unwrap_or(1),
            _ => 1 + term.args.iter().map(|&a| self.depth(a)).max().unwrap_or(0),
        }
    }

    /// Declares a named type's (or a literal's) traits: `must` certainly hold, `may` may (`must` ⊆ `may`).
    pub(crate) fn declare_traits(&mut self, ctor: Ctor, name: Sym, must: u64, may: u64) {
        self.traits.insert((ctor, name), (must, may | must));
    }

    /// A term's (must, may) trait masks. Undeclared named types and references must nothing and may everything;
    /// a variable has its bound's (bounds are must-traits); a union must what every member must and may what any
    /// member may; an open union may anything as well.
    pub(crate) fn traits(&self, t: Tid) -> (u64, u64) {
        let term = &self.terms[t as usize];
        match term.ctor {
            Ctor::Top | Ctor::Reference => (0, u64::MAX),
            Ctor::Named | Ctor::Literal => self.traits.get(&(term.ctor, term.name)).copied().unwrap_or((0, u64::MAX)),
            Ctor::Var => self.traits(term.args[0]),
            Ctor::Callable | Ctor::Tuple => self.traits.get(&(term.ctor, NO_NAME)).copied().unwrap_or((0, u64::MAX)),
            Ctor::Union | Ctor::OpenUnion => {
                let (mut must, mut may) = (u64::MAX, 0);
                for &m in &term.args {
                    let (a, b) = self.traits(m);
                    must &= a;
                    may |= b;
                }
                if term.ctor == Ctor::OpenUnion { (0, u64::MAX) } else { (must, may) }
            }
        }
    }

    /// Whether trait `bit` holds of `t`.
    pub(crate) fn check(&self, t: Tid, bit: u32) -> Truth {
        let (must, may) = self.traits(t);
        if must & (1 << bit) != 0 {
            Truth::True
        } else if may & (1 << bit) == 0 {
            Truth::False
        } else {
            Truth::Unknown
        }
    }

    /// `t`'s members: a union's, or `t` alone; and whether more may exist (an open union, ⊤).
    pub(crate) fn members(&self, t: Tid) -> (Vec<Tid>, bool) {
        let term = &self.terms[t as usize];
        match term.ctor {
            Ctor::Union => (term.args.clone(), false),
            Ctor::OpenUnion => (term.args.clone(), true),
            Ctor::Top => (Vec::new(), true),
            _ => (vec![t], false),
        }
    }

    /// `some`: the predicate holds of a member. An unknown member, or more members, keep it from being false.
    pub(crate) fn some(&self, t: Tid, pred: &dyn Fn(Tid) -> Truth) -> Truth {
        let (members, open) = self.members(t);
        let mut out = if open { Truth::Unknown } else { Truth::False };
        for m in members {
            match pred(m) {
                Truth::True => return Truth::True,
                Truth::Unknown => out = Truth::Unknown,
                Truth::False => {}
            }
        }
        out
    }

    /// `every`: the predicate holds of each member. An unknown member, or more members, keep it from being true.
    pub(crate) fn every(&self, t: Tid, pred: &dyn Fn(Tid) -> Truth) -> Truth {
        self.some(t, &|m| pred(m).not()).not()
    }

    /// `t` with each variable bound in `subst` replaced.
    pub(crate) fn substitute(&mut self, t: Tid, subst: &HashMap<Sym, Tid>) -> Tid {
        let term = self.terms[t as usize].clone();
        if term.ctor == Ctor::Var {
            return subst.get(&term.name).copied().unwrap_or(t);
        }
        if term.args.is_empty() {
            return t;
        }
        let args: Vec<Tid> = term.args.iter().map(|&a| self.substitute(a, subst)).collect();
        match term.ctor {
            Ctor::Union | Ctor::OpenUnion => self.union_of(args, term.ctor == Ctor::OpenUnion),
            _ => self.make(term.ctor, term.name, args),
        }
    }

    /// The variables of `t`, in order of first appearance.
    pub(crate) fn vars(&self, t: Tid, out: &mut Vec<(Sym, Tid)>) {
        let term = &self.terms[t as usize];
        if term.ctor == Ctor::Var {
            if !out.iter().any(|(n, _)| *n == term.name) {
                out.push((term.name, term.args[0]));
            }
            return;
        }
        for &a in &term.args {
            self.vars(a, out);
        }
    }

    /// One-way matching: binds `pattern`'s unbound variables to the parts of `actual` they stand for. Named types
    /// match by name and arity, argument by argument; a mismatch binds nothing (no unification, no error).
    pub(crate) fn match_into(&self, pattern: Tid, actual: Tid, subst: &mut HashMap<Sym, Tid>) {
        let (p, a) = (&self.terms[pattern as usize], &self.terms[actual as usize]);
        if p.ctor == Ctor::Var {
            subst.entry(p.name).or_insert(actual);
            return;
        }
        if p.ctor == a.ctor && p.name == a.name && p.args.len() == a.args.len() && p.ctor != Ctor::Union && p.ctor != Ctor::OpenUnion {
            for (&x, &y) in p.args.iter().zip(&a.args) {
                self.match_into(x, y, subst);
            }
        }
    }

    /// A generic callable's return type at one call (03 §13 order):
    /// 1. the receiver, matched against the first parameter (`self`);
    /// 2. the explicit type arguments, bound to the variables in order of first appearance;
    /// 3. each argument matched one way against its parameter;
    /// 4. one callback step: a callable argument whose parameter is callable binds the parameter's return from the
    ///    argument's return, after its parameters are substituted;
    /// 5. each still unbound variable is its bound.
    pub(crate) fn instantiate(&mut self, callable: Tid, receiver: Option<Tid>, explicit: &[Tid], args: &[Tid]) -> Tid {
        let term = self.terms[callable as usize].clone();
        if term.ctor != Ctor::Callable {
            return self.top;
        }
        let (params, ret) = term.args.split_at(term.args.len() - 1);
        let ret = ret[0];
        let mut subst = HashMap::new();
        let mut rest = params;
        if let Some(r) = receiver
            && let Some((&first, tail)) = params.split_first()
        {
            self.match_into(first, r, &mut subst);
            rest = tail;
        }
        let mut vars = Vec::new();
        self.vars(callable, &mut vars);
        for ((name, _), &t) in vars.iter().filter(|(n, _)| !subst.contains_key(n)).zip(explicit).collect::<Vec<_>>() {
            subst.insert(*name, t);
        }
        let mut callbacks = Vec::new();
        for (&p, &a) in rest.iter().zip(args) {
            if self.terms[p as usize].ctor == Ctor::Callable && self.terms[a as usize].ctor == Ctor::Callable {
                callbacks.push((p, a));
            }
            self.match_into(p, a, &mut subst);
        }
        for (p, a) in callbacks {
            let (pret, aret) = (*self.terms[p as usize].args.last().unwrap(), *self.terms[a as usize].args.last().unwrap());
            let pret = self.substitute(pret, &subst);
            self.match_into(pret, aret, &mut subst);
        }
        for (name, bound) in vars {
            subst.entry(name).or_insert(bound);
        }
        self.substitute(ret, &subst)
    }

    /// Writes every term as `type_term(tid, ctor, name_atom, arity)` and `type_arg(tid, i, arg)` in `schema`.
    pub(crate) fn write(&self, db: &rusqlite::Connection, schema: &str) -> rusqlite::Result<()> {
        db.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {schema}.type_term(tid INTEGER PRIMARY KEY, ctor INTEGER, name_atom INTEGER, arity INTEGER);
             CREATE TABLE IF NOT EXISTS {schema}.type_arg(tid INTEGER, i INTEGER, arg INTEGER, PRIMARY KEY (tid, i)) WITHOUT ROWID;"
        ))?;
        let mut term = db.prepare(&format!("INSERT OR IGNORE INTO {schema}.type_term VALUES (?1, ?2, ?3, ?4)"))?;
        let mut arg = db.prepare(&format!("INSERT OR IGNORE INTO {schema}.type_arg VALUES (?1, ?2, ?3)"))?;
        for (tid, t) in self.terms.iter().enumerate() {
            let name = if t.name == NO_NAME { None } else { Some(i64::from(t.name)) };
            term.execute(rusqlite::params![tid as i64, t.ctor as i64, name, t.args.len() as i64])?;
            for (i, &a) in t.args.iter().enumerate() {
                arg.execute(rusqlite::params![tid as i64, i as i64, i64::from(a)])?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
