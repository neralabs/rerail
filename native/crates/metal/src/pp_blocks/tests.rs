//! R1 of decision 0448: the searches equal a host reference, across SIMD-group and group edges.
//! The scans, segmented scans and sorts are the persistent grid's, tested with UKL's library
//! (`gpu/src/library_tests.rs`: `the_grid_*`).

use super::*;
use crate::launch::{LaunchScope, commit_checked, wait_checked};
use crate::runtime::MappedBuffer;

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

fn buf<T: Copy>(v: &[T]) -> MappedBuffer {
    let device = crate::device().unwrap();
    if v.is_empty() { MappedBuffer::zeroed::<T>(&device, 1).unwrap() } else { MappedBuffer::from_slice(&device, v).unwrap() }
}

/// A store the host reads after a run: a test's own buffer, or one of an arena's.
trait Host {
    fn host_bytes(&self) -> *mut u8;
}
impl Host for MappedBuffer {
    fn host_bytes(&self) -> *mut u8 {
        self.host()
    }
}
impl Host for Buf {
    fn host_bytes(&self) -> *mut u8 {
        self.host()
    }
}

fn read<T: Copy>(b: &impl Host, n: usize) -> Vec<T> {
    unsafe { std::slice::from_raw_parts(b.host_bytes().cast::<T>(), n).to_vec() }
}

/// Runs `f`'s encoding in one command buffer, and waits.
fn run(f: impl FnOnce(&Enc<'_>) -> Result<(), Error>) {
    let device = crate::device().unwrap();
    let _scope = LaunchScope::enter();
    let queue = crate::pool::queue(&device).unwrap();
    let command = queue.new_command_buffer();
    f(&Enc { command, source: source().unwrap() }).unwrap();
    commit_checked(command).unwrap();
    wait_checked(command).unwrap();
}

#[test]
fn search_and_expand_equal_the_host() {
    let mut r = Rng(17);
    let dev = crate::device().unwrap();
    for &n in &SIZES[..10] {
        let mut sorted: Vec<u64> = (0..n).map(|_| r.next() % 5000).collect();
        sorted.sort_unstable();
        let queries: Vec<u64> = (0..n.max(1) * 2).map(|_| r.next() % 5100).collect();
        let (sb, qb, out, counts) = (buf(&sorted), buf(&queries), MappedBuffer::zeroed::<u32>(&dev, queries.len()).unwrap(), buf(&[n as u32, queries.len() as u32]));
        run(|e| e.run("pb_search", None, &[&sb.buffer, &counts.buffer, &qb.buffer, &counts.buffer, &out.buffer], &[0, 1], Grid::Groups((queries.len() as u64).div_ceil(TG))));
        let want: Vec<u32> = queries.iter().map(|&q| sorted.partition_point(|&s| s < q) as u32).collect();
        assert_eq!(read::<u32>(&out, queries.len()), want, "search, n = {n}");
        // Expand: items with counts 0 to 3, their offsets an exclusive scan.
        let item_counts: Vec<u32> = (0..n).map(|_| (r.next() % 4) as u32).collect();
        let mut acc = 0u32;
        let offsets: Vec<u32> = item_counts.iter().map(|&c| { let a = acc; acc += c; a }).collect();
        let total = acc as usize;
        let (ob, nb, tb) = (buf(&offsets), buf(&[n as u32]), buf(&[acc]));
        let (item, rank) = (MappedBuffer::zeroed::<u32>(&dev, total.max(1)).unwrap(), MappedBuffer::zeroed::<u32>(&dev, total.max(1)).unwrap());
        run(|e| e.run("pb_expand", None, &[&ob.buffer, &nb.buffer, &tb.buffer, &item.buffer, &rank.buffer], &[0], Grid::Groups((total as u64).div_ceil(TG))));
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

/// R9 of decision 0448: an evaluation is one command buffer, so the host never reads between its
/// passes. `pp_eval::run` takes a whole evaluation's encoding (an expansion's included) and
/// refuses one that used a second buffer.
#[test]
fn an_evaluation_is_one_command_buffer() {
    let device = crate::device().unwrap();
    let (sorted, queries, out) = (buf(&[1u64, 3]), buf(&[2u64]), buf(&[0u32]));
    let counts = buf(&[2u32, 1]);
    let search = |enc: &Enc<'_>| enc.run("pb_search", None, &[&sorted.buffer, &counts.buffer, &queries.buffer, &counts.buffer, &out.buffer], &[0, 1], Grid::Groups(1));
    let one = crate::pp_eval::run(|enc| search(enc));
    assert_eq!(one.unwrap(), 1);
    let two = crate::pp_eval::run(|enc| {
        search(enc)?;
        // A second buffer, waited on mid-evaluation: a host read between passes.
        let queue = crate::pool::queue(&device)?;
        let other = queue.new_command_buffer();
        search(&Enc { command: other, source: source()? })?;
        commit_checked(other)?;
        wait_checked(other)
    });
    assert!(two.is_err(), "a second command buffer in one evaluation");
}

/// A measurement, not a check: the grid's sort alone, a few keys, one group.
#[test]
#[ignore = "a measurement"]
fn the_grid_sort_alone_is_fast() {
    let dev = crate::device().unwrap();
    // Hashed keys (a definitions' rows), and the closes' (a depth by 2^15 over a position).
    let hashed = |n: usize| (0..n as u64).map(|k| k.wrapping_mul(0x9e37_79b9_7f4a_7c15)).collect::<Vec<u64>>();
    let closes = |n: usize| (0..n as u64).map(|k| ((32767 + k % 5) << 24) | (k * 7)).collect::<Vec<u64>>();
    for (what, n, low, bits) in [("hashed", 500usize, 0u32, 64u32), ("hashed", 1000, 0, 64), ("closes", 1600, 0, 40), ("closes", 1600, 24, 40), ("closes", 6000, 24, 40)] {
        let keys = if what == "hashed" { hashed(n) } else { closes(n) };
        for groups in [1u64, 32] {
            let z32 = |k: usize| MappedBuffer::zeroed::<u32>(&dev, k).unwrap();
            let t = (0..5)
                .map(|_| {
                    let (ctl, kb, va, k2, v2, hist, sums) = (z32(8), buf(&keys), z32(n), MappedBuffer::zeroed::<u64>(&dev, n).unwrap(), z32(n), z32(256 * n.div_ceil(1024) + 1), z32(4 * groups as usize + 4));
                    crate::pp_eval::run_timed(|e| e.run("pb_sort_alone", None, &[&ctl.buffer, &kb.buffer, &va.buffer, &k2.buffer, &v2.buffer, &hist.buffer, &sums.buffer], &[n as u32, bits, low], Grid::Groups(groups)))
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
    let dev = crate::device().unwrap();
    let names = ["grid barrier", "device fence", "tg barrier (device)", "coherent load chain", "plain load chain"];
    for what in 0..5u32 {
        for groups in [1u64, 32] {
            let time = |n: u32| {
                (0..5)
                    .map(|_| {
                        let (ctl, data) = (MappedBuffer::zeroed::<u32>(&dev, 8).unwrap(), MappedBuffer::zeroed::<u32>(&dev, 512).unwrap());
                        crate::pp_eval::run_timed(|e| e.run("pb_barriers_alone", None, &[&ctl.buffer, &data.buffer], &[n, what], Grid::Groups(groups))).unwrap().1
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

/// The evaluator's source compiles (a failed compile is fatal, so its message is shown here).
#[test]
fn the_source_compiles() {
    let device = crate::device().unwrap();
    let src = source().unwrap();
    if let Err(e) = device.new_library_with_source(src, &metal::CompileOptions::new()) {
        let lines: Vec<&str> = e.lines().filter(|l| l.contains("error")).take(12).collect();
        panic!("{}", lines.join("\n"));
    }
}
