//! Candidate plans lower to SQL sets of (tape, preorder). The matcher retains capture semantics.
//!
//! Adapted from Pulling-it-all-together's `matcher/anchors/sql/lower.rs`. The sets read the
//! content store's per-tape prefilter rows (`content_store::prefilter`); `fid` names a tape
//! (`tape_id`) throughout, and the requested tapes are the `request_file` temp table. P decoded
//! node properties (parents, ends, fields, spans) into SQL tables to evaluate relations, pattern
//! parent hops and uncovered text exactly; here nothing is decoded (03 §7: transducers decode),
//! so each of those becomes a sound superset — the requirement narrowed to the tapes where it can
//! hold — and is marked inexact, so a negation never subtracts it. Tape-level filters composed
//! into the candidate set were good enough for word containment in ast-index-spike (history index
//! l.404, S192–S193), and each rule's sets stay scoped to its own candidate tapes (T2, S149).
use crate::code_rule_dsl::matcher::{AnchorPlan, Condition, Required, Requirement};
use crate::code_rule_dsl::stats::inputs::Statistics;
use rusqlite::types::Value;
use std::collections::{BTreeSet, HashMap};
#[path = "access.rs"]
mod access;
#[path = "lower/render.rs"]
mod render;
use render::Definitions;
pub(super) mod lifetime;
pub(super) mod ownership;
mod waves;

#[derive(Clone)]
struct Set {
    name: String,
    exact: bool,
}
#[derive(serde::Serialize)]
pub(super) struct Admission {
    pub consumer: usize,
    pub generated: usize,
    pub broadened: bool,
}
pub(super) struct Statement {
    pub admissions: Vec<Admission>,
    pub graph: Vec<String>,
    pub language: u32,
    pub files: Vec<i64>,
    pub queries: Vec<String>,
    pub stages: Vec<Stage>,
    pub schedule: lifetime::Schedule,
    pub parameters: Vec<Value>,
    pub ownership: ownership::Layout,
}
impl Statement {
    pub fn preparations(&self) -> usize {
        self.schedule.actions.iter().filter(|a| matches!(a, lifetime::Action::Prepare(_))).count()
    }
    /// Whether running the statement writes the store: it registers regex predicates and fills
    /// their vocabulary (`vocabulary.sql`). Everything else it creates is temporary.
    pub fn writes(&self) -> bool {
        self.stages.iter().any(|stage| stage.name.starts_with("vocabulary_"))
    }

    fn own(&mut self) {
        let compiled = ownership::compile(
            &self
                .stages
                .iter()
                .map(|s| lifetime::Input {
                    name: &s.name,
                    sql: &s.sql,
                    temporary: s.temporary,
                })
                .collect::<Vec<_>>(),
            &self.stages.iter().map(|s| s.index).collect::<Vec<_>>(),
            &self.queries,
            &self.graph,
        );
        for (i, (stage, sql)) in self.stages.iter_mut().zip(compiled.sql).enumerate() {
            stage.sql = sql;
            if compiled.layout.slot(i).is_some() {
                stage.index = None;
            }
        }
        self.queries = compiled.queries;
        self.schedule = compiled.schedule;
        self.ownership = compiled.layout;
    }
    #[cfg(test)]
    fn reschedule(&mut self) {
        self.schedule = lifetime::Schedule::new(
            &self
                .stages
                .iter()
                .map(|s| lifetime::Input {
                    name: &s.name,
                    sql: &s.sql,
                    temporary: s.temporary,
                })
                .collect::<Vec<_>>(),
            &self.queries,
        );
    }
}
struct Lower {
    ctes: Vec<String>,
    parameters: Vec<Value>,
    utilities: HashMap<(usize, String), Set>,
    /// The tape tag (`language::tape_tag`): kind atoms are `lang * LANG_STRIDE + code`.
    lang: u32,
    roots: HashMap<BTreeSet<u16>, Set>,
    limit: usize,
    indexed: BTreeSet<String>,
    reuse: Reuse,
    predicates: HashMap<String, (String, String)>,
    /// Plan inputs read before lowering (`stats::inputs`); empty statistics keep pattern order.
    statistics: Statistics,
}

/// SQL set identities, independent of the matcher's conservative exactness metadata.
#[derive(Default)]
struct Reuse {
    bodies: HashMap<String, String>,
    factors: HashMap<String, BTreeSet<String>>,
    /// Sets known to contain a set that is not their intersection: a union of their subsets.
    within: HashMap<String, BTreeSet<String>>,
    intersections: std::collections::BTreeMap<BTreeSet<String>, String>,
    strings: HashMap<String, usize>,
    integers: HashMap<i64, usize>,
    values: HashMap<(String, bool, Option<BTreeSet<u16>>), Set>,
    terms: HashMap<(String, bool), String>,
    kinds: HashMap<String, BTreeSet<u16>>,
}

impl Lower {
    fn new(lang: u32) -> Self {
        Self {
            ctes: Vec::new(),
            parameters: Vec::new(),
            utilities: HashMap::new(),
            lang,
            roots: HashMap::new(),
            limit: 256,
            indexed: BTreeSet::new(),
            reuse: Reuse::default(),
            predicates: HashMap::new(),
            statistics: Statistics::default(),
        }
    }
    fn parameter(&mut self, value: impl Into<Value>) -> String {
        let value = value.into();
        let existing = match &value {
            Value::Text(text) => self.reuse.strings.get(text),
            Value::Integer(number) => self.reuse.integers.get(number),
            _ => None,
        };
        if let Some(index) = existing {
            return format!("?{index}");
        }
        match &value {
            Value::Text(text) => {
                self.reuse
                    .strings
                    .insert(text.clone(), self.parameters.len() + 1);
            }
            Value::Integer(number) => {
                self.reuse
                    .integers
                    .insert(*number, self.parameters.len() + 1);
            }
            _ => {}
        }
        self.parameters.push(value);
        format!("?{}", self.parameters.len())
    }
    fn relation(&mut self, body: String, exact: bool) -> Result<Set, ()> {
        if let Some(name) = self.reuse.bodies.get(&body) {
            return Ok(Set {
                name: name.clone(),
                exact,
            });
        }
        if self.ctes.len() >= self.limit {
            return Err(());
        }
        let name = format!("s{}", self.ctes.len());
        self.ctes
            .push(format!("{name}(fid,pre) AS MATERIALIZED ({body})"));
        self.reuse.bodies.insert(body, name.clone());
        Ok(Set { name, exact })
    }
    fn all(&self) -> Set {
        Set {
            name: "all_nodes".into(),
            exact: true,
        }
    }
    fn combine(&mut self, left: &Set, right: &Set, operator: &str, exact: bool) -> Result<Set, ()> {
        let kinds = match (
            self.reuse.kinds.get(&left.name),
            self.reuse.kinds.get(&right.name),
            operator,
        ) {
            (Some(a), Some(b), "INTERSECT") => Some(a.intersection(b).copied().collect()),
            (Some(a), Some(b), "UNION") => Some(a.union(b).copied().collect()),
            (Some(a), _, "EXCEPT" | "INTERSECT") => Some(a.clone()),
            (_, Some(b), "INTERSECT") => Some(b.clone()),
            _ => None,
        };
        let selected = self.combine_rows(left, right, operator, exact)?;
        if let Some(kinds) = kinds {
            self.reuse.kinds.insert(selected.name.clone(), kinds);
        }
        Ok(selected)
    }
    fn combine_rows(
        &mut self,
        left: &Set,
        right: &Set,
        operator: &str,
        exact: bool,
    ) -> Result<Set, ()> {
        if operator == "INTERSECT" {
            if left.name == "all_nodes" {
                return Ok(Set {
                    name: right.name.clone(),
                    exact,
                });
            }
            if right.name == "all_nodes" {
                return Ok(Set {
                    name: left.name.clone(),
                    exact,
                });
            }
        }
        if left.name == right.name {
            if operator == "EXCEPT" {
                return self.relation("SELECT 0,0 WHERE 0".into(), exact);
            }
            return Ok(Set {
                name: left.name.clone(),
                exact,
            });
        }
        if operator == "INTERSECT" {
            if self.inside(&left.name, &right.name) {
                return Ok(Set {
                    name: left.name.clone(),
                    exact,
                });
            }
            if self.inside(&right.name, &left.name) {
                return Ok(Set {
                    name: right.name.clone(),
                    exact,
                });
            }
        }
        let factors = if operator == "INTERSECT" {
            let terms = |set: &Set| {
                self.reuse
                    .factors
                    .get(&set.name)
                    .cloned()
                    .unwrap_or_else(|| BTreeSet::from([set.name.clone()]))
            };
            let a = terms(left);
            let b = terms(right);
            if b.is_subset(&a) {
                return Ok(Set {
                    name: left.name.clone(),
                    exact,
                });
            }
            if a.is_subset(&b) {
                return Ok(Set {
                    name: right.name.clone(),
                    exact,
                });
            }
            let factors: BTreeSet<_> = a.union(&b).cloned().collect();
            if let Some(name) = self.reuse.intersections.get(&factors) {
                return Ok(Set {
                    name: name.clone(),
                    exact,
                });
            }
            Some(factors)
        } else {
            None
        };
        let selected = self.relation(
            format!(
                "SELECT fid,pre FROM {} {operator} SELECT fid,pre FROM {}",
                left.name, right.name
            ),
            exact,
        )?;
        if let Some(factors) = factors {
            let within = self
                .containing(&left.name)
                .union(&self.containing(&right.name))
                .cloned()
                .collect();
            self.reuse.within.insert(selected.name.clone(), within);
            self.reuse
                .factors
                .insert(selected.name.clone(), factors.clone());
            self.reuse
                .intersections
                .insert(factors, selected.name.clone());
        }
        Ok(selected)
    }
    /// Records that `set` was selected from `scope`'s rows.
    pub(super) fn narrowed(&mut self, set: Set, scope: &Set) -> Set {
        if set.name != scope.name && scope.name != "all_nodes" {
            let containing = self.containing(&scope.name);
            self.reuse
                .within
                .entry(set.name.clone())
                .or_default()
                .extend(containing);
        }
        set
    }
    /// The sets known to contain `name`, itself included.
    fn containing(&self, name: &str) -> BTreeSet<String> {
        let mut sets = BTreeSet::from([name.to_owned()]);
        sets.extend(self.reuse.factors.get(name).into_iter().flatten().cloned());
        sets.extend(self.reuse.within.get(name).into_iter().flatten().cloned());
        sets
    }
    /// Whether `set` is known to be a subset of `outer`, or of every factor of an intersection.
    fn inside(&self, set: &str, outer: &str) -> bool {
        let containing = self.containing(set);
        let within = |outer: &String| containing.contains(outer) || self.of_kinds(set, outer);
        within(&outer.to_owned())
            || self
                .reuse
                .factors
                .get(outer)
                .is_some_and(|factors| factors.iter().all(within))
    }
    /// The kinds of a kind posting set, without the error-node conventions of kinds 0 and MAX.
    fn root_kinds(&self, name: &str) -> Option<&BTreeSet<u16>> {
        self.roots
            .iter()
            .find(|(kinds, set)| {
                set.name == name && !kinds.contains(&0) && !kinds.contains(&u16::MAX)
            })
            .map(|(kinds, _)| kinds)
    }
    /// Whether every kind `set` can hold is in the kind posting set `outer`.
    fn of_kinds(&self, set: &str, outer: &str) -> bool {
        self.root_kinds(outer).is_some_and(|outer| {
            self.reuse
                .kinds
                .get(set)
                .is_some_and(|kinds| kinds.is_subset(outer))
        })
    }
    /// A union of restrictions of one set to kinds is that set restricted to all their kinds.
    fn factored(&mut self, names: &[String], exact: bool) -> Result<Option<Set>, ()> {
        let mut common: Option<BTreeSet<String>> = None;
        let mut kinds = BTreeSet::new();
        for name in names {
            let factors = self
                .reuse
                .factors
                .get(name)
                .cloned()
                .unwrap_or_else(|| BTreeSet::from([name.clone()]));
            let (roots, rest): (BTreeSet<_>, BTreeSet<_>) = factors
                .into_iter()
                .partition(|factor| self.root_kinds(factor).is_some());
            let Some(restricted) = roots
                .iter()
                .map(|root| self.root_kinds(root).expect("root").clone())
                .reduce(|a, b| a.intersection(&b).copied().collect())
            else {
                return Ok(None);
            };
            if rest.is_empty() || common.as_ref().is_some_and(|common| *common != rest) {
                return Ok(None);
            }
            kinds.extend(restricted);
            common = Some(rest);
        }
        let Some(common) = common else {
            return Ok(None);
        };
        let base = match common.len() {
            1 => common.first().cloned(),
            _ => self.reuse.intersections.get(&common).cloned(),
        };
        let Some(base) = base else {
            return Ok(None);
        };
        let base = Set { name: base, exact };
        let kinds = self.kinds(&kinds)?;
        self.combine(&base, &kinds, "INTERSECT", exact).map(Some)
    }
    /// One compound UNION: a chain of binary unions materializes and deduplicates every prefix.
    fn union(&mut self, branches: Vec<Set>) -> Result<Set, ()> {
        let empty = self.reuse.bodies.get("SELECT 0,0 WHERE 0").cloned();
        let exact = branches.iter().all(|set| set.exact);
        if branches.iter().any(|set| set.name == "all_nodes") {
            return Ok(Set {
                exact,
                ..self.all()
            });
        }
        let mut names: Vec<String> = Vec::new();
        for set in branches {
            if Some(&set.name) != empty.as_ref() && !names.contains(&set.name) {
                names.push(set.name);
            }
        }
        let kinds = names
            .iter()
            .map(|name| self.reuse.kinds.get(name))
            .collect::<Option<Vec<_>>>()
            .map(|kinds| {
                kinds
                    .into_iter()
                    .flatten()
                    .copied()
                    .collect::<BTreeSet<_>>()
            });
        if names.len() > 1 {
            if let Some(selected) = self.factored(&names, exact)? {
                return Ok(selected);
            }
        }
        // A branch that contains every other branch is the union.
        if let Some(name) = names
            .iter()
            .find(|outer| names.iter().all(|name| self.inside(name, outer)))
        {
            return Ok(Set {
                name: name.clone(),
                exact,
            });
        }
        let within = names
            .iter()
            .map(|name| self.containing(name))
            .reduce(|a, b| a.intersection(&b).cloned().collect());
        let selected = match names.as_slice() {
            [] => return self.relation("SELECT 0,0 WHERE 0".into(), exact),
            [name] => {
                return Ok(Set {
                    name: name.clone(),
                    exact,
                });
            }
            _ => {
                names.sort();
                let body = names
                    .iter()
                    .map(|name| format!("SELECT fid,pre FROM {name}"))
                    .collect::<Vec<_>>()
                    .join(" UNION ");
                self.relation(body, exact)?
            }
        };
        if let Some(within) = within {
            self.reuse.within.insert(selected.name.clone(), within);
        }
        if let Some(kinds) = kinds {
            self.reuse.kinds.insert(selected.name.clone(), kinds);
        }
        Ok(selected)
    }
    /// A kind's atom in this language. The tape stores tree-sitter's ERROR symbol (u16::MAX) as
    /// code 0, the grammar's end symbol, which never labels a node (`tape.rs`).
    fn atom(&self, kind: u16) -> i64 {
        let code = if kind == u16::MAX { 0 } else { u32::from(kind) };
        i64::from(self.lang * super::LANG_STRIDE + code)
    }
    fn kinds(&mut self, kinds: &BTreeSet<u16>) -> Result<Set, ()> {
        if let Some(set) = self.roots.get(kinds) {
            return Ok(set.clone());
        }
        // Symbol 0 labels no node: P told it from ERROR by a decoded flag; here it selects nothing.
        let atoms = kinds
            .iter()
            .filter(|&&kind| kind != 0)
            .map(|&kind| {
                let atom = self.atom(kind);
                self.parameter(atom)
            })
            .collect::<Vec<_>>();
        if atoms.is_empty() {
            return self.relation("SELECT 0,0 WHERE 0".into(), true);
        }
        // Seek the tape/kind count row before decoding its compact positions.
        let body = format!(
            "SELECT k.tape_id,p.pre FROM cf CROSS JOIN tape_kind_counts k ON k.tape_id=cf.fid CROSS JOIN nera_postings(k.postings) p WHERE k.kind IN ({})",
            atoms.join(",")
        );
        let selected = self.relation(body, true)?;
        self.roots.insert(kinds.clone(), selected.clone());
        // Base postings otherwise get re-scanned inside every physical consumer statement.
        self.indexed.insert(selected.name.clone());
        self.reuse
            .kinds
            .insert(selected.name.clone(), kinds.clone());
        Ok(selected)
    }
    fn plan(&mut self, plan: &AnchorPlan, scope: Set) -> Result<Set, ()> {
        let scope = match &plan.root_kinds {
            Some(kinds) => {
                let kinds = self.kinds(kinds)?;
                self.combine(&scope, &kinds, "INTERSECT", scope.exact)?
            }
            None => scope,
        };
        let kinds = self.reuse.kinds.get(&scope.name).cloned();
        let mut selected = match &plan.condition {
            Condition::Always => Ok(scope),
            Condition::Text { regex, .. } => self.text(&scope, regex.as_str(), false),
            Condition::Pattern(required) => self.pattern(required, scope),
            Condition::All(plans) => {
                // A conjunction commutes. Postings first, so no member reads a wider scope.
                let mut selected = scope;
                for plan in plans
                    .iter()
                    .filter(|p| anchored(p))
                    .chain(plans.iter().filter(|p| !anchored(p)))
                {
                    selected = self.plan(plan, selected)?;
                }
                Ok(selected)
            }
            Condition::Any(plans) => {
                // Kind-only branches read one posting list, intersected with the scope once.
                let (only, rest): (Vec<_>, Vec<_>) = plans.iter().partition(|plan| {
                    plan.root_kinds.is_some() && matches!(plan.condition, Condition::Always)
                });
                let mut branches = rest
                    .into_iter()
                    .map(|plan| self.plan(plan, scope.clone()))
                    .collect::<Result<Vec<_>, _>>()?;
                if let [plan] = only.as_slice() {
                    branches.push(self.plan(plan, scope.clone())?);
                } else if !only.is_empty() {
                    let kinds = only
                        .iter()
                        .flat_map(|plan| plan.root_kinds.iter().flatten().copied())
                        .collect();
                    let kinds = self.kinds(&kinds)?;
                    let exact = scope.exact && only.iter().all(|plan| plan.exact);
                    branches.push(self.combine(&scope, &kinds, "INTERSECT", exact)?);
                }
                self.union(branches)
            }
            Condition::Not(inner) => {
                let inner = self.plan(inner, scope.clone())?;
                if inner.exact {
                    self.combine(&scope, &inner, "EXCEPT", scope.exact)
                } else {
                    Ok(Set {
                        exact: false,
                        ..scope
                    })
                }
            }
            Condition::Utility(inner) => {
                let key = (std::sync::Arc::as_ptr(inner) as usize, scope.name.clone());
                if let Some(selected) = self.utilities.get(&key) {
                    return Ok(Set {
                        exact: selected.exact && plan.exact,
                        ..selected.clone()
                    });
                }
                let selected = self.plan(inner, scope)?;
                self.utilities.insert(key, selected.clone());
                Ok(selected)
            }
            Condition::Relation { inner, .. } => self.related(scope, inner),
        }?;
        selected.exact &= plan.exact;
        if let Some(kinds) = kinds {
            self.reuse.kinds.insert(selected.name.clone(), kinds);
        }
        Ok(selected)
    }
    /// Both ends of a relation lie in one tape. P walked decoded parents, ends and fields to
    /// relate them; undecoded, the scope keeps its nodes in tapes where the inner plan has a
    /// candidate. An inner plan not anchored on postings is every node, which narrows nothing.
    fn related(&mut self, scope: Set, inner: &AnchorPlan) -> Result<Set, ()> {
        if !anchored(inner) {
            return Ok(Set {
                exact: false,
                ..scope
            });
        }
        let all = self.all();
        let inner = self.plan(inner, all)?;
        self.within_tapes(scope, &[inner])
    }

    /// `scope`'s nodes in tapes where every set in `sets` has a node: a superset.
    fn within_tapes(&mut self, scope: Set, sets: &[Set]) -> Result<Set, ()> {
        let tests = sets
            .iter()
            .map(|set| format!("s.fid IN (SELECT fid FROM {})", set.name))
            .collect::<Vec<_>>();
        if tests.is_empty() {
            return Ok(Set {
                exact: false,
                ..scope
            });
        }
        let selected = self.relation(
            format!(
                "SELECT s.fid,s.pre FROM {} s WHERE {}",
                scope.name,
                tests.join(" AND ")
            ),
            false,
        )?;
        Ok(self.narrowed(selected, &scope))
    }

    /// A pattern root holds every requirement in its subtree. P reached roots by decoded parent
    /// hops from the requirement with the fewest postings (counted per request) and confirmed
    /// subtree spans. Here a requirement without parents is the root itself, so its postings bound
    /// the roots, and the others narrow them to the tapes that hold every requirement, rarest
    /// first by the statistics read before lowering (D10), not by a count per request.
    fn pattern(&mut self, required: &[Requirement], scope: Set) -> Result<Set, ()> {
        if required.is_empty() {
            return Ok(scope);
        }
        let mut roots = scope.clone();
        let mut inside = Vec::new();
        let mut order: Vec<_> = required.iter().enumerate().collect();
        order.sort_by_key(|(ordinal, requirement)| {
            let estimate = self.statistics.choose(self.lang, std::slice::from_ref(*requirement)).1;
            (estimate.map_or(u64::MAX, |estimate| estimate.rows), *ordinal)
        });
        for (_, requirement) in order {
            let (kind, literal) = match &requirement.value {
                Required::Kind(kind) => (*kind, None),
                Required::Terminal(kind, text) => (*kind, Some(text)),
            };
            let mut posting = self.kinds(&BTreeSet::from([kind]))?;
            if let Some(text) = literal {
                posting = self.text(&posting, text, true)?;
            }
            if requirement.parents.is_empty() {
                roots = self.combine(&roots, &posting, "INTERSECT", false)?;
            } else {
                inside.push(posting);
            }
        }
        self.within_tapes(roots, &inside)
    }
}

/// The candidate sets of `plans`, over the requested tapes of language `lang` (a tape tag) that
/// have prefilter rows: `execute::request` fills `request_file`.
pub(super) fn batch(plans: &[&AnchorPlan], lang: u32, files: &[i64], statistics: &Statistics) -> Statement {
    batch_scope(plans, lang, files, statistics)
}

fn scope(lang: u32, statistics: &Statistics) -> Lower {
    let mut lower = Lower::new(lang);
    lower.statistics = statistics.clone();
    let lang_parameter = lower.parameter(i64::from(lang));
    lower.ctes.push(format!(
        "cf(fid) AS MATERIALIZED (SELECT r.fid FROM request_file r JOIN tape_objects t ON t.tape_id=r.fid JOIN prefilter_sets s ON s.tape_id=r.fid WHERE t.grammar_id={lang_parameter})"
    ));
    lower.ctes.push("all_nodes(fid,pre) AS NOT MATERIALIZED (SELECT cf.fid,p.value FROM cf JOIN prefilter_sets s ON s.tape_id=cf.fid CROSS JOIN generate_series(0,s.nodes-1,1) p)".into());
    lower
}

/// Whether a plan selects from kind postings before it reads its scope: then evaluating it over
/// every node costs only its postings. A conjunction evaluates its anchored members first (`plan`).
fn anchored(plan: &AnchorPlan) -> bool {
    plan.root_kinds.is_some()
        || match &plan.condition {
            Condition::Pattern(required) => !required.is_empty(),
            Condition::All(plans) => plans.iter().any(anchored),
            Condition::Any(plans) => plans.iter().all(anchored),
            Condition::Utility(inner) => anchored(inner),
            // A relation keeps its scope's nodes (`related`).
            Condition::Always
            | Condition::Text { .. }
            | Condition::Not(_)
            | Condition::Relation { .. } => false,
        }
}
fn batch_scope(plans: &[&AnchorPlan], lang: u32, fids: &[i64], statistics: &Statistics) -> Statement {
    use super::graph::program::Program;
    let mut program = Program::default();
    let mut roots = Vec::new();
    let mut indexed_nodes = BTreeSet::new();
    let mut predicates = std::collections::BTreeMap::new();
    let mut admissions = Vec::new();
    for (qid, plan) in plans.iter().enumerate() {
        // Admission belongs to the individual plan. Another consumer's cached
        // producers cannot expand or shrink this plan's existing 256-node allowance.
        let mut lower = scope(lang, statistics);
        let checkpoint = lower.ctes.len();
        lower.limit = checkpoint + 256;
        let attempted = lower.plan(plan, lower.all());
        admissions.push(Admission {
            consumer: qid,
            generated: lower.ctes.len() - checkpoint,
            broadened: attempted.is_err(),
        });
        let selected = match attempted {
            Ok(selected) => selected,
            Err(()) => {
                // Discard the entire failed fragment, including predicates, parameters
                // and reuse state. Earlier fragments already belong to the shared program.
                lower = scope(lang, statistics);
                lower.limit = lower.ctes.len() + 256;
                match &plan.root_kinds {
                    Some(kinds) => lower.kinds(kinds).unwrap_or_else(|_| lower.all()),
                    None => lower.all(),
                }
            }
        };
        let definitions = Definitions::new(std::mem::take(&mut lower.ctes));
        let live = definitions
            .live(&format!("SELECT fid,pre FROM {}", selected.name))
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        let nodes = program.add(&live, &lower.parameters, lang);
        roots.push((nodes[&selected.name].id, qid));
        for name in lower.indexed {
            if let Some(node) = nodes.get(&name) {
                indexed_nodes.insert(node.id);
            }
        }
        for (name, (language, expression)) in lower.predicates {
            let Some(node) = nodes.get(&name) else {
                continue;
            };
            let value = |parameter: &str| {
                lower.parameters[parameter
                    .strip_prefix('?')
                    .expect("bound value")
                    .parse::<usize>()
                    .expect("parameter index")
                    - 1]
                .clone()
            };
            predicates
                .entry(node.id)
                .or_insert_with(|| (value(&language), value(&expression)));
        }
    }
    let layout = program.finish();
    // Group output arms by the canonical producer, while retaining caller identities.
    // Packet boundaries then depend on shared work rather than allocation order.
    roots.sort_unstable_by_key(|&(node, qid)| (layout.ranks[&node], qid));
    let mut arms = roots
        .iter()
        .map(|(node, qid)| format!("SELECT {qid} AS qid,fid,pre FROM {}", layout.names[node]))
        .collect::<Vec<_>>();
    if arms.is_empty() {
        arms.push("SELECT 0 AS qid,0 AS fid,0 AS pre WHERE 0".into());
    }
    let queries = arms
        .chunks(128)
        .map(|arms| arms.join(" UNION ALL ") + " ORDER BY qid,fid,pre")
        .collect();
    let indexed = indexed_nodes
        .into_iter()
        .map(|node| layout.names[&node].clone())
        .collect();
    let values: BTreeSet<_> = predicates
        .values()
        .map(|(language, expression)| {
            format!(
                "({},{},0)",
                layout.bound(language),
                layout.bound(expression)
            )
        })
        .collect();
    let mut statement = prepare(layout.ctes, queries, layout.parameters, indexed);
    statement.admissions = admissions;
    if !values.is_empty() {
        let stages = [
            (
                "vocabulary_register",
                format!(
                    "INSERT OR IGNORE INTO vocabulary_predicate(lang,expression,ready) VALUES {}",
                    values.into_iter().collect::<Vec<_>>().join(",")
                ),
            ),
            (
                "vocabulary_literals",
                "INSERT OR IGNORE INTO vocabulary_literal SELECT p.lang,p.expression,j.value FROM vocabulary_predicate p CROSS JOIN json_each(nera_literal_set(p.expression)) j WHERE p.ready=0".to_owned(),
            ),
            (
                "vocabulary_fill",
                include_str!("vocabulary-fill.sql").to_owned(),
            ),
            (
                "vocabulary_complete",
                "UPDATE vocabulary_predicate SET ready=1 WHERE ready=0".to_owned(),
            ),
            (
                "vocabulary_release",
                "DELETE FROM vocabulary_pending".to_owned(),
            ),
        ]
        .into_iter()
        .map(|(name, sql)| Stage {
            name: name.into(),
            sql,
            index: None,
            temporary: false,
        })
        .collect::<Vec<_>>();
        statement.stages.splice(0..0, stages);
    }
    statement.language = lang;
    statement.files = fids.to_vec();
    statement.own();
    statement
}

#[derive(Clone)]
pub(super) struct Stage {
    pub name: String,
    pub sql: String,
    pub index: Option<&'static str>,
    pub temporary: bool,
}

/// SQLite expands CTE references during preparation, including AS MATERIALIZED CTEs.
/// Cut shared dependencies into real SQL tables before that expansion becomes exponential.
#[cfg(test)]
pub(super) fn finish(
    ctes: Vec<String>,
    queries: Vec<String>,
    parameters: Vec<Value>,
    indexed: BTreeSet<String>,
) -> Statement {
    let mut statement = prepare(ctes, queries, parameters, indexed);
    statement.reschedule();
    statement
}

fn prepare(
    ctes: Vec<String>,
    queries: Vec<String>,
    parameters: Vec<Value>,
    indexed: BTreeSet<String>,
) -> Statement {
    let query = queries.join("\n");
    // Candidate broadening and approximate negation can discard a whole dependency tree.
    // Only materialize tables reachable from the final batch outputs.
    let ctes: Vec<_> = Definitions::new(ctes)
        .live(&query)
        .into_iter()
        .cloned()
        .collect();
    let (_, uses) = waves::analyze(&ctes, &BTreeSet::new(), &query);
    let graph = ctes.clone();
    let names: Vec<_> = ctes
        .iter()
        .map(|cte| cte.split_once('(').expect("CTE columns").0.to_owned())
        .collect();
    let mut inline = Definitions::default();
    let mut stages = Vec::new();
    let mut expansion: HashMap<&str, usize> = HashMap::new();
    for (name, cte) in names.iter().zip(ctes) {
        let columns = cte.split_once(") AS ").expect("CTE header").0.to_owned();
        let index = if columns.contains("fid") && columns.contains("pre") {
            Some("fid,pre")
        } else if columns == "cf(fid" {
            Some("fid")
        } else {
            None
        };
        let body = cte.split_once(" AS ").expect("CTE body").1;
        let cost = 1usize.saturating_add(
            waves::identifiers(body)
                .filter_map(|token| expansion.get(token))
                .copied()
                .sum::<usize>(),
        );
        // Keep small shared sets in one statement. Cut before repeated dependencies
        // multiply beyond SQLite's preparation limits; each physical table resets the cost.
        let shared = indexed.contains(name) || (uses[name] > 1 && cost >= 32);
        expansion.insert(name.as_str(), if shared { 1 } else { cost });
        let cte = if shared {
            render::owned(cte, name)
        } else {
            cte
        };
        inline.push(cte);
        if shared {
            // CTAS otherwise inherits empty affinity from FTS columns and compound CTEs.
            // Every candidate ordinal is an integer.
            let projection = columns
                .split_once('(')
                .expect("CTE columns")
                .1
                .split(',')
                .map(|column| {
                    if column == "term" {
                        "term".to_owned()
                    } else {
                        format!("CAST({column} AS INTEGER) AS {column}")
                    }
                })
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "CREATE TEMP TABLE \"{name}\" AS {}",
                inline.render(&format!("SELECT {projection} FROM {name}"))
            );
            stages.push(Stage {
                name: name.clone(),
                sql,
                index,
                temporary: true,
            });
            inline.pop();
        }
    }
    let queries: Vec<_> = queries.iter().map(|query| inline.render(query)).collect();
    let schedule = lifetime::Schedule {
        actions: Vec::new(),
        peak_relations: 0,
    };
    Statement {
        admissions: Vec::new(),
        graph,
        language: 0,
        files: Vec::new(),
        queries,
        stages,
        schedule,
        parameters,
        ownership: ownership::Layout::default(),
    }
}

#[cfg(test)]
#[path = "lower_tests.rs"]
mod tests;
