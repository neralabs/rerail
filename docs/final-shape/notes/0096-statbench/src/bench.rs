#![allow(dead_code)]
use super::sketch::Sketch;
use rustc_hash::FxHashMap;

fn cpu() -> f64 {
    let mut t = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut t) };
    t.tv_sec as f64 + t.tv_nsec as f64 * 1e-9
}
/// Best of 7 runs of `f`, in seconds of CPU.
fn best(mut f: impl FnMut()) -> f64 {
    (0..7).map(|_| { let t = cpu(); f(); cpu() - t }).fold(f64::INFINITY, f64::min)
}
/// A column: `rows` values over `ndv` distinct, Zipf (s=1) by default; `key`: every value once.
fn column(rows: usize, ndv: u64, key: bool) -> Vec<i64> {
    let mut x = 0x1234_5678u64;
    let mut next = || { x ^= x << 13; x ^= x >> 7; x ^= x << 17; x };
    let h: f64 = (1..=ndv).map(|k| 1.0 / k as f64).sum();
    (0..rows).map(|i| {
        if key { return i as i64 * 7919 + 3 }
        // Inverse of the Zipf CDF, roughly: k ~ exp(u * H).
        let u = (next() >> 11) as f64 / (1u64 << 53) as f64;
        ((u * h).exp().min(ndv as f64) as i64) * 104_729 + 11
    }).collect()
}

pub fn main() {
    let rows = 200_000;
    let edit = 1_000; // one file's rows of the relation: taken out and put back changed
    let cols: Vec<(&str, Vec<i64>)> = vec![("zipf 60k", column(rows, 60_000, false)), ("key", column(rows, 0, true)), ("few 40", column(rows, 40, false))];
    println!("{rows} rows per column, an edit of {edit} rows out and {edit} in; ns per row per column (CPU, best of 7)\n");
    for (name, vals) in &cols {
        let delta_out = &vals[..edit];
        let delta_in: Vec<i64> = delta_out.iter().map(|v| v + 1).collect();
        // 1. The sketch, per row.
        let mut s = Sketch::default();
        for &v in vals { s.add(v, 1) }
        let t_sk = best(|| { for &v in delta_out { s.add(v, -1) } for &v in &delta_in { s.add(v, 1) } for &v in &delta_in { s.add(v, -1) } for &v in delta_out { s.add(v, 1) } }) / (4 * edit) as f64;
        // 2. Exact counts in memory, with ndv and sq kept by the 0↔1 and 2a±1 identities.
        let mut m: FxHashMap<i64, i64> = FxHashMap::default();
        let (mut ndv, mut sq) = (0i64, 0i64);
        let mut upd = |m: &mut FxHashMap<i64, i64>, v: i64, k: i64| {
            let e = m.entry(v).or_insert(0);
            let a = *e; *e += k; let b = *e;
            ndv += i64::from(b > 0) - i64::from(a > 0); sq += b * b - a * a;
            if b == 0 { m.remove(&v); }
        };
        for &v in vals { upd(&mut m, v, 1) }
        let t_ex = best(|| { for &v in delta_out { upd(&mut m, v, -1) } for &v in &delta_in { upd(&mut m, v, 1) } for &v in &delta_in { upd(&mut m, v, -1) } for &v in delta_out { upd(&mut m, v, 1) } }) / (4 * edit) as f64;
        // 3. Today's full measure (syntax::measured): one sort of (value, document) per column, every row.
        let docs: Vec<i64> = (0..rows as i64).map(|i| i / 200).collect();
        let t_full = best(|| {
            let mut p: Vec<(i64, i64)> = vals.iter().copied().zip(docs.iter().copied()).collect();
            p.sort_unstable();
            let (mut n, mut q) = (0i64, 0i64); let mut at = 0;
            while at < p.len() { let v = p[at].0; let e = at + p[at..].partition_point(|x| x.0 == v); n += 1; q += ((e - at) * (e - at)) as i64; at = e; }
            std::hint::black_box((n, q));
        });
        // 4. Estimates read, and the blob made and read.
        let t_est = best(|| { for _ in 0..1000 { std::hint::black_box((s.sq(), s.ndv())); } }) / 1000.0;
        let t_enc = best(|| { for _ in 0..1000 { std::hint::black_box(Sketch::decode(&s.encode())); } }) / 1000.0;
        println!("{name:9}: sketch {:6.1} ns/row   exact in memory {:6.1} ns/row   full re-measure {:7.2} ms ({:5.1} ns/row of the relation)", t_sk * 1e9, t_ex * 1e9, t_full * 1e3, t_full / rows as f64 * 1e9);
        println!("           estimates {:5.2} µs   encode+decode {:5.2} µs ({} B)   exact distinct entries {}  ndv est {:.0} sq est/exact {:.2}", t_est * 1e6, t_enc * 1e6, s.bytes(), m.len(), s.ndv(), s.sq() / sq as f64);
    }
    // 5. SQLite: the writes an edit makes, on disk (WAL, synchronous=NORMAL), per transaction.
    let dir = std::env::args().nth(1).expect("a scratch directory");
    let path = format!("{dir}/statbench.sqlite");
    let _ = std::fs::remove_file(&path);
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
        CREATE TABLE stat_value(rel TEXT, col TEXT, val INTEGER, n INTEGER, PRIMARY KEY(rel, col, val)) WITHOUT ROWID;
        CREATE TABLE stat_total(rel TEXT, col TEXT, v INTEGER, rows INTEGER, sqd INTEGER, ndv INTEGER, sq INTEGER, sketch BLOB, PRIMARY KEY(rel, col)) WITHOUT ROWID;").unwrap();
    let (name, vals) = &cols[0];
    // Exact in SQLite: every distinct value a row, an upsert per changed row.
    {
        let tx = db.unchecked_transaction().unwrap();
        { let mut ins = tx.prepare("INSERT INTO stat_value VALUES('r', 'c', ?1, 1) ON CONFLICT DO UPDATE SET n = n + 1").unwrap(); for &v in vals { ins.execute([v]).unwrap(); } }
        tx.commit().unwrap();
    }
    let delta_out = &vals[..edit];
    let delta_in: Vec<i64> = delta_out.iter().map(|v| v + 1).collect();
    let t_sql = best(|| {
        let tx = db.unchecked_transaction().unwrap();
        {
            let mut up = tx.prepare_cached("INSERT INTO stat_value VALUES('r', 'c', ?1, ?2) ON CONFLICT DO UPDATE SET n = n + excluded.n").unwrap();
            let mut gone = tx.prepare_cached("DELETE FROM stat_value WHERE rel = 'r' AND col = 'c' AND val = ?1 AND n <= 0").unwrap();
            for &v in delta_out { up.execute(rusqlite::params![v, -1]).unwrap(); gone.execute([v]).unwrap(); }
            for &v in &delta_in { up.execute(rusqlite::params![v, 1]).unwrap(); }
        }
        tx.commit().unwrap();
        let tx = db.unchecked_transaction().unwrap();
        {
            let mut up = tx.prepare_cached("INSERT INTO stat_value VALUES('r', 'c', ?1, ?2) ON CONFLICT DO UPDATE SET n = n + excluded.n").unwrap();
            let mut gone = tx.prepare_cached("DELETE FROM stat_value WHERE rel = 'r' AND col = 'c' AND val = ?1 AND n <= 0").unwrap();
            for &v in &delta_in { up.execute(rusqlite::params![v, -1]).unwrap(); gone.execute([v]).unwrap(); }
            for &v in delta_out { up.execute(rusqlite::params![v, 1]).unwrap(); }
        }
        tx.commit().unwrap();
    }) / (4 * edit) as f64;
    // Sketched: one blob per changed column, rewritten in the rows' transaction.
    let mut s = Sketch::default();
    for &v in vals { s.add(v, 1) }
    let t_blob = best(|| {
        for _ in 0..100 {
            let tx = db.unchecked_transaction().unwrap();
            tx.execute("INSERT OR REPLACE INTO stat_total VALUES('r', 'c', 1, 200000, 0, 0, 0, ?1)", [s.encode()]).unwrap();
            tx.commit().unwrap();
        }
    }) / 100.0;
    let t_blob_in = {
        let tx = db.unchecked_transaction().unwrap();
        let t = best(|| { for _ in 0..1000 { tx.execute("INSERT OR REPLACE INTO stat_total VALUES('r', 'c', 1, 200000, 0, 0, 0, ?1)", [s.encode()]).unwrap(); } }) / 1000.0;
        tx.commit().unwrap(); t
    };
    println!("\nSQLite on disk, {name}: exact upsert {:.0} ns per changed row (own transactions of {} rows); a sketch blob {:.1} µs as its own commit, {:.1} µs inside the rows' transaction", t_sql * 1e9, 2 * edit, t_blob * 1e6, t_blob_in * 1e6);
    let _ = std::fs::remove_file(&path);
}
