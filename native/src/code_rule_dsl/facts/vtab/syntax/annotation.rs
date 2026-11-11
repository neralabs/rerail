//! Python's string annotations as core.rr's relation, made by a batch only for a program that reads it
//! (`Annotations::new`'s `reads`), off the document's tape (`Document::tape`):
//!
//! ```text
//! string_annotation(s)             s (a `string` or `concatenated_string`) is a string Ruff 0.16.9's semantic model
//!                                  reads as a type definition (`scope::python::typedefs`: the scope table's
//!                                  `annotations`), nested ones included
//! ```
//!
//! Its tree is decision 0055's guest: `parsed(s, "annotation", root)` asks for it (`corpus::guests` parses the string's
//! value as `( … )`, the expression inside the parentheses its root), and a string Ruff's parser refuses has
//! `string_annotation(s)` and no `parsed` row (its guest does not parse and is withheld). A string nested in an
//! annotation (`"List['A']"`'s `'A'`) is a node of its outer annotation's guest: its row is made once that guest is a
//! document of the batch, found by its range in the guest's text (one byte on, past the `(`).

use super::{Dialect, Document, Projected, Relation};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::literal::SubTree;
use crate::code_rule_dsl::scope::{Reading, ScopeTable};
use crate::content_store::{StoreError, StoreResult};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

/// The relation's name.
pub(crate) const RELATION: &str = "string_annotation";

/// The grammar of a string annotation's guest (`corpus::guests`).
const GRAMMAR: &str = "annotation";

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

/// A string annotation nested in another: its range in the outer one's text, and its own nested ones.
#[derive(Clone)]
struct Nested {
    start: usize,
    end: usize,
    inner: Vec<Nested>,
}

#[derive(Default)]
pub(super) struct Annotations {
    string_annotation: bool,
    hosts: Vec<i64>,
    /// The nested string annotations of each string annotation (by its node), for its guest.
    nested: HashMap<i64, Vec<Nested>>,
    /// Each annotation guest of the batch: the node it stands at, and its string nodes by range in its text.
    guests: Vec<(i64, ByRange)>,
}

/// String nodes by their range in a text.
type ByRange = HashMap<(u32, u32), i64>;

impl Annotations {
    pub(super) fn new(reads: &BTreeSet<String>) -> Self {
        Self { string_annotation: reads.contains(RELATION), ..Self::default() }
    }

    /// Document `p`'s rows (`d` its document; `python`: it is Python's). A file's are read off its tape; an annotation
    /// guest's strings wait for `relations`, as its host may be a guest the batch holds after it.
    pub(super) fn document(&mut self, p: &Projected, d: &Document, dialect: Option<&Dialect>, python: bool) -> StoreResult<()> {
        let (true, true, Some(dialect)) = (self.string_annotation, python, dialect) else { return Ok(()) };
        // The string hosts of the projection, by range.
        let f = &*p.facts;
        let by_range: ByRange = p
            .core
            .iter()
            .enumerate()
            .filter(|&(_, &id)| HOSTS.contains(&dialect.name(f, id)))
            .map(|(local, &id)| ((f.start[id as usize], f.end[id as usize]), p.global(local as u32)))
            .collect();
        if let Some(g) = d.guest.as_deref() {
            if g.grammar == GRAMMAR {
                self.guests.push((g.at, by_range));
            }
            return Ok(());
        }
        let Some((tape, _)) = &d.tape else {
            return Err(StoreError::Invalid(format!("{}: string_annotation needs the document's tape", d.path)));
        };
        if tape.transducer_len() != f.len() {
            return Err(StoreError::Invalid(format!("{}: the tape is not the one the facts were read off", d.path)));
        }
        let found = string_annotations(tape).map_err(|e| StoreError::Invalid(format!("{}: {e}", d.path)))?;
        // Each annotation's nested ones: an outer one comes before its nested ones, so the innermost are placed first.
        let mut nested: Vec<Vec<Nested>> = vec![Vec::new(); found.len()];
        for k in (0..found.len()).rev() {
            if let Some(o) = found[k].outer {
                let inner = std::mem::take(&mut nested[k]);
                nested[o].push(Nested { start: found[k].start, end: found[k].end, inner });
            }
        }
        for (k, a) in found.iter().enumerate().filter(|(_, a)| a.outer.is_none()) {
            if let Some(&s) = by_range.get(&(a.start as u32, a.end as u32)) {
                self.hosts.push(s);
                let inner = std::mem::take(&mut nested[k]);
                if !inner.is_empty() {
                    self.nested.insert(s, inner);
                }
            }
        }
        Ok(())
    }

    /// The relation read: the files' string annotations, and those nested in the annotation guests of the batch.
    pub(super) fn relations(mut self) -> Vec<(&'static str, Relation)> {
        if !self.string_annotation {
            return Vec::new();
        }
        // A guest's strings are placed once its own host's are: a guest of a guest may come before it.
        let mut pending = std::mem::take(&mut self.guests);
        loop {
            let before = pending.len();
            pending.retain(|(at, by_range)| {
                let Some(nested) = self.nested.get(at).cloned() else { return true };
                for n in nested {
                    // The guest's text is the string's value in `( … )`.
                    if let Some(&s) = by_range.get(&((n.start + 1) as u32, (n.end + 1) as u32)) {
                        self.hosts.push(s);
                        if !n.inner.is_empty() {
                            self.nested.insert(s, n.inner);
                        }
                    }
                }
                false
            });
            if pending.len() == before {
                break;
            }
        }
        let mut r = Relation::new(&["s"]);
        let hosts: BTreeSet<i64> = self.hosts.into_iter().collect();
        hosts.iter().for_each(|&s| r.push(&[s]));
        vec![(RELATION, r)]
    }
}

/// Python's string annotations as a batch's producer (`syntax::Producer`).
pub(crate) struct Producer;

impl super::Producer for Producer {
    fn dialect(&self) -> crate::code_rule_dsl::language::GrammarDialect {
        crate::code_rule_dsl::language::GrammarDialect::Python
    }
    fn taped(&self) -> &'static [&'static str] {
        &[RELATION]
    }
    fn start(&self, reads: &BTreeSet<String>, _: &[String]) -> Box<dyn super::ProducerRun> {
        Box::new(Annotations::new(reads))
    }
}

impl super::ProducerRun for Annotations {
    fn document(&mut self, p: &Projected, d: &Document, dialect: Option<&Dialect>, own: bool, _: &mut dyn FnMut(&str) -> usize) -> StoreResult<()> {
        Annotations::document(self, p, d, dialect, own)
    }
    fn relations(self: Box<Self>, _: &dyn Fn(i64) -> i64) -> Vec<(&'static str, Relation)> {
        Annotations::relations(*self)
    }
    fn reads_guests(&self) -> bool {
        true
    }
}
