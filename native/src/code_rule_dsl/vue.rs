//! Vue single-file components (`.vue`): their blocks (`sfc`), template directives (`template`)
//! and the program a component embeds (`view`), which the tape carries (`embed::ScriptBlock`).
//! The grammar is the registry's (`GrammarDialect::Vue`, `vendored:vue`).
//!
//! A Vue tape holds the component's tree-sitter-vue tree and, as the root `document`'s last
//! child, the `program` its scripts and template expressions make (`view`), each node spanning
//! the `.vue` bytes it came from. Vue rules name Vue's kinds and ECMAScript's alike; an
//! ECMAScript node is only ever inside the `program`.
#[cfg(test)]
mod oracle_tests;
mod sfc;
mod template;
pub(crate) mod view;

pub(crate) use sfc::tape_script_dialect;

use ast_grep_core::{Doc, Node};

/// The program a Vue tape embeds: its root's last child, if that is a `program`. The dialect it
/// is written in is its script's `lang` (`tape_script_dialect`).
pub(crate) fn program<'r, D: Doc>(root: &Node<'r, D>) -> Option<Node<'r, D>> {
    root.children()
        .last()
        .filter(|last| last.kind() == "program" && last.is_named())
}

#[cfg(test)]
mod tests {
    use crate::code_rule_dsl::language::GrammarDialect;
    use ast_grep_language::LanguageExt;
    use std::collections::HashSet;

    #[test]
    fn parses_two_scripts_fixture() {
        let path = std::path::Path::new(crate::code_rule_dsl::fixtures_dir!())
            .join("vue/language/view/fixtures/two-scripts.vue");
        let source = std::fs::read_to_string(path).expect("Vue fixture");
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&GrammarDialect::Vue.get_ts_language())
            .expect("Vue grammar");
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
