use super::*;
use rerail_blocks::block::{Block, kind};
use rerail_blocks::relation::{RelW, Spec, freeze};
use rusqlite::Connection;

/// A relation of the run's, as the engine lends one: its rows in a block of its own.
struct Lent {
    block: Block,
    at: usize,
    dead: Vec<u64>,
}

// SAFETY: a frozen relation in a block nothing writes.
unsafe impl Send for Lent {}
unsafe impl Sync for Lent {}

impl KeptRows for Lent {
    fn relation(&self) -> &Relation {
        // SAFETY: `at` is the relation's descriptor in `block`, which lives as long as this.
        unsafe { &*self.block.ptr::<Relation>(self.at) }
    }
    fn dead(&self) -> &[u64] {
        &self.dead
    }
    fn levels(&self) -> &[u32] {
        &[]
    }
}

fn lent(rows: &[(i64, i64)]) -> Box<dyn KeptRows> {
    let block = Block::anon(kind::FACTS, 16 << 20);
    let spec = Spec { widths: vec![8, 8], order: vec![0], indexes: vec![vec![0]], min_log: 4 };
    let rel = Relation::create(&block, &spec, None);
    let lock = std::sync::Mutex::new(());
    let w = RelW { block: &block, rel, lock: &lock };
    for &(a, b) in rows {
        w.push(&[a, b]);
    }
    freeze(&block, rel, &spec, None);
    let at = block.offset_of(std::ptr::from_ref(rel));
    Box::new(Lent { block, at, dead: Vec::new() })
}

const SPACE: [u8; 32] = [7; 32];
const ID: [u8; 32] = [1; 32];

fn put(rel: &str, part: i64, digest: u64, what: KeptWhat) -> KeptPut {
    KeptPut { rel: rel.into(), ident: ID, part, digest, reads: vec![("r".into(), digest)], types: b"ii".to_vec(), what }
}

fn flat(rows: &[i64]) -> KeptWhat {
    KeptWhat::Flat { widths: vec![8, 8], rows: rows.to_vec(), n: rows.len() / 2, levels: None }
}

/// A store's file with its schema, and a connection to it.
fn store(name: &str) -> (PathBuf, Connection) {
    let dir = std::env::temp_dir().join(format!("rerail-kept-facts-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("store.sqlite");
    drop(crate::content_store::ContentStore::open(&path).unwrap());
    let c = Connection::open(&path).unwrap();
    (path, c)
}

fn write(path: &Path, c: &Connection, puts: Vec<KeptPut>) {
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    write_space(path, c, &SPACE, puts, None).unwrap();
    c.execute_batch("COMMIT").unwrap();
}

/// The space as a new reader opens it: each entry's (rel, part, digest, rows in canonical order).
fn read(path: &Path, c: &Connection) -> Vec<(String, i64, u64, Vec<Vec<i64>>)> {
    let rows = chain_rows(c, &SPACE).unwrap();
    let space = KeptSpace::open(path, &SPACE, &rows).unwrap();
    space.view().all().map(|k| (k.key.rel.to_owned(), k.key.part, k.digest, (0..k.rows.len()).map(|i| k.row(i)).collect())).collect()
}

fn files(path: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(space_dir(path, &SPACE)).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    out.sort();
    out
}

/// K-R5 (decision 0402): a write appends one file of its puts only (a relation's live rows packed, a granule's
/// rows, a reads-only entry, a gone entry), read with the chain as one; small writes merge into the newest small
/// file; a write as big as the chain merges every file into one, read the same, and a reader of the old chain reads
/// it still; the files a commit replaced go at the next write.
#[test]
fn a_space_appends_and_rolls_up() {
    let (path, c) = store("space");
    // 100,000 rows of no pattern a column's encoding takes away: about 16 bits a value, past what a write merges.
    let big: Vec<i64> = (0..100_000i64).flat_map(|i| [(i * 7919) % 100_003, (i * 104_729) % 150_001]).collect();
    write(&path, &c, vec![put("p", 0, 1, KeptWhat::Rows(lent(&[(3, 30), (1, 10)]))), put("q", 0, 2, flat(&big)), put("g", 4, 3, flat(&[5, 6]))]);
    assert_eq!(chain_rows(&c, &SPACE).unwrap().len(), 1);
    assert_eq!(read(&path, &c)[1], ("p".into(), 0, 1, vec![vec![1, 10], vec![3, 30]]), "a relation's rows in canonical order");
    let q_bytes = std::fs::metadata(space_dir(&path, &SPACE).join(&chain_rows(&c, &SPACE).unwrap()[0].1)).unwrap().len();
    assert!(q_bytes < 100_000 * 16 / 2, "packed: {q_bytes} bytes for 100,000 rows of two 8-byte columns");
    // A reader of the first file as it is now.
    let first = KeptSpace::open(&path, &SPACE, &chain_rows(&c, &SPACE).unwrap()).unwrap();
    write(&path, &c, vec![put("p", 0, 11, flat(&[9, 9])), put("g", 4, 0, KeptWhat::Gone)]);
    let rows = chain_rows(&c, &SPACE).unwrap();
    assert_eq!(rows.len(), 2, "appended");
    let second = std::fs::metadata(space_dir(&path, &SPACE).join(&rows[1].1)).unwrap().len();
    assert!(second < q_bytes / 4, "the second file holds its puts only: {second} bytes");
    let got = read(&path, &c);
    assert_eq!(got.iter().map(|(r, p, d, _)| (r.as_str(), *p, *d)).collect::<Vec<_>>(), [("p", 0, 11), ("q", 0, 2)]);
    // Reads only: the rows as they were, with new reads.
    let mut retag = put("q", 0, 2, KeptWhat::Reads);
    retag.reads = vec![("r".into(), 99)];
    write(&path, &c, vec![retag]);
    let space = KeptSpace::open(&path, &SPACE, &chain_rows(&c, &SPACE).unwrap()).unwrap();
    let view = space.view();
    let q = view.get(&FactKey { rel: "q", ident: &ID, part: 0 }).unwrap();
    assert_eq!((q.reads().collect::<Vec<_>>(), q.rows.len()), (vec![("r", 99)], 100_000));
    assert!(view.get(&FactKey { rel: "g", ident: &ID, part: 4 }).is_none(), "gone");
    assert_eq!(files(&path).len(), 3, "{:?}", files(&path));
    // Small writes merge into the newest small file: the chain does not grow a file a write.
    let before = read(&path, &c);
    for k in 1..=64 {
        write(&path, &c, vec![put("s", k, 100 + k as u64, flat(&[k, k]))]);
        let n = chain_rows(&c, &SPACE).unwrap().len();
        assert!(n <= 2, "{n} files after {k} small writes");
    }
    let after = read(&path, &c);
    assert_eq!(after.iter().filter(|e| e.0 != "s").cloned().collect::<Vec<_>>(), before);
    assert_eq!(after.iter().filter(|e| e.0 == "s").count(), 64);
    // A write weighing as much as the chain merges every file: one file, every live entry read the same.
    let big2: Vec<i64> = big.iter().map(|v| v + 1).collect();
    write(&path, &c, vec![put("q", 0, 7, flat(&big2))]);
    assert_eq!(chain_rows(&c, &SPACE).unwrap().len(), 1, "merged into one");
    let merged = read(&path, &c);
    assert_eq!(merged.iter().filter(|e| e.0 != "q").cloned().collect::<Vec<_>>(), after.iter().filter(|e| e.0 != "q").cloned().collect::<Vec<_>>());
    assert_eq!(merged.iter().find(|e| e.0 == "q").unwrap().2, 7);
    assert_eq!(first.view().all().count(), 3, "a reader of the old chain reads it still");
    write(&path, &c, vec![put("t", 0, 6, flat(&[1, 1]))]);
    assert_eq!(chain_rows(&c, &SPACE).unwrap().len(), 2, "a small write after a big file is appended");
    assert_eq!(files(&path).len(), 2, "the merged files swept: {:?}", files(&path));
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// K-R6: a write whose transaction rolls back leaves the space as it was; its file goes at the next write.
#[test]
fn a_rolled_back_write_is_not_the_spaces() {
    let (path, c) = store("rollback");
    write(&path, &c, vec![put("p", 0, 1, flat(&[1, 2]))]);
    let rows = chain_rows(&c, &SPACE).unwrap();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    write_space(&path, &c, &SPACE, vec![put("p", 0, 2, flat(&[3, 4]))], None).unwrap();
    c.execute_batch("ROLLBACK").unwrap();
    assert_eq!(chain_rows(&c, &SPACE).unwrap(), rows);
    assert_eq!(read(&path, &c)[0].2, 1);
    assert_eq!(files(&path).len(), 2, "the rolled-back write's file is there until the next write");
    write(&path, &c, vec![put("q", 0, 3, flat(&[5, 6]))]);
    assert_eq!(files(&path).len(), 2, "{:?}", files(&path));
    assert_eq!(read(&path, &c).iter().map(|(r, _, d, _)| (r.as_str(), *d)).collect::<Vec<_>>(), [("p", 1), ("q", 3)]);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// K-R8a (decision 0402 step 5): a run binds a relation whose frame its space's chain keeps (the same digest and
/// types) from the chain: its rows in canonical order, its frame's digest and reads, and its levels as the entry has
/// them, in its rows' order (not the frame's levels part, in the frame's order); a relation whose entry is stale or
/// missing (a store from before the chain) from its frame.
#[test]
fn a_kept_relation_is_bound_from_its_chain() {
    use crate::content_store::{FRAME_LEVELS_PART, FramePut, FrameRel, FrameRows, FrameStored};
    let (path, c) = store("bind");
    let rel = |n: &str| FrameRel { name: n.into(), ident: ID };
    let (p, q, s) = (rel("p"), rel("q"), rel("s"));
    let frame = |rel: &FrameRel, part: i64, types: &[u8], v: &[i64]| FramePut { rel: rel.clone(), part, digest: 1, reads: vec![("r".into(), 1)], rows: FrameRows { types: types.to_vec(), rows: v.to_vec(), n: v.len() / types.len() } };
    {
        let cs = crate::content_store::ContentStore::open(&path).unwrap();
        let puts = [frame(&p, 0, b"ii", &[3, 30, 1, 10]), frame(&p, FRAME_LEVELS_PART, b"i", &[5, 6]), frame(&q, 0, b"ii", &[5, 50]), frame(&s, 0, b"ii", &[7, 70])];
        cs.keep_frames(&SPACE, &puts, &[], None).unwrap();
    }
    // p's entry is its frame's, with its levels; q's is stale (another digest); s has none.
    let mut with_levels = put("p", 0, 1, flat(&[3, 30, 1, 10]));
    if let KeptWhat::Flat { levels, .. } = &mut with_levels.what {
        *levels = Some(vec![5, 6]);
    }
    write(&path, &c, vec![with_levels, put("q", 0, 2, flat(&[5, 50]))]);
    let cs = crate::content_store::ContentStore::open(&path).unwrap();
    cs.preload_frames().unwrap();
    let got = cs.admit_frame_parts(&SPACE, &[(p.clone(), 0), (p.clone(), FRAME_LEVELS_PART), (q.clone(), 0), (s.clone(), 0)]).unwrap();
    let (rows, levels) = (&got[&(p.clone(), 0)], &got[&(p.clone(), FRAME_LEVELS_PART)]);
    assert!(rows.from_chain() && levels.from_chain(), "a kept relation and its levels are read from the chain");
    assert_eq!(rows.stored, FrameStored { digest: 1, reads: vec![("r".into(), 1)] });
    assert_eq!((rows.rows().unwrap().rows, rows.len()), (vec![1, 10, 3, 30], Some(2)));
    assert_eq!(levels.rows().unwrap().rows, vec![6, 5], "the levels follow the entry's rows");
    assert!(!got[&(q.clone(), 0)].from_chain() && !got[&(s.clone(), 0)].from_chain(), "a stale or missing entry: the frame");
    assert_eq!(got[&(q.clone(), 0)].rows().unwrap().rows, vec![5, 50]);
    crate::content_store::forget_frames_snapshot(path.to_str().unwrap());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// K-R7 (decision 0402 step 4), over a real store (`RERAIL_KEPT_STORE`, a copy no run is writing): every kept entry's
/// rows equal its frame's as sets, and the chain's bytes against the frames' segments, with what each encoding holds.
/// Run with `--ignored`; prints its report.
#[test]
#[ignore = "reads the store RERAIL_KEPT_STORE names"]
fn kept_entries_equal_their_frames() {
    let path = PathBuf::from(std::env::var("RERAIL_KEPT_STORE").expect("RERAIL_KEPT_STORE: a store's file"));
    let store = crate::content_store::ContentStore::open(&path).unwrap();
    let c = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let spaces: Vec<Vec<u8>> = c.prepare("SELECT DISTINCT space FROM facts_chain").unwrap().query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
    let (mut entries, mut rows, mut differ, mut no_frame, mut chain_bytes, mut files) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    let mut by_tag: std::collections::BTreeMap<&'static str, (usize, usize, usize)> = Default::default();
    let mut levels_bytes = 0usize;
    for s in &spaces {
        let space: [u8; 32] = s.as_slice().try_into().unwrap();
        let kept = KeptSpace::open(&path, &space, &chain_rows(&c, &space).unwrap()).unwrap();
        files += kept.files.len();
        chain_bytes += kept.view().bytes();
        for k in kept.view().all() {
            entries += 1;
            rows += k.rows.len();
            for col in 0..k.rows.arity as usize {
                let ic = k.rows.col(col);
                let e = by_tag.entry(ic.tag_name()).or_default();
                (e.0, e.1, e.2) = (e.0 + 1, e.1 + ic.len(), e.2 + ic.bytes());
            }
            levels_bytes += k.rows.levels().map_or(0, |l| l.bytes());
            let rel = crate::content_store::FrameRel { name: k.key.rel.to_owned(), ident: *k.key.ident };
            let Some(frame) = store.frame_rows(&space, &rel, k.key.part).unwrap() else {
                no_frame += 1;
                continue;
            };
            let w = frame.types.len();
            // A nullary relation's rows are its count: one row (true) or none.
            let mut want: Vec<Vec<i64>> = if w == 0 { vec![Vec::new(); frame.n.min(1)] } else { frame.rows.chunks_exact(w).map(<[i64]>::to_vec).collect() };
            want.sort();
            want.dedup();
            let mut got: Vec<Vec<i64>> = (0..k.rows.len()).map(|i| k.row(i)).collect();
            got.sort();
            if got != want {
                differ += 1;
                if differ <= 5 {
                    eprintln!("differs: {} part {}: kept {} rows, frame {}", k.key.rel, k.key.part, got.len(), want.len());
                }
            }
        }
    }
    // Every frame of a space with a chain has its kept entry (a delta frame, part < 0, is its relation's part 0's).
    let mut no_kept = 0usize;
    for s in &spaces {
        let space: [u8; 32] = s.as_slice().try_into().unwrap();
        let kept = KeptSpace::open(&path, &space, &chain_rows(&c, &space).unwrap()).unwrap();
        let view = kept.view();
        let mut st = c.prepare("SELECT rel, ident, part FROM frames WHERE space = ?1 AND part >= 0 AND rel NOT IN ('local', 'local_out')").unwrap();
        let keys: Vec<(String, Vec<u8>, i64)> = st.query_map([&space[..]], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().collect::<Result<_, _>>().unwrap();
        for (rel, ident, part) in keys {
            let ident: [u8; 32] = ident.as_slice().try_into().unwrap();
            if view.get(&FactKey { rel: &rel, ident: &ident, part }).is_none() {
                no_kept += 1;
                if no_kept <= 5 {
                    eprintln!("no kept entry: space {:02x}{:02x}{:02x}{:02x} {rel} part {part}", space[0], space[1], space[2], space[3]);
                }
            }
        }
    }
    let (held, named) = store.frame_segment_bytes().unwrap();
    eprintln!("spaces {}  files {files}  entries {entries}  rows {rows}", spaces.len());
    eprintln!("chain {chain_bytes} bytes; frames' segments {held} bytes ({named} named by rows)");
    for (tag, (cols, values, bytes)) in &by_tag {
        eprintln!("  {tag:6} {cols:7} columns {values:11} values {bytes:11} bytes  {:.2} bits a value", *bytes as f64 * 8.0 / (*values).max(1) as f64);
    }
    eprintln!("  levels {levels_bytes} bytes");
    eprintln!("differ {differ}  with no frame {no_frame}  frames with no kept entry {no_kept}");
    assert_eq!((differ, no_kept), (0, 0));
}
