//! One typed region input feeds atomic scope/usage facts; SQL retains composition.
use crate::code_rule_dsl::{
    literal::SubDoc,
    matcher::{Binds, sql_binds_region_facts::binds_region_budget, with_sql_clause_owner},
    sql::{
        plan::SlotKind,
        terminals::{Verdict, Verdicts, values_of},
    },
    tape::TapeDoc,
};
use ast_grep_core::{Doc, Node, meta_var::MetaVarEnv};
pub(super) struct RegionFacts {
    pub test: Binds,
    pub bindings: Vec<(String, SlotKind)>,
    pub owner: usize,
}
impl RegionFacts {
    fn evaluate<'t, D: Doc>(
        &self,
        node: Node<'t, D>,
        env: &MetaVarEnv<'t, D>,
        work: usize,
    ) -> (Result<Vec<Verdict>, String>, usize) {
        with_sql_clause_owner(self.owner, || {
            let (verdict, left) = binds_region_budget(&self.test, &node, env, work);
            (
                verdict.map(|verdict| match verdict {
                    Some(true) => vec![Verdict::holds(values_of(&self.bindings, env))],
                    Some(false) => vec![],
                    None => vec![Verdict::unknown()],
                }),
                left,
            )
        })
    }
}
impl Verdicts for RegionFacts {
    fn rows<'t>(
        &self,
        _: Node<'t, TapeDoc>,
        _: &MetaVarEnv<'t, TapeDoc>,
    ) -> Result<Vec<Verdict>, String> {
        Err("binding region facts require finite work".into())
    }
    fn budget_rows<'t>(
        &self,
        node: Node<'t, TapeDoc>,
        env: &MetaVarEnv<'t, TapeDoc>,
        work: usize,
    ) -> (Result<Vec<Verdict>, String>, usize) {
        self.evaluate(node, env, work)
    }
    fn tree_budget_rows<'t>(
        &self,
        node: Node<'t, SubDoc>,
        env: &MetaVarEnv<'t, SubDoc>,
        work: usize,
    ) -> (Result<Vec<Verdict>, String>, usize) {
        self.evaluate(node, env, work)
    }
    fn supports_tree_budget(&self) -> bool {
        true
    }
    fn supports_tree_rows(&self) -> bool {
        true
    }
}
