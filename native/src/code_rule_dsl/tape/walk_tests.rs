//! A walk over a named-only document reaches the nodes a full document's walk does (`walk`).

use super::{Grammar, TapeDoc, TapeImage};
use crate::code_rule_dsl::language::GrammarDialect;
use std::sync::Arc;

/// Every walk node, by kind and span, with its parent, children, next siblings (any and named),
/// descendants and named-leaf test, each node named by kind and span.
fn walk(doc: &TapeDoc) -> Vec<String> {
    let name = |node: u32| format!("{}@{:?}", doc.transducer_kind(node), doc.transducer_span(node));
    let mut out: Vec<String> = (0..doc.transducer_len() as u32)
        .map(|node| {
            let mut below: Vec<String> = doc.transducer_descendants(node).chain(doc.transducer_descendant_tokens(node)).map(name).collect();
            below.sort();
            format!(
                "{} f{} x{} parent {:?} children {:?} next {:?} named {:?} below {below:?} leaf {}",
                name(node),
                doc.transducer_field(node),
                doc.transducer_flags(node),
                doc.transducer_parent(node).map(name),
                doc.transducer_children(node).map(name).collect::<Vec<_>>(),
                doc.transducer_next_sibling(node, false).map(name),
                doc.transducer_next_sibling(node, true).map(name),
                doc.transducer_is_named_leaf(node),
            )
        })
        .collect();
    out.sort();
    out
}

#[test]
fn a_walk_over_a_named_only_document_reaches_what_a_full_one_does() {
    for (lang, source) in [
        (GrammarDialect::Python, "async def f(a, b=1):  # c\n    try:\n        return a if a is not b else [x for x in b]\n    except:\n        pass\n"),
        (GrammarDialect::TypeScript, "const f = async (a, b?) => { let x = [1,,2]; return a?.b ?? 0; };\n"),
        (GrammarDialect::Rust, "fn f(x: &[u8]) -> Option<u8> { if let Some(y) = x.first() { return Some(*y + 1); } None }\n"),
        (GrammarDialect::Php, "<?php\nFUNCTION f($a, $b = [1, 2,]) { return $a ?? $b; }\n"),
    ] {
        let native = ast_grep_core::tree_sitter::StrDoc::new(source, lang);
        let grammar = Grammar::new(lang).unwrap();
        let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
        let source = Arc::new(source.to_owned());
        let full = TapeDoc::restore(Arc::clone(&source), Arc::clone(&grammar), &image.primary, &|| false).unwrap();
        let named = TapeDoc::restore_named(source, grammar, &image.primary, &|| false).unwrap();
        assert!(named.transducer_len() < full.transducer_len(), "{lang:?}");
        let walking = named.walking_tokens();
        assert_eq!(walking.transducer_len(), full.transducer_len(), "{lang:?}");
        assert_eq!(walk(&walking), walk(&full.walking_tokens()), "{lang:?}");
    }
}
