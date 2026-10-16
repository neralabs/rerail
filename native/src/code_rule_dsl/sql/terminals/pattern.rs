//! The `pattern(term)` terminal. Without input it lists, once per batch, every node of the
//! pattern's root kinds the pattern matches in a fresh environment, with what it bound. With an
//! `ek IN (…)` input (the env-input variant) it matches each input row's node with the row's
//! bindings of the pattern's captures already in the environment: an ellipsis's greedy alignment
//! and every unification then happen natively, exactly as the matcher does them.

use super::{PATTERN_COLUMNS, PatternTerm, Terminal};
use crate::code_rule_dsl::matcher::node_at;
use crate::code_rule_dsl::sql::exec::{Batch, BatchFile, BatchSlot};
use crate::code_rule_dsl::sql::nodes::{NodeFacts, hasher, integer};
use crate::code_rule_dsl::sql::plan::SlotKind;
use crate::code_rule_dsl::tape::TapeDoc;
use ast_grep_core::matcher::Matcher;
use ast_grep_core::meta_var::MetaVarEnv;
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::types::{Value, ValueRef};
use rusqlite::vtab::{
    Context, Filters, IndexConstraintOp, IndexInfo, VTab, VTabConfig, VTabConnection, VTabCursor,
    sqlite3_vtab, sqlite3_vtab_cursor,
};
use rusqlite::{Error, Result};
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::ffi::{CStr, c_int};
use std::hash::Hasher;
use std::marker::PhantomData;
use std::sync::{Arc, LazyLock};

/// One match: its node, the input row it answers (env-input) and its capture columns.
pub(in crate::code_rule_dsl::sql) struct PatternRow {
    file: u32,
    node: u32,
    ek: Option<String>,
    values: Vec<Option<i64>>,
}

/// The four columns of a list capture: its first and last node (`-1` when empty), the hash of
/// its nodes' texts and how many of them are named.
pub(in crate::code_rule_dsl::sql) fn list_columns(
    facts: &NodeFacts,
    nodes: &[u32],
) -> [Option<i64>; 4] {
    let mut hash = hasher();
    hash.write_usize(nodes.len());
    for &node in nodes {
        hash.write_i64(facts.thash(node));
    }
    let named = nodes.iter().filter(|&&node| facts.named(node)).count();
    let (lo, hi) = match (nodes.first(), nodes.last()) {
        (Some(&lo), Some(&hi)) => (i64::from(lo), i64::from(hi)),
        _ => (-1, -1),
    };
    [
        Some(lo),
        Some(hi),
        Some(hash.finish() as i64),
        Some(named as i64),
    ]
}

/// The nodes of a list capture from its first and last: the run of siblings between them.
pub(in crate::code_rule_dsl::sql) fn list_nodes(facts: &NodeFacts, lo: i64, hi: i64) -> Vec<u32> {
    let (Ok(lo), Ok(hi)) = (u32::try_from(lo), u32::try_from(hi)) else {
        return Vec::new();
    };
    let mut nodes = vec![lo];
    let mut at = lo;
    while at != hi {
        match facts.next_sibling(at) {
            Some(next) => {
                nodes.push(next);
                at = next;
            }
            None => break,
        }
    }
    nodes
}

fn values<'t>(
    term: &PatternTerm,
    facts: &NodeFacts,
    env: &MetaVarEnv<'t, TapeDoc>,
) -> Vec<Option<i64>> {
    let mut values = Vec::with_capacity(term.width());
    for (name, kind) in &term.vars {
        match kind {
            SlotKind::List => match env.get_labels(name) {
                Some(nodes) => {
                    let ids: Vec<u32> = nodes.iter().map(|node| node.node_id() as u32).collect();
                    values.extend(list_columns(facts, &ids));
                }
                None => values.extend([None; 4]),
            },
            _ => values.push(env.get_match(name).map(|node| node.node_id() as i64)),
        }
    }
    values
}

/// The fresh-env matches of `term` over every file of its dialect in the batch.
pub(in crate::code_rule_dsl::sql) fn fresh_rows(
    batch: &Batch,
    term: &PatternTerm,
) -> Vec<PatternRow> {
    #[cfg(test)]
    super::calls::note(&super::calls::FRESH_MATCHES);
    let mut rows = Vec::new();
    for (index, file) in batch.files.iter().enumerate() {
        if file.dialect != term.dialect {
            continue;
        }
        let mut test = |id: u32| {
            let node = node_at(&file.root, id);
            let mut env = Cow::Owned(MetaVarEnv::new());
            if term.pattern.match_node_with_env(node, &mut env).is_some() {
                rows.push(PatternRow {
                    file: index as u32,
                    node: id,
                    ek: None,
                    values: values(term, &file.facts, &env),
                });
            }
        };
        match &term.kinds {
            Some(kinds) => {
                let mut ids: Vec<u32> = kinds
                    .iter()
                    .flat_map(|&kind| file.facts.of_kind(kind).iter().copied())
                    .collect();
                ids.sort_unstable();
                ids.into_iter().for_each(&mut test);
            }
            None => (0..file.facts.len()).for_each(&mut test),
        }
    }
    rows
}

/// Decodes an env-input key (`file,node,<value>…`, one value per read capture: a node id, or a
/// list's `lo:hi`, empty where unbound), matches it, and returns its row where it matches.
fn env_row(batch: &Batch, term: &PatternTerm, ek: &str) -> Option<PatternRow> {
    let mut parts = ek.split(',');
    let file: u32 = parts.next()?.parse().ok()?;
    let node: u32 = parts.next()?.parse().ok()?;
    let entry: &BatchFile = batch.files.get(file as usize)?;
    if node >= entry.facts.len() {
        return None;
    }
    let mut env = MetaVarEnv::new();
    for &read in &term.reads {
        let part = parts.next()?;
        if part.is_empty() {
            continue;
        }
        let (name, kind) = &term.vars[read];
        match kind {
            SlotKind::List => {
                let (lo, hi) = part.split_once(':')?;
                let nodes = list_nodes(&entry.facts, lo.parse().ok()?, hi.parse().ok()?)
                    .into_iter()
                    .map(|id| node_at(&entry.root, id))
                    .collect();
                env.insert_multi(name, nodes)?;
            }
            _ => {
                env.insert(name, node_at(&entry.root, part.parse().ok()?))?;
            }
        }
    }
    let mut env = Cow::Owned(env);
    term.pattern
        .match_node_with_env(node_at(&entry.root, node), &mut env)?;
    Some(PatternRow {
        file,
        node,
        ek: Some(ek.to_owned()),
        values: values(term, &entry.facts, &env),
    })
}

const COL_TERM: c_int = 0;
const COL_EK: c_int = 1;
const COL_FILE: c_int = 2;
const COL_NODE: c_int = 3;

static SCHEMA: LazyLock<std::ffi::CString> = LazyLock::new(|| {
    let values = (0..PATTERN_COLUMNS)
        .map(|index| format!(", v{index}"))
        .collect::<String>();
    std::ffi::CString::new(format!(
        "CREATE TABLE x(term HIDDEN, ek HIDDEN, file, node{values})"
    ))
    .expect("no NUL")
});

#[repr(C)]
pub(in crate::code_rule_dsl::sql) struct PatternTable {
    base: sqlite3_vtab,
    slot: Arc<BatchSlot>,
}

unsafe impl<'vtab> VTab<'vtab> for PatternTable {
    type Aux = Arc<BatchSlot>;
    type Cursor = PatternCursor<'vtab>;

    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Arc<BatchSlot>>,
        _module: &[u8],
        _database: &[u8],
        _table: &[u8],
        _args: &[&[u8]],
    ) -> Result<(std::borrow::Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        let slot =
            Arc::clone(aux.ok_or_else(|| Error::ModuleError("pattern: no batch slot".into()))?);
        Ok((
            std::borrow::Cow::Borrowed(SCHEMA.as_c_str()),
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
        let Some(term) = term else {
            info.set_estimated_cost(f64::MAX);
            info.set_estimated_rows(i64::MAX);
            info.set_idx_num(0);
            return Ok(true);
        };
        let mut usage = info.constraint_usage(term);
        usage.set_argv_index(1);
        usage.set_omit(true);
        let mut idx_num = 1;
        if let Some(ek) = ek {
            let list = info.is_in_constraint(ek)? && info.set_in_constraint(ek, true)?;
            let mut usage = info.constraint_usage(ek);
            usage.set_argv_index(2);
            usage.set_omit(true);
            idx_num |= if list { 4 } else { 2 };
        }
        info.set_idx_num(idx_num);
        info.set_estimated_cost(if ek.is_some() { 100.0 } else { 10_000.0 });
        info.set_estimated_rows(if ek.is_some() { 100 } else { 10_000 });
        Ok(true)
    }

    fn open(&'vtab mut self) -> Result<PatternCursor<'vtab>> {
        Ok(PatternCursor {
            base: sqlite3_vtab_cursor::default(),
            slot: Arc::clone(&self.slot),
            term: 0,
            rows: Rows::Owned(Vec::new()),
            pos: 0,
            phantom: PhantomData,
        })
    }
}

enum Rows {
    Shared(Arc<Vec<PatternRow>>),
    Owned(Vec<PatternRow>),
}

impl Rows {
    fn get(&self, pos: usize) -> Option<&PatternRow> {
        match self {
            Self::Shared(rows) => rows.get(pos),
            Self::Owned(rows) => rows.get(pos),
        }
    }
}

#[repr(C)]
pub(in crate::code_rule_dsl::sql) struct PatternCursor<'vtab> {
    base: sqlite3_vtab_cursor,
    slot: Arc<BatchSlot>,
    term: i64,
    rows: Rows,
    pos: usize,
    phantom: PhantomData<&'vtab PatternTable>,
}

unsafe impl VTabCursor for PatternCursor<'_> {
    fn filter(&mut self, idx_num: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        #[cfg(test)]
        super::calls::note(&super::calls::PATTERN_FILTERS);
        if idx_num & 1 == 0 {
            return Err(Error::ModuleError("pattern requires a term".into()));
        }
        let batch = self
            .slot
            .current()
            .ok_or_else(|| Error::ModuleError("pattern: no batch installed".into()))?;
        let term_id = integer(ValueRef::from(&args.get::<Value>(0)?))
            .ok_or_else(|| Error::ModuleError("pattern: term must be an integer".into()))?;
        let Some(Terminal::Pattern(term)) = usize::try_from(term_id)
            .ok()
            .and_then(|index| batch.program.terminals.get(index))
        else {
            return Err(Error::ModuleError(format!(
                "pattern: no pattern term {term_id}"
            )));
        };
        self.term = term_id;
        self.pos = 0;
        self.rows = if idx_num & 6 == 0 {
            Rows::Shared(batch.fresh(term_id as usize, term))
        } else {
            let mut keys = BTreeSet::new();
            if idx_num & 4 != 0 {
                let mut items = args.in_values(1)?;
                while let Some(item) = items.next()? {
                    if let ValueRef::Text(text) = item {
                        keys.insert(String::from_utf8_lossy(text).into_owned());
                    }
                }
            } else if let Value::Text(text) = args.get::<Value>(1)? {
                keys.insert(text);
            }
            Rows::Owned(
                keys.iter()
                    .filter_map(|ek| env_row(&batch, term, ek))
                    .collect(),
            )
        };
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        self.pos += 1;
        Ok(())
    }

    fn eof(&self) -> bool {
        self.rows.get(self.pos).is_none()
    }

    fn column(&self, context: &mut Context, column: c_int) -> Result<()> {
        let row = self.rows.get(self.pos).expect("a row");
        match column {
            COL_TERM => context.set_result(&self.term),
            COL_EK => context.set_result(&row.ek),
            COL_FILE => context.set_result(&i64::from(row.file)),
            COL_NODE => context.set_result(&i64::from(row.node)),
            other => {
                let index = (other - 4) as usize;
                context.set_result(&row.values.get(index).copied().flatten())
            }
        }
    }

    fn rowid(&self) -> Result<i64> {
        Ok(self.pos as i64)
    }
}
