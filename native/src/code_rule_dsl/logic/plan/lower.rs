//! A program lowered to the integers of the base relations (decision 0009; O13).
//!
//! Every column gets a domain. Declared types give `int`, `node` and `sym`; the base relations' symbol columns are
//! names, except `node.kind` (kind atoms) and `field.f` (field atoms); a derived `sym` column takes the domain of
//! the variables written into it, to a fixpoint over the rules, and is a name if nothing says otherwise. String
//! constants then become integers by the domain they meet: name atoms (interned), or kind and field atoms in the
//! program's dialect (`-1`, which matches nothing, for a name the grammar lacks). Planning and SQL see only integers.

use super::super::front::{Branded, Class, Value};
use crate::code_rule_dsl::logic::program_image::Version;
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
/// `strlen` an integer, `rx_group` a regex's named group, `pkg_up` a package (`.rr`'s `+ - * /`, `cat`, `substr`, `to_string`, `strlen`).
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

/// Every column's domain (see the module comment), read from the program's version: a fixpoint over the rules, its
/// relations numbered by their declarations in name order and its rules in the program's; a rule's variables are its
/// slots.
///
/// On numbers: each declared relation by its place among the declarations, its columns a vector by number, each
/// rule's atoms resolved to numbers once, and the readers by number. Looked up by name in a sorted map at every atom
/// of every step, the columns were a string comparison per level (`memcmp`, the largest share of a cold batch's main
/// thread under `domains`). The worklist's order is the same, so the domains are.
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

/// The domains of rule `r`'s variables, by slot: `ds` gives each atom's column domains,
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

/// Each relation's columns as `rule_var_domains` reads them, from the domains: by name, one lookup per atom.
pub(in crate::code_rule_dsl) fn columns_of(domains: &Domains) -> HashMap<&str, Vec<Option<Domain>>> {
domains.iter().map(|(r, ds)| (r.as_str(), ds.iter().copied().map(Some).collect())).collect()
}

/// A program lowered for one dialect, as the run reads it (decision 0281's path, step 4): its version, with a
/// maintained program's parts (decision 0215), and what the run reads beside it.
#[derive(Clone)]
pub struct Lowered {
    pub version: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>,
    /// Shared by every part and edit of the program (`with_version`), never copied per part (0276 §2).
    pub domains: std::sync::Arc<Domains>,
    /// The name atoms of `flat_bin`'s operators `+`, `-` and `*`.
    pub(in crate::code_rule_dsl) ops: [i64; 3],
    pub dialect: GrammarDialect,
}

impl Lowered {
    /// The program `p` places, read beside `domains`, `ops` and `dialect`.
    pub fn of_placed(p: super::body::Placed, domains: Domains, ops: [i64; 3], dialect: GrammarDialect) -> Lowered {
        Lowered { version: p.v, domains: std::sync::Arc::new(domains), ops, dialect }
    }
    /// A program of version `v` read beside this one's domains, operators and dialect (a part of it, or an edit of
    /// it): the domains shared, not copied.
    pub fn with_version(&self, v: std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>) -> Lowered {
        Lowered { version: v, domains: std::sync::Arc::clone(&self.domains), ops: self.ops, dialect: self.dialect }
    }
    /// What the program's run reads beside its version (`plan::run`).
    pub fn lowering(&self) -> Lowering<'_> {
        Lowering { domains: &self.domains, dialect: self.dialect }
    }
    /// The version `plan::run` reads.
    pub fn placed(&self) -> super::Result<super::body::Placed> {
        Ok(super::body::Placed::whole(self.version.clone()))
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
}

/// Version `v`, lowered for another store, with its name atoms moved to this store's (decision 0270): `to` gives each
/// name atom's atom here. Only a branded name moves; a kind or a field is the grammar's, and an integer is an integer.
/// `None` when a name has none in `to` (the caller prepares again).
pub fn relocate(v: &Version, to: &HashMap<u32, u32>) -> Result<Option<std::sync::Arc<Version>>> {
    use crate::code_rule_dsl::logic::program_image::fields::{FieldsMut as _, ProgramFields as _};
    use crate::code_rule_dsl::logic::program_image::{Builder, Rewrite, Site, T, TermRef};
    /// Each branded name moved; `missing` once one has no atom in `to`.
    struct Moved<'t> {
        to: &'t HashMap<u32, u32>,
        missing: bool,
    }
    impl Rewrite for Moved<'_> {
        fn term(&mut self, _site: Site<'_>, t: TermRef<'_>) -> Option<T<'static>> {
            match t {
                TermRef::Branded(b) if b.class == Class::Name => match self.to.get(&b.atom) {
                    Some(&atom) => Some(T::Branded(Branded { atom, ..b })),
                    None => {
                        self.missing = true;
                        None
                    }
                },
                _ => None,
            }
        }
    }
    let mut b = Builder::derive(v);
    let mut moved = Moved { to, missing: false };
    b.rewrite_all(v, &mut moved).map_err(Error::Plan)?;
    for rel in v.fact_names() {
        let mut rows = v.fact_rows(&rel);
        for row in &mut rows {
            let mut cells: Vec<Value> = row.to_vec();
            for c in &mut cells {
                if let Value::Branded(x) = c
                    && x.class == Class::Name
                {
                    match to.get(&x.atom) {
                        Some(&atom) => x.atom = atom,
                        None => return Ok(None),
                    }
                }
            }
            *row = cells.into();
        }
        b.set_facts(&rel, &rows);
    }
    if moved.missing {
        return Ok(None);
    }
    Ok(Some(b.finish().map_err(Error::Plan)?.0))
}

mod on_version;
pub use on_version::{lower_version, lowered};

#[cfg(test)]
pub mod tests;
