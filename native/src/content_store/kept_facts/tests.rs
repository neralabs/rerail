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
    lent_dead(rows, &[])
}

/// `lent`, its rows `dead` killed by the run (the entry's dead bits).
fn lent_dead(rows: &[(i64, i64)], dead: &[u32]) -> Box<dyn KeptRows> {
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
    let mut bits = vec![0u64; rows.len().div_ceil(64)];
    for id in dead {
        bits[*id as usize / 64] |= 1 << (id % 64);
    }
    Box::new(Lent { block, at, dead: if dead.is_empty() { Vec::new() } else { bits } })
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

/// A wave's puts are gathered by space and written as one block each: a later put of a key replaces an earlier one, a
/// retag keeps the rows the wave gave its key with its own reads, and a gone entry stays gone. Two spaces, one
/// write each.
#[test]
fn a_waves_puts_are_one_block_a_space() {
    let (path, c) = store("gather");
    const OTHER: [u8; 32] = [9; 32];
    let mut g = KeptGather::default();
    g.add(SPACE, vec![put("p", 0, 1, flat(&[1, 1])), put("q", 0, 2, flat(&[2, 2])), put("g", 0, 3, flat(&[3, 3]))]);
    g.add(OTHER, vec![put("o", 0, 4, flat(&[4, 4]))]);
    // `p` replaced; `q` retagged (its rows and digest this wave's, its reads the retag's: a retag carries no digest,
    // `retain::kept_reads`); `g` gone.
    let mut retag = put("q", 0, 0, KeptWhat::Reads);
    retag.reads = vec![("r".into(), 77)];
    g.add(SPACE, vec![put("p", 0, 11, flat(&[5, 5, 6, 6])), retag, put("g", 0, 0, KeptWhat::Gone)]);
    assert_eq!(g.spaces[&SPACE].0.len(), 3, "each key once");
    let spaces: Vec<([u8; 32], Vec<KeptPut>)> = g.spaces.into_iter().map(|(s, (p, _))| (s, p)).collect();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    for (space, puts) in spaces {
        write_space(&path, &c, &space, puts, None).unwrap();
    }
    c.execute_batch("COMMIT").unwrap();
    let got = read(&path, &c);
    assert_eq!(
        got.iter().map(|(r, p, d, rows)| (r.as_str(), *p, *d, rows.clone())).collect::<Vec<_>>(),
        [("p", 0, 11, vec![vec![5, 5], vec![6, 6]]), ("q", 0, 2, vec![vec![2, 2]])],
        "p replaced, q retagged with this wave's rows and digest, g gone"
    );
    let row = space_row(&c, &SPACE).unwrap().unwrap();
    let space = KeptSpace::open(&path, &SPACE, &row).unwrap();
    let q = space.view().get(&FactKey { rel: "q", ident: &ID, part: 0 }).map(|k| k.reads().map(|(r, d)| (r.to_owned(), d)).collect::<Vec<_>>());
    assert_eq!(q, Some(vec![("r".to_owned(), 77)]));
    assert_eq!(files(&path).len(), 1, "one block for the space's wave: {:?}", files(&path));
    assert!(space_row(&c, &OTHER).unwrap().is_some(), "the other space written in the same job");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// F-R5 (decision 0401 F5): the kept relations read through SQL are their views. `kept_entries` gives each entry's
/// key, digest, types, reads and live rows' count; `kept_rows` each live row's values where they lie, a row the run
/// killed left out, a column past the entry's width NULL; an entry not kept, none.
#[test]
fn the_kept_relations_read_through_sql_equal_their_views() {
    let (path, c) = store("vtab");
    write(&path, &c, vec![put("q", 0, 2, KeptWhat::Rows(lent_dead(&[(1, 10), (2, 20), (3, 30)], &[1]))), put("p", 4, 3, flat(&[7, 70, 8, 80]))]);
    crate::content_store::atoms::Atoms::attach(&c).unwrap();
    let row = space_row(&c, &SPACE).unwrap().unwrap();
    let space = KeptSpace::open(&path, &SPACE, &row).unwrap();
    // The views, as a run binds them: each entry's live rows (neither dead in the entry nor in its relation).
    let view = space.view();
    let views: Vec<(String, i64, i64, i64, Vec<Vec<i64>>)> = view
        .all()
        .map(|k| {
            let v = k.view();
            let rows: Vec<Vec<i64>> = (0..v.total()).filter(|&id| v.live(id) && k.dead.get(id as usize / 64).is_none_or(|w| w & (1 << (id % 64)) == 0)).map(|id| v.row(id)).collect();
            (k.key.rel.to_owned(), k.key.part, k.digest as i64, rows.len() as i64, rows)
        })
        .collect();
    let entries: Vec<(String, i64, i64, i64, String, String)> = c
        .prepare("SELECT rel, part, digest, rows, types, reads FROM kept_entries WHERE space = ?1 ORDER BY rel, part")
        .unwrap()
        .query_map([SPACE.as_slice()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(entries.iter().map(|e| (e.0.clone(), e.1, e.2, e.3)).collect::<Vec<_>>(), views.iter().map(|v| (v.0.clone(), v.1, v.2, v.3)).collect::<Vec<_>>());
    assert_eq!((entries[1].4.as_str(), entries[1].5.as_str()), ("ii", "r:0000000000000002"), "q's types and reads");
    for (rel, part, _, _, rows) in &views {
        let sql: Vec<Vec<i64>> = c
            .prepare("SELECT c0, c1 FROM kept_rows(?1, ?2, ?3, ?4)")
            .unwrap()
            .query_map(rusqlite::params![SPACE.as_slice(), rel, ID.as_slice(), part], |r| Ok(vec![r.get(0)?, r.get(1)?]))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(&sql, rows, "{rel}'s rows through SQL");
    }
    assert_eq!(views[1].4, [vec![1, 10], vec![3, 30]], "the killed row is left out");
    let past: Option<i64> = c.query_row("SELECT c2 FROM kept_rows(?1, 'q', ?2, 0)", rusqlite::params![SPACE.as_slice(), ID.as_slice()], |r| r.get(0)).unwrap();
    assert_eq!(past, None, "a column past the entry's width");
    let none: i64 = c.query_row("SELECT count(*) FROM kept_rows(?1, 'nothing', ?2, 0)", rusqlite::params![SPACE.as_slice(), ID.as_slice()], |r| r.get(0)).unwrap();
    assert_eq!(none, 0, "an entry not kept has no rows");
    let all: i64 = c.query_row("SELECT count(*) FROM kept_entries", [], |r| r.get(0)).unwrap();
    assert_eq!(all, 2, "every space's entries, unconstrained");
}
