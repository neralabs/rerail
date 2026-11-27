//! Lexing from the middle for a token loop that keeps a nest (Kotlin's, decision 0621): the loop's state between two
//! tokens as a value (`Entry`), the loop from any entry to any block (`lex_range`), and the three passes, a chunk's
//! summary its lanes (`Lane`) as the chunk-summaries design states them: each lane what the chunk does for one class
//! of entries, the levels of the entry it closes (`kc`) and those it leaves open (`p`), composed as typed bracket
//! sequences.
//!
//! What crosses a chunk's start is the loop's state there: where the next token starts, the nest and the strings its
//! templates hold open, and the few values the loop keeps of the token before. Stage 1, which the loop reads as it
//! comes to its blocks, depends on the bytes alone: a chunk starts it `PRIME` blocks before its first, where its windows
//! and carries are the serial run's again.

use std::fmt::Write;

/// A field of the loop's state: its name, its type and its value at the file's first byte.
struct Field {
    name: String,
    ty: String,
    init: String,
}

/// The type of a declaration's value (`0usize`, `false`, `usize::MAX`, `[0usize; 4]`).
fn type_of(init: &str) -> Option<String> {
    let init = init.trim();
    if let Some(inner) = init.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        let (v, k) = inner.rsplit_once(';')?;
        return Some(format!("[{}; {}]", type_of(v)?, k.trim()));
    }
    if let Some(inner) = init.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
        return Some(format!("({})", inner.split(',').map(type_of).collect::<Option<Vec<_>>>()?.join(", ")));
    }
    Some(
        match init {
            "false" | "true" => "bool",
            "usize::MAX" => "usize",
            "u32::MAX" => "u32",
            v if v.ends_with("usize") => "usize",
            v if v.ends_with("u32") => "u32",
            v if v.ends_with("u64") => "u64",
            v if v.ends_with("u8") => "u8",
            _ => return None,
        }
        .to_owned(),
    )
}

/// The loop's state declared before it, from `let mut at = 0usize;` (included) to `let mut b = at / 64;`: each `let mut
/// NAME = V;` and `let (mut A, mut B) = (V, W);`. `close`, declared with no value, is each token's own.
fn fields(decls: &str) -> Option<Vec<Field>> {
    let mut out = Vec::new();
    for l in decls.lines() {
        let t = l.trim();
        if t.is_empty() || t.starts_with("//") || t == "let mut close: usize;" {
            continue;
        }
        if let Some(r) = t.strip_prefix("let (") {
            let (names, vals) = r.strip_suffix(");")?.split_once(") = (")?;
            for (n, v) in names.split(',').zip(vals.split(',')) {
                let name = n.trim().strip_prefix("mut ")?.to_owned();
                out.push(Field { ty: type_of(v)?, name, init: v.trim().to_owned() });
            }
            continue;
        }
        let r = t.strip_prefix("let mut ")?.strip_suffix(';')?;
        let (name, init) = r.split_once(" = ")?;
        let (name, ty) = match name.split_once(':') {
            Some((n, ty)) => (n.trim().to_owned(), ty.trim().to_owned()),
            None => (name.trim().to_owned(), type_of(init)?),
        };
        out.push(Field { name, ty, init: init.trim().to_owned() });
    }
    Some(out)
}

/// Whether the loop `body` reads `name`: an occurrence in its code (not a comment, a literal or a path's) that is not the
/// target of a plain `=`.
fn reads(body: &str, name: &str) -> bool {
    let code = String::from_utf8(super::middle::code_only(body)).expect("blanks keep the text");
    let body = code.as_str();
    let b = body.as_bytes();
    body.match_indices(name).any(|(i, _)| {
        let before = i.checked_sub(1).map(|k| b[k]);
        let after = b.get(i + name.len()).copied();
        let word = |c: Option<u8>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_');
        if word(before) || word(after) || before == Some(b'.') || before == Some(b':') {
            return false;
        }
        let rest = body[i + name.len()..].trim_start();
        !(rest.starts_with('=') && !rest.starts_with("=="))
    })
}

/// The loop's product body with its writes on where `WRITE` (pass 2 alone writes records), and its reads of the nest
/// below its entry counted (`Reads`): a bracket at depth 0, `template` at depth 0 (nothing open), `opened` at 0, and the
/// levels it goes to.
fn instrument(body: &str) -> Result<String, String> {
    let lines: Vec<&str> = body.lines().collect();
    let mut out = String::new();
    let mut guarded = 0;
    // The `match`es made `if let Some(f) = force { … } else { match … }`: their indents, and what follows their close.
    let mut closes: Vec<(String, String)> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let t = l.trim_start();
        let pad = &l[..l.len() - t.len()];
        if t == "unsafe { out.add(len).write(rec) };" || t == "extras.push(rec);" || (t.starts_with("unsafe { (*out.add(") && t.ends_with(';')) {
            guarded += 1;
            let _ = writeln!(out, "{pad}if WRITE {{\n{pad}    {t}\n{pad}}}");
            continue;
        }
        if t == "'tokens: while b < nw {" {
            let _ = writeln!(out, "{pad}'tokens: while b < nw.min(to) {{");
            continue;
        }
        // A forced first token (`Force`, a region lane's): its start, its region and its row, not the bytes'.
        if l.starts_with("        ") && !l.starts_with("         ") {
            match t {
                "while firsts != 0 {" => {
                    let _ = writeln!(out, "{pad}while firsts != 0 || force.is_some() {{");
                    continue;
                }
                "let s = base + firsts.trailing_zeros() as usize;" => {
                    let _ = writeln!(out, "{pad}let s = match force {{\n{pad}    Some(f) => f.s,\n{pad}    None => base + firsts.trailing_zeros() as usize,\n{pad}}};");
                    continue;
                }
                "let open = match group {" => {
                    let _ = writeln!(out, "{pad}let open = if let Some(f) = force {{ f.open }} else {{ match group {{");
                    closes.push((pad.to_owned(), String::new()));
                    continue;
                }
                "at = l + 1;" => {
                    let _ = writeln!(out, "{l}\n{pad}if ONE {{\n{pad}    break 'tokens;\n{pad}}}");
                    continue;
                }
                _ => {}
            }
        }
        if l.starts_with("            ") && !l.starts_with("             ") && t == "let row = match group {" {
            let _ = writeln!(out, "{pad}let row = if let Some(f) = force {{ f.row }} else {{ match group {{");
            closes.push((pad.to_owned(), format!("{pad}let force_lv = force.map_or(0, |f| f.lv);\n{pad}force = None;\n")));
            continue;
        }
        if t == "let mut lv = 0usize;" {
            let _ = writeln!(out, "{pad}let mut lv = force_lv;");
            continue;
        }
        if t == "};"
            && let Some(k) = closes.iter().position(|(p, _)| p == pad)
        {
            let (_, after) = closes.remove(k);
            let _ = write!(out, "{pad}}} }};\n{after}");
            continue;
        }
        if t == "if depth > 0 {" {
            // `if (A) {` … `} else if (B) {` at the next level: the brackets, read past the bottom where none is open.
            let inner = lines.get(i + 1).ok_or("the line after `if depth > 0`")?.trim();
            let a = inner.strip_prefix("if ").and_then(|r| r.strip_suffix(" {")).ok_or_else(|| format!("no `if A {{` after `if depth > 0`: {inner}"))?;
            let ipad = &lines[i + 1][..lines[i + 1].len() - inner.len()];
            let b = lines[i + 2..].iter().find_map(|m| m.strip_prefix(ipad).and_then(|r| r.strip_prefix("} else if ")).and_then(|r| r.strip_suffix(" {"))).ok_or("no `} else if B {`")?;
            let _ = writeln!(out, "{pad}if depth == 0 && ({a} || {b}) {{\n{pad}    reads.bottom = true;\n{pad}}}");
        }
        if t == "match opened {" {
            let _ = writeln!(out, "{pad}match opened_read(reads, &nest, depth, opened) {{");
            continue;
        }
        let l = l.replace("(template == ", "(at_bottom(reads, depth, template) && template == ");
        let _ = writeln!(out, "{l}");
        if t == "depth += 1;" {
            let _ = writeln!(out, "{pad}reads.high = reads.high.max(depth);");
        } else if t == "depth -= 1;" {
            let _ = writeln!(out, "{pad}reads.low = reads.low.min(depth);");
        }
    }
    // Every write of a record is one pass 2 alone makes.
    if !closes.is_empty() || !out.contains("f.row") || !out.contains("f.open") || !out.contains("if ONE {") {
        return Err("the token loop's start, its region, its row or its end: not of the shape a forced token is".into());
    }
    let writes = body.matches("out.add(").count() + body.matches("extras.push(").count();
    if writes != guarded {
        return Err(format!("{writes} writes of records, {guarded} of them pass 2's"));
    }
    Ok(out)
}

/// Each region a chunk can start inside, by the token loop's rows (`body`): its number (`open`), the row that reads it,
/// the bytes its closer's search starts past the token's start (`s + K`), and, for a region that nests, its level
/// (one entry a level).
fn region_entries(body: &str) -> Result<Vec<(usize, usize, usize, usize)>, String> {
    let rm = body.find("            let row = match group {").ok_or("no row match")?;
    let rows_end = rm + body[rm..].find("\n            };").ok_or("the row match's end")?;
    let rows = &body[rm..rows_end];
    let mut out: Vec<(usize, usize, usize, usize)> = Vec::new();
    // A region a group opens alone (`G => K,` in the open match) and the row that group takes (`G => R,`).
    let om = body.find("        let open = match group {").ok_or("no open match")?;
    let opens = &body[om..om + body[om..].find("\n        };").ok_or("the open match's end")?];
    let consts = |t: &str| -> Vec<(usize, usize)> {
        t.lines().filter_map(|l| l.trim().strip_suffix(',')?.split_once(" => ").and_then(|(g, v)| Some((g.trim().parse().ok()?, v.trim().parse().ok()?)))).collect()
    };
    let by_group: Vec<(usize, usize)> = consts(opens).into_iter().filter_map(|(g, open)| consts(rows).into_iter().find(|&(h, _)| h == g).map(|(_, row)| (open, row))).collect();
    let mut pairs: Vec<(usize, usize)> = by_group;
    for (i, _) in rows.match_indices("open == ") {
        let k: String = rows[i + 8..].chars().take_while(char::is_ascii_digit).collect();
        // (`open == usize::MAX`: a token that opens none.)
        if k.is_empty() {
            continue;
        }
        let rest = rows[i + 8 + k.len()..].trim_start().strip_prefix('{').ok_or("an `open == K` with no block")?.trim_start();
        let r: String = rest.chars().take_while(char::is_ascii_digit).collect();
        let (Ok(open), Ok(row)) = (k.parse::<usize>(), r.parse::<usize>()) else { return Err(format!("`open == {k}`'s row")) };
        if !pairs.iter().any(|&(o, _)| o == open) {
            pairs.push((open, row));
        }
    }
    for (open, row) in pairs {
        // The row's arm, to the next.
        let head = format!("\n                {row} => {{\n");
        let a = body.find(&head).ok_or_else(|| format!("row {row}'s arm"))? + head.len();
        let z = a + body[a..].find("\n                }\n").ok_or("the arm's end")?;
        let arm = &body[a..z];
        let from = ["wfirst_bit(n, s + ", "let mut q = s + "].iter().filter_map(|f| arm.find(f).map(|i| i + f.len())).chain(arm.find("first_bit(n, base, ").and_then(|i| arm[i..].find(", s + ").map(|j| i + j + 6))).min().ok_or_else(|| format!("row {row}: no closer search"))?;
        let k: usize = arm[from..].chars().take_while(char::is_ascii_digit).collect::<String>().parse().map_err(|_| format!("row {row}: its closer's offset"))?;
        let levels = arm.find("if lv == ").and_then(|i| arm[i + 9..].chars().take_while(char::is_ascii_digit).collect::<String>().parse::<usize>().ok()).unwrap_or(0);
        for lv in 0..=levels {
            out.push((open, row, k, lv));
        }
    }
    if out.is_empty() {
        return Err("no regions".into());
    }
    out.sort_unstable();
    Ok(out)
}

/// The chunked form of `out`'s token loop where it keeps a nest (`nest`, `depth`, `template`, `held`, `opened`), its
/// stage 1 made as it comes to its blocks (`Stage1Run`), or where its nest is its string reader's (`reader`,
/// `chunked_reader`): `out` with it appended, else `out`.
pub(super) fn with_chunks(out: String, reader: Option<&ReaderNest>) -> String {
    let t = match reader {
        Some(r) => chunked_reader(&out, r),
        None => chunked_nest(&out),
    };
    match t {
        Ok(t) => out + &t,
        Err(_) => out,
    }
}

/// What a token loop's chunked form needs of its string reader (`read R { … }`, decision 0698), whose state is its nest:
/// its quotes, its interpolation's flag (`format`), the flags a string may hold besides it (`free`: the others but those
/// refused at its opener, and the triple's), and the interpolation opener's byte.
pub(super) struct ReaderNest {
    pub quotes: u8,
    pub format: u8,
    pub free: Vec<u8>,
    pub open_byte: u8,
}

fn chunked_nest(out: &str) -> Result<String, String> {
    let head = "unsafe fn lex_on(src: &[u8]) -> Lexed {\n";
    let f = out.find(head).ok_or("step 1")?;
    let end = f + out[f..].find("\n}\n").ok_or("the function's end")? + 3;
    let prod = super::middle::product_text(&out[f..end]);
    for need in ["    let mut st1 = Stage1Run::new();\n", "    let mut s1 = Stage1::new(nw);\n", "    let mut nest = [", "    let mut held = [", "    let mut opened = "] {
        if !prod.contains(need) {
            return Err(format!("a condition at line {}", line!()));
        }
    }
    // Stage 1's refusals are the loop's: stage 1 sets none of its own.
    if out.matches("st.refused").count() != 1 {
        return Err(format!("a condition at line {}", line!()));
    }
    let d0 = prod.find("    let mut at = 0usize;\n").ok_or("step 2")?;
    let d1 = prod.find("    let mut b = at / 64;\n").ok_or("step 3")?;
    let l0 = prod.find("    'tokens: while b < nw {\n").ok_or("step 4")?;
    let l1 = l0 + prod[l0..].find("\n    }\n").ok_or("the loop's end")? + "\n    }\n".len();
    let body = &prod[l0..l1];
    let all = fields(&prod[d0..d1]).ok_or("step 5")?;
    // The state the loop reads; a value it only writes is each run's own.
    let (live, dead): (Vec<&Field>, Vec<&Field>) = all.iter().partition(|fl| reads(body, &fl.name));
    if !["at", "nest", "depth", "template", "held", "opened"].iter().all(|n| live.iter().any(|fl| fl.name == *n)) {
        return Err(format!("a condition at line {}", line!()));
    }
    let maxd: usize = body.lines().find_map(|l| l.trim().strip_prefix("if depth == ").and_then(|r| r.strip_suffix(" {"))).ok_or("step 6")?.parse().ok().ok_or("step 7")?;
    // The kinds a level can be: a bracket's, then each template's (`template = K;`).
    let mut kinds: Vec<usize> = Vec::new();
    for (i, _) in body.match_indices("template = ") {
        let k: String = body[i + "template = ".len()..].chars().take_while(char::is_ascii_digit).collect();
        if let Ok(k) = k.parse::<usize>()
            && !kinds.contains(&k)
        {
            kinds.push(k);
        }
    }
    if kinds.is_empty() || kinds.iter().any(|&k| k >= 255) {
        return Err(format!("a condition at line {}", line!()));
    }
    let lp = instrument(body)?;
    let regions = region_entries(body)?;
    // After the loop: what the serial run checks at the file's end, of the state alone.
    let after = &prod[l1..prod.find("    unsafe { recs.set_len(len) };").ok_or("the records' length")?];
    let mut finish = String::new();
    for l in after.lines() {
        let t = l.trim();
        if t.is_empty() || t.starts_with("//") || t.starts_with("let _ = (") || t == "stage1_best(src, &mut st1, &mut s1, usize::MAX);" || t == "let escapes = s1.refused;" || t == "refused |= escapes;" {
            continue;
        }
        if t.contains("st1") || t.contains("s1.") || t.contains("len") || t.contains("out") {
            return Err(format!("a condition at line {}", line!()));
        }
        let _ = writeln!(finish, "{l}");
    }
    let names: Vec<&str> = live.iter().map(|fl| fl.name.as_str()).collect();
    let struct_fields: String = live.iter().map(|fl| format!("    pub {}: {},\n", fl.name, fl.ty)).collect();
    let inits: String = live.iter().map(|fl| format!("{}: {}, ", fl.name, fl.init)).collect();
    let muts: String = names.iter().map(|n| format!("mut {n}, ")).collect();
    let back = names.join(", ");
    let locals: String = dead.iter().map(|fl| format!("    let mut {}: {} = {};\n", fl.name, fl.ty, fl.init)).collect();
    let kinds_s = std::iter::once("255".to_owned()).chain(kinds.iter().map(usize::to_string)).collect::<Vec<_>>().join(", ");
    let t = TEMPLATE
        .replace("@FIELDS@", &struct_fields)
        .replace("@INITS@", &inits)
        .replace("@MUTS@", &muts)
        .replace("@BACK@", &back)
        .replace("@LOCALS@", &locals)
        .replace("@LOOP@", &lp)
        .replace("@FINISH@", &finish)
        .replace("@ALL@", &names.iter().map(|n| format!("{n}, ")).collect::<String>())
        .replace("@NREGIONS@", &regions.len().to_string())
        .replace("@REGIONS@", &regions.iter().map(|(o, r, k, lv)| format!("({o}, {r}, {k}, {lv})")).collect::<Vec<_>>().join(", "));
    // The lanes: a chunk's code, each place a token it starts in ends, each region's end, each split by its nest's class.
    Ok(t + &core_with(maxd, &kinds_s, kinds.len() + 1, 64, false, false))
}

/// The chunked form of a token loop that reads its blocks' bytes (no stage 1) and whose nest is its string reader's state
/// (`rd_on`, `rd_q`, `rd_fl`, `rd_level`, `rd_total`, `rd_around`, `rd_depth`) and the code's bracket count `c_depth`
/// (Python's): `chunked_nest`'s template, the nest read as levels (`READER_NEST`) and the loop's reads of it checked
/// against them where it makes them (`instrument_reader`).
fn chunked_reader(out: &str, rn: &ReaderNest) -> Result<String, String> {
    let head = "unsafe fn lex_on(src: &[u8], n: usize) -> Lexed {\n";
    let f = out.find(head).ok_or("step 1")?;
    let end = f + out[f..].find("\n}\n").ok_or("the function's end")? + 3;
    let prod = super::middle::product_text(&out[f..end]);
    if prod.contains("Stage1Run::new()") || !prod.contains("let cur = unsafe { block_classes(src, n, base) };") {
        return Err(format!("a condition at line {}", line!()));
    }
    let d0 = prod.find("    let mut at = 0usize;\n").ok_or("step 2")?;
    let d1 = prod.find("    let mut b = at / 64;\n").ok_or("step 3")?;
    let l0 = prod.find("    'tokens: while b < nw {\n").ok_or("step 4")?;
    let l1 = l0 + prod[l0..].find("\n    }\n").ok_or("the loop's end")? + "\n    }\n".len();
    let body = &prod[l0..l1];
    let all = fields(&prod[d0..d1]).ok_or("step 5")?;
    let (live, dead): (Vec<&Field>, Vec<&Field>) = all.iter().partition(|fl| reads(body, &fl.name));
    let need = ["at", "c_depth", "rd_on", "rd_q", "rd_fl", "rd_level", "rd_total", "rd_around", "rd_depth", "rd_open", "rd_code"];
    if let Some(m) = need.iter().find(|n| !live.iter().any(|fl| fl.name == **n)) {
        return Err(format!("the loop reads no `{m}`"));
    }
    let maxd: usize = body.lines().find_map(|l| l.trim().strip_prefix("if c_depth > ").and_then(|r| r.strip_suffix(" {"))).ok_or("the count's bound")?.parse().map_err(|_| "the count's bound")?;
    let picked = reader_picked(body)?;
    let lp = instrument_reader(body, &rn.free, &picked)?;
    let ws: usize = body.lines().find_map(|l| l.trim().strip_prefix("let k_m_ws = cur[").and_then(|r| r.strip_suffix("];"))).ok_or("the loop's whitespace")?.parse().map_err(|_| "the loop's whitespace")?;
    let regions = reader_regions(body, rn)?;
    let after = &prod[l1..prod.find("    unsafe { recs.set_len(len) };").ok_or("the records' length")?];
    let mut finish = String::new();
    for l in after.lines() {
        let t = l.trim();
        if t.is_empty() || t.starts_with("//") || t.starts_with("let _ = (") {
            continue;
        }
        if t.contains("len") || t.contains("out") {
            return Err(format!("a condition at line {}", line!()));
        }
        let _ = writeln!(finish, "{l}");
    }
    // The kinds of string: those an interpolation can be in (a level's kind, `RD_KINDS`), and every string's (where a
    // chunk can start in its text, `RD_PLACES`).
    let subsets = |bits: &[u8]| -> Vec<u8> { (0..1u32 << bits.len()).map(|m| bits.iter().enumerate().filter(|(i, _)| m >> i & 1 == 1).fold(0u8, |a, (_, b)| a | b)).collect() };
    let interp: Vec<(u8, u8)> = (0..rn.quotes).flat_map(|q| subsets(&rn.free).into_iter().map(move |f| (q, f | rn.format))).collect();
    let mut place_bits = rn.free.clone();
    place_bits.push(rn.format);
    let places: Vec<(u8, u8)> = (0..rn.quotes).flat_map(|q| subsets(&place_bits).into_iter().map(move |f| (q, f))).collect();
    if interp.len() >= 252 || maxd > 64 {
        return Err(format!("a condition at line {}", line!()));
    }
    let table = |v: &[(u8, u8)]| v.iter().map(|(q, f)| format!("({q}, {f:#04x})")).collect::<Vec<_>>().join(", ");
    // An interpolation's kind as far as a lane read it (`PARTS`): its free flags' bits, then its quote's. Its whole kinds'
    // codes are `RD_KINDS`' indexes (a quote's kinds after the one before's, its flags' subsets in order).
    let nb = rn.free.len() + 1;
    if rn.quotes != 2 || nb > 4 {
        return Err(format!("a condition at line {}", line!()));
    }
    let full = (1u8 << nb) - 1;
    let mut parts: Vec<(u8, u8)> = (0..=full).map(|v| (full, v)).collect();
    for known in 1..full {
        parts.extend((0..=full).filter(|v| v & !known == 0).map(|v| (known, v)));
    }
    let mut code_of = [255u8; 256];
    for (c, &(k, v)) in parts.iter().enumerate() {
        code_of[usize::from(k << 4 | v)] = c as u8;
    }
    let code = |k: u8, v: u8| if k == 0 { 252 } else { code_of[usize::from(k << 4 | v)] };
    let splits: Vec<String> = parts
        .iter()
        .copied()
        .chain([(0, 0)])
        .map(|(k, v)| format!("[{}]", (0..nb).map(|b| if k >> b & 1 == 1 { let c = code(k, v); format!("[{c}, {c}]") } else { format!("[{}, {}]", code(k | 1 << b, v), code(k | 1 << b, v | 1 << b)) }).collect::<Vec<_>>().join(", ")))
        .collect();
    let first_format = places.iter().position(|&(_, f)| f & rn.format != 0).ok_or("no format string")?;
    let regions: Vec<(usize, usize, usize, usize)> = regions.into_iter().flat_map(|(row, k, by_kind)| -> Vec<(usize, usize, usize, usize)> {
        match by_kind {
            Some(true) => (0..places.len()).map(|p| (p, row, k, 0)).collect(),
            Some(false) => vec![(first_format, row, k, 0)],
            None => vec![(usize::MAX, row, k, 0)],
        }
    }).collect();
    let names: Vec<&str> = live.iter().map(|fl| fl.name.as_str()).collect();
    let struct_fields: String = live.iter().map(|fl| format!("    pub {}: {},\n", fl.name, fl.ty)).collect();
    let inits: String = live.iter().map(|fl| format!("{}: {}, ", fl.name, fl.init)).collect();
    let muts: String = names.iter().map(|n| format!("mut {n}, ")).collect();
    let locals: String = dead.iter().map(|fl| format!("    let mut {}: {} = {};\n", fl.name, fl.ty, fl.init)).collect();
    let t = swap_section(TEMPLATE, "// @NEST{", "// }NEST@", READER_NEST)?;
    let t = swap_section(&t, "// @STAGE1{", "// }STAGE1@", NO_STAGE1)?;
    let t = with_n(&t);
    let kinds_s = std::iter::once("255".to_owned()).chain((0..interp.len()).map(|i| i.to_string())).collect::<Vec<_>>().join(", ");
    let t = t
        .replace("@FIELDS@", &struct_fields)
        .replace("@INITS@", &inits)
        .replace("@MUTS@", &muts)
        .replace("@BACK@", &names.join(", "))
        .replace("@LOCALS@", &locals)
        .replace("@LOOP@", &lp)
        .replace("@FINISH@", &finish)
        .replace("@ALL@", &names.iter().map(|n| format!("{n}, ")).collect::<String>())
        .replace("@NREGIONS@", &regions.len().to_string())
        .replace("@REGIONS@", &regions.iter().map(|(o, r, k, lv)| format!("({}, {r}, {k}, {lv})", if *o == usize::MAX { "usize::MAX".to_owned() } else { o.to_string() })).collect::<Vec<_>>().join(", "))
        .replace("@NRDKINDS@", &interp.len().to_string())
        .replace("@RDKINDS@", &table(&interp))
        .replace("@NPARTS@", &parts.len().to_string())
        .replace("@PARTS@", &parts.iter().map(|(k, v)| format!("({k:#04x}, {v:#04x})")).collect::<Vec<_>>().join(", "))
        .replace("@NFREE@", &rn.free.len().to_string())
        .replace("@FREE@", &rn.free.iter().map(|f| format!("{f:#04x}")).collect::<Vec<_>>().join(", "))
        .replace("@QBIT@", &format!("{:#04x}", 1u8 << rn.free.len()))
        .replace("@CODEOF@", &code_of.iter().map(u8::to_string).collect::<Vec<_>>().join(", "))
        .replace("@NBITS@", &nb.to_string())
        .replace("@NSPLITS@", &splits.len().to_string())
        .replace("@SPLITS@", &splits.join(", "))
        .replace("@FORMAT@", &format!("{:#04x}", rn.format))
        .replace("@NRDPLACES@", &places.len().to_string())
        .replace("@RDPLACES@", &table(&places))
        .replace("@OPENBYTE@", &format!("{:#04x}", rn.open_byte))
        .replace("@WS@", &ws.to_string());
    Ok(t + &core_with(maxd, &kinds_s, interp.len() + 1, 128, false, true))
}

/// `t` with the section from the line `open` to the line `close` (both kept out) `with`.
fn swap_section(t: &str, open: &str, close: &str, with: &str) -> Result<String, String> {
    let a = t.find(open).ok_or_else(|| format!("no {open}"))?;
    let z = a + t[a..].find(close).ok_or_else(|| format!("no {close}"))? + close.len();
    Ok(format!("{}{}{}", &t[..a], with, &t[z..]))
}

/// The template's functions of the source with its length apart (`src` padded, its bytes `n`, as `lex(buf, n)`'s).
fn with_n(t: &str) -> String {
    let mut t = t.to_owned();
    for name in ["lex_range", "finish", "stage1_at", "code_entry", "lanes_from", "chunk_ends", "summaries", "serial_entries", "lex_chunked"] {
        t = t.replace(&format!("fn {name}(src: &[u8],"), &format!("fn {name}(src: &[u8], n: usize,"));
        t = t.replace(&format!("fn {name}<const WRITE: bool, const ONE: bool>(src: &[u8],"), &format!("fn {name}<const WRITE: bool, const ONE: bool>(src: &[u8], n: usize,"));
        t = t.replace(&format!(" {name}(src, "), &format!(" {name}(src, n, "));
        t = t.replace(&format!("({name}(src, "), &format!("({name}(src, n, "));
        for g in ["false, false", "false, true", "true, false", "true, true"] {
            t = t.replace(&format!("{name}::<{g}>(src, "), &format!("{name}::<{g}>(src, n, "));
        }
    }
    let t: String = t.lines().filter(|l| l.trim() != "let n = src.len();").map(|l| format!("{l}\n")).collect();
    t.replace("src.len()", "n")
}

/// The string reader's rows at a template's level 0 (`rd_pick`'s): its tail, its spec, its conversion.
fn reader_picked(body: &str) -> Result<Vec<usize>, String> {
    let p = body.find("let rd_pick = if rd_on && rd_level == 0 {").ok_or("no `rd_pick`")?;
    let pz = p + body[p..].find("        };").ok_or("`rd_pick`'s end")?;
    Ok(body[p..pz].lines().filter_map(|l| l.trim().parse().ok()).collect())
}

/// The string reader's rows a chunk can start inside, by the token loop's rows (`body`): each its row, the bytes its
/// search starts past the token's start, and whether it reads the string's kind (the tail's, `Some(true)`), its
/// interpolation's alone (a spec's, a conversion's: `Some(false)`), or neither (a comment's, a search to its end).
fn reader_regions(body: &str, rn: &ReaderNest) -> Result<Vec<(usize, usize, Option<bool>)>, String> {
    let _ = rn;
    let mut out = Vec::new();
    // The reader's rows at a template's level 0: its tail, its spec, its conversion (`rd_pick`), in that order.
    let p = body.find("let rd_pick = if rd_on && rd_level == 0 {").ok_or("no `rd_pick`")?;
    let pz = p + body[p..].find("        };").ok_or("`rd_pick`'s end")?;
    let picked: Vec<usize> = body[p..pz].lines().filter_map(|l| l.trim().parse().ok()).collect();
    for (i, &row) in picked.iter().enumerate() {
        out.push((row, if i == 1 { 2 } else { 1 }, Some(i == 0)));
    }
    // A row whose arm is a search to its end from a byte past its start: a comment's.
    let rm = body.find("            let row = if rd_pick != usize::MAX { rd_pick } else { match group {").ok_or("no row match")?;
    let mut at = rm;
    while let Some(i) = body[at..].find("\n                ") {
        let a = at + i + "\n                ".len();
        at = a;
        let Some((r, rest)) = body[a..].split_once(" => {\n") else { break };
        let Ok(row) = r.parse::<usize>() else { continue };
        let first = rest.lines().map(str::trim).find(|l| !l.starts_with("//")).unwrap_or("");
        if let Some(k) = first.strip_prefix("let l = wfirst_bit(n, s + ").and_then(|r| r.split(',').next()).and_then(|k| k.parse::<usize>().ok())
            && first.ends_with(".unwrap_or(n) - 1;")
            && !out.iter().any(|x| x.0 == row)
        {
            out.push((row, k, None));
        }
    }
    if picked.len() < 1 {
        return Err("no tail".into());
    }
    Ok(out)
}

/// The loop's product body with its writes on where `WRITE` (pass 2 alone), its first token forced where given
/// (`Force`), and its nest's changes and reads made on the levels a lane enters with (`Reads::sh`, `READER_NEST`): each
/// step of `c_depth` a level pushed or popped, and the reader's reads of its nest checked where it makes them.
fn instrument_reader(body: &str, free: &[u8], picked: &[usize]) -> Result<String, String> {
    let lines: Vec<&str> = body.lines().collect();
    let mut out = String::new();
    let mut in_tail = false;
    // In a reader's row at its template's level 0 (`picked`), before its first statement past its checks of the token
    // that opens it (its `}`, `:` or `!`): a forced first token (a region's, `Force`) has none to check.
    let mut arm_head = false;
    let (mut guarded, mut after_row, mut seen) = (0, false, std::collections::BTreeMap::<&str, usize>::new());
    let mut prev = "";
    for l in &lines {
        let t = l.trim_start();
        let pad = &l[..l.len() - t.len()];
        let mut hit = |k: &'static str| *seen.entry(k).or_default() += 1;
        if t == "unsafe { out.add(len).write(rec) };" || t == "extras.push(rec);" || t.starts_with("unsafe { out.add(len + 1 + more).write(") {
            guarded += 1;
            let _ = writeln!(out, "{pad}if WRITE {{\n{pad}    {t}\n{pad}}}");
            prev = t;
            continue;
        }
        if pad.len() == 16 && let Some(r) = t.strip_suffix(" => {").and_then(|r| r.parse::<usize>().ok()) {
            arm_head = picked.contains(&r);
        } else if arm_head && !(t.is_empty() || t.starts_with("//") || t.starts_with("if ") || matches!(t, "let l = {" | "more = 0;" | "refused = true;" | "break 'tokens;" | "}" | "let (qi, fl, seg, o1) = (rd_q, rd_fl, s, s + 1);")) {
            arm_head = false;
        }
        if arm_head && t.starts_with("if ") && t.ends_with(" {") {
            let _ = writeln!(out, "{pad}if !forced && ({}) {{", &t[3..t.len() - 2]);
            prev = t;
            continue;
        }
        if !t.is_empty() && !t.starts_with("//") {
            match t {
                "'tokens: while b < nw {" => {
                    hit("loop");
                    let _ = writeln!(out, "{pad}'tokens: while b < nw.min(to) {{");
                }
                "while firsts != 0 {" if pad.len() == 8 => {
                    hit("firsts");
                    let _ = writeln!(out, "{pad}while firsts != 0 || force.is_some() {{");
                }
                "let s = base + firsts.trailing_zeros() as usize;" if pad.len() == 8 => {
                    hit("s");
                    let _ = writeln!(out, "{pad}let s = match force {{\n{pad}    Some(f) => f.s,\n{pad}    None => base + firsts.trailing_zeros() as usize,\n{pad}}};");
                }
                "let g_depth = c_depth;" => {
                    hit("g_depth");
                    let _ = writeln!(out, "{l}\n{pad}// The count over the gap 0: of the entry's, nothing below.\n{pad}if g_depth == 0 {{\n{pad}    reads.bottom = true;\n{pad}}}");
                }
                "let rd_pick = if rd_on && rd_level == 0 {" => {
                    hit("pick");
                    let _ = writeln!(out, "{pad}rd_top(reads);\n{l}");
                }
                "let row = if rd_pick != usize::MAX { rd_pick } else { match group {" => {
                    hit("row");
                    after_row = true;
                    let _ = writeln!(out, "{pad}let row = if let Some(f) = force {{ f.row }} else if rd_pick != usize::MAX {{ rd_pick }} else {{ match group {{");
                }
                "} };" if after_row && pad.len() == 12 => {
                    after_row = false;
                    let _ = writeln!(out, "{l}\n{pad}let forced = force.is_some();\n{pad}force = None;");
                }
                "rd_end = usize::MAX;" if pad.len() == 8 && prev.starts_with("at = ") => {
                    hit("one");
                    let _ = writeln!(out, "{l}\n{pad}if ONE {{\n{pad}    break 'tokens;\n{pad}}}");
                }
                "c_depth += 1;" => {
                    hit("push");
                    // A bracket's (the scan of a token's brackets), else an interpolation's.
                    // A bracket's; the next interpolation of the string a tail's `}` ended one of (its kind that one's, as far
                    // as read); a string's first.
                    if prev.contains("matches!(src[i]") {
                        let _ = writeln!(out, "{pad}sh_push(reads, 255);\n{l}");
                    } else if in_tail {
                        let _ = writeln!(out, "{pad}sh_push(reads, reads.popped);\n{l}");
                    } else {
                        let _ = writeln!(out, "{pad}sh_push(reads, rd_kind(rd_q, rd_fl));\n{l}");
                    }
                }
                "c_depth = c_depth.saturating_sub(1);" => {
                    hit("pop");
                    let _ = writeln!(out, "{pad}sh_pop(reads);\n{l}");
                }
                "let (qi, fl, seg, o1) = (rd_q, rd_fl, s, s + 1);" => {
                    hit("tail");
                    in_tail = true;
                    let _ = writeln!(out, "{pad}rd_frame(reads);\n{l}");
                }
                _ if in_tail && pad.len() == 16 && t.ends_with(" => {") => {
                    in_tail = false;
                    let _ = writeln!(out, "{l}");
                }
                _ if in_tail && (t.contains("match qi {") || free.iter().any(|f| t.contains(&format!("fl & {f:#04x}")))) => {
                    let mut x = l.replace("match qi {", "match rd_quote(reads, qi) {");
                    for f in free {
                        x = x.replace(&format!("fl & {f:#04x}"), &format!("rd_flag(reads, fl, {f:#04x})"));
                    }
                    let _ = writeln!(out, "{x}");
                }
                _ if t.contains("rd_code == usize::MAX") || t.contains("rd_code != usize::MAX") => {
                    hit("code");
                    let _ = writeln!(out, "{pad}rd_code_read(reads);\n{l}");
                }
                _ if t.starts_with("rd_code = ") => {
                    let _ = writeln!(out, "{l}\n{pad}reads.set |= 2;\n{pad}reads.code_known = true;");
                }
                _ if t.starts_with("rd_open = ") || t.contains("rd_open) = (") => {
                    let _ = writeln!(out, "{l}\n{pad}reads.set |= 1;");
                }
                _ => {
                    let _ = writeln!(out, "{l}");
                }
            }
            prev = t;
        } else {
            let _ = writeln!(out, "{l}");
        }
    }
    for k in ["loop", "firsts", "s", "g_depth", "pick", "row", "one", "push", "pop", "tail", "code"] {
        if !seen.contains_key(k) {
            return Err(format!("the reader's loop: no `{k}`"));
        }
    }
    if after_row {
        return Err("the row match's end".into());
    }
    let writes = body.matches("out.add(").count() + body.matches("extras.push(").count();
    if writes != guarded {
        return Err(format!("{writes} writes of records, {guarded} of them pass 2's"));
    }
    Ok(out)
}

/// A loop with no stage 1 (its blocks' masks made where it reads them) carries none from chunk to chunk.
const NO_STAGE1: &str = r#"
/// No stage 1 runs before the loop: a chunk carries none of it.
struct Stage1;

impl Stage1 {
    fn new(_: usize) -> Stage1 {
        Stage1
    }
}

struct Stage1Run;

impl Stage1Run {
    fn new() -> Stage1Run {
        Stage1Run
    }
}

/// None: no blocks before a chunk's first are read again.
pub const PRIME: usize = 0;

fn stage1_at(_src: &[u8], _n: usize, _s1: &mut Stage1, _from: usize) -> Stage1Run {
    Stage1Run
}
"#;

/// The nest of a loop whose nest is its string reader's state (`chunked_reader`), as levels: brackets (255) and
/// interpolations (their string's kind, `RD_KINDS`' index). `c_depth` counts them all; the brackets below the outermost
/// interpolation are the code's, those above one its code's (`rd_level`, `rd_total`). A lane's level of no kind yet
/// (254) is a bracket or an interpolation, an interpolation of no kind yet 252.
const READER_NEST: &str = r#"
/// A bracket.
const NB: usize = usize::MAX;

/// The kinds of string an interpolation is in (its quote, its flags), a level's kind its index: its bits (`PARTS`).
const RD_KINDS: [(u8, u8); @NRDKINDS@] = [@RDKINDS@];

/// A level's kind of interpolation as far as a lane read it: which of its bits it knows, and their values (bit `i` the
/// flag `FREE[i]`, the top one its quote). Its whole kinds first (`RD_KINDS`', all bits known), then those of some
/// known; 252 none.
const PARTS: [(u8, u8); @NPARTS@] = [@PARTS@];
const FREE: [u8; @NFREE@] = [@FREE@];
const QUOTE_BIT: u8 = @QBIT@;
const ALL_BITS: u8 = (QUOTE_BIT << 1) - 1;
/// A kind's code by its known bits and their values (`known << 4 | values`).
const CODE_OF: [u8; 256] = [@CODEOF@];
/// A kind's two kinds by the value of bit `i` (`split_kinds`), 252's last.
const SPLITS: [[[u8; 2]; @NBITS@]; @NSPLITS@] = [@SPLITS@];

/// The bits a kind knows, and their values.
#[inline]
fn part(k: u8) -> (u8, u8) {
    PARTS.get(usize::from(k)).copied().unwrap_or((0, 0))
}

/// The kind of known bits `known`, values `v`: 252 where none is known.
#[inline]
fn part_code(known: u8, v: u8) -> u8 {
    if known == 0 { 252 } else { CODE_OF[usize::from(known << 4 | v & known)] }
}

/// The string a kind is in, its unknown bits none: its quote, its flags.
#[inline]
fn whole(k: u8) -> (u8, u8) {
    let (_, v) = part(k);
    let fl = FREE.iter().enumerate().filter(|(i, _)| v >> i & 1 == 1).fold(@FORMAT@, |a, (_, b)| a | b);
    (u8::from(v & QUOTE_BIT != 0), fl)
}

/// A level a lane leaves of no kind, or an interpolation not whole: one it entered with, as it was.
#[inline]
fn wild(k: u8) -> bool {
    k == 254 || k != 255 && part(k).0 != ALL_BITS
}

/// The kind of level both `k` and `m` are (254: any; an interpolation, those of its known bits), none where none is.
#[inline]
fn meet(k: u8, m: u8) -> Option<u8> {
    if k == m || k == 254 {
        return Some(m);
    }
    if m == 254 {
        return Some(k);
    }
    if k == 255 || m == 255 {
        return None;
    }
    let ((kk, kv), (mk, mv)) = (part(k), part(m));
    if (kv ^ mv) & kk & mk != 0 {
        return None;
    }
    Some(part_code(kk | mk, kv | mv))
}

/// Every kind of string: where a chunk can start in its text (`REGIONS`' `open`).
const RD_PLACES: [(u8, u8); @NRDPLACES@] = [@RDPLACES@];

/// The interpolation opener's byte.
const OPEN_BYTE: u8 = @OPENBYTE@;

/// The whitespace class's bits from `p` (`w_m_ws`, as a loop with stage 1 reads them): the loop's own, of `p`'s block.
#[inline(always)]
unsafe fn w_m_ws(src: &[u8], n: usize, p: isize, _esc: &mut u64) -> u64 {
    if p < 0 || p as usize >= n {
        return 0;
    }
    let base = p as usize & !63;
    (unsafe { block_classes(src, n, base) })[@WS@] >> (p as usize - base)
}

/// An interpolation's kind of string: its level's kind (251: none, a string no interpolation is in).
#[inline]
fn rd_kind(q: u8, fl: u8) -> u8 {
    RD_KINDS.iter().position(|&k| k == (q, fl)).map_or(251, |i| i as u8)
}

impl Entry {
    /// The nest as levels: the count's brackets below the reader's frames, then each frame (its kind, its code's
    /// brackets).
    #[inline]
    fn levels(&self) -> ([usize; MAXD], [usize; MAXD], usize) {
        let mut nest = [NB; MAXD];
        let d = (self.c_depth as usize).min(MAXD);
        let frames = if self.rd_on { self.rd_depth + 1 } else { 0 };
        let mut i = d.saturating_sub(usize::from(self.rd_total));
        for f in 0..frames {
            let (q, fl, lv) = if f < self.rd_depth { self.rd_around[f] } else { (self.rd_q, self.rd_fl, self.rd_level) };
            if i < MAXD {
                nest[i] = usize::from(rd_kind(q, fl));
            }
            i += 1 + usize::from(lv);
        }
        (nest, [0; MAXD], d)
    }

    /// `self` with the nest `nest[..depth]`: the reader's state of its frames, the count its depth. A level of no kind
    /// yet is a bracket; a bracket above an interpolation one of its code's.
    #[inline]
    fn set_levels(&mut self, nest: &[usize; MAXD], _held: &[usize; MAXD], _opened: usize, depth: usize) {
        let (mut fr, mut nf, mut total) = ([(0u8, 0u8, 0u8); MAXD], 0usize, 0u8);
        for &v in &nest[..depth] {
            match kind_of(v) {
                255 | 254 => {
                    if nf > 0 {
                        fr[nf - 1].2 = fr[nf - 1].2.saturating_add(1);
                        total = total.saturating_add(1);
                    }
                }
                k => {
                    let (q, fl) = whole(k);
                    fr[nf] = (q, fl, 0);
                    nf += 1;
                    total = total.saturating_add(1);
                }
            }
        }
        self.c_depth = depth as _;
        self.rd_on = nf > 0;
        (self.rd_q, self.rd_fl, self.rd_level) = if nf > 0 { fr[nf - 1] } else { (0, 0, 0) };
        self.rd_depth = nf.saturating_sub(1);
        self.rd_around = Entry::START.rd_around;
        for f in 0..self.rd_depth.min(self.rd_around.len()) {
            self.rd_around[f] = fr[f];
        }
        self.rd_total = total;
    }

    /// The values a lane keeps of its entry (`Lane::keep`), `from`'s: the interpolation's `{` (1), where its code began
    /// (2).
    #[inline]
    fn carry(&mut self, from: &Entry, keep: u8) {
        if keep & 1 != 0 {
            self.rd_open = from.rd_open;
        }
        if keep & 2 != 0 {
            self.rd_code = from.rd_code;
        }
    }

    /// What a lane assumes of its entry: its interpolation's `{` the byte before it where that is one (no other is read
    /// before the lane sets it: an empty interpolation's `}` is the byte after), where its code began none (a read of it
    /// is checked, `rd_code_read`).
    #[inline]
    fn lane_entry(&mut self, src: &[u8]) {
        self.rd_open = if self.at > 0 && src[self.at - 1] == OPEN_BYTE { self.at - 1 } else { usize::MAX - 1 };
        self.rd_code = usize::MAX;
    }

    /// The entry a region's forced first token is read from: in a string of kind `RD_PLACES[open]`, its interpolation
    /// the one open.
    #[inline]
    fn region_entry(&mut self, open: usize) {
        if let Some(&(q, fl)) = RD_PLACES.get(open) {
            (self.rd_on, self.rd_q, self.rd_fl, self.rd_level, self.rd_total, self.c_depth) = (true, q, fl, 0, 1, 1);
            (self.rd_open, self.rd_code) = (usize::MAX - 1, usize::MAX);
        }
    }

    /// The frames past those open as at the start: no read reaches them before a write.
    #[inline]
    fn canonical(&mut self) {
        let k = if self.rd_on { self.rd_depth } else { 0 };
        for f in k..self.rd_around.len() {
            self.rd_around[f] = Entry::START.rd_around[f];
        }
    }

    /// `self` with its nest as at the start: a lane's exit, whose nest its `p` holds.
    #[inline]
    fn without_nest(mut self) -> Entry {
        self.set_levels(&[NB; MAXD], &[0; MAXD], 0, 0);
        self
    }
}

/// What a lane read of the nest below where it entered (`chunked_nest`'s), and the levels it holds as it goes (`sh`,
/// `shd`: those it entered with, then its own): a read of a level of no kind yet asks its kind, one of none (the
/// count 0 at the bottom of what it entered with) the bottom.
#[derive(Clone, Copy, Debug)]
pub struct Reads {
    pub bottom: bool,
    pub kind_of: Option<usize>,
    pub low: usize,
    pub high: usize,
    pub set: u8,
    pub lost: bool,
    sh: [u8; MAXD],
    shd: usize,
    /// Whether where the interpolation's code began is the run's own or known none.
    code_known: bool,
    /// The level of the interpolation a tail reads (`rd_frame`), and the bit of its kind it read unknown (`split_kinds`).
    fd: usize,
    bit: u8,
    /// The kind of the level popped last (254: one below those it held).
    popped: u8,
}

impl Reads {
    /// A run from an entry whose nest is given whole: its levels from it (`enter`).
    const NONE: Reads = Reads { bottom: false, kind_of: None, low: 0, high: 0, set: 0, lost: false, sh: [255; MAXD], shd: usize::MAX, code_known: true, fd: 0, bit: 0, popped: 254 };

    #[inline]
    fn lane(k: &[u8; MAXD], kd: usize) -> Reads {
        Reads { low: kd, high: kd, sh: *k, shd: kd, code_known: false, ..Reads::NONE }
    }

    #[inline]
    fn enter(&mut self, e: &Entry) {
        if self.shd == usize::MAX {
            let (nest, _, d) = e.levels();
            for i in 0..d {
                self.sh[i] = kind_of(nest[i]);
            }
            (self.shd, self.low, self.high) = (d, d, d);
        }
    }
}

/// A level pushed: `c_depth` up.
#[inline(always)]
fn sh_push(r: &mut Reads, k: u8) {
    if r.shd < MAXD {
        r.sh[r.shd] = k;
    }
    r.shd += 1;
    r.high = r.high.max(r.shd.min(MAXD));
}

/// A level popped: `c_depth` down, none below the bottom.
#[inline(always)]
fn sh_pop(r: &mut Reads) {
    if r.shd == 0 {
        r.bottom = true;
        r.popped = 254;
    } else {
        r.shd -= 1;
        r.low = r.low.min(r.shd);
        r.popped = if r.shd < MAXD { r.sh[r.shd] } else { 254 };
    }
}

/// A read of the top level's class (`rd_on`, `rd_level == 0`: an interpolation, of any kind): none open, the bottom;
/// of no kind yet, whether it is one. The lane's class holds each level it read (`low`).
#[inline(always)]
fn rd_top(r: &mut Reads) {
    if r.shd == 0 {
        r.bottom = true;
    } else {
        r.low = r.low.min(r.shd - 1);
        if r.sh[r.shd - 1] == 254 && r.kind_of.is_none() {
            r.kind_of = Some(r.shd - 1);
        }
    }
}

/// A tail's read of its interpolation (`rd_q`, `rd_fl`), below the top's brackets of its code: its level, read
/// bit by bit (`rd_flag`, `rd_quote`).
#[inline(always)]
fn rd_frame(r: &mut Reads) {
    let mut d = r.shd.min(MAXD);
    while d > 0 && r.sh[d - 1] == 255 {
        d -= 1;
    }
    if d == 0 {
        r.bottom = true;
        r.low = 0;
        r.fd = usize::MAX;
    } else {
        r.low = r.low.min(d - 1);
        r.fd = d - 1;
        if r.sh[d - 1] == 254 && r.kind_of.is_none() {
            r.kind_of = Some(d - 1);
            r.bit = 0;
        }
    }
}

/// A read of bit `b` of the interpolation's kind (`rd_frame`'s): one its level's kind does not know, its kind by it.
#[inline(always)]
fn rd_bit(r: &mut Reads, b: u8) {
    if r.fd == usize::MAX || r.kind_of.is_some() {
        return;
    }
    let k = r.sh[r.fd];
    if k != 254 && part(k).0 & 1 << b == 0 {
        r.kind_of = Some(r.fd);
        r.bit = b;
    }
}

/// The tail's read of its flag `f` (`fl & f`).
#[inline(always)]
fn rd_flag(r: &mut Reads, fl: u8, f: u8) -> u8 {
    if let Some(b) = FREE.iter().position(|&x| x == f) {
        rd_bit(r, b as u8);
    }
    fl & f
}

/// The tail's read of its quote.
#[inline(always)]
fn rd_quote(r: &mut Reads, qi: u8) -> u8 {
    rd_bit(r, QUOTE_BIT.trailing_zeros() as u8);
    qi
}

/// A read of where the interpolation's code began (`rd_code`), not set by the run: not given by the lane's levels
/// (`lost`), but by its class of it (`Lane::cond`, `enter_cond`).
#[inline(always)]
fn rd_code_read(r: &mut Reads) {
    if !r.code_known {
        r.lost = true;
    }
}

/// The kinds a level a lane read the kind of is: of no kind yet, a bracket or an interpolation; an interpolation, its
/// two kinds by the bit read (`Reads::bit`).
#[inline]
fn split_kinds(k: u8, r: &Reads) -> &'static [u8] {
    match k {
        254 => &[255, 252],
        255 => &[],
        k => {
            let i = if k == 252 { PARTS.len() } else { usize::from(k) };
            &SPLITS[i][usize::from(r.bit)]
        }
    }
}

/// Any level can be under any other (a bracket's, the code's or an interpolation's code's, is where it is).
#[inline]
fn may_hold(_below: u8, _above: u8) -> bool {
    true
}

/// Where the interpolation's code began (`rd_code`, kept: `carry`'s 2), read by class (`Lane::cond`): none (1), or one
/// before the lane (2: its reads of it from the lane's start, a part of the serial lexer's, which pass 2 reads whole).
const COND_KEEP: u8 = 2;
const CONDS: &[u8] = &[1, 2];

#[inline]
fn enter_cond(e: &mut Entry, r: &mut Reads, cond: u8) {
    match cond {
        1 => (e.rd_code, r.code_known) = (usize::MAX, true),
        2 => (e.rd_code, r.code_known) = (e.at, true),
        _ => {}
    }
}

impl Entry {
    #[inline]
    fn cond(&self) -> u8 {
        if self.rd_code == usize::MAX { 1 } else { 2 }
    }
}

/// The nest a lane's run leaves: its levels as it went.
#[inline]
fn exit_levels(_e: &Entry, r: &Reads) -> ([usize; MAXD], [usize; MAXD], usize) {
    let d = r.shd.min(MAXD);
    let mut nest = [NB; MAXD];
    for i in 0..d {
        nest[i] = level_of(r.sh[i]);
    }
    (nest, [0; MAXD], d)
}
"#;

/// The three passes' summary for nesting, one for every token loop's chunked form (`chunked_nest`'s, `chunked_lean`'s):
/// lanes by the class of nest below their entry, their composition as typed bracket sequences, and pass 3's prefix sum.
/// A loop's form gives its `Entry` (its `at`, its nest's fields `nest`, `depth`, `template`, `held`, `opened`, and
/// `START`), and makes its chunks' summaries.
/// The kinds' operations of a nest whose levels are of one kind each (`chunked_nest`'s, `chunked_lean`'s): 254 any, 252
/// any below 252.
const KIND_OPS: &str = r#"
/// A level a lane leaves of no kind (open, or an interpolation of none): one it entered with, as it was.
#[inline]
fn wild(k: u8) -> bool {
    k == 254 || k == 252
}

/// The kind of level both `k` and `m` are (254: any; 252: any of the kinds below 252), none where none is.
#[inline]
fn meet(k: u8, m: u8) -> Option<u8> {
    if k == m || k == 254 || k == 252 && m < 252 {
        Some(m)
    } else if m == 254 || m == 252 && k < 252 {
        Some(k)
    } else {
        None
    }
}
"#;

/// The three passes' summary (`CORE`) for a nest of `maxd` levels of kinds `kinds` (`nkinds` of them), `l` lanes a
/// chunk; the kinds' operations (`wild`, `meet`) `KIND_OPS`, where `own_kinds` the nest's own.
fn core_with(maxd: usize, kinds: &str, nkinds: usize, l: usize, code_anywhere: bool, own_kinds: bool) -> String {
    core(maxd, kinds, nkinds, l, code_anywhere).replace("@KIND_OPS@", if own_kinds { "" } else { KIND_OPS })
}

fn core(maxd: usize, kinds: &str, nkinds: usize, l: usize, code_anywhere: bool) -> String {
    CORE.replace("@MAXD@", &maxd.to_string()).replace("@NKINDS@", &nkinds.to_string()).replace("@KINDS@", kinds).replace("@L@", &l.to_string()).replace("@CODE_ANYWHERE@", &code_anywhere.to_string())
}

const CORE: &str = r#"
/// A level the lane's entry has open, of a kind it has read none of yet: a bracket's or a template's.
pub const OPEN: usize = usize::MAX - 1;

/// A string's record a lane keeps from its entry (a tail resumes the string of the level it closes): `INHERIT + i`, the
/// `i`th level the lane closes (innermost first), where a lane's own are the records it counts.
pub const INHERIT: usize = 1 << 62;

/// The deepest the nest goes (the serial lexer refuses past it), and the kinds a level can be: a bracket's (255), then
/// each template's string kind.
pub const MAXD: usize = @MAXD@;
pub const LEVEL_KINDS: [u8; @NKINDS@] = [@KINDS@];

/// Whether an entry in a range's code that no lane starts at goes on in its code lanes: where the loop's tokens from any
/// byte of the code come to the same place (rr's, whose regions alone hold their openers), the code lane's place.
const CODE_ANYWHERE: bool = @CODE_ANYWHERE@;

/// The lanes a summary holds at most; a chunk that needs more is unresolved (`Summary::over`): pass 2 enters it from the
/// entry the chunks before it leave.
pub const L: usize = @L@;

/// A level's kind in a lane (`LEVEL_KINDS`, or 254: open, of no kind yet), its value in `nest`.
#[inline]
fn kind_of(k: usize) -> u8 {
    match k {
        usize::MAX => 255,
        OPEN => 254,
        k => k as u8,
    }
}

#[inline]
fn level_of(k: u8) -> usize {
    match k {
        255 => usize::MAX,
        254 => OPEN,
        k => usize::from(k),
    }
}

@KIND_OPS@
/// Whether a lane's level of kind `k` meets a level of kind `m`: every level of kind `m` is one of `k`.
#[inline]
fn meets(k: u8, m: u8) -> bool {
    meet(k, m) == Some(m)
}

/// One lane of a summary: what a range of chunks does for the entries at `at0` whose innermost `kcn` levels are `kc`
/// (innermost first), all of them where `exact` (it read the bottom: "nothing open" below them). It leaves `p[..pn]`
/// open (outermost first) above what is left of the entry, the strings of those that are templates at the records
/// `ph` (from the range's first), and goes `high` levels above what is left. `exit`'s `at` and kept values are its
/// end's; its nest fields are not read (`p` is).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lane {
    pub at0: usize,
    pub kc: [u8; MAXD],
    pub kcn: u8,
    pub exact: bool,
    /// The levels the entry has open below those the lane closes, which it never comes to: it holds for an entry with
    /// at least these (a lane with fewer is another's, the one that read the bottom). None where `exact`.
    pub need: u8,
    pub p: [u8; MAXD],
    pub pn: u8,
    pub ph: [usize; MAXD],
    pub high: u8,
    pub recs: usize,
    pub exit: Entry,
    pub refused: bool,
    /// Whether it lexes a token: one that lexes none leaves its entry as it was.
    pub moved: bool,
    /// The values of its entry it leaves as they were, which it never set (`Entry::carry`'s bits): its exit's are not
    /// read.
    pub keep: u8,
    /// What it read of a value its entry keeps (`COND_KEEP`'s): the entries it holds for, by `Entry::cond` (0: any).
    pub cond: u8,
}

/// A range's summary: its lanes (an entry past the range's head, its first chunk, at one of its later chunks' places,
/// the lanes of those); its bytes (`start..end`, from the first that lexes a token), an entry at or past whose end goes
/// through it as it came; where its code lanes start (`code`: past the blanks before `start`, the lanes for an entry
/// before it) and the end of their chunk (`head`); and
/// whether an entry may have gone on in a lane past `L` (`over`, how the range was made of its parts, not a fact of the
/// range: its lanes are those).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Summary {
    pub lanes: [Lane; L],
    pub n: u8,
    pub start: usize,
    pub end: usize,
    pub code: usize,
    pub head: usize,
    pub over: bool,
}

impl Summary {
    fn push(&mut self, l: Lane) {
        if self.lanes[..usize::from(self.n)].contains(&l) {
            return;
        }
        // The same lane for the other class of a kept value it read (`cond`: the classes are two): for every entry.
        if l.cond != 0
            && let Some(m) = self.lanes[..usize::from(self.n)].iter_mut().find(|m| m.cond != 0 && m.cond != l.cond && Lane { cond: l.cond, ..**m } == l)
        {
            m.cond = 0;
            return;
        }
        if usize::from(self.n) == L {
            self.over = true;
            return;
        }
        self.lanes[usize::from(self.n)] = l;
        self.n += 1;
    }

    pub fn lanes(&self) -> &[Lane] {
        &self.lanes[..usize::from(self.n)]
    }

    /// Whether no lane lexes a token: every entry goes through it as it came (the chunk is blank past its code entry,
    /// or inside a region to past its end).
    pub fn transparent(&self) -> bool {
        self.n > 0 && !self.over && self.lanes().iter().all(|l| !l.moved)
    }
}

const NO_LANE: Lane = Lane { at0: 0, kc: [0; MAXD], kcn: 0, exact: false, need: 0, p: [0; MAXD], pn: 0, ph: [0; MAXD], high: 0, recs: 0, exit: Entry::START, refused: false, moved: false, keep: 0, cond: 0 };

/// The lanes of `b` a lane of the range before it goes on in: those entered where it leaves the next token, else the
/// code's (a token from before `b` ends in its code).
fn next_lanes(b: &Summary, at: usize) -> impl Iterator<Item = &Lane> {
    // An entry before the range's first byte comes through blanks to where its code lanes start.
    // (Where `b` dropped lanes past `L`, an entry with none may have had one: no code lane for it.)
    let at0 = if at < b.start || (CODE_ANYWHERE && !b.over && at < b.head && !b.lanes().iter().any(|l| l.at0 == at)) { b.code } else { at };
    b.lanes().iter().filter(move |l| l.at0 == at0)
}

/// Lane `a` then lane `b` (entered where `a` leaves), as typed bracket sequences: `b`'s closed levels matched against
/// what `a` leaves open, the rest of them closed below `a`'s entry. None where they do not meet.
fn then(a: &Lane, b: &Lane) -> Option<Lane> {
    // `a` refuses the file: nothing past it is read.
    if a.refused {
        return Some(*a);
    }
    // `b` lexes no token: what `a` leaves goes through it.
    if !b.moved {
        return Some(*a);
    }
    if !a.moved {
        let mut l = *b;
        l.at0 = a.at0;
        return Some(l);
    }
    let (an, bn, pn) = (usize::from(a.kcn), usize::from(b.kcn), usize::from(a.pn));
    let j = bn.min(pn);
    // `b`'s first `j` closed levels, innermost first, are `a`'s open ones, innermost last. One `a` leaves of no kind
    // (its entry's, `wild`) that `b` read the kind of: `a`'s class of it that kind (`refine`).
    let (mut refine, mut nr) = ([(0usize, 0u8); MAXD], 0);
    for i in 0..j {
        let (k, m) = (b.kc[i], a.p[pn - 1 - i]);
        match meet(k, m) {
            Some(x) if x == m => {}
            // (What `a`'s class knows of the level, which its wildcard does not say.)
            Some(_) if wild(m) && pn - 1 - i < an => {
                let at = an - 1 - (pn - 1 - i);
                let Some(x) = meet(k, a.kc[at]) else { return None };
                refine[nr] = (at, x);
                nr += 1;
            }
            _ => return None,
        }
    }
    let mut l = *b;
    l.at0 = a.at0;
    // What `b` keeps of its entry is `a`'s exit's, where `a` set it; what both keep, `a`'s entry's.
    l.keep = a.keep & b.keep;
    l.exit.carry(&a.exit, b.keep & !a.keep);
    // What `b` read of its entry's kept values (`cond`): `a`'s exit's where `a` set them, else `a`'s entry's.
    l.cond = a.cond;
    if b.cond != 0 {
        if a.keep & COND_KEEP == 0 {
            if a.exit.cond() != b.cond {
                return None;
            }
        } else if a.cond != 0 && a.cond != b.cond {
            return None;
        } else {
            l.cond = b.cond;
        }
    }
    if bn > pn {
        // `b` closes past what `a` left open: below `a`'s entry, where `a` read none (`exact`) there is none.
        // (No entry has more than `MAXD` levels open: the serial lexer refuses past them.)
        if a.exact || an + bn - pn > MAXD {
            return None;
        }
        l.kc = [0; MAXD];
        l.kc[..an].copy_from_slice(&a.kc[..an]);
        l.kc[an..an + bn - pn].copy_from_slice(&b.kc[pn..bn]);
        l.kcn = (an + bn - pn) as u8;
        l.exact = b.exact;
        l.need = b.need.max(a.need.saturating_sub((bn - pn) as u8));
    } else {
        // `b` read the bottom with levels `a` left open below its own: no such entry.
        if b.exact && pn > bn {
            return None;
        }
        l.kc = a.kc;
        l.kcn = a.kcn;
        l.exact = a.exact || b.exact;
        // `b` holds where `a` leaves at least `bn + b.need` open: `a`'s entry at least that less what `a` opens.
        l.need = a.need.max((usize::from(b.need) + bn).saturating_sub(pn) as u8);
    }
    for &(i, k) in &refine[..nr] {
        l.kc[i] = k;
    }
    // An entry with exactly `kn` levels open can have none more below.
    if l.exact && l.need > 0 {
        return None;
    }
    let kn = usize::from(l.kcn);
    // Open: `a`'s below what `b` closed, then `b`'s; `b`'s strings' records after `a`'s.
    let keep = pn - j;
    let mut p = [0u8; MAXD];
    let mut ph = [0usize; MAXD];
    p[..keep].copy_from_slice(&a.p[..keep]);
    ph[..keep].copy_from_slice(&a.ph[..keep]);
    let bp = usize::from(b.pn);
    if keep + bp > MAXD {
        return None;
    }
    for i in 0..bp {
        // A level `b` leaves of no kind is its entry's: `a`'s, where `a` left it.
        let ai = (i + pn).checked_sub(bn).filter(|&x| x < pn);
        p[keep + i] = match ai {
            Some(x) if wild(b.p[i]) => a.p[x],
            _ => b.p[i],
        };
        ph[keep + i] = if b.p[i] == 255 {
            0
        } else if b.ph[i] >= INHERIT {
            // A record `b` keeps from a level it closes: `a`'s, where `a` left it open; else one below `a`'s entry.
            let q = b.ph[i] - INHERIT;
            if q < j { a.ph[pn - 1 - q] } else { INHERIT + an + q - pn }
        } else {
            b.ph[i] + a.recs
        };
    }
    l.p = p;
    l.ph = ph;
    l.pn = (keep + bp) as u8;
    // The deepest, above the combined base (the entry less its `kn` closed levels): `a` starts `kn - an` above it, and
    // `b` where `a` leaves it, `kn + pn - an - bn` above it (none where `b` closes past `a`'s open levels).
    let ha = usize::from(a.high) + (kn - an);
    let hb = usize::from(b.high) + (kn + pn) - (an + bn);
    l.high = ha.max(hb) as u8;
    l.recs = a.recs + b.recs;
    l.refused = a.refused || b.refused;
    Some(l)
}

/// Two ranges' summaries, one after the other: each lane of the first, on through the lanes of the second it meets.
/// Associative, so a prefix sum composes them in any tree.
pub fn compose(a: &Summary, b: &Summary) -> Summary {
    let (start, end) = (a.start, b.end);
    if b.transparent() {
        return Summary { start, end, over: a.over || b.over, ..*a };
    }
    if a.transparent() {
        // Every entry goes through `a` as it came, to `b`: `b`'s lanes, an entry before `b` at its code lanes.
        let _ = start;
        return Summary { end, over: a.over || b.over, ..*b };
    }
    let mut s = Summary { lanes: [NO_LANE; L], n: 0, start, end, code: a.code, head: a.head, over: a.over };
    for la in a.lanes() {
        // A lane that leaves at or past `b`'s end goes through `b` as it is.
        if la.exit.at >= b.end {
            s.push(*la);
            continue;
        }
        let mut met = false;
        for lb in next_lanes(b, la.exit.at) {
            if let Some(l) = then(la, lb) {
                met = true;
                s.push(l);
            }
        }
        // A lane of `a` may go on in one `b` dropped.
        s.over |= !met && b.over;
    }
    // An entry at or past `a`'s end goes through `a` to `b`'s lanes there.
    for lb in b.lanes().iter().filter(|l| l.at0 >= a.end) {
        s.push(*lb);
    }
    s.over |= b.over && b.lanes().iter().any(|l| l.at0 >= a.end);
    s
}

/// The entry a range leaves after entry `e` (`e` at its first byte, the records before it `recs`): its lane for `e`'s
/// nest, and whether the file is refused on the way. None where no lane meets it (one past `L` would have).
pub fn apply(s: &Summary, e: &Entry, recs: usize) -> Option<(Entry, usize, bool)> {
    // Each lane holds for its class of entries, and the lanes of a range split them apart: a lane kept where others went
    // past `L` still holds, and no other lane meets its entries.
    if s.transparent() || e.at >= s.end {
        return Some((*e, recs, false));
    }
    let (en, eh, d) = e.levels();
    let l = next_lanes(s, e.at).find(|l| {
        let kn = usize::from(l.kcn);
        kn + usize::from(l.need) <= d && (0..kn).all(|i| meets(l.kc[i], kind_of(en[d - 1 - i]))) && (!l.exact || d == kn) && (l.cond == 0 || l.cond == e.cond())
    })?;
    if !l.moved {
        return Some((*e, recs, false));
    }
    let rest = d - usize::from(l.kcn);
    // Past the deepest nest: the serial lexer refuses the file there.
    if rest + usize::from(l.high) > MAXD || rest + usize::from(l.pn) > MAXD {
        return Some((Entry::START, recs + l.recs, true));
    }
    let mut x = l.exit;
    let strings = (0..rest).filter(|&i| en[i] != usize::MAX).count();
    let mut nest = [usize::MAX; MAXD];
    nest[..rest].copy_from_slice(&en[..rest]);
    let mut held = [0; MAXD];
    held[..strings].copy_from_slice(&eh[..strings]);
    let mut opened = strings;
    for i in 0..usize::from(l.pn) {
        // A level of no kind it leaves is one it entered with as it was: the entry's.
        nest[rest + i] = if wild(l.p[i]) && rest + i < d { en[rest + i] } else { level_of(l.p[i]) };
        if l.p[i] != 255 {
            held[opened] = if l.ph[i] >= INHERIT {
                // The string of the entry's level the lane closed `q`th: its record, the entry's.
                let lv = d - 1 - (l.ph[i] - INHERIT);
                eh[(0..lv).filter(|&i| en[i] != usize::MAX).count()]
            } else {
                recs + l.ph[i]
            };
            opened += 1;
        }
    }
    x.set_levels(&nest, &held, opened, rest + usize::from(l.pn));
    // What the lane keeps of its entry (`keep`): the entry's.
    x.carry(e, l.keep);
    Some((x, recs + l.recs, l.refused))
}

/// The chunks pass 3 has in flight at once (`entries`): each composes the summaries of those before it in flight.
pub const IN_FLIGHT: usize = 32;

/// Pass 3: each chunk's entry and the records before it, by a single-pass prefix sum with decoupled look-back (Merrill
/// and Garland's): a chunk whose predecessor's entry is known applies its predecessor's summary to it; one whose
/// predecessor is still in flight composes the summaries back to the last known entry and applies that, and where no
/// lane of the composition meets it (one went past `L`, `over`), waits for its predecessor's entry instead. None for a chunk no lane
/// reaches; and the chunks that waited.
pub fn entries_waits(ends: &[Summary]) -> (Vec<Option<(Entry, usize, bool)>>, usize) {
    let after = |out: &[Option<(Entry, usize, bool)>], c: usize| out[c - 1].and_then(|(e, r, x)| if x { None } else { apply(&ends[c - 1], &e, r) });
    let mut out: Vec<Option<(Entry, usize, bool)>> = Vec::with_capacity(ends.len());
    let mut waits = 0;
    let mut w = 0;
    while w < ends.len() {
        // The chunks `w..w + IN_FLIGHT` in flight. The last chunk whose entry is known (`k`: the wave's first, or one
        // that waited), and the summaries composed from it.
        let mut k = w;
        let mut known = if w == 0 { Some((Entry::START, 0, false)) } else { after(&out, w) };
        let top = (w + IN_FLIGHT).min(ends.len());
        let mut agg: Option<Summary> = None;
        out.push(known);
        for c in w + 1..top {
            let a = match agg {
                None => ends[c - 1],
                Some(a) => compose(&a, &ends[c - 1]),
            };
            agg = Some(a);
            // A lane that meets the entry is its own, whatever lanes went past `L` (a lane holds for its class alone).
            let e = match known {
                Some((ke, kr, _)) => apply(&a, &ke, kr),
                _ => None,
            };
            let e = match e {
                Some(e) => e,
                None => {
                    // Its predecessor's entry, when known: from it on, the chunks after look back to this one.
                    waits += 1;
                    let e = after(&out, c);
                    (k, known, agg) = (c, e, None);
                    out.push(e);
                    continue;
                }
            };
            let _ = k;
            out.push(Some(e));
        }
        w = top;
    }
    (out, waits)
}

/// Pass 3's entries (`entries_waits`).
pub fn entries(ends: &[Summary]) -> Vec<Option<(Entry, usize, bool)>> {
    entries_waits(ends).0
}

/// The first block of each chunk of `blocks` blocks of a file of `n` bytes.
pub fn chunk_starts(n: usize, blocks: usize) -> Vec<usize> {
    (0..n.div_ceil(64).max(1)).step_by(blocks.max(1)).collect()
}

"#;

const TEMPLATE: &str = r#"
// Lexing from the middle (the chunk-summaries design): the token loop's state between two tokens, the loop from any of
// them to any block, and the three passes. Pass 1 lexes each chunk from each class of entries (`chunk_ends`), pass 3
// composes the chunks' summaries by a prefix sum (`entries`), pass 2 lexes each chunk from its entry (`lex_chunked`).

/// The token loop's state between two tokens: where the next token starts (`at`), the nest (`nest[..depth]`, its top
/// `template`), the strings its templates hold open (`held[..opened]`, their records), and what the loop keeps of the
/// token before. The serial lexer's at each chunk's start is that chunk's entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
@FIELDS@}

impl Entry {
    /// The file's first byte: in code, nothing open.
    pub const START: Entry = Entry { @INITS@};
}

// @NEST{
impl Entry {
    /// The nest: each level's value (`usize::MAX` a bracket, else its string's kind), the records of its strings in
    /// order, and its depth.
    #[inline]
    fn levels(&self) -> ([usize; MAXD], [usize; MAXD], usize) {
        (self.nest, self.held, self.depth)
    }

    /// `self` with the nest `nest[..depth]`, its strings' records `held[..opened]`.
    #[inline]
    fn set_levels(&mut self, nest: &[usize; MAXD], held: &[usize; MAXD], opened: usize, depth: usize) {
        self.nest = *nest;
        self.held = *held;
        self.opened = opened;
        self.depth = depth;
        self.template = if depth == 0 { usize::MAX } else { nest[depth - 1] };
    }

    /// The values a lane keeps of its entry (`Lane::keep`), `from`'s: the loop keeps none apart from its nest.
    #[inline]
    fn carry(&mut self, _from: &Entry, _keep: u8) {}

    /// What a lane assumes of its entry where its nest is given (`lanes_from`): nothing more.
    #[inline]
    fn lane_entry(&mut self, _src: &[u8]) {}

    /// The entry a region's forced first token (`REGIONS`' `open`) is read from: the file's start's.
    #[inline]
    fn region_entry(&mut self, _open: usize) {}

    /// The levels above the nest's depth and the strings past those open as at the start: no read reaches them before
    /// a write, so two entries the loop goes on from alike are equal.
    #[inline]
    fn canonical(&mut self) {
        for d in self.depth..self.nest.len() {
            self.nest[d] = Entry::START.nest[d];
        }
        for h in self.opened..self.held.len() {
            self.held[h] = Entry::START.held[h];
        }
    }

    /// `self` with its nest and its strings as at the start: a lane's exit, whose nest its `p` holds.
    #[inline]
    fn without_nest(mut self) -> Entry {
        self.nest = Entry::START.nest;
        self.depth = Entry::START.depth;
        self.template = Entry::START.template;
        self.held = Entry::START.held;
        self.opened = Entry::START.opened;
        self
    }
}

/// What a lane read of the nest below where it entered: that it read the bottom (`bottom`: a bracket with none open, a
/// template's kind with none open, no string open), the kind of a level it entered with none (`kind_of`, the level), the
/// lowest level it came to (`low`), and the deepest (`high`).
#[derive(Clone, Copy, Debug)]
pub struct Reads {
    pub bottom: bool,
    pub kind_of: Option<usize>,
    pub low: usize,
    pub high: usize,
    /// The values of the entry the run set (`Lane::keep`'s complement).
    pub set: u8,
    /// Whether it read a value of its entry its class does not give: no lane for it (pass 3 waits for the entry).
    pub lost: bool,
}

impl Reads {
    /// A run whose reads are not asked (pass 2, the serial runs).
    const NONE: Reads = Reads { bottom: false, kind_of: None, low: 0, high: 0, set: 0, lost: false };

    /// A lane's, entered with the levels `k[..kd]` (`lanes_from`).
    #[inline]
    fn lane(_k: &[u8; MAXD], kd: usize) -> Reads {
        Reads { low: kd, high: kd, ..Reads::NONE }
    }

    /// At the run's start, from its entry: nothing.
    #[inline]
    fn enter(&mut self, _e: &Entry) {}
}

/// The kinds a level of kind `k` a lane read the kind of is, one lane each: a level of no kind yet (254), any of them.
#[inline]
fn split_kinds(_k: u8, _r: &Reads) -> &'static [u8] {
    &LEVEL_KINDS
}

/// Whether a level of kind `below` can be under one of kind `above`: any can.
#[inline]
fn may_hold(_below: u8, _above: u8) -> bool {
    true
}

/// The nest a lane's run leaves (`levels`'), its reads aside.
#[inline]
fn exit_levels(e: &Entry, _r: &Reads) -> ([usize; MAXD], [usize; MAXD], usize) {
    e.levels()
}

/// No kept value is read by class (`Lane::cond`).
const COND_KEEP: u8 = 0;
const CONDS: &[u8] = &[];

#[inline]
fn enter_cond(_e: &mut Entry, _r: &mut Reads, _cond: u8) {}

impl Entry {
    #[inline]
    fn cond(&self) -> u8 {
        0
    }
}

/// A read of `opened` (the strings open): none, the bottom (the lane's entry may have more); with a level of no kind
/// yet below, which it counts as a string, that level's kind.
#[inline(always)]
fn opened_read(r: &mut Reads, nest: &[usize; MAXD], depth: usize, opened: usize) -> usize {
    if opened == 0 {
        r.bottom = true;
    }
    if r.kind_of.is_none()
        && let Some(d) = (0..depth.min(MAXD)).rev().find(|&d| nest[d] == OPEN)
    {
        r.kind_of = Some(d);
    }
    opened
}

/// A read of `template` (the nest's top) at depth `depth`: at 0, the bottom (the lane's entry may have more open); of a
/// level of no kind yet (`OPEN`), its kind.
#[inline(always)]
fn at_bottom(r: &mut Reads, depth: usize, template: usize) -> bool {
    if depth == 0 {
        r.bottom = true;
    } else if template == OPEN && r.kind_of.is_none() {
        r.kind_of = Some(depth - 1);
    }
    true
}
// }NEST@

/// A first token the loop reads as given, not by its bytes: a region a chunk starts inside, read as its row reads it
/// from a start `s` its closer's search starts past at the chunk's first byte, at level `lv` where it nests.
#[derive(Clone, Copy, Debug)]
struct Force {
    s: usize,
    open: usize,
    row: usize,
    lv: usize,
}

/// The token loop (`lex_on`'s, the product's) from `e` to block `to`: `e` where the next token starts after it, its
/// records written where `WRITE` (pass 2) from `*len_io`, counted where not (pass 1), and its reads of the nest below
/// its entry in `reads`. Its first token `force` where given; only that one where `ONE`.
#[inline(always)]
#[allow(unused_variables, unused_assignments, unused_mut, unused_labels, clippy::all)]
unsafe fn lex_range<const WRITE: bool, const ONE: bool>(src: &[u8], e: &mut Entry, mut st1: &mut Stage1Run, mut s1: &mut Stage1, to: usize, refused_io: &mut bool, out: *mut Rec, len_io: &mut usize, extras: &mut Vec<Rec>, reads: &mut Reads, force: Option<Force>) {
    let mut force = force;
    let n = src.len();
    let nw = n.div_ceil(64);
    let mut refused = *refused_io;
    let mut len = *len_io;
    reads.enter(e);
    let Entry { @MUTS@} = *e;
@LOCALS@    let mut close: usize;
    let mut b = at / 64;
@LOOP@    *e = Entry { @BACK@ };
    e.canonical();
    (*refused_io, *len_io) = (refused, len);
}

/// What the serial lexer checks of its state at the file's end.
#[allow(unused_variables, unused_mut, clippy::all)]
fn finish(src: &[u8], e: &Entry, refused_io: &mut bool) {
    let n = src.len();
    let Entry { @ALL@} = *e;
    let mut refused = *refused_io;
@FINISH@    *refused_io = refused;
}

/// The regions a chunk can start inside (`Force`): each its number, its row, its closer's offset and its level.
const REGIONS: [(usize, usize, usize, usize); @NREGIONS@] = [@REGIONS@];

// @STAGE1{
/// The blocks before a chunk's first that its stage 1 starts at: its windows and its carries are the serial run's again
/// past them.
pub const PRIME: usize = 8;

/// Stage 1 for a chunk from block `from`: started `PRIME` blocks before it (none at the file's start).
fn stage1_at(src: &[u8], s1: &mut Stage1, from: usize) -> Stage1Run {
    let mut st = Stage1Run::new();
    if from > 0 {
        st.t = from.saturating_sub(PRIME);
        stage1_best(src, &mut st, s1, from);
    }
    st
}
// }STAGE1@

/// Where the chunk from block `from` is entered in code: past the whitespace before its first byte (the token before
/// ends there, or runs into the chunk).
fn code_entry(src: &[u8], from: usize) -> usize {
    let n = src.len();
    let mut a = (64 * from).min(n);
    // SAFETY: `a - 1` is a byte of the file.
    while a > 0 && unsafe { w_m_ws(src, n, (a - 1) as isize, &mut 0) } & 1 == 1 {
        a -= 1;
    }
    a
}

/// The backslashes before byte `at`.
#[inline]
fn run_before(src: &[u8], at: usize) -> usize {
    let mut k = 0;
    while k < at && src[at - 1 - k] == b'\\' {
        k += 1;
    }
    k
}

/// The lanes from `e0` (its `at`, and what it keeps of the token before) to block `to`: one for each nest below its entry
/// a lane reads, none first, and where a lane read the bottom, one more level below it of each kind, to `MAXD`.
fn lanes_from(src: &[u8], s1: &mut Stage1, st1: &mut Stage1Run, starts: &[Entry], to: usize, sum: &mut Summary) {
    let mut extras = Vec::new();
    // The nests to enter each start with, outermost first: every start's of one depth before any start's deeper
    // (where the lanes pass `L`, the deepest go).
    let mut todo: Vec<(usize, [u8; MAXD], usize, u8)> = (0..starts.len()).map(|si| (si, [0; MAXD], 0, 0)).collect();
    let mut deeper: Vec<(usize, [u8; MAXD], usize, u8)> = Vec::new();
    let mut i = 0;
    // The runs a summary's lanes take, at most `4 L`: past them, as past `L` lanes, the classes not yet entered are no
    // lane's (most of a run's classes can come to lanes it has, which take no room).
    let mut runs = 0;
    loop {
        // Past `L` no lane is kept: the classes not yet entered are no lane's (`over`).
        if runs == 4 * L {
            sum.over = true;
        }
        if sum.over {
            break;
        }
        runs += 1;
        if i == todo.len() {
            if deeper.is_empty() {
                break;
            }
            todo = core::mem::take(&mut deeper);
            i = 0;
        }
        let (si, k, kd, cond) = todo[i];
        i += 1;
        let e0 = starts[si];
        let at0 = e0.at;
        let mut e = e0;
        let (mut nest, mut held, mut opened) = ([usize::MAX; MAXD], [0usize; MAXD], 0usize);
        for d in 0..kd {
            nest[d] = level_of(k[d]);
            if k[d] != 255 {
                held[opened] = INHERIT + (kd - 1 - d);
                opened += 1;
            }
        }
        e.set_levels(&nest, &held, opened, kd);
        e.lane_entry(src);
        let mut reads = Reads::lane(&k, kd);
        enter_cond(&mut e, &mut reads, cond);
        let (mut refused, mut len) = (false, 0usize);
        // SAFETY: no record written (`WRITE` false).
        unsafe { lex_range::<false, false>(src, &mut e, st1, s1, to, &mut refused, core::ptr::null_mut(), &mut len, &mut extras, &mut reads, None) };
        // It read the kind of a level it entered with none: one lane for each kind, not this one.
        if let Some(d) = reads.kind_of {
            for &kind in split_kinds(k[d], &reads) {
                if d + 1 < kd && !may_hold(kind, k[d + 1]) || d > 0 && !may_hold(k[d - 1], kind) {
                    continue;
                }
                let mut k2 = k;
                k2[d] = kind;
                todo.push((si, k2, kd, cond));
            }
            continue;
        }
        // It read a kept value of its entry its class does not give: one lane for each class of it (`CONDS`).
        if reads.lost {
            if cond == 0 {
                todo.extend(CONDS.iter().map(|&c| (si, k, kd, c)));
            }
            continue;
        }
        let low = reads.low;
        let mut l = NO_LANE;
        l.at0 = at0;
        l.kcn = (kd - low) as u8;
        for j in 0..kd - low {
            l.kc[j] = k[kd - 1 - j];
        }
        l.exact = reads.bottom;
        l.need = low as u8;
        let (xn, xh, xd) = exit_levels(&e, &reads);
        l.pn = (xd - low) as u8;
        // The templates' strings below `low` are the entry's; those above, the lane's, at the records it counted.
        let below = (0..low).filter(|&d| xn[d] != usize::MAX).count();
        let mut h = below;
        for j in 0..xd - low {
            l.p[j] = kind_of(xn[low + j]);
            if xn[low + j] != usize::MAX {
                l.ph[j] = xh[h];
                h += 1;
            }
        }
        l.high = (reads.high - low) as u8;
        l.recs = len;
        l.exit = e.without_nest();
        // Its exit's kept values are not read: one form of them (the start's).
        l.exit.carry(&Entry::START, l.keep);
        l.refused = refused;
        l.moved = e.at != at0 || refused;
        // A lane that refuses the file leaves nothing read: one form of it, its class's.
        if refused {
            (l.exit, l.p, l.ph, l.pn, l.high, l.recs, l.keep) = (Entry::START, [0; MAXD], [0; MAXD], 0, 0, 0, 0);
        }
        l.keep = !reads.set;
        l.cond = cond;
        sum.push(l);
        // It read the bottom: one more lane, with a level more open below, of no kind until one is read.
        if reads.bottom && kd < MAXD {
            let mut k2 = [0u8; MAXD];
            k2[0] = 254;
            k2[1..=kd].copy_from_slice(&k[..kd]);
            deeper.push((si, k2, kd + 1, cond));
        }
    }
}

/// Pass 1: the chunk of blocks `from..to`, writing nothing: its lanes from its code entry (`code_entry`), then from the
/// end of each region it can start inside (its row's read of it from the chunk's first byte, `Force`).
fn chunk_ends(src: &[u8], s1: &mut Stage1, from: usize, to: usize) -> Summary {
    let mut st1 = stage1_at(src, s1, from);
    let mut e = Entry::START;
    e.at = if from == 0 { 0 } else { code_entry(src, from) };
    let start = 64 * from;
    let mut sum = Summary { lanes: [NO_LANE; L], n: 0, start, end: (64 * to).min(src.len()).max(start), code: e.at, head: (64 * to).min(src.len()).max(start), over: false };
    // A token the chunk starts inside, of code: blanks part the tokens of code, so the serial lexer's tokens from the
    // first byte of the run of bytes before the chunk (a block at most) are its, the last of them the one it starts in.
    // Lexed from there in each class of nest (`lanes_from`, to the chunk's first block), each end past the chunk's first
    // byte is one. (A run's first byte starts a token, or is in a region: a region's ends are places of their own.)
    let mut starts = vec![e];
    let mut from_run = false;
    if from > 0 && e.at == start && start < src.len() {
        let n = src.len();
        let mut r = start;
        // SAFETY: `r - 1` is a byte of the file.
        while r > 0 && start - r < 64 && unsafe { w_m_ws(src, n, (r - 1) as isize, &mut 0) } & 1 == 0 {
            r -= 1;
        }
        if r == 0 || start - r < 64 {
            let mut x = Entry::START;
            x.at = r;
            let mut run = Summary { lanes: [NO_LANE; L], n: 0, start: r, end: start, code: r, head: start, over: false };
            let mut st0 = stage1_at(src, s1, r / 64);
            lanes_from(src, s1, &mut st0, &[x], from, &mut run);
            let end = (64 * to).min(n);
            for l in run.lanes() {
                if !l.refused && l.exit.at > start && l.exit.at <= end && !starts.iter().any(|y| y.at == l.exit.at) {
                    starts.push(l.exit.without_nest());
                }
            }
            from_run = true;
        }
    }
    if from == 0 || start >= src.len() {
        lanes_from(src, s1, &mut st1, &starts, to, &mut sum);
        return sum;
    }
    // A token the chunk starts inside, of code: it ends where the code lane's first token does (a name's, a number's,
    // a template's `{`), and the next starts there.
    // The token's end is where one lexed from a byte of it ends: from each byte before the first blank, each end once.
    // (And where the run's first byte is a region's closer, which reads as an opener from there.) Where the run before
    // was lexed in each class, the first end alone: a token lexed from a byte inside one of code ends where it does.
    if e.at == start {
        let end = (64 * to).min(src.len());
        let mut ends: Vec<usize> = Vec::new();
        let mut p = start;
        // SAFETY: `p` is a byte of the file.
        while p < end && unsafe { w_m_ws(src, src.len(), p as isize, &mut 0) } & 1 == 0 {
            let (mut refused, mut len) = (false, 0usize);
            let mut reads = Reads::NONE;
            let mut extras = Vec::new();
            let mut first = Entry::START;
            first.at = p;
            // SAFETY: no record written (`WRITE` false).
            unsafe { lex_range::<false, true>(src, &mut first, &mut st1, s1, usize::MAX, &mut refused, core::ptr::null_mut(), &mut len, &mut extras, &mut reads, None) };
            if !refused && first.at > start && first.at <= end && !ends.contains(&first.at) {
                ends.push(first.at);
                if !starts.iter().any(|x| x.at == first.at) {
                    starts.push(first.without_nest());
                }
                if from_run {
                    break;
                }
            }
            p += 1;
        }
    }
    // Inside a region: its closer's search from the chunk's first byte, or from the backslashes before it.
    let from_byte = start - run_before(src, start);
    let mut seen: Vec<usize> = Vec::new();
    for &(open, row, k, lv) in &REGIONS {
        if from_byte < k {
            continue;
        }
        let s = from_byte - k;
        let mut e = Entry::START;
        e.at = s;
        e.region_entry(open);
        let (mut refused, mut len) = (false, 0usize);
        let mut reads = Reads::NONE;
        let mut extras = Vec::new();
        // SAFETY: no record written (`WRITE` false).
        unsafe { lex_range::<false, true>(src, &mut e, &mut st1, s1, usize::MAX, &mut refused, core::ptr::null_mut(), &mut len, &mut extras, &mut reads, Some(Force { s, open, row, lv })) };
        if refused || seen.contains(&e.at) {
            continue;
        }
        seen.push(e.at);
        // (One start a place: the first, the code's before a region's.)
        if !starts.iter().any(|x| x.at == e.at) {
            starts.push(e.without_nest());
        }
    }
    lanes_from(src, s1, &mut st1, &starts, to, &mut sum);
    sum
}

/// Pass 1 over `src`: each chunk of `blocks` blocks' summary (`chunk_ends`).
pub fn summaries(src: &[u8], blocks: usize) -> Vec<Summary> {
    let mut s1 = Stage1::new(src.len().div_ceil(64));
    chunk_starts(src.len(), blocks).into_iter().map(|q| chunk_ends(src, &mut s1, q, q + blocks.max(1))).collect()
}

/// The serial lexer's entry at each chunk's start, and the records before it: what pass 3 gives where it is right.
pub fn serial_entries(src: &[u8], blocks: usize) -> Vec<(Entry, usize)> {
    let n = src.len();
    let mut s1 = Stage1::new(n.div_ceil(64));
    let mut st1 = Stage1Run::new();
    let (mut e, mut refused, mut len, mut out) = (Entry::START, false, 0usize, Vec::new());
    let mut extras = Vec::new();
    chunk_starts(n, blocks)
        .into_iter()
        .map(|q| {
            let at = (e, len);
            let mut reads = Reads::NONE;
            // SAFETY: no record written (`WRITE` false).
            unsafe { lex_range::<false, false>(src, &mut e, &mut st1, &mut s1, q + blocks.max(1), &mut refused, core::ptr::null_mut(), &mut len, &mut extras, &mut reads, None) };
            out.push(());
            at
        })
        .collect()
}

/// `src` lexed in chunks of `blocks` blocks: pass 1, the prefix sum, then each chunk from its entry, which checks its
/// exit is the next chunk's entry. The records, and the chunks entered again where pass 3's entry was not the one the
/// chunk before left (or no lane gave one).
pub fn lex_chunked(src: &[u8], blocks: usize) -> (Lexed, usize) {
    let n = src.len();
    let nw = n.div_ceil(64);
    let blocks = blocks.max(1);
    let starts = chunk_starts(n, blocks);
    let ends = summaries(src, blocks);
    let mut s1 = Stage1::new(nw);
    let pred = entries(&ends);
    let mut recs: Vec<Rec> = Vec::with_capacity(n + 1);
    let mut extras: Vec<Rec> = Vec::new();
    let (mut e, mut refused, mut again) = (Entry::START, false, 0usize);
    for (c, &q) in starts.iter().enumerate() {
        if pred[c].is_none_or(|(p, r, _)| p != e || r != recs.len()) {
            again += 1;
        }
        let mut st1 = stage1_at(src, &mut s1, q);
        let mut len = recs.len();
        let mut reads = Reads::NONE;
        // SAFETY: room for a record a byte of the file (`n + 1`), each after the last.
        unsafe { lex_range::<true, false>(src, &mut e, &mut st1, &mut s1, q + blocks, &mut refused, recs.as_mut_ptr(), &mut len, &mut extras, &mut reads, None) };
        // SAFETY: `len` records written.
        unsafe { recs.set_len(len) };
        if refused {
            break;
        }
    }
    if !refused {
        finish(src, &e, &mut refused);
    }
    (Lexed { recs, extras, refused }, again)
}
"#;

/// Lexing from the middle for a token loop that reads its blocks' bytes (rr's): `lex_on`'s block as a function
/// (`lex_block`, its writes on in pass 2 alone), each region's end from inside it (`enters`), and the three passes.
/// What crosses a chunk's end is where the next token starts (`at`), so a chunk's summary maps its entry `at` to its
/// exit: a lane from the chunk's first byte (code) and one from inside each region. Lanes share each block's classes,
/// and lanes at the same `at` are one. None where `lex_on` is not that loop.
pub(super) fn chunked_lean(out: &str, enters: &[(String, String)], opens: &[bool; 256]) -> Option<String> {
    let head = "unsafe fn lex_on(src: &[u8], n: usize) -> Lexed {\n";
    let f = out.find(head)?;
    let open = "    'tokens: while b < nw {\n";
    let a = f + out[f..].find(open)? + open.len();
    let close = "    }\n    // SAFETY: `len` records written.";
    let z = a + out[a..].find(close)?;
    let k: usize = {
        let sig = "unsafe fn block_classes(src: &[u8], n: usize, base: usize) -> [u64; ";
        let at = out.find(sig)? + sig.len();
        out[at..].split(']').next()?.parse().ok()?
    };
    let cur = "        let cur = unsafe { block_classes(src, n, base) };\n";
    let body = &out[a..z];
    if !body.contains(cur) {
        return None;
    }
    let mut lines = String::new();
    for l in body.replace(cur, "").lines() {
        let t = l.trim_start();
        let pad = &l[..l.len() - t.len()];
        let l = l.replace("continue 'tokens;", "break 'blk;").replace("break 'tokens;", "break 'blk;");
        if t.starts_with("unsafe { out.add(len).write(") || t == "len += 1;" || t == "extras.push(rec);" {
            let _ = writeln!(lines, "{pad}if WRITE {{\n    {l}\n{pad}}}");
        } else if t == "while run != 0 {" {
            let _ = writeln!(lines, "{pad}// Pass 1 writes no record: of the run, only its last token, which may run past the block.\n{pad}if !WRITE && run != 0 {{\n{pad}    run = 1u64 << (63 - run.leading_zeros());\n{pad}}}\n{l}");
        } else {
            let _ = writeln!(lines, "{l}");
        }
    }
    // Pass 1: the regions' tokens alone (`walk_block`).
    let serial = body.replace(cur, "");
    let (walk, skip, picks, refusing) = walk_block(&serial)?;
    // The bytes a block's rare mask holds, from its classes (`block_classes`).
    let rare_of = if out.contains("s::or(s::test(c, s::splat(RARE)), s::ge(v, 0x80))") {
        "LO[c & 15] & HI[c >> 4] & RARE != 0 || c >= 0x80"
    } else if out.contains("s::test(c, s::splat(RARE))") {
        "LO[c & 15] & HI[c >> 4] & RARE != 0"
    } else {
        return None;
    };
    let refusing = if refusing.is_empty() { "u8::MAX".to_owned() } else { refusing };
    let open_bytes: Vec<usize> = (0..256).filter(|&c| opens[c]).collect();
    if open_bytes.is_empty() || open_bytes.len() > 8 {
        return None;
    }
    let opens_table = format!("/// The bytes of the block at `base` a region's opener can start at: pass 1 reads only the tokens that start at one, and\n/// the block's last. One load, a compare a byte, for every lane.\n#[inline(always)]\nfn open_mask(src: &[u8], base: usize) -> u64 {{\n    let v = s::load(src[base..base + 64].try_into().expect(\"the padded source\"));\n    {}\n}}\n\n", open_bytes.iter().map(|c| format!("s::eq_mask(v, {c:#04x})")).collect::<Vec<_>>().join(" | "));
    let lanes = 1 + enters.len();
    let entries: Vec<String> = std::iter::once("start".to_owned()).chain(enters.iter().map(|(name, _)| format!("{name}(src, n, start, 64 * to)"))).collect();
    let entries = entries.join(", ");
    let checks = format!("/// Pass 1 reads only the tokens at opener bytes, so it sees each refusal the token loop makes: every rare byte's pick has\n/// an arm (`_` refuses none), and an arm that can refuse is an opener byte's.\nconst _: () = {{\n    let mut c = 0;\n    while c < 256 {{\n        let pick = TOKEN[c].0;\n        assert!(!({rare_of}) || matches!(pick, {picks}), \"a rare byte no arm takes\");\n        assert!(!matches!(pick, {refusing}) || matches!(c, {}), \"an arm that refuses at a byte no region opens at\");\n        c += 1;\n    }}\n}};\n\n", open_bytes.iter().map(|c| format!("{c:#04x}")).collect::<Vec<_>>().join(" | "));
    let fns: String = checks + &opens_table + &enters.iter().map(|(_, f)| f.as_str()).collect::<String>();
    let mut t = String::new();
    let _ = write!(
        t,
        r#"
/// The token loop's block `b` (`lex_on`'s), its classes `cur`, from `at`: its records written where `WRITE` (pass 2),
/// none where not (pass 1's chunk end, which reads only where the next token starts after it).
#[inline(always)]
unsafe fn lex_block<const WRITE: bool>(src: &[u8], n: usize, b: usize, cur: [u64; {k}], at_io: &mut usize, refused_io: &mut bool, out: *mut Rec, len_io: &mut usize, extras: &mut Vec<Rec>) {{
    let nw = n.div_ceil(64);
    let (mut at, mut len, mut refused, mut b) = (*at_io, *len_io, *refused_io, b);
    'blk: loop {{
{lines}        break 'blk;
    }}
    (*at_io, *len_io, *refused_io) = (at, len, refused);
}}

/// The token loop from `at` to block `to` (pass 2, `WRITE`; a chunk's last token in pass 1): where the next token
/// starts after it.
#[inline(always)]
unsafe fn lex_range<const WRITE: bool>(src: &[u8], n: usize, mut at: usize, to: usize, refused: &mut bool, out: *mut Rec, len: &mut usize, extras: &mut Vec<Rec>) -> usize {{
    let nw = n.div_ceil(64).min(to);
    let mut b = at / 64;
    while b < nw && !*refused {{
        // SAFETY: `b` is a block of the padded file.
        let cur = unsafe {{ block_classes(src, n, b * 64) }};
        // SAFETY: `out` has room for a record a byte where `WRITE`.
        unsafe {{ lex_block::<WRITE>(src, n, b, cur, &mut at, refused, out, len, extras) }};
        b = (b + 1).max(at / 64);
    }}
    at.max(64 * to)
}}

/// The token loop's state between two tokens (lexing from the middle): where the next token starts. The loop keeps no
/// nest, so its nest's fields hold none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {{
    pub at: usize,
    pub template: usize,
    pub held: [usize; 0],
    pub opened: usize,
    pub nest: [usize; 0],
    pub depth: usize,
}}

impl Entry {{
    /// The file's first byte, in code.
    pub const START: Entry = Entry {{ at: 0, template: usize::MAX, held: [], opened: 0, nest: [], depth: 0 }};

    #[inline]
    fn at(at: usize) -> Entry {{
        Entry {{ at, ..Entry::START }}
    }}

    /// The nest (`Entry::levels` as the core reads it): none.
    #[inline]
    fn levels(&self) -> ([usize; MAXD], [usize; MAXD], usize) {{
        ([usize::MAX; MAXD], [0; MAXD], 0)
    }}

    #[inline]
    fn set_levels(&mut self, _nest: &[usize; MAXD], _held: &[usize; MAXD], _opened: usize, _depth: usize) {{}}

    #[inline]
    fn carry(&mut self, _from: &Entry, _keep: u8) {{}}

    #[inline]
    fn cond(&self) -> u8 {{
        0
    }}
}}

/// No kept value is read by class (`Lane::cond`).
const COND_KEEP: u8 = 0;

/// A chunk's places (`chunk_ends`): code, then one inside each region.
pub const LANES: usize = {lanes};

/// A region's place where its closer is not in the chunk: the region runs through it, and an entry past its end goes
/// through (`Summary::end`).
const NONE: usize = usize::MAX;

/// The backslashes before byte `start`.
#[inline]
fn run_before(src: &[u8], start: usize, b: u8) -> usize {{
    let mut k = 0;
    while k < start && src[start - 1 - k] == b {{
        k += 1;
    }}
    k
}}

{fns}/// Pass 1's block `b` from `at` (a token's start), the block's opener bytes `open`: only the tokens that start at one, each
/// read as the token loop reads it (a region's to its end): `at` the start after the last read (on past the block where a
/// region runs on), or a refusal. The
/// regions are the grammar's by their bytes (`regions`): one opens at each opener byte outside them, and no token
/// other than a region's holds an opener byte past its first, so the token loop's starts are not read.
#[inline(always)]
#[allow(unused_variables, unused_assignments)]
fn walk_block(src: &[u8], n: usize, b: usize, open: u64, at: &mut usize, refused: &mut bool) {{
    let base = 64 * b;
{walk}}}

/// Where the next token starts after block `to`, a lane in code at `at` with no opener byte after it in the chunk: the
/// token loop from the byte after the chunk's last byte `skip` holds (no token's outside the regions), or from `at`.
#[inline]
fn tail(src: &[u8], n: usize, at: usize, to: usize, refused: &mut bool) -> usize {{
    let end = (64 * to).min(n);
    let mut lo = at;
    if at < end {{
        let mut b = (end - 1) / 64;
        loop {{
            let base = 64 * b;
            // SAFETY: `base` is a block of the padded file.
            let skip = unsafe {{ block_classes(src, n, base) }}[{skip}];
            let below = if end - base >= 64 {{ !0u64 }} else {{ (1u64 << (end - base)) - 1 }};
            let from = if at > base {{ !0u64 << (at - base) }} else {{ !0u64 }};
            let m = skip & below & from;
            if m != 0 {{
                lo = base + 64 - m.leading_zeros() as usize;
                break;
            }}
            if base <= at {{
                break;
            }}
            b -= 1;
        }}
    }}
    let (mut len, mut extras) = (0usize, Vec::new());
    // SAFETY: no record written (`WRITE` false).
    unsafe {{ lex_range::<false>(src, n, lo, to, refused, core::ptr::null_mut(), &mut len, &mut extras) }}
}}

/// Pass 1: chunk `from..to` (blocks) walked from each lane's entry (`walk_block`), writing nothing: each lane's exit, as the core's summary (`Summary`).
/// Each lane walks alone, and keeps where it is after each block it reads (the first `SEEN`): a later lane at a place an
/// earlier one was after the same block is that lane from there on.
pub fn chunk_ends(src: &[u8], n: usize, from: usize, to: usize) -> Summary {{
    const SEEN: usize = 32;
    let start = 64 * from;
    // The file's first chunk starts in code.
    let at0: [usize; LANES] = if start == 0 {{ [0; LANES] }} else {{ [{entries}] }};
    let end = n.div_ceil(64).min(to);
    let mut seen = [[0u32; SEEN]; LANES];
    let mut seen_len = [0usize; LANES];
    let mut lanes = [(NONE, 0usize, false); LANES];
    for k in 0..LANES {{
        let (mut at, mut refused) = (at0[k], false);
        let mut b = at / 64;
        // Each earlier lane's place to compare with next, and the one this lane is.
        let mut ptr = [0usize; LANES];
        let mut same = (0..k).find(|&j| at0[j] == at);
        if at == NONE {{
            continue;
        }}
        while same.is_none() && b < end && !refused {{
            walk_block(src, n, b, open_mask(src, 64 * b), &mut at, &mut refused);
            if seen_len[k] < SEEN {{
                seen[k][seen_len[k]] = at as u32;
                seen_len[k] += 1;
            }}
            for j in 0..k {{
                while ptr[j] < seen_len[j] && (seen[j][ptr[j]] as usize) < at {{
                    ptr[j] += 1;
                }}
                if ptr[j] < seen_len[j] && seen[j][ptr[j]] as usize == at && !lanes[j].2 {{
                    same = Some(j);
                    break;
                }}
            }}
            b = (b + 1).max(at / 64);
        }}
        lanes[k] = match same {{
            Some(j) => (at0[k], lanes[j].1, lanes[j].2),
            None => {{
                // In code at the chunk's end: its last token, which may run past it.
                let exit = if !refused && at < (64 * to).min(n) {{ tail(src, n, at, to, &mut refused) }} else {{ at }};
                (at0[k], exit.max(64 * to), refused)
            }}
        }};
    }}
    // The core's summary: each place's lane, none for a region whose closer is not in the chunk (it runs through).
    let mut sum = Summary {{ lanes: [NO_LANE; L], n: 0, start, end: (64 * to).min(n).max(start), code: at0[0], head: (64 * to).min(n).max(start), over: false }};
    for (k, &(a, exit, refused)) in lanes.iter().enumerate() {{
        if k > 0 && a == NONE {{
            continue;
        }}
        let mut l = NO_LANE;
        l.at0 = a;
        l.exit = Entry::at(exit);
        l.refused = refused;
        l.moved = exit != a || refused;
        sum.push(l);
    }}
    sum
}}

/// Pass 2 of one chunk: the token loop from its entry `at` to block `to`, its records appended to `recs` and `extras`.
/// Where the next token starts after it, and whether the file is refused.
pub fn lex_chunk(buf: &[u8], n: usize, at: usize, to: usize, recs: &mut Vec<Rec>, extras: &mut Vec<Rec>) -> (usize, bool) {{
    // A record a byte of the chunk from `at` at most: each starts in it.
    recs.reserve((64 * to).min(n).saturating_sub(at) + 1);
    let (mut len, mut refused) = (recs.len(), false);
    // SAFETY: room for a record a byte of the chunk past `len`.
    let exit = unsafe {{ lex_range::<true>(buf, n, at, to, &mut refused, recs.as_mut_ptr(), &mut len, extras) }};
    // SAFETY: `len` records written.
    unsafe {{ recs.set_len(len) }};
    (exit, refused)
}}

/// `buf[..n]` lexed in chunks of `blocks` blocks: pass 1, the prefix sum, then each chunk from its entry, which checks
/// its exit is the next chunk's entry. The records, and the chunks entered again where pass 3's entry was not the one
/// the chunk before left (or no lane gave one).
pub fn lex_chunked(buf: &[u8], n: usize, blocks: usize) -> (Lexed, usize) {{
    assert!(buf.len() >= n + PADDING && buf[n..n + PADDING].chunks_exact(8).all(|w| w == b"        "), "the source padded with spaces");
    let starts = chunk_starts(n, blocks);
    let ends: Vec<Summary> = starts.iter().map(|&q| chunk_ends(buf, n, q, q + blocks.max(1))).collect();
    let entry = entries(&ends);
    let (mut recs, mut extras) = (Vec::with_capacity(n + 1), Vec::new());
    let (mut refused, mut again, mut at) = (false, 0, 0usize);
    for c in 0..starts.len() {{
        if entry[c].is_none_or(|(e, _, _)| e.at != at) {{
            again += 1;
        }}
        let (exit, r) = lex_chunk(buf, n, at, starts[c] + blocks.max(1), &mut recs, &mut extras);
        if r {{
            refused = true;
            break;
        }}
        at = exit;
    }}
    (Lexed {{ recs, extras, refused }}, again)
}}
"#
    );
    // The summaries' lanes, by place alone (the loop keeps no nest), and pass 3's prefix sum: the core's.
    // (A range's lanes are its head's and those of its later chunks' places: room for several chunks'.)
    Some(t + &core_with(0, "", 0, 32.max(lanes), true, false))
}

/// The statement of `text` that starts at the line starting with `start` (trimmed), to the `;` its braces close at.
fn statement_at(text: &str, start: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let i = lines.iter().position(|l| l.trim_start().starts_with(start))?;
    let mut depth = 0i32;
    let mut out = String::new();
    for l in &lines[i..] {
        out.push_str(l);
        out.push('\n');
        // Braces outside character and string literals.
        let (mut in_chr, mut in_str, mut prev) = (false, false, ' ');
        for c in l.chars() {
            match c {
                '"' if !in_chr && prev != '\\' => in_str = !in_str,
                '\'' if !in_str && prev != '\\' => in_chr = !in_chr,
                '{' if !in_chr && !in_str => depth += 1,
                '}' if !in_chr && !in_str => depth -= 1,
                _ => {}
            }
            prev = if prev == '\\' && c == '\\' { ' ' } else { c };
        }
        if depth == 0 && l.trim_end().ends_with(';') {
            return Some(out);
        }
    }
    None
}

/// Pass 1's block (`walk_block`'s body) from the token loop `serial` (`lex_on`'s): at each opener byte from `at` on, the
/// token there read as the loop reads a rare token (its `pick`'s arm), the block masks it reads none (an operator of one
/// byte: no opener byte is a pair's). And the index of `skip`'s mask in the block's classes, the picks an arm takes, and
/// those whose arm can refuse. None where the loop is not of that shape.
fn walk_block(serial: &str) -> Option<(String, String, String, String)> {
    let skip_mask = serial.lines().find_map(|l| l.trim().strip_prefix("let s = !").and_then(|r| r.split(' ').next()))?.to_owned();
    let skip = serial.lines().find_map(|l| l.trim().strip_prefix(&format!("let {skip_mask} = cur[")).and_then(|r| r.strip_suffix("];")))?.to_owned();
    let rare = statement_at(serial, "let (pick, kind1, word1) = TOKEN[")?;
    let arms = statement_at(serial, "let (l, kind, extra, word): (usize, u32, bool, u32) = match pick {")?;
    // The picks an arm takes whatever its byte's tests: each rare byte's must be one, or `_` refuses where the serial
    // loop's run of the block would.
    let arms_d = super::dedent(&arms);
    let picks: Vec<String> = arms_d
        .lines()
        .filter(|l| l.starts_with("    ") && !l.starts_with("     ") && l.ends_with("=> {") && !l.contains(" if "))
        .filter_map(|l| l.trim().strip_suffix(" => {").filter(|p| p.chars().all(|c| c.is_ascii_digit() || c == ' ' || c == '|')).map(str::to_owned))
        .collect();
    if picks.is_empty() {
        return None;
    }
    // The picks whose arm can refuse: each must be an opener byte's alone, where pass 1 reads it.
    let mut refusing: Vec<String> = Vec::new();
    let mut arm: Option<String> = None;
    for l in arms_d.lines() {
        if l.starts_with("    ") && !l.starts_with("     ") && l.contains("=> {") {
            arm = l.trim().split(" if ").next().and_then(|p| p.split(" =>").next()).map(str::to_owned);
        }
        if l.contains("refused = true;")
            && let Some(a) = &arm
            && a != "_"
            && !refusing.contains(a)
        {
            refusing.push(a.clone());
        }
    }
    let snippet = (rare + &arms).replace("break 'tokens;", "break 'blk usize::MAX;");
    // The block masks the arms read, none: the token at an opener byte that opens no region is one byte (`at` past it).
    let mut masks: Vec<String> = Vec::new();
    for (i, _) in snippet.match_indices("k_") {
        let name: String = snippet[i..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
        let before = snippet[..i].chars().last().unwrap_or(' ');
        if !(before.is_ascii_alphanumeric() || before == '_') && !masks.contains(&name) {
            masks.push(name);
        }
    }
    let zeros: String = masks.iter().map(|m| format!("    let {m} = 0u64;\n")).collect();
    let re = |t: &str, k: usize| super::indent(&super::dedent(t), &" ".repeat(k));
    Some((
        format!(
            "{zeros}    let mut open = if *at > base {{ open & (!0u64 << (*at - base)) }} else {{ open }};\n    let mut refused_now = false;\n    while open != 0 {{\n        let s = base + open.trailing_zeros() as usize;\n        let l = 'blk: loop {{\n{}            break 'blk l;\n        }};\n        if refused_now {{\n            *refused = true;\n            return;\n        }}\n        *at = l + 1;\n        if *at >= base + 64 {{\n            return;\n        }}\n        open &= !0u64 << (*at - base);\n    }}\n",
            re(&snippet.replace("refused = true;", "refused_now = true;"), 12),
        ),
        skip,
        picks.join(" | "),
        refusing.join(" | "),
    ))
}

#[cfg(test)]
mod tests {
    /// Kotlin's tier 1 is of the shape: its chunked form is made.
    #[test]
    fn kotlins_loop_has_a_chunked_form() {
        let src = include_str!("../../../kotlin_aot/src/tier1.rs");
        if let Err(e) = super::chunked_nest(src) {
            panic!("Kotlin's token loop has no chunked form: {e}");
        }
    }

    /// Python's tier 1 is of the shape: its chunked form, whose nest is its string reader's, is made (its quotes, its
    /// format flag, its raw, bytes and triple flags, its interpolation's `{`).
    #[test]
    fn pythons_loop_has_a_chunked_form() {
        let src = include_str!("../../../python_aot/src/tier1.rs");
        let rn = super::ReaderNest { quotes: 2, format: 0x04, free: vec![0x01, 0x02, 0x80], open_byte: b'{' };
        match super::chunked_reader(src, &rn) {
            Ok(t) => assert!(src.ends_with(&t), "Python's tier 1 is not its chunked form's"),
            Err(e) => panic!("Python's token loop has no chunked form: {e}"),
        }
    }
}
