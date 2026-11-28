//! core.rerail's relations in a program: core.rerail is every program's prelude (decision 0043), as the libraries read it.
//!
//! - **Declarations.** `rr::program` reads core.rerail before every file, so a file's program has all of core.rerail's
//!   declarations, and a program that includes core.rerail reads core.rerail's shape (`plan::text_demand::projection_of`); the
//!   inputs it does not read are neither bound nor planned. A file may not declare or derive a
//!   core.rerail relation (decision 0141, `rr::read_file`). A relation read and not declared, which core.rerail declares,
//!   has core.rerail's declaration and is an input (`declare_core`: a program's text parsed alone, `rr::parse`). Such a
//!   text's own declaration replaces core.rerail's: a converted program's `syntax_error(d: number)` (the tape has an
//!   error node, what the corpus binds) stands where core.rerail's `syntax_error(d, l, msg)` (the batch's `core_rr` row:
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
//!   `within` is the engine's relation (decision 0050), `kind` and T's `docof` projections of `node` and core.rerail's
//!   `docof` an input (`Lowerer::projections`), and `unset` the value -1 (`lower::unset`).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use super::At;
use super::{Lowerer, ty};
use super::Tm;
use crate::code_rule_dsl::logic::front::{Error, is_axis};

const CORE: &str = include_str!("../../../../../../../../docs/rule-syntax/lib/core.rerail");

/// core.rerail's declarations, in the engine's column types.
pub(super) static DECLS: LazyLock<BTreeMap<String, Vec<(String, String)>>> = LazyLock::new(|| {
    let decls = CORE.lines().filter(|l| l.starts_with(".decl ")).collect::<Vec<_>>().join("\n");
    let file = super::super::from_tape::RrFile::read(decls).expect("core.rerail's declarations parse");
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
pub(super) const REWRITTEN: [&str; 9] = ["next_sibling", "precedes", "earlier", "node_to_line", "node_to_end_line", "line_to_node", "match_to_node", "document_line", "place_to_node"];

/// Whether core.rerail declares `rel`.
pub fn is_core(rel: &str) -> bool {
    DECLS.contains_key(rel)
}

/// A rewritten relation of several literals: its parameters, and its own variables in order of name (the order their
/// fresh names are made in).
fn shape(rel: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match rel {
        "next_sibling" | "precedes" => (&["a", "b"][..], &["i", "j", "p"][..]),
        "earlier" => (&["m", "n"][..], &["d", "i", "j"][..]),
        "node_to_line" | "node_to_end_line" => (&["n", "l"][..], &["d", "k"][..]),
        "line_to_node" => (&["l", "n"][..], &["d", "k"][..]),
        "match_to_node" => (&["m", "n"][..], &["a", "c", "d", "s"][..]),
        "document_line" => (&["d", "l"][..], &["k"][..]),
        "place_to_node" => (&["p", "n"][..], &[][..]),
        other => unreachable!("{other} has no definition"),
    }
}

/// The typed readers and conversions that are one of the engine's reads under another name (decision 0724 §4): the
/// relation, the engine's read, and the arity both have. A typed name is what the type check reads (`place_type`);
/// the engine's read takes a node or a span alike.
pub(super) const ALIASES: &[(&str, &str, usize)] = &[
    ("line_text", "text", 2),
    ("token_text", "text", 2),
    ("match_text", "text", 2),
    ("line_document", "docof", 2),
    ("token_document", "docof", 2),
    ("match_document", "docof", 2),
    ("node_line_number", "line", 2),
    ("line_number", "line", 2),
    ("token_line_number", "line", 2),
    ("match_line_number", "line", 2),
    ("node_end_line_number", "end_line", 2),
    ("token_end_line_number", "end_line", 2),
    ("match_end_line_number", "end_line", 2),
    ("node_start_column", "col", 2),
    ("token_start_column", "col", 2),
    ("match_start_column", "col", 2),
    ("document_source", "source", 2),
    ("document_line_by_number", "src_line", 3),
    ("node_to_first_token", "first_token", 2),
    ("node_to_last_token", "last_token", 2),
    ("node_to_token_before", "token_before", 2),
    ("node_to_token_after", "token_after", 2),
    ("token_previous", "token_before", 2),
    ("token_next", "token_after", 2),
    ("node_to_end_position", "end_point", 2),
    ("place_text", "text", 2),
    ("place_document", "docof", 2),
    ("place_line_number", "line", 2),
    ("place_end_line_number", "end_line", 2),
    ("place_start_column", "col", 2),
    ("place_to_first_token", "first_token", 2),
    ("place_to_last_token", "last_token", 2),
    ("place_to_token_before", "token_before", 2),
    ("place_to_token_after", "token_after", 2),
    ("place_to_end_position", "end_point", 2),
];

/// `rel`'s engine read and arity, where it is an alias (`ALIASES`): the lowering reads the engine's read in its
/// place (`clause::lit_node`), its terms as written, so a computed one is bound as any atom's is.
pub(crate) fn alias(rel: &str) -> Option<(&'static str, &'static str, usize)> {
    ALIASES.iter().copied().find(|(r, _, _)| *r == rel)
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
        "node_to_line" | "node_to_end_line" => {
            let read = if rel == "node_to_line" { "line" } else { "end_line" };
            me.emit_atom(read, vec![x("n"), x("k")], false, false)?;
            me.emit_atom("docof", vec![x("n"), x("d")], false, false)?;
            me.emit_atom("src_line", vec![x("d"), x("k"), x("l")], false, false)
        }
        "line_to_node" => {
            me.emit_atom("line", vec![x("l"), x("k")], false, false)?;
            me.emit_atom("docof", vec![x("l"), x("d")], false, false)?;
            me.emit_atom("node", vec![x("d"), x("n"), w(), w(), w()], false, false)?;
            me.emit_atom("line", vec![x("n"), x("k")], false, false)
        }
        // The nodes whose span is the match's: the same start (line and column) and the same text, so the same end.
        "match_to_node" => {
            me.emit_atom("docof", vec![x("m"), x("d")], false, false)?;
            me.emit_atom("line", vec![x("m"), x("a")], false, false)?;
            me.emit_atom("col", vec![x("m"), x("c")], false, false)?;
            me.emit_atom("text", vec![x("m"), x("s")], false, false)?;
            me.emit_atom("node", vec![x("d"), x("n"), w(), w(), w()], false, false)?;
            me.emit_atom("line", vec![x("n"), x("a")], false, false)?;
            me.emit_atom("col", vec![x("n"), x("c")], false, false)?;
            me.emit_atom("node_text", vec![x("n"), x("s")], false, false)
        }
        "document_line" => me.emit_atom("src_line", vec![x("d"), x("k"), x("l")], false, false),
        // A place that is a node: the node.
        "place_to_node" => {
            me.emit_atom("node", vec![w(), x("p"), w(), w(), w()], false, false)?;
            me.emit_cmp(x("n"), "=", x("p"))
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
            return Err(Error::Syntax(format!("{rel} takes {arity} arguments (core.rerail), not {}", args.len())));
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
            self.prog.fields().declare(&name, &cols);
            let map: Vec<(&str, Tm)> = params.iter().chain(own).map(|v| (*v, self.var_tm(v))).collect();
            let args: Vec<Tm> = params.iter().map(|p| term_of(&map, p)).collect();
            self.fixed_rule_with(&name, &args, false, |me| write_definition(me, rel, &map));
        }
        name
    }
}

/// core.rerail's declaration of each relation `read` names (with the arity it is read with: what the rules read, `reads`)
/// that `prog` does not declare, as an input; `docof_of_locs` set where it is core.rerail's `docof` (over spans). `prog`
/// may be a header alone (a version's), since it reads no rules.
pub fn declare_core(prog: &mut (impl crate::code_rule_dsl::logic::program_image::fields::FieldsMut + ?Sized), read: &BTreeSet<(String, usize)>, docof_of_locs: &mut bool) -> Result<(), Error> {
    use crate::code_rule_dsl::logic::program_facts::Field;
    for (rel, arity) in read {
        if prog.declares(rel) || is_axis(rel) {
            continue;
        }
        let Some(cols) = DECLS.get(rel) else { continue };
        if cols.len() != *arity {
            return Err(Error::Syntax(format!("{rel} is read with {arity} columns and core.rerail declares {}", cols.len())));
        }
        if rel == "docof" {
            // core.rerail's `docof(x: loc, d)`: an input that spans have rows in, not `node`'s projection.
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

/// `v` less core.rerail's relations it neither reads, derives nor outputs, declared as core.rerail declares them: the
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
        b.fields().undeclare(rel);
    }
    let inputs: Vec<String> = v.input_names().into_iter().filter(|i| !unused.contains(i)).collect();
    b.fields().set_inputs(&inputs);
    Ok(b.finish()?.0)
}
