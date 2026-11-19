//! The owners of corpus manifests and audit snapshots. Their rows are roots for collection.

use super::{CollectionLease, ContentStore, SnapshotId, StoreResult};
use super::connection::Cached as _;
use std::path::Path;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AnalysisOwner {
    pub scope: String,
    generation: i64,
    #[serde(default)]
    store_identity: Option<i64>,
    #[serde(default)]
    pending: Option<String>,
    /// Acquisition generation from the control database. Keeps its original serialized field name.
    #[serde(default)]
    acquired_at: i64,
    #[serde(default)]
    pub(super) acquisition: i64,
    #[serde(skip)]
    warning: Option<std::sync::Arc<std::sync::Mutex<RunWarning>>>,
}

#[derive(Debug, Default)]
struct RunWarning {
    message: Option<String>,
    finished: bool,
}

impl AnalysisOwner {
    pub fn membership(&self) -> &str {
        self.pending.as_deref().unwrap_or(&self.scope)
    }

    pub(super) fn acquired_at(&self) -> i64 { self.acquired_at }

    pub fn is_staged(&self) -> bool { self.warning.is_some() }

    pub fn warn(&self, message: String) -> bool {
        let Some(warning) = &self.warning else { return false };
        let mut warning = warning.lock().unwrap_or_else(|e| e.into_inner());
        if warning.finished { return false; }
        warning.message.get_or_insert(message);
        true
    }

    pub fn cache_warning(&self) -> Option<String> {
        self.warning.as_ref()?.lock().unwrap_or_else(|e| e.into_inner()).message.clone()
    }

    pub(super) fn current(&self, tx: &rusqlite::Connection) -> rusqlite::Result<bool> {
        tx.query_row_cached(
            "SELECT EXISTS (SELECT 1 FROM analysis_scope_generation WHERE scope=?1 AND generation=?2) \
                AND (?5 IS NULL OR EXISTS (SELECT 1 FROM analysis_scope_pending WHERE id=?5)) \
                AND (?6 IS NULL OR (SELECT id FROM store_identity WHERE singleton=1)=?6) \
                AND NOT EXISTS (SELECT 1 FROM analysis_scope_release WHERE scope IN (SELECT value FROM json_each(?4)) AND before>=?3) \
                AND NOT EXISTS (SELECT 1 FROM analysis_scope_release WHERE legacy_scope IN (SELECT value FROM json_each(?4))
                    AND before>=?3 AND legacy_acquisition>=?7)",
            (&self.scope, self.generation, self.acquired_at, receipt_scopes(&self.scope), self.pending.as_deref(), self.store_identity, self.acquisition), |row| row.get(0),
        )
    }
}

// A stop covers its directory and descendants, including stops at a volume root.
fn receipt_scopes(scope: &str) -> String {
    let mut ancestors = Vec::new();
    if scope.starts_with("cwd:") {
        ancestors.push(scope);
        for (at, sep) in scope.match_indices(std::path::MAIN_SEPARATOR) {
            ancestors.extend([&scope[..at], &scope[..at + sep.len()]]);
        }
    }
    serde_json::to_string(&ancestors).expect("string list")
}

/// Keep a scan's unpublished memberships live until it reports an outcome. A process
/// that exits releases the collection lock, so upkeep can discard abandoned rows.
pub struct AnalysisRun {
    pub owner: AnalysisOwner,
    store: ContentStore,
    lease: Option<CollectionLease>,
    scan_id: String,
    scope_value: String,
    source: Option<RunSource>,
    pub publication: Option<Publication>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Publication { Published, Released, Aborted, Unavailable }

impl Publication {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Published => "published", Self::Released => "released",
            Self::Aborted => "aborted", Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunSource {
    Directory(String),
    Git { repository: String, tree: String, commit: Option<String> },
}

impl RunSource {
    pub fn directory(path: &Path) -> Self {
        Self::Directory(path.canonicalize().unwrap_or_else(|_| path.to_owned()).to_string_lossy().into_owned())
    }
}

#[derive(Default)]
pub struct AnalysisRecords {
    pub scopes: Vec<(String, String)>,
    pub runs: Vec<CompletedRun>,
}

pub struct CompletedRun {
    pub scope: String,
    pub value: String,
    pub acquisition: i64,
    pub scan_id: String,
    pub directory: Option<String>,
    pub repository: Option<String>,
    pub tree: Option<String>,
    pub commit: Option<String>,
    pub snapshot: Option<i64>,
}

impl AnalysisRun {
    pub fn store_path(&self) -> &Path { self.store.database_path() }
    pub fn store_identity(&self) -> i64 { self.store.identity() }
    pub fn acquisition(&self) -> i64 { self.owner.acquisition }

    pub fn is_open(&self) -> bool { self.lease.is_some() }

    pub fn publication_warning(&self) -> Option<String> {
        self.owner.cache_warning().or_else(|| (self.publication == Some(Publication::Released))
            .then(|| "The analysis scope was released before publication.".to_owned()))
    }

    pub fn set_directory_scope(&mut self, path: &Path) {
        self.scope_value = directory(path)["cwd:".len()..].to_owned();
    }

    pub fn select_source(&mut self, source: RunSource) -> StoreResult<()> {
        if !self.is_open() || self.source.as_ref().is_some_and(|previous| previous != &source) {
            return Err(super::StoreError::Invalid("cannot change the selected source of an analysis run".into()));
        }
        self.source = Some(source);
        Ok(())
    }

    pub fn begin(store: ContentStore, scope: &str, id: &str) -> StoreResult<Self> {
        Self::begin_at(store, scope, id, 0)
    }

    pub fn begin_batch(store: ContentStore, scope: &str, source: RunSource) -> StoreResult<Self> {
        let mut run = Self::begin(store, scope, "batch")?;
        run.scan_id = format!("batch:{}:{}", run.store_identity(), run.owner.acquisition);
        run.select_source(source)?;
        Ok(run)
    }

    pub fn begin_at(store: ContentStore, scope: &str, id: &str, acquired_at: i64) -> StoreResult<Self> {
        let lease = store.hold_collection()?;
        let mut owner = store.begin_analysis_scope_at(scope, acquired_at)?;
        let pending = format!("pending:{}:{}:{id}", store.connection.identity(), owner.acquisition);
        store.immediate_transaction(|tx| {
            store.check_scope_store(tx)?;
            tx.execute_cached("INSERT INTO analysis_scope_pending(id) VALUES (?1)", [&pending])?;
            Ok(())
        })?;
        owner.pending = Some(pending);
        owner.warning = Some(Default::default());
        let value = scope.strip_prefix("named:").or_else(|| scope.strip_prefix("cwd:")).unwrap_or(scope);
        Ok(Self { owner, store, lease: Some(lease), scan_id: id.to_owned(), scope_value: value.to_owned(), source: None, publication: None })
    }

    pub fn finish(&mut self, succeeded: bool) -> StoreResult<()> {
        if self.lease.is_none() {
            return Ok(());
        }
        let owner = &self.owner;
        let pending = owner.membership();
        let (publication, mut warning) = self.store.immediate_transaction(|tx| {
            self.store.check_scope_store(tx)?;
            // Keep the same warning from the outcome decision through commit. A rollback leaves it open for retry.
            let warning = owner.warning.as_ref().map(|warning| warning.lock().unwrap_or_else(|e| e.into_inner()));
            let outcome = if warning.as_ref().is_some_and(|warning| warning.message.is_some()) { Publication::Unavailable }
                else if !succeeded { Publication::Aborted }
                else if !owner.current(tx)? { Publication::Released }
                else { Publication::Published };
            if outcome == Publication::Published {
                if let Some(source) = &self.source {
                    let (directory, repo, tree, commit) = match source {
                        RunSource::Directory(path) => (Some(path), None, None, None),
                        RunSource::Git { repository, tree, commit } => (None, Some(repository), Some(tree), commit.as_ref()),
                    };
                    tx.execute_cached("INSERT OR IGNORE INTO analysis_history_scope(scope,value,stop_scopes) VALUES (?1,?2,?3)",
                        (&owner.scope, &self.scope_value, receipt_scopes(&owner.scope)))?;
                    tx.execute_cached("INSERT INTO analysis_scope_run(scope,acquisition,acquired_at,scan_id,directory,repository,tree,git_commit)
                        VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                        (&owner.scope, owner.acquisition, owner.acquired_at, &self.scan_id, directory, repo, tree, commit))?;
                }
                tx.execute_cached("INSERT INTO analysis_scope_manifest(scope,manifest_id,acquired_at,acquisition,recorded_run)
                    SELECT ?1,manifest_id,acquired_at,acquisition,recorded_run FROM analysis_scope_manifest WHERE scope=?2
                    ON CONFLICT(scope,manifest_id) DO UPDATE SET
                      (acquired_at,acquisition,recorded_run)=(excluded.acquired_at,excluded.acquisition,excluded.recorded_run)
                    WHERE (acquired_at,acquisition,recorded_run)<(excluded.acquired_at,excluded.acquisition,excluded.recorded_run)", (&owner.scope, pending))?;
                let rows = tx.prepare_cached("SELECT workspace,snapshot_id FROM analysis_scope_workspace_snapshot WHERE scope=?1")?
                    .query_map([pending], |r| Ok((r.get::<_, String>(0)?, SnapshotId(r.get(1)?))))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                for (workspace, snapshot) in rows {
                    own_snapshot(tx, owner, &workspace, snapshot, &owner.scope)?;
                    if self.source.is_some() {
                        tx.execute_cached("INSERT OR IGNORE INTO analysis_run_workspace_snapshot(scope,acquisition,snapshot_id) VALUES (?1,?2,?3)",
                            (&owner.scope, owner.acquisition, snapshot.0))?;
                    }
                }
            }
            release_memberships(tx, "scope=?1", [pending])?;
            tx.execute_cached("DELETE FROM analysis_scope_pending WHERE id=?1", [pending])?;
            Ok((outcome, warning))
        })?;
        if let Some(warning) = warning.as_mut() { warning.finished = true; }
        drop(warning);
        self.publication = Some(publication);
        self.lease.take();
        Ok(())
    }
}

impl Drop for AnalysisRun {
    fn drop(&mut self) {
        let _ = self.finish(false);
    }
}

pub fn named(name: &str) -> Option<String> {
    (!name.trim().is_empty()).then(|| format!("named:{name}"))
}

pub fn directory(path: &Path) -> String {
    directory_in(path, None)
}

pub fn directory_in(path: &Path, home: Option<&str>) -> String {
    recorded_directory(&path.canonicalize().unwrap_or_else(|_| path.to_owned()), home)
}

// A committed stop must use the path captured at enqueue, even after a rename or symlink change.
fn recorded_directory(path: &Path, home: Option<&str>) -> String {
    let value = path.to_string_lossy();
    match home {
        Some(home) => format!("cwd:{home}:{value}"),
        None => format!("cwd:{value}"),
    }
}

fn scope_prefix(scope: &str) -> String {
    let separator = std::path::MAIN_SEPARATOR;
    if scope.ends_with(separator) { scope.to_owned() } else { format!("{scope}{separator}") }
}

/// A route from before the directory registry must have actual ownership rows.
/// Only a successful query can establish that an old store holds no matching rows.
pub fn existing_directory_ownership(store: &Path, directory: &Path, home: &str) -> rusqlite::Result<bool> {
    Ok(owning_directory_store(store, directory, home)?.is_some())
}

// Some(None) identifies a real owner from before store identities were recorded.
pub fn owning_directory_store(store: &Path, directory: &Path, home: &str) -> rusqlite::Result<Option<Option<i64>>> {
    owning_recorded_directory_store(store, &directory.canonicalize().unwrap_or_else(|_| directory.to_owned()), home)
}

pub fn owning_recorded_directory_store(store: &Path, directory: &Path, home: &str) -> rusqlite::Result<Option<Option<i64>>> {
    let db = rusqlite::Connection::open_with_flags(store, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let legacy = recorded_directory(directory, None);
    let scoped = recorded_directory(directory, Some(home));
    let scoped_prefix = scope_prefix(&scoped);
    let legacy_prefix = scope_prefix(&legacy);
    for table in ["analysis_scope_manifest", "analysis_scope_snapshot", "analysis_scope_workspace_snapshot", "analysis_scope_run"] {
        let exists: bool = db.query_row(
            "SELECT EXISTS (SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)", [table], |row| row.get(0),
        )?;
        if !exists {
            continue;
        }
        let has_owner: bool = db.query_row(&format!(
            "SELECT EXISTS (SELECT 1 FROM {table}
             WHERE scope = ?1 OR scope = ?2
                OR substr(scope,1,length(?3)) = ?3
                OR substr(scope,1,length(?4)) = ?4)"
        ), (&scoped, &legacy, &scoped_prefix, &legacy_prefix), |row| row.get(0))?;
        if has_owner {
            let identified: bool = db.query_row("SELECT EXISTS (SELECT 1 FROM sqlite_schema WHERE type='table' AND name='store_identity')", [], |row| row.get(0))?;
            let identity = if identified { Some(db.query_row("SELECT id FROM store_identity WHERE singleton=1", [], |row| row.get(0))?) } else { None };
            return Ok(Some(identity));
        }
    }
    Ok(None)
}

pub fn acquisition_time() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |t| t.as_micros() as i64)
}

pub(super) fn next_acquisition(tx: &rusqlite::Connection) -> rusqlite::Result<i64> {
    tx.query_row_cached("UPDATE store_state SET analysis_acquisitions=analysis_acquisitions+1
        WHERE singleton=1 RETURNING analysis_acquisitions", [], |row| row.get(0))
}

// Older memberships have only the scope key. An absolute directory can itself contain a colon.
fn legacy_scope_value(scope: &str) -> &str {
    if let Some(name) = scope.strip_prefix("named:") { return name; }
    let value = scope.strip_prefix("cwd:").unwrap_or(scope);
    if value.starts_with(['/', '\\']) { return value; }
    match value.split_once(':') {
        Some((drive, path)) if drive.len() == 1 && drive.as_bytes()[0].is_ascii_alphabetic() && path.starts_with('\\') => value,
        Some((_, path)) => path,
        None => value,
    }
}

fn completed_runs(db: &rusqlite::Connection) -> StoreResult<Vec<CompletedRun>> {
    let mut statement = db.prepare_cached("SELECT r.scope,h.value,r.acquisition,r.scan_id,r.directory,r.repository,r.tree,r.git_commit,s.snapshot_id
        FROM analysis_scope_run r JOIN analysis_history_scope h USING(scope)
        LEFT JOIN analysis_run_workspace_snapshot s USING(scope,acquisition) ORDER BY r.scope,r.acquisition,s.snapshot_id")?;
    let rows = statement.query_map([], |r| Ok(CompletedRun {
        scope: r.get(0)?, value: r.get(1)?, acquisition: r.get(2)?, scan_id: r.get(3)?,
        directory: r.get(4)?, repository: r.get(5)?, tree: r.get(6)?, commit: r.get(7)?, snapshot: r.get(8)?,
    }))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

impl ContentStore {
    pub fn completed_analysis_runs(&self) -> StoreResult<Vec<CompletedRun>> {
        Ok(self.analysis_records(false, true)?.runs)
    }

    pub fn analysis_records(&self, scopes: bool, runs: bool) -> StoreResult<AnalysisRecords> {
        let mut db = self.connection.lock().map_err(super::StoreError::Invalid)?;
        let tx = db.transaction()?;
        self.check_scope_store(&tx)?;
        let mut records = AnalysisRecords::default();
        if scopes {
            let mut statement = tx.prepare_cached("WITH owners(scope) AS (
                SELECT scope FROM analysis_history_scope
                UNION SELECT scope FROM analysis_scope_manifest WHERE scope GLOB 'named:*' OR scope GLOB 'cwd:*'
                UNION SELECT scope FROM analysis_scope_snapshot WHERE scope GLOB 'named:*' OR scope GLOB 'cwd:*'
                UNION SELECT scope FROM analysis_scope_workspace_snapshot WHERE scope GLOB 'named:*' OR scope GLOB 'cwd:*'
            ) SELECT o.scope,h.value FROM owners o LEFT JOIN analysis_history_scope h USING(scope) ORDER BY o.scope")?;
            records.scopes = statement.query_map([], |row| {
                let scope: String = row.get(0)?;
                let value = row.get::<_, Option<String>>(1)?.unwrap_or_else(|| legacy_scope_value(&scope).to_owned());
                Ok((scope, value))
            })?.collect::<rusqlite::Result<_>>()?;
        }
        if runs {
            records.runs = completed_runs(&tx)?;
        }
        tx.commit()?;
        Ok(records)
    }

    pub(super) fn check_scope_store(&self, db: &rusqlite::Connection) -> StoreResult<()> {
        let actual: i64 = db.query_row_cached("SELECT id FROM store_identity WHERE singleton=1", [], |row| row.get(0))?;
        if actual != self.identity() {
            return Err(super::StoreError::Invalid(format!("analysis store identity changed at {}", self.database_path().display())));
        }
        Ok(())
    }

    pub fn directory_release_cutoff(&self, path: &Path, home: &str) -> StoreResult<Option<i64>> {
        let scope = directory_in(path, Some(home));
        let db = self.connection.lock().map_err(super::StoreError::Invalid)?;
        self.check_scope_store(&db)?;
        Ok(db.query_row_cached("SELECT max(before) FROM analysis_scope_release
            WHERE scope IN (SELECT value FROM json_each(?1))", [receipt_scopes(&scope)], |row| row.get(0))?)
    }

    pub fn identity(&self) -> i64 { self.connection.identity() }

    pub fn begin_analysis_scope(&self, scope: &str) -> StoreResult<AnalysisOwner> {
        self.begin_analysis_scope_at(scope, 0)
    }

    pub fn begin_analysis_scope_at(&self, scope: &str, acquired_at: i64) -> StoreResult<AnalysisOwner> {
        self.immediate_transaction(|tx| {
            self.check_scope_store(tx)?;
            tx.execute_cached("INSERT OR IGNORE INTO analysis_scope_generation VALUES (?1,0)", [scope])?;
            let generation = tx.query_row_cached(
                "SELECT generation FROM analysis_scope_generation WHERE scope=?1", [scope], |row| row.get(0),
            )?;
            let acquisition = next_acquisition(tx)?;
            Ok(AnalysisOwner { scope: scope.to_owned(), generation, store_identity: Some(self.identity()), pending: None, acquired_at, acquisition, warning: None })
        })
    }

    pub fn own_analysis_snapshot(&self, owner: &AnalysisOwner, workspace: &str, snapshot: SnapshotId) -> StoreResult<()> {
        self.immediate_transaction(|tx| {
            if !owner.current(tx)? {
                return Ok(());
            }
            own_snapshot(tx, owner, workspace, snapshot, owner.membership())?;
            Ok(())
        })
    }

    /// Called with the exclusive collection lease held.
    pub(super) fn discard_abandoned_analysis(tx: &rusqlite::Connection) -> rusqlite::Result<()> {
        release_memberships(tx, "scope GLOB ?1", ["pending:*"])?;
        tx.execute_cached("DELETE FROM analysis_scope_pending", [])?;
        Ok(())
    }
}

fn own_snapshot(tx: &rusqlite::Connection, owner: &AnalysisOwner, workspace: &str, snapshot: SnapshotId, scope: &str) -> rusqlite::Result<()> {
            if scope.starts_with("pending:") || owner.scope.starts_with("cwd:") {
                tx.execute_cached(
                    "INSERT OR IGNORE INTO analysis_scope_collect SELECT snapshot_id FROM analysis_scope_workspace_snapshot \
                     WHERE scope=?1 AND workspace=?2 AND snapshot_id<>?3 AND (acquired_at,acquisition)<=(?4,?5)",
                    (scope, workspace, snapshot.0, owner.acquired_at, owner.acquisition),
                )?;
                tx.execute_cached(
                    "INSERT INTO analysis_scope_workspace_snapshot(scope,workspace,snapshot_id,acquired_at,acquisition) VALUES (?1,?2,?3,?4,?5) \
                     ON CONFLICT(scope,workspace) DO UPDATE SET snapshot_id=excluded.snapshot_id,acquired_at=excluded.acquired_at,acquisition=excluded.acquisition \
                     WHERE (acquired_at,acquisition)<=(excluded.acquired_at,excluded.acquisition)",
                    (scope, workspace, snapshot.0, owner.acquired_at, owner.acquisition),
                )?;
            } else {
                tx.execute_cached("INSERT INTO analysis_scope_snapshot(scope,snapshot_id,acquired_at) VALUES (?1,?2,?3) \
                    ON CONFLICT(scope,snapshot_id) DO UPDATE SET acquired_at=max(acquired_at,excluded.acquired_at)", (scope, snapshot.0, owner.acquired_at))?;
            }
            Ok(())
}

pub struct DirectoryRelease<'a> {
    id: Option<&'a str>,
    before: Option<i64>,
    home: Option<&'a str>,
}

pub const MAX_GENERATION: i64 = (1_i64 << 53) - 1;

impl<'a> DirectoryRelease<'a> {
    pub fn parse(id: Option<&'a str>) -> StoreResult<Self> {
        let invalid = || super::StoreError::Invalid("invalid directory release receipt".into());
        if id == Some("") { return Err(invalid()); }
        let Some((cutoff, receipt)) = id.and_then(|value| value.split_once(':')) else {
            return Ok(Self { id, before: None, home: None });
        };
        let (delivery, home) = receipt.split_once(':').map_or((receipt, None), |(delivery, home)| (delivery, Some(home)));
        if delivery.is_empty() || home.is_some_and(|home| home.is_empty() || home.contains(':'))
            || !cutoff.bytes().all(|c| c.is_ascii_digit()) {
            return Err(invalid());
        }
        let before = cutoff.parse::<i64>().map_err(|_| invalid())?;
        if before > MAX_GENERATION { return Err(invalid()); }
        Ok(Self { id, before: Some(before), home })
    }

    fn apply_to(&self, tx: &rusqlite::Connection, path: &Path) -> StoreResult<usize> {
        let scope = recorded_directory(path, self.home);
        let subtree = scope_prefix(&scope);
        let legacy = self.home.map(|_| recorded_directory(path, None));
        release_analysis_scopes(tx, &scope, Some(&subtree), self.id, self.before, legacy.as_deref())
    }

    /// Pin this existing database for identity validation, schema upgrade and release.
    pub fn deliver(&self, file: &Path, directory: &Path, expected: Option<i64>) -> StoreResult<usize> {
        self.deliver_recorded(file, &directory.canonicalize().unwrap_or_else(|_| directory.to_owned()), expected)
    }

    pub fn deliver_recorded(&self, file: &Path, directory: &Path, expected: Option<i64>) -> StoreResult<usize> {
        with_existing_release_store(file, expected.as_slice(), |tx, _| self.apply_to(tx, directory))
    }
}

pub fn release_named_scope(file: &Path, name: &str, expected: &[i64]) -> StoreResult<(i64, bool)> {
    let scope = named(name).ok_or_else(|| super::StoreError::Invalid("analysis scope name must not be empty".into()))?;
    with_existing_release_store(file, expected, |tx, identity| {
        let history: bool = tx.query_row_cached("SELECT EXISTS (SELECT 1 FROM analysis_scope_run WHERE scope=?1)", [&scope], |row| row.get(0))?;
        let memberships = release_analysis_scopes(tx, &scope, None, None, None, None)?;
        Ok((identity, history || memberships > 0))
    })
}

/// Recover the registry update for one identified acquisition after a caller exits.
pub fn named_acquisition_state(file: &Path, name: &str, identity: i64, acquisition: i64, scan: &str) -> StoreResult<&'static str> {
    let db = rusqlite::Connection::open_with_flags(file, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let scope = named(name).ok_or_else(|| super::StoreError::Invalid("analysis scope name must not be empty".into()))?;
    let pending = format!("pending:{identity}:{acquisition}:{scan}");
    let state = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM analysis_scope_pending WHERE id=?2),
                EXISTS(SELECT 1 FROM analysis_scope_run WHERE scope=?3 AND acquisition=?4 AND scan_id=?5)
         FROM store_identity WHERE singleton=1 AND id=?1",
        (identity, pending, scope, acquisition, scan), |row| Ok((row.get::<_, bool>(0)?, row.get::<_, bool>(1)?)),
    )?;
    Ok(match state { (_, true) => "published", (true, _) => "pending", _ => "absent" })
}

/// Old registries recorded a store before a named run had published. Their pending IDs omit the scope.
pub fn legacy_named_route_state(file: &Path, name: &str, identity: i64) -> StoreResult<&'static str> {
    let db = rusqlite::Connection::open_with_flags(file, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let scope = named(name).ok_or_else(|| super::StoreError::Invalid("analysis scope name must not be empty".into()))?;
    Ok(db.query_row(
        "SELECT CASE WHEN EXISTS(SELECT 1 FROM analysis_scope_run WHERE scope=?2)
                       OR EXISTS(SELECT 1 FROM analysis_scope_manifest WHERE scope=?2)
                       OR EXISTS(SELECT 1 FROM analysis_scope_snapshot WHERE scope=?2)
                       OR EXISTS(SELECT 1 FROM analysis_scope_workspace_snapshot WHERE scope=?2) THEN 2
                     WHEN EXISTS(SELECT 1 FROM analysis_scope_pending) THEN 1 ELSE 0 END
         FROM store_identity WHERE singleton=1 AND id=?1",
        (identity, scope), |row| Ok(match row.get::<_, i64>(0)? { 2 => "published", 1 => "pending", _ => "absent" }),
    )?)
}

// Validate and modify the same open database throughout release, even if another file replaces it.
fn with_existing_release_store<T>(file: &Path, expected: &[i64], release: impl Fn(&rusqlite::Connection, i64) -> StoreResult<T>) -> StoreResult<T> {
    let (mut db, identity) = existing_analysis_store(file, expected)?;
    let atoms = super::atoms::Atoms::for_database(file, Some(identity));
    super::connection::retry_busy(&mut super::BusyRetries::default(), || {
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let result = release(&tx, identity)?;
        let upto = atoms.write_pending(&tx)?;
        tx.commit()?;
        atoms.committed(upto);
        Ok(result)
    })
}

fn existing_analysis_store(file: &Path, expected: &[i64]) -> StoreResult<(rusqlite::Connection, i64)> {
    let mut db = rusqlite::Connection::open_with_flags(file, rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE)?;
    let identity = super::connection::retry_busy(&mut super::BusyRetries::default(), || {
        if !expected.is_empty() {
            let actual: i64 = db.query_row_cached("SELECT id FROM store_identity WHERE singleton=1", [], |row| row.get(0))?;
            if !expected.contains(&actual) {
                return Err(super::StoreError::Invalid(format!("analysis store identity changed at {}", file.display())));
            }
        } else {
            let known: bool = db.query_row_cached("SELECT count(*)=3 FROM sqlite_schema
                WHERE type='table' AND name IN ('source_objects','corpus_state','corpus_manifests')", [], |row| row.get(0))?;
            if !known { return Err(super::StoreError::Invalid("expected an existing analysis store".into())); }
        }
        ContentStore::initialize(&mut db)
    })?;
    Ok((db, identity))
}

impl ContentStore {
    /// Open a maintenance target without creating a file or changing an unrelated database.
    pub fn open_for_collection(file: &Path, expected: &[i64]) -> StoreResult<Self> {
        let (db, identity) = existing_analysis_store(file, expected)?;
        let connection = super::connection::ThreadConnection::pinned(file.to_owned(), db, identity);
        let store = Self::with_connection(file.to_owned(), connection, super::atoms::Atoms::for_database(file, Some(identity)), None);
        #[cfg(not(target_family = "wasm"))]
        let store = {
            let mut store = store;
            store.pin_collection_lock()?;
            store
        };
        Ok(store)
    }

    pub fn delete_analysis_scope(&self, scope: &str) -> StoreResult<usize> {
        self.immediate_transaction(|tx| release_analysis_scopes(tx, scope, None, None, None, None))
    }

    pub fn delete_directory_analysis_scopes(&self, path: &Path, release: Option<&str>) -> StoreResult<usize> {
        let receipt = DirectoryRelease::parse(release)?;
        let directory = path.canonicalize().unwrap_or_else(|_| path.to_owned());
        self.immediate_transaction(|tx| receipt.apply_to(tx, &directory))
    }

    pub(super) fn analysis_snapshot_due(&self, now: i64) -> StoreResult<bool> {
        let connection = self.connection.lock().map_err(super::StoreError::Invalid)?;
        Ok(connection.query_row_cached(
            "SELECT EXISTS (SELECT 1 FROM analysis_scope_pending) OR EXISTS (SELECT 1 FROM analysis_scope_manifest WHERE scope GLOB 'pending:*') \
             OR EXISTS (SELECT 1 FROM analysis_scope_workspace_snapshot WHERE scope GLOB 'pending:*') \
             OR EXISTS (SELECT 1 FROM analysis_scope_collect q \
             WHERE NOT EXISTS (SELECT 1 FROM analysis_scope_snapshot a WHERE a.snapshot_id=q.snapshot_id) \
               AND NOT EXISTS (SELECT 1 FROM analysis_scope_workspace_snapshot a WHERE a.snapshot_id=q.snapshot_id) \
               AND NOT EXISTS (SELECT 1 FROM workspaces w WHERE w.snapshot_id=q.snapshot_id AND w.lease_until>=?1 AND w.state_id=1))",
            [now], |row| row.get(0),
        )?)
    }
}

fn release_analysis_scopes(tx: &rusqlite::Connection, scope: &str, subtree: Option<&str>, release: Option<&str>, before: Option<i64>, legacy: Option<&str>) -> StoreResult<usize> {
    if let Some(cutoff) = before {
        let covered: bool = tx.query_row_cached("SELECT EXISTS (SELECT 1 FROM analysis_scope_release
            WHERE scope=?1 AND before>=?2)", (scope, cutoff), |row| row.get(0))?;
        if covered { return Ok(0); }
        tx.execute_cached("DELETE FROM analysis_scope_release WHERE scope=?1 AND before<?2", (scope, cutoff))?;
    }
    if let Some(release) = release
        && tx.execute_cached("INSERT OR IGNORE INTO analysis_scope_release(id,scope,before) VALUES (?1,?2,?3)",
            (release, if before.is_some() { scope } else { "" }, before.unwrap_or_default()))? == 0
    {
        return Ok(0);
    }
    const RELEASING: &str = "(scope=?1 OR (?2 IS NOT NULL AND substr(scope,1,length(?2))=?2))";
    if let Some(before) = before {
        let predicate = format!("{RELEASING} AND acquired_at<=?3");
        let mut released = release_memberships(tx, &predicate, (scope, subtree, before))?;
        // Entries made before scopes recorded their home still use the bare directory.
        if let Some(legacy) = legacy {
            let subtree = scope_prefix(legacy);
            released += release_memberships(tx, &predicate, (legacy, Some(subtree.as_str()), before))?;
        }
        Ok(released)
    } else {
        let args = (scope, subtree);
        tx.execute_cached(&format!("UPDATE analysis_scope_generation SET generation=generation+1 WHERE {RELEASING}"), args)?;
        if scope.starts_with("named:") {
            tx.execute_cached("DELETE FROM analysis_scope_generation WHERE scope=?1", [scope])?;
        }
        Ok(release_memberships(tx, RELEASING, args)?)
    }
}

fn release_memberships(tx: &rusqlite::Connection, predicate: &str, args: impl rusqlite::Params + Copy) -> rusqlite::Result<usize> {
            for table in ["analysis_scope_snapshot", "analysis_scope_workspace_snapshot"] {
                tx.execute_cached(&format!("INSERT OR IGNORE INTO analysis_scope_collect SELECT snapshot_id FROM {table} WHERE {predicate}"), args)?;
            }
            let snapshots = tx.execute_cached(&format!("DELETE FROM analysis_scope_snapshot WHERE {predicate}"), args)?;
            let workspaces = tx.execute_cached(&format!("DELETE FROM analysis_scope_workspace_snapshot WHERE {predicate}"), args)?;
            // Compute the last owners before deleting the membership rows.
            let unowned = format!(
                "SELECT manifest_id FROM analysis_scope_manifest WHERE ({predicate})
                 EXCEPT SELECT manifest_id FROM analysis_scope_manifest WHERE NOT ({predicate})"
            );
            tx.execute_cached(&format!("DELETE FROM corpus_manifest_plans WHERE manifest_id IN ({unowned})"), args)?;
            let manifests = tx.execute_cached(&format!("DELETE FROM corpus_manifests WHERE manifest_id IN ({unowned})"), args)?;
            let memberships = tx.execute_cached(&format!("DELETE FROM analysis_scope_manifest WHERE {predicate}"), args)?;
            if snapshots + workspaces + manifests > 0 {
                tx.execute_cached("UPDATE corpus_state SET due=1 WHERE singleton=1", [])?;
                tx.execute_cached("UPDATE corpus_decided SET time=0 WHERE singleton=1", [])?;
            }
            Ok(snapshots + workspaces + manifests + memberships)
}
