//! A program's output as a flat image (decision 0239): a fixed set of regions of fixed-width items, every reference
//! an index, read where its bytes are.
//!
//! ```text
//! header   magic, format, region count, then per region: item width, item count, byte offset (16-byte aligned)
//! meta     what the image is: a base or an overlay, how its nodes are shown
//! rels     per relation entry: its name (a string), its width, its rows, where its types and columns start, its kind
//! types    a byte per column: text, node, int
//! cols     per relation, per column, its rows' values (i64); the rows sorted by their values, each once
//! bytes    the strings' bytes
//! ends     each string's end in `bytes`
//! docs     per document a node names: its key, its path (a string, or none), its first node (or none); by key
//! places   per `@place` value a node column holds: the value, its path (a string) and its line; by value
//! ```
//!
//! A text column holds strings' indexes, a node column `doc << 32 | local`, an int column the integer. A string is
//! held as its text, once per image, so an image means the same in every process (decision 0212).
//!
//! **A stack.** A run's outputs are a file of images, one after another: a base, then overlays. An overlay holds
//! what a later run changed: a relation whole (`kind::WHOLE`), or its rows added (`kind::ADDED`) and the hashes of its
//! rows gone (`kind::GONE`, [`Rel::hash`]). A relation is read as a [`View`]: from the last image that holds it
//! whole, through each later image's rows gone and added. Each image's strings, documents and places are its own. A
//! stack is folded into one base at [`MAX_DEPTH`] images.
//!
//! Text is a reader: [`lines`] and [`text`] give a relation's rows as the lines its `.csv` file held, sorted; [`to_csv`]
//! writes the files for scripts that read them.

use std::collections::HashMap;
use std::path::Path;

pub const MAGIC: [u8; 8] = *b"RROUT\x00\x00\x01";
pub const FORMAT: u32 = 3;
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

/// A node or int column's value that is none (SQL's NULL), shown `None`: a value no node or place takes.
pub const NULL: i64 = i64::MIN;

/// A relation entry's kind.
pub mod kind {
    /// The relation's rows, all of them.
    pub const WHOLE: u64 = 0;
    /// Rows added to the relation as the images before read it.
    pub const ADDED: u64 = 1;
    /// The hashes of rows gone from it (`Rel::hash`): two int columns, the hash's low and high words.
    pub const GONE: u64 = 2;
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
const WIDTH: [u32; REGIONS] = [8, 48, 1, 8, 1, 4, 24, 24];
/// The words of a relation entry.
const REL_WORDS: usize = 6;
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
    rels: Vec<(u32, Vec<u8>, usize, Vec<i64>, u64)>,
    docs: std::collections::BTreeMap<i64, (u64, u64)>,
    places: std::collections::BTreeMap<i64, (u64, i64)>,
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
    /// A `@place` value a node column holds: its path and line.
    pub fn place(&mut self, value: i64, path: &str, line: i64) {
        let s = u64::from(self.text(path));
        self.places.insert(value, (s, line));
    }
    /// A relation of `types` from its rows, flat (text columns as string indexes): sorted, each row once.
    pub fn rows(&mut self, name: &str, types: &[u8], rows: Vec<i64>) {
        self.entry(name, types, rows, kind::WHOLE);
    }
    /// An entry of `kind` from its rows, flat: sorted, each row once.
    fn entry(&mut self, name: &str, types: &[u8], mut rows: Vec<i64>, kind: u64) {
        let name = self.text(name);
        let k = types.len();
        if let Some(count) = rows.len().checked_div(k) {
            let mut order: Vec<usize> = (0..count).collect();
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
            self.rels.push((name, types.to_vec(), n, rows, kind));
        } else {
            self.rels.push((name, Vec::new(), usize::from(!rows.is_empty()), Vec::new(), kind));
        }
    }
    /// A relation of `types` from its rows, flat, in the order and number given (a memo entry's, decision 0240 §4:
    /// replayed as it was made). Readers of the run's outputs ask for `rows`, which sorts them.
    pub fn rows_as_is(&mut self, name: &str, types: &[u8], rows: Vec<i64>) {
        let name = self.text(name);
        let k = types.len();
        if k == 0 {
            self.rels.push((name, Vec::new(), 0, Vec::new(), kind::WHOLE));
            return;
        }
        let n = rows.len() / k;
        let mut cols = vec![0i64; n * k];
        for (i, row) in rows.chunks_exact(k).enumerate() {
            for (c, &v) in row.iter().enumerate() {
                cols[c * n + i] = v;
            }
        }
        self.rels.push((name, types.to_vec(), n, cols, kind::WHOLE));
    }
    /// A relation from the parts a run handed of it (`Rows`), their values as they are: a column is text where any
    /// part holds it as text (another part's integers there held as they are shown), a node column where `nodes`
    /// says or a part holds it as one. Every part is of one width.
    pub fn put(&mut self, name: &str, parts: &[&Rows], nodes: &[usize]) {
        let width = parts.iter().find(|p| !p.is_empty()).map_or_else(|| parts.first().map_or(0, |p| p.width()), |p| p.width());
        assert!(parts.iter().all(|p| p.is_empty() || p.width() == width), "{name}: parts of differing widths");
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
                        (col::TEXT, _) => *here[v as usize].get_or_insert_with(|| i64::from(self.text(p.text(v)))),
                        (_, col::TEXT) if v == NULL => i64::from(self.text("None")),
                        (_, col::TEXT) => i64::from(self.text(&v.to_string())),
                        _ => v,
                    });
                }
            }
        }
        self.rows(name, &types, rows);
    }
    /// A relation as a view reads it, copied whole: its strings held again here, its documents and places with them
    /// (a stack folded into one base).
    pub fn take(&mut self, name: &str, view: &View<'_>) {
        let rows = self.copied(view.parts.iter().map(|(r, kept)| (*r, kept.as_deref())));
        self.entry(name, view.types(), rows, kind::WHOLE);
    }
    /// Relations as views read them, copied whole into one relation `name` (their union; each row once).
    pub fn union(&mut self, name: &str, views: &[&View<'_>]) {
        let types = views.iter().map(|v| v.types()).find(|t| !t.is_empty()).unwrap_or(&[]).to_vec();
        let rows = self.copied(views.iter().flat_map(|v| v.parts.iter().map(|(r, kept)| (*r, kept.as_deref()))));
        self.entry(name, &types, rows, kind::WHOLE);
    }
    /// Rows of another stack's relations, copied whole into relation `name` (all of one type).
    pub fn rows_of(&mut self, name: &str, types: &[u8], rows: &[Row<'_>]) {
        let kept: Vec<(Rel<'_>, [usize; 1])> = rows.iter().map(|r| (r.rel, [r.i])).collect();
        let rows = self.copied(kept.iter().map(|(r, i)| (*r, Some(&i[..]))));
        self.entry(name, types, rows, kind::WHOLE);
    }
    /// Rows `kept` of `rel` (another image's), added to relation `name` as the images before this read it.
    pub fn added(&mut self, name: &str, rel: Rel<'_>, kept: &[usize]) {
        let rows = self.copied(std::iter::once((rel, Some(kept))));
        self.entry(name, rel.types(), rows, kind::ADDED);
    }
    /// The hashes of rows gone from relation `name` (`Rel::hash`).
    pub fn gone(&mut self, name: &str, hashes: &[u128]) {
        let rows: Vec<i64> = hashes.iter().flat_map(|&h| [h as u64 as i64, (h >> 64) as u64 as i64]).collect();
        self.entry(name, &[col::INT, col::INT], rows, kind::GONE);
    }
    /// Rows of other images' relations (all of one, or those `kept`), flat, their strings, documents and places held
    /// again here.
    fn copied<'a>(&mut self, parts: impl Iterator<Item = (Rel<'a>, Option<&'a [usize]>)>) -> Vec<i64> {
        let mut rows = Vec::new();
        for (rel, kept) in parts {
            let types = rel.types();
            let all: Vec<usize>;
            let kept = match kept {
                Some(k) => k,
                None => {
                    all = (0..rel.rows).collect();
                    &all
                }
            };
            if rel.width == 0 {
                rows.extend(kept.iter().take(1).map(|_| 0));
                continue;
            }
            for &i in kept {
                for (c, &t) in types.iter().enumerate() {
                    let v = rel.col(c)[i];
                    rows.push(match t {
                        col::TEXT => i64::from(self.text(rel.image.string(v as u32))),
                        col::NODE => {
                            if let Some((path, line)) = rel.image.place(v) {
                                let path = path.to_owned();
                                self.place(v, &path, line);
                            } else if v >= 0 && let Some(doc) = rel.image.doc(v >> 32) {
                                self.doc(v >> 32, &doc);
                            }
                            v
                        }
                        _ => v,
                    });
                }
            }
        }
        rows
    }
    /// Whether it holds no relation entry.
    pub fn is_empty(&self) -> bool {
        self.rels.is_empty()
    }
    /// The image's bytes.
    pub fn finish(self) -> Vec<u8> {
        let mut w: Vec<Vec<u8>> = vec![Vec::new(); REGIONS];
        let put = |w: &mut Vec<Vec<u8>>, r: R, v: u64| w[r as usize].extend_from_slice(&v.to_le_bytes());
        put(&mut w, R::Meta, if self.overlay { meta::OVERLAY } else { meta::BASE });
        put(&mut w, R::Meta, self.flags);
        let (mut col_at, mut type_at) = (0u64, 0u64);
        for (name, types, n, cols, kind) in &self.rels {
            for v in [u64::from(*name), types.len() as u64, *n as u64, col_at, type_at, *kind] {
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
        for (value, (path, line)) in &self.places {
            put(&mut w, R::Places, *value as u64);
            put(&mut w, R::Places, *path);
            put(&mut w, R::Places, *line as u64);
        }
        let mut out = vec![0u8; HEADER];
        out[..8].copy_from_slice(&MAGIC);
        out[8..12].copy_from_slice(&FORMAT.to_le_bytes());
        out[12..16].copy_from_slice(&(REGIONS as u32).to_le_bytes());
        for (r, bytes) in w.iter().enumerate() {
            while !out.len().is_multiple_of(16) {
                out.push(0);
            }
            let at = 16 + 24 * r;
            let offset = out.len() as u64;
            out[at..at + 4].copy_from_slice(&WIDTH[r].to_le_bytes());
            out[at + 8..at + 16].copy_from_slice(&((bytes.len() / WIDTH[r] as usize) as u64).to_le_bytes());
            out[at + 16..at + 24].copy_from_slice(&offset.to_le_bytes());
            out.extend_from_slice(bytes);
        }
        while !out.len().is_multiple_of(16) {
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
}

impl Rows {
    /// No rows, of columns typed `types`.
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
    /// Its texts, by index (a text column's values index them).
    pub fn texts(&self) -> &[Box<str>] {
        &self.texts
    }
    /// The rows of their parts as `texts` and `row` give them (a memo entry's): columns typed `types`, `count` rows
    /// of values flat, a text column's each an index of `texts`.
    pub fn of_parts(types: Vec<u8>, vals: Vec<i64>, texts: Vec<String>, count: usize) -> Result<Rows, String> {
        let k = types.len();
        if (k == 0 && !vals.is_empty()) || (k > 0 && vals.len() != k * count) {
            return Err(format!("{} values, not {count} rows of {k}", vals.len()));
        }
        if let Some(c) = (0..k).find(|&c| types[c] == col::TEXT && (0..count).any(|i| !(0..texts.len() as i64).contains(&vals[i * k + c]))) {
            return Err(format!("a text of column {c} out of its texts"));
        }
        let texts: Vec<Box<str>> = texts.into_iter().map(String::into_boxed_str).collect();
        let ids = texts.iter().enumerate().map(|(i, t)| (t.clone(), i as u32)).collect();
        Ok(Rows { types, vals, texts, ids, bare: if k == 0 { count } else { 0 } })
    }
    /// A row appended: of its columns' width and types (a text column may take an integer, held as it is shown; an
    /// integer column no text: that is a fault of the run that made the row).
    pub fn push(&mut self, row: &[Cell<'_>]) {
        assert!(row.len() == self.types.len() || self.is_empty_shape(), "a row of {} values in rows of {}", row.len(), self.types.len());
        if self.types.len() != row.len() {
            // The first row of a relation typed with no columns: its columns are this row's, ints until a text.
            self.types = row.iter().map(|c| if matches!(c, Cell::Int(_)) { col::INT } else { col::TEXT }).collect();
        }
        if row.is_empty() {
            self.bare += 1;
            return;
        }
        for (c, cell) in row.iter().enumerate() {
            let v = match (*cell, self.types[c]) {
                (Cell::Text(s), col::TEXT) => self.text_id(s),
                (Cell::Text(s), _) => panic!("a text ({s:?}) in integer column {c}"),
                (Cell::Int(NULL), col::TEXT) => self.text_id("None"),
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
        let mut cells = Vec::new();
        for i in 0..self.len() {
            cells.clear();
            cells.extend((1..self.width()).map(|c| self.cell(i, c)));
            out.push(&cells);
        }
        out
    }
    /// Its rows, each value as shown (a node as its number): sorted, each once. For comparing rows, never read back.
    pub fn shown(&self) -> Vec<Vec<String>> {
        let mut rows: Vec<Vec<String>> = (0..self.len()).map(|i| (0..self.width()).map(|c| match self.cell(i, c) { Cell::Int(NULL) => "None".to_owned(), Cell::Int(v) => v.to_string(), Cell::Text(s) => s.to_owned() }).collect()).collect();
        rows.sort_unstable();
        rows.dedup();
        rows
    }
}

/// Rows are equal where their columns' types are and their rows are, value for value, in order.
impl PartialEq for Rows {
    fn eq(&self, other: &Rows) -> bool {
        self.types == other.types && self.len() == other.len() && (0..self.len()).all(|i| (0..self.width()).all(|c| self.cell(i, c) == other.cell(i, c)))
    }
}

impl Eq for Rows {}

/// One image of a stack, viewed in place: its bytes at 16-byte alignment and its region table.
pub struct Image {
    buf: std::sync::Arc<Backing>,
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
    pub kind: u64,
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
    /// Row `i`'s value in column `c`.
    pub fn value(&self, i: usize, c: usize) -> Value<'a> {
        let v = self.col(c)[i];
        match self.types()[c] {
            col::TEXT => Value::Text(self.image.string(v as u32)),
            _ if v == NULL => Value::Null,
            col::NODE => Value::Node(self.image.node(v)),
            _ => Value::Int(v),
        }
    }
    /// Row `i`'s value in column `c` as the `.csv` file showed it (a renderer).
    pub fn shown(&self, i: usize, c: usize) -> String {
        self.value(i, c).to_string()
    }
    /// Row `i`'s hash, by what it shows: a text by its bytes, an int by its value, a node by its document's path and
    /// first node and its local part (a `@place` by how it is shown, a guest's by its value), each with its type. The
    /// same in every image and every process: an overlay names the rows gone by it.
    pub fn hash(&self, i: usize) -> u128 {
        let mut h = Hash::new(self.width as u64);
        for c in 0..self.width {
            self.cell_hash(&mut h, c, i, &mut Seen::default());
        }
        h.finish()
    }
    /// Every row's hash (`hash`), in the rows' order: each string and document hashed once.
    pub fn each_hash(&self) -> Vec<u128> {
        if self.width == 0 {
            return vec![0; self.rows];
        }
        let mut seen = Seen { texts: vec![0; self.image.u32s(R::Ends).len()], docs: HashMap::new() };
        (0..self.rows)
            .map(|i| {
                let mut h = Hash::new(self.width as u64);
                for c in 0..self.width {
                    self.cell_hash(&mut h, c, i, &mut seen);
                }
                h.finish()
            })
            .collect()
    }
    /// Cell (`i`, `c`)'s words: its type, then a text's or a document's 64-bit hash (`Seen`), or its value.
    fn cell_hash(&self, h: &mut Hash, c: usize, i: usize, seen: &mut Seen) {
        let t = self.types()[c];
        let v = self.col(c)[i];
        h.word(u64::from(t));
        match t {
            col::TEXT => h.word(seen.text(self.image, v as u32)),
            col::NODE if v < -1 => match self.image.u64s(R::Places).as_chunks::<3>().0.iter().find(|p| p[0] as i64 == v) {
                Some(p) => {
                    h.word(seen.text(self.image, p[1] as u32));
                    h.word(p[2]);
                }
                None => h.word(v as u64),
            },
            col::NODE if v >= 0 => {
                let (path, first) = self.image.doc_words(v >> 32);
                if path == NONE && first == NONE {
                    // A guest's node (no such document): its value.
                    h.word(NONE);
                    h.word(v as u64);
                } else {
                    h.word(seen.doc(self.image, v >> 32, path, first));
                    h.word((v & 0xffff_ffff) as u64);
                }
            }
            _ => h.word(v as u64),
        }
    }
    /// Its rows' hashes, sorted, each once.
    pub fn hashes(&self) -> Vec<u128> {
        let mut h = self.each_hash();
        h.sort_unstable();
        h.dedup();
        h
    }
}

/// The bytes an image is read from: its file, mapped, or a copy in memory (an image a run just made; a file where
/// mapping is not had, as on WASI). Either starts 16-byte aligned (a map at a page).
enum Backing {
    Mapped(memmap2::Mmap),
    Owned(Vec<u128>, usize),
}

impl Backing {
    fn copied(bytes: &[u8]) -> Backing {
        let mut words = vec![0u128; bytes.len().div_ceil(16)];
        // SAFETY: `words` holds at least `bytes.len()` bytes.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), words.as_mut_ptr().cast::<u8>(), bytes.len()) };
        Backing::Owned(words, bytes.len())
    }
    fn bytes(&self) -> &[u8] {
        match self {
            Backing::Mapped(m) => m,
            // SAFETY: the words hold `len` bytes.
            Backing::Owned(words, len) => unsafe { std::slice::from_raw_parts(words.as_ptr().cast::<u8>(), *len) },
        }
    }
}

impl Image {
    /// The images of `bytes`, a stack: each checked, the last first in its reading.
    pub fn stack(bytes: &[u8]) -> Result<Vec<Image>, String> {
        Image::stack_of(std::sync::Arc::new(Backing::copied(bytes)))
    }
    /// The stack in `file`, mapped: read where its bytes are. The file is never written in place (`write` puts a
    /// whole new file in its place), so a map stays as it was while the stack is read.
    pub fn mapped(file: &Path) -> std::io::Result<Vec<Image>> {
        let f = std::fs::File::open(file)?;
        // SAFETY: no writer changes the file in place: each write renames a new file over it (`write`).
        let backing = match unsafe { memmap2::Mmap::map(&f) } {
            Ok(m) => Backing::Mapped(m),
            Err(_) => Backing::copied(&std::fs::read(file)?),
        };
        Image::stack_of(std::sync::Arc::new(backing)).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{}: {e}", file.display())))
    }
    fn stack_of(buf: std::sync::Arc<Backing>) -> Result<Vec<Image>, String> {
        let total = buf.bytes().len();
        let mut out = Vec::new();
        let mut at = 0;
        while at < total {
            let image = Image::at(&buf, at, total)?;
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
        Image::at(&std::sync::Arc::new(Backing::Owned(words, len)), 0, len)
    }
    /// One image, base or overlay, read alone (the image a run just made).
    pub fn one(bytes: &[u8]) -> Result<Image, String> {
        Image::at(&std::sync::Arc::new(Backing::copied(bytes)), 0, bytes.len())
    }
    fn at(buf: &std::sync::Arc<Backing>, at: usize, total: usize) -> Result<Image, String> {
        let all = &buf.bytes()[..total];
        if !(all.as_ptr() as usize).is_multiple_of(16) {
            return Err(bad("an image not 16-byte aligned"));
        }
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
        for r in self.u64s(R::Rels).as_chunks::<REL_WORDS>().0 {
            let [name, width, rows, col_at, type_at, k] = [r[0], r[1], r[2], r[3], r[4], r[5]];
            if name >= strings || type_at + width > types || col_at + width * rows > cols {
                return Err(bad("a relation out of bounds"));
            }
            if k > kind::GONE || (k == kind::GONE && width != 2) || (k != kind::WHOLE && !self.is_overlay()) {
                return Err(bad(format!("a relation entry of kind {k}")));
            }
        }
        for (i, r) in self.u64s(R::Rels).as_chunks::<REL_WORDS>().0.iter().enumerate() {
            let rel = self.rel_at(i);
            for c in 0..rel.width {
                match rel.types()[c] {
                    col::TEXT if rel.col(c).iter().any(|&v| v < 0 || v as u64 >= strings) => return Err(bad(format!("relation {} column {c}: no such string", r[0]))),
                    col::TEXT | col::NODE | col::INT => {}
                    t => return Err(bad(format!("column type {t}"))),
                }
            }
        }
        for d in self.u64s(R::Docs).as_chunks::<3>().0 {
            if d[1] != NONE && d[1] >= strings {
                return Err(bad("a document's path"));
            }
        }
        if self.u64s(R::Places).as_chunks::<3>().0.iter().any(|p| p[1] >= strings) {
            return Err(bad("a place"));
        }
        Ok(())
    }
    fn bytes_of(&self, r: R) -> (*const u8, usize) {
        let (offset, count) = self.regions[r as usize];
        // SAFETY: `at` checked the region lies in the buffer.
        (unsafe { self.buf.bytes().as_ptr().add(offset) }, count)
    }
    fn slice<T>(&self, r: R) -> &[T] {
        let (p, count) = self.bytes_of(r);
        // An item may be several `T`s (a relation entry is six words).
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
        &self.buf.bytes()[self.at..self.at + self.len]
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
        let r = &self.u64s(R::Rels)[i * REL_WORDS..(i + 1) * REL_WORDS];
        Rel { image: self, width: r[1] as usize, rows: r[2] as usize, cols: r[3] as usize, types: r[4] as usize, kind: r[5] }
    }
    /// The relations it holds an entry of, by name (a relation of both added and gone rows twice).
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.u64s(R::Rels).as_chunks::<REL_WORDS>().0.iter().map(|r| self.string(r[0] as u32))
    }
    /// The relations it holds, with their names, in the order they were put.
    pub fn relations(&self) -> impl Iterator<Item = (&str, Rel<'_>)> {
        (0..self.u64s(R::Rels).len() / REL_WORDS).map(|i| (self.string(self.u64s(R::Rels)[i * REL_WORDS] as u32), self.rel_at(i)))
    }
    /// How many strings it holds.
    pub fn strings(&self) -> usize {
        self.u32s(R::Ends).len()
    }
    /// Relation `name`, if it holds it whole.
    pub fn rel(&self, name: &str) -> Option<Rel<'_>> {
        self.entry(name, kind::WHOLE)
    }
    /// Its entry of `kind` for relation `name`, if it holds one.
    pub fn entry(&self, name: &str, k: u64) -> Option<Rel<'_>> {
        (0..self.u64s(R::Rels).len() / REL_WORDS).map(|i| self.rel_at(i)).zip(self.names()).find(|(r, n)| r.kind == k && *n == name).map(|(r, _)| r)
    }
    /// The `@place` value `v`'s path and line, if the image holds it (`places` is sorted by value).
    pub fn place(&self, v: i64) -> Option<(&str, i64)> {
        let places = self.u64s(R::Places);
        let n = places.len() / 3;
        let (mut lo, mut hi) = (0, n);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if (places[mid * 3] as i64) < v { lo = mid + 1 } else { hi = mid }
        }
        (lo < n && places[lo * 3] as i64 == v).then(|| (self.string(places[lo * 3 + 1] as u32), places[lo * 3 + 2] as i64))
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
    /// A node column's value `v`, as a reader sees it.
    pub fn node(&self, v: i64) -> Node<'_> {
        if v < -1 {
            return match self.place(v) {
                Some((path, line)) => Node::Place { path, line },
                None => Node::Other(v),
            };
        }
        if v < 0 {
            return Node::Other(v);
        }
        let (doc, local) = (v >> 32, v & 0xffff_ffff);
        let (path, first) = self.doc_words(doc);
        if path == NONE && first == NONE {
            return Node::Guest { doc, local };
        }
        let flags = self.flags();
        Node::Of {
            path: (path != NONE).then(|| self.string(path as u32)),
            first: (first != NONE).then_some(first as i64),
            local,
            value: v,
            own: flags & flags::OWN != 0,
            by_definition: flags & flags::DEFINITION_IDS != 0,
        }
    }
}

/// A value of a relation's row, as a reader sees it: typed, nothing to parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value<'a> {
    Text(&'a str),
    Int(i64),
    Node(Node<'a>),
    /// None (SQL's NULL), in a node or int column.
    Null,
}

/// A node column's value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Node<'a> {
    /// A node of a document the run read: its path (a file of the run's), its first node in T's numbering, its local
    /// part (preorder index, or `slot << INDEX_BITS | index` by definition), its value (`doc << 32 | local`), and
    /// how the run shows it.
    Of { path: Option<&'a str>, first: Option<i64>, local: i64, value: i64, own: bool, by_definition: bool },
    /// A guest's node (decision 0055): its document is no file of the run.
    Guest { doc: i64, local: i64 },
    /// A `@place`: a path and a line.
    Place { path: &'a str, line: i64 },
    /// No node: `unset` (-1), or a value the image does not name.
    Other(i64),
}

impl<'a> Node<'a> {
    /// The node as a number, where the run numbers it: T's (its first node plus its local part) for a run of T's
    /// documents, else its value. None for a node shown by its path, a guest's or a place.
    pub fn number(&self) -> Option<i64> {
        match *self {
            Node::Of { path, first, local, value, own, by_definition } => {
                if path.is_some() && (own || by_definition) {
                    None
                } else if !own && let Some(f) = first {
                    Some(f + local)
                } else {
                    Some(value)
                }
            }
            Node::Other(v) => Some(v),
            Node::Guest { .. } | Node::Place { .. } => None,
        }
    }
    /// Its document's path and its local part, for a node shown by them.
    pub fn at(&self) -> Option<(&'a str, i64)> {
        match *self {
            Node::Of { path: Some(p), local, .. } => Some((p, local)),
            _ => None,
        }
    }
}

impl<'a> Value<'a> {
    /// An int, or a node shown as a number (`Node::number`).
    pub fn int(&self) -> Option<i64> {
        match self {
            Value::Int(v) => Some(*v),
            Value::Node(n) => n.number(),
            Value::Text(_) | Value::Null => None,
        }
    }
    /// A text, held as long as the image is.
    pub fn text(self) -> Option<&'a str> {
        match self {
            Value::Text(t) => Some(t),
            _ => None,
        }
    }
    /// A node's document's path, or a place's.
    pub fn path(self) -> Option<&'a str> {
        match self {
            Value::Node(Node::Of { path, .. }) => path,
            Value::Node(Node::Place { path, .. }) => Some(path),
            _ => None,
        }
    }
    /// A node.
    pub fn node(self) -> Option<Node<'a>> {
        match self {
            Value::Node(n) => Some(n),
            _ => None,
        }
    }
}

/// How a value is shown (what the `.csv` files held; `rerail-engine csv`): a renderer, never read back.
impl std::fmt::Display for Value<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Text(t) => f.write_str(t),
            Value::Int(v) => write!(f, "{v}"),
            Value::Node(n) => write!(f, "{n}"),
            Value::Null => f.write_str("None"),
        }
    }
}

impl std::fmt::Display for Node<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Node::Of { path: Some(p), local, own, by_definition, .. } if own || by_definition => write!(f, "{p}:{}", shown_local(local, by_definition)),
            Node::Guest { doc, local } => write!(f, "guest{doc}:{local}"),
            Node::Place { path, line } => write!(f, "@place({}, {line})", crate::code_rule_dsl::logic::rr::quote(path)),
            n => write!(f, "{}", n.number().expect("a node shown as a number")),
        }
    }
}

/// A node's local part as shown: by definition `slot.index`, or its preorder index (`syntax::shown_local`).
fn shown_local(low: i64, by_definition: bool) -> String {
    use crate::code_rule_dsl::facts::vtab::syntax::granules::INDEX_BITS;
    if by_definition { format!("{}.{}", low >> INDEX_BITS, low & ((1 << INDEX_BITS) - 1)) } else { low.to_string() }
}

/// A run's outputs as a reader reads them: the stack in its directory, mapped. Every reader reads through it.
pub struct Outputs {
    stack: Vec<Image>,
}

impl Outputs {
    /// The outputs in `dir` (none where it holds no output file).
    pub fn open(dir: &Path) -> std::io::Result<Outputs> {
        match Image::mapped(&dir.join(FILE)) {
            Ok(stack) => Ok(Outputs { stack }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Outputs { stack: Vec::new() }),
            Err(e) => Err(e),
        }
    }
    /// The outputs of a stack's bytes (an image made in memory, a record's copy).
    pub fn of(bytes: &[u8]) -> Result<Outputs, String> {
        Image::stack(bytes).map(|stack| Outputs { stack })
    }
    /// The relations it holds, by name, each once.
    pub fn names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.stack.iter().flat_map(Image::names).collect();
        names.sort_unstable();
        names.dedup();
        names.retain(|n| find(&self.stack, n).is_some());
        names
    }
    /// Relation `name`, if it holds it.
    pub fn get(&self, name: &str) -> Option<Relation<'_>> {
        find(&self.stack, name).map(|view| Relation { view })
    }
    /// Relation `name`'s rows, none where it holds no such relation.
    pub fn rows(&self, name: &str) -> Vec<Row<'_>> {
        self.get(name).map(|r| r.rows().collect()).unwrap_or_default()
    }
    /// The stack's images.
    pub fn images(&self) -> &[Image] {
        &self.stack
    }
}

/// A relation of a run's outputs.
pub struct Relation<'a> {
    view: View<'a>,
}

impl<'a> Relation<'a> {
    pub fn types(&self) -> &[u8] {
        self.view.types()
    }
    pub fn width(&self) -> usize {
        self.view.types().len()
    }
    /// Its rows' count (a relation of no columns: one where it holds, none where not).
    pub fn len(&self) -> usize {
        if self.width() == 0 { self.view.len().min(1) } else { self.view.len() }
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Its rows: each once, in no order a reader may count on.
    pub fn rows(&self) -> impl Iterator<Item = Row<'a>> + '_ {
        let bare = self.width() == 0;
        self.view.each().take(if bare { 1 } else { usize::MAX }).map(|(rel, i)| Row { rel, i })
    }
    /// The view it reads (for copying it into another image).
    pub fn view(&self) -> &View<'a> {
        &self.view
    }
    /// Its rows, each value as shown (`Value`'s Display): sorted, each once. For comparing and showing rows, never
    /// for reading a value back.
    pub fn shown(&self) -> Vec<Vec<String>> {
        let mut rows: Vec<Vec<String>> = self.rows().map(|r| r.values().map(|v| v.to_string()).collect()).collect();
        rows.sort_unstable();
        rows.dedup();
        rows
    }
}

/// A row of a relation.
#[derive(Clone, Copy)]
pub struct Row<'a> {
    rel: Rel<'a>,
    i: usize,
}

impl<'a> Row<'a> {
    pub fn len(&self) -> usize {
        self.rel.width
    }
    pub fn is_empty(&self) -> bool {
        self.rel.width == 0
    }
    /// Its value in column `c`.
    pub fn get(&self, c: usize) -> Value<'a> {
        self.rel.value(self.i, c)
    }
    /// Its values.
    pub fn values(&self) -> impl Iterator<Item = Value<'a>> + '_ {
        (0..self.rel.width).map(|c| self.get(c))
    }
    /// Its hash (`Rel::hash`).
    pub fn hash(&self) -> u128 {
        self.rel.hash(self.i)
    }
    /// The integer column `c` holds for it, as the run held it: a node's value itself, however the image shows it
    /// (a text column's is its string's index).
    pub fn raw(&self, c: usize) -> i64 {
        self.rel.col(c)[self.i]
    }
}

impl std::fmt::Debug for Row<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.values()).finish()
    }
}

/// A relation as a stack reads it: from the last image that holds it whole, through each later image's rows gone and
/// added. Its parts are entries of the images, each with the rows of it that stand (all, where none went).
pub struct View<'a> {
    pub parts: Vec<(Rel<'a>, Option<Vec<usize>>)>,
}

impl<'a> View<'a> {
    /// Its columns' types: its whole entry's.
    pub fn types(&self) -> &[u8] {
        self.parts[0].0.types()
    }
    /// Its rows' parts and indexes.
    pub fn each(&self) -> impl Iterator<Item = (Rel<'a>, usize)> + '_ {
        self.parts.iter().flat_map(|(r, kept)| -> Box<dyn Iterator<Item = (Rel<'a>, usize)> + '_> {
            match kept {
                Some(k) => Box::new(k.iter().map(move |&i| (*r, i))),
                None => Box::new((0..r.rows).map(move |i| (*r, i))),
            }
        })
    }
    /// Its rows' count (a row in two parts counted twice; lines are each once).
    pub fn len(&self) -> usize {
        self.parts.iter().map(|(r, k)| k.as_ref().map_or(r.rows, Vec::len)).sum()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    /// Its rows' hashes, sorted, each once.
    pub fn hashes(&self) -> Vec<u128> {
        let mut h: Vec<u128> = if self.types().is_empty() {
            vec![0; self.len().min(1)]
        } else {
            self.parts.iter().flat_map(|(r, kept)| {
                let each = r.each_hash();
                match kept {
                    Some(k) => k.iter().map(|&i| each[i]).collect::<Vec<_>>(),
                    None => each,
                }
            }).collect()
        };
        h.sort_unstable();
        h.dedup();
        h
    }
}

/// A stack's relation `name` as it reads it (`View`), if an image holds it whole.
pub fn find<'a>(stack: &'a [Image], name: &str) -> Option<View<'a>> {
    let at = stack.iter().rposition(|i| i.rel(name).is_some())?;
    let mut parts: Vec<(Rel<'a>, Option<Vec<usize>>)> = vec![(stack[at].rel(name).expect("found"), None)];
    for image in &stack[at + 1..] {
        if let Some(gone) = image.entry(name, kind::GONE).filter(|g| g.rows > 0) {
            let gone: std::collections::HashSet<u128> = (0..gone.rows).map(|i| (gone.col(0)[i] as u64 as u128) | ((gone.col(1)[i] as u64 as u128) << 64)).collect();
            for (r, kept) in &mut parts {
                let each = r.each_hash();
                let k: Vec<usize> = match kept.take() {
                    Some(k) => k,
                    None => (0..r.rows).collect(),
                };
                *kept = Some(k.into_iter().filter(|&i| !gone.contains(&each[i])).collect());
            }
        }
        if let Some(added) = image.entry(name, kind::ADDED).filter(|a| a.rows > 0) {
            parts.push((added, None));
        }
    }
    Some(View { parts })
}

/// The 64-bit hashes of the texts and documents an image's rows name, each made once (0: not yet).
#[derive(Default)]
struct Seen {
    texts: Vec<u64>,
    docs: HashMap<i64, u64>,
}

impl Seen {
    fn text(&mut self, image: &Image, i: u32) -> u64 {
        if let Some(&h) = self.texts.get(i as usize).filter(|h| **h != 0) {
            return h;
        }
        let b = image.string(i).as_bytes();
        let mut h = Hash::new(b.len() as u64);
        h.bytes(b);
        let h = (h.finish() as u64).max(1);
        if let Some(slot) = self.texts.get_mut(i as usize) {
            *slot = h;
        }
        h
    }
    fn doc(&mut self, image: &Image, key: i64, path: u64, first: u64) -> u64 {
        if let Some(&h) = self.docs.get(&key) {
            return h;
        }
        let mut h = Hash::new(2);
        h.word(if path == NONE { NONE } else { self.text(image, path as u32) });
        h.word(first);
        let h = h.finish() as u64;
        self.docs.insert(key, h);
        h
    }
}

/// A 128-bit hash, the same in every process: two lanes of multiply-fold mixing over words.
struct Hash(u64, u64);

impl Hash {
    const K: [u64; 4] = [0x9e37_79b9_7f4a_7c15, 0xc2b2_ae3d_27d4_eb4f, 0x1656_67b1_9e37_79f9, 0x27d4_eb2f_1656_67c5];
    fn new(seed: u64) -> Self {
        Hash(seed ^ Self::K[0], seed.rotate_left(32) ^ Self::K[1])
    }
    fn mum(a: u64, b: u64) -> u64 {
        let m = u128::from(a).wrapping_mul(u128::from(b));
        (m as u64) ^ ((m >> 64) as u64)
    }
    fn word(&mut self, w: u64) {
        self.0 = Self::mum(self.0 ^ w ^ Self::K[2], self.1 ^ Self::K[0]);
        self.1 = Self::mum(self.1 ^ w.rotate_left(29) ^ Self::K[3], self.0 ^ Self::K[1]);
    }
    /// Bytes eight at a time, then their length.
    fn bytes(&mut self, b: &[u8]) {
        let (chunks, rest) = b.as_chunks::<8>();
        for c in chunks {
            self.word(u64::from_le_bytes(*c));
        }
        let mut last = [0u8; 8];
        last[..rest.len()].copy_from_slice(rest);
        self.word(u64::from_le_bytes(last));
        self.word(b.len() as u64);
    }
    fn finish(self) -> u128 {
        let (a, b) = (Self::mum(self.0 ^ Self::K[3], self.1 ^ Self::K[2]), Self::mum(self.1 ^ Self::K[0], self.0 ^ Self::K[3]));
        u128::from(a) | (u128::from(b) << 64)
    }
}

/// The stack in `dir`'s output file, mapped, or none.
pub fn read(dir: &Path) -> Result<Vec<Image>, String> {
    match Image::mapped(&dir.join(FILE)) {
        Ok(stack) => Ok(stack),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(bad(e)),
    }
}

/// `image` written to `dir`'s output file, whole or not at all: a base as the file, an overlay as the stack there
/// and it. The new file is written beside it and renamed over it, so a reader sees the old file or the new one, and
/// a map of the old one stays valid.
pub fn write(dir: &Path, image: &[u8]) -> std::io::Result<()> {
    // Its kind is its meta region's first word (`meta::KIND`); the meta region's offset is the header's first entry.
    let word = |at: usize| image.get(at..at + 8).map(|b| u64::from_le_bytes(b.try_into().expect("eight bytes")));
    let overlay = image.starts_with(&MAGIC) && word(16 + 16).and_then(|off| word(off as usize)) == Some(meta::OVERLAY);
    let below = if overlay { read(dir).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))? } else { Vec::new() };
    if overlay && below.is_empty() {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, format!("{}: an overlay over no stack", dir.display())));
    }
    let mut bytes: Vec<&[u8]> = below.first().map(|b| b.buf.bytes()).into_iter().collect();
    bytes.push(image);
    replace(dir, &bytes)
}

/// `dir`'s output file made of `parts`, whole or not at all (a file beside it renamed over it).
pub fn replace(dir: &Path, parts: &[&[u8]]) -> std::io::Result<()> {
    use std::io::Write;
    static WRITES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let new = dir.join(format!(".{FILE}.{}.{n}", std::process::id()));
    let written = (|| {
        let mut f = std::fs::File::create(&new)?;
        for p in parts {
            f.write_all(p)?;
        }
        drop(f);
        std::fs::rename(&new, dir.join(FILE))
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&new);
    }
    written
}

/// Relation `rel` of the stack in `dir`, read where it is: absent (`NotFound`) where `dir` holds no stack or the stack
/// no such relation.
/// Relation `rel` of the outputs in `dir`, each value as shown (`Relation::shown`); NotFound where it holds no such
/// relation.
pub fn shown(dir: &Path, rel: &str) -> std::io::Result<Vec<Vec<String>>> {
    let outputs = Outputs::open(dir)?;
    outputs.get(rel).map(|r| r.shown()).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("{}: no relation {rel}", dir.display())))
}

/// A base of the stack in `dir`'s relations `names` (those it holds), each as the stack reads it.
pub fn base_of<'a>(dir: &Path, names: impl Iterator<Item = &'a str>) -> std::io::Result<Vec<u8>> {
    let stack = read(dir).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{}: {e}", dir.display())))?;
    let mut image = Builder::new(stack.last().map_or(0, Image::flags), false);
    for name in names {
        if let Some(view) = find(&stack, name) {
            image.take(name, &view);
        }
    }
    Ok(image.finish())
}

/// The stack in `dir` written out as `.csv` files in `to`, one per relation (for scripts that read them).
pub fn to_csv(dir: &Path, to: &Path) -> std::io::Result<usize> {
    std::fs::create_dir_all(to)?;
    let outputs = Outputs::open(dir)?;
    let names = outputs.names();
    for &name in &names {
        let rel = outputs.get(name).expect("named");
        let text: String = if rel.width() == 0 { "\n".repeat(rel.len()) } else { rel.shown().iter().map(|r| r.join("\t") + "\n").collect() };
        std::fs::write(to.join(format!("{name}.csv")), text)?;
    }
    Ok(names.len())
}

#[cfg(test)]
mod tests;
