//! The `fact(term)` terminal: an input-set terminal returning three-valued verdicts. It receives
//! a frame's distinct keys in one `xFilter` call (`ek IN (…)`), each `file,node` and the bindings
//! of the captures the term reads, decodes each into the node and an environment holding those
//! bindings, and evaluates it natively (`Verdicts`) with the file's scopes installed: built on the
//! file's first key and parked between statements, so a batch builds each file's scope tables,
//! values and line tables once (`FileScopeSlot::park`). It returns the keys whose verdict is true
//! or unknown, with `unk` and the values the term binds; `Op::Fact` reads them into a frame. A
//! related-node terminal returns several rows per key, one per related node (`Walk::Related`).
//! A key's file may be a parsed tree of the batch (`exec::SubFile`), evaluated with its host
//! file's scopes.

use super::Terminal;
use crate::code_rule_dsl::literal::{ID_FLAG, SubDoc};
use crate::code_rule_dsl::matcher::{
    AstMatcher, FileScopeSlot, ParkedScopes, SubTables, export_text, exported_span, exported_text,
    leaf_env, leaf_verdict, node_at,
};
use crate::code_rule_dsl::soundness::SyntaxErrors;
use crate::code_rule_dsl::sql::exec::{Batch, BatchFile, BatchSlot};
use crate::code_rule_dsl::sql::nodes::{NodeFacts, integer, sub_index};
use crate::code_rule_dsl::sql::plan::SlotKind;
use crate::code_rule_dsl::sql::timing::{Started, Timing};
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

/// The most value columns a fact terminal returns (`v0`…).
pub(in crate::code_rule_dsl::sql) const FACT_COLUMNS: usize = 16;

thread_local! {
    /// The time this thread's fact terminals spent evaluating their keys, since it was last taken.
    static FACT_TIME: std::cell::Cell<Timing> = const { std::cell::Cell::new(Timing::ZERO) };
}

/// The time this thread's fact terminals spent evaluating their keys since the last call: a
/// session's statements run their terminals on its thread (`exec::Session::run`).
pub(in crate::code_rule_dsl::sql) fn take_fact_time() -> Timing {
    FACT_TIME.take()
}

/// A verdict that is not false: unknown or true, with the values of the term's columns.
pub(in crate::code_rule_dsl::sql) struct Verdict {
    pub(in crate::code_rule_dsl::sql) unknown: bool,
    pub(in crate::code_rule_dsl::sql) values: Vec<Value>,
}

impl Verdict {
    pub(in crate::code_rule_dsl::sql) fn unknown() -> Self {
        Self {
            unknown: true,
            values: Vec::new(),
        }
    }

    pub(in crate::code_rule_dsl::sql) fn holds(values: Vec<Value>) -> Self {
        Self {
            unknown: false,
            values,
        }
    }
}

/// What an input-set terminal evaluates per key: its rows at `node` with `env`, which holds the
/// bindings of the term's reads. No row is false; a verdict's terminal returns at most one, a
/// related-node terminal one per related node. `Err` is a limit the evaluation hit.
pub(in crate::code_rule_dsl::sql) trait Verdicts:
    Send + Sync
{
    fn rows<'t>(
        &self,
        node: Node<'t, TapeDoc>,
        env: &MetaVarEnv<'t, TapeDoc>,
    ) -> std::result::Result<Vec<Verdict>, String>;

    /// The rows at a node of a parsed literal's tree (`parse`); only the terms a tree's rule may
    /// hold read one.
    fn tree_rows<'t>(
        &self,
        _node: Node<'t, SubDoc>,
        _env: &MetaVarEnv<'t, SubDoc>,
    ) -> std::result::Result<Vec<Verdict>, String> {
        Err("this term reads no parsed tree".to_owned())
    }
}

/// A leaf the native matcher evaluates (`value`, `compare`, `layout`, `resolves`, `testUnknown`),
/// read through `matcher::leaf_verdict`: it binds nothing.
pub(in crate::code_rule_dsl::sql) struct Leaf(pub(in crate::code_rule_dsl::sql) AstMatcher);

fn leaf_rows<'t, D: Doc>(
    leaf: &AstMatcher,
    node: Node<'t, D>,
    env: &MetaVarEnv<'t, D>,
) -> std::result::Result<Vec<Verdict>, String> {
    match leaf_verdict(leaf, node, env) {
        Ok(Some(false)) => Ok(Vec::new()),
        Ok(Some(true)) => Ok(vec![Verdict::holds(Vec::new())]),
        Ok(None) => Ok(vec![Verdict::unknown()]),
        Err(limit) => Err(format!("{limit:?}")),
    }
}

impl Verdicts for Leaf {
    fn rows<'t>(
        &self,
        node: Node<'t, TapeDoc>,
        env: &MetaVarEnv<'t, TapeDoc>,
    ) -> std::result::Result<Vec<Verdict>, String> {
        leaf_rows(&self.0, node, env)
    }

    fn tree_rows<'t>(
        &self,
        node: Node<'t, SubDoc>,
        env: &MetaVarEnv<'t, SubDoc>,
    ) -> std::result::Result<Vec<Verdict>, String> {
        leaf_rows(&self.0, node, env)
    }
}

/// A leaf that binds (`arguments`' `bind`, `capture`) or exports (`calls`' `chain`): where it is
/// true, the values of `binds` in the environment it leaves.
pub(in crate::code_rule_dsl::sql) struct Binder {
    pub(in crate::code_rule_dsl::sql) leaf: AstMatcher,
    pub(in crate::code_rule_dsl::sql) binds: Vec<(String, SlotKind)>,
}

fn binder_rows<'t, D: Doc>(
    binder: &Binder,
    node: Node<'t, D>,
    env: &MetaVarEnv<'t, D>,
) -> std::result::Result<Vec<Verdict>, String> {
    match leaf_env(&binder.leaf, node, env) {
        Ok((Some(true), Some(bound))) => Ok(vec![Verdict::holds(values_of(&binder.binds, &bound))]),
        Ok((None, _)) => Ok(vec![Verdict::unknown()]),
        Ok(_) => Ok(Vec::new()),
        Err(limit) => Err(format!("{limit:?}")),
    }
}

impl Verdicts for Binder {
    fn rows<'t>(
        &self,
        node: Node<'t, TapeDoc>,
        env: &MetaVarEnv<'t, TapeDoc>,
    ) -> std::result::Result<Vec<Verdict>, String> {
        binder_rows(self, node, env)
    }

    fn tree_rows<'t>(
        &self,
        node: Node<'t, SubDoc>,
        env: &MetaVarEnv<'t, SubDoc>,
    ) -> std::result::Result<Vec<Verdict>, String> {
        binder_rows(self, node, env)
    }
}

/// A node's number in its document: a tape's id, or a parsed tree's preorder index.
pub(in crate::code_rule_dsl::sql) fn local_id(node_id: usize) -> i64 {
    if node_id & ID_FLAG != 0 {
        i64::from(sub_index(node_id))
    } else {
        node_id as i64
    }
}

/// The column values of `binds` in `env`, in order: a node's number, a count, an exported text
/// and its range; NULL where unbound. A list is not bound by a terminal.
pub(in crate::code_rule_dsl::sql) fn values_of<D: Doc>(
    binds: &[(String, SlotKind)],
    env: &MetaVarEnv<'_, D>,
) -> Vec<Value> {
    let mut values = Vec::new();
    for (name, kind) in binds {
        match kind {
            SlotKind::Node => values.push(
                env.get_match(name)
                    .map_or(Value::Null, |node| Value::Integer(local_id(node.node_id()))),
            ),
            SlotKind::Count => values.push(
                env.get_transformed(name)
                    .and_then(|bytes| {
                        <D::Source as Content>::encode_bytes(bytes)
                            .parse::<i64>()
                            .ok()
                    })
                    .map_or(Value::Null, Value::Integer),
            ),
            SlotKind::Text => match (exported_text(env, name), exported_span(env, name)) {
                (Some(text), Some(span)) => {
                    values.push(Value::Text(text));
                    values.push(Value::Text(format!("{}:{}", span.start, span.end)));
                }
                _ => values.extend([Value::Null, Value::Null]),
            },
            SlotKind::List => values.extend([Value::Null, Value::Null, Value::Null, Value::Null]),
        }
    }
    values
}

/// A fact term: what it evaluates and the captures its keys carry, in key order. Its value
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
/// batch, else fresh ones as the native evaluator installs them per file (its cached scope
/// tables, module, path and whether its syntax errors hide bindings), and its call-graph closure
/// where it has one, as the evaluator installs it for `calls`.
pub(in crate::code_rule_dsl::sql) fn with_scopes<T>(
    index: usize,
    file: &BatchFile,
    body: impl FnOnce() -> T,
) -> T {
    let parked = SCOPES.with_borrow_mut(|scopes| scopes.get_mut(index).and_then(Option::take));
    let slot = match parked {
        Some(parked) => FileScopeSlot::resume(parked),
        None => {
            let errors = SyntaxErrors::of_doc(file.root.root().get_doc());
            let slot = FileScopeSlot::install(
                file.context.scopes.clone(),
                file.context.module.clone(),
                errors.hide_bindings(),
            );
            match &file.context.path {
                Some(path) => slot.path(path),
                None => slot,
            }
        }
    };
    let calls = file.context.calls.as_ref().map(|(view, summary)| {
        crate::code_rule_dsl::calls::FileCallsSlot::install(Arc::clone(view), Arc::clone(summary))
    });
    let out = body();
    drop(calls);
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

/// Decodes the hexadecimal of a text a key carries (`hex()` of a text slot).
fn unhex(hex: &str) -> Option<String> {
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    String::from_utf8(bytes).ok()
}

/// The environment a key's capture values stand for, over a document whose node `id` is
/// `node(id)` and whose facts are `facts`; `None` for a value that names no node of it.
fn environment<'t, D: Doc>(
    facts: &NodeFacts,
    node: impl Fn(u32) -> Node<'t, D>,
    reads: &[(String, SlotKind)],
    values: &[&str],
) -> Option<MetaVarEnv<'t, D>> {
    let mut env = MetaVarEnv::new();
    for ((name, kind), value) in reads.iter().zip(values) {
        if value.is_empty() {
            continue;
        }
        match kind {
            SlotKind::Node => {
                let id: u32 = value.parse().ok()?;
                if id >= facts.len() {
                    return None;
                }
                env.insert(name, node(id))?;
            }
            SlotKind::List => {
                let (lo, hi) = value.split_once(':')?;
                let nodes = super::pattern::list_nodes(facts, lo.parse().ok()?, hi.parse().ok()?)
                    .into_iter()
                    .map(&node)
                    .collect();
                env.insert_multi(name, nodes)?;
            }
            SlotKind::Count => {
                let bytes = <D::Source as Content>::decode_str(value).into_owned();
                env.insert_transformation(&MetaVariable::Multiple, name, bytes);
            }
            SlotKind::Text => {
                let (text, range) = value.split_once(':')?;
                let (start, end) = range.split_once(':')?;
                let range = start.parse().ok()?..end.parse().ok()?;
                export_text(&mut env, name, &unhex(text)?, range);
            }
        }
    }
    Some(env)
}

/// One answered key: each of its rows.
struct FactRow {
    ek: Value,
    unknown: bool,
    values: Vec<Value>,
}

/// The values of a key after its `file,node`, one per read.
fn key_values<'k>(term: &FactTerm, ek: &'k str) -> Result<Vec<&'k str>> {
    let values: Vec<&str> = ek.split(',').skip(2).collect();
    if values.len() != term.reads.len() {
        return Err(Error::ModuleError(format!("fact: malformed key {ek}")));
    }
    Ok(values)
}

fn answered(rows: &mut Vec<FactRow>, ek: &Value, verdicts: Vec<Verdict>) {
    for verdict in verdicts {
        rows.push(FactRow {
            ek: ek.clone(),
            unknown: verdict.unknown,
            values: verdict.values,
        });
    }
}

thread_local! {
    /// The batch and file whose keys are being evaluated: a tree a terminal parses there
    /// becomes a document of the batch (`tree_file`).
    static EVALUATING: RefCell<Option<(Arc<Batch>, usize)>> = const { RefCell::new(None) };
}

/// The batch's number of `tree`, parsed from a node of the file whose keys are being evaluated
/// (`exec::Batch::add_subfile`); an annotation's with the table its names are read in.
pub(in crate::code_rule_dsl::sql) fn tree_file(
    tree: &Arc<crate::code_rule_dsl::literal::SubTree>,
    table: Option<Arc<crate::code_rule_dsl::scope::ScopeTable>>,
) -> Option<i64> {
    EVALUATING.with_borrow(|evaluating| {
        evaluating
            .as_ref()
            .map(|(batch, host)| i64::from(batch.add_subfile(*host, tree, table)))
    })
}

/// Evaluates `keys` of `term`, file by file.
type Inputs = BTreeMap<usize, Vec<(Value, u32, Vec<String>)>>;
fn evaluate(batch: &Arc<Batch>, term: &FactTerm, keys: BTreeSet<String>) -> Result<Vec<FactRow>> {
    let mut by_file: Inputs = BTreeMap::new();
    for ek in keys {
        let mut parts = ek.splitn(3, ',');
        let (Some(Ok(file)), Some(Ok(node))) = (
            parts.next().map(str::parse::<usize>),
            parts.next().map(str::parse::<u32>),
        ) else {
            continue;
        };
        let values = key_values(term, &ek)?
            .into_iter()
            .map(str::to_owned)
            .collect();
        by_file
            .entry(file)
            .or_default()
            .push((Value::Text(ek), node, values));
    }
    evaluate_inputs(batch, term, by_file)
}
fn evaluate_inputs(batch: &Arc<Batch>, term: &FactTerm, by_file: Inputs) -> Result<Vec<FactRow>> {
    let limit = |limit: String| Error::ModuleError(format!("fact: limit {limit}"));
    let mut rows = Vec::new();
    for (index, keys) in by_file {
        if let Some(file) = batch.files.get(index) {
            EVALUATING.set(Some((Arc::clone(batch), index)));
            let answered_all = with_scopes(index, file, || -> Result<()> {
                for (ek, node, values) in keys {
                    if node >= file.facts.len() {
                        continue;
                    }
                    let values: Vec<_> = values.iter().map(String::as_str).collect();
                    let Some(env) = environment(
                        &file.facts,
                        |id| node_at(&file.root, id),
                        &term.reads,
                        &values,
                    ) else {
                        continue;
                    };
                    let verdicts = term
                        .verdicts
                        .rows(node_at(&file.root, node), &env)
                        .map_err(limit)?;
                    answered(&mut rows, &ek, verdicts);
                }
                Ok(())
            });
            EVALUATING.set(None);
            answered_all?;
        } else if let Some(sub) = batch.subfile(index) {
            // A parsed tree's node: read with its host file's scopes, as the matcher reads it
            // while matching the host, and an annotation's in its file's table.
            let host = &batch.files[sub.host];
            let root = &sub.root;
            let at = |id: u32| root.adopt(root.root().get_doc().node(id));
            let _tables = sub.table.clone().map(SubTables::enter);
            let answered_all = with_scopes(sub.host, host, || -> Result<()> {
                for (ek, node, values) in keys {
                    if node >= sub.facts.len() {
                        continue;
                    }
                    let values: Vec<_> = values.iter().map(String::as_str).collect();
                    let Some(env) = environment(&sub.facts, at, &term.reads, &values) else {
                        continue;
                    };
                    let verdicts = term.verdicts.tree_rows(at(node), &env).map_err(limit)?;
                    answered(&mut rows, &ek, verdicts);
                }
                Ok(())
            });
            answered_all?;
        }
    }
    Ok(rows)
}

const COL_TERM: c_int = 0;
const COL_EK: c_int = 1;
const COL_UNK: c_int = 2;
const COL_NK: c_int = 3 + FACT_COLUMNS as c_int;
#[cfg(test)]
const COL_FRAME: c_int = COL_NK + 1;

static SCHEMA: LazyLock<std::ffi::CString> = LazyLock::new(|| {
    let values = (0..FACT_COLUMNS)
        .map(|index| format!(", v{index}"))
        .collect::<String>();
    let extra = if cfg!(test) { ", frame HIDDEN" } else { "" };
    std::ffi::CString::new(format!(
        "CREATE TABLE x(term HIDDEN, ek, unk{values}, nk HIDDEN{extra})"
    ))
    .expect("no NUL")
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
        let mut nk = None;
        #[cfg(test)]
        let mut frame = None;
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
                COL_NK => {
                    nk.get_or_insert(index);
                }
                COL_EK => {
                    ek.get_or_insert(index);
                }
                #[cfg(test)]
                COL_FRAME => {
                    frame.get_or_insert(index);
                }
                _ => {}
            }
        }
        let (Some(term), Some(ek)) = (term, nk.or(ek)) else {
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
        info.set_idx_num((if list { 3 } else { 1 }) | if nk.is_some() { 4 } else { 0 });
        #[cfg(test)]
        if let Some(frame) = frame {
            if nk.is_some() {
                return Err(Error::ModuleError(
                    "fact spike: frame and packed node keys cannot be combined".into(),
                ));
            }
            let mut usage = info.constraint_usage(frame);
            usage.set_argv_index(3);
            usage.set_omit(true);
            info.set_idx_num((if list { 3 } else { 1 }) | 8);
        }
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
        let started = Started::now();
        self.rows = match idx_num {
            #[cfg(test)]
            n if n & 8 != 0 => published_inputs(&self.slot, &batch, term, idx_num, args)?,
            _ => {
                if idx_num & 4 != 0 {
                    if !term.reads.is_empty() {
                        return Err(Error::ModuleError(
                            "fact: packed keys require no capture reads".into(),
                        ));
                    }
                    let mut keys = BTreeSet::new();
                    if idx_num & 2 != 0 {
                        let mut items = args.in_values(1)?;
                        while let Some(item) = items.next()? {
                            keys.extend(integer(item));
                        }
                    } else {
                        keys.extend(integer(ValueRef::from(&args.get::<Value>(1)?)));
                    }
                    let mut inputs: Inputs = BTreeMap::new();
                    for key in keys {
                        inputs
                            .entry((key >> 32) as u32 as usize)
                            .or_default()
                            .push((Value::Integer(key), key as u32, Vec::new()));
                    }
                    evaluate_inputs(&batch, term, inputs)?
                } else {
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
                    evaluate(&batch, term, keys)?
                }
            }
        };
        FACT_TIME.set(FACT_TIME.get().plus(started.elapsed()));
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
            COL_EK | COL_NK => context.set_result(&row.ek),
            COL_UNK => context.set_result(&i64::from(row.unknown)),
            other => {
                let index = (other - 3) as usize;
                context.set_result(row.values.get(index).unwrap_or(&Value::Null))
            }
        }
    }

    fn rowid(&self) -> Result<i64> {
        Ok(self.pos as i64)
    }
}

/// Integer terminal keys and typed input publication, only in compiler experiments. Native
/// environment reconstruction still uses the existing scalar decoder; no composite key is built.
#[cfg(test)]
fn published_inputs(
    slot: &BatchSlot,
    batch: &Arc<Batch>,
    term: &FactTerm,
    idx: c_int,
    args: &Filters<'_>,
) -> Result<Vec<FactRow>> {
    let descriptor = args.get::<String>(2)?;
    let width = term
        .reads
        .iter()
        .map(|(_, kind)| match kind {
            SlotKind::List => 4,
            SlotKind::Text => 2,
            _ => 1,
        })
        .sum::<usize>();
    if descriptor.split(',').count() != width + 1 {
        return Err(Error::ModuleError(
            "typed fact spike: input schema width differs from capture reads".into(),
        ));
    }
    let mut keys = Vec::new();
    if idx & 2 != 0 {
        let mut list = args.in_values(1)?;
        while let Some(value) = list.next()? {
            keys.extend(integer(value));
        }
    } else {
        keys.extend(integer(ValueRef::from(&args.get::<Value>(1)?)));
    }
    keys.sort_unstable();
    keys.dedup();
    let mut inputs: Inputs = BTreeMap::new();
    let mut unsupported = false;
    slot.frames
        .pattern_inputs(&descriptor, &keys, |key, file, node, values| {
            let mut columns = Vec::new();
            let mut at = 0usize;
            for (_, kind) in &term.reads {
                let value = |i: usize| values.get(at + i).copied().flatten();
                match kind {
                    SlotKind::Node | SlotKind::Count => {
                        columns.push(value(0).map_or(String::new(), |v| v.to_string()));
                        at += 1;
                    }
                    SlotKind::List => {
                        columns.push(match (value(0), value(1)) {
                            (Some(a), Some(b)) => format!("{a}:{b}"),
                            _ => String::new(),
                        });
                        at += 4;
                    }
                    SlotKind::Text => {
                        unsupported = true;
                        return;
                    }
                }
            }
            inputs
                .entry(file as usize)
                .or_default()
                .push((Value::Integer(key), node, columns));
        })?;
    if unsupported {
        return Err(Error::ModuleError(
            "typed fact spike: text/range reads need typed string columns".into(),
        ));
    }
    evaluate_inputs(batch, term, inputs)
}
