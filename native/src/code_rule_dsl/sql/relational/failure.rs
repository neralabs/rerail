//! Typed atomic limits. Unexpected SQL/host errors remain statement errors.
use crate::code_rule_dsl::matcher::UtilLimit;
const LIMITS: [UtilLimit; 4] = [
    UtilLimit::Depth,
    UtilLimit::Work,
    UtilLimit::Scope,
    UtilLimit::Value,
];
pub(super) fn encode(error: &str) -> Option<i64> {
    LIMITS
        .iter()
        .position(|limit| format!("{limit:?}") == error)
        .map(|i| -(i as i64) - 1)
}
pub(super) fn decode(tag: i64) -> Option<UtilLimit> {
    tag.checked_neg()
        .and_then(|n| n.checked_sub(1))
        .and_then(|n| usize::try_from(n).ok())
        .and_then(|n| LIMITS.get(n))
        .copied()
}
#[test]
fn atomic_limit_tags_round_trip_and_reject_other_errors() {
    for (index, limit) in LIMITS.into_iter().enumerate() {
        let tag = -(index as i64) - 1;
        assert_eq!(encode(&format!("{limit:?}")), Some(tag));
        assert_eq!(decode(tag), Some(limit));
    }
    for tag in [i64::MIN, -5, 0, 1, 2] {
        assert_eq!(decode(tag), None);
    }
    assert_eq!(encode("invalid terminal"), None);
}

/// Final API boundary: internal typed outcomes become the existing public diagnostic.
pub(super) fn diagnostic(tag: i64, clause: &str, path: &str) -> Result<serde_json::Value, String> {
    let limit = decode(tag).ok_or_else(|| format!("publication: invalid failure tag {tag}"))?;
    Ok(crate::code_rule_dsl::eval::failure(
        clause,
        "budget-exceeded",
        limit.message(),
        Some(path),
    ))
}
