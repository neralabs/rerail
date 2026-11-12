//! Vue single-file components' own tree (tree-sitter-vue). A component's semantics are those of
//! the ECMAScript program its script blocks and template expressions make (`vue::view`, embedded
//! by `embed::ScriptBlock`), a guest document read with the Ecma family (`embed`): a clause reads
//! it with `ast.embedded` or `embedded`. The concerns name Ecma's, which the compile refuses on
//! the component's own tree (`compile::refuse_unavailable_semantics`). Calls and format arguments
//! are refused, as ECMAScript's are: vue-eslint-parser hands the scripts to ESLint, which has
//! neither. Suppressions are ESLint's: its directive comments in the `<script>` blocks, and
//! eslint-plugin-vue's template comment directives (`suppress::vue`).

use super::QueryWhen;
use super::syntax::QueryOptions;
use super::ecma::Ecma;
use super::{Adapter, Refusal, Refused, SyntaxFeature, SyntaxRules};
use crate::code_rule_dsl::compile::config::Options;
use crate::code_rule_dsl::language::{Family, SourceType};
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::matcher::{file_index};
use crate::code_rule_dsl::model::ViolationSpec;
use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
use crate::code_rule_dsl::suppress::Suppress;
use crate::code_rule_dsl::suppress::vue::{VueDirectives, script_directives as vue_script_directives};
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;
use std::ops::Range;
use std::path::Path;

/// Vue single-file components (`.vue`).
pub struct Vue;

impl Adapter for Vue {
    const FAMILY: Family = Family::Vue;
    const LABEL: &'static str = "vue";
    const TERMINATORS: Terminators = <Ecma as Adapter>::TERMINATORS;
    const PACKAGES: bool = <Ecma as Adapter>::PACKAGES;
    // A component's script is a module, whatever its package's `type` (vue-eslint-parser's
    // default), and its JSX runtime is Vue's.
    const PACKAGE_MANIFESTS: bool = false;
    const JSX_RUNTIMES: bool = false;
    const DECLARATION_FILES: bool = <Ecma as Adapter>::DECLARATION_FILES;
    const STRIPS_BOM: bool = <Ecma as Adapter>::STRIPS_BOM;
    /// The component's own `<!-- -->` comments; its scripts' are read with the Ecma family.
    const COMMENT_KINDS: &'static [&'static str] = &["comment"];
    const PREAMBLE_KINDS: &'static [&'static str] = &[];
    const BODY_KINDS: &'static [&'static str] = &[];
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
        suppress.get::<crate::code_rule_dsl::suppress::eslint::Suppressions>().is_some_and(|rules| {
            let scripts = file_index(|| vue_script_directives(node, source, lines));
            scripts.suppresses(rules, at, lines) || {
                let template = file_index(|| VueDirectives::read(node, source, lines));
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
    ) -> napi::Result<QueryWhen> {
        // A component's script is a module: a `when.sourceType` reads that.
        let (applies, types) =
            <Ecma as SyntaxRules>::compile_when(value, path, options, python, document)?;
        Ok((
            applies && types.is_none_or(|types| types.contains(&SourceType::Module)),
            None,
        ))
    }
}

#[cfg(test)]
pub mod tests;
