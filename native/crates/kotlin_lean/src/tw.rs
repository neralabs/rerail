//! The tape as it is read (`Tw`), shared by the lean Kotlin, Swift and Scala parsers (each crate
//! mounts this file over its own `kinds` tables, which `gen_kinds.py` writes in one shape): each node
//! written in preorder as it is read (`rerail_tape_format::Writer`, decision 0226), with no tree built
//! first. As `rerail-sql-lean`'s writer:
//!
//! - A node is opened when its kind is known (`open`), or put before its first child when the kind
//!   is known only after it (`wrap`: an operator after its left operand). A frame for each node open,
//!   and for each mark where a node may still be put.
//! - Each token is decided by its parent's token automaton (`kinds::MOVES`), exactly as the engine's
//!   `TokenGrammar::stored_children` decides it: at once where the kind decides it (`kinds::FATE`),
//!   else held and decided when its parent closes (`claim`), by the walk over the children written.
//! - A comment goes into the node open when the next token or node is written, as tree-sitter's
//!   reductions leave it (a comment before a node's first child is its parent's).

use crate::kinds::{self, K};
use rerail_tape_format::{Widths, Writer};

/// In an extras entry's key: its depth under the extra before it (0: an extra of its own; 1: its
/// child, 2: a grandchild), as Scala's comment holds its `//` and a `using` directive.
pub const EXTRA_DEPTH: u32 = 30;

/// A node's key in the writer: its kind (10 bits), field (6) and the extra flag.
#[inline(always)]
pub const fn key(kind: K, field: u8, extra: bool) -> u32 {
    kind as u32 | (field as u32) << 10 | (extra as u32) << 16
}

/// The kind of a key's kind bits.
#[inline(always)]
pub fn kind_of(k: usize) -> K {
    assert!(k <= kinds::LAST as usize, "a kind");
    // SAFETY: `K` is `repr(u16)`, its values every number from 0 to `LAST`.
    unsafe { std::mem::transmute::<u16, K>(k as u16) }
}

/// The moves out of automaton state `s`.
#[inline]
fn moves(s: u32) -> &'static [(u32, u32)] {
    &kinds::MOVES[kinds::MOVES_AT[s as usize] as usize..kinds::MOVES_AT[s as usize + 1] as usize]
}

/// The state after `l` from `s`: most states have a few moves, read in order; a long list is
/// searched.
#[inline]
fn step(s: u32, l: u32) -> Option<u32> {
    let m = moves(s);
    if m.len() <= 8 {
        return m.iter().find(|&&(x, _)| x == l).map(|&(_, to)| to);
    }
    m.binary_search_by_key(&l, |&(x, _)| x).ok().map(|i| m[i].1)
}

#[inline]
fn accepts(s: u32) -> bool {
    kinds::ACCEPTS[s as usize / 64] >> (s % 64) & 1 == 1
}

/// The implied word from `s` to the named child `next` (`None`: to the end), where it holds a token
/// the tape can leave out: packed (`kinds::IMPLIED`), 0 for none.
#[inline]
fn implied(s: u32, next: Option<u32>) -> u32 {
    match next {
        Some(l) => {
            let rows = &kinds::IMPLIED[kinds::IMPLIED_AT[s as usize] as usize..kinds::IMPLIED_AT[s as usize + 1] as usize];
            if rows.len() <= 8 {
                return rows.iter().find(|&&(x, _)| x == l).map_or(0, |&(_, w)| w);
            }
            rows.binary_search_by_key(&l, |&(x, _)| x).map_or(0, |i| rows[i].1)
        }
        None => kinds::IMPLIED_END[s as usize],
    }
}

/// The tokens a node of automaton `a` leaves out, as the engine's `stored_children` decides them:
/// `kids` its children's labels in order (`kind | field << 10`, and whether the child is named),
/// `out[i]` set for each child left out. None where the walk is refused (the automaton does not take
/// the children).
fn walk(a: usize, kids: &[(u32, bool)], out: &mut [bool]) {
    // The gap `from..to` before the named child `next` (`None`: the end), from `state`.
    let close = |state: u32, from: usize, to: usize, next: Option<u32>, out: &mut [bool]| -> Option<u32> {
        let mut s = state;
        for k in &kids[from..to] {
            s = step(s, k.0)?;
        }
        let after = match next {
            Some(l) => step(s, l)?,
            None if accepts(s) => s,
            None => return None,
        };
        if to > from {
            // The word, packed: its start in `WORD`, length, and the tokens it leaves out.
            let w = implied(state, next);
            if w != 0 && (w >> 8 & 15) as usize == to - from {
                let word = &kinds::WORD[(w >> 12) as usize..(w >> 12) as usize + (to - from)];
                if word.iter().zip(&kids[from..to]).all(|(&l, k)| l == k.0) {
                    for i in from..to {
                        out[i] |= w >> (i - from) & 1 == 1;
                    }
                }
            }
        }
        Some(after)
    };
    let mut go = || -> Option<()> {
        let mut state = kinds::START[a];
        let mut from = 0;
        for (i, &(l, named)) in kids.iter().enumerate() {
            if named {
                state = close(state, from, i, Some(l), out)?;
                from = i + 1;
            }
        }
        close(state, from, kids.len(), None, out)?;
        Some(())
    };
    if go().is_none() {
        out.fill(false);
    }
}

/// Whether automaton `a` (its index) leaves token kind `t` out wherever it stands (`Some(true)`),
/// stores it (`Some(false)`), or its walk decides (`None`).
#[inline(always)]
fn fate(a: usize, t: K) -> Option<bool> {
    let fate = &kinds::FATE[a];
    let (word, bit) = (t as usize / 64, 1u64 << (t as usize % 64));
    if fate[kinds::FATE_WORDS + word] & bit != 0 { None } else { Some(fate[word] & bit != 0) }
}

/// A node being read (written, open), or a mark: a place a node may still be put over what is read
/// after it (`node`, `wrap`).
#[derive(Clone, Copy, Debug)]
struct Frame {
    /// Its node's place in the writer; a mark's, where a node put there would go.
    at: u32,
    /// Its held tokens: `held[held..]`, less those of the frames above it.
    held: u32,
    /// Its node's kind; `None` for a mark.
    kind: Option<K>,
    /// Whether it has a child (a node or a token; a comment is not one).
    any: bool,
}

/// A token whose fate waits for its parent (a mark's, a dynamic automaton's): its key, span, and its
/// place in the writer (before the node there now).
#[derive(Clone, Copy, Debug)]
struct Held {
    key: u32,
    s: u32,
    e: u32,
    at: u32,
}

/// An open node: its frame.
#[derive(Clone, Copy, Debug)]
pub struct Node(u32);

/// A mark: its frame.
#[derive(Clone, Copy, Debug)]
pub struct Mark(u32);

/// The writer's state, to go back to (`Tw::rewind`).
#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub struct Saved {
    nodes: usize,
    frames: usize,
    top: Frame,
    held: usize,
    flushed: usize,
    last_end: u32,
}

/// The tape being written: the nodes, the frames open, the held tokens and the comments.
pub struct Tw {
    pub w: Writer<Widths>,
    frames: Vec<Frame>,
    held: Vec<Held>,
    /// Comments, in order: (key, start, end); those before `flushed` are written.
    pub extras: Vec<(u32, u32, u32)>,
    pub flushed: usize,
    kids: Vec<(u32, bool)>,
    is_held: Vec<bool>,
    out: Vec<bool>,
    keep: Vec<bool>,
    /// The last token's end: an open node's end when it closes.
    pub last_end: u32,
}

#[allow(dead_code)]
impl Tw {
    pub fn new(widths: Widths, nodes: usize) -> Self {
        Tw {
            w: Writer::new(0, widths, nodes),
            frames: Vec::with_capacity(64),
            held: Vec::with_capacity(64),
            extras: Vec::with_capacity(16),
            flushed: 0,
            kids: Vec::with_capacity(64),
            is_held: Vec::with_capacity(64),
            out: Vec::with_capacity(64),
            keep: Vec::with_capacity(64),
            last_end: 0,
        }
    }

    pub fn save(&self) -> Saved {
        Saved {
            nodes: self.w.len(),
            frames: self.frames.len(),
            top: *self.frames.last().expect("the root's frame"),
            held: self.held.len(),
            flushed: self.flushed,
            last_end: self.last_end,
        }
    }

    /// Back to `s`: the nodes written after it, the frames above it, the comments written since
    /// (they are written again), the innermost frame's state.
    pub fn rewind(&mut self, s: Saved) {
        self.w.truncate(s.nodes);
        self.frames.truncate(s.frames);
        *self.frames.last_mut().expect("the root's frame") = s.top;
        self.held.truncate(s.held);
        self.flushed = s.flushed;
        self.last_end = s.last_end;
    }

    #[inline(always)]
    fn top(&mut self) -> &mut Frame {
        self.frames.last_mut().expect("the root's frame")
    }

    /// The comments before `before`, written into the node open (a comment before a node's first
    /// child is its parent's).
    #[inline(always)]
    pub fn flush(&mut self, before: u32) {
        while let Some(&(k, s, e)) = self.extras.get(self.flushed) {
            if s >= before {
                break;
            }
            self.flushed += 1;
            let at = self.w.push(k & !(3 << EXTRA_DEPTH), s);
            if self.extras.get(self.flushed).is_some_and(|x| x.0 >> EXTRA_DEPTH != 0) {
                self.flush_children(at, e);
            } else {
                self.w.close(at, e);
            }
        }
    }

    /// The children of the extra just pushed (`at`, ending at `e`): the entries after it at depth 1
    /// and on (`EXTRA_DEPTH`), each inside the one before it at a lesser depth.
    #[cold]
    fn flush_children(&mut self, at: usize, e: u32) {
        let mut open: Vec<(usize, u32, u32)> = vec![(at, e, 0)];
        while let Some(&(k, s, e)) = self.extras.get(self.flushed) {
            let d = k >> EXTRA_DEPTH;
            if d == 0 {
                break;
            }
            self.flushed += 1;
            while open.last().is_some_and(|o| o.2 >= d) {
                let (a, e, _) = open.pop().expect("a frame");
                self.w.close(a, e);
            }
            let a = self.w.push(k & !(3 << EXTRA_DEPTH), s);
            open.push((a, e, d));
        }
        while let Some((a, e, _)) = open.pop() {
            self.w.close(a, e);
        }
    }

    /// The root, open before anything is read.
    pub fn open_root(&mut self, kind: K) -> Node {
        let at = self.w.push(key(kind, 0, false), 0);
        self.frames.push(Frame { at: at as u32, held: 0, kind: Some(kind), any: false });
        Node(0)
    }

    /// The root closed: its span from `start` to `end`, the comments left its own.
    pub fn close_root(&mut self, n: Node, start: u32, end: u32) {
        self.end_marks(n.0);
        self.flush(u32::MAX);
        let f = self.frames.pop().expect("the root's frame");
        self.claim(0, f.kind.expect("a node"), f.held as usize);
        self.w.set_start(0, start);
        self.w.close(0, end);
    }

    /// A mark at `s` (the next token's start): a node may be put over what is read from here on
    /// (`node`, `wrap`). It needs no end: the node below it closing ends it.
    #[inline]
    pub fn mark(&mut self, s: u32) -> Mark {
        self.flush(s);
        self.frames.push(Frame { at: self.w.len() as u32, held: self.held.len() as u32, kind: None, any: false });
        Mark(self.frames.len() as u32 - 1)
    }

    /// A node of `kind` given `field`, open at `s` (the next token's start): what is read until its
    /// `close` is its.
    #[inline]
    pub fn open(&mut self, kind: K, field: u8, s: u32) -> Node {
        self.flush(s);
        self.top().any = true;
        let at = self.w.push(key(kind, field, false), s);
        self.frames.push(Frame { at: at as u32, held: self.held.len() as u32, kind: Some(kind), any: false });
        Node(self.frames.len() as u32 - 1)
    }

    /// The marks above frame `i` ended: their children are frame `i`'s.
    #[inline(always)]
    fn end_marks(&mut self, i: u32) {
        let mut any = false;
        while self.frames.len() > i as usize + 1 {
            let f = self.frames.pop().expect("a frame");
            // A node left open would have no size: a parser bug stops here.
            assert!(f.kind.is_none(), "a node left open");
            any |= f.any;
        }
        self.frames[i as usize].any |= any;
    }

    /// Mark `m` dropped (nothing put over it): what it holds is its parent's.
    #[inline]
    pub fn unmark(&mut self, m: Mark) {
        debug_assert!(self.frames.len() == m.0 as usize + 1, "a mark not the innermost frame");
        self.end_marks(m.0 - 1);
    }

    /// A node of `kind` given `field` put at mark `m`, over what was read since (an operator's left
    /// operand), left open: what is read until its `close` is its too. `m` stays, now holding it.
    pub fn wrap(&mut self, m: Mark, kind: K, field: u8) -> Node {
        self.wrap_from(m, kind, field, 0)
    }

    /// `wrap`, its start `s` where nothing was read since the mark.
    #[inline]
    pub fn wrap_from(&mut self, m: Mark, kind: K, field: u8, s: u32) -> Node {
        self.end_marks(m.0);
        let f = self.frames[m.0 as usize];
        let (at, held) = (f.at as usize, f.held as usize);
        // Its start: its first child's, a held token's where one comes first.
        let start = match self.held.get(held) {
            Some(h) if h.at as usize == at => h.s,
            _ if at < self.w.len() => self.w.start(at),
            _ => s,
        };
        self.w.insert(at, key(kind, field, false), start);
        for h in &mut self.held[held..] {
            h.at += 1;
        }
        self.frames[m.0 as usize] = Frame { any: true, ..f };
        self.frames.push(Frame { at: at as u32, held: held as u32, kind: Some(kind), any: f.any });
        Node(self.frames.len() as u32 - 1)
    }

    /// A node of `kind` over what was read since mark `m`, closed: its place. `m` stays (a node of
    /// another kind may be put over this one: `a + b + c`).
    #[inline]
    pub fn node(&mut self, kind: K, m: Mark) -> u32 {
        let n = self.wrap(m, kind, 0);
        self.close(n)
    }

    /// Whether a child (a node or a token) was read since mark `m`.
    pub fn read_since(&self, m: Mark) -> bool {
        self.frames[m.0 as usize..].iter().any(|f| f.any)
    }

    /// The first node read since mark `m` (its place).
    #[inline]
    pub fn mark_child(&self, m: Mark) -> u32 {
        self.frames[m.0 as usize].at
    }

    /// Closes node `n`: its held tokens decided and the stored ones written, its end the last
    /// token's. Its place.
    #[inline]
    pub fn close(&mut self, n: Node) -> u32 {
        self.close_span(n, None)
    }

    /// Closes node `n` at `e` (a node whose last bytes are no child's: hidden tokens).
    pub fn close_at(&mut self, n: Node, e: u32) -> u32 {
        self.close_span(n, Some(e))
    }

    fn close_span(&mut self, n: Node, end: Option<u32>) -> u32 {
        self.end_marks(n.0);
        let f = self.frames.pop().expect("its frame");
        let at = f.at as usize;
        self.claim(at, f.kind.expect("a node"), f.held as usize);
        match end {
            Some(e) => self.w.close(at, e),
            None => self.w.close(at, self.last_end.max(self.w.start(at))),
        }
        at as u32
    }

    /// The held tokens from `held[from]` on, children of the node at `at` of `kind`: each left out
    /// or written at its place, by the kind (`FATE`) or by its automaton's walk over the node's
    /// children.
    fn claim(&mut self, at: usize, kind: K, from: usize) {
        let n = self.held.len() - from;
        if n == 0 {
            return;
        }
        let a = kinds::AUTOMATON[kind as usize] as usize;
        self.keep.clear();
        if a == 0 {
            self.keep.resize(n, true);
        } else if kinds::DYNAMIC[a - 1] {
            // The children in order: the nodes written (a node's subtree skipped by its size, a
            // comment passed), each held token before the node at its place.
            self.kids.clear();
            self.is_held.clear();
            let (mut h, mut i) = (from, at + 1);
            loop {
                while h < self.held.len() && self.held[h].at as usize <= i {
                    self.kids.push((self.held[h].key & 0xffff, false));
                    self.is_held.push(true);
                    h += 1;
                }
                if i >= self.w.len() {
                    break;
                }
                let k = self.w.key(i);
                if k >> 16 & 1 == 0 {
                    self.kids.push((k & 0xffff, kinds::KINDS[(k & 0x3ff) as usize].1));
                    self.is_held.push(false);
                }
                i += self.w.size(i) as usize;
            }
            self.out.clear();
            self.out.resize(self.kids.len(), false);
            walk(a - 1, &self.kids, &mut self.out);
            for (j, &held) in self.is_held.iter().enumerate() {
                if held {
                    self.keep.push(!self.out[j]);
                }
            }
        } else {
            // A token the kind decides (a mark's, held before its node was known); a token the
            // automaton never takes is stored, as the walk's refusal stores it.
            for h in &self.held[from..] {
                let t = (h.key & 0x3ff) as usize;
                self.keep.push(fate(a - 1, kind_of(t)) != Some(true));
            }
        }
        let mut off = 0;
        for (h, &keep) in self.held[from..].iter().zip(&self.keep) {
            if keep {
                self.w.insert_leaf(h.at as usize + off, h.key, h.s, h.e);
                off += 1;
            }
        }
        self.held.truncate(from);
    }

    /// A named leaf (a name, a literal) at `s..e`, given `field`: its place.
    #[inline]
    pub fn leaf(&mut self, kind: K, field: u8, s: u32, e: u32) -> u32 {
        self.flush(s);
        self.top().any = true;
        let at = self.w.push(key(kind, field, false), s);
        self.w.close(at, e);
        self.last_end = e;
        at as u32
    }

    /// A token (punctuation, an anonymous kind) at `s..e`: written, left out, or held, by its
    /// parent's kind (`FATE`). Held in a mark (its node is not known yet) and in a node whose walk
    /// decides some token (its children all read first).
    #[inline]
    pub fn token(&mut self, kind: K, field: u8, s: u32, e: u32) {
        self.flush(s);
        self.last_end = e;
        let f = self.top();
        f.any = true;
        let k = key(kind, field, false);
        if let Some(p) = f.kind {
            let a = kinds::AUTOMATON[p as usize] as usize;
            let stored = if a == 0 {
                Some(true)
            } else if kinds::DYNAMIC[a - 1] {
                None
            } else {
                fate(a - 1, kind).map(|dropped| !dropped)
            };
            match stored {
                Some(true) => {
                    let at = self.w.push(k, s);
                    self.w.close(at, e);
                    return;
                }
                Some(false) => return,
                None => {}
            }
        }
        self.held.push(Held { key: k, s, e, at: self.w.len() as u32 });
    }

    /// Node `id` given `field`, where it has none: a field given inside a hidden rule wins over one
    /// given outside it, as tree-sitter's cursor looks for a node's field from its nearest hidden
    /// parent out.
    #[inline]
    pub fn set_field(&mut self, id: u32, field: u8) {
        let k = self.w.key(id as usize);
        if k >> 10 & 0x3f == 0 {
            self.w.set_key(id as usize, k | u32::from(field) << 10);
        }
    }

    /// The nodes read since mark `m` (the comments passed) given `field`, where they have none: a
    /// hidden rule's children all take the field given it (`a?` as a navigation's target).
    pub fn field_since(&mut self, m: Mark, field: u8) {
        let f = &self.frames[m.0 as usize];
        let (mut i, held) = (f.at as usize, f.held as usize);
        while i < self.w.len() {
            if self.w.key(i) >> 16 & 1 == 0 {
                self.set_field(i as u32, field);
            }
            i += self.w.size(i) as usize;
        }
        // The tokens held since the mark (their node not yet known) take it too.
        for h in &mut self.held[held..] {
            if h.key >> 10 & 0x3f == 0 {
                h.key |= u32::from(field) << 10;
            }
        }
    }

    /// Node `id`'s kind.
    #[inline]
    pub fn kind_at(&self, id: u32) -> K {
        kind_of((self.w.key(id as usize) & 0x3ff) as usize)
    }

    /// Node `id` given `field` in place of the one it has (none for 0).
    pub fn refield(&mut self, id: u32, field: u8) {
        let k = self.w.key(id as usize) & !(0x3f << 10);
        self.w.set_key(id as usize, k | u32::from(field) << 10);
    }

    /// Node `id` put in supertype set `class` (bits 17 on: Scala's `{expression}`, where the grammar
    /// reaches the node through its supertype).
    pub fn set_class(&mut self, id: u32, class: u32) {
        let k = self.w.key(id as usize);
        self.w.set_key(id as usize, k | class << 17);
    }

    /// Node `id` given `kind`.
    pub fn retype(&mut self, id: u32, kind: K) {
        let k = self.w.key(id as usize);
        self.w.set_key(id as usize, k & !0x3ff | kind as u32);
    }

    /// Closed node `id`'s end.
    pub fn end_at(&self, id: u32) -> u32 {
        self.w.end(id as usize)
    }

    /// Node `id`'s start.
    pub fn start_at(&self, id: u32) -> u32 {
        self.w.start(id as usize)
    }

    /// Node `id`'s size (itself and its subtree).
    pub fn size_at(&self, id: u32) -> u32 {
        self.w.size(id as usize)
    }
}
