//! A document's shared utilities (`define`), compiled once for every query that reaches them.
//!
//! A query compiles the shared utilities it reaches with its own options, as if it listed them in
//! its own `utils`. Where nothing of the query but its language, grammar and `languageOptions`
//! shapes them (it has no utilities of its own, no `transform`, and reads one document), every
//! such query compiles them alike, so they compile once per document, grammar and
//! `languageOptions`, into one table its queries share, each utility when a query first reaches
//! it. A query then reads the table as it would its own: the gaps, calls and capture names of the
//! utilities it reaches, and nothing of the others.
//!
//! Compiling a utility here leaves out what its queries settle with their own state: an `all`
//! ordering conflict (settled with what else binds each name) and a `calls` term (whose seed is
//! salted with the query's utilities). A table that meets either, or any error, is given up, and
//! its queries compile their utilities in tables of their own, as before.

use super::ast_rule::compile_ast_rule;
use super::define::SharedUtils;
use super::documents::Side;
use super::util_origins;
use super::utils::{Instances, UtilDef, UtilIds, UtilScope};
use crate::code_rule_dsl::calls::CallsCollector;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::{CallArgs, MAX_UTIL_INSTANCES, UtilOrigin, UtilTable};
use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
use napi::Result;
use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// The query context a shared table compiles in: the query's `languageOptions`, as written and
/// compiled.
pub(super) struct Context {
    pub(super) written: Option<String>,
    pub(super) options: Option<Arc<LanguageOptions>>,
    pub(super) python: Option<Arc<PythonOptions>>,
}

/// The document's shared tables; `None` where one was given up.
pub(super) struct SharedTables<'a> {
    languages: BTreeMap<&'a str, Language<'a>>,
    tables: RefCell<HashMap<TableKey, Option<Table>>>,
}

/// What a shared table compiles for: a language, a grammar, and its queries' `languageOptions` as
/// written.
type TableKey = (String, GrammarDialect, Option<String>);

/// One language's shared utilities, in id order (a shared table's index order).
struct Language<'a> {
    language: &'a str,
    utils: &'a BTreeMap<String, UtilDef<'a>>,
    defs: Vec<&'a UtilDef<'a>>,
    ids: UtilIds<'a>,
    indices: HashMap<&'a str, usize>,
    /// Each utility's `UtilOrigin`, once a table needs them; `None` where they cannot be computed.
    origins: OnceCell<Option<Vec<Option<UtilOrigin>>>>,
}

/// A shared table and what compiling each utility in it added to the queries that reach it.
struct Table {
    table: Arc<UtilTable>,
    added: Vec<Option<Added>>,
}

/// What compiling a utility adds to a query that reaches it: its first grammar gap and its
/// parameterized calls.
struct Added {
    gap: Option<String>,
    instances: BTreeSet<(usize, CallArgs)>,
}

/// A query's utilities in a shared table.
pub(super) struct Reach<'s> {
    pub(super) table: Arc<UtilTable>,
    /// The ids a rule of the table names.
    pub(super) ids: &'s UtilIds<'s>,
    /// The query's utilities' indices, in id order.
    pub(super) indices: Vec<usize>,
    /// The first grammar gap compiling them met, in id order.
    pub(super) gap: Option<String>,
    /// Their distinct parameterized calls, which the query's own rules add to.
    pub(super) instances: Instances,
}

impl<'a> SharedTables<'a> {
    pub(super) fn new(shared: &'a SharedUtils<'a>) -> Self {
        let languages = shared
            .iter()
            .map(|(language, utils)| {
                let language = Language {
                    language,
                    utils,
                    defs: utils.values().collect(),
                    ids: utils
                        .iter()
                        .map(|(id, util)| (id.as_str(), util.params.as_slice()))
                        .collect(),
                    indices: utils
                        .keys()
                        .enumerate()
                        .map(|(index, id)| (id.as_str(), index))
                        .collect(),
                    origins: OnceCell::new(),
                };
                (language.language, language)
            })
            .collect();
        Self {
            languages,
            tables: RefCell::new(HashMap::new()),
        }
    }

    /// The shared utilities `reached` (ids of `language`'s, in id order) compiled for `dialect` in
    /// `context`, in the document's table; `None` where they compile in a table of the query's
    /// own, since the table was given up.
    pub(super) fn reach<'r>(
        &self,
        language: &str,
        dialect: GrammarDialect,
        context: &Context,
        reached: impl IntoIterator<Item = &'r str>,
    ) -> Option<Reach<'_>> {
        let shared = self.languages.get(language)?;
        let indices = reached
            .into_iter()
            .map(|id| shared.indices.get(id).copied())
            .collect::<Option<Vec<_>>>()?;
        let mut tables = self.tables.borrow_mut();
        let slot = tables
            .entry((language.to_owned(), dialect, context.written.clone()))
            .or_insert_with(|| shared.table());
        let table = slot.as_mut()?;
        let missing = indices
            .iter()
            .copied()
            .filter(|&index| table.added[index].is_none())
            .collect::<Vec<_>>();
        if !missing.is_empty() && table.compile(shared, &missing, dialect, context).is_none() {
            *slot = None;
            return None;
        }
        let mut gap = None;
        let mut instances = BTreeSet::new();
        for &index in &indices {
            let added = table.added[index]
                .as_ref()
                .expect("a reached utility is compiled");
            gap = gap.or_else(|| added.gap.clone());
            instances.extend(added.instances.iter().cloned());
        }
        // The define's check compiled all its utilities' calls in one table per grammar, within
        // the limit, so the calls of any of them are too.
        debug_assert!(instances.len() <= MAX_UTIL_INSTANCES);
        Some(Reach {
            table: Arc::clone(&table.table),
            ids: &shared.ids,
            indices,
            gap,
            instances: RefCell::new(instances),
        })
    }
}

impl Language<'_> {
    /// An empty table of the language's shared utilities; `None` where their origins cannot be
    /// computed.
    fn table(&self) -> Option<Table> {
        let origins = self
            .origins
            .get_or_init(|| {
                let origins = util_origins(self.utils, self.language).ok()?;
                Some(origins.into_iter().map(Some).collect())
            })
            .clone()?;
        Some(Table {
            table: Arc::new(UtilTable::new(
                self.utils.keys().cloned().collect(),
                self.defs
                    .iter()
                    .map(|util| !util.params.is_empty())
                    .collect(),
                origins,
            )),
            added: self.defs.iter().map(|_| None).collect(),
        })
    }
}

impl Table {
    /// Compiles and fills utilities `missing`, which with the filled ones hold every utility they
    /// reach, as a query's own table compiles its utilities: once, and again against that first
    /// compilation where a walk met one not compiled yet. `None` where they compile only in a
    /// query's own table.
    fn compile(
        &mut self,
        shared: &Language,
        missing: &[usize],
        dialect: GrammarDialect,
        context: &Context,
    ) -> Option<()> {
        let conflicts = RefCell::new(BTreeSet::new());
        // A seed's collector refuses a `calls` term.
        let calls = CallsCollector::new(String::new(), true);
        let instances = Instances::default();
        let scope = UtilScope {
            table: Arc::downgrade(&self.table),
            ids: &shared.ids,
            options: context.options.clone(),
            python: context.python.clone(),
            transform: None,
            conflicts: Some(&conflicts),
            captures: Default::default(),
            calls: Some(&calls),
            documents: None,
            instances: Some(&instances),
            side: Side::Host,
        };
        let compile = || {
            missing
                .iter()
                .map(|&index| {
                    let util = shared.defs[index];
                    let mut gaps = Vec::new();
                    let rule =
                        compile_ast_rule(util.rule, dialect, &util.rule_path, &mut gaps, &scope)?;
                    let added = Added {
                        gap: gaps.into_iter().next(),
                        instances: instances.take(),
                    };
                    Ok((rule, added))
                })
                .collect::<Result<Vec<_>>>()
                .ok()
        };
        self.table.begin_compilation();
        let mut compiled = compile()?;
        if self.table.was_unfilled() {
            for (&index, (rule, _)) in missing.iter().zip(compiled) {
                self.table.draft(index, rule);
            }
            // The first compilation's conflicts read utilities it could not; the second's stand.
            conflicts.borrow_mut().clear();
            compiled = compile()?;
        }
        if !conflicts.borrow().is_empty() {
            return None;
        }
        for (&index, (rule, added)) in missing.iter().zip(compiled) {
            self.table.fill(index, rule);
            self.added[index] = Some(added);
        }
        Some(())
    }
}
