//! A program's output as a flat image (decision 0239): a fixed set of regions of fixed-width items, every reference
//! an index, read where its bytes are.
//!
//! ```text
//! header   magic, format, region count, then per region: item width, item count, byte offset (16-byte aligned)
//! meta     what the image is: a base or an overlay, how its nodes are shown
//! rels     per relation: its name (a string), its width, its rows, where its types and columns start
//! types    a byte per column: text, node, int
//! cols     per relation, per column, its rows' values (i64); the rows sorted by their values, each once
//! bytes    the strings' bytes
//! ends     each string's end in `bytes`
//! docs     per document a node names: its key, its path (a string, or none), its first node (or none); by key
//! places   per `@place` value a node column holds: the value, and how it is shown (a string); by value
//! ```
//!
//! A text column holds strings' indexes, a node column `doc << 32 | local`, an int column the integer. A string is
//! held as its text, once per image, so an image means the same in every process (decision 0212).
//!
//! **A stack.** A run's outputs are a file of images, one after another: a base, then overlays. An overlay holds the
//! outputs a later run made again, each whole; a relation is read from the last image that holds it. Each image's
//! strings, documents and places are its own. A stack is folded into one base past [`MAX_DEPTH`] images.
//!
//! Text is a reader: [`Image::show`] gives a relation's rows as the lines its `.csv` file held, sorted.

use std::collections::HashMap;
use std::path::Path;

pub const MAGIC: [u8; 8] = *b"RROUT\x00\x00\x01";
pub const FORMAT: u32 = 1;
/// The file a run's outputs are written to, in its output directory.
pub const FILE: &str = "outputs.rro";
/// The most images a stack holds before it is folded into one.
pub const MAX_DEPTH: usize = 8;

/// A column's type.
pub mod col {
    /// A string's index in the image's strings.
    pub const TEXT: u8 = 0;
    /// A node: `doc << 32 | local`; below -1, a `@place` (`places`); -1 none (`unset`).
    pub const NODE: u8 = 1;
    /// An integer, shown as it is.
    pub const INT: u8 = 2;
}

/// The image's meta words.
mod meta {
    pub const KIND: usize = 0;
    pub const FLAGS: usize = 1;
    pub const WORDS: usize = 2;
    /// `KIND`: a base, or an overlay over the images before it.
    pub const BASE: u64 = 0;
    pub const OVERLAY: u64 = 1;
}

/// How a run shows its nodes (`FLAGS`).
pub mod flags {
    /// The run is its own corpus's (not T's): a node of a document with no path is not shown by its first node.
    pub const OWN: u64 = 1;
    /// A node's local part is shown by definition, `slot.index` (`syntax::definition_ids`).
    pub const DEFINITION_IDS: u64 = 2;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(usize)]
enum R {
    Meta,
    Rels,
    Types,
    Cols,
    Bytes,
    Ends,
    Docs,
    Places,
}
const REGIONS: usize = 8;
const WIDTH: [u32; REGIONS] = [8, 40, 1, 8, 1, 4, 24, 16];
const HEADER: usize = 16 + 24 * REGIONS;
/// No string, no first node (`docs`).
const NONE: u64 = u64::MAX;

fn bad(what: impl std::fmt::Display) -> String {
    format!("output image: {what}")
}

/// A document as a node column shows it: its path (a file of the run), its first node (T's numbering), neither (a
/// guest's, decision 0055).
#[derive(Clone, Debug, Default)]
pub struct Doc {
    pub path: Option<String>,
    pub first: Option<i64>,
}

/// An image being made: relations from their rows or their lines, then `finish`.
pub struct Builder {
    flags: u64,
    overlay: bool,
    strings: Vec<u8>,
    ends: Vec<u32>,
    ids: HashMap<String, u32>,
    rels: Vec<(u32, Vec<u8>, usize, Vec<i64>)>,
    docs: std::collections::BTreeMap<i64, (u64, u64)>,
    places: std::collections::BTreeMap<i64, u64>,
}

impl Builder {
    /// A base image (`overlay`: false) or an overlay, its nodes shown as `flags` says.
    pub fn new(flags: u64, overlay: bool) -> Self {
        Builder { flags, overlay, strings: Vec::new(), ends: Vec::new(), ids: HashMap::new(), rels: Vec::new(), docs: Default::default(), places: Default::default() }
    }
    /// The index of the string `s`, held once.
    pub fn text(&mut self, s: &str) -> u32 {
        if let Some(&id) = self.ids.get(s) {
            return id;
        }
        let id = self.ends.len() as u32;
        self.strings.extend_from_slice(s.as_bytes());
        self.ends.push(self.strings.len() as u32);
        self.ids.insert(s.to_owned(), id);
        id
    }
    /// A document a node column names.
    pub fn doc(&mut self, key: i64, doc: &Doc) {
        let path = doc.path.as_deref().map_or(NONE, |p| u64::from(self.text(p)));
        self.docs.insert(key, (path, doc.first.map_or(NONE, |f| f as u64)));
    }
    /// A `@place` value a node column holds, and how it is shown.
    pub fn place(&mut self, value: i64, shown: &str) {
        let s = u64::from(self.text(shown));
        self.places.insert(value, s);
    }
    /// A relation of `types` from its rows, flat (text columns as string indexes): sorted, each row once.
    pub fn rows(&mut self, name: &str, types: &[u8], mut rows: Vec<i64>) {
        let name = self.text(name);
        let k = types.len();
        if k > 0 {
            let mut order: Vec<usize> = (0..rows.len() / k).collect();
            order.sort_unstable_by(|&a, &b| rows[a * k..a * k + k].cmp(&rows[b * k..b * k + k]));
            order.dedup_by(|a, b| rows[*a * k..*a * k + k] == rows[*b * k..*b * k + k]);
            let n = order.len();
            // Column-major: each column's values, in the rows' order.
            let mut cols = vec![0i64; n * k];
            for (i, &r) in order.iter().enumerate() {
                for c in 0..k {
                    cols[c * n + i] = rows[r * k + c];
                }
            }
            rows = cols;
            self.rels.push((name, types.to_vec(), n, rows));
        } else {
            self.rels.push((name, Vec::new(), usize::from(!rows.is_empty()), Vec::new()));
        }
    }
    /// A relation of `types` from its rows, flat, in the order and number given (a memo entry's, decision 0240 §4:
    /// replayed as it was made). Readers of the run's outputs ask for `rows`, which sorts them.
    pub fn rows_as_is(&mut self, name: &str, types: &[u8], rows: Vec<i64>) {
        let name = self.text(name);
        let k = types.len();
        if k == 0 {
            self.rels.push((name, Vec::new(), 0, Vec::new()));
            return;
        }
        let n = rows.len() / k;
        let mut cols = vec![0i64; n * k];
        for (i, row) in rows.chunks_exact(k).enumerate() {
            for (c, &v) in row.iter().enumerate() {
                cols[c * n + i] = v;
            }
        }
        self.rels.push((name, types.to_vec(), n, cols));
    }
    /// A relation from the lines a run gathered for it (tab-separated, as `decoded_outputs` makes them): a column is
    /// a node column where `nodes` says and every value is an integer, an int column where `ints` says and every value
    /// is one as it is written, otherwise text. A node column holding any other value is held as the text the writer
    /// showed (`shown`), so the image shows every line as it was.
    pub fn lines(&mut self, name: &str, nodes: &[usize], ints: &[usize], lines: &[String], shown: &dyn Fn(usize, &str) -> String) {
        let width = lines.first().map_or(0, |l| l.split('\t').count());
        if lines.iter().any(|l| l.split('\t').count() != width) {
            // Rows of differing widths (a value holding a tab): each line one text value.
            let rows: Vec<i64> = lines.iter().map(|l| i64::from(self.text(&l.split('\t').enumerate().map(|(i, v)| shown(i, v)).collect::<Vec<_>>().join("\t")))).collect();
            self.rows(name, &[col::TEXT], rows);
            return;
        }
        let cells: Vec<Vec<&str>> = lines.iter().map(|l| l.split('\t').collect()).collect();
        let types: Vec<u8> = (0..width)
            .map(|c| {
                let all = |f: &dyn Fn(&str) -> bool| cells.iter().all(|r| f(r[c]));
                if nodes.contains(&c) && all(&|v| v.parse::<i64>().is_ok_and(|n| n.to_string() == v)) {
                    col::NODE
                } else if ints.contains(&c) && !nodes.contains(&c) && all(&|v| v.parse::<i64>().is_ok_and(|n| n.to_string() == v)) {
                    col::INT
                } else {
                    col::TEXT
                }
            })
            .collect();
        let mut rows = Vec::with_capacity(cells.len() * width);
        for r in &cells {
            for (c, v) in r.iter().enumerate() {
                rows.push(match types[c] {
                    col::TEXT if nodes.contains(&c) => i64::from(self.text(&shown(c, v))),
                    col::TEXT => i64::from(self.text(v)),
                    _ => v.parse::<i64>().expect("checked"),
                });
            }
        }
        self.rows(name, &types, rows);
    }
    /// A relation from the parts a run handed of it (`Rows`), their values as they are: a column is text where any
    /// part holds it as text. A node column (`nodes`) held as text is held as the writer showed each value (`shown`),
    /// as `lines` holds one; parts of differing widths are put by their lines.
    pub fn put(&mut self, name: &str, parts: &[&Rows], nodes: &[usize], shown: &dyn Fn(usize, &str) -> String) {
        let width = parts.iter().find(|p| !p.is_empty()).map_or_else(|| parts.first().map_or(0, |p| p.width()), |p| p.width());
        if parts.iter().any(|p| p.ragged() || (!p.is_empty() && p.width() != width)) {
            let mut lines: Vec<String> = parts.iter().flat_map(|p| p.lines()).collect();
            lines.sort_unstable();
            lines.dedup();
            self.lines(name, nodes, &[], &lines, shown);
            return;
        }
        let types: Vec<u8> = (0..width)
            .map(|c| {
                let of = |t: u8| parts.iter().filter(|p| !p.is_empty()).any(|p| p.types()[c] == t);
                if of(col::TEXT) {
                    col::TEXT
                } else if nodes.contains(&c) || parts.iter().find(|p| !p.is_empty()).is_some_and(|p| p.types()[c] == col::NODE) {
                    col::NODE
                } else {
                    col::INT
                }
            })
            .collect();
        if width == 0 {
            let any = parts.iter().any(|p| !p.is_empty());
            self.rows(name, &[], if any { vec![0] } else { Vec::new() });
            return;
        }
        let mut rows = Vec::with_capacity(parts.iter().map(|p| p.len()).sum::<usize>() * width);
        // Per part, its texts' indexes here: each text held once in the image.
        for p in parts.iter().filter(|p| !p.is_empty()) {
            let mut here: Vec<Option<i64>> = vec![None; p.texts.len()];
            for i in 0..p.len() {
                for (c, &v) in p.row(i).iter().enumerate() {
                    rows.push(match (p.types()[c], types[c]) {
                        (col::TEXT, _) if nodes.contains(&c) => i64::from(self.text(&shown(c, p.text(v)))),
                        (col::TEXT, _) => *here[v as usize].get_or_insert_with(|| i64::from(self.text(p.text(v)))),
                        (_, col::TEXT) if nodes.contains(&c) => i64::from(self.text(&shown(c, &v.to_string()))),
                        (_, col::TEXT) => i64::from(self.text(&v.to_string())),
                        _ => v,
                    });
                }
            }
        }
        self.rows(name, &types, rows);
    }
    /// Relation `rel` of another image, copied: its strings held again here, its documents and places with them (a
    /// stack folded into one base).
    pub fn take(&mut self, name: &str, rel: Rel<'_>) {
        let types = rel.types().to_vec();
        let mut rows = Vec::with_capacity(rel.rows * rel.width);
        for i in 0..rel.rows {
            for (c, &t) in types.iter().enumerate() {
                let v = rel.col(c)[i];
                rows.push(match t {
                    col::TEXT => i64::from(self.text(rel.image.string(v as u32))),
                    col::NODE => {
                        if let Some(shown) = rel.image.place_shown(v) {
                            let shown = shown.to_owned();
                            self.place(v, &shown);
                        } else if v >= 0 && let Some(doc) = rel.image.doc(v >> 32) {
                            self.doc(v >> 32, &doc);
                        }
                        v
                    }
                    _ => v,
                });
            }
        }
        self.rows(name, &types, rows);
    }
    /// The image's bytes.
    pub fn finish(self) -> Vec<u8> {
        let mut w: Vec<Vec<u8>> = vec![Vec::new(); REGIONS];
        let put = |w: &mut Vec<Vec<u8>>, r: R, v: u64| w[r as usize].extend_from_slice(&v.to_le_bytes());
        put(&mut w, R::Meta, if self.overlay { meta::OVERLAY } else { meta::BASE });
        put(&mut w, R::Meta, self.flags);
        let (mut col_at, mut type_at) = (0u64, 0u64);
        for (name, types, n, cols) in &self.rels {
            for v in [u64::from(*name), types.len() as u64, *n as u64, col_at, type_at] {
                put(&mut w, R::Rels, v);
            }
            w[R::Types as usize].extend_from_slice(types);
            for v in cols {
                put(&mut w, R::Cols, *v as u64);
            }
            col_at += cols.len() as u64;
            type_at += types.len() as u64;
        }
        w[R::Bytes as usize] = self.strings;
        for e in &self.ends {
            w[R::Ends as usize].extend_from_slice(&e.to_le_bytes());
        }
        for (key, (path, first)) in &self.docs {
            for v in [*key as u64, *path, *first] {
                put(&mut w, R::Docs, v);
            }
        }
        for (value, s) in &self.places {
            put(&mut w, R::Places, *value as u64);
            put(&mut w, R::Places, *s);
        }
        let mut out = vec![0u8; HEADER];
        out[..8].copy_from_slice(&MAGIC);
        out[8..12].copy_from_slice(&FORMAT.to_le_bytes());
        out[12..16].copy_from_slice(&(REGIONS as u32).to_le_bytes());
        for (r, bytes) in w.iter().enumerate() {
            while out.len() % 16 != 0 {
                out.push(0);
            }
            let at = 16 + 24 * r;
            let offset = out.len() as u64;
            out[at..at + 4].copy_from_slice(&WIDTH[r].to_le_bytes());
            out[at + 8..at + 16].copy_from_slice(&((bytes.len() / WIDTH[r] as usize) as u64).to_le_bytes());
            out[at + 16..at + 24].copy_from_slice(&offset.to_le_bytes());
            out.extend_from_slice(bytes);
        }
        while out.len() % 16 != 0 {
            out.push(0);
        }
        out
    }
}

/// A value of a row as a run hands it: an integer (an int or a node column's), or a text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell<'a> {
    Int(i64),
    Text(&'a str),
}

/// A relation's rows as a run hands them (decision 0239, stage 2): typed columns (`col`), the values flat and by row,
/// each text held once by its content. No row is text until a reader asks (`lines`); the writer puts the values in an
/// image as they are (`Builder::put`). Shared (`Arc`) by the run's outcome, a session's kept outputs and the writer:
/// never copied.
#[derive(Clone, Debug, Default)]
pub struct Rows {
    types: Vec<u8>,
    vals: Vec<i64>,
    texts: Vec<Box<str>>,
    ids: HashMap<Box<str>, u32>,
    /// Rows of no columns.
    bare: usize,
    /// Rows of differing widths: one text column, each row its line.
    ragged: bool,
}

impl Rows {
    /// No rows, of columns typed `types` (a text arriving in an int or node column makes it a text column).
    pub fn new(types: Vec<u8>) -> Self {
        Rows { types, ..Default::default() }
    }
    pub fn types(&self) -> &[u8] {
        &self.types
    }
    pub fn width(&self) -> usize {
        self.types.len()
    }
    /// How many rows (each as handed: not yet each once).
    pub fn len(&self) -> usize {
        if self.types.is_empty() { self.bare } else { self.vals.len() / self.types.len() }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// The bytes it holds, about: its values and its texts.
    pub fn bytes(&self) -> usize {
        self.vals.len() * 8 + self.texts.iter().map(|t| t.len() * 2 + 32).sum::<usize>()
    }
    /// Whether its rows were of differing widths, held as their lines (`lines` gives them; `put` does not take it).
    pub fn ragged(&self) -> bool {
        self.ragged
    }
    fn text_id(&mut self, s: &str) -> i64 {
        if let Some(&id) = self.ids.get(s) {
            return i64::from(id);
        }
        let id = self.texts.len() as u32;
        self.texts.push(s.into());
        self.ids.insert(s.into(), id);
        i64::from(id)
    }
    /// Row `i`'s value in column `c`.
    pub fn cell(&self, i: usize, c: usize) -> Cell<'_> {
        let v = self.vals[i * self.types.len() + c];
        if self.types[c] == col::TEXT { Cell::Text(&self.texts[v as usize]) } else { Cell::Int(v) }
    }
    /// Row `i`'s values, as held (a text column's: its texts' indexes, `text`).
    pub fn row(&self, i: usize) -> &[i64] {
        let k = self.types.len();
        &self.vals[i * k..i * k + k]
    }
    /// The text a text column's value names.
    pub fn text(&self, v: i64) -> &str {
        &self.texts[v as usize]
    }
    /// Column `c` held as text from now on: its integers as they are shown.
    fn to_text(&mut self, c: usize) {
        let k = self.types.len();
        self.types[c] = col::TEXT;
        for i in 0..self.len() {
            let v = self.vals[i * k + c];
            self.vals[i * k + c] = self.text_id(&v.to_string());
        }
    }
    /// Every row held as its line, in one text column: a row of another width arrived.
    fn to_ragged(&mut self) {
        let lines: Vec<String> = (0..self.len()).map(|i| self.line(i)).collect();
        let bare = if self.types.is_empty() { self.bare } else { 0 };
        *self = Rows { types: vec![col::TEXT], ragged: true, ..Default::default() };
        for l in lines.into_iter().chain(std::iter::repeat_n(String::new(), bare)) {
            let v = self.text_id(&l);
            self.vals.push(v);
        }
    }
    /// A row appended.
    pub fn push(&mut self, row: &[Cell<'_>]) {
        if self.ragged || (row.len() != self.types.len() && !self.is_empty_shape()) {
            if !self.ragged {
                self.to_ragged();
            }
            let line = row.iter().map(|c| match c { Cell::Int(v) => v.to_string(), Cell::Text(s) => (*s).to_owned() }).collect::<Vec<_>>().join("\t");
            let v = self.text_id(&line);
            self.vals.push(v);
            return;
        }
        if self.types.len() != row.len() {
            // The first row of a relation typed with no columns: its columns are this row's, ints until a text.
            self.types = row.iter().map(|c| if matches!(c, Cell::Int(_)) { col::INT } else { col::TEXT }).collect();
        }
        if row.is_empty() {
            self.bare += 1;
            return;
        }
        for (c, cell) in row.iter().enumerate() {
            if matches!(cell, Cell::Text(_)) && self.types[c] != col::TEXT {
                self.to_text(c);
            }
        }
        for (c, cell) in row.iter().enumerate() {
            let v = match (*cell, self.types[c]) {
                (Cell::Text(s), _) => self.text_id(s),
                (Cell::Int(v), col::TEXT) => self.text_id(&v.to_string()),
                (Cell::Int(v), _) => v,
            };
            self.vals.push(v);
        }
    }
    /// Whether no row has fixed its width yet (a relation typed with no columns, no rows).
    fn is_empty_shape(&self) -> bool {
        self.types.is_empty() && self.bare == 0
    }
    /// The rows of `lines` (tab-separated, as a run's text outputs were): a node column (`nodes`) or any other whose
    /// every value is an integer as written is held as integers, the others as text.
    pub fn from_lines(lines: &[String], nodes: &[usize]) -> Rows {
        let width = lines.first().map_or(0, |l| l.split('\t').count());
        let int = |v: &str| v.parse::<i64>().ok().filter(|n| n.to_string() == v);
        let mut rows = Rows::new((0..width).map(|c| if nodes.contains(&c) { col::NODE } else { col::INT }).collect());
        let mut cells: Vec<Cell<'_>> = Vec::with_capacity(width);
        for l in lines {
            cells.clear();
            cells.extend(l.split('\t').map(|v| int(v).map_or(Cell::Text(v), Cell::Int)));
            rows.push(&cells);
        }
        rows
    }
    /// The rows of `other` appended.
    pub fn append(&mut self, other: &Rows) {
        let mut cells: Vec<Cell<'_>> = Vec::with_capacity(other.width());
        for i in 0..other.len() {
            cells.clear();
            cells.extend((0..other.width()).map(|c| other.cell(i, c)));
            self.push(&cells);
        }
    }
    /// The rows `keep` holds to (given each row's index and values), the others dropped.
    pub fn retain(&mut self, mut keep: impl FnMut(usize, &[i64]) -> bool) {
        let k = self.types.len();
        if k == 0 {
            return;
        }
        let mut at = 0;
        for i in 0..self.len() {
            if keep(i, &self.vals[i * k..i * k + k]) {
                self.vals.copy_within(i * k..i * k + k, at * k);
                at += 1;
            }
        }
        self.vals.truncate(at * k);
    }
    /// Its rows without their first column (a sentinel's).
    pub fn without_first(&self) -> Rows {
        let mut out = Rows::new(self.types.get(1..).unwrap_or_default().to_vec());
        if self.ragged {
            for i in 0..self.len() {
                let line = self.line(i);
                out.push(&line.split_once('\t').map_or("", |(_, rest)| rest).split('\t').map(Cell::Text).collect::<Vec<_>>());
            }
            return out;
        }
        let mut cells = Vec::new();
        for i in 0..self.len() {
            cells.clear();
            cells.extend((1..self.width()).map(|c| self.cell(i, c)));
            out.push(&cells);
        }
        out
    }
    /// Row `i` as a line: tab-separated, each value as it is (a node as its number).
    fn line(&self, i: usize) -> String {
        if self.types.is_empty() {
            return String::new();
        }
        (0..self.width()).map(|c| match self.cell(i, c) { Cell::Int(v) => v.to_string(), Cell::Text(s) => s.to_owned() }).collect::<Vec<_>>().join("\t")
    }
    /// Its rows as the run's text outputs were: each a line, sorted, each once.
    pub fn lines(&self) -> Vec<String> {
        let mut lines: Vec<String> = (0..self.len()).map(|i| self.line(i)).collect();
        lines.sort_unstable();
        lines.dedup();
        lines
    }
}

/// One image of a stack, viewed in place: its bytes at 16-byte alignment and its region table.
pub struct Image {
    buf: std::sync::Arc<Vec<u128>>,
    /// Where this image starts in `buf`, in bytes, and its length.
    at: usize,
    len: usize,
    regions: [(usize, usize); REGIONS],
}

/// A relation of an image.
#[derive(Clone, Copy)]
pub struct Rel<'a> {
    image: &'a Image,
    pub width: usize,
    pub rows: usize,
    cols: usize,
    types: usize,
}

impl<'a> Rel<'a> {
    pub fn types(&self) -> &'a [u8] {
        &self.image.u8s(R::Types)[self.types..self.types + self.width]
    }
    /// Column `c`'s values, in the rows' order.
    pub fn col(&self, c: usize) -> &'a [i64] {
        let at = self.cols + c * self.rows;
        &self.image.i64s(R::Cols)[at..at + self.rows]
    }
    /// Row `i`'s values.
    pub fn row(&self, i: usize) -> Vec<i64> {
        (0..self.width).map(|c| self.col(c)[i]).collect()
    }
    /// Row `i`'s value in column `c` as the `.csv` file showed it.
    pub fn shown(&self, i: usize, c: usize) -> String {
        let v = self.col(c)[i];
        match self.types()[c] {
            col::TEXT => self.image.string(v as u32).to_owned(),
            col::NODE => self.image.node(v),
            _ => v.to_string(),
        }
    }
    /// Its rows as the `.csv` file's lines: each tab-separated, sorted.
    pub fn lines(&self) -> Vec<String> {
        if self.width == 0 {
            return vec![String::new(); self.rows];
        }
        let mut lines: Vec<String> = (0..self.rows).map(|i| (0..self.width).map(|c| self.shown(i, c)).collect::<Vec<_>>().join("\t")).collect();
        lines.sort_unstable();
        lines.dedup();
        lines
    }
}

impl Image {
    /// The images of `bytes`, a stack: each checked, the last first in its reading.
    pub fn stack(bytes: &[u8]) -> Result<Vec<Image>, String> {
        let mut words = vec![0u128; bytes.len().div_ceil(16)];
        // SAFETY: `words` holds at least `bytes.len()` bytes.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), words.as_mut_ptr().cast::<u8>(), bytes.len()) };
        let buf = std::sync::Arc::new(words);
        let mut out = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            let image = Image::at(&buf, at, bytes.len())?;
            if out.is_empty() == image.is_overlay() {
                return Err(bad(if out.is_empty() { "a stack starts with an overlay" } else { "a base inside a stack" }));
            }
            at += image.len;
            out.push(image);
        }
        Ok(out)
    }
    /// One image held in `words` (its first `len` bytes), checked and read in place: no copy.
    pub fn own(words: Vec<u128>, len: usize) -> Result<Image, String> {
        if len > words.len() * 16 {
            return Err(bad("an image longer than its buffer"));
        }
        Image::at(&std::sync::Arc::new(words), 0, len)
    }
    /// One image, base or overlay, read alone (the image a run just made).
    pub fn one(bytes: &[u8]) -> Result<Image, String> {
        let mut words = vec![0u128; bytes.len().div_ceil(16)];
        // SAFETY: `words` holds at least `bytes.len()` bytes.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), words.as_mut_ptr().cast::<u8>(), bytes.len()) };
        Image::at(&std::sync::Arc::new(words), 0, bytes.len())
    }
    fn at(buf: &std::sync::Arc<Vec<u128>>, at: usize, total: usize) -> Result<Image, String> {
        // SAFETY: `buf` holds at least `total` bytes.
        let all = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), total) };
        let b = &all[at..];
        if b.len() < HEADER || b[..8] != MAGIC {
            return Err(bad("no image"));
        }
        let word = |i: usize| u32::from_le_bytes(b[i..i + 4].try_into().expect("four bytes"));
        let long = |i: usize| u64::from_le_bytes(b[i..i + 8].try_into().expect("eight bytes"));
        if word(8) != FORMAT {
            return Err(bad(format!("format {}, not {FORMAT}", word(8))));
        }
        if word(12) as usize != REGIONS {
            return Err(bad(format!("{} regions, not {REGIONS}", word(12))));
        }
        let mut regions = [(0, 0); REGIONS];
        let mut end = HEADER;
        for (r, region) in regions.iter_mut().enumerate() {
            let i = 16 + 24 * r;
            let (width, count, offset) = (word(i), long(i + 8) as usize, long(i + 16) as usize);
            if width != WIDTH[r] {
                return Err(bad(format!("region {r} is {width} bytes wide, not {}", WIDTH[r])));
            }
            let e = count.checked_mul(width as usize).and_then(|n| n.checked_add(offset));
            if offset % 16 != 0 || offset < HEADER || e.is_none_or(|e| e > b.len()) {
                return Err(bad(format!("region {r} out of bounds")));
            }
            end = end.max(e.expect("checked"));
            *region = (at + offset, count);
        }
        let image = Image { buf: std::sync::Arc::clone(buf), at, len: end.next_multiple_of(16).min(b.len()), regions };
        image.check()?;
        Ok(image)
    }
    fn check(&self) -> Result<(), String> {
        if self.i64s(R::Meta).len() != meta::WORDS {
            return Err(bad("no meta"));
        }
        let ends = self.u32s(R::Ends);
        if ends.windows(2).any(|w| w[0] > w[1]) || ends.last().is_some_and(|&e| e as usize != self.u8s(R::Bytes).len()) || std::str::from_utf8(self.u8s(R::Bytes)).is_err() {
            return Err(bad("strings"));
        }
        let strings = ends.len() as u64;
        let (cols, types) = (self.i64s(R::Cols).len() as u64, self.u8s(R::Types).len() as u64);
        for r in self.u64s(R::Rels).chunks_exact(5) {
            let [name, width, rows, col_at, type_at] = [r[0], r[1], r[2], r[3], r[4]];
            if name >= strings || type_at + width > types || col_at + width * rows > cols {
                return Err(bad("a relation out of bounds"));
            }
        }
        for (i, r) in self.u64s(R::Rels).chunks_exact(5).enumerate() {
            let rel = self.rel_at(i);
            for c in 0..rel.width {
                match rel.types()[c] {
                    col::TEXT if rel.col(c).iter().any(|&v| v < 0 || v as u64 >= strings) => return Err(bad(format!("relation {} column {c}: no such string", r[0]))),
                    col::TEXT | col::NODE | col::INT => {}
                    t => return Err(bad(format!("column type {t}"))),
                }
            }
        }
        for d in self.u64s(R::Docs).chunks_exact(3) {
            if d[1] != NONE && d[1] >= strings {
                return Err(bad("a document's path"));
            }
        }
        if self.u64s(R::Places).chunks_exact(2).any(|p| p[1] >= strings) {
            return Err(bad("a place"));
        }
        Ok(())
    }
    fn bytes_of(&self, r: R) -> (*const u8, usize) {
        let (offset, count) = self.regions[r as usize];
        // SAFETY: `at` checked the region lies in the buffer.
        (unsafe { self.buf.as_ptr().cast::<u8>().add(offset) }, count)
    }
    fn slice<T>(&self, r: R) -> &[T] {
        let (p, count) = self.bytes_of(r);
        // An item may be several `T`s (a relation is five words).
        let len = count * WIDTH[r as usize] as usize / std::mem::size_of::<T>();
        // SAFETY: the region's bounds and width were checked; its offset is 16-byte aligned in a 16-byte-aligned
        // buffer (every image starts at a multiple of 16); every T is a plain integer.
        unsafe { std::slice::from_raw_parts(p.cast::<T>(), len) }
    }
    fn u8s(&self, r: R) -> &[u8] {
        self.slice(r)
    }
    fn u32s(&self, r: R) -> &[u32] {
        self.slice(r)
    }
    fn u64s(&self, r: R) -> &[u64] {
        self.slice(r)
    }
    fn i64s(&self, r: R) -> &[i64] {
        self.slice(r)
    }
    pub fn is_overlay(&self) -> bool {
        self.i64s(R::Meta)[meta::KIND] as u64 == meta::OVERLAY
    }
    pub fn flags(&self) -> u64 {
        self.i64s(R::Meta)[meta::FLAGS] as u64
    }
    /// The image's bytes.
    pub fn bytes(&self) -> &[u8] {
        // SAFETY: the image lies in the buffer.
        unsafe { std::slice::from_raw_parts(self.buf.as_ptr().cast::<u8>().add(self.at), self.len) }
    }
    /// String `i`.
    pub fn string(&self, i: u32) -> &str {
        let ends = self.u32s(R::Ends);
        let start = if i == 0 { 0 } else { ends[i as usize - 1] as usize };
        let b = &self.u8s(R::Bytes)[start..ends[i as usize] as usize];
        // SAFETY: `check` found the bytes UTF-8, and every end is a string's end as `Builder::text` wrote it.
        unsafe { std::str::from_utf8_unchecked(b) }
    }
    fn rel_at(&self, i: usize) -> Rel<'_> {
        let r = &self.u64s(R::Rels)[i * 5..i * 5 + 5];
        Rel { image: self, width: r[1] as usize, rows: r[2] as usize, cols: r[3] as usize, types: r[4] as usize }
    }
    /// The relations it holds, by name.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.u64s(R::Rels).chunks_exact(5).map(|r| self.string(r[0] as u32))
    }
    /// The relations it holds, with their names, in the order they were put.
    pub fn relations(&self) -> impl Iterator<Item = (&str, Rel<'_>)> {
        (0..self.u64s(R::Rels).len() / 5).map(|i| (self.string(self.u64s(R::Rels)[i * 5] as u32), self.rel_at(i)))
    }
    /// How many strings it holds.
    pub fn strings(&self) -> usize {
        self.u32s(R::Ends).len()
    }
    /// Relation `name`, if it holds it.
    pub fn rel(&self, name: &str) -> Option<Rel<'_>> {
        self.names().position(|n| n == name).map(|i| self.rel_at(i))
    }
    /// How the `@place` value `v` is shown, if the image holds it.
    pub fn place_shown(&self, v: i64) -> Option<&str> {
        let places = self.u64s(R::Places);
        places.chunks_exact(2).position(|p| p[0] as i64 == v).map(|i| self.string(places[i * 2 + 1] as u32))
    }
    /// Document `key`'s path and first node, as raw words (`NONE` where it has none); both `NONE` where the image
    /// holds no such document.
    fn doc_words(&self, key: i64) -> (u64, u64) {
        let docs = self.u64s(R::Docs);
        let (mut lo, mut hi) = (0, docs.len() / 3);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if (docs[mid * 3] as i64) < key { lo = mid + 1 } else { hi = mid }
        }
        if lo < docs.len() / 3 && docs[lo * 3] as i64 == key { (docs[lo * 3 + 1], docs[lo * 3 + 2]) } else { (NONE, NONE) }
    }
    /// Document `key`, if the image holds it.
    pub fn doc(&self, key: i64) -> Option<Doc> {
        let (path, first) = self.doc_words(key);
        (path != NONE || first != NONE).then(|| Doc { path: (path != NONE).then(|| self.string(path as u32).to_owned()), first: (first != NONE).then_some(first as i64) })
    }
    /// How a node column's value is shown, as the `.csv` writer showed it.
    pub fn node(&self, v: i64) -> String {
        if v < -1 {
            return self.place_shown(v).map_or_else(|| v.to_string(), str::to_owned);
        }
        if v < 0 {
            return v.to_string();
        }
        let (doc, local) = (v >> 32, v & 0xffff_ffff);
        let (path, first) = self.doc_words(doc);
        let flags = self.flags();
        let own = flags & flags::OWN != 0;
        if path == NONE && first == NONE {
            format!("guest{doc}:{local}")
        } else if path != NONE && (own || flags & flags::DEFINITION_IDS != 0) {
            format!("{}:{}", self.string(path as u32), shown_local(local, flags & flags::DEFINITION_IDS != 0))
        } else if !own && first != NONE {
            (first as i64 + local).to_string()
        } else {
            v.to_string()
        }
    }
}

/// A node's local part as shown: by definition `slot.index`, or its preorder index (`syntax::shown_local`).
fn shown_local(low: i64, by_definition: bool) -> String {
    use crate::code_rule_dsl::facts::vtab::syntax::granules::INDEX_BITS;
    if by_definition { format!("{}.{}", low >> INDEX_BITS, low & ((1 << INDEX_BITS) - 1)) } else { low.to_string() }
}

/// A stack's relation `name`: from the last image that holds it.
pub fn find<'a>(stack: &'a [Image], name: &str) -> Option<Rel<'a>> {
    stack.iter().rev().find_map(|i| i.rel(name))
}

/// The stack in `dir`'s output file, or none.
pub fn read(dir: &Path) -> Result<Vec<Image>, String> {
    match std::fs::read(dir.join(FILE)) {
        Ok(bytes) => Image::stack(&bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(bad(e)),
    }
}

/// `image` written to `dir`'s output file: as its whole stack when a base, or appended when an overlay (a stack
/// already `depth` images deep is folded by its caller instead).
pub fn write(dir: &Path, image: &[u8]) -> std::io::Result<()> {
    let path = dir.join(FILE);
    // Its kind is its meta region's first word (`meta::KIND`); the meta region's offset is the header's first entry.
    let word = |at: usize| image.get(at..at + 8).map(|b| u64::from_le_bytes(b.try_into().expect("eight bytes")));
    let overlay = image.starts_with(&MAGIC) && word(16 + 16).and_then(|off| word(off as usize)) == Some(meta::OVERLAY);
    if overlay {
        use std::io::Write;
        std::fs::OpenOptions::new().append(true).open(path)?.write_all(image)
    } else {
        std::fs::write(path, image)
    }
}

#[cfg(test)]
mod tests;
