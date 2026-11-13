use super::super::python_facts;
use crate::code_rule_dsl::tape::TapeDoc;
use ast_grep_core::{Doc, Node};
fn unparen<'t, D: Doc>(mut node: Node<'t, D>) -> Node<'t, D> {
    while node.kind() == "parenthesized_expression" {
        let Some(child) = node.children().find(|n| {
            n.is_named() && !n.kind().ends_with("comment") && n.kind() != "line_continuation"
        }) else {
            break;
        };
        node = child;
    }
    node
}
fn at<'t>(root: &Node<'t, TapeDoc>, pre: usize) -> Node<'t, TapeDoc> {
    root.root().adopt(root.get_doc().node(pre as u32))
}
/// Scope tables name targets by source span and kind. Tape starts are monotone
/// in preorder, so this locates a target without a global range-index copy.
fn target_at<'t>(
    root: &Node<'t, TapeDoc>,
    target: python_facts::Target,
) -> Option<Node<'t, TapeDoc>> {
    let tape = root.get_doc();
    let (mut lo, mut hi) = (0, tape.transducer_len());
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if tape.transducer_span(mid as u32).start < target.range.0 {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    while lo < tape.transducer_len() {
        let span = tape.transducer_span(lo as u32);
        if span.start != target.range.0 {
            break;
        }
        if span.end == target.range.1 && tape.transducer_kind(lo as u32) == target.kind {
            return Some(at(root, lo));
        }
        lo += 1;
    }
    None
}

pub fn value_origins<'t>(
    owner: Node<'t, TapeDoc>,
    binding: Option<&python_facts::BindingProjection>,
    root: &Node<'t, TapeDoc>,
    with_context: bool,
) -> Vec<Node<'t, TapeDoc>> {
    let owner = unparen(owner);
    let mut out = vec![owner.clone()];
    let Some(binding) = binding else { return out };
    let find = |target: python_facts::Target| target_at(root, target);
    if let Some(value) = binding.value.and_then(find) {
        out.push(unparen(value));
    }
    if let Some(site) = binding.site.and_then(find) {
        if site.kind() == "assignment" {
            if let (Some(left), Some(right)) = (site.field("left"), site.field("right")) {
                let list = |node: &Node<'_, TapeDoc>| {
                    matches!(
                        node.kind().as_ref(),
                        "list"
                            | "tuple"
                            | "set"
                            | "dictionary"
                            | "argument_list"
                            | "parameters"
                            | "lambda_parameters"
                            | "list_pattern"
                            | "tuple_pattern"
                            | "pattern_list"
                            | "expression_list"
                    )
                };
                // Catalog fallback: last direct name of this text and the value at
                // the same comma index. It intentionally includes set displays.
                let mut comma = 0;
                let mut target = None;
                for child in left.children() {
                    if child.kind() == "identifier" && child.text() == owner.text() {
                        target = Some(comma);
                    }
                    if !child.is_named() && child.text() == "," {
                        comma += 1;
                    }
                }
                if let Some(target) = target.filter(|_| list(&left) && list(&right)) {
                    let mut comma = 0;
                    for child in right.children() {
                        if child.is_named() && !child.kind().ends_with("comment") && comma == target
                        {
                            out.push(unparen(child.clone()));
                        }
                        if !child.is_named() && child.text() == "," {
                            comma += 1;
                        }
                    }
                }
            }
        }
        if with_context
            && site.kind() == "as_pattern"
            && site.parent().is_some_and(|p| p.kind() == "with_item")
        {
            let children: Vec<_> = site.children().collect();
            let plain = children
                .iter()
                .find(|n| n.kind() == "as_pattern_target")
                .and_then(|n| {
                    n.children()
                        .find(|n| n.is_named() && !n.kind().ends_with("comment"))
                })
                .is_some_and(|n| unparen(n).kind() == "identifier");
            if plain {
                for child in children.iter().take_while(|n| n.text() != "as") {
                    if child.is_named() && !child.kind().ends_with("comment") {
                        out.push(unparen(child.clone()));
                    }
                }
            }
        }
    }
    out.sort_by_key(Node::node_id);
    out.dedup_by_key(|n| n.node_id());
    out
}
