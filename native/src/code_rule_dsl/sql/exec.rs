//! The batch driver: builds the `node` table state from a batch's tapes, computes the shared
//! stages, runs one statement per clause and returns each clause's `(file, root, bindings)` rows
//! for the native witness rendering.

use super::emit::{self, Effect, Emitted, Writes};
use super::frames::{self, FrameStore};
use super::lower::{ProgramBuilder, Stage};
use super::nodes::{NodeFacts, NodeTable};
use super::plan::{SlotKind, Statement, Unsupported};
use super::skip::{self, Counts, MODE, Mode, Outcome};
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
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, OnceLock};

/// One file of a batch.
pub(in crate::code_rule_dsl) struct BatchFile {
    pub(in crate::code_rule_dsl) root: AstGrep<TapeDoc>,
    pub(in crate::code_rule_dsl) dialect: GrammarDialect,
    pub(super) facts: NodeFacts,
    /// What the native evaluator knows of the file beyond its tape (`FileContext`).
    pub(super) context: FileContext,
}

/// A parsed literal's tree (`parse`, `formatArgs`): a document of the batch's own, numbered after
/// the batch's files in the order the terminals parsed them, which the `node` table and the
/// terminals read as they read a file.
pub(in crate::code_rule_dsl) struct SubFile {
    pub(in crate::code_rule_dsl) root: AstGrep<crate::code_rule_dsl::literal::SubDoc>,
    pub(super) facts: NodeFacts,
    /// The batch file the literal is in.
    pub(super) host: usize,
    /// An annotation's: the file's scope table its names are read in (`ParseTest::tree_at`).
    pub(super) table: Option<Arc<crate::code_rule_dsl::scope::ScopeTable>>,
}

/// The batch's parsed trees, each once by its tree (parsed trees are remembered per file).
#[derive(Default)]
struct SubFiles {
    files: Vec<Arc<SubFile>>,
    index: HashMap<usize, u32>,
}

/// What the native evaluator installs for a file besides its tree: its host path, its Python
/// module path and the scope tables it has cached for the file's revision (the workspace's),
/// which the input-set terminals' file scopes read (`FileScopeSlot::install`).
#[derive(Clone, Default)]
pub(in crate::code_rule_dsl) struct FileContext {
    pub(in crate::code_rule_dsl) path: Option<String>,
    pub(in crate::code_rule_dsl) module: Option<Arc<crate::code_rule_dsl::module_path::ModulePath>>,
    pub(in crate::code_rule_dsl) scopes: Vec<Arc<crate::code_rule_dsl::scope::ScopeTable>>,
    /// The workspace's (or stateless evaluation's) call-graph closure and the file's summary,
    /// which `calls` reads as `calls::FileCallsSlot` installs them for the file's evaluation.
    pub(in crate::code_rule_dsl) calls: Option<(
        Arc<crate::code_rule_dsl::calls::CallsView>,
        Arc<crate::code_rule_dsl::calls::summary::Summary>,
    )>,
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
    subfiles: parking_lot::Mutex<SubFiles>,
}

impl Batch {
    /// A batch of `docs`, each with its context, their node facts computed once each, for a
    /// program's `terminals`.
    #[cfg(test)]
    pub(super) fn new(docs: Vec<(TapeDoc, FileContext)>, terminals: Vec<Terminal>) -> Arc<Self> {
        Self::of(docs, Arc::new(Program { terminals }))
    }

    /// A batch of `docs`, each with its context, their node facts computed once each, for
    /// `program`.
    pub(super) fn of(docs: Vec<(TapeDoc, FileContext)>, program: Arc<Program>) -> Arc<Self> {
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
            fresh: program.terminals.iter().map(|_| OnceLock::new()).collect(),
            program,
            subfiles: parking_lot::Mutex::default(),
        })
    }

    /// How many documents the batch holds: its files, then the trees parsed so far.
    pub(super) fn documents(&self) -> usize {
        self.files.len() + self.subfiles.lock().files.len()
    }

    /// The parsed tree numbered `file` (after the batch's files).
    pub(super) fn subfile(&self, file: usize) -> Option<Arc<SubFile>> {
        let index = file.checked_sub(self.files.len())?;
        self.subfiles.lock().files.get(index).cloned()
    }

    /// The number of `tree`, a tree parsed from a node of file `host` (an annotation's, with the
    /// table its names are read in): a document of the batch from now on.
    pub(super) fn add_subfile(
        &self,
        host: usize,
        tree: &Arc<crate::code_rule_dsl::literal::SubTree>,
        table: Option<Arc<crate::code_rule_dsl::scope::ScopeTable>>,
    ) -> u32 {
        let key = Arc::as_ptr(tree) as usize;
        let mut subfiles = self.subfiles.lock();
        if let Some(&file) = subfiles.index.get(&key) {
            return file;
        }
        let root = AstGrep::doc(crate::code_rule_dsl::literal::SubDoc {
            tree: Arc::clone(tree),
        });
        let facts = NodeFacts::of_tree(&root);
        let file = (self.files.len() + subfiles.files.len()) as u32;
        subfiles.files.push(Arc::new(SubFile {
            root,
            facts,
            host,
            table,
        }));
        subfiles.index.insert(key, file);
        file
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
    /// An exported text and its source range.
    Text(String, std::ops::Range<usize>),
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
    program: Arc<Program>,
    stages: Vec<Stage>,
    clauses: Vec<ClauseSql>,
}

/// One clause's statements, and where each clause-scope capture is in the rows of the last.
struct ClauseSql {
    sql: Emitted,
    captures: Vec<(String, SlotKind, usize)>,
    /// Whether the clause enumerates `violation.each`: its statements run in rounds.
    each: bool,
}

impl ClauseSql {
    fn of(statement: &Statement, each: Option<&str>) -> Self {
        let mut column = 2;
        let captures = statement
            .slots
            .iter()
            .enumerate()
            .filter(|(_, slot)| slot.scope == 0)
            .map(|(id, slot)| {
                let at = column;
                column += slot.columns(id).len();
                (slot.name.clone(), slot.kind, at)
            })
            .collect();
        let sql = match each {
            Some(name) => {
                let slot = statement.slots.iter().position(|slot| {
                    slot.scope == 0 && slot.kind == SlotKind::Node && slot.name == name
                });
                emit::clause_each(statement, slot)
            }
            None => emit::clause(statement),
        };
        Self {
            sql,
            captures,
            each: each.is_some(),
        }
    }
}

/// Compiles `grammars`, one clause each; `Unsupported` where one holds an operator the engine
/// does not lower.
#[cfg(test)]
pub(in crate::code_rule_dsl) fn compile(
    grammars: &[&GrammarMatcher],
) -> Result<Compiled, SqlError> {
    compile_enumerating(grammars, None)
}

/// `compile`, with `violation.each` enumerating the capture `each` at every matching root.
pub(in crate::code_rule_dsl) fn compile_enumerating(
    grammars: &[&GrammarMatcher],
    each: Option<&str>,
) -> Result<Compiled, SqlError> {
    let mut builder = ProgramBuilder::default();
    let mut clauses = Vec::new();
    for grammar in grammars {
        let statement = builder
            .clause_each(grammar, each)
            .map_err(SqlError::Unsupported)?;
        clauses.push(ClauseSql::of(&statement, each));
    }
    Ok(Compiled::new(builder, clauses))
}

/// Compiles every one of `grammars` the engine lowers into one program, each with the capture its
/// `violation.each` enumerates. Per grammar: its clause index in the program, or why the engine
/// refuses it. A refused grammar leaves nothing in the program (no stage or terminal of it runs):
/// each is lowered alone first, and lowering is deterministic, so one that lowers alone lowers in
/// the shared program.
pub(in crate::code_rule_dsl) fn compile_each(
    grammars: &[(&GrammarMatcher, Option<&str>)],
) -> (Compiled, Vec<Result<usize, Unsupported>>) {
    let mut builder = ProgramBuilder::default();
    let mut clauses = Vec::new();
    let outcomes = grammars
        .iter()
        .map(|&(grammar, each)| {
            ProgramBuilder::default().clause_each(grammar, each)?;
            let statement = builder.clause_each(grammar, each)?;
            clauses.push(ClauseSql::of(&statement, each));
            Ok(clauses.len() - 1)
        })
        .collect();
    (Compiled::new(builder, clauses), outcomes)
}

/// Every operator the engine would refuse in `grammar`'s clause (`each`: its `violation.each`),
/// not only the first: each refusal is recorded and lowered as `Never`.
#[cfg(test)]
pub(in crate::code_rule_dsl) fn census(
    grammar: &GrammarMatcher,
    each: Option<&str>,
) -> Vec<String> {
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

/// One clause's work in a batch: its statement (a clause index of the program) and the batch files
/// it runs on, each with every root of the clause's kinds (`None`) or the roots an anchor index
/// selected (in preorder; none selected runs nothing).
pub(in crate::code_rule_dsl) struct ClauseRun {
    pub(in crate::code_rule_dsl) statement: usize,
    pub(in crate::code_rule_dsl) files: Vec<(usize, Option<Arc<[u32]>>)>,
}

impl Compiled {
    fn new(builder: ProgramBuilder, clauses: Vec<ClauseSql>) -> Self {
        let ProgramBuilder {
            terminals, stages, ..
        } = builder;
        Self {
            program: Arc::new(Program { terminals }),
            stages,
            clauses,
        }
    }

    /// The statements one pass of each recursion loop runs, outermost loops first: stages', then
    /// clauses'. A nested loop's statements count in its own entry, not in its parent's.
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn loop_statements(&self) -> Vec<usize> {
        let mut passes = Vec::new();
        let lists = self
            .stages
            .iter()
            .map(|stage| &stage.sql.statements)
            .chain(self.clauses.iter().map(|clause| &clause.sql.statements));
        for statements in lists {
            // Per open loop: its statements so far.
            let mut open: Vec<usize> = Vec::new();
            for statement in statements {
                if statement == emit::LOOP {
                    open.push(0);
                } else if statement.starts_with(emit::UNTIL) {
                    passes.push(open.pop().expect("an open loop") + 1);
                } else if statement.starts_with(emit::BLOCK)
                    || statement == emit::END
                    || statement.starts_with(emit::RUN)
                {
                    // A block's loop counts in its own entry, once, however many sites run it.
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
            self.stages
                .iter()
                .map(|stage| stage.sql.statements.len() + 1)
                .sum(),
            self.clauses
                .iter()
                .map(|clause| clause.sql.statements.len())
                .sum(),
        )
    }

    /// How many bytes of SQL text the program's stages and clauses hold.
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn statement_bytes(&self) -> usize {
        self.stages
            .iter()
            .flat_map(|stage| &stage.sql.statements)
            .chain(
                self.clauses
                    .iter()
                    .flat_map(|clause| &clause.sql.statements),
            )
            .map(String::len)
            .sum()
    }

    /// Every statement of the program's stages, then of its clauses.
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn statements(&self) -> impl Iterator<Item = &String> {
        self.stages
            .iter()
            .flat_map(|stage| &stage.sql.statements)
            .chain(
                self.clauses
                    .iter()
                    .flat_map(|clause| &clause.sql.statements),
            )
    }

    /// A connection set up to run this program's batches.
    pub(in crate::code_rule_dsl) fn session(&self) -> Result<Session, SqlError> {
        Session::new(self)
    }

    /// Runs one batch on a session of its own: `files` are its tapes, each with the statement (by
    /// index into the compiled grammars) that matches it, if any, and what the file's scopes read
    /// beyond its tape (`FileContext`). Returns each file's matches, ordered by root.
    pub(in crate::code_rule_dsl) fn run(
        &self,
        files: Vec<(TapeDoc, Option<usize>, FileContext)>,
    ) -> Result<Vec<Vec<SqlMatch>>, SqlError> {
        let mut runs: Vec<ClauseRun> = (0..self.clauses.len())
            .map(|statement| ClauseRun {
                statement,
                files: Vec::new(),
            })
            .collect();
        for (index, (_, statement, _)) in files.iter().enumerate() {
            if let Some(statement) = statement {
                runs[*statement].files.push((index, None));
            }
        }
        let mut results: Vec<Vec<SqlMatch>> = files.iter().map(|_| Vec::new()).collect();
        let docs = files
            .into_iter()
            .map(|(doc, _, context)| (doc, context))
            .collect();
        let found = self.session()?.run(docs, &runs)?;
        for (run, found) in runs.iter().zip(found) {
            for ((file, _), matches) in run.files.iter().zip(found) {
                results[*file] = matches;
            }
        }
        Ok(results)
    }
}

/// What a session's statements did: how many ran, the rows they wrote (or, a clause's last,
/// returned), and where the time went. `nanos` covers every stage and clause statement; of it,
/// `stage_nanos` the stages' and `select_nanos` the clauses' last statements (the rest is frames).
/// `input_nanos` is the batch's own rows (`runs`, `roots`) and emptying the tables; `facts_nanos`
/// the node facts of the batch's files.
///
/// `skipped` counts the statements the emptiness rules passed over (`skip`): one at a time, in a
/// stage or clause run whose root frame was empty (`skipped_runs` such runs: every statement
/// after its root frame, a loop's once), or in a recursion block's run with no new demand
/// (`skipped_blocks` such runs: each of the block's statements once). `skipped_clears`: the
/// tables left as they were because they were empty already.
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::code_rule_dsl) struct Stats {
    pub(in crate::code_rule_dsl) statements: usize,
    pub(in crate::code_rule_dsl) rows: usize,
    pub(in crate::code_rule_dsl) nanos: u64,
    pub(in crate::code_rule_dsl) stage_nanos: u64,
    pub(in crate::code_rule_dsl) select_nanos: u64,
    pub(in crate::code_rule_dsl) input_nanos: u64,
    pub(in crate::code_rule_dsl) facts_nanos: u64,
    pub(in crate::code_rule_dsl) skipped: usize,
    pub(in crate::code_rule_dsl) skipped_runs: usize,
    pub(in crate::code_rule_dsl) skipped_blocks: usize,
    pub(in crate::code_rule_dsl) skipped_clears: usize,
}

impl Stats {
    /// Adds what a session did from `before` to `after`.
    pub(in crate::code_rule_dsl) fn add_since(&mut self, after: &Self, before: &Self) {
        self.statements += after.statements - before.statements;
        self.rows += after.rows - before.rows;
        self.nanos += after.nanos - before.nanos;
        self.stage_nanos += after.stage_nanos - before.stage_nanos;
        self.select_nanos += after.select_nanos - before.select_nanos;
        self.input_nanos += after.input_nanos - before.input_nanos;
        self.facts_nanos += after.facts_nanos - before.facts_nanos;
        self.skipped += after.skipped - before.skipped;
        self.skipped_runs += after.skipped_runs - before.skipped_runs;
        self.skipped_blocks += after.skipped_blocks - before.skipped_blocks;
        self.skipped_clears += after.skipped_clears - before.skipped_clears;
    }
}

/// One statement as a session runs it, with its effect over the session's counted tables (a
/// marker's, `emit::LOOP`, `emit::RUN`…, is `Rule::AlwaysRun` and writes nothing).
struct Step {
    sql: String,
    effect: Effect<usize>,
}

impl Step {
    /// Empties `table` (the counted table `id`), where it holds rows: a frame through its
    /// buffers (`frames::CLEAR`), a B-tree by its `DELETE`.
    fn clear(id: usize, table: &str, frame: bool) -> Self {
        Self {
            sql: if frame {
                format!("{}{table}", frames::CLEAR)
            } else {
                format!("DELETE FROM {table}")
            },
            effect: Effect {
                rule: emit::Rule::EmptyIfAnyInputEmpty,
                inputs: vec![id],
                writes: Writes::Clear(id),
            },
        }
    }

    /// Runs the block of this index (`emit::RUN`).
    fn run(index: usize) -> Self {
        Self {
            sql: format!("{}{index}", emit::RUN),
            effect: Effect::default(),
        }
    }
}

/// A recursion component's loop (`emit::BLOCK`): each `emit::RUN` of it runs its steps again.
/// `demand`: its units' demand tables; `mark`: where the session keeps their rows as the block's
/// last run left them (`skip`); `size`: its statements, each once.
struct Block {
    steps: Vec<Step>,
    demand: Vec<usize>,
    mark: usize,
    size: usize,
}

/// A stage's or clause's root frame: the step that fills it, its counted table, and how many
/// statements follow it, the last included (each loop's once).
struct Root {
    at: usize,
    table: usize,
    rest: usize,
}

/// A clause's `violation.each` enumeration, whose tables live across the rounds of a batch: its
/// table prefix (`emit::EACH`), their clears, and the counted tables of the roots still
/// enumerated (`a`) and of the round (`r`).
struct Each {
    prefix: String,
    clear: Vec<Step>,
    active: usize,
    round: usize,
}

/// A stage's or clause's statements as a session runs them: their text is fixed when the session
/// starts, so every batch runs the same cached statements.
struct Prepared {
    /// The statements that fill its frames, in order.
    steps: Vec<Step>,
    /// The loops its steps run by index (`emit::RUN<index>`), each emitted once (`emit::BLOCK`).
    blocks: Vec<Block>,
    /// A clause's: selects its roots and bindings. A stage's: fills the stage table.
    last: Step,
    /// Empties each of its tables.
    clear: Vec<Step>,
    /// The stages its statements read.
    stages: Vec<usize>,
    /// A clause under `violation.each`: its enumeration.
    each: Option<Each>,
    /// Its root frame, where it has one (a recursive stage's loop starts from seeds instead).
    root: Option<Root>,
}

/// The counted tables of a session, by name: every table its statements create, each stage's
/// and each enumeration's; `frames`, by id, each one's name where it is a frame.
#[derive(Default)]
struct Tables {
    ids: HashMap<String, usize>,
    frames: Vec<Option<String>>,
}

impl Tables {
    fn add(&mut self, name: &str, frame: bool) -> usize {
        let next = self.ids.len();
        let id = *self.ids.entry(name.to_owned()).or_insert(next);
        if id == next {
            self.frames.push(frame.then(|| name.to_owned()));
        }
        id
    }

    fn id(&self, name: &str) -> usize {
        *self
            .ids
            .get(name)
            .unwrap_or_else(|| panic!("{name} is a counted table"))
    }
}

/// How many statements `steps` hold, each block's run counting the block's: the markers and the
/// loops' queries are not statements.
fn size(steps: &[Step], blocks: &[Block]) -> usize {
    steps
        .iter()
        .map(|step| match step.sql.strip_prefix(emit::RUN) {
            Some(index) => blocks[index.parse::<usize>().expect("a block index")].size,
            None => usize::from(!step.sql.starts_with("/*")),
        })
        .sum()
}

/// Moves each block of `steps` (`emit::BLOCK<id>` … `emit::END`, blocks nested in it too) into
/// `blocks`, leaving `emit::RUN<index>` where it was; every later `emit::RUN<id>` runs the block
/// `id` last named. Each block takes the next of `marks`.
fn extract_blocks(
    steps: &mut std::vec::IntoIter<Step>,
    blocks: &mut Vec<Block>,
    named: &mut HashMap<String, usize>,
    marks: &mut usize,
) -> Vec<Step> {
    let mut out = Vec::new();
    while let Some(step) = steps.next() {
        if let Some(id) = step.sql.strip_prefix(emit::BLOCK) {
            let id = id.to_owned();
            let index = blocks.len();
            blocks.push(Block {
                steps: Vec::new(),
                demand: step.effect.inputs,
                mark: *marks,
                size: 0,
            });
            *marks += 1;
            let inner = extract_blocks(steps, blocks, named, marks);
            blocks[index].size = size(&inner, blocks);
            blocks[index].steps = inner;
            named.insert(id, index);
            out.push(Step::run(index));
        } else if let Some(id) = step.sql.strip_prefix(emit::RUN) {
            let index = *named.get(id).expect("a block emitted before it runs");
            out.push(Step::run(index));
        } else if step.sql == emit::END {
            return out;
        } else {
            out.push(step);
        }
    }
    out
}

/// Creates the tables (and a recursion unit's indexes) of `emitted` now, each a counted table of
/// `tables`: a table its statements only append to and empty whole is a frame of `frames`
/// (`frames::rewrite`), any other a B-tree, never both (`btrees`). Fixes the text of the rest,
/// loops' markers included (`Runner::steps`), with their effects over `tables`. The statements
/// read their root frame's files and roots under the run id `run`; their tables are named with
/// `namespace`, an enumeration's with `each`.
#[allow(clippy::too_many_arguments)]
fn prepare(
    connection: &Connection,
    frames: &Arc<FrameStore>,
    btrees: &mut HashSet<String>,
    tables: &mut Tables,
    marks: &mut usize,
    emitted: &Emitted,
    namespace: &str,
    run: i64,
    each: &str,
) -> Result<Prepared, SqlError> {
    let files = format!("SELECT file FROM runs WHERE run = {run}");
    let roots = format!("SELECT file, node FROM roots WHERE run = {run}");
    let named = |table: &str| {
        table
            .replace(emit::NAMESPACE, namespace)
            .replace(emit::EACH, each)
    };
    let statements: Vec<String> = emitted
        .statements
        .iter()
        .map(|statement| {
            named(statement)
                .replace(emit::FILES, &files)
                .replace(emit::ROOTS, &roots)
        })
        .collect();
    // `rewrite` keeps every statement but the schema's in order (a frame's clear for its
    // `DELETE`): their effects, in that order.
    let schema = |sql: &str| sql.starts_with("CREATE ");
    let mut effects = statements
        .iter()
        .zip(&emitted.effects)
        .filter(|(sql, _)| !schema(sql))
        .map(|(_, effect)| effect)
        .collect::<Vec<_>>()
        .into_iter();
    let mut steps = Vec::new();
    let mut clear = Vec::new();
    let mut cleared = HashSet::new();
    let mut stages = Vec::new();
    for sql in frames::rewrite(statements) {
        stages_read(&sql, &mut stages);
        if sql.starts_with("CREATE INDEX IF NOT EXISTS ") {
            connection.execute_batch(&sql)?;
            continue;
        }
        if let Some(declaration) = sql.strip_prefix(frames::FRAME) {
            let name = frames.register(connection, declaration)?;
            if btrees.contains(name) {
                return Err(SqlError::Sql(format!(
                    "{name} is both a frame and a B-tree"
                )));
            }
            if cleared.insert(name.to_owned()) {
                clear.push(Step::clear(tables.add(name, true), name, true));
            }
            continue;
        }
        match sql.strip_prefix("CREATE TEMP TABLE ") {
            Some(table) => {
                connection.execute_batch(&sql)?;
                let table = table.strip_prefix("IF NOT EXISTS ").unwrap_or(table);
                let name = table.split(['(', ' ']).next().unwrap_or_default();
                if frames.contains(name) {
                    return Err(SqlError::Sql(format!(
                        "{name} is both a frame and a B-tree"
                    )));
                }
                btrees.insert(name.to_owned());
                if cleared.insert(name.to_owned()) {
                    clear.push(Step::clear(tables.add(name, false), name, false));
                }
            }
            None => steps.push((sql, effects.next().expect("an effect per statement"))),
        }
    }
    assert!(effects.next().is_none(), "a statement per effect");
    // Once every table is created: a block's marker names its units' demand tables, which the
    // loop creates after it.
    let steps: Vec<Step> = steps
        .into_iter()
        .map(|(sql, effect)| Step {
            sql,
            effect: effect.clone().map(|table| tables.id(&named(&table))),
        })
        .collect();
    let mut blocks = Vec::new();
    let mut steps = extract_blocks(
        &mut steps.into_iter(),
        &mut blocks,
        &mut HashMap::new(),
        marks,
    );
    let last = steps.pop().expect("a final select");
    let root = emitted.root.as_ref().map(|root| {
        let table = tables.id(&named(root));
        let at = steps
            .iter()
            .position(|step| step.effect.writes == Writes::Insert(table))
            .expect("the root frame's statement");
        Root {
            at,
            table,
            rest: size(&steps[at + 1..], &blocks) + 1,
        }
    });
    Ok(Prepared {
        steps,
        blocks,
        last,
        clear,
        stages,
        each: None,
        root,
    })
}

/// The stages `sql` reads: each `stage_<index>` it names.
fn stages_read(sql: &str, stages: &mut Vec<usize>) {
    let prefix = emit::stage_table(0);
    let prefix = prefix.trim_end_matches('0');
    for (at, _) in sql.match_indices(prefix) {
        let digits: String = sql[at + prefix.len()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if let Ok(stage) = digits.parse() {
            stages.push(stage);
        }
    }
    stages.sort_unstable();
    stages.dedup();
}

/// The run id a stage's or clause's root frame reads its files and roots under.
fn run_id(stage: Option<usize>, clause: usize) -> i64 {
    match stage {
        Some(stage) => -1 - stage as i64,
        None => clause as i64,
    }
}

const CLEAR_RUNS: &str = "DELETE FROM runs";
const CLEAR_ROOTS: &str = "DELETE FROM roots";
const INSERT_RUN: &str = "INSERT INTO runs(run, file) VALUES (?1, ?2)";
const INSERT_ROOT: &str = "INSERT OR IGNORE INTO roots(run, file, node) VALUES (?1, ?2, ?3)";

/// A connection set up for one compiled program: every frame and stage table is created once and
/// every statement's text fixed when the session starts, so a batch only empties and refills
/// tables through cached prepared statements, and never changes the schema. The files and roots a
/// statement's root frame reads are rows of `runs` and `roots`, under the statement's run id.
/// `frames`: the frame tables' buffers (`frames`); `counts`: every counted table's rows (`skip`).
pub(in crate::code_rule_dsl) struct Session {
    connection: Connection,
    slot: Arc<BatchSlot>,
    program: Arc<Program>,
    stages: Vec<(GrammarDialect, Prepared)>,
    clauses: Vec<(Prepared, Vec<CaptureColumn>)>,
    stats: Stats,
    frames: Arc<FrameStore>,
    counts: Counts,
}

impl Session {
    fn new(compiled: &Compiled) -> Result<Self, SqlError> {
        let slot = Arc::new(BatchSlot::default());
        let connection = connect(&slot)?;
        let frames = Arc::new(FrameStore::default());
        connection.execute_batch(
            "PRAGMA temp_store = MEMORY; \
             CREATE TEMP TABLE runs(run INTEGER NOT NULL, file INTEGER NOT NULL, \
             PRIMARY KEY (run, file)) WITHOUT ROWID; \
             CREATE TEMP TABLE roots(run INTEGER NOT NULL, file INTEGER NOT NULL, \
             node INTEGER NOT NULL, PRIMARY KEY (run, file, node)) WITHOUT ROWID;",
        )?;
        let mut tables = Tables::default();
        let mut btrees = HashSet::new();
        let mut marks = 0;
        let mut stages = Vec::with_capacity(compiled.stages.len());
        for (index, stage) in compiled.stages.iter().enumerate() {
            let table = emit::stage_table(index);
            // A three-valued stage keeps the nodes it is unknown at too, with `unk`; a two-valued
            // stage's are all 0.
            frames.register(
                &connection,
                &format!(
                    "{table}(file INTEGER, node INTEGER, unk INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (file, node))"
                ),
            )?;
            let id = tables.add(&table, true);
            let mut prepared = prepare(
                &connection,
                &frames,
                &mut btrees,
                &mut tables,
                &mut marks,
                &stage.sql,
                &emit::stage_namespace(index),
                run_id(Some(index), 0),
                "",
            )?;
            prepared.clear.push(Step::clear(id, &table, true));
            let columns = if stage.three {
                "file, node, unk"
            } else {
                "file, node"
            };
            prepared.last.sql = format!("INSERT INTO {table}({columns}) {}", prepared.last.sql);
            prepared.last.effect.writes = Writes::Insert(id);
            stages.push((stage.dialect, prepared));
        }
        let mut clauses = Vec::with_capacity(compiled.clauses.len());
        for (index, clause) in compiled.clauses.iter().enumerate() {
            let each = format!("e{index}_");
            if clause.each {
                connection.execute_batch(&format!(
                    "CREATE TEMP TABLE {each}x(file INTEGER, root INTEGER, start INTEGER, \"end\" INTEGER, \
                     PRIMARY KEY (file, root, start, \"end\")) WITHOUT ROWID; \
                     CREATE TEMP TABLE {each}a(file INTEGER, root INTEGER, PRIMARY KEY (file, root)) WITHOUT ROWID; \
                     CREATE TEMP TABLE {each}r(round INTEGER)"
                ))?;
            }
            let enumeration = clause.each.then(|| {
                let ids = ["x", "a", "r"].map(|table| tables.add(&format!("{each}{table}"), false));
                Each {
                    clear: ["x", "a", "r"]
                        .iter()
                        .zip(ids)
                        .map(|(table, id)| Step::clear(id, &format!("{each}{table}"), false))
                        .collect(),
                    active: ids[1],
                    round: ids[2],
                    prefix: each.clone(),
                }
            });
            let mut prepared = prepare(
                &connection,
                &frames,
                &mut btrees,
                &mut tables,
                &mut marks,
                &clause.sql,
                &format!("c{index}_"),
                run_id(None, index),
                &each,
            )?;
            prepared.each = enumeration;
            clauses.push((prepared, clause.captures.clone()));
        }
        // Every statement a batch can run stays prepared.
        let statements: usize = stages
            .iter()
            .map(|(_, prepared)| prepared)
            .chain(clauses.iter().map(|(prepared, _)| prepared))
            .map(|prepared| {
                prepared.steps.len()
                    + prepared
                        .blocks
                        .iter()
                        .map(|block| block.steps.len())
                        .sum::<usize>()
                    + prepared.clear.len()
                    + 8
            })
            .sum();
        connection.set_prepared_statement_cache_capacity(statements + 4);
        Ok(Self {
            connection,
            slot,
            program: Arc::clone(&compiled.program),
            stages,
            clauses,
            stats: Stats::default(),
            counts: Counts::new(Arc::clone(&frames), tables.frames, marks),
            frames,
        })
    }

    /// What the session's statements did so far.
    pub(in crate::code_rule_dsl) fn stats(&self) -> Stats {
        self.stats
    }

    /// The session's tables: how many are B-trees and how many frames (`frames`).
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn tables(&self) -> Result<(usize, usize), SqlError> {
        let btrees: i64 = self.connection.query_row(
            "SELECT count(*) FROM sqlite_temp_master WHERE type = 'table'",
            [],
            |row| row.get(0),
        )?;
        Ok((btrees as usize, self.frames.len()))
    }

    /// Every statement a batch can run (stages', then clauses'), with its `EXPLAIN QUERY PLAN`
    /// rows `(id, parent, detail)`, planned as the session prepares it: on its tables, empty.
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn plans(&self) -> Result<Vec<(String, QueryPlan)>, SqlError> {
        let mut plans = Vec::new();
        let prepared = self
            .stages
            .iter()
            .map(|(_, prepared)| prepared)
            .chain(self.clauses.iter().map(|(prepared, _)| prepared));
        for prepared in prepared {
            let blocks = prepared.blocks.iter().flat_map(|block| &block.steps);
            for step in prepared.steps.iter().chain(blocks).chain([&prepared.last]) {
                let sql = &step.sql;
                if sql == emit::LOOP || sql.starts_with(emit::RUN) || sql.starts_with(frames::CLEAR)
                {
                    continue;
                }
                let sql = sql
                    .strip_prefix(emit::UNTIL)
                    .or_else(|| sql.strip_prefix(emit::ACYCLIC))
                    .unwrap_or(sql);
                let mut statement = self
                    .connection
                    .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
                let rows = statement
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(3)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                plans.push((sql.to_owned(), rows));
            }
        }
        Ok(plans)
    }

    /// Runs one batch of `docs`, each with what its scopes read beyond its tape (`FileContext`):
    /// each of `runs` over its files (a statement in one run at most). Returns, per run and per
    /// file of it (in its order), the file's matches, ordered by root. A batch that fails leaves
    /// every table empty, so the session runs the next one afresh. The input-set terminals' file
    /// scopes live for the batch: built on a file's first key, kept across the batch's
    /// statements, and dropped when it ends.
    pub(in crate::code_rule_dsl) fn run(
        &mut self,
        docs: Vec<(TapeDoc, FileContext)>,
        runs: &[ClauseRun],
    ) -> Result<Vec<Vec<Vec<SqlMatch>>>, SqlError> {
        let started = std::time::Instant::now();
        let batch = Batch::of(docs, Arc::clone(&self.program));
        self.stats.facts_nanos += started.elapsed().as_nanos() as u64;
        *self.slot.current.write() = Some(Arc::clone(&batch));
        let scopes = BatchScopes::begin();
        let result = self.run_batch(&batch, runs);
        drop(scopes);
        *self.slot.current.write() = None;
        if result.is_err() {
            let clears = self
                .stages
                .iter()
                .map(|(_, prepared)| prepared)
                .chain(self.clauses.iter().map(|(prepared, _)| prepared))
                .flat_map(|prepared| {
                    let each = prepared.each.iter().flat_map(|each| &each.clear);
                    prepared.clear.iter().chain(each)
                })
                .map(|step| step.sql.as_str())
                .chain([CLEAR_RUNS, CLEAR_ROOTS]);
            for clear in clears {
                match clear.strip_prefix(frames::CLEAR) {
                    Some(name) => {
                        self.frames.clear(name);
                    }
                    None => {
                        self.connection.execute(clear, [])?;
                    }
                }
            }
            self.counts.reset();
        }
        self.frames.trim();
        result
    }

    fn run_batch(
        &mut self,
        batch: &Batch,
        runs: &[ClauseRun],
    ) -> Result<Vec<Vec<Vec<SqlMatch>>>, SqlError> {
        let Self {
            connection,
            stages,
            clauses,
            stats,
            frames,
            counts,
            ..
        } = self;
        let connection = &*connection;
        let mut runner = Runner {
            connection,
            frames,
            stats,
            counts,
        };
        let inputs = std::time::Instant::now();
        connection.prepare_cached(CLEAR_RUNS)?.execute([])?;
        connection.prepare_cached(CLEAR_ROOTS)?.execute([])?;
        // Each stage runs over the whole files of the clauses that read it, directly or not.
        let mut stage_files: Vec<Vec<usize>> = stages.iter().map(|_| Vec::new()).collect();
        {
            let mut insert_run = connection.prepare_cached(INSERT_RUN)?;
            let mut insert_root = connection.prepare_cached(INSERT_ROOT)?;
            for run in runs.iter().filter(|run| !run.files.is_empty()) {
                let id = run_id(None, run.statement);
                for (file, roots) in &run.files {
                    match roots {
                        None => {
                            insert_run.execute(rusqlite::params![id, *file as i64])?;
                        }
                        Some(roots) => {
                            for root in roots.iter() {
                                insert_root.execute(rusqlite::params![
                                    id,
                                    *file as i64,
                                    i64::from(*root)
                                ])?;
                            }
                        }
                    }
                }
                let mut reads = clauses[run.statement].0.stages.clone();
                while let Some(stage) = reads.pop() {
                    let files = &mut stage_files[stage];
                    let before = files.len();
                    files.extend(run.files.iter().map(|(file, _)| *file));
                    files.sort_unstable();
                    files.dedup();
                    // Files new to the stage are new to the stages it reads.
                    if files.len() > before {
                        reads.extend(stages[stage].1.stages.iter().copied());
                    }
                }
            }
            for (stage, files) in stage_files.iter().enumerate() {
                let id = run_id(Some(stage), 0);
                for &file in files {
                    if batch.files[file].dialect == stages[stage].0 {
                        insert_run.execute(rusqlite::params![id, file as i64])?;
                    }
                }
            }
        }
        runner.stats.input_nanos += inputs.elapsed().as_nanos() as u64;
        // Stages in index order: a stage only reads stages made before it.
        let started = std::time::Instant::now();
        for (stage, (_, prepared)) in stages.iter().enumerate() {
            if stage_files[stage].is_empty() {
                continue;
            }
            let ran = runner.prepared(prepared)?;
            if ran || *MODE == Mode::Check {
                runner.statement(&prepared.last, !ran)?;
            }
        }
        runner.stats.stage_nanos += started.elapsed().as_nanos() as u64;
        let mut results = Vec::with_capacity(runs.len());
        let mut position: Vec<usize> = vec![usize::MAX; batch.files.len()];
        for run in runs {
            let mut found: Vec<Vec<(usize, SqlMatch)>> =
                run.files.iter().map(|_| Vec::new()).collect();
            if !run.files.is_empty() {
                #[cfg(feature = "unbounded-work")]
                let clause_started = std::time::Instant::now();
                for (at, (file, _)) in run.files.iter().enumerate() {
                    position[*file] = at;
                }
                let (prepared, captures) = &clauses[run.statement];
                if let Some(each) = &prepared.each {
                    let rows = connection
                        .prepare_cached(&format!("INSERT INTO {}r VALUES (0)", each.prefix))?
                        .execute([])?;
                    runner.counts.wrote(&Writes::Insert(each.round), rows);
                }
                // `violation.each`: rounds over the whole batch. A round re-runs the clause on the
                // roots whose last match bound a fresh range of the capture, with every range
                // reported for the root excluded; each reports what it found, after the earlier
                // rounds' findings for the same root.
                for round in 0.. {
                    // A clause whose root frame was empty selects nothing (`Runner::prepared`,
                    // which counted its last as passed over).
                    let ran = runner.prepared(prepared)?;
                    let outcome = runner.outcome(&prepared.last, !ran);
                    let skipped = *MODE == Mode::Skip && (!ran || outcome == Outcome::Empty);
                    if skipped && ran {
                        runner.stats.skipped += 1;
                    }
                    if !skipped {
                        let started = std::time::Instant::now();
                        let mut statement = connection.prepare_cached(&prepared.last.sql)?;
                        let mut rows = statement.query([])?;
                        let mut count = 0;
                        while let Some(row) = rows.next()? {
                            count += 1;
                            let file: i64 = row.get(0)?;
                            let root: i64 = row.get(1)?;
                            found[position[file as usize]].push((
                                round,
                                SqlMatch {
                                    root: root as u32,
                                    captures: bindings(row, captures)?,
                                },
                            ));
                        }
                        drop(rows);
                        drop(statement);
                        let elapsed = started.elapsed().as_nanos() as u64;
                        let stats = &mut *runner.stats;
                        stats.statements += 1;
                        stats.rows += count;
                        stats.nanos += elapsed;
                        stats.select_nanos += elapsed;
                        #[cfg(feature = "unbounded-work")]
                        record(&prepared.last.sql, elapsed, count);
                        if *MODE == Mode::Check {
                            skip::check(&prepared.last.sql, outcome, count);
                        }
                    }
                    let started = std::time::Instant::now();
                    runner.clear(prepared)?;
                    runner.stats.input_nanos += started.elapsed().as_nanos() as u64;
                    let Some(each) = &prepared.each else {
                        break;
                    };
                    // The roots still enumerated: none ends the rounds.
                    let active = if *MODE == Mode::Skip {
                        runner.counts.rows(each.active)
                    } else {
                        let active: i64 = connection
                            .prepare_cached(&format!("SELECT count(*) FROM {}a", each.prefix))?
                            .query_row([], |row| row.get(0))?;
                        if *MODE == Mode::Check {
                            skip::check(
                                "the enumerated roots",
                                Outcome::Exactly(runner.counts.rows(each.active)),
                                active as usize,
                            );
                        }
                        active
                    };
                    if active == 0 {
                        break;
                    }
                    connection
                        .prepare_cached(&format!("UPDATE {}r SET round = round + 1", each.prefix))?
                        .execute([])?;
                }
                if let Some(each) = &prepared.each {
                    runner.clears(&each.clear)?;
                }
                #[cfg(feature = "unbounded-work")]
                CLAUSE_PROFILE.with_borrow_mut(|profile| {
                    if let Some(profile) = profile {
                        profile.push((run.statement, clause_started.elapsed().as_nanos() as u64));
                    }
                });
            }
            results.push(
                found
                    .into_iter()
                    .map(|mut matches| {
                        // A root's matches in round order, after the matches of earlier roots.
                        matches.sort_by_key(|(round, found)| (found.root, *round));
                        matches.into_iter().map(|(_, found)| found).collect()
                    })
                    .collect(),
            );
        }
        let started = std::time::Instant::now();
        for (stage, (_, prepared)) in stages.iter().enumerate() {
            if !stage_files[stage].is_empty() {
                runner.clear(prepared)?;
            }
        }
        runner.stats.input_nanos += started.elapsed().as_nanos() as u64;
        Ok(results)
    }
}

/// A session's connection, frames, statistics and counts while a batch runs its statements.
struct Runner<'s> {
    connection: &'s Connection,
    frames: &'s FrameStore,
    stats: &'s mut Stats,
    counts: &'s mut Counts,
}

impl Runner<'_> {
    /// Runs a stage's or clause's steps, from its root frame on (`skip`): where the root frame is
    /// empty for the batch, every statement after it reads nothing, and is passed over (checking,
    /// run and found to change nothing); returns whether the root frame had rows.
    fn prepared(&mut self, prepared: &Prepared) -> Result<bool, SqlError> {
        let Some(root) = &prepared.root else {
            self.steps(&prepared.steps, &prepared.blocks, false)?;
            return Ok(true);
        };
        self.steps(&prepared.steps[..=root.at], &prepared.blocks, false)?;
        let empty = *MODE != Mode::Run && self.counts.rows(root.table) == 0;
        if empty && *MODE == Mode::Skip {
            self.stats.skipped_runs += 1;
            self.stats.skipped += root.rest;
            return Ok(false);
        }
        self.steps(&prepared.steps[root.at + 1..], &prepared.blocks, empty)?;
        Ok(!empty)
    }

    /// Executes a stage's or clause's statements, each loop (`emit::LOOP` … `emit::UNTIL`) again
    /// while its last query counts a change, and accepting a loop's last pass by its reads
    /// (`emit::ACYCLIC`). A loop that still changes after `MAX_LEVELS` passes refuses the batch: a
    /// recursion that deep is not evaluated. `emit::RUN<index>` runs `blocks[index]` there, over
    /// the tables as they stand. `known_empty`: checking a run the skip would pass over, whose
    /// statements must change nothing (but a loop's clears of its frames from an earlier run).
    fn steps(
        &mut self,
        steps: &[Step],
        blocks: &[Block],
        known_empty: bool,
    ) -> Result<(), SqlError> {
        let mut at = 0;
        while at < steps.len() {
            let sql = &steps[at].sql;
            if let Some(reads) = sql.strip_prefix(emit::ACYCLIC) {
                acyclic(self.connection, reads)?;
                at += 1;
                continue;
            }
            if sql.starts_with(frames::CLEAR) {
                self.clear_step(&steps[at])?;
                at += 1;
                continue;
            }
            if let Some(index) = sql.strip_prefix(emit::RUN) {
                let index: usize = index.parse().expect("a block index");
                self.block(&blocks[index], blocks, known_empty)?;
                at += 1;
                continue;
            }
            if sql != emit::LOOP {
                self.statement(&steps[at], known_empty)?;
                at += 1;
                continue;
            }
            let mut depth = 0;
            let end = (at..steps.len())
                .find(|&index| {
                    if steps[index].sql == emit::LOOP {
                        depth += 1;
                    } else if steps[index].sql.starts_with(emit::UNTIL) {
                        depth -= 1;
                    }
                    depth == 0
                })
                .expect("a loop's end");
            let until = &steps[end].sql[emit::UNTIL.len()..];
            let mut passes = 0;
            loop {
                self.steps(&steps[at + 1..end], blocks, known_empty)?;
                passes += 1;
                let changed: i64 = self
                    .connection
                    .prepare_cached(until)?
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

    /// Runs a recursion block, unless its units hold no demand row its last run did not see: it
    /// ended with no key dirty, and only a new demand row is dirty outside it (`skip`).
    fn block(
        &mut self,
        block: &Block,
        blocks: &[Block],
        known_empty: bool,
    ) -> Result<(), SqlError> {
        let idle =
            *MODE != Mode::Run && self.counts.sum(&block.demand) == self.counts.marks[block.mark];
        if idle && *MODE == Mode::Skip {
            self.stats.skipped_blocks += 1;
            self.stats.skipped += block.size;
            return Ok(());
        }
        self.steps(&block.steps, blocks, known_empty || idle)?;
        self.counts.marks[block.mark] = self.counts.sum(&block.demand);
        Ok(())
    }

    /// What `step`'s rule says it will change (`skip`); `known_empty`, nothing, but a clear.
    fn outcome(&self, step: &Step, known_empty: bool) -> Outcome {
        match *MODE {
            Mode::Run => Outcome::Unknown,
            _ if known_empty && !matches!(step.effect.writes, Writes::Clear(_)) => Outcome::Empty,
            _ => skip::outcome(&step.effect, |table| self.counts.rows(table)),
        }
    }

    /// Executes one statement that writes rows, unless its rule proves it empty; counts what it
    /// changed.
    fn statement(&mut self, step: &Step, known_empty: bool) -> Result<(), SqlError> {
        let outcome = self.outcome(step, known_empty);
        if outcome == Outcome::Empty && *MODE == Mode::Skip {
            self.stats.skipped += 1;
            return Ok(());
        }
        let started = std::time::Instant::now();
        let rows = self.connection.prepare_cached(&step.sql)?.execute([])?;
        let nanos = started.elapsed().as_nanos() as u64;
        self.stats.statements += 1;
        self.stats.rows += rows;
        self.stats.nanos += nanos;
        #[cfg(any(test, feature = "unbounded-work"))]
        record(&step.sql, nanos, rows);
        if *MODE == Mode::Check {
            skip::check(&step.sql, outcome, rows);
        }
        self.counts.wrote(&step.effect.writes, rows);
        Ok(())
    }

    /// Empties the tables of `prepared` (its blocks' demand among them, so none has run).
    fn clear(&mut self, prepared: &Prepared) -> Result<(), SqlError> {
        self.clears(&prepared.clear)?;
        for block in &prepared.blocks {
            self.counts.marks[block.mark] = 0;
        }
        Ok(())
    }

    /// Empties the tables of `clears`, passing over those already empty.
    fn clears(&mut self, clears: &[Step]) -> Result<(), SqlError> {
        for clear in clears {
            self.clear_step(clear)?;
        }
        Ok(())
    }

    /// Empties one table, unless it is empty already: a frame through its buffers
    /// (`frames::CLEAR`, which runs no statement), a B-tree by its `DELETE`.
    fn clear_step(&mut self, clear: &Step) -> Result<(), SqlError> {
        let outcome = self.outcome(clear, false);
        if outcome == Outcome::Empty && *MODE == Mode::Skip {
            self.stats.skipped_clears += 1;
            return Ok(());
        }
        let rows = match clear.sql.strip_prefix(frames::CLEAR) {
            Some(name) => {
                let rows = self.frames.rows(name).expect("a registered frame");
                self.frames.clear(name);
                rows
            }
            None => self.connection.prepare_cached(&clear.sql)?.execute([])?,
        };
        if *MODE == Mode::Check {
            skip::check(&clear.sql, outcome, rows);
        }
        self.counts.wrote(&clear.effect.writes, rows);
        Ok(())
    }
}

#[cfg(any(test, feature = "unbounded-work"))]
thread_local! {
    /// Where set, this thread's frame and stage statements, by text: their time, rows and runs.
    /// The `unbounded-work` (benchmark) build adds the clauses' final selects.
    pub(super) static STATEMENT_PROFILE: std::cell::RefCell<Option<StatementProfile>> =
        const { std::cell::RefCell::new(None) };
}

/// Per statement text: its time, rows and runs.
#[cfg(any(test, feature = "unbounded-work"))]
pub(in crate::code_rule_dsl) type StatementProfile =
    std::collections::HashMap<String, (u64, usize, usize)>;

/// Adds a statement's run to this thread's `STATEMENT_PROFILE`, where set.
#[cfg(any(test, feature = "unbounded-work"))]
fn record(sql: &str, nanos: u64, rows: usize) {
    STATEMENT_PROFILE.with_borrow_mut(|profile| {
        if let Some(profile) = profile {
            // The texts repeat every batch: allocate a key only on a statement's first run.
            if let Some(entry) = profile.get_mut(sql) {
                entry.0 += nanos;
                entry.1 += rows;
                entry.2 += 1;
            } else {
                profile.insert(sql.to_owned(), (nanos, rows, 1));
            }
        }
    });
}

#[cfg(feature = "unbounded-work")]
thread_local! {
    /// Where set, this thread's clause runs: the program's clause and the time its statements took
    /// over a batch (every `violation.each` round and its tables' clears included).
    static CLAUSE_PROFILE: std::cell::RefCell<Option<Vec<(usize, u64)>>> =
        const { std::cell::RefCell::new(None) };
}

/// The benchmark build's profile of `body`'s batches on this thread: its statements by text
/// (`STATEMENT_PROFILE`) and its clause runs (`CLAUSE_PROFILE`). A profile set around the call is
/// kept, with this one's statements added.
#[cfg(feature = "unbounded-work")]
pub(in crate::code_rule_dsl) fn profiled<T>(
    body: impl FnOnce() -> T,
) -> (T, StatementProfile, Vec<(usize, u64)>) {
    let outer = STATEMENT_PROFILE.replace(Some(StatementProfile::default()));
    let outer_clauses = CLAUSE_PROFILE.replace(Some(Vec::new()));
    let result = body();
    let statements = STATEMENT_PROFILE.replace(outer).unwrap_or_default();
    let clauses = CLAUSE_PROFILE.replace(outer_clauses).unwrap_or_default();
    STATEMENT_PROFILE.with_borrow_mut(|profile| {
        if let Some(profile) = profile {
            for (sql, (nanos, rows, runs)) in &statements {
                let entry = profile.entry(sql.clone()).or_default();
                entry.0 += nanos;
                entry.1 += rows;
                entry.2 += runs;
            }
        }
    });
    (result, statements, clauses)
}

/// A statement's `EXPLAIN QUERY PLAN` rows: `(id, parent, detail)`.
#[cfg(test)]
pub(in crate::code_rule_dsl) type QueryPlan = Vec<(i64, i64, String)>;

/// A clause-scope capture's name, kind and first column in a clause's last statement.
type CaptureColumn = (String, SlotKind, usize);

/// The clause-scope bindings of one row of a clause's last statement.
fn bindings(
    row: &rusqlite::Row<'_>,
    captures: &[(String, SlotKind, usize)],
) -> rusqlite::Result<Vec<(String, Captured)>> {
    let mut bound = Vec::new();
    for (name, kind, at) in captures {
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
            SlotKind::Text => match (row.get::<_, Value>(*at)?, row.get::<_, Value>(at + 1)?) {
                (Value::Text(text), Value::Text(range)) => {
                    let (start, end) = range.split_once(':').unwrap_or_default();
                    match (start.parse(), end.parse()) {
                        (Ok(start), Ok(end)) => Some(Captured::Text(text, start..end)),
                        _ => None,
                    }
                }
                _ => None,
            },
        };
        if let Some(captured) = captured {
            bound.push((name.clone(), captured));
        }
    }
    Ok(bound)
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
            Captured::Text(text, range) => {
                crate::code_rule_dsl::matcher::export_text(&mut env, name, text, range.clone());
            }
        }
    }
    env
}
