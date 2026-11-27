//! Tier 2's loop from the middle (decision 0726): the walk's state at a place as a value, the walk from any place to
//! the first place at or past a record, and the three passes over chunks of records, appended to the tier.
//!
//! - **A chunk starts after a separator** (a list's: `,`), where the loop is at a list's cycle place: an object's next
//!   pair or an array's next element. Those few places are the superposition of a chunk's start: pass 1 walks the
//!   chunk from each.
//! - **A chunk may close more levels than it opens.** A close below the chunk's entry reads the place the level
//!   returns to (`st = ret[depth]`), which the chunk cannot know: the lane forks there, once for each place a level
//!   returns to. What it read is its class (`kc`, innermost first), what it leaves open its `p`, as the token loops'
//!   lanes (`bitlex_lex_rust::chunked`).
//! - **The depth is a prefix sum** of each record's opener (+1) and closer (−1), a fact of the bytes, so a lane's depth
//!   is the file's and the loop's `DEPTH` test is the serial one.
//! - **Pass 3 composes the summaries by a prefix scan** (as tier 1's `entries`): a lane of the first range goes on in
//!   the lanes of the second entered at its place whose closed levels are those it leaves open.
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
    for l in out.lines() {
        let fn_line = l.starts_with("fn ") || l.starts_with("unsafe fn ");
        if fn_line && l.contains("d: &mut Cursor<Cells>") {
            let at = l.find('(').ok_or_else(|| why("a helper's parameters"))?;
            let _ = writeln!(lines, "{}<D: Out>{}", &l[..at], l[at..].replace("d: &mut Cursor<Cells>", "d: &mut D"));
        } else {
            let _ = writeln!(lines, "{l}");
        }
    }
    let out = lines;
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
        .replace("@OPENS@", &hex(&mut brackets.iter().map(|b| b.0)))
        .replace("@CLOSES@", &hex(&mut brackets.iter().map(|b| b.1)));
    Ok(out + &t)
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
// first place at or past a record (`run_range`), and the three passes over chunks of records. Pass 1 walks each chunk
// from each place its first record can be at, forking where it closes a level below its entry (`lanes`); pass 3
// composes the chunks' summaries by a prefix scan (`entries`); pass 2 walks each chunk from its entry.

/// What the walk writes through: the writer's cursor, or pass 1's count of the nodes.
pub trait Out {
    unsafe fn open(&mut self, number: u32, start: u64);
    unsafe fn leaf(&mut self, number: u32, start: u64, end: u64);
    unsafe fn leaf_in(&mut self, number: u32, child: u32, start: u64, cs: u64, ce: u64, end: u64);
    unsafe fn close(&mut self, end: u64);
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
pub struct Shadow {
    pub n: u32,
    pub own: Vec<Frame>,
    pub base: Vec<Frame>,
    pub touch: Vec<Touch>,
    pub closed: usize,
    pub last_start: Option<u64>,
}

impl Shadow {
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

impl Out for Shadow {
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
/// known, those below `low` read where the walk closes to them (`Stop::Read`).
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

/// Each chunk's first record: every `size` records on to the first after a separator; the first chunk at the file's.
pub fn chunk_starts(src: &[u8], lx: &crate::tier1::Lexed, size: usize) -> Vec<usize> {
    let size = size.max(1);
    let mut out = vec![0];
    let mut t = size;
    while t < lx.count {
        if SEPS.contains(&src[lx.starts[t - 1] as usize]) {
            out.push(t);
            t += size;
        } else {
            t += 1;
        }
    }
    out
}

/// The depth at each chunk's first record: the prefix sum of its records' openers and closers (none below 0: a file
/// whose closers pass its openers is refused before them).
pub fn depths(src: &[u8], lx: &crate::tier1::Lexed, chunks: &[usize]) -> Vec<usize> {
    let mut out = Vec::with_capacity(chunks.len());
    let mut d = 0isize;
    let mut c = 0;
    for t in 0..lx.count {
        if c < chunks.len() && chunks[c] == t {
            out.push(d.max(0) as usize);
            c += 1;
        }
        d += delta(src[lx.starts[t] as usize]);
    }
    while out.len() < chunks.len() {
        out.push(d.max(0) as usize);
    }
    out
}

/// One lane of a summary: what a range does for the entries at place `st0` whose innermost levels return to `kc`
/// (innermost first; those it closed). It leaves `p` open above what is left of them (outermost first), and leaves at
/// `exit`, or the file ends in it (`end`), having written `nodes`. Of the writer's frames: it closes `fkc` of its
/// entry's, writes `touch` into those (innermost first, the last the one it is in then), leaves `pf` open (their nodes
/// and last children from the range's first node), and its last node starts at `last_start`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lane {
    pub st0: u8,
    pub kc: Vec<u8>,
    pub p: Vec<u8>,
    pub exit: Entry,
    pub nodes: u32,
    pub end: Option<Result<(), u32>>,
    pub fkc: usize,
    pub touch: Vec<Touch>,
    pub pf: Vec<Frame>,
    pub last_start: Option<u64>,
}

impl Lane {
    /// A lane from where its walk stopped: its frames from what `sh` kept.
    fn of(st0: u8, kc: Vec<u8>, p: Vec<u8>, exit: Entry, end: Option<Result<(), u32>>, sh: Shadow) -> Lane {
        let mut touch = sh.touch;
        touch.resize(sh.closed + 1, Touch::default());
        Lane { st0, kc, p, exit, nodes: sh.n, end, fkc: sh.closed, touch, pf: sh.own, last_start: sh.last_start }
    }
}

/// A range's summary: its lanes, its first record and the record past it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub lanes: Vec<Lane>,
    pub start: usize,
    pub end: usize,
}

/// Pass 1: chunk `start..stop`'s summary, its first record at depth `depth0`: from each place its first record can be
/// at, the walk forked where it closes a level below its entry, once for each place a level returns to.
pub fn lanes(src: &[u8], lx: &crate::tier1::Lexed, start: usize, stop: usize, depth0: usize) -> Summary {
    let places: &[u8] = if start == 0 { &[0] } else { CYCLES };
    // Deeper than the walk goes: the serial walk refused the file before this chunk.
    if depth0 > DEPTH {
        let lanes = places.iter().map(|&st0| Lane::of(st0, Vec::new(), Vec::new(), Entry::default(), Some(Err(lx.starts[start])), Shadow::default())).collect();
        return Summary { lanes, start, end: stop };
    }
    let mut work: Vec<(u8, Vec<u8>, std::boxed::Box<[u8; DEPTH]>, usize, Entry, Shadow)> =
        places.iter().map(|&st| (st, Vec::new(), std::boxed::Box::new([0u8; DEPTH]), depth0, Entry { i: start, e: 0, depth: depth0, st }, Shadow::default())).collect();
    let mut out = Summary { lanes: Vec::new(), start, end: stop };
    while let Some((st0, kc, mut ret, low, en, mut count)) = work.pop() {
        // SAFETY: the records and the padded source are tier 1's, as the serial walk's.
        match unsafe { run_range(src, lx, &mut count, en, &mut *ret, low, stop) } {
            Stop::At(x) => {
                let p = ret[low..x.depth].to_vec();
                out.lanes.push(Lane::of(st0, kc, p, x, None, count));
            }
            Stop::Done(r) => out.lanes.push(Lane::of(st0, kc, Vec::new(), Entry::default(), Some(r), count)),
            Stop::Read(x) => {
                for &k in RETS {
                    let mut kc = kc.clone();
                    kc.push(k);
                    let mut r = ret.clone();
                    r[x.depth] = k;
                    work.push((st0, kc, r, x.depth, Entry { st: k, ..x }, count.clone()));
                }
            }
        }
    }
    out
}

/// Lane `a` then lane `b`, entered where `a` leaves: `b`'s closed levels matched against those `a` leaves open, the
/// rest of them closed below `a`'s entry. None where they do not meet.
fn then(a: &Lane, b: &Lane) -> Option<Lane> {
    if a.end.is_some() {
        return Some(a.clone());
    }
    if b.st0 != a.exit.st {
        return None;
    }
    let m = b.kc.len().min(a.p.len());
    if (0..m).any(|j| b.kc[j] != a.p[a.p.len() - 1 - j]) {
        return None;
    }
    let mut kc = a.kc.clone();
    kc.extend_from_slice(&b.kc[m..]);
    let mut p = a.p[..a.p.len() - m].to_vec();
    p.extend_from_slice(&b.p);
    // The frames: `b`'s closed ones matched against those `a` leaves open, the rest closed below `a`'s entry; what
    // `b` wrote into the one it is then in, that frame's.
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
    let last_start = b.last_start.or(a.last_start);
    Some(Lane { st0: a.st0, kc, p, exit: b.exit, nodes: a.nodes + b.nodes, end: b.end, fkc: a.fkc + b.fkc - fm, touch, pf, last_start })
}

/// Two ranges' summaries, one after the other. Associative, so a prefix scan composes them in any tree.
pub fn compose(a: &Summary, b: &Summary) -> Summary {
    let mut lanes: Vec<Lane> = Vec::new();
    for la in &a.lanes {
        if la.end.is_none() {
            debug_assert_eq!(la.exit.i, b.start, "a lane leaves at the next range's first record");
        }
        let next: Vec<Lane> = if la.end.is_some() { vec![la.clone()] } else { b.lanes.iter().filter_map(|lb| then(la, lb)).collect() };
        for l in next {
            if !lanes.contains(&l) {
                lanes.push(l);
            }
        }
    }
    Summary { lanes, start: a.start, end: b.end }
}

/// The walk's state at a range's start: its place, the levels' places, the nodes before it; or the file's end.
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

/// The state range `s` leaves after state `a` at its start: the lane of `a`'s place whose closed levels are `a`'s.
/// None where no lane meets it.
pub fn apply(s: &Summary, a: &At) -> Option<At> {
    if a.end.is_some() {
        return Some(a.clone());
    }
    let d = a.ret.len();
    let l = s.lanes.iter().find(|l| l.st0 == a.entry.st && l.kc.len() <= d && (0..l.kc.len()).all(|j| l.kc[j] == a.ret[d - 1 - j]))?;
    let mut ret = a.ret[..d - l.kc.len()].to_vec();
    ret.extend_from_slice(&l.p);
    let at = |x: u32| x + a.nodes;
    let nf = a.frames.len().checked_sub(l.fkc)?;
    let mut frames = a.frames[..nf].to_vec();
    if let Some(f) = frames.last_mut() {
        let t = l.touch[l.fkc];
        f.kids += t.kids;
        if let Some(x) = t.last {
            f.last = at(x);
        }
    }
    frames.extend(l.pf.iter().map(|f| Frame { node: at(f.node), last: if f.last == NONE { NONE } else { at(f.last) }, ..*f }));
    Some(At { entry: l.exit, ret, nodes: a.nodes + l.nodes, end: l.end, frames, prev_start: l.last_start.unwrap_or(a.prev_start) })
}

/// Pass 3: each chunk's state at its start, then the file's end, from the chunks' summaries, by a prefix scan of their compositions: round
/// `r` composes each chunk's summary after the one `2^r` chunks before it (Hillis and Steele's, as tier 1's
/// `entries`), each chunk on its own in a round.
pub fn entries(sums: &[Summary]) -> Vec<Option<At>> {
    let mut acc: Vec<Summary> = sums.to_vec();
    let mut d = 1;
    while d < acc.len() {
        let next: Vec<Summary> = (0..acc.len()).map(|c| if c < d { acc[c].clone() } else { compose(&acc[c - d], &acc[c]) }).collect();
        acc = next;
        d *= 2;
    }
    (0..=acc.len()).map(|c| if c == 0 { Some(At::start()) } else { apply(&acc[c - 1], &At::start()) }).collect()
}

/// Every chunk's summary (pass 1), of chunks of about `size` records.
pub fn summaries(src: &[u8], lx: &crate::tier1::Lexed, size: usize) -> Vec<Summary> {
    let chunks = chunk_starts(src, lx, size);
    let ds = depths(src, lx, &chunks);
    (0..chunks.len()).map(|c| lanes(src, lx, chunks[c], chunks.get(c + 1).copied().unwrap_or(lx.count + 1), ds[c])).collect()
}

/// The serial walk's state at each chunk's start, walked chunk after chunk from the file's, then at the file's end
/// (the test of pass 3).
pub fn serial_entries(src: &[u8], lx: &crate::tier1::Lexed, size: usize) -> Vec<At> {
    let chunks = chunk_starts(src, lx, size);
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
        // SAFETY: as `lanes`'.
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
pub unsafe fn write_range(src: &[u8], lx: &crate::tier1::Lexed, state: *mut u8, at: &At, stop: usize) -> Stop {
    let mut ret = std::boxed::Box::new([0u8; DEPTH]);
    ret[..at.ret.len()].copy_from_slice(&at.ret);
    // The frames' room: those open at its entry, and as many as a chunk opens at most.
    let mut buf: Vec<u8> = Vec::with_capacity(((DEPTH + 1) * 3 + 1 + at.frames.len()) * 24);
    let frames: Vec<(u32, u32, u32, u64)> = at.frames.iter().map(|f| (f.node, f.kids, f.last, f.start)).collect();
    // SAFETY: the caller's; the frames' room is `buf`'s.
    unsafe {
        let mut d = Cursor::<Cells, true>::resume(state, buf.as_mut_ptr(), at.nodes, at.prev_start, &frames);
        run_range(src, lx, &mut d, at.entry, &mut *ret, 0, stop)
    }
}

/// `src`'s tape into `w` from the middle: each chunk of about `size` records summarised (pass 1), each chunk's state by
/// a prefix scan of them (pass 3), then each chunk's nodes written from its state (pass 2), the chunks in the order
/// `order` gives (each on its own: any order writes the same tape). Or the byte it was refused at.
pub fn parse_middle(src: &[u8], lx: &crate::tier1::Lexed, w: &mut Writer, size: usize, order: impl Fn(usize) -> Vec<usize>) -> Result<(), u32> {
    if lx.refused {
        return Err(0);
    }
    if lx.count == 0 {
        return Err(lx.len as u32);
    }
    let chunks = chunk_starts(src, lx, size);
    let sums = summaries(src, lx, size);
    let ats = entries(&sums);
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
    for c in order(chunks.len()) {
        let at = ats[c].as_ref().ok_or(0u32)?;
        let stop = chunks.get(c + 1).copied().unwrap_or(lx.count + 1);
        // SAFETY: the state's room is reserved and its blocks made; `at` is pass 3's.
        match unsafe { write_range(src, lx, state, at, stop) } {
            Stop::At(x) if x.i == stop => {}
            Stop::Done(Ok(())) if c + 1 == chunks.len() => {}
            _ => return Err(u32::MAX),
        }
    }
    w.end_middle(last.nodes as usize, last.prev_start);
    match w.failure() {
        None => Ok(()),
        Some(_) => Err(u32::MAX),
    }
}
"#;
