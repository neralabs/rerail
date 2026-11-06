//! core.rr's relations in a program: core.rr is every program's prelude (decision 0043), as the libraries read it.
//!
//! - **Declarations.** `rr::program` reads core.rr before every file, so a file's program has all of core.rr's
//!   declarations; it keeps those it reads, derives or outputs (`unused_core`). A relation read and not declared,
//!   which core.rr declares, has core.rr's declaration and is an input (`declare_core`: a program's text parsed
//!   alone, `rr::parse`). A program's own declaration replaces core.rr's: an authored overlay's
//!   `syntax_error(d: number)` (the tape has an error node: the converter's, what the corpus binds) stands where
//!   core.rr's `syntax_error(d, l, msg)` (CPython's verdict, line and message) would.
//! - **Tree arithmetic.** The core relations that are questions about the tape, where the program does not derive
//!   them, lower to the relations the engine has (`rewrite`):
//!
//!   ```text
//!   kind(n, k)          node(_, n, k, _, _)
//!   within(n, a)        inside(n, a, {K}) where the rule tests a's kind K (node(_, a, "K", _, _)), else over every kind
//!   next_sibling(a, b)  child(p, a, i), j = i + 1, child(p, b, j)
//!   precedes(a, b)      child(p, a, i), child(p, b, j), i < j
//!   earlier(m, n)       node(d, m, _, _, _), visit_index(m, i), node(d, n, _, _, _), visit_index(n, j), i < j
//!   unset(x)            rr_core_unset(x), one fact: -1, which no node, span or document is, so no tree or text
//!                       relation has it
//!   docof(n, d)         rr_docof(n, d), the corpus's (T's facts derive a docof of their own)
//!   syntax_error(d, l, msg)  cpython_syntax_error(d, l, msg), CPython's verdict (`cpython::syntax`), where the
//!                       program declares no syntax_error of its own
//!   ```
//!
//!   Negated, the three of several literals read a relation of their rows, `rr_core_<name>`, with the same body.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use super::super::syntax::{Item, Parser, Term as STerm};
use super::{Lowerer, kind_set_name, ty};
use crate::code_rule_dsl::logic::front::{AXIS, Atom, EVERY_KIND, Error, Literal, Rule, Term, Value};

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

/// The core relations `rewrite` lowers.
const REWRITTEN: [&str; 8] = ["kind", "within", "next_sibling", "precedes", "earlier", "unset", "docof", "syntax_error"];

/// The relation the corpus binds for `docof` (`facts::vtab::syntax::Extra`): named apart, since programs that run on
/// T's facts derive a `docof` of their own from `node`.
pub(in crate::code_rule_dsl) const DOCOF: &str = "rr_docof";

/// The relation the corpus binds for core.rr's `syntax_error(d, l, msg)`, CPython's verdict
/// (`facts::vtab::syntax::cpython`): named apart from the tape's `syntax_error(d)`, which a program declares.
pub(in crate::code_rule_dsl) const SYNTAX_ERROR: &str = crate::code_rule_dsl::facts::vtab::syntax::cpython::SYNTAX_ERROR;

/// Whether core.rr declares `rel`.
pub(in crate::code_rule_dsl) fn is_core(rel: &str) -> bool {
    DECLS.contains_key(rel)
}

/// The kind set of `within` over every kind: its one member is `EVERY_KIND`.
const ANY: &str = "rr_kind_any";

/// The id of `unset`'s one row.
const UNSET: i64 = -1;

fn atom(rel: &str, args: Vec<Term>, neg: bool) -> Literal {
    Literal::Atom(Atom { rel: rel.to_owned(), args, neg, gated: false })
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
    /// Whether `rel` is a core relation `rewrite` lowers in this program: one it does not derive (nor, for
    /// `syntax_error`, declare otherwise than core.rr does: a program's `syntax_error(d: number)` is the tape's).
    pub(super) fn rewritten(&self, rel: &str) -> bool {
        REWRITTEN.contains(&rel) && !self.derived.contains(rel) && !(rel == "syntax_error" && self.prog.decls.get(rel).is_some_and(|cols| *cols != DECLS[rel]))
    }

    /// `rel(args)` (negated: `neg`) as the literals the engine reads, where `rewritten(rel)`.
    pub(super) fn rewrite(&mut self, rel: &str, args: &[STerm], neg: bool) -> Result<Option<Vec<Literal>>, Error> {
        if !self.rewritten(rel) {
            return Ok(None);
        }
        let arity = DECLS[rel].len();
        if args.len() != arity {
            let why = if rel == "syntax_error" {
                ": the corpus binds syntax_error(d: number), the tape's verdict (declare it); core.rr's syntax_error(d, l, msg) is CPython's"
            } else {
                ""
            };
            return Err(Error::Syntax(format!("{rel} takes {arity} arguments (core.rr), not {}{why}", args.len())));
        }
        let args = args.iter().map(|a| self.term(a)).collect::<Result<Vec<_>, _>>()?;
        Ok(Some(match rel {
            "kind" => vec![atom("node", vec![Term::Wild, args[0].clone(), args[1].clone(), Term::Wild, Term::Wild], neg)],
            "within" => vec![atom(&format!("{AXIS}{ANY}"), args, neg)],
            "docof" | "syntax_error" => {
                let bound = if rel == "docof" { DOCOF } else { SYNTAX_ERROR };
                if !self.prog.decls.contains_key(bound) {
                    self.prog.decls.insert(bound.to_owned(), DECLS[rel].clone());
                    self.prog.inputs.push(bound.to_owned());
                }
                vec![atom(bound, args, neg)]
            }
            "unset" => {
                let name = "rr_core_unset";
                if !self.prog.decls.contains_key(name) {
                    self.prog.decls.insert(name.to_owned(), DECLS[rel].clone());
                    self.prog.facts.entry(name.to_owned()).or_default().push(vec![Value::Int(UNSET)]);
                }
                vec![atom(name, args, neg)]
            }
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
                    if !map.contains_key(&v) {
                        let fresh = self.fresh(rel);
                        map.insert(v, Term::Var(fresh));
                    }
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
            self.prog.rules.push(Rule { head: Atom { rel: name.clone(), args: params.iter().map(|p| var(p)).collect(), neg: false, gated: false }, body });
        }
        name
    }

    /// `within`'s kind sets: the one kind a rule tests the ancestor for, else every kind.
    pub(super) fn within_kinds(&mut self) {
        let any = format!("{AXIS}{ANY}");
        let (mut every, mut sets) = (false, BTreeSet::new());
        for rule in &mut self.prog.rules {
            let kinds: BTreeMap<String, String> = rule
                .body
                .iter()
                .filter_map(|l| match l {
                    Literal::Atom(a) if a.rel == "node" && !a.neg => match (a.args.get(1), a.args.get(2)) {
                        (Some(Term::Var(n)), Some(Term::Str(k))) => Some((n.clone(), k.clone())),
                        _ => None,
                    },
                    _ => None,
                })
                .collect();
            for l in &mut rule.body {
                let Literal::Atom(a) = l else { continue };
                if a.rel != any {
                    continue;
                }
                match a.args.get(1) {
                    Some(Term::Var(x)) if kinds.contains_key(x) => {
                        let kind = kinds[x].clone();
                        let set = kind_set_name(&kind);
                        a.rel = format!("{AXIS}{set}");
                        sets.insert((set, kind));
                    }
                    _ => every = true,
                }
            }
        }
        for (set, kind) in sets {
            self.prog.kindsets.insert(set, vec![kind]);
        }
        if every {
            self.prog.kindsets.insert(ANY.to_owned(), vec![EVERY_KIND.to_owned()]);
        }
    }

    /// core.rr's declaration of each relation the program reads and does not declare, as an input.
    pub(super) fn declare_core(&mut self) -> Result<(), Error> {
        let read: BTreeSet<(String, usize)> = self
            .prog
            .rules
            .iter()
            .flat_map(|r| r.body.iter().filter_map(Literal::atom))
            .map(|a| (a.rel.clone(), a.args.len()))
            .collect();
        for (rel, arity) in read {
            if self.prog.decls.contains_key(&rel) || rel.starts_with(AXIS) {
                continue;
            }
            let Some(cols) = DECLS.get(&rel) else { continue };
            if cols.len() != arity {
                return Err(Error::Syntax(format!("{rel} is read with {arity} columns and core.rr declares {}", cols.len())));
            }
            self.prog.decls.insert(rel.clone(), cols.clone());
            if !self.prog.inputs.contains(&rel) {
                self.prog.inputs.push(rel);
            }
        }
        Ok(())
    }

    /// core.rr's relations the program neither reads, derives nor outputs, as core.rr declares them: the prelude's
    /// declarations and inputs, which are no relations of this program.
    pub(super) fn unused_core(&mut self) {
        let prog = &self.prog;
        let used: BTreeSet<String> = prog
            .rules
            .iter()
            .flat_map(|r| std::iter::once(&r.head).chain(r.body.iter().filter_map(Literal::atom)))
            .map(|a| a.rel.clone())
            .chain(prog.facts.keys().cloned())
            .chain(prog.outputs.iter().cloned())
            .collect();
        let unused: Vec<String> = DECLS.iter().filter(|(rel, cols)| !used.contains(*rel) && prog.decls.get(*rel).is_none_or(|own| own == *cols)).map(|(rel, _)| rel.clone()).collect();
        for rel in unused {
            self.prog.decls.remove(&rel);
            self.prog.inputs.retain(|i| *i != rel);
        }
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
