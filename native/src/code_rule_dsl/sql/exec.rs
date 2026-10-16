//! The batch driver: builds the `node` table state from a batch's tapes, computes the shared
//! stages, runs one statement per clause and returns each clause's `(file, root, bindings)` rows
//! for the native witness rendering.

use super::emit;
use super::lower::ProgramBuilder;
use super::nodes::{NodeFacts, NodeTable};
use super::plan::{SlotKind, Statement, Unsupported};
use super::terminals::{PatternRow, PatternTable, PatternTerm, Terminal, TextTable, fresh_rows};
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
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

/// One file of a batch.
pub(in crate::code_rule_dsl) struct BatchFile {
    pub(in crate::code_rule_dsl) root: AstGrep<TapeDoc>,
    pub(in crate::code_rule_dsl) dialect: GrammarDialect,
    pub(super) facts: NodeFacts,
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
    /// A batch of `docs`, their node facts computed once each, for a program's `terminals`.
    pub(super) fn new(docs: Vec<TapeDoc>, terminals: Vec<Terminal>) -> Arc<Self> {
        let files = docs
            .into_iter()
            .map(|doc| {
                let dialect = *doc.get_lang();
                let root = AstGrep::doc(doc);
                let facts = NodeFacts::of(&root);
                BatchFile {
                    root,
                    dialect,
                    facts,
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

/// An in-memory connection whose virtual tables read `slot`'s batch.
fn connect(slot: &Arc<BatchSlot>) -> Result<Connection, SqlError> {
    let connection = Connection::open_in_memory()?;
    // Recursion loops run their statements once per pass: every one stays prepared.
    connection.set_prepared_statement_cache_capacity(4096);
    connection.create_module(c"node", &NODE_MODULE, Some(Arc::clone(slot)))?;
    connection.create_module(c"pattern", &PATTERN_MODULE, Some(Arc::clone(slot)))?;
    connection.create_module(c"text_match", &TEXT_MODULE, Some(Arc::clone(slot)))?;
    Ok(connection)
}

/// `body` with a connection over a batch of `docs` (no terminals), for tests of the `node` table.
#[cfg(test)]
pub(super) fn with_nodes<T>(docs: Vec<TapeDoc>, body: impl FnOnce(&Connection, &Batch) -> T) -> T {
    let batch = Batch::new(docs, Vec::new());
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
    /// Whether the clauses enumerate `violation.each`: their statements run in rounds.
    each: bool,
}

/// Compiles `grammars`, one clause each; `Unsupported` where one holds an operator the engine
/// does not lower.
#[cfg(test)]
pub(in crate::code_rule_dsl) fn compile(
    grammars: &[&GrammarMatcher],
) -> Result<Compiled, SqlError> {
    compile_each(grammars, None)
}

/// `compile`, with `violation.each` enumerating the capture `each` at every matching root.
pub(in crate::code_rule_dsl) fn compile_each(
    grammars: &[&GrammarMatcher],
    each: Option<&str>,
) -> Result<Compiled, SqlError> {
    let mut builder = ProgramBuilder::default();
    let mut clauses = Vec::new();
    for grammar in grammars {
        let statement = builder
            .clause_each(grammar, each)
            .map_err(SqlError::Unsupported)?;
        let sql = match each {
            Some(name) => {
                let slot = statement.slots.iter().position(|slot| {
                    slot.scope == 0 && slot.kind == SlotKind::Node && slot.name == name
                });
                emit::clause_each(&statement, slot)
            }
            None => emit::clause(&statement),
        };
        clauses.push((statement, sql));
    }
    Ok(Compiled {
        builder,
        clauses,
        each: each.is_some(),
    })
}

/// Every operator the engine would refuse in `grammar`'s clause (`each`: its `violation.each`),
/// not only the first: each refusal is recorded and lowered as `Never`.
#[cfg(test)]
pub(in crate::code_rule_dsl) fn census(grammar: &GrammarMatcher, each: Option<&str>) -> Vec<String> {
    let mut builder = ProgramBuilder::default();
    builder.census = Some(Vec::new());
    let _ = builder.clause_each(grammar, each);
    builder
        .census
        .unwrap_or_default()
        .into_iter()
        .map(|refusal| refusal.0)
        .collect()
}

/// The most passes a recursion loop runs before the batch is refused: the matcher's utility
/// depth limit, which no recursion over real code reaches with its limits lifted.
const MAX_LEVELS: usize = 256;

/// Runs `statements` (namespaced and substituted by `substitute`), each loop (`emit::LOOP` …
/// `emit::UNTIL`) again while its last query counts a change. A loop that still changes after
/// `MAX_LEVELS` passes refuses the batch: a recursion that deep is not evaluated. A loop's
/// statements run on every pass, so they are prepared once (`looping`).
fn execute(
    connection: &Connection,
    statements: &[String],
    substitute: &dyn Fn(&str) -> String,
    looping: bool,
) -> Result<(), SqlError> {
    let mut at = 0;
    while at < statements.len() {
        if let Some(reads) = statements[at].strip_prefix(emit::ACYCLIC) {
            acyclic(connection, &substitute(reads))?;
            at += 1;
            continue;
        }
        if statements[at] != emit::LOOP {
            let sql = substitute(&statements[at]);
            if looping {
                connection.prepare_cached(&sql)?.execute([])?;
            } else {
                connection.execute_batch(&sql)?;
            }
            at += 1;
            continue;
        }
        let mut depth = 0;
        let end = (at..statements.len())
            .find(|&index| {
                if statements[index] == emit::LOOP {
                    depth += 1;
                } else if statements[index].starts_with(emit::UNTIL) {
                    depth -= 1;
                }
                depth == 0
            })
            .expect("a loop's end");
        let until = substitute(&statements[end][emit::UNTIL.len()..]);
        let mut passes = 0;
        loop {
            execute(connection, &statements[at + 1..end], substitute, true)?;
            passes += 1;
            let changed: i64 = connection
                .prepare_cached(&until)?
                .query_row([], |row| row.get(0))?;
            if changed == 0 {
                break;
            }
            if passes > MAX_LEVELS {
                return Err(SqlError::unsupported("recursion deeper than 256 levels"));
            }
        }
        at = end + 1;
    }
    Ok(())
}

/// Accepts a recursion loop's last pass (`recursion`): its recursive reads (`query`'s rows,
/// `(unit, key, unit, key)`) must form no cycle, and no chain of them may be longer than the
/// matcher's utility depth limit.
fn acyclic(connection: &Connection, query: &str) -> Result<(), SqlError> {
    let mut nodes: HashMap<(i64, i64), usize> = HashMap::new();
    let mut edges: Vec<Vec<usize>> = Vec::new();
    let mut node = |key: (i64, i64), edges: &mut Vec<Vec<usize>>| -> usize {
        *nodes.entry(key).or_insert_with(|| {
            edges.push(Vec::new());
            edges.len() - 1
        })
    };
    let mut prepared = connection.prepare(query)?;
    let mut rows = prepared.query([])?;
    while let Some(row) = rows.next()? {
        let from = node((row.get(0)?, row.get(1)?), &mut edges);
        let to = node((row.get(2)?, row.get(3)?), &mut edges);
        edges[from].push(to);
    }
    // Iterative depth-first search: `depth` is a key's longest chain of reads, `None` while the
    // key is on the search path (reaching it again closes a cycle).
    let mut depth: Vec<Option<Option<usize>>> = vec![None; edges.len()];
    for start in 0..edges.len() {
        if depth[start].is_some() {
            continue;
        }
        depth[start] = Some(None);
        let mut path: Vec<(usize, usize)> = vec![(start, 0)];
        while let Some(&mut (key, ref mut next)) = path.last_mut() {
            if let Some(&to) = edges[key].get(*next) {
                *next += 1;
                match depth[to] {
                    Some(None) => {
                        return Err(SqlError::unsupported("recursion whose reads loop back"));
                    }
                    Some(Some(_)) => {}
                    None => {
                        depth[to] = Some(None);
                        path.push((to, 0));
                    }
                }
                continue;
            }
            let longest = edges[key]
                .iter()
                .filter_map(|&to| depth[to].flatten())
                .max()
                .map_or(0, |longest| longest + 1);
            if longest > MAX_LEVELS {
                return Err(SqlError::unsupported("recursion deeper than 256 levels"));
            }
            depth[key] = Some(Some(longest));
            path.pop();
        }
    }
    Ok(())
}

/// Reads the rows of a clause's final query: each `(file, root, captures…)`.
fn read_matches(
    connection: &Connection,
    query: &str,
    statement: &Statement,
    mut found: impl FnMut(usize, SqlMatch),
) -> Result<(), SqlError> {
    let mut prepared = connection.prepare(query)?;
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
                    (Some(lo), Some(hi)) => Some(Captured::List(Some((lo as u32, hi as u32)))),
                    _ => None,
                },
            };
            if let Some(captured) = captured {
                bound.push((name.clone(), captured));
            }
        }
        found(
            file as usize,
            SqlMatch {
                root: root as u32,
                captures: bound,
            },
        );
    }
    Ok(())
}

/// Drops the temporary tables whose names start with `prefix`.
fn drop_tables(connection: &Connection, prefix: &str) -> Result<(), SqlError> {
    let names: Vec<String> = connection
        .prepare(
            "SELECT name FROM sqlite_temp_master WHERE type = 'table' AND substr(name, 1, length(?1)) = ?1",
        )?
        .query_map([prefix], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for name in names {
        connection.execute_batch(&format!("DROP TABLE {name}"))?;
    }
    Ok(())
}

impl Compiled {
    /// The statements one pass of each recursion loop runs, outermost loops first: stages', then
    /// clauses'. A nested loop's statements count in its own entry, not in its parent's.
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn loop_statements(&self) -> Vec<usize> {
        let mut passes = Vec::new();
        let lists = self
            .builder
            .stages
            .iter()
            .map(|stage| &stage.statements)
            .chain(self.clauses.iter().map(|(_, sql)| sql));
        for statements in lists {
            // Per open loop: its statements so far.
            let mut open: Vec<usize> = Vec::new();
            for statement in statements {
                if statement == emit::LOOP {
                    open.push(0);
                } else if statement.starts_with(emit::UNTIL) {
                    passes.push(open.pop().expect("an open loop") + 1);
                } else if let Some(count) = open.last_mut() {
                    *count += 1;
                }
            }
        }
        passes
    }

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
    /// compiled grammars) that matches it, if any. Returns each file's matches, ordered by root.
    pub(in crate::code_rule_dsl) fn run(
        self,
        files: Vec<(TapeDoc, Option<usize>)>,
    ) -> Result<Vec<Vec<SqlMatch>>, SqlError> {
        let Compiled {
            builder,
            clauses,
            each,
        } = self;
        let assigned: Vec<Option<usize>> = files.iter().map(|(_, statement)| *statement).collect();
        let batch = Batch::new(
            files.into_iter().map(|(doc, _)| doc).collect(),
            builder.terminals,
        );
        let slot = Arc::new(BatchSlot::default());
        *slot.current.write() = Some(Arc::clone(&batch));
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
        let run = |statements: &[String],
                   namespace: &str,
                   files: &str,
                   each: &str|
         -> Result<(), SqlError> {
            execute(
                &connection,
                statements,
                &|statement| {
                    statement
                        .replace(emit::NAMESPACE, namespace)
                        .replace(emit::FILES, files)
                        .replace(emit::EACH, each)
                },
                false,
            )
        };
        for (index, stage) in builder.stages.iter().enumerate() {
            let table = emit::stage_table(index);
            let namespace = emit::stage_namespace(index);
            let files = file_list(&|_, file| file.dialect == stage.dialect);
            let (query, steps) = stage.statements.split_last().expect("a final select");
            run(steps, &namespace, &files, "")?;
            connection.execute_batch(&format!(
                "CREATE TEMP TABLE {table}(file INTEGER, node INTEGER, PRIMARY KEY (file, node)) WITHOUT ROWID; \
                 INSERT INTO {table}(file, node) {}",
                query
                    .replace(emit::NAMESPACE, &namespace)
                    .replace(emit::FILES, &files)
            ))?;
        }
        let mut results: Vec<Vec<SqlMatch>> = batch.files.iter().map(|_| Vec::new()).collect();
        for (index, (statement, sql)) in clauses.iter().enumerate() {
            let files = file_list(&|file, _| assigned[file] == Some(index));
            if files == "NULL" {
                continue;
            }
            let (query, steps) = sql.split_last().expect("a final select");
            if !each {
                let namespace = format!("c{index}_");
                run(steps, &namespace, &files, "")?;
                read_matches(
                    &connection,
                    &query.replace(emit::NAMESPACE, &namespace),
                    statement,
                    |file, found| results[file].push(found),
                )?;
                continue;
            }
            // `violation.each`: rounds over the whole batch. A round re-runs the clause on the
            // roots whose last match bound a fresh range of the capture, with every range
            // reported for the root excluded; each reports what it found, after the earlier
            // rounds' findings for the same root.
            let each = format!("e{index}_");
            connection.execute_batch(&format!(
                "CREATE TEMP TABLE {each}x(file INTEGER, root INTEGER, start INTEGER, \"end\" INTEGER, \
                 PRIMARY KEY (file, root, start, \"end\")) WITHOUT ROWID; \
                 CREATE TEMP TABLE {each}a(file INTEGER, root INTEGER, PRIMARY KEY (file, root)) WITHOUT ROWID; \
                 CREATE TEMP TABLE {each}r(round INTEGER); INSERT INTO {each}r VALUES (0)"
            ))?;
            let mut found: Vec<(usize, usize, SqlMatch)> = Vec::new();
            for round in 0.. {
                let namespace = format!("c{index}r{round}_");
                run(steps, &namespace, &files, &each)?;
                read_matches(
                    &connection,
                    &query
                        .replace(emit::NAMESPACE, &namespace)
                        .replace(emit::EACH, &each),
                    statement,
                    |file, matched| found.push((file, round, matched)),
                )?;
                drop_tables(&connection, &namespace)?;
                let active: i64 =
                    connection.query_row(&format!("SELECT count(*) FROM {each}a"), [], |row| {
                        row.get(0)
                    })?;
                if active == 0 {
                    break;
                }
                connection.execute_batch(&format!("UPDATE {each}r SET round = round + 1"))?;
            }
            found.sort_by_key(|(file, round, matched)| (*file, matched.root, *round));
            for (file, _, matched) in found {
                results[file].push(matched);
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
