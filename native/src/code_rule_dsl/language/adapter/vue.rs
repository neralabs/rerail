//! Vue single-file components: their semantics are those of the ECMAScript program their script
//! blocks and template expressions make (`vue::view`, embedded by `embed::ScriptBlock`), which each
//! concern reads with the Ecma family's code. Scope is Ecma's, which reads the program as the
//! document's region (`adapter::regions`). Calls and format arguments are refused, as
//! ECMAScript's are: vue-eslint-parser hands the scripts to ESLint, which has neither.
//! Suppressions are ESLint's: its directive comments in the `<script>` blocks, and
//! eslint-plugin-vue's template comment directives (`suppress::vue`).

use super::syntax::QueryOptions;
use super::{
    Adapter, Ecma, Refusal, Refused, SyntaxFeature, SyntaxRules, ValueFeature, ValueRules,
};
use crate::code_rule_dsl::compile::config::Options;
use crate::code_rule_dsl::language::{Family, GrammarDialect};
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::matcher::{file_eslint_directives, file_vue_directives};
use crate::code_rule_dsl::model::ViolationSpec;
use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
use crate::code_rule_dsl::suppress::{Suppress, VueDirectives, vue_script_directives};
use crate::code_rule_dsl::value::structure::{Equality, Tok};
use crate::code_rule_dsl::value::{BigInt, Context, Value};
use crate::code_rule_dsl::vue;
use ast_grep_core::{Doc, Language, Node};
use ast_grep_language::LanguageExt;
use std::borrow::Cow;
use std::cmp::Ordering;
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
    type Value = Self;
    type Arguments = Ecma;
    type Calls = Refused;
    type FormatArgs = Refused;

    /// ECMAScript's, but for a same-name shorthand's identifier (`:aria-label`), the one
    /// identifier the view writes over file bytes that are no identifier: the tape keeps its
    /// span, `aria-label`, and the name Vue reads is the camelized argument, `ariaLabel`
    /// (`vue::view`). No ECMAScript identifier holds a `-`.
    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        if text.contains('-') {
            Cow::Owned(vue::view::camelize(&text))
        } else {
            <Ecma as Adapter>::name(text)
        }
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

/// The values of the ECMAScript program the scripts and template expressions make, but for the
/// tokens the view writes, which the file does not spell.
impl ValueRules for Vue {
    const SUPPORTED: bool = <Ecma as ValueRules>::SUPPORTED;
    const LANG: Family = <Ecma as ValueRules>::LANG;
    const EXPRESSION_KINDS: &'static [&'static str] = <Ecma as ValueRules>::EXPRESSION_KINDS;
    const VALUE_KINDS: &'static [&'static str] = <Ecma as ValueRules>::VALUE_KINDS;
    const NAME_KINDS: &'static [&'static str] = <Ecma as ValueRules>::NAME_KINDS;

    fn truthy(value: &Value) -> Option<bool> {
        <Ecma as ValueRules>::truthy(value)
    }

    fn type_name(value: &Value) -> Option<&'static str> {
        <Ecma as ValueRules>::type_name(value)
    }

    fn equals(left: &Value, right: &Value) -> Option<bool> {
        <Ecma as ValueRules>::equals(left, right)
    }

    fn order(left: &Value, right: &Value) -> Option<Option<Ordering>> {
        <Ecma as ValueRules>::order(left, right)
    }

    fn relation(operator: &str, left: &Value, right: &Value) -> Option<bool> {
        <Ecma as ValueRules>::relation(operator, left, right)
    }

    fn numeric(value: &Value) -> bool {
        <Ecma as ValueRules>::numeric(value)
    }

    fn integer(value: &Value) -> Option<BigInt> {
        <Ecma as ValueRules>::integer(value)
    }

    fn to_str<D: Doc>(value: &Value, node: &Node<'_, D>) -> Option<String> {
        <Ecma as ValueRules>::to_str(value, node)
    }

    fn decode<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
        <Ecma as ValueRules>::decode(node, context)
    }

    fn structure<D: Doc>(
        node: &Node<'_, D>,
        equality: Equality,
        context: &mut Context<'_>,
    ) -> Option<Vec<Tok>> {
        <Ecma as ValueRules>::structure(node, equality, context)
    }

    fn string(text: &str) -> Value {
        <Ecma as ValueRules>::string(text)
    }

    fn number(number: &serde_json::Number) -> Value {
        <Ecma as ValueRules>::number(number)
    }

    /// Whether no token of the node is one the view writes (the `(` and `);` around an
    /// interpolation, a character reference's character), which a comparison of tokens must not
    /// read as the file's (`embed::rewritten`).
    fn spelled<D: Doc>(node: &Node<'_, D>) -> bool {
        !crate::code_rule_dsl::embed::rewritten(node)
    }

    fn node_name<D: Doc>(node: &Node<'_, D>) -> Option<Vec<u16>> {
        <Ecma as ValueRules>::node_name(node)
    }

    fn key_name<'r, D: Doc>(
        node: &Node<'r, D>,
        value: &mut dyn FnMut(&Node<'r, D>) -> Value,
    ) -> Option<Vec<u16>> {
        <Ecma as ValueRules>::key_name(node, value)
    }

    fn member_name<'r, D: Doc>(
        node: &Node<'r, D>,
        value: &mut dyn FnMut(&Node<'r, D>) -> Value,
    ) -> Option<Vec<u16>> {
        <Ecma as ValueRules>::member_name(node, value)
    }

    fn same_reference<D: Doc>(
        left: &Node<'_, D>,
        right: &Node<'_, D>,
        static_keys: bool,
    ) -> Option<bool> {
        <Ecma as ValueRules>::same_reference(left, right, static_keys)
    }

    fn feature(feature: ValueFeature<'_>) -> Result<(), Refusal> {
        <Ecma as ValueRules>::feature(feature)
    }
}

/// ESLint's comments, `languageOptions` and `when` keys (vue-eslint-parser hands ESLint's parser
/// options to the scripts' parser). A pattern with no Vue construct (an element, a directive, an
/// interpolation, a comment) is TypeScript, whose codes the TSX and JavaScript a file's program
/// may be written in share wherever their kinds are named alike (`KindCatalog`), so it matches
/// any program; a pattern with JSX matches none, its kinds being TSX's alone.
impl SyntaxRules for Vue {
    const DOLLAR_NAMES: bool = <Ecma as SyntaxRules>::DOLLAR_NAMES;

    fn feature(feature: SyntaxFeature) -> Result<(), Refusal> {
        <Ecma as SyntaxRules>::feature(feature)
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

    fn embedded_pattern(text: &str) -> Option<GrammarDialect> {
        template_text(text).then_some(GrammarDialect::TypeScript)
    }

    fn php_comment(_: GrammarDialect, _: u16) -> bool {
        true
    }
}

/// Whether tree-sitter-vue reads `source` as text alone, with no element, directive,
/// interpolation or comment: a rule's pattern that is the embedded program's (`rule_pattern`).
fn template_text(source: &str) -> bool {
    let vue = GrammarDialect::Vue;
    let root = vue.ast_grep(vue.pre_process_pattern(source));
    root.root()
        .dfs()
        .all(|node| !node.is_named() || matches!(&*node.kind(), "document" | "text" | "ERROR"))
}

#[cfg(test)]
mod tests {
    use crate::code_rule_dsl::language::GrammarDialect;
    use crate::code_rule_dsl::language::adapter::regions;
    use crate::code_rule_dsl::tape::{Grammar, TapeDoc, TapeImage};
    use ast_grep_core::AstGrep;
    use std::sync::Arc;

    /// A component's region is read in the dialect its script's `lang` names (`<script setup>`'s
    /// first), though TypeScript, TSX and JavaScript share the program's kind codes.
    #[test]
    fn a_region_is_read_in_its_scripts_lang() {
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
            let doc = AstGrep::doc(
                TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false)
                    .expect("restores"),
            );
            let root = doc.root();
            let dialects = regions(&root, GrammarDialect::Vue)
                .map(|region| region.dialect)
                .collect::<Vec<_>>();
            assert_eq!(dialects, [dialect], "{source:?}");
        }
    }
}
