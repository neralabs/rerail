//! Finite name closure over evidence rows. Each stage uses a queue and visits each distinct
//! reachable name once per partition. Stage outputs annotate the original rows; their witnesses
//! stay intact. This is syntactic reachability, independent of semantic call/import resolution.

use super::crossfile::Relation;
use crate::code_rule_dsl::model::NameClosure;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet, VecDeque, hash_map::Entry};

#[derive(Clone, Copy)]
pub(in crate::code_rule_dsl) enum ClosureError {
    Incomplete,
    Limit,
}

impl ClosureError {
    pub(in crate::code_rule_dsl) fn code(self) -> &'static str {
        match self {
            Self::Incomplete => "source-incomplete",
            Self::Limit => "budget-exceeded",
        }
    }
    pub(in crate::code_rule_dsl) fn message(self) -> &'static str {
        match self {
            Self::Incomplete => "name closure cannot decide membership over incomplete evidence",
            Self::Limit => "name closure exceeded maxVisitedNodes or was interrupted",
        }
    }
}

#[derive(Default)]
struct Partition {
    known: HashSet<String>,
    edges: HashMap<String, Vec<String>>,
    pending: VecDeque<String>,
}

impl Partition {
    fn seed(&mut self, name: &str) {
        if self.known.insert(name.to_owned()) {
            self.pending.push_back(name.to_owned());
        }
    }
}

struct Work {
    left: usize,
}

impl Work {
    fn spend(&mut self) -> Result<(), ClosureError> {
        // Count every operation locally; poll the shared allowance at its calibrated loop interval.
        if self.left == 0 || crate::code_rule_dsl::budget::step_budget_exhausted() {
            return Err(ClosureError::Limit);
        }
        self.left -= 1;
        Ok(())
    }
}

fn partition_key(row: &Map<String, Value>, fields: &[String]) -> Option<String> {
    let values = fields
        .iter()
        .map(|field| row.get(field).filter(|v| !v.is_null()))
        .collect::<Option<Vec<_>>>()?;
    Some(serde_json::to_string(&values).expect("JSON partition key"))
}

pub(in crate::code_rule_dsl) fn derive(
    relation: &mut Relation,
    stages: &[NameClosure],
    limit: usize,
) -> Result<(), ClosureError> {
    if stages.is_empty() {
        return Ok(());
    }
    // Absence from an incomplete graph is not false. Refuse the derived relation rather than
    // letting `not`/`ne` or a zero count convert unknown membership into a clean verdict.
    if !relation.incomplete.is_empty() {
        return Err(ClosureError::Incomplete);
    }
    let mut work = Work { left: limit };
    let mut completed = HashMap::<String, HashMap<String, Partition>>::new();
    for stage in stages {
        let mut partitions = HashMap::<String, Partition>::new();
        for row in &relation.rows {
            work.spend()?;
            let Some(key) = partition_key(row, &stage.partition_by) else {
                continue;
            };
            let partition = match partitions.entry(key.clone()) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => {
                    let mut partition = Partition::default();
                    for seed in &stage.seeds {
                        work.spend()?;
                        partition.seed(seed);
                    }
                    for dependency in &stage.extends {
                        if let Some(earlier) = completed.get(dependency).and_then(|p| p.get(&key)) {
                            for name in &earlier.known {
                                work.spend()?;
                                partition.seed(name);
                            }
                        }
                    }
                    entry.insert(partition)
                }
            };
            if let Some(seed) = stage
                .seed_field
                .as_ref()
                .and_then(|field| row.get(field))
                .and_then(Value::as_str)
            {
                partition.seed(seed);
            }
            if let Some((from, to)) = &stage.edges
                && let (Some(from), Some(to)) = (
                    row.get(from).and_then(Value::as_str),
                    row.get(to).and_then(Value::as_str),
                )
            {
                partition
                    .edges
                    .entry(from.to_owned())
                    .or_default()
                    .push(to.to_owned());
            }
        }
        for partition in partitions.values_mut() {
            while let Some(name) = partition.pending.pop_front() {
                work.spend()?;
                for next in partition.edges.remove(&name).into_iter().flatten() {
                    work.spend()?;
                    partition.seed(&next);
                }
            }
        }
        for row in &mut relation.rows {
            work.spend()?;
            let partition =
                partition_key(row, &stage.partition_by).and_then(|key| partitions.get(&key));
            for (output, field) in &stage.contains {
                work.spend()?;
                let known = partition.is_some_and(|p| {
                    row.get(field)
                        .and_then(Value::as_str)
                        .is_some_and(|value| p.known.contains(value))
                });
                row.insert(output.clone(), Value::Bool(known));
            }
        }
        completed.insert(stage.name.clone(), partitions);
    }
    Ok(())
}
