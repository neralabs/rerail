//! Decision 0293 §3: each language's format 16 choices, written for the engine to ship
//! (`native/crates/tape_format/tables/<tag>.tsv`, read by `rerail_tape_format::f16::lang`).
//!
//! - **The table**: every code the grammar allows (E25, `gcodes`), as the engine's ids: a kind name expanded to
//!   every id of that name, a field by its id, supertypes by theirs; the plain codes first, then by kind, field and
//!   memberships. No corpus orders or bounds it (decision 0626): a file of any shape is written in format 16.
//! - **The widths**: a byte each; a wider value is escaped to its side list.
//! - **The revision**: the language's (`f16::revision`).

use crate::gcodes::{Ids, Tuple};
use rerail_tape_format::f16::Lang;
use rerail_tape_format::tape::{CodeTable, Entry};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

/// A node's kind on the tape: its public symbol, and its type's name where that symbol is named otherwise.
type Kind = (u16, Option<(String, bool)>);

pub struct Report {
    pub tag: u8,
    pub codes: usize,
    /// No node types declared (an own parser): the grammar's list is the bound.
    pub bound: bool,
}

/// The engine's entries for a code the grammar allows, by names: each kind name to its codes' kinds (`by_name`), each
/// supertype name to every id of that name. A supertype the engine's ids lack is no membership: its grammar names it (`supertypes`), but the
/// parser the generator wrote keeps no symbol of it (Rust's `_declaration_statement`, Java's `_simple_type`, Scala's
/// `_pattern`), so no node is under one. `Err` names a kind or field the ids lack: a grammar other than the engine's.
fn entries_of(t: &Tuple, by_name: &HashMap<(String, bool), Vec<Kind>>, fields: &HashMap<String, u16>, supers: &HashMap<String, Vec<u16>>) -> Result<Vec<Entry>, String> {
    let kinds: Vec<Kind> = if t.kind.0 == "ERROR" {
        vec![(rerail_tape_format::ERROR_CODE, None)]
    } else {
        // A name as the C API reads it, to its first NUL: Go's `"\0"` token is named "" there.
        let name = (t.kind.0.split('\0').next().unwrap_or("").to_owned(), t.kind.1);
        by_name.get(&name).cloned().ok_or_else(|| format!("kind {:?} has no id", t.kind))?
    };
    let field = if t.field.is_empty() { 0 } else { *fields.get(&t.field).ok_or_else(|| format!("field {:?} has no id", t.field))? };
    let mut classes: Vec<Vec<u16>> = vec![Vec::new()];
    for s in &t.supers {
        let Some(ids) = supers.get(s).cloned() else { continue };
        classes = classes.iter().flat_map(|c| ids.iter().map(move |&id| c.iter().copied().chain([id]).collect::<Vec<_>>())).collect();
    }
    let mut out = Vec::new();
    for (kind, alias) in &kinds {
        for c in &classes {
            let mut c = c.clone();
            c.sort_unstable();
            c.dedup();
            out.push(Entry { kind: *kind, field, flags: t.flags, alias: alias.clone(), classes: c });
        }
    }
    Ok(out)
}

/// Language `tag`'s choices, from its grammar alone (the user, 2026-11-22: "it has to be grammar. We don't have
/// exhaustive corpora, so how would specializing to a corpus possibly be correct?"): every code the grammar allows
/// (`gcodes::allowed`, from `scratch`'s generated node types where none are shipped; an own parser's bound), the
/// plain codes first, then by kind, field and memberships; widths of a byte (a wider value is escaped). Written to
/// `out/<tag>.tsv`.
pub fn run(tag: u8, scratch: &Path, out: &Path) -> Result<Report, String> {
    let ids = Ids::load();
    let of = crate::gcodes::ids_tag(tag);
    let (allowed, bound) = match crate::gcodes::load(tag, scratch) {
        Ok((grammar, types)) => (crate::gcodes::allowed(&grammar, &types), false),
        Err(_) if ids.kinds.keys().any(|(t, _)| *t == of) => (crate::gcodes::own_bound(of, &ids), true),
        Err(e) => return Err(e),
    };
    // Names to the engine's codes, for this language, as tree-sitter's encoding gives a node them (`encode_node`):
    // its kind is its type's public symbol (`ts_node_symbol`), so the grammar's other symbols of a name and kind
    // (Kotlin's four `call_expression`s) are on no tape; where that symbol is named otherwise, the node's type is its
    // contextual alias (Scala's `(`, whose public symbol is named `arguments`). A supertype is its own symbol (a
    // cursor's status names it as the parse table does).
    let mut by_name: HashMap<(String, bool), Vec<Kind>> = HashMap::new();
    let mut supers: HashMap<String, Vec<u16>> = HashMap::new();
    for ((t, id), k) in &ids.kinds {
        if *t == of {
            let public = ids.public[&(*t, *id)];
            if let Some(p) = ids.kinds.get(&(*t, public)).filter(|_| public != 0) {
                by_name.entry(k.clone()).or_default().push((public, (p != k).then(|| k.clone())));
            }
            supers.entry(k.0.clone()).or_default().push(*id);
        }
    }
    for v in by_name.values_mut() {
        v.sort_unstable();
        v.dedup();
    }
    for v in supers.values_mut() {
        v.sort_unstable();
    }
    let fields: HashMap<String, u16> = ids.fields.iter().filter(|((t, _), _)| *t == of).map(|((_, id), name)| (name.clone(), u16::from(*id))).collect();
    let mut codes: BTreeSet<Entry> = BTreeSet::new();
    for t in allowed.tuples.keys() {
        codes.extend(entries_of(t, &by_name, &fields, &supers).map_err(|e| format!("{}: {e}", crate::capture::language(tag)))?);
    }
    let mut table: Vec<Entry> = codes.into_iter().collect();
    table.sort_by(|a, b| (a.flags, a.kind, a.field, &a.classes).cmp(&(b.flags, b.kind, b.field, &b.classes)));
    let lang = Lang::new(CodeTable::new(table), [1, 1, 1], crate::f16::revision(tag))?;
    let comment = format!(
        "Tape format 16's choices for tape tag {tag} ({}), decisions 0293 §3 and 0626. Written by the lab from the grammar:\n  tape16 table <tags…> (native/lab/tape16/src/table.rs)\n{} codes{}.",
        crate::capture::language(tag),
        lang.table().len(),
        if bound { "; no node types declared: the grammar's list is the bound" } else { "" }
    );
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    std::fs::write(out.join(format!("{tag}.tsv")), lang.to_text(&comment)).map_err(|e| e.to_string())?;
    Ok(Report { tag, codes: lang.table().len(), bound })
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
