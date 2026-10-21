use super::lex;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Policy {
    Boundaries,
    Reuse,
    All,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Boundary {
    None,
    Identity,
    Terminal,
    Demand,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Column {
    pub name: String,
    pub integer: bool,
}
#[derive(Clone)]
pub(super) struct Relation {
    pub columns: Vec<Column>,
    pub body: String,
    pub boundary: Boundary,
    pub uses: usize,
}
#[derive(Default, Clone)]
pub(super) struct Graph {
    pub relations: Vec<Relation>,
    index: HashMap<(Vec<Column>, String, Boundary), usize>,
    pub intern_hits: usize,
    pub barriers: Vec<usize>,
    pub handles: Vec<(String, usize)>,
}
impl Graph {
    pub fn add(&mut self, columns: Vec<Column>, body: String, boundary: Boundary) -> usize {
        let key = (columns.clone(), body.clone(), boundary);
        if let Some(&id) = self.index.get(&key) {
            self.intern_hits += 1;
            return id;
        }
        let id = self.relations.len();
        for word in lex::identifiers(&body) {
            if let Some(id) = word
                .strip_prefix('q')
                .and_then(|s| s.parse::<usize>().ok())
                .filter(|&id| id < self.relations.len())
            {
                self.relations[id].uses += 1;
            }
        }
        self.relations.push(Relation {
            columns,
            body,
            boundary,
            uses: 0,
        });
        self.index.insert(key, id);
        if boundary == Boundary::Terminal {
            self.barriers.push(id);
        }
        id
    }
    pub fn dependencies(&self, body: &str) -> HashSet<usize> {
        lex::identifiers(body)
            .into_iter()
            .filter_map(|word| {
                word.strip_prefix('q')
                    .and_then(|id| id.parse::<usize>().ok())
                    .filter(|&id| id < self.relations.len())
            })
            .collect()
    }

    pub fn name(id: usize) -> String {
        format!("q{id}")
    }
    pub fn expanded_references(&self) -> u64 {
        let mut expanded = Vec::<u64>::new();
        for relation in &self.relations {
            let count = lex::identifiers(&relation.body)
                .into_iter()
                .filter_map(|word| {
                    word.strip_prefix('q')
                        .and_then(|id| id.parse::<usize>().ok())
                        .and_then(|id| expanded.get(id).copied())
                })
                .fold(1u64, u64::saturating_add);
            expanded.push(count);
        }
        expanded.into_iter().max().unwrap_or(0)
    }
    pub fn render(&self, outputs: &[(usize, usize)], policy: Policy) -> String {
        let mut reachable = HashSet::new();
        let mut pending: Vec<_> = outputs
            .iter()
            .map(|&(_, id)| id)
            .chain(self.barriers.iter().copied())
            .collect();
        while let Some(id) = pending.pop() {
            if reachable.insert(id) {
                pending.extend(self.dependencies(&self.relations[id].body));
            }
        }
        let mut definitions = self
            .relations
            .iter()
            .enumerate()
            .filter(|(id, _)| reachable.contains(id))
            .map(|(id, rel)| {
                let materialize = rel.boundary != Boundary::None
                    || policy == Policy::All
                    || (policy == Policy::Reuse && rel.uses > 1);
                format!(
                    "{}({}) AS {} ({})",
                    Self::name(id),
                    rel.columns
                        .iter()
                        .map(|c| c.name.as_str())
                        .collect::<Vec<_>>()
                        .join(","),
                    if materialize {
                        "MATERIALIZED"
                    } else {
                        "NOT MATERIALIZED"
                    },
                    rel.body
                )
            })
            .collect::<Vec<_>>()
            .join(",\n");
        // This sentinel forces required terminal evaluations even when later filters remove
        // every finding. It exposes ordering/effect preservation instead of hiding extra SQL.
        let fences = self
            .barriers
            .iter()
            .map(|&id| format!("(SELECT count(*) FROM {})", Self::name(id)))
            .collect::<Vec<_>>();
        let force = if fences.is_empty() {
            "0".into()
        } else {
            fences.join(" + ")
        };
        let mut arms = vec![format!(
            "SELECT -1 AS clause, -1 AS file, -1 AS node, CAST({force} AS TEXT) AS captures"
        )];
        for &(clause, id) in outputs {
            let cols = &self.relations[id].columns;
            let captures = cols
                .iter()
                .skip(2)
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>()
                .join(",");
            arms.push(format!(
                "SELECT {clause},file,node,json_array({captures}) FROM {}",
                Self::name(id)
            ));
        }
        let combined = if arms.len() > 128 {
            let chunks = arms
                .chunks(128)
                .enumerate()
                .map(|(i, chunk)| {
                    definitions.push_str(&format!(
                        ",\npack{i}(clause,file,node,captures) AS MATERIALIZED ({})",
                        chunk.join(" UNION ALL ")
                    ));
                    format!("SELECT * FROM pack{i}")
                })
                .collect::<Vec<_>>();
            chunks.join(" UNION ALL ")
        } else {
            arms.join(" UNION ALL ")
        };
        format!("WITH {definitions}\n{combined} ORDER BY clause,file,node")
    }
}
