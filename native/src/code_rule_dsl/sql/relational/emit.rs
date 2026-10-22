use super::{
    ir::{Edge, Program, Set},
    physical::{self, Arm, Predicate, RelationPlan},
};

#[derive(Default)]
struct Emitter {
    aliases: usize,
}

pub(super) fn emit(program: &Program, plan: &[RelationPlan]) -> Result<String, String> {
    Ok(format!(
        "WITH RECURSIVE\n{}\nSELECT 0,file,node,json_array() FROM r0 ORDER BY file,node",
        definitions(program, plan)?.join(",\n")
    ))
}

pub(super) fn definitions(program: &Program, plan: &[RelationPlan]) -> Result<Vec<String>, String> {
    let mut emitter = Emitter::default();
    let mut relations = Vec::new();
    let active = super::ir::reachable(program);
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
        .filter(|(id, _)| active.contains(id))
    {
        if let RelationPlan::SearchRoots {
            origins,
            filter,
            stop,
        } = planned
        {
            let stopped=stop.map_or(String::new(),|stop|format!(" AND NOT EXISTS (SELECT 1 FROM r{stop} s WHERE s.file=w.file AND s.node=w.node)"));
            relations.push(format!("walk{id}(file,origin,node,distance) AS MATERIALIZED (SELECT n.file,k.node,n.parent,1 FROM r{origins} k CROSS JOIN node n ON n.file=k.file AND n.id=k.node WHERE n.parent IS NOT NULL UNION ALL SELECT w.file,w.origin,n.parent,w.distance+1 FROM walk{id} w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE n.parent IS NOT NULL{stopped}),\npairs{id}(file,origin,node,distance) AS MATERIALIZED (SELECT w.* FROM walk{id} w JOIN r{filter} f ON f.file=w.file AND f.node=w.node),\nr{id}(file,node) AS MATERIALIZED (SELECT DISTINCT file,node FROM pairs{id})"));
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
        let selects = arms
            .iter()
            .map(|arm| {
                let mut arm = arm.clone();
                if program.relations[id].keys.is_some() {
                    arm.predicates.push(Predicate::Keys(0, id));
                }
                if !arm.references.iter().any(|(_, r)| *r == id)
                    && let Some(domain) = program.relations[id].domain
                {
                    arm.predicates.push(Predicate::Region(0, domain));
                }
                emitter.arm(&arm, Some(id), None, arms.len() == 1)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let body = if selects.is_empty() {
            "SELECT file,id FROM node WHERE 0".into()
        } else {
            selects.join("\nUNION\n")
        };
        let stored = if id != 0 && !matches!(program.relations[id].expression, Set::Reference(_)) {
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

impl Emitter {
    fn absent(&mut self, set: &Set, root: &str) -> Result<String, String> {
        let arms = physical::expand(set)?;
        let queries = arms
            .iter()
            .map(|a| self.arm(a, None, Some(root), false))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(if queries.is_empty() {
            "1".into()
        } else {
            format!("NOT EXISTS ({})", queries.join(" UNION "))
        })
    }

    fn arm(
        &mut self,
        arm: &Arm,
        owner: Option<usize>,
        bound: Option<&str>,
        distinct: bool,
    ) -> Result<String, String> {
        if bound.is_none()
            && arm.nodes == 1
            && arm.predicates.is_empty()
            && let [(0, relation)] = arm.references.as_slice()
        {
            // A set reference already has the required key and multiplicity. No AST lookup.
            return Ok(format!("SELECT file,node FROM r{relation}"));
        }
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
            .filter(|(_, (_, r))| !requested || Some(*r) == owner)
            .min_by_key(|(_, (_, r))| if Some(*r) == owner { 0 } else { 1 })
        {
            let name = &names[node];
            sources.push(format!("r{relation} AS seed{serial}"));
            sources.push(format!(
                "node AS {name} ON {name}.file=seed{serial}.file AND {name}.id=seed{serial}.node"
            ));
            known[node] = true;
            consumed = Some(index);
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
            let node = arm
                .predicates
                .iter()
                .filter_map(|p| match p {
                    Predicate::Kinds(n, k) => Some((*n, k.len())),
                    _ => None,
                })
                .min_by_key(|(_, size)| *size)
                .map_or(0, |(n, _)| n);
            sources.push(format!("node AS {}", names[node]));
            known[node] = true;
        }
        while known.iter().any(|k| !k) {
            let Some(&(from, edge, to)) = arm
                .edges
                .iter()
                .find(|(from, _, to)| known[*from] != known[*to])
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
                Predicate::Regex(node, term) => format!(
                    "node_regex({term},{}.file,{}.id)",
                    names[*node], names[*node]
                ),
                Predicate::Pattern(node,term)=>format!("relation_pattern({term},{}.file,{}.id)",names[*node],names[*node]),
                Predicate::Region(node,roots)=>format!("EXISTS (SELECT 1 FROM regions{roots} domain WHERE domain.file={}.file AND {}.id>domain.node AND {}.id<=domain.last)",names[*node],names[*node],names[*node]),
                Predicate::Keys(node,producer)=>format!("EXISTS (SELECT 1 FROM needed{producer} k WHERE k.file={}.file AND k.node={}.id)",names[*node],names[*node]),
                Predicate::Absent(node, set) => self.absent(set, &names[*node])?,
            });
        }
        // A bound root can leave the first source with an ON clause. SQLite needs a left source.
        let from = if sources.is_empty() {
            String::new()
        } else if bound.is_some() {
            format!(
                " FROM (SELECT 1) AS unit{serial} CROSS JOIN {}",
                sources.join(" CROSS JOIN ")
            )
        } else {
            format!(" FROM {}", sources.join(" CROSS JOIN "))
        };
        let filter = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };
        let projection = if bound.is_some() {
            "1".into()
        } else {
            format!(
                "{}{}.file,{}.id",
                if distinct { "DISTINCT " } else { "" },
                names[0],
                names[0]
            )
        };
        Ok(format!("SELECT {projection}{from}{filter}"))
    }
}

pub(super) fn capture_demand(
    program: &Program,
    plan: &[RelationPlan],
    demand: &super::ir::CaptureDemand,
) -> Result<String, String> {
    let (roots, targets, leaf_stop) = (demand.roots, demand.targets, demand.leaf_stop);
    let mut definitions = definitions(program, plan)?;
    let named = if demand.named { " AND n.named" } else { "" };
    definitions.push(format!("binding_seeds(file,node,atom) AS MATERIALIZED (SELECT n.file,n.id,relation_leaf_atom(n.file,n.id,n.atom) FROM r{targets} t CROSS JOIN node n ON n.file=t.file AND n.id=t.node WHERE 1{named})"));
    let stopped = leaf_stop.map_or(String::new(), |id| {
        format!(" AND NOT EXISTS (SELECT 1 FROM r{id} s WHERE s.file=w.file AND s.node=w.node)")
    });
    definitions.push(format!("binding_walk(file,atom,witness,node) AS (SELECT n.file,t.atom,t.node,n.parent FROM binding_seeds t CROSS JOIN node n ON n.file=t.file AND n.id=t.node WHERE n.parent IS NOT NULL UNION ALL SELECT w.file,w.atom,w.witness,n.parent FROM binding_walk w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE n.parent IS NOT NULL{stopped})"));
    // AST tape ordinals are preorder keys, which is precisely this admitted `has` traversal order.
    definitions.push(format!("binding_choices(file,node,atom,witness) AS MATERIALIZED (SELECT w.file,w.node,w.atom,MIN(w.witness) FROM binding_walk w JOIN r{roots} k ON k.file=w.file AND k.node=w.node GROUP BY w.file,w.node,w.atom)"));
    definitions.push(format!("origin_choices AS (SELECT p.file,p.origin,b.atom,b.witness,p.node AS ancestor,p.distance,ROW_NUMBER() OVER (PARTITION BY p.file,p.origin,b.atom ORDER BY p.distance) AS choice FROM pairs{roots} p JOIN binding_choices b ON b.file=p.file AND b.node=p.node)"));
    definitions.push("capture_demand AS MATERIALIZED (SELECT file,origin,atom,witness,ancestor,distance FROM origin_choices WHERE choice=1)".into());
    let sql = format!(
        "WITH RECURSIVE\n{}\nSELECT file,origin,atom,witness,ancestor,distance FROM capture_demand ORDER BY file,origin,atom",
        definitions.join(",\n")
    );
    Ok(sql)
}
