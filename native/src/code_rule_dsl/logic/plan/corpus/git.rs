//! Explicit Git requests in the corpus fixpoint. The handle belongs to this run.

use super::front::{Program, Value};
use gix::bstr::ByteSlice as _;
use std::path::Path;

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
pub(super) const INPUTS: [&str; 8] = [ROOT, REPOSITORY, TREE, COMMIT, SELECTION, HOST_REGISTRY, HOST_SUFFIX, HOST_NAME];

#[derive(Default)]
pub(super) struct Git {
    repository: Option<(String, gix::Repository)>,
    tree: Option<gix::ObjectId>,
    commit: Option<gix::ObjectId>,
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
        let object = repo
            .try_find_object(id)
            .map_err(|_| "unreadable")?
            .ok_or("missing")?;
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
                crate::code_rule_dsl::prof::count("git.tree_decodes", 1);
                let tree = object.into_tree();
                for entry in tree.iter() {
                    let entry = entry.map_err(|_| "malformed")?;
                    let mut record = match entry.filename().to_str() {
                        Ok(name) if !name.is_empty() && name != "." && name != ".." && !name.contains('/') =>
                            Record::text("entry", name),
                        Ok(_) => return Err("malformed"),
                        Err(_) => Record::text("entry-bytes", hex(entry.filename())),
                    };
                    record.second = entry.id().to_string();
                    record.number = i64::from(entry.mode().value());
                    records.push(record);
                }
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
        Ok(records)
    }

    pub(super) fn records(&self, key: &str, oid: &str) -> Vec<Record> {
        self.object(key, oid)
            .unwrap_or_else(|error| vec![Record::text("error", error)])
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
                    match repo
                        .and_then(|repo| Self::object_id(repo, arg).map(|id| repo.has_object(id)))
                    {
                        Ok(true) => "present",
                        Ok(false) => "missing",
                        Err(reason) => reason,
                    }
                    .to_owned(),
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

fn observes_git(program: &Program) -> bool {
    INPUTS.iter().any(|name| program.inputs.iter().any(|input| input == name))
}

pub(super) fn observation(program: &Program, root: &Path) -> Option<String> {
    observes_git(program).then(|| Git::new(root).identity())
}

pub(super) fn inputs(program: &Program, root: &Path) -> std::collections::BTreeMap<String, Vec<Vec<Value>>> {
    let mut inputs = std::collections::BTreeMap::new();
    if !observes_git(program) {
        return inputs;
    }
    let source = Git::new(root);
    for name in INPUTS.iter().filter(|name| program.inputs.iter().any(|input| input == **name)) {
        inputs.insert((*name).to_owned(), Vec::new());
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

/// Materialize the placements from the source phase. Rust only adapts the rows to
/// the snapshot store's inventory format.
pub(in crate::code_rule_dsl) fn inventory(root: &Path, store: Option<&Path>) -> Result<crate::git_workspace::snapshot::Inventory, crate::git_workspace::snapshot::InventoryError> {
    use crate::content_store::{GitTree, GitTreeEntry};
    use crate::git_workspace::{BaseFile, snapshot::{self, InventoryError}};
    use crate::code_rule_dsl::logic::plan::output::{Outputs, Value as Cell};
    use std::collections::BTreeMap;

    let _store = store.map(super::ThreadStore::enter);
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
    for row in outputs.rows("git_tree_entry") {
        let (Cell::Text(tree), Cell::Text(name), Some(mode), Cell::Text(target)) =
            (row.get(1), row.get(2), row.get(3).int(), row.get(4))
        else { return Err("invalid Git entry fact".into()) };
        entries.entry(tree.to_owned()).or_default().push(GitTreeEntry { name: name.to_owned(), mode: mode as u16, target: key(target)? });
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
    Ok(snapshot::Inventory { files, trees, languages })
}
