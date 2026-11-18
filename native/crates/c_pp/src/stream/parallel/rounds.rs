//! P1: the directives' fixpoint. A round reads the state the round before left (the include
//! tree, and each instance's active groups), and from it:
//!
//! 1. lays the unit out: each instance's size bottom-up, then each one's first position
//!    top-down (prefix sums over the tree), so every directive and token has its position;
//! 2. sorts the active definitions by (name, position): the table every use searches;
//! 3. evaluates every test at its position, each alone;
//! 4. takes each chain's first true arm, and makes a group active where its chain's parent is
//!    (a scan over the groups, outermost first);
//! 5. resolves every active include at its position, and drops those an earlier `#pragma once`
//!    or `#import` of their file skips (a search per include).
//!
//! The rounds stop when one leaves the state it read.

use super::files::{Act, Tab, tabulate};
use super::{Ev, Isolated, SUB, Stats, Vis, in_context};
use std::collections::HashSet;
use super::super::Pp;
use crate::directives::Kind;
use rerail_c_lean::pp::P;
use std::collections::HashMap;

/// No instance: an include not entered.
pub(super) const NONE: u32 = u32::MAX;

/// The rounds' bound: a unit past it goes to the serial pass.
const MAX_ROUNDS: u32 = 1000;

/// An instance of a file in the include tree: its file, and its files from the main one's.
pub(super) struct Inst {
    pub file: u32,
    pub chain: Vec<u32>,
}

/// A unit's instances and files' tables.
pub(super) struct Tu {
    pub insts: Vec<Inst>,
    /// By (instance, directive, file): the instance its include enters.
    key: HashMap<(u32, u32, u32), u32>,
    /// By file: its tables, made when it is first entered.
    pub tabs: Vec<Option<Tab>>,
    /// By (file, directive): the file an include with a literal name finds (none: `NONE`), the
    /// same from every instance.
    found: HashMap<(u32, u32), u32>,
    /// The predefines' instance, then the main file's: the unit's order.
    pub roots: [u32; 2],
}

impl Tu {
    pub fn new(pp: &mut Pp, builtin: u32, main: u32) -> Tu {
        let mut tu = Tu { insts: Vec::new(), key: HashMap::new(), tabs: Vec::new(), found: HashMap::new(), roots: [0, 1] };
        // The predefines are read with the main file below them (`Pp::run`).
        tu.insts.push(Inst { file: builtin, chain: vec![main, builtin] });
        tu.insts.push(Inst { file: main, chain: vec![main] });
        tu.tab(pp, builtin);
        tu.tab(pp, main);
        tu
    }

    pub fn tab_of(&self, inst: u32) -> &Tab {
        self.tabs[self.insts[inst as usize].file as usize].as_ref().expect("an entered file's tables")
    }

    fn tab(&mut self, pp: &mut Pp, file: u32) {
        if self.tabs.len() <= file as usize {
            self.tabs.resize_with(file as usize + 1, || None);
        }
        if self.tabs[file as usize].is_none() {
            self.tabs[file as usize] = Some(tabulate(pp, file));
        }
    }
}

/// What a round reads and leaves: by instance (empty where it is not in the tree), its groups'
/// activity, the instance each of its directives entered, and the pragmas its text's `_Pragma`s
/// made (the invocation's first token, the pragma, its name's symbol).
#[derive(Clone, PartialEq, Eq, Default)]
pub(super) struct State {
    pub groups: Vec<Vec<bool>>,
    pub child: Vec<Vec<u32>>,
    pub events: Vec<Vec<(u32, Ev, u32)>>,
}

impl State {
    pub fn active(&self, tab: &Tab, inst: u32, d: usize) -> bool {
        self.groups[inst as usize][tab.dirs[d].group as usize]
    }
}

/// The unit laid out: the instances in the tree, in order; by instance, its first position, its
/// size, and before each directive the sizes its earlier directives' includes take.
pub(super) struct Layout {
    pub order: Vec<u32>,
    pub base: Vec<u64>,
    shift: Vec<Vec<u64>>,
}

impl Layout {
    /// Directive `d`'s slot in instance `i`: a slot a directive or token, in the unit's order.
    fn dir_slot(&self, tab: &Tab, i: u32, d: usize) -> u64 {
        self.base[i as usize] + d as u64 + tab.dirs[d].text_before as u64 + self.shift[i as usize][d]
    }

    /// Directive `d`'s position in instance `i`: its slot, `SUB` bits up.
    pub fn dir_pos(&self, tab: &Tab, i: u32, d: usize) -> u64 {
        self.dir_slot(tab, i, d) << SUB
    }

    /// The position of the first text token after `k` directives in instance `i`.
    pub fn text_pos(&self, tab: &Tab, i: u32, k: usize) -> u64 {
        let first = if k == 0 { 0 } else { tab.dirs[k - 1].text_before };
        (self.base[i as usize] + first as u64 + k as u64 + self.shift[i as usize][k]) << SUB
    }

    /// Text token `t`'s position in instance `i`.
    pub fn token_pos(&self, tab: &Tab, i: u32, t: u32) -> u64 {
        let k = tab.dirs.partition_point(|d| d.text_before <= t);
        let first = if k == 0 { 0 } else { tab.dirs[k - 1].text_before };
        self.text_pos(tab, i, k) + (((t - first) as u64) << SUB)
    }
}

fn layout(tu: &Tu, state: &State) -> Layout {
    let n = tu.insts.len();
    // The tree in preorder, from the roots.
    let mut order = Vec::new();
    let mut todo: Vec<u32> = tu.roots.iter().rev().copied().collect();
    while let Some(i) = todo.pop() {
        order.push(i);
        for &c in state.child[i as usize].iter().rev() {
            if c != NONE {
                todo.push(c);
            }
        }
    }
    // Sizes bottom-up: an instance's directives and tokens, and its includes' sizes.
    let mut size = vec![0u64; n];
    let mut shift = vec![Vec::new(); n];
    for &i in order.iter().rev() {
        let tab = tu.tab_of(i);
        let mut sh = Vec::with_capacity(tab.dirs.len() + 1);
        let mut acc = 0u64;
        sh.push(0);
        for &c in &state.child[i as usize] {
            if c != NONE {
                acc += size[c as usize];
            }
            sh.push(acc);
        }
        size[i as usize] = tab.dirs.len() as u64 + tab.text.len() as u64 + acc;
        shift[i as usize] = sh;
    }
    // First positions top-down: an include's instance starts after its directive.
    let mut lay = Layout { order, base: vec![0; n], shift };
    let mut next = 1u64;
    for &r in &tu.roots {
        lay.base[r as usize] = next;
        next += size[r as usize];
    }
    for k in 0..lay.order.len() {
        let i = lay.order[k];
        let tab = tu.tab_of(i);
        for (d, &c) in state.child[i as usize].iter().enumerate() {
            if c != NONE {
                lay.base[c as usize] = lay.dir_slot(tab, i, d) + 1;
            }
        }
    }
    lay
}

/// The fixpoint: the state no round changes, and its layout.
pub(super) fn settle(pp: &mut Pp, tu: &mut Tu, builtins: &[(u32, u64, u32)], stats: &mut Stats) -> Result<(State, Layout), &'static str> {
    let mut state = State::default();
    for i in 0..tu.insts.len() {
        fresh(tu, &mut state, i as u32);
    }
    let pragma = pp.syms.get(b"_Pragma");
    let mut reach = (0, HashSet::new());
    for round in 1..=MAX_ROUNDS {
        stats.rounds = round;
        let lay = layout(tu, &state);
        // The active definitions, pushes and pops by (name, position).
        let mut raw: Vec<(u32, u64, Row)> = builtins.iter().map(|&(s, p, d)| (s, p, Row::Set(d))).collect();
        for &i in &lay.order {
            let tab = tu.tab_of(i);
            for (d, dir) in tab.dirs.iter().enumerate() {
                if !state.active(tab, i, d) {
                    continue;
                }
                let pos = lay.dir_pos(tab, i, d);
                match dir.act {
                    Act::Define(k) => raw.push((pp.defs[k as usize].name, pos, Row::Set(k + 1))),
                    Act::Undef(s) => raw.push((s, pos, Row::Set(0))),
                    Act::Push(s) => raw.push((s, pos, Row::Push)),
                    Act::Pop(s) => raw.push((s, pos, Row::Pop)),
                    _ => {}
                }
            }
            for (k, &(t, ev, sym)) in state.events[i as usize].iter().enumerate() {
                let pos = lay.token_pos(tab, i, t) + 1 + k as u64;
                match ev {
                    Ev::Push => raw.push((sym, pos, Row::Push)),
                    Ev::Pop => raw.push((sym, pos, Row::Pop)),
                    Ev::Once => {}
                }
            }
        }
        let rows = resolve_rows(raw);
        stats.definitions = (rows.len() - builtins.len()) as u64;
        pp.vis = Some(Vis { rows });
        let mut next = State { groups: vec![Vec::new(); tu.insts.len()], child: vec![Vec::new(); tu.insts.len()], events: vec![Vec::new(); tu.insts.len()] };
        // Each test at its position, then each group's activity.
        for &i in &lay.order {
            let tab = tu.tab_of(i);
            let chain = &tu.insts[i as usize].chain;
            let mut g = vec![false; tab.groups.len()];
            g[0] = true;
            // The chains in their openings' order, so that a chain's parent group is read before
            // it: a chain is read where its parent group is active, its arms to the first true.
            for arms in &tab.chains {
                let parent = tab.groups[tab.dirs[arms[0] as usize].after as usize].parent;
                if !g[parent as usize] {
                    continue;
                }
                for &d in arms {
                    let pos = lay.dir_pos(tab, i, d as usize);
                    stats.tests += 1;
                    if in_context(pp, chain, pos, |pp| test(pp, tab, d as usize))? {
                        g[tab.dirs[d as usize].after as usize] = true;
                        break;
                    }
                }
            }
            next.groups[i as usize] = g;
            next.child[i as usize] = vec![NONE; tab.dirs.len()];
        }
        // Each active include resolved at its position; one with a literal name, once a file.
        let mut found: Vec<(u32, u32, u32, u64, Kind)> = Vec::new();
        let mut learnt = Vec::new();
        for &i in &lay.order {
            let tab = tu.tab_of(i);
            let (file, chain) = (tu.insts[i as usize].file, &tu.insts[i as usize].chain);
            for (d, dir) in tab.dirs.iter().enumerate() {
                if dir.act != Act::Include || !next.active(tab, i, d) {
                    continue;
                }
                let pos = lay.dir_pos(tab, i, d);
                let literal = dir.line.first().is_some_and(|t| matches!(t.kind, P::Header | P::Str) || t.is(rerail_c_lean::T::Lt));
                let f = match tu.found.get(&(file, d as u32)) {
                    Some(&f) if literal => (f != NONE).then_some(f),
                    _ => {
                        stats.includes += 1;
                        let f = in_context(pp, chain, pos, |pp| resolve(pp, tab, d))?;
                        if literal && chain.len() as u32 <= pp.cfg.max_depth {
                            learnt.push(((file, d as u32), f.unwrap_or(NONE)));
                        }
                        f
                    }
                };
                // Past the depth's bound, nothing is entered (`Pp::include`).
                if let Some(f) = f.filter(|_| chain.len() as u32 <= pp.cfg.max_depth) {
                    found.push((i, d as u32, f, pos, dir.kind));
                }
            }
        }
        tu.found.extend(learnt);
        // The pragmas the text's `_Pragma`s make: the invocations that can reach `_Pragma`,
        // expanded under this round's table where this round's groups are active.
        if reach.0 != pp.defs.len() {
            reach = (pp.defs.len(), reaching(pp, pragma));
        }
        let events = super::text::pragma_events(pp, tu, &next.groups, &lay, &reach.1);
        // `#pragma once` and `#import`: from their position on, their file is not entered again.
        let mut once: Vec<(u32, u64)> = Vec::new();
        for &i in &lay.order {
            let tab = tu.tab_of(i);
            for (d, dir) in tab.dirs.iter().enumerate() {
                if dir.act == Act::Once && next.active(tab, i, d) {
                    once.push((tu.insts[i as usize].file, lay.dir_pos(tab, i, d)));
                }
            }
            for (k, &(t, ev, _)) in events[i as usize].iter().enumerate() {
                if ev == Ev::Once {
                    once.push((tu.insts[i as usize].file, lay.token_pos(tab, i, t) + 1 + k as u64));
                }
            }
        }
        once.extend(found.iter().filter(|f| f.4 == Kind::Import).map(|f| (f.2, f.3)));
        once.sort_unstable();
        for (i, d, f, pos, _) in found {
            let k = once.partition_point(|&(g, _)| g < f);
            if once.get(k).is_some_and(|&(g, p)| g == f && p < pos) {
                continue;
            }
            tu.tab(pp, f);
            let guard = tu.tabs[f as usize].as_ref().unwrap().guard;
            if guard.is_some_and(|g| pp.vis.as_ref().unwrap().at(g, pos).is_some()) {
                continue;
            }
            let c = match tu.key.get(&(i, d, f)) {
                Some(&c) => c,
                None => {
                    let c = tu.insts.len() as u32;
                    let mut chain = tu.insts[i as usize].chain.clone();
                    chain.push(f);
                    tu.insts.push(Inst { file: f, chain });
                    tu.key.insert((i, d, f), c);
                    c
                }
            };
            next.child[i as usize][d as usize] = c;
        }
        for &i in &lay.order {
            next.events[i as usize] = events[i as usize].clone();
        }
        // An instance entered for the first time: its groups as yet unread.
        let entered: Vec<u32> = next.child.iter().flatten().copied().filter(|&c| c != NONE && next.groups.get(c as usize).is_none_or(|g| g.is_empty())).collect();
        next.groups.resize(tu.insts.len(), Vec::new());
        next.child.resize(tu.insts.len(), Vec::new());
        next.events.resize(tu.insts.len(), Vec::new());
        for c in entered {
            fresh(tu, &mut next, c);
        }
        if next == state {
            for &i in &lay.order {
                let tab = tu.tab_of(i);
                stats.directives += tab.dirs.len() as u64;
                stats.tokens += tab.text.len() as u64;
                for (d, dir) in tab.dirs.iter().enumerate() {
                    if state.active(tab, i, d) {
                        stats.active_directives += 1;
                        if let Act::Unsupported(why) = dir.act {
                            return Err(why);
                        }
                    }
                }
            }
            stats.instances = lay.order.len() as u32;
            stats.files = tu.tabs.iter().flatten().count() as u32;
            stats.depth = lay.order.iter().map(|&i| tu.insts[i as usize].chain.len() as u32).max().unwrap_or(0);
            return Ok((state, lay));
        }
        state = next;
    }
    Err("past the rounds' bound")
}

/// Instance `i` as first entered: its own group active, nothing else, no includes.
fn fresh(tu: &Tu, state: &mut State, i: u32) {
    let tab = tu.tab_of(i);
    if state.groups.len() <= i as usize {
        state.groups.resize(i as usize + 1, Vec::new());
        state.child.resize(i as usize + 1, Vec::new());
        state.events.resize(i as usize + 1, Vec::new());
    }
    state.events[i as usize].clear();
    let mut g = vec![false; tab.groups.len()];
    g[0] = true;
    state.groups[i as usize] = g;
    state.child[i as usize] = vec![NONE; tab.dirs.len()];
}

/// A row of the table before pushes and pops are resolved: a definition (+1; 0 an `#undef`),
/// a push, a pop.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Row {
    Set(u32),
    Push,
    Pop,
}

/// The table: rows sorted by (name, position), each pop the value at its push (pushes and pops
/// matched per name by depth; a pop with no push changes nothing), pushes no rows.
fn resolve_rows(mut raw: Vec<(u32, u64, Row)>) -> Vec<(u32, u64, u32)> {
    raw.sort_unstable();
    let mut rows = Vec::with_capacity(raw.len());
    let (mut sym, mut cur, mut pushed) = (NONE, 0, Vec::new());
    for (s, p, r) in raw {
        if s != sym {
            (sym, cur) = (s, 0);
            pushed.clear();
        }
        match r {
            Row::Set(v) => cur = v,
            Row::Push => {
                pushed.push(cur);
                continue;
            }
            Row::Pop => match pushed.pop() {
                Some(v) => cur = v,
                None => continue,
            },
        }
        rows.push((s, p, cur));
    }
    rows
}

/// The names whose invocations can reach `_Pragma`: `_Pragma`, and any whose definition (any
/// of them) names one.
fn reaching(pp: &Pp, pragma: u32) -> HashSet<u32> {
    let mut r = HashSet::from([pragma]);
    loop {
        let n = r.len();
        for d in &pp.defs {
            if d.builtin.is_none() && !r.contains(&d.name) && d.body.iter().any(|t| r.contains(&t.sym)) {
                r.insert(d.name);
            }
        }
        if r.len() == n {
            return r;
        }
    }
}

/// A directive read where `pp` stands, with what it meets checked: `_Pragma`s (the shape places
/// them in text alone), and `__COUNTER__`, which counts in the unit's order.
fn checked<R>(pp: &mut Pp, f: impl FnOnce(&mut Pp) -> R) -> Result<R, &'static str> {
    let counter = pp.counter;
    pp.par = Some(Isolated { outer: usize::MAX, peeked: false, cut: false, events: Vec::new() });
    let r = f(pp);
    let par = pp.par.take().expect("the reading's state");
    if !par.events.is_empty() {
        return Err("_Pragma in a directive");
    }
    if pp.counter != counter {
        return Err("__COUNTER__ in a directive");
    }
    Ok(r)
}

/// Arm `d`'s test (`Pp::directive`'s and `Pp::skip_group`'s reading).
fn test(pp: &mut Pp, tab: &Tab, d: usize) -> Result<bool, &'static str> {
    let dir = &tab.dirs[d];
    let file = dir.hash.buf - 1;
    checked(pp, |pp| match dir.kind {
        Kind::Else => true,
        Kind::If | Kind::Elif => pp.if_value(file, dir.hash.start, dir.line.clone()),
        k => {
            let v = match dir.line.first() {
                Some(n) if n.kind == P::Ident => pp.def_of(n.sym).is_some(),
                _ => false,
            };
            v == matches!(k, Kind::Ifdef | Kind::Elifdef)
        }
    })
}

/// Include `d`'s file (`Pp::include`'s search), `None` where it enters none.
fn resolve(pp: &mut Pp, tab: &Tab, d: usize) -> Result<Option<u32>, &'static str> {
    let dir = &tab.dirs[d];
    checked(pp, |pp| {
        let (name, angle) = pp.header_of(dir.line.clone())?;
        let depth = pp.stack.len() as u32;
        if depth > pp.cfg.max_depth {
            return None;
        }
        let (path, found_by) = pp.find(&name, angle, dir.kind == Kind::IncludeNext)?;
        Some(match pp.by_path.get(&path) {
            Some(&id) => id,
            None => {
                let src = pp.io.read(&path)?;
                pp.add_file(path, src, found_by)
            }
        })
    })
}
