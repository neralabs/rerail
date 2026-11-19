//! Decision 0293 §3: each language's format 16 choices, written for the engine to ship
//! (`native/crates/tape_format/tables/<tag>.tsv`, read by `rerail_tape_format::f16::lang`).
//!
//! - **The table**: every code the grammar allows (E25, `gcodes`), as the engine's ids: a kind name expanded to
//!   every id of that name, a field by its id, supertypes by theirs. The codes the captures show come first, the
//!   most frequent first (E21), as the engine's format 15 code tables hold them (an alias by its name); the rest
//!   follow in a fixed order. A code the captures show that the grammar's list lacks is kept, and counted.
//! - **The widths**: learned on the captures as E24 learned them (`F16Lang::learn`).
//! - **The revision**: the language's (`f16::revision`).

use crate::capture::File;
use crate::gcodes::{Ids, Tuple};
use rerail_tape_format::f16::Lang;
use rerail_tape_format::tape::{CodeTable, Entry, F15, Tape, f15_columns};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

pub struct Report {
    pub tag: u8,
    pub files: usize,
    pub nodes: u64,
    pub codes: usize,
    pub seen: usize,
    /// Seen codes the grammar's list lacks, and their nodes.
    pub holes: usize,
    pub hole_nodes: u64,
    pub widths: [usize; 3],
    pub bound: bool,
}

/// The engine's entries for a code the grammar allows, by names: each kind and supertype name expanded to every id
/// of that name.
fn entries_of(t: &Tuple, tag: u8, by_name: &HashMap<(String, bool), Vec<u16>>, fields: &HashMap<String, u16>, supers: &HashMap<String, Vec<u16>>) -> Vec<Entry> {
    let kinds: Vec<u16> = if t.kind.0 == "ERROR" { vec![rerail_tape_format::ERROR_CODE] } else { by_name.get(&t.kind).cloned().unwrap_or_default() };
    let Some(field) = (if t.field.is_empty() { Some(0) } else { fields.get(&t.field).copied() }) else { return Vec::new() };
    let mut classes: Vec<Vec<u16>> = vec![Vec::new()];
    for s in &t.supers {
        let ids = supers.get(s).cloned().unwrap_or_default();
        classes = classes.iter().flat_map(|c| ids.iter().map(move |&id| c.iter().copied().chain([id]).collect::<Vec<_>>())).collect();
    }
    let _ = tag;
    let mut out = Vec::new();
    for &kind in &kinds {
        for c in &classes {
            let mut c = c.clone();
            c.sort_unstable();
            c.dedup();
            out.push(Entry { kind, field, flags: t.flags, alias: None, classes: c });
        }
    }
    out
}

/// Language `tag`'s choices from `files` (one language), with the grammar's list from `scratch`'s generated node
/// types; written to `out/<tag>.tsv`.
pub fn run(tag: u8, files: &[&File], scratch: &Path, out: &Path) -> Result<Report, String> {
    let ids = Ids::load();
    let (allowed, bound) = match crate::gcodes::load(tag, scratch) {
        Ok((grammar, types)) => (crate::gcodes::allowed(&grammar, &types), false),
        Err(_) if ids.kinds.keys().any(|(t, _)| *t == tag) => (crate::gcodes::own_bound(tag, &ids), true),
        Err(e) => return Err(e),
    };
    // Names to the engine's ids, for this language.
    let mut by_name: HashMap<(String, bool), Vec<u16>> = HashMap::new();
    let mut supers: HashMap<String, Vec<u16>> = HashMap::new();
    for ((t, id), k) in &ids.kinds {
        if *t == tag {
            by_name.entry(k.clone()).or_default().push(*id);
            supers.entry(k.0.clone()).or_default().push(*id);
        }
    }
    for v in by_name.values_mut().chain(supers.values_mut()) {
        v.sort_unstable();
    }
    let fields: HashMap<String, u16> = ids.fields.iter().filter(|((t, _), _)| *t == tag).map(|((_, id), name)| (name.clone(), u16::from(*id))).collect();
    // The codes the captures show, by count, as format 15's tables hold them.
    let mut seen: HashMap<Entry, u64> = HashMap::new();
    let mut nodes = 0u64;
    for f in files {
        let (columns, table) = f15_columns(&f.image)?;
        let t = F15::new(&f.image, columns, &table)?;
        let mut count = vec![0u64; table.len()];
        for i in 0..t.len() {
            count[t.code(i) as usize] += 1;
        }
        nodes += t.len() as u64;
        for (k, c) in count.into_iter().enumerate() {
            if c > 0 {
                *seen.entry(table.entry(k as u32).clone()).or_default() += c;
            }
        }
    }
    let grammar: BTreeSet<Entry> = allowed.tuples.keys().flat_map(|t| entries_of(t, tag, &by_name, &fields, &supers)).collect();
    let mut first: Vec<(&Entry, u64)> = seen.iter().map(|(e, &c)| (e, c)).collect();
    first.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    let holes: Vec<&(&Entry, u64)> = first.iter().filter(|(e, _)| e.alias.is_none() && !grammar.contains(*e)).collect();
    let (hole_count, hole_nodes) = (holes.len(), holes.iter().map(|h| h.1).sum());
    let mut table: Vec<Entry> = first.iter().map(|(e, _)| (*e).clone()).collect();
    table.extend(grammar.iter().filter(|e| !seen.contains_key(*e)).cloned());
    // The widths, learned on the captures as E24 learned them.
    let mut codes = crate::inplace::Codes::default();
    let trees: Vec<crate::tree::Tree> = files.iter().map(|f| crate::v15::read(&f.image)).collect::<Result<_, _>>()?;
    for t in &trees {
        codes.learn(t);
    }
    let cols: Vec<crate::inplace::Cols> = trees.iter().map(|t| crate::inplace::Cols::of_tree(t, &codes)).collect();
    let learned = crate::f16::F16Lang::learn(&codes, table.len(), cols.iter());
    let widths = learned.widths.map(|w| (w as usize).div_ceil(8));
    let lang = Lang::new(CodeTable::new(table), widths, crate::f16::revision(tag))?;
    let comment = format!(
        "Tape format 16's choices for tape tag {tag} ({}), decision 0293 §3. Written by the lab:\n  tape16 table <captures…> (native/lab/tape16/src/table.rs)\nfrom {} files, {nodes} nodes; {} codes, {} seen first by count{}.",
        crate::capture::language(tag),
        files.len(),
        lang.table().len(),
        seen.len(),
        if bound { "; no node types declared: the grammar's list is the bound" } else { "" }
    );
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    std::fs::write(out.join(format!("{tag}.tsv")), lang.to_text(&comment)).map_err(|e| e.to_string())?;
    Ok(Report { tag, files: files.len(), nodes, codes: lang.table().len(), seen: seen.len(), holes: hole_count, hole_nodes, widths, bound })
}

/// `index.rs`: each written table's text by tag.
pub fn index(tags: &[u8], out: &Path) -> Result<(), String> {
    let mut s = String::from("// The tables the lab writes (`tape16 table`), by tape tag. Regenerated with them.\npub fn text(tag: u8) -> Option<&'static str> {\n    match tag {\n");
    for t in tags {
        s.push_str(&format!("        {t} => Some(include_str!(\"{t}.tsv\")),\n"));
    }
    s.push_str("        _ => None,\n    }\n}\n");
    std::fs::write(out.join("index.rs"), s).map_err(|e| e.to_string())
}
