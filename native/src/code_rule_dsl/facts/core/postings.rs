/// Stable counting placement gives the same (kind, node) order as sorting pairs.
/// A document normally has a single grammar's <=1024 kind codes. Bound allocation
/// and fall back to sorting for unusual cross-grammar inputs.
pub(super) fn by_kind(kinds: &[u32]) -> (Vec<u32>, Vec<u32>, Vec<u32>) {
    let Some(&min) = kinds.iter().min() else {
        return (vec![], vec![0], vec![]);
    };
    let max = *kinds.iter().max().unwrap();
    if max - min > 65535 {
        return super::postings(
            kinds
                .iter()
                .enumerate()
                .map(|(id, &k)| (k, id as u32))
                .collect(),
        );
    }
    let mut counts = vec![0u32; (max - min) as usize + 1];
    for &k in kinds {
        counts[(k - min) as usize] += 1;
    }
    let (mut keys, mut offsets) = (Vec::new(), Vec::new());
    let mut total = 0;
    for (i, count) in counts.iter_mut().enumerate() {
        let n = *count;
        if n > 0 {
            keys.push(min + i as u32);
            offsets.push(total);
        }
        *count = total;
        total += n;
    }
    offsets.push(total);
    let mut nodes = vec![0u32; kinds.len()];
    for (id, &k) in kinds.iter().enumerate() {
        let cursor = &mut counts[(k - min) as usize];
        nodes[*cursor as usize] = id as u32;
        *cursor += 1;
    }
    (keys, offsets, nodes)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    #[test]
    fn counting_postings_preserve_order_empty_sparse_and_extreme_keys() {
        let mut cases = vec![
            vec![],
            vec![0],
            vec![u32::MAX, 0, u32::MAX],
            vec![2048, 2053, 2048, 3000],
        ];
        cases.push(
            (0..100_000)
                .map(|i| 1024 + ((i * 37 + i / 13) % 800))
                .collect(),
        );
        for kinds in cases {
            let expected = super::super::postings(
                kinds
                    .iter()
                    .enumerate()
                    .map(|(i, &k)| (k, i as u32))
                    .collect(),
            );
            assert_eq!(by_kind(&kinds), expected);
        }
    }
}
