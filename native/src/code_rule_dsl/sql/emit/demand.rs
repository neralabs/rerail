//! Conservative capture restrictions before candidate allocation. A text hash is a necessary
//! condition only; native unification remains the final check, including hash collisions.

use super::{Emitter, Op, Reads};
use crate::code_rule_dsl::sql::plan::{Need, SlotId, SlotKind, Stop, Unit, Walk};
use std::collections::{BTreeMap, BTreeSet};

fn required_capture(op: &Op, units: &[Unit], visiting: &mut BTreeSet<usize>) -> Option<SlotId> {
    match op {
        Op::Pattern { single_capture, .. } => *single_capture,
        Op::All(items) => {
            // An earlier binding or semantic operation can change what a later pattern reads.
            // Only capture-free deterministic prefixes are crossed by this initial proof.
            for item in items {
                if let Some(slot) = required_capture(item, units, visiting) {
                    return Some(slot);
                }
                if !matches!(item, Op::Kinds(_) | Op::Regex(_) | Op::Never) {
                    break;
                }
            }
            None
        }
        Op::AllOpen(terms) => {
            for term in terms {
                if let Some(slot) = required_capture(&term.op, units, visiting) {
                    return Some(slot);
                }
                if !matches!(term.op, Op::Kinds(_) | Op::Regex(_) | Op::Never) {
                    break;
                }
            }
            None
        }
        Op::AnyOpen(branches) => {
            let mut slots = branches
                .iter()
                .map(|(branch, _)| required_capture(branch, units, visiting));
            let first = slots.next()??;
            slots.all(|slot| slot == Some(first)).then_some(first)
        }
        Op::Any(items) => {
            let mut slots = items
                .iter()
                .map(|item| required_capture(item, units, visiting));
            let first = slots.next()??;
            slots.all(|slot| slot == Some(first)).then_some(first)
        }
        Op::Recur { unit, slots, .. } if visiting.len() < 64 && visiting.insert(*unit) => {
            let slot = required_capture(&units[*unit].body, units, visiting);
            visiting.remove(unit);
            slot.and_then(|slot| match slots {
                Some(site) => units[*unit]
                    .key
                    .iter()
                    .position(|&key| key == slot)
                    .map(|index| site[index]),
                None => Some(slot),
            })
        }
        _ => None,
    }
}

impl Emitter<'_> {
    pub(super) fn capture_need(&self, need: &Need, sub: &Op) -> Need {
        let mut narrowed = need.clone();
        // A guard must execute before any unknown-producing or binding prefix. A mismatched
        // bound leaf therefore makes the body false, never unknown or withheld. Each operator
        // starts its own verdict/mask, so incoming conjunction metadata is not a prefix here.
        narrowed.capture = required_capture(sub, self.units, &mut BTreeSet::new());
        narrowed
    }

    /// Demand a recursive key only if a mandatory leaf witness can occur in its subtree.
    /// This retains occurrence keys: semantic atoms are not substituted into general memo keys.
    pub(super) fn supported_demand(
        &mut self,
        unit: usize,
        site: Option<&[SlotId]>,
        input: &str,
    ) -> Option<String> {
        let key = &self.units[unit].key;
        let proof = key
            .iter()
            .enumerate()
            .filter(|(_, slot)| self.slots[**slot].kind == SlotKind::Node)
            .find_map(|(index, &slot)| {
                let kinds = subtree_support(
                    &self.units[unit].body,
                    slot,
                    None,
                    0,
                    self.units,
                    &mut BTreeMap::new(),
                )?;
                (!kinds.is_empty()).then(|| (site.map_or(slot, |site| site[index]), kinds))
            });
        let (slot, kinds) = proof?;
        let kinds = kinds
            .into_iter()
            .map(|kind| kind.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let output = self.frame("ds", &format!(
            "SELECT {} FROM {input} f LEFT JOIN node needle ON needle.file = f.file AND needle.id = f.s{slot} AND needle.leaf = 1 \
             CROSS JOIN node anchor ON anchor.file = f.file AND anchor.id = f.node \
             WHERE needle.id IS NULL OR EXISTS (SELECT 1 FROM node hit WHERE hit.file = f.file \
             AND hit.id >= f.node AND hit.id <= anchor.\"last\" AND hit.kind IN ({kinds}) AND hit.thash = needle.thash)",
            self.cols("f")), Reads::join(&[input]));
        self.inherit(input, &output, &BTreeSet::new());
        self.same_space(input, &output);
        Some(output)
    }
}

/// A positive, strictly descending recursive proof. Every successful or unknown evaluation needs
/// an equal-text witness of the returned kinds in the node's subtree. A revisited unit contributes
/// no new base case only after a strict descent; same-node cycles and other axes are barriers.
/// Kind restrictions are required so lookups use bounded value postings, not all-node hashes.
fn subtree_support(
    op: &Op,
    slot: SlotId,
    allowed: Option<BTreeSet<u16>>,
    depth: usize,
    units: &[Unit],
    visiting: &mut BTreeMap<usize, usize>,
) -> Option<BTreeSet<u16>> {
    if visiting.len() > 64 {
        return None;
    }
    match op {
        Op::Pattern {
            single_capture: Some(read),
            ..
        } if *read == slot => allowed,
        Op::All(items) => support_all(items.iter(), slot, allowed, depth, units, visiting),
        Op::AllOpen(terms) => support_all(
            terms.iter().map(|term| &term.op),
            slot,
            allowed,
            depth,
            units,
            visiting,
        ),
        Op::Any(items) => support_any(items.iter(), slot, allowed, depth, units, visiting),
        Op::AnyOpen(branches) => support_any(
            branches.iter().map(|(branch, _)| branch),
            slot,
            allowed,
            depth,
            units,
            visiting,
        ),
        Op::Relation {
            walk: Walk::Has(Stop::End | Stop::Neighbor),
            sub,
            ..
        }
        | Op::Search {
            walk: Walk::Has(Stop::End | Stop::Neighbor),
            sub,
            ..
        } => subtree_support(sub, slot, None, depth + 1, units, visiting),
        Op::Recur {
            unit, slots: None, ..
        } => {
            if let Some(previous) = visiting.get(unit) {
                return (depth > *previous).then(BTreeSet::new);
            }
            visiting.insert(*unit, depth);
            let proof = subtree_support(&units[*unit].body, slot, allowed, depth, units, visiting);
            visiting.remove(unit);
            proof
        }
        Op::Constrained { rule, .. } => {
            subtree_support(rule, slot, allowed, depth, units, visiting)
        }
        Op::Never => Some(BTreeSet::new()),
        _ => None,
    }
}

fn support_all<'a>(
    items: impl Iterator<Item = &'a Op>,
    slot: SlotId,
    mut allowed: Option<BTreeSet<u16>>,
    depth: usize,
    units: &[Unit],
    visiting: &mut BTreeMap<usize, usize>,
) -> Option<BTreeSet<u16>> {
    for item in items {
        if let Op::Kinds(kinds) = item {
            let kinds: BTreeSet<_> = kinds.iter().copied().collect();
            allowed = Some(allowed.map_or_else(
                || kinds.clone(),
                |allowed| allowed.intersection(&kinds).copied().collect(),
            ));
        }
        if let Some(proof) = subtree_support(item, slot, allowed.clone(), depth, units, visiting) {
            return Some(proof);
        }
        if !matches!(item, Op::Kinds(_) | Op::Regex(_) | Op::Never) {
            return None;
        }
    }
    None
}

fn support_any<'a>(
    items: impl Iterator<Item = &'a Op>,
    slot: SlotId,
    allowed: Option<BTreeSet<u16>>,
    depth: usize,
    units: &[Unit],
    visiting: &mut BTreeMap<usize, usize>,
) -> Option<BTreeSet<u16>> {
    let mut union = BTreeSet::new();
    for item in items {
        union.extend(subtree_support(
            item,
            slot,
            allowed.clone(),
            depth,
            units,
            visiting,
        )?);
    }
    Some(union)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reference() -> Op {
        Op::Recur {
            unit: 0,
            slots: None,
            context: None,
        }
    }
    fn witness() -> Op {
        Op::All(vec![
            Op::Kinds(vec![7]),
            Op::Pattern {
                term: 0,
                env: true,
                vars: vec![0],
                reads: vec![0],
                single_capture: Some(0),
            },
        ])
    }
    fn proof(body: Op) -> Option<BTreeSet<u16>> {
        let units = vec![Unit {
            key: vec![0],
            body,
            scc: 0,
            following: None,
        }];
        subtree_support(&units[0].body, 0, None, 0, &units, &mut BTreeMap::new())
    }
    #[test]
    fn only_positive_strictly_descending_cycles_have_subtree_support() {
        let descend = Op::Relation {
            walk: Walk::Has(Stop::Neighbor),
            sub: Box::new(reference()),
            binds: true,
            need: Need::default(),
        };
        assert_eq!(
            proof(Op::Any(vec![witness(), descend])),
            Some(BTreeSet::from([7]))
        );
        assert_eq!(proof(Op::Any(vec![witness(), reference()])), None);
        assert_eq!(proof(Op::Not(Box::new(reference()))), None);
        let ascend = Op::Relation {
            walk: Walk::Inside(Stop::End),
            sub: Box::new(reference()),
            binds: true,
            need: Need::default(),
        };
        assert_eq!(proof(Op::Any(vec![witness(), ascend])), None);
        assert_eq!(proof(Op::Any(vec![witness(), Op::Kinds(vec![8])])), None);
    }
    #[test]
    #[ignore = "prints the real B020 utility IR and proof barriers"]
    fn inspect_b020_demand_support() {
        use crate::code_rule_dsl::sql::lower::ProgramBuilder;
        use crate::code_rule_dsl::{
            CodeRuleDslRuntime,
            test_support::{CAPABILITIES, grammar_matchers},
        };
        let compiled = CodeRuleDslRuntime::compile(
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../packs/python/rules/ruff-b020-loop-variable-overrides-iterator.json"
            ))
            .to_owned(),
            "json".to_owned(),
            CAPABILITIES.to_owned(),
        )
        .unwrap();
        let grammars = grammar_matchers(&compiled);
        let mut builder = ProgramBuilder::default();
        let statement = builder.clause_or_nothing(&grammars[0], None).unwrap();
        for (index, unit) in statement.units.iter().enumerate() {
            for &slot in &unit.key {
                let proof = subtree_support(
                    &unit.body,
                    slot,
                    None,
                    0,
                    &statement.units,
                    &mut BTreeMap::new(),
                );
                eprintln!(
                    "unit={index} slot={} support={proof:?} body={:?}",
                    statement.slots[slot].name, unit.body
                );
            }
        }
    }
}
