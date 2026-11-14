//! An entry as the store numbers its symbols (decision 0170), and its frame.
//!
//! A frame's symbols are the store's ids; an entry the run uses or makes holds this process's. The two agree until
//! this process forks from the store, and then differ: a store id read as this process's names another string, or
//! none (`symbol 4291 is no atom of this process's (it holds 2765)`). So the frame's codec is here alone, on
//! [`Stored`]: a frame decodes to a `Stored`, which becomes an [`Entry`] only by [`Stored::to_process`]; an entry
//! becomes a `Stored` only by [`to_store`], and only a `Stored` is encoded. Reading the store's ids as this
//! process's, or writing this process's to the store, is no longer a call that type-checks.

use super::{Entry, has_syms, map_entries, outputs_of};
use crate::content_store::{ContentStore, ProcessIds, StoreError, StoreIds, StoreResult};
use std::borrow::Cow;
use std::collections::BTreeMap;

/// An entry whose symbols are the store's ids: decoded from a frame, or mapped for a write.
#[derive(Debug)]
pub(super) struct Stored<'a>(Cow<'a, Entry>);

impl Stored<'static> {
    /// A frame's entry, its relations those `keep` takes (its key is the frame's): the store's ids.
    pub(super) fn decoded(frame: &crate::content_store::FrameRows, keep: &dyn Fn(&str) -> bool) -> StoreResult<Self> {
        decode(frame, keep).map(|e| Self(Cow::Owned(e)))
    }

    /// `entry`, taken to hold the store's ids as it is: a test's, whose entry names no atom.
    #[cfg(test)]
    pub(super) fn unmapped(entry: Entry) -> Self {
        Self(Cow::Owned(entry))
    }
}

impl Stored<'_> {
    /// The rows, each relation's (width, values), as the store numbers them.
    pub(super) fn rows(&self) -> &BTreeMap<String, (usize, Vec<i64>)> {
        &self.0.rows
    }

    pub(super) fn key(&self) -> i64 {
        self.0.key
    }

    /// The entry's outputs alone (its `own:` rows and lines), as the store numbers them.
    pub(super) fn outputs(&self) -> Stored<'static> {
        Stored(Cow::Owned(outputs_of(&self.0)))
    }

    /// Its frame.
    pub(super) fn encode(&self) -> crate::content_store::FrameRows {
        encode(&self.0)
    }

    /// This process's entry, keyed `key`: its symbols mapped by `atoms` (the table the frame was read under), where
    /// that table forked from the store, or holds a plan's entries another process may have numbered past its end.
    /// Forked, a relation whose symbol columns are not registered is refused (`DIVERGED_TEXT`): its rows may name
    /// the store's ids.
    pub(super) fn to_process(self, plan: &[u8; 32], key: i64, atoms: Option<&crate::content_store::atoms::Atoms>) -> StoreResult<Entry> {
        let mut entry = self.0.into_owned();
        entry.key = key;
        if let Some(atoms) = atoms
            && (atoms.forked() || has_syms(plan))
        {
            map_entries(std::slice::from_mut(&mut entry), plan, atoms.forked(), |v| atoms.to_local(StoreIds::as_stored(v)).map(drop))?;
        }
        Ok(entry)
    }

    /// The entry as it is, its ids the store's: a test's, whose entry names no atom.
    #[cfg(test)]
    pub(super) fn into_unmapped(self) -> Entry {
        self.0.into_owned()
    }
}

/// `entries` (this process's) as the store numbers them, in the writer's transaction: borrowed where the two
/// numberings agree (not forked), else each mapped, one call for all. Forked, a relation whose symbol columns are not
/// registered is refused.
pub(super) fn to_store<'a>(entries: &[&'a Entry], plan: &[u8; 32], store: &ContentStore) -> StoreResult<Vec<Stored<'a>>> {
    if !store.atoms_handle().forked() {
        return Ok(entries.iter().map(|e| Stored(Cow::Borrowed(*e))).collect());
    }
    let mut all: Vec<Entry> = entries.iter().map(|e| Entry::clone(e)).collect();
    map_entries(&mut all, plan, true, |v| store.symbols_to_store(ProcessIds::of_process(v)).map(drop))?;
    Ok(all.into_iter().map(|e| Stored(Cow::Owned(e))).collect())
}

// An entry's frame (decision 0146): typed rows of integers, each row's first value its kind, its second the
// relation or output it is of. A relation's rows are rows of it, a value to a column; text (names, lines) is packed
// eight bytes to a value.
const NODES: i64 = 0;
const REL: i64 = 2;
const REL_NAME: i64 = 3;
const REL_ROW: i64 = 4;
const OUT: i64 = 5;
const OUT_NAME: i64 = 6;
const OUT_COLUMN: i64 = 7;
const LINE: i64 = 8;
const LINE_TEXT: i64 = 9;

/// The entry's rows; the frame's width: two, and the widest relation's, at least eight.
fn encode(entry: &Entry) -> crate::content_store::FrameRows {
    let width = 2 + entry.rows.values().map(|(w, _)| *w).max().unwrap_or(0).max(8);
    let mut rows: Vec<i64> = Vec::new();
    let mut n = 0;
    let mut row = |rows: &mut Vec<i64>, values: &[i64]| {
        rows.extend_from_slice(values);
        rows.extend(std::iter::repeat_n(0, width - values.len()));
        n += 1;
    };
    // Text after a row that gives its length: `width - 3` values a row, after the kind, the owner and a sequence.
    let text = |rows: &mut Vec<i64>, row: &mut dyn FnMut(&mut Vec<i64>, &[i64]), kind: i64, owner: i64, at: i64, s: &str| {
        for chunk in s.as_bytes().chunks((width - 3) * 8) {
            let mut values = vec![kind, owner, at];
            values.extend(chunk.chunks(8).map(|b| {
                let mut word = [0u8; 8];
                word[..b.len()].copy_from_slice(b);
                i64::from_le_bytes(word)
            }));
            row(rows, &values);
        }
    };
    row(&mut rows, &[NODES, 0, entry.nodes as i64]);
    for (r, (name, (w, values))) in entry.rows.iter().enumerate() {
        let r = r as i64;
        let count = if *w == 0 { 0 } else { values.len() / w };
        row(&mut rows, &[REL, r, *w as i64, count as i64, name.len() as i64]);
        text(&mut rows, &mut row, REL_NAME, r, 0, name);
        for v in values.chunks(*w.max(&1)) {
            let mut values = vec![REL_ROW, r];
            values.extend_from_slice(v);
            row(&mut rows, &values);
        }
    }
    for (o, (name, (columns, lines))) in entry.lines.iter().enumerate() {
        let o = o as i64;
        row(&mut rows, &[OUT, o, columns.len() as i64, lines.len() as i64, name.len() as i64]);
        text(&mut rows, &mut row, OUT_NAME, o, 0, name);
        for c in columns {
            row(&mut rows, &[OUT_COLUMN, o, *c as i64]);
        }
        for (i, line) in lines.iter().enumerate() {
            row(&mut rows, &[LINE, o, i as i64, line.len() as i64]);
            text(&mut rows, &mut row, LINE_TEXT, o, i as i64, line);
        }
    }
    crate::content_store::FrameRows { types: vec![b'i'; width], rows, n }
}

/// The entry `encode` made (its key is the frame's), keeping only the relations `keep` takes: the others' rows are
/// checked, not kept.
fn decode(frame: &crate::content_store::FrameRows, keep: &dyn Fn(&str) -> bool) -> StoreResult<Entry> {
    let bad = |what: &str| StoreError::Invalid(format!("a corpus memo entry's frame {what}"));
    let width = frame.types.len();
    if width < 3 || frame.rows.len() != width * frame.n {
        return Err(bad("has no rows of its width"));
    }
    let mut entry = Entry::default();
    let mut rels: Vec<(String, usize, usize, Vec<u8>, Vec<i64>)> = Vec::new();
    let mut outs: Vec<(Vec<u8>, usize, Vec<usize>, Vec<(usize, Vec<u8>)>)> = Vec::new();
    let bytes = |values: &[i64]| values.iter().flat_map(|v| v.to_le_bytes()).collect::<Vec<u8>>();
    for row in frame.rows.chunks_exact(width) {
        let owner = row[1] as usize;
        match row[0] {
            NODES => entry.nodes = row[2] as u64,
            REL => {
                if owner != rels.len() {
                    return Err(bad("names a relation out of order"));
                }
                rels.push((String::new(), row[2] as usize, row[4] as usize, Vec::new(), Vec::new()));
            }
            REL_NAME => rels.get_mut(owner).ok_or_else(|| bad("names no relation"))?.3.extend(bytes(&row[3..])),
            REL_ROW => {
                let r = rels.get_mut(owner).ok_or_else(|| bad("has a row of no relation"))?;
                let w = r.1;
                r.4.extend_from_slice(row.get(2..2 + w).ok_or_else(|| bad("has a row wider than itself"))?);
            }
            OUT => {
                if owner != outs.len() {
                    return Err(bad("names an output out of order"));
                }
                outs.push((Vec::new(), row[4] as usize, Vec::new(), Vec::new()));
            }
            OUT_NAME => outs.get_mut(owner).ok_or_else(|| bad("names no output"))?.0.extend(bytes(&row[3..])),
            OUT_COLUMN => outs.get_mut(owner).ok_or_else(|| bad("has a column of no output"))?.2.push(row[2] as usize),
            LINE => outs.get_mut(owner).ok_or_else(|| bad("has a line of no output"))?.3.push((row[3] as usize, Vec::new())),
            LINE_TEXT => {
                let o = outs.get_mut(owner).ok_or_else(|| bad("has a line of no output"))?;
                o.3.last_mut().ok_or_else(|| bad("has text of no line"))?.1.extend(bytes(&row[3..]));
            }
            _ => return Err(bad("has a row of no kind")),
        }
    }
    let text = |mut b: Vec<u8>, len: usize| -> StoreResult<String> {
        if b.len() < len {
            return Err(bad("has text shorter than its length"));
        }
        b.truncate(len);
        String::from_utf8(b).map_err(|_| bad("has text that is not UTF-8"))
    };
    for (_, w, len, name, values) in rels {
        let name = text(name, len)?;
        if keep(&name) {
            entry.rows.insert(name, (w, values));
        }
    }
    for (name, len, columns, lines) in outs {
        let lines = lines.into_iter().map(|(n, b)| text(b, n)).collect::<StoreResult<Vec<_>>>()?;
        entry.lines.insert(text(name, len)?, (columns, lines));
    }
    Ok(entry)
}
