use super::*;
use crate::code_rule_dsl::language::{GrammarDialect, dialect_by_label};

/// A file as facts read it, with no request around it.
struct Text(String, Arc<String>);

impl SourceFile for Text {
    fn language(&self) -> &str {
        &self.0
    }
    fn source(&self) -> &Arc<String> {
        &self.1
    }
}

fn facts(path: &str, language: &str, source: &str) -> Facts {
    let file = Text(language.to_owned(), Arc::new(source.to_owned()));
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
    assert_eq!(f.start_line(*identifiers.last().unwrap()), 2);
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

#[test]
fn the_digest_is_of_the_content_not_the_path() {
    let a = digest(&facts("a.py", "python", "x = 1\n"));
    assert_eq!(a, digest(&facts("b.py", "python", "x = 1\n")));
    assert_ne!(a, digest(&facts("a.py", "python", "x  = 1\n")));
    // Same kinds and spans, other text: a reader of `text` sees the difference, so the digest does.
    assert_ne!(a, digest(&facts("a.py", "python", "y = 2\n")));
}

#[test]
fn direct_tape_columns_equal_restored_columns_and_line_contracts() {
    use crate::code_rule_dsl::{lines::{LineTable, Terminators}, tape::{Grammar, TapeDoc, TapeImage}};
    for (label, source) in [
        ("python", "é = f(1)\r\nclass A:\r def f(self):\n  return 'x'\n"),
        ("python", "def broken(:\n    pass\n"),
        ("python", ""),
        ("javascript", "const x = (a) => a?.b ?? `x${a}`; // c\n"),
        ("typescript", "interface A { x?: number }; const x: A = {};\n"),
        ("rust", "fn main() { let x = Some(1); if let Some(y) = x { println!(\"{y}\"); } }"),
        ("php", "<?php function f($x) { return $x + 1; }\n"),
        ("sql", "SELECT a, count(*) FROM t GROUP BY a;"),
        ("vue", "<template><div>{{ value }}</div></template><script>let value = 1;</script>"),
        ("blade", "@if($ok)<div>{{ $value }}</div>@endif"),
        ("tsx", "const A = () => <div title=\"x\">text</div>;"),
        ("rr", ".decl item(n:number)\nitem(1).\n.output item\n"),
    ] {
        let lang = dialect_by_label(label).unwrap();
        let grammar = Grammar::new(lang).unwrap();
        let image = TapeImage::parse(&grammar, source, &|| false).unwrap().unwrap();
        let source = Arc::new(source.to_owned());
        let lines = Arc::new(LineTable::new(&source, Terminators::of(lang)));
        let doc = TapeDoc::restore(Arc::clone(&source), Arc::clone(&grammar), &image.primary, &|| false).unwrap();
        let old = from_tape(&doc, Arc::clone(&lines)).unwrap();
        let new = from_image(Arc::clone(&source), &grammar, &image.primary, Arc::clone(&lines)).unwrap();
        for (a,b) in [(&old.kind,&new.kind),(&old.parent,&new.parent),(&old.start,&new.start),(&old.end,&new.end),
            (&old.subtree_end,&new.subtree_end),(&old.children,&new.children),(&old.child_offsets,&new.child_offsets),
            (&old.kind_keys,&new.kind_keys),(&old.kind_offsets,&new.kind_offsets),(&old.kind_nodes,&new.kind_nodes),
            (&old.class_keys,&new.class_keys),(&old.class_offsets,&new.class_offsets),(&old.class_nodes,&new.class_nodes)] {
            assert_eq!(a,b,"{label}: {source}");
        }
        assert_eq!(old.flags,new.flags,"{label}");
        assert_eq!(old.field,new.field,"{label}");
        assert_eq!(old.syntax_errors,new.syntax_errors,"{label}");
        assert_eq!(digest(&old),digest(&new),"{label}");
        for id in 0..new.len() as u32 { assert_eq!(new.start_line(id), lines.line(new.start[id as usize] as usize) as u32 + 1); }
    }
}
