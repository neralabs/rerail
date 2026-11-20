//! The store's atoms where they lie (decision 0401 A2–A4): a chain of atoms blocks in `<store>.atoms/`, each covering
//! the ids after the one before, mapped by every process that opens the store and read up to the end its last commit
//! reached. SQLite keeps `atom_blocks`: each file and that end. Nothing stored is copied onto a heap: a string is read,
//! and a lookup probes each block's table, in the mapping.
//!
//! The newest block is open, and the process that holds SQLite's write lock appends to it (`append`), under the atoms
//! lease (0049) or as the writer that resolves a fork (0170). A block that fills is left as it is and the next, twice
//! its size, is started; the roll-up writes them into one base.

use super::Sym;
use crate::content_store::{StoreError, StoreResult};
use rerail_blocks::atoms::{AtomRead, AtomsV, AtomsW};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

/// The first block's room: atoms, string bytes. Each next block has twice the last's. (The store's own tests start
/// small, so that their stores grow chains and roll them up.)
#[cfg(not(test))]
const FIRST_CAP: u32 = 1 << 16;
#[cfg(not(test))]
const FIRST_BYTES: u64 = 4 << 20;
#[cfg(test)]
pub(super) const FIRST_CAP: u32 = 16;
#[cfg(test)]
const FIRST_BYTES: u64 = 256;

/// More deltas than this make a roll-up due (A5), as do deltas holding more atoms than the base.
const MOST_DELTAS: usize = 4;

/// Whether a block's file is a base (a roll-up's, frozen) rather than a delta.
fn is_base(r: &Row) -> bool {
    r.name.starts_with('b')
}

/// A block as `atom_blocks` names it: its number, its file's name in the directory, its first id, the end the
/// store's last commit reached in it, and its maker's tag (the block's own, checked when it is mapped).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub seq: i64,
    pub name: String,
    pub first: Sym,
    pub upto: Sym,
    pub tag: u64,
}

/// The store's blocks as `connection` sees them (in its transaction, if it has one), by first id.
pub(crate) fn rows(connection: &Connection) -> rusqlite::Result<Vec<Row>> {
    connection
        .prepare_cached("SELECT seq, name, first, upto, tag FROM main.atom_blocks ORDER BY first")?
        .query_map([], |r| Ok(Row { seq: r.get(0)?, name: r.get(1)?, first: r.get::<_, i64>(2)? as Sym, upto: r.get::<_, i64>(3)? as Sym, tag: r.get::<_, i64>(4)? as u64 }))?
        .collect()
}

/// Where the store's ids end: atom 0 (the empty string) is no block's.
pub(crate) fn end_of(rows: &[Row]) -> Sym {
    rows.iter().map(|r| r.upto).max().unwrap_or(1).max(1)
}

/// Where `connection`'s store's ids end.
pub(crate) fn database_end(connection: &Connection) -> rusqlite::Result<usize> {
    let end: Option<i64> = connection.prepare_cached("SELECT max(upto) FROM main.atom_blocks")?.query_row([], |r| r.get(0))?;
    Ok(end.unwrap_or(1).max(1) as usize)
}

/// The directory of a store's atoms blocks, beside its file.
pub fn atoms_dir(database: &Path) -> PathBuf {
    let mut dir = database.as_os_str().to_owned();
    dir.push(".atoms");
    PathBuf::from(dir)
}

fn io(error: std::io::Error) -> StoreError {
    StoreError::Corrupt(format!("atoms blocks: {error}"))
}

/// A block this process knows of: its number, file name, first id and tag.
struct Known {
    seq: i64,
    name: String,
    first: Sym,
    tag: u64,
}

/// A new block's tag: no other block's, in practice.
pub(crate) fn new_tag() -> u64 {
    use std::hash::BuildHasher as _;
    let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    std::collections::hash_map::RandomState::new().hash_one((at, std::process::id()))
}

/// `v`, if it is the block its row names (`tag`).
fn checked<T>(v: T, tag_of: impl Fn(&T) -> u64, tag: u64, path: &Path) -> StoreResult<T> {
    if tag_of(&v) != tag {
        return Err(StoreError::Corrupt(format!("{}: not the atoms block the store names (a store copied without its .atoms folder, or a file left from another store)", path.display())));
    }
    Ok(v)
}

/// The store's chain, as this process maps it.
#[derive(Default)]
pub(crate) struct Stored {
    /// `None` for an in-memory database, whose atoms are never written.
    dir: Option<PathBuf>,
    /// The blocks known, by first id: from `atom_blocks` as last read, and those this process started since.
    known: Vec<Known>,
    /// Each mapped block by its number, read up to the end below which it is committed, in `known`'s order.
    views: Vec<(i64, AtomsV)>,
    /// Every id below this is read through the views.
    end: Sym,
    /// The open block, this process's writer of it (kept between writes; checked against the rows every write).
    writer: Option<(i64, AtomsW)>,
}

impl Stored {
    pub(crate) fn new(database: Option<&Path>) -> Stored {
        Stored { dir: database.map(atoms_dir), known: Vec::new(), views: Vec::new(), end: 1, writer: None }
    }

    /// The blocks are those of `rows` (read on a connection). Their ids are mapped by `view_to` or `view_rows`, and a
    /// block no longer named (rolled up) is unmapped then, once those that hold its ids are mapped: if mapping fails,
    /// what was read before still reads.
    pub(crate) fn learn(&mut self, rows: &[Row]) {
        self.known = rows.iter().map(|r| Known { seq: r.seq, name: r.name.clone(), first: r.first, tag: r.tag }).collect();
    }

    /// The views of blocks no longer known go.
    fn forget_unnamed(&mut self) {
        let known = &self.known;
        self.views.retain(|(seq, v)| known.iter().any(|k| k.seq == *seq && k.first == v.first()));
    }

    /// Reads every id below `upto` (at least what it read before): the blocks that hold them mapped, each up to where
    /// the next begins. `upto` is an end a commit reached. A block known and not mapped yet (a roll-up's base, once its
    /// deltas' views went with `learn`) is mapped now.
    pub(crate) fn view_to(&mut self, upto: Sym) -> StoreResult<()> {
        let upto = upto.max(self.end);
        let Some(dir) = self.dir.clone() else { return Ok(()) };
        for (i, k) in self.known.iter().enumerate() {
            if k.first >= upto {
                break;
            }
            let bound = self.known.get(i + 1).map_or(upto, |next| next.first.min(upto));
            match self.views.iter_mut().find(|(seq, _)| *seq == k.seq) {
                Some((_, v)) => v.extend(bound).map_err(io)?,
                None => {
                    let path = dir.join(&k.name);
                    let v = checked(AtomsV::open(&path, bound).map_err(io)?, AtomsV::tag, k.tag, &path)?;
                    let at = self.views.partition_point(|(_, w)| w.first() < v.first());
                    self.views.insert(at, (k.seq, v));
                }
            }
        }
        self.forget_unnamed();
        if upto > 1 && self.views.last().is_none_or(|(_, v)| v.ids().end < upto) {
            return Err(StoreError::Corrupt(format!("the store's atoms end at {upto}, past its blocks")));
        }
        self.end = upto;
        Ok(())
    }

    /// Reads every block exactly to its row's end, nearer than before too: a fork's view of the store inside a write
    /// transaction (its own appends included), and after one that rolled back (decision 0170). Only ids past the fork
    /// are read so; the store's below it are committed, and no row goes back below them.
    pub(crate) fn view_rows(&mut self, rows: &[Row]) -> StoreResult<()> {
        self.learn(rows);
        let Some(dir) = self.dir.clone() else { return Ok(()) };
        for (i, r) in rows.iter().enumerate() {
            let bound = rows.get(i + 1).map_or(r.upto, |next| next.first.min(r.upto));
            match self.views.iter_mut().find(|(seq, _)| *seq == r.seq) {
                Some((_, v)) => v.set_end(bound).map_err(io)?,
                None => {
                    let path = dir.join(&r.name);
                    let v = checked(AtomsV::open(&path, bound).map_err(io)?, AtomsV::tag, r.tag, &path)?;
                    let at = self.views.partition_point(|(_, w)| w.first() < v.first());
                    self.views.insert(at, (r.seq, v));
                }
            }
        }
        self.forget_unnamed();
        self.end = end_of(rows);
        Ok(())
    }

    fn at(&self, id: Sym) -> Option<&AtomsV> {
        let i = self.views.partition_point(|(_, v)| v.first() <= id).checked_sub(1)?;
        Some(&self.views[i].1)
    }

    pub(crate) fn str(&self, id: Sym) -> Option<&str> {
        self.at(id)?.str(id)
    }

    /// The ids read, as a directory's runs (decision 0478 step 2b): each view's from its first id to the next's, at
    /// most to the end below which ids are read here.
    pub(crate) fn runs(&self, below: Sym) -> Vec<(rerail_blocks::atoms::Run, std::sync::Arc<rerail_blocks::block::Block>)> {
        let end = self.end.min(below);
        (0..self.views.len())
            .map(|k| {
                let next = self.views.get(k + 1).map_or(end, |(_, v)| v.first().min(end));
                (self.views[k].1.run_upto(next), std::sync::Arc::clone(self.views[k].1.block()))
            })
            .filter(|(r, _)| r.first < r.end)
            .collect()
    }

    pub(crate) fn hash(&self, id: Sym) -> Option<u64> {
        self.at(id)?.hash(id)
    }

    /// The id of `s` (its hash `h`) among the ids read: each block's table probed, the newest first.
    pub(crate) fn find(&self, s: &str, h: u64) -> Option<Sym> {
        self.views.iter().rev().find_map(|(_, v)| v.find(s, h))
    }

    /// Lets the open block's writer go (its file's descriptor with it): the store's last handle closed. The next write
    /// opens it again.
    pub(crate) fn release_writer(&mut self) {
        self.writer = None;
    }

    /// Mapped bytes (none of them heap).
    pub(crate) fn mapped(&self) -> usize {
        self.views.iter().map(|(_, v)| v.mapped()).sum()
    }

    /// In `connection`'s write transaction (SQLite's write lock held): appends `atoms` (each string and its hash) at
    /// the store's end, which is `from`, and records how far each block they went into reaches, in the transaction.
    /// Each block written is on the disk before this returns (A3: before the commit that names it). The ids are
    /// `from..`; returns the end. The rows are read first, so the blocks are the store's as of this transaction.
    pub(crate) fn append<'s>(&mut self, connection: &Connection, from: Sym, atoms: impl ExactSizeIterator<Item = (&'s str, u64)>) -> StoreResult<Sym> {
        let Some(dir) = self.dir.clone() else { return Err(StoreError::Invalid("an in-memory database's atoms are not stored".into())) };
        let rows = rows(connection)?;
        let end = end_of(&rows);
        if end != from {
            return Err(StoreError::Corrupt(format!("atoms appended from {from}, but the store's end is {end}")));
        }
        self.learn(&rows);
        if atoms.len() == 0 {
            return Ok(end);
        }
        // The open block, taken up from its committed end; another process's crashed appends past it are taken back.
        let open = rows.last().filter(|r| r.upto == end);
        match (open, self.writer.as_mut()) {
            (Some(r), Some((seq, w))) if *seq == r.seq && w.first() == r.first && w.tag() == r.tag && !w.frozen() => w.resume(r.upto).map_err(io)?,
            (Some(r), _) => {
                let path = dir.join(&r.name);
                self.writer = match AtomsW::reopen(&path, r.upto, crate::content_store::connection::flush_of(connection)) {
                    Ok(w) => Some((r.seq, checked(w, AtomsW::tag, r.tag, &path)?)),
                    // Frozen (rolled up into): the next block is started.
                    Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => None,
                    Err(e) => return Err(io(e)),
                };
            }
            (None, _) => self.writer = None,
        }
        let mut next_seq = rows.iter().map(|r| r.seq).max().unwrap_or(0) + 1;
        let mut id = end;
        for (s, h) in atoms {
            loop {
                if let Some((_, w)) = self.writer.as_mut()
                    && let Some(given) = w.append(s, h)
                {
                    debug_assert_eq!(given, id);
                    break;
                }
                // Full, or none open: the one written so far is on the disk and its row says how far it reaches; the
                // next is twice its size, and has room for this string.
                let (cap, bytes) = match self.writer.take() {
                    Some((seq, w)) => {
                        Self::seal(connection, seq, &w)?;
                        (w.cap().saturating_mul(2), w.bytes_cap().saturating_mul(2))
                    }
                    None => (FIRST_CAP, FIRST_BYTES),
                };
                let bytes = bytes.max(s.len() as u64 * 2);
                private_dir(&dir).map_err(io)?;
                let name = format!("d{next_seq}.blk");
                let tag = new_tag();
                // Made under a name of its own, then renamed into place: a file left at the name (a rolled-back
                // transaction's, a deleted store's) may be mapped by a process still reading it, and is never
                // truncated under it.
                let made = dir.join(format!("{name}.{tag:016x}.new"));
                let w = AtomsW::create(&made, id, tag, cap, bytes, crate::content_store::connection::flush_of(connection)).map_err(io)?;
                std::fs::rename(&made, dir.join(&name)).map_err(io)?;
                sync_dir(&dir, crate::content_store::connection::flush_of(connection)).map_err(io)?;
                connection
                    .prepare_cached("INSERT INTO main.atom_blocks(seq, name, first, upto, tag) VALUES (?1, ?2, ?3, ?3, ?4)")?
                    .execute(rusqlite::params![next_seq, name, i64::from(id), tag as i64])?;
                self.known.push(Known { seq: next_seq, name, first: id, tag });
                self.writer = Some((next_seq, w));
                next_seq += 1;
            }
            id += 1;
        }
        if let Some((seq, w)) = &self.writer {
            Self::seal(connection, *seq, w)?;
        }
        let rows = self::rows(connection)?;
        if roll_up_due(&rows) {
            self.roll_up(connection, &rows)?;
        } else {
            sweep(&dir, &rows);
        }
        Ok(id)
    }

    /// A5: every atom of `rows` written into one base, frozen (its table sized for them all), in place of them, in
    /// `connection`'s write transaction. Ids never change. This process reads through the base at once (it holds what
    /// the blocks did, for every id they held); the old files go at the next write after the commit (`sweep`), and a
    /// process still mapping one keeps reading it.
    fn roll_up(&mut self, connection: &Connection, rows: &[Row]) -> StoreResult<()> {
        let Some(dir) = self.dir.clone() else { return Ok(()) };
        let end = end_of(rows);
        // The blocks as this transaction has them, its own appends included.
        let mut all = Stored { dir: Some(dir.clone()), known: Vec::new(), views: Vec::new(), end: 1, writer: None };
        all.view_rows(rows)?;
        let mut bytes = 0u64;
        for id in 1..end {
            bytes += all.str(id).ok_or_else(|| StoreError::Corrupt(format!("stored atom {id} is in no block")))?.len() as u64;
        }
        let seq = rows.iter().map(|r| r.seq).max().unwrap_or(0) + 1;
        let (name, tag) = (format!("b{seq}.blk"), new_tag());
        let made = dir.join(format!("{name}.{tag:016x}.new"));
        let mut w = AtomsW::create(&made, 1, tag, (end - 1).max(1), bytes.max(1), crate::content_store::connection::flush_of(connection)).map_err(io)?;
        for id in 1..end {
            let (s, h) = (all.str(id).expect("read above"), all.hash(id).expect("read above"));
            if w.append(s, h) != Some(id) {
                return Err(StoreError::Corrupt(format!("a roll-up's base refused atom {id}")));
            }
        }
        w.freeze().map_err(io)?;
        std::fs::rename(&made, dir.join(&name)).map_err(io)?;
        sync_dir(&dir, crate::content_store::connection::flush_of(connection)).map_err(io)?;
        connection.prepare_cached("DELETE FROM main.atom_blocks")?.execute([])?;
        connection
            .prepare_cached("INSERT INTO main.atom_blocks(seq, name, first, upto, tag) VALUES (?1, ?2, 1, ?3, ?4)")?
            .execute(rusqlite::params![seq, name, i64::from(end), tag as i64])?;
        // Read through the base from now on: the deltas' views go, and the base's covers what they did.
        let base = Row { seq, name, first: 1, upto: end, tag };
        let was = self.end;
        self.learn(std::slice::from_ref(&base));
        self.end = 1;
        self.view_to(was)?;
        self.writer = None;
        Ok(())
    }

    /// A block's appends written back (on the disk with `Flush::Synced`, decision 0481), then its row's end moved to
    /// them, in the transaction.
    fn seal(connection: &Connection, seq: i64, w: &AtomsW) -> StoreResult<()> {
        w.sync().map_err(io)?;
        connection.prepare_cached("UPDATE main.atom_blocks SET upto = ?1 WHERE seq = ?2")?.execute(rusqlite::params![i64::from(w.end()), seq])?;
        Ok(())
    }
}

/// Whether a roll-up is due (A5): more than `MOST_DELTAS` deltas, or deltas holding more atoms than the base.
fn roll_up_due(rows: &[Row]) -> bool {
    let base: u64 = rows.iter().filter(|r| is_base(r)).map(|r| u64::from(r.upto - r.first)).sum();
    let deltas: Vec<&Row> = rows.iter().filter(|r| !is_base(r)).collect();
    let held: u64 = deltas.iter().map(|r| u64::from(r.upto - r.first)).sum();
    deltas.len() > MOST_DELTAS || (base > 0 && held > base)
}

/// Removes the files of `dir` that `rows` (the store's, read in a write transaction) do not name: blocks a committed
/// roll-up replaced, a rolled-back one's base, a crashed maker's file under its making name. A process still mapping
/// one keeps reading it (unlinked, never truncated).
fn sweep(dir: &Path, rows: &[Row]) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !rows.iter().any(|r| r.name == name) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// The directory made private to its user, as the store's other directories are.
pub(crate) fn private_dir(dir: &Path) -> std::io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)
}

/// A new file's name on the disk before the commit that names it, with `Flush::Synced`; otherwise the OS's to write
/// (decision 0481).
pub(crate) fn sync_dir(dir: &Path, flush: rerail_blocks::block::Flush) -> std::io::Result<()> {
    if !flush.waits() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        std::fs::File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

/// The atoms blocks of the store at `from` copied beside the store at `to` (in place of any there): a store copied
/// whole (`VACUUM INTO` the file, then this). Each file is cloned where the file system makes one (APFS). A process
/// still mapping a file it replaces keeps it (unlinked, never truncated).
pub fn copy_blocks(from: &Path, to: &Path) -> std::io::Result<()> {
    let (blocks, into) = (atoms_dir(from), atoms_dir(to));
    if into.exists() {
        std::fs::remove_dir_all(&into)?;
    }
    let Ok(entries) = std::fs::read_dir(&blocks) else { return Ok(()) };
    private_dir(&into)?;
    for e in entries.flatten() {
        let name = e.file_name();
        if name.to_string_lossy().ends_with(".new") {
            continue;
        }
        clone_file(&e.path(), &into.join(name))?;
    }
    Ok(())
}

/// `to`, a clone of `from` where the file system makes one (APFS), else a copy.
pub(crate) fn clone_file(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(target_vendor = "apple")]
    {
        use std::os::unix::ffi::OsStrExt as _;
        let a = std::ffi::CString::new(from.as_os_str().as_bytes())?;
        let b = std::ffi::CString::new(to.as_os_str().as_bytes())?;
        // SAFETY: both are NUL-terminated paths that live across the call.
        if unsafe { libc::clonefile(a.as_ptr(), b.as_ptr(), 0) } == 0 {
            return Ok(());
        }
    }
    std::fs::copy(from, to).map(|_| ())
}

/// Freezes every block of the store at `database` (a template, about to be copied): each shrinks to what it holds,
/// and a copy's first write starts a block of its own. Ids and rows do not change.
pub fn freeze_blocks(database: &Path) -> StoreResult<()> {
    let connection = Connection::open(database)?;
    let dir = atoms_dir(database);
    for r in rows(&connection)? {
        let path = dir.join(&r.name);
        match AtomsW::reopen(&path, r.upto, crate::content_store::connection::flush_of(&connection)) {
            Ok(w) => checked(w, AtomsW::tag, r.tag, &path)?.freeze().map_err(io)?,
            Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => {}
            Err(e) => return Err(io(e)),
        }
    }
    Ok(())
}

/// Appends `strings` the store does not hold yet at its end, in a write transaction of `connection`'s own, as
/// another process would (tests of forks, decision 0170, and `RERAIL_TEST_FORK_AFTER`). Returns how many were new and
/// the store's end after.
pub(crate) fn append_as_another(connection: &Connection, strings: &[String]) -> StoreResult<(usize, Sym)> {
    let path = connection.path().filter(|p| !p.is_empty()).map(PathBuf::from).ok_or_else(|| StoreError::Invalid("not a store on disk".into()))?;
    let own = connection.is_autocommit();
    if own {
        connection.execute_batch("BEGIN IMMEDIATE")?;
    }
    let done = (|| {
        let mut stored = Stored::new(Some(&path));
        let rows = rows(connection)?;
        let end = end_of(&rows);
        stored.learn(&rows);
        stored.view_to(end)?;
        let mut seen = std::collections::HashSet::new();
        let fresh: Vec<&String> = strings.iter().filter(|s| !s.is_empty() && stored.find(s, super::hash_str(s)).is_none() && seen.insert(s.as_str())).collect();
        let end = stored.append(connection, end, fresh.iter().map(|s| (s.as_str(), super::hash_str(s))))?;
        Ok((fresh.len(), end))
    })();
    if own {
        match &done {
            Ok(_) => connection.execute_batch("COMMIT")?,
            Err(_) => connection.execute_batch("ROLLBACK")?,
        }
    }
    done
}

/// Every stored atom of `connection`'s store, by id (atom 0 the empty string): what its blocks hold, read on its rows.
pub(crate) fn every_stored(connection: &Connection) -> StoreResult<Vec<String>> {
    let path = connection.path().filter(|p| !p.is_empty()).map(PathBuf::from);
    let mut stored = Stored::new(path.as_deref());
    let rows = rows(connection)?;
    let end = end_of(&rows);
    stored.learn(&rows);
    stored.view_to(end)?;
    let mut out = vec![String::new()];
    for id in 1..end {
        out.push(stored.str(id).ok_or_else(|| StoreError::Corrupt(format!("stored atom {id} is in no block")))?.to_owned());
    }
    Ok(out)
}
