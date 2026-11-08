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

/// Tree-sitter's ERROR symbol, past every grammar's kinds and so past the stride.
const ERROR_SYMBOL: u16 = u16::MAX;

/// The atom of kind `code` in the dialect whose tape tag is `tag`. ERROR's atom takes code 0, the grammar's end
/// symbol, which never labels a node (the tape's own `ERROR_CODE`), so every atom of a dialect stays in its stride:
/// `tag * 1024 + 0xffff` would read as another dialect's kind.
pub(crate) fn kind_atom(tag: u8, code: u16) -> u32 {
    u32::from(tag) * KIND_STRIDE + if code == ERROR_SYMBOL { 0 } else { u32::from(code) }
}

/// The tape tag of a kind atom's dialect.
pub(crate) fn kind_tag(atom: u32) -> u8 {
    (atom / KIND_STRIDE) as u8
}

/// The kind code of a kind atom in its dialect's catalog (`kind_atom`'s inverse: code 0 is ERROR).
pub(crate) fn kind_code(atom: u32) -> u16 {
    match atom % KIND_STRIDE {
        0 => ERROR_SYMBOL,
        code => code as u16,
    }
}

/// The tape tag of a document's dialect (`Facts::tag`): a text-only document, which has no nodes, has its own.
pub(crate) fn facts_tag(f: &Facts) -> u8 {
    f.tag
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
    /// Computed only for a demanded Python syntax verdict; shared by every projection of this tape.
    pub python_syntax: std::sync::OnceLock<Option<(u32, String)>>,
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

/// [`build`], with the tape the facts were read off (`None`: the parser stopped at a limit): W's values, f-string
/// trees and string annotations read it.
pub(crate) fn build_with_tape(file: &FileInput) -> Result<Option<(Facts, TapeDoc)>, String> {
    let language = eval::file_language(file).ok_or_else(|| format!("{} has no grammar", file.language))?;
    let tape = eval::file_tape(language, file).map_err(|e| e.reason.to_string())?;
    let Some((doc, lines)) = tape else {
        return Ok(None);
    };
    let _facts = crate::code_rule_dsl::logic::profile::span(|| "facts from tape".into());
    let facts = from_tape(&doc, lines)?;
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Node, facts.start.len() as u64);
    Ok(Some((facts, doc)))
}

/// [`build`], with the file's tape kept in `cache` (a content store, keyed by the source's content,
/// the grammar's version and the tape format): read from it where it has one, else parsed and stored.
/// The corpus runner's parse cache: a file parsed once is not parsed again by a later run.
pub(crate) fn build_cached(file: &FileInput, cache: &crate::content_store::ContentStore) -> Result<Built, String> {
    build_cached_as(file, cache, None)
}

/// [`build_cached`] for a caller that has already stored the file's source as `known`: it is not hashed or looked up again.
pub(crate) fn build_cached_as(file: &FileInput, cache: &crate::content_store::ContentStore, known: Option<crate::content_store::SourceId>) -> Result<Built, String> {
    build_cached_source(file, cache, known).map(|(built, _)| built)
}

/// [`build_cached_as`], with the source's id as the store holds it after the build. A parsed tape is stored with its
/// source in one transaction (O95), so a source collected since the caller resolved `known` is stored again, under a
/// new id; `None` when the file did not parse and its source is not known.
pub(crate) fn build_cached_source(file: &FileInput, cache: &crate::content_store::ContentStore, known: Option<crate::content_store::SourceId>) -> Result<(Built, Option<crate::content_store::SourceId>), String> {
    let (built, id) = build_cached_with_tape(file, cache, known)?;
    Ok((built.map_or(Built::Unparsed, |(facts, _)| Built::Facts(facts)), id))
}

/// [`build_cached_source`], with the tape the facts were read off (`None`: the parser stopped at a limit), as
/// [`build_with_tape`] gives it.
pub(crate) fn build_cached_with_tape(
    file: &FileInput,
    cache: &crate::content_store::ContentStore,
    known: Option<crate::content_store::SourceId>,
) -> Result<(Option<(Facts, TapeDoc)>, Option<crate::content_store::SourceId>), String> {
    // A standalone cache build has the same lookup/parse/publication window as a corpus run.
    let _reading = cache.hold_collection().map_err(|e| e.to_string())?;
    use crate::code_rule_dsl::lines::Terminators;
    use crate::code_rule_dsl::tape::{self, FileImage, TAPE_FORMAT_VERSION, TapeImage};
    use crate::code_rule_dsl::workspace::grammar_version;
    use crate::content_store::HashKey;
    let language = eval::file_language(file).ok_or_else(|| format!("{} has no grammar", file.language))?;
    let tag = i64::from(tape_tag(language).ok_or("the dialect has no tape tag")?);
    let (gv, fv) = (grammar_version(), i64::from(TAPE_FORMAT_VERSION));
    let e = |e: crate::content_store::StoreError| e.to_string();
    let bytes = file.source.as_bytes();
    let lookup = crate::code_rule_dsl::logic::profile::span(|| "tape cache lookup".into());
    // The source as the store has it, when it has it: a new one is stored with its tape below.
    let mut source_id = match known {
        Some(id) => Some(id),
        None => {
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, bytes.len() as u64);
            cache.source_by_hash(HashKey::sha256(bytes)).map_err(e)?
        }
    };
    let stored = match source_id {
        Some(id) => cache.tape_by_source(id, tag, gv, fv).map_err(e)?,
        None => None,
    };
    drop(lookup);
    let grammar = tape::Grammar::new(language)?;
    let primary = match stored.and_then(|id| cache.get_tapes_bulk(&[id]).ok()?.remove(&id)) {
        Some((stored, _)) => {
            let _decode = crate::code_rule_dsl::logic::profile::span(|| "tape decode".into());
            TapeImage::from_bytes(&stored, &grammar)?
        }
        None => {
            let parse = crate::code_rule_dsl::logic::profile::span(|| "tape parse".into());
            // Its own work: the source prepared for the parser (tree-sitter and the encoding charge their own).
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, file.source.len() as u64);
            let Some(image) = TapeImage::parse(&grammar, &file.source, &|| false) else {
                return Ok((None, source_id));
            };
            let image = image?;
            drop(parse);
            let _store = crate::code_rule_dsl::logic::profile::span(|| "tape store".into());
            let tape = image.primary.to_bytes()?;
            crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, tape.len() as u64);
            let (id, _) = cache.put_source_and_tape(bytes, HashKey::sha256(bytes), tag, gv, fv, &tape).map_err(e)?;
            source_id = Some(id);
            image.primary
        }
    };
    let image = FileImage { primary, derived: Vec::new() };
    let lines = Arc::new(LineTable::new(&file.source, Terminators::of(language)));
    let restore = crate::code_rule_dsl::logic::profile::span(|| "tape restore".into());
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, file.source.len() as u64);
    let doc = TapeDoc::restore_with_lines(Arc::clone(&file.source), grammar, &image, Arc::clone(&lines), &|| false)?;
    drop(restore);
    let _facts = crate::code_rule_dsl::logic::profile::span(|| "facts from tape".into());
    let facts = from_tape(&doc, lines)?;
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Node, facts.start.len() as u64);
    Ok((Some((facts, doc)), source_id))
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
        python_syntax: Default::default(),
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
        python_syntax: Default::default(),
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
        python_syntax: Default::default(),
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
    // Each column as one run of little-endian bytes (what one `update` per value fed, at a fraction of the calls).
    let mut bytes = f.source.len() + f.flags.len() + std::mem::size_of_val(&f.field[..]);
    for column in [&f.kind, &f.parent, &f.start, &f.end, &f.class_keys, &f.class_offsets, &f.class_nodes] {
        hash.update((column.len() as u64).to_le_bytes());
        update_le(&mut hash, column);
        bytes += std::mem::size_of_val(&column[..]);
    }
    hash.update(&f.flags);
    update_le(&mut hash, &f.field);
    crate::code_rule_dsl::logic::profile::work(crate::code_rule_dsl::logic::profile::Unit::Byte, bytes as u64);
    hash.update([u8::from(f.syntax_errors)]);
    hash.finalize().into()
}

/// Feeds `values` to `hash` as their little-endian bytes, in order.
fn update_le<T: Copy + LeBytes>(hash: &mut sha2::Sha256, values: &[T]) {
    use sha2::Digest as _;
    if cfg!(target_endian = "little") {
        // SAFETY: `T` is a primitive integer (`LeBytes`), so `values` is `size_of_val(values)` initialized bytes with
        // no padding, in memory order, which is little-endian here.
        hash.update(unsafe { std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values)) });
    } else {
        for value in values {
            value.feed(hash);
        }
    }
}

/// An integer type with no padding whose bytes `update_le` may read in place.
trait LeBytes {
    fn feed(&self, hash: &mut sha2::Sha256);
}
impl LeBytes for u32 {
    fn feed(&self, hash: &mut sha2::Sha256) {
        sha2::Digest::update(hash, self.to_le_bytes());
    }
}
impl LeBytes for u16 {
    fn feed(&self, hash: &mut sha2::Sha256) {
        sha2::Digest::update(hash, self.to_le_bytes());
    }
}

#[cfg(test)]
mod tests;
