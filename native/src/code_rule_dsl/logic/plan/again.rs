//! Why a stratum an edit reaches is derived again rather than maintained (decision 0364, R3 and R4): the reasons a run
//! reports, and 0090 §4's threshold and price. The level driver (`plan::Maintain`) chooses by them, and the corpus's
//! maintenance (`corpus::maintain`, which re-exports them) reports them.

/// The reasons a stratum an edit reaches may be derived again (decision 0364, R3): its part's first run, which has
/// nothing kept, and 0090 §4's threshold (R4). Every other stratum is maintained by the mode, or skipped.
pub const FIRST_RUN: &str = "nothing kept: its part's first run";
pub const THRESHOLD: &str = "the edit reaches too much of it (0090 §4)";
/// A part the level driver does not walk (no kept part, or gates): every stratum of its cut derived again.
pub const NOT_WALKED: &str = "its part is not walked: no kept part, or gates";
/// A stratum no changed read reaches whose rows are not kept (a fact relation's, or one the last run did not keep):
/// derived from its reads, as they were.
pub const NOT_KEPT: &str = "not reached, its rows not kept";
/// A native operator's stratum (the reach index, decision 0119): made again from its reads as they are now.
pub const NATIVE_OPERATOR: &str = "a native operator's stratum, made again";
/// A read changed with no version (a relation derived again in SQL, or with its kept rows not held).
pub const NO_VERSION: &str = "a read changed without a version";

/// 0090 §4's threshold (0364 R4): whether an edit whose changed reads give `delta` rows, of the `stored` rows those
/// reads hold, reaches too much of a stratum to maintain it: `delta ≥ θ · stored` (`RERAIL_AGAIN_THETA`, default 0.25) and `delta ≥
/// RERAIL_AGAIN_MIN` (default 1,024 rows: below it, maintaining costs little whatever the share). `RERAIL_AGAIN=off`
/// maintains every stratum. Read at each choice, so a test can set them.
pub fn past_threshold(delta: usize, stored: usize) -> bool {
    if std::env::var("RERAIL_AGAIN").is_ok_and(|v| v == "off") {
        return false;
    }
    let theta = std::env::var("RERAIL_AGAIN_THETA").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.25);
    let min = std::env::var("RERAIL_AGAIN_MIN").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(1024);
    delta >= min && delta as f64 >= theta * stored as f64
}

/// Whether a change of `delta` rows is past `RERAIL_AGAIN_MIN` (default 1,024): below it, a stratum is maintained
/// whatever its share or price.
pub fn past_floor(delta: usize) -> bool {
    if std::env::var("RERAIL_AGAIN").is_ok_and(|v| v == "off") {
        return false;
    }
    delta >= std::env::var("RERAIL_AGAIN_MIN").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(1024)
}

/// 0090 §4's second clause (0364 R4): whether maintaining, priced at `mode` work, costs more than deriving again at
/// `derive` (`RERAIL_AGAIN_PRICE`: the mode's share of a derive past which it derives again, default 1).
pub fn costs_more(mode: f64, derive: f64) -> bool {
    let share = std::env::var("RERAIL_AGAIN_PRICE").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(1.0);
    mode > share * derive
}

/// The class of a reason the maintain mode gave for not taking a stratum (`Run::maintain_mode_blocks`), as the
/// report counts it: one of its fixed reasons, `NO_VERSION` for a read named in it, or `other`.
pub fn reason_class(why: &str) -> &'static str {
    const FIXED: &[&str] = &[
        FIRST_RUN,
        THRESHOLD,
        "a gated aggregate",
        "a native, lattice, gate, request, produced, input or fact relation",
        "a nearest or ast literal: no native step",
        NOT_KEPT,
        NATIVE_OPERATOR,
        "an aggregate over its own stratum",
        "its kept rows are not in memory",
        "no rules",
        "recursive, its kept rows without levels",
        "RERAIL_MAINTAIN_MODE=off",
    ];
    if why.ends_with("changed without a version") {
        return NO_VERSION;
    }
    FIXED.iter().find(|r| **r == why).copied().unwrap_or("other")
}
