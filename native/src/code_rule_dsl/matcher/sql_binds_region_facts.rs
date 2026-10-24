//! Atomic binding-filter facts with explicit capture inputs and no child evaluation.
use super::*;

pub(in crate::code_rule_dsl) fn binds_region_budget<'tree, D: Doc>(
    test: &Binds,
    node: &Node<'tree, D>,
    env: &MetaVarEnv<'tree, D>,
    work: usize,
) -> (Result<Option<bool>, String>, usize) {
    if test.sub_rules().next().is_some() || test.redefines.is_some() {
        return (
            Err("binding region facts require a child-free predicate".into()),
            work,
        );
    }
    struct Reset(bool, bool, usize, Option<UtilLimit>);
    impl Drop for Reset {
        fn drop(&mut self) {
            THREE_VALUED.set(self.0);
            UNKNOWN.set(self.1);
            UTIL_STEPS_LEFT.set(self.2);
            UTIL_EXCEEDED.set(self.3);
        }
    }
    let _reset = Reset(
        THREE_VALUED.replace(true),
        UNKNOWN.replace(false),
        UTIL_STEPS_LEFT.replace(work),
        UTIL_EXCEEDED.replace(None),
    );
    let mut verdict = Some(false);
    if let Some(site) = test.at(node) {
        let mut unknown = false;
        for candidate in site.candidates(test, env) {
            if candidate.pre == PASSES {
                verdict = Some(true);
                break;
            }
            unknown |= candidate.pre == OPEN;
        }
        if verdict != Some(true) && (unknown || UNKNOWN.get()) {
            verdict = None;
        }
    }
    (
        match UTIL_EXCEEDED.get() {
            Some(limit) => Err(format!("{limit:?}")),
            None => Ok(verdict),
        },
        UTIL_STEPS_LEFT.get(),
    )
}
