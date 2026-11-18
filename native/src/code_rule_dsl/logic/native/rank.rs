//! `Native::Rank`: a count of a group's rows below a bound (decision 0361 step 7), without the pairs. A rule's
//! `count : { …, y OP x }` whose comparison splits its body in two (`rank_form`) is made from two relations:
//! the asked rows `query(g…, q…, kx)` and the counted rows `counted(g…, c…, ky)`. Each output row `(g…, q…, n)`
//! holds how many distinct `(g…, c…)` tuples of its group `g…` have a key `ky OP kx`.
//!
//! A tuple of several keys passes when one of them does, so it counts by its least key for `<` and `<=` and by its
//! greatest for `>` and `>=`. An output tuple of several bounds counts what one of them passes: its greatest bound for
//! `<` and `<=`, its least for `>` and `>=`. Each group's keys are sorted once and each bound is a binary search:
//! O((Q + C) log C) work, where the pairs were O(Q × C) in each group.

/// The comparison `ky OP kx`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Lt,
    Le,
    Gt,
    Ge,
}

impl Op {
    pub fn parse(op: &str) -> Option<Op> {
        Some(match op {
            "<" => Op::Lt,
            "<=" => Op::Le,
            ">" => Op::Gt,
            ">=" => Op::Ge,
            _ => return None,
        })
    }
    pub fn text(self) -> &'static str {
        match self {
            Op::Lt => "<",
            Op::Le => "<=",
            Op::Gt => ">",
            Op::Ge => ">=",
        }
    }
    /// `x OP y` written the other way round: `y OP' x`.
    pub fn flipped(self) -> Op {
        match self {
            Op::Lt => Op::Gt,
            Op::Le => Op::Ge,
            Op::Gt => Op::Lt,
            Op::Ge => Op::Le,
        }
    }
    fn below(self) -> bool {
        matches!(self, Op::Lt | Op::Le)
    }
}

/// The output rows: each distinct `query` tuple but its last column (the bound), then its count. `query` and
/// `counted` are rows of their relations, each led by the `group` columns, its key last.
pub fn rank(query: &[Vec<i64>], counted: &[Vec<i64>], group: usize, op: Op) -> Vec<Vec<i64>> {
    // Each counted tuple by its extreme key, then by group and key.
    let mut keys: Vec<&[i64]> = counted.iter().map(Vec::as_slice).collect();
    keys.sort_unstable();
    let mut by_group: Vec<(&[i64], i64)> = Vec::with_capacity(keys.len());
    let mut at = 0;
    while at < keys.len() {
        let w = keys[at].len();
        let tuple = &keys[at][..w - 1];
        let end = at + keys[at..].partition_point(|r| &r[..r.len() - 1] == tuple);
        // Sorted, so the tuple's least key is first and its greatest last.
        let k = if op.below() { keys[at][w - 1] } else { keys[end - 1][w - 1] };
        by_group.push((&tuple[..group], k));
        at = end;
    }
    by_group.sort_unstable();
    // Each asked tuple by its extreme bound.
    let mut asked: Vec<&[i64]> = query.iter().map(Vec::as_slice).collect();
    asked.sort_unstable();
    let mut out = Vec::new();
    let mut at = 0;
    while at < asked.len() {
        let w = asked[at].len();
        let tuple = &asked[at][..w - 1];
        let end = at + asked[at..].partition_point(|r| &r[..r.len() - 1] == tuple);
        let bound = if op.below() { asked[end - 1][w - 1] } else { asked[at][w - 1] };
        let g = &tuple[..group];
        let lo = by_group.partition_point(|(h, _)| *h < g);
        let hi = lo + by_group[lo..].partition_point(|(h, _)| *h == g);
        let ks = &by_group[lo..hi];
        let n = match op {
            Op::Lt => ks.partition_point(|&(_, k)| k < bound),
            Op::Le => ks.partition_point(|&(_, k)| k <= bound),
            Op::Gt => ks.len() - ks.partition_point(|&(_, k)| k <= bound),
            Op::Ge => ks.len() - ks.partition_point(|&(_, k)| k < bound),
        };
        let mut row = tuple.to_vec();
        row.push(n as i64);
        out.push(row);
        at = end;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The count is the pairs' count: every op, a tuple of two keys, a tuple asked with two bounds, a group with no
    /// counted rows.
    #[test]
    fn rank_counts_what_the_pairs_would() {
        // counted(g, c, ky); query(g, q, kx).
        let counted = vec![vec![1, 10, 5], vec![1, 11, 7], vec![1, 11, 2], vec![1, 12, 9], vec![2, 20, 1]];
        let query = vec![vec![1, 100, 7], vec![1, 101, 3], vec![1, 101, 8], vec![2, 200, 0], vec![3, 300, 5]];
        for op in [Op::Lt, Op::Le, Op::Gt, Op::Ge] {
            let mut want: Vec<Vec<i64>> = Vec::new();
            let mut tuples: Vec<(i64, i64)> = query.iter().map(|r| (r[0], r[1])).collect();
            tuples.sort();
            tuples.dedup();
            for (g, q) in tuples {
                let bounds: Vec<i64> = query.iter().filter(|r| r[0] == g && r[1] == q).map(|r| r[2]).collect();
                let pass = |ky: i64| bounds.iter().any(|&kx| match op { Op::Lt => ky < kx, Op::Le => ky <= kx, Op::Gt => ky > kx, Op::Ge => ky >= kx });
                let mut cs: Vec<i64> = counted.iter().filter(|r| r[0] == g && pass(r[2])).map(|r| r[1]).collect();
                cs.sort();
                cs.dedup();
                want.push(vec![g, q, cs.len() as i64]);
            }
            assert_eq!(rank(&query, &counted, 1, op), want, "{op:?}");
        }
    }
}
