//! Atomic format analysis returns ordered candidate facts, never candidate matcher verdicts.
use super::super::{
    exec::Batch,
    plan::SlotKind,
    terminals::{Terminal, Verdict, Verdicts, with_scopes},
};
use super::capture_cell::{Cell, decode_list};
use crate::code_rule_dsl::matcher::{FormatArgs, Prepared, missing_fields, node_at};
use ast_grep_core::{AstGrep, meta_var::MetaVarEnv};
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

pub(super) struct FormatAtomic {
    pub test: FormatArgs,
    pub owner: usize,
}
impl Verdicts for FormatAtomic {
    fn format_test(&self) -> Option<&FormatArgs> {
        Some(&self.test)
    }
    fn format_owner(&self) -> usize {
        self.owner
    }
    fn rows<'t>(
        &self,
        _: ast_grep_core::Node<'t, crate::code_rule_dsl::tape::TapeDoc>,
        _: &MetaVarEnv<'t, crate::code_rule_dsl::tape::TapeDoc>,
    ) -> std::result::Result<Vec<Verdict>, String> {
        Err("format analysis requires its budget cursor".into())
    }
}
fn invalid(message: &str) -> Error {
    Error::ModuleError(format!("format facts: {message}"))
}
pub(super) fn register(db: &Connection, batch: &Arc<Batch>) -> Result<()> {
    const MODULE: Module<'static, Table> = Module::eponymous_only_module();
    db.create_module(c"relation_format_budget", &MODULE, Some(Arc::clone(batch)))
}
#[repr(C)]
struct Table {
    base: sqlite3_vtab,
    batch: Arc<Batch>,
}
// SAFETY: SQLite's base is first; each cursor retains the immutable batch.
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
        Ok((Cow::Borrowed(c"CREATE TABLE x(truth INTEGER,work INTEGER,unused_json TEXT,missing_json TEXT,unused_open INTEGER,missing_open INTEGER,parsed_file INTEGER,term INTEGER HIDDEN,file INTEGER HIDDEN,node INTEGER HIDDEN,allowance INTEGER HIDDEN,string HIDDEN,args HIDDEN)"),Self{base:sqlite3_vtab::default(),batch:batch.ok_or_else(||invalid("missing batch"))?.clone()}))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let mut inputs = [None; 6];
        for (index, c) in info.constraints().enumerate() {
            if c.is_usable()
                && (7..13).contains(&c.column())
                && matches!(
                    c.operator(),
                    IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
                        | IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_IS
                )
            {
                inputs[(c.column() - 7) as usize] = Some(index);
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
            unused: String::from("[]"),
            missing: String::from("[]"),
            unused_open: false,
            missing_open: false,
            parsed_file: None,
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
    unused: String,
    missing: String,
    unused_open: bool,
    missing_open: bool,
    parsed_file: Option<i64>,
    done: bool,
    marker: PhantomData<&'v Table>,
}
// SAFETY: SQLite's base is first; result cells belong to this cursor.
unsafe impl VTabCursor for Cursor<'_> {
    fn filter(&mut self, _: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        self.done = true;
        let Some(allowance) = args.get::<Option<i64>>(3)? else {
            return Ok(());
        };
        let term = args.get::<u32>(0)? as usize;
        let file = args.get::<u32>(1)? as usize;
        let node = args.get::<u32>(2)?;
        let work = usize::try_from(allowance).map_err(|_| invalid("negative work"))?;
        let Some(Terminal::Fact(term)) = self.batch.program.terminals.get(term) else {
            return Err(invalid("terminal"));
        };
        let test = term
            .verdicts
            .format_test()
            .ok_or_else(|| invalid("terminal contract"))?;
        let data = self.batch.files.get(file).ok_or_else(|| invalid("file"))?;
        if node >= data.facts.len() || term.reads.len() != 2 {
            return Err(invalid("node or input width"));
        }
        let mut env = MetaVarEnv::new();
        for (index, (name, kind)) in term.reads.iter().enumerate() {
            match (kind, args.get::<Cell>(4 + index)?) {
                (_, Cell::Null) => {}
                (SlotKind::Node, Cell::Integer(id)) => {
                    let id = u32::try_from(id).map_err(|_| invalid("input node"))?;
                    if id >= data.facts.len() {
                        return Err(invalid("input node outside file"));
                    }
                    env.insert(name, node_at(&data.root, id))
                        .ok_or_else(|| invalid("conflicting input"))?;
                }
                (SlotKind::List, Cell::List(bytes)) => {
                    let ids = decode_list(&bytes, file as u32, data.facts.len())?;
                    env.insert_multi(
                        name,
                        ids.into_iter().map(|id| node_at(&data.root, id)).collect(),
                    );
                }
                _ => return Err(invalid("typed input mismatch")),
            }
        }
        let (prepared, left) = with_scopes(file, data, || {
            crate::code_rule_dsl::matcher::with_sql_clause_owner(
                term.verdicts.format_owner(),
                || crate::code_rule_dsl::matcher::sql_format_budget(test, &env, work),
            )
        });
        self.work = i64::try_from(left).map_err(|_| invalid("work overflow"))?;
        self.unused = "[]".into();
        self.missing = "[]".into();
        self.unused_open = false;
        self.missing_open = false;
        self.parsed_file = None;
        self.truth = match prepared {
            Err(error) => super::failure::encode(&error).ok_or(Error::ModuleError(error))?,
            Ok(Prepared::NoMatch) => 0,
            Ok(Prepared::Unknown) => 2,
            Ok(Prepared::Sides {
                tree,
                unused,
                missing,
            }) => {
                self.parsed_file = Some(i64::from(self.batch.add_subfile(file, &tree, None)));
                self.unused = serde_json::to_string(
                    &unused
                        .candidates
                        .iter()
                        .map(|(node, known)| (node.node_id() as u32, *known))
                        .collect::<Vec<_>>(),
                )
                .map_err(|e| invalid(&e.to_string()))?;
                let root = AstGrep::doc(crate::code_rule_dsl::literal::SubDoc { tree });
                self.missing = serde_json::to_string(
                    &missing_fields(&root.root(), &missing)
                        .map(|(index, _, known)| (index, known))
                        .collect::<Vec<_>>(),
                )
                .map_err(|e| invalid(&e.to_string()))?;
                self.unused_open = unused.incomplete;
                self.missing_open = missing.incomplete;
                1
            }
        };
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
    fn column(&self, ctx: &mut Context, column: c_int) -> Result<()> {
        match column {
            0 => ctx.set_result(&self.truth),
            1 => ctx.set_result(&self.work),
            2 => ctx.set_result(&self.unused),
            3 => ctx.set_result(&self.missing),
            4 => ctx.set_result(&self.unused_open),
            5 => ctx.set_result(&self.missing_open),
            6 => ctx.set_result(&self.parsed_file),
            _ => Err(invalid("hidden output")),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(1)
    }
}
