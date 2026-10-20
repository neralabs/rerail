//! Source-revision-bound rec6 preorder tape and transient ast-grep document.
//! No parser's tree survives encoding; restoration never invokes a parser. A tree-sitter tree's
//! syntax errors are its ERROR and MISSING records; a Blade template's (Forte's lexer errors, or a
//! template Forte does not parse) are byte ranges the tape keeps beside its records.
use crate::code_rule_dsl::blade::forte::tree::{BladeTree, parse_tree};
use crate::code_rule_dsl::blade::view::View;
use crate::code_rule_dsl::embed::{CompiledView, Embedder, Program};
use crate::code_rule_dsl::language::{Embedded, GrammarDialect, KindCatalog};
use crate::code_rule_dsl::lines::LineTable;
#[cfg(test)]
use crate::code_rule_dsl::lines::Terminators;
use ast_grep_core::Position;
use ast_grep_core::source::{Doc, Edit, SgNode};
use ast_grep_language::LanguageExt;
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::ops::Range;
use std::sync::Arc;
use tree_sitter::{Language, Node as TsNode};

const WIDTH: usize = 6;
pub(super) const TAPE_FORMAT_VERSION: u32 = 5;
const TAPE_MAGIC: &[u8; 8] = b"TMPTAPE5";
const KIND_LIMIT: usize = 1024;
const FIELD_LIMIT: usize = 64;
const SENTINEL: u16 = u16::MAX;
const NONE: u32 = u32::MAX;
const CHECKPOINT_STRIDE: usize = 16;
const CHECKPOINT_LIMIT: usize = 0x00ff_ffff;
/// Tree-sitter's ERROR symbol does not fit the 10-bit kind code. ERROR records store the
/// grammar's hidden end symbol, which never labels a visible node, and are listed in `error`.
const ERROR_CODE: u16 = 0;
const ERROR_SYMBOL: u16 = u16::MAX;

#[cfg(test)]
thread_local! {
    /// The largest table capacity `TapeImage::from_bytes` reserved on this thread since the last
    /// `take_table_reservation`.
    static TABLE_RESERVATION: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn note_table_reservation(capacity: usize) {
    TABLE_RESERVATION.set(TABLE_RESERVATION.get().max(capacity));
}

/// The largest alias or override table capacity reserved since the last call, then resets it.
#[cfg(test)]
pub(super) fn take_table_reservation() -> usize {
    TABLE_RESERVATION.replace(0)
}

/// Deterministic grammar-wide codes: kind code is the dialect's kind id, field code its field id
/// (zero means no field), both as `GrammarDialect::kinds` numbers them; a Blade tape also holds
/// the PHP its template embeds (`blade::view`), its codes following Blade's.
pub(super) struct Grammar {
    lang: GrammarDialect,
    kinds: KindCatalog,
    /// Each kind code's name and whether it is named, read once: Tree-sitter answers a name
    /// by measuring and validating its C string on every call, and `kind` is the matcher's
    /// most frequent question.
    named: Vec<(Option<&'static str>, bool)>,
}

impl Grammar {
    pub(super) fn new(lang: GrammarDialect) -> Result<Arc<Self>, String> {
        let kinds = lang.kinds();
        if kinds.kind_count() > KIND_LIMIT || kinds.field_count() >= FIELD_LIMIT {
            return Err("grammar exceeds rec6 kind/field code width".into());
        }
        let named = (0..kinds.kind_count())
            .map(|code| {
                let code = code as u16;
                (kinds.kind_name(code), kinds.kind_is_named(code))
            })
            .collect();
        Ok(Arc::new(Self { lang, kinds, named }))
    }

    fn kind_name(&self, code: u16) -> Option<&'static str> {
        match self.named.get(usize::from(code)) {
            Some((name, _)) => *name,
            // ERROR's code lies past the grammar's symbols.
            None => self.kinds.kind_name(code),
        }
    }

    fn kind_is_named(&self, code: u16) -> bool {
        match self.named.get(usize::from(code)) {
            Some((_, named)) => *named,
            None => self.kinds.kind_is_named(code),
        }
    }
}

/// Owns only compact structure and a binding to the exact source and grammar.
#[derive(Clone)]
pub(super) struct TapeImage {
    rec: Vec<u8>,
    spans: Vec<u8>,
    checkpoints: Vec<u8>,
    par_overflow: Vec<(u32, u32)>,
    size_overflow: Vec<(u32, u32)>,
    extra: Vec<u32>,
    /// Preorder indices of ERROR nodes and of zero-width MISSING tokens.
    error: Vec<u32>,
    missing: Vec<u32>,
    /// Contextual Tree-sitter aliases whose displayed kind or named flag differs from the static
    /// language catalog. The numeric kind remains the native `kind_id` for matcher parity.
    kind_aliases: Vec<(String, bool)>,
    kind_overrides: Vec<(u32, u16)>,
    /// Syntax errors no record stands for, as `start..end` byte ranges in source order: a Blade
    /// template's (`BladeTree::error_ranges`).
    syntax_errors: Vec<(u32, u32)>,
    /// The programs the tape embeds (`embed`), by preorder index of their roots.
    programs: Vec<EmbeddedProgram>,
    language: GrammarDialect,
    source_len: usize,
    source_digest: [u8; 32],
}

/// A program a tape embeds (`embed`): its root, a child of the tape's root; the dialect it is
/// read as, which its kind codes do not tell (TypeScript, TSX and JavaScript share them:
/// `KindCatalog`); and whether it stands apart from the host's tree (`Embedder::APART`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct EmbeddedProgram {
    pub(crate) root: u32,
    pub(crate) dialect: GrammarDialect,
    pub(crate) apart: bool,
}

#[derive(Clone)]
struct Pending {
    kf: u16,
    parent: Option<u32>,
    start: u32,
    length: u32,
    size: u32,
    extra: bool,
    error: bool,
    missing: bool,
    kind_alias: Option<(String, bool)>,
}

fn checked(budget: &impl Fn() -> bool) -> Result<(), String> {
    if budget() {
        Err("tape operation interrupted".into())
    } else {
        Ok(())
    }
}

fn source_digest(source: &str, budget: &impl Fn() -> bool) -> Result<[u8; 32], String> {
    let mut digest = Sha256::new();
    for chunk in source.as_bytes().chunks(4096) {
        checked(budget)?;
        digest.update(chunk);
    }
    checked(budget)?;
    Ok(digest.finalize().into())
}

fn varint(mut value: u32, output: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 127) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 128;
        }
        output.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn read_varint(bytes: &[u8], at: &mut usize) -> Result<u32, String> {
    let mut result = 0u32;
    for shift in (0..=28).step_by(7) {
        let byte = *bytes.get(*at).ok_or("truncated tape span")?;
        *at += 1;
        if shift == 28 && byte > 0x0f {
            return Err("span varint overflow".into());
        }
        result |= u32::from(byte & 127) << shift;
        if byte & 128 == 0 {
            return Ok(result);
        }
    }
    Err("span varint overflow".into())
}

fn checkpoint(value: usize, bytes: &mut Vec<u8>) -> Result<(), String> {
    if value > CHECKPOINT_LIMIT {
        return Err("rec6 checkpoint overflow".into());
    }
    bytes.extend_from_slice(&(value as u32).to_le_bytes()[..3]);
    Ok(())
}

struct TapeReader<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> TapeReader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], String> {
        let end = self
            .offset
            .checked_add(length)
            .filter(|&end| end <= self.bytes.len())
            .ok_or("truncated tape image")?;
        let value = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(value)
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(
            self.take(4)?.try_into().expect("u32 width"),
        ))
    }

    fn u64(&mut self) -> Result<u64, String> {
        Ok(u64::from_le_bytes(
            self.take(8)?.try_into().expect("u64 width"),
        ))
    }

    fn length(&mut self) -> Result<usize, String> {
        usize::try_from(self.u64()?).map_err(|_| "tape image length overflow".into())
    }

    fn pairs(&mut self, count: usize) -> Result<Vec<(u32, u32)>, String> {
        let byte_length = count.checked_mul(8).ok_or("tape image length overflow")?;
        let mut reader = TapeReader {
            bytes: self.take(byte_length)?,
            offset: 0,
        };
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push((reader.u32()?, reader.u32()?));
        }
        Ok(values)
    }

    fn words(&mut self, count: usize) -> Result<Vec<u32>, String> {
        let byte_length = count.checked_mul(4).ok_or("tape image length overflow")?;
        let mut reader = TapeReader {
            bytes: self.take(byte_length)?,
            offset: 0,
        };
        let mut values = Vec::with_capacity(count);
        for _ in 0..count {
            values.push(reader.u32()?);
        }
        Ok(values)
    }
}

fn validate_image_tables(
    rec: &[u8],
    checkpoints: &[u8],
    par_overflow: &[(u32, u32)],
    size_overflow: &[(u32, u32)],
    extra: &[u32],
    error: &[u32],
    missing: &[u32],
) -> Result<(), String> {
    if rec.is_empty() || !rec.len().is_multiple_of(WIDTH) {
        return Err("invalid tape record width".into());
    }
    let nodes = rec.len() / WIDTH;
    let expected_checkpoints = nodes
        .div_ceil(CHECKPOINT_STRIDE)
        .checked_mul(WIDTH)
        .ok_or("checkpoint length overflow")?;
    if checkpoints.len() != expected_checkpoints {
        return Err("invalid tape checkpoint length".into());
    }
    for (entries, word) in [(par_overflow, 2_usize), (size_overflow, 4_usize)] {
        let mut previous = None;
        for &(index, value) in entries {
            if usize::try_from(index).map_or(true, |index| index >= nodes)
                || previous.is_some_and(|previous| previous >= index)
                || value < u32::from(SENTINEL)
            {
                return Err("noncanonical tape overflow table".into());
            }
            let offset = index as usize * WIDTH + word;
            if u16::from_le_bytes([rec[offset], rec[offset + 1]]) != SENTINEL {
                return Err("unused tape overflow entry".into());
            }
            previous = Some(index);
        }
        for index in 0..nodes {
            let offset = index * WIDTH + word;
            if u16::from_le_bytes([rec[offset], rec[offset + 1]]) == SENTINEL
                && entries
                    .binary_search_by_key(&(index as u32), |&(at, _)| at)
                    .is_err()
            {
                return Err("missing tape overflow entry".into());
            }
        }
    }
    for (entries, name) in [(extra, "extra"), (error, "error"), (missing, "missing")] {
        let mut previous = None;
        for &index in entries {
            if usize::try_from(index).map_or(true, |index| index >= nodes)
                || previous.is_some_and(|previous| previous >= index)
            {
                return Err(format!("noncanonical tape {name} table"));
            }
            previous = Some(index);
        }
    }
    Ok(())
}

fn overflow_value(entries: &[(u32, u32)], index: u32, sentinel: u16) -> Result<u32, String> {
    if sentinel != SENTINEL {
        return Ok(u32::from(sentinel));
    }
    entries
        .binary_search_by_key(&index, |&(at, _)| at)
        .ok()
        .map(|slot| entries[slot].1)
        .ok_or_else(|| "missing rec6 overflow".into())
}

/// Appends `program`, read with `embedded`, as the root's next child, and records it with its
/// dialect and whether it stands `apart` from the host's tree (`EmbeddedProgram`).
fn encode_program<P: Program>(
    nodes: &mut Vec<Pending>,
    programs: &mut Vec<EmbeddedProgram>,
    program: &P,
    embedded: &Embedded,
    apart: bool,
    budget: &impl Fn() -> bool,
) -> Result<(), String> {
    let root = u32::try_from(nodes.len()).map_err(|_| "rec6 node count overflow")?;
    encode_tree(
        nodes,
        program.root(),
        Some(0),
        embedded.grammar,
        Some(embedded),
        Some(program),
        budget,
    )?;
    if nodes
        .get(root as usize)
        .is_none_or(|node| node.parent != Some(0))
    {
        return Err("an embedded program's root is a placeholder".into());
    }
    programs.push(EmbeddedProgram {
        root,
        dialect: program.dialect(),
        apart,
    });
    Ok(())
}

/// Appends `root`'s tree to `nodes` in preorder under `parent`. `language` is the tree's grammar
/// and `numbering` how the tape's grammar codes its kinds and fields: as they are, for the file's
/// own tree (`None`), or as an embedded grammar's (`language::Embedded`). An embedded program's
/// nodes take the host span they stand for (`Program::span`), kept inside their parent's, and its
/// placeholders are left out.
fn encode_tree<P: Program>(
    nodes: &mut Vec<Pending>,
    root: TsNode<'_>,
    parent: Option<u32>,
    language: &Language,
    numbering: Option<&Embedded>,
    program: Option<&P>,
    budget: &impl Fn() -> bool,
) -> Result<(), String> {
    // Per kind id, the grammar's name for it and whether it is named, read once: a node whose
    // own name or namedness differs is a contextual alias.
    let mut catalog: Vec<Option<(Option<&str>, bool)>> = vec![None; language.node_kind_count()];
    // One cursor walks the tree in preorder. `parent` is the tape index of the node the cursor's
    // node is a child of; `above` holds the same for each node the cursor descended through.
    let mut cursor = root.walk();
    let mut parent = parent;
    let mut above: Vec<Option<u32>> = Vec::new();
    let mut visited = 0usize;
    loop {
        // Every 256 nodes, omitted ones included.
        if visited.is_multiple_of(256) {
            checked(budget)?;
        }
        visited += 1;
        let node = cursor.node();
        let index = if program.is_some_and(|program| program.omitted(node)) {
            None
        } else {
            // The cursor's root is `root` itself, which has no field.
            let field = cursor.field_id().map_or(0, |id| {
                numbering.map_or(id.get(), |numbering| numbering.field_code(id.get()))
            });
            Some(encode_node(
                nodes,
                node,
                parent,
                field,
                language,
                numbering,
                program,
                &mut catalog,
            )?)
        };
        if let Some(index) = index
            && cursor.goto_first_child()
        {
            above.push(parent);
            parent = Some(index);
            continue;
        }
        // The next sibling, of this node or of its nearest ancestor that has one.
        loop {
            let Some(grandparent) = above.last().copied() else {
                return Ok(());
            };
            if cursor.goto_next_sibling() {
                break;
            }
            cursor.goto_parent();
            above.pop();
            parent = grandparent;
        }
    }
}

/// Appends `node` under `parent` as the child in `field`; its tape index.
#[allow(clippy::too_many_arguments)]
fn encode_node<'l, P: Program>(
    nodes: &mut Vec<Pending>,
    node: TsNode<'_>,
    parent: Option<u32>,
    field: u16,
    language: &'l Language,
    numbering: Option<&Embedded>,
    program: Option<&P>,
    catalog: &mut [Option<(Option<&'l str>, bool)>],
) -> Result<u32, String> {
    let index = u32::try_from(nodes.len()).map_err(|_| "rec6 node count overflow")?;
    let kind = usize::from(node.kind_id());
    let error = node.is_error();
    if usize::from(field) >= FIELD_LIMIT {
        return Err(format!(
            "grammar field id {field} exceeds tape limit {FIELD_LIMIT}"
        ));
    }
    let code = numbering.map_or(node.kind_id(), |numbering| {
        numbering.kind_code(node.kind_id())
    });
    if error {
        if node.kind_id() != ERROR_SYMBOL || node.is_missing() {
            return Err("unsupported error or missing grammar symbol".into());
        }
    } else if kind >= language.node_kind_count() {
        return Err(format!(
            "grammar symbol {kind} is not in the semantic catalog"
        ));
    } else if usize::from(code) >= KIND_LIMIT {
        return Err(format!(
            "grammar symbol {code} exceeds tape limit {KIND_LIMIT}"
        ));
    } else if code == ERROR_CODE {
        return Err("grammar symbol conflicts with the reserved tape error code".into());
    } else if node.is_missing() && !node.byte_range().is_empty() {
        return Err("missing grammar symbol has a non-empty source span".into());
    }
    let span = match program {
        Some(program) => {
            let span = program.span(node);
            // A placeholder's anchor or a recovered node may reach past its parent's span.
            let (low, high) = parent.map_or((span.start, span.end), |parent| {
                let parent = &nodes[parent as usize];
                let start = parent.start as usize;
                (start, start + parent.length as usize)
            });
            let start = span.start.clamp(low, high);
            start..span.end.clamp(start, high)
        }
        None => node.byte_range(),
    };
    let start = u32::try_from(span.start).map_err(|_| "node span overflow")?;
    let end = u32::try_from(span.end).map_err(|_| "node span overflow")?;
    let length = end.checked_sub(start).ok_or("inverted node span")?;
    let kind_alias = if error {
        None
    } else {
        let (name, named) = *catalog[kind].get_or_insert_with(|| {
            (
                language.node_kind_for_id(node.kind_id()),
                language.node_kind_is_named(node.kind_id()),
            )
        });
        // SAFETY: `node` is a live node of a tree in `language`, whose kind names are static
        // NUL-terminated strings.
        let own = unsafe { tree_sitter::ffi::ts_node_type(node.into_raw()) };
        // The same string when the same pointer; otherwise compared as Tree-sitter's `kind` would.
        let same_name = name.is_some_and(|name| {
            std::ptr::eq(name.as_ptr(), own.cast())
                || name.as_bytes() == unsafe { std::ffi::CStr::from_ptr(own) }.to_bytes()
        });
        (!same_name || named != node.is_named()).then(|| (node.kind().to_owned(), node.is_named()))
    };
    nodes.push(Pending {
        kf: if error { ERROR_CODE } else { code } | (field << 10),
        parent,
        start,
        length,
        size: 1,
        extra: node.is_extra(),
        error,
        missing: node.is_missing(),
        kind_alias,
    });
    Ok(index)
}

/// Appends a Blade template's tree (`blade::forte::tree`) to `nodes` in preorder: its kinds are the
/// catalog's Blade kinds, its fields the catalog's ids for their names.
fn encode_blade_tree(
    nodes: &mut Vec<Pending>,
    tree: &BladeTree,
    kinds: KindCatalog,
    budget: &impl Fn() -> bool,
) -> Result<(), String> {
    let mut stack = vec![(tree.root(), None)];
    let mut visited = 0usize;
    while let Some((node, parent)) = stack.pop() {
        if visited.is_multiple_of(256) {
            checked(budget)?;
        }
        visited += 1;
        let index = u32::try_from(nodes.len()).map_err(|_| "rec6 node count overflow")?;
        let field = match node.field_name() {
            Some(name) => kinds
                .field_id(name)
                .ok_or_else(|| format!("Blade field {name} is not in the catalog"))?,
            None => 0,
        };
        let code = node.kind_id();
        if code == ERROR_CODE
            || usize::from(code) >= KIND_LIMIT
            || usize::from(field) >= FIELD_LIMIT
        {
            return Err(format!(
                "Blade kind {code} or field {field} exceeds the tape's codes"
            ));
        }
        let range = node.range();
        let start = u32::try_from(range.start).map_err(|_| "node span overflow")?;
        let end = u32::try_from(range.end).map_err(|_| "node span overflow")?;
        nodes.push(Pending {
            kf: code | (field << 10),
            parent,
            start,
            length: end.checked_sub(start).ok_or("inverted node span")?,
            size: 1,
            extra: false,
            error: false,
            missing: false,
            kind_alias: None,
        });
        let children = node.children().collect::<Vec<_>>();
        stack.extend(children.into_iter().rev().map(|child| (child, Some(index))));
    }
    Ok(())
}

impl TapeImage {
    pub(super) fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let language =
            super::language::tape_tag(self.language).ok_or("unsupported tape language")?;
        let source_len = u64::try_from(self.source_len).map_err(|_| "source length overflow")?;
        let alias_bytes = self
            .kind_aliases
            .iter()
            .try_fold(0usize, |total, (name, _)| {
                total
                    .checked_add(5)
                    .and_then(|value| value.checked_add(name.len()))
                    .ok_or("contextual grammar alias length overflow")
            })?;
        let lengths = [
            u64::try_from(self.rec.len()).map_err(|_| "record length overflow")?,
            u64::try_from(self.spans.len()).map_err(|_| "span length overflow")?,
            u64::try_from(self.checkpoints.len()).map_err(|_| "checkpoint length overflow")?,
            u64::try_from(self.par_overflow.len()).map_err(|_| "parent overflow count")?,
            u64::try_from(self.size_overflow.len()).map_err(|_| "size overflow count")?,
            u64::try_from(self.extra.len()).map_err(|_| "extra count overflow")?,
            u64::try_from(self.error.len()).map_err(|_| "error count overflow")?,
            u64::try_from(self.missing.len()).map_err(|_| "missing count overflow")?,
            u64::try_from(self.kind_aliases.len()).map_err(|_| "kind alias count overflow")?,
            u64::try_from(alias_bytes).map_err(|_| "kind alias length overflow")?,
            u64::try_from(self.kind_overrides.len()).map_err(|_| "kind override count overflow")?,
            u64::try_from(self.syntax_errors.len()).map_err(|_| "syntax error count overflow")?,
            u64::try_from(self.programs.len()).map_err(|_| "embedded program count overflow")?,
        ];
        // Each program's dialect as its index among the grammars the language embeds.
        let embeddings = self.language.kinds().embeddings();
        let programs = self
            .programs
            .iter()
            .map(|program| {
                let embedding = embeddings
                    .iter()
                    .position(|embedded| embedded.dialect == program.dialect)
                    .and_then(|index| u8::try_from(index).ok())
                    .ok_or("an embedded program's dialect is not one the language embeds")?;
                Ok((program.root, embedding, u8::from(program.apart)))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let payload = self
            .rec
            .len()
            .checked_add(self.spans.len())
            .and_then(|value| value.checked_add(self.checkpoints.len()))
            .and_then(|value| value.checked_add(self.par_overflow.len().checked_mul(8)?))
            .and_then(|value| value.checked_add(self.size_overflow.len().checked_mul(8)?))
            .and_then(|value| value.checked_add(self.extra.len().checked_mul(4)?))
            .and_then(|value| value.checked_add(self.error.len().checked_mul(4)?))
            .and_then(|value| value.checked_add(self.missing.len().checked_mul(4)?))
            .and_then(|value| value.checked_add(self.syntax_errors.len().checked_mul(8)?))
            .and_then(|value| value.checked_add(programs.len().checked_mul(6)?))
            .and_then(|value| value.checked_add(alias_bytes))
            .and_then(|value| value.checked_add(self.kind_overrides.len().checked_mul(6)?))
            .ok_or("tape image length overflow")?;
        let mut bytes = Vec::with_capacity(
            160_usize
                .checked_add(payload)
                .ok_or("tape image length overflow")?,
        );
        bytes.extend_from_slice(TAPE_MAGIC);
        bytes.push(language);
        bytes.extend_from_slice(&[0; 3]);
        bytes.extend_from_slice(&TAPE_FORMAT_VERSION.to_le_bytes());
        bytes.extend_from_slice(&source_len.to_le_bytes());
        bytes.extend_from_slice(&self.source_digest);
        for length in lengths {
            bytes.extend_from_slice(&length.to_le_bytes());
        }
        bytes.extend_from_slice(&self.rec);
        bytes.extend_from_slice(&self.spans);
        bytes.extend_from_slice(&self.checkpoints);
        for &(index, value) in &self.par_overflow {
            bytes.extend_from_slice(&index.to_le_bytes());
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for &(index, value) in &self.size_overflow {
            bytes.extend_from_slice(&index.to_le_bytes());
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for entries in [&self.extra, &self.error, &self.missing] {
            for &index in entries {
                bytes.extend_from_slice(&index.to_le_bytes());
            }
        }
        for &(start, end) in &self.syntax_errors {
            bytes.extend_from_slice(&start.to_le_bytes());
            bytes.extend_from_slice(&end.to_le_bytes());
        }
        for (root, embedding, apart) in programs {
            bytes.extend_from_slice(&root.to_le_bytes());
            bytes.push(embedding);
            bytes.push(apart);
        }
        for (name, named) in &self.kind_aliases {
            bytes.push(u8::from(*named));
            bytes.extend_from_slice(
                &u32::try_from(name.len())
                    .map_err(|_| "contextual grammar alias length overflow")?
                    .to_le_bytes(),
            );
            bytes.extend_from_slice(name.as_bytes());
        }
        for &(index, alias) in &self.kind_overrides {
            bytes.extend_from_slice(&index.to_le_bytes());
            bytes.extend_from_slice(&alias.to_le_bytes());
        }
        Ok(bytes)
    }

    pub(super) fn from_bytes(bytes: &[u8], grammar: &Arc<Grammar>) -> Result<Self, String> {
        let mut reader = TapeReader { bytes, offset: 0 };
        if reader.take(8)? != TAPE_MAGIC {
            return Err("wrong tape image magic".into());
        }
        let language = super::language::from_tape_tag(reader.u8()?)
            .ok_or("unsupported tape image language")?;
        if reader.take(3)? != [0; 3] {
            return Err("noncanonical tape image header".into());
        }
        if reader.u32()? != TAPE_FORMAT_VERSION {
            return Err("unsupported tape image version".into());
        }
        if language != grammar.lang {
            return Err("tape image grammar mismatch".into());
        }
        let source_len = usize::try_from(reader.u64()?).map_err(|_| "source length overflow")?;
        let source_digest: [u8; 32] = reader.take(32)?.try_into().expect("digest width");
        let rec_len = reader.length()?;
        let spans_len = reader.length()?;
        let checkpoints_len = reader.length()?;
        let par_count = reader.length()?;
        let size_count = reader.length()?;
        let extra_count = reader.length()?;
        let error_count = reader.length()?;
        let missing_count = reader.length()?;
        let alias_count = reader.length()?;
        let alias_bytes = reader.length()?;
        let override_count = reader.length()?;
        let syntax_error_count = reader.length()?;
        let program_count = reader.length()?;
        let rec = reader.take(rec_len)?.to_vec();
        let spans = reader.take(spans_len)?.to_vec();
        let checkpoints = reader.take(checkpoints_len)?.to_vec();
        let par_overflow = reader.pairs(par_count)?;
        let size_overflow = reader.pairs(size_count)?;
        let extra = reader.words(extra_count)?;
        let error = reader.words(error_count)?;
        let missing = reader.words(missing_count)?;
        let syntax_errors = reader.pairs(syntax_error_count)?;
        if syntax_errors.windows(2).any(|pair| pair[0] > pair[1])
            || syntax_errors
                .iter()
                .any(|&(start, end)| start > end || end as usize > source_len)
        {
            return Err("noncanonical tape syntax error ranges".into());
        }
        let embeddings = grammar.kinds.embeddings();
        let programs = reader
            .take(
                program_count
                    .checked_mul(6)
                    .ok_or("embedded program count overflow")?,
            )?
            .chunks_exact(6)
            .map(|entry| {
                let root = u32::from_le_bytes(entry[..4].try_into().expect("u32 width"));
                let dialect = embeddings
                    .get(usize::from(entry[4]))
                    .ok_or("an embedded program's dialect is not one the language embeds")?
                    .dialect;
                let apart = match entry[5] {
                    0 => false,
                    1 => true,
                    _ => return Err("noncanonical embedded program flag".to_owned()),
                };
                Ok(EmbeddedProgram {
                    root,
                    dialect,
                    apart,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        if programs.windows(2).any(|pair| pair[0].root >= pair[1].root) {
            return Err("noncanonical embedded programs".into());
        }
        let aliases_end = reader
            .offset
            .checked_add(alias_bytes)
            .filter(|&end| end <= bytes.len())
            .ok_or("truncated contextual grammar aliases")?;
        // Each alias occupies at least a flag byte and a u32 length, overrides address aliases
        // through a u16, and each override occupies six bytes after the alias table: counts the
        // bytes cannot back are rejected before a table is reserved for them.
        if alias_count > usize::from(u16::MAX) + 1
            || alias_count
                .checked_mul(5)
                .is_none_or(|minimum| minimum > alias_bytes)
        {
            return Err("invalid contextual grammar alias count".into());
        }
        if override_count
            .checked_mul(6)
            .is_none_or(|length| length != bytes.len() - aliases_end)
        {
            return Err("invalid contextual grammar override count".into());
        }
        #[cfg(test)]
        note_table_reservation(alias_count);
        let mut kind_aliases = Vec::with_capacity(alias_count);
        for _ in 0..alias_count {
            let named = match reader.u8()? {
                0 => false,
                1 => true,
                _ => return Err("noncanonical contextual grammar alias flag".into()),
            };
            let length = usize::try_from(reader.u32()?)
                .map_err(|_| "contextual grammar alias length overflow")?;
            let name = std::str::from_utf8(reader.take(length)?)
                .map_err(|_| "contextual grammar alias is not UTF-8")?
                .to_owned();
            kind_aliases.push((name, named));
        }
        if reader.offset != aliases_end {
            return Err("noncanonical contextual grammar alias length".into());
        }
        #[cfg(test)]
        note_table_reservation(override_count);
        let mut kind_overrides = Vec::with_capacity(override_count);
        for _ in 0..override_count {
            kind_overrides.push((
                reader.u32()?,
                u16::from_le_bytes(reader.take(2)?.try_into().expect("u16 width")),
            ));
        }
        if reader.offset != bytes.len() {
            return Err("trailing tape image bytes".into());
        }
        validate_image_tables(
            &rec,
            &checkpoints,
            &par_overflow,
            &size_overflow,
            &extra,
            &error,
            &missing,
        )?;
        Ok(Self {
            rec,
            spans,
            checkpoints,
            par_overflow,
            size_overflow,
            extra,
            error,
            missing,
            kind_aliases,
            kind_overrides,
            syntax_errors,
            programs,
            language,
            source_len,
            source_digest,
        })
    }

    /// Parses `source` as `grammar`'s dialect and encodes its tape: with tree-sitter (a PHP file's
    /// view, `php_view`, against `source`), or a Blade template with Rerail's port of Forte
    /// (`encode_template`). `None` where tree-sitter gave up (`budget` interrupted it).
    pub(super) fn parse(
        grammar: &Grammar,
        source: &str,
        budget: &impl Fn() -> bool,
    ) -> Option<Result<Self, String>> {
        if grammar.lang == GrammarDialect::Blade {
            return Some(Self::encode_template(grammar, source, budget));
        }
        let mut parser = tree_sitter::Parser::new();
        if let Err(error) = parser.set_language(&grammar.lang.get_ts_language()) {
            return Some(Err(error.to_string()));
        }
        let parsed = super::language::adapter::parse_input(grammar.lang, source);
        let bytes = parsed.as_bytes();
        let mut input = |offset: usize, _: tree_sitter::Point| &bytes[offset..];
        let mut progress = |_: &tree_sitter::ParseState| {
            if budget() {
                std::ops::ControlFlow::Break(())
            } else {
                std::ops::ControlFlow::Continue(())
            }
        };
        let tree = parser.parse_with_options(
            &mut input,
            None,
            Some(tree_sitter::ParseOptions::new().progress_callback(&mut progress)),
        )?;
        Some(Self::encode(tree.root_node(), grammar, source, budget))
    }

    /// The tape of a tree-sitter tree of `source`.
    pub(super) fn encode(
        root: TsNode<'_>,
        grammar: &Grammar,
        source: &str,
        budget: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        checked(budget)?;
        let Some(language) = grammar.kinds.grammar() else {
            return Err("a Blade template is encoded from its Forte tree".into());
        };
        if *root.language() != *language {
            return Err("parsed tree grammar mismatch".into());
        }
        if source.len() > u32::MAX as usize
            || root.start_byte() > root.end_byte()
            || root.end_byte() > source.len()
            || !source.is_char_boundary(root.start_byte())
            || !source.is_char_boundary(root.end_byte())
        {
            return Err("source exceeds tape range or does not match parsed root".into());
        }
        let mut nodes = Vec::<Pending>::new();
        encode_tree(
            &mut nodes,
            root,
            None,
            language,
            None,
            None::<&View>,
            budget,
        )?;
        // An embedding dialect's tape holds each program its embedders build from the tree as a
        // child of the root after the tree's own, numbered as `KindCatalog` numbers the program's
        // grammar: at most one per `via`. (Blade's compiled view is built from its Forte tree:
        // `encode_blade`.)
        let mut programs = Vec::new();
        let embeds = grammar.lang.embeds();
        for (index, &(via, _)) in embeds.iter().enumerate() {
            if embeds[..index].iter().any(|&(earlier, _)| earlier == via) {
                continue;
            }
            crate::code_rule_dsl::language::with_embedder!(via, E => {
                if let Some(program) = <E as Embedder>::build(root, source) {
                    let dialect = program.dialect();
                    let embedded = embeds
                        .contains(&(via, dialect))
                        .then(|| grammar.kinds.embedding(dialect))
                        .flatten()
                        .ok_or("an embedder built a program in a dialect its host does not embed")?;
                    encode_program(&mut nodes, &mut programs, &program, embedded, E::APART, budget)?;
                }
            });
        }
        Self::finish(nodes, Vec::new(), programs, grammar, source, budget)
    }

    /// The tape of a Blade template, parsed with Rerail's port of Forte (`encode_blade`).
    pub(super) fn encode_template(
        grammar: &Grammar,
        source: &str,
        budget: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        checked(budget)?;
        Self::encode_blade(&parse_tree(source.as_bytes()), grammar, source, budget)
    }

    /// The tape of a Blade template from its Forte tree (`tree::parse_tree`): the template's
    /// tree, its embedded PHP (`blade::view`, the `compiled-view` embedding) as the root's last
    /// child, and its syntax errors.
    pub(super) fn encode_blade(
        tree: &BladeTree,
        grammar: &Grammar,
        source: &str,
        budget: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        checked(budget)?;
        let Some(embedded) = grammar.kinds.embedding(GrammarDialect::Php) else {
            return Err("only a Blade template is encoded from a Forte tree".into());
        };
        if source.len() > u32::MAX as usize {
            return Err("source exceeds tape range".into());
        }
        let mut nodes = Vec::<Pending>::new();
        encode_blade_tree(&mut nodes, tree, grammar.kinds, budget)?;
        let reading = View::build(tree, source, budget)?;
        let mut programs = Vec::new();
        if let Some(view) = &reading.view {
            encode_program(
                &mut nodes,
                &mut programs,
                view,
                embedded,
                CompiledView::APART,
                budget,
            )?;
        }
        // Sorted (`Reading::syntax_errors`), as `from_bytes` requires; spans fit `u32`, as the
        // source does.
        let syntax_errors = reading
            .syntax_errors
            .iter()
            .map(|range| (range.start as u32, range.end as u32))
            .collect::<Vec<_>>();
        Self::finish(nodes, syntax_errors, programs, grammar, source, budget)
    }

    /// The image of `nodes`, in preorder with their parents set.
    fn finish(
        mut nodes: Vec<Pending>,
        syntax_errors: Vec<(u32, u32)>,
        programs: Vec<EmbeddedProgram>,
        grammar: &Grammar,
        source: &str,
        budget: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        checked(budget)?;
        for index in (1..nodes.len()).rev() {
            if index % 256 == 0 {
                checked(budget)?;
            }
            let parent = nodes[index].parent.ok_or("orphan node")? as usize;
            let size = nodes[index].size;
            nodes[parent].size = nodes[parent]
                .size
                .checked_add(size)
                .ok_or("subtree overflow")?;
        }
        let mut rec = Vec::with_capacity(
            nodes
                .len()
                .checked_mul(WIDTH)
                .ok_or("rec6 length overflow")?,
        );
        let mut spans = Vec::new();
        let mut checkpoints = Vec::new();
        let mut par_overflow = Vec::new();
        let mut size_overflow = Vec::new();
        let mut extra = Vec::new();
        let mut error = Vec::new();
        let mut missing = Vec::new();
        let mut kind_aliases = Vec::<(String, bool)>::new();
        let mut kind_overrides = Vec::new();
        let mut previous = 0i64;
        for (index, node) in nodes.iter().enumerate() {
            if index % 256 == 0 {
                checked(budget)?;
            }
            let index_u32 = index as u32;
            let pd = match node.parent {
                Some(parent) => index_u32
                    .checked_sub(parent)
                    .filter(|&distance| distance > 0)
                    .ok_or("invalid preorder parent")?,
                None if index == 0 => 1,
                None => return Err("orphan node".into()),
            };
            let pd16 = if pd >= u32::from(SENTINEL) {
                par_overflow.push((index_u32, pd));
                SENTINEL
            } else {
                pd as u16
            };
            let size16 = if node.size >= u32::from(SENTINEL) {
                size_overflow.push((index_u32, node.size));
                SENTINEL
            } else {
                node.size as u16
            };
            rec.extend_from_slice(&node.kf.to_le_bytes());
            rec.extend_from_slice(&pd16.to_le_bytes());
            rec.extend_from_slice(&size16.to_le_bytes());
            if node.extra {
                extra.push(index_u32);
            }
            if node.error {
                error.push(index_u32);
            }
            if node.missing {
                missing.push(index_u32);
            }
            if let Some(alias) = &node.kind_alias {
                let alias_id = kind_aliases
                    .iter()
                    .position(|candidate| candidate == alias)
                    .unwrap_or_else(|| {
                        kind_aliases.push(alias.clone());
                        kind_aliases.len() - 1
                    });
                kind_overrides.push((
                    index_u32,
                    u16::try_from(alias_id).map_err(|_| "too many contextual grammar aliases")?,
                ));
            }
            if index % CHECKPOINT_STRIDE == 0 {
                checkpoint(spans.len(), &mut checkpoints)?;
                checkpoint(node.start as usize, &mut checkpoints)?;
            }
            let delta = i32::try_from(i64::from(node.start) - previous)
                .map_err(|_| "span delta overflow")?;
            varint(((delta << 1) ^ (delta >> 31)) as u32, &mut spans);
            varint(node.length, &mut spans);
            previous = i64::from(node.start);
        }
        checked(budget)?;
        Ok(Self {
            rec,
            spans,
            checkpoints,
            par_overflow,
            size_overflow,
            extra,
            error,
            missing,
            kind_aliases,
            kind_overrides,
            syntax_errors,
            programs,
            language: grammar.lang,
            source_len: source.len(),
            source_digest: source_digest(source, budget)?,
        })
    }

    pub(super) fn node_count(&self) -> usize {
        self.rec.len() / WIDTH
    }

    pub(super) fn estimated_bytes(&self) -> usize {
        self.rec.capacity()
            + self.spans.capacity()
            + self.checkpoints.capacity()
            + (self.par_overflow.capacity() + self.size_overflow.capacity())
                * std::mem::size_of::<(u32, u32)>()
            + (self.extra.capacity() + self.error.capacity() + self.missing.capacity())
                * std::mem::size_of::<u32>()
            + self.kind_aliases.capacity() * std::mem::size_of::<(String, bool)>()
            + self
                .kind_aliases
                .iter()
                .map(|(name, _)| name.capacity())
                .sum::<usize>()
            + self.kind_overrides.capacity() * std::mem::size_of::<(u32, u16)>()
            + self.syntax_errors.capacity() * std::mem::size_of::<(u32, u32)>()
            + self.programs.capacity() * std::mem::size_of::<EmbeddedProgram>()
            + std::mem::size_of::<Self>()
    }
}

#[derive(Clone, Copy)]
struct Rec {
    kind: u16,
    field: u16,
    start: u32,
    end: u32,
    size: u32,
    parent: u32,
    first: u32,
    next: u32,
    prev: u32,
    children: u32,
    extra: bool,
    error: bool,
    missing: bool,
}

#[derive(Clone)]
pub(super) struct TapeDoc {
    source: Arc<String>,
    grammar: Arc<Grammar>,
    data: Arc<Vec<Rec>>,
    lines: Arc<LineTable>,
    points: Arc<Points>,
    kind_aliases: Arc<Vec<(String, bool)>>,
    kind_overrides: Arc<Vec<(u32, u16)>>,
    syntax_errors: Arc<Vec<(u32, u32)>>,
    programs: Arc<Vec<EmbeddedProgram>>,
}

/// A node's row and column, as tree-sitter's `TSPoint` counts them: only `\n` ends a row. The
/// evaluator's line table follows the linter a port mirrors (a lone `\r` ends a Ruff line), so
/// positions keep their own table, built on first use: violations place spans with the line table,
/// and nothing but parity with the native tree reads a tape node's position.
struct Points {
    source: Arc<String>,
    rows: std::sync::OnceLock<LineTable>,
}

impl Points {
    fn point(&self, byte: u32) -> Position {
        let byte = byte as usize;
        let (row, column) = self
            .rows
            .get_or_init(|| LineTable::tree_sitter(&self.source))
            .point(byte);
        Position::new(row, column, byte)
    }
}

impl TapeDoc {
    pub(super) fn restore_with_lines(
        source: Arc<String>,
        grammar: Arc<Grammar>,
        image: &TapeImage,
        lines: Arc<LineTable>,
        budget: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        checked(budget)?;
        if image.language != grammar.lang
            || image.source_len != source.len()
            || image.source_digest != source_digest(&source, budget)?
        {
            return Err("tape grammar or source revision mismatch".into());
        }
        let count = image.node_count();
        let expected_checkpoints = count
            .div_ceil(CHECKPOINT_STRIDE)
            .checked_mul(WIDTH)
            .ok_or("checkpoint length overflow")?;
        if count == 0
            || !image.rec.len().is_multiple_of(WIDTH)
            || count > u32::MAX as usize
            || image.checkpoints.len() != expected_checkpoints
        {
            return Err("malformed rec6 tape".into());
        }
        for list in [&image.par_overflow, &image.size_overflow] {
            let mut previous = None;
            for (entry, &(index, value)) in list.iter().enumerate() {
                if entry % 256 == 0 {
                    checked(budget)?;
                }
                if previous.is_some_and(|last| last >= index)
                    || index as usize >= count
                    || value < u32::from(SENTINEL)
                {
                    return Err("invalid rec6 overflow entries".into());
                }
                previous = Some(index);
            }
        }
        for list in [&image.extra, &image.error, &image.missing] {
            let mut previous = None;
            for (entry, &index) in list.iter().enumerate() {
                if entry % 256 == 0 {
                    checked(budget)?;
                }
                if previous.is_some_and(|last| last >= index) || index as usize >= count {
                    return Err("invalid extra, error or missing node entries".into());
                }
                previous = Some(index);
            }
        }
        let mut previous = None;
        for &(index, alias) in &image.kind_overrides {
            if previous.is_some_and(|last| last >= index)
                || index as usize >= count
                || alias as usize >= image.kind_aliases.len()
                || image.kind_aliases[alias as usize].0.is_empty()
            {
                return Err("invalid contextual grammar alias entry".into());
            }
            previous = Some(index);
        }
        let mut nodes: Vec<Rec> = Vec::with_capacity(count);
        let mut at = 0usize;
        let mut previous = 0i64;
        let mut parent_stack: Vec<usize> = Vec::new();
        let mut unwound = 0usize;
        for index in 0..count {
            if index % 256 == 0 {
                checked(budget)?;
            }
            let offset = index * WIDTH;
            let kf = u16::from_le_bytes([image.rec[offset], image.rec[offset + 1]]);
            let code = kf & 1023;
            let field = kf >> 10;
            let error = image.error.binary_search(&(index as u32)).is_ok();
            let missing = image.missing.binary_search(&(index as u32)).is_ok();
            let kind = if error { ERROR_SYMBOL } else { code };
            if (error && (code != ERROR_CODE || missing))
                || (!error
                    && (usize::from(code) >= grammar.kinds.kind_count()
                        || !grammar.kinds.kind_is_visible(code)))
                || usize::from(field) > grammar.kinds.field_count()
                || (field != 0 && grammar.kinds.field_name(field).is_none())
            {
                return Err("unknown rec6 grammar code".into());
            }
            let pd_raw = u16::from_le_bytes([image.rec[offset + 2], image.rec[offset + 3]]);
            let size_raw = u16::from_le_bytes([image.rec[offset + 4], image.rec[offset + 5]]);
            let pd = overflow_value(&image.par_overflow, index as u32, pd_raw)?;
            let size = overflow_value(&image.size_overflow, index as u32, size_raw)?;
            if size == 0
                || index
                    .checked_add(size as usize)
                    .is_none_or(|end| end > count)
            {
                return Err("invalid subtree boundary".into());
            }
            if index % CHECKPOINT_STRIDE == 0 {
                let cp = &image.checkpoints[(index / CHECKPOINT_STRIDE) * WIDTH..][..WIDTH];
                let expected_offset = u32::from_le_bytes([cp[0], cp[1], cp[2], 0]) as usize;
                if expected_offset != at {
                    return Err("invalid span checkpoint offset".into());
                }
            }
            let encoded_delta = read_varint(&image.spans, &mut at)?;
            let delta = ((encoded_delta >> 1) as i32) ^ -((encoded_delta & 1) as i32);
            let start_i64 = previous
                .checked_add(i64::from(delta))
                .ok_or("span delta overflow")?;
            let start = u32::try_from(start_i64).map_err(|_| "span start overflow")?;
            let length = read_varint(&image.spans, &mut at)?;
            let end = start.checked_add(length).ok_or("span end overflow")?;
            if (missing && (length != 0 || size != 1))
                || end as usize > source.len()
                || !source.is_char_boundary(start as usize)
                || !source.is_char_boundary(end as usize)
            {
                return Err("span outside source or UTF-8 boundary".into());
            }
            if index % CHECKPOINT_STRIDE == 0 {
                let cp = &image.checkpoints[(index / CHECKPOINT_STRIDE) * WIDTH..][..WIDTH];
                let expected_start = u32::from_le_bytes([cp[3], cp[4], cp[5], 0]);
                if expected_start != start {
                    return Err("invalid span checkpoint start".into());
                }
            }
            previous = i64::from(start);
            while parent_stack
                .last()
                .is_some_and(|&parent| index >= parent + nodes[parent].size as usize)
            {
                if unwound.is_multiple_of(256) {
                    checked(budget)?;
                }
                unwound += 1;
                parent_stack.pop();
            }
            let parent = if index == 0 {
                if pd != 1 || field != 0 || size as usize != count {
                    return Err("invalid tape root".into());
                }
                NONE
            } else {
                let enclosing = *parent_stack.last().ok_or("orphan tape node")?;
                if pd == 0
                    || pd as usize > index
                    || index - pd as usize != enclosing
                    || index
                        .checked_add(size as usize)
                        .is_none_or(|end| end > enclosing + nodes[enclosing].size as usize)
                    || start < nodes[enclosing].start
                    || end > nodes[enclosing].end
                {
                    return Err("invalid tape parent or span".into());
                }
                enclosing as u32
            };
            let extra = image.extra.binary_search(&(index as u32)).is_ok();
            nodes.push(Rec {
                kind,
                field,
                start,
                end,
                size,
                parent,
                first: NONE,
                next: NONE,
                prev: NONE,
                children: 0,
                extra,
                error,
                missing,
            });
            parent_stack.push(index);
        }
        if at != image.spans.len() {
            return Err("trailing span bytes".into());
        }
        for (list, word) in [(&image.par_overflow, 2), (&image.size_overflow, 4)] {
            for (entry, &(index, _)) in list.iter().enumerate() {
                if entry % 256 == 0 {
                    checked(budget)?;
                }
                let offset = index as usize * WIDTH + word;
                if u16::from_le_bytes([image.rec[offset], image.rec[offset + 1]]) != SENTINEL {
                    return Err("unused overflow entry".into());
                }
            }
        }
        let mut last = vec![NONE; count];
        for index in 1..count {
            if index % 256 == 0 {
                checked(budget)?;
            }
            let parent = nodes[index].parent as usize;
            if nodes[parent].first == NONE {
                nodes[parent].first = index as u32;
            }
            let prev = last[parent];
            // Siblings sit in byte order, so their ends never decrease (`walk_anchor` steps
            // from a node on that). A Blade tape's embedded program overlaps the template's
            // nodes (`blade::view`): its siblings may not, and `walk_anchor` scans them.
            if prev != NONE
                && !grammar.kinds.has_embedded()
                && nodes[prev as usize].end > nodes[index].end
            {
                return Err("tape siblings out of byte order".into());
            }
            nodes[index].prev = prev;
            if prev != NONE {
                nodes[prev as usize].next = index as u32;
            }
            last[parent] = index as u32;
            nodes[parent].children += 1;
        }
        checked(budget)?;
        if image.programs.iter().any(|program| {
            nodes
                .get(program.root as usize)
                .is_none_or(|node| node.parent != 0)
        }) {
            return Err("an embedded program's root is not a child of the tape's root".into());
        }
        Ok(Self {
            points: Arc::new(Points {
                source: Arc::clone(&source),
                rows: std::sync::OnceLock::new(),
            }),
            source,
            grammar,
            data: Arc::new(nodes),
            lines,
            kind_aliases: Arc::new(image.kind_aliases.clone()),
            kind_overrides: Arc::new(image.kind_overrides.clone()),
            syntax_errors: Arc::new(image.syntax_errors.clone()),
            programs: Arc::new(image.programs.clone()),
        })
    }

    /// Ranges of ERROR nodes and MISSING tokens that no ERROR node contains, as
    /// `SyntaxErrors::collect` finds them on the native tree, then the tape's own (a Blade
    /// template's lexer errors, zero-width at their offset, or the whole template Forte did not
    /// parse).
    pub(super) fn syntax_error_ranges(&self) -> Vec<Range<usize>> {
        let mut ranges = Vec::new();
        let mut skip_until = 0usize;
        for (index, rec) in self.data.iter().enumerate() {
            if index < skip_until || !(rec.error || rec.missing) {
                continue;
            }
            ranges.push(rec.start as usize..rec.end as usize);
            skip_until = index + rec.size as usize;
        }
        ranges.extend(
            self.syntax_errors
                .iter()
                .map(|&(start, end)| start as usize..end as usize),
        );
        ranges
    }

    /// The programs the tape embeds (`embed`), in tape order.
    pub(crate) fn programs(&self) -> &[EmbeddedProgram] {
        &self.programs
    }

    /// Each embedded program's root kind code, and whether its subtree has an ERROR node or a
    /// MISSING token of its own.
    pub(super) fn program_syntax(&self) -> impl Iterator<Item = (u16, bool)> + '_ {
        self.programs.iter().map(|program| {
            let root = program.root as usize;
            let subtree = &self.data[root..root + self.data[root].size as usize];
            (
                self.data[root].kind,
                subtree.iter().any(|rec| rec.error || rec.missing),
            )
        })
    }

    #[cfg(test)]
    pub(super) fn restore(
        source: Arc<String>,
        grammar: Arc<Grammar>,
        image: &TapeImage,
        budget: &impl Fn() -> bool,
    ) -> Result<Self, String> {
        let lines = Arc::new(LineTable::new(&source, Terminators::of(grammar.lang)));
        Self::restore_with_lines(source, grammar, image, lines, budget)
    }

    /// The source's line table, shared with the evaluator that restored this document.
    pub(super) fn line_table(&self) -> &LineTable {
        &self.lines
    }

    pub(super) fn transducer_len(&self) -> usize {
        self.data.len()
    }

    pub(super) fn transducer_kind(&self, node: u32) -> u16 {
        self.data[node as usize].kind
    }

    fn kind_alias(&self, node: u32) -> Option<&(String, bool)> {
        self.kind_overrides
            .binary_search_by_key(&node, |&(index, _)| index)
            .ok()
            .map(|slot| &self.kind_aliases[self.kind_overrides[slot].1 as usize])
    }

    fn node_is_named(&self, node: u32) -> bool {
        self.kind_alias(node).map_or_else(
            || self.grammar.kind_is_named(self.data[node as usize].kind),
            |alias| alias.1,
        )
    }

    pub(super) fn transducer_field(&self, node: u32) -> u16 {
        self.data[node as usize].field
    }

    pub(super) fn transducer_flags(&self, node: u32) -> u8 {
        let rec = &self.data[node as usize];
        u8::from(self.node_is_named(node))
            | (u8::from(rec.extra) << 1)
            | (u8::from(rec.missing) << 2)
            | (u8::from(rec.error) << 3)
    }

    pub(super) fn transducer_span(&self, node: u32) -> Range<usize> {
        let rec = &self.data[node as usize];
        rec.start as usize..rec.end as usize
    }

    /// Node `node`'s parent, first child, and structural next and previous siblings, as its
    /// record holds them (`u32::MAX` for none).
    #[inline]
    pub(super) fn transducer_links(&self, node: u32) -> [u32; 4] {
        let rec = &self.data[node as usize];
        [rec.parent, rec.first, rec.next, rec.prev]
    }

    /// Whether node `node` is extra, an ERROR node, and a MISSING token.
    #[inline]
    pub(super) fn transducer_marks(&self, node: u32) -> [bool; 3] {
        let rec = &self.data[node as usize];
        [rec.extra, rec.error, rec.missing]
    }

    #[cfg(test)]
    pub(super) fn transducer_parent(&self, node: u32) -> Option<u32> {
        let parent = self.data[node as usize].parent;
        (parent != NONE).then_some(parent)
    }

    pub(super) fn transducer_children(&self, node: u32) -> impl Iterator<Item = u32> + '_ {
        let mut next = self.data[node as usize].first;
        std::iter::from_fn(move || {
            if next == NONE {
                return None;
            }
            let current = next;
            next = self.data[current as usize].next;
            Some(current)
        })
    }

    pub(super) fn transducer_descendants(&self, node: u32) -> Range<u32> {
        let rec = &self.data[node as usize];
        node + 1..node + rec.size
    }

    #[cfg(test)]
    pub(super) fn transducer_next_sibling(&self, node: u32, named: bool) -> Option<u32> {
        let mut next = self.data[node as usize].next;
        while next != NONE {
            if !named || self.node_is_named(next) {
                return Some(next);
            }
            next = self.data[next as usize].next;
        }
        None
    }

    /// The whole source, for the Metal backend's concatenated buffer.
    #[cfg(all(test, feature = "bench-backends", target_os = "macos"))]
    pub(super) fn transducer_source(&self) -> &[u8] {
        self.source.as_bytes()
    }

    /// A node's source slice. Restoring the tape validates every span as a UTF-8 boundary.
    pub(super) fn transducer_text_str(&self, node: u32) -> &str {
        &self.source[self.transducer_span(node)]
    }

    /// The node at a preorder ordinal: `root.adopt(doc.node(ordinal))` is the node ast-grep
    /// matches on.
    pub(super) fn node(&self, ordinal: u32) -> TapeNode<'_> {
        self.root_node().at(ordinal)
    }

    #[cfg(feature = "code-rule-profile")]
    pub(super) fn navigation_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.data.capacity() * std::mem::size_of::<Rec>()
            + self.kind_aliases.capacity() * std::mem::size_of::<(String, bool)>()
            + self
                .kind_aliases
                .iter()
                .map(|(name, _)| name.capacity())
                .sum::<usize>()
            + self.kind_overrides.capacity() * std::mem::size_of::<(u32, u16)>()
    }
}

#[derive(Clone, Copy)]
pub(super) struct TapeNode<'a> {
    nodes: &'a [Rec],
    grammar: &'a Grammar,
    points: &'a Points,
    kind_aliases: &'a [(String, bool)],
    kind_overrides: &'a [(u32, u16)],
    index: u32,
}

impl<'a> TapeNode<'a> {
    fn rec(&self) -> &Rec {
        &self.nodes[self.index as usize]
    }
    fn at(&self, index: u32) -> Self {
        Self { index, ..*self }
    }
    fn kind_alias(&self) -> Option<&'a (String, bool)> {
        self.kind_overrides
            .binary_search_by_key(&self.index, |&(index, _)| index)
            .ok()
            .map(|slot| &self.kind_aliases[self.kind_overrides[slot].1 as usize])
    }
    /// The structurally adjacent sibling, for traversal; `SgNode::next` follows Tree-sitter.
    pub(super) fn next_sibling(&self) -> Option<Self> {
        (self.rec().next != NONE).then(|| self.at(self.rec().next))
    }
    fn prev_sibling(&self) -> Option<Self> {
        (self.rec().prev != NONE).then(|| self.at(self.rec().prev))
    }
    /// Where ast-grep's `next_all`/`prev_all` start: a cursor on the parent (the root for the
    /// root) moved to the first child that ends after this node's start byte.
    ///
    /// Sibling ends never decrease (restore checks it), so the children that end after the start
    /// are a suffix of the parent's. The walk steps from this node (the root's first child for
    /// the root) to that suffix's first child: back over earlier siblings that also end after the
    /// start, or forward past zero-width ones that end at it. For a node with width, whose earlier
    /// siblings end at or before its start, that is one or two steps, not a scan from the
    /// parent's first child. A Blade tape's siblings may overlap (its embedded program sits among
    /// the template's nodes), so its walk scans the parent's children from the first.
    fn walk_anchor(&self) -> Option<Self> {
        if self.grammar.kinds.has_embedded() {
            let container = self.parent().unwrap_or(*self);
            let start = self.rec().start;
            return container.children().find(|child| child.rec().end > start);
        }
        let start = self.rec().start;
        let ends_after = |node: &Self| node.rec().end > start;
        let mut anchor = match self.parent() {
            Some(_) => *self,
            None => self.children().next()?,
        };
        if ends_after(&anchor) {
            while let Some(prev) = anchor.prev_sibling().filter(ends_after) {
                anchor = prev;
            }
            return Some(anchor);
        }
        loop {
            anchor = anchor.next_sibling()?;
            if ends_after(&anchor) {
                return Some(anchor);
            }
        }
    }
}

struct Children<'a> {
    node: TapeNode<'a>,
    next: u32,
    remaining: usize,
}
impl<'a> Iterator for Children<'a> {
    type Item = TapeNode<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.next == NONE {
            return None;
        }
        let index = self.next;
        self.next = self.node.nodes[index as usize].next;
        self.remaining -= 1;
        Some(self.node.at(index))
    }
    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}
impl ExactSizeIterator for Children<'_> {}

impl<'a> SgNode<'a> for TapeNode<'a> {
    fn parent(&self) -> Option<Self> {
        (self.rec().parent != NONE).then(|| self.at(self.rec().parent))
    }
    fn ancestors(&self, _root: Self) -> impl Iterator<Item = Self> {
        let mut ancestor = self.parent();
        std::iter::from_fn(move || {
            let next = ancestor?;
            ancestor = next.parent();
            Some(next)
        })
    }
    fn children(&self) -> impl ExactSizeIterator<Item = Self> {
        Children {
            node: *self,
            next: self.rec().first,
            remaining: self.rec().children as usize,
        }
    }
    fn dfs(&self) -> impl Iterator<Item = Self> {
        let start = self.index;
        let end = start + self.rec().size;
        (start..end).map(move |index| self.at(index))
    }
    fn child(&self, nth: usize) -> Option<Self> {
        self.children().nth(nth)
    }
    // Tree-sitter's next sibling is byte-positioned: it skips later siblings that end at or
    // before this node's end, i.e. zero-width siblings sitting at that end.
    fn next(&self) -> Option<Self> {
        let end = self.rec().end;
        let mut next = self.next_sibling();
        while let Some(node) = next {
            if node.rec().end > end {
                return Some(node);
            }
            next = node.next_sibling();
        }
        None
    }
    fn prev(&self) -> Option<Self> {
        self.prev_sibling()
    }
    fn next_all(&self) -> impl Iterator<Item = Self> {
        let mut next = self.walk_anchor().and_then(|anchor| anchor.next_sibling());
        std::iter::from_fn(move || {
            let node = next?;
            next = node.next_sibling();
            Some(node)
        })
    }
    fn prev_all(&self) -> impl Iterator<Item = Self> {
        let mut prev = self.walk_anchor().and_then(|anchor| anchor.prev_sibling());
        std::iter::from_fn(move || {
            let node = prev?;
            prev = node.prev_sibling();
            Some(node)
        })
    }
    fn kind(&self) -> Cow<'_, str> {
        Cow::Borrowed(self.kind_alias().map_or_else(
            || {
                self.grammar
                    .kind_name(self.rec().kind)
                    .expect("validated grammar symbol")
            },
            |alias| alias.0.as_str(),
        ))
    }
    fn kind_id(&self) -> u16 {
        self.rec().kind
    }
    fn node_id(&self) -> usize {
        self.index as usize
    }
    fn range(&self) -> Range<usize> {
        self.rec().start as usize..self.rec().end as usize
    }
    fn start_pos(&self) -> Position {
        self.points.point(self.rec().start)
    }
    fn end_pos(&self) -> Position {
        self.points.point(self.rec().end)
    }
    fn is_named(&self) -> bool {
        self.kind_alias().map_or_else(
            || self.grammar.kind_is_named(self.rec().kind),
            |alias| alias.1,
        )
    }
    fn is_named_leaf(&self) -> bool {
        self.children().all(|child| !child.is_named())
    }
    fn is_extra(&self) -> bool {
        self.rec().extra
    }
    fn is_error(&self) -> bool {
        self.rec().error
    }
    fn is_missing(&self) -> bool {
        self.rec().missing
    }
    fn field(&self, name: &str) -> Option<Self> {
        let field = self.grammar.kinds.field_id(name)?;
        self.child_by_field_id(field)
    }
    fn field_children(&self, field_id: Option<u16>) -> impl Iterator<Item = Self> {
        self.children().filter(move |child| {
            field_id.is_some_and(|field| field != 0 && child.rec().field == field)
        })
    }
    fn child_by_field_id(&self, field_id: u16) -> Option<Self> {
        if field_id == 0 {
            None
        } else {
            self.children().find(|child| child.rec().field == field_id)
        }
    }
}

impl Doc for TapeDoc {
    type Source = String;
    type Lang = GrammarDialect;
    type Node<'a> = TapeNode<'a>;
    fn get_lang(&self) -> &GrammarDialect {
        &self.grammar.lang
    }
    fn get_source(&self) -> &String {
        &self.source
    }
    fn do_edit(&mut self, _edit: &Edit<String>) -> Result<(), String> {
        Err("replace the source-bound tape after editing".into())
    }
    fn root_node(&self) -> Self::Node<'_> {
        TapeNode {
            nodes: &self.data,
            grammar: &self.grammar,
            points: &self.points,
            kind_aliases: &self.kind_aliases,
            kind_overrides: &self.kind_overrides,
            index: 0,
        }
    }
    fn get_node_text<'a>(&'a self, node: &Self::Node<'a>) -> Cow<'a, str> {
        Cow::Borrowed(&self.source[node.range()])
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod corpus_parity;
