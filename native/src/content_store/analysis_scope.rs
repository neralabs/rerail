//! The owners of corpus manifests and audit snapshots. Their rows are roots for collection.

use super::{CollectionLease, ContentStore, SnapshotId, StoreResult};
use super::connection::Cached as _;
use std::path::Path;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AnalysisOwner {
    pub scope: String,
    generation: i64,
    #[serde(default)]
    pending: Option<String>,
    /// Acquisition generation from the control database. Keeps its original serialized field name.
    #[serde(default)]
    acquired_at: i64,
    #[serde(default)]
    acquisition: i64,
    #[serde(skip)]
    warning: Option<std::sync::Arc<std::sync::Mutex<Option<String>>>>,
}

impl AnalysisOwner {
    pub fn membership(&self) -> &str {
        self.pending.as_deref().unwrap_or(&self.scope)
    }

    pub(super) fn acquired_at(&self) -> i64 { self.acquired_at }

    pub fn is_staged(&self) -> bool { self.warning.is_some() }

    pub fn warn(&self, message: String) -> bool {
        let Some(warning) = &self.warning else { return false };
        warning.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert(message);
        true
    }

    pub fn cache_warning(&self) -> Option<String> {
        self.warning.as_ref()?.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub(super) fn current(&self, tx: &rusqlite::Connection) -> rusqlite::Result<bool> {
        // Receipt keys are the exact directory and its ancestors, including a volume root.
        let mut ancestors = Vec::new();
        if self.scope.starts_with("cwd:") {
            ancestors.push(self.scope.as_str());
            for (at, sep) in self.scope.match_indices(std::path::MAIN_SEPARATOR) {
                ancestors.extend([&self.scope[..at], &self.scope[..at + sep.len()]]);
            }
        }
        tx.query_row_cached(
            "SELECT EXISTS (SELECT 1 FROM analysis_scope_generation WHERE scope=?1 AND generation=?2) \
                AND (?5 IS NULL OR EXISTS (SELECT 1 FROM analysis_scope_pending WHERE id=?5)) \
                AND NOT EXISTS (SELECT 1 FROM analysis_scope_release WHERE scope IN (SELECT value FROM json_each(?4)) AND before>=?3)",
            (&self.scope, self.generation, self.acquired_at, serde_json::to_string(&ancestors).expect("string list"), self.pending.as_deref()), |row| row.get(0),
        )
    }
}

/// Keep a scan's unpublished memberships live until it reports an outcome. A process
/// that exits releases the collection lock, so upkeep can discard abandoned rows.
pub struct AnalysisRun {
    pub owner: AnalysisOwner,
    store: ContentStore,
    lease: Option<CollectionLease>,
}

impl AnalysisRun {
    pub fn store_path(&self) -> &Path { self.store.database_path() }

    pub fn begin(store: ContentStore, scope: &str, id: &str) -> StoreResult<Self> {
        Self::begin_at(store, scope, id, 0)
    }

    pub fn begin_at(store: ContentStore, scope: &str, id: &str, acquired_at: i64) -> StoreResult<Self> {
        let lease = store.hold_collection()?;
        let mut owner = store.begin_analysis_scope_at(scope, acquired_at)?;
        let pending = format!("pending:{}:{}:{id}", store.connection.identity(), owner.acquisition);
        store.immediate_transaction(|tx| {
            tx.execute_cached("INSERT INTO analysis_scope_pending(id) VALUES (?1)", [&pending])?;
            Ok(())
        })?;
        owner.pending = Some(pending);
        owner.warning = Some(Default::default());
        Ok(Self { owner, store, lease: Some(lease) })
    }

    pub fn finish(&mut self, succeeded: bool) -> StoreResult<()> {
        if self.lease.is_none() {
            return Ok(());
        }
        let owner = &self.owner;
        let pending = owner.membership();
        self.store.immediate_transaction(|tx| {
            if succeeded && owner.cache_warning().is_none() && owner.current(tx)? {
                tx.execute_cached("INSERT INTO analysis_scope_manifest(scope,manifest_id,acquired_at) \
                    SELECT ?1,manifest_id,acquired_at FROM analysis_scope_manifest WHERE scope=?2 \
                    ON CONFLICT(scope,manifest_id) DO UPDATE SET acquired_at=max(acquired_at,excluded.acquired_at)", (&owner.scope, pending))?;
                let rows = tx.prepare_cached("SELECT workspace,snapshot_id FROM analysis_scope_workspace_snapshot WHERE scope=?1")?
                    .query_map([pending], |r| Ok((r.get::<_, String>(0)?, SnapshotId(r.get(1)?))))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                for (workspace, snapshot) in rows {
                    own_snapshot(tx, owner, &workspace, snapshot, &owner.scope)?;
                }
            }
            release_memberships(tx, "scope=?1", [pending])?;
            tx.execute_cached("DELETE FROM analysis_scope_pending WHERE id=?1", [pending])?;
            Ok(())
        })?;
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
    let path = path.canonicalize().unwrap_or_else(|_| path.to_owned());
    format!("cwd:{}", path.to_string_lossy())
}

pub fn directory_in(path: &Path, home: Option<&str>) -> String {
    let directory = directory(path);
    match home {
        Some(home) => format!("cwd:{home}:{}", &directory["cwd:".len()..]),
        None => directory,
    }
}

fn scope_prefix(scope: &str) -> String {
    let separator = std::path::MAIN_SEPARATOR;
    if scope.ends_with(separator) { scope.to_owned() } else { format!("{scope}{separator}") }
}

/// A route from before the directory registry must have actual ownership rows.
/// A file that never held a usable store has no such rows.
pub fn existing_directory_ownership(store: &Path, directory: &Path, home: &str) -> rusqlite::Result<bool> {
    fn check(store: &Path, directory: &Path, home: &str) -> rusqlite::Result<bool> {
        let db = rusqlite::Connection::open_with_flags(store, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let legacy = directory_in(directory, None);
        let scoped = directory_in(directory, Some(home));
        let scoped_prefix = scope_prefix(&scoped);
        let legacy_prefix = scope_prefix(&legacy);
        for table in ["analysis_scope_manifest", "analysis_scope_snapshot", "analysis_scope_workspace_snapshot"] {
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
                return Ok(true);
            }
        }
        Ok(false)
    }
    match check(store, directory, home) {
        Err(error) if matches!(error.sqlite_error_code(),
            Some(rusqlite::ErrorCode::NotADatabase)) => Ok(false),
        result => result,
    }
}

pub fn acquisition_time() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |t| t.as_micros() as i64)
}

impl ContentStore {
    pub fn begin_analysis_scope(&self, scope: &str) -> StoreResult<AnalysisOwner> {
        self.begin_analysis_scope_at(scope, 0)
    }

    pub fn begin_analysis_scope_at(&self, scope: &str, acquired_at: i64) -> StoreResult<AnalysisOwner> {
        self.immediate_transaction(|tx| {
            tx.execute_cached("INSERT OR IGNORE INTO analysis_scope_generation VALUES (?1,0)", [scope])?;
            let generation = tx.query_row_cached(
                "SELECT generation FROM analysis_scope_generation WHERE scope=?1", [scope], |row| row.get(0),
            )?;
            let acquisition = tx.query_row_cached("UPDATE store_state SET analysis_acquisitions=analysis_acquisitions+1
                WHERE singleton=1 RETURNING analysis_acquisitions", [], |row| row.get(0))?;
            Ok(AnalysisOwner { scope: scope.to_owned(), generation, pending: None, acquired_at, acquisition, warning: None })
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

impl ContentStore {
    pub fn delete_analysis_scope(&self, scope: &str) -> StoreResult<usize> {
        self.release_analysis_scopes(scope, None, None, None, None)
    }

    pub fn delete_directory_analysis_scopes(&self, path: &Path, release: Option<&str>) -> StoreResult<usize> {
        let (before, home) = release
            .and_then(|id| id.split_once(':'))
            .map_or((None, None), |(before, receipt)| {
                (before.parse::<i64>().ok(), receipt.split_once(':').map(|(_, home)| home))
            });
        let scope = directory_in(path, home);
        let subtree = scope_prefix(&scope);
        let legacy = home.map(|_| directory(path));
        self.release_analysis_scopes(&scope, Some(&subtree), release, before, legacy.as_deref())
    }

    fn release_analysis_scopes(&self, scope: &str, subtree: Option<&str>, release: Option<&str>, before: Option<i64>, legacy: Option<&str>) -> StoreResult<usize> {
        self.immediate_transaction(|tx| {
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
                Ok(release_memberships(tx, RELEASING, args)?)
            }
        })
    }

    pub(super) fn analysis_snapshot_due(&self, now: i64) -> StoreResult<bool> {
        let connection = self.connection.lock().expect("content store connection unavailable");
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
