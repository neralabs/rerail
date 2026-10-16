//! Compiles a clause's rule to one SQLite statement.
//!
//! Every term becomes a SQL expression over the current document, node and environment (see
//! `env.rs` for the environment values). Composition is SQL: `all` chains its terms' environments,
//! `any` takes the first that matches (`COALESCE`), `not` inverts, and a relation walks
//! `rerail_nodes` along its axis and keeps the first related node that matches (`tq_first`); a
//! sibling walk looking for the key of a capture bound outside it reads only the siblings of that
//! key (`same_key`).
//! Terminals (`pattern`, `kind`) and node predicates (`regex`, Rerail's predicates without
//! sub-rules) are functions over the batch's cached rows. A capture-free structural sub-rule
//! (`kind`, `regex`, `all`, `any`, `not`, relations that stop at the neighbour or walk to the end,
//! and `matches` of a non-recursive utility made only of these) is one call to the structural
//! kernel (`structure.rs`), which remembers each file's answers and spends no utility step.
//! A Rerail predicate with sub-rules reads its facts from `rerail_facts` and matches its sub-rules
//! over them in SQL; a parsed literal is a derived document its sub-rules walk like a file.
//!
//! An expression reads its environment argument exactly once, so an environment computed by an
//! earlier term is never computed twice; a term that needs it more than once binds it first
//! (`bind`).

use super::catalog::{CatalogBuilder, Term};
use super::recursion::Sql;
use super::structure;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::{
    AstMatcher, Binds, CaptureAnalysis, Count, FieldRelation, GrammarMatcher, NthChild, StopBy,
    UtilRef, UtilitySummary,
};
use ast_grep_core::matcher::{Matcher, PatternNode};
use ast_grep_core::meta_var::MetaVariable;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
mod semantic;

/// Why a rule has no statement yet.
#[derive(Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct Unsupported(pub(in crate::code_rule_dsl) &'static str);

type Compiled = Result<String, Unsupported>;

/// One clause's statements. Parameters: `?1` the batch, `?2` the file index; `root_sql` also
/// `?3`, one root node (a `violation.each` enumeration matches a root again). `selected_sql` reads
/// only the roots an anchor index selected (`BatchState::roots`). Rows: `(node, env)` for each
/// candidate root, `env` as `env.rs` describes (NULL where the root does not match).
pub(in crate::code_rule_dsl) struct ClausePlan {
    pub(super) sql: String,
    pub(super) selected_sql: String,
    pub(super) root_sql: String,
}

impl ClausePlan {
    /// Its statements' SQL.
    pub(super) fn statements(&self) -> [&str; 3] {
        [&self.sql, &self.selected_sql, &self.root_sql]
    }
}

#[cfg(test)]
thread_local! {
    static CHECKS_COMPILED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Whether this thread compiles sibling walks without their same-key lists (`same_key`), as
    /// the reference the lists are checked against.
    static SAME_KEY_OFF: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Compiles sibling walks on this thread without their same-key lists while `off`.
#[cfg(test)]
pub(in crate::code_rule_dsl) fn set_same_key_off(off: bool) {
    SAME_KEY_OFF.set(off);
}

/// Grammar matchers this thread has lowered to statements (`compile_clause`).
#[cfg(test)]
pub(super) fn clauses_compiled() -> usize {
    CHECKS_COMPILED.with(std::cell::Cell::get)
}

/// Compiles one grammar matcher. With `each` (the clause has `violation.each`), relations and `any`
/// pass over candidates that bind the enumerated capture to a range already reported, as the
/// matcher's enumeration does (`matcher::each`).
pub(in crate::code_rule_dsl) fn compile_clause(
    matcher: &GrammarMatcher,
    catalog: &mut CatalogBuilder,
    each: bool,
) -> Result<ClausePlan, Unsupported> {
    #[cfg(test)]
    CHECKS_COMPILED.with(|compiled| compiled.set(compiled.get() + 1));
    let mut compiler = Compiler {
        catalog,
        language: matcher.language,
        three: matcher.three_valued,
        aliases: 0,
        utils: BTreeSet::new(),
        util_terms: BTreeMap::new(),
        excluding: each,
        captures: CaptureAnalysis::default(),
        host: None,
        guards: Vec::new(),
    };
    let root = compiler.expr(&matcher.matcher, "r.file_index", "r.node", "0")?;
    let kinds = match matcher.anchors.root_kinds() {
        Some(kinds) if kinds.is_empty() => " AND 0".to_owned(),
        Some(kinds) => format!(
            " AND r.kind IN ({})",
            kinds
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
        None => String::new(),
    };
    let select = format!(
        "SELECT r.node, {root} FROM rerail_nodes r \
         WHERE r.batch = ?1 AND r.file_index = ?2 AND r.axis = 'all'{kinds}"
    );
    Ok(ClausePlan {
        root_sql: format!("{select} AND r.node = ?3"),
        selected_sql: format!("{select} AND r.selected = 1"),
        sql: select,
    })
}

struct Compiler<'a> {
    catalog: &'a mut CatalogBuilder,
    language: GrammarDialect,
    /// Whether the clause can reach an unknown verdict (`GrammarMatcher::three_valued`): its
    /// composites then keep the matcher's three-valued rules.
    three: bool,
    aliases: usize,
    /// Utilities currently expanded into this SQL expression, by table and index.
    utils: BTreeSet<(usize, usize)>,
    /// Utility statements, by table, index and whether they exclude. Every utility crosses this
    /// statement boundary so a wide utility graph cannot exceed SQLite's fixed expression-depth
    /// limit.
    util_terms: BTreeMap<(usize, usize, bool), usize>,
    /// Whether this part of the rule passes over candidates an enumeration already reported
    /// (false inside negations, counts and stop rules, which read their own bindings).
    excluding: bool,
    /// What the terms of three-valued `all`s bind and mention, each utility summarized once.
    captures: CaptureAnalysis,
    /// Inside a `parse`'s rule: its host, which `host` rules read.
    host: Option<semantic::Host>,
    /// The `binds` guards of the composites being compiled in this statement (`guarded`): the
    /// node's SQL and the `binds` tested, so an inner composite does not test the same again.
    guards: Vec<(String, Vec<Binds>)>,
}

impl Compiler<'_> {
    /// Compiles `inner` with exclusion suspended.
    fn suspended(&mut self, inner: impl FnOnce(&mut Self) -> Compiled) -> Compiled {
        let saved = std::mem::replace(&mut self.excluding, false);
        let result = inner(self);
        self.excluding = saved;
        result
    }

    /// A candidate's verdict, passed over when it binds the enumerated capture to a reported
    /// range.
    fn fresh(&self, file: &str, verdict: String) -> String {
        if self.excluding {
            format!("tq_fresh(?1, {file}, {verdict})")
        } else {
            verdict
        }
    }

    /// Whether this part of the rule may lower to the structural kernel: not where an enumeration
    /// excludes reported captures (`fresh` reads the environment inside the sub-rule), nor inside a
    /// `parse`'s rule.
    fn lowering(&self) -> bool {
        !self.excluding && self.host.is_none()
    }

    /// A structural sub-rule as one kernel call; a lone `kind`, `regex` or `value` stays the
    /// cheaper test it already is.
    fn lowered(
        &mut self,
        matcher: &AstMatcher,
        file: &str,
        node: &str,
        env: &str,
    ) -> Option<String> {
        if !self.lowering()
            || matches!(
                matcher,
                AstMatcher::Kind(_)
                    | AstMatcher::Regex(_)
                    | AstMatcher::Semantic(crate::code_rule_dsl::matcher::Semantic::Value(_))
            )
            || !structure::lowers(matcher)
        {
            return None;
        }
        let term = self.catalog.structure(matcher, self.three);
        Some(format!("tq_struct(?1, {term}, {file}, {node}, {env})"))
    }

    /// An `all`'s terms with each run of consecutive structural terms as one `all`, which lowers
    /// to one kernel call. Only neighbours join: a term between them could spend budget or bind
    /// what a later one reads, so the order around it stays.
    fn grouped(&self, items: &[AstMatcher]) -> Vec<AstMatcher> {
        if !self.lowering() {
            return items.to_vec();
        }
        let mut grouped = Vec::with_capacity(items.len());
        let mut run: Vec<AstMatcher> = Vec::new();
        let flush = |run: &mut Vec<AstMatcher>, grouped: &mut Vec<AstMatcher>| match run.len() {
            0 => {}
            1 => grouped.push(run.pop().expect("one term")),
            _ => grouped.push(AstMatcher::All(std::mem::take(run))),
        };
        for item in items {
            if structure::lowers(item) {
                run.push(item.clone());
            } else {
                flush(&mut run, &mut grouped);
                grouped.push(item.clone());
            }
        }
        flush(&mut run, &mut grouped);
        grouped
    }
}

/// Whether an expression is a column, parameter or literal, safe to read more than once.
fn is_simple(expression: &str) -> bool {
    expression
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'?'))
}

/// The capture a pattern that is one metavariable binds: the node it is matched at.
fn lone_capture(term: &AstMatcher) -> Option<&str> {
    match term {
        AstMatcher::Pattern(pattern) => match &pattern.node {
            PatternNode::MetaVar {
                meta_var: MetaVariable::Capture(name, _),
            } => Some(name),
            _ => None,
        },
        _ => None,
    }
}

/// Whether `matcher`'s verdict at a node is the same in every environment, and a match leaves the
/// environment's bindings as they were: it binds and reads no capture (its patterns define no
/// metavariable) and has no term that reads anything else of the environment (Rerail's
/// predicates, which also follow bindings; calls; constraints; counts; comparisons). A stop rule
/// runs in a fresh environment, so any one qualifies. A utility qualifies when its rule does,
/// recursion through itself included.
pub(super) fn env_free(matcher: &AstMatcher) -> bool {
    env_free_within(matcher, &mut UtilitySummary::new())
}

fn env_free_within(
    matcher: &AstMatcher,
    utilities: &mut UtilitySummary<(usize, usize, u8), bool>,
) -> bool {
    match matcher {
        AstMatcher::Pattern(pattern) => pattern.defined_vars().is_empty(),
        AstMatcher::Kind(_) | AstMatcher::Regex(_) | AstMatcher::Never => true,
        AstMatcher::All(items) | AstMatcher::Any(items) => {
            items.iter().all(|item| env_free_within(item, utilities))
        }
        AstMatcher::Not(item) => env_free_within(item, utilities),
        AstMatcher::Inside(item, _)
        | AstMatcher::Has(item, _)
        | AstMatcher::Precedes(item, _)
        | AstMatcher::Follows(item, _) => env_free_within(item.rule(), utilities),
        AstMatcher::Field(relation) => env_free_within(&relation.rule, utilities),
        AstMatcher::Util(util) => utilities.summarize(util.summary_key(), true, |utilities| {
            util.with_target(|rule| env_free_within(rule, utilities))
                .unwrap_or(false)
        }),
        _ => false,
    }
}

impl Compiler<'_> {
    fn alias(&mut self) -> String {
        self.aliases += 1;
        format!("x{}", self.aliases)
    }

    /// Makes `env` readable more than once inside `body`: a column stays as is; anything else is
    /// computed once in a one-row subquery.
    fn bind(&mut self, env: &str, body: impl FnOnce(&mut Self, &str) -> Compiled) -> Compiled {
        if is_simple(env) {
            return body(self, env);
        }
        let alias = self.alias();
        let column = format!("{alias}.e");
        let body = body(self, &column)?;
        Ok(format!(
            "(SELECT {body} FROM (SELECT {env} AS e) AS {alias})"
        ))
    }

    /// A term as a SQL expression; a term whose expression nests deeper than SQLite allows a
    /// statement to (`MAX_NESTING`) becomes its own statement, as a utility is, without spending
    /// a step of the budget. Inside a `parse`'s rule a term reads its host and stays inline.
    fn expr(&mut self, matcher: &AstMatcher, file: &str, node: &str, env: &str) -> Compiled {
        let sql = self.expr_inline(matcher, file, node, env)?;
        if self.host.is_some() || nesting(&sql) <= MAX_NESTING {
            return Ok(sql);
        }
        let term = self.catalog.term(Term::Util {
            sql: Sql::default(),
            memoize: !self.excluding,
            step: false,
            env_free: env_free(matcher),
        });
        let enclosing = std::mem::take(&mut self.guards);
        let body = self.expr_inline(matcher, "?2", "?3", "?4");
        self.guards = enclosing;
        let body = body?;
        self.catalog
            .set_util_sql(term, format!("SELECT {body}"), !self.excluding);
        Ok(format!("tq_util(?1, {term}, {file}, {node}, {env})"))
    }

    fn expr_inline(&mut self, matcher: &AstMatcher, file: &str, node: &str, env: &str) -> Compiled {
        if let Some(sql) = self.lowered(matcher, file, node, env) {
            return Ok(sql);
        }
        Ok(match matcher {
            AstMatcher::Pattern(pattern) => {
                let term = self.catalog.pattern(pattern, self.language);
                format!("tq_pattern(?1, {term}, {file}, {node}, {env})")
            }
            AstMatcher::Kind(kind) => {
                // A node from `rerail_nodes` carries its kind id: compare the column, no call.
                let single = kind.potential_kinds().and_then(|kinds| {
                    let mut ids = kinds.iter();
                    let id = ids.next()?;
                    ids.next().is_none().then_some(id)
                });
                match (node.strip_suffix(".node"), single) {
                    (Some(alias), Some(id)) => {
                        // The environment is computed first, as the matcher runs its terms in
                        // order (a `CASE` would test the kind before it).
                        self.bind(env, |_, env| {
                            Ok(format!("CASE WHEN {alias}.kind = {id} THEN {env} END"))
                        })?
                    }
                    _ => {
                        let term = self.catalog.term(Term::Kind(kind.clone()));
                        format!("tq_kind(?1, {term}, {file}, {node}, {env})")
                    }
                }
            }
            AstMatcher::Regex(regex) => {
                let term = self.catalog.term(Term::Regex(regex.clone()));
                format!("tq_regex(?1, {term}, {file}, {node}, {env})")
            }
            AstMatcher::Never => "NULL".to_owned(),
            AstMatcher::All(items) if self.three => {
                self.guarded(matcher, file, node, env, |this, env| {
                    let items = this.grouped(items);
                    this.all_three_valued(&items, file, node, env)
                })?
            }
            AstMatcher::All(items) => self.guarded(matcher, file, node, env, |this, env| {
                let mut current = env.to_owned();
                for item in &this.grouped(items) {
                    current = this.expr(item, file, node, &current)?;
                }
                Ok(current)
            })?,
            AstMatcher::Any(items) => self.guarded(matcher, file, node, env, |this, env| {
                this.bind(env, |this, env| {
                    let branches = items
                        .iter()
                        .map(|item| {
                            let branch = this.expr(item, file, node, env)?;
                            Ok(this.fresh(file, branch))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(match branches.len() {
                        0 => "NULL".to_owned(),
                        1 => branches.into_iter().next().expect("one branch"),
                        _ if this.three => this.any_three_valued(items, &branches),
                        _ => coalesce(&branches),
                    })
                })
            })?,
            AstMatcher::Not(item) => self.bind(env, |this, env| {
                let inner = this.suspended(|this| this.expr(item, file, node, env))?;
                Ok(format!("tq_not({inner}, {env})"))
            })?,
            AstMatcher::Inside(item, stop) => {
                self.relation(item, stop, Axis::Up, file, node, env)?
            }
            AstMatcher::Has(item, stop) => {
                self.relation(item, stop, Axis::Down, file, node, env)?
            }
            AstMatcher::Precedes(item, stop) => {
                self.relation(item, stop, Axis::Next, file, node, env)?
            }
            AstMatcher::Follows(item, stop) => {
                self.relation(item, stop, Axis::Prev, file, node, env)?
            }
            AstMatcher::Util(util) => {
                let key = (util.table.as_ptr() as usize, util.index);
                let term = self.util_term(util, key)?;
                format!("tq_util(?1, {term}, {file}, {node}, {env})")
            }
            AstMatcher::Constrained(core) => {
                let mut current = self.expr(&core.rule, file, node, env)?;
                for (name, constraint) in &core.constraints {
                    current = self.bind(&current, |this, env| {
                        let alias = this.alias();
                        let captured = format!("{alias}.n");
                        let checked = this.expr(constraint, file, &captured, env)?;
                        Ok(format!(
                            "(SELECT CASE WHEN {captured} IS NULL THEN {env} ELSE {checked} END \
                             FROM (SELECT tq_capture(?1, {env}, {}) AS n) AS {alias})",
                            quote(name)
                        ))
                    })?;
                }
                current
            }
            AstMatcher::Field(relation) => self.field(relation, file, node, env)?,
            AstMatcher::NthChild(nth) if nth.of_rule.is_some() => {
                self.nth_child(matcher, nth, file, node, env)?
            }
            AstMatcher::Call(call) => {
                let key = (call.util.table.as_ptr() as usize, call.util.index);
                let body = self.util_term(&call.util, key)?;
                let term = self.catalog.term(Term::Call {
                    body,
                    call: (**call).clone(),
                });
                format!("tq_call(?1, {term}, {file}, {node}, {env})")
            }
            AstMatcher::Semantic(semantic) => self.semantic(semantic, matcher, file, node, env)?,
            AstMatcher::NthChild(_) | AstMatcher::Compare(_) => {
                let term = self.catalog.term(Term::Predicate(matcher.clone()));
                let three = i64::from(self.three);
                format!("tq_pred(?1, {term}, {file}, {node}, {env}, {three})")
            }
            #[cfg(test)]
            AstMatcher::TestUnknown(_) => {
                let term = self.catalog.term(Term::Predicate(matcher.clone()));
                let three = i64::from(self.three);
                format!("tq_pred(?1, {term}, {file}, {node}, {env}, {three})")
            }
            AstMatcher::Count(count) => self.count(count, file, node, env)?,
        })
    }

    /// The `WHERE` clause walking `axis` from `node` as candidates `alias`, as ast-grep's
    /// `StopBy` walks it: with a stop rule, `has` does not enter below a stop node and, when `cut`,
    /// the other axes end after the first one (which is still a candidate).
    #[allow(clippy::too_many_arguments)]
    fn walk(
        &mut self,
        stop: &StopBy,
        axis: Axis,
        alias: &str,
        file: &str,
        node: &str,
        env: &str,
        cut: bool,
    ) -> Compiled {
        let walk = |name: &str| {
            format!(
                "{name}.batch = ?1 AND {name}.file_index = {file} AND {name}.axis = '{}' \
                 AND {name}.of = {node}",
                axis.name(stop)
            )
        };
        let mut filter = format!("{} AND {env} IS NOT NULL", walk(alias));
        if let StopBy::Rule(rule) = stop {
            let stop_alias = self.alias();
            let stops =
                self.suspended(|this| this.expr(rule, file, &format!("{stop_alias}.node"), "0"))?;
            match axis {
                Axis::Down => write!(
                    filter,
                    " AND NOT EXISTS (SELECT 1 FROM rerail_nodes {stop_alias} \
                     WHERE {stop_alias}.batch = ?1 AND {stop_alias}.file_index = {file} \
                     AND {stop_alias}.axis = 'ancestor' AND {stop_alias}.of = {alias}.node \
                     AND {stop_alias}.node > {node} AND {stops} IS NOT NULL)"
                ),
                Axis::Up | Axis::Next | Axis::Prev if cut => write!(
                    filter,
                    " AND {alias}.pos <= COALESCE((SELECT min({stop_alias}.pos) \
                     FROM rerail_nodes {stop_alias} WHERE {} AND {stops} IS NOT NULL), \
                     9223372036854775807)",
                    walk(&stop_alias)
                ),
                Axis::Up | Axis::Next | Axis::Prev => Ok(()),
            }
            .expect("write to string");
        }
        Ok(filter)
    }

    /// A relation: the first node along `axis` from `node` where `item` matches, as ast-grep's
    /// `StopBy::find` (and `has`) test them.
    fn relation(
        &mut self,
        item: &AstMatcher,
        stop: &StopBy,
        axis: Axis,
        file: &str,
        node: &str,
        env: &str,
    ) -> Compiled {
        self.bind(env, |this, env| {
            this.aliases += 1;
            let frame = this.aliases;
            let source = this.alias();
            let alias = this.alias();
            if this.three {
                let test = this.expr(item, file, &format!("{alias}.node"), env)?;
                let test = this.fresh(file, test);
                // The aggregate closes its frame on the first decisive candidate. SQLite completes
                // that step before evaluating the next row, so later expressions are gated.
                let mut filter = this.walk(stop, axis, &source, file, node, env, false)?;
                filter.push_str(&this.same_key(item, stop, axis, &source, file, node, env));
                let strict = i64::from(crate::code_rule_dsl::matcher::may_bind(item));
                let (stops, mode) = match stop {
                    StopBy::Rule(rule) => (
                        this.suspended(|this| {
                            this.expr(rule, file, &format!("{alias}.node"), "0")
                        })?,
                        i64::from(matches!(axis, Axis::Down)),
                    ),
                    StopBy::Neighbor | StopBy::End => ("NULL".to_owned(), 0),
                };
                return Ok(format!(
                    "CASE WHEN {env} IS NOT NULL AND tq_walk_start({frame}) THEN (SELECT \
                     tq_search({frame}, {alias}.pos, CASE WHEN tq_walk_live({frame}) THEN {test} END, \
                     {strict}, CASE WHEN tq_walk_live({frame}) THEN {stops} END, {mode}) FROM \
                     (SELECT {source}.* FROM rerail_nodes {source} WHERE {filter} ORDER BY {source}.pos \
                     LIMIT -1) AS {alias}) END"
                ));
            }
            let test = this.expr(item, file, &format!("{source}.node"), env)?;
            let test = this.fresh(file, test);
            let mut filter = this.walk(stop, axis, &source, file, node, env, true)?;
            filter.push_str(&this.same_key(item, stop, axis, &source, file, node, env));
            let first = this.alias();
            Ok(format!(
                "CASE WHEN {env} IS NOT NULL THEN (SELECT {first}.v FROM (SELECT {test} AS v \
                 FROM rerail_nodes {source} WHERE {filter} ORDER BY {source}.pos LIMIT -1) \
                 AS {first} WHERE {first}.v IS NOT NULL LIMIT 1) END"
            ))
        })
    }

    /// `all` in a three-valued clause (`AstMatcher::All`): terms run in order; a false term decides;
    /// past an unknown term the others still run, except those that read a capture an unknown or
    /// skipped term would have bound, which are skipped and withhold their own captures too. The
    /// state (withheld terms, unknown seen) travels with the environment (`tq_all_*`).
    fn all_three_valued(
        &mut self,
        items: &[AstMatcher],
        file: &str,
        node: &str,
        env: &str,
    ) -> Compiled {
        if items.len() > 63 {
            return Err(Unsupported("three-valued `all` of more than 63 terms"));
        }
        let binds = items
            .iter()
            .map(|item| self.captures.binds(item))
            .collect::<Vec<_>>();
        let mut current = format!("tq_all_begin(?1, {env})");
        for (index, item) in items.iter().enumerate() {
            let mentions = self.captures.mentions(item);
            let mask = (0..index)
                .filter(|&earlier| !binds[earlier].is_disjoint(&mentions))
                .fold(0_i64, |mask, earlier| mask | (1 << earlier));
            let bit = 1_i64 << index;
            let contributes = if crate::code_rule_dsl::matcher::binds_nothing(item) {
                0
            } else {
                bit
            };
            current = self.bind(&current, |this, state| {
                let test = this.expr(item, file, node, state)?;
                Ok(format!(
                    "CASE WHEN {state} IS NULL THEN NULL \
                     WHEN tq_all_withheld(?1, {state}, {mask}) THEN tq_all_mark(?1, {state}, {bit}) \
                     ELSE tq_all_after(?1, {state}, {test}, {contributes}) END"
                ))
            })?;
        }
        Ok(format!("tq_all_end(?1, {current})"))
    }

    /// `any` in a three-valued clause: the first branch that matches; unknown when none matches
    /// and some branch was unknown. An unknown branch that may bind captures ends the `any` as
    /// unknown: a later branch must not substitute its captures for the ones it would have bound.
    /// Flat, so a long `any` stays within SQLite's expression depth: `tq_any_try` passes a match on
    /// and notes an unknown branch under the expression's frame, which `tq_any` reads once
    /// `COALESCE` has stopped.
    fn any_three_valued(&mut self, items: &[AstMatcher], branches: &[String]) -> String {
        self.aliases += 1;
        let frame = self.aliases;
        let tried = items
            .iter()
            .zip(branches)
            .map(|(item, branch)| {
                let strict = i64::from(crate::code_rule_dsl::matcher::may_bind(item));
                format!("tq_any_try({frame}, {branch}, {strict})")
            })
            .collect::<Vec<_>>();
        format!("tq_any({frame}, {})", coalesce(&tried))
    }

    /// `count`: how many nodes along the relation's walk match (`has` descends below matches),
    /// within `min..=max`. The rule reads the environment; nothing it binds is kept. Each row also
    /// carries its own stop verdict: an unknown one ended the walk (or left a subtree unentered)
    /// without saying how many nodes lie past it, so the count is unbounded above.
    fn count(&mut self, count: &Count, file: &str, node: &str, env: &str) -> Compiled {
        let (item, stop, axis) = match &count.relation {
            AstMatcher::Inside(item, stop) => (item, stop, Axis::Up),
            AstMatcher::Has(item, stop) => (item, stop, Axis::Down),
            AstMatcher::Precedes(item, stop) => (item, stop, Axis::Next),
            AstMatcher::Follows(item, stop) => (item, stop, Axis::Prev),
            _ => return Ok("NULL".to_owned()),
        };
        self.bind(env, |this, env| {
            this.aliases += 1;
            let frame = this.aliases;
            let source = this.alias();
            let alias = this.alias();
            let test =
                this.suspended(|this| this.expr(item, file, &format!("{alias}.node"), env))?;
            let stops = match stop {
                StopBy::Rule(rule) => this
                    .suspended(|this| this.expr(rule, file, &format!("{alias}.node"), "0"))?,
                StopBy::Neighbor | StopBy::End => "NULL".to_owned(),
            };
            let mut filter = this.walk(stop, axis, &source, file, node, env, true)?;
            filter.push_str(&this.same_key(item, stop, axis, &source, file, node, env));
            let max = count.max.map_or(-1, |max| max as i64);
            Ok(format!(
                "CASE WHEN {env} IS NOT NULL AND tq_walk_start({frame}) THEN (SELECT CASE WHEN \
                 count(*) = 0 THEN CASE WHEN {} = 0 THEN {env} END ELSE tq_count({frame}, {env}, \
                 {}, {max}, CASE WHEN tq_walk_live({frame}) THEN CASE WHEN tq_step() THEN {test} END \
                 END, CASE WHEN tq_walk_live({frame}) THEN {stops} END) END FROM (SELECT {source}.* \
                 FROM rerail_nodes {source} WHERE {filter} ORDER BY {source}.pos LIMIT -1) AS {alias}) \
                 END",
                count.min, count.min
            ))
        })
    }

    /// A sibling walk to the end whose item looks for a node with the same key as a capture bound
    /// outside it reads only those siblings: the walk's `IN` list (`tq_same_key`), or nothing to
    /// add. The item is an `all` holding `pattern: $Q` and, after it, `compare: {left: Q, op: eq,
    /// right: P}` (either way round) under a text mode whose verdict each side's class decides
    /// (`Comparison::eq_class`), and none of its other terms binds `Q` or `P`. A sibling whose
    /// class is neither `P`'s nor unknown then fails the comparison, which nothing unknown can
    /// withhold (only the pattern binds what it reads), and a false term fails an `all`: the
    /// sibling neither matches nor leaves the verdict unknown, so the walk passes over it without
    /// changing the answer, and every candidate reads its few equal siblings rather than all.
    #[allow(clippy::too_many_arguments)]
    fn same_key(
        &mut self,
        item: &AstMatcher,
        stop: &StopBy,
        axis: Axis,
        source: &str,
        file: &str,
        node: &str,
        env: &str,
    ) -> String {
        #[cfg(test)]
        if SAME_KEY_OFF.get() {
            return String::new();
        }
        let AstMatcher::All(terms) = item else {
            return String::new();
        };
        if !matches!(stop, StopBy::End)
            || !matches!(axis, Axis::Next | Axis::Prev)
            || self.host.is_some()
        {
            return String::new();
        }
        for (at, term) in terms.iter().enumerate() {
            let AstMatcher::Compare(comparison) = term else {
                continue;
            };
            if !comparison.classes_decide() {
                continue;
            }
            for (inside, outside) in [
                (&comparison.left, &comparison.right),
                (&comparison.right, &comparison.left),
            ] {
                let Some(pattern) = terms[..at]
                    .iter()
                    .position(|term| lone_capture(term) == Some(inside.as_str()))
                else {
                    continue;
                };
                let others_bind = terms
                    .iter()
                    .enumerate()
                    .filter(|&(index, _)| index != at && index != pattern)
                    .any(|(_, term)| {
                        let binds = self.captures.binds(term);
                        binds.contains(inside) || binds.contains(outside)
                    });
                if inside == outside || others_bind {
                    continue;
                }
                let term = self.catalog.term(Term::Predicate(term.clone()));
                return format!(
                    " AND {source}.node IN (SELECT value FROM \
                     json_each(tq_same_key(?1, {term}, {file}, {node}, {env}, {})))",
                    quote(outside)
                );
            }
        }
        String::new()
    }

    /// The term of a utility's own statement (`?2` file, `?3` node, `?4` environment), compiled
    /// once. A placeholder is registered before the body so mutually recursive utilities can
    /// call each other through `tq_util`.
    fn util_term(&mut self, util: &UtilRef, key: (usize, usize)) -> Result<usize, Unsupported> {
        let statement = (key.0, key.1, self.excluding);
        if let Some(&term) = self.util_terms.get(&statement) {
            return Ok(term);
        }
        let term = self.catalog.term(Term::Util {
            sql: Sql::default(),
            memoize: !self.excluding,
            step: true,
            env_free: env_free(&AstMatcher::Util(util.clone())),
        });
        self.util_terms.insert(statement, term);
        let inlining = std::mem::replace(&mut self.utils, BTreeSet::from([key]));
        // A utility's statement is shared by every caller: no caller's guard holds in it.
        let enclosing = std::mem::take(&mut self.guards);
        // A utility is its own statement: it reads no host a `parse` around its caller holds.
        let host = self.host.take();
        let body = util
            .with_target(|rule| self.expr(rule, "?2", "?3", "?4"))
            .ok_or(Unsupported("unfilled utility"));
        self.host = host;
        let body = body??;
        self.utils = inlining;
        self.guards = enclosing;
        self.catalog
            .set_util_sql(term, format!("SELECT {body}"), !self.excluding);
        Ok(term)
    }

    /// The first of `from`'s rows (in its order) whose `candidate` matches: its environment; in a
    /// three-valued clause, unknown where none matched and some candidate was unknown. With
    /// `strict`, a candidate of `-2` ends the search as unknown.
    fn first(&mut self, candidate: &str, from: &str, strict: bool) -> String {
        let rows = self.alias();
        if !self.three {
            return format!(
                "(SELECT {rows}.v FROM (SELECT {candidate} AS v FROM {from} LIMIT -1) AS {rows} \
                 WHERE {rows}.v IS NOT NULL LIMIT 1)"
            );
        }
        self.aliases += 1;
        let frame = self.aliases;
        let tried = if strict {
            let tried = self.alias();
            format!(
                "(SELECT CASE WHEN {tried}.v = -2 THEN -2 ELSE tq_any_try({frame}, {tried}.v, 0) END \
                 FROM (SELECT {candidate} AS v) AS {tried})"
            )
        } else {
            format!("tq_any_try({frame}, {candidate}, 0)")
        };
        let found = format!(
            "tq_any({frame}, (SELECT {rows}.v FROM (SELECT {tried} AS v FROM {from} LIMIT -1) \
             AS {rows} WHERE {rows}.v IS NOT NULL LIMIT 1))"
        );
        if strict {
            let ended = self.alias();
            format!(
                "(SELECT CASE WHEN {ended}.v = -2 THEN -1 ELSE {ended}.v END \
                 FROM (SELECT {found} AS v) AS {ended})"
            )
        } else {
            found
        }
    }

    /// `nthChild` with `ofRule`: the parent's named children in order, each probed with the rule
    /// (a step each, exclusion suspended) until one is unknown; the node's position among those
    /// that match, then its own bindings under the rule committed. A rule that reads and binds no
    /// capture gives every candidate the same siblings: they are probed once per parent
    /// (`tq_nth_counted`) by the rule's own statement.
    fn nth_child(
        &mut self,
        matcher: &AstMatcher,
        nth: &NthChild,
        file: &str,
        node: &str,
        env: &str,
    ) -> Compiled {
        let rule = nth.of_rule.as_ref().expect("nthChild with ofRule");
        let term = self.catalog.term(Term::Predicate(matcher.clone()));
        if self.captures.mentions(rule).is_empty() {
            let probe = self.catalog.term(Term::Util {
                sql: Sql::default(),
                memoize: false,
                step: false,
                env_free: false,
            });
            let body = self.suspended(|this| this.expr(rule, "?2", "?3", "?4"))?;
            self.catalog
                .set_util_sql(probe, format!("SELECT {body}"), false);
            return self.bind(env, |this, env| {
                let commit = this.expr(rule, file, node, env)?;
                let (counted, committed) = (this.alias(), this.alias());
                Ok(format!(
                    "CASE WHEN {env} IS NULL OR {env} < 0 THEN {env} ELSE (SELECT CASE WHEN \
                     {counted}.r IS NULL THEN NULL WHEN {counted}.r < 0 THEN -1 ELSE (SELECT CASE \
                     WHEN {counted}.r = 1 THEN (CASE WHEN {committed}.v >= 0 THEN {committed}.v \
                     ELSE {env} END) END FROM (SELECT {commit} AS v) AS {committed}) END FROM \
                     (SELECT tq_nth_counted(?1, {term}, {probe}, {file}, {node}) AS r) AS \
                     {counted}) END"
                ))
            });
        }
        self.bind(env, |this, env| {
            this.aliases += 1;
            let frame = this.aliases;
            let sibling = this.alias();
            let parent = this.alias();
            let probe = this.suspended(|this| {
                this.expr(rule, file, &format!("{sibling}.node"), env)
            })?;
            let commit = this.expr(rule, file, node, env)?;
            let (siblings, counted, committed) = (this.alias(), this.alias(), this.alias());
            Ok(format!(
                "CASE WHEN {env} IS NULL OR {env} < 0 THEN {env} ELSE (SELECT CASE WHEN \
                 {counted}.r IS NULL THEN NULL WHEN {counted}.r < 0 THEN -1 ELSE (SELECT CASE WHEN \
                 {counted}.r = 1 THEN (CASE WHEN {committed}.v >= 0 THEN {committed}.v ELSE {env} END) \
                 END FROM (SELECT {commit} AS v) AS {committed}) END FROM (SELECT tq_nth(?1, {term}, \
                 {frame}, {siblings}.node = {node}, {siblings}.v) AS r FROM (SELECT {sibling}.node AS \
                 node, CASE WHEN tq_frame_open({frame}) THEN NULL WHEN tq_step() THEN \
                 tq_frame_note({frame}, {probe}) END AS v FROM rerail_nodes {sibling} WHERE \
                 {sibling}.batch = ?1 AND {sibling}.file_index = {file} AND {sibling}.axis = 'child' \
                 AND {sibling}.of = (SELECT {parent}.parent FROM rerail_nodes {parent} WHERE \
                 {parent}.batch = ?1 AND {parent}.file_index = {file} AND {parent}.axis = 'self' AND \
                 {parent}.of = {node}) AND {sibling}.named = 1 ORDER BY {sibling}.pos LIMIT -1) AS \
                 {siblings}) AS {counted}) END"
            ))
        })
    }

    /// `inside` or `has` with `field`: a relation along the field axes of `rerail_nodes`, walked
    /// as the matcher's `FieldRelation` walks them.
    fn field(&mut self, relation: &FieldRelation, file: &str, node: &str, env: &str) -> Compiled {
        let field = relation.field;
        let (axis, entered, stop_axis) = match (relation.inside, &relation.stop) {
            (true, StopBy::Neighbor) => (format!("field_parent:{field}"), false, None),
            (true, StopBy::End) => (format!("field_ancestor:{field}"), true, None),
            (true, StopBy::Rule(_)) => (
                format!("field_ancestor:{field}"),
                true,
                Some(format!("field_ancestor:{field}")),
            ),
            (false, StopBy::Neighbor) => (format!("field_child:{field}"), false, None),
            (false, StopBy::End) => (format!("field_descendant:{field}"), false, None),
            (false, StopBy::Rule(_)) => (
                format!("field_children:{field}"),
                false,
                Some(format!("field_child:{field}")),
            ),
        };
        self.bind(env, |this, env| {
            this.aliases += 1;
            let frame = this.aliases;
            let alias = this.alias();
            let test = this.expr(&relation.rule, file, &format!("{alias}.node"), env)?;
            let test = this.fresh(file, test);
            let test = if entered {
                format!("CASE WHEN {alias}.via = 1 THEN {test} END")
            } else {
                test
            };
            let walk = format!(
                "{alias}.batch = ?1 AND {alias}.file_index = {file} AND {alias}.axis = '{axis}' \
                 AND {alias}.of = {node} AND {env} IS NOT NULL"
            );
            let stop_rule = match &relation.stop {
                StopBy::Rule(stop) => Some(stop.as_ref()),
                _ => None,
            };
            // Where a stop node is: at a candidate (every ancestor, `inside`), or at the field
            // child alone (`has`, which then enters the child's children only where it does not stop).
            let stops_at = |this: &mut Self, at: &str| -> Compiled {
                let stop = stop_rule.expect("a stop rule");
                this.suspended(|this| this.expr(stop, file, at, "0"))
            };
            if this.three {
                let strict = i64::from(crate::code_rule_dsl::matcher::may_bind(&relation.rule));
                let stops = match (&stop_axis, relation.inside) {
                    (None, _) => "NULL".to_owned(),
                    (Some(_), true) => stops_at(this, &format!("{alias}.node"))?,
                    (Some(_), false) => {
                        let at_child = stops_at(this, &format!("{alias}.node"))?;
                        format!("CASE WHEN {alias}.pos = 0 THEN {at_child} END")
                    }
                };
                let filter = if entered && stop_axis.is_none() {
                    format!("{walk} AND {alias}.via = 1")
                } else {
                    walk
                };
                return Ok(format!(
                    "CASE WHEN {env} IS NOT NULL AND tq_walk_start({frame}) THEN (SELECT \
                     tq_search({frame}, {alias}.pos, CASE WHEN tq_walk_live({frame}) THEN {test} END, \
                     {strict}, CASE WHEN tq_walk_live({frame}) THEN {stops} END, 0) FROM \
                     rerail_nodes {alias} WHERE {filter}) END"
                ));
            }
            let mut filter = walk;
            if entered {
                write!(filter, " AND {alias}.via = 1").expect("write to string");
            }
            if let Some(stop_axis) = &stop_axis {
                let stop_alias = this.alias();
                let stops = stops_at(this, &format!("{stop_alias}.node"))?;
                let stop_walk = format!(
                    "{stop_alias}.batch = ?1 AND {stop_alias}.file_index = {file} AND \
                     {stop_alias}.axis = '{stop_axis}' AND {stop_alias}.of = {node}"
                );
                if relation.inside {
                    write!(
                        filter,
                        " AND {alias}.pos <= COALESCE((SELECT min({stop_alias}.pos) FROM \
                         rerail_nodes {stop_alias} WHERE {stop_walk} AND {stops} IS NOT NULL), \
                         9223372036854775807)"
                    )
                } else {
                    write!(
                        filter,
                        " AND ({alias}.pos = 0 OR NOT EXISTS (SELECT 1 FROM rerail_nodes \
                         {stop_alias} WHERE {stop_walk} AND {stops} IS NOT NULL))"
                    )
                }
                .expect("write to string");
            }
            let first = this.alias();
            Ok(format!(
                "CASE WHEN {env} IS NOT NULL THEN (SELECT {first}.v FROM (SELECT {test} AS v \
                 FROM rerail_nodes {alias} WHERE {filter} ORDER BY {alias}.pos LIMIT -1) \
                 AS {first} WHERE {first}.v IS NOT NULL LIMIT 1) END"
            ))
        })
    }
}

#[derive(Clone, Copy)]
enum Axis {
    Up,
    Down,
    Next,
    Prev,
}

impl Axis {
    fn name(self, stop: &StopBy) -> &'static str {
        let neighbour = matches!(stop, StopBy::Neighbor);
        match (self, neighbour) {
            (Self::Up, true) => "parent",
            (Self::Up, false) => "ancestor",
            (Self::Down, true) => "child",
            (Self::Down, false) => "descendant",
            (Self::Next, true) => "next",
            (Self::Next, false) => "next_all",
            (Self::Prev, true) => "prev",
            (Self::Prev, false) => "prev_all",
        }
    }
}

fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

/// SQLite's `COALESCE` arity varies by bundled build. Pairing branches keeps each call portable
/// while preserving ordered, lazy first-match semantics with logarithmic expression depth.
fn coalesce(branches: &[String]) -> String {
    if branches.len() == 1 {
        return branches[0].clone();
    }
    if branches.len() == 2 {
        return format!("COALESCE({}, {})", branches[0], branches[1]);
    }
    let grouped = branches.chunks(2).map(coalesce).collect::<Vec<_>>();
    coalesce(&grouped)
}

/// SQLite adds the enclosing expression heights while resolving nested SELECTs, so its 1,000
/// limit can be reached with fewer than 50 parenthesis levels. Keep headroom for the surrounding
/// clause and for CASE/operator nodes, which do not introduce parentheses.
const MAX_NESTING: usize = 20;

/// The deepest parenthesis nesting of `sql`, quoted text aside.
fn nesting(sql: &str) -> usize {
    let (mut depth, mut deepest, mut quoted) = (0_usize, 0, false);
    for byte in sql.bytes() {
        match byte {
            b'\'' => quoted = !quoted,
            b'(' if !quoted => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            b')' if !quoted => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest
}
