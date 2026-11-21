//! The oracle: the lean parser's tapes against tree-sitter's, file by file. The engine writes
//! tree-sitter's (`python_tape_expect` with `RERAIL_PY_DIALECT=vue`, to `RERAIL_VUE_OUT`); this
//! compares, and reports the files that differ (the first differing line, with the lines around
//! it) and those refused. With `RERAIL_PY_CUTS`, the engine also writes each file cut short.

use crate::tape_text;
use std::io::Write;

#[test]
#[ignore = "needs RERAIL_VUE_OUT, the engine's expected tapes (python_tape_expect)"]
fn oracle() {
    let dir = std::env::var("RERAIL_VUE_OUT").expect("set RERAIL_VUE_OUT");
    let show: usize = std::env::var("RERAIL_VUE_SHOW").ok().and_then(|s| s.parse().ok()).unwrap_or(8);
    // With `RERAIL_VUE_DIFFS`, a line for each file that differs or is refused.
    let mut diffs = std::env::var("RERAIL_VUE_DIFFS").ok().filter(|s| !s.is_empty()).map(|p| std::fs::File::create(p).unwrap());
    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.is_file()).collect();
    paths.sort();
    let (mut same, mut differ, mut refused, mut shown) = (0usize, 0usize, 0usize, 0usize);
    let (mut errors, mut errors_read) = (0usize, 0usize);
    for p in &paths {
        let expected = std::fs::read_to_string(p).unwrap();
        let (head, want) = expected.split_once('\n').unwrap();
        // A cut file (`RERAIL_PY_CUTS`): its first `cut` bytes, as a file is while it is written.
        let (path, cut) = match head.split_once('\t') {
            Some((p, c)) => (p, c.parse::<usize>().ok()),
            None => (head, None),
        };
        let Ok(mut src) = std::fs::read(path) else { continue };
        if let Some(cut) = cut {
            src.truncate(cut);
        }
        let name = p.file_name().unwrap().to_string_lossy();
        let flat = |a: usize, b: usize| String::from_utf8_lossy(&src[a.min(src.len())..b.min(src.len())]).replace(['\n', '\r', '\t'], "~");
        // A file where tree-sitter's tree has an error: the parser is to refuse it.
        if let Some(_recovered) = want.strip_prefix("ERROR\n") {
            errors += 1;
            if crate::tape(&src).is_ok() {
                errors_read += 1;
                eprintln!("--- {name} {path}: tree-sitter's tree has an error; the lean parser read it");
            }
            continue;
        }
        match crate::tape(&src) {
            Err(r) => {
                refused += 1;
                let at = match r {
                    crate::Refusal::Lex(a) | crate::Refusal::Parse(a) => a as usize,
                    crate::Refusal::TooLarge => 0,
                };
                if let Some(out) = &mut diffs {
                    writeln!(out, "R\t{name}\t{r:?}\t{}\t>>{}", flat(at.saturating_sub(40), at), flat(at, at + 30)).unwrap();
                }
            }
            Ok(t) => {
                let got = tape_text(&t);
                if got == want {
                    same += 1;
                    continue;
                }
                differ += 1;
                let (g, w): (Vec<_>, Vec<_>) = (got.lines().collect(), want.lines().collect());
                let k = g.iter().zip(&w).position(|(a, b)| a != b).unwrap_or(g.len().min(w.len()));
                if let Some(out) = &mut diffs {
                    writeln!(out, "D\t{name}\t{}\t{}", w.get(k).unwrap_or(&"(end)").trim(), g.get(k).unwrap_or(&"(end)").trim()).unwrap();
                }
                if shown < show {
                    shown += 1;
                    eprintln!("--- {name} {path} (line {k})");
                    for (i, line) in w.iter().enumerate().skip(k.saturating_sub(4)).take(8) {
                        eprintln!("  want {}{line}", if i == k { ">" } else { " " });
                    }
                    for (i, line) in g.iter().enumerate().skip(k.saturating_sub(4)).take(8) {
                        eprintln!("  got  {}{line}", if i == k { ">" } else { " " });
                    }
                }
            }
        }
    }
    eprintln!("{same} same, {differ} differ, {refused} refused, of {}; {errors} with errors ({errors_read} read)", paths.len());
    assert_eq!(errors_read, 0, "files with errors the parser read");
}
