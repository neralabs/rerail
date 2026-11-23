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
    pub fn of(v: &Version) -> Shape {
        let mut s = Shape::default();
        for r in v.rules() {
            let head = v.rel_name(r.head().rel()).to_owned();
            let mut reads: BTreeSet<String> = BTreeSet::new();
            for l in r.body() {
                match l.view() {
                    Lit::Atom(a) => {
                        let rel = v.rel_name(a.rel());
                        if !a.neg() {
                            s.positive.insert(rel.to_owned());
                        }
                        reads.insert(rel.to_owned());
                    }
                    Lit::Agg { atom, .. } => {
                        let rel = v.rel_name(atom.rel());
                        if !atom.neg() {
                            s.positive.insert(rel.to_owned());
                        }
                        reads.insert(rel.to_owned());
                    }
                    _ => {}
                }
            }
            s.read.extend(reads.iter().cloned());
            s.reads_of.entry(head.clone()).or_default().extend(reads);
            s.made.insert(head);
        }
        s
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
