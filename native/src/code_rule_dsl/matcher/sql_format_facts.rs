//! Budgeted format analysis only. Candidate sub-rules remain SQL operators.
use super::*;

pub(in crate::code_rule_dsl) fn sql_format_budget<'t, D: Doc>(
    test: &FormatArgs,
    env: &MetaVarEnv<'t, D>,
    work: usize,
) -> (Result<Prepared<'t, D>, String>, usize) {
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
    let prepared = test.prepare(env);
    let result = match UTIL_EXCEEDED.get() {
        Some(limit) => Err(format!("{limit:?}")),
        None => Ok(prepared),
    };
    (result, UTIL_STEPS_LEFT.get())
}
