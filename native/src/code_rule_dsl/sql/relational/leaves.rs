//! Atomic pattern, value, and capture-equality leaves. SQL owns all relation and Boolean composition.
use super::super::{exec::Batch, terminals::Terminal};
use crate::code_rule_dsl::matcher::node_at;
use ast_grep_core::matcher::MatcherExt;
use rusqlite::{Connection, Error, Result, functions::FunctionFlags};
use std::{
    panic::AssertUnwindSafe,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

thread_local! {
    pub(super) static VALUE_REQUESTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static EQUALITY_REQUESTS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

struct Leaves(AssertUnwindSafe<Arc<Batch>>);
impl Leaves {
    fn pattern(&self, term: usize, file: usize, node: u32) -> Result<bool> {
        let Some(Terminal::Pattern(term)) = self.0.program.terminals.get(term) else {
            return Err(Error::ModuleError(
                "relational pattern: invalid terminal".into(),
            ));
        };
        if !term.vars.is_empty() {
            return Err(Error::ModuleError(
                "relational pattern: capture-bearing terminal".into(),
            ));
        }
        let Some(file) = self.0.files.get(file) else {
            return Ok(false);
        };
        Ok(node < file.facts.len() && term.pattern.match_node(node_at(&file.root, node)).is_some())
    }
}

pub(in crate::code_rule_dsl::sql) fn register(
    db: &Connection,
    batch: &Arc<Batch>,
) -> Result<Arc<AtomicUsize>> {
    super::row::register(db)?;
    let leaves = Leaves(AssertUnwindSafe(Arc::clone(batch)));
    let calls = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&calls);
    let keys = Leaves(AssertUnwindSafe(Arc::clone(batch)));
    let values = Leaves(AssertUnwindSafe(Arc::clone(batch)));
    let equality = Leaves(AssertUnwindSafe(Arc::clone(batch)));
    db.create_scalar_function(
        "relation_value",
        3,
        FunctionFlags::SQLITE_UTF8,
        move |ctx| {
            VALUE_REQUESTS.set(VALUE_REQUESTS.get() + 1);
            let term = ctx.get::<u32>(0)? as usize;
            let file = ctx.get::<u32>(1)? as usize;
            let node = ctx.get::<u32>(2)?;
            let Some(Terminal::Fact(term)) = values.0.program.terminals.get(term) else {
                return Err(Error::ModuleError(
                    "relational value: invalid terminal".into(),
                ));
            };
            let file_data = &values.0.files[file];
            let rows = super::super::terminals::with_scopes(file, file_data, || {
                term.verdicts
                    .rows(node_at(&file_data.root, node), &Default::default())
            });
            match rows {
                Ok(rows) => Ok(rows.first().map_or(0i64, |r| if r.unknown { 2 } else { 1 })),
                Err(error) => super::failure::encode(&error).ok_or(Error::ModuleError(error)),
            }
        },
    )?;
    // Atomic structural equality, with native capture semantics. No rule composition runs here.
    db.create_scalar_function(
        "relation_capture_equal",
        3,
        FunctionFlags::SQLITE_UTF8,
        move |ctx| {
            let file = ctx.get::<u32>(0)? as usize;
            EQUALITY_REQUESTS.set(EQUALITY_REQUESTS.get() + 1);
            let left = ctx.get::<u32>(1)?;
            let right = ctx.get::<u32>(2)?;
            let file = &equality.0.files[file];
            let mut env = ast_grep_core::meta_var::MetaVarEnv::new();
            env.insert("capture", node_at(&file.root, left));
            Ok(env.insert("capture", node_at(&file.root, right)).is_some())
        },
    )?;
    db.create_scalar_function(
        "relation_leaf_atom",
        3,
        FunctionFlags::SQLITE_UTF8,
        move |ctx| {
            let file = ctx.get::<u32>(0)? as usize;
            let node = ctx.get::<u32>(1)?;
            let Some(file) = keys.0.files.get(file) else {
                return Err(Error::ModuleError("capture demand: missing file".into()));
            };
            if node >= file.facts.len() || !node_at(&file.root, node).is_named_leaf() {
                return Err(Error::ModuleError(
                    "capture demand: composite capture requires structural equality keys".into(),
                ));
            }
            ctx.get::<u32>(2)
        },
    )?;
    db.create_scalar_function(
        "relation_pattern",
        3,
        FunctionFlags::SQLITE_UTF8,
        move |ctx| {
            count.fetch_add(1, Ordering::Relaxed);
            leaves.pattern(
                ctx.get::<u32>(0)? as usize,
                ctx.get::<u32>(1)? as usize,
                ctx.get(2)?,
            )
        },
    )?;
    Ok(calls)
}
