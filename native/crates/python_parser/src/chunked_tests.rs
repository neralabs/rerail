//! Tier 1 from the middle (`rerail_python_aot::lex_chunked`): a file lexed in chunks, each from every class of entries
//! (its code entry, and the nest below it it reads: the code's brackets, the string reader's interpolations and their
//! brackets), the entries joined by a prefix sum of the chunks' summaries, then each chunk from its entry, is the serial
//! lexer's (`lex`), record for record.

use rerail_python_aot::{Entry, IN_FLIGHT, Lexed, Summary, apply, compose, entries, entries_waits, lex, lex_chunked, padded, serial_entries, summaries};

/// An f-string with interpolations nested `k` deep, each one's code a call with an f-string in it.
fn nested(k: usize) -> String {
    let mut s = String::from("x");
    for i in 0..k.min(3) {
        s = format!("f{i}(f\"a{i}{{{s}}}b{i}\")");
    }
    s
}

/// An interpolation whose code's strings and brackets run across chunks, then code in brackets: the look-back composes
/// lanes that read the interpolation's kind in parts (posthog's `test_event.py`).
const SPLIT_READS: &str = r#"response = self.client.get(
    f"/api/projects/{self.team.id}/events/values/?key=random_prop&properties_filter_prop={json.dumps(['123', 'true', 'value1'])}"
).json()
assert {r["name"] for r in response["results"]} == {"asdf", "qwerty"}

# Test with non-string property values
"#;

/// The cases: the kinds of strings, interpolations, brackets and comments, and each cut by chunk edges at every offset.
fn files() -> Vec<String> {
    let mut out: Vec<String> = vec![
        "def main():\n    print(f\"hi {name}!\")\n".to_owned(),
        "s = f\"a{ g(f'b{x}c') }d\"\nt = 1\n".to_owned(),
        "r = rf'raw {x} \\d+ {y!r:>10}'\nq = b'bytes'\n".to_owned(),
        "m = f\"\"\"triple {a}\n{b:{w}}\n\"\"\"\n".to_owned(),
        "c = (1,\n     2,  # a comment\n     [3, {4: 5}])\n".to_owned(),
        "x = {'k': [f\"{v['a']}\" for v in vs]}\n".to_owned(),
        "def f(a,\n      b):\n    return (a\n            + b)\n".to_owned(),
    ];
    for k in 1..=3 {
        out.push(format!("v = {}\n", nested(k)));
    }
    for at in 0..140 {
        let pad = " ".repeat(at);
        out.push(format!("{pad}s = f\"a{{ g(f'b{{x{pad}}}c') }}d\"\n"));
        out.push(format!("a = 1{pad}# a comment {pad}\nb = 2\n"));
        out.push(format!("a = (1,{pad}\n     2) + f'{pad}{{b!r}}'\n"));
        out.push(format!("r = '''{pad}{{x}}{pad}''' + rf\"{{y}}\\{pad}\"\n"));
        out.push(format!("{pad}def g():\n    return f\"{{ [f'{pad}{{it}}' for it in xs] }}\"\n"));
        out.push(format!("v = {pad}{}\n", nested(2)));
        out.push(format!("x = f\"{{a:{pad}>10}}\" + f\"{{b!s}}\"\n"));
        out.push(format!("        return [Fact(label=\"Data\", value=\"Counts of{pad} the data\", href=url(obj))]\n"));
        out.push(format!("x = 'plain {pad} text' + \"more{pad}\" + b'{pad}'\n"));
        out.push(format!("{pad}y = f(value=\"Counts of the data\", n=f'{{a}}')\n"));
        // (Every offset in a block: the case is long.)
        if at < 64 {
            out.push(format!("{pad}{SPLIT_READS}"));
        }
        out.push(format!("s = 'a{pad}"));
    }
    out
}

fn lexed(text: &str) -> Lexed {
    lex(&padded(text.as_bytes()), text.len())
}

/// `lex_chunked`'s records and the serial lexer's: the same refusal, and where it lexes the file, the same records and
/// comments. The chunks entered again where pass 3's entry was not the one the chunk before left.
fn same(text: &str, blocks: usize) -> usize {
    let want = lexed(text);
    let buf = padded(text.as_bytes());
    let (got, again) = lex_chunked(&buf, text.len(), blocks);
    let recs = |l: &Lexed| (l.recs.iter().map(|r| (r.start, r.last, r.code)).collect::<Vec<_>>(), l.extras.iter().map(|r| (r.start, r.last, r.code)).collect::<Vec<_>>());
    assert_eq!(got.refused, want.refused, "chunks of {blocks} blocks: refused, in\n{text}");
    if !want.refused {
        assert!(recs(&got) == recs(&want), "chunks of {blocks} blocks: the records, in\n{text}");
    }
    again
}

/// The lanes a file's chunks hold: the most in one chunk, the chunks past `L`, and those pass 3 waits for.
fn lanes(text: &str, blocks: usize) -> (usize, usize, usize) {
    let s = summaries(&padded(text.as_bytes()), text.len(), blocks);
    (s.iter().map(|s| s.lanes().len()).max().unwrap_or(0), s.iter().filter(|s| s.over).count(), entries_waits(&s).1)
}

#[test]
fn chunked_is_the_serial_lexer() {
    let (files, mut again, mut chunks, mut most, mut over, mut waits) = (files(), 0, 0, 0, 0, 0);
    for f in &files {
        for blocks in [1, 2, 3, 16] {
            again += same(f, blocks);
            chunks += f.len().div_ceil(64 * blocks).max(1);
            let (m, o, w) = lanes(f, blocks);
            most = most.max(m);
            over += o;
            waits += w;
        }
    }
    eprintln!("{} files, each in chunks of 1, 2, 3 and 16 blocks ({chunks} chunks): the serial lexer's records; {again} chunks entered again; at most {most} lanes a chunk, {over} chunks past L; pass 3's waits {waits}", files.len());
}

/// Summaries compose in any tree: each chunk's entry by the prefix sum is the one composed left to right, and
/// `(A·B)·C = A·(B·C)` at each place a lane of `A` starts.
#[test]
fn summaries_compose_in_any_order() {
    for f in files().iter().filter(|f| f.len() > 128) {
        let ends: Vec<Summary> = summaries(&padded(f.as_bytes()), f.len(), 1);
        let scanned = entries(&ends);
        let mut left = ends[0];
        for c in 1..ends.len() {
            if let (Some(x), Some(y)) = (scanned[c], apply(&left, &Entry::START, 0)) {
                assert_eq!(x, y, "chunk {c}, in\n{f}");
            }
            left = compose(&left, &ends[c]);
        }
        for w in ends.windows(3) {
            let (a, b, c) = (&w[0], &w[1], &w[2]);
            let (l, r) = (compose(&compose(a, b), c), compose(a, &compose(b, c)));
            let mut ats: Vec<usize> = a.lanes().iter().map(|l| l.at0).collect();
            ats.extend([a.start.saturating_sub(1), c.end]);
            for &at in &ats {
                let mut e = Entry::START;
                e.at = at;
                if let (Some(x), Some(y)) = (apply(&l, &e, 7), apply(&r, &e, 7))
                    && !(x.2 && y.2)
                {
                    assert_eq!(x, y, "(A·B)·C is not A·(B·C) at {at}, in\n{f}");
                }
            }
        }
    }
}

/// The corpus (`RERAIL_PYTHON_CORPUS`, a folder of `.py` files, read to any depth; its first `RERAIL_PYTHON_CORPUS_FILES`
/// in path order, else all): the serial lexer's records in chunks of 1, 2, 16 and 256 blocks, and the chunks entered
/// again.
#[test]
#[ignore = "reads RERAIL_PYTHON_CORPUS, a folder of .py files"]
fn corpus_chunked_is_the_serial_lexer() {
    let dir = std::env::var("RERAIL_PYTHON_CORPUS").expect("set RERAIL_PYTHON_CORPUS");
    let mut paths = Vec::new();
    let mut todo = vec![std::path::PathBuf::from(dir)];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                todo.push(p);
            } else if p.extension().is_some_and(|x| x == "py") {
                paths.push(p);
            }
        }
    }
    paths.sort();
    let k: usize = std::env::var("RERAIL_PYTHON_CORPUS_FILES").ok().and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let files: Vec<String> = paths.iter().take(k).filter_map(|p| std::fs::read_to_string(p).ok()).collect();
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

/// The same over the corpus's first `RERAIL_REPORT_FILES` files (20).
#[test]
#[ignore = "reads RERAIL_PYTHON_CORPUS, a folder of .py files"]
fn report_the_corpus_entries_pass_3_misses() {
    let dir = std::env::var("RERAIL_PYTHON_CORPUS").expect("set RERAIL_PYTHON_CORPUS");
    let k: usize = std::env::var("RERAIL_REPORT_FILES").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
    let mut paths = Vec::new();
    let mut todo = vec![std::path::PathBuf::from(dir)];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                todo.push(p);
            } else if p.extension().is_some_and(|x| x == "py") {
                paths.push(p);
            }
        }
    }
    paths.sort();
    report_misses(&paths.iter().take(k).filter_map(|p| std::fs::read_to_string(p).ok()).collect::<Vec<_>>());
}

fn report_misses(files: &[String]) {
    let mut why: std::collections::BTreeMap<String, usize> = Default::default();
    let mut example: std::collections::BTreeMap<String, String> = Default::default();
    for f in files {
        let buf = padded(f.as_bytes());
        for blocks in [1, 2] {
            let sums = summaries(&buf, f.len(), blocks);
            let pred = entries(&sums);
            for (c, (want, recs)) in serial_entries(&buf, f.len(), blocks).into_iter().enumerate() {
                let k = match pred[c] {
                    None => format!("no lane{}", if sums[..c].iter().any(|x| x.over) { " (a chunk before past L)" } else { "" }),
                    // (A refused file's entries past the refusal are no one's.)
                    Some((_, _, true)) if lexed(f).refused => continue,
                    Some((_, _, true)) => "refused, the serial lexer not".to_owned(),
                    Some((got, r, _)) => {
                        let (g, w) = (format!("{got:?}"), format!("{want:?}"));
                        let split = |t: &str| t.trim_start_matches("Entry { ").trim_end_matches(" }").split(", ").map(str::to_owned).collect::<Vec<_>>();
                        let mut d: Vec<String> = split(&g).iter().zip(split(&w)).filter(|(a, b)| *a != b).map(|(a, _)| a.split(':').next().unwrap_or("").to_owned()).collect();
                        d.dedup();
                        if r != recs {
                            d.push("recs".to_owned());
                        }
                        if d.is_empty() {
                            continue;
                        }
                        let k = d.join("+");
                        k
                    }
                };
                *why.entry(k.clone()).or_default() += 1;
                example.entry(k).or_insert_with(|| {
                    let at = (64 * blocks * c).min(f.len());
                    format!("chunk {c} of {blocks}-block chunks at byte {at}: {:?} | {:?}", &f[at.saturating_sub(60)..at], &f[at..(at + 30).min(f.len())])
                });
            }
        }
    }
    for (k, v) in &why {
        eprintln!("{v:6} {k}: e.g. {}", &example[k][..example[k].len().min(220)]);
    }
}
