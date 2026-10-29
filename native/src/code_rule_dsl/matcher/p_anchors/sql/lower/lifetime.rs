//! Schedule generated SQL producers and release physical inputs after their last consumer.
use std::collections::{BTreeSet, HashMap};
use std::sync::LazyLock;

pub struct Input<'a> {
    pub name: &'a str,
    pub sql: &'a str,
    pub temporary: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Action {
    Prepare(usize),
    Query(usize),
    Release(usize),
}

pub struct Schedule {
    pub actions: Vec<Action>,
    pub peak_relations: usize,
}

/// The same preparation graph supplies scheduling and compatible physical grouping.
pub struct Dependencies<'a> {
    pub prerequisites: Vec<Vec<usize>>,
    consumers: Vec<Vec<usize>>,
    names: HashMap<&'a str, usize>,
    writers: HashMap<&'a str, usize>,
}

impl<'a> Dependencies<'a> {
    pub fn new(inputs: &[Input<'a>]) -> Self {
        static IDENTIFIER: LazyLock<regex::Regex> = LazyLock::new(|| {
            regex::Regex::new(r"[A-Za-z_][A-Za-z_0-9]*").expect("generated identifier")
        });
        static WRITE: LazyLock<regex::Regex> = LazyLock::new(|| {
            regex::Regex::new(
                r"(?i)\b(?:INSERT(?: OR IGNORE)? INTO|UPDATE|DELETE FROM) ([A-Za-z_][A-Za-z_0-9]*)",
            )
            .expect("generated write target")
        });
        let names: HashMap<_, _> = inputs
            .iter()
            .enumerate()
            .filter(|(_, input)| input.temporary)
            .map(|(i, input)| (input.name, i))
            .collect();
        let writes: Vec<_> = inputs
            .iter()
            .map(|input| {
                if input.temporary {
                    None
                } else {
                    Some(
                        WRITE
                            .captures(input.sql)
                            .expect("generated retained write")
                            .get(1)
                            .unwrap()
                            .as_str(),
                    )
                }
            })
            .collect();
        #[derive(Default)]
        struct Access {
            writer: Option<usize>,
            readers: Vec<usize>,
        }
        let mut resources: HashMap<_, Access> = writes
            .iter()
            .flatten()
            .map(|&name| (name, Access::default()))
            .collect();
        let mut dependencies = Vec::with_capacity(inputs.len());
        let mut consumers = Vec::with_capacity(inputs.len());
        for (i, input) in inputs.iter().enumerate() {
            let tokens: BTreeSet<_> = IDENTIFIER
                .find_iter(input.sql)
                .map(|t| t.as_str())
                .filter(|name| names.contains_key(name) || resources.contains_key(name))
                .collect();
            let mut needs: BTreeSet<_> = tokens
                .iter()
                .filter_map(|token| names.get(token).copied())
                .filter(|&n| n != i)
                .collect();
            consumers.push(needs.iter().copied().collect::<Vec<_>>());
            for token in tokens {
                if let Some(access) = resources.get_mut(token) {
                    needs.extend(access.writer);
                    if writes[i] == Some(token) {
                        needs.extend(access.readers.drain(..));
                        access.writer = Some(i);
                    } else {
                        access.readers.push(i);
                    }
                }
            }
            assert!(
                needs.iter().all(|&n| n < i),
                "generated preparation dependencies must precede their consumer"
            );
            dependencies.push(needs.into_iter().collect::<Vec<_>>());
        }
        Self {
            prerequisites: dependencies,
            consumers,
            names,
            writers: resources
                .into_iter()
                .filter_map(|(name, access)| access.writer.map(|writer| (name, writer)))
                .collect(),
        }
    }
}

impl Schedule {
    /// Inputs contain compiler-generated SQL; user strings are bound parameters.
    /// Retained writes are ordered against both earlier readers and earlier writers.
    #[allow(dead_code)] // Used by standalone replay and low-level compiler tests.
    pub fn new(inputs: &[Input<'_>], queries: &[String]) -> Self {
        let groups: Vec<_> = (0..inputs.len()).map(|i| vec![i]).collect();
        Self::batched(inputs, queries, Dependencies::new(inputs), &groups)
    }

    /// A batch completes each logical member, including members with zero rows.
    /// Storage liveness remains per member; it does not follow the batch's last consumer.
    pub fn batched(
        inputs: &[Input<'_>],
        queries: &[String],
        graph: Dependencies<'_>,
        groups: &[Vec<usize>],
    ) -> Self {
        static IDENTIFIER: LazyLock<regex::Regex> = LazyLock::new(|| {
            regex::Regex::new(r"[A-Za-z_][A-Za-z_0-9]*").expect("generated identifier")
        });
        let Dependencies {
            prerequisites: dependencies,
            consumers,
            names,
            writers,
        } = graph;
        let results: Vec<Vec<_>> = queries
            .iter()
            .map(|sql| {
                IDENTIFIER
                    .find_iter(sql)
                    .flat_map(|token| {
                        names
                            .get(token.as_str())
                            .copied()
                            .into_iter()
                            .chain(writers.get(token.as_str()).copied())
                    })
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            })
            .collect();
        let result_inputs: Vec<Vec<_>> = queries
            .iter()
            .map(|sql| {
                IDENTIFIER
                    .find_iter(sql)
                    .filter_map(|token| names.get(token.as_str()).copied())
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            })
            .collect();
        let mut actions = Vec::new();
        let mut ready = vec![false; inputs.len()];
        let mut owner = vec![0; inputs.len()];
        for (g, members) in groups.iter().enumerate() {
            for &i in members {
                owner[i] = g;
            }
        }
        let prerequisites: Vec<Vec<_>> = groups
            .iter()
            .map(|members| {
                members
                    .iter()
                    .flat_map(|&i| dependencies[i].iter().copied())
                    .map(|i| owner[i])
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            })
            .collect();
        fn visit(
            index: usize,
            deps: &[Vec<usize>],
            groups: &[Vec<usize>],
            ready: &mut [bool],
            actions: &mut Vec<Action>,
        ) {
            let mut pending = vec![(index, false)];
            while let Some((g, expanded)) = pending.pop() {
                if ready[g] {
                    continue;
                }
                if expanded {
                    ready[g] = true;
                    actions.push(Action::Prepare(groups[g][0]));
                } else {
                    pending.push((g, true));
                    pending.extend(deps[g].iter().rev().map(|&n| (n, false)));
                }
            }
        }
        ready.resize(groups.len(), false);
        for (q, needs) in results.iter().enumerate() {
            for &i in needs {
                visit(owner[i], &prerequisites, groups, &mut ready, &mut actions);
            }
            actions.push(Action::Query(q));
        }
        for i in 0..groups.len() {
            visit(i, &prerequisites, groups, &mut ready, &mut actions);
        }
        let mut last = vec![0; inputs.len()];
        for (position, action) in actions.iter().enumerate() {
            let needs = match *action {
                Action::Prepare(i) => {
                    for &member in &groups[owner[i]] {
                        last[member] = position;
                        for &source in &consumers[member] {
                            last[source] = position;
                        }
                    }
                    continue;
                }
                Action::Query(q) => &result_inputs[q],
                Action::Release(_) => unreachable!(),
            };
            for &i in needs {
                last[i] = position;
            }
        }
        let mut releases = vec![Vec::new(); actions.len()];
        for (i, input) in inputs.iter().enumerate() {
            if input.temporary {
                releases[last[i]].push(i);
            }
        }
        let mut scheduled = Vec::with_capacity(actions.len() + names.len());
        let (mut live, mut peak_relations) = (0usize, 0usize);
        for (position, action) in actions.into_iter().enumerate() {
            if let Action::Prepare(i) = action {
                live += groups[owner[i]]
                    .iter()
                    .filter(|&&j| inputs[j].temporary)
                    .count();
            }
            peak_relations = peak_relations.max(live);
            scheduled.push(action);
            for &i in releases[position].iter().rev() {
                scheduled.push(Action::Release(i));
                live -= 1;
            }
        }
        Self {
            actions: scheduled,
            peak_relations,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Action::*, *};

    #[test]
    fn releases_intermediates_and_delays_later_packets_without_rebuilding_shared_inputs() {
        let inputs = [
            Input {
                name: "seed",
                sql: "CREATE TEMP TABLE seed AS SELECT 1",
                temporary: true,
            },
            Input {
                name: "early",
                sql: "CREATE TEMP TABLE early AS SELECT * FROM seed",
                temporary: true,
            },
            Input {
                name: "late",
                sql: "CREATE TEMP TABLE late AS SELECT * FROM seed",
                temporary: true,
            },
            Input {
                name: "independent",
                sql: "CREATE TEMP TABLE independent AS SELECT 2",
                temporary: true,
            },
        ];
        let schedule = Schedule::new(
            &inputs,
            &[
                "SELECT * FROM early".into(),
                "SELECT * FROM late JOIN independent".into(),
            ],
        );
        assert_eq!(
            schedule.actions,
            [
                Prepare(0),
                Prepare(1),
                Query(0),
                Release(1),
                Prepare(2),
                Release(0),
                Prepare(3),
                Query(1),
                Release(3),
                Release(2)
            ]
        );
        assert_eq!(schedule.peak_relations, 2);
    }

    #[test]
    fn retained_property_writes_preserve_read_write_and_write_read_hazards() {
        let inputs = [
            Input {
                name: "fill",
                sql: "INSERT INTO properties VALUES(1)",
                temporary: false,
            },
            Input {
                name: "snapshot",
                sql: "CREATE TEMP TABLE snapshot AS SELECT * FROM properties",
                temporary: true,
            },
            Input {
                name: "fill_later",
                sql: "WITH wanted(x) AS (VALUES(2)) INSERT OR IGNORE INTO properties SELECT * FROM wanted",
                temporary: false,
            },
        ];
        let schedule = Schedule::new(
            &inputs,
            &["SELECT * FROM properties UNION ALL SELECT * FROM snapshot".into()],
        );
        assert_eq!(
            schedule.actions,
            [Prepare(0), Prepare(1), Prepare(2), Query(0), Release(1)]
        );
    }

    #[test]
    fn executes_unreferenced_side_effects_and_releases_their_temporary_inputs() {
        let inputs = [
            Input {
                name: "scope",
                sql: "CREATE TEMP TABLE scope AS SELECT 1",
                temporary: true,
            },
            Input {
                name: "fill",
                sql: "INSERT INTO properties SELECT * FROM scope",
                temporary: false,
            },
            Input {
                name: "clear",
                sql: "DELETE FROM pending",
                temporary: false,
            },
        ];
        let schedule = Schedule::new(&inputs, &["SELECT 1".into()]);
        assert_eq!(
            schedule.actions,
            [Query(0), Prepare(0), Prepare(1), Release(0), Prepare(2)]
        );
    }

    #[test]
    fn write_ordering_does_not_extend_an_unused_snapshot_storage_lifetime() {
        let inputs = [
            Input {
                name: "fill",
                sql: "INSERT INTO properties VALUES(1)",
                temporary: false,
            },
            Input {
                name: "snapshot",
                sql: "CREATE TEMP TABLE snapshot AS SELECT * FROM properties",
                temporary: true,
            },
            Input {
                name: "later",
                sql: "CREATE TEMP TABLE later AS SELECT 2",
                temporary: true,
            },
            Input {
                name: "fill_later",
                sql: "INSERT INTO properties SELECT * FROM later",
                temporary: false,
            },
        ];
        let schedule = Schedule::new(&inputs, &["SELECT * FROM properties".into()]);
        assert_eq!(
            schedule.actions,
            [
                Prepare(0),
                Prepare(1),
                Release(1),
                Prepare(2),
                Prepare(3),
                Release(2),
                Query(0)
            ]
        );
        assert_eq!(schedule.peak_relations, 1);
    }
}
