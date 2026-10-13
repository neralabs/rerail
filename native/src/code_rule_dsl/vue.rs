//! Vue single-file components (`.vue`): the vendored SFC grammar root.
#[cfg(test)]
mod oracle_tests;
mod sfc;
mod template;
pub(crate) mod view;
pub(crate) fn grammar() -> ast_grep_core::tree_sitter::TSLanguage {
    unsafe extern "C" {
        fn tree_sitter_vue() -> *const ();
    }
    // SAFETY: `tree_sitter_vue` is the generated parser's valid language symbol.
    unsafe { tree_sitter_language::LanguageFn::from_raw(tree_sitter_vue) }.into()
}

#[cfg(test)]
mod tests {
    use super::grammar;
    use std::collections::HashSet;

    #[test]
    fn parses_two_scripts_fixture() {
        let path = std::path::Path::new(crate::code_rule_dsl::fixtures_dir!())
            .join("vue/language/view/fixtures/two-scripts.vue");
        let source = std::fs::read_to_string(path).expect("Vue fixture");
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&grammar()).expect("Vue grammar");
        let tree = parser.parse(source, None).expect("Vue tree");
        let root = tree.root_node();
        assert_eq!(root.kind(), "document");
        assert!(!root.has_error());
        let kinds: HashSet<_> = root
            .named_children(&mut root.walk())
            .map(|n| n.kind())
            .collect();
        for expected in ["script_element", "template_element", "style_element"] {
            assert!(kinds.contains(expected), "missing {expected}");
        }
    }
}
