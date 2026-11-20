//! The UKL library's tests, on the process's device (decision 0440 §5, step 5's gate): each primitive equals its Rust
//! reference on random inputs and on edge sizes; each backend form equals the library's reference form. They run on
//! Metal (macOS) and CUDA (Linux) alike; without a device they pass, saying so.

use crate::device::ComputeQueue;
use crate::kernels::{alloc, buffer_of, launch, program, read, Program};
use crate::library::{compact_u32, histogram, radix_sort_pairs, scan_exclusive_popcount_u32, scan_exclusive_u32, HashTable, RadixKey, Sorted};
use crate::Pod;

const TESTS: &str = include_str!("library_tests.ukl");

#[repr(C)]
#[derive(Clone, Copy)]
struct TestArgs {
    n: u32,
    w: u32,
}
unsafe impl Pod for TestArgs {}

/// The tests' program and a queue, or `None` (said) without a device.
fn setup() -> Option<(&'static Program, ComputeQueue)> {
    let Some(shared) = crate::shared::get() else {
        eprintln!("no device: the UKL library's tests did not run");
        return None;
    };
    let p = program("library_tests.ukl", TESTS).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()));
    Some((p, ComputeQueue::new(&shared.queue)))
}

/// xorshift64*: the tests' inputs, the same on every run.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    /// Words with runs of zeros and ones as well as noise: the edge cases of bit searches.
    fn word(&mut self) -> u64 {
        match self.below(6) {
            0 => 0,
            1 => !0,
            2 => 1 << self.below(64),
            3 => !0 << self.below(64),
            4 => self.next() & self.next(),
            _ => self.next(),
        }
    }
}

fn mask(n: u32) -> u64 {
    if n >= 64 { !0 } else { (1u64 << n) - 1 }
}

/// Little-endian bytes from `a` up, as a u64 (past the end: zero).
fn le(b: &[u8], a: usize, n: usize) -> u64 {
    (0..n).map(|k| (*b.get(a + k).unwrap_or(&0) as u64) << (8 * k)).fold(0, |x, y| x | y)
}

#[test]
fn byte_loads_at_every_address() {
    let Some((p, mut q)) = setup() else { return };
    let mut rng = Rng(7);
    for n in [1u32, 7, 8, 9, 63, 1000] {
        for w in [1u32, 3, 8, 13, 31, 32, 33, 57] {
            // Room for the widest read from the last address, and the word that holds its last byte.
            let len = ((n as usize * w as usize + 7 + 64) / 8 + n as usize + 16).next_multiple_of(8);
            let bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
            let inb = buffer_of(&bytes).unwrap();
            let out = alloc::<u64>(n as usize * 6).unwrap();
            launch(&q, p.function("t_loads").unwrap()).value("args", &TestArgs { n, w }).buffer("bytes", &inb).buffer("out", &out).run(&[n.div_ceil(64) as u64], &[64]).unwrap();
            q.flush().await_().unwrap();
            let got: Vec<u64> = read(&out, n as usize * 6);
            for i in 0..n as usize {
                let field = |b: usize| {
                    let x = (0..16).fold(0u128, |x, k| x | (*bytes.get(b / 8 + k).unwrap_or(&0) as u128) << (8 * k));
                    ((x >> (b % 8)) as u64) & mask(w)
                };
                let want = [le(&bytes, i, 1), le(&bytes, i, 2), le(&bytes, i, 4), le(&bytes, i, 8), field(i * w as usize + i % 5), field(i * w as usize)];
                assert_eq!(&got[i * 6..i * 6 + 6], &want, "{}: loads at {i} (n {n}, w {w}): u8 u16 u32 u64 bits packed", crate::device::backend());
            }
        }
    }
}

#[test]
fn byte_stores_at_unaligned_addresses() {
    let Some((p, mut q)) = setup() else { return };
    let n = 200u32;
    let bytes = buffer_of(&vec![0u8; n as usize * 32]).unwrap();
    let out = alloc::<u64>(n as usize * 3).unwrap();
    launch(&q, p.function("t_stores").unwrap()).value("args", &TestArgs { n, w: 0 }).buffer("bytes", &bytes).buffer("out", &out).run(&[n.div_ceil(64) as u64], &[64]).unwrap();
    q.flush().await_().unwrap();
    let got: Vec<u64> = read(&out, n as usize * 3);
    let mem: Vec<u8> = read(&bytes, n as usize * 32);
    let mut want_mem = vec![0u8; n as usize * 32];
    for i in 0..n as usize {
        let v = (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0x0123_4567_89ab_cdef;
        let at = [i * 32 + i % 7, i * 32 + 8 + i % 5, i * 32 + 16 + i % 9 % 8];
        let vals = [v & 0xffff, (v >> 7) & 0xffff_ffff, v >> 3];
        for (k, (a, x)) in at.iter().zip(vals).enumerate() {
            let len = [2, 4, 8][k];
            want_mem[*a..a + len].copy_from_slice(&x.to_le_bytes()[..len]);
        }
        assert_eq!(&got[i * 3..i * 3 + 3], &vals, "{}: thread {i}'s stores, read back", crate::device::backend());
    }
    assert_eq!(mem, want_mem, "{}: the stores' bytes, and no others", crate::device::backend());
}

#[test]
fn bits_within_words() {
    let Some((p, mut q)) = setup() else { return };
    let mut rng = Rng(11);
    let n = 4096u32;
    let words: Vec<u64> = (0..n).map(|_| rng.word()).collect();
    let inb = buffer_of(&words).unwrap();
    let out = alloc::<u64>(n as usize * 12).unwrap();
    launch(&q, p.function("t_bits").unwrap()).value("args", &TestArgs { n, w: 0 }).buffer("words", &inb).buffer("out", &out).run(&[(n / 64) as u64], &[64]).unwrap();
    q.flush().await_().unwrap();
    let got: Vec<u64> = read(&out, n as usize * 12);
    let select = |w: u64, k: u32, width: u32| (0..width).filter(|b| w >> b & 1 == 1).nth(k as usize).unwrap_or(width) as u64;
    for (i, &w) in words.iter().enumerate() {
        let lo = w as u32;
        let i32_ = i as u32;
        let bits = |v: u64, off: u32, n: u32, width: u32| if off >= width { 0 } else { (v >> off) & mask(n) };
        let want = [
            lo.count_ones() as u64,
            w.count_ones() as u64,
            lo.leading_zeros() as u64,
            w.leading_zeros() as u64,
            lo.trailing_zeros() as u64,
            w.trailing_zeros() as u64,
            (lo as u64 & mask(i32_ % 33)).count_ones() as u64,
            (w & mask(i32_ % 65)).count_ones() as u64,
            select(lo as u64, i32_ % 33, 32),
            select(w, i32_ % 65, 64),
            bits(w, i32_ % 64, i32_ * 7 % 65, 64),
            bits(lo as u64, i32_ % 33, i32_ * 5 % 33, 32) & 0xffff_ffff,
        ];
        assert_eq!(&got[i * 12..i * 12 + 12], &want, "{}: word {i} ({w:#x}): popcount clz ctz (32, 64), rank, select, bits", crate::device::backend());
    }
}

#[test]
fn bitsets_search_and_set() {
    let Some((p, mut q)) = setup() else { return };
    let mut rng = Rng(13);
    for nbits in [1u32, 31, 32, 33, 100, 1024, 5000] {
        let nw = nbits.div_ceil(32) as usize;
        // Sparse, dense and empty stretches.
        let words: Vec<u32> = (0..nw).map(|k| if k % 7 == 3 { 0 } else { (rng.word() & rng.word()) as u32 }).collect();
        let is_set = |b: u32| b < nbits && words[b as usize / 32] >> (b % 32) & 1 == 1;
        let n = nbits + 2;
        let wb = buffer_of(&words).unwrap();
        let out = alloc::<u32>(n as usize).unwrap();
        launch(&q, p.function("t_bitset_next").unwrap()).value("args", &TestArgs { n, w: nbits }).buffer("words", &wb).buffer("out", &out).run(&[n.div_ceil(64) as u64], &[64]).unwrap();
        q.flush().await_().unwrap();
        let got: Vec<u32> = read(&out, n as usize);
        for i in 0..n {
            let want = (i..nbits).find(|&b| is_set(b)).unwrap_or(u32::MAX);
            assert_eq!(got[i as usize], want, "{}: next set bit from {i} of {nbits}", crate::device::backend());
        }

        // Setting: some bits twice; each distinct bit is "new" to exactly one thread.
        let sets: Vec<u32> = (0..n * 2).map(|_| rng.below(nbits as u64) as u32).collect();
        let sb = buffer_of(&sets).unwrap();
        let empty = buffer_of(&vec![0u32; nw]).unwrap();
        let was = alloc::<u32>(sets.len()).unwrap();
        let args = TestArgs { n: sets.len() as u32, w: nbits };
        launch(&q, p.function("t_bitset_set").unwrap()).value("args", &args).buffer("bits", &sb).buffer("words", &empty).buffer("out", &was).run(&[(sets.len() as u64).div_ceil(64)], &[64]).unwrap();
        q.flush().await_().unwrap();
        let mut want = vec![0u32; nw];
        for &b in &sets {
            want[b as usize / 32] |= 1 << (b % 32);
        }
        assert_eq!(read::<u32>(&empty, nw), want, "{}: the set bits", crate::device::backend());
        let fresh = read::<u32>(&was, sets.len()).iter().filter(|&&x| x == 0).count();
        let distinct = want.iter().map(|w| w.count_ones() as usize).sum::<usize>();
        assert_eq!(fresh, distinct, "{}: each bit new to one thread", crate::device::backend());
    }
}

#[test]
fn simd_group_operations_and_their_references() {
    let Some((p, mut q)) = setup() else { return };
    let mut rng = Rng(17);
    let groups = 64u32;
    let n = groups * 32;
    // Some groups of small values, some of large (sums wrap), some all equal.
    let vals: Vec<u32> = (0..n)
        .map(|i| match (i / 32) % 4 {
            0 => rng.below(10) as u32,
            1 => rng.next() as u32,
            2 => 5,
            _ => rng.below(1 << 20) as u32,
        })
        .collect();
    let inb = buffer_of(&vals).unwrap();
    let out = alloc::<u32>(n as usize * 16).unwrap();
    launch(&q, p.function("t_simd").unwrap()).value("args", &TestArgs { n, w: 0 }).buffer("in", &inb).buffer("out", &out).run(&[(n / 128) as u64], &[128]).unwrap();
    q.flush().await_().unwrap();
    let got: Vec<u32> = read(&out, n as usize * 16);
    // The device's SIMD groups: 32 lanes, or another width (the CPU's `RERAIL_CPU_SIMD_WIDTH`, an OpenCL device's).
    let w = crate::device::simd_width() as usize;
    for g in 0..n as usize / w {
        let v = &vals[g * w..g * w + w];
        let sum = v.iter().fold(0u32, |a, &b| a.wrapping_add(b));
        let odd: Vec<bool> = v.iter().map(|x| x & 1 == 1).collect();
        let first = v.iter().position(|x| x % 5 == 0).map_or(u32::MAX, |l| l as u32);
        for lane in 0..w {
            let incl = v[..=lane].iter().fold(0u32, |a, &b| a.wrapping_add(b));
            let below = odd[..lane].iter().filter(|&&b| b).count() as u32;
            let mn = *v.iter().min().unwrap();
            let mx = *v.iter().max().unwrap();
            let same = (0..w).filter(|&j| v[j] % 4 == v[lane] % 4).fold(0u32, |m, j| m | 1 << j);
            let want = [
                sum,
                sum,
                mn,
                mn,
                mx,
                mx,
                incl,
                incl,
                incl.wrapping_sub(v[lane]),
                incl.wrapping_sub(v[lane]),
                below,
                odd.iter().filter(|&&b| b).count() as u32,
                first,
                v[g % w],
                same,
                same,
            ];
            let i = g * w + lane;
            assert_eq!(&got[i * 16..i * 16 + 16], &want, "{}: group {g}, lane {lane}: sum min max (backend, reference), scans, votes, broadcast, match", crate::device::backend());
        }
    }
}

#[test]
fn threadgroup_sums_and_scans() {
    let Some((p, mut q)) = setup() else { return };
    let mut rng = Rng(19);
    for lsz in [32u32, 64, 96, 256, 1024].into_iter().filter(|&l| l <= crate::device::max_threadgroup()) {
        let groups = 5u32;
        let n = lsz * groups;
        let vals: Vec<u32> = (0..n).map(|_| if rng.below(3) == 0 { rng.next() as u32 } else { rng.below(100) as u32 }).collect();
        let inb = buffer_of(&vals).unwrap();
        let out = alloc::<u32>(n as usize * 3).unwrap();
        launch(&q, p.function("t_group").unwrap()).value("args", &TestArgs { n, w: 0 }).buffer("in", &inb).buffer("out", &out).run(&[groups as u64], &[lsz as u64]).unwrap();
        q.flush().await_().unwrap();
        let got: Vec<u32> = read(&out, n as usize * 3);
        for g in 0..groups as usize {
            let v = &vals[g * lsz as usize..(g + 1) * lsz as usize];
            let total = v.iter().fold(0u32, |a, &b| a.wrapping_add(b));
            let mut run = 0u32;
            for (t, &x) in v.iter().enumerate() {
                let i = g * lsz as usize + t;
                assert_eq!(&got[i * 3..i * 3 + 3], &[total, run, total], "{}: group of {lsz}, group {g}, thread {t}: sum, exclusive, total", crate::device::backend());
                run = run.wrapping_add(x);
            }
        }
    }
}

/// LZ4's block format, decoded as `ukl/lz4.h` decodes it: the reference for malformed blocks too.
fn lz4_reference(s: &[u8], cap: usize) -> Option<Vec<u8>> {
    let (mut ip, mut out) = (0usize, Vec::new());
    let length = |ip: &mut usize, n: &mut usize| -> bool {
        loop {
            let Some(&c) = s.get(*ip) else { return false };
            *ip += 1;
            *n += c as usize;
            if c != 255 {
                return true;
            }
        }
    };
    loop {
        let &tok = s.get(ip)?;
        ip += 1;
        let mut lit = (tok >> 4) as usize;
        if lit == 15 && !length(&mut ip, &mut lit) {
            return None;
        }
        if ip + lit > s.len() || out.len() + lit > cap {
            return None;
        }
        out.extend_from_slice(&s[ip..ip + lit]);
        ip += lit;
        if ip == s.len() {
            return Some(out);
        }
        if ip + 2 > s.len() {
            return None;
        }
        let dist = s[ip] as usize | (s[ip + 1] as usize) << 8;
        ip += 2;
        let mut ml = (tok & 15) as usize;
        if ml == 15 && !length(&mut ip, &mut ml) {
            return None;
        }
        ml += 4;
        if dist == 0 || dist > out.len() || out.len() + ml > cap {
            return None;
        }
        for _ in 0..ml {
            out.push(out[out.len() - dist]);
        }
    }
}

fn lz4_compress(src: &[u8]) -> Vec<u8> {
    let cap = unsafe { lz4_sys::LZ4_compressBound(src.len() as i32) } as usize;
    let mut out = vec![0u8; cap];
    let k = unsafe { lz4_sys::LZ4_compress_default(src.as_ptr().cast(), out.as_mut_ptr().cast(), src.len() as i32, cap as i32) };
    assert!(k > 0 || src.is_empty(), "LZ4 compressed nothing");
    out.truncate(k.max(0) as usize);
    out
}

#[test]
fn lz4_blocks_decode_as_lz4_does() {
    let Some((p, mut q)) = setup() else { return };
    let mut rng = Rng(23);
    // Data with matches at every distance from 1 (runs) to far, short and long (extended lengths), and noise.
    let mut make = |len: usize| -> Vec<u8> {
        let mut v: Vec<u8> = Vec::with_capacity(len);
        while v.len() < len {
            match rng.below(4) {
                0 if !v.is_empty() => {
                    let far = [1, 3, 31, 33, 600][rng.below(5) as usize].min(v.len() as u64);
                    let dist = 1 + rng.below(far) as usize;
                    let ml = [4, 7, 18, 19, 300, 2000][rng.below(6) as usize];
                    for _ in 0..ml {
                        v.push(v[v.len() - dist]);
                    }
                }
                1 => v.extend(std::iter::repeat_n(rng.next() as u8, rng.below(40) as usize)),
                _ => v.extend((0..rng.below(300)).map(|_| rng.next() as u8)),
            }
        }
        v.truncate(len);
        v
    };
    let mut flip = Rng(37);
    let mut blocks: Vec<(Vec<u8>, usize)> = Vec::new();
    for len in [1usize, 5, 13, 31, 32, 33, 100, 1000, 4096, 70000] {
        let plain = make(len);
        let z = lz4_compress(&plain);
        assert_eq!(lz4_reference(&z, len).as_deref(), Some(&plain[..]), "the reference decodes LZ4's block of {len}");
        blocks.push((z.clone(), len));
        // Malformed: cut short, a capacity too small, a byte changed.
        blocks.push((z[..z.len() * 2 / 3].to_vec(), len));
        if len > 1 {
            blocks.push((z.clone(), len - 1));
        }
        let mut bad = z.clone();
        let k = flip.below(bad.len() as u64) as usize;
        bad[k] ^= 0x5a;
        blocks.push((bad, len));
    }
    let (mut jobs, mut src, mut out_len) = (Vec::new(), Vec::new(), 0usize);
    for (z, cap) in &blocks {
        jobs.extend([src.len() as u32, z.len() as u32, out_len as u32, *cap as u32]);
        src.extend_from_slice(z);
        out_len += cap;
    }
    let n = blocks.len() as u32;
    let (jb, sb) = (buffer_of(&jobs).unwrap(), buffer_of(&src).unwrap());
    let ob = buffer_of(&vec![0u8; out_len]).unwrap();
    let lb = alloc::<u32>(blocks.len()).unwrap();
    launch(&q, p.function("t_lz4").unwrap()).value("args", &TestArgs { n, w: 0 }).buffer("jobs", &jb).buffer("in", &sb).buffer("outData", &ob).buffer("lengths", &lb).run(&[n as u64], &[32]).unwrap();
    q.flush().await_().unwrap();
    let lengths: Vec<u32> = read(&lb, blocks.len());
    let out: Vec<u8> = read(&ob, out_len);
    for (j, (z, cap)) in blocks.iter().enumerate() {
        let at = jobs[j * 4 + 2] as usize;
        match lz4_reference(z, *cap) {
            Some(want) => {
                assert_eq!(lengths[j] as usize, want.len(), "{}: block {j}'s length", crate::device::backend());
                assert!(out[at..at + want.len()] == want[..], "{}: block {j}'s bytes", crate::device::backend());
            }
            None => assert_eq!(lengths[j], u32::MAX, "{}: block {j} is malformed", crate::device::backend()),
        }
    }
}

#[test]
fn device_wide_scans_and_compaction() {
    let Some((_, mut q)) = setup() else { return };
    let mut rng = Rng(29);
    // Edge sizes: empty, one, a tile and either side, past the second level (1024 tiles), and wrapping sums.
    for n in [0u32, 1, 2, 1023, 1024, 1025, 4097, 1 << 20, (1 << 20) + 1025] {
        let big = n % 2 == 1;
        let vals: Vec<u32> = (0..n).map(|_| if big { rng.next() as u32 } else { rng.below(3) as u32 }).collect();
        let inb = buffer_of(&vals).unwrap();
        let out = alloc::<u32>(n as usize + 1).unwrap();
        scan_exclusive_u32(&q, &inb, &out, n).unwrap();
        q.flush().await_().unwrap();
        let got: Vec<u32> = read(&out, n as usize + 1);
        let mut run = 0u32;
        for i in 0..=n as usize {
            assert_eq!(got[i], run, "{}: scan of {n}, at {i}", crate::device::backend());
            if i < n as usize {
                run = run.wrapping_add(vals[i]);
            }
        }

        // The same values as bitset words: each word's first rank.
        let out = alloc::<u32>(n as usize + 1).unwrap();
        scan_exclusive_popcount_u32(&q, &inb, &out, n).unwrap();
        q.flush().await_().unwrap();
        let got: Vec<u32> = read(&out, n as usize + 1);
        let mut rank = 0u32;
        for i in 0..=n as usize {
            assert_eq!(got[i], rank, "{}: popcount scan of {n}, at {i}", crate::device::backend());
            if i < n as usize {
                rank += vals[i].count_ones();
            }
        }

        let flags: Vec<u32> = (0..n).map(|i| (vals[i as usize] % 2 == 0) as u32).collect();
        let fb = buffer_of(&flags).unwrap();
        let (kept, offsets) = (alloc::<u32>(n as usize).unwrap(), alloc::<u32>(n as usize + 1).unwrap());
        compact_u32(&q, &inb, &fb, &kept, &offsets, n).unwrap();
        q.flush().await_().unwrap();
        let want: Vec<u32> = vals.iter().copied().filter(|v| v % 2 == 0).collect();
        let count = read::<u32>(&offsets, n as usize + 1)[n as usize] as usize;
        assert_eq!(count, want.len(), "{}: compaction of {n} kept", crate::device::backend());
        assert!(read::<u32>(&kept, count) == want, "{}: compaction of {n}'s values, in order", crate::device::backend());
    }
}

#[test]
fn histograms_by_partition() {
    let Some((_, mut q)) = setup() else { return };
    let mut rng = Rng(31);
    // Widths within one table, across windows, and a partition's codes all past the first window.
    for (width, parts) in [(1u32, 3u32), (100, 7), (2048, 2), (2049, 3), (5000, 4)] {
        let mut codes: Vec<u16> = Vec::new();
        let mut offsets = vec![0u32];
        for p in 0..parts {
            let len = [0, 1, 300, 5000][(p % 4) as usize];
            codes.extend((0..len).map(|_| if p == 3 { (width - 1 - rng.below(width.min(10) as u64) as u32) as u16 } else { rng.below(width as u64) as u16 }));
            offsets.push(codes.len() as u32);
        }
        for invalid in [false, true] {
            let mut codes = codes.clone();
            if invalid && width < 65535 {
                codes.push(width as u16);
                *offsets.last_mut().unwrap() += 1;
            }
            let (cb, ob) = (buffer_of(&codes).unwrap(), buffer_of(&offsets).unwrap());
            let h = alloc::<u32>((parts * width) as usize).unwrap();
            let bad = buffer_of(&[0u32]).unwrap();
            histogram::<u16>(&q, &cb, &ob, parts, width, &h, &bad).unwrap();
            q.flush().await_().unwrap();
            let mut want = vec![0u32; (parts * width) as usize];
            for p in 0..parts as usize {
                for &c in &codes[offsets[p] as usize..offsets[p + 1] as usize] {
                    if (c as u32) < width {
                        want[p * width as usize + c as usize] += 1;
                    }
                }
            }
            assert!(read::<u32>(&h, want.len()) == want, "{}: histogram of width {width}, {parts} partitions", crate::device::backend());
            assert_eq!(read::<u32>(&bad, 1)[0], invalid as u32, "{}: a code past the width is flagged", crate::device::backend());
            if invalid {
                *offsets.last_mut().unwrap() -= 1;
            }
        }
    }
}

/// Sorted by the low `bits` bits of each key, stably: the values (each row's first place) of equal keys in order.
fn sort_and_check<K: RadixKey + Ord + std::fmt::Debug + Into<u64> + From<u32>>(q: &mut ComputeQueue, keys: &[K], bits: u32) {
    let n = keys.len() as u32;
    let values: Vec<u32> = (0..n).collect();
    let (kb, vb) = (buffer_of(keys).unwrap(), buffer_of(&values).unwrap());
    let (tk, tv) = (alloc::<K>(keys.len()).unwrap(), alloc::<u32>(keys.len()).unwrap());
    let sorted = radix_sort_pairs::<K>(q, &kb, &vb, &tk, &tv, n, bits).unwrap();
    q.flush().await_().unwrap();
    assert_eq!(sorted, if bits.div_ceil(8) % 2 == 0 { Sorted::InPlace } else { Sorted::InTemp });
    let (rk, rv) = if sorted == Sorted::InPlace { (&kb, &vb) } else { (&tk, &tv) };
    let (got_k, got_v): (Vec<K>, Vec<u32>) = (read(rk, keys.len()), read(rv, keys.len()));
    let mut want: Vec<u32> = values.clone();
    let low = |k: K| k.into() & mask(bits);
    want.sort_by_key(|&i| low(keys[i as usize]));
    let want_k: Vec<K> = want.iter().map(|&i| keys[i as usize]).collect();
    assert!(got_v == want, "{}: the sort of {n} keys by {bits} bits is stable", crate::device::backend());
    assert!(got_k == want_k, "{}: the keys go with their values", crate::device::backend());
}

#[test]
fn radix_sorts_are_stable() {
    let Some((_, mut q)) = setup() else { return };
    if crate::device::simd_width() < 8 {
        eprintln!("SIMD groups of {} lanes: no radix sort (ukl/radix.h): skipped", crate::device::simd_width());
        return;
    }
    let mut rng = Rng(41);
    for n in [0usize, 1, 31, 1023, 1024, 1025, 70_000] {
        // Few distinct keys (long runs: stability), and keys over all their bits.
        for (bits, distinct) in [(8u32, 5u64), (24, 1 << 24), (32, 1000), (32, 1 << 32)] {
            let keys: Vec<u32> = (0..n).map(|_| (rng.below(distinct) as u32) & mask(bits) as u32).collect();
            sort_and_check(&mut q, &keys, bits);
        }
        for bits in [40u32, 64] {
            let keys: Vec<u64> = (0..n).map(|_| if rng.below(2) == 0 { rng.below(7) } else { rng.next() & mask(bits) }).collect();
            sort_and_check(&mut q, &keys, bits);
        }
    }
}

/// The tree walk of `nodes` by `kernel`'s workers, launched again while items remain; the launches.
fn walk(p: &Program, q: &mut ComputeQueue, kernel: &str, nodes: u32, polls: u32) -> u32 {
    {
        let capacity = 4 * nodes as usize;
        let mut slots = vec![0u32; capacity];
        slots[0] = 1; // node 0, pushed by the host
        let (sb, cb) = (buffer_of(&slots).unwrap(), buffer_of(&[0u32, 1, 1, 0]).unwrap());
        let vb = buffer_of(&vec![0u32; nodes as usize]).unwrap();
        // Workers that give up early leave items to the next launch.
        let mut launches = 0;
        loop {
            launches += 1;
            assert!(launches <= 1000, "{}: the work queue of {nodes} did not drain", crate::device::backend());
            launch(q, p.function(kernel).unwrap())
                .value("args", &TestArgs { n: polls, w: nodes })
                .buffer("slots", &sb)
                .buffer("control", &cb)
                .buffer("visits", &vb)
                .run(&[64], &[64])
                .unwrap();
            q.flush().await_().unwrap();
            if read::<u32>(&cb, 4)[2] == 0 {
                break;
            }
        }
        let control: Vec<u32> = read(&cb, 4);
        assert_eq!(control[3], 0, "{}: the queue of {nodes} overflowed", crate::device::backend());
        let visits: Vec<u32> = read(&vb, nodes as usize);
        let wrong = visits.iter().position(|&v| v != 1);
        assert_eq!(wrong, None, "{}: {kernel}: every node visited once ({nodes} nodes, {launches} launches)", crate::device::backend());
        launches
    }
}

#[test]
fn a_work_queue_visits_every_item_once() {
    let Some((p, mut q)) = setup() else { return };
    // A SIMD group at a time: the form for many workers.
    for (nodes, polls) in [(1u32, 64u32), (1000, 64), (100_000, 64), (100_000, 2)] {
        walk(p, &mut q, "t_workqueue_sg", nodes, polls);
    }
    // A thread at a time: each item a swap that every worker contends for, so few items.
    for (nodes, polls) in [(1u32, 64u32), (300, 64), (300, 1)] {
        walk(p, &mut q, "t_workqueue", nodes, polls);
    }
}

#[test]
fn hash_tables_build_and_probe() {
    let Some((_, mut q)) = setup() else { return };
    let mut rng = Rng(43);
    for (n, distinct, capacity) in [(0usize, 1u64, 16u32), (1, 1, 16), (1000, 50, 128), (100_000, 30_000, 1 << 16), (100_000, 1 << 32, 1 << 18)] {
        let rows: Vec<u32> = (0..n).map(|_| (rng.below(distinct) as u32).min(u32::MAX - 1)).collect();
        let table = HashTable::new(capacity).unwrap();
        let rb = buffer_of(&rows).unwrap();
        table.build(&q, &rb, n as u32).unwrap();
        // Queries: every row's key, and keys that are absent.
        let mut queries = rows.clone();
        queries.extend((0..1000).map(|_| rng.next() as u32 | 0x8000_0000).filter(|k| *k != u32::MAX));
        let qb = buffer_of(&queries).unwrap();
        let (fo, co) = (alloc::<u32>(queries.len()).unwrap(), alloc::<u32>(queries.len()).unwrap());
        table.probe(&q, &qb, queries.len() as u32, &table.first, &fo).unwrap();
        table.probe(&q, &qb, queries.len() as u32, &table.counts, &co).unwrap();
        q.flush().await_().unwrap();
        assert_eq!(read::<u32>(&table.full, 1)[0], 0, "{}: {n} rows fit {capacity} slots", crate::device::backend());
        let mut want: std::collections::HashMap<u32, (u32, u32)> = Default::default();
        for (i, &k) in rows.iter().enumerate() {
            want.entry(k).or_insert((i as u32, 0)).1 += 1;
        }
        let (first, counts): (Vec<u32>, Vec<u32>) = (read(&fo, queries.len()), read(&co, queries.len()));
        for (i, k) in queries.iter().enumerate() {
            let (f, c) = want.get(k).copied().unwrap_or((u32::MAX, u32::MAX));
            assert_eq!((first[i], counts[i]), (f, c), "{}: key {k:#x} ({n} rows): first row, count", crate::device::backend());
        }
    }

    // A table too small: reported, not looped on; the keys it holds are each in it once.
    let rows: Vec<u32> = (0..200).collect();
    let table = HashTable::new(64).unwrap();
    let rb = buffer_of(&rows).unwrap();
    table.build(&q, &rb, 200).unwrap();
    q.flush().await_().unwrap();
    assert_eq!(read::<u32>(&table.full, 1)[0], 1, "{}: a full table is reported", crate::device::backend());
    let mut keys: Vec<u32> = read(&table.keys, 64);
    keys.sort();
    keys.dedup();
    assert_eq!(keys.len(), 64, "{}: each slot holds a distinct key", crate::device::backend());
}

/// A grid's control words, zero (`ukl/grid.h`).
fn grid_ctl() -> crate::device::Buffer {
    buffer_of(&[0u32; 3]).unwrap()
}

/// R6 of decision 0468: 10,000 grid barriers in a row, at 1, 8 and 32 groups, each round's reads seeing every
/// group's writes before it, and no barrier failed.
#[test]
fn grid_barriers_see_every_groups_writes() {
    let Some((p, mut q)) = setup() else { return };
    let lsz = 256u32.min(crate::device::max_threadgroup());
    for groups in [1u32, 8, 32] {
        let rounds = 5_000u32;
        let (ctl, slots, out) = (grid_ctl(), alloc::<u32>((groups * lsz) as usize).unwrap(), buffer_of(&vec![0u32; groups as usize]).unwrap());
        launch(&q, p.function("t_grid_barrier").unwrap())
            .value("args", &TestArgs { n: rounds, w: groups })
            .buffer("ctl", &ctl)
            .buffer("slots", &slots)
            .buffer("out", &out)
            .run(&[groups as u64], &[lsz as u64])
            .unwrap();
        q.flush().await_().unwrap();
        let c: Vec<u32> = read(&ctl, 3);
        let wrong: Vec<u32> = read(&out, groups as usize);
        assert_eq!(c[2], 0, "{}: {groups} groups: a barrier failed", crate::device::backend());
        assert_eq!(c[1], 4 * rounds, "{}: {groups} groups: the barriers' generation (two a barrier)", crate::device::backend());
        assert!(wrong.iter().all(|&w| w == 0), "{}: {groups} groups: reads before the barrier's writes: {wrong:?}", crate::device::backend());
    }
}

/// R6: a grid past what the device holds at once fails its barrier (the failure word set), never hangs: the groups
/// that run wait their bound, and those that start after see the failure and end.
#[test]
fn a_grid_past_residency_fails_its_barrier() {
    let Some((p, mut q)) = setup() else { return };
    let lsz = 1024u32.min(crate::device::max_threadgroup());
    let groups = 4096u32;
    let (ctl, slots, out) = (grid_ctl(), alloc::<u32>((groups * lsz) as usize).unwrap(), buffer_of(&vec![0u32; groups as usize]).unwrap());
    let start = std::time::Instant::now();
    launch(&q, p.function("t_grid_barrier").unwrap())
        .value("args", &TestArgs { n: 1, w: groups })
        .buffer("ctl", &ctl)
        .buffer("slots", &slots)
        .buffer("out", &out)
        .run(&[groups as u64], &[lsz as u64])
        .unwrap();
    q.flush().await_().unwrap();
    let c: Vec<u32> = read(&ctl, 3);
    eprintln!("{}: {groups} groups of {lsz}: failed {} after {:?}", crate::device::backend(), c[2], start.elapsed());
    assert_eq!(c[2], 1, "{}: a grid of {groups} groups of {lsz} met its barrier", crate::device::backend());
}

/// R5 of decision 0468: the grid's exclusive scan equals the host's from 0 to 10^6 values, across tile and share
/// boundaries, at 1, 8 and 32 groups, its count read on the device.
#[test]
fn the_grid_scan_equals_the_host() {
    let Some((p, mut q)) = setup() else { return };
    let mut rng = Rng(41);
    let lsz = 256u32.min(crate::device::max_threadgroup());
    for groups in [1u32, 8, 32] {
        for n in [0u32, 1, 255, 256, 257, 8 * 256 + 3, 65_537, 1_000_000] {
            let vals: Vec<u32> = (0..n).map(|_| if n % 2 == 1 { rng.next() as u32 } else { rng.below(5) as u32 }).collect();
            // The buffers have room past the count: the scan reads `count`, not their length.
            let cap = n + 100;
            let mut padded = vals.clone();
            padded.resize(cap as usize, 7);
            let (ctl, count, inb, out, sums) = (grid_ctl(), buffer_of(&[n]).unwrap(), buffer_of(&padded).unwrap(), alloc::<u32>(cap as usize + 1).unwrap(), alloc::<u32>(groups as usize + 1).unwrap());
            launch(&q, p.function("t_grid_scan").unwrap())
                .value("args", &TestArgs { n: cap, w: groups })
                .buffer("ctl", &ctl)
                .buffer("count", &count)
                .buffer("in", &inb)
                .buffer("out", &out)
                .buffer("sums", &sums)
                .run(&[groups as u64], &[lsz as u64])
                .unwrap();
            q.flush().await_().unwrap();
            assert_eq!(read::<u32>(&ctl, 3)[2], 0, "{}: {groups} groups, {n}: a barrier failed", crate::device::backend());
            let got: Vec<u32> = read(&out, n as usize + 1);
            let mut run = 0u32;
            for i in 0..=n as usize {
                assert_eq!(got[i], run, "{}: {groups} groups, scan of {n}, at {i}", crate::device::backend());
                if i < n as usize {
                    run = run.wrapping_add(vals[i]);
                }
            }
        }
    }
}

/// R5: the grid's segmented scans (add and max) equal the host's from 0 to 10^6 values, heads dense and sparse, at
/// 1, 8 and 32 groups.
#[test]
fn the_grid_segmented_scans_equal_the_host() {
    let Some((p, mut q)) = setup() else { return };
    let mut rng = Rng(43);
    let lsz = 256u32.min(crate::device::max_threadgroup());
    for groups in [1u32, 8, 32] {
        for n in [0u32, 1, 255, 257, 8 * 256 + 3, 65_537, 1_000_000] {
            for op in [0u32, 1] {
                let vals: Vec<u32> = (0..n).map(|_| rng.below(1000) as u32).collect();
                let every = [2u64, 50, 100_000][rng.below(3) as usize];
                let heads: Vec<u32> = (0..n).map(|_| u32::from(rng.below(every) == 0)).collect();
                let (ctl, count, inb, hb, out, sums) = (grid_ctl(), buffer_of(&[n]).unwrap(), buffer_of(&vals).unwrap(), buffer_of(&heads).unwrap(), alloc::<u32>(n as usize).unwrap(), alloc::<u32>(2 * groups as usize).unwrap());
                launch(&q, p.function("t_grid_seg").unwrap())
                    .value("args", &TestArgs { n: n.max(1), w: groups | op << 16 })
                    .buffer("ctl", &ctl)
                    .buffer("count", &count)
                    .buffer("in", &inb)
                    .buffer("heads", &hb)
                    .buffer("out", &out)
                    .buffer("sums", &sums)
                    .run(&[groups as u64], &[lsz as u64])
                    .unwrap();
                q.flush().await_().unwrap();
                assert_eq!(read::<u32>(&ctl, 3)[2], 0, "{}: {groups} groups, {n}: a barrier failed", crate::device::backend());
                let got: Vec<u32> = read(&out, n as usize);
                let mut run = 0u32;
                for i in 0..n as usize {
                    run = if heads[i] != 0 || i == 0 { vals[i] } else if op == 0 { run.wrapping_add(vals[i]) } else { run.max(vals[i]) };
                    assert_eq!(got[i], run, "{}: {groups} groups, op {op}, segmented scan of {n}, at {i}", crate::device::backend());
                }
            }
        }
    }
}

/// R5: the grid's sort is stable and equals the host's from 0 to 10^6 keys, at 8 to 64 bits, at 1, 8 and 32 groups.
#[test]
fn the_grid_sort_is_stable_and_equals_the_host() {
    let Some((p, mut q)) = setup() else { return };
    if crate::device::max_threadgroup() < 256 {
        return;
    }
    let mut rng = Rng(47);
    for groups in [1u32, 8, 32] {
        for (n, bits) in [(0u32, 8u32), (1, 8), (1023, 16), (1025, 24), (70_001, 40), (1_000_000, 64)] {
            let keys: Vec<u64> = (0..n).map(|_| rng.next() & if bits == 64 { !0 } else { (1u64 << bits) - 1 } & if rng.below(2) == 0 { 0xff } else { !0 }).collect();
            let tiles = n.div_ceil(1024).max(1);
            let (ctl, count, kb, vb, k2, v2) = (grid_ctl(), buffer_of(&[n]).unwrap(), buffer_of(&keys).unwrap(), alloc::<u32>(n as usize).unwrap(), alloc::<u64>(n as usize).unwrap(), alloc::<u32>(n as usize).unwrap());
            let (hist, sums) = (alloc::<u32>(256 * tiles as usize + 1).unwrap(), alloc::<u32>(groups as usize + 1).unwrap());
            launch(&q, p.function("t_grid_sort").unwrap())
                .value("args", &TestArgs { n: n.max(1), w: groups | bits << 16 })
                .buffer("ctl", &ctl)
                .buffer("count", &count)
                .buffer("keys", &kb)
                .buffer("values", &vb)
                .buffer("keys2", &k2)
                .buffer("values2", &v2)
                .buffer("hist", &hist)
                .buffer("sums", &sums)
                .run(&[groups as u64], &[256])
                .unwrap();
            q.flush().await_().unwrap();
            assert_eq!(read::<u32>(&ctl, 3)[2], 0, "{}: {groups} groups, {n}: a barrier failed", crate::device::backend());
            let even = bits.div_ceil(8) % 2 == 0;
            let (gk, gv): (Vec<u64>, Vec<u32>) = if even { (read(&kb, n as usize), read(&vb, n as usize)) } else { (read(&k2, n as usize), read(&v2, n as usize)) };
            let mut want: Vec<u32> = (0..n).collect();
            want.sort_by_key(|&i| keys[i as usize]);
            assert_eq!(gv, want, "{}: {groups} groups, sort of {n} by {bits} bits: the order", crate::device::backend());
            assert!(gk.iter().zip(&want).all(|(k, &i)| *k == keys[i as usize]), "{}: {groups} groups, {n}: the keys", crate::device::backend());
        }
    }
}
