//! What the run reads of the combined program's rules before it prepares it (decision 0281's path, step 7a): the
//! relations rules make, the relations they read (positively, or at all, an aggregate's atom included), by head.
//! The readers before preparation (the bridged programs, the phases, the constants, the repository inputs, the
//! guests' requests, the world, the components) ask these of the version, and its fields (`ProgramFields`) for
//! everything else, so no rule is decoded.

use crate::code_rule_dsl::logic::program_image::{Lit, Version};
use std::collections::{BTreeMap, BTreeSet};

/// The combined program's rules, as the readers before preparation ask of them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shape {
    /// The relations some rule makes.
    pub made: BTreeSet<String>,
    /// The relations some rule's body reads (an atom, negated or not, or an aggregate's).
    pub read: BTreeSet<String>,
    /// The relations some rule's body reads positively (not negated: a plain atom, or an aggregate's).
    pub positive: BTreeSet<String>,
    /// Each made relation's reads, over its rules.
    pub reads_of: BTreeMap<String, BTreeSet<String>>,
}

impl Shape {
    /// One walk of the rules by relation id; each relation named once, at the end (named per atom, a set's string
    /// made and compared per atom, it was 16 ms of a warm run).
    pub fn of(v: &Version) -> Shape {
        use std::collections::{HashMap, HashSet};
        let (mut positive, mut reads_of): (HashSet<_>, HashMap<_, HashSet<_>>) = Default::default();
        for r in v.rules() {
            let reads = reads_of.entry(r.head().rel()).or_default();
            for l in r.body() {
                let atom = match l.view() {
                    Lit::Atom(a) => a,
                    Lit::Agg { atom, .. } => atom,
                    _ => continue,
                };
                if !atom.neg() {
                    positive.insert(atom.rel());
                }
                reads.insert(atom.rel());
            }
        }
        let names = |rels: &mut dyn Iterator<Item = &_>| -> BTreeSet<String> { rels.map(|&r| v.rel_name(r).to_owned()).collect() };
        let read: HashSet<_> = reads_of.values().flatten().copied().collect();
        Shape {
            made: names(&mut reads_of.keys()),
            read: names(&mut read.iter()),
            positive: names(&mut positive.iter()),
            reads_of: reads_of.iter().map(|(&h, rs)| (v.rel_name(h).to_owned(), names(&mut rs.iter()))).collect(),
        }
    }
}

/// An analysis of the version (`manager::analysis`): made once a version, held by it, and shared by the readers that
/// ask it of the same version (the constants' fold, the phases, the combine's): each made it anew, each a decode of
/// every rule (28 ms of a warm run's 0.36 s). Not kept for the store: it is asked before the run reads its kept plans
/// (whose entry is the prepared program's, made after it), so a warm run would make it and write it again.
impl crate::code_rule_dsl::logic::program_image::manager::Analysis for Shape {
    const NAME: &'static str = "corpus shape";
    const READS: crate::code_rule_dsl::logic::program_image::manager::Reads = crate::code_rule_dsl::logic::program_image::manager::Reads::RULES;
    fn compute(cx: &crate::code_rule_dsl::logic::program_image::manager::Cx<'_>) -> Result<Shape, String> {
        Ok(Shape::of(cx.v()))
    }
}
