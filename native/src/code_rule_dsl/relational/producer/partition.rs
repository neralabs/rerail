//! Physical input quanta for one logical subtree relation. SQL retains its complete scope.
use super::*;
use std::collections::HashMap;

pub(in crate::code_rule_dsl::relational) fn execute(
    index: usize,
    demand: Demand,
    contexts: &mut [crate::code_rule_dsl::budget::Context],
    allowance: &mut Allowance,
    limit: usize,
) -> Result<Completed> {
    if demand.python_ast.is_some() {
        return Err(invalid("producer capacity exceeded"));
    }
    let original_paths = demand.paths.clone();
    let mut completed = Completed {
        output: Output {
            roots: Vec::new(),
            nodes: Vec::new(),
            files: Vec::new(),
        },
        paths: original_paths,
        python_ast: None,
    };
    let mut cursor = 0;
    while cursor < demand.roots.len() {
        let mut part = Demand {
            roots: Vec::new(),
            trees: Vec::new(),
            paths: Vec::new(),
            work: Arc::clone(&demand.work),
            python_ast: None,
        };
        let mut files = HashMap::new();
        let mut original_ids = Vec::new();
        let mut bytes = 0usize;
        while let Some(root) = demand.roots.get(cursor) {
            let original = root.file as usize;
            let tree = demand
                .trees
                .get(original)
                .ok_or_else(|| invalid("producer file identity"))?;
            let extra = 24usize.saturating_add(if files.contains_key(&original) {
                0
            } else {
                tree.facts
                    .subtree_ends()
                    .len()
                    .saturating_add(tree.visible.len())
                    .saturating_mul(4)
                    .saturating_add(16)
            });
            if bytes.saturating_add(extra) > limit {
                break;
            }
            bytes += extra;
            let file = *files.entry(original).or_insert_with(|| {
                let local = part.trees.len() as u32;
                part.trees.push(tree.clone());
                part.paths.push(demand.paths[original].clone());
                original_ids.push(root.file);
                local
            });
            part.roots.push(Root {
                file,
                node: root.node,
                expand: root.expand,
            });
            cursor += 1;
        }
        if part.roots.is_empty() {
            return Err(invalid("producer capacity exceeded"));
        }
        allowance.clear_resident();
        let next = execute_subtrees(vec![(index, part)], contexts, allowance)
            .pop()
            .expect("one physical producer")?;
        capacity(
            completed
                .output
                .roots
                .len()
                .checked_add(next.output.roots.len())
                .and_then(|n| n.checked_mul(12)),
            MAX_OUTPUT_BYTES,
        )?;
        completed.output.roots.extend(next.output.roots);
        completed.output.nodes.extend(next.output.nodes);
        completed.output.files.extend(
            next.output
                .files
                .into_iter()
                .map(|file| original_ids[file as usize]),
        );
    }
    Ok(completed)
}
