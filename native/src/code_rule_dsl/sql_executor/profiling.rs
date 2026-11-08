//! The shared SQL query catalogue and logic CPU spans, including failed session operations.
use super::super::logic::profile;
use super::*;

pub(super) struct Scope {
    _context: Option<super::super::sql_profile::Context>,
    _cpu: profile::Span,
}
pub(super) fn scope(db: &Database, stage: &str) -> Scope {
    Scope {
        _context: db.profile_scope(|| {
            let phase = if stage.starts_with("bind_") {
                "filePublication"
            } else if matches!(stage, "compile" | "lower" | "create_index") {
                "connectionSetup"
            } else if stage == "verification snapshot" {
                "verification"
            } else if stage == "reset" {
                "cleanup"
            } else {
                "relationalEvaluation"
            };
            json!({"scope":"sql-extension","stage":stage,"executionPhase":phase})
        }),
        _cpu: profile::span(|| stage.to_owned()),
    }
}
impl CodeRuleSqlExecutor {
    /// Rust probes can reuse the installed trace instead of replacing SQLite's sole callback.
    pub fn sql_profile_totals(&self) -> Option<[u64; 6]> {
        self.db.profiler().map(|p| p.totals())
    }
    pub(super) fn profiled<T>(
        &mut self,
        name: &str,
        f: impl FnOnce(&mut Self) -> Result<T>,
    ) -> Result<T> {
        let _scope = scope(&self.db, name);
        let _operation = self.db.profile_scope(|| json!({"operation":name}));
        let result = f(self);
        if let Some(p) = self.db.profiler() {
            p.result(name, result.as_ref().err().map(ToString::to_string));
        }
        self.db.profile_checkpoint();
        result
    }
    pub(super) fn profiled_read<T>(
        &self,
        name: &str,
        f: impl FnOnce(&Self) -> Result<T>,
    ) -> Result<T> {
        let _scope = scope(&self.db, name);
        let _operation = self.db.profile_scope(|| json!({"operation":name}));
        let result = f(self);
        if let Some(p) = self.db.profiler() {
            p.result(name, result.as_ref().err().map(ToString::to_string));
        }
        self.db.profile_checkpoint();
        result
    }
}
