use super::{
    ir::{Edge, Program, Set},
    physical::{self, Arm, Predicate},
};

#[derive(Default)]
struct Emitter {
    aliases: usize,
}

pub(super) fn emit(program: &Program, plan: &[Vec<Arm>]) -> Result<String, String> {
    let mut emitter = Emitter::default();
    let mut relations = Vec::new();
    for (id, arms) in plan.iter().enumerate() {
        let selects = arms
            .iter()
            .map(|arm| emitter.arm(arm, Some(id), None, arms.len() == 1))
            .collect::<Result<Vec<_>, _>>()?;
        let body = if selects.is_empty() {
            "SELECT file,id FROM node WHERE 0".into()
        } else {
            selects.join("\nUNION\n")
        };
        relations.push(format!(
            "-- {}\nr{id}(file,node) AS (\n{body}\n)",
            program.relations[id].source
        ));
    }
    Ok(format!(
        "WITH RECURSIVE\n{}\nSELECT 0,file,node,json_array() FROM r0 ORDER BY file,node",
        relations.join(",\n")
    ))
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
        if let Some(root) = bound {
            names[0] = root.to_owned();
            known[0] = true;
        } else if let Some((index, &(node, relation))) = arm
            .references
            .iter()
            .enumerate()
            .min_by_key(|(_, (_, r))| if Some(*r) == owner { 0 } else { 1 })
        {
            let name = &names[node];
            sources.push(format!("r{relation} AS seed{serial}"));
            sources.push(format!(
                "node AS {name} ON {name}.file=seed{serial}.file AND {name}.id=seed{serial}.node"
            ));
            known[node] = true;
            consumed = Some(index);
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
