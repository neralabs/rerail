//! SQLite table-valued boundary for batched terminal execution.
//!
//! A batch is a set of file versions and a set of terminals. SQL remains the outer rule engine:
//! it joins terminal rows with files, nodes and facts, and no scalar function runs a matcher for
//! each joined row.
//!
//! Two access paths share one table:
//! - With `program_index` (the terminal), and optionally `file_index` (equality or an `IN` set)
//!   and `start_ordinal`, the table executes that terminal over every file of the batch the first
//!   time any row of it is asked for, caches the rows, and answers every later call, however many
//!   joined rows ask, from that cache. SQLite's nested loops call a table once per outer row; the
//!   cache is what keeps that from being a terminal execution per row.
//! - `rerail_matches(batch)` alone executes every file/program pair once and returns all match,
//!   capture and completion rows: the transducer batch this module started as. Only a batch of
//!   transducer programs, which tests build, has it; production statements always name the
//!   terminal, and without one the table fails.

#[cfg(test)]
use super::{Error, Output, Program, execute_batch};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::tape::TapeDoc;
#[cfg(test)]
use ast_grep_core::matcher::KindMatcher;
use ast_grep_core::matcher::{Matcher, MatcherExt, Pattern};
use ast_grep_core::meta_var::MetaVariable;
use ast_grep_core::{AstGrep as Root, Doc, Node};
use foldhash::{HashMap, HashMapExt};
use rayon::prelude::*;
use regex::Regex;
use rusqlite::fallible_iterator::FallibleIterator;
use rusqlite::ffi;
use rusqlite::vtab::{
    Context, Filters, IndexConstraintOp, IndexInfo, Module, VTab, VTabConfig, VTabConnection,
    VTabCursor,
};
use rusqlite::{Connection, Result as SqlResult};
use std::any::Any;
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::ffi::{CStr, c_int};
use std::marker::PhantomData;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

const MODULE_NAME: &CStr = c"rerail_matches";
const BATCH_COLUMN: c_int = 0;
const FILE_COLUMN: c_int = 1;
const PROGRAM_COLUMN: c_int = 2;
const START_COLUMN: c_int = 4;

/// `idx_num` bits: which constraints `best_index` consumed, in argv order after the batch.
const USE_BATCH: c_int = 1;
const USE_PROGRAM: c_int = 2;
const USE_FILE_EQ: c_int = 4;
const USE_FILE_IN: c_int = 8;
const USE_START: c_int = 16;

const NEEDS_PROGRAMS: &str = "rerail_matches without a program constraint needs programs only";

#[cfg(test)]
thread_local! {
    static TEXT_BATCH_SOURCES: std::cell::RefCell<Vec<*const String>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The source allocations text batches registered on this thread since the last call read, in
/// registration order; the call forgets them.
#[cfg(test)]
#[allow(dead_code, reason = "retained for SQL allocation profiling")]
pub(in crate::code_rule_dsl) fn take_text_batch_sources() -> Vec<*const String> {
    TEXT_BATCH_SOURCES.take()
}

#[derive(Clone)]
struct MatchRow {
    batch: i64,
    file: i64,
    program: i64,
    match_index: Option<i64>,
    start_ordinal: Option<i64>,
    output_label: Option<i64>,
    capture_index: Option<i64>,
    capture_label: Option<i64>,
    capture_node: Option<i64>,
    start_byte: Option<i64>,
    end_byte: Option<i64>,
    complete: i64,
    error_code: Option<String>,
}

/// A node-set producer SQL cannot compute itself: an ast-grep pattern or a text regex (and, for
/// tests of the virtual table, a transducer program or node kind). Text terminals return source
/// byte spans in match order.
pub(in crate::code_rule_dsl) enum Terminal {
    #[cfg(test)]
    #[allow(dead_code, reason = "retained for direct virtual-table tests")]
    Program(Program),
    /// An ast-grep pattern of one grammar, matched at every node of a kind it can match (all nodes
    /// when it can match any kind), in a fresh environment. Captures are its named metavariables.
    /// Files of other grammars have no matches.
    Pattern(Arc<PatternTerminal>),
    /// An ast-grep kind rule of one grammar.
    #[cfg(test)]
    Kind(KindMatcher, GrammarDialect),
    /// A bounded regex over source content or the host path. Text terminals deliberately need no
    /// tape: a file-only code_rule must not parse merely to scan text.
    Text(Regex, TextSource, usize),
}

/// A pattern terminal: an ast-grep pattern of one grammar and the captures it reports, derived
/// once when the terminal is built.
pub(in crate::code_rule_dsl) struct PatternTerminal {
    pub(in crate::code_rule_dsl) pattern: Pattern,
    language: GrammarDialect,
    /// Capture names in label order.
    pub(in crate::code_rule_dsl) labels: Vec<Arc<str>>,
    /// Which labels are multi-node (`$$$`) captures.
    pub(in crate::code_rule_dsl) multi: Vec<bool>,
}

impl PatternTerminal {
    pub(in crate::code_rule_dsl) fn new(pattern: Pattern, language: GrammarDialect) -> Self {
        let labels = pattern_labels(&pattern);
        Self {
            multi: labels
                .iter()
                .map(|label| pattern_is_multi(&pattern, label))
                .collect(),
            labels: labels.into_iter().map(Arc::from).collect(),
            pattern,
            language,
        }
    }

    /// Whether the pattern can match nodes of `file`: a taped file of its grammar.
    fn reads(&self, file: &BatchFile) -> bool {
        file.root.is_some() && *file.tape().get_lang() == self.language
    }

    /// The pattern's match at `node` of a file it reads, in a fresh environment.
    fn match_at(&self, file: &BatchFile, node: u32) -> Option<TerminalMatch> {
        let matched = self.pattern.match_node(file.node(node))?;
        let env = matched.get_env();
        let captures = self
            .labels
            .iter()
            .zip(&self.multi)
            .enumerate()
            .filter_map(|(label, (name, &multi))| {
                let nodes = if multi {
                    env.get_multiple_matches(name)
                        .iter()
                        .map(|node| node.node_id() as u32)
                        .collect()
                } else {
                    vec![env.get_match(name)?.node_id() as u32]
                };
                Some((label as u16, nodes))
            })
            .collect();
        Some(TerminalMatch {
            node,
            output_label: 0,
            span: None,
            captures,
        })
    }
}

#[derive(Clone, Copy)]
pub(in crate::code_rule_dsl) enum TextSource {
    Content,
    Path,
}

/// One file version of a batch. Text-only batches have no tape; AST terminals see an empty tree.
pub(in crate::code_rule_dsl) struct BatchFile {
    root: Option<Root<TapeDoc>>,
    text: Arc<String>,
    pub(in crate::code_rule_dsl) path: Arc<str>,
    pub(in crate::code_rule_dsl) language: Arc<str>,
    pub(in crate::code_rule_dsl) module: Option<Arc<str>>,
    /// Preorder ordinals per kind id, built on first use.
    kinds: OnceLock<HashMap<u16, Vec<u32>>>,
    /// A full AST terminal scan of this file visits at most this many nodes (`bounded`).
    node_limit: usize,
    /// The file evaluation's budget, retained across preparation and batched execution.
    deadline: Option<crate::code_rule_dsl::budget::MatchDeadline>,
}

impl BatchFile {
    pub(in crate::code_rule_dsl) fn new(
        tape: TapeDoc,
        path: Arc<str>,
        language: Arc<str>,
        module: Option<Arc<str>>,
    ) -> Self {
        Self {
            root: Some(Root::doc(tape)),
            text: Arc::new(String::new()),
            path,
            language,
            module,
            kinds: OnceLock::new(),
            node_limit: usize::MAX,
            deadline: crate::code_rule_dsl::budget::current_deadline(),
        }
    }

    /// A text-only file over the caller's source allocation, which the batch shares.
    pub(in crate::code_rule_dsl) fn text_shared(
        text: Arc<String>,
        path: Arc<str>,
        language: Arc<str>,
        module: Option<Arc<str>>,
    ) -> Self {
        #[cfg(test)]
        TEXT_BATCH_SOURCES.with_borrow_mut(|sources| sources.push(Arc::as_ptr(&text)));
        Self {
            root: None,
            text,
            path,
            language,
            module,
            kinds: OnceLock::new(),
            node_limit: usize::MAX,
            deadline: crate::code_rule_dsl::budget::current_deadline(),
        }
    }

    /// Bounds this file's full AST terminal scans to `node_limit` visited nodes (the largest
    /// `maxVisitedNodes` of the clauses reading it): past it a terminal stops, its rows incomplete.
    pub(in crate::code_rule_dsl) fn bounded(mut self, node_limit: usize) -> Self {
        self.node_limit = node_limit;
        self
    }

    pub(in crate::code_rule_dsl) fn tape(&self) -> &TapeDoc {
        self.root
            .as_ref()
            .expect("AST terminal cannot read a text-only batch")
            .root()
            .get_doc()
    }

    /// The node at a preorder ordinal.
    pub(in crate::code_rule_dsl) fn node(&self, ordinal: u32) -> Node<'_, TapeDoc> {
        self.root
            .as_ref()
            .expect("AST terminal cannot read a text-only batch")
            .adopt(self.tape().node(ordinal))
    }

    pub(in crate::code_rule_dsl) fn len(&self) -> usize {
        self.root
            .as_ref()
            .map_or(0, |_| self.tape().transducer_len())
    }

    /// The nodes of one kind id, in preorder (none in a text-only file, which has no tree).
    pub(in crate::code_rule_dsl) fn nodes_of_kind(&self, kind: u16) -> &[u32] {
        let Some(root) = &self.root else {
            return &[];
        };
        self.kinds
            .get_or_init(|| {
                let tape = root.root().get_doc();
                let mut kinds: HashMap<u16, Vec<u32>> = HashMap::new();
                for node in 0..tape.transducer_len() as u32 {
                    kinds
                        .entry(tape.transducer_kind(node))
                        .or_default()
                        .push(node);
                }
                kinds
            })
            .get(&kind)
            .map_or(&[], Vec::as_slice)
    }
}

/// One match of a terminal: its node and its captures, `(label, nodes)`, where a single-node
/// capture holds one node and a multi-node capture (`$$$`) any number. Text terminals carry their
/// byte span and use no node.
pub(in crate::code_rule_dsl) struct TerminalMatch {
    pub(in crate::code_rule_dsl) node: u32,
    pub(in crate::code_rule_dsl) output_label: u16,
    pub(in crate::code_rule_dsl) span: Option<(usize, usize)>,
    pub(in crate::code_rule_dsl) captures: Vec<(u16, Vec<u32>)>,
}

/// A terminal's rows for one file. `complete` is false when the terminal could not finish the
/// file; its matches are then a subset and never prove absence.
pub(in crate::code_rule_dsl) struct FileMatches {
    pub(in crate::code_rule_dsl) matches: Vec<TerminalMatch>,
    pub(in crate::code_rule_dsl) complete: bool,
    pub(in crate::code_rule_dsl) error: Option<String>,
}

#[cfg(test)]
thread_local! {
    static PATTERN_LABEL_DERIVATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Pattern capture-label sets derived on this thread.
#[cfg(test)]
pub(in crate::code_rule_dsl) fn pattern_label_derivations() -> usize {
    PATTERN_LABEL_DERIVATIONS.get()
}

/// The capture labels a pattern reports, in label order.
fn pattern_labels(pattern: &Pattern) -> Vec<String> {
    #[cfg(test)]
    PATTERN_LABEL_DERIVATIONS.set(PATTERN_LABEL_DERIVATIONS.get() + 1);
    pattern
        .defined_vars()
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// A terminal's scalar lookups (`MatchBatch::terminal_at`) by `(file, node)`.
type NodeMatches = Mutex<HashMap<(usize, u32), Option<Arc<TerminalMatch>>>>;

struct TerminalCell {
    rows: OnceLock<Arc<Vec<FileMatches>>>,
    nodes: NodeMatches,
    executions: AtomicUsize,
}

struct MatchBatch {
    token: i64,
    terminals: Arc<Vec<Terminal>>,
    cells: Vec<TerminalCell>,
    files: Vec<BatchFile>,
    deadline: Option<crate::code_rule_dsl::budget::MatchDeadline>,
    /// The transducer batch's rows (`MatchBatch::rows`).
    #[cfg(test)]
    rows: OnceLock<Result<Arc<Vec<MatchRow>>, String>>,
    #[cfg(test)]
    executions: AtomicUsize,
    filters: AtomicUsize,
    #[cfg(test)]
    pattern_match_attempts: AtomicUsize,
    /// State the batch's owner attaches (the query engine's plan and bindings).
    extension: Option<Arc<dyn Any + Send + Sync>>,
}

impl MatchBatch {
    #[cfg(test)]
    fn programs(&self) -> Result<Vec<Program>, String> {
        self.terminals
            .iter()
            .map(|terminal| match terminal {
                Terminal::Program(program) => Ok(program.clone()),
                _ => Err(NEEDS_PROGRAMS.into()),
            })
            .collect()
    }

    #[cfg(test)]
    fn tapes(&self) -> Vec<TapeDoc> {
        self.files
            .iter()
            .filter(|file| file.root.is_some())
            .map(|file| file.tape().clone())
            .collect()
    }

    /// Every file/program pair in one pass: the transducer batch.
    #[cfg(test)]
    fn rows(&self) -> Result<Arc<Vec<MatchRow>>, String> {
        self.rows
            .get_or_init(|| {
                self.executions.fetch_add(1, Ordering::Relaxed);
                let programs = self.programs()?;
                let files = self.tapes();
                let result = execute_batch(&programs, &files, None);
                match result {
                    Ok(outputs) => Ok(Arc::new(self.output_rows(outputs))),
                    Err(error) => Ok(Arc::new(self.failure_rows(error))),
                }
            })
            .clone()
    }

    /// One terminal over every file of the batch, executed once and cached.
    fn terminal(&self, index: usize) -> Arc<Vec<FileMatches>> {
        let cell = &self.cells[index];
        Arc::clone(cell.rows.get_or_init(|| {
            cell.executions.fetch_add(1, Ordering::Relaxed);
            let terminal = &self.terminals[index];
            let batch_deadline = self.deadline.clone();
            let execute = |batch_file: &BatchFile| {
                execute_terminal_with_budget(
                    terminal,
                    batch_file,
                    || {
                        batch_file
                            .deadline
                            .as_ref()
                            .or(batch_deadline.as_ref())
                            .is_some_and(crate::code_rule_dsl::budget::MatchDeadline::exhausted)
                    },
                    #[cfg(test)]
                    &self.pattern_match_attempts,
                )
            };
            // Text scans are short and already run inside SQLite's worker. Keeping them on that
            // worker avoids nested Rayon dispatch for large all-file batches.
            let rows = if matches!(terminal, Terminal::Text(..)) {
                self.files.iter().map(execute).collect()
            } else {
                self.files.par_iter().map(execute).collect()
            };
            Arc::new(rows)
        }))
    }

    fn terminal_at(
        &self,
        index: usize,
        file_index: usize,
        node: u32,
    ) -> Option<Arc<TerminalMatch>> {
        let cell = self.cells.get(index)?;
        let Terminal::Pattern(pattern) = &self.terminals[index] else {
            return None;
        };
        let file = self.files.get(file_index)?;
        if !pattern.reads(file) || node as usize >= file.len() {
            return None;
        }
        let mut cached = cell
            .nodes
            .lock()
            .expect("terminal node cache mutex poisoned");
        if let Some(found) = cached.get(&(file_index, node)) {
            return found.clone();
        }
        #[cfg(test)]
        self.pattern_match_attempts.fetch_add(1, Ordering::Relaxed);
        let found = pattern.match_at(file, node).map(Arc::new);
        cached.insert((file_index, node), found.clone());
        found
    }

    #[cfg(test)]
    fn output_rows(&self, outputs: Vec<Vec<Vec<Output>>>) -> Vec<MatchRow> {
        let mut rows = Vec::new();
        for (file, programs) in outputs.into_iter().enumerate() {
            for (program, matches) in programs.into_iter().enumerate() {
                if matches.is_empty() {
                    rows.push(self.status_row(file, program, true, None));
                    continue;
                }
                for (match_index, output) in matches.into_iter().enumerate() {
                    if output.captures.is_empty() {
                        rows.push(MatchRow {
                            batch: self.token,
                            file: file as i64,
                            program: program as i64,
                            match_index: Some(match_index as i64),
                            start_ordinal: Some(i64::from(output.start)),
                            output_label: Some(i64::from(output.label)),
                            capture_index: None,
                            capture_label: None,
                            capture_node: None,
                            start_byte: None,
                            end_byte: None,
                            complete: 1,
                            error_code: None,
                        });
                        continue;
                    }
                    for (capture_index, capture) in output.captures.into_iter().enumerate() {
                        rows.push(MatchRow {
                            batch: self.token,
                            file: file as i64,
                            program: program as i64,
                            match_index: Some(match_index as i64),
                            start_ordinal: Some(i64::from(output.start)),
                            output_label: Some(i64::from(output.label)),
                            capture_index: Some(capture_index as i64),
                            capture_label: Some(i64::from(capture.label)),
                            capture_node: Some(i64::from(capture.node)),
                            start_byte: Some(capture.span.start as i64),
                            end_byte: Some(capture.span.end as i64),
                            complete: 1,
                            error_code: None,
                        });
                    }
                }
            }
        }
        rows
    }

    #[cfg(test)]
    fn failure_rows(&self, error: Error) -> Vec<MatchRow> {
        let code = format!("{error:?}");
        (0..self.files.len())
            .flat_map(|file| {
                let code = code.clone();
                (0..self.terminals.len())
                    .map(move |program| self.status_row(file, program, false, Some(code.clone())))
            })
            .collect()
    }

    fn status_row(
        &self,
        file: usize,
        program: usize,
        complete: bool,
        error_code: Option<String>,
    ) -> MatchRow {
        MatchRow {
            batch: self.token,
            file: file as i64,
            program: program as i64,
            match_index: None,
            start_ordinal: None,
            output_label: None,
            capture_index: None,
            capture_label: None,
            capture_node: None,
            start_byte: None,
            end_byte: None,
            complete: i64::from(complete),
            error_code,
        }
    }

    /// The rows of one terminal for the requested files (all when `files` is `None`), optionally
    /// only the match at `start`. Empty and incomplete file/program pairs contribute a status row.
    fn terminal_rows(
        &self,
        program: usize,
        files: Option<&[usize]>,
        start: Option<u32>,
    ) -> Vec<MatchRow> {
        let cached = self.terminal(program);
        let mut rows = Vec::new();
        let mut emit = |file: usize| {
            let Some(found) = cached.get(file) else {
                return;
            };
            let selected: Box<dyn Iterator<Item = (usize, &TerminalMatch)>> = match start {
                Some(node) => Box::new(
                    found
                        .matches
                        .binary_search_by_key(&node, |found| found.node)
                        .ok()
                        .map(|index| (index, &found.matches[index]))
                        .into_iter(),
                ),
                None => Box::new(found.matches.iter().enumerate()),
            };
            for (match_index, found_match) in selected {
                let (start_byte, end_byte) = found_match.span.unwrap_or_else(|| {
                    let span = self.files[file].tape().transducer_span(found_match.node);
                    (span.start, span.end)
                });
                let base = MatchRow {
                    batch: self.token,
                    file: file as i64,
                    program: program as i64,
                    match_index: Some(match_index as i64),
                    start_ordinal: Some(i64::from(found_match.node)),
                    output_label: Some(i64::from(found_match.output_label)),
                    capture_index: None,
                    capture_label: None,
                    capture_node: None,
                    start_byte: Some(start_byte as i64),
                    end_byte: Some(end_byte as i64),
                    complete: 1,
                    error_code: None,
                };
                let mut capture_index = 0;
                for (label, nodes) in &found_match.captures {
                    for &node in nodes {
                        let span = self.files[file].tape().transducer_span(node);
                        rows.push(MatchRow {
                            capture_index: Some(capture_index),
                            capture_label: Some(i64::from(*label)),
                            capture_node: Some(i64::from(node)),
                            start_byte: Some(span.start as i64),
                            end_byte: Some(span.end as i64),
                            ..base.clone()
                        });
                        capture_index += 1;
                    }
                }
                if capture_index == 0 {
                    rows.push(base);
                }
            }
            if start.is_none() && (found.matches.is_empty() || !found.complete) {
                rows.push(MatchRow {
                    error_code: found.error.clone(),
                    ..self.status_row(file, program, found.complete, None)
                });
            }
        };
        match files {
            Some(files) => files.iter().copied().for_each(&mut emit),
            None => (0..self.files.len()).for_each(emit),
        }
        rows
    }
}

/// Runs one terminal over one file, polling `exhausted` while materializing matches; AST scans
/// also stop at the file's node budget (`BatchFile::bounded`).
fn execute_terminal_with_budget(
    terminal: &Terminal,
    file: &BatchFile,
    exhausted: impl Fn() -> bool,
    #[cfg(test)] pattern_match_attempts: &AtomicUsize,
) -> FileMatches {
    if let Terminal::Text(regex, source, limit) = terminal {
        let haystack: &str = match source {
            TextSource::Content => &file.text,
            TextSource::Path => &file.path,
        };
        let mut matches = Vec::new();
        for found in regex.find_iter(haystack) {
            if exhausted() || matches.len() >= *limit {
                return FileMatches {
                    matches,
                    complete: false,
                    error: Some("budget-exceeded".to_owned()),
                };
            }
            matches.push(TerminalMatch {
                node: matches.len() as u32,
                output_label: 0,
                span: Some((found.start(), found.end())),
                captures: Vec::new(),
            });
        }
        return FileMatches {
            matches,
            complete: true,
            error: None,
        };
    }
    if file.root.is_none() {
        return FileMatches {
            matches: Vec::new(),
            complete: true,
            error: None,
        };
    }
    let foreign = match terminal {
        #[cfg(test)]
        Terminal::Program(_) => false,
        Terminal::Pattern(pattern) => !pattern.reads(file),
        #[cfg(test)]
        Terminal::Kind(_, language) => file.tape().get_lang() != language,
        Terminal::Text(..) => unreachable!(),
    };
    if foreign {
        return FileMatches {
            matches: Vec::new(),
            complete: true,
            error: None,
        };
    }
    match terminal {
        #[cfg(test)]
        Terminal::Program(program) => {
            let outputs = program.execute(file.tape());
            match outputs {
                Ok(outputs) => FileMatches {
                    matches: outputs
                        .into_iter()
                        .map(|output| TerminalMatch {
                            node: output.start,
                            output_label: output.label,
                            span: None,
                            captures: output
                                .captures
                                .into_iter()
                                .map(|capture| (capture.label, vec![capture.node]))
                                .collect(),
                        })
                        .collect(),
                    complete: true,
                    error: None,
                },
                Err(error) => FileMatches {
                    matches: Vec::new(),
                    complete: false,
                    error: Some(format!("{error:?}")),
                },
            }
        }
        Terminal::Pattern(terminal) => {
            let kinds = terminal.pattern.potential_kinds();
            scan(
                file,
                kinds.as_ref().map(|kinds| kinds.iter()),
                exhausted,
                |node| {
                    #[cfg(test)]
                    pattern_match_attempts.fetch_add(1, Ordering::Relaxed);
                    terminal.match_at(file, node)
                },
            )
        }
        #[cfg(test)]
        Terminal::Kind(kind, _) => {
            let kinds = kind.potential_kinds();
            scan(
                file,
                kinds.as_ref().map(|kinds| kinds.iter()),
                exhausted,
                |node| {
                    kind.match_node(file.node(node))
                        .is_some()
                        .then_some(TerminalMatch {
                            node,
                            output_label: 0,
                            span: None,
                            captures: Vec::new(),
                        })
                },
            )
        }
        Terminal::Text(..) => unreachable!(),
    }
}

/// Visits every node of `kinds` (every node when any kind can match) in preorder until the file's
/// scan budget (`BatchFile::bounded`) or `exhausted` ends the scan: the matches found by then are
/// a subset, so the rows are incomplete and never prove absence.
fn scan(
    file: &BatchFile,
    kinds: Option<impl Iterator<Item = usize>>,
    exhausted: impl Fn() -> bool,
    mut visit: impl FnMut(u32) -> Option<TerminalMatch>,
) -> FileMatches {
    let nodes: Box<dyn Iterator<Item = u32>> = match kinds {
        Some(kinds) => {
            let mut nodes = kinds
                .flat_map(|kind| file.nodes_of_kind(kind as u16).iter().copied())
                .collect::<Vec<_>>();
            nodes.sort_unstable();
            Box::new(nodes.into_iter())
        }
        None => Box::new(0..file.len() as u32),
    };
    let mut matches = Vec::new();
    for (visited, node) in nodes.enumerate() {
        if visited >= file.node_limit || exhausted() {
            return FileMatches {
                matches,
                complete: false,
                error: Some("budget-exceeded".to_owned()),
            };
        }
        matches.extend(visit(node));
    }
    FileMatches {
        matches,
        complete: true,
        error: None,
    }
}

/// Whether a pattern binds `name` as a multi-node (`$$$NAME`) capture.
fn pattern_is_multi(pattern: &Pattern, name: &str) -> bool {
    fn walk(node: &ast_grep_core::matcher::PatternNode, name: &str) -> bool {
        use ast_grep_core::matcher::PatternNode;
        match node {
            PatternNode::MetaVar { meta_var } => {
                matches!(meta_var, MetaVariable::MultiCapture(multi) if multi == name)
            }
            PatternNode::Terminal { .. } => false,
            PatternNode::Internal { children, .. } => {
                children.iter().any(|child| walk(child, name))
            }
        }
    }
    walk(&pattern.node, name)
}

#[derive(Default)]
pub(in crate::code_rule_dsl) struct MatchRegistry {
    next: AtomicI64,
    batches: Mutex<HashMap<i64, Arc<MatchBatch>>>,
}

/// Counters that show how SQLite used a batch: how often each terminal executed, and how often
/// the table was filtered (called) in all.
#[cfg(test)]
#[derive(Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct BatchCounters {
    pub(in crate::code_rule_dsl) terminal_executions: Vec<usize>,
    pub(in crate::code_rule_dsl) filters: usize,
    pub(in crate::code_rule_dsl) pattern_match_attempts: usize,
}

impl MatchRegistry {
    #[cfg(test)]
    #[allow(dead_code, reason = "retained for direct virtual-table tests")]
    pub(super) fn register_batch(&self, programs: &[Program], files: &[TapeDoc]) -> i64 {
        let files = files
            .iter()
            .map(|tape| BatchFile::new(tape.clone(), Arc::from(""), Arc::from(""), None))
            .collect();
        self.insert(
            Arc::new(programs.iter().cloned().map(Terminal::Program).collect()),
            files,
            None,
            None,
        )
    }

    /// Registers a query batch: file versions, the terminals its statements read, and the state
    /// its owner attaches. Returns the batch token statements pass as `batch`.
    pub(in crate::code_rule_dsl) fn register_query_batch(
        &self,
        terminals: Arc<Vec<Terminal>>,
        files: Vec<BatchFile>,
        extension: Arc<dyn Any + Send + Sync>,
        deadline: Option<crate::code_rule_dsl::budget::MatchDeadline>,
    ) -> i64 {
        self.insert(terminals, files, Some(extension), deadline)
    }

    fn insert(
        &self,
        terminals: Arc<Vec<Terminal>>,
        files: Vec<BatchFile>,
        extension: Option<Arc<dyn Any + Send + Sync>>,
        deadline: Option<crate::code_rule_dsl::budget::MatchDeadline>,
    ) -> i64 {
        let token = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let cells = terminals
            .iter()
            .map(|_| TerminalCell {
                rows: OnceLock::new(),
                nodes: Mutex::new(HashMap::new()),
                executions: AtomicUsize::new(0),
            })
            .collect();
        let batch = Arc::new(MatchBatch {
            token,
            terminals,
            cells,
            files,
            deadline,
            #[cfg(test)]
            rows: OnceLock::new(),
            #[cfg(test)]
            executions: AtomicUsize::new(0),
            filters: AtomicUsize::new(0),
            #[cfg(test)]
            pattern_match_attempts: AtomicUsize::new(0),
            extension,
        });
        self.batches
            .lock()
            .expect("match registry mutex poisoned")
            .insert(token, batch);
        token
    }

    #[cfg(test)]
    #[allow(dead_code, reason = "retained for SQL execution-count profiling")]
    pub(super) fn execution_count(&self, token: i64) -> Option<usize> {
        self.batch(token)
            .map(|batch| batch.executions.load(Ordering::Relaxed))
    }

    pub(in crate::code_rule_dsl) fn query_execution_counters(
        &self,
        token: i64,
    ) -> Option<(usize, usize)> {
        self.batch(token).map(|batch| {
            (
                batch
                    .cells
                    .iter()
                    .map(|cell| cell.executions.load(Ordering::Relaxed))
                    .sum(),
                batch.filters.load(Ordering::Relaxed),
            )
        })
    }

    #[cfg(test)]
    pub(in crate::code_rule_dsl) fn counters(&self, token: i64) -> Option<BatchCounters> {
        self.batch(token).map(|batch| BatchCounters {
            terminal_executions: batch
                .cells
                .iter()
                .map(|cell| cell.executions.load(Ordering::Relaxed))
                .collect(),
            filters: batch.filters.load(Ordering::Relaxed),
            pattern_match_attempts: batch.pattern_match_attempts.load(Ordering::Relaxed),
        })
    }

    pub(in crate::code_rule_dsl) fn remove_batch(&self, token: i64) {
        self.batches
            .lock()
            .expect("match registry mutex poisoned")
            .remove(&token);
    }

    fn batch(&self, token: i64) -> Option<Arc<MatchBatch>> {
        self.batches
            .lock()
            .expect("match registry mutex poisoned")
            .get(&token)
            .cloned()
    }

    /// A batch's files, for the other tables and functions of the same statement.
    pub(in crate::code_rule_dsl) fn files(&self, token: i64) -> Option<BatchView> {
        self.batch(token).map(|batch| BatchView { batch })
    }
}

/// Shared read access to one registered batch.
#[derive(Clone)]
pub(in crate::code_rule_dsl) struct BatchView {
    batch: Arc<MatchBatch>,
}

impl BatchView {
    pub(in crate::code_rule_dsl) fn files(&self) -> &[BatchFile] {
        &self.batch.files
    }

    pub(in crate::code_rule_dsl) fn file(&self, index: i64) -> Option<&BatchFile> {
        usize::try_from(index)
            .ok()
            .and_then(|index| self.batch.files.get(index))
    }

    /// Match one pattern terminal at one selected node without materializing its relation.
    pub(in crate::code_rule_dsl) fn terminal_at(
        &self,
        index: usize,
        file: usize,
        node: u32,
    ) -> Option<Arc<TerminalMatch>> {
        self.batch.terminal_at(index, file, node)
    }

    pub(in crate::code_rule_dsl) fn extension(&self) -> Option<&Arc<dyn Any + Send + Sync>> {
        self.batch.extension.as_ref()
    }
}

pub(in crate::code_rule_dsl) fn register_module(
    connection: &Connection,
    registry: Arc<MatchRegistry>,
) -> SqlResult<()> {
    const MODULE: Module<MatchTable> = Module::eponymous_only_module();
    connection.create_module(MODULE_NAME, &MODULE, Some(registry))
}

#[repr(C)]
struct MatchTable {
    base: ffi::sqlite3_vtab,
    registry: Arc<MatchRegistry>,
}

unsafe impl<'vtab> VTab<'vtab> for MatchTable {
    type Aux = Arc<MatchRegistry>;
    type Cursor = MatchCursor<'vtab>;

    fn connect(
        db: &mut VTabConnection,
        aux: Option<&Arc<MatchRegistry>>,
        _module_name: &[u8],
        _database_name: &[u8],
        _table_name: &[u8],
        _args: &[&[u8]],
    ) -> SqlResult<(Cow<'static, CStr>, Self)> {
        db.config(VTabConfig::DirectOnly)?;
        let registry = Arc::clone(aux.ok_or_else(|| {
            rusqlite::Error::ModuleError("rerail_matches registry is missing".to_owned())
        })?);
        Ok((
            Cow::Borrowed(c"CREATE TABLE x(batch HIDDEN,file_index,program_index,match_index,start_ordinal,output_label,capture_index,capture_label,capture_node,start_byte,end_byte,complete,error_code)"),
            Self {
                base: ffi::sqlite3_vtab::default(),
                registry,
            },
        ))
    }

    fn best_index(&self, info: &mut IndexInfo) -> SqlResult<bool> {
        let mut slots: [Option<usize>; 4] = [None; 4];
        let mut file_in = false;
        for (index, constraint) in info.constraints().enumerate() {
            if !constraint.is_usable()
                || constraint.operator() != IndexConstraintOp::SQLITE_INDEX_CONSTRAINT_EQ
            {
                continue;
            }
            let slot = match constraint.column() {
                BATCH_COLUMN => 0,
                PROGRAM_COLUMN => 1,
                FILE_COLUMN => 2,
                START_COLUMN => 3,
                _ => continue,
            };
            slots[slot].get_or_insert(index);
        }
        if let Some(index) = slots[2] {
            file_in = info.is_in_constraint(index)? && info.set_in_constraint(index, true)?;
        }
        let Some(_) = slots[0] else {
            info.set_idx_num(0);
            info.set_estimated_cost(f64::MAX);
            info.set_estimated_rows(i64::MAX);
            return Ok(true);
        };
        // Without a terminal the whole transducer batch is read (test batches only); a start
        // ordinal alone does not narrow it.
        if slots[1].is_none() {
            slots[2] = None;
            slots[3] = None;
        }
        let mut idx_num = 0;
        let mut argv = 0;
        let bits = [
            USE_BATCH,
            USE_PROGRAM,
            if file_in { USE_FILE_IN } else { USE_FILE_EQ },
            USE_START,
        ];
        let used = slots;
        for (slot, constraint) in used.iter().enumerate() {
            if let Some(constraint) = constraint {
                argv += 1;
                idx_num |= bits[slot];
                let mut usage = info.constraint_usage(*constraint);
                usage.set_argv_index(argv);
                usage.set_omit(true);
            }
        }
        let (cost, rows) = if idx_num & USE_START != 0 {
            (2.0, 2)
        } else if idx_num & (USE_FILE_EQ | USE_FILE_IN) != 0 {
            (50.0, 50)
        } else if idx_num & USE_PROGRAM != 0 {
            (5_000.0, 5_000)
        } else {
            (100_000.0, 100_000)
        };
        info.set_idx_num(idx_num);
        info.set_estimated_cost(cost);
        info.set_estimated_rows(rows);
        Ok(true)
    }

    fn open(&mut self) -> SqlResult<Self::Cursor> {
        Ok(MatchCursor {
            base: ffi::sqlite3_vtab_cursor::default(),
            registry: Arc::clone(&self.registry),
            rows: Arc::new(Vec::new()),
            position: 0,
            phantom: PhantomData,
        })
    }
}

#[repr(C)]
struct MatchCursor<'vtab> {
    base: ffi::sqlite3_vtab_cursor,
    registry: Arc<MatchRegistry>,
    rows: Arc<Vec<MatchRow>>,
    position: usize,
    phantom: PhantomData<&'vtab MatchTable>,
}

fn nonnegative(value: i64, what: &str) -> SqlResult<usize> {
    usize::try_from(value)
        .map_err(|_| rusqlite::Error::ModuleError(format!("rerail_matches {what} is negative")))
}

unsafe impl VTabCursor for MatchCursor<'_> {
    fn filter(
        &mut self,
        idx_num: c_int,
        _idx_str: Option<&str>,
        args: &Filters<'_>,
    ) -> SqlResult<()> {
        if idx_num & USE_BATCH == 0 {
            return Err(rusqlite::Error::ModuleError(
                "rerail_matches requires one batch equality constraint".to_owned(),
            ));
        }
        let token = args.get::<i64>(0)?;
        let batch = self.registry.batch(token).ok_or_else(|| {
            rusqlite::Error::ModuleError("rerail_matches batch is missing".to_owned())
        })?;
        batch.filters.fetch_add(1, Ordering::Relaxed);
        self.position = 0;
        if idx_num & USE_PROGRAM == 0 {
            #[cfg(test)]
            {
                self.rows = batch.rows().map_err(rusqlite::Error::ModuleError)?;
                return Ok(());
            }
            #[cfg(not(test))]
            return Err(rusqlite::Error::ModuleError(NEEDS_PROGRAMS.to_owned()));
        }
        let mut argv = 1;
        let program = nonnegative(args.get::<i64>(argv)?, "program index")?;
        argv += 1;
        if program >= batch.terminals.len() {
            self.rows = Arc::new(Vec::new());
            return Ok(());
        }
        let files = if idx_num & USE_FILE_EQ != 0 {
            let file = nonnegative(args.get::<i64>(argv)?, "file index")?;
            argv += 1;
            Some(vec![file])
        } else if idx_num & USE_FILE_IN != 0 {
            let mut files = Vec::new();
            let mut values = args.in_values(argv)?;
            while let Some(value) = values.next()? {
                files.push(nonnegative(value.as_i64()?, "file index")?);
            }
            argv += 1;
            files.sort_unstable();
            files.dedup();
            Some(files)
        } else {
            None
        };
        let start = if idx_num & USE_START != 0 {
            u32::try_from(args.get::<i64>(argv)?).ok()
        } else {
            None
        };
        if idx_num & USE_START != 0 && start.is_none() {
            self.rows = Arc::new(Vec::new());
            return Ok(());
        }
        self.rows = Arc::new(batch.terminal_rows(program, files.as_deref(), start));
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
        let row = &self.rows[self.position];
        match column {
            0 => context.set_result(&row.batch),
            1 => context.set_result(&row.file),
            2 => context.set_result(&row.program),
            3 => context.set_result(&row.match_index),
            4 => context.set_result(&row.start_ordinal),
            5 => context.set_result(&row.output_label),
            6 => context.set_result(&row.capture_index),
            7 => context.set_result(&row.capture_label),
            8 => context.set_result(&row.capture_node),
            9 => context.set_result(&row.start_byte),
            10 => context.set_result(&row.end_byte),
            11 => context.set_result(&row.complete),
            12 => context.set_result(&row.error_code.as_deref()),
            _ => Err(rusqlite::Error::InvalidColumnIndex(column as usize)),
        }
    }

    fn rowid(&self) -> SqlResult<i64> {
        Ok(self.position as i64 + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use ast_grep_core::tree_sitter::StrDoc;
    use std::sync::Arc;

    fn python_file(source: &str) -> BatchFile {
        let grammar = Grammar::new(GrammarDialect::Python).unwrap();
        let native = StrDoc::new(source, GrammarDialect::Python);
        let image =
            TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        let bytes = image.to_bytes().unwrap();
        let image = TapeImage::from_bytes(&bytes, &grammar).unwrap();
        let tape =
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
        BatchFile::new(tape, Arc::from("test.py"), Arc::from("python"), None)
    }

    #[test]
    fn pattern_terminal_reports_incomplete_when_deadline_is_spent() {
        let terminal = Terminal::Pattern(Arc::new(PatternTerminal::new(
            Pattern::try_new("target", GrammarDialect::Python).unwrap(),
            GrammarDialect::Python,
        )));
        let attempts = AtomicUsize::new(0);
        let matches =
            execute_terminal_with_budget(&terminal, &python_file("target\n"), || true, &attempts);
        assert!(!matches.complete);
        assert_eq!(matches.error.as_deref(), Some("budget-exceeded"));
        assert!(matches.matches.is_empty());
    }

    #[test]
    fn terminal_observes_owner_deadline_on_rayon_worker() {
        let owner_budget = crate::code_rule_dsl::budget::MatchBudget::expired_for_test();
        let registry = MatchRegistry::default();
        let token = registry.register_query_batch(
            Arc::new(vec![Terminal::Pattern(Arc::new(PatternTerminal::new(
                Pattern::try_new("target", GrammarDialect::Python).unwrap(),
                GrammarDialect::Python,
            )))]),
            vec![python_file("target\n")],
            Arc::new(()),
            Some(owner_budget.deadline()),
        );
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        let matches = pool.install(|| registry.batch(token).unwrap().terminal(0));
        assert!(!matches[0].complete);
        assert_eq!(matches[0].error.as_deref(), Some("budget-exceeded"));
    }

    #[test]
    fn ast_terminals_stop_at_their_scan_budget() {
        let source = "target\n".repeat(5);
        for terminal in [
            Terminal::Pattern(Arc::new(PatternTerminal::new(
                Pattern::try_new("target", GrammarDialect::Python).unwrap(),
                GrammarDialect::Python,
            ))),
            Terminal::Kind(
                KindMatcher::new("identifier", GrammarDialect::Python),
                GrammarDialect::Python,
            ),
        ] {
            let attempts = AtomicUsize::new(0);
            let whole =
                execute_terminal_with_budget(&terminal, &python_file(&source), || false, &attempts);
            assert!(whole.complete);
            assert_eq!(whole.matches.len(), 5);

            let attempts = AtomicUsize::new(0);
            let limited = execute_terminal_with_budget(
                &terminal,
                &python_file(&source).bounded(2),
                || false,
                &attempts,
            );
            assert!(
                !limited.complete,
                "a scan past its node budget is incomplete"
            );
            assert_eq!(limited.matches.len(), 2);
            assert_eq!(limited.error.as_deref(), Some("budget-exceeded"));
            assert!(attempts.load(Ordering::Relaxed) <= 2);
        }
    }
}
