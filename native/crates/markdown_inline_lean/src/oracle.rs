//! The oracle: the lean parser's tapes against tree-sitter's, text by text. The inline texts are
//! the block parser's (`rerail-markdown-lean`'s `inline_texts`); the engine writes tree-sitter's
//! tapes of them (`python_tape_expect` with `RERAIL_PY_DIALECT=markdown-inline`, to
//! `RERAIL_MDI_OUT`). This compares, and reports the texts that differ and those refused.

use crate::tape_text;
use std::io::Write;

#[test]
#[ignore = "needs RERAIL_MDI_OUT, the engine's expected tapes (python_tape_expect)"]
fn oracle() {
    let dir = std::env::var("RERAIL_MDI_OUT").expect("set RERAIL_MDI_OUT");
    let show: usize = std::env::var("RERAIL_MDI_SHOW").ok().and_then(|s| s.parse().ok()).unwrap_or(8);
    // With `RERAIL_MDI_DIFFS`, a line for each text that differs or is refused.
    let mut diffs = std::env::var("RERAIL_MDI_DIFFS").ok().filter(|s| !s.is_empty()).map(|p| std::fs::File::create(p).unwrap());
    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.is_file()).collect();
    paths.sort();
    let (mut same, mut differ, mut refused, mut shown) = (0usize, 0usize, 0usize, 0usize);
    for p in &paths {
        let expected = std::fs::read_to_string(p).unwrap();
        let (path, want) = expected.split_once('\n').unwrap();
        let Ok(src) = std::fs::read(path) else { continue };
        let name = p.file_name().unwrap().to_string_lossy();
        let flat = |a: usize, b: usize| String::from_utf8_lossy(&src[a.min(src.len())..b.min(src.len())]).replace(['\n', '\r', '\t'], "~");
        match crate::tape(&src) {
            Err(r) => {
                refused += 1;
                let at = match r {
                    crate::Refusal::Lex(a) | crate::Refusal::Parse(a) => a as usize,
                    crate::Refusal::TooLarge => 0,
                };
                if let Some(out) = &mut diffs {
                    writeln!(out, "R\t{name}\t{r:?}\t{}\t>>{}", flat(at.saturating_sub(40), at), flat(at, at + 20)).unwrap();
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
                    writeln!(out, "D\t{name}\t{}\t{}\t{}", w.get(k).unwrap_or(&"(end)").trim(), g.get(k).unwrap_or(&"(end)").trim(), flat(0, 200)).unwrap();
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
                    eprintln!("  src: {:?}", flat(0, 300));
                }
            }
        }
    }
    eprintln!("{same} same, {differ} differ, {refused} refused, of {}", paths.len());
}
