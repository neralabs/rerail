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
}
pub(super) struct Statement {
    pub admissions: Vec<Admission>,
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
                for plan in plans.iter().filter(|p| anchored(p)).chain(plans.iter().filter(|p| !anchored(p))) {
                    selected = self.plan(plan, selected)?;
                }
                Ok(selected)
            }
            Condition::Any(plans) => {
                let mut selected = self.relation("SELECT 0,0 WHERE 0".into(), true)?;
                for plan in plans {
                    let branch = self.plan(plan, scope.clone())?;
                    selected =
                        self.combine(&selected, &branch, "UNION", selected.exact && branch.exact)?;
                }
                Ok(selected)
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
            Condition::Relation {
                inner,
                direction,
                recursive,
                field,
            } => self.related(scope, inner, *direction, *recursive, *field),
        }?;
        selected.exact &= plan.exact;
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
        if matches!(direction, Direction::Inside) && scope.name == "all_nodes" && anchored(inner) {
            // Over every node, the parents of every node cost a decode each; the children of the
            // inner postings are read from the tape.
            let inner = self.plan(inner, scope)?;
            let children = self.relation(
                format!("SELECT DISTINCT s.fid,t.pre FROM {} s CROSS JOIN nera_tree(s.fid,s.pre,{}) t", inner.name, super::tree::CHILDREN),
                inner.exact,
            )?;
            let Some(field) = field else {
                return Ok(children);
            };
            let properties = self.decoded(&children, Property::Node);
            return self.relation(
                format!("SELECT s.fid,s.pre FROM {properties} s WHERE s.field={field} AND (s.flags&16)<>0"),
                inner.exact,
            );
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
            (Direction::Has, false, None) => self.neighbours(&scope, inner, super::tree::CHILDREN)?,
            _ => self.all(),
        };
        let inner = self.plan(inner, inner_scope)?;
        let seed = if matches!(direction, Direction::Has) {
            &inner
        } else {
            &scope
        };
        let properties = self.decoded(seed, Property::Node);
        let name = format!("w{}", self.ctes.len());
        self.ctes.push(format!("{name}(fid,origin,child,pre,field,flags) AS MATERIALIZED (SELECT s.fid,s.pre,s.pre,s.parent,s.field,s.flags FROM {properties} s)"));
        let field = field.map_or(String::new(), |field| {
            format!(" AND w.field={field} AND (w.flags&16)<>0")
        });
        let selected = match direction {
            Direction::Has => self.relation(format!("SELECT DISTINCT w.fid,w.pre FROM {name} w WHERE w.pre>=0{field}"), inner.exact)?,
            // Membership, not a join: neither side is indexed, and SQLite planned the join as
            // a scan of one per row of the other. An IN list always gets an ephemeral index.
            Direction::Inside => self.relation(format!("SELECT DISTINCT w.fid,w.origin FROM {name} w WHERE w.pre>=0{field} AND (w.fid,w.pre) IN (SELECT fid,pre FROM {})", inner.name), inner.exact && scope.exact)?,
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
        if matches!(direction, Direction::Has) {
            // The inner candidates of a kind-less plan are the scope's descendants.
            let ranges = self.intervals(&scope, field)?;
            let inner_scope = if !anchored(inner) && scope.name != "all_nodes" {
                let frontier = self.frontier(&ranges);
                self.relation(format!("SELECT r.fid,d.value FROM {frontier} r CROSS JOIN generate_series(r.lo,r.hi-1,1) d"), scope.exact)?
            } else {
                self.all()
            };
            let inner = self.plan(inner, inner_scope)?;
            self.indexed.insert(inner.name.clone());
            return self.relation(format!("SELECT r.fid,r.pre FROM {ranges} r WHERE r.lo<r.hi AND EXISTS(SELECT 1 FROM {} m WHERE m.fid=r.fid AND m.pre>=r.lo AND m.pre<r.hi)", inner.name), scope.exact && inner.exact);
        }
        let inner_scope = self.neighbours(&scope, inner, super::tree::ANCESTORS)?;
        let inner = self.plan(inner, inner_scope)?;
        let ranges = self.intervals(&inner, field)?;
        self.indexed.insert(scope.name.clone());
        let frontier = self.frontier(&ranges);
        self.relation(format!("SELECT DISTINCT s.fid,s.pre FROM {frontier} r CROSS JOIN {} s WHERE s.fid=r.fid AND s.pre>=r.lo AND s.pre<r.hi", scope.name), scope.exact && inner.exact)
    }

    /// Subtree intervals are nested or disjoint. Keep the outer intervals once.
    fn frontier(&mut self, ranges: &str) -> String {
        let frontier = format!("frontier{}", self.ctes.len());
        self.ctes.push(format!("{frontier}(fid,lo,hi) AS MATERIALIZED (SELECT fid,lo,hi FROM (SELECT fid,lo,hi,max(hi) OVER(PARTITION BY fid ORDER BY lo,hi DESC ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING) previous FROM {ranges} WHERE lo<hi) WHERE previous IS NULL OR hi>previous)"));
        frontier
    }

    /// A kind-less inner plan would otherwise be evaluated over every node, and relating every
    /// node decodes every node's parent. Its candidates are only the scope's tree neighbours, read
    /// from the tape per scope node (spike `nera_kids`; design index topic 2). An inner plan anchored
    /// on postings keeps them: the rarest anchor is cheaper than every scope node's neighbourhood.
    fn neighbours(&mut self, scope: &Set, inner: &AnchorPlan, axis: i64) -> Result<Set, ()> {
        if anchored(inner) || scope.name == "all_nodes" {
            return Ok(self.all());
        }
        self.relation(
            format!("SELECT DISTINCT s.fid,t.pre FROM {} s CROSS JOIN nera_tree(s.fid,s.pre,{axis}) t", scope.name),
            scope.exact,
        )
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
        let mut postings = Vec::new();
        let mut choices = Vec::new();
        for (number, requirement) in required.iter().enumerate() {
            let (kind, literal) = match &requirement.value {
                Required::Kind(kind) => (*kind, None),
                Required::Terminal(kind, text) => (*kind, Some(text)),
            };
            let mut posting = self.kinds(&BTreeSet::from([kind]))?;
            if let Some(text) = literal {
                posting = self.text(&posting, text, true)?;
            }
            self.indexed.insert(posting.name.clone());
            // Actual posting counts include literal selectivity, as the Rust selector does.
            choices.push(format!(
                "SELECT {number} AS id,(SELECT count(*) FROM {})*{} AS cost",
                posting.name,
                requirement.parents.len() + 1
            ));
            postings.push(posting);
        }
        let choice = format!("choice{}", self.ctes.len());
        self.ctes.push(format!(
            "{choice}(id,cost) AS MATERIALIZED (SELECT id,cost FROM ({}) ORDER BY cost,id LIMIT 1)",
            choices.join(" UNION ALL ")
        ));
        let mut roots = Vec::new();
        for (number, requirement) in required.iter().enumerate() {
            let mut current = self.relation(
                format!(
                    "SELECT p.fid,p.pre FROM {} p JOIN {choice} c ON c.id={number}",
                    postings[number].name
                ),
                false,
            )?;
            for kind in &requirement.parents {
                let properties = self.decoded(&current, Property::Node);
                let hopped =
                    self.relation(format!("SELECT p.fid,p.parent FROM {properties} p"), false)?;
                current = self.relation(
                    format!("SELECT fid,pre FROM {} WHERE pre>=0", hopped.name),
                    false,
                )?;
                if let Some(kind) = kind {
                    let properties = self.decoded(&current, Property::Node);
                    current = self.relation(
                        format!("SELECT p.fid,p.pre FROM {properties} p WHERE p.kind={kind}"),
                        false,
                    )?;
                }
            }
            roots.push(format!("SELECT fid,pre FROM {}", current.name));
        }
        let roots = self.relation(roots.join(" UNION "), false)?;
        let selected = self.combine(&scope, &roots, "INTERSECT", false)?;
        let properties = self.decoded(&selected, Property::Node);
        let spans = format!("span{}", self.ctes.len());
        self.ctes.push(format!(
            "{spans}(fid,pre,end) AS MATERIALIZED (SELECT s.fid,s.pre,s.end FROM {properties} s)"
        ));
        let tests: Vec<_> = postings
            .iter()
            .map(|p| {
                format!(
                    "EXISTS(SELECT 1 FROM {} p WHERE p.fid=s.fid AND p.pre>=s.pre AND p.pre<s.end)",
                    p.name
                )
            })
            .collect();
        self.relation(
            format!(
                "SELECT s.fid,s.pre FROM {spans} s WHERE {}",
                tests.join(" AND ")
            ),
            false,
        )
    }
}

pub(super) fn batch_files(plans: &[&AnchorPlan], lang: u32, files: &[i64]) -> Statement {
    batch_scope(plans, lang, Some(files), None)
}

#[cfg(test)]
pub(super) fn batch(
    plans: &[&AnchorPlan],
    lang: u32,
    fid: Option<i64>,
    path_glob: Option<&str>,
) -> Statement {
    batch_scope(
        plans,
        lang,
        fid.as_ref().map(std::slice::from_ref),
        path_glob,
    )
}

fn scope(lang: u32, fids: Option<&[i64]>, path_glob: Option<&str>) -> Lower {
    let mut lower = Lower::new(lang);
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

/// Whether a plan selects from kind postings before it reads its scope: then evaluating it over
/// every node costs only its postings and their neighbours. A conjunction evaluates its anchored
/// members first (`plan`).
fn anchored(plan: &AnchorPlan) -> bool {
    plan.root_kinds.is_some()
        || match &plan.condition {
            Condition::Pattern(required) => !required.is_empty(),
            Condition::All(plans) => plans.iter().any(anchored),
            Condition::Any(plans) => plans.iter().all(anchored),
            Condition::Utility(inner) => anchored(inner),
            // Evaluated from the inner postings when the scope is every node (`related`).
            Condition::Relation {
                inner,
                direction: Direction::Inside,
                recursive: false,
                ..
            }
            | Condition::Relation {
                inner,
                direction: Direction::Has,
                recursive: false,
                field: None,
            } => anchored(inner),
            Condition::Always
            | Condition::Text { .. }
            | Condition::Not(_)
            | Condition::Relation { .. } => false,
        }
}

fn batch_scope(
    plans: &[&AnchorPlan],
    lang: u32,
    fids: Option<&[i64]>,
    path_glob: Option<&str>,
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
    for (qid, plan) in plans.iter().enumerate() {
        // Admission belongs to the individual plan. Another consumer's cached
        // producers cannot expand or shrink this plan's existing 256-node allowance.
        let mut lower = scope(lang, fids, path_glob);
        lower.consumer = qid;
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
                #[cfg(feature = "code-rule-profile")]
                crate::code_rule_dsl::profile::record_anchor_plan_broadening();
                // Discard the entire failed fragment, including predicates, parameters
                // and reuse state. Earlier fragments already belong to the shared program.
                lower = scope(lang, fids, path_glob);
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
    for demand in ctes
        .iter()
        .filter_map(|cte| decoding.get(cte.split_once('(').expect("CTE columns").0))
    {
        if !(universal.is_some() && demand.property == Property::Node) {
            indexed.insert(demand.scope.clone());
        }
    }
    if universal.is_some() {
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
        .filter(|(_, demand)| matches!(demand.property, Property::Span | Property::Child(_)))
        .map(|(name, _)| name.clone())
        .collect();
    let (levels, uses) = waves::analyze(&ctes, &barrier_names, &query);
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
            && !(universal.is_some() && demand.property == Property::Node)
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
