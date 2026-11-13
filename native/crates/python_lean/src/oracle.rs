//! The oracle: the lean parser's tapes against tree-sitter's, file by file. The engine writes
//! tree-sitter's (`python_tape_expect`, to `RERAIL_PY_OUT`); this compares, and reports the files
//! that differ (the first differing line, with the lines around it) and the ones refused.

use crate::{tape, tape_text};

#[test]
#[ignore = "needs RERAIL_PY_OUT, the engine's expected tapes (python_tape_expect)"]
fn oracle() {
    let dir = std::env::var("RERAIL_PY_OUT").expect("set RERAIL_PY_OUT");
    let show: usize = std::env::var("RERAIL_PY_SHOW").ok().and_then(|s| s.parse().ok()).unwrap_or(12);
    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    paths.sort();
    let (mut same, mut differ, mut refused, mut shown) = (0usize, 0usize, 0usize, 0usize);
    let (mut errors_refused, mut errors_accepted) = (0usize, 0usize);
    let mut refusals: Vec<String> = Vec::new();
    for p in &paths {
        let expected = std::fs::read_to_string(p).unwrap();
        let (head, want) = expected.split_once('\n').unwrap();
        // A cut text: its file's first `cut` bytes; `ERROR` where tree-sitter's tree has an error.
        let (path, cut) = match head.split_once('\t') {
            Some((p, c)) => (p, c.parse::<usize>().ok()),
            None => (head, None),
        };
        let Ok(mut src) = std::fs::read(path) else { continue };
        if let Some(cut) = cut {
            src.truncate(cut);
        }
        let path = &format!("{path}{}", cut.map_or(String::new(), |c| format!(" (cut at {c})")));
        if want == "ERROR\n" {
            match tape(&src) {
                Err(_) => errors_refused += 1,
                Ok(_) => {
                    errors_accepted += 1;
                    if shown < show {
                        shown += 1;
                        let at = src.len().saturating_sub(80);
                        eprintln!("--- {path}: tree-sitter's tree has an error, the lean parser read it\n  end: {:?}", String::from_utf8_lossy(&src[at..]));
                    }
                }
            }
            continue;
        }
        match tape(&src) {
            Err(r) => {
                refused += 1;
                let at = match r {
                    crate::Refusal::Lex(a) | crate::Refusal::Parse(a) => a as usize,
                    crate::Refusal::TooLarge => 0,
                };
                let line_start = src[..at.min(src.len())].iter().rposition(|&c| c == b'\n').map_or(0, |i| i + 1);
                let line_end = src[at.min(src.len())..].iter().position(|&c| c == b'\n').map_or(src.len(), |i| at + i);
                refusals.push(format!("{r:?} {path}: {:?}", String::from_utf8_lossy(&src[line_start..line_end.max(line_start)])));
            }
            Ok(t) => {
                let got = tape_text(&t);
                if got == want {
                    same += 1;
                    continue;
                }
                differ += 1;
                if shown < show {
                    shown += 1;
                    let (g, w): (Vec<_>, Vec<_>) = (got.lines().collect(), want.lines().collect());
                    let k = g.iter().zip(&w).position(|(a, b)| a != b).unwrap_or(g.len().min(w.len()));
                    let from = k.saturating_sub(4);
                    eprintln!("--- {path} (line {k})");
                    for (i, line) in w.iter().enumerate().skip(from).take(8) {
                        eprintln!("  want {}{line}", if i == k { ">" } else { " " });
                    }
                    for (i, line) in g.iter().enumerate().skip(from).take(8) {
                        eprintln!("  got  {}{line}", if i == k { ">" } else { " " });
                    }
                    let s = w.get(k).and_then(|l| l.rsplit_once('[')).and_then(|(_, r)| r.split_once("..")).and_then(|(a, _)| a.parse::<usize>().ok()).unwrap_or(0);
                    let e = (s + 80).min(src.len());
                    eprintln!("  src: {:?}", String::from_utf8_lossy(&src[s.min(src.len())..e]));
                }
            }
        }
    }
    for r in refusals.iter().take(show) {
        eprintln!("refused: {r}");
    }
    eprintln!("{same} same, {differ} differ, {refused} refused, of {}", paths.len());
    eprintln!("texts with errors: {errors_refused} refused, {errors_accepted} read (wrongly)");
}
