//! A test's plans as one dense automaton table, for the device's text tests (`metal_join`'s `row_text`, its
//! `ROW_VERDICT` and `ROW_TEST` ops).
//!
//! The device answers a text test once per symbol not yet in the verdict cache (decision 0478 step 2d), and once
//! per row for a made text, a SIMD group per text, by composing each byte's
//! transition function over a chunk of 32 bytes at a time (decision 0038's chunked matching). So it needs the plans
//! as one deterministic automaton with few states (each a byte, at most 255), whose final state alone says whether
//! any plan matched anywhere: a match state is made absorbing (every byte, and the end of the text, keep it), and
//! the end-of-text transition is applied last.
//!
//! The table is made from the same lowered plans the CPU's `Compiled` runs (the text layer's one canonical plan per
//! test), so both answer the same question. A plan the table cannot hold (a backtracking pattern, a Unicode word
//! boundary, more than 255 states) is refused, with why: the test then runs on the CPU.

use super::{Lowered, Plan};
use regex_automata::dfa::{Automaton, StartKind, dense};
use regex_automata::nfa::thompson;
use regex_automata::util::start;
use regex_automata::{Anchored, MatchKind};
use regex_syntax::hir::Hir;
use std::collections::HashMap;
use std::sync::Arc;

/// The most states a table holds (a state is a byte in the device's scratch).
pub const MAX_STATES: usize = 255;

/// A dense automaton: its bytes (`bytes()`, in the automata block, decision 0478 step 2c) are the byte classes
/// (256), then `next` (`states` rows of `classes`), then the end-of-text transition per state, then whether each state
/// accepts (all bytes).
#[derive(Clone, Copy, Debug)]
pub struct Table {
    pub states: u32,
    pub classes: u32,
    pub start: u32,
    /// Where the automata block holds it.
    pub automaton: super::automata::Automaton,
}

impl Table {
    /// Its bytes, where the automata block holds them.
    pub fn bytes(&self) -> super::automata::Bytes {
        self.automaton.bytes()
    }

    /// Whether `text` matches: the CPU's walk of the same table (tests only; the device walks it in chunks).
    #[cfg(test)]
    pub fn is_match(&self, text: &[u8]) -> bool {
        let (s, c) = (self.states as usize, self.classes as usize);
        let bytes = self.bytes();
        let next = &bytes[256..256 + s * c];
        let eoi = &bytes[256 + s * c..256 + s * c + s];
        let accept = &bytes[256 + s * c + s..];
        let mut state = self.start as usize;
        for &b in text {
            state = next[state * c + bytes[b as usize] as usize] as usize;
        }
        accept[eoi[state] as usize] != 0
    }
}

/// A plan as a regex of the whole test (anchors included); `None` for a backtracking one.
fn hir(plan: &Plan) -> Option<Hir> {
    Some(match plan {
        Plan::Always => Hir::empty(),
        Plan::Never => Hir::fail(),
        Plan::Literals { anchor, literals } => {
            let core = Hir::alternation(literals.iter().map(|l| Hir::literal(l.clone())).collect());
            Plan::automaton_hir(*anchor, &core)
        }
        Plan::Automaton { anchor, core } => Plan::automaton_hir(*anchor, core),
        Plan::Backtracking { .. } => return None,
    })
}

/// The plans' key: the text layer's canonical keys, in order.
fn plans_key(plans: &[Arc<Lowered>]) -> String {
    plans.iter().map(|p| p.key.0.as_str()).collect::<Vec<_>>().join(",")
}

/// The id of the test of `plans` (any of them matching), for its verdicts in the rolling cache (decision 0478 step
/// 2d): the same for the CPU's matcher of those plans and the device's table of them. Nothing is built for it.
pub fn test_id(plans: &[Arc<Lowered>]) -> Option<u32> {
    super::automata::id(&format!("test:{}", plans_key(plans)))
}

/// The table of `plans` (any of them matching), made once per set of plans into the automata block.
pub fn table(plans: &[Arc<Lowered>]) -> Result<Arc<Table>, String> {
    let key = format!("table:{}", plans_key(plans));
    let a = super::automata::get_or_make(&key, || build(plans).map(|(bytes, [states, classes, start])| (bytes, [states, classes, start, 0])))?;
    Ok(Arc::new(Table { states: a.words[0], classes: a.words[1], start: a.words[2], automaton: a }))
}

/// The table's bytes and its states, classes and start.
fn build(plans: &[Arc<Lowered>]) -> Result<(Vec<u8>, [u32; 3]), String> {
    let hirs = plans.iter().map(|p| hir(&p.plan).ok_or_else(|| "a backtracking pattern".to_owned())).collect::<Result<Vec<_>, _>>()?;
    let hirs = if hirs.is_empty() { vec![Hir::fail()] } else { hirs };
    let nfa = thompson::Compiler::new()
        .configure(thompson::Config::new().utf8(false))
        .build_many_from_hir(&hirs)
        .map_err(|e| format!("its automaton: {e}"))?;
    let dfa = dense::Builder::new()
        .configure(dense::Config::new().match_kind(MatchKind::All).start_kind(StartKind::Unanchored))
        .build_from_nfa(&nfa)
        .map_err(|e| format!("its automaton: {e}"))?;
    let first = dfa.start_state(&start::Config::new().anchored(Anchored::No)).map_err(|e| format!("its start: {e}"))?;
    // Every state reachable from the start, numbered in the order found.
    let mut ids = vec![first];
    let mut index: HashMap<regex_automata::util::primitives::StateID, usize> = HashMap::from([(first, 0)]);
    let mut next: Vec<[usize; 256]> = Vec::new();
    let mut eoi: Vec<usize> = Vec::new();
    let mut at = 0;
    while at < ids.len() {
        let id = ids[at];
        if dfa.is_quit_state(id) {
            return Err("its automaton quits on some bytes".into());
        }
        let mut row = [0usize; 256];
        let mut see = |to| {
            let n = index.len();
            *index.entry(to).or_insert_with(|| {
                ids.push(to);
                n
            })
        };
        for b in 0..=255u8 {
            row[b as usize] = see(dfa.next_state(id, b));
        }
        let e = see(dfa.next_eoi_state(id));
        next.push(row);
        eoi.push(e);
        if ids.len() > MAX_STATES {
            return Err(format!("its automaton has more than {MAX_STATES} states"));
        }
        at += 1;
    }
    let n = ids.len();
    let accept: Vec<bool> = ids.iter().map(|&id| dfa.is_match_state(id)).collect();
    // A match seen stays seen.
    for s in 0..n {
        if accept[s] {
            next[s] = [s; 256];
            eoi[s] = s;
        }
    }
    // Bytes with the same column of transitions are one class.
    let mut class_of: HashMap<Vec<usize>, u8> = HashMap::new();
    let mut classes = [0u8; 256];
    let mut columns: Vec<Vec<usize>> = Vec::new();
    for b in 0..256 {
        let column: Vec<usize> = (0..n).map(|s| next[s][b]).collect();
        let k = class_of.len();
        let c = *class_of.entry(column.clone()).or_insert_with(|| {
            columns.push(column);
            k as u8
        });
        classes[b] = c;
    }
    let nc = columns.len();
    let mut bytes = classes.to_vec();
    for s in 0..n {
        bytes.extend(columns.iter().map(|col| col[s] as u8));
    }
    bytes.extend(eoi.iter().map(|&e| e as u8));
    bytes.extend(accept.iter().map(|&a| u8::from(a)));
    Ok((bytes, [n as u32, nc as u32, 0]))
}

/// A regex's automata for `replace_all` on the device (`metal_join`'s `text_replace`, decision 0038's chunked
/// matching), as `regex_automata`'s DFA regex finds matches: a forward unanchored leftmost-first automaton whose last
/// match before it dies is a match's end, and a reverse anchored automaton (every match) run back from that end whose
/// furthest match is its start. Their states are bytes, at most `MAX_STATES`. `bytes`: a header of sixteen words
/// (forward states, classes, start at the text's start, start elsewhere, dead; reverse states, classes, start at the
/// text's end, start elsewhere, dead; the two tables' offsets), then each table as `Table`'s: byte classes, the
/// transitions, the end-of-text transition per state, whether each state is a match (a byte late: a match state after
/// byte k is a match ending, or in reverse starting, at k).
#[derive(Clone, Copy, Debug)]
pub struct Replacer {
    /// Where the automata block holds its bytes (decision 0478 step 2c).
    pub automaton: super::automata::Automaton,
    /// The larger table's states: a SIMD group's scratch holds two rows of them per lane.
    pub states: u32,
}

/// The replacement automata of regex `pattern`, made once per pattern. Refused (the replacement runs on the CPU): a
/// pattern that may match the empty text (Rust's rules for empty matches are not the device's), a look other than the
/// text's start and end, an automaton past `MAX_STATES`.
pub fn replacer(pattern: &str) -> Result<Arc<Replacer>, String> {
    let a = super::automata::get_or_make(&format!("replace:{pattern}"), || build_replacer(pattern).map(|(bytes, states)| (bytes, [states, 0, 0, 0])))?;
    Ok(Arc::new(Replacer { automaton: a, states: a.words[0] }))
}

/// One direction's automaton as a dense table: its starts (at the text's edge, elsewhere), dead state and bytes.
fn dense_table(dfa: &dense::DFA<Vec<u32>>, anchored: Anchored) -> Result<(u32, u32, u32, u32, u32, Vec<u8>), String> {
    let at = |behind: Option<u8>| dfa.start_state(&start::Config::new().anchored(anchored).look_behind(behind)).map_err(|e| format!("its start: {e}"));
    let edge = at(None)?;
    let elsewhere = at(Some(b'a'))?;
    for b in 0..=255u8 {
        if at(Some(b))? != elsewhere {
            return Err("a regex whose start depends on the byte beside it".into());
        }
    }
    let mut ids = vec![edge];
    let mut index: HashMap<regex_automata::util::primitives::StateID, usize> = HashMap::from([(edge, 0)]);
    let mut see = |to, ids: &mut Vec<_>| {
        let n = index.len();
        *index.entry(to).or_insert_with(|| {
            ids.push(to);
            n
        })
    };
    let mid = see(elsewhere, &mut ids);
    let (mut next, mut eoi): (Vec<[usize; 256]>, Vec<usize>) = (Vec::new(), Vec::new());
    let mut at = 0;
    while at < ids.len() {
        let id = ids[at];
        if dfa.is_quit_state(id) {
            return Err("its automaton quits on some bytes".into());
        }
        let mut row = [0usize; 256];
        for b in 0..=255u8 {
            row[b as usize] = see(dfa.next_state(id, b), &mut ids);
        }
        eoi.push(see(dfa.next_eoi_state(id), &mut ids));
        next.push(row);
        if ids.len() > MAX_STATES {
            return Err(format!("its automaton has more than {MAX_STATES} states"));
        }
        at += 1;
    }
    let n = ids.len();
    // A state for the dead one even when none is reached (it is never entered then).
    let dead = ids.iter().position(|&id| dfa.is_dead_state(id)).unwrap_or(n.min(MAX_STATES - 1));
    let mut class_of: HashMap<Vec<usize>, u8> = HashMap::new();
    let mut classes = [0u8; 256];
    let mut columns: Vec<Vec<usize>> = Vec::new();
    for b in 0..256 {
        let column: Vec<usize> = (0..n).map(|s| next[s][b]).collect();
        let k = class_of.len();
        classes[b] = *class_of.entry(column.clone()).or_insert_with(|| {
            columns.push(column);
            k as u8
        });
    }
    let mut bytes = classes.to_vec();
    for s in 0..n {
        bytes.extend(columns.iter().map(|col| col[s] as u8));
    }
    bytes.extend(eoi.iter().map(|&e| e as u8));
    bytes.extend(ids.iter().map(|&id| u8::from(dfa.is_match_state(id))));
    Ok((n as u32, columns.len() as u32, 0, mid as u32, dead as u32, bytes))
}

/// The replacement automata's bytes, and the larger table's states.
fn build_replacer(pattern: &str) -> Result<(Vec<u8>, u32), String> {
    use regex_syntax::hir::{Look, LookSet};
    let hir = regex_syntax::ParserBuilder::new().build().parse(pattern).map_err(|e| format!("its regex: {e}"))?;
    let props = hir.properties();
    if props.minimum_len().is_none_or(|m| m == 0) {
        return Err("a regex that may match the empty text".into());
    }
    let ends = LookSet::empty().insert(Look::Start).insert(Look::End);
    if !props.look_set().subtract(ends).is_empty() {
        return Err("a regex with a look other than the text's start and end".into());
    }
    let forward = thompson::Compiler::new().build_from_hir(&hir).map_err(|e| format!("its automaton: {e}"))?;
    let fdfa = dense::Builder::new()
        .configure(dense::Config::new().match_kind(MatchKind::LeftmostFirst).start_kind(StartKind::Unanchored))
        .build_from_nfa(&forward)
        .map_err(|e| format!("its automaton: {e}"))?;
    let reverse = thompson::Compiler::new()
        .configure(thompson::Config::new().reverse(true).which_captures(thompson::WhichCaptures::None))
        .build_from_hir(&hir)
        .map_err(|e| format!("its reverse automaton: {e}"))?;
    let rdfa = dense::Builder::new()
        .configure(dense::Config::new().match_kind(MatchKind::All).start_kind(StartKind::Anchored))
        .build_from_nfa(&reverse)
        .map_err(|e| format!("its reverse automaton: {e}"))?;
    let (fs, fc, f0, f1, fd, ft) = dense_table(&fdfa, Anchored::No)?;
    let (rs, rc, r0, r1, rd, rt) = dense_table(&rdfa, Anchored::Yes)?;
    let foff = 64;
    let roff = foff + ft.len();
    let mut bytes = Vec::with_capacity(roff + rt.len());
    for w in [fs, fc, f0, f1, fd, rs, rc, r0, r1, rd, foff as u32, roff as u32, 0, 0, 0, 0] {
        bytes.extend(w.to_le_bytes());
    }
    bytes.extend(ft);
    bytes.extend(rt);
    Ok((bytes, fs.max(rs)))
}

impl Replacer {
    /// `text` with every match replaced by `with` (a literal), walking the automata as the device does, a byte at a
    /// time (tests only; the device composes 32 bytes' transitions at once).
    #[cfg(test)]
    pub fn replace(&self, text: &str, with: &str) -> String {
        let bytes = self.automaton.bytes();
        let w = |i: usize| u32::from_le_bytes(bytes[4 * i..4 * i + 4].try_into().unwrap()) as usize;
        struct Dir<'a> { t: &'a [u8], s: usize, c: usize, edge: usize, mid: usize, dead: usize }
        impl Dir<'_> {
            fn next(&self, q: usize, b: u8) -> usize { self.t[256 + q * self.c + self.t[b as usize] as usize] as usize }
            fn eoi(&self, q: usize) -> usize { self.t[256 + self.s * self.c + q] as usize }
            fn accept(&self, q: usize) -> bool { self.t[256 + self.s * self.c + self.s + q] != 0 }
        }
        let f = Dir { t: &bytes[w(10)..w(11)], s: w(0), c: w(1), edge: w(2), mid: w(3), dead: w(4) };
        let r = Dir { t: &bytes[w(11)..], s: w(5), c: w(6), edge: w(7), mid: w(8), dead: w(9) };
        let t = text.as_bytes();
        let n = t.len();
        let (mut out, mut pos) = (Vec::new(), 0);
        while pos < n {
            let (mut st, mut last, mut dead) = (if pos == 0 { f.edge } else { f.mid }, None, false);
            for (k, &b) in t.iter().enumerate().skip(pos) {
                st = f.next(st, b);
                if st == f.dead {
                    dead = true;
                    break;
                }
                if f.accept(st) {
                    last = Some(k);
                }
            }
            if !dead && f.accept(f.eoi(st)) {
                last = Some(n);
            }
            let Some(e) = last else { break };
            let lo = pos.saturating_sub(1);
            let (mut st, mut first, mut dead) = (if e == n { r.edge } else { r.mid }, None, false);
            for k in (lo..e).rev() {
                st = r.next(st, t[k]);
                if st == r.dead {
                    dead = true;
                    break;
                }
                if r.accept(st) {
                    first = Some(k + 1);
                }
            }
            if !dead && pos == 0 && r.accept(r.eoi(st)) {
                first = Some(0);
            }
            let Some(s) = first.filter(|&s| s >= pos) else { break };
            out.extend_from_slice(&t[pos..s]);
            out.extend_from_slice(with.as_bytes());
            pos = e;
        }
        out.extend_from_slice(&t[pos..]);
        String::from_utf8(out).unwrap()
    }
}

/// The most states a capture group's program holds (its scratch is a group's, about 32 bytes a state).
pub const GROUP_MAX_STATES: usize = 2048;

/// A regex's capture group on the device (`rx_group`, decision 0688): the regex's Thompson NFA, the very one
/// `regex_automata` compiles for Rust's `regex` (whose captures the CPU reads), as a program the device's Pike VM
/// walks (`rx_group_range` in `join_stages.uglykl`), as `regex_automata`'s `PikeVM` walks it: leftmost-first, its
/// threads in priority order. `bytes`: a header of eight words (states, the group's start slot, the start state, the
/// stack's entries at most, the states' word offset, 0, 0, 0), then four words a state (`GROUP_*` kind, then its
/// words), then the sparse states' ranges (start, end, next) and the unions' alternates.
#[derive(Clone, Copy, Debug)]
pub struct GroupProgram {
    /// Where the automata block holds its bytes (decision 0478 step 2c).
    pub automaton: super::automata::Automaton,
    /// Its scratch, in units of a SIMD group's two rows of bytes a lane (64 bytes, a group of 32 lanes or more).
    pub scratch: u32,
}

/// A program's states' kinds.
pub const GROUP_FAIL: u32 = 0;
/// A byte in [a, b], then state c.
pub const GROUP_RANGE: u32 = 1;
/// a ranges at word b, each (start, end, next).
pub const GROUP_SPARSE: u32 = 2;
/// a alternates at word b, in priority order.
pub const GROUP_UNION: u32 = 3;
/// The group's start (a = 0) or end (a = 1) slot, or another's (a = 2), then state b.
pub const GROUP_CAPTURE: u32 = 4;
/// The text's start (a = 0) or end (a = 1), then state b.
pub const GROUP_LOOK: u32 = 5;
pub const GROUP_MATCH: u32 = 6;

/// The program of group `name` of regex `pattern`, made once per pattern and group. Refused (the group is the CPU's):
/// a regex not anchored at the text's start (the device runs from the start alone), a look other than the text's
/// start and end, a group it does not name, a program past `GROUP_MAX_STATES`.
pub fn group_program(pattern: &str, name: &str) -> Result<Arc<GroupProgram>, String> {
    let a = super::automata::get_or_make(&format!("group:{pattern}\u{0}{name}"), || {
        build_group(pattern, name).map(|(bytes, scratch)| (bytes, [scratch, 0, 0, 0]))
    })?;
    Ok(Arc::new(GroupProgram { automaton: a, scratch: a.words[0] }))
}

fn build_group(pattern: &str, name: &str) -> Result<(Vec<u8>, u32), String> {
    use regex_automata::util::look::Look as LookKind;
    use regex_automata::util::primitives::PatternID;
    let hir = regex_syntax::ParserBuilder::new().build().parse(pattern).map_err(|e| format!("its regex: {e}"))?;
    if !hir.properties().look_set_prefix().contains(regex_syntax::hir::Look::Start) {
        return Err("a regex not anchored at the text's start".into());
    }
    let nfa = thompson::Compiler::new().build_from_hir(&hir).map_err(|e| format!("its automaton: {e}"))?;
    let index = nfa.group_info().to_index(PatternID::ZERO, name).ok_or_else(|| format!("a group it does not name ({name})"))?;
    let slot = 2 * index;
    let states = nfa.states();
    if states.len() > GROUP_MAX_STATES {
        return Err(format!("a program of {} states (at most {GROUP_MAX_STATES})", states.len()));
    }
    const HEAD: usize = 8;
    let mut words = vec![0u32; HEAD + 4 * states.len()];
    let mut lists: Vec<u32> = Vec::new();
    let base = words.len() as u32;
    // The stack's entries at most: the start's, and each state's pushes when it is explored (once a closure).
    let mut stack = 1u32;
    for (i, st) in states.iter().enumerate() {
        use thompson::State::*;
        let mut ranges = |ts: &mut dyn Iterator<Item = (u8, u8, u32)>| {
            let at = base + lists.len() as u32;
            let mut count = 0;
            for (s, e, n) in ts {
                lists.extend([s as u32, e as u32, n]);
                count += 1;
            }
            [GROUP_SPARSE, count, at, 0]
        };
        let w: [u32; 4] = match st {
            ByteRange { trans } => [GROUP_RANGE, trans.start as u32, trans.end as u32, trans.next.as_u32()],
            Sparse(s) => ranges(&mut s.transitions.iter().map(|t| (t.start, t.end, t.next.as_u32()))),
            Dense(d) => {
                // Runs of bytes to one state, as ranges (a byte to state 0, the fail state, has none).
                let mut runs: Vec<(u8, u8, u32)> = Vec::new();
                for (b, n) in d.transitions.iter().enumerate() {
                    let (b, n) = (b as u8, n.as_u32());
                    match runs.last_mut() {
                        Some(r) if r.2 == n && r.1 as usize + 1 == b as usize => r.1 = b,
                        _ => runs.push((b, b, n)),
                    }
                }
                ranges(&mut runs.into_iter().filter(|r| r.2 != 0))
            }
            Look { look, next } => {
                let which = match look {
                    LookKind::Start => 0,
                    LookKind::End => 1,
                    other => return Err(format!("a regex with a look other than the text's start and end ({other:?})")),
                };
                stack += 1;
                [GROUP_LOOK, which, next.as_u32(), 0]
            }
            Union { alternates } => {
                let at = base + lists.len() as u32;
                lists.extend(alternates.iter().map(|s| s.as_u32()));
                stack += alternates.len() as u32;
                [GROUP_UNION, alternates.len() as u32, at, 0]
            }
            BinaryUnion { alt1, alt2 } => {
                let at = base + lists.len() as u32;
                lists.extend([alt1.as_u32(), alt2.as_u32()]);
                stack += 2;
                [GROUP_UNION, 2, at, 0]
            }
            Capture { next, slot: s, .. } => {
                let which = if s.as_usize() == slot { 0 } else if s.as_usize() == slot + 1 { 1 } else { 2 };
                stack += 2;
                [GROUP_CAPTURE, which, next.as_u32(), 0]
            }
            Fail => [GROUP_FAIL, 0, 0, 0],
            Match { .. } => [GROUP_MATCH, 0, 0, 0],
        };
        words[HEAD + 4 * i..HEAD + 4 * i + 4].copy_from_slice(&w);
    }
    let n = states.len() as u32;
    words[..HEAD].copy_from_slice(&[n, slot as u32, nfa.start_anchored().as_u32(), stack, HEAD as u32, 0, 0, 0]);
    words.extend(lists);
    // Its scratch: two thread lists (each state's place, its slot in the order, the group's two slots) and the stack
    // (two words an entry), in u32s, as rows of 64 bytes.
    let scratch = (8 * n + 2 * stack + 4) * 4;
    Ok((words.iter().flat_map(|w| w.to_le_bytes()).collect(), scratch.div_ceil(64)))
}

impl GroupProgram {
    /// Group's range of `text`, walking the program as the device's Pike VM does (tests only).
    #[cfg(test)]
    pub fn range(&self, text: &str) -> Option<(usize, usize)> {
        let b = self.automaton.bytes();
        let w: Vec<u32> = b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        group_range(&w, text.as_bytes())
    }
}

/// The device's Pike VM (`rx_group_range`) in Rust, over a program's words (tests only).
#[cfg(test)]
fn group_range(w: &[u32], x: &[u8]) -> Option<(usize, usize)> {
    const NONE: u32 = u32::MAX;
    let (n, start, at) = (w[0] as usize, w[2], w[4] as usize);
    let st = |s: u32| &w[at + 4 * s as usize..at + 4 * s as usize + 4];
    struct List {
        dense: Vec<u32>,
        sparse: Vec<u32>,
        slots: Vec<[u32; 2]>,
    }
    let closure = |l: &mut List, sid: u32, pos: usize, mut cur: [u32; 2]| {
        let mut stack: Vec<(bool, u32, u32)> = vec![(false, sid, 0)];
        while let Some((restore, a, old)) = stack.pop() {
            if restore {
                cur[a as usize] = old;
                continue;
            }
            let i = l.sparse[a as usize] as usize;
            if i < l.dense.len() && l.dense[i] == a {
                continue;
            }
            l.sparse[a as usize] = l.dense.len() as u32;
            l.dense.push(a);
            l.slots.push(cur);
            let s = st(a);
            match s[0] {
                GROUP_UNION => {
                    for k in (0..s[1]).rev() {
                        stack.push((false, w[(s[2] + k) as usize], 0));
                    }
                }
                GROUP_CAPTURE if s[1] < 2 => {
                    stack.push((true, s[1], cur[s[1] as usize]));
                    cur[s[1] as usize] = pos as u32;
                    stack.push((false, s[2], 0));
                }
                GROUP_CAPTURE => stack.push((false, s[2], 0)),
                GROUP_LOOK if (s[1] == 0 && pos == 0) || (s[1] == 1 && pos == x.len()) => stack.push((false, s[2], 0)),
                _ => {}
            }
        }
    };
    let new = || List { dense: Vec::new(), sparse: vec![0; n], slots: Vec::new() };
    let mut cl = new();
    closure(&mut cl, start, 0, [NONE, NONE]);
    let mut found = None;
    for pos in 0..=x.len() {
        let mut nl = new();
        for i in 0..cl.dense.len() {
            let s = st(cl.dense[i]);
            if s[0] == GROUP_MATCH {
                found = Some(cl.slots[i]);
                break;
            }
            if pos == x.len() {
                continue;
            }
            let b = x[pos] as u32;
            let next = match s[0] {
                GROUP_RANGE if s[1] <= b && b <= s[2] => s[3],
                GROUP_SPARSE => (0..s[1]).map(|k| (s[2] + 3 * k) as usize).find(|&r| w[r] <= b && b <= w[r + 1]).map_or(NONE, |r| w[r + 2]),
                _ => NONE,
            };
            if next != NONE {
                closure(&mut nl, next, pos + 1, cl.slots[i]);
            }
        }
        cl = nl;
    }
    found.filter(|s| s[0] != NONE && s[1] != NONE).map(|s| (s[0] as usize, s[1] as usize))
}

#[cfg(test)]
mod tests {
    /// A capture group's program, walked as the device's Pike VM walks it, gives Rust's `regex`'s group: the C
    /// includes' splits, greedy and lazy repeats, alternations of overlapping arms, a group in a repeat (its last
    /// iteration), a group that takes no part, multi-byte classes. Patterns not anchored at the start, with other
    /// looks, or naming no such group are refused.
    #[test]
    fn a_groups_program_captures_as_regex_does() {
        let cases: &[(&str, &[&str])] = &[
            ("^(?:(?<d>.*)/[^/]*)$", &["d"]),
            ("^(?:(?<dd>.*)/(?<bb>[^/]*))$", &["dd", "bb"]),
            ("^(?:(?<fw>[^/]+)/(?<rest>.+))$", &["fw", "rest"]),
            ("^(?:(?<a>a*?)(?<b>a*))$", &["a", "b"]),
            ("^(?:(?<x>ab|a)(?<y>c|bcd))$", &["x", "y"]),
            ("^(?:(?:(?<p>[a-z])\\.)+(?<q>.*))$", &["p", "q"]),
            ("^(?:(?<u>x)|(?<v>[^x].*))$", &["u", "v"]),
            ("^(?:(?<g>[αβ]+)é?(?<h>\\w*))$", &["g", "h"]),
            ("^(?<s>\\s*)(?<t>.*?)(?<e>\\s*)$", &["s", "t", "e"]),
        ];
        let texts = ["", "a", "aa", "a/b", "a/b/c", "/x", "x/", "A.framework/b.h", "abcd", "abc", "a.b.c.rest", "x", "xy", "y", "αβé", "αβαx_1", "  pad  ", "a//b", "é/ü/ö"];
        for &(p, names) in cases {
            let re = regex::Regex::new(p).unwrap();
            for &name in names {
                let g = super::group_program(p, name).unwrap_or_else(|e| panic!("{p} {name}: {e}"));
                for t in texts {
                    let want = re.captures(t).and_then(|c| c.name(name)).map(|m| (m.start(), m.end()));
                    assert_eq!(g.range(t), want, "group {name} of {p:?} in {t:?}");
                }
            }
        }
        for (p, name) in [("(?<d>a)/", "d"), ("^(?<d>a)\\b", "d"), ("^(?m)(?<d>a)$", "d"), ("^(?<d>a)$", "e")] {
            assert!(super::group_program(p, name).is_err(), "{p} {name}");
        }
    }

    /// The replacement automaton replaces as Rust's `regex` does, over the packs' patterns; the empty-matching and
    /// word-boundary ones are refused.
    #[test]
    fn a_replacers_automaton_replaces_as_regex_does() {
        let texts = ["", "  a b  ", "x\tsrc/lib.ts  ", "node:fs", "__init__", "#x\n#y", "tURt", "a%2Cb%2cc", "@ts-ignore here", "αβ  γ", "n", "nn", "a.b.c", "x*y", "`a${b}`", "q&display=x&y", "aabbc abbbc", "abcd abc", "xαβy é xαy", "\u{2003}a\u{2003}"];
        let patterns = ["^\\s+|\\s+$", "\\s+", "#[^\\n]*", "_", "n$", "^node:", "^_+", "[rtU]", "%2[Cc]", "\\*", "\\.", "/", "@ts-ignore", "\\s", "^`|\\$\\{[\\s\\S]*$|`$", "&display[=&][\\s\\S]*", "a+b|ab+c", "(ab|a)(c|bcd)", "x[αβ]+y|é"];
        for p in patterns {
            let r = super::replacer(p).unwrap_or_else(|e| panic!("{p}: {e}"));
            let re = regex::Regex::new(p).unwrap();
            for t in texts {
                assert_eq!(r.replace(t, "<>"), re.replace_all(t, "<>"), "replace_all({t:?}, {p:?})");
            }
        }
        for p in ["a*", "\\bx", "(?m)^x", "x?"] {
            assert!(super::replacer(p).is_err(), "{p} refused");
        }
    }


    use crate::code_rule_dsl::logic::strings::Matcher;

    /// The table answers as the CPU's matcher does, for every kind of test.
    #[test]
    fn a_tests_table_answers_as_its_matcher() {
        let texts = ["", "foo", "foobar", "barfoo", "xfoox", "Foo", "a.b.c", "src/lib/x.ts", "αβγ", "fo", "test_case", "TODO: x"];
        let cases: [(&str, &[&str]); 12] = [
            ("prefix", &["foo"]),
            ("suffix", &["foo", ".ts"]),
            ("contains", &["oo", "β"]),
            ("equals", &["foo", "fo"]),
            ("iprefix", &["FOO"]),
            ("glob", &["src/**/*.ts"]),
            ("rx", &["^[a-z]+_[a-z]+$"]),
            ("rx", &["(?i)todo"]),
            ("rx", &["o{2}"]),
            ("rx", &["(?s).*"]),
            ("rx", &["a\\.b"]),
            ("word", &["case"]),
        ];
        for (op, patterns) in cases {
            let m = Matcher::new(op, &patterns.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>()).unwrap();
            let t = match super::table(m.plans()) {
                Ok(t) => t,
                // A Unicode word boundary has no table: the test runs on the CPU.
                Err(e) => {
                    assert_eq!(op, "word", "{op} {patterns:?}: {e}");
                    continue;
                }
            };
            for text in texts {
                assert_eq!(t.is_match(text.as_bytes()), m.is_match(text), "@{op}({text:?}, {patterns:?})");
            }
        }
    }
}
