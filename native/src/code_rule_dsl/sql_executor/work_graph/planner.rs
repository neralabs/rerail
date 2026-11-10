//! Bounded CTE fusion. A dependency crosses a statement boundary only when a group is full.
use super::*;

const MAX_ARMS: usize = 400;
const MAX_EXPANSION: usize = phase::Phase::EXPANSION_BUDGET;

#[derive(Default)]
pub(super) struct Group {
    pub(super) nodes: Vec<usize>,
    pub dependencies: BTreeSet<usize>,
    pub inputs: BTreeSet<String>,
    pub outputs: Vec<(String, usize)>,
    pub cost: usize,
    partitioned: bool,
}
pub(super) struct Plan {
    pub groups: Vec<Group>,
}

impl Group {
    pub fn sql(&self, request: &Request) -> String {
        let width = self.outputs.iter().map(|(_, w)| *w).max().unwrap();
        let definitions = self
            .nodes
            .iter()
            .map(|&id| {
                let node = &request.nodes[id];
                let columns = (0..node.width)
                    .map(|c| format!("c{c}"))
                    .collect::<Vec<_>>()
                    .join(",");
                format!(
                    "{}({columns}) AS MATERIALIZED ({})",
                    quote(&node.name),
                    node.sql
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let reads = self
            .outputs
            .iter()
            .enumerate()
            .map(|(id, (name, w))| {
                let columns = (0..width)
                    .map(|c| {
                        if c < *w {
                            format!("c{c}")
                        } else {
                            "NULL".into()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                format!("SELECT {id},{columns} FROM {}", quote(name))
            })
            .collect::<Vec<_>>()
            .join(" UNION ALL ");
        format!("WITH RECURSIVE {definitions} {reads}")
    }
    fn fits(&self, component: &[usize], request: &Request) -> Option<usize> {
        if self.nodes.len() + component.len() > MAX_ARMS {
            return None;
        }
        // Preparation expands dependency references even behind materialization fences.
        let mut costs = BTreeMap::<&str, usize>::new();
        let mut total = 0usize;
        for &id in self.nodes.iter().chain(component) {
            let node = &request.nodes[id];
            let mut cost = node.sql.len().div_ceil(16).max(1);
            let tokens = identifiers(&node.sql);
            for read in &node.reads {
                let uses = tokens
                    .iter()
                    .filter(|t| t.eq_ignore_ascii_case(read))
                    .count()
                    .max(1);
                cost = cost.saturating_add(
                    costs
                        .get(read.as_str())
                        .copied()
                        .unwrap_or(1)
                        .saturating_mul(uses),
                );
            }
            costs.insert(&node.name, cost);
            total = total.saturating_add(cost);
        }
        (total <= MAX_EXPANSION).then_some(total)
    }
}
impl Plan {
    pub fn new(
        request: &Request,
        sources: &BTreeMap<String, Arc<dyn Source>>,
        threads: usize,
    ) -> std::result::Result<Self, String> {
        if request.outputs.is_empty() {
            return Err("SQL work graph needs an output".into());
        }
        let mut names = BTreeMap::new();
        for (id, node) in request.nodes.iter().enumerate() {
            if !identifier(&node.name)
                || node.width == 0
                || node.width > 400
                || sources.keys().any(|n| n.eq_ignore_ascii_case(&node.name))
                || names.insert(node.name.to_ascii_lowercase(), id).is_some()
            {
                return Err(format!("invalid or duplicate SQL work node {}", node.name));
            }
            if node.reads.iter().collect::<BTreeSet<_>>().len() != node.reads.len() {
                return Err(format!("duplicate SQL work dependency for {}", node.name));
            }
        }
        // Declared edges are a contract, not a hint. Conservatively reject references to
        // other graph/input names outside that contract, including quoted identifiers.
        // The compiler uses unique names; authored local aliases must avoid graph names.
        let known = names
            .keys()
            .cloned()
            .chain(sources.keys().map(|n| n.to_ascii_lowercase()))
            .collect::<BTreeSet<_>>();
        for node in &request.nodes {
            let reads = node
                .reads
                .iter()
                .map(|n| n.to_ascii_lowercase())
                .collect::<BTreeSet<_>>();
            for token in identifiers(&node.sql) {
                if known.contains(&token) && !reads.contains(&token) {
                    return Err(format!(
                        "{} references {token} without a declared read",
                        node.name
                    ));
                }
            }
        }
        let mut dependencies = vec![BTreeSet::new(); request.nodes.len()];
        let mut parent = (0..request.nodes.len()).collect::<Vec<_>>();
        fn root(parent: &[usize], mut n: usize) -> usize {
            while parent[n] != n {
                n = parent[n];
            }
            n
        }
        for (id, node) in request.nodes.iter().enumerate() {
            for read in &node.reads {
                if let Some(&dep) = names.get(&read.to_ascii_lowercase()) {
                    if read != &request.nodes[dep].name {
                        return Err("SQL work names must use their declared case".into());
                    }
                    dependencies[id].insert(dep);
                    let a = root(&parent, id);
                    let b = root(&parent, dep);
                    parent[a] = b;
                } else if !sources.contains_key(read) {
                    return Err(format!("{} reads missing SQL input {read}", node.name));
                }
            }
        }
        if request.outputs.iter().collect::<BTreeSet<_>>().len() != request.outputs.len()
            || request.outputs.iter().any(|n| {
                !names.contains_key(&n.to_ascii_lowercase())
                    || request.nodes[names[&n.to_ascii_lowercase()]].name != *n
            })
        {
            return Err("SQL work outputs must name distinct work nodes".into());
        }
        // Prune unobserved work before grouping; it must consume neither SQL nor a job ticket.
        let mut wanted = request
            .outputs
            .iter()
            .map(|n| names[&n.to_ascii_lowercase()])
            .collect::<BTreeSet<_>>();
        let mut todo = wanted.iter().copied().collect::<Vec<_>>();
        while let Some(id) = todo.pop() {
            for &dep in &dependencies[id] {
                if wanted.insert(dep) {
                    todo.push(dep);
                }
            }
        }
        let mut ordered = vec![];
        let mut seen = BTreeSet::new();
        while seen.len() < wanted.len() {
            let ready = wanted
                .iter()
                .copied()
                .filter(|id| !seen.contains(id) && dependencies[*id].is_subset(&seen))
                .collect::<Vec<_>>();
            if ready.is_empty() {
                return Err("SQL work graph has a dependency cycle".into());
            }
            for id in ready {
                seen.insert(id);
                ordered.push(id);
            }
        }
        let mut components = BTreeMap::<usize, Vec<usize>>::new();
        for id in ordered {
            components.entry(root(&parent, id)).or_default().push(id);
        }
        let mut components = components.into_values().collect::<Vec<_>>();
        components.sort_by_key(|ids| {
            std::cmp::Reverse(
                ids.iter()
                    .map(|&id| request.nodes[id].sql.len())
                    .sum::<usize>(),
            )
        });
        let parallel = threads.max(1).min(request.max_submissions.max(1));
        let mut groups = Vec::<Group>::new();
        for component in components {
            let whole = Group::default().fits(&component, request);
            if let Some(cost) = whole {
                let target = if groups.iter().filter(|g| !g.partitioned).count() < parallel
                    && groups.len() < request.max_submissions
                {
                    None
                } else {
                    groups
                        .iter()
                        .enumerate()
                        .filter(|(_, g)| !g.partitioned || groups.len() >= request.max_submissions)
                        .filter_map(|(id, g)| g.fits(&component, request).map(|cost| (id, cost)))
                        .min_by_key(|&(id, _)| groups[id].cost)
                };
                if let Some((id, cost)) = target {
                    groups[id].nodes.extend(component);
                    groups[id].cost = cost;
                } else {
                    groups.push(Group {
                        nodes: component,
                        cost,
                        ..Group::default()
                    });
                }
            } else {
                let mut chunk = Group::default();
                chunk.partitioned = true;
                for id in component {
                    if chunk.fits(&[id], request).is_none() && !chunk.nodes.is_empty() {
                        groups.push(chunk);
                        chunk = Group {
                            partitioned: true,
                            ..Group::default()
                        };
                    }
                    let cost = chunk.fits(&[id], request).ok_or_else(|| {
                        format!(
                            "SQL work node {} exceeds preparation allowance",
                            request.nodes[id].name
                        )
                    })?;
                    chunk.nodes.push(id);
                    chunk.cost = cost;
                }
                if !chunk.nodes.is_empty() {
                    groups.push(chunk);
                }
            }
        }
        if groups.len() > request.max_submissions {
            return Err(format!(
                "SQL work plan requires {} submissions; the job allowance is {} ({} nodes, expansion bound {MAX_EXPANSION}, arm bound {MAX_ARMS})",
                groups.len(),
                request.max_submissions,
                wanted.len()
            ));
        }
        let owner = groups
            .iter()
            .enumerate()
            .flat_map(|(g, group)| group.nodes.iter().map(move |&n| (n, g)))
            .collect::<BTreeMap<_, _>>();
        let mut exports = vec![BTreeSet::new(); groups.len()];
        for name in &request.outputs {
            let id = names[&name.to_ascii_lowercase()];
            exports[owner[&id]].insert(id);
        }
        for id in wanted {
            let g = owner[&id];
            for read in &request.nodes[id].reads {
                if let Some(&dep) = names.get(&read.to_ascii_lowercase()) {
                    let from = owner[&dep];
                    if from != g {
                        groups[g].dependencies.insert(from);
                        groups[g].inputs.insert(read.clone());
                        exports[from].insert(dep);
                    }
                } else {
                    groups[g].inputs.insert(read.clone());
                }
            }
        }
        for (group, export) in groups.iter_mut().zip(exports) {
            group.outputs = export
                .into_iter()
                .map(|id| {
                    let node = &request.nodes[id];
                    (node.name.clone(), node.width)
                })
                .collect();
        }
        Ok(Self { groups })
    }
    pub fn explain(&self, request: &Request) -> Value {
        json!({"executor":"sql","plannedSubmissions":self.groups.len(),"submissionAllowance":request.max_submissions,
            "maxDependencyExpansion":MAX_EXPANSION,"maxOutputArms":MAX_ARMS,
            "scheduling":"Release each successor when its dependencies finish; unrelated groups do not form a wave barrier",
            "policy":"Fuse connected operators; export cut edges once; schedule independent read-only groups on the shared pool",
            "groups":self.groups.iter().enumerate().map(|(id,g)|json!({"id":id,"executor":"sql",
                "nodes":g.nodes.iter().map(|&id|&request.nodes[id].name).collect::<Vec<_>>(),
                "dependencies":g.dependencies,"inputs":g.inputs,"outputs":g.outputs,
                "preparationCost":g.cost,"submissions":1})).collect::<Vec<_>>()})
    }
}

/// SQL lexical boundaries only; SQLite still validates syntax, reads, functions and effects.
fn identifiers(sql: &str) -> Vec<String> {
    let bytes = sql.as_bytes();
    let mut at = 0;
    let mut found = vec![];
    while at < bytes.len() {
        let start = at;
        match bytes[at] {
            b'\'' => {
                at += 1;
                while at < bytes.len() {
                    if bytes[at] == b'\'' {
                        at += 1;
                        if bytes.get(at) != Some(&b'\'') {
                            break;
                        }
                    }
                    at += 1;
                }
            }
            b'-' if bytes.get(at + 1) == Some(&b'-') => {
                while at < bytes.len() && bytes[at] != b'\n' {
                    at += 1;
                }
            }
            b'/' if bytes.get(at + 1) == Some(&b'*') => {
                at += 2;
                while at + 1 < bytes.len() && &bytes[at..at + 2] != b"*/" {
                    at += 1;
                }
                at = (at + 2).min(bytes.len());
            }
            b'"' | b'`' | b'[' => {
                let end = if bytes[at] == b'[' { b']' } else { bytes[at] };
                at += 1;
                let mut word = Vec::new();
                while at < bytes.len() {
                    if bytes[at] == end {
                        at += 1;
                        if bytes.get(at) != Some(&end) {
                            break;
                        }
                    }
                    word.push(bytes[at]);
                    at += 1;
                }
                found.push(String::from_utf8_lossy(&word).to_ascii_lowercase());
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                at += 1;
                while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
                    at += 1;
                }
                found.push(sql[start..at].to_ascii_lowercase());
            }
            _ => at += 1,
        }
    }
    found
}
