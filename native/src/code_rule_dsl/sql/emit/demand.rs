//! Conservative capture restrictions before candidate allocation. A text hash is a necessary
//! condition only; native unification remains the final check, including hash collisions.

use super::{Emitter, Op};
use crate::code_rule_dsl::sql::plan::{Need, SlotId, SlotKind, Unit};
use crate::code_rule_dsl::sql::plan::{Stop, Walk};
use std::collections::BTreeMap;
use std::collections::BTreeSet;

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
    pub(super) fn supported_demand(&self, unit: usize, site: Option<&[SlotId]>) -> Option<String> {
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
        Some(format!(
            "NOT EXISTS (SELECT 1 FROM node needle WHERE needle.file = f.file AND needle.id = f.s{slot} AND needle.leaf = 1 \
             AND NOT EXISTS (SELECT 1 FROM node anchor CROSS JOIN node hit ON hit.file = f.file \
             AND hit.id >= f.node AND hit.id <= anchor.\"last\" AND hit.kind IN ({kinds}) AND hit.thash = needle.thash \
             WHERE anchor.file = f.file AND anchor.id = f.node))"
        ))
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
    support(op, slot, allowed, depth, units, visiting, &mut 4096)
}

fn support(
    op: &Op,
    slot: SlotId,
    allowed: Option<BTreeSet<u16>>,
    depth: usize,
    units: &[Unit],
    visiting: &mut BTreeMap<usize, usize>,
    fuel: &mut usize,
) -> Option<BTreeSet<u16>> {
    *fuel = fuel.checked_sub(1)?;
    if visiting.len() > 64 {
        return None;
    }
    match op {
        Op::Pattern {
            single_capture: Some(read),
            ..
        } if *read == slot => allowed,
        Op::All(items) => support_all(items.iter(), slot, allowed, depth, units, visiting, fuel),
        Op::AllOpen(terms) => support_all(
            terms.iter().map(|term| &term.op),
            slot,
            allowed,
            depth,
            units,
            visiting,
            fuel,
        ),
        Op::Any(items) => support_any(items.iter(), slot, allowed, depth, units, visiting, fuel),
        Op::AnyOpen(branches) => support_any(
            branches.iter().map(|(branch, _)| branch),
            slot,
            allowed,
            depth,
            units,
            visiting,
            fuel,
        ),
        Op::Relation {
            walk: Walk::Has(_),
            sub,
            ..
        }
        | Op::Search {
            walk: Walk::Has(Stop::End | Stop::Neighbor),
            sub,
            ..
        } => support(sub, slot, None, depth + 1, units, visiting, fuel),
        Op::Recur {
            unit, slots: None, ..
        } => {
            if let Some(previous) = visiting.get(unit) {
                return (depth > *previous).then(BTreeSet::new);
            }
            visiting.insert(*unit, depth);
            let proof = support(
                &units[*unit].body,
                slot,
                allowed,
                depth,
                units,
                visiting,
                fuel,
            );
            visiting.remove(unit);
            proof
        }
        Op::Constrained { rule, .. } => support(rule, slot, allowed, depth, units, visiting, fuel),
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
    fuel: &mut usize,
) -> Option<BTreeSet<u16>> {
    let items: Vec<_> = items.collect();
    // Every direct kind test is mandatory in a conjunction, even after the capture pattern.
    // The node stays fixed; this does not commute bindings or unknown-producing terms.
    for item in &items {
        if let Op::Kinds(kinds) = item {
            let kinds: BTreeSet<_> = kinds.iter().copied().collect();
            allowed = Some(allowed.map_or_else(
                || kinds.clone(),
                |allowed| allowed.intersection(&kinds).copied().collect(),
            ));
        }
    }
    for item in items {
        if let Op::Kinds(kinds) = item {
            let kinds: BTreeSet<_> = kinds.iter().copied().collect();
            allowed = Some(allowed.map_or_else(
                || kinds.clone(),
                |allowed| allowed.intersection(&kinds).copied().collect(),
            ));
        }
        if let Some(proof) = support(item, slot, allowed.clone(), depth, units, visiting, fuel) {
            return Some(proof);
        }
        let pure = matches!(item, Op::Kinds(_) | Op::Regex(_) | Op::Never)
            || matches!(item, Op::Not(inner) if matches!(&**inner, Op::Kinds(_)));
        if !pure {
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
    fuel: &mut usize,
) -> Option<BTreeSet<u16>> {
    let mut union = BTreeSet::new();
    for item in items {
        union.extend(support(
            item,
            slot,
            allowed.clone(),
            depth,
            units,
            visiting,
            fuel,
        )?);
    }
    Some(union)
}

/// Every read of this leaf-valued input uses unification text. With `exports`, successful
/// returns must also preserve the caller occurrence. Negation/count discard inner bindings.
fn value_only(
    op: &Op,
    slot: SlotId,
    exports: bool,
    units: &[Unit],
    visiting: &mut BTreeSet<(usize, SlotId, bool)>,
    fuel: &mut usize,
) -> bool {
    let Some(left) = fuel.checked_sub(1) else {
        return false;
    };
    *fuel = left;
    match op {
        Op::Kinds(_) | Op::Regex(_) | Op::Never => true,
        Op::Pattern { vars, .. } => !exports || !vars.contains(&slot),
        Op::All(items) | Op::Any(items) | Op::Seq(items) => items
            .iter()
            .all(|op| value_only(op, slot, exports, units, visiting, fuel)),
        Op::AllOpen(items) => items
            .iter()
            .all(|t| value_only(&t.op, slot, exports, units, visiting, fuel)),
        Op::AnyOpen(items) => items
            .iter()
            .all(|(op, _)| value_only(op, slot, exports, units, visiting, fuel)),
        Op::Not(inner) => value_only(inner, slot, false, units, visiting, fuel),
        Op::Unknowable { op, .. } => value_only(op, slot, exports, units, visiting, fuel),
        Op::Relation { walk, sub, .. } | Op::Search { walk, sub, .. } | Op::Every { walk, sub } => {
            !matches!(walk, Walk::Slot(_) | Walk::Related(_))
                && value_only(sub, slot, exports, units, visiting, fuel)
        }
        Op::Count {
            walk, sub, bind, ..
        } => {
            !matches!(walk, Walk::Slot(_) | Walk::Related(_))
                && bind.is_none_or(|(bound, _)| !exports || bound != slot)
                && value_only(sub, slot, false, units, visiting, fuel)
        }
        Op::Fact { reads, binds, .. } => {
            !reads.contains(&slot) && (!exports || !binds.contains(&slot))
        }
        Op::Recur {
            unit,
            slots,
            context,
        } => {
            if *context == Some(slot) || units[*unit].following == Some(slot) {
                return false;
            }
            let callee = match slots {
                None => slot,
                Some(site) => match site.iter().position(|&s| s == slot) {
                    None => return true,
                    Some(i) => units[*unit].key[i],
                },
            };
            let key = (*unit, callee, exports);
            if visiting.contains(&key) {
                return true;
            }
            visiting.insert(key);
            let result = value_only(&units[*unit].body, callee, exports, units, visiting, fuel);
            visiting.remove(&key);
            result
        }
        // These operators may inspect occurrence, scope, exclusion, or exported source spans.
        _ => false,
    }
}
/// Every successful path replaces this input with a candidate occurrence. Unknown answers
/// still preserve the caller. A recursive success must eventually take a finite base path.
fn rebinds(
    op: &Op,
    slot: SlotId,
    units: &[Unit],
    visiting: &mut BTreeSet<(usize, SlotId)>,
    fuel: &mut usize,
) -> bool {
    let Some(left) = fuel.checked_sub(1) else {
        return false;
    };
    *fuel = left;
    match op {
        Op::Pattern {
            single_capture: Some(bound),
            ..
        } => *bound == slot,
        Op::Never => true,
        Op::All(items) | Op::Seq(items) => items
            .iter()
            .any(|op| rebinds(op, slot, units, visiting, fuel)),
        Op::AllOpen(items) => items
            .iter()
            .any(|t| rebinds(&t.op, slot, units, visiting, fuel)),
        Op::Any(items) => items
            .iter()
            .all(|op| rebinds(op, slot, units, visiting, fuel)),
        Op::AnyOpen(items) => items
            .iter()
            .all(|(op, _)| rebinds(op, slot, units, visiting, fuel)),
        Op::Relation { sub, .. } | Op::Search { sub, .. } => {
            rebinds(sub, slot, units, visiting, fuel)
        }
        Op::Unknowable { op, .. } => rebinds(op, slot, units, visiting, fuel),
        Op::Recur { unit, slots, .. } => {
            let callee = match slots {
                None => slot,
                Some(site) => match site.iter().position(|&s| s == slot) {
                    Some(i) => units[*unit].key[i],
                    None => return false,
                },
            };
            if !units[*unit].key.contains(&callee) {
                return false;
            }
            let key = (*unit, callee);
            if visiting.contains(&key) {
                return true;
            }
            visiting.insert(key);
            let result = rebinds(&units[*unit].body, callee, units, visiting, fuel);
            visiting.remove(&key);
            result
        }
        _ => false,
    }
}
impl Emitter<'_> {
    /// Value equivalence with a fixed return contract: false preserves the caller, true
    /// returns a newly bound candidate. Mixed paths retain occurrence keys.
    pub(super) fn value_keys(&self, unit: usize) -> BTreeMap<SlotId, bool> {
        self.units[unit]
            .key
            .iter()
            .copied()
            .filter_map(|slot| {
                if self.slots[slot].kind != SlotKind::Node {
                    return None;
                }
                let body = &self.units[unit].body;
                if value_only(
                    body,
                    slot,
                    true,
                    self.units,
                    &mut BTreeSet::new(),
                    &mut 4096,
                ) {
                    return Some((slot, false));
                }
                (value_only(
                    body,
                    slot,
                    false,
                    self.units,
                    &mut BTreeSet::new(),
                    &mut 4096,
                ) && rebinds(body, slot, self.units, &mut BTreeSet::new(), &mut 4096))
                .then_some((slot, true))
            })
            .collect()
    }
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
            Op::Pattern {
                term: 0,
                env: true,
                vars: vec![0],
                reads: vec![0],
                single_capture: Some(0),
            },
            Op::Kinds(vec![7]),
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
    fn value_keys_distinguish_uniform_rebinding_from_mixed_return_paths() {
        let descent = || Op::Relation {
            walk: Walk::Has(Stop::Neighbor),
            sub: Box::new(reference()),
            binds: true,
            need: Need::default(),
        };
        let definite = |body| {
            let units = [Unit {
                key: vec![0],
                body,
                scc: 0,
                following: None,
            }];
            rebinds(&units[0].body, 0, &units, &mut BTreeSet::new(), &mut 4096)
        };
        assert!(definite(Op::Any(vec![witness(), descent()])));
        assert!(!definite(Op::Any(vec![witness(), Op::Kinds(vec![7])])));
        assert!(!definite(Op::Not(Box::new(witness()))));
        assert!(!definite(Op::Pattern {
            term: 0,
            env: true,
            vars: vec![0],
            reads: vec![0],
            single_capture: None
        }));
    }

    #[test]
    fn semantic_key_proof_rejects_occurrence_reads_and_exported_bindings() {
        let allowed = |body| {
            let units = [Unit {
                key: vec![0],
                body,
                scc: 0,
                following: None,
            }];
            value_only(
                &units[0].body,
                0,
                true,
                &units,
                &mut BTreeSet::new(),
                &mut 4096,
            )
        };
        assert!(!allowed(witness()));
        assert!(allowed(Op::Not(Box::new(witness()))));
        assert!(allowed(Op::All(vec![
            Op::Not(Box::new(witness())),
            reference()
        ])));
        assert!(!allowed(Op::All(vec![witness(), reference()])));
        assert!(!allowed(Op::Fact {
            term: 0,
            reads: vec![0],
            at_node: true,
            binds: vec![]
        }));
        assert!(!allowed(Op::Relation {
            walk: Walk::Slot(0),
            sub: Box::new(Op::Kinds(vec![7])),
            need: Need::default(),
            binds: false
        }));
        assert!(!allowed(Op::Constrained {
            rule: Box::new(Op::Kinds(vec![7])),
            constraints: vec![(0, Op::Kinds(vec![7]))]
        }));
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
        assert_eq!(proof(Op::Any((0..5000).map(|_| witness()).collect())), None);
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
