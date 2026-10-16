//! The `fact(term)` terminal: an input-set terminal returning three-valued verdicts. It receives
//! a frame's distinct keys in one `xFilter` call (`ek IN (…)`), each `file,node` and the bindings
//! of the captures the term reads, decodes each into the node and an environment holding those
//! bindings, and evaluates it natively (`Verdicts`) with the file's scopes installed: built on the
//! file's first key and parked between statements, so a batch builds each file's scope tables,
//! values and line tables once (`FileScopeSlot::park`). It returns the keys whose verdict is true
//! or unknown, with `unk` and the values the term binds; `Op::Fact` reads them into a frame.

use super::Terminal;
use crate::code_rule_dsl::matcher::{
    AstMatcher, FileScopeSlot, ParkedScopes, leaf_verdict, node_at,
};
use crate::code_rule_dsl::soundness::SyntaxErrors;
use crate::code_rule_dsl::sql::exec::{BatchFile, BatchSlot};
use crate::code_rule_dsl::sql::nodes::integer;
use crate::code_rule_dsl::sql::plan::SlotKind;
use crate::code_rule_dsl::tape::TapeDoc;
use ast_grep_core::Doc;
use ast_grep_core::Node;
use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
use ast_grep_core::source::Content;
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::types::{Value, ValueRef};
use rusqlite::vtab::{
    Context, Filters, IndexConstraintOp, IndexInfo, VTab, VTabConfig, VTabConnection, VTabCursor,
    sqlite3_vtab, sqlite3_vtab_cursor,
};
use rusqlite::{Error, Result};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{CStr, c_int};
use std::marker::PhantomData;
use std::sync::{Arc, LazyLock};

/// The most binding columns a fact terminal returns (`v0`…).
pub(in crate::code_rule_dsl::sql) const FACT_COLUMNS: usize = 8;

/// A verdict that is not false: unknown or true, with the values of the term's binding columns.
pub(in crate::code_rule_dsl::sql) struct Verdict {
    pub(in crate::code_rule_dsl::sql) unknown: bool,
    pub(in crate::code_rule_dsl::sql) values: Vec<Option<i64>>,
}

/// What an input-set terminal evaluates per key: the verdict at `node` with `env`, which holds
/// the bindings of the term's reads. `Ok(None)` is false; `Err` a limit the evaluation hit.
pub(in crate::code_rule_dsl::sql) trait Verdicts:
    Send + Sync
{
    fn verdict<'t>(
        &self,
        node: Node<'t, TapeDoc>,
        env: &MetaVarEnv<'t, TapeDoc>,
    ) -> std::result::Result<Option<Verdict>, String>;
}

/// A leaf the native matcher evaluates (`value`, `compare`, `layout`, `testUnknown`), read
/// through `matcher::leaf_verdict`: it binds nothing.
pub(in crate::code_rule_dsl::sql) struct Leaf(pub(in crate::code_rule_dsl::sql) AstMatcher);

impl Verdicts for Leaf {
    fn verdict<'t>(
        &self,
        node: Node<'t, TapeDoc>,
        env: &MetaVarEnv<'t, TapeDoc>,
    ) -> std::result::Result<Option<Verdict>, String> {
        match leaf_verdict(&self.0, node, env) {
            Ok(Some(false)) => Ok(None),
            Ok(verdict) => Ok(Some(Verdict {
                unknown: verdict.is_none(),
                values: Vec::new(),
            })),
            Err(limit) => Err(format!("{limit:?}")),
        }
    }
}

/// A fact term: what it evaluates and the captures its keys carry, in key order. Its binding
/// columns (`v0`…, at most `FACT_COLUMNS`) are what its verdicts return.
pub(in crate::code_rule_dsl::sql) struct FactTerm {
    pub(in crate::code_rule_dsl::sql) verdicts: Box<dyn Verdicts>,
    pub(in crate::code_rule_dsl::sql) reads: Vec<(String, SlotKind)>,
}

thread_local! {
    /// The scopes of each file of the batch being run, parked between evaluations.
    static SCOPES: RefCell<Vec<Option<ParkedScopes>>> = const { RefCell::new(Vec::new()) };
}

/// The batch's parked file scopes: empty while it lives, dropped with it.
pub(in crate::code_rule_dsl::sql) struct BatchScopes;

impl BatchScopes {
    pub(in crate::code_rule_dsl::sql) fn begin() -> Self {
        SCOPES.with_borrow_mut(Vec::clear);
        Self
    }
}

impl Drop for BatchScopes {
    fn drop(&mut self) {
        SCOPES.with_borrow_mut(Vec::clear);
    }
}

/// Runs `body` with file `index`'s scopes installed: those parked by an earlier statement of the
/// batch, else fresh ones as the native evaluator installs them per file (its module, path and
/// whether its syntax errors hide bindings).
fn with_scopes<T>(index: usize, file: &BatchFile, body: impl FnOnce() -> T) -> T {
    let parked = SCOPES.with_borrow_mut(|scopes| scopes.get_mut(index).and_then(Option::take));
    let slot = match parked {
        Some(parked) => FileScopeSlot::resume(parked),
        None => {
            let errors = SyntaxErrors::of_doc(file.root.root().get_doc());
            let slot = FileScopeSlot::install(
                Vec::new(),
                file.context.module.clone(),
                errors.hide_bindings(),
            );
            match &file.context.path {
                Some(path) => slot.path(path),
                None => slot,
            }
        }
    };
    let out = body();
    if let Some(parked) = slot.park() {
        SCOPES.with_borrow_mut(|scopes| {
            if scopes.len() <= index {
                scopes.resize_with(index + 1, || None);
            }
            scopes[index] = Some(parked);
        });
    }
    out
}

/// The environment a key's capture values stand for, over `file`'s tree; `None` for a value
/// that names no node of it.
fn environment<'t>(
    file: &'t BatchFile,
    reads: &[(String, SlotKind)],
    values: &[&str],
) -> Option<MetaVarEnv<'t, TapeDoc>> {
    let mut env = MetaVarEnv::new();
    for ((name, kind), value) in reads.iter().zip(values) {
        if value.is_empty() {
            continue;
        }
        match kind {
            SlotKind::Node => {
                let id: u32 = value.parse().ok()?;
                if id >= file.facts.len() {
                    return None;
                }
                env.insert(name, node_at(&file.root, id))?;
            }
            SlotKind::List => {
                let (lo, hi) = value.split_once(':')?;
                let nodes =
                    super::pattern::list_nodes(&file.facts, lo.parse().ok()?, hi.parse().ok()?)
                        .into_iter()
                        .map(|id| node_at(&file.root, id))
                        .collect();
                env.insert_multi(name, nodes)?;
            }
            SlotKind::Count => {
                let bytes = <<TapeDoc as Doc>::Source as Content>::decode_str(value).into_owned();
                env.insert_transformation(&MetaVariable::Multiple, name, bytes);
            }
        }
    }
    Some(env)
}

/// One answered key.
struct FactRow {
    ek: String,
    unknown: bool,
    values: Vec<Option<i64>>,
}

/// Evaluates `keys` of `term`, file by file.
fn evaluate(
    batch: &crate::code_rule_dsl::sql::exec::Batch,
    term: &FactTerm,
    keys: BTreeSet<String>,
) -> Result<Vec<FactRow>> {
    let mut by_file: BTreeMap<usize, Vec<(String, u32)>> = BTreeMap::new();
    for ek in keys {
        let mut parts = ek.splitn(3, ',');
        let (Some(Ok(file)), Some(Ok(node))) = (
            parts.next().map(str::parse::<usize>),
            parts.next().map(str::parse::<u32>),
        ) else {
            continue;
        };
        by_file.entry(file).or_default().push((ek, node));
    }
    let mut rows = Vec::new();
    for (index, keys) in by_file {
        let Some(file) = batch.files.get(index) else {
            continue;
        };
        let answered = with_scopes(index, file, || -> Result<()> {
            for (ek, node) in keys {
                if node >= file.facts.len() {
                    continue;
                }
                let values: Vec<&str> = ek.split(',').skip(2).collect();
                if values.len() != term.reads.len() {
                    return Err(Error::ModuleError(format!("fact: malformed key {ek}")));
                }
                let Some(env) = environment(file, &term.reads, &values) else {
                    continue;
                };
                let verdict = term
                    .verdicts
                    .verdict(node_at(&file.root, node), &env)
                    .map_err(|limit| Error::ModuleError(format!("fact: limit {limit}")))?;
                if let Some(verdict) = verdict {
                    rows.push(FactRow {
                        ek,
                        unknown: verdict.unknown,
                        values: verdict.values,
                    });
                }
            }
            Ok(())
        });
        answered?;
    }
    Ok(rows)
}

const COL_TERM: c_int = 0;
const COL_EK: c_int = 1;
const COL_UNK: c_int = 2;

static SCHEMA: LazyLock<std::ffi::CString> = LazyLock::new(|| {
    let values = (0..FACT_COLUMNS)
        .map(|index| format!(", v{index}"))
        .collect::<String>();
    std::ffi::CString::new(format!("CREATE TABLE x(term HIDDEN, ek, unk{values})")).expect("no NUL")
});

#[repr(C)]
pub(in crate::code_rule_dsl::sql) struct FactTable {
    base: sqlite3_vtab,
    slot: Arc<BatchSlot>,
}

unsafe impl<'vtab> VTab<'vtab> for FactTable {
    type Aux = Arc<BatchSlot>;
    type Cursor = FactCursor<'vtab>;

    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Arc<BatchSlot>>,
        _module: &[u8],
        _database: &[u8],
        _table: &[u8],
        _args: &[&[u8]],
    ) -> Result<(std::borrow::Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        let slot = Arc::clone(aux.ok_or_else(|| Error::ModuleError("fact: no batch slot".into()))?);
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

    fn open(&'vtab mut self) -> Result<FactCursor<'vtab>> {
        Ok(FactCursor {
            base: sqlite3_vtab_cursor::default(),
            slot: Arc::clone(&self.slot),
            term: 0,
            rows: Vec::new(),
            pos: 0,
            phantom: PhantomData,
        })
    }
}

#[repr(C)]
pub(in crate::code_rule_dsl::sql) struct FactCursor<'vtab> {
    base: sqlite3_vtab_cursor,
    slot: Arc<BatchSlot>,
    term: i64,
    rows: Vec<FactRow>,
    pos: usize,
    phantom: PhantomData<&'vtab FactTable>,
}

unsafe impl VTabCursor for FactCursor<'_> {
    fn filter(&mut self, idx_num: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        #[cfg(test)]
        super::calls::note(&super::calls::FACT_FILTERS);
        if idx_num & 1 == 0 {
            return Err(Error::ModuleError(
                "fact requires a term and an input set".into(),
            ));
        }
        let batch = self
            .slot
            .current()
            .ok_or_else(|| Error::ModuleError("fact: no batch installed".into()))?;
        let term_id = integer(ValueRef::from(&args.get::<Value>(0)?))
            .ok_or_else(|| Error::ModuleError("fact: term must be an integer".into()))?;
        let Some(Terminal::Fact(term)) = usize::try_from(term_id)
            .ok()
            .and_then(|index| batch.program.terminals.get(index))
        else {
            return Err(Error::ModuleError(format!("fact: no fact term {term_id}")));
        };
        let mut keys = BTreeSet::new();
        if idx_num & 2 != 0 {
            let mut items = args.in_values(1)?;
            while let Some(item) = items.next()? {
                if let ValueRef::Text(text) = item {
                    keys.insert(String::from_utf8_lossy(text).into_owned());
                }
            }
        } else if let Value::Text(text) = args.get::<Value>(1)? {
            keys.insert(text);
        }
        self.rows = evaluate(&batch, term, keys)?;
        #[cfg(test)]
        super::calls::UNKNOWN_VERDICTS.set(
            super::calls::UNKNOWN_VERDICTS.get()
                + self.rows.iter().filter(|row| row.unknown).count(),
        );
        self.term = term_id;
        self.pos = 0;
        Ok(())
    }

    fn next(&mut self) -> Result<()> {
        self.pos += 1;
        Ok(())
    }

    fn eof(&self) -> bool {
        self.pos >= self.rows.len()
    }

    fn column(&self, context: &mut Context, column: c_int) -> Result<()> {
        let row = &self.rows[self.pos];
        match column {
            COL_TERM => context.set_result(&self.term),
            COL_EK => context.set_result(&row.ek),
            COL_UNK => context.set_result(&i64::from(row.unknown)),
            other => {
                let index = (other - 3) as usize;
                context.set_result(&row.values.get(index).copied().flatten())
            }
        }
    }

    fn rowid(&self) -> Result<i64> {
        Ok(self.pos as i64)
    }
}
