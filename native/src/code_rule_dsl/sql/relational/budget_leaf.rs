//! Atomic value cursor. SQL supplies and receives the allowance; no host budget accumulator.
use super::super::{
    exec::Batch,
    terminals::{Terminal, with_scopes},
};
use crate::code_rule_dsl::matcher::node_at;
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

thread_local! { static TRACE: std::cell::RefCell<Option<Vec<serde_json::Value>>> = const { std::cell::RefCell::new(None) }; }
pub(super) fn record(value: serde_json::Value) {
    TRACE.with_borrow_mut(|trace| {
        if let Some(trace) = trace {
            trace.push(value);
        }
    });
}
pub(super) fn trace<T>(body: impl FnOnce() -> T) -> (T, Vec<serde_json::Value>) {
    struct Reset(Option<Vec<serde_json::Value>>);
    impl Drop for Reset {
        fn drop(&mut self) {
            TRACE.set(self.0.take());
        }
    }
    let _reset = Reset(TRACE.replace(Some(vec![])));
    let result = body();
    (
        result,
        TRACE.with_borrow_mut(|t| std::mem::take(t.as_mut().unwrap())),
    )
}

pub(super) fn register(db: &Connection, batch: &Arc<Batch>) -> Result<()> {
    // These are observations of one atomic cache-accounting counter, not rule callbacks.
    // They must remain nondeterministic so SQLite cannot hoist them out of a reached miss.
    let flags = rusqlite::functions::FunctionFlags::SQLITE_UTF8;
    db.create_scalar_function("relation_cache_fill_steps", 0, flags, |_| {
        i64::try_from(crate::code_rule_dsl::matcher::sql_cache_fill_steps())
            .map_err(|_| Error::ModuleError("cache-fill counter exceeds signed storage".into()))
    })?;
    db.create_scalar_function("relation_cache_fill_delta", 1, flags, |ctx| {
        let start = ctx.get::<i64>(0)?;
        let end = i64::try_from(crate::code_rule_dsl::matcher::sql_cache_fill_steps())
            .map_err(|_| Error::ModuleError("cache-fill counter exceeds signed storage".into()))?;
        end.checked_sub(start)
            .filter(|delta| start >= 0 && *delta >= 0)
            .ok_or_else(|| Error::ModuleError("cache-fill counter moved backwards".into()))
    })?;
    const MODULE: Module<'static, Table> = Module::eponymous_only_module();
    db.create_module(c"relation_value_budget", &MODULE, Some(Arc::clone(batch)))
}
#[repr(C)]
struct Table {
    base: sqlite3_vtab,
    batch: Arc<Batch>,
}
// SAFETY: SQLite base is first and the batch outlives its cursors.
unsafe impl<'v> VTab<'v> for Table {
    type Aux = Arc<Batch>;
    type Cursor = Cursor<'v>;
    fn connect(
        db: &mut VTabConnection,
        batch: Option<&Arc<Batch>>,
        _: &[u8],
        _: &[u8],
        _: &[u8],
        _: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        Ok((Cow::Borrowed(c"CREATE TABLE x(truth INTEGER,work INTEGER,term INTEGER HIDDEN,file INTEGER HIDDEN,node INTEGER HIDDEN,allowance INTEGER HIDDEN)"), Self { base: sqlite3_vtab::default(), batch: batch.unwrap().clone() }))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let mut inputs = [None; 4];
        for (index, c) in info.constraints().enumerate() {
            if c.is_usable()
                && (2..6).contains(&c.column())
                && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
            {
                inputs[(c.column() - 2) as usize] = Some(index);
            }
        }
        if inputs.iter().any(Option::is_none) {
            return Ok(false);
        }
        for (arg, index) in inputs.into_iter().enumerate() {
            let mut usage = info.constraint_usage(index.unwrap());
            usage.set_argv_index((arg + 1) as c_int);
            usage.set_omit(true);
        }
        info.set_estimated_cost(1.);
        info.set_estimated_rows(1);
        Ok(true)
    }
    fn open(&'v mut self) -> Result<Self::Cursor> {
        Ok(Cursor {
            base: sqlite3_vtab_cursor::default(),
            batch: self.batch.clone(),
            truth: 0,
            work: 0,
            done: true,
            marker: PhantomData,
        })
    }
}
#[repr(C)]
struct Cursor<'v> {
    base: sqlite3_vtab_cursor,
    batch: Arc<Batch>,
    truth: i64,
    work: i64,
    done: bool,
    marker: PhantomData<&'v Table>,
}
// SAFETY: SQLite base is first. Each filter owns its result, and keeps the batch alive.
unsafe impl VTabCursor for Cursor<'_> {
    fn filter(&mut self, _: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.done = true;
        let Some(allowance) = args.get::<Option<i64>>(3)? else {
            return Ok(());
        };
        let term = args.get::<u32>(0)? as usize;
        let term_id = term;
        let file = args.get::<u32>(1)? as usize;
        let node = args.get::<u32>(2)?;
        let work = usize::try_from(allowance)
            .map_err(|_| Error::ModuleError("negative work allowance".into()))?;
        let Some(Terminal::Fact(term)) = self.batch.program.terminals.get(term) else {
            return Err(Error::ModuleError("budget leaf: invalid terminal".into()));
        };
        super::leaves::VALUE_REQUESTS.set(super::leaves::VALUE_REQUESTS.get() + 1);
        let probes = crate::code_rule_dsl::matcher::value_probe::position();
        let ((verdict, left), text) = if let Some(file_data) = self.batch.files.get(file) {
            if node >= file_data.facts.len() {
                return Err(Error::ModuleError("budget leaf: invalid node".into()));
            }
            let at = node_at(&file_data.root, node);
            let text = at.text().into_owned();
            (
                with_scopes(file, file_data, || term.verdicts.budget_verdict(at, work)),
                text,
            )
        } else if let Some(sub) = self.batch.subfile(file) {
            if node >= sub.facts.len() {
                return Err(Error::ModuleError(
                    "budget leaf: invalid parsed node".into(),
                ));
            }
            let at = sub.root.adopt(sub.root.root().get_doc().node(node));
            let text = at.text().into_owned();
            let host = self
                .batch
                .files
                .get(sub.host)
                .ok_or_else(|| Error::ModuleError("budget leaf: invalid parsed host".into()))?;
            let _tables = sub
                .table
                .clone()
                .map(crate::code_rule_dsl::matcher::SubTables::enter);
            (
                with_scopes(sub.host, host, || {
                    term.verdicts.tree_budget_verdict(at, work)
                }),
                text,
            )
        } else {
            return Err(Error::ModuleError("budget leaf: invalid file".into()));
        };
        self.truth = match verdict {
            Ok(Some(true)) => 1,
            Ok(Some(false)) => 0,
            Ok(None) => 2,
            Err(error) => super::failure::encode(&error).ok_or(Error::ModuleError(error))?,
        };
        self.work = i64::try_from(left)
            .map_err(|_| Error::ModuleError("budget leaf: allowance overflow".into()))?;
        let trace_host = self.batch.subfile(file).map(|sub| sub.host);
        record(
            serde_json::json!({"terminal":term_id,"file":trace_host.unwrap_or(file),"parsedFile":trace_host.map(|_| file),"node":node,"input":work,"truth":self.truth,"remaining":self.work,"text":text,"valueCalls":crate::code_rule_dsl::matcher::value_probe::since(probes)}),
        );
        self.done = false;
        Ok(())
    }
    fn next(&mut self) -> Result<()> {
        self.done = true;
        Ok(())
    }
    fn eof(&self) -> bool {
        self.done
    }
    fn column(&self, ctx: &mut Context, col: c_int) -> Result<()> {
        match col {
            0 => ctx.set_result(&self.truth),
            1 => ctx.set_result(&self.work),
            _ => Err(Error::ModuleError(
                "budget leaf: hidden input cannot be projected".into(),
            )),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(0)
    }
}
