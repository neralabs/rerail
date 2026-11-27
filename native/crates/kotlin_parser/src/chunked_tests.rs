//! Tier 1 from the middle (`rerail_kotlin_aot::lex_chunked`): a file lexed in chunks, each from every class of entries
//! (its code entry, and the nest below it it reads), the entries joined by a prefix sum of the chunks' summaries, then
//! each chunk from its entry, is the serial lexer's (`lex`), record for record.

use rerail_kotlin_aot::{Entry, IN_FLIGHT, LEVEL_KINDS, Lexed, Summary, apply, compose, entries, entries_waits, lex, lex_chunked, serial_entries, summaries};

/// A string with templates nested `k` deep, each template's code a call with a string in it.
fn nested(k: usize) -> String {
    let mut s = String::from("x");
    for i in 0..k {
        s = format!("f{i}(\"a{i}${{{s}}}b{i}\")");
    }
    s
}

/// The cases: the kinds of regions and templates, and each cut by chunk edges at every offset.
fn files() -> Vec<String> {
    let mut out: Vec<String> = vec![
        "fun main() { println(\"hi ${name}!\") }\n".to_owned(),
        "val s = \"a${ f(\"b${x}c\") }d\"\nval t = 1\n".to_owned(),
        "val r = \"\"\"raw ${x} \"quoted\" ${ y { it } }\"\"\"\n".to_owned(),
        "val m = $$\"cost $${price} and ${not}\"\n".to_owned(),
        "val c = '\\'' + 'x' + `a name`\n".to_owned(),
        "/* a /* nested */ comment */ val x = 1 // a line\n".to_owned(),
        "val l = list.map { \"${it.name}: ${it.value}\" }.filter { it.isNotEmpty() }\n".to_owned(),
    ];
    for k in 1..=5 {
        out.push(format!("val v = {}\n", nested(k)));
    }
    out.push("/* 1 /* 2 /* 3 /* 4 */ */ */ */ val y = 2\n".to_owned());
    for at in 0..140 {
        let pad = " ".repeat(at);
        out.push(format!("{pad}val s = \"a${{ f(\"b${{x{pad}}}c\") }}d\"\n"));
        out.push(format!("val a = 1{pad}// a comment {pad}\nval b = 2\n"));
        out.push(format!("val a = 1 /* a {pad} /* in */ block */ val b = \"{pad}\\\\\"\n"));
        out.push(format!("val r = \"\"\"{pad}${{x}}{pad}\"\"\" + 'c' + `n{pad}`\n"));
        out.push(format!("{pad}fun g() = \"${{ list.map {{ \"{pad}${{it}}\" }} }}\"\n"));
        out.push(format!("val v = {pad}{}\n", nested(3)));
        out.push(format!("val s = \"a{pad}"));
        out.push(format!("val a = 1 /* {pad}"));
    }
    out
}

/// `lex_chunked`'s records and the serial lexer's: the same refusal, and where it lexes the file, the same records and
/// comments. The chunks entered again where pass 3's entry was not the one the chunk before left.
fn same(text: &str, blocks: usize) -> usize {
    let want = lex(text.as_bytes());
    let (got, again) = lex_chunked(text.as_bytes(), blocks);
    let recs = |l: &Lexed| (l.recs.iter().map(|r| (r.start, r.last, r.code)).collect::<Vec<_>>(), l.extras.iter().map(|r| (r.start, r.last, r.code)).collect::<Vec<_>>());
    assert_eq!(got.refused, want.refused, "chunks of {blocks} blocks: refused, in\n{text}");
    if !want.refused {
        assert!(recs(&got) == recs(&want), "chunks of {blocks} blocks: the records, in\n{text}");
    }
    again
}

/// The lanes a file's chunks hold: the most in one chunk, the chunks past `L`, and those pass 3 had wait for their
/// predecessor's entry (`entries_waits`).
fn lanes(text: &str, blocks: usize) -> (usize, usize, usize) {
    let s = summaries(text.as_bytes(), blocks);
    (s.iter().map(|s| s.lanes().len()).max().unwrap_or(0), s.iter().filter(|s| s.over).count(), entries_waits(&s).1)
}

#[test]
fn chunked_is_the_serial_lexer() {
    let (files, mut again, mut chunks, mut most) = (files(), 0, 0, 0);
    for f in &files {
        for blocks in [1, 2, 3, 16] {
            again += same(f, blocks);
            chunks += f.len().div_ceil(64 * blocks).max(1);
            most = most.max(lanes(f, blocks).0);
        }
    }
    eprintln!("{} files, each in chunks of 1, 2, 3 and 16 blocks ({chunks} chunks): the serial lexer's records; {again} chunks entered again; at most {most} lanes a chunk", files.len());
}

/// A file whose chunks the summary gives every entry of: each chunk's entry by the prefix sum is the serial lexer's.
#[test]
fn nested_templates_are_entered_where_the_serial_lexer_is() {
    for k in 1..=4 {
        for at in 0..64 {
            let f = format!("{}val v = {}\n", " ".repeat(at), nested(k));
            assert_eq!(same(&f, 1), 0, "templates {k} deep, cut {at} bytes in: a chunk entered again, in\n{f}");
        }
    }
}

/// Summaries compose in any tree: each chunk's entry by the prefix sum is the one composed left to right, and
/// `(A·B)·C = A·(B·C)`.
#[test]
fn summaries_compose_in_any_order() {
    for f in files().iter().filter(|f| f.len() > 128) {
        let ends: Vec<Summary> = summaries(f.as_bytes(), 1);
        let scanned = entries(&ends);
        let mut left = ends[0];
        for c in 1..ends.len() {
            assert_eq!(scanned[c], apply(&left, &Entry::START, 0), "chunk {c}, in\n{f}");
            left = compose(&left, &ends[c]);
        }
        // As functions of the entry: at each place a lane of `A` starts, before `A`, and past it, with each nest of up to
        // two levels of each kind.
        let nests: Vec<Vec<u8>> = std::iter::once(vec![]).chain(LEVEL_KINDS.iter().map(|&k| vec![k])).chain(LEVEL_KINDS.iter().flat_map(|&k| LEVEL_KINDS.iter().map(move |&m| vec![k, m]))).collect();
        for w in ends.windows(3) {
            let (a, b, c) = (&w[0], &w[1], &w[2]);
            let (l, r) = (compose(&compose(a, b), c), compose(a, &compose(b, c)));
            let mut ats: Vec<usize> = a.lanes().iter().map(|l| l.at0).collect();
            ats.extend([a.start.saturating_sub(1), c.end]);
            for &at in &ats {
                for nest in &nests {
                    let mut e = Entry::START;
                    e.at = at;
                    for (d, &k) in nest.iter().enumerate() {
                        e.nest[d] = if k == 255 { usize::MAX } else { usize::from(k) };
                        if k != 255 {
                            e.held[e.opened] = 1000 + d;
                            e.opened += 1;
                        }
                    }
                    e.depth = nest.len();
                    e.template = if e.depth == 0 { usize::MAX } else { e.nest[e.depth - 1] };
                    let (x, y) = (apply(&l, &e, 7), apply(&r, &e, 7));
                    // (A refused file's entries past the refusal are no one's.)
                    if let (Some(x), Some(y)) = (x, y)
                        && !(x.2 && y.2)
                        && x != y
                    {
                        panic!("(A·B)·C is not A·(B·C) at {at}, nest {nest:?}: {x:?}\n{y:?}\nin\n{f}");
                    }
                }
            }
        }
    }
}

/// The corpus (`RERAIL_KOTLIN_CORPUS`, a folder of `.kt` files): the serial lexer's records in chunks of 1, 2, 16 and
/// 256 blocks, and the chunks entered again.
#[test]
#[ignore = "reads RERAIL_KOTLIN_CORPUS, a folder of .kt files"]
fn corpus_chunked_is_the_serial_lexer() {
    let dir = std::env::var("RERAIL_KOTLIN_CORPUS").expect("set RERAIL_KOTLIN_CORPUS");
    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    paths.sort();
    let files: Vec<String> = paths.iter().filter_map(|p| std::fs::read_to_string(p).ok()).collect();
    let bytes: usize = files.iter().map(String::len).sum();
    let (mut again, mut chunks, mut most, mut over, mut waits) = ([0usize; 4], [0usize; 4], [0usize; 4], [0usize; 4], [0usize; 4]);
    for f in &files {
        for (i, blocks) in [1, 2, 16, 256].into_iter().enumerate() {
            again[i] += same(f, blocks);
            chunks[i] += f.len().div_ceil(64 * blocks).max(1);
            let (m, o, w) = lanes(f, blocks);
            most[i] = most[i].max(m);
            over[i] += o;
            waits[i] += w;
        }
    }
    eprintln!("{} files, {:.1} MB: the serial lexer's records in chunks of 1, 2, 16 and 256 blocks", files.len(), bytes as f64 / 1e6);
    eprintln!("  chunks {chunks:?}; entered again {again:?}; the most lanes a chunk {most:?}; chunks past L {over:?}; pass 3's waits ({IN_FLIGHT} in flight) {waits:?}");
}

/// Where pass 3's entry is not the serial lexer's: none given, or the fields that differ, counted over the cases.
#[test]
fn report_the_entries_pass_3_misses() {
    report_misses(&files());
}

/// The same over the corpus's first `RERAIL_REPORT_FILES` files (40).
#[test]
#[ignore = "reads RERAIL_KOTLIN_CORPUS, a folder of .kt files"]
fn report_the_corpus_entries_pass_3_misses() {
    let dir = std::env::var("RERAIL_KOTLIN_CORPUS").expect("set RERAIL_KOTLIN_CORPUS");
    let k: usize = std::env::var("RERAIL_REPORT_FILES").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    paths.sort();
    report_misses(&paths.iter().take(k).filter_map(|p| std::fs::read_to_string(p).ok()).collect::<Vec<_>>());
}

fn report_misses(files: &[String]) {
    let mut why: std::collections::BTreeMap<String, usize> = Default::default();
    let mut example: std::collections::BTreeMap<String, String> = Default::default();
    for f in files {
        for blocks in [1, 2] {
            let pred = entries(&summaries(f.as_bytes(), blocks));
            for (c, (want, recs)) in serial_entries(f.as_bytes(), blocks).into_iter().enumerate() {
                let k = match pred[c] {
                    None => "no lane".to_owned(),
                    Some((got, r, _)) => {
                        let (g, w) = (format!("{got:?}"), format!("{want:?}"));
                        let split = |t: &str| t.trim_start_matches("Entry { ").trim_end_matches(" }").split(", ").map(str::to_owned).collect::<Vec<_>>();
                        let mut d: Vec<String> = split(&g).iter().zip(split(&w)).filter(|(a, b)| *a != b).map(|(a, _)| a.split(':').next().unwrap_or("").to_owned()).collect();
                        d.dedup();
                        if r != recs {
                            d.push("recs".to_owned());
                        }
                        if d.is_empty() { continue } else { d.join("+") }
                    }
                };
                *why.entry(k.clone()).or_default() += 1;
                example.entry(k).or_insert_with(|| {
                    let at = 64 * blocks * c;
                    let lo = at.saturating_sub(80);
                    let hi = (at + 40).min(f.len());
                    let (lo, hi) = ((lo..=at).find(|&i| f.is_char_boundary(i)).unwrap_or(at), (hi..=f.len()).find(|&i| f.is_char_boundary(i)).unwrap_or(f.len()));
                    format!("chunk {c} of {blocks}-block chunks at byte {at}: {:?} | {:?}", &f[lo..at.min(f.len()).max(lo)], &f[at.min(f.len()).max(lo)..hi])
                });
            }
        }
    }
    for (k, v) in &why {
        eprintln!("{v:6} {k}: e.g. {}", &example[k][..example[k].len().min(220)]);
    }
}
