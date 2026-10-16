//! The batch driver: builds the `node` table state from a batch's tapes, computes the shared
//! stages, runs one statement per clause and returns each clause's `(file, root, bindings)` rows
//! for the native witness rendering.

use super::emit;
use super::lower::ProgramBuilder;
use super::nodes::{NodeFacts, NodeTable};
use super::plan::{SlotKind, Statement, Unsupported};
use super::terminals::{
    BatchScopes, FactTable, PatternRow, PatternTable, PatternTerm, Terminal, TextTable, fresh_rows,
};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::GrammarMatcher;
use crate::code_rule_dsl::tape::TapeDoc;
use ast_grep_core::AstGrep;
use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
use ast_grep_core::source::Content;
use ast_grep_core::{Doc, Node};
use parking_lot::RwLock;
use rusqlite::Connection;
use rusqlite::types::Value;
use rusqlite::vtab::Module;
use std::sync::{Arc, OnceLock};

/// One file of a batch.
pub(in crate::code_rule_dsl) struct BatchFile {
    pub(in crate::code_rule_dsl) root: AstGrep<TapeDoc>,
    pub(in crate::code_rule_dsl) dialect: GrammarDialect,
    pub(super) facts: NodeFacts,
    /// What the native evaluator knows of the file beyond its tape (`FileContext`).
    pub(super) context: FileContext,
}

/// What the native evaluator installs for a file besides its tree: its host path and Python
/// module path, which the input-set terminals' file scopes read (`FileScopeSlot`).
#[derive(Clone, Default)]
pub(in crate::code_rule_dsl) struct FileContext {
    pub(in crate::code_rule_dsl) path: Option<String>,
    pub(in crate::code_rule_dsl) module: Option<Arc<crate::code_rule_dsl::module_path::ModulePath>>,
}

/// The terminals and stages of the clauses a batch runs.
pub(super) struct Program {
    pub(super) terminals: Vec<Terminal>,
}

/// A batch: its files' tapes and node facts, its program, and the fresh-env matches of each
/// pattern terminal, computed on first read.
pub(super) struct Batch {
    pub(super) files: Vec<BatchFile>,
    pub(super) program: Arc<Program>,
    fresh: Vec<OnceLock<Arc<Vec<PatternRow>>>>,
}

impl Batch {
    /// A batch of `docs`, each with its context, their node facts computed once each, for a
    /// program's `terminals`.
    pub(super) fn new(docs: Vec<(TapeDoc, FileContext)>, terminals: Vec<Terminal>) -> Arc<Self> {
        let files = docs
            .into_iter()
            .map(|(doc, context)| {
                let dialect = *doc.get_lang();
                let root = AstGrep::doc(doc);
                let facts = NodeFacts::of(&root);
                BatchFile {
                    root,
                    dialect,
                    facts,
                    context,
                }
            })
            .collect();
        Arc::new(Self {
            files,
            fresh: terminals.iter().map(|_| OnceLock::new()).collect(),
            program: Arc::new(Program { terminals }),
        })
    }

    /// The fresh-env matches of pattern terminal `term`, computed once per batch.
    pub(super) fn fresh(&self, index: usize, term: &PatternTerm) -> Arc<Vec<PatternRow>> {
        Arc::clone(self.fresh[index].get_or_init(|| Arc::new(fresh_rows(self, term))))
    }
}

/// Where a connection's virtual tables find the batch being evaluated.
#[derive(Default)]
pub(in crate::code_rule_dsl) struct BatchSlot {
    current: RwLock<Option<Arc<Batch>>>,
}

impl BatchSlot {
    pub(super) fn current(&self) -> Option<Arc<Batch>> {
        self.current.read().clone()
    }
}

// Constants: `&MODULE` is promoted to a `'static` borrow, as `create_module` needs.
const NODE_MODULE: Module<'static, NodeTable> = Module::eponymous_only_module();
const PATTERN_MODULE: Module<'static, PatternTable> = Module::eponymous_only_module();
const TEXT_MODULE: Module<'static, TextTable> = Module::eponymous_only_module();
const FACT_MODULE: Module<'static, FactTable> = Module::eponymous_only_module();

/// An in-memory connection whose virtual tables read `slot`'s batch.
fn connect(slot: &Arc<BatchSlot>) -> Result<Connection, SqlError> {
    let connection = Connection::open_in_memory()?;
    connection.create_module(c"node", &NODE_MODULE, Some(Arc::clone(slot)))?;
    connection.create_module(c"pattern", &PATTERN_MODULE, Some(Arc::clone(slot)))?;
    connection.create_module(c"text_match", &TEXT_MODULE, Some(Arc::clone(slot)))?;
    connection.create_module(c"fact", &FACT_MODULE, Some(Arc::clone(slot)))?;
    Ok(connection)
}

/// `body` with a connection over a batch of `docs` (no terminals), for tests of the `node` table.
#[cfg(test)]
pub(super) fn with_nodes<T>(docs: Vec<TapeDoc>, body: impl FnOnce(&Connection, &Batch) -> T) -> T {
    let batch = Batch::new(
        docs.into_iter()
            .map(|doc| (doc, FileContext::default()))
            .collect(),
        Vec::new(),
    );
    let slot = Arc::new(BatchSlot::default());
    *slot.current.write() = Some(Arc::clone(&batch));
    let connection = connect(&slot).expect("connects");
    body(&connection, &batch)
}

/// A clause's binding of one capture, as the native renderer reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) enum Captured {
    Node(u32),
    /// A list's first and last node; `None` for a list bound to no node.
    List(Option<(u32, u32)>),
    /// A count's `as`.
    Count(i64),
}

/// One matching root of a clause and its bindings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct SqlMatch {
    pub(in crate::code_rule_dsl) root: u32,
    pub(in crate::code_rule_dsl) captures: Vec<(String, Captured)>,
}

/// Why the SQL engine did not answer.
#[derive(Debug)]
pub(in crate::code_rule_dsl) enum SqlError {
    Unsupported(Unsupported),
    Sql(String),
}

impl SqlError {
    /// The error for an operator the engine does not lower.
    pub(in crate::code_rule_dsl) fn unsupported(operator: &str) -> Self {
        Self::Unsupported(Unsupported(operator.to_owned()))
    }
}

impl std::fmt::Display for SqlError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported(unsupported) => write!(formatter, "SQL engine: {unsupported}"),
            Self::Sql(message) => write!(formatter, "SQL engine: {message}"),
        }
    }
}

impl From<rusqlite::Error> for SqlError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Sql(error.to_string())
    }
}

/// Clauses compiled for the SQL engine: their statements and the terminals and stages they share.
pub(in crate::code_rule_dsl) struct Compiled {
    builder: ProgramBuilder,
    clauses: Vec<(Statement, Vec<String>)>,
}

/// Compiles `grammars`, one clause each; `Unsupported` where one holds an operator the engine
/// does not lower.
pub(in crate::code_rule_dsl) fn compile(
    grammars: &[&GrammarMatcher],
) -> Result<Compiled, SqlError> {
    let mut builder = ProgramBuilder::default();
    let mut clauses = Vec::new();
    for grammar in grammars {
        let statement = builder.clause(grammar).map_err(SqlError::Unsupported)?;
        let sql = emit::clause(&statement);
        clauses.push((statement, sql));
    }
    Ok(Compiled { builder, clauses })
}

impl Compiled {
    /// How many statements one batch runs: each stage's, then each clause's.
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn statement_count(&self) -> (usize, usize) {
        (
            self.builder
                .stages
                .iter()
                .map(|stage| stage.statements.len() + 1)
                .sum(),
            self.clauses.iter().map(|(_, sql)| sql.len()).sum(),
        )
    }

    /// Runs the batch: `files` are its tapes, each with the statement (by index into the
    /// compiled grammars) that matches it, if any, and what the file's scopes read beyond its tape
    /// (`FileContext`). Returns each file's matches, ordered by root.
    pub(in crate::code_rule_dsl) fn run(
        self,
        files: Vec<(TapeDoc, Option<usize>, FileContext)>,
    ) -> Result<Vec<Vec<SqlMatch>>, SqlError> {
        let Compiled { builder, clauses } = self;
        let assigned: Vec<Option<usize>> =
            files.iter().map(|(_, statement, _)| *statement).collect();
        let batch = Batch::new(
            files
                .into_iter()
                .map(|(doc, _, context)| (doc, context))
                .collect(),
            builder.terminals,
        );
        let slot = Arc::new(BatchSlot::default());
        *slot.current.write() = Some(Arc::clone(&batch));
        let _scopes = BatchScopes::begin();
        let connection = connect(&slot)?;
        let file_list = |wanted: &dyn Fn(usize, &BatchFile) -> bool| -> String {
            let listed: Vec<String> = batch
                .files
                .iter()
                .enumerate()
                .filter(|(index, file)| wanted(*index, file))
                .map(|(index, _)| index.to_string())
                .collect();
            if listed.is_empty() {
                "NULL".to_owned()
            } else {
                listed.join(", ")
            }
        };
        let run = |statements: &[String], namespace: &str, files: &str| -> Result<(), SqlError> {
            for statement in statements {
                let sql = statement
                    .replace(emit::NAMESPACE, namespace)
                    .replace(emit::FILES, files);
                connection.execute_batch(&sql)?;
            }
            Ok(())
        };
        for (index, stage) in builder.stages.iter().enumerate() {
            let table = emit::stage_table(index);
            let namespace = format!("s{index}_");
            let files = file_list(&|_, file| file.dialect == stage.dialect);
            let (query, steps) = stage.statements.split_last().expect("a final select");
            run(steps, &namespace, &files)?;
            // A three-valued stage keeps the nodes it is unknown at too, with `unk`.
            let columns = if stage.three {
                "file, node, unk"
            } else {
                "file, node"
            };
            connection.execute_batch(&format!(
                "CREATE TEMP TABLE {table}(file INTEGER, node INTEGER, unk INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (file, node)) WITHOUT ROWID; \
                 INSERT INTO {table}({columns}) {}",
                query.replace(emit::NAMESPACE, &namespace)
            ))?;
        }
        let mut results: Vec<Vec<SqlMatch>> = batch.files.iter().map(|_| Vec::new()).collect();
        for (index, (statement, sql)) in clauses.iter().enumerate() {
            let files = file_list(&|file, _| assigned[file] == Some(index));
            if files == "NULL" {
                continue;
            }
            let namespace = format!("c{index}_");
            let (query, steps) = sql.split_last().expect("a final select");
            run(steps, &namespace, &files)?;
            let mut prepared = connection.prepare(&query.replace(emit::NAMESPACE, &namespace))?;
            let captures: Vec<(String, SlotKind, usize)> = {
                let mut column = 2;
                statement
                    .slots
                    .iter()
                    .enumerate()
                    .filter(|(_, slot)| slot.scope == 0)
                    .map(|(id, slot)| {
                        let at = column;
                        column += slot.columns(id).len();
                        (slot.name.clone(), slot.kind, at)
                    })
                    .collect()
            };
            let mut rows = prepared.query([])?;
            while let Some(row) = rows.next()? {
                let file: i64 = row.get(0)?;
                let root: i64 = row.get(1)?;
                let mut bound = Vec::new();
                for (name, kind, at) in &captures {
                    let value = |offset: usize| -> rusqlite::Result<Option<i64>> {
                        match row.get::<_, Value>(at + offset)? {
                            Value::Integer(value) => Ok(Some(value)),
                            _ => Ok(None),
                        }
                    };
                    let captured = match kind {
                        SlotKind::Node => value(0)?.map(|id| Captured::Node(id as u32)),
                        SlotKind::Count => value(0)?.map(Captured::Count),
                        SlotKind::List => match (value(0)?, value(1)?) {
                            (Some(-1), _) => Some(Captured::List(None)),
                            (Some(lo), Some(hi)) => {
                                Some(Captured::List(Some((lo as u32, hi as u32))))
                            }
                            _ => None,
                        },
                    };
                    if let Some(captured) = captured {
                        bound.push((name.clone(), captured));
                    }
                }
                results[file as usize].push(SqlMatch {
                    root: root as u32,
                    captures: bound,
                });
            }
        }
        *slot.current.write() = None;
        Ok(results)
    }
}

/// The environment a SQL match stands for, over `root`, the same file's tape: what the native
/// matcher would have bound at the root.
pub(in crate::code_rule_dsl) fn environment<'t>(
    root: &'t AstGrep<TapeDoc>,
    found: &SqlMatch,
) -> MetaVarEnv<'t, TapeDoc> {
    use crate::code_rule_dsl::matcher::node_at;
    let mut env = MetaVarEnv::new();
    for (name, captured) in &found.captures {
        match captured {
            Captured::Node(id) => {
                env.insert(name, node_at(root, *id));
            }
            Captured::List(range) => {
                let mut nodes: Vec<Node<'t, TapeDoc>> = Vec::new();
                if let Some((lo, hi)) = *range {
                    let mut at = node_at(root, lo);
                    loop {
                        let id = at.node_id() as u32;
                        nodes.push(at.clone());
                        if id == hi {
                            break;
                        }
                        // The structural next sibling: a list is a run of one parent's children.
                        let Some(next) = root.root().get_doc().node(id).next_sibling() else {
                            break;
                        };
                        at = root.adopt(next);
                    }
                }
                env.insert_multi(name, nodes);
            }
            Captured::Count(value) => {
                let bytes = <<TapeDoc as Doc>::Source as Content>::decode_str(&value.to_string())
                    .into_owned();
                env.insert_transformation(&MetaVariable::Multiple, name, bytes);
            }
        }
    }
    env
}
