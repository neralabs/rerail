//! Rust forms (`.form r FORM [reads s…] [args w…]`, `front::ast::Native::Form`), on the version (decision 0281's
//! path), in two passes:
//!
//! - `Forms`, before the program's demand: each declared relation's rules give way to one rule that only orders it
//!   (each node column over `node`, each other column a constant, each relation it reads read), and it is made a
//!   form. Its rules stay its definition in the rule file, and its form's tests' oracle.
//! - `FormDemand`, after the program's demand: a form whose every rule the transformation guarded by one demand
//!   relation on a column the form is keyed by (`magic_r_bf(n)`, `magic_r_fb(m)`: one for each binding its readers
//!   use) is made for the values those ask of their columns alone, where no such demand depends on the form; any
//!   other is made whole. Whole, a form reads the tape and what it names alone, so
//!   it is never in a recursive stratum (the planner refuses one there). The form's readers read its rows as any
//!   relation's, by index, whatever they bind.

use super::demand::{about_p, edit_native_rows};
use crate::code_rule_dsl::facts::vtab::syntax::forms;
use crate::code_rule_dsl::logic::program_image::fields::ProgramFields;
use crate::code_rule_dsl::logic::program_image::manager::{Cx, ProgramPass, ProgramW};
use crate::code_rule_dsl::logic::program_image::{About, ArchivedNativeSpec, CmpOp, Cell, Lit, NativeKind, Rows, T, TermRef, Version};
use std::collections::{BTreeMap, HashSet};

type Result<T> = std::result::Result<T, String>;

/// The setting that declares a form.
pub const SETTING: &str = "form";

/// A `.form` declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Declared {
    pub rel: String,
    pub form: String,
    pub reads: Vec<String>,
    pub args: Vec<String>,
}

/// `text`, a `.form` setting's: `r FORM [reads s…] [args w…]`, quotes around a word dropped.
pub fn parse(text: &str) -> Result<Declared> {
    let words: Vec<&str> = text.split_whitespace().map(|w| w.trim_matches('"')).collect();
    let [rel, form, rest @ ..] = words.as_slice() else { return Err(format!("`.form {text}`: expected `.form r FORM [reads s…] [args w…]`")) };
    let (mut reads, mut args) = (Vec::new(), Vec::new());
    let mut into: Option<&mut Vec<String>> = None;
    for w in rest {
        match *w {
            "reads" => into = Some(&mut reads),
            "args" => into = Some(&mut args),
            w => match into.as_deref_mut() {
                Some(list) => list.push(w.to_owned()),
                None => return Err(format!("`.form {text}`: {w} follows neither `reads` nor `args`")),
            },
        }
    }
    Ok(Declared { rel: (*rel).to_owned(), form: (*form).to_owned(), reads, args })
}

/// The version's declared forms, checked: each relation declared, each form registered with its relation's column
/// types, each relation read declared, no relation twice.
pub fn declared(v: &Version) -> Result<Vec<Declared>> {
    let mut out: Vec<Declared> = Vec::new();
    for text in ProgramFields::setting_values(v, SETTING) {
        let d = parse(&text)?;
        let bad = |why: String| format!("`.form {text}`: {why}");
        let decl = v.decl_of(&d.rel).ok_or_else(|| bad(format!("{} is not declared", d.rel)))?;
        let types: Vec<&str> = decl.cols().map(|(_, t)| v.text(t)).collect();
        let form = forms::form(&d.form).ok_or_else(|| bad(format!("no Rust form {} is registered", d.form)))?;
        if form.columns() != types.as_slice() {
            return Err(bad(format!("{} makes columns {:?}, {} has {:?}", d.form, form.columns(), d.rel, types)));
        }
        if let Some(r) = d.reads.iter().find(|r| v.decl_of(r).is_none()) {
            return Err(bad(format!("it reads {r}, which is not declared")));
        }
        if out.iter().any(|o| o.rel == d.rel) {
            return Err(bad(format!("{} has two forms", d.rel)));
        }
        out.push(d);
    }
    Ok(out)
}

/// Makes each declared relation a form (module docs).
#[derive(Default)]
pub struct Forms {
    /// The relations made forms.
    pub made: Vec<String>,
}

impl ProgramPass for Forms {
    const NAME: &'static str = "Rust forms";
    fn run(&mut self, _cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let input = w.input().clone();
        let v: &Version = &input;
        let declared = declared(v)?;
        if declared.is_empty() {
            return Ok(());
        }
        let b = w.on()?;
        let rels: HashSet<&str> = declared.iter().map(|d| d.rel.as_str()).collect();
        b.retain(|r| !rels.contains(v.rel_name(r.head().rel())));
        let p = about_p(b);
        let (mut forms_rows, mut reads_rows, mut args_rows) = (Rows::new(), Rows::new(), Rows::new());
        for d in &declared {
            ordering_rule(v, b, &d.rel, &d.reads)?;
            let rel = Cell::Str(b.atom(&d.rel));
            forms_rows.push([p, rel, Cell::Str(b.atom(&d.form))]);
            for (i, r) in d.reads.iter().enumerate() {
                reads_rows.push([p, rel, Cell::Int(i as i64), Cell::Str(b.atom(r))]);
            }
            for (i, a) in d.args.iter().enumerate() {
                args_rows.push([p, rel, Cell::Int(i as i64), Cell::Str(b.atom(a))]);
            }
            self.made.push(d.rel.clone());
        }
        let drop: HashSet<String> = declared.iter().map(|d| d.rel.clone()).collect();
        edit_native_rows(b, "native_form", &drop, forms_rows);
        edit_native_rows(b, "native_form_read", &drop, reads_rows);
        edit_native_rows(b, "native_form_arg", &drop, args_rows);
        Ok(())
    }
}

/// `rel`'s rule that only orders it after what it reads: its first node column over `node` and any other equal to
/// it, each other column a constant, each relation in `reads` read on its first node column (the form's first node).
fn ordering_rule(v: &Version, b: &mut crate::code_rule_dsl::logic::program_image::Builder<'_>, rel: &str, reads: &[String]) -> Result<()> {
    let types: Vec<String> = v.decl_of(rel).ok_or_else(|| format!("{rel} is not declared"))?.cols().map(|(_, t)| v.text(t).to_owned()).collect();
    let vars: Vec<String> = (0..types.len()).map(|i| format!("rr_f{i}")).collect();
    let mut o = b.open_rule();
    b.put_head(&mut o, rel, &vars.iter().map(|x| T::Var(x)).collect::<Vec<_>>());
    // The first node column over `node`, any other equal to it: no product of nodes.
    let mut first: Option<&str> = None;
    for (x, t) in vars.iter().zip(&types) {
        match t.as_str() {
            "node" => match first {
                Some(f) => b.put_cmp(&mut o, T::Var(x), CmpOp::Eq, T::Var(f)),
                None => {
                    b.put_atom(&mut o, "node", &[T::Wild, T::Var(x), T::Wild, T::Wild, T::Wild], false, false);
                    first = Some(x);
                }
            },
            "sym" => b.put_cmp(&mut o, T::Var(x), CmpOp::Eq, T::Str("")),
            _ => b.put_cmp(&mut o, T::Var(x), CmpOp::Eq, T::Int(0)),
        }
    }
    // Each relation read on its first node column, the form's first node: in the form's file, as the form reads it.
    for r in reads {
        let cols: Vec<String> = v.decl_of(r).ok_or_else(|| format!("{rel} reads {r}, which is not declared"))?.cols().map(|(_, t)| v.text(t).to_owned()).collect();
        let at = first.and(cols.iter().position(|t| t == "node"));
        let args: Vec<T<'_>> = (0..cols.len()).map(|i| if Some(i) == at { T::Var(first.expect("a node column")) } else { T::Wild }).collect();
        b.put_atom(&mut o, r.as_str(), &args, false, false);
    }
    b.close_rule(o)?;
    Ok(())
}

/// Keys each form by the demand that guards its rules (module docs).
#[derive(Default)]
pub struct FormDemand {
    /// Each form, and its demand relations with the column each asks (`None`: made whole).
    pub keyed: BTreeMap<String, Option<Vec<(usize, String)>>>,
    /// Whether a form's rules were written again unguarded: its demand's rules are then unread.
    pub unguarded: bool,
}

/// The demand relations `rel`'s rules are each guarded by (`magic_<rel>_…(x)`, `x` the head's column `c`, one of
/// `keyed`), with their columns; `None` when a rule has no such guard.
fn guarded(v: &Version, rel: &str, keyed: &[usize]) -> Option<Vec<(usize, String)>> {
    let prefix = format!("magic_{rel}_");
    let mut demand: Vec<(usize, String)> = Vec::new();
    let mut any = false;
    for r in v.rules().filter(|r| v.rel_name(r.head().rel()) == rel) {
        any = true;
        let head: Vec<TermRef<'_>> = r.head().args().collect();
        let guard = r.body().find_map(|l| {
            let Lit::Atom(a) = l.view() else { return None };
            let name = v.rel_name(a.rel());
            if a.neg() || !name.starts_with(&prefix) {
                return None;
            }
            let args: Vec<TermRef<'_>> = a.args().collect();
            let [TermRef::Var(x)] = args.as_slice() else { return None };
            let x = *x;
            let c = head.iter().position(|t| t.var() == Some(x))?;
            Some((c, name.to_owned()))
        });
        let (c, name) = guard?;
        if !keyed.contains(&c) {
            return None;
        }
        if !demand.contains(&(c, name.clone())) {
            demand.push((c, name));
        }
    }
    any.then_some(())?;
    // A demand made from what the form's rows make (its readers' recursion asks it again) would put the form in
    // their stratum: it is made whole instead.
    if demand.iter().any(|(_, d)| depends_on(v, d, rel)) {
        return None;
    }
    Some(demand)
}

/// Whether relation `from` reads `to`, through rules and natives' reads.
fn depends_on(v: &Version, from: &str, to: &str) -> bool {
    let mut reads: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for r in v.rules() {
        let head = v.rel_name(r.head().rel());
        for l in r.body() {
            let rel = match l.view() {
                Lit::Atom(a) => a.rel(),
                Lit::Agg { atom, .. } => atom.rel(),
                _ => continue,
            };
            reads.entry(head).or_default().push(v.rel_name(rel));
        }
    }
    let about = About::of(v);
    for n in &about.natives {
        reads.entry(v.rel_name(n.rel)).or_default().extend(n.reads.iter().map(|&r| v.rel_name(r)));
    }
    let mut seen: HashSet<&str> = HashSet::new();
    let mut todo = vec![from];
    while let Some(r) = todo.pop() {
        if r == to {
            return true;
        }
        if seen.insert(r) {
            todo.extend(reads.get(r).into_iter().flatten().copied());
        }
    }
    false
}

impl ProgramPass for FormDemand {
    const NAME: &'static str = "Rust forms' demand";
    fn run(&mut self, _cx: &Cx<'_>, w: &mut ProgramW<'_>) -> Result<()> {
        let input = w.input().clone();
        let v: &Version = &input;
        let about = About::of(v);
        let mut found: Vec<(String, String, Option<Vec<(usize, String)>>)> = Vec::new();
        let mut reads_of: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for n in about.natives.iter().filter(|n| n.kind == NativeKind::Form) {
            let ArchivedNativeSpec::Form { form, reads, .. } = &n.spec else { continue };
            let (rel, form) = (v.rel_name(n.rel).to_owned(), v.text(*form).to_owned());
            reads_of.insert(rel.clone(), reads.iter().map(|&r| v.rel_name(r).to_owned()).collect());
            let keyed = forms::form(&form).map_or(&[][..], |f| f.keyed());
            let g = guarded(v, &rel, keyed);
            self.keyed.insert(rel.clone(), g.clone());
            found.push((rel, form, g));
        }
        // A form made whole whose rules demand guarded: written again unguarded, out of its demand's stratum.
        let guarded_whole: HashSet<&str> = found
            .iter()
            .filter(|(rel, _, g)| g.is_none() && v.rules().any(|r| v.rel_name(r.head().rel()) == rel.as_str() && r.body().any(|l| matches!(l.view(), Lit::Atom(a) if v.rel_name(a.rel()).starts_with(&format!("magic_{rel}_"))))))
            .map(|(rel, _, _)| rel.as_str())
            .collect();
        if found.iter().all(|(_, _, g)| g.is_none()) && guarded_whole.is_empty() {
            return Ok(());
        }
        let b = w.on()?;
        if !guarded_whole.is_empty() {
            b.retain(|r| !guarded_whole.contains(v.rel_name(r.head().rel())));
            for rel in &guarded_whole {
                ordering_rule(v, b, rel, &reads_of[*rel])?;
            }
            self.unguarded = true;
        }
        let p = about_p(b);
        let (mut forms_rows, mut demand_rows) = (Rows::new(), Rows::new());
        for (rel, form, g) in &found {
            let r = Cell::Str(b.atom(rel));
            forms_rows.push([p, r, Cell::Str(b.atom(form))]);
            for (i, (c, d)) in g.iter().flatten().enumerate() {
                demand_rows.push([p, r, Cell::Int(i as i64), Cell::Str(b.atom(d)), Cell::Int(*c as i64)]);
            }
        }
        let drop: HashSet<String> = found.iter().map(|f| f.0.clone()).collect();
        edit_native_rows(b, "native_form", &drop, forms_rows);
        edit_native_rows(b, "native_form_demand", &drop, demand_rows);
        Ok(())
    }
}
