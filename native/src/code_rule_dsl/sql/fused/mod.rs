//! Immutable relational components in the SQL engine. Stateful instructions retain their
//! existing scheduling contract; eligible clauses share one prepared program across a batch.
pub(super) mod adapter;
pub(super) mod buffers;
pub(super) mod constants;
pub(super) mod graph;
pub(super) mod lex;
pub(super) mod physical;
#[cfg(any(test, feature = "unbounded-work"))]
pub(super) mod profile;
mod publish;
pub(super) mod shapes;
mod tree;
pub(super) mod witness;
pub(super) use tree::register;

use super::{emit::Emitted, lower::Stage, plan::Statement};
use adapter::Compiler;

#[derive(Default)]
pub(super) struct Builder {
    compiler: Compiler,
    outputs: Vec<(usize, usize)>,
    roots: std::collections::HashMap<usize, usize>,
    scheduled: Vec<(usize, String)>,
}

pub(super) struct Program {
    pub plan: physical::Plan,
    pub clauses: Vec<usize>,
}

impl Builder {
    pub fn clause(
        &mut self,
        index: usize,
        statement: &Statement,
        sql: &Emitted,
        stages: &[Stage],
        terminals: &[super::terminals::Terminal],
        each: bool,
    ) {
        if each {
            self.scheduled
                .push((index, "enumeration requires exclusion rounds".into()));
            return;
        }
        if let Ok(body) = witness::engine(statement, terminals, index) {
            let columns = ["file", "node", "s0"]
                .into_iter()
                .map(|name| graph::Column {
                    name: name.into(),
                    integer: false,
                })
                .collect();
            let id = self
                .compiler
                .graph
                .add(columns, body, graph::Boundary::Terminal);
            self.outputs.push((index, id));
            return;
        }
        // Admission is atomic. A rejected stateful component cannot leave terminal effects,
        // handles or interned definitions in the immutable component.
        let saved = self.compiler.checkpoint();
        self.compiler.stage_files = Some("SELECT file FROM runs WHERE run=-1-{stage}".into());
        self.compiler.selected_roots =
            Some(format!("SELECT file,node FROM roots WHERE run={index}"));
        let mut columns = vec!["file".to_owned(), "node".to_owned()];
        columns.extend(
            statement
                .slots
                .iter()
                .enumerate()
                .filter(|(_, s)| s.scope == 0)
                .flat_map(|(id, s)| s.columns(id)),
        );
        match self.compiler.emitted(
            sql,
            &columns.join(","),
            stages,
            &format!("SELECT file FROM runs WHERE run={index}"),
        ) {
            Ok(id) => {
                if let Some(root) = self.compiler.last_root {
                    self.roots.insert(index, root);
                }
                self.outputs.push((index, id));
            }
            Err(reason) => {
                self.compiler.rollback(saved);
                self.scheduled.push((index, reason));
            }
        }
    }

    pub fn finish(mut self, sole: &[Option<usize>], batch_size: usize) -> Option<Program> {
        if self.outputs.is_empty() {
            return None;
        }
        // Preserve the scheduler's deferred-stage contract. A sole consumer with an empty
        // root domain must not evaluate an independent, potentially fallible stage.
        for (stage, clause) in sole.iter().enumerate() {
            let Some(root) = clause.and_then(|clause| self.roots.get(&clause).copied()) else {
                continue;
            };
            let files = format!("SELECT file FROM runs WHERE run=-1-{stage}");
            let guarded = format!("{files} AND EXISTS(SELECT file FROM q{root})");
            for relation in &mut self.compiler.graph.relations {
                relation.body = relation
                    .body
                    .replace(&format!("{files})"), &format!("{guarded})"));
            }
        }
        let narrow = constants::apply(&mut self.compiler.graph, &self.outputs);
        let mut plan = physical::plan(
            &self.compiler.graph,
            &self.outputs,
            if batch_size <= 8 { 4 } else { 16 },
            self.compiler.graph.relations.len().div_ceil(3000).max(
                if self.compiler.graph.relations.len() > 512 {
                    4
                } else {
                    1
                },
            ),
        );
        plan.metadata["narrowCaptures"] = narrow;
        plan.metadata["scheduledClauses"] = serde_json::json!(
            self.scheduled
                .into_iter()
                .map(|(clause, reason)| serde_json::json!({"clause":clause,"reason":reason}))
                .collect::<Vec<_>>()
        );
        Some(Program {
            plan,
            clauses: self.outputs.into_iter().map(|o| o.0).collect(),
        })
    }
}

impl Program {
    pub fn register(
        &self,
        connection: &rusqlite::Connection,
        frames: &std::sync::Arc<super::frames::FrameStore>,
    ) -> rusqlite::Result<()> {
        publish::register(connection, frames, &self.plan.graph)
    }
    pub fn clear(&self, frames: &super::frames::FrameStore) {
        for (name, _) in &self.plan.graph.handles {
            frames.clear(name);
        }
    }
    pub fn contains(&self, clause: usize) -> bool {
        self.clauses.binary_search(&clause).is_ok()
    }
}
