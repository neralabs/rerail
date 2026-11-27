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
    /// scan does, for keys present and absent, and keys whose rows cross fences: by its first column (printed on it),
    /// and by both columns (printed on the first alone, or on both).
    #[test]
    fn an_orders_search_finds_a_scans_span() {
        // Rows (3k, j): each lead 3k held k % 5 + 1 times (150 more for some, crossing fences), j its place among them.
        let mut rows: Vec<[i64; 2]> = Vec::new();
        for k in 0..400i64 {
            for j in 0..(k % 5 + 1 + if k % 37 == 0 { 150 } else { 0 }) {
                rows.push([3 * k, j]);
            }
        }
        let n = rows.len();
        let fences: Vec<i64> = rows.iter().step_by(FENCE).map(|r| r[0]).collect();
        let lead: Vec<u8> = rows.iter().map(|r| print_vals(std::iter::once(r[0]))).collect();
        let both: Vec<u8> = rows.iter().map(|r| print_vals(r.iter().copied())).collect();
        let held = &rows;
        let cmp = |key: &[i64]| { let key = key.to_vec(); move |i: usize, m: usize| held[i][..m].cmp(&key[..m]) };
        let span = |key: &[i64]| (rows.partition_point(|r| r[..key.len()] < *key), rows.partition_point(|r| r[..key.len()] <= *key));
        let same = |got: (usize, usize), want: (usize, usize), what: &str| {
            if want.0 == want.1 { assert_eq!(got.0, got.1, "{what} (absent)") } else { assert_eq!(got, want, "{what}") }
        };
        for lead_key in -2..1210i64 {
            let key = [lead_key];
            assert_eq!(find(n, &fences, &[], &[], 0, &key, cmp(&key)), span(&key), "fences, key {key:?}");
            same(find(n, &fences, &[], &lead, 1, &key, cmp(&key)), span(&key), &format!("printed on the lead, key {key:?}"));
            same(find(n, &fences, &[], &both, 2, &key, cmp(&key)), span(&key), &format!("printed on both, a lead key {key:?} (bisected)"));
            for j in [-1, 0, 2, 7, 160] {
                let key = [lead_key, j];
                assert_eq!(find(n, &fences, &[], &[], 0, &key, cmp(&key)), span(&key), "fences, key {key:?}");
                same(find(n, &fences, &[], &lead, 1, &key, cmp(&key)), span(&key), &format!("printed on the lead, key {key:?}"));
                same(find(n, &fences, &[], &both, 2, &key, cmp(&key)), span(&key), &format!("printed on both, key {key:?}"));
            }
        }
    }
}
