//! Capture additions replayed by a successful cohort utility memo entry.
use ast_grep_core::{
    Doc, Node,
    meta_var::{MetaVarEnv, MetaVariable},
    source::Content,
};
use std::{collections::BTreeSet, sync::Arc};

/// Added or changed bindings. Nodes retain their complete document identity and list order.
#[derive(Clone)]
pub(super) struct Delta<'t, D: Doc<Source: Content<Underlying = u8>>> {
    bindings: Vec<(String, Binding<'t, D>)>,
}
#[derive(Clone)]
enum Binding<'t, D: Doc<Source: Content<Underlying = u8>>> {
    Single(Node<'t, D>),
    Multi(Vec<Node<'t, D>>),
    Text(Vec<u8>),
}
fn same<D: Doc>(a: &Node<'_, D>, b: &Node<'_, D>) -> bool {
    std::ptr::eq(a.root(), b.root()) && a.node_id() == b.node_id()
}

/// Records the selected successful output bindings that differ from the input. Reserved span
/// names follow their capture's scope; the enumerated capture is always selected. `None` selects
/// every capture. This is only for successful verdicts: a miss or unknown reuses its current input.
/// Scope and document lifetimes must match the corresponding memo key.
pub(super) fn between<'t, D: Doc<Source: Content<Underlying = u8>>>(
    input: &MetaVarEnv<'t, D>,
    output: &MetaVarEnv<'t, D>,
    scope: Option<&[Arc<str>]>,
    excluded_name: Option<&str>,
) -> Delta<'t, D> {
    let keep = |name: &str| {
        let name = name.strip_prefix('\0').unwrap_or(name);
        scope.is_none_or(|names| names.iter().any(|n| n.as_ref() == name))
            || excluded_name == Some(name)
    };
    let mut names = BTreeSet::new();
    for variable in output.get_matched_variables() {
        match variable {
            MetaVariable::Capture(name, _) if keep(&name) => {
                names.insert((name, 0));
            }
            MetaVariable::MultiCapture(name) if keep(&name) => {
                names.insert((name, 1));
            }
            _ => {}
        }
    }
    let input_multiple_names: BTreeSet<_> = input
        .get_matched_variables()
        .filter_map(|variable| match variable {
            MetaVariable::MultiCapture(name) => Some(name),
            _ => None,
        })
        .collect();
    let mut bindings = Vec::new();
    for (name, kind) in names {
        if kind == 0 {
            if let Some(node) = output.get_match(&name) {
                if !input
                    .get_match(&name)
                    .is_some_and(|before| same(before, node))
                {
                    bindings.push((name.clone(), Binding::Single(node.clone())));
                }
            }
            if let Some(bytes) = output.get_transformed(&name) {
                if input.get_transformed(&name) != Some(bytes) {
                    bindings.push((name, Binding::Text(bytes.clone())));
                }
            }
        } else {
            let nodes = output.get_multiple_matches(&name);
            let before = input.get_multiple_matches(&name);
            // An empty binding is distinct from an absent binding.
            let present = input_multiple_names.contains(&name);
            if !present
                || nodes.len() != before.len()
                || !nodes.iter().zip(&before).all(|(a, b)| same(a, b))
            {
                bindings.push((name, Binding::Multi(nodes)));
            }
        }
    }
    Delta { bindings }
}

/// Replays additions into the caller's current input. Single and multiple node captures use the
/// native environment's unification; transformed bytes use its native overwrite behavior.
/// A failed unification leaves the caller's environment untouched.
pub(super) fn apply<'t, D: Doc<Source: Content<Underlying = u8>>>(
    delta: &Delta<'t, D>,
    current: Arc<MetaVarEnv<'t, D>>,
) -> Option<Arc<MetaVarEnv<'t, D>>> {
    if delta.bindings.is_empty() {
        return Some(current);
    }
    let mut output = current.as_ref().clone();
    for (name, binding) in &delta.bindings {
        match binding {
            Binding::Single(node) => {
                output.insert(name, node.clone())?;
            }
            Binding::Multi(nodes) => {
                output.insert_multi(name, nodes.clone())?;
            }
            Binding::Text(bytes) => {
                output.insert_transformation(&MetaVariable::Multiple, name, bytes.clone())
            }
        }
    }
    Some(Arc::new(output))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use ast_grep_core::tree_sitter::LanguageExt;

    #[test]
    fn replay_distinguishes_empty_multiple_bindings_from_absent_bindings() {
        let root = GrammarDialect::TypeScript.ast_grep("a;");
        let node = root.root();
        let mut input = MetaVarEnv::new();
        input.insert("X", node.clone()).unwrap();
        let mut output = input.clone();
        output.insert_multi("M", Vec::new()).unwrap();
        let delta = between(&input, &output, None, None);
        assert_eq!(delta.bindings.len(), 1);
        let replayed = apply(&delta, Arc::new(input)).unwrap();
        assert!(
            replayed.get_matched_variables().any(
                |variable| matches!(variable, MetaVariable::MultiCapture(name) if name == "M")
            )
        );
        assert!(replayed.get_multiple_matches("M").is_empty());
        let unchanged = between(&replayed, &output, None, None);
        assert!(unchanged.bindings.is_empty());
        let mut conflict = MetaVarEnv::new();
        conflict.insert_multi("M", vec![node]).unwrap();
        assert!(apply(&delta, Arc::new(conflict)).is_none());
    }

    #[test]
    fn replay_preserves_unrelated_captures_and_rejects_conflicting_node_bindings() {
        let root = GrammarDialect::TypeScript.ast_grep("a; b; c;");
        let nodes = root
            .root()
            .dfs()
            .filter(|n| n.kind() == "identifier")
            .collect::<Vec<_>>();
        let input = MetaVarEnv::new();
        let mut output = input.clone();
        output.insert("X", nodes[0].clone()).unwrap();
        let delta = between(&input, &output, Some(&[Arc::from("X")]), None);
        let mut current = MetaVarEnv::new();
        current.insert("Y", nodes[1].clone()).unwrap();
        let applied = apply(&delta, Arc::new(current)).unwrap();
        assert_eq!(applied.get_match("X").unwrap().text(), "a");
        assert_eq!(applied.get_match("Y").unwrap().text(), "b");
        let mut conflict = MetaVarEnv::new();
        conflict.insert("X", nodes[2].clone()).unwrap();
        let original = Arc::new(conflict);
        assert!(apply(&delta, original.clone()).is_none());
        assert_eq!(original.get_match("X").unwrap().text(), "c");
    }

    #[test]
    fn replay_preserves_reserved_exports_transformed_bytes_and_multiple_order() {
        let root = GrammarDialect::TypeScript.ast_grep("a; b;");
        let nodes = root
            .root()
            .dfs()
            .filter(|n| n.kind() == "identifier")
            .collect::<Vec<_>>();
        let input = MetaVarEnv::new();
        let mut output = input.clone();
        output.insert_transformation(&MetaVariable::Multiple, "E", b"decoded".to_vec());
        output.insert_transformation(&MetaVariable::Multiple, "\0E", b"2:7".to_vec());
        output
            .insert_multi("M", vec![nodes[1].clone(), nodes[0].clone()])
            .unwrap();
        output.insert_transformation(&MetaVariable::Multiple, "IGNORED", b"hidden".to_vec());
        let delta = between(&input, &output, Some(&[Arc::from("M")]), Some("E"));
        let result = apply(&delta, Arc::new(input)).unwrap();
        assert_eq!(result.get_transformed("E").unwrap(), b"decoded");
        assert_eq!(result.get_transformed("\0E").unwrap(), b"2:7");
        assert!(result.get_transformed("IGNORED").is_none());
        assert_eq!(
            result
                .get_multiple_matches("M")
                .iter()
                .map(|n| n.text().into_owned())
                .collect::<Vec<_>>(),
            ["b", "a"]
        );
        let mut conflict = MetaVarEnv::new();
        conflict
            .insert_multi("M", vec![nodes[0].clone(), nodes[1].clone()])
            .unwrap();
        assert!(apply(&delta, Arc::new(conflict)).is_none());
    }
}
