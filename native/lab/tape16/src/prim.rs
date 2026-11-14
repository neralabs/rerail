//! E12 (t39): the tree primitives rules ask for, answered on each candidate tape in place, in two forms:
//!
//! - **bulk**: the answer for every node of a file in one pass, as the engine evaluates a rule a set at a
//!   time (`nearest`, `inside`, `has` are "one tape pass per axis and set", core.rr);
//! - **point**: one node's answer, at random.
//!
//! Kind sets are named as rules name them (`kinds.tsv`, from `rerail-engine kind-names`), resolved per
//! language to a mask over the language's codes. Nothing is stored beside the tape: each answer is read
//! from the reader's own questions (`inplace::Reader`). Every answer is checked against the tree.

use crate::inplace::{Codes, Reader};
use crate::tree::{NONE, Tree};
use std::collections::HashMap;

/// `kinds.tsv`: `tag, id, name, named` for kinds, `tag, "field id", name` for fields.
pub struct Names {
    kinds: HashMap<(u8, String), Vec<u16>>,
    fields: HashMap<(u8, String), u8>,
}

impl Names {
    pub fn load() -> Names {
        let (mut kinds, mut fields) = (HashMap::<(u8, String), Vec<u16>>::new(), HashMap::new());
        for line in include_str!("../kinds.tsv").lines() {
            let c: Vec<&str> = line.split('\t').collect();
            let Ok(tag) = c[0].parse::<u8>() else { continue };
            if let Some(id) = c[1].strip_prefix("field ") {
                fields.insert((tag, c[2].to_string()), id.parse().unwrap());
            } else if c.get(3) == Some(&"1") {
                kinds.entry((tag, c[2].to_string())).or_default().push(c[1].parse().unwrap());
            }
        }
        Names { kinds, fields }
    }
    pub fn field(&self, tag: u8, name: &str) -> Option<u8> {
        self.fields.get(&(tag, name.to_string())).copied()
    }
    /// The language's codes whose kind is one of `names`.
    pub fn set(&self, tag: u8, codes: &Codes, names: &[&str]) -> Vec<bool> {
        let ids: Vec<u16> = names.iter().flat_map(|n| self.kinds.get(&(tag, n.to_string())).cloned().unwrap_or_default()).collect();
        codes.table.iter().map(|t| ids.contains(&t.0)).collect()
    }
}

/// The sets measured, as the rules write them (functions, classes, blocks, calls, strings; wrappers).
pub const SETS: [(&str, &[&str]); 5] = [
    ("fn", &["function_definition", "lambda", "function_declaration", "function_expression", "arrow_function", "method_definition", "generator_function_declaration", "function_item", "closure_expression"]),
    ("class", &["class_definition", "class_declaration", "class", "impl_item", "struct_item", "trait_item"]),
    ("block", &["block", "statement_block"]),
    ("call", &["call", "call_expression"]),
    ("string", &["string", "string_literal", "template_string"]),
];
pub const WRAP: &[&str] = &["parenthesized_expression"];

/// nearest(n, a, S) for every node: a is n's nearest strict ancestor in S (`NONE`: none), by preorder.
/// One sweep with a stack of the open S-nodes; only an S-node's subtree end is read.
pub fn nearest_all<R: Reader>(r: &R, set: &[bool], out: &mut Vec<u32>) {
    out.clear();
    let mut stack: Vec<(u32, usize)> = Vec::with_capacity(32);
    let mut id = r.id_of(0);
    let mut p = 0u32;
    loop {
        while stack.last().is_some_and(|&(_, end)| end <= id) {
            stack.pop();
        }
        out.push(stack.last().map_or(NONE, |&(a, _)| a));
        if set[r.code(id) as usize] {
            stack.push((p, r.subtree_end(id)));
        }
        p += 1;
        match r.next(id) {
            Some(n) => id = n,
            None => break,
        }
    }
}

/// The outermost strict ancestor in S, for every node: the bottom of the same stack.
pub fn outermost_all<R: Reader>(r: &R, set: &[bool], out: &mut Vec<u32>) {
    out.clear();
    let mut stack: Vec<(u32, usize)> = Vec::with_capacity(32);
    let mut id = r.id_of(0);
    let mut p = 0u32;
    loop {
        while stack.last().is_some_and(|&(_, end)| end <= id) {
            stack.pop();
        }
        out.push(stack.first().map_or(NONE, |&(a, _)| a));
        if set[r.code(id) as usize] {
            stack.push((p, r.subtree_end(id)));
        }
        p += 1;
        match r.next(id) {
            Some(n) => id = n,
            None => break,
        }
    }
}

/// has(a, _, S) for every node: some strict descendant of a is in S. One sweep keeping, for each open node,
/// the preorder number of the last S-node seen: a node has one if an S-node came after it before its end.
/// (Forward, so B's ids need no reverse step: the stack holds the open nodes and their first number.)
pub fn has_all<R: Reader>(r: &R, set: &[bool], out: &mut Vec<bool>) {
    out.clear();
    let mut stack: Vec<(u32, usize)> = Vec::with_capacity(64);
    let mut id = r.id_of(0);
    let mut p = 0u32;
    // The preorder number of the last S-node seen so far (u32::MAX: none).
    let mut last = u32::MAX;
    loop {
        while stack.last().is_some_and(|&(_, end)| end <= id) {
            let (a, _) = stack.pop().unwrap();
            out[a as usize] = last != u32::MAX && last > a;
        }
        out.push(false);
        if set[r.code(id) as usize] {
            last = p;
        }
        stack.push((p, r.subtree_end(id)));
        p += 1;
        match r.next(id) {
            Some(n) => id = n,
            None => break,
        }
    }
    while let Some((a, _)) = stack.pop() {
        out[a as usize] = last != u32::MAX && last > a;
    }
}

/// nearest for one node: climb parents until one is in S.
#[inline(always)]
pub fn nearest_one<R: Reader>(r: &R, set: &[bool], id: usize) -> Option<usize> {
    let mut q = r.parent(id);
    while let Some(a) = q {
        if set[r.code(a) as usize] {
            return Some(a);
        }
        q = r.parent(a);
    }
    None
}

/// has for one node: the codes of its subtree read in order until one is in S.
#[inline(always)]
pub fn has_one<R: Reader>(r: &R, set: &[bool], id: usize) -> bool {
    let end = r.subtree_end(id);
    let mut q = r.next(id);
    while let Some(c) = q {
        if c >= end {
            return false;
        }
        if set[r.code(c) as usize] {
            return true;
        }
        q = r.next(c);
    }
    false
}

/// field(p, f, c): the first child of `id` whose field is `f` (`field_of` maps a code to its field).
#[inline(always)]
pub fn field_one<R: Reader>(r: &R, field_of: &[u8], f: u8, id: usize) -> Option<usize> {
    let pe = r.subtree_end(id);
    let mut c = r.first_child(id);
    while let Some(x) = c {
        if field_of[r.code(x) as usize] == f {
            return Some(x);
        }
        c = r.next_sibling(x, pe);
    }
    None
}

/// unparen: down through wrapper kinds by first child.
#[inline(always)]
pub fn unparen_one<R: Reader>(r: &R, wrap: &[bool], mut id: usize) -> usize {
    while wrap[r.code(id) as usize] {
        match r.first_child(id) {
            Some(c) => id = c,
            None => break,
        }
    }
    id
}

/// The tree's answers, by preorder number, for the checks.
pub mod truth {
    use super::*;
    pub fn nearest(t: &Tree, inset: &[bool]) -> Vec<u32> {
        (0..t.len()).map(|i| {
            let mut q = t.parent[i];
            while q != NONE && !inset[q as usize] {
                q = t.parent[q as usize];
            }
            q
        }).collect()
    }
    pub fn outermost(t: &Tree, inset: &[bool]) -> Vec<u32> {
        (0..t.len()).map(|i| {
            let (mut q, mut best) = (t.parent[i], NONE);
            while q != NONE {
                if inset[q as usize] {
                    best = q;
                }
                q = t.parent[q as usize];
            }
            best
        }).collect()
    }
    pub fn has(t: &Tree, inset: &[bool]) -> Vec<bool> {
        (0..t.len()).map(|i| (i + 1..i + t.size[i] as usize).any(|j| inset[j])).collect()
    }
}
