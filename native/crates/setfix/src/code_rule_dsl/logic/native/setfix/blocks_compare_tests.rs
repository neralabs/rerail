//! The engine's relations (`Rel`, closed into `InRel`, a maintained run's started by `Rel::based`) against the
//! `rerail-blocks` spike's (rows in a block, `freeze`, a delta read with its base), on the same rows: instructions
//! retired, malloc calls, live heap and bytes held. A measurement, run by name:
//!
//! ```text
//! cargo test --lib --profile profiling blocks_compare_baseline -- --ignored --nocapture
//! cargo test --lib --profile profiling blocks_compare_blocks -- --ignored --nocapture
//! ```
//!
//! Each arm is its own test, run in its own process, so its heap figures start clean.

use super::*;
use rerail_blocks::block::{Block, kind};
use rerail_blocks::relation::{self as brel, RelW, Relation, Spec, View};

const N: usize = 1_000_000;
const K: usize = 3;
const PROBES: usize = 1_000_000;
const EDIT_KILL: usize = 100;
const EDIT_ADD: usize = 100;

/// `N` rows of 3 columns (caller, callee, doc), about 10% of them repeats: the shape of a call-edge relation.
fn rows() -> Vec<i64> {
    let mut v = Vec::with_capacity(N * K);
    let mut x: u64 = 0x2545F4914F6CDD1D;
    for i in 0..N {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let j = if i % 10 == 9 { (x as usize) % i.max(1) } else { i };
        v.extend_from_slice(&[(j * 7 % 400_000) as i64, (j * 13 % 200_000) as i64, (j / 300) as i64]);
    }
    v
}

fn added() -> Vec<i64> {
    (0..EDIT_ADD).flat_map(|i| [900_000 + i as i64, (i * 13 % 200_000) as i64, 4_000]).collect()
}

fn probe_key(i: usize) -> i64 {
    (i * 7919 % 200_000) as i64
}

// --- Allocation counting: libmalloc's logger hook, called on every allocation and free once set. ---

static ALLOCS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static FREES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

unsafe extern "C" fn logger(ty: u32, _a1: usize, _a2: usize, _a3: usize, _res: usize, _skip: u32) {
    use std::sync::atomic::Ordering::Relaxed;
    if ty & 2 != 0 {
        ALLOCS.fetch_add(1, Relaxed);
    }
    if ty & 4 != 0 {
        FREES.fetch_add(1, Relaxed);
    }
}

#[repr(C)]
#[derive(Default)]
struct MStats {
    blocks_in_use: u32,
    size_in_use: usize,
    max_size_in_use: usize,
    size_allocated: usize,
}

unsafe extern "C" {
    static mut malloc_logger: Option<unsafe extern "C" fn(u32, usize, usize, usize, usize, u32)>;
    fn malloc_zone_statistics(zone: *mut std::ffi::c_void, stats: *mut MStats);
}

fn heap() -> MStats {
    let mut s = MStats::default();
    unsafe { malloc_zone_statistics(std::ptr::null_mut(), &mut s) };
    s
}

struct Meter {
    lines: Vec<String>,
}

impl Meter {
    fn new(arm: &str) -> Meter {
        unsafe { malloc_logger = Some(logger) };
        Meter { lines: vec![format!("{arm}:"), format!("  {:<34} {:>14} {:>10} {:>10} {:>12} {:>9}", "step", "insn", "mallocs", "frees", "live heap Δ", "wall ms")] }
    }

    fn time<T>(&mut self, step: &str, f: impl FnOnce() -> T) -> T {
        use std::sync::atomic::Ordering::Relaxed;
        let (a0, f0, h0) = (ALLOCS.load(Relaxed), FREES.load(Relaxed), heap().size_in_use as i64);
        let (i0, t0) = (rerail_blocks::retired(), std::time::Instant::now());
        let out = f();
        let (i1, t1) = (rerail_blocks::retired(), t0.elapsed());
        let (a1, f1, h1) = (ALLOCS.load(Relaxed), FREES.load(Relaxed), heap().size_in_use as i64);
        self.lines.push(format!(
            "  {:<34} {:>14} {:>10} {:>10} {:>12} {:>9.1}",
            step,
            i1 - i0,
            a1 - a0,
            f1 - f0,
            h1 - h0,
            t1.as_secs_f64() * 1e3
        ));
        out
    }

    fn note(&mut self, s: String) {
        self.lines.push(format!("  {s}"));
    }

    fn print(self) {
        unsafe { malloc_logger = None };
        let h = heap();
        println!("{}\n  heap: {} live allocations, {} bytes live, peak {} bytes; footprint {} MB", self.lines.join("\n"), h.blocks_in_use, h.size_in_use, h.max_size_in_use, rerail_blocks::footprint() >> 20);
    }
}

#[test]
#[ignore = "a measurement: run by name (see the module's comment)"]
fn blocks_compare_baseline() {
    let flat = rows();
    let mut m = Meter::new("baseline (setfix Rel → InRel)");
    let types = vec![b'i'; K];
    let kept = m.time("build: push + dedup at push", || {
        let mut rel = Rel::new("edge", &types);
        rel.want(&[1]);
        rel.reserve(N);
        for r in flat.as_chunks::<K>().0 {
            rel.push(r);
        }
        rel
    });
    let kept = m.time("close (index_rest, moved)", || InRel::moved(kept));
    let live = kept.n - kept.ndead;
    m.note(format!("{live} live rows of {N} pushed; values {} bytes, {} orders", kept.rows.capacity_bytes(), kept.chains.len()));

    let hits = m.time("probe col 1, 1M keys", || {
        let (ix, _) = best_order(&kept.chains, &[1]).expect("an order on col 1");
        let mut n = 0u64;
        for i in 0..PROBES {
            for id in ix.find(&kept.rows, K, &[probe_key(i)], None).ids() {
                if !is_dead(&kept.dead, id) {
                    n += kept.rows.value(id, K, 0) as u64 & 1;
                }
            }
        }
        n
    });
    let scan = m.time("scan every live row (id order)", || {
        let mut s = 0i64;
        for id in 0..kept.n as u32 {
            if !is_dead(&kept.dead, id) {
                s = s.wrapping_add(kept.rows.value(id, K, 1));
            }
        }
        s
    });
    let gone: Vec<i64> = flat[..EDIT_KILL * K].to_vec();
    let gone = InRel::from_flat(gone, EDIT_KILL, types.clone());
    let add = added();
    let next = m.time("one edit: based + push + close", || {
        let mut rel = Rel::based("edge", &types, &kept, Some(&gone)).expect("based");
        for r in add.as_chunks::<K>().0 {
            rel.push(r);
        }
        InRel::moved(rel)
    });
    m.note(format!("after the edit: {} live; probe hits {hits}, scan {scan}", next.n - next.ndead));
    m.print();
}

#[test]
#[ignore = "a measurement: run by name (see the module's comment)"]
fn blocks_compare_blocks() {
    let flat = rows();
    let mut m = Meter::new("blocks (rows in a block, freeze)");
    let spec = Spec { widths: vec![4; K], order: vec![0, 1, 2], indexes: vec![vec![1]], min_log: 10 };
    let block = Block::anon(kind::FACTS, 1 << 30);
    let lock = std::sync::Mutex::new(());
    let rel = m.time("build: extend (one claim)", || {
        let rel = Relation::create(&block, &spec, None);
        RelW { block: &block, rel, lock: &lock }.extend(&flat);
        rel
    });
    let st = m.time("freeze (dedup, order, index, stats)", || brel::freeze(&block, rel, &spec, None));
    m.note(format!("{} live rows of {N} pushed; block {} bytes used", st.live, block.used()));
    let v = View { base: None, top: rel };

    let hits = m.time("probe col 1, 1M keys", || {
        let mut n = 0u64;
        for i in 0..PROBES {
            for id in v.probe_iter(0, &[probe_key(i)]) {
                n += v.get(id, 0) as u64 & 1;
            }
        }
        n
    });
    let scan = m.time("scan every live row (canonical order)", || {
        let mut s = 0i64;
        for id in v.ordered_iter() {
            s = s.wrapping_add(v.get(id, 1));
        }
        s
    });
    let scan_ids = m.time("scan every live row (id order)", || {
        let mut s = 0i64;
        for id in 0..rel.rows.len() {
            if v.live(id) {
                s = s.wrapping_add(v.get(id, 1));
            }
        }
        s
    });
    assert_eq!(scan, scan_ids);
    let add = added();
    let delta_block = Block::anon(kind::FACTS, 64 << 20);
    let dlock = std::sync::Mutex::new(());
    let d = m.time("one edit: delta + kill + push + freeze", || {
        let d = Relation::create(&delta_block, &spec, Some(rel));
        let w = RelW { block: &delta_block, rel: d, lock: &dlock };
        for id in 0..EDIT_KILL as u32 {
            w.kill(id);
        }
        w.extend(&add);
        brel::freeze(&delta_block, d, &spec, Some(rel));
        d
    });
    let dv = View { base: Some(rel), top: d };
    m.note(format!("after the edit: {} live; delta block {} bytes; probe hits {hits}, scan {scan}", dv.live_len(), delta_block.used()));
    m.print();
}
