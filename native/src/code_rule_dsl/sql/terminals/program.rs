//! The `program_match(term)` terminal for a pushed-down sub-rule (`pushdown`): a transducer
//! program run over the batch's documents by the node-set executor (`Program::holds`), once per
//! statement over its whole input set.
//!
//! # Interface: an input node set to the nodes that match
//!
//! This table is the one seam between the SQL engine and a transducer executor; another
//! executor (a compiled or GPU one) replaces `holding` and keeps everything else:
//!
//! - **Columns:** `program_match(term HIDDEN, ek, file HIDDEN)`. `term` is the program's index
//!   among the program's terminals (`Terminal::Program`); `ek = file << 32 | node` is a node key,
//!   `file` a batch document (a file, or a parsed tree numbered after them, `exec::SubFile`) and
//!   `node` its preorder id in that document's tree (`transducer::Tree`, which `NodeFacts`
//!   implements over the document's tape).
//! - **Keys in (`term = ? AND ek IN (…)`):** one `xFilter` call receives every distinct key of a
//!   frame; the table returns the keys at which the program emits a path started there. That is
//!   the per-statement form an operator's frame reads (`Emitter::input_set`).
//! - **Files in (`term = ? AND file IN (…)`):** every node of those documents at which it emits,
//!   as keys: a stage's statement (`pushdown::stage`).
//! - **Semantics:** a node is returned exactly when the path executor (`Program::execute_roots`,
//!   the language's reference, `docs/transducer-design.md`) started at it emits; captures and
//!   labels are not read. Programs are capture-free and forward-only (`Program::forward`), with
//!   no text but `TEXT` predicates on node text. Output order is free: SQL reads the rows as a set.
//! - **Cost model:** the table is called once per statement, never per row; an executor may take
//!   the whole batch's input set at once (all files' keys arrive in one call, grouped by file).

use super::Terminal;
use crate::code_rule_dsl::sql::exec::{Batch, BatchSlot};
use crate::code_rule_dsl::sql::nodes::integer;
use crate::code_rule_dsl::transducer::{NodeSet, Program};
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
const COL_FILE: c_int = 2;

/// `idx_num` bits: the keys are given, as a list; the files are given, as a list.
const KEYS: c_int = 1;
const KEY_LIST: c_int = 2;
const FILES: c_int = 4;
const FILE_LIST: c_int = 8;

/// A pushed-down sub-rule's program.
pub(in crate::code_rule_dsl::sql) struct ProgramTerm {
    pub(in crate::code_rule_dsl::sql) program: Program,
}

#[repr(C)]
pub(in crate::code_rule_dsl::sql) struct ProgramTable {
    base: sqlite3_vtab,
    slot: Arc<BatchSlot>,
}

unsafe impl<'vtab> VTab<'vtab> for ProgramTable {
    type Aux = Arc<BatchSlot>;
    type Cursor = ProgramCursor<'vtab>;

    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Arc<BatchSlot>>,
        _module: &[u8],
        _database: &[u8],
        _table: &[u8],
        _args: &[&[u8]],
    ) -> Result<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        let slot = Arc::clone(
            aux.ok_or_else(|| Error::ModuleError("program_match: no batch slot".into()))?,
        );
        Ok((
            Cow::Borrowed(c"CREATE TABLE x(term HIDDEN, ek, file HIDDEN)"),
            Self {
                base: sqlite3_vtab::default(),
                slot,
            },
        ))
    }

    fn best_index(&self, info: &mut IndexInfo) -> Result<bool> {
        let (mut term, mut ek, mut file) = (None, None, None);
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
                COL_FILE => {
                    file.get_or_insert(index);
                }
                _ => {}
            }
        }
        let (term, (input, mode, list)) = match (term, ek, file) {
            (Some(term), Some(ek), _) => (term, (ek, KEYS, KEY_LIST)),
            (Some(term), None, Some(file)) => (term, (file, FILES, FILE_LIST)),
            _ => {
                info.set_estimated_cost(f64::MAX);
                info.set_estimated_rows(i64::MAX);
                info.set_idx_num(0);
                return Ok(true);
            }
        };
        let listed = info.is_in_constraint(input)? && info.set_in_constraint(input, true)?;
        for (argv, index) in [term, input].into_iter().enumerate() {
            let mut usage = info.constraint_usage(index);
            usage.set_argv_index(argv as c_int + 1);
            usage.set_omit(true);
        }
        info.set_idx_num(mode | if listed { list } else { 0 });
        info.set_estimated_cost(100.0);
        info.set_estimated_rows(100);
        Ok(true)
    }

    fn open(&'vtab mut self) -> Result<ProgramCursor<'vtab>> {
        Ok(ProgramCursor {
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
pub(in crate::code_rule_dsl::sql) struct ProgramCursor<'vtab> {
    base: sqlite3_vtab_cursor,
    slot: Arc<BatchSlot>,
    term: i64,
    keys: Vec<i64>,
    pos: usize,
    phantom: PhantomData<&'vtab ProgramTable>,
}

/// The nodes of document `file` (a batch file, or a parsed tree of the batch's own) among
/// `starts` at which `program` holds, as keys.
fn holding(
    batch: &Batch,
    program: &Program,
    file: usize,
    starts: impl FnOnce(u32) -> NodeSet,
) -> Result<Vec<i64>> {
    let sub;
    let facts = match batch.files.get(file) {
        Some(source) => &source.facts,
        None => {
            sub = batch.subfile(file).ok_or_else(|| {
                Error::ModuleError(format!(
                    "program_match: document {file} is not a document of the batch"
                ))
            })?;
            &sub.facts
        }
    };
    let starts = starts(facts.len());
    let held = program.holds(facts, &starts).map_err(|error| {
        Error::ModuleError(format!("program_match: the program fails: {error:?}"))
    })?;
    Ok(held
        .iter()
        .map(|node| (file as i64) << 32 | i64::from(node))
        .collect())
}

unsafe impl VTabCursor for ProgramCursor<'_> {
    fn filter(&mut self, idx_num: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        if idx_num & (KEYS | FILES) == 0 {
            return Err(Error::ModuleError(
                "program_match requires a term and an input set or files".into(),
            ));
        }
        let batch = self
            .slot
            .current()
            .ok_or_else(|| Error::ModuleError("program_match: no batch installed".into()))?;
        let term_id = integer(ValueRef::from(&args.get::<Value>(0)?))
            .ok_or_else(|| Error::ModuleError("program_match: term must be an integer".into()))?;
        let Some(Terminal::Program(term)) = usize::try_from(term_id)
            .ok()
            .and_then(|index| batch.program.terminals.get(index))
        else {
            return Err(Error::ModuleError(format!(
                "program_match: no program term {term_id}"
            )));
        };
        let mut inputs = Vec::new();
        if idx_num & (KEY_LIST | FILE_LIST) != 0 {
            let mut items = args.in_values(1)?;
            while let Some(item) = items.next()? {
                inputs.extend(integer(item));
            }
        } else {
            inputs.extend(integer(ValueRef::from(&args.get::<Value>(1)?)));
        }
        inputs.sort_unstable();
        inputs.dedup();
        let mut keys = Vec::new();
        if idx_num & KEYS != 0 {
            // The keys of one file at a time, in key order.
            let mut at = 0;
            while at < inputs.len() {
                let file = inputs[at] >> 32;
                let end = at + inputs[at..].partition_point(|key| key >> 32 == file);
                let nodes = &inputs[at..end];
                keys.extend(holding(&batch, &term.program, file as usize, |len| {
                    let mut starts = NodeSet::new(len);
                    for &key in nodes {
                        let node = (key & 0xffff_ffff) as u32;
                        if node < len {
                            starts.insert(node);
                        }
                    }
                    starts
                })?);
                at = end;
            }
        } else {
            for file in inputs {
                let Ok(file) = usize::try_from(file) else {
                    continue;
                };
                keys.extend(holding(&batch, &term.program, file, NodeSet::full)?);
            }
        }
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
            COL_FILE => context.set_result(&(self.keys[self.pos] >> 32)),
            _ => context.set_result(&self.keys[self.pos]),
        }
    }

    fn rowid(&self) -> Result<i64> {
        Ok(self.pos as i64)
    }
}
