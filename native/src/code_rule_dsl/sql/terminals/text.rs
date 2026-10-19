//! The `text_match(term)` terminal for `regex`: an input-set terminal. It receives the distinct
//! `(file, node)` keys of a frame (`file << 32 | node`) in one `xFilter` call and returns those
//! whose node text the regex matches (`Regex::is_match`, unanchored, as the matcher tests it).

use super::Terminal;
use crate::code_rule_dsl::sql::exec::BatchSlot;
use crate::code_rule_dsl::sql::nodes::integer;
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::types::{Value, ValueRef};
use rusqlite::vtab::{
    Context, Filters, IndexConstraintOp, IndexInfo, VTab, VTabConfig, VTabConnection, VTabCursor,
    sqlite3_vtab, sqlite3_vtab_cursor,
};
use rusqlite::{Error, Result};
use std::borrow::Cow;
use std::ffi::{CStr, c_int};
use std::marker::PhantomData;
use std::sync::Arc;

const COL_TERM: c_int = 0;
const COL_EK: c_int = 1;

#[repr(C)]
pub(in crate::code_rule_dsl::sql) struct TextTable {
    base: sqlite3_vtab,
    slot: Arc<BatchSlot>,
}

unsafe impl<'vtab> VTab<'vtab> for TextTable {
    type Aux = Arc<BatchSlot>;
    type Cursor = TextCursor<'vtab>;

    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Arc<BatchSlot>>,
        _module: &[u8],
        _database: &[u8],
        _table: &[u8],
        _args: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        let slot =
            Arc::clone(aux.ok_or_else(|| Error::ModuleError("text_match: no batch slot".into()))?);
        Ok((
            Cow::Borrowed(c"CREATE TABLE x(term HIDDEN, ek)"),
            Self {
                base: sqlite3_vtab::default(),
                slot,
            },
        ))
    }

    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let mut term = None;
        let mut ek = None;
        for (index, constraint) in info.constraints().enumerate() {
            if !constraint.is_usable()
                || constraint.operator() != IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
            {
                continue;
            }
            match constraint.column() {
                COL_TERM => {
                    term.get_or_insert(index);
                }
                COL_EK => {
                    ek.get_or_insert(index);
                }
                _ => {}
            }
        }
        let (Some(term), Some(ek)) = (term, ek) else {
            info.set_estimated_cost(f64::MAX);
            info.set_estimated_rows(i64::MAX);
            info.set_idx_num(0);
            return Ok(true);
        };
        let list = info.is_in_constraint(ek)? && info.set_in_constraint(ek, true)?;
        for (argv, index) in [term, ek].into_iter().enumerate() {
            let mut usage = info.constraint_usage(index);
            usage.set_argv_index(argv as c_int + 1);
            usage.set_omit(true);
        }
        info.set_idx_num(if list { 3 } else { 1 });
        info.set_estimated_cost(100.0);
        info.set_estimated_rows(100);
        Ok(true)
    }

    fn open(&'vtab mut self) -> Result<TextCursor<'vtab>> {
        Ok(TextCursor {
            base: sqlite3_vtab_cursor::default(),
            slot: Arc::clone(&self.slot),
            term: 0,
            keys: Vec::new(),
            pos: 0,
            phantom: PhantomData,
        })
    }
}

#[repr(C)]
pub(in crate::code_rule_dsl::sql) struct TextCursor<'vtab> {
    base: sqlite3_vtab_cursor,
    slot: Arc<BatchSlot>,
    term: i64,
    keys: Vec<i64>,
    pos: usize,
    phantom: PhantomData<&'vtab TextTable>,
}

unsafe impl VTabCursor for TextCursor<'_> {
    fn filter(&mut self, idx_num: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        #[cfg(test)]
        super::calls::note(&super::calls::TEXT_FILTERS);
        if idx_num & 1 == 0 {
            return Err(Error::ModuleError(
                "text_match requires a term and an input set".into(),
            ));
        }
        let batch = self
            .slot
            .current()
            .ok_or_else(|| Error::ModuleError("text_match: no batch installed".into()))?;
        let term_id = integer(ValueRef::from(&args.get::<Value>(0)?))
            .ok_or_else(|| Error::ModuleError("text_match: term must be an integer".into()))?;
        let Some(Terminal::Regex(regex)) = usize::try_from(term_id)
            .ok()
            .and_then(|index| batch.program.terminals.get(index))
        else {
            return Err(Error::ModuleError(format!(
                "text_match: no regex term {term_id}"
            )));
        };
        let mut keys = Vec::new();
        if idx_num & 2 != 0 {
            let mut items = args.in_values(1)?;
            while let Some(item) = items.next()? {
                keys.extend(integer(item));
            }
        } else {
            keys.extend(integer(ValueRef::from(&args.get::<Value>(1)?)));
        }
        keys.sort_unstable();
        keys.dedup();
        keys.retain(|&key| {
            let (file, node) = ((key >> 32) as usize, (key & 0xffff_ffff) as u32);
            match batch.files.get(file) {
                Some(file) => {
                    node < file.facts.len()
                        && regex.is_match(file.root.root().get_doc().transducer_text_str(node))
                }
                // A parsed tree's node: its text, as the matcher reads it.
                None => batch.subfile(file).is_some_and(|sub| {
                    node < sub.facts.len()
                        && regex.is_match(
                            &sub.root.adopt(sub.root.root().get_doc().node(node)).text(),
                        )
                }),
            }
        });
        self.term = term_id;
        self.keys = keys;
        self.pos = 0;
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        self.pos += 1;
        Ok(())
    }

    fn eof(&self) -> bool {
        self.pos >= self.keys.len()
    }

    fn column(&self, context: &mut Context, column: c_int) -> Result<()> {
        match column {
            COL_TERM => context.set_result(&self.term),
            _ => context.set_result(&self.keys[self.pos]),
        }
    }

    fn rowid(&self) -> Result<i64> {
        Ok(self.pos as i64)
    }
}
