//! A memo entry as stored (decision 0240 §4): one output image (decision 0239), compressed, its symbols by content.
//!
//! The entry's relations are the image's, each as made (`Builder::rows_as_is`: in its order, every row): a relation
//! `r:<key>` of the rows under `key` (`Entry::rows`), its symbol columns (the plan's, `Syms`) text, the others
//! integers; for each output, `c:<name>` its node columns and `l:<name>` its lines, a text each; `#nodes` the
//! document's nodes. A symbol is held as its string, so an entry reads the same in every process: a process that
//! numbered the store's atoms otherwise (a fork, decision 0170) reads it as any other. Before, the entry held the
//! store's ids, and each read and write mapped them (`numbering`: `to_store`, `to_process`, and every entry's ids
//! held before the waves).

use super::{Entry, Syms};
use crate::code_rule_dsl::logic::plan::output::{Builder, Image, col};
use crate::content_store::atoms::{Atoms, Sym};
use crate::content_store::{StoreError, StoreResult};
use std::collections::HashMap;

/// zstd's level: the entry is written by the store's writer, behind the run, and read in the waves.
const LEVEL: i32 = 1;
const NODES: &str = "#nodes";

fn bad(what: impl std::fmt::Display) -> StoreError {
    StoreError::Invalid(format!("a corpus memo entry {what}"))
}

/// The entry's image, compressed, and the image's length: its symbols' strings read from `atoms`, the table its
/// rows were made under.
pub(super) fn encode(entry: &Entry, syms: &Syms, atoms: &Atoms) -> StoreResult<(Vec<u8>, usize)> {
    let mut b = Builder::new(0, false);
    b.rows_as_is(NODES, &[col::INT], vec![entry.nodes as i64]);
    let mut shown: HashMap<i64, i64> = HashMap::new();
    for (key, (w, values)) in &entry.rows {
        let w = *w;
        let name = format!("r:{key}");
        if w == 0 {
            b.rows_as_is(&name, &[], Vec::new());
            continue;
        }
        let cols = syms.columns(key, w).filter(|c| c.len() == w).unwrap_or_else(|| vec![false; w]);
        let types: Vec<u8> = cols.iter().map(|&s| if s { col::TEXT } else { col::INT }).collect();
        let mut rows = Vec::with_capacity(values.len());
        for row in values.chunks_exact(w) {
            for (c, &v) in row.iter().enumerate() {
                if !cols[c] {
                    rows.push(v);
                    continue;
                }
                let at = match shown.get(&v) {
                    Some(&at) => at,
                    None => {
                        let atom = Sym::try_from(v).map_err(|_| bad(format!("of {key}: symbol {v} is no atom")))?;
                        let at = atoms.with_str(atom, |s| i64::from(b.text(s))).ok_or_else(|| bad(format!("of {key}: symbol {v} is no atom of this process's")))?;
                        shown.insert(v, at);
                        at
                    }
                };
                rows.push(at);
            }
        }
        b.rows_as_is(&name, &types, rows);
    }
    for (name, (nodes, lines)) in &entry.lines {
        b.rows_as_is(&format!("c:{name}"), &[col::INT], nodes.iter().map(|&c| c as i64).collect());
        let lines: Vec<i64> = lines.iter().map(|l| i64::from(b.text(l))).collect();
        b.rows_as_is(&format!("l:{name}"), &[col::TEXT], lines);
    }
    let image = b.finish();
    let raw = image.len();
    Ok((crate::content_store::compress(&image, LEVEL)?, raw))
}

/// The image of `bytes` (`encode`'s, `raw` long), checked.
fn image(bytes: &[u8], raw: usize) -> StoreResult<Image> {
    let mut words = vec![0u128; raw.div_ceil(16)];
    // SAFETY: `words` holds at least `raw` bytes, and any byte is a valid `u8`.
    let out = unsafe { std::slice::from_raw_parts_mut(words.as_mut_ptr().cast::<u8>(), raw) };
    crate::content_store::decompress_into(bytes, out).map_err(|e| bad(format!("does not decompress: {e}")))?;
    Image::own(words, raw).map_err(bad)
}

/// Each string the symbol columns of the relations `keep` takes hold (by the image's index), as this process's atom.
fn atoms_of(image: &Image, keep: &dyn Fn(&str) -> bool, atoms: &Atoms) -> StoreResult<HashMap<u32, i64>> {
    let mut wanted: Vec<u32> = Vec::new();
    for (name, rel) in image.relations() {
        let Some(key) = name.strip_prefix("r:") else { continue };
        if !keep(key) {
            continue;
        }
        for (c, &t) in rel.types().iter().enumerate() {
            if t == col::TEXT {
                wanted.extend(rel.col(c).iter().map(|&v| v as u32));
            }
        }
    }
    wanted.sort_unstable();
    wanted.dedup();
    let strings: Vec<&str> = wanted.iter().map(|&i| image.string(i)).collect();
    let got = atoms.intern_all(&strings)?;
    Ok(wanted.into_iter().zip(got).map(|(i, a)| (i, i64::from(a))).collect())
}

/// Holds the atoms of the entry's symbols in this process (interned, or found where the store holds them), before a
/// run's waves: so a wave decoding the entry finds each in memory.
pub(super) fn hold(bytes: &[u8], raw: usize, atoms: &Atoms) -> StoreResult<()> {
    atoms_of(&image(bytes, raw)?, &|_| true, atoms).map(drop)
}

/// The entry of `bytes` (`encode`'s; its key is its row's), keeping only the relations `keep` takes: the others are
/// checked with the image, not read. Its symbols are this process's atoms of their strings.
pub(super) fn decode(bytes: &[u8], raw: usize, keep: &dyn Fn(&str) -> bool, atoms: &Atoms) -> StoreResult<Entry> {
    let image = image(bytes, raw)?;
    let ids = atoms_of(&image, keep, atoms)?;
    let mut entry = Entry::default();
    let mut columns: HashMap<&str, Vec<usize>> = HashMap::new();
    for (name, rel) in image.relations() {
        if name == NODES {
            entry.nodes = rel.col(0).first().copied().unwrap_or(0) as u64;
        } else if let Some(key) = name.strip_prefix("r:") {
            if !keep(key) {
                continue;
            }
            let types = rel.types();
            let mut values = Vec::with_capacity(rel.rows * rel.width);
            for i in 0..rel.rows {
                for (c, &t) in types.iter().enumerate() {
                    let v = rel.col(c)[i];
                    values.push(if t == col::TEXT { *ids.get(&(v as u32)).ok_or_else(|| bad("names a string not interned"))? } else { v });
                }
            }
            entry.rows.insert(key.to_owned(), (rel.width, values));
        } else if let Some(out) = name.strip_prefix("c:") {
            columns.insert(out, rel.col(0).iter().map(|&c| c as usize).collect());
        } else if let Some(out) = name.strip_prefix("l:") {
            let lines = rel.col(0).iter().map(|&v| image.string(v as u32).to_owned()).collect();
            entry.lines.insert(out.to_owned(), (columns.remove(out).ok_or_else(|| bad(format!("has lines of {out} before its columns")))?, lines));
        } else {
            return Err(bad(format!("has a relation of no kind ({name})")));
        }
    }
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An entry reads back as it was made: its rows in their order and number, its symbols the same atoms, its lines
    /// and their node columns, its nodes; a relation not taken is not read; bytes cut short are refused.
    #[test]
    fn an_entry_reads_back_as_made_its_symbols_by_content() {
        use super::super::super::super::lower::Domain;
        let (db, db2) = (rusqlite::Connection::open_in_memory().unwrap(), rusqlite::Connection::open_in_memory().unwrap());
        let atoms = Atoms::for_connection(&db);
        let (x, y) = (atoms.intern("x").unwrap(), atoms.intern("ünï").unwrap());
        let domains: std::collections::BTreeMap<String, Vec<Domain>> = [("named".to_owned(), vec![Domain::Int, Domain::Name])].into();
        let syms = Syms::of(&domains);
        let mut entry = Entry { key: 5, nodes: 7, ..Entry::default() };
        entry.rows.insert("named".into(), (2, vec![3, i64::from(y), 1, i64::from(x), 3, i64::from(y)]));
        entry.rows.insert("wide".into(), (11, (0..22).collect()));
        entry.rows.insert("empty".into(), (2, Vec::new()));
        entry.rows.insert("bare".into(), (0, Vec::new()));
        entry.lines.insert("violation".into(), (vec![1], vec!["r\t21474836480\tx".into(), "a".into()]));
        entry.lines.insert("none".into(), (vec![], vec![String::new(), "ünïcode".into()]));
        let (bytes, raw) = encode(&entry, &syms, &atoms).unwrap();
        assert_eq!(decode(&bytes, raw, &|_| true, &atoms).map(|e| Entry { key: 5, ..e }).unwrap(), entry);
        let some = decode(&bytes, raw, &|k| k == "named", &atoms).unwrap();
        assert_eq!(some.rows.keys().collect::<Vec<_>>(), ["named"]);
        assert_eq!(some.lines, entry.lines, "outputs are read whatever relations are taken");
        assert!(decode(&bytes[..bytes.len() - 1], raw, &|_| true, &atoms).is_err(), "cut short");
        let other = Atoms::for_connection(&db2);
        other.intern("w").unwrap();
        let z = other.intern("z").unwrap();
        let back = decode(&bytes, raw, &|_| true, &other).unwrap();
        let named = &back.rows["named"].1;
        assert_eq!(other.with_str(named[1] as Sym, str::to_owned).as_deref(), Some("ünï"), "another table reads the strings");
        assert_ne!(named[3], i64::from(z));
        assert_eq!(named[1], named[5], "one string, one atom");
    }
}
