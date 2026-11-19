//! core.rr's relations in a program: core.rr is every program's prelude (decision 0043), as the libraries read it.
//!
//! - **Declarations.** `rr::program` reads core.rr before every file, so a file's program has all of core.rr's
//!   declarations, and a program that includes core.rr reads core.rr's shape (`plan::text_demand::projection_of`); the
//!   inputs it does not read are neither bound nor planned. A file may not declare or derive a
//!   core.rr relation (decision 0141, `rr::read_file`). A relation read and not declared, which core.rr declares,
//!   has core.rr's declaration and is an input (`declare_core`: a program's text parsed alone, `rr::parse`). Such a
//!   text's own declaration replaces core.rr's: a converted program's `syntax_error(d: number)` (the tape has an
//!   error node, what the corpus binds) stands where core.rr's `syntax_error(d, l, msg)` (the batch's `core_rr` row:
//!   CPython's verdict for Python, `cpython::syntax`, through `facts::vtab::syntax::python_syntax`) would. An authored
//!   overlay, a file, reads the tape's verdict as the converter gives it, `tape_syntax_error(d)` (`logic::convert`),
//!   and CPython's under the name the converter gives it, `cpython_syntax_error(d, l, msg)`.
//! - **Tree arithmetic.** The core relations that are questions about the tape, where the program does not derive
//!   them, lower to the relations the engine has (`rewrite`):
//!
//!   ```text
//!   next_sibling(a, b)  child(p, a, i), j = i + 1, child(p, b, j)
//!   precedes(a, b)      child(p, a, i), child(p, b, j), i < j
//!   earlier(m, n)       node(d, m, _, _, _), visit_index(m, i), node(d, n, _, _, _), visit_index(n, j), i < j
//!   ```
//!
//!   Negated, the three of several literals read a relation of their rows, `rr_core_<name>`, with the same body.
//!   `within` is the engine's relation (decision 0050), `kind` and T's `docof` projections of `node` and core.rr's
//!   `docof` an input (`Lowerer::projections`), and `unset` the value -1 (`lower::unset`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use super::At;
use super::{Lowerer, ty};
use super::Tm;
use crate::code_rule_dsl::logic::front::{Error, is_axis};

const CORE: &str = include_str!("../../../../../../docs/rule-syntax/lib/core.rr");

/// core.rr's declarations, in the engine's column types.
pub(super) static DECLS: LazyLock<BTreeMap<String, Vec<(String, String)>>> = LazyLock::new(|| {
    let decls = CORE.lines().filter(|l| l.starts_with(".decl ")).collect::<Vec<_>>().join("\n");
    let file = super::super::from_tape::RrFile::read(decls).expect("core.rr's declarations parse");
    let t = file.tape();
    file.statements()
        .iter()
        .filter_map(|s| {
            let (name, cols, _) = t.decl(s.node()).ok()?;
            Some((name, cols.into_iter().map(|(c, t)| (c, ty(&t))).collect()))
        })
        .collect()
});

/// The core relations `rewrite` lowers: where the program does not derive one, no relation of the program.
pub(super) const REWRITTEN: [&str; 3] = ["next_sibling", "precedes", "earlier"];

/// Whether core.rr declares `rel`.
pub fn is_core(rel: &str) -> bool {
    DECLS.contains_key(rel)
}

/// A rewritten relation of several literals: its parameters, and its own variables in order of name (the order their
/// fresh names are made in).
fn shape(rel: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match rel {
        "next_sibling" | "precedes" => (&["a", "b"][..], &["i", "j", "p"][..]),
        "earlier" => (&["m", "n"][..], &["d", "i", "j"][..]),
        other => unreachable!("{other} has no definition"),
    }
}

/// Variable `n`'s term in `v`.
fn term_of(v: &[(&str, Tm)], n: &str) -> Tm {
    v.iter().find(|(m, _)| *m == n).map(|(_, t)| t.clone()).expect("a variable of the definition")
}

/// Writes the body of rewritten relation `rel` (`emit_atom`, `emit_cmp`), each variable as `v` gives it.
fn write_definition(me: &mut Lowerer, rel: &str, v: &[(&str, Tm)]) -> Result<(), Error> {
    let x = |n: &str| term_of(v, n);
    let w = || Tm::Wild;
    match rel {
        "next_sibling" => {
            me.emit_atom("child", vec![x("p"), x("a"), x("i")], false, false)?;
            let next = Tm::CallAtom(me.prog.atom("add"), vec![x("i"), Tm::Int(1)]);
            me.emit_cmp(x("j"), "=", next)?;
            me.emit_atom("child", vec![x("p"), x("b"), x("j")], false, false)
        }
        "precedes" => {
            me.emit_atom("child", vec![x("p"), x("a"), x("i")], false, false)?;
            me.emit_atom("child", vec![x("p"), x("b"), x("j")], false, false)?;
            me.emit_cmp(x("i"), "<", x("j"))
        }
        "earlier" => {
            me.emit_atom("node", vec![x("d"), x("m"), w(), w(), w()], false, false)?;
            me.emit_atom("visit_index", vec![x("m"), x("i")], false, false)?;
            me.emit_atom("node", vec![x("d"), x("n"), w(), w(), w()], false, false)?;
            me.emit_atom("visit_index", vec![x("n"), x("j")], false, false)?;
            me.emit_cmp(x("i"), "<", x("j"))
        }
        other => unreachable!("{other} has no definition"),
    }
}

impl Lowerer {
    /// Whether `rel` is a core relation `rewrite` lowers in this program: one it does not derive.
    pub(super) fn rewritten(&self, rel: &str) -> bool {
        REWRITTEN.contains(&rel) && !self.derived.contains(rel)
    }

    /// `rel(args)` (negated: `neg`) written as the literals the engine reads, where `rewritten(rel)`: whether it is.
    pub(super) fn rewrite(&mut self, rel: &str, args: &[At], neg: bool) -> Result<bool, Error> {
        if !self.rewritten(rel) {
            return Ok(false);
        }
        let arity = DECLS[rel].len();
        if args.len() != arity {
            return Err(Error::Syntax(format!("{rel} takes {arity} arguments (core.rr), not {}", args.len())));
        }
        let args = args.iter().map(|a| self.term_tm(a)).collect::<Result<Vec<_>, _>>()?;
        if neg {
            let helper = self.helper(rel);
            self.emit_atom(&helper, args, true, false)?;
            return Ok(true);
        }
        let (params, own) = shape(rel);
        let mut map: Vec<(&str, Tm)> = Vec::new();
        for (p, a) in params.iter().zip(args) {
            let a = if a == Tm::Wild {
                let v = self.fresh(rel);
                self.var_tm(&v)
            } else {
                a
            };
            map.push((p, a));
        }
        for v in own {
            let fresh = self.fresh(rel);
            let t = self.var_tm(&fresh);
            map.push((v, t));
        }
        write_definition(self, rel, &map)?;
        Ok(true)
    }

    /// The relation of a rewritten relation's rows, `rr_core_<rel>`, made on its first use.
    fn helper(&mut self, rel: &str) -> String {
        let name = format!("rr_core_{rel}");
        if !self.prog.declares(&name) {
            let (params, own) = shape(rel);
            let cols: Vec<(&str, &str)> = DECLS[rel].iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
            self.prog.declare(&name, &cols);
            let map: Vec<(&str, Tm)> = params.iter().chain(own).map(|v| (*v, self.var_tm(v))).collect();
            let args: Vec<Tm> = params.iter().map(|p| term_of(&map, p)).collect();
            self.fixed_rule_with(&name, &args, false, |me| write_definition(me, rel, &map));
        }
        name
    }
}

/// core.rr's declaration of each relation `read` names (with the arity it is read with: what the rules read, `reads`)
/// that `prog` does not declare, as an input; `docof_of_locs` set where it is core.rr's `docof` (over spans). `prog`
/// may be a header alone (a version's), since it reads no rules.
pub fn declare_core(prog: &mut (impl crate::code_rule_dsl::logic::program_image::fields::FieldsMut + ?Sized), read: &BTreeSet<(String, usize)>, docof_of_locs: &mut bool) -> Result<(), Error> {
    use crate::code_rule_dsl::logic::program_facts::Field;
    for (rel, arity) in read {
        if prog.declares(rel) || is_axis(rel) {
            continue;
        }
        let Some(cols) = DECLS.get(rel) else { continue };
        if cols.len() != *arity {
            return Err(Error::Syntax(format!("{rel} is read with {arity} columns and core.rr declares {}", cols.len())));
        }
        if rel == "docof" {
            // core.rr's `docof(x: loc, d)`: an input that spans have rows in, not `node`'s projection.
            *docof_of_locs = true;
        }
        let pairs: Vec<(&str, &str)> = cols.iter().map(|(c, t)| (c.as_str(), t.as_str())).collect();
        prog.declare(rel, &pairs);
        // Its tape columns stay tapes (the engine's `sym`): a printed program declares them `tape` again.
        let tapes = super::super::tape_type::tape_columns(rel);
        if !tapes.is_empty() {
            let mut all = prog.tapes();
            all.entry(rel.clone()).or_default().extend(tapes);
            prog.set_field(Field::Tapes(&all));
        }
        prog.add_input(rel);
    }
    Ok(())
}

/// `v` less core.rr's relations it neither reads, derives nor outputs, declared as core.rr declares them: the
/// prelude's declarations and inputs, which are no relations of the program. The converter's overlay reads an
/// authored file so (`logic::convert::overlay`), its declarations checked against the converter's.
#[cfg(any(test, feature = "test-hooks"))]
pub fn drop_unread_core(v: &std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>) -> Result<std::sync::Arc<crate::code_rule_dsl::logic::program_image::Version>, String> {
    use crate::code_rule_dsl::logic::program_image::Builder;
    let used: BTreeSet<String> = v
        .rules()
        .flat_map(|r| std::iter::once(r.head()).chain(r.reads()))
        .map(|a| v.rel_name(a.rel()).to_owned())
        .chain(v.fact_names())
        .chain(v.output_names())
        .collect();
    let unused: Vec<String> = DECLS
        .iter()
        .filter(|(rel, cols)| !used.contains(rel.as_str()) && v.columns_of(rel).is_none_or(|own| own == **cols))
        .map(|(rel, _)| rel.clone())
        .collect();
    if unused.is_empty() {
        return Ok(v.clone());
    }
    let mut b = Builder::on(v);
    for rel in &unused {
        b.undeclare(rel);
    }
    let inputs: Vec<String> = v.input_names().into_iter().filter(|i| !unused.contains(i)).collect();
    b.set_inputs(&inputs);
    Ok(b.finish()?.0)
}
