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
            // K's values: each chunk's records, each type's and its newlines.
            let kw = rerail_json_aot::TYPES + 2;
            let pool: Vec<u32> = read(&lex.pool, 32 + (1 + kw) * (2 * lex.chunks + 64) + lex.chunks);
            let (ok, oe) = (pool[5] as usize, pool[6] as usize);
            for c in 0..rchunks {
                assert_eq!(pool[oe + c] as usize, rerail_json_aot::lane_of(ent[c]), "{}", at(&format!("chunk {c}'s entry")));
                let mut want = [0usize; rerail_json_aot::TYPES + 2];
                rerail_json_aot::stage1_from_best(&buf, n, c * blocks, (c + 1) * blocks, ent[c], &mut want, std::ptr::null_mut(), [std::ptr::null_mut(); 3], false, std::ptr::null_mut(), [0; 4]);
                let have: Vec<usize> = pool[ok + kw * c..ok + kw * c + kw].iter().map(|&x| x as usize).collect();
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

/// Timings of tier 2 as phases on the device against the CPU's tiers, on a synthetic file of about 2 MB (objects with
/// strings, numbers, booleans and arrays, a nest every 50th), warm, the best of 5: tier 1 (`lex`), the token pass,
/// tier 2's serial walk into the writer (`parse_lexed`), its finish, and `z_of`, on the CPU; the device's GPU time
/// (its clock: `gpu_seconds_awaited`) for tier 1 and for tier 2 with 16z, in chunks of 16, 64 and 256 records. Run by name:
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
    let f = json_of(2 << 20, 0);
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
    let want_z = phases_z(&lang, &f).expect("the 16z tape by phases");
    let tz = best(&mut || drop(std::hint::black_box(rerail_tape_format::f16::z_of(&want, &rerail_tape_format::f16::lz4_greedy).unwrap())));
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
    // `RERAIL_T2_PHASE_TIMES`: each phase's GPU time, a command buffer a phase (the old program's switch too).
    let timed = std::env::var_os("RERAIL_T2_PHASE_TIMES").is_some();
    for size in [16usize, 64, 256] {
        let (mut lexing, mut chain, mut wall) = (f64::MAX, f64::MAX, std::time::Duration::MAX);
        // Each step's wall, the best round's: tier 1 (the upload, its launches and the wait), tier 2's host part (its
        // rooms laid, the writer's region handed over), tier 2 and 16z (their launches and the wait), the tapes read.
        let mut steps = [std::time::Duration::MAX; 4];
        let mut first = std::time::Duration::ZERO;
        let made = kept.made;
        // The writer kept too, its tape given back each round (`recycle`).
        let mut m = rerail_tape_format::f16::Writer::empty();
        for round in 0..5 {
            m.begin(&lang, &f, rerail_tape_format::newlines::Terminators::Newlines, 0, false, true, f.len() / 2 + 16);
            let (g0, t) = (crate::device::gpu_seconds_awaited(), std::time::Instant::now());
            let lex = Lex::go(&mut q, &mut kept, p1, p, &buf, n, LEX_BLOCKS);
            let (gl, tl) = (crate::device::gpu_seconds_awaited(), t.elapsed());
            let ph = Ph::lay(&mut kept, &lex, size, &mut m);
            let tp = t.elapsed();
            let mut each = [0f64; Ph::PHASES as usize];
            for phase in 0..Ph::PHASES {
                let g = crate::device::gpu_seconds_awaited();
                ph.launch(&mut q, p, &lex, phase);
                q.barrier();
                if timed {
                    q.flush().await_().unwrap();
                    each[phase as usize] = crate::device::gpu_seconds_awaited() - g;
                }
            }
            q.flush().await_().unwrap();
            let (g1, tr) = (crate::device::gpu_seconds_awaited(), t.elapsed());
            if timed {
                let groups: Vec<String> = PHASE_GROUPS.iter().map(|&(name, a, b)| format!("{name} {:.2}", each[a..b].iter().sum::<f64>() * 1e3)).collect();
                eprintln!("  round {round}, each phase's GPU ms (a command buffer a phase): {}", groups.join(", "));
            }
            assert_eq!(ph.verdict(), (0, 1), "chunks of {size}: the failure and the finish");
            let zb = ph.z();
            let done = ph.tape(&mut m);
            let tz = t.elapsed();
            if round == 0 {
                first = tz;
            }
            wall = wall.min(tz);
            for (k, d) in [tl, tp - tl, tr - tp, tz - tr].into_iter().enumerate() {
                steps[k] = steps[k].min(d);
            }
            assert!(zb == want_z, "chunks of {size}: the 16z tape");
            assert!(done.tape.as_bytes() == &want[..], "chunks of {size}: the tape");
            m.recycle(done.tape);
            (lexing, chain) = (lexing.min(gl - g0), chain.min(g1 - gl));
        }
        eprintln!("{}, {} groups, chunks of {size} records: tier 1 and the token pass {:.2} ms GPU (4 launches), tier 2 to the finished tape and 16z {:.2} ms GPU ({} launches), all {:.2} ms", crate::device::backend(), groups(), lexing * 1e3, chain * 1e3, Ph::PHASES, (lexing + chain) * 1e3);
        eprintln!("  wall: the first round {first:?} (buffers made: {}), the best {wall:?}: tier 1 {:?}, tier 2's host part {:?}, tier 2 and 16z {:?}, the tapes read {:?}; buffers held {} MB", kept.made - made, steps[0], steps[1], steps[2], steps[3], kept.held() >> 20);
    }
}

/// The program's phases (`<ukl/t2_phases.h>`'s numbers) by the page's names: each name's first and past-last phase.
const PHASE_GROUPS: [(&str, usize, usize); 14] = [("LAY", 0, 1), ("P1", 1, 3), ("P2", 3, 15), ("P3", 15, 17), ("P4", 17, 19), ("P5", 19, 21), ("P7", 21, 33), ("P8", 33, 37), ("MAKE", 37, 38), ("P6", 38, 40), ("END", 40, 41), ("FIN", 41, 42), ("TAIL", 42, 44), ("16z", 44, 62)];

/// A JSON file of at least `bytes` bytes: a list of objects (numbers, strings, lists, a nest every 50), its numbers
/// from `seed`.
pub(crate) fn json_of(bytes: usize, seed: usize) -> Vec<u8> {
    let mut f = b"[".to_vec();
    let mut i = 0usize;
    while f.len() < bytes {
        let k = i + seed;
        let nest = if i % 50 == 0 { format!(", \"deep\": [[{{\"x\": [1, [2, {{\"y\": [{k}]}}]]}}]]") } else { String::new() };
        f.extend_from_slice(format!("{}{{\"id\": {k}, \"name\": \"item number {k}\", \"tags\": [\"a\", \"b{}\"], \"v\": {}.25, \"ok\": {}{nest}}}\n", if i == 0 { "" } else { "," }, k % 7, k * 3, k % 2 == 0).as_bytes());
        i += 1;
    }
    f.push(b']');
    f
}

/// Batches of files on the device at once (`batch`), from the bytes to 16z, against Bitlex's CPU path to 16z on
/// every core (a file a thread at a time, each thread its writer kept): 1 to 16 files of 2 MB, 32 to 256 of 64 KB.
/// Each the best of 3 rounds: wall, the device's GPU time, and MB/s. Run by name: `--ignored json_batch_timings`.
#[test]
#[ignore]
fn json_batch_timings() {
    use rerail_tape_format::f16::Writer;
    let Some(shared) = crate::shared::get() else { return };
    let mut q = crate::device::ComputeQueue::new(&shared.queue);
    let p = program("json_tier2.uglykl", rerail_json_aot::TIER2_UKL).unwrap();
    let p1 = program("json_tier1.uglykl", rerail_json_aot::TIER1_UKL).unwrap();
    let lang = json_lang();
    let cores = std::thread::available_parallelism().map_or(8, |n| n.get());
    eprintln!("{}, {} groups; the CPU's {cores} threads", crate::device::backend(), groups());
    // Each kernel's most threads a group (Metal's: below 1,024, its registers bound how many a core holds).
    for (prog, name) in [(p1, "json_t1_lex"), (p, "json_t2_tokens"), (p, "json_t2_phases"), (p, "json_t2_zphases")] {
        let f = prog.function(name).unwrap();
        eprintln!("  {name}: at most {:?} threads a group, {} bytes of threadgroup memory", f.max_threads_per_group(), f.threadgroup_bytes());
    }
    for (bytes, counts) in [(2usize << 20, &[1usize, 2, 4, 8, 16][..]), (64 << 10, &[32, 128, 256][..])] {
        let most = *counts.last().unwrap();
        let fs: Vec<Vec<u8>> = (0..most).map(|k| json_of(bytes, 1000 * k)).collect();
        let inputs: Vec<(Vec<u8>, usize)> = fs.iter().map(|f| (rerail_json_aot::padded(f), f.len())).collect();
        let want_z: Vec<Vec<u8>> = fs.iter().map(|f| phases_z(&lang, f).expect("the 16z tape by phases")).collect();
        let mut kept: Vec<Kept> = (0..most).map(|_| Kept::default()).collect();
        let mut ms: Vec<Writer> = (0..most).map(|_| Writer::empty()).collect();
        let mut cpu_writers: Vec<Writer> = (0..cores).map(|_| Writer::empty()).collect();
        // `RERAIL_BATCH=COUNT,KB`: that batch alone (a trace's).
        let only: Option<(usize, usize)> = std::env::var("RERAIL_BATCH").ok().and_then(|v| v.split_once(',').map(|(c, k)| (c.parse().unwrap(), k.parse().unwrap())));
        for &count in counts {
            if only.is_some_and(|(c, k)| c != count || k != bytes >> 10) {
                continue;
            }
            let total: usize = fs[..count].iter().map(Vec::len).sum();
            let (mut wall, mut gpu) = (std::time::Duration::MAX, f64::MAX);
            for _ in 0..3 {
                for (m, f) in ms.iter_mut().zip(&fs[..count]) {
                    m.begin(&lang, f, rerail_tape_format::newlines::Terminators::Newlines, 0, false, true, f.len() / 2 + 16);
                }
                let (g0, t) = (crate::device::gpu_seconds_awaited(), std::time::Instant::now());
                let out = batch(&mut q, &mut kept[..count], p1, p, &inputs[..count], 16, &mut ms[..count]);
                (wall, gpu) = (wall.min(t.elapsed()), gpu.min(crate::device::gpu_seconds_awaited() - g0));
                for (i, (_, z)) in out.iter().enumerate() {
                    assert!(z == &want_z[i], "{count} files of {bytes}: file {i}'s 16z");
                }
            }
            // The CPU: every core, a file a thread at a time, each thread its writer kept from round to round (the
            // first round grows them, and is not counted).
            let mut cpu = std::time::Duration::MAX;
            for round in 0..4 {
                let next = std::sync::atomic::AtomicUsize::new(0);
                let t = std::time::Instant::now();
                std::thread::scope(|sc| {
                    for w in cpu_writers.iter_mut().take(cores.min(count)) {
                        let (next, fs, inputs, lang) = (&next, &fs, &inputs, &lang);
                        sc.spawn(move || {
                            loop {
                                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                if i >= count {
                                    break;
                                }
                                let f = &fs[i];
                                w.begin(lang, f, rerail_tape_format::newlines::Terminators::Newlines, 0, true, true, f.len() / 2 + 16);
                                rerail_json_aot::parse(&inputs[i].0, f.len(), w).unwrap();
                                let done = w.finish().unwrap();
                                w.recycle(std::hint::black_box(done).tape);
                            }
                        });
                    }
                });
                if round > 0 {
                    cpu = cpu.min(t.elapsed());
                }
            }
            let mbs = |d: std::time::Duration| total as f64 / d.as_secs_f64() / 1e6;
            let held: usize = kept[..count].iter().map(Kept::held).sum();
            eprintln!("  {count:>4} files of {:>4} KB: the device {wall:>10.2?} wall ({:>7.0} MB/s), {:>6.2} ms GPU ({:>7.0} MB/s); the CPU's {} threads {cpu:>10.2?} ({:>6.0} MB/s); device buffers held {} MB", bytes >> 10, mbs(wall), gpu * 1e3, total as f64 / gpu / 1e6, cores.min(count), mbs(cpu), held >> 20);
        }
    }
}

/// What finding better LZ4 matches would give 16z (the question of a suffix array or the matrix units finding them on
/// the device): the 2 MB file's tape stored as 16z with the device's greedy parse (`lz4_greedy`), LZ4's fast parse,
/// and LZ4's HC parse at levels 9 and 12 (12 the optimal parse: the most any match finder could give), each piece
/// stored as the encoder gives it where that saves a tenth. Its bytes and the CPU's time. Run by name: `--ignored
/// json_16z_match_finders`.
#[test]
#[ignore]
fn json_16z_match_finders() {
    let lang = json_lang();
    let f = json_of(2 << 20, 0);
    let mut w = json_writer(&lang, &f);
    rerail_json_aot::parse(&rerail_json_aot::padded(&f), f.len(), &mut w).unwrap();
    let t = w.finish().unwrap().tape.as_bytes().to_vec();
    let lz4 = |level: i32| {
        move |s: &[u8]| -> Vec<u8> {
            let cap = unsafe { lz4_sys::LZ4_compressBound(s.len() as i32) } as usize;
            let mut out = vec![0u8; cap];
            // SAFETY: `out` holds the bound's bytes.
            let n = unsafe {
                if level == 0 {
                    lz4_sys::LZ4_compress_default(s.as_ptr().cast(), out.as_mut_ptr().cast(), s.len() as i32, cap as i32)
                } else {
                    lz4_sys::LZ4_compress_HC(s.as_ptr().cast(), out.as_mut_ptr().cast(), s.len() as i32, cap as i32, level)
                }
            };
            out.truncate(n as usize);
            out
        }
    };
    eprintln!("a tape of {} bytes ({} bytes of JSON) as 16z:", t.len(), f.len());
    let encoders: [(&str, Box<dyn Fn(&[u8]) -> Vec<u8>>); 4] = [("the device's greedy parse", Box::new(rerail_tape_format::f16::lz4_greedy)), ("LZ4's fast parse", Box::new(lz4(0))), ("LZ4 HC, level 9", Box::new(lz4(9))), ("LZ4 HC, level 12 (optimal)", Box::new(lz4(12)))];
    for (name, e) in &encoders {
        let t0 = std::time::Instant::now();
        let z = rerail_tape_format::f16::z_of(&t, e.as_ref()).unwrap();
        eprintln!("  {name:<28} {:>8} bytes ({:.1}% of the tape), {:?}", z.len(), 100.0 * z.len() as f64 / t.len() as f64, t0.elapsed());
    }
    // For reference: the whole tape as one LZ4 stream (matches across the pieces; no piece read on its own).
    for (name, level) in [("one stream, LZ4's fast parse", 0), ("one stream, LZ4 HC level 12", 12)] {
        let z = lz4(level)(&t);
        eprintln!("  {name:<28} {:>8} bytes ({:.1}% of the tape)", z.len(), 100.0 * z.len() as f64 / t.len() as f64);
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
    let text = rerail_json_aot::TIER2_UKL.replace("    r.phase = args.phase;\n", &format!("    r.phase = args.phase == {salt}u ? 0u : args.phase;\n"));
    assert_ne!(text, rerail_json_aot::TIER2_UKL);
    let t = std::time::Instant::now();
    let p = program("json_tier2_salted.uglykl", Box::leak(text.into_boxed_str())).unwrap();
    eprintln!("{}: the library {:?}", crate::device::backend(), t.elapsed());
    let mut total = std::time::Duration::ZERO;
    for name in ["json_t2_tokens", "json_t2_phases", "json_t2_zphases"] {
        let t = std::time::Instant::now();
        p.function(name).unwrap();
        total += t.elapsed();
        eprintln!("{}: {name} {:?}", crate::device::backend(), t.elapsed());
    }
    eprintln!("{}: every kernel {total:?}", crate::device::backend());
}

/// JSON's language for format 16 writers.
pub(crate) fn json_lang() -> rerail_tape_format::f16::Lang {
    let entries = rerail_json_aot::ENTRIES.iter().map(|&(kind, field, classes)| rerail_tape_format::tape::Entry { kind, field, flags: 0, alias: None, classes: classes.to_vec() }).collect();
    rerail_tape_format::f16::Lang::new(rerail_tape_format::tape::CodeTable::new(entries), rerail_json_aot::WIDTHS, 1).expect("a language")
}

/// A format 16 writer of `f`'s tape, its blocks' presence kept.
pub(crate) fn json_writer(lang: &rerail_tape_format::f16::Lang, f: &[u8]) -> rerail_tape_format::f16::Writer {
    rerail_tape_format::f16::Writer::new(lang, f, rerail_tape_format::newlines::Terminators::Newlines, 0, false, true, f.len() / 2 + 16)
}

/// Files' chains from the bytes on the device at once (`files`, each padded, and its length): every file's phase
/// launched, then a barrier, so the files' items fill the device together; each file in its own kept buffers (`kept`,
/// a slot a file) and its own writer (begun). Tier 1 and the token pass of every file, then a wait (tier 2's rooms are
/// sized by their records), then tier 2 as phases and 16z of every file. A file past tier 1's room runs again alone
/// with what it needs. Each file's run and 16z bytes.
#[allow(clippy::too_many_arguments)]
fn batch(q: &mut crate::device::ComputeQueue, kept: &mut [Kept], p1: &crate::kernels::Program, p: &crate::kernels::Program, files: &[(Vec<u8>, usize)], size: usize, ms: &mut [rerail_tape_format::f16::Writer]) -> Vec<(Ph, Vec<u8>)> {
    q.set_concurrent(true);
    let mut lexes: Vec<Lex> = files
        .iter()
        .zip(kept.iter_mut())
        .map(|((buf, n), k)| {
            Lex::upload(k, buf);
            let records = k.records_room(*n);
            Lex::lay(k, *n, LEX_BLOCKS, records)
        })
        .collect();
    for phase in 0..LEX_PHASES {
        for l in &lexes {
            l.launch(q, p1, p, phase);
        }
        q.barrier();
    }
    q.flush().await_().unwrap();
    for (i, k) in kept.iter_mut().enumerate() {
        if lexes[i].done(k).is_some() {
            lexes[i] = Lex::go(q, k, p1, p, &files[i].0, files[i].1, LEX_BLOCKS);
        }
    }
    let phs: Vec<Ph> = lexes.iter().zip(kept.iter_mut()).zip(ms.iter_mut()).map(|((l, k), m)| Ph::lay(k, l, size, m)).collect();
    for phase in 0..Ph::PHASES {
        for (ph, l) in phs.iter().zip(&lexes) {
            ph.launch(q, p, l, phase);
        }
        q.barrier();
    }
    q.flush().await_().unwrap();
    q.set_concurrent(false);
    phs.into_iter()
        .enumerate()
        .map(|(i, ph)| {
            assert_eq!(ph.verdict(), (0, 1), "{}: file {i}: the failure and the finish", crate::device::backend());
            let z = ph.z();
            (ph, z)
        })
        .collect()
}

/// A batch of files (`batch`) on the device at once gives each file its own tape and 16z, the serial walk's and the
/// CPU phases' (with the device's encoder) byte for byte: every test file at once, then again (the kept buffers holding
/// the last batch's bytes).
#[test]
fn json_batch_gives_each_files_tape() {
    let Some(shared) = crate::shared::get() else {
        eprintln!("no device: the batch did not run");
        return;
    };
    let mut q = crate::device::ComputeQueue::new(&shared.queue);
    let p = program("json_tier2.uglykl", rerail_json_aot::TIER2_UKL).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()));
    let p1 = program("json_tier1.uglykl", rerail_json_aot::TIER1_UKL).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()));
    let lang = json_lang();
    let mut all: Vec<Vec<u8>> = files();
    all.extend(nested());
    // The files whose chunks the phases take (none refused at its place), each its serial tape and its 16z.
    let (mut wants, mut inputs, mut kept_files) = (Vec::new(), Vec::new(), Vec::new());
    for f in &all {
        let buf = rerail_json_aot::padded(f);
        let mut w = json_writer(&lang, f);
        if rerail_json_aot::parse(&buf, f.len(), &mut w).is_err() {
            continue;
        }
        let lx = rerail_json_aot::lex(&buf, f.len());
        let typed = rerail_json_aot::lex_typed(&buf, f.len(), LEX_BLOCKS);
        let toks = rerail_json_aot::middle::tokens(&buf, &typed.lexed, &typed.types, &rerail_json_aot::middle::in_order);
        if rerail_json_aot::middle::Phases::run(&buf, &lx, &toks, 16, &rerail_json_aot::middle::in_order).refused.is_some() {
            continue;
        }
        let want = w.finish().expect("the serial tape").tape.as_bytes().to_vec();
        wants.push((want, phases_z(&lang, f).expect("the 16z tape by phases")));
        inputs.push((buf, f.len()));
        kept_files.push(f.clone());
    }
    let mut kept: Vec<Kept> = (0..inputs.len()).map(|_| Kept::default()).collect();
    for round in 0..2 {
        let mut ms: Vec<_> = kept_files.iter().map(|f| json_writer(&lang, f)).collect();
        let out = batch(&mut q, &mut kept, p1, p, &inputs, 16, &mut ms);
        for (i, ((ph, z), m)) in out.iter().zip(ms.iter_mut()).enumerate() {
            assert!(z == &wants[i].1, "{}: round {round}, file {i}: the 16z tape", crate::device::backend());
            assert!(ph.tape(m).tape.as_bytes() == &wants[i].0[..], "{}: round {round}, file {i}: the tape", crate::device::backend());
        }
    }
    eprintln!("{}: {} files at once, twice: each file's tape and 16z its own", crate::device::backend(), inputs.len());
}

/// The programs' threadgroups: `RERAIL_T2_GROUPS` (the timings' knob), else 512 (on the M3 Max, 2 MB of JSON: tier 2's
/// phases 11.6 ms of GPU at 64, 7.4 at 512, 7.9 at 2,048).
fn groups() -> u64 {
    std::env::var("RERAIL_T2_GROUPS").ok().and_then(|v| v.parse().ok()).unwrap_or(512)
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
    /// The trees' arrivals (`ukl_lex_meet`).
    meet: crate::device::Buffer,
    n: usize,
    chunks: usize,
    /// The records' room.
    records: usize,
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
    /// The run's epoch (`ukl_ph_meet`, `ukl_lex_meet`): new each run, never 0 (a new arrivals buffer's words) nor 0xa5a5a5a5.
    epoch: u32,
    /// What files so far took: their bytes and records; the room for a file is twice what they say on average, and a
    /// bit more.
    seen: [f64; 2],
    /// The runs that bailed past a room, and the room each said it needs (records).
    bails: Vec<(&'static str, usize)>,
}

impl Default for Kept {
    fn default() -> Kept {
        // Before any file, JSON's average: 0.3 records a byte.
        Kept { bufs: Default::default(), made: 0, bytes: 0, epoch: 0, seen: [1e6, 3e5], bails: Vec::new() }
    }
}

impl Kept {
    /// Kept buffers whose averages say far too little (a record in 10,000 bytes), so tier 1 bails past its room and
    /// runs again with what it says it needs.
    fn tight() -> Kept {
        let mut k = Kept::default();
        k.seen = [1e6, 100.0];
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
    /// them, since tier 2's room is sized by the records they find. Past its records' room it runs again with what
    /// it says it needs.
    fn go(q: &mut crate::device::ComputeQueue, kept: &mut Kept, p1: &crate::kernels::Program, p2: &crate::kernels::Program, buf: &[u8], n: usize, blocks: usize) -> Lex {
        Lex::upload(kept, buf);
        let mut records = kept.records_room(n);
        loop {
            let lex = Lex::lay(kept, n, blocks, records);
            for phase in 0..LEX_PHASES {
                lex.launch(q, p1, p2, phase);
                q.barrier();
            }
            q.flush().await_().unwrap();
            match lex.done(kept) {
                Some(need) => records = need,
                None => return lex,
            }
        }
    }

    /// The source, uploaded into `kept` (the one thing the host sends).
    fn upload(kept: &mut Kept, buf: &[u8]) {
        let sb = kept.get("src", buf.len());
        // SAFETY: the buffer holds `buf`'s bytes.
        unsafe { std::ptr::copy_nonoverlapping(buf.as_ptr(), sb.contents().cast::<u8>(), buf.len()) };
    }

    /// A run of tier 1 and the token pass laid out in `kept`, with room for `records` records: its pool's layout and
    /// header (the source uploaded first).
    fn lay(kept: &mut Kept, n: usize, blocks: usize, records: usize) -> Lex {
        let chunks = n.div_ceil(64).div_ceil(blocks).max(1);
        let tn = 2 * chunks + 64;
        let (om, ok) = (32, 32 + tn);
        // K's values: each chunk's records, each type's and its newlines.
        let oe = ok + (rerail_json_aot::TYPES + 2) * tn;
        let words = oe + chunks + 64;
        let pb = kept.get("lex pool", 4 * words);
        let epoch = kept.next_epoch();
        {
            // The layout, and the header's words the phases count into.
            let pool = words_of(&pb, 32);
            pool[..7].copy_from_slice(&[chunks as u32, n as u32, blocks as u32, tn as u32, om as u32, ok as u32, oe as u32]);
            pool[7..32].fill(0);
            (pool[21], pool[22]) = (epoch, records as u32);
        }
        let meet = kept.meet("lex meet", 2 * tn);
        let src = kept.bufs["src"].clone();
        let (starts, types, tokens) = (kept.get("starts", 4 * (records + 3)), kept.get("types", 4 * records), kept.get("tokens", 20 * records));
        Lex { pool: pb, src, starts, types, tokens, meet, n, chunks, records }
    }

    /// Its phase `phase`: tier 1's three (`json_t1_lex`), then the token pass, a thread a typed record (at most the
    /// room's), 64 a group.
    fn launch(&self, q: &mut crate::device::ComputeQueue, p1: &crate::kernels::Program, p2: &crate::kernels::Program, phase: u32) {
        if phase < 3 {
            launch(q, p1.function("json_t1_lex").unwrap())
                .value("args", &T1LArgs { n: self.n as u32, phase })
                .buffer("src", &self.src)
                .buffer("pool", &self.pool)
                .buffer("starts", &self.starts)
                .buffer("types", &self.types)
                .buffer("tokens", &self.tokens)
                .buffer("meet", &self.meet)
                .run(&[groups()], &[64])
                .unwrap();
        } else {
            let args = T2Args { n: self.records.next_multiple_of(64) as u32, count: self.records as u32 };
            launch(q, p2.function("json_t2_tokens").unwrap())
                .value("args", &args)
                .buffer("src", &self.src)
                .buffer("starts", &self.starts)
                .buffer("list", &self.types)
                .buffer("lexed", &self.pool)
                .buffer("tokens", &self.tokens)
                .run(&[self.records.div_ceil(64) as u64], &[64])
                .unwrap();
        }
    }

    /// After its run: `None` where its records fit (the averages updated), else the room it says it needs.
    fn done(&self, kept: &mut Kept) -> Option<usize> {
        let need = read::<u32>(&self.pool, 32)[23] as usize;
        if need != 0 {
            kept.bails.push(("tier 1's records", need));
            return Some(need);
        }
        kept.seen[0] += self.n as f64;
        kept.seen[1] += self.said().1 as f64;
        None
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

/// Tier 1's three phases and the token pass.
const LEX_PHASES: u32 = 4;

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

#[repr(C)]
#[derive(Clone, Copy)]
struct T2PArgs {
    phase: u32,
    pad: u32,
}
unsafe impl Pod for T2PArgs {}

/// The runner's header and the program's (`<ukl/phases_run.h>`, `<ukl/t2_phases.h>`).
const PH_H: usize = 200;
const T2_H: usize = PH_H + 80;


/// `file`'s 16z by the CPU's phases (`parse_phases`, then `finish_by_phases_with`) with the device's encoder
/// (`lz4_greedy`), where it parses: what the device's 16z by phases is.
fn phases_z(lang: &rerail_tape_format::f16::Lang, file: &[u8]) -> Option<Vec<u8>> {
    use rerail_json_aot::middle;
    let buf = rerail_json_aot::padded(file);
    let order: middle::Order<'_> = &middle::in_order;
    let mut zw = rerail_tape_format::f16::Writer::empty();
    let doc = rerail_tape_format::f16::Document { lang, source: file, tag: 0, z: true, blocks: true, nodes: file.len() / 2 + 16 };
    let counted = rerail_json_aot::lex_count(&buf, file.len(), 64, order);
    let mut typed = None;
    zw.begin_with(&doc, |out| typed = Some(counted.write(Some(out), order)));
    let typed = typed.expect("lexed");
    middle::parse_phases(&buf, &typed.lexed, &typed.types, &mut zw, 64, order).ok()?;
    Some(zw.finish_by_phases_with(&rerail_tape_format::phases::in_order, Some(&rerail_tape_format::f16::lz4_greedy)).expect("the 16z tape by phases").tape.as_bytes().to_vec())
}

/// Tier 2 as phases on the device for one file (`<ukl/t2_phases.h>`: `json_t2_phases`, and `json_t2_zphases` for
/// 16z's phases of group items), from tier 1's records and the token pass's answers: its rooms from the records'
/// bound, the writer's region handed over, the tape finished there and stored as 16z.
struct Ph {
    pb: crate::device::Buffer,
    ab: crate::device::Buffer,
    meet: crate::device::Buffer,
    rb: crate::device::Buffer,
    fb: crate::device::Buffer,
    tb: crate::device::Buffer,
    zb: crate::device::Buffer,
    room: usize,
    aroom: usize,
    region: usize,
    zroom: usize,
}

impl Ph {
    /// The program's phases, and those of group items (16z's ZP1 and ZP4).
    const PHASES: u32 = 62;
    const GROUP: [u32; 2] = [44, 61];
    /// The sorts' stages (P2, P7, 16z's ZS: the library's sort of rows, `json_t2_sort`).
    const SORTS: [std::ops::Range<u32>; 3] = [3..15, 21..33, 45..57];

    /// The rooms of `lex`'s file in chunks of `size` records, from its records' bound; the writer `mw` (begun) hands
    /// its region over; the pool's header set.
    fn lay(kept: &mut Kept, lex: &Lex, size: usize, mw: &mut rerail_tape_format::f16::Writer) -> Ph {
        let count = lex.said().1;
        let chunks = (lex.records + 1).div_ceil(size);
        // The level sort's chunks at their bound (two items a record), the events' (two a record), the blocks'.
        let ichunks = (2 * (lex.records + 1)).div_ceil(size);
        let tree = |leaves: usize, w: usize| w * (2 * leaves + 40 + 2 * leaves);
        let most_ev = 2 * (lex.records + 1) + 2;
        let echunks = (2 * most_ev).div_ceil(size);
        let most = 1 + count + lex.token_nodes();
        let most_blocks = most.div_ceil(1024).max(1);
        let zc = (4 * most_blocks).div_ceil(size);
        let ms = [ichunks, echunks, most_blocks, zc].into_iter().map(|n| 2 * n + 40).max().unwrap();
        let room = T2_H + tree(chunks, 3) + tree(ichunks, 2) + tree(chunks, 32) + tree(chunks, 10);
        let room = room + tree(echunks, 2) + tree(echunks, 3) + tree(most_blocks, 5);
        let room = room + tree(zc, 2) + tree(zc, 1);
        // The arrays as LAY lays them, and the sorts' (`<ukl/stages.h>`'s sort of rows) for the most rows of the three.
        let ns = (2 * (lex.records + 1)).max(2 * most_ev).max(4 * most_blocks);
        let aroom = 9 * (lex.records + 1) + 11 * chunks + 22 * most_ev + 36 * most_blocks + 6 * ns + 256 * ns.div_ceil(1024) + 1 + 1025 + 1;
        // The writer's region, with room for the nodes the records and their tokens can make.
        mw.reserve_run(most, (1024 + 1) * 3 + 1);
        mw.final_as_written();
        mw.reserve_device_finish(most);
        let region = mw.device_region_len();
        let tb = kept.get("phases region", region);
        // SAFETY: the buffer holds `region` bytes; no launch runs while the host writes them.
        mw.device_region_into(unsafe { std::slice::from_raw_parts_mut(tb.contents().cast::<u8>(), region) });
        let zroom = region + 9 * 4 * most_blocks + 256;
        let ph = Ph {
            pb: kept.get("phases pool", 4 * room),
            ab: kept.get("phases arrays", 4 * aroom),
            meet: kept.meet("phases meet", 24 * ms),
            rb: kept.get("phases rets", 1024 * chunks),
            fb: kept.get("phases frames", 4 * 25 * (lex.records + 1 + chunks)),
            tb,
            zb: kept.get("phases z", zroom),
            room,
            aroom,
            region,
            zroom,
        };
        let epoch = kept.next_epoch();
        let h = words_of(&ph.pb, T2_H);
        h.fill(0);
        (h[0], h[1]) = (epoch, ms as u32);
        (h[PH_H], h[PH_H + 1], h[PH_H + 8], h[PH_H + 28], h[PH_H + 50]) = (size as u32, room as u32, aroom as u32, most_blocks as u32, zroom as u32);
        ph
    }

    /// Phase `phase`'s launch.
    fn launch(&self, q: &mut crate::device::ComputeQueue, p: &crate::kernels::Program, lex: &Lex, phase: u32) {
        let args = T2PArgs { phase, pad: 0 };
        if Ph::SORTS.iter().any(|r| r.contains(&phase)) {
            // The sort's tiles: at most the program's 1,024 (`UKL_T2_SORT_TILES`), of 256 threads.
            launch(q, p.function("json_t2_sort").unwrap())
                .value("args", &args)
                .buffer("lexed", &lex.pool)
                .buffer("pool", &self.pb)
                .buffer("arrays", &self.ab)
                .run(&[(groups() as u64).min(1024)], &[256])
                .unwrap();
        } else if Ph::GROUP.contains(&phase) {
            launch(q, p.function("json_t2_zphases").unwrap())
                .value("args", &args)
                .buffer("src", &lex.src)
                .buffer("starts", &lex.starts)
                .buffer("lexed", &lex.pool)
                .buffer("pool", &self.pb)
                .buffer("meet", &self.meet)
                .buffer("arrays", &self.ab)
                .buffer("t", &self.tb)
                .buffer("z", &self.zb)
                .run(&[groups()], &[crate::device::simd_width() as u64])
                .unwrap();
        } else {
            launch(q, p.function("json_t2_phases").unwrap())
                .value("args", &args)
                .buffer("src", &lex.src)
                .buffer("starts", &lex.starts)
                .buffer("lexed", &lex.pool)
                .buffer("pool", &self.pb)
                .buffer("meet", &self.meet)
                .buffer("arrays", &self.ab)
                .buffer("toks", &lex.tokens)
                .buffer("rets", &self.rb)
                .buffer("t", &self.tb)
                .buffer("f2", &self.fb)
                .run(&[groups()], &[64])
                .unwrap();
        }
    }

    /// Laid, every phase launched, a barrier between each, and waited for.
    fn go(q: &mut crate::device::ComputeQueue, kept: &mut Kept, p: &crate::kernels::Program, lex: &Lex, size: usize, mw: &mut rerail_tape_format::f16::Writer) -> Ph {
        let ph = Ph::lay(kept, lex, size, mw);
        for phase in 0..Ph::PHASES {
            ph.launch(q, p, lex, phase);
            q.barrier();
        }
        q.flush().await_().unwrap();
        ph
    }

    /// The pool's header: the failure (0 none) and whether the tape finished.
    fn verdict(&self) -> (u32, u32) {
        let m: Vec<u32> = read(&self.pb, T2_H);
        (m[2], m[PH_H + 66])
    }

    /// The finished tape, taken by the writer that handed its region over.
    fn tape(&self, mw: &mut rerail_tape_format::f16::Writer) -> rerail_tape_format::f16::Finished {
        mw.from_device_region(&read::<u8>(&self.tb, self.region));
        mw.finished_on_device().expect("the tape finished on the device")
    }

    /// The 16z tape.
    fn z(&self) -> Vec<u8> {
        let end = read::<u32>(&self.pb, T2_H)[PH_H + 62] as usize;
        assert!(end > 0 && end <= self.zroom, "{}: 16z's end {end} in its room of {}", crate::device::backend(), self.zroom);
        read::<u8>(&self.zb, end)
    }
}

/// Tier 2 as phases on the device (`json_t2_phases`, `<ukl/t2_phases.h>`), P1 from tier 1's records on the device:
/// the layout laid from the records the device found, each chunk's depth summary a leaf of P1's arrival tree, each
/// chunk's records' depths and items written from its sum walked down, the items sorted by level (P2, two digits of
/// keyed counts), each record's container and each head's place (P3), each chunk's place from its map of places
/// and the chunk refused at its place (P4), each chunk's shape before it, the events and their items (P5), the
/// events' items in frame order (P7), each one's frame and its frame's children so far (P8). Each
/// record's depth is the CPU phases' (`Phases::run`), and the items in level order are the CPU's, for the files and the nested
/// trees, in chunks of 1, 7 and 64 records.
#[test]
fn json_tier2_phases_are_the_cpus() {
    use rerail_json_aot::middle;
    let Some(shared) = crate::shared::get() else {
        eprintln!("no device: tier 2 as phases did not run");
        return;
    };
    let mut q = crate::device::ComputeQueue::new(&shared.queue);
    // Its averages far too small: tier 1 runs past its room, and again with what it says it needs.
    let mut kept = Kept::tight();
    let p1 = program("json_tier1.uglykl", rerail_json_aot::TIER1_UKL).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()));
    let p2 = program("json_tier2.uglykl", rerail_json_aot::TIER2_UKL).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()));
    let (mut runs, mut records, mut refusals, mut events, mut tapes, mut bytes, mut zbytes, mut repeats) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    let lang = json_lang();
    // And an array of one value 30,000 times: its blocks' pieces repeat (16z's repeats).
    let alike = {
        let mut v = b"[".to_vec();
        v.extend(std::iter::repeat_n(&b"7,"[..], 30_000).flatten());
        v.extend_from_slice(b"7]");
        v
    };
    for (k, file) in files().into_iter().chain(nested()).chain([alike]).enumerate() {
        let buf = rerail_json_aot::padded(&file);
        let n = file.len();
        let lex = Lex::go(&mut q, &mut kept, p1, p2, &buf, n, LEX_BLOCKS);
        let (refused, count, _) = lex.said();
        if refused {
            continue;
        }
        let typed = rerail_json_aot::lex_typed(&buf, n, LEX_BLOCKS);
        let toks = middle::tokens(&buf, &typed.lexed, &typed.types, &middle::in_order);
        // The serial walk's finished tape, where it parses.
        let lx = rerail_json_aot::lex(&buf, n);
        let mut sw = json_writer(&lang, &file);
        let want = rerail_json_aot::parse_lexed(&buf, &lx, &mut sw).ok().map(|()| sw.finish().expect("the serial tape").tape.as_bytes().to_vec());
        // And its 16z by the CPU's phases with the device's encoder.
        let want_z = phases_z(&lang, &file);
        for size in [1usize, 7, 64] {
            let at = |what: &str| format!("{}: file {k}, chunks of {size}: {what}", crate::device::backend());
            let mut mw = json_writer(&lang, &file);
            let dev = Ph::go(&mut q, &mut kept, p2, &lex, size, &mut mw);
            let (room, aroom, region, zroom) = (dev.room, dev.aroom, dev.region, dev.zroom);
            let m: Vec<u32> = read(&dev.pb, room);
            let arr: Vec<u32> = read(&dev.ab, aroom);
            assert_eq!(m[PH_H + 2], 0, "{}", at("the room"));
            assert_eq!(m[PH_H + 3] as usize, count, "{}", at("the records"));
            let ph = middle::Phases::run(&buf, &typed.lexed, &toks, size, &middle::in_order);
            let od = m[PH_H + 5] as usize;
            let depth: Vec<i64> = arr[od..od + count + 1].iter().map(|&d| i64::from(d) - (1i64 << 30)).collect();
            if let Some(r) = (0..depth.len().max(ph.depth.len())).find(|&r| depth.get(r) != ph.depth.get(r)) {
                let lo = r.saturating_sub(3);
                panic!("{}", at(&format!("record {r}'s depth of {}: {:?}, the CPU's {:?}", count + 1, &depth[lo..(r + 3).min(depth.len())], &ph.depth[lo..(r + 3).min(ph.depth.len())])));
            }
            let (oi, ni) = (m[PH_H + 6] as usize, m[PH_H + 7] as usize);
            let items: Vec<middle::phases::Item> = arr[oi..oi + 3 * ni].chunks_exact(3).map(|w| middle::phases::Item { level: w[0], record: w[1], head: w[2] != 0 }).collect();
            assert_eq!(items.len(), ph.items.len(), "{}", at("the items"));
            if let Some(x) = (0..ni).find(|&x| items[x] != ph.items[x]) {
                panic!("{}", at(&format!("item {x} of {ni} by level: {:?}, the CPU's {:?}", items[x], ph.items[x])));
            }
            for (what, o, want) in [("container", m[PH_H + 12] as usize, &ph.container), ("head item", m[PH_H + 13] as usize, &ph.head_at)] {
                if let Some(r) = (0..count + 1).find(|&r| arr[o + r] != want[r]) {
                    panic!("{}", at(&format!("record {r}'s {what}: {}, the CPU's {}", arr[o + r], want[r])));
                }
            }
            // P4: each chunk's place, and the first refused.
            let rc = (count + 1).div_ceil(size);
            let op = m[PH_H + 14] as usize;
            for c in 0..rc {
                assert_eq!(arr[op + c], u32::from(ph.place[c]), "{}", at(&format!("chunk {c}'s place")));
            }
            let refused_at = m[PH_H + 18];
            assert_eq!(refused_at, ph.refused.map_or(u32::MAX, |c| c as u32), "{}", at("the chunk refused"));
            if ph.refused.is_some() {
                refusals += 1;
                runs += 1;
                continue;
            }
            // P5: each chunk's sum, the events and their items (in frame order as P7 orders them).
            let os = m[PH_H + 15] as usize;
            for c in 0..rc {
                let want = middle::phases::shape_words(&ph.shape_at[c]);
                assert_eq!(&arr[os + 10 * c..os + 10 * c + 10], &want[..], "{}", at(&format!("chunk {c}'s shape before it")));
            }
            let (ne, nei) = (m[PH_H + 19] as usize, m[PH_H + 20] as usize);
            assert_eq!(ne, ph.events.len(), "{}", at("the events"));
            let oe = m[PH_H + 16] as usize;
            for (g, w) in ph.events.iter().enumerate() {
                let e = &arr[oe + 8 * g..oe + 8 * g + 8];
                let have = (e[0] as u8, e[1], e[2], e[3], u64::from(e[4]) | (u64::from(e[5]) << 32), u64::from(e[6]) | (u64::from(e[7]) << 32));
                assert_eq!(have, (w.what, w.f, w.node, w.nodes, w.start, w.last), "{}", at(&format!("event {g}")));
            }
            // P7, P8: the events' items in frame order, each one's frame and its frame's children so far.
            let oei = m[PH_H + 17] as usize;
            let eitems: Vec<middle::phases::Item> = arr[oei..oei + 3 * nei].chunks_exact(3).map(|w| middle::phases::Item { level: w[0], record: w[1], head: w[2] != 0 }).collect();
            assert!(eitems == ph.eitems, "{}", at("the events' items in frame order"));
            let (of, ok) = (m[PH_H + 25] as usize, m[PH_H + 26] as usize);
            assert!(arr[of..of + nei] == ph.frame_of[..], "{}", at("each event item's frame"));
            for (x, kd) in ph.kids.iter().enumerate() {
                assert_eq!(&arr[ok + 3 * x..ok + 3 * x + 3], &[u32::from(kd.reset), kd.count, kd.last], "{}", at(&format!("event item {x}'s frame's children")));
            }
            events += ne;
            // P6 and F: the tape finished on the device, the serial walk's byte for byte.
            assert_eq!((m[2], m[PH_H + 66]), (0, 1), "{}", at(&format!("the failure and the finish (the first chunk stopped: {})", m[PH_H + 68])));
            mw.from_device_region(&read::<u8>(&dev.tb, region));
            let have = mw.finished_on_device().expect("the tape finished on the device").tape.as_bytes().to_vec();
            let want = want.as_ref().expect("the serial walk parses what the phases do");
            assert!(have == *want, "{}", at(&format!("the tape differs at byte {:?} of {} ({} bytes)", have.iter().zip(want).position(|(a, b)| a != b), want.len(), have.len())));
            tapes += 1;
            bytes += have.len();
            // Z: 16z by phases on the device, the CPU phases' with the device's encoder.
            let zend = m[PH_H + 62] as usize;
            assert!(zend > 0 && zend <= zroom, "{}", at(&format!("16z's end {zend} in its room of {zroom}")));
            let z = read::<u8>(&dev.zb, zend);
            let want_z = want_z.as_ref().expect("the 16z tape by phases");
            let fields = |t: &[u8]| (0..7).map(|k| u32::from_le_bytes(t[88 + 4 * k..92 + 4 * k].try_into().unwrap()) as usize).collect::<Vec<_>>();
            let kinds = |t: &[u8]| {
                let f = fields(t);
                let ks = &t[f[5]..f[6].min(t.len())];
                (0..3).map(|k| ks.iter().filter(|&&x| x == k).count()).collect::<Vec<_>>()
            };
            assert!(z == *want_z, "{}", at(&format!("the 16z tape differs at byte {:?} of {} ({} bytes): fields {:?} against {:?}, kinds {:?} against {:?}", z.iter().zip(want_z).position(|(a, b)| a != b), want_z.len(), z.len(), fields(&z), fields(want_z), kinds(&z), kinds(want_z))));
            zbytes += z.len();
            repeats += kinds(&z)[2];
            runs += 1;
            records += count;
        }
    }
    assert!(repeats > 0, "{}: no piece repeated", crate::device::backend());
    assert!(kept.bails.iter().any(|b| b.0 == "tier 1's records"), "{}: tier 1 never ran past its room: {:?}", crate::device::backend(), kept.bails);
    eprintln!("{}: tier 2 as phases, P1-P8, P6 and the finish on the device: {runs} runs ({refusals} refused at a chunk's place), {records} records' depths, items in level order, containers, heads and places, {events} events, their items in frame order, frames and children the CPU phases'; {tapes} tapes finished on the device, the serial walk's, {bytes} bytes; stored as 16z by phases, the CPU phases' with the device's encoder, {zbytes} bytes, {repeats} pieces repeats", crate::device::backend());
}
