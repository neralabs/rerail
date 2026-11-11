//! Retained native document indexes. File edits replace only changed search paths and shared-fact counts.
use super::*;
mod shards;
mod shared;
mod tree;
use shards::Family;
use shared::Shared;

type Sources = BTreeMap<String, Arc<dyn Source>>;

/// A file of a snapshot a session binds: its path, its dialect's label and its text.
#[derive(Clone)]
pub(crate) struct File {
    pub(crate) path: String,
    pub(crate) language: String,
    pub(crate) source: Arc<String>,
}

impl crate::code_rule_dsl::facts::core::SourceFile for File {
    fn language(&self) -> &str {
        &self.language
    }
    fn source(&self) -> &Arc<String> {
        &self.source
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// What reads a snapshot's files from its JSON (`{"files": [...]}`, the evaluator's request shape), registered at
/// load (`install_snapshot_files`).
pub(crate) type SnapshotFiles = fn(&str) -> Result<Vec<File>>;

static SNAPSHOT_FILES: std::sync::OnceLock<SnapshotFiles> = std::sync::OnceLock::new();

/// Registers what reads a snapshot's files (once, at load).
pub(crate) fn install_snapshot_files(read: SnapshotFiles) {
    let _ = SNAPSHOT_FILES.set(read);
}

fn snapshot_files(json: &str) -> Result<Vec<File>> {
    match SNAPSHOT_FILES.get() {
        Some(read) => read(json),
        None => Err(error("no snapshot reader is registered")),
    }
}
#[derive(Clone)]
pub struct Document {
    file: File,
    key: u32,
    pub(super) sources: Sources,
}
pub struct Documents {
    files: BTreeMap<String, Document>,
    families: BTreeMap<String, Arc<Family>>,
    shared: Arc<Shared>,
}
impl std::ops::Index<&str> for Documents {
    type Output = Document;
    fn index(&self, path: &str) -> &Document {
        &self.files[path]
    }
}
type Changes = BTreeMap<String, Option<Document>>;
struct Indexed {
    families: BTreeMap<String, Arc<Family>>,
    shared: Arc<Shared>,
    literals: BTreeMap<Vec<i64>, isize>,
    nodes: usize,
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
fn document(store: &ContentStore, file: File) -> Result<Document> {
    let facts = match core::build_cached(&file, store).map_err(error)? {
        Built::Facts(facts) if !facts.syntax_errors => facts,
        _ => return Err(error(format!("{}: core facts are incomplete", file.path))),
    };
    let key = store.intern_atoms(&[&file.path]).map_err(error)?[0];
    let doc = syntax::Document {
        key,
        path: file.path.clone(),
        core: String::new(),
        digest: [0; 32],
        facts: Arc::new(facts),
        tape: None,
        guest: None,
        module: None,
    };
    Ok(Document {
        file,
        key,
        sources: native_sources(store, vec![doc])?,
    })
}
fn tuples(source: &dyn Source) -> BTreeSet<Vec<i64>> {
    let width = source.columns().len();
    (0..source.len())
        .map(|r| (0..width).map(|c| source.value(r, c)).collect())
        .collect()
}
impl Documents {
    pub(super) fn verification_files(&self) -> Value {
        json!(self.files.values().map(|d| json!({"path":d.file.path,"language":d.file.language,"source":d.file.source.as_str()})).collect::<Vec<_>>())
    }
    fn indexes(&self, changes: &Changes) -> Indexed {
        let mut nodes = 0;
        let mut families = BTreeMap::new();
        for (name, family) in &self.families {
            let updates = changes
                .iter()
                .filter_map(|(path, after)| {
                    let before = self.files.get(path);
                    let key = after.as_ref().or(before)?.key;
                    Some((key, after.as_ref().map(|d| d.sources[name].clone())))
                })
                .collect::<Vec<_>>();
            if let Some(next) = family.changed(&updates, &mut nodes) {
                families.insert(name.clone(), Arc::new(next));
            }
        }
        let mut literals = BTreeMap::<Vec<i64>, isize>::new();
        for (path, after) in changes {
            for (doc, sign) in [(self.files.get(path), -1), (after.as_ref(), 1)] {
                if let Some(doc) = doc {
                    for row in tuples(doc.sources["value_type"].as_ref()) {
                        *literals.entry(row).or_default() += sign;
                    }
                }
            }
        }
        literals.retain(|_, n| *n != 0);
        let shared = if literals.is_empty() {
            self.shared.clone()
        } else {
            Arc::new(self.shared.changed(&literals, &mut nodes))
        };
        Indexed {
            families,
            shared,
            literals,
            nodes,
        }
    }
    fn publish(&mut self, changes: Changes, indexed: Indexed) {
        for (path, doc) in changes {
            if let Some(doc) = doc {
                self.files.insert(path, doc);
            } else {
                self.files.remove(&path);
            }
        }
        self.families.extend(indexed.families);
        self.shared = indexed.shared;
    }
    fn snapshot(&self, store: &ContentStore, json: &str) -> Result<(Changes, usize)> {
        let files = snapshot_files(json)?;
        let examined = files.len() + self.files.len();
        let mut seen = BTreeSet::new();
        let mut changes = Changes::new();
        for file in files {
            if !seen.insert(file.path.clone()) {
                return Err(error(format!("duplicate snapshot path: {}", file.path)));
            }
            self.upsert(store, file, &mut changes)?;
        }
        for path in self.files.keys().filter(|p| !seen.contains(*p)) {
            changes.insert(path.clone(), None);
        }
        Ok((changes, examined))
    }
    fn upsert(&self, store: &ContentStore, file: File, changes: &mut Changes) -> Result<()> {
        if self
            .files
            .get(&file.path)
            .is_some_and(|d| d.file.source == file.source && d.file.language == file.language)
        {
            return Ok(());
        }
        changes.insert(file.path.clone(), Some(document(store, file)?));
        Ok(())
    }
    fn patch(&self, store: &ContentStore, json: &str) -> Result<(Changes, usize)> {
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Patch {
            #[serde(default)]
            upsert: Vec<Value>,
            #[serde(default)]
            delete: Vec<String>,
        }
        let patch: Patch = serde_json::from_str(json).map_err(error)?;
        let examined = patch.upsert.len() + patch.delete.len();
        let files = snapshot_files(&json!({"files":patch.upsert}).to_string())?;
        let mut seen = BTreeSet::new();
        let mut changes = Changes::new();
        for file in files {
            if !seen.insert(file.path.clone()) {
                return Err(error(format!("duplicate file change path: {}", file.path)));
            }
            self.upsert(store, file, &mut changes)?;
        }
        for path in patch.delete {
            if !seen.insert(path.clone()) {
                return Err(error(format!("duplicate file change path: {path}")));
            }
            if self.files.contains_key(&path) {
                changes.insert(path, None);
            }
        }
        Ok((changes, examined))
    }
}
pub(super) fn bind(store: &ContentStore, json: &str) -> Result<(Documents, Sources)> {
    let template = native_sources(store, vec![])?;
    let mut docs = Documents {
        files: BTreeMap::new(),
        families: template
            .into_iter()
            .filter(|(n, _)| n != "value_type")
            .map(|(n, s)| {
                let family = Arc::new(Family::empty(&n, s));
                (n, family)
            })
            .collect(),
        shared: Arc::new(Shared::default()),
    };
    let (changes, _) = docs.snapshot(store, json)?;
    let indexed = docs.indexes(&changes);
    docs.publish(changes, indexed);
    let mut sources: Sources = docs
        .families
        .iter()
        .map(|(n, s)| (n.clone(), s.clone() as Arc<dyn Source>))
        .collect();
    sources.insert("value_type".into(), docs.shared.clone());
    Ok((docs, sources))
}
impl CodeRuleSqlExecutor {
    pub(super) fn change_snapshot(
        &mut self,
        json: String,
        max_waves: Option<u32>,
    ) -> Result<String> {
        self.change_documents(json, max_waves, false)
    }
    pub(super) fn change_files(&mut self, json: String, max_waves: Option<u32>) -> Result<String> {
        self.change_documents(json, max_waves, true)
    }
    fn change_documents(
        &mut self,
        json: String,
        max_waves: Option<u32>,
        patch: bool,
    ) -> Result<String> {
        self.db.ensure_idle().map_err(error)?;
        if !self.db.is_autocommit() {
            return Err(error("file changes require an autocommit session"));
        }
        let docs = self
            .documents
            .as_ref()
            .ok_or_else(|| error("file changes require bindSnapshot first"))?;
        let (changes, examined) = if patch {
            docs.patch(&self.store, &json)?
        } else {
            docs.snapshot(&self.store, &json)?
        };
        if changes.is_empty() {
            return Ok(json!({"inserted_facts":0,"deleted_facts":0,"programs":[],"changed_files":[],"rebuilt_files":0,"examined_files":examined,"metadata_nodes":0,"initializations":[],"migrated_inputs":[]}).to_string());
        }
        let indexed = docs.indexes(&changes);
        let reads: BTreeSet<_> = self
            .executions
            .iter()
            .flat_map(|e| e.inputs.iter().cloned())
            .collect();
        let mut prepared = vec![];
        for (name, family) in &indexed.families {
            let (plus, minus) = if reads.contains(name) {
                let before: BTreeSet<_> = changes
                    .keys()
                    .filter_map(|p| docs.files.get(p))
                    .flat_map(|d| tuples(d.sources[name].as_ref()))
                    .collect();
                let after: BTreeSet<_> = changes
                    .values()
                    .flatten()
                    .flat_map(|d| tuples(d.sources[name].as_ref()))
                    .collect();
                // These families carry a document or node id. Shared literal values have their own counted index.
                (
                    after.difference(&before).cloned().collect(),
                    before.difference(&after).cloned().collect(),
                )
            } else {
                (vec![], vec![])
            };
            prepared.push(updates::Prepared {
                name: name.clone(),
                source: family.clone(),
                plus,
                minus,
                live: None,
                migrated: false,
            });
        }
        if !indexed.literals.is_empty() {
            let (plus, minus) = if reads.contains("value_type") {
                (
                    indexed
                        .literals
                        .keys()
                        .filter(|r| !docs.shared.contains(r) && indexed.shared.contains(r))
                        .cloned()
                        .collect(),
                    indexed
                        .literals
                        .keys()
                        .filter(|r| docs.shared.contains(r) && !indexed.shared.contains(r))
                        .cloned()
                        .collect(),
                )
            } else {
                (vec![], vec![])
            };
            prepared.push(updates::Prepared {
                name: "value_type".into(),
                source: indexed.shared.clone(),
                plus,
                minus,
                live: None,
                migrated: false,
            });
        }
        let mut report: Value =
            serde_json::from_str(&self.apply_prepared(prepared, max_waves)?).map_err(error)?;
        report["rebuilt_files"] = json!(changes.values().flatten().count());
        report["changed_files"] = json!(changes.keys().collect::<Vec<_>>());
        report["examined_files"] = json!(examined);
        report["metadata_nodes"] = json!(indexed.nodes);
        self.documents.as_mut().unwrap().publish(changes, indexed);
        Ok(report.to_string())
    }
}
