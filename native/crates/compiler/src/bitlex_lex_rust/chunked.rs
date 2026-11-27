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
        let (v, k) = inner.split_once(';')?;
        return Some(format!("[{}; {}]", type_of(v)?, k.trim()));
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
/// stage 1 made as it comes to its blocks (`Stage1Run`): `out` with it appended, else `out`.
pub(super) fn with_chunks(out: String) -> String {
    match chunked_nest(&out) {
        Ok(t) => out + &t,
        Err(_) => out,
    }
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
    Ok(t + &core(maxd, &kinds_s, kinds.len() + 1, 64, false))
}

/// The three passes' summary for nesting, one for every token loop's chunked form (`chunked_nest`'s, `chunked_lean`'s):
/// lanes by the class of nest below their entry, their composition as typed bracket sequences, and pass 3's prefix sum.
/// A loop's form gives its `Entry` (its `at`, its nest's fields `nest`, `depth`, `template`, `held`, `opened`, and
/// `START`), and makes its chunks' summaries.
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

/// Whether a lane's level of kind `k` (254: any) meets a level of kind `m`.
#[inline]
fn meets(k: u8, m: u8) -> bool {
    k == 254 || k == m
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

const NO_LANE: Lane = Lane { at0: 0, kc: [0; MAXD], kcn: 0, exact: false, need: 0, p: [0; MAXD], pn: 0, ph: [0; MAXD], high: 0, recs: 0, exit: Entry::START, refused: false, moved: false };

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
    // `b`'s first `j` closed levels, innermost first, are `a`'s open ones, innermost last.
    if (0..j).any(|i| !meets(b.kc[i], a.p[pn - 1 - i])) {
        return None;
    }
    let mut l = *b;
    l.at0 = a.at0;
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
        p[keep + i] = b.p[i];
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
    let d = e.depth;
    let l = next_lanes(s, e.at).find(|l| {
        let kn = usize::from(l.kcn);
        kn + usize::from(l.need) <= d && (0..kn).all(|i| meets(l.kc[i], kind_of(e.nest[d - 1 - i]))) && (!l.exact || d == kn)
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
    let strings = (0..rest).filter(|&i| e.nest[i] != usize::MAX).count();
    x.nest = [usize::MAX; MAXD];
    x.nest[..rest].copy_from_slice(&e.nest[..rest]);
    x.held = [0; MAXD];
    x.held[..strings].copy_from_slice(&e.held[..strings]);
    x.opened = strings;
    for i in 0..usize::from(l.pn) {
        x.nest[rest + i] = level_of(l.p[i]);
        if l.p[i] != 255 {
            x.held[x.opened] = if l.ph[i] >= INHERIT {
                // The string of the entry's level the lane closed `q`th: its record, the entry's.
                let lv = d - 1 - (l.ph[i] - INHERIT);
                e.held[(0..lv).filter(|&i| e.nest[i] != usize::MAX).count()]
            } else {
                recs + l.ph[i]
            };
            x.opened += 1;
        }
    }
    x.depth = rest + usize::from(l.pn);
    x.template = if x.depth == 0 { usize::MAX } else { x.nest[x.depth - 1] };
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
    let mut todo: Vec<(usize, [u8; MAXD], usize)> = (0..starts.len()).map(|si| (si, [0; MAXD], 0)).collect();
    let mut deeper: Vec<(usize, [u8; MAXD], usize)> = Vec::new();
    let mut i = 0;
    loop {
        if i == todo.len() {
            if deeper.is_empty() {
                break;
            }
            todo = core::mem::take(&mut deeper);
            i = 0;
        }
        let (si, k, kd) = todo[i];
        i += 1;
        let e0 = starts[si];
        let at0 = e0.at;
        let mut e = e0;
        e.nest = Entry::START.nest;
        e.held = Entry::START.held;
        e.opened = 0;
        for d in 0..kd {
            e.nest[d] = level_of(k[d]);
            if k[d] != 255 {
                e.held[e.opened] = INHERIT + (kd - 1 - d);
                e.opened += 1;
            }
        }
        e.depth = kd;
        e.template = if kd == 0 { usize::MAX } else { level_of(k[kd - 1]) };
        let mut reads = Reads { bottom: false, kind_of: None, low: kd, high: kd };
        let (mut refused, mut len) = (false, 0usize);
        // SAFETY: no record written (`WRITE` false).
        unsafe { lex_range::<false, false>(src, &mut e, st1, s1, to, &mut refused, core::ptr::null_mut(), &mut len, &mut extras, &mut reads, None) };
        // It read the kind of a level it entered with none: one lane for each kind, not this one.
        if let Some(d) = reads.kind_of {
            for &kind in &LEVEL_KINDS {
                let mut k2 = k;
                k2[d] = kind;
                todo.push((si, k2, kd));
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
        l.pn = (e.depth - low) as u8;
        // The templates' strings below `low` are the entry's; those above, the lane's, at the records it counted.
        let below = (0..low).filter(|&d| e.nest[d] != usize::MAX).count();
        let mut h = below;
        for j in 0..e.depth - low {
            l.p[j] = kind_of(e.nest[low + j]);
            if e.nest[low + j] != usize::MAX {
                l.ph[j] = e.held[h];
                h += 1;
            }
        }
        l.high = (reads.high - low) as u8;
        l.recs = len;
        l.exit = e.without_nest();
        l.refused = refused;
        l.moved = e.at != at0 || refused;
        sum.push(l);
        // It read the bottom: one more lane, with a level more open below, of no kind until one is read.
        if reads.bottom && kd < MAXD {
            let mut k2 = [0u8; MAXD];
            k2[0] = 254;
            k2[1..=kd].copy_from_slice(&k[..kd]);
            deeper.push((si, k2, kd + 1));
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
    let mut crossed = None;
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
            let (mut refused, mut len) = (false, 0usize);
            let mut reads = Reads { bottom: false, kind_of: None, low: 0, high: 0 };
            let mut extras = Vec::new();
            let mut st0 = stage1_at(src, s1, r / 64);
            while x.at < start && !refused {
                let was = x.at;
                // SAFETY: no record written (`WRITE` false).
                unsafe { lex_range::<false, true>(src, &mut x, &mut st0, s1, usize::MAX, &mut refused, core::ptr::null_mut(), &mut len, &mut extras, &mut reads, None) };
                if x.at == was {
                    refused = true;
                }
            }
            if !refused && !reads.bottom && x.depth == 0 && x.at > start && x.at <= (64 * to).min(n) {
                crossed = Some(x.without_nest());
            }
        }
    }
    let mut starts = vec![e];
    // (The chunk may start after a token, too: a string's, whose run reads as an opener.)
    if let Some(x) = crossed {
        starts.push(x);
    }
    if from == 0 || start >= src.len() {
        lanes_from(src, s1, &mut st1, &starts, to, &mut sum);
        return sum;
    }
    // A token the chunk starts inside, of code: it ends where the code lane's first token does (a name's, a number's,
    // a template's `{`), and the next starts there.
    // The token's end is where one lexed from a byte of it ends: from each byte before the first blank, each end once.
    // (And where the run's first byte is a region's closer, which reads as an opener from there.)
    if e.at == start {
        let end = (64 * to).min(src.len());
        let mut ends: Vec<usize> = Vec::new();
        let mut p = start;
        // SAFETY: `p` is a byte of the file.
        while p < end && unsafe { w_m_ws(src, src.len(), p as isize, &mut 0) } & 1 == 0 {
            let (mut refused, mut len) = (false, 0usize);
            let mut reads = Reads { bottom: false, kind_of: None, low: 0, high: 0 };
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
        let (mut refused, mut len) = (false, 0usize);
        let mut reads = Reads { bottom: false, kind_of: None, low: 0, high: 0 };
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
            let mut reads = Reads { bottom: false, kind_of: None, low: 0, high: 0 };
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
        let mut reads = Reads { bottom: false, kind_of: None, low: 0, high: 0 };
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
}}

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
    Some(t + &core(0, "", 0, 32.max(lanes), true))
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
}
