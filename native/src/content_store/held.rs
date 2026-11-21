//! The store's state this process holds, by the store's file (decision 0104: a run reads before its first wave and
//! asks no SQL inside the waves): each space's kept blocks' row (decision 0401, `kept_facts`), and the per-file memo's
//! admitted entries (decision 0240 §4, `memo_pack`). Every write this process makes keeps it current.
//!
//! Frames (decision 0104's first form: a relation's rows encoded in a row of the `frames` table) are retired: the
//! kept facts are blocks (decision 0401 step 9), and a store's frame tables and segments of before are dropped when
//! it is opened (`schema::RETIRED_TABLES`).
//!
//! What the memo keeps is what the kept manifests reach (decision 0227, `collect_unreachable_memo`): an entry while
//! its plan is live and a kept manifest names its content. No run marks what it used.

#[allow(unused_imports)]
use crate::content_store::connection::Cached as _;
use super::{ContentStore, StoreResult};
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub(super) mod memo_pack;
pub use memo_pack::{MemoBlob, MemoPut, MemoRow};
pub(super) use memo_pack::drop_memo_space;

/// A relation as a kept entry names it: its name and its identity, the digest of the rules it is derived from, their
/// reads' identities and the engine (`plan::identities`).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Rel {
    pub name: String,
    pub ident: [u8; 32],
}

/// A kept relation, without its rows: its digest and what it was derived from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stored {
    pub digest: u64,
    /// Each relation it was derived from, by name, with that relation's digest then.
    pub reads: Vec<(String, u64)>,
}

/// What this process holds of a store (decision 0104), shared by the store's handles in this process, by its path.
#[derive(Default)]
struct Snapshot {
    /// The memo spaces whose entries were admitted (`admit_memo_entries`).
    admitted: HashSet<[u8; 32]>,
    /// The memo entries admitted (`admit_memo_entries`) and written since, by space and part (decision 0240 §4).
    memo: HashMap<[u8; 32], HashMap<i64, Arc<MemoBlob>>>,
    /// Each space's chain of kept files (decision 0456) as its `facts_chain` rows name it: read before a run's first
    /// wave (`ContentStore::preload_kept`, decision 0104), and replaced by this process's writer as it commits a
    /// space's next block. `None` until read. A run binding a space inside its waves asks no SQL (`kept_row`).
    kept: Option<HashMap<[u8; 32], super::kept_facts::ChainRows>>,
}

/// The stores' snapshots, by store: the shared object's (decision 0442), shared by design by the runs on a store.
#[derive(Default)]
struct Snapshots(std::sync::Mutex<HashMap<std::path::PathBuf, Arc<std::sync::Mutex<Snapshot>>>>);

fn snapshots() -> Arc<Snapshots> {
    rerail_prof::code_rule_dsl::logic::profile::shared::<Snapshots>()
}

/// `f` over the kept blocks' rows this process holds of the store at `path` (`Snapshot::kept`); `None` when no
/// snapshot of it is installed.
pub(super) fn snapshot_kept<T>(path: &std::path::Path, f: impl FnOnce(&mut Option<HashMap<[u8; 32], super::kept_facts::ChainRows>>) -> T) -> Option<T> {
    let snapshot = snapshots().0.lock().expect("the store's snapshots").get(&super::atoms::key(path)).cloned()?;
    let mut snapshot = snapshot.lock().expect("the store's snapshot");
    Some(f(&mut snapshot.kept))
}

/// A store created at `path` (its schema new): a snapshot of an earlier file there is not this one's.
pub fn forget_snapshot(path: &str) {
    snapshots().0.lock().expect("the store's snapshots").remove(&super::atoms::key(std::path::Path::new(path)));
}

impl ContentStore {
    /// The snapshot of this store's file, by its canonical path: handles that name it differently share it.
    fn snapshot(&self) -> Option<Arc<std::sync::Mutex<Snapshot>>> {
        snapshots().0.lock().expect("the store's snapshots").get(&super::atoms::key(&self.database_path)).cloned()
    }

    /// What a run reads of the store before its first wave (decision 0104), once a process: the kept blocks' rows
    /// (`preload_kept`), held in the store's snapshot, which this process's writes keep current, and with it the
    /// memo's admissions (`admit_memo_entries`). Returns the spaces read now.
    pub fn preload_kept_state(&self) -> StoreResult<usize> {
        snapshots().0.lock().expect("the store's snapshots").entry(super::atoms::key(&self.database_path)).or_default();
        self.preload_kept()
    }
}

/// Collects the memo entries nothing live reaches (decision 0227), in `t` (a write transaction): those of a live
/// plan's space (`local_space`) whose content no kept manifest names. Their reads hold their document's sha256, and
/// `corpus_live` the kept manifests' sources. A dead plan's whole space goes with the plan (`collect_corpus_pass`).
/// How many.
pub(super) fn collect_unreachable_memo(t: &Connection) -> rusqlite::Result<usize> {
    let live: HashSet<[u8; 32]> = t
        .prepare("SELECT plan_digest FROM corpus_plans WHERE live = 1")?
        .query_map([], |r| r.get::<_, Vec<u8>>(0))?
        .filter_map(|p| p.ok()?.try_into().ok())
        .collect();
    // The kept manifests' contents, by sha256.
    let contents: HashSet<[u8; 32]> = t
        .prepare(
            "SELECT DISTINCT h.algorithm_id, h.lane0, h.lane1, h.lane2, h.lane3 FROM corpus_live l \
             JOIN source_objects s ON s.source_id = l.source_id JOIN hashes h ON h.hash_id = s.canonical_hash_id",
        )?
        .query_map([], |r| Ok(super::HashKey { algorithm: r.get(0)?, lanes: [r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?] }))?
        .filter_map(|k| k.ok()?.sha256_bytes())
        .collect();
    memo_pack::collect_memo_entries(t, &live, &contents)
}

/// The space of the per-file memo of plan `plan` (decision 0146): one space per file-local program.
pub fn local_space(plan: &[u8; 32]) -> [u8; 32] {
    use sha2::{Digest as _, Sha256};
    let mut h = Sha256::new();
    h.update(b"rerail.space/v1\0local\0");
    h.update(plan);
    h.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decision 0117: a memo keeps a path's newest contents only. Decision 0240 §4: its bytes are in the pack, which
    /// compaction rewrites with only what its rows name;.
    #[test]
    fn a_memo_keeps_a_paths_newest_and_its_pack_is_compacted() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        let space = local_space(&[5u8; 32]);
        // Entries of 4000 bytes: a segment's header and entry table are small beside them.
        const N: u64 = 4000;
        let put = |part: i64, path: &str| MemoPut { part, path: path.into(), digest: [part as u8; 32], key: 1, bytes: vec![part as u8; N as usize], over: false };
        let segments = || std::fs::read_dir(dir.path().join("store.sqlite.memo")).unwrap().count();
        // Six contents of a.py, one batch each, and one of b.py.
        for part in 0..6 {
            store.put_memo_entries(&space, vec![put(part, "a.py")], 4).unwrap();
        }
        store.put_memo_entries(&space, vec![put(100, "b.py")], 4).unwrap();
        let parts: Vec<i64> = store.connection.lock().unwrap().prepare("SELECT part FROM memo_entries WHERE space = ?1 ORDER BY part").unwrap().query_map([&space[..]], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
        assert_eq!(parts, vec![2, 3, 4, 5, 100], "a.py's two oldest contents go");
        let (held, named) = store.memo_pack_bytes().unwrap();
        assert_eq!((named, segments()), (5 * N, 7), "one segment a write");
        assert!(held > 7 * N && held < 7 * N + 7 * 1024, "each segment its entry and a little more: {held}");
        assert_eq!(store.compact_memo_pack(1 << 40).unwrap(), 0, "below the floor");
        assert_eq!(store.compact_memo_pack(0).unwrap(), 0, "less than half unnamed");
        assert_eq!(store.compact_memo_pack_within(1 << 40, 7).unwrap(), 0, "no more segments than the budget");
        store.put_memo_entries(&space, (0..6).map(|i| put(200 + i, "c.py")).collect(), 1).unwrap();
        assert_eq!(store.memo_pack_bytes().unwrap().1, 6 * N);
        let before = store.memo_entries(&space, &[2, 5, 100, 205], true).unwrap();
        let (held, _) = store.memo_pack_bytes().unwrap();
        let freed = store.compact_memo_pack(0).unwrap();
        let (now, named) = store.memo_pack_bytes().unwrap();
        assert_eq!((freed, named, segments()), (held - now, 6 * N, 1), "one segment left, of the named entries");
        assert!(now < 6 * N + 1024, "{now}");
        let after = store.memo_entries(&space, &[2, 5, 100, 205], true).unwrap();
        assert_eq!(after.len(), 4);
        for (part, blob) in &before {
            assert_eq!(after[part].bytes.bytes(), blob.bytes.bytes(), "part {part} reads the same from the new segment");
        }
        drop(before);
    }

    /// The writer runs each job as a savepoint of its batch's one immediate transaction (`in_one_transaction`), so a
    /// job's segment is named (`SealedSegments::name`) before the batch commits, and a compaction, which takes the
    /// store's write lock, cannot read the job's rows before then: one tried during the batch finds the store busy
    /// and merges nothing; one after it merges both entries and forgets neither.
    #[test]
    fn a_compaction_cannot_run_inside_the_batch_that_names_its_segments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.sqlite");
        let store = ContentStore::open(&path).unwrap();
        let space = local_space(&[9u8; 32]);
        let put = |part: i64| MemoPut { part, path: format!("{part}.py"), digest: [part as u8; 32], key: 1, bytes: vec![part as u8; 100], over: false };
        // The memo directory's files: a named segment is `<n>.seg`, one not named yet its writer's `w-<pid>-<n>.seg`.
        let segments = || -> Vec<String> { std::fs::read_dir(dir.path().join("store.sqlite.memo")).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect() };
        let named = |files: &[String]| files.iter().all(|f| f.strip_suffix(".seg").is_some_and(|n| n.parse::<i64>().is_ok()));
        store.put_memo_entries(&space, vec![put(0)], 4).unwrap();
        let during = store
            .in_one_transaction(|| {
                store.put_memo_entries(&space, vec![put(1)], 4).unwrap();
                let compaction = std::thread::scope(|s| s.spawn(|| ContentStore::open(&path).unwrap().compact_memo_pack_within(u64::MAX, 0)).join().unwrap());
                (compaction.map_err(|e| e.to_string()), segments())
            })
            .unwrap();
        assert_eq!(during.0, Err("content store writer is busy".to_owned()), "during the batch: busy");
        assert!(during.1.len() == 2 && named(&during.1), "during the batch, before its commit: both segments named, none merged: {:?}", during.1);
        ContentStore::open(&path).unwrap().compact_memo_pack_within(u64::MAX, 0).unwrap();
        assert_eq!(segments().len(), 1, "after it: both segments merged");
        assert_eq!(store.memo_entries(&space, &[0, 1], true).unwrap().len(), 2, "neither entry forgotten");
    }

    /// A memo's segments past a budget are merged though their bytes are named, all but the oldest half of the budget,
    /// into one numbered after them: a path's entries keep their order (its newest the one in the newest segment), so
    /// what is kept of a path is its newest; every entry, and what is kept over one, reads the same.
    #[test]
    fn a_memos_segments_past_the_budget_are_merged_in_their_order() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        let space = local_space(&[5u8; 32]);
        let put = |part: i64, over: bool| MemoPut { part, path: "a.py".into(), digest: [part as u8; 32], key: 1, bytes: vec![part as u8 + u8::from(over); 4000], over };
        let segments = || std::fs::read_dir(dir.path().join("store.sqlite.memo")).unwrap().count();
        // Seven contents of a.py, one batch each, all kept, and what is kept over the newest, in an eighth.
        for part in 0..7 {
            store.put_memo_entries(&space, vec![put(part, false)], 8).unwrap();
        }
        store.put_memo_entries(&space, vec![put(6, true)], 8).unwrap();
        let read = || -> std::collections::BTreeMap<i64, (Vec<u8>, Option<Vec<u8>>)> {
            let found = store.memo_entries(&space, &[0, 1, 2, 3, 4, 5, 6], true).unwrap();
            found.into_iter().map(|(part, blob)| (part, (blob.bytes.bytes().to_vec(), blob.over.as_ref().map(|o| o.bytes().to_vec())))).collect()
        };
        let unmerged = read();
        assert_eq!((segments(), unmerged.len(), unmerged[&6].1.is_some()), (8, 7, true));
        assert_eq!(store.compact_memo_pack(0).unwrap(), 0, "named, and fewer than the budget");
        assert_eq!(store.compact_memo_pack_within(1 << 40, 8).unwrap(), 0, "no more than the budget");
        store.compact_memo_pack_within(1 << 40, 4).unwrap();
        assert_eq!(segments(), 3, "the oldest two left, the newer six merged into one after them");
        assert_eq!(read(), unmerged, "every entry reads the same");
        // An eighth content, keeping three: a.py's newest three are kept, as before the merge.
        store.put_memo_entries(&space, vec![put(7, false)], 3).unwrap();
        let parts: Vec<i64> = store.connection.lock().unwrap().prepare("SELECT part FROM memo_entries WHERE space = ?1 ORDER BY part").unwrap().query_map([&space[..]], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
        assert_eq!(parts, [5, 6, 7], "the newest three contents");
    }

    /// Decision 0280: what a later run keeps over an entry (its identities) is read with it, from the store and from
    /// the snapshot, survives compaction, is dropped where the entry is not the same content, and goes when the
    /// entry is written again whole.
    #[test]
    fn what_is_kept_over_an_entry_is_read_with_it_and_goes_with_it() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContentStore::open(&dir.path().join("store.sqlite")).unwrap();
        let space = local_space(&[6u8; 32]);
        let put = |part: i64, bytes: Vec<u8>, over: bool| MemoPut { part, path: "a.py".into(), digest: [1; 32], key: 1, bytes, over };
        // Written twice whole: the first copy's bytes are no row's.
        store.put_memo_entries(&space, vec![put(1, vec![6; 4000], false)], 4).unwrap();
        store.put_memo_entries(&space, vec![put(1, vec![7; 50], false)], 4).unwrap();
        store.put_memo_entries(&space, vec![put(1, vec![8; 10], true), MemoPut { digest: [2; 32], ..put(2, vec![9; 10], true) }], 4).unwrap();
        let found = store.memo_entries(&space, &[1, 2], true).unwrap();
        assert_eq!(found.len(), 1, "an entry kept over nothing is no entry");
        let over = found[&1].over.clone().expect("kept over the entry");
        assert_eq!((found[&1].bytes.bytes(), over.bytes()), (&[7; 50][..], &[8; 10][..]));
        assert_eq!(store.memo_pack_bytes().unwrap().1, 60, "what is kept over it is named; the other is not");
        assert!(store.compact_memo_pack(0).unwrap() > 4000);
        let after = store.memo_entries(&space, &[1], true).unwrap();
        assert_eq!((after[&1].bytes.bytes(), after[&1].over.as_ref().map(|o| o.bytes())), (&[7; 50][..], Some(&[8; 10][..])), "moved together");
        store.put_memo_entries(&space, vec![put(1, vec![5; 20], false)], 4).unwrap();
        assert!(store.memo_entries(&space, &[1], true).unwrap()[&1].over.is_none(), "written again whole");
    }
}
