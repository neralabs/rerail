//! Vue single-file components: their semantics are those of the ECMAScript program their script
//! blocks and template expressions make (`vue::view`, embedded by `embed::ScriptBlock`), which each
//! concern reads with the Ecma family's code. Calls and format arguments are refused, as
//! ECMAScript's are: vue-eslint-parser hands the scripts to ESLint, which has neither.
//! Suppressions are ESLint's: its directive comments in the `<script>` blocks, and
//! eslint-plugin-vue's template comment directives (`suppress::vue`).

use super::syntax::QueryOptions;
use super::{
    Adapter, Ecma, Refusal, Refused, Region, ScopeFeature, ScopeRules, SyntaxFeature, SyntaxRules,
    ValueFeature, ValueRules,
};
use crate::code_rule_dsl::compile::config::Options;
use crate::code_rule_dsl::language::{Family, GrammarDialect};
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::matcher::{
    ScopeName, Unresolved, file_eslint_directives, file_vue_directives,
};
use crate::code_rule_dsl::model::ViolationSpec;
use crate::code_rule_dsl::module_path::ModulePath;
use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
use crate::code_rule_dsl::scope::usage::{Steps, Usage};
use crate::code_rule_dsl::scope::{
    Binding, BuildError, Builder, Directives, Form, Lookup, Reading, Reference, ScopeTable,
};
use crate::code_rule_dsl::suppress::{Suppress, VueDirectives, vue_script_directives};
use crate::code_rule_dsl::value::structure::{Equality, Tok};
use crate::code_rule_dsl::value::{BigInt, Context, NodeRef, Value};
use crate::code_rule_dsl::vue;
use ast_grep_core::{Doc, Language, Node};
use ast_grep_language::LanguageExt;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::path::Path;
use std::sync::Arc;

/// Vue single-file components (`.vue`).
pub(crate) struct Vue;

impl Adapter for Vue {
    const FAMILY: Family = Family::Vue;
    const LABEL: &'static str = "vue";
    const TERMINATORS: Terminators = <Ecma as Adapter>::TERMINATORS;
    const PACKAGES: bool = <Ecma as Adapter>::PACKAGES;
    const DECLARATION_FILES: bool = <Ecma as Adapter>::DECLARATION_FILES;
    type Scope = Self;
    type Value = Self;
    type Arguments = Ecma;
    type Calls = Refused;
    type FormatArgs = Refused;

    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        <Ecma as Adapter>::name(text)
    }

    /// The program the component embeds (`vue::program`), read as the dialect its script's `lang`
    /// names (`vue::tape_script_dialect`); none where the script is in another language
    /// (`lang="coffee"`) or the component holds nothing Vue compiles.
    fn regions<'r, D: Doc>(
        root: &Node<'r, D>,
        _: GrammarDialect,
    ) -> impl Iterator<Item = Region<'r, D>> {
        vue::program(root)
            .and_then(|program| {
                Some(Region {
                    dialect: vue::tape_script_dialect(root)?,
                    root: program,
                })
            })
            .into_iter()
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
        at: usize,
        _: (usize, usize),
    ) -> bool {
        suppress.eslint.as_ref().is_some_and(|rules| {
            let scripts = file_eslint_directives(|| vue_script_directives(node, source, lines));
            scripts.suppresses(rules, at, lines) || {
                let template = file_vue_directives(|| VueDirectives::read(node, source, lines));
                template.suppresses(rules, at, lines)
            }
        })
    }
}

/// ECMAScript's scope (eslint-scope's, as vue-eslint-parser builds it for the scripts), over the
/// program the scripts and template expressions make: the template's references are read in the
/// scopes Vue's compiler gives them. Everything but the visit, which reads the program, is
/// Ecma's.
impl ScopeRules for Vue {
    const SUPPORTED: bool = true;
    const FLOW: bool = <Ecma as ScopeRules>::FLOW;
    const STUBS: bool = <Ecma as ScopeRules>::STUBS;
    const REDEFINES_SHADOWED: bool = <Ecma as ScopeRules>::REDEFINES_SHADOWED;
    const REFERENCE_KINDS: &'static [&'static str] = <Ecma as ScopeRules>::REFERENCE_KINDS;
    const ANNOTATION_STRING_KINDS: &'static [&'static str] =
        <Ecma as ScopeRules>::ANNOTATION_STRING_KINDS;
    const NAME_KINDS: &'static [&'static str] = <Ecma as ScopeRules>::NAME_KINDS;
    const PRIVATE_NAME_KINDS: &'static [&'static str] = <Ecma as ScopeRules>::PRIVATE_NAME_KINDS;
    const FORMS: &'static [(&'static str, Form)] = <Ecma as ScopeRules>::FORMS;
    const SCOPES: &'static [(&'static str, ScopeName)] = <Ecma as ScopeRules>::SCOPES;
    const REFUSED_FILTERS: &'static [&'static str] = <Ecma as ScopeRules>::REFUSED_FILTERS;

    fn visit<D: Doc>(
        builder: &mut Builder<'_>,
        root: &Node<'_, D>,
        reading: Reading,
    ) -> Result<(), BuildError> {
        for region in <Self as Adapter>::regions(root, GrammarDialect::Vue) {
            <Ecma as ScopeRules>::visit(builder, &region.root, reading)?;
        }
        Ok(())
    }

    fn finish<D: Doc>(table: &mut ScopeTable, root: &Node<'_, D>) {
        for region in <Self as Adapter>::regions(root, GrammarDialect::Vue) {
            <Ecma as ScopeRules>::finish(table, &region.root);
        }
    }

    fn resolve<'t>(
        table: &'t ScopeTable,
        reference: &Reference,
        name: &str,
    ) -> Option<&'t Binding> {
        <Ecma as ScopeRules>::resolve(table, reference, name)
    }

    fn binding<'t>(
        table: &'t ScopeTable,
        reference: &Reference,
        name: &str,
    ) -> Option<(&'t Binding, Option<NodeRef>)> {
        <Ecma as ScopeRules>::binding(table, reference, name)
    }

    fn binding_by<'t>(
        table: &'t ScopeTable,
        reference: &Reference,
        name: &str,
        lookup: Lookup,
    ) -> Option<(&'t Binding, Option<NodeRef>)> {
        <Ecma as ScopeRules>::binding_by(table, reference, name, lookup)
    }

    fn index(
        table: &ScopeTable,
        reference: &Reference,
        name: &str,
        lookup: Lookup,
    ) -> Option<(u32, usize)> {
        <Ecma as ScopeRules>::index(table, reference, name, lookup)
    }

    fn undecided(table: &ScopeTable, reference: &Reference) -> bool {
        <Ecma as ScopeRules>::undecided(table, reference)
    }

    fn unbound_annotation(
        table: &ScopeTable,
        reference: &Reference,
        name: &str,
    ) -> Option<(u32, usize)> {
        <Ecma as ScopeRules>::unbound_annotation(table, reference, name)
    }

    fn unresolved(
        table: &ScopeTable,
        reference: &Reference,
        name: &str,
        mode: Unresolved,
        python: &PythonOptions,
        options: Option<&LanguageOptions>,
        directives: Option<&Directives>,
    ) -> bool {
        <Ecma as ScopeRules>::unresolved(table, reference, name, mode, python, options, directives)
    }

    fn eslint_options(options: Option<&Arc<LanguageOptions>>) -> Option<Arc<LanguageOptions>> {
        <Ecma as ScopeRules>::eslint_options(options)
    }

    fn ruff_options(python: Option<&Arc<PythonOptions>>) -> Option<Arc<PythonOptions>> {
        <Ecma as ScopeRules>::ruff_options(python)
    }

    #[cfg(test)]
    fn unbound_locally(
        table: &ScopeTable,
        reference: &Reference,
        name: &str,
        builtin: bool,
    ) -> bool {
        <Ecma as ScopeRules>::unbound_locally(table, reference, name, builtin)
    }

    /// The program's usage: a file without one has no bindings.
    fn usage<D: Doc>(
        table: &ScopeTable,
        root: &Node<'_, D>,
        steps: &mut Steps<'_>,
    ) -> Result<Usage, BuildError> {
        let program = vue::program(root).unwrap_or_else(|| root.clone());
        <Ecma as ScopeRules>::usage(table, &program, steps)
    }

    fn unpackaged_module(path: &str) -> Option<ModulePath> {
        <Ecma as ScopeRules>::unpackaged_module(path)
    }

    fn feature(feature: ScopeFeature) -> Result<(), Refusal> {
        <Ecma as ScopeRules>::feature(feature)
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
    use super::{Adapter, Vue};
    use crate::code_rule_dsl::language::GrammarDialect;
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
            let dialects = Vue::regions(&root, GrammarDialect::Vue)
                .map(|region| region.dialect)
                .collect::<Vec<_>>();
            assert_eq!(dialects, [dialect], "{source:?}");
        }
    }
}
