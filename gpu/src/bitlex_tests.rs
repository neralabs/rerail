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

/// Tier 2 from the middle on the device, passes 1, 3 and 2 there: the bracket scan (`json_t2_words`, then the words'
/// tree up and the kinds down it), each chunk's one walk (`json_t2_spans`), the spans' scan and each chunk's state
/// (`json_t2_up`, `_root`, `_down`, `_levels`), the tape's blocks made (`json_t2_blocks`), each chunk's
/// nodes written from its state by a cursor from the middle (`<ukl/writer_mid.h>`) on the writer's region, every chunk
/// at once, counting its side-list entries then writing them (`json_t2_write`), and the tape ended (`json_t2_ended`).
/// The region taken back and finished is the serial walk's tape, byte for byte. The host reads nothing between the
/// passes: one command buffer.
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
    for (k, f) in files().into_iter().chain(nested()).enumerate() {
        let buf = rerail_json_aot::padded(&f);
        let n = f.len();
        let lx = rerail_json_aot::lex(&buf, n);
        let mut w = writer(&f);
        if rerail_json_aot::parse_lexed(&buf, &lx, &mut w).is_err() {
            continue;
        }
        let want = w.finish().expect("the serial tape").tape.as_bytes().to_vec();
        let types = rerail_json_aot::lex_typed(&buf, n, 16).types;
        let toks = middle::tokens(&buf, &lx, &types, &middle::in_order);
        for size in [1usize, 3, 16] {
            let chunks = middle::chunk_starts(&buf, &lx, size, &middle::in_order);
            let nc = chunks.len();
            let run = Mid::run(&q, p, &buf, &lx, &toks, size);
            let sb = &run.ats;
            // The writer's room for the most nodes a file of its records and bytes can have.
            let most = 1 + lx.count + lx.len;
            let mut m = writer(&f);
            m.reserve_run(most, (DEPTH + 1) * 3 + 1);
            m.final_as_written();
            let region = m.device_region();
            let blocks = most.div_ceil(1024).max(1);
            let cw: Vec<u32> = chunks.iter().map(|&x| x as u32).collect();
            let (sb_, stb, tb, cb) = (buffer_of(&buf).unwrap(), buffer_of(&lx.starts).unwrap(), buffer_of(&region).unwrap(), buffer_of(&cw).unwrap());
            let room = lx.count + 1 + nc;
            let (roomb, rb, ob) = (alloc::<u32>(room * 25).unwrap(), alloc::<u8>(nc * DEPTH).unwrap(), alloc::<u32>(nc * 2).unwrap());
            let bargs = T2BArgs { most: blocks as u32, end: nc as u32 };
            launch(&q, p.function("json_t2_blocks").unwrap()).value("args", &bargs).buffer("t", &tb).buffer("pool", &run.pool).buffer("states", sb).run(&[blocks as u64], &[1]).unwrap();
            for write in [0u32, 1] {
                let args = T2WArgs { chunks: nc as u32, count: lx.count as u32, len: n as u32, write };
                launch(&q, p.function("json_t2_write").unwrap())
                    .value("args", &args)
                    .buffer("src", &sb_)
                    .buffer("starts", &stb)
                    .buffer("t", &tb)
                    .buffer("chunks", &cb)
                    .buffer("pool", &run.pool)
                    .buffer("states", sb)
                    .buffer("f", &roomb)
                    .buffer("rets", &rb)
                    .buffer("out", &ob)
                    .run(&[nc as u64], &[1])
                    .unwrap();
            }
            launch(&q, p.function("json_t2_ended").unwrap()).value("args", &bargs).buffer("t", &tb).buffer("pool", &run.pool).buffer("states", sb).run(&[blocks as u64], &[1]).unwrap();
            q.flush().await_().unwrap();
            let out: Vec<u32> = read(&ob, nc * 2);
            for c in 0..nc {
                let stop = chunks.get(c + 1).copied().unwrap_or(lx.count + 1);
                let fine = if c + 1 == nc { out[2 * c] == 2 } else { out[2 * c] == 0 && out[2 * c + 1] as usize == stop };
                assert!(fine, "{}: file {k}, chunks of {size}: chunk {c} stopped as ({}, {})", crate::device::backend(), out[2 * c], out[2 * c + 1]);
            }
            m.from_device_region(&read::<u8>(&tb, region.len()));
            let have = m.finish().expect("the tape from the middle").tape.as_bytes().to_vec();
            assert!(have == want, "{}: file {k}, chunks of {size}: the tape differs at byte {:?} of {}", crate::device::backend(), have.iter().zip(&want).position(|(a, b)| a != b), want.len());
            tapes += 1;
            chunks_run += nc;
            bytes_out += have.len();
        }
    }
    eprintln!("{}: tier 2 from the middle as UKL, passes 1, 3 and 2 and the blocks on the device, {tapes} tapes in chunks of 1, 3 and 16 records ({chunks_run} chunks): the serial walk's tape, {bytes_out} bytes", crate::device::backend());
}

#[repr(C)]
#[derive(Clone, Copy)]
struct T2BArgs {
    most: u32,
    end: u32,
}
unsafe impl Pod for T2BArgs {}


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
/// do not match their openers (a broken word).
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

/// Tier 2 from the middle on the device for one file in chunks of `size` records: the bracket scan, each chunk's one
/// walk, and the spans' scan, a level of each tree a launch. Its pool, each scan's tree and states (a word a node,
/// the chunks' first; the states then the file's end), and each chunk's state with its levels (`ats`).
struct Mid {
    pool: crate::device::Buffer,
    pool_words: usize,
    nc: usize,
    words: crate::device::Buffer,
    kinds: crate::device::Buffer,
    spans: crate::device::Buffer,
    ats: crate::device::Buffer,
    launches: usize,
}

impl Mid {
    fn run(q: &crate::device::ComputeQueue, p: &crate::kernels::Program, buf: &[u8], lx: &rerail_json_aot::Lexed, toks: &[rerail_json_aot::middle::Token], size: usize) -> Mid {
        use rerail_json_aot::middle;
        const DEPTH: usize = 1024;
        let n = lx.len;
        let chunks = middle::chunk_starts(buf, lx, size, &middle::in_order);
        let depths = middle::depths(buf, lx, &chunks, &middle::in_order);
        let nc = chunks.len();
        let tw: Vec<u32> = toks.iter().flat_map(|t| [t.end, t.kind, t.nodes, t.last, t.err]).chain([0]).collect();
        let room = lx.count + 1 + nc;
        let cw: Vec<u32> = chunks.iter().map(|&x| x as u32).collect();
        let dw: Vec<u32> = depths.iter().map(|&x| x as u32).collect();
        let pool_words = 64 * (lx.count + 1) + 64 * nc + 4096;
        let mut pool = vec![0u32; pool_words];
        pool[0] = 4;
        pool[1] = pool_words as u32;
        let lv = levels(nc);
        let nodes = lv.iter().sum::<usize>();
        let bases: Vec<usize> = lv
            .iter()
            .scan(0, |a, &x| {
                let b = *a;
                *a += x;
                Some(b)
            })
            .collect();
        let (sb, stb, tb, cb, db) = (buffer_of(buf).unwrap(), buffer_of(&lx.starts).unwrap(), buffer_of(&tw).unwrap(), buffer_of(&cw).unwrap(), buffer_of(&dw).unwrap());
        let (rb, fb, thb, mb) = (alloc::<u8>(nc * DEPTH).unwrap(), alloc::<u32>(room * 10).unwrap(), alloc::<u32>(room * 2).unwrap(), buffer_of(&pool).unwrap());
        let (wt, ks, st, ss, ats) = (alloc::<u32>(nodes).unwrap(), alloc::<u32>(nodes + 1).unwrap(), alloc::<u32>(nodes).unwrap(), alloc::<u32>(nodes + 1).unwrap(), alloc::<u32>(nc + 1).unwrap());
        let args = T2LArgs { chunks: nc as u32, count: lx.count as u32, len: n as u32, ntoks: toks.len() as u32 };
        let mut launches = 0;
        launch(q, p.function("json_t2_words").unwrap()).value("args", &args).buffer("src", &sb).buffer("starts", &stb).buffer("chunks", &cb).buffer("pool", &mb).buffer("words", &wt).run(&[nc as u64], &[1]).unwrap();
        launches += 1;
        // A scan: its tree up, its root, its states down.
        let scan = |op: u32, tree: &crate::device::Buffer, states: &crate::device::Buffer| {
            let top = lv.len() - 1;
            for r in 0..top {
                let a = T2SArgs { n: lv[r] as u32, low: bases[r] as u32, up: bases[r + 1] as u32, end: 0, op };
                launch(q, p.function("json_t2_up").unwrap()).value("args", &a).buffer("pool", &mb).buffer("tree", tree).run(&[lv[r + 1] as u64], &[1]).unwrap();
            }
            let a = T2SArgs { n: 1, low: 0, up: bases[top] as u32, end: nodes as u32, op };
            launch(q, p.function("json_t2_root").unwrap()).value("args", &a).buffer("pool", &mb).buffer("tree", tree).buffer("states", states).run(&[1], &[1]).unwrap();
            for r in (0..top).rev() {
                let a = T2SArgs { n: lv[r] as u32, low: bases[r] as u32, up: bases[r + 1] as u32, end: 0, op };
                launch(q, p.function("json_t2_down").unwrap()).value("args", &a).buffer("pool", &mb).buffer("tree", tree).buffer("states", states).run(&[lv[r] as u64], &[1]).unwrap();
            }
            2 * top + 1
        };
        launches += scan(0, &wt, &ks);
        launch(q, p.function("json_t2_spans").unwrap())
            .value("args", &args)
            .buffer("src", &sb)
            .buffer("starts", &stb)
            .buffer("toks", &tb)
            .buffer("chunks", &cb)
            .buffer("depths", &db)
            .buffer("words", &wt)
            .buffer("kinds", &ks)
            .buffer("rets", &rb)
            .buffer("f", &fb)
            .buffer("touch", &thb)
            .buffer("pool", &mb)
            .buffer("spans", &st)
            .run(&[nc as u64], &[1])
            .unwrap();
        launches += 1;
        launches += scan(1, &st, &ss);
        let a = T2SArgs { n: nc as u32, low: 0, up: 0, end: nodes as u32, op: 0 };
        launch(q, p.function("json_t2_levels").unwrap()).value("args", &a).buffer("pool", &mb).buffer("words", &wt).buffer("kinds", &ks).buffer("states", &ss).buffer("ats", &ats).run(&[nc as u64 + 1], &[1]).unwrap();
        launches += 1;
        Mid { pool: mb, pool_words, nc, words: wt, kinds: ks, spans: st, ats, launches }
    }
}

/// The scan tree's level sizes over `n` values: the chunks', then each level half the one below, to one.
fn levels(n: usize) -> Vec<usize> {
    let mut v = vec![n.max(1)];
    while v[v.len() - 1] > 1 {
        v.push(v[v.len() - 1].div_ceil(2));
    }
    v
}

#[repr(C)]
#[derive(Clone, Copy)]
struct T2SArgs {
    n: u32,
    low: u32,
    up: u32,
    end: u32,
    op: u32,
}
unsafe impl Pod for T2SArgs {}

/// A value of a pool (`<ukl/middle.h>`'s records), `None` for `UKL_MID_NONE`; past the pool's room, a failure.
fn offset(x: u32, what: &str) -> Option<usize> {
    assert_ne!(x, u32::MAX - 1, "{}: {what} past the pool's room", crate::device::backend());
    (x != u32::MAX).then_some(x as usize)
}

/// The word at `at` of a pool.
fn word_at(m: &[u32], at: usize) -> rerail_json_aot::middle::Word {
    let (na, no) = (m[at] as usize, m[at + 1] as usize);
    rerail_json_aot::middle::Word { anti: m[at + 3..at + 3 + na].iter().map(|&x| x as u8).collect(), opens: m[at + 3 + na..at + 3 + na + no].iter().map(|&x| x as u8).collect(), broken: m[at + 2] != 0 }
}

/// The kinds at `at` of a pool.
fn kinds_at(m: &[u32], at: usize) -> Vec<u8> {
    m[at + 1..at + 1 + m[at] as usize].iter().map(|&x| x as u8).collect()
}

/// The span at `at` of a pool.
fn span_at(m: &[u32], at: usize) -> rerail_json_aot::middle::Span {
    use rerail_json_aot::middle::{Entry, Frame, Span, Touch};
    let h = &m[at..at + 13];
    let (nt, npf) = (h[8] as usize, h[9] as usize);
    let t0 = at + 13;
    let touch = m[t0..t0 + 2 * nt].chunks(2).map(|t| Touch { kids: t[0], last: (t[1] != u32::MAX).then_some(t[1]) }).collect();
    let f0 = t0 + 2 * nt;
    let pf = m[f0..f0 + 5 * npf].chunks(5).map(|f| Frame { node: f[0], kids: f[1], last: f[2], start: u64::from(f[3]) | (u64::from(f[4]) << 32) }).collect();
    Span {
        exit: Entry { i: h[0] as usize, e: h[1] as usize, depth: h[2] as usize, st: h[3] as u8 },
        nodes: h[4],
        end: match h[5] {
            0 => None,
            1 => Some(Ok(())),
            _ => Some(Err(h[6])),
        },
        fkc: h[7] as usize,
        touch,
        pf,
        last_start: (h[12] != 0).then(|| u64::from(h[10]) | (u64::from(h[11]) << 32)),
    }
}

/// The state at `at` of a pool.
fn at_at(m: &[u32], at: usize) -> rerail_json_aot::middle::At {
    use rerail_json_aot::middle::{At, Entry, Frame};
    let h = &m[at..at + 11];
    let (nret, nf) = (h[9] as usize, h[10] as usize);
    let ret = m[at + 11..at + 11 + nret].iter().map(|&x| x as u8).collect();
    let frames = m[at + 11 + nret..at + 11 + nret + 5 * nf].chunks(5).map(|f| Frame { node: f[0], kids: f[1], last: f[2], start: u64::from(f[3]) | (u64::from(f[4]) << 32) }).collect();
    At {
        entry: Entry { i: h[0] as usize, e: h[1] as usize, depth: h[2] as usize, st: h[3] as u8 },
        ret,
        nodes: h[4],
        end: match h[5] {
            0 => None,
            1 => Some(Ok(())),
            _ => Some(Err(h[6])),
        },
        frames,
        prev_start: u64::from(h[7]) | (u64::from(h[8]) << 32),
    }
}

/// Tier 2's bracket scan and pass 1 as UKL on the device (`json_t2_words`, the words' tree up and the kinds down it,
/// `json_t2_spans`): each chunk's word, its open levels' kinds at its first record, and its one walk's span are Rust's
/// (`Word::of`, `kinds`, `span`), every chunk at once, the token pass's answers read.
#[test]
fn json_tier2_spans_are_its_rust() {
    use rerail_json_aot::middle;
    let Some(shared) = crate::shared::get() else {
        eprintln!("no device: tier 2's pass 1 as UKL did not run");
        return;
    };
    let mut q = crate::device::ComputeQueue::new(&shared.queue);
    let p = program("json_tier2.uglykl", rerail_json_aot::TIER2_UKL).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()));
    let (mut runs, mut chunks_run, mut anti, mut broken) = (0usize, 0usize, 0usize, 0usize);
    for (k, f) in files().into_iter().chain(nested()).enumerate() {
        let buf = rerail_json_aot::padded(&f);
        let n = f.len();
        let lx = rerail_json_aot::lex(&buf, n);
        if lx.refused || lx.count == 0 {
            continue;
        }
        let types = rerail_json_aot::lex_typed(&buf, n, 16).types;
        let toks = middle::tokens(&buf, &lx, &types, &middle::in_order);
        for size in [1usize, 3, 16] {
            let chunks = middle::chunk_starts(&buf, &lx, size, &middle::in_order);
            let ds = middle::depths(&buf, &lx, &chunks, &middle::in_order);
            let (words, kinds) = middle::kinds(&buf, &lx, &chunks, &middle::in_order);
            let run = Mid::run(&q, p, &buf, &lx, &toks, size);
            q.flush().await_().unwrap();
            let nc = run.nc;
            assert_eq!(nc, chunks.len());
            let m: Vec<u32> = read(&run.pool, run.pool_words);
            assert!((m[0] as usize) <= run.pool_words, "{}: file {k}, chunks of {size}: the pool's room passed", crate::device::backend());
            let (wt, ks, st): (Vec<u32>, Vec<u32>, Vec<u32>) = (read(&run.words, nc), read(&run.kinds, nc), read(&run.spans, nc));
            for c in 0..nc {
                let w = word_at(&m, offset(wt[c], "a word").expect("a word"));
                assert_eq!(w, words[c], "{}: file {k}, chunks of {size}: chunk {c}'s word", crate::device::backend());
                let kd = offset(ks[c], "kinds").map(|x| kinds_at(&m, x));
                assert_eq!(kd, kinds[c], "{}: file {k}, chunks of {size}: chunk {c}'s kinds", crate::device::backend());
                let stop = chunks.get(c + 1).copied().unwrap_or(lx.count + 1);
                let want = middle::span(&buf, &lx, &toks, chunks[c], stop, ds[c], &kinds[c], words[c].anti.len());
                let have = span_at(&m, offset(st[c], "a span").expect("a span"));
                assert_eq!(have, want, "{}: file {k}, chunks of {size}: chunk {c}'s span", crate::device::backend());
                anti += w.anti.len();
                broken += usize::from(w.broken);
            }
            runs += 1;
            chunks_run += nc;
        }
    }
    eprintln!("{}: tier 2's bracket scan and pass 1 as UKL, {runs} files' runs in chunks of 1, 3 and 16 records ({chunks_run} chunks, {anti} anti-opens, {broken} broken words): each chunk's word, kinds and one walk's span Rust's", crate::device::backend());
}

/// Tier 2's pass 3 as UKL on the device, on the bracket scan and pass 1 there: the spans' tree up, a level a launch,
/// and the states down it, each chunk's state at its start with its levels and the file's end Rust's (`entries`),
/// state for state.
#[test]
fn json_tier2_entries_are_its_rust() {
    use rerail_json_aot::middle;
    let Some(shared) = crate::shared::get() else {
        eprintln!("no device: tier 2's pass 3 as UKL did not run");
        return;
    };
    let mut q = crate::device::ComputeQueue::new(&shared.queue);
    let p = program("json_tier2.uglykl", rerail_json_aot::TIER2_UKL).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()));
    let (mut runs, mut states, mut launches) = (0usize, 0usize, 0usize);
    for (k, f) in files().into_iter().chain(nested()).enumerate() {
        let buf = rerail_json_aot::padded(&f);
        let n = f.len();
        let lx = rerail_json_aot::lex(&buf, n);
        if lx.refused || lx.count == 0 {
            continue;
        }
        let types = rerail_json_aot::lex_typed(&buf, n, 16).types;
        let toks = middle::tokens(&buf, &lx, &types, &middle::in_order);
        for size in [1usize, 3, 16] {
            let want = middle::entries(&buf, &lx, &toks, size, &middle::in_order);
            let run = Mid::run(&q, p, &buf, &lx, &toks, size);
            q.flush().await_().unwrap();
            let m: Vec<u32> = read(&run.pool, run.pool_words);
            assert!((m[0] as usize) <= run.pool_words, "{}: file {k}, chunks of {size}: the pool's room passed", crate::device::backend());
            let have: Vec<Option<middle::At>> = read::<u32>(&run.ats, run.nc + 1).into_iter().map(|x| offset(x, "a state").map(|x| at_at(&m, x))).collect();
            assert_eq!(have.len(), want.len());
            for (c, (h, w)) in have.iter().zip(&want).enumerate() {
                assert_eq!(h, w, "{}: file {k}, chunks of {size}: chunk {c}'s state", crate::device::backend());
            }
            runs += 1;
            states += have.len();
            launches += run.launches;
        }
    }
    eprintln!("{}: tier 2's pass 3 as UKL, {runs} files' runs in chunks of 1, 3 and 16 records ({states} states, {launches} launches with the bracket scan and pass 1): each chunk's state and the file's end Rust's", crate::device::backend());
}
