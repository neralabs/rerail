//! Validated SQL optimizations, enabled in production. Only the benchmark build can disable
//! them to measure the old access paths independently.

#[inline]
fn enabled(_variable: &str, _disabled: &str) -> bool {
    #[cfg(feature = "unbounded-work")]
    return std::env::var(_variable).map_or(true, |value| value != _disabled);
    #[cfg(not(feature = "unbounded-work"))]
    true
}

pub(super) fn batch_transaction() -> bool {
    enabled("RERAIL_SQL_BATCH_TRANSACTION", "0")
}

pub(super) fn direct_candidates() -> bool {
    enabled("RERAIL_SQL_DIRECT_CANDIDATES", "0")
}

pub(super) fn integer_pattern_keys() -> bool {
    enabled("RERAIL_SQL_PATTERN_KEYS", "text")
}

pub(super) fn root_guards() -> bool {
    enabled("RERAIL_SQL_ROOT_GUARDS", "0")
}

pub(super) fn elide_kind_copies() -> bool {
    enabled("RERAIL_SQL_ELIDE_KIND_COPIES", "0")
}
