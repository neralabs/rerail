use super::*;
use crate::code_rule_dsl::language::{GrammarDialect, dialect_by_label};

fn facts(path: &str, language: &str, source: &str) -> Facts {
    let file = FileInput::new(path.to_owned(), language.to_owned(), Arc::new(source.to_owned()), None, None, "core")
        .unwrap_or_else(|e| panic!("{}", e.reason));
    match build(&file).unwrap() {
        Built::Facts(facts) => facts,
        Built::Unparsed => panic!("{path} did not parse"),
    }
}

fn atom(language: &str, kind: &str) -> u32 {
    let lang: GrammarDialect = dialect_by_label(language).unwrap();
    kind_atom(tape_tag(lang).unwrap(), lang.kinds().kind_id(kind))
}

#[test]
fn columns_are_a_preorder_tree_with_children_and_kind_postings() {
    let f = facts("a.py", "python", "value = left + right\nprint(value)\n");
    assert_eq!(f.parent[0], NONE);
    for id in 1..f.len() as u32 {
        let parent = f.parent(id).unwrap();
        assert!(parent < id, "a parent precedes its children in preorder");
        assert!(f.children(parent).contains(&id));
        assert!(id < f.subtree_end[parent as usize]);
        assert!(f.start[parent as usize] <= f.start[id as usize] && f.end[id as usize] <= f.end[parent as usize]);
    }
    let identifiers = f.of_kind(atom("python", "identifier"));
    assert_eq!(
        identifiers.iter().map(|&id| f.text(id)).collect::<Vec<_>>(),
        ["value", "left", "right", "print", "value"]
    );
    assert_eq!(f.start_line[*identifiers.last().unwrap() as usize], 2);
    assert!(f.of_kind(atom("python", "binary_operator")).iter().all(|&id| f.named(id)));
    assert!(!f.syntax_errors);
    assert!(f.depth() >= 4);
}

#[test]
fn abstract_classes_post_the_nodes_the_tape_records() {
    let f = facts("a.py", "python", "value = left + right\n");
    let expressions = f.of_class(atom("python", "expression"));
    let binary = f.of_kind(atom("python", "binary_operator"));
    assert_eq!(binary.len(), 1);
    assert!(expressions.contains(&binary[0]));
    // A class is not a concrete kind: no node has it as its own kind.
    assert!(f.of_kind(atom("python", "expression")).is_empty());
}

#[test]
fn every_dialect_reads_its_own_grammar_and_kinds_never_collide() {
    let python = facts("a.py", "python", "f(x)\n");
    let typescript = facts("a.ts", "typescript", "f(x);\n");
    assert_eq!(typescript.of_kind(atom("typescript", "call_expression")).len(), 1);
    assert_eq!(python.of_kind(atom("python", "call")).len(), 1);
    let (p, t) = (python.kind[0] / KIND_STRIDE, typescript.kind[0] / KIND_STRIDE);
    assert_ne!(p, t);
    assert!(python.kind.iter().all(|kind| kind / KIND_STRIDE == p));
}
