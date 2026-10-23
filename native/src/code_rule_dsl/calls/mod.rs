//! Cross-file `calls`: a rule relation that follows a Python or PHP call into the callee's
//! definition, wherever it lives. Per-file call summaries (`summary`) are linked into one call graph
//! (`closure`); each `calls` term is answered by a backward walk from the definitions its inner
//! rule matches, computed when first read and kept until an edit can change it. The inner rule
//! runs per file as the term's seed query; the callee's subtree is never walked at query time.

pub(crate) mod closure;
pub mod contracts;
pub(crate) mod names;
pub(crate) mod php;
pub(crate) mod rust;
pub mod summary;

#[cfg(test)]
mod php_tests;
#[cfg(test)]
mod python_tests;
#[cfg(test)]
pub(crate) mod tests;

use crate::code_rule_dsl::json::{invalid, object};
use crate::code_rule_dsl::language::adapter::{Adapter, CallRules, with_adapter};
use crate::code_rule_dsl::language::is_stub_file;
use crate::code_rule_dsl::language::{Family, GrammarDialect};
use crate::code_rule_dsl::lines::Span;
use crate::code_rule_dsl::module_path::ModulePath;
use crate::code_rule_dsl::scope::php_names::PhpNames;
use crate::code_rule_dsl::scope::{Reading, ScopeTable};
use crate::content_store::HashKey;
use ast_grep_core::{Doc, Node};
use closure::{DefRef, Graph, GraphUpdate, Hop, Region};
use names::Name;
use napi::Result;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, OnceLock};
use summary::{ASYNC, CLASS, Callee, Summary};

/// Which definitions a chain may pass through: the callee and every definition after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Through {
    /// Synchronous functions only: an async function returns a coroutine instead of running.
    Sync,
    /// Any function.
    Any,
}

impl Through {
    const ALL: [Through; 2] = [Through::Sync, Through::Any];

    fn allows(self, flags: u8) -> bool {
        match self {
            Through::Sync => flags & (ASYNC | CLASS) == 0,
            Through::Any => flags & CLASS == 0,
        }
    }

    fn index(self) -> usize {
        match self {
            Through::Sync => 0,
            Through::Any => 1,
        }
    }
}

/// The graph relation and the node at which a term asks its reachability question.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Mode {
    pub(crate) through: Through,
    pub(crate) references: bool,
    pub(crate) definition: bool,
    pub(crate) links: contracts::Key,
}

impl From<Through> for Mode {
    fn from(through: Through) -> Self {
        Self {
            through,
            references: false,
            definition: false,
            links: 0,
        }
    }
}

impl Mode {
    fn allows(self, flags: u8) -> bool {
        self.through.allows(flags)
    }
    fn index(self) -> (bool, contracts::Key, usize) {
        (self.references, self.links, self.through.index())
    }
}

/// The `calls` term at a call node: its term's key.
#[derive(Clone, Debug)]
pub(crate) struct CallsTest {
    pub(crate) key: Arc<str>,
    /// The language whose calls the term follows.
    pub(crate) family: Family,
    /// The dialect the term's rule is in: the one its files' tables are read in.
    pub(crate) language: GrammarDialect,
    /// The grammar's call kinds: the only nodes the term holds at.
    pub(crate) call_kinds: Box<[u16]>,
    /// `chain`: the capture a reaching call binds to its chain of definitions
    /// (`a.fetch -> b.pause`), for messages.
    pub(crate) chain: Option<String>,
    pub(crate) definition: bool,
}

/// A term found while compiling a query's rules, before its seed query is compiled.
pub(crate) struct FoundTerm {
    pub(crate) key: Arc<str>,
    pub(crate) seed_key: Arc<str>,
    pub(crate) through: Mode,
    pub(crate) inner: Value,
    pub(crate) path: String,
}

/// Collects the `calls` terms of one query while its rules compile.
pub(crate) struct CallsCollector {
    /// What else a term's seed depends on: the query's utilities and language options.
    salt: String,
    /// Collecting for a seed query, which cannot hold `calls` (a shared utility may bring one).
    seed: bool,
    found: RefCell<Vec<FoundTerm>>,
    max_pattern: usize,
}

impl CallsCollector {
    pub(crate) fn new(salt: String, seed: bool, max_pattern: usize) -> Self {
        Self {
            salt,
            seed,
            found: RefCell::new(Vec::new()),
            max_pattern,
        }
    }

    /// The terms found, each key once, in the order first found.
    pub(crate) fn take(&self) -> Vec<FoundTerm> {
        let mut found = self.found.take();
        let mut seen = std::collections::HashSet::new();
        found.retain(|term| seen.insert(Arc::clone(&term.key)));
        found
    }

    /// Records the `calls` value at `path` and returns its test, in `language`, at `call_kinds`
    /// nodes.
    pub(crate) fn term(
        &self,
        value: &Value,
        path: &str,
        language: GrammarDialect,
        call_kinds: Box<[u16]>,
    ) -> Result<CallsTest> {
        if self.seed {
            return Err(invalid(format!(
                "{path}: a calls rule cannot contain calls"
            )));
        }
        let Split {
            through,
            chain,
            inner,
            references,
            definition,
            links,
            constructor,
        } = split(value, path)?;
        if (references || definition) && Family::of(language) != Family::Python {
            return Err(invalid(format!(
                "{path}: reference graphs and definition queries require Python"
            )));
        }
        if (links.is_some() || constructor.is_some()) && !references {
            return Err(invalid(format!(
                "{path}/siblingLinks requires graph references"
            )));
        }
        let links =
            contracts::compile(links.as_ref(), constructor.as_ref(), path, self.max_pattern)?;
        let mode = Mode {
            through,
            references,
            definition,
            links,
        };
        let digest = |canonical: serde_json::Result<String>| -> Result<Arc<str>> {
            let canonical = canonical.map_err(|error| invalid(error.to_string()))?;
            Ok(format!("{:x}", Sha256::digest(canonical.as_bytes())).into())
        };
        let key = digest(serde_json::to_string(&(
            through_name(through),
            references,
            definition,
            contracts::get(links).canonical.as_ref(),
            &inner,
            &self.salt,
        )))?;
        let seed_key = digest(serde_json::to_string(&(&inner, &self.salt)))?;
        self.found.borrow_mut().push(FoundTerm {
            key: Arc::clone(&key),
            seed_key,
            through: mode,
            inner,
            path: path.to_owned(),
        });
        Ok(CallsTest {
            key,
            family: Family::of(language),
            language,
            call_kinds,
            chain,
            definition,
        })
    }
}

fn through_name(through: Through) -> &'static str {
    match through {
        Through::Sync => "sync",
        Through::Any => "any",
    }
}

/// A `calls` value's parts: its own keys and its inner rule (every other key).
pub(crate) struct Split {
    pub(crate) through: Through,
    pub(crate) chain: Option<String>,
    pub(crate) inner: Value,
    pub(crate) references: bool,
    pub(crate) definition: bool,
    pub(crate) links: Option<Value>,
    pub(crate) constructor: Option<Value>,
}

/// The keys of a `calls` value that are its own; the rest is its inner rule.
const OWN_KEYS: [&str; 6] = [
    "through",
    "chain",
    "graph",
    "at",
    "siblingLinks",
    "constructorFields",
];

pub(crate) fn split(value: &Value, path: &str) -> Result<Split> {
    let calls = object(value, path)?;
    let references = match calls.get("graph").and_then(Value::as_str) {
        None if !calls.contains_key("graph") => false,
        Some("calls") => false,
        Some("references") => true,
        _ => return Err(invalid(format!("{path}/graph must be calls or references"))),
    };
    let definition = match calls.get("at").and_then(Value::as_str) {
        None if !calls.contains_key("at") => false,
        Some("call") => false,
        Some("definition") => true,
        _ => return Err(invalid(format!("{path}/at must be call or definition"))),
    };
    if references && !definition {
        return Err(invalid(format!(
            "{path}: graph references requires at definition"
        )));
    }
    let through = match calls.get("through") {
        None => Through::Any,
        Some(Value::String(through)) if through == "sync" => Through::Sync,
        Some(Value::String(through)) if through == "any" => Through::Any,
        Some(_) => {
            return Err(invalid(format!(
                "{path}/through must be \"sync\" or \"any\""
            )));
        }
    };
    let chain = match calls.get("chain") {
        None => None,
        Some(Value::String(name))
            if !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_') =>
        {
            Some(name.clone())
        }
        Some(_) => {
            return Err(invalid(format!(
                "{path}/chain must be a capture name ([A-Z0-9_]+, without $)"
            )));
        }
    };
    let inner = calls
        .iter()
        .filter(|(key, _)| !OWN_KEYS.contains(&key.as_str()))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<Map<_, _>>();
    Ok(Split {
        through,
        chain,
        inner: Value::Object(inner),
        references,
        definition,
        links: calls.get("siblingLinks").cloned(),
        constructor: calls.get("constructorFields").cloned(),
    })
}

/// Which definitions a seed query's witnesses stand for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SeedScope {
    /// The witnesses are the definitions the inner rule matches.
    Definitions,
    /// The inner rule is `{ has: { stopBy: { kind: function_definition }, ...R } }`: the witnesses
    /// match `R`, and each stands for the innermost function around it.
    Innermost,
    /// The inner rule is `{ has: { stopBy: "end", ...R } }`: each witness stands for every
    /// function around it.
    Enclosing,
}

/// Where an inner rule only asks what a definition's body holds, the rule for what it holds
/// and how its matches map back to definitions: the query then runs at the matched nodes, as a
/// top-level rule does, not from every definition down. `R`'s `kind` must be another than a
/// definition's (one of `function_kinds`), so the stop node itself is never a match. Python's
/// definitions are `function_definition`s; PHP's are several kinds, with no one kind for
/// `Innermost` to stop at, so a PHP rule runs at held nodes only for `stopBy: "end"`.
fn body_rule(inner: &Value, function_kinds: &[&str]) -> Option<(Value, SeedScope)> {
    let Value::Object(inner) = inner else {
        return None;
    };
    let (key, Value::Object(has)) = inner.iter().next().filter(|_| inner.len() == 1)? else {
        return None;
    };
    if key != "has" || has.contains_key("field") {
        return None;
    }
    let scope = match (has.get("stopBy")?, function_kinds) {
        (Value::String(end), _) if end == "end" => SeedScope::Enclosing,
        (Value::Object(stop), [definition])
            if stop.len() == 1 && stop.get("kind").and_then(Value::as_str) == Some(definition) =>
        {
            SeedScope::Innermost
        }
        _ => return None,
    };
    match has.get("kind") {
        Some(Value::String(kind)) if !function_kinds.contains(&kind.as_str()) => {}
        _ => return None,
    }
    let mut body = has.clone();
    body.remove("stopBy");
    Some((Value::Object(body), scope))
}

/// The seed query of an inner rule in `language` (of `family`): the definitions it matches (or
/// the nodes they hold, see `SeedScope`), with the query's utilities but those holding a
/// `calls` term, or reaching one through `matches` (a seed cannot hold `calls`). `Err` where the
/// inner rule holds or reaches one itself. The seed runs in the family's seed language
/// (`CallRules::SEED_LANGUAGE`) over the definitions a call may reach
/// (`CallRules::FUNCTION_KINDS`).
pub(crate) fn seed_query(
    inner: &Value,
    ast: &Map<String, Value>,
    path: &str,
    language: &str,
    family: Family,
) -> Result<(Value, SeedScope)> {
    let nested = || invalid(format!("{path}: a calls rule cannot contain calls"));
    if mentions(inner, &|key, _| key == "calls") {
        return Err(nested());
    }
    let (seed_language, function_kinds) = with_adapter!(family, A => (
        <<A as Adapter>::Calls as CallRules>::SEED_LANGUAGE,
        <<A as Adapter>::Calls as CallRules>::FUNCTION_KINDS,
    ));
    let language = seed_language.unwrap_or(language);
    let mut seed = Map::new();
    seed.insert("language".into(), Value::String(language.into()));
    let (rule, scope) = body_rule(inner, function_kinds).unwrap_or_else(|| {
        let definitions = match function_kinds {
            [kind] => serde_json::json!({ "kind": kind }),
            kinds => serde_json::json!({
                "any": kinds.iter().map(|kind| serde_json::json!({ "kind": kind })).collect::<Vec<_>>()
            }),
        };
        (
            serde_json::json!({ "all": [definitions, inner] }),
            SeedScope::Definitions,
        )
    });
    seed.insert("rule".into(), rule);
    if let Some(options) = ast.get("languageOptions") {
        seed.insert("languageOptions".into(), options.clone());
    }
    if let Some(Value::Object(utils)) = ast.get("utils") {
        let mut dropped = utils
            .iter()
            .filter(|(_, util)| mentions(util, &|key, _| key == "calls"))
            .map(|(id, _)| id.clone())
            .collect::<std::collections::BTreeSet<_>>();
        loop {
            let reaching = utils
                .iter()
                .filter(|(id, util)| {
                    !dropped.contains(*id)
                        && mentions(util, &|key, value| {
                            key == "matches" && calls_util(value, &dropped)
                        })
                })
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>();
            if reaching.is_empty() {
                break;
            }
            dropped.extend(reaching);
        }
        if mentions(inner, &|key, value| {
            key == "matches" && calls_util(value, &dropped)
        }) {
            return Err(nested());
        }
        let kept = utils
            .iter()
            .filter(|(id, _)| !dropped.contains(*id))
            .map(|(id, util)| (id.clone(), util.clone()))
            .collect::<Map<_, _>>();
        if !kept.is_empty() {
            seed.insert("utils".into(), Value::Object(kept));
        }
    }
    Ok((serde_json::json!({ "ast": seed }), scope))
}

/// Whether `value` holds a key and value `hit` accepts, at any depth.
fn mentions(value: &Value, hit: &dyn Fn(&str, &Value) -> bool) -> bool {
    match value {
        Value::Object(map) => map
            .iter()
            .any(|(key, item)| hit(key, item) || mentions(item, hit)),
        Value::Array(items) => items.iter().any(|item| mentions(item, hit)),
        _ => false,
    }
}

/// Whether a `matches` value names one of `utils`: `"id"` or `{ "id": { ...arguments } }`.
fn calls_util(matches: &Value, utils: &std::collections::BTreeSet<String>) -> bool {
    match matches {
        Value::String(id) => utils.contains(id),
        Value::Object(call) => call.keys().any(|id| utils.contains(id)),
        _ => false,
    }
}

/// A term's index among a workspace's (or a stateless evaluation's) `calls` terms.
pub(crate) type TermId = u32;

/// One file's seeds: the terms whose seed query matched each definition (by node start), and
/// those whose query is unknown for the whole file (it failed, or read around syntax errors).
/// Each list is sorted.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct FileSeeds {
    pub(crate) matched: BTreeMap<u32, Vec<TermId>>,
    pub(crate) unknown: Vec<TermId>,
}

impl FileSeeds {
    /// Records that `term`'s seed query matched the definition at `start`; terms are recorded in
    /// increasing order.
    pub(crate) fn matched(&mut self, start: u32, term: TermId) {
        let terms = self.matched.entry(start).or_default();
        if terms.last() != Some(&term) {
            terms.push(term);
        }
    }
}

/// What one term asks of the graph: its mode, its seed definitions and the files where its seed
/// query is unknown. Terms asking the same share one answer.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Question {
    through: Mode,
    seeds: Vec<DefRef>,
    unknown: Vec<u32>,
}

/// A term's answer: the definitions reaching its seeds, each with its hop, and those that may
/// through a file where its seed query is unknown.
#[derive(Debug, PartialEq)]
struct Answer {
    reach: Region,
    unknown: HashSet<DefRef>,
}

impl Answer {
    fn defs(&self) -> impl Iterator<Item = &DefRef> {
        self.reach.keys().chain(&self.unknown)
    }

    fn holds(&self, def: &DefRef) -> bool {
        self.reach.contains_key(def) || self.unknown.contains(def)
    }

    /// Adds to `moved` the definitions whose verdict or chain differs from `old`'s: those that
    /// entered or left either region, those whose hop changed, and every reaching definition
    /// whose chain, in either answer, passes through one of them.
    fn moved_from(&self, old: &Answer, moved: &mut HashSet<DefRef>) {
        moved.extend(self.unknown.symmetric_difference(&old.unknown));
        let mut changed = old
            .reach
            .iter()
            .filter(|(def, hop)| self.reach.get(def) != Some(hop))
            .map(|(def, _)| *def)
            .collect::<HashSet<_>>();
        changed.extend(self.reach.keys().filter(|def| !old.reach.contains_key(def)));
        for answer in [old, self] {
            let mut passes = HashMap::<DefRef, bool>::new();
            for &start in answer.reach.keys() {
                let mut path = Vec::new();
                let mut at = start;
                let through = loop {
                    if changed.contains(&at) {
                        break true;
                    }
                    if let Some(&known) = passes.get(&at) {
                        break known;
                    }
                    path.push(at);
                    match answer.reach[&at] {
                        Hop::Seed => break false,
                        Hop::Via(next) => at = next,
                    }
                };
                for def in path {
                    passes.insert(def, through);
                }
                if through {
                    moved.insert(start);
                }
            }
        }
        moved.extend(changed);
    }
}

/// A `calls` term's answer for one callee: the call reaches its seed, or may.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Verdict {
    pub(crate) reach: bool,
    /// Not known to reach: a definition on the way calls something unknown.
    pub(crate) open: bool,
}

/// A linked call graph and each term's answer over it: what `calls` terms read. Answers are
/// computed when first read, shared by terms asking the same, and kept across updates until an
/// edit can change them.
type OpenRegions = BTreeMap<(bool, contracts::Key, usize), OnceLock<Arc<HashSet<DefRef>>>>;

pub(crate) struct CallsView {
    graph: Graph,
    references: BTreeMap<contracts::Key, Graph>,
    keys: Vec<Arc<str>>,
    ids: HashMap<Arc<str>, TermId>,
    questions: Vec<Question>,
    answers: Vec<OnceLock<Arc<Answer>>>,
    shared: parking_lot::Mutex<HashMap<Question, Arc<Answer>>>,
    /// Per mode, the definitions that may reach something unknown: every term's open answer
    /// but for its unknown seed files.
    open: OpenRegions,
}

impl Clone for CallsView {
    fn clone(&self) -> Self {
        Self {
            graph: self.graph.clone(),
            references: self.references.clone(),
            keys: self.keys.clone(),
            ids: self.ids.clone(),
            questions: self.questions.clone(),
            answers: self.answers.clone(),
            shared: parking_lot::Mutex::new(self.shared.lock().clone()),
            open: self.open.clone(),
        }
    }
}

impl CallsView {
    pub(crate) fn reference_rows(&self, key: contracts::Key) -> Option<closure::ReferenceRows> {
        self.references.get(&key).map(Graph::reference_rows)
    }
    /// Links `summaries` for `terms` with `seeds`; no term is answered yet.
    #[cfg(test)]
    pub(crate) fn build(
        terms: Vec<(Arc<str>, Through)>,
        summaries: Vec<(Arc<str>, Arc<Summary>)>,
        seeds: &HashMap<Arc<str>, FileSeeds>,
    ) -> Self {
        Self::build_modes(
            terms
                .into_iter()
                .map(|(key, through)| (key, through.into()))
                .collect(),
            summaries,
            seeds,
        )
    }

    pub(crate) fn build_modes(
        terms: Vec<(Arc<str>, Mode)>,
        summaries: Vec<(Arc<str>, Arc<Summary>)>,
        seeds: &HashMap<Arc<str>, FileSeeds>,
    ) -> Self {
        let link_keys = terms
            .iter()
            .filter(|(_, mode)| mode.references)
            .map(|(_, mode)| mode.links)
            .collect::<BTreeSet<_>>();
        let references = link_keys
            .iter()
            .map(|links| {
                (
                    *links,
                    Graph::link_references(
                        *links,
                        summaries
                            .iter()
                            .map(|(path, summary)| {
                                (Arc::clone(path), Arc::new(summary.as_references(*links)))
                            })
                            .collect(),
                    ),
                )
            })
            .collect();
        let mut open = BTreeMap::new();
        for through in Through::ALL {
            open.insert(Mode::from(through).index(), OnceLock::new());
            for links in &link_keys {
                open.insert(
                    Mode {
                        through,
                        references: true,
                        definition: false,
                        links: *links,
                    }
                    .index(),
                    OnceLock::new(),
                );
            }
        }
        let graph = Graph::link(summaries);
        let questions = questions(&graph, &terms, seeds);
        Self {
            graph,
            references,
            ids: terms
                .iter()
                .enumerate()
                .map(|(id, (key, _))| (Arc::clone(key), id as TermId))
                .collect(),
            keys: terms.into_iter().map(|(key, _)| key).collect(),
            answers: vec![OnceLock::new(); questions.len()],
            questions,
            shared: parking_lot::Mutex::default(),
            open,
        }
    }

    /// Replaces, adds (`Some`) or removes (`None`) summaries and takes `seeds`, keeping every
    /// answer an edit cannot change. An answer is taken again when its question changed, or when
    /// the graph's shape changed and it holds a definition of a changed file (a removed edge's
    /// caller, a moved definition) or a changed file's edge calls into it. Returns the linked
    /// files whose stamp may have changed: the changed files, and those calling a definition
    /// whose answer changed.
    pub(crate) fn update(
        &mut self,
        changes: Vec<(Arc<str>, Option<Arc<Summary>>)>,
        seeds: &HashMap<Arc<str>, FileSeeds>,
    ) -> BTreeSet<Arc<str>> {
        let reference_update = self
            .references
            .iter_mut()
            .map(|(links, graph)| {
                (
                    *links,
                    graph.update(changes.iter().map(|(path, summary)| {
                        (
                            Arc::clone(path),
                            summary
                                .as_ref()
                                .map(|summary| Arc::new(summary.as_references(*links))),
                        )
                    })),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let update = self.graph.update(changes);
        // Sorted and distinct.
        let changed = &update.slots;
        let terms = self
            .keys
            .iter()
            .zip(&self.questions)
            .map(|(key, question)| (Arc::clone(key), question.through))
            .collect::<Vec<_>>();
        let questions = questions(&self.graph, &terms, seeds);
        let reaches_into = |answer: &Answer, graph: &Graph, update: &GraphUpdate| {
            let changed = &update.slots;
            update.reshaped
                && (answer
                    .defs()
                    .any(|def| changed.binary_search(&def.file()).is_ok())
                    || changed.iter().any(|slot| {
                        graph
                            .file_edges(*slot)
                            .iter()
                            .any(|(_, to)| answer.holds(to))
                    }))
        };
        let mut moved = HashSet::new();
        let mut shared = HashMap::<Question, Arc<Answer>>::new();
        for (term, question) in questions.iter().enumerate() {
            let Some(old) = self.answers[term].take() else {
                continue;
            };
            let graph = self.graph_for(question.through);
            let changed_graph = if question.through.references {
                &reference_update[&question.through.links]
            } else {
                &update
            };
            let answer = match shared.get(question) {
                Some(answer) => Arc::clone(answer),
                None if *question == self.questions[term]
                    && !reaches_into(&old, graph, changed_graph) =>
                {
                    Arc::clone(&old)
                }
                None => Arc::new(answer_of(graph, question)),
            };
            if !Arc::ptr_eq(&answer, &old) && answer != old {
                answer.moved_from(&old, &mut moved);
            }
            shared.insert(question.clone(), Arc::clone(&answer));
            let _ = self.answers[term].set(answer);
        }
        self.questions = questions;
        *self.shared.get_mut() = shared;
        for key in self.open.keys().copied().collect::<Vec<_>>() {
            let through = Mode {
                through: if key.2 == 0 {
                    Through::Sync
                } else {
                    Through::Any
                },
                references: key.0,
                definition: false,
                links: key.1,
            };
            let Some(old) = self.open.get_mut(&key).and_then(OnceLock::take) else {
                continue;
            };
            let graph = self.graph_for(through);
            let changed_graph = if through.references {
                &reference_update[&through.links]
            } else {
                &update
            };
            let open = if changed_graph.reshaped {
                Arc::new(open_after(
                    graph,
                    through.through,
                    old,
                    changed_graph,
                    &mut moved,
                ))
            } else {
                old
            };
            let _ = self.open[&key].set(open);
        }
        let mut files = changed
            .iter()
            .filter_map(|slot| self.graph.path(*slot).cloned())
            .collect::<BTreeSet<_>>();
        if !moved.is_empty() {
            files.extend(self.graph.calling(&moved));
            files.extend(
                moved
                    .iter()
                    .filter_map(|def| self.graph.path(def.file()).cloned()),
            );
            for graph in self.references.values() {
                files.extend(graph.calling(&moved));
            }
        }
        files
    }

    /// Swaps in `path`'s new summary, which reads the same as its linked one but for where its
    /// definitions start.
    pub(crate) fn resummarize(&mut self, path: &str, summary: Arc<Summary>) {
        for (links, graph) in &mut self.references {
            graph.resummarize(path, Arc::new(summary.as_references(*links)));
        }
        self.graph.resummarize(path, summary);
    }

    fn graph_for(&self, mode: Mode) -> &Graph {
        if mode.references {
            &self.references[&mode.links]
        } else {
            &self.graph
        }
    }

    fn graph(&self, term: TermId) -> &Graph {
        self.graph_for(self.questions[term as usize].through)
    }

    pub(crate) fn term(&self, key: &str) -> Option<TermId> {
        self.ids.get(key).copied()
    }

    fn answer(&self, term: TermId) -> &Arc<Answer> {
        self.answers[term as usize].get_or_init(|| {
            let question = &self.questions[term as usize];
            if let Some(answer) = self.shared.lock().get(question) {
                return Arc::clone(answer);
            }
            let answer = Arc::new(answer_of(self.graph(term), question));
            Arc::clone(self.shared.lock().entry(question.clone()).or_insert(answer))
        })
    }

    fn open(&self, through: Mode) -> &HashSet<DefRef> {
        self.open[&through.index()]
            .get_or_init(|| Arc::new(open_region(self.graph_for(through), through.through)))
    }

    fn verdict_of(&self, defs: &[DefRef], open: bool, term: TermId) -> Verdict {
        let answer = self.answer(term);
        if defs.iter().any(|def| answer.reach.contains_key(def)) {
            return Verdict {
                reach: true,
                open: false,
            };
        }
        let through = self.questions[term as usize].through;
        Verdict {
            reach: false,
            open: open
                || defs
                    .iter()
                    .any(|def| answer.unknown.contains(def) || self.open(through).contains(def)),
        }
    }

    /// Term `term`'s answer for a call of `name`.
    pub(crate) fn verdict(&self, name: &Name, term: TermId) -> Verdict {
        let (defs, open) = self.graph(term).link_of(name);
        self.verdict_of(&defs, open, term)
    }

    /// The definitions a call of `name` reaches `term`'s seed through, along a shortest chain:
    /// `a.fetch -> b.pause`.
    pub(crate) fn chain(&self, name: &Name, term: TermId) -> String {
        let graph = self.graph(term);
        let (defs, _) = graph.link_of(name);
        let answer = self.answer(term);
        let mut chain = Vec::new();
        let mut at = defs
            .iter()
            .find_map(|def| Some((*def, *answer.reach.get(def)?)));
        while let Some((def, hop)) = at {
            chain.push(graph.def_label(def));
            at = match hop {
                Hop::Seed => None,
                Hop::Via(next) => Some((next, answer.reach[&next])),
            };
        }
        chain.join(" -> ")
    }

    fn chain_of_definition(&self, def: DefRef, term: TermId) -> String {
        let graph = self.graph(term);
        let answer = self.answer(term);
        let mut chain = Vec::new();
        let mut at = Some(def);
        while let Some(def) = at {
            let Some(hop) = answer.reach.get(&def) else {
                break;
            };
            chain.push(graph.def_label(def));
            at = match hop {
                Hop::Seed => None,
                Hop::Via(next) => Some(*next),
            };
        }
        chain.join(" -> ")
    }

    /// Those of `terms` true at no call of `summary`'s file: it is fully known and no callee
    /// reaches their seeds. A callee that may reach them leaves the term unknown there, which a
    /// rule reading `calls` only positively (the only one this answers for: `CallsState::quiet`)
    /// matches nowhere either.
    pub(crate) fn silent(&self, summary: &Summary, terms: &[TermId]) -> Vec<TermId> {
        if summary.open {
            return Vec::new();
        }
        terms
            .iter()
            .copied()
            .filter(|term| {
                let mode = self.questions[*term as usize].through;
                if mode.definition {
                    return false;
                }
                let targets = if mode.references {
                    &summary.reference_targets
                } else {
                    &summary.targets
                };
                !targets
                    .iter()
                    .any(|target| self.verdict(summary.interned(*target), *term).reach)
            })
            .collect()
    }

    /// What `summary`'s file reads of `terms`' answers: every callee name's verdict for each, and
    /// the chain of each it reaches, hashed. A file whose stamp is unchanged gets the same `calls`
    /// verdicts and chains at every call.
    pub(crate) fn stamp(&self, path: &str, summary: &Summary, terms: &[TermId]) -> HashKey {
        let mut bytes = Vec::with_capacity(summary.targets.len() * 16);
        for &term in terms {
            let mode = self.questions[term as usize].through;
            let graph = self.graph(term);
            let targets = if mode.definition {
                summary
                    .defs
                    .iter()
                    .map(|def| (def.name, Some(def.start)))
                    .collect::<Vec<_>>()
            } else {
                (if mode.references {
                    &summary.reference_targets
                } else {
                    &summary.targets
                })
                .iter()
                .map(|name| (*name, None))
                .collect()
            };
            for (target, start) in targets {
                let name = summary.interned(target);
                let verdict = if let Some(start) = start {
                    graph
                        .def_at(path, start)
                        .map_or_else(Verdict::default, |def| self.verdict_of(&[def], false, term))
                } else {
                    self.verdict(name, term)
                };
                if verdict == Verdict::default() {
                    continue;
                }
                bytes.extend_from_slice(name.text().as_bytes());
                bytes.push(0);
                bytes.extend_from_slice(self.keys[term as usize].as_bytes());
                if let Some(start) = start {
                    bytes.extend_from_slice(&start.to_le_bytes());
                }
                bytes.push(u8::from(verdict.reach) | (u8::from(verdict.open) << 1));
                if verdict.reach {
                    let chain = start
                        .and_then(|start| graph.def_at(path, start))
                        .map_or_else(
                            || self.chain(name, term),
                            |def| self.chain_of_definition(def, term),
                        );
                    bytes.extend_from_slice(chain.as_bytes());
                }
                bytes.push(0);
            }
        }
        HashKey::sha256(&bytes)
    }
}

/// Each term's question over `graph`: its seeds (as the graph names the definitions `seeds` give
/// by start) and its unknown files.
fn questions(
    graph: &Graph,
    terms: &[(Arc<str>, Mode)],
    seeds: &HashMap<Arc<str>, FileSeeds>,
) -> Vec<Question> {
    let mut questions = terms
        .iter()
        .map(|(_, through)| Question {
            through: *through,
            seeds: Vec::new(),
            unknown: Vec::new(),
        })
        .collect::<Vec<_>>();
    for (path, file) in seeds {
        for (start, matched) in &file.matched {
            let Some(def) = graph.def_at(path, *start) else {
                continue;
            };
            for term in matched {
                questions[*term as usize].seeds.push(def);
            }
        }
        if let Some(slot) = graph.slot(path) {
            for term in &file.unknown {
                questions[*term as usize].unknown.push(slot);
            }
        }
    }
    for question in &mut questions {
        question.seeds.sort_unstable();
        question.unknown.sort_unstable();
    }
    questions
}

fn answer_of(graph: &Graph, question: &Question) -> Answer {
    let allows = |flags| question.through.allows(flags);
    Answer {
        reach: graph.reach(&question.seeds, allows),
        unknown: if question.unknown.is_empty() {
            HashSet::new()
        } else {
            graph.upward(
                question
                    .unknown
                    .iter()
                    .flat_map(|slot| graph.file_defs(*slot)),
                allows,
            )
        },
    }
}

fn open_region(graph: &Graph, through: Through) -> HashSet<DefRef> {
    graph.upward(graph.open_defs(), |flags| through.allows(flags))
}

/// `through`'s open region after `update`, from the one before it: the definitions that reach a
/// changed one through the region are taken out, then those of them and of the changed that
/// are open, or call into what remains, are put back with what calls them. Definitions whose
/// membership changed are `moved`.
fn open_after(
    graph: &Graph,
    through: Through,
    mut region: Arc<HashSet<DefRef>>,
    update: &GraphUpdate,
    moved: &mut HashSet<DefRef>,
) -> HashSet<DefRef> {
    let allows = |def: DefRef| through.allows(graph.flags(def));
    let mut region = std::mem::take(Arc::make_mut(&mut region));
    for def in &update.dropped {
        if region.remove(def) {
            moved.insert(*def);
        }
    }
    let mut taken = HashSet::new();
    let mut stack = update
        .touched
        .iter()
        .filter(|def| region.contains(*def))
        .copied()
        .collect::<Vec<_>>();
    while let Some(def) = stack.pop() {
        if !region.remove(&def) {
            continue;
        }
        taken.insert(def);
        stack.extend(
            graph
                .callers_of(def)
                .iter()
                .filter(|caller| region.contains(*caller)),
        );
    }
    let mut stack = taken
        .iter()
        .chain(&update.touched)
        .copied()
        .filter(|def| {
            graph.holds(*def)
                && allows(*def)
                && (graph.is_open(*def) || graph.callees_of(*def).any(|to| region.contains(&to)))
        })
        .collect::<Vec<_>>();
    while let Some(def) = stack.pop() {
        if !region.insert(def) {
            continue;
        }
        if !taken.contains(&def) {
            moved.insert(def);
        }
        stack.extend(
            graph
                .callers_of(def)
                .iter()
                .filter(|caller| allows(**caller) && !region.contains(*caller)),
        );
    }
    moved.extend(taken.into_iter().filter(|def| !region.contains(def)));
    region
}

/// The graph and summary `calls` terms read while one file is matched, and the callee of each
/// call already asked about: every term at a call names the same callee.
struct FileCalls {
    view: Arc<CallsView>,
    summary: Arc<Summary>,
    callees: HashMap<(usize, usize), Option<Option<Name>>>,
}

thread_local! {
    static FILE_CALLS: RefCell<Option<FileCalls>> = const { RefCell::new(None) };
}

/// Installs a file's closure and summary for the `calls` terms of its evaluation.
pub(crate) struct FileCallsSlot {
    previous: Option<FileCalls>,
}

impl FileCallsSlot {
    pub(crate) fn install(view: Arc<CallsView>, summary: Arc<Summary>) -> Self {
        let previous = FILE_CALLS.replace(Some(FileCalls {
            view,
            summary,
            callees: HashMap::new(),
        }));
        Self { previous }
    }

    /// The slot of `path` in the evaluation `EvaluationCalls` installed, if it has a summary.
    pub(crate) fn for_path(path: &str) -> Option<Self> {
        let (view, summary) = EVALUATION.with_borrow(|evaluation| {
            let evaluation = evaluation.as_ref()?;
            Some((
                Arc::clone(&evaluation.view),
                Arc::clone(evaluation.summaries.get(path)?),
            ))
        })?;
        Some(Self::install(view, summary))
    }

    /// The closure and summary `for_path` installs for `path`, for the SQL engine to install
    /// where its terminals read `calls`.
    pub(crate) fn evaluation_calls(path: &str) -> Option<(Arc<CallsView>, Arc<Summary>)> {
        EVALUATION.with_borrow(|evaluation| {
            let evaluation = evaluation.as_ref()?;
            Some((
                Arc::clone(&evaluation.view),
                Arc::clone(evaluation.summaries.get(path)?),
            ))
        })
    }
}

impl Drop for FileCallsSlot {
    fn drop(&mut self) {
        FILE_CALLS.set(self.previous.take());
    }
}

/// A stateless evaluation's closure and every file's summary, which each file's slot reads.
pub(crate) struct EvaluationCalls {
    pub(crate) view: Arc<CallsView>,
    pub(crate) summaries: HashMap<String, Arc<Summary>>,
}

thread_local! {
    static EVALUATION: RefCell<Option<EvaluationCalls>> = const { RefCell::new(None) };
}

pub(crate) fn reference_rows(key: contracts::Key) -> Option<closure::ReferenceRows> {
    EVALUATION.with_borrow(|slot| slot.as_ref()?.view.reference_rows(key))
}

/// Installs an evaluation's `EvaluationCalls` until dropped.
pub(crate) struct EvaluationSlot {
    previous: Option<EvaluationCalls>,
}

impl EvaluationSlot {
    pub(crate) fn install(calls: EvaluationCalls) -> Self {
        Self {
            previous: EVALUATION.replace(Some(calls)),
        }
    }
}

impl Drop for EvaluationSlot {
    fn drop(&mut self) {
        EVALUATION.set(self.previous.take());
    }
}

/// How `calls` reads a file's bindings: `resolves`' default reading, as a stub where the path is
/// one (what `matcher::calls_tables` builds at a call node).
pub(crate) fn reading(path: &str) -> Reading {
    Reading {
        stub: is_stub_file(path),
        ..Reading::default()
    }
}

/// Nodes one summary's scope table may visit before the file's summary is open.
const MAX_TABLE_NODES: usize = 5_000_000;

/// Whether `calls` follows the calls of files in `language`: those of a family whose calls join
/// the call graph (`CallRules::SUPPORTED`).
pub(crate) fn summarizes(language: GrammarDialect) -> bool {
    with_adapter!(Family::of(language), A => <<A as Adapter>::Calls as CallRules>::SUPPORTED)
}

/// The document a `language` file's calls are read from, and its dialect: of a file that embeds
/// code (`embed`), the embedded program whose dialect's calls join the call graph (a Blade
/// template's view; `None` where it has none: it calls nothing); else the file's own.
pub(crate) fn calls_document(
    doc: crate::code_rule_dsl::tape::TapeDoc,
    language: GrammarDialect,
) -> Option<(crate::code_rule_dsl::tape::TapeDoc, GrammarDialect)> {
    if language.embeds().is_empty() {
        return Some((doc, language));
    }
    doc.guests()?.iter().find_map(|guest| {
        let dialect = *ast_grep_core::Doc::get_lang(&guest.doc);
        summarizes(dialect).then(|| (guest.doc.clone(), dialect))
    })
}

/// Extracts the summary of a `language` document whose root is `root` (a file's, or the embedded
/// program a file's calls are: `workspace::calls_state`), of a file of `bytes` bytes, through
/// `known`, the document's table as `calls` reads it where its evaluation built one, else a table
/// built here. A table (or PHP's name resolution) that cannot be built within the file's time
/// budget or `MAX_TABLE_NODES` leaves the summary open. Returns the table it built, for the
/// file's evaluation to reuse.
#[allow(clippy::too_many_arguments)]
pub fn summarize<D: Doc>(
    root: Node<'_, D>,
    language: GrammarDialect,
    path: &str,
    module: Option<&ModulePath>,
    has_errors: bool,
    bytes: usize,
    known: Option<&ScopeTable>,
) -> (Summary, Option<ScopeTable>) {
    let built = known.is_none().then(|| {
        ScopeTable::build_in(
            root.clone(),
            language,
            MAX_TABLE_NODES,
            &crate::code_rule_dsl::budget::matcher_budget_exhausted,
            reading(path),
            module,
        )
        .ok()
    });
    let table = known.or(built.as_ref().and_then(Option::as_ref));
    let mut summary = with_adapter!(Family::of(language), A => {
        <<A as Adapter>::Calls as CallRules>::summary(&root, path, module, table, has_errors)
    });
    summary.bytes = u32::try_from(bytes).unwrap_or(u32::MAX);
    (summary, built.flatten())
}

/// The summary of the PHP `program` in the file at `path`, through `table`, the file's scope
/// table where one was built.
pub(crate) fn extract_php<D: Doc>(
    program: &Node<'_, D>,
    path: &str,
    table: Option<&ScopeTable>,
    has_errors: bool,
) -> Summary {
    // The table's flow read the program's names; build them only where no table holds them.
    let names = table
        .and_then(|table| table.php.as_deref())
        .map(|flow| Arc::clone(&flow.names))
        .or_else(|| {
            PhpNames::build(
                program.clone(),
                MAX_TABLE_NODES,
                &crate::code_rule_dsl::budget::matcher_budget_exhausted,
            )
            .ok()
            .map(Arc::new)
        });
    php::extract(program, path, table, names.as_deref(), has_errors)
}

/// A seed query's witnesses in one file, by their spans: the definitions (by node start) they
/// stand for, read through the file's summary where they are nodes the definitions hold. `None`
/// without it.
pub(crate) fn seed_starts<'s>(
    spans: impl IntoIterator<Item = &'s Span>,
    scope: SeedScope,
    summary: Option<&Summary>,
) -> Option<Vec<u32>> {
    if scope == SeedScope::Definitions {
        return Some(
            spans
                .into_iter()
                .map(|span| span.start_byte as u32)
                .collect(),
        );
    }
    let summary = summary?;
    let mut starts = Vec::new();
    for span in spans {
        let (start, end) = (span.start_byte as u32, span.end_byte as u32);
        let around = summary
            .functions_around(start)
            .filter(|def| end <= def.end && (def.start, def.end) != (start, end));
        match scope {
            SeedScope::Innermost => starts.extend(around.take(1).map(|def| def.start)),
            _ => starts.extend(around.map(|def| def.start)),
        }
    }
    starts.sort_unstable();
    starts.dedup();
    Some(starts)
}

/// A `calls` verdict at a node.
pub(crate) enum Reach {
    /// The call reaches a definition the inner rule matches, through `chain`'s definitions (when
    /// the term binds it).
    Reaches(Option<String>),
    No,
    /// It may: an unknown callee or definition on the way. Also every call without an installed
    /// closure (not current): unknown, never absent.
    Unknown,
}

/// What `calls` reads in a file to name a callee the summary did not record.
pub(crate) enum FileTables {
    /// Python's scope table and the file's module path.
    Python(Arc<ScopeTable>, Option<Arc<ModulePath>>),
    /// PHP's variable flow and name resolution (a Blade template's: its embedded program's).
    Php(Arc<ScopeTable>, Arc<PhpNames>),
    /// Rust's scope table.
    Rust(Arc<ScopeTable>),
}

/// The `calls` verdict at `node`; `tables` gives what `calls` reads in `node`'s file for its
/// dialect (`None`: it could not be built).
pub(crate) fn holds<'t, D: Doc>(
    test: &CallsTest,
    node: &Node<'t, D>,
    tables: impl FnOnce(&Node<'t, D>, GrammarDialect) -> Option<FileTables>,
) -> Reach {
    if !test.call_kinds.contains(&node.kind_id()) {
        return Reach::No;
    }
    let range = node.range();
    let installed = FILE_CALLS.with_borrow(|slot| {
        slot.as_ref().map(|slot| {
            (
                Arc::clone(&slot.view),
                Arc::clone(&slot.summary),
                slot.callees.get(&(range.start, range.end)).cloned(),
            )
        })
    });
    let Some((view, summary, known)) = installed else {
        return Reach::Unknown;
    };
    let Some(term) = view.term(&test.key) else {
        return Reach::Unknown;
    };
    if test.definition {
        let Some(path) = crate::code_rule_dsl::matcher::file_path() else {
            return Reach::Unknown;
        };
        let Some(def) = view.graph(term).def_at(&path, range.start as u32) else {
            return Reach::No;
        };
        let verdict = view.verdict_of(&[def], false, term);
        return if verdict.reach {
            Reach::Reaches(
                test.chain
                    .as_ref()
                    .map(|_| view.chain_of_definition(def, term)),
            )
        } else if verdict.open {
            Reach::Unknown
        } else {
            Reach::No
        };
    }
    // `None`: no callee; `Some(None)`: an unknown one.
    let callee = match known {
        Some(callee) => callee,
        None => {
            // A table that cannot be built fails the clause for the file (`UTIL_EXCEEDED`).
            let Some(callee) = with_adapter!(test.family, A => <<A as Adapter>::Calls as CallRules>::callee_at(
                node,
                &summary,
                || tables(node, test.language),
            )) else {
                return Reach::Unknown;
            };
            // A callee's name is in the namespace of the family's summaries (Blade's are PHP's).
            let names =
                with_adapter!(test.family, A => <<A as Adapter>::Calls as CallRules>::NAMES);
            let callee = callee.map(|callee| match callee {
                Callee::Open => None,
                Callee::Name(name) => names.map(|family| Name::new(family, &name)),
            });
            FILE_CALLS.with_borrow_mut(|slot| {
                if let Some(slot) = slot {
                    slot.callees
                        .insert((range.start, range.end), callee.clone());
                }
            });
            callee
        }
    };
    match callee {
        None => Reach::No,
        Some(None) => Reach::Unknown,
        Some(Some(name)) => {
            let verdict = view.verdict(&name, term);
            if verdict.reach {
                Reach::Reaches(test.chain.as_ref().map(|_| view.chain(&name, term)))
            } else if verdict.open {
                Reach::Unknown
            } else {
                Reach::No
            }
        }
    }
}
