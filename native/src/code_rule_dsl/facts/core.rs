//! Core node facts from the tape (docs/final-shape/03-architecture.md §1, §5; 05 Phase 2), adapted
//! from facts-a's per-content syntactic facts (Moti-Velz/facts-engine `spikes/facts-a/src/facts.rs`):
//! columns indexed by preorder node id, built from Rerail's own tape so ids, kinds, fields and
//! spans equal the matcher's (preorder transducer ids over every tree-sitter node, named and
//! anonymous).
//!
//! facts-a parsed Python only and interned kind names in a process-wide table. Here any dialect's
//! tape is read, and a kind is the atom `tape tag · 1024 + kind code`, so kinds of different
//! dialects never collide and no table is needed. The abstract classes the tape records
//! (Tree-sitter supertypes, tape v8) get postings beside the concrete kinds.

use crate::code_rule_dsl::eval;
use crate::code_rule_dsl::language::tape_tag;
use crate::code_rule_dsl::lines::LineTable;
use crate::code_rule_dsl::model::FileInput;
use crate::code_rule_dsl::tape::TapeDoc;
use std::sync::Arc;

pub(crate) const NONE: u32 = u32::MAX;
/// Kind codes per dialect (the tape's `KIND_LIMIT`).
pub(crate) const KIND_STRIDE: u32 = 1024;

/// The atom of kind `code` in the dialect whose tape tag is `tag`. ERROR's code (`u16::MAX`) reaches past the
/// stride, so a kind atom's dialect is its document's (`Facts::tag`), never the atom divided by the stride.
pub(crate) fn kind_atom(tag: u8, code: u16) -> u32 {
    u32::from(tag) * KIND_STRIDE + u32::from(code)
}

/// The kind code of `atom` in the dialect whose tape tag is `tag` (`kind_atom`'s inverse), if it is one of that
/// dialect's.
pub(crate) fn kind_code(tag: u8, atom: u32) -> Option<u16> {
    atom.checked_sub(u32::from(tag) * KIND_STRIDE).and_then(|code| u16::try_from(code).ok())
}

/// One file's facts. Columns are indexed by node id.
pub(crate) struct Facts {
    /// The tape tag of the file's dialect, which every kind atom is of.
    pub tag: u8,
    pub source: Arc<String>,
    pub lines: Arc<LineTable>,
    /// node(id, kind, parent, start, end, start_line, end_line) + subtree_end, flags, field.
    pub kind: Vec<u32>,
    pub parent: Vec<u32>,
    pub start: Vec<u32>,
    pub end: Vec<u32>,
    pub start_line: Vec<u32>,
    pub end_line: Vec<u32>,
    pub subtree_end: Vec<u32>,
    /// named | extra<<1 | missing<<2 | error<<3
    pub flags: Vec<u8>,
    /// Field (grammar field code, 0 = none) under which the node hangs from its parent.
    pub field: Vec<u16>,
    /// Children in order: CSR over parent ids.
    pub child_offsets: Vec<u32>,
    pub children: Vec<u32>,
    /// Kind postings (CSR over kind atoms present in this file, sorted by atom).
    pub kind_keys: Vec<u32>,
    pub kind_offsets: Vec<u32>,
    pub kind_nodes: Vec<u32>,
    /// Abstract class postings (CSR over supertype kind atoms, sorted by atom).
    pub class_keys: Vec<u32>,
    pub class_offsets: Vec<u32>,
    pub class_nodes: Vec<u32>,
    pub syntax_errors: bool,
}

pub(crate) enum Built {
    Facts(Facts),
    /// The parser stopped at a limit (`file_tape` returned `None`).
    Unparsed,
}

/// Parses `file` in its own dialect and reads its facts off the tape.
pub(crate) fn build(file: &FileInput) -> Result<Built, String> {
    Ok(build_with_tape(file)?.map_or(Built::Unparsed, |(facts, _)| Built::Facts(facts)))
}

/// `build`, with the tape the facts were read off (`None`: the parser stopped at a limit).
pub(crate) fn build_with_tape(file: &FileInput) -> Result<Option<(Facts, TapeDoc)>, String> {
    let language = eval::file_language(file).ok_or_else(|| format!("{} has no grammar", file.language))?;
    let tape = eval::file_tape(language, file).map_err(|e| e.reason.to_string())?;
    let Some((doc, lines)) = tape else {
        return Ok(None);
    };
    Ok(Some((from_tape(&doc, lines)?, doc)))
}

pub(crate) fn from_tape(doc: &TapeDoc, lines: Arc<LineTable>) -> Result<Facts, String> {
    let tag = tape_tag(doc.file_dialect()).ok_or("the tape's dialect has no tape tag")?;
    let n = doc.transducer_len();
    let mut f = Facts {
        tag,
        source: Arc::clone(doc.document_source()),
        lines,
        kind: Vec::with_capacity(n),
        parent: Vec::with_capacity(n),
        start: Vec::with_capacity(n),
        end: Vec::with_capacity(n),
        start_line: Vec::with_capacity(n),
        end_line: Vec::with_capacity(n),
        subtree_end: Vec::with_capacity(n),
        flags: Vec::with_capacity(n),
        field: Vec::with_capacity(n),
        child_offsets: Vec::new(),
        children: Vec::new(),
        kind_keys: Vec::new(),
        kind_offsets: Vec::new(),
        kind_nodes: Vec::new(),
        class_keys: Vec::new(),
        class_offsets: Vec::new(),
        class_nodes: Vec::new(),
        syntax_errors: !doc.syntax_error_ranges().is_empty(),
    };
    for id in 0..n as u32 {
        f.kind.push(kind_atom(tag, doc.transducer_kind(id)));
        f.parent.push(doc.transducer_parent(id).unwrap_or(NONE));
        let span = doc.transducer_span(id);
        f.start.push(span.start as u32);
        f.end.push(span.end as u32);
        f.start_line.push(f.lines.line(span.start) as u32 + 1);
        f.end_line.push(f.lines.line(span.end) as u32 + 1);
        f.subtree_end.push(doc.transducer_descendants(id).end);
        f.flags.push(doc.transducer_flags(id));
        f.field.push(doc.transducer_field(id));
    }
    index(&mut f);
    let classes = doc.transducer_supertypes().map(|(node, class)| (kind_atom(tag, class), node)).collect();
    (f.class_keys, f.class_offsets, f.class_nodes) = postings(classes);
    Ok(f)
}

/// The tape tag of a text-only document's facts (`text`): no dialect has it.
pub(crate) const TEXT: u8 = u8::MAX;

/// The facts of a file read as text only (core.rr's `.scope text`): its source and lines, no tree. Its lines are W's
/// for a file with no grammar (`Terminators::Eslint`: `\r\n`, `\r`, `\n`, U+2028, U+2029).
pub(crate) fn text(source: Arc<String>) -> Facts {
    let lines = Arc::new(LineTable::new(&source, crate::code_rule_dsl::lines::Terminators::Eslint));
    let mut f = Facts {
        tag: TEXT,
        source,
        lines,
        kind: Vec::new(),
        parent: Vec::new(),
        start: Vec::new(),
        end: Vec::new(),
        start_line: Vec::new(),
        end_line: Vec::new(),
        subtree_end: Vec::new(),
        flags: Vec::new(),
        field: Vec::new(),
        child_offsets: Vec::new(),
        children: Vec::new(),
        kind_keys: Vec::new(),
        kind_offsets: Vec::new(),
        kind_nodes: Vec::new(),
        class_keys: Vec::new(),
        class_offsets: Vec::new(),
        class_nodes: Vec::new(),
        syntax_errors: false,
    };
    index(&mut f);
    (f.class_keys, f.class_offsets, f.class_nodes) = postings(Vec::new());
    f
}

/// The facts of a parsed literal (`literal::SubTree`) as a document of its own, in the dialect whose tape tag is `tag`
/// (the tree's kinds and fields are that grammar's: an annotation's are Python's): its source is the tree's text
/// (decoded), its lines Ruff's (`\r\n`, `\r`, `\n`), its ranges in that text. A tree has no abstract classes.
pub(crate) fn from_subtree(tag: u8, tree: &crate::code_rule_dsl::literal::SubTree) -> Facts {
    let source = Arc::new(tree.text().to_owned());
    let lines = Arc::new(LineTable::new(&source, crate::code_rule_dsl::lines::Terminators::Newlines));
    let n = tree.len();
    let mut f = Facts {
        tag,
        source,
        lines,
        kind: Vec::with_capacity(n),
        parent: Vec::with_capacity(n),
        start: Vec::with_capacity(n),
        end: Vec::with_capacity(n),
        start_line: Vec::with_capacity(n),
        end_line: Vec::with_capacity(n),
        subtree_end: Vec::with_capacity(n),
        flags: Vec::with_capacity(n),
        field: Vec::with_capacity(n),
        child_offsets: Vec::new(),
        children: Vec::new(),
        kind_keys: Vec::new(),
        kind_offsets: Vec::new(),
        kind_nodes: Vec::new(),
        class_keys: Vec::new(),
        class_offsets: Vec::new(),
        class_nodes: Vec::new(),
        syntax_errors: false,
    };
    for node in tree.preorder() {
        f.kind.push(kind_atom(tag, node.kind));
        f.parent.push(node.parent.unwrap_or(NONE));
        f.start.push(node.start);
        f.end.push(node.end);
        f.start_line.push(f.lines.line(node.start as usize) as u32 + 1);
        f.end_line.push(f.lines.line(node.end as usize) as u32 + 1);
        f.subtree_end.push(f.kind.len() as u32);
        f.flags.push(u8::from(node.named) | u8::from(node.extra) << 1 | u8::from(node.error) << 3);
        f.field.push(node.field);
        f.syntax_errors |= node.error;
    }
    // Preorder: a node's descendants end where its last child's do.
    for id in (0..n).rev() {
        let p = f.parent[id];
        if p != NONE {
            f.subtree_end[p as usize] = f.subtree_end[p as usize].max(f.subtree_end[id]);
        }
    }
    index(&mut f);
    (f.class_keys, f.class_offsets, f.class_nodes) = postings(Vec::new());
    f
}

/// The derived columns: the children CSR (preorder lists a parent's children in order) and the
/// kind postings.
fn index(f: &mut Facts) {
    let n = f.len();
    let mut counts = vec![0u32; n + 1];
    for &p in &f.parent {
        if p != NONE {
            counts[p as usize + 1] += 1;
        }
    }
    for i in 1..=n {
        counts[i] += counts[i - 1];
    }
    let mut cursor = counts.clone();
    let mut children = vec![0u32; counts[n] as usize];
    for (id, &p) in f.parent.iter().enumerate() {
        if p != NONE {
            children[cursor[p as usize] as usize] = id as u32;
            cursor[p as usize] += 1;
        }
    }
    f.child_offsets = counts;
    f.children = children;
    let pairs = f.kind.iter().enumerate().map(|(i, &k)| (k, i as u32)).collect();
    (f.kind_keys, f.kind_offsets, f.kind_nodes) = postings(pairs);
}

/// (key, node) pairs as a CSR: sorted keys, offsets (one past the last), nodes by key then id.
fn postings(mut pairs: Vec<(u32, u32)>) -> (Vec<u32>, Vec<u32>, Vec<u32>) {
    let (mut keys, mut offsets, mut nodes) = (Vec::new(), Vec::new(), Vec::with_capacity(pairs.len()));
    pairs.sort_unstable();
    for (k, id) in pairs {
        if keys.last() != Some(&k) {
            keys.push(k);
            offsets.push(nodes.len() as u32);
        }
        nodes.push(id);
    }
    offsets.push(nodes.len() as u32);
    (keys, offsets, nodes)
}

fn posting<'a>(keys: &[u32], offsets: &[u32], nodes: &'a [u32], key: u32) -> &'a [u32] {
    match keys.binary_search(&key) {
        Ok(i) => &nodes[offsets[i] as usize..offsets[i + 1] as usize],
        Err(_) => &[],
    }
}

impl Facts {
    pub fn len(&self) -> usize {
        self.kind.len()
    }
    /// The tree's depth: nodes on the longest root-to-leaf path (ids are preorder, so a parent's
    /// id is below its children's).
    pub fn depth(&self) -> u32 {
        let mut at = vec![1u32; self.len()];
        for i in 0..self.len() {
            let p = self.parent[i] as usize;
            if p < i {
                at[i] = at[p] + 1;
            }
        }
        at.into_iter().max().unwrap_or(0)
    }
    pub fn children(&self, id: u32) -> &[u32] {
        let a = self.child_offsets[id as usize] as usize;
        let b = self.child_offsets[id as usize + 1] as usize;
        &self.children[a..b]
    }
    pub fn of_kind(&self, kind: u32) -> &[u32] {
        posting(&self.kind_keys, &self.kind_offsets, &self.kind_nodes, kind)
    }
    /// Nodes of the abstract class `class` (a supertype's kind atom), by id.
    pub fn of_class(&self, class: u32) -> &[u32] {
        posting(&self.class_keys, &self.class_offsets, &self.class_nodes, class)
    }
    pub fn text(&self, id: u32) -> &str {
        &self.source[self.start[id as usize] as usize..self.end[id as usize] as usize]
    }
    pub fn named(&self, id: u32) -> bool {
        self.flags[id as usize] & 1 != 0
    }
    pub fn parent(&self, id: u32) -> Option<u32> {
        let p = self.parent[id as usize];
        (p != NONE).then_some(p)
    }
    /// Approximate resident bytes of the columns (excluding source).
    pub fn bytes(&self) -> usize {
        let n = self.len();
        n * (4 * 7 + 1 + 2)
            + self.children.len() * 4
            + self.child_offsets.len() * 4
            + self.kind_nodes.len() * 4
            + self.class_nodes.len() * 4
    }
}

/// The digest of a file's core facts: every column a reader can see, and the source their spans
/// read text from, so equal digests mean equal facts (the core derivation's output, `families`).
/// Cutoff on an edit happens in the narrower families that read these, not here.
pub(crate) fn digest(f: &Facts) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"rerail.core-facts/v1\0");
    hash.update((f.source.len() as u64).to_le_bytes());
    hash.update(f.source.as_bytes());
    hash.update((f.len() as u64).to_le_bytes());
    for column in [&f.kind, &f.parent, &f.start, &f.end, &f.class_keys, &f.class_offsets, &f.class_nodes] {
        hash.update((column.len() as u64).to_le_bytes());
        for value in column.iter() {
            hash.update(value.to_le_bytes());
        }
    }
    hash.update(&f.flags);
    for field in &f.field {
        hash.update(field.to_le_bytes());
    }
    hash.update([u8::from(f.syntax_errors)]);
    hash.finalize().into()
}

#[cfg(test)]
mod tests;
