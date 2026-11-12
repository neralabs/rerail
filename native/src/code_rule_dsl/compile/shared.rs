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
    /// Each utility's id and whether it has parameters, which every table of the language names.
    names: Arc<[String]>,
    framed: Arc<[bool]>,
    /// Each utility's `UtilOrigin`, once a table needs them; `None` where they cannot be computed.
    origins: OnceCell<Option<Arc<[Option<UtilOrigin>]>>>,
}

/// A shared table and what compiling each utility in it added to the queries that reach it, by
/// index: only the utilities compiled so far, as the table holds state for only those.
struct Table {
    table: Arc<UtilTable>,
    added: HashMap<usize, Added>,
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
                    names: utils.keys().cloned().collect(),
                    framed: utils.values().map(|util| !util.params.is_empty()).collect(),
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
            .filter(|index| !table.added.contains_key(index))
            .collect::<Vec<_>>();
        if !missing.is_empty() && table.compile(shared, &missing, dialect, context).is_none() {
            *slot = None;
            return None;
        }
        let mut gap = None;
        let mut instances = BTreeSet::new();
        for &index in &indices {
            let added = table
                .added
                .get(&index)
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
                Arc::clone(&self.names),
                Arc::clone(&self.framed),
                origins,
            )),
            added: HashMap::new(),
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
            self.added.insert(index, added);
        }
        Some(())
    }
}

/// What a compiled document holds, read from jemalloc's counters: Windows builds use the system
/// allocator, which keeps none.
#[cfg(all(test, not(windows)))]
mod tests {
    use crate::code_rule_dsl::test_support::*;
    use serde_json::{Map, Value, json};

    /// The bytes `build` leaves allocated on this thread, with what it built: jemalloc's
    /// per-thread counters, which other tests' threads do not move.
    fn retained<T>(build: impl FnOnce() -> T) -> (T, i64) {
        use tikv_jemalloc_ctl::thread::{allocatedp, deallocatedp};
        let allocated = allocatedp::read().expect("jemalloc counts the thread's allocations");
        let deallocated = deallocatedp::read().expect("jemalloc counts the thread's frees");
        let held = || allocated.get() as i64 - deallocated.get() as i64;
        let before = held();
        let built = build();
        (built, held() - before)
    }

    /// The bytes a compiled document of one define, `utils`, holds, with a query per rule of
    /// `reaches`. Each query declares a global of its own, as a rule generated from a linter's
    /// list of built-in APIs declares the ones it reads, so its `languageOptions` compile the
    /// utilities it reaches in a shared table of their own, once per grammar.
    fn held_by(utils: Map<String, Value>, reaches: Vec<Value>) -> i64 {
        let mut clauses =
            vec![json!({ "id": "table", "define": { "language": "typescript", "utils": utils } })];
        clauses.extend(reaches.into_iter().enumerate().map(|(query, rule)| {
            json!({ "id": format!("q{query}"), "violation": violation(), "forbid": { "ast": {
                "language": "typescript",
                "languageOptions": { "globals": { format!("g{query}"): "readonly" } },
                "rule": rule,
            } } })
        }));
        let document = json!({ "format": "rerail.policy/v1", "id": "table", "revision": 1, "mode": "steer", "clauses": clauses });
        let capabilities = serde_json::from_str::<Value>(CAPABILITIES).expect("capabilities");
        // A copy goes in: the compilation frees what it is given, which `retained` would take
        // off what the rule holds.
        let (compiled, held) = retained(|| compile_document_with(document.clone(), &capabilities));
        compiled.unwrap_or_else(|error| panic!("{}", error.reason));
        held
    }

    #[test]
    fn a_document_holds_each_regex_pattern_once_however_many_tables_compile_it() {
        // 120 utilities of 2 regexes each over 8 patterns, all reached by 8 queries in 3
        // grammars: 5,760 regex terms. Compiled one per term they held about 85 MiB.
        let utils = (0..120)
            .map(|index| {
                let rule = json!({ "any": [
                    { "kind": "property_identifier", "regex": format!("^name{}$", index % 4) },
                    { "kind": "string", "value": { "regex": format!("^(?:node:)?module{}$", index % 4) } },
                ] });
                (format!("u{index}"), rule)
            })
            .collect();
        let every = (0..120)
            .map(|index| json!({ "matches": format!("u{index}") }))
            .collect::<Vec<_>>();
        let held = held_by(utils, vec![json!({ "any": every }); 8]);
        assert!(held < 16 << 20, "the document holds {} MiB", held >> 20);
    }

    #[test]
    fn a_shared_table_holds_only_the_utilities_its_queries_reach() {
        // 24 queries in 3 grammars, each reaching one of 2,000 utilities: 72 shared tables. With
        // room for every utility's rule and memo in each, they held about 30 MiB.
        let utils = (0..2000)
            .map(|index| (format!("u{index}"), json!({ "kind": "identifier" })))
            .collect();
        let reaches = (0..24)
            .map(|query| json!({ "matches": format!("u{query}") }))
            .collect();
        let held = held_by(utils, reaches);
        assert!(held < 8 << 20, "the document holds {} MiB", held >> 20);
    }
}
