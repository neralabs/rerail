//! The UKL library's tests, on the process's device (decision 0400 §5, step 5's gate): each primitive equals its Rust
//! reference on random inputs and on edge sizes; each backend form equals the library's reference form. They run on
//! Metal (macOS) and CUDA (Linux) alike; without a device they pass, saying so.

use crate::device::{ComputeQueue, BACKEND};
use crate::kernels::{alloc, buffer_of, launch, program, read, Program};
use crate::library::{compact_u32, histogram, radix_sort_pairs, scan_exclusive_u32, RadixKey, Sorted};
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
    let p = program("library_tests.ukl", TESTS).unwrap_or_else(|e| panic!("{BACKEND}: {e}"));
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
                assert_eq!(&got[i * 6..i * 6 + 6], &want, "{BACKEND}: loads at {i} (n {n}, w {w}): u8 u16 u32 u64 bits packed");
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
        assert_eq!(&got[i * 3..i * 3 + 3], &vals, "{BACKEND}: thread {i}'s stores, read back");
    }
    assert_eq!(mem, want_mem, "{BACKEND}: the stores' bytes, and no others");
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
        assert_eq!(&got[i * 12..i * 12 + 12], &want, "{BACKEND}: word {i} ({w:#x}): popcount clz ctz (32, 64), rank, select, bits");
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
            assert_eq!(got[i as usize], want, "{BACKEND}: next set bit from {i} of {nbits}");
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
        assert_eq!(read::<u32>(&empty, nw), want, "{BACKEND}: the set bits");
        let fresh = read::<u32>(&was, sets.len()).iter().filter(|&&x| x == 0).count();
        let distinct = want.iter().map(|w| w.count_ones() as usize).sum::<usize>();
        assert_eq!(fresh, distinct, "{BACKEND}: each bit new to one thread");
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
    for g in 0..groups as usize {
        let v = &vals[g * 32..g * 32 + 32];
        let sum = v.iter().fold(0u32, |a, &b| a.wrapping_add(b));
        let odd: Vec<bool> = v.iter().map(|x| x & 1 == 1).collect();
        let first = v.iter().position(|x| x % 5 == 0).map_or(u32::MAX, |l| l as u32);
        for lane in 0..32 {
            let incl = v[..=lane].iter().fold(0u32, |a, &b| a.wrapping_add(b));
            let below = odd[..lane].iter().filter(|&&b| b).count() as u32;
            let mn = *v.iter().min().unwrap();
            let mx = *v.iter().max().unwrap();
            let same = (0..32).filter(|&j| v[j] % 4 == v[lane] % 4).fold(0u32, |m, j| m | 1 << j);
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
                v[g % 32],
                same,
                same,
            ];
            let i = g * 32 + lane;
            assert_eq!(&got[i * 16..i * 16 + 16], &want, "{BACKEND}: group {g}, lane {lane}: sum min max (backend, reference), scans, votes, broadcast, match");
        }
    }
}

#[test]
fn threadgroup_sums_and_scans() {
    let Some((p, mut q)) = setup() else { return };
    let mut rng = Rng(19);
    for lsz in [32u32, 64, 96, 256, 1024] {
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
                assert_eq!(&got[i * 3..i * 3 + 3], &[total, run, total], "{BACKEND}: group of {lsz}, group {g}, thread {t}: sum, exclusive, total");
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
                assert_eq!(lengths[j] as usize, want.len(), "{BACKEND}: block {j}'s length");
                assert!(out[at..at + want.len()] == want[..], "{BACKEND}: block {j}'s bytes");
            }
            None => assert_eq!(lengths[j], u32::MAX, "{BACKEND}: block {j} is malformed"),
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
            assert_eq!(got[i], run, "{BACKEND}: scan of {n}, at {i}");
            if i < n as usize {
                run = run.wrapping_add(vals[i]);
            }
        }

        let flags: Vec<u32> = (0..n).map(|i| (vals[i as usize] % 2 == 0) as u32).collect();
        let fb = buffer_of(&flags).unwrap();
        let (kept, offsets) = (alloc::<u32>(n as usize).unwrap(), alloc::<u32>(n as usize + 1).unwrap());
        compact_u32(&q, &inb, &fb, &kept, &offsets, n).unwrap();
        q.flush().await_().unwrap();
        let want: Vec<u32> = vals.iter().copied().filter(|v| v % 2 == 0).collect();
        let count = read::<u32>(&offsets, n as usize + 1)[n as usize] as usize;
        assert_eq!(count, want.len(), "{BACKEND}: compaction of {n} kept");
        assert!(read::<u32>(&kept, count) == want, "{BACKEND}: compaction of {n}'s values, in order");
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
            assert!(read::<u32>(&h, want.len()) == want, "{BACKEND}: histogram of width {width}, {parts} partitions");
            assert_eq!(read::<u32>(&bad, 1)[0], invalid as u32, "{BACKEND}: a code past the width is flagged");
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
    assert!(got_v == want, "{BACKEND}: the sort of {n} keys by {bits} bits is stable");
    assert!(got_k == want_k, "{BACKEND}: the keys go with their values");
}

#[test]
fn radix_sorts_are_stable() {
    let Some((_, mut q)) = setup() else { return };
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
