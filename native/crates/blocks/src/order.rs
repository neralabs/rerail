//! A sorted order's search, written once (t56 step 3): the ids of a relation sorted by some of its columns, and beside
//! them, in the relation's block (`relation::Relation::try_add_order`, decision 0478 step 5b), the leading value of
//! every `FENCE`th id (`fences`), or each id's 16-bit tag of it (`tags`, an order sorted by its lead's hash), and each
//! id's 8-bit print of its whole key (`prints`, an order probed by whole keys). The set engine's runs
//! (`setfix::runs::Run`) and the batch's relations (`facts::vtab::Relation`) both search their orders here.
//!
//! A probe by a whole key reads the fences (or tags), then the span's prints, eight a word, and then only the rows
//! whose print is the key's: about one row read a probe, where a bisection read about eight, each a decode of the
//! row's sealed chunk.

// The sorted orders' reads: the floor's one definition (`rerail_delta_core::order`).
pub use rerail_delta_core::order::*;

#[cfg(test)]
mod tests {
    use super::*;

    /// An order of runs of equal keys, searched by fences alone and by fences and prints, finds each key's span as a
    /// scan does, for keys present and absent, and keys whose rows cross fences.
    #[test]
    fn an_orders_search_finds_a_scans_span() {
        // Values 0, 0, 0, 3, 3, 6, ...: each value 3k, k % 5 + 1 times; long runs cross fences.
        let mut vals: Vec<i64> = Vec::new();
        for k in 0..400i64 {
            for _ in 0..(k % 5 + 1 + if k % 37 == 0 { 150 } else { 0 }) {
                vals.push(3 * k);
            }
        }
        let n = vals.len();
        let fences: Vec<i64> = vals.iter().step_by(FENCE).copied().collect();
        let prints: Vec<u8> = vals.iter().map(|&v| print_vals(std::iter::once(v))).collect();
        for key in -2..1210i64 {
            let want = (vals.partition_point(|&v| v < key), vals.partition_point(|&v| v <= key));
            let cmp = |i: usize| vals[i].cmp(&key);
            let (s, e) = find(n, &fences, &[], &[], &[key], true, cmp);
            assert_eq!((s, e), want, "fences, key {key}");
            let (s, e) = find(n, &fences, &[], &prints, &[key], true, cmp);
            if want.0 == want.1 {
                assert_eq!(s, e, "prints, key {key} (absent)");
            } else {
                assert_eq!((s, e), want, "prints, key {key}");
            }
        }
    }
}
