//! Set membership proofs over frozen sources. Independent of SQL and physical storage.
use super::Decision;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
struct Projection {
    source: String,
    columns: Vec<usize>,
}

/// A union of projected sources. This describes membership, never bag multiplicity.
#[derive(Clone, Debug, serde::Serialize)]
pub struct KeySet {
    width: usize,
    terms: BTreeSet<Projection>,
}
impl KeySet {
    pub fn source(source: impl Into<String>, width: usize) -> Self {
        Self {
            width,
            terms: [Projection {
                source: source.into(),
                columns: (0..width).collect(),
            }]
            .into(),
        }
    }
    pub fn union(width: usize, sets: impl IntoIterator<Item = Self>) -> Self {
        let mut terms = BTreeSet::new();
        for set in sets {
            assert_eq!(width, set.width);
            terms.extend(set.terms);
        }
        Self { width, terms }
    }
    pub fn project(&self, columns: &[usize]) -> Self {
        assert!(columns.iter().all(|&c| c < self.width));
        Self {
            width: columns.len(),
            terms: self
                .terms
                .iter()
                .map(|t| Projection {
                    source: t.source.clone(),
                    columns: columns.iter().map(|&c| t.columns[c]).collect(),
                })
                .collect(),
        }
    }
    pub fn contained_in(&self, other: &Self) -> Decision {
        let proven = self.width == other.width && self.terms.is_subset(&other.terms);
        Decision::new(
            proven,
            if proven {
                "Every projected source of the affected keys is included in the replacement keys"
            } else {
                "Source, projection and union lineage does not establish key containment"
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn evaluate(set: &KeySet, a: &[Vec<i64>], b: &[Vec<i64>]) -> BTreeSet<Vec<i64>> {
        set.terms
            .iter()
            .flat_map(|t| {
                let rows = if t.source == "a" { a } else { b };
                rows.iter()
                    .map(|r| t.columns.iter().map(|&c| r[c]).collect())
            })
            .collect()
    }
    #[test]
    fn containment_proofs_hold_for_empty_duplicate_and_overlapping_sources() {
        let a = KeySet::source("a", 2);
        let b = KeySet::source("b", 2);
        let ab = KeySet::union(2, [a.clone(), b.clone()]);
        let sets = [
            KeySet::union(1, []),
            a.project(&[0]),
            a.project(&[1]),
            b.project(&[0]),
            ab.project(&[0]),
            KeySet::union(1, [a.project(&[0]), b.project(&[0])]),
            a.project(&[1, 0]).project(&[1]),
        ];
        for mask in 0..256_u32 {
            let rows = (0..8).map(|i| vec![i % 3, i / 3]).collect::<Vec<_>>();
            let ar = rows
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, r)| r.clone())
                .collect::<Vec<_>>();
            let br = rows
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) == 0)
                .map(|(_, r)| r.clone())
                .collect::<Vec<_>>();
            for x in &sets {
                for y in &sets {
                    if x.contained_in(y).proven {
                        assert!(evaluate(x, &ar, &br).is_subset(&evaluate(y, &ar, &br)));
                    }
                }
            }
        }
        assert!(sets[1].contained_in(&sets[6]).proven);
        assert!(!sets[1].contained_in(&sets[3]).proven);
        assert!(!sets[4].contained_in(&sets[1]).proven);
    }
}
