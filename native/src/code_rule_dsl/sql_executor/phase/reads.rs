//! Read identical materialized projections once, then distribute their rows to publication targets.
use super::*;

struct Read {
    width: usize,
    first: (usize, bool),
    more: Vec<(usize, bool)>,
}
pub(super) struct Plan {
    pub arms: Vec<String>,
    pub logical_reads: usize,
    reads: Vec<Read>,
}
impl Plan {
    pub fn new(phase: &Phase, width: usize) -> Self {
        let mut plan = Self {
            arms: vec![],
            logical_reads: 0,
            reads: vec![],
        };
        let mut shared = BTreeMap::<(&str, usize), usize>::new();
        let mut projections = BTreeMap::<usize, String>::new();
        for (id, output) in phase.outputs.iter().enumerate() {
            let keys = match &output.keys {
                Keys::Query(keys) => Some((keys.as_str(), output.key_width, true)),
                _ => None,
            };
            for (source, columns, keys) in
                std::iter::once((output.source.as_str(), output.width, false)).chain(keys)
            {
                plan.logical_reads += 1;
                let materialized = source
                    .strip_prefix("__rr_phase_")
                    .filter(|n| {
                        n.bytes().all(|b| b.is_ascii_digit())
                            && (n.len() == 1 || !n.starts_with('0'))
                    })
                    .and_then(|n| n.parse::<usize>().ok())
                    .and_then(|n| phase.definitions.get(n))
                    .is_some_and(|d| d.materialized);
                let existing = materialized
                    .then(|| shared.get(&(source, columns)))
                    .flatten()
                    .copied();
                if let Some(at) = existing {
                    plan.reads[at].more.push((id, keys));
                    continue;
                }
                let at = plan.reads.len();
                let fields = projections.entry(columns).or_insert_with(|| {
                    (0..width)
                        .map(|c| {
                            if c < columns {
                                format!("c{c}")
                            } else {
                                "NULL".into()
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(",")
                });
                plan.arms
                    .push(format!("SELECT {at},{fields} FROM {source}"));
                plan.reads.push(Read {
                    width: columns,
                    first: (id, keys),
                    more: vec![],
                });
                if materialized {
                    shared.insert((source, columns), at);
                }
            }
        }
        plan
    }
    pub fn collect(
        &self,
        row: &rusqlite::Row<'_>,
        rows: &mut [Vec<Vec<Option<i64>>>],
        keys: &mut [Vec<Vec<Option<i64>>>],
    ) -> Result<()> {
        let tag = row.get::<_, i64>(0).map_err(error)? as usize;
        let read = self
            .reads
            .get(tag)
            .ok_or_else(|| error("invalid SQL phase output tag"))?;
        let mut cells = (1..=read.width)
            .map(|c| row.get(c))
            .collect::<rusqlite::Result<Vec<Option<i64>>>>()
            .map_err(error)?;
        for (at, &(id, key)) in std::iter::once(&read.first).chain(&read.more).enumerate() {
            let target = if key { &mut keys[id] } else { &mut rows[id] };
            target.push(if at == read.more.len() {
                std::mem::take(&mut cells)
            } else {
                cells.clone()
            });
        }
        Ok(())
    }
}
