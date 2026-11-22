//! The relational admission's base, in the code rules' crate (rerail-rules-base): the compiled plan a relational
//! clause holds (`Plan`, its stages and their specs), which the model keeps (`ClauseKind::Relational`), and the
//! admission itself behind a registered trait (`Admission`), which the compiler and the evaluator call. The
//! admission (rerail-rules, `relational.rs`) compiles the plans and evaluates them, and registers itself
//! (`ADMISSION`, boot), as each test binary that compiles a relational clause does. `relational.rs` re-exports
//! this module.
use crate::code_rule_dsl::compile::define::Defines;
use crate::code_rule_dsl::model::{Capabilities, CompiledClause, CompiledCodeRule, Evaluation, FactQuery, Snapshot};
use napi::Result;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

/// The relational admission: compiles a clause's `relational` form and evaluates its clauses.
pub trait Admission: Send + Sync {
    /// The plan of a clause's `relational` form (`relational::compile::compile`).
    fn compile(
        &self,
        value: &Value,
        path: &str,
        capabilities: &Capabilities,
        defines: &Defines,
        required_ast: &mut BTreeSet<(String, String)>,
    ) -> Result<Plan>;
    /// The evaluations of relational clauses, one per job (`relational::evaluate_batch`), each over its snapshot
    /// alone: no gate, rows or facts handed.
    fn evaluate(&self, jobs: Vec<ClauseRun<'_>>) -> Vec<Evaluation>;
    /// The syntax facts' pool, installed for the guard's life (`relational::syntax::Pool::install`).
    fn syntax_pool(&self) -> Box<dyn std::any::Any>;
}

/// One relational clause to evaluate over a snapshot (`Admission::evaluate`).
pub struct ClauseRun<'a> {
    pub code_rule: &'a CompiledCodeRule,
    pub clause: &'a CompiledClause,
    pub snapshot: Snapshot,
    pub budget: crate::code_rule_dsl::budget::Context,
}

pub static ADMISSION: rerail_registry::Slot<&'static dyn Admission> =
    rerail_registry::Slot::new("the relational admission (residency::admission::relational::ADMISSION)");

/// The registered admission (`ADMISSION`).
pub fn registered() -> &'static dyn Admission {
    ADMISSION.get().unwrap_or_else(|missing| panic!("{missing}: no program registered the relational admission"))
}

#[derive(Clone)]
pub struct Stage {
    pub name: String,
    pub query: String,
    pub indexes: Vec<Vec<String>>,
    pub reachability: Option<reachability::Reachability>,
    pub python_ast: Option<ast_stage::Spec>,
    pub deferred: BTreeMap<String, String>,
    pub stored_columns: Vec<String>,
    pub output_columns: Vec<String>,
    pub declaration: Arc<str>,
}
#[derive(Clone)]
pub struct References {
    pub query: crate::code_rule_dsl::model::Query,
    pub key: crate::code_rule_dsl::calls::contracts::Key,
}

#[derive(Clone)]
pub struct Plan {
    pub require_nonempty: Option<usize>,
    pub syntax: Option<Arc<syntax::Spec>>,
    pub per_file: bool,
    pub references: Option<References>,
    pub locations: BTreeSet<String>,
    pub inputs: Vec<(String, FactQuery)>,
    pub stages: Vec<Stage>,
    pub query: String,
    pub key: Vec<String>,
    pub code: Option<String>,
    pub message: Option<String>,
    pub last_uses: BTreeMap<String, usize>,
    pub reads_source: bool,
}

/// The syntax stage's spec (`relational::syntax`).
pub mod syntax {
    use std::collections::BTreeMap;

    #[derive(Clone, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Owner {
        pub kind: String,
        #[serde(default)]
        pub outermost: bool,
    }
    #[derive(Clone, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct Spec {
        pub name: String,
        pub gate: String,
        #[serde(default)]
        pub fields: Vec<String>,
        #[serde(default)]
        pub columns: Vec<String>,
        #[serde(skip)]
        pub field_codes: BTreeMap<String, u16>,
        #[serde(default, rename = "excludeKinds")]
        pub exclude_kinds: Vec<String>,
        #[serde(default, rename = "emptyTextKinds")]
        pub empty_text_kinds: Vec<String>,
        pub edges: Option<String>,
        pub ownership: Option<String>,
        #[serde(default)]
        pub owners: BTreeMap<String, Owner>,
    }
}

/// A stage's reachability (`relational::reachability`).
pub mod reachability {
    #[derive(Clone)]
    pub struct Reachability {
        pub ordered_first: Option<super::ordered_first::Spec>,
        pub roots: String,
        pub edges: String,
        pub batch_size: usize,
        pub forest: bool,
        pub syntax: Option<String>,
        pub expand: bool,
    }
}

/// A reachability's ordered first match (`relational::ordered_first`).
pub mod ordered_first {
    #[derive(Clone)]
    pub struct Spec {
        pub sinks: String,
        pub on_edge: bool,
    }
}

/// A canonical AST stage (`relational::ast_stage`).
pub mod ast_stage {
    #[derive(Clone)]
    pub struct Spec {
        pub roots: String,
        pub syntax: String,
        pub fields: Vec<String>,
    }
}
