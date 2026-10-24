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
    }
    pub fn of(op: &Op) -> Self {
        let mut result = Self::default();
        match op {
            Op::Pattern(..) => result.typed_exports = true,
            Op::FieldNeighbor { child, .. } => {
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
            Op::Not(child) => result = child.effects,
            Op::Charge(_, child) => {
                result = child.effects;
                result.fallible = true;
                result.uses_memo = child.reads_value();
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
