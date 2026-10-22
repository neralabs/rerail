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
}
impl Sql<'_> {
    fn name(&mut self) -> String {
        let id = self.next;
        self.next += 1;
        format!("answer{id}")
    }
    fn condition(s: &Set, n: &str) -> Result<String, String> {
        match s {
            Set::AllNodes => Ok("1".into()),
            Set::Kinds(k) => Ok(format!(
                "{n}.kind IN ({})",
                k.iter().map(u16::to_string).collect::<Vec<_>>().join(",")
            )),
            Set::Intersect(items) if items.is_empty() => Ok("1".into()),
            Set::Intersect(items) => Ok(format!(
                "({})",
                items
                    .iter()
                    .map(|s| Self::condition(s, n))
                    .collect::<Result<Vec<_>, _>>()?
                    .join(" AND ")
            )),
            _ => emit::condition(s, n),
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
            Answer::Frontier => {
                let name = self.name();
                self.definitions.push(format!("{name}(file,node,atom,witness) AS MATERIALIZED (SELECT file,origin,atom,witness FROM capture_demand)"));
                Ok(name)
            }
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
                let (source, expanded) = if let Answer::Expand(axis, inner) = child.as_ref() {
                    (self.expand(*axis, inner, filters, *bind)?, true)
                } else if let Answer::Any(items) = child.as_ref() {
                    // These conditions depend on root and value, never the selected witness.
                    // They therefore commute with branch choice and reach indexed expansions.
                    let filtered = Answer::Any(
                        items
                            .iter()
                            .map(|a| {
                                Answer::Filter(
                                    Box::new(a.clone()),
                                    filters.clone(),
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
                if expanded && probes.is_empty() && !overwrite {
                    return Ok(source);
                }
                let name = self.name();
                let mut conditions = vec!["1".to_string()];
                if !expanded {
                    for s in filters {
                        conditions.push(Self::condition(s, "n")?);
                    }
                    if *bind {
                        conditions.push(
                            "n.named AND relation_leaf_atom(n.file,n.id,n.atom)=a.atom".into(),
                        );
                    }
                }
                for probe in probes {
                    let p = self.probe(probe, &source)?;
                    conditions.push(format!("NOT EXISTS (SELECT 1 FROM {p} p WHERE p.file=a.file AND p.origin=a.node AND p.atom=a.atom)"));
                }
                let witness = if *overwrite { "a.node" } else { "a.witness" };
                self.definitions.push(format!("{name}(file,node,atom,witness) AS MATERIALIZED (SELECT a.file,a.node,a.atom,{witness} FROM {source} a CROSS JOIN node n ON n.file=a.file AND n.id=a.node WHERE {})",conditions.join(" AND ")));
                Ok(name)
            }
            Answer::Any(items) => {
                let sources = items
                    .iter()
                    .map(|a| self.answer(a))
                    .collect::<Result<Vec<_>, _>>()?;
                let name = self.name();
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
        };
        let mut conditions = filters
            .iter()
            .map(|s| Self::condition(s, "n"))
            .collect::<Result<Vec<_>, _>>()?;
        let value = if bind {
            conditions.push("n.named AND relation_leaf_atom(n.file,n.id,n.atom)=a.atom".into());
            " CROSS JOIN node v ON v.file=a.file AND v.id=a.atom"
        } else {
            ""
        };
        let hash = if bind { " AND n.thash=v.thash" } else { "" };
        conditions.push("1".into());
        self.definitions.push(format!("{name}(file,node,atom,witness) AS MATERIALIZED (SELECT file,node,atom,witness FROM (SELECT a.file,n.id AS node,a.atom,a.witness,ROW_NUMBER() OVER (PARTITION BY a.file,n.id,a.atom ORDER BY a.node DESC) AS choice FROM {source} a CROSS JOIN node b ON b.file=a.file AND b.id=a.node{value} CROSS JOIN node n ON n.file=b.file AND {edge}{hash} WHERE {}) WHERE choice=1)",conditions.join(" AND ")));
        Ok(name)
    }
    fn path(path: &Path) -> Result<(String, String, String), String> {
        let mut from = "node n0 ON n0.file=w.file AND n0.id=w.node".to_string();
        let mut filters = vec![Self::condition(&path.filters[0], "n0")?];
        for (i, axis) in path.edges.iter().enumerate() {
            let j = i + 1;
            let edge = match axis {
                Axis::Child => format!("n{j}.parent=n{i}.id"),
                Axis::Descendant => format!("n{j}.id>n{i}.id AND n{j}.id<=n{i}.\"last\""),
            };
            from.push_str(&format!(
                " CROSS JOIN node n{j} ON n{j}.file=n{i}.file AND {edge}"
            ));
            filters.push(Self::condition(&path.filters[j], &format!("n{j}"))?);
        }
        Ok((
            from,
            filters.join(" AND "),
            format!("n{}", path.edges.len()),
        ))
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
        let mut recursion = vec![format!("SELECT DISTINCT file,node,node,0 FROM {source}")];
        let mut answers = vec![];
        for (state, paths) in probe.states.iter().enumerate() {
            for path in paths {
                let (from, condition, leaf) = Self::path(path)?;
                match path.end {
                End::State(next)=>recursion.push(format!("SELECT w.file,w.origin,{leaf}.id,{next} FROM {walk} w CROSS JOIN {from} WHERE w.state={state} AND {condition}")),
                End::Bind(named)=>answers.push(format!("SELECT w.file,w.origin,relation_leaf_atom({leaf}.file,{leaf}.id,{leaf}.atom) AS atom FROM {walk} w CROSS JOIN {from} WHERE w.state={state} AND {condition}{}",if named {format!(" AND {leaf}.named")} else {String::new()})),
            }
            }
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
    let mut origin_predicates = vec![];
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
    builder.lower.program.relations[0].expression = frontier_outputs;
    demand::apply(&mut builder.lower.program);
    rewrite(
        &mut answer,
        &mut builder.lower,
        frontier.roots,
        &mut HashMap::new(),
    );
    let program = builder.lower.program;
    let planned = physical::plan(&program)?;
    let mut sql = Sql {
        program: &program,
        definitions: emit::capture_definitions(&program, &planned, &frontier)?,
        next: 0,
        shared: vec![],
        probes: vec![],
    };
    let output = sql.answer(&answer)?;
    Ok(Compiled {
        sql: format!(
            "WITH RECURSIVE\n{}\nSELECT 0,file,node,json_array(witness) FROM {output} ORDER BY file,node",
            sql.definitions.join(",\n")
        ),
        terminals: program.terminals,
        relations: serde_json::json!({"scope":"Complete ordered single-leaf answer plan","capture":search.capture,"output":output,"roots":frontier.roots,"demandDecisions":program.demand_decisions,"nativeComposition":false,"limitations":["Native work and stack/depth budgets remain outside this test-only kernel.","One bound frontier; positive recursive capture components and composite equality keys are not admitted."]}),
    })
}
