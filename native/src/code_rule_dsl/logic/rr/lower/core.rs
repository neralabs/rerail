//! core.rr's relations in a program: core.rr is every program's prelude (decision 0043), as the libraries read it.
//!
//! - **Declarations.** `rr::program` reads core.rr before every file, so a file's program has all of core.rr's
//!   declarations, and a program that includes core.rr reads core.rr's shape (`syntax::Projection::of`); the
//!   inputs it does not read are neither bound nor planned. A file may not declare or derive a
//!   core.rr relation (decision 0047a, `rr::read_file`). A relation read and not declared, which core.rr declares,
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

use rerail_rr_syntax::syntax::{Item, Parser, Term as STerm};
use super::{Lowerer, ty};
use crate::code_rule_dsl::logic::front::{Atom, Error, Literal, Rule, Term, is_axis};

const CORE: &str = include_str!("../../../../../../docs/rule-syntax/lib/core.rr");

/// core.rr's declarations, in the engine's column types.
pub(super) static DECLS: LazyLock<BTreeMap<String, Vec<(String, String)>>> = LazyLock::new(|| {
    let decls = CORE.lines().filter(|l| l.starts_with(".decl ")).collect::<Vec<_>>().join("\n");
    let statements = Parser::new(&decls).and_then(|mut p| p.statements()).expect("core.rr's declarations parse");
    statements
        .into_iter()
        .filter_map(|(item, _)| match item {
            Item::Decl { name, cols, .. } => Some((name, cols.into_iter().map(|(c, t)| (c, ty(&t))).collect())),
            _ => None,
        })
        .collect()
});

/// The core relations `rewrite` lowers: where the program does not derive one, no relation of the program.
pub(super) const REWRITTEN: [&str; 3] = ["next_sibling", "precedes", "earlier"];

/// Whether core.rr declares `rel`.
pub fn is_core(rel: &str) -> bool {
    DECLS.contains_key(rel)
}

fn atom(rel: &str, args: Vec<Term>, neg: bool) -> Literal {
    Literal::Atom(Atom { rel: rel.to_owned().into(), args, neg, gated: false })
}

fn var(v: &str) -> Term {
    Term::Var(v.to_owned())
}

fn less(l: &str, r: &str) -> Literal {
    Literal::Cmp { l: var(l), op: "<", r: var(r) }
}

/// A rewritten relation of several literals: its parameters, and its body over them and variables of its own.
fn definition(rel: &str) -> (&'static [&'static str], Vec<Literal>) {
    let w = || Term::Wild;
    match rel {
        "next_sibling" => (
            &["a", "b"],
            vec![
                atom("child", vec![var("p"), var("a"), var("i")], false),
                Literal::Cmp { l: var("j"), op: "=", r: Term::Call("add".to_owned(), vec![var("i"), Term::Int(1)]) },
                atom("child", vec![var("p"), var("b"), var("j")], false),
            ],
        ),
        "precedes" => (
            &["a", "b"],
            vec![atom("child", vec![var("p"), var("a"), var("i")], false), atom("child", vec![var("p"), var("b"), var("j")], false), less("i", "j")],
        ),
        "earlier" => (
            &["m", "n"],
            vec![
                atom("node", vec![var("d"), var("m"), w(), w(), w()], false),
                atom("visit_index", vec![var("m"), var("i")], false),
                atom("node", vec![var("d"), var("n"), w(), w(), w()], false),
                atom("visit_index", vec![var("n"), var("j")], false),
                less("i", "j"),
            ],
        ),
        other => unreachable!("{other} has no definition"),
    }
}

fn substitute(t: &Term, map: &BTreeMap<String, Term>) -> Term {
    match t {
        Term::Var(v) => map.get(v).cloned().unwrap_or_else(|| t.clone()),
        Term::Call(f, args) => Term::Call(f.clone(), args.iter().map(|a| substitute(a, map)).collect()),
        t => t.clone(),
    }
}

impl Lowerer {
    /// Whether `rel` is a core relation `rewrite` lowers in this program: one it does not derive.
    pub(super) fn rewritten(&self, rel: &str) -> bool {
        REWRITTEN.contains(&rel) && !self.derived.contains(rel)
    }

    /// `rel(args)` (negated: `neg`) as the literals the engine reads, where `rewritten(rel)`.
    pub(super) fn rewrite(&mut self, rel: &str, args: &[STerm], neg: bool) -> Result<Option<Vec<Literal>>, Error> {
        if !self.rewritten(rel) {
            return Ok(None);
        }
        let arity = DECLS[rel].len();
        if args.len() != arity {
            return Err(Error::Syntax(format!("{rel} takes {arity} arguments (core.rr), not {}", args.len())));
        }
        let args = args.iter().map(|a| self.term(a)).collect::<Result<Vec<_>, _>>()?;
        Ok(Some(match rel {
            _ if !neg => {
                let (params, body) = definition(rel);
                let mut map = BTreeMap::new();
                for (p, a) in params.iter().zip(&args) {
                    let a = if *a == Term::Wild { Term::Var(self.fresh(rel)) } else { a.clone() };
                    map.insert((*p).to_owned(), a);
                }
                let mut own = BTreeSet::new();
                body.iter().for_each(|l| literal_vars(l, &mut own));
                for v in own {
                    map.entry(v).or_insert_with(|| Term::Var(self.fresh(rel)));
                }
                body.iter().map(|l| substitute_literal(l, &map)).collect()
            }
            _ => vec![atom(&self.helper(rel), args, true)],
        }))
    }

    /// The relation of a rewritten relation's rows, `rr_core_<rel>`, made on its first use.
    fn helper(&mut self, rel: &str) -> String {
        let name = format!("rr_core_{rel}");
        if !self.prog.decls.contains_key(&name) {
            let (params, body) = definition(rel);
            self.prog.decls.insert(name.clone(), DECLS[rel].clone());
            self.prog.rules.push(Rule { head: Atom { rel: name.clone().into(), args: params.iter().map(|p| var(p)).collect(), neg: false, gated: false }, body });
        }
        name
    }

    /// core.rr's declaration of each relation the program reads and does not declare, as an input.
    pub(super) fn declare_core(&mut self) -> Result<(), Error> {
        let read: BTreeSet<(String, usize)> = self
            .prog
            .rules
            .iter()
            .flat_map(|r| r.body.iter().filter_map(Literal::atom))
            .map(|a| (a.rel.to_string(), a.args.len()))
            .collect();
        for (rel, arity) in read {
            if self.prog.decls.contains_key(&rel) || is_axis(&rel) {
                continue;
            }
            let Some(cols) = DECLS.get(&rel) else { continue };
            if cols.len() != arity {
                return Err(Error::Syntax(format!("{rel} is read with {arity} columns and core.rr declares {}", cols.len())));
            }
            if rel == "docof" {
                // core.rr's `docof(x: loc, d)`: an input that spans have rows in, not `node`'s projection.
                self.docof_of_locs = true;
            }
            self.prog.decls.insert(rel.clone(), cols.clone());
            // Its tape columns stay tapes (the engine's `sym`): a printed program declares them `tape` again.
            let tapes = super::super::tape_type::tape_columns(&rel);
            if !tapes.is_empty() {
                self.prog.tapes.entry(rel.clone()).or_default().extend(tapes);
            }
            if !self.prog.inputs.contains(&rel) {
                self.prog.inputs.push(rel);
            }
        }
        Ok(())
    }
}

/// Takes out core.rr's relations `prog` neither reads, derives nor outputs, declared as core.rr declares them: the
/// prelude's declarations and inputs, which are no relations of the program. The converter's overlay reads an
/// authored file so (`logic::convert::overlay`), its declarations checked against the converter's.
#[cfg(any(test, feature = "test-hooks"))]
pub fn drop_unread_core(prog: &mut crate::code_rule_dsl::logic::front::Program) {
    let used: BTreeSet<String> = prog
        .rules
        .iter()
        .flat_map(|r| std::iter::once(&r.head).chain(r.body.iter().filter_map(Literal::atom)))
        .map(|a| a.rel.to_string())
        .chain(prog.facts.keys().cloned())
        .chain(prog.outputs.iter().cloned())
        .collect();
    let unused: Vec<String> = DECLS.iter().filter(|(rel, cols)| !used.contains(*rel) && prog.decls.get(*rel).is_none_or(|own| own == *cols)).map(|(rel, _)| rel.clone()).collect();
    for rel in unused {
        prog.decls.remove(&rel);
        prog.inputs.retain(|i| *i != rel);
    }
}

fn term_vars(t: &Term, out: &mut BTreeSet<String>) {
    match t {
        Term::Var(v) => {
            out.insert(v.clone());
        }
        Term::Call(_, args) => args.iter().for_each(|a| term_vars(a, out)),
        _ => {}
    }
}

fn literal_vars(l: &Literal, out: &mut BTreeSet<String>) {
    match l {
        Literal::Atom(a) => a.args.iter().for_each(|t| term_vars(t, out)),
        Literal::Cmp { l, r, .. } => {
            term_vars(l, out);
            term_vars(r, out);
        }
        _ => unreachable!("a definition has atoms and comparisons"),
    }
}

fn substitute_literal(l: &Literal, map: &BTreeMap<String, Term>) -> Literal {
    match l {
        Literal::Atom(a) => Literal::Atom(Atom { args: a.args.iter().map(|t| substitute(t, map)).collect(), ..a.clone() }),
        Literal::Cmp { l, op, r } => Literal::Cmp { l: substitute(l, map), op, r: substitute(r, map) },
        _ => unreachable!("a definition has atoms and comparisons"),
    }
}
