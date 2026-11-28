//! Bitlex's tiers printed as UKL (decision 0726), on the process's device: each pass's kernels compute what the tier's
//! Rust computes, chunk by chunk.

use crate::kernels::{alloc, launch, program, read};
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
    // The device's buffers, kept from file to file.
    let mut kept = Kept::default();
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
            let lex = Lex::go(&mut q, &mut kept, p1, p2, &buf, n, blocks);
            q.flush().await_().unwrap();
            let typed = rerail_json_aot::lex_typed(&buf, n, blocks);
            let (refused, count, typed_count) = lex.said();
            assert_eq!(refused, typed.lexed.refused, "{}", at("the verdict"));
            // Each chunk's entry and counts: Rust's chunks (the device's may have one more, past a byte-order mark,
            // empty).
            let rchunks = nw.div_ceil(blocks);
            let ends: Vec<[rerail_json_aot::Carries; 4]> = (0..rchunks).map(|c| rerail_json_aot::stage1_ends_best(&buf, n, c * blocks, (c + 1) * blocks)).collect();
            let ent = rerail_json_aot::entries(&ends);
            let pool: Vec<u32> = read(&lex.pool, 32 + 5 * (2 * lex.chunks + 64) + lex.chunks);
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
    // The device's buffers, kept from file to file.
    let mut kept = Kept::tight();
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
            let lex = Lex::go(&mut q, &mut kept, p1, p, &buf, n, LEX_BLOCKS);
            let run = Run::go(&mut q, &mut kept, p, &lex, size, &mut m);
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
            let (z, zl) = run.z(&mut q, &mut kept, p);
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
    assert!(kept.bails.iter().any(|b| b.0 == "tier 1's records") && kept.bails.iter().any(|b| b.0 == "tier 2's pool words"), "{}: no run bailed past its room: {:?}", crate::device::backend(), kept.bails);
    eprintln!("{}: {} runs bailed past their room and ran again with what they said they need, the first: {:?}", crate::device::backend(), kept.bails.len(), &kept.bails[..kept.bails.len().min(4)]);
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
    // The device's buffers, kept from file to file.
    let mut kept = Kept::default();
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
        // Each step's wall, the best round's: tier 1 (the upload, its launches and the wait), tier 2's host part, tier
        // 2's launches and wait, 16z (its launches, the wait and its bytes read).
        let mut steps = [std::time::Duration::MAX; 4];
        let (mut first, mut launches, mut used) = (std::time::Duration::ZERO, (0, 0), (0usize, 0usize, 0usize));
        let made = kept.made;
        // The writer kept too, its tape given back each round (`recycle`).
        let mut m = rerail_tape_format::f16::Writer::empty();
        for round in 0..5 {
            m.begin(&lang, &f, rerail_tape_format::newlines::Terminators::Newlines, 0, false, true, f.len() / 2 + 16);
            let (g0, t) = (crate::device::gpu_seconds_awaited(), std::time::Instant::now());
            let lex = Lex::go(&mut q, &mut kept, p1, p, &buf, n, LEX_BLOCKS);
            let (gl, tl) = (crate::device::gpu_seconds_awaited(), t.elapsed());
            let run = Run::go(&mut q, &mut kept, p, &lex, size, &mut m);
            let (g1, tr) = (crate::device::gpu_seconds_awaited(), t.elapsed());
            let (zb, zl) = run.z(&mut q, &mut kept, p);
            let (g2, tz) = (crate::device::gpu_seconds_awaited(), t.elapsed());
            if round == 0 {
                first = tz;
            }
            wall = wall.min(tz);
            for (k, d) in [tl, run.prep, tr - tl - run.prep, tz - tr].into_iter().enumerate() {
                steps[k] = steps[k].min(d);
            }
            assert!(zb == want_z, "chunks of {size}: the 16z tape");
            used = (read::<u32>(&run.pool, 1)[0] as usize, run.words, run.region);
            m.from_device_region(&read::<u8>(&run.t, run.region));
            let done = m.finished_on_device().unwrap();
            assert!(done.tape.as_bytes() == &want[..], "chunks of {size}: the tape");
            m.recycle(done.tape);
            (lexing, chain, z) = (lexing.min(gl - g0), chain.min(g1 - gl), z.min(g2 - g1));
            launches = (run.launches, zl);
            if !run.phase_gpu.is_empty() {
                eprintln!("  round {round}, each phase's GPU ms: {:?}", run.phase_gpu.iter().map(|g| (g * 1e4).round() / 10.0).collect::<Vec<_>>());
            }
        }
        eprintln!("{}, {} groups, chunks of {size} records: tier 1 and the token pass {:.2} ms GPU (4 launches), tier 2 to the finished tape {:.2} ms GPU ({} launches), 16z {:.2} ms GPU ({} launches), all {:.2} ms", crate::device::backend(), groups(), lexing * 1e3, chain * 1e3, launches.0, z * 1e3, launches.1, (lexing + chain + z) * 1e3);
        eprintln!("  wall: the first round {first:?} (buffers made: {}), the best {wall:?}: tier 1 {:?}, tier 2's host part {:?}, tier 2 {:?}, 16z {:?}; tier 2's pool {} of {} words used, the writer's region {} bytes; buffers held {} MB", kept.made - made, steps[0], steps[1], steps[2], steps[3], used.0, used.1, used.2, kept.held() >> 20);
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
    /// The most blocks its tape has (its room's).
    blocks: usize,
    /// Each phase's GPU seconds, where `RERAIL_T2_PHASE_TIMES` asks for them (a command buffer a phase); else empty.
    phase_gpu: Vec<f64>,
    /// The host's part before the launches: the writer's region made and handed over, the pool's words set.
    prep: std::time::Duration,
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

/// The device's buffers kept from file to file, as the CPU's writer is: each made once and grown (by a quarter more)
/// only when a file needs more room than it has, so a file that fits makes none. A run sets again only the words it
/// reads before it writes them (its counters, arrivals and layout); the rest it writes first.
struct Kept {
    bufs: std::collections::HashMap<&'static str, crate::device::Buffer>,
    /// The buffers made, and their bytes.
    made: usize,
    bytes: usize,
    /// The run's epoch (`ukl_run_meet`): new each run, never 0 (a new arrivals buffer's words) nor 0xa5a5a5a5.
    epoch: u32,
    /// What files so far took: their bytes, records, tier 2's runs and pool words; the room for a file is twice what
    /// they say on average, and a bit more.
    seen: [f64; 4],
    /// The runs that bailed past a room, and the room each said it needs (records, or pool words).
    bails: Vec<(&'static str, usize)>,
}

impl Default for Kept {
    fn default() -> Kept {
        // Before any file, JSON's average: 0.3 records a byte; tier 2's pool half a word a record and 350 a run.
        Kept { bufs: Default::default(), made: 0, bytes: 0, epoch: 0, seen: [1e6, 3e5, 0.0, 0.0], bails: Vec::new() }
    }
}

impl Kept {
    /// Kept buffers whose averages say far too little (a record in 10,000 bytes, a pool a tenth its use), so a run
    /// bails past its room and runs again with what it says it needs.
    fn tight() -> Kept {
        let mut k = Kept::default();
        k.seen = [1e6, 100.0, 1e4, 0.1 * (0.5 * 100.0 + 350.0 * 1e4)];
        k
    }

    /// Buffer `name`, of at least `bytes` bytes.
    fn get(&mut self, name: &'static str, bytes: usize) -> crate::device::Buffer {
        let bytes = bytes.max(64);
        if let Some(b) = self.bufs.get(name)
            && b.length() as usize >= bytes
        {
            return b.clone();
        }
        let size = bytes + bytes / 4;
        let b = alloc::<u8>(size).unwrap();
        // The tests' check that the device writes before it reads: a new buffer's bytes all 0xa5
        // (`RERAIL_T2_POISON`), as the writer's `FRESH`.
        if std::env::var_os("RERAIL_T2_POISON").is_some() {
            // SAFETY: the buffer holds `size` bytes.
            unsafe { std::ptr::write_bytes(b.contents().cast::<u8>(), 0xa5, size) };
        }
        self.made += 1;
        self.bytes += size;
        self.bufs.insert(name, b.clone());
        b
    }

    /// A new run's epoch.
    fn next_epoch(&mut self) -> u32 {
        loop {
            self.epoch = self.epoch.wrapping_add(1);
            if self.epoch != 0 && self.epoch != 0xa5a5_a5a5 {
                return self.epoch;
            }
        }
    }

    /// Arrivals buffer `name` of at least `words` words (only ever epochs: 0 where new, once, as it is made).
    fn meet(&mut self, name: &'static str, words: usize) -> crate::device::Buffer {
        let made = self.made;
        let b = self.get(name, 4 * words);
        if self.made != made {
            // SAFETY: the buffer is new, and holds its length's bytes.
            unsafe { std::ptr::write_bytes(b.contents().cast::<u8>(), 0, b.length() as usize) };
        }
        b
    }

    /// The records' room for a file of `n` bytes: twice the average so far, and a bit.
    fn records_room(&self, n: usize) -> usize {
        (2.0 * n as f64 * self.seen[1] / self.seen[0]) as usize + 1024
    }

    /// Tier 2's pool's free words for `count` records in `runs` runs: twice the average so far, and a bit (before a
    /// run, JSON's: half a word a record and 350 a run).
    fn pool_room(&self, count: usize, runs: usize) -> usize {
        let per = if self.seen[2] > 0.0 { self.seen[3] / (0.5 * self.seen[1] + 350.0 * self.seen[2]) } else { 1.0 };
        (2.0 * per * (0.5 * count as f64 + 350.0 * runs as f64)) as usize + 4096
    }

    /// The bytes of the buffers held now.
    fn held(&self) -> usize {
        self.bufs.values().map(|b| b.length() as usize).sum()
    }
}

/// A buffer's first `n` words, written in place (the host's view of the device's memory).
fn words_of(b: &crate::device::Buffer, n: usize) -> &mut [u32] {
    assert!(n * 4 <= b.length() as usize);
    // SAFETY: the buffer holds `n` words; no launch runs while the host writes them.
    unsafe { std::slice::from_raw_parts_mut(b.contents().cast::<u32>(), n) }
}

impl Lex {
    /// Tier 1 and the token pass of the file `buf` (`n` bytes, padded), in buffers `kept` holds; the host waits for
    /// them, since tier 2's room is sized by the records they find.
    fn go(q: &mut crate::device::ComputeQueue, kept: &mut Kept, p1: &crate::kernels::Program, p2: &crate::kernels::Program, buf: &[u8], n: usize, blocks: usize) -> Lex {
        let chunks = n.div_ceil(64).div_ceil(blocks).max(1);
        let tn = 2 * chunks + 64;
        let (om, ok) = (32, 32 + tn);
        let oe = ok + 4 * tn;
        let words = oe + chunks + 64;
        // The source, uploaded (the one thing the host sends).
        let sb = kept.get("src", buf.len());
        // SAFETY: the buffer holds `buf`'s bytes.
        unsafe { std::ptr::copy_nonoverlapping(buf.as_ptr(), sb.contents().cast::<u8>(), buf.len()) };
        let mut records = kept.records_room(n);
        loop {
            let lex = Lex::once(q, kept, p1, p2, &sb, n, blocks, chunks, [tn, om, ok, oe, words], records);
            let need = read::<u32>(&lex.pool, 32)[23] as usize;
            if need == 0 {
                let count = lex.said().1;
                kept.seen[0] += n as f64;
                kept.seen[1] += count as f64;
                return lex;
            }
            // Past the room: run again with what it needs.
            kept.bails.push(("tier 1's records", need));
            records = need;
        }
    }

    /// One run of tier 1 and the token pass, with room for `records` records.
    #[allow(clippy::too_many_arguments)]
    fn once(q: &mut crate::device::ComputeQueue, kept: &mut Kept, p1: &crate::kernels::Program, p2: &crate::kernels::Program, sb: &crate::device::Buffer, n: usize, blocks: usize, chunks: usize, [tn, om, ok, oe, words]: [usize; 5], records: usize) -> Lex {
        let pb = kept.get("lex pool", 4 * words);
        let epoch = kept.next_epoch();
        {
            // The layout, and the header's words the phases count into.
            let pool = words_of(&pb, 32);
            pool[..7].copy_from_slice(&[chunks as u32, n as u32, blocks as u32, tn as u32, om as u32, ok as u32, oe as u32]);
            pool[7..32].fill(0);
            (pool[21], pool[22]) = (epoch, records as u32);
        }
        let mb = kept.meet("lex meet", 2 * tn);
        let sb = sb.clone();
        let (stb, tyb, tkb) = (kept.get("starts", 4 * (records + 3)), kept.get("types", 4 * records), kept.get("tokens", 20 * records));
        for phase in 0..3u32 {
            launch(q, p1.function("json_t1_lex").unwrap())
                .value("args", &T1LArgs { n: n as u32, phase })
                .buffer("src", &sb)
                .buffer("pool", &pb)
                .buffer("starts", &stb)
                .buffer("types", &tyb)
                .buffer("tokens", &tkb)
                .buffer("meet", &mb)
                .run(&[groups()], &[64])
                .unwrap();
        }
        // The token pass, a thread a typed record (at most the room's), 64 a group.
        let args = T2Args { n: records.next_multiple_of(64) as u32, count: records as u32 };
        launch(q, p2.function("json_t2_tokens").unwrap())
            .value("args", &args)
            .buffer("src", &sb)
            .buffer("starts", &stb)
            .buffer("list", &tyb)
            .buffer("lexed", &pb)
            .buffer("tokens", &tkb)
            .run(&[records.div_ceil(64) as u64], &[64])
            .unwrap();
        q.flush().await_().unwrap();
        Lex { pool: pb, src: sb, starts: stb, types: tyb, tokens: tkb, n, chunks }
    }

    /// What tier 1 said: whether it refused the file, its records, its typed records.
    fn said(&self) -> (bool, usize, usize) {
        let h: Vec<u32> = read(&self.pool, 32);
        (h[8] != 0, h[9] as usize, h[10] as usize)
    }

    /// The token pass's tokens' nodes, summed.
    fn token_nodes(&self) -> usize {
        read::<u32>(&self.pool, 32)[20] as usize
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
    fn go(q: &mut crate::device::ComputeQueue, kept: &mut Kept, p: &crate::kernels::Program, lex: &Lex, size: usize, m: &mut rerail_tape_format::f16::Writer) -> Run {
        let count = lex.said().1;
        let runs = count.div_ceil(size.max(1)).max(1);
        let mut room = kept.pool_room(count, runs);
        loop {
            let run = Run::once(q, kept, p, lex, size, m, room);
            let h: Vec<u32> = read(&run.pool, 8);
            if h[4] == 3 && h[0] as usize > run.words {
                // Past the pool's room: the words it took are what it needs.
                let need = h[0] as usize - (run.words - room);
                kept.bails.push(("tier 2's pool words", need));
                room = need + 4096;
                continue;
            }
            if h[4] == 1 {
                (kept.seen[2], kept.seen[3]) = (kept.seen[2] + runs as f64, kept.seen[3] + (h[0] as usize - (run.words - room)) as f64);
            }
            return run;
        }
    }

    /// One run of tier 2, with `room` free words in its pool.
    fn once(q: &mut crate::device::ComputeQueue, kept: &mut Kept, p: &crate::kernels::Program, lex: &Lex, size: usize, m: &mut rerail_tape_format::f16::Writer, room: usize) -> Run {
        const DEPTH: usize = 1024;
        let t0 = std::time::Instant::now();
        // The records tier 1 found and their tokens' nodes, the room's measure: a record opens at most one node besides
        // its token's (the device checks it: `nodes` below).
        let count = lex.said().1;
        let most = 1 + count + lex.token_nodes();
        m.reserve_run(most, (DEPTH + 1) * 3 + 1);
        m.final_as_written();
        m.reserve_device_finish(most);
        let region = m.device_region_len();
        if std::env::var_os("RERAIL_T2_PARTS").is_some() {
            eprintln!("the writer's parts for {count} records, {most} nodes at most: {:?}", m.part_lens());
        }
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
        let tree = [take(2 * tn + 1), take(2 * tn + 1), take(2 * tn + 1), take(2 * tn + 1), take(2 * tbn + 1)];
        let fin = take(32);
        let words = at + room;
        let mb = kept.get("pool", 4 * words);
        let epoch = kept.next_epoch();
        let ms = tn.max(tbn);
        let meet = kept.meet("meet", 5 * ms);
        {
            // The layout, the header's counters and verdict, and the finish's words. Everything else a run writes
            // before it reads.
            let pool = words_of(&mb, at);
            pool[..64].fill(0);
            pool[0] = at as u32;
            pool[1] = words as u32;
            for (k, &x) in o.iter().enumerate() {
                pool[16 + k] = x as u32;
            }
            for (k, &x) in tree.iter().enumerate() {
                pool[22 + k] = x as u32;
            }
            (pool[27], pool[28], pool[29], pool[30]) = (tn as u32, tbn as u32, fin as u32, at as u32);
            (pool[60], pool[61]) = (epoch, ms as u32);
            pool[fin..fin + 32].fill(0);
        }
        let room = count + 1 + runs;
        let rb = kept.get("region", region);
        // The state and each part's bytes in use, in place in the kept buffer.
        // SAFETY: the buffer holds `region` bytes; no launch runs while the host writes them.
        m.device_region_into(unsafe { std::slice::from_raw_parts_mut(rb.contents().cast::<u8>(), region) });
        let (retb, f1, th, f2) = (kept.get("rets", runs * DEPTH), kept.get("f1", 40 * room), kept.get("touch", 8 * room), kept.get("f2", 100 * room));
        let prep = t0.elapsed();
        // A launch a phase, in one command buffer.
        let (groups, threads) = (groups(), 64u64);
        let each = std::env::var_os("RERAIL_T2_PHASE_TIMES").is_some();
        let mut phase_gpu = Vec::new();
        for (phase, kernel) in RUN_PHASES.iter().enumerate() {
            let g = crate::device::gpu_seconds_awaited();
            let args = T2RArgs { count: count.max(1) as u32, len: lex.n as u32, ntoks: count as u32, size: size as u32, runs: runs as u32, most: blocks as u32, phase: phase as u32, pad: 0 };
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
                .buffer("meet", &meet)
                .run(&[groups], &[threads])
                .unwrap();
            if each {
                q.flush().await_().unwrap();
                phase_gpu.push(crate::device::gpu_seconds_awaited() - g);
            }
        }
        q.flush().await_().unwrap();
        let launches = RUN_PHASES.len();
        let h: Vec<u32> = read(&mb, 8);
        assert!(h[4] != 1 || h[7] as usize <= most, "{}: {} nodes past the room's {most}", crate::device::backend(), h[7]);
        Run { pool: mb, words, t: rb, region, o, tree, tn, launches, prep, blocks, phase_gpu }
    }

    /// The tape the chain finished in the region, stored as 16z where it lies (`json_t2_z`, `<ukl/z16_run.h>`), for a
    /// tape of at most its run's blocks: its bytes, and the launches it took.
    fn z(&self, q: &mut crate::device::ComputeQueue, kept: &mut Kept, p: &crate::kernels::Program) -> (Vec<u8>, usize) {
        let np = 4 * self.blocks;
        let room = self.region + 9 * np + 256;
        let words = 24 + 3 * np;
        let (zb, mb) = (kept.get("z", room), kept.get("z pool", 4 * words));
        // Its header (the counters, the tickets); each piece's words its count phase writes.
        words_of(&mb, 24).fill(0);
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
    // The device's buffers, kept from file to file.
    let mut kept = Kept::default();
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
            let lex = Lex::go(&mut q, &mut kept, p1, p, &buf, n, LEX_BLOCKS);
            let run = Run::go(&mut q, &mut kept, p, &lex, size, &mut mw);
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
