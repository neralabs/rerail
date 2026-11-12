    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use ast_grep_core::AstGrep;
    use ast_grep_core::tree_sitter::StrDoc;
    use std::sync::Arc;

    /// The named queries answer as on tree-sitter's tree, and the token queries read what the
    /// tape stores: each node's parts are tree-sitter's less the tokens the writer leaves out, with
    /// tree-sitter's spans, and no part is made up.
    #[test]
    fn shape_queries_agree_on_native_trees_and_restored_tapes() {
        for (lang, source) in [
            (GrammarDialect::Python, "async def f(a, b=1):  # c\n    return a if a is not b else [x for x in b]\n"),
            (GrammarDialect::TypeScript, "const f = async (a, b?) => { let x = [1,,2]; return a?.b ?? 0; };\n"),
            (GrammarDialect::Rust, "fn f(x: &[u8]) -> Option<u8> { if let Some(y) = x.first() { return Some(*y + 1); } None }\n"),
            (GrammarDialect::Php, "<?php\nFUNCTION f($a, $b = [1, 2,]) { return $a ?? $b; }\n"),
        ] {
            let native = StrDoc::new(source, lang);
            let grammar = Grammar::new(lang).unwrap();
            let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap();
            let tape = TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap();
            let stored: Vec<Vec<(String, std::ops::Range<usize>, bool)>> = {
                let mut out = Vec::new();
                let mut stack = vec![native.tree.root_node()];
                while let Some(node) = stack.pop() {
                    let children = crate::code_rule_dsl::tape::stored_children_for_test(lang, node);
                    if node.is_named() {
                        out.push(children.iter().map(|(child, _)| (child.kind().to_owned(), child.byte_range(), !child.is_named())).collect());
                    }
                    stack.extend(children.into_iter().rev().map(|(child, _)| child));
                }
                out
            };
            let (native, tape) = (AstGrep::doc(native), AstGrep::doc(tape));
            assert_eq!(named(tape.root()), named(native.root()), "{lang:?}");
            let tape_parts: Vec<_> = parts(tape.root()).into_iter().map(|(_, parts)| parts).collect();
            assert_eq!(tape_parts, stored, "{lang:?}");
            let all: usize = parts(native.root()).iter().flat_map(|(_, parts)| parts).filter(|part| part.2).count();
            let kept: usize = tape_parts.iter().flatten().filter(|part| part.2).count();
            assert!(kept < all, "{lang:?}: the tape leaves tokens out, and no query makes them up");
        }
    }

    /// Each named node's parts: kind, span, and whether a token.
    fn parts<D: Doc>(root: Node<'_, D>) -> Vec<(String, Vec<(String, std::ops::Range<usize>, bool)>)> {
        root.dfs()
            .filter(|node| node.is_named())
            .map(|node| (format!("{} {:?}", node.kind(), node.range()), node.parts().map(|part| (part.kind().into_owned(), part.range(), part.token().is_some())).collect()))
            .collect()
    }

    /// Each named node's named children, as each query reads them.
    fn named<D: Doc>(root: Node<'_, D>) -> Vec<String> {
        root.dfs()
            .filter(|node| node.is_named())
            .map(|node| {
                let named: Vec<_> = node.named().map(|n| n.range()).collect();
                let code: Vec<_> = node.named_code().map(|n| n.range()).collect();
                let first = node.first_named().map(|n| n.range());
                let last = node.last_named().map(|n| n.range());
                format!("{} {:?} {named:?} {code:?} {first:?} {last:?}", node.kind(), node.range())
            })
            .collect()
    }
