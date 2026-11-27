//! Stage 1 made as the token loop comes to its blocks (decision 0703): the two-pass form's stage 1, a pass over
//! the blocks with carries, run a step at a time from its state (`Stage1Run`), and the token loop's reads of its masks
//! after the blocks they read are made, as the lean lexer makes its blocks (`Blocks::of`) as its loop needs them.
//!
//! A pass over the tier 1 the two-pass form writes: its `stage1` (the carries before its block loop, the loop, the
//! masks it returns), its `lex` (the masks it takes, the test build's walk of the regions, the token loop, what
//! follows). Each block of the loop makes stage 1 through the next block; a scan of a mask forward (`first_bit`,
//! `run_last`) makes it as it comes to blocks not made; after the loop, stage 1 is made to the file's end, for its
//! escapes and the passes that read its masks there. The test build's walk reads a stage 1 made whole, its own.
//! `None` where the text is not of that form.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;

/// Each name of `names` in `text` that is a whole name (not a part of one, nor a field after `.`), with `pre` before it.
fn prefixed(text: &str, names: &HashSet<&str>, pre: &str) -> String {
    let b = text.as_bytes();
    let id = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    let mut i = 0;
    while i < b.len() {
        if (b[i].is_ascii_alphabetic() || b[i] == b'_') && (i == 0 || !(id(b[i - 1]) || b[i - 1] == b'.')) {
            let mut j = i;
            while j < b.len() && id(b[j]) {
                j += 1;
            }
            let w = &text[i..j];
            if names.contains(w) {
                out.push_str(pre);
            }
            out.push_str(w);
            i = j;
        } else {
            let c = text[i..].chars().next().expect("a char");
            out.push(c);
            i += c.len_utf8();
        }
    }
    out
}

pub(super) fn interleave(text: &str) -> Option<String> {
    let l: Vec<&str> = text.split('\n').collect();
    let find = |from: usize, p: &dyn Fn(&str) -> bool| (from..l.len()).find(|&i| p(l[i]));
    let h = find(0, &|x| x == "unsafe fn stage1(src: &[u8]) -> Stage1 {")?;
    let lp = find(h, &|x| x.starts_with("    for t in 0..nw + ") && x.ends_with(" {"))?;
    let lag: usize = l[lp].trim_start_matches("    for t in 0..nw + ").trim_end_matches(" {").parse().ok()?;
    let le = find(lp + 1, &|x| x == "    }")?;
    let sl = find(le, &|x| x == "    Stage1 {")?;
    let se = find(sl, &|x| x == "    }")?;
    if l.get(se + 1) != Some(&"}") {
        return None;
    }
    // The masks' struct, its fields.
    let sd = find(0, &|x| x == "struct Stage1 {")?;
    let sde = find(sd, &|x| x == "}")?;
    let mut fields: Vec<(&str, &str)> = Vec::new();
    for x in &l[sd + 1..sde] {
        let t = x.trim();
        if let Some((f, ty)) = t.strip_suffix(',').and_then(|t| t.split_once(": ")) {
            fields.push((f, ty));
        }
    }
    let vecs: HashSet<&str> = fields.iter().filter(|f| f.1 == "Vec<u64>").map(|f| f.0).collect();
    // The returned literal: each field's value.
    let mut lit: Vec<(&str, &str)> = Vec::new();
    for x in &l[sl + 1..se] {
        let t = x.trim().strip_suffix(',')?;
        lit.push(t.split_once(": ").unwrap_or((t, t)));
    }
    // The carries: the locals before the loop that are no mask it returns.
    let (mut ring, mut carries, mut keep): (Option<&str>, Vec<(&str, &str, String)>, Vec<&str>) = (None, Vec::new(), Vec::new());
    for x in &l[h + 1..lp] {
        if let Some(r) = x.strip_prefix("    let mut ").and_then(|r| r.strip_suffix(';'))
            && let Some((name, init)) = r.split_once(" = ")
        {
            if vecs.contains(name) && init == "vec![0u64; nw]" {
                continue;
            }
            let (ty, init) = match init {
                "0u64" => ("u64", init.to_owned()),
                "false" => ("bool", init.to_owned()),
                "0usize" => ("usize", init.to_owned()),
                "[0u64; RING]" => ("[u64; RING_1]", "[0u64; RING_1]".to_owned()),
                _ => return None,
            };
            carries.push((name, ty, init));
        } else if let Some(r) = x.strip_prefix("    const RING: usize = ").and_then(|r| r.strip_suffix(';')) {
            ring = Some(r);
        } else {
            keep.push(x);
        }
    }
    let cn: HashSet<&str> = carries.iter().map(|c| c.0).collect();
    let both = |t: &str| prefixed(&prefixed(t, &cn, "st."), &vecs, "o.");
    let body = both(&l[lp + 1..le].join("\n"));
    let finals = both(&l[le + 1..sl].join("\n"));
    let mut set = String::new();
    for (f, ty) in &fields {
        if *ty != "Vec<u64>" {
            let v = lit.iter().find(|x| x.0 == *f)?.1;
            let _ = writeln!(set, "        o.{f} = {};", both(v));
        }
    }
    let mut head = String::new();
    let _ = writeln!(head, "/// Stage 1's state between its steps, its carries (decision 0703): the token loop makes its blocks as it comes to them.");
    let _ = writeln!(head, "const RING_1: usize = {};", ring.unwrap_or("1"));
    let _ = writeln!(head, "struct Stage1Run {{\n    t: usize,\n    done: bool,");
    for (c, t, _) in &carries {
        let _ = writeln!(head, "    {c}: {t},");
    }
    head.push_str("}\n\nimpl Stage1Run {\n    fn new() -> Self {\n        Stage1Run { t: 0, done: false");
    for (c, _, i) in &carries {
        let _ = write!(head, ", {c}: {i}");
    }
    head.push_str(" }\n    }\n}\n\nimpl Stage1 {\n    /// The masks, no block made yet.\n    fn new(nw: usize) -> Self {\n        Stage1 { ");
    let inits: Vec<String> = fields.iter().map(|(f, t)| if *t == "Vec<u64>" { format!("{f}: vec![0u64; nw]") } else { format!("{f}: Default::default()") }).collect();
    head.push_str(&inits.join(", "));
    head.push_str(" }\n    }\n}\n");
    let hdr = l[h].replace("(src: &[u8]) -> Stage1 {", "(src: &[u8], st: &mut Stage1Run, o: &mut Stage1, upto: usize) {");
    let mut stage = format!("/// Stage 1 to step `upto` from where `st` left it: block `t - {lag}`'s masks made at step `t`.\n{hdr}\n");
    for x in &keep {
        stage.push_str(x);
        stage.push('\n');
    }
    let _ = write!(stage, "    const RING: usize = RING_1;\n    let end = upto.min(nw + {lag});\n    while st.t < end {{\n        let t = st.t;\n        st.t += 1;\n{body}\n    }}\n    if st.t == nw + {lag} && !st.done {{\n        st.done = true;\n");
    if !finals.trim().is_empty() {
        stage.push_str(&finals);
        stage.push('\n');
    }
    let _ = write!(stage, "{set}    }}\n}}");
    // Its doc and attributes stay before it; the state's items before them.
    let mut hd = h;
    while hd > 0 && (l[hd - 1].starts_with("///") || l[hd - 1].starts_with("#[")) {
        hd -= 1;
    }
    let mut out: Vec<String> = l[..hd].iter().map(|x| (*x).to_owned()).collect();
    out.push(head);
    out.extend(l[hd..h].iter().map(|x| (*x).to_owned()));
    out.push(stage);
    out.extend(l[se + 2..].iter().map(|x| (*x).to_owned()));
    let text = out.join("\n");
    let text = text.replacen("fn stage1_best(src: &[u8]) -> Stage1 = stage1, on stage1_on;", "fn stage1_best(src: &[u8], st: &mut Stage1Run, o: &mut Stage1, upto: usize) -> () = stage1, on stage1_on;", 1);
    // `lex`: the masks it takes, then the walk, the loop and what follows.
    let l: Vec<&str> = text.split('\n').collect();
    let find = |from: usize, p: &dyn Fn(&str) -> bool| (from..l.len()).find(|&i| p(l[i]));
    let lx = find(0, &|x| x.starts_with("pub fn lex(src: &[u8]) -> Lexed {"))?;
    let dl = find(lx, &|x| x.starts_with("    let Stage1 { ") && x.ends_with(" } = stage1_best(src);"))?;
    let taken = l[dl].trim_start_matches("    let Stage1 { ").trim_end_matches(" } = stage1_best(src);");
    let mut masks: HashSet<&str> = HashSet::new();
    let mut lets: Vec<(&str, &str)> = Vec::new();
    for x in taken.split(", ") {
        let x = x.trim_start_matches("mut ");
        match x.split_once(": ") {
            Some((f, v)) if v != "_" => lets.push((f, v)),
            Some(_) => {}
            None if vecs.contains(x) => {
                masks.insert(x);
            }
            None => lets.push((x, x)),
        }
    }
    let lend = find(lx, &|x| x == "}")?;
    let lpi = find(dl, &|x| x.starts_with("    'tokens: while b < nw {"))?;
    let lpe = find(lpi + 1, &|x| x == "    }")?;
    let walk = (dl + 1..lpi).find(|&i| l[i].starts_with("    let (") && l[i].ends_with(") = {") && l[i - 1].trim() == "#[cfg(feature = \"test-hooks\")]");
    let seg = |a: usize, b: usize| l[a..b].join("\n");
    let scans = |t: &str| -> String {
        let mut t = t.to_owned();
        for f in &masks {
            for (scan, lazy) in [("first_bit", "s1_first_bit"), ("run_last", "s1_run_last")] {
                let pat = format!("{scan}(&{f}, ");
                let mut o = String::new();
                let mut rest = t.as_str();
                while let Some(p) = rest.find(&pat) {
                    let ok = rest[..p].chars().next_back().is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'));
                    o.push_str(&rest[..p]);
                    if ok {
                        let _ = write!(o, "{lazy}(src, &mut st1, &mut s1, |o: &Stage1| &o.{f}[..], ");
                    } else {
                        o.push_str(&pat);
                    }
                    rest = &rest[p + pat.len()..];
                }
                o.push_str(rest);
                t = o;
            }
        }
        // A scan of the bytes (`bit_reads`) ends where stage 1 may not be made: made through its end, for the reads
        // there.
        for scan in ["wfirst_bit", "wfirst_in", "wfirst_clear", "wrun_last", "wrun_once_last", "bfirst_bit", "bfirst_in", "bfirst_clear", "brun_last", "brun_once_last"] {
            let pat = format!("{scan}(n, ");
            let mut o = String::new();
            let mut rest = t.as_str();
            while let Some(p) = rest.find(&pat) {
                let ok = rest[..p].chars().next_back().is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'));
                o.push_str(&rest[..p]);
                let inner = &rest[p + scan.len() + 1..];
                match super::bytes::closing(inner) {
                    Some(c) if ok => {
                        let _ = write!(o, "s1_past(src, &mut st1, &mut s1, {scan}({}))", &inner[..c]);
                        rest = &inner[c + 1..];
                    }
                    _ => {
                        o.push_str(&pat);
                        rest = &rest[p + pat.len()..];
                    }
                }
            }
            o.push_str(rest);
            t = o;
        }
        t
    };
    let product = |t: &str| prefixed(&scans(t), &masks, "s1.");
    let (pre, walk_text, mid) = match walk {
        Some(w) => {
            let we = find(w, &|x| x == "    };")?;
            let wt = prefixed(&seg(w - 1, we + 1), &masks, "s1w.");
            let wt = wt.replacen("#[cfg(feature = \"test-hooks\")]\n", "#[cfg(feature = \"test-hooks\")]\n    let s1w = {\n        let mut w = Stage1::new(nw);\n        stage1_best(src, &mut Stage1Run::new(), &mut w, usize::MAX);\n        w\n    };\n    #[cfg(feature = \"test-hooks\")]\n", 1);
            (product(&seg(dl + 1, w - 1)), wt, product(&seg(we + 1, lpi)))
        }
        None => (product(&seg(dl + 1, lpi)), String::new(), String::new()),
    };
    let mut lp_text = product(&seg(lpi, lpe + 1));
    let at = "    'tokens: while b < nw {\n        let base = b * 64;\n";
    if !lp_text.starts_with(at) {
        return None;
    }
    lp_text = lp_text.replacen(at, &format!("{at}        // Stage 1 through the next block (the last a token of a few bytes reads), 16 more at a time.\n        if st1.t < b + 2 + {lag} {{\n            stage1_best(src, &mut st1, &mut s1, b + 2 + {lag} + 16);\n        }}\n"), 1);
    let mut fin = "    // Stage 1 to the file's end: its escapes, and the masks the passes after the loop read.\n    stage1_best(src, &mut st1, &mut s1, usize::MAX);\n".to_owned();
    let mut pre = pre;
    for (f, v) in &lets {
        let _ = writeln!(fin, "    let {v} = s1.{f};");
        // Stage 1's escapes were the refusal's start: now they join it after the loop.
        let start = format!("let mut refused = {v};");
        if pre.contains(&start) {
            pre = pre.replacen(&start, "let mut refused = false;", 1);
            let _ = writeln!(fin, "    refused |= {v};");
        }
    }
    let post = product(&seg(lpe + 1, lend));
    let decl = "    // Stage 1, made as the token loop comes to its blocks (decision 0703): its state and its masks.\n    let mut st1 = Stage1Run::new();\n    let mut s1 = Stage1::new(nw);";
    let mut out = format!("{}\n{decl}\n{pre}\n", seg(0, dl));
    if !walk_text.is_empty() {
        let _ = write!(out, "{walk_text}\n{mid}\n");
    }
    let _ = write!(out, "{lp_text}\n{fin}{post}\n{}", seg(lend, l.len()));
    let helpers = format!(
        "\n/// `r`, stage 1 made through the block after the byte it names (a scan of the bytes' end).\n#[inline(always)]\nfn s1_past<T: Copy + Into<Option<usize>>>(src: &[u8], st: &mut Stage1Run, s1: &mut Stage1, r: T) -> T {{\n    if let Some(p) = r.into() {{\n        let to = p / 64 + 2 + {lag};\n        if st.t < to {{\n            stage1_best(src, st, s1, to.max(st.t + 16));\n        }}\n    }}\n    r\n}}\n\n/// `first_bit` of a mask of stage 1's, its blocks made as the scan comes to them (decision 0703).\n#[inline(always)]\nfn s1_first_bit(src: &[u8], st: &mut Stage1Run, s1: &mut Stage1, m: impl Fn(&Stage1) -> &[u64], from: usize) -> Option<usize> {{\n    let nw = src.len().div_ceil(64);\n    loop {{\n        let made = st.t.saturating_sub({lag}).min(nw);\n        if from / 64 + 1 < made || made == nw {{\n            match first_bit(&m(s1)[..made], from) {{\n                Some(p) if p / 64 + 1 < made || made == nw => return Some(p),\n                None if made == nw => return None,\n                _ => {{}}\n            }}\n        }}\n        let to = (st.t * 2).max(st.t + 16);\n        stage1_best(src, st, s1, to);\n    }}\n}}\n\n/// `run_last` of a mask of stage 1's, its blocks made as the run comes to them (decision 0703).\n#[inline(always)]\nfn s1_run_last(src: &[u8], st: &mut Stage1Run, s1: &mut Stage1, m: impl Fn(&Stage1) -> &[u64], s: usize) -> usize {{\n    let nw = src.len().div_ceil(64);\n    loop {{\n        let made = st.t.saturating_sub({lag}).min(nw);\n        if s / 64 + 1 < made || made == nw {{\n            let l = run_last(&m(s1)[..made], s);\n            if l / 64 + 1 < made || made == nw {{\n                return l;\n            }}\n        }}\n        let to = (st.t * 2).max(st.t + 16);\n        stage1_best(src, st, s1, to);\n    }}\n}}\n"
    );
    let at = "\n/// `src`'s records.\npub fn lex(";
    if !out.contains(at) {
        return None;
    }
    let out = test_only(&out.replacen(at, &format!("{helpers}{at}"), 1), &fields);
    Some(together(&out).unwrap_or(out))
}

/// Stage 1's masks kept together a block (decision 0703), as the lean lexer keeps a block's masks in one (`Vec<Masks>`):
/// one `m: Vec<[u64; K]>`, each mask its column (`Col`), read by the helpers through `Words`. A mask tier 2 takes, the
/// loop writes, or only a test build keeps stays a vector of its own. `None` where the text is not of that form.
fn together(text: &str) -> Option<String> {
    let whole = |t: &str, pat: &str| t.match_indices(pat).filter(|(i, _)| !t[..*i].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.') && !t[i + pat.len()..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')).count();
    let sd = text.find("struct Stage1 {\n")?;
    let se = sd + text[sd..].find("\n}\n")?;
    let def = &text[sd..se];
    let lines: Vec<&str> = def.lines().collect();
    let mut cols: Vec<String> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let Some(f) = l.trim().strip_suffix(": Vec<u64>,") else { continue };
        if i > 0 && lines[i - 1].trim().starts_with("#[cfg(") {
            continue;
        }
        // Moved (to tier 2's masks, out of the struct) or written: its own vector.
        let by_value = |p: &str| text.match_indices(p).any(|(i, _)| !text[..i].ends_with('&') && !text[..i].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.'));
        let moved = by_value(&format!("s1.{f},")) || by_value(&format!("s1.{f};")) || by_value(&format!("s1.{f}\n")) || text.contains(&format!("&mut s1.{f}")) || text.contains(&format!("&mut o.{f}"));
        // A write but the block's store (`o.f[q] = `).
        let pat = format!("o.{f}[");
        let written = text.match_indices(&pat).any(|(i, _)| {
            let rest = &text[i + pat.len()..];
            let Some(close) = rest.find(']') else { return true };
            let after = &rest[close + 1..];
            let w = (after.starts_with(" = ") || after.starts_with(" |= ") || after.starts_with(" &= ")) && !text[..i].ends_with('.');
            w && &rest[..close] != "q"
        });
        if moved || written {
            continue;
        }
        cols.push(f.to_owned());
    }
    if cols.len() < 2 {
        return None;
    }
    // A scan that makes blocks of a mask that is a vector of its own: the vector's form (`_v`), as it was.
    let mut text = text.to_owned();
    let lazy_v: Vec<String> = text
        .match_indices("|o: &Stage1| &o.")
        .map(|(i, p)| text[i + p.len()..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect::<String>())
        .filter(|f| !cols.contains(f))
        .collect();
    if !lazy_v.is_empty() {
        for f in &lazy_v {
            for h in ["s1_first_bit", "s1_run_last"] {
                text = text.replace(&format!("{h}(src, &mut st1, &mut s1, |o: &Stage1| &o.{f}[..], "), &format!("{h}_v(src, &mut st1, &mut s1, |o: &Stage1| &o.{f}[..], "));
            }
        }
        for h in ["s1_first_bit", "s1_run_last"] {
            let start = text.find(&format!("\n/// `{}` of a mask of stage 1's", &h[3..]))?;
            let end = start + 1 + text[start + 1..].find("\n}\n")? + 3;
            let v = text[start..end].replacen(&format!("fn {h}("), &format!("fn {h}_v("), 1);
            text.insert_str(end, &v);
        }
    }
    let text = text.as_str();
    let k = cols.len();
    let idx = |f: &str| format!("S1_{}", f.to_ascii_uppercase());
    let mut out = text.to_owned();
    // The struct and its making.
    for f in &cols {
        out = out.replacen(&format!("\n    {f}: Vec<u64>,\n"), "\n", 1);
        out = out.replacen(&format!(" {f}: vec![0u64; nw],"), "", 1);
    }
    out = out.replacen("struct Stage1 {\n", &format!("struct Stage1 {{\n    /// The masks the product reads, a block's together (`S1_*` their columns).\n    m: Vec<[u64; {k}]>,\n"), 1);
    out = out.replacen("        Stage1 { ", &format!("        Stage1 {{ m: vec![[0u64; {k}]; nw], "), 1);
    // The stores, the reads, the scans that make blocks.
    let mut text2 = String::with_capacity(out.len());
    let b = out.as_bytes();
    let id = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut i = 0;
    let set: HashSet<&str> = cols.iter().map(String::as_str).collect();
    while i < b.len() {
        // `PFX.NAME` with PFX one of `s1`, `s1w`, `o`.
        if (b[i] == b's' || b[i] == b'o') && (i == 0 || !(id(b[i - 1]) || b[i - 1] == b'.')) {
            let mut j = i;
            while j < b.len() && id(b[j]) {
                j += 1;
            }
            let pfx = &out[i..j];
            if matches!(pfx, "s1" | "s1w" | "o") && b.get(j) == Some(&b'.') {
                let mut e = j + 1;
                while e < b.len() && id(b[e]) {
                    e += 1;
                }
                let f = &out[j + 1..e];
                if set.contains(f) {
                    let rest = &out[e..];
                    if pfx == "o" && rest.starts_with("[q] = ") {
                        let _ = write!(text2, "o.m[q][{}] = ", idx(f));
                        i = e + "[q] = ".len();
                        continue;
                    }
                    if pfx == "o" && rest.starts_with("[..]") && text2.ends_with("|o: &Stage1| &") {
                        text2.truncate(text2.len() - "|o: &Stage1| &".len());
                        text2.push_str(&idx(f));
                        i = e + "[..]".len();
                        continue;
                    }
                    // A column borrows the blocks alone: the loop may write a vector of its own beside it.
                    let _ = write!(text2, "Col {{ m: &{pfx}.m, k: {} }}", idx(f));
                    i = e;
                    continue;
                }
            }
            text2.push_str(pfx);
            i = j;
            continue;
        }
        let c = out[i..].chars().next()?;
        text2.push(c);
        i += c.len_utf8();
    }
    let mut out = text2;
    // The scans that make blocks: by column.
    for h in ["s1_first_bit", "s1_run_last"] {
        let Some(start) = out.find(&format!("\nfn {h}(")) else { continue };
        let Some(len) = out[start + 1..].find("\n}\n") else { continue };
        let body = out[start..start + 1 + len].replace("m: impl Fn(&Stage1) -> &[u64], ", "k: usize, ").replace("first_bit(&m(s1)[..made], from)", "first_bit(&Col { m: &s1.m[..made], k }, from)").replace("run_last(&m(s1)[..made], s)", "run_last(&Col { m: &s1.m[..made], k }, s)");
        out.replace_range(start..start + 1 + len, &body);
    }
    // The helpers: of any words, each mask a parameter of its own type.
    let mut o = String::new();
    let mut at = 0;
    for line in out.split_inclusive('\n') {
        let here = at;
        at += line.len();
        let _ = here;
        let generic = line.starts_with("fn ") && line.contains(": &[u64]") && line.find('(').is_some_and(|o| !line[..o].contains('<'));
        if !generic {
            o.push_str(line);
            continue;
        }
        let (Some(open), Some(close)) = (line.find('('), line.find(')')) else {
            o.push_str(line);
            continue;
        };
        let mut params = line[open..close].to_owned();
        let mut ws = Vec::new();
        while let Some(at) = params.find(": &[u64]") {
            let w = format!("W{}", ws.len());
            params.replace_range(at..at + ": &[u64]".len(), &format!(": &{w}"));
            ws.push(w);
        }
        let bounds: Vec<String> = ws.iter().map(|w| format!("{w}: Words + core::ops::Index<usize, Output = u64> + ?Sized")).collect();
        let _ = write!(o, "{}<{}>{params}{}", &line[..open], bounds.join(", "), &line[close..]);
    }
    // A generic helper's zips over its masks: of their words (`Words::iter`).
    let mut out = String::with_capacity(o.len());
    let mut params: Vec<String> = Vec::new();
    for line in o.split_inclusive('\n') {
        if line.starts_with("fn ") && line.contains(": Words + core::ops::Index") {
            params = line.match_indices(": &W").filter_map(|(i, _)| line[..i].rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).next().map(str::to_owned)).collect();
        } else if line == "}\n" {
            params.clear();
        }
        let mut l = line.to_owned();
        for p in &params {
            l = l.replace(&format!(".zip({p})"), &format!(".zip({p}.iter())"));
        }
        out.push_str(&l);
    }
    let mut consts = String::new();
    for (j, f) in cols.iter().enumerate() {
        let _ = writeln!(consts, "const {}: usize = {j};", idx(f));
    }
    let items = format!(
        "\n/// Each mask's column in stage 1's blocks (`Stage1::m`).\n{consts}\n/// A mask's words in stage 1's blocks: its column.\n#[derive(Clone, Copy)]\nstruct Col<'a> {{\n    m: &'a [[u64; {k}]],\n    k: usize,\n}}\n\nimpl Stage1 {{\n    /// Mask `k`'s words.\n    #[inline(always)]\n    fn col(&self, k: usize) -> Col<'_> {{\n        Col {{ m: &self.m, k }}\n    }}\n}}\n\nimpl core::ops::Index<usize> for Col<'_> {{\n    type Output = u64;\n    #[inline(always)]\n    fn index(&self, q: usize) -> &u64 {{\n        &self.m[q][self.k]\n    }}\n}}\n\n/// A mask's words, a vector's or a column's.\ntrait Words {{\n    fn get(&self, q: usize) -> Option<&u64>;\n    fn len(&self) -> usize;\n    fn iter(&self) -> impl Iterator<Item = &u64> + '_;\n}}\n\nimpl Words for [u64] {{\n    #[inline(always)]\n    fn get(&self, q: usize) -> Option<&u64> {{\n        <[u64]>::get(self, q)\n    }}\n    #[inline(always)]\n    fn len(&self) -> usize {{\n        <[u64]>::len(self)\n    }}\n    #[inline(always)]\n    fn iter(&self) -> impl Iterator<Item = &u64> + '_ {{\n        <[u64]>::iter(self)\n    }}\n}}\n\nimpl Words for Vec<u64> {{\n    #[inline(always)]\n    fn get(&self, q: usize) -> Option<&u64> {{\n        <[u64]>::get(self, q)\n    }}\n    #[inline(always)]\n    fn len(&self) -> usize {{\n        <[u64]>::len(self)\n    }}\n    #[inline(always)]\n    fn iter(&self) -> impl Iterator<Item = &u64> + '_ {{\n        <[u64]>::iter(self)\n    }}\n}}\n\nimpl Words for Col<'_> {{\n    #[inline(always)]\n    fn get(&self, q: usize) -> Option<&u64> {{\n        self.m.get(q).map(|w| &w[self.k])\n    }}\n    #[inline(always)]\n    fn len(&self) -> usize {{\n        self.m.len()\n    }}\n    #[inline(always)]\n    fn iter(&self) -> impl Iterator<Item = &u64> + '_ {{\n        let k = self.k;\n        self.m.iter().map(move |w| &w[k])\n    }}\n}}\n"
    );
    let at = out.find("\nconst RING_1: usize")?;
    let line_end = at + 1 + out[at + 1..].find('\n')?;
    out.insert_str(line_end + 1, &items);
    Some(out)
}

/// The masks only the test build's walk of the regions reads (its own stage 1, `s1w`): stage 1 keeps them in a test
/// build alone, and the product's makes no vector of them.
fn test_only(text: &str, fields: &[(&str, &str)]) -> String {
    let Some(lx) = text.find("\npub fn lex(src: &[u8]) -> Lexed {") else { return text.to_owned() };
    let lex = &text[lx..];
    // Its code, not its comments (which name masks).
    let lex: String = lex[..lex.find("\n}\n").unwrap_or(lex.len())].split('\n').map(|l| format!("{}\n", code_of(l))).collect();
    let lex = lex.as_str();
    let read = |f: &str| {
        let whole = |pat: &str| lex.match_indices(pat).any(|(i, _)| !lex[i + pat.len()..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_'));
        whole(&format!("s1.{f}")) || whole(&format!("&o.{f}[..]"))
    };
    let mut out = text.to_owned();
    for (f, ty) in fields {
        if *ty != "Vec<u64>" || read(f) {
            continue;
        }
        // Its store: of its value at the block, or of its carry (a mask made a block late).
        let store = [format!("            if q < nw {{\n                o.{f}[q] = v_{f};\n            }}\n"), format!("            if q < nw {{\n                o.{f}[q] = st.v_{f};\n            }}\n")].into_iter().find(|st| out.matches(st.as_str()).count() == 1).unwrap_or_default();
        let field = format!("\n    {f}: Vec<u64>,\n");
        let init = format!(" {f}: vec![0u64; nw],");
        if store.is_empty() || out.matches(&field).count() != 1 || out.matches(&init).count() != 1 {
            continue;
        }
        out = out.replacen(&store, &format!("            #[cfg(feature = \"test-hooks\")]\n{store}"), 1);
        out = out.replacen(&field, &format!("\n    #[cfg(feature = \"test-hooks\")]\n    {f}: Vec<u64>,\n"), 1);
        out = out.replacen(&init, &format!(" #[cfg(feature = \"test-hooks\")] {f}: vec![0u64; nw],"), 1);
    }
    out
}

/// Stage 1's masks read from the bytes where `lex` reads them (decision 0703), as the lean lexer reads its bytes: a
/// one-byte test as a test of the bytes there (`c == b'/'`), a search or a count over bytes 64 at a time from its
/// first (`Bytes`' windows, the one pass's). A mask that reads no region, each of whose reads in `lex` (not the test
/// build's walk of the regions) has that form. Stage 1 then keeps none of it in the product build (`test_only`), and
/// `lex` reads the bytes on `S`'s backend (`generic_lex`). `exprs`: each mask's expression.
pub(super) fn bit_reads(text: &str, stmts: &[::bitlex::syntax::Stmt], exprs: &[(String, ::bitlex::syntax::Expr)]) -> String {
    let Some(lx) = text.find("\npub fn lex(src: &[u8]) -> Lexed {") else { return text.to_owned() };
    let Some(len) = text[lx..].find("\n}\n") else { return text.to_owned() };
    // The product's sections: `lex`, and tier 2's reads of the region tokens (`span_reads`), all of them product lines.
    let mut sections = vec![(lx, lx + len, true)];
    if let Some(a) = text.find(super::T2_BEGIN)
        && let Some(b) = text[a..].find(super::T2_END)
    {
        sections.push((a, a + b, false));
    }
    sections.sort_unstable();
    // The product's lines: not the destructuring of stage 1's masks, nor the walk.
    let mut lines: Vec<&str> = Vec::new();
    let mut product: Vec<bool> = Vec::new();
    let mut cuts = Vec::new();
    for &(a, b, is_lex) in &sections {
        let from = lines.len();
        let mut in_walk = false;
        for l in text[a..b].split('\n') {
            let i = lines.len();
            if is_lex && l.starts_with("    let (") && l.ends_with(") = {") && i > from && lines[i - 1].trim() == "#[cfg(feature = \"test-hooks\")]" {
                in_walk = true;
            }
            product.push(!is_lex || (!in_walk && !l.starts_with("    let Stage1 { ")));
            if in_walk && l == "    };" {
                in_walk = false;
            }
            lines.push(l);
        }
        cuts.push((a, b, from, lines.len()));
    }
    let whole = |t: &str, pat: &str| t.match_indices(pat).filter(|(i, _)| !t[..*i].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.') && !t[i + pat.len()..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')).count();
    let all: String = lines.iter().zip(&product).filter(|x| *x.1).map(|x| format!("{}\n", code_of(x.0))).collect();
    let mut cands: HashSet<&str> = exprs.iter().map(|e| e.0.as_str()).filter(|n| whole(&all, n) > 0).collect();
    // Until every mask left has each read in the bytes' form.
    loop {
        if cands.is_empty() {
            return text.to_owned();
        }
        let locals: HashMap<String, ::bitlex::syntax::Expr> = exprs.iter().filter(|e| cands.contains(e.0.as_str())).cloned().collect();
        let mut by = super::bytes::Bytes::new(stmts, locals, HashMap::new());
        // Each line's place: in the token loop and a token of code's row (`// `kind`: rule`, not a region's `read …`),
        // where a read starts in the loop's block.
        let mut in_loop = false;
        let mut code_row = false;
        let out: Vec<String> = lines
            .iter()
            .zip(&product)
            .map(|(l, &p)| {
                if l.starts_with("    'tokens: while b < nw {") {
                    // Its head's reads (the gap's, the starts') are of code too, from the block.
                    in_loop = true;
                    code_row = true;
                } else if in_loop && *l == "    }" {
                    in_loop = false;
                    code_row = false;
                }
                if let Some((_, rule)) = l.trim_start().strip_prefix("// `").and_then(|r| r.split_once("`: ")) {
                    code_row = in_loop && !rule.starts_with("read ");
                }
                if p { by.partial_line(l, !(in_loop && code_row)) } else { (*l).to_owned() }
            })
            .collect();
        let done: String = out.iter().zip(&product).filter(|x| *x.1).map(|x| format!("{}\n", code_of(x.0))).collect();
        let keep: Vec<&str> = cands.iter().copied().filter(|n| by.failed.contains(*n) || whole(&done, n) > 0).collect();
        if keep.is_empty() {
            let mut t = String::with_capacity(text.len());
            let mut at = 0;
            for &(a, b, from, to) in &cuts {
                t.push_str(&text[at..a]);
                let sec = out[from..to].join("\n");
                // The block's words its rows read, made once a block (`partial_line`).
                let head = "    'tokens: while b < nw {\n        let base = b * 64;\n";
                let sec = if sec.contains(head) { sec.replacen(head, &format!("{head}{}", by.block_decls()), 1) } else { sec };
                let sec = by.seen_tests(&sec);
                // A field's word of the 64 bytes from `p`: its writes of the window (`win_put`, `win_set`, `win_clear`).
                if text[a..].starts_with(super::T2_BEGIN) {
                    t.push_str(&sec.replace("wput(&mut fwin, ", "win_put(&mut fwin, p, ").replace("set(&mut fwin, ", "win_set(&mut fwin, p, ").replace("clear(&mut fwin, ", "win_clear(&mut fwin, p, "));
                } else {
                    t.push_str(&sec);
                }
                at = b;
            }
            t.push_str(&text[at..]);
            return format!("{}\n{}", drop_never(&t), by.code());
        }
        for k in keep {
            cands.remove(k);
        }
    }
}

/// A line's code: not its comment.
fn code_of(l: &str) -> &str {
    if l.trim_start().starts_with("//") { "" } else { l.split_once(" // ").map_or(l, |x| x.0) }
}

/// `lex` on `S`'s backend where it reads the bytes 64 at a time (`bit_reads`), dispatched as stage 1 is.
pub(super) fn generic_lex(text: &str) -> String {
    let at = "\npub fn lex(src: &[u8]) -> Lexed {";
    let Some(lx) = text.find(at) else { return text.to_owned() };
    let Some(len) = text[lx..].find("\n}\n") else { return text.to_owned() };
    if !text[lx..lx + len].contains("(src, n, ") {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len() + 256);
    out.push_str(&text[..lx]);
    out.push_str("\nunsafe fn lex_on(src: &[u8]) -> Lexed {");
    out.push_str(&text[lx + at.len()..lx + len + 3]);
    out.push_str("\nrerail_simd8::kernel! {\n    /// `src`'s records.\n    pub fn lex(src: &[u8]) -> Lexed = lex_on, on lex_on_best;\n}\n");
    out.push_str(&text[lx + len + 3..]);
    out
}

/// The escapes made of `code` in the token loop (decision 0703 step 17), as the lean lexer refuses a file at the token
/// it reads: word `i` of `code` is known once the loop has read every token that starts before its end, so the passes
/// over the words (`one_pass_after`) step to the block at each block's start (`code_pass!`), and the rest after the
/// loop. A pass that reads an earlier pass's mask past its word (`next`) steps a word behind that one, the mask in a
/// ring of four words. `code_l`, the vector of the file's `code`, is then the region tokens' spans not yet passed
/// (`code_spans`), each word of it kept in a ring too; the test build keeps the vector and holds each word to it. None
/// where a pass reads anything else of the file.
pub(super) fn code_pass_in_loop(text: &str) -> String {
    let head = "    // The escapes made of `code`: one pass over the words, each mask a word (";
    let lp = "    'tokens: while b < nw {\n        let base = b * 64;\n";
    let decl = "    let mut code_l = vec![!0u64; nw];\n";
    let zip = "    for (c, v) in code_l.iter_mut().zip(&lv) {\n        *c &= v;\n    }\n";
    if text.matches(lp).count() != 1 || text.matches(decl).count() != 1 || text.matches(zip).count() != 1 {
        return text.to_owned();
    }
    let Some(a) = text.find(&format!("\n{head}")).map(|a| a + 1) else { return text.to_owned() };
    // The passes, one after another: each its declarations and its body.
    let mut passes: Vec<(Vec<&str>, Vec<&str>)> = Vec::new();
    let mut at = a;
    while text[at..].starts_with(head) {
        let Some(f) = text[at..].find("\n    for i in 0..nw {\n").map(|f| at + f + 1) else { return text.to_owned() };
        let body_at = f + "    for i in 0..nw {\n".len();
        let Some(e) = text[body_at..].find("\n    }\n").map(|e| body_at + e + 1) else { return text.to_owned() };
        passes.push((text[at..f].lines().skip(1).collect(), text[body_at..e].lines().collect()));
        at = e + "    }\n".len();
    }
    let end = at;
    // A ring of four words holds what a pass two words behind reads.
    if text[end..].contains(head) || passes.len() > 3 {
        return text.to_owned();
    }
    // The masks kept for a later pass: rings of four words.
    let mut rings: Vec<String> = Vec::new();
    for (decls, _) in &mut passes {
        for d in decls.iter_mut() {
            if let Some(v) = d.trim().strip_prefix("let mut ").and_then(|r| r.strip_suffix(" = vec![0u64; nw];")) {
                rings.push(v.to_owned());
            }
        }
    }
    if rings.iter().any(|v| text[end..].contains(v.as_str())) {
        return text.to_owned();
    }
    let is_id = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let mut steps = String::new();
    let last = passes.len() - 1;
    for (p, (decls, body)) in passes.iter().enumerate() {
        let mut b: String = body.iter().map(|l| format!("            {l}\n")).collect();
        for v in &rings {
            b = b.replace(&format!("prev(&{v}, i, "), &format!("rprev(&{v}, i, ")).replace(&format!("next(&{v}, i, "), &format!("rnext(&{v}, nw, i, ")).replace(&format!("{v}[i]"), &format!("{v}[i & 3]"));
            if b.contains(&format!("{v}, ")) && !b.contains(&format!("rprev(&{v}, ")) && !b.contains(&format!("rnext(&{v}, ")) {
                return text.to_owned();
            }
        }
        // `code` a word before: the ring's; `code` at a byte, of the spans read.
        let b = b.replace("bit(&code_l, q)", "code_of(&code_spans, q)").replace("prev(&code_l, i, ", "rprev(&code_r, i, ").replace("code_l[i]", "code_w").replace("lv[i]", "lv_w");
        // Nothing else of the file's: no vector read at a word but a ring's.
        if b.contains("code_l") || b.match_indices("[i").any(|(k, _)| b[..k].ends_with(is_id) && !b[k..].starts_with("[i & 3]")) || b.match_indices(", i, ").any(|(k, _)| b[..k].rsplit(['(', '&']).next().is_some_and(|x| !x.is_empty() && x != "code_r" && !rings.iter().any(|v| v == x)) && !b[..k].ends_with("nw")) {
            return text.to_owned();
        }
        let _ = decls;
        let upto = if p == 0 { "upto".to_owned() } else { format!("if upto == nw {{ nw }} else {{ upto.saturating_sub({p}) }}") };
        let code = if p == 0 {
            "            // The word's bytes outside the region tokens read (`code`).\n            let mut code_w = lv_w;\n            for &(a, b) in &code_spans[code_k..] {\n                if a > 64 * i + 63 {\n                    break;\n                }\n                if b >= 64 * i {\n                    let (lo, hi) = (a.max(64 * i), b.min(64 * i + 63));\n                    code_w &= !((if hi - lo == 63 { !0 } else { (1u64 << (hi - lo + 1)) - 1 }) << (lo - 64 * i));\n                }\n            }\n            // The spans the words after start from (every span is kept: an escape's test reads `code` back).\n            while code_k < code_spans.len() && code_spans[code_k].1 <= 64 * i + 63 {\n                code_k += 1;\n            }\n            #[cfg(feature = \"test-hooks\")]\n            assert_eq!(code_w, code_l[i] & lv_w, \"`code_spans` against `code_l` at word {i}\");\n            code_r[i & 3] = code_w;\n".to_owned()
        } else {
            "            let code_w = code_r[i & 3];\n".to_owned()
        };
        let _ = std::fmt::Write::write_fmt(&mut steps, format_args!("        // Pass {p}{}.\n        let to: usize = {upto};\n        while code_i[{p}] < to {{\n            let i = code_i[{p}];\n            code_i[{p}] += 1;\n            let lv_w = if 64 * i + 64 <= n {{ !0u64 }} else {{ (1u64 << (n % 64)) - 1 }};\n{code}{b}        }}\n", if p == 0 { "" } else { ", a word behind the pass before: it reads that one's masks after its word" }));
        let _ = last;
    }
    let mut decls = String::new();
    for (ds, _) in &passes {
        for d in ds {
            match d.trim().strip_prefix("let mut ").and_then(|r| r.strip_suffix(" = vec![0u64; nw];")) {
                Some(v) => {
                    let _ = std::fmt::Write::write_fmt(&mut decls, format_args!("    let mut {v} = [0u64; 4];\n"));
                }
                None => {
                    decls.push_str(d);
                    decls.push('\n');
                }
            }
        }
    }
    let ring_fns = if rings.is_empty() && !steps.contains("rprev(&code_r") { "" } else { "    /// A ring's word `i` of `prev(x, k)`: each byte's bit from `k` bytes before.\n    #[inline(always)]\n    fn rprev(x: &[u64; 4], i: usize, k: u32) -> u64 {\n        x[i & 3] << k | if i > 0 { x[(i - 1) & 3] >> (64 - k) } else { 0 }\n    }\n    /// A ring's word `i` of `next(x, k)`, of `nw` words: each byte's bit from `k` bytes after.\n    #[inline(always)]\n    fn rnext(x: &[u64; 4], nw: usize, i: usize, k: u32) -> u64 {\n        x[i & 3] >> k | if i + 1 < nw { x[(i + 1) & 3] << (64 - k) } else { 0 }\n    }\n" };
    let pass = format!(
        "    // The escapes made of `code`, a word at a time as the token loop leaves each block (decision 0703 step 17): word `i`\n    // is `code` once every token that starts before its end is read.\n{decls}{ring_fns}    let mut code_r = [0u64; 4];\n    let mut code_i = [0usize; {}];\n    macro_rules! code_pass {{\n        ($upto:expr) => {{\n        let upto: usize = $upto;\n{steps}        }};\n    }}\n",
        passes.len()
    );
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..a]);
    out.push_str("    // The escapes made of `code`: the words the loop left.\n    code_pass!(nw);\n");
    out.push_str(&text[end..]);
    // The spans, not the vector; the vector the test build's.
    let mut out = out.replacen(decl, &format!("    // The region tokens' spans, in order, and the first not yet passed by the escapes made of `code` (`code_pass`).\n    let mut code_k = 0usize;\n    let mut code_spans: Vec<(usize, usize)> = Vec::with_capacity(64);\n    #[cfg(feature = \"test-hooks\")]\n{decl}"), 1);
    out = out.replacen(zip, &format!("    #[cfg(feature = \"test-hooks\")]\n{zip}"), 1);
    let mut lines: Vec<String> = Vec::new();
    for l in out.split_inclusive('\n') {
        if l.trim() == "clear(&mut code_l, s, l);" {
            let ind = &l[..l.len() - l.trim_start().len()];
            lines.push(format!("{ind}code_spans.push((s, l));\n{ind}#[cfg(feature = \"test-hooks\")]\n"));
        }
        lines.push(l.to_owned());
    }
    let out: String = lines.concat();
    let out = out.replacen(lp, &format!("{pass}{lp}        code_pass!(b);\n"), 1);
    // The file's word bits (`lv`), where only the test build reads them: its alone.
    let lv = "    // Each word's bits inside the file.\n    let lv: Vec<u64> = (0..nw).map(|q| if 64 * q + 64 <= n { !0 } else { (1u64 << (n % 64)) - 1 }).collect();\n";
    if out.matches(lv).count() == 1 && !product_reads(&out[out.find(lv).expect("one") + lv.len()..], &["lv[", "&lv)", "&lv,"]) {
        return out.replacen(lv, &lv.replacen("\n    let lv", "\n    #[cfg(feature = \"test-hooks\")]\n    let lv", 1), 1);
    }
    out
}

/// Whether a line of `text` up to its function's end reads one of `pats` outside a block or a statement the test build
/// alone has (`#[cfg(feature = "test-hooks")]` before it).
fn product_reads(text: &str, pats: &[&str]) -> bool {
    let mut test_until: Option<String> = None;
    let mut cfg = false;
    for l in text.lines() {
        if l == "}" {
            return false;
        }
        if let Some(end) = &test_until {
            if l == end {
                test_until = None;
            }
            continue;
        }
        if cfg {
            cfg = false;
            let ind = &l[..l.len() - l.trim_start().len()];
            if l.trim_end().ends_with('{') {
                test_until = Some(format!("{ind}}}"));
                // A `let … = {` closes at its `};`.
                if l.trim_start().starts_with("let ") {
                    test_until = Some(format!("{ind}}};"));
                }
            }
            continue;
        }
        if l.trim() == "#[cfg(feature = \"test-hooks\")]" {
            cfg = true;
            continue;
        }
        if pats.iter().any(|p| code_of(l).contains(p)) {
            return true;
        }
    }
    false
}

/// Stage 1 run only in a test build, where the product reads nothing of it (decision 0703 step 31): every field of
/// `Stage1` but `refused` only a test build's, and no escape of stage 1's sets `refused`. The token loop reads the
/// bytes, as the lean lexer's reads its block's masks: so the product makes no mask the loop does not read.
pub(super) fn stage1_for_tests(text: &str) -> String {
    let cfg = "#[cfg(feature = \"test-hooks\")]";
    let Some(a) = text.find("\nstruct Stage1 {\n") else { return text.to_owned() };
    let Some(e) = text[a + 1..].find("\n}\n").map(|e| a + 1 + e) else { return text.to_owned() };
    let lines: Vec<&str> = text[a + 1..e].lines().skip(1).collect();
    for (k, l) in lines.iter().enumerate() {
        let t = l.trim();
        if t.is_empty() || t.starts_with("//") || t.starts_with("#[") {
            continue;
        }
        if t != "refused: bool," && (k == 0 || lines[k - 1].trim() != cfg) {
            return text.to_owned();
        }
    }
    if text.contains("st.refused |=") || text.contains("st.refused = true") {
        return text.to_owned();
    }
    let head = "        if st1.t < b + 2 + ";
    let fin = "    stage1_best(src, &mut st1, &mut s1, usize::MAX);\n";
    let past = "    if let Some(p) = r.into() {\n        let to = p / 64 + 2 + ";
    if text.matches(head).count() != 1 || text.matches(fin).count() != 1 || text.matches(past).count() != 1 {
        return text.to_owned();
    }
    text.replacen(head, &format!("        // Stage 1 for the test build's checks alone: the product reads none of it.\n        {cfg}\n{head}"), 1)
        .replacen(fin, &format!("    {cfg}\n{fin}"), 1)
        .replacen(past, &format!("    #[cfg(not(feature = \"test-hooks\"))]\n    let _ = (src, &st, &s1);\n    {cfg}\n{past}"), 1)
}

/// `text` without the tests at a token's end the bytes' form showed hold nowhere (`if … (false) {`): their comment, the
/// test and its block.
fn drop_never(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let never = |l: &str| {
            let t = l.trim();
            t.starts_with("if ") && t.ends_with(" {") && {
                let c = &t[3..t.len() - 2];
                c == "(false)" || c.strip_prefix("l + ").and_then(|r| r.split_once(" < n && ")).is_some_and(|(k, r)| k.parse::<u32>().is_ok() && r == "(false)")
            }
        };
        if lines[i].trim_start().starts_with("// ") && lines[i].trim_end().ends_with("where it may hold") && i + 1 < lines.len() && never(lines[i + 1]) {
            let ind = lines[i + 1].len() - lines[i + 1].trim_start().len();
            if let Some(e) = (i + 2..lines.len()).find(|&j| lines[j].len() - lines[j].trim_start().len() == ind && lines[j].trim() == "}") {
                i = e + 1;
                continue;
            }
        }
        // `if (false) { … } else { B }` over lines: `B`, a level less in.
        if lines[i].trim() == "if (false) {" {
            let ind = lines[i].len() - lines[i].trim_start().len();
            let at = |j: usize, t: &str| lines[j].len() - lines[j].trim_start().len() == ind && lines[j].trim() == t;
            if let Some(m) = (i + 1..lines.len()).find(|&j| at(j, "} else {") || at(j, "}"))
                && at(m, "} else {")
                && let Some(e) = (m + 1..lines.len()).find(|&j| at(j, "}"))
            {
                for l in &lines[m + 1..e] {
                    out.push(l.strip_prefix("    ").unwrap_or(l));
                }
                i = e + 1;
                continue;
            }
        }
        out.push(lines[i]);
        i += 1;
    }
    // A branch the bytes' form showed holds nowhere: `if (false) { X } else ` is its `else`'s.
    let mut t = out.join("\n");
    while let Some(a) = t.find("if (false) { ") {
        let Some(e) = t[a..].find(" } else ").map(|e| a + e) else { break };
        if t[a + "if (false) { ".len()..e].contains(['{', '}']) {
            break;
        }
        t.replace_range(a..e + " } else ".len(), "");
    }
    t
}
