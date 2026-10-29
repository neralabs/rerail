//! Immutable snapshot text access. SQL retains metadata and asks for source slices on demand.
use super::{Connection, Result, Snapshot, sql_error};
use rusqlite::functions::FunctionFlags;
use rusqlite::types::{ToSql, ToSqlOutput, ValueRef};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

struct Slice {
    source: Arc<String>,
    start: usize,
    end: usize,
}
impl ToSql for Slice {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        // The constructor checks UTF-8 boundaries. The Arc keeps this immutable
        // slice alive until SQLite copies its result; no intermediate String.
        Ok(ToSqlOutput::Borrowed(ValueRef::Text(
            &self.source.as_bytes()[self.start..self.end],
        )))
    }
}

pub(in crate::code_rule_dsl) fn charge(work: &AtomicUsize, count: usize) -> Result<()> {
    if work
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |left| {
            left.checked_sub(count)
        })
        .is_err()
        || crate::code_rule_dsl::budget::step_budget_exhausted()
    {
        return Err(super::invalid(
            "interrupted: relational native work budget exceeded",
        ));
    }
    Ok(())
}

pub(super) struct Binding {
    files: HashMap<String, Arc<String>>,
    work: Arc<AtomicUsize>,
}
pub(super) type SharedBinding = Arc<parking_lot::RwLock<Binding>>;
pub(super) fn binding() -> SharedBinding {
    Arc::new(parking_lot::RwLock::new(Binding {
        files: HashMap::new(),
        work: Arc::new(AtomicUsize::new(0)),
    }))
}
fn update(binding: &SharedBinding, snapshot: Option<&Snapshot>, work: Arc<AtomicUsize>) {
    let files = snapshot
        .into_iter()
        .flat_map(|s| &s.files)
        .map(|file| (file.path.clone(), Arc::clone(&file.source)))
        .collect();
    *binding.write() = Binding { files, work };
}
pub(super) fn bind(
    db: &super::Database,
    snapshot: Option<&Snapshot>,
    work: Arc<AtomicUsize>,
) -> Result<()> {
    update(&db.source_binding, snapshot, work);
    if !db.source_installed.get() {
        install_bound(db, db.source_binding.clone())?;
        db.source_installed.set(true);
    }
    Ok(())
}
fn install_bound(db: &Connection, binding: SharedBinding) -> Result<()> {
    db.create_scalar_function(
        c"source_text",
        3,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        move |ctx| {
            let binding = binding.read();
            let work = &binding.work;
            let files = &binding.files;
            charge(work, 1).map_err(|e| rusqlite::Error::UserFunctionError(e.reason.into()))?;
            let path = match ctx.get_raw(0) {
                ValueRef::Null => None,
                ValueRef::Text(bytes) => Some(
                    std::str::from_utf8(bytes)
                        .map_err(|e| rusqlite::Error::UserFunctionError(e.into()))?,
                ),
                _ => {
                    return Err(rusqlite::Error::UserFunctionError(
                        "source_text path must be text".into(),
                    ));
                }
            };
            let start = ctx
                .get::<Option<i64>>(1)?
                .and_then(|v| usize::try_from(v).ok());
            let end = ctx
                .get::<Option<i64>>(2)?
                .and_then(|v| usize::try_from(v).ok());
            let source = path.and_then(|p| files.get(p));
            let text = source.and_then(|s| s.get(start?..end?));
            if let Some(text) = text {
                charge(work, text.len().div_ceil(256))
                    .map_err(|e| rusqlite::Error::UserFunctionError(e.reason.into()))?;
            }
            Ok(text.map(|_| Slice {
                source: Arc::clone(source.unwrap()),
                start: start.unwrap(),
                end: end.unwrap(),
            }))
        },
    )
    .map_err(sql_error)
}
