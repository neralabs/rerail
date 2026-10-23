//! Ordered single-leaf answers and value-keyed negative probes.
//! Traversal rows contain node keys; captures enter only at binding and answer boundaries.
use super::{
    Compiled, demand, emit,
    ir::{self, Set},
    lower::Lowerer,
    physical, witness,
};
use crate::code_rule_dsl::matcher::{AstMatcher, GrammarMatcher, StopBy};
use ast_grep_core::{matcher::PatternNode, meta_var::MetaVariable};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    Child,
    Descendant,
    InclusiveDescendant,
}
#[derive(Clone, PartialEq, Eq)]
enum Answer {
    Frontier,
    Filter(Box<Answer>, Vec<Set>, bool, bool, Vec<Probe>),
    Any(Vec<Answer>),
    Expand(Axis, Box<Answer>),
}
#[derive(Clone, PartialEq, Eq)]
struct Path {
    filters: Vec<Set>,
    edges: Vec<Axis>,
    end: End,
}
#[derive(Clone, PartialEq, Eq)]
enum End {
    Bind(bool),
    State(usize),
}
#[derive(Clone, Default, PartialEq, Eq)]
struct Probe {
    states: Vec<Vec<Path>>,
    utilities: HashMap<(usize, usize), usize>,
}
struct Builder<'a> {
    lower: Lowerer,
    search: &'a witness::Search,
    active: HashSet<(usize, usize)>,
    answers: HashMap<(usize, usize), Answer>,
    origin_predicates: Vec<Set>,
}
impl Builder<'_> {
    fn binding(&self, rule: &AstMatcher) -> Result<bool, String> {
        if let AstMatcher::Pattern(p) = rule
            && let PatternNode::MetaVar {
                meta_var: MetaVariable::Capture(name, named),
            } = &p.node
            && name == &self.search.capture
        {
            return Ok(*named);
        }
        Err("ordered answers: expected the same whole-node capture".into())
    }
    // Borrow original utility nodes: the frontier identity belongs to the source AST.
    fn terms<'a>(rule: &'a AstMatcher, out: &mut Vec<&'a AstMatcher>) {
        if let AstMatcher::All(items) = rule {
            for item in items {
                Self::terms(item, out);
            }
        } else {
            out.push(rule);
        }
    }
    fn answer(&mut self, rule: &AstMatcher) -> Result<Answer, String> {
        if rule as *const AstMatcher as usize == self.search.site {
            return Ok(Answer::Frontier);
        }
        match rule {
            AstMatcher::Util(u) => {
                let key = (u.table.as_ptr() as usize, u.index);
                if let Some(answer) = self.answers.get(&key) {
                    return Ok(answer.clone());
                }
                if !self.active.insert(key) {
                    return Err("ordered answers: positive recursive capture needs an ordered component plan".into());
                }
                let answer = u
                    .with_target(|r| self.answer(r))
                    .ok_or("ordered answers: missing utility")?;
                self.active.remove(&key);
                if let Ok(answer) = &answer {
                    self.answers.insert(key, answer.clone());
                }
                answer
            }
            AstMatcher::All(_) => {
                let mut terms = vec![];
                Self::terms(rule, &mut terms);
                let mut pure = vec![];
                let mut probes = vec![];
                let mut answer = None;
                let (mut bind, mut overwrite) = (false, false);
                for term in terms {
                    if witness::pure(term) {
                        pure.push(self.lower.set(term)?);
                    } else if matches!(term, AstMatcher::Pattern(_)) {
                        let named = self.binding(term)?;
                        if !named {
                            return Err(
                                "ordered answers: unnamed capture keys are not admitted".into()
                            );
                        }
                        bind = true;
                        overwrite = answer.is_some();
                    } else if let AstMatcher::Not(body) = term {
                        let mut probe = Probe::default();
                        probe.states.push(vec![]);
                        probe.states[0] = self.paths(body, &mut probe)?;
                        probe.validate()?;
                        probes.push(probe);
                    } else {
                        if answer.is_some() {
                            return Err(
                                "ordered answers: conjunction needs one outward capture producer"
                                    .into(),
                            );
                        }
                        answer = Some(self.answer(term)?);
                    }
                }
                if matches!(answer, Some(Answer::Frontier)) {
                    // The frontier's origin scan already proved these same-node predicates.
                    pure.retain(|set| {
                        let mut terms = vec![];
                        conjuncts(set, &mut terms);
                        !terms
                            .iter()
                            .all(|term| self.origin_predicates.contains(term))
                    });
                }
                Ok(Answer::Filter(
                    Box::new(answer.ok_or("ordered answers: binding without a demand producer")?),
                    pure,
                    bind,
                    overwrite,
                    probes,
                ))
            }
            AstMatcher::Any(items) => Ok(Answer::Any(
                items
                    .iter()
                    .map(|r| self.answer(r))
                    .collect::<Result<_, _>>()?,
            )),
            AstMatcher::Inside(body, stop) => Ok(Answer::Expand(
                match stop {
                    StopBy::Neighbor => Axis::Child,
                    StopBy::End => Axis::Descendant,
                    _ => {
                        return Err(
                            "ordered answers: unsupported ancestor choice outside the frontier"
                                .into(),
                        );
                    }
                },
                Box::new(self.answer(body)?),
            )),
            _ => Err("ordered answers: unsupported outward capture operator".into()),
        }
    }
    fn paths(&mut self, rule: &AstMatcher, probe: &mut Probe) -> Result<Vec<Path>, String> {
        match rule {
            AstMatcher::Util(u) if !witness::pure(rule) => {
                let key = (u.table.as_ptr() as usize, u.index);
                let state = if let Some(id) = probe.utilities.get(&key) {
                    *id
                } else {
                    let id = probe.states.len();
                    probe.states.push(vec![]);
                    probe.utilities.insert(key, id);
                    probe.states[id] = u
                        .with_target(|r| self.paths(r, probe))
                        .ok_or("negative probe: missing utility")??;
                    id
                };
                Ok(vec![Path {
                    filters: vec![Set::AllNodes],
                    edges: vec![],
                    end: End::State(state),
                }])
            }
            AstMatcher::Any(items) => {
                let mut paths = vec![];
                for r in items {
                    paths.extend(self.paths(r, probe)?);
                }
                Ok(paths)
            }
            AstMatcher::All(_) => {
                let mut terms = vec![];
                Self::terms(rule, &mut terms);
                let mut pure = vec![];
                let mut paths = None;
                for term in terms {
                    if witness::pure(term) {
                        pure.push(self.lower.set(term)?);
                    } else {
                        if paths.is_some() {
                            return Err(
                                "negative probe: multiple capture operations at one path node"
                                    .into(),
                            );
                        }
                        paths = Some(self.paths(term, probe)?);
                    }
                }
                let mut paths = paths.ok_or("negative probe: missing capture path")?;
                for path in &mut paths {
                    path.filters[0] =
                        Set::Intersect(vec![path.filters[0].clone(), Set::Intersect(pure.clone())]);
                }
                Ok(paths)
            }
            AstMatcher::Has(body, stop) => {
                let axis = match stop {
                    StopBy::Neighbor => Axis::Child,
                    StopBy::End => Axis::Descendant,
                    _ => return Err("negative probe: unsupported descendant stop".into()),
                };
                let mut paths = self.paths(body, probe)?;
                for path in &mut paths {
                    path.edges.insert(0, axis);
                    path.filters.insert(0, Set::AllNodes);
                }
                Ok(paths)
            }
            AstMatcher::Pattern(_) => Ok(vec![Path {
                filters: vec![Set::AllNodes],
                edges: vec![],
                end: End::Bind(self.binding(rule)?),
            }]),
            _ => Err("negative probe: unsupported value-dependent operator".into()),
        }
    }
}
impl Probe {
    fn validate(&self) -> Result<(), String> {
        fn visit(id: usize, p: &Probe, active: &mut HashSet<usize>) -> Result<(), String> {
            if !active.insert(id) {
                return Err("negative probe: recursion must move strictly down the tree".into());
            }
            for path in &p.states[id] {
                if path.edges.is_empty()
                    && let End::State(next) = path.end
                {
                    visit(next, p, active)?;
                }
            }
            active.remove(&id);
            Ok(())
        }
        for id in 0..self.states.len() {
            visit(id, self, &mut HashSet::new())?;
        }
        Ok(())
    }
}
fn rewrite(
    answer: &mut Answer,
    lower: &mut Lowerer,
    roots: usize,
    cache: &mut HashMap<usize, usize>,
) {
    fn set(s: &mut Set, lower: &mut Lowerer, roots: usize, cache: &mut HashMap<usize, usize>) {
        *s = demand::bound(s, roots, &mut lower.program, cache);
        *s = demand::inline(s, &lower.program);
        let mut deps = vec![];
        ir::references(s, &mut deps);
        if let Set::Union(outputs) = &mut lower.program.relations[0].expression {
            outputs.extend(deps.into_iter().map(Set::Reference));
        }
    }
    match answer {
        Answer::Frontier => {}
        Answer::Filter(child, filters, _, _, probes) => {
            rewrite(child, lower, roots, cache);
            for s in filters {
                set(s, lower, roots, cache);
            }
            for probe in probes {
                for state in &mut probe.states {
                    for path in state {
                        for s in &mut path.filters {
                            set(s, lower, roots, cache);
                        }
                    }
                }
            }
        }
        Answer::Any(items) => {
            for a in items {
                rewrite(a, lower, roots, cache);
            }
        }
        Answer::Expand(_, child) => rewrite(child, lower, roots, cache),
    }
}
fn conjuncts(set: &Set, out: &mut Vec<Set>) {
    match set {
        Set::Intersect(items) => {
            for item in items {
                conjuncts(item, out);
            }
        }
        Set::AllNodes => {}
        _ => out.push(set.clone()),
    }
}

struct Sql<'a> {
    program: &'a ir::Program,
    definitions: Vec<String>,
    next: usize,
    shared: Vec<(Answer, String)>,
    probes: Vec<(Probe, String, String)>,
    leaves: Vec<(Set, bool, String)>,
}
impl Sql<'_> {
    fn name(&mut self) -> String {
        let id = self.next;
        self.next += 1;
        format!("answer{id}")
    }
    fn condition(s: &Set, n: &str) -> Result<String, String> {
        emit::condition(s, n)
    }
    fn kinds(&self, answer: &Answer) -> Option<std::collections::BTreeSet<u16>> {
        match answer {
            Answer::Filter(child, filters, ..) => {
                let own = demand::kinds(
                    &Set::Intersect(filters.clone()),
                    self.program,
                    &mut std::collections::BTreeSet::new(),
                );
                match (own, self.kinds(child)) {
                    (Some(a), Some(b)) => Some(a.intersection(&b).copied().collect()),
                    (a, b) => a.or(b),
                }
            }
            _ => None,
        }
    }
    fn inclusive(&self, answer: &Answer, filters: &[Set]) -> Option<Answer> {
        let Answer::Any(items) = answer else {
            return None;
        };
        let Answer::Expand(Axis::Descendant, base) = items.last()? else {
            return None;
        };
        let branches: Vec<_> = match base.as_ref() {
            Answer::Any(items) => items.iter().collect(),
            other => vec![other],
        };
        let admitted = |a: &Answer| {
            !self.empty(&Answer::Filter(
                Box::new(a.clone()),
                filters.to_vec(),
                false,
                false,
                vec![],
            ))
        };
        let direct: Vec<_> = items[..items.len() - 1]
            .iter()
            .filter(|a| admitted(a))
            .collect();
        let admitted_branches: Vec<_> = branches.into_iter().filter(|a| admitted(a)).collect();
        if direct.is_empty() || direct != admitted_branches {
            return None;
        }
        // An admitted body root wins before strict descendants. Inclusive descent
        // ordered by decreasing body ID makes that same root the nearest choice.
        Some(Answer::Expand(Axis::InclusiveDescendant, base.clone()))
    }
    fn empty(&self, answer: &Answer) -> bool {
        if self.kinds(answer).is_some_and(|k| k.is_empty()) {
            return true;
        }
        match answer {
            Answer::Frontier => false,
            Answer::Expand(_, child) | Answer::Filter(child, ..) => self.empty(child),
            Answer::Any(items) => items.iter().all(|a| self.empty(a)),
        }
    }
    fn answer(&mut self, answer: &Answer) -> Result<String, String> {
        if let Some((_, name)) = self.shared.iter().find(|(key, _)| key == answer) {
            return Ok(name.clone());
        }
        let name = self.uncached(answer)?;
        self.shared.push((answer.clone(), name.clone()));
        Ok(name)
    }
    fn uncached(&mut self, answer: &Answer) -> Result<String, String> {
        match answer {
            Answer::Frontier => Ok("capture_demand".into()),
            Answer::Filter(child, filters, bind, overwrite, probes) => {
                if let Answer::Filter(
                    inner,
                    inner_filters,
                    inner_bind,
                    inner_overwrite,
                    inner_probes,
                ) = child.as_ref()
                {
                    let mut combined = inner_filters.clone();
                    combined.extend(filters.clone());
                    let mut negatives = inner_probes.clone();
                    negatives.extend(probes.clone());
                    return self.answer(&Answer::Filter(
                        inner.clone(),
                        combined,
                        *bind || *inner_bind,
                        *overwrite || *inner_overwrite,
                        negatives,
                    ));
                }
                if demand::kinds(
                    &Set::Intersect(filters.clone()),
                    self.program,
                    &mut std::collections::BTreeSet::new(),
                )
                .is_some_and(|k| k.is_empty())
                {
                    let name = self.name();
                    self.definitions.push(format!(
                        "{name}(file,node,atom,witness) AS (SELECT NULL,NULL,NULL,NULL WHERE 0)"
                    ));
                    return Ok(name);
                }
                if let Some(inclusive) = self.inclusive(child, filters) {
                    return self.answer(&Answer::Filter(
                        Box::new(inclusive),
                        filters.clone(),
                        *bind,
                        *overwrite,
                        probes.clone(),
                    ));
                }
                let mut remaining = filters.clone();
                let (source, expanded) = if let Answer::Expand(axis, inner) = child.as_ref() {
                    (self.expand(*axis, inner, filters, *bind)?, true)
                } else if let Answer::Any(items) = child.as_ref() {
                    // These conditions depend on root and value, never the selected witness.
                    // They therefore commute with branch choice and reach indexed expansions.
                    let mut flat = Vec::new();
                    for set in filters {
                        conjuncts(set, &mut flat);
                    }
                    let (pushed, retained): (Vec<_>, Vec<_>) =
                        flat.into_iter().partition(|s| matches!(s, Set::Kinds(_)));
                    remaining = retained;
                    let filtered = Answer::Any(
                        items
                            .iter()
                            .map(|a| {
                                Answer::Filter(
                                    Box::new(a.clone()),
                                    pushed.clone(),
                                    *bind,
                                    false,
                                    vec![],
                                )
                            })
                            .collect(),
                    );
                    (self.answer(&filtered)?, true)
                } else {
                    (self.answer(child)?, false)
                };
                if expanded
                    && (remaining.is_empty() || !matches!(child.as_ref(), Answer::Any(_)))
                    && probes.is_empty()
                    && !overwrite
                {
                    return Ok(source);
                }
                let name = self.name();
                let mut conditions = vec!["1".to_string()];
                let mut bound = super::bound::Bound::default();
                if !expanded || matches!(child.as_ref(), Answer::Any(_)) {
                    for s in &remaining {
                        conditions.push(bound.condition(s, "n")?);
                    }
                    if *bind && !expanded {
                        conditions.push(
                            "n.named AND relation_leaf_atom(n.file,n.id,n.atom)=a.atom".into(),
                        );
                    }
                }
                for probe in probes {
                    conditions.extend(self.negative(probe, &source)?);
                }
                let witness = if *overwrite { "a.node" } else { "a.witness" };
                self.definitions.push(format!("{name}(file,node,atom,witness) AS MATERIALIZED (SELECT a.file,a.node,a.atom,{witness} FROM {source} a CROSS JOIN node n ON n.file=a.file AND n.id=a.node{} WHERE {})",bound.joins.join(""),conditions.join(" AND ")));
                Ok(name)
            }
            Answer::Any(items) => {
                let live: Vec<_> = items.iter().filter(|a| !self.empty(a)).collect();
                if let Some(name) = self.child_branches(&live)? {
                    return Ok(name);
                }
                let mut sources = live
                    .into_iter()
                    .map(|a| self.answer(a))
                    .collect::<Result<Vec<_>, _>>()?;
                if sources.len() == 1 {
                    return Ok(sources.remove(0));
                }
                let name = self.name();
                if sources.is_empty() {
                    self.definitions.push(format!(
                        "{name}(file,node,atom,witness) AS (SELECT NULL,NULL,NULL,NULL WHERE 0)"
                    ));
                    return Ok(name);
                }
                let union = sources
                    .iter()
                    .enumerate()
                    .map(|(i, s)| format!("SELECT file,node,atom,witness,{i} AS branch FROM {s}"))
                    .collect::<Vec<_>>()
                    .join(" UNION ALL ");
                self.definitions.push(format!("{name}(file,node,atom,witness) AS MATERIALIZED (SELECT file,node,atom,witness FROM (SELECT *,ROW_NUMBER() OVER (PARTITION BY file,node,atom ORDER BY branch) AS choice FROM ({union})) WHERE choice=1)"));
                Ok(name)
            }
            Answer::Expand(axis, inner) => self.expand(*axis, inner, &[], false),
        }
    }
    fn child_branches(&mut self, items: &[&Answer]) -> Result<Option<String>, String> {
        struct Branch<'a> {
            base: &'a Answer,
            parent: Vec<Set>,
            child: Vec<Set>,
            probes: Vec<Probe>,
            bind: bool,
        }
        fn branch(mut answer: &Answer) -> Option<Branch<'_>> {
            let mut child = Vec::new();
            let mut bind = false;
            while let Answer::Filter(inner, filters, binding, false, probes) = answer {
                if !probes.is_empty() {
                    return None;
                }
                child.extend(filters.clone());
                bind |= binding;
                answer = inner;
            }
            let Answer::Expand(Axis::Child, inner) = answer else {
                return None;
            };
            answer = inner;
            let mut parent = Vec::new();
            let mut probes = Vec::new();
            while let Answer::Filter(inner, filters, false, false, negatives) = answer {
                parent.extend(filters.clone());
                probes.extend(negatives.clone());
                answer = inner;
            }
            Some(Branch {
                base: answer,
                parent,
                child,
                probes,
                bind,
            })
        }
        if items.len() < 2 {
            return Ok(None);
        }
        let Some(branches) = items.iter().map(|a| branch(a)).collect::<Option<Vec<_>>>() else {
            return Ok(None);
        };
        let first = &branches[0];
        if branches
            .iter()
            .any(|b| b.base != first.base || b.probes != first.probes || b.bind != first.bind)
        {
            return Ok(None);
        }
        // Every branch chooses from the same unique (file,parent,atom) answer.
        // For a child, alternatives can differ only in pure tests, not in witness.
        let source = self.answer(&Answer::Filter(
            Box::new(first.base.clone()),
            vec![],
            false,
            false,
            first.probes.clone(),
        ))?;
        let mut bound = super::bound::Bound::default();
        let mut alternatives = Vec::new();
        for branch in &branches {
            alternatives.push(format!(
                "({} AND {})",
                bound.condition(&Set::Intersect(branch.parent.clone()), "b")?,
                bound.condition(&Set::Intersect(branch.child.clone()), "n")?
            ));
        }
        let value = if first.bind {
            " CROSS JOIN node v ON v.file=a.file AND v.id=a.atom"
        } else {
            ""
        };
        let hash = if first.bind {
            " AND n.thash=v.thash"
        } else {
            ""
        };
        let binding = if first.bind {
            " AND n.named AND relation_leaf_atom(n.file,n.id,n.atom)=a.atom"
        } else {
            ""
        };
        let name = self.name();
        self.definitions.push(format!("{name}(file,node,atom,witness) AS MATERIALIZED (SELECT a.file,n.id,a.atom,a.witness FROM {source} a CROSS JOIN node b ON b.file=a.file AND b.id=a.node{value} CROSS JOIN node n ON n.file=b.file AND n.parent=b.id{hash}{} WHERE ({}){binding})", bound.joins.join(""), alternatives.join(" OR ")));
        Ok(Some(name))
    }
    fn expand(
        &mut self,
        axis: Axis,
        inner: &Answer,
        filters: &[Set],
        bind: bool,
    ) -> Result<String, String> {
        let source = self.answer(inner)?;
        let name = self.name();
        let edge = match axis {
            Axis::Child => "n.parent=b.id",
            Axis::Descendant => "n.id>b.id AND n.id<=b.\"last\"",
            Axis::InclusiveDescendant => "n.id>=b.id AND n.id<=b.\"last\"",
        };
        let mut bound = super::bound::Bound::default();
        let mut flat = Vec::new();
        for filter in filters {
            physical::expand(filter)?;
            conjuncts(filter, &mut flat);
        }
        let mut kinds = Vec::new();
        flat.retain(|set| {
            if matches!(set, Set::Kinds(_)) {
                if kinds.contains(set) {
                    return false;
                }
                kinds.push(set.clone());
            }
            true
        });
        let mut conditions = flat
            .iter()
            .map(|s| bound.condition(s, "n"))
            .collect::<Result<Vec<_>, _>>()?;
        let value = if bind {
            conditions.push("n.named AND relation_leaf_atom(n.file,n.id,n.atom)=a.atom".into());
            " CROSS JOIN node v ON v.file=a.file AND v.id=a.atom"
        } else {
            ""
        };
        let hash = if bind { " AND n.thash=v.thash" } else { "" };
        if conditions.is_empty() {
            conditions.push("1".into());
        }
        let from = format!(
            "{source} a CROSS JOIN node b ON b.file=a.file AND b.id=a.node{value} CROSS JOIN node n ON n.file=b.file AND {edge}{hash}{} WHERE {}",
            bound.joins.join(""),
            conditions.join(" AND ")
        );
        let query = if axis == Axis::Child {
            // A child has one parent; each (file,parent,atom) answer is already unique.
            format!("SELECT a.file,n.id,a.atom,a.witness FROM {from}")
        } else {
            format!(
                "SELECT file,node,atom,witness FROM (SELECT a.file,n.id AS node,a.atom,a.witness,ROW_NUMBER() OVER (PARTITION BY a.file,n.id,a.atom ORDER BY a.node DESC) AS choice FROM {from}) WHERE choice=1"
            )
        };
        self.definitions.push(format!(
            "{name}(file,node,atom,witness) AS MATERIALIZED ({query})"
        ));
        Ok(name)
    }
    fn path(path: &Path) -> Result<(String, String, String), String> {
        Self::paths(std::slice::from_ref(path))
    }
    fn paths(paths: &[Path]) -> Result<(String, String, String), String> {
        let mut bound = super::bound::Bound::default();
        let mut conditions = Vec::new();
        let mut source = String::new();
        let mut leaf = String::new();
        for path in paths {
            let (from, filter, last) = Self::path_parts(path, &mut bound)?;
            if !source.is_empty() {
                assert_eq!(source, from);
            }
            source = from;
            leaf = last;
            conditions.push(filter);
        }
        source.push_str(&bound.joins.join(""));
        Ok((source, format!("({})", conditions.join(" OR ")), leaf))
    }
    fn path_parts(
        path: &Path,
        bound: &mut super::bound::Bound,
    ) -> Result<(String, String, String), String> {
        let mut from = "node n0 ON n0.file=w.file AND n0.id=w.node".to_string();
        let mut filters = vec![bound.condition(&path.filters[0], "n0")?];
        for (i, axis) in path.edges.iter().enumerate() {
            let j = i + 1;
            let edge = match axis {
                Axis::Child => format!("n{j}.parent=n{i}.id"),
                Axis::Descendant => format!("n{j}.id>n{i}.id AND n{j}.id<=n{i}.\"last\""),
                Axis::InclusiveDescendant => format!("n{j}.id>=n{i}.id AND n{j}.id<=n{i}.\"last\""),
            };
            from.push_str(&format!(
                " CROSS JOIN node n{j} ON n{j}.file=n{i}.file AND {edge}"
            ));
            filters.push(bound.condition(&path.filters[j], &format!("n{j}"))?);
        }
        Ok((
            from,
            {
                filters.retain(|condition| condition != "1");
                if filters.is_empty() {
                    "1".into()
                } else {
                    filters.join(" AND ")
                }
            },
            format!("n{}", path.edges.len()),
        ))
    }
    fn fold_leaf_edges(&self, probe: &mut Probe) {
        let original = probe.clone();
        for state in &mut probe.states {
            for path in state {
                let End::Bind(named) = path.end else {
                    continue;
                };
                if path.edges.is_empty() {
                    continue;
                }
                let end = path.filters.last().unwrap();
                let Some(kinds) =
                    demand::kinds(end, self.program, &mut std::collections::BTreeSet::new())
                else {
                    continue;
                };
                for (id, candidates) in original.states.iter().enumerate() {
                    let same_leaf = |p: &Path| {
                        p.edges.is_empty()
                            && p.end == End::Bind(named)
                            && p.filters == vec![end.clone()]
                    };
                    if candidates.iter().any(same_leaf)
                        && candidates.iter().all(|p| {
                            same_leaf(p)
                                || demand::kinds(
                                    &p.filters[0],
                                    self.program,
                                    &mut std::collections::BTreeSet::new(),
                                )
                                .is_some_and(|other| kinds.is_disjoint(&other))
                        })
                    {
                        path.end = End::State(id);
                        break;
                    }
                }
            }
        }
    }
    fn negative(&mut self, probe: &Probe, source: &str) -> Result<Vec<String>, String> {
        let mut remaining = probe.clone();
        let mut conditions = Vec::new();
        let mut paths = Vec::new();
        for path in &probe.states[0] {
            let mut root = Vec::new();
            conjuncts(&path.filters[0], &mut root);
            let mut terms = Vec::new();
            conjuncts(path.filters.last().unwrap(), &mut terms);
            let references: Vec<_> = terms
                .iter()
                .filter_map(|s| {
                    if let Set::Reference(id) = s {
                        Some(*id)
                    } else {
                        None
                    }
                })
                .collect();
            if let End::Bind(named) = path.end
                && path.edges == [Axis::Descendant]
                && root.is_empty()
                && references.len() == 1
                && terms
                    .iter()
                    .all(|s| matches!(s, Set::Kinds(_) | Set::Reference(_)))
            {
                let set = Set::Intersect(terms);
                let name = if let Some((_, _, name)) = self
                    .leaves
                    .iter()
                    .find(|(s, n, _)| *s == set && *n == named)
                {
                    name.clone()
                } else {
                    let name = self.name();
                    let mut filters = Vec::new();
                    conjuncts(&set, &mut filters);
                    filters.retain(|s| !matches!(s, Set::Reference(_)));
                    let condition = Self::condition(&Set::Intersect(filters), "p")?;
                    let named_test = if named { " AND p.named" } else { "" };
                    let relation = references[0];
                    self.definitions.push(format!("{name}(file,node,atom) AS MATERIALIZED (SELECT p.file,p.id,relation_leaf_atom(p.file,p.id,p.atom) FROM r{relation} k CROSS JOIN node p ON p.file=k.file AND p.id=k.node WHERE {condition}{named_test})"));
                    self.leaves.push((set, named, name.clone()));
                    name
                };
                conditions.push(format!("NOT EXISTS (SELECT 1 FROM {name} p WHERE p.file=a.file AND p.atom=a.atom AND p.node>a.node AND p.node<=n.\"last\")"));
            } else {
                paths.push(path.clone());
            }
        }
        remaining.states[0] = paths;
        self.fold_leaf_edges(&mut remaining);
        if !remaining.states[0].is_empty() {
            let p = self.probe(&remaining, source)?;
            conditions.push(format!("NOT EXISTS (SELECT 1 FROM {p} p WHERE p.file=a.file AND p.origin=a.node AND p.atom=a.atom)"));
        }
        Ok(conditions)
    }
    fn probe(&mut self, probe: &Probe, source: &str) -> Result<String, String> {
        if let Some((_, _, name)) = self
            .probes
            .iter()
            .find(|(key, input, _)| key == probe && input == source)
        {
            return Ok(name.clone());
        }
        let name = self.probe_definition(probe, source)?;
        self.probes
            .push((probe.clone(), source.to_owned(), name.clone()));
        Ok(name)
    }
    fn probe_definition(&mut self, probe: &Probe, source: &str) -> Result<String, String> {
        let name = self.name();
        let walk = format!("{name}_walk");
        let inline_entry = probe.states[0]
            .iter()
            .all(|p| matches!(p.end, End::State(next) if next != 0))
            && probe
                .states
                .iter()
                .flatten()
                .all(|p| p.end != End::State(0));
        let mut recursion = if inline_entry {
            Vec::new()
        } else {
            vec![format!("SELECT DISTINCT file,node,node,0 FROM {source}")]
        };
        if inline_entry {
            for path in &probe.states[0] {
                let End::State(next) = path.end else {
                    unreachable!()
                };
                let (from, condition, leaf) = Self::path(path)?;
                recursion.push(format!("SELECT DISTINCT w.file,w.node,{leaf}.id,{next} FROM {source} w CROSS JOIN {from} WHERE {condition}"));
            }
        }
        let mut answers = vec![];
        let mut transitions: Vec<(usize, usize, Vec<Path>)> = Vec::new();
        for (state, paths) in probe.states.iter().enumerate() {
            if inline_entry && state == 0 {
                continue;
            }
            for path in paths {
                let (from, condition, leaf) = Self::path(path)?;
                match path.end {
                End::State(next) => {
                    if let Some((_, _, paths)) = transitions.iter_mut().find(|(s, n, paths)| *s == state && *n == next && paths[0].edges == path.edges) { paths.push(path.clone()); }
                    else { transitions.push((state, next, vec![path.clone()])); }
                },
                End::Bind(named)=>answers.push(format!("SELECT w.file,w.origin,relation_leaf_atom({leaf}.file,{leaf}.id,{leaf}.atom) AS atom FROM {walk} w CROSS JOIN {from} WHERE w.state={state} AND {condition}{}",if named {format!(" AND {leaf}.named")} else {String::new()})),
            }
            }
        }
        for (state, next, paths) in transitions {
            let (from, condition, leaf) = Self::paths(&paths)?;
            recursion.push(format!("SELECT w.file,w.origin,{leaf}.id,{next} FROM {walk} w CROSS JOIN {from} WHERE w.state={state} AND {condition}"));
        }
        self.definitions.push(format!(
            "{walk}(file,origin,node,state) AS MATERIALIZED ({})",
            recursion.join(" UNION ")
        ));
        self.definitions.push(format!(
            "{name}(file,origin,atom) AS MATERIALIZED ({})",
            answers.join(" UNION ")
        ));
        Ok(name)
    }
}

/// Propagate strict-ancestor facts through the answer graph. This removes tests
/// already proved by capture demand or an outward traversal, before emission.
fn prune_domains(
    answer: &mut Answer,
    program: &ir::Program,
    frontier: &ir::CaptureDemand,
    origins: usize,
) {
    use std::collections::BTreeSet;
    fn exact(set: &Set) -> Option<BTreeSet<u16>> {
        match set {
            Set::Kinds(values) => Some(values.iter().copied().collect()),
            Set::Union(items) => {
                let mut all = BTreeSet::new();
                for item in items {
                    all.extend(exact(item)?);
                }
                Some(all)
            }
            Set::Intersect(items) if items.len() == 1 => exact(&items[0]),
            _ => None,
        }
    }
    fn remove(set: &mut Set, regions: &BTreeSet<usize>) {
        match set {
            Set::Within(id) if regions.contains(id) => *set = Set::AllNodes,
            Set::Intersect(items) => {
                for item in items {
                    remove(item, regions);
                }
            }
            _ => {}
        }
    }
    fn visit(
        answer: &mut Answer,
        program: &ir::Program,
        frontier: &ir::CaptureDemand,
        origins: usize,
    ) -> (BTreeSet<usize>, Option<BTreeSet<u16>>) {
        match answer {
            Answer::Frontier => {
                let mut regions = BTreeSet::from([frontier.roots]);
                if let Set::SearchRoots { filter, .. } =
                    program.relations[frontier.roots].expression
                {
                    regions.insert(filter);
                }
                (
                    regions,
                    demand::kinds(&Set::Reference(origins), program, &mut BTreeSet::new()),
                )
            }
            Answer::Filter(child, filters, ..) => {
                let (regions, child_kinds) = visit(child, program, frontier, origins);
                for filter in filters.iter_mut() {
                    remove(filter, &regions);
                }
                let own = demand::kinds(
                    &Set::Intersect(filters.clone()),
                    program,
                    &mut BTreeSet::new(),
                );
                let kinds = match (own, child_kinds) {
                    (Some(a), Some(b)) => Some(a.intersection(&b).copied().collect()),
                    (a, b) => a.or(b),
                };
                (regions, kinds)
            }
            Answer::Expand(axis, child) => {
                let (mut regions, kinds) = visit(child, program, frontier, origins);
                if *axis != Axis::InclusiveDescendant
                    && let Some(kinds) = kinds
                {
                    for (id, relation) in program.relations.iter().enumerate() {
                        if relation.domain.is_none()
                            && relation.keys.is_none()
                            && exact(&relation.expression)
                                .is_some_and(|members| kinds.is_subset(&members))
                        {
                            regions.insert(id);
                        }
                    }
                }
                (regions, None)
            }
            Answer::Any(items) => {
                let mut branches = items
                    .iter_mut()
                    .map(|a| visit(a, program, frontier, origins));
                let Some((mut regions, mut kinds)) = branches.next() else {
                    return (BTreeSet::new(), Some(BTreeSet::new()));
                };
                for (other, other_kinds) in branches {
                    regions = regions.intersection(&other).copied().collect();
                    kinds = kinds.zip(other_kinds).map(|(mut a, b)| {
                        a.extend(b);
                        a
                    });
                }
                (regions, kinds)
            }
        }
    }
    visit(answer, program, frontier, origins);
}

fn normalize_answer(answer: Answer) -> Answer {
    match answer {
        Answer::Filter(child, mut filters, bind, overwrite, mut probes) => {
            filters.retain(|s| *s != Set::AllNodes);
            match normalize_answer(*child) {
                Answer::Filter(inner, mut prior, inner_bind, inner_overwrite, mut negatives) => {
                    prior.append(&mut filters);
                    negatives.append(&mut probes);
                    normalize_answer(Answer::Filter(
                        inner,
                        prior,
                        bind || inner_bind,
                        overwrite || inner_overwrite,
                        negatives,
                    ))
                }
                child if filters.is_empty() && !bind && !overwrite && probes.is_empty() => child,
                child => Answer::Filter(Box::new(child), filters, bind, overwrite, probes),
            }
        }
        Answer::Any(items) => Answer::Any(
            items
                .into_iter()
                .map(normalize_answer)
                .flat_map(|a| match a {
                    Answer::Any(items) => items,
                    other => vec![other],
                })
                .collect(),
        ),
        Answer::Expand(axis, child) => Answer::Expand(axis, Box::new(normalize_answer(*child))),
        other => other,
    }
}

/// Capture-dependent negation can use a value anti-join only when its input value is definite.
fn inputs(
    rule: &AstMatcher,
    bound: bool,
    active: &mut HashSet<(usize, usize)>,
) -> Result<bool, String> {
    if witness::pure(rule) {
        return Ok(bound);
    }
    match rule {
        AstMatcher::Pattern(_) => Ok(true),
        AstMatcher::All(items) => {
            let mut b = bound;
            for item in items {
                b = inputs(item, b, active)?;
            }
            Ok(b)
        }
        AstMatcher::Any(items) => {
            let mut definite = true;
            for item in items {
                definite &= inputs(item, bound, active)?;
            }
            Ok(bound || definite)
        }
        AstMatcher::Not(_) => {
            if !bound {
                return Err(
                    "ordered answers: negative value probe requires a bound input capture".into(),
                );
            }
            Ok(bound)
        }
        AstMatcher::Inside(body, _) | AstMatcher::Has(body, _) => inputs(body, bound, active),
        AstMatcher::Util(u) => {
            let key = (u.table.as_ptr() as usize, u.index);
            if !active.insert(key) {
                return Err("ordered answers: recursive outward capture is not admitted".into());
            }
            let result = u
                .with_target(|r| inputs(r, bound, active))
                .ok_or("ordered answers: missing utility")?;
            active.remove(&key);
            result
        }
        _ => Err("ordered answers: unsupported input capture effect".into()),
    }
}

pub(super) fn compile(grammar: &GrammarMatcher) -> Result<Compiled, String> {
    inputs(&grammar.matcher, false, &mut HashSet::new())?;
    let mut searches = witness::discover(grammar)?;
    if searches.len() != 1 {
        return Err(
            "ordered answers: exactly one independently bound frontier is currently admitted"
                .into(),
        );
    }
    let search = searches.remove(0);
    let (lower, frontier, origins) = witness::lower_frontier(grammar, &search)?;
    let mut origin_predicates = vec![Set::Reference(origins)];
    conjuncts(
        &lower.program.relations[origins].expression,
        &mut origin_predicates,
    );
    let mut builder = Builder {
        lower,
        search: &search,
        active: HashSet::new(),
        answers: HashMap::new(),
        origin_predicates,
    };
    let mut answer = builder.answer(&grammar.matcher)?;
    // Validate every lowered utility before inlining it, including predicates not yet linked
    // into the frontier root. The final root will retain only actual emitted dependencies.
    let frontier_outputs = builder.lower.program.relations[0].expression.clone();
    builder.lower.program.relations[0].expression = Set::Union(
        (1..builder.lower.program.relations.len())
            .map(Set::Reference)
            .collect(),
    );
    physical::plan(&builder.lower.program)?;
    builder.lower.program.relations[0].expression = frontier_outputs.clone();
    demand::apply(&mut builder.lower.program);
    rewrite(
        &mut answer,
        &mut builder.lower,
        frontier.roots,
        &mut HashMap::new(),
    );
    let mut program = builder.lower.program;
    prune_domains(&mut answer, &program, &frontier, origins);
    answer = normalize_answer(answer);
    fn dependencies(answer: &Answer, out: &mut Vec<usize>) {
        match answer {
            Answer::Frontier => {}
            Answer::Filter(child, filters, _, _, probes) => {
                dependencies(child, out);
                for set in filters {
                    ir::references(set, out);
                }
                for probe in probes {
                    for path in probe.states.iter().flatten() {
                        for set in &path.filters {
                            ir::references(set, out);
                        }
                    }
                }
            }
            Answer::Any(items) => {
                for item in items {
                    dependencies(item, out);
                }
            }
            Answer::Expand(_, child) => dependencies(child, out),
        }
    }
    let mut required = Vec::new();
    ir::references(&frontier_outputs, &mut required);
    dependencies(&answer, &mut required);
    required.sort_unstable();
    required.dedup();
    program.relations[0].expression =
        Set::Union(required.into_iter().map(Set::Reference).collect());
    let planned = physical::plan(&program)?;
    // Only a binding on the final root proves that its value must occur in the
    // origin subtree. Bindings below an expansion refer to a different node.
    let used_kinds = if let Answer::Filter(_, filters, true, _, _) = &answer {
        demand::kinds(
            &Set::Intersect(filters.clone()),
            &program,
            &mut Default::default(),
        )
        .map(|k| k.into_iter().collect::<Vec<_>>())
    } else {
        None
    };
    let mut sql = Sql {
        program: &program,
        definitions: emit::capture_definitions(
            &program,
            &planned,
            &frontier,
            used_kinds.as_deref(),
        )?,
        next: 0,
        shared: vec![],
        probes: vec![],
        leaves: vec![],
    };
    let output = sql.answer(&answer)?;
    Ok(Compiled {
        sql: format!(
            "WITH RECURSIVE\n{}\nSELECT 0,file,node,json_array(witness) FROM {output} ORDER BY file,node",
            sql.definitions.join(",\n")
        ),
        terminals: program.terminals,
        relations: serde_json::json!({"scope":"Complete ordered single-leaf answer plan","capture":search.capture,"output":output,"usedValueKinds":used_kinds,"roots":frontier.roots,"demandDecisions":program.demand_decisions,"nativeComposition":false,"limitations":["Native work and stack/depth budgets remain outside this test-only kernel.","One bound frontier; positive recursive capture components and composite equality keys are not admitted."]}),
    })
}

#[test]
fn reference_shape_inclusive_descent_preserves_overlapping_branch_priority() {
    // Both branches match the root, but they publish different witnesses. The
    // descendant source chooses the rebound witness; direct order can disagree.
    let db = rusqlite::Connection::open_in_memory().unwrap();
    db.execute_batch("CREATE TABLE node(file,id,parent,kind,last); INSERT INTO node VALUES(0,1,NULL,1,2),(0,2,1,1,2);").unwrap();
    let rebound = Answer::Filter(Box::new(Answer::Frontier), vec![], false, true, vec![]);
    let base = Answer::Any(vec![rebound.clone(), Answer::Frontier]);
    for reversed in [false, true] {
        let mut direct = vec![rebound.clone(), Answer::Frontier];
        if reversed {
            direct.reverse();
        }
        direct.push(Answer::Expand(Axis::Descendant, Box::new(base.clone())));
        let answer = Answer::Filter(
            Box::new(Answer::Any(direct)),
            vec![Set::Kinds(vec![1])],
            false,
            false,
            vec![],
        );
        let program = ir::Program::default();
        let mut sql = Sql {
            program: &program,
            definitions: vec!["capture_demand(file,node,atom,witness) AS (VALUES(0,1,7,9))".into()],
            next: 0,
            shared: vec![],
            probes: vec![],
            leaves: vec![],
        };
        let result = sql.answer(&answer).unwrap();
        let query = format!(
            "WITH {} SELECT node,witness FROM {result} ORDER BY node",
            sql.definitions.join(",")
        );
        let actual: Vec<(i64, i64)> = db
            .prepare(&query)
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            actual,
            vec![(1, if reversed { 9 } else { 1 }), (2, 1)],
            "{query}"
        );
        assert_eq!(query.contains("n.id>=b.id"), !reversed);
    }
}
