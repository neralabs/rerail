//! Finite-work evaluation of one atomic matcher with explicit capture inputs.
use super::*;

#[allow(clippy::type_complexity)]
pub(in crate::code_rule_dsl) fn leaf_env_budget<'tree, D: Doc>(
    leaf: &AstMatcher,
    node: Node<'tree, D>,
    env: &MetaVarEnv<'tree, D>,
    work: usize,
) -> (
    Result<(Option<bool>, Option<MetaVarEnv<'tree, D>>), UtilLimit>,
    usize,
) {
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
    let mut trial = Cow::Borrowed(env);
    let matched = leaf.match_node_with_env(node, &mut trial).is_some();
    let verdict = if matched {
        Some(true)
    } else if UNKNOWN.get() {
        None
    } else {
        Some(false)
    };
    let result = match UTIL_EXCEEDED.get() {
        Some(limit) => Err(limit),
        None => Ok((verdict, matched.then(|| trial.into_owned()))),
    };
    (result, UTIL_STEPS_LEFT.get())
}
