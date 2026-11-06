//! Candidate plans lower to SQL sets of (tape, preorder). The matcher retains capture semantics.
//!
//! Adapted from Pulling-it-all-together's `matcher/anchors/sql/lower.rs`. The sets read the
//! content store's per-tape prefilter rows (`content_store::prefilter`); `fid` names a tape
//! (`tape_id`) throughout, and the requested tapes are the `request_file` temp table. P decoded
//! node properties (parents, ends, fields, spans) into SQL tables to evaluate relations, pattern
//! parent hops and uncovered text exactly; here nothing is decoded (03 §7: transducers decode),
//! so each of those becomes a sound superset — the requirement narrowed to the tapes where it can
//! hold — and is marked inexact, so a negation never subtracts it.
//!
//! The pure-SQL fallback (03 §17, `fallback`) lowers the same plans with relations exact: each
//! relation reads the tape's structure through `nera_tree` and `nera_size` (`tapes.rs`), only at
//! the scope's nodes, so nothing is decoded into tables (ast-index-spike Representation rule 3).
//! Pattern parent hops and uncovered text stay supersets; `Statement::exact` names the plans
//! whose rows are exactly their matches' roots. Tape-level filters composed
//! into the candidate set were good enough for word containment in ast-index-spike (history index
//! l.404, S192–S193), and each rule's sets stay scoped to its own candidate tapes (T2, S149).
use crate::code_rule_dsl::matcher::{AnchorPlan, Condition, Required, Requirement};
use crate::code_rule_dsl::stats::inputs::Statistics;
use rusqlite::types::Value;
use std::collections::{BTreeSet, HashMap};
#[path = "access.rs"]
mod access;
#[path = "decode.rs"]
#[allow(dead_code)]
mod decode;
use decode::Property;
#[path = "lower/render.rs"]
mod render;
use render::Definitions;
#[allow(dead_code)]
mod children;
pub(super) mod lifetime;
pub(super) mod ownership;
mod waves;
#[cfg(feature = "code-rule-profile")]
pub(super) mod working_set;

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
    /// Per plan, whether its rows are exactly its rule's roots (the fallback's relations exact).
    pub exact: Vec<bool>,
    pub graph: Vec<String>,
    pub language: u32,
    pub files: Vec<i64>,
    pub queries: Vec<String>,
    pub stages: Vec<Stage>,
    pub schedule: lifetime::Schedule,
    pub parameters: Vec<Value>,
    pub ownership: ownership::Layout,
}

/// A plan's output arm: a set's rows, or (`whole`) every node of the file scope's (`node`) tapes
/// where each of `whole`'s sets has a node.
struct Root {
    node: usize,
    qid: usize,
    whole: Option<Vec<usize>>,
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
    /// Relations read the tape (`nera_tree`) and are exact: the §17 fallback.
    tree: bool,
    roots: HashMap<BTreeSet<u16>, Set>,
    limit: usize,
    indexed: BTreeSet<String>,
    reuse: Reuse,
    decoding: HashMap<String, decode::Demand>,
    predicates: HashMap<String, (String, String)>,
    /// Plan inputs read before lowering (`stats::inputs`); empty statistics keep pattern order.
    statistics: Statistics,
    producers: crate::code_rule_dsl::producers::Registry<u32>,
    consumer: usize,
    producer_bindings: HashMap<Property, usize>,
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
    decoded: HashMap<(String, usize), String>,
    kinds: HashMap<String, BTreeSet<u16>>,
    /// Each `EXCEPT` set's (left, right): a later narrowing to tapes narrows its left side
    /// before the difference (`within_tapes`).
    differences: HashMap<String, (String, String)>,
    /// Each union's branches, and each set of every node in the tapes where each of its sets
    /// has a node (`within_tapes` of every node): a plan whose output is a union of them emits
    /// the latter as one row a tape (`batch_scope`).
    unions: HashMap<String, Vec<String>>,
    wholes: HashMap<String, Vec<String>>,
}

impl Lower {
    fn new(lang: u32) -> Self {
        Self {
            ctes: Vec::new(),
            parameters: Vec::new(),
            utilities: HashMap::new(),
            lang,
            tree: false,
            roots: HashMap::new(),
            limit: 256,
            indexed: BTreeSet::new(),
            reuse: Reuse::default(),
            decoding: HashMap::new(),
            predicates: HashMap::new(),
            statistics: Statistics::default(),
            producers: Default::default(),
            consumer: 0,
            producer_bindings: HashMap::new(),
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
        if operator == "EXCEPT" {
            self.reuse.differences.insert(selected.name.clone(), (left.name.clone(), right.name.clone()));
        }
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
                let selected = self.relation(body, exact)?;
                self.reuse.unions.insert(selected.name.clone(), names.clone());
                selected
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
            "SELECT k.tape_id,p.pre FROM cf CROSS JOIN tape_kind_counts k ON k.tape_id=cf.fid CROSS JOIN nera_segment(k.postings) p WHERE k.kind IN ({})",
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
            Condition::Relation { inner, direction, recursive } if self.tree => self.related_on_tape(scope, inner, *direction, *recursive),
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

    /// The fallback's relation: `scope`'s nodes related to a node of `inner` as the matcher relates
    /// them (`matcher::anchors::relation_selection`), read from the tape at each scope node:
    /// children and the parent from `nera_tree`, descendants and ancestors by interval
    /// (`nera_size`), siblings as the parent's children. Exact when both sides are.
    fn related_on_tape(&mut self, scope: Set, inner: &AnchorPlan, direction: crate::code_rule_dsl::matcher::Direction, recursive: bool) -> Result<Set, ()> {
        use crate::code_rule_dsl::matcher::Direction;
        if !anchored(inner) {
            return self.related(scope, inner);
        }
        let all = self.all();
        let inner = self.plan(inner, all)?;
        let (i, s) = (&inner.name, &scope.name);
        // Uncorrelated set operations, not an `EXISTS` per scope row (re-run per row: spike S203,
        // O32). Each side is read from where the neighbour is one `nera_tree` step away: an inner
        // node's parent or ancestors, a scope node's parent or siblings. `IN (SELECT fid,pre …)`
        // is indexed once by SQLite.
        let parent_of = |x: &str| format!("JOIN nera_tree p ON p.fid={x}.fid AND p.node={x}.pre AND p.axis=2");
        let next = |x: &str| format!("{x}.pre+nera_size({x}.fid,{x}.pre)");
        let body = match (direction, recursive) {
            // A child of the scope node is in inner: the parents of inner's nodes.
            (Direction::Has, false) => format!("SELECT i.fid,p.pre FROM {i} i {} INTERSECT SELECT fid,pre FROM {s}", parent_of("i")),
            (Direction::Has, true) => format!("SELECT i.fid,a.pre FROM {i} i JOIN nera_tree a ON a.fid=i.fid AND a.node=i.pre AND a.axis=1 INTERSECT SELECT fid,pre FROM {s}"),
            (Direction::Inside, false) => format!("SELECT s.fid,s.pre FROM {s} s {} WHERE (s.fid,p.pre) IN (SELECT fid,pre FROM {i})", parent_of("s")),
            (Direction::Inside, true) => format!("SELECT DISTINCT s.fid,s.pre FROM {s} s JOIN nera_tree a ON a.fid=s.fid AND a.node=s.pre AND a.axis=1 WHERE (s.fid,a.pre) IN (SELECT fid,pre FROM {i})"),
            // The next sibling starts where the node's subtree ends, inside the parent's.
            (Direction::Precedes, false) => format!("SELECT s.fid,s.pre FROM {s} s {} WHERE {n}<p.pre+p.size AND (s.fid,{n}) IN (SELECT fid,pre FROM {i})", parent_of("s"), n = next("s")),
            (Direction::Follows, false) => format!("SELECT i.fid,{n} FROM {i} i {} WHERE {n}<p.pre+p.size INTERSECT SELECT fid,pre FROM {s}", parent_of("i"), n = next("i")),
            // A later (earlier) sibling is in inner: the two share a parent.
            (Direction::Precedes | Direction::Follows, true) => {
                let order = if matches!(direction, Direction::Precedes) { ">" } else { "<" };
                format!(
                    "SELECT DISTINCT s.fid,s.pre FROM {s} s {} JOIN (SELECT i.fid,i.pre,q.pre AS parent FROM {i} i JOIN nera_tree q ON q.fid=i.fid AND q.node=i.pre AND q.axis=2) o ON o.fid=s.fid AND o.parent=p.pre AND o.pre{order}s.pre",
                    parent_of("s")
                )
            }
        };
        let exact = scope.exact && inner.exact;
        let selected = self.relation(body, exact)?;
        Ok(self.narrowed(selected, &scope))
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
        // (L \ R) within tapes is (L within them) \ R: a complement of every node is not built
        // whole only to keep a few tapes' (06 S7: `all_nodes EXCEPT …`, 370 K rows, ~220 ms).
        let body = match self.reuse.differences.get(&scope.name) {
            Some((left, right)) => format!(
                "SELECT s.fid,s.pre FROM {left} s WHERE {} EXCEPT SELECT fid,pre FROM {right}",
                tests.join(" AND ")
            ),
            None => format!("SELECT s.fid,s.pre FROM {} s WHERE {}", scope.name, tests.join(" AND ")),
        };
        let selected = self.relation(body, false)?;
        if scope.name == "all_nodes" {
            self.reuse.wholes.insert(selected.name.clone(), sets.iter().map(|set| set.name.clone()).collect());
        }
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
    batch_scope(plans, lang, files, statistics, false)
}

/// `batch` for the pure-SQL fallback (03 §17): relations read the tape and are exact.
pub(super) fn fallback(plans: &[&AnchorPlan], lang: u32, files: &[i64], statistics: &Statistics) -> Statement {
    batch_scope(plans, lang, files, statistics, true)
}

/// The vocabulary's stages after the regex predicates are registered (`vocabulary_predicate`,
/// ready 0): their literal sets, their terms, and the predicates marked ready.
pub(super) const VOCABULARY: [(&str, &str); 4] = [
    (
        "vocabulary_literals",
        "INSERT OR IGNORE INTO vocabulary_literal SELECT p.lang,p.expression,j.value FROM vocabulary_predicate p CROSS JOIN json_each(nera_literal_set(p.expression)) j WHERE p.ready=0",
    ),
    ("vocabulary_fill", include_str!("vocabulary-fill.sql")),
    ("vocabulary_complete", "UPDATE vocabulary_predicate SET ready=1 WHERE ready=0"),
    ("vocabulary_release", "DELETE FROM vocabulary_pending"),
];

fn scope(lang: u32, statistics: &Statistics, tree: bool) -> Lower {
    let mut lower = Lower::new(lang);
    lower.tree = tree;
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
pub(super) fn anchored(plan: &AnchorPlan) -> bool {
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
fn batch_scope(plans: &[&AnchorPlan], lang: u32, fids: &[i64], statistics: &Statistics, tree: bool) -> Statement {
    let mut exact = Vec::with_capacity(plans.len());
    use super::graph::program::Program;
    let mut program = Program::default();
    let mut roots: Vec<Root> = Vec::new();
    let mut indexed_nodes = BTreeSet::new();
    let mut predicates = std::collections::BTreeMap::new();
    let mut admissions = Vec::new();
    for (qid, plan) in plans.iter().enumerate() {
        // Admission belongs to the individual plan. Another consumer's cached
        // producers cannot expand or shrink this plan's existing 256-node allowance.
        let mut lower = scope(lang, statistics, tree);
        let checkpoint = lower.ctes.len();
        lower.limit = checkpoint + 256;
        let attempted = lower.plan(plan, lower.all());
        admissions.push(Admission {
            consumer: qid,
            generated: lower.ctes.len() - checkpoint,
            broadened: attempted.is_err(),
        });
        exact.push(attempted.as_ref().is_ok_and(|selected| selected.exact));
        let selected = match attempted {
            Ok(selected) => selected,
            Err(()) => {
                // Discard the entire failed fragment, including predicates, parameters
                // and reuse state. Earlier fragments already belong to the shared program.
                lower = scope(lang, statistics, tree);
                lower.limit = lower.ctes.len() + 256;
                match &plan.root_kinds {
                    Some(kinds) => lower.kinds(kinds).unwrap_or_else(|_| lower.all()),
                    None => lower.all(),
                }
            }
        };
        // A union with sets of every node in some tapes emits its branches, those sets as a row
        // a tape (`execute` expands them): not every node of those tapes built, sorted against
        // the other branches and returned (06 S7: 440 K of a plan's rows).
        let mut branches: Vec<(String, Option<Vec<String>>)> = Vec::new();
        let mut pending = vec![selected.name.clone()];
        while let Some(name) = pending.pop() {
            if let Some(names) = lower.reuse.unions.get(&name) {
                pending.extend(names.iter().cloned());
            } else if !branches.iter().any(|(branch, _)| *branch == name) {
                branches.push((name.clone(), lower.reuse.wholes.get(&name).cloned()));
            }
        }
        // The other branches as one union (its rows distinct, as the plan's were).
        let rest = branches.iter().filter(|(_, whole)| whole.is_none()).map(|(name, _)| Set { name: name.clone(), exact: false }).collect::<Vec<_>>();
        let rest = match rest.len() {
            0 => Ok(None),
            _ => lower.union(rest).map(Some),
        };
        match rest {
            Ok(rest) if branches.iter().any(|(_, whole)| whole.is_some()) => {
                branches.retain(|(_, whole)| whole.is_some());
                branches.extend(rest.map(|rest| (rest.name, None)));
            }
            _ => branches = vec![(selected.name.clone(), None)],
        }
        let read = branches
            .iter()
            .flat_map(|(name, whole)| match whole {
                Some(sets) => sets.iter().cloned().chain(["cf".to_owned()]).collect::<Vec<_>>(),
                None => vec![name.clone()],
            })
            .map(|name| format!("SELECT fid FROM {name}"))
            .collect::<Vec<_>>()
            .join(" UNION ALL ");
        let definitions = Definitions::new(std::mem::take(&mut lower.ctes));
        let live = definitions.live(&read).into_iter().cloned().collect::<Vec<_>>();
        let nodes = program.add(&live, &lower.parameters, lang);
        for (name, whole) in branches {
            roots.push(match whole {
                Some(sets) => Root { node: nodes["cf"].id, qid, whole: Some(sets.iter().map(|set| nodes[set].id).collect()) },
                None => Root { node: nodes[&name].id, qid, whole: None },
            });
        }
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
    // A split plan's arms stay together, its whole tapes (rows with a negative `pre`, minus the
    // tape's nodes) first: `execute` expands them and drops the other arm's rows in those tapes.
    let mut first = HashMap::new();
    for root in &roots {
        let rank = layout.ranks[&root.node];
        first.entry(root.qid).and_modify(|at: &mut _| *at = std::cmp::min(*at, rank)).or_insert(rank);
    }
    roots.sort_unstable_by_key(|root| (first[&root.qid], root.qid, root.whole.is_none(), layout.ranks[&root.node]));
    let mut arms = roots
        .iter()
        .map(|root| match &root.whole {
            None => format!("SELECT {} AS qid,fid,pre FROM {}", root.qid, layout.names[&root.node]),
            Some(sets) => {
                let tests = std::iter::once(root.node)
                    .chain(sets.iter().copied())
                    .map(|node| format!(" AND s.tape_id IN (SELECT fid FROM {})", layout.names[&node]))
                    .collect::<String>();
                format!("SELECT {} AS qid,s.tape_id AS fid,-s.nodes AS pre FROM prefilter_sets s WHERE s.nodes>0{tests}", root.qid)
            }
        })
        .collect::<Vec<_>>();
    if arms.is_empty() {
        arms.push("SELECT 0 AS qid,0 AS fid,0 AS pre WHERE 0".into());
    }
    // No ORDER BY: SQLite sorted every arm in a temp B-tree to merge them (a constant `qid` first
    // hides the index's (fid, pre) order), ~90 sorts on S7. `select` sorts each plan's rows.
    let queries = arms.chunks(128).map(|arms| arms.join(" UNION ALL ")).collect();
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
    statement.exact = exact;
    if !values.is_empty() {
        let stages = [
            (
                "vocabulary_register",
                format!(
                    "INSERT OR IGNORE INTO vocabulary_predicate(lang,expression,ready) VALUES {}",
                    values.into_iter().collect::<Vec<_>>().join(",")
                ),
            ),
        ]
        .into_iter()
        .chain(VOCABULARY.iter().map(|&(name, sql)| (name, sql.to_owned())))
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
        exact: Vec::new(),
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
