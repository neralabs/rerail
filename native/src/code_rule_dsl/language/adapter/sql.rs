//! SQL's family: SQLFluff 4.4.0's semantics over the vendored tree-sitter-sql grammar, one
//! generic dialect for PostgreSQL, SQLite, MySQL and ANSI files. Scope reads the query blocks of
//! `scope/sql.rs` (RF01's references, AL05's aliases), values the literals and names of
//! `value/sql.rs` where PostgreSQL and SQLite agree, arguments PostgreSQL's call binding
//! (`matcher/arguments_sql.rs`), and suppressions SQLFluff's `-- noqa` (`suppress/sqlfluff.rs`).
//!
//! `calls` and `formatArgs` are refused. No SQLFluff rule follows calls across routines; a
//! routine's body is a string literal of another language (`$$ ... $$`, PL/pgSQL), and a `.sql`
//! file has no workspace-resolvable call graph. No SQLFluff rule reads a format string, and ISO
//! SQL has no format function (PostgreSQL's `format()` and SQLite's `printf()` are extensions).

use super::{
    Adapter, ArgumentFeature, ArgumentRules, Refusal, Refused, Region, ScopeFeature, ScopeRules,
    SyntaxFeature, SyntaxRules, ValueFeature, ValueRules,
};
use crate::code_rule_dsl::compile::config::Options;
use crate::code_rule_dsl::compile::options;
use crate::code_rule_dsl::language::{Family, GrammarDialect};
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::matcher::{
    Arguments, Passed, SQL_CALL_KINDS, ScopeName, SemanticText, Unresolved, file_sqlfluff,
};
use crate::code_rule_dsl::model::ViolationSpec;
use crate::code_rule_dsl::module_path::ModulePath;
use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
use crate::code_rule_dsl::scope::usage::{Steps, Usage};
use crate::code_rule_dsl::scope::{
    Binding, BuildError, Builder, Directives, Form, Lookup, Reading, Reference, ScopeKind,
    ScopeTable, sql,
};
use crate::code_rule_dsl::suppress::{SqlfluffIndex, Suppress, root};
use crate::code_rule_dsl::value::structure::{Equality, Tok};
use crate::code_rule_dsl::value::{self, BigInt, Context, NodeRef, Value};
use ast_grep_core::{Doc, Node};
use serde_json::Value as Json;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::path::Path;
use std::sync::Arc;

/// SQL's family: SQLFluff's semantics.
pub(crate) struct Sql;

fn refused(message: impl Into<Cow<'static, str>>) -> Result<(), Refusal> {
    Err(Refusal::Refused(message.into()))
}

impl Adapter for Sql {
    const FAMILY: Family = Family::Sql;
    const LABEL: &'static str = "sql";
    /// SQLFluff's lexer ends a line at `\r\n` and `\n`; a lone `\r`, which it does not, is a line
    /// break here as Ruff's is.
    const TERMINATORS: Terminators = Terminators::Newlines;
    const PACKAGES: bool = false;
    const DECLARATION_FILES: bool = false;
    type Scope = Self;
    type Value = Self;
    type Arguments = Self;
    type Calls = Refused;
    type FormatArgs = Refused;

    /// PostgreSQL's identifier: a naked name folds to lower case, a quoted one is exact.
    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        value::sql::identifier(text)
    }

    fn regions<'r, D: Doc>(
        root: &Node<'r, D>,
        dialect: GrammarDialect,
    ) -> impl Iterator<Item = Region<'r, D>> {
        std::iter::once(Region {
            root: root.clone(),
            dialect,
        })
    }

    fn module_path(_: &Path) -> Option<Json> {
        None
    }

    /// SQLFluff reads a `-- noqa` directive on the violation's line, and `noqa: disable=` /
    /// `enable=` ranges before it.
    fn suppresses<D: Doc>(
        _: &ViolationSpec,
        suppress: &Suppress,
        node: &Node<'_, D>,
        source: &str,
        lines: &LineTable,
        at: usize,
        _: (usize, usize),
    ) -> bool {
        suppress.sqlfluff.as_ref().is_some_and(|references| {
            let index = file_sqlfluff(|| SqlfluffIndex::read(&root(node), source, lines));
            index.suppresses(references, lines.line(at) + 1)
        })
    }
}

/// RF01's references and AL05's aliases (`scope/sql.rs`): a qualified column reference's table
/// part resolves to the relation, CTE or DML target of its query block or an enclosing one.
impl ScopeRules for Sql {
    const SUPPORTED: bool = true;
    const FLOW: bool = false;
    const STUBS: bool = false;
    const REDEFINES_SHADOWED: bool = false;
    const REFERENCE_KINDS: &'static [&'static str] = &["identifier"];
    const ANNOTATION_STRING_KINDS: &'static [&'static str] = &[];
    const NAME_KINDS: &'static [&'static str] = &["identifier"];
    const PRIVATE_NAME_KINDS: &'static [&'static str] = &[];
    const FORMS: &'static [(&'static str, Form)] = &[
        ("relation", Form::Relation),
        ("cte", Form::Cte),
        ("target", Form::Target),
    ];
    const SCOPES: &'static [(&'static str, ScopeName)] =
        &[("query", ScopeName::Kind(ScopeKind::Query))];
    const REFUSED_FILTERS: &'static [&'static str] =
        &["exported", "latest", "outer", "builtin", "directives"];

    fn visit<D: Doc>(
        builder: &mut Builder<'_>,
        root: &Node<'_, D>,
        _: Reading,
    ) -> Result<(), BuildError> {
        sql::visit(builder, root)
    }

    /// The visit resolved nothing ahead: a reference resolves on demand.
    fn finish<D: Doc>(_: &mut ScopeTable, _: &Node<'_, D>) {}

    fn resolve<'t>(table: &'t ScopeTable, reference: &Reference, _: &str) -> Option<&'t Binding> {
        let (scope, name, index) = table
            .sql
            .as_deref()?
            .resolved(reference.start, reference.end)?;
        table.scopes[scope as usize].bindings.get(name)?.get(index)
    }

    /// A relation carries no value.
    fn binding<'t>(
        table: &'t ScopeTable,
        reference: &Reference,
        name: &str,
    ) -> Option<(&'t Binding, Option<NodeRef>)> {
        Self::resolve(table, reference, name).map(|binding| (binding, None))
    }

    /// `only`: the name has that one binding in its block.
    fn binding_by<'t>(
        table: &'t ScopeTable,
        reference: &Reference,
        name: &str,
        lookup: Lookup,
    ) -> Option<(&'t Binding, Option<NodeRef>)> {
        let (scope, key, _) = table
            .sql
            .as_deref()?
            .resolved(reference.start, reference.end)?;
        let single = table.scopes[scope as usize]
            .bindings
            .get(key)
            .is_some_and(|chain| chain.len() == 1);
        Self::binding(table, reference, name).filter(|_| !lookup.only || single)
    }

    fn index(
        table: &ScopeTable,
        reference: &Reference,
        _: &str,
        _: Lookup,
    ) -> Option<(u32, usize)> {
        let (scope, _, index) = table
            .sql
            .as_deref()?
            .resolved(reference.start, reference.end)?;
        Some((scope, index))
    }

    fn undecided(_: &ScopeTable, _: &Reference) -> bool {
        false
    }

    fn unbound_annotation(_: &ScopeTable, _: &Reference, _: &str) -> Option<(u32, usize)> {
        None
    }

    /// RF01: a `SELECT`'s qualifier no relation of its block or an enclosing one names. RF01 reads
    /// no `UPDATE`, `DELETE` or `INSERT` clause of its own.
    fn unresolved(
        table: &ScopeTable,
        reference: &Reference,
        _: &str,
        mode: Unresolved,
        _: &PythonOptions,
        _: Option<&LanguageOptions>,
        _: Option<&Directives>,
    ) -> bool {
        mode == Unresolved::Unbound
            && table
                .sql
                .as_deref()
                .is_some_and(|sql| sql.reported(reference.start, reference.end))
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

    fn usage<D: Doc>(
        table: &ScopeTable,
        _: &Node<'_, D>,
        steps: &mut Steps<'_>,
    ) -> Result<Usage, BuildError> {
        Usage::sql(table, steps)
    }

    fn unpackaged_module(_: &str) -> Option<ModulePath> {
        None
    }

    fn feature(feature: ScopeFeature) -> Result<(), Refusal> {
        match feature {
            ScopeFeature::This => refused("`this` is JavaScript's; SQL has none"),
            ScopeFeature::SloppyFunctions => {
                refused("sloppy-mode functions are JavaScript's; SQL has none")
            }
            ScopeFeature::GlobalComments => {
                refused("/* global */ comments are ESLint's; SQL has none")
            }
            ScopeFeature::Declared => refused(
                "declared globals come from ESLint's and Ruff's configuration; SQL has none",
            ),
            ScopeFeature::ScriptDeclarations => refused("scripts are ESLint's; SQL has none"),
            ScopeFeature::Relative => {
                refused("relative imports are Python's; a SQL file imports nothing")
            }
            ScopeFeature::SeenBefore => {
                refused("Ruff's `seen_module` reads Python's imports; a SQL file imports nothing")
            }
            ScopeFeature::BeforeLocal => refused(
                "reads Python's binding order; a SQL query block's relations bind for all of its clauses",
            ),
            ScopeFeature::StarImports => refused(
                "\"star\" asks whether a Python star import binds the name; SQL policies ask `unresolved: true`",
            ),
            ScopeFeature::Imports => {
                refused("import resolution is Python's; a SQL file imports nothing")
            }
            ScopeFeature::StopAtGlobal | ScopeFeature::ThroughGlobal => {
                refused("`global` statements are Python's; SQL has none")
            }
            ScopeFeature::Occurrence => {
                refused("a variable's occurrences are TypeScript's and PHP's; SQL has none")
            }
            ScopeFeature::LastWrite => refused(
                "where ESLint reports an unused variable is TypeScript's; SQL's unused relation is `binds: { used: false }`",
            ),
            ScopeFeature::DestructuredValue => {
                refused("destructured values are TypeScript's; a SQL relation carries no value")
            }
            ScopeFeature::SameImport => {
                refused("qualified import names are Python's; SQL has none")
            }
            ScopeFeature::Filter(key) => {
                let what = match key {
                    "exported" => "Python's re-exports and TypeScript's exports",
                    "latest" | "outer" => "Python's binding order and declarations",
                    "builtin" => "ESLint's configured globals",
                    "directives" => "ESLint's /* exported */ comments",
                    _ => return Ok(()),
                };
                refused(format!("`{key}` reads {what}; SQL has none"))
            }
            ScopeFeature::Every => Ok(()),
        }
    }
}

/// SQL's literals and names (`value/sql.rs`), where PostgreSQL's and SQLite's readings agree.
impl ValueRules for Sql {
    const SUPPORTED: bool = true;
    const LANG: Family = Family::Sql;
    const EXPRESSION_KINDS: &'static [&'static str] = &[
        "array",
        "between_expression",
        "binary_expression",
        "case",
        "cast",
        "exists",
        "field",
        "interval",
        "invocation",
        "list",
        "literal",
        "object_id",
        "parameter",
        "parenthesized_expression",
        "subquery",
        "subscript",
        "system_variable",
        "term",
        "unary_expression",
        "window_function",
    ];
    const VALUE_KINDS: &'static [&'static str] = &["literal", "parenthesized_expression", "term"];
    /// A SQL name holds no value a reading follows.
    const NAME_KINDS: &'static [&'static str] = &[];

    fn truthy(value: &Value) -> Option<bool> {
        value::sql::truthy(value)
    }

    /// `type` is refused (`feature`).
    fn type_name(_: &Value) -> Option<&'static str> {
        None
    }

    fn equals(left: &Value, right: &Value) -> Option<bool> {
        value::sql::equals(left, right)
    }

    fn order(left: &Value, right: &Value) -> Option<Option<Ordering>> {
        value::sql::order(left, right)
    }

    /// Values that do not order fail an ordering.
    fn relation(operator: &str, left: &Value, right: &Value) -> Option<bool> {
        Self::order(left, right).map(|order| {
            order.is_some_and(|order| match operator {
                "<" => order == Ordering::Less,
                "<=" => order != Ordering::Greater,
                ">" => order == Ordering::Greater,
                _ => order != Ordering::Less,
            })
        })
    }

    fn numeric(value: &Value) -> bool {
        matches!(value, Value::Int(_) | Value::Float(_))
    }

    fn integer(value: &Value) -> Option<BigInt> {
        match value {
            Value::Int(value) => Some(value.clone()),
            _ => None,
        }
    }

    fn to_str<D: Doc>(value: &Value, _: &Node<'_, D>) -> Option<String> {
        value::sql::to_str(value)
    }

    fn decode<D: Doc>(node: &Node<'_, D>, _: &mut Context<'_>) -> Value {
        value::sql::decode(node)
    }

    fn structure<D: Doc>(_: &Node<'_, D>, _: Equality, _: &mut Context<'_>) -> Option<Vec<Tok>> {
        unreachable!("the structural modes are refused for this language when the policy compiles")
    }

    fn string(text: &str) -> Value {
        Value::Str(text.to_owned())
    }

    /// A JSON integer is an integer literal's value.
    fn number(number: &serde_json::Number) -> Value {
        match (number.as_i64(), number.as_u64()) {
            (Some(integer), _) => Value::int(integer),
            (None, Some(integer)) => Value::int(integer),
            (None, None) => Value::Float(number.as_f64().unwrap_or(f64::NAN)),
        }
    }

    fn spelled<D: Doc>(_: &Node<'_, D>) -> bool {
        true
    }

    /// The name PostgreSQL looks an identifier up by: folded, or quoted and exact.
    fn node_name<D: Doc>(node: &Node<'_, D>) -> Option<Vec<u16>> {
        value::sql::node_name(node).map(|name| name.encode_utf16().collect())
    }

    fn key_name<'r, D: Doc>(
        _: &Node<'r, D>,
        _: &mut dyn FnMut(&Node<'r, D>) -> Value,
    ) -> Option<Vec<u16>> {
        unreachable!("`key` is refused for this language when the policy compiles")
    }

    fn member_name<'r, D: Doc>(
        _: &Node<'r, D>,
        _: &mut dyn FnMut(&Node<'r, D>) -> Value,
    ) -> Option<Vec<u16>> {
        unreachable!("`member` is refused for this language when the policy compiles")
    }

    fn same_reference<D: Doc>(_: &Node<'_, D>, _: &Node<'_, D>, _: bool) -> Option<bool> {
        unreachable!("`reference` is refused for this language when the policy compiles")
    }

    fn feature(feature: ValueFeature<'_>) -> Result<(), Refusal> {
        match feature {
            ValueFeature::Type(_) => refused(
                "a SQL literal's type is its dialect's: PostgreSQL's pg_typeof and SQLite's typeof name different types (1.5 is numeric or real, 'a' unknown or text)",
            ),
            ValueFeature::Compare(text, name) => match text {
                SemanticText::Key | SemanticText::Member => refused(format!(
                    "`{name}` reads JavaScript and PHP property names; SQL has none"
                )),
                SemanticText::Reference { .. } => refused(format!(
                    "`{name}` reads JavaScript and PHP references (ESLint's isSameReference); SQL has none"
                )),
                SemanticText::Structure(_) => refused(
                    "the structural modes compare Python and PHP syntax; use tokens for SQL",
                ),
                SemanticText::Value | SemanticText::Name => Ok(()),
            },
        }
    }
}

/// PostgreSQL's call binding: by position, then by name (`Arguments::sql`).
impl ArgumentRules for Sql {
    const SUPPORTED: bool = true;
    const CALL_KINDS: &'static [&'static str] = SQL_CALL_KINDS;

    fn feature(feature: ArgumentFeature) -> Result<(), Refusal> {
        match feature {
            ArgumentFeature::SignatureMarkers => refused(
                "`/` and `*` mark Python's positional-only and keyword-only slots; a SQL parameter may be passed by position or by name",
            ),
            ArgumentFeature::Syntactic => refused(
                "`syntactic` finds arguments as Ruff's find_argument does; SQL's are bound as PostgreSQL binds them",
            ),
            ArgumentFeature::Estree => refused(
                "`estree` reads ESLint's `node.arguments` of ESTree, JavaScript's syntax tree; SQL has none",
            ),
        }
    }

    fn passed<'r, D: Doc>(arguments: &Arguments, call: &Node<'r, D>) -> Option<Vec<Passed<'r, D>>> {
        Some(arguments.sql(call))
    }
}

/// SQL hosts no literal grammar and has no `languageOptions`; `when` reads the document's
/// options alone.
impl SyntaxRules for Sql {
    const DOLLAR_NAMES: bool = false;

    fn feature(feature: SyntaxFeature) -> Result<(), Refusal> {
        match feature {
            SyntaxFeature::LanguageOptions => {
                refused("`languageOptions` is not available for sql policies")
            }
            SyntaxFeature::Parse(grammar) => {
                refused(format!("`{}` parses {}", grammar.name(), grammar.host()))
            }
        }
    }

    fn compile_language_options(
        _: &Json,
        _: &str,
    ) -> napi::Result<(Option<Arc<LanguageOptions>>, Option<Arc<PythonOptions>>)> {
        Ok((None, None))
    }

    fn compile_when(
        value: &Json,
        path: &str,
        _: Option<&LanguageOptions>,
        _: Option<&PythonOptions>,
        document: &Options,
    ) -> napi::Result<bool> {
        options::compile_option_when(value, path, document)
    }

    fn embedded_pattern(_: &str) -> Option<GrammarDialect> {
        None
    }

    fn php_comment(_: GrammarDialect, _: u16) -> bool {
        true
    }
}
