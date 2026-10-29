//! Compose independently bounded fragments. Full definitions establish sharing; canonical
//! topological numbering makes placement independent of fragment and allocation order.
use super::{Definition, Graph, Node, Part, parameter, parse_parts};
use rusqlite::types::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Default)]
pub(crate) struct Program {
    graph: Graph,
    values: BTreeMap<Vec<u8>, Value>,
    cf: Option<(usize, Vec<Part>)>,
    all: Option<usize>,
}
pub(crate) struct Layout {
    pub ctes: Vec<String>,
    pub parameters: Vec<Value>,
    pub names: HashMap<usize, String>,
    pub ranks: HashMap<usize, usize>,
    pub parameter_indices: BTreeMap<Vec<u8>, usize>,
}
impl Layout {
    pub fn bound(&self, value: &Value) -> String {
        format!("?{}", self.parameter_indices[&parameter(value)])
    }
}

impl Program {
    pub fn add(
        &mut self,
        ctes: &[String],
        parameters: &[Value],
        language: u32,
    ) -> HashMap<String, Node> {
        for value in parameters {
            self.values
                .entry(parameter(value))
                .or_insert_with(|| value.clone());
        }
        let nodes = self.graph.register(ctes, parameters, language);
        if let Some(node) = nodes.get("cf") {
            let cte = ctes
                .iter()
                .find(|cte| cte.starts_with("cf("))
                .expect("file scope");
            let parts = parse_parts(
                cte.split_once(") AS ").expect("file body").1,
                parameters,
                &HashMap::new(),
            )
            .0;
            if let Some((id, previous)) = &self.cf {
                assert_eq!(*id, node.id, "one tape scope per program");
                assert_eq!(*previous, parts, "file domain is a semantic input");
            } else {
                self.cf = Some((node.id, parts));
            }
        }
        if let Some(node) = nodes.get("all_nodes") {
            if let Some(id) = self.all {
                assert_eq!(id, node.id, "one all-node source");
            } else {
                self.all = Some(node.id);
            }
        }
        nodes
    }
    pub fn finish(self) -> Layout {
        let count = self.graph.definitions.len();
        let mut definitions = vec![None; count + 1];
        for (definition, id) in self.graph.definitions {
            definitions[id] = Some(definition);
        }
        let mut remaining = vec![0; count + 1];
        let mut consumers = vec![Vec::new(); count + 1];
        for id in 1..=count {
            let dependencies: BTreeSet<_> = definitions[id]
                .as_ref()
                .expect("definition")
                .parts
                .iter()
                .filter_map(|p| {
                    if let Part::Reference(id) = p {
                        Some(*id)
                    } else {
                        None
                    }
                })
                .collect();
            remaining[id] = dependencies.len();
            for dependency in dependencies {
                consumers[dependency].push(id);
            }
        }
        fn normalized(definition: &Definition, ranks: &HashMap<usize, usize>) -> Definition {
            let mut value = definition.clone();
            for part in &mut value.parts {
                if let Part::Reference(id) = part {
                    *id = ranks[id];
                }
            }
            value
        }
        let mut ready = BTreeSet::new();
        let mut ranks = HashMap::new();
        let mut order = Vec::new();
        for id in 1..=count {
            if remaining[id] == 0 {
                ready.insert((definitions[id].as_ref().expect("definition").clone(), id));
            }
        }
        while let Some((_, id)) = ready.pop_first() {
            ranks.insert(id, order.len() + 1);
            order.push(id);
            for &consumer in &consumers[id] {
                remaining[consumer] -= 1;
                if remaining[consumer] == 0 {
                    ready.insert((
                        normalized(definitions[consumer].as_ref().expect("definition"), &ranks),
                        consumer,
                    ));
                }
            }
        }
        assert_eq!(
            order.len(),
            count,
            "generated outer producer dependencies are acyclic"
        );
        let names: HashMap<_, _> = ranks
            .iter()
            .map(|(&id, &rank)| {
                (
                    id,
                    if self.cf.as_ref().is_some_and(|(cf, _)| *cf == id) {
                        "cf".into()
                    } else if self.all == Some(id) {
                        "all_nodes".into()
                    } else {
                        format!("r{rank}")
                    },
                )
            })
            .collect();
        let used: BTreeSet<_> = definitions
            .iter()
            .flatten()
            .flat_map(|d| d.parts.iter())
            .chain(self.cf.iter().flat_map(|(_, parts)| parts.iter()))
            .filter_map(|part| {
                if let Part::Parameter(value) = part {
                    Some(value.clone())
                } else {
                    None
                }
            })
            .collect();
        let parameters: Vec<_> = used.iter().map(|key| self.values[key].clone()).collect();
        let parameter_indices: BTreeMap<_, _> = used
            .into_iter()
            .enumerate()
            .map(|(i, key)| (key, i + 1))
            .collect();
        let render = |parts: &[Part]| {
            parts
                .iter()
                .map(|part| match part {
                    Part::Sql(sql) => sql.clone(),
                    Part::Reference(id) => names[id].clone(),
                    Part::Parameter(key) => format!("?{}", parameter_indices[key]),
                    Part::FileDomain => {
                        unreachable!("file domain resolved from the pinned source slot")
                    }
                })
                .collect::<String>()
        };
        let ctes = order
            .into_iter()
            .map(|id| {
                let d = definitions[id].as_ref().expect("definition");
                let parts = if d.parts == [Part::FileDomain] {
                    &self.cf.as_ref().expect("file domain").1
                } else {
                    &d.parts
                };
                format!("{}({}) AS {}", names[&id], d.columns, render(parts))
            })
            .collect();
        Layout {
            ctes,
            parameters,
            names,
            ranks,
            parameter_indices,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_producers_parameters_and_bag_rows_ignore_fragment_order_and_aliases() {
        fn build(reverse: bool) -> (Layout, Vec<(usize, usize)>) {
            let fragments = [
                (
                    vec![
                        "a(fid,pre) AS MATERIALIZED (SELECT ?1,?2 UNION ALL SELECT ?1,?2)".into(),
                        "b(fid,pre) AS MATERIALIZED (SELECT * FROM a)".into(),
                    ],
                    vec![1.into(), 7.into()],
                    "b",
                ),
                (
                    vec![
                        "renamed(fid,pre) AS MATERIALIZED (SELECT ?2,?1 UNION ALL SELECT ?2,?1)"
                            .into(),
                        "consumer(fid,pre) AS MATERIALIZED (SELECT * FROM renamed)".into(),
                    ],
                    vec![7.into(), 1.into()],
                    "consumer",
                ),
            ];
            let mut program = Program::default();
            let mut outputs = Vec::new();
            for i in if reverse { vec![1, 0] } else { vec![0, 1] } {
                let (ctes, values, root) = &fragments[i];
                let nodes = program.add(ctes, values, 1);
                outputs.push((i, nodes[*root].id));
            }
            (program.finish(), outputs)
        }
        let (a, ar) = build(false);
        let (b, br) = build(true);
        assert_eq!(a.ctes, b.ctes);
        assert_eq!(a.parameters, b.parameters);
        assert_eq!(a.ctes.len(), 2);
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let query = |layout: &Layout, roots: &[(usize, usize)]| {
            format!(
                "WITH {} {}",
                layout.ctes.join(","),
                roots
                    .iter()
                    .map(|(_, root)| format!("SELECT * FROM {}", layout.names[root]))
                    .collect::<Vec<_>>()
                    .join(" UNION ALL ")
            )
        };
        for (layout, roots) in [(a, ar), (b, br)] {
            let rows = db
                .prepare(&query(&layout, &roots))
                .unwrap()
                .query_map(rusqlite::params_from_iter(&layout.parameters), |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
                })
                .unwrap()
                .collect::<rusqlite::Result<Vec<_>>>()
                .unwrap();
            assert_eq!(rows, vec![(1, 7); 4]);
        }
    }
    #[test]
    fn file_domain_and_parameter_types_remain_explicit() {
        let mut program = Program::default();
        let ctes = vec![
            "cf(fid) AS MATERIALIZED (SELECT fid FROM file WHERE lang=?1)".into(),
            "all_nodes(fid,pre) AS MATERIALIZED (SELECT fid,?2 FROM cf)".into(),
        ];
        program.add(&ctes, &[1.into(), 3.into()], 1);
        let layout = program.finish();
        assert!(layout.ctes[0].starts_with("cf("));
        assert!(layout.ctes.iter().any(|s| s.starts_with("all_nodes(")));
        assert_eq!(layout.parameters.len(), 2);
        let mut program = Program::default();
        let a = program.add(
            &["a(fid,pre) AS MATERIALIZED (SELECT 1,?1)".into()],
            &[7.into()],
            1,
        );
        let b = program.add(
            &["b(fid,pre) AS MATERIALIZED (SELECT 1,?1)".into()],
            &["7".to_owned().into()],
            1,
        );
        assert_ne!(a["a"].id, b["b"].id);
    }
}

#[cfg(test)]
mod scope_tests {
    use super::*;
    #[test]
    #[should_panic(expected = "file domain is a semantic input")]
    fn distinct_requested_domains_cannot_share_one_program() {
        let mut p = Program::default();
        let ctes = ["cf(fid) AS MATERIALIZED (SELECT fid FROM file WHERE fid=?1)".into()];
        p.add(&ctes, &[1.into()], 1);
        p.add(&ctes, &[2.into()], 1);
    }
    #[test]
    fn recursive_sql_remains_inside_one_outer_operation() {
        let mut p = Program::default();
        let nodes=p.add(&["walk(fid,pre) AS MATERIALIZED (WITH RECURSIVE positions(pre) AS (SELECT 1 UNION ALL SELECT pre+1 FROM positions WHERE pre<3) SELECT ?1,pre FROM positions)".into()],&[7.into()],1);
        let layout = p.finish();
        let root = &layout.names[&nodes["walk"].id];
        let sql = format!("WITH {} SELECT * FROM {root}", layout.ctes.join(","));
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let rows = db
            .prepare(&sql)
            .unwrap()
            .query_map(rusqlite::params_from_iter(&layout.parameters), |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(rows, [(7, 1), (7, 2), (7, 3)]);
    }
}
