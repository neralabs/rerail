//! The oracle: the lean parser's tapes against tree-sitter's, file by file. The engine writes
//! tree-sitter's (`python_tape_expect`, to `RERAIL_PY_OUT`); this compares, and reports the files
//! that differ (the first differing line, with the lines around it) and the ones refused.

use crate::tape_text;

#[test]
#[ignore = "needs RERAIL_PY_OUT, the engine's expected tapes (python_tape_expect)"]
fn oracle() {
    let dir = std::env::var("RERAIL_PY_OUT").expect("set RERAIL_PY_OUT");
    // The product's parse (error texts refused), or with `RERAIL_PY_RECOVER=1` decision 0169's recovery.
    let tape = if std::env::var("RERAIL_PY_RECOVER").is_ok_and(|v| v == "1") { crate::tape_recovering } else { crate::tape };
    let show: usize = std::env::var("RERAIL_PY_SHOW").ok().and_then(|s| s.parse().ok()).unwrap_or(12);
    let mut paths: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    paths.sort();
    let (mut same, mut differ, mut refused, mut shown) = (0usize, 0usize, 0usize, 0usize);
    let (mut errors_refused, mut errors_same, mut errors_differ) = (0usize, 0usize, 0usize);
    let mut refusals: Vec<String> = Vec::new();
    // With `RERAIL_PY_DIFFS`, each recovered text that differs: its first differing lines, a line each.
    let mut diffs = std::env::var("RERAIL_PY_DIFFS").ok().filter(|s| !s.is_empty()).map(|p| std::fs::File::create(p).unwrap());
    // With `RERAIL_PY_ONLY`, the oracle files whose names are in it (comma-separated) alone.
    let only: Option<Vec<String>> = std::env::var("RERAIL_PY_ONLY").ok().filter(|s| !s.is_empty()).map(|s| s.split(',').map(str::to_owned).collect());
    for p in &paths {
        if only.as_ref().is_some_and(|o| !o.iter().any(|n| p.file_name().is_some_and(|f| f.to_string_lossy() == *n))) {
            continue;
        }
        let expected = std::fs::read_to_string(p).unwrap();
        let (head, want) = expected.split_once('\n').unwrap();
        // A cut text: its file's first `cut` bytes; `ERROR` where tree-sitter's tree has an error.
        // An edited text: `@<cut>-<line end>`, the line cut short in the middle of the file.
        let (path, cut, edit) = match head.split_once('\t') {
            Some((p, c)) => match c.strip_prefix('@').and_then(|e| e.split_once('-')) {
                Some((a, b)) => (p, None, a.parse::<usize>().ok().zip(b.parse::<usize>().ok())),
                None => (p, c.parse::<usize>().ok(), None),
            },
            None => (head, None, None),
        };
        let Ok(mut src) = std::fs::read(path) else { continue };
        if let Some(cut) = cut {
            src.truncate(cut);
        }
        if let Some((a, b)) = edit {
            src = [&src[..a], &src[b..]].concat();
        }
        let path = &format!("{path}{}{}", cut.map_or(String::new(), |c| format!(" (cut at {c})")), edit.map_or(String::new(), |(a, b)| format!(" (line cut at {a}, to {b})")));
        // A text where tree-sitter's tree has an error: its recovered tape follows the `ERROR` line.
        let (want, error) = match want.strip_prefix("ERROR\n") {
            Some(w) => (w, true),
            // tree-sitter's `has_error` can miss an ERROR its tree holds: the tape says so too.
            None => (want, want.contains(" flags=2 ") || want.contains(" flags=3 ")),
        };
        if error {
            match tape(&src) {
                Err(r) => {
                    errors_refused += 1;
                    if let Some(out) = &mut diffs {
                        use std::io::Write;
                        let at = match r {
                            crate::Refusal::Lex(a) | crate::Refusal::Parse(a) => a as usize,
                            crate::Refusal::TooLarge => 0,
                        };
                        let near = String::from_utf8_lossy(&src[at.saturating_sub(30).min(src.len())..(at + 10).min(src.len())]).replace(['\n', '\t'], "~");
                        writeln!(out, "{}\trefused {:?}\t{} from the end: {near}", p.file_name().unwrap().to_string_lossy(), std::mem::discriminant(&r), src.len().saturating_sub(at)).unwrap();
                    }
                }
                Ok(t) if tape_text(&t) == want => errors_same += 1,
                Ok(t) => {
                    errors_differ += 1;
                    let got = tape_text(&t);
                    if let Some(out) = &mut diffs {
                        use std::io::Write;
                        let (g, w): (Vec<_>, Vec<_>) = (got.lines().collect(), want.lines().collect());
                        let k = g.iter().zip(&w).position(|(a, b)| a != b).unwrap_or(g.len().min(w.len()));
                        writeln!(out, "{}\t{}\t{}", p.file_name().unwrap().to_string_lossy(), w.get(k).unwrap_or(&"(end)").trim(), g.get(k).unwrap_or(&"(end)").trim()).unwrap();
                    }
                    if shown < show {
                        shown += 1;
                        show_diff(&format!("{path} [error text]"), &got, want, &src);
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
                    show_diff(path, &got, want, &src);
                }
            }
        }
    }
    for r in refusals.iter().take(show) {
        eprintln!("refused: {r}");
    }
    eprintln!("{same} same, {differ} differ, {refused} refused, of {}", paths.len());
    eprintln!("texts with errors: {errors_same} recovered as tree-sitter does, {errors_differ} differ, {errors_refused} refused");
}

/// The first line where `got` and `want` differ, with the lines around it, and the text there.
fn show_diff(path: &str, got: &str, want: &str, src: &[u8]) {
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
