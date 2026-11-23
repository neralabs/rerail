//! A memo entry of program facts kept as kept facts of the store (t33 step 3, decisions 0104 and 0456), where the
//! facts memo kept it as one encoded row (`facts_results`): each relation of the entry a kept relation of its user's
//! space (or of one space per scope, `within`), the entry's rows one partition (its key's), held by the source the
//! entry was kept under and the plan it was made under (`HOLDER`): the store's collection drops the partition once
//! either goes, as the memo's row died with them.
//!
//! A column holds one kind of value in every row: a number (`i`), a string (`s`, the atom the store interned it as) or
//! a branded atom (`s`, its class a name, a kind or a field). Atoms are typed `s`, so a forked process's ids are
//! mapped to the store's as the writer writes them (decision 0170). A relation's rows of one shape are a kept
//! relation named by the entry's relation and their columns' kinds (`rel#` and one letter a column: `i`, `s`, `n`,
//! `k`, `f`), each row led by its place among the relation's rows, so a relation of rows of several shapes reads
//! back in its order.

use super::super::front::Value;
use super::super::front::ast::{Branded, Class};
use super::super::program_facts::Facts;
use crate::content_store::ContentStore;
use crate::content_store::kept_facts::{BlockFlat, KeptPut, KeptWhat, holder_put};
use rerail_blocks::kept::KeptRead as _;
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// One user's entries: the name its space and its relations' identities are made of, and the scope its space is of.
#[derive(Clone, Copy, Debug)]
pub struct Entries {
    pub name: &'static str,
    pub scope: Option<[u8; 32]>,
}

/// A column's kind, as its letter in the kept relation's name.
fn kind(v: &Value) -> Option<u8> {
    Some(match v {
        Value::Int(_) => b'i',
        Value::Str(_) => b's',
        Value::Branded(b) => match b.class {
            Class::Name => b'n',
            Class::Kind => b'k',
            Class::Field => b'f',
            #[allow(unreachable_patterns)]
            _ => return None,
        },
    })
}

impl Entries {
    pub const fn new(name: &'static str) -> Self {
        Entries { name, scope: None }
    }

    /// The entries of `scope` (a prepared program's, say): a space of their own.
    pub const fn within(self, scope: [u8; 32]) -> Self {
        Entries { name: self.name, scope: Some(scope) }
    }

    pub fn space(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(format!("{}-space/v2", self.name).as_bytes());
        if let Some(scope) = &self.scope {
            h.update(scope);
        }
        h.finalize().into()
    }

    fn ident(&self, rel: &str) -> [u8; 32] {
        Sha256::digest(format!("{}/v2\0{rel}", self.name).as_bytes()).into()
    }

    /// The partition of the entry keyed `key`.
    pub fn part(key: &[u8; 32]) -> i64 {
        i64::from_le_bytes(key[..8].try_into().expect("8 bytes")) & i64::MAX
    }

    /// The entry of partition `part`, as kept; `None` when the store keeps none (or one that no longer reads).
    pub fn get(&self, store: &ContentStore, part: i64) -> Option<Facts> {
        self.read(store, Some(part), None).remove(&part)
    }

    /// The entry of partition `part` as `get` reads it, but relation `rel`'s rows of numbers alone (every column `i`)
    /// handed to `each` as the block holds them, without their place, in no order, and left out of the facts: a
    /// reader that decodes its own rows reads them in place, not through values (the kept plans, t33 K4).
    pub fn get_numbers(&self, store: &ContentStore, part: i64, rel: &str, each: &mut dyn FnMut(&[i64])) -> Option<Facts> {
        self.read(store, Some(part), Some((rel, each))).remove(&part)
    }

    /// Every entry of the space, by partition.
    pub fn all(&self, store: &ContentStore) -> BTreeMap<i64, Facts> {
        self.read(store, None, None)
    }

    fn read(&self, store: &ContentStore, only: Option<i64>, mut numbers: Option<(&str, &mut dyn FnMut(&[i64]))>) -> BTreeMap<i64, Facts> {
        let mut out: BTreeMap<i64, Facts> = BTreeMap::new();
        let mut placed: BTreeMap<i64, BTreeMap<String, Vec<(i64, Vec<Value>)>>> = BTreeMap::new();
        let mut bad = BTreeSet::new();
        let Ok(Some(kept)) = store.kept_space(&self.space()) else { return out };
        let chain = kept.view();
        // One partition's read merges its own entries alone: the space holds one a program, every engine build's.
        let entries: Box<dyn Iterator<Item = rerail_blocks::kept::Kept<'_>> + '_> = match only {
            Some(p) => Box::new(chain.of_part(p).into_iter()),
            None => chain.all(),
        };
        // Each atom's text read once for the entry, shared by the rows that hold it (the relations' and rules' names
        // repeat across a program's rows): read per cell, they were most of a warm run's kept plans' read.
        let mut texts: std::collections::HashMap<u32, Option<super::super::front::Name>> = std::collections::HashMap::new();
        for entry in entries.filter(|e| *e.key.ident == self.ident(e.key.rel)) {
            let part = entry.key.part;
            let Some((rel, kinds)) = entry.key.rel.rsplit_once('#') else { continue };
            let kinds = kinds.as_bytes();
            if kinds.len() + 1 != entry.types.len() {
                bad.insert(part);
                continue;
            }
            if let Some((want, each)) = numbers.as_mut()
                && *want == rel
                && kinds.iter().all(|&k| k == b'i')
            {
                entry.each_row(|words| each(&words[1..]));
                continue;
            }
            let rows = placed.entry(part).or_default().entry(rel.to_owned()).or_default();
            let mut broke = false;
            entry.each_row(|words| {
                if broke {
                    return;
                }
                let at = words[0];
                let row: Option<Vec<Value>> = words[1..]
                    .iter()
                    .zip(kinds)
                    .map(|(&w, &k)| {
                        Some(match k {
                            b'i' => Value::Int(w),
                            b's' => {
                                let atom = u32::try_from(w).ok()?;
                                Value::Str(texts.entry(atom).or_insert_with(|| store.atom_value(atom).ok().flatten().map(Into::into)).clone()?)
                            }
                            b'n' | b'k' | b'f' => {
                                let class = match k {
                                    b'n' => Class::Name,
                                    b'k' => Class::Kind,
                                    _ => Class::Field,
                                };
                                Value::Branded(Branded { class, atom: u32::try_from(w).ok()? })
                            }
                            _ => return None,
                        })
                    })
                    .collect();
                match row {
                    Some(row) => rows.push((at, row)),
                    None => broke = true,
                }
            });
            if broke {
                bad.insert(part);
            }
        }
        for (part, rels) in placed {
            let facts = out.entry(part).or_default();
            for (rel, mut rows) in rels {
                rows.sort_by_key(|(at, _)| *at);
                facts.insert(rel, rows.into_iter().map(|(_, row)| row.into()).collect());
            }
        }
        // An entry with a value that no longer reads is not one.
        for part in bad {
            out.remove(&part);
        }
        out
    }

    /// Keeps `facts` as partition `part`'s entry, held by the source of SHA-256 key `holder` and by `plan` (it goes
    /// when either does), behind the run (the store's writer): its relations' rows, in place of what it kept before.
    /// Nothing when the store refuses its strings (the next run makes it again).
    pub fn put(&self, store: &ContentStore, part: i64, holder: [u8; 32], plan: Option<[u8; 32]>, facts: &Facts) {
        let strings: Vec<&str> = facts.values().flatten().flat_map(|r| r.iter()).filter_map(|v| if let Value::Str(s) = v { Some(s.as_ref()) } else { None }).collect();
        let Ok(atoms) = store.intern_atoms(&strings) else { return };
        let mut atoms = atoms.into_iter();
        let mut puts = Vec::new();
        for (rel, rows) in facts {
            // The relation's rows by shape: each shape a kept relation, each row led by its place. Counted first, so
            // each shape's rows are written into a block of their size as they are found (`BlockFlat`).
            let mut shapes: BTreeMap<Vec<u8>, usize> = BTreeMap::new();
            let mut row_kinds = Vec::with_capacity(rows.len());
            for row in rows {
                let Some(kinds) = row.iter().map(kind).collect::<Option<Vec<u8>>>() else { return };
                *shapes.entry(kinds.clone()).or_default() += 1;
                row_kinds.push(kinds);
            }
            let mut blocks: BTreeMap<Vec<u8>, (BlockFlat, usize, Sha256)> =
                shapes.into_iter().map(|(kinds, n)| { let w = 1 + kinds.len(); (kinds, (BlockFlat::new(n, w, true), 0, Sha256::new())) }).collect();
            for (at, (row, kinds)) in rows.iter().zip(&row_kinds).enumerate() {
                let (block, r, h) = blocks.get_mut(kinds).expect("a shape counted");
                let values = std::iter::once(at as i64).chain(row.iter().map(|v| match v {
                    Value::Int(i) => *i,
                    Value::Str(_) => i64::from(atoms.next().expect("an atom a string")),
                    Value::Branded(b) => i64::from(b.atom),
                }));
                for (c, v) in values.enumerate() {
                    block.put(*r, c, v);
                    h.update(v.to_le_bytes());
                }
                *r += 1;
            }
            for (kinds, (block, _, h)) in blocks {
                let digest = u64::from_le_bytes(h.finalize()[..8].try_into().expect("8 bytes"));
                let types: Vec<u8> = std::iter::once(b'i').chain(kinds.iter().map(|&k| if k == b'i' { b'i' } else { b's' })).collect();
                let name = format!("{rel}#{}", String::from_utf8(kinds).expect("ascii"));
                puts.push(KeptPut { ident: self.ident(&name), rel: name, part, digest, reads: Vec::new(), types, what: KeptWhat::Picked(Box::new(block)) });
            }
        }
        // Put whole with its holder: what the partition kept before that this entry has no rows of goes with it, by
        // the writer (`HOLDER`), so a run writes an entry inside its waves with no read of the store.
        puts.push(holder_put(part, holder, plan));
        store.queue_kept(self.space(), puts);
    }
}
