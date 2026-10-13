//! Rust: rustc's name resolution and constant evaluation read from syntax alone, and Clippy's
//! lint-level attributes. Each concern selects the Rust module that implements it: scope
//! (`scope/rust.rs`, `scope/usage/rust.rs`), values (`value/rust.rs`), call arguments
//! (`matcher/arguments/rust.rs`), calls (`calls/rust.rs`) and format macros
//! (`literal/rust_format.rs`, `matcher/format_args/rust.rs`). Names in macro token trees, and what
//! needs types or macro expansion, are not read.

use super::{
    Adapter, ArgumentFeature, ArgumentRules, CallFeature, CallRules, FormatArgFeature,
    FormatArgRules, Refusal, Region, ScopeFeature, ScopeRules, SyntaxFeature, SyntaxRules,
    ValueFeature, ValueRules,
};
use crate::code_rule_dsl::calls::closure::{Graph, Link};
use crate::code_rule_dsl::calls::names::Name;
use crate::code_rule_dsl::calls::summary::{Callee, Summary};
use crate::code_rule_dsl::calls::{self, FileTables};
use crate::code_rule_dsl::compile::config::Options;
use crate::code_rule_dsl::compile::options;
use crate::code_rule_dsl::language::{Family, GrammarDialect};
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::literal::{Grammar, LiteralHost};
use crate::code_rule_dsl::matcher::{
    Arguments, Passed, ScopeName, SemanticText, Style, Unresolved, file_clippy,
};
use crate::code_rule_dsl::model::ViolationSpec;
use crate::code_rule_dsl::module_path::ModulePath;
use crate::code_rule_dsl::scope::options::{LanguageOptions, PythonOptions};
use crate::code_rule_dsl::scope::php_names::PhpNames;
use crate::code_rule_dsl::scope::rust::{self as scope_rust, LABEL, MACRO};
use crate::code_rule_dsl::scope::usage::{Steps, Usage};
use crate::code_rule_dsl::scope::{
    Binding, BuildError, Builder, Directives, Form, Lookup, Reading, Reference, ScopeKind,
    ScopeTable, TYPE,
};
use crate::code_rule_dsl::suppress::{ClippyIndex, Suppress, root};
use crate::code_rule_dsl::value::rust::{FloatTy, IntTy, RustValue, evaluate, evaluate_following};
use crate::code_rule_dsl::value::structure::{Equality, Tok};
use crate::code_rule_dsl::value::{BigInt, Context, NodeRef, Value};
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;
use std::cmp::Ordering;
use std::path::Path;
use std::sync::Arc;

/// Rust's family.
pub(crate) struct Rust;

impl Adapter for Rust {
    const FAMILY: Family = Family::Rust;
    const LABEL: &'static str = "rust";
    const TERMINATORS: Terminators = Terminators::Newlines;
    const PACKAGES: bool = false;
    const DECLARATION_FILES: bool = false;
    type Scope = Self;
    type Value = Self;
    type Arguments = Self;
    type Calls = Self;
    type FormatArgs = Self;

    fn name(text: Cow<'_, str>) -> Cow<'_, str> {
        text
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

    fn module_path(_: &Path) -> Option<serde_json::Value> {
        None
    }

    /// Clippy's and rustc's `#[allow]` and `#[expect]` of a lint the violation lists, on an item
    /// around it or the file, or a `#[clippy::msrv]` there older than the lint needs
    /// (`suppress/clippy.rs`).
    fn suppresses<D: Doc>(
        _: &ViolationSpec,
        suppress: &Suppress,
        node: &Node<'_, D>,
        source: &str,
        _: &LineTable,
        at: usize,
        _: (usize, usize),
    ) -> bool {
        suppress.clippy.as_ref().is_some_and(|clippy| {
            let index = file_clippy(|| ClippyIndex::read(&root(node), source));
            index.suppresses(clippy, at)
        })
    }
}

/// The refusal reporting `message`.
fn refused(message: impl Into<Cow<'static, str>>) -> Result<(), Refusal> {
    Err(Refusal::Refused(message.into()))
}

/// The names the standard prelude, the extern prelude's `std`, `core` and `alloc`, and the
/// language itself give every module (rustc's `std::prelude::rust_2024`), which no lexical
/// binding makes.
const PRELUDE: &[&str] = &[
    "Copy",
    "Send",
    "Sized",
    "Sync",
    "Unpin",
    "Drop",
    "Fn",
    "FnMut",
    "FnOnce",
    "AsyncFn",
    "AsyncFnMut",
    "AsyncFnOnce",
    "drop",
    "align_of",
    "align_of_val",
    "size_of",
    "size_of_val",
    "Box",
    "ToOwned",
    "Clone",
    "PartialEq",
    "PartialOrd",
    "Eq",
    "Ord",
    "AsRef",
    "AsMut",
    "Into",
    "From",
    "Default",
    "Iterator",
    "Extend",
    "IntoIterator",
    "DoubleEndedIterator",
    "ExactSizeIterator",
    "Option",
    "Some",
    "None",
    "Result",
    "Ok",
    "Err",
    "String",
    "ToString",
    "Vec",
    "TryFrom",
    "TryInto",
    "FromIterator",
    "Future",
    "IntoFuture",
    "std",
    "core",
    "alloc",
    "self",
    "Self",
    "'static",
    "'_",
];

impl ScopeRules for Rust {
    const SUPPORTED: bool = true;
    const FLOW: bool = false;
    const STUBS: bool = false;
    const REDEFINES_SHADOWED: bool = false;
    const REFERENCE_KINDS: &'static [&'static str] = &[
        "identifier",
        "type_identifier",
        "self",
        "shorthand_field_identifier",
        "lifetime",
        "label",
    ];
    const ANNOTATION_STRING_KINDS: &'static [&'static str] = &[];
    const NAME_KINDS: &'static [&'static str] = &[
        "identifier",
        "type_identifier",
        "self",
        "shorthand_field_identifier",
        "lifetime",
        "label",
    ];
    const PRIVATE_NAME_KINDS: &'static [&'static str] = &[];
    /// `let` and `const` bind by assignment, `static` by itself; items are definitions, `type`
    /// aliases and `use` imports; generic parameters, `Self`, parameters, `for` and `match` (and
    /// `if let`, `while let`) patterns.
    const FORMS: &'static [(&'static str, Form)] = &[
        ("assignment", Form::Assignment),
        ("static", Form::Static),
        ("definition", Form::Definition),
        ("typeAlias", Form::TypeAlias),
        ("import", Form::Import),
        ("typeParameter", Form::TypeParameter),
        ("implicit", Form::Implicit),
        ("parameter", Form::Parameter),
        ("for", Form::For),
        ("match", Form::Match),
    ];
    /// A file or inline `mod` (`module`), a function's parameters and body (`function`), a
    /// closure's (`closure`), a block, loop or match arm (`block`), an item's generic parameters
    /// (`typeParameters`).
    const SCOPES: &'static [(&'static str, ScopeName)] = &[
        ("module", ScopeName::Kind(ScopeKind::Module)),
        ("function", ScopeName::Kind(ScopeKind::Function)),
        ("closure", ScopeName::Kind(ScopeKind::Lambda)),
        ("block", ScopeName::Kind(ScopeKind::Block)),
        ("typeParameters", ScopeName::Kind(ScopeKind::TypeParams)),
    ];
    const REFUSED_FILTERS: &'static [&'static str] =
        &["exported", "latest", "outer", "builtin", "directives"];
    const IMPORT_PATHS: bool = true;

    fn visit<D: Doc>(
        builder: &mut Builder<'_>,
        root: &Node<'_, D>,
        _: Reading,
    ) -> Result<(), BuildError> {
        scope_rust::visit_source_file(builder, root)
    }

    fn finish<D: Doc>(table: &mut ScopeTable, root: &Node<'_, D>) {
        scope_rust::finish(table, root);
    }

    fn resolve<'t>(
        table: &'t ScopeTable,
        reference: &Reference,
        name: &str,
    ) -> Option<&'t Binding> {
        let found = scope_rust::resolve_index(table, reference, name)?;
        Some(table.binding_at(found, name))
    }

    /// A value where the binding carries one (a `let` or `const` initializer) and no assignment
    /// writes the name: a `let mut` variable reassigned later holds another value.
    fn binding<'t>(
        table: &'t ScopeTable,
        reference: &Reference,
        name: &str,
    ) -> Option<(&'t Binding, Option<NodeRef>)> {
        let binding = Self::resolve(table, reference, name)?;
        Some((binding, binding.site.value.filter(|_| binding.writes == 0)))
    }

    /// `only`: the name has that one binding in its scope.
    fn binding_by<'t>(
        table: &'t ScopeTable,
        reference: &Reference,
        name: &str,
        lookup: Lookup,
    ) -> Option<(&'t Binding, Option<NodeRef>)> {
        let (scope, _) = scope_rust::resolve_index(table, reference, name)?;
        let single = table.scopes[scope as usize]
            .bindings
            .get(name)
            .is_some_and(|bindings| bindings.len() == 1);
        Self::binding(table, reference, name).filter(|_| !lookup.only || single)
    }

    fn index(
        table: &ScopeTable,
        reference: &Reference,
        name: &str,
        _: Lookup,
    ) -> Option<(u32, usize)> {
        scope_rust::resolve_index(table, reference, name)
    }

    fn undecided(_: &ScopeTable, _: &Reference) -> bool {
        false
    }

    fn unbound_annotation(_: &ScopeTable, _: &Reference, _: &str) -> Option<(u32, usize)> {
        None
    }

    /// A read no binding, prelude name or glob import may give (rustc's E0425, E0412): not a
    /// macro (a `#[macro_use]` crate's are no file's bindings), and not a lowercase type-namespace
    /// name, which may be an extern crate the manifest names.
    fn unresolved(
        table: &ScopeTable,
        reference: &Reference,
        name: &str,
        mode: Unresolved,
        _: &PythonOptions,
        _: Option<&LanguageOptions>,
        _: Option<&Directives>,
    ) -> bool {
        let extern_crate =
            reference.spaces & TYPE != 0 && name.starts_with(|c: char| c.is_ascii_lowercase());
        mode == Unresolved::Unbound
            && reference.spaces & (MACRO | LABEL) == 0
            && !PRELUDE.contains(&name)
            && !extern_crate
            && !scope_rust::glob_visible(table, reference)
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
        root: &Node<'_, D>,
        steps: &mut Steps<'_>,
    ) -> Result<Usage, BuildError> {
        Usage::rust(table, root, steps)
    }

    fn unpackaged_module(_: &str) -> Option<ModulePath> {
        None
    }

    fn import_path<D: Doc>(
        table: &ScopeTable,
        node: &Node<'_, D>,
        file: Option<&str>,
    ) -> Option<String> {
        scope_rust::import_path(table, node, file)
    }

    fn feature(feature: ScopeFeature) -> Result<(), Refusal> {
        match feature {
            ScopeFeature::This => refused(
                "`this` asks whether JavaScript's `this` is the global object; Rust has none (`self` is a method's parameter, which `binding` reads)",
            ),
            ScopeFeature::SloppyFunctions => {
                refused("sloppy-mode functions are JavaScript's; Rust has none")
            }
            ScopeFeature::GlobalComments => {
                refused("/* global */ comments are ESLint's; Rust has none")
            }
            ScopeFeature::Declared => refused(
                "declared globals come from ESLint's and Ruff's configuration; Rust's prelude is fixed, and no languageOptions configure others",
            ),
            ScopeFeature::ScriptDeclarations => refused("scripts are ESLint's; Rust has none"),
            ScopeFeature::Relative => refused(
                "relative imports are Python's; a Rust `use` path's `self::`, `super::` and `crate::` are read as written, which `import` lists",
            ),
            ScopeFeature::SeenBefore => refused(
                "Ruff's `seen_module` asks whether a module was imported before; a Rust `use` only names an item for its module and loads nothing",
            ),
            ScopeFeature::BeforeLocal => refused(
                "`beforeLocal` reads Python's binding order; a Rust function sees no local of an enclosing one, which `binding` reads",
            ),
            ScopeFeature::StarImports => refused(
                "\"star\" asks whether a Python star import binds the name; what a Rust glob import binds is the imported module's, so `unresolved: true` reads no name a glob may give",
            ),
            ScopeFeature::StopAtGlobal | ScopeFeature::ThroughGlobal => {
                refused("`global` statements are Python's; Rust has none")
            }
            ScopeFeature::LastWrite => {
                refused("where ESLint reports a variable is TypeScript's; Rust has none")
            }
            ScopeFeature::DestructuredValue => refused(
                "destructured values are TypeScript's; a Rust pattern's names carry no value of their own",
            ),
            ScopeFeature::Filter(
                key @ ("exported" | "latest" | "outer" | "builtin" | "directives"),
            ) => {
                let what = match key {
                    "exported" => "Python's re-exports and TypeScript's exports",
                    "latest" | "outer" => "Python's binding order and declarations",
                    "builtin" => "ESLint's configured globals",
                    _ => "ESLint's /* exported */ comments",
                };
                refused(format!("`{key}` reads {what}; Rust has none"))
            }
            ScopeFeature::Imports
            | ScopeFeature::Occurrence
            | ScopeFeature::Filter(_)
            | ScopeFeature::Every
            | ScopeFeature::SameImport => Ok(()),
        }
    }
}

/// Type names `type` accepts: a literal's type as rustc gives it (an unsuffixed number's is
/// inferred, so it has none here).
const RUST_TYPES: &[&str] = &[
    "bool", "char", "&str", "&[u8]", "&CStr", "i8", "i16", "i32", "i64", "i128", "isize", "u8",
    "u16", "u32", "u64", "u128", "usize", "f32", "f64",
];

/// A decoded Rust value.
fn value(value: RustValue) -> Value {
    match value {
        RustValue::Unknown => Value::Unknown,
        RustValue::Bool(value) => Value::Bool(value),
        RustValue::Int { value, ty } => Value::RustInt { value, ty },
        RustValue::Float { value, ty } => Value::RustFloat { value, ty },
        RustValue::Char(value) => Value::Char(value),
        RustValue::Str(value) => Value::Str(value),
        RustValue::Bytes(value) => Value::Bytes(value),
        RustValue::CStr(value) => Value::CStr(value),
    }
}

/// An integer and its type; a literal grammar's index (`Value::Int`) has none.
fn int(value: &Value) -> Option<(&BigInt, Option<IntTy>)> {
    match value {
        Value::RustInt { value, ty } => Some((value, *ty)),
        Value::Int(value) => Some((value, None)),
        _ => None,
    }
}

/// A float and its type.
fn float(value: &Value) -> Option<(f64, Option<FloatTy>)> {
    match value {
        Value::RustFloat { value, ty } => Some((*value, *ty)),
        Value::Float(value) => Some((*value, None)),
        _ => None,
    }
}

/// Whether two types may be one: equal, or one not known.
fn compatible<T: PartialEq>(left: Option<T>, right: Option<T>) -> bool {
    left.zip(right).is_none_or(|(left, right)| left == right)
}

impl ValueRules for Rust {
    const SUPPORTED: bool = true;
    const LANG: Family = Family::Rust;
    const EXPRESSION_KINDS: &'static [&'static str] = &[
        "boolean_literal",
        "char_literal",
        "float_literal",
        "integer_literal",
        "raw_string_literal",
        "string_literal",
        "array_expression",
        "assignment_expression",
        "async_block",
        "await_expression",
        "binary_expression",
        "block",
        "break_expression",
        "call_expression",
        "closure_expression",
        "compound_assignment_expr",
        "const_block",
        "continue_expression",
        "field_expression",
        "for_expression",
        "gen_block",
        "generic_function",
        "identifier",
        "if_expression",
        "index_expression",
        "loop_expression",
        "macro_invocation",
        "match_expression",
        "parenthesized_expression",
        "range_expression",
        "reference_expression",
        "return_expression",
        "scoped_identifier",
        "self",
        "struct_expression",
        "try_block",
        "try_expression",
        "tuple_expression",
        "type_cast_expression",
        "unary_expression",
        "unit_expression",
        "unsafe_block",
        "while_expression",
        "yield_expression",
    ];
    /// What `value/rust.rs` folds: literals through operators, casts, parentheses, constant
    /// `if` and blocks, and (`follow`) a name of a `const` item.
    const VALUE_KINDS: &'static [&'static str] = &[
        "boolean_literal",
        "char_literal",
        "float_literal",
        "integer_literal",
        "raw_string_literal",
        "string_literal",
        "parenthesized_expression",
        "unary_expression",
        "binary_expression",
        "type_cast_expression",
        "if_expression",
        "block",
        "identifier",
    ];
    const NAME_KINDS: &'static [&'static str] = &["identifier"];

    /// Only a `bool` is a condition.
    fn truthy(value: &Value) -> Option<bool> {
        match value {
            Value::Bool(value) => Some(*value),
            _ => None,
        }
    }

    fn type_name(value: &Value) -> Option<&'static str> {
        Some(match value {
            Value::Bool(_) => "bool",
            Value::Char(_) => "char",
            Value::Str(_) => "&str",
            Value::Bytes(_) => "&[u8]",
            Value::CStr(_) => "&CStr",
            Value::RustInt { ty: Some(ty), .. } => ty.name(),
            Value::RustFloat { ty: Some(ty), .. } => ty.name(),
            _ => return None,
        })
    }

    /// `==` of two values of one type; values of two types never compare equal (rustc refuses
    /// the comparison). An unsuffixed number's type is the other's.
    fn equals(left: &Value, right: &Value) -> Option<bool> {
        if !left.is_known() || !right.is_known() {
            return None;
        }
        if let (Some((left, lt)), Some((right, rt))) = (int(left), int(right)) {
            return Some(compatible(lt, rt) && left == right);
        }
        if let (Some((left, lt)), Some((right, rt))) = (float(left), float(right)) {
            return Some(compatible(lt, rt) && left == right);
        }
        Some(match (left, right) {
            (Value::Bool(left), Value::Bool(right)) => left == right,
            (Value::Char(left), Value::Char(right)) => left == right,
            (Value::Str(left), Value::Str(right)) => left == right,
            (Value::Bytes(left), Value::Bytes(right)) | (Value::CStr(left), Value::CStr(right)) => {
                left == right
            }
            _ => false,
        })
    }

    /// `PartialOrd` of two values of one type: integers and floats numerically (NaN orders with
    /// nothing), `false < true`, chars by code point, strings and bytes lexicographically.
    fn order(left: &Value, right: &Value) -> Option<Option<Ordering>> {
        if !left.is_known() || !right.is_known() {
            return None;
        }
        if let (Some((left, lt)), Some((right, rt))) = (int(left), int(right)) {
            return Some(compatible(lt, rt).then(|| left.cmp(right)));
        }
        if let (Some((left, lt)), Some((right, rt))) = (float(left), float(right)) {
            return Some(
                compatible(lt, rt)
                    .then(|| left.partial_cmp(&right))
                    .flatten(),
            );
        }
        Some(match (left, right) {
            (Value::Bool(left), Value::Bool(right)) => Some(left.cmp(right)),
            (Value::Char(left), Value::Char(right)) => Some(left.cmp(right)),
            (Value::Str(left), Value::Str(right)) => Some(left.cmp(right)),
            (Value::Bytes(left), Value::Bytes(right)) | (Value::CStr(left), Value::CStr(right)) => {
                Some(left.cmp(right))
            }
            _ => None,
        })
    }

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
        int(value).is_some() || float(value).is_some()
    }

    fn integer(value: &Value) -> Option<BigInt> {
        int(value).map(|(value, _)| value.clone())
    }

    /// `Display` of a scalar: `format!("{}", v)`.
    fn to_str<D: Doc>(value: &Value, _: &Node<'_, D>) -> Option<String> {
        Some(match value {
            Value::Bool(value) => value.to_string(),
            Value::Char(value) => value.to_string(),
            Value::Str(value) => value.clone(),
            Value::RustInt { value, .. } | Value::Int(value) => value.to_string(),
            Value::RustFloat {
                value,
                ty: Some(FloatTy::F32),
            } => (*value as f32).to_string(),
            Value::RustFloat { value, .. } => value.to_string(),
            _ => return None,
        })
    }

    /// `value/rust.rs`: `evaluate`, or `evaluate_following` through the file's table where the
    /// reading follows names.
    fn decode<D: Doc>(node: &Node<'_, D>, context: &mut Context<'_>) -> Value {
        if !context.step() {
            return Value::Unknown;
        }
        match context.table() {
            Some(table) => value(evaluate_following(node, &table)),
            None => value(evaluate(node)),
        }
    }

    fn structure<D: Doc>(_: &Node<'_, D>, _: Equality, _: &mut Context<'_>) -> Option<Vec<Tok>> {
        unreachable!("the structural modes are refused for this language when the policy compiles")
    }

    fn string(text: &str) -> Value {
        Value::Str(text.to_owned())
    }

    /// An integer of an inferred type, else a float.
    fn number(number: &serde_json::Number) -> Value {
        let integer = number
            .as_i64()
            .map(BigInt::from)
            .or_else(|| number.as_u64().map(BigInt::from));
        match integer {
            Some(value) => Value::RustInt { value, ty: None },
            None => Value::RustFloat {
                value: number.as_f64().unwrap_or(f64::NAN),
                ty: None,
            },
        }
    }

    fn spelled<D: Doc>(_: &Node<'_, D>) -> bool {
        true
    }

    fn node_name<D: Doc>(_: &Node<'_, D>) -> Option<Vec<u16>> {
        unreachable!("`name` is refused for this language when the policy compiles")
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
            ValueFeature::Type(name) if RUST_TYPES.contains(&name) => Ok(()),
            ValueFeature::Type(name) => refused(format!(
                "`{name}` is not a Rust type name; expected one of {}",
                RUST_TYPES.join(", ")
            )),
            ValueFeature::Compare(text, name) => match text {
                SemanticText::Value => Ok(()),
                SemanticText::Key | SemanticText::Member => refused(format!(
                    "`{name}` reads JavaScript and PHP property names; Rust has none"
                )),
                SemanticText::Reference { .. } => refused(format!(
                    "`{name}` reads JavaScript and PHP references (ESLint's isSameReference); use tokens for Rust"
                )),
                SemanticText::Name => refused(
                    "`name` reads the names JavaScript and PHP identifiers and strings write; use tokens for Rust",
                ),
                SemanticText::Structure(_) => refused(
                    "the structural modes compare Python and PHP syntax; use tokens for Rust",
                ),
            },
        }
    }
}

impl ArgumentRules for Rust {
    const SUPPORTED: bool = true;
    const CALL_KINDS: &'static [&'static str] = &["call_expression"];

    fn feature(feature: ArgumentFeature) -> Result<(), Refusal> {
        match feature {
            ArgumentFeature::SignatureMarkers => refused(
                "`/` and `*` mark Python's positional-only and keyword-only slots; Rust passes arguments by position only",
            ),
            ArgumentFeature::Syntactic => refused(
                "`syntactic` finds arguments by position and keyword as Ruff's find_argument does; Rust has no keywords",
            ),
            ArgumentFeature::Estree => refused(
                "`estree` reads ESLint's `node.arguments` of ESTree, JavaScript's syntax tree; rust has none (use language)",
            ),
        }
    }

    fn passed<'r, D: Doc>(arguments: &Arguments, call: &Node<'r, D>) -> Option<Vec<Passed<'r, D>>> {
        Some(arguments.rust(call))
    }
}

/// A crate's functions by path (`calls/rust.rs`): free functions, associated functions and
/// methods of the crate's types, `self.m()` in an inherent `impl`. Method calls on other values,
/// trait dispatch, closures and function pointers need types: they are open.
impl CallRules for Rust {
    const SUPPORTED: bool = true;
    const NAMES: Option<Family> = Some(Family::Rust);
    const CALL_KINDS: &'static [&'static str] = &["call_expression"];
    const FUNCTION_KINDS: &'static [&'static str] = &["function_item"];
    const SEED_LANGUAGE: Option<&'static str> = None;
    const TAPED: bool = false;
    const SUBCLASSES: bool = false;

    fn feature(_: CallFeature) -> Result<(), Refusal> {
        Ok(())
    }

    fn summary<D: Doc>(
        root: &Node<'_, D>,
        path: &str,
        _: Option<&ModulePath>,
        table: Option<&ScopeTable>,
        has_errors: bool,
    ) -> Summary {
        calls::rust::summary(root, path, table, has_errors)
    }

    /// An unread file defines nothing the graph can name: calls into it are open.
    fn unread(_: Option<&ModulePath>) -> Summary {
        Summary::default()
    }

    fn tables(
        table: impl FnOnce() -> Option<Arc<ScopeTable>>,
        _: impl FnOnce() -> Option<Arc<ModulePath>>,
        _: impl FnOnce() -> Option<Arc<PhpNames>>,
    ) -> Option<FileTables> {
        Some(FileTables::Rust(table()?))
    }

    fn callee_at<'t, D: Doc>(
        node: &Node<'t, D>,
        summary: &Summary,
        tables: impl FnOnce() -> Option<FileTables>,
    ) -> Option<Option<Callee>> {
        let Some(FileTables::Rust(table)) = tables() else {
            return None;
        };
        Some(calls::rust::callee_at(node, summary, &table))
    }

    /// A function is linked by its path alone.
    fn definition_keys<I: Iterator<Item = Name>>(
        name: &Name,
        _: u8,
        _: impl FnOnce(&Name) -> I,
        mut key: impl FnMut(Name),
    ) {
        key(name.clone());
    }

    fn label(name: &Name) -> Cow<'_, str> {
        Cow::Borrowed(name.text())
    }

    fn link(graph: &Graph, name: &Name, _: u32, _: &mut Vec<Name>) -> Link {
        graph.link_rust(name)
    }
}

/// `format_args!` and the macros built on it (`print!`, `write!`, `panic!`, `assert!`, ...).
impl FormatArgRules for Rust {
    const SUPPORTED: bool = true;
    const FOLLOWS_STRINGS: bool = false;

    fn feature(feature: FormatArgFeature) -> Result<(), Refusal> {
        match feature {
            FormatArgFeature::Style(Style::Rust) => Ok(()),
            FormatArgFeature::Style(Style::Brace) => refused(
                "`brace` reads Python's `str.format` calls; Rust's format macros are style `rust`",
            ),
            FormatArgFeature::Style(Style::Percent) => refused(
                "`percent` reads Python's `%` formats; Rust's format macros are style `rust`",
            ),
            FormatArgFeature::Style(Style::Printf) => refused(
                "`printf` reads PHP's printf-family calls; Rust's format macros are style `rust`",
            ),
            FormatArgFeature::Style(Style::Scanf) => {
                refused("`scanf` reads PHP's sscanf and fscanf calls; Rust has no scanf formats")
            }
        }
    }
}

/// String literals: format templates.
impl SyntaxRules for Rust {
    const DOLLAR_NAMES: bool = false;

    fn feature(feature: SyntaxFeature) -> Result<(), Refusal> {
        let unhosted = |grammar: Grammar, reason: &str| {
            refused(format!(
                "`{}` parses {}{reason}",
                grammar.name(),
                grammar.host()
            ))
        };
        match feature {
            SyntaxFeature::LanguageOptions => {
                refused("`languageOptions` is not available for rust policies")
            }
            SyntaxFeature::Parse(grammar) => match grammar.literal_host() {
                LiteralHost::RustString => Ok(()),
                LiteralHost::EcmaComment | LiteralHost::PhpComment => unhosted(grammar, ""),
                LiteralHost::PythonString | LiteralHost::PhpString => match grammar {
                    Grammar::Brace | Grammar::Percent | Grammar::Printf => {
                        unhosted(grammar, "; Rust's format templates are `rustFormat`")
                    }
                    Grammar::Scanf => unhosted(grammar, "; Rust has no scanf formats"),
                    _ => unhosted(grammar, ""),
                },
            },
        }
    }

    fn compile_language_options(
        _: &serde_json::Value,
        path: &str,
    ) -> napi::Result<super::syntax::QueryOptions> {
        Err(crate::code_rule_dsl::json::invalid(format!(
            "{path}: `languageOptions` is not available for rust policies"
        )))
    }

    fn compile_when(
        value: &serde_json::Value,
        path: &str,
        options: Option<&LanguageOptions>,
        _: Option<&PythonOptions>,
        document: &Options,
    ) -> napi::Result<bool> {
        options::compile_ecma_when(value, path, options, document)
    }

    fn embedded_pattern(_: &str) -> Option<GrammarDialect> {
        None
    }

    fn php_comment(_: GrammarDialect, _: u16) -> bool {
        false
    }
}
