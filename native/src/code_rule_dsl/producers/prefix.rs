//! Backend-independent prefix factorization. Callers certify that terms are safe to share and
//! establish equal semantic input domains before executing a stage. Equality uses complete terms.
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub(crate) struct Stage<K> {
    pub parent: Option<usize>,
    pub terms: Vec<K>,
    pub consumers: Vec<usize>,
}
#[derive(Clone, Debug)]
pub(crate) struct Tail {
    pub consumer: usize,
    pub parent: Option<usize>,
    pub consumed: usize,
}
#[derive(Clone, Debug, Default)]
pub(crate) struct Plan<K> {
    pub stages: Vec<Stage<K>>,
    pub tails: Vec<Tail>,
}
impl<K: Ord + Clone> Plan<K> {
    pub fn new(programs: &[(usize, &[K])]) -> Self {
        let mut plan = Self {
            stages: vec![],
            tails: vec![],
        };
        plan.split(programs, 0, None);
        plan
    }
    fn split(&mut self, programs: &[(usize, &[K])], depth: usize, parent: Option<usize>) {
        let mut groups = BTreeMap::<&K, Vec<(usize, &[K])>>::new();
        for &(consumer, terms) in programs {
            if let Some(term) = terms.get(depth) {
                groups.entry(term).or_default().push((consumer, terms));
            } else {
                self.tails.push(Tail {
                    consumer,
                    parent,
                    consumed: depth,
                });
            }
        }
        for group in groups.into_values() {
            if group.len() == 1 {
                self.tails.push(Tail {
                    consumer: group[0].0,
                    parent,
                    consumed: depth,
                });
                continue;
            }
            let first = group[0].1;
            let mut end = depth + 1;
            while end < first.len()
                && group
                    .iter()
                    .all(|(_, terms)| terms.get(end) == first.get(end))
            {
                end += 1;
            }
            let id = self.stages.len();
            self.stages.push(Stage {
                parent,
                terms: first[depth..end].to_vec(),
                consumers: group.iter().map(|(id, _)| *id).collect(),
            });
            self.split(&group, end, Some(id));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recursive_prefixes_preserve_each_tail_and_complete_identity() {
        let plan = Plan::new(&[
            (0, &[1, 2, 3][..]),
            (1, &[1, 2, 4]),
            (2, &[1, 5, 6]),
            (3, &[1, 5, 7]),
            (4, &[9]),
        ]);
        assert_eq!(plan.stages.len(), 3);
        assert_eq!(plan.stages[0].terms, vec![1]);
        assert_eq!(plan.stages[1].terms, vec![2]);
        assert_eq!(plan.stages[2].terms, vec![5]);
        assert_eq!(plan.stages[1].parent, Some(0));
        assert_eq!(
            plan.tails
                .iter()
                .map(|t| (t.consumer, t.consumed))
                .collect::<BTreeMap<_, _>>(),
            BTreeMap::from([(0, 2), (1, 2), (2, 2), (3, 2), (4, 0)])
        );
        // A digest-like prefix is insufficient: the rest of the typed value differs.
        assert!(
            Plan::new(&[(0, &[(7, "a")][..]), (1, &[(7, "b")])])
                .stages
                .is_empty()
        );
    }
}
