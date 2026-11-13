    use super::*;
    use ast_grep_core::AstGrep;
    use ast_grep_core::tree_sitter::LanguageExt;

    fn tree(source: &str) -> Arc<SubTree> {
        let root = GrammarDialect::Python.ast_grep(source);
        let node = root
            .root()
            .dfs()
            .find(|node| node.kind() == "string")
            .expect("a string");
        match crate::code_rule_dsl::literal::python_string::parse_string(&node, CHARS, &mut |_| true) {
            Some(Parsed::Tree(tree)) => tree,
            _ => panic!("parses"),
        }
    }

    #[test]
    fn sub_trees_are_documents_with_mapped_nodes() {
        let source = "x = 'a\\x62c'";
        let tree = tree(source);
        let root = AstGrep::doc(SubDoc { tree: tree.clone() });
        let root = root.root();
        assert_eq!(root.kind(), "text");
        assert_eq!(root.text(), "abc");
        let chars: Vec<_> = root.children().collect();
        assert_eq!(chars.len(), 3);
        assert_eq!(chars[1].text(), "b");
        assert_eq!(
            chars[1].prev().map(|node| node.text().into_owned()),
            Some("a".into())
        );
        assert_eq!(
            chars[1].next().map(|node| node.text().into_owned()),
            Some("c".into())
        );
        assert_eq!(
            chars[2].parent().map(|node| node.kind().into_owned()),
            Some("text".into())
        );
        let host = crate::code_rule_dsl::origin::Origin::of_node(
            &*tree,
            chars[1].node_id(),
            chars[1].range(),
        )
        .expect("mapped")
        .primary();
        assert_eq!(&source[host], "\\x62");
        assert_eq!(
            tree.value(chars[2].node_id()),
            Some(&Value::Str("c".into()))
        );
        assert!(sub_tree(&chars[0]).is_some());
        assert!(chars[0].node_id() & ID_FLAG != 0);
        assert_eq!(
            chars.iter().map(|node| node.range()).collect::<Vec<_>>(),
            [0..1, 1..2, 2..3]
        );
    }
