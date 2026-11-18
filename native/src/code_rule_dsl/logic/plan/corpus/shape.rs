//! What the run reads of the combined program's rules before it prepares it (decision 0281's path, step 7a): the
//! relations rules make, the relations they read (positively, or at all, an aggregate's atom included), by head.
//! The readers before preparation (the bridged programs, the phases, the constants, the repository inputs, the
//! guests' requests, the world, the components) ask these of the version, and its header (`Version::header`) for
//! everything else, so no rule is decoded.

use crate::code_rule_dsl::logic::program_image::{Lit, Version};
use std::collections::{BTreeMap, BTreeSet};

/// The combined program's rules, as the readers before preparation ask of them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Shape {
    /// The relations some rule makes.
    pub(crate) made: BTreeSet<String>,
    /// The relations some rule's body reads (an atom, negated or not, or an aggregate's).
    pub(crate) read: BTreeSet<String>,
    /// The relations some rule's body reads positively (`Literal::atom` not negated: a plain atom, or an aggregate's).
    pub(crate) positive: BTreeSet<String>,
    /// Each made relation's reads, over its rules.
    pub(crate) reads_of: BTreeMap<String, BTreeSet<String>>,
}

impl Shape {
    pub(crate) fn of(v: &Version) -> Shape {
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

    /// The tree's, for a debug build's check and for programs the run has as trees (a phase's, a bridged one's).
    pub(crate) fn of_tree(prog: &crate::code_rule_dsl::logic::front::Program) -> Shape {
        use crate::code_rule_dsl::logic::front::Literal;
        let mut s = Shape::default();
        for r in &prog.rules {
            let mut reads: BTreeSet<String> = BTreeSet::new();
            for l in r.body.iter() {
                match l {
                    Literal::Atom(a) => {
                        if !a.neg {
                            s.positive.insert(a.rel.to_string());
                        }
                        reads.insert(a.rel.to_string());
                    }
                    Literal::Agg { atom, .. } => {
                        if !atom.neg {
                            s.positive.insert(atom.rel.to_string());
                        }
                        reads.insert(atom.rel.to_string());
                    }
                    _ => {}
                }
            }
            s.read.extend(reads.iter().cloned());
            s.reads_of.entry(r.head.rel.to_string()).or_default().extend(reads);
            s.made.insert(r.head.rel.to_string());
        }
        s
    }
}
