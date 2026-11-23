//! Decision 0293 §3: each language's format 16 choices, written for the engine to ship
//! (`native/crates/tape_format/tables/<tag>.tsv`, read by `rerail_tape_format::f16::lang`).
//!
//! - **The table**: every code the grammar allows (E25, `gcodes`), as the engine's ids: a kind name expanded to
//!   every id of that name, a field by its id, supertypes by theirs. What it holds is the grammar's alone (decisions
//!   0626, 0631): a file of any shape is written in format 16. Its order is a corpus's (`Counts`, decision 0638): the codes most nodes
//!   have first, so they are below the cutoff and read without the side list; a code no counted node has, by the
//!   plain codes first, then kind, field and memberships.
//! - **The widths**: a byte each; a wider value is escaped to its side list.
//! - **The revision**: the language's (`f16::revision`).
//! - **The names** (`f16::Names`): each kind id a node can have with its name, the names that look up another id, and
//!   each field's name, as `kinds.tsv` has them: what a parser without the grammar numbers its nodes by.

use crate::gcodes::{Ids, Tuple};
use rerail_tape_format::f16::{Lang, Names};
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

/// How often each code is a node's over a corpus (`rerail-engine code-counts`), by tag and entry: what a table is
/// numbered by. The counts name codes by the shipped table's numbers, read here as entries.
#[derive(Default)]
pub struct Counts {
    by: HashMap<(u8, Entry), u64>,
    /// Each tag's files and nodes counted.
    langs: HashMap<u8, (u64, u64)>,
    pub nodes: u64,
}

impl Counts {
    /// The counts of `paths` (`tag<TAB>number<TAB>count` lines, `nodes` and `revision` lines, `#` comments), each
    /// number an entry of the shipped table of its tag; a file whose revision is not the shipped table's is refused.
    pub fn read(paths: &[&str]) -> Result<Self, String> {
        let mut c = Counts::default();
        for p in paths {
            let text = std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"))?;
            for line in text.lines().filter(|l| !l.starts_with('#') && !l.is_empty()) {
                let cells: Vec<&str> = line.split('\t').collect();
                let num = |i: usize| cells.get(i).and_then(|c| c.parse::<u64>().ok()).ok_or_else(|| format!("{p}: {line:?}"));
                if cells[0] == "revision" {
                    let (tag, revision) = (num(1)? as u8, num(2)?);
                    let lang = rerail_tape_format::f16::lang(tag).ok_or_else(|| format!("{p}: tag {tag} has no table"))?;
                    if u64::from(lang.revision()) != revision {
                        return Err(format!("{p}: tag {tag}'s counts number revision {revision}'s table, not the shipped {}", lang.revision()));
                    }
                    continue;
                }
                if cells[0] == "nodes" {
                    let e = c.langs.entry(num(1)? as u8).or_default();
                    (e.0, e.1) = (e.0 + num(2)?, e.1 + num(3)?);
                    c.nodes += num(3)?;
                    continue;
                }
                let (tag, number, n) = (num(0)? as u8, num(1)? as u32, num(2)?);
                let lang = rerail_tape_format::f16::lang(tag).ok_or_else(|| format!("{p}: tag {tag} has no table"))?;
                if number as usize >= lang.table().len() {
                    return Err(format!("{p}: tag {tag} has no code {number}"));
                }
                *c.by.entry((tag, lang.table().entry(number).clone())).or_default() += n;
            }
        }
        Ok(c)
    }
    fn of(&self, tag: u8, e: &Entry) -> u64 {
        self.by.get(&(tag, e.clone())).copied().unwrap_or(0)
    }
}

/// Language `tag`'s choices, from its grammar alone (the user, 2026-11-22: "it has to be grammar. We don't have
/// exhaustive corpora, so how would specializing to a corpus possibly be correct?"): every code the grammar allows
/// (`gcodes::allowed`, from `scratch`'s generated node types where none are shipped; an own parser's bound), the
/// plain codes first, then by kind, field and memberships; widths of a byte (a wider value is escaped). Written to
/// `out/<tag>.tsv`.
pub fn run(tag: u8, scratch: &Path, out: &Path, counts: &Counts) -> Result<Report, String> {
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
    table.sort_by(|a, b| (std::cmp::Reverse(counts.of(tag, a)), a.flags, a.kind, a.field, &a.classes).cmp(&(std::cmp::Reverse(counts.of(tag, b)), b.flags, b.kind, b.field, &b.classes)));
    // The codes a table holds are the grammar's: a numbering changes their order alone. Where a table is shipped, the
    // grammar's set is held to it (a grammar changed since is written by a run without counts).
    if counts.nodes > 0 && let Some(shipped) = rerail_tape_format::f16::lang(tag) {
        let mut was: Vec<&Entry> = shipped.table().entries().iter().collect();
        let mut now: Vec<&Entry> = table.iter().collect();
        was.sort();
        now.sort();
        if was != now {
            return Err(format!("the grammar's codes ({}) are not the shipped table's ({}): regenerate without counts first", now.len(), was.len()));
        }
    }
    let names = names_of(of, &ids, &table);
    let lang = Lang::new(CodeTable::new(table), [1, 1, 1], crate::f16::revision(tag))?.with_names(names);
    let comment = format!(
        "Tape format 16's choices for tape tag {tag} ({}), decisions 0293 §3 and 0626. Written by the lab from the grammar:\n  tape16 table <tags…> [--counts <code-counts.tsv>…] (native/lab/tape16/src/table.rs)\n{} codes{}{}.",
        crate::capture::language(tag),
        lang.table().len(),
        if bound { "; no node types declared: the grammar's list is the bound" } else { "" },
        match counts.langs.get(&tag) {
            Some((files, nodes)) => format!("; numbered by how often each is a node's over {files} files, {nodes} nodes (`rerail-engine code-counts`)"),
            None if counts.nodes > 0 => "; no file of it counted: in the grammar's order".to_owned(),
            None => String::new(),
        }
    );
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    std::fs::write(out.join(format!("{tag}.tsv")), lang.to_text(&comment)).map_err(|e| e.to_string())?;
    Ok(Report { tag, codes: lang.table().len(), bound })
}

/// Language `of`'s names, as tree-sitter's lookups give them: each name and namedness with the id its lookup gives
/// (its public symbol, `id_for_node_kind`), in id order of the names' first symbols; each id a lookup gives, with its
/// own name; each field by id; each supertype id the codes name as a class (a hidden symbol, no node's kind), with
/// its name; and each class name's lookup as a named kind (the supertype a parser names), where the codes have it.
fn names_of(of: u8, ids: &Ids, table: &[Entry]) -> Names {
    let mut names = Names::default();
    let mut kinds: Vec<(u16, &(String, bool))> = ids.kinds.iter().filter(|((t, _), _)| *t == of).map(|((_, id), k)| (*id, k)).collect();
    kinds.sort_unstable_by_key(|k| k.0);
    let mut publics = BTreeSet::new();
    for &(id, k) in &kinds {
        let public = ids.public[&(of, id)];
        if public == 0 {
            continue;
        }
        let lookup = (k.1, k.0.clone(), public);
        if !names.lookups.iter().any(|l| (l.0, &l.1) == (k.1, &k.0)) {
            names.lookups.push(lookup);
        }
        publics.insert(public);
    }
    names.kinds = publics.into_iter().map(|id| (id, ids.kinds[&(of, id)].1, ids.kinds[&(of, id)].0.clone())).collect();
    let mut fields: Vec<(u16, String)> = ids.fields.iter().filter(|((t, _), _)| *t == of).map(|((_, id), name)| (u16::from(*id), name.clone())).collect();
    fields.sort_unstable();
    names.fields = fields;
    let classes: BTreeSet<u16> = table.iter().flat_map(|e| e.classes.iter().copied()).collect();
    names.classes = classes.iter().map(|&id| (id, ids.kinds[&(of, id)].0.clone())).collect();
    let mut supertypes: Vec<(String, u16)> = Vec::new();
    for &id in &classes {
        let (name, at) = (&ids.kinds[&(of, id)].0, ids.as_named[&(of, id)]);
        if classes.contains(&at) && !supertypes.iter().any(|s| &s.0 == name) {
            supertypes.push((name.clone(), at));
        }
    }
    names.supertypes = supertypes;
    names
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
