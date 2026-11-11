//! Functional dependencies over symbolic columns. Filters preserve keys; projections can lose them.
use super::Decision;
use std::collections::{BTreeMap, BTreeSet};

pub(in crate::code_rule_dsl) type Keys = BTreeMap<String, Vec<Vec<usize>>>;

#[derive(Clone, Debug, serde::Serialize)]
pub(in crate::code_rule_dsl) struct Input {
    pub relation: String,
    pub columns: Vec<String>,
    pub keys: Vec<Vec<usize>>,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Projection {
    pub inputs: Vec<Input>,
    /// Direct column references only. Computed outputs cannot be inverted.
    pub outputs: Vec<Option<String>>,
    pub equalities: Vec<(String, String)>,
    pub fixed: BTreeSet<String>,
    /// A relation-valued or opaque operation can introduce additional witnesses.
    pub opaque: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(in crate::code_rule_dsl) struct Proof {
    pub decision: Decision,
    pub inputs: Vec<Input>,
    pub determined: BTreeSet<String>,
    pub unresolved: BTreeSet<String>,
}

impl Projection {
    pub fn prove(&self, outputs: &[usize]) -> Proof {
        let mut determined = self.fixed.clone();
        determined.extend(outputs.iter().filter_map(|i| self.outputs.get(*i)?.clone()));
        loop {
            let before = determined.len();
            for (a, b) in &self.equalities {
                if determined.contains(a) {
                    determined.insert(b.clone());
                }
                if determined.contains(b) {
                    determined.insert(a.clone());
                }
            }
            for input in &self.inputs {
                if input.keys.iter().any(|key| {
                    key.iter().all(|i| {
                        input
                            .columns
                            .get(*i)
                            .is_some_and(|c| determined.contains(c))
                    })
                }) {
                    determined.extend(input.columns.iter().cloned());
                }
            }
            if determined.len() == before {
                break;
            }
        }
        let unresolved: BTreeSet<_> = self
            .inputs
            .iter()
            .flat_map(|i| &i.columns)
            .filter(|c| !determined.contains(*c))
            .cloned()
            .collect();
        let proven = !self.opaque && unresolved.is_empty();
        let reason = if self.opaque {
            "An opaque or relation-valued operation can introduce multiple witnesses".into()
        } else if proven {
            "Projected columns and equalities determine a key of every input".into()
        } else {
            format!(
                "Projected columns do not determine input columns: {}",
                unresolved.iter().cloned().collect::<Vec<_>>().join(", ")
            )
        };
        Proof {
            decision: Decision::new(proven, reason),
            inputs: self.inputs.clone(),
            determined,
            unresolved,
        }
    }

    pub fn unique(&self) -> Proof {
        self.prove(&(0..self.outputs.len()).collect::<Vec<_>>())
    }

    /// A bounded, conservative minimal key, rather than an exponential search for all keys.
    pub fn key(&self) -> Option<Vec<usize>> {
        let mut key: Vec<_> = (0..self.outputs.len()).collect();
        if !self.prove(&key).decision.proven {
            return None;
        }
        if key.len() > 64 {
            return Some(key);
        }
        for i in (0..self.outputs.len()).rev() {
            let candidate: Vec<_> = key.iter().copied().filter(|k| *k != i).collect();
            if self.prove(&candidate).decision.proven {
                key = candidate;
            }
        }
        Some(key)
    }
}

/// Individually unique producers remain unique when literal head tags separate every pair.
pub(in crate::code_rule_dsl) fn union_support(producers: &[(bool, Vec<super::scalars::Expression>)]) -> Decision {
    use super::scalars::Expression as E;
    if producers.is_empty() || producers.iter().any(|(unique,_)| !unique) {
        return Decision::new(false,"Every producer needs a witness uniqueness proof");
    }
    for (i,(_,a)) in producers.iter().enumerate() {
        for (_,b) in &producers[..i] {
            let disjoint = a.iter().zip(b).any(|(a,b)| match (a,b) {
                (E::Integer(a),E::Integer(b)) => a != b,
                (E::Text(a),E::Text(b)) => a != b,
                _ => false,
            });
            if !disjoint { return Decision::new(false,"Producer head domains can overlap; support reconciliation is required"); }
        }
    }
    Decision::new(true,"Unique witnesses and pairwise distinct literal head tags prove disjoint producer supports")
}
