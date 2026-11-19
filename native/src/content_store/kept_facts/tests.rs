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
    KeptWhat::Flat { widths: vec![8, 8], rows: rows.to_vec(), n: rows.len() / 2 }
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

/// The space as a new reader opens it: each entry's (rel, part, digest, rows).
fn read(path: &Path, c: &Connection) -> Vec<(String, i64, u64, Vec<Vec<i64>>)> {
    let row = space_row(c, &SPACE).unwrap().unwrap();
    let space = KeptSpace::open(path, &SPACE, &row).unwrap();
    space
        .view()
        .all()
        .map(|k| {
            let v = k.view();
            // In id order: a frozen relation's ids are its canonical order, and a granule's rows are as they came.
            (k.key.rel.to_owned(), k.key.part, k.digest, (0..v.total()).map(|id| v.row(id)).collect())
        })
        .collect()
}

fn files(path: &Path) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(space_dir(path, &SPACE)).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    out.sort();
    out
}

/// F2: the first write is a base; a smaller next one a delta over it, whose entries replace the base's and whose gone
/// entries hide them; the files a commit replaced go at the next write; a delta that outgrows its base is rolled into
/// a new one. A reader of the old files reads them still.
#[test]
fn a_space_keeps_a_base_and_one_delta() {
    let (path, c) = store("space");
    let big: Vec<i64> = (0..20_000).flat_map(|i| [i, i * 2]).collect();
    write(&path, &c, vec![put("p", 0, 1, KeptWhat::Rows(lent(&[(3, 30), (1, 10)]))), put("q", 0, 2, flat(&big)), put("g", 4, 3, flat(&[5, 6]))]);
    let row = space_row(&c, &SPACE).unwrap().unwrap();
    assert!(row.base.is_some() && row.delta.is_none(), "the first write is a base");
    assert_eq!(read(&path, &c)[1], ("p".into(), 0, 1, vec![vec![1, 10], vec![3, 30]]), "a relation's rows in its canonical order");
    // A reader of the base as it is now.
    let first = KeptSpace::open(&path, &SPACE, &row).unwrap();
    write(&path, &c, vec![put("p", 0, 11, flat(&[9, 9])), put("g", 4, 0, KeptWhat::Gone)]);
    let row2 = space_row(&c, &SPACE).unwrap().unwrap();
    assert_eq!(row2.base, row.base, "the base stays");
    assert!(row2.delta.is_some());
    let got = read(&path, &c);
    assert_eq!(got.iter().map(|(r, p, d, _)| (r.as_str(), *p, *d)).collect::<Vec<_>>(), [("p", 0, 11), ("q", 0, 2)]);
    // Reads only: the rows as they were, with new reads.
    let mut retag = put("q", 0, 2, KeptWhat::Reads);
    retag.reads = vec![("r".into(), 99)];
    write(&path, &c, vec![retag]);
    let row3 = space_row(&c, &SPACE).unwrap().unwrap();
    let space = KeptSpace::open(&path, &SPACE, &row3).unwrap();
    let view = space.view();
    let q = view.get(&FactKey { rel: "q", ident: &ID, part: 0 }).unwrap();
    assert_eq!((q.reads().collect::<Vec<_>>(), q.view().live_len()), (vec![("r", 99)], 20_000));
    assert!(view.get(&FactKey { rel: "g", ident: &ID, part: 4 }).is_none(), "a gone entry carried in the next delta");
    // The delta this write replaced stays until the next write (its commit could still have rolled back).
    assert_eq!(files(&path).len(), 3, "{:?}", files(&path));
    assert_eq!(first.view().all().count(), 3, "a reader of the old base reads it still");
    // A delta that would hold more than its base: rolled into a new one.
    let more: Vec<i64> = (0..60_000).flat_map(|i| [i, -i]).collect();
    write(&path, &c, vec![put("r", 0, 5, flat(&more))]);
    let row4 = space_row(&c, &SPACE).unwrap().unwrap();
    assert!(row4.delta.is_none() && row4.base != row.base, "rolled up: {row4:?}");
    let got = read(&path, &c);
    assert_eq!(got.iter().map(|(r, p, d, _)| (r.as_str(), *p, *d)).collect::<Vec<_>>(), [("p", 0, 11), ("q", 0, 2), ("r", 0, 5)]);
    write(&path, &c, vec![put("s", 0, 6, flat(&[1, 1]))]);
    assert_eq!(files(&path).len(), 2, "the roll-up's predecessors swept: {:?}", files(&path));
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// A write whose transaction rolls back leaves the space as it was; its file goes at the next write.
#[test]
fn a_rolled_back_write_is_not_the_spaces() {
    let (path, c) = store("rollback");
    write(&path, &c, vec![put("p", 0, 1, flat(&[1, 2]))]);
    let row = space_row(&c, &SPACE).unwrap();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    write_space(&path, &c, &SPACE, vec![put("p", 0, 2, flat(&[3, 4]))], None).unwrap();
    c.execute_batch("ROLLBACK").unwrap();
    assert_eq!(space_row(&c, &SPACE).unwrap(), row);
    assert_eq!(read(&path, &c)[0].2, 1);
    assert_eq!(files(&path).len(), 2, "the rolled-back write's file is there until the next write");
    write(&path, &c, vec![put("q", 0, 3, flat(&[5, 6]))]);
    assert_eq!(files(&path).len(), 2, "{:?}", files(&path));
    assert_eq!(read(&path, &c).iter().map(|(r, _, d, _)| (r.as_str(), *d)).collect::<Vec<_>>(), [("p", 1), ("q", 3)]);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// A wave's puts are gathered by space: one block a space, whatever the callers that put them (each input's job), a
/// key put twice keeping its last put.
#[test]
fn a_waves_kept_puts_are_one_block_a_space_and_a_key_keeps_its_last() {
    use crate::content_store::write_back::Gather as _;
    const OTHER: [u8; 32] = [9; 32];
    let mut g = KeptGather::default();
    g.add(SPACE, vec![put("a", 0, 1, flat(&[1, 2]))]);
    g.add(OTHER, vec![put("c", 0, 5, flat(&[7, 7]))]);
    g.add(SPACE, vec![put("b", 0, 2, flat(&[3, 4])), put("a", 0, 3, flat(&[5, 6]))]);
    assert_eq!(g.bytes(), 4 * 2 * 8 + 4 * 64);
    let closed = Box::new(g).close().expect("a job");
    let super::super::write_back::Work(super::super::write_back::Kind::Kept(w)) = closed.job else { panic!("a kept write") };
    let spaces: Vec<([u8; 32], Vec<(String, u64)>)> = w.spaces.iter().map(|(s, puts)| (*s, puts.iter().map(|p| (p.rel.clone(), p.digest)).collect())).collect();
    assert_eq!(spaces, vec![(SPACE, vec![("b".into(), 2), ("a".into(), 3)]), (OTHER, vec![("c".into(), 5)])]);
    // Written, the space reads back the last put of each key.
    let (path, c) = store("gathered");
    let mut w = w;
    let puts = w.spaces.remove(0).1;
    write(&path, &c, puts);
    let mut got: Vec<(String, u64)> = read(&path, &c).into_iter().map(|(r, _, d, _)| (r, d)).collect();
    got.sort();
    assert_eq!(got, vec![("a".into(), 3), ("b".into(), 2)]);
}
