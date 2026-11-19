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

/// K-R5 (decision 0442): a write appends one file of its puts only (a relation's live rows packed, a granule's
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
    let entries_of: Vec<(String, i64, i64, i64, Vec<Vec<i64>>)> = view.all().map(|k| (k.key.rel.to_owned(), k.key.part, k.digest as i64, k.rows.len() as i64, (0..k.rows.len()).map(|i| k.row(i)).collect())).collect();
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

/// K-R8a's and K-R9's store halves (decision 0442 step 5-6): an entry's levels follow its rows into canonical order;
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

/// K-R10 (decision 0442 step 6, 0401 F5): the kept facts are a virtual table: a space's relation's rows by `rel` and
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

/// Decision 0442's open question, over a real store (`RERAIL_KEPT_STORE`): what each column of the chain would take
/// under the candidate encodings. Leading columns and the rest apart; GROUPS' group widths; sample values. Run with
/// `--ignored --nocapture`; prints its report.
#[test]
#[ignore = "reads the store RERAIL_KEPT_STORE names"]
fn chain_columns_by_candidate() {
    use rerail_blocks::col;
    let path = PathBuf::from(std::env::var("RERAIL_KEPT_STORE").expect("RERAIL_KEPT_STORE: a store's file"));
    let c = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let spaces: Vec<Vec<u8>> = c.prepare("SELECT DISTINCT space FROM facts_chain").unwrap().query_map([], |r| r.get(0)).unwrap().collect::<Result<_, _>>().unwrap();
    let bits = |r: u64| 64 - r.leading_zeros() as usize;
    // A group of 128 on a line of its own: its first value and slope, then each residual at the group's width.
    let gline = |v: &[i64]| -> usize {
        let (lo, hi) = v.iter().fold((i64::MAX, i64::MIN), |(a, b), &x| (a.min(x), b.max(x)));
        let mut data = 0usize;
        let (mut slo, mut shi) = (i64::MAX, i64::MIN);
        for g in v.chunks(128) {
            let n = g.len();
            let slope = if n > 1 { (((g[n - 1] as i128 - g[0] as i128) << 16) / (n as i128 - 1)) as i64 } else { 0 };
            (slo, shi) = (slo.min(slope), shi.max(slope));
            let r: Vec<i128> = g.iter().enumerate().map(|(i, &x)| x as i128 - (g[0] as i128 + ((i as i128 * slope as i128) >> 16))).collect();
            let (a, b) = r.iter().fold((i128::MAX, i128::MIN), |(a, b), &x| (a.min(x), b.max(x)));
            data += n * bits((b - a) as u64);
        }
        let ng = v.len().div_ceil(128);
        let entry = bits((hi as i128 - lo as i128) as u64) + bits((shi as i128 - slo as i128) as u64) + 7 + bits(data as u64) + 16;
        (data + ng * entry).div_ceil(64) * 8 + 16
    };
    // Each value's delta from the one before, in its own bits (a delta varint's floor; not random access).
    let deltas = |v: &[i64]| -> usize { v.windows(2).map(|w| bits((w[1] as i128 - w[0] as i128).unsigned_abs() as u64) + 1).sum::<usize>().div_ceil(8) };
    #[derive(Default)]
    struct Sum { cols: usize, values: usize, now: usize, gline: usize, rel0: usize, split: usize, s18: usize, r18: usize, best: usize, deltas: usize }
    let mut by: std::collections::BTreeMap<String, Sum> = Default::default();
    let mut widths = [0usize; 65];
    let mut by_rel: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    let mut shown = [0usize; 2];
    for s in &spaces {
        let space: [u8; 32] = s.as_slice().try_into().unwrap();
        let kept = KeptSpace::open(&path, &space, &chain_rows(&c, &space).unwrap()).unwrap();
        for k in kept.view().all() {
            let cols: Vec<Vec<i64>> = (0..k.rows.arity as usize).map(|i| { let mut v = Vec::with_capacity(k.rows.len()); k.rows.col(i).for_each(|x| v.push(x)); v }).collect();
            for (i, v) in cols.iter().enumerate() {
                let ic = k.rows.col(i);
                let now = ic.bytes();
                let g = if v.len() > 1 { gline(v) } else { now };
                let rel0 = if i > 0 { col::choose(&v.iter().zip(&cols[0]).map(|(a, b)| a.wrapping_sub(*b)).collect::<Vec<_>>()).bytes() } else { now };
                let split = col::choose(&v.iter().map(|x| x >> 32).collect::<Vec<_>>()).bytes() + col::choose(&v.iter().map(|x| x & 0xffff_ffff).collect::<Vec<_>>()).bytes() + 64;
                let ch = |x: Vec<i64>| col::choose(&x).bytes();
                let s18 = ch(v.iter().map(|x| x >> 18).collect()) + ch(v.iter().map(|x| x & 0x3ffff).collect()) + 64;
                let r18 = if i > 0 { ch(v.iter().zip(&cols[0]).map(|(a, b)| (a >> 18).wrapping_sub(b >> 18)).collect()) + ch(v.iter().zip(&cols[0]).map(|(a, b)| (a & 0x3ffff) - (b & 0x3ffff)).collect()) + 64 } else { s18 };
                let key = format!("{} {}", ic.form(), if k.rows.leads(i) { "lead" } else if i == 0 { "first" } else { "rest" });
                let e = by.entry(key).or_default();
                e.cols += 1;
                e.values += v.len();
                e.now += now;
                e.gline += g.min(now);
                e.rel0 += rel0.min(now);
                e.split += split.min(now);
                e.s18 += s18.min(now);
                e.r18 += r18.min(rel0).min(s18).min(now);
                e.best += now.min(g).min(rel0).min(split).min(s18).min(r18);
                e.deltas += deltas(v);
                let r = by_rel.entry(format!("{} c{i}", k.key.rel)).or_default();
                (r.0, r.1) = (r.0 + v.len(), r.1 + now);
                if ic.form() == "groups" {
                    for gr in v.chunks(128) {
                        let (a, b) = gr.iter().fold((i64::MAX, i64::MIN), |(a, b), &x| (a.min(x), b.max(x)));
                        widths[bits((b as i128 - a as i128) as u64)] += gr.len();
                    }
                    for (gi, gr) in v.chunks(128).enumerate() {
                        let (a, b) = gr.iter().fold((i64::MAX, i64::MIN), |(a, b), &x| (a.min(x), b.max(x)));
                        let w = bits((b as i128 - a as i128) as u64);
                        let slot = if w >= 50 { 1 } else if w >= 30 { 0 } else { continue };
                        if shown[slot] < 6 {
                            shown[slot] += 1;
                            let show: Vec<String> = gr.iter().step_by(8).map(|x| format!("{x:x}")).collect();
                            eprintln!("wide {w} {} c{i} of {} group {gi} ({} rows, leads {}): {}", k.key.rel, k.rows.arity, v.len(), k.rows.leads(i), show.join(" "));
                        }
                        break;
                    }
                }
            }
        }
    }
    eprintln!("{:14} {:>7} {:>11} {:>11} {:>11} {:>11} {:>11} {:>11} {:>11} {:>11} {:>11}", "tag place", "cols", "values", "now", "gline", "rel c0", "split", "s18", "r18", "best", "deltas");
    let mut t = Sum::default();
    for (key, e) in &by {
        eprintln!("{key:14} {:7} {:11} {:11} {:11} {:11} {:11} {:11} {:11} {:11} {:11}   now {:.1} best {:.1} deltas {:.1} bits", e.cols, e.values, e.now, e.gline, e.rel0, e.split, e.s18, e.r18, e.best, e.deltas, e.now as f64 * 8.0 / e.values.max(1) as f64, e.best as f64 * 8.0 / e.values.max(1) as f64, e.deltas as f64 * 8.0 / e.values.max(1) as f64);
        (t.values, t.now, t.gline, t.rel0, t.split, t.s18, t.r18, t.best, t.deltas) = (t.values + e.values, t.now + e.now, t.gline + e.gline, t.rel0 + e.rel0, t.split + e.split, t.s18 + e.s18, t.r18 + e.r18, t.best + e.best, t.deltas + e.deltas);
    }
    eprintln!("all: now {} gline {} rel c0 {} split {} s18 {} r18 (with rel c0, s18) {} best {} deltas {}", t.now, t.gline, t.rel0, t.split, t.s18, t.r18, t.best, t.deltas);
    eprintln!("GROUPS' values by their group's width: {}", widths.iter().enumerate().filter(|x| *x.1 > 0).map(|(w, n)| format!("{w}:{n}")).collect::<Vec<_>>().join(" "));
    let mut top: Vec<_> = by_rel.into_iter().collect();
    top.sort_by_key(|x| std::cmp::Reverse(x.1.1));
    for (name, (values, bytes)) in top.iter().take(25) {
        eprintln!("  {name:40} {values:10} values {bytes:11} bytes {:.1} bits", *bytes as f64 * 8.0 / (*values).max(1) as f64);
    }
}

/// A relation of the run's that continues another (decision 0384 steps 7 and 8, `Relation::try_continue`), lent as a
/// run lends one: its block and its descriptor.
struct LentAt {
    block: Block,
    at: usize,
}

// SAFETY: relations in a block nothing writes while the put is packed.
unsafe impl Send for LentAt {}
unsafe impl Sync for LentAt {}

impl KeptRows for LentAt {
    fn relation(&self) -> &Relation {
        // SAFETY: `at` is the relation's descriptor in `block`, which lives as long as this.
        unsafe { &*self.block.ptr::<Relation>(self.at) }
    }
    fn dead(&self) -> &[u64] {
        &[]
    }
    fn levels(&self) -> &[u32] {
        &[]
    }
}

/// A continuation is kept with every row it reads (decision 0384 steps 7 and 8): its base's below its start, then
/// its own, a fork of a fork read down the chain; not its own rows alone (the base's rows written past its start are
/// not its).
#[test]
fn a_continuation_is_kept_with_every_row_it_reads() {
    let (path, c) = store("continuation");
    let block = Block::anon(kind::FACTS, 16 << 20).with_free_list();
    let spec = Spec { widths: vec![8, 8], order: Vec::new(), indexes: Vec::new(), min_log: 4 };
    let put_rows = |r: &Relation, rows: &[(i64, i64)]| {
        assert!(r.rows.try_reserve(&block, u64::from(r.rows.len()) + rows.len() as u64));
        let at = r.rows.len();
        unsafe {
            r.rows.claim_reserved(rows.len() as u32);
            for (i, &(a, b)) in rows.iter().enumerate() {
                r.rows.put(at + i as u32, 0, a);
                r.rows.put(at + i as u32, 1, b);
            }
        }
    };
    let base = Relation::try_create(&block, &spec, None).unwrap();
    put_rows(base, &[(1, 10), (2, 20)]);
    let fork = Relation::try_continue(&block, &spec, base, 2).expect("a continuation");
    put_rows(fork, &[(3, 30)]);
    put_rows(base, &[(9, 90)]);
    let fork2 = Relation::try_continue(&block, &spec, fork, 3).expect("a fork of a fork");
    put_rows(fork2, &[(4, 40)]);
    let at = block.offset_of(std::ptr::from_ref(fork2));
    write(&path, &c, vec![put("p", 0, 1, KeptWhat::Rows(Box::new(LentAt { block, at })))]);
    assert_eq!(read(&path, &c), [("p".to_owned(), 0, 1, vec![vec![1, 10], vec![2, 20], vec![3, 30], vec![4, 40]])], "every row down the chain, not the base's past its start");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
