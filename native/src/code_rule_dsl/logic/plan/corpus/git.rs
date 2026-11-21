//! Explicit Git requests in the corpus fixpoint. The handle belongs to this run.

use super::front::Value;
use crate::code_rule_dsl::logic::program_image::{Version, fields::ProgramFields};
use crate::git_workspace::snapshot::{self, RawTreeEntry, storage};
use gix::bstr::ByteSlice as _;
use std::path::{Path, PathBuf};

pub(super) const REVISION: &str = "git_view_revision_request";
pub(super) const REPO_REVISION: &str = "git_revision_request";
pub(super) const OBJECT: &str = "git_object_request";
pub(super) const INVENTORY_CHILD: &str = "git_inventory_child_request";

// The inputs' names are the planner's (`plan::phased`), which typing reads.
use crate::code_rule_dsl::logic::plan::phased::git::{COMMIT, LOCATION, REPOSITORY, ROOT, SELECTION, SOURCE_BLOB, TREE, VIEW};
pub(super) use crate::code_rule_dsl::logic::plan::phased::git::{INPUTS, SELECTED_DIRECTORY};

#[derive(Default)]
pub(super) struct Git {
    repository: Option<(snapshot::Identity, gix::Repository)>,
    tree: Option<gix::ObjectId>,
    commit: Option<gix::ObjectId>,
    snapshot_root: Option<PathBuf>,
    revision_repository: std::cell::OnceCell<std::io::Result<gix::Repository>>,
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
            None => (crate::git_access::opener::open(root).ok().map(snapshot::physical_objects), None, None),
        };
        let repository = repo.and_then(|repo| {
            let identity = snapshot::pinned_identity(root).or_else(|| snapshot::Identity::of(&repo, false).ok())?;
            Some((identity, repo))
        });
        Self {
            repository,
            tree,
            commit,
            snapshot_root: tree.map(|_| root.to_owned()),
            revision_repository: Default::default(),
            storage: Default::default(),
        }
    }

    fn repo(&self, key: &str) -> Result<&gix::Repository, &'static str> {
        self.repository
            .as_ref()
            .filter(|(id, _)| id.repository == key)
            .map(|(_, repo)| repo)
            .ok_or("repository-not-open")
    }

    fn revision(&self, operation: &str, key: &str, expression: &str) -> Result<gix::ObjectId, &'static str> {
        let repo_key = if operation == REVISION {
            &self.repository.as_ref().filter(|(id, _)| id.view == key).ok_or("repository-not-open")?.0.repository
        } else { key };
        self.with_repository(repo_key, |repo, _| {
            let repo = if self.snapshot_root.is_some() {
                self.revision_repository.get_or_init(|| snapshot::reopen_for_revisions(repo)).as_ref().map_err(|_| "unreadable-object")?
            } else { repo };
            let base = repo.workdir().map(|_| Path::new(""));
            snapshot::resolve_revision(repo, expression, base).map_err(|error| snapshot::revision_failure_reason(&error))
        })
    }

    pub(super) fn resolve(&self, operation: &str, key: &str, expression: &str) -> (String, &'static str) {
        match self.revision(operation, key, expression) {
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
        self.with_repository(key, |repo, storage| self.read_object(repo, storage, oid))
    }

    fn with_repository<T>(&self, key: &str, read: impl FnOnce(&gix::Repository, Option<&storage::Storage>) -> Result<T, &'static str>) -> Result<T, &'static str> {
        let repo = self.repo(key)?;
        if let Some(root) = &self.snapshot_root {
            return snapshot::with_repository(root, read).map_err(|_| "unreadable-object")?;
        }
        read(repo, self.storage.get_or_init(|| storage::Storage::of(repo)).as_ref())
    }

    pub(super) fn payload(&self, key: &str, oid: &str) -> Result<(String, Vec<u8>, String), &'static str> {
        self.with_repository(key, |repo, storage| {
            let id = Self::object_id(repo, oid)?;
            let stamp = storage.and_then(|s| s.object(id)).unwrap_or_else(|| "unverified".to_owned());
            let object = snapshot::verified_object(repo, id).map_err(|error| {
                if error.kind() == std::io::ErrorKind::NotFound { "missing" } else { "unreadable" }
            })?;
            Ok((object.kind.to_string(), object.detach().data, stamp))
        })
    }

    fn read_object(&self, repo: &gix::Repository, storage: Option<&storage::Storage>, oid: &str) -> Result<Vec<Record>, &'static str> {
        let id = Self::object_id(repo, oid)?;
        let observed = storage.and_then(|storage| storage.object(id));
        if let (Some(root), Some(stamp)) = (&self.snapshot_root, observed.as_deref())
            && let Some(tree) = snapshot::observed_tree(root, id, stamp)
        {
            if !repo.has_object(id) { return Err("missing"); }
            let mut records = vec![Record::text("kind", "tree"), Record::text("storage", stamp)];
            records.extend(tree.iter().map(raw_tree_record));
            return Ok(records);
        }
        crate::code_rule_dsl::logic::profile::decision("Git metadata object read", || oid.to_owned());
        let object = snapshot::verified_object(repo, id).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound { "missing" } else { "unreadable" }
        })?;
        let mut records = vec![Record::text("kind", object.kind.to_string())];
        match object.kind {
            gix::object::Kind::Commit => {
                let commit = object.into_commit();
                let decoded = snapshot::decoded_commit(&commit).map_err(|_| "malformed")?;
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
                    let mut record = match name.to_str() {
                        Ok(text) => Record::text("header", text),
                        Err(_) => Record::text("header-bytes", hex(name)),
                    };
                    record.position = position as i64;
                    record.second = hex(value.as_ref());
                    records.push(record);
                }
                let author = commit.author().map_err(|_| "malformed")?;
                let committer = commit.committer().map_err(|_| "malformed")?;
                for (role, signature, bytes) in [("author", author, decoded.author), ("committer", committer, decoded.committer)] {
                    let time = signature.time().map_err(|_| "malformed")?;
                    let (mut record, email_or_bytes) = match (signature.name.to_str(), signature.email.to_str()) {
                        (Ok(name), Ok(email)) => (Record::text(role, name), email.to_owned()),
                        _ => (Record::text("signature-bytes", role), hex(bytes)),
                    };
                    record.second = email_or_bytes;
                    record.number = time.seconds;
                    record.offset = i64::from(time.offset);
                    records.push(record);
                }
            }
            gix::object::Kind::Tree => {
                let entries = match snapshot::decode_verified_tree(self.snapshot_root.as_deref(), observed.as_deref(), object.into_tree()) {
                    Ok(entries) => entries,
                    Err(error) => {
                        let mut record = Record::text("error", "malformed");
                        record.second = error.to_string();
                        return Ok(vec![record]);
                    }
                };
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

    /// Presence is a separate observation. Re-observe former misses and open the new ref each run.
    pub(super) fn observation(&self, operation: &str, key: &str, arg: &str) -> Option<String> {
        match operation {
            REVISION | REPO_REVISION => {
                let (id, status) = self.resolve(operation, key, arg);
                Some(format!("{status}:{id}"))
            }
            OBJECT => {
                Some(self.with_repository(key, |repo, storage| {
                    let id = Self::object_id(repo, arg)?;
                    if !repo.has_object(id) { return Err("missing"); }
                    Ok(storage.and_then(|storage| storage.object(id)).unwrap_or_else(|| "unverified".to_owned()))
                }).unwrap_or_else(str::to_owned))
            }
            _ => None,
        }
    }

    fn identity(&self, locations: bool) -> String {
        let key = self
            .repository
            .as_ref()
            .map(|(id, _)| (id.repository.as_str(), id.view.as_str()))
            .unwrap_or_default();
        let mut identity = format!("{}\n{}\n{:?}\n{:?}", key.0, key.1, self.tree, self.commit);
        if locations && let Some((_, repo)) = &self.repository {
            identity.push('\n');
            identity.push_str(&serde_json::to_string(&location_rows(repo)).expect("Git location strings"));
        }
        identity
    }
}

fn location_rows(repo: &gix::Repository) -> Vec<(&'static str, &'static str, String)> {
    let paths = std::iter::once(("common-dir", repo.common_dir()))
        .chain(repo.workdir().map(|path| ("worktree", path)));
    paths.map(|(role, path)| {
        let (encoding, value) = match path.canonicalize() {
            Ok(path) => location_value(&path),
            Err(error) => ("error", error.to_string()),
        };
        (role, encoding, value)
    }).collect()
}

fn location_value(path: &Path) -> (&'static str, String) {
    if let Some(text) = path.to_str() { return ("text", text.to_owned()); }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        ("utf16le", hex(&path.as_os_str().encode_wide().flat_map(u16::to_le_bytes).collect::<Vec<_>>()))
    }
    #[cfg(not(windows))]
    { ("unix-bytes", hex(path.as_os_str().as_encoded_bytes())) }
}

#[test]
#[cfg(any(unix, windows))]
fn non_unicode_git_locations_keep_the_original_path_units() {
    #[cfg(unix)]
    {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
        assert_eq!(location_value(Path::new(OsStr::from_bytes(b"x\xff"))), ("unix-bytes", "78ff".into()));
    }
    #[cfg(windows)]
    {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt};
        let path = OsString::from_wide(&[0x78, 0xd800]);
        assert_eq!(location_value(Path::new(&path)), ("utf16le", "780000d8".into()));
    }
}

pub(super) fn hex(bytes: &[u8]) -> String {
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

fn observes_git(program: &(impl ProgramFields + ?Sized)) -> bool {
    let inputs = program.input_names();
    INPUTS.iter().any(|name| inputs.iter().any(|input| input == name))
}

pub(super) fn observation(program: &(impl ProgramFields + ?Sized), root: &Path) -> Option<(&'static str, String)> {
    observes_git(program).then(|| {
        let locations = program.input_names().iter().any(|input| input == LOCATION);
        (if locations { LOCATION } else { "" }, Git::new(root).identity(locations))
    })
}

pub(super) fn inputs(
    program: &(impl ProgramFields + ?Sized), root: &Path, options: &serde_json::Value, consulted: &mut Vec<(String, String, String)>,
) -> std::collections::BTreeMap<String, Vec<Vec<Value>>> {
    let mut inputs = std::collections::BTreeMap::new();
    if !observes_git(program) {
        return inputs;
    }
    let source = Git::new(root);
    let declared = program.input_names();
    for name in INPUTS.iter().filter(|name| declared.iter().any(|input| input == **name)) {
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
    if let Some((identity, repo)) = &source.repository {
        let key = &identity.repository;
        if let Some(rows) = inputs.get_mut(SELECTED_DIRECTORY) {
            let context = snapshot::selection_context(root);
            consulted.push((SELECTED_DIRECTORY.to_owned(), String::new(), serde_json::to_string(&context).expect("selection strings")));
            if let (Some(tree), Some((prefix, config, selection))) = (source.tree, context) {
                rows.push([key.clone(), tree.to_string(), prefix, config, selection].into_iter().map(|s| Value::Str(s.into())).collect());
            }
        }
        if let Some(rows) = inputs.get_mut(LOCATION) {
            rows.extend(location_rows(repo).into_iter().map(|(role, encoding, value)| {
                vec![Value::Str(key.as_str().into()), Value::Str(identity.view.as_str().into()), Value::Str(role.into()), Value::Str(encoding.into()), Value::Str(value.into())]
            }));
        }
        if let Some(rows) = inputs.get_mut(SELECTION)
            && let Some(selection) = crate::git_workspace::snapshot::inventory_selection(root)
        {
            rows.push(vec![Value::Str(selection.into())]);
        }
        let rows = [
            (ROOT, vec![key.clone()]),
            (VIEW, vec![key.clone(), identity.view.clone()]),
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
    let program = concat!(
        include_str!("../../../../../../docs/rule-syntax/lib/core.rerail"), "\n",
        include_str!("../../../../../../docs/rule-syntax/lib/git.rerail"), "\n",
        include_str!("../../../typing/programs/git-source-objects.rerail"),
    );
    let options = serde_json::json!({ "reuse": false, "inputFiles": [], "gitSourcePaths": paths });
    let image = super::run_fact_image(program, root, &options)?;
    let outputs = Outputs::of(&image).map_err(super::Error::Plan)?;
    let invalid = || super::Error::Plan("invalid Git source observation".into());
    let checked: std::collections::HashMap<_, _> = outputs.rows("git_selected_source_readable").into_iter().map(|row| {
        let (Cell::Text(path), Cell::Text(repo), Cell::Text(oid), Cell::Text(digest), Cell::Text(stamp)) =
            (row.get(0), row.get(1), row.get(2), row.get(3), row.get(4)) else { return Err(invalid()) };
        let blob = gix::ObjectId::from_hex(oid.as_bytes()).map_err(|_| invalid())?;
        let digest = gix::ObjectId::from_hex(digest.as_bytes()).map_err(|_| invalid())?;
        let digest = digest.as_bytes().try_into().map_err(|_| invalid())?;
        Ok((path.to_owned(), SourceObservation { repo: repo.to_owned(), blob, digest, stamp: stamp.to_owned() }))
    }).collect::<super::Result<_>>()?;
    for path in checked.keys() {
        snapshot::clear_source_error(root, path);
    }
    let failures = outputs.rows("git_selected_source_error").iter()
        .filter_map(|row| Some((row.get(0).text()?.to_owned(), row.get(1).text()?.to_owned()))).collect();
    Ok((checked, failures))
}

pub(super) fn check_source_digests(root: &Path, store: &Path, paths: &[String], owner: Option<&crate::content_store::analysis_scope::AnalysisOwner>) -> Result<std::collections::HashMap<String, crate::content_store::HashKey>, snapshot::InventoryError> {
    let _store = super::ThreadStore::select(store, owner.map(|o| (root.to_owned(), o.clone())));
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
        if let Some(reads) = super::ThreadStore::git_reads() { reads.mark_incomplete(); }
        snapshot::record_source_error(root, path, format!("Git source object is {reason}"));
        false
    });
    Ok(())
}

pub(super) fn selection_stamp(root: &Path) -> String {
    serde_json::to_string(&snapshot::selection_context(root)).expect("selection strings")
}

pub(super) fn still(root: &Path, input: &str, previous: &str) -> bool {
    Git::new(root).identity(input == LOCATION) == previous
}

/// An output whose dependencies are entirely Git inputs and ordinary rules
/// can run before any source inventory is available.
pub(super) fn metadata_only(program: &Version) -> bool {
    let needed = super::world::needed(program);
    if !needed.iter().any(|rel| INPUTS.contains(&rel.as_str()) || super::analysis_history::is_input(rel) || rel == super::analysis_history::ANSWER || rel == super::git_content::ANSWER) {
        return false;
    }
    let heads: std::collections::BTreeSet<&str> = program.rules().map(|rule| program.rel_name(rule.head().rel())).collect();
    let natives = program.natives();
    needed.iter().all(|rel| {
        // These operators read strings from their input relation. `world::needed` visits that relation too.
        if let Some(native) = natives.get(rel.as_str()) {
            return matches!(native, super::front::Native::MatchAll { .. } | super::front::Native::TextTest { .. });
        }
        heads.contains(rel.as_str())
            || program.states_facts(rel)
            || INPUTS.contains(&rel.as_str())
            || super::analysis_history::is_input(rel)
            || rel == super::analysis_history::ANSWER || rel == super::git_content::ANSWER
            || super::source_languages::is_input(rel)
            // A declared relation with no producers is empty, unless a phase can supply it.
            || (program.declares(rel) && !rel.contains("__") && !program.has_input(rel))
            || matches!(rel.as_str(), "git_view_revision_of" | "git_revision_of" | "git_object_of" | "git_inventory_child_of")
    })
}

/// Restore the root entries before the host looks up Git-backed control files.
pub(super) fn prime_root(root: &Path) -> super::Result<()> {
    use crate::code_rule_dsl::logic::plan::output::{Outputs, Value as Cell};
    let reads = super::ThreadStore::git_reads();
    if !snapshot::contains(root) || reads.as_deref().is_some_and(|reads| snapshot::root_observed(root, reads)) {
        return Ok(());
    }
    let program = concat!(
        include_str!("../../../../../../docs/rule-syntax/lib/core.rerail"), "\n",
        include_str!("../../../../../../docs/rule-syntax/lib/git.rerail"), "\n",
        include_str!("../../../typing/programs/git-root-tree.rerail"),
    );
    let options = serde_json::json!({ "reuse": false, "inputFiles": [] });
    let image = super::run_fact_image(program, root, &options)?;
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
    snapshot::remember_root(root, stamp.to_owned(), entries, reads.as_deref());
    Ok(())
}

/// Materialize the placements from the source phase. Rust only adapts the rows to
/// the snapshot store's inventory format.
pub(in crate::code_rule_dsl) fn inventory(root: &Path, store: Option<&Path>, owner: Option<&crate::content_store::analysis_scope::AnalysisOwner>) -> Result<crate::git_workspace::snapshot::Inventory, crate::git_workspace::snapshot::InventoryError> {
    use crate::content_store::{GitTree, GitTreeEntry};
    use crate::git_workspace::{BaseFile, snapshot::{self, InventoryError}};
    use crate::code_rule_dsl::logic::plan::output::{Outputs, Value as Cell};
    use std::collections::BTreeMap;

    let _store = store.map(|store| super::ThreadStore::select(store, owner.map(|o| (root.to_owned(), o.clone()))));
    prime_root(root).map_err(|error| match error {
        super::Error::Cache(error) => InventoryError::Cache(error),
        other => InventoryError::Source(other.to_string()),
    })?;
    // Installed addons have no source checkout. Embed the same library used by .rerail policies.
    let program = concat!(
        include_str!("../../../../../../docs/rule-syntax/lib/core.rerail"), "\n",
        include_str!("../../../../../../docs/rule-syntax/lib/git.rerail"), "\n",
        include_str!("../../../../../../docs/rule-syntax/lib/source-language.rerail"), "\n",
        include_str!("../../../../../../docs/rule-syntax/lib/git/selected-tree.rerail"), "\n",
        include_str!("../../../typing/programs/git-inventory.rerail"),
    );
    let options = serde_json::json!({ "reuse": false, "inputFiles": [] });
    let image = super::run_fact_image(program, root, &options).map_err(|error| match error {
        super::Error::Cache(error) => InventoryError::Cache(error),
        other => InventoryError::Source(other.to_string()),
    })?;
    let outputs = Outputs::of(&image)?;
    let invalid = || "invalid Git inventory fact".to_owned();
    let tree_failures = outputs.rows("inventory_error").into_iter().map(|row| {
        let (Cell::Text(path), Cell::Text(tree), Cell::Text(reason)) = (row.get(0), row.get(1), row.get(2))
            else { return Err(invalid()) };
        Ok(snapshot::SourceFailure::text(path.to_owned(), format!("cannot inventory Git tree {tree}: {reason}")))
    }).collect::<Result<_, String>>()?;
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
    for row in outputs.rows("inventory_readable_directory") {
        let (Cell::Text(prefix), Cell::Text(tree)) = (row.get(0), row.get(1))
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
    let invalid_paths = outputs.rows("inventory_path_error").into_iter().map(|row| {
        let (Cell::Text(directory), Cell::Text(hex_name)) = (row.get(0), row.get(1)) else { return Err(invalid()) };
        Ok(snapshot::InvalidGitPath { directory: directory.to_owned(), hex_name: hex_name.to_owned() })
    }).collect::<Result<_, String>>()?;
    let tree_observations = outputs.rows("inventory_tree_observation").into_iter().map(|row| {
        let (Cell::Text(tree), Cell::Text(stamp)) = (row.get(0), row.get(1))
            else { return Err("invalid Git tree observation".to_owned()) };
        let id = gix::ObjectId::from_hex(tree.as_bytes()).map_err(|_| invalid())?;
        Ok((id, stamp.to_owned()))
    }).collect::<Result<_, String>>()?;
    let selection = outputs.rows("git_inventory_selection").first()
        .and_then(|row| row.get(0).text()).ok_or_else(invalid)?.to_owned();
    Ok(snapshot::Inventory { files, trees, languages, invalid_paths, tree_failures, tree_observations, selection })
}
