//! Summaries are computed once from immediate children, including shared utilities.
use super::Op;

#[derive(Clone, Copy, Default)]
pub(super) struct Effects {
    pub reads_value: bool,
    pub uses_memo: bool,
    pub fallible: bool,
    pub walk_depth: usize,
    pub file_memo: bool,
}
impl Effects {
    fn merge(&mut self, child: Self) {
        self.reads_value |= child.reads_value;
        self.uses_memo |= child.uses_memo;
        self.fallible |= child.fallible;
        self.walk_depth = self.walk_depth.max(child.walk_depth);
        self.file_memo |= child.file_memo;
    }
    pub fn of(op: &Op) -> Self {
        let mut result = Self::default();
        match op {
            Op::Value(_) => {
                result.reads_value = true;
                result.fallible = true;
            }
            Op::Not(child) => result = child.effects,
            Op::Charge(_, child) => {
                result = child.effects;
                result.fallible = true;
                result.uses_memo = child.reads_value();
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
