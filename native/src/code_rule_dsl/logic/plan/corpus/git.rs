//! Explicit Git requests in the corpus fixpoint. The handle belongs to this run.

use super::front::{Program, Value};
use crate::git_workspace::snapshot::{self, RawTreeEntry, storage};
use gix::bstr::ByteSlice as _;
use std::path::{Path, PathBuf};

pub(super) const REVISION: &str = "git_revision_request";
pub(super) const OBJECT: &str = "git_object_request";
pub(super) const INVENTORY_CHILD: &str = "git_inventory_child_request";
pub(super) const INVENTORY_TRAIL: &str = "git_inventory_trail_request";

const ROOT: &str = "git_root_repo";
const REPOSITORY: &str = "git_repository";
const TREE: &str = "git_source_tree";
const COMMIT: &str = "git_source_commit";
const SELECTION: &str = "git_inventory_selection";
const HOST_REGISTRY: &str = "git_inventory_host_registry";
const HOST_SUFFIX: &str = "git_inventory_host_suffix";
const HOST_NAME: &str = "git_inventory_host_name";
const SOURCE_BLOB: &str = "git_selected_source_blob";
pub(super) const INPUTS: [&str; 9] = [ROOT, REPOSITORY, TREE, COMMIT, SELECTION, HOST_REGISTRY, HOST_SUFFIX, HOST_NAME, SOURCE_BLOB];

#[derive(Default)]
pub(super) struct Git {
    repository: Option<(String, gix::Repository)>,
    tree: Option<gix::ObjectId>,
    commit: Option<gix::ObjectId>,
    snapshot_root: Option<PathBuf>,
    storage: std::cell::OnceCell<Option<storage::Storage>>,
}

/// A decoded object's records share one acquisition. The library gives them their public schemas.
pub(super) struct Record {
    pub field: &'static str,
    pub position: i64,
    pub first: String,
    pub second: String,
    pub number: i64,
    pub offset: i64,
}

impl Record {
    fn text(field: &'static str, first: impl Into<String>) -> Self {
        Self {
            field,
            position: 0,
            first: first.into(),
            second: String::new(),
            number: 0,
            offset: 0,
        }
    }
}

impl Git {
    pub(super) fn new(root: &Path) -> Self {
        let (repo, tree, commit) = match crate::git_workspace::snapshot::repository(root) {
            Some((repo, _, tree, commit)) => (Some(repo), Some(tree), commit),
            None => (gix::discover(root).ok(), None, None),
        };
        let repository = repo.and_then(|repo| {
            use sha2::Digest as _;
            let directory = repo.git_dir().canonicalize().ok()?;
            let key = format!(
                "repo:{}",
                hex(&sha2::Sha256::digest(
                    directory.as_os_str().as_encoded_bytes()
                ))
            );
            Some((key, repo))
        });
        Self {
            repository,
            tree,
            commit,
            snapshot_root: tree.map(|_| root.to_owned()),
            storage: Default::default(),
        }
    }

    fn repo(&self, key: &str) -> Result<&gix::Repository, &'static str> {
        self.repository
            .as_ref()
            .filter(|(id, _)| id == key)
            .map(|(_, repo)| repo)
            .ok_or("repository-not-open")
    }

    fn revision(&self, key: &str, expression: &str) -> Result<gix::ObjectId, &'static str> {
        self.repo(key)?
            .rev_parse_single(expression)
            .map(|id| id.detach())
            .map_err(|_| "unresolved")
    }

    pub(super) fn resolve(&self, key: &str, expression: &str) -> (String, &'static str) {
        match self.revision(key, expression) {
            Ok(id) => (id.to_string(), "ok"),
            Err(reason) => (String::new(), reason),
        }
    }

    fn object_id(repo: &gix::Repository, oid: &str) -> Result<gix::ObjectId, &'static str> {
        let id = gix::ObjectId::from_hex(oid.as_bytes()).map_err(|_| "invalid-id")?;
        (id.kind() == repo.object_hash())
            .then_some(id)
            .ok_or("invalid-id")
    }

    fn object(&self, key: &str, oid: &str) -> Result<Vec<Record>, &'static str> {
        let repo = self.repo(key)?;
        let id = Self::object_id(repo, oid)?;
        let observed = self.storage_stamp(repo, id);
        if let (Some(root), Some(stamp)) = (&self.snapshot_root, observed.as_deref()) {
            if let Some(tree) = snapshot::observed_tree(root, id, stamp) {
                if !repo.has_object(id) { return Err("missing"); }
                let mut records = vec![Record::text("kind", "tree"), Record::text("storage", stamp)];
                records.extend(tree.iter().map(raw_tree_record));
                return Ok(records);
            }
        }
        let object = snapshot::verified_object(repo, id).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound { "missing" } else { "unreadable" }
        })?;
        let mut records = vec![Record::text("kind", object.kind.to_string())];
        match object.kind {
            gix::object::Kind::Commit => {
                let commit = object.into_commit();
                let decoded = commit.decode().map_err(|_| "malformed")?;
                records.push(Record::text(
                    "tree",
                    decoded.tree.to_str().map_err(|_| "malformed")?,
                ));
                for (position, parent) in decoded.parents.iter().enumerate() {
                    let mut record =
                        Record::text("parent", parent.to_str().map_err(|_| "malformed")?);
                    record.position = position as i64;
                    records.push(record);
                }
                match decoded.message.to_str() {
                    Ok(message) => records.push(Record::text("message", message)),
                    Err(_) => records.push(Record::text("message-bytes", hex(decoded.message))),
                }
                if let Some(encoding) = decoded.encoding {
                    records.push(Record::text("encoding", hex(encoding)));
                }
                for (position, (name, value)) in decoded.extra_headers.iter().enumerate() {
                    let mut record =
                        Record::text("header", name.to_str().map_err(|_| "malformed")?);
                    record.position = position as i64;
                    record.second = hex(value.as_ref());
                    records.push(record);
                }
                let author = commit.author().map_err(|_| "malformed")?;
                let committer = commit.committer().map_err(|_| "malformed")?;
                for (role, signature) in [("author", author), ("committer", committer)] {
                    let time = signature.time().map_err(|_| "malformed")?;
                    let (Ok(name), Ok(email)) = (signature.name.to_str(), signature.email.to_str())
                    else {
                        let mut record = Record::text("signature-bytes", role);
                        record.second = hex(if role == "author" {
                            decoded.author
                        } else {
                            decoded.committer
                        });
                        records.push(record);
                        continue;
                    };
                    let mut record = Record::text(role, name);
                    record.second = email.to_owned();
                    record.number = time.seconds;
                    record.offset = i64::from(time.offset);
                    records.push(record);
                }
            }
            gix::object::Kind::Tree => {
                let entries = snapshot::decode_verified_tree(self.snapshot_root.as_deref(), observed.as_deref(), object.into_tree())
                    .map_err(|_| "malformed")?;
                records.extend(entries.iter().map(raw_tree_record));
            }
            gix::object::Kind::Tag => {
                let tag = object.into_tag();
                let decoded = tag.decode().map_err(|_| "malformed")?;
                let mut record =
                    Record::text("target", decoded.target.to_str().map_err(|_| "malformed")?);
                record.second = decoded.target_kind.to_string();
                records.push(record);
            }
            gix::object::Kind::Blob => {
                use sha2::Digest as _;
                records.push(Record::text(
                    "content",
                    hex(&sha2::Sha256::digest(&object.data)),
                ));
            }
        }
        records.push(Record::text("storage", observed.unwrap_or_else(|| "unchecked".to_owned())));
        Ok(records)
    }

    pub(super) fn records(&self, key: &str, oid: &str) -> Vec<Record> {
        self.object(key, oid)
            .unwrap_or_else(|error| vec![Record::text("error", error)])
    }

    fn storage_stamp(&self, repo: &gix::Repository, id: gix::ObjectId) -> Option<String> {
        if let Some(root) = &self.snapshot_root {
            return snapshot::object_storage(root, repo, id);
        }
        self.storage.get_or_init(|| storage::Storage::of(repo)).as_ref()?.object(id)
    }

    /// Presence is a separate observation. Re-observe former misses and open the new ref each run.
    pub(super) fn observation(&self, operation: &str, key: &str, arg: &str) -> Option<String> {
        match operation {
            REVISION => {
                let (id, status) = self.resolve(key, arg);
                Some(format!("{status}:{id}"))
            }
            OBJECT => {
                let repo = self.repo(key);
                Some(
                    match repo.and_then(|repo| Self::object_id(repo, arg).map(|id| (repo, id)))
                    {
                        Ok((repo, id)) if repo.has_object(id) => self.storage_stamp(repo, id).unwrap_or_else(|| "unverified".to_owned()),
                        Ok(_) => "missing".to_owned(),
                        Err(reason) => reason.to_owned(),
                    }
                )
            }
            _ => None,
        }
    }

    fn identity(&self) -> String {
        let key = self
            .repository
            .as_ref()
            .map(|(key, _)| key.as_str())
            .unwrap_or_default();
        format!("{key}\n{:?}\n{:?}", self.tree, self.commit)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn raw_tree_record(entry: &RawTreeEntry) -> Record {
    let mut record = match std::str::from_utf8(&entry.name) {
        Ok(name) => Record::text("entry", name),
        Err(_) => Record::text("entry-bytes", hex(&entry.name)),
    };
    record.second = entry.target.to_string();
    record.number = i64::from(entry.mode);
    record
}

fn observes_git(program: &Program) -> bool {
    INPUTS.iter().any(|name| program.inputs.iter().any(|input| input == name))
}

pub(super) fn observation(program: &Program, root: &Path) -> Option<String> {
    observes_git(program).then(|| Git::new(root).identity())
}

pub(super) fn inputs(program: &Program, root: &Path, options: &serde_json::Value) -> std::collections::BTreeMap<String, Vec<Vec<Value>>> {
    let mut inputs = std::collections::BTreeMap::new();
    if !observes_git(program) {
        return inputs;
    }
    let source = Git::new(root);
    for name in INPUTS.iter().filter(|name| program.inputs.iter().any(|input| input == **name)) {
        inputs.insert((*name).to_owned(), Vec::new());
    }
    if let Some(rows) = inputs.get_mut(SOURCE_BLOB)
        && let Some(paths) = options.get("gitSourcePaths").and_then(|paths| paths.as_array())
    {
        rows.extend(paths.iter().filter_map(|path| path.as_str()).filter_map(|path| {
            let oid = snapshot::source_object(root, path)?;
            Some(vec![Value::Str(path.into()), Value::Str(oid.to_string().into())])
        }));
    }
    if inputs.contains_key(HOST_REGISTRY)
        && let Some(registry) = crate::git_workspace::snapshot::source_languages(root)
    {
        use sha2::Digest as _;
        let version = hex(&sha2::Sha256::digest(serde_json::to_vec(&registry).expect("language registry")));
        inputs.get_mut(HOST_REGISTRY).unwrap().push(vec![Value::Str(version.clone().into())]);
        for (rel, labels) in [(HOST_SUFFIX, registry.suffixes), (HOST_NAME, registry.names)] {
            if let Some(rows) = inputs.get_mut(rel) {
                for (selector, language) in labels {
                    rows.push(vec![Value::Str(version.clone().into()), Value::Str(selector.into()), Value::Str(language.into())]);
                }
            }
        }
    }
    if let Some((key, repo)) = &source.repository {
        if let Some(rows) = inputs.get_mut(SELECTION)
            && let Some(selection) = crate::git_workspace::snapshot::inventory_selection(root)
        {
            rows.push(vec![Value::Str(selection.into())]);
        }
        let rows = [
            (ROOT, vec![key.clone()]),
            (
                REPOSITORY,
                vec![
                    key.clone(),
                    if repo.is_bare() { "bare" } else { "worktree" }.into(),
                    repo.object_hash().to_string(),
                ],
            ),
            (
                TREE,
                source
                    .tree
                    .map(|id| vec![key.clone(), id.to_string()])
                    .unwrap_or_default(),
            ),
            (
                COMMIT,
                source
                    .commit
                    .map(|id| vec![key.clone(), id.to_string()])
                    .unwrap_or_default(),
            ),
        ];
        for (rel, columns) in rows {
            if let Some(rows) = inputs.get_mut(rel) && !columns.is_empty() {
                rows.push(columns.into_iter().map(|v| Value::Str(v.into())).collect());
            }
        }
    }
    inputs
}

/// Use the ordinary Git acquisition rules for the paths this consumer will read.
/// A lookup failure belongs to this result rather than to the immutable tree inventory.
struct SourceObservation {
    repo: String,
    blob: gix::ObjectId,
    digest: [u8; 32],
    stamp: String,
}

type SourceChecks = (std::collections::HashMap<String, SourceObservation>, std::collections::HashMap<String, String>);

fn source_checks(root: &Path, paths: &[String]) -> super::Result<SourceChecks> {
    use crate::code_rule_dsl::logic::plan::output::{Outputs, Value as Cell};
    if paths.is_empty() {
        return Ok(Default::default());
    }
    let program = crate::code_rule_dsl::logic::rr::parse(concat!(
        include_str!("../../../../../../docs/rule-syntax/lib/core.rr"), "\n",
        include_str!("../../../../../../docs/rule-syntax/lib/git.rr"), "\n",
        include_str!("../../../typing/programs/git-source-objects.rr"),
    )).map_err(|err| super::Error::Plan(err.to_string()))?;
    let options = serde_json::json!({ "reuse": false, "inputFiles": [], "gitSourcePaths": paths });
    let image = super::run_fact_image(&program, root, &options)?;
    let outputs = Outputs::of(&image).map_err(super::Error::Plan)?;
    let invalid = || super::Error::Plan("invalid Git source observation".into());
    let checked = outputs.rows("git_selected_source_readable").into_iter().map(|row| {
        let (Cell::Text(path), Cell::Text(repo), Cell::Text(oid), Cell::Text(digest), Cell::Text(stamp)) =
            (row.get(0), row.get(1), row.get(2), row.get(3), row.get(4)) else { return Err(invalid()) };
        let blob = gix::ObjectId::from_hex(oid.as_bytes()).map_err(|_| invalid())?;
        let digest = gix::ObjectId::from_hex(digest.as_bytes()).map_err(|_| invalid())?;
        let digest = digest.as_bytes().try_into().map_err(|_| invalid())?;
        Ok((path.to_owned(), SourceObservation { repo: repo.to_owned(), blob, digest, stamp: stamp.to_owned() }))
    }).collect::<super::Result<_>>()?;
    let failures = outputs.rows("git_selected_source_error").iter()
        .filter_map(|row| Some((row.get(0).text()?.to_owned(), row.get(1).text()?.to_owned()))).collect();
    Ok((checked, failures))
}

pub(super) fn check_source_digests(root: &Path, store: &Path, paths: &[String]) -> Result<std::collections::HashMap<String, crate::content_store::HashKey>, snapshot::InventoryError> {
    let _store = super::ThreadStore::enter(store);
    let (checked, _) = source_checks(root, paths).map_err(|err| match err {
        super::Error::Cache(error) => snapshot::InventoryError::Cache(error),
        other => snapshot::InventoryError::Source(other.to_string()),
    })?;
    Ok(checked.into_iter().map(|(path, entry)| (path, crate::content_store::HashKey::from_sha256_digest(entry.digest))).collect())
}

pub(super) fn verify_sources(
    root: &Path,
    paths: &mut Vec<String>,
    consulted: &mut Vec<(String, String, String)>,
) -> super::Result<()> {
    if paths.is_empty() || !snapshot::contains(root) {
        return Ok(());
    }
    let (checked, failures) = source_checks(root, paths)?;
    super::observe(|observer| {
        for observation in checked.values() {
            let key = match observation.blob {
                gix::ObjectId::Sha1(bytes) => crate::content_store::HashKey::git_sha1(bytes),
                gix::ObjectId::Sha256(bytes) => crate::content_store::HashKey::git_sha256(bytes),
                _ => continue,
            };
            observer.record_alias(&key.key_bytes(), observation.digest);
            let request = serde_json::to_string(&(&observation.repo, observation.blob.to_string())).expect("string tuple");
            consulted.push((OBJECT.to_owned(), request, observation.stamp.clone()));
        }
    });
    paths.retain(|path| {
        if Path::new(path).is_absolute() && !snapshot::contains(Path::new(path)) || checked.contains_key(path) {
            return true;
        }
        let reason = failures.get(path).map_or("missing", String::as_str);
        snapshot::record_source_error(root, path, format!("Git source object is {reason}"));
        false
    });
    Ok(())
}

pub(super) fn still(root: &Path, previous: &str) -> bool {
    Git::new(root).identity() == previous
}

/// An output whose dependencies are entirely Git inputs and ordinary rules
/// can run before any source inventory is available.
pub(super) fn metadata_only(program: &Program) -> bool {
    let needed = super::front::needed(program, program.outputs.clone());
    if !needed.iter().any(|rel| INPUTS.contains(&rel.as_str())) {
        return false;
    }
    let heads: std::collections::BTreeSet<_> = program.rules.iter().map(|rule| rule.head.rel.as_str()).collect();
    needed.iter().all(|rel| {
        !program.natives.contains_key(rel.as_str())
            && (heads.contains(rel.as_str())
                || program.facts.contains_key(rel.as_str())
                || INPUTS.contains(&rel.as_str())
                // A declared relation with no producers is empty, unless a phase can supply it.
                || (program.decls.contains_key(rel.as_str()) && !rel.contains("__") && !program.inputs.contains(rel))
                || matches!(rel.as_str(), "git_revision_of" | "git_object_of" | "git_inventory_child_of" | "git_inventory_trail_of"))
    })
}

/// Restore the root entries before the host looks up Git-backed control files.
pub(super) fn prime_root(root: &Path) -> super::Result<()> {
    use crate::code_rule_dsl::logic::plan::output::{Outputs, Value as Cell};
    if !snapshot::contains(root) || snapshot::root_observed(root) {
        return Ok(());
    }
    let program = crate::code_rule_dsl::logic::rr::parse(concat!(
        include_str!("../../../../../../docs/rule-syntax/lib/core.rr"), "\n",
        include_str!("../../../../../../docs/rule-syntax/lib/git.rr"), "\n",
        include_str!("../../../typing/programs/git-root-tree.rr"),
    )).map_err(|err| super::Error::Plan(err.to_string()))?;
    let options = serde_json::json!({ "reuse": false, "inputFiles": [] });
    let image = super::run_fact_image(&program, root, &options)?;
    let outputs = Outputs::of(&image).map_err(super::Error::Plan)?;
    if let Some(row) = outputs.rows("git_root_tree_error").first() {
        return Err(super::Error::Plan(format!("cannot inventory Git tree {}: {}", row.get(0).text().unwrap_or(""), row.get(1).text().unwrap_or("unknown error"))));
    }
    let invalid = || super::Error::Plan("invalid Git root tree fact".to_owned());
    let stamps = outputs.rows("git_root_tree_storage");
    let stamp = stamps.first().and_then(|row| row.get(0).text()).ok_or_else(invalid)?;
    let mut entries = Vec::new();
    for (relation, bytes) in [("git_tree_entry", false), ("git_tree_entry_bytes", true)] {
        for row in outputs.rows(relation) {
            let (Cell::Text(name), Some(mode), Cell::Text(target)) = (row.get(2), row.get(3).int(), row.get(4))
                else { return Err(invalid()) };
            let name = if bytes { snapshot::raw_entry_name(name).ok_or_else(invalid)? } else { name.as_bytes().to_vec() };
            let target = gix::ObjectId::from_hex(target.as_bytes()).map_err(|_| invalid())?;
            entries.push(RawTreeEntry { name, mode: mode as u16, target });
        }
    }
    snapshot::remember_root(root, stamp.to_owned(), entries.into());
    Ok(())
}

/// Materialize the placements from the source phase. Rust only adapts the rows to
/// the snapshot store's inventory format.
pub(in crate::code_rule_dsl) fn inventory(root: &Path, store: Option<&Path>) -> Result<crate::git_workspace::snapshot::Inventory, crate::git_workspace::snapshot::InventoryError> {
    use crate::content_store::{GitTree, GitTreeEntry};
    use crate::git_workspace::{BaseFile, snapshot::{self, InventoryError}};
    use crate::code_rule_dsl::logic::plan::output::{Outputs, Value as Cell};
    use std::collections::BTreeMap;

    let _store = store.map(super::ThreadStore::enter);
    prime_root(root).map_err(|error| match error {
        super::Error::Cache(error) => InventoryError::Cache(error),
        other => InventoryError::Source(other.to_string()),
    })?;
    // Installed addons have no source checkout. Embed the same library used by .rr policies.
    let program = crate::code_rule_dsl::logic::rr::parse(concat!(
        include_str!("../../../../../../docs/rule-syntax/lib/core.rr"), "\n",
        include_str!("../../../../../../docs/rule-syntax/lib/git.rr"), "\n",
        include_str!("../../../../../../docs/rule-syntax/lib/source-language.rr"), "\n",
        include_str!("../../../typing/programs/git-inventory.rr"),
    ))
        .map_err(|err| err.to_string())?;
    let options = serde_json::json!({ "reuse": false, "inputFiles": [] });
    let image = super::run_fact_image(&program, root, &options).map_err(|error| match error {
        super::Error::Cache(error) => InventoryError::Cache(error),
        other => InventoryError::Source(other.to_string()),
    })?;
    let outputs = Outputs::of(&image)?;
    if let Some(row) = outputs.rows("inventory_error").first() {
        return Err(format!("cannot inventory Git tree {}: {}", row.get(0).text().unwrap_or(""), row.get(1).text().unwrap_or("unknown error")).into());
    }
    let key = |oid: &str| {
        let id = gix::ObjectId::from_hex(oid.as_bytes()).map_err(|err| err.to_string())?;
        match id {
            gix::ObjectId::Sha1(bytes) => Ok(crate::content_store::HashKey::git_sha1(bytes)),
            gix::ObjectId::Sha256(bytes) => Ok(crate::content_store::HashKey::git_sha256(bytes)),
            _ => Err("unsupported Git object hash".to_owned()),
        }
    };
    let mut entries = BTreeMap::<_, Vec<GitTreeEntry>>::new();
    for relation in ["git_tree_entry", "git_tree_entry_bytes"] {
        for row in outputs.rows(relation) {
            let (Cell::Text(tree), Cell::Text(name), Some(mode), Cell::Text(target)) =
                (row.get(1), row.get(2), row.get(3).int(), row.get(4))
            else { return Err("invalid Git entry fact".into()) };
            // Git names cannot contain a NUL. Use it to distinguish byte names in
            // the store's textual tree entries. They never become document paths.
            let name = if relation == "git_tree_entry_bytes" { format!("\0{name}") } else { name.to_owned() };
            entries.entry(tree.to_owned()).or_default().push(GitTreeEntry { name, mode: mode as u16, target: key(target)? });
        }
    }
    let mut trees = Vec::new();
    for row in outputs.rows("inventory_directory") {
        let (Cell::Text(prefix), Cell::Text(tree)) = (row.get(1), row.get(2))
        else { return Err("invalid Git directory fact".into()) };
        trees.push(GitTree { id: key(tree)?, prefix: prefix.to_owned(), entries: entries.get(tree).cloned().unwrap_or_default() });
    }
    if !trees.iter().any(|tree| tree.prefix.is_empty()) {
        return Err("Git inventory lacks a root directory".into());
    }
    let mut files = Vec::new();
    for row in outputs.rows("inventory_file") {
        let (Cell::Text(path), Some(mode), Cell::Text(blob)) = (row.get(0), row.get(1).int(), row.get(2))
        else { return Err("invalid Git file fact".into()) };
        files.push(BaseFile { path: path.to_owned(), blob: key(blob)?, mode: mode as u16, checkout: None });
    }
    let language_rows = outputs.rows("inventory_language");
    let languages = language_rows.into_iter()
        .map(|row| {
            let (Cell::Text(path), Cell::Text(grammar)) = (row.get(0), row.get(1))
            else { return Err("invalid source language fact".to_owned()) };
            Ok((path.to_owned(), grammar.to_owned()))
        })
        .collect::<Result<_, _>>()?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    trees.sort_by_key(|tree| tree.id);
    let invalid_paths = outputs.rows("inventory_path_error").into_iter()
        .filter_map(|row| row.get(0).text().map(str::to_owned)).collect();
    let tree_observations = outputs.rows("inventory_tree_observation").into_iter().map(|row| {
        let (Cell::Text(tree), Cell::Text(stamp)) = (row.get(0), row.get(1))
            else { return Err("invalid Git tree observation".to_owned()) };
        Ok((key(tree)?, stamp.to_owned()))
    }).collect::<Result<_, String>>()?;
    Ok(snapshot::Inventory { files, trees, languages, invalid_paths, tree_observations })
}
