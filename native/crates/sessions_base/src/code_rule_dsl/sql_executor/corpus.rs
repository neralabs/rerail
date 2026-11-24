//! A corpus run's memo frames as relations read lazily (`docs/sql-access/01-design.md` §2).
//!
//! `bindCorpus` keeps the run's frames compressed and reads only each frame's document key, from
//! its first eight decompressed bytes. A relation's rows of one document are decoded the first time
//! a statement asks for them: a lookup on a location column (a node `key << 32 | index`, or a
//! document key) decodes the one document it names. A scan, or a lookup on another column, decodes
//! every document once.
//!
//! An export's entry holds rows of `i64` values as the run made them. A file-local output's entry
//! holds rendered lines: tab-separated text, its node and number columns as decimals. Their text
//! columns are interned as symbols when the document is decoded.
use rerail_blocks::vector::Vector;
use super::*;
use crate::content_store::atoms::Atoms;
use plan::corpus::memo;
use std::sync::OnceLock;

/// A row found by a lookup: `PROBE | document << 32 | row in the document`. Scanned rows count
/// from zero over the whole relation.
const PROBE: usize = 1 << 62;

/// The run's documents: each frame, its key, and its entry once decoded.
pub struct Run {
    frames: Vec<memo::Frame>,
    keys: Vector<i64>,
    by_key: HashMap<i64, usize>,
    entries: Vec<OnceLock<Option<Arc<memo::Entry>>>>,
    /// The first frame that did not decode: its path and the store's message.
    failed: OnceLock<String>,
}

impl Run {
    pub fn open(frames: Vec<memo::Frame>) -> Result<Self> {
        let keys = frames.iter().map(|f| memo::frame_key(f).map_err(|e| error(format!("{}: {e}", f.path)))).collect::<Result<Vector<_>>>()?;
        let by_key = keys.iter().enumerate().map(|(i, k)| (*k, i)).collect();
        let entries = frames.iter().map(|_| OnceLock::new()).collect();
        Ok(Self { frames, keys, by_key, entries, failed: OnceLock::new() })
    }

    /// Each document's key and stored path.
    pub fn documents(&self) -> impl Iterator<Item = (i64, &str)> {
        self.keys.iter().copied().zip(self.frames.iter().map(|f| f.path.as_str()))
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// A document's entry. A frame that does not decode has none, and is recorded as the run's
    /// failure (`Source::failure`), which fails the statement that read it.
    fn entry(&self, doc: usize) -> Option<Arc<memo::Entry>> {
        self.entries[doc]
            .get_or_init(|| match memo::decode_frame(&self.frames[doc]) {
                Ok(entry) => Some(Arc::new(entry)),
                Err(e) => {
                    let _ = self.failed.set(format!(
                        "{}: its corpus memo frame does not decode ({e}); run the corpus again",
                        self.frames[doc].path
                    ));
                    None
                }
            })
            .clone()
    }

    pub fn failure(&self) -> Option<String> {
        self.failed.get().cloned()
    }
}

/// Whether an entry holds the relation's rows or its rendered lines.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Form {
    Rows,
    Lines,
}

/// One relation of the run, decoded per document on demand.
pub struct Corpus {
    run: Arc<Run>,
    name: String,
    columns: Vec<String>,
    /// The `.decl` types: `int` and `node`/`loc` columns of lines are parsed, the rest interned.
    types: Vec<String>,
    form: Form,
    /// A logically nullary relation: one `present` column, `1` where a document holds it.
    nullary: bool,
    atoms: Arc<Atoms>,
    /// The columns whose values name a document (`node`, `loc`).
    locations: Vector<usize>,
    docs: Vec<OnceLock<Arc<[i64]>>>,
    all: OnceLock<Vector<i64>>,
    /// Rows per document, from the first document, for the planner.
    per_doc: usize,
}

impl Corpus {
    pub fn new(run: Arc<Run>, name: &str, columns: Vec<String>, types: Vec<String>, form: Form, atoms: Arc<Atoms>) -> Self {
        let nullary = types.is_empty();
        let types = if nullary { vec!["int".to_owned()] } else { types };
        let locations = types.iter().enumerate().filter(|(_, t)| matches!(t.as_str(), "node" | "loc")).map(|(i, _)| i).collect();
        let docs = (0..run.len()).map(|_| OnceLock::new()).collect();
        let mut source = Self { run, name: name.to_owned(), columns, types, form, nullary, atoms, locations, docs, all: OnceLock::new(), per_doc: 1 };
        if source.run.len() > 0 {
            source.per_doc = (source.rows_of(0).len() / source.width()).max(1);
        }
        source
    }

    /// The binding's column types (`bindRelation`'s names): rows keep the run's values, lines are
    /// parsed or interned.
    pub fn bound_types(&self) -> Option<Vec<String>> {
        self.types
            .iter()
            .map(|t| match (t.as_str(), self.form) {
                _ if self.nullary => None,
                ("int", _) => Some("number".to_owned()),
                ("node" | "loc", _) => Some(t.clone()),
                ("sym", _) | (_, Form::Lines) => Some("symbol".to_owned()),
                _ => None,
            })
            .collect()
    }

    fn width(&self) -> usize {
        self.columns.len()
    }

    /// The relation's rows of document `doc`, flat, sorted and distinct.
    fn rows_of(&self, doc: usize) -> Arc<[i64]> {
        self.docs[doc]
            .get_or_init(|| {
                let w = self.width();
                let mut rows: Vec<Vector<i64>> = match (self.run.entry(doc), self.form) {
                    (Some(e), Form::Rows) if self.nullary => e.rows.get(&self.name).map_or_else(Vec::new, |_| vec![rerail_blocks::vector![1]]),
                    (Some(e), Form::Lines) if self.nullary => e.outputs.get(&self.name).filter(|(_, r)| !r.is_empty()).map_or_else(Vec::new, |_| vec![rerail_blocks::vector![1]]),
                    (Some(e), Form::Rows) => match e.rows.get(&self.name) {
                        Some((width, values)) if *width == w => values.chunks(w).map(Vector::<i64>::from).collect(),
                        _ => vec![],
                    },
                    (Some(e), Form::Lines) => e.outputs.get(&self.name).map_or_else(Vec::new, |(_, rows)| self.values(rows)),
                    (None, _) => vec![],
                };
                rows.sort_unstable();
                rows.dedup();
                rows.concat().into()
            })
            .clone()
    }

    /// A memo entry's typed rows as rows of values: an int, node or location column's integers as they are, a text
    /// column's texts as atoms. A row of another width, or with a text where an integer is declared, is left out.
    fn values(&self, rows: &crate::code_rule_dsl::logic::plan::output::Rows) -> Vec<Vector<i64>> {
        use crate::code_rule_dsl::logic::plan::output::Cell;
        let w = self.width();
        if rows.width() != w {
            return vec![];
        }
        let integer = |t: &str| matches!(t, "int" | "node" | "loc");
        // A text column's values, each shown (an integer a text column holds: as it is written).
        let shown: Vec<String> = (0..rows.len())
            .flat_map(|i| (0..w).filter(|&c| !integer(&self.types[c])).map(move |c| (i, c)))
            .map(|(i, c)| match rows.cell(i, c) {
                Cell::Text(t) => t.to_owned(),
                Cell::Int(v) => v.to_string(),
            })
            .collect();
        let Ok(atoms) = self.atoms.intern_all(&shown.iter().map(String::as_str).collect::<Vec<_>>()) else { return vec![] };
        let mut atoms = atoms.into_iter();
        (0..rows.len())
            .filter_map(|i| {
                let row: Vec<Option<i64>> = (0..w)
                    .map(|c| match (rows.cell(i, c), integer(&self.types[c])) {
                        (Cell::Int(v), true) => Some(v),
                        (Cell::Text(_), true) => None,
                        (_, false) => atoms.next().map(i64::from),
                    })
                    .collect();
                row.into_iter().collect()
            })
            .collect()
    }

    fn all(&self) -> &[i64] {
        self.all.get_or_init(|| {
            let w = self.width();
            let docs: Vec<Arc<[i64]>> = (0..self.run.len()).map(|d| self.rows_of(d)).collect();
            let rows: BTreeSet<&[i64]> = docs.iter().flat_map(|r| r.chunks(w)).collect();
            rows.into_iter().flatten().copied().collect()
        })
    }
}

impl Source for Corpus {
    fn columns(&self) -> Vec<String> {
        self.columns.clone()
    }
    fn len(&self) -> usize {
        self.all().len() / self.width()
    }
    fn estimated_len(&self) -> usize {
        self.per_doc * self.run.len()
    }
    fn estimated_matches(&self, column: usize, _batch: bool) -> usize {
        if self.locations.contains(&column) { 1 } else { self.estimated_len() }
    }
    fn indexed(&self, column: usize) -> bool {
        self.locations.contains(&column)
    }
    fn made_on_lookup(&self) -> bool {
        true
    }
    fn failure(&self) -> Option<String> {
        self.run.failure()
    }
    fn lookup(&self, column: usize, values: &[i64]) -> Option<rerail_blocks::vector::Vector<usize>> {
        if !self.locations.contains(&column) {
            return None;
        }
        let w = self.width();
        let mut found = rerail_blocks::vector![];
        for &v in values.iter().filter(|v| **v > 0) {
            let key = if v >= 1 << 32 { v >> 32 } else { v };
            let Some(&doc) = self.run.by_key.get(&key) else { continue };
            let rows = self.rows_of(doc);
            found.extend(rows.chunks(w).enumerate().filter(|(_, r)| r[column] == v).map(|(i, _)| PROBE | doc << 32 | i));
        }
        Some(found)
    }
    fn value(&self, row: usize, column: usize) -> i64 {
        let w = self.width();
        if row & PROBE != 0 {
            let (doc, i) = ((row & !PROBE) >> 32, row & 0xffff_ffff);
            return self.rows_of(doc)[i * w + column];
        }
        self.all()[row * w + column]
    }
}
