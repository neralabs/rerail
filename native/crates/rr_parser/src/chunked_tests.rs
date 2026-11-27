//! Tier 1 from the middle (`rerail_rr_aot::lex_chunked`): a file lexed in chunks, each from every entry at once (code,
//! or inside each region), the entries joined by a prefix sum, then each chunk from its entry, is the serial lexer's
//! (`lex`), record for record.

use rerail_rr_aot::{Entry, IN_FLIGHT, Lexed, apply, chunk_ends, compose, entries, entries_waits, lex, lex_chunked, padded};

/// The cases: the samples, and regions cut by chunk edges at every offset.
fn files() -> Vec<String> {
    let mut out: Vec<String> = crate::SAMPLES.iter().map(|s| (*s).to_owned()).collect();
    for at in 0..140 {
        let pad = " ".repeat(at);
        out.push(format!("{pad}a(\"x\\\\\\\"y {pad}\") :- b(1).\n"));
        out.push(format!("a(1).{pad}// a comment {pad}\nb(2).\n"));
        out.push(format!("a(1). /* a {pad} block */ b(\"{pad}\\\\\").\n"));
        out.push(format!("a(`{pad}x`, ``y{pad}``, ```z{pad}```).\n"));
        out.push(format!("{pad}a(b) :- c(\"/* {pad}\"), d(`\"`).\n"));
        out.push(format!("a(\"{pad}"));
        out.push(format!("a(1). /* {pad}"));
    }
    out
}

/// `lex_chunked`'s records and the serial lexer's: the same refusal, and where it lexes the file, the same records and
/// comments. The chunks entered again where pass 1's lane was wrong.
fn same(text: &str, blocks: usize) -> usize {
    let buf = padded(text.as_bytes());
    let want = lex(&buf, text.len());
    let (got, again) = lex_chunked(&buf, text.len(), blocks);
    let recs = |l: &Lexed| (l.recs.iter().map(|r| (r.start, r.last, r.code)).collect::<Vec<_>>(), l.extras.iter().map(|r| (r.start, r.last, r.code)).collect::<Vec<_>>());
    assert_eq!(got.refused, want.refused, "chunks of {blocks} blocks: refused, in\n{text}");
    if !want.refused {
        assert!(recs(&got) == recs(&want), "chunks of {blocks} blocks: the records, in\n{text}");
    }
    again
}

#[test]
fn chunked_is_the_serial_lexer() {
    let (files, mut again) = (files(), 0);
    for f in &files {
        for blocks in [1, 2, 3, 16] {
            again += same(f, blocks);
        }
    }
    eprintln!("{} files, each in chunks of 1, 2, 3 and 16 blocks: the serial lexer's records; {again} chunks entered again", files.len());
}

/// Summaries compose in any tree: each chunk's entry by the prefix sum is the one composed left to right, and
/// `(A·B)·C = A·(B·C)` as functions of the entry (at each place a lane of `A` starts, before `A` and past `C`).
#[test]
fn summaries_compose_in_any_order() {
    for f in files().iter().filter(|f| f.len() > 256) {
        let buf = padded(f.as_bytes());
        let n = f.len();
        let ends: Vec<_> = (0..n.div_ceil(64)).map(|q| chunk_ends(&buf, n, q, q + 1)).collect();
        let scanned = entries(&ends);
        let mut left = ends[0];
        for c in 1..ends.len() {
            assert_eq!(scanned[c], apply(&left, &Entry::START, 0), "chunk {c}");
            left = compose(&left, &ends[c]);
        }
        for w in ends.windows(3) {
            let (a, b, c) = (&w[0], &w[1], &w[2]);
            let (l, r) = (compose(&compose(a, b), c), compose(a, &compose(b, c)));
            let ats = a.lanes().iter().map(|l| l.at0).chain([a.start.saturating_sub(1), c.end]);
            for at in ats {
                let e = Entry { at, ..Entry::START };
                // (None: no lane met it, one went past `L`; pass 3 waits for the entry before.)
                if let (Some(x), Some(y)) = (apply(&l, &e, 0), apply(&r, &e, 0))
                    && x != y
                {
                    let show = |s: &rerail_rr_aot::Summary| s.lanes().iter().map(|l| (l.at0, l.exit.at, l.refused, l.moved)).collect::<Vec<_>>();
                    panic!("(A·B)·C is not A·(B·C) at {at}: {:?} {:?}\nA {} {} {:?}\nB {} {} {:?}\nC {} {} {:?}\nl {:?}\nr {:?}", apply(&l, &e, 0).map(|x| x.0.at), apply(&r, &e, 0).map(|x| x.0.at), a.start, a.end, show(a), b.start, b.end, show(b), c.start, c.end, show(c), show(&l), show(&r));
                }
            }
        }
    }
}

#[test]
#[ignore = "the corpus: run by name"]
fn corpus_chunked_is_the_serial_lexer() {
    let files = crate::tests::corpus();
    let (mut again, mut waits) = ([0usize; 4], [0usize; 4]);
    for f in &files {
        let buf = padded(f.as_bytes());
        for (i, blocks) in [1, 2, 16, 256].into_iter().enumerate() {
            again[i] += same(f, blocks);
            let ends: Vec<_> = (0..f.len().div_ceil(64)).step_by(blocks).map(|q| chunk_ends(&buf, f.len(), q, q + blocks)).collect();
            waits[i] += entries_waits(&ends).1;
        }
    }
    eprintln!("{} files: the serial lexer's records in chunks of 1, 2, 16 and 256 blocks; chunks entered again: {again:?}; pass 3's waits ({IN_FLIGHT} in flight): {waits:?}", files.len());
}

#[test]
#[ignore = "a report: the serial lexer against the chunked one over the corpus"]
fn report_chunked_speed() {
    let bufs: Vec<(Vec<u8>, usize)> = crate::tests::corpus().iter().map(|f| (padded(f.as_bytes()), f.len())).collect();
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
    let serial = time(&|b, n| lex(b, n).recs.len());
    eprintln!("{} files, {:.1} MB; serial {:.2} GB/s", bufs.len(), bytes as f64 / 1e6, bytes as f64 / serial / 1e9);
    for blocks in [16, 64, 256, 1 << 30] {
        let chunks = |n: usize| (0..n.div_ceil(64)).step_by(blocks);
        let ends = time(&|b, n| chunks(n).map(|q| chunk_ends(b, n, q, q + blocks).lanes()[0].exit.at).sum());
        let all: Vec<Vec<Option<(Entry, usize, bool)>>> = bufs.iter().map(|(b, n)| entries(&chunks(*n).map(|q| chunk_ends(b, *n, q, q + blocks)).collect::<Vec<_>>())).collect();
        let scan = time(&|b, n| entries(&chunks(n).map(|q| chunk_ends(b, n, q, q + blocks)).collect::<Vec<_>>()).len()) - ends;
        let from = {
            let mut best = f64::MAX;
            for _ in 0..5 {
                let t = std::time::Instant::now();
                for ((b, n), e) in bufs.iter().zip(&all) {
                    let (mut recs, mut extras) = (Vec::with_capacity(n + 1), Vec::new());
                    for (c, q) in chunks(*n).enumerate() {
                        _ = rerail_rr_aot::lex_chunk(b, *n, e[c].map_or(64 * q, |x| x.0.at), q + blocks, &mut recs, &mut extras);
                    }
                    std::hint::black_box(recs.len());
                }
                best = best.min(t.elapsed().as_secs_f64());
            }
            best
        };
        let all_three = time(&|b, n| lex_chunked(b, n, blocks).0.recs.len());
        eprintln!("  chunks of {blocks} blocks ({} KB), {} lanes: all three passes {:.2}x the serial time; pass 1 {:.2}x, pass 3 {:.3}x, pass 2 from the true entries {:.2}x", blocks * 64 / 1024, rerail_rr_aot::LANES, all_three / serial, ends / serial, scan.max(0.0) / serial, from / serial);
    }
}

#[test]
#[ignore = "a profile's load: pass 1 alone over the corpus, in chunks of RERAIL_PROFILE_BLOCKS blocks (16), for about 4 seconds"]
fn profile_pass_1() {
    let bufs: Vec<(Vec<u8>, usize)> = crate::tests::corpus().iter().map(|f| (padded(f.as_bytes()), f.len())).collect();
    // Chunks of `RERAIL_PROFILE_BLOCKS` blocks (16), else one a file.
    let blocks: usize = std::env::var("RERAIL_PROFILE_BLOCKS").ok().and_then(|v| v.parse().ok()).unwrap_or(16);
    let t = std::time::Instant::now();
    let mut c = 0usize;
    while t.elapsed().as_secs() < 4 {
        for (b, n) in &bufs {
            for q in (0..n.div_ceil(64)).step_by(blocks) {
                c += chunk_ends(b, *n, q, q + blocks).lanes()[0].exit.at;
            }
        }
    }
    std::hint::black_box(c);
}
