//! A program lowered to the integers of the base relations (decision 0009; O13).
//!
//! Every column gets a domain. Declared types give `int`, `node` and `sym`; the base relations' symbol columns are
//! names, except `node.kind` (kind atoms) and `field.f` (field atoms); a derived `sym` column takes the domain of
//! the variables written into it, to a fixpoint over the rules, and is a name if nothing says otherwise. String
//! constants then become integers by the domain they meet: name atoms (interned), or kind and field atoms in the
//! program's dialect (`-1`, which matches nothing, for a name the grammar lacks). Planning and SQL see only integers.

use super::super::front::{AggOp, Atom, Literal, Program, Term, Value};
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

/// The domains of a functor's arguments and of its value, where it fixes them: `@cat`'s parts and whole are names,
/// `flat_bin`'s operator is one; arithmetic is over integers; `concat`, `substr` and `to_string` make names,
/// `strlen` an integer, `rx_group` a regex's named group, `pkg_up` a package (`.rr`'s `+ - * /`, `cat`, `substr`, `to_string`, `strlen`).
/// A text test's string is a name, but for `equals`, a membership (`(k = "a" ; k = "b")`), whose string may be a kind
/// or a field (`expand_memberships`).
pub(in crate::code_rule_dsl) fn functor_domains(f: &str, arity: usize) -> (Vec<Option<Domain>>, Option<Domain>) {
    use Domain::{Int, Name};
    match f {
        "cat" | "concat" => (vec![Some(Name); arity], Some(Name)),
        "flat_bin" => (vec![Some(Name), None, None], None),
        "add" | "sub" | "mul" | "div" => (vec![Some(Int); arity], Some(Int)),
        "strlen" => (vec![Some(Name)], Some(Int)),
        "to_string" => (vec![Some(Int)], Some(Name)),
        "substr" => (vec![Some(Name), Some(Int), Some(Int)], Some(Name)),
        // W's semantics, ahead of `strings::functions`' (`plan::sql_term`'s arms for them).
        "replace_all" | "rx_replace" => (vec![Some(Name); 3], Some(Name)),
        "json_str" | "nfkc" => (vec![Some(Name)], Some(Name)),
        "ed25519_ok" => (vec![Some(Name); 3], Some(Int)),
        "rx_group" => (vec![Some(Name); 3], Some(Name)),
        // `@place(path, line)` (`rr::lower`'s `PLACE`): a place where a node goes, taken apart again by
        // `place_path` and `place_line`.
        "place" => (vec![Some(Name), Some(Int)], Some(Domain::Node)),
        "place_path" => (vec![Some(Domain::Node)], Some(Name)),
        "place_line" => (vec![Some(Domain::Node)], Some(Int)),
        f if let Some((_, args, ret)) = crate::code_rule_dsl::logic::strings::functions::signature(f) => {
            let domain = |k: &crate::code_rule_dsl::logic::strings::functions::Kind| {
                Some(if *k == crate::code_rule_dsl::logic::strings::functions::Kind::Text { Name } else { Int })
            };
            (args.iter().map(domain).collect(), domain(&ret))
        }
        // `equals` (membership) takes its subject's domain: kinds compare as kind atoms (`lower`, `@in`).
        "equals" | "in" => (vec![None; arity], None),
        f if crate::code_rule_dsl::logic::strings::is_test(f) => (vec![Some(Name); arity], None),
        _ => (vec![None; arity], None),
    }
}

/// `prog` with each membership of a kind or field variable (`equals($k, {"a", "b"})`, from `(k = "a" ; k = "b")`)
/// as comparisons, which take the variable's domain: one rule per member, or (negated) a comparison with each.
/// `None` where `prog` has no membership.
fn expand_memberships(prog: &Program) -> Result<Option<Program>> {
    let membership = |l: &Literal| match l {
        Literal::Cmp { l: Term::Call(f, args), op: "=", r: Term::Int(b @ (0 | 1)) } if f == "equals" => match args.as_slice() {
            [Term::Var(x), Term::Call(set, items)] if set == crate::code_rule_dsl::logic::strings::SET => Some((x.clone(), items.clone(), *b == 1)),
            _ => None,
        },
        _ => None,
    };
    if !prog.rules.iter().flat_map(|r| &r.body).any(|l| membership(l).is_some()) {
        return Ok(None);
    }
    let domains = domains(prog)?;
    let cols = domains.iter().map(|(r, ds)| (r.clone(), ds.iter().copied().map(Some).collect())).collect();
    let mut out = prog.clone();
    out.rules.clear();
    for (i, rule) in prog.rules.iter().enumerate() {
        if !rule.body.iter().any(|l| membership(l).is_some()) {
            out.rules.push(rule.clone());
            continue;
        }
        let vars = rule_vars(prog, &cols, i)?;
        let mut bodies = vec![Vec::new()];
        for l in &rule.body {
            match membership(l) {
                Some((x, items, holds)) if matches!(vars.get(&x), Some(Domain::Kind | Domain::Field)) => {
                    let cmp = |item: &Term, op| Literal::Cmp { l: Term::Var(x.clone()), op, r: item.clone() };
                    if holds {
                        bodies = bodies.iter().flat_map(|b| items.iter().map(move |item| b.iter().cloned().chain([cmp(item, "=")]).collect())).collect();
                    } else {
                        bodies.iter_mut().for_each(|b| b.extend(items.iter().map(|item| cmp(item, "!="))));
                    }
                }
                _ => bodies.iter_mut().for_each(|b| b.push(l.clone())),
            }
        }
        out.rules.extend(bodies.into_iter().map(|body| super::super::front::Rule { head: rule.head.clone(), body }));
    }
    Ok(Some(out))
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
            let (domains, _) = functor_domains(f, args.len());
            for (a, d) in args.iter().zip(domains) {
                if let (Term::Var(v), Some(d)) = (a, d) {
                    changed |= self.set(v, d)?;
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
                    // `$x = @cat(...)`: the whole is a name; `$x = @add(...)` an integer.
                    if let (Term::Var(x), Term::Call(f, args)) = (l, r)
                        && let (_, Some(d)) = functor_domains(f, args.len())
                    {
                        changed |= vars.set(x, d)?;
                    }
                }
                Literal::Nearest { n, a, .. } => {
                    changed |= vars.set(n, Domain::Node)?;
                    changed |= vars.set(a, Domain::Node)?;
                }
                // `join` makes a name; `count` and `sum` integers.
                Literal::Agg { var, op: AggOp::Join { .. }, .. } => changed |= vars.set(var, Domain::Name)?,
                Literal::Agg { var, op: AggOp::Count | AggOp::Sum, .. } => changed |= vars.set(var, Domain::Int)?,
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
                let (domains, _) = functor_domains(f, args.len());
                for (a, d) in args.iter_mut().zip(domains) {
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
    let expanded = expand_memberships(prog)?;
    let prog = expanded.as_ref().unwrap_or(prog);
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
                Literal::Cmp { l, op, r } => {
                    let of = |t: &Term| if let Term::Var(v) = t { vars.get(v).copied() } else { None };
                    // Symbols are ordered by their text, not their atoms: `$t > $s` is `@str_cmp($t, $s) > 0`.
                    if matches!(*op, "<" | "<=" | ">" | ">=") && (of(l) == Some(Domain::Name) || of(r) == Some(Domain::Name)) {
                        let mut pair = [std::mem::replace(l, Term::Int(0)), std::mem::replace(r, Term::Int(0))];
                        pair.iter_mut().try_for_each(|t| k.term(t, Some(Domain::Name)))?;
                        *l = Term::Call("str_cmp".to_owned(), pair.into());
                        continue;
                    }
                    // Membership of a kind or field, `@equals($k, {"a", "b"})`: `@in` of their atoms.
                    if let Term::Call(f, args) = l
                        && f == "equals"
                        && let [x, set] = args.as_mut_slice()
                        && let Some(d @ (Domain::Kind | Domain::Field)) = of(x)
                    {
                        match set {
                            Term::Call(g, items) if g == crate::code_rule_dsl::logic::strings::SET => items.iter_mut().try_for_each(|i| k.term(i, Some(d)))?,
                            one => k.term(one, Some(d))?,
                        }
                        *f = "in".to_owned();
                        continue;
                    }
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
        // `EVERY_KIND`: the set of every kind (core.rr's `within`).
        if members.iter().any(|m| m == super::super::front::EVERY_KIND) {
            *members = (1..k.kinds.kind_count() as u16).map(|code| i64::from(kind_atom(tag, code)).to_string()).collect();
            continue;
        }
        for m in members.iter_mut() {
            *m = k.of(m, Domain::Kind)?.to_string();
        }
    }
    let ops = [names["+"], names["-"], names["*"]];
    Ok(Lowered { program: out, domains, ops, dialect })
}

impl Lowered {
    /// `nearest($n, $a, set)` read from the transducer's file-local FIX (O12, option a): each literal becomes
    /// the input atom `nearest_<set>($n, $a)`, which the run binds beside the base relations
    /// (`facts::vtab::syntax::with_nearest`). The relations the run must make (`nearest_<set>`, and the tree
    /// axes' `inside_<set>` and `inside_rr_has_<set>`), each with its set's kind atoms.
    pub(in crate::code_rule_dsl) fn transduce_nearest(&mut self) -> Vec<(String, Vec<i64>)> {
        let prog = &mut self.program;
        let mut used = std::collections::BTreeSet::new();
        for rule in &mut prog.rules {
            for literal in &mut rule.body {
                if let Literal::Nearest { n, a, kindset } = literal {
                    used.insert(kindset.clone());
                    *literal = Literal::Atom(Atom {
                        rel: format!("nearest_{kindset}"),
                        args: vec![Term::Var(n.clone()), Term::Var(a.clone())],
                        neg: false,
                        gated: false,
                    });
                }
            }
        }
        let mut sets = Vec::new();
        // The tree axes (`inside_<set>`, `inside_rr_has_<set>`, decision 0036), made from the same sets.
        for rel in prog.inputs.iter().filter(|r| r.starts_with(crate::code_rule_dsl::logic::front::AXIS)) {
            let Some((set, _)) = crate::code_rule_dsl::logic::front::axis(rel) else { continue };
            self.domains.insert(rel.clone(), vec![Domain::Node, Domain::Node]);
            let kinds = prog.kindsets.get(set).into_iter().flatten().filter_map(|k| k.parse::<i64>().ok()).collect();
            sets.push((rel.clone(), kinds));
        }
        for set in used {
            let rel = format!("nearest_{set}");
            prog.decls.insert(rel.clone(), vec![("n".to_owned(), "node".to_owned()), ("a".to_owned(), "node".to_owned())]);
            if !prog.inputs.contains(&rel) {
                prog.inputs.push(rel.clone());
            }
            self.domains.insert(rel.clone(), vec![Domain::Node, Domain::Node]);
            let kinds = prog.kindsets.get(&set).into_iter().flatten().filter_map(|k| k.parse::<i64>().ok()).collect();
            sets.push((rel, kinds));
        }
        sets
    }
}

#[cfg(test)]
mod tests;
