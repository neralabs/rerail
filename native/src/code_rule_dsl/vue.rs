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

use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::language::adapter::regions;
use ast_grep_core::{Doc, Node};

/// The program a Vue tape embeds (`adapter::regions`); `None` where it embeds none.
pub(crate) fn program<'r, D: Doc>(root: &Node<'r, D>) -> Option<Node<'r, D>> {
    regions(root, GrammarDialect::Vue)
        .next()
        .map(|region| region.root)
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

    /// A same-name shorthand reads its camelized argument (`:aria-label` is `ariaLabel`), though
    /// the tape spans its identifier over `aria-label`: the slot binding resolves and is used, as
    /// vue-eslint-parser reads it (eslint-plugin-vue's no-unused-vars reports nothing).
    #[test]
    fn same_name_shorthand_reads_its_camelized_name() {
        let source =
            r#"<template><Comp v-slot="{ ariaLabel }"><div :aria-label></div></Comp></template>"#;
        let doc = tape(source);
        let table = ScopeTable::build(doc.root(), GrammarDialect::Vue, usize::MAX, &|| false)
            .expect("table builds");
        let read = source.find("aria-label").expect("the shorthand");
        let reference = table
            .reference(&(read..read + "aria-label".len()))
            .expect("the shorthand is a reference");
        let name = table.reference_name(reference, source, 0).expect("a name");
        assert_eq!(name, "ariaLabel");
        assert!(table.resolve(reference, &name).is_some(), "it resolves");
        let scope = table
            .scopes
            .iter()
            .position(|scope| scope.bindings.contains_key("ariaLabel"))
            .expect("the slot binding") as u32;
        let usage = Usage::build(&table, &doc.root(), usize::MAX, &|| false).expect("usage");
        let facts = usage.facts(BindingId {
            scope,
            name: "ariaLabel",
            index: 0,
        });
        assert_eq!(
            (facts.referenced, facts.used, facts.references),
            (Some(true), Some(true), 1)
        );
    }

    /// vue-eslint-parser 10.4.1 reads `<script>` and `<script setup>` as one module program, so
    /// eslint-scope resolves a `<script>` function's read of a `<script setup>` declaration to it,
    /// though Vue's compiler puts that declaration in `setup()`: the rules' oracle reads it so,
    /// and so does the table (eslint-plugin-vue reports no `no-undef` here).
    #[test]
    fn scripts_share_one_module_scope_as_vue_eslint_parser_reads_them() {
        let source = "<script>export function read() { return local }</script><script setup>const local = 1</script>";
        let doc = tape(source);
        let table = ScopeTable::build(doc.root(), GrammarDialect::Vue, usize::MAX, &|| false)
            .expect("table builds");
        let read = source.find("local").expect("the read");
        let reference = table
            .reference(&(read..read + "local".len()))
            .expect("a reference");
        let declared = source.rfind("local").expect("the declaration");
        let binding = table.resolve(reference, "local").expect("it resolves");
        assert_eq!(binding.name.map(|name| name.start as usize), Some(declared));
    }

    /// A template's syntax error leaves the program's facts read (`SyntaxErrors::hide_bindings`)
    /// where the program has no error of its own, and the program holds the template as
    /// tree-sitter-vue recovers it. An unclosed `{{ x </div>` is text to both parsers, so the
    /// `v-for` alias is unused, as eslint-plugin-vue 10.11.1 (vue-eslint-parser 10.4.1) reports
    /// it: `vue/no-unused-vars 10:15 'x' is defined but never used` on this file, with
    /// `vue/no-ref-as-operand` at 5:11 and 6:11 (the combined audit's `BrokenTpl.vue`).
    #[test]
    fn a_template_error_leaves_an_unclosed_interpolations_alias_unused() {
        use crate::code_rule_dsl::soundness::SyntaxErrors;
        let source = "<script setup>\n// E2E-MARKER\nimport { ref } from 'vue'\nconst a = ref(1)\nconst b = a + 1\nconst c = a * 2\n</script>\n\n<template>\n  <div v-for=\"x in xs\">{{ x </div>\n  <p>{{ b }}</p>\n</template>\n";
        let doc = tape(source);
        let errors = SyntaxErrors::of_doc(doc.root().get_doc());
        assert!(!errors.is_empty(), "the template has a syntax error");
        assert!(!errors.hide_bindings(), "the program has none of its own");
        let table = ScopeTable::build(doc.root(), GrammarDialect::Vue, usize::MAX, &|| false)
            .expect("table builds");
        let usage = Usage::build(&table, &doc.root(), usize::MAX, &|| false).expect("usage");
        let facts = |name: &'static str| {
            let scope = table
                .scopes
                .iter()
                .position(|scope| scope.bindings.contains_key(name))
                .unwrap_or_else(|| panic!("{name} is bound")) as u32;
            let facts = usage.facts(BindingId {
                scope,
                name,
                index: 0,
            });
            (facts.referenced, facts.used)
        };
        assert_eq!(facts("x"), (Some(false), Some(false)), "the alias");
        assert_eq!(facts("b"), (Some(true), Some(true)), "read by {{ b }}");
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
