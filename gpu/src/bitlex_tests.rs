//! Bitlex's tiers printed as UKL (decision 0726), on the process's device: each pass's kernels compute what the tier's
//! Rust computes, chunk by chunk.

use crate::kernels::{alloc, buffer_of, launch, program, read};
use crate::Pod;

#[repr(C)]
#[derive(Clone, Copy)]
struct T1Args {
    n: u32,
    blocks: u32,
    chunks: u32,
    write: u32,
}
unsafe impl Pod for T1Args {}

/// JSON files for tier 1: lists of objects (several chunks), strings with escapes and multibyte text, a byte-order
/// mark, and the refusals (a control byte in a string, bad UTF-8, a string open at the end).
fn files() -> Vec<Vec<u8>> {
    let mut v: Vec<Vec<u8>> = Vec::new();
    let mut f = b"[".to_vec();
    for i in 0..400 {
        f.extend_from_slice(format!("{}{{\"id\": {i}, \"s\": \"a\\\"b\\\\c é中 {}\", \"v\": [1.5e3, true, null, -7]}}", if i == 0 { "" } else { ",\n" }, "x".repeat(i % 70)).as_bytes());
    }
    f.push(b']');
    v.push(f);
    v.push(b"\xef\xbb\xbf{\"a\": [1, 2, {\"b\": \"\\\\\\\\\\\"\"}]}".to_vec());
    v.push(b"[\"a\x01b\"]".to_vec());
    v.push(b"[\"\xc3\x28\"]".to_vec());
    v.push(b"[\"open".to_vec());
    v.push(b"{}".to_vec());
    v.push(format!("[{}]", (0..3000).map(|i| format!("\"{}\"", "\\\\".repeat(i % 5))).collect::<Vec<_>>().join(",")).into_bytes());
    v
}

/// Tier 1's passes as UKL (`json_t1_ends`, `json_t1_from`) on the device are its Rust's, chunk by chunk: each chunk's
/// end from each lane's entry, its records' and each type's counts and its verdict from its entry, then its records and
/// each type's records' numbers written at their places, which are `lex_typed`'s.
#[test]
fn json_tier1_as_ukl_is_its_rust() {
    let Some(shared) = crate::shared::get() else {
        eprintln!("no device: tier 1 as UKL did not run");
        return;
    };
    let mut q = crate::device::ComputeQueue::new(&shared.queue);
    let p = program("json_tier1.uglykl", rerail_json_aot::TIER1_UKL).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()));
    let lanes = rerail_json_aot::LANES;
    let types = rerail_json_aot::TYPES;
    let (mut chunks_run, mut records) = (0usize, 0usize);
    for (k, f) in files().iter().enumerate() {
        let buf = rerail_json_aot::padded(f);
        let n = f.len();
        let off = if f.starts_with(&[0xef, 0xbb, 0xbf]) { 3 } else { 0 };
        let nw = (n - off).div_ceil(64);
        for blocks in [1usize, 3] {
            let chunks = nw.div_ceil(blocks).max(1);
            let args = |write: u32| T1Args { n: n as u32, blocks: blocks as u32, chunks: chunks as u32, write };
            // Pass 1.
            let want: Vec<[rerail_json_aot::Carries; 4]> = (0..chunks).map(|c| rerail_json_aot::stage1_ends_best(&buf, n, c * blocks, (c + 1) * blocks)).collect();
            let sb = buffer_of(&buf).unwrap();
            let eb = alloc::<u64>(chunks * lanes * 2).unwrap();
            launch(&q, p.function("json_t1_ends").unwrap()).value("args", &args(0)).buffer("src", &sb).buffer("ends", &eb).run(&[chunks as u64], &[1]).unwrap();
            q.flush().await_().unwrap();
            let got: Vec<u64> = read(&eb, chunks * lanes * 2);
            for (c, w) in want.iter().enumerate() {
                for (l, e) in w.iter().enumerate() {
                    let at = (c * lanes + l) * 2;
                    assert_eq!((got[at], got[at + 1]), (e.m_escaped, e.m_in_string), "{}: file {k}, chunks of {blocks}: chunk {c}, lane {l}'s end", crate::device::backend());
                }
            }
            // Pass 2, counting, from pass 3's entries.
            let ent = rerail_json_aot::entries(&want);
            let words: Vec<u64> = ent.iter().flat_map(|e| [e.m_escaped, e.m_in_string]).collect();
            let entb = buffer_of(&words).unwrap();
            let atb = alloc::<u32>(chunks * (types + 1)).unwrap();
            let (stb, tyb, rb) = (alloc::<u32>(1).unwrap(), alloc::<u32>(1).unwrap(), alloc::<u32>(chunks).unwrap());
            launch(&q, p.function("json_t1_from").unwrap()).value("args", &args(0)).buffer("src", &sb).buffer("entries", &entb).buffer("at", &atb).buffer("starts", &stb).buffer("types", &tyb).buffer("refused", &rb).run(&[chunks as u64], &[1]).unwrap();
            q.flush().await_().unwrap();
            let counts: Vec<u32> = read(&atb, chunks * (types + 1));
            let refused: Vec<u32> = read(&rb, chunks);
            let mut rust_refused = false;
            for c in 0..chunks {
                let mut at = [0usize; 4];
                // Counting writes nothing.
                let (r, _) = rerail_json_aot::stage1_from_best(&buf, n, c * blocks, (c + 1) * blocks, ent[c], &mut at, std::ptr::null_mut(), [std::ptr::null_mut(); 3], false);
                rust_refused |= r;
                let have: Vec<usize> = counts[c * (types + 1)..(c + 1) * (types + 1)].iter().map(|&x| x as usize).collect();
                assert_eq!(have, at.to_vec(), "{}: file {k}, chunks of {blocks}: chunk {c}'s counts", crate::device::backend());
                assert_eq!(refused[c] != 0, r, "{}: file {k}, chunks of {blocks}: chunk {c}'s verdict", crate::device::backend());
            }
            chunks_run += chunks;
            if rust_refused {
                continue;
            }
            // Pass 2, writing, at the places an exclusive scan of the counts gives: the records, then each type's list
            // after the types before it.
            let total: Vec<usize> = (0..=types).map(|t| (0..chunks).map(|c| counts[c * (types + 1) + t] as usize).sum()).collect();
            let mut places = vec![0u32; chunks * (types + 1)];
            let mut run = vec![0usize; types + 1];
            for t in 1..=types {
                run[t] = total[1..t].iter().sum();
            }
            for c in 0..chunks {
                for t in 0..=types {
                    places[c * (types + 1) + t] = run[t] as u32;
                    run[t] += counts[c * (types + 1) + t] as usize;
                }
            }
            let atb = buffer_of(&places).unwrap();
            let stb = alloc::<u32>(total[0].max(1)).unwrap();
            let tyb = alloc::<u32>(total[1..].iter().sum::<usize>().max(1)).unwrap();
            launch(&q, p.function("json_t1_from").unwrap()).value("args", &args(1)).buffer("src", &sb).buffer("entries", &entb).buffer("at", &atb).buffer("starts", &stb).buffer("types", &tyb).buffer("refused", &rb).run(&[chunks as u64], &[1]).unwrap();
            q.flush().await_().unwrap();
            let starts: Vec<u32> = read(&stb, total[0]);
            let lists: Vec<u32> = read(&tyb, total[1..].iter().sum());
            let typed = rerail_json_aot::lex_typed(&buf, n, 16);
            assert!(starts[..] == typed.lexed.starts[..total[0]], "{}: file {k}, chunks of {blocks}: the records", crate::device::backend());
            let mut at = 0;
            for t in 0..types {
                assert!(lists[at..at + total[1 + t]] == typed.types[t][..], "{}: file {k}, chunks of {blocks}: type {t}'s records", crate::device::backend());
                at += total[1 + t];
            }
            records += total[0];
        }
    }
    eprintln!("{}: tier 1 as UKL, {chunks_run} chunks in chunks of 1 and 3 blocks: each chunk's ends, counts and verdict its Rust's; {records} records and their types written as lex_typed's", crate::device::backend());
}

#[repr(C)]
#[derive(Clone, Copy)]
struct T2Args {
    n: u32,
    count: u32,
}
unsafe impl Pod for T2Args {}

/// Tier 2's token pass as UKL (`json_t2_tokens`, of `TIER2_UKL`) on the device, a thread a record of tier 1's type lists (one buffer,
/// as `json_t1_from` writes them): each record's token is Rust's `tokens`' (its end, first node's number, nodes, last
/// node's start, or the byte it is refused at).
#[test]
fn json_tokens_as_ukl_are_its_rust() {
    let Some(shared) = crate::shared::get() else {
        eprintln!("no device: the token pass as UKL did not run");
        return;
    };
    let mut q = crate::device::ComputeQueue::new(&shared.queue);
    let p = program("json_tier2.uglykl", rerail_json_aot::TIER2_UKL).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()));
    let mut more: Vec<Vec<u8>> = vec![
        b"[1, -0, 0.5, 1e9, -12.75E-3, 01, 1., -, 1e, true, false, null, tru, nul, \"\\ud83d\\ude00\", \"\\ud83d\", \"\\ude00\", \"\\u12\", \"\\x\", \"\"]".to_vec(),
        b"{\"k\": 3truex}".to_vec(),
    ];
    let mut all = files();
    all.append(&mut more);
    let (mut records, mut refused) = (0usize, 0usize);
    for (k, f) in all.iter().enumerate() {
        let buf = rerail_json_aot::padded(f);
        let typed = rerail_json_aot::lex_typed(&buf, f.len(), 16);
        if typed.lexed.refused {
            continue;
        }
        let want = rerail_json_aot::middle::tokens(&buf, &typed.lexed, &typed.types, &rerail_json_aot::middle::in_order);
        let list: Vec<u32> = typed.types.iter().flatten().copied().collect();
        let count = typed.lexed.count;
        let sb = buffer_of(&buf).unwrap();
        let stb = buffer_of(&typed.lexed.starts).unwrap();
        let lb = buffer_of(&list).unwrap();
        let tb = alloc::<u32>(count * 5).unwrap();
        let args = T2Args { n: list.len() as u32, count: count as u32 };
        launch(&q, p.function("json_t2_tokens").unwrap()).value("args", &args).buffer("src", &sb).buffer("starts", &stb).buffer("list", &lb).buffer("tokens", &tb).run(&[list.len() as u64], &[1]).unwrap();
        q.flush().await_().unwrap();
        let got: Vec<u32> = read(&tb, count * 5);
        for &r in &list {
            let r = r as usize;
            let w = want[r];
            assert_eq!(&got[r * 5..r * 5 + 5], &[w.end, w.kind, w.nodes, w.last, w.err], "{}: file {k}, record {r}'s token", crate::device::backend());
            refused += usize::from(w.err != rerail_json_aot::middle::NONE);
        }
        records += list.len();
    }
    eprintln!("{}: the token pass as UKL, {records} records: each token Rust's ({refused} refused)", crate::device::backend());
}

#[repr(C)]
#[derive(Clone, Copy)]
struct T2WArgs {
    chunks: u32,
    count: u32,
    len: u32,
    write: u32,
}
unsafe impl Pod for T2WArgs {}

/// Tier 2's pass 2 as UKL (`json_t2_write`) on the device: each chunk's nodes written from its state by a cursor from
/// the middle (`<ukl/writer_mid.h>`) on the writer's region, every chunk at once, counting its side-list entries and
/// then writing them; the region taken back and finished is the serial walk's tape, byte for byte. Each chunk's state
/// is Rust's passes 1 and 3 (`summaries`, `entries`), laid in the kernel's buffers.
#[test]
fn json_tier2_writes_from_the_middle_as_its_rust() {
    use rerail_json_aot::middle;
    let Some(shared) = crate::shared::get() else {
        eprintln!("no device: tier 2's pass 2 as UKL did not run");
        return;
    };
    let mut q = crate::device::ComputeQueue::new(&shared.queue);
    let p = program("json_tier2.uglykl", rerail_json_aot::TIER2_UKL).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()));
    let entries = rerail_json_aot::ENTRIES.iter().map(|&(kind, field, classes)| rerail_tape_format::tape::Entry { kind, field, flags: 0, alias: None, classes: classes.to_vec() }).collect();
    let lang = rerail_tape_format::f16::Lang::new(rerail_tape_format::tape::CodeTable::new(entries), rerail_json_aot::WIDTHS, 1).expect("a language");
    let writer = |f: &[u8]| rerail_tape_format::f16::Writer::new(&lang, f, rerail_tape_format::newlines::Terminators::Newlines, 0, false, true, f.len() / 2 + 16);
    const DEPTH: usize = 1024;
    let (mut tapes, mut chunks_run, mut bytes_out) = (0usize, 0usize, 0usize);
    for (k, f) in files().iter().enumerate() {
        let buf = rerail_json_aot::padded(f);
        let n = f.len();
        let lx = rerail_json_aot::lex(&buf, n);
        let mut w = writer(f);
        if rerail_json_aot::parse_lexed(&buf, &lx, &mut w).is_err() {
            continue;
        }
        let want = w.finish().expect("the serial tape").tape.as_bytes().to_vec();
        let types = rerail_json_aot::lex_typed(&buf, n, 16).types;
        for size in [1usize, 3, 16] {
            let chunks = middle::chunk_starts(&buf, &lx, size, &middle::in_order);
            let toks = middle::tokens(&buf, &lx, &types, &middle::in_order);
            let sums = middle::summaries(&buf, &lx, &toks, size, &middle::in_order);
            let ats = middle::entries(&sums, &middle::in_order);
            let last = ats.last().cloned().flatten().expect("the file's end");
            let mut m = writer(f);
            m.reserve_run(1 + lx.count + lx.len, (DEPTH + 1) * 3 + 1);
            m.final_as_written();
            m.make_blocks(last.nodes as usize).expect("the blocks");
            // Each chunk's state, its frames and levels, and its frames' room.
            let (mut words, mut frames, mut levels, mut room) = (Vec::new(), Vec::<u32>::new(), Vec::<u8>::new(), 0usize);
            for c in 0..chunks.len() {
                let at = ats[c].as_ref().expect("a chunk's state");
                let stop = chunks.get(c + 1).copied().unwrap_or(lx.count + 1);
                words.extend([at.entry.i as u32, at.entry.e as u32, at.entry.depth as u32, u32::from(at.entry.st), stop as u32, at.nodes, at.prev_start as u32, (at.prev_start >> 32) as u32]);
                words.extend([room as u32, at.frames.len() as u32, (frames.len() / 5) as u32, at.ret.len() as u32, levels.len() as u32]);
                for fr in &at.frames {
                    frames.extend([fr.node, fr.kids, fr.last, fr.start as u32, (fr.start >> 32) as u32]);
                }
                levels.extend(at.ret.iter());
                room += at.frames.len() + 3 * (stop - at.entry.i) + 4;
            }
            let nc = chunks.len();
            let region = m.device_region();
            let (sb, stb, tb) = (buffer_of(&buf).unwrap(), buffer_of(&lx.starts).unwrap(), buffer_of(&region).unwrap());
            frames.push(0);
            levels.push(0);
            let (ab, fb, lb) = (buffer_of(&words).unwrap(), buffer_of(&frames).unwrap(), buffer_of(&levels).unwrap());
            let (roomb, rb, ob) = (alloc::<u32>(room * 5).unwrap(), alloc::<u8>(nc * DEPTH).unwrap(), alloc::<u32>(nc * 2).unwrap());
            for write in [0u32, 1] {
                let args = T2WArgs { chunks: nc as u32, count: lx.count as u32, len: n as u32, write };
                launch(&q, p.function("json_t2_write").unwrap())
                    .value("args", &args)
                    .buffer("src", &sb)
                    .buffer("starts", &stb)
                    .buffer("t", &tb)
                    .buffer("ats", &ab)
                    .buffer("frames", &fb)
                    .buffer("levels", &lb)
                    .buffer("f", &roomb)
                    .buffer("rets", &rb)
                    .buffer("out", &ob)
                    .run(&[nc as u64], &[1])
                    .unwrap();
                q.flush().await_().unwrap();
                let out: Vec<u32> = read(&ob, nc * 2);
                for c in 0..nc {
                    let stop = chunks.get(c + 1).copied().unwrap_or(lx.count + 1);
                    let fine = if c + 1 == nc { out[2 * c] == 2 } else { out[2 * c] == 0 && out[2 * c + 1] as usize == stop };
                    assert!(fine, "{}: file {k}, chunks of {size}, run {write}: chunk {c} stopped as ({}, {})", crate::device::backend(), out[2 * c], out[2 * c + 1]);
                }
            }
            m.from_device_region(&read::<u8>(&tb, region.len()));
            m.end_middle(last.nodes as usize, last.prev_start);
            let have = m.finish().expect("the tape from the middle").tape.as_bytes().to_vec();
            assert!(have == want, "{}: file {k}, chunks of {size}: the tape differs at byte {:?} of {}", crate::device::backend(), have.iter().zip(&want).position(|(a, b)| a != b), want.len());
            tapes += 1;
            chunks_run += nc;
            bytes_out += have.len();
        }
    }
    eprintln!("{}: tier 2's pass 2 as UKL, {tapes} tapes in chunks of 1, 3 and 16 records ({chunks_run} chunks, each counted then written at once): the serial walk's tape, {bytes_out} bytes", crate::device::backend());
}


#[repr(C)]
#[derive(Clone, Copy)]
struct T2LArgs {
    chunks: u32,
    count: u32,
    len: u32,
    ntoks: u32,
}
unsafe impl Pod for T2LArgs {}

/// Nested JSON for pass 1: levels closed far below a chunk's entry, objects in arrays in objects, and closers that
/// do not match their openers (a refusal in a lane).
fn nested() -> Vec<Vec<u8>> {
    let mut v = Vec::new();
    let mut f = String::new();
    for i in 0..60 {
        f.push_str(if i % 2 == 0 { "[" } else { "{\"k\": " });
    }
    f.push('1');
    for i in (0..60).rev() {
        f.push_str(if i % 2 == 0 { ", 2]" } else { "}" });
    }
    v.push(f.into_bytes());
    v.push(b"{\"a\": [1, {\"b\": [2, 3, {\"c\": {}}], \"d\": []}, [[[\"x\"]]]], \"e\": {\"f\": [null, true]}}".to_vec());
    v.push(b"[{\"a\": [1, 2}, 3]".to_vec());
    v.push(b"{\"a\": {\"b\": [1, 2]]}".to_vec());
    v.push(format!("[{}]", (0..200).map(|i| format!("{{\"x\": [{}]}}", "[1, 2], ".repeat(i % 4) + "0")).collect::<Vec<_>>().join(", ")).into_bytes());
    v
}

/// Tier 2's pass 1 as UKL (`json_t2_lanes`) on the device: each chunk's summary, its lanes from each place its first
/// record can be at, forked at each level it closes below its entry and folded, is Rust's (`summaries`) lane for lane,
/// every chunk at once, the token pass's answers read.
#[test]
fn json_tier2_lanes_are_its_rust() {
    use rerail_json_aot::middle;
    let Some(shared) = crate::shared::get() else {
        eprintln!("no device: tier 2's pass 1 as UKL did not run");
        return;
    };
    let mut q = crate::device::ComputeQueue::new(&shared.queue);
    let p = program("json_tier2.uglykl", rerail_json_aot::TIER2_UKL).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()));
    const DEPTH: usize = 1024;
    const LANES: usize = 16;
    const RAW: usize = 64;
    let (mut runs, mut chunks_run, mut lanes, mut refusals) = (0usize, 0usize, 0usize, 0usize);
    for (k, f) in files().into_iter().chain(nested()).enumerate() {
        let buf = rerail_json_aot::padded(&f);
        let n = f.len();
        let lx = rerail_json_aot::lex(&buf, n);
        if lx.refused || lx.count == 0 {
            continue;
        }
        let types = rerail_json_aot::lex_typed(&buf, n, 16).types;
        let toks = middle::tokens(&buf, &lx, &types, &middle::in_order);
        let tw: Vec<u32> = toks.iter().flat_map(|t| [t.end, t.kind, t.nodes, t.last, t.err]).chain([0]).collect();
        for size in [1usize, 3, 16] {
            let chunks = middle::chunk_starts(&buf, &lx, size, &middle::in_order);
            let depths = middle::depths(&buf, &lx, &chunks, &middle::in_order);
            let want = middle::summaries(&buf, &lx, &toks, size, &middle::in_order);
            let nc = chunks.len();
            let room = lx.count + 1 + nc;
            let cw: Vec<u32> = chunks.iter().map(|&x| x as u32).collect();
            let dw: Vec<u32> = depths.iter().map(|&x| x as u32).collect();
            let pool_words = 64 * (lx.count + 1) + 4096 * nc;
            let mut pool = vec![0u32; pool_words];
            pool[0] = 2;
            pool[1] = pool_words as u32;
            let (sb, stb, tb, cb, db) = (buffer_of(&buf).unwrap(), buffer_of(&lx.starts).unwrap(), buffer_of(&tw).unwrap(), buffer_of(&cw).unwrap(), buffer_of(&dw).unwrap());
            let (rb, fb, thb, pb) = (alloc::<u8>(nc * DEPTH).unwrap(), alloc::<u32>(room * 10).unwrap(), alloc::<u32>(room * 2).unwrap(), alloc::<u32>(room * 2).unwrap());
            let (wb, mb, ob) = (alloc::<u32>(nc * RAW * 2).unwrap(), buffer_of(&pool).unwrap(), alloc::<u32>(nc * (2 + LANES)).unwrap());
            let args = T2LArgs { chunks: nc as u32, count: lx.count as u32, len: n as u32, ntoks: toks.len() as u32 };
            launch(&q, p.function("json_t2_lanes").unwrap())
                .value("args", &args)
                .buffer("src", &sb)
                .buffer("starts", &stb)
                .buffer("toks", &tb)
                .buffer("chunks", &cb)
                .buffer("depths", &db)
                .buffer("rets", &rb)
                .buffer("f", &fb)
                .buffer("touch", &thb)
                .buffer("path", &pb)
                .buffer("raw", &wb)
                .buffer("pool", &mb)
                .buffer("sums", &ob)
                .run(&[nc as u64], &[1])
                .unwrap();
            q.flush().await_().unwrap();
            let m: Vec<u32> = read(&mb, pool_words);
            let s: Vec<u32> = read(&ob, nc * (2 + LANES));
            for c in 0..nc {
                let h = &s[c * (2 + LANES)..(c + 1) * (2 + LANES)];
                assert_eq!(h[0], 0, "{}: file {k}, chunks of {size}: chunk {c}'s status", crate::device::backend());
                let have: Vec<middle::Lane> = h[2..2 + h[1] as usize].iter().map(|&at| lane_at(&m, at as usize)).collect();
                assert_eq!(have, want[c].lanes, "{}: file {k}, chunks of {size}: chunk {c}'s lanes", crate::device::backend());
                lanes += have.len();
                refusals += have.iter().map(|l| l.refuse.len()).sum::<usize>();
            }
            runs += 1;
            chunks_run += nc;
        }
    }
    eprintln!("{}: tier 2's pass 1 as UKL, {runs} files' runs in chunks of 1, 3 and 16 records ({chunks_run} chunks, {lanes} lanes, {refusals} refusals): each chunk's summary Rust's", crate::device::backend());
}

/// The lane at word `at` of a pass 1 pool (`<ukl/lanes_mid.h>`'s layout).
fn lane_at(m: &[u32], at: usize) -> rerail_json_aot::middle::Lane {
    use rerail_json_aot::middle::{Entry, Frame, Lane, Refusal, Touch};
    const NONE: u32 = u32::MAX;
    let h = &m[at..at + 17];
    let (nkc, np, nt, npf, nr) = (h[1] as usize, h[2] as usize, h[11] as usize, h[12] as usize, h[16] as usize);
    let mut w = at + 17;
    let mut take = |n: usize| {
        let v = m[w..w + n].to_vec();
        w += n;
        v
    };
    let kc = take(nkc).iter().map(|&x| x as u8).collect();
    let p = take(np).iter().map(|&x| x as u8).collect();
    let touch = take(2 * nt).chunks(2).map(|t| Touch { kids: t[0], last: (t[1] != NONE).then_some(t[1]) }).collect();
    let pf = take(5 * npf).chunks(5).map(|f| Frame { node: f[0], kids: f[1], last: f[2], start: u64::from(f[3]) | (u64::from(f[4]) << 32) }).collect();
    let refuse = take(5 * nr).chunks(5).map(|r| Refusal { level: r[0], places: r[1], err: r[2], nodes: r[3], last_start: r[4] }).collect();
    Lane {
        st0: h[0] as u8,
        kc,
        p,
        exit: Entry { i: h[3] as usize, e: h[4] as usize, depth: h[5] as usize, st: h[6] as u8 },
        nodes: h[7],
        end: match h[8] {
            0 => None,
            1 => Some(Ok(())),
            _ => Some(Err(h[9])),
        },
        fkc: h[10] as usize,
        touch,
        pf,
        last_start: (h[15] != 0).then(|| u64::from(h[13]) | (u64::from(h[14]) << 32)),
        refuse,
    }
}
