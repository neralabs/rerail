//! The analysis scope's base (rerail-store-base): an owner of a scope (`AnalysisOwner`, its acquisition and
//! generation), a run's source (`RunSource`) and the scopes' names (`named`, `directory`), which git reads with the
//! observer (rerail-observe). `analysis_scope.rs` (rerail-store) re-exports them.
use super::super::Cached as _;
use rerail_store::Canonical as _;
use std::path::Path;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct AnalysisOwner {
    pub scope: String,
    pub generation: i64,
    #[serde(default)]
    pub store_identity: Option<i64>,
    #[serde(default)]
    pub pending: Option<String>,
    /// Acquisition generation from the control database. Keeps its original serialized field name.
    #[serde(default)]
    pub acquired_at: i64,
    #[serde(default)]
    pub acquisition: i64,
    /// A run's owner (decision 0547): whether its first write creates the scope's generation (`generation`, its
    /// lease's), none being there when it started. A run writes nothing as it starts; each of its writes acquires
    /// first (`acquire`).
    #[serde(default)]
    pub create: bool,
    #[serde(skip)]
    pub warning: Option<std::sync::Arc<std::sync::Mutex<RunWarning>>>,
}

#[derive(Debug, Default)]
pub struct RunWarning {
    pub message: Option<String>,
    pub finished: bool,
}

impl AnalysisOwner {
    pub fn membership(&self) -> &str {
        self.pending.as_deref().unwrap_or(&self.scope)
    }

    pub fn acquired_at(&self) -> i64 { self.acquired_at }

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

    /// Whether this owner may still publish. A run's lease (`scope_lease`) keeps it live: no deletion or stop that
    /// would revoke it runs meanwhile, and collection keeps its staged rows under the run's collection lease.
    pub fn current(&self, tx: &rusqlite::Connection) -> rusqlite::Result<bool> {
        self.current_as(tx, false)
    }

    /// `current`, as it will be once the owner has acquired (`planned`): a run that has written nothing yet reads
    /// the generation its first write would create.
    pub fn current_as(&self, tx: &rusqlite::Connection, planned: bool) -> rusqlite::Result<bool> {
        // A run's staging key closes as it finishes: a worker's late write finds its lease gone (decision 0547).
        if self.pending.is_some() && !tx.path().is_some_and(|p| super::super::scope_lease::live(Path::new(p), Some(self.acquisition))) {
            return Ok(false);
        }
        let mut now: i64 = tx.query_row_cached("SELECT coalesce((SELECT generation FROM analysis_scope_generation WHERE scope=?1),-1)",
            [&self.scope], |row| row.get(0))?;
        if planned && now < 0 && self.create {
            now = planned_generation(tx, &self.scope, self.acquired_at, self.generation)?;
        }
        if now < 0 || now != self.generation {
            return Ok(false);
        }
        tx.query_row_cached(
            "SELECT (?4 IS NULL OR (SELECT id FROM store_identity WHERE singleton=1)=?4) \
                AND NOT EXISTS (SELECT 1 FROM analysis_scope_release WHERE scope IN (SELECT value FROM json_each(?3)) AND before>=?2) \
                AND NOT EXISTS (SELECT 1 FROM analysis_scope_release WHERE legacy_scope IN (SELECT value FROM json_each(?3))
                    AND before>=?2 AND legacy_acquisition>=?5)",
            (&self.scope, self.acquired_at, receipt_scopes(&self.scope), self.store_identity, self.acquisition), |row| row.get(0),
        )
    }

    /// A run's acquisition, written by its first write (and again, idempotently, by each later one): what its start
    /// read and planned (decision 0547). An owner that acquired as it began (`begin_analysis_scope_at`) has nothing
    /// to write.
    pub fn acquire(&self, tx: &rusqlite::Connection) -> rusqlite::Result<()> {
        if self.pending.is_none() {
            return Ok(());
        }
        acquire_rows(tx, &self.scope, self.acquired_at, self.acquisition, self.create.then_some(self.generation))
    }
}

/// The rows of an acquisition of `scope`: the generation it creates (`create`: none was there as the run started;
/// a number of the acquisitions' sequence, so the seeding trigger leaves it), the directory's highest control
/// generation, and the store's acquisition counter at least `acquisition` and the generation.
pub fn acquire_rows(tx: &rusqlite::Connection, scope: &str, acquired_at: i64, acquisition: i64, create: Option<i64>) -> rusqlite::Result<()> {
    let create = create.filter(|g| *g > 0);
    if scope.starts_with("cwd:") {
        // A legacy INSERT OR IGNORE clears the known bound even on conflict. Current
        // acquisitions insert only when a new generation is actually required.
        let inserted = if let Some(generation) = create {
            tx.execute_cached("INSERT INTO analysis_scope_generation
                SELECT ?1,?4 WHERE NOT EXISTS (SELECT 1 FROM analysis_scope_generation WHERE scope=?1)
                AND NOT EXISTS (SELECT 1 FROM analysis_scope_release WHERE scope IN (SELECT value FROM json_each(?2)) AND before>=?3)",
                (scope, receipt_scopes(scope), acquired_at, generation))?
        } else {
            0
        };
        if inserted != 0 {
            tx.execute_cached("INSERT INTO analysis_directory_acquisition VALUES (?1,?2)", (scope, acquired_at))?;
        } else {
            tx.execute_cached("UPDATE analysis_directory_acquisition SET max_control_generation=?2 WHERE scope=?1 AND max_control_generation<?2",
                (scope, acquired_at))?;
        }
    } else if let Some(generation) = create {
        tx.execute_cached("INSERT OR IGNORE INTO analysis_scope_generation VALUES (?1,?2)", (scope, generation))?;
    }
    tx.execute_cached("UPDATE store_state SET analysis_acquisitions=max(analysis_acquisitions,?1,?2) WHERE singleton=1",
        (acquisition, create.unwrap_or(0)))?;
    Ok(())
}

/// The generation an acquisition of `scope` at `acquired_at` creates where none is: `made`, or for a directory a
/// stop with a later cutoff has already covered, none (-1).
pub fn planned_generation(tx: &rusqlite::Connection, scope: &str, acquired_at: i64, made: i64) -> rusqlite::Result<i64> {
    if !scope.starts_with("cwd:") {
        return Ok(made);
    }
    let covered: bool = tx.query_row_cached("SELECT EXISTS (SELECT 1 FROM analysis_scope_release
        WHERE scope IN (SELECT value FROM json_each(?1)) AND before>=?2)", (receipt_scopes(scope), acquired_at), |row| row.get(0))?;
    Ok(if covered { -1 } else { made })
}

// A stop covers its directory and descendants, including stops at a volume root.
pub fn receipt_scopes(scope: &str) -> String {
    let mut ancestors = Vec::new();
    if scope.starts_with("cwd:") {
        ancestors.push(scope);
        for (at, sep) in scope.match_indices(std::path::MAIN_SEPARATOR) {
            ancestors.extend([&scope[..at], &scope[..at + sep.len()]]);
        }
    }
    serde_json::to_string(&ancestors).expect("string list")
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum RunSource {
    Directory(String),
    Git { repository: String, tree: String, commit: Option<String> },
}

impl RunSource {
    pub fn directory(path: &Path) -> Self {
        Self::Directory(directory_value(path))
    }
}

pub fn named(name: &str) -> Option<String> {
    (!name.trim().is_empty()).then(|| format!("named:{name}"))
}

pub fn directory(path: &Path) -> String {
    directory_in(path, None)
}

pub fn directory_in(path: &Path, home: Option<&str>) -> String {
    recorded_directory(&directory_value(path), home)
}

#[cfg(unix)]
pub fn directory_value(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let path = path.ancestors().find_map(|ancestor| {
        let mut existing = ancestor.canonical().ok()?;
        existing.extend(path.strip_prefix(ancestor).ok()?.components());
        Some(existing)
    }).unwrap_or_else(|| path.components().collect());
    if let Some(text) = path.to_str() && !text.contains(char::REPLACEMENT_CHARACTER) { return text.to_owned(); }
    path.as_os_str().as_bytes().split(|&b| b == b'/').map(|part| {
        if let Ok(text) = std::str::from_utf8(part) && !text.contains(char::REPLACEMENT_CHARACTER) { return text.to_owned(); }
        // A physical path cannot have a literal dot component. Keep separators visible for ancestor release.
        use std::fmt::Write as _;
        let mut value = String::from("./");
        for byte in part { write!(value, "{byte:02x}").expect("writing to a string"); }
        value
    }).collect::<Vec<_>>().join("/")
}

#[cfg(not(unix))]
pub fn directory_value(path: &Path) -> String {
    path.canonical().unwrap_or_else(|_| path.to_owned()).to_string_lossy().into_owned()
}

// A committed stop must use the path captured at enqueue, even after a rename or symlink change.
pub fn recorded_directory(value: &str, home: Option<&str>) -> String {
    match home {
        Some(home) => format!("cwd:{home}:{value}"),
        None => format!("cwd:{value}"),
    }
}

