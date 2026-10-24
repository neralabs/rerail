//! Decode and parse only; child matcher composition belongs to SQL.
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

pub(super) struct ParseAtomic {
    pub test: crate::code_rule_dsl::matcher::ParseTest,
    pub owner: usize,
}
impl crate::code_rule_dsl::sql::terminals::Verdicts for ParseAtomic {
    fn parsed_test(&self) -> Option<&crate::code_rule_dsl::matcher::ParseTest> {
        Some(&self.test)
    }
    fn parsed_owner(&self) -> usize {
        self.owner
    }
    fn rows<'t>(
        &self,
        _: ast_grep_core::Node<'t, crate::code_rule_dsl::tape::TapeDoc>,
        _: &ast_grep_core::meta_var::MetaVarEnv<'t, crate::code_rule_dsl::tape::TapeDoc>,
    ) -> std::result::Result<Vec<crate::code_rule_dsl::sql::terminals::Verdict>, String> {
        Err("parse requires the atomic budget cursor".into())
    }
}
pub(super) fn register(db: &Connection, batch: &Arc<Batch>) -> Result<()> {
    const MODULE: Module<'static, Table> = Module::eponymous_only_module();
    db.create_module(c"relation_parse_budget", &MODULE, Some(Arc::clone(batch)))
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
        Ok((Cow::Borrowed(c"CREATE TABLE x(truth INTEGER,work INTEGER,parsed_file INTEGER,parsed_node INTEGER,term INTEGER HIDDEN,file INTEGER HIDDEN,node INTEGER HIDDEN,allowance INTEGER HIDDEN)"), Self { base: sqlite3_vtab::default(), batch: batch.unwrap().clone() }))
    }
    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let mut inputs = [None; 4];
        for (index, c) in info.constraints().enumerate() {
            if c.is_usable()
                && (4..8).contains(&c.column())
                && c.operator() == IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
            {
                inputs[(c.column() - 4) as usize] = Some(index);
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
            parsed_file: None,
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
    parsed_file: Option<i64>,
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
        let file = args.get::<u32>(1)? as usize;
        let node = args.get::<u32>(2)?;
        let work = usize::try_from(allowance)
            .map_err(|_| Error::ModuleError("negative work allowance".into()))?;
        let Some(Terminal::Fact(term)) = self.batch.program.terminals.get(term) else {
            return Err(Error::ModuleError("budget leaf: invalid terminal".into()));
        };
        let test = term
            .verdicts
            .parsed_test()
            .ok_or_else(|| Error::ModuleError("parse leaf: invalid terminal contract".into()))?;
        let owner = term.verdicts.parsed_owner();
        let (host_index, (parsed, left)) = if let Some(file_data) = self.batch.files.get(file) {
            if node >= file_data.facts.len() {
                return Err(Error::ModuleError("parse leaf: invalid node".into()));
            }
            (
                file,
                with_scopes(file, file_data, || {
                    crate::code_rule_dsl::matcher::with_sql_clause_owner(owner, || {
                        crate::code_rule_dsl::matcher::sql_parse_budget(
                            test,
                            &node_at(&file_data.root, node),
                            work,
                        )
                    })
                }),
            )
        } else if let Some(sub) = self.batch.subfile(file) {
            if node >= sub.facts.len() {
                return Err(Error::ModuleError("parse leaf: invalid parsed node".into()));
            }
            let host = self
                .batch
                .files
                .get(sub.host)
                .ok_or_else(|| Error::ModuleError("parse leaf: invalid parsed host".into()))?;
            let _tables = sub
                .table
                .clone()
                .map(crate::code_rule_dsl::matcher::SubTables::enter);
            let at = sub.root.adopt(sub.root.root().get_doc().node(node));
            (
                sub.host,
                with_scopes(sub.host, host, || {
                    crate::code_rule_dsl::matcher::with_sql_clause_owner(owner, || {
                        crate::code_rule_dsl::matcher::sql_parse_budget(test, &at, work)
                    })
                }),
            )
        } else {
            return Err(Error::ModuleError("parse leaf: invalid host file".into()));
        };
        self.parsed_file = None;
        self.truth = match parsed {
            Ok(Some((crate::code_rule_dsl::literal::Parsed::Tree(tree), table))) => {
                self.parsed_file =
                    Some(i64::from(self.batch.add_subfile(host_index, &tree, table)));
                1
            }
            Ok(Some((crate::code_rule_dsl::literal::Parsed::Unknown, _))) => 2,
            Ok(Some((crate::code_rule_dsl::literal::Parsed::NotString, _))) | Ok(None) => 0,
            Err(error) => super::failure::encode(&error).ok_or(Error::ModuleError(error))?,
        };
        self.work = i64::try_from(left)
            .map_err(|_| Error::ModuleError("parse allowance overflow".into()))?;
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
            2 => ctx.set_result(&self.parsed_file),
            3 => ctx.set_result(&self.parsed_file.map(|_| 0_i64)),
            _ => Err(Error::ModuleError(
                "budget leaf: hidden input cannot be projected".into(),
            )),
        }
    }
    fn rowid(&self) -> Result<i64> {
        Ok(0)
    }
}
