//! R1 of decision 0448: the searches equal a host reference, across SIMD-group and group edges.
//! The scans, segmented scans and sorts are the persistent grid's, tested with UKL's library
//! (`gpu/src/library_tests.rs`: `the_grid_*`).

use super::*;
use crate::lane::Pod;

/// Sizes that cross a SIMD group (32), a group (256) and a scan block (1024).
const SIZES: [usize; 12] = [0, 1, 31, 33, 255, 257, 1023, 1025, 4097, 70_001, 1_048_577, 1_100_000];

/// A small generator (xorshift), so the inputs are the same every run.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// A test's own buffer of `v` (at least one value's room).
fn buf<T: Pod + Default>(v: &[T]) -> Buffer {
    let device = lane::device().unwrap();
    if v.is_empty() { lane::buffer_of(device, &[T::default()]) } else { lane::buffer_of(device, v) }
}

fn zeroed<T: Pod + Default>(n: usize) -> Buffer {
    buf(&vec![T::default(); n.max(1)])
}

fn read<T: Copy>(b: &Buffer, n: usize) -> Vec<T> {
    // SAFETY: the device is done with the buffer, which holds at least `n` values.
    unsafe { std::slice::from_raw_parts(b.contents().cast::<T>(), n).to_vec() }
}

/// Runs `f`'s launches in one command, and waits.
fn run<'a>(f: impl FnOnce(&Enc<'a>)) {
    crate::pp_eval::run(|e| {
        f(e);
        Ok(())
    })
    .unwrap();
}

#[test]
fn search_and_expand_equal_the_host() {
    let mut r = Rng(17);
    for &n in &SIZES[..10] {
        let mut sorted: Vec<u64> = (0..n).map(|_| r.next() % 5000).collect();
        sorted.sort_unstable();
        let queries: Vec<u64> = (0..n.max(1) * 2).map(|_| r.next() % 5100).collect();
        let (sb, qb, out, counts) = (buf(&sorted), buf(&queries), zeroed::<u32>(queries.len()), buf(&[n as u32, queries.len() as u32]));
        run(|e| e.run("pb_search", &[("sorted", &sb), ("ns", &counts), ("queries", &qb), ("nq", &counts), ("out", &out)], Some(ProbeArgs([0, 1, 0, 0])), (queries.len() as u64).div_ceil(TG)));
        let want: Vec<u32> = queries.iter().map(|&q| sorted.partition_point(|&s| s < q) as u32).collect();
        assert_eq!(read::<u32>(&out, queries.len()), want, "search, n = {n}");
        // Expand: items with counts 0 to 3, their offsets an exclusive scan.
        let item_counts: Vec<u32> = (0..n).map(|_| (r.next() % 4) as u32).collect();
        let mut acc = 0u32;
        let offsets: Vec<u32> = item_counts.iter().map(|&c| { let a = acc; acc += c; a }).collect();
        let total = acc as usize;
        let (ob, nb, tb) = (buf(&offsets), buf(&[n as u32]), buf(&[acc]));
        let (item, rank) = (zeroed::<u32>(total), zeroed::<u32>(total));
        run(|e| e.run("pb_expand", &[("offsets", &ob), ("n", &nb), ("total", &tb), ("item", &item), ("rank", &rank)], Some(ProbeArgs::default()), (total as u64).div_ceil(TG)));
        let mut want = Vec::new();
        for (k, &c) in item_counts.iter().enumerate() {
            for j in 0..c {
                want.push((k as u32, j));
            }
        }
        let got: Vec<(u32, u32)> = read::<u32>(&item, total).into_iter().zip(read::<u32>(&rank, total)).collect();
        assert_eq!(got, want, "expand, n = {n}");
    }
}

/// R9 of decision 0448: an evaluation is one command, so the host never reads between its
/// passes. `pp_eval::run` takes a whole evaluation's launches (an expansion's included) and
/// refuses one that used a second command.
#[test]
fn an_evaluation_is_one_command_buffer() {
    let b = [buf(&[1u64, 3]), buf(&[2u32, 1]), buf(&[2u64]), buf(&[0u32])];
    fn search<'a>(enc: &Enc<'a>, b: &'a [Buffer; 4]) {
        enc.run("pb_search", &[("sorted", &b[0]), ("ns", &b[1]), ("queries", &b[2]), ("nq", &b[1]), ("out", &b[3])], Some(ProbeArgs([0, 1, 0, 0])), 1);
    }
    let one = crate::pp_eval::run(|enc| {
        search(enc, &b);
        Ok(())
    });
    assert_eq!(one.unwrap(), 1);
    let two = crate::pp_eval::run(|enc| {
        search(enc, &b);
        // A second command, waited on mid-evaluation: a host read between passes.
        let other = Enc::new("a second command");
        search(&other, &b);
        other.finish().map(|_| ())
    });
    assert!(two.is_err(), "a second command in one evaluation");
}

/// A measurement, not a check: the grid's sort alone, a few keys, one group.
#[test]
#[ignore = "a measurement"]
fn the_grid_sort_alone_is_fast() {
    // Hashed keys (a definitions' rows), and the closes' (a depth by 2^15 over a position).
    let hashed = |n: usize| (0..n as u64).map(|k| k.wrapping_mul(0x9e37_79b9_7f4a_7c15)).collect::<Vec<u64>>();
    let closes = |n: usize| (0..n as u64).map(|k| ((32767 + k % 5) << 24) | (k * 7)).collect::<Vec<u64>>();
    for (what, n, low, bits) in [("hashed", 500usize, 0u32, 64u32), ("hashed", 1000, 0, 64), ("closes", 1600, 0, 40), ("closes", 1600, 24, 40), ("closes", 6000, 24, 40)] {
        let keys = if what == "hashed" { hashed(n) } else { closes(n) };
        for groups in [1u64, 32] {
            let z32 = zeroed::<u32>;
            let t = (0..5)
                .map(|_| {
                    let (ctl, kb, va, k2, v2, hist, sums) = (z32(8), buf(&keys), z32(n), zeroed::<u64>(n), z32(n), z32(256 * n.div_ceil(1024) + 1), z32(4 * groups as usize + 4));
                    let rows: [(&str, &dyn Bound); 7] = [("ctl", &ctl), ("keys", &kb), ("values", &va), ("keys2", &k2), ("values2", &v2), ("hist", &hist), ("sums", &sums)];
                    crate::pp_eval::run_timed(|e| {
                        e.run("pb_sort_alone", &rows, Some(ProbeArgs([n as u32, bits, low, 0])), groups);
                        Ok(())
                    })
                    .unwrap()
                    .1
                })
                .fold(f64::MAX, f64::min);
            eprintln!("sort alone: {n} {what} keys, bits {low}..{bits}, {groups} groups: {:.1} us", t * 1e6);
        }
    }
}

/// A measurement, not a check: what a grid barrier, a device fence, a threadgroup barrier and a
/// dependent coherent load each cost.
#[test]
#[ignore = "a measurement"]
fn what_a_barrier_costs() {
    let names = ["grid barrier", "device fence", "tg barrier (device)", "coherent load chain", "plain load chain"];
    for what in 0..5u32 {
        for groups in [1u64, 32] {
            let time = |n: u32| {
                (0..5)
                    .map(|_| {
                        let (ctl, data) = (zeroed::<u32>(8), zeroed::<u32>(512));
                        crate::pp_eval::run_timed(|e| {
                            e.run("pb_barriers_alone", &[("ctl", &ctl), ("data", &data)], Some(ProbeArgs([n, what, 0, 0])), groups);
                            Ok(())
                        })
                        .unwrap()
                        .1
                    })
                    .fold(f64::MAX, f64::min)
            };
            let (t0, t1) = (time(0), time(1000));
            if what == 0 {
                eprintln!("empty run, {groups} groups: {:.1} us", t0 * 1e6);
            }
            eprintln!("{}, {groups} groups: {:.3} us each", names[what as usize], (t1 - t0) * 1e6 / 1000.0);
        }
    }
}

/// The evaluator's kernels compile (a failed compile is fatal: its message is the panic's).
#[test]
fn the_source_compiles() {
    // `RERAIL_PP_DUMP`: the source as Metal's front end takes it, written there (its errors' lines).
    if let Some(path) = std::env::var_os("RERAIL_PP_DUMP") {
        std::fs::write(path, rerail_ukl::metal_source("pp_setup", source()).unwrap_or_else(|e| e.to_string())).unwrap();
    }
    for name in ["pp_setup", "px_expand", "pp_check", "pb_search", "pb_expand", "pd_relex_test", "pb_sort_alone", "pb_barriers_alone"] {
        kernel(name);
    }
}
