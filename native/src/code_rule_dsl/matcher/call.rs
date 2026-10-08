//! Parameterized utility calls and the `calls` predicate.

use super::{UtilCall, each, spend_util_step, unify_capture};
use ast_grep_core::meta_var::MetaVarEnv;
use ast_grep_core::{Doc, Node};
use std::collections::BTreeSet;

/// `calls` at `node`: `None` where unknown. A reaching call binds the term's `chain` capture.
pub(super) fn calls<'t, D: Doc>(
    test: &crate::code_rule_dsl::calls::CallsTest,
    node: &Node<'t, D>,
    env: &mut MetaVarEnv<'t, D>,
) -> Option<bool> {
    use crate::code_rule_dsl::calls::{Reach, holds};
    match holds(test, node, super::calls_tables) {
        Reach::Reaches(chain) => {
            if let (Some(name), Some(chain)) = (&test.chain, chain) {
                super::export_text(env, name, &chain, node.range());
            }
            Some(true)
        }
        Reach::No => Some(false),
        Reach::Unknown => None,
    }
}

/// A parameterized call at `node`: within the utility depth, stack and step limits, the caller's
/// captures are copied into a frame (`frame` starts it: the caller's environment without captures),
/// `body` matches the utility in it with `violation.each` framed, and what the utility bound to a
/// parameter the caller left unbound comes back under the caller's name, unified with the other
/// parameters mapped to that name (`unify_capture`): conflicting outputs fail the call, and of
/// equal ones the first in the source is bound, whatever order the parameters are declared or
/// mapped in. `body` returns the frame after the match, or `Err(unknown)` where it did not match.
/// `Ok(None)`: no match (a limit hit is left in `UTIL_EXCEEDED`); `Err(())`: the utility's verdict
/// is unknown.
pub(super) fn call<'t, D: Doc>(
    call: &UtilCall,
    env: &MetaVarEnv<'t, D>,
    frame: MetaVarEnv<'t, D>,
    body: impl FnOnce(MetaVarEnv<'t, D>) -> Result<Option<MetaVarEnv<'t, D>>, ()>,
) -> Result<Option<MetaVarEnv<'t, D>>, ()> {
    let Ok(depth) = super::enter_util_level() else {
        return Ok(None);
    };
    let matched = (|| {
        if !spend_util_step() {
            return Ok(None);
        }
        let mut frame = frame;
        for (param, caller) in &call.args {
            if unify_capture(env, std::iter::once(caller.as_str()), &mut frame, param).is_none() {
                return Ok(None);
            }
        }
        each::framed(&call.args, || body(frame))
    })();
    super::leave_util_level(depth);
    let Some(frame) = matched? else {
        return Ok(None);
    };
    let mut env = env.clone();
    let returned = |(param, caller): &(String, String)| {
        frame.get_match(param).is_some() && env.get_match(caller).is_none()
            || !frame.get_multiple_matches(param).is_empty()
                && env.get_multiple_matches(caller).is_empty()
            || frame.get_transformed(param).is_some() && env.get_transformed(caller).is_none()
    };
    if call.args.iter().any(returned) {
        let mut unified = BTreeSet::new();
        for (_, caller) in &call.args {
            if !unified.insert(caller.as_str()) {
                continue;
            }
            let params = call
                .args
                .iter()
                .filter(|(_, mapped)| mapped == caller)
                .map(|(param, _)| param.as_str());
            if unify_capture(&frame, params, &mut env, caller).is_none() {
                return Ok(None);
            }
        }
    }
    Ok(Some(env))
}

#[cfg(test)]
mod tests {
    use crate::code_rule_dsl::test_support::*;
    use serde_json::json;

    #[test]
    fn outputs_mapped_to_one_capture_unify_in_any_parameter_order() {
        // `P` and `Q` both return to `X`: `f(g)` binds them to different nodes, so the call fails
        // whichever parameter is declared or mapped first; `h(h)` binds equal ones.
        let source = "f(g);\nh(h);\n";
        for (params, mapping) in [
            (json!(["P", "Q"]), json!({ "P": "$X", "Q": "$X" })),
            (json!(["Q", "P"]), json!({ "Q": "$X", "P": "$X" })),
        ] {
            let rule = compiled(json!({
                "rule": { "kind": "call_expression", "matches": { "pair": mapping } },
                "utils": { "pair": { "params": params, "rule": { "pattern": "$P($Q)" } } },
            }));
            let result = evaluate_file(&rule, "typescript", "a.ts", source);
            assert_eq!(matched_texts(&result, source), ["h(h) X=h"], "{params}");
            // Of the equal outputs, `X` is the one first in the source (the callee).
            let capture = &result["violations"][0]["captures"]["X"][0];
            assert_eq!(capture["startByte"], json!(6), "{params}: {result}");
        }
    }
}
