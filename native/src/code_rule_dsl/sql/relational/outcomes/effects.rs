//! Summaries are computed once from immediate children, including shared utilities.
use super::Op;

#[derive(Clone, Copy, Default)]
pub(super) struct Effects {
    pub reads_value: bool,
    pub uses_memo: bool,
    pub fallible: bool,
    pub walk_depth: usize,
    pub file_memo: bool,
    /// Native cache construction spends work that a utility answer must not replay.
    pub cache_fills: bool,
    /// Exports may refer to descendants, so a root-binding bitmask is insufficient.
    pub typed_exports: bool,
    pub contains_chain: bool,
    pub needs_peak: bool,
    pub sibling_cache: bool,
}
impl Effects {
    fn merge(&mut self, child: Self) {
        self.reads_value |= child.reads_value;
        self.uses_memo |= child.uses_memo;
        self.fallible |= child.fallible;
        self.walk_depth = self.walk_depth.max(child.walk_depth);
        self.file_memo |= child.file_memo;
        self.cache_fills |= child.cache_fills;
        self.typed_exports |= child.typed_exports;
        self.contains_chain |= child.contains_chain;
        self.needs_peak |= child.needs_peak;
        self.sibling_cache |= child.sibling_cache;
    }
    pub fn of(op: &Op) -> Self {
        let mut result = Self::default();
        match op {
            Op::MixedRecursive { program, .. } => {
                for region in &program.regions {
                    result.merge(region.effects);
                }
                result.fallible = true;
                result.uses_memo = true;
                result.file_memo = true;
                result.needs_peak = true;
                result.contains_chain = true;
                result.typed_exports = true;
            }
            Op::NthChild {
                of_rule: Some(child),
                memoize,
                ..
            } => {
                result = child.effects;
                result.fallible = true;
                result.cache_fills |= *memoize;
                result.sibling_cache |= *memoize;
            }
            Op::FormatArgs {
                unused, missing, ..
            } => {
                for child in [unused, missing].into_iter().flatten() {
                    result.merge(child.effects);
                }
                result.reads_value = true;
                result.fallible = true;
                result.typed_exports = true;
                result.file_memo |= result.uses_memo;
                result.walk_depth += 1;
            }
            Op::CallFrame { body, .. } => {
                result = body.effects;
                result.reads_value = true;
                result.uses_memo = true;
                result.file_memo = true;
                result.fallible = true;
                result.typed_exports = true;
                result.needs_peak |= result.contains_chain;
            }
            Op::Parse { child, .. } => {
                result = child.effects;
                result.reads_value = true;
                result.fallible = true;
                result.walk_depth += 1;
                result.file_memo |= child.uses_memo();
                result.typed_exports = true;
            }
            Op::Host { child, .. } => {
                result = child.effects;
                result.typed_exports = true;
                result.file_memo |= child.uses_memo();
            }
            Op::Pattern(..) | Op::CaptureBind(_) => result.typed_exports = true,
            Op::FieldNeighbor { child, .. } | Op::FieldEnd { child, .. } => {
                result = child.effects;
                result.walk_depth += 1;
                result.typed_exports = true;
                result.file_memo |= child.uses_memo();
            }
            Op::Constrained(core, constraints) => {
                result = core.effects;
                // This transaction is opaque to local shared-call counting. Its core can
                // share an answer with the caller, so preserve that lifetime in keyed storage.
                result.file_memo |= core.uses_memo();
                for (_, child) in constraints {
                    result.merge(child.effects);
                    // A constraint changes the current node, so local fixed-node memo keys
                    // cannot share its utility answers with the caller.
                    result.file_memo |= child.uses_memo();
                }
                result.typed_exports = true;
            }
            Op::Value(_) => {
                result.reads_value = true;
                result.fallible = true;
            }
            Op::RecursiveSearch { component, .. } => {
                result = component.base.effects;
                result.needs_peak |= component.memo;
                result.uses_memo |= component.memo;
                result.file_memo |= component.memo;
                result.fallible = true;
                result.contains_chain = true;
                result.typed_exports = true;
            }
            Op::PureRecursive(_) => {
                result.fallible = true;
                result.contains_chain = true;
                result.needs_peak = true;
            }
            Op::Chain(..) | Op::Downward(..) => {
                result.fallible = true;
                result.contains_chain = true;
            }
            Op::Not(child) => result = child.effects,
            Op::TransientCharge(child) => {
                result = child.effects;
                result.fallible = true;
            }
            Op::Charge(_, child) => {
                result = child.effects;
                result.fallible = true;
                result.uses_memo = child.reads_value();
                result.needs_peak |= result.uses_memo && result.contains_chain;
                // Only the keyed memo path measures per-invocation cache-fill deltas.
                result.file_memo |= result.uses_memo && result.cache_fills;
            }
            Op::All(items) | Op::Any(items) => {
                for child in items {
                    result.merge(child.effects);
                }
            }
            Op::Search(_, _, child, stop)
            | Op::Count {
                rule: child, stop, ..
            } => {
                result = child.effects;
                if let Some(stop) = stop {
                    result.merge(stop.effects);
                }
                result.walk_depth += 1;
                result.file_memo |= result.uses_memo;
                result.fallible |= matches!(op, Op::Count { .. });
            }
            _ => {}
        }
        result
    }
}
