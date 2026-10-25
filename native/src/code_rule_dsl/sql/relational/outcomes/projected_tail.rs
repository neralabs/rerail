//! A pure base followed by a kind-guarded, immediate-child self tail.
//! The proof uses source structure and utility identity, never rule names.
use super::super::{ir::Set, lower::Lowerer};
use super::utility_projection_lower;
use crate::code_rule_dsl::matcher::{AstMatcher, StopBy, UtilRef};
pub(super) struct Projection {
    pub base: utility_projection_lower::Program,
    pub guard: Set,
    pub filter: Option<Set>,
}
pub(super) fn admit(
    util: &UtilRef,
    lower: &mut Lowerer,
    three: bool,
    filter: Option<Set>,
) -> Option<Projection> {
    let AstMatcher::Any(mut arms) = util.with_target(Clone::clone)? else {
        return None;
    };
    let AstMatcher::All(tail) = arms.pop()? else {
        return None;
    };
    let [guard, AstMatcher::Has(child, StopBy::Neighbor)] = tail.as_slice() else {
        return None;
    };
    if !matches!(guard, AstMatcher::Kind(_)) {
        return None;
    }
    let AstMatcher::Util(target) = child.rule() else {
        return None;
    };
    if target.index != util.index || target.table.as_ptr() != util.table.as_ptr() {
        return None;
    }
    let base =
        utility_projection_lower::lower_ast(&AstMatcher::Any(arms), lower, three, |u| u.index)
            .ok()??;
    Some(Projection {
        base,
        guard: lower.set(guard).ok()?,
        filter,
    })
}

#[cfg(test)]
#[path = "projected_tail_tests.rs"]
mod tests;
