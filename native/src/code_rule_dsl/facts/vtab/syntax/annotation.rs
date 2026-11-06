//! Python's string annotations as core.rr's relations, made by a batch only for a program that reads them
//! (`Annotations::new`'s `reads`), off the document's tape (`Document::tape`):
//!
//! ```text
//! string_annotation(s)             s (a `string` or `concatenated_string`) is a string Ruff 0.16.9's semantic model
//!                                  reads as a type definition (`scope::python::typedefs`: the scope table's
//!                                  `annotations`), nested ones included
//! parsed(s, "annotation", root)    root is the tree of the expression s holds, as Ruff's `parse_type_annotation`
//!                                  reads it (`literal::annotation`): an `expression_statement` over the whole text
//! ```
//!
//! Each tree that parses is a document of its own, past the batch's real documents: key `PARSED | host << 11 | k` for
//! the `k`th annotation of document `host`, its text the string's value (decoded), its nodes numbered as a document's
//! (`Projected`). Its nodes have the rows of a document's nodes (`node`, `child`, `field`, `tape`, `text`, `name`,
//! `str`, `strprefix`, `intval`, `op`, `line`, `end_line`, `col`, `node_text`, `docof`, each over the tree's own text
//! and lines, its document its own key), so `inside(n, root, k)` and the rest read it as any tree; the document has
//! no `doc`, `pkg`, `syntax_error` or span rows. A string nested in an annotation (`"List['A']"`'s `'A'`) is a node of
//! its outer annotation's document, and its own tree another document. A string Ruff's parser refuses gives
//! `string_annotation(s)` and no `parsed` row (CPython's `ast.parse(value, mode="eval")` refuses it too). Rows hold
//! string indexes, as `Batch::build`'s do.

use super::{Dialect, Document, Projected, Relation};
use crate::code_rule_dsl::facts::core;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::literal::SubTree;
use crate::code_rule_dsl::scope::{Reading, ScopeTable};
use crate::content_store::{StoreError, StoreResult};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

/// The relations' names.
pub(crate) const RELATIONS: [&str; 2] = ["string_annotation", "parsed"];

/// The key flag of an annotation's document: real documents' keys are below it.
pub(crate) const PARSED: u32 = 1 << 30;
/// Bits of the annotation's index in its document's key, and the hosts' keys they leave.
const INDEX_BITS: u32 = 11;
const HOST_LIMIT: u32 = 1 << (30 - INDEX_BITS);

/// The kinds a string annotation is a node of.
const HOSTS: [&str; 2] = ["string", "concatenated_string"];

/// One string type definition of a file (`string_annotations`).
pub(crate) struct Found {
    /// The string's range: in the source, or in its outer annotation's tree's text (`outer`).
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) tree: Arc<SubTree>,
    /// The annotation (an index of the list) whose tree holds this string, for a nested one.
    pub(crate) outer: Option<usize>,
}

/// The string type definitions of a Python file, sorted by the scope table's order (the source's first, then nested
/// ones), each nested one's range in its outer annotation's tree's text.
pub(crate) fn string_annotations(tape: &crate::code_rule_dsl::tape::TapeDoc) -> Result<Vec<Found>, String> {
    let grep = ast_grep_core::AstGrep::doc(tape.clone());
    let table = ScopeTable::build_in(grep.root(), GrammarDialect::Python, usize::MAX, &|| false, Reading::default(), None)
        .map_err(|e| format!("scope table: {e:?}"))?;
    let mut out: Vec<Found> = Vec::with_capacity(table.annotations.len());
    for a in &table.annotations {
        // A nested string's range is in its outer tree's range space: that tree's offset plus its text's.
        let outer = out.iter().position(|o| {
            let offset = o.tree.offset();
            !o.tree.is_error() && offset <= a.start && a.end <= offset + o.tree.text().len()
        });
        let (start, end) = match outer {
            Some(o) => (a.start - out[o].tree.offset(), a.end - out[o].tree.offset()),
            None => (a.start, a.end),
        };
        out.push(Found { start, end, tree: Arc::clone(&a.tree), outer });
    }
    Ok(out)
}

#[derive(Default)]
pub(super) struct Annotations {
    string_annotation: bool,
    parsed: bool,
    /// The string index of `"annotation"`, once a `parsed` row holds it.
    grammar: i64,
    hosts: Vec<i64>,
    parsed_rows: Vec<[i64; 2]>,
}

impl Annotations {
    pub(super) fn new(reads: &BTreeSet<String>) -> Self {
        Self { string_annotation: reads.contains(RELATIONS[0]), parsed: reads.contains(RELATIONS[1]), ..Self::default() }
    }

    /// Document `p`'s rows (`d` its document; `python`: it is Python's), and its annotations' documents, to be
    /// placed after the batch's real documents.
    pub(super) fn document(&mut self, p: &Projected, d: &Document, dialect: Option<&Dialect>, python: bool, string: &mut dyn FnMut(&str) -> usize) -> StoreResult<Vec<Projected>> {
        let (true, true, Some(dialect)) = (self.string_annotation || self.parsed, python, dialect) else { return Ok(Vec::new()) };
        let f = &*p.facts;
        let Some((tape, _)) = &d.tape else {
            return Err(StoreError::Invalid(format!("{}: string_annotation and parsed need the document's tape", d.path)));
        };
        if tape.transducer_len() != f.len() {
            return Err(StoreError::Invalid(format!("{}: the tape is not the one the facts were read off", d.path)));
        }
        let found = string_annotations(tape).map_err(|e| StoreError::Invalid(format!("{}: {e}", d.path)))?;
        if found.is_empty() {
            return Ok(Vec::new());
        }
        if p.key >= HOST_LIMIT || found.len() > 1 << INDEX_BITS {
            return Err(StoreError::Invalid(format!(
                "{}: string annotations need a document key below {HOST_LIMIT} and at most {} annotations (key {}, {} annotations)",
                d.path,
                1 << INDEX_BITS,
                p.key,
                found.len()
            )));
        }
        // The string hosts of a projection, by range.
        let hosts_of = |q: &Projected| -> HashMap<(u32, u32), u32> {
            let g = &*q.facts;
            q.core
                .iter()
                .enumerate()
                .filter(|&(_, &id)| HOSTS.contains(&dialect.name(g, id)))
                .map(|(local, &id)| ((g.start[id as usize], g.end[id as usize]), local as u32))
                .collect()
        };
        let top = hosts_of(p);
        // Each annotation's document (none where its string does not parse) and that document's hosts.
        let mut trees: Vec<Option<Projected>> = Vec::with_capacity(found.len());
        let mut inner: Vec<HashMap<(u32, u32), u32>> = Vec::with_capacity(found.len());
        for (k, a) in found.iter().enumerate() {
            let tree = (!a.tree.is_error())
                .then(|| Projected::new(PARSED | p.key << INDEX_BITS | k as u32, Arc::new(core::from_subtree(f.tag, &a.tree)), Some(dialect)))
                .filter(|t| t.len() > 0 && t.local[0] == 0);
            let (projection, by_range) = match a.outer {
                None => (Some(p), &top),
                Some(o) => (trees[o].as_ref(), &inner[o]),
            };
            let host = projection.and_then(|q| by_range.get(&(a.start as u32, a.end as u32)).map(|&local| q.global(local)));
            inner.push(tree.as_ref().map(hosts_of).unwrap_or_default());
            if let Some(s) = host {
                if self.string_annotation {
                    self.hosts.push(s);
                }
                if self.parsed && let Some(t) = &tree {
                    self.grammar = string("annotation") as i64;
                    self.parsed_rows.push([s, t.global(0)]);
                }
            }
            trees.push(tree);
        }
        Ok(trees.into_iter().flatten().collect())
    }

    /// The relations read, their strings as atoms (`atom` of a string index).
    pub(super) fn relations(self, atom: &dyn Fn(i64) -> i64) -> Vec<(&'static str, Relation)> {
        let mut out = Vec::new();
        if self.string_annotation {
            let mut r = Relation::new(&["s"]);
            let hosts: BTreeSet<i64> = self.hosts.into_iter().collect();
            hosts.iter().for_each(|&s| r.push(&[s]));
            out.push((RELATIONS[0], r));
        }
        if self.parsed {
            let mut r = Relation::new(&["s", "grammar", "root"]);
            self.parsed_rows.iter().for_each(|&[s, root]| r.push(&[s, atom(self.grammar), root]));
            out.push((RELATIONS[1], r));
        }
        out
    }
}
