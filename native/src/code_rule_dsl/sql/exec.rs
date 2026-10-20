//! The batch driver: builds the `node` table state from a batch's tapes, computes the shared
//! stages, runs one statement per clause and returns each clause's `(file, root, bindings)` rows
//! for the native witness rendering.

use super::emit::{self, Effect, Emitted, Writes};
use super::frames::{self, FrameStore};
use super::lower::{ProgramBuilder, Stage};
use super::nearest::NearestTable;
use super::nodes::{NodeFacts, NodeTable};
use super::plan::{SlotKind, Statement, Unsupported};
use super::skip::{self, Counts, MODE, Mode, Outcome};
use super::terminals::{
    BatchScopes, FactTable, PatternRow, PatternTable, PatternTerm, Terminal, TextTable, fresh_match,
};
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
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

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

/// A batch: its files' tapes and node facts, its program, and the fresh-env matches of each
/// pattern terminal at the nodes tested so far.
pub(super) struct Batch {
    pub(super) files: Vec<BatchFile>,
    pub(super) program: Arc<Program>,
    fresh: Vec<parking_lot::Mutex<FreshMatches>>,
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
    connection.create_module(c"node", &NODE_MODULE, Some(Arc::clone(slot)))?;
    connection.create_module(c"pattern", &PATTERN_MODULE, Some(Arc::clone(slot)))?;
    connection.create_module(c"text_match", &TEXT_MODULE, Some(Arc::clone(slot)))?;
    connection.create_module(c"fact", &FACT_MODULE, Some(Arc::clone(slot)))?;
    connection.create_module(c"nearest", &NEAREST_MODULE, Some(Arc::clone(slot)))?;
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
        planner.clause(&mut clause, cfg!(test));
        clauses.push(clause);
    }
    Ok(Compiled::new(builder, planner, Some(clauses)))
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
    let mut planner = Planner::default();
    let mut planned = 0;
    let outcomes = grammars
        .iter()
        .map(|&(grammar, each)| {
            ProgramBuilder::default().clause_each(grammar, each)?;
            let statement = builder.clause_each(grammar, each)?;
            planner.clause(&mut ClauseSql::of(&statement, each), false);
            planned += 1;
            Ok(planned - 1)
        })
        .collect();
    (Compiled::new(builder, planner, None), outcomes)
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
    /// The program of `builder`'s terminals and stages and of the clauses `planner` planned.
    /// `clauses`: in the tests, a program `compile_enumerating` compiled keeps its emitted
    /// statements for the tests' censuses (`emitted`); a session runs the plan's.
    fn new(builder: ProgramBuilder, planner: Planner, clauses: Option<Vec<ClauseSql>>) -> Self {
        let ProgramBuilder {
            terminals,
            mut stages,
            ..
        } = builder;
        let keep = cfg!(test) && clauses.is_some();
        let plan = planner.finish(&mut stages, keep).map(Arc::new);
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
    pub(in crate::code_rule_dsl) fn planned_statements(&self) -> Vec<Arc<str>> {
        let Ok(plan) = &self.plan else {
            return Vec::new();
        };
        plan.stages
            .iter()
            .map(|(_, prepared)| prepared)
            .chain(plan.clauses.iter().map(|(prepared, _)| prepared))
            .flat_map(|prepared| {
                let blocks = prepared.blocks.iter().flat_map(|block| &block.steps);
                prepared.steps.iter().chain(blocks).chain([&prepared.last])
            })
            .map(|step| Arc::clone(&step.sql))
            .collect()
    }

    /// A connection set up to run this program's batches.
    pub(in crate::code_rule_dsl) fn session(&self) -> Result<Session, SqlError> {
        Session::new(self)
    }

    /// The program as its sessions run it.
    fn plan(&self) -> Result<Arc<Plan>, SqlError> {
        self.plan.clone().map_err(SqlError::Sql)
    }

    /// What the program's plan holds; nothing where it has none.
    pub(in crate::code_rule_dsl) fn census(&self) -> Census {
        self.plan
            .as_ref()
            .map(|plan| plan.census())
            .unwrap_or_default()
    }

    /// Runs one batch on a session of its own: `files` are its tapes, each with the statement (by
    /// index into the compiled grammars) that matches it, if any, and its roots (every root of the
    /// statement's kinds, or those an anchor index selected), and what the file's scopes read
    /// beyond its tape (`FileContext`). Returns each file's matches, ordered by root.
    pub(in crate::code_rule_dsl) fn run(
        &self,
        files: Vec<(TapeDoc, Option<(usize, Option<Arc<[u32]>>)>, FileContext)>,
    ) -> Result<Vec<Vec<SqlMatch>>, SqlError> {
        let mut session = self.session()?;
        let mut runs: Vec<ClauseRun> = (0..session.plan.clauses.len())
            .map(|statement| ClauseRun {
                statement,
                files: Vec::new(),
            })
            .collect();
        for (index, (_, statement, _)) in files.iter().enumerate() {
            if let Some((statement, roots)) = statement {
                runs[*statement].files.push((index, roots.clone()));
            }
        }
        let mut results: Vec<Vec<SqlMatch>> = files.iter().map(|_| Vec::new()).collect();
        let docs = files
            .into_iter()
            .map(|(doc, _, context)| (doc, context))
            .collect();
        let found = session.run(docs, &runs)?;
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
///
/// `sessions` counts the sessions made and `setup_nanos` the time their setup took: making the
/// connection, then each stage's and clause's tables on its first run (`Session::activate`).
/// `texts` is the statement texts the sessions' caches were sized for (each a distinct text of a
/// stage or clause that ran), `tables` the tables they made.
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
    pub(in crate::code_rule_dsl) sessions: usize,
    pub(in crate::code_rule_dsl) setup_nanos: u64,
    pub(in crate::code_rule_dsl) texts: usize,
    pub(in crate::code_rule_dsl) tables: usize,
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
        self.sessions += after.sessions - before.sessions;
        self.setup_nanos += after.setup_nanos - before.setup_nanos;
        self.texts += after.texts - before.texts;
        self.tables += after.tables - before.tables;
    }
}

/// One statement as a session runs it, with its effect over the session's counted tables (a
/// marker's, `emit::LOOP`, `emit::RUN`…, is `Rule::AlwaysRun` and writes nothing). `sql` is the
/// plan's one copy of its text, `text` that text's index (`Texts`).
struct Step {
    sql: Arc<str>,
    text: usize,
    effect: Effect<usize>,
}

/// A plan's distinct statement texts, each by its index: steps of the same text share it.
#[derive(Default)]
struct Texts(HashMap<Arc<str>, usize>);

impl Texts {
    /// `sql`, which does `effect`, as a step.
    fn step(&mut self, sql: String, effect: Effect<usize>) -> Step {
        let (sql, text) = match self.0.get_key_value(sql.as_str()) {
            Some((sql, &text)) => (Arc::clone(sql), text),
            None => {
                let text = self.0.len();
                let sql: Arc<str> = Arc::from(sql);
                self.0.insert(Arc::clone(&sql), text);
                (sql, text)
            }
        };
        Step { sql, text, effect }
    }

    /// Empties `table` (the counted table `id`), where it holds rows: a frame through its
    /// buffers (`frames::CLEAR`), a B-tree by its `DELETE`.
    fn clear(&mut self, id: usize, table: &str, frame: bool) -> Step {
        let sql = if frame {
            format!("{}{table}", frames::CLEAR)
        } else {
            format!("DELETE FROM {table}")
        };
        self.step(sql, clearing(id))
    }

    /// Runs the block of this index (`emit::RUN`).
    fn run(&mut self, index: usize) -> Step {
        self.step(format!("{}{index}", emit::RUN), Effect::default())
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

/// A stage's or clause's statements as a session runs them: their text is fixed once for the
/// program (`Plan`), so every batch of every session runs the same cached statements.
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
    /// The counted tables its statements read and write, which a session makes on its first run
    /// (`Session::activate`); the stages' tables it reads are theirs.
    tables: Vec<usize>,
    /// The distinct texts of the statements a session's cache holds for it: every step's but the
    /// markers' and frame clears', which prepare nothing (a loop's `emit::UNTIL` query does).
    texts: Vec<usize>,
}

impl Prepared {
    fn collect_texts(&mut self) {
        let blocks = self.blocks.iter().flat_map(|block| &block.steps);
        let each = self.each.iter().flat_map(|each| &each.clear);
        let mut texts: Vec<usize> = self
            .steps
            .iter()
            .chain(blocks)
            .chain([&self.last])
            .chain(&self.clear)
            .chain(each)
            .filter(|step| !step.sql.starts_with("/*") || step.sql.starts_with(emit::UNTIL))
            .map(|step| step.text)
            .collect();
        texts.sort_unstable();
        texts.dedup();
        self.texts = texts;
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
/// and how it is made, each stage's and clause's statements, and the recursion blocks' marks.
/// The frames of operators' temporaries are registers (`registers`), so clauses that differ only
/// in their frames' names run the same statement texts.
struct Plan {
    tables: Vec<Table>,
    stages: Vec<(GrammarDialect, Prepared)>,
    clauses: Vec<(Prepared, Vec<CaptureColumn>)>,
    marks: usize,
    texts: usize,
    bytes: usize,
    steps: usize,
}

/// What a plan holds: its steps (markers and clears included), their distinct texts, and its
/// frame and B-tree names.
#[derive(Clone, Copy, Debug, Default)]
pub(in crate::code_rule_dsl) struct Census {
    pub(in crate::code_rule_dsl) steps: usize,
    pub(in crate::code_rule_dsl) texts: usize,
    pub(in crate::code_rule_dsl) frames: usize,
    pub(in crate::code_rule_dsl) btrees: usize,
    /// The distinct texts' bytes.
    pub(in crate::code_rule_dsl) bytes: usize,
}

/// A plan being made: each clause is planned as it is compiled, so its emitted statements are
/// freed before the next is emitted, and the stages, which clauses share, once every clause is.
/// The first error spoils the plan.
#[derive(Default)]
struct Planner {
    catalog: Catalog,
    texts: Texts,
    classes: HashMap<String, usize>,
    marks: usize,
    clauses: Vec<(Prepared, Vec<CaptureColumn>)>,
    error: Option<String>,
}

impl Planner {
    /// Plans the program's next clause, freeing its emitted statements unless it is to `keep`
    /// them.
    fn clause(&mut self, clause: &mut ClauseSql, keep: bool) {
        let index = self.clauses.len();
        match self.plan_clause(index, clause, keep) {
            Ok(prepared) => self.clauses.push((prepared, clause.captures.clone())),
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
    ) -> Result<Prepared, String> {
        let emitted = taken(&mut clause.sql, keep);
        if self.error.is_some() {
            return Err(String::new());
        }
        let each = format!("e{index}_");
        let enumeration = if clause.each {
            let made = [
                (
                    "x",
                    "file INTEGER, root INTEGER, start INTEGER, \"end\" INTEGER, PRIMARY KEY (file, root, start, \"end\")) WITHOUT ROWID",
                ),
                (
                    "a",
                    "file INTEGER, root INTEGER, PRIMARY KEY (file, root)) WITHOUT ROWID",
                ),
                ("r", "round INTEGER)"),
            ];
            let mut ids = [0; 3];
            let mut clear = Vec::with_capacity(3);
            for (at, (table, columns)) in made.into_iter().enumerate() {
                let name = format!("{each}{table}");
                ids[at] = self.catalog.add(
                    &name,
                    Make::BTree(format!("CREATE TEMP TABLE {name}({columns}")),
                )?;
                clear.push(self.texts.clear(ids[at], &name, false));
            }
            Some((
                ids,
                Each {
                    clear,
                    active: ids[1],
                    round: ids[2],
                    prefix: each.clone(),
                },
            ))
        } else {
            None
        };
        let mut prepared = plan(
            &mut self.catalog,
            &mut self.texts,
            &mut self.classes,
            &mut self.marks,
            &emitted,
            (&format!("c{index}_"), "", &each),
            run_id(None, index),
        )?;
        if let Some((ids, enumeration)) = enumeration {
            prepared.tables.extend(ids);
            prepared.each = Some(enumeration);
        }
        prepared.collect_texts();
        Ok(prepared)
    }

    /// The plan, once every clause is planned: with `stages`, whose emitted statements it frees
    /// unless it is to `keep` them.
    fn finish(mut self, stages: &mut [Stage], keep: bool) -> Result<Plan, String> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let mut planned_stages = Vec::with_capacity(stages.len());
        for (index, stage) in stages.iter_mut().enumerate() {
            let table = emit::stage_table(index);
            let id = self.catalog.stage(index)?;
            // A stage's frames live until the batch ends: its registers are its own.
            let namespace = emit::stage_namespace(index);
            let mut prepared = plan(
                &mut self.catalog,
                &mut self.texts,
                &mut self.classes,
                &mut self.marks,
                &taken(&mut stage.sql, keep),
                (&namespace, &namespace, ""),
                run_id(Some(index), 0),
            )?;
            prepared.tables.push(id);
            prepared.clear.push(self.texts.clear(id, &table, true));
            let columns = if stage.three {
                "file, node, unk"
            } else {
                "file, node"
            };
            let mut effect = prepared.last.effect.clone();
            effect.writes = Writes::Insert(id);
            prepared.last = self.texts.step(
                format!("INSERT INTO {table}({columns}) {}", prepared.last.sql),
                effect,
            );
            prepared.collect_texts();
            planned_stages.push((stage.dialect, prepared));
        }
        let steps = planned_stages
            .iter()
            .map(|(_, prepared)| prepared)
            .chain(self.clauses.iter().map(|(prepared, _)| prepared))
            .map(|prepared| {
                let blocks: usize = prepared.blocks.iter().map(|block| block.steps.len()).sum();
                let each = prepared.each.as_ref().map_or(0, |each| each.clear.len());
                prepared.steps.len() + blocks + 1 + prepared.clear.len() + each
            })
            .sum();
        Ok(Plan {
            tables: self.catalog.tables,
            stages: planned_stages,
            clauses: self.clauses,
            marks: self.marks,
            texts: self.texts.0.len(),
            bytes: self.texts.0.keys().map(|text| text.len()).sum(),
            steps,
        })
    }
}

impl Plan {
    fn census(&self) -> Census {
        let frames = self
            .tables
            .iter()
            .filter(|table| matches!(table.make, Make::Frame(_)))
            .count();
        Census {
            steps: self.steps,
            texts: self.texts,
            bytes: self.bytes,
            frames,
            btrees: self.tables.len() - frames,
        }
    }

    /// The stage (`index` below the stages' count) or clause of this index.
    fn prepared(&self, index: usize) -> &Prepared {
        match index.checked_sub(self.stages.len()) {
            None => &self.stages[index].1,
            Some(clause) => &self.clauses[clause].0,
        }
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
    texts: &mut Texts,
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
            let inner = extract_blocks(steps, blocks, named, marks, texts);
            blocks[index].size = size(&inner, blocks);
            blocks[index].steps = inner;
            named.insert(id, index);
            out.push(texts.run(index));
        } else if let Some(id) = step.sql.strip_prefix(emit::RUN) {
            let index = *named.get(id).expect("a block emitted before it runs");
            out.push(texts.run(index));
        } else if &*step.sql == emit::END {
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
/// unextended; and where each `emit::RUN` of a block that mentions it is, and the outermost loop
/// it is in.
#[derive(Default)]
struct Uses {
    span: Option<(usize, usize)>,
    declared: usize,
    cleared: Option<usize>,
    mentions: Option<(usize, usize)>,
    runs: Vec<(usize, (usize, usize))>,
}

/// Where a temporary is live (`Uses::live`): one span, and the runs of its blocks, which write it
/// anew there.
struct Live {
    span: (usize, usize),
    runs: Vec<usize>,
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
    /// block's runs, where it is written anew (`Live::runs`). Any other temporary is live over
    /// its mentions' span extended to their loops, and from there to its blocks' last run.
    fn live(self, inner: &[Option<(usize, usize)>]) -> Live {
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
                runs: self.runs.into_iter().map(|(at, _)| at).collect(),
            },
            None => Live {
                span: self
                    .runs
                    .iter()
                    .fold(span, |(start, end), &(_, (from, to))| {
                        (start.min(from), end.max(to))
                    }),
                runs: Vec::new(),
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
        if item == emit::LOOP {
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
    // The temporaries each block mentions, nested blocks' included: the open blocks' so far,
    // innermost last, and the closed blocks', by id.
    let mut blocks: HashMap<&str, Vec<usize>> = HashMap::new();
    let mut inside: Vec<(&str, Vec<usize>)> = Vec::new();
    let mut mentioned = Vec::new();
    for (at, item) in items.iter().enumerate() {
        mentioned.clear();
        if let Some(id) = item.strip_prefix(emit::BLOCK) {
            inside.push((id, Vec::new()));
            continue;
        }
        if item == emit::END {
            let (id, mut temporaries) = inside.pop().expect("an open block");
            temporaries.sort_unstable();
            temporaries.dedup();
            if let Some((_, outer)) = inside.last_mut() {
                outer.extend_from_slice(&temporaries);
            }
            blocks.insert(id, temporaries);
            continue;
        }
        let span = outer[at].unwrap_or((at, at));
        if let Some(id) = item.strip_prefix(emit::RUN) {
            mentioned.extend(blocks.get(id).into_iter().flatten().copied());
            for &temporary in &mentioned {
                uses[temporary].runs.push((at, span));
            }
        } else {
            for (found, _) in item.match_indices(emit::TEMP) {
                let name = temporary(&item[found + emit::TEMP.len()..]);
                mentioned.extend(temporaries.get(name).copied());
            }
            for &temporary in &mentioned {
                uses[temporary].mention(item, at, span);
            }
        }
        if let Some((_, block)) = inside.last_mut() {
            block.extend_from_slice(&mentioned);
        }
    }
    let live: Vec<Live> = uses.into_iter().map(|uses| uses.live(&inner)).collect();
    let mut order: Vec<usize> = (0..names.len()).collect();
    order.sort_by_key(|&temporary| (live[temporary].span.0, temporary));
    // Per class, each register's spans held, by start, and where blocks' runs write it.
    let mut held: HashMap<usize, Vec<(BTreeMap<usize, usize>, BTreeSet<usize>)>> = HashMap::new();
    let mut allocated = HashMap::with_capacity(names.len());
    let mut clears = Vec::new();
    for temporary in order {
        let class = class_of[temporary];
        let Live {
            span: (start, end),
            ref runs,
        } = live[temporary];
        let registers = held.entry(class).or_default();
        // The span overlaps one held where the last held span starting before its end ends at
        // its start or after: held spans do not overlap.
        let free = |(spans, written): &(BTreeMap<usize, usize>, BTreeSet<usize>)| {
            spans
                .range(..=end)
                .next_back()
                .is_none_or(|(_, &until)| until < start)
                && written.range(start..=end).next().is_none()
        };
        let register = registers.iter().position(free).unwrap_or_else(|| {
            registers.push(Default::default());
            registers.len() - 1
        });
        let (spans, written) = &mut registers[register];
        let reused = !spans.is_empty();
        spans.insert(start, end);
        written.extend(runs.iter().copied());
        let name = format!("{prefix}f{class}_{register}");
        if reused {
            clears.push((start, name.clone()));
        }
        allocated.insert(names[temporary].to_owned(), name);
    }
    (allocated, clears)
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

/// `sql` with every `(from, to)` substituted; `sql` itself where it holds none.
fn substitute(sql: &str, substitutions: &[(&str, &str)]) -> String {
    let mut sql = sql.to_owned();
    for (from, to) in substitutions {
        if sql.contains(from) {
            sql = sql.replace(from, to);
        }
    }
    sql
}

/// Fixes the text of `emitted`'s statements, loops' markers included (`Runner::steps`), with
/// their effects over the counted tables of `catalog`, to which it adds the tables they make: a
/// table its statements only append to and empty whole is a frame (`frames::rewrite`), any other
/// a B-tree. An operator's temporary that is a frame is a register (`registers`), emptied before
/// a later temporary takes it. The statements read their root frame's files and roots under the
/// run id `run`; `names` is the prefixes of their own tables (`emit::NAMESPACE`), of their
/// registers, and of an enumeration's tables (`emit::EACH`).
fn plan(
    catalog: &mut Catalog,
    texts: &mut Texts,
    classes: &mut HashMap<String, usize>,
    marks: &mut usize,
    emitted: &Emitted,
    (namespace, prefix, each): (&str, &str, &str),
    run: i64,
) -> Result<Prepared, String> {
    let files = format!("SELECT file FROM runs WHERE run = {run}");
    let roots = format!("SELECT file, node FROM roots WHERE run = {run}");
    let substitutions = [
        (emit::NAMESPACE, namespace),
        (emit::EACH, each),
        (emit::FILES, files.as_str()),
        (emit::ROOTS, roots.as_str()),
    ];
    let statements: Vec<String> = emitted
        .statements
        .iter()
        .map(|statement| substitute(statement, &substitutions))
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
        if !sql.contains(emit::TEMP) {
            return sql;
        }
        let mut out = String::with_capacity(sql.len());
        let mut rest = sql.as_str();
        while let Some(at) = rest.find(emit::TEMP) {
            out.push_str(&rest[..at]);
            let after = &rest[at + emit::TEMP.len()..];
            let name = temporary(after);
            temporary_name(name, &mut out);
            rest = &after[name.len()..];
        }
        out.push_str(rest);
        out
    };
    let mut clears = clears.into_iter().peekable();
    let mut steps: Vec<(String, Effect)> = Vec::new();
    let mut tables = Vec::new();
    let mut made = HashSet::new();
    let mut clear = Vec::new();
    let mut stages = Vec::new();
    for (at, sql) in items.into_iter().enumerate() {
        while let Some((_, register)) = clears.next_if(|(position, _)| *position == at) {
            steps.push((format!("{}{register}", frames::CLEAR), clearing(register)));
        }
        let sql = rename(sql);
        stages_read(&sql, &mut stages);
        if sql.starts_with("CREATE INDEX ") {
            catalog.index(&sql)?;
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
            steps.push((
                sql,
                effects.next().expect("an effect per statement").clone(),
            ));
            continue;
        };
        let frame = matches!(make, Make::Frame(_));
        let id = catalog.add(&name, make)?;
        if made.insert(id) {
            tables.push(id);
            clear.push(texts.clear(id, &name, frame));
        }
    }
    assert!(effects.next().is_none(), "a statement per effect");
    // The stages' tables the statements read: a clause is planned before the stages.
    for &stage in &stages {
        catalog.stage(stage)?;
    }
    // A table's name in an effect, as the statements name it.
    let resolve = |name: &str| -> String {
        match name.strip_prefix(emit::TEMP) {
            Some(name) => {
                let mut out = String::new();
                temporary_name(name, &mut out);
                out
            }
            None => substitute(name, &substitutions[..2]),
        }
    };
    // Once every table is added: a block's marker names its units' demand tables, which the
    // loop creates after it.
    let steps: Vec<Step> = steps
        .into_iter()
        .map(|(sql, effect)| {
            let effect = effect.map(|table| catalog.id(&resolve(&table)));
            texts.step(sql, effect)
        })
        .collect();
    let mut blocks = Vec::new();
    let mut steps = extract_blocks(
        &mut steps.into_iter(),
        &mut blocks,
        &mut HashMap::new(),
        marks,
        texts,
    );
    let last = steps.pop().expect("a final select");
    let root = emitted.root.as_ref().map(|root| {
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
    Ok(Prepared {
        steps,
        blocks,
        last,
        clear,
        stages,
        each: None,
        root,
        tables,
        texts: Vec::new(),
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
/// The statements every batch runs besides its stages' and clauses'.
const BATCH_STATEMENTS: usize = 4;

/// A connection running one compiled program's batches (`Plan`). A session starts with only the
/// tables of the batch's own rows, `runs` and `roots`: the files and roots a statement's root
/// frame reads, under the statement's run id. A stage's or clause's tables are made on its first
/// run (`activate`), and its statements prepared on their first execution, into a cache sized
/// for the distinct texts of the stages and clauses that ran. The session keeps them across
/// batches: a batch empties its tables (`frames`: the frame tables' buffers) and refills them
/// through the cached statements. `counts`: every counted table's rows (`skip`).
pub(in crate::code_rule_dsl) struct Session {
    connection: Connection,
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
}

impl Session {
    fn new(compiled: &Compiled) -> Result<Self, SqlError> {
        let started = std::time::Instant::now();
        let plan = compiled.plan()?;
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
        connection.set_prepared_statement_cache_capacity(BATCH_STATEMENTS);
        let names = plan
            .tables
            .iter()
            .map(|table| (table.name.clone(), matches!(table.make, Make::Frame(_))))
            .collect();
        Ok(Self {
            connection,
            slot,
            program: Arc::clone(&compiled.program),
            counts: Counts::new(Arc::clone(&frames), names, plan.marks),
            made: vec![false; plan.tables.len()],
            active: vec![false; plan.stages.len() + plan.clauses.len()],
            cached: vec![false; plan.texts],
            capacity: BATCH_STATEMENTS,
            stats: Stats {
                sessions: 1,
                setup_nanos: started.elapsed().as_nanos() as u64,
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
        let started = std::time::Instant::now();
        let plan = Arc::clone(&self.plan);
        let capacity = self.capacity;
        while let Some(index) = pending.pop() {
            if std::mem::replace(&mut self.active[index], true) {
                continue;
            }
            let prepared = plan.prepared(index);
            pending.extend(prepared.stages.iter().copied());
            for &table in &prepared.tables {
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
            for &text in &prepared.texts {
                if !std::mem::replace(&mut self.cached[text], true) {
                    self.capacity += 1;
                }
            }
            if prepared.each.is_some() {
                self.capacity += 3;
            }
        }
        if self.capacity > capacity {
            self.connection
                .set_prepared_statement_cache_capacity(self.capacity);
            self.stats.texts += self.capacity - capacity;
        }
        self.stats.setup_nanos += started.elapsed().as_nanos() as u64;
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
        self.activate((0..self.active.len()).collect())?;
        let mut plans = Vec::new();
        let prepared = self
            .plan
            .stages
            .iter()
            .map(|(_, prepared)| prepared)
            .chain(self.plan.clauses.iter().map(|(prepared, _)| prepared));
        for prepared in prepared {
            let blocks = prepared.blocks.iter().flat_map(|block| &block.steps);
            for step in prepared.steps.iter().chain(blocks).chain([&prepared.last]) {
                let sql: &str = &step.sql;
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
        let stages = self.plan.stages.len();
        self.activate(
            runs.iter()
                .filter(|run| !run.files.is_empty())
                .map(|run| stages + run.statement)
                .collect(),
        )?;
        let started = std::time::Instant::now();
        let batch = Batch::of(docs, Arc::clone(&self.program));
        self.stats.facts_nanos += started.elapsed().as_nanos() as u64;
        *self.slot.current.write() = Some(Arc::clone(&batch));
        let scopes = BatchScopes::begin();
        let result = self.run_batch(&batch, runs);
        drop(scopes);
        *self.slot.current.write() = None;
        if result.is_err() {
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
            for clear in [CLEAR_RUNS, CLEAR_ROOTS] {
                self.connection.execute(clear, [])?;
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
            plan,
            stats,
            frames,
            counts,
            ..
        } = self;
        let (stages, clauses) = (&plan.stages, &plan.clauses);
        let connection = &*connection;
        let mut runner = Runner {
            connection,
            frames,
            stats,
            counts,
            at: ("", 0),
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
            runner.at = ("stage", stage);
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
                runner.at = ("clause", run.statement);
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
                        #[cfg(any(test, feature = "unbounded-work"))]
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
                runner.at = ("stage", stage);
                runner.clear(prepared)?;
            }
        }
        runner.stats.input_nanos += started.elapsed().as_nanos() as u64;
        Ok(results)
    }
}

/// A session's connection, frames, statistics and counts while a batch runs its statements;
/// `at`, the stage or clause running (`verify`).
struct Runner<'s> {
    connection: &'s Connection,
    frames: &'s FrameStore,
    stats: &'s mut Stats,
    counts: &'s mut Counts,
    at: (&'static str, usize),
}

impl Runner<'_> {
    /// `RERAIL_SQL_SKIP=check`: panics where a table of `tables` holds other than its count,
    /// before `sql` decides anything by it.
    fn verify(&self, tables: &[usize], sql: &str) -> Result<(), SqlError> {
        if *MODE != Mode::Check {
            return Ok(());
        }
        for &table in tables {
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

    /// Executes a stage's or clause's statements, each loop (`emit::LOOP` … `emit::UNTIL`) while
    /// its last query counts a dirty key (none: not at all), and accepting a loop's last pass by its reads
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
            let sql: &str = &steps[at].sql;
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
                    if &*steps[index].sql == emit::LOOP {
                        depth += 1;
                    } else if steps[index].sql.starts_with(emit::UNTIL) {
                        depth -= 1;
                    }
                    depth == 0
                })
                .expect("a loop's end");
            let until = &steps[end].sql[emit::UNTIL.len()..];
            // A loop's pass evaluates the dirty keys of its units: with none (the site demanded
            // no new key), it would change nothing, and the units it reaches would be run for
            // nothing again, at every site of every pass above them.
            let dirty = |connection: &Connection| -> Result<bool, SqlError> {
                let changed: i64 = connection
                    .prepare_cached(until)?
                    .query_row([], |row| row.get(0))?;
                Ok(changed != 0)
            };
            if !dirty(self.connection)? {
                at = end + 1;
                continue;
            }
            let mut passes = 0;
            // Mutant 11: the loop's last pass is undone.
            let drop_last = super::mutant(11);
            loop {
                if drop_last {
                    self.connection.execute_batch("SAVEPOINT mutant_pass")?;
                }
                self.steps(&steps[at + 1..end], blocks, known_empty)?;
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
        self.verify(&block.demand, "a block's run")?;
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
        self.verify(&step.effect.inputs, &step.sql)?;
        if let Writes::Insert(table) | Writes::Delete(table) = step.effect.writes {
            self.verify(&[table], &step.sql)?;
        }
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
        self.verify(&clear.effect.inputs, &clear.sql)?;
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
