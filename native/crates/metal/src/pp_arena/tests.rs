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

/// `<ukl/pool.h>` over a grid (decision 0478 §10): the pieces of ranges given, taken by every
/// thread of 8 groups at once, then given back and taken again. No block is taken twice in a
/// round, each lies inside one given range and fits its class, and where the takers ask for
/// more than a class holds, every block of it is taken.
#[test]
fn the_pool_gives_each_block_once() {
    use std::collections::BTreeMap;
    let size = |c: u32| 1u32 << c;
    let floor = |n: u32| 31 - n.leading_zeros();
    let mut r = Rng(29);
    // Ranges of 1 to 300 units, apart.
    let (mut ranges, mut at) = (Vec::new(), 5u32);
    for _ in 0..3000 {
        let n = 1 + (r.next() % 300) as u32;
        ranges.extend([at, n]);
        at += n + 1 + (r.next() % 3) as u32;
    }
    // The pieces, as the pool gives them.
    let mut supply: BTreeMap<u32, usize> = BTreeMap::new();
    let mut owner = vec![u32::MAX; at as usize];
    for (i, w) in ranges.chunks(2).enumerate() {
        let (mut s, mut n) = (w[0], w[1]);
        while n > 0 {
            let c = floor(n);
            *supply.entry(c).or_default() += 1;
            s += size(c);
            n -= size(c);
        }
        for u in w[0]..w[0] + w[1] { owner[u as usize] = i as u32; }
    }
    let groups = 8u64;
    let threads = groups as usize * TG as usize;
    let per = 4usize;
    // Classes the pieces have, so the takers ask for more than some hold and less than others.
    let have: Vec<u32> = supply.keys().copied().collect();
    let classes: Vec<u32> = (0..threads * per).map(|_| have[(r.next() % have.len() as u64) as usize]).collect();
    let mut demand: BTreeMap<u32, usize> = BTreeMap::new();
    for &c in &classes { *demand.entry(c).or_default() += 1; }
    for rounds in [1u32, 3] {
        let (ctl, rb, cb) = (zeroed::<u32>(3), buf(&ranges), buf(&classes));
        let (heads, next, out) = (zeroed::<u32>(128), zeroed::<u32>(at as usize), zeroed::<u32>(threads * per));
        run(|e| e.run("pb_pool", &[("ctl", &ctl), ("ranges", &rb), ("classes", &cb), ("heads", &heads), ("next", &next), ("out", &out)], Some(ProbeArgs([(ranges.len() / 2) as u32, per as u32, rounds, 0])), groups));
        assert_eq!(read::<u32>(&ctl, 3)[2], 0, "a grid barrier failed");
        let got = read::<u32>(&out, threads * per);
        let mut used = vec![false; at as usize];
        let mut taken: BTreeMap<u32, usize> = BTreeMap::new();
        for (k, &s) in got.iter().enumerate() {
            if s == u32::MAX { continue; }
            let c = classes[k];
            *taken.entry(c).or_default() += 1;
            let o = owner[s as usize];
            for u in s..s + size(c) {
                assert!(o != u32::MAX && owner[u as usize] == o, "rounds {rounds}: a block of class {c} at {s} is not inside one given range");
                assert!(!std::mem::replace(&mut used[u as usize], true), "rounds {rounds}: unit {u} taken twice");
            }
        }
        for (&c, &d) in &demand {
            let s = supply.get(&c).copied().unwrap_or(0);
            assert_eq!(taken.get(&c).copied().unwrap_or(0), d.min(s), "rounds {rounds}: class {c}'s blocks taken ({d} asked, {s} held)");
        }
    }
}

/// `<ukl/pool.h>`'s split takes over a grid (decision 0478 §10): every thread of 8 groups takes
/// blocks of one class from pieces of every class, a larger piece split and its rest pushed back
/// while others take; then they are given back and taken again. No unit is taken twice, each block
/// lies inside one given range, and the takes met are the demand or the pieces' room in blocks of
/// the class (a piece of class `k` at least `c` holds 2^(k-c)), whichever is less.
#[test]
fn the_pool_splits_a_larger_block() {
    let size = |c: u32| 1u32 << c;
    let floor = |n: u32| 31 - n.leading_zeros();
    let mut r = Rng(31);
    let (mut ranges, mut at) = (Vec::new(), 5u32);
    for _ in 0..400 {
        let n = 1 + (r.next() % 300) as u32;
        ranges.extend([at, n]);
        at += n + 1 + (r.next() % 3) as u32;
    }
    let mut pieces = Vec::new();
    let mut owner = vec![u32::MAX; at as usize];
    for (i, w) in ranges.chunks(2).enumerate() {
        let mut n = w[1];
        while n > 0 {
            let c = floor(n);
            pieces.push(c);
            n -= size(c);
        }
        for u in w[0]..w[0] + w[1] { owner[u as usize] = i as u32; }
    }
    let groups = 8u64;
    let threads = groups as usize * TG as usize;
    let per = 4usize;
    for c in [0u32, 1, 3] {
        let room: usize = pieces.iter().filter(|&&k| k >= c).map(|&k| 1usize << (k - c)).sum();
        let classes = vec![c; threads * per];
        for rounds in [1u32, 3] {
            let (ctl, rb, cb) = (zeroed::<u32>(3), buf(&ranges), buf(&classes));
            let (heads, next, out) = (zeroed::<u32>(128), zeroed::<u32>(at as usize), zeroed::<u32>(threads * per));
            run(|e| e.run("pb_pool", &[("ctl", &ctl), ("ranges", &rb), ("classes", &cb), ("heads", &heads), ("next", &next), ("out", &out)], Some(ProbeArgs([(ranges.len() / 2) as u32, per as u32, rounds, 1])), groups));
            assert_eq!(read::<u32>(&ctl, 3)[2], 0, "a grid barrier failed");
            let got = read::<u32>(&out, threads * per);
            let mut used = vec![false; at as usize];
            let mut met = 0usize;
            for &s in &got {
                if s == u32::MAX { continue; }
                met += 1;
                let o = owner[s as usize];
                for u in s..s + size(c) {
                    assert!(o != u32::MAX && owner[u as usize] == o, "class {c}, rounds {rounds}: a block at {s} is not inside one given range");
                    assert!(!std::mem::replace(&mut used[u as usize], true), "class {c}, rounds {rounds}: unit {u} taken twice");
                }
            }
            assert_eq!(met, (threads * per).min(room), "class {c}, rounds {rounds}: the takes met ({} asked, room for {room})", threads * per);
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
    for name in ["pp_setup", "pp_check", "pb_search", "pb_expand", "pd_relex_test", "pb_sort_alone", "pb_barriers_alone", "pb_pool"] {
        kernel(name);
    }
}

/// §5 of decision 0460: the evaluator's source is UKL for every backend, not Metal's alone: for
/// OpenCL (its SIMD group emulated, width 32) it is OpenCL C 1.2 that clang's front end accepts,
/// so it holds no C++ and every function that reaches a SIMD-group operation takes its state
/// (`__UKL_PARAMS`). The OpenCL gate of decision 0440 §3.5.
#[test]
fn the_source_is_opencl_c_1_2() {
    use rerail_ukl::{Backend, Preprocess};
    let main = "/<kernels>/pp.cl";
    let src = Preprocess::new(Backend::OpenCl).define("UKL_SIMD_WIDTH", Some("32u")).file(main, source()).run(main).unwrap_or_else(|e| panic!("{e}"));
    let file = std::env::temp_dir().join(format!("rerail-pp-cl-{}.cl", std::process::id()));
    std::fs::write(&file, &src).unwrap();
    let Ok(o) = std::process::Command::new("clang").args(["-x", "cl", "-cl-std=CL1.2", "-fsyntax-only", "-Werror=implicit-function-declaration", "-ferror-limit=40"]).arg(&file).output() else {
        eprintln!("no clang here: the evaluator preprocessed for OpenCL, not compiled");
        return;
    };
    let errors = String::from_utf8_lossy(&o.stderr).lines().filter(|l| l.contains("error")).take(40).collect::<Vec<_>>().join("\n");
    if o.status.success() {
        std::fs::remove_file(&file).ok();
    }
    assert!(o.status.success(), "the evaluator is not OpenCL C 1.2 ({}):\n{errors}", file.display());
}

/// §4 of decision 0460: the grid is the device's resident count at the grid kernels' threadgroup
/// memory (`Device::resident_groups`), at most `GROUPS_MOST`, and `GROUPS_UNKNOWN` on a device
/// that gives no cores.
#[test]
fn the_grid_is_the_devices_resident_count() {
    let device = lane::device().unwrap();
    let local = kernel("pp_setup").threadgroup_bytes();
    match device.gpu_cores() {
        Some(cores) => {
            eprintln!("{cores} GPU cores, {local} bytes of threadgroup memory a group: {} groups", groups());
            let resident = device.resident_groups(TG as u32, local).unwrap();
            assert_eq!(groups(), resident.min(GROUPS_MOST), "the resident count, capped");
            assert!(resident >= cores as u64, "a core holds a group at least");
        }
        None => assert_eq!(groups(), GROUPS_UNKNOWN),
    }
}
