//! Format 16's node stream (0156 §1): an open record per node in preorder, a close record per node in
//! postorder, every position the distance from the event before it.

use crate::tree::{NONE, Node, Tree};
use std::collections::BTreeMap;

/// Bit 31 of `lead` or `trail`: the distance is measured backwards (an event before the one before it).
pub const BACK: u32 = 1 << 31;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Open {
    pub kind: u16,
    pub field: u8,
    pub flags: u8,
    /// Supertype memberships as a mask over the kind's candidate list (`Candidates`).
    pub memb: u16,
    pub lead: u32,
    pub pops: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Events {
    pub opens: Vec<Open>,
    /// One per node, in postorder.
    pub trails: Vec<u32>,
    /// Closes after the last open.
    pub end_pops: u32,
    /// The source's length less the last event's position.
    pub final_gap: u32,
}

/// The supertypes each (language, kind) can have, ascending: what a membership mask indexes. The grammar
/// fixes them (`node-types.json`); the lab takes the union seen on the corpus, a subset of the grammar's.
#[derive(Clone, Debug, Default)]
pub struct Candidates {
    pub by_kind: BTreeMap<(u8, u16), Vec<u16>>,
}

impl Candidates {
    pub fn learn(&mut self, tag: u8, tree: &Tree) {
        for n in &tree.nodes {
            let list = self.by_kind.entry((tag, n.kind)).or_default();
            for c in &n.classes {
                if let Err(at) = list.binary_search(c) {
                    list.insert(at, *c);
                }
            }
        }
    }

    pub fn mask(&self, tag: u8, node: &Node) -> u16 {
        if node.classes.is_empty() {
            return 0;
        }
        let list = &self.by_kind[&(tag, node.kind)];
        assert!(list.len() <= 16, "more than 16 candidate supertypes for kind {} (language {tag})", node.kind);
        node.classes.iter().fold(0u16, |m, c| m | 1 << list.binary_search(c).expect("a learned candidate"))
    }

    pub fn classes(&self, tag: u8, kind: u16, mask: u16) -> Vec<u16> {
        if mask == 0 {
            return Vec::new();
        }
        let list = &self.by_kind[&(tag, kind)];
        (0..16).filter(|b| mask & (1 << b) != 0).map(|b| list[b]).collect()
    }

    /// The largest candidate list per language.
    pub fn widest(&self) -> BTreeMap<u8, (usize, u16)> {
        let mut out = BTreeMap::new();
        for (&(tag, kind), list) in &self.by_kind {
            let e = out.entry(tag).or_insert((0, 0));
            if list.len() > e.0 {
                *e = (list.len(), kind);
            }
        }
        out
    }
}

fn delta(to: u64, from: u64) -> u32 {
    let (d, back) = if to >= from { (to - from, 0) } else { (from - to, BACK) };
    assert!(d < u64::from(BACK), "a gap or leaf of 2 GiB or more");
    d as u32 | back
}

pub fn apply(from: u64, d: u32) -> u64 {
    if d & BACK != 0 { from - u64::from(d & !BACK) } else { from + u64::from(d) }
}

pub fn encode(tree: &Tree, tag: u8, candidates: &Candidates) -> Events {
    let n = tree.len();
    let mut ev = Events { opens: Vec::with_capacity(n), trails: Vec::with_capacity(n), ..Default::default() };
    let mut last = 0u64;
    let mut open: Vec<usize> = Vec::new();
    for i in 0..n {
        let mut pops = 0;
        while let Some(&j) = open.last() {
            if j + tree.size[j] as usize > i {
                break;
            }
            open.pop();
            ev.trails.push(delta(tree.nodes[j].end, last));
            last = tree.nodes[j].end;
            pops += 1;
        }
        let node = &tree.nodes[i];
        ev.opens.push(Open {
            kind: node.kind,
            field: node.field,
            flags: node.flags,
            memb: candidates.mask(tag, node),
            lead: delta(node.start, last),
            pops,
        });
        last = node.start;
        open.push(i);
    }
    while let Some(j) = open.pop() {
        ev.trails.push(delta(tree.nodes[j].end, last));
        last = tree.nodes[j].end;
        ev.end_pops += 1;
    }
    ev.final_gap = delta(tree.source_len, last);
    ev
}

/// The tree back, with sizes and parents derived by a stack (the CPU's form of 0156 §4).
pub fn decode(ev: &Events, tag: u8, candidates: &Candidates, source_len: u64) -> Tree {
    let n = ev.opens.len();
    let mut nodes = Vec::with_capacity(n);
    let mut size = vec![0u32; n];
    let mut parent = vec![NONE; n];
    let mut open: Vec<usize> = Vec::new();
    let mut last = 0u64;
    let mut t = 0usize;
    let mut close = |open: &mut Vec<usize>, nodes: &mut Vec<Node>, last: &mut u64, at: usize| {
        let j = open.pop().expect("a close with a node open");
        *last = apply(*last, ev.trails[t]);
        t += 1;
        nodes[j].end = *last;
        size[j] = (at - j) as u32;
    };
    for (i, o) in ev.opens.iter().enumerate() {
        for _ in 0..o.pops {
            close(&mut open, &mut nodes, &mut last, i);
        }
        last = apply(last, o.lead);
        parent[i] = open.last().map_or(NONE, |&p| p as u32);
        nodes.push(Node {
            kind: o.kind,
            field: o.field,
            flags: o.flags,
            classes: candidates.classes(tag, o.kind, o.memb),
            start: last,
            end: 0,
        });
        open.push(i);
    }
    for _ in 0..ev.end_pops {
        close(&mut open, &mut nodes, &mut last, n);
    }
    Tree { nodes, size, parent, source_len }
}

/// Positions only, no parents or sizes: what a reader that needs spans pays (T5).
pub fn positions(ev: &Events, start: &mut Vec<u64>, end: &mut Vec<u64>) {
    let n = ev.opens.len();
    start.clear();
    end.clear();
    start.resize(n, 0);
    end.resize(n, 0);
    let mut open: Vec<u32> = Vec::with_capacity(64);
    let mut last = 0u64;
    let mut t = 0;
    for (i, o) in ev.opens.iter().enumerate() {
        for _ in 0..o.pops {
            let j = open.pop().unwrap() as usize;
            last = apply(last, ev.trails[t]);
            t += 1;
            end[j] = last;
        }
        last = apply(last, o.lead);
        start[i] = last;
        open.push(i as u32);
    }
    for _ in 0..ev.end_pops {
        let j = open.pop().unwrap() as usize;
        last = apply(last, ev.trails[t]);
        t += 1;
        end[j] = last;
    }
}
