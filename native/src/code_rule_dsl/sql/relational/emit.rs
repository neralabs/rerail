use super::{
    ir::{Edge, Program, Set},
    physical::{self, Arm, Predicate, RelationPlan},
};

#[derive(Default)]
struct PlanningDecisions {
    anchors: Vec<super::planner_stats::AnchorDecision>,
    edges: Vec<super::planner_stats::EdgeDecision>,
}
type AnchorDecisions = std::rc::Rc<std::cell::RefCell<PlanningDecisions>>;

#[derive(Default)]
struct Emitter {
    aliases: usize,
    bound_regions: bool,
    domains: std::collections::HashMap<usize, usize>,
    component: Option<(usize, Vec<usize>)>,
    stats: Option<super::planner_stats::PlannerStats>,
    decisions: AnchorDecisions,
}

pub(super) fn emit(program: &Program, plan: &[RelationPlan]) -> Result<String, String> {
    Ok(format!(
        "WITH RECURSIVE\n{}\nSELECT 0,file,node,json_array() FROM r0 ORDER BY file,node",
        definitions(program, plan)?.join(",\n")
    ))
}

/// Statistics affect only pure join anchors. Default compilation remains independent of data.
pub(super) fn emit_with_stats(
    program: &Program,
    plan: &[RelationPlan],
    stats: &super::planner_stats::PlannerStats,
) -> Result<(String, Vec<super::planner_stats::AnchorDecision>), String> {
    let (sql, anchors, _) = emit_with_stats_detailed(program, plan, stats)?;
    Ok((sql, anchors))
}

type PlannedSql = (
    String,
    Vec<super::planner_stats::AnchorDecision>,
    Vec<super::planner_stats::EdgeDecision>,
);

pub(super) fn emit_with_stats_detailed(
    program: &Program,
    plan: &[RelationPlan],
    stats: &super::planner_stats::PlannerStats,
) -> Result<PlannedSql, String> {
    let decisions = std::rc::Rc::new(std::cell::RefCell::new(PlanningDecisions::default()));
    let definitions = definitions_for_stats(
        program,
        plan,
        true,
        &[],
        Some(stats),
        Some(decisions.clone()),
    )?;
    let decisions = decisions.borrow();
    Ok((
        format!(
            "WITH RECURSIVE\n{}\nSELECT 0,file,node,json_array() FROM r0 ORDER BY file,node",
            definitions.join(",\n")
        ),
        decisions.anchors.clone(),
        decisions.edges.clone(),
    ))
}

pub(super) fn definitions(program: &Program, plan: &[RelationPlan]) -> Result<Vec<String>, String> {
    definitions_for(program, plan, true, &[])
}

fn definitions_for(
    program: &Program,
    plan: &[RelationPlan],
    project_root: bool,
    inline_outputs: &[usize],
) -> Result<Vec<String>, String> {
    definitions_for_stats(program, plan, project_root, inline_outputs, None, None)
}

fn definitions_for_stats(
    program: &Program,
    plan: &[RelationPlan],
    project_root: bool,
    inline_outputs: &[usize],
    stats: Option<&super::planner_stats::PlannerStats>,
    decisions: Option<AnchorDecisions>,
) -> Result<Vec<String>, String> {
    let mut emitter = Emitter {
        stats: stats.cloned(),
        decisions: decisions.unwrap_or_default(),
        domains: program
            .relations
            .iter()
            .enumerate()
            .filter_map(|(id, r)| r.domain.map(|domain| (id, domain)))
            .collect(),
        ..Emitter::default()
    };
    let storage =
        stats.map(|stats| super::planner_stats::materialization_decisions(program, plan, stats));
    let mut relations = Vec::new();
    let active = emitted_relations(program, project_root, inline_outputs);
    let mut regions = std::collections::BTreeSet::new();
    fn domains(set: &Set, out: &mut std::collections::BTreeSet<usize>) {
        match set {
            Set::Within(id) => {
                out.insert(*id);
            }
            Set::Intersect(sets) | Set::Union(sets) => {
                for set in sets {
                    domains(set, out);
                }
            }
            Set::Complement(set) | Set::Preimage(_, set) => domains(set, out),
            _ => {}
        }
    }
    for &id in &active {
        domains(&program.relations[id].expression, &mut regions);
        regions.extend(program.relations[id].domain);
        if let Some(keys) = &program.relations[id].keys {
            let roots = keys
                .root_kinds
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(",");
            let recursive = if keys.parent_kinds.is_empty() {
                String::new()
            } else {
                let parents = keys
                    .parent_kinds
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                format!(
                    " UNION SELECT p.file,p.id FROM needed{id} k CROSS JOIN node n ON n.file=k.file AND n.id=k.node CROSS JOIN node p ON p.file=n.file AND p.id=n.parent WHERE p.kind IN ({parents})"
                )
            };
            relations.push(format!("needed{id}(file,node) AS MATERIALIZED (SELECT file,id FROM node WHERE kind IN ({roots}){recursive})"));
        }
    }
    for id in regions {
        relations.push(format!("bounds{id}(file,node,last) AS MATERIALIZED (SELECT k.file,k.node,n.\"last\" FROM r{id} k CROSS JOIN node n ON n.file=k.file AND n.id=k.node),\nregions{id}(file,node,last) AS MATERIALIZED (SELECT b.file,b.node,b.last FROM bounds{id} b WHERE NOT EXISTS (SELECT 1 FROM bounds{id} outer_region WHERE outer_region.file=b.file AND outer_region.node<b.node AND outer_region.last>=b.node))"));
    }
    for (id, planned) in plan
        .iter()
        .enumerate()
        .filter(|(id, _)| active.contains(id) && (project_root || *id != 0))
    {
        if let RelationPlan::Component {
            leader, members, ..
        } = planned
        {
            if id == *leader {
                emitter.component = Some((*leader, members.clone()));
                let mut seeds = Vec::new();
                let mut steps = Vec::new();
                for &member in members {
                    let RelationPlan::Component { arms, .. } = &plan[member] else {
                        unreachable!()
                    };
                    for arm in arms {
                        let query = emitter.arm(arm, Some(member), None, false)?;
                        if arm.references.iter().any(|(_, r)| members.contains(r)) {
                            steps.push(query);
                        } else {
                            seeds.push(query);
                        }
                    }
                }
                // Seedless positive closures are empty. Keep a nonrecursive SELECT so SQLite
                // accepts the CTE; do not invent an answer or iterate through native callbacks.
                if seeds.is_empty() {
                    seeds.push("SELECT 0,0,0 WHERE 0".into());
                }
                seeds.extend(steps);
                relations.push(format!(
                    "component{leader}(state,file,node) AS MATERIALIZED (\n{}\n)",
                    seeds.join("\nUNION\n")
                ));
                emitter.component = None;
            }
            relations.push(format!("-- {}\nr{id}(file,node) AS (SELECT file,node FROM component{leader} WHERE state={id})",program.relations[id].source));
            continue;
        }
        if let RelationPlan::SearchRoots {
            origins,
            filter,
            stop,
        } = planned
        {
            let source = if let Some(kinds) =
                super::demand::kinds(&Set::Reference(*origins), program, &mut Default::default())
            {
                let kinds = kinds
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                let predicate = condition(
                    &super::demand::inline(&Set::Reference(*origins), program),
                    "n",
                )?;
                // Every successful origin has a strict matching ancestor. Restrict seed
                // production by indexed subtree ranges; retain the exact stopped walk.
                let ancestor = condition(
                    &super::demand::inline(&Set::Reference(*filter), program),
                    "a",
                )?;
                relations.push(format!("search{id}(file,node) AS MATERIALIZED (SELECT DISTINCT n.file,n.id FROM node a CROSS JOIN node n ON n.file=a.file AND n.id>a.id AND n.id<=a.\"last\" WHERE {ancestor} AND n.kind IN ({kinds}) AND {predicate})"));
                format!("search{id}")
            } else {
                format!("r{origins}")
            };
            let stopped = stop
                .map(|stop| {
                    condition(&super::demand::inline(&Set::Reference(stop), program), "n")
                        .map(|p| format!(" AND NOT ({p})"))
                })
                .transpose()?
                .unwrap_or_default();
            let predicate = condition(
                &super::demand::inline(&Set::Reference(*filter), program),
                "f",
            )?;
            relations.push(format!("walk{id}(file,origin,node,distance) AS (SELECT n.file,k.node,n.parent,1 FROM {source} k CROSS JOIN node n ON n.file=k.file AND n.id=k.node WHERE n.parent IS NOT NULL UNION ALL SELECT w.file,w.origin,n.parent,w.distance+1 FROM walk{id} w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE n.parent IS NOT NULL{stopped}),\npairs{id}(file,origin,node,distance) AS MATERIALIZED (SELECT w.* FROM walk{id} w CROSS JOIN node f ON f.file=w.file AND f.id=w.node WHERE {predicate}),\nr{id}(file,node) AS MATERIALIZED (SELECT DISTINCT file,node FROM pairs{id})"));
            continue;
        }
        let RelationPlan::Joins(arms) = planned else {
            let RelationPlan::Ancestors { seeds, stop } = planned else {
                unreachable!()
            };
            let stopped=stop.map_or(String::new(),|stop|format!(" AND NOT EXISTS (SELECT 1 FROM r{stop} s WHERE s.file=w.file AND s.node=w.node)"));
            relations.push(format!("-- {}\nr{id}(file,node) AS MATERIALIZED (SELECT n.file,n.parent FROM r{seeds} seed CROSS JOIN node n ON n.file=seed.file AND n.id=seed.node WHERE n.parent IS NOT NULL UNION SELECT n.file,n.parent FROM r{id} w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE n.parent IS NOT NULL{stopped})",program.relations[id].source));
            continue;
        };
        let seeds = compact_seeds(arms, id, program)?;
        let mut selects = arms
            .iter()
            .enumerate()
            .filter(|(index, _)| {
                seeds
                    .as_ref()
                    .is_none_or(|(indexes, _)| !indexes.contains(index))
            })
            .map(|(_, arm)| arm)
            .map(|arm| {
                let mut arm = arm.clone();
                if program.relations[id].keys.is_some() {
                    arm.predicates.push(Predicate::Keys(0, id));
                }
                if !arm.references.iter().any(|(_, r)| *r == id)
                    && let Some(domain) = program.relations[id].domain
                    && !arm.references.iter().any(|(node, reference)| {
                        *node == 0 && program.relations[*reference].domain == Some(domain)
                    })
                {
                    arm.predicates.push(Predicate::Region(0, domain));
                }
                emitter.arm(&arm, Some(id), None, arms.len() == 1)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if let Some((_, query)) = seeds {
            selects.insert(0, query);
        }
        if !arms.is_empty()
            && arms
                .iter()
                .all(|arm| arm.references.iter().any(|(_, r)| *r == id))
        {
            selects.insert(0, "SELECT 0,0 WHERE 0".into());
        }
        let body = if selects.is_empty() {
            "SELECT file,id FROM node WHERE 0".into()
        } else {
            selects.join("\nUNION\n")
        };
        let alias = arms.len() == 1
            && arms[0].nodes == 1
            && arms[0].predicates.is_empty()
            && arms[0].references.len() == 1
            && program.relations[id].keys.is_none()
            && program.relations[id].domain.is_none();
        let stored = if id != 0
            && !alias
            && storage
                .as_ref()
                .is_none_or(|choices| choices[id].materialized)
        {
            "MATERIALIZED "
        } else {
            ""
        };
        relations.push(format!(
            "-- {}\nr{id}(file,node) AS {stored}(\n{body}\n)",
            program.relations[id].source
        ));
    }
    Ok(relations)
}

/// Track dependencies of the selected physical shapes. Inlined predicates do
/// not require a stored answer set. Other consumers still retain that set.
fn emitted_relations(
    program: &Program,
    project_root: bool,
    inline_outputs: &[usize],
) -> std::collections::BTreeSet<usize> {
    fn refs(program: &Program, id: usize, todo: &mut Vec<usize>) {
        super::ir::references(&super::demand::inline(&Set::Reference(id), program), todo);
    }
    let mut todo = Vec::new();
    if project_root {
        todo.push(0);
    } else {
        let mut outputs = Vec::new();
        super::ir::references(&program.relations[0].expression, &mut outputs);
        for id in outputs {
            if inline_outputs.contains(&id) {
                refs(program, id, &mut todo);
            } else {
                todo.push(id);
            }
        }
    }
    let mut active = std::collections::BTreeSet::new();
    while let Some(id) = todo.pop() {
        if !active.insert(id) {
            continue;
        }
        let relation = &program.relations[id];
        if let Set::SearchRoots {
            origins,
            filter,
            stop,
        } = &relation.expression
        {
            if super::demand::kinds(&Set::Reference(*origins), program, &mut Default::default())
                .is_some()
            {
                refs(program, *origins, &mut todo);
            } else {
                todo.push(*origins);
            }
            refs(program, *filter, &mut todo);
            if let Some(stop) = stop {
                refs(program, *stop, &mut todo);
            }
        } else {
            super::ir::references(&relation.expression, &mut todo);
        }
        todo.extend(relation.domain);
    }
    active
}

/// A demanded seed set has one root source. Compose its alternatives around that
/// source, sharing functional witnesses instead of repeating a complete join arm.
fn compact_seeds(
    arms: &[Arm],
    id: usize,
    program: &Program,
) -> Result<Option<(Vec<usize>, String)>, String> {
    let relation = &program.relations[id];
    if relation.keys.is_none() && relation.domain.is_none() {
        return Ok(None);
    }
    let indexes: Vec<_> = arms
        .iter()
        .enumerate()
        .filter(|(_, arm)| arm.references.is_empty())
        .map(|(i, _)| i)
        .collect();
    if indexes.len() < 2 {
        return Ok(None);
    }
    let mut bound = super::bound::Bound::default();
    let (source, guard) = if relation.keys.is_some() {
        (
            format!("needed{id} k CROSS JOIN node seed ON seed.file=k.file AND seed.id=k.node"),
            "1".to_string(),
        )
    } else {
        let mut kinds = std::collections::BTreeSet::new();
        for &index in &indexes {
            let arm = &arms[index];
            let Some((_, _, parent)) = arm
                .edges
                .iter()
                .find(|(from, edge, _)| *from == 0 && *edge == Edge::Parent)
            else {
                return Ok(None);
            };
            let Some(values) = arm.predicates.iter().find_map(|p| match p {
                Predicate::Kinds(at, values) if at == parent => Some(values),
                _ => None,
            }) else {
                return Ok(None);
            };
            kinds.extend(values.iter().copied());
        }
        bound.parent("seed", "parent");
        let region = relation.domain.unwrap();
        (
            format!(
                "regions{region} d CROSS JOIN node parent ON parent.file=d.file AND parent.id>=d.node AND parent.id<=d.last CROSS JOIN node seed ON seed.file=parent.file AND seed.parent=parent.id"
            ),
            format!(
                "parent.kind IN ({})",
                kinds
                    .iter()
                    .map(u16::to_string)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        )
    };
    let set = Set::Union(
        indexes
            .iter()
            .map(|&i| super::bound::arm_set(&arms[i], 0))
            .collect(),
    );
    let set = if relation.keys.is_some() {
        relation.domain.map_or(set.clone(), |domain| {
            Set::Intersect(vec![set, Set::Within(domain)])
        })
    } else {
        set
    };
    let condition = bound.condition(&set, "seed")?;
    // The enclosing UNION already deduplicates seeds when other arms remain.
    let distinct = if indexes.len() == arms.len() {
        "DISTINCT "
    } else {
        ""
    };
    Ok(Some((
        indexes,
        format!(
            "SELECT {distinct}seed.file,seed.id FROM {source}{} WHERE {guard} AND {condition}",
            bound.joins.join("")
        ),
    )))
}

/// Keep the bound expression separate from query formatting. A witness-free arm
/// is already a total predicate on an existing node; it needs no scalar query.
struct ArmSql {
    projection: String,
    sources: Vec<String>,
    conditions: Vec<String>,
    unit: Option<usize>,
}
impl ArmSql {
    fn query(&self) -> String {
        let from = if self.sources.is_empty() {
            String::new()
        } else if let Some(serial) = self.unit {
            format!(
                " FROM (SELECT 1) AS unit{serial} CROSS JOIN {}",
                self.sources.join(" CROSS JOIN ")
            )
        } else {
            format!(" FROM {}", self.sources.join(" CROSS JOIN "))
        };
        let filter = if self.conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", self.conditions.join(" AND "))
        };
        format!("SELECT {}{from}{filter}", self.projection)
    }
    fn predicate(&self) -> String {
        if !self.sources.is_empty() {
            return format!("EXISTS ({})", self.query());
        }
        match self.conditions.as_slice() {
            [] => "1".into(),
            [condition] => condition.clone(),
            conditions => format!("({})", conditions.join(" AND ")),
        }
    }
}

impl Emitter {
    fn recursive(&self, relation: usize, owner: Option<usize>) -> bool {
        self.component
            .as_ref()
            .map_or(Some(relation) == owner, |(_, members)| {
                members.contains(&relation)
            })
    }

    fn condition(&mut self, set: &Set, root: &str) -> Result<String, String> {
        let arms = physical::expand(set)?;
        let predicates = arms
            .iter()
            .map(|arm| Ok(self.arm_parts(arm, None, Some(root), false)?.predicate()))
            .collect::<Result<Vec<_>, String>>()?;
        Ok(match predicates.as_slice() {
            [] => "0".into(),
            [predicate] => predicate.clone(),
            predicates => format!("({})", predicates.join(" OR ")),
        })
    }

    fn absent(&mut self, set: &Set, root: &str) -> Result<String, String> {
        Ok(format!("NOT ({})", self.condition(set, root)?))
    }

    fn arm(
        &mut self,
        arm: &Arm,
        owner: Option<usize>,
        bound: Option<&str>,
        distinct: bool,
    ) -> Result<String, String> {
        if bound.is_none()
            && self.component.is_none()
            && arm.nodes == 1
            && arm.predicates.is_empty()
            && let [(0, relation)] = arm.references.as_slice()
        {
            // A set reference already has the required key and multiplicity. No AST lookup.
            return Ok(format!("SELECT file,node FROM r{relation}"));
        }
        Ok(self.arm_parts(arm, owner, bound, distinct)?.query())
    }

    fn arm_parts(
        &mut self,
        arm: &Arm,
        owner: Option<usize>,
        bound: Option<&str>,
        distinct: bool,
    ) -> Result<ArmSql, String> {
        let serial = self.aliases;
        self.aliases += 1;
        let mut names: Vec<_> = (0..arm.nodes).map(|n| format!("a{serial}n{n}")).collect();
        let mut known = vec![false; arm.nodes];
        let mut sources = Vec::new();
        let mut conditions = Vec::new();
        let mut consumed = None;
        let mut consumed_region = None;
        let mut consumed_keys = None;
        let requested = arm
            .predicates
            .iter()
            .any(|p| matches!(p, Predicate::Region(..) | Predicate::Keys(..)));
        if let Some(root) = bound {
            names[0] = root.to_owned();
            known[0] = true;
        } else if let Some((index, &(node, relation))) = arm
            .references
            .iter()
            .enumerate()
            .filter(|(_, (node, r))| !requested || self.recursive(*r, owner) || arm.predicates.iter().any(|p| matches!(p,Predicate::Region(at,domain) if at==node && self.domains.get(r)==Some(domain))))
            .min_by_key(|(_, (_, r))| if self.recursive(*r, owner) { 0 } else { 1 })
        {
            let name = &names[node];
            if let Some((leader, members)) = &self.component
                && members.contains(&relation)
            {
                sources.push(format!("component{leader} AS seed{serial}"));
                conditions.push(format!("seed{serial}.state={relation}"));
            } else {
                sources.push(format!("r{relation} AS seed{serial}"));
            }
            sources.push(format!(
                "node AS {name} ON {name}.file=seed{serial}.file AND {name}.id=seed{serial}.node"
            ));
            known[node] = true;
            consumed = Some(index);
            consumed_region=arm.predicates.iter().find_map(|p|match p {
                Predicate::Region(at,domain) if *at==node && self.domains.get(&relation)==Some(domain)=>Some((*at,*domain)),
                _=>None,
            });
        } else if let Some((node, producer)) = arm.predicates.iter().find_map(|p| match p {
            Predicate::Keys(node, producer) => Some((*node, *producer)),
            _ => None,
        }) {
            let name = &names[node];
            sources.push(format!("needed{producer} AS keys{serial}"));
            sources.push(format!(
                "node AS {name} ON {name}.file=keys{serial}.file AND {name}.id=keys{serial}.node"
            ));
            known[node] = true;
            consumed_keys = Some((node, producer));
        } else if let Some((node, roots)) = arm.predicates.iter().find_map(|p| match p {
            Predicate::Region(node, roots) => Some((*node, *roots)),
            _ => None,
        }) {
            // For a strict descendant candidate, its parent lies inside the inclusive region.
            // Start at a kind-filtered parent when possible, then probe its children.
            let anchor = arm
                .edges
                .iter()
                .find_map(|(from, edge, to)| {
                    (*from == node
                        && *edge == Edge::Parent
                        && arm
                            .predicates
                            .iter()
                            .any(|p| matches!(p,Predicate::Kinds(at,_) if at==to)))
                    .then_some(*to)
                })
                .unwrap_or(node);
            let lower = if anchor == node { ">" } else { ">=" };
            let name = &names[anchor];
            sources.push(format!("regions{roots} AS domain{serial}"));
            sources.push(format!("node AS {name} ON {name}.file=domain{serial}.file AND {name}.id{lower}domain{serial}.node AND {name}.id<=domain{serial}.last"));
            known[anchor] = true;
            consumed_region = Some((node, roots));
        } else {
            // Begin at selective kind evidence rather than at the output root by default.
            let decision = self.stats.as_ref().and_then(|stats| stats.anchor(arm));
            if let Some(decision) = &decision { self.decisions.borrow_mut().anchors.push(decision.clone()); }
            let node = decision.map(|decision| decision.variable).unwrap_or_else(|| arm
                .predicates
                .iter()
                .filter_map(|p| match p {
                    Predicate::Kinds(n, k) => Some((*n, k.len())),
                    _ => None,
                })
                .min_by_key(|(_, size)| *size)
                .map_or(0, |(n, _)| n));
            sources.push(format!("node AS {}", names[node]));
            known[node] = true;
        }
        while known.iter().any(|k| !k) {
            let decision = self
                .stats
                .as_ref()
                .and_then(|stats| stats.incident_edge(arm, &known));
            if let Some(decision) = &decision {
                self.decisions.borrow_mut().edges.push(decision.clone());
            }
            let Some(&(from, edge, to)) = decision
                .as_ref()
                .map(|d| &arm.edges[d.edge_index])
                .or_else(|| {
                    arm.edges
                        .iter()
                        .find(|(from, _, to)| known[*from] != known[*to])
                })
            else {
                return Err("relational compiler: disconnected join graph".into());
            };
            let (new, old) = if known[from] { (to, from) } else { (from, to) };
            let (n, o) = (&names[new], &names[old]);
            let condition = match (edge, known[from]) {
                (Edge::Parent, true) | (Edge::Child, false) => format!("{n}.id={o}.parent"),
                (Edge::Parent, false) | (Edge::Child, true) => format!("{n}.parent={o}.id"),
                (Edge::Previous, true) | (Edge::Next, false) => format!("{n}.id={o}.prev"),
                (Edge::Previous, false) | (Edge::Next, true) => format!("{n}.id={o}.next"),
            };
            // CROSS JOIN fixes the measured probe direction; predicates still use vtable indexes.
            sources.push(format!("node AS {n} ON {n}.file={o}.file AND {condition}"));
            known[new] = true;
        }
        for (index, &(node, relation)) in arm.references.iter().enumerate() {
            if consumed == Some(index) {
                continue;
            }
            let n = &names[node];
            conditions.push(format!(
                "EXISTS (SELECT 1 FROM r{relation} k WHERE k.file={n}.file AND k.node={n}.id)"
            ));
        }
        for predicate in &arm.predicates {
            if matches!(predicate,Predicate::Region(node,roots) if consumed_region==Some((*node,*roots)))
                || matches!(predicate,Predicate::Keys(node,producer) if consumed_keys==Some((*node,*producer)))
            {
                continue;
            }
            conditions.push(match predicate {
                Predicate::Kinds(node, kinds) => format!(
                    "{}.kind IN ({})",
                    names[*node],
                    kinds
                        .iter()
                        .map(u16::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                ),
                Predicate::Nonempty(node) => format!("{}.\"end\">{}.start", names[*node], names[*node]),
                Predicate::Regex(node, term) => format!(
                    "node_regex({term},{}.file,{}.id)",
                    names[*node], names[*node]
                ),
                Predicate::Pattern(node,term)=>format!("relation_pattern({term},{}.file,{}.id)",names[*node],names[*node]),
                Predicate::Region(node,roots) if self.bound_regions => format!("EXISTS (SELECT 1 FROM r{roots} k CROSS JOIN node ancestor ON ancestor.file=k.file AND ancestor.id=k.node WHERE k.file={0}.file AND ancestor.id<{0}.id AND ancestor.\"last\">={0}.id)",names[*node]),
                Predicate::Region(node,roots)=>format!("EXISTS (SELECT 1 FROM regions{roots} domain WHERE domain.file={}.file AND {}.id>domain.node AND {}.id<=domain.last)",names[*node],names[*node],names[*node]),
                Predicate::Keys(node,producer)=>format!("EXISTS (SELECT 1 FROM needed{producer} k WHERE k.file={}.file AND k.node={}.id)",names[*node],names[*node]),
                Predicate::Absent(node, set) => self.absent(set, &names[*node])?,
            });
        }
        let projection = if bound.is_some() {
            "1".into()
        } else {
            let tag = if self.component.is_some() {
                format!("{},", owner.expect("component arm has an owner"))
            } else {
                String::new()
            };
            format!(
                "{}{tag}{}.file,{}.id",
                if distinct && arm.edges.iter().any(|(_, edge, _)| *edge == Edge::Child) {
                    "DISTINCT "
                } else {
                    ""
                },
                names[0],
                names[0]
            )
        };
        Ok(ArmSql {
            projection,
            sources,
            conditions,
            unit: bound.map(|_| serial),
        })
    }
}

pub(super) fn capture_demand(
    program: &Program,
    plan: &[RelationPlan],
    demand: &super::ir::CaptureDemand,
) -> Result<String, String> {
    let definitions = capture_definitions(program, plan, demand, None)?;
    Ok(format!(
        "WITH RECURSIVE\n{}\nSELECT file,node,atom,witness,ancestor,distance FROM capture_demand ORDER BY file,node,atom",
        definitions.join(",\n")
    ))
}

pub(super) fn condition(set: &Set, alias: &str) -> Result<String, String> {
    // Preserve expansion admission independently of physical sharing.
    physical::expand(set)?;
    let mut bound = super::bound::Bound::default();
    let condition = bound.condition(set, alias)?;
    if bound.joins.is_empty() {
        return Ok(condition);
    }
    Ok(format!(
        "EXISTS (SELECT 1 FROM (SELECT 1) AS bound_unit{} WHERE {condition})",
        bound.joins.join("")
    ))
}

pub(super) fn unshared_condition(set: &Set, alias: &str) -> Result<String, String> {
    let mut emitter = Emitter {
        bound_regions: true,
        ..Emitter::default()
    };
    emitter.condition(set, alias)
}

pub(super) fn capture_definitions(
    program: &Program,
    plan: &[RelationPlan],
    demand: &super::ir::CaptureDemand,
    used_kinds: Option<&[u16]>,
) -> Result<Vec<String>, String> {
    let (roots, targets, leaf_stop) = (demand.roots, demand.targets, demand.leaf_stop);
    let mut definitions = definitions_for(
        program,
        plan,
        false,
        &leaf_stop.into_iter().collect::<Vec<_>>(),
    )?;
    let named = if demand.named { " AND n.named" } else { "" };
    definitions.push(format!("binding_seeds(file,node,atom) AS MATERIALIZED (SELECT t.file,t.node,relation_leaf_atom(n.file,n.id,n.atom) FROM r{targets} t CROSS JOIN node n ON n.file=t.file AND n.id=t.node WHERE 1{named})"));
    let stopped = leaf_stop.map(|id| {
        condition(&super::demand::inline(&Set::Reference(id), program), "s").map(|predicate|
            format!(" WHERE NOT EXISTS (SELECT 1 FROM node s WHERE s.file=t.file AND s.id>a.id AND s.id<t.node AND s.\"last\">=t.node AND {predicate})"))
    }).transpose()?.unwrap_or_default();
    // A stopped descendant search accepts exactly the targets with no stopping
    // proper ancestor between target and root. The virtual table seeks ancestor
    // postings, and MIN retains the first successful preorder witness.
    definitions.push(format!("binding_choices(file,node,atom,witness) AS MATERIALIZED (SELECT k.file,k.node,t.atom,MIN(t.node) FROM r{roots} k CROSS JOIN node a ON a.file=k.file AND a.id=k.node CROSS JOIN binding_seeds t ON t.file=k.file AND t.node>a.id AND t.node<=a.\"last\"{stopped} GROUP BY k.file,k.node,t.atom)"));
    let choice_source = if let Some(kinds) = used_kinds {
        let kinds = kinds
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(",");
        // Output nodes only move down from the frontier. A same-root binding
        // therefore needs this value somewhere in that origin's inclusive subtree.
        // This necessary test is independent of ancestor and assignment witness choice.
        // Read the atom directly: this superset must not call fallible equality leaves.
        definitions.push(format!("used_values(file,origin,atom) AS MATERIALIZED (SELECT DISTINCT p.file,p.origin,n.atom FROM (SELECT DISTINCT file,origin FROM pairs{roots}) p CROSS JOIN node o ON o.file=p.file AND o.id=p.origin CROSS JOIN node n ON n.file=o.file AND n.id>=o.id AND n.id<=o.\"last\" WHERE n.kind IN ({kinds}) AND n.named)"));
        format!(
            "used_values u CROSS JOIN pairs{roots} p ON p.file=u.file AND p.origin=u.origin CROSS JOIN binding_choices b ON b.file=p.file AND b.node=p.node AND b.atom=u.atom"
        )
    } else {
        format!("pairs{roots} p JOIN binding_choices b ON b.file=p.file AND b.node=p.node")
    };
    definitions.push(format!("origin_choices AS (SELECT p.file,p.origin,b.atom,b.witness,p.node AS ancestor,p.distance,ROW_NUMBER() OVER (PARTITION BY p.file,p.origin,b.atom ORDER BY p.distance) AS choice FROM {choice_source})"));
    definitions.push("capture_demand AS MATERIALIZED (SELECT file,origin AS node,atom,witness,ancestor,distance FROM origin_choices WHERE choice=1)".into());
    Ok(definitions)
}

#[cfg(test)]
mod bound_predicate_tests {
    use super::*;

    #[test]
    fn bound_predicate_membership_witnesses_regions_and_recursive_references() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE node(file INTEGER NOT NULL,id INTEGER NOT NULL,parent INTEGER,prev INTEGER,next INTEGER,kind INTEGER NOT NULL,\"last\" INTEGER NOT NULL);
             INSERT INTO node VALUES (0,1,NULL,NULL,NULL,10,3),(0,2,1,NULL,3,20,2),(0,3,1,2,NULL,30,3),
                                     (1,1,NULL,NULL,NULL,20,2),(1,2,1,NULL,NULL,30,2);",
        ).unwrap();
        let pre = |edge, set| Set::Preimage(edge, Box::new(set));
        let not = |set| Set::Complement(Box::new(set));
        let all = vec![(0, 1), (0, 2), (0, 3), (1, 1), (1, 2)];
        let cases = vec![
            (Set::AllNodes, all.clone(), false),
            (Set::Empty, vec![], false),
            (Set::Union(vec![]), vec![], false),
            (Set::Intersect(vec![]), all.clone(), false),
            (Set::Kinds(vec![]), vec![], false),
            (Set::Reference(0), all.clone(), false),
            (Set::Reference(1), vec![(0, 1)], false),
            (
                not(Set::Reference(1)),
                vec![(0, 2), (0, 3), (1, 1), (1, 2)],
                false,
            ),
            (Set::Within(1), vec![(0, 2), (0, 3)], false),
            (not(Set::Within(1)), vec![(0, 1), (1, 1), (1, 2)], false),
            (
                Set::Union(vec![
                    Set::Empty,
                    Set::Kinds(vec![20]),
                    Set::Kinds(vec![20, 30]),
                ]),
                vec![(0, 2), (0, 3), (1, 1), (1, 2)],
                false,
            ),
            (
                Set::Intersect(vec![
                    Set::Union(vec![Set::Kinds(vec![10]), Set::Kinds(vec![30])]),
                    not(Set::Within(1)),
                ]),
                vec![(0, 1), (1, 2)],
                false,
            ),
            (
                pre(Edge::Parent, Set::Kinds(vec![10])),
                vec![(0, 2), (0, 3)],
                true,
            ),
            (
                not(pre(Edge::Child, Set::Kinds(vec![20]))),
                vec![(0, 2), (0, 3), (1, 1), (1, 2)],
                true,
            ),
            (pre(Edge::Next, Set::Kinds(vec![30])), vec![(0, 2)], true),
            (
                pre(Edge::Previous, Set::Kinds(vec![20])),
                vec![(0, 3)],
                true,
            ),
            (
                pre(
                    Edge::Child,
                    Set::Intersect(vec![Set::Reference(0), not(Set::Within(1))]),
                ),
                vec![(1, 1)],
                true,
            ),
            (
                pre(Edge::Child, pre(Edge::Parent, Set::Kinds(vec![20]))),
                vec![(1, 1)],
                true,
            ),
            (
                Set::Union(vec![
                    pre(Edge::Child, Set::Kinds(vec![20])),
                    pre(Edge::Child, Set::Kinds(vec![30])),
                ]),
                vec![(0, 1), (1, 1)],
                true,
            ),
        ];
        for (set, expected, witness) in cases {
            let expression = condition(&set, "n").unwrap();
            assert!(!expression.contains("SELECT 1 WHERE"), "{expression}");
            if witness {
                assert!(expression.contains("JOIN node"), "{expression}");
            }
            for negate in [false, true] {
                let predicate = if negate {
                    format!("NOT ({expression})")
                } else {
                    expression.clone()
                };
                let query = format!(
                    "WITH RECURSIVE r0(file,node) AS (SELECT file,id FROM node WHERE parent IS NULL UNION SELECT n.file,n.id FROM r0 r JOIN node n ON n.file=r.file AND n.parent=r.node), r1(file,node) AS (SELECT 0,1) SELECT n.file,n.id FROM node n WHERE {predicate} ORDER BY n.file,n.id"
                );
                let mut statement = db.prepare(&query).unwrap();
                let actual: Vec<(i64, i64)> = statement
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap();
                let expected = if negate {
                    all.iter()
                        .copied()
                        .filter(|key| !expected.contains(key))
                        .collect()
                } else {
                    expected.clone()
                };
                assert_eq!(actual, expected, "{query}");
            }
        }
    }
}
