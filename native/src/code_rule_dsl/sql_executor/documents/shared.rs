//! Shared literal facts retain document reference counts and two immutable ordered indexes.
use super::{
    tree::{Link, Measure, Node, size},
    *,
};
use std::ops::Bound;

#[derive(Clone)]
struct Count(usize);
#[derive(Default)]
struct Summary {
    distinct: usize,
    first: i64,
    last: i64,
}
impl Measure<Vec<i64>> for Count {
    type Summary = Summary;
    fn measure(&self, key: &Vec<i64>, left: Option<&Summary>, right: Option<&Summary>) -> Summary {
        let v = key[0];
        Summary {
            distinct: 1
                + left.map_or(0, |s| s.distinct - usize::from(s.last == v))
                + right.map_or(0, |s| s.distinct - usize::from(s.first == v)),
            first: left.map_or(v, |s| s.first),
            last: right.map_or(v, |s| s.last),
        }
    }
}
type Index = Node<Vec<i64>, Count>;
#[derive(Clone, Default)]
pub(super) struct Shared {
    primary: Link<Vec<i64>, Count>,
    secondary: Link<Vec<i64>, Count>,
}
impl Shared {
    pub fn changed(&self, delta: &BTreeMap<Vec<i64>, isize>, work: &mut usize) -> Self {
        let mut next = self.clone();
        for (row, &delta) in delta {
            if delta == 0 {
                continue;
            }
            let refs = Index::get(&next.primary, row).map_or(0, |n| n.value.0);
            let refs = refs
                .checked_add_signed(delta)
                .expect("literal references reflect document membership");
            let count = (refs > 0).then_some(Count(refs));
            next.primary = Index::set(&next.primary, row.clone(), count.clone(), work);
            next.secondary = Index::set(&next.secondary, vec![row[1], row[0]], count, work);
        }
        next
    }
    pub fn contains(&self, row: &Vec<i64>) -> bool {
        Index::get(&self.primary, row).is_some()
    }
    fn selected(&self, c: usize, lo: Option<(i64, bool)>, hi: Option<(i64, bool)>) -> Vec<usize> {
        let lower = lo.map_or(Bound::Unbounded, |(v, inc)| {
            if inc {
                Bound::Included(vec![v, i64::MIN])
            } else {
                Bound::Excluded(vec![v, i64::MAX])
            }
        });
        let upper = hi.map_or(Bound::Unbounded, |(v, inc)| {
            if inc {
                Bound::Included(vec![v, i64::MAX])
            } else {
                Bound::Excluded(vec![v, i64::MIN])
            }
        });
        let mut rows = vec![];
        let index = if c == 0 {
            &self.primary
        } else {
            &self.secondary
        };
        Index::range(index, &lower, &upper, 0, &mut |n, rank| {
            rows.push(if c == 0 {
                rank
            } else {
                Index::rank(&self.primary, &vec![n.key[1], n.key[0]]).unwrap()
            });
        });
        rows
    }
}
impl Source for Shared {
    fn revision(&self)->Option<u64> {Some(0)}
    fn columns(&self) -> Vec<String> {
        vec!["v".into(), "t".into()]
    }
    fn len(&self) -> usize {
        size(&self.primary)
    }
    fn estimated_matches(&self, c: usize, b: bool) -> usize {
        let index = if c == 0 {
            &self.primary
        } else {
            &self.secondary
        };
        let n = self
            .len()
            .div_ceil(index.as_ref().map_or(1, |n| n.summary.distinct).max(1));
        if b { n.saturating_mul(8) } else { n }
    }
    fn indexed(&self, _: usize) -> bool {
        true
    }
    fn range_indexed(&self, _: usize) -> bool {
        true
    }
    fn lookup(&self, c: usize, values: &[i64]) -> Option<Vec<usize>> {
        Some(
            values
                .iter()
                .flat_map(|&v| self.selected(c, Some((v, true)), Some((v, true))))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
        )
    }
    fn range_lookup(
        &self,
        c: usize,
        lo: Option<(i64, bool)>,
        hi: Option<(i64, bool)>,
    ) -> Option<Vec<usize>> {
        Some(self.selected(c, lo, hi))
    }
    fn value(&self, r: usize, c: usize) -> i64 {
        Index::nth(&self.primary, r).key[c]
    }
}
