//! The physical plan's node types (agreed with S2, who constructs them), their `EXPLAIN` text
//! and the plan audit (INVARIANTS I5): plans are compiled once per rule, printed stably, and
//! a plan that scans a relation per outer row (an inner scan) is refused.
//!
//! A select runs as a `Pipeline` of FROM-order items: the first item is scanned; every later
//! item must be reached by a key: an index nested loop (hash index built once per table per
//! statement, probed per binding, candidates in table order), an OR probe (union of branch
//! lookups, sorted and deduplicated, the OR re-checked), a lateral `json_each`, or a constant
//! relation. Correlated subqueries probe the same indexes; uncorrelated ones run once per
//! statement.
use std::fmt::Write;

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// A full scan of `rel`, with the conjuncts placed at this level.
    Scan { rel: String, pred: Vec<String> },
    /// Probe a hash index on `key_cols` of `rel` with the binding's values.
    IndexNestedLoop { rel: String, key_cols: Vec<u16>, key_affs: Vec<String>, residual: Vec<String> },
    /// Union of one index probe per OR branch, in table order, the OR re-checked.
    OrIndexProbe { rel: String, branches: Vec<Vec<u16>>, residual: Vec<String> },
    /// `json_each(arg)` over the binding.
    LateralJsonEach { arg: String },
    /// A relation known to hold at most a constant number of rows (literals, `Unit`).
    Constant { rel: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct PipelineItem {
    pub item: Item,
    /// `LEFT JOIN`: a binding with no match is null-extended.
    pub left: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Pipeline { items: Vec<PipelineItem> },
    SemiJoin { correlated: bool },
    AntiJoin { correlated: bool },
    Scalar { correlated: bool },
    Group { keys: Vec<String>, aggs: Vec<String> },
    Window { partition: Vec<String>, order: Vec<String> },
    Sort { keys: Vec<String>, limit: Option<String>, offset: Option<String> },
    Union { all: bool },
    Distinct { prefix: u16 },
    Fixpoint { seminaive: bool, set: bool },
    Project { exprs: Vec<String> },
    /// Reserved for rewrite R-tree-interval: descendants of `root_col` as `(id, subtree_end]`.
    IntervalProbe { root_col: u16 },
    /// Reserved: a per-node predicate memoised by its rule digest.
    MemoPredicate { digest: String },
}

/// A physical node: its operator, the digest of the logical relation it implements, and its
/// inputs (subqueries of a pipeline are children after its items' inputs).
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub op: Op,
    pub digest: String,
    pub children: Vec<Node>,
}

fn list(v: &[String]) -> String {
    v.join(", ")
}

fn item_text(i: &PipelineItem) -> String {
    let l = if i.left { "left " } else { "" };
    match &i.item {
        Item::Scan { rel, pred } => format!("{l}scan {rel}{}", if pred.is_empty() { String::new() } else { format!(" where {}", list(pred)) }),
        Item::IndexNestedLoop { rel, key_cols, key_affs, residual } => format!(
            "{l}index {rel} on ({}) affs ({}){}",
            key_cols.iter().map(|c| format!("#{c}")).collect::<Vec<_>>().join(", "),
            list(key_affs),
            if residual.is_empty() { String::new() } else { format!(" where {}", list(residual)) }
        ),
        Item::OrIndexProbe { rel, branches, residual } => format!(
            "{l}or-probe {rel} branches ({}){}",
            branches.iter().map(|b| b.iter().map(|c| format!("#{c}")).collect::<Vec<_>>().join("+")).collect::<Vec<_>>().join(" | "),
            if residual.is_empty() { String::new() } else { format!(" where {}", list(residual)) }
        ),
        Item::LateralJsonEach { arg } => format!("{l}json_each({arg})"),
        Item::Constant { rel } => format!("{l}constant {rel}"),
    }
}

fn op_text(op: &Op) -> String {
    match op {
        Op::Pipeline { .. } => "Pipeline".into(),
        Op::SemiJoin { correlated } => format!("SemiJoin{}", if *correlated { " correlated" } else { " cached" }),
        Op::AntiJoin { correlated } => format!("AntiJoin{}", if *correlated { " correlated" } else { " cached" }),
        Op::Scalar { correlated } => format!("Scalar{}", if *correlated { " correlated" } else { " cached" }),
        Op::Group { keys, aggs } => format!("Group keys ({}) aggs ({})", list(keys), list(aggs)),
        Op::Window { partition, order } => format!("Window partition ({}) order ({})", list(partition), list(order)),
        Op::Sort { keys, limit, offset } => format!("Sort ({}){}{}", list(keys), limit.as_ref().map(|l| format!(" limit {l}")).unwrap_or_default(), offset.as_ref().map(|o| format!(" offset {o}")).unwrap_or_default()),
        Op::Union { all } => format!("Union{}", if *all { " all" } else { "" }),
        Op::Distinct { prefix } => format!("Distinct prefix {prefix}"),
        Op::Fixpoint { seminaive, set } => format!("Fixpoint{}{}", if *seminaive { " semi-naive" } else { "" }, if *set { " set" } else { " bag" }),
        Op::Project { exprs } => format!("Project ({})", list(exprs)),
        Op::IntervalProbe { root_col } => format!("IntervalProbe root #{root_col}"),
        Op::MemoPredicate { digest } => format!("MemoPredicate {digest}"),
    }
}

/// Stable `EXPLAIN` text of a physical plan.
pub fn explain(n: &Node) -> String {
    let mut out = String::new();
    fn walk(n: &Node, indent: usize, out: &mut String) {
        let _ = writeln!(out, "{:indent$}{} [{}]", "", op_text(&n.op), n.digest);
        if let Op::Pipeline { items } = &n.op {
            for (k, i) in items.iter().enumerate() {
                let _ = writeln!(out, "{:w$}{k}: {}", "", item_text(i), w = indent + 2);
            }
        }
        for c in &n.children {
            walk(c, indent + 2, out);
        }
    }
    walk(n, 0, &mut out);
    out
}

/// The plan audit: every refused placement, named. An inner scan is a full `Scan` at a
/// pipeline item after the first (it runs once per binding), or a correlated subquery whose
/// pipeline starts with a full scan (it runs once per outer row).
pub fn audit(n: &Node) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(n: &Node, correlated: bool, path: &str, out: &mut Vec<String>) {
        if let Op::Pipeline { items } = &n.op {
            for (k, i) in items.iter().enumerate() {
                if let Item::Scan { rel, .. } = &i.item {
                    // Mutant 8: the audit admits an inner scan.
                    if (k > 0 || correlated) && !crate::mutant::on(8) {
                        out.push(format!("{path}: inner scan of {rel} at item {k}{}", if correlated && k == 0 { " of a correlated subquery" } else { "" }));
                    }
                }
            }
        }
        // A correlated subquery runs once per outer row down to its first pipeline (through the
        // projections, sorts and groups on top of it).
        let child_corr = matches!(n.op, Op::SemiJoin { correlated: true } | Op::AntiJoin { correlated: true } | Op::Scalar { correlated: true }) || (correlated && !matches!(n.op, Op::Pipeline { .. }));
        for (j, c) in n.children.iter().enumerate() {
            walk(c, child_corr, &format!("{path}/{j}"), out);
        }
    }
    walk(n, false, "plan", &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pipeline(items: Vec<Item>) -> Node {
        Node { op: Op::Pipeline { items: items.into_iter().map(|item| PipelineItem { item, left: false }).collect() }, digest: "d".into(), children: vec![] }
    }

    #[test]
    fn audit_refuses_inner_scan() {
        let ok = pipeline(vec![Item::Scan { rel: "nodes".into(), pred: vec![] }, Item::IndexNestedLoop { rel: "edges".into(), key_cols: vec![1], key_affs: vec!["None".into()], residual: vec![] }]);
        assert!(audit(&ok).is_empty());
        let bad = pipeline(vec![Item::Scan { rel: "nodes".into(), pred: vec![] }, Item::Scan { rel: "edges".into(), pred: vec!["#1 = #5".into()] }]);
        assert_eq!(audit(&bad).len(), 1);
        let corr = Node { op: Op::SemiJoin { correlated: true }, digest: "e".into(), children: vec![pipeline(vec![Item::Scan { rel: "calls".into(), pred: vec![] }])] };
        assert_eq!(audit(&corr).len(), 1);
        // Through a projection on top of the subquery's pipeline.
        let deep = Node { op: Op::SemiJoin { correlated: true }, digest: "e".into(), children: vec![Node { op: Op::Project { exprs: vec![] }, digest: "p".into(), children: vec![pipeline(vec![Item::Scan { rel: "calls".into(), pred: vec![] }])] }] };
        assert_eq!(audit(&deep).len(), 1);
        let probed = Node { op: Op::SemiJoin { correlated: true }, digest: "e".into(), children: vec![pipeline(vec![Item::IndexNestedLoop { rel: "calls".into(), key_cols: vec![0], key_affs: vec!["None".into()], residual: vec![] }])] };
        assert!(audit(&probed).is_empty());
        let cached = Node { op: Op::SemiJoin { correlated: false }, digest: "e".into(), children: vec![pipeline(vec![Item::Scan { rel: "calls".into(), pred: vec![] }])] };
        assert!(audit(&cached).is_empty());
    }

    #[test]
    fn audit_explain_is_stable() {
        let n = Node { op: Op::Project { exprs: vec!["#0".into()] }, digest: "p".into(), children: vec![pipeline(vec![Item::Scan { rel: "a".into(), pred: vec![] }, Item::LateralJsonEach { arg: "#2".into() }])] };
        assert_eq!(explain(&n), explain(&n.clone()));
        assert_eq!(explain(&n), "Project (#0) [p]\n  Pipeline [d]\n    0: scan a\n    1: json_each(#2)\n");
    }
}
