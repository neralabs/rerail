//! The `pattern(term)` terminal, an input-set terminal in both its variants. With an `nk IN (…)`
//! input (the fresh-env variant) it receives the distinct `(file, node)` keys of a frame
//! (`file << 32 | node`) in one `xFilter` call and returns those whose node the pattern matches in
//! a fresh environment, with what it bound; each (term, node) is matched once per batch
//! (`Batch::fresh`), whichever frames reach it. With an `ek IN (…)` input (the env-input variant)
//! it matches each input row's node with the row's bindings of the pattern's captures already in
//! the environment: an ellipsis's greedy alignment and every unification then happen natively,
//! exactly as the matcher does them.

use super::{PATTERN_COLUMNS, PatternTerm, Terminal};
use crate::code_rule_dsl::matcher::node_at;
use crate::code_rule_dsl::sql::exec::{Batch, BatchSlot};
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
use std::ffi::{CStr, c_int};
use std::hash::Hasher;
use std::marker::PhantomData;
use std::sync::{Arc, LazyLock};

/// One match: its node, the input row it answers (env-input) and its capture columns.
pub(in crate::code_rule_dsl::sql) struct PatternRow {
    file: u32,
    node: u32,
    ek: Option<i64>,
    values: Arc<[Option<i64>]>,
}

impl PatternRow {
    /// A fresh-env match at `node` of `file`, binding `values`.
    pub(in crate::code_rule_dsl::sql) fn fresh(
        file: u32,
        node: u32,
        values: Arc<[Option<i64>]>,
    ) -> Self {
        Self {
            file,
            node,
            ek: None,
            values,
        }
    }
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

type CaptureMatch = Option<Arc<[Option<i64>]>>;

/// The exact Smart root-capture subset. `None` leaves composite structural unification to the
/// general matcher; `Some(None)` is a proved nonmatch. Native insert overwrites a unified capture
/// with the candidate occurrence. Smart root metavariables also capture named extras.
fn root_capture(
    batch: &Batch,
    term: &PatternTerm,
    file: usize,
    node: u32,
    inputs: &[Option<i64>],
) -> Option<CaptureMatch> {
    if !term.root_capture {
        return None;
    }
    let entry = batch.files.get(file)?;
    if node >= entry.facts.len() {
        return Some(None);
    }
    let candidate = node_at(&entry.root, node);
    if !candidate.is_named()
        || term
            .kinds
            .as_ref()
            .is_some_and(|kinds| !kinds.contains(&candidate.kind_id()))
    {
        return Some(None);
    }
    if !term.reads.is_empty() {
        let bound = *inputs.first()?;
        if let Some(bound) = bound {
            let Ok(bound) = u32::try_from(bound) else {
                return Some(None);
            };
            if bound >= entry.facts.len() {
                return Some(None);
            }
            if bound != node {
                let bound = node_at(&entry.root, bound);
                if !bound.is_named_leaf() && !candidate.is_named_leaf() {
                    return None;
                }
                if bound.text() != candidate.text() {
                    return Some(None);
                }
            }
        }
    }
    Some(Some(Arc::from([Some(i64::from(node))])))
}

/// The capture columns of `term`'s fresh-env match at `node` of batch file `file`, where it
/// matches. Only a node of a batch file of the term's dialect, of one of its root kinds, can match.
pub(in crate::code_rule_dsl::sql) fn fresh_match(
    batch: &Batch,
    term: &PatternTerm,
    file: usize,
    node: u32,
) -> Option<Arc<[Option<i64>]>> {
    #[cfg(test)]
    super::calls::note(&super::calls::FRESH_MATCHES);
    let entry = batch.files.get(file)?;
    if entry.dialect != term.dialect || node >= entry.facts.len() {
        return None;
    }
    if let Some(kinds) = &term.kinds
        && !kinds.contains(&entry.facts.kind(node))
    {
        return None;
    }
    if let Some(result) = root_capture(batch, term, file, node, &[]) {
        return result;
    }
    let mut env = Cow::Owned(MetaVarEnv::new());
    term.pattern
        .match_node_with_env(node_at(&entry.root, node), &mut env)?;
    Some(values(term, &entry.facts, &env).into())
}

/// The frame owns the input tuple; only its integer row key crosses SQLite.
fn typed_match(
    batch: &Batch,
    term: &PatternTerm,
    file: u32,
    node: u32,
    inputs: &[Option<i64>],
) -> Option<Arc<[Option<i64>]>> {
    if let Some(result) = root_capture(batch, term, file as usize, node, inputs) {
        return result;
    }
    general_match(batch, term, file, node, inputs)
}

fn general_match(
    batch: &Batch,
    term: &PatternTerm,
    file: u32,
    node: u32,
    inputs: &[Option<i64>],
) -> Option<Arc<[Option<i64>]>> {
    let entry = batch.files.get(file as usize)?;
    if node >= entry.facts.len() {
        return None;
    }
    let mut env = MetaVarEnv::new();
    let mut inputs = inputs.iter();
    for &read in &term.reads {
        let (name, kind) = &term.vars[read];
        let first = *inputs.next()?;
        match kind {
            SlotKind::List => {
                let last = *inputs.next()?;
                if let (Some(lo), Some(hi)) = (first, last) {
                    let nodes = list_nodes(&entry.facts, lo, hi)
                        .into_iter()
                        .map(|id| node_at(&entry.root, id))
                        .collect();
                    env.insert_multi(name, nodes)?;
                }
            }
            _ => {
                if let Some(id) = first {
                    let id = u32::try_from(id).ok()?;
                    if id >= entry.facts.len() {
                        return None;
                    }
                    env.insert(name, node_at(&entry.root, id))?;
                }
            }
        }
    }
    let mut env = Cow::Owned(env);
    term.pattern
        .match_node_with_env(node_at(&entry.root, node), &mut env)?;
    Some(values(term, &entry.facts, &env).into())
}

const COL_TERM: c_int = 0;
const COL_EK: c_int = 1;
const COL_NK: c_int = 2;
const COL_FILE: c_int = 3;
const COL_NODE: c_int = 4;
/// The first value column (`v0`).
const COL_VALUES: c_int = 5;
const COL_FRAME: c_int = COL_VALUES + PATTERN_COLUMNS as c_int;

/// `idx_num` bits: a term, then the input set, one key (`EQ`) or a list (`IN`), of `ek` (env-input)
/// or `nk` (fresh-env).
const IDX_TERM: c_int = 1;
const IDX_EK: c_int = 2;
const IDX_EK_LIST: c_int = 4;
const IDX_NK: c_int = 8;
const IDX_NK_LIST: c_int = 16;
const IDX_FRAME: c_int = 32;

static SCHEMA: LazyLock<std::ffi::CString> = LazyLock::new(|| {
    let values = (0..PATTERN_COLUMNS)
        .map(|index| format!(", v{index}"))
        .collect::<String>();
    std::ffi::CString::new(format!(
        "CREATE TABLE x(term HIDDEN, ek HIDDEN, nk HIDDEN, file, node{values}, frame HIDDEN)"
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
        let mut nk = None;
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
                COL_EK => {
                    ek.get_or_insert(index);
                }
                COL_NK => {
                    nk.get_or_insert(index);
                }
                COL_FRAME => {
                    frame.get_or_insert(index);
                }
                _ => {}
            }
        }
        let input = ek
            .filter(|_| frame.is_some())
            .map(|ek| (ek, IDX_EK, IDX_EK_LIST))
            .or(nk.map(|nk| (nk, IDX_NK, IDX_NK_LIST)));
        let (Some(term), Some((input, one, list))) = (term, input) else {
            info.set_estimated_cost(f64::MAX);
            info.set_estimated_rows(i64::MAX);
            info.set_idx_num(0);
            return Ok(true);
        };
        let is_list = info.is_in_constraint(input)? && info.set_in_constraint(input, true)?;
        for (argv, index) in [term, input].into_iter().enumerate() {
            let mut usage = info.constraint_usage(index);
            usage.set_argv_index(argv as c_int + 1);
            usage.set_omit(true);
        }
        if let Some(frame) = frame {
            let mut usage = info.constraint_usage(frame);
            usage.set_argv_index(3);
            usage.set_omit(true);
        }
        info.set_idx_num(
            IDX_TERM
                | if is_list { list } else { one }
                | if frame.is_some() { IDX_FRAME } else { 0 },
        );
        info.set_estimated_cost(100.0);
        info.set_estimated_rows(100);
        Ok(true)
    }

    fn open(&'vtab mut self) -> Result<PatternCursor<'vtab>> {
        Ok(PatternCursor {
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
pub(in crate::code_rule_dsl::sql) struct PatternCursor<'vtab> {
    base: sqlite3_vtab_cursor,
    slot: Arc<BatchSlot>,
    term: i64,
    rows: Vec<PatternRow>,
    pos: usize,
    phantom: PhantomData<&'vtab PatternTable>,
}

unsafe impl VTabCursor for PatternCursor<'_> {
    fn filter(&mut self, idx_num: c_int, _: Option<&str>, args: &Filters<'_>) -> Result<()> {
        #[cfg(test)]
        super::calls::note(&super::calls::PATTERN_FILTERS);
        if idx_num & IDX_TERM == 0 {
            return Err(Error::ModuleError(
                "pattern requires a term and an input set".into(),
            ));
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
        self.rows = if idx_num & (IDX_NK | IDX_NK_LIST) != 0 {
            let mut keys = Vec::new();
            if idx_num & IDX_NK_LIST != 0 {
                let mut items = args.in_values(1)?;
                while let Some(item) = items.next()? {
                    keys.extend(integer(item));
                }
            } else {
                keys.extend(integer(ValueRef::from(&args.get::<Value>(1)?)));
            }
            keys.sort_unstable();
            keys.dedup();
            batch.fresh(term_id as usize, term, &keys)
        } else if idx_num & IDX_FRAME != 0 {
            {
                let descriptor = args.get::<String>(2)?;
                let mut keys = Vec::new();
                if idx_num & IDX_EK_LIST != 0 {
                    let mut items = args.in_values(1)?;
                    while let Some(item) = items.next()? {
                        keys.extend(integer(item));
                    }
                } else {
                    keys.extend(integer(ValueRef::from(&args.get::<Value>(1)?)));
                }
                keys.sort_unstable();
                keys.dedup();
                let mut rows = Vec::new();
                self.slot.frames.pattern_inputs(
                    &descriptor,
                    &keys,
                    |key, file, node, inputs| {
                        if let Some(values) = typed_match(&batch, term, file, node, inputs) {
                            rows.push(PatternRow {
                                file,
                                node,
                                ek: Some(key),
                                values,
                            });
                        }
                    },
                )?;
                rows
            }
        } else {
            return Err(Error::ModuleError(
                "pattern: environment inputs require an integer frame descriptor".into(),
            ));
        };
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
            COL_NK => context.set_result(&(i64::from(row.file) << 32 | i64::from(row.node))),
            COL_FILE => context.set_result(&i64::from(row.file)),
            COL_NODE => context.set_result(&i64::from(row.node)),
            COL_FRAME => context.set_result(&Value::Null),
            other => {
                let index = (other - COL_VALUES) as usize;
                context.set_result(&row.values.get(index).copied().flatten())
            }
        }
    }

    fn rowid(&self) -> Result<i64> {
        Ok(self.pos as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::{
        language::GrammarDialect,
        sql::{exec::with_nodes, tests::differential::tape},
    };
    use ast_grep_core::{MatchStrictness, Pattern};

    #[test]
    fn packed_root_capture_matches_general_unification_for_every_input_pair() {
        for (dialect, source) in [
            (
                GrammarDialect::Python,
                "# a\nf(a, a)\nx = 'a'\nf(a + b)\nf(\n",
            ),
            (
                GrammarDialect::TypeScript,
                "// a\nf(a, a); ({a}); let x = 'a'; f(;\n",
            ),
        ] {
            with_nodes(vec![tape(dialect, source)], |_, batch| {
                let mut patterns = vec![Pattern::try_new("$X", dialect).unwrap()];
                let mut cst = patterns[0].clone();
                cst.strictness = MatchStrictness::Cst;
                patterns.push(cst);
                if dialect == GrammarDialect::TypeScript {
                    patterns.push(Pattern::contextual("$X;", "identifier", dialect).unwrap());
                }
                for pattern in patterns {
                    let mut term = PatternTerm::new(pattern, dialect);
                    term.reads = vec![0];
                    let len = batch.files[0].facts.len();
                    for bound in std::iter::once(None).chain((0..len).map(|id| Some(i64::from(id))))
                    {
                        for candidate in 0..len {
                            assert_eq!(
                                typed_match(batch, &term, 0, candidate, &[bound]),
                                general_match(batch, &term, 0, candidate, &[bound]),
                                "{dialect:?}: {bound:?} -> {candidate}"
                            );
                        }
                    }
                }
            });
        }
    }
}
