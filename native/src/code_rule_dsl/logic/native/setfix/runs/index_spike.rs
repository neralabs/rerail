//! Spike (measurement only, t50 step 5): the probe structures a relation's index could be, over the relations a
//! run captured (`RERAIL_RELATIONAL_LAB`), probed as the joins probe them.
//!
//! `RERAIL_INDEX_SPIKE=<capture dir>` (its `*.sqlite`, `RERAIL_INDEX_SPIKE_ONLY=<text>` to take those whose name holds it): each source
//! table of each capture, of 1000 rows or more, is held five ways:
//!
//! - `runs`: today's order (`Index::new`, fences, or tags for the order on every column), probed by `find_at`;
//! - `printed`: today's order with each id's key print (`Index::printed`), as a derived input's order probed by whole
//!   keys is;
//! - `hash`: a hash map from the key to its range of the ids sorted by key (`hashbrown`, Fx);
//! - `sorted hash`: the ids sorted by a 64-bit hash of the key, the hashes beside them, a key found by interpolation
//!   then a gallop (mldb's `id_hash` idea: hash order, so two of them merge in a pass);
//! - `dense`: where the lead column is a node (`doc << 32 | local`), per document an offset per local into the ids
//!   sorted by key: a probe is the document's base, then two loads.
//!
//! Each is probed by the lead column (`lead`) and by the whole key (`whole`, hits; `miss`, the key with its last
//! column changed), the keys in key order (`sorted`, the best case of `find_at`'s hints) and shuffled. Every
//! structure must find the same ids; the test fails if one does not.

use super::*;
use std::hint::black_box;
use std::time::Instant;

/// A table's rows, flat (`k` a row).
struct Table {
    name: String,
    k: usize,
    rows: Vec<i64>,
}

impl Table {
    fn n(&self) -> usize {
        self.rows.len() / self.k
    }
    fn at(&self, id: u32, c: usize) -> i64 {
        self.rows[id as usize * self.k + c]
    }
}

fn load(path: &std::path::Path) -> Vec<Table> {
    let db = rusqlite::Connection::open(path).unwrap();
    let names: Vec<String> = db
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name GLOB 's[0-9]*' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let mut out = Vec::new();
    for name in names {
        let k = db.prepare(&format!("SELECT * FROM {name} LIMIT 0")).unwrap().column_count();
        let mut st = db.prepare(&format!("SELECT * FROM {name}")).unwrap();
        let mut rows = Vec::new();
        let mut q = st.query([]).unwrap();
        while let Some(r) = q.next().unwrap() {
            for c in 0..k {
                rows.push(r.get::<_, Option<i64>>(c).unwrap().unwrap_or(0));
            }
        }
        if k > 0 && rows.len() / k >= 1000 {
            out.push(Table { name: format!("{}:{name}", path.file_stem().unwrap().to_string_lossy()), k, rows });
        }
    }
    out
}

#[inline]
fn mix(mut h: u64) -> u64 {
    // splitmix64's finalizer: every bit of the key reaches the high bits interpolation reads.
    h ^= h >> 30;
    h = h.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    h ^= h >> 27;
    h = h.wrapping_mul(0x94d0_49bb_1331_11eb);
    h ^ (h >> 31)
}

#[inline]
fn hash_key(key: &[i64]) -> u64 {
    let mut h = 0x9E37_79B9_7F4A_7C15u64;
    for &v in key {
        h = (h.rotate_left(23) ^ v as u64).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
    mix(h)
}

/// The ids sorted by the columns (then id): what `hash` and `dense` hold ranges of.
fn sorted_ids(t: &Table) -> Vec<u32> {
    let mut ids: Vec<u32> = (0..t.n() as u32).collect();
    ids.sort_unstable_by(|&a, &b| (0..t.k).map(|c| t.at(a, c).cmp(&t.at(b, c))).find(|o| o.is_ne()).unwrap_or(Ordering::Equal).then(a.cmp(&b)));
    ids
}

/// `hash`: key (its first `m` columns) to its range of `ids`.
struct HashIx {
    ids: Vec<u32>,
    map: hashbrown::HashMap<Vec<i64>, (u32, u32), rustc_hash::FxBuildHasher>,
}

impl HashIx {
    fn new(t: &Table, ids: Vec<u32>, m: usize) -> HashIx {
        let mut map: hashbrown::HashMap<Vec<i64>, (u32, u32), rustc_hash::FxBuildHasher> = hashbrown::HashMap::default();
        let mut i = 0;
        while i < ids.len() {
            let key: Vec<i64> = (0..m).map(|c| t.at(ids[i], c)).collect();
            let mut j = i + 1;
            while j < ids.len() && (0..m).all(|c| t.at(ids[j], c) == key[c]) {
                j += 1;
            }
            map.insert(key, (i as u32, j as u32));
            i = j;
        }
        HashIx { ids, map }
    }
    #[inline]
    fn find(&self, key: &[i64]) -> &[u32] {
        match self.map.get(key) {
            Some(&(a, b)) => &self.ids[a as usize..b as usize],
            None => &[],
        }
    }
    fn bytes(&self) -> usize {
        self.ids.len() * 4 + self.map.capacity() * (24 + 8 + 8 + 1)
    }
}

/// `sorted hash`: ids in the order of their key's hash (the first `m` columns), the hashes beside them.
struct SortedHash {
    hashes: Vec<u64>,
    ids: Vec<u32>,
    m: usize,
}

impl SortedHash {
    fn new(t: &Table, m: usize) -> SortedHash {
        let mut v: Vec<(u64, u32)> = (0..t.n() as u32).map(|id| (hash_key(&(0..m).map(|c| t.at(id, c)).collect::<Vec<_>>()), id)).collect();
        v.sort_unstable();
        SortedHash { hashes: v.iter().map(|p| p.0).collect(), ids: v.iter().map(|p| p.1).collect(), m }
    }
    /// The ids whose key is `key`: placed by the hash's high bits, galloped to its first hash, its equal hashes
    /// checked against the rows.
    #[inline]
    fn find(&self, t: &Table, key: &[i64], out: &mut Vec<u32>) {
        out.clear();
        let n = self.hashes.len();
        if n == 0 {
            return;
        }
        let h = hash_key(key);
        let guess = ((u128::from(h) * n as u128) >> 64) as usize;
        let s = gallop(&self.hashes, guess, |&x| x < h);
        let mut i = s;
        while i < n && self.hashes[i] == h {
            let id = self.ids[i];
            if (0..self.m).all(|c| t.at(id, c) == key[c]) {
                out.push(id);
            }
            i += 1;
        }
    }
    fn bytes(&self) -> usize {
        self.hashes.len() * 12
    }
}

/// `dense`: per document (the lead column's high half), the offset of each local's ids in `ids` (sorted by key).
struct Dense {
    ids: Vec<u32>,
    /// (document, first offset slot, locals): sorted by document.
    docs: Vec<(u32, u32, u32)>,
    offsets: Vec<u32>,
}

impl Dense {
    /// `None` unless every lead value is a node id (`doc << 32 | local`, a document past 0).
    fn new(t: &Table, ids: Vec<u32>) -> Option<Dense> {
        if (0..t.n() as u32).any(|id| (t.at(id, 0) >> 32) <= 0) {
            return None;
        }
        let mut docs: Vec<(u32, u32, u32)> = Vec::new();
        for &id in &ids {
            let v = t.at(id, 0);
            let (d, l) = ((v >> 32) as u32, v as u32);
            match docs.last_mut() {
                Some(last) if last.0 == d => last.2 = last.2.max(l.saturating_add(1)),
                _ => docs.push((d, 0, l.saturating_add(1))),
            }
        }
        // Dense only where it is: offsets past 4 a row (a lead that is not a node, or nodes spread thin) are not.
        let slots: u64 = docs.iter().map(|d| u64::from(d.2) + 1).sum();
        if slots > 4 * t.n() as u64 + 1024 {
            return None;
        }
        let mut slot = 0u32;
        for d in &mut docs {
            d.1 = slot;
            slot += d.2 + 1;
        }
        let mut offsets = vec![0u32; slot as usize];
        // offsets[base + l] = the first position of local l (or of the next local present).
        let mut pos = 0usize;
        for d in 0..docs.len() {
            let (doc, base, locals) = docs[d];
            for l in 0..=locals {
                while pos < ids.len() && {
                    let v = t.at(ids[pos], 0);
                    ((v >> 32) as u32, v as u32) < (doc, l)
                } {
                    pos += 1;
                }
                offsets[(base + l) as usize] = pos as u32;
            }
        }
        Some(Dense { ids, docs, offsets })
    }
    #[inline]
    fn range(&self, lead: i64) -> (usize, usize) {
        let (d, l) = ((lead >> 32) as u32, lead as u32);
        let Ok(i) = self.docs.binary_search_by_key(&d, |e| e.0) else { return (0, 0) };
        let (_, base, locals) = self.docs[i];
        if l >= locals {
            return (0, 0);
        }
        let at = (base + l) as usize;
        (self.offsets[at] as usize, self.offsets[at + 1] as usize)
    }
    /// The ids whose first `key.len()` columns are `key`: the lead's range, then (a longer key) its rows compared.
    #[inline]
    fn find(&self, t: &Table, key: &[i64], out: &mut Vec<u32>) {
        out.clear();
        let (a, b) = self.range(key[0]);
        for &id in &self.ids[a..b] {
            if (1..key.len()).all(|c| t.at(id, c) == key[c]) {
                out.push(id);
            }
        }
    }
    fn bytes(&self) -> usize {
        self.ids.len() * 4 + self.offsets.len() * 4 + self.docs.len() * 12
    }
}

/// A shuffle (xorshift), the same every run.
fn shuffled<T>(mut v: Vec<T>) -> Vec<T> {
    let mut x = 0x2545_F491_4F6C_DD1Du64;
    for i in (1..v.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        v.swap(i, (x % (i as u64 + 1)) as usize);
    }
    v
}

/// Nanoseconds a probe of `probe` over `keys`, repeated to a million probes or more; and the ids found, summed
/// (each id's value, so two structures that find different ids differ).
fn time(keys: &[Vec<i64>], mut probe: impl FnMut(&[i64]) -> u64) -> (f64, u64) {
    let reps = (1_000_000 / keys.len().max(1)).max(1);
    let mut sum = 0u64;
    for key in keys {
        sum = sum.wrapping_add(probe(key));
    }
    let t0 = Instant::now();
    let mut s = 0u64;
    for _ in 0..reps {
        for key in keys {
            s = s.wrapping_add(probe(black_box(key)));
        }
    }
    black_box(s);
    (t0.elapsed().as_nanos() as f64 / (reps * keys.len()) as f64, sum)
}

fn checksum(ids: impl Iterator<Item = u32>) -> u64 {
    ids.fold(0u64, |s, id| s.wrapping_add(u64::from(id).wrapping_mul(0x9E37_79B9) ^ 1))
}

#[test]
#[ignore = "reads captures a run made with RERAIL_RELATIONAL_LAB"]
fn index_spike() {
    let dir = std::env::var("RERAIL_INDEX_SPIKE").expect("RERAIL_INDEX_SPIKE");
    let only = std::env::var("RERAIL_INDEX_SPIKE_ONLY").unwrap_or_default();
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "sqlite") && p.file_name().is_some_and(|n| n != "atoms.sqlite" && n.to_string_lossy().contains(only.as_str())))
        .collect();
    paths.sort();
    println!("{:<26} {:>8} {:>2} {:<7} {:<8} | {:>8} {:>8} {:>8} {:>8} {:>8} | build ns/row, bytes/row: runs printed hash shash dense", "table", "rows", "k", "probe", "order", "runs", "printed", "hash", "shash", "dense");
    let mut mismatches = 0;
    for path in &paths {
        for t in load(path) {
            let (n, k) = (t.n(), t.k);
            let data: &[i64] = &t.rows;
            let cols: Vec<usize> = (0..k).collect();
            // Builds, each timed.
            let b0 = Instant::now();
            let mut runs = Index::new(cols.clone());
            runs.add_run(data, k, 0, n, 0);
            let b_runs = b0.elapsed().as_nanos() as f64 / n as f64;
            let b0 = Instant::now();
            let mut printed = Index::printed(cols.clone());
            printed.add_run(data, k, 0, n, 0);
            let b_printed = b0.elapsed().as_nanos() as f64 / n as f64;
            let ids = sorted_ids(&t);
            let b0 = Instant::now();
            let hash_lead = HashIx::new(&t, ids.clone(), 1);
            let hash_whole = HashIx::new(&t, ids.clone(), k);
            let b_hash = b0.elapsed().as_nanos() as f64 / n as f64 / 2.0;
            let b0 = Instant::now();
            let sh_lead = SortedHash::new(&t, 1);
            let sh_whole = SortedHash::new(&t, k);
            let b_sh = b0.elapsed().as_nanos() as f64 / n as f64 / 2.0;
            let b0 = Instant::now();
            let dense = Dense::new(&t, ids.clone());
            let b_dense = b0.elapsed().as_nanos() as f64 / n as f64;
            let builds = format!(
                "{b_runs:.0} {b_printed:.0} {b_hash:.0} {b_sh:.0} {} | {:.1} {:.1} {:.1} {:.1} {}",
                dense.as_ref().map_or("-".to_owned(), |_| format!("{b_dense:.0}")),
                runs.bytes() as f64 / n as f64,
                printed.bytes() as f64 / n as f64,
                (hash_lead.bytes() + hash_whole.bytes()) as f64 / n as f64 / 2.0,
                (sh_lead.bytes() + sh_whole.bytes()) as f64 / n as f64 / 2.0,
                dense.as_ref().map_or("-".to_owned(), |d| format!("{:.1}", d.bytes() as f64 / n as f64)),
            );
            // The keys: each row's lead (distinct), each row whole, and misses (the last column moved off).
            let mut leads: Vec<Vec<i64>> = (0..n as u32).map(|id| vec![t.at(id, 0)]).collect();
            leads.sort_unstable();
            leads.dedup();
            let mut wholes: Vec<Vec<i64>> = (0..n as u32).map(|id| (0..k).map(|c| t.at(id, c)).collect()).collect();
            wholes.sort_unstable();
            wholes.dedup();
            let misses: Vec<Vec<i64>> = wholes.iter().map(|w| { let mut m = w.clone(); *m.last_mut().unwrap() ^= 0x5555_5555; m }).collect();
            let mut kinds: Vec<(&str, Vec<Vec<i64>>)> = vec![("lead", leads)];
            if k >= 2 {
                kinds.push(("whole", wholes));
                kinds.push(("miss", misses));
            }
            let mut first = true;
            for (kind, keys) in kinds {
                for order in ["sorted", "shuffled"] {
                    let keys = if order == "sorted" { keys.clone() } else { shuffled(keys.clone()) };
                    let whole = kind != "lead";
                    let mut hints = [0u32; HINTS];
                    let (t_runs, c_runs) = time(&keys, |key| checksum(runs.find_at(data, k, key, None, &mut hints).ids()));
                    let mut hints = [0u32; HINTS];
                    let (t_printed, c_printed) = time(&keys, |key| checksum(printed.find_at(data, k, key, None, &mut hints).ids()));
                    let hix = if whole { &hash_whole } else { &hash_lead };
                    let (t_hash, c_hash) = time(&keys, |key| checksum(hix.find(key).iter().copied()));
                    let shx = if whole { &sh_whole } else { &sh_lead };
                    let mut buf = Vec::new();
                    let (t_sh, c_sh) = time(&keys, |key| { shx.find(&t, key, &mut buf); checksum(buf.iter().copied()) });
                    let dres = dense.as_ref().map(|d| time(&keys, |key| { d.find(&t, key, &mut buf); checksum(buf.iter().copied()) }));
                    // The sums are of ids' values, in any order: a structure that found other ids differs.
                    let want = c_runs;
                    let mut bad = Vec::new();
                    if c_printed != want { bad.push("printed") }
                    if c_hash != want { bad.push("hash") }
                    if c_sh != want { bad.push("sorted hash") }
                    if dres.is_some_and(|(_, c)| c != want) { bad.push("dense") }
                    if !bad.is_empty() {
                        mismatches += 1;
                        println!("    MISMATCH {} {kind} {order}: {bad:?} found other ids than runs", t.name);
                    }
                    println!(
                        "{:<26} {:>8} {:>2} {:<7} {:<8} | {t_runs:>8.1} {t_printed:>8.1} {t_hash:>8.1} {t_sh:>8.1} {:>8} | {}",
                        if first { t.name.as_str() } else { "" },
                        if first { n.to_string() } else { String::new() },
                        if first { k.to_string() } else { String::new() },
                        kind,
                        order,
                        dres.map_or("-".to_owned(), |(d, _)| format!("{d:.1}")),
                        if first { builds.as_str() } else { "" },
                    );
                    first = false;
                }
            }
        }
    }
    assert_eq!(mismatches, 0, "every structure finds the ids today's order finds");
}
