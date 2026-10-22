//! Seeded mutants for the gate (`just facts-gate --mutants`): `FACTS_MUTANT=<n>` turns mutant n
//! on in a build with the `mutants` feature; without the feature `on` is `false` and folds away.
//! Numbers: 7 classifier (`classify::file_source`), 11.. rewrites (docs/facts/REWRITES.md).

#[cfg(feature = "mutants")]
pub fn on(n: u32) -> bool {
    use std::sync::OnceLock;
    static SELECTED: OnceLock<Option<u32>> = OnceLock::new();
    *SELECTED.get_or_init(|| std::env::var("FACTS_MUTANT").ok().and_then(|v| v.parse().ok())) == Some(n)
}

#[cfg(not(feature = "mutants"))]
#[inline(always)]
pub const fn on(_: u32) -> bool {
    false
}
