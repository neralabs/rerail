//! Test-only incoming-level oracle. Synthetic logical level does not consume
//! native physical stack and cannot qualify physical stack-byte equivalence.
use super::{
    CLAUSE_STATE_READS, CLAUSES_BEGUN, FILL_STEPS, GrammarMatcher, THREE_VALUED, UNKNOWN,
    UTIL_DEPTH, UTIL_EXCEEDED, UTIL_PEAK, UTIL_STACK_BASE, UTIL_STEPS_LEFT, UtilLimit,
};
use crate::code_rule_dsl::tape::TapeDoc;
use ast_grep_core::Node;
#[derive(Debug)]
pub(in crate::code_rule_dsl) struct DepthAnswer {
    pub truth: i64,
    pub work: usize,
    /// Raw native utility memo peak. Started::now may seed incoming even
    /// when entry is rejected. Compare peak.saturating_sub(incoming).
    pub peak: u32,
    pub limit: Option<UtilLimit>,
}
struct Restore {
    depth: u32,
    exceeded: Option<UtilLimit>,
    stack: usize,
    work: usize,
    peak: u32,
    three: bool,
    unknown: bool,
    clauses: u64,
    fills: usize,
    reads: u64,
    owner: usize,
}
impl Restore {
    fn snapshot() -> Self {
        Self {
            depth: UTIL_DEPTH.get(),
            exceeded: UTIL_EXCEEDED.get(),
            stack: UTIL_STACK_BASE.get(),
            work: UTIL_STEPS_LEFT.get(),
            peak: UTIL_PEAK.get(),
            three: THREE_VALUED.get(),
            unknown: UNKNOWN.get(),
            clauses: CLAUSES_BEGUN.get(),
            fills: FILL_STEPS.get(),
            reads: CLAUSE_STATE_READS.get(),
            owner: super::resolve::CURRENT_CLAUSE.get(),
        }
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        UTIL_DEPTH.set(self.depth);
        UTIL_EXCEEDED.set(self.exceeded);
        UTIL_STACK_BASE.set(self.stack);
        UTIL_STEPS_LEFT.set(self.work);
        UTIL_PEAK.set(self.peak);
        THREE_VALUED.set(self.three);
        UNKNOWN.set(self.unknown);
        CLAUSES_BEGUN.set(self.clauses);
        FILL_STEPS.set(self.fills);
        CLAUSE_STATE_READS.set(self.reads);
        super::resolve::CURRENT_CLAUSE.set(self.owner);
    }
}
/// Runs the ORIGINAL matcher only from a Rust test. No SQL function registers
/// this evaluator. Memo/cache lifetime is owned by the caller's with_file_memo.
pub(in crate::code_rule_dsl) fn match_at_depth(
    grammar: &GrammarMatcher,
    node: Node<'_, TapeDoc>,
    incoming: u32,
    work: usize,
) -> DepthAnswer {
    assert!(incoming <= 256);
    assert!(
        grammar.transform.is_none(),
        "depth oracle excludes transforms"
    );
    let _restore = Restore::snapshot();
    super::begin_clause(grammar, work);
    UTIL_DEPTH.set(incoming);
    UTIL_PEAK.set(0);
    let matched = grammar.match_rule(node).is_some();
    let limit = UTIL_EXCEEDED.get();
    let truth = match limit {
        Some(UtilLimit::Depth) => -1,
        Some(UtilLimit::Work) => -2,
        Some(other) => panic!("depth oracle encountered an unrelated limit: {other:?}"),
        None if matched => 1,
        None if UNKNOWN.get() => 2,
        None => 0,
    };
    DepthAnswer {
        truth,
        work: UTIL_STEPS_LEFT.get(),
        peak: UTIL_PEAK.get(),
        limit,
    }
}
#[test]
fn depth_oracle_restores_every_owned_tls_on_unwind() {
    let before = Restore::snapshot();
    let result = std::panic::catch_unwind(|| {
        let _restore = Restore::snapshot();
        UTIL_DEPTH.set(252);
        UTIL_EXCEEDED.set(Some(UtilLimit::Depth));
        UTIL_STACK_BASE.set(17);
        UTIL_STEPS_LEFT.set(3);
        UTIL_PEAK.set(253);
        THREE_VALUED.set(!before.three);
        UNKNOWN.set(!before.unknown);
        CLAUSES_BEGUN.set(before.clauses + 1);
        FILL_STEPS.set(before.fills + 1);
        CLAUSE_STATE_READS.set(before.reads + 1);
        super::resolve::CURRENT_CLAUSE.set(19);
        panic!("prove panic restoration");
    });
    assert!(result.is_err());
    let after = Restore::snapshot();
    assert_eq!(
        (
            after.depth,
            after.exceeded,
            after.stack,
            after.work,
            after.peak
        ),
        (
            before.depth,
            before.exceeded,
            before.stack,
            before.work,
            before.peak
        )
    );
    assert_eq!(
        (
            after.three,
            after.unknown,
            after.clauses,
            after.fills,
            after.reads,
            after.owner
        ),
        (
            before.three,
            before.unknown,
            before.clauses,
            before.fills,
            before.reads,
            before.owner
        )
    );
}
