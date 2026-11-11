//! A canonical AST stage (`pythonAst`): the roots a stage selects, read by the language that registered a canonical
//! AST producer (`AstProducer`, Python's CPython `ast`), published as the stage's relation.
use super::{Result, syntax};
use rusqlite::Connection;
use std::sync::{Arc, atomic::AtomicUsize};

#[derive(Clone)]
pub(in crate::code_rule_dsl) struct Spec {
    pub roots: String,
    pub syntax: String,
    pub fields: Vec<String>,
}
pub(super) fn shape(fields: &[String]) -> String {
    let mut columns = vec![
        "CAST(NULL AS TEXT) AS __path".to_owned(),
        "CAST(NULL AS INTEGER) AS id".to_owned(),
    ];
    columns.extend(fields.iter().map(|f| format!("CAST(NULL AS TEXT) AS {f}")));
    format!("SELECT {} WHERE 0", columns.join(","))
}
pub(super) fn demand(
    db: &Connection,
    spec: &Spec,
    trees: &[super::producer::Tree],
    files: &syntax::Files,
    work: Arc<AtomicUsize>,
) -> Result<super::producer::Demand> {
    let request = super::Reachability {
        ordered_first: None,
        roots: spec.roots.clone(),
        edges: String::new(),
        batch_size: 256,
        forest: false,
        syntax: Some(spec.syntax.clone()),
        expand: false,
    };
    let mut demand = syntax::demand(db, &request, trees, files, work)?;
    demand.python_ast = Some(spec.fields.clone());
    Ok(demand)
}

/// What reads a canonical AST stage's roots: registered by the language whose AST it is (once, at load).
pub(in crate::code_rule_dsl) trait AstProducer: Send + Sync {
    /// The fields a stage may ask of a root.
    fn fields(&self) -> &'static [&'static str];
    /// The stages' outputs, one per pending demand.
    fn execute(
        &self,
        pending: Vec<(usize, super::producer::Demand)>,
        contexts: &mut [crate::code_rule_dsl::budget::Context],
        allowance: &mut super::producer::Allowance,
    ) -> Vec<Result<super::producer::Completed>>;
}

/// A canonical AST stage's output, published as its relation.
pub(in crate::code_rule_dsl) trait PublishedAst: Send {
    fn publish(self: Box<Self>, paths: Vec<String>, db: &Connection, name: &str, work: Arc<AtomicUsize>) -> Result<()>;
}

static PRODUCER: std::sync::OnceLock<&'static dyn AstProducer> = std::sync::OnceLock::new();

/// Registers the canonical AST producer (once, at load).
pub(in crate::code_rule_dsl) fn install_ast_producer(producer: &'static dyn AstProducer) {
    let _ = PRODUCER.set(producer);
}

pub(super) fn producer() -> Option<&'static dyn AstProducer> {
    PRODUCER.get().copied()
}

/// The fields a stage may ask of a root: none without a producer.
pub(in crate::code_rule_dsl) fn fields() -> &'static [&'static str] {
    producer().map_or(&[], |p| p.fields())
}
