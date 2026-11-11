//! `.rr` rule files' family (decision 0061): the rule engine's own tree (`logic::rr::tape`), read by `.rr` rules
//! through the tape and its text facts. No linter's semantics apply: scope, values, arguments, calls and format
//! arguments are refused (the registry's `refused`), so the compile refuses a rule that asks for them on a `.rr` file
//! and nothing here is reached. Each refused hook answers "nothing known".

use super::QueryWhen;
use super::{
    Adapter, Refusal, Refused, ScopeFeature, ScopeRules, SyntaxFeature, SyntaxRules, ValueFeature, ValueRules,
};
use crate::code_rule_dsl::compile::config::Options;
use crate::code_rule_dsl::compile::options;
use crate::code_rule_dsl::language::Family;
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::matcher::{ScopeName, Unresolved};
use crate::code_rule_dsl::model::ViolationSpec;
use crate::code_rule_dsl::module_path::ModulePath;
use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
use crate::code_rule_dsl::scope::usage::{Steps, Usage};
use crate::code_rule_dsl::scope::{
    Binding, BuildError, Builder, Directives, Form, Lookup, Reading, Reference, ScopeTable,
};
use crate::code_rule_dsl::suppress::Suppress;
use crate::code_rule_dsl::value::structure::{Equality, Tok};
use crate::code_rule_dsl::value::{BigInt, Context, NodeRef, Value};
use ast_grep_core::{Doc, Node};
use serde_json::Value as Json;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::ops::Range;
use std::path::Path;
use std::sync::Arc;

/// `.rr` rule files.
pub(crate) struct Rr;

fn refused(message: impl Into<Cow<'static, str>>) -> Result<(), Refusal> {
    Err(Refusal::Refused(message.into()))
}

impl Adapter for Rr {
    const FAMILY: Family = Family::Rr;
    const LABEL: &'static str = "rr";
    const TERMINATORS: Terminators = Terminators::Newlines;
    const PACKAGES: bool = false;
    const PACKAGE_MANIFESTS: bool = false;
    const JSX_RUNTIMES: bool = false;
    const DECLARATION_FILES: bool = false;
    const STRIPS_BOM: bool = false;
    /// `.rr` files have no suppression comments, so no `rerail-ignore` directives either.
    const COMMENT_KINDS: &'static [&'static str] = &[];
    const PREAMBLE_KINDS: &'static [&'static str] = &[];
    const BODY_KINDS: &'static [&'static str] = &[];
    type Scope = Self;
    type Value = Self;
    type Arguments = Refused;
    type Calls = Refused;
    type FormatArgs = Refused;

    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        text
    }

    fn parse_input(source: &str) -> Cow<'_, str> {
        Cow::Borrowed(source)
    }

    fn module_path(_: &Path) -> Option<Json> {
        None
    }

    /// `.rr` files have no suppression comments.
    fn suppresses<D: Doc>(
        _: &ViolationSpec,
        _: &Suppress,
        _: &Node<'_, D>,
        _: &str,
        _: &LineTable,
        _: Range<usize>,
        _: (usize, usize),
    ) -> bool {
        false
    }
}

impl ScopeRules for Rr {
    const SUPPORTED: bool = false;
    const FLOW: bool = false;
    const STUBS: bool = false;
    const REDEFINES_SHADOWED: bool = false;
    const REFERENCE_KINDS: &'static [&'static str] = &[];
    const ANNOTATION_STRING_KINDS: &'static [&'static str] = &[];
    // Main's scope kinds (#462): every dialect's parentheses are `parenthesized_expression`, which `.rr` has too (the
    // kit's `shared_walks_read_the_kinds_they_named`); it has no dotted names, attributes or `this`.
    const PAREN_KINDS: &'static [&'static str] = &["parenthesized_expression"];
    const DOTTED_NAME_KINDS: &'static [&'static str] = &[];
    const ATTRIBUTE_KINDS: &'static [&'static str] = &[];
    const THIS_KINDS: &'static [&'static str] = &[];
    const NAME_KINDS: &'static [&'static str] = &[];
    const PRIVATE_NAME_KINDS: &'static [&'static str] = &[];
    const FORMS: &'static [(&'static str, Form)] = &[];
    const SCOPES: &'static [(&'static str, ScopeName)] = &[];
    const REFUSED_FILTERS: &'static [&'static str] = &[];
    const IMPORT_PATHS: bool = false;

    fn visit<D: Doc>(_: &mut Builder<'_>, _: &Node<'_, D>, _: Reading) -> Result<(), BuildError> {
        Ok(())
    }

    fn finish<D: Doc>(_: &mut ScopeTable, _: &Node<'_, D>) {}

    fn resolve<'t>(_: &'t ScopeTable, _: &Reference, _: &str) -> Option<&'t Binding> {
        None
    }

    fn binding<'t>(_: &'t ScopeTable, _: &Reference, _: &str) -> Option<(&'t Binding, Option<NodeRef>)> {
        None
    }

    fn binding_by<'t>(_: &'t ScopeTable, _: &Reference, _: &str, _: Lookup) -> Option<(&'t Binding, Option<NodeRef>)> {
        None
    }

    fn index(_: &ScopeTable, _: &Reference, _: &str, _: Lookup) -> Option<(u32, usize)> {
        None
    }

    fn undecided(_: &ScopeTable, _: &Reference) -> bool {
        true
    }

    fn unbound_annotation(_: &ScopeTable, _: &Reference, _: &str) -> Option<(u32, usize)> {
        None
    }

    fn unresolved(
        _: &ScopeTable,
        _: &Reference,
        _: &str,
        _: Unresolved,
        _: &PythonOptions,
        _: Option<&LanguageOptions>,
        _: Option<&Directives>,
    ) -> bool {
        false
    }

    fn eslint_options(_: Option<&Arc<LanguageOptions>>) -> Option<Arc<LanguageOptions>> {
        None
    }

    fn ruff_options(_: Option<&Arc<PythonOptions>>) -> Option<Arc<PythonOptions>> {
        None
    }

    #[cfg(test)]
    fn unbound_locally(_: &ScopeTable, _: &Reference, _: &str, _: bool) -> bool {
        false
    }

    /// Never reached: scope is refused, so no rule asks for a `.rr` file's usage.
    fn usage<D: Doc>(_: &ScopeTable, _: &Node<'_, D>, _: &mut Steps<'_>) -> Result<Usage, BuildError> {
        Err(BuildError::Budget)
    }

    fn unpackaged_module(_: &str) -> Option<ModulePath> {
        None
    }

    fn import_path<D: Doc>(_: &ScopeTable, _: &Node<'_, D>, _: Option<&str>) -> Option<String> {
        None
    }

    fn feature(_: ScopeFeature) -> Result<(), Refusal> {
        refused("scope is not available for rr rules")
    }
}

impl ValueRules for Rr {
    const SUPPORTED: bool = false;
    const LANG: Family = Family::Rr;
    const EXPRESSION_KINDS: &'static [&'static str] = &[];
    const VALUE_KINDS: &'static [&'static str] = &[];
    const NAME_KINDS: &'static [&'static str] = &[];

    fn truthy(_: &Value) -> Option<bool> {
        None
    }

    fn type_name(_: &Value) -> Option<&'static str> {
        None
    }

    fn equals(_: &Value, _: &Value) -> Option<bool> {
        None
    }

    fn order(_: &Value, _: &Value) -> Option<Option<Ordering>> {
        None
    }

    fn relation(_: &str, _: &Value, _: &Value) -> Option<bool> {
        None
    }

    fn numeric(_: &Value) -> bool {
        false
    }

    fn integer(_: &Value) -> Option<BigInt> {
        None
    }

    fn to_str<D: Doc>(_: &Value, _: &Node<'_, D>) -> Option<String> {
        None
    }

    fn decode<D: Doc>(_: &Node<'_, D>, _: &mut Context<'_>) -> Value {
        Value::Unknown
    }

    fn structure<D: Doc>(_: &Node<'_, D>, _: Equality, _: &mut Context<'_>) -> Option<Vec<Tok>> {
        None
    }

    fn string(_: &str) -> Value {
        Value::Unknown
    }

    fn number(_: &serde_json::Number) -> Value {
        Value::Unknown
    }

    fn spelled<D: Doc>(_: &Node<'_, D>) -> bool {
        false
    }

    fn node_name<D: Doc>(_: &Node<'_, D>) -> Option<Vec<u16>> {
        None
    }

    fn key_name<'r, D: Doc>(_: &Node<'r, D>, _: &mut dyn FnMut(&Node<'r, D>) -> Value) -> Option<Vec<u16>> {
        None
    }

    fn member_name<'r, D: Doc>(_: &Node<'r, D>, _: &mut dyn FnMut(&Node<'r, D>) -> Value) -> Option<Vec<u16>> {
        None
    }

    fn same_reference<D: Doc>(_: &Node<'_, D>, _: &Node<'_, D>, _: bool) -> Option<bool> {
        None
    }

    fn feature(_: ValueFeature<'_>) -> Result<(), Refusal> {
        refused("values are not available for rr rules")
    }
}

/// A `.rr` file hosts no literal grammar a policy parses, and has no `languageOptions`.
impl SyntaxRules for Rr {
    const DOLLAR_NAMES: bool = false;

    fn feature(feature: SyntaxFeature) -> Result<(), Refusal> {
        match feature {
            SyntaxFeature::LanguageOptions => refused("`languageOptions` is not available for rr policies"),
            SyntaxFeature::Parse(grammar) => refused(format!("`{}` parses {}", grammar.name(), grammar.host())),
        }
    }

    fn compile_language_options(_: &Json, _: &str) -> napi::Result<(Option<Arc<LanguageOptions>>, Option<Arc<PythonOptions>>)> {
        Ok((None, None))
    }

    fn compile_when(
        value: &Json,
        path: &str,
        _: Option<&LanguageOptions>,
        _: Option<&PythonOptions>,
        document: &Options,
    ) -> napi::Result<QueryWhen> {
        Ok((options::compile_option_when(value, path, document)?, None))
    }
}
