
// Tier 2 as phases (decision 0726's tier 2 as phases). Each phase runs a block operator over fixed-size chunks (of
// records, or of items in level order), each reading only its chunk, the arrays earlier phases wrote and their sums
// at its start; returns a fixed-size summary; and may write an array. One exclusive prefix sum of the summaries by
// the phase's associative merge gives each chunk its sum, the next phase's input. Nothing is carried from one chunk to
// the next, and nothing forks.
//
// - P1 depth: each chunk's change of depth and its lowest depth below its start; writes each record's depth and its
//   items (an opener two: one in its parent's level, one as its own level's head).
// - P2 the level sort: two phases of keyed counts (an 8-bit digit of the level a phase, 256 counts a chunk, merged
//   elementwise), each item scattered at its digit's offset.
// - P3a containers: over the items in level order, a segmented last-value of the heads: each record's container.
// - P4 places: each chunk's map over the split loop's places; a pop below its start reads the return place of its
//   containers' kinds (P3a): known, so nothing forks.
// - P5a shape: each chunk from its place: its writer events, nodes, and its change of frame depth and lowest frame
//   depth below its start (the writer's frames are a stack of their own: a JSON object opens its node and a pair).
// - P5b events: each chunk again, each event written at its number (P5a's sum), with its frame depth and its items (an
//   open two: a child in its frame's level, and its own level's head; a child one).
// - P7 the frame sort: the events' items by frame depth, as P2.
// - P8a frames, P8b children: over the events' items in frame order, the segmented last-value of the heads (each
//   item's frame) and the segmented count and last-value of the children's nodes.
// - P6 rows: each chunk from its state (P1-P8), by `write_step`.

/// A place map's refusal.
pub const STEP_REFUSED: u8 = 255;
/// A place map's end of the file.
pub const STEP_ENDS: u8 = 254;
/// No record.
const NO: u32 = u32::MAX;

/// The exclusive prefix sum of `outs` by `merge` (`unit` its identity): each chunk's value the merge of all before it.
/// The library's one scan (`rerail_tape_format::phases::prefix`), the same for every phase.
pub fn prefix<T: Clone>(outs: &[T], unit: T, order: Order<'_>, merge: impl Fn(&T, &T) -> T) -> Vec<T> {
    rerail_tape_format::phases::prefix(outs, unit, order, merge).0
}

/// The chunks of `n` elements, `size` each: their bounds.
fn chunks_of(n: usize, size: usize) -> Vec<(usize, usize)> {
    let size = size.max(1);
    (0..n.div_ceil(size).max(1)).map(|c| (c * size, ((c + 1) * size).min(n))).collect()
}

/// P1's summary: the change of depth over a chunk, and the lowest depth reached below its start (0 or less); the
/// items it writes. `(d, m) · (d', m') = (d + d', min(m, d + m'))`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Depth {
    pub d: i64,
    pub m: i64,
    pub items: u32,
}

impl Depth {
    pub fn merge(a: &Depth, b: &Depth) -> Depth {
        Depth { d: a.d + b.d, m: a.m.min(a.d + b.m), items: a.items + b.items }
    }
}

/// An item of the level order: its level, its record, and whether it is its record's level's head.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Item {
    pub level: u32,
    pub record: u32,
    pub head: bool,
}

/// P3a's summary: a segmented last-value of the heads (a level's first item resets it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Last {
    pub reset: bool,
    pub v: u32,
}

impl Last {
    pub const UNIT: Last = Last { reset: false, v: NO };
    pub fn merge(a: &Last, b: &Last) -> Last {
        if b.reset { *b } else { Last { reset: a.reset, v: if b.v != NO { b.v } else { a.v } } }
    }
}

/// P3b's summary: a segmented count of children and the last child's node (a level's first item and each head reset
/// it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Kids {
    pub reset: bool,
    pub count: u32,
    pub last: u32,
}

impl Kids {
    pub const UNIT: Kids = Kids { reset: false, count: 0, last: NO };
    pub fn merge(a: &Kids, b: &Kids) -> Kids {
        if b.reset { *b } else { Kids { reset: a.reset, count: a.count + b.count, last: if b.last != NO { b.last } else { a.last } } }
    }
}

/// A writer event, as P5's walk writes it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Event {
    /// 1 a node opened (a child of the frame open, and a frame), 2 a child, 3 a frame closed.
    pub what: u8,
    /// The frames open before it.
    pub f: u32,
    /// Its first node (within its walk, then the file's), and its nodes.
    pub node: u32,
    pub nodes: u32,
    /// The node's start; the start of its last node.
    pub start: u64,
    pub last: u64,
}

/// P5a's summary: nodes, events and their items; the change of frame depth and the lowest frame depth below the
/// start (as `Depth`); the start of the last node and the last token's end (last-values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shape {
    pub nodes: u32,
    pub events: u32,
    pub items: u32,
    pub net: i64,
    pub low: i64,
    pub last_start: Option<u64>,
    pub e: Option<usize>,
}

impl Shape {
    pub const UNIT: Shape = Shape { nodes: 0, events: 0, items: 0, net: 0, low: 0, last_start: None, e: None };
    pub fn merge(a: &Shape, b: &Shape) -> Shape {
        Shape {
            nodes: a.nodes + b.nodes,
            events: a.events + b.events,
            items: a.items + b.items,
            net: a.net + b.net,
            low: a.low.min(a.net + b.low),
            last_start: b.last_start.or(a.last_start),
            e: b.e.or(a.e),
        }
    }

    /// One event's.
    fn of(ev: &Event) -> Shape {
        match ev.what {
            1 => Shape { nodes: ev.nodes, events: 1, items: 2, net: 1, low: 0, last_start: Some(ev.last), e: None },
            2 => Shape { nodes: ev.nodes, events: 1, items: 1, net: 0, low: 0, last_start: Some(ev.last), e: None },
            _ => Shape { nodes: 0, events: 1, items: 0, net: -1, low: -1, last_start: None, e: None },
        }
    }
}

/// A place map: each of the split loop's places at a chunk's start to its place past it, or `STEP_REFUSED` or
/// `STEP_ENDS`. `a · b` maps through `a`, then `b`.
pub type Places = [u8; 32];

pub fn places_merge(a: &Places, b: &Places) -> Places {
    let mut out = [STEP_REFUSED; 32];
    for p in 0..STEP_PLACES {
        out[p] = if (a[p] as usize) < STEP_PLACES { b[a[p] as usize] } else { a[p] };
    }
    out
}

/// The identity place map.
pub fn places_unit() -> Places {
    let mut out = [STEP_REFUSED; 32];
    for (p, o) in out.iter_mut().enumerate().take(STEP_PLACES) {
        *o = p as u8;
    }
    out
}

/// What a walk writes, through the token pass's answers: nothing (`events: None`), or each event.
struct Walk<'a> {
    toks: &'a [Token],
    events: Vec<Event>,
    nodes: u32,
}

impl Out for Walk<'_> {
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
    unsafe fn skip(&mut self, start: u64, t: Token) -> usize {
        self.events.push(Event { what: 2, f: 0, node: self.nodes, nodes: t.nodes, start, last: u64::from(t.last) });
        self.nodes += t.nodes;
        t.end as usize
    }
    unsafe fn open(&mut self, _: u32, start: u64) {
        self.events.push(Event { what: 1, f: 0, node: self.nodes, nodes: 1, start, last: start });
        self.nodes += 1;
    }
    unsafe fn leaf(&mut self, _: u32, start: u64, _: u64) {
        self.events.push(Event { what: 2, f: 0, node: self.nodes, nodes: 1, start, last: start });
        self.nodes += 1;
    }
    unsafe fn leaf_in(&mut self, _: u32, _: u32, start: u64, cs: u64, _: u64, _: u64) {
        self.events.push(Event { what: 2, f: 0, node: self.nodes, nodes: 2, start, last: cs });
        self.nodes += 2;
    }
    unsafe fn close(&mut self, _: u64) {
        self.events.push(Event { what: 3, f: 0, node: self.nodes, nodes: 0, start: 0, last: 0 });
    }
}

/// The phases' arrays and sums for one file: what P1-P5 give, from which each chunk's state is read.
pub struct Phases {
    /// Records `0..=n`, the last the file's end.
    pub n: usize,
    pub size: usize,
    /// Each record's depth before it (P1's), and its walk depth (an empty container's closer in its parent's level);
    /// the depths biased, as the operators take them.
    pub depth: Vec<i64>,
    pub depth_w: Vec<u32>,
    /// Each chunk's P1 summary.
    pub p1: Vec<Depth>,
    /// The items in level order (P2), and each record's head item's place there (`NO`: none).
    pub items: Vec<Item>,
    pub head_at: Vec<u32>,
    /// Each record's container: the opener of the level its item is in (P3a; `NO`: the root's level).
    pub container: Vec<u32>,
    /// Each chunk's place at its start (P4's sum at place 0), and the chunk it is refused in.
    pub place: Vec<u8>,
    pub refused: Option<usize>,
    /// Each chunk's shape and its sum (P5a); the events in file order (P5b).
    pub shapes: Vec<Shape>,
    pub shape_at: Vec<Shape>,
    pub events: Vec<Event>,
    /// The events' items in frame order (P7: `record` is the event's number), each one's frame's opening event (P8a)
    /// and its frame's children so far (P8b, inclusive).
    pub eitems: Vec<Item>,
    pub frame_of: Vec<u32>,
    pub kids: Vec<Kids>,
    /// The events' items, their frames' children and the events as words, as the operators take them.
    pub eitems_w: Vec<u32>,
    pub kids_w: Vec<u32>,
    pub events_w: Vec<u32>,
}

// ---- The phases' block operators in closed form: u32 words and pointers, as the UKL printer prints them (one
// source for the CPU's phases and the device's, decision 0726 step 4) ----

/// The bias of a depth held in a u32: depth `d` is `D0 + d`, so a chunk's depths below its start stay unsigned, and
/// two biased words add to under 2^32 (a merge's `a + b - D0`), for files of under `D0 / 2` records (`MOST_RECORDS`).
pub const D0: u32 = 1 << 30;
/// The most records the phases take: past it, every depth's sum would not fit its word.
pub const MOST_RECORDS: usize = 1 << 29;

/// A word of a phase's record: a chunk's output, a sum (a tree's node on the device, `<ukl/phases_run.h>`).
pub type Rec = u32;

/// Record `r`'s bracket code (`n` the file's records; `n` its end, none): twice its kind, plus one where it opens;
/// `NONE` for none.
pub fn bracket_code(src: &[u8], starts: &[u32], n: usize, r: usize) -> u32 {
    if r >= n {
        return NONE;
    }
    bracket_byte(src[starts[r] as usize])
}

/// A byte's bracket code (`bracket`'s, as one word: the printer's `bracket`).
pub fn bracket_byte(b: u8) -> u32 {
    match bracket(b) {
        Some((k, open)) => 2 * u32::from(k) + u32::from(open),
        None => NONE,
    }
}

/// Whether record `r` closes an empty container: its record before opens it (the same kind, opening).
pub fn empty_close_at(src: &[u8], starts: &[u32], n: usize, r: usize) -> bool {
    if r == 0 {
        return false;
    }
    let x = bracket_code(src, starts, n, r);
    let y = bracket_code(src, starts, n, r - 1);
    x != NONE && x % 2 == 0 && y == x + 1
}

/// P1's unit: no change of depth, nothing below the start, no items.
///
/// # Safety
/// `out` has room for three words.
pub unsafe fn p1_unit(out: *mut Rec) {
    unsafe { *out.add(0) = D0 };
    unsafe { *out.add(1) = D0 };
    unsafe { *out.add(2) = 0 };
}

/// P1, records `a..b` of `n`: the chunk's change of depth (`D0`-biased) and its lowest depth below its start
/// (biased, at most `D0`), and the items it writes, at `out[0..3]`.
///
/// # Safety
/// `out` has room for three words.
pub unsafe fn p1_run(src: &[u8], starts: &[u32], n: usize, a: usize, b: usize, out: *mut Rec) {
    let mut d = D0;
    let mut m = D0;
    let mut k = 0u32;
    for r in a..b {
        let x = bracket_code(src, starts, n, r);
        if x == NONE {
            k += 1;
        } else if x % 2 == 1 {
            d += 1;
            k += 2;
        } else {
            d -= 1;
            k += 1;
            if d < m {
                m = d;
            }
        }
    }
    unsafe { *out.add(0) = d };
    unsafe { *out.add(1) = m };
    unsafe { *out.add(2) = k };
}

/// P1's merge: `(d, m) · (d', m') = (d + d', min(m, d + m'))`, each biased; the items added.
///
/// # Safety
/// `a` and `b` hold three words each, `out` has room for three.
pub unsafe fn p1_merge(a: &[Rec], b: &[Rec], out: *mut Rec) {
    let d = a[0] + b[0] - D0;
    let mb = a[0] + b[1] - D0;
    let m = if mb < a[1] { mb } else { a[1] };
    unsafe { *out.add(0) = d };
    unsafe { *out.add(1) = m };
    unsafe { *out.add(2) = a[2] + b[2] };
}

/// P1's writes, records `a..b` from the chunk's sum `sum` (P1's prefix: the depth before it, biased, and its first
/// item): each record's depth (biased) at `depth[r]`, and its items at `items[3k..]` (level, record, head): an
/// opener two (its parent's level, and its own level's head), a closer one (an empty container's in its parent's
/// level), any other record one; a record that opens nothing no head item (`head_at[r]`, `NONE`: P3 writes the
/// openers').
///
/// # Safety
/// `depth` and `head_at` have room for `b` words, `items` for the chunk's items from its first.
pub unsafe fn p1_write(src: &[u8], starts: &[u32], n: usize, a: usize, b: usize, sum: &[Rec], depth: *mut u32, items: *mut u32, head_at: *mut u32) {
    let mut d = sum[0];
    let mut k = sum[2] as usize;
    for r in a..b {
        unsafe { *depth.add(r) = d };
        let x = bracket_code(src, starts, n, r);
        // The level of its item in the level order: its depth (an empty container's closer one less), at least 0.
        let mut level = if d > D0 { d - D0 } else { 0 };
        if x != NONE && x % 2 == 0 && empty_close_at(src, starts, n, r) {
            level = if d > D0 + 1 { d - D0 - 1 } else { 0 };
        }
        unsafe { *items.add(3 * k) = level };
        unsafe { *items.add(3 * k + 1) = r as u32 };
        unsafe { *items.add(3 * k + 2) = 0 };
        k += 1;
        if x != NONE && x % 2 == 1 {
            unsafe { *items.add(3 * k) = d - D0 + 1 };
            unsafe { *items.add(3 * k + 1) = r as u32 };
            unsafe { *items.add(3 * k + 2) = 1 };
            k += 1;
            d += 1;
        } else {
            unsafe { *head_at.add(r) = NONE };
            if x != NONE {
                d -= 1;
            }
        }
    }
}

/// The keys of a level sort's digit.
pub const KEYS: usize = 256;

/// A sum of one word: no count.
///
/// # Safety
/// `out` has room for a word.
pub unsafe fn add1_unit(out: *mut Rec) {
    unsafe { *out = 0 };
}

/// One word's sum.
///
/// # Safety
/// `a` and `b` hold a word each, `out` has room for one.
pub unsafe fn add1_merge(a: &[Rec], b: &[Rec], out: *mut Rec) {
    unsafe { *out = a[0] + b[0] };
}

/// P2's unit: no key counted.
///
/// # Safety
/// `out` has room for `KEYS` words.
pub unsafe fn p2_unit(out: *mut Rec) {
    for g in 0..KEYS {
        unsafe { *out.add(g) = 0 };
    }
}

/// P2's merge: the keys' counts added.
///
/// # Safety
/// `a` and `b` hold `KEYS` words each, `out` has room for as many.
pub unsafe fn p2_merge(a: &[Rec], b: &[Rec], out: *mut Rec) {
    for g in 0..KEYS {
        unsafe { *out.add(g) = a[g] + b[g] };
    }
}

/// P2, items `a..b` (three words each: level, record, head), the digit at `shift` of each one's level: each key's
/// count.
///
/// # Safety
/// `items` holds the chunk's items, `out` has room for `KEYS` words.
pub unsafe fn p2_run(items: &[u32], a: usize, b: usize, shift: u32, out: *mut Rec) {
    unsafe { p2_unit(out) };
    for x in a..b {
        let g = ((items[3 * x] >> shift) & 0xff) as usize;
        unsafe { *out.add(g) += 1 };
    }
}

/// P2's writes, items `a..b`: each at its key's start (`keys`) and the chunk's count of its key before it (its sum,
/// `sum`, counted on in place).
///
/// # Safety
/// `sum` is the chunk's own (`KEYS` words), `out` has room for every item.
pub unsafe fn p2_write(items: &[u32], a: usize, b: usize, shift: u32, sum: *mut Rec, keys: &[u32], out: *mut u32) {
    for x in a..b {
        let g = ((items[3 * x] >> shift) & 0xff) as usize;
        let at = (keys[g] + unsafe { *sum.add(g) }) as usize;
        unsafe { *out.add(3 * at) = items[3 * x] };
        unsafe { *out.add(3 * at + 1) = items[3 * x + 1] };
        unsafe { *out.add(3 * at + 2) = items[3 * x + 2] };
        unsafe { *sum.add(g) += 1 };
    }
}

/// Whether item `x` of the level order starts its level (`items`, three words each): a read of its neighbour.
pub fn starts_level(items: &[u32], x: usize) -> bool {
    x == 0 || items[3 * (x - 1)] != items[3 * x]
}

/// A segmented last-value's unit (`Last`'s: no reset, no value), two words.
///
/// # Safety
/// `out` has room for two words.
pub unsafe fn last_unit(out: *mut Rec) {
    unsafe { *out.add(0) = 0 };
    unsafe { *out.add(1) = NONE };
}

/// `Last::merge` in words: `b` where it resets, else `a`'s reset and `b`'s value where it has one, else `a`'s.
///
/// # Safety
/// `a` and `b` hold two words each, `out` has room for two.
pub unsafe fn last_merge(a: &[Rec], b: &[Rec], out: *mut Rec) {
    if b[0] != 0 {
        unsafe { *out.add(0) = b[0] };
        unsafe { *out.add(1) = b[1] };
    } else {
        unsafe { *out.add(0) = a[0] };
        unsafe { *out.add(1) = if b[1] != NONE { b[1] } else { a[1] } };
    }
}

/// P3 (and P8a), items `a..b` in level order: the segmented last-value of the heads' records (a level's first item
/// resets it), at `out[0..2]`.
///
/// # Safety
/// `items` holds the chunk's items, `out` has room for two words.
pub unsafe fn p3_run(items: &[u32], a: usize, b: usize, out: *mut Rec) {
    let mut reset = 0;
    let mut v = NONE;
    for x in a..b {
        let head = items[3 * x + 2] != 0;
        if starts_level(items, x) {
            reset = 1;
            v = NONE;
        }
        if head {
            v = items[3 * x + 1];
        }
    }
    unsafe { *out.add(0) = reset };
    unsafe { *out.add(1) = v };
}

/// P3's writes, items `a..b` from the chunk's sum: each record's container (the head before its item in its level,
/// `NONE` for the root's level) at `container[record]` for a record's item that is no head, and each head's place
/// at `head_at[record]`.
///
/// # Safety
/// `container` and `head_at` have room for every record.
pub unsafe fn p3_write(items: &[u32], a: usize, b: usize, sum: &[Rec], container: *mut u32, head_at: *mut u32) {
    let mut v = sum[1];
    for x in a..b {
        let head = items[3 * x + 2] != 0;
        let r = items[3 * x + 1] as usize;
        if starts_level(items, x) {
            v = NONE;
        }
        if head {
            v = items[3 * x + 1];
            unsafe { *head_at.add(r) = x as u32 };
        } else {
            unsafe { *container.add(r) = v };
        }
    }
}

/// P8a's writes, items `a..b` in frame order from the chunk's sum: each item's frame (the head before it in its
/// level, inclusive: a head its own) at `frame_of[x]`.
///
/// # Safety
/// `frame_of` has room for every item.
pub unsafe fn p8a_write(items: &[u32], a: usize, b: usize, sum: &[Rec], frame_of: *mut u32) {
    let mut v = sum[1];
    for x in a..b {
        if starts_level(items, x) {
            v = NONE;
        }
        if items[3 * x + 2] != 0 {
            v = items[3 * x + 1];
        }
        unsafe { *frame_of.add(x) = v };
    }
}

/// `Kids`' unit in words (reset, count, last child's node).
///
/// # Safety
/// `out` has room for three words.
pub unsafe fn kids_unit(out: *mut Rec) {
    unsafe { *out.add(0) = 0 };
    unsafe { *out.add(1) = 0 };
    unsafe { *out.add(2) = NONE };
}

/// `Kids::merge` in words: `b` where it resets, else `a`'s reset, the counts added, `b`'s last where it has one.
///
/// # Safety
/// `a` and `b` hold three words each, `out` has room for three.
pub unsafe fn kids_merge(a: &[Rec], b: &[Rec], out: *mut Rec) {
    if b[0] != 0 {
        unsafe { *out.add(0) = b[0] };
        unsafe { *out.add(1) = b[1] };
        unsafe { *out.add(2) = b[2] };
    } else {
        unsafe { *out.add(0) = a[0] };
        unsafe { *out.add(1) = a[1] + b[1] };
        unsafe { *out.add(2) = if b[2] != NONE { b[2] } else { a[2] } };
    }
}

/// P8b, items `a..b` in frame order: the segmented count of children and the last one's node (`events`, 8 words
/// each, the node its third), a level's first item and each head resetting it.
///
/// # Safety
/// `out` has room for three words.
pub unsafe fn p8b_run(items: &[u32], events: &[u32], a: usize, b: usize, out: *mut Rec) {
    let mut reset = 0;
    let mut count = 0;
    let mut last = NONE;
    for x in a..b {
        let head = items[3 * x + 2] != 0;
        if starts_level(items, x) || head {
            reset = 1;
            count = 0;
            last = NONE;
        }
        if !head {
            count += 1;
            last = events[8 * items[3 * x + 1] as usize + 2];
        }
    }
    unsafe { *out.add(0) = reset };
    unsafe { *out.add(1) = count };
    unsafe { *out.add(2) = last };
}

/// P8b's writes, items `a..b` from the chunk's sum: each item's frame's children so far, inclusive (reset, count,
/// last child's node), at `kids[3x..]`.
///
/// # Safety
/// `kids` has room for every item's three words.
pub unsafe fn p8b_write(items: &[u32], events: &[u32], a: usize, b: usize, sum: &[Rec], kids: *mut u32) {
    let mut reset = sum[0];
    let mut count = sum[1];
    let mut last = sum[2];
    for x in a..b {
        let head = items[3 * x + 2] != 0;
        if starts_level(items, x) || head {
            reset = 1;
            count = 0;
            last = NONE;
        }
        if !head {
            count += 1;
            last = events[8 * items[3 * x + 1] as usize + 2];
        }
        unsafe { *kids.add(3 * x) = reset };
        unsafe { *kids.add(3 * x + 1) = count };
        unsafe { *kids.add(3 * x + 2) = last };
    }
}

/// A writer frame's words (`<ukl/writer_mid.h>`'s): its node, its children so far, its last child (`NONE`: none), its
/// start's low and high words.
pub const FRAME_W: usize = 5;

/// The frames chunk `c` writes into at its start (P8's), outermost first, at `out` (`FRAME_W` words each): its sum
/// `at` (P5's) says the frames open before it and its events before it, its own shape's lowest frame depth (`low`,
/// biased) how many of them it reaches. Frame `k`'s is level `k + 1` of the events' items in frame order (`eitems`,
/// `ne` of them), its last item before the chunk's first event (a bounded search): that item's frame's opening event
/// and its children so far. Their count.
///
/// # Safety
/// `out` has room for the frames.
pub unsafe fn chunk_frames(eitems: &[u32], ne: usize, frame_of: &[u32], kids: &[u32], events: &[u32], at: &[u32], low: u32, out: *mut u32) -> usize {
    let fd = (at[3] - D0) as usize;
    let up = (D0 + 1 - low) as usize;
    let need = if up < fd { up } else { fd };
    let mut w = 0;
    for k in fd - need..fd {
        let level = (k + 1) as u32;
        let mut lo = 0;
        let mut hi = ne;
        while lo < hi {
            let mid = (lo + hi) / 2;
            let l = eitems[3 * mid];
            if l < level || (l == level && eitems[3 * mid + 1] < at[1]) {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let x = lo - 1;
        let head = eitems[3 * x + 2] != 0;
        let o = frame_of[x] as usize;
        unsafe { *out.add(FRAME_W * w) = events[8 * o + 2] };
        unsafe { *out.add(FRAME_W * w + 1) = if head { 0 } else { kids[3 * x + 1] } };
        unsafe { *out.add(FRAME_W * w + 2) = if head { NONE } else { kids[3 * x + 2] } };
        unsafe { *out.add(FRAME_W * w + 3) = events[8 * o + 4] };
        unsafe { *out.add(FRAME_W * w + 4) = events[8 * o + 5] };
        w += 1;
    }
    w
}

/// The return place of a level of kind `k` (`STEP_CONT`'s, as the printer's table).
pub fn step_cont(k: usize) -> u32 {
    step_cont_t(k)
}

/// `STEP_CONT`'s entry `k` (the printer prints the table as this function).
fn step_cont_t(k: usize) -> u32 {
    u32::from(STEP_CONT[k])
}

/// Record `r`'s walk depth: its depth (`depth`, biased) less an empty container's closer, at least 0.
pub fn walk_depth_at(src: &[u8], starts: &[u32], n: usize, depth: &[u32], r: usize) -> usize {
    let e = if empty_close_at(src, starts, n, r) { 1 } else { 0 };
    if depth[r] >= D0 + e { (depth[r] - D0 - e) as usize } else { 0 }
}

/// Chunk `a..`'s known levels: the return places of the levels it closes below its start (`low`, its P1 lowest depth,
/// biased), each from the kind of the container that holds it (P3's), at `ret[depth - len..depth]` (its walk depth's),
/// innermost last; `len`. None past the walk's levels (it is refused there).
pub fn known_rets(src: &[u8], starts: &[u32], n: usize, depth: &[u32], container: &[u32], low: u32, a: usize, ret: &mut [u8; DEPTH]) -> usize {
    let wd = walk_depth_at(src, starts, n, depth, a);
    if wd > STEP_DEPTH {
        return 0;
    }
    let e = if empty_close_at(src, starts, n, a) { 1 } else { 0 };
    let below = D0 - low;
    let closes = if below >= e { (below - e) as usize } else { 0 };
    let len = closes.min(wd);
    // The open levels' openers, innermost first: the record's container, its container's, ...
    let mut o = container[a];
    for i in 1..len + 1 {
        if o != NONE {
            o = container[o as usize];
        }
        let k = if o != NONE { bracket_code(src, starts, n, o as usize) / 2 } else { ROOT as u32 };
        ret[wd - i] = step_cont(k as usize) as u8;
    }
    len
}

/// The identity place map, a word a place.
///
/// # Safety
/// `out` has room for `STEP_PLACES` words.
pub unsafe fn places_unit_w(out: *mut Rec) {
    for p in 0..STEP_PLACES {
        unsafe { *out.add(p) = p as u32 };
    }
}

/// `a · b`: each place through `a`, then `b` (`STEP_REFUSED`, `STEP_ENDS` kept).
///
/// # Safety
/// `a` and `b` hold `STEP_PLACES` words each, `out` has room for as many.
pub unsafe fn places_merge_w(a: &[Rec], b: &[Rec], out: *mut Rec) {
    for p in 0..STEP_PLACES {
        let x = a[p];
        unsafe { *out.add(p) = if x < STEP_PLACES as u32 { b[x as usize] } else { x } };
    }
}

/// A shape's words (`Shape`): nodes, events, items, the frames' change and lowest (biased), the last node's start
/// (two words) and whether, the last token's end and whether.
pub const SHAPE_W: usize = 10;

/// No shape.
///
/// # Safety
/// `out` has room for `SHAPE_W` words.
pub unsafe fn shape_unit(out: *mut Rec) {
    for k in 0..SHAPE_W {
        unsafe { *out.add(k) = 0 };
    }
    unsafe { *out.add(3) = D0 };
    unsafe { *out.add(4) = D0 };
}

/// `Shape::merge` in words.
///
/// # Safety
/// `a` and `b` hold `SHAPE_W` words each, `out` has room for as many.
pub unsafe fn shape_merge(a: &[Rec], b: &[Rec], out: *mut Rec) {
    unsafe { *out.add(0) = a[0] + b[0] };
    unsafe { *out.add(1) = a[1] + b[1] };
    unsafe { *out.add(2) = a[2] + b[2] };
    unsafe { *out.add(3) = a[3] + b[3] - D0 };
    let lb = a[3] + b[4] - D0;
    unsafe { *out.add(4) = if lb < a[4] { lb } else { a[4] } };
    if b[7] != 0 {
        unsafe { *out.add(5) = b[5] };
        unsafe { *out.add(6) = b[6] };
        unsafe { *out.add(7) = 1 };
    } else {
        unsafe { *out.add(5) = a[5] };
        unsafe { *out.add(6) = a[6] };
        unsafe { *out.add(7) = a[7] };
    }
    if b[9] != 0 {
        unsafe { *out.add(8) = b[8] };
        unsafe { *out.add(9) = 1 };
    } else {
        unsafe { *out.add(8) = a[8] };
        unsafe { *out.add(9) = a[9] };
    }
}

/// A shape as its words.
pub fn shape_words(s: &Shape) -> [u32; SHAPE_W] {
    let ls = s.last_start.unwrap_or(0);
    [s.nodes, s.events, s.items, (i64::from(D0) + s.net) as u32, (i64::from(D0) + s.low) as u32, ls as u32, (ls >> 32) as u32, u32::from(s.last_start.is_some()), s.e.unwrap_or(0) as u32, u32::from(s.e.is_some())]
}

/// Words as a shape.
pub fn shape_of_words(w: &[u32; SHAPE_W]) -> Shape {
    Shape {
        nodes: w[0],
        events: w[1],
        items: w[2],
        net: i64::from(w[3]) - i64::from(D0),
        low: i64::from(w[4]) - i64::from(D0),
        last_start: (w[7] != 0).then_some(u64::from(w[5]) | (u64::from(w[6]) << 32)),
        e: (w[9] != 0).then_some(w[8] as usize),
    }
}

/// A record's bracket, by its first byte (`n`: the file's end, none).
fn bracket_of(src: &[u8], lx: &crate::tier1::Lexed, r: usize) -> Option<(u8, bool)> {
    if r < lx.count { bracket(src[lx.starts[r] as usize]) } else { None }
}

/// Whether record `r` closes an empty container (its record before opens it): one level the walk never opens.
fn empty_close(src: &[u8], lx: &crate::tier1::Lexed, r: usize) -> bool {
    r > 0 && matches!((bracket_of(src, lx, r - 1), bracket_of(src, lx, r)), (Some((a, true)), Some((b, false))) if a == b)
}

impl Phases {
    /// P1-P5 over `src`'s records in chunks of `size`, each phase's chunks in `order`'s order.
    pub fn run(src: &[u8], lx: &crate::tier1::Lexed, toks: &[Token], size: usize, order: Order<'_>) -> Phases {
        let n = lx.count;
        let rc = chunks_of(n + 1, size);
        // P1: each chunk's depth summary (`p1_run`), their prefix sum (`p1_merge`), each chunk's writes from its sum
        // (`p1_write`): the block operators the device runs too.
        let st = &lx.starts[..];
        let words: Vec<[u32; 3]> = each(rc.len(), order, |c| {
            let mut w = [0u32; 3];
            // SAFETY: three words.
            unsafe { p1_run(src, st, n, rc[c].0, rc[c].1, w.as_mut_ptr()) };
            w
        });
        let merge3 = |a: &[u32; 3], b: &[u32; 3]| {
            let mut w = [0u32; 3];
            // SAFETY: three words each.
            unsafe { p1_merge(a, b, w.as_mut_ptr()) };
            w
        };
        let mut unit = [0u32; 3];
        // SAFETY: three words.
        unsafe { p1_unit(unit.as_mut_ptr()) };
        let at3 = prefix(&words, unit, order, merge3);
        let total = merge3(&at3[rc.len() - 1], &words[rc.len() - 1]);
        let p1: Vec<Depth> = words.iter().map(|w| Depth { d: i64::from(w[0]) - i64::from(D0), m: i64::from(w[1]) - i64::from(D0), items: w[2] }).collect();
        let mut dw = vec![0u32; n + 1];
        let mut iw = vec![0u32; 3 * total[2] as usize];
        let mut head_at = vec![0u32; n + 1];
        let (dp, ip, hp) = (dw.as_mut_ptr(), iw.as_mut_ptr(), head_at.as_mut_ptr());
        each(rc.len(), order, |c| {
            // SAFETY: each chunk its own records' depths and heads, and its own items, at its sum.
            unsafe { p1_write(src, st, n, rc[c].0, rc[c].1, &at3[c], dp, ip, hp) };
        });
        let depth: Vec<i64> = dw.iter().map(|&d| i64::from(d) - i64::from(D0)).collect();
        let items: Vec<Item> = iw.chunks_exact(3).map(|w| Item { level: w[0], record: w[1], head: w[2] != 0 }).collect();
        // P2: the level sort, two 8-bit digits, each a phase of keyed counts over chunks of items. Within a level the
        // items keep their order: by record, an opener's head after its item in its parent's level.
        let items = sort_by_level(items, size, order);
        // P3: each record's container and each head's place, a segmented last-value of the heads over the items in
        // level order (`p3_run`, `last_merge`, `p3_write`).
        let ic = chunks_of(items.len(), size);
        let iw: Vec<u32> = items.iter().flat_map(|x| [x.level, x.record, u32::from(x.head)]).collect();
        let lasts: Vec<[u32; 2]> = each(ic.len(), order, |c| {
            let mut w = [0u32; 2];
            // SAFETY: two words; the chunk's items.
            unsafe { p3_run(&iw, ic[c].0, ic[c].1, w.as_mut_ptr()) };
            w
        });
        let merge2 = |a: &[u32; 2], b: &[u32; 2]| {
            let mut w = [0u32; 2];
            // SAFETY: two words each.
            unsafe { last_merge(a, b, w.as_mut_ptr()) };
            w
        };
        let mut unit2 = [0u32; 2];
        // SAFETY: two words.
        unsafe { last_unit(unit2.as_mut_ptr()) };
        let p3_at = prefix(&lasts, unit2, order, merge2);
        let mut container = vec![NO; n + 1];
        let (cp, hp) = (container.as_mut_ptr(), head_at.as_mut_ptr());
        each(ic.len(), order, |c| {
            // SAFETY: each record's item in one chunk only; its head in one.
            unsafe { p3_write(&iw, ic[c].0, ic[c].1, &p3_at[c], cp, hp) };
        });
        let mut ph = Phases { n, size, depth, depth_w: dw, p1, items, head_at, container, place: Vec::new(), refused: None, shapes: Vec::new(), shape_at: Vec::new(), events: Vec::new(), eitems: Vec::new(), frame_of: Vec::new(), kids: Vec::new(), eitems_w: Vec::new(), kids_w: Vec::new(), events_w: Vec::new() };
        // P4: each chunk's place map, its pops below its start reading its containers' return places.
        let maps: Vec<Places> = each(rc.len(), order, |c| {
            let mut m = [STEP_REFUSED; 32];
            for (p, o) in m.iter_mut().enumerate().take(STEP_PLACES) {
                *o = ph.walk(src, lx, toks, rc[c], p as u8, None);
            }
            m
        });
        let maps_w: Vec<[u32; STEP_PLACES]> = maps.iter().map(|m| core::array::from_fn(|p| u32::from(m[p]))).collect();
        let pmerge = |a: &[u32; STEP_PLACES], b: &[u32; STEP_PLACES]| {
            let mut w = [0u32; STEP_PLACES];
            // SAFETY: `STEP_PLACES` words each.
            unsafe { places_merge_w(a, b, w.as_mut_ptr()) };
            w
        };
        let mut punit = [0u32; STEP_PLACES];
        // SAFETY: `STEP_PLACES` words.
        unsafe { places_unit_w(punit.as_mut_ptr()) };
        let at = prefix(&maps_w, punit, order, pmerge);
        ph.place = each(rc.len(), order, |c| at[c][0] as u8);
        // The first chunk refused at its place: a min over the chunks.
        ph.refused = (0..rc.len()).filter(|&c| (ph.place[c] as usize) < STEP_PLACES && maps[c][ph.place[c] as usize] == STEP_REFUSED).min();
        if ph.refused.is_some() {
            return ph;
        }
        // P5a: each chunk from its place: its shape.
        let shape_of = |c: usize| -> (Shape, Vec<Event>) {
            // A chunk after the file's end (its place `STEP_ENDS`: the root ended before it) writes nothing.
            if ph.place[c] as usize >= STEP_PLACES {
                return (Shape::UNIT, Vec::new());
            }
            let mut w = (Vec::new(), None);
            ph.walk(src, lx, toks, rc[c], ph.place[c], Some(&mut w));
            let mut s = Shape::UNIT;
            let mut evs = Vec::new();
            for ev in w.0 {
                // Its frame depth and first node within the chunk.
                evs.push(Event { f: s.net as u32, node: s.nodes, ..ev });
                s = Shape::merge(&s, &Shape::of(&ev));
            }
            s.e = w.1;
            (s, evs)
        };
        let shapes: Vec<Shape> = each(rc.len(), order, |c| shape_of(c).0);
        let smerge = |a: &[u32; SHAPE_W], b: &[u32; SHAPE_W]| {
            let mut w = [0u32; SHAPE_W];
            // SAFETY: `SHAPE_W` words each.
            unsafe { shape_merge(a, b, w.as_mut_ptr()) };
            w
        };
        let mut sunit = [0u32; SHAPE_W];
        // SAFETY: `SHAPE_W` words.
        unsafe { shape_unit(sunit.as_mut_ptr()) };
        let shapes_w: Vec<[u32; SHAPE_W]> = shapes.iter().map(shape_words).collect();
        let shape_at: Vec<Shape> = prefix(&shapes_w, sunit, order, smerge).iter().map(shape_of_words).collect();
        let total = Shape::merge(&shape_at[rc.len() - 1], &shapes[rc.len() - 1]);
        // P5b: each chunk again, its events written at their numbers, with the file's frame depths and nodes, and
        // their items.
        let mut events = vec![Event::default(); total.events as usize];
        let mut eitems = vec![Item::default(); total.items as usize];
        for c in order(rc.len()) {
            let at = shape_at[c];
            let (_, evs) = shape_of(c);
            let (mut g, mut k) = (at.events as usize, at.items as usize);
            for ev in evs {
                let ev = Event { f: (at.net + i64::from(ev.f)) as u32, node: at.nodes + ev.node, ..ev };
                events[g] = ev;
                match ev.what {
                    1 => {
                        eitems[k] = Item { level: ev.f, record: g as u32, head: false };
                        eitems[k + 1] = Item { level: ev.f + 1, record: g as u32, head: true };
                        k += 2;
                    }
                    2 => {
                        eitems[k] = Item { level: ev.f, record: g as u32, head: false };
                        k += 1;
                    }
                    _ => {}
                }
                g += 1;
            }
        }
        ph.shapes = shapes;
        ph.shape_at = shape_at;
        ph.events = events;
        // P7: the events' items by frame depth.
        let eitems = sort_by_level(eitems, size, order);
        let ec = chunks_of(eitems.len(), size);
        // P8a: each item's frame (the opening event of the level it is in), a segmented last-value of the heads
        // (`p3_run`, `last_merge`, `p8a_write`).
        let ew: Vec<u32> = eitems.iter().flat_map(|x| [x.level, x.record, u32::from(x.head)]).collect();
        let lasts: Vec<[u32; 2]> = each(ec.len(), order, |c| {
            let mut w = [0u32; 2];
            // SAFETY: two words; the chunk's items.
            unsafe { p3_run(&ew, ec[c].0, ec[c].1, w.as_mut_ptr()) };
            w
        });
        let p8a_at = prefix(&lasts, unit2, order, merge2);
        let mut frame_of = vec![NO; eitems.len()];
        let fp = frame_of.as_mut_ptr();
        each(ec.len(), order, |c| {
            // SAFETY: each chunk its own items.
            unsafe { p8a_write(&ew, ec[c].0, ec[c].1, &p8a_at[c], fp) };
        });
        // P8b: each item's frame's children so far and the last one's node (`p8b_run`, `kids_merge`, `p8b_write`).
        let evw: Vec<u32> = ph.events.iter().flat_map(|e| [u32::from(e.what), e.f, e.node, e.nodes, e.start as u32, (e.start >> 32) as u32, e.last as u32, (e.last >> 32) as u32]).collect();
        let kmerge = |a: &[u32; 3], b: &[u32; 3]| {
            let mut w = [0u32; 3];
            // SAFETY: three words each.
            unsafe { kids_merge(a, b, w.as_mut_ptr()) };
            w
        };
        let mut kunit = [0u32; 3];
        // SAFETY: three words.
        unsafe { kids_unit(kunit.as_mut_ptr()) };
        let p8b: Vec<[u32; 3]> = each(ec.len(), order, |c| {
            let mut w = [0u32; 3];
            // SAFETY: three words; the chunk's items.
            unsafe { p8b_run(&ew, &evw, ec[c].0, ec[c].1, w.as_mut_ptr()) };
            w
        });
        let p8b_at = prefix(&p8b, kunit, order, kmerge);
        let mut kw = vec![0u32; 3 * eitems.len()];
        let kp = kw.as_mut_ptr();
        each(ec.len(), order, |c| {
            // SAFETY: each chunk its own items.
            unsafe { p8b_write(&ew, &evw, ec[c].0, ec[c].1, &p8b_at[c], kp) };
        });
        let kids: Vec<Kids> = kw.chunks_exact(3).map(|w| Kids { reset: w[0] != 0, count: w[1], last: w[2] }).collect();
        ph.eitems_w = ew;
        ph.kids_w = kw;
        ph.events_w = evw;
        ph.eitems = eitems;
        ph.frame_of = frame_of;
        ph.kids = kids;
        ph
    }

    /// Record `r`'s walk depth: its depth less an empty container's closer.
    fn walk_depth(&self, src: &[u8], lx: &crate::tier1::Lexed, r: usize) -> usize {
        (self.depth[r] - i64::from(r < lx.count && empty_close(src, lx, r))).max(0) as usize
    }

    /// The return places of the levels chunk `c` (first record `a`) closes below its start, innermost last (as
    /// `At::ret`), and its walk depth: `known_rets`'s.
    fn rets(&self, src: &[u8], lx: &crate::tier1::Lexed, c: usize, a: usize) -> (Vec<u8>, usize) {
        let dw = &self.depth_w;
        let low = (i64::from(D0) + self.p1[c].m) as u32;
        let mut ret = [0u8; DEPTH];
        let len = known_rets(src, &lx.starts, lx.count, dw, &self.container, low, a, &mut ret);
        let depth = walk_depth_at(src, &lx.starts, lx.count, dw, a);
        let known = if depth > STEP_DEPTH { Vec::new() } else { ret[depth - len..depth].to_vec() };
        (known, depth)
    }

    /// Chunk `(a, b)` walked from place `p` with its levels known: its place past it (or `STEP_REFUSED`,
    /// `STEP_ENDS`). With `events`, each event the walk makes, and the last token's end it leaves where it stops at a
    /// place (none past the file's end: nothing reads it).
    fn walk(&self, src: &[u8], lx: &crate::tier1::Lexed, toks: &[Token], (a, b): (usize, usize), p: u8, events: Option<&mut (Vec<Event>, Option<usize>)>) -> u8 {
        let c = a / self.size.max(1);
        let (known, depth) = self.rets(src, lx, c, a);
        // Deeper than the walk's levels: refused (the serial walk refuses at the bracket that passes them).
        if depth > STEP_DEPTH {
            return STEP_REFUSED;
        }
        let mut ret = [0u8; STEP_DEPTH];
        let base = depth - known.len();
        ret[base..depth].copy_from_slice(&known);
        let en = Entry { i: a, e: usize::MAX, depth, st: p };
        let mut d = Walk { toks, events: Vec::new(), nodes: 0 };
        // SAFETY: `ret` holds the levels below `base` it reads; `d` writes nothing.
        let s = unsafe { run_step(src, lx, &mut d, en, &mut ret, base, b) };
        let (out, e) = match s {
            Stop::At(x) if x.i >= b => (x.st, (x.e != usize::MAX).then_some(x.e)),
            Stop::Done(Ok(())) => (STEP_ENDS, None),
            _ => (STEP_REFUSED, None),
        };
        if let Some(ev) = events {
            *ev = (d.events, e);
        }
        out
    }

    /// Chunk `c`'s state at its start: its place, levels, first node, frames and the node before (P1-P5's sums).
    pub fn at(&self, src: &[u8], lx: &crate::tier1::Lexed, c: usize) -> At {
        let a = c * self.size.max(1);
        let (ret, depth) = self.rets(src, lx, c, a);
        // The frames it closes and the one it writes into (`chunk_frames`).
        let (ew, kw, evw) = (&self.eitems_w, &self.kids_w, &self.events_w);
        let at_w = shape_words(&self.shape_at[c]);
        let low = shape_words(&self.shapes[c])[4];
        let mut fw = vec![0u32; FRAME_W * (STEP_DEPTH + 2)];
        // SAFETY: room for a frame a level the walk holds, and one.
        let nf = unsafe { chunk_frames(ew, ew.len() / 3, &self.frame_of, kw, evw, &at_w, low, fw.as_mut_ptr()) };
        let frames = fw[..FRAME_W * nf].chunks_exact(FRAME_W).map(|f| Frame { node: f[0], kids: f[1], last: f[2], start: u64::from(f[3]) | (u64::from(f[4]) << 32) }).collect();
        let s = self.shape_at[c];
        At { entry: Entry { i: a, e: s.e.unwrap_or(0), depth, st: self.place[c] }, ret, nodes: s.nodes, end: None, frames, prev_start: s.last_start.unwrap_or(0) }
    }
}

/// `items` stably sorted by level: per 8-bit digit of the level, the chunks' keyed counts (`p2_run`) and their prefix
/// sum (`p2_merge`), the keys' starts (a prefix sum over the keys' totals), each chunk's items scattered from its sum
/// (`p2_write`): the block operators the device runs too.
fn sort_by_level(items: Vec<Item>, size: usize, order: Order<'_>) -> Vec<Item> {
    let mut xs: Vec<u32> = items.iter().flat_map(|x| [x.level, x.record, u32::from(x.head)]).collect();
    let n = items.len();
    for shift in [0u32, 8] {
        let ic = chunks_of(n, size);
        let counts: Vec<[u32; KEYS]> = each(ic.len(), order, |c| {
            let mut k = [0u32; KEYS];
            // SAFETY: `KEYS` words; the chunk's items.
            unsafe { p2_run(&xs, ic[c].0, ic[c].1, shift, k.as_mut_ptr()) };
            k
        });
        let add = |a: &[u32; KEYS], b: &[u32; KEYS]| {
            let mut s = [0u32; KEYS];
            // SAFETY: `KEYS` words each.
            unsafe { p2_merge(a, b, s.as_mut_ptr()) };
            s
        };
        let mut unit = [0u32; KEYS];
        // SAFETY: `KEYS` words.
        unsafe { p2_unit(unit.as_mut_ptr()) };
        let mut at = prefix(&counts, unit, order, add);
        let total = add(&at[ic.len() - 1], &counts[ic.len() - 1]);
        // The keys' starts: a prefix sum over the keys' totals.
        let add1 = |a: &[u32; 1], b: &[u32; 1]| {
            let mut s = [0u32; 1];
            // SAFETY: a word each.
            unsafe { add1_merge(a, b, s.as_mut_ptr()) };
            s
        };
        let keys: Vec<u32> = prefix(&total.map(|t| [t; 1]), [0], order, add1).iter().map(|k| k[0]).collect();
        let mut out = vec![0u32; 3 * n];
        let op = out.as_mut_ptr();
        let ap = at.as_mut_ptr();
        each(ic.len(), order, |c| {
            // SAFETY: each chunk its own sum, and its own items' places (its keys' ranges past its sum).
            unsafe { p2_write(&xs, ic[c].0, ic[c].1, shift, (*ap.add(c)).as_mut_ptr(), &keys, op) };
        });
        xs = out;
    }
    xs.chunks_exact(3).map(|w| Item { level: w[0], record: w[1], head: w[2] != 0 }).collect()
}

/// `src`'s tape into `w` by phases: P1-P5, then each chunk written from its state (P6), twice (each block's side-list
/// entries counted, then written at their ranks). Or the byte it is refused at.
pub fn parse_phases(src: &[u8], lx: &crate::tier1::Lexed, types: &[Vec<u32>], w: &mut Writer, size: usize, order: Order<'_>) -> Result<(), u32> {
    if lx.refused {
        return Err(0);
    }
    if lx.count == 0 {
        return Err(lx.len as u32);
    }
    if lx.count >= MOST_RECORDS {
        return Err(u32::MAX);
    }
    let toks = tokens(src, lx, types, order);
    let ph = Phases::run(src, lx, &toks, size, order);
    let rc = chunks_of(lx.count + 1, size);
    if let Some(c) = ph.refused {
        // The byte: chunk `c` from its place.
        let at = ph.at_place(src, lx, c);
        let mut d = Walk { toks: &toks, events: Vec::new(), nodes: 0 };
        let mut ret = [0u8; STEP_DEPTH];
        let base = at.entry.depth - at.ret.len();
        ret[base..at.entry.depth].copy_from_slice(&at.ret);
        // SAFETY: as `Phases::walk`'s.
        return match unsafe { run_step(src, lx, &mut d, at.entry, &mut ret, base, rc[c].1) } {
            Stop::Done(Err(b)) => Err(b),
            Stop::At(_) | Stop::Read(_) | Stop::Done(Ok(())) => Err(u32::MAX),
        };
    }
    let last = rc.len() - 1;
    let total = Shape::merge(&ph.shape_at[last], &ph.shapes[last]);
    w.reserve_run(1 + lx.count + lx.len, (DEPTH + 1) * 3 + 1);
    w.final_as_written();
    w.make_blocks(total.nodes as usize).map_err(|_| u32::MAX)?;
    let state = w.state_address();
    let ats: Vec<At> = each(rc.len(), order, |c| if (ph.place[c] as usize) < STEP_PLACES { ph.at(src, lx, c) } else { At::start() });
    // The chunks the walk reaches: those before the file's end.
    let live = |c: usize| (ph.place[c] as usize) < STEP_PLACES;
    for count in [true, false] {
        for c in order(rc.len()) {
            if !live(c) {
                continue;
            }
            // SAFETY: the state's room is reserved and its blocks made; `ats[c]` is the chunk's state.
            match unsafe { write_step(src, lx, state, &ats[c], rc[c].1, count) } {
                Stop::At(x) if x.i == rc[c].1 => {}
                Stop::Done(Ok(())) if c == last || !live(c + 1) => {}
                _ => return Err(u32::MAX),
            }
        }
    }
    w.end_middle(total.nodes as usize, total.last_start.unwrap_or(0));
    match w.failure() {
        None => Ok(()),
        Some(_) => Err(u32::MAX),
    }
}

impl Phases {
    /// Chunk `c`'s place and levels alone (what a refused chunk is walked from).
    fn at_place(&self, src: &[u8], lx: &crate::tier1::Lexed, c: usize) -> At {
        let a = c * self.size.max(1);
        let (ret, depth) = self.rets(src, lx, c, a);
        At { entry: Entry { i: a, e: 0, depth, st: self.place[c] }, ret, nodes: 0, end: None, frames: Vec::new(), prev_start: 0 }
    }
}
