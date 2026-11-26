//! A capture's tapes (`rerail-engine capture-tapes`: format 16, as the engine writes them) read into a `Tree`
//! where they lie, by the tape's own reads (`rerail_tape_format::reads::f16`) and its language's table, and their
//! stored size as the store keeps them (`content_store::compression::compress_as`: zstd level 3, and below 64 KiB
//! the smaller of a plain frame and one with the language's dictionary).

use crate::tree::{Node, Tree};
use rerail_tape_format::f16::{self, Lang, MAGIC16};
use rerail_tape_format::reads::f16 as reads;

/// The tape's language's table, by the tag its header states.
fn lang_of(bytes: &[u8]) -> Result<&'static Lang, String> {
    if !bytes.starts_with(MAGIC16) {
        return Err("not a format 16 tape".into());
    }
    let tag = *bytes.get(f16::head::LANGUAGE).ok_or("a short tape header")?;
    f16::lang(tag).ok_or_else(|| format!("tape tag {tag} has no format 16 table"))
}

/// The tape's contextual alias names (its language's table's aliases, in the table's order), by the index a node's
/// `0x8000 | alias` kind names: each (name, named).
pub fn aliases(bytes: &[u8]) -> Vec<(String, bool)> {
    let Ok(lang) = lang_of(bytes) else { return Vec::new() };
    let mut out: Vec<(String, bool)> = Vec::new();
    for e in lang.table().entries() {
        if let Some(a) = &e.alias
            && !out.contains(a)
        {
            out.push(a.clone());
        }
    }
    out
}

pub fn read(bytes: &[u8]) -> Result<Tree, String> {
    let lang = lang_of(bytes)?;
    let v = f16::placement(bytes)?.view(lang, 0)?;
    let aliases = aliases(bytes);
    let mut nodes = Vec::with_capacity(v.nodes as usize);
    let mut sizes = Vec::with_capacity(v.nodes as usize);
    for i in 0..v.nodes {
        let e = lang.table().entry(reads::number(bytes, &v, i));
        // An alias is its own kind here (format 16: the grammar's alias range), so it shows in the kind.
        let kind = match &e.alias {
            Some(a) => 0x8000 | aliases.iter().position(|x| x == a).expect("the table's alias") as u16,
            None => e.kind,
        };
        nodes.push(Node { kind, field: e.field as u8, flags: e.flags, classes: e.classes.clone(), start: u64::from(reads::start(bytes, &v, i)), end: u64::from(reads::end(bytes, &v, i)) });
        sizes.push(reads::size(bytes, &v, i));
    }
    let parent = Tree::parents_from_sizes(&sizes);
    Ok(Tree { nodes, size: sizes, parent, source_len: u64::from(v.source_len) })
}

/// The store's dictionary for a tape of tape tag `tag` (`native/dictionaries`), if it has one.
pub fn store_tape_dictionary(tag: u8) -> Option<Vec<u8>> {
    let name = match tag {
        1 => "python",
        2 | 3 => "typescript",
        6 => "javascript",
        7 => "php",
        9 => "rust",
        10 => "sql",
        _ => return None,
    };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dictionaries");
    std::fs::read(dir.join(format!("{name}-tape.zdict"))).ok()
}

/// The store's source dictionary for tape tag `tag`, if it has one.
pub fn store_source_dictionary(tag: u8) -> Option<Vec<u8>> {
    let name = match tag {
        1 => "python",
        2 | 3 => "typescript",
        6 => "javascript",
        7 => "php",
        9 => "rust",
        _ => return None,
    };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../dictionaries");
    std::fs::read(dir.join(format!("{name}-source.zdict"))).ok()
}

/// Bytes as the store keeps them: zstd 3; below 64 KiB the smaller of plain and with `dictionary`.
pub fn stored_size(bytes: &[u8], dictionary: Option<&[u8]>) -> usize {
    let plain = zstd::bulk::compress(bytes, 3).expect("zstd").len();
    match dictionary {
        Some(d) if bytes.len() < 64 * 1024 => {
            let mut c = zstd::bulk::Compressor::with_dictionary(3, d).expect("zstd dictionary");
            plain.min(c.compress(bytes).expect("zstd").len())
        }
        _ => plain,
    }
}
