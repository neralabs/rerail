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

use rerail_blocks::vector::Vector;
use super::{Entry, Syms};
use crate::code_rule_dsl::logic::plan::output::{Builder, Finished, Image, Rows, col};
use crate::content_store::atoms::{Atoms, Sym};
use crate::content_store::{SegmentBytes, StoreError, StoreResult};
use std::collections::HashMap;

const NODES: &str = "#nodes";
/// How many of the image's first strings are its symbols' (`encode_parts` writes them first); then one past the
/// largest atom id they had where the entry was made (the store's atoms handle), where it names any: a process holding
/// every stored atom below it, and not forked from the store's (decision 0170), holds the entry's symbols already, so
/// `hold` reads nothing of the entry. Only a hint for that: the entry reads by its strings, in every process. An entry
/// written before it has one row.
const SYMS: &str = "#syms";

fn bad(what: impl std::fmt::Display) -> StoreError {
    StoreError::Invalid(format!("a corpus memo entry {what}"))
}

/// The entry's image (`encode_parts`), as bytes.
#[cfg(any(test, feature = "test-hooks"))]
pub fn encode(entry: &Entry, syms: &Syms, atoms: &Atoms) -> StoreResult<Vec<u8>> {
    let made = encode_parts(entry, syms, atoms)?;
    let mut out = vec![0u8; made.len()];
    made.write(&mut out);
    Ok(out)
}

/// The entry's image, made and not yet written (`Finished::write`: into its segment): its symbols' strings read from
/// `atoms`, the table its rows were made under.
pub(super) fn encode_parts(entry: &Entry, syms: &Syms, atoms: &Atoms) -> StoreResult<Finished> {
    let mut b = Builder::new(0, false);
    // The symbols' strings first, so they are the image's strings `0..SYMS`: holding the entry's atoms reads them
    // there, not every row (`hold`).
    let mut shown: HashMap<i64, i64> = HashMap::new();
    for (key, (w, values)) in &entry.rows {
        let w = *w;
        if w == 0 {
            continue;
        }
        let Some(cols) = syms.columns(key, w).filter(|c| c.len() == w) else { continue };
        for row in values.chunks_exact(w) {
            for (c, &v) in row.iter().enumerate() {
                if cols[c] && !shown.contains_key(&v) {
                    let atom = Sym::try_from(v).map_err(|_| bad(format!("of {key}: symbol {v} is no atom")))?;
                    let at = atoms.with_str(atom, |s| i64::from(b.text(s))).ok_or_else(|| bad(format!("of {key}: symbol {v} is no atom of this process's")))?;
                    shown.insert(v, at);
                }
            }
        }
    }
    let end = shown.keys().copied().max().map_or(0, |m| m + 1);
    b.rows_as_is(SYMS, &[col::INT], rerail_blocks::vector![shown.len() as i64, end]);
    b.rows_as_is(NODES, &[col::INT], rerail_blocks::vector![entry.nodes as i64]);
    for (key, (w, values)) in &entry.rows {
        let w = *w;
        let name = format!("r:{key}");
        if w == 0 {
            b.rows_as_is(&name, &[], rerail_blocks::vector::Vector::new());
            continue;
        }
        let cols = syms.columns(key, w).filter(|c| c.len() == w).unwrap_or_else(|| vec![false; w]);
        let types: Vec<u8> = cols.iter().map(|&s| if s { col::TEXT } else { col::INT }).collect();
        let mut rows = rerail_blocks::vector::Vector::with_capacity(values.len());
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
            b.rows_as_is(&format!("n:{name}"), &[col::INT], rerail_blocks::vector![rows.len() as i64]);
        }
        let types = rows.types();
        let mut values = rerail_blocks::vector::Vector::with_capacity(rows.len() * types.len());
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
    let mut wanted: Vector<u32> = rerail_blocks::vector::Vector::new();
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
/// All of `entries` at once: one lookup of what this process lacks, not one per entry.
/// An entry whose symbols this process holds already (`SYMS`'s end at or below `atoms.held_below()`) reads nothing.
/// Returns how many entries it read.
pub fn hold(entries: &[&SegmentBytes], atoms: &Atoms) -> StoreResult<usize> {
    let held = atoms.held_below();
    let images = entries.iter().map(|b| image(b)).collect::<StoreResult<Vec<_>>>()?;
    let mut strings: Vec<&str> = Vec::new();
    let mut wanted: Vector<u32> = rerail_blocks::vector::Vector::new();
    let mut read = 0;
    for image in &images {
        let syms = image.rel(SYMS).map(|r| r.col(0));
        if let (Some(&[_, end]), Some(held)) = (syms, held)
            && end as usize <= held
        {
            continue;
        }
        read += 1;
        if let Some(n) = syms.and_then(|c| c.first().copied()) {
            strings.extend((0..n as u32).map(|i| image.string(i)));
            continue;
        }
        // An entry written before `SYMS`: its symbol columns read.
        // Each image's strings by index, once: a symbol column repeats them row after row.
        wanted.clear();
        for (name, rel) in image.relations() {
            if !name.starts_with("r:") {
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
        strings.extend(wanted.iter().map(|&i| image.string(i)));
    }
    // The strings many entries share, once, by hash: no string sort.
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::with_capacity(strings.len());
    strings.retain(|s| seen.insert(s));
    if strings.is_empty() {
        return Ok(read);
    }
    let _s = crate::code_rule_dsl::logic::profile::span(|| format!("memo lookup: the entries' {} strings interned", strings.len()));
    atoms.intern_all(&strings).map(|_| read)
}

/// The entry of `bytes` (`encode`'s; its key is its row's), keeping only the relations `keep` takes: the others are
/// checked with the image, not read. Its symbols are this process's atoms of their strings.
pub fn decode(bytes: &SegmentBytes, keep: &dyn Fn(&str) -> bool, atoms: &Atoms) -> StoreResult<Entry> {
    let image = image(bytes)?;
    let ids = atoms_of(&image, keep, atoms)?;
    let mut entry = Entry::default();
    let mut columns: HashMap<&str, Vector<usize>> = HashMap::new();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for (name, rel) in image.relations() {
        if name == SYMS {
            continue;
        } else if name == NODES {
            entry.nodes = rel.col(0).first().copied().unwrap_or(0) as u64;
        } else if let Some(key) = name.strip_prefix("r:") {
            if !keep(key) {
                continue;
            }
            let types = rel.types();
            let mut values = rerail_blocks::vector::Vector::with_capacity(rel.rows * rel.width);
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
            let mut vals = rerail_blocks::vector::Vector::with_capacity(rel.rows * rel.width);
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

// cratify: `tests` is rerail-logic's (it mounts it: it reads the language, `rr::parse_version`, or the corpus's root)
