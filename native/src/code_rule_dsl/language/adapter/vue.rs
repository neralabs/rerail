//! Vue single-file components' own tree (tree-sitter-vue). A component's semantics are those of
//! the ECMAScript program its script blocks and template expressions make (`vue::view`, embedded
//! by `embed::ScriptBlock`), a guest document read with the Ecma family (`embed`): a clause reads
//! it with `ast.embedded` or `embedded`. The concerns name Ecma's, which the compile refuses on
//! the component's own tree (`compile::refuse_unavailable_semantics`). Calls and format arguments
//! are refused, as ECMAScript's are: vue-eslint-parser hands the scripts to ESLint, which has
//! neither. Suppressions are ESLint's: its directive comments in the `<script>` blocks, and
//! eslint-plugin-vue's template comment directives (`suppress::vue`).

use super::syntax::QueryOptions;
use super::{Adapter, Ecma, Refusal, Refused, SyntaxFeature, SyntaxRules};
use crate::code_rule_dsl::compile::config::Options;
use crate::code_rule_dsl::language::Family;
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::matcher::{file_eslint_directives, file_vue_directives};
use crate::code_rule_dsl::model::ViolationSpec;
use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
use crate::code_rule_dsl::suppress::{Suppress, VueDirectives, vue_script_directives};
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;
use std::ops::Range;
use std::path::Path;

/// Vue single-file components (`.vue`).
pub(crate) struct Vue;

impl Adapter for Vue {
    const FAMILY: Family = Family::Vue;
    const LABEL: &'static str = "vue";
    const TERMINATORS: Terminators = <Ecma as Adapter>::TERMINATORS;
    const PACKAGES: bool = <Ecma as Adapter>::PACKAGES;
    const DECLARATION_FILES: bool = <Ecma as Adapter>::DECLARATION_FILES;
    type Scope = Ecma;
    type Value = Ecma;
    type Arguments = Ecma;
    type Calls = Refused;
    type FormatArgs = Refused;

    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        <Ecma as Adapter>::name(text)
    }

    fn parse_input(source: &str) -> Cow<'_, str> {
        Cow::Borrowed(source)
    }

    fn module_path(file: &Path) -> Option<serde_json::Value> {
        <Ecma as Adapter>::module_path(file)
    }

    /// ESLint's directive comments in the scripts (`// eslint-disable-line`), as ESLint core
    /// applies them to the problems eslint-plugin-vue reports, or eslint-plugin-vue's
    /// `<!-- eslint-disable -->` template comments (`vue/comment-directive`).
    fn suppresses<D: Doc>(
        _: &ViolationSpec,
        suppress: &Suppress,
        node: &Node<'_, D>,
        source: &str,
        lines: &LineTable,
        anchored: Range<usize>,
        _: (usize, usize),
    ) -> bool {
        let at = anchored.start;
        suppress.eslint.as_ref().is_some_and(|rules| {
            let scripts = file_eslint_directives(|| vue_script_directives(node, source, lines));
            scripts.suppresses(rules, at, lines) || {
                let template = file_vue_directives(|| VueDirectives::read(node, source, lines));
                template.suppresses(rules, at, lines)
            }
        })
    }
}

/// ESLint's `languageOptions` and `when` keys (vue-eslint-parser hands ESLint's parser options to
/// the scripts' parser). The component's own tree holds no literal `parse` reads: its scripts'
/// comments are the embedded program's.
impl SyntaxRules for Vue {
    const DOLLAR_NAMES: bool = <Ecma as SyntaxRules>::DOLLAR_NAMES;

    fn feature(feature: SyntaxFeature) -> Result<(), Refusal> {
        match feature {
            SyntaxFeature::LanguageOptions => <Ecma as SyntaxRules>::feature(feature),
            SyntaxFeature::Parse(grammar) => Err(Refusal::Refused(Cow::Owned(format!(
                "`{}` parses {}; a component's scripts are its embedded program: read them with `ast.embedded: script-block` or `embedded`",
                grammar.name(),
                grammar.host()
            )))),
        }
    }

    fn compile_language_options(
        value: &serde_json::Value,
        path: &str,
    ) -> napi::Result<QueryOptions> {
        <Ecma as SyntaxRules>::compile_language_options(value, path)
    }

    fn compile_when(
        value: &serde_json::Value,
        path: &str,
        options: Option<&LanguageOptions>,
        python: Option<&PythonOptions>,
        document: &Options,
    ) -> napi::Result<bool> {
        <Ecma as SyntaxRules>::compile_when(value, path, options, python, document)
    }
}

#[cfg(test)]
mod tests {
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use ast_grep_core::Doc;
    use std::sync::Arc;

    /// A component's embedded program is a document in the dialect its script's `lang` names
    /// (`<script setup>`'s first).
    #[test]
    fn a_components_program_is_read_in_its_scripts_lang() {
        for (source, dialect) in [
            (
                "<script>\nlet x = 1;\n</script>\n",
                GrammarDialect::JavaScript,
            ),
            (
                "<script lang=\"jsx\">\nlet x = 1;\n</script>\n",
                GrammarDialect::JavaScript,
            ),
            (
                "<script lang=\"ts\">\nlet x = 1;\n</script>\n",
                GrammarDialect::TypeScript,
            ),
            (
                "<script lang='tsx'>\nlet x = <a />;\n</script>\n",
                GrammarDialect::Tsx,
            ),
            (
                "<script>\nlet y = 1;\n</script>\n<script setup lang=\"ts\">\nlet x = 1;\n</script>\n",
                GrammarDialect::TypeScript,
            ),
            ("<template>{{ x }}</template>\n", GrammarDialect::JavaScript),
        ] {
            let grammar = Grammar::new(GrammarDialect::Vue).expect("grammar");
            let image = TapeImage::parse(&grammar, source, &|| false)
                .expect("parses")
                .expect("encodes");
            let doc = TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false)
                .expect("restores");
            let dialects = doc
                .guests()
                .expect("owned")
                .iter()
                .map(|guest| *guest.doc.get_lang())
                .collect::<Vec<_>>();
            assert_eq!(dialects, [dialect], "{source:?}");
        }
    }
}
