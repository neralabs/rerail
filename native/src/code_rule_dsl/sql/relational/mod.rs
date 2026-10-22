//! Direct DSL-to-relation compiler. Test-only until the transition's semantic coverage gate.
//! No frame IR, SQL reparsing, rule-ID recognizers, or native composition callbacks.
mod analysis;
mod emit;
mod ir;
mod lower;
mod physical;
pub(super) mod tests;

use super::terminals::Terminal;
use crate::code_rule_dsl::matcher::GrammarMatcher;

pub(super) struct Compiled {
    pub sql: String,
    pub terminals: Vec<Terminal>,
    pub relations: serde_json::Value,
}

pub(super) fn compile(grammar: &GrammarMatcher) -> Result<Compiled, String> {
    analysis::admit(grammar)?;
    let program = lower::compile(grammar)?;
    let plan = physical::plan(&program)?;
    let relations = serde_json::json!({
        "scope": "capture-free, total, neighbor relations and positive parent recursion",
        "key": ["file", "node"],
        "captureReads": [], "captureWrites": [], "multiplicity": "set",
        "outcomes": "two-valued", "nativeComposition": false,
        "limitations": ["Native utility work and stack/depth budgets are not reproduced by this test-only kernel."],
        "relations": program.relations.iter().enumerate().map(|(id, r)| serde_json::json!({
            "id":id, "source":r.source, "schema":ir::SCHEMA,
            "operator":format!("{:?}",r.expression),
            "arms":plan[id].len(),
            "recursive":plan[id].iter().any(|a| a.references.iter().any(|(_,r)| *r==id)),
        })).collect::<Vec<_>>()
    });
    Ok(Compiled {
        sql: emit::emit(&program, &plan)?,
        terminals: program.terminals,
        relations,
    })
}
