//! Scope-sensitive leaves retain immutable clause ownership without changing SQL signatures.
use crate::code_rule_dsl::{
    matcher::with_sql_clause_owner,
    sql::terminals::{Leaf, Verdict, Verdicts},
    tape::TapeDoc,
};
use ast_grep_core::{Node, meta_var::MetaVarEnv};

pub(super) struct OwnedLeaf {
    pub leaf: Leaf,
    pub owner: usize,
}

impl Verdicts for OwnedLeaf {
    fn budget_verdict(
        &self,
        node: Node<'_, TapeDoc>,
        work: usize,
    ) -> (Result<Option<bool>, String>, usize) {
        with_sql_clause_owner(self.owner, || self.leaf.budget_verdict(node, work))
    }

    fn supports_tree_budget(&self) -> bool {
        true
    }

    fn tree_budget_verdict(
        &self,
        node: Node<'_, crate::code_rule_dsl::literal::SubDoc>,
        work: usize,
    ) -> (Result<Option<bool>, String>, usize) {
        with_sql_clause_owner(self.owner, || self.leaf.tree_budget_verdict(node, work))
    }

    fn rows<'t>(
        &self,
        node: Node<'t, TapeDoc>,
        env: &MetaVarEnv<'t, TapeDoc>,
    ) -> Result<Vec<Verdict>, String> {
        with_sql_clause_owner(self.owner, || self.leaf.rows(node, env))
    }
}
