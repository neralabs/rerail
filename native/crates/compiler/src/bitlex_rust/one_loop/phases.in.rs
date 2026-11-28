
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
    /// Each record's depth before it (P1's), and its walk depth (an empty container's closer in its parent's level).
    pub depth: Vec<i64>,
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
        // P1: each chunk's depth summary.
        let p1: Vec<Depth> = each(rc.len(), order, |c| {
            let mut s = Depth::default();
            for r in rc[c].0..rc[c].1 {
                let one = match bracket_of(src, lx, r) {
                    Some((_, true)) => Depth { d: 1, m: 0, items: 2 },
                    Some((_, false)) => Depth { d: -1, m: -1, items: 1 },
                    None => Depth { d: 0, m: 0, items: 1 },
                };
                s = Depth::merge(&s, &one);
            }
            s
        });
        let p1_at = prefix(&p1, Depth::default(), order, Depth::merge);
        // P1's writes: each record's depth and its items, at the chunk's sum.
        let total = Depth::merge(&p1_at[rc.len() - 1], &p1[rc.len() - 1]);
        let mut depth = vec![0i64; n + 1];
        let mut items = vec![Item::default(); total.items as usize];
        for c in order(rc.len()) {
            let (mut d, mut k) = (p1_at[c].d, p1_at[c].items as usize);
            for r in rc[c].0..rc[c].1 {
                depth[r] = d;
                match bracket_of(src, lx, r) {
                    Some((_, true)) => {
                        items[k] = Item { level: d as u32, record: r as u32, head: false };
                        items[k + 1] = Item { level: (d + 1) as u32, record: r as u32, head: true };
                        k += 2;
                        d += 1;
                    }
                    Some((_, false)) => {
                        // An empty container's closer is in its parent's level, as its opener's first item.
                        let level = if empty_close(src, lx, r) { d - 1 } else { d };
                        items[k] = Item { level: level.max(0) as u32, record: r as u32, head: false };
                        k += 1;
                        d -= 1;
                    }
                    None => {
                        items[k] = Item { level: d.max(0) as u32, record: r as u32, head: false };
                        k += 1;
                    }
                }
            }
        }
        // P2: the level sort, two 8-bit digits, each a phase of keyed counts over chunks of items. Within a level the
        // items keep their order: by record, an opener's head after its item in its parent's level.
        let items = sort_by_level(items, size, order);
        let ic = chunks_of(items.len(), size);
        let mut head_at = vec![NO; n + 1];
        for c in order(ic.len()) {
            for x in ic[c].0..ic[c].1 {
                if items[x].head {
                    head_at[items[x].record as usize] = x as u32;
                }
            }
        }
        // P3a: each item's container, a segmented last-value of the heads.
        let one = |x: usize| Last { reset: level_start(&items, x), v: if items[x].head { items[x].record } else { NO } };
        let p3a: Vec<Last> = each(ic.len(), order, |c| (ic[c].0..ic[c].1).fold(Last::UNIT, |s, x| Last::merge(&s, &one(x))));
        let p3a_at = prefix(&p3a, Last::UNIT, order, Last::merge);
        let mut container = vec![NO; n + 1];
        for c in order(ic.len()) {
            let mut s = p3a_at[c];
            for x in ic[c].0..ic[c].1 {
                s = Last::merge(&s, &one(x));
                if !items[x].head {
                    container[items[x].record as usize] = s.v;
                }
            }
        }
        let mut ph = Phases { n, size, depth, p1, items, head_at, container, place: Vec::new(), refused: None, shapes: Vec::new(), shape_at: Vec::new(), events: Vec::new(), eitems: Vec::new(), frame_of: Vec::new(), kids: Vec::new() };
        // P4: each chunk's place map, its pops below its start reading its containers' return places.
        let maps: Vec<Places> = each(rc.len(), order, |c| {
            let mut m = [STEP_REFUSED; 32];
            for (p, o) in m.iter_mut().enumerate().take(STEP_PLACES) {
                *o = ph.walk(src, lx, toks, rc[c], p as u8, None);
            }
            m
        });
        let at = prefix(&maps, places_unit(), order, places_merge);
        ph.place = each(rc.len(), order, |c| at[c][0]);
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
            let mut w = Vec::new();
            ph.walk(src, lx, toks, rc[c], ph.place[c], Some(&mut w));
            let mut s = Shape::UNIT;
            let mut evs = Vec::new();
            for (_, es, e) in w {
                for ev in es {
                    // Its frame depth and first node within the chunk.
                    evs.push(Event { f: s.net as u32, node: s.nodes, ..ev });
                    s = Shape::merge(&s, &Shape::of(&ev));
                }
                if e.is_some() {
                    s.e = e;
                }
            }
            (s, evs)
        };
        let shapes: Vec<Shape> = each(rc.len(), order, |c| shape_of(c).0);
        let shape_at = prefix(&shapes, Shape::UNIT, order, Shape::merge);
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
        // P8a: each item's frame (the opening event of the level it is in), a segmented last-value of the heads.
        let one = |x: usize| Last { reset: level_start(&eitems, x), v: if eitems[x].head { eitems[x].record } else { NO } };
        let p8a: Vec<Last> = each(ec.len(), order, |c| (ec[c].0..ec[c].1).fold(Last::UNIT, |s, x| Last::merge(&s, &one(x))));
        let p8a_at = prefix(&p8a, Last::UNIT, order, Last::merge);
        let mut frame_of = vec![NO; eitems.len()];
        for c in order(ec.len()) {
            let mut s = p8a_at[c];
            for (x, f) in frame_of.iter_mut().enumerate().take(ec[c].1).skip(ec[c].0) {
                s = Last::merge(&s, &one(x));
                *f = s.v;
            }
        }
        // P8b: each item's frame's children so far and the last one's node.
        let ev = &ph.events;
        let kid = |x: usize| {
            let it = eitems[x];
            let child = !it.head;
            Kids { reset: level_start(&eitems, x) || it.head, count: u32::from(child), last: if child { ev[it.record as usize].node } else { NO } }
        };
        let p8b: Vec<Kids> = each(ec.len(), order, |c| (ec[c].0..ec[c].1).fold(Kids::UNIT, |s, x| Kids::merge(&s, &kid(x))));
        let p8b_at = prefix(&p8b, Kids::UNIT, order, Kids::merge);
        let mut kids = vec![Kids::UNIT; eitems.len()];
        for c in order(ec.len()) {
            let mut s = p8b_at[c];
            for (x, k) in kids.iter_mut().enumerate().take(ec[c].1).skip(ec[c].0) {
                s = Kids::merge(&s, &kid(x));
                *k = s;
            }
        }
        ph.eitems = eitems;
        ph.frame_of = frame_of;
        ph.kids = kids;
        ph
    }

    /// Record `r`'s walk depth: its depth less an empty container's closer.
    fn walk_depth(&self, src: &[u8], lx: &crate::tier1::Lexed, r: usize) -> usize {
        (self.depth[r] - i64::from(r < lx.count && empty_close(src, lx, r))).max(0) as usize
    }

    /// The return places of the levels chunk `(a, b)` closes below its start, innermost last (as `At::ret`), and its
    /// walk depth: each from its containers' kinds (P3a), the root's `STEP_CONT` last.
    fn rets(&self, src: &[u8], lx: &crate::tier1::Lexed, c: usize, a: usize) -> (Vec<u8>, usize) {
        let depth = self.walk_depth(src, lx, a);
        let closes = (-self.p1[c].m - i64::from(a < lx.count && empty_close(src, lx, a))).max(0) as usize;
        let len = closes.min(depth);
        // The open levels' openers, innermost first: the record's container, its container's, ...
        let mut chain = Vec::with_capacity(len + 1);
        let mut o = self.container[a];
        while chain.len() <= len && o != NO {
            chain.push(o);
            o = self.container[o as usize];
        }
        let kind = |i: usize| chain.get(i).and_then(|&o| bracket_of(src, lx, o as usize)).map_or(ROOT, |k| k.0) as usize;
        ((0..len).map(|t| STEP_CONT[kind(len - t)]).collect(), depth)
    }

    /// Chunk `(a, b)` walked from place `p` with its levels known: its place past it (or `STEP_REFUSED`,
    /// `STEP_ENDS`). With `events`, a record at a time, each record's events and the last token's end it leaves.
    fn walk(&self, src: &[u8], lx: &crate::tier1::Lexed, toks: &[Token], (a, b): (usize, usize), p: u8, mut events: Option<&mut Vec<(usize, Vec<Event>, Option<usize>)>>) -> u8 {
        let c = a / self.size.max(1);
        let (known, depth) = self.rets(src, lx, c, a);
        // Deeper than the walk's levels: refused (the serial walk refuses at the bracket that passes them).
        if depth > STEP_DEPTH {
            return STEP_REFUSED;
        }
        let mut ret = [0u8; STEP_DEPTH];
        let base = depth - known.len();
        ret[base..depth].copy_from_slice(&known);
        let mut en = Entry { i: a, e: usize::MAX, depth, st: p };
        let mut d = Walk { toks, events: Vec::new(), nodes: 0 };
        let by_record = events.is_some();
        loop {
            let stop = if by_record { en.i + 1 } else { b };
            d.events.clear();
            // SAFETY: `ret` holds the levels below `base` it reads; `d` writes nothing.
            let s = unsafe { run_step(src, lx, &mut d, en, &mut ret, base, stop) };
            let (next, out) = match s {
                Stop::At(x) if x.i >= stop => (Some(x), x.st),
                Stop::Done(Ok(())) => (None, STEP_ENDS),
                _ => (None, STEP_REFUSED),
            };
            if let Some(ev) = events.as_deref_mut() {
                ev.push((en.i, d.events.clone(), next.and_then(|x| (x.e != usize::MAX).then_some(x.e))));
            }
            match next {
                Some(x) if x.i < b => en = Entry { e: usize::MAX, ..x },
                _ => return out,
            }
        }
    }

    /// Chunk `c`'s state at its start: its place, levels, first node, frames and the node before (P1-P5's sums).
    pub fn at(&self, src: &[u8], lx: &crate::tier1::Lexed, c: usize) -> At {
        let a = c * self.size.max(1);
        let (ret, depth) = self.rets(src, lx, c, a);
        // The frames it closes and the one it writes into: frame k's level k + 1 in the frame order, its last item
        // before the chunk's first event: that item's frame and its children so far.
        let s = self.shape_at[c];
        let fd = s.net as usize;
        let need = ((1 - self.shapes[c].low) as usize).min(fd);
        let frames = (fd - need..fd)
            .map(|k| {
                let level = (k + 1) as u32;
                let p = self.eitems.partition_point(|x| (x.level, x.record) < (level, s.events));
                let (it, o) = (self.eitems[p - 1], self.frame_of[p - 1]);
                let kd = self.kids[p - 1];
                let (kids, last) = if it.head { (0, NONE) } else { (kd.count, kd.last) };
                let open = self.events[o as usize];
                Frame { node: open.node, kids, last, start: open.start }
            })
            .collect();
        let s = self.shape_at[c];
        At { entry: Entry { i: a, e: s.e.unwrap_or(0), depth, st: self.place[c] }, ret, nodes: s.nodes, end: None, frames, prev_start: s.last_start.unwrap_or(0) }
    }
}

/// Whether item `x` starts its level in the level order: a read of its neighbour.
fn level_start(items: &[Item], x: usize) -> bool {
    x == 0 || items[x - 1].level != items[x].level
}

/// `items` stably sorted by level: two phases of keyed counts over chunks of `size` items, an 8-bit digit each.
fn sort_by_level(items: Vec<Item>, size: usize, order: Order<'_>) -> Vec<Item> {
    let mut xs = items;
    for shift in [0u32, 8] {
        let ic = chunks_of(xs.len(), size);
        let digit = |x: &Item| ((x.level >> shift) & 0xff) as usize;
        let counts: Vec<[u32; 256]> = each(ic.len(), order, |c| {
            let mut k = [0u32; 256];
            for x in &xs[ic[c].0..ic[c].1] {
                k[digit(x)] += 1;
            }
            k
        });
        let add = |a: &[u32; 256], b: &[u32; 256]| {
            let mut s = *a;
            for (s, b) in s.iter_mut().zip(b) {
                *s += b;
            }
            s
        };
        let at = prefix(&counts, [0; 256], order, add);
        let total = add(&at[ic.len() - 1], &counts[ic.len() - 1]);
        // The digits' starts: a prefix sum over the 256 keys.
        let key_at = prefix(&total.map(|t| [t; 1]), [0], order, |a, b| [a[0] + b[0]]);
        let mut out = vec![Item::default(); xs.len()];
        for c in order(ic.len()) {
            let mut k = at[c];
            for x in &xs[ic[c].0..ic[c].1] {
                let g = digit(x);
                out[(key_at[g][0] + k[g]) as usize] = *x;
                k[g] += 1;
            }
        }
        xs = out;
    }
    xs
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
