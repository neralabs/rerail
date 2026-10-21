//! A heterogeneous leaf batch: one xFilter receives typed (term,file,node) demand IDs across
//! all rules. It evaluates regex leaves only; composition and rule decisions stay in SQL.
use crate::code_rule_dsl::{
    matcher::node_at,
    sql::{exec::Batch, frames::FrameStore, terminals::Terminal},
};
use rusqlite::{
    Connection, Error, Result,
    fallible_iterator::FallibleIterator,
    vtab::{
        Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
        VTabCursor, sqlite3_vtab, sqlite3_vtab_cursor,
    },
};
use std::{
    borrow::Cow,
    ffi::{CStr, c_int},
    marker::PhantomData,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Default)]
pub(super) struct Counts {
    pub calls: AtomicUsize,
    pub demands: AtomicUsize,
}
#[derive(Clone)]
struct Aux {
    batch: Arc<Batch>,
    frames: Arc<FrameStore>,
    counts: Arc<Counts>,
}
#[repr(C)]
struct Table {
    base: sqlite3_vtab,
    aux: Aux,
}
unsafe impl<'v> VTab<'v> for Table {
    type Aux = Aux;
    type Cursor = Cursor<'v>;
    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Aux>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        Ok((
            Cow::Borrowed(c"CREATE TABLE x(k,input HIDDEN)"),
            Self {
                base: sqlite3_vtab::default(),
                aux: aux.unwrap().clone(),
            },
        ))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let (mut key, mut input) = (None, None);
        for (i, c) in info.constraints().enumerate() {
            if c.is_usable() && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ {
                match c.column() {
                    0 => key = Some(i),
                    1 => input = Some(i),
                    _ => {}
                }
            }
        }
        let (Some(key), Some(input)) = (key, input) else {
            info.set_estimated_cost(f64::MAX);
            return Ok(true);
        };
        if !info.is_in_constraint(key)? || !info.set_in_constraint(key, true)? {
            info.set_estimated_cost(f64::MAX);
            return Ok(true);
        }
        for (argv, i) in [input, key].into_iter().enumerate() {
            let mut use_ = info.constraint_usage(i);
            use_.set_argv_index(argv as i32 + 1);
            use_.set_omit(true);
        }
        info.set_idx_num(1);
        info.set_estimated_cost(100.);
        info.set_estimated_rows(100);
        Ok(true)
    }
    fn open(&'v mut self) -> Result<Cursor<'v>> {
        Ok(Cursor {
            base: sqlite3_vtab_cursor::default(),
            aux: self.aux.clone(),
            keys: Vec::new(),
            at: 0,
            marker: PhantomData,
        })
    }
}
#[repr(C)]
struct Cursor<'v> {
    base: sqlite3_vtab_cursor,
    aux: Aux,
    keys: Vec<i64>,
    at: usize,
    marker: PhantomData<&'v Table>,
}
unsafe impl VTabCursor for Cursor<'_> {
    fn filter(&mut self, index: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        if index != 1 {
            return Err(Error::ModuleError(
                "terminal_set needs a complete input set".into(),
            ));
        }
        self.aux.counts.calls.fetch_add(1, Ordering::Relaxed);
        let descriptor = args.get::<String>(0)?;
        let mut keys = Vec::new();
        let mut list = args.in_values(1)?;
        while let Some(value) = list.next()? {
            let rusqlite::types::ValueRef::Integer(key) = value else {
                return Err(Error::ModuleError(
                    "terminal_set needs integer demand IDs".into(),
                ));
            };
            keys.push(key);
        }
        keys.sort_unstable();
        keys.dedup();
        self.aux
            .counts
            .demands
            .fetch_add(keys.len(), Ordering::Relaxed);
        self.keys.clear();
        self.at = 0;
        let output = &mut self.keys;
        let mut invalid = false;
        self.aux.frames.pattern_inputs(
            &format!("{descriptor},term"),
            &keys,
            |key, file, node, values| {
                let Some(index) = values
                    .first()
                    .copied()
                    .flatten()
                    .and_then(|i| usize::try_from(i).ok())
                else {
                    invalid = true;
                    return;
                };
                let Some(Terminal::Regex(regex)) = self.aux.batch.program.terminals.get(index)
                else {
                    invalid = true;
                    return;
                };
                let Some(file) = self.aux.batch.files.get(file as usize) else {
                    invalid = true;
                    return;
                };
                if node >= file.facts.len() {
                    invalid = true;
                    return;
                }
                if regex.is_match(&node_at(&file.root, node).text()) {
                    output.push(key);
                }
            },
        )?;
        if invalid {
            return Err(Error::ModuleError(
                "terminal_set: invalid typed demand".into(),
            ));
        }
        Ok(())
    }
    fn next(&mut self) -> Result<()> {
        self.at += 1;
        Ok(())
    }
    fn eof(&self) -> bool {
        self.at >= self.keys.len()
    }
    fn column(&self, ctx: &mut Context, col: c_int) -> Result<()> {
        if col == 0 {
            ctx.set_result(&self.keys[self.at])
        } else {
            ctx.set_result(&rusqlite::types::Null)
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(self.at as i64 + 1)
    }
}
pub(super) fn register(
    connection: &Connection,
    batch: &Arc<Batch>,
    frames: &Arc<FrameStore>,
) -> Result<Arc<Counts>> {
    const MODULE: Module<'static, Table> = Module::eponymous_only_module();
    let counts = Arc::new(Counts::default());
    connection.create_module(
        c"terminal_set",
        &MODULE,
        Some(Aux {
            batch: Arc::clone(batch),
            frames: Arc::clone(frames),
            counts: Arc::clone(&counts),
        }),
    )?;
    Ok(counts)
}
