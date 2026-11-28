//! Stage 1 from the middle (`lex_chunked`): a file lexed in chunks, each from every entry state at once, then joined,
//! is the serial lexer's (`lex`), record for record. Tier 2 from the middle (`middle`, decision 0726): each chunk's
//! state at its start, from its summaries composed, is the serial walk's there.

use crate::{lex, lex_chunked, padded};

/// The files: the cases below, and `RERAIL_JSON_CORPUS`'s `.json` files where it is set.
fn files() -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = [
        &b"{\"a\": [1, 2.5e3, -0, true, null], \"b\": \"x\\\"y\\\\\", \"c\": {\"d\": \"\\u00e9\"}}\n"[..],
        b"[\"a long string with \\\"quotes\\\" and , commas [ ] { } inside it\", 1, 2, 3]",
        b"",
        b"\"",
        b"\"open",
        b"\"\x01\"",
        b"\xef\xbb\xbf{\"bom\": 1}",
        b"[\"\xc3\xa9\", \"\xc3\"]",
    ]
    .iter()
    .map(|s| s.to_vec())
    .collect();
    // Runs of backslashes and quotes across block edges, at every offset.
    for at in 0..70 {
        for k in 1..6 {
            let mut f = vec![b' '; at];
            f.insert(0, b'"');
            f.extend(std::iter::repeat_n(b'\\', k));
            f.extend_from_slice(b"\"\", 1, \"x\"");
            out.push(f);
        }
    }
    // A UTF-8 sequence cut by a block edge, at every offset.
    for at in 55..70 {
        let mut f = b"[\"".to_vec();
        f.extend(std::iter::repeat_n(b'a', at));
        f.extend_from_slice("é€😀\"]".as_bytes());
        out.push(f.clone());
        f.truncate(at + 3);
        f.extend_from_slice(b"\"]");
        out.push(f);
    }
    out.extend(page_cases());
    if let Some(dir) = std::env::var_os("RERAIL_JSON_CORPUS") {
        let mut paths: Vec<_> = std::fs::read_dir(dir).expect("the corpus").filter_map(|e| e.ok().map(|e| e.path())).filter(|p| p.extension().is_some_and(|x| x == "json")).collect();
        paths.sort();
        out.extend(paths.iter().filter_map(|p| std::fs::read(p).ok()));
    }
    out
}

/// The design page's case: a 3 KB array of numbers, and the same bytes as one string.
fn page_cases() -> [Vec<u8>; 2] {
    let mut numbers = String::new();
    let mut i = 0;
    while numbers.len() < 3_000 {
        numbers.push_str(&format!("{i}, "));
        i += 7;
    }
    [format!("[{numbers}0]\n").into_bytes(), format!("[\"{numbers}0\"]\n").into_bytes()]
}

#[test]
fn chunked_is_the_serial_lexer() {
    let files = files();
    for (k, f) in files.iter().enumerate() {
        let buf = padded(f);
        let want = lex(&buf, f.len());
        for blocks in [1, 2, 3, 16] {
            let got = lex_chunked(&buf, f.len(), blocks);
            assert_eq!(got.refused, want.refused, "file {k}, chunks of {blocks} blocks: refused");
            assert_eq!(got.count, want.count, "file {k}, chunks of {blocks} blocks: the count");
            assert!(got.starts == want.starts, "file {k}, chunks of {blocks} blocks: the records");
        }
    }
    eprintln!("{} files, each in chunks of 1, 2, 3 and 16 blocks: the serial lexer's records", files.len());
}

/// A random order of `n` chunks at each ask (Fisher and Yates' shuffle, xorshift's numbers from `seed`): every pass, and
/// each round of every scan, runs its chunks in a new one.
fn shuffled(seed: u64) -> impl Fn(usize) -> Vec<usize> {
    let state = std::cell::Cell::new(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1);
    move |n| {
        let mut v: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            let mut x = state.get();
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            state.set(x);
            v.swap(i, (x % (i as u64 + 1)) as usize);
        }
        v
    }
}

/// Pass 2 run twice (count, then write): the records are the serial lexer's, and each type's are the records split by
/// their first byte (a string's quote, a punctuation byte, else a word), each a record's number in order.
#[test]
fn typed_records_are_the_records_split_by_type() {
    let files = files();
    let ty = |name: &str| crate::TYPE_NAMES.iter().position(|t| *t == name).unwrap_or_else(|| panic!("a type {name}"));
    let (quote, punct, word) = (ty("\""), ty("punctuation"), ty("number"));
    let mut records = 0;
    for (k, f) in files.iter().enumerate() {
        let buf = padded(f);
        let want = lex(&buf, f.len());
        for blocks in [1, 2, 3, 16] {
            // Every pass's chunks and every scan round's in a random order, a seed a file and size.
            let order = shuffled((k * 31 + blocks) as u64);
            let got = crate::lex_typed_in(&buf, f.len(), blocks, &order);
            assert_eq!(got.lexed.refused, want.refused, "file {k}, chunks of {blocks} blocks: refused");
            if want.refused {
                continue;
            }
            assert!(got.lexed.starts == want.starts, "file {k}, chunks of {blocks} blocks: the records");
            let mut split: Vec<Vec<u32>> = vec![Vec::new(); crate::TYPES];
            for r in 0..want.count {
                let t = match buf[want.starts[r] as usize] {
                    b'"' => quote,
                    b'{' | b'}' | b'[' | b']' | b',' | b':' => punct,
                    _ => word,
                };
                split[t].push(r as u32);
            }
            assert!(got.types == split, "file {k}, chunks of {blocks} blocks: the records of each type");
            records += want.count;
        }
    }
    eprintln!("{} files in chunks of 1, 2, 3 and 16 blocks ({records} records): each type's records the records split by type", files.len());
}

/// In the string, a chunk's lanes split by `in_string` never meet: both run to the chunk's end, and the join takes the
/// one the chunk before ends in.
#[test]
fn the_page_cases_join_to_the_serial_lexer() {
    for f in page_cases() {
        let buf = padded(&f);
        let (want, got) = (lex(&buf, f.len()), lex_chunked(&buf, f.len(), 16));
        assert!(got.starts == want.starts && got.refused == want.refused);
    }
}

#[test]
#[ignore = "a report: the serial lexer against the chunked one, GB/s, over RERAIL_JSON_CORPUS (or the cases)"]
fn report_chunked_speed() {
    let files = files();
    let bufs: Vec<(Vec<u8>, usize)> = files.iter().map(|f| (padded(f), f.len())).collect();
    let bytes: usize = bufs.iter().map(|b| b.1).sum();
    let time = |f: &dyn Fn(&[u8], usize) -> usize| {
        let mut best = f64::MAX;
        for _ in 0..5 {
            let t = std::time::Instant::now();
            let mut c = 0;
            for (b, n) in &bufs {
                c += f(b, *n);
            }
            std::hint::black_box(c);
            best = best.min(t.elapsed().as_secs_f64());
        }
        best
    };
    let serial = time(&|b, n| lex(b, n).count);
    eprintln!("{} files, {:.1} MB; serial {:.2} GB/s", bufs.len(), bytes as f64 / 1e6, bytes as f64 / serial / 1e9);
    for blocks in [16, 64, 256, 1 << 30] {
        let chunked = time(&|b, n| lex_chunked(b, n, blocks).count);
        eprintln!("  chunks of {blocks} blocks ({} KB), {} lanes: {:.2} GB/s, {:.2}x the serial time", blocks * 64 / 1024, crate::LANES, bytes as f64 / chunked / 1e9, chunked / serial);
        let off = |b: &[u8]| if b.starts_with(&[0xef, 0xbb, 0xbf]) { 3 } else { 0 };
        let ends = time(&|b, n| (0..(n - off(b)).div_ceil(64)).step_by(blocks).map(|q| crate::stage1_ends_best(b, n, q, q + blocks)[0].m_in_string as usize).sum());
        // Each chunk's true entry (passes 1 and 3), made before pass 2 is timed from it.
        let ends_of = |b: &[u8], n: usize| -> Vec<[crate::Carries; crate::LANES]> { (0..(n - off(b)).div_ceil(64)).step_by(blocks).map(|q| crate::stage1_ends_best(b, n, q, q + blocks)).collect() };
        let entries: Vec<Vec<crate::Carries>> = bufs.iter().map(|(b, n)| crate::entries(&ends_of(b, *n))).collect();
        let scan = time(&|b, n| crate::entries(&ends_of(b, n)).len()) - ends;
        let from = {
            let mut best = f64::MAX;
            for _ in 0..5 {
                let t = std::time::Instant::now();
                for ((b, n), e) in bufs.iter().zip(&entries) {
                    let mut s: Vec<u32> = Vec::with_capacity(n + 67);
                    let mut t: Vec<Vec<u32>> = (0..crate::TYPES).map(|_| Vec::with_capacity(n + 67)).collect();
                    let ptrs: [*mut u32; crate::TYPES] = core::array::from_fn(|k| t[k].as_mut_ptr());
                    let mut at = [0usize; crate::TYPES + 2];
                    for (c, q) in (0..(n - off(b)).div_ceil(64)).step_by(blocks).enumerate() {
                        _ = crate::stage1_from_best(b, *n, q, q + blocks, e[c], &mut at, s.as_mut_ptr(), ptrs, true, core::ptr::null_mut(), [0; 4]);
                    }
                    std::hint::black_box(at[0]);
                }
                best = best.min(t.elapsed().as_secs_f64());
            }
            best
        };
        eprintln!("    pass 1 alone {:.2}x the serial time ({:.2} GB/s), pass 3 {:.3}x, pass 2 from the true entries {:.2}x", ends / serial, bytes as f64 / ends / 1e9, scan.max(0.0) / serial, from / serial);
    }
}

/// Trees whose levels cross chunks: lists of objects and of arrays, deep nests, empty levels, and the refusals tier 2
/// makes of a level's closer, a separator and a value.
fn trees() -> Vec<Vec<u8>> {
    let mut out: Vec<String> = [
        r#"{"a":{"b":[{"c":1},{"d":[2,3]}]},"e":[],"f":{},"g":[{},[],{"h":[[]]}]}"#,
        r#"[[1,[2,[3,[4,{"k":[5,6]}]]]],7,{"x":{"y":{"z":"w"}}},8]"#,
        r#"{"a": [1, 2}"#,
        r#"[1, 2}"#,
        r#"{"a": 1]"#,
        r#"[1,]"#,
        r#"{"a" 1}"#,
        r#"[1]]"#,
        r#"[[1], 2"#,
        r#"{"a": 1,}"#,
        r#"[1 2]"#,
        r#"{"a":[{"b":1},{"b":2}],"c":[[1,2],[3,4]]}"#,
        r#""just a string""#,
        r#"[{"a":1},{"a":2},3]"#,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let deep = |n: usize| "[".repeat(n) + "1" + &",2]".repeat(n);
    out.push(deep(40));
    // Around the deepest the walk opens (`DEPTH`): refused past it.
    out.push(deep(1023));
    out.push(deep(1025));
    out.push(format!("{{\"k\":{}}}", deep(30)));
    // An array of records, and an object of arrays, long enough for many chunks.
    let rows: Vec<String> = (0..60).map(|i| format!(r#"{{"id":{i},"tags":["a","b"],"m":{{"x":[{i},{{}}],"y":null}}}}"#)).collect();
    out.push(format!("[{}]", rows.join(",")));
    let cols: Vec<String> = (0..40).map(|i| format!(r#""c{i}":[{},[{i}],{{"n":true}}]"#, i * 3)).collect();
    out.push(format!("{{{}}}", cols.join(",")));
    out.into_iter().map(String::into_bytes).collect()
}

/// Two states the same: the place, the next record, the depth, the levels' places, the nodes before, the file's end
/// (the last token's end is not read at a list's cycle place, where every chunk starts).
fn same_state(a: &crate::middle::At, b: &crate::middle::At) -> bool {
    // Pass 3's states hold the innermost levels and frames their chunk reads: the serial walk's innermost ones.
    let inner = |x: &[u8], y: &[u8]| x.len() <= y.len() && x == &y[y.len() - x.len()..];
    let inner_frames = |x: &[crate::middle::Frame], y: &[crate::middle::Frame]| x.len() <= y.len() && x == &y[y.len() - x.len()..];
    a.end == b.end && a.nodes == b.nodes && a.prev_start == b.prev_start && (a.end.is_some() || (a.entry.i == b.entry.i && a.entry.depth == b.entry.depth && a.entry.st == b.entry.st && inner(&a.ret, &b.ret) && inner_frames(&a.frames, &b.frames)))
}

#[test]
fn tier2_entries_are_the_serial_walks() {
    let (mut chunks, mut states, mut read, mut anti, mut most_word) = (0, 0, 0, 0, 0);
    let all: Vec<Vec<u8>> = files().into_iter().chain(trees()).collect();
    let entries = crate::ENTRIES.iter().map(|&(kind, field, classes)| rerail_tape_format::tape::Entry { kind, field, flags: 0, alias: None, classes: classes.to_vec() }).collect();
    let lang = rerail_tape_format::f16::Lang::new(rerail_tape_format::tape::CodeTable::new(entries), crate::WIDTHS, 1).expect("a language");
    for (k, f) in all.iter().enumerate() {
        let buf = padded(f);
        let lx = lex(&buf, f.len());
        if lx.refused || lx.count == 0 {
            continue;
        }
        // The serial walk's verdict.
        let mut w = rerail_tape_format::f16::Writer::new(&lang, f, rerail_tape_format::newlines::Terminators::Newlines, 0, false, true, f.len() / 2 + 16);
        let parsed = crate::parse_lexed(&buf, &lx, &mut w);
        for size in [1, 2, 3, 16] {
            // Every pass's chunks and every scan level's in a random order.
            let order = shuffled((k * 31 + size) as u64);
            // Stage 2: every token on its own, in a random order; pass 1 reads them, and is the walk that reads each
            // token itself.
            let ty = crate::lex_typed(&buf, f.len(), 16);
            let toks = crate::middle::tokens(&buf, &lx, &ty.types, &order);
            assert_eq!(toks, crate::middle::tokens(&buf, &lx, &ty.types, &crate::middle::in_order), "file {k}: the tokens in any order");
            read += toks.iter().filter(|t| t.kind != crate::middle::NONE || t.err != crate::middle::NONE).count();
            let got = crate::middle::entries(&buf, &lx, &toks, size, &order);
            assert!(got == crate::middle::entries(&buf, &lx, &[], size, &crate::middle::in_order), "file {k}, chunks of {size}: passes 1 and 3 on the token pass's answers, in any order, are those reading each token in order");
            let want = crate::middle::serial_entries(&buf, &lx, size);
            assert_eq!(want.last().and_then(|a| a.end), Some(parsed), "file {k}: the chunks walked one after another are the serial walk");
            assert_eq!(got.len(), want.len());
            // The states hold what their chunk reads, not the nest: at most as many levels and frames as records.
            let held: usize = got.iter().flatten().map(|a| a.ret.len() + a.frames.len()).sum();
            states = states.max(held / (lx.count + 1));
            assert!(held <= 4 * (lx.count + 1) + 2 * got.len(), "file {k}, chunks of {size}: the states hold {held} levels and frames for {} records", lx.count);
            for (c, (g, w)) in got.iter().zip(&want).enumerate() {
                let g = g.as_ref().unwrap_or_else(|| panic!("file {k}, chunks of {size}: no state at chunk {c}"));
                assert!(same_state(g, w), "file {k}, chunks of {size}, chunk {c}: {g:?}, the serial walk's {w:?}");
            }
            // The bracket scan: each chunk's kinds are the serial walk's levels there (where it has not ended), and
            // every word of the scan's tree is as long as the brackets it spans leave unmatched.
            let cs = crate::middle::chunk_starts(&buf, &lx, size, &order);
            let (words, kinds) = crate::middle::kinds(&buf, &lx, &cs, &order);
            for (c, w) in want.iter().enumerate().take(cs.len()) {
                if w.end.is_some() {
                    break;
                }
                let kc = kinds[c].as_ref().unwrap_or_else(|| panic!("file {k}, chunks of {size}: no kinds at chunk {c}, before the file's end"));
                let places: Vec<u8> = kc[..kc.len() - 1].iter().map(|&x| crate::middle::CONT[x as usize]).collect();
                assert!(w.ret.ends_with(&places), "file {k}, chunks of {size}, chunk {c}: the kinds' places {places:?}, the serial walk's levels {:?}", w.ret);
                anti += words[c].anti.len();
            }
            for level in crate::middle::tree_of(&words, &order, crate::middle::Word::then) {
                most_word = most_word.max(level.iter().map(|x| x.anti.len() + x.opens.len()).max().unwrap_or(0));
            }
            chunks += cs.len();
        }
    }
    eprintln!("{} files in chunks of 1, 2, 3 and 16 records ({chunks} chunks, {read} tokens read by the token pass, {anti} anti-opens): the serial walk's states, one walk a chunk; the longest word {most_word} brackets; the states {states} levels and frames a record", all.len());
}

/// Pass 2: each chunk's nodes written from its state, every pass's chunks (and every scan round's) in order and in two
/// random orders, give the serial walk's tape,
/// byte for byte.
#[test]
fn tier2_tape_from_the_middle_is_the_serial_tape() {
    let all: Vec<Vec<u8>> = files().into_iter().chain(trees()).collect();
    let entries = crate::ENTRIES.iter().map(|&(kind, field, classes)| rerail_tape_format::tape::Entry { kind, field, flags: 0, alias: None, classes: classes.to_vec() }).collect();
    let lang = rerail_tape_format::f16::Lang::new(rerail_tape_format::tape::CodeTable::new(entries), crate::WIDTHS, 1).expect("a language");
    let writer = |f: &[u8]| rerail_tape_format::f16::Writer::new(&lang, f, rerail_tape_format::newlines::Terminators::Newlines, 0, false, true, f.len() / 2 + 16);
    let (mut tapes, mut chunks, mut zipped, mut zbytes, mut plain) = (0, 0, 0, 0, 0);
    for (k, f) in all.iter().enumerate() {
        let buf = padded(f);
        let lx = lex(&buf, f.len());
        let types = crate::lex_typed(&buf, f.len(), 16).types;
        let mut w = writer(f);
        let parsed = crate::parse_lexed(&buf, &lx, &mut w);
        let want = parsed.ok().map(|()| w.finish().expect("the serial tape").tape.as_bytes().to_vec());
        for size in [1, 3, 16] {
            for seed in [0u64, 1, 2] {
                let mut m = writer(f);
                let random = shuffled((k * 97 + size) as u64 * 3 + seed);
                let order: crate::middle::Order<'_> = if seed == 0 { &crate::middle::in_order } else { &random };
                let got = crate::middle::parse_middle(&buf, &lx, &types, &mut m, size, order);
                assert_eq!(got, parsed, "file {k}, chunks of {size}: the verdict");
                if let Some(want) = &want {
                    let have = m.finish().expect("the tape from the middle").tape.as_bytes().to_vec();
                    assert!(have == *want, "file {k}, chunks of {size}, order {seed}: the tape from the middle differs at byte {:?} of {}", have.iter().zip(want).position(|(a, b)| a != b), want.len());
                    tapes += 1;
                    chunks += crate::middle::chunk_starts(&buf, &lx, size, &crate::middle::in_order).len();
                    // S6: its 16z, every piece on its own by the greedy encoder, decodes to it.
                    if seed == 0 {
                        let z = rerail_tape_format::f16::z_of(&have, &rerail_tape_format::f16::lz4_greedy).expect("its 16z");
                        let t = rerail_tape_format::f16::Tape16Z::from_bytes(rerail_tape_format::Vector::from(&z[..])).unwrap_or_else(|e| panic!("file {k}: its 16z: {e}"));
                        assert!(t.decoded_whole().expect("decodes") == have, "file {k}, chunks of {size}: its 16z decodes to it");
                        (zipped, zbytes) = (zipped + 1, zbytes + z.len());
                        plain += have.len();
                    }
                }
            }
        }
    }
    eprintln!("{tapes} tapes from the middle ({chunks} chunks), each the serial tape; {zipped} as 16z by the greedy encoder, {plain} bytes to {zbytes}, each decoding to its tape");
}

/// A walk that writes nothing: the serial walk's places, its oracle.
struct Nothing;

impl crate::middle::Out for Nothing {
    unsafe fn open(&mut self, _: u32, _: u64) {}
    unsafe fn leaf(&mut self, _: u32, _: u64, _: u64) {}
    unsafe fn leaf_in(&mut self, _: u32, _: u32, _: u64, _: u64, _: u64, _: u64) {}
    unsafe fn close(&mut self, _: u64) {}
}

/// The serial walk a record at a time (`run_step` from the root, its levels its own): its place at each record, up
/// to its end or refusal.
fn serial_places(buf: &[u8], lx: &crate::tier1::Lexed) -> Vec<u8> {
    let mut ret = [0u8; crate::middle::STEP_DEPTH];
    let mut en = crate::middle::Entry::default();
    let mut out = Vec::new();
    loop {
        out.push(en.st);
        // SAFETY: the walk holds its levels from the root.
        match unsafe { crate::middle::run_step(buf, lx, &mut Nothing, en, &mut ret, 0, en.i + 1) } {
            crate::middle::Stop::At(x) => en = x,
            _ => return out,
        }
    }
}

/// Tier 2 as phases (decision 0726): at chunks of 1, 2, 3 and 16 records and in order and two random orders, each
/// chunk's place at its start (P4's sum) is the serial walk's at that record, every verdict the serial walk's (a
/// refusal at its byte), and every tape, in 16 and in 16z, the serial tape byte for byte, made wholly by phases: its
/// records and its newline index by the lexer's (`lex_count`, `Counted::write`), its rows (`parse_phases`), and its
/// blocks, its 16z pieces and its tail (`finish_by_phases`).
#[test]
fn tier2_by_phases_is_the_serial_tape() {
    let all: Vec<Vec<u8>> = files().into_iter().chain(trees()).collect();
    let entries = crate::ENTRIES.iter().map(|&(kind, field, classes)| rerail_tape_format::tape::Entry { kind, field, flags: 0, alias: None, classes: classes.to_vec() }).collect();
    let lang = rerail_tape_format::f16::Lang::new(rerail_tape_format::tape::CodeTable::new(entries), crate::WIDTHS, 1).expect("a language");
    let writer = |f: &[u8], z: bool| rerail_tape_format::f16::Writer::new(&lang, f, rerail_tape_format::newlines::Terminators::Newlines, 0, z, true, f.len() / 2 + 16);
    let (mut tapes, mut refused, mut places) = (0, 0, 0);
    for (k, f) in all.iter().enumerate() {
        let buf = padded(f);
        let lx = lex(&buf, f.len());
        let types = crate::lex_typed(&buf, f.len(), 16).types;
        // The serial tapes, 16 and 16z.
        let wants: Vec<Option<Vec<u8>>> = [false, true]
            .map(|z| {
                let mut w = writer(f, z);
                crate::parse_lexed(&buf, &lx, &mut w).ok().map(|()| w.finish().expect("the serial tape").tape.as_bytes().to_vec())
            })
            .to_vec();
        let parsed = crate::parse_lexed(&buf, &lx, &mut writer(f, false));
        let serial = if lx.refused || lx.count == 0 { Vec::new() } else { serial_places(&buf, &lx) };
        for size in [1, 2, 3, 16] {
            for seed in [0u64, 1, 2] {
                let random = shuffled((k * 31 + size) as u64 * 3 + seed);
                let order: crate::middle::Order<'_> = if seed == 0 { &crate::middle::in_order } else { &random };
                if !serial.is_empty() {
                    let toks = crate::middle::tokens(&buf, &lx, &types, order);
                    let ph = crate::middle::Phases::run(&buf, &lx, &toks, size, order);
                    let last = ph.refused.unwrap_or(ph.place.len());
                    for c in 0..last {
                        let r = c * size;
                        if r < serial.len() {
                            assert_eq!(ph.place[c], serial[r], "file {k}, chunks of {size}, order {seed}: chunk {c}'s place");
                            places += 1;
                        }
                    }
                }
                for (z, want) in [false, true].into_iter().zip(&wants) {
                    let mut m = rerail_tape_format::f16::Writer::empty();
                    let doc = rerail_tape_format::f16::Document { lang: &lang, source: f, tag: 0, z, blocks: true, nodes: f.len() / 2 + 16 };
                    // L1 and L2, then L3 into the tape's line index: the lexer's newlines, at their ranks.
                    let counted = crate::lex_count(&buf, f.len(), size, order);
                    let mut typed = None;
                    m.begin_with(&doc, |out| typed = Some(counted.write(Some(out), order)));
                    let typed = typed.expect("lexed");
                    assert!(typed.lexed.starts == lx.starts && typed.types == types, "file {k}: the lexer's records by phases");
                    let got = crate::middle::parse_phases(&buf, &typed.lexed, &typed.types, &mut m, size, order);
                    assert_eq!(got, parsed, "file {k} ({:?}), chunks of {size}, order {seed}: the verdict", String::from_utf8_lossy(&f[..f.len().min(60)]));
                    match want {
                        Some(want) => {
                            let have = m.finish_by_phases(order).expect("the tape by phases").tape.as_bytes().to_vec();
                            assert!(have == *want, "file {k}, 16z {z}, chunks of {size}, order {seed}: the tape by phases differs at byte {:?} of {}", have.iter().zip(want).position(|(a, b)| a != b), want.len());
                            tapes += 1;
                        }
                        None => refused += 1,
                    }
                }
            }
        }
    }
    eprintln!("{tapes} tapes by phases, each the serial tape byte for byte; {refused} refusals, each at the serial byte; {places} chunks' places, each the serial walk's");
}

/// Every phase's merge is associative, on random triples (and its unit an identity).
#[test]
fn tier2_phase_merges_are_associative() {
    use crate::middle::phases::{Depth, Kids, Last, Shape, places_merge, places_unit, STEP_ENDS, STEP_REFUSED};
    let mut x = 0x9e37_79b9_7f4a_7c15u64;
    let mut next = |m: u64| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x % m
    };
    for _ in 0..10_000 {
        let d: Vec<Depth> = (0..3).map(|_| {
            let dd = next(7) as i64 - 3;
            Depth { d: dd, m: dd.min(0) - next(3) as i64, items: next(9) as u32 }
        }).collect();
        assert_eq!(Depth::merge(&Depth::merge(&d[0], &d[1]), &d[2]), Depth::merge(&d[0], &Depth::merge(&d[1], &d[2])));
        let l: Vec<Last> = (0..3).map(|_| Last { reset: next(3) == 0, v: if next(2) == 0 { u32::MAX } else { next(100) as u32 } }).collect();
        assert_eq!(Last::merge(&Last::merge(&l[0], &l[1]), &l[2]), Last::merge(&l[0], &Last::merge(&l[1], &l[2])));
        assert_eq!(Last::merge(&Last::UNIT, &l[0]), l[0]);
        let kd: Vec<Kids> = (0..3).map(|_| Kids { reset: next(3) == 0, count: next(5) as u32, last: if next(2) == 0 { u32::MAX } else { next(100) as u32 } }).collect();
        assert_eq!(Kids::merge(&Kids::merge(&kd[0], &kd[1]), &kd[2]), Kids::merge(&kd[0], &Kids::merge(&kd[1], &kd[2])));
        let s: Vec<Shape> = (0..3)
            .map(|_| {
                let net = next(7) as i64 - 3;
                Shape { nodes: next(9) as u32, events: next(9) as u32, items: next(9) as u32, net, low: net.min(0) - next(3) as i64, last_start: (next(2) == 0).then(|| next(100)), e: (next(2) == 0).then(|| next(100) as usize) }
            })
            .collect();
        assert_eq!(Shape::merge(&Shape::merge(&s[0], &s[1]), &s[2]), Shape::merge(&s[0], &Shape::merge(&s[1], &s[2])));
        let p: Vec<[u8; 32]> = (0..3).map(|_| {
            let mut m = places_unit();
            for q in m.iter_mut().take(crate::middle::STEP_PLACES) {
                *q = match next(20) {
                    0 => STEP_REFUSED,
                    1 => STEP_ENDS,
                    _ => next(crate::middle::STEP_PLACES as u64) as u8,
                };
            }
            m
        }).collect();
        assert_eq!(places_merge(&places_merge(&p[0], &p[1]), &p[2]), places_merge(&p[0], &places_merge(&p[1], &p[2])));
        assert_eq!(places_merge(&places_unit(), &p[0]), p[0]);
    }
}

/// The lexer's newlines (L2 counts them, L3 puts them at their ranks) make the tape's line index, byte for byte the
/// index of a scan of the source: `\n`, `\r\n` and a lone `\r` at every place about the blocks' and chunks' ends, in
/// chunks of 1 to 3 blocks, in order and at random.
#[test]
fn the_lexers_newlines_are_the_line_index() {
    let mut checked = 0;
    for len in [1usize, 63, 64, 65, 127, 128, 129, 200, 400] {
        for (k, ends) in [&b"\n"[..], b"\r\n", b"\r", b"\n\r"].iter().enumerate() {
            for at in (0..len).step_by(7).chain([len.saturating_sub(1), len.saturating_sub(2)]) {
                // An array of numbers, the terminator in the white space at `at` and every 61 bytes.
                let mut f = b"[".to_vec();
                while f.len() < len {
                    if f.len() % 61 == at % 61 {
                        f.extend_from_slice(ends);
                    } else {
                        f.extend_from_slice(b"1,");
                    }
                }
                f.extend_from_slice(b"1]");
                let want = rerail_tape_format::newlines::index(&f, rerail_tape_format::newlines::Terminators::Newlines);
                let buf = padded(&f);
                for blocks in [1usize, 2, 3] {
                    for seed in 0..2u64 {
                        let random = shuffled(seed + k as u64);
                        let order: crate::tier1::Order<'_> = if seed == 0 { &crate::tier1::in_order } else { &random };
                        let mut got = rerail_tape_format::Vector::new();
                        let typed = crate::lex_count(&buf, f.len(), blocks, order).write(Some(&mut got), order);
                        assert!(got[..] == want[..], "{len} bytes, {ends:?} at {at}, chunks of {blocks}: the index differs");
                        assert_eq!(typed.lexed.starts, lex(&buf, f.len()).starts);
                        checked += 1;
                    }
                }
            }
        }
    }
    eprintln!("{checked} line indexes from the lexer, each the scan's");
}
