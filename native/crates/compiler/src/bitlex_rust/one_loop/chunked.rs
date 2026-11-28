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
        let arms = arms_of(body, r);
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
    let hex = |v: &mut dyn Iterator<Item = u8>| v.map(|b| format!("{b:#04x}")).collect::<Vec<_>>().join(" | ");
    let t = TEMPLATE
        .replace("@LOOP@", &lp)
        .replace("@RETS@", &list(&rets))
        .replace("@CYCLES@", &list(&cycles))
        .replace("@SEPS@", &list(&seps))
        .replace("@CONT@", &list(&cont))
        .replace("@CYCLE@", &list(&cycle))
        .replace("@BRACKETS@", bracket_arms.trim_end())
        .replace("@OPENS@", &hex(&mut brackets.iter().map(|b| b.0)))
        .replace("@CLOSES@", &hex(&mut brackets.iter().map(|b| b.1)))
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
// first place at or past a record (`run_range`), the bracket scan (`kinds`), and the three passes over chunks of
// records. Pass 1 walks each chunk once, from the place and levels its open levels' kinds give (`span`); pass 3
// scans the walks' spans of the writer's frames (`entries`); pass 2 walks each chunk from its entry.

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

/// What a range wrote into a frame open at its entry, which it does not know: the children it added, its last.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Touch {
    pub kids: u32,
    pub last: Option<u32>,
}

/// The writer's frames, kept as the cursor keeps them, without writing (pass 1's, and the serial walk's in the tests):
/// the nodes written (from the range's first, `n`), the frames the range opened (`own`), those open at its entry that
/// it knows (`base`, outermost first) and what it wrote into those below them (`touch`, innermost first), the frames
/// of its entry it closed (`closed`), and the last node's start.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shadow<'a> {
    /// The token pass's answers (none: the walk reads each token itself, as the serial walk).
    pub toks: &'a [Token],
    pub n: u32,
    pub own: Vec<Frame>,
    pub base: Vec<Frame>,
    pub touch: Vec<Touch>,
    pub closed: usize,
    pub last_start: Option<u64>,
}

impl Shadow<'_> {
    /// A node written at `start`: its parent's child.
    #[inline(always)]
    fn node(&mut self, start: u64) -> u32 {
        let i = self.n;
        if let Some(f) = self.own.last_mut() {
            f.kids += 1;
            f.last = i;
        } else if self.closed < self.base.len() {
            let k = self.base.len() - 1 - self.closed;
            self.base[k].kids += 1;
            self.base[k].last = i;
        } else {
            let k = self.closed - self.base.len();
            if self.touch.len() <= k {
                self.touch.resize(k + 1, Touch::default());
            }
            self.touch[k].kids += 1;
            self.touch[k].last = Some(i);
        }
        self.n += 1;
        self.last_start = Some(start);
        i
    }
}

impl Out for Shadow<'_> {
    #[inline(always)]
    fn known(&self, i: usize) -> Option<Result<Token, u32>> {
        let t = *self.toks.get(i)?;
        if t.err != NONE {
            Some(Err(t.err))
        } else if t.kind == NONE {
            None
        } else {
            Some(Ok(t))
        }
    }
    /// The token's nodes as one child of the node open: the first a child, the rest inside it, the last at `t.last`.
    #[inline(always)]
    unsafe fn skip(&mut self, start: u64, t: Token) -> usize {
        self.node(start);
        self.n += t.nodes - 1;
        self.last_start = Some(u64::from(t.last));
        t.end as usize
    }
    #[inline(always)]
    unsafe fn open(&mut self, _: u32, start: u64) {
        let i = self.node(start);
        self.own.push(Frame { node: i, kids: 0, last: NONE, start });
    }
    #[inline(always)]
    unsafe fn leaf(&mut self, _: u32, start: u64, _: u64) {
        self.node(start);
    }
    #[inline(always)]
    unsafe fn leaf_in(&mut self, _: u32, _: u32, start: u64, cs: u64, _: u64, _: u64) {
        self.node(start);
        self.n += 1;
        self.last_start = Some(cs);
    }
    #[inline(always)]
    unsafe fn close(&mut self, _: u64) {
        if self.own.pop().is_none() {
            self.closed += 1;
        }
    }
}

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

/// The lists' separators.
pub const SEPS: &[u8] = &[@SEPS@];

/// A record's change of the depth, by its first byte: a level's opener or closer.
#[inline(always)]
fn delta(b: u8) -> isize {
    match b {
        @OPENS@ => 1,
        @CLOSES@ => -1,
        _ => 0,
    }
}

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

/// Hillis and Steele's inclusive scan of `xs` by `op` (`op(a, b)`, `a` the earlier): round `r` takes each chunk's value
/// after the one `2^r` chunks before it, each round's chunks in `order`'s order, the two buffers apart.
pub fn scan<T: Clone>(xs: Vec<T>, order: Order<'_>, op: impl Fn(&T, &T) -> T) -> Vec<T> {
    let mut acc = xs;
    let mut d = 1;
    while d < acc.len() {
        let next = each(acc.len(), order, |c| if c < d { acc[c].clone() } else { op(&acc[c - d], &acc[c]) });
        acc = next;
        d *= 2;
    }
    acc
}

/// Each chunk's first record: in each run of `size` records, the first after a separator, placed by a prefix sum of
/// the runs that have one; the first chunk at the file's first record. A run with none is its chunk's.
pub fn chunk_starts(src: &[u8], lx: &crate::tier1::Lexed, size: usize, order: Order<'_>) -> Vec<usize> {
    let size = size.max(1);
    let runs = lx.count.div_ceil(size).max(1);
    let found: Vec<Option<usize>> = each(runs, order, |k| if k == 0 { Some(0) } else { (k * size..((k + 1) * size).min(lx.count)).find(|&t| SEPS.contains(&src[lx.starts[t - 1] as usize])) });
    let places = scan(found.iter().map(|f| usize::from(f.is_some())).collect(), order, |a, b| a + b);
    let mut out = vec![0; places[runs - 1]];
    for k in order(runs) {
        if let Some(t) = found[k] {
            out[places[k] - 1] = t;
        }
    }
    out
}

/// The depth at each chunk's first record: each chunk's sum of its records' openers and closers, then their prefix
/// sum (none below 0: a file whose closers pass its openers is refused before them).
pub fn depths(src: &[u8], lx: &crate::tier1::Lexed, chunks: &[usize], order: Order<'_>) -> Vec<usize> {
    let n = chunks.len();
    let sums: Vec<isize> = each(n, order, |c| (chunks[c]..chunks.get(c + 1).copied().unwrap_or(lx.count)).map(|t| delta(src[lx.starts[t] as usize])).sum());
    let inc = scan(sums, order, |a, b| a + b);
    each(n, order, |c| if c == 0 { 0 } else { inc[c - 1].max(0) as usize })
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

/// A range's brackets reduced (the bracket monoid): the closers it cannot match, their kinds in file order (its
/// anti-opens, the mirror of its opens), then the openers it leaves open, their kinds outermost first. `broken`: a
/// closer met an opener of another kind (the serial walk refuses there or before).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Word {
    pub anti: Vec<u8>,
    pub opens: Vec<u8>,
    pub broken: bool,
}

impl Word {
    /// Records `start..stop`'s word, by their first bytes.
    pub fn of(src: &[u8], lx: &crate::tier1::Lexed, start: usize, stop: usize) -> Word {
        let mut w = Word::default();
        for t in start..stop.min(lx.count) {
            match bracket(src[lx.starts[t] as usize]) {
                Some((k, true)) => w.opens.push(k),
                Some((k, false)) => w.cancel(k),
                None => {}
            }
        }
        w
    }

    /// A closer of kind `k` after the word: it cancels the innermost open, or is an anti-open.
    fn cancel(&mut self, k: u8) {
        match self.opens.pop() {
            Some(o) if o == k => {}
            Some(_) => self.broken = true,
            None => self.anti.push(k),
        }
    }

    /// Word `a` then word `b`: `b`'s anti-opens cancelled against `a`'s opens, innermost first. Associative.
    pub fn then(a: &Word, b: &Word) -> Word {
        let mut w = a.clone();
        w.broken |= b.broken;
        for &k in &b.anti {
            w.cancel(k);
        }
        w.opens.extend_from_slice(&b.opens);
        w
    }
}

/// The open levels' kinds at a range's start, outermost first, the root's at the bottom: as many as the range reads
/// (its anti-opens and the innermost level, `Kinds::keep`). `None` past a broken word.
pub type Kinds = Option<Vec<u8>>;

/// The kinds range `w` leaves after kinds `a` at its start: its anti-opens cancel `a`'s levels, its opens follow.
fn kinds_after(a: &Kinds, w: &Word) -> Kinds {
    let mut k = a.clone()?;
    if w.broken {
        return None;
    }
    for &x in &w.anti {
        // The root closes to nothing: the serial walk refuses there.
        match k.pop() {
            Some(o) if o == x && o != ROOT => {}
            _ => return None,
        }
    }
    k.extend_from_slice(&w.opens);
    Some(k)
}

/// Kinds `a` as range `w` reads them: its anti-opens' levels and the innermost one.
fn kinds_keep(a: Kinds, w: &Word) -> Kinds {
    let mut k = a?;
    let cut = k.len().saturating_sub(w.anti.len() + 1);
    k.drain(..cut);
    Some(k)
}

/// A balanced tree of `xs` by `op` (Blelloch's up-sweep), level 0 `xs`, each next level's node the composition of two
/// of the level below (the last alone where they are odd), each level's nodes in `order`'s order.
pub fn tree_of<T: Clone>(xs: &[T], order: Order<'_>, op: impl Fn(&T, &T) -> T) -> Vec<Vec<T>> {
    let mut levels: Vec<Vec<T>> = vec![xs.to_vec()];
    while levels[levels.len() - 1].len() > 1 {
        let low = &levels[levels.len() - 1];
        let up = each(low.len().div_ceil(2), order, |i| if 2 * i + 1 < low.len() { op(&low[2 * i], &low[2 * i + 1]) } else { low[2 * i].clone() });
        levels.push(up);
    }
    levels
}

/// The down-sweep of a tree (`tree_of`): each leaf's state at its start from `start`, a node's left child's its own,
/// its right child's its state through its left child's value (`after`), each as its node reads it (`keep`); then the
/// state after the root. Each level's nodes in `order`'s order.
pub fn down<T, S: Clone>(levels: &[Vec<T>], start: S, order: Order<'_>, after: impl Fn(&S, &T) -> S, keep: impl Fn(S, &T) -> S) -> Vec<S> {
    let top = levels.len() - 1;
    let end = after(&start, &levels[top][0]);
    let mut states: Vec<S> = vec![keep(start, &levels[top][0])];
    for r in (0..top).rev() {
        let (low, up) = (&levels[r], &states);
        states = each(low.len(), order, |c| {
            let parent = &up[c / 2];
            let at = if c % 2 == 0 { parent.clone() } else { after(parent, &low[c - 1]) };
            keep(at, &low[c])
        });
    }
    states.push(end);
    states
}

/// Each chunk's open levels' kinds at its first record (the bracket scan), then after the file: the chunks' words,
/// their tree, and the kinds down it. No walk: each record's first byte.
pub fn kinds(src: &[u8], lx: &crate::tier1::Lexed, chunks: &[usize], order: Order<'_>) -> (Vec<Word>, Vec<Kinds>) {
    let words = each(chunks.len(), order, |c| Word::of(src, lx, chunks[c], chunks.get(c + 1).copied().unwrap_or(lx.count)));
    let levels = tree_of(&words, order, Word::then);
    let ks = down(&levels, Some(vec![ROOT]), order, kinds_after, kinds_keep);
    (words, ks)
}

/// A range's span of the writer's frames (pass 1's summary): it closes `fkc` of the frames open at its entry (its
/// anti-frames, the mirror of those it opens), writes `touch` into those (innermost first, the last the one it is in
/// then), leaves `pf` open (their nodes and last children from the range's first node), writes `nodes`, and its last
/// node starts at `last_start`. It leaves at `exit`, or the file ends in it (`end`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub exit: Entry,
    pub nodes: u32,
    pub end: Option<Result<(), u32>>,
    pub fkc: usize,
    pub touch: Vec<Touch>,
    pub pf: Vec<Frame>,
    pub last_start: Option<u64>,
}

impl Span {
    /// The span of a walk that stopped at `exit` (or ended, `end`), from what `sh` kept.
    fn of(exit: Entry, end: Option<Result<(), u32>>, sh: Shadow<'_>) -> Span {
        let (touch, pf) = if end.is_some() {
            (vec![Touch::default(); sh.closed + 1], Vec::new())
        } else {
            let mut touch = sh.touch;
            touch.resize(sh.closed + 1, Touch::default());
            (touch, sh.own)
        };
        Span { exit, nodes: sh.n, end, fkc: sh.closed, touch, pf, last_start: sh.last_start }
    }

    /// Span `a` then span `b`: `b`'s anti-frames cancel the frames `a` leaves open, the rest closed below `a`'s
    /// entry; what `b` wrote into the one it is then in, that frame's. A span that ended goes on as itself.
    pub fn then(a: &Span, b: &Span) -> Span {
        if a.end.is_some() {
            return a.clone();
        }
        let at = |x: u32| x + a.nodes;
        let (n, fm) = (a.pf.len(), b.fkc.min(a.pf.len()));
        let mut pf = a.pf[..n - fm].to_vec();
        let mut touch = a.touch.clone();
        if b.fkc < n {
            let (f, t) = (pf.last_mut().expect("a frame left open"), b.touch[b.fkc]);
            f.kids += t.kids;
            if let Some(l) = t.last {
                f.last = at(l);
            }
        } else {
            let (s, t) = (touch.last_mut().expect("the frame `a` is in"), b.touch[n]);
            s.kids += t.kids;
            if let Some(l) = t.last {
                s.last = Some(at(l));
            }
            touch.extend(b.touch[n + 1..].iter().map(|t| Touch { kids: t.kids, last: t.last.map(at) }));
        }
        pf.extend(b.pf.iter().map(|f| Frame { node: at(f.node), last: if f.last == NONE { NONE } else { at(f.last) }, ..*f }));
        Span { exit: b.exit, nodes: a.nodes + b.nodes, end: b.end, fkc: a.fkc + b.fkc - fm, touch, pf, last_start: b.last_start.or(a.last_start) }
    }
}

/// Pass 1: chunk `start..stop`'s span, its first record at depth `depth0` with the open levels' kinds `kinds` (the
/// bracket scan's): one walk, from the place and the levels' places they give, so it never reads a level it does not
/// know. A chunk with no kinds is after the file's refusal (a broken word), as one deeper than the walk goes.
pub fn span(src: &[u8], lx: &crate::tier1::Lexed, toks: &[Token], start: usize, stop: usize, depth0: usize, kinds: &Kinds, anti: usize) -> Span {
    let refused = || Span::of(Entry::default(), Some(Err(lx.starts[start])), Shadow::default());
    let Some(k) = kinds.as_ref().filter(|_| depth0 <= DEPTH) else {
        return refused();
    };
    // The levels it closes that its kinds hold (all but where it closes past the root, which the walk refuses).
    let known = anti.min(k.len() - 1).min(depth0);
    let inner = k[k.len() - 1];
    let st = if start == 0 {
        0
    } else if inner == ROOT {
        return refused();
    } else {
        CYCLE[inner as usize]
    };
    let mut ret = std::boxed::Box::new([0u8; DEPTH]);
    for j in 0..known {
        ret[depth0 - 1 - j] = CONT[k[k.len() - 2 - j] as usize];
    }
    let mut sh = Shadow { toks, ..Shadow::default() };
    // SAFETY: the records and the padded source are tier 1's, as the serial walk's.
    match unsafe { run_range(src, lx, &mut sh, Entry { i: start, e: 0, depth: depth0, st }, &mut *ret, depth0 - known, stop) } {
        Stop::At(x) => Span::of(x, None, sh),
        Stop::Done(r) => Span::of(Entry::default(), Some(r), sh),
        // A level its word does not close: the walk closes a level only at a closer.
        Stop::Read(_) => refused(),
    }
}

/// The walk's state at a range's start: its place, the levels' places, the nodes before it; or the file's end. Its
/// levels and frames are the innermost ones, as many as the range reads: the places of the levels below
/// `entry.depth` it closes, the frames of the open nodes it closes and the one it then writes into, outermost first.
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

/// The frames' state range `s` leaves after state `a` (nodes, frames, last start, end; its place `s`'s exit): its
/// anti-frames cancel `a`'s innermost frames, its opens follow. None where `a` holds fewer frames than it closes.
fn at_after(a: &Option<At>, s: &Span) -> Option<At> {
    let a = a.as_ref()?;
    if a.end.is_some() {
        return Some(a.clone());
    }
    let at = |x: u32| x + a.nodes;
    let nf = a.frames.len().checked_sub(s.fkc)?;
    let mut frames = a.frames[..nf].to_vec();
    if let Some(f) = frames.last_mut() {
        let t = s.touch[s.fkc];
        f.kids += t.kids;
        if let Some(x) = t.last {
            f.last = at(x);
        }
    }
    frames.extend(s.pf.iter().map(|f| Frame { node: at(f.node), last: if f.last == NONE { NONE } else { at(f.last) }, ..*f }));
    Some(At { entry: s.exit, ret: Vec::new(), nodes: a.nodes + s.nodes, end: s.end, frames, prev_start: s.last_start.unwrap_or(a.prev_start) })
}

/// State `a` as span `s` reads it: the frames it closes and the one it then writes into.
fn at_keep(a: Option<At>, s: &Span) -> Option<At> {
    let mut a = a?;
    if a.end.is_none() {
        let cut = a.frames.len().saturating_sub(s.fkc + 1);
        a.frames.drain(..cut);
    }
    Some(a)
}

/// Passes 1 and 3: each chunk's state at its start, then the file's end. The bracket scan gives each chunk's open
/// levels' kinds, so its place and the levels it closes; one walk a chunk from there gives its span (pass 1); the
/// frame scan down the spans' tree gives each chunk's nodes before it, its frames and the start before it. Every
/// pass's chunks, and each level of its scans, in `order`'s order.
pub fn entries(src: &[u8], lx: &crate::tier1::Lexed, toks: &[Token], size: usize, order: Order<'_>) -> Vec<Option<At>> {
    let chunks = chunk_starts(src, lx, size, order);
    let ds = depths(src, lx, &chunks, order);
    let (words, ks) = kinds(src, lx, &chunks, order);
    let n = chunks.len();
    let spans = each(n, order, |c| span(src, lx, toks, chunks[c], chunks.get(c + 1).copied().unwrap_or(lx.count + 1), ds[c], &ks[c], words[c].anti.len()));
    let levels = tree_of(&spans, order, Span::then);
    let mut ats = down(&levels, Some(At::start()), order, at_after, at_keep);
    // Each chunk's levels' places, those it closes, from its kinds.
    for c in order(n) {
        if let (Some(a), Some(k)) = (ats[c].as_mut(), ks[c].as_ref()) {
            let known = words[c].anti.len().min(k.len() - 1).min(a.entry.depth);
            if a.end.is_none() {
                a.ret = (0..known).rev().map(|j| CONT[k[k.len() - 2 - j] as usize]).collect();
            }
        }
    }
    ats
}

/// The serial walk's state at each chunk's start, walked chunk after chunk from the file's, then at the file's end
/// (the test of pass 3).
pub fn serial_entries(src: &[u8], lx: &crate::tier1::Lexed, size: usize) -> Vec<At> {
    let chunks = chunk_starts(src, lx, size, &in_order);
    let mut out = Vec::with_capacity(chunks.len());
    let mut at = At::start();
    let mut ret = std::boxed::Box::new([0u8; DEPTH]);
    for c in 0..chunks.len() {
        out.push(at.clone());
        if at.end.is_some() {
            continue;
        }
        let stop = chunks.get(c + 1).copied().unwrap_or(lx.count + 1);
        let mut sh = Shadow { n: at.nodes, base: at.frames.clone(), ..Shadow::default() };
        // SAFETY: as `span`'s.
        let stopped = unsafe { run_range(src, lx, &mut sh, at.entry, &mut *ret, 0, stop) };
        let mut frames = sh.base[..sh.base.len() - sh.closed].to_vec();
        frames.extend_from_slice(&sh.own);
        let prev_start = sh.last_start.unwrap_or(at.prev_start);
        match stopped {
            Stop::At(x) => at = At { entry: x, ret: ret[..x.depth].to_vec(), nodes: sh.n, end: None, frames, prev_start },
            Stop::Done(r) => at = At { entry: Entry::default(), ret: Vec::new(), nodes: sh.n, end: Some(r), frames: Vec::new(), prev_start },
            Stop::Read(_) => unreachable!("the whole nest is known"),
        }
    }
    out.push(at);
    out
}

/// Pass 2: chunk `at..stop`'s nodes written from its state `at`, through a cursor from the middle on the writer's state
/// `state` (its blocks made: `Writer::make_blocks`). Where it stops: at the next chunk's first record, or the file's end.
///
/// # Safety
/// `state` is a writer's state, its room reserved and its blocks made for the tape the chunks write; `at` is the
/// chunk's state from pass 3; no other chunk writes this one's nodes.
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

/// `src`'s tape into `w` from the middle: each chunk of about `size` records summarised (pass 1), each chunk's state by
/// a prefix scan of them (pass 3), then each chunk's nodes written from its state (pass 2): every pass's chunks, and
/// each round of its scans, in the order `order` gives (each on its own: any order writes the same tape). Or the byte
/// it was refused at.
pub fn parse_middle(src: &[u8], lx: &crate::tier1::Lexed, types: &[Vec<u32>], w: &mut Writer, size: usize, order: Order<'_>) -> Result<(), u32> {
    if lx.refused {
        return Err(0);
    }
    if lx.count == 0 {
        return Err(lx.len as u32);
    }
    let chunks = chunk_starts(src, lx, size, order);
    let toks = tokens(src, lx, types, order);
    let ats = entries(src, lx, &toks, size, order);
    let last = ats.last().cloned().flatten().ok_or(0u32)?;
    match last.end {
        Some(Ok(())) => {}
        Some(Err(b)) => return Err(b),
        None => return Err(0),
    }
    w.reserve_run(1 + lx.count + lx.len, (DEPTH + 1) * 3 + 1);
    w.final_as_written();
    w.make_blocks(last.nodes as usize).map_err(|_| u32::MAX)?;
    let state = w.state_address();
    // Twice: each chunk's side-list entries counted, then each chunk written, each entry at its rank.
    for count in [true, false] {
        for c in order(chunks.len()) {
            let at = ats[c].as_ref().ok_or(0u32)?;
            let stop = chunks.get(c + 1).copied().unwrap_or(lx.count + 1);
            // SAFETY: the state's room is reserved and its blocks made; `at` is pass 3's.
            match unsafe { write_range(src, lx, state, at, stop, count) } {
                Stop::At(x) if x.i == stop => {}
                Stop::Done(Ok(())) if c + 1 == chunks.len() => {}
                _ => return Err(u32::MAX),
            }
        }
    }
    w.end_middle(last.nodes as usize, last.prev_start);
    match w.failure() {
        None => Ok(()),
        Some(_) => Err(u32::MAX),
    }
}
"#;
