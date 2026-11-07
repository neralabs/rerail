//! Immutable native fact shards, reused across source edits. No AST rows are copied into SQL.
use super::super::model::FileInput;
use super::*;
use std::sync::OnceLock;

type Sources = BTreeMap<String, Arc<dyn Source>>;
#[derive(Clone)]
pub(super) struct Document {
    file: FileInput,
    key: u32,
    pub(super) sources: Sources,
}
pub(super) type Documents = BTreeMap<String, Document>;

/// Probe row ids have a tagged shard and local id. Scans use dense ids. Keeping these apart
/// preserves lazy containment sources, whose lookup ids need not be in `0..len()`.
struct Shards {
    columns: Vec<String>,
    parts: Vec<Arc<dyn Source>>,
    ends: OnceLock<Vec<usize>>,
    keys: BTreeMap<i64, usize>,
    estimates: (usize, Vec<(usize, usize, bool, bool)>),
    probes: parking_lot::Mutex<(Vec<(usize, usize)>, HashMap<(usize, usize), usize>)>,
    lazy: bool,
}
const PROBE: usize = 1usize << 31;
impl Shards {
    fn new(columns: Vec<String>, parts: Vec<Arc<dyn Source>>, keys: BTreeMap<i64, usize>) -> Self {
        let n = parts.iter().map(|p| p.estimated_len()).sum();
        let estimates = (0..columns.len())
            .map(|c| {
                let indexed = parts.iter().all(|p| p.indexed(c));
                let range = parts.iter().all(|p| p.range_indexed(c));
                let local = parts.first().is_some_and(|p| p.doc_column() == Some(c));
                let count = |batch| {
                    let values = parts.iter().map(|p| p.estimated_matches(c, batch));
                    if local {
                        values.max().unwrap_or(0)
                    } else {
                        values.sum()
                    }
                };
                (count(false), count(true), indexed, range)
            })
            .collect();
        let lazy = parts.iter().any(|p| p.made_on_lookup());
        Self {
            columns,
            parts,
            keys,
            ends: OnceLock::new(),
            estimates: (n, estimates),
            probes: parking_lot::Mutex::new((vec![], HashMap::new())),
            lazy,
        }
    }
    fn probe(&self, part: usize, row: usize) -> usize {
        // Native input consumers also keep Source ids as u32. Dense ids retain that contract;
        // lazy tree probes use a stable small handle instead of encoding a shard in high bits.
        if !self.lazy {
            return row + if part == 0 { 0 } else { self.ends()[part - 1] };
        }
        let mut ids = self.probes.lock();
        let key = (part, row);
        let id = match ids.1.get(&key) {
            Some(&id) => id,
            None => {
                let id = ids.0.len();
                ids.0.push(key);
                ids.1.insert(key, id);
                id
            }
        };
        assert!(id < PROBE);
        PROBE | id
    }
    fn probe_value(&self, r: usize, c: usize) -> i64 {
        let (part, row) = self.probes.lock().0[r & !PROBE];
        self.parts[part].value(row, c)
    }
    fn ends(&self) -> &[usize] {
        self.ends.get_or_init(|| {
            let mut n = 0;
            self.parts
                .iter()
                .map(|p| {
                    n += p.len();
                    n
                })
                .collect()
        })
    }
    fn probes(&self, f: impl Fn(&dyn Source) -> Option<Vec<usize>>) -> Option<Vec<usize>> {
        let mut rows = vec![];
        for (i, p) in self.parts.iter().enumerate() {
            rows.extend(f(p.as_ref())?.into_iter().map(|r| self.probe(i, r)));
        }
        Some(rows)
    }
}
impl Source for Shards {
    fn columns(&self) -> Vec<String> {
        self.columns.clone()
    }
    fn len(&self) -> usize {
        self.ends().last().copied().unwrap_or(0)
    }
    fn estimated_len(&self) -> usize {
        self.estimates.0
    }
    fn estimated_matches(&self, c: usize, b: bool) -> usize {
        if b {
            self.estimates.1[c].1
        } else {
            self.estimates.1[c].0
        }
    }
    fn indexed(&self, c: usize) -> bool {
        self.estimates.1[c].2
    }
    fn made_on_lookup(&self) -> bool {
        self.lazy
    }
    fn lookup(&self, c: usize, v: &[i64]) -> Option<Vec<usize>> {
        let document = self.doc_column() == Some(c);
        if document {
            let mut values = BTreeMap::<usize, Vec<i64>>::new();
            for &v in v {
                let key = v;
                if let Some(&i) = self.keys.get(&key) {
                    values.entry(i).or_default().push(v);
                }
            }
            let mut out = vec![];
            for (i, v) in values {
                out.extend(
                    self.parts[i]
                        .lookup(c, &v)?
                        .into_iter()
                        .map(|r| self.probe(i, r)),
                );
            }
            Some(out)
        } else {
            self.probes(|p| p.lookup(c, v))
        }
    }
    fn doc_column(&self) -> Option<usize> {
        self.parts.first()?.doc_column()
    }
    fn kept_to_doc(&self, c: usize) -> bool {
        self.parts.iter().all(|p| p.kept_to_doc(c))
    }
    fn lookup_in(&self, c: usize, v: &[i64], doc: i64) -> Option<Vec<usize>> {
        let Some(&i) = self.keys.get(&doc) else {
            return Some(vec![]);
        };
        Some(
            self.parts[i]
                .lookup_in(c, v, doc)?
                .into_iter()
                .map(|r| self.probe(i, r))
                .collect(),
        )
    }
    fn range_indexed(&self, c: usize) -> bool {
        self.estimates.1[c].3
    }
    fn range_lookup(
        &self,
        c: usize,
        l: Option<(i64, bool)>,
        h: Option<(i64, bool)>,
    ) -> Option<Vec<usize>> {
        self.probes(|p| p.range_lookup(c, l, h))
    }
    fn value(&self, r: usize, c: usize) -> i64 {
        if self.lazy && r & PROBE != 0 {
            self.probe_value(r, c)
        } else {
            let i = self.ends().partition_point(|&end| end <= r);
            self.parts[i].value(r - if i == 0 { 0 } else { self.ends()[i - 1] }, c)
        }
    }
}

fn native_sources(store: &ContentStore, docs: Vec<syntax::Document>) -> Result<Sources> {
    syntax::with_nearest(
        store,
        docs,
        &[],
        &Default::default(),
        super::super::typing::file_local::Binding::Cpu,
        &syntax::Projection::core(),
    )
    .map(|s| s.into_iter().collect())
    .map_err(error)
}

pub(super) fn prepare(
    store: &ContentStore,
    json: &str,
    old: Option<&Documents>,
) -> Result<(Documents, BTreeSet<String>)> {
    let (snapshot, _) = super::super::eval::parse_snapshot(json, false)?;
    let mut next = Documents::new();
    let mut changed = BTreeSet::new();
    for file in snapshot.files {
        if next.contains_key(&file.path) {
            return Err(error(format!("duplicate snapshot path: {}", file.path)));
        }
        let path = file.path.clone();
        if let Some(doc) = old
            .and_then(|old| old.get(&path))
            .filter(|d| d.file.source == file.source && d.file.language == file.language)
        {
            next.insert(path, doc.clone());
            continue;
        }
        let facts = match core::build_cached(&file, store).map_err(error)? {
            Built::Facts(facts) if !facts.syntax_errors => facts,
            _ => return Err(error(format!("{}: core facts are incomplete", path))),
        };
        let key = store.intern_atoms(&[&path]).map_err(error)?[0];
        let document = syntax::Document {
            key,
            path: path.clone(),
            core: String::new(),
            digest: [0; 32],
            facts: Arc::new(facts),
            guest: None,
            module: None,
        };
        let sources = native_sources(store, vec![document])?;
        next.insert(path.clone(), Document { file, key, sources });
        changed.insert(path);
    }
    if let Some(old) = old {
        changed.extend(old.keys().filter(|p| !next.contains_key(*p)).cloned());
    }
    Ok((next, changed))
}

pub(super) fn sources(store: &ContentStore, docs: &Documents) -> Result<Sources> {
    let template = native_sources(store, vec![])?;
    let mut out = Sources::new();
    for (name, empty) in template {
        let parts: Vec<_> = docs
            .values()
            .map(|d| Arc::clone(&d.sources[&name]))
            .collect();
        let source: Arc<dyn Source> = if name == "value_type" {
            // Literal types are keyed by interned value, shared across documents. Preserve set semantics.
            let rows = parts
                .iter()
                .flat_map(|p| {
                    (0..p.len()).map(|i| {
                        (0..p.columns().len())
                            .map(|c| p.value(i, c))
                            .collect::<Vec<_>>()
                    })
                })
                .collect::<BTreeSet<_>>();
            let columns = empty.columns();
            let mut r = Relation::new(&columns.iter().map(String::as_str).collect::<Vec<_>>());
            for row in rows {
                r.push(&row);
            }
            Arc::new(r)
        } else if parts.is_empty() {
            empty
        } else {
            Arc::new(Shards::new(
                empty.columns(),
                parts,
                docs.values()
                    .enumerate()
                    .map(|(i, d)| (i64::from(d.key), i))
                    .collect(),
            ))
        };
        out.insert(name.to_owned(), source);
    }
    Ok(out)
}

fn tuples(source: &dyn Source) -> BTreeSet<Vec<i64>> {
    let width = source.columns().len();
    (0..source.len())
        .map(|r| (0..width).map(|c| source.value(r, c)).collect())
        .collect()
}
impl CodeRuleSqlExecutor {
    pub(super) fn change_snapshot(
        &mut self,
        json: String,
        max_waves: Option<u32>,
    ) -> Result<String> {
        self.db.ensure_idle().map_err(error)?;
        if !self.db.is_autocommit() {
            return Err(error("snapshot changes require an autocommit session"));
        }
        let old = self
            .documents
            .as_ref()
            .ok_or_else(|| error("applySnapshot requires bindSnapshot first"))?;
        let (next, changed) = prepare(&self.store, &json, Some(old))?;
        if changed.is_empty() {
            return Ok(json!({"inserted_facts":0,"deleted_facts":0,"programs":[],"changed_files":[],"rebuilt_files":0}).to_string());
        }
        let sources = sources(&self.store, &next)?;
        // Only relation families read by retained programs need tuple journals. All other families
        // are rebound for extension queries without walking their rows (notably lazy containment).
        let reads: BTreeSet<_> = self
            .executions
            .iter()
            .flat_map(|e| e.inputs.iter().cloned())
            .collect();
        let mut prepared = vec![];
        for (name, source) in sources {
            let (plus, minus) = if reads.contains(&name) {
                let before: BTreeSet<_> = changed
                    .iter()
                    .filter_map(|p| old.get(p))
                    .flat_map(|d| tuples(d.sources[&name].as_ref()))
                    .collect();
                let after: BTreeSet<_> = changed
                    .iter()
                    .filter_map(|p| next.get(p))
                    .flat_map(|d| tuples(d.sources[&name].as_ref()))
                    .collect();
                let plus = after
                    .difference(&before)
                    .filter(|r| !updates::contains(self.inputs[&name].as_ref(), r))
                    .cloned()
                    .collect();
                let minus = before
                    .difference(&after)
                    .filter(|r| !updates::contains(source.as_ref(), r))
                    .cloned()
                    .collect();
                (plus, minus)
            } else {
                (vec![], vec![])
            };
            prepared.push(updates::Prepared {
                name,
                source,
                plus,
                minus,
                live: None,
                migrated: false,
            });
        }
        let mut report: Value =
            serde_json::from_str(&self.apply_prepared(prepared, max_waves)?).map_err(error)?;
        report["rebuilt_files"] = json!(changed.iter().filter(|p| next.contains_key(*p)).count());
        report["changed_files"] = json!(changed);
        self.documents = Some(next);
        Ok(report.to_string())
    }
}
