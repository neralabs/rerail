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
        for size in [4usize << 10, 16 << 10, 32 << 10, 64 << 10] {
            let prefix: Vec<u8> = ps.concat().into_iter().take(size).collect();
            let t0 = std::time::Instant::now();
            let zd = zstd_dictionary(&ps, size);
            let s_zstd = t0.elapsed();
            let t0 = std::time::Instant::now();
            let sd = sa_dictionary(&ps, size);
            let s_sa = t0.elapsed();
            let mut line = format!("  {:>2} KB:", size >> 10);
            for (dn, d, took) in [("first bytes", &prefix, None), ("zstd", &zd, Some(s_zstd)), ("suffix array", &sd, Some(s_sa))] {
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
            eprintln!("{line}");
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
