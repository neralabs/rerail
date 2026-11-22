//! The observer's answers, canonical forms and settings as kept facts (t33 K2, decisions 0104 and 0456): three
//! relations of one space in the content store whose observer it is, where its own SQLite file kept them as the
//! `observed` and `observer_meta` tables and the `canonical:` records of `run_record`.
//!
//! - `observed(path, kind, dev, ino, size, mtime_ns, ctime_ns, digest, listed, listing, recorded_ns)` and
//!   `canonical(path, canonical, recorded_ns)`, one partition per directory: the answers and forms for the paths in
//!   it. A root's part of the tree is read as the partitions at and under it, and the root's own row from its
//!   parent's, as the tables were read by a range of paths.
//! - `observer_meta(key, is_text, number, text)`, one partition: each root's journal cursor, the observer's own
//!   journal's generation and floor, its own output directories.
//!
//! A change is applied as one put of the space (`apply`): the partitions it touches, read as they are, edited and
//! written whole, with the settings when they change. So the answers forgotten under a root and the cursor that
//! vouches for the rest are written together, as the table's transaction wrote them. A put waits for this process's
//! earlier puts of the space (`ContentStore::kept_row` settles them first), so each reads the last.
//!
//! The space is the observer's path's (`<store>.files`, or a test thread's own): two observers of one store keep
//! apart, as their files did.

use super::{Meta, Stat};
use crate::content_store::ContentStore;
use crate::content_store::kept_facts::{KeptPut, KeptWhat};
use rerail_blocks::kept::{FactKey, KeptRead};
use sha2::{Digest as _, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use parking_lot::Mutex;
use std::path::{Path, PathBuf};

const META: &str = "observer_meta";
/// `observer_meta`'s column types: `s` an atom of the store.
const META_TYPES: &[u8] = b"siis";

/// An answer as the table held it: path, kind, stat, digest, listing (encoded), when recorded.
pub(super) type Row = (String, i64, Stat, Vec<u8>, Option<Vec<u8>>, i64);
/// A canonical form: the path, its canonical path, when recorded.
pub(super) type Canon = (String, String, i64);

/// A relation of the space partitioned by directory: a row's first column is its path, an atom.
trait ByDir: Sized {
    const REL: &'static str;
    /// Its column types: `s` an atom of the store.
    const TYPES: &'static [u8];
    /// How many of its columns are texts (`texts`).
    const TEXTS: usize;
    fn path(&self) -> &str;
    /// Its texts, interned as atoms for `words`.
    fn texts(&self) -> [String; 3];
    fn words(&self, syms: &[u32]) -> Vec<i64>;
    fn decode(facts: &Open<'_>, r: &[i64]) -> Option<Self>;
}

impl ByDir for Row {
    const REL: &'static str = "observed";
    const TYPES: &'static [u8] = b"siiiiiisisi";
    const TEXTS: usize = 3;
    fn path(&self) -> &str {
        &self.0
    }
    fn texts(&self) -> [String; 3] {
        [self.0.clone(), hex_of(&self.3), self.4.as_ref().map(|l| String::from_utf8_lossy(l).into_owned()).unwrap_or_default()]
    }
    fn words(&self, s: &[u32]) -> Vec<i64> {
        vec![i64::from(s[0]), self.1, self.2.dev, self.2.ino, self.2.size, self.2.mtime_ns, self.2.ctime_ns, i64::from(s[1]), i64::from(self.4.is_some()), i64::from(s[2]), self.5]
    }
    fn decode(facts: &Open<'_>, r: &[i64]) -> Option<Row> {
        let path = facts.text(r[0])?;
        let stat = Stat { dev: r[2], ino: r[3], size: r[4], mtime_ns: r[5], ctime_ns: r[6] };
        let digest = super::unhex(&facts.text(r[7]).unwrap_or_default()).map_or_else(Vec::new, |d| d.to_vec());
        let listing = (r[8] == 1).then(|| facts.text(r[9]).unwrap_or_default().into_bytes());
        Some((path, r[1], stat, digest, listing, r[10]))
    }
}

impl ByDir for Canon {
    const REL: &'static str = "canonical";
    const TYPES: &'static [u8] = b"ssi";
    const TEXTS: usize = 2;
    fn path(&self) -> &str {
        &self.0
    }
    fn texts(&self) -> [String; 3] {
        [self.0.clone(), self.1.clone(), String::new()]
    }
    fn words(&self, s: &[u32]) -> Vec<i64> {
        vec![i64::from(s[0]), i64::from(s[1]), self.2]
    }
    fn decode(facts: &Open<'_>, r: &[i64]) -> Option<Canon> {
        Some((facts.text(r[0])?, facts.text(r[1])?, r[2]))
    }
}

/// One relation's part of a change: rows put, paths whose rows go, paths whose rows go with every row under them.
pub(super) struct Edit<R> {
    pub put: Vec<R>,
    pub delete: Vec<String>,
    pub delete_under: Vec<String>,
}

impl<R> Default for Edit<R> {
    fn default() -> Self {
        Edit { put: Vec::new(), delete: Vec::new(), delete_under: Vec::new() }
    }
}

impl<R> Edit<R> {
    fn is_empty(&self) -> bool {
        self.put.is_empty() && self.delete.is_empty() && self.delete_under.is_empty()
    }
}

/// One change of the observer's kept facts, written as one put.
#[derive(Default)]
pub(super) struct Change {
    pub answers: Edit<Row>,
    pub canon: Edit<Canon>,
    pub meta_put: Vec<(String, rusqlite::types::Value)>,
    pub meta_delete: Vec<String>,
}

impl Change {
    fn is_empty(&self) -> bool {
        self.answers.is_empty() && self.canon.is_empty() && self.meta_put.is_empty() && self.meta_delete.is_empty()
    }
}

/// The observer's kept facts in the content store at `path`. The store is opened as it is first asked and closed at
/// the outermost run's end (`close`): a run leaves no handle of its store open.
pub(super) struct Facts {
    path: PathBuf,
    space: [u8; 32],
    store: Mutex<Option<ContentStore>>,
}

/// The facts read and written through an open store.
struct Open<'s> {
    store: &'s ContentStore,
    space: [u8; 32],
}

fn ident(rel: &str) -> [u8; 32] {
    Sha256::digest(format!("rerail.observer/v1\0{rel}").as_bytes()).into()
}

/// A directory's partition.
fn part(dir: &str) -> i64 {
    let d: [u8; 32] = Sha256::digest(dir.as_bytes()).into();
    i64::from_le_bytes(d[..8].try_into().expect("8 bytes")) & i64::MAX
}

/// A path's separators: `/`, and on Windows `\` too.
fn separator(b: u8) -> bool {
    b == b'/' || (cfg!(windows) && b == b'\\')
}

/// A path's directory, as the partitions are named: its parent (the root itself at the top), or `""` for none.
fn dir_of(path: &str) -> &str {
    match path.bytes().rposition(separator) {
        // At the top of a volume (`/x`, `C:\x`): its root, with the separator.
        Some(i) if !path[..i].bytes().any(separator) => &path[..=i],
        Some(i) => &path[..i],
        None => "",
    }
}

/// Whether `path` is `at` or under it.
fn at_or_under(path: &str, at: &str) -> bool {
    path == at || (path.starts_with(at) && (at.bytes().last().is_some_and(separator) || path.as_bytes().get(at.len()).is_some_and(|&b| separator(b))))
}

impl Facts {
    /// The facts of the observer whose file is `observer` (`<store>.files…`), in the content store beside it; `None`
    /// when the store cannot be opened.
    pub(super) fn open(observer: &Path) -> Option<Facts> {
        let text = observer.to_string_lossy();
        let at = text.rfind(".files")?;
        let path = PathBuf::from(&text[..at]);
        let store = ContentStore::open_images(&path).ok()?;
        let space = Sha256::digest(format!("rerail.observer-space/v1\0{text}").as_bytes()).into();
        Some(Facts { path, space, store: Mutex::new(Some(store)) })
    }

    /// `f` of the facts through the store, opened again if closed; `None` when it cannot be.
    fn with<T>(&self, f: impl FnOnce(&Open<'_>) -> T) -> Option<T> {
        let mut held = self.store.lock();
        if held.is_none() {
            *held = Some(ContentStore::open_images(&self.path).ok()?);
        }
        Some(f(&Open { store: held.as_ref()?, space: self.space }))
    }

    /// Every setting kept.
    pub(super) fn meta(&self) -> Meta {
        self.with(|o| o.meta()).unwrap_or_default()
    }

    /// The answers and canonical forms kept at and under `unit`.
    pub(super) fn under(&self, unit: &str) -> (Vec<Row>, Vec<Canon>) {
        self.with(|o| o.under(unit)).unwrap_or_default()
    }

    /// Every answer and canonical form kept (`collect`).
    pub(super) fn all(&self) -> (Vec<Row>, Vec<Canon>) {
        self.with(|o| o.all()).unwrap_or_default()
    }

    /// Writes `change` as one put, a job called `what` (`Open::apply`).
    pub(super) fn apply(&self, change: Change, what: &'static str) {
        self.with(|o| o.apply(change, what));
    }

    /// Waits until every put queued is written, reading nothing (so a thread may ask it as it ends).
    pub(super) fn written(&self) {
        if let Some(store) = self.store.lock().as_ref() {
            store.settle_kept(&self.space);
        }
    }

    /// Waits until every put queued is written, and closes the store (the outermost run's end).
    pub(super) fn close(&self) {
        if let Some(store) = self.store.lock().take() {
            store.settle_kept(&self.space);
        }
    }
}

impl Open<'_> {
    fn text(&self, sym: i64) -> Option<String> {
        self.store.atom_value(u32::try_from(sym).ok()?).ok().flatten()
    }

    fn meta(&self) -> Meta {
        let mut meta = Meta::new();
        let Ok(Some(space)) = self.store.kept_space(&self.space) else { return meta };
        let chain = space.view();
        let Some(kept) = chain.get(&FactKey { rel: META, ident: &ident(META), part: 0 }) else { return meta };
        for i in 0..kept.rows.len() {
            let row = kept.row(i);
            let Some(key) = self.text(row[0]) else { continue };
            let value = if row[1] == 1 { rusqlite::types::Value::Text(self.text(row[3]).unwrap_or_default()) } else { rusqlite::types::Value::Integer(row[2]) };
            meta.insert(key, value);
        }
        meta
    }

    fn under(&self, unit: &str) -> (Vec<Row>, Vec<Canon>) {
        let Ok(Some(space)) = self.store.kept_space(&self.space) else { return Default::default() };
        let chain = space.view();
        (self.rows_under(&chain, unit), self.rows_under(&chain, unit))
    }

    fn rows_under<R: ByDir>(&self, chain: &dyn KeptRead, unit: &str) -> Vec<R> {
        let mut out = Vec::new();
        for kept in chain.of(R::REL) {
            if kept.rows.len() == 0 {
                continue;
            }
            // A partition's rows share their directory: its first row names it.
            let Some(first) = self.text(kept.row(0)[0]) else { continue };
            let dir = dir_of(&first);
            let whole = at_or_under(dir, unit);
            // The unit's own row is in its parent's partition.
            if !whole && dir != dir_of(unit) {
                continue;
            }
            for i in 0..kept.rows.len() {
                if let Some(row) = R::decode(self, &kept.row(i))
                    && (whole || row.path() == unit)
                {
                    out.push(row);
                }
            }
        }
        out
    }

    fn all(&self) -> (Vec<Row>, Vec<Canon>) {
        let Ok(Some(space)) = self.store.kept_space(&self.space) else { return Default::default() };
        let chain = space.view();
        (self.all_of(&chain), self.all_of(&chain))
    }

    fn all_of<R: ByDir>(&self, chain: &dyn KeptRead) -> Vec<R> {
        chain.of(R::REL).flat_map(|kept| (0..kept.rows.len()).filter_map(|i| R::decode(self, &kept.row(i))).collect::<Vec<_>>()).collect()
    }

    /// Writes `change`: the partitions it touches, and the settings when they change, as one put, a job called `what`
    /// (`ContentStore::queue_kept_as`).
    fn apply(&self, change: Change, what: &'static str) {
        if change.is_empty() {
            return;
        }
        let space = self.store.kept_space(&self.space).ok().flatten();
        let chain = space.as_ref().map(|s| s.view());
        let chain = chain.as_ref().map(|c| c as &dyn KeptRead);
        let mut puts = Vec::new();
        if !self.edit(chain, change.answers, &mut puts) || !self.edit(chain, change.canon, &mut puts) {
            return;
        }
        if !change.meta_put.is_empty() || !change.meta_delete.is_empty() {
            let mut meta: HashMap<String, rusqlite::types::Value> = self.meta();
            for k in &change.meta_delete {
                meta.remove(k);
            }
            meta.extend(change.meta_put);
            let ordered: BTreeMap<String, rusqlite::types::Value> = meta.into_iter().collect();
            let texts: Vec<String> = ordered.iter().flat_map(|(k, v)| [k.clone(), match v { rusqlite::types::Value::Text(t) => t.clone(), _ => String::new() }]).collect();
            let Ok(syms) = self.store.intern_atoms(&texts.iter().map(String::as_str).collect::<Vec<_>>()) else { return };
            let mut flat = Vec::new();
            for ((_, v), s) in ordered.iter().zip(syms.chunks_exact(2)) {
                let (is_text, number) = match v {
                    rusqlite::types::Value::Text(_) => (1, 0),
                    rusqlite::types::Value::Integer(i) => (0, *i),
                    _ => (0, 0),
                };
                flat.extend([i64::from(s[0]), is_text, number, i64::from(s[1])]);
            }
            let n = ordered.len();
            let rows_or_gone = if n == 0 { KeptWhat::Gone } else { KeptWhat::Flat { widths: vec![8; META_TYPES.len()], rows: flat.clone(), n, levels: None, given: true } };
            puts.push(KeptPut { rel: META.to_owned(), ident: ident(META), part: 0, digest: digest_of(&flat), reads: Vec::new(), types: META_TYPES.to_vec(), what: rows_or_gone });
        }
        self.store.queue_kept_as(self.space, puts, what);
    }

    /// `edit`'s partitions of `R`, read as kept, edited and pushed to `puts` whole. False if their atoms could not be
    /// interned (nothing is written then).
    fn edit<R: ByDir>(&self, chain: Option<&dyn KeptRead>, edit: Edit<R>, puts: &mut Vec<KeptPut>) -> bool {
        if edit.is_empty() {
            return true;
        }
        let ident = ident(R::REL);
        // The directories touched, each with its rows as kept.
        let mut dirs: BTreeMap<String, BTreeMap<String, R>> = BTreeMap::new();
        let load = |dirs: &mut BTreeMap<String, BTreeMap<String, R>>, dir: &str| {
            if dirs.contains_key(dir) {
                return;
            }
            let mut rows = BTreeMap::new();
            if let Some(kept) = chain.and_then(|c| c.get(&FactKey { rel: R::REL, ident: &ident, part: part(dir) })) {
                for i in 0..kept.rows.len() {
                    if let Some(row) = R::decode(self, &kept.row(i)) {
                        rows.insert(row.path().to_owned(), row);
                    }
                }
            }
            dirs.insert(dir.to_owned(), rows);
        };
        // Every partition at or under a path deleted with what is under it, found by its first row's directory.
        let mut whole_gone: BTreeSet<String> = BTreeSet::new();
        if !edit.delete_under.is_empty()
            && let Some(c) = chain
        {
            for kept in c.of(R::REL) {
                if kept.rows.len() == 0 {
                    continue;
                }
                let Some(first) = self.text(kept.row(0)[0]) else { continue };
                let dir = dir_of(&first).to_owned();
                if edit.delete_under.iter().any(|u| at_or_under(&dir, u)) {
                    whole_gone.insert(dir);
                }
            }
        }
        for p in edit.delete.iter().chain(&edit.delete_under) {
            load(&mut dirs, dir_of(p));
            if let Some(rows) = dirs.get_mut(dir_of(p)) {
                rows.remove(p);
            }
        }
        for dir in &whole_gone {
            dirs.insert(dir.clone(), BTreeMap::new());
        }
        for row in edit.put {
            let dir = dir_of(row.path()).to_owned();
            load(&mut dirs, &dir);
            dirs.get_mut(&dir).expect("loaded").insert(row.path().to_owned(), row);
        }
        for (dir, rows) in dirs {
            if rows.is_empty() {
                puts.push(KeptPut { rel: R::REL.to_owned(), ident, part: part(&dir), digest: 0, reads: Vec::new(), types: R::TYPES.to_vec(), what: KeptWhat::Gone });
                continue;
            }
            let texts: Vec<String> = rows.values().flat_map(|r| r.texts().into_iter().take(R::TEXTS)).collect();
            let Ok(syms) = self.store.intern_atoms(&texts.iter().map(String::as_str).collect::<Vec<_>>()) else { return false };
            let mut flat = Vec::with_capacity(rows.len() * R::TYPES.len());
            for (r, s) in rows.values().zip(syms.chunks_exact(R::TEXTS)) {
                flat.extend(r.words(s));
            }
            let n = rows.len();
            puts.push(KeptPut { rel: R::REL.to_owned(), ident, part: part(&dir), digest: digest_of(&flat), reads: Vec::new(), types: R::TYPES.to_vec(), what: KeptWhat::Flat { widths: vec![8; R::TYPES.len()], rows: flat, n, levels: None, given: true } });
        }
        true
    }
}

fn digest_of(flat: &[i64]) -> u64 {
    let mut h = Sha256::new();
    for v in flat {
        h.update(v.to_le_bytes());
    }
    u64::from_le_bytes(h.finalize()[..8].try_into().expect("8 bytes"))
}

/// Every answer's path kept, sorted, for a test.
#[cfg(test)]
pub(super) fn all_paths(facts: &Facts) -> Vec<String> {
    let mut out = facts.all().0.into_iter().map(|r| r.0).collect::<Vec<_>>();
    out.sort();
    out
}

/// Every canonical form's path kept, sorted, for a test.
#[cfg(test)]
pub(super) fn all_canonical(facts: &Facts) -> Vec<String> {
    let mut out = facts.all().1.into_iter().map(|r| r.0).collect::<Vec<_>>();
    out.sort();
    out
}

/// A digest as hex (empty for none).
fn hex_of(d: &[u8]) -> String {
    d.iter().map(|b| format!("{b:02x}")).collect()
}
