//! A program lowered to the integers of the base relations (decision 0009; O13).
//!
//! Every column gets a domain. Declared types give `int`, `node` and `sym`; the base relations' symbol columns are
//! names, except `node.kind` (kind atoms) and `field.f` (field atoms); a derived `sym` column takes the domain of
//! the variables written into it, to a fixpoint over the rules, and is a name if nothing says otherwise. String
//! constants then become integers by the domain they meet: name atoms (interned), or kind and field atoms in the
//! program's dialect (`-1`, which matches nothing, for a name the grammar lacks). Planning and SQL see only integers.

use super::super::front::{AggOp, Atom, Branded, Class, Literal, Program, Term, Value};
use super::{Error, Result};
use crate::code_rule_dsl::facts::core::kind_atom;
use crate::code_rule_dsl::facts::vtab::syntax::field_atom;
use crate::code_rule_dsl::language::{GrammarDialect, KindCatalog, tape_tag};
use std::collections::BTreeMap;
// FxHash: lowering's variable maps are hot (std's SipHash showed in a cold batch's samples).
use rustc_hash::FxHashMap as HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[derive(Hash)]
#[derive(Ord)]
#[derive(PartialOrd)]
pub enum Domain {
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
/// `strlen` an integer, `rx_group` a regex's named group, `pkg_up` a package (`.rerail`'s `+ - * /`, `cat`, `substr`, `to_string`, `strlen`).
/// A text test's string is a name, but for `equals`, a membership (`(k = "a" ; k = "b")`), whose string may be a kind
/// or a field (`expand_memberships`).
pub(in crate::code_rule_dsl) fn functor_domains(f: &str, arity: usize) -> (Vec<Option<Domain>>, Option<Domain>) {
    use Domain::{Int, Name};
    match f {
        "cat" | "concat" => (vec![Some(Name); arity], Some(Name)),
        "flat_bin" => (vec![Some(Name), None, None], None),
        "add" | "sub" | "mul" | "div" | "max" | "min" => (vec![Some(Int); arity], Some(Int)),
        "strlen" => (vec![Some(Name)], Some(Int)),
        "doc" => (vec![None], Some(Int)),
        "to_string" => (vec![Some(Int)], Some(Name)),
        "substr" => (vec![Some(Name), Some(Int), Some(Int)], Some(Name)),
        // W's semantics, ahead of `strings::functions`' (`plan::sql_term`'s arms for them).
        "replace_all" | "rx_replace" => (vec![Some(Name); 3], Some(Name)),
        "json_str" | "nfkc" | "py_repr" | "shell_unquote" => (vec![Some(Name)], Some(Name)),
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
    let cols = columns_of(&domains);
    let mut out = prog.clone();
    // Each rule with its source, so the request record (decision 0331) follows a split rule: each part serves its
    // source's readers.
    let mut rules = Vec::with_capacity(prog.rules.len());
    for (k, rule) in prog.rules.iter().enumerate() {
        if !rule.body.iter().any(|l| membership(l).is_some()) {
            rules.push((rule.clone(), vec![k]));
            continue;
        }
        let vars = rule_vars(rule, &mut |a: &Atom| cols.get(a.rel.as_str()).map(Vec::as_slice))?;
        let mut bodies = vec![Vec::new()];
        for l in &rule.body {
            match membership(l) {
                Some((x, items, holds)) if matches!(vars.get(x.as_str()), Some(Domain::Kind | Domain::Field)) => {
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
        rules.extend(bodies.into_iter().map(|body| (super::super::front::Rule { head: rule.head.clone(), body: body.into() }, vec![k])));
    }
    out.set_rules_from(rules);
    Ok(Some(out))
}

/// Each relation's column domains.
pub(in crate::code_rule_dsl) type Domains = BTreeMap<String, Vec<Domain>>;

/// Which of relation `rel`'s columns hold atoms (decision 0170: what a forked process maps, what an entry may name):
/// its `sym` columns whose values are names, not a grammar's kind or field ids (`Domain::Kind`, `Domain::Field`:
/// `field_atom`'s `tag << 16 | code`, which no store numbers). A relation `domains` does not cover keeps every `sym`
/// column.
/// `types`: the relation's declared column types, in order (none where it is not declared).
pub(in crate::code_rule_dsl) fn atom_columns(types: &[&str], domains: &Domains, rel: &str) -> Vec<bool> {
    let ds = domains.get(rel);
    types.iter().enumerate().map(|(c, t)| *t == "sym" && ds.and_then(|d| d.get(c)).is_none_or(|d| *d == Domain::Name)).collect()
}

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
    map: HashMap<&'a str, Domain>,
}

impl<'a> Vars<'a> {
    /// Records `$v` in `d`; whether that is new. Two symbol domains for one variable is an error.
    fn set(&mut self, v: &'a str, d: Domain) -> Result<bool> {
        match self.map.get(v) {
            Some(&e) if e.symbolic() && d.symbolic() && e != d => {
                Err(Error::Plan(format!("rule for {}: ${v} is a {e:?} and a {d:?}", self.head)))
            }
            Some(_) => Ok(false),
            None => {
                self.map.insert(v, d);
                Ok(true)
            }
        }
    }

    /// The domains a builtin call gives its variables: `@cat`'s parts are names, `flat_bin`'s operator too.
    fn call(&mut self, t: &'a Term) -> Result<bool> {
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

/// The domains of a rule's variables, given each of its atoms' columns (`cols`, in the order of `atoms`: the head,
/// then the body's).
fn rule_vars<'r, 'c>(r: &'r super::super::front::Rule, cols: &mut dyn FnMut(&Atom) -> Option<&'c [Option<Domain>]>) -> Result<HashMap<&'r str, Domain>> {
    let mut vars = Vars { head: &r.head, map: HashMap::default() };
    // Each atom's columns, looked up once, not once per argument per pass.
    let atoms: Vec<(&'r Atom, Option<&'c [Option<Domain>]>)> = std::iter::once(&r.head).chain(r.body.iter().flat_map(atoms)).map(|a| (a, cols(a))).collect();
    let mut changed = true;
    while changed {
        changed = false;
        for &(a, ds) in &atoms {
            for (c, t) in a.args.iter().enumerate() {
                if let (Term::Var(v), Some(Some(d))) = (t, ds.and_then(|ds| ds.get(c))) {
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
                        if let Some(&d) = vars.map.get(x.as_str()) {
                            changed |= vars.set(y.as_str(), d)?;
                        }
                        if let Some(&d) = vars.map.get(y.as_str()) {
                            changed |= vars.set(x.as_str(), d)?;
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
///
/// On numbers: each declared relation by its place among the declarations, its columns a vector by number, each
/// rule's atoms resolved to numbers once, and the readers by number. Looked up by name in a sorted map at every atom
/// of every step, the columns were a string comparison per level (`memcmp`, the largest share of a cold batch's main
/// thread under `domains`). The worklist's order is the same, so the domains are.
pub fn domains(prog: &Program) -> Result<Domains> {
    let names: Vec<&str> = prog.decls.keys().map(|r| r.as_str()).collect();
    let id: HashMap<&str, u32> = names.iter().enumerate().map(|(i, r)| (*r, i as u32)).collect();
    let inputs: std::collections::HashSet<&str> = prog.inputs.iter().map(String::as_str).collect();
    let mut cols: Vec<Vec<Option<Domain>>> = Vec::with_capacity(names.len());
    for (rel, decl) in &prog.decls {
        let input = inputs.contains(rel.as_str());
        let ds = decl
            .iter()
            .enumerate()
            .map(|(c, (_, t))| match t.as_str() {
                "int" | "flat" | "min" | "max" => Ok(Some(Domain::Int)),
                "node" => Ok(Some(Domain::Node)),
                "sym" if input => Ok(Some(base_domain(rel, c).unwrap_or(Domain::Name))),
                "sym" => Ok(None),
                _ => Err(Error::Plan(format!("{rel}: no domain for {t}"))),
            })
            .collect::<Result<_>>()?;
        cols.push(ds);
    }
    // Each rule's atoms (the head, then the body's) by number: `None` for a relation not declared.
    let rels: Vec<Vec<Option<u32>>> = prog
        .rules
        .iter()
        .map(|r| std::iter::once(&r.head).chain(r.body.iter().flat_map(atoms)).map(|a| id.get(a.rel.as_str()).copied()).collect())
        .collect();
    // To a fixpoint, by a worklist: a rule is read again only when a relation it names gained a column's domain. Every
    // rule in every round was quadratic in a combined pack (O111: 539 rules, a round per step of a chain).
    let mut readers: Vec<Vec<usize>> = vec![Vec::new(); names.len()];
    for (i, atoms) in rels.iter().enumerate() {
        for &d in atoms.iter().flatten() {
            let list = &mut readers[d as usize];
            if list.last() != Some(&i) {
                list.push(i);
            }
        }
    }
    let mut queued = vec![true; prog.rules.len()];
    let mut work: std::collections::VecDeque<usize> = (0..prog.rules.len()).collect();
    while let Some(i) = work.pop_front() {
        queued[i] = false;
        let r = &prog.rules[i];
        let vars = {
            let mut at = rels[i].iter();
            let cols = &cols;
            rule_vars(r, &mut |_| at.next().copied().flatten().map(|d| cols[d as usize].as_slice()))?
        };
        let mut gained: Vec<u32> = Vec::new();
        for (a, d) in std::iter::once(&r.head).chain(r.body.iter().flat_map(atoms)).zip(&rels[i]) {
            let Some(d) = *d else { continue };
            let ds = &mut cols[d as usize];
            for (c, t) in a.args.iter().enumerate() {
                if let (Term::Var(v), Some(slot @ None)) = (t, ds.get_mut(c))
                    && let Some(&dom) = vars.get(v.as_str())
                {
                    *slot = Some(dom);
                    gained.push(d);
                }
            }
        }
        for d in gained {
            for &j in &readers[d as usize] {
                if !queued[j] {
                    queued[j] = true;
                    work.push_back(j);
                }
            }
        }
    }
    Ok(names.into_iter().zip(cols).map(|(rel, ds)| (rel.to_owned(), ds.into_iter().map(|d| d.unwrap_or(Domain::Name)).collect())).collect())
}

/// `domains`, read from the program's version (decision 0281's path, step 5): the same fixpoint, its relations
/// numbered by their declarations in name order and its rules in the program's, as the tree's, so it finds the same
/// domains; a rule's variables are its slots.
pub fn domains_of(v: &crate::code_rule_dsl::logic::program_image::Version) -> Result<Domains> {
    use crate::code_rule_dsl::logic::program_image::{AggRef, Lit as L, RuleRef, TermRef};
    let mut decls: Vec<(&str, _)> = v.decls().into_iter().map(|d| (v.text(d.name()), d)).collect();
    decls.sort_unstable_by(|a, b| a.0.cmp(b.0));
    let inputs: std::collections::HashSet<&str> = v.inputs().map(|a| v.text(a)).collect();
    // Each relation's declaration, by `RelId`: `None` for a relation not declared.
    let mut decl_of: Vec<Option<u32>> = vec![None; v.rel_end() as usize];
    let mut cols: Vec<Vec<Option<Domain>>> = Vec::with_capacity(decls.len());
    for (i, (rel, d)) in decls.iter().enumerate() {
        if let Some(r) = v.rel(rel) {
            decl_of[r.0 as usize] = Some(i as u32);
        }
        let input = inputs.contains(rel);
        let ds = d
            .cols()
            .enumerate()
            .map(|(c, (_, t))| match v.text(t) {
                "int" | "flat" | "min" | "max" => Ok(Some(Domain::Int)),
                "node" => Ok(Some(Domain::Node)),
                "sym" if input => Ok(Some(base_domain(rel, c).unwrap_or(Domain::Name))),
                "sym" => Ok(None),
                t => Err(Error::Plan(format!("{rel}: no domain for {t}"))),
            })
            .collect::<Result<_>>()?;
        cols.push(ds);
    }
    let rules: Vec<RuleRef<'_>> = v.rules().collect();
    fn atoms(r: crate::code_rule_dsl::logic::program_image::RuleRef<'_>) -> Vec<crate::code_rule_dsl::logic::program_image::AtomRef<'_>> {
        std::iter::once(r.head()).chain(r.body().filter_map(|l| l.atom())).collect()
    }
    let rels: Vec<Vec<Option<u32>>> = rules.iter().map(|&r| atoms(r).iter().map(|a| decl_of[a.rel().0 as usize]).collect()).collect();
    let mut readers: Vec<Vec<usize>> = vec![Vec::new(); decls.len()];
    for (i, atoms) in rels.iter().enumerate() {
        for &d in atoms.iter().flatten() {
            let list = &mut readers[d as usize];
            if list.last() != Some(&i) {
                list.push(i);
            }
        }
    }
    let rule_vars = |r: RuleRef<'_>, cols: &[Vec<Option<Domain>>], rels: &[Option<u32>]| -> Result<Vec<Option<Domain>>> {
        rule_var_domains(v, r, &rels.iter().map(|d| d.map(|d| cols[d as usize].as_slice())).collect::<Vec<_>>())
    };
    let mut queued = vec![true; rules.len()];
    let mut work: std::collections::VecDeque<usize> = (0..rules.len()).collect();
    while let Some(i) = work.pop_front() {
        queued[i] = false;
        let r = rules[i];
        let vars = rule_vars(r, &cols, &rels[i])?;
        let mut gained: Vec<u32> = Vec::new();
        for (a, d) in atoms(r).iter().zip(&rels[i]) {
            let Some(d) = *d else { continue };
            let ds = &mut cols[d as usize];
            for (c, t) in a.args().enumerate() {
                if let (TermRef::Var(s), Some(slot @ None)) = (t, ds.get_mut(c))
                    && let Some(dom) = vars[s.0 as usize]
                {
                    *slot = Some(dom);
                    gained.push(d);
                }
            }
        }
        for d in gained {
            for &j in &readers[d as usize] {
                if !queued[j] {
                    queued[j] = true;
                    work.push_back(j);
                }
            }
        }
    }
    Ok(decls.into_iter().zip(cols).map(|((rel, _), ds)| (rel.to_owned(), ds.into_iter().map(|d| d.unwrap_or(Domain::Name)).collect())).collect())
}

/// The domains of rule `r`'s variables, by slot (`rule_vars` on the tree): `ds` gives each atom's column domains,
/// the head's then each body atom's (an aggregate's included), in order, `None` for a relation with none.
pub(in crate::code_rule_dsl) fn rule_var_domains(
    v: &crate::code_rule_dsl::logic::program_image::Version,
    r: crate::code_rule_dsl::logic::program_image::RuleRef<'_>,
    ds: &[Option<&[Option<Domain>]>],
) -> Result<Vec<Option<Domain>>> {
    use crate::code_rule_dsl::logic::program_image::{AggRef, Lit as L, TermRef};
    let rels = ds;
    let mut vars: Vec<Option<Domain>> = vec![None; r.var_count()];
    let head = v.show_atom(r, r.head());
    let set = |vars: &mut Vec<Option<Domain>>, s: crate::code_rule_dsl::logic::program_image::Slot, d: Domain| -> Result<bool> {
        match vars[s.0 as usize] {
            Some(e) if e.symbolic() && d.symbolic() && e != d => Err(Error::Plan(format!("rule for {head}: ${} is a {e:?} and a {d:?}", v.text(r.var(s))))),
            Some(_) => Ok(false),
            None => {
                vars[s.0 as usize] = Some(d);
                Ok(true)
            }
        }
    };
    fn call(v: &crate::code_rule_dsl::logic::program_image::Version, t: TermRef<'_>, vars: &mut Vec<Option<Domain>>, set: &dyn Fn(&mut Vec<Option<Domain>>, crate::code_rule_dsl::logic::program_image::Slot, Domain) -> Result<bool>) -> Result<bool> {
        let mut changed = false;
        if let TermRef::Call(f, args) = t {
            let args: Vec<TermRef<'_>> = args.collect();
            let (domains, _) = functor_domains(v.text(f), args.len());
            for (&a, d) in args.iter().zip(domains) {
                if let (TermRef::Var(s), Some(d)) = (a, d) {
                    changed |= set(vars, s, d)?;
                }
                changed |= call(v, a, vars, set)?;
            }
        }
        Ok(changed)
    }
    let heads_and_atoms: Vec<crate::code_rule_dsl::logic::program_image::AtomRef<'_>> = std::iter::once(r.head()).chain(r.body().filter_map(|l| l.atom())).collect();
    let atoms: Vec<(Vec<TermRef<'_>>, Option<&[Option<Domain>]>)> = heads_and_atoms.iter().zip(rels).map(|(a, d)| (a.args().collect(), *d)).collect();
    let body: Vec<L<'_>> = r.body().map(|l| l.view()).collect();
    let mut changed = true;
    while changed {
        changed = false;
        for (args, ds) in &atoms {
            for (c, &t) in args.iter().enumerate() {
                if let (TermRef::Var(s), Some(Some(d))) = (t, ds.and_then(|ds| ds.get(c))) {
                    changed |= set(&mut vars, s, *d)?;
                }
                changed |= call(v, t, &mut vars, &set)?;
            }
        }
        for l in &body {
            match *l {
                L::Cmp { l, r: rt, .. } => {
                    changed |= call(v, l, &mut vars, &set)?;
                    changed |= call(v, rt, &mut vars, &set)?;
                    if let (TermRef::Var(x), TermRef::Var(y)) = (l, rt) {
                        if let Some(d) = vars[x.0 as usize] {
                            changed |= set(&mut vars, y, d)?;
                        }
                        if let Some(d) = vars[y.0 as usize] {
                            changed |= set(&mut vars, x, d)?;
                        }
                    }
                    if let (TermRef::Var(x), TermRef::Call(f, args)) = (l, rt)
                        && let (_, Some(d)) = functor_domains(v.text(f), args.count())
                    {
                        changed |= set(&mut vars, x, d)?;
                    }
                }
                L::Nearest { n, a, .. } => {
                    changed |= set(&mut vars, n, Domain::Node)?;
                    changed |= set(&mut vars, a, Domain::Node)?;
                }
                L::Agg { var, op: AggRef::Join { .. }, .. } => changed |= set(&mut vars, var, Domain::Name)?,
                L::Agg { var, op: AggRef::Count | AggRef::Sum, .. } => changed |= set(&mut vars, var, Domain::Int)?,
                _ => {}
            }
        }
    }
    Ok(vars)
}

/// Each relation's columns as `rule_vars` reads them, from the domains: by name, one lookup per atom.
pub(in crate::code_rule_dsl) fn columns_of(domains: &Domains) -> HashMap<&str, Vec<Option<Domain>>> {
domains.iter().map(|(r, ds)| (r.as_str(), ds.iter().copied().map(Some).collect())).collect()
}

/// A program lowered for one dialect, before its image: the tree `lower_tree` gives, which the preparation passes edit
/// (`corpus`) and the SQL emitter reads (`emit::emit`). The run reads none (decision 0281's path, step 4): `image`
/// makes the run's `Lowered`, a version.
#[derive(Clone)]
pub struct Given {
    pub program: Program,
    pub domains: Domains,
    /// The name atoms of `flat_bin`'s operators `+`, `-` and `*`.
    pub(in crate::code_rule_dsl) ops: [i64; 3],
    pub dialect: GrammarDialect,
}

impl Given {
    /// The run's program: an image of the tree (`Placed::of_program`, with its maintained parts), and what it reads
    /// beside it. A program the image has no form for (a comparison it does not know) is refused: the run reads a
    /// whole version (its rules alone did not run either: `Placed::normalized`).
    pub fn image(&self) -> super::Result<Lowered> {
        let placed = super::body::Placed::of_program(&self.program)?;
        if !placed.whole {
            let why = crate::code_rule_dsl::logic::program_image::Builder::of_program(&self.program).and_then(|b| b.finish()).err().unwrap_or_default();
            return Err(Error::Plan(format!("the program has no image: {why}")));
        }
        Ok(Lowered::of_placed(placed, self.domains.clone(), self.ops, self.dialect))
    }

    /// `nearest($n, $a, set)` read from the transducer's file-local FIX (O12, option a): each literal becomes
    /// the input atom `nearest_<set>($n, $a)`, which the run binds beside the base relations
    /// (`facts::vtab::syntax::with_nearest`). The relations the run must make (`nearest_<set>`, and the tree
    /// axes' `rr_inside_<set>` and `rr_has_<set>`), each with its set's kind atoms.
    ///
    /// A pass of preparation, before the program's one image.
    pub fn transduce_nearest(&mut self) -> Vec<(String, Vec<i64>)> {
        transduce_nearest(&mut self.program, &mut self.domains)
    }
}

/// A program lowered for one dialect, as the run reads it (decision 0281's path, step 4): its version, with a
/// maintained program's parts (decision 0215), and what the run reads beside it. No tree: one is made only by a
/// caller that decodes the version (`Version::program`, counted: `program_image::trees_decoded`).
#[derive(Clone)]
pub struct Lowered {
    pub version: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>,
    pub domains: Domains,
    /// The name atoms of `flat_bin`'s operators `+`, `-` and `*`.
    pub(in crate::code_rule_dsl) ops: [i64; 3],
    pub dialect: GrammarDialect,
    /// The program normalized as a run reads it (`body::Placed::normalized`), by `apart`: made by its first run and
    /// read by every later one (Q203 item 3 (ii)). A corpus run's chunks each normalized it again, a version of
    /// their own, and so every analysis that version holds again (decision 0281 §2); now they share one.
    pub(in crate::code_rule_dsl) normal: std::sync::Arc<[std::sync::OnceLock<std::sync::Arc<super::body::Normalized>>; 2]>,
}

impl Lowered {
    /// The program `p` places, read beside `domains`, `ops` and `dialect`.
    pub fn of_placed(p: super::body::Placed, domains: Domains, ops: [i64; 3], dialect: GrammarDialect) -> Lowered {
        Lowered { version: p.v, domains, ops, dialect, normal: Default::default() }
    }
    /// A program of version `v` read beside this one's domains, operators and dialect (a part of it, or an edit of
    /// it).
    pub fn with_version(&self, v: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>) -> Lowered {
        Lowered { version: v, domains: self.domains.clone(), ops: self.ops, dialect: self.dialect, normal: Default::default() }
    }
    /// What the program's run reads beside its version (`plan::run`).
    pub fn lowering(&self) -> Lowering<'_> {
        Lowering { domains: &self.domains, dialect: self.dialect }
    }
    /// The version `plan::run` reads.
    pub fn placed(&self) -> super::Result<super::body::Placed> {
        Ok(super::body::Placed::whole(self.version.clone()))
    }
    /// The program normalized for a run (`Placed::normalized`, `apart` its stated facts apart): made once, by the
    /// first run that asks, with its fields read, and the same for every run after (`normal`).
    pub fn normalized(&self, apart: bool) -> super::Result<std::sync::Arc<super::body::Normalized>> {
        let slot = &self.normal[usize::from(apart)];
        if let Some(n) = slot.get() {
            return Ok(n.clone());
        }
        let n = self.placed()?.normalized(apart)?;
        // Its fields read now, on the one every run clones (`Placed::fields` is shared by the clones made after).
        let _ = n.placed.fields();
        Ok(slot.get_or_init(|| std::sync::Arc::new(n)).clone())
    }
    /// The program's digest (`Placed::digest`): its version's, kept as it was made, with its maintained parts'.
    pub fn digest(&self) -> super::Result<u64> {
        Ok(self.placed()?.digest())
    }
}

/// What a lowered program's run reads beside its version (`plan::run`, t36 item 7 step 8): each relation's column
/// domains, and the dialect its kinds are of.
#[derive(Clone, Copy)]
pub struct Lowering<'l> {
    pub(in crate::code_rule_dsl) domains: &'l Domains,
    pub dialect: GrammarDialect,
}

struct Constants<'a> {
    tag: u8,
    kinds: KindCatalog,
    names: &'a HashMap<String, i64>,
}

impl Constants<'_> {
    /// The atom `s` is in domain `d`, branded with what it names (decision 0270); `None` for a kind or field the
    /// grammar does not have, which matches nothing (the run's -1).
    fn sym(&self, s: &str, d: Domain) -> Result<Option<Branded>> {
        let kind = |code: u16, tag: u8| (code != 0).then(|| Branded { class: Class::Kind, atom: kind_atom(tag, code) });
        let field = |code: Option<u16>, tag: u8| code.map(|code| Branded { class: Class::Field, atom: field_atom(tag, code) });
        Ok(match d {
            Domain::Name => Some(Branded::name(u32::try_from(self.names[s]).expect("a name atom is a u32"))),
            // `"sql:select"`: a kind of another grammar, as a guest's are (decision 0058 §2).
            Domain::Kind | Domain::Field if let Some((label, name)) = super::corpus::qualified(s) => {
                let dialect = crate::code_rule_dsl::language::dialect_by_label(label).expect("a registry label");
                let (Some(tag), kinds) = (tape_tag(dialect), dialect.kinds()) else { return Ok(None) };
                match d {
                    Domain::Kind => kind(kinds.kind_id(name), tag),
                    _ => field(kinds.field_id(name), tag),
                }
            }
            Domain::Kind => kind(self.kinds.kind_id(s), self.tag),
            Domain::Field => field(self.kinds.field_id(s), self.tag),
            Domain::Int | Domain::Node => return Err(Error::Plan(format!("the string \"{s}\" stands where an integer goes"))),
        })
    }
    /// `sym`'s value as the run holds it.
    fn of(&self, s: &str, d: Domain) -> Result<i64> {
        Ok(self.sym(s, d)?.map_or(-1, Branded::value))
    }
    fn term(&self, t: &mut Term, d: Option<Domain>) -> Result<()> {
        match t {
            Term::Str(s) => *t = self.sym(s, d.unwrap_or(Domain::Name))?.map_or(Term::Int(-1), Term::Branded),
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
        Term::Str(s) => out.push(s.to_string()),
        Term::Call(_, args) => args.iter().for_each(|a| strings(a, out)),
        _ => {}
    }
}

/// `prog`, lowered for another store, with its name atoms moved to this store's (decision 0270): `to` gives each name
/// atom's atom here. Only a branded name moves; a kind or a field is the grammar's, and an integer is an integer.
/// `false` when a name has none in `to` (the caller prepares again), with `prog` part moved.
pub fn relocate(prog: &mut Program, to: &HashMap<u32, u32>) -> bool {
    fn term(t: &mut Term, to: &HashMap<u32, u32>) -> bool {
        match t {
            Term::Branded(b) if b.class == Class::Name => match to.get(&b.atom) {
                Some(&a) => {
                    b.atom = a;
                    true
                }
                None => false,
            },
            Term::Call(_, args) => args.iter_mut().all(|a| term(a, to)),
            _ => true,
        }
    }
    let atom = |a: &mut Atom| a.args.iter_mut().all(|t| term(t, to));
    let rule = |r: &mut super::super::front::Rule| {
        atom(&mut r.head)
            && r.body.iter_mut().all(|l| match l {
                Literal::Atom(a) | Literal::Agg { atom: a, .. } => atom(a),
                Literal::Cmp { l, r, .. } => term(l, to) && term(r, to),
                Literal::Nearest { .. } | Literal::Ast { .. } => true,
            })
    };
    if !prog.rules.iter_mut().all(rule) {
        return false;
    }
    prog.facts.values_mut().flatten().all(|row| {
        row.iter_mut().all(|v| match v {
            Value::Branded(b) if b.class == Class::Name => to.get(&b.atom).map(|&a| b.atom = a).is_some(),
            _ => true,
        })
    })
}

/// Lowers `prog` for `dialect`, as the run reads it (`lower_tree`, imaged); `intern` gives the name atoms of strings,
/// interning new ones.
pub fn lower(
    prog: &Program,
    dialect: GrammarDialect,
    intern: &mut dyn FnMut(&[&str]) -> Result<Vec<u32>>,
) -> Result<Lowered> {
    lower_tree(prog, dialect, intern)?.image()
}

/// Lowers `prog` for `dialect` to a tree, before its image: what the preparation passes edit (`Given`).
pub fn lower_tree(
    prog: &Program,
    dialect: GrammarDialect,
    intern: &mut dyn FnMut(&[&str]) -> Result<Vec<u32>>,
) -> Result<Given> {
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
                all.push(s.to_string());
            }
        }
    }
    all.sort();
    all.dedup();
    let atoms_of = intern(&all.iter().map(String::as_str).collect::<Vec<_>>())?;
    let names: HashMap<String, i64> = all.into_iter().zip(atoms_of.into_iter().map(i64::from)).collect();
    let k = Constants { tag, kinds: dialect.kinds(), names: &names };
    let mut out = prog.clone();
    // Every column's domain as `rule_vars` reads it, made once: made per rule, it was rules × relations (O111).
    let cols = columns_of(&domains);
    for i in 0..out.rules.len() {
        // A constant that does not fit its column names its rule (`the string "" stands where an integer goes`).
        let head = out.rules[i].head.to_string();
        (|| -> Result<()> {
            let vars = rule_vars(&prog.rules[i], &mut |a: &Atom| cols.get(a.rel.as_str()).map(Vec::as_slice))?;
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
                        let of = |t: &Term| if let Term::Var(v) = t { vars.get(v.as_str()).copied() } else { None };
                        // Symbols are ordered by their text, not their atoms: `$t > $s` is `@str_cmp($t, $s) > 0`.
                        // A call whose value is a symbol (`cat(a, "!")`, a text function's) orders by its text too.
                        let symbol = |t: &Term| {
                            of(t) == Some(Domain::Name)
                                || matches!(t, Term::Call(f, _) if f == "concat" || f == "cat"
                                    || crate::code_rule_dsl::logic::strings::functions::native_call(f).is_some_and(|(_, _, k)| k == crate::code_rule_dsl::logic::strings::functions::Kind::Text))
                        };
                        if matches!(*op, "<" | "<=" | ">" | ">=") && (symbol(l) || symbol(r)) {
                            let mut pair = [std::mem::replace(l, Term::Int(0)), std::mem::replace(r, Term::Int(0))];
                            pair.iter_mut().try_for_each(|t| k.term(t, Some(Domain::Name)))?;
                            *l = Term::Call("str_cmp".into(), pair.into());
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
                            *f = "in".into();
                            continue;
                        }
                        let (dl, dr) = (of(r), of(l));
                        k.term(l, dl)?;
                        k.term(r, dr)?;
                    }
                    _ => {}
                }
            }

            Ok(())
        })()
        .map_err(|e| match e {
            Error::Plan(m) => Error::Plan(format!("rule for {head}: {m}")),
            e => e,
        })?;
    }
    for (rel, rows) in &mut out.facts {
        for row in rows {
            for (c, v) in row.iter_mut().enumerate() {
                if let Value::Str(s) = v {
                    let d = domains.get(rel.as_str()).and_then(|ds| ds.get(c)).copied().unwrap_or(Domain::Name);
                    *v = k.sym(s, d).map_err(|e| match e {
                        Error::Plan(m) => Error::Plan(format!("fact of {rel}, column {c}: {m}")),
                        e => e,
                    })?.map_or(Value::Int(-1), Value::Branded);
                }
            }
        }
    }
    for members in out.kindsets.values_mut() {
        // `EVERY_KIND`: the set of every kind (core.rerail's `within`).
        if members.iter().any(|m| m == super::super::front::EVERY_KIND) {
            *members = (1..k.kinds.kind_count() as u16).map(|code| i64::from(kind_atom(tag, code)).to_string()).collect();
            continue;
        }
        for m in members.iter_mut() {
            *m = k.of(m, Domain::Kind)?.to_string();
        }
    }
    let ops = [names["+"], names["-"], names["*"]];
    Ok(Given { program: out, domains, ops, dialect })
}

/// `Given::transduce_nearest` over the tree `prog` and its domains.
fn transduce_nearest(prog: &mut Program, domains: &mut Domains) -> Vec<(String, Vec<i64>)> {
    {
        let mut used = std::collections::BTreeSet::new();
        for rule in &mut prog.rules {
            for literal in &mut rule.body {
                if let Literal::Nearest { n, a, kindset } = literal {
                    used.insert(kindset.clone());
                    *literal = Literal::Atom(Atom {
                        rel: format!("{}{kindset}", crate::code_rule_dsl::logic::front::NEAREST).into(),
                        args: [Term::Var(n.clone().into()), Term::Var(a.clone().into())].into(),
                        neg: false,
                        gated: false,
                    });
                }
            }
        }
        let mut sets = Vec::new();
        // The tree axes (`rr_inside_<set>` and `rr_has_<set>`, decision 0036), made from the same sets' nearest ancestors.
        use crate::code_rule_dsl::logic::front::{AXIS, DESCENDANTS};
        for rel in prog.inputs.iter().filter(|r| crate::code_rule_dsl::logic::front::is_axis(r)) {
            let set = rel.strip_prefix(AXIS).or_else(|| rel.strip_prefix(DESCENDANTS)).expect("an axis");
            domains.insert(rel.clone(), vec![Domain::Node, Domain::Node]);
            let kinds = prog.kindsets.get(set).into_iter().flatten().filter_map(|k| k.parse::<i64>().ok()).collect();
            sets.push((rel.clone(), kinds));
        }
        for set in used {
            let rel = format!("{}{set}", crate::code_rule_dsl::logic::front::NEAREST);
            prog.decls.insert(rel.clone().into(), [("n".into(), "node".into()), ("a".into(), "node".into())].into());
            if !prog.inputs.contains(&rel) {
                prog.inputs.push(rel.clone());
            }
            domains.insert(rel.clone(), vec![Domain::Node, Domain::Node]);
            let kinds = prog.kindsets.get(&set).into_iter().flatten().filter_map(|k| k.parse::<i64>().ok()).collect();
            sets.push((rel, kinds));
        }
        sets
    }
}

mod on_version;
pub use on_version::{lower_version, lowered};

#[cfg(test)]
pub mod tests;
