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
    KeptWhat::Flat { widths: vec![8, 8], rows: rows.to_vec(), n: rows.len() / 2, levels: None, given: false }
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

/// K-R5 (decision 0456): a write appends one file of its puts only (a relation's live rows packed, a granule's
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
    // Reads only: the rows as they were, their digest the entry's, with new reads. A retag's put carries no digest
    // (`kept_reads`); one taken from it left ep2's retagged relations unbound.
    let mut retag = put("q", 0, 0, KeptWhat::Reads);
    retag.reads = vec![("r".into(), 99)];
    write(&path, &c, vec![retag]);
    let space = KeptSpace::open(&path, &SPACE, &chain_rows(&c, &SPACE).unwrap()).unwrap();
    let view = space.view();
    let q = view.get(&FactKey { rel: "q", ident: &ID, part: 0 }).unwrap();
    assert_eq!((q.reads().collect::<Vec<_>>(), q.rows.len(), q.digest), (vec![("r", 99)], 100_000, 2));
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

/// A wave's kept puts, queued through the store (`queue_kept`) under a run's waves, are gathered and written as one
/// file per space when the wave is sealed: queued as a job each, every input's puts wrote a write of its own (O125
/// step 05: 112 a run; t43 step 16). A key put twice in the wave is written once, its later put what the space keeps.
#[test]
fn a_waves_kept_puts_are_one_block_per_space() {
    let (path, _) = store("wave-queue");
    let store = crate::content_store::ContentStore::open(&path).unwrap();
    store.queue_kept(SPACE, vec![put("base", 0, 1, flat(&(0..40_000).collect::<Vec<i64>>()))]);
    store.written().unwrap();
    let before = files(&path);
    {
        let _waves = crate::content_store::write_back::run_waves();
        for (i, rel) in ["a", "b", "c", "d"].into_iter().enumerate() {
            store.queue_kept(SPACE, vec![put(rel, 0, 10 + i as u64, flat(&[i as i64, 1]))]);
        }
        store.queue_kept(SPACE, vec![put("a", 0, 20, flat(&[7, 7]))]);
    }
    store.written().unwrap();
    let c = Connection::open(&path).unwrap();
    let got: Vec<(String, u64, Vec<Vec<i64>>)> = read(&path, &c).into_iter().map(|(r, _, d, rows)| (r, d, rows)).filter(|(r, _, _)| r != "base").collect();
    assert_eq!(got, [("a".into(), 20, vec![vec![7, 7]]), ("b".into(), 11, vec![vec![1, 1]]), ("c".into(), 12, vec![vec![2, 1]]), ("d".into(), 13, vec![vec![3, 1]])]);
    // The wave's five puts in one appended file of the chain (decision 0456), where five jobs wrote five.
    let made = files(&path).into_iter().filter(|f| !before.contains(f)).count();
    assert_eq!(made, 1, "one file made in the wave: {:?} after {before:?}", files(&path));
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// A wave's puts are gathered by space and appended as one file each: a later put of a key replaces an earlier one, a
/// retag keeps the rows the wave gave its key with its own reads, and a gone entry stays gone. Two spaces, one
/// write each.
#[test]
fn a_waves_puts_are_one_file_a_space() {
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
    let space = KeptSpace::open(&path, &SPACE, &chain_rows(&c, &SPACE).unwrap()).unwrap();
    let q = space.view().get(&FactKey { rel: "q", ident: &ID, part: 0 }).map(|k| k.reads().map(|(r, d)| (r.to_owned(), d)).collect::<Vec<_>>());
    assert_eq!(q, Some(vec![("r".to_owned(), 77)]));
    assert_eq!(files(&path).len(), 1, "one file for the space's wave: {:?}", files(&path));
    assert_eq!(chain_rows(&c, &OTHER).unwrap().len(), 1, "the other space written in the same job");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// F-R5 (decisions 0401 F5, 0439): the kept relations read through SQL are their entries. `kept_entries` gives each
/// entry's key, digest, types, reads and rows' count; `kept_facts` each entry's rows, a row the run killed not kept;
/// an entry not kept has no rows.
#[test]
fn the_kept_relations_read_through_sql_equal_their_entries() {
    let (path, c) = store("entries");
    write(&path, &c, vec![put("q", 0, 2, KeptWhat::Rows(lent_dead(&[(1, 10), (2, 20), (3, 30)], &[1]))), put("p", 4, 3, flat(&[7, 70, 8, 80]))]);
    crate::content_store::atoms::Atoms::attach(&c).unwrap();
    let space = KeptSpace::open(&path, &SPACE, &chain_rows(&c, &SPACE).unwrap()).unwrap();
    let view = space.view();
    // An entry's relation, part, digest, row count and rows.
    type Entry = (String, i64, i64, i64, Vec<Vec<i64>>);
    let entries_of: Vec<Entry> = view.all().map(|k| (k.key.rel.to_owned(), k.key.part, k.digest as i64, k.rows.len() as i64, (0..k.rows.len()).map(|i| k.row(i)).collect())).collect();
    let entries: Vec<(String, i64, i64, i64, String, String)> = c
        .prepare("SELECT rel, part, digest, rows, types, reads FROM kept_entries WHERE space = ?1 ORDER BY rel, part")
        .unwrap()
        .query_map([SPACE.as_slice()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(entries.iter().map(|e| (e.0.clone(), e.1, e.2, e.3)).collect::<Vec<_>>(), entries_of.iter().map(|v| (v.0.clone(), v.1, v.2, v.3)).collect::<Vec<_>>());
    assert_eq!((entries[1].4.as_str(), entries[1].5.as_str()), ("ii", "r:0000000000000002"), "q's types and reads");
    for (rel, part, _, _, rows) in &entries_of {
        let sql: Vec<Vec<i64>> = c
            .prepare("SELECT c0, c1 FROM kept_facts WHERE space = ?1 AND rel = ?2 AND ident = ?3 AND part = ?4")
            .unwrap()
            .query_map(rusqlite::params![SPACE.as_slice(), rel, ID.as_slice(), part], |r| Ok(vec![r.get(0)?, r.get(1)?]))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(&sql, rows, "{rel}'s rows through SQL");
    }
    assert_eq!(entries_of[1].4, [vec![1, 10], vec![3, 30]], "the killed row is not kept");
    let none: i64 = c.query_row("SELECT count(*) FROM kept_facts WHERE space = ?1 AND rel = 'nothing'", [SPACE.as_slice()], |r| r.get(0)).unwrap();
    assert_eq!(none, 0, "an entry not kept has no rows");
    let all: i64 = c.query_row("SELECT count(*) FROM kept_entries", [], |r| r.get(0)).unwrap();
    assert_eq!(all, 2, "every space's entries, unconstrained");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// K-R8a's and K-R9's store halves (decision 0456 step 5-6): an entry's levels follow its rows into canonical order;
/// an entry put as given (an input's granule, read by position) is read in its order, every row. (Their frames
/// halves went with the frames, decision 0439.)
#[test]
fn levels_follow_their_rows_and_an_entry_put_as_given_keeps_its_order() {
    let (path, c) = store("levels-given");
    let mut with_levels = put("p", 0, 1, flat(&[3, 30, 1, 10]));
    if let KeptWhat::Flat { levels, .. } = &mut with_levels.what {
        *levels = Some(vec![5, 6]);
    }
    let granule_rows = vec![9, 2, 8, 1, 9, 2];
    write(&path, &c, vec![with_levels, put("g", 0, 0, KeptWhat::Flat { widths: vec![8, 8], rows: granule_rows.clone(), n: 3, levels: None, given: true })]);
    let space = KeptSpace::open(&path, &SPACE, &chain_rows(&c, &SPACE).unwrap()).unwrap();
    let view = space.view();
    let p = view.get(&FactKey { rel: "p", ident: &ID, part: 0 }).unwrap();
    assert_eq!((0..p.rows.len()).map(|i| p.row(i)).collect::<Vec<_>>(), [vec![1, 10], vec![3, 30]]);
    let levels = p.rows.levels().expect("its levels");
    assert_eq!((0..2).map(|i| levels.get(i)).collect::<Vec<_>>(), [6, 5], "the levels follow the entry's rows");
    let g = view.get(&FactKey { rel: "g", ident: &ID, part: 0 }).unwrap();
    assert_eq!((0..g.rows.len()).flat_map(|i| g.row(i)).collect::<Vec<_>>(), granule_rows, "a granule in its order, every row");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// K-R10 (decision 0456 step 6, 0401 F5): the kept facts are a virtual table: a space's relation's rows by `rel` and
/// `part`, newest entry only, a gone one absent; `c0 = ?` a range of the entry; columns past the arity NULL.
#[test]
fn kept_facts_are_a_virtual_table() {
    let (path, c) = store("vtab");
    write(&path, &c, vec![put("p", 0, 1, flat(&[3, 30, 1, 10, 3, 31])), put("q", 0, 2, flat(&[5, 50])), put("g", 0, 3, flat(&[7, 70]))]);
    write(&path, &c, vec![put("q", 0, 4, flat(&[6, 60])), put("g", 0, 0, KeptWhat::Gone)]);
    crate::content_store::atoms::Atoms::attach(&c).unwrap();
    let rows = |sql: &str| -> Vec<Vec<Option<i64>>> {
        let mut st = c.prepare(sql).unwrap();
        st.query_map([&SPACE[..]], |r| (0..3).map(|i| r.get::<_, Option<i64>>(i)).collect()).unwrap().map(Result::unwrap).collect()
    };
    assert_eq!(rows("SELECT c0, c1, c2 FROM kept_facts WHERE space = ?1 AND rel = 'p' AND part = 0"), [[Some(1), Some(10), None], [Some(3), Some(30), None], [Some(3), Some(31), None]]);
    assert_eq!(rows("SELECT c0, c1, part FROM kept_facts WHERE space = ?1 AND rel = 'q'"), [[Some(6), Some(60), Some(0)]], "the newest entry");
    assert!(rows("SELECT c0, c1, part FROM kept_facts WHERE space = ?1 AND rel = 'g'").is_empty(), "gone");
    assert_eq!(rows("SELECT c0, c1, level FROM kept_facts WHERE space = ?1 AND rel = 'p' AND c0 = 3"), [[Some(3), Some(30), None], [Some(3), Some(31), None]]);
    assert_eq!(rows("SELECT c0, c1, id FROM kept_facts WHERE space = ?1 AND rel = 'p' AND c0 = 3"), [[Some(3), Some(30), Some(1)], [Some(3), Some(31), Some(2)]], "each row's place in its entry, where `c0 = ?` narrows it");
    let n: i64 = c.query_row("SELECT count(*) FROM kept_facts", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 4, "every space, every live entry");
    // `c0 = ?` is a range because the entry records its canonical order (an entry put as given records none).
    let space = KeptSpace::open(&path, &SPACE, &chain_rows(&c, &SPACE).unwrap()).unwrap();
    let view = space.view();
    let p = view.all().find(|k| k.key.rel == "p").unwrap();
    assert!(p.rows.leads(0) && !p.rows.leads(1), "a canonical entry leads by its first column");
    write(&path, &c, vec![put("r", 0, 5, KeptWhat::Flat { widths: vec![8, 8], rows: vec![2, 0, 1, 0], n: 2, levels: None, given: true })]);
    let space = KeptSpace::open(&path, &SPACE, &chain_rows(&c, &SPACE).unwrap()).unwrap();
    let view = space.view();
    let r = view.all().find(|k| k.key.rel == "r").unwrap();
    assert!(!r.rows.leads(0), "an entry put as given records no order");
    assert_eq!(rows("SELECT c0, c1, part FROM kept_facts WHERE space = ?1 AND rel = 'r' AND c0 = 1"), [[Some(1), Some(0), Some(0)]], "scanned, not a range");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

/// Decision 0456's open question, over a real store (`RERAIL_KEPT_STORE`): what each column of the chain would take
/// under the candidate encodings. Leading columns and the rest apart; GROUPS' group widths; sample values. Run with
/// `--ignored --nocapture`; prints its report.
#[test]
#[ignore = "reads the store RERAIL_KEPT_STORE names"]
fn chain_bytes_by_relation() {
    let path = PathBuf::from(std::env::var("RERAIL_KEPT_STORE").expect("RERAIL_KEPT_STORE: a store's file"));
    let rows = by_relation(&path).unwrap();
    let total: usize = rows.iter().map(|(_, (_, _, b))| b).sum();
    let by_what = |w: &str| rows.iter().filter(|((_, x), _)| *x == w).map(|(_, (_, _, b))| b).sum::<usize>();
    eprintln!("{total} bytes of values: rows {}, granules {}, beside {}", by_what("rows"), by_what("granule"), by_what("beside"));
    for ((rel, what), (n, r, b)) in rows.iter().take(40) {
        eprintln!("{b:>11} {:5.1}%  {what:7} {n:6} entries {r:10} rows  {rel}", *b as f64 * 100.0 / total.max(1) as f64);
    }
}
