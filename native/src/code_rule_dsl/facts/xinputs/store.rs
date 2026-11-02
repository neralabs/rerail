//! A content's inputs in the content store, as a derivation family keyed by content (03 §14).
//!
//! - The rows: `Produced` as the narrow tables of `content_store::xinputs` (`to_set`, `from_set`),
//!   keyed by the content's tape. Only what the readers read is kept: `cgn_reference_alias` rows
//!   whose key holds `.<base-slot>.` (Y1's `base_slots`), `cgn_open`'s `open`, and of each pack
//!   input the columns `exports` reads.
//! - The derivation: `xinputs:{tape hash}` reads the tape's canonical hash (an immutable input,
//!   as the tape family outputs it) and outputs the digest of the rows (`XinputSet::digest`). A
//!   content seen before, at any path or in any workspace, is a lookup (`Inputs::set`).
//! - The reader (`StoreSource`): a (path, content) the cross-file families have not loaded yet,
//!   as the exports they read, from the store: the content (a source with its blake3 alias), its
//!   tape, its set (produced and published when absent), then `exports` at the path.
use super::cgnative::{Rendered, Variant};
use super::exports::{Exports, Scope, exports};
use super::pack::{INPUTS, Table};
use super::{Produced, Producer};
use crate::code_rule_dsl::language::{GrammarDialect, tape_tag};
use crate::code_rule_dsl::tape::{FileImage, Grammar, TAPE_FORMAT_VERSION, TapeImage};
use crate::code_rule_dsl::workspace::grammar_version;
use crate::content_store::xinputs::{Rows, TABLES, XinputSet};
use crate::content_store::{ContentStore, DerivationInput, DerivationRecord, HashKey, StoreError, StoreResult, TapeId};
use rusqlite::types::Value;
use sha2::{Digest, Sha256};
use std::sync::Arc;

/// The producer's identity: the pack (its inputs and stages), the summary's reading, the tape
/// format the rows are read from.
pub(crate) fn producer_id() -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"rerail.xinputs/v1\0");
    h.update(include_bytes!("symbols-merged.json"));
    h.update(TAPE_FORMAT_VERSION.to_le_bytes());
    h.update(grammar_version().to_le_bytes());
    h.finalize().into()
}

/// The name of a content's derivation: its tape's canonical hash.
pub(crate) fn derivation_name(tape_hash: &[u8; 32]) -> String {
    format!("xinputs:{}", hex(tape_hash))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The stored columns of each pack input: (input, table index, start, end, columns).
const PACK_TABLES: [(&str, usize, bool, bool, &[&str]); 12] = [
    ("py_functions", 7, true, true, &["name", "class_name", "return_annotation", "class_start", "body_start"]),
    ("py_classes", 8, true, true, &["name", "body_start"]),
    ("py_decorators", 9, false, false, &["name", "owner"]),
    ("py_parameters", 10, true, true, &["annotation", "name", "owner"]),
    ("py_parameter_annotations", 11, false, false, &["name", "parameter"]),
    ("py_return_annotations", 12, false, false, &["name", "owner"]),
    ("py_imports", 13, true, false, &["base", "items", "top"]),
    ("py_bases", 14, true, false, &["name", "owner"]),
    ("py_all", 15, true, false, &["array"]),
    ("py_all_names", 16, false, false, &["name", "owner"]),
    ("dc__raw_dynamic_imports", 18, false, false, &["entry"]),
    ("ef__raw_execution_calls", 19, false, false, &["name", "owner"]),
];
const MODULE_NAMES: usize = 17;

/// The summary relations: (table index, rendered relation, columns kept, integer columns).
const CGN_TABLES: [(usize, &str, usize, &[bool]); 7] = [
    (0, "cgn_def", 4, &[false, true, true, true]),
    (1, "cgn_edge", 2, &[true, false]),
    (2, "cgn_alias", 2, &[false, false]),
    (3, "cgn_opaque", 1, &[false]),
    (4, "cgn_module", 1, &[false]),
    (5, "cgn_open", 1, &[true]),
    (6, "cgn_reference_alias", 2, &[false, false]),
];
const BASE_SLOT: &str = ".<base-slot>.";

fn int(s: &str) -> Result<Value, String> {
    s.parse::<i64>().map(Value::Integer).map_err(|_| format!("not an integer: {s:?}"))
}

/// `x` as the store holds it, with its digest.
pub(crate) fn to_set(p: &Producer, x: &Produced) -> Result<XinputSet, String> {
    let mut rows: Vec<Rows> = vec![Vec::new(); TABLES.len()];
    for (v, r) in x.cgn.iter().enumerate() {
        for (k, rel, keep, ints) in CGN_TABLES {
            for row in r.rels.get(rel).map(Vec::as_slice).unwrap_or_default() {
                // `cgn_def`'s first column is its index: the row's ordinal.
                let cols = if rel == "cgn_def" { &row[1..] } else { &row[..] };
                if rel == "cgn_reference_alias" && !cols[0].contains(BASE_SLOT) {
                    continue;
                }
                let values = cols[..keep].iter().zip(ints).map(|(c, &i)| if i { int(c) } else { Ok(Value::Text(c.clone())) }).collect::<Result<_, _>>()?;
                rows[k].push((v as u8, values));
            }
        }
    }
    for (input, k, start, end, cols) in PACK_TABLES {
        let i = INPUTS.iter().position(|n| *n == input).unwrap();
        let at: Vec<usize> = cols.iter().map(|c| Table::col(&p.pack.inputs[i], c)).collect();
        for (s, e, v) in &x.inputs[i].rows {
            let mut values = Vec::with_capacity(cols.len() + 2);
            if start {
                values.push(Value::Integer(*s));
            }
            if end {
                values.push(Value::Integer(*e));
            }
            values.extend(at.iter().map(|&c| v[c].clone()));
            rows[k].push((0, values));
        }
    }
    rows[MODULE_NAMES] = x.module_names.iter().map(|n| (0, vec![Value::Text(n.clone())])).collect();
    Ok(XinputSet { producer: producer_id(), digest: digest(&rows), failure: None, rows })
}

/// The digest of a set's rows: each table's rows in order, each value typed.
fn digest(rows: &[Rows]) -> [u8; 32] {
    let mut h = Sha256::new();
    for (k, table) in rows.iter().enumerate() {
        h.update((k as u32).to_le_bytes());
        h.update((table.len() as u64).to_le_bytes());
        for (variant, values) in table {
            h.update([*variant]);
            for v in values {
                match v {
                    Value::Null => h.update([0]),
                    Value::Integer(i) => {
                        h.update([1]);
                        h.update(i.to_le_bytes());
                    }
                    Value::Text(t) => {
                        h.update([2]);
                        h.update((t.len() as u64).to_le_bytes());
                        h.update(t.as_bytes());
                    }
                    Value::Real(r) => {
                        h.update([3]);
                        h.update(r.to_le_bytes());
                    }
                    Value::Blob(b) => {
                        h.update([4]);
                        h.update((b.len() as u64).to_le_bytes());
                        h.update(b);
                    }
                }
            }
        }
    }
    h.finalize().into()
}

/// The set of a content the producer could not read.
fn failed(why: String) -> XinputSet {
    let rows = vec![Vec::new(); TABLES.len()];
    XinputSet { producer: producer_id(), digest: digest(&rows), failure: Some(why), rows }
}

fn text(v: &Value) -> String {
    match v {
        Value::Integer(i) => i.to_string(),
        Value::Text(t) => t.clone(),
        Value::Null => String::new(),
        o => panic!("{o:?} in a summary row"),
    }
}

/// A stored set as the producer's output (the columns it does not store left NULL).
pub(crate) fn from_set(p: &Producer, set: &XinputSet) -> Produced {
    let mut cgn: Vec<Rendered> = Variant::ALL.iter().map(|_| Rendered::default()).collect();
    for (k, rel, _, _) in CGN_TABLES {
        for r in cgn.iter_mut() {
            r.rels.insert(rel, Vec::new());
        }
        for (ord_in_variant, (v, values)) in ordinals(&set.rows[k]) {
            let mut row: Vec<String> = values.iter().map(text).collect();
            if rel == "cgn_def" {
                row.insert(0, ord_in_variant.to_string());
            }
            cgn[*v as usize].rels.get_mut(rel).unwrap().push(row);
        }
    }
    let mut inputs: Vec<Table> = INPUTS.iter().map(|_| Table::default()).collect();
    for (input, k, start, end, cols) in PACK_TABLES {
        let i = INPUTS.iter().position(|n| *n == input).unwrap();
        let width = p.pack.inputs[i].columns.len();
        let at: Vec<usize> = cols.iter().map(|c| Table::col(&p.pack.inputs[i], c)).collect();
        for (_, values) in &set.rows[k] {
            let mut it = values.iter();
            let s = if start { as_int(it.next()) } else { 0 };
            let e = if end { as_int(it.next()) } else { 0 };
            let mut row = vec![Value::Null; width];
            for (&c, v) in at.iter().zip(it) {
                row[c] = v.clone();
            }
            inputs[i].rows.push((s, e, row));
        }
    }
    let module_names = set.rows[MODULE_NAMES].iter().map(|(_, v)| text(&v[0])).collect();
    Produced { cgn, inputs, module_names }
}

fn as_int(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Integer(i)) => *i,
        o => panic!("a span that is not an integer: {o:?}"),
    }
}

/// Each row with its ordinal within its variant.
fn ordinals(rows: &Rows) -> impl Iterator<Item = (usize, &(u8, Vec<Value>))> {
    let mut next = [0usize; 3];
    rows.iter().map(move |r| {
        let k = next[r.0 as usize];
        next[r.0 as usize] += 1;
        (k, r)
    })
}

/// The content store's inputs: contents found by their blake3 (the replay's and the exports'
/// name), their tapes stored by the tape family's rules, their sets produced on first read.
pub(crate) struct StoreSource {
    pub store: Arc<ContentStore>,
    pub producer: Producer,
    pub scope: Scope,
    grammar: Arc<Grammar>,
}

/// What reading one content did.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub(crate) struct Read {
    pub sets_read: usize,
    pub sets_produced: usize,
    pub tapes_encoded: usize,
}

fn blake3_key(hex: &str) -> Option<HashKey> {
    let b = hex.as_bytes();
    if b.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(std::str::from_utf8(&b[2 * i..2 * i + 2]).ok()?, 16).ok()?;
    }
    Some(HashKey::blake3(out))
}

impl StoreSource {
    pub fn new(store: Arc<ContentStore>) -> Result<StoreSource, String> {
        let producer = Producer::new()?;
        let scope = Scope::new(&producer);
        let grammar = Grammar::new(GrammarDialect::Python)?;
        Ok(StoreSource { store, producer, scope, grammar })
    }

    /// Stores `source` as the content `blake3` names (no-op when stored); its source id.
    pub fn put_content(&self, blake3: &str, source: &[u8]) -> StoreResult<crate::content_store::SourceId> {
        let key = blake3_key(blake3).ok_or_else(|| StoreError::Invalid(format!("not a blake3 content name: {blake3}")))?;
        match self.store.source_by_alias(key)? {
            Some(id) => Ok(id),
            None => self.store.put_source(source, &[key]),
        }
    }

    /// The content's tape (id, canonical hash, image, source): the stored one, else encoded and
    /// stored as the tape family stores it. None: no such content, or the parser gave up.
    fn tape(&self, blake3: &str, read: &mut Read) -> StoreResult<Option<(TapeId, [u8; 32], FileImage, Arc<String>)>> {
        let Some(key) = blake3_key(blake3) else { return Ok(None) };
        let Some(source_id) = self.store.source_by_alias(key)? else { return Ok(None) };
        let Ok(source) = String::from_utf8(self.store.get_source(source_id)?) else { return Ok(None) };
        let source = Arc::new(source);
        let tag = i64::from(tape_tag(GrammarDialect::Python).expect("a tape dialect"));
        let version = i64::from(TAPE_FORMAT_VERSION);
        let (id, bytes) = match self.store.tape_by_source(source_id, tag, grammar_version(), version)? {
            Some(id) => {
                let Some((bytes, _)) = self.store.get_tapes_bulk(&[id])?.remove(&id) else { return Ok(None) };
                (id, bytes[..].to_vec())
            }
            None => {
                let Some(image) = TapeImage::parse(&self.grammar, &source, &|| false) else { return Ok(None) };
                let bytes = image.map_err(StoreError::Invalid)?.primary.to_bytes().map_err(StoreError::Invalid)?;
                read.tapes_encoded += 1;
                (self.store.put_tape(source_id, tag, grammar_version(), version, &bytes)?, bytes)
            }
        };
        let primary = TapeImage::from_bytes(&bytes, &self.grammar).map_err(StoreError::Invalid)?;
        Ok(Some((id, Sha256::digest(&bytes).into(), FileImage { primary, derived: Vec::new() }, source)))
    }

    /// The content's set: stored, else produced from its tape, published and its derivation
    /// recorded. None: the content is not stored.
    pub fn set(&self, blake3: &str, read: &mut Read) -> StoreResult<Option<XinputSet>> {
        let Some((tape, tape_hash, image, source)) = self.tape(blake3, read)? else { return Ok(None) };
        if let Some(set) = self.store.xinputs(tape)? {
            if set.producer == producer_id() {
                read.sets_read += 1;
                return Ok(Some(set));
            }
        }
        let set = self.producer.produce(source, &image).and_then(|x| to_set(&self.producer, &x)).unwrap_or_else(failed);
        self.publish(tape, tape_hash, &set)?;
        read.sets_produced += 1;
        Ok(Some(set))
    }

    /// Publishes `set` as `tape`'s and records its derivation.
    pub fn publish(&self, tape: TapeId, tape_hash: [u8; 32], set: &XinputSet) -> StoreResult<()> {
        self.store.put_xinputs(tape, set)?;
        self.store.record_derivations(&[DerivationRecord {
            output: derivation_name(&tape_hash),
            producer: set.producer,
            inputs: vec![DerivationInput { name: None, digest: tape_hash }],
            digest: set.digest,
            status: i64::from(set.failure.is_some()),
            cost: 0,
        }])?;
        Ok(())
    }

    /// The exports `(path, content)` reads (`failure` when the producer could not read the
    /// content); None: the content is not stored. Out of the pack's scope nothing is read.
    pub fn exports(&self, path: &str, blake3: &str, read: &mut Read) -> StoreResult<Option<(bool, Exports)>> {
        if !self.scope.admits(path) {
            return Ok(Some((false, exports(&self.producer, &self.scope, path, &Produced::default()))));
        }
        let Some(set) = self.set(blake3, read)? else { return Ok(None) };
        let x = from_set(&self.producer, &set);
        Ok(Some((set.failure.is_some(), exports(&self.producer, &self.scope, path, &x))))
    }

    /// Stores and produces every content of `jobs` ((blake3, source bytes)) not produced yet:
    /// sources and tapes first, then the sets in parallel, published in order.
    pub fn ingest(&self, jobs: &[(String, Vec<u8>)]) -> StoreResult<Read> {
        use rayon::prelude::*;
        let mut read = Read::default();
        let mut todo = Vec::new();
        for (blake3, bytes) in jobs {
            self.put_content(blake3, bytes)?;
            let Some((tape, hash, image, source)) = self.tape(blake3, &mut read)? else { continue };
            match self.store.xinputs(tape)? {
                Some(set) if set.producer == producer_id() => read.sets_read += 1,
                _ => todo.push((tape, hash, image, source)),
            }
        }
        for chunk in todo.chunks(256) {
            let sets: Vec<XinputSet> = chunk
                .par_iter()
                .map(|(_, _, image, source)| self.producer.produce(Arc::clone(source), image).and_then(|x| to_set(&self.producer, &x)).unwrap_or_else(failed))
                .collect();
            for ((tape, hash, _, _), set) in chunk.iter().zip(&sets) {
                self.publish(*tape, *hash, set)?;
                read.sets_produced += 1;
            }
        }
        Ok(read)
    }
}

/// A `StoreSource` read by the cross-file steps (`y3::wire::Source`), counting what it did.
pub(crate) struct Reading {
    pub source: StoreSource,
    pub read: std::rc::Rc<std::cell::Cell<Read>>,
}
