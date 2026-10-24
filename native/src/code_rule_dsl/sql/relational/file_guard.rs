//! Atomic file metadata selection. The cursor never reads an AST or spends matcher work.
use super::super::{
    exec::Batch,
    terminals::{FactTerm, Terminal, Verdict, Verdicts},
};
use crate::code_rule_dsl::{language::FileCondition, tape::TapeDoc};
use ast_grep_core::{Node, meta_var::MetaVarEnv};
use rusqlite::{
    Connection, Error, Result,
    vtab::{
        Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
        VTabCursor, sqlite3_vtab, sqlite3_vtab_cursor,
    },
};
use std::{
    borrow::Cow,
    ffi::{CStr, c_int},
    marker::PhantomData,
    sync::Arc,
};

pub(super) struct Guard(Arc<FileCondition>);
impl Verdicts for Guard {
    fn file_condition(&self) -> Option<&FileCondition> {
        Some(&self.0)
    }
    fn rows<'t>(
        &self,
        _: Node<'t, TapeDoc>,
        _: &MetaVarEnv<'t, TapeDoc>,
    ) -> std::result::Result<Vec<Verdict>, String> {
        Err("file metadata requires the file selection cursor".into())
    }
}
pub(super) fn push(terminals: &mut Vec<Terminal>, condition: &Arc<FileCondition>) -> usize {
    let id = terminals.len();
    terminals.push(Terminal::Fact(FactTerm {
        verdicts: Box::new(Guard(Arc::clone(condition))),
        reads: vec![],
    }));
    id
}
thread_local! {static COUNTS:std::cell::Cell<[usize;2]>=const{std::cell::Cell::new([0;2])};}
pub(super) fn counts() -> [usize; 2] {
    COUNTS.get()
}
fn count(index: usize, amount: usize) {
    let mut counts = COUNTS.get();
    counts[index] += amount;
    COUNTS.set(counts);
}
fn invalid(message: &str) -> Error {
    Error::ModuleError(format!("file condition: {message}"))
}
pub(super) fn register(db: &Connection, batch: &Arc<Batch>) -> Result<()> {
    const MODULE: Module<'static, Table> = Module::eponymous_only_module();
    db.create_module(c"relation_file_guard", &MODULE, Some(Arc::clone(batch)))
}
#[repr(C)]
struct Table {
    base: sqlite3_vtab,
    batch: Arc<Batch>,
}
// SAFETY: SQLite base is first; each cursor retains its batch.
unsafe impl<'v> VTab<'v> for Table {
    type Aux = Arc<Batch>;
    type Cursor = Cursor<'v>;
    fn connect(
        db: &mut VTabConnection,
        batch: Option<&Self::Aux>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        Ok((
            Cow::Borrowed(c"CREATE TABLE x(file INTEGER,term INTEGER HIDDEN)"),
            Self {
                base: sqlite3_vtab::default(),
                batch: batch.ok_or_else(|| invalid("missing batch"))?.clone(),
            },
        ))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let term = info.constraints().enumerate().find_map(|(index, c)| {
            (c.is_usable()
                && c.column() == 1
                && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ)
                .then_some(index)
        });
        let Some(term) = term else {
            return Ok(false);
        };
        let mut usage = info.constraint_usage(term);
        usage.set_argv_index(1);
        usage.set_omit(true);
        info.set_estimated_cost(1.);
        info.set_estimated_rows(self.batch.files.len() as i64);
        Ok(true)
    }
    fn open(&'v mut self) -> Result<Self::Cursor> {
        Ok(Cursor {
            base: sqlite3_vtab_cursor::default(),
            batch: Arc::clone(&self.batch),
            files: vec![],
            position: 0,
            term: 0,
            marker: PhantomData,
        })
    }
}
#[repr(C)]
struct Cursor<'v> {
    base: sqlite3_vtab_cursor,
    batch: Arc<Batch>,
    files: Vec<u32>,
    position: usize,
    term: u32,
    marker: PhantomData<&'v Table>,
}
// SAFETY: SQLite base is first; results contain only original immutable file IDs.
unsafe impl VTabCursor for Cursor<'_> {
    fn filter(&mut self, _: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.files.clear();
        self.position = 0;
        self.term = args.get(0)?;
        let Some(Terminal::Fact(term)) = self.batch.program.terminals.get(self.term as usize)
        else {
            return Err(invalid("invalid terminal"));
        };
        let condition = term
            .verdicts
            .file_condition()
            .ok_or_else(|| invalid("terminal has no metadata condition"))?;
        count(0, 1);
        for (file, data) in self.batch.files.iter().enumerate() {
            count(1, 1);
            if condition.holds(
                data.context.path.as_deref().unwrap_or_default(),
                data.context.module.as_deref(),
            ) {
                self.files
                    .push(u32::try_from(file).map_err(|_| invalid("file identity overflow"))?);
            }
        }
        Ok(())
    }
    fn next(&mut self) -> Result<()> {
        self.position += 1;
        Ok(())
    }
    fn eof(&self) -> bool {
        self.position >= self.files.len()
    }
    fn column(&self, ctx: &mut Context, column: c_int) -> Result<()> {
        match column {
            0 => ctx.set_result(&self.files[self.position]),
            1 => ctx.set_result(&self.term),
            _ => Err(invalid("invalid column")),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(i64::from(self.files[self.position]))
    }
}
