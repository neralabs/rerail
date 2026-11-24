//! A clause's steps: the limits its `matches` recursion has, the budget its utilities, counts and value decodes
//! spend (`maxVisitedNodes`), what cache fills and reads of a clause's own state it noted, and the matcher's Unknown
//! flag. Thread-local, set up by `begin_clause` (`matcher`). Below the matcher: the file context names resolve in
//! (`resolve`) and the language packs' argument slots (`arguments`) spend and read them too.

use crate::code_rule_dsl::budget;
use std::cell::Cell;

/// `matches` recursion deeper than this fails the clause for the file instead of risking the stack.
/// Utilities recurse only through relations, so real rules recurse as deep as the syntax they walk.
pub const MAX_UTIL_DEPTH: u32 = 256;

/// Stack a root's `matches` recursion may use, measured from `begin_clause`. A level's frames grow
/// with how much rule it nests and with the build (a small recursive rule takes about 2 KiB per
/// level in release, 15 KiB in debug), so bytes back up the level count. Every thread matching runs
/// on has room for this on top of its callers: Node's main thread and libuv workers (8 MiB) and
/// Rust test threads (2 MiB).
pub const MAX_UTIL_STACK_BYTES: usize = 1 << 20;

thread_local! {
    pub static UTIL_DEPTH: Cell<u32> = const { Cell::new(0) };
    /// The stack address `begin_clause` started at; 0 outside one (no stack limit applies).
    pub static UTIL_STACK_BASE: Cell<usize> = const { Cell::new(0) };
    pub static UTIL_STEPS_LEFT: Cell<usize> = const { Cell::new(usize::MAX) };
    pub static UTIL_EXCEEDED: Cell<Option<UtilLimit>> = const { Cell::new(None) };
    /// The matcher's Unknown flag: set when a rule failed because a verdict it read was unknown
    /// rather than false (a value it could not decide). A matcher that returns a match leaves it
    /// as it found it; one that fails leaves it set if it was set on entry or its own failure is
    /// unknown. Readers (`not`, relations, `count`) clear it before the sub-rule they read and
    /// restore it after.
    pub static UNKNOWN: Cell<bool> = const { Cell::new(false) };
    /// Whether the root being matched can reach an unknown verdict (it reads values). Without one,
    /// no matcher touches `UNKNOWN`, so rules that read no values match exactly as they did.
    pub static THREE_VALUED: Cell<bool> = const { Cell::new(false) };
}

/// An address in the current stack frame.
#[inline(always)]
pub fn stack_address() -> usize {
    let marker = 0u8;
    std::hint::black_box(&marker) as *const u8 as usize
}

/// A limit `matches` recursion hit. The root's verdict is then unknown (a `not` above the cut-off
/// reference would read its failure as a non-match), so the clause fails for the file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UtilLimit {
    /// `MAX_UTIL_DEPTH` or `MAX_UTIL_STACK_BYTES`.
    Depth,
    /// More utility evaluations and counted nodes than the query's `maxVisitedNodes` in one file.
    Work,
    /// The file's scope table (`resolves`) could not be built within `maxVisitedNodes`, the time
    /// budget, or its nesting limit.
    Scope,
    /// Decoding values (`value`, `compare` over values) spent the rest of `maxVisitedNodes`.
    Value,
    /// The parser stopped at a limit (Forte's nesting depth or step budget) before it read the
    /// file: no clause reads it.
    Parse,
}

impl UtilLimit {
    pub fn message(self) -> &'static str {
        match self {
            Self::Depth => {
                "`matches` recursion exceeded its depth limit (256 nested utility rules or 1 MiB of stack)"
            }
            Self::Work => {
                "`matches` and `count` exceeded their work budget (maxVisitedNodes utility evaluations and counted nodes per file)"
            }
            Self::Scope => {
                "`resolves` could not build the file's scope table within its budget (maxVisitedNodes nodes, the time budget, or 2,000 nesting levels)"
            }
            Self::Value => {
                "decoding values exceeded the work budget (maxVisitedNodes decoded nodes, with utility evaluations and counted nodes, per file)"
            }
            Self::Parse => {
                "the parser stopped at a limit (its nesting depth or step budget) before it read the file"
            }
        }
    }
}

/// Spends one step of the root's utility budget for a decode; false once it or the deadline ran
/// out (the budget then fails the clause for the file with `UtilLimit::Value`).
pub fn spend_step() -> bool {
    if UTIL_EXCEEDED.get().is_some() || budget::matcher_call_budget_exhausted() {
        return false;
    }
    match UTIL_STEPS_LEFT.get().checked_sub(1) {
        Some(left) => {
            UTIL_STEPS_LEFT.set(left);
            true
        }
        None => {
            UTIL_EXCEEDED.set(Some(UtilLimit::Value));
            false
        }
    }
}

thread_local! {
    static FILL_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Runs `fill`, which fills a cache later evaluations read for free (a file's scope table and
/// usage, a parent's counted siblings), counting the steps it spends apart
/// (`fill_steps`): a memoized verdict whose computation filled one replays only the rest, which
/// is what computing it again would spend.
pub fn filling<T>(fill: impl FnOnce() -> T) -> T {
    let (steps, fills) = (UTIL_STEPS_LEFT.get(), FILL_STEPS.get());
    let result = fill();
    // Fills within this one are part of it.
    FILL_STEPS.set(fills + steps.saturating_sub(UTIL_STEPS_LEFT.get()));
    result
}

/// Counts `steps` a cache fill spent directly (`filling`).
pub fn filled(steps: usize) {
    FILL_STEPS.set(FILL_STEPS.get() + steps);
}

/// Steps cache fills have spent on this thread (`filling`).
pub fn fill_steps() -> usize {
    FILL_STEPS.get()
}

thread_local! {
    static CLAUSE_STATE_READS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Notes that the clause in progress read state kept for it alone: a cache keyed by one of its
/// terms (`nthChild`'s counted siblings, a same-key walk's classes) or a file table charged to
/// each clause once (scopes, usage, PHP names). What a computation spends then depends on the
/// clause, so its verdict is that clause's alone (`memo`).
pub fn read_clause_state() {
    CLAUSE_STATE_READS.set(CLAUSE_STATE_READS.get() + 1);
}

/// How many reads of a clause's own state this thread has noted (`read_clause_state`).
pub fn clause_state_reads() -> u64 {
    CLAUSE_STATE_READS.get()
}
