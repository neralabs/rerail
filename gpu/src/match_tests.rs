//! 16z's matches across a tape's pieces (decision 0727): what a dictionary stored once in the tape gives LZ4, its
//! bytes counted, and what a suffix array costs to build. Each piece is still decoded on its own (with the
//! dictionary), as 16z's pieces are now.

use crate::bitlex_tests::{json_lang, json_of, json_writer};
use std::os::raw::{c_char, c_int, c_void};

unsafe extern "C" {
    fn LZ4_loadDict(s: *mut lz4_sys::LZ4StreamEncode, dict: *const c_char, size: c_int) -> c_int;
    fn LZ4_compress_fast_continue(s: *mut lz4_sys::LZ4StreamEncode, src: *const c_char, dst: *mut c_char, n: c_int, cap: c_int, accel: c_int) -> c_int;
    fn LZ4_decompress_safe_usingDict(src: *const c_char, dst: *mut c_char, n: c_int, cap: c_int, dict: *const c_char, size: c_int) -> c_int;
    fn LZ4_createStreamHC() -> *mut c_void;
    fn LZ4_freeStreamHC(s: *mut c_void) -> c_int;
    fn LZ4_resetStreamHC_fast(s: *mut c_void, level: c_int);
    fn LZ4_loadDictHC(s: *mut c_void, dict: *const c_char, size: c_int) -> c_int;
    fn LZ4_compress_HC_continue(s: *mut c_void, src: *const c_char, dst: *mut c_char, n: c_int, cap: c_int) -> c_int;
}

/// `s` as one LZ4 block against dictionary `dict` (its last 64 KB: LZ4's window), the fast parse (`level` 0) or HC.
fn lz4_dict(s: &[u8], dict: &[u8], level: i32) -> Vec<u8> {
    let cap = unsafe { lz4_sys::LZ4_compressBound(s.len() as i32) } as usize;
    let mut out = vec![0u8; cap];
    // SAFETY: the streams are made and freed here; `out` holds the bound's bytes.
    let n = unsafe {
        if level == 0 {
            let st = lz4_sys::LZ4_createStream();
            LZ4_loadDict(st, dict.as_ptr().cast(), dict.len() as c_int);
            let n = LZ4_compress_fast_continue(st, s.as_ptr().cast(), out.as_mut_ptr().cast(), s.len() as c_int, cap as c_int, 1);
            lz4_sys::LZ4_freeStream(st);
            n
        } else {
            let st = LZ4_createStreamHC();
            LZ4_resetStreamHC_fast(st, level);
            LZ4_loadDictHC(st, dict.as_ptr().cast(), dict.len() as c_int);
            let n = LZ4_compress_HC_continue(st, s.as_ptr().cast(), out.as_mut_ptr().cast(), s.len() as c_int, cap as c_int);
            LZ4_freeStreamHC(st);
            n
        }
    };
    assert!(n > 0, "LZ4 failed");
    out.truncate(n as usize);
    out
}

/// `z`, a block made against `dict`, decoded (`n` bytes).
fn unlz4_dict(z: &[u8], dict: &[u8], n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n];
    // SAFETY: `out` holds `n` bytes.
    let got = unsafe { LZ4_decompress_safe_usingDict(z.as_ptr().cast(), out.as_mut_ptr().cast(), z.len() as c_int, n as c_int, dict.as_ptr().cast(), dict.len() as c_int) };
    assert_eq!(got as usize, n, "a piece did not decode against its dictionary");
    out
}

/// The pieces `z_of` would encode (those of at least LZ4's minimum), in its order.
fn pieces(t: &[u8]) -> Vec<Vec<u8>> {
    let got = std::cell::RefCell::new(Vec::new());
    rerail_tape_format::f16::z_of(t, &|s: &[u8]| {
        got.borrow_mut().push(s.to_vec());
        s.to_vec()
    })
    .unwrap();
    got.into_inner()
}

/// The suffix array of `t` (libsais, one thread) and its LCP array (`lcp[i]` the common prefix of the suffixes at
/// `sa[i - 1]` and `sa[i]`; 0 at 0), and the seconds of each.
fn suffix_array(t: &[u8]) -> (Vec<i32>, Vec<i32>, f64, f64) {
    let n = t.len();
    let mut sa = vec![0i32; n];
    let t0 = std::time::Instant::now();
    // SAFETY: `sa` holds `n` entries.
    assert_eq!(unsafe { libsais_sys::libsais::libsais(t.as_ptr(), sa.as_mut_ptr(), n as i32, 0, std::ptr::null_mut()) }, 0);
    let s_sa = t0.elapsed().as_secs_f64();
    let (mut plcp, mut lcp) = (vec![0i32; n], vec![0i32; n]);
    let t1 = std::time::Instant::now();
    // SAFETY: each array holds `n` entries.
    unsafe {
        assert_eq!(libsais_sys::libsais::libsais_plcp(t.as_ptr(), sa.as_ptr(), plcp.as_mut_ptr(), n as i32), 0);
        assert_eq!(libsais_sys::libsais::libsais_lcp(plcp.as_ptr(), sa.as_ptr(), lcp.as_mut_ptr(), n as i32), 0);
    }
    (sa, lcp, s_sa, t1.elapsed().as_secs_f64())
}

/// A dictionary of `size` bytes chosen from the pieces' repeats by their suffix array: each LCP interval of at least
/// 16 bytes (a substring at `c` places, `len` long, at most 1 KB) scored by the bytes it would cover, `c · len`, taken
/// best first unless its bytes are already in the dictionary. The best last, nearest the piece (LZ4's offsets).
fn sa_dictionary(ps: &[Vec<u8>], size: usize) -> Vec<u8> {
    const MIN: usize = 16;
    let text: Vec<u8> = ps.concat();
    let (sa, lcp, _, _) = suffix_array(&text);
    let n = text.len();
    let mut cands: Vec<(u64, usize, usize)> = Vec::new();
    // The LCP intervals, by a stack of (length, left bound).
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for i in 1..=n {
        let h = if i < n { lcp[i] as usize } else { 0 };
        let mut lb = i - 1;
        while let Some(&(len, l)) = stack.last() {
            if len <= h {
                break;
            }
            stack.pop();
            let c = i - l;
            let len = len.min(1024);
            if len >= MIN {
                cands.push(((c * len) as u64, sa[l] as usize, len));
            }
            lb = l;
        }
        if h > 0 && stack.last().is_none_or(|&(len, _)| len < h) {
            stack.push((h, lb));
        }
    }
    cands.sort_unstable_by(|a, b| b.0.cmp(&a.0));
    let mut have = std::collections::HashSet::new();
    let mut chosen: Vec<&[u8]> = Vec::new();
    let mut used = 0;
    for &(_, at, len) in &cands {
        if used >= size {
            break;
        }
        let s = &text[at..at + len.min(size - used)];
        if s.len() < MIN || have.contains(&s[..MIN]) {
            continue;
        }
        for w in s.windows(MIN) {
            have.insert(w);
        }
        chosen.push(s);
        used += s.len();
    }
    chosen.reverse();
    chosen.concat()
}

/// A dictionary of `size` bytes by zstd's cover (fastCover's segments), its counts exact from the pieces' suffix array
/// and sensitive to where they lie:
/// - each d-mer (the suffixes that share their first `d` bytes, a run of the LCP array at `d` or more) counts the
///   pieces it is in, once each: within a piece LZ4 matches its repeats itself;
/// - a segment of `k` bytes scores the counts of the distinct d-mers in it; the best of each epoch (the pieces cut into
///   `size / k / 4` runs, taken in turn) is taken, and its d-mers' counts zeroed, so an overlap is never counted twice;
/// - the dictionary fills from its end, the first taken nearest the pieces (LZ4's window is 64 KB back).
fn cover_dictionary(ps: &[Vec<u8>], size: usize, d: usize, k: usize) -> Vec<u8> {
    let text: Vec<u8> = ps.concat();
    let n = text.len();
    let mut piece = vec![0u32; n];
    let mut at = 0;
    for (q, p) in ps.iter().enumerate() {
        piece[at..at + p.len()].fill(q as u32);
        at += p.len();
    }
    let (sa, lcp, _, _) = suffix_array(&text);
    // Each position's d-mer (its group in the array), and each group's pieces; none where fewer than d bytes are left
    // in the position's piece.
    let fits = |p: usize| p + d <= n && piece[p + d - 1] == piece[p];
    let mut group = vec![u32::MAX; n];
    let mut freq: Vec<u32> = Vec::new();
    let mut r = 0;
    while r < n {
        let mut e = r + 1;
        while e < n && lcp[e] as usize >= d {
            e += 1;
        }
        if fits(sa[r] as usize) {
            let g = freq.len() as u32;
            let mut seen = std::collections::HashSet::new();
            for &p in &sa[r..e] {
                if fits(p as usize) {
                    group[p as usize] = g;
                    seen.insert(piece[p as usize]);
                }
            }
            freq.push(if seen.len() > 1 { seen.len() as u32 } else { 0 });
        }
        r = e;
    }
    let epochs = (size / k / 4).max(1);
    let span = n / epochs;
    let mut dict = vec![0u8; size];
    let mut tail = size;
    let mut in_seg = vec![0u16; freq.len()];
    let mut stale = 0;
    let mut e = 0;
    while tail > 0 && stale < epochs {
        let (b, end) = (e * span, ((e + 1) * span).min(n));
        e = (e + 1) % epochs;
        // The best segment of the epoch by its distinct d-mers' counts (fastCover's sliding window).
        let (mut best, mut score, mut s0) = ((0usize, 0usize, 0u64), 0u64, b);
        for p in b..end {
            let g = group[p];
            if g != u32::MAX {
                if in_seg[g as usize] == 0 {
                    score += freq[g as usize] as u64;
                }
                in_seg[g as usize] += 1;
            }
            if p + 1 - s0 > k - d + 1 {
                let h = group[s0];
                if h != u32::MAX {
                    in_seg[h as usize] -= 1;
                    if in_seg[h as usize] == 0 {
                        score -= freq[h as usize] as u64;
                    }
                }
                s0 += 1;
            }
            if score > best.2 {
                best = (s0, p + 1, score);
            }
        }
        for p in s0..end {
            if group[p] != u32::MAX {
                in_seg[group[p] as usize] -= 1;
            }
        }
        if best.2 == 0 {
            stale += 1;
            continue;
        }
        stale = 0;
        for p in best.0..best.1 {
            if group[p] != u32::MAX {
                freq[group[p] as usize] = 0;
            }
        }
        // The segment's bytes, its last d-mer's whole, at the dictionary's tail.
        let seg = &text[best.0..(best.1 + d - 1).min(n)];
        let take = seg.len().min(tail);
        dict[tail - take..tail].copy_from_slice(&seg[seg.len() - take..]);
        tail -= take;
    }
    dict[tail..].to_vec()
}

/// `cover_dictionary` at the d and k that make the pieces' 16z the smallest (zstd's search of its cover's parameters,
/// by LZ4's own output here), dictionary counted: the dictionary and its (d, k).
fn best_cover(t: &[u8], ps: &[Vec<u8>], size: usize, level: i32) -> (Vec<u8>, usize, usize) {
    let mut best: Option<(usize, Vec<u8>, usize, usize)> = None;
    for d in [6usize, 8] {
        for k in [16usize, 32, 64, 128, 256] {
            let dict = cover_dictionary(ps, size, d, k);
            let z = rerail_tape_format::f16::z_of(t, &|s: &[u8]| lz4_dict(s, &dict, level)).unwrap().len() + dict.len();
            if best.as_ref().is_none_or(|b| z < b.0) {
                best = Some((z, dict, d, k));
            }
        }
    }
    let (_, dict, d, k) = best.unwrap();
    (dict, d, k)
}

/// zstd's dictionary trainer (fastCover) over the pieces, its content only (no entropy tables: LZ4 reads bytes).
fn zstd_dictionary(ps: &[Vec<u8>], size: usize) -> Vec<u8> {
    let d = zstd::dict::from_samples(ps, size + 1024).expect("a zstd dictionary");
    // SAFETY: `d` is the dictionary the trainer made.
    let h = unsafe { zstd::zstd_safe::zstd_sys::ZDICT_getDictHeaderSize(d.as_ptr().cast(), d.len()) };
    let start = d.len() - (d.len() - h).min(size);
    d[start..].to_vec()
}

/// The JSON inputs: the synthetic 2 MB file, and simdjson's examples from `RERAIL_JSON_EXAMPLES` where it is set.
fn inputs() -> Vec<(String, Vec<u8>)> {
    let mut v = vec![("synthetic 2 MB".to_string(), json_of(2 << 20, 0))];
    if let Some(d) = std::env::var_os("RERAIL_JSON_EXAMPLES") {
        for name in ["twitter.json", "citm_catalog.json", "canada.json", "mesh.json", "gsoc-2018.json", "marine_ik.json", "update-center.json"] {
            if let Ok(b) = std::fs::read(std::path::Path::new(&d).join(name)) {
                v.push((name.to_string(), b));
            }
        }
    }
    v
}

/// The tape of `f` in format 16.
fn tape_of(f: &[u8]) -> Option<Vec<u8>> {
    let lang = json_lang();
    let mut w = json_writer(&lang, f);
    rerail_json_aot::parse(&rerail_json_aot::padded(f), f.len(), &mut w).ok()?;
    Some(w.finish().ok()?.tape.as_bytes().to_vec())
}

/// What a dictionary stored once in the tape gives 16z, its bytes counted: each input's tape as 16z with no
/// dictionary, with a dictionary of 4 to 64 KB (LZ4's window) chosen three ways (the pieces' first bytes; zstd's
/// trainer; the pieces' repeats by their suffix array), LZ4's fast parse and HC 12; and the tape as one LZ4 stream,
/// the bound of matches across pieces. Every piece is decoded again against its dictionary. Run by name: `--ignored
/// z16_dictionaries`.
#[test]
#[ignore]
fn z16_dictionaries() {
    for (name, f) in inputs() {
        let Some(t) = tape_of(&f) else {
            eprintln!("{name}: does not parse");
            continue;
        };
        let ps = pieces(&t);
        let plain = |level: i32| rerail_tape_format::f16::z_of(&t, &|s: &[u8]| lz4_dict(s, &[], level)).unwrap().len();
        let whole = lz4_dict(&t, &[], 12).len();
        eprintln!("{name}: {} bytes of JSON, a tape of {} bytes, {} pieces encoded; 16z {} (fast), {} (HC 12); one stream {} (HC 12)", f.len(), t.len(), ps.len(), plain(0), plain(12), whole);
        for size in [4usize << 10, 8 << 10, 16 << 10, 32 << 10] {
            let prefix: Vec<u8> = ps.concat().into_iter().take(size).collect();
            let t0 = std::time::Instant::now();
            let zd = zstd_dictionary(&ps, size);
            let s_zstd = t0.elapsed();
            let t0 = std::time::Instant::now();
            let sd = sa_dictionary(&ps, size);
            let s_sa = t0.elapsed();
            let t0 = std::time::Instant::now();
            let (cd, cdd, cdk) = best_cover(&t, &ps, size, 12);
            let s_cover = t0.elapsed();
            let t0 = std::time::Instant::now();
            let one = cover_dictionary(&ps, size, cdd, cdk);
            let s_one = t0.elapsed();
            assert!(one == cd);
            let mut line = format!("  {:>2} KB:", size >> 10);
            for (dn, d, took) in [("first bytes", &prefix, None), ("zstd", &zd, Some(s_zstd)), ("repeats", &sd, Some(s_sa)), ("cover", &cd, Some(s_one))] {
                for level in [0, 12] {
                    let z = rerail_tape_format::f16::z_of(&t, &|s: &[u8]| {
                        let z = lz4_dict(s, d, level);
                        assert!(unlz4_dict(&z, d, s.len()) == s);
                        z
                    })
                    .unwrap();
                    let total = z.len() + d.len();
                    line += &format!(" {dn} {} {} = {total} ({:+.1}%);", if level == 0 { "fast" } else { "HC" }, d.len(), 100.0 * (total as f64 / plain(level) as f64 - 1.0));
                }
                if let Some(took) = took {
                    line += &format!(" built in {took:.1?};");
                }
            }
            eprintln!("{line} cover's d {cdd}, k {cdk}, its search {s_cover:.1?}");
        }
    }
}

/// What a suffix array costs on the CPU: libsais (SA-IS, one thread) and its LCP array, over each input's JSON, its
/// tape, and its pieces end to end. Run by name: `--ignored suffix_array_cpu_cost`.
#[test]
#[ignore]
fn suffix_array_cpu_cost() {
    for (name, f) in inputs() {
        let Some(t) = tape_of(&f) else { continue };
        let ps = pieces(&t).concat();
        for (what, s) in [("the JSON", &f), ("the tape", &t), ("its pieces", &ps)] {
            let mut best = (f64::MAX, f64::MAX);
            for _ in 0..3 {
                let (_, _, a, b) = suffix_array(s);
                best = (best.0.min(a), best.1.min(b));
            }
            eprintln!("{name}, {what}: {} bytes: SA {:.2} ms ({:.0} MB/s), LCP {:.2} ms", s.len(), best.0 * 1e3, s.len() as f64 / best.0 / 1e6, best.1 * 1e3);
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MatchArgs {
    nq: u32,
    qs: u32,
    q0: u32,
    nr: u32,
    r0: u32,
    pad: [u32; 3],
}
unsafe impl crate::Pod for MatchArgs {}

const MATCH: &str = include_str!("match_tests.uglykl");

/// For each query (the 32 bytes at `q0 + q · qs` of `t`), the first reference `j < nr` whose 32 bytes at `r0 + j`
/// are the query's (`u32::MAX` for none): the host's answer.
fn windows_host(t: &[u8], a: &MatchArgs) -> Vec<u32> {
    let mut first = std::collections::HashMap::new();
    for j in (0..a.nr as usize).rev() {
        first.insert(&t[a.r0 as usize + j..a.r0 as usize + j + 32], j as u32);
    }
    (0..a.nq as usize).map(|q| *first.get(&t[a.q0 as usize + q * a.qs as usize..][..32]).unwrap_or(&u32::MAX)).collect()
}

/// The int8 matrix units on substring matching (decision 0727): an exact join of 32-byte windows, each query's first
/// equal reference, on the unit (`m_mma`: the windows' squared differences by an i8 product of Hankel matrices), the
/// same kernel with the unit's emulation forced, and the threads (`m_threads`). Each the host's answer, query by
/// query; and each's device time and rate (window pairs a second, the product's int8 operations a second). The
/// text: the synthetic 2 MB file's tape, queries every 16 bytes of it, against a 16 KB dictionary zstd trained on its
/// pieces (or the tape's first 16 KB with `RERAIL_MATCH_REFS=tape`). Run by name: `--ignored int8_window_join`.
#[test]
#[ignore]
fn int8_window_join() {
    use crate::kernels::{alloc, buffer_of, launch, program_with, read};
    let Some(shared) = crate::shared::get() else { return };
    let mut q = crate::device::ComputeQueue::new(&shared.queue);
    let cpu = crate::device::backend() == "CPU";
    let tape = tape_of(&json_of(2 << 20, 0)).unwrap();
    let dict = if std::env::var("RERAIL_MATCH_REFS").is_ok_and(|v| v == "tape") { tape[..16 << 10].to_vec() } else { zstd_dictionary(&pieces(&tape), 16 << 10) };
    let mut t = dict.clone();
    t.extend_from_slice(&tape);
    t.resize(t.len() + 64, 0);
    let (nq, nr) = if cpu { (1024usize, 2048usize) } else { (tape.len() / 16 - 4, dict.len() / 16 * 16) };
    let args = MatchArgs { nq: nq as u32, qs: 16, q0: dict.len() as u32, nr: nr as u32, r0: 0, pad: [0; 3] };
    let want = windows_host(&t, &args);
    let found = want.iter().filter(|&&j| j != u32::MAX).count();
    let tb = buffer_of(&t).unwrap();
    let (pq, pr, out) = (alloc::<i32>(nq).unwrap(), alloc::<i32>(nr).unwrap(), alloc::<u32>(nq).unwrap());
    // `RERAIL_MATCH_QT`: the query tiles an owner takes (each B read once for them).
    let qt: usize = std::env::var("RERAIL_MATCH_QT").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
    let qd = format!("{qt}u");
    let native = program_with("match_tests.uglykl", MATCH, &[("M_QT", qd.as_str())]).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()));
    let emulated = program_with("match_tests.uglykl", MATCH, &[("M_QT", qd.as_str()), ("UKL_MMA_FORCE_EMULATED", "1")]).unwrap();
    let cb = alloc::<u32>(6).unwrap();
    launch(&q, native.function("m_consts").unwrap()).buffer("out", &cb).run(&[1], &[1]).unwrap();
    q.flush().await_().unwrap();
    let c: Vec<u32> = read(&cb, 6);
    eprintln!("{} ({}): i8 {} ({}×{}×{}), an owner {} lanes; {nq} queries × {nr} references ({} found by the host)", crate::device::backend(), shared.device.name(), if c[0] == 1 { "on the unit" } else { "emulated" }, c[1], c[2], c[3], c[4], found);
    let squares = |a: MatchArgs, p: &crate::device::Buffer, n: usize| {
        launch(&q, native.function("m_squares").unwrap()).value("args", &a).buffer("T", &tb).buffer("P", p).run(&[n.div_ceil(64) as u64], &[64]).unwrap();
    };
    squares(args, &pq, nq);
    squares(MatchArgs { nq: nr as u32, qs: 1, q0: 0, ..args }, &pr, nr);
    q.flush().await_().unwrap();
    let pairs = (nq * nr) as f64;
    for (name, p) in [("the unit", Some(native)), ("the unit's emulation", Some(emulated)), ("the threads", None)] {
        if name == "the unit" && c[0] != 1 {
            continue;
        }
        let mut best = f64::MAX;
        for round in 0..4 {
            let none = vec![u32::MAX; nq];
            // SAFETY: the buffer holds nq words; no launch runs.
            unsafe { std::ptr::copy_nonoverlapping(none.as_ptr(), out.contents().cast::<u32>(), nq) };
            match p {
                Some(p) => launch(&q, p.function("m_mma").unwrap()).value("args", &args).buffer("T", &tb).buffer("Pq", &pq).buffer("Pr", &pr).buffer("out", &out).run(&[nq.div_ceil(16 * qt) as u64], &[c[4] as u64]).unwrap(),
                None => launch(&q, native.function("m_threads").unwrap()).value("args", &args).buffer("T", &tb).buffer("out", &out).run(&[nq.div_ceil(64) as u64], &[64]).unwrap(),
            }
            let e = q.flush();
            e.await_().unwrap();
            if round > 0 {
                best = best.min(e.gpu_seconds());
            }
            let got: Vec<u32> = read(&out, nq);
            let bad = (0..nq).filter(|&i| got[i] != want[i]).count();
            assert!(bad == 0, "{name}: {bad} of {nq} queries' first matches are not the host's (query {:?})", (0..nq).find(|&i| got[i] != want[i]).map(|i| (i, got[i], want[i])));
        }
        eprintln!("  {name:<22} ({qt} tiles an owner) {:>8.3} ms: {:>7.1} G window pairs/s, {:>6.1} T int8 ops/s (a product's)", best * 1e3, pairs / best / 1e9, pairs * 32.0 * 2.0 / best / 1e12);
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SaArgs {
    n: u32,
    h: u32,
    b: u32,
    pad: u32,
}
unsafe impl crate::Pod for SaArgs {}

/// The suffix array of `t` on the device, by prefix doubling over the rows not yet alone in their group: its entries,
/// each round's GPU seconds (its sort's, its ranks' and its compaction's) and rows sorted, and the launches. Each sort
/// takes a multiple of 16 bits, an even number of passes, so the rows end in the pair they started in.
/// Sorted only to the first `limit` bytes (`usize::MAX`: wholly): the rounds stop once the ranks cover `limit` bytes,
/// and the suffixes still tied keep the sort's order, by position. The last is the bytes the order covers (7 · 2^r,
/// at least `limit`; the text's length where it sorted wholly).
fn suffix_array_device(q: &mut crate::device::ComputeQueue, t: &[u8], limit: usize) -> (Vec<u32>, Vec<(f64, usize)>, usize, usize) {
    use crate::kernels::{alloc, buffer_of, launch, program_with, read};
    use crate::library::{radix_sort_pairs_in, scan_exclusive_u32_in, RadixSpace, ScanSpace, Sorted};
    let p = program_with("match_tests.uglykl", MATCH, &[]).unwrap();
    let n = t.len();
    let tb = buffer_of(t).unwrap();
    let [keys, k1] = [alloc::<u64>(n).unwrap(), alloc::<u64>(n).unwrap()];
    let [mut pos, mut p1] = [alloc::<u32>(n).unwrap(), alloc::<u32>(n).unwrap()];
    let w = || alloc::<u32>(n + 1).unwrap();
    let (heads, sid, oheads, soid, first, ofirst, rank, keep, koff) = (w(), w(), w(), w(), w(), w(), w(), w(), w());
    let (space, sspace) = (RadixSpace::new(n as u32).unwrap(), ScanSpace::new(n as u32).unwrap());
    let sa_b = alloc::<u32>(n).unwrap();
    let b = 32 - (n as u32 + 1).leading_zeros();
    let (mut launches, mut rounds) = (2, Vec::new());
    let grid = |m: usize| [m.div_ceil(256).max(1) as u64];
    let mut g0 = crate::device::gpu_seconds_awaited();
    launch(q, p.function("sa_init").unwrap()).value("args", &SaArgs { n: n as u32, h: 0, b, pad: 0 }).buffer("T", &tb).buffer("keys", &keys).buffer("pos", &pos).run(&grid(n), &[256]).unwrap();
    let (mut m, mut h, mut bits, mut first_round) = (n, 7u32, 64u32, true);
    loop {
        let a = SaArgs { n: m as u32, h, b, pad: first_round as u32 };
        let (sorted, made) = radix_sort_pairs_in::<u64>(q, [&keys, &pos], [&k1, &p1], m as u32, bits, &space).unwrap();
        assert_eq!(sorted, Sorted::InPlace);
        launch(q, p.function("sa_heads").unwrap()).value("args", &a).buffer("keys", &keys).buffer("heads", &heads).buffer("oheads", &oheads).run(&grid(m), &[256]).unwrap();
        let s1 = scan_exclusive_u32_in(q, &heads, &sid, m as u32, false, &sspace).unwrap();
        let s2 = scan_exclusive_u32_in(q, &oheads, &soid, m as u32, false, &sspace).unwrap();
        launch(q, p.function("sa_firsts").unwrap()).value("args", &a).buffer("heads", &heads).buffer("sid", &sid).buffer("oheads", &oheads).buffer("soid", &soid).buffer("first", &first).buffer("ofirst", &ofirst).run(&grid(m), &[256]).unwrap();
        launch(q, p.function("sa_rerank").unwrap())
            .value("args", &a)
            .buffer("keys", &keys)
            .buffer("pos", &pos)
            .buffer("heads", &heads)
            .buffer("sid", &sid)
            .buffer("oheads", &oheads)
            .buffer("soid", &soid)
            .buffer("first", &first)
            .buffer("ofirst", &ofirst)
            .buffer("rank", &rank)
            .buffer("keep", &keep)
            .run(&grid(m), &[256])
            .unwrap();
        // The bytes the ranks now cover: 7 after the first round, then `h` (the keys' shift, doubled since).
        let covered = if first_round { 7 } else { h as usize };
        if covered >= limit {
            launch(q, p.function("sa_final_rank").unwrap()).value("args", &a).buffer("keys", &keys).buffer("pos", &pos).buffer("oheads", &oheads).buffer("soid", &soid).buffer("ofirst", &ofirst).buffer("rank", &rank).run(&grid(m), &[256]).unwrap();
            launches += made + s1 + s2 + 4;
            q.flush().await_().unwrap();
            let g1 = crate::device::gpu_seconds_awaited();
            rounds.push((g1 - g0, m));
            g0 = g1;
            launch(q, p.function("sa_out").unwrap()).value("args", &SaArgs { n: n as u32, h: 0, b, pad: 0 }).buffer("rank", &rank).buffer("sa", &sa_b).run(&grid(n), &[256]).unwrap();
            q.flush().await_().unwrap();
            rounds.last_mut().unwrap().0 += crate::device::gpu_seconds_awaited() - g0;
            return (read::<u32>(&sa_b, n), rounds, launches, covered);
        }
        let s3 = scan_exclusive_u32_in(q, &keep, &koff, m as u32, false, &sspace).unwrap();
        launch(q, p.function("sa_compact").unwrap()).value("args", &a).buffer("pos", &pos).buffer("keep", &keep).buffer("koff", &koff).buffer("out", &p1).run(&grid(m), &[256]).unwrap();
        launches += made + s1 + s2 + s3 + 5;
        q.flush().await_().unwrap();
        let left = read::<u32>(&koff, m + 1)[m] as usize;
        let g1 = crate::device::gpu_seconds_awaited();
        rounds.push((g1 - g0, m));
        g0 = g1;
        std::mem::swap(&mut pos, &mut p1);
        if left == 0 {
            break;
        }
        (m, first_round) = (left, false);
        launch(q, p.function("sa_keys").unwrap()).value("args", &SaArgs { n: m as u32, h, b, pad: n as u32 }).buffer("pos", &pos).buffer("rank", &rank).buffer("keys", &keys).run(&grid(m), &[256]).unwrap();
        launches += 1;
        h *= 2;
        bits = (2 * b).next_multiple_of(16);
    }
    launch(q, p.function("sa_out").unwrap()).value("args", &SaArgs { n: n as u32, h: 0, b, pad: 0 }).buffer("rank", &rank).buffer("sa", &sa_b).run(&grid(n), &[256]).unwrap();
    q.flush().await_().unwrap();
    rounds.last_mut().unwrap().0 += crate::device::gpu_seconds_awaited() - g0;
    (read::<u32>(&sa_b, n), rounds, launches, n)
}

/// Whether `sa` is a permutation of `t`'s suffixes ordered by their first `limit` bytes (a suffix that ends first
/// before), ties by position.
fn prefix_sorted(t: &[u8], sa: &[u32], limit: usize) -> bool {
    let mut seen = vec![false; t.len()];
    for &p in sa {
        if std::mem::replace(&mut seen[p as usize], true) {
            return false;
        }
    }
    let pre = |p: u32| &t[p as usize..(p as usize + limit).min(t.len())];
    sa.windows(2).all(|w| match pre(w[0]).cmp(pre(w[1])) {
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Equal => w[0] < w[1],
        std::cmp::Ordering::Greater => false,
    })
}

/// A suffix array sorted only to a prefix (what a match finder reads: LZ4's matches are found by their first bytes and
/// measured where they lie), on the device: each input's tape and JSON at prefixes of 16, 32, 64 and 256 bytes, each
/// checked (`prefix_sorted`), its rounds and GPU time, against the whole sort's and libsais's. Run by name: `--ignored
/// suffix_array_prefix_cost`.
#[test]
#[ignore]
fn suffix_array_prefix_cost() {
    let Some(shared) = crate::shared::get() else { return };
    let mut q = crate::device::ComputeQueue::new(&shared.queue);
    let cpu = crate::device::backend() == "CPU";
    eprintln!("{} ({})", crate::device::backend(), shared.device.name());
    for (name, f) in inputs() {
        let Some(t) = tape_of(&f) else { continue };
        for (what, s) in [("tape", &t), ("JSON", &f)] {
            let s: &[u8] = if cpu { &s[..s.len().min(20_000)] } else { s };
            let (_, _, cpu_s, _) = suffix_array(s);
            let mut line = format!("  {name}'s {what}, {} bytes, libsais {:.2} ms:", s.len(), cpu_s * 1e3);
            for limit in [16usize, 32, 64, 256, usize::MAX] {
                let mut best = (f64::MAX, 0);
                for _ in 0..3 {
                    let (got, rounds, _, covered) = suffix_array_device(&mut q, s, limit);
                    assert!(prefix_sorted(s, &got, covered.min(s.len())), "{name}'s {what}: not sorted to {covered} bytes");
                    let total: f64 = rounds.iter().map(|r| r.0).sum();
                    if total < best.0 {
                        best = (total, rounds.len());
                    }
                }
                let l = if limit == usize::MAX { "all".to_string() } else { limit.to_string() };
                line += &format!(" {l}: {:.2} ms ({} rounds);", best.0 * 1e3, best.1);
            }
            eprintln!("{line}");
        }
    }
}

/// A format 16 tape's nodes as values read from the tape: code number, raw slot, subtree size, start, length.
struct Nodes {
    code: Vec<u32>,
    slot: Vec<u64>,
    size: Vec<u32>,
    start: Vec<u32>,
    len: Vec<u32>,
    cw: usize,
}

fn nodes_of(t: &[u8]) -> Nodes {
    use rerail_tape_format::reads::f16 as r;
    let lang = json_lang();
    let p = rerail_tape_format::f16::placement(t).unwrap();
    let v = p.view(&lang, 0).unwrap();
    let n = p.n as u32;
    let mut prev = 0;
    let start = (0..n)
        .map(|i| {
            prev = r::start_next(t, &v, i, prev);
            prev
        })
        .collect();
    Nodes {
        code: (0..n).map(|i| r::number(t, &v, i)).collect(),
        slot: (0..n).map(|i| r::slot(t, &v, i)).collect(),
        size: (0..n).map(|i| r::size(t, &v, i)).collect(),
        start,
        len: (0..n).map(|i| r::length(t, &v, i)).collect(),
        cw: p.cw,
    }
}

/// The tree's at-rest form with what one pass rebuilds removed (the duplicated-code spike's closes form, 2087ba7391
/// and 29f12edaf1): each node's leaf bit and the subtrees that close right after it (no sizes, no parent steps:
/// copy-invariant), the gaps, a one-node subtree's length, an inner node's end as its distance past its last node's
/// end. `residual` stores each gap less the previous node's length where that is a one-node subtree (a gap's bytes
/// between two tokens: the separators). Returns its columns, each in node order, and the node each value belongs to.
struct AtRest {
    structure: Vec<u32>,
    gap: Vec<u32>,
    len1: Vec<(u32, u32)>,
    trail: Vec<(u32, u32)>,
}

fn at_rest(nd: &Nodes, residual: bool) -> AtRest {
    let n = nd.code.len();
    let mut ar = AtRest { structure: vec![0; n], gap: vec![0; n], len1: Vec::new(), trail: Vec::new() };
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for k in 0..n {
        stack.push((k, k + nd.size[k] as usize));
        if nd.size[k] == 1 {
            ar.len1.push((k as u32, nd.len[k]));
        }
        let last_end = nd.start[k] + nd.len[k];
        let mut c = 0;
        while let Some(&(j, e)) = stack.last() {
            if e > k + 1 {
                break;
            }
            stack.pop();
            c += 1;
            if nd.size[j] > 1 {
                ar.trail.push((k as u32, nd.start[j] + nd.len[j] - last_end));
            }
        }
        ar.structure[k] = (nd.slot[k] & 1) as u32 | c << 1;
        let prev_end = if k == 0 { 0 } else if residual && nd.size[k - 1] == 1 { nd.start[k - 1] + nd.len[k - 1] } else { nd.start[k - 1] };
        ar.gap[k] = nd.start[k] - prev_end;
    }
    ar
}

/// The tape's slots, starts and lengths rebuilt from the at-rest form by one pass with a stack (the spike's
/// `rest_check`): starts by a running sum of the gaps (with `residual`, of each gap and the previous one-node subtree's
/// length), sizes by the closes, a leaf's slot from the node after it (its previous sibling, or every `K`-th sibling,
/// its parent), an inner node's length from its last node's end and its trailing distance.
fn rebuild(ar: &AtRest, residual: bool) -> (Vec<u64>, Vec<u32>, Vec<u32>) {
    const K: u32 = rerail_tape_format::f16::K as u32;
    let n = ar.structure.len();
    let (mut slots, mut starts, mut lens) = (vec![0u64; n], vec![0u32; n], vec![0u32; n]);
    // Frames: (node, children so far, last child).
    let mut open: Vec<(u32, u32, u32)> = Vec::with_capacity(64);
    let (mut li, mut ti, mut start, mut prev_end) = (0, 0, 0u32, 0u32);
    for k in 0..n {
        let i = k as u32;
        start = prev_end + ar.gap[k];
        starts[k] = start;
        if let Some(f) = open.last_mut() {
            let (p, idx, last) = *f;
            f.1 += 1;
            f.2 = i;
            if i - 1 != p {
                slots[k - 1] = if idx % K == 0 { u64::from(i - p) << 2 | 3 } else { u64::from(i - last) << 2 | 1 };
            }
        }
        let c = ar.structure[k] >> 1;
        if c == 0 {
            open.push((i, 0, 0));
            prev_end = start;
            continue;
        }
        let end = start + ar.len1[li].1;
        li += 1;
        lens[k] = end - start;
        if ar.structure[k] & 1 == 0 {
            slots[k] = 1 << 1;
        }
        prev_end = if residual { end } else { start };
        for _ in 1..c {
            let (j, ..) = open.pop().unwrap();
            slots[j as usize] = u64::from(i + 1 - j) << 1;
            lens[j as usize] = end + ar.trail[ti].1 - starts[j as usize];
            ti += 1;
        }
    }
    (slots, starts, lens)
}

/// A column of values as format 16 stores one, in pieces of `PART` nodes: a byte a value, all ones an escape and the
/// value as u32 in the piece's side bytes. `at` is the node each value belongs to.
fn column_pieces(vals: impl Iterator<Item = (u32, u32)>, n: usize) -> Vec<Vec<u8>> {
    const PART: usize = rerail_tape_format::f16::PART;
    let mut fixed = vec![Vec::new(); n.div_ceil(PART)];
    let mut side = vec![Vec::new(); n.div_ceil(PART)];
    for (at, x) in vals {
        let b = at as usize / PART;
        if x < 255 {
            fixed[b].push(x as u8);
        } else {
            fixed[b].push(255);
            side[b].extend_from_slice(&x.to_le_bytes());
        }
    }
    fixed.into_iter().zip(side).map(|(mut f, s)| {
        f.extend_from_slice(&s);
        f
    }).collect()
}

/// 16z's bytes for pieces: each LZ4 where that saves a tenth, else raw.
fn z_bytes_of(ps: &[Vec<u8>], level: i32) -> usize {
    ps.iter().map(|raw| if raw.len() >= 16 { let z = lz4_dict(raw, &[], level).len(); if z * 10 <= raw.len() * 9 { z } else { raw.len() } } else { raw.len() }).sum()
}

/// How much 16z gains when the tree is stored without what one pass rebuilds on decoding (the duplicated-code spike's
/// closes form): each input's tape, its pieces re-encoded the same way (a byte a value, escapes aside, a piece every
/// `PART` nodes, LZ4 fast and HC 12) as today's columns (code, slot, gap, length, and the raw checkpoints), as the
/// closes form (code, leaf bit and closes, gap, one-node lengths, trailing distances; no checkpoints), and as it with
/// residual gaps. Each form is rebuilt and checked exactly against the tape. Run by name: `--ignored z16_at_rest`.
#[test]
#[ignore]
fn z16_at_rest() {
    for (name, f) in inputs() {
        let Some(t) = tape_of(&f) else { continue };
        let nd = nodes_of(&t);
        let n = nd.code.len();
        let z16: usize = rerail_tape_format::f16::z_of(&t, &|s: &[u8]| lz4_dict(s, &[], 0)).unwrap().len();
        let idx = |v: &[u32]| v.iter().enumerate().map(|(k, &x)| (k as u32, x)).collect::<Vec<_>>();
        let codes = column_pieces(idx(&nd.code).into_iter(), n);
        let slots: Vec<u32> = nd.slot.iter().map(|&s| s as u32).collect();
        let gaps: Vec<u32> = (0..n).map(|k| if k == 0 { nd.start[0] } else { nd.start[k] - nd.start[k - 1] }).collect();
        let ckpt = n.div_ceil(rerail_tape_format::f16::K) * nd.cw;
        let today: Vec<Vec<u8>> = [codes.clone(), column_pieces(idx(&slots).into_iter(), n), column_pieces(idx(&gaps).into_iter(), n), column_pieces(idx(&nd.len).into_iter(), n)].concat();
        let z16hc: usize = rerail_tape_format::f16::z_of(&t, &|s: &[u8]| lz4_dict(s, &[], 12)).unwrap().len();
        let base: Vec<usize> = [0, 12].iter().map(|&l| z_bytes_of(&today, l) + ckpt).collect();
        // The rest of 16z (header, parts, the newline index, presence): the same in every form.
        let ours = [z16 - z_bytes_of(&pieces(&t), 0) - ckpt, z16hc - z_bytes_of(&pieces(&t), 12) - ckpt];
        let mut line = format!("{name}: {n} nodes, 16z fast {z16} HC {z16hc} bytes, its rest {} / {}; the columns re-encoded fast {} HC {};", ours[0], ours[1], base[0], base[1]);
        for (what, residual) in [("closes", false), ("closes, residual gaps", true)] {
            let ar = at_rest(&nd, residual);
            let t0 = std::time::Instant::now();
            let (slots2, starts2, lens2) = rebuild(&ar, residual);
            let ns = t0.elapsed().as_nanos() as f64 / n as f64;
            assert_eq!(starts2, nd.start, "{name}: starts");
            assert_eq!(lens2, nd.len, "{name}: lengths");
            let bad = (0..n.saturating_sub(1)).filter(|&k| !(nd.size[k] == 1 && nd.slot[k] & 1 == 0) && slots2[k] != nd.slot[k]).count();
            assert_eq!(bad, 0, "{name}: slots");
            let ps: Vec<Vec<u8>> = [codes.clone(), column_pieces(idx(&ar.structure).into_iter(), n), column_pieces(idx(&ar.gap).into_iter(), n), column_pieces(ar.len1.iter().copied(), n), column_pieces(ar.trail.iter().copied(), n)].concat();
            let z: Vec<usize> = [0, 12].iter().map(|&l| z_bytes_of(&ps, l)).collect();
            let whole = [ours[0] + z[0], ours[1] + z[1]];
            line += &format!(
                " {what} fast {} ({:+.1}%, whole {:+.1}%) HC {} ({:+.1}%, whole {:+.1}%), rebuilt {ns:.1} ns a node;",
                z[0],
                100.0 * (z[0] as f64 / base[0] as f64 - 1.0),
                100.0 * (whole[0] as f64 / z16 as f64 - 1.0),
                z[1],
                100.0 * (z[1] as f64 / base[1] as f64 - 1.0),
                100.0 * (whole[1] as f64 / z16hc as f64 - 1.0)
            );
        }
        eprintln!("{line}");
    }
}

/// The duplicated-code spike's suffix array to a depth (`Base::doubling`, native/lab/dupsa/src/incr.rs on
/// jeremybarnes/duplicated-code-spike): Manber and Myers' prefix doubling with Larsson and Sadakane's re-sort of the
/// groups still tied only, stopped at depth `w` (`usize::MAX`: wholly); here over bytes, one text. A counting sort by
/// byte, then each round sorts every open group by the rank `h` further on, from the round's ranks. Returns the
/// order (ties by position), the rounds, and the bytes the order covers.
fn doubling_to(t: &[u8], w: usize) -> (Vec<u32>, usize, usize) {
    let n = t.len();
    if WIDE.with(|x| x.get()) {
        return doubling_from_words(t, w);
    }
    let mut count = [0u32; 257];
    for &x in t {
        count[x as usize + 1] += 1;
    }
    for k in 1..257 {
        count[k] += count[k - 1];
    }
    let mut sa = vec![0u32; n];
    for (p, &x) in t.iter().enumerate() {
        sa[count[x as usize] as usize] = p as u32;
        count[x as usize] += 1;
    }
    // Ranks are a group's first index + 1 (0: past the end); groups of more than one are open.
    let mut r = vec![0u32; n];
    let mut groups: Vec<(u32, u32)> = Vec::new();
    let mut k = 0;
    while k < n {
        let x = t[sa[k] as usize];
        let mut e = k + 1;
        while e < n && t[sa[e] as usize] == x {
            e += 1;
        }
        for &p in &sa[k..e] {
            r[p as usize] = k as u32 + 1;
        }
        if e - k > 1 {
            groups.push((k as u32, e as u32));
        }
        k = e;
    }
    let (mut len, mut rounds) = (1usize, 0);
    let mut keyed: Vec<(u32, u32)> = Vec::new();
    while len < w && !groups.is_empty() {
        rounds += 1;
        let h = len.min(w - len);
        keyed.clear();
        for &(s, e) in &groups {
            for &p in &sa[s as usize..e as usize] {
                let q = p as usize + h;
                keyed.push((if q < n { r[q] } else { 0 }, p));
            }
        }
        let mut next = Vec::new();
        let mut at = 0usize;
        for &(s, e) in &groups {
            let g = &mut keyed[at..at + (e - s) as usize];
            at += (e - s) as usize;
            g.sort_unstable();
            let mut i = 0;
            while i < g.len() {
                let mut j = i + 1;
                while j < g.len() && g[j].0 == g[i].0 {
                    j += 1;
                }
                for (x, &(_, p)) in g[i..j].iter().enumerate() {
                    sa[s as usize + i + x] = p;
                    r[p as usize] = s + i as u32 + 1;
                }
                if j - i > 1 {
                    next.push((s + i as u32, s + j as u32));
                }
                i = j;
            }
        }
        groups = next;
        len += h;
    }
    (sa, rounds, if groups.is_empty() { n } else { len })
}

thread_local! {
    static WIDE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// `doubling_to` started from each suffix's first 8 bytes (a big-endian word: past the end 0, and the suffix's length
/// below 8 breaking the tie of a shorter one first), sorted by an LSD radix sort of 11-bit digits, so that the first
/// round's ranks cover 8 bytes and each round doubles them: 16 bytes in one round, 32 in two.
fn doubling_from_words(t: &[u8], w: usize) -> (Vec<u32>, usize, usize) {
    let n = t.len();
    let key = |p: usize| -> u64 {
        let mut k = 0u64;
        for j in 0..8 {
            k = k << 8 | t.get(p + j).copied().unwrap_or(0) as u64;
        }
        // A suffix shorter than 8 bytes ties with one padded by zeros: its length, low, puts it first.
        k
    };
    let short = |p: usize| (n - p).min(8) as u64;
    // (key, length cap, position) as a u128 sorted by an LSD radix sort on the key's 64 bits and the cap's 4.
    let mut a: Vec<(u64, u32)> = (0..n).map(|p| (key(p), p as u32)).collect();
    let mut b = vec![(0u64, 0u32); n];
    // The cap first (the least significant), then the key's 11-bit digits.
    let digit = |x: &(u64, u32), d: usize| -> usize { if d == 0 { short(x.1 as usize) as usize } else { ((x.0 >> (11 * (d - 1))) & 0x7ff) as usize } };
    for d in 0..7 {
        let mut count = vec![0u32; 2049];
        for x in &a {
            count[digit(x, d) + 1] += 1;
        }
        for k in 1..count.len() {
            count[k] += count[k - 1];
        }
        for x in &a {
            let c = &mut count[digit(x, d)];
            b[*c as usize] = *x;
            *c += 1;
        }
        std::mem::swap(&mut a, &mut b);
    }
    let mut sa: Vec<u32> = a.iter().map(|x| x.1).collect();
    let mut r = vec![0u32; n];
    let mut groups: Vec<(u32, u32)> = Vec::new();
    let mut k = 0;
    while k < n {
        let x = (a[k].0, short(a[k].1 as usize));
        let mut e = k + 1;
        while e < n && (a[e].0, short(a[e].1 as usize)) == x {
            e += 1;
        }
        for &p in &sa[k..e] {
            r[p as usize] = k as u32 + 1;
        }
        // A suffix of fewer than 8 bytes is whole: its group is one.
        if e - k > 1 && x.1 == 8 {
            groups.push((k as u32, e as u32));
        }
        k = e;
    }
    drop((a, b));
    let (mut len, mut rounds) = (8usize, 0);
    let mut keyed: Vec<(u32, u32)> = Vec::new();
    while len < w && !groups.is_empty() {
        rounds += 1;
        let h = len.min(w - len);
        keyed.clear();
        for &(s, e) in &groups {
            for &p in &sa[s as usize..e as usize] {
                let q = p as usize + h;
                keyed.push((if q < n { r[q] } else { 0 }, p));
            }
        }
        let mut next = Vec::new();
        let mut at = 0usize;
        for &(s, e) in &groups {
            let g = &mut keyed[at..at + (e - s) as usize];
            at += (e - s) as usize;
            g.sort_unstable();
            let mut i = 0;
            while i < g.len() {
                let mut j = i + 1;
                while j < g.len() && g[j].0 == g[i].0 {
                    j += 1;
                }
                for (x, &(_, p)) in g[i..j].iter().enumerate() {
                    sa[s as usize + i + x] = p;
                    r[p as usize] = s + i as u32 + 1;
                }
                if j - i > 1 {
                    next.push((s + i as u32, s + j as u32));
                }
                i = j;
            }
        }
        groups = next;
        len += h;
    }
    (sa, rounds, if groups.is_empty() { n } else { len })
}

/// The spike's prefix doubling (`doubling_to`) against libsais, one thread, on each input's tape and JSON: to 16, 32
/// and 64 bytes and wholly, each checked sorted to the bytes it covers. Run by name: `--ignored
/// suffix_array_doubling_cost`.
#[test]
#[ignore]
fn suffix_array_doubling_cost() {
    for (name, f) in inputs() {
        let Some(t) = tape_of(&f) else { continue };
        for (what, s) in [("tape", &t), ("JSON", &f)] {
            let (_, _, cpu_s, _) = suffix_array(s);
            let mut line = format!("  {name}'s {what}, {} bytes, libsais {:.2} ms:", s.len(), cpu_s * 1e3);
            for (wide, w) in [16usize, 32, 64, usize::MAX].iter().flat_map(|&w| [(false, w), (true, w)]) {
                WIDE.with(|x| x.set(wide));
                let mut best = (f64::MAX, 0);
                for _ in 0..2 {
                    let t0 = std::time::Instant::now();
                    let (sa, rounds, covered) = doubling_to(s, w);
                    let took = t0.elapsed().as_secs_f64();
                    assert!(prefix_sorted(s, &sa, covered.min(s.len())), "{name}'s {what}: not sorted to {covered} bytes");
                    if took < best.0 {
                        best = (took, rounds);
                    }
                }
                let l = if w == usize::MAX { "all".to_string() } else { w.to_string() };
                line += &format!(" {l}{}: {:.2} ms ({} rounds);", if wide { " from 8 bytes" } else { "" }, best.0 * 1e3, best.1);
            }
            eprintln!("{line}");
        }
    }
}

/// The suffix array on the device equals libsais's, entry by entry, and what it costs: each input's tape (and its
/// JSON), the device's GPU time by round and in all, against libsais on one CPU thread. Run by name: `--ignored
/// suffix_array_device_cost`.
#[test]
#[ignore]
fn suffix_array_device_cost() {
    let Some(shared) = crate::shared::get() else { return };
    let mut q = crate::device::ComputeQueue::new(&shared.queue);
    let cpu = crate::device::backend() == "CPU";
    eprintln!("{} ({})", crate::device::backend(), shared.device.name());
    let mut all: Vec<(String, Vec<u8>)> = Vec::new();
    for (name, f) in inputs() {
        if let Some(t) = tape_of(&f) {
            all.push((format!("{name}'s tape"), t));
        }
        all.push((format!("{name}'s JSON"), f));
    }
    for (name, s) in &all {
        let s: &[u8] = if cpu { &s[..s.len().min(20_000)] } else { s };
        let (want, _, cpu_s, _) = suffix_array(s);
        let mut best: Option<(f64, Vec<(f64, usize)>, usize)> = None;
        for _ in 0..3 {
            let (got, rounds, launches, _) = suffix_array_device(&mut q, s, usize::MAX);
            assert!(got.iter().zip(&want).all(|(&g, &w)| g as i32 == w), "{name}: the device's suffix array is not libsais's");
            let total: f64 = rounds.iter().map(|r| r.0).sum();
            if best.as_ref().is_none_or(|b| total < b.0) {
                best = Some((total, rounds, launches));
            }
        }
        let (total, rounds, launches) = best.unwrap();
        eprintln!("  {name}: {} bytes: the device {:.2} ms GPU ({:.0} MB/s; {} rounds (ms, rows sorted) {:?}, {launches} launches), libsais {:.2} ms", s.len(), total * 1e3, s.len() as f64 / total / 1e6, rounds.len(), rounds.iter().map(|r| ((r.0 * 1e4).round() / 10.0, r.1)).collect::<Vec<_>>(), cpu_s * 1e3);
    }
}
