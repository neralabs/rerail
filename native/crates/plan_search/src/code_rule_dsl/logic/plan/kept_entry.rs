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
        self.read(store, Some(&[part]), None, None).remove(&part)
    }

    /// The entry of partition `part` as `get` reads it, of the relations `rels` keeps alone: the others' rows are not
    /// read. `None` where it holds no row of them.
    pub fn get_only(&self, store: &ContentStore, part: i64, rels: &dyn Fn(&str) -> bool) -> Option<Facts> {
        self.read(store, Some(&[part]), None, Some(rels)).remove(&part)
    }

    /// The entries of partitions `parts`, by partition, in one read: those the store keeps (`get`'s).
    pub fn get_many(&self, store: &ContentStore, parts: &[i64]) -> BTreeMap<i64, Facts> {
        self.read(store, Some(parts), None, None)
    }

    /// The entry of partition `part` as `get` reads it, but relation `rel`'s rows of numbers alone (every column `i`)
    /// handed to `each` as the block holds them, without their place, in no order, and left out of the facts: a
    /// reader that decodes its own rows reads them in place, not through values (the kept plans, t33 K4).
    pub fn get_numbers(&self, store: &ContentStore, part: i64, rel: &str, each: &mut dyn FnMut(&[i64])) -> Option<Facts> {
        self.read(store, Some(&[part]), Some((rel, each)), None).remove(&part)
    }

    /// `get_numbers`, of the relations `rels` keeps alone (`rel` among them).
    pub fn get_numbers_only(&self, store: &ContentStore, part: i64, rel: &str, each: &mut dyn FnMut(&[i64]), rels: &dyn Fn(&str) -> bool) -> Option<Facts> {
        self.read(store, Some(&[part]), Some((rel, each)), Some(rels)).remove(&part)
    }

    /// Every entry of the space, by partition.
    pub fn all(&self, store: &ContentStore) -> BTreeMap<i64, Facts> {
        self.read(store, None, None, None)
    }

    /// Each row of the entries of `only` (every partition where `None`) of the relations `rels` keeps, as the block
    /// holds it (`Row`), handed to `each` with its partition and relation: in the blocks' order, not the rows' places
    /// (`Row::at` gives a row's), and nothing decoded but the cells a reader asks for, each atom's text read once for
    /// the call. A reader reads its rows here, in place (decision 0353), not through a copy of them as values. `each`
    /// returns whether the row reads: a partition with a row that does not, or of a relation whose name does not say
    /// its cells' kinds, is in the second set, beside the partitions found.
    pub fn visit(&self, store: &ContentStore, only: Option<&[i64]>, rels: &dyn Fn(&str) -> bool, each: &mut dyn FnMut(i64, &str, &Row<'_>) -> bool) -> (BTreeSet<i64>, BTreeSet<i64>) {
        let (mut found, mut bad) = (BTreeSet::new(), BTreeSet::new());
        let Ok(Some(kept)) = store.kept_space(&self.space()) else { return (found, bad) };
        let chain = kept.view();
        // One partition's read merges its own entries alone: the space holds one a program, every engine build's.
        let entries: Box<dyn Iterator<Item = rerail_blocks::kept::Kept<'_>> + '_> = match only {
            Some(parts) => Box::new(parts.iter().flat_map(|&p| chain.of_part(p))),
            None => chain.all(),
        };
        // Each atom's text read once for the call, shared by the rows that hold it (the relations' and rules' names
        // repeat across a program's rows): read per cell, they were most of a warm run's kept plans' read.
        let texts = Texts { store, seen: std::cell::RefCell::new(std::collections::HashMap::new()) };
        for entry in entries {
            let part = entry.key.part;
            let Some((rel, kinds)) = entry.key.rel.rsplit_once('#') else { continue };
            // The reader's relations first: an entry's identity is a hash of its name, made only for those.
            if !rels(rel) || *entry.key.ident != self.ident(entry.key.rel) {
                continue;
            }
            found.insert(part);
            let kinds = kinds.as_bytes();
            if kinds.len() + 1 != entry.types.len() {
                bad.insert(part);
                continue;
            }
            let mut broke = false;
            entry.each_row(|words| {
                if !broke && !each(part, rel, &Row { at: words[0], cells: &words[1..], kinds, texts: &texts }) {
                    broke = true;
                }
            });
            if broke {
                bad.insert(part);
            }
        }
        (found, bad)
    }

    fn read(&self, store: &ContentStore, only: Option<&[i64]>, mut numbers: Option<(&str, &mut dyn FnMut(&[i64]))>, rels: Option<&dyn Fn(&str) -> bool>) -> BTreeMap<i64, Facts> {
        type Placed = BTreeMap<i64, BTreeMap<String, Vec<(i64, Vec<Value>)>>>;
        let mut placed: Placed = BTreeMap::new();
        // The rows of the entry being read: an entry's rows come together, so they are placed once it ends.
        let mut current: Option<(i64, String, Vec<(i64, Vec<Value>)>)> = None;
        let place = |placed: &mut Placed, (part, rel, rows): (i64, String, Vec<(i64, Vec<Value>)>)| {
            placed.entry(part).or_default().entry(rel).or_default().extend(rows);
        };
        let keep = |rel: &str| rels.is_none_or(|keep| keep(rel));
        let (found, bad) = self.visit(store, only, &keep, &mut |part, rel, row| {
            if let Some((want, each)) = numbers.as_mut()
                && *want == rel
                && row.kinds.iter().all(|&k| k == b'i')
            {
                each(row.cells);
                return true;
            }
            let Some(values) = (0..row.len()).map(|c| row.value(c)).collect::<Option<Vec<Value>>>() else { return false };
            if current.as_ref().is_none_or(|(p, r, _)| *p != part || r != rel) {
                if let Some(done) = current.take() {
                    place(&mut placed, done);
                }
                current = Some((part, rel.to_owned(), Vec::new()));
            }
            current.as_mut().expect("set above").2.push((row.at, values));
            true
        });
        if let Some(done) = current.take() {
            place(&mut placed, done);
        }
        // A partition read, though its rows went to `numbers`: an entry of these rows alone is one. An entry with a
        // value that no longer reads is not one.
        let mut out: BTreeMap<i64, Facts> = BTreeMap::new();
        for part in found.difference(&bad) {
            let facts = out.entry(*part).or_default();
            for (rel, mut rows) in placed.remove(part).unwrap_or_default() {
                rows.sort_by_key(|(at, _)| *at);
                facts.insert(rel, rows.into_iter().map(|(_, row)| row.into()).collect());
            }
        }
        out
    }

    /// Keeps `facts` as partition `part`'s entry, held by the source of SHA-256 key `holder` and by `plan` (it goes
    /// when either does), behind the run (the store's writer): its relations' rows, in place of what it kept before.
    /// Nothing when the store refuses its strings (the next run makes it again).
    pub fn put(&self, store: &ContentStore, part: i64, holder: [u8; 32], plan: Option<[u8; 32]>, facts: &Facts) {
        self.put_with(store, part, holder, plan, facts, &[]);
    }

    /// `put`, and relations of one integer a row (`ints`: each its name and its values in order), written into their
    /// blocks from where they lie: the same relations, rows and digests as rows of `Value::Int` in `facts` make, but
    /// no row made of each value first (a frame's image was a row a word: 2.7 M on cold PostHog).
    pub fn put_with(&self, store: &ContentStore, part: i64, holder: [u8; 32], plan: Option<[u8; 32]>, facts: &Facts, ints: &[(&str, &[i64])]) {
        let strings: Vec<&str> = facts.values().flatten().flat_map(|r| r.iter()).filter_map(|v| if let Value::Str(s) = v { Some(s.as_ref()) } else { None }).collect();
        let Ok(atoms) = store.intern_atoms(&strings) else { return };
        let mut atoms = atoms.into_iter();
        let mut puts = Vec::new();
        for (rel, rows) in facts {
            // The relation's rows by shape: each shape a kept relation, each row led by its place. Counted first, so
            // each shape's rows are written into a block of their size as they are found (`BlockFlat`).
            // Each shape's kinds made once (a row's are read into one list, and found by it): a list a row and its
            // clone into the map were 2 x 2.37 M arrays on cold PostHog. Each row is its shape's number.
            let mut shapes: BTreeMap<Vec<u8>, u32> = BTreeMap::new();
            let mut counts: Vec<usize> = Vec::new();
            let mut row_shape: Vec<u32> = Vec::with_capacity(rows.len());
            let mut kinds: Vec<u8> = Vec::new();
            for row in rows {
                kinds.clear();
                for v in row.iter() {
                    let Some(k) = kind(v) else { return };
                    kinds.push(k);
                }
                let id = match shapes.get(kinds.as_slice()) {
                    Some(&id) => id,
                    None => {
                        let id = counts.len() as u32;
                        shapes.insert(kinds.clone(), id);
                        counts.push(0);
                        id
                    }
                };
                counts[id as usize] += 1;
                row_shape.push(id);
            }
            let mut widths = vec![0usize; counts.len()];
            for (k, &id) in &shapes {
                widths[id as usize] = 1 + k.len();
            }
            let mut blocks: Vec<Option<(BlockFlat, usize, Sha256)>> = counts.iter().zip(&widths).map(|(&n, &w)| Some((BlockFlat::new(n, w, true), 0, Sha256::new()))).collect();
            for (at, (row, &id)) in rows.iter().zip(&row_shape).enumerate() {
                let (block, r, h) = blocks[id as usize].as_mut().expect("a shape counted");
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
            // In the shapes' order, as before.
            for (kinds, id) in shapes {
                let (block, _, h) = blocks[id as usize].take().expect("each shape once");
                let digest = u64::from_le_bytes(h.finalize()[..8].try_into().expect("8 bytes"));
                let types: Vec<u8> = std::iter::once(b'i').chain(kinds.iter().map(|&k| if k == b'i' { b'i' } else { b's' })).collect();
                let name = format!("{rel}#{}", String::from_utf8(kinds).expect("ascii"));
                puts.push(KeptPut { ident: self.ident(&name), rel: name, part, digest, reads: Vec::new(), types, what: KeptWhat::Picked(Box::new(block)) });
            }
        }
        for &(rel, values) in ints.iter().filter(|(_, v)| !v.is_empty()) {
            // As `facts`' rows of one `Value::Int` each: shape `i`, each row its place and its value.
            let (mut block, mut h) = (BlockFlat::new(values.len(), 2, true), Sha256::new());
            for (at, &v) in values.iter().enumerate() {
                for (c, x) in [at as i64, v].into_iter().enumerate() {
                    block.put(at, c, x);
                    h.update(x.to_le_bytes());
                }
            }
            let digest = u64::from_le_bytes(h.finalize()[..8].try_into().expect("8 bytes"));
            let name = format!("{rel}#i");
            puts.push(KeptPut { ident: self.ident(&name), rel: name, part, digest, reads: Vec::new(), types: b"ii".to_vec(), what: KeptWhat::Picked(Box::new(block)) });
        }
        // Put whole with its holder: what the partition kept before that this entry has no rows of goes with it, by
        // the writer (`HOLDER`), so a run writes an entry inside its waves with no read of the store.
        puts.push(holder_put(part, holder, plan));
        store.queue_kept(self.space(), puts);
    }
}

/// The atoms' texts a visit read (`Entries::visit`), each once.
struct Texts<'s> {
    store: &'s ContentStore,
    seen: std::cell::RefCell<std::collections::HashMap<u32, Option<super::super::front::Name>>>,
}

/// A row of an entry as its block holds it (`Entries::visit`): its place among its relation's rows, and its cells,
/// each of the kind its relation's name says, decoded only when asked.
pub struct Row<'r> {
    /// The row's place among its relation's rows.
    pub at: i64,
    cells: &'r [i64],
    kinds: &'r [u8],
    texts: &'r Texts<'r>,
}

impl Row<'_> {
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Cell `c` as a number, where it is one.
    pub fn int(&self, c: usize) -> Option<i64> {
        (*self.kinds.get(c)? == b'i').then(|| self.cells[c])
    }

    /// Cell `c` as a string, where it is one: the atom's text, read once for the visit.
    pub fn text(&self, c: usize) -> Option<super::super::front::Name> {
        if *self.kinds.get(c)? != b's' {
            return None;
        }
        let atom = u32::try_from(self.cells[c]).ok()?;
        self.texts.seen.borrow_mut().entry(atom).or_insert_with(|| self.texts.store.atom_value(atom).ok().flatten().map(Into::into)).clone()
    }

    /// Cell `c` as the value it was kept from.
    pub fn value(&self, c: usize) -> Option<Value> {
        let w = *self.cells.get(c)?;
        Some(match self.kinds[c] {
            b'i' => Value::Int(w),
            b's' => Value::Str(self.text(c)?),
            k @ (b'n' | b'k' | b'f') => {
                let class = match k {
                    b'n' => Class::Name,
                    b'k' => Class::Kind,
                    _ => Class::Field,
                };
                Value::Branded(Branded { class, atom: u32::try_from(w).ok()? })
            }
            _ => return None,
        })
    }
}
