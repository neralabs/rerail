//! Test-only input-lifecycle experiments. Production has one unchanged execution path.
pub(crate) fn enabled(part: &str) -> bool {
    std::env::var("RERAIL_INPUT_SPIKE").is_ok_and(|s| s.split(',').any(|p| p == part))
}

/// Reuse the current or following line interval; binary-search arbitrary jumps.
/// Unlike a forward-only scan, this preserves overlapping and backwards spans.
pub(crate) fn line(starts: &[usize], byte: usize, cursor: &mut usize) -> usize {
    if byte < starts[*cursor] {
        *cursor = starts.partition_point(|&start| start <= byte) - 1;
    } else if starts.get(*cursor + 1).is_some_and(|&start| start <= byte) {
        *cursor += 1;
        if starts.get(*cursor + 1).is_some_and(|&start| start <= byte) {
            *cursor = starts.partition_point(|&start| start <= byte) - 1;
        }
    }
    *cursor
}

/// Stable counting placement gives the same (kind, node) order as sorting pairs.
/// A document normally has a single grammar's <=1024 kind codes. Bound allocation
/// and fall back to sorting for unusual cross-grammar inputs.
pub(crate) fn postings(kinds: &[u32]) -> (Vec<u32>, Vec<u32>, Vec<u32>) {
    let Some(&min) = kinds.iter().min() else { return (vec![], vec![0], vec![]); };
    let max = *kinds.iter().max().unwrap();
    if max - min > 65535 {
        return super::postings(kinds.iter().enumerate().map(|(id, &k)| (k, id as u32)).collect());
    }
    let mut counts = vec![0u32; (max - min) as usize + 1];
    for &k in kinds { counts[(k-min) as usize] += 1; }
    let (mut keys, mut offsets) = (Vec::new(), Vec::new());
    let mut total = 0;
    for (i, count) in counts.iter_mut().enumerate() {
        let n = *count;
        if n > 0 { keys.push(min + i as u32); offsets.push(total); }
        *count = total;
        total += n;
    }
    offsets.push(total);
    let mut nodes = vec![0u32; kinds.len()];
    for (id, &k) in kinds.iter().enumerate() {
        let cursor = &mut counts[(k-min) as usize];
        nodes[*cursor as usize] = id as u32;
        *cursor += 1;
    }
    (keys, offsets, nodes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::lines::{LineTable, Terminators};
    #[test]
    fn interval_lines_equal_binary_search_at_every_byte_in_both_directions() {
        for endings in [Terminators::Newlines, Terminators::Eslint] {
            for source in ["", "a\r\nb\rc\nd\u{2028}e\u{2029}f\n", "x\n\n\nlong line"] {
                let lines = LineTable::new(source, endings);
                let mut cursor = 0;
                for byte in (0..=source.len()).chain((0..=source.len()).rev()).chain([source.len(), 0, source.len()/2, source.len(), 0]) {
                    assert_eq!(line(lines.starts(), byte, &mut cursor), lines.line(byte));
                }
            }
        }
    }
    #[test]
    fn counting_postings_preserve_order_empty_sparse_and_extreme_keys() {
        let mut cases = vec![vec![], vec![0], vec![u32::MAX, 0, u32::MAX], vec![2048, 2053, 2048, 3000]];
        cases.push((0..100_000).map(|i| 1024 + ((i * 37 + i / 13) % 800)).collect());
        for kinds in cases {
            let expected = super::super::postings(kinds.iter().enumerate().map(|(i, &k)| (k, i as u32)).collect());
            assert_eq!(postings(&kinds), expected);
        }
    }
}
