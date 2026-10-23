//! Test-only fault injection at the atomic value leaf, shared by native and SQL evaluation.
use ast_grep_core::{Doc, Node};
use std::cell::RefCell;
#[derive(Default)]
struct Probe {
    fault: Option<String>,
    calls: Vec<String>,
    reached_only: bool,
}
thread_local! { static PROBE: RefCell<Option<Probe>> = const { RefCell::new(None) }; }
pub(super) fn record<D: Doc>(node: &Node<'_, D>) -> bool {
    PROBE.with_borrow_mut(|probe| {
        let Some(probe) = probe else { return false };
        let text = node.text().into_owned();
        let fail = probe.fault.as_ref() == Some(&text);
        if !probe.reached_only || !super::util_limit_hit() {
            probe.calls.push(text);
        }
        fail
    })
}
pub(in crate::code_rule_dsl) fn run<T>(
    fault: Option<&str>,
    body: impl FnOnce() -> T,
) -> (T, Vec<String>) {
    run_inner(fault, false, body)
}
/// Calls through the first limit, excluding native cleanup probes after failure.
pub(in crate::code_rule_dsl) fn run_reached<T>(
    fault: Option<&str>,
    body: impl FnOnce() -> T,
) -> (T, Vec<String>) {
    run_inner(fault, true, body)
}
fn run_inner<T>(
    fault: Option<&str>,
    reached_only: bool,
    body: impl FnOnce() -> T,
) -> (T, Vec<String>) {
    struct Reset(Option<Probe>);
    impl Drop for Reset {
        fn drop(&mut self) {
            PROBE.with_borrow_mut(|p| *p = self.0.take());
        }
    }
    let _reset = Reset(PROBE.replace(Some(Probe {
        fault: fault.map(str::to_owned),
        calls: vec![],
        reached_only,
    })));
    let result = body();
    let calls = PROBE.with_borrow_mut(|p| std::mem::take(&mut p.as_mut().unwrap().calls));
    (result, calls)
}
