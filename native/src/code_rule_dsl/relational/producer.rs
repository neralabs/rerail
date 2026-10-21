//! Broad CPU producer passes. SQL publication is a separate completed-output consumer.
use super::{Result, invalid, quote, source, sql_error};
use rusqlite::{Connection, types::Value};
use std::{
    cell::Cell,
    sync::{Arc, atomic::AtomicUsize},
};
#[cfg(feature = "code-rule-profile")]
mod completed;
const MAX_STAGES: usize = 100;
const MAX_OUTPUT_BYTES: usize = 256 * 1024 * 1024;
thread_local! { static READERS: Cell<usize> = const { Cell::new(0) }; }
pub(super) struct ReadOnly;
impl ReadOnly {
    pub(super) fn enter() -> Self {
        READERS.set(READERS.get() + 1);
        Self
    }
}
impl Drop for ReadOnly {
    fn drop(&mut self) {
        READERS.set(READERS.get() - 1);
    }
}
#[derive(Default)]
pub(super) struct Allowance {
    used: usize,
    resident: Resident,
}
impl Allowance {
    fn reserve(&mut self, n: usize) -> Result<()> {
        if READERS.get() != 0 {
            return Err(invalid("producer execution inside SQL callback"));
        }
        self.used = self
            .used
            .checked_add(n)
            .ok_or_else(|| invalid("producer stage limit"))?;
        if self.used > MAX_STAGES {
            return Err(invalid("producer stage limit"));
        }
        Ok(())
    }
}
/// All offsets are checked scalar indices. No source, path, or host pointer is a request field.
#[repr(C)]
#[derive(Clone, Copy)]
pub(super) struct Root {
    pub file: u32,
    pub node: u32,
}
#[derive(Clone)]
pub(super) struct Tree {
    pub facts: Arc<super::syntax::Facts>,
    pub visible: Arc<Vec<u32>>,
}
pub(super) struct Demand {
    pub roots: Vec<Root>,
    pub trees: Vec<Tree>,
    pub paths: Vec<String>,
    pub work: Arc<AtomicUsize>,
}
struct Output {
    roots: Vec<u32>,
    nodes: Vec<u32>,
    files: Vec<u32>,
}
/// Constructed only after all passes succeed for this relation.
pub(super) struct Completed {
    output: Output,
    paths: Vec<String>,
}
impl Completed {
    pub fn publish(self, db: &Connection, name: &str, work: &Arc<AtomicUsize>) -> Result<()> {
        #[cfg(feature = "code-rule-profile")]
        let _span = crate::code_rule_dsl::profile::trace::Span::start("producer-publish", name);
        #[cfg(feature = "code-rule-profile")]
        if std::env::var("RERAIL_SQL_COMPLETED_OUTPUT").is_ok_and(|v| v == "virtual") {
            return completed::publish(self, db, name, Arc::clone(work));
        }
        db.execute_batch(&format!(
            "CREATE TEMP TABLE {} (__path TEXT,root,node)",
            quote(name)
        ))
        .map_err(sql_error)?;
        let Self { output, paths } = self;
        super::insert::stage_rows(
            db,
            name,
            3,
            output
                .roots
                .into_iter()
                .zip(output.nodes)
                .zip(output.files)
                .map(|((root, node), file)| {
                    [
                        Value::Text(paths[file as usize].clone()),
                        Value::Integer(i64::from(root)),
                        Value::Integer(i64::from(node)),
                    ]
                }),
            work,
        )?;
        db.execute_batch(&format!("CREATE INDEX {} ON {}(__path,root,node);CREATE INDEX {} ON {}(__path,node,root);ANALYZE temp.{}",quote(&format!("__{name}_forward")),quote(name),quote(&format!("__{name}_reverse")),quote(name),quote(name))).map_err(sql_error)
    }
}
/// Checked, flat request columns. Relation boundaries are numeric CSR offsets.
#[derive(Default)]
struct Resident {
    ends: Vec<u32>,
    visible: Vec<u32>,
    files: Vec<[u32; 4]>,
    roots: Vec<Root>,
    relations: Vec<usize>,
    documents: std::collections::HashMap<usize, (u32, u32)>,
    selectors: std::collections::HashMap<(usize, usize), u32>,
    retained: Vec<Tree>,
    resident_bytes: usize,
}
const MAX_RESIDENT_BYTES: usize = 64 * 1024 * 1024;
fn capacity(bytes: Option<usize>, limit: usize) -> Result<usize> {
    bytes
        .filter(|&n| n <= limit)
        .ok_or_else(|| invalid("producer capacity exceeded"))
}
impl Resident {
    fn prepare(&mut self, demands: &[Demand]) -> Result<()> {
        self.roots.clear();
        self.relations.clear();
        self.relations.push(0);
        let r = self;
        let mut bytes = r.resident_bytes;
        for d in demands {
            let mut mapping = Vec::new();
            for tree in &d.trees {
                let document = Arc::as_ptr(&tree.facts) as usize;
                let key = (document, Arc::as_ptr(&tree.visible) as usize);
                let file = if let Some(&id) = r.selectors.get(&key) {
                    id
                } else {
                    let (offset, len) = if let Some(&range) = r.documents.get(&document) {
                        range
                    } else {
                        let ends = tree.facts.subtree_ends();
                        bytes = capacity(
                            bytes.checked_add(
                                ends.len()
                                    .checked_mul(4)
                                    .ok_or_else(|| invalid("producer capacity"))?,
                            ),
                            MAX_RESIDENT_BYTES,
                        )?;
                        let offset = u32::try_from(r.ends.len())
                            .map_err(|_| invalid("producer node capacity"))?;
                        let len = u32::try_from(ends.len())
                            .map_err(|_| invalid("producer node capacity"))?;
                        r.ends.extend_from_slice(ends);
                        r.documents.insert(document, (offset, len));
                        r.resident_bytes += ends.len() * 4;
                        (offset, len)
                    };
                    bytes = capacity(
                        bytes.checked_add(
                            tree.visible
                                .len()
                                .checked_mul(4)
                                .and_then(|n| n.checked_add(16))
                                .ok_or_else(|| invalid("producer capacity"))?,
                        ),
                        MAX_RESIDENT_BYTES,
                    )?;
                    let start = u32::try_from(r.visible.len())
                        .map_err(|_| invalid("producer selector capacity"))?;
                    let count = u32::try_from(tree.visible.len())
                        .map_err(|_| invalid("producer selector capacity"))?;
                    let id = u32::try_from(r.files.len())
                        .map_err(|_| invalid("producer file capacity"))?;
                    r.files.push([offset, len, start, count]);
                    r.visible.extend_from_slice(&tree.visible);
                    r.selectors.insert(key, id);
                    r.retained.push(tree.clone());
                    r.resident_bytes += tree.visible.len() * 4 + 16;
                    id
                };
                mapping.push(file);
            }
            bytes = capacity(
                bytes.checked_add(
                    d.roots
                        .len()
                        .checked_mul(24)
                        .ok_or_else(|| invalid("producer request capacity"))?,
                ),
                MAX_RESIDENT_BYTES,
            )?;
            for root in &d.roots {
                r.roots.push(Root {
                    file: *mapping
                        .get(root.file as usize)
                        .ok_or_else(|| invalid("producer file identity"))?,
                    node: root.node,
                });
            }
            r.relations.push(r.roots.len());
        }
        Ok(())
    }
    fn range(&self, root: Root) -> Result<std::ops::Range<usize>> {
        let &[offset, len, start, count] = self
            .files
            .get(root.file as usize)
            .ok_or_else(|| invalid("producer file identity"))?;
        if root.node >= len {
            return Err(invalid("invalid syntax root"));
        }
        let end = self.ends[offset as usize + root.node as usize];
        let visible = &self.visible[start as usize..start as usize + count as usize];
        let a = visible.partition_point(|&id| id < root.node);
        let b = visible.partition_point(|&id| id < end);
        Ok(start as usize + a..start as usize + b)
    }
}
pub(super) fn execute(
    pending: Vec<(usize, Demand)>,
    contexts: &mut [crate::code_rule_dsl::budget::Context],
    allowance: &mut Allowance,
) -> Vec<Result<Completed>> {
    #[cfg(feature = "code-rule-profile")]
    let mut wave = crate::code_rule_dsl::profile::trace::Span::start("producer-wave", "subtrees");
    let (indices, demands): (Vec<_>, Vec<_>) = pending.into_iter().unzip();
    let n = demands.len();
    let before = allowance.resident.files.len();
    if let Err(e) = allowance
        .reserve(3)
        .and_then(|()| allowance.resident.prepare(&demands))
    {
        return (0..n).map(|_| Err(invalid(&e.reason))).collect();
    }
    let resident = &allowance.resident;
    #[cfg(not(feature = "code-rule-profile"))]
    let _ = before;
    let mut errors: Vec<Option<String>> = (0..n).map(|_| None).collect();
    let mut begins = vec![0u32; resident.roots.len()];
    let mut counts = vec![0u32; resident.roots.len()];
    let mut offsets = vec![0u32; resident.roots.len()];
    let mut totals = vec![0usize; n];
    {
        #[cfg(feature = "code-rule-profile")]
        let _pass = crate::code_rule_dsl::profile::trace::Span::start("producer-stage", "count");
        for i in 0..n {
            let result = contexts[indices[i]].run(|| -> Result<()> {
                for j in resident.relations[i]..resident.relations[i + 1] {
                    let range = resident.range(resident.roots[j])?;
                    source::charge(&demands[i].work, range.len().div_ceil(64))?;
                    begins[j] = u32::try_from(range.start)
                        .map_err(|_| invalid("producer offset capacity"))?;
                    counts[j] = u32::try_from(range.len())
                        .map_err(|_| invalid("producer count capacity"))?;
                }
                Ok(())
            });
            if let Err(e) = result {
                errors[i] = Some(e.reason);
            }
        }
    }
    {
        #[cfg(feature = "code-rule-profile")]
        let _pass = crate::code_rule_dsl::profile::trace::Span::start("producer-stage", "scan");
        let mut bytes = 0usize;
        for i in 0..n {
            if errors[i].is_some() {
                continue;
            }
            for j in resident.relations[i]..resident.relations[i + 1] {
                offsets[j] = totals[i] as u32;
                let next = totals[i].checked_add(counts[j] as usize);
                if let Some(next) =
                    next.filter(|n| n.checked_mul(12).is_some_and(|b| b <= MAX_OUTPUT_BYTES))
                {
                    totals[i] = next;
                } else {
                    errors[i] = Some("producer output capacity".to_owned());
                    break;
                }
            }
            if errors[i].is_none() {
                bytes = bytes.saturating_add(totals[i] * 12);
            }
        }
        if bytes > MAX_OUTPUT_BYTES {
            for error in &mut errors {
                *error = Some("producer wave capacity".to_owned());
            }
        }
    }
    let mut outputs = Vec::new();
    {
        #[cfg(feature = "code-rule-profile")]
        let _pass = crate::code_rule_dsl::profile::trace::Span::start("producer-stage", "scatter");
        for (i, demand) in demands.into_iter().enumerate() {
            if let Some(error) = errors[i].take() {
                outputs.push(Err(invalid(error)));
                continue;
            }
            let mut output = Output {
                roots: vec![0; totals[i]],
                nodes: vec![0; totals[i]],
                files: vec![0; totals[i]],
            };
            let result = contexts[indices[i]].run(|| -> Result<()> {
                for (local, j) in (resident.relations[i]..resident.relations[i + 1]).enumerate() {
                    source::charge(&demand.work, 0)?;
                    let out = offsets[j] as usize..offsets[j] as usize + counts[j] as usize;
                    let range = begins[j] as usize..begins[j] as usize + counts[j] as usize;
                    output.nodes[out.clone()].copy_from_slice(&resident.visible[range]);
                    output.roots[out.clone()].fill(demand.roots[local].node);
                    output.files[out].fill(demand.roots[local].file);
                }
                Ok(())
            });
            outputs.push(result.map(|()| Completed {
                output,
                paths: demand.paths,
            }));
        }
    }
    #[cfg(feature="code-rule-profile")]
    wave.detail(||serde_json::json!({"relations":n,"roots":resident.roots.len(),"documents":resident.files.len(),"inputDocumentsAdded":resident.files.len()-before,"stages":3,"backend":"cpu"}));
    outputs
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_cannot_reserve_producer_work() {
        let mut allowance = Allowance::default();
        {
            let _reader = ReadOnly::enter();
            assert!(allowance.reserve(3).is_err());
            assert_eq!(allowance.used, 0);
        }
        assert!(allowance.reserve(3).is_ok());
    }
    #[test]
    fn waves_share_a_non_resettable_stage_allowance() {
        let mut a = Allowance::default();
        for _ in 0..33 {
            a.reserve(3).unwrap();
        }
        assert!(a.reserve(3).is_err());
        assert!(a.reserve(1).is_err());
    }
    #[test]
    fn capacity_checks_precede_allocation() {
        assert_eq!(capacity(Some(64), 64).unwrap(), 64);
        assert!(capacity(Some(65), 64).is_err());
        assert!(capacity(None, 64).is_err());
    }
}
