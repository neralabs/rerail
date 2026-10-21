//! Direct symbolic lowering for total, capture-free operators. It does not pass through frame
//! allocation or parse emitted SQL. The broader SSA adapter is the migration experiment.
use super::graph::{Boundary, Column, Graph};
use crate::code_rule_dsl::sql::plan::{Op, Statement, Stop, Walk};

#[derive(Default)]
pub(super) struct Direct {
    pub graph: Graph,
    source: Option<usize>,
}
impl Direct {
    fn source(&mut self) -> usize {
        if let Some(id) = self.source {
            return id;
        }
        let columns = [
            "k", "file", "node", "kind", "parent", "last", "wpar", "anext", "aprev", "next", "prev",
        ]
        .iter()
        .map(|s| Column {
            name: (*s).into(),
            integer: true,
        })
        .collect();
        let id=self.graph.add(columns,"SELECT file*4294967296+id,file,id,kind,parent,\"last\",wpar,anext,aprev,next,prev FROM node".into(),Boundary::None);
        self.source = Some(id);
        id
    }
    fn selection(&mut self, input: usize, where_clause: String, boundary: Boundary) -> usize {
        self.graph.add(
            self.graph.relations[input].columns.clone(),
            format!(
                "SELECT f.* FROM {} f WHERE {where_clause}",
                Graph::name(input)
            ),
            boundary,
        )
    }
    fn op(&mut self, op: &Op, input: usize) -> Result<usize, String> {
        Ok(match op {
            Op::Kinds(kinds) => {
                let predicate = if kinds.is_empty() {
                    "0".into()
                } else {
                    let kinds = kinds
                        .iter()
                        .map(u16::to_string)
                        .collect::<Vec<_>>()
                        .join(",");
                    format!("f.kind IN ({kinds})")
                };
                self.selection(input, predicate, Boundary::None)
            }
            Op::Never => self.selection(input, "0".into(), Boundary::None),
            Op::Regex(term) => {
                let source = Graph::name(input);
                let predicate = format!(
                    "f.k IN (SELECT ek FROM text_match WHERE term={term} AND ek IN (SELECT k FROM {source}))"
                );
                self.selection(input, predicate, Boundary::Terminal)
            }
            Op::All(items) => {
                let mut current = input;
                for op in items {
                    current = self.op(op, current)?;
                }
                current
            }
            Op::Any(items) => {
                let mut remainder = input;
                let mut outputs = Vec::new();
                for op in items {
                    let matched = self.op(op, remainder)?;
                    outputs.push(matched);
                    let predicate = format!("f.k NOT IN (SELECT k FROM {})", Graph::name(matched));
                    remainder = self.selection(remainder, predicate, Boundary::None);
                }
                if outputs.is_empty() {
                    self.selection(input, "0".into(), Boundary::None)
                } else {
                    let body = outputs
                        .into_iter()
                        .map(|id| format!("SELECT * FROM {}", Graph::name(id)))
                        .collect::<Vec<_>>()
                        .join(" UNION ALL ");
                    self.graph.add(
                        self.graph.relations[input].columns.clone(),
                        body,
                        Boundary::None,
                    )
                }
            }
            Op::Not(inner) => {
                let inner = self.op(inner, input)?;
                let predicate = format!(
                    "NOT EXISTS(SELECT 1 FROM {} m WHERE m.k=f.k)",
                    Graph::name(inner)
                );
                self.selection(input, predicate, Boundary::None)
            }
            Op::Relation {
                walk,
                sub,
                binds: false,
                ..
            } => {
                let source = self.source();
                let eligible = self.op(sub, source)?;
                let condition = match walk {
                    Walk::Has(Stop::Neighbor) => "e.parent=f.node",
                    Walk::Has(Stop::End) => "e.node>f.node AND e.node<=f.last",
                    Walk::Inside(Stop::Neighbor) => "e.node=f.parent",
                    Walk::Inside(Stop::End) => "e.node<f.node AND e.last>=f.node",
                    Walk::Precedes(Stop::Neighbor) => "e.node=f.next",
                    Walk::Precedes(Stop::End) => "e.parent=f.wpar AND e.node>=f.anext",
                    Walk::Follows(Stop::Neighbor) => "e.node=f.prev",
                    Walk::Follows(Stop::End) => "e.parent=f.wpar AND e.node<=f.aprev",
                    _ => return Err("direct lowering: stop-set or field walk".into()),
                };
                let predicate = format!(
                    "EXISTS(SELECT 1 FROM {} e WHERE e.file=f.file AND {condition})",
                    Graph::name(eligible)
                );
                self.selection(input, predicate, Boundary::None)
            }
            _ => return Err("direct lowering: captures, effects, or recursive state".into()),
        })
    }
    pub fn clause(&mut self, statement: &Statement) -> Result<usize, String> {
        if !statement.slots.is_empty() || !statement.units.is_empty() {
            return Err("direct lowering: scoped captures or recursion".into());
        }
        let mut input = self.source();
        if let Some(kinds) = &statement.kinds {
            input = self.op(&Op::Kinds(kinds.clone()), input)?;
        }
        let output = self.op(&statement.op, input)?;
        Ok(self.graph.add(
            vec![
                Column {
                    name: "file".into(),
                    integer: true,
                },
                Column {
                    name: "node".into(),
                    integer: true,
                },
            ],
            format!("SELECT file,node FROM {}", Graph::name(output)),
            Boundary::None,
        ))
    }
}
