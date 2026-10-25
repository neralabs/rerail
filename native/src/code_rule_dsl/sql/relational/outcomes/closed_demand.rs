//! Conservative, closed producer demands from the immutable source plan.
//! Predicates can remove impossible body invocations. Value/cache/binding work
//! is never executed here. Unknown contexts retain the existing wider domain.
use super::super::{Axis, Op, Plan};
use super::Sql;
use crate::code_rule_dsl::sql::relational::emit::condition;
use std::collections::{BTreeMap, BTreeSet};
#[derive(Default)]
struct Builder {
    prefix: String,
    definitions: Vec<String>,
    dependencies: BTreeMap<String, Vec<String>>,
    demands: BTreeMap<usize, Vec<String>>,
    demand_order: Vec<usize>,
    blocked: BTreeSet<usize>,
    visits: usize,
    alias: usize,
}
impl Builder {
    fn record_demand(&mut self, key: usize, input: &str) {
        if !self.demands.contains_key(&key) {
            self.demand_order.push(key);
        }
        self.demands.entry(key).or_default().push(input.into());
    }
    // Addresses identify producers but must not determine emitted SQL names.
    fn take_demands(&mut self) -> Vec<(usize, Vec<String>)> {
        std::mem::take(&mut self.demand_order)
            .into_iter()
            .map(|key| {
                (
                    key,
                    self.demands.remove(&key).expect("recorded producer demand"),
                )
            })
            .collect()
    }

    fn relation(&mut self, query: String, dependencies: &[&str]) -> String {
        let name = format!("{}_{}", self.prefix, self.definitions.len());
        self.dependencies.insert(
            name.clone(),
            dependencies.iter().map(|s| s.to_string()).collect(),
        );
        self.definitions
            .push(format!("{name}(file,node) AS MATERIALIZED ({query})"));
        name
    }
    fn exact(&mut self, p: &Plan, n: &str) -> Option<String> {
        match &p.op {
            Op::Predicate(s) => condition(s, n).ok(),
            Op::Shared(p) => self.exact(p, n),
            Op::All(v) | Op::Any(v) => {
                let all = matches!(p.op, Op::All(_));
                if v.is_empty() {
                    return Some(usize::from(all).to_string());
                }
                Some(
                    v.iter()
                        .map(|p| self.exact(p, n).map(|s| format!("({s})")))
                        .collect::<Option<Vec<_>>>()?
                        .join(if all { " AND " } else { " OR " }),
                )
            }
            Op::Not(p) => Some(format!("NOT ({})", self.exact(p, n)?)),
            Op::FieldNeighbor {
                inside,
                field,
                child,
            } => {
                self.alias += 1;
                let c = format!("pd{}", self.alias);
                let body = self.exact(child, &c)?;
                let edge = if *inside {
                    format!("{c}.id={n}.parent AND {n}.field={field} AND {n}.ffirst=1")
                } else {
                    format!("{c}.parent={n}.id AND {c}.field={field} AND {c}.ffirst=1")
                };
                Some(format!(
                    "EXISTS(SELECT 1 FROM node {c} WHERE {c}.file={n}.file AND {edge} AND ({body}))"
                ))
            }
            Op::Search(axis, false, child, None) => {
                self.alias += 1;
                let c = format!("pd{}", self.alias);
                let body = self.exact(child, &c)?;
                let edge = edge(*axis, n, &c);
                Some(format!(
                    "EXISTS(SELECT 1 FROM node {c} WHERE {c}.file={n}.file AND {edge} AND ({body}))"
                ))
            }
            _ => None,
        }
    }
    fn possible(&mut self, p: &Plan, n: &str, truth: bool) -> String {
        if let Some(exact) = self.exact(p, n) {
            return if truth {
                exact
            } else {
                format!("NOT ({exact})")
            };
        }
        // Native Any can inspect later alternatives after a failed utility.
        // A successful body proof cannot exclude those failure followers.
        if !truth && p.fallible() {
            return "1".into();
        }
        match &p.op {
            Op::Shared(p) | Op::Charge(_, p) | Op::TransientCharge(p) | Op::MemoPredicate(_, p) => {
                self.possible(p, n, truth)
            }
            Op::Not(p) => self.possible(p, n, !truth),
            Op::All(v) | Op::Any(v) => {
                let all = matches!(p.op, Op::All(_));
                let conjunction = all == truth;
                if v.is_empty() {
                    return usize::from(all == truth).to_string();
                }
                v.iter()
                    .map(|p| format!("({})", self.possible(p, n, truth)))
                    .collect::<Vec<_>>()
                    .join(if conjunction { " AND " } else { " OR " })
            }
            _ => "1".into(),
        }
    }
    fn walk(&mut self, p: &Plan, input: Option<&str>) -> Option<()> {
        self.visits += 1;
        if self.visits > 2048 {
            return None;
        }
        match &p.op {
            Op::PureRecursive(p) => {
                if let Some(projection) = &p.projection {
                    let key =
                        projection as *const super::super::projected_tail::Projection as usize;
                    if let Some(input) = input {
                        self.record_demand(key, input);
                    } else {
                        self.blocked.insert(key);
                    }
                }
            }
            Op::Shared(p) | Op::Charge(_, p) | Op::TransientCharge(p) | Op::MemoPredicate(_, p) => {
                self.walk(p, input)?
            }
            Op::Not(p) => self.walk(p, input)?,
            Op::All(v) | Op::Any(v) => {
                let all = matches!(p.op, Op::All(_));
                let mut current = input.map(str::to_owned);
                for child in v {
                    self.walk(child, current.as_deref())?;
                    if let Some(before) = &current {
                        let guard = self.possible(child, "n", all);
                        if guard != "1" {
                            current=Some(self.relation(format!("SELECT d.file,d.node FROM {before} d JOIN node n ON n.file=d.file AND n.id=d.node WHERE {guard}"),&[before]));
                        }
                    }
                }
            }
            Op::Search(axis, end, child, stop) => {
                if let Some(input) =
                    input.filter(|_| stop.is_none() && (!end || matches!(axis, Axis::Child)))
                {
                    let domain = if *end {
                        "c.id>n.id AND c.id<=n.\"last\"".into()
                    } else {
                        edge(*axis, "n", "c")
                    };
                    let next=self.relation(format!("SELECT DISTINCT c.file,c.id FROM {input} d JOIN node n ON n.file=d.file AND n.id=d.node CROSS JOIN node c ON c.file=n.file AND ({domain})"),&[input]);
                    self.walk(child, Some(&next))?;
                } else {
                    self.walk(child, None)?;
                }
                if let Some(stop) = stop {
                    self.walk(stop, None)?;
                }
            }
            Op::FieldNeighbor {
                inside,
                field,
                child,
            } => {
                if let Some(input) = input {
                    let domain = if *inside {
                        format!("c.id=n.parent AND n.field={field} AND n.ffirst=1")
                    } else {
                        format!("c.parent=n.id AND c.field={field} AND c.ffirst=1")
                    };
                    let next=self.relation(format!("SELECT DISTINCT c.file,c.id FROM {input} d JOIN node n ON n.file=d.file AND n.id=d.node CROSS JOIN node c ON c.file=n.file AND ({domain})"),&[input]);
                    self.walk(child, Some(&next))?;
                } else {
                    self.walk(child, None)?;
                }
            }
            Op::MixedRecursive { program, .. } => {
                for p in &program.regions {
                    self.walk(p, None)?;
                }
            }
            Op::ParsedRecursive { program, .. } => {
                for p in &program.regions {
                    self.walk(p, None)?;
                }
            }
            Op::RecursiveSearch { component, .. } => self.walk(&component.base, None)?,
            Op::FiniteProgram(p) => {
                for p in &p.regions {
                    self.walk(p, None)?;
                }
            }
            Op::FieldEnd { child, .. } | Op::Host { child, .. } | Op::Parse { child, .. } => {
                self.walk(child, None)?
            }
            Op::CallFrame { body, .. } => self.walk(body, None)?,
            Op::FieldRule { child, stop, .. } => {
                self.walk(child, None)?;
                self.walk(stop, None)?;
            }
            Op::Binding {
                core,
                children,
                occurrence,
                ..
            } => {
                self.walk(core, None)?;
                for (_, p) in children {
                    self.walk(p, None)?;
                }
                if let Some(p) = occurrence {
                    self.walk(p, None)?;
                }
            }
            Op::Constrained(p, v) => {
                self.walk(p, None)?;
                for (_, p) in v {
                    self.walk(p, None)?;
                }
            }
            Op::Count { rule, stop, .. } => {
                self.walk(rule, None)?;
                if let Some(p) = stop {
                    self.walk(p, None)?;
                }
            }
            Op::NthChild {
                of_rule: Some(p), ..
            } => self.walk(p, None)?,
            Op::FormatArgs {
                unused, missing, ..
            } => {
                for p in unused.iter().chain(missing) {
                    self.walk(p, None)?;
                }
            }
            _ => {}
        }
        Some(())
    }
}
fn edge(axis: Axis, n: &str, c: &str) -> String {
    match axis {
        Axis::Child => format!("{c}.parent={n}.id"),
        Axis::Parent => format!("{c}.id={n}.parent"),
        Axis::Previous => format!("{c}.id={n}.prev"),
        Axis::Next => format!("{c}.id={n}.next"),
    }
}
impl Sql {
    pub(super) fn collect_closed_demands(&mut self, source: &Plan) -> Result<(), String> {
        if self.generation.components.borrow().closed_demand_collected {
            return Ok(());
        }
        self.generation
            .components
            .borrow_mut()
            .closed_demand_collected = true;
        if !source.mentions.is_empty() || !source.writes.is_empty() || self.generation.memo_bypass {
            return Ok(());
        }
        let roots = self.generation.components.borrow().roots.clone();
        if roots.is_empty() {
            return Ok(());
        }
        let mut b = Builder {
            prefix: format!("{}_demands", self.projection_namespace()),
            definitions: vec![],
            dependencies: BTreeMap::new(),
            demands: BTreeMap::new(),
            demand_order: vec![],
            blocked: BTreeSet::new(),
            visits: 0,
            alias: 0,
        };
        if b.walk(source, Some(&roots)).is_none() {
            return Ok(());
        }
        let mut closed = vec![];
        for (key, inputs) in b.take_demands() {
            if b.blocked.contains(&key) {
                continue;
            }
            let inputs = inputs.into_iter().collect::<BTreeSet<_>>();
            let name = b.relation(
                inputs
                    .iter()
                    .map(|name| format!("SELECT file,node FROM {name}"))
                    .collect::<Vec<_>>()
                    .join(" UNION "),
                &inputs.iter().map(String::as_str).collect::<Vec<_>>(),
            );
            closed.push((key, name));
        }
        if closed.is_empty() {
            return Ok(());
        }
        let mut live = BTreeSet::new();
        let mut pending = closed.iter().map(|(_, n)| n.clone()).collect::<Vec<_>>();
        while let Some(n) = pending.pop() {
            if live.insert(n.clone())
                && let Some(deps) = b.dependencies.get(&n)
            {
                pending.extend(deps.iter().cloned());
            }
        }
        for (key, target) in &closed {
            let mut required = BTreeSet::new();
            let mut pending = vec![target.clone()];
            while let Some(n) = pending.pop() {
                if required.insert(n.clone())
                    && let Some(deps) = b.dependencies.get(&n)
                {
                    pending.extend(deps.iter().cloned());
                }
            }
            let definitions = b
                .definitions
                .iter()
                .enumerate()
                .filter_map(|(i, s)| {
                    required
                        .contains(&format!("{}_{}", b.prefix, i))
                        .then_some(s.clone())
                })
                .collect::<Vec<_>>();
            self.generation
                .components
                .borrow_mut()
                .stage_demands
                .insert(*key, definitions);
        }
        let definitions = b
            .definitions
            .into_iter()
            .enumerate()
            .filter_map(|(i, s)| live.contains(&format!("{}_{}", b.prefix, i)).then_some(s))
            .collect::<Vec<_>>();
        self.profile.push(serde_json::json!({"operator":"closed potential demands","producers":closed.len(),"relations":definitions.len(),"root":roots,"callbacks":false}));
        for _ in 0..definitions.len() {
            self.generation.operator_as("closed potential demand")?;
        }
        self.register_projection_definitions(definitions);
        self.generation
            .components
            .borrow_mut()
            .closed_demands
            .extend(closed);
        Ok(())
    }
}

#[cfg(test)]
mod identity_order_tests {
    use super::Builder;

    #[test]
    fn producer_demand_order_is_independent_of_addresses() {
        for keys in [[9, 1], [1, 9]] {
            let mut builder = Builder::default();
            builder.record_demand(keys[0], "first");
            builder.record_demand(keys[1], "second");
            builder.record_demand(keys[0], "third");
            let demands = builder.take_demands();
            assert_eq!(
                demands
                    .iter()
                    .map(|(_, inputs)| inputs.clone())
                    .collect::<Vec<_>>(),
                vec![vec!["first", "third"], vec!["second"]]
            );
            assert!(builder.take_demands().is_empty());
        }
    }
}
