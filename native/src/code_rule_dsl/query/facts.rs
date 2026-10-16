//! `rerail_facts`: the semantic facts a Rerail predicate reads at one node, as rows SQL composes
//! sub-rules over (`matcher::facts` defines the slots).
//!
//! A predicate's facts at a node and environment are computed once per batch by the predicate's
//! own code (over the file's scope and usage tables, cached per file version) and answered from
//! that cache however often a statement asks. A parsed literal a fact names becomes a derived
//! document of the batch (`docs`), so its rows point at that document's nodes.

use super::BatchState;
use super::catalog::Term;
use super::docs::{self, Document, batch_state, derive, document};
use crate::code_rule_dsl::matcher::facts::{self as sources, Target};
use crate::code_rule_dsl::transducer::sql_backend::{BatchView, MatchRegistry};
use ast_grep_core::Doc;
use rusqlite::ffi;
use rusqlite::vtab::{
    Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
    VTabCursor,
};
use rusqlite::{Connection, Result as SqlResult};
use std::borrow::Cow;
use std::ffi::{CStr, c_int};
use std::marker::PhantomData;
use std::sync::Arc;

/// One fact row.
pub(super) struct Row {
    slot: u16,
    key: i64,
    pos: i64,
    flag: i64,
    /// The document and node the row points at, with the node's kind.
    target: Option<(i64, u32, u16)>,
}

/// A predicate's facts at one node, ordered by slot, key and position.
pub(super) struct Facts {
    rows: Vec<Row>,
}

pub(super) fn register(connection: &Connection, registry: &Arc<MatchRegistry>) -> SqlResult<()> {
    const FACTS: Module<FactsTable> = Module::eponymous_only_module();
    connection.create_module(c"rerail_facts", &FACTS, Some(Arc::clone(registry)))?;
    // tq_may_have_facts(batch, term, file, node): false only where the predicate's `rerail_facts`
    // rows at the node are none in every environment (`matcher::facts::may_have_facts`), which a
    // statement then need not read.
    let owner = Arc::clone(registry);
    connection.create_scalar_function(
        "tq_may_have_facts",
        4,
        rusqlite::functions::FunctionFlags::SQLITE_UTF8,
        move |context| {
            let view = owner
                .files(context.get::<i64>(0)?)
                .ok_or_else(|| module_error("tq_may_have_facts batch is missing"))?;
            let state =
                batch_state(&view).ok_or_else(|| module_error("tq_may_have_facts batch state"))?;
            let (Ok(term), Some(file), Some(Ok(of))) = (
                usize::try_from(context.get::<i64>(1)?),
                context.get::<Option<i64>>(2)?,
                context.get::<Option<i64>>(3)?.map(u32::try_from),
            ) else {
                return Ok(true);
            };
            let Some(Term::Semantic(leaf)) = state.catalog.terms.get(term) else {
                return Err(module_error("tq_may_have_facts term is not a predicate"));
            };
            let Some(document) = document(&view, Some(state), file) else {
                return Ok(true);
            };
            Ok(match &document {
                Document::File(batch_file) if (of as usize) < document.len() => {
                    sources::may_have_facts(leaf, batch_file.node(of))
                }
                // A parsed literal reads its own table; its facts are read as they are.
                _ => true,
            })
        },
    )
}

fn module_error(message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::ModuleError(message.into())
}

/// The facts of the predicate at `term` at node `of` of document `file`, in environment `env`:
/// computed on first use, then cached for the batch. `Err` names a limit the predicate hit.
fn facts(
    view: &BatchView,
    state: &BatchState,
    term: usize,
    file: i64,
    of: u32,
    env: i64,
) -> SqlResult<Arc<Facts>> {
    let key = (term, file, of, env);
    if let Some(found) = state
        .facts
        .lock()
        .expect("facts cache mutex poisoned")
        .get(&key)
    {
        #[cfg(feature = "code-rule-profile")]
        state
            .fact_hits
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return Ok(Arc::clone(found));
    }
    #[cfg(feature = "code-rule-profile")]
    state
        .fact_builds
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let Some(Term::Semantic(leaf)) = state.catalog.terms.get(term) else {
        return Err(module_error("rerail_facts term is not a predicate"));
    };
    let bound = state
        .envs
        .get(env)
        .ok_or_else(|| module_error("environment is missing"))?;
    let document = document(view, Some(state), file)
        .ok_or_else(|| module_error("rerail_facts document is missing"))?;
    if of as usize >= document.len() {
        return Ok(Arc::new(Facts { rows: Vec::new() }));
    }
    let rows = match &document {
        Document::File(batch_file) => {
            let meta = bound.to_meta(batch_file);
            let found = sources::facts(leaf, batch_file.node(of), &meta);
            rows(view, state, file, found, |id| id as u32)
        }
        Document::Derived(derived) => sources::with_sub_table(derived.table.as_ref(), || {
            let meta = bound.meta_with(|ordinal| derived.node(ordinal));
            let found = sources::facts(leaf, derived.node(of), &meta);
            rows(view, state, file, found, docs::derived_ordinal)
        }),
    };
    if let Some(limit) = sources::limit_hit() {
        return Err(module_error(limit.message()));
    }
    let facts = Arc::new(Facts { rows });
    state
        .facts
        .lock()
        .expect("facts cache mutex poisoned")
        .insert(key, Arc::clone(&facts));
    Ok(facts)
}

/// The matcher's facts as rows: nodes of the document by ordinal, parsed literals registered as
/// derived documents of `file`.
fn rows<D: Doc>(
    view: &BatchView,
    state: &BatchState,
    file: i64,
    found: Option<sources::Facts<'_, D>>,
    ordinal: impl Fn(usize) -> u32,
) -> Vec<Row> {
    let Some(found) = found else {
        return Vec::new();
    };
    let mut rows = found
        .rows
        .into_iter()
        .map(|fact| {
            let target = match fact.target {
                Target::None => None,
                Target::Node(node) => Some((file, ordinal(node.node_id()), node.kind_id())),
                Target::Tree(tree, table) => {
                    let index = derive(view, state, &tree, file, table);
                    let root = docs::document(view, Some(state), index);
                    let kind = match root {
                        Some(Document::Derived(derived)) => derived.node(0).kind_id(),
                        _ => 0,
                    };
                    Some((index, 0, kind))
                }
                Target::TreeNode(tree, id) => {
                    let index = derive(view, state, &tree, file, None);
                    let ordinal = docs::derived_ordinal(id);
                    let kind = match docs::document(view, Some(state), index) {
                        Some(Document::Derived(derived)) => derived.node(ordinal).kind_id(),
                        _ => 0,
                    };
                    Some((index, ordinal, kind))
                }
            };
            Row {
                slot: fact.slot,
                key: fact.key,
                pos: fact.pos,
                flag: fact.flag,
                target,
            }
        })
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| (row.slot, row.key, row.pos));
    rows
}

const BATCH: c_int = 0;
const TERM: c_int = 1;
const FILE: c_int = 2;
const OF: c_int = 3;
const ENV: c_int = 4;
const SLOT: c_int = 5;
const KEY: c_int = 6;
const POS: c_int = 7;

#[repr(C)]
struct FactsTable {
    base: ffi::sqlite3_vtab,
    registry: Arc<MatchRegistry>,
}

unsafe impl<'vtab> VTab<'vtab> for FactsTable {
    type Aux = Arc<MatchRegistry>;
    type Cursor = FactsCursor<'vtab>;

    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Arc<MatchRegistry>>,
        _module_name: &[u8],
        _database_name: &[u8],
        _table_name: &[u8],
        _args: &[&[u8]],
    ) -> SqlResult<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        let registry = Arc::clone(aux.ok_or_else(|| module_error("rerail_facts registry"))?);
        Ok((
            Cow::Borrowed(
                c"CREATE TABLE x(batch HIDDEN,term HIDDEN,file HIDDEN,of HIDDEN,env HIDDEN,slot HIDDEN,key HIDDEN,pos,node,flag,doc,kind)",
            ),
            Self {
                base: ffi::sqlite3_vtab::default(),
                registry,
            },
        ))
    }

    fn best_index(&self, info: &mut IndexInfo) -> SqlResult<bool> {
        // Slots: batch, term, file, of, env, slot, key.
        let mut slots: [Option<usize>; 7] = [None; 7];
        for (index, constraint) in info.constraints().enumerate() {
            if !constraint.is_usable()
                || constraint.operator() != IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
            {
                continue;
            }
            let column = constraint.column();
            if (BATCH..=KEY).contains(&column) {
                slots[column as usize].get_or_insert(index);
            }
        }
        if slots[..5].iter().any(Option::is_none) {
            info.set_idx_num(0);
            info.set_estimated_cost(f64::MAX);
            info.set_estimated_rows(i64::MAX);
            return Ok(true);
        }
        let mut idx_num = 0;
        let mut argv = 0;
        for (slot, constraint) in slots.iter().enumerate() {
            if let Some(constraint) = constraint {
                argv += 1;
                idx_num |= 1 << slot;
                let mut usage = info.constraint_usage(*constraint);
                usage.set_argv_index(argv);
                usage.set_omit(true);
            }
        }
        let ordered = info.num_of_order_by() == 1
            && info
                .order_bys()
                .next()
                .is_some_and(|order| order.column() == POS && !order.is_order_by_desc());
        if ordered {
            info.set_order_by_consumed(true);
        }
        info.set_idx_num(idx_num);
        info.set_estimated_cost(1.0);
        info.set_estimated_rows(4);
        Ok(true)
    }

    fn open(&mut self) -> SqlResult<Self::Cursor> {
        Ok(FactsCursor {
            base: ffi::sqlite3_vtab_cursor::default(),
            registry: Arc::clone(&self.registry),
            facts: None,
            rows: Vec::new(),
            position: 0,
            phantom: PhantomData,
        })
    }
}

#[repr(C)]
struct FactsCursor<'vtab> {
    base: ffi::sqlite3_vtab_cursor,
    registry: Arc<MatchRegistry>,
    facts: Option<Arc<Facts>>,
    /// Indices of the rows the constraints select.
    rows: Vec<usize>,
    position: usize,
    phantom: PhantomData<&'vtab FactsTable>,
}

unsafe impl VTabCursor for FactsCursor<'_> {
    fn filter(&mut self, idx_num: c_int, _: Option<&str>, args: &Filters<'_>) -> SqlResult<()> {
        if idx_num & 0b1_1111 != 0b1_1111 {
            return Err(module_error(
                "rerail_facts requires a batch, term, file, node and environment",
            ));
        }
        self.position = 0;
        self.rows.clear();
        self.facts = None;
        let view = self
            .registry
            .files(args.get::<i64>(0)?)
            .ok_or_else(|| module_error("rerail_facts batch is missing"))?;
        let term = args.get::<i64>(1)?;
        let file = args.get::<Option<i64>>(2)?;
        let of = args.get::<Option<i64>>(3)?;
        let env = args.get::<Option<i64>>(4)?;
        let mut argv = 5;
        let slot = if idx_num & (1 << 5) != 0 {
            let slot = args.get::<i64>(argv)?;
            argv += 1;
            Some(slot)
        } else {
            None
        };
        let key = if idx_num & (1 << 6) != 0 {
            Some(args.get::<i64>(argv)?)
        } else {
            None
        };
        let (Some(file), Some(of), Some(env)) = (file, of, env) else {
            return Ok(());
        };
        let (Ok(term), Ok(of)) = (usize::try_from(term), u32::try_from(of)) else {
            return Ok(());
        };
        if env < 0 {
            return Ok(());
        }
        let state = batch_state(&view).ok_or_else(|| module_error("rerail_facts batch state"))?;
        let facts = facts(&view, state, term, file, of, env)?;
        self.rows = facts
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                slot.is_none_or(|slot| i64::from(row.slot) == slot)
                    && key.is_none_or(|key| row.key == key)
            })
            .map(|(index, _)| index)
            .collect();
        self.facts = Some(facts);
        Ok(())
    }

    fn next(&mut self) -> SqlResult<()> {
        self.position += 1;
        Ok(())
    }

    fn eof(&self) -> bool {
        self.position >= self.rows.len()
    }

    fn column(&self, context: &mut Context, column: c_int) -> SqlResult<()> {
        let row = &self.facts.as_ref().expect("filtered cursor").rows[self.rows[self.position]];
        match column {
            BATCH | TERM | FILE | OF | ENV => context.set_result(&Option::<i64>::None),
            SLOT => context.set_result(&i64::from(row.slot)),
            KEY => context.set_result(&row.key),
            POS => context.set_result(&row.pos),
            8 => context.set_result(&row.target.map(|(_, node, _)| i64::from(node))),
            9 => context.set_result(&row.flag),
            10 => context.set_result(&row.target.map(|(doc, _, _)| doc)),
            11 => context.set_result(&row.target.map(|(_, _, kind)| i64::from(kind))),
            _ => Err(rusqlite::Error::InvalidColumnIndex(column as usize)),
        }
    }

    fn rowid(&self) -> SqlResult<i64> {
        Ok(self.position as i64)
    }
}
