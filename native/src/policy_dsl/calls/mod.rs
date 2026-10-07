//! Cross-file `calls`: a rule relation that follows a Python or PHP call into the callee's
//! definition, wherever it lives. Per-file call summaries (`summary`) are linked into one call graph
//! (`closure`); each `calls` term is answered by a backward walk from the definitions its inner
//! rule matches, computed when first read and kept until an edit can change it. The inner rule
//! runs per file as the term's seed query; the callee's subtree is never walked at query time.

pub(crate) mod closure;
pub(crate) mod names;
pub(crate) mod php;
pub(crate) mod summary;

#[cfg(test)]
mod php_tests;
#[cfg(test)]
mod tests;

use crate::content_store::HashKey;
use crate::policy_dsl::json::{invalid, object};
use crate::policy_dsl::language::is_stub_file;
use crate::policy_dsl::language::{Family, GrammarDialect};
use crate::policy_dsl::module_path::ModulePath;
use crate::policy_dsl::scope::php_names::PhpNames;
use crate::policy_dsl::scope::{Reading, ScopeTable};
use ast_grep_core::{Doc, Node};
use closure::{DefRef, Graph, Hop, Region};
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

/// The `calls` term at a call node: its term's key.
#[derive(Clone, Debug)]
pub(crate) struct CallsTest {
    pub(crate) key: Arc<str>,
    /// The language whose calls the term follows.
    pub(crate) family: Family,
    /// The grammar's call kinds: the only nodes the term holds at.
    pub(crate) call_kinds: Box<[u16]>,
    /// `chain`: the capture a reaching call binds to its chain of definitions
    /// (`a.fetch -> b.pause`), for messages.
    pub(crate) chain: Option<String>,
}

/// A term found while compiling a query's rules, before its seed query is compiled.
pub(crate) struct FoundTerm {
    pub(crate) key: Arc<str>,
    pub(crate) seed_key: Arc<str>,
    pub(crate) through: Through,
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
}

impl CallsCollector {
    pub(crate) fn new(salt: String, seed: bool) -> Self {
        Self {
            salt,
            seed,
            found: RefCell::new(Vec::new()),
        }
    }

    /// The terms found, each key once, in the order first found.
    pub(crate) fn take(&self) -> Vec<FoundTerm> {
        let mut found = self.found.take();
        let mut seen = std::collections::HashSet::new();
        found.retain(|term| seen.insert(Arc::clone(&term.key)));
        found
    }

    /// Records the `calls` value at `path` and returns its test at `family`'s `call_kinds` nodes.
    pub(crate) fn term(
        &self,
        value: &Value,
        path: &str,
        family: Family,
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
        } = split(value, path)?;
        let digest = |canonical: serde_json::Result<String>| -> Result<Arc<str>> {
            let canonical = canonical.map_err(|error| invalid(error.to_string()))?;
            Ok(format!("{:x}", Sha256::digest(canonical.as_bytes())).into())
        };
        let key = digest(serde_json::to_string(&(
            through_name(through),
            &inner,
            &self.salt,
        )))?;
        let seed_key = digest(serde_json::to_string(&(&inner, &self.salt)))?;
        self.found.borrow_mut().push(FoundTerm {
            key: Arc::clone(&key),
            seed_key,
            through,
            inner,
            path: path.to_owned(),
        });
        Ok(CallsTest {
            key,
            family,
            call_kinds,
            chain,
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
}

/// The keys of a `calls` value that are its own; the rest is its inner rule.
const OWN_KEYS: [&str; 2] = ["through", "chain"];

pub(crate) fn split(value: &Value, path: &str) -> Result<Split> {
    let calls = object(value, path)?;
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
/// definition's, so the stop node itself is never a match. Python's definitions are
/// `function_definition`s; PHP's are `function_kinds`, with no one kind for `Innermost` to stop
/// at, so a PHP rule runs at held nodes only for `stopBy: "end"`.
fn body_rule(inner: &Value, family: Family) -> Option<(Value, SeedScope)> {
    let Value::Object(inner) = inner else {
        return None;
    };
    let (key, Value::Object(has)) = inner.iter().next().filter(|_| inner.len() == 1)? else {
        return None;
    };
    if key != "has" || has.contains_key("field") {
        return None;
    }
    let scope = match has.get("stopBy")? {
        Value::String(end) if end == "end" => SeedScope::Enclosing,
        Value::Object(stop)
            if family == Family::Python
                && stop.len() == 1
                && stop.get("kind") == Some(&Value::String("function_definition".into())) =>
        {
            SeedScope::Innermost
        }
        _ => return None,
    };
    match has.get("kind") {
        Some(Value::String(kind)) if !function_kinds(family).contains(&kind.as_str()) => {}
        _ => return None,
    }
    let mut body = has.clone();
    body.remove("stopBy");
    Some((Value::Object(body), scope))
}

/// The node kinds of what a call may reach: the definitions a seed query runs on. A Blade
/// template's definitions (in `@php` blocks) are its embedded PHP's.
pub(crate) fn function_kinds(family: Family) -> &'static [&'static str] {
    match family {
        Family::Python => &["function_definition"],
        Family::Php | Family::Blade => &php::FUNCTION_KINDS,
        Family::Ecma => &[],
    }
}

/// The seed query of an inner rule in `language` (of `family`): the definitions it matches (or
/// the nodes they hold, see `SeedScope`), with the query's utilities but those holding a
/// `calls` term, or reaching one through `matches` (a seed cannot hold `calls`). `Err` where the
/// inner rule holds or reaches one itself. A Blade policy's seeds are the workspace's PHP
/// definitions, which its templates' calls reach: its seed query is PHP's.
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
    let (language, family) = match family {
        Family::Blade => ("php", Family::Php),
        Family::Python | Family::Php | Family::Ecma => (language, family),
    };
    let mut seed = Map::new();
    seed.insert("language".into(), Value::String(language.into()));
    let (rule, scope) = body_rule(inner, family).unwrap_or_else(|| {
        let definitions = match function_kinds(family) {
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
    through: Through,
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
pub(crate) struct CallsView {
    graph: Graph,
    keys: Vec<Arc<str>>,
    ids: HashMap<Arc<str>, TermId>,
    questions: Vec<Question>,
    answers: Vec<OnceLock<Arc<Answer>>>,
    shared: parking_lot::Mutex<HashMap<Question, Arc<Answer>>>,
    /// Per mode, the definitions that may reach something unknown: every term's open answer
    /// but for its unknown seed files.
    open: [OnceLock<Arc<HashSet<DefRef>>>; 2],
}

impl Clone for CallsView {
    fn clone(&self) -> Self {
        Self {
            graph: self.graph.clone(),
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
    /// Links `summaries` for `terms` with `seeds`; no term is answered yet.
    pub(crate) fn build(
        terms: Vec<(Arc<str>, Through)>,
        summaries: Vec<(Arc<str>, Arc<Summary>)>,
        seeds: &HashMap<Arc<str>, FileSeeds>,
    ) -> Self {
        let graph = Graph::link(summaries);
        let questions = questions(&graph, &terms, seeds);
        Self {
            graph,
            ids: terms
                .iter()
                .enumerate()
                .map(|(id, (key, _))| (Arc::clone(key), id as TermId))
                .collect(),
            keys: terms.into_iter().map(|(key, _)| key).collect(),
            answers: vec![OnceLock::new(); questions.len()],
            questions,
            shared: parking_lot::Mutex::default(),
            open: Default::default(),
        }
    }

    /// Replaces, adds (`Some`) or removes (`None`) summaries and takes `seeds`, keeping every
    /// answer an edit cannot change. An answer is taken again when its question changed, when
    /// it holds a definition of a changed file (a removed edge's caller, a moved definition), or
    /// when a changed file's edge calls into it. Returns the linked files whose stamp may have
    /// changed: the changed files, and those calling a definition whose answer changed.
    pub(crate) fn update(
        &mut self,
        changes: Vec<(Arc<str>, Option<Arc<Summary>>)>,
        seeds: &HashMap<Arc<str>, FileSeeds>,
    ) -> BTreeSet<Arc<str>> {
        let changed = self.graph.update(changes);
        let slots = changed.iter().copied().collect::<HashSet<_>>();
        let terms = self
            .keys
            .iter()
            .zip(&self.questions)
            .map(|(key, question)| (Arc::clone(key), question.through))
            .collect::<Vec<_>>();
        let questions = questions(&self.graph, &terms, seeds);
        let graph = &self.graph;
        let reaches_into = |answer: &Answer| {
            answer.defs().any(|def| slots.contains(&def.file()))
                || changed.iter().any(|slot| {
                    graph
                        .file_edges(*slot)
                        .iter()
                        .any(|(_, to)| answer.holds(to))
                })
        };
        let mut moved = HashSet::new();
        let mut shared = HashMap::<Question, Arc<Answer>>::new();
        for (term, question) in questions.iter().enumerate() {
            let Some(old) = self.answers[term].take() else {
                continue;
            };
            let answer = match shared.get(question) {
                Some(answer) => Arc::clone(answer),
                None if *question == self.questions[term] && !reaches_into(&old) => {
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
        for through in Through::ALL {
            let slot = &mut self.open[through.index()];
            let Some(old) = slot.take() else {
                continue;
            };
            let open = if changed.is_empty() {
                old
            } else {
                let open = Arc::new(open_region(&self.graph, through));
                moved.extend(old.symmetric_difference(&open).copied());
                open
            };
            let _ = slot.set(open);
        }
        let mut files = changed
            .iter()
            .filter_map(|slot| self.graph.path(*slot).cloned())
            .collect::<BTreeSet<_>>();
        if !moved.is_empty() {
            files.extend(self.graph.calling(&moved));
        }
        files
    }

    /// Swaps in `path`'s new summary, which reads the same as its linked one but for where its
    /// definitions start.
    pub(crate) fn resummarize(&mut self, path: &str, summary: Arc<Summary>) {
        self.graph.resummarize(path, summary);
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
            let answer = Arc::new(answer_of(&self.graph, question));
            Arc::clone(self.shared.lock().entry(question.clone()).or_insert(answer))
        })
    }

    fn open(&self, through: Through) -> &HashSet<DefRef> {
        self.open[through.index()].get_or_init(|| Arc::new(open_region(&self.graph, through)))
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
        let (defs, open) = self.graph.link_of(name);
        self.verdict_of(&defs, open, term)
    }

    /// The definitions a call of `name` reaches `term`'s seed through, along a shortest chain:
    /// `a.fetch -> b.pause`.
    pub(crate) fn chain(&self, name: &Name, term: TermId) -> String {
        let (defs, _) = self.graph.link_of(name);
        let answer = self.answer(term);
        let mut chain = Vec::new();
        let mut at = defs
            .iter()
            .find_map(|def| Some((*def, *answer.reach.get(def)?)));
        while let Some((def, hop)) = at {
            chain.push(self.graph.def_label(def));
            at = match hop {
                Hop::Seed => None,
                Hop::Via(next) => Some((next, answer.reach[&next])),
            };
        }
        chain.join(" -> ")
    }

    /// Those of `terms` false at every call of `summary`'s file: it is fully known, no call names
    /// something unknown, and no callee reaches or may reach their seeds.
    pub(crate) fn silent(&self, summary: &Summary, terms: &[TermId]) -> Vec<TermId> {
        if summary.open || summary.open_sites {
            return Vec::new();
        }
        let mut silent = terms.to_vec();
        for target in &summary.targets {
            if silent.is_empty() {
                break;
            }
            let (defs, open) = self.graph.link_of(summary.interned(*target));
            silent.retain(|term| self.verdict_of(&defs, open, *term) == Verdict::default());
        }
        silent
    }

    /// What `summary`'s file reads of `terms`' answers: every callee name's verdict for each, and
    /// the chain of each it reaches, hashed. A file whose stamp is unchanged gets the same `calls`
    /// verdicts and chains at every call.
    pub(crate) fn stamp(&self, summary: &Summary, terms: &[TermId]) -> HashKey {
        let mut bytes = Vec::with_capacity(summary.targets.len() * 16);
        for target in &summary.targets {
            let name = summary.interned(*target);
            let (defs, open) = self.graph.link_of(name);
            for &term in terms {
                let verdict = self.verdict_of(&defs, open, term);
                if verdict == Verdict::default() {
                    continue;
                }
                bytes.extend_from_slice(name.text().as_bytes());
                bytes.push(0);
                bytes.extend_from_slice(self.keys[term as usize].as_bytes());
                bytes.push(u8::from(verdict.reach) | (u8::from(verdict.open) << 1));
                if verdict.reach {
                    bytes.extend_from_slice(self.chain(name, term).as_bytes());
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
    terms: &[(Arc<str>, Through)],
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

/// Whether `calls` follows the calls of files in `language`: Python's, PHP's, and those of the
/// PHP a Blade template embeds (`blade::view`).
pub(crate) fn summarizes(language: GrammarDialect) -> bool {
    match Family::of(language) {
        Family::Python | Family::Php | Family::Blade => true,
        Family::Ecma => false,
    }
}

/// Extracts the summary of a Python, PHP or Blade (`language`) file of `bytes` bytes whose tree
/// is `root` (a Blade file's a tape's, holding its embedded program), through `known`, the file's
/// table as `calls` reads it where its evaluation built one, else a table built here. A table (or
/// PHP's name resolution) that cannot be built within the file's time budget or
/// `MAX_TABLE_NODES` leaves the summary open. Returns the table it built, for the file's
/// evaluation to reuse.
#[allow(clippy::too_many_arguments)]
pub(crate) fn summarize<D: Doc>(
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
            &crate::policy_dsl::budget::matcher_budget_exhausted,
            reading(path),
            module,
        )
        .ok()
    });
    let table = known.or(built.as_ref().and_then(Option::as_ref));
    let extract_php = |program: &Node<'_, D>| {
        // The table's flow read the program's names; build them only where no table holds them.
        let names = table
            .and_then(|table| table.php.as_deref())
            .map(|flow| Arc::clone(&flow.names))
            .or_else(|| {
                PhpNames::build(
                    program.clone(),
                    MAX_TABLE_NODES,
                    &crate::policy_dsl::budget::matcher_budget_exhausted,
                )
                .ok()
                .map(Arc::new)
            });
        php::extract(program, path, table, names.as_deref(), has_errors)
    };
    let mut summary = match Family::of(language) {
        Family::Php => extract_php(&root),
        // A template's calls are its embedded program's; a template without one calls nothing.
        Family::Blade => match crate::policy_dsl::blade::program(&root) {
            Some(program) => extract_php(&program),
            None => Summary::default(),
        },
        Family::Python | Family::Ecma => Summary::extract(&root, table, module, has_errors),
    };
    summary.bytes = u32::try_from(bytes).unwrap_or(u32::MAX);
    (summary, built.flatten())
}

/// A seed query's witnesses in one file, by their spans: the definitions (by node start) they
/// stand for, read through the file's summary where they are nodes the definitions hold. `None`
/// without it.
pub(crate) fn seed_starts<'s>(
    spans: impl IntoIterator<Item = &'s Value>,
    scope: SeedScope,
    summary: Option<&Summary>,
) -> Option<Vec<u32>> {
    let at = |span: &Value, key: &str| span.get(key).and_then(Value::as_u64).map(|at| at as u32);
    if scope == SeedScope::Definitions {
        return Some(
            spans
                .into_iter()
                .filter_map(|span| at(span, "startByte"))
                .collect(),
        );
    }
    let summary = summary?;
    let mut starts = Vec::new();
    for span in spans {
        let (Some(start), Some(end)) = (at(span, "startByte"), at(span, "endByte")) else {
            continue;
        };
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
    // `None`: no callee; `Some(None)`: an unknown one.
    let callee = match known {
        Some(callee) => callee,
        None => {
            // A table that cannot be built fails the check for the file (`UTIL_EXCEEDED`).
            let callee = match test.family {
                Family::Python => {
                    let Some(FileTables::Python(table, module)) =
                        tables(node, GrammarDialect::Python)
                    else {
                        return Reach::Unknown;
                    };
                    summary::callee_at(&table, module.as_deref(), &summary, node)
                }
                // A Blade template's calls are its embedded program's (`blade::view`); its
                // top level is no class's, so `$this`, `self` and `static` there are unknown.
                Family::Php | Family::Blade => {
                    let language = match test.family {
                        Family::Blade => GrammarDialect::Blade,
                        Family::Php | Family::Python | Family::Ecma => GrammarDialect::Php,
                    };
                    let Some(FileTables::Php(table, names)) = tables(node, language) else {
                        return Reach::Unknown;
                    };
                    let reader = php::Reader {
                        names: &names,
                        table: &table,
                        root: node.root().root(),
                    };
                    php::callee_at(&reader, &summary, node)
                }
                Family::Ecma => return Reach::Unknown,
            }
            .map(|callee| match callee {
                Callee::Open => None,
                Callee::Name(name) => Some(Name::new(&name)),
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
