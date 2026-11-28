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


/// Tier 2 from the middle on the device as one program (`json_t2_run`, `<ukl/middle_run.h>`), from tier 1's records
/// and the token pass's answers to the finished tape: the chunk starts and depths, the bracket scan, each chunk's one
/// walk, the spans' scan and each chunk's state, the tape's blocks made, each chunk's nodes counted then written from
/// its state by a cursor from the middle on the writer's region, every block final, the tail and the presence laid,
/// all items of one work queue, launched a fixed few times. The tape taken from the region as the device left it is
/// the serial walk's finished tape, byte for byte.
#[test]
fn json_tier2_writes_from_the_middle_as_its_rust() {
    use rerail_json_aot::middle;
    let Some(shared) = crate::shared::get() else {
        eprintln!("no device: tier 2 as one program did not run");
        return;
    };
    let mut q = crate::device::ComputeQueue::new(&shared.queue);
    let p = program("json_tier2.uglykl", rerail_json_aot::TIER2_UKL).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()));
    let lang = json_lang();
    let (mut tapes, mut chunks_run, mut bytes_out, mut launches) = (0usize, 0usize, 0usize, 0usize);
    let (mut zbytes, mut zlaunches) = (0usize, 0usize);
    for (k, f) in files().into_iter().chain(nested()).enumerate() {
        let buf = rerail_json_aot::padded(&f);
        let n = f.len();
        let lx = rerail_json_aot::lex(&buf, n);
        let mut w = json_writer(&lang, &f);
        if rerail_json_aot::parse_lexed(&buf, &lx, &mut w).is_err() {
            continue;
        }
        let want = w.finish().expect("the serial tape").tape.as_bytes().to_vec();
        let types = rerail_json_aot::lex_typed(&buf, n, 16).types;
        let toks = middle::tokens(&buf, &lx, &types, &middle::in_order);
        for size in [1usize, 3, 16] {
            let mut m = json_writer(&lang, &f);
            let run = Run::go(&mut q, p, &buf, &lx, &toks, size, &mut m);
            let pool: Vec<u32> = read(&run.pool, run.words);
            assert_eq!((pool[4], pool[6], pool[14]), (1, 0, 1), "{}: file {k}, chunks of {size}: the verdict, failure and finish", crate::device::backend());
            let nc = pool[2] as usize;
            let out = &pool[run.o[5]..run.o[5] + 2 * nc];
            let chunks = middle::chunk_starts(&buf, &lx, size, &middle::in_order);
            for c in 0..nc {
                let stop = chunks.get(c + 1).copied().unwrap_or(lx.count + 1);
                let fine = if c + 1 == nc { out[2 * c] == 2 } else { out[2 * c] == 0 && out[2 * c + 1] as usize == stop };
                assert!(fine, "{}: file {k}, chunks of {size}: chunk {c} stopped as ({}, {})", crate::device::backend(), out[2 * c], out[2 * c + 1]);
            }
            // Then stored as 16z where it lies, by the second program.
            let (z, zl) = run.z(&mut q, p, most_blocks(&lx));
            let want_z = rerail_tape_format::f16::z_of(&want, &rerail_tape_format::f16::lz4_greedy).expect("16z");
            assert!(z == want_z, "{}: file {k}, chunks of {size}: the 16z tape differs at byte {:?} of {} ({} bytes)", crate::device::backend(), z.iter().zip(&want_z).position(|(a, b)| a != b), want_z.len(), z.len());
            zbytes += z.len();
            zlaunches = zlaunches.max(zl);
            m.from_device_region(&read::<u8>(&run.t, run.region));
            let have = m.finished_on_device().expect("the tape finished on the device").tape.as_bytes().to_vec();
            assert!(have == want, "{}: file {k}, chunks of {size}: the tape differs at byte {:?} of {} ({} bytes)", crate::device::backend(), have.iter().zip(&want).position(|(a, b)| a != b), want.len(), have.len());
            tapes += 1;
            chunks_run += nc;
            bytes_out += have.len();
            launches = launches.max(run.launches);
        }
    }
    eprintln!("{}: tier 2 from the middle as one program, the tape finished on the device, {tapes} tapes in chunks of 1, 3 and 16 records ({chunks_run} chunks, at most {launches} launches): the serial walk's tape, {bytes_out} bytes; stored as 16z on the device in at most {zlaunches} launches, z_of's, {zbytes} bytes", crate::device::backend());
}

/// JSON's language for format 16 writers.
fn json_lang() -> rerail_tape_format::f16::Lang {
    let entries = rerail_json_aot::ENTRIES.iter().map(|&(kind, field, classes)| rerail_tape_format::tape::Entry { kind, field, flags: 0, alias: None, classes: classes.to_vec() }).collect();
    rerail_tape_format::f16::Lang::new(rerail_tape_format::tape::CodeTable::new(entries), rerail_json_aot::WIDTHS, 1).expect("a language")
}

/// A format 16 writer of `f`'s tape, its blocks' presence kept.
fn json_writer(lang: &rerail_tape_format::f16::Lang, f: &[u8]) -> rerail_tape_format::f16::Writer {
    rerail_tape_format::f16::Writer::new(lang, f, rerail_tape_format::newlines::Terminators::Newlines, 0, false, true, f.len() / 2 + 16)
}

/// The most blocks a tape of the file's records and bytes has (`Run::go`'s room).
fn most_blocks(lx: &rerail_json_aot::Lexed) -> usize {
    (1 + lx.count + lx.len).div_ceil(1024).max(1)
}

#[repr(C)]
#[derive(Clone, Copy)]
struct T2ZArgs {
    capacity: u32,
    budget: u32,
}
unsafe impl Pod for T2ZArgs {}

#[repr(C)]
#[derive(Clone, Copy)]
struct T2RArgs {
    count: u32,
    len: u32,
    ntoks: u32,
    size: u32,
    runs: u32,
    most: u32,
    capacity: u32,
    budget: u32,
}
unsafe impl Pod for T2RArgs {}

/// Tier 2 from the middle run on the device as one program for one file in chunks of `size` records, into the
/// writer `m`'s region: its pool (the header's layout: `o`, the found runs, chunks, readiness, depths, states and
/// stops; `tree`, the five scans' trees, `tn` the chunks' trees' stride), the region, and the launches it took.
struct Run {
    pool: crate::device::Buffer,
    words: usize,
    t: crate::device::Buffer,
    region: usize,
    o: [usize; 6],
    tree: [usize; 5],
    tn: usize,
    launches: usize,
}

impl Run {
    fn go(q: &mut crate::device::ComputeQueue, p: &crate::kernels::Program, buf: &[u8], lx: &rerail_json_aot::Lexed, toks: &[rerail_json_aot::middle::Token], size: usize, m: &mut rerail_tape_format::f16::Writer) -> Run {
        const DEPTH: usize = 1024;
        // The writer's room for the most nodes a file of its records and bytes can have.
        let most = 1 + lx.count + lx.len;
        m.reserve_run(most, (DEPTH + 1) * 3 + 1);
        debug_assert_eq!(most.div_ceil(1024).max(1), most_blocks(lx));
        m.final_as_written();
        m.reserve_device_finish(most);
        let region = m.device_region();
        let blocks = most.div_ceil(1024).max(1);
        let runs = lx.count.div_ceil(size.max(1)).max(1);
        let (tn, tbn) = (2 * runs + 64, 2 * blocks + 64);
        // The pool's layout, after its 64 header words.
        let mut at = 64;
        let mut take = |n: usize| {
            let x = at;
            at += n;
            x
        };
        let o = [take(runs), take(runs + 1), take(runs), take(runs), take(runs + 1), take(2 * runs)];
        let tree = [take(3 * tn + 1), take(3 * tn + 1), take(3 * tn + 1), take(3 * tn + 1), take(3 * tbn + 1)];
        let fin = take(32);
        let words = at + 64 * (lx.count + 1) + 64 * runs + 16 * blocks + 4096;
        let mut pool = vec![0u32; words];
        pool[0] = at as u32;
        pool[1] = words as u32;
        for (k, &x) in o.iter().enumerate() {
            pool[16 + k] = x as u32;
        }
        for (k, &x) in tree.iter().enumerate() {
            pool[22 + k] = x as u32;
        }
        (pool[27], pool[28], pool[29], pool[30]) = (tn as u32, tbn as u32, fin as u32, at as u32);
        let tw: Vec<u32> = toks.iter().flat_map(|t| [t.end, t.kind, t.nodes, t.last, t.err]).chain([0]).collect();
        let room = lx.count + 1 + runs;
        // The queue: every item the chain can push, the first run's seeded.
        let capacity = 16 * (runs + blocks) + 5 * tn + tbn + lx.len / 256 + 4096;
        let mut slots = vec![0u32; capacity];
        slots[0] = 1;
        let (sb, stb, tb, mb, rb) = (buffer_of(buf).unwrap(), buffer_of(&lx.starts).unwrap(), buffer_of(&tw).unwrap(), buffer_of(&pool).unwrap(), buffer_of(&region).unwrap());
        let (retb, f1, th, f2) = (alloc::<u8>(runs * DEPTH).unwrap(), alloc::<u32>(room * 10).unwrap(), alloc::<u32>(room * 2).unwrap(), alloc::<u32>(room * 25).unwrap());
        let (qs, qc) = (buffer_of(&slots).unwrap(), buffer_of(&[0u32, 1, 1, 0, 0]).unwrap());
        // 64 groups of 64 threads, each SIMD group a worker of a budget twice an even share of the items.
        let (groups, threads) = (64u64, 64u64);
        let workers = (groups * threads) as usize / crate::device::simd_width().max(1) as usize;
        let args = T2RArgs { count: lx.count as u32, len: lx.len as u32, ntoks: toks.len() as u32, size: size as u32, runs: runs as u32, most: blocks as u32, capacity: capacity as u32, budget: (2 * capacity / workers + 16) as u32 };
        let mut launches = 0;
        loop {
            for _ in 0..2 {
                launch(q, p.function("json_t2_run").unwrap())
                    .value("args", &args)
                    .buffer("src", &sb)
                    .buffer("starts", &stb)
                    .buffer("toks", &tb)
                    .buffer("pool", &mb)
                    .buffer("t", &rb)
                    .buffer("rets", &retb)
                    .buffer("f1", &f1)
                    .buffer("touch", &th)
                    .buffer("f2", &f2)
                    .buffer("slots", &qs)
                    .buffer("control", &qc)
                    .run(&[groups], &[threads])
                    .unwrap();
                launches += 1;
            }
            q.flush().await_().unwrap();
            let control: Vec<u32> = read(&qc, 5);
            assert_eq!(control[3], 0, "{}: the work queue of {capacity} overflowed", crate::device::backend());
            if control[2] == 0 {
                break;
            }
            assert!(launches < 64, "{}: the work queue did not drain in {launches} launches ({} items left)", crate::device::backend(), control[2]);
        }
        Run { pool: mb, words, t: rb, region: region.len(), o, tree, tn, launches }
    }

    /// The tape the chain finished in the region, stored as 16z where it lies (`json_t2_z`, `<ukl/z16_run.h>`), for a
    /// tape of at most `blocks` blocks: its bytes, and the launches it took.
    fn z(&self, q: &mut crate::device::ComputeQueue, p: &crate::kernels::Program, blocks: usize) -> (Vec<u8>, usize) {
        let np = 4 * blocks;
        let room = self.region + 9 * np + 256;
        let words = 16 + 3 * np;
        let capacity = 2 * np + self.region / 4096 + 64;
        // COUNT(0) seeded (a slot holds its item plus one).
        let mut slots = vec![0u32; capacity];
        slots[0] = 1;
        let (zb, mb) = (alloc::<u8>(room).unwrap(), buffer_of(&vec![0u32; words]).unwrap());
        let (qs, qc) = (buffer_of(&slots).unwrap(), buffer_of(&[0u32, 1, 1, 0, 0]).unwrap());
        let groups = 64u64;
        let args = T2ZArgs { capacity: capacity as u32, budget: (2 * capacity / groups as usize + 16) as u32 };
        let mut launches = 0;
        loop {
            for _ in 0..2 {
                launch(q, p.function("json_t2_z").unwrap())
                    .value("args", &args)
                    .buffer("t", &self.t)
                    .buffer("z", &zb)
                    .buffer("pool", &mb)
                    .buffer("slots", &qs)
                    .buffer("control", &qc)
                    .run(&[groups], &[crate::device::simd_width() as u64])
                    .unwrap();
                launches += 1;
            }
            q.flush().await_().unwrap();
            let control: Vec<u32> = read(&qc, 5);
            assert_eq!(control[3], 0, "{}: the 16z queue of {capacity} overflowed", crate::device::backend());
            if control[2] == 0 {
                break;
            }
            assert!(launches < 64, "{}: the 16z queue did not drain in {launches} launches", crate::device::backend());
        }
        let m: Vec<u32> = read(&mb, 16);
        assert_eq!(m[2], 0, "{}: the 16z program failed", crate::device::backend());
        let end = m[3] as usize;
        assert!(end <= room, "{}: a 16z tape of {end} bytes past its room of {room}", crate::device::backend());
        (read::<u8>(&zb, end), launches)
    }

    /// Tree `tr`'s value and state words of node `x`.
    fn val(&self, m: &[u32], tr: usize, x: usize) -> u32 {
        m[self.tree[tr] + x]
    }
    fn st(&self, m: &[u32], tr: usize, x: usize) -> u32 {
        m[self.tree[tr] + self.tn + x]
    }
}




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

/// Tier 2 from the middle as one program on the device, each value it hands on Rust's: each chunk's first record
/// (`chunk_starts`) and depth (`depths`), its bracket word and its open levels' kinds (`kinds`), its one walk's span
/// (`span`), and its state and the file's end (`entries`), every chunk of the files, the nests and the broken words,
/// in chunks of 1, 3 and 16 records; the verdict the serial walk's.
#[test]
fn json_tier2_program_is_its_rust() {
    use rerail_json_aot::middle;
    let Some(shared) = crate::shared::get() else {
        eprintln!("no device: tier 2 as one program did not run");
        return;
    };
    let mut q = crate::device::ComputeQueue::new(&shared.queue);
    let compiling = std::time::Instant::now();
    let p = program("json_tier2.uglykl", rerail_json_aot::TIER2_UKL).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()));
    eprintln!("{}: tier 2's program compiled in {:?}", crate::device::backend(), compiling.elapsed());
    let lang = json_lang();
    let (mut runs, mut chunks_run, mut anti, mut broken, mut launches) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let mut ran = std::time::Duration::ZERO;
    for (k, f) in files().into_iter().chain(nested()).enumerate() {
        let buf = rerail_json_aot::padded(&f);
        let n = f.len();
        let lx = rerail_json_aot::lex(&buf, n);
        if lx.refused || lx.count == 0 {
            continue;
        }
        let mut w = json_writer(&lang, &f);
        let parsed = rerail_json_aot::parse_lexed(&buf, &lx, &mut w);
        let types = rerail_json_aot::lex_typed(&buf, n, 16).types;
        let toks = middle::tokens(&buf, &lx, &types, &middle::in_order);
        for size in [1usize, 3, 16] {
            let at = |what: &str| format!("{}: file {k}, chunks of {size}: {what}", crate::device::backend());
            let chunks = middle::chunk_starts(&buf, &lx, size, &middle::in_order);
            let ds = middle::depths(&buf, &lx, &chunks, &middle::in_order);
            let (words, kinds) = middle::kinds(&buf, &lx, &chunks, &middle::in_order);
            let ats = middle::entries(&buf, &lx, &toks, size, &middle::in_order);
            let mut mw = json_writer(&lang, &f);
            let started = std::time::Instant::now();
            let run = Run::go(&mut q, p, &buf, &lx, &toks, size, &mut mw);
            ran += started.elapsed();
            let m: Vec<u32> = read(&run.pool, run.words);
            assert!((m[0] as usize) <= run.words, "{}", at("the pool's room passed"));
            let nc = m[2] as usize;
            assert_eq!(nc, chunks.len(), "{}", at("the chunks"));
            assert_eq!(&m[run.o[1]..run.o[1] + nc], &chunks.iter().map(|&x| x as u32).collect::<Vec<_>>()[..], "{}", at("the chunks' starts"));
            assert_eq!(&m[run.o[3]..run.o[3] + nc], &ds.iter().map(|&x| x as u32).collect::<Vec<_>>()[..], "{}", at("the depths"));
            for c in 0..nc {
                let w = word_at(&m, offset(run.val(&m, 1, c), "a word").expect("a word"));
                assert_eq!(w, words[c], "{}", at(&format!("chunk {c}'s word")));
                let kd = offset(run.st(&m, 1, c), "kinds").map(|x| kinds_at(&m, x));
                assert_eq!(kd, kinds[c], "{}", at(&format!("chunk {c}'s kinds")));
                let stop = chunks.get(c + 1).copied().unwrap_or(lx.count + 1);
                let want = middle::span(&buf, &lx, &toks, chunks[c], stop, ds[c], &kinds[c], words[c].anti.len());
                let have = span_at(&m, offset(run.val(&m, 3, c), "a span").expect("a span"));
                assert_eq!(have, want, "{}", at(&format!("chunk {c}'s span")));
                anti += w.anti.len();
                broken += usize::from(w.broken);
            }
            let have: Vec<Option<middle::At>> = m[run.o[4]..run.o[4] + nc + 1].iter().map(|&x| offset(x, "a state").map(|x| at_at(&m, x))).collect();
            assert_eq!(have.len(), ats.len());
            for (c, (h, w)) in have.iter().zip(&ats).enumerate() {
                assert_eq!(h, w, "{}", at(&format!("chunk {c}'s state")));
            }
            // The verdict: the serial walk's.
            let verdict = match m[4] {
                1 => Ok(()),
                2 => Err(m[5]),
                v => panic!("{}", at(&format!("the verdict {v}, failure {}", m[6]))),
            };
            assert_eq!(verdict, parsed, "{}", at("the verdict"));
            runs += 1;
            chunks_run += nc;
            launches = launches.max(run.launches);
        }
    }
    eprintln!("{}: tier 2 as one program, {runs} files' runs in chunks of 1, 3 and 16 records ({chunks_run} chunks, {anti} anti-opens, {broken} broken words, at most {launches} launches, {ran:?} running): each chunk's start, depth, word, kinds, span and state Rust's, the verdict the serial walk's", crate::device::backend());
}
