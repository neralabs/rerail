// F-string interpolations without an automaton: depth by prefix sum, roles from the previous
// round's context, repeated to a fixed point. Checked against tree-sitter's interpolation and
// format_specifier spans.
use tree_sitter::{Node, Parser, Language};
use std::collections::BTreeMap;

fn escaped(src: &[u8]) -> Vec<bool> {
    let mut e = vec![false; src.len() + 1]; let mut run = 0usize;
    for (i, &c) in src.iter().enumerate() { if c == b'\\' { run += 1 } else { if run % 2 == 1 { e[i] = true } run = 0 } }
    e
}

#[derive(Clone, PartialEq, Default)]
struct Ctx { d: Vec<i32>, inner: Vec<u8>, cmt: Vec<bool>, spec: Vec<bool> } // context *before* each byte

#[derive(Debug)] enum Out { Spans(Vec<(usize, usize)>, Vec<(usize, usize)>, usize), Unsupported(&'static str), Unsettled }

fn analyse(src: &[u8], esc: &[bool], a: usize, b: usize, triple_outer: bool, raw: bool, kmax: usize, naive: usize) -> Out {
    let n = b - a; let at = |i: usize| src[a + i];
    let isq = |i: usize| (at(i) == b'\'' || at(i) == b'"') && !esc[a + i];
    let mut ctx = Ctx { d: vec![0; n + 1], inner: vec![0; n + 1], cmt: vec![false; n + 1], spec: vec![false; n + 1] };
    for round in 0..kmax {
        // Roles, each decided from the previous round's context before the byte (Jacobi step).
        // delta: depth change; open_i/close_i: inner strings; cmt_s; spec_s; interp open/close.
        let mut delta = vec![0i32; n]; let mut open_inner = vec![0u8; n]; let mut close_inner = vec![false; n];
        let mut cmt_s = vec![false; n]; let mut spec_s = vec![false; n]; let mut err: Option<&'static str> = None;
        // brace escapes: within a run of one brace char whose bytes all have depth 0 and are literal,
        // pairs aligned from the run's first such byte are escapes; an odd last '{' opens.
        let lit = |c: &Ctx, i: usize| c.d[i] == 0;
        // Inner strings and comments are scans within the round: the reflection scan over the
        // quotes that the previous round puts at depth >= 1, and a fill from '#' to the newline.
        let (mut cur_inner, mut cur_cmt) = (0u8, false);
        let mut i = 0;
        while i < n {
            let c = at(i);
            if ctx.d[i] == 0 { cur_inner = 0; if !triple_outer { cur_cmt = false } }
            let (d, inn, cm, sp) = (ctx.d[i], cur_inner, cur_cmt, ctx.spec[i]);
            if cm && c == b'\n' { cur_cmt = false }
            // A run of one brace character is decided from the context at its first byte only, so
            // every role depends on bytes strictly before it.
            // \N{NAME} in literal text of a non-raw f-string is an escape, through its '}'.
            if c == b'{' && d == 0 && !raw && i >= 1 && at(i - 1) == b'N' && esc[a + i - 1] {
                let mut j = i; while j < n && at(j) != b'}' { j += 1 } i = j + 1; continue
            }
            if (c == b'{' || c == b'}') && !cm && inn == 0 {
                let mut j = i; while j < n && at(j) == c { j += 1 }
                let len = j - i;
                if round < naive {
                    // First guess, independent of depth: pairs are escapes, an odd last one counts.
                    if len % 2 == 1 { delta[j - 1] = if c == b'{' { 1 } else { -1 } }
                } else if c == b'{' {
                    if d == 0 { if len % 2 == 1 { delta[j - 1] = 1 } }            // {{ escapes; an odd last one opens
                    else { for k in i..j { delta[k] = 1 } }                         // code, or a nested field in a spec
                } else {
                    let closes = (d as usize).min(len);                             // close down to depth 0 ...
                    for k in i..i + closes { delta[k] = -1 }
                    if (len - closes) % 2 == 1 { err.get_or_insert("single '}' in literal text"); } // ... then }} escapes
                }
                let _ = lit; i = j; continue
            }
            if cm { i += 1; continue }                                  // comment until newline (handled by the fill)
            if inn != 0 { if isq(i) && (if at(i) == b'\'' { 1 } else { 2 }) == inn { close_inner[i] = true; cur_inner = 0 } i += 1; continue }
            if d >= 1 {
                if sp && d == 1 {                                         // format-spec text: only braces matter
                    if c == b'{' { delta[i] = 1 } else if c == b'}' { delta[i] = -1 }
                } else {
                    match c {
                        b'{' | b'(' | b'[' => delta[i] = 1,
                        b'}' | b')' | b']' => delta[i] = -1,
                        b'\'' | b'"' if !esc[a + i] => {
                            if i + 2 < n && at(i + 1) == c && at(i + 2) == c { err.get_or_insert("triple-quoted string in an interpolation"); }
                            open_inner[i] = if c == b'\'' { 1 } else { 2 }; cur_inner = open_inner[i] }
                        b'#' => { cmt_s[i] = true; cur_cmt = true }
                        b':' if d == 1 && !sp => spec_s[i] = true,
                        b'\n' if !triple_outer => { err.get_or_insert("newline in a one-line f-string's interpolation"); }
                        _ => {}
                    }
                }
            }
            i += 1;
        }
        if let Some(e) = err { if round + 1 == kmax { return Out::Unsupported(e) } }
        // New context: prefix sum for depth; fills for inner strings, comments and spec text.
        let mut nc = Ctx { d: vec![0; n + 1], inner: vec![0; n + 1], cmt: vec![false; n + 1], spec: vec![false; n + 1] };
        let (mut d, mut inn, mut cm, mut sp) = (0i32, 0u8, false, false);
        for i in 0..n {
            nc.d[i] = d; nc.inner[i] = inn; nc.cmt[i] = cm; nc.spec[i] = sp;
            d += delta[i];
            if open_inner[i] != 0 { inn = open_inner[i] } if close_inner[i] { inn = 0 }
            if cmt_s[i] { cm = true } if cm && at(i) == b'\n' { cm = false }
            if spec_s[i] { sp = true } if d == 0 { sp = false }
            if d < 0 { d = 0 }
        }
        nc.d[n] = d; nc.inner[n] = inn; nc.cmt[n] = cm; nc.spec[n] = sp;
        if nc == ctx {
            if err.is_some() { return Out::Unsupported(err.unwrap()) }
            if d != 0 || inn != 0 { return Out::Unsupported("unbalanced") }
            // spans: interpolation from a '{' taking depth 0 -> 1 to the '}' taking it back; and,
            // inside a spec, nested fields at depth 1 -> 2 and back. Specs: colon to before the '}'.
            let mut interp = vec![]; let mut fexpr = vec![]; let mut specs = vec![]; let mut stack: Vec<(usize, i32)> = vec![]; let mut spec_open: Option<usize> = None;
            for i in 0..n {
                let (d0, d1) = (ctx.d[i], ctx.d[i + 1]);
                if at(i) == b'{' && d1 == d0 + 1 && (d0 == 0 || (ctx.spec[i] && d0 == 1)) { stack.push((i, d0)) }
                if at(i) == b'}' && d1 == d0 - 1 { if let Some(&(s, sd)) = stack.last() { if sd == d1 { stack.pop(); if sd == 0 { interp.push((a + s, a + i + 1)) } else { fexpr.push((a + s, a + i + 1)) } } } }
                if spec_s[i] && !ctx.spec[i] { spec_open = Some(i) }
                if let Some(s) = spec_open { if at(i) == b'}' && d0 == 1 && d1 == 0 { specs.push((a + s, a + i)); spec_open = None } }
            }
            interp.extend(fexpr.iter().map(|&(x, y)| (x | (1 << 62), y))); interp.sort(); specs.sort();
            return Out::Spans(interp, specs, round + 1)
        }
        ctx = nc;
    }
    Out::Unsettled
}

fn truth(n: Node, out_i: &mut Vec<(usize, usize)>, out_s: &mut Vec<(usize, usize)>, top: bool) {
    if !top && n.kind() == "string" { return }
    if n.kind() == "interpolation" { out_i.push((n.start_byte(), n.end_byte())) }
    if n.kind() == "format_expression" { out_i.push((n.start_byte() | (1 << 62), n.end_byte())) }
    if n.kind() == "format_specifier" { out_s.push((n.start_byte(), n.end_byte())) }
    let mut c = n.walk(); for ch in n.children(&mut c) { truth(ch, out_i, out_s, false) }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let lang: Language = tree_sitter_python::LANGUAGE.into(); let mut p = Parser::new(); p.set_language(&lang).unwrap();
    let (mut fstrings, mut agree, mut differ, mut unsup, mut unsettled) = (0u64, 0u64, 0u64, 0u64, 0u64);
    let mut rounds: BTreeMap<usize, u64> = BTreeMap::new(); let mut why: BTreeMap<&str, u64> = BTreeMap::new(); let mut shown = 0;
    let mut body_bytes = 0u64; let mut total_bytes = 0u64; let mut files_bad: std::collections::BTreeSet<String> = Default::default(); let mut files = 0u64;
    for path in std::fs::read_to_string(&a[1]).unwrap().lines() {
        let Ok(src) = std::fs::read(path) else { continue }; let t = p.parse(&src, None).unwrap(); if t.root_node().has_error() { continue }
        files += 1; total_bytes += src.len() as u64; let esc = escaped(&src);
        let mut stack = vec![t.root_node()];
        while let Some(n) = stack.pop() {
            let mut c = n.walk(); for ch in n.children(&mut c) { stack.push(ch) }
            if n.kind() != "string" { continue }
            let (Some(st), Some(en)) = (n.child(0), n.child(n.child_count() - 1)) else { continue };
            let pre = &src[st.byte_range()]; if !pre.iter().any(|&b| b == b'f' || b == b'F') { continue }
            fstrings += 1; body_bytes += (en.start_byte() - st.end_byte()) as u64;
            let triple = pre.len() >= 3 && pre[pre.len() - 3..].iter().all(|&b| b == pre[pre.len() - 1]);
            let (mut ti, mut ts) = (vec![], vec![]); truth(n, &mut ti, &mut ts, true); ti.sort(); ts.sort();
            let raw = pre.iter().any(|&b| b == b'r' || b == b'R');
            let naive: usize = std::env::var("NAIVE").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
            match analyse(&src, &esc, st.end_byte(), en.start_byte(), triple, raw, 40, naive) {
                Out::Spans(oi, os, r) => { *rounds.entry(r).or_default() += 1; if r >= 9 && std::env::var("SLOW").is_ok() { eprintln!("SLOW {r}: {:?}", String::from_utf8_lossy(&src[n.byte_range()][..n.byte_range().len().min(220)])) }
                    if oi == ti && os == ts { agree += 1 } else { differ += 1; files_bad.insert(path.to_string());
                        if shown < 8 { eprintln!("DIFFER {:?}\n  ts interp {:?} spec {:?}\n  us interp {:?} spec {:?}", String::from_utf8_lossy(&src[n.byte_range()]), ti, ts, oi, os); shown += 1 } } }
                Out::Unsupported(w) => { if w == "unbalanced" && shown < 12 { eprintln!("UNBALANCED {:?}", String::from_utf8_lossy(&src[n.byte_range()][..n.byte_range().len().min(150)])); shown += 1 } unsup += 1; *why.entry(w).or_default() += 1; files_bad.insert(path.to_string()); }
                Out::Unsettled => { unsettled += 1; files_bad.insert(path.to_string()); }
            }
        }
    }
    println!("f-string body bytes {body_bytes} of {total_bytes} ({:.2}%)", 100.0 * body_bytes as f64 / total_bytes as f64);
    println!("f-strings {fstrings}: agree {agree}, differ {differ}, unsupported {unsup} {:?}, unsettled {unsettled}; rounds {:?}; files affected {} of {files}", why, rounds, files_bad.len());
}
