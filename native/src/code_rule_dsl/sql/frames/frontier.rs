//! Candidate phases over shared frame pages. The driver owns one cursor per loop invocation;
//! nested loops and later recursive passes cannot inherit a previous invocation's progress.

use super::*;

struct Parent {
    start: usize,
    end: usize,
    limit: usize,
    done: bool,
}

/// Stable source row IDs and per-parent progress; all captures remain in the source pages.
/// A phase replays its whole prefix so deferred lineage sees one consistent final evaluation.
pub(in crate::code_rule_dsl::sql) struct Frontier {
    source: Arc<Shared>,
    target: Arc<Shared>,
    matched: Arc<Shared>,
    opened: Option<Arc<Shared>>,
    parents: Vec<Parent>,
    parent_of: HashMap<i64, usize>,
    order: Vec<u32>,
    group: Vec<usize>,
    mode: String,
    three: bool,
    min: usize,
    max: Option<usize>,
}

impl FrameStore {
    pub(in crate::code_rule_dsl::sql) fn frontier(&self, description: &str) -> Result<Frontier> {
        let parts: Vec<_> = description.split_whitespace().collect();
        if parts.len() != 8 {
            return Err(Error::ModuleError(
                "frame: invalid frontier descriptor".into(),
            ));
        }
        let frames = self.frames.lock();
        let frame = |name: &str| {
            frames
                .get(name)
                .cloned()
                .ok_or_else(|| Error::ModuleError(format!("frame: frontier needs {name}")))
        };
        let source = frame(parts[0])?;
        let target = frame(parts[1])?;
        let matched = frame(parts[2])?;
        let opened = if parts[3] == "-" {
            None
        } else {
            Some(frame(parts[3])?)
        };
        let input = source.get();
        let pk = input
            .names
            .iter()
            .position(|n| n == "pk")
            .ok_or_else(|| Error::ModuleError("frame: frontier needs parent keys".into()))?;
        let ord =
            input.names.iter().position(|n| n == "ord").ok_or_else(|| {
                Error::ModuleError("frame: frontier needs candidate order".into())
            })?;
        let mut order: Vec<u32> = (0..input.len as u32).collect();
        order.sort_unstable_by_key(|&r| {
            (
                input.columns[pk].integer(r as usize),
                input.columns[ord].integer(r as usize),
                input.columns[0].integer(r as usize),
            )
        });
        let mut parents = Vec::<Parent>::new();
        let mut parent_of = HashMap::new();
        let mut group = vec![0; input.len];
        let mut previous = None;
        for (at, &row) in order.iter().enumerate() {
            let key = input.columns[pk].integer(row as usize);
            if previous != Some(key) {
                parent_of.insert(key, parents.len());
                parents.push(Parent {
                    start: at,
                    end: at,
                    limit: 0,
                    done: false,
                });
                previous = Some(key);
            }
            let index = parents.len() - 1;
            parents[index].end = at + 1;
            group[row as usize] = index;
        }
        Ok(Frontier {
            source,
            target,
            matched,
            opened,
            parents,
            parent_of,
            order,
            group,
            mode: parts[4].to_owned(),
            three: parts[5] == "1",
            min: parts[6]
                .parse()
                .map_err(|_| Error::ModuleError("frame: invalid frontier minimum".into()))?,
            max: if parts[7] == "-" {
                None
            } else {
                Some(
                    parts[7].parse().map_err(|_| {
                        Error::ModuleError("frame: invalid frontier maximum".into())
                    })?,
                )
            },
        })
    }

    /// Publish a prefix without SQL ranking, key probes or per-cell xUpdate calls. The target
    /// receives immutable views; on the retained-page limit it receives ordinary owned columns.
    pub(in crate::code_rule_dsl::sql) fn frontier_fill(
        &self,
        frontier: &mut Frontier,
    ) -> Result<usize> {
        let mut rows = Vec::new();
        for parent in &mut frontier.parents {
            if !parent.done {
                let total = parent.end - parent.start;
                parent.limit = if parent.limit == 0 {
                    total.min(8)
                } else if total <= parent.limit.saturating_mul(4) {
                    // Skip a replay of an almost complete domain before its final phase.
                    total
                } else {
                    parent.limit.saturating_mul(2)
                };
            }
            parent.done |= parent.start + parent.limit == parent.end;
        }
        if frontier
            .parents
            .iter()
            .all(|parent| parent.start + parent.limit == parent.end)
        {
            rows.extend(0..frontier.order.len() as u32);
        } else {
            for parent in &frontier.parents {
                rows.extend_from_slice(&frontier.order[parent.start..parent.start + parent.limit]);
            }
            rows.sort_unstable();
        }
        self.publish_rows(&frontier.source, &frontier.target, rows.into())
    }

    fn publish_rows(
        &self,
        source: &Arc<Shared>,
        target: &Arc<Shared>,
        rows: Arc<[u32]>,
    ) -> Result<usize> {
        if Arc::ptr_eq(source, target) {
            return Err(Error::ModuleError(
                "frame: frontier aliases its source".into(),
            ));
        }
        // SAFETY: the driver calls this between statements, with no live SQLite cursor.
        let output = unsafe { target.get_mut() };
        self.live.fetch_sub(output.len, Atomic::Relaxed);
        output.clear();
        if rows.is_empty() {
            return Ok(0);
        }
        if self.live.load(Atomic::Relaxed).saturating_add(rows.len()) > BUDGET {
            return Err(Error::ModuleError("frame: row budget exceeded".into()));
        }
        let input = unsafe { source.get_mut() };
        if input.names != output.names || rows.iter().any(|&r| r as usize >= input.len) {
            return Err(Error::ModuleError(
                "frame: incompatible frontier projection".into(),
            ));
        }
        let selected = (rows.len() != input.len).then_some(&rows);
        let mut composed = HashMap::new();
        let mut projected = Vec::with_capacity(input.columns.len());
        for column in &mut input.columns {
            projected.push(
                column
                    .view(selected, &self.retained, &mut composed)
                    .unwrap_or_else(|| {
                        let mut copy = Column::new(column.integer, column.default);
                        for (at, &row) in rows.iter().enumerate() {
                            copy.push(at, column.get(row as usize));
                        }
                        copy
                    }),
            );
        }
        output.columns = projected;
        output.len = rows.len();
        output.ascending = rows.windows(2).all(|p| {
            input.columns[0].integer(p[0] as usize) < input.columns[0].integer(p[1] as usize)
        });
        output.dense = rows.windows(2).all(|p| {
            input.columns[0]
                .integer(p[1] as usize)
                .checked_sub(input.columns[0].integer(p[0] as usize))
                == Some(1)
        });
        output.largest = rows
            .iter()
            .map(|&r| input.columns[0].integer(r as usize))
            .max()
            .unwrap_or(0);
        let live = self.live.fetch_add(rows.len(), Atomic::Relaxed) + rows.len();
        self.peak.fetch_max(live, Atomic::Relaxed);
        Ok(rows.len())
    }
}

impl Frontier {
    /// Mark decisive parents and report whether another phase is needed. Unknown events close
    /// a strict search; non-strict searches need a true event. Count bounds use true rows only.
    pub(in crate::code_rule_dsl::sql) fn advance(&mut self) -> Result<bool> {
        if self.parents.iter().all(|parent| parent.done) {
            return Ok(false);
        }
        let input = self.source.get();
        let matched = self.matched.get();
        let unk = matched.names.iter().position(|n| n == "unk");
        let ord = input.names.iter().position(|n| n == "ord").unwrap();
        let mut yes = vec![0usize; self.parents.len()];
        let lookup = if input.ascending {
            None
        } else {
            Some(input.lookup(&[0])?)
        };
        for row in 0..matched.len {
            let unknown = self.three && unk.is_some_and(|c| matched.columns[c].integer(row) != 0);
            if unknown && self.mode != "strict" {
                continue;
            }
            let key = matched.columns[0].integer(row);
            let at = if input.dense && input.ascending && input.len > 0 {
                usize::try_from(key.wrapping_sub(input.columns[0].integer(0))).unwrap_or(input.len)
            } else if let Some(lookup) = &lookup {
                let at = lookup.partition_point(|&r| input.columns[0].integer(r as usize) < key);
                lookup.get(at).map_or(input.len, |&r| r as usize)
            } else {
                partition(input.len, |r| input.columns[0].integer(r) < key)
            };
            if at >= input.len || input.columns[0].integer(at) != key {
                return Err(Error::ModuleError(
                    "frame: frontier answer has an invalid candidate key".into(),
                ));
            }
            yes[self.group[at]] += 1;
        }
        for (index, parent) in self.parents.iter_mut().enumerate() {
            let count = yes[index];
            let decided = if self.mode == "count" {
                self.max.map_or(count >= self.min, |max| count > max)
            } else {
                count > 0
            };
            parent.done |= parent.start + parent.limit == parent.end || decided;
        }
        if self.mode == "strict"
            && let Some(opened) = &self.opened
        {
            let opened = opened.get();
            for row in 0..opened.len {
                let pk = opened.columns[0].integer(row);
                if let Some(&index) = self.parent_of.get(&pk) {
                    let parent = &mut self.parents[index];
                    if parent.limit > 0 {
                        let last = self.order[parent.start + parent.limit - 1] as usize;
                        parent.done |=
                            opened.columns[1].integer(row) <= 2 * input.columns[ord].integer(last);
                    }
                }
            }
        }
        Ok(self.parents.iter().any(|p| !p.done))
    }
}
