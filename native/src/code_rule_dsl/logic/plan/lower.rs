//! A program lowered to the integers of the base relations (decision 0009; O13).
//!
//! Every column gets a domain. Declared types give `int`, `node` and `sym`; the base relations' symbol columns are
//! names, except `node.kind` (kind atoms) and `field.f` (field atoms); a derived `sym` column takes the domain of
//! the variables written into it, to a fixpoint over the rules, and is a name if nothing says otherwise. String
//! constants then become integers by the domain they meet: name atoms (interned), or kind and field atoms in the
//! program's dialect (`-1`, which matches nothing, for a name the grammar lacks). Planning and SQL see only integers.

use super::super::front::{Atom, Literal, Program, Term, Value};
use super::{Error, Result};
use crate::code_rule_dsl::facts::core::kind_atom;
use crate::code_rule_dsl::facts::vtab::syntax::field_atom;
use crate::code_rule_dsl::language::{GrammarDialect, KindCatalog, tape_tag};
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Domain {
    Int,
    Node,
    Kind,
    Field,
    Name,
}

impl Domain {
    fn symbolic(self) -> bool {
        matches!(self, Domain::Kind | Domain::Field | Domain::Name)
    }
}

/// Each relation's column domains.
pub(in crate::code_rule_dsl) type Domains = BTreeMap<String, Vec<Domain>>;

/// A base relation's symbol column that is not a name.
fn base_domain(rel: &str, col: usize) -> Option<Domain> {
    match (rel, col) {
        ("node", 2) => Some(Domain::Kind),
        ("field", 1) => Some(Domain::Field),
        _ => None,
    }
}

fn atoms(l: &Literal) -> impl Iterator<Item = &Atom> {
    match l {
        Literal::Atom(a) | Literal::Agg { atom: a, .. } => Some(a),
        _ => None,
    }
    .into_iter()
}

/// A rule's variables' domains, as they are found.
struct Vars<'a> {
    head: &'a Atom,
    map: HashMap<String, Domain>,
}

impl Vars<'_> {
    /// Records `$v` in `d`; whether that is new. Two symbol domains for one variable is an error.
    fn set(&mut self, v: &str, d: Domain) -> Result<bool> {
        match self.map.get(v) {
            Some(&e) if e.symbolic() && d.symbolic() && e != d => {
                Err(Error::Plan(format!("rule for {}: ${v} is a {e:?} and a {d:?}", self.head)))
            }
            Some(_) => Ok(false),
            None => {
                self.map.insert(v.to_owned(), d);
                Ok(true)
            }
        }
    }

    /// The domains a builtin call gives its variables: `@cat`'s parts are names, `flat_bin`'s operator too.
    fn call(&mut self, t: &Term) -> Result<bool> {
        let mut changed = false;
        if let Term::Call(f, args) = t {
            for (i, a) in args.iter().enumerate() {
                if let Term::Var(v) = a
                    && (f == "cat" || (f == "flat_bin" && i == 0))
                {
                    changed |= self.set(v, Domain::Name)?;
                }
                changed |= self.call(a)?;
            }
        }
        Ok(changed)
    }
}

/// The domains of a rule's variables, given the columns'.
fn rule_vars(prog: &Program, cols: &BTreeMap<String, Vec<Option<Domain>>>, rule: usize) -> Result<HashMap<String, Domain>> {
    let r = &prog.rules[rule];
    let mut vars = Vars { head: &r.head, map: HashMap::new() };
    let mut changed = true;
    while changed {
        changed = false;
        for a in std::iter::once(&r.head).chain(r.body.iter().flat_map(atoms)) {
            for (c, t) in a.args.iter().enumerate() {
                if let (Term::Var(v), Some(Some(d))) = (t, cols.get(&a.rel).and_then(|ds| ds.get(c))) {
                    changed |= vars.set(v, *d)?;
                }
                changed |= vars.call(t)?;
            }
        }
        for l in &r.body {
            match l {
                Literal::Cmp { l, r, .. } => {
                    changed |= vars.call(l)?;
                    changed |= vars.call(r)?;
                    if let (Term::Var(x), Term::Var(y)) = (l, r) {
                        if let Some(&d) = vars.map.get(x) {
                            changed |= vars.set(y, d)?;
                        }
                        if let Some(&d) = vars.map.get(y) {
                            changed |= vars.set(x, d)?;
                        }
                    }
                    // `$x = @cat(...)`: the whole is a name.
                    if let (Term::Var(x), Term::Call(f, _)) = (l, r)
                        && f == "cat"
                    {
                        changed |= vars.set(x, Domain::Name)?;
                    }
                }
                Literal::Nearest { n, a, .. } => {
                    changed |= vars.set(n, Domain::Node)?;
                    changed |= vars.set(a, Domain::Node)?;
                }
                _ => {}
            }
        }
    }
    Ok(vars.map)
}

/// Every column's domain (see the module comment).
pub(in crate::code_rule_dsl) fn domains(prog: &Program) -> Result<Domains> {
    let mut cols: BTreeMap<String, Vec<Option<Domain>>> = BTreeMap::new();
    for (rel, decl) in &prog.decls {
        let input = prog.inputs.contains(rel);
        let ds = decl
            .iter()
            .enumerate()
            .map(|(c, (_, t))| match t.as_str() {
                "int" | "flat" => Ok(Some(Domain::Int)),
                "node" => Ok(Some(Domain::Node)),
                "sym" if input => Ok(Some(base_domain(rel, c).unwrap_or(Domain::Name))),
                "sym" => Ok(None),
                _ => Err(Error::Plan(format!("{rel}: no domain for {t}"))),
            })
            .collect::<Result<_>>()?;
        cols.insert(rel.clone(), ds);
    }
    let mut changed = true;
    while changed {
        changed = false;
        for i in 0..prog.rules.len() {
            let vars = rule_vars(prog, &cols, i)?;
            let r = &prog.rules[i];
            for a in std::iter::once(&r.head).chain(r.body.iter().flat_map(atoms)) {
                let Some(ds) = cols.get_mut(&a.rel) else { continue };
                for (c, t) in a.args.iter().enumerate() {
                    if let (Term::Var(v), Some(slot @ None)) = (t, ds.get_mut(c))
                        && let Some(&d) = vars.get(v)
                    {
                        *slot = Some(d);
                        changed = true;
                    }
                }
            }
        }
    }
    Ok(cols.into_iter().map(|(rel, ds)| (rel, ds.into_iter().map(|d| d.unwrap_or(Domain::Name)).collect())).collect())
}

/// A program lowered for one dialect.
pub(in crate::code_rule_dsl) struct Lowered {
    pub(in crate::code_rule_dsl) program: Program,
    pub(in crate::code_rule_dsl) domains: Domains,
    /// The name atoms of `flat_bin`'s operators `+`, `-` and `*`.
    pub(in crate::code_rule_dsl) ops: [i64; 3],
    pub(in crate::code_rule_dsl) dialect: GrammarDialect,
}

struct Constants<'a> {
    tag: u8,
    kinds: KindCatalog,
    names: &'a HashMap<String, i64>,
}

impl Constants<'_> {
    fn of(&self, s: &str, d: Domain) -> Result<i64> {
        Ok(match d {
            Domain::Name => self.names[s],
            Domain::Kind => match self.kinds.kind_id(s) {
                0 => -1,
                code => i64::from(kind_atom(self.tag, code)),
            },
            Domain::Field => self.kinds.field_id(s).map_or(-1, |code| i64::from(field_atom(self.tag, code))),
            Domain::Int | Domain::Node => return Err(Error::Plan(format!("the string \"{s}\" stands where an integer goes"))),
        })
    }
    fn term(&self, t: &mut Term, d: Option<Domain>) -> Result<()> {
        match t {
            Term::Str(s) => *t = Term::Int(self.of(s, d.unwrap_or(Domain::Name))?),
            Term::Call(f, args) => {
                let f = f.clone();
                for (i, a) in args.iter_mut().enumerate() {
                    let d = (f == "cat" || (f == "flat_bin" && i == 0)).then_some(Domain::Name);
                    self.term(a, d)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn strings(t: &Term, out: &mut Vec<String>) {
    match t {
        Term::Str(s) => out.push(s.clone()),
        Term::Call(_, args) => args.iter().for_each(|a| strings(a, out)),
        _ => {}
    }
}

/// Lowers `prog` for `dialect`; `intern` gives the name atoms of strings, interning new ones.
pub(in crate::code_rule_dsl) fn lower(
    prog: &Program,
    dialect: GrammarDialect,
    intern: &mut dyn FnMut(&[&str]) -> Result<Vec<u32>>,
) -> Result<Lowered> {
    let domains = domains(prog)?;
    let tag = tape_tag(dialect).ok_or_else(|| Error::Plan("the dialect has no tape tag".to_owned()))?;
    // Every string a name could be, interned in one call: interning a kind's name too costs one atom.
    let mut all = vec!["+".to_owned(), "-".to_owned(), "*".to_owned()];
    for r in &prog.rules {
        for a in std::iter::once(&r.head).chain(r.body.iter().flat_map(atoms)) {
            a.args.iter().for_each(|t| strings(t, &mut all));
        }
        for l in &r.body {
            if let Literal::Cmp { l, r, .. } = l {
                strings(l, &mut all);
                strings(r, &mut all);
            }
        }
    }
    for rows in prog.facts.values() {
        for v in rows.iter().flatten() {
            if let Value::Str(s) = v {
                all.push(s.clone());
            }
        }
    }
    all.sort();
    all.dedup();
    let atoms_of = intern(&all.iter().map(String::as_str).collect::<Vec<_>>())?;
    let names: HashMap<String, i64> = all.into_iter().zip(atoms_of.into_iter().map(i64::from)).collect();
    let k = Constants { tag, kinds: dialect.kinds(), names: &names };
    let mut out = prog.clone();
    for i in 0..out.rules.len() {
        let vars = {
            let cols = domains.iter().map(|(r, ds)| (r.clone(), ds.iter().copied().map(Some).collect())).collect();
            rule_vars(prog, &cols, i)?
        };
        let rule = &mut out.rules[i];
        let col = |rel: &str, c: usize| domains.get(rel).and_then(|ds| ds.get(c)).copied();
        for (c, t) in rule.head.args.iter_mut().enumerate() {
            k.term(t, col(&rule.head.rel, c))?;
        }
        for l in &mut rule.body {
            match l {
                Literal::Atom(a) | Literal::Agg { atom: a, .. } => {
                    for (c, t) in a.args.iter_mut().enumerate() {
                        k.term(t, col(&a.rel, c))?;
                    }
                }
                Literal::Cmp { l, r, .. } => {
                    let of = |t: &Term| if let Term::Var(v) = t { vars.get(v).copied() } else { None };
                    let (dl, dr) = (of(r), of(l));
                    k.term(l, dl)?;
                    k.term(r, dr)?;
                }
                _ => {}
            }
        }
    }
    for (rel, rows) in &mut out.facts {
        for row in rows {
            for (c, v) in row.iter_mut().enumerate() {
                if let Value::Str(s) = v {
                    let d = domains.get(rel.as_str()).and_then(|ds| ds.get(c)).copied().unwrap_or(Domain::Name);
                    *v = Value::Int(k.of(s, d)?);
                }
            }
        }
    }
    for members in out.kindsets.values_mut() {
        for m in members.iter_mut() {
            *m = k.of(m, Domain::Kind)?.to_string();
        }
    }
    let ops = [names["+"], names["-"], names["*"]];
    Ok(Lowered { program: out, domains, ops, dialect })
}

#[cfg(test)]
mod tests;
