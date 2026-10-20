//! Progressive input-set pattern walks. One virtual-table call advances all unresolved roots.
//! The compiler admits only total, nonrecursive pattern/kind/regex bodies and fixed tree stops.
//! General unknowns and recursive dependencies remain in the relational driver.
use super::{
    PatternTerm, Terminal,
    pattern::{PatternRow, typed_match},
};
use crate::code_rule_dsl::matcher::node_at;
use crate::code_rule_dsl::sql::{exec::Batch, nodes::NONE};
use rusqlite::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

#[cfg(test)]
thread_local! { pub(in crate::code_rule_dsl::sql) static PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[derive(Serialize, Deserialize)]
pub(in crate::code_rule_dsl::sql) struct Scan {
    pub walk: u8,
    pub kinds: Option<Vec<u16>>,
    pub regexes: Vec<usize>,
    /// None: first match; Some: min, max, exact output count required.
    pub count: Option<(usize, Option<usize>, bool)>,
}
struct Cursor {
    key: i64,
    file: u32,
    next: u32,
    last: u32,
    inputs: Vec<Option<i64>>,
    count: usize,
    done: bool,
    posted: Option<(Arc<[u32]>, usize, usize)>,
}

type Input = (i64, u32, u32, Vec<Option<i64>>);

pub(super) fn run(
    batch: &Batch,
    term: &PatternTerm,
    scan: &Scan,
    input: Vec<Input>,
) -> Result<Vec<PatternRow>> {
    let mut cursors = Vec::with_capacity(input.len());
    let mut files = HashMap::<u32, Arc<[u32]>>::new();
    for (key, file, node, inputs) in input {
        let Some(entry) = batch.files.get(file as usize) else {
            continue;
        };
        if entry.dialect != term.dialect || node >= entry.facts.len() {
            continue;
        }
        let f = &entry.facts;
        let next = match scan.walk {
            0 => {
                if node < f.last(node) {
                    node + 1
                } else {
                    NONE
                }
            }
            1 => f.first(node),
            2 | 3 => f.parent(node),
            4 => node_at(&entry.root, node)
                .next_all()
                .next()
                .map_or(NONE, |n| n.node_id() as u32),
            5 => node_at(&entry.root, node)
                .next()
                .map_or(NONE, |n| n.node_id() as u32),
            6 => node_at(&entry.root, node)
                .prev_all()
                .next()
                .map_or(NONE, |n| n.node_id() as u32),
            7 => f.prev(node),
            _ => return Err(Error::ModuleError("scan: invalid walk".into())),
        };
        let posted = if scan.walk == 0 {
            scan.kinds.as_ref().map(|kinds| {
                let doc = entry.root.root().get_doc();
                let bound = term
                    .root_capture
                    .then(|| inputs.first().copied().flatten())
                    .flatten()
                    .and_then(|id| u32::try_from(id).ok())
                    .filter(|&id| id < f.len())
                    .filter(|&id| {
                        doc.transducer_children(id)
                            .all(|child| doc.transducer_flags(child) & 1 == 0)
                    });
                let ids = match (kinds.as_slice(), bound) {
                    ([kind], Some(bound)) => f
                        .value_nodes(*kind, f.thash(bound))
                        .unwrap_or_else(|| Arc::from([])),
                    _ => Arc::clone(
                        files
                            .entry(file)
                            .or_insert_with(|| f.kind_ids(kinds).into()),
                    ),
                };
                let start = ids.partition_point(|&id| id <= node);
                let end = ids.partition_point(|&id| id <= f.last(node));
                (ids, start, end)
            })
        } else {
            None
        };
        let next = posted.as_ref().map_or(
            next,
            |(ids, start, end)| if start < end { ids[*start] } else { NONE },
        );
        cursors.push(Cursor {
            key,
            file,
            next,
            last: f.last(node),
            inputs,
            count: 0,
            done: false,
            posted,
        });
    }
    let mut rows = Vec::new();
    let mut work = 0usize;
    let mut active: Vec<usize> = (0..cursors.len()).collect();
    while !active.is_empty() {
        for &index in &active {
            let c = &mut cursors[index];
            if matches!(scan.count, Some((0, None, false))) {
                rows.push(PatternRow::answer(c.file, 0, c.key, Arc::from([Some(0)])));
                c.done = true;
                continue;
            }
            // Keep each ordered prefix between phases; never rebuild a candidate frame.
            for _ in 0..32 {
                if c.next == NONE {
                    break;
                }
                work += 1;
                #[cfg(test)]
                PROBES.set(PROBES.get() + 1);
                if work > super::super::frames::BUDGET {
                    return Err(Error::ModuleError("scan: work budget exceeded".into()));
                }
                let node = c.next;
                let entry = &batch.files[c.file as usize];
                let f = &entry.facts;
                c.next = if let Some((ids, at, end)) = &mut c.posted {
                    *at += 1;
                    if *at < *end { ids[*at] } else { NONE }
                } else {
                    match scan.walk {
                        0 => {
                            if node < c.last {
                                node + 1
                            } else {
                                NONE
                            }
                        }
                        1 => f.nsib(node),
                        2 => f.parent(node),
                        4 => f.nsib(node),
                        6 => f.prev(node),
                        _ => NONE,
                    }
                };
                if scan
                    .kinds
                    .as_ref()
                    .is_some_and(|ks| !ks.contains(&f.kind(node)))
                {
                    continue;
                }
                let doc = entry.root.root().get_doc();
                if !scan.regexes.iter().all(|&id| matches!(&batch.program.terminals[id], Terminal::Regex(r) if r.is_match(doc.transducer_text_str(node)))) { continue }
                let Some(values) = typed_match(batch, term, c.file, node, &c.inputs) else {
                    continue;
                };
                let Some((min, max, exact)) = scan.count else {
                    rows.push(PatternRow::answer(c.file, node, c.key, values));
                    c.done = true;
                    break;
                };
                c.count += 1;
                if max.is_some_and(|max| c.count > max) {
                    c.done = true;
                    break;
                }
                if !exact && max.is_none() && c.count >= min {
                    rows.push(PatternRow::answer(
                        c.file,
                        node,
                        c.key,
                        Arc::from([Some(c.count as i64)]),
                    ));
                    c.done = true;
                    break;
                }
            }
            if !c.done && c.next == NONE {
                if let Some((min, max, _)) = scan.count
                    && c.count >= min
                    && max.is_none_or(|max| c.count <= max)
                {
                    rows.push(PatternRow::answer(
                        c.file,
                        0,
                        c.key,
                        Arc::from([Some(c.count as i64)]),
                    ));
                }
                c.done = true;
            }
        }
        active.retain(|&index| !cursors[index].done);
    }
    Ok(rows)
}
