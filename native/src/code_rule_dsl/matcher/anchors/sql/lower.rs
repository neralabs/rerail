//! Candidate plans lower to SQL sets of (file, preorder). The matcher retains capture semantics.
use super::super::{AnchorPlan, Condition, Direction, Required};
use rusqlite::types::Value;
use std::collections::{BTreeSet, HashMap};
#[path = "access.rs"]
mod access;
#[path = "decode.rs"]
mod decode;
use decode::Property;
#[path = "lower/render.rs"]
mod render;
use render::Definitions;
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
    pub partial: bool,
}
pub(super) struct Statement {
    pub admissions: Vec<Admission>,
    pub anchors: Vec<super::statistics::Decision>,
    #[cfg(feature = "code-rule-profile")]
    pub producers: crate::code_rule_dsl::producers::Registry<u32>,
    #[cfg(feature = "code-rule-profile")]
    pub consumers: Vec<usize>,
    pub graph: Vec<String>,
    pub language: u32,
    pub files: Vec<i64>,
    pub file_scope: bool,
    pub universal: bool,
    pub queries: Vec<String>,
    pub stages: Vec<Stage>,
    pub schedule: lifetime::Schedule,
    pub parameters: Vec<Value>,
    pub ownership: ownership::Layout,
}
impl Statement {
    pub fn preparations(&self, covered: bool) -> usize {
        self.schedule.actions.iter().filter(|a|matches!(a,lifetime::Action::Prepare(i) if !(covered&&self.stages[*i].covering))).count()
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
    lang: u32,
    roots: HashMap<BTreeSet<u16>, Set>,
    limit: usize,
    indexed: BTreeSet<String>,
    reuse: Reuse,
    decoding: HashMap<String, decode::Demand>,
    predicates: HashMap<String, (String, String)>,
    producers: crate::code_rule_dsl::producers::Registry<u32>,
    consumer: usize,
    statistics: super::statistics::Statistics,
    partial: bool,
    costs: HashMap<usize, usize>,
    anchors: Vec<super::statistics::Decision>,
    producer_bindings: HashMap<Property, usize>,
}

/// SQL set identities, independent of the matcher's conservative exactness metadata.
#[derive(Default)]
struct Reuse {
    bodies: HashMap<String, String>,
    factors: HashMap<String, BTreeSet<String>>,
    intersections: std::collections::BTreeMap<BTreeSet<String>, String>,
    strings: HashMap<String, usize>,
    integers: HashMap<i64, usize>,
    values: HashMap<(String, bool, Option<BTreeSet<u16>>), Set>,
    terms: HashMap<(String, bool), String>,
    decoded: HashMap<(String, usize), String>,
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
            decoding: HashMap::new(),
            predicates: HashMap::new(),
            producers: Default::default(),
            consumer: 0,
            statistics: Default::default(),
            partial: false,
            costs: HashMap::new(),
            anchors: Vec::new(),
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
    fn union(&mut self, sets: &[Set]) -> Result<Set, ()> {
        if sets.is_empty() {
            return self.relation("SELECT 0,0 WHERE 0".into(), true);
        }
        let exact = sets.iter().all(|set| set.exact);
        let kinds = sets.iter().try_fold(BTreeSet::new(), |mut kinds, set| {
            kinds.extend(self.reuse.kinds.get(&set.name)?.iter().copied());
            Some(kinds)
        });
        let names: BTreeSet<_> = sets.iter().map(|set| &set.name).collect();
        let arms = names
            .into_iter()
            .map(|name| format!("SELECT fid,pre FROM {name}"))
            .collect();
        // One deduplication boundary for the complete alternative family. A
        // left-deep chain copies each growing prefix into another ephemeral set.
        let selected = self.relation(
            format!(
                "SELECT DISTINCT fid,pre FROM ({})",
                waves::compound("fid,pre", arms)
            ),
            exact,
        )?;
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
            self.reuse
                .factors
                .insert(selected.name.clone(), factors.clone());
            self.reuse
                .intersections
                .insert(factors, selected.name.clone());
        }
        Ok(selected)
    }
    fn kinds(&mut self, kinds: &BTreeSet<u16>) -> Result<Set, ()> {
        if let Some(set) = self.roots.get(kinds) {
            return Ok(set.clone());
        }
        if kinds.is_empty() {
            return self.relation("SELECT 0,0 WHERE 0".into(), true);
        }
        let codes = kinds
            .iter()
            .map(|&kind| self.parameter(i64::from(if kind == u16::MAX { 0 } else { kind })))
            .collect::<Vec<_>>();
        // Seek the file/kind count row before decoding its compact positions. fts5vocab
        // cannot push file constraints into its corpus-wide instance stream.
        let body = format!(
            "SELECT k.fid,p.pre FROM cf CROSS JOIN file_kind_counts k ON k.fid=cf.fid CROSS JOIN nera_postings(k.postings) p WHERE k.code IN ({})",
            codes.join(",")
        );
        let mut selected = self.relation(body, true)?;
        if kinds.contains(&0) != kinds.contains(&u16::MAX) {
            let op = if kinds.contains(&u16::MAX) { "<>" } else { "=" };
            let properties = self.decoded(&selected, Property::Node);
            selected=self.relation(format!("SELECT p.fid,p.pre FROM {properties} p WHERE p.kind NOT IN (0,65535) OR (p.flags&8) {op} 0"),true)?;
        }
        self.roots.insert(kinds.clone(), selected.clone());
        // Base postings otherwise get re-scanned inside every physical consumer statement.
        self.indexed.insert(selected.name.clone());
        self.reuse
            .kinds
            .insert(selected.name.clone(), kinds.clone());
        Ok(selected)
    }

    /// Initial discovery uses postings. Refinement reads only the supplied keys;
    /// even an empty scope must not construct the complete alternative kind set.
    fn restrict_kinds(&mut self, scope: Set, kinds: &BTreeSet<u16>) -> Result<Set, ()> {
        if scope.name == "all_nodes" {
            return self.kinds(kinds);
        }
        if let Some(known) = self.reuse.kinds.get(&scope.name) {
            if known.is_subset(kinds) {
                return Ok(scope);
            }
            if known.is_disjoint(kinds) {
                return self.relation("SELECT 0,0 WHERE 0".into(), scope.exact);
            }
        }
        if kinds.is_empty() {
            return self.relation("SELECT 0,0 WHERE 0".into(), scope.exact);
        }
        let properties = self.decoded(&scope, Property::Node);
        let codes = kinds
            .iter()
            .map(|&kind| self.parameter(i64::from(kind)))
            .collect::<Vec<_>>()
            .join(",");
        let selected = self.relation(
            format!("SELECT p.fid,p.pre FROM {properties} p WHERE p.kind IN ({codes})"),
            scope.exact,
        )?;
        self.reuse
            .kinds
            .insert(selected.name.clone(), kinds.clone());
        Ok(selected)
    }
    // Bound admission ordering by the existing SQL-fragment allowance. This is a
    // compiler complexity estimate, not a fitted execution or regex cost model.
    fn compile_cost(&mut self, plan: &AnchorPlan) -> usize {
        let key = plan as *const AnchorPlan as usize;
        if let Some(cost) = self.costs.get(&key) {
            return *cost;
        }
        let mut work = vec![plan];
        let mut cost = 0;
        while let Some(plan) = work.pop() {
            cost += 1;
            if cost >= self.limit {
                break;
            }
            match &plan.condition {
                Condition::All(plans) | Condition::Any(plans) => work.extend(plans),
                Condition::Not(inner) | Condition::Relation { inner, .. } => work.push(inner),
                Condition::Utility(inner) => work.push(inner),
                _ => {}
            }
        }
        self.costs.insert(key, cost);
        cost
    }

    fn plan(&mut self, plan: &AnchorPlan, scope: Set) -> Result<Set, ()> {
        if let Condition::Pattern(required) = &plan.condition
            && !required.is_empty()
        {
            let selected = self.pattern(required, scope)?;
            return match &plan.root_kinds {
                Some(kinds) => self.restrict_kinds(selected, kinds),
                None => Ok(selected),
            };
        }
        let scope = match &plan.root_kinds {
            Some(kinds) => self.restrict_kinds(scope, kinds)?,
            None => scope,
        };
        let kinds = self.reuse.kinds.get(&scope.name).cloned();
        let mut selected = match &plan.condition {
            Condition::Always => Ok(scope),
            Condition::Text { regex, .. } => self.text(&scope, regex.as_str(), false),
            Condition::Pattern(required) => self.pattern(required, scope),
            Condition::All(plans) => {
                let mut selected = scope;
                let mut ordered = plans.iter().collect::<Vec<_>>();
                ordered.sort_by_cached_key(|plan| self.compile_cost(plan));
                for plan in ordered {
                    match self.plan(plan, selected.clone()) {
                        Ok(next) => selected = next,
                        Err(()) => {
                            // A partial conjunction is a sound superset. Stop before
                            // another fragment can use the failed child's caches; dead
                            // definitions are removed when the completed root is rendered.
                            self.partial = true;
                            selected.exact = false;
                            return Ok(selected);
                        }
                    }
                }
                Ok(selected)
            }
            Condition::Any(plans) => {
                // Optional captures commonly have an unrestricted candidate arm.
                // The authoritative matcher retains capture order; preparing the
                // other candidate arms cannot narrow this union.
                let unrestricted = plans.iter().find(|plan| {
                    matches!(plan.condition, Condition::Always)
                        && plan.root_kinds.as_ref().is_none_or(|required| {
                            self.reuse
                                .kinds
                                .get(&scope.name)
                                .is_some_and(|known| known.is_subset(required))
                        })
                });
                if let Some(plan) = unrestricted {
                    Ok(Set {
                        exact: scope.exact && plan.exact,
                        ..scope
                    })
                } else {
                    let mut branches = Vec::new();
                    let mut covering = None;
                    for plan in plans {
                        let branch = self.plan(plan, scope.clone())?;
                        if branch.name == scope.name {
                            covering = Some(branch);
                            break;
                        }
                        branches.push(branch);
                    }
                    covering.map_or_else(|| self.union(&branches), Ok)
                }
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
                        exact: selected.exact && plan.exact && !self.partial,
                        ..selected.clone()
                    });
                }
                let selected = self.plan(inner, scope)?;
                self.utilities.insert(key, selected.clone());
                Ok(selected)
            }
            Condition::Relation {
                inner,
                direction,
                recursive,
                field,
            } => self.related(scope, inner, *direction, *recursive, *field),
        }?;
        selected.exact &= plan.exact && !self.partial;
        if let Some(kinds) = kinds {
            self.reuse.kinds.insert(selected.name.clone(), kinds);
        }
        Ok(selected)
    }
    fn related(
        &mut self,
        scope: Set,
        inner: &AnchorPlan,
        direction: Direction,
        recursive: bool,
        field: Option<u16>,
    ) -> Result<Set, ()> {
        if matches!(direction, Direction::Precedes | Direction::Follows) {
            // Overlapping and zero-width siblings use the authoritative matcher's walk. This
            // remains a SQL candidate superset and cannot be subtracted under negation.
            return Ok(Set {
                exact: false,
                ..scope
            });
        }
        if recursive {
            return self.subtree_relation(scope, inner, direction, field);
        }
        // Evaluate inner predicates in their structural context, before reading node text.
        let inner_scope = match (direction, recursive, field) {
            (Direction::Has, false, Some(field)) => {
                let edges = self.decoded(&scope, Property::Child(field));
                let children = self.relation(
                    format!("SELECT s.fid,s.child AS pre FROM {edges} s"),
                    scope.exact,
                )?;
                self.relation(
                    format!(
                        "SELECT fid,pre FROM {} WHERE pre IS NOT NULL",
                        children.name
                    ),
                    scope.exact,
                )?
            }
            (Direction::Inside, false, field) => {
                let properties = self.decoded(&scope, Property::Node);
                let guard = field.map_or(String::new(), |field| {
                    format!(" WHERE s.field={field} AND (s.flags&16)<>0")
                });
                let parents = self.relation(
                    format!("SELECT DISTINCT s.fid,s.parent AS pre FROM {properties} s{guard}"),
                    scope.exact,
                )?;
                self.relation(
                    format!("SELECT fid,pre FROM {} WHERE pre>=0", parents.name),
                    scope.exact,
                )?
            }
            _ => self.all(),
        };
        let inner = self.plan(inner, inner_scope)?;
        let guard = field.map_or(String::new(), |field| {
            format!(" AND c.field={field} AND (c.flags&16)<>0")
        });
        let selected = match direction {
            Direction::Has => {
                let properties = self.decoded(&inner, Property::Node);
                self.relation(format!("SELECT DISTINCT c.fid,c.parent FROM {properties} c WHERE c.parent>=0{guard}"), inner.exact)?
            }
            // The scope's structural demand populated these child rows before the
            // inner predicate. Seek the persistent (fid,parent,field,pre) index from
            // selected parents; joining two materialized projections can scan their
            // Cartesian product when SQLite underestimates either relation.
            Direction::Inside => self.relation(format!("SELECT m.fid,c.pre FROM {} m CROSS JOIN decoded_properties c ON c.fid=m.fid AND c.parent=m.pre WHERE m.pre>=0{guard}", inner.name), inner.exact && scope.exact)?,
            _ => unreachable!(),
        };
        self.combine(
            &scope,
            &selected,
            "INTERSECT",
            scope.exact && selected.exact,
        )
    }

    /// AST descendants are preorder intervals. General graph reachability still needs recursion;
    /// a tree relation does not need one parent read per candidate and ancestor.
    fn subtree_relation(
        &mut self,
        scope: Set,
        inner: &AnchorPlan,
        direction: Direction,
        field: Option<u16>,
    ) -> Result<Set, ()> {
        let inner = self.plan(inner, self.all())?;
        let roots = if matches!(direction, Direction::Has) {
            &scope
        } else {
            &inner
        };
        let ranges = self.intervals(roots, field)?;
        match direction {
            Direction::Has => {
                self.indexed.insert(inner.name.clone());
                self.relation(format!("SELECT r.fid,r.pre FROM {ranges} r WHERE r.lo<r.hi AND EXISTS(SELECT 1 FROM {} m WHERE m.fid=r.fid AND m.pre>=r.lo AND m.pre<r.hi)", inner.name), scope.exact && inner.exact)
            }
            Direction::Inside => {
                self.indexed.insert(scope.name.clone());
                // Subtree intervals are nested or disjoint. Keep the outer intervals once.
                let frontier = format!("frontier{}", self.ctes.len());
                self.ctes.push(format!("{frontier}(fid,lo,hi) AS MATERIALIZED (SELECT fid,lo,hi FROM (SELECT fid,lo,hi,max(hi) OVER(PARTITION BY fid ORDER BY lo,hi DESC ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING) previous FROM {ranges} WHERE lo<hi) WHERE previous IS NULL OR hi>previous)"));
                self.relation(format!("SELECT DISTINCT s.fid,s.pre FROM {frontier} r CROSS JOIN {} s WHERE s.fid=r.fid AND s.pre>=r.lo AND s.pre<r.hi", scope.name), scope.exact && inner.exact)
            }
            _ => unreachable!("parent relation"),
        }
    }

    fn intervals(&mut self, roots: &Set, field: Option<u16>) -> Result<String, ()> {
        let source = if let Some(field) = field {
            let edges = self.decoded(roots, Property::Child(field));
            let children = format!("children{}", self.ctes.len());
            self.ctes.push(format!(
                "{children}(fid,pre,lo) AS MATERIALIZED (SELECT s.fid,s.pre,s.child FROM {edges} s)"
            ));
            let valid = format!("valid{}", self.ctes.len());
            self.ctes.push(format!("{valid}(fid,pre,lo) AS MATERIALIZED (SELECT fid,pre,lo FROM {children} WHERE lo IS NOT NULL)"));
            let child_set = self.relation(format!("SELECT DISTINCT fid,lo FROM {valid}"), true)?;
            let properties = self.decoded(&child_set, Property::Node);
            format!(
                "SELECT v.fid,v.pre,v.lo,p.end FROM {valid} v JOIN {properties} p ON p.fid=v.fid AND p.pre=v.lo"
            )
        } else {
            let properties = self.decoded(roots, Property::Node);
            format!("SELECT s.fid,s.pre,s.pre+1,s.end FROM {properties} s")
        };
        let name = format!("ranges{}", self.ctes.len());
        self.ctes
            .push(format!("{name}(fid,pre,lo,hi) AS MATERIALIZED ({source})"));
        Ok(name)
    }
    fn pattern(&mut self, required: &[super::super::Requirement], scope: Set) -> Result<Set, ()> {
        if required.is_empty() {
            return Ok(scope);
        }
        let (ordinal, estimate) = self.statistics.choose(self.lang, required);
        let requirement = &required[ordinal];
        let (kind, literal) = match &requirement.value {
            Required::Kind(kind) => (*kind, None),
            Required::Terminal(kind, text) => (*kind, Some(text.as_str())),
        };
        self.anchors.push(super::statistics::Decision {
            consumer: self.consumer,
            pattern: self.anchors.len(),
            requirement: ordinal,
            kind,
            literal: literal.map(str::to_owned),
            estimated_rows: estimate.map(|e| e.rows),
            uncovered_rows: estimate.map(|e| e.uncovered),
        });
        // A literal seeds directly from value postings. Building its kind first would
        // still enumerate the common arm that the planner just declined.
        let mut current = if let Some(text) = literal {
            let domain = self.all();
            let previous = self
                .reuse
                .kinds
                .insert(domain.name.clone(), BTreeSet::from([kind]));
            let selected = self.text(&domain, text, true);
            if let Some(previous) = previous {
                self.reuse.kinds.insert(domain.name, previous);
            } else {
                self.reuse.kinds.remove(&domain.name);
            }
            selected?
        } else {
            self.kinds(&BTreeSet::from([kind]))?
        };
        for parent_kind in &requirement.parents {
            let properties = self.decoded(&current, Property::Node);
            let hopped = self.relation(
                format!("SELECT DISTINCT p.fid,p.parent FROM {properties} p"),
                false,
            )?;
            current = self.relation(
                format!("SELECT fid,pre FROM {} WHERE pre>=0", hopped.name),
                false,
            )?;
            if let Some(kind) = parent_kind {
                let properties = self.decoded(&current, Property::Node);
                current = self.relation(
                    format!("SELECT p.fid,p.pre FROM {properties} p WHERE p.kind={kind}"),
                    false,
                )?;
            }
        }
        // All other requirements are checked by the existing matcher at these roots.
        // Whole-scope postings just for a refinement recreate the discarded anchor work.
        self.combine(&scope, &current, "INTERSECT", false)
    }
}

pub(super) fn batch_files(plans: &[&AnchorPlan], lang: u32, files: &[i64]) -> Statement {
    batch_files_planned(plans, lang, files, &Default::default())
}

pub(super) fn batch_files_planned(
    plans: &[&AnchorPlan],
    lang: u32,
    files: &[i64],
    statistics: &super::statistics::Statistics,
) -> Statement {
    batch_scope(plans, lang, Some(files), None, statistics)
}

#[cfg(test)]
pub(super) fn batch(
    store: &super::store::Store,
    plans: &[&AnchorPlan],
    lang: u32,
    fid: Option<i64>,
    path_glob: Option<&str>,
) -> Statement {
    let statistics =
        super::statistics::Statistics::read(&store.db, plans.iter().map(|p| (lang, *p))).unwrap();
    batch_scope(
        plans,
        lang,
        fid.as_ref().map(std::slice::from_ref),
        path_glob,
        &statistics,
    )
}

fn scope(
    lang: u32,
    fids: Option<&[i64]>,
    path_glob: Option<&str>,
    statistics: &super::statistics::Statistics,
) -> Lower {
    let mut lower = Lower::new(lang);
    lower.statistics = statistics.clone();
    let lang_parameter = lower.parameter(i64::from(lang));
    let mut files =
        format!("SELECT f.fid FROM file f JOIN fk k ON k.fid=f.fid WHERE k.lang={lang_parameter}");
    if fids.is_some() {
        files.push_str(" AND f.fid IN (SELECT fid FROM request_file)");
    }
    if let Some(path) = path_glob {
        files.push_str(&format!(
            " AND f.path GLOB {}",
            lower.parameter(path.to_owned())
        ));
    }
    lower
        .ctes
        .push(format!("cf(fid) AS MATERIALIZED ({files})"));
    lower.ctes.push("all_nodes(fid,pre) AS NOT MATERIALIZED (SELECT f.fid,p.value FROM cf JOIN file f ON f.fid=cf.fid CROSS JOIN generate_series(0,f.nodes-1,1) p)".into());
    lower
}

fn batch_scope(
    plans: &[&AnchorPlan],
    lang: u32,
    fids: Option<&[i64]>,
    path_glob: Option<&str>,
    statistics: &super::statistics::Statistics,
) -> Statement {
    use super::graph::program::Program;
    let mut program = Program::default();
    let mut roots = Vec::new();
    let mut indexed_nodes = BTreeSet::new();
    let mut decoded_nodes: std::collections::BTreeMap<
        usize,
        (Option<usize>, Property, BTreeSet<usize>),
    > = std::collections::BTreeMap::new();
    let mut predicates = std::collections::BTreeMap::new();
    let mut admissions = Vec::new();
    let mut anchors = Vec::new();
    for (qid, plan) in plans.iter().enumerate() {
        // Admission belongs to the individual plan. Another consumer's cached
        // producers cannot expand or shrink this plan's existing 256-node allowance.
        let mut lower = scope(lang, fids, path_glob, statistics);
        lower.consumer = qid;
        let checkpoint = lower.ctes.len();
        lower.limit = checkpoint + 256;
        let attempted = lower.plan(plan, lower.all());
        anchors.extend(std::mem::take(&mut lower.anchors));
        admissions.push(Admission {
            consumer: qid,
            generated: lower.ctes.len() - checkpoint,
            broadened: attempted.is_err() || lower.partial,
            partial: lower.partial && attempted.is_ok(),
        });
        #[cfg(feature = "code-rule-profile")]
        if attempted.is_err() || lower.partial {
            crate::code_rule_dsl::profile::record_anchor_plan_broadening();
        }
        let selected = match attempted {
            Ok(selected) => selected,
            Err(()) => {
                // Discard the entire failed fragment, including predicates, parameters
                // and reuse state. Earlier fragments already belong to the shared program.
                lower = scope(lang, fids, path_glob, statistics);
                lower.consumer = qid;
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
        for (name, demand) in lower.decoding {
            let Some(node) = nodes.get(&name) else {
                continue;
            };
            let scope = if demand.scope == "all_nodes" {
                None
            } else {
                Some(nodes[&demand.scope].id)
            };
            decoded_nodes
                .entry(node.id)
                .and_modify(|(previous, property, consumers)| {
                    assert!(
                        *previous == scope && *property == demand.property,
                        "shared decoder definition"
                    );
                    consumers.extend(&demand.consumers);
                })
                .or_insert((scope, demand.property, demand.consumers));
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
    let decoding = decoded_nodes
        .into_iter()
        .map(|(node, (scope, property, consumers))| {
            (
                layout.names[&node].clone(),
                decode::Demand {
                    scope: scope
                        .map_or_else(|| "all_nodes".into(), |scope| layout.names[&scope].clone()),
                    property,
                    consumers,
                },
            )
        })
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
    let mut statement = prepare_decoded(layout.ctes, queries, layout.parameters, indexed, decoding);
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
            covering: false,
        })
        .collect::<Vec<_>>();
        statement.stages.splice(0..0, stages);
    }
    #[cfg(feature = "code-rule-profile")]
    {
        statement.consumers = plans
            .iter()
            .map(|p| *p as *const AnchorPlan as usize)
            .collect();
    }
    statement.anchors = anchors;
    statement.language = lang;
    statement.files = fids.unwrap_or_default().to_vec();
    statement.file_scope = fids.is_some();
    statement.own();
    statement
}

#[derive(Clone)]
pub(super) struct Stage {
    pub name: String,
    pub sql: String,
    pub index: Option<&'static str>,
    pub temporary: bool,
    pub covering: bool,
}

/// SQLite expands CTE references during preparation, including AS MATERIALIZED CTEs.
/// Cut shared dependencies into real SQL tables before that expansion becomes exponential.
#[cfg(test)]
pub(super) fn finish(
    ctes: Vec<String>,
    query: String,
    parameters: Vec<Value>,
    indexed: BTreeSet<String>,
) -> Statement {
    finish_decoded(ctes, vec![query], parameters, indexed, HashMap::new())
}

#[cfg(test)]
fn finish_decoded(
    ctes: Vec<String>,
    queries: Vec<String>,
    parameters: Vec<Value>,
    indexed: BTreeSet<String>,
    decoding: HashMap<String, decode::Demand>,
) -> Statement {
    let mut statement = prepare_decoded(ctes, queries, parameters, indexed, decoding);
    statement.reschedule();
    statement
}

fn prepare_decoded(
    ctes: Vec<String>,
    queries: Vec<String>,
    parameters: Vec<Value>,
    mut indexed: BTreeSet<String>,
    decoding: HashMap<String, decode::Demand>,
) -> Statement {
    let query = queries.join("\n");
    // Candidate broadening and approximate negation can discard a whole dependency tree.
    // Only materialize tables reachable from the final batch outputs.
    let mut ctes: Vec<_> = Definitions::new(ctes)
        .live(&query)
        .into_iter()
        .cloned()
        .collect();
    // A live universal demand already requires every structural property in this file batch.
    // Hoist that one fill before consumers, instead of checking and filling each subset again.
    let universal = ctes
        .iter()
        .filter_map(|cte| decoding.get(cte.split_once('(').expect("CTE columns").0))
        .find(|d| d.property == Property::Node && d.scope == "all_nodes");
    let required_indexes = indexed.clone();
    for demand in ctes
        .iter()
        .filter_map(|cte| decoding.get(cte.split_once('(').expect("CTE columns").0))
    {
        if !(universal.is_some() && matches!(demand.property, Property::Node | Property::Child(_)))
        {
            indexed.insert(demand.scope.clone());
        }
    }
    if universal.is_some() {
        for cte in &mut ctes {
            let name = cte.split_once('(').expect("CTE name").0;
            if let Some(demand) = decoding.get(name)
                && let Property::Child(field) = demand.property
            {
                *cte = format!(
                    "{name}(fid,pre,field,child) AS NOT MATERIALIZED (SELECT s.fid,s.pre,{field},(SELECT c.pre FROM decoded_properties c WHERE c.fid=s.fid AND c.parent=s.pre AND c.field={field} AND c.pre>s.pre ORDER BY c.pre LIMIT 1) FROM {} s)",
                    demand.scope
                );
            }
        }
        indexed.remove("all_nodes");
        if let Some(all) = ctes.iter_mut().find(|cte| cte.starts_with("all_nodes(")) {
            *all="all_nodes(fid,pre) AS NOT MATERIALIZED (SELECT d.fid,d.pre FROM cf CROSS JOIN decoded_properties d ON d.fid=cf.fid)".into();
        }
    }
    #[cfg(feature = "code-rule-profile")]
    let mut producers = crate::code_rule_dsl::producers::Registry::default();
    #[cfg(feature = "code-rule-profile")]
    for demand in ctes
        .iter()
        .filter_map(|cte| decoding.get(cte.split_once('(').expect("CTE columns").0))
    {
        for &consumer in &demand.consumers {
            demand.property.binding(
                &mut producers,
                0,
                format!("candidate-result.{consumer} selector {}", demand.scope),
            );
        }
    }
    let barrier_names = decoding
        .iter()
        .filter(|(_, demand)| {
            demand.property == Property::Span
                || (universal.is_none() && matches!(demand.property, Property::Child(_)))
        })
        .map(|(name, _)| name.clone())
        .collect();
    let (levels, uses) = waves::analyze(&ctes, &barrier_names, &query);
    // Repeated output arms share one WITH scope. They are one SQL reader,
    // rather than a reason to cut a cheap selector into another physical table.
    let mut duplicate_output_uses = HashMap::<String, usize>::new();
    for packet in &queries {
        let mut seen = BTreeSet::new();
        for name in waves::identifiers(packet) {
            if !seen.insert(name) {
                *duplicate_output_uses.entry(name.to_owned()).or_default() += 1;
            }
        }
    }
    let mut span_scopes: std::collections::BTreeMap<usize, BTreeSet<String>> =
        std::collections::BTreeMap::new();
    let mut child_scopes: std::collections::BTreeMap<usize, BTreeSet<(u16, String)>> =
        std::collections::BTreeMap::new();
    let mut stage_levels = HashMap::new();
    let graph = ctes.clone();
    let names: Vec<_> = ctes
        .iter()
        .map(|cte| cte.split_once('(').expect("CTE columns").0.to_owned())
        .collect();
    let mut inline = Definitions::default();
    let mut stages = Vec::new();
    if let Some(demand) = universal {
        let mut scope = Definitions::new(ctes.iter().filter(|cte| cte.starts_with("cf(")).cloned());
        stages.push(Stage {
            name: "decode_all_nodes".into(),
            sql: demand.node_population(&mut scope),
            index: None,
            temporary: false,
            covering: true,
        });
        stages.push(Stage {
            name: "mark_decoded_files".into(),
            sql: scope.render("INSERT OR IGNORE INTO decoded_files SELECT fid FROM cf"),
            index: None,
            temporary: false,
            covering: true,
        });
    }
    let mut expansion: HashMap<&str, usize> = HashMap::new();
    for (name, cte) in names.iter().zip(ctes) {
        if let Some(demand) = decoding.get(name)
            && !(universal.is_some()
                && matches!(demand.property, Property::Node | Property::Child(_)))
        {
            if demand.property == Property::Span {
                // All span demands in this language-qualified statement bind the same
                // source-spans contract and tape slot. Selectors contribute keys only.
                span_scopes
                    .entry(levels[name])
                    .or_default()
                    .insert(demand.scope.clone());
            } else if let Property::Child(field) = demand.property {
                child_scopes
                    .entry(levels[name])
                    .or_default()
                    .insert((field, demand.scope.clone()));
            } else {
                let sql = demand.node_population(&mut inline);
                let stage_name = format!("decode_{name}");
                stage_levels.insert(stage_name.clone(), (levels[name], 1));
                stages.push(Stage {
                    name: stage_name,
                    sql,
                    index: None,
                    temporary: false,
                    covering: false,
                });
            }
        }
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
        // A cheap selector with one logical reader stays in that SQL pipeline.
        // Decoder population can read it again at its real barrier; retain cuts
        // for shared or expensive selectors so preparation remains bounded.
        let readers = uses[name].saturating_sub(*duplicate_output_uses.get(name).unwrap_or(&0));
        let shared = required_indexes.contains(name)
            || (indexed.contains(name) && (readers > 1 || cost >= 32))
            || (uses[name] > 1 && cost >= 32);
        expansion.insert(name.as_str(), if shared { 1 } else { cost });
        let cte = if shared {
            render::owned(cte, name)
        } else {
            cte
        };
        inline.push(cte);
        if shared {
            // CTAS otherwise inherits empty affinity from FTS columns and compound CTEs.
            // INTEGER bounds from decoded properties then coerce those indexed keys, and
            // SQLite drops their range constraints. Every candidate ordinal is an integer.
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
            stage_levels.insert(name.clone(), (levels[name], 1));
            stages.push(Stage {
                name: name.clone(),
                sql,
                index,
                temporary: true,
                covering: false,
            });
            inline.pop();
        }
    }
    for (level, scopes) in span_scopes {
        let name = format!("decode_source_spans_wave_{level}");
        stage_levels.insert(name.clone(), (level, 0));
        stages.push(Stage {
            name,
            sql: decode::Demand::span_population(&scopes, &mut inline),
            index: None,
            temporary: false,
            covering: false,
        });
    }
    for (level, scopes) in child_scopes {
        let name = format!("decode_child_fields_wave_{level}");
        stage_levels.insert(name.clone(), (level, 0));
        stages.push(Stage {
            name,
            sql: decode::Demand::child_population(&scopes, &mut inline),
            index: None,
            temporary: false,
            covering: false,
        });
    }
    // Stable order preserves every unbatched dependency within a wave. A decoder fill
    // follows all its lower-wave selectors and precedes readers in its own wave.
    stages.sort_by_cached_key(|stage| stage_levels.get(&stage.name).copied().unwrap_or((0, 1)));
    let queries: Vec<_> = queries.iter().map(|query| inline.render(query)).collect();
    let schedule = lifetime::Schedule {
        actions: Vec::new(),
        peak_relations: 0,
    };
    Statement {
        admissions: Vec::new(),
        anchors: Vec::new(),
        #[cfg(feature = "code-rule-profile")]
        producers,
        #[cfg(feature = "code-rule-profile")]
        consumers: Vec::new(),
        graph,
        language: 0,
        files: Vec::new(),
        file_scope: false,
        universal: universal.is_some(),
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
