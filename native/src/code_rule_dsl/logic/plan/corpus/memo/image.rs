//! A memo entry as stored (decision 0240 §4): one output image (decision 0239), compressed, its symbols by content.
//!
//! The entry's relations are the image's, each as made (`Builder::rows_as_is`: in its order, every row): a relation
//! `r:<key>` of the rows under `key` (`Entry::rows`), its symbol columns (the plan's, `Syms`) text, the others
//! integers; for each output, `c:<name>` its node columns, `o:<name>` its rows typed as the run handed them (decision
//! 0239: a text column's value one of the image's strings), and `n:<name>` their number where they have no columns;
//! `#nodes` the document's nodes. A symbol is held as its string, so an entry reads the same in every process: a process that
//! numbered the store's atoms otherwise (a fork, decision 0170) reads it as any other. Before, the entry held the
//! store's ids, and each read and write mapped them (`numbering`: `to_store`, `to_process`, and every entry's ids
//! held before the waves). The image is stored as made, not compressed (decision 0295 §2), and read where it is in
//! its segment (`SegmentBytes`).

use super::{Entry, Syms};
use crate::code_rule_dsl::logic::plan::output::{Builder, Finished, Image, Rows, col};
use crate::content_store::atoms::{Atoms, Sym};
use crate::content_store::{SegmentBytes, StoreError, StoreResult};
use std::collections::HashMap;

const NODES: &str = "#nodes";

fn bad(what: impl std::fmt::Display) -> StoreError {
    StoreError::Invalid(format!("a corpus memo entry {what}"))
}

/// The entry's image (`encode_parts`), as bytes.
#[cfg(test)]
pub(super) fn encode(entry: &Entry, syms: &Syms, atoms: &Atoms) -> StoreResult<Vec<u8>> {
    let made = encode_parts(entry, syms, atoms)?;
    let mut out = vec![0u8; made.len()];
    made.write(&mut out);
    Ok(out)
}

/// The entry's image, made and not yet written (`Finished::write`: into its segment): its symbols' strings read from
/// `atoms`, the table its rows were made under.
pub(super) fn encode_parts(entry: &Entry, syms: &Syms, atoms: &Atoms) -> StoreResult<Finished> {
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
    for (name, (nodes, rows)) in &entry.outputs {
        b.rows_as_is(&format!("c:{name}"), &[col::INT], nodes.iter().map(|&c| c as i64).collect());
        if rows.width() == 0 {
            b.rows_as_is(&format!("n:{name}"), &[col::INT], vec![rows.len() as i64]);
        }
        let types = rows.types();
        let mut values = Vec::with_capacity(rows.len() * types.len());
        for i in 0..rows.len() {
            for (c, &v) in rows.row(i).iter().enumerate() {
                values.push(if types[c] == col::TEXT { i64::from(b.text(rows.text(v))) } else { v });
            }
        }
        b.rows_as_is(&format!("o:{name}"), types, values);
    }
    Ok(b.finish_parts())
}

/// The image of `bytes` (`encode`'s), checked, read where it is.
fn image(bytes: &SegmentBytes) -> StoreResult<Image> {
    let (block, at) = bytes.block();
    Image::in_block(block, at, bytes.len()).map_err(bad)
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
pub(super) fn hold(bytes: &SegmentBytes, atoms: &Atoms) -> StoreResult<()> {
    atoms_of(&image(bytes)?, &|_| true, atoms).map(drop)
}

/// The entry of `bytes` (`encode`'s; its key is its row's), keeping only the relations `keep` takes: the others are
/// checked with the image, not read. Its symbols are this process's atoms of their strings.
pub(super) fn decode(bytes: &SegmentBytes, keep: &dyn Fn(&str) -> bool, atoms: &Atoms) -> StoreResult<Entry> {
    let image = image(bytes)?;
    let ids = atoms_of(&image, keep, atoms)?;
    let mut entry = Entry::default();
    let mut columns: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut counts: HashMap<&str, usize> = HashMap::new();
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
        } else if let Some(out) = name.strip_prefix("n:") {
            counts.insert(out, rel.col(0).first().copied().unwrap_or(0) as usize);
        } else if let Some(out) = name.strip_prefix("o:") {
            // The output's texts, each once, in the order its rows name them.
            let types = rel.types().to_vec();
            let (mut texts, mut at): (Vec<String>, HashMap<u32, i64>) = (Vec::new(), HashMap::new());
            let mut vals = Vec::with_capacity(rel.rows * rel.width);
            for i in 0..rel.rows {
                for (c, &t) in types.iter().enumerate() {
                    let v = rel.col(c)[i];
                    vals.push(if t == col::TEXT {
                        *at.entry(v as u32).or_insert_with(|| {
                            texts.push(image.string(v as u32).to_owned());
                            texts.len() as i64 - 1
                        })
                    } else {
                        v
                    });
                }
            }
            let count = counts.remove(out).unwrap_or(rel.rows);
            let rows = Rows::of_parts(types, vals, texts, count).map_err(|e| bad(format!("has rows of {out} that do not hold: {e}")))?;
            entry.outputs.insert(out.to_owned(), (columns.remove(out).ok_or_else(|| bad(format!("has rows of {out} before its columns")))?, rows));
        } else {
            return Err(bad(format!("has a relation of no kind ({name})")));
        }
    }
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::tests::rows_of;
    use crate::code_rule_dsl::logic::plan::output::Cell;

    /// An entry reads back as it was made: its rows in their order and number, its symbols the same atoms, its
    /// outputs' typed rows (a bare output's number too) and their node columns, its nodes; a relation not taken is not read; bytes cut short are refused.
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
        entry.outputs.insert("violation".into(), (vec![1], rows_of(&[col::TEXT, col::NODE, col::TEXT], &[&[Cell::Text("r"), Cell::Int(21474836480), Cell::Text("x")], &[Cell::Text("a"), Cell::Int(3), Cell::Text("r")]])));
        entry.outputs.insert("a name longer than one row of text holds, which is fifty-six bytes".into(), (vec![], rows_of(&[col::TEXT, col::INT], &[&[Cell::Text(""), Cell::Int(-3)], &[Cell::Text("ünïcode"), Cell::Int(i64::MAX)]])));
        entry.outputs.insert("bare".into(), (vec![], rows_of(&[], &[&[], &[]])));
        let made = encode(&entry, &syms, &atoms).unwrap();
        let bytes = SegmentBytes::of(&made);
        assert_eq!(decode(&bytes, &|_| true, &atoms).map(|e| Entry { key: 5, ..e }).unwrap(), entry);
        let some = decode(&bytes, &|k| k == "named", &atoms).unwrap();
        assert_eq!(some.rows.keys().collect::<Vec<_>>(), ["named"]);
        assert_eq!(some.outputs, entry.outputs, "outputs are read whatever relations are taken");
        assert!(decode(&SegmentBytes::of(&made[..made.len() - 1]), &|_| true, &atoms).is_err(), "cut short");
        let other = Atoms::for_connection(&db2);
        other.intern("w").unwrap();
        let z = other.intern("z").unwrap();
        let back = decode(&bytes, &|_| true, &other).unwrap();
        let named = &back.rows["named"].1;
        assert_eq!(other.with_str(named[1] as Sym, str::to_owned).as_deref(), Some("ünï"), "another table reads the strings");
        assert_ne!(named[3], i64::from(z));
        assert_eq!(named[1], named[5], "one string, one atom");
    }
}
