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
                    let mut at = [0usize; crate::TYPES + 1];
                    for (c, q) in (0..(n - off(b)).div_ceil(64)).step_by(blocks).enumerate() {
                        _ = crate::stage1_from_best(b, *n, q, q + blocks, e[c], &mut at, s.as_mut_ptr(), ptrs, true);
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

/// What a walk's nodes count, through the token pass's answers (pass 1's `Shadow` without its frames).
struct Count<'a> {
    toks: &'a [crate::middle::Token],
    nodes: usize,
}

impl crate::middle::Out for Count<'_> {
    fn known(&self, i: usize) -> Option<Result<crate::middle::Token, u32>> {
        let t = *self.toks.get(i)?;
        if t.err != crate::middle::NONE {
            Some(Err(t.err))
        } else if t.kind == crate::middle::NONE {
            None
        } else {
            Some(Ok(t))
        }
    }
    unsafe fn skip(&mut self, _: u64, t: crate::middle::Token) -> usize {
        self.nodes += t.nodes as usize;
        t.end as usize
    }
    unsafe fn open(&mut self, _: u32, _: u64) {
        self.nodes += 1;
    }
    unsafe fn leaf(&mut self, _: u32, _: u64, _: u64) {
        self.nodes += 1;
    }
    unsafe fn leaf_in(&mut self, _: u32, _: u32, _: u64, _: u64, _: u64, _: u64) {
        self.nodes += 2;
    }
    unsafe fn close(&mut self, _: u64) {}
}

/// A step map's values past the places: the step closes its level, ends the file, or is refused.
const CLOSES: u8 = 253;
const ENDS: u8 = 254;
const REFUSED: u8 = 255;

/// Record `r`'s step from each place (`run_step` to the first place past it, at the walk's depth `depth`, nothing
/// known of the levels below): the place after it, or where it opens a level the place that level returns to (its
/// parent's next place), and the place its level starts at; `CLOSES`, `ENDS` or `REFUSED`. And its nodes from each.
fn step_map(buf: &[u8], lx: &crate::tier1::Lexed, toks: &[crate::middle::Token], r: usize, depth: usize) -> ([u8; 32], [u8; 32], [u32; 32]) {
    use crate::middle::{Entry, Stop};
    let (mut map, mut child, mut nodes) = ([REFUSED; 32], [REFUSED; 32], [0u32; 32]);
    for p in 0..crate::middle::STEP_PLACES {
        let mut ret = [0u8; crate::middle::STEP_DEPTH];
        let mut d = Count { toks, nodes: 0 };
        // SAFETY: `buf` is the file padded, `lx` its records.
        let stop = unsafe { crate::middle::run_step(buf, lx, &mut d, Entry { i: r, e: 0, depth, st: p as u8 }, &mut ret, depth, r + 1) };
        nodes[p] = d.nodes as u32;
        map[p] = match stop {
            Stop::At(x) if x.i == r + 1 && x.depth == depth + 1 => {
                child[p] = x.st;
                ret[depth]
            }
            Stop::At(x) if x.i == r + 1 && x.depth == depth => x.st,
            Stop::At(x) => panic!("record {r} from place {p}: a step to record {} at depth {}, from depth {depth}", x.i, x.depth),
            Stop::Read(_) => CLOSES,
            Stop::Done(Ok(())) => ENDS,
            Stop::Done(Err(_)) => REFUSED,
        };
    }
    (map, child, nodes)
}

/// Tier 2 by record (decision 0726): each record's place found with no walk, by its level, a stable sort by level and
/// one segmented scan of step maps, equals the serial walk's place there, record by record; the refusal is the first
/// record whose step from its place is refused, at the serial walk's record. Each record's step map is the generated
/// loop's own (`run_step`, the loop with every record read ending a place), from each of its places. The scan composes
/// maps (a place in, a place out) in a random order of each round's elements.
#[test]
fn tier2_places_by_record_are_the_walks() {
    use crate::middle::{Entry, Stop, STEP_PLACES};
    let all: Vec<Vec<u8>> = files().into_iter().chain(trees()).collect();
    let (mut records, mut files_done, mut refused) = (0, 0, 0);
    for (k, f) in all.iter().enumerate() {
        let buf = padded(f);
        let lx = lex(&buf, f.len());
        if lx.refused || lx.count == 0 {
            continue;
        }
        let n = lx.count;
        let ty = crate::lex_typed(&buf, f.len(), 16);
        let toks = crate::middle::tokens(&buf, &lx, &ty.types, &crate::middle::in_order);
        // The serial walk a record at a time: its place before each record, every read a place, and its verdict.
        let mut serial = vec![REFUSED; n + 1];
        let mut ret = [0u8; crate::middle::STEP_DEPTH];
        let mut en = Entry { i: 0, e: 0, depth: 0, st: 0 };
        let mut d = Count { toks: &toks, nodes: 0 };
        let verdict = loop {
            serial[en.i] = en.st;
            // SAFETY: as `step_map`'s.
            match unsafe { crate::middle::run_step(&buf, &lx, &mut d, en, &mut ret, 0, en.i + 1) } {
                Stop::At(x) => {
                    assert_eq!(x.i, en.i + 1, "file {k}: a step past record {} took more than one record", en.i);
                    en = x;
                }
                Stop::Done(r) => break (en.i, r),
                Stop::Read(_) => unreachable!("every level is known from the start"),
            }
        };
        let entries = crate::ENTRIES.iter().map(|&(kind, field, classes)| rerail_tape_format::tape::Entry { kind, field, flags: 0, alias: None, classes: classes.to_vec() }).collect();
        let lang = rerail_tape_format::f16::Lang::new(rerail_tape_format::tape::CodeTable::new(entries), crate::WIDTHS, 1).expect("a language");
        let mut w = rerail_tape_format::f16::Writer::new(&lang, f, rerail_tape_format::newlines::Terminators::Newlines, 0, false, true, f.len() / 2 + 16);
        assert_eq!(verdict.1, crate::parse_lexed(&buf, &lx, &mut w), "file {k}: the walk a record at a time's verdict is the serial walk's");
        // Each record's first byte and its change of the depth; an empty level's opener and closer (a local read).
        let byte = |r: usize| buf[lx.starts[r] as usize];
        let (opens, closes) = (|b: u8| b == b'{' || b == b'[', |b: u8| b == b'}' || b == b']');
        let pair = |o: u8, c: u8| (o == b'{' && c == b'}') || (o == b'[' && c == b']');
        let empty_open = |r: usize| r + 1 < n && opens(byte(r)) && pair(byte(r), byte(r + 1));
        let empty_close = |r: usize| r > 0 && closes(byte(r)) && pair(byte(r - 1), byte(r));
        let delta: Vec<i64> = (0..n).map(|r| if opens(byte(r)) { 1 } else if closes(byte(r)) { -1 } else { 0 }).collect();
        // The depth before each record: a prefix sum (exclusive).
        let order = shuffled(k as u64 + 7);
        let incl = crate::middle::scan(delta.clone(), &order, |a, b| a + b);
        let before = |r: usize| if r == 0 { 0 } else { incl[r - 1] };
        // The walk's depth before a record: an empty level's closer is inside no level the walk opened.
        let walk_depth = |r: usize| (before(r) - i64::from(r < n && empty_close(r))).max(0) as usize;
        // Items: (level, record, head): an opener of a level its parent's item and its own level's head; an empty
        // level's opener and closer, and every other record, one item at the level they are read in. The file's end
        // (record n) an item of the level open there.
        let mut items: Vec<(i64, usize, bool)> = Vec::new();
        for r in 0..=n {
            let b = if r < n { byte(r) } else { 0 };
            if r < n && opens(b) && !empty_open(r) {
                items.push((before(r), r, false));
                items.push((before(r) + 1, r, true));
            } else if r < n && closes(b) && !empty_close(r) {
                items.push((before(r), r, false));
            } else if r < n && empty_close(r) {
                items.push((before(r) - 1, r, false));
            } else {
                items.push((if r < n { before(r) } else { incl[n - 1] }, r, false));
            }
        }
        // The stable sort by level (the records already in order, a head before its level's items).
        items.sort_by_key(|&(l, r, h)| (l, r, !h));
        // Each item's step map: a head's is the level's start (the place its opener's step opens it at, the same
        // from every place its opener is read at); an opener's in its parent, the place its level returns to.
        let maps: Vec<[u8; 32]> = (0..=n).map(|_| [REFUSED; 32]).collect();
        let mut maps = maps;
        let mut starts = vec![REFUSED; n + 1];
        for r in 0..=n {
            let (m, c, _) = step_map(&buf, &lx, &toks, r, walk_depth(r));
            maps[r] = m;
            let mut s = c.iter().take(STEP_PLACES).copied().filter(|&x| x != REFUSED);
            if let Some(first) = s.next() {
                assert!(s.all(|x| x == first), "file {k}: record {r}'s level starts at more than one place");
                starts[r] = first;
            }
        }
        // The segmented scan: (head, map). A segment's first item is its head (the level's start, a constant map);
        // the root level's first item starts at place 0.
        let compose = |a: &[u8; 32], b: &[u8; 32]| -> [u8; 32] {
            let mut out = [REFUSED; 32];
            for p in 0..STEP_PLACES {
                out[p] = if (a[p] as usize) < STEP_PLACES { b[a[p] as usize] } else { a[p] };
            }
            out
        };
        let constant = |s: u8| -> [u8; 32] { [s; 32] };
        let elems: Vec<(bool, [u8; 32])> = items
            .iter()
            .enumerate()
            .map(|(x, &(l, r, h))| {
                if h {
                    (true, constant(starts[r]))
                } else if x == 0 || items[x - 1].0 != l {
                    // A level's first item with no head: the root's, from place 0.
                    let mut m = maps[r];
                    let first = m[0];
                    m = constant(first);
                    (true, m)
                } else {
                    (false, maps[r])
                }
            })
            .collect();
        let scanned = crate::middle::scan(elems.clone(), &order, |a, b| if b.0 { *b } else { (a.0, compose(&a.1, &b.1)) });
        // Each record's place: the scan before its item, applied to anything (a segment's value is constant).
        let mut place = vec![REFUSED; n + 1];
        for (x, &(l, r, h)) in items.iter().enumerate() {
            if h {
                continue;
            }
            let first = x == 0 || items[x - 1].0 != l;
            place[r] = if first { 0 } else { scanned[x - 1].1[0] };
        }
        // The refusal: the first record whose step from its place is refused (or whose place is none).
        // A file whose root ends at its last record's step has ended there: its end's place is `ENDS`.
        let stuck = (0..=n).find(|&r| if place[r] == ENDS { r < n } else { place[r] as usize >= STEP_PLACES || maps[r][place[r] as usize] == REFUSED });
        for r in 0..=verdict.0 {
            if Some(r) == stuck {
                break;
            }
            assert_eq!(place[r], serial[r], "file {k}: record {r}'s place by its level's scan, the walk's");
        }
        match verdict.1 {
            Ok(()) => assert!(stuck.is_none(), "file {k} ({:?}): refused by the scan at record {stuck:?}, the walk accepted it; places {:?}, the walk's {:?}", String::from_utf8_lossy(&f[..f.len().min(80)]), &place[..place.len().min(12)], &serial[..serial.len().min(12)]),
            Err(_) => {
                refused += 1;
                assert_eq!(stuck, Some(verdict.0), "file {k}: refused by the scan at record {stuck:?}, the walk at {}", verdict.0);
            }
        }
        records += n;
        files_done += 1;
    }
    eprintln!("{files_done} files ({refused} refused), {records} records: each record's place by its level's scan is the walk's");
}
