//! Tier 2's loop from the middle (decision 0726): the walk's state at a place as a value, the walk from any place to
//! the first place at or past a record, and the three passes over chunks of records, appended to the tier.
//!
//! - **A chunk starts after a separator** (a list's: `,`), where the loop is at a list's cycle place: an object's next
//!   pair or an array's next element: its innermost level's kind's cycle place (`CYCLE`).
//! - **A chunk may close more levels than it opens: its closers are anti-opens**, the mirror of its opens. The place
//!   an open pushes is a fact of its level's kind (`CONT`), so a bracket scan (`kinds`: each chunk's word of
//!   anti-opens and opens, composed by cancelling one against the other) gives each chunk the kinds of the levels it
//!   closes, and pass 1 walks it once, from its place with those levels known. `CONT` and `CYCLE` are derived from the
//!   loop's text: a return place's arm takes its kind's closer, its separator goes to its kind's cycle place.
//! - **The depth is a prefix sum** of each record's opener (+1) and closer (−1), a fact of the bytes, so a walk's depth
//!   is the file's and the loop's `DEPTH` test is the serial one.
//! - **Pass 3 scans the walks' spans** of the writer's frames: a span's anti-frames (the frames it closes) cancel the
//!   frames before it, as its word's anti-opens cancel opens.
//!
//! The loop's text is the serial walk's (`run`), its places' exits and its reads of the nest made where the loop makes
//! them. The helpers write through `Out`: the writer's cursor, or pass 1's count of the nodes.

use std::fmt::Write;

/// The tier's text with its chunked form appended: the helpers made generic over what they write through, and the
/// walk from any place (`run_range`) and the three passes. `brackets`: each level's opener and closer.
pub(super) fn with_chunks(out: String, brackets: &[(u8, u8)]) -> Result<String, String> {
    let why = |s: &str| format!("tier 2 from the middle: {s}");
    // The helpers that take the cursor: generic over what they write through.
    let mut lines = String::new();
    // The primitive (`value`'s alternatives that call no level): its name and its alternatives' count.
    let mut prim: Option<(String, usize)> = None;
    for l in out.lines() {
        let fn_line = l.starts_with("fn ") || l.starts_with("unsafe fn ");
        if fn_line && l.contains("d: &mut Cursor<Cells>") {
            let at = l.find('(').ok_or_else(|| why("a helper's parameters"))?;
            let _ = writeln!(lines, "{}<D: Out>{}", &l[..at], l[at..].replace("d: &mut Cursor<Cells>", "d: &mut D"));
            // A token's helper (it takes the next record): the token pass's answer, where the walk has it (stage 2).
            if l.contains("ip: &mut usize") {
                lines.push_str("if let Some(t) = d.known(*ip) {\nlet s = starts[*ip] as u64;\n*ip += 1;\n// SAFETY: as the helper's.\nreturn t.map(|t| unsafe { d.skip(s, t) });\n}\n");
                let name = l[..at].rsplit(' ').next().unwrap_or("");
                if name.ends_with("_primitive") {
                    let k: String = l[at..].split("n: [u32; ").nth(1).unwrap_or("").chars().take_while(char::is_ascii_digit).collect();
                    let k: usize = k.parse().map_err(|_| why("the primitive's alternatives"))?;
                    if prim.replace((name.to_string(), k)).is_some() {
                        return Err(why("more than one primitive"));
                    }
                }
            }
        } else {
            let _ = writeln!(lines, "{l}");
        }
    }
    let out = lines;
    let (prim, alts) = prim.ok_or_else(|| why("no primitive"))?;
    // The primitive's tokens' first bytes: its arms' patterns.
    let pf = out.find(&format!("fn {prim}<D: Out>")).ok_or_else(|| why("the primitive"))?;
    // Its end: the blank line after it (its body's braces are at the line's start too).
    let pe = pf + out[pf..].find("\n}\n\n").ok_or_else(|| why("the primitive's end"))?;
    let mut firsts: Vec<(u8, u8)> = Vec::new();
    for l in out[pf..pe].lines().filter(|l| l.starts_with("0x")) {
        let pat = l.split(" if ").next().unwrap_or(l).split(" => ").next().unwrap_or(l);
        for alt in pat.split(" | ") {
            let byte = |b: &str| b.strip_prefix("0x").and_then(|b| u8::from_str_radix(b, 16).ok()).ok_or_else(|| why("a primitive's first byte"));
            let (a, z) = match alt.split_once("..=") {
                Some((a, z)) => (byte(a)?, byte(z)?),
                None => (byte(alt)?, byte(alt)?),
            };
            firsts.push((a, z));
        }
    }
    if firsts.is_empty() {
        return Err(why("the primitive's first bytes"));
    }
    // The serial walk's loop.
    let head = "unsafe fn run(src: &[u8], lx: &crate::tier1::Lexed, state: *mut u8) -> Result<(), u32> {\n";
    let f = out.find(head).ok_or_else(|| why("the walk"))?;
    let l0 = f + out[f..].find("'run: loop {\n").ok_or_else(|| why("the walk's loop"))?;
    let l1 = l0 + out[l0..].find("\n}\n\nrerail_simd8::kernel!").ok_or_else(|| why("the walk's end"))?;
    let body = &out[l0..l1];
    if !out[f..l0].contains("let mut ret: [u8; DEPTH] = [0; DEPTH];\n") {
        return Err(why("a nest of places of a byte"));
    }
    // The places a level returns to, and each list's cycle place (after its separator), and the separators.
    let mut rets: Vec<u8> = Vec::new();
    for (k, _) in body.match_indices("ret[depth] = ") {
        let n: String = body[k + 13..].chars().take_while(char::is_ascii_digit).collect();
        let n: u8 = n.parse().map_err(|_| why("a return place"))?;
        if !rets.contains(&n) {
            rets.push(n);
        }
    }
    rets.sort_unstable();
    let mut cycles: Vec<u8> = Vec::new();
    let mut seps: Vec<u8> = Vec::new();
    for (k, _) in body.match_indices("continue 'l") {
        let n: String = body[k + 11..].chars().take_while(char::is_ascii_digit).collect();
        let n: u8 = n.parse().map_err(|_| why("a list's place"))?;
        // Its separator: the arm it ends, `0x.. => {\ni += 1;\ncontinue 'lN;`.
        let before = &body[..k];
        let arm = before.strip_suffix("i += 1;\n").and_then(|b| b.strip_suffix(" => {\n")).ok_or_else(|| why("a list's separator arm"))?;
        let byte = arm.rsplit('\n').next().and_then(|b| b.strip_prefix("0x")).and_then(|b| u8::from_str_radix(b, 16).ok()).ok_or_else(|| why("a separator's byte"))?;
        if !cycles.contains(&n) {
            cycles.push(n);
        }
        if !seps.contains(&byte) {
            seps.push(byte);
        }
    }
    // A record at a time, a separator goes to its list's place (`st = N;`, the list's loop a place of its own), not
    // round its loop: `0x.. => {\ni += 1;\nst = N;\ncontinue 'run;` with N a list's place.
    if cycles.is_empty() {
        // Each list's place, as its comment names it (`// The list 'lN: …`).
        let heads: Vec<u8> = body.lines().filter_map(|l| l.strip_prefix("// The list 'l").and_then(|r| r.split(':').next())).filter_map(|n| n.parse::<u8>().ok()).collect();
        for &n in &heads {
            let tail = format!(" => {{\ni += 1;\nst = {n};\ncontinue 'run;\n");
            for (k, _) in body.match_indices(&tail) {
                let byte = body[..k].rsplit('\n').next().and_then(|b| b.strip_prefix("0x")).and_then(|b| u8::from_str_radix(b, 16).ok());
                if let Some(byte) = byte {
                    if !cycles.contains(&n) {
                        cycles.push(n);
                    }
                    if !seps.contains(&byte) {
                        seps.push(byte);
                    }
                }
            }
        }
    }
    cycles.sort_unstable();
    if rets.is_empty() || cycles.is_empty() || brackets.is_empty() {
        return Err(why("no level or no list"));
    }
    // Each level kind's return place and cycle place (the kinds are the brackets'): a return place's arm takes its
    // level's closer, and its separator goes to its list's cycle place; the arm that takes no closer is the root's. So
    // a level's return place is a fact of its kind, the kinds of the open levels a fact of the bytes (decision 0726's
    // anti-opens), and no walk guesses one.
    let mut cont: Vec<Option<u8>> = vec![None; brackets.len() + 1];
    let mut cycle: Vec<Option<u8>> = vec![None; brackets.len()];
    for &r in &rets {
        // A record at a time, a return place may only close what its caller holds and go on (`st = N;`): its kind is
        // the place it goes to's, which takes the closer.
        let mut arms = arms_of(body, r);
        for _ in 0..8 {
            let [a] = &arms[..] else { break };
            if a.contains("match ") {
                break;
            }
            let Some(n) = a.rsplit_once("\nst = ").and_then(|(_, t)| t.split(';').next()).and_then(|t| t.parse::<u8>().ok()) else { break };
            arms = arms_of(body, n);
        }
        if arms.is_empty() {
            return Err(why(&format!("no arm of return place {r}")));
        }
        let kinds: Vec<usize> = (0..brackets.len()).filter(|&k| arms.iter().any(|a| a.contains(&format!("\n{:#04x} => {{\n", brackets[k].1)))).collect();
        let k = match kinds[..] {
            [] => brackets.len(),
            [k] => k,
            _ => return Err(why(&format!("return place {r} takes the closers of more than one kind"))),
        };
        if cont[k].replace(r).is_some() {
            return Err(why("a kind with two return places"));
        }
        if k < brackets.len() {
            let mut target = None;
            for a in &arms {
                for &sep in &seps {
                    if let Some(at) = a.find(&format!("\n{sep:#04x} => {{\n")) {
                        target = a[at..].split("\nst = ").nth(1).and_then(|t| t.split(';').next()).and_then(|t| t.parse::<u8>().ok());
                    }
                }
            }
            let c = target.filter(|c| cycles.contains(c)).ok_or_else(|| why(&format!("kind {k}'s list: no cycle place")))?;
            cycle[k] = Some(c);
        }
    }
    let cont: Vec<u8> = cont.into_iter().collect::<Option<_>>().ok_or_else(|| why("a kind with no return place"))?;
    let cycle: Vec<u8> = cycle.into_iter().collect::<Option<_>>().ok_or_else(|| why("a kind with no list"))?;
    let mut bracket_arms = String::new();
    for (k, &(o, c)) in brackets.iter().enumerate() {
        let _ = write!(bracket_arms, "{o:#04x} => Some(({k}, true)),\n        {c:#04x} => Some(({k}, false)),\n        ");
    }
    // The walk from a place: its exits at a place at or past `stop`, its reads below what it knows of the nest, its
    // ends a value.
    let mut lp = String::new();
    for l in body.lines() {
        let t = l.trim_start();
        if let Some(rest) = t.strip_prefix("e = unsafe { ").and_then(|r| r.strip_suffix(" }?;")) {
            let _ = writeln!(lp, "e = match unsafe {{ {rest} }} {{\nOk(v) => v,\nErr(x) => return Stop::Done(Err(x)),\n}};");
        } else if l.contains("return Err(") || l.contains("return Ok(") {
            let _ = writeln!(lp, "{}", done(l).ok_or_else(|| why("an exit's value"))?);
        } else if t.ends_with("'run: loop {") || t.ends_with("'ret: loop {") {
            let _ = writeln!(lp, "{l}\nif i >= stop {{\nreturn Stop::At(Entry {{ i, e, depth, st }});\n}}");
        } else if t == "depth -= 1;" {
            lp.push_str("if depth == 0 {\nreturn Stop::Done(Err(at(src, starts, i)));\n}\ndepth -= 1;\n");
        } else if t == "st = ret[depth];" {
            lp.push_str("if depth < low {\nreturn Stop::Read(Entry { i, e, depth, st });\n}\nst = ret[depth];\n");
        } else if let Some(n) = t.strip_prefix("continue 'l").and_then(|r| r.strip_suffix(';')) {
            let _ = writeln!(lp, "if i >= stop {{\nst = {n};\ncontinue 'run;\n}}\ncontinue 'l{n};");
        } else {
            let _ = writeln!(lp, "{l}");
        }
    }
    if lp.contains("?;") || lp.contains(")?") || lp.contains("return Err(") || lp.contains("return Ok(") {
        return Err(why("an exit not made a stop"));
    }
    let list = |v: &[u8]| v.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", ");
    let t = TEMPLATE
        .replace("@LOOP@", &lp)
        .replace("@RETS@", &list(&rets))
        .replace("@CYCLES@", &list(&cycles))
        .replace("@CONT@", &list(&cont))
        .replace("@CYCLE@", &list(&cycle))
        .replace("@BRACKETS@", bracket_arms.trim_end())
        .replace("@PRIM@", &prim)
        .replace("@ALTS@", &(0..alts).map(|k| k.to_string()).collect::<Vec<_>>().join(", "))
        .replace("@FIRSTS@", &firsts.iter().map(|(a, z)| format!("({a:#04x}, {z:#04x})")).collect::<Vec<_>>().join(", "));
    Ok(out + &t)
}

/// The arms of the loop's `match st` for place `n`, each from its line to its close (its braces counted).
fn arms_of(body: &str, n: u8) -> Vec<String> {
    let head = format!("{n} => {{");
    let lines: Vec<&str> = body.lines().collect();
    let mut out = Vec::new();
    for (k, l) in lines.iter().enumerate() {
        if *l != head {
            continue;
        }
        let mut depth = 0i32;
        let mut arm = String::from("\n");
        for l in &lines[k..] {
            arm.push_str(l);
            arm.push('\n');
            depth += l.matches('{').count() as i32 - l.matches('}').count() as i32;
            if depth <= 0 {
                break;
            }
        }
        out.push(arm);
    }
    out
}

/// `l` with each `return Err(..)` and `return Ok(..)` a `Stop::Done` of it.
fn done(l: &str) -> Option<String> {
    let mut out = String::new();
    let mut rest = l;
    while let Some(k) = ["return Err(", "return Ok("].iter().filter_map(|p| rest.find(p).map(|k| (k, p.len()))).min() {
        let (k, n) = k;
        // The value's closing parenthesis.
        let mut depth = 1;
        let mut j = k + n;
        for (q, c) in rest[k + n..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                j = k + n + q;
                break;
            }
        }
        if depth != 0 {
            return None;
        }
        let _ = write!(out, "{}return Stop::Done({})", &rest[..k], &rest[k + "return ".len()..=j]);
        rest = &rest[j + 1..];
    }
    out.push_str(rest);
    Some(out)
}

const TEMPLATE: &str = r#"
// Parsing from the middle (decision 0726): the walk's state at a place (`Entry`), the walk from any place to the
// first place at or past a record (`run_range`), and a chunk's nodes written from its state (`write_range`). Tier 2
// as phases walks each chunk a record at a time: these two of the split loop are its `run_step` and `write_step`
// (`step_of`), and the whole loop's are dropped.

/// What the walk writes through: the writer's cursor, or pass 1's count of the nodes.
pub trait Out {
    unsafe fn open(&mut self, number: u32, start: u64);
    unsafe fn leaf(&mut self, number: u32, start: u64, end: u64);
    unsafe fn leaf_in(&mut self, number: u32, child: u32, start: u64, cs: u64, ce: u64, end: u64);
    unsafe fn close(&mut self, end: u64);
    /// The token pass's answer at record `i`, where this walk reads it (pass 1): the token, or the byte it is refused
    /// at; `None` where the walk reads the token itself.
    #[inline(always)]
    fn known(&self, _i: usize) -> Option<Result<Token, u32>> {
        None
    }
    /// Token `t`, its first byte at `start`, as its nodes would have been written: its end.
    #[inline(always)]
    unsafe fn skip(&mut self, _start: u64, t: Token) -> usize {
        t.end as usize
    }
}

/// Stage 2's answer at a token's record (decision 0726): its end, its first node's number among the primitive's
/// alternatives (`kind`), its nodes and the last one's start; or the byte it is refused at (`err`, else `NONE`). A record
/// that starts no token of the primitive's has `kind` `NONE` and `err` `NONE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub end: u32,
    pub kind: u32,
    pub nodes: u32,
    pub last: u32,
    pub err: u32,
}

impl Token {
    /// A record that starts no token.
    pub const NOT: Token = Token { end: 0, kind: NONE, nodes: 0, last: 0, err: NONE };
}

/// The first bytes of the primitive's tokens (its arms').
pub const TOKEN_FIRSTS: &[(u8, u8)] = &[@FIRSTS@];

/// What the token pass writes through: the first node's number, the nodes, the last one's start.
#[derive(Default)]
struct Tok {
    kind: u32,
    nodes: u32,
    last: u32,
}

impl Tok {
    #[inline(always)]
    fn node(&mut self, number: u32, start: u64) {
        if self.nodes == 0 {
            self.kind = number;
        }
        self.nodes += 1;
        self.last = start as u32;
    }
}

impl Out for Tok {
    #[inline(always)]
    unsafe fn open(&mut self, number: u32, start: u64) {
        self.node(number, start);
    }
    #[inline(always)]
    unsafe fn leaf(&mut self, number: u32, start: u64, _: u64) {
        self.node(number, start);
    }
    #[inline(always)]
    unsafe fn leaf_in(&mut self, number: u32, _: u32, start: u64, cs: u64, _: u64, _: u64) {
        self.node(number, start);
        self.node(number, cs);
    }
    #[inline(always)]
    unsafe fn close(&mut self, _: u64) {}
}

/// Stage 2, one token: the primitive read at record `r` on its own (each of its alternatives' nodes numbered by the
/// alternative). Every token reads only its own bytes, so every token is read at once.
pub fn token_at(src: &[u8], lx: &crate::tier1::Lexed, r: usize) -> Token {
    let b = src[lx.starts[r] as usize];
    if !TOKEN_FIRSTS.iter().any(|&(a, z)| a <= b && b <= z) {
        return Token::NOT;
    }
    let mut d = Tok::default();
    let mut i = r;
    // SAFETY: the records and the padded source are tier 1's, as the serial walk's.
    match unsafe { @PRIM@(src, &lx.starts, &mut d, &mut i, [@ALTS@]) } {
        Ok(e) => Token { end: e as u32, kind: d.kind, nodes: d.nodes, last: d.last, err: NONE },
        Err(x) => Token { err: x, ..Token::NOT },
    }
}

/// Stage 2: every token, from each type's records (tier 1's `Typed::types`), each list's tokens in `order`: the token
/// at each record.
pub fn tokens(src: &[u8], lx: &crate::tier1::Lexed, types: &[Vec<u32>], order: Order<'_>) -> Vec<Token> {
    let mut col = vec![Token::NOT; lx.count];
    for list in types {
        for (j, t) in each(list.len(), order, |j| token_at(src, lx, list[j] as usize)).into_iter().enumerate() {
            col[list[j] as usize] = t;
        }
    }
    col
}

impl<const MID: bool> Out for Cursor<Cells, MID> {
    #[inline(always)]
    unsafe fn open(&mut self, number: u32, start: u64) {
        unsafe { Cursor::<Cells, MID>::open(self, number, start) }
    }
    #[inline(always)]
    unsafe fn leaf(&mut self, number: u32, start: u64, end: u64) {
        unsafe { Cursor::<Cells, MID>::leaf(self, number, start, end) }
    }
    #[inline(always)]
    unsafe fn leaf_in(&mut self, number: u32, child: u32, start: u64, cs: u64, ce: u64, end: u64) {
        unsafe { Cursor::<Cells, MID>::leaf_in(self, number, child, start, cs, ce, end) }
    }
    #[inline(always)]
    unsafe fn close(&mut self, end: u64) {
        unsafe { Cursor::<Cells, MID>::close(self, end) }
    }
}

/// The writer's frame of an open node, as the cursor keeps it: the node, its children so far, its last child
/// (`NONE`: none yet), its start.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Frame {
    pub node: u32,
    pub kids: u32,
    pub last: u32,
    pub start: u64,
}

/// No last child yet.
pub const NONE: u32 = u32::MAX;

/// The walk's state at a place: the next record, the last token's end, the levels open, the place. The levels'
/// places (`ret`) are held beside it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub i: usize,
    pub e: usize,
    pub depth: usize,
    pub st: u8,
}

/// Where a walk stops: at a place at or past its end, where it closes a level below what it knows of the nest (its
/// place then the level's to read), or at the file's end or refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    At(Entry),
    Read(Entry),
    Done(Result<(), u32>),
}

/// The places a level returns to.
pub const RETS: &[u8] = &[@RETS@];

/// Each list's cycle place, where the loop is after its separator: the places a chunk's first record can be at.
pub const CYCLES: &[u8] = &[@CYCLES@];

/// The walk from `en` to the first place at or past record `stop`, through `d`: the levels' places `ret[low..depth]`
/// known; a close below `low` stops it (`Stop::Read`), which no walk of a chunk from its kinds meets but past a broken
/// word, where the chunk is past the file's refusal.
#[inline(always)]
pub unsafe fn run_range<D: Out>(src: &[u8], lx: &crate::tier1::Lexed, d: &mut D, en: Entry, ret: &mut [u8; DEPTH], low: usize, stop: usize) -> Stop {
let (starts, count, len) = (&lx.starts[..], lx.count, lx.len);
let _ = (count, len);
let mut i = en.i;
let mut e = en.e;
let mut depth = en.depth;
let mut st: u8 = en.st;
@LOOP@
}

/// The order a pass runs its chunks in: a permutation of `0..n`, asked once each time a pass runs them. Each chunk is
/// on its own, so any order gives the same answer (`in_order`; the tests' random ones).
pub type Order<'a> = &'a dyn Fn(usize) -> Vec<usize>;

/// The chunks in order.
pub fn in_order(n: usize) -> Vec<usize> {
    (0..n).collect()
}

/// `f` of each of `n` chunks, run in `order`'s order: each chunk's result in its place.
pub fn each<T>(n: usize, order: Order<'_>, f: impl Fn(usize) -> T) -> Vec<T> {
    let mut out: Vec<Option<T>> = (0..n).map(|_| None).collect();
    for c in order(n) {
        assert!(out[c].is_none(), "a chunk run twice");
        out[c] = Some(f(c));
    }
    out.into_iter().map(|x| x.expect("every chunk run")).collect()
}

/// Each level kind's return place: the place after a value in a level of that kind, which an open in it pushes
/// (`ret[depth]`), by the kind's bracket, the root's last.
pub const CONT: &[u8] = &[@CONT@];

/// Each level kind's cycle place: where its list's loop is after its separator, a chunk's first record's place.
pub const CYCLE: &[u8] = &[@CYCLE@];

/// The root's kind: past every bracket's.
pub const ROOT: u8 = (CONT.len() - 1) as u8;

/// A record's bracket by its first byte: its level's kind, and whether it opens one.
#[inline(always)]
fn bracket(b: u8) -> Option<(u8, bool)> {
    match b {
        @BRACKETS@
        _ => None,
    }
}

/// The walk's state at a chunk's start: its place, the levels' places, the nodes before it; or the file's end. Its
/// levels and frames are the innermost ones, as many as the chunk reads: the places of the levels below `entry.depth`
/// it closes, the frames of the open nodes it closes and the one it then writes into, outermost first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct At {
    pub entry: Entry,
    pub ret: Vec<u8>,
    pub nodes: u32,
    pub end: Option<Result<(), u32>>,
    /// The writer's frames open there, outermost first, and the start of the node before.
    pub frames: Vec<Frame>,
    pub prev_start: u64,
}

impl At {
    /// The file's first record: the root's place, no level open.
    pub fn start() -> At {
        At { entry: Entry::default(), ret: Vec::new(), nodes: 0, end: None, frames: Vec::new(), prev_start: 0 }
    }
}

/// Chunk `at..stop`'s nodes written from its state `at`, through a cursor from the middle on the writer's state
/// `state` (its blocks made: `Writer::make_blocks`). Where it stops: at the next chunk's first record, or the file's end.
///
/// # Safety
/// `state` is a writer's state, its room reserved and its blocks made for the tape the chunks write; `at` is the
/// chunk's state (the phases'); no other chunk writes this one's nodes.
pub unsafe fn write_range(src: &[u8], lx: &crate::tier1::Lexed, state: *mut u8, at: &At, stop: usize, count: bool) -> Stop {
    let mut ret = std::boxed::Box::new([0u8; DEPTH]);
    // The levels it holds, at their depths.
    let base = at.entry.depth - at.ret.len();
    ret[base..at.entry.depth].copy_from_slice(&at.ret);
    // The frames' room: those open at its entry, and as many as a chunk opens at most.
    let mut buf: Vec<u8> = Vec::with_capacity(((DEPTH + 1) * 3 + 1 + at.frames.len()) * 24);
    let frames: Vec<(u32, u32, u32, u64)> = at.frames.iter().map(|f| (f.node, f.kids, f.last, f.start)).collect();
    // SAFETY: the caller's; the frames' room is `buf`'s.
    unsafe {
        let mut d = Cursor::<Cells, true>::resume(state, buf.as_mut_ptr(), at.nodes, at.prev_start, &frames, count);
        run_range(src, lx, &mut d, at.entry, &mut *ret, 0, stop)
    }
}
"#;
