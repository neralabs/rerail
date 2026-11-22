//! A memo entry of program facts kept as kept facts of the store (t33 step 3, decisions 0104 and 0456), where the
//! facts memo kept it as one encoded row (`facts_results`): each relation of the entry a kept relation of its user's
//! space, the entry's rows one partition (its key's), held by the source the entry was kept under and the plan it was
//! made under (`HOLDER`): the store's collection drops the partition once either goes, as the memo's row died with
//! them.
//!
//! A value is two words, its tag and its payload: 0 a number, 1 a string (the atom the store interned it as), 2 to 4 a
//! branded atom (a name, a kind, a field). A relation is kept as `ii` per column.

use super::super::front::Value;
use super::super::front::ast::{Branded, Class};
use super::super::program_facts::Facts;
use crate::content_store::ContentStore;
use crate::content_store::kept_facts::{KeptPut, KeptWhat, holder_put};
use rerail_blocks::kept::KeptRead as _;
use sha2::{Digest as _, Sha256};
use std::collections::BTreeSet;

/// One user's entries: its space, and the name its relations' identities are made of.
#[derive(Clone, Copy, Debug)]
pub struct Entries {
    pub name: &'static str,
}

impl Entries {
    pub const fn new(name: &'static str) -> Self {
        Entries { name }
    }

    pub fn space(&self) -> [u8; 32] {
        Sha256::digest(format!("{}-space/v1", self.name).as_bytes()).into()
    }

    fn ident(&self, rel: &str) -> [u8; 32] {
        Sha256::digest(format!("{}/v1\0{rel}", self.name).as_bytes()).into()
    }

    /// The partition of the entry keyed `key`.
    pub fn part(key: &[u8; 32]) -> i64 {
        i64::from_le_bytes(key[..8].try_into().expect("8 bytes")) & i64::MAX
    }

    /// The entry of partition `part`, as kept; `None` when the store keeps none (or one that no longer reads).
    pub fn get(&self, store: &ContentStore, part: i64) -> Option<Facts> {
        let kept = store.kept_space(&self.space()).ok()??;
        let chain = kept.view();
        let mut facts = Facts::new();
        for entry in chain.all().filter(|e| e.key.part == part && *e.key.ident == self.ident(e.key.rel)) {
            let width = entry.types.len() / 2;
            let mut rows = Vec::with_capacity(entry.rows.len());
            for i in 0..entry.rows.len() {
                let words = entry.row(i);
                let mut row = Vec::with_capacity(width);
                for pair in words.chunks_exact(2) {
                    row.push(match pair[0] {
                        0 => Value::Int(pair[1]),
                        1 => Value::Str(store.atom_value(u32::try_from(pair[1]).ok()?).ok()??.into()),
                        tag @ 2..=4 => Value::Branded(Branded { class: [Class::Name, Class::Kind, Class::Field][usize::try_from(tag - 2).ok()?], atom: u32::try_from(pair[1]).ok()? }),
                        _ => return None,
                    });
                }
                rows.push(row.into());
            }
            facts.insert(entry.key.rel.to_owned(), rows);
        }
        (!facts.is_empty()).then_some(facts)
    }

    /// Whether the store keeps partition `part`'s entry.
    pub fn has(&self, store: &ContentStore, part: i64) -> bool {
        let Ok(Some(kept)) = store.kept_space(&self.space()) else { return false };
        kept.view().all().any(|e| e.key.part == part && *e.key.ident == self.ident(e.key.rel))
    }

    /// Keeps `facts` as partition `part`'s entry, held by the source of SHA-256 key `holder` and by `plan` (it goes
    /// when either does), behind the run (the store's writer): its relations' rows, and the relations it kept before and has no longer, gone. Nothing when a
    /// relation's rows differ in width or the store refuses its strings (the next run makes it again).
    pub fn put(&self, store: &ContentStore, part: i64, holder: [u8; 32], plan: Option<[u8; 32]>, facts: &Facts) {
        let strings: Vec<&str> = facts.values().flatten().flat_map(|r| r.iter()).filter_map(|v| if let Value::Str(s) = v { Some(s.as_ref()) } else { None }).collect();
        let Ok(atoms) = store.intern_atoms(&strings) else { return };
        let mut atoms = atoms.into_iter();
        let mut puts = Vec::new();
        for (rel, rows) in facts {
            let Some(width) = rows.first().map(|r| r.len()) else { continue };
            if rows.iter().any(|r| r.len() != width) {
                return;
            }
            let mut flat = Vec::with_capacity(rows.len() * width * 2);
            for row in rows {
                for v in row.iter() {
                    match v {
                        Value::Int(i) => flat.extend([0, *i]),
                        Value::Str(_) => flat.extend([1, i64::from(atoms.next().expect("an atom a string"))]),
                        Value::Branded(b) => flat.extend([2 + b.class as i64, i64::from(b.atom)]),
                    }
                }
            }
            let mut h = Sha256::new();
            for v in &flat {
                h.update(v.to_le_bytes());
            }
            let digest = u64::from_le_bytes(h.finalize()[..8].try_into().expect("8 bytes"));
            let types = b"ii".repeat(width);
            puts.push(KeptPut { rel: rel.clone(), ident: self.ident(rel), part, digest, reads: Vec::new(), types: types.clone(), what: KeptWhat::Flat { widths: vec![8; types.len()], rows: flat, n: rows.len(), levels: None, given: true } });
        }
        // What the partition kept before that this entry has no rows of.
        let written: BTreeSet<&str> = puts.iter().map(|p| p.rel.as_str()).collect();
        if let Ok(Some(kept)) = store.kept_space(&self.space()) {
            let chain = kept.view();
            let before: Vec<(String, Vec<u8>)> = chain.all().filter(|e| e.key.part == part && *e.key.ident == self.ident(e.key.rel) && !written.contains(e.key.rel)).map(|e| (e.key.rel.to_owned(), e.types.to_vec())).collect();
            for (rel, types) in before {
                puts.push(KeptPut { ident: self.ident(&rel), rel, part, digest: 0, reads: Vec::new(), types, what: KeptWhat::Gone });
            }
        }
        puts.push(holder_put(part, holder, plan));
        store.queue_kept(self.space(), puts);
    }
}
