//! Tape v8 spike: v7 baseline (copied from rerail native/src/code_rule_dsl/tape.rs and tape/wire.rs)
//! against regular column variants. Modes:
//!   measure <corpus> <lang> <dir>...   storage, encode, Rust decode, invariant statistics (one JSON line)
//!   sql <lang> <db> <maxfiles> <dir>... pure-SQL decoding (ast-index-spike technique) on bundled SQLite
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tree_sitter::{Language, Node as TsNode, Parser};

mod sql;

// ---- copied from tape.rs (v7) ----
pub const KIND_LIMIT: usize = 1024;
const FIELD_LIMIT: usize = 64;
pub const SENTINEL: u16 = u16::MAX;
pub const CHECKPOINT_STRIDE: usize = 16;
pub const WIDTH: usize = 6;
pub const ERROR_CODE: u16 = 0;
const ERROR_SYMBOL: u16 = u16::MAX;
const HEADER_V7: usize = 56 + 11 * 8;
const CHECKPOINT_LIMIT: usize = 0x00ff_ffff;
/// v8 header: magic 4, lang 1, widths 1, version 2, node count 4, source length 4.
const HEADER_V8: usize = 16;

#[derive(Clone)]
pub struct Pending {
    pub kf: u16,
    pub parent: Option<u32>,
    pub start: u32,
    pub length: u32,
    pub size: u32,
    pub extra: bool,
    pub error: bool,
    pub missing: bool,
    pub named: bool,
    pub kind_alias: Option<(String, bool)>,
}

fn encode_tree(nodes: &mut Vec<Pending>, root: TsNode<'_>, language: &Language) -> Result<(), String> {
    let mut catalog: Vec<Option<(Option<&str>, bool)>> = vec![None; language.node_kind_count()];
    let mut cursor = root.walk();
    let mut parent = None;
    let mut above: Vec<Option<u32>> = Vec::new();
    loop {
        let node = cursor.node();
        let field = cursor.field_id().map_or(0, |id| id.get());
        let index = encode_node(nodes, node, parent, field, language, &mut catalog)?;
        if cursor.goto_first_child() {
            above.push(parent);
            parent = Some(index);
            continue;
        }
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

fn encode_node<'l>(
    nodes: &mut Vec<Pending>,
    node: TsNode<'_>,
    parent: Option<u32>,
    field: u16,
    language: &'l Language,
    catalog: &mut [Option<(Option<&'l str>, bool)>],
) -> Result<u32, String> {
    let index = u32::try_from(nodes.len()).map_err(|_| "rec6 node count overflow")?;
    let kind = usize::from(node.kind_id());
    let error = node.is_error();
    if usize::from(field) >= FIELD_LIMIT {
        return Err(format!("grammar field id {field} exceeds tape limit {FIELD_LIMIT}"));
    }
    let code = node.kind_id();
    if error {
        if code != ERROR_SYMBOL || node.is_missing() {
            return Err("unsupported error or missing grammar symbol".into());
        }
    } else if kind >= language.node_kind_count() {
        return Err(format!("grammar symbol {kind} is not in the semantic catalog"));
    } else if usize::from(code) >= KIND_LIMIT {
        return Err(format!("grammar symbol {code} exceeds tape limit {KIND_LIMIT}"));
    } else if code == ERROR_CODE {
        return Err("grammar symbol conflicts with the reserved tape error code".into());
    } else if node.is_missing() && !node.byte_range().is_empty() {
        return Err("missing grammar symbol has a non-empty source span".into());
    }
    let span = node.byte_range();
    let start = u32::try_from(span.start).map_err(|_| "node span overflow")?;
    let end = u32::try_from(span.end).map_err(|_| "node span overflow")?;
    let length = end.checked_sub(start).ok_or("inverted node span")?;
    let kind_alias = if error {
        None
    } else {
        let (name, named) = *catalog[kind].get_or_insert_with(|| {
            (language.node_kind_for_id(node.kind_id()), language.node_kind_is_named(node.kind_id()))
        });
        let same_name = name.is_some_and(|name| name == node.kind());
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
        named: node.is_named(),
        kind_alias,
    });
    Ok(index)
}

/// Subtree sizes (the first half of v7 `TapeImage::finish`).
fn sizes(nodes: &mut [Pending]) -> Result<(), String> {
    for index in (1..nodes.len()).rev() {
        let parent = nodes[index].parent.ok_or("orphan node")? as usize;
        let size = nodes[index].size;
        nodes[parent].size = nodes[parent].size.checked_add(size).ok_or("subtree overflow")?;
    }
    Ok(())
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

fn checkpoint(value: usize, bytes: &mut Vec<u8>) -> Result<(), String> {
    if value > CHECKPOINT_LIMIT {
        return Err("rec6 checkpoint overflow".into());
    }
    bytes.extend_from_slice(&(value as u32).to_le_bytes()[..3]);
    Ok(())
}

pub fn read_varint(bytes: &[u8], at: &mut usize) -> u32 {
    let mut result = 0u32;
    let mut shift = 0;
    loop {
        let byte = bytes[*at];
        *at += 1;
        result |= u32::from(byte & 127) << shift;
        if byte & 128 == 0 {
            return result;
        }
        shift += 7;
    }
}

#[derive(Default)]
pub struct V7 {
    pub rec: Vec<u8>,
    pub spans: Vec<u8>,
    pub checkpoints: Vec<u8>,
    pub par_overflow: Vec<(u32, u32)>,
    pub size_overflow: Vec<(u32, u32)>,
    pub extra: Vec<u32>,
    pub error: Vec<u32>,
    pub missing: Vec<u32>,
    pub kind_aliases: Vec<(String, bool)>,
    pub kind_overrides: Vec<(u32, u16)>,
}

impl V7 {
    /// The second half of v7 `TapeImage::finish`.
    pub fn encode(nodes: &[Pending]) -> Result<Self, String> {
        let mut t = V7::default();
        t.rec.reserve(nodes.len() * WIDTH);
        let mut previous = 0i64;
        for (index, node) in nodes.iter().enumerate() {
            let index_u32 = index as u32;
            let pd = match node.parent {
                Some(parent) => index_u32.checked_sub(parent).filter(|&d| d > 0).ok_or("invalid preorder parent")?,
                None if index == 0 => 1,
                None => return Err("orphan node".into()),
            };
            let pd16 = if pd >= u32::from(SENTINEL) {
                t.par_overflow.push((index_u32, pd));
                SENTINEL
            } else {
                pd as u16
            };
            let size16 = if node.size >= u32::from(SENTINEL) {
                t.size_overflow.push((index_u32, node.size));
                SENTINEL
            } else {
                node.size as u16
            };
            t.rec.extend_from_slice(&node.kf.to_le_bytes());
            t.rec.extend_from_slice(&pd16.to_le_bytes());
            t.rec.extend_from_slice(&size16.to_le_bytes());
            if node.extra {
                t.extra.push(index_u32);
            }
            if node.error {
                t.error.push(index_u32);
            }
            if node.missing {
                t.missing.push(index_u32);
            }
            if let Some(alias) = &node.kind_alias {
                let alias_id = t.kind_aliases.iter().position(|c| c == alias).unwrap_or_else(|| {
                    t.kind_aliases.push(alias.clone());
                    t.kind_aliases.len() - 1
                });
                t.kind_overrides.push((index_u32, alias_id as u16));
            }
            if index % CHECKPOINT_STRIDE == 0 {
                checkpoint(t.spans.len(), &mut t.checkpoints)?;
                checkpoint(node.start as usize, &mut t.checkpoints)?;
            }
            let delta = i32::try_from(i64::from(node.start) - previous).map_err(|_| "span delta overflow")?;
            varint(((delta << 1) ^ (delta >> 31)) as u32, &mut t.spans);
            varint(node.length, &mut t.spans);
            previous = i64::from(node.start);
        }
        Ok(t)
    }

    /// `TapeImage::to_bytes` layout (header, tables in order).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(HEADER_V7 + self.rec.len() + self.spans.len() + self.checkpoints.len());
        b.extend_from_slice(&[0u8; HEADER_V7]);
        b.extend_from_slice(&self.rec);
        b.extend_from_slice(&self.spans);
        b.extend_from_slice(&self.checkpoints);
        for &(i, v) in self.par_overflow.iter().chain(&self.size_overflow) {
            b.extend_from_slice(&i.to_le_bytes());
            b.extend_from_slice(&v.to_le_bytes());
        }
        for list in [&self.extra, &self.error, &self.missing] {
            for &i in list {
                b.extend_from_slice(&i.to_le_bytes());
            }
        }
        for (name, named) in &self.kind_aliases {
            b.push(u8::from(*named));
            b.extend_from_slice(&(name.len() as u32).to_le_bytes());
            b.extend_from_slice(name.as_bytes());
        }
        for &(i, a) in &self.kind_overrides {
            b.extend_from_slice(&i.to_le_bytes());
            b.extend_from_slice(&a.to_le_bytes());
        }
        b
    }

    fn u16_at(&self, i: usize, word: usize) -> u16 {
        let o = i * WIDTH + word;
        u16::from_le_bytes([self.rec[o], self.rec[o + 1]])
    }
    #[inline]
    pub fn kind(&self, i: usize) -> u16 {
        self.u16_at(i, 0) & 1023
    }
    #[inline]
    pub fn parent(&self, i: usize) -> i64 {
        let d = self.u16_at(i, 2);
        let d = if d == SENTINEL { overflow(&self.par_overflow, i as u32) } else { u32::from(d) };
        i as i64 - i64::from(d)
    }
    #[inline]
    pub fn size(&self, i: usize) -> u32 {
        let s = self.u16_at(i, 4);
        if s == SENTINEL { overflow(&self.size_overflow, i as u32) } else { u32::from(s) }
    }
    /// `wire::index_span`.
    #[inline]
    pub fn span(&self, pre: usize) -> (u32, u32) {
        let block = pre / CHECKPOINT_STRIDE;
        let cp = &self.checkpoints[block * WIDTH..block * WIDTH + WIDTH];
        let mut offset = u32::from_le_bytes([cp[0], cp[1], cp[2], 0]) as usize;
        let mut start = i64::from(u32::from_le_bytes([cp[3], cp[4], cp[5], 0]));
        let mut length = 0;
        for i in block * CHECKPOINT_STRIDE..=pre {
            let encoded = read_varint(&self.spans, &mut offset);
            let delta = ((encoded >> 1) as i32) ^ -((encoded & 1) as i32);
            length = read_varint(&self.spans, &mut offset);
            if i != block * CHECKPOINT_STRIDE {
                start += i64::from(delta);
            }
        }
        (start as u32, start as u32 + length)
    }
    /// The named flag as the restored document reads it: the catalog's unless overridden by an alias.
    #[inline]
    pub fn named(&self, i: usize, catalog_named: &[bool]) -> bool {
        if self.kind(i) == ERROR_CODE {
            return true;
        }
        match self.kind_overrides.binary_search_by_key(&(i as u32), |&(at, _)| at) {
            Ok(slot) => self.kind_aliases[self.kind_overrides[slot].1 as usize].1,
            Err(_) => catalog_named.get(self.kind(i) as usize).copied().unwrap_or(false),
        }
    }
}

fn overflow(entries: &[(u32, u32)], index: u32) -> u32 {
    entries[entries.binary_search_by_key(&index, |&(at, _)| at).unwrap()].1
}

/// Column widths in bytes, chosen per document.
#[derive(Clone, Copy)]
pub struct Widths {
    pub w: usize, // parent delta, size
    pub s: usize, // start, length
}

pub fn widths(n: usize, source_len: usize) -> Widths {
    Widths { w: if n < 0xffff { 2 } else { 4 }, s: if source_len < 0x1_0000 { 2 } else { 4 } }
}

fn put(out: &mut Vec<u8>, value: u32, width: usize) {
    out.extend_from_slice(&value.to_le_bytes()[..width]);
}

#[inline]
pub fn get(col: &[u8], i: usize, width: usize) -> u32 {
    if width == 2 {
        u16::from_le_bytes([col[2 * i], col[2 * i + 1]]) as u32
    } else {
        u32::from_le_bytes([col[4 * i], col[4 * i + 1], col[4 * i + 2], col[4 * i + 3]])
    }
}

/// v8 columns. `flags`: named 1, extra 2, missing 4, error 8. With `fold`, the flags live in the kind
/// code space (`code` column carries the folded code) and there is no flags column.
pub struct V8 {
    pub n: usize,
    pub wd: Widths,
    pub kf: Vec<u8>,
    pub par: Vec<u8>,
    pub size: Vec<u8>,
    pub start: Vec<u8>,
    pub len: Vec<u8>,
    pub flags: Vec<u8>,
}

pub fn flag_byte(p: &Pending) -> u8 {
    u8::from(p.named) | u8::from(p.extra) << 1 | u8::from(p.missing) << 2 | u8::from(p.error) << 3
}

impl V8 {
    pub fn encode(nodes: &[Pending], source_len: usize) -> Self {
        let n = nodes.len();
        let wd = widths(n, source_len);
        let mut t = V8 {
            n,
            wd,
            kf: Vec::with_capacity(2 * n),
            par: Vec::with_capacity(wd.w * n),
            size: Vec::with_capacity(wd.w * n),
            start: Vec::with_capacity(wd.s * n),
            len: Vec::with_capacity(wd.s * n),
            flags: Vec::with_capacity(n),
        };
        for (i, p) in nodes.iter().enumerate() {
            t.kf.extend_from_slice(&p.kf.to_le_bytes());
            put(&mut t.par, p.parent.map_or(1, |q| i as u32 - q), wd.w);
            put(&mut t.size, p.size, wd.w);
            put(&mut t.start, p.start, wd.s);
            put(&mut t.len, p.length, wd.s);
            t.flags.push(flag_byte(p));
        }
        t
    }
    fn header(&self, b: &mut Vec<u8>) {
        b.extend_from_slice(&[0u8; HEADER_V8]);
    }
    /// v8: all six columns.
    pub fn columns(&self) -> Vec<u8> {
        let mut b = Vec::new();
        self.header(&mut b);
        for c in [&self.kf, &self.par, &self.size, &self.start, &self.len, &self.flags] {
            b.extend_from_slice(c);
        }
        b
    }
    /// v8i: the same fields interleaved per node.
    pub fn interleaved(&self) -> Vec<u8> {
        let mut b = Vec::new();
        self.header(&mut b);
        let (w, s) = (self.wd.w, self.wd.s);
        for i in 0..self.n {
            b.extend_from_slice(&self.kf[2 * i..2 * i + 2]);
            b.extend_from_slice(&self.par[w * i..w * i + w]);
            b.extend_from_slice(&self.size[w * i..w * i + w]);
            b.extend_from_slice(&self.start[s * i..s * i + s]);
            b.extend_from_slice(&self.len[s * i..s * i + s]);
            b.push(self.flags[i]);
        }
        b
    }
    /// v8k: flags folded into the kind code (columns kf, par, size, start, len).
    pub fn folded(&self) -> Vec<u8> {
        let mut b = Vec::new();
        self.header(&mut b);
        for c in [&self.kf, &self.par, &self.size, &self.start, &self.len] {
            b.extend_from_slice(c);
        }
        b
    }
    /// v8np: folded and without the parent column (kf, size, start, len).
    pub fn no_parent(&self) -> Vec<u8> {
        let mut b = Vec::new();
        self.header(&mut b);
        for c in [&self.kf, &self.size, &self.start, &self.len] {
            b.extend_from_slice(c);
        }
        b
    }
    #[inline]
    pub fn kind(&self, i: usize) -> u16 {
        (get(&self.kf, i, 2) & 1023) as u16
    }
    #[inline]
    pub fn parent(&self, i: usize) -> i64 {
        i as i64 - get(&self.par, i, self.wd.w) as i64
    }
    #[inline]
    pub fn size(&self, i: usize) -> u32 {
        get(&self.size, i, self.wd.w)
    }
    #[inline]
    pub fn span(&self, i: usize) -> (u32, u32) {
        let s = get(&self.start, i, self.wd.s);
        (s, s + get(&self.len, i, self.wd.s))
    }
}


/// v9: regular columns without absolute positions. `start(i) = base[i / 16] + off[i]` (starts are
/// non-decreasing in preorder); `end(i) = start(i + size(i) - 1) + tail[i]`, the end measured from
/// the start of the node's last descendant (a leaf's own length). Flags are folded into the kind code.
/// (Measuring from the base of the last descendant's block was tried: 2.27 vs 1.77 zstd B/node.)
/// `o` is the width of `off`, `t` of `tail`; both chosen per document.
pub struct V9 {
    pub n: usize,
    pub w: usize,
    pub o: usize,
    pub t: usize,
    pub kf: Vec<u8>,
    pub par: Vec<u8>,
    pub size: Vec<u8>,
    pub base: Vec<u8>,
    pub off: Vec<u8>,
    pub tail: Vec<u8>,
}

impl V9 {
    pub fn encode(nodes: &[Pending]) -> Self {
        let n = nodes.len();
        let w = if n < 0xffff { 2 } else { 4 };
        let mut max_off = 0u32;
        let mut max_tail = 0u32;
        for (i, p) in nodes.iter().enumerate() {
            let b = nodes[i / CHECKPOINT_STRIDE * CHECKPOINT_STRIDE].start;
            max_off = max_off.max(p.start - b);
            let last = &nodes[i + p.size as usize - 1];
            max_tail = max_tail.max(p.start + p.length - last.start);
        }
        let o = if max_off < 0x1_0000 { 2 } else { 4 };
        let t = if max_tail < 0x1_0000 { 2 } else { 4 };
        let mut v = V9 { n, w, o, t, kf: Vec::with_capacity(2 * n), par: Vec::with_capacity(w * n), size: Vec::with_capacity(w * n),
            base: Vec::with_capacity(4 * n.div_ceil(CHECKPOINT_STRIDE)), off: Vec::with_capacity(o * n), tail: Vec::with_capacity(t * n) };
        for (i, p) in nodes.iter().enumerate() {
            if i % CHECKPOINT_STRIDE == 0 {
                v.base.extend_from_slice(&p.start.to_le_bytes());
            }
            let b = nodes[i / CHECKPOINT_STRIDE * CHECKPOINT_STRIDE].start;
            v.kf.extend_from_slice(&p.kf.to_le_bytes());
            put(&mut v.par, p.parent.map_or(1, |q| i as u32 - q), w);
            put(&mut v.size, p.size, w);
            put(&mut v.off, p.start - b, o);
            let last = &nodes[i + p.size as usize - 1];
            put(&mut v.tail, p.start + p.length - last.start, t);
        }
        v
    }
    pub fn bytes(&self, parents: bool) -> Vec<u8> {
        let mut b = vec![0u8; HEADER_V8];
        b.extend_from_slice(&self.kf);
        if parents {
            b.extend_from_slice(&self.par);
        }
        for c in [&self.size, &self.base, &self.off, &self.tail] {
            b.extend_from_slice(c);
        }
        b
    }
    /// v9r: v7's interleaved 6-byte record (kf, parent delta, size) unchanged, then the fixed span
    /// record (off, tail) interleaved, then `base`.
    pub fn record_with_spans(&self) -> (Vec<u8>, Vec<u8>) {
        let mut rec = Vec::with_capacity(self.n * (2 + 2 * self.w));
        let mut sp = Vec::with_capacity(self.n * (self.o + self.t));
        for i in 0..self.n {
            rec.extend_from_slice(&self.kf[2 * i..2 * i + 2]);
            rec.extend_from_slice(&self.par[self.w * i..self.w * (i + 1)]);
            rec.extend_from_slice(&self.size[self.w * i..self.w * (i + 1)]);
            sp.extend_from_slice(&self.off[self.o * i..self.o * (i + 1)]);
            sp.extend_from_slice(&self.tail[self.t * i..self.t * (i + 1)]);
        }
        (rec, sp)
    }
    /// v9i: one interleaved record per node without the parent (kf, size, off, tail), then `base`.
    pub fn interleaved_np(&self) -> Vec<u8> {
        let mut rec = Vec::with_capacity(self.n * (2 + self.w + self.o + self.t));
        for i in 0..self.n {
            rec.extend_from_slice(&self.kf[2 * i..2 * i + 2]);
            rec.extend_from_slice(&self.size[self.w * i..self.w * (i + 1)]);
            rec.extend_from_slice(&self.off[self.o * i..self.o * (i + 1)]);
            rec.extend_from_slice(&self.tail[self.t * i..self.t * (i + 1)]);
        }
        rec
    }
    #[inline]
    pub fn kind(&self, i: usize) -> u16 {
        (get(&self.kf, i, 2) & 1023) as u16
    }
    #[inline]
    pub fn parent(&self, i: usize) -> i64 {
        i as i64 - get(&self.par, i, self.w) as i64
    }
    #[inline]
    pub fn size(&self, i: usize) -> u32 {
        get(&self.size, i, self.w)
    }
    #[inline]
    pub fn start(&self, i: usize) -> u32 {
        get(&self.base, i / CHECKPOINT_STRIDE, 4) + get(&self.off, i, self.o)
    }
    #[inline]
    pub fn span(&self, i: usize) -> (u32, u32) {
        let last = i + self.size(i) as usize - 1;
        (self.start(i), self.start(last) + get(&self.tail, i, self.t))
    }
}

// ---- corpus ----

pub fn language(lang: &str, path: &Path) -> Option<Language> {
    let ext = path.extension()?.to_str()?;
    Some(match (lang, ext) {
        ("python", "py") => tree_sitter_python::LANGUAGE.into(),
        ("typescript", "ts" | "mts" | "cts") => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        ("typescript", "tsx") => tree_sitter_typescript::LANGUAGE_TSX.into(),
        ("rust", "rs") => tree_sitter_rust::LANGUAGE.into(),
        ("php", "php") if !path.to_string_lossy().ends_with(".blade.php") => tree_sitter_php::LANGUAGE_PHP.into(),
        _ => return None,
    })
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let name = e.file_name();
        let name = name.to_string_lossy();
        if let Ok(ft) = e.file_type() {
            if ft.is_dir() {
                if !matches!(name.as_ref(), "node_modules" | ".git" | "target" | ".venv" | "venv" | "__pycache__" | "dist") {
                    walk(&p, out);
                }
            } else if ft.is_file() {
                out.push(p);
            }
        }
    }
}

pub struct Doc {
    pub path: PathBuf,
    pub source: String,
    pub nodes: Vec<Pending>,
    pub language: Language,
}

pub fn load(lang: &str, dirs: &[String], max_files: usize) -> (Vec<Doc>, usize, f64) {
    let mut files = Vec::new();
    for d in dirs {
        walk(Path::new(d), &mut files);
    }
    let mut docs = Vec::new();
    let mut rejected = 0;
    let mut parse_secs = 0.0;
    let mut parser = Parser::new();
    for path in files {
        if docs.len() >= max_files {
            break;
        }
        let Some(language) = language(lang, &path) else { continue };
        let Ok(source) = std::fs::read_to_string(&path) else { continue };
        if source.is_empty() {
            continue;
        }
        parser.set_language(&language).unwrap();
        let t0 = Instant::now();
        let Some(tree) = parser.parse(&source, None) else {
            rejected += 1;
            continue;
        };
        let mut nodes = Vec::new();
        let ok = encode_tree(&mut nodes, tree.root_node(), &language).and_then(|_| sizes(&mut nodes));
        parse_secs += t0.elapsed().as_secs_f64();
        match ok {
            Ok(()) => docs.push(Doc { path, source, nodes, language }),
            Err(_) => rejected += 1,
        }
    }
    (docs, rejected, parse_secs)
}

fn zlen(bytes: &[u8]) -> usize {
    zstd::bulk::compress(bytes, 3).unwrap().len()
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn measure(corpus: &str, lang: &str, dirs: &[String]) {
    let (docs, rejected, parse_secs) = load(lang, dirs, usize::MAX);
    let files = docs.len();
    let nodes: usize = docs.iter().map(|d| d.nodes.len()).sum();
    let source: usize = docs.iter().map(|d| d.source.len()).sum();

    // ---- content and invariant statistics ----
    let mut max_kind = 0u16;
    let mut max_field = 0u16;
    let mut alias_nodes = 0usize;
    let mut aliases = std::collections::BTreeSet::new();
    let (mut extra, mut missing, mut error, mut named) = (0usize, 0usize, 0usize, 0usize);
    // per (grammar, kind): counts of (extra, named) combinations, to test folding flags into codes
    let mut per_kind: HashMap<(usize, u16), [usize; 4]> = HashMap::new();
    let mut folded_codes: std::collections::BTreeSet<(usize, u16, bool, bool, bool, Option<String>)> = Default::default();
    let (mut internal, mut start_viol, mut end_viol, mut end_viol_tail_special) = (0usize, 0usize, 0usize, 0usize);
    let mut leaves = 0usize;
    let mut leaf_len_ge256 = 0usize;
    let (mut docs_w4, mut docs_s4, mut nodes_w4, mut nodes_s4) = (0usize, 0usize, 0usize, 0usize);
    let (mut big_nodes, mut big_rel_fit) = (0usize, 0usize);
    let mut grammar_ids: Vec<Language> = Vec::new();
    for d in &docs {
        let g = match grammar_ids.iter().position(|l| *l == d.language) {
            Some(g) => g,
            None => {
                grammar_ids.push(d.language.clone());
                grammar_ids.len() - 1
            }
        };
        let n = d.nodes.len();
        let wd = widths(n, d.source.len());
        if wd.w == 4 {
            docs_w4 += 1;
            nodes_w4 += n;
        }
        if wd.s == 4 {
            docs_s4 += 1;
            nodes_s4 += n;
        }
        for (i, p) in d.nodes.iter().enumerate() {
            let kind = p.kf & 1023;
            max_kind = max_kind.max(kind);
            max_field = max_field.max(p.kf >> 10);
            if let Some(a) = &p.kind_alias {
                alias_nodes += 1;
                aliases.insert((g, a.0.clone(), a.1));
            }
            extra += p.extra as usize;
            missing += p.missing as usize;
            error += p.error as usize;
            named += p.named as usize;
            per_kind.entry((g, kind)).or_default()[usize::from(p.extra) * 2 + usize::from(p.named)] += 1;
            folded_codes.insert((g, kind, p.named, p.extra, p.missing, p.kind_alias.as_ref().map(|a| a.0.clone())));
            if p.size > 1 {
                internal += 1;
                let first = &d.nodes[i + 1];
                if first.start != p.start {
                    start_viol += 1;
                }
                let last = &d.nodes[i + p.size as usize - 1];
                if last.start + last.length != p.start + p.length {
                    end_viol += 1;
                    if last.extra || last.missing || last.error || last.length == 0 {
                        end_viol_tail_special += 1;
                    }
                }
            } else {
                leaves += 1;
                if p.length >= 256 {
                    leaf_len_ge256 += 1;
                }
            }
            if wd.s == 4 {
                big_nodes += 1;
                let ps = p.parent.map_or(0, |q| d.nodes[q as usize].start);
                if p.start - ps < 0x1_0000 && p.length < 0x1_0000 {
                    big_rel_fit += 1;
                }
            }
        }
    }
    let mut kinds_nonuniform_extra = 0usize;
    let mut nodes_nonuniform_extra = 0usize;
    let mut kinds_nonuniform_named = 0usize;
    for c in per_kind.values() {
        let ex = c[2] + c[3];
        let nex = c[0] + c[1];
        if ex > 0 && nex > 0 {
            kinds_nonuniform_extra += 1;
            nodes_nonuniform_extra += ex.min(nex);
        }
        let nm = c[1] + c[3];
        let unm = c[0] + c[2];
        if nm > 0 && unm > 0 {
            kinds_nonuniform_named += 1;
        }
    }
    let mut folded_per_grammar = vec![0usize; grammar_ids.len()];
    for f in &folded_codes {
        folded_per_grammar[f.0] += 1;
    }
    let catalog_kinds: Vec<usize> = grammar_ids.iter().map(|l| l.node_kind_count()).collect();

    // ---- encode all variants, sizes ----
    let t = Instant::now();
    let v7: Vec<V7> = docs.iter().map(|d| V7::encode(&d.nodes).unwrap()).collect();
    let enc_v7 = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let v8: Vec<V8> = docs.iter().map(|d| V8::encode(&d.nodes, d.source.len())).collect();
    let enc_v8 = t.elapsed().as_secs_f64();
    let t = Instant::now();
    let v9: Vec<V9> = docs.iter().map(|d| V9::encode(&d.nodes)).collect();
    let enc_v9 = t.elapsed().as_secs_f64();
    let docs_o4 = v9.iter().filter(|v| v.o == 4).count();
    let docs_t4 = v9.iter().filter(|v| v.t == 4).count();

    let mut logical: HashMap<&str, usize> = HashMap::new();
    let mut packed: HashMap<&str, usize> = HashMap::new();
    let t = Instant::now();
    let mut v7_overflow = 0usize;
    let mut v7_side = 0usize;
    for ((a, b), c) in v7.iter().zip(&v8).zip(&v9) {
        let (rec, sp) = c.record_with_spans();
        let mut v9r = vec![0u8; HEADER_V8];
        v9r.extend_from_slice(&rec);
        v9r.extend_from_slice(&sp);
        v9r.extend_from_slice(&c.base);
        let mut v9i = vec![0u8; HEADER_V8];
        v9i.extend_from_slice(&c.interleaved_np());
        v9i.extend_from_slice(&c.base);
        for (name, bytes) in [("v9", c.bytes(true)), ("v9np", c.bytes(false)), ("v9r", v9r), ("v9i", v9i)] {
            *logical.entry(name).or_default() += bytes.len();
            *packed.entry(name).or_default() += zlen(&bytes);
        }
        v7_overflow += a.par_overflow.len() + a.size_overflow.len();
        v7_side += (a.par_overflow.len() + a.size_overflow.len()) * 8
            + (a.extra.len() + a.error.len() + a.missing.len()) * 4
            + a.kind_aliases.iter().map(|x| 5 + x.0.len()).sum::<usize>()
            + a.kind_overrides.len() * 6;
        let variants: [(&str, Vec<u8>); 5] = [
            ("v7", a.to_bytes()),
            ("v8", b.columns()),
            ("v8i", b.interleaved()),
            ("v8k", b.folded()),
            ("v8np", b.no_parent()),
        ];
        for (name, bytes) in variants {
            *logical.entry(name).or_default() += bytes.len();
            *packed.entry(name).or_default() += zlen(&bytes);
        }
        // v7 without its 144-byte header and digest: the record/span tables alone
        *logical.entry("v7_body").or_default() += a.rec.len() + a.spans.len() + a.checkpoints.len();
        *logical.entry("v7_spans").or_default() += a.spans.len() + a.checkpoints.len();
    }
    let compress_secs = t.elapsed().as_secs_f64();
    let src_z: usize = docs.iter().map(|d| zlen(d.source.as_bytes())).sum();

    // ---- Rust decode benchmarks (each run 5 times, interleaved; minimum reported) ----
    let catalogs: Vec<Vec<bool>> = docs
        .iter()
        .map(|d| (0..d.language.node_kind_count() as u16).map(|k| d.language.node_kind_is_named(k)).collect())
        .collect();
    let mut rng = Rng(0x9E3779B97F4A7C15);
    let probes: Vec<(usize, usize)> = (0..2_000_000)
        .map(|_| {
            let d = (rng.next() % docs.len() as u64) as usize;
            (d, (rng.next() % docs[d].nodes.len() as u64) as usize)
        })
        .collect();
    let time = |f: &mut dyn FnMut() -> u64| -> (f64, u64) {
        let t = Instant::now();
        let r = f();
        (t.elapsed().as_nanos() as f64, r)
    };
    let mut best: HashMap<&str, f64> = HashMap::new();
    let mut checks: HashMap<&str, u64> = HashMap::new();
    let mut record = |name: &'static str, (ns, r): (f64, u64), per: usize| {
        let v = ns / per as f64;
        let e = best.entry(name).or_insert(f64::MAX);
        *e = e.min(v);
        checks.insert(name, r);
    };
    for _ in 0..9 {
        record("random_v7", time(&mut || {
            let mut sink = 0u64;
            for &(d, i) in &probes {
                let a = &v7[d];
                let (s, e) = a.span(i);
                sink = sink.wrapping_add(a.kind(i) as u64).wrapping_add(a.parent(i) as u64).wrapping_add(a.size(i) as u64)
                    .wrapping_add(s as u64 ^ e as u64).wrapping_add(a.named(i, &catalogs[d]) as u64);
            }
            sink
        }), probes.len());
        record("random_v8", time(&mut || {
            let mut sink = 0u64;
            for &(d, i) in &probes {
                let b = &v8[d];
                let (s, e) = b.span(i);
                sink = sink.wrapping_add(b.kind(i) as u64).wrapping_add(b.parent(i) as u64).wrapping_add(b.size(i) as u64)
                    .wrapping_add(s as u64 ^ e as u64).wrapping_add((b.flags[i] & 1) as u64);
            }
            sink
        }), probes.len());
        record("random_v9", time(&mut || {
            let mut sink = 0u64;
            for &(d, i) in &probes {
                let c = &v9[d];
                let (s, e) = c.span(i);
                sink = sink.wrapping_add(c.kind(i) as u64).wrapping_add(c.parent(i) as u64).wrapping_add(c.size(i) as u64)
                    .wrapping_add(s as u64 ^ e as u64).wrapping_add((c.kind(i) == ERROR_CODE || catalogs[d].get(c.kind(i) as usize).copied().unwrap_or(true)) as u64);
            }
            sink
        }), probes.len());
        record("random_span_v7", time(&mut || {
            let mut sink = 0u64;
            for &(d, i) in &probes { let (s, e) = v7[d].span(i); sink = sink.wrapping_add(s as u64 ^ e as u64); }
            sink
        }), probes.len());
        record("random_span_v8", time(&mut || {
            let mut sink = 0u64;
            for &(d, i) in &probes { let (s, e) = v8[d].span(i); sink = sink.wrapping_add(s as u64 ^ e as u64); }
            sink
        }), probes.len());
        record("random_span_v9", time(&mut || {
            let mut sink = 0u64;
            for &(d, i) in &probes { let (s, e) = v9[d].span(i); sink = sink.wrapping_add(s as u64 ^ e as u64); }
            sink
        }), probes.len());
        record("seq_v7", time(&mut || {
            let mut sink = 0u64;
            for a in &v7 {
                let n = a.rec.len() / WIDTH;
                let mut offset = 0usize;
                let mut start = 0i64;
                for i in 0..n {
                    let enc = read_varint(&a.spans, &mut offset);
                    let delta = ((enc >> 1) as i32) ^ -((enc & 1) as i32);
                    let len = read_varint(&a.spans, &mut offset);
                    start += i64::from(delta);
                    sink = sink.wrapping_add(a.kind(i) as u64).wrapping_add(a.parent(i) as u64).wrapping_add(a.size(i) as u64)
                        .wrapping_add(start as u64 ^ (start as u64 + len as u64));
                }
            }
            sink
        }), nodes);
        record("seq_v8", time(&mut || {
            let mut sink = 0u64;
            for b in &v8 {
                for i in 0..b.n {
                    let (s, e) = b.span(i);
                    sink = sink.wrapping_add(b.kind(i) as u64).wrapping_add(b.parent(i) as u64).wrapping_add(b.size(i) as u64)
                        .wrapping_add(s as u64 ^ e as u64);
                }
            }
            sink
        }), nodes);
        record("seq_v9", time(&mut || {
            let mut sink = 0u64;
            for c in &v9 {
                for i in 0..c.n {
                    let (s, e) = c.span(i);
                    sink = sink.wrapping_add(c.kind(i) as u64).wrapping_add(c.parent(i) as u64).wrapping_add(c.size(i) as u64)
                        .wrapping_add(s as u64 ^ e as u64);
                }
            }
            sink
        }), nodes);
        record("v9np_parent_rebuild", time(&mut || {
            let mut sink = 0u64;
            for c in &v9 {
                let mut parents = vec![0u32; c.n];
                let mut stack: Vec<(u32, u32)> = Vec::new();
                for i in 0..c.n as u32 {
                    while let Some(&(_, end)) = stack.last() {
                        if i >= end { stack.pop(); } else { break; }
                    }
                    parents[i as usize] = stack.last().map_or(u32::MAX, |&(p, _)| p);
                    stack.push((i, i + c.size(i as usize)));
                }
                sink = sink.wrapping_add(parents.iter().map(|&p| p as u64).sum::<u64>());
            }
            sink
        }), nodes);
        record("v7_restore_to_u32x5", time(&mut || {
            let mut sink = 0u64;
            for a in &v7 {
                let n = a.rec.len() / WIDTH;
                let mut cols = vec![[0u32; 5]; n];
                let mut offset = 0usize;
                let mut start = 0i64;
                for i in 0..n {
                    let enc = read_varint(&a.spans, &mut offset);
                    let delta = ((enc >> 1) as i32) ^ -((enc & 1) as i32);
                    let len = read_varint(&a.spans, &mut offset);
                    start += i64::from(delta);
                    cols[i] = [a.kind(i) as u32, a.parent(i) as u32, a.size(i), start as u32, start as u32 + len];
                }
                sink = sink.wrapping_add(cols.len() as u64);
            }
            sink
        }), nodes);
    }
    let parity_random = checks["random_v7"] == checks["random_v8"] && checks["random_v8"] == checks["random_v9"];
    let parity_seq = checks["seq_v7"] == checks["seq_v8"] && checks["seq_v8"] == checks["seq_v9"];
    let mut bj = String::new();
    let mut names: Vec<_> = best.keys().copied().collect();
    names.sort();
    for k in names {
        bj.push_str(&format!("\"{k}\":{:.2},", best[k]));
    }
    let bpn = |x: usize| x as f64 / nodes as f64;
    let mut lj = String::new();
    let mut names: Vec<_> = logical.keys().copied().collect();
    names.sort();
    for k in &names {
        lj.push_str(&format!("\"{k}\":{:.3},", bpn(logical[k])));
    }
    let mut pj = String::new();
    let mut names: Vec<_> = packed.keys().copied().collect();
    names.sort();
    for k in &names {
        pj.push_str(&format!("\"{k}\":{:.3},", bpn(packed[k])));
    }
    println!(
        "{{\"corpus\":\"{corpus}\",\"lang\":\"{lang}\",\"files\":{files},\"rejected\":{rejected},\"nodes\":{nodes},\"source_bytes\":{source},\"nodes_per_kb\":{:.1},\
\"parse_s\":{parse_secs:.2},\"logical_Bpn\":{{{}}},\"zstd3_Bpn\":{{{}}},\"source_zstd_Bpn\":{:.3},\"source_Bpn\":{:.3},\
\"encode_ns_per_node\":{{\"v7\":{:.1},\"v8\":{:.1},\"v9\":{:.1}}},\"compress_s\":{compress_secs:.1},\
\"decode_ns\":{{{}}},\"parity_random\":{parity_random},\"parity_seq\":{parity_seq},\"docs_v9_off32\":{docs_o4},\"docs_v9_tail32\":{docs_t4},\
\"v7_overflow_entries\":{v7_overflow},\"v7_side_tables_Bpn\":{:.4},\
\"max_kind\":{max_kind},\"max_field\":{max_field},\"catalog_kinds\":{catalog_kinds:?},\"alias_nodes\":{alias_nodes},\"distinct_aliases\":{},\
\"extra\":{extra},\"missing\":{missing},\"error\":{error},\"named\":{named},\
\"kinds_nonuniform_extra\":{kinds_nonuniform_extra},\"nodes_minority_extra\":{nodes_nonuniform_extra},\"kinds_nonuniform_named\":{kinds_nonuniform_named},\
\"folded_codes_per_grammar\":{folded_per_grammar:?},\
\"internal\":{internal},\"start_derivation_violations\":{start_viol},\"end_derivation_violations\":{end_viol},\"end_violations_special_tail\":{end_viol_tail_special},\
\"leaves\":{leaves},\"leaf_len_ge256\":{leaf_len_ge256},\
\"docs_w4\":{docs_w4},\"docs_s4\":{docs_s4},\"nodes_in_w4_docs\":{nodes_w4},\"nodes_in_s4_docs\":{nodes_s4},\"big_doc_nodes\":{big_nodes},\"big_doc_rel16_fit\":{big_rel_fit},\
\"end\":0}}",
        nodes as f64 / (source as f64 / 1024.0),
        lj.trim_end_matches(','),
        pj.trim_end_matches(','),
        bpn(src_z),
        bpn(source),
        enc_v7 * 1e9 / nodes as f64,
        enc_v8 * 1e9 / nodes as f64,
        enc_v9 * 1e9 / nodes as f64,
        bj.trim_end_matches(','),
        bpn(v7_side),
        aliases.len(),
    );
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args[1].as_str() {
        "measure" => measure(&args[2], &args[3], &args[4..]),
        "sql" => sql::run(&args[2], &args[3], args[4].parse().unwrap(), &args[5..]),
        _ => panic!("mode"),
    }
}
