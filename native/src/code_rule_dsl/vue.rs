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
    use crate::code_rule_dsl::scope::ScopeTable;
    use crate::code_rule_dsl::scope::usage::{BindingId, Usage};
    use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use ast_grep_core::AstGrep;
    use ast_grep_core::tree_sitter::StrDoc;
    use ast_grep_language::LanguageExt;
    use std::collections::HashSet;
    use std::sync::Arc;

    /// A Vue tape of `source`: the component's tree and its embedded program.
    fn tape(source: &str) -> AstGrep<TapeDoc> {
        let grammar = Grammar::new(GrammarDialect::Vue).expect("grammar");
        let native = StrDoc::new(source, GrammarDialect::Vue);
        let image = TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false)
            .expect("encodes");
        AstGrep::doc(
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false)
                .expect("restores"),
        )
    }

    /// The view writes `<p :c="k" v-if="a" />` as `if (a) { (k); }`, the condition first though
    /// its directive follows `:c`, and `<C v-for="i in l" />` as `(l), (i) => { C; }`, the tag
    /// last though it comes first: a template's bindings are read and used whatever the order of
    /// the program's spans (eslint-plugin-vue's no-unused-vars valid cases 015 and 017). Each
    /// case: the file and the binding its last occurrence of the name reads.
    #[test]
    fn usage_reads_a_program_whose_spans_run_out_of_order() {
        for (source, name) in [
            (
                r#"<template><R v-slot="{ k }"><p :c="k" v-if="a" /></R></template>"#,
                "k",
            ),
            (
                r#"<template><R v-slot="{ k }"><p v-if="a" :c="k" /></R></template>"#,
                "k",
            ),
            (
                r#"<template><R v-slot="{ k }"><p :c="b" v-if="a" :d="k" /></R></template>"#,
                "k",
            ),
            (
                r#"<template><R v-slot="{ C }"><div><C v-for="i in l" :key="i" /></div></R></template>"#,
                "C",
            ),
            (
                r#"<template><R #w="{ l, C }"><C v-for="(k, i) in l" :key="k" :i="i" /></R></template>"#,
                "C",
            ),
        ] {
            let doc = tape(source);
            let table = ScopeTable::build(doc.root(), GrammarDialect::Vue, usize::MAX, &|| false)
                .expect("table builds");
            let read = source.rfind(name).expect("a read");
            let reference = table
                .reference(&(read..read + name.len()))
                .unwrap_or_else(|| panic!("{name} is a reference in {source}"));
            assert!(
                table.resolve(reference, name).is_some(),
                "{name} resolves in {source}"
            );
            let scope = table
                .scopes
                .iter()
                .position(|scope| scope.bindings.contains_key(name))
                .expect("the name is bound") as u32;
            let usage = Usage::build(&table, &doc.root(), usize::MAX, &|| false).expect("usage");
            let facts = usage.facts(BindingId {
                scope,
                name,
                index: 0,
            });
            assert_eq!(
                (facts.referenced, facts.used, facts.references),
                (Some(true), Some(true), 1),
                "{name} in {source}"
            );
        }
    }

    /// ast-grep walks a node's later siblings from the first sibling that ends after its start
    /// (`next_all`): a `v-for` parameter's are the parameters after it, the alias's parentheses
    /// standing for no bytes (eslint-plugin-vue no-unused-vars invalid-007 to 009: a parameter
    /// before a read one counts as used, one after it does not).
    #[test]
    fn for_parameters_walk_to_their_later_siblings() {
        for alias in ["(a, b, c) in foo", "a, b, c in foo"] {
            let source = format!(r#"<template><div v-for="{alias}">{{{{ a }}}}</div></template>"#);
            let doc = tape(&source);
            let root = doc.root();
            let b = root
                .dfs()
                .find(|node| node.kind() == "identifier" && node.text() == "b")
                .expect("the parameter b");
            let later: Vec<String> = b
                .next_all()
                .filter(|node| node.is_named())
                .map(|node| node.text().into_owned())
                .collect();
            assert_eq!(later, ["c"], "{alias}");
        }
    }

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
