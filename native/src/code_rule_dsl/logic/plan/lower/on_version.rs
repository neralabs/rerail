//! The program lowered for one dialect (decision 0281's path, step 6b), by passes over its image. A membership of a kind or field in a set of them is a rule per member
//! (`Memberships`); then every string becomes its atom, by its column's domain (a name's atom, a kind's or a field's
//! id), in rules (`Constants`), facts and kind sets (`FactsAndSets`).

use super::{Constants, Domain, Domains, columns_of, domains_of, functor_domains, rule_var_domains};
use crate::code_rule_dsl::facts::core::kind_atom;
use crate::code_rule_dsl::language::{GrammarDialect, tape_tag};
use crate::code_rule_dsl::logic::front::EVERY_KIND;
use crate::code_rule_dsl::logic::program_image::manager::{Cx, Pipeline, ProgramPass, ProgramW, RulePass, RuleOut, RuleSet, Select, Step};
use crate::code_rule_dsl::logic::program_image::{AtomRef, Cell, CmpOp, Lit, LitRef, Rewrite, RuleRef, Site, Slot, T, TermRef, Version};
use crate::code_rule_dsl::logic::strings::SET;
use super::super::{Error, Result};
use rustc_hash::FxHashMap as HashMap;
use std::sync::Arc;

/// `v` lowered for `dialect` and read as the run reads it (`Lowered::of_placed`), with no preparation pass between:
/// what a test or a tool that runs a program as written lowers. `intern` gives the name atoms of strings.
pub fn lowered(v: Arc<Version>, dialect: GrammarDialect, intern: &mut dyn FnMut(&[&str]) -> Result<Vec<u32>>) -> Result<super::Lowered> {
    let (v, domains, ops) = lower_version(v, dialect, intern)?;
    Ok(super::Lowered::of_placed(super::super::body::Placed::whole(v), domains, ops, dialect))
}

/// `v` lowered for `dialect`, with the domains of its relations' columns (found before its strings became atoms) and
/// the name atoms of `+`, `-` and `*`; `intern` gives the name atoms of strings.
pub fn lower_version(v: Arc<Version>, dialect: GrammarDialect, intern: &mut dyn FnMut(&[&str]) -> Result<Vec<u32>>) -> Result<(Arc<Version>, Domains, [i64; 3])> {
    let tag = tape_tag(dialect).ok_or_else(|| Error::Plan("the dialect has no tape tag".to_owned()))?;
    let mut p = Pipeline::new("lower", v);
    p.rules(&mut Memberships::default());
    let v = p.current().map_err(Error::Plan)?.clone();
    let domains = domains_of(&v)?;
    // Every string a name could be, interned in one call: interning a kind's name too costs one atom.
    let mut all: Vec<String> = vec!["+".to_owned(), "-".to_owned(), "*".to_owned()];
    fn strings(v: &Version, t: TermRef<'_>, out: &mut Vec<String>) {
        match t {
            TermRef::Str(a) => out.push(v.text(a).to_owned()),
            TermRef::Call(_, args) => args.for_each(|a| strings(v, a, out)),
            _ => {}
        }
    }
    for r in v.rules() {
        for a in std::iter::once(r.head()).chain(r.body().filter_map(|l| l.atom())) {
            a.args().for_each(|t| strings(&v, t, &mut all));
        }
        for l in r.body() {
            if let Lit::Cmp { l, r: rt, .. } = l.view() {
                strings(&v, l, &mut all);
                strings(&v, rt, &mut all);
            }
        }
    }
    for f in v.all_facts().into_iter().filter(|f| !f.about()) {
        for row in f.rows() {
            for c in row {
                if let Cell::Str(a) = c {
                    all.push(v.text(a).to_owned());
                }
            }
        }
    }
    all.sort();
    all.dedup();
    let atoms_of = intern(&all.iter().map(String::as_str).collect::<Vec<_>>())?;
    let names: HashMap<String, i64> = all.into_iter().zip(atoms_of.into_iter().map(i64::from)).collect();
    let k = Constants { tag, kinds: dialect.kinds(), names: &names };
    let cols = columns_of(&domains);
    p.rules(&mut Lowering { k: &k, domains: &domains, cols: &cols });
    p.program(&mut FactsAndSets { k: &k, domains: &domains });
    let (out, _) = p.finish().map_err(Error::Plan)?;
    let ops = [names["+"], names["-"], names["*"]];
    Ok((out, domains, ops))
}

/// A term as it is, for a rule written from `r`.
fn as_is(r: RuleRef<'_>, t: TermRef<'_>) -> T<'static> {
    match t {
        TermRef::Var(s) => T::VarAtom(r.var(s)),
        TermRef::Wild => T::Wild,
        TermRef::Top => T::Top,
        TermRef::Int(i) => T::Int(i),
        TermRef::Str(a) => T::StrAtom(a),
        TermRef::Branded(b) => T::Branded(b),
        TermRef::Call(f, args) => T::CallAtom(f, args.map(|a| as_is(r, a)).collect()),
    }
}

/// `@equals($x, {…}) = 1` (or `= 0`): the variable, the set's items, and whether it holds.
fn membership<'v>(v: &Version, l: LitRef<'v>) -> Option<(Slot, Vec<TermRef<'v>>, bool)> {
    let Lit::Cmp { l: TermRef::Call(f, args), op: CmpOp::Eq, r: TermRef::Int(b @ (0 | 1)) } = l.view() else { return None };
    if v.text(f) != "equals" {
        return None;
    }
    let args: Vec<TermRef<'v>> = args.collect();
    match args.as_slice() {
        [TermRef::Var(x), TermRef::Call(set, items)] if v.text(*set) == SET => Some((*x, (*items).collect(), b == 1)),
        _ => None,
    }
}

/// Each atom's column domains in rule `r`, the head's then its body atoms', for `rule_var_domains`.
fn atom_domains<'d>(v: &Version, r: RuleRef<'_>, cols: &'d HashMap<&str, Vec<Option<Domain>>>) -> Vec<Option<&'d [Option<Domain>]>> {
    std::iter::once(r.head()).chain(r.body().filter_map(|l| l.atom())).map(|a| cols.get(v.rel_name(a.rel())).map(Vec::as_slice)).collect()
}

/// A membership of a kind or field in a set of them (`@equals($k, {"a", "b"}) = 1`) as a rule per member, each
/// comparing it to one (`$k = "a"`); a membership that must not hold as a comparison per member (`$k != "a"`).
#[derive(Default)]
struct Memberships {
    domains: Option<Domains>,
}

impl RulePass for Memberships {
    const NAME: &'static str = "kind and field memberships expanded";
    fn begin(&mut self, cx: &Cx<'_>) -> std::result::Result<Select, String> {
        let v = cx.v();
        if !v.rules().any(|r| r.body().any(|l| membership(v, l).is_some())) {
            return Ok(Select::Rules(RuleSet::new()));
        }
        self.domains = Some(domains_of(v).map_err(|e| e.to_string())?);
        // A derivation: each rule's expansion in its place, as `expand_memberships` keeps it on the tree (an edit would
        // put them after the rest, and the request record, by place, with them). Lowering derives the whole program
        // next anyway.
        Ok(Select::All)
    }
    fn rule(&mut self, cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> std::result::Result<Step, String> {
        let v = cx.v();
        if !r.body().any(|l| membership(v, l).is_some()) {
            return Ok(Step::Keep);
        }
        let cols = columns_of(self.domains.as_ref().expect("begun"));
        let vars = rule_var_domains(v, r, &atom_domains(v, r, &cols)).map_err(|e| e.to_string())?;
        enum Piece<'v> {
            Lit(LitRef<'v>),
            Cmp(Slot, CmpOp, TermRef<'v>),
        }
        let mut bodies: Vec<Vec<Piece<'_>>> = vec![Vec::new()];
        let mut expanded = false;
        for l in r.body() {
            match membership(v, l) {
                Some((x, items, holds)) if matches!(vars[x.0 as usize], Some(Domain::Kind | Domain::Field)) => {
                    expanded = true;
                    if holds {
                        bodies = bodies
                            .iter()
                            .flat_map(|b| {
                                items.iter().map(move |&item| {
                                    let mut nb: Vec<Piece<'_>> = b.iter().map(|p| match p {
                                        Piece::Lit(l) => Piece::Lit(*l),
                                        Piece::Cmp(x, op, t) => Piece::Cmp(*x, *op, *t),
                                    }).collect();
                                    nb.push(Piece::Cmp(x, CmpOp::Eq, item));
                                    nb
                                })
                            })
                            .collect();
                    } else {
                        bodies.iter_mut().for_each(|b| b.extend(items.iter().map(|&item| Piece::Cmp(x, CmpOp::Ne, item))));
                    }
                }
                _ => bodies.iter_mut().for_each(|b| b.push(Piece::Lit(l))),
            }
        }
        if !expanded {
            return Ok(Step::Keep);
        }
        if bodies.is_empty() {
            return Ok(Step::Drop);
        }
        for body in bodies {
            let mut w = out.rule().head_from(v, r, r.head());
            for p in body {
                w = match p {
                    Piece::Lit(l) => w.lit(v, r, l),
                    Piece::Cmp(x, op, item) => w.cmp(T::VarAtom(r.var(x)), op, as_is(r, item)),
                };
            }
            w.finish()?;
        }
        Ok(Step::Written)
    }
}

/// Every string of a rule as its atom, by its column's domain; a comparison of symbols ordered by their text
/// (`@str_cmp($t, $s) > 0`), and a kind's or field's membership tested on atoms (`@in`).
struct Lowering<'a> {
    k: &'a Constants<'a>,
    domains: &'a Domains,
    cols: &'a HashMap<&'a str, Vec<Option<Domain>>>,
}

impl RulePass for Lowering<'_> {
    const NAME: &'static str = "the program lowered";
    fn rule(&mut self, cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> std::result::Result<Step, String> {
        let v = cx.v();
        fn has_str(t: TermRef<'_>) -> bool {
            match t {
                TermRef::Str(_) => true,
                TermRef::Call(_, mut args) => args.any(has_str),
                _ => false,
            }
        }
        let strings = std::iter::once(r.head()).chain(r.body().filter_map(|l| l.atom())).any(|a| a.args().any(has_str));
        let compares = r.body().any(|l| matches!(l.view(), Lit::Cmp { .. }));
        if !strings && !compares {
            return Ok(Step::Keep);
        }
        let head = v.show_atom(r, r.head());
        let named = |m: String| format!("rule for {head}: {m}");
        let vars = rule_var_domains(v, r, &atom_domains(v, r, self.cols)).map_err(|e| match e {
            Error::Plan(m) => named(m),
            e => e.to_string(),
        })?;
        let mut rw = Rw { v, r, k: self.k, domains: self.domains, vars, rel: None, col: 0, right: None, err: None };
        out.rewrite(&mut rw)?;
        match rw.err {
            Some(Error::Plan(m)) => Err(named(m)),
            Some(e) => Err(e.to_string()),
            None => Ok(Step::Written),
        }
    }
}

struct Rw<'a, 'v> {
    v: &'v Version,
    r: RuleRef<'v>,
    k: &'a Constants<'a>,
    domains: &'a Domains,
    vars: Vec<Option<Domain>>,
    /// The atom whose arguments are being written, and the next argument's column.
    rel: Option<&'v str>,
    col: usize,
    /// The right side of the comparison whose left side was just written (by the literal's node).
    right: Option<(u32, T<'static>)>,
    err: Option<Error>,
}

impl Rw<'_, '_> {
    /// `t` with each string its atom in domain `d` (a name where none is known), into a call's arguments by the
    /// function's domains.
    fn lowered(&mut self, t: TermRef<'_>, d: Option<Domain>) -> T<'static> {
        match t {
            // Branded by its domain, as the tree's lowering brands it (decision 0270): `-1` for none.
            TermRef::Str(a) => match self.k.sym(self.v.text(a), d.unwrap_or(Domain::Name)) {
                Ok(b) => b.map_or(T::Int(-1), T::Branded),
                Err(e) => {
                    self.err.get_or_insert(e);
                    T::Int(0)
                }
            },
            TermRef::Call(f, args) => {
                let args: Vec<TermRef<'_>> = args.collect();
                let (ds, _) = functor_domains(self.v.text(f), args.len());
                T::CallAtom(f, args.iter().zip(ds.into_iter().chain(std::iter::repeat(None))).map(|(&a, d)| self.lowered(a, d)).collect())
            }
            t => as_is(self.r, t),
        }
    }
    fn of(&self, t: TermRef<'_>) -> Option<Domain> {
        if let TermRef::Var(s) = t { self.vars[s.0 as usize] } else { None }
    }
    /// The comparison `l op r` lowered: its left side and its right.
    fn cmp(&mut self, l: TermRef<'_>, op: CmpOp, r: TermRef<'_>) -> (T<'static>, T<'static>) {
        let v = self.v;
        // Symbols are ordered by their text, not their atoms: `$t > $s` is `@str_cmp($t, $s) > 0`. A call whose value
        // is a symbol (`cat(a, "!")`, a text function's) orders by its text too.
        let symbol = |t: TermRef<'_>| {
            self.of(t) == Some(Domain::Name)
                || matches!(t, TermRef::Call(f, _) if {
                    let f = v.text(f);
                    f == "concat" || f == "cat" || crate::code_rule_dsl::logic::strings::functions::native_call(f).is_some_and(|(_, _, k)| k == crate::code_rule_dsl::logic::strings::functions::Kind::Text)
                })
        };
        if matches!(op, CmpOp::Lt | CmpOp::Le | CmpOp::Gt | CmpOp::Ge) && (symbol(l) || symbol(r)) {
            return (T::Call("str_cmp", vec![self.lowered(l, Some(Domain::Name)), self.lowered(r, Some(Domain::Name))]), T::Int(0));
        }
        // Membership of a kind or field, `@equals($k, {"a", "b"})`: `@in` of their atoms.
        if let TermRef::Call(f, args) = l
            && v.text(f) == "equals"
            && let [x, set] = args.collect::<Vec<_>>().as_slice()
            && let Some(d @ (Domain::Kind | Domain::Field)) = self.of(*x)
        {
            let set = match *set {
                TermRef::Call(g, items) if v.text(g) == SET => T::CallAtom(g, items.map(|i| self.lowered(i, Some(d))).collect()),
                one => self.lowered(one, Some(d)),
            };
            return (T::Call("in", vec![as_is(self.r, *x), set]), as_is(self.r, r));
        }
        let (dl, dr) = (self.of(r), self.of(l));
        (self.lowered(l, dl), self.lowered(r, dr))
    }
}

impl Rewrite for Rw<'_, '_> {
    fn prefix(&mut self, a: AtomRef<'_>, _out: &mut Vec<T<'static>>) {
        self.rel = Some(self.v.rel_name(a.rel()));
        self.col = 0;
    }
    fn term(&mut self, site: Site<'_>, t: TermRef<'_>) -> Option<T<'static>> {
        match site {
            Site::Head(_) | Site::Atom(_) => {
                let d = self.rel.and_then(|rel| self.domains.get(rel)).and_then(|ds| ds.get(self.col)).copied();
                self.col += 1;
                Some(self.lowered(t, d))
            }
            Site::Cmp(l) => {
                if let Some((n, right)) = self.right.take()
                    && n == l.node() {
                        return Some(right);
                    }
                let Lit::Cmp { l: lt, op, r: rt } = l.view() else { return None };
                let (left, right) = self.cmp(lt, op, rt);
                self.right = Some((l.node(), right));
                Some(left)
            }
        }
    }
}

/// The facts' strings as atoms by their columns' domains, and each kind set's kinds as their ids (`EVERY_KIND`: every
/// kind of the dialect).
struct FactsAndSets<'a> {
    k: &'a Constants<'a>,
    domains: &'a Domains,
}

impl ProgramPass for FactsAndSets<'_> {
    const NAME: &'static str = "facts and kind sets lowered";
    fn run(&mut self, cx: &Cx<'_>, w: &mut ProgramW<'_>) -> std::result::Result<(), String> {
        let v = cx.v();
        let mut facts: Vec<(String, Vec<Vec<Cell>>)> = Vec::new();
        for f in v.all_facts().into_iter().filter(|f| !f.about()) {
            let rows: Vec<Vec<Cell>> = f.rows().map(|r| r.collect()).collect();
            if !rows.iter().flatten().any(|c| matches!(c, Cell::Str(_))) {
                continue;
            }
            let rel = v.text(f.name());
            let mut out = Vec::with_capacity(rows.len());
            for row in rows {
                let mut lowered = Vec::with_capacity(row.len());
                for (c, cell) in row.into_iter().enumerate() {
                    lowered.push(match cell {
                        Cell::Str(a) => {
                            let d = self.domains.get(rel).and_then(|ds| ds.get(c)).copied().unwrap_or(Domain::Name);
                            self.k.sym(v.text(a), d).map_err(|e| match e {
                                Error::Plan(m) => format!("fact of {rel}, column {c}: {m}"),
                                e => e.to_string(),
                            })?.map_or(Cell::Int(-1), Cell::Branded)
                        }
                        c => c,
                    });
                }
                out.push(lowered);
            }
            facts.push((rel.to_owned(), out));
        }
        let b = w.on()?;
        for (rel, rows) in facts {
            b.fields().set_fact_cells(&rel, rows);
        }
        let rows = b.about_rows("kindset_kind");
        if rows.is_empty() {
            return Ok(());
        }
        // The sets holding `EVERY_KIND`, by their name's atom.
        let every: Vec<Cell> = rows.iter().filter(|r| matches!(r.get(3), Some(Cell::Str(a)) if b.text(*a) == EVERY_KIND)).filter_map(|r| r.get(1).copied()).collect();
        let mut out: Vec<Vec<Cell>> = Vec::with_capacity(rows.len());
        let mut done: Vec<Cell> = Vec::new();
        for row in rows {
            let set = row[1];
            if every.contains(&set) {
                if !done.contains(&set) {
                    done.push(set);
                    for code in 1..self.k.kinds.kind_count() as u16 {
                        let kind = b.atom(&i64::from(kind_atom(self.k.tag, code)).to_string());
                        out.push(vec![row[0], set, Cell::Int(i64::from(code) - 1), Cell::Str(kind)]);
                    }
                }
                continue;
            }
            let mut row = row;
            if let Some(Cell::Str(a)) = row.get(3).copied() {
                let id = self.k.of(b.text(a), Domain::Kind).map_err(|e| e.to_string())?;
                row[3] = Cell::Str(b.atom(&id.to_string()));
            }
            out.push(row);
        }
        b.fields().set_about_rows("kindset_kind", out);
        Ok(())
    }
}
