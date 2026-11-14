//! A program read from its image (`body::Placed::of_program`, decision 0173): its other fields beside its rules, by
//! name, as the tree passes read `Program`'s (`keeps::choose`, `demand::split_negations`; t36 item 6).

use super::super::program_image::{About, AtomId, NativeRef, RelId, Version};
use super::body::Placed;
use std::collections::BTreeSet;

/// A version's fields, read once.
pub struct Fields<'v> {
    pub v: &'v Version,
    pub about: About,
    outputs: BTreeSet<AtomId>,
    inputs: BTreeSet<AtomId>,
}

impl<'v> Fields<'v> {
    /// `p`'s fields: `None` where its version is its rules alone (`Placed::whole`).
    pub fn of(p: &'v Placed) -> Option<Fields<'v>> {
        if !p.whole {
            return None;
        }
        let v: &Version = &p.v;
        Some(Fields { v, about: About::of(v), outputs: v.outputs().collect(), inputs: v.inputs().collect() })
    }
    fn atom(&self, name: &str) -> Option<AtomId> {
        self.v.find(name)
    }
    /// `Program::qualified`.
    pub fn qualified(&self, rel: &str, word: &str) -> bool {
        self.atom(rel).zip(self.atom(word)).is_some_and(|(r, w)| self.about.qualified(r, w))
    }
    pub fn is_output(&self, rel: &str) -> bool {
        self.atom(rel).is_some_and(|a| self.outputs.contains(&a))
    }
    pub fn is_input(&self, rel: &str) -> bool {
        self.atom(rel).is_some_and(|a| self.inputs.contains(&a))
    }
    /// Whether `rel` has facts, one row or more.
    pub fn has_facts(&self, rel: &str) -> bool {
        self.atom(rel).and_then(|a| self.v.facts(a)).is_some_and(|f| f.rows().len() > 0)
    }
    /// Whether `rel` is demanded (`Program::demand`) or a gate (`Program::gates`' keys).
    pub fn demanded_or_gate(&self, rel: &str) -> bool {
        self.atom(rel).is_some_and(|a| self.about.demand.binary_search(&a).is_ok() || self.about.gate_keys.binary_search(&a).is_ok())
    }
    /// Whether `rel` is demanded (`Program::demand`).
    pub fn demanded(&self, rel: &str) -> bool {
        self.atom(rel).is_some_and(|a| self.about.demand.binary_search(&a).is_ok())
    }
    /// Whether `rel` is a gate (`Program::gates`' keys).
    pub fn gate(&self, rel: &str) -> bool {
        self.atom(rel).is_some_and(|a| self.about.gate_keys.binary_search(&a).is_ok())
    }
    fn rel(&self, rel: &str) -> Option<RelId> {
        self.v.rel(rel)
    }
    /// Whether `rel` is a request or a relation an answer produces (`Program::answers`).
    pub fn answer(&self, rel: &str) -> bool {
        self.rel(rel).is_some_and(|r| self.about.answers.iter().any(|&(q, p)| q == r || p == r))
    }
    pub fn nullary(&self, rel: &str) -> bool {
        self.rel(rel).is_some_and(|r| self.about.nullary.binary_search(&r).is_ok())
    }
    pub fn lattice(&self, rel: &str) -> bool {
        self.rel(rel).is_some_and(|r| self.about.lattice_decl(r).is_some())
    }
    pub fn native(&self, rel: &str) -> Option<&NativeRef> {
        self.rel(rel).and_then(|r| self.about.native(r))
    }
    /// The relations natives read (`Native::reads`), by name.
    pub fn natives_read(&self) -> BTreeSet<&'v str> {
        self.about.natives.iter().flat_map(|n| n.reads.iter()).map(|&r| self.v.rel_name(r)).collect()
    }
    /// `rel`'s declared columns' count, if it is declared.
    pub fn arity(&self, rel: &str) -> Option<usize> {
        self.v.decl_of(rel).map(|d| d.cols().len())
    }
}
