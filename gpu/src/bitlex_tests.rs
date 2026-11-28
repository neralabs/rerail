//! Bitlex's tiers printed as UKL (decision 0726), on the process's device: each pass's kernels compute what the tier's
//! Rust computes, chunk by chunk.

use crate::kernels::{alloc, buffer_of, launch, program, read};
use crate::Pod;

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

/// Tier 1 and the token pass on the device from the file's bytes alone (`Lex`: `json_t1_lex`, `<ukl/lex_run.h>`, and
/// `json_t2_tokens`) are their Rust's: each chunk's entry (`entries`, the maps' scan) and counts, the refusal, and
/// where the file is not refused the records, each type's records and every record's token (`lex_typed`,
/// `middle::tokens`), for the tier 1 cases and the token errors (numbers, words, surrogate pairs, bad escapes), in
/// chunks of 1, 3 and 16 blocks.
#[test]
fn json_tier1_as_ukl_is_its_rust() {
    use rerail_json_aot::middle;
    let Some(shared) = crate::shared::get() else {
        eprintln!("no device: tier 1 as UKL did not run");
        return;
    };
    let mut q = crate::device::ComputeQueue::new(&shared.queue);
    let p1 = program("json_tier1.uglykl", rerail_json_aot::TIER1_UKL).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()));
    let p2 = program("json_tier2.uglykl", rerail_json_aot::TIER2_UKL).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()));
    let mut all = files();
    all.push(b"[1, -0, 0.5, 1e9, -12.75E-3, 01, 1., -, 1e, true, false, null, tru, nul, \"\\ud83d\\ude00\", \"\\ud83d\", \"\\ude00\", \"\\u12\", \"\\x\", \"\"]".to_vec());
    all.push(b"{\"k\": 3truex}".to_vec());
    let (mut chunks_run, mut records, mut tokens_refused, mut refusals) = (0usize, 0usize, 0usize, 0usize);
    for (k, f) in all.iter().enumerate() {
        let buf = rerail_json_aot::padded(f);
        let n = f.len();
        let off = if f.starts_with(&[0xef, 0xbb, 0xbf]) { 3 } else { 0 };
        let nw = (n - off).div_ceil(64);
        for blocks in [1usize, 3, 16] {
            let at = |what: &str| format!("{}: file {k}, chunks of {blocks}: {what}", crate::device::backend());
            let lex = Lex::go(&mut q, p1, p2, &buf, n, blocks);
            q.flush().await_().unwrap();
            let typed = rerail_json_aot::lex_typed(&buf, n, blocks);
            let (refused, count, typed_count) = lex.said();
            assert_eq!(refused, typed.lexed.refused, "{}", at("the verdict"));
            // Each chunk's entry and counts: Rust's chunks (the device's may have one more, past a byte-order mark,
            // empty).
            let rchunks = nw.div_ceil(blocks);
            let ends: Vec<[rerail_json_aot::Carries; 4]> = (0..rchunks).map(|c| rerail_json_aot::stage1_ends_best(&buf, n, c * blocks, (c + 1) * blocks)).collect();
            let ent = rerail_json_aot::entries(&ends);
            let pool: Vec<u32> = read(&lex.pool, 32 + 7 * (2 * lex.chunks + 64) + lex.chunks);
            let (ok, oe) = (pool[5] as usize, pool[6] as usize);
            for c in 0..rchunks {
                assert_eq!(pool[oe + c] as usize, rerail_json_aot::lane_of(ent[c]), "{}", at(&format!("chunk {c}'s entry")));
                let mut want = [0usize; 4];
                rerail_json_aot::stage1_from_best(&buf, n, c * blocks, (c + 1) * blocks, ent[c], &mut want, std::ptr::null_mut(), [std::ptr::null_mut(); 3], false);
                let have: Vec<usize> = pool[ok + 4 * c..ok + 4 * c + 4].iter().map(|&x| x as usize).collect();
                assert_eq!(have, want.to_vec(), "{}", at(&format!("chunk {c}'s counts")));
            }
            chunks_run += rchunks;
            if refused {
                refusals += 1;
                continue;
            }
            assert_eq!(count, typed.lexed.count, "{}", at("the records"));
            let starts: Vec<u32> = read(&lex.starts, count + 3);
            assert!(starts == typed.lexed.starts, "{}", at("the records' starts"));
            let lists: Vec<u32> = read(&lex.types, typed_count);
            assert!(lists == typed.types.concat(), "{}", at("each type's records"));
            let want = middle::tokens(&buf, &typed.lexed, &typed.types, &middle::in_order);
            let got: Vec<u32> = read(&lex.tokens, count * 5);
            for (r, w) in want.iter().enumerate() {
                assert_eq!(&got[r * 5..r * 5 + 5], &[w.end, w.kind, w.nodes, w.last, w.err], "{}", at(&format!("record {r}'s token")));
                tokens_refused += usize::from(w.err != middle::NONE);
            }
            records += count;
        }
    }
    eprintln!("{}: tier 1 and the token pass from the bytes, {chunks_run} chunks in chunks of 1, 3 and 16 blocks ({refusals} runs refused): each chunk's entry and counts Rust's; {records} records, their types and their tokens ({tokens_refused} refused) lex_typed's and tokens'", crate::device::backend());
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
    let p1 = program("json_tier1.uglykl", rerail_json_aot::TIER1_UKL).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()));
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
        for size in [1usize, 3, 16] {
            // From the file's bytes alone: tier 1 and the token pass, then tier 2.
            let mut m = json_writer(&lang, &f);
            let lex = Lex::go(&mut q, p1, p, &buf, n, LEX_BLOCKS);
            let run = Run::go(&mut q, p, &lex, size, &mut m);
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
            let (z, zl) = run.z(&mut q, p, most_blocks(n));
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

/// Timings of tier 2 from the middle on the device against the CPU's tiers, on a synthetic file of about 2 MB (objects
/// with strings, numbers, booleans and arrays, a nest every 50th), warm, the best of 3: tier 1 (`lex`), the token
/// pass, tier 2's serial walk into the writer (`parse_lexed`), its finish, and `z_of`, on the CPU; the two programs'
/// GPU time (the device's clock: `gpu_seconds_awaited`), in chunks of 16, 64 and 256 records. Run by name:
/// `--ignored json_tier2_timings`.
#[test]
#[ignore]
fn json_tier2_timings() {
    use rerail_json_aot::middle;
    let Some(shared) = crate::shared::get() else { return };
    let mut q = crate::device::ComputeQueue::new(&shared.queue);
    let p = program("json_tier2.uglykl", rerail_json_aot::TIER2_UKL).unwrap();
    let p1 = program("json_tier1.uglykl", rerail_json_aot::TIER1_UKL).unwrap();
    let lang = json_lang();
    let mut f = b"[".to_vec();
    let mut i = 0usize;
    while f.len() < 2 << 20 {
        let nest = if i % 50 == 0 { format!(", \"deep\": [[{{\"x\": [1, [2, {{\"y\": [{i}]}}]]}}]]") } else { String::new() };
        f.extend_from_slice(format!("{}{{\"id\": {i}, \"name\": \"item number {i}\", \"tags\": [\"a\", \"b{}\"], \"v\": {}.25, \"ok\": {}{nest}}}\n", if i == 0 { "" } else { "," }, i % 7, i * 3, i % 2 == 0).as_bytes());
        i += 1;
    }
    f.push(b']');
    let n = f.len();
    let buf = rerail_json_aot::padded(&f);
    let best = |run: &mut dyn FnMut()| {
        let mut b = std::time::Duration::MAX;
        for _ in 0..5 {
            let t = std::time::Instant::now();
            run();
            b = b.min(t.elapsed());
        }
        b
    };
    let lx = rerail_json_aot::lex(&buf, n);
    let t1 = best(&mut || drop(std::hint::black_box(rerail_json_aot::lex(&buf, n))));
    let types = rerail_json_aot::lex_typed(&buf, n, 16).types;
    let tt = best(&mut || drop(std::hint::black_box(middle::tokens(&buf, &lx, &types, &middle::in_order))));
    let mut want = Vec::new();
    let t2 = best(&mut || {
        let mut w = json_writer(&lang, &f);
        rerail_json_aot::parse_lexed(&buf, &lx, &mut w).unwrap();
        std::hint::black_box(&w);
    });
    let mut tf = std::time::Duration::MAX;
    for _ in 0..3 {
        let mut w = json_writer(&lang, &f);
        rerail_json_aot::parse_lexed(&buf, &lx, &mut w).unwrap();
        let t = std::time::Instant::now();
        want = w.finish().unwrap().tape.as_bytes().to_vec();
        tf = tf.min(t.elapsed());
    }
    let mut want_z = Vec::new();
    let tz = best(&mut || want_z = rerail_tape_format::f16::z_of(&want, &rerail_tape_format::f16::lz4_greedy).unwrap());
    eprintln!("{}: {n} bytes, {} records, a tape of {} bytes, 16z {} bytes", crate::device::backend(), lx.count, want.len(), want_z.len());
    eprintln!("CPU, one thread: tier 1 {t1:?}, the token pass {tt:?}, tier 2 serial {t2:?}, its finish {tf:?}, z_of {tz:?}");
    // The fair CPU paths, each its writer kept from round to round and its tape given back (`recycle`), as
    // `three_parsers_phase_by_phase` runs them: the lean parser (stage 1 and the walk) giving its events to format
    // 16's writer, and Bitlex's whole CPU path (tier 1, tier 2), each to 16 and to 16z.
    #[cfg(target_arch = "aarch64")]
    {
        use rerail_tape_format::f16::{Lang, Writer};
        use rerail_tape_format::tape::{CodeTable, Entry};
        let kinds = rerail_json_lean::Kinds { document: 1, object: 2, pair: 3, array: 4, string: 5, string_content: 6, escape_sequence: 7, number: 8, true_: 9, false_: 10, null: 11, comment: 12, key_field: 1, value_field: 2 };
        let mut lp = rerail_json_lean::Parser::new();
        let codes = lp.events(&f, &kinds, &mut |_| {}).expect("the lean parser parses it");
        let entry = |&(kind, field, in_value): &(u16, u16, bool)| Entry { kind, field, flags: 0, alias: None, classes: if in_value { vec![13] } else { vec![] } };
        let theirs = Lang::new(CodeTable::new(codes.iter().map(entry).collect()), rerail_json_aot::WIDTHS, 1).expect("the lean parser's language");
        let numbers: Vec<u32> = codes.iter().map(|c| theirs.number(&entry(c)).expect("numbered")).collect();
        let nodes = n / 2 + 16;
        let mut w = Writer::empty();
        let mut sizes = (0usize, 0usize);
        let mut lean = |z: bool| {
            w.begin(&theirs, &f, rerail_tape_format::newlines::Terminators::Newlines, 0, z, true, nodes);
            lp.events(&f, &kinds, &mut |words| {
                for e in words.as_chunks::<3>().0 {
                    let number = numbers[(e[0] >> 8) as usize];
                    match e[0] & 0xff {
                        rerail_simdjson::EV_OPEN => _ = w.open(number, u64::from(e[1])),
                        rerail_simdjson::EV_LEAF => _ = w.leaf(number, u64::from(e[1]), u64::from(e[2])),
                        _ => w.close(u64::from(e[1])),
                    }
                }
                w.commit();
            })
            .expect("parses");
            let t = w.finish().expect("finishes");
            if z { sizes.1 = t.tape.as_bytes().len() } else { sizes.0 = t.tape.as_bytes().len() }
            w.recycle(std::hint::black_box(t).tape);
        };
        let l16 = best(&mut || lean(false));
        let l16z = best(&mut || lean(true));
        let mut w = Writer::empty();
        let mut bitlex = |z: bool| {
            w.begin(&lang, &f, rerail_tape_format::newlines::Terminators::Newlines, 0, z, true, nodes);
            rerail_json_aot::parse(&buf, n, &mut w).expect("parses");
            let t = w.finish().expect("finishes");
            w.recycle(std::hint::black_box(t).tape);
        };
        let b16 = best(&mut || bitlex(false));
        let b16z = best(&mut || bitlex(true));
        eprintln!("CPU, one thread, writer kept: the lean parser to 16 {l16:?} ({} bytes), to 16z {l16z:?} ({} bytes); Bitlex's tiers to 16 {b16:?}, to 16z {b16z:?}", sizes.0, sizes.1);
    }
    for size in [16usize, 64, 256] {
        let (mut lexing, mut chain, mut z, mut wall) = (f64::MAX, f64::MAX, f64::MAX, std::time::Duration::MAX);
        let mut launches = (0, 0);
        for _ in 0..3 {
            let mut m = json_writer(&lang, &f);
            let (g0, t) = (crate::device::gpu_seconds_awaited(), std::time::Instant::now());
            let lex = Lex::go(&mut q, p1, p, &buf, n, LEX_BLOCKS);
            q.flush().await_().unwrap();
            let gl = crate::device::gpu_seconds_awaited();
            let run = Run::go(&mut q, p, &lex, size, &mut m);
            let g1 = crate::device::gpu_seconds_awaited();
            let (zb, zl) = run.z(&mut q, p, most_blocks(n));
            let g2 = crate::device::gpu_seconds_awaited();
            wall = wall.min(t.elapsed());
            assert!(zb == want_z, "chunks of {size}: the 16z tape");
            m.from_device_region(&read::<u8>(&run.t, run.region));
            assert!(m.finished_on_device().unwrap().tape.as_bytes() == &want[..], "chunks of {size}: the tape");
            (lexing, chain, z) = (lexing.min(gl - g0), chain.min(g1 - gl), z.min(g2 - g1));
            launches = (run.launches, zl);
        }
        eprintln!("{}, {} groups, chunks of {size} records: tier 1 and the token pass {:.2} ms GPU (4 launches), tier 2 to the finished tape {:.2} ms GPU ({} launches), 16z {:.2} ms GPU ({} launches), all {:.2} ms; wall with the test's buffers {wall:?}", crate::device::backend(), groups(), lexing * 1e3, chain * 1e3, launches.0, z * 1e3, launches.1, (lexing + chain + z) * 1e3);
    }
}

/// Each of tier 2's kernels compiled cold, its time: the program's text salted in its code (a comment's salt finds the
/// device's cache), each kernel's pipeline made by `function`. Run by name: `--ignored json_tier2_compile_times`.
#[test]
#[ignore]
fn json_tier2_compile_times() {
    if crate::shared::get().is_none() {
        return;
    }
    let salt = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u32 | 0x8000_0000;
    let text = rerail_json_aot::TIER2_UKL
        .replace("    r.most = args.most;\n", &format!("    r.most = args.count == {salt}u ? 0u : args.most;\n"))
        .replace("    r.table = table;\n", &format!("    r.table = table;\n    if (r.phase == {salt}u)\n        return;\n"));
    assert_ne!(text, rerail_json_aot::TIER2_UKL);
    let t = std::time::Instant::now();
    let p = program("json_tier2_salted.uglykl", Box::leak(text.into_boxed_str())).unwrap();
    eprintln!("{}: the library {:?}", crate::device::backend(), t.elapsed());
    let mut total = std::time::Duration::ZERO;
    for name in ["scan", "walk", "write"].map(|x| format!("json_t2_run_{x}")).into_iter().chain(["count", "write"].map(|x| format!("json_t2_z_{x}"))) {
        let t = std::time::Instant::now();
        p.function(&name).unwrap();
        total += t.elapsed();
        eprintln!("{}: {name} {:?}", crate::device::backend(), t.elapsed());
    }
    eprintln!("{}: every kernel {total:?}", crate::device::backend());
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

/// The programs' threadgroups: `RERAIL_T2_GROUPS` (the timings' knob), else 512 (on the M3 Max, 2 MB of JSON: tier 2's
/// phases 11.6 ms of GPU at 64, 7.4 at 512, 7.9 at 2,048).
fn groups() -> u64 {
    std::env::var("RERAIL_T2_GROUPS").ok().and_then(|v| v.parse().ok()).unwrap_or(512)
}

/// The most blocks a tape of a file of `n` bytes has (`Run::go`'s room).
fn most_blocks(n: usize) -> usize {
    (2 + 2 * n).div_ceil(1024).max(1)
}

/// Each of `<ukl/middle_run.h>`'s phases' kernel, in the phases' order.
const RUN_PHASES: [&str; 11] = ["scan", "scan", "scan", "scan", "walk", "scan", "write", "write", "scan", "scan", "scan"];

#[repr(C)]
#[derive(Clone, Copy)]
struct T2ZArgs {
    pad: u32,
    pad2: u32,
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
    phase: u32,
    pad: u32,
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

/// Tier 1 and the token pass on the device from the file's bytes alone (`json_t1_lex`, `<ukl/lex_run.h>`, then
/// `json_t2_tokens`): its pool (the header's layout, and what the phases said), the source, the records (`starts`),
/// each type's records (`types`, one list after another), each record's token, and the source's length. Every buffer
/// is sized by the length: a file of `n` bytes has at most `n + 1` records.
struct Lex {
    pool: crate::device::Buffer,
    src: crate::device::Buffer,
    starts: crate::device::Buffer,
    types: crate::device::Buffer,
    tokens: crate::device::Buffer,
    n: usize,
    chunks: usize,
}

/// Tier 1's chunks: `LEX_BLOCKS` blocks of 64 bytes (`lex_typed`'s 16).
const LEX_BLOCKS: usize = 16;

impl Lex {
    fn go(q: &mut crate::device::ComputeQueue, p1: &crate::kernels::Program, p2: &crate::kernels::Program, buf: &[u8], n: usize, blocks: usize) -> Lex {
        let chunks = n.div_ceil(64).div_ceil(blocks).max(1);
        let tn = 2 * chunks + 64;
        let (om, ok) = (32, 32 + 2 * tn);
        let oe = ok + 5 * tn;
        let words = oe + chunks + 64;
        let mut pool = vec![0u32; words];
        pool[..7].copy_from_slice(&[chunks as u32, n as u32, blocks as u32, tn as u32, om as u32, ok as u32, oe as u32]);
        let records = n + 1;
        let (pb, sb) = (buffer_of(&pool).unwrap(), buffer_of(buf).unwrap());
        let (stb, tyb, tkb) = (alloc::<u32>(records + 3).unwrap(), alloc::<u32>(records).unwrap(), alloc::<u32>(records * 5).unwrap());
        for phase in 0..3u32 {
            launch(q, p1.function("json_t1_lex").unwrap())
                .value("args", &T1LArgs { n: n as u32, phase })
                .buffer("src", &sb)
                .buffer("pool", &pb)
                .buffer("starts", &stb)
                .buffer("types", &tyb)
                .buffer("tokens", &tkb)
                .run(&[groups()], &[64])
                .unwrap();
        }
        // The token pass, a thread a typed record.
        let args = T2Args { n: records as u32, count: records as u32 };
        launch(q, p2.function("json_t2_tokens").unwrap())
            .value("args", &args)
            .buffer("src", &sb)
            .buffer("starts", &stb)
            .buffer("list", &tyb)
            .buffer("lexed", &pb)
            .buffer("tokens", &tkb)
            .run(&[records as u64], &[1])
            .unwrap();
        Lex { pool: pb, src: sb, starts: stb, types: tyb, tokens: tkb, n, chunks }
    }

    /// What tier 1 said: whether it refused the file, its records, its typed records.
    fn said(&self) -> (bool, usize, usize) {
        let h: Vec<u32> = read(&self.pool, 32);
        (h[8] != 0, h[9] as usize, h[10] as usize)
    }
}

/// The token pass's arguments: the threads, the records' room.
#[repr(C)]
#[derive(Clone, Copy)]
struct T2Args {
    n: u32,
    count: u32,
}
unsafe impl Pod for T2Args {}

#[repr(C)]
#[derive(Clone, Copy)]
struct T1LArgs {
    n: u32,
    phase: u32,
}
unsafe impl Pod for T1LArgs {}

impl Run {
    fn go(q: &mut crate::device::ComputeQueue, p: &crate::kernels::Program, lex: &Lex, size: usize, m: &mut rerail_tape_format::f16::Writer) -> Run {
        const DEPTH: usize = 1024;
        // The records and the nodes, bounded by the file's length (`Lex`): the buffers' room.
        let count = lex.n + 1;
        let most = 1 + count + lex.n;
        m.reserve_run(most, (DEPTH + 1) * 3 + 1);
        debug_assert_eq!(most.div_ceil(1024).max(1), most_blocks(lex.n));
        m.final_as_written();
        m.reserve_device_finish(most);
        let region = m.device_region();
        let blocks = most.div_ceil(1024).max(1);
        let runs = count.div_ceil(size.max(1)).max(1);
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
        let words = at + 64 * (count + 1) + 64 * runs + 16 * blocks + 4096;
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
        let room = count + 1 + runs;
        let (mb, rb) = (buffer_of(&pool).unwrap(), buffer_of(&region).unwrap());
        let (retb, f1, th, f2) = (alloc::<u8>(runs * DEPTH).unwrap(), alloc::<u32>(room * 10).unwrap(), alloc::<u32>(room * 2).unwrap(), alloc::<u32>(room * 25).unwrap());
        // A launch a phase, in one command buffer.
        let (groups, threads) = (groups(), 64u64);
        for (phase, kernel) in RUN_PHASES.iter().enumerate() {
            let args = T2RArgs { count: count as u32, len: lex.n as u32, ntoks: count as u32, size: size as u32, runs: runs as u32, most: blocks as u32, phase: phase as u32, pad: 0 };
            launch(q, p.function(&format!("json_t2_run_{kernel}")).unwrap())
                .value("args", &args)
                .buffer("src", &lex.src)
                .buffer("starts", &lex.starts)
                .buffer("toks", &lex.tokens)
                .buffer("pool", &mb)
                .buffer("t", &rb)
                .buffer("rets", &retb)
                .buffer("f1", &f1)
                .buffer("touch", &th)
                .buffer("f2", &f2)
                .buffer("lexed", &lex.pool)
                .run(&[groups], &[threads])
                .unwrap();
        }
        q.flush().await_().unwrap();
        let launches = RUN_PHASES.len();
        Run { pool: mb, words, t: rb, region: region.len(), o, tree, tn, launches }
    }

    /// The tape the chain finished in the region, stored as 16z where it lies (`json_t2_z`, `<ukl/z16_run.h>`), for a
    /// tape of at most `blocks` blocks: its bytes, and the launches it took.
    fn z(&self, q: &mut crate::device::ComputeQueue, p: &crate::kernels::Program, blocks: usize) -> (Vec<u8>, usize) {
        let np = 4 * blocks;
        let room = self.region + 9 * np + 256;
        let words = 24 + 3 * np;
        let (zb, mb) = (alloc::<u8>(room).unwrap(), buffer_of(&vec![0u32; words]).unwrap());
        // A launch a phase, in one command buffer.
        for phase in ["count", "write"] {
            launch(q, p.function(&format!("json_t2_z_{phase}")).unwrap())
                .value("args", &T2ZArgs { pad: 0, pad2: 0 })
                .buffer("t", &self.t)
                .buffer("z", &zb)
                .buffer("pool", &mb)
                .run(&[groups()], &[crate::device::simd_width() as u64])
                .unwrap();
        }
        q.flush().await_().unwrap();
        let launches = 2;
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
    let p1 = program("json_tier1.uglykl", rerail_json_aot::TIER1_UKL).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()));
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
            let lex = Lex::go(&mut q, p1, p, &buf, n, LEX_BLOCKS);
            let run = Run::go(&mut q, p, &lex, size, &mut mw);
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
