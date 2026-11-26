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
    let h = find(0, &|x| x.starts_with("unsafe fn stage1<") && x.ends_with("(src: &[u8]) -> Stage1 {"))?;
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
        "\n/// `first_bit` of a mask of stage 1's, its blocks made as the scan comes to them (decision 0703).\n#[inline(always)]\nfn s1_first_bit(src: &[u8], st: &mut Stage1Run, s1: &mut Stage1, m: impl Fn(&Stage1) -> &[u64], from: usize) -> Option<usize> {{\n    let nw = src.len().div_ceil(64);\n    loop {{\n        let made = st.t.saturating_sub({lag}).min(nw);\n        if from / 64 + 1 < made || made == nw {{\n            match first_bit(&m(s1)[..made], from) {{\n                Some(p) if p / 64 + 1 < made || made == nw => return Some(p),\n                None if made == nw => return None,\n                _ => {{}}\n            }}\n        }}\n        let to = (st.t * 2).max(st.t + 16);\n        stage1_best(src, st, s1, to);\n    }}\n}}\n\n/// `run_last` of a mask of stage 1's, its blocks made as the run comes to them (decision 0703).\n#[inline(always)]\nfn s1_run_last(src: &[u8], st: &mut Stage1Run, s1: &mut Stage1, m: impl Fn(&Stage1) -> &[u64], s: usize) -> usize {{\n    let nw = src.len().div_ceil(64);\n    loop {{\n        let made = st.t.saturating_sub({lag}).min(nw);\n        if s / 64 + 1 < made || made == nw {{\n            let l = run_last(&m(s1)[..made], s);\n            if l / 64 + 1 < made || made == nw {{\n                return l;\n            }}\n        }}\n        let to = (st.t * 2).max(st.t + 16);\n        stage1_best(src, st, s1, to);\n    }}\n}}\n"
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
    let lex = &lex[..lex.find("\n}\n").unwrap_or(lex.len())];
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

/// The one-byte tests of stage 1's masks as tests of the bytes (decision 0703), as the lean lexer tests `c == b'/'`:
/// a `bit(&M, i)` in `lex` (not the test build's walk of the regions) where `M` reads no region and `lex` reads it only
/// so. Stage 1 then keeps no vector of it in the product build (`test_only`). `exprs`: each mask's expression.
pub(super) fn bit_reads(text: &str, stmts: &[::bitlex::syntax::Stmt], exprs: &[(String, ::bitlex::syntax::Expr)]) -> String {
    let Some(lx) = text.find("\npub fn lex(src: &[u8]) -> Lexed {") else { return text.to_owned() };
    let Some(len) = text[lx..].find("\n}\n") else { return text.to_owned() };
    let (head, lex, tail) = (&text[..lx], &text[lx..lx + len], &text[lx + len..]);
    // The product's lines: not the destructuring of stage 1's masks, nor the walk.
    let mut product = String::new();
    let mut in_walk = false;
    let lines: Vec<&str> = lex.split('\n').collect();
    for (i, l) in lines.iter().enumerate() {
        if l.starts_with("    let (") && l.ends_with(") = {") && i > 0 && lines[i - 1].trim() == "#[cfg(feature = \"test-hooks\")]" {
            in_walk = true;
        }
        if !in_walk && !l.starts_with("    let Stage1 { ") {
            product.push_str(l);
            product.push('\n');
        }
        if in_walk && *l == "    };" {
            in_walk = false;
        }
    }
    let whole = |t: &str, pat: &str| t.match_indices(pat).filter(|(i, _)| !t[..*i].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_' || c == '.') && !t[i + pat.len()..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')).count();
    let by = super::bytes::Bytes::new(stmts, HashMap::new(), HashMap::new());
    let mut tests: Vec<(&str, &::bitlex::syntax::Expr)> = Vec::new();
    for (name, e) in exprs {
        let reads = whole(&product, name);
        let bits = product.matches(&format!("bit(&{name}, ")).count();
        if reads > 0 && reads == bits && by.at(e, "s", 0).is_ok() {
            tests.push((name, e));
        }
    }
    if tests.is_empty() {
        return text.to_owned();
    }
    // Each `bit(&M, i)` outside the walk and the destructuring, as the test of the bytes at `i`.
    let mut out = String::new();
    let mut in_walk = false;
    for (i, l) in lines.iter().enumerate() {
        if l.starts_with("    let (") && l.ends_with(") = {") && i > 0 && lines[i - 1].trim() == "#[cfg(feature = \"test-hooks\")]" {
            in_walk = true;
        }
        let mut line = (*l).to_owned();
        if !in_walk && !l.starts_with("    let Stage1 { ") {
            for (name, e) in &tests {
                let pat = format!("bit(&{name}, ");
                let mut o = String::new();
                let mut rest = line.as_str();
                while let Some(p) = rest.find(&pat) {
                    let ok = !rest[..p].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_');
                    o.push_str(&rest[..p]);
                    let after = &rest[p + pat.len()..];
                    // The position: to the call's close.
                    let mut depth = 0i32;
                    let end = after.char_indices().find(|&(_, c)| {
                        match c {
                            '(' | '[' => depth += 1,
                            ')' | ']' if depth > 0 => depth -= 1,
                            ')' => return true,
                            _ => {}
                        }
                        false
                    });
                    match (ok, end) {
                        (true, Some((k, _))) => {
                            let at = after[..k].trim();
                            let at = if at.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') { at.to_owned() } else { format!("({at})") };
                            match by.at(e, &at, 0) {
                                Ok(t) => {
                                    let _ = write!(o, "({t})");
                                    rest = &after[k + 1..];
                                }
                                Err(_) => {
                                    o.push_str(&pat);
                                    rest = after;
                                }
                            }
                        }
                        _ => {
                            o.push_str(&pat);
                            rest = after;
                        }
                    }
                }
                o.push_str(rest);
                line = o;
            }
        }
        if in_walk && *l == "    };" {
            in_walk = false;
        }
        out.push_str(&line);
        out.push('\n');
    }
    out.pop();
    format!("{head}{out}{tail}")
}
