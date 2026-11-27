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
                    let mut s = Vec::with_capacity(n + 67);
                    for (c, q) in (0..(n - off(b)).div_ceil(64)).step_by(blocks).enumerate() {
                        _ = crate::stage1_from_best(b, *n, q, q + blocks, e[c], &mut s);
                    }
                    std::hint::black_box(s.len());
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
    a.end == b.end && a.nodes == b.nodes && a.prev_start == b.prev_start && (a.end.is_some() || (a.entry.i == b.entry.i && a.entry.depth == b.entry.depth && a.entry.st == b.entry.st && a.ret == b.ret && a.frames == b.frames))
}

#[test]
fn tier2_entries_are_the_serial_walks() {
    let (mut chunks, mut most) = (0, 0);
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
            let sums = crate::middle::summaries(&buf, &lx, size);
            let got = crate::middle::entries(&sums);
            let want = crate::middle::serial_entries(&buf, &lx, size);
            assert_eq!(want.last().and_then(|a| a.end), Some(parsed), "file {k}: the chunks walked one after another are the serial walk");
            assert_eq!(got.len(), want.len());
            for (c, (g, w)) in got.iter().zip(&want).enumerate() {
                let g = g.as_ref().unwrap_or_else(|| panic!("file {k}, chunks of {size}: no lane meets chunk {c}'s entry"));
                assert!(same_state(g, w), "file {k}, chunks of {size}, chunk {c}: {g:?}, the serial walk's {w:?}");
            }
            chunks += sums.len();
            most = most.max(sums.iter().map(|s| s.lanes.len()).max().unwrap_or(0));
        }
    }
    eprintln!("{} files in chunks of 1, 2, 3 and 16 records ({chunks} chunks): the serial walk's states; at most {most} lanes a chunk", all.len());
}

/// Pass 2: each chunk's nodes written from its state, the chunks in order and in reverse, give the serial walk's tape,
/// byte for byte.
#[test]
fn tier2_tape_from_the_middle_is_the_serial_tape() {
    let all: Vec<Vec<u8>> = files().into_iter().chain(trees()).collect();
    let entries = crate::ENTRIES.iter().map(|&(kind, field, classes)| rerail_tape_format::tape::Entry { kind, field, flags: 0, alias: None, classes: classes.to_vec() }).collect();
    let lang = rerail_tape_format::f16::Lang::new(rerail_tape_format::tape::CodeTable::new(entries), crate::WIDTHS, 1).expect("a language");
    let writer = |f: &[u8]| rerail_tape_format::f16::Writer::new(&lang, f, rerail_tape_format::newlines::Terminators::Newlines, 0, false, true, f.len() / 2 + 16);
    let (mut tapes, mut chunks) = (0, 0);
    for (k, f) in all.iter().enumerate() {
        let buf = padded(f);
        let lx = lex(&buf, f.len());
        let mut w = writer(f);
        let parsed = crate::parse_lexed(&buf, &lx, &mut w);
        let want = parsed.ok().map(|()| w.finish().expect("the serial tape").tape.as_bytes().to_vec());
        for size in [1, 3, 16] {
            for reverse in [false, true] {
                let mut m = writer(f);
                let got = crate::middle::parse_middle(&buf, &lx, &mut m, size, |n| if reverse { (0..n).rev().collect() } else { (0..n).collect() });
                assert_eq!(got, parsed, "file {k}, chunks of {size}: the verdict");
                if let Some(want) = &want {
                    let have = m.finish().expect("the tape from the middle").tape.as_bytes().to_vec();
                    assert!(have == *want, "file {k}, chunks of {size}, reverse {reverse}: the tape from the middle differs at byte {:?} of {}", have.iter().zip(want).position(|(a, b)| a != b), want.len());
                    tapes += 1;
                    chunks += crate::middle::chunk_starts(&buf, &lx, size).len();
                }
            }
        }
    }
    eprintln!("{tapes} tapes from the middle ({chunks} chunks), each the serial tape");
}
