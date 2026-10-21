//! The batch driver: builds the `node` table state from a batch's tapes, computes the shared
//! stages, runs one statement per clause and returns each clause's `(file, root, bindings)` rows
//! for the native witness rendering.

use super::emit::{self, Effect, Emitted, Writes};
use super::frames::{self, FrameStore};
use super::inputs::RootInputs;
use super::lower::{ProgramBuilder, Stage};
use super::nearest::NearestTable;
use super::nodes::{NodeFacts, NodeTable};
use super::plan::{SlotKind, Statement, Unsupported};
use super::shapes::{
    Block, Bound, Draft, DraftBlock, DraftEach, DraftStep, Instance, Kind, Root, ShapeCensus,
    ShapeMaker, Shapes, Step,
};
use super::skip::{self, Counts, MODE, Mode, Outcome};
use super::terminals::{
    BatchScopes, FactTable, PatternRow, PatternTable, PatternTerm, Terminal, TextTable,
    fresh_match, take_fact_time,
};
use super::timing::{Started, Timing, timed};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::GrammarMatcher;
use crate::code_rule_dsl::tape::TapeDoc;
use ast_grep_core::AstGrep;
use ast_grep_core::meta_var::{MetaVarEnv, MetaVariable};
use ast_grep_core::source::Content;
use ast_grep_core::{Doc, Node};
use foldhash::{HashMap, HashMapExt, HashSet, HashSetExt};
use parking_lot::RwLock;
use rusqlite::Connection;
use rusqlite::types::Value;
use rusqlite::vtab::Module;
use std::cmp::Reverse;
use std::collections::BTreeSet;
use std::sync::Arc;
#[path = "large/mod.rs"]
mod large;

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

/// A pattern terminal's fresh-env matches tested so far in a batch, by key (`file << 32 | node`):
/// the capture columns, `None` where the pattern does not match.
type FreshMatches = HashMap<i64, Option<Arc<[Option<i64>]>>>;
type PatternInputs = (usize, u32, u32, Box<[Option<i64>]>);
#[derive(Default)]
pub(super) struct BoundPatterns {
    pub(super) values: HashMap<PatternInputs, Option<Arc<[Option<i64>]>>>,
    pub(super) bytes: usize,
}

/// A file to run: its tape, the statement (by index into the compiled grammars) that matches it,
/// if any, with its selected roots, and what its scopes read beyond the tape.
pub(in crate::code_rule_dsl) type RunFile =
    (TapeDoc, Option<(usize, Option<Arc<[u32]>>)>, FileContext);

/// A batch: its files' tapes and node facts, its program, and the fresh-env matches of each
/// pattern terminal at the nodes tested so far.
pub(super) struct Batch {
    pub(super) files: Vec<BatchFile>,
    pub(super) program: Arc<Program>,
    fresh: Vec<parking_lot::Mutex<FreshMatches>>,
    pub(super) bound_patterns: parking_lot::Mutex<BoundPatterns>,
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
            fresh: program
                .terminals
                .iter()
                .map(|_| Default::default())
                .collect(),
            program,
            bound_patterns: Default::default(),
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

    /// The fresh-env matches of pattern terminal `index` at `keys` (`file << 32 | node`), each
    /// (term, node) matched once per batch whichever statements ask for it.
    pub(super) fn fresh(&self, index: usize, term: &PatternTerm, keys: &[i64]) -> Vec<PatternRow> {
        // Mutant 17: the tested (term, node) pairs are keyed by node only (one map for every
        // pattern terminal), so a node tested for one term answers for the others.
        let mut tested = self.fresh[if super::mutant(17) { 0 } else { index }].lock();
        keys.iter()
            .filter_map(|&key| {
                let (file, node) = ((key >> 32) as u32, key as u32);
                let values = tested
                    .entry(key)
                    .or_insert_with(|| fresh_match(self, term, file as usize, node))
                    .clone()?;
                Some(PatternRow::fresh(file, node, values))
            })
            .collect()
    }
}

/// Where a connection's virtual tables find the batch being evaluated.
#[derive(Default)]
pub(in crate::code_rule_dsl) struct BatchSlot {
    current: RwLock<Option<Arc<Batch>>>,
    pub(super) frames: Arc<FrameStore>,
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
const NEAREST_MODULE: Module<'static, NearestTable> = Module::eponymous_only_module();

/// An in-memory connection whose virtual tables read `slot`'s batch.
fn connect(slot: &Arc<BatchSlot>) -> Result<Connection, SqlError> {
    let connection = Connection::open_in_memory()?;
    #[cfg(all(test, rerail_scanstatus))]
    // SAFETY: SQLite owns this live connection; the documented output argument is nullable.
    // Profiling experiments enable counters explicitly; baseline sessions do not pay for them.
    unsafe {
        rusqlite::ffi::sqlite3_db_config(
            connection.handle(),
            rusqlite::ffi::SQLITE_DBCONFIG_STMT_SCANSTATUS,
            0i32,
            std::ptr::null_mut::<i32>(),
        );
    }

    connection.create_module(c"node", &NODE_MODULE, Some(Arc::clone(slot)))?;
    connection.create_module(c"pattern", &PATTERN_MODULE, Some(Arc::clone(slot)))?;
    connection.create_module(c"text_match", &TEXT_MODULE, Some(Arc::clone(slot)))?;
    connection.create_module(c"fact", &FACT_MODULE, Some(Arc::clone(slot)))?;
    connection.create_module(c"nearest", &NEAREST_MODULE, Some(Arc::clone(slot)))?;
    super::fused::register(&connection, slot)?;
    Ok(connection)
}

/// Policy-composition connections participate in the same job trace as AST sessions.
/// Declare the observer first so it drops before the connection on every exit.
pub(super) struct AuxiliaryConnection {
    #[cfg(any(test, feature = "unbounded-work"))]
    profiling: Option<Box<super::profiling::Profiler>>,
    connection: Connection,
}

impl AuxiliaryConnection {
    pub fn new(source: &'static str) -> rusqlite::Result<Self> {
        let connection = Connection::open_in_memory()?;
        #[cfg(any(test, feature = "unbounded-work"))]
        // SAFETY: the observer drops before this connection, including constructor errors.
        let profiling = unsafe { super::profiling::Profiler::configured_for(&connection, source) }?;
        #[cfg(not(any(test, feature = "unbounded-work")))]
        let _ = source;
        Ok(Self {
            #[cfg(any(test, feature = "unbounded-work"))]
            profiling,
            connection,
        })
    }

    pub fn checkpoint(&self) -> rusqlite::Result<()> {
        #[cfg(any(test, feature = "unbounded-work"))]
        if let Some(profiling) = &self.profiling {
            profiling.checkpoint(&self.connection)?;
        }
        Ok(())
    }
}

impl std::ops::Deref for AuxiliaryConnection {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        &self.connection
    }
}

/// Whole-call compiler experiments reuse the real terminal modules without entering the
/// statement scheduler. The callback owns the only connection and runs on one thread.
#[cfg(test)]
pub(super) fn with_terminals<T>(
    docs: Vec<(TapeDoc, FileContext)>,
    terminals: Vec<Terminal>,
    body: impl FnOnce(&Connection, &Arc<Batch>, &Arc<FrameStore>) -> T,
) -> T {
    let batch = Batch::new(docs, terminals);
    let slot = Arc::new(BatchSlot::default());
    *slot.current.write() = Some(Arc::clone(&batch));
    let connection = connect(&slot).expect("connects");
    let _scopes = BatchScopes::begin();
    struct FactTiming;
    impl Drop for FactTiming {
        fn drop(&mut self) {
            let _ = take_fact_time();
        }
    }
    let _timing = FactTiming;
    let _ = take_fact_time();
    body(&connection, &batch, &slot.frames)
}

/// Reuse one connection and compiled terminals across immutable whole-call snapshots.
#[cfg(test)]
pub(super) fn with_call_batches(
    batches: Vec<Vec<(TapeDoc, FileContext)>>,
    terminals: Vec<Terminal>,
    mut body: impl FnMut(&Connection, &Arc<Batch>, &Arc<FrameStore>),
) {
    let slot = Arc::new(BatchSlot::default());
    let connection = connect(&slot).expect("connects");
    let program = Arc::new(Program { terminals });
    for docs in batches {
        let batch = Batch::of(docs, Arc::clone(&program));
        *slot.current.write() = Some(Arc::clone(&batch));
        let scopes = BatchScopes::begin();
        let _ = take_fact_time();
        body(&connection, &batch, &slot.frames);
        let _ = take_fact_time();
        drop(scopes);
        *slot.current.write() = None;
    }
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
        let message = error.to_string();
        if message.contains("recursion whose reads loop back") {
            Self::unsupported("recursion whose reads loop back")
        } else if message.contains("rerail: recursion deeper than 256 levels") {
            Self::unsupported("recursion deeper than 256 levels")
        } else {
            Self::Sql(message)
        }
    }
}

/// Clauses compiled for the SQL engine: the terminals and stages they share, and their statements
/// as sessions run them (`Plan`, or why none can). A test program `compile_enumerating` compiled
/// keeps its emitted statements too, for the tests' censuses.
pub(in crate::code_rule_dsl) struct Compiled {
    program: Arc<Program>,
    plan: Result<Arc<Plan>, String>,
    #[cfg(test)]
    emitted: Option<(Vec<Stage>, Vec<ClauseSql>)>,
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
    let mut planner = Planner::default();
    let mut clauses = Vec::new();
    for grammar in grammars {
        let statement = builder
            .clause_each(grammar, each)
            .map_err(SqlError::Unsupported)?;
        let mut clause = ClauseSql::of(&statement, each);
        planner.clause(
            &mut clause,
            cfg!(test),
            &statement,
            &builder.stages,
            &builder.terminals,
        );
        clauses.push(clause);
    }
    Ok(Compiled::new(
        builder,
        planner,
        Some(clauses),
        super::DEFAULT_BATCH_SIZE,
    ))
}

/// Compiles every one of `grammars` the engine lowers into one program (`ProgramCompiler`).
#[cfg(test)]
pub(in crate::code_rule_dsl) fn compile_each(
    grammars: &[(&GrammarMatcher, Option<&str>)],
) -> (Compiled, Vec<Result<usize, Unsupported>>) {
    let mut compiler = ProgramCompiler::default();
    let outcomes = grammars
        .iter()
        .map(|&(grammar, each)| compiler.clause(grammar, each))
        .collect();
    (compiler.finish().0, outcomes)
}

/// Where compiling a program's clauses went: lowering, emitting their statements (a stage's too,
/// which lowering emits as it reaches the stage), planning each clause's statements (frames,
/// registers, effects), and planning the stages once every clause is.
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::code_rule_dsl) struct CompilePhases {
    pub(in crate::code_rule_dsl) lower: Timing,
    pub(in crate::code_rule_dsl) emit: Timing,
    pub(in crate::code_rule_dsl) plan: Timing,
    pub(in crate::code_rule_dsl) finish: Timing,
}

/// A program compiled one clause at a time, as its rules are registered: each clause is lowered
/// into the shared program, emitted and planned, then its emitted statements are freed.
#[derive(Default)]
pub(in crate::code_rule_dsl) struct ProgramCompiler {
    builder: ProgramBuilder,
    planner: Planner,
    clauses: usize,
    phases: CompilePhases,
}

impl ProgramCompiler {
    /// Compiles `grammar`'s clause, with the capture its `violation.each` enumerates: its clause
    /// index in the program, or why the engine refuses it. A refused clause leaves nothing in the
    /// program (no stage or terminal of it runs).
    pub(in crate::code_rule_dsl) fn clause(
        &mut self,
        grammar: &GrammarMatcher,
        each: Option<&str>,
    ) -> Result<usize, Unsupported> {
        let emitted_before = self.builder.stage_emission;
        let mut lower = Timing::default();
        let lowered = timed(&mut lower, || self.builder.clause_or_nothing(grammar, each));
        // The stages it reached were emitted as they were lowered.
        let stages = self.builder.stage_emission.since(emitted_before);
        self.phases.lower.add(lower.since(stages));
        self.phases.emit.add(stages);
        let statement = lowered?;
        let mut clause = timed(&mut self.phases.emit, || ClauseSql::of(&statement, each));
        timed(&mut self.phases.plan, || {
            self.planner.clause(
                &mut clause,
                false,
                &statement,
                &self.builder.stages,
                &self.builder.terminals,
            );
        });
        self.clauses += 1;
        Ok(self.clauses - 1)
    }

    /// The program of every clause compiled, and where its compile went.
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn finish(self) -> (Compiled, CompilePhases) {
        self.finish_with_batch(super::DEFAULT_BATCH_SIZE)
    }

    pub(in crate::code_rule_dsl) fn finish_with_batch(
        self,
        batch_size: usize,
    ) -> (Compiled, CompilePhases) {
        let Self {
            builder,
            planner,
            mut phases,
            ..
        } = self;
        let compiled = timed(&mut phases.finish, || {
            Compiled::new(builder, planner, None, batch_size)
        });
        (compiled, phases)
    }
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

/// Accepts a statement's recursion (`recursion`): `query`'s rows are the recursive reads its
/// units' last passes and its own rows made from their needed rows (`emit::lineage`), `(unit,
/// key, unit, key, inner)`, the statement's own from unit `-1`'s key `0`, `inner` where both keys
/// are of one component. Over the keys those reads reach from the statement's own, the inner
/// reads must form no cycle, and no chain of them may be longer than the matcher's utility depth
/// limit. A key they do not reach is read by no row the matcher evaluates: its answer decides
/// nothing, so it is not checked.
fn acyclic(connection: &Connection, query: &str) -> Result<(), SqlError> {
    let mut prepared = connection.prepare(query)?;
    let rows = prepared
        .query_map([], |r| {
            Ok([r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?])
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    acyclic_rows(rows)
}

fn acyclic_rows(rows: Vec<[i64; 5]>) -> Result<(), SqlError> {
    let mut nodes: HashMap<(i64, i64), usize> = HashMap::new();
    // Every read, and the inner reads alone.
    let mut reads: Vec<Vec<usize>> = Vec::new();
    let mut edges: Vec<Vec<usize>> = Vec::new();
    let mut node = |key: (i64, i64), reads: &mut Vec<Vec<usize>>, edges: &mut Vec<Vec<usize>>| {
        *nodes.entry(key).or_insert_with(|| {
            reads.push(Vec::new());
            edges.push(Vec::new());
            edges.len() - 1
        })
    };
    let start = node((-1, 0), &mut reads, &mut edges);
    for row in rows {
        let from = node((row[0], row[1]), &mut reads, &mut edges);
        let to = node((row[2], row[3]), &mut reads, &mut edges);
        reads[from].push(to);
        if row[4] != 0 {
            edges[from].push(to);
        }
    }
    let mut reached = vec![false; edges.len()];
    reached[start] = true;
    let mut frontier = vec![start];
    while let Some(key) = frontier.pop() {
        for &to in &reads[key] {
            if !reached[to] {
                reached[to] = true;
                frontier.push(to);
            }
        }
    }
    // Iterative depth-first search over the inner reads of the reached keys: `depth` is a key's
    // longest chain of reads, `None` while the key is on the search path (reaching it again
    // closes a cycle).
    let mut depth: Vec<Option<Option<usize>>> = vec![None; edges.len()];
    for start in 0..edges.len() {
        if depth[start].is_some() || !reached[start] {
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
    /// The program of `builder`'s terminals and stages and of the clauses `planner` planned.
    /// `clauses`: in the tests, a program `compile_enumerating` compiled keeps its emitted
    /// statements for the tests' censuses (`emitted`); a session runs the plan's.
    fn new(
        builder: ProgramBuilder,
        planner: Planner,
        clauses: Option<Vec<ClauseSql>>,
        batch_size: usize,
    ) -> Self {
        let ProgramBuilder {
            terminals,
            mut stages,
            ..
        } = builder;
        let keep = cfg!(test) && clauses.is_some();
        let plan = planner.finish(&mut stages, keep, batch_size).map(Arc::new);
        Self {
            program: Arc::new(Program { terminals }),
            plan,
            #[cfg(test)]
            emitted: clauses.map(|clauses| (stages, clauses)),
        }
    }

    /// The emitted statements of a program `compile_enumerating` compiled.
    #[cfg(test)]
    fn emitted(&self) -> (&[Stage], &[ClauseSql]) {
        let (stages, clauses) = self
            .emitted
            .as_ref()
            .expect("a program compile_enumerating compiled");
        (stages, clauses)
    }

    /// The statements one pass of each recursion loop runs, outermost loops first: stages', then
    /// clauses'. A nested loop's statements count in its own entry, not in its parent's.
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn loop_statements(&self) -> Vec<usize> {
        let mut passes = Vec::new();
        let (stages, clauses) = self.emitted();
        let lists = stages
            .iter()
            .map(|stage| &stage.sql.statements)
            .chain(clauses.iter().map(|clause| &clause.sql.statements));
        for statements in lists {
            // Per open loop: its statements so far.
            let mut open: Vec<usize> = Vec::new();
            for statement in statements {
                if emit::loop_start(statement) {
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
        let (stages, clauses) = self.emitted();
        (
            stages
                .iter()
                .map(|stage| stage.sql.statements.len() + 1)
                .sum(),
            clauses
                .iter()
                .map(|clause| clause.sql.statements.len())
                .sum(),
        )
    }

    /// Every statement of the program's stages, then of its clauses.
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn statements(&self) -> impl Iterator<Item = &String> {
        let (stages, clauses) = self.emitted();
        stages
            .iter()
            .flat_map(|stage| &stage.sql.statements)
            .chain(clauses.iter().flat_map(|clause| &clause.sql.statements))
    }

    /// Every step of the program's plan as a session runs it (each block's once): stages', then
    /// clauses'.
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn planned_statements(&self) -> Vec<String> {
        let Ok(plan) = &self.plan else {
            return Vec::new();
        };
        let mut statements = Vec::new();
        for index in 0..plan.stages.len() + plan.clauses.len() {
            let bound = plan.prepared(index);
            let blocks = bound.shape.blocks.iter().flat_map(|block| &block.steps);
            for step in bound
                .shape
                .steps
                .iter()
                .chain(blocks)
                .chain([&bound.shape.last])
            {
                let mut text = String::new();
                bound.text(step, &mut text);
                statements.push(text);
            }
        }
        statements
    }

    /// How many stages and clauses the plan holds, of how many shapes, and how many of them
    /// binding checked against their drafts (`shapes::verify`).
    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn shared(&self) -> (usize, usize, usize) {
        self.plan.as_ref().map_or((0, 0, 0), |plan| {
            (
                plan.stages.len() + plan.clauses.len(),
                plan.census.shapes,
                plan.verified,
            )
        })
    }

    /// A connection set up to run this program's batches.
    pub(in crate::code_rule_dsl) fn session(&self) -> Result<Session, SqlError> {
        Session::new(self)
    }

    /// The program as its sessions run it.
    fn plan(&self) -> Result<Arc<Plan>, SqlError> {
        self.plan.clone().map_err(SqlError::Sql)
    }

    /// The selected relational component, including its admitted clauses and physical costs.
    pub(in crate::code_rule_dsl) fn relational_plan(&self) -> serde_json::Value {
        let Ok(plan) = &self.plan else {
            return serde_json::Value::Null;
        };
        let mut metadata = plan
            .fused
            .as_ref()
            .map_or_else(|| serde_json::json!({}), |f| f.plan.metadata.clone());
        let mut clauses = plan
            .fused
            .as_ref()
            .map_or_else(Vec::new, |f| f.clauses.clone());
        metadata["acyclicClauses"] = serde_json::json!(clauses);
        if let Some(program) = &plan.large {
            clauses.extend(&program.clauses);
            clauses.sort_unstable();
            metadata["cteClauses"] = serde_json::json!(program.clauses);
            metadata["ctePrograms"] = serde_json::json!(program.queries.len());
            metadata["cteProgramBytes"] =
                serde_json::json!(program.queries.iter().map(String::len).sum::<usize>());
            metadata["cteLoweringReasons"] = metadata["scheduledClauses"].take();
            metadata["scheduledClauses"] = serde_json::json!([]);
        }
        metadata["clauses"] = serde_json::json!(clauses);
        metadata["filePartitioning"] = serde_json::json!("one complete-job partition per worker");
        metadata["maxMatchingStatementsPerPartition"] = serde_json::json!(18);
        metadata["sqlStatements"] = serde_json::json!(
            plan.large.as_ref().map_or(0, |p| p.queries.len())
                + plan.fused.as_ref().map_or(0, |f| f.plan.queries.len())
        );
        metadata
    }

    /// What the program's plan holds; nothing where it has none.
    pub(in crate::code_rule_dsl) fn census(&self) -> Census {
        self.plan
            .as_ref()
            .map(|plan| plan.census())
            .unwrap_or_default()
    }

    /// Runs `files` as batches of `batch` files each, in order, on one session of its own: `files`
    /// are the tapes, each with the statement (by index into the compiled grammars) that matches
    /// it, if any, and its roots (every root of the statement's kinds, or those an anchor index
    /// selected), and what the file's scopes read beyond its tape (`FileContext`). Returns each
    /// file's matches, ordered by root.
    pub(in crate::code_rule_dsl) fn run_batches(
        &self,
        files: Vec<RunFile>,
        batch: usize,
    ) -> Result<Vec<Vec<SqlMatch>>, SqlError> {
        let mut session = self.session()?;
        let mut results: Vec<Vec<SqlMatch>> = Vec::with_capacity(files.len());
        let mut files = files.into_iter().peekable();
        while files.peek().is_some() {
            let chunk: Vec<_> = files.by_ref().take(batch.max(1)).collect();
            let mut runs: Vec<ClauseRun> = (0..session.plan.clauses.len())
                .map(|statement| ClauseRun {
                    statement,
                    files: Vec::new(),
                })
                .collect();
            for (index, (_, statement, _)) in chunk.iter().enumerate() {
                if let Some((statement, roots)) = statement {
                    runs[*statement].files.push((index, roots.clone()));
                }
            }
            let mut found_files: Vec<Vec<SqlMatch>> = chunk.iter().map(|_| Vec::new()).collect();
            let docs = chunk
                .into_iter()
                .map(|(doc, _, context)| (doc, context))
                .collect();
            let found = session.run(docs, &runs)?;
            for (run, found) in runs.iter().zip(found) {
                for ((file, _), matches) in run.files.iter().zip(found) {
                    found_files[*file] = matches;
                }
            }
            results.extend(found_files);
        }
        Ok(results)
    }
}

/// What a session's statements did: how many ran, the rows they wrote (or, a clause's last,
/// returned), and where the time went, each timer in wall and thread CPU time (`Timing`). `time`
/// covers every stage and clause statement; of it, `stages` the stages' and `select` the clauses'
/// last statements (the rest is frames), and `facts` the fact terminals' native evaluation
/// (`terminals::fact`) inside them. `input` is the batch's own rows (`runs`, `roots`) and
/// emptying the tables; `node_facts` the node facts of the batch's files (`Batch::of`).
///
/// `skipped` counts the statements the emptiness rules passed over (`skip`): one at a time, in a
/// stage or clause run whose root frame was empty (`skipped_runs` such runs: every statement
/// after its root frame, a loop's once), or in a recursion block's run with no new demand
/// (`skipped_blocks` such runs: each of the block's statements once). `skipped_clears`: the
/// tables left as they were because they were empty already.
///
/// `sessions` counts the sessions made and `setup` the time their setup took: making the
/// connection, then each stage's and clause's tables on its first run (`Session::activate`).
/// `texts` is the statement texts the sessions' caches were sized for (each a distinct text of a
/// stage or clause that ran), `tables` the tables they made.
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::code_rule_dsl) struct Stats {
    pub(in crate::code_rule_dsl) statements: usize,
    pub(in crate::code_rule_dsl) rows: usize,
    pub(in crate::code_rule_dsl) time: Timing,
    pub(in crate::code_rule_dsl) stages: Timing,
    pub(in crate::code_rule_dsl) select: Timing,
    pub(in crate::code_rule_dsl) input: Timing,
    pub(in crate::code_rule_dsl) node_facts: Timing,
    pub(in crate::code_rule_dsl) facts: Timing,
    pub(in crate::code_rule_dsl) skipped: usize,
    pub(in crate::code_rule_dsl) skipped_runs: usize,
    pub(in crate::code_rule_dsl) skipped_blocks: usize,
    pub(in crate::code_rule_dsl) skipped_clears: usize,
    pub(in crate::code_rule_dsl) sessions: usize,
    pub(in crate::code_rule_dsl) setup: Timing,
    pub(in crate::code_rule_dsl) texts: usize,
    pub(in crate::code_rule_dsl) tables: usize,
}

impl Stats {
    /// Adds what a session did from `before` to `after`.
    pub(in crate::code_rule_dsl) fn add_since(&mut self, after: &Self, before: &Self) {
        self.statements += after.statements - before.statements;
        self.rows += after.rows - before.rows;
        self.time.add(after.time.since(before.time));
        self.stages.add(after.stages.since(before.stages));
        self.select.add(after.select.since(before.select));
        self.input.add(after.input.since(before.input));
        self.node_facts
            .add(after.node_facts.since(before.node_facts));
        self.facts.add(after.facts.since(before.facts));
        self.skipped += after.skipped - before.skipped;
        self.skipped_runs += after.skipped_runs - before.skipped_runs;
        self.skipped_blocks += after.skipped_blocks - before.skipped_blocks;
        self.skipped_clears += after.skipped_clears - before.skipped_clears;
        self.sessions += after.sessions - before.sessions;
        self.setup.add(after.setup.since(before.setup));
        self.texts += after.texts - before.texts;
        self.tables += after.tables - before.tables;
    }
}

/// Empties `table` (the counted table `id`), where it holds rows: a frame through its buffers
/// (`frames::CLEAR`), a B-tree by its `DELETE`.
fn clear_step(id: usize, table: &str, frame: bool) -> DraftStep {
    let sql = if frame {
        format!("{}{table}", frames::CLEAR)
    } else {
        format!("DELETE FROM {table}")
    };
    DraftStep {
        sql,
        effect: clearing(id),
    }
}

/// Runs the block of this index (`emit::RUN`).
fn run_step(index: usize) -> DraftStep {
    DraftStep {
        sql: format!("{}{index}", emit::RUN),
        effect: Effect::default(),
    }
}

/// The effect of emptying `table`.
fn clearing<T: Clone>(table: T) -> Effect<T> {
    Effect {
        rule: emit::Rule::EmptyIfAnyInputEmpty,
        inputs: vec![table.clone()],
        writes: Writes::Clear(table),
    }
}

/// How a session makes a counted table.
enum Make {
    /// A frame, by its declaration (`frames::FRAME`, `FrameStore::register`).
    Frame(String),
    /// A B-tree, by its `CREATE TABLE` and its indexes' statements.
    BTree(String),
}

/// A counted table: its name, and how a session makes it.
struct Table {
    name: String,
    make: Make,
}

/// The counted tables of a plan as its stages and clauses name them, by id.
#[derive(Default)]
struct Catalog {
    ids: HashMap<String, usize>,
    tables: Vec<Table>,
}

impl Catalog {
    /// The table `name`, made by `make`: a frame is made once for every list that declares it
    /// alike (a register), a B-tree once (`IF NOT EXISTS`). A name is a frame or a B-tree, never
    /// both.
    fn add(&mut self, name: &str, make: Make) -> Result<usize, String> {
        if let Some(&id) = self.ids.get(name) {
            return match (&self.tables[id].make, &make) {
                (Make::Frame(made), Make::Frame(declared)) if made == declared => Ok(id),
                (Make::BTree(_), Make::BTree(_)) => Ok(id),
                (Make::Frame(_), Make::Frame(_)) => {
                    Err(format!("{name} is two frames of different declarations"))
                }
                _ => Err(format!("{name} is both a frame and a B-tree")),
            };
        }
        let id = self.tables.len();
        self.ids.insert(name.to_owned(), id);
        self.tables.push(Table {
            name: name.to_owned(),
            make,
        });
        Ok(id)
    }

    /// The table of stage `index`. A three-valued stage keeps the nodes it is unknown at too,
    /// with `unk`; a two-valued stage's are all 0.
    fn stage(&mut self, index: usize) -> Result<usize, String> {
        let table = emit::stage_table(index);
        self.add(
            &table,
            Make::Frame(format!(
                "{table}(file INTEGER, node INTEGER, unk INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (file, node))"
            )),
        )
    }

    /// Adds `sql`, a `CREATE INDEX`, to the making of the B-tree it indexes.
    fn index(&mut self, sql: &str) -> Result<(), String> {
        let name = sql
            .split_once(" ON ")
            .and_then(|(_, on)| on.split([' ', '(']).next())
            .unwrap_or_default();
        match self.ids.get(name).map(|&id| &mut self.tables[id].make) {
            Some(Make::BTree(create)) => {
                create.push_str("; ");
                create.push_str(sql);
                Ok(())
            }
            _ => Err(format!("an index of no B-tree: {sql}")),
        }
    }

    fn id(&self, name: &str) -> usize {
        *self
            .ids
            .get(name)
            .unwrap_or_else(|| panic!("{name} is a counted table"))
    }
}

/// A compiled program as its sessions run it, fixed once and shared by them: every counted table
/// and how it is made, each stage's and clause's statements (`shapes`: those of stages and clauses
/// alike are stored once), and the recursion blocks' marks. The frames of operators' temporaries
/// are registers (`registers`), so clauses that differ only in their frames' names run the same
/// statement texts.
struct Plan {
    large: Option<large::Program>,
    fused: Option<super::fused::Program>,
    tables: Vec<Table>,
    shapes: Shapes,
    stages: Vec<(GrammarDialect, Instance)>,
    clauses: Vec<(Instance, Vec<CaptureColumn>)>,
    /// Per stage, the clause that is its only reader, where no other clause or stage reads it:
    /// such a stage is materialized for one consumer, so it runs inside that clause's run, after
    /// its root frame, and not where the clause's root frame is empty (`Runner::prepared_with`).
    sole: Vec<Option<usize>>,
    marks: usize,
    census: ShapeCensus,
    /// How many stages and clauses binding checked against their drafts (`shapes::verify`).
    #[cfg(test)]
    verified: usize,
}

/// What a plan holds: its steps (markers and clears included), their distinct texts, and its
/// frame and B-tree names; and how they are stored: the stages' and clauses' distinct shapes, and
/// their texts' distinct skeletons (`texts`), in `stored` bytes.
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::code_rule_dsl) struct Census {
    pub(in crate::code_rule_dsl) steps: usize,
    pub(in crate::code_rule_dsl) texts: usize,
    pub(in crate::code_rule_dsl) frames: usize,
    pub(in crate::code_rule_dsl) btrees: usize,
    /// The distinct texts' bytes.
    pub(in crate::code_rule_dsl) bytes: usize,
    pub(in crate::code_rule_dsl) shapes: usize,
    pub(in crate::code_rule_dsl) skeletons: usize,
    pub(in crate::code_rule_dsl) stored: usize,
}

/// A plan being made: each clause is planned as it is compiled, so its emitted statements are
/// freed before the next is emitted, and the stages, which clauses share, once every clause is.
/// Each stage's and clause's draft is bound to its shape as it is planned (`ShapeMaker`). The
/// first error spoils the plan.
#[derive(Default)]
struct Planner {
    fused: super::fused::Builder,
    catalog: Catalog,
    maker: ShapeMaker,
    classes: HashMap<String, usize>,
    marks: usize,
    clauses: Vec<(Instance, Vec<CaptureColumn>)>,
    error: Option<String>,
}

impl Planner {
    /// Plans the program's next clause, freeing its emitted statements unless it is to `keep`
    /// them.
    fn clause(
        &mut self,
        clause: &mut ClauseSql,
        keep: bool,
        statement: &Statement,
        stages: &[Stage],
        terminals: &[Terminal],
    ) {
        let index = self.clauses.len();
        self.fused.clause(
            index,
            statement,
            &clause.sql,
            stages,
            terminals,
            clause.each,
        );
        match self.plan_clause(index, clause, keep) {
            Ok(instance) => self.clauses.push((instance, clause.captures.clone())),
            Err(error) => {
                self.error.get_or_insert(error);
            }
        }
    }

    fn plan_clause(
        &mut self,
        index: usize,
        clause: &mut ClauseSql,
        keep: bool,
    ) -> Result<Instance, String> {
        let emitted = taken(&mut clause.sql, keep);
        if self.error.is_some() {
            return Err(String::new());
        }
        let each = format!("e{index}_");
        let enumeration: Vec<(String, Make)> = if clause.each {
            [
                (
                    "x",
                    "file INTEGER, root INTEGER, start INTEGER, \"end\" INTEGER, PRIMARY KEY (file, root, start, \"end\")) WITHOUT ROWID",
                ),
                (
                    "a",
                    "file INTEGER, root INTEGER, PRIMARY KEY (file, root)) WITHOUT ROWID",
                ),
                ("r", "round INTEGER)"),
            ]
            .into_iter()
            .map(|(table, columns)| {
                let name = format!("{each}{table}");
                let make = Make::BTree(format!("CREATE TEMP TABLE {name}({columns}"));
                (name, make)
            })
            .collect()
        } else {
            Vec::new()
        };
        let names: Vec<String> = enumeration.iter().map(|(name, _)| name.clone()).collect();
        let mut draft = plan(
            &mut self.catalog,
            &mut self.classes,
            &mut self.marks,
            emitted.into_owned(),
            (&format!("c{index}_"), "", &each),
            run_id(None, index),
            enumeration,
        )?;
        if clause.each {
            let ids: Vec<usize> = names.iter().map(|name| self.catalog.id(name)).collect();
            let clear = ids
                .iter()
                .zip(&names)
                .map(|(&id, name)| clear_step(id, name, false))
                .collect();
            draft.tables.extend(&ids);
            draft.each = Some(DraftEach {
                prefix: each,
                clear,
                active: ids[1],
                round: ids[2],
            });
        }
        self.bind(draft)
    }

    /// `draft` bound to its shape: its own tables are the catalog's last.
    fn bind(&mut self, draft: Draft) -> Result<Instance, String> {
        let catalog = &self.catalog;
        self.maker
            .bind(draft, catalog.tables.len(), &|name| catalog.id(name))
    }

    /// The plan, once every clause is planned: with `stages`, whose emitted statements it frees
    /// unless it is to `keep` them.
    fn finish(
        mut self,
        stages: &mut [Stage],
        keep: bool,
        batch_size: usize,
    ) -> Result<Plan, String> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let mut planned_stages = Vec::with_capacity(stages.len());
        for (index, stage) in stages.iter_mut().enumerate() {
            let table = emit::stage_table(index);
            let id = self.catalog.stage(index)?;
            // A stage's frames live until the batch ends: its registers are its own.
            let namespace = emit::stage_namespace(index);
            let mut draft = plan(
                &mut self.catalog,
                &mut self.classes,
                &mut self.marks,
                taken(&mut stage.sql, keep).into_owned(),
                (&namespace, &namespace, ""),
                run_id(Some(index), 0),
                Vec::new(),
            )?;
            draft.tables.push(id);
            draft.clear.push(clear_step(id, &table, true));
            let columns = if stage.three {
                "file, node, unk"
            } else {
                "file, node"
            };
            let mut effect = draft.last.effect.clone();
            effect.writes = Writes::Insert(id);
            draft.last = DraftStep {
                sql: format!("INSERT INTO {table}({columns}) {}", draft.last.sql),
                effect,
            };
            planned_stages.push((stage.dialect, self.bind(draft)?));
        }
        #[cfg(test)]
        let verified = self.maker.verified();
        let (shapes, census) = self.maker.finish();
        // Each stage's readers: a stage read by another stage, or by two clauses, is shared.
        let mut readers: Vec<(usize, Option<usize>)> = vec![(0, None); planned_stages.len()];
        for (_, instance) in &planned_stages {
            for &stage in &shapes.bound(instance).shape.stages {
                readers[stage] = (readers[stage].0 + 2, None);
            }
        }
        for (clause, (instance, _)) in self.clauses.iter().enumerate() {
            for &stage in &shapes.bound(instance).shape.stages {
                readers[stage] = (readers[stage].0 + 1, Some(clause));
            }
        }
        let sole: Vec<Option<usize>> = readers
            .into_iter()
            .map(|(count, clause)| clause.filter(|_| count == 1))
            .collect();
        let mut plan = Plan {
            large: None,
            fused: self.fused.finish(&sole, batch_size),
            tables: self.catalog.tables,
            shapes,
            stages: planned_stages,
            clauses: self.clauses,
            sole,
            marks: self.marks,
            census,
            #[cfg(test)]
            verified,
        };
        plan.large = large::compile(&plan)?;
        let covered = plan.fused.as_ref().map_or(0, |f| f.clauses.len())
            + plan.large.as_ref().map_or(0, |p| p.clauses.len());
        if covered != plan.clauses.len() {
            return Err("SQL compiler left an unscheduled clause".into());
        }
        Ok(plan)
    }
}

impl Plan {
    fn census(&self) -> Census {
        let frames = self
            .tables
            .iter()
            .filter(|table| matches!(table.make, Make::Frame(_)))
            .count();
        let census = &self.census;
        Census {
            steps: census.steps,
            texts: census.texts,
            bytes: census.bytes,
            frames: self.large.as_ref().map_or(frames, |p| p.table_count()),
            btrees: if self.large.is_some() {
                0
            } else {
                self.tables.len() - frames
            },
            shapes: census.shapes,
            skeletons: census.skeletons,
            stored: census.stored,
        }
    }

    /// The stage (`index` below the stages' count) or clause of this index.
    fn prepared(&self, index: usize) -> Bound<'_> {
        let instance = match index.checked_sub(self.stages.len()) {
            None => &self.stages[index].1,
            Some(clause) => &self.clauses[clause].0,
        };
        self.shapes.bound(instance)
    }
}

/// How many statements `steps` hold, each once: a block's (its nested blocks' included) count at
/// its first run, not at every run, so blocks run at several sites, nested in each other, count
/// their statements once rather than once per path to them. The markers and the loops' queries
/// are not statements.
fn size(steps: &[DraftStep], blocks: &[DraftBlock]) -> usize {
    fn count(steps: &[DraftStep], blocks: &[DraftBlock], counted: &mut HashSet<usize>) -> usize {
        steps
            .iter()
            .map(|step| match step.sql.strip_prefix(emit::RUN) {
                Some(index) => {
                    let index = index.parse::<usize>().expect("a block index");
                    if counted.insert(index) {
                        count(&blocks[index].steps, blocks, counted)
                    } else {
                        0
                    }
                }
                None => usize::from(!step.sql.starts_with("/*")),
            })
            .sum()
    }
    count(steps, blocks, &mut HashSet::new())
}

/// Moves each block of `steps` (`emit::BLOCK<id>` … `emit::END`, blocks nested in it too) into
/// `blocks`, leaving `emit::RUN<index>` where it was; every later `emit::RUN<id>` runs the block
/// `id` last named. Each block takes the next of `marks`.
fn extract_blocks(
    steps: &mut std::vec::IntoIter<DraftStep>,
    blocks: &mut Vec<DraftBlock>,
    named: &mut HashMap<String, usize>,
    marks: &mut usize,
) -> Vec<DraftStep> {
    let mut out = Vec::new();
    while let Some(step) = steps.next() {
        if let Some(id) = step.sql.strip_prefix(emit::BLOCK) {
            let id = id.to_owned();
            let index = blocks.len();
            blocks.push(DraftBlock {
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
            out.push(run_step(index));
        } else if let Some(id) = step.sql.strip_prefix(emit::RUN) {
            let index = *named.get(id).expect("a block emitted before it runs");
            out.push(run_step(index));
        } else if step.sql == emit::END {
            return out;
        } else {
            out.push(step);
        }
    }
    out
}

/// The temporary `rest` starts with (after `emit::TEMP`): its prefix's letters, then digits.
fn temporary(rest: &str) -> &str {
    let letters = rest.bytes().take_while(u8::is_ascii_lowercase).count();
    let digits = rest[letters..]
        .bytes()
        .take_while(u8::is_ascii_digit)
        .count();
    &rest[..letters + digits]
}

/// Where one temporary of a stage's or clause's statements is mentioned (`registers`): its
/// mentions' span, each extended to its outermost loop; its declaration (`frames::FRAME`), its
/// first clear (a loop's `create` empties it on every pass) and the span of its other mentions,
/// unextended; and the blocks that mention it, whose every `emit::RUN` does (`BlockRuns`).
#[derive(Default)]
struct Uses {
    span: Option<(usize, usize)>,
    declared: usize,
    cleared: Option<usize>,
    mentions: Option<(usize, usize)>,
    blocks: Vec<usize>,
}

/// Where a block's `emit::RUN`s are (`registers`): their positions in order, and the span of
/// the outermost loops they are in (or their own positions), where it has a run.
#[derive(Default)]
struct BlockRuns {
    at: Vec<usize>,
    reach: Option<(usize, usize)>,
}

/// Where a temporary is live (`Uses::live`): one span, and the blocks whose runs write it anew.
struct Live {
    span: (usize, usize),
    blocks: Vec<usize>,
}

impl Uses {
    /// `item`, at `at` in a loop that spans `span` (or none: `at` alone), mentions it.
    fn mention(&mut self, item: &str, at: usize, span: (usize, usize)) {
        let widen = |into: &mut Option<(usize, usize)>, (from, to): (usize, usize)| {
            let (start, end) = into.get_or_insert((from, to));
            *start = (*start).min(from);
            *end = (*end).max(to);
        };
        widen(&mut self.span, span);
        if item.starts_with(frames::FRAME) {
            self.declared = at;
            return;
        }
        if item.starts_with(frames::CLEAR) && self.cleared.is_none() {
            self.cleared = Some(at);
        }
        widen(&mut self.mentions, (at, at));
    }

    /// Where it is live. A pass's own temporary (declared, then emptied, then mentioned only
    /// within one loop: `inner`, each item's innermost) is written anew by every pass before it
    /// is read: it is live from its declaration to its last mention, and dead between its
    /// blocks' runs, where it is written anew (`Live::blocks`). Any other temporary is live over
    /// its mentions' span extended to their loops, and from there to its blocks' last run.
    fn live(self, inner: &[Option<(usize, usize)>], runs: &[BlockRuns]) -> Live {
        let span = self.span.expect("a declared temporary");
        let own = self.cleared.and_then(|cleared| {
            let (first, last) = self.mentions.unwrap_or((cleared, cleared));
            inner[cleared]
                .is_some_and(|(start, end)| {
                    start <= self.declared && first >= cleared && last <= end
                })
                .then_some(last)
        });
        match own {
            Some(last) => Live {
                span: (self.declared, last),
                blocks: self.blocks,
            },
            None => Live {
                span: self
                    .blocks
                    .iter()
                    .filter_map(|&block| runs[block].reach)
                    .fold(span, |(start, end), (from, to)| {
                        (start.min(from), end.max(to))
                    }),
                blocks: Vec::new(),
            },
        }
    }
}

/// Allocates the temporaries of one stage's or clause's statements (`items`, after
/// `frames::rewrite`) that are frames to registers, as a compiler allocates variables: a register
/// is a frame name of one declaration (its class, numbered in `classes`, which every list
/// shares). Each temporary, in the order of its first live position, takes its class's lowest
/// register that no other temporary holds over its live span (`Uses::live`), and that no block
/// writes within the span: a block's run writes its own temporaries' registers there. A
/// temporary live across a run started before its block's temporaries, so it overlaps their
/// spans, or after them, so it finds their registers written. A clause's tables are emptied when
/// it ends, so every list starts with its registers empty, and lists run one after another:
/// lists share registers (`prefix` names them: a stage's frames live until the batch ends, so a
/// stage's prefix is its own). Returns each temporary's register, and where a register taken
/// after another temporary is emptied: `(position, register)`, before the item at the later
/// temporary's first live position (a pass's own temporary empties itself on every pass after).
fn registers(
    items: &[String],
    classes: &mut HashMap<String, usize>,
    prefix: &str,
) -> (HashMap<String, String>, Vec<(usize, String)>) {
    // The temporaries that are frames, each with its class.
    let mut temporaries: HashMap<&str, usize> = HashMap::new();
    let mut names: Vec<&str> = Vec::new();
    let mut class_of: Vec<usize> = Vec::new();
    for item in items {
        if let Some(declaration) = item.strip_prefix(frames::FRAME)
            && let Some(open) = declaration.find('(')
            && let Some(name) = declaration[..open].trim().strip_prefix(emit::TEMP)
        {
            let next = classes.len();
            let class = *classes
                .entry(declaration[open..].to_owned())
                .or_insert(next);
            temporaries.entry(name).or_insert_with(|| {
                names.push(name);
                class_of.push(class);
                names.len() - 1
            });
        }
    }
    if names.is_empty() {
        return Default::default();
    }
    // Each item's outermost and innermost loop (`emit::LOOP` … `emit::UNTIL`), where it is in one.
    let mut loops = Vec::new();
    let mut open = Vec::new();
    for (at, item) in items.iter().enumerate() {
        if emit::loop_start(item) {
            open.push(at);
        } else if item.starts_with(emit::UNTIL) {
            loops.push((open.pop().expect("an open loop"), at));
        }
    }
    let mut outer: Vec<Option<(usize, usize)>> = vec![None; items.len()];
    let mut inner: Vec<Option<(usize, usize)>> = vec![None; items.len()];
    loops.sort_by_key(|&(start, end)| Reverse(end - start));
    for &(start, end) in &loops {
        for at in start..=end {
            outer[at].get_or_insert((start, end));
            inner[at] = Some((start, end));
        }
    }
    let mut uses: Vec<Uses> = names.iter().map(|_| Uses::default()).collect();
    // Per block (each `emit::BLOCK` … `emit::END`, by its order of closing): the temporaries it
    // mentions, those of the blocks nested in it or run in it included, and its runs. The open
    // blocks, innermost last, keep the temporaries they mention and the blocks they hold or
    // run; every `emit::RUN<id>` runs the block `id` last closed.
    let mut members: Vec<Vec<usize>> = Vec::new();
    let mut runs: Vec<BlockRuns> = Vec::new();
    let mut named: HashMap<&str, usize> = HashMap::new();
    let mut inside: Vec<(&str, Vec<usize>, Vec<usize>)> = Vec::new();
    let mut mentioned = Vec::new();
    for (at, item) in items.iter().enumerate() {
        if let Some(id) = item.strip_prefix(emit::BLOCK) {
            inside.push((id, Vec::new(), Vec::new()));
            continue;
        }
        if item == emit::END {
            let (id, mut temporaries, mut held) = inside.pop().expect("an open block");
            held.sort_unstable();
            held.dedup();
            for &block in &held {
                temporaries.extend_from_slice(&members[block]);
            }
            temporaries.sort_unstable();
            temporaries.dedup();
            let block = members.len();
            members.push(temporaries);
            runs.push(BlockRuns::default());
            if let Some((_, _, outer)) = inside.last_mut() {
                outer.push(block);
            }
            named.insert(id, block);
            continue;
        }
        let span = outer[at].unwrap_or((at, at));
        if let Some(id) = item.strip_prefix(emit::RUN) {
            if let Some(&block) = named.get(id) {
                let run = &mut runs[block];
                run.at.push(at);
                let (from, to) = run.reach.get_or_insert(span);
                *from = (*from).min(span.0);
                *to = (*to).max(span.1);
                if let Some((_, _, held)) = inside.last_mut() {
                    held.push(block);
                }
            }
            continue;
        }
        mentioned.clear();
        for found in temporaries_in(item) {
            let name = temporary(&item[found + emit::TEMP.len()..]);
            mentioned.extend(temporaries.get(name).copied());
        }
        for &temporary in &mentioned {
            uses[temporary].mention(item, at, span);
        }
        if let Some((_, block, _)) = inside.last_mut() {
            block.extend_from_slice(&mentioned);
        }
    }
    // A block run nowhere writes and extends nothing.
    for (block, temporaries) in members.iter().enumerate() {
        if !runs[block].at.is_empty() {
            for &temporary in temporaries {
                uses[temporary].blocks.push(block);
            }
        }
    }
    let live: Vec<Live> = uses
        .into_iter()
        .map(|uses| uses.live(&inner, &runs))
        .collect();
    let mut order: Vec<usize> = (0..names.len()).collect();
    order.sort_by_key(|&temporary| (live[temporary].span.0, temporary));
    // Per class, its registers. Temporaries come in the order of their spans' starts, so every
    // span a register held starts before the current one, and the last it took ends last (held
    // spans do not overlap): a register is free where that span ends before the current one
    // starts and no block's run writes it within the current span. A write before the current
    // start matters to no later temporary.
    let mut held: HashMap<usize, ClassRegisters> = HashMap::new();
    let mut allocated = HashMap::with_capacity(names.len());
    let mut clears = Vec::new();
    for temporary in order {
        let class = class_of[temporary];
        let Live {
            span: (start, end),
            ref blocks,
        } = live[temporary];
        let ClassRegisters {
            registers,
            busy,
            idle,
        } = held.entry(class).or_default();
        while let Some(&(until, register)) = busy.first()
            && until < start
        {
            busy.pop_first();
            idle.insert(register);
        }
        let free = idle
            .iter()
            .copied()
            .find(|&register| registers[register].unwritten(start, end));
        let reused = free.is_some();
        let register = match free {
            Some(register) => {
                idle.remove(&register);
                register
            }
            None => {
                registers.push(Register::default());
                registers.len() - 1
            }
        };
        busy.insert((end, register));
        registers[register].written_by(blocks, &runs, start);
        let name = format!("{prefix}f{class}_{register}");
        if reused {
            clears.push((start, name.clone()));
        }
        allocated.insert(names[temporary].to_owned(), name);
    }
    (allocated, clears)
}

/// One declaration class's registers as `registers` allocates them: those holding a span not yet
/// ended (by its end), and the others.
#[derive(Default)]
struct ClassRegisters {
    registers: Vec<Register>,
    busy: BTreeSet<(usize, usize)>,
    idle: BTreeSet<usize>,
}

/// A register's writes (`registers`): the blocks whose runs write it, and where those runs are
/// from the last start it was asked about on, in order, from `next`.
#[derive(Default)]
struct Register {
    writers: Vec<usize>,
    writes: Vec<usize>,
    next: usize,
}

impl Register {
    /// Whether no block's run writes it within `start..=end`; `start` never decreases.
    fn unwritten(&mut self, start: usize, end: usize) -> bool {
        while self.writes.get(self.next).is_some_and(|&at| at < start) {
            self.next += 1;
        }
        self.writes.get(self.next).is_none_or(|&at| at > end)
    }

    /// The runs of `blocks` write it too, from `start` on.
    fn written_by(&mut self, blocks: &[usize], runs: &[BlockRuns], start: usize) {
        let new: Vec<usize> = blocks
            .iter()
            .copied()
            .filter(|block| !self.writers.contains(block))
            .collect();
        if new.is_empty() {
            return;
        }
        let mut writes = self.writes.split_off(self.next);
        for &block in &new {
            let at = &runs[block].at;
            writes.extend_from_slice(&at[at.partition_point(|&at| at < start)..]);
        }
        writes.sort_unstable();
        writes.dedup();
        self.writes = writes;
        self.next = 0;
        self.writers.extend(new);
    }
}

/// `emitted`, taken out of its stage or clause unless the program is to `keep` it: what is taken
/// is freed once planned.
fn taken(emitted: &mut Emitted, keep: bool) -> std::borrow::Cow<'_, Emitted> {
    if keep {
        std::borrow::Cow::Borrowed(emitted)
    } else {
        std::borrow::Cow::Owned(std::mem::take(emitted))
    }
}

/// `sql` with every `(from, to)` substituted, in one pass: each `from` is a marker (`/*…*/`, no
/// one a prefix of another) and no `to` holds one; `sql` itself where it holds none.
fn substitute(sql: String, substitutions: &[(&str, &str)]) -> String {
    let bytes = sql.as_bytes();
    let mut out: Option<String> = None;
    let mut copied = 0;
    for at in memchr::memchr_iter(b'/', bytes) {
        if at < copied || bytes.get(at + 1) != Some(&b'*') {
            continue;
        }
        let rest = &sql[at..];
        if let Some((from, to)) = substitutions
            .iter()
            .find(|(from, _)| rest.starts_with(from))
        {
            let out = out.get_or_insert_with(|| String::with_capacity(sql.len() + 64));
            out.push_str(&sql[copied..at]);
            out.push_str(to);
            copied = at + from.len();
        }
    }
    match out {
        None => sql,
        Some(mut out) => {
            out.push_str(&sql[copied..]);
            out
        }
    }
}

/// Where `emit::TEMP` starts in `sql`, each place.
fn temporaries_in(sql: &str) -> memchr::memmem::FindIter<'_, 'static> {
    static TEMPORARY: std::sync::LazyLock<memchr::memmem::Finder<'static>> =
        std::sync::LazyLock::new(|| memchr::memmem::Finder::new(emit::TEMP));
    TEMPORARY.find_iter(sql.as_bytes())
}

/// Fixes the text of `emitted`'s statements, loops' markers included (`Runner::steps`), with
/// their effects over the counted tables of `catalog`, to which it adds the tables they make: a
/// table its statements only append to and empty whole is a frame (`frames::rewrite`), any other
/// a B-tree. An operator's temporary that is a frame is a register (`registers`), emptied before
/// a later temporary takes it. The statements read their root frame's files and roots under the
/// run id `run`; `names` is the prefixes of their own tables (`emit::NAMESPACE`), of their
/// registers, and of an enumeration's tables (`emit::EACH`), which `enumeration` makes. The
/// tables of its own (named by a prefix of `names`) are the catalog's last, one after another
/// (`Draft::own`).
fn plan(
    catalog: &mut Catalog,
    classes: &mut HashMap<String, usize>,
    marks: &mut usize,
    emitted: Emitted,
    (namespace, prefix, each): (&str, &str, &str),
    run: i64,
    enumeration: Vec<(String, Make)>,
) -> Result<Draft, String> {
    let first_mark = *marks;
    let files = format!("SELECT file FROM runs WHERE run = {run}");
    let roots = format!("SELECT file, node FROM roots WHERE run = {run}");
    let substitutions = [
        (emit::NAMESPACE, namespace),
        (emit::EACH, each),
        (emit::FILES, files.as_str()),
        (emit::ROOTS, roots.as_str()),
    ];
    let Emitted {
        statements,
        effects,
        root,
    } = emitted;
    let statements: Vec<String> = statements
        .into_iter()
        .map(|statement| substitute(statement, &substitutions))
        .collect();
    // `rewrite` keeps every statement but the schema's in order (a frame's clear for its
    // `DELETE`): their effects, in that order.
    let schema = |sql: &str| sql.starts_with("CREATE ");
    let mut effects = statements
        .iter()
        .zip(effects)
        .filter(|(sql, _)| !schema(sql))
        .map(|(_, effect)| effect)
        .collect::<Vec<_>>()
        .into_iter();
    let items = frames::rewrite(statements);
    let (registers, clears) = registers(&items, classes, prefix);
    // A temporary's name: its register, or, a B-tree, the namespace's.
    let temporary_name = |name: &str, out: &mut String| match registers.get(name) {
        Some(register) => out.push_str(register),
        None => {
            out.push_str(namespace);
            out.push_str(name);
        }
    };
    let rename = |sql: String| -> String {
        let mut found = temporaries_in(&sql).peekable();
        if found.peek().is_none() {
            return sql;
        }
        let mut out = String::with_capacity(sql.len());
        let mut copied = 0;
        for at in found {
            if at < copied {
                continue;
            }
            out.push_str(&sql[copied..at]);
            let after = &sql[at + emit::TEMP.len()..];
            let name = temporary(after);
            temporary_name(name, &mut out);
            copied = at + emit::TEMP.len() + name.len();
        }
        out.push_str(&sql[copied..]);
        out
    };
    let mut clears = clears.into_iter().peekable();
    let mut steps: Vec<(String, Effect)> = Vec::new();
    // The tables the statements make, in order, and their indexes.
    let mut made: Vec<(String, Option<Make>)> = Vec::new();
    let mut indexes = Vec::new();
    let mut stages = Vec::new();
    for (at, sql) in items.into_iter().enumerate() {
        while let Some((_, register)) = clears.next_if(|(position, _)| *position == at) {
            steps.push((format!("{}{register}", frames::CLEAR), clearing(register)));
        }
        let sql = rename(sql);
        stages_read(&sql, &mut stages);
        if sql.starts_with("CREATE INDEX ") {
            indexes.push(sql);
            continue;
        }
        let (name, make) = if let Some(declaration) = sql.strip_prefix(frames::FRAME) {
            let name = declaration
                .split_once('(')
                .map(|(name, _)| name.trim())
                .ok_or_else(|| format!("a frame without columns: {sql}"))?;
            (name.to_owned(), Make::Frame(declaration.to_owned()))
        } else if let Some(table) = sql.strip_prefix("CREATE TEMP TABLE ") {
            let table = table.strip_prefix("IF NOT EXISTS ").unwrap_or(table);
            let name = table.split(['(', ' ']).next().unwrap_or_default();
            (name.to_owned(), Make::BTree(sql.clone()))
        } else {
            steps.push((sql, effects.next().expect("an effect per statement")));
            continue;
        };
        made.push((name, Some(make)));
    }
    assert!(effects.next().is_none(), "a statement per effect");
    stages.sort_unstable();
    stages.dedup();
    // The stages' tables the statements read: a clause is planned before the stages.
    for &stage in &stages {
        catalog.stage(stage)?;
    }
    // The tables shared with other stages and clauses (registers), then those of its own.
    let own_table =
        |name: &str| name.starts_with(namespace) || (!each.is_empty() && name.starts_with(each));
    let mut ids = vec![0; made.len()];
    for (at, (name, make)) in made.iter_mut().enumerate() {
        if !own_table(name) {
            ids[at] = catalog.add(name, make.take().expect("a table's making"))?;
        }
    }
    let own = catalog.tables.len();
    for (name, make) in enumeration {
        catalog.add(&name, make)?;
    }
    for (at, (name, make)) in made.iter_mut().enumerate() {
        if let Some(make) = make.take() {
            ids[at] = catalog.add(name, make)?;
        }
    }
    for sql in &indexes {
        catalog.index(sql)?;
    }
    let mut tables = Vec::new();
    let mut clear = Vec::new();
    let mut cleared = HashSet::new();
    for ((name, _), id) in made.iter().zip(ids) {
        if cleared.insert(id) {
            tables.push(id);
            let frame = matches!(catalog.tables[id].make, Make::Frame(_));
            clear.push(clear_step(id, name, frame));
        }
    }
    // A table's name in an effect, as the statements name it.
    let resolve = |name: &str| -> String {
        match name.strip_prefix(emit::TEMP) {
            Some(name) => {
                let mut out = String::new();
                temporary_name(name, &mut out);
                out
            }
            None => substitute(name.to_owned(), &substitutions[..2]),
        }
    };
    // Once every table is added: a block's marker names its units' demand tables, which the
    // loop creates after it.
    let steps: Vec<DraftStep> = steps
        .into_iter()
        .map(|(sql, effect)| DraftStep {
            sql,
            effect: effect.map(|table| catalog.id(&resolve(&table))),
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
    let root = root.as_ref().map(|root| {
        let table = catalog.id(&resolve(root));
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
    Ok(Draft {
        steps,
        blocks,
        last,
        clear,
        stages,
        each: None,
        root,
        tables,
        own,
        marks: first_mark,
    })
}

/// The stages `sql` reads, added to `stages`: each `stage_<index>` it names.
fn stages_read(sql: &str, stages: &mut Vec<usize>) {
    static STAGE: std::sync::LazyLock<(String, memchr::memmem::Finder<'static>)> =
        std::sync::LazyLock::new(|| {
            let prefix = emit::stage_table(0).trim_end_matches('0').to_owned();
            let finder = memchr::memmem::Finder::new(prefix.as_bytes()).into_owned();
            (prefix, finder)
        });
    let (prefix, finder) = &*STAGE;
    for at in finder.find_iter(sql.as_bytes()) {
        let digits = sql[at + prefix.len()..]
            .bytes()
            .take_while(u8::is_ascii_digit)
            .count();
        if let Ok(stage) = sql[at + prefix.len()..at + prefix.len() + digits].parse() {
            stages.push(stage);
        }
    }
}

/// The run id a stage's or clause's root frame reads its files and roots under.
fn run_id(stage: Option<usize>, clause: usize) -> i64 {
    match stage {
        Some(stage) => -1 - stage as i64,
        None => clause as i64,
    }
}

/// The statements every batch runs besides its stages' and clauses'.
const BATCH_STATEMENTS: usize = 2;

/// A connection running one compiled program's batches (`Plan`). A session starts with only the
/// tables of the batch's own rows, `runs` and `roots`: the files and roots a statement's root
/// frame reads, under the statement's run id. A stage's or clause's tables are made on its first
/// run (`activate`), and its statements prepared on their first execution, into a cache sized
/// for the distinct texts of the stages and clauses that ran. The session keeps them across
/// batches: a batch empties its tables (`frames`: the frame tables' buffers) and refills them
/// through the cached statements. `counts`: every counted table's rows (`skip`).
pub(in crate::code_rule_dsl) struct Session {
    // Drop the callback owner before closing the connection.
    #[cfg(any(test, feature = "unbounded-work"))]
    profiling: Option<Box<super::profiling::Profiler>>,
    #[cfg(any(test, feature = "unbounded-work"))]
    relational_profile: Option<Arc<super::fused::profile::Profile>>,
    connection: Connection,
    roots: Arc<RootInputs>,
    slot: Arc<BatchSlot>,
    program: Arc<Program>,
    plan: Arc<Plan>,
    stats: Stats,
    frames: Arc<FrameStore>,
    counts: Counts,
    /// Per counted table: whether it is made.
    made: Vec<bool>,
    /// Per stage, then clause: whether its tables are made and its texts counted.
    active: Vec<bool>,
    /// Per distinct text of the plan: whether the statement cache counts it.
    cached: Vec<bool>,
    /// The statement cache's capacity.
    capacity: usize,
    /// The buffers the steps' texts are written into as they run (`Runner::texts`).
    texts: Vec<String>,
}

impl Session {
    fn new(compiled: &Compiled) -> Result<Self, SqlError> {
        let started = Started::now();
        let plan = compiled.plan()?;
        let slot = Arc::new(BatchSlot::default());
        let connection = connect(&slot)?;
        let frames = Arc::clone(&slot.frames);
        #[cfg(any(test, feature = "unbounded-work"))]
        // SAFETY: Session drops profiling before connection; constructor errors do the same.
        let profiling = unsafe { super::profiling::Profiler::configured(&connection) }?;
        connection.execute_batch("PRAGMA temp_store = MEMORY")?;
        let roots = Arc::new(RootInputs::default());
        roots.register(&connection)?;
        if let Some(fused) = &plan.fused {
            fused.register(&connection, &frames)?;
        }
        if let Some(large) = &plan.large {
            large.register(&connection, &frames)?;
        }
        #[cfg(feature = "unbounded-work")]
        super::diagnostics::configure(&connection)?;
        #[cfg(any(test, feature = "unbounded-work"))]
        let relational_profile = if profiling.is_some() && plan.fused.is_some() {
            let profile = super::fused::profile::Profile::new(true);
            profile.register(&connection)?;
            Some(profile)
        } else {
            None
        };
        let fused_texts = plan.fused.as_ref().map_or(0, |f| f.plan.queries.len())
            + plan.large.as_ref().map_or(0, |p| p.queries.len());
        connection.set_prepared_statement_cache_capacity(BATCH_STATEMENTS + fused_texts);
        let names = plan
            .tables
            .iter()
            .map(|table| (table.name.clone(), matches!(table.make, Make::Frame(_))))
            .collect();
        Ok(Self {
            #[cfg(any(test, feature = "unbounded-work"))]
            profiling,
            #[cfg(any(test, feature = "unbounded-work"))]
            relational_profile,
            connection,
            roots,
            slot,
            program: Arc::clone(&compiled.program),
            counts: Counts::new(Arc::clone(&frames), names, plan.marks),
            made: vec![false; plan.tables.len()],
            active: vec![false; plan.stages.len() + plan.clauses.len()],
            cached: vec![false; plan.census.texts],
            texts: Vec::new(),
            capacity: BATCH_STATEMENTS + fused_texts,
            stats: Stats {
                sessions: 1,
                texts: fused_texts,
                tables: plan
                    .fused
                    .as_ref()
                    .map_or(0, |f| f.plan.graph.handles.len())
                    + plan.large.as_ref().map_or(0, |p| p.table_count()),
                setup: started.elapsed(),
                ..Stats::default()
            },
            plan,
            frames,
        })
    }

    /// Makes the tables of the stages and clauses `pending` names (stages first, then clauses, as
    /// `Plan::prepared` indexes them) and of the stages they read, where none ran on this session
    /// before, and sizes the statement cache for their texts: a `violation.each` clause's rounds
    /// run three statements of their own.
    fn activate(&mut self, mut pending: Vec<usize>) -> Result<(), SqlError> {
        let started = Started::now();
        let plan = Arc::clone(&self.plan);
        let capacity = self.capacity;
        while let Some(index) = pending.pop() {
            if std::mem::replace(&mut self.active[index], true) {
                continue;
            }
            let bound = plan.prepared(index);
            pending.extend(bound.shape.stages.iter().copied());
            for table in bound.tables() {
                if self.made[table] {
                    continue;
                }
                match &plan.tables[table].make {
                    Make::Frame(declaration) => {
                        self.frames.register(&self.connection, declaration)?;
                    }
                    Make::BTree(create) => self.connection.execute_batch(create)?,
                }
                self.made[table] = true;
                self.stats.tables += 1;
            }
            for &text in &bound.instance().texts {
                if !std::mem::replace(&mut self.cached[text as usize], true) {
                    self.capacity += 1;
                }
            }
            if bound.shape.each.is_some() {
                self.capacity += 3;
            }
        }
        if self.capacity > capacity {
            self.connection
                .set_prepared_statement_cache_capacity(self.capacity);
            self.stats.texts += self.capacity - capacity;
        }
        self.stats.setup.add(started.elapsed());
        Ok(())
    }

    /// What the session's statements did so far.
    pub(in crate::code_rule_dsl) fn stats(&self) -> Stats {
        self.stats
    }

    /// How many statements the connection holds prepared: its cache's.
    pub(in crate::code_rule_dsl) fn prepared_statements(&self) -> usize {
        let mut count = 0;
        // SAFETY: the connection is open and used by this thread only; `sqlite3_next_stmt` reads
        // its list of statements, which nothing changes meanwhile.
        unsafe {
            let handle = self.connection.handle();
            let mut statement = rusqlite::ffi::sqlite3_next_stmt(handle, std::ptr::null_mut());
            while !statement.is_null() {
                count += 1;
                statement = rusqlite::ffi::sqlite3_next_stmt(handle, statement);
            }
        }
        count
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
    pub(in crate::code_rule_dsl) fn plans(&mut self) -> Result<Vec<(String, QueryPlan)>, SqlError> {
        let queries = self
            .plan
            .fused
            .as_ref()
            .map(|f| f.plan.queries.as_slice())
            .into_iter()
            .chain(self.plan.large.as_ref().map(|p| p.queries.as_slice()));
        let mut plans = Vec::new();
        for sql in queries.flatten() {
            let mut statement = self
                .connection
                .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
            let rows = statement
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(3)?)))?
                .collect::<Result<Vec<_>, _>>()?;
            plans.push((sql.clone(), rows));
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
        #[cfg(any(test, feature = "unbounded-work"))]
        if let Some(profile) = &self.relational_profile {
            profile.reset();
        }
        let stages = self.plan.stages.len();
        let pending = if self.plan.fused.as_ref().is_some_and(|f| {
            runs.iter()
                .any(|r| !r.files.is_empty() && f.contains(r.statement))
        }) {
            (0..self.plan.clauses.len())
                .filter(|&c| !self.plan.fused.as_ref().unwrap().contains(c))
                .map(|c| stages + c)
                .collect()
        } else {
            runs.iter()
                .filter(|r| !r.files.is_empty())
                .map(|r| stages + r.statement)
                .collect()
        };
        if self.plan.large.is_none() {
            self.activate(pending)?;
        }
        let started = Started::now();
        let batch = Batch::of(docs, Arc::clone(&self.program));
        self.stats.node_facts.add(started.elapsed());
        *self.slot.current.write() = Some(Arc::clone(&batch));
        let scopes = BatchScopes::begin();
        let facts = take_fact_time();
        let result = (|| {
            self.connection.execute_batch("BEGIN")?;
            let result = self.run_batch(&batch, runs);
            let finished =
                self.connection
                    .execute_batch(if result.is_ok() { "COMMIT" } else { "ROLLBACK" });
            if finished.is_err() && !self.connection.is_autocommit() {
                let _ = self.connection.execute_batch("ROLLBACK");
            }
            finished?;
            result
        })();
        self.stats.facts.add(take_fact_time());
        debug_assert_eq!(
            facts,
            Timing::default(),
            "a fact terminal's time outside a batch"
        );
        drop(scopes);
        *self.slot.current.write() = None;
        self.roots.clear();
        if let Some(large) = &self.plan.large {
            large.clear(&self.frames);
        }
        if result.is_err() && self.plan.large.is_none() {
            for (table, made) in self.plan.tables.iter().zip(&self.made) {
                if !made {
                    continue;
                }
                match table.make {
                    Make::Frame(_) => {
                        self.frames.clear(&table.name);
                    }
                    Make::BTree(_) => {
                        self.connection
                            .prepare_cached(&format!("DELETE FROM {}", table.name))?
                            .execute([])?;
                    }
                }
            }

            self.counts.reset();
        }
        #[cfg(any(test, feature = "unbounded-work"))]
        if let (Some(profiler), Some(profile), Some(fused)) =
            (&self.profiling, &self.relational_profile, &self.plan.fused)
        {
            profiler.relational(profile.report(&fused.plan.graph));
        }
        if let Some(fused) = &self.plan.fused {
            fused.clear(&self.frames);
        }
        self.frames.trim();
        #[cfg(any(test, feature = "unbounded-work"))]
        if let Some(profiling) = &self.profiling {
            profiling.checkpoint(&self.connection)?;
        }
        result
    }

    fn run_batch(
        &mut self,
        batch: &Batch,
        runs: &[ClauseRun],
    ) -> Result<Vec<Vec<Vec<SqlMatch>>>, SqlError> {
        let Self {
            #[cfg(any(test, feature = "unbounded-work"))]
            profiling,
            connection,
            roots,
            plan,
            stats,
            frames,
            counts,
            texts,
            ..
        } = self;
        let plan = &**plan;
        let (stages, clauses, sole) = (&plan.stages, &plan.clauses, &plan.sole);
        let connection = &*connection;
        let mut runner = Runner {
            #[cfg(any(test, feature = "unbounded-work"))]
            profiling: profiling.as_deref(),
            connection,
            frames,
            stats,
            counts,
            texts,
            at: ("", 0),
            depth: 0,
        };
        let inputs = Started::now();

        #[cfg(any(test, feature = "unbounded-work"))]
        let publication = profiling.as_deref().map(|profiler| {
            profiler.operation(
                "/*ROOT_INPUT*/ publish selected root slices",
                ("input", 0),
                0,
                Vec::new(),
            )
        });
        let selected_roots = roots.install(runs);
        #[cfg(any(test, feature = "unbounded-work"))]
        if let Some(publication) = publication {
            publication.finish(selected_roots);
        }
        #[cfg(not(any(test, feature = "unbounded-work")))]
        let _ = selected_roots;
        // Each stage runs over the whole files of the clauses that read it, directly or not.
        let mut stage_files: Vec<Vec<usize>> = stages.iter().map(|_| Vec::new()).collect();
        let mut scheduled_stage = vec![false; stages.len()];
        {
            let mut memberships = Vec::new();
            for run in runs.iter().filter(|run| !run.files.is_empty()) {
                let id = run_id(None, run.statement);
                for (file, roots) in &run.files {
                    if roots.is_none() {
                        memberships.push((id, *file as i64));
                    }
                }
                let mut reads = plan
                    .prepared(stages.len() + run.statement)
                    .shape
                    .stages
                    .to_vec();
                while let Some(stage) = reads.pop() {
                    if !plan
                        .fused
                        .as_ref()
                        .is_some_and(|f| f.contains(run.statement))
                    {
                        scheduled_stage[stage] = true;
                    }
                    let files = &mut stage_files[stage];
                    let before = files.len();
                    files.extend(run.files.iter().map(|(file, _)| *file));
                    files.sort_unstable();
                    files.dedup();
                    // Files new to the stage are new to the stages it reads.
                    if files.len() > before
                        || !plan
                            .fused
                            .as_ref()
                            .is_some_and(|f| f.contains(run.statement))
                    {
                        reads.extend(plan.prepared(stage).shape.stages.iter().copied());
                    }
                }
            }
            for (stage, files) in stage_files.iter().enumerate() {
                let id = run_id(Some(stage), 0);
                for &file in files {
                    if batch.files[file].dialect == stages[stage].0 {
                        memberships.push((id, file as i64));
                    }
                }
            }
            roots.install_runs(memberships);
        }
        runner.stats.input.add(inputs.elapsed());
        let mut fused_found: HashMap<usize, Vec<(usize, i64, SqlMatch)>> = HashMap::new();
        let query_plans = plan
            .fused
            .as_ref()
            .filter(|f| {
                runs.iter()
                    .any(|r| !r.files.is_empty() && f.contains(r.statement))
            })
            .map(|f| ("fused", f.plan.queries.as_slice()))
            .into_iter()
            .chain(
                plan.large
                    .as_ref()
                    .filter(|p| {
                        runs.iter()
                            .any(|r| !r.files.is_empty() && p.contains(r.statement))
                    })
                    .map(|p| ("cte-program", p.queries.as_slice())),
            );
        for (_scope, queries) in query_plans {
            // Physical phase IDs belong to the optional instrumentation context.
            #[cfg_attr(not(feature = "unbounded-work"), allow(clippy::unused_enumerate_index))]
            for (_phase, sql) in queries.iter().enumerate() {
                #[cfg(any(test, feature = "unbounded-work"))]
                let operation = profiling
                    .as_deref()
                    .map(|p| p.operation(sql, (_scope, _phase), 0, Vec::new()));
                let started = Started::now();
                let mut statement = connection.prepare_cached(sql)?;
                let has_round = statement.column_count() > 4;
                let mut rows = statement.query([])?;
                let mut count = 0;
                while let Some(row) = rows.next()? {
                    let clause: i64 = row.get(0)?;
                    if clause < 0 {
                        continue;
                    }
                    let file = row.get::<_, u32>(1)? as usize;
                    let root: u32 = row.get(2)?;
                    let payload: String = row.get(3)?;
                    let values: Vec<serde_json::Value> =
                        serde_json::from_str(&payload).map_err(|e| SqlError::Sql(e.to_string()))?;
                    let capture = &clauses[clause as usize].1;
                    let captures = bindings_from(capture, |at| {
                        Ok(match values.get(at - 2) {
                            Some(serde_json::Value::Number(n)) if n.as_i64().is_some() => {
                                Value::Integer(n.as_i64().unwrap())
                            }
                            Some(serde_json::Value::String(s)) => Value::Text(s.clone()),
                            _ => Value::Null,
                        })
                    })?;
                    fused_found.entry(clause as usize).or_default().push((
                        file,
                        if has_round { row.get(4)? } else { 0 },
                        SqlMatch { root, captures },
                    ));
                    count += 1;
                }
                drop(rows);
                drop(statement);
                let elapsed = started.elapsed();
                #[cfg(any(test, feature = "unbounded-work"))]
                if let Some(operation) = operation {
                    operation.finish(count);
                }
                runner.stats.statements += 1;
                runner.stats.rows += count;
                runner.stats.time.add(elapsed);
                runner.stats.select.add(elapsed);
                #[cfg(any(test, feature = "unbounded-work"))]
                record(sql, elapsed.wall, count);
            }
        }
        // Stages in index order: a stage only reads stages made before it. A stage with one
        // consumer runs in its clause's run instead (`Plan::sole`).
        let started = Started::now();
        for (stage, files) in stage_files
            .iter()
            .enumerate()
            .filter(|_| plan.large.is_none())
        {
            if files.is_empty() || sole[stage].is_some() || !scheduled_stage[stage] {
                continue;
            }
            runner.at = ("stage", stage);
            let bound = plan.prepared(stage);
            let ran = runner.prepared(bound)?;
            if ran || *MODE == Mode::Check {
                runner.statement(bound, &bound.shape.last, !ran)?;
            }
        }
        runner.stats.stages.add(started.elapsed());
        let mut sole_ran = vec![false; stages.len()];
        let mut results = Vec::with_capacity(runs.len());
        let mut position: Vec<usize> = vec![usize::MAX; batch.files.len()];
        for run in runs {
            if plan
                .fused
                .as_ref()
                .is_some_and(|f| f.contains(run.statement))
                || plan
                    .large
                    .as_ref()
                    .is_some_and(|p| p.contains(run.statement))
            {
                let mut found = vec![Vec::new(); run.files.len()];
                let mut matches = fused_found.remove(&run.statement).unwrap_or_default();
                matches.sort_by_key(|(file, round, hit)| (*file, hit.root, *round));
                for (file, _, hit) in matches {
                    for (at, (selected_file, _)) in run.files.iter().enumerate() {
                        if *selected_file == file {
                            found[at].push(hit.clone());
                        }
                    }
                }
                results.push(found);
                continue;
            }
            let mut found: Vec<Vec<(usize, SqlMatch)>> =
                run.files.iter().map(|_| Vec::new()).collect();
            if !run.files.is_empty() {
                #[cfg(feature = "unbounded-work")]
                let clause_started = std::time::Instant::now();
                for (at, (file, _)) in run.files.iter().enumerate() {
                    position[*file] = at;
                }
                let bound = plan.prepared(stages.len() + run.statement);
                let captures = &clauses[run.statement].1;
                runner.at = ("clause", run.statement);
                // The stages only this clause reads, run once in the batch after its root frame.
                let mut deferred: Vec<(usize, Bound<'_>)> = Vec::new();
                for (stage, clause) in sole.iter().enumerate() {
                    if *clause == Some(run.statement)
                        && !stage_files[stage].is_empty()
                        && !sole_ran[stage]
                    {
                        sole_ran[stage] = true;
                        deferred.push((stage, plan.prepared(stage)));
                    }
                }
                let each = bound.each();
                if let Some((prefix, _, round)) = each {
                    let rows = connection
                        .prepare_cached(&format!("INSERT INTO {prefix}r VALUES (0)"))?
                        .execute([])?;
                    runner.counts.wrote(&Writes::Insert(round), rows);
                }
                // `violation.each`: rounds over the whole batch. A round re-runs the clause on the
                // roots whose last match bound a fresh range of the capture, with every range
                // reported for the root excluded; each reports what it found, after the earlier
                // rounds' findings for the same root.
                for round in 0.. {
                    // A clause whose root frame was empty selects nothing (`Runner::prepared`,
                    // which counted its last as passed over).
                    let ran = runner.prepared_with(bound, &mut deferred)?;
                    let last = &bound.shape.last;
                    let outcome = runner.outcome(bound, last, !ran);
                    let skipped = *MODE == Mode::Skip && (!ran || outcome == Outcome::Empty);
                    if skipped && ran {
                        runner.stats.skipped += 1;
                    }
                    if !skipped {
                        let mut text = runner.buffer();
                        bound.text(last, &mut text);
                        #[cfg(any(test, feature = "unbounded-work"))]
                        let operation = runner.profile_operation(&text, bound, last);
                        let started = Started::now();
                        let mut statement = connection.prepare_cached(&text)?;
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
                        #[cfg(any(test, feature = "unbounded-work"))]
                        if let Some(operation) = operation {
                            operation.finish(count);
                        }
                        let elapsed = started.elapsed();
                        let stats = &mut *runner.stats;
                        stats.statements += 1;
                        stats.rows += count;
                        stats.time.add(elapsed);
                        stats.select.add(elapsed);
                        #[cfg(any(test, feature = "unbounded-work"))]
                        record(&text, elapsed.wall, count);
                        if *MODE == Mode::Check {
                            skip::check(&text, outcome, count);
                        }
                        runner.release(text);
                    }
                    let started = Started::now();
                    runner.clear(bound)?;
                    runner.stats.input.add(started.elapsed());
                    let Some((prefix, active, _)) = each else {
                        break;
                    };
                    // The roots still enumerated: none ends the rounds.
                    let active = if *MODE == Mode::Skip {
                        runner.counts.rows(active)
                    } else {
                        let active_rows: i64 = connection
                            .prepare_cached(&format!("SELECT count(*) FROM {prefix}a"))?
                            .query_row([], |row| row.get(0))?;
                        if *MODE == Mode::Check {
                            skip::check(
                                "the enumerated roots",
                                Outcome::Exactly(runner.counts.rows(active)),
                                active_rows as usize,
                            );
                        }
                        active_rows
                    };
                    if active == 0 {
                        break;
                    }
                    connection
                        .prepare_cached(&format!("UPDATE {prefix}r SET round = round + 1"))?
                        .execute([])?;
                }
                if let Some(each) = &bound.shape.each {
                    runner.clears(bound, &each.clear)?;
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
        let started = Started::now();
        for (stage, files) in stage_files
            .iter()
            .enumerate()
            .filter(|_| plan.large.is_none())
        {
            if !files.is_empty() && plan.prepared(stage).tables().all(|id| self.made[id]) {
                runner.at = ("stage", stage);
                runner.clear(plan.prepared(stage))?;
            }
        }
        runner.stats.input.add(started.elapsed());
        Ok(results)
    }
}

/// A session's connection, frames, statistics and counts while a batch runs its statements;
/// `at`, the stage or clause running (`verify`).
struct Runner<'s> {
    #[cfg(any(test, feature = "unbounded-work"))]
    profiling: Option<&'s super::profiling::Profiler>,
    connection: &'s Connection,
    frames: &'s FrameStore,
    stats: &'s mut Stats,
    counts: &'s mut Counts,
    /// Spare buffers to write the steps' texts into, one per text being run at once.
    texts: &'s mut Vec<String>,
    at: (&'static str, usize),
    /// How many recursion loops the statement running is in.
    depth: usize,
}

impl<'s> Runner<'s> {
    #[cfg(any(test, feature = "unbounded-work"))]
    fn profile_operation(
        &self,
        sql: &str,
        bound: Bound<'_>,
        step: &Step,
    ) -> Option<super::profiling::Operation<'s>> {
        self.profiling.map(|profiler| {
            profiler.operation(
                sql,
                self.at,
                self.depth,
                bound
                    .inputs(bound.action(step))
                    .map(|table| (self.counts.name(table).to_owned(), self.counts.rows(table)))
                    .collect(),
            )
        })
    }

    /// A buffer to write a step's text into; `release` it once the text is run.
    fn buffer(&mut self) -> String {
        self.texts.pop().unwrap_or_default()
    }

    fn release(&mut self, buffer: String) {
        self.texts.push(buffer);
    }

    /// `RERAIL_SQL_SKIP=check`: panics where a table of `tables` holds other than its count,
    /// before `sql` decides anything by it.
    fn verify(&self, tables: impl IntoIterator<Item = usize>, sql: &str) -> Result<(), SqlError> {
        if *MODE != Mode::Check {
            return Ok(());
        }
        for table in tables {
            let name = self.counts.name(table);
            let rows: i64 = self
                .connection
                .prepare_cached(&format!("SELECT count(*) FROM {name}"))?
                .query_row([], |row| row.get(0))?;
            let counted = self.counts.rows(table);
            assert!(
                rows == counted,
                "RERAIL_SQL_SKIP=check: {} {}: {name} holds {rows} rows, counted {counted}, before {sql}",
                self.at.0,
                self.at.1
            );
        }
        Ok(())
    }

    /// Runs a stage's or clause's steps, from its root frame on (`skip`): where the root frame is
    /// empty for the batch, every statement after it reads nothing, and is passed over (checking,
    /// run and found to change nothing); returns whether the root frame had rows.
    fn prepared(&mut self, bound: Bound<'_>) -> Result<bool, SqlError> {
        self.prepared_with(bound, &mut Vec::new())
    }

    /// `prepared`, running `deferred` first after the root frame (and not at all where the skip
    /// passes over the rest): the stages only this clause reads (`Plan::sole`), each made once.
    fn prepared_with(
        &mut self,
        bound: Bound<'_>,
        deferred: &mut Vec<(usize, Bound<'_>)>,
    ) -> Result<bool, SqlError> {
        let steps = &bound.shape.steps;
        let Some(root) = bound.root() else {
            self.deferred(deferred)?;
            self.steps(bound, steps, false)?;
            return Ok(true);
        };
        self.steps(bound, &steps[..=root.at], false)?;
        let empty = *MODE != Mode::Run && self.counts.rows(root.table) == 0;
        if empty && *MODE == Mode::Skip {
            self.stats.skipped_runs += 1;
            self.stats.skipped += root.rest;
            return Ok(false);
        }
        self.deferred(deferred)?;
        self.steps(bound, &steps[root.at + 1..], empty)?;
        Ok(!empty)
    }

    /// Makes the stages `deferred` holds, in index order, and empties it.
    fn deferred(&mut self, deferred: &mut Vec<(usize, Bound<'_>)>) -> Result<(), SqlError> {
        if deferred.is_empty() {
            return Ok(());
        }
        let (at, started) = (self.at, Started::now());
        for (stage, bound) in deferred.drain(..) {
            self.at = ("stage", stage);
            let ran = self.prepared(bound)?;
            if ran || *MODE == Mode::Check {
                self.statement(bound, &bound.shape.last, !ran)?;
            }
        }
        self.at = at;
        self.stats.stages.add(started.elapsed());
        Ok(())
    }

    /// Executes a stage's or clause's statements, each loop (`emit::LOOP` … `emit::UNTIL`) while
    /// its last query counts a dirty key (none: not at all), and accepting a loop's last pass by its reads
    /// (`emit::ACYCLIC`). A loop that still changes after `MAX_LEVELS` passes refuses the batch: a
    /// recursion that deep is not evaluated. `emit::RUN<index>` runs the shape's block `index`
    /// there, over the tables as they stand. `known_empty`: checking a run the skip would pass
    /// over, whose statements must change nothing (but a loop's clears of its frames from an
    /// earlier run).
    fn steps(
        &mut self,
        bound: Bound<'_>,
        steps: &[Step],
        known_empty: bool,
    ) -> Result<(), SqlError> {
        let mut at = 0;
        while at < steps.len() {
            let step = &steps[at];
            match bound.action(step).kind {
                Kind::Acyclic => {
                    let mut text = self.buffer();
                    bound.text(step, &mut text);
                    acyclic(self.connection, &text[emit::ACYCLIC.len()..])?;
                    self.release(text);
                    at += 1;
                    continue;
                }
                Kind::Clear(_) => {
                    self.clear_step(bound, step)?;
                    at += 1;
                    continue;
                }
                Kind::Run(index) => {
                    self.block(bound, &bound.shape.blocks[index as usize], known_empty)?;
                    at += 1;
                    continue;
                }
                Kind::Statement | Kind::Until => {
                    self.statement(bound, step, known_empty)?;
                    at += 1;
                    continue;
                }
                Kind::Loop | Kind::Frontier => {}
            }
            let mut depth = 0;
            let end = (at..steps.len())
                .find(|&index| {
                    match bound.action(&steps[index]).kind {
                        Kind::Loop | Kind::Frontier => depth += 1,
                        Kind::Until => depth -= 1,
                        _ => {}
                    }
                    depth == 0
                })
                .expect("a loop's end");
            let mut text = self.buffer();
            bound.text(&steps[end], &mut text);
            let until = &text[emit::UNTIL.len()..];
            if bound.action(step).kind == Kind::Frontier {
                let description = until
                    .strip_prefix(emit::FRONTIER)
                    .expect("a frontier descriptor");
                #[cfg(any(test, feature = "unbounded-work"))]
                let initialization = self.profile_operation(until, bound, &steps[end]);
                let started = Started::now();
                let mut frontier = self.frames.frontier(description)?;
                #[cfg(any(test, feature = "unbounded-work"))]
                if let Some(operation) = initialization {
                    operation.finish(0);
                }
                let mut initialized = started.elapsed();
                #[cfg(any(test, feature = "unbounded-work"))]
                let mut passes = 0;
                self.depth += 1;
                loop {
                    #[cfg(any(test, feature = "unbounded-work"))]
                    let operation = self.profile_operation(until, bound, &steps[end]);
                    let started = Started::now();
                    let rows = self.frames.frontier_fill(&mut frontier)?;
                    let mut elapsed = started.elapsed().plus(initialized);
                    initialized = Timing::ZERO;
                    #[cfg(any(test, feature = "unbounded-work"))]
                    if let Some(operation) = operation {
                        operation.finish(rows);
                    }
                    self.steps(bound, &steps[at + 1..end], known_empty)?;
                    #[cfg(any(test, feature = "unbounded-work"))]
                    let operation = self.profile_operation(until, bound, &steps[end]);
                    let advanced = Started::now();
                    let more = frontier.advance()?;
                    #[cfg(any(test, feature = "unbounded-work"))]
                    if let Some(operation) = operation {
                        operation.finish(0);
                    }
                    elapsed.add(advanced.elapsed());
                    self.stats.statements += 1;
                    self.stats.rows += rows;
                    self.stats.time.add(elapsed);
                    #[cfg(feature = "unbounded-work")]
                    if super::diagnostics::enabled(until) {
                        super::diagnostics::record(
                            self.connection,
                            until,
                            elapsed,
                            rows,
                            [0; 4],
                            bound
                                .inputs(bound.action(&steps[end]))
                                .map(|table| (self.counts.name(table), self.counts.rows(table))),
                        );
                    }
                    #[cfg(any(test, feature = "unbounded-work"))]
                    {
                        record(until, elapsed.wall, rows);
                        record_looped(until, elapsed.wall, rows);
                    }
                    #[cfg(any(test, feature = "unbounded-work"))]
                    {
                        passes += 1;
                    }
                    if !more {
                        break;
                    }
                }
                self.depth -= 1;
                #[cfg(any(test, feature = "unbounded-work"))]
                record_loop(until, passes, end - at - 1);
                self.release(text);
                at = end + 1;
                continue;
            }
            // A loop's pass evaluates the dirty keys of its units: with none (the site demanded
            // no new key), it would change nothing, and the units it reaches would be run for
            // nothing again, at every site of every pass above them.
            #[cfg(any(test, feature = "unbounded-work"))]
            let (profiler, scope, depth) = (self.profiling, self.at, self.depth);
            let dirty = |connection: &Connection| -> Result<bool, SqlError> {
                #[cfg(any(test, feature = "unbounded-work"))]
                let operation = profiler.map(|p| p.operation(until, scope, depth, Vec::new()));
                let changed: i64 = connection
                    .prepare_cached(until)?
                    .query_row([], |row| row.get(0))?;
                #[cfg(any(test, feature = "unbounded-work"))]
                if let Some(operation) = operation {
                    operation.finish(1);
                }
                Ok(changed != 0)
            };
            if !dirty(self.connection)? {
                self.release(text);
                at = end + 1;
                continue;
            }
            let mut passes = 0;
            // Mutant 11: the loop's last pass is undone.
            let drop_last = super::mutant(11);
            self.depth += 1;
            loop {
                if drop_last {
                    self.connection.execute_batch("SAVEPOINT mutant_pass")?;
                }
                self.steps(bound, &steps[at + 1..end], known_empty)?;
                passes += 1;
                let changed = dirty(self.connection)?;
                if drop_last {
                    let undo = if changed {
                        ""
                    } else {
                        "ROLLBACK TO mutant_pass; "
                    };
                    self.connection
                        .execute_batch(&format!("{undo}RELEASE mutant_pass"))?;
                }
                if !changed {
                    break;
                }
                if passes > MAX_LEVELS {
                    return Err(SqlError::unsupported("recursion deeper than 256 levels"));
                }
            }
            self.depth -= 1;
            #[cfg(any(test, feature = "unbounded-work"))]
            record_loop(until, passes, end - at - 1);
            self.release(text);
            at = end + 1;
        }
        Ok(())
    }

    /// Runs a recursion block, unless its units hold no demand row its last run did not see: it
    /// ended with no key dirty, and only a new demand row is dirty outside it (`skip`).
    fn block(
        &mut self,
        bound: Bound<'_>,
        block: &Block,
        known_empty: bool,
    ) -> Result<(), SqlError> {
        self.verify(bound.demand(block), "a block's run")?;
        let mark = bound.mark(block);
        let idle =
            *MODE != Mode::Run && self.counts.sum(bound.demand(block)) == self.counts.marks[mark];
        if idle && *MODE == Mode::Skip {
            self.stats.skipped_blocks += 1;
            self.stats.skipped += block.size;
            return Ok(());
        }
        self.steps(bound, &block.steps, known_empty || idle)?;
        self.counts.marks[mark] = self.counts.sum(bound.demand(block));
        Ok(())
    }

    /// What `step`'s rule says it will change (`skip`); `known_empty`, nothing, but a clear.
    fn outcome(&self, bound: Bound<'_>, step: &Step, known_empty: bool) -> Outcome {
        let action = bound.action(step);
        match *MODE {
            Mode::Run => Outcome::Unknown,
            _ if known_empty && !matches!(bound.writes(action), Writes::Clear(_)) => Outcome::Empty,
            _ => bound.outcome(action, |table| self.counts.rows(table)),
        }
    }

    /// Executes one statement that writes rows, unless its rule proves it empty; counts what it
    /// changed.
    fn statement(
        &mut self,
        bound: Bound<'_>,
        step: &Step,
        known_empty: bool,
    ) -> Result<(), SqlError> {
        let action = bound.action(step);
        let writes = bound.writes(action);
        let mut text = self.buffer();
        if *MODE == Mode::Check {
            bound.text(step, &mut text);
            self.verify(bound.inputs(action), &text)?;
            if let Writes::Insert(table) | Writes::Delete(table) = writes {
                self.verify([table], &text)?;
            }
        }
        let outcome = self.outcome(bound, step, known_empty);
        if outcome == Outcome::Empty && *MODE == Mode::Skip {
            self.stats.skipped += 1;
            self.release(text);
            return Ok(());
        }
        if *MODE != Mode::Check {
            bound.text(step, &mut text);
        }
        #[cfg(test)]
        super::query_shapes::before_statement(self.connection, self.frames, &mut text)?;
        #[cfg(any(test, feature = "unbounded-work"))]
        let operation = self.profile_operation(&text, bound, step);
        let started = Started::now();
        let selected = self.frames.select(self.connection, &text)?;
        let mut statement = self.connection.prepare_cached(&text)?;
        let rows = match selected {
            Some(rows) => rows,
            None => statement.execute([])?,
        };
        let elapsed = started.elapsed();
        #[cfg(any(test, feature = "unbounded-work"))]
        if let Some(operation) = operation {
            operation.finish(rows);
        }
        #[cfg(feature = "unbounded-work")]
        if super::diagnostics::enabled(&text) {
            let selection = frames::take_selection_trace();
            let mut counters = super::diagnostics::counters(&statement);
            if let Some((_, query_counters)) = &selection {
                for (counter, query) in counters.iter_mut().zip(query_counters) {
                    *counter += query;
                }
            }
            super::diagnostics::record(
                self.connection,
                selection
                    .as_ref()
                    .filter(|_| selected.is_some())
                    .map_or(text.as_str(), |(query, _)| query.as_str()),
                elapsed,
                rows,
                counters,
                bound
                    .inputs(action)
                    .map(|table| (self.counts.name(table), self.counts.rows(table))),
            );
        }
        self.stats.statements += 1;
        self.stats.rows += rows;
        self.stats.time.add(elapsed);
        #[cfg(any(test, feature = "unbounded-work"))]
        {
            record(&text, elapsed.wall, rows);
            if self.depth > 0 {
                record_looped(&text, elapsed.wall, rows);
            }
        }
        if *MODE == Mode::Check {
            skip::check(&text, outcome, rows);
        }
        self.release(text);
        self.counts.wrote(&writes, rows);
        Ok(())
    }

    /// Empties the tables of a stage or clause (its blocks' demand among them, so none has run).
    fn clear(&mut self, bound: Bound<'_>) -> Result<(), SqlError> {
        self.clears(bound, &bound.shape.clear)?;
        for block in &bound.shape.blocks {
            self.counts.marks[bound.mark(block)] = 0;
        }
        Ok(())
    }

    /// Empties the tables of `clears`, passing over those already empty.
    fn clears(&mut self, bound: Bound<'_>, clears: &[Step]) -> Result<(), SqlError> {
        for clear in clears {
            self.clear_step(bound, clear)?;
        }
        Ok(())
    }

    /// Empties one table, unless it is empty already: a frame through its buffers
    /// (`frames::CLEAR`, which runs no statement), a B-tree by its `DELETE`.
    fn clear_step(&mut self, bound: Bound<'_>, clear: &Step) -> Result<(), SqlError> {
        let action = bound.action(clear);
        let mut text = self.buffer();
        if *MODE == Mode::Check {
            bound.text(clear, &mut text);
            self.verify(bound.inputs(action), &text)?;
        }
        let outcome = self.outcome(bound, clear, false);
        if outcome == Outcome::Empty && *MODE == Mode::Skip {
            self.stats.skipped_clears += 1;
            self.release(text);
            return Ok(());
        }
        #[cfg(any(test, feature = "unbounded-work"))]
        let operation = if self.profiling.is_some() {
            bound.text(clear, &mut text);
            self.profile_operation(&text, bound, clear)
        } else {
            None
        };
        let rows = match action.kind {
            Kind::Clear(table) => {
                let name = self.counts.name(bound.table(table));
                let rows = self.frames.rows(name).expect("a registered frame");
                self.frames.clear(name);
                rows
            }
            _ => {
                bound.text(clear, &mut text);
                self.connection.prepare_cached(&text)?.execute([])?
            }
        };
        if *MODE == Mode::Check {
            skip::check(&text, outcome, rows);
        }
        self.release(text);
        #[cfg(any(test, feature = "unbounded-work"))]
        if let Some(operation) = operation {
            operation.finish(rows);
        }
        self.counts.wrote(&bound.writes(action), rows);
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

/// A thread's recursion loops (`LOOP_PROFILE`): per loop, by its `UNTIL` query, the passes of
/// each entry and its body's steps; and the statements run inside loops, by text.
#[cfg(any(test, feature = "unbounded-work"))]
#[derive(Debug, Default)]
pub(in crate::code_rule_dsl) struct LoopProfile {
    /// Per loop: (entries that ran a pass, passes, steps of its body).
    pub(in crate::code_rule_dsl) loops: std::collections::HashMap<String, (usize, usize, usize)>,
    /// The statements run inside a loop, as `STATEMENT_PROFILE` counts them.
    pub(in crate::code_rule_dsl) statements: StatementProfile,
}

#[cfg(any(test, feature = "unbounded-work"))]
thread_local! {
    /// Where set, this thread's recursion loops and the statements they ran (`LoopProfile`).
    pub(in crate::code_rule_dsl) static LOOP_PROFILE: std::cell::RefCell<Option<LoopProfile>> =
        const { std::cell::RefCell::new(None) };
}

/// Adds a loop's entry, its passes and its body's steps to `LOOP_PROFILE`, where set.
#[cfg(any(test, feature = "unbounded-work"))]
fn record_loop(until: &str, passes: usize, steps: usize) {
    LOOP_PROFILE.with_borrow_mut(|profile| {
        if let Some(profile) = profile {
            let entry = profile.loops.entry(until.to_owned()).or_default();
            entry.0 += 1;
            entry.1 += passes;
            entry.2 = steps;
        }
    });
}

/// Adds a statement run inside a loop to `LOOP_PROFILE`, where set.
#[cfg(any(test, feature = "unbounded-work"))]
fn record_looped(sql: &str, nanos: u64, rows: usize) {
    LOOP_PROFILE.with_borrow_mut(|profile| {
        if let Some(profile) = profile {
            let entry = profile.statements.entry(sql.to_owned()).or_default();
            entry.0 += nanos;
            entry.1 += rows;
            entry.2 += 1;
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
    super::diagnostics::flush();
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
    bindings_from(captures, |at| row.get::<_, Value>(at))
}

fn bindings_from(
    captures: &[(String, SlotKind, usize)],
    mut get: impl FnMut(usize) -> rusqlite::Result<Value>,
) -> rusqlite::Result<Vec<(String, Captured)>> {
    let mut bound = Vec::new();
    for (name, kind, at) in captures {
        let mut value = |offset: usize| -> rusqlite::Result<Option<i64>> {
            match get(at + offset)? {
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
            SlotKind::Text => match (get(*at)?, get(at + 1)?) {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `items` with `$name` standing for the temporary `name`, allocated with no class made yet:
    /// each temporary's register, and the clears.
    fn allocate(items: &[&str]) -> (HashMap<String, String>, Vec<(usize, String)>) {
        let items: Vec<String> = items
            .iter()
            .map(|item| item.replace('$', emit::TEMP))
            .collect();
        registers(&items, &mut HashMap::new(), "")
    }

    fn frame(name: &str) -> String {
        format!(
            "{}${name}(k INTEGER PRIMARY KEY, file, node)",
            frames::FRAME
        )
    }

    fn clear(name: &str) -> String {
        format!("{}${name}", frames::CLEAR)
    }

    #[test]
    fn a_dead_temporary_s_register_is_taken_again_and_emptied_first() {
        let items = [
            frame("x1"),
            "INSERT INTO $x1 SELECT 1, 1, 1".to_owned(),
            frame("x2"),
            "INSERT INTO $x2 SELECT * FROM $x1".to_owned(),
            frame("x3"),
            "INSERT INTO $x3 SELECT * FROM $x2".to_owned(),
            "SELECT * FROM $x3".to_owned(),
        ];
        let (registers, clears) = allocate(&items.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(registers["x1"], "f0_0");
        assert_eq!(registers["x2"], "f0_1");
        assert_eq!(registers["x3"], "f0_0");
        assert_eq!(clears, [(4, "f0_0".to_owned())]);
    }

    #[test]
    fn a_temporary_a_loop_reads_lives_through_the_loop() {
        let items = [
            frame("x1"),
            "INSERT INTO $x1 SELECT 1, 1, 1".to_owned(),
            emit::LOOP.to_owned(),
            frame("x2"),
            clear("x2"),
            "INSERT INTO $x2 SELECT * FROM $x1".to_owned(),
            format!("{}SELECT 0", emit::UNTIL),
            frame("x3"),
            "INSERT INTO $x3 SELECT 1, 1, 1".to_owned(),
        ];
        let (registers, clears) = allocate(&items.iter().map(String::as_str).collect::<Vec<_>>());
        // `x1` is read on every pass: `x2` may not take its register; `x3`, after the loop, may.
        assert_ne!(registers["x2"], registers["x1"]);
        assert_eq!(registers["x3"], registers["x1"]);
        assert_eq!(clears, [(7, registers["x1"].clone())]);
    }

    #[test]
    fn a_block_s_own_temporary_lives_at_each_run_and_not_between() {
        let items = [
            format!("{}7", emit::BLOCK),
            emit::LOOP.to_owned(),
            frame("x1"),
            clear("x1"),
            "INSERT INTO $x1 SELECT 1, 1, 1".to_owned(),
            format!("{}SELECT 0", emit::UNTIL),
            emit::END.to_owned(),
            frame("x2"),
            "INSERT INTO $x2 SELECT 1, 1, 1".to_owned(),
            "SELECT * FROM $x2".to_owned(),
            frame("x3"),
            "INSERT INTO $x3 SELECT 1, 1, 1".to_owned(),
            format!("{}7", emit::RUN),
            "SELECT * FROM $x3".to_owned(),
        ];
        let (registers, _) = allocate(&items.iter().map(String::as_str).collect::<Vec<_>>());
        // Between the block and its run, `x1` is dead: `x2` takes its register. `x3` is live
        // across the run, which writes `x1` again: it may not.
        assert_eq!(registers["x2"], registers["x1"]);
        assert_ne!(registers["x3"], registers["x1"]);
    }

    #[test]
    fn a_block_temporary_not_its_pass_s_own_lives_to_the_last_run() {
        let items = [
            format!("{}7", emit::BLOCK),
            frame("x1"),
            emit::LOOP.to_owned(),
            "INSERT INTO $x1 SELECT 1, 1, 1".to_owned(),
            format!("{}SELECT 0", emit::UNTIL),
            emit::END.to_owned(),
            frame("x2"),
            "INSERT INTO $x2 SELECT 1, 1, 1".to_owned(),
            "SELECT * FROM $x2".to_owned(),
            format!("{}7", emit::RUN),
        ];
        let (registers, _) = allocate(&items.iter().map(String::as_str).collect::<Vec<_>>());
        assert_ne!(registers["x2"], registers["x1"]);
    }
}

#[cfg(test)]
#[path = "inputs/session_tests.rs"]
mod input_tests;
