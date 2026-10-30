use super::GrammarMatcher;
use super::util::{UNKNOWN, UTIL_EXCEEDED, UTIL_STEPS_LEFT, UtilLimit};
use crate::code_rule_dsl::budget;
use crate::code_rule_dsl::identifier::{name_in, python_identifier};
use crate::code_rule_dsl::language::adapter::{Adapter, ScopeRules, with_adapter};
use crate::code_rule_dsl::language::{Family, GrammarDialect};
use crate::code_rule_dsl::language::{
    JsxRuntime, SourceType, has_module_syntax, is_stub_file, jsx_runtime_in_source,
};
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::literal::{Grammar, SubTree};
use crate::code_rule_dsl::model::FileInput;
use crate::code_rule_dsl::module_path::ModulePath;
use crate::code_rule_dsl::package_manifest::Manifest;
use crate::code_rule_dsl::scope::options::{DEFAULT_PYTHON_MINOR, LanguageOptions, PythonOptions};
use crate::code_rule_dsl::scope::php_names::{self, PhpNames, Resolved};
use crate::code_rule_dsl::scope::usage::Usage;
use crate::code_rule_dsl::scope::{Binding, BuildError, Import, Reading, ScopeTable, TYPE, VALUE};
use crate::code_rule_dsl::suppress::{
    BiomeSuppressions, ClippyIndex, EslintDirectives, NoqaIndex, NosecIndex, OxlintDirectives,
    PhpcsIndex, PhpstanIndex, PsalmIndex, RerailIgnores, SqlfluffIndex, VueDirectives,
};
use crate::code_rule_dsl::value::NodeRef;
use crate::code_rule_dsl::value::structure::{Equality, Tok};
use crate::code_rule_dsl::value::{Followed, Value};
use ast_grep_core::{Doc, Node};
use foldhash::{HashMap, HashMapExt};
use std::cell::{Cell, OnceCell, RefCell};
use std::collections::BTreeSet;
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

/// `resolves`: file-local name resolution (a Rerail extension). It binds and reads no captures.
#[derive(Clone)]
pub(crate) struct Resolves {
    /// `global`: the node is a reference to one of these names that no file-local binding
    /// visible there shadows.
    pub(crate) globals: BTreeSet<String>,
    /// `import`: the node is a name or attribute chain whose head is bound by an import, and the
    /// import's qualified name joined with the rest of the chain is one of these (Python); or a
    /// name or path whose path, its head read through the file's imports, is one of these (Rust,
    /// `ScopeRules::import_path`).
    pub(crate) imports: BTreeSet<String>,
    /// Attribute-chain kinds (Python) or path kinds (Rust), which only `import` accepts.
    pub(crate) attribute_kinds: BTreeSet<u16>,
    /// `directives: "eslint"`: `/* global */` comments add and remove global names.
    pub(crate) directives: bool,
    pub(crate) language: GrammarDialect,
    /// Kinds a matching node can have: the grammar's reference kinds.
    pub(crate) reference_kinds: BTreeSet<u16>,
    /// Parenthesized expressions, which are transparent: Ruff's and ESTree's ASTs have none.
    pub(crate) paren_kinds: BTreeSet<u16>,
    /// The query's `languageOptions`: with them, a `global` name must also be a declared global.
    pub(crate) options: Option<Arc<LanguageOptions>>,
    /// A Python query's `languageOptions`: a `global` name must also be a builtin of its
    /// version, and from 3.14 annotations are read after the module.
    pub(crate) python: Option<Arc<PythonOptions>>,
    /// `declared` (default true): under `languageOptions` a `global` name must also be a declared
    /// global (TypeScript) or a builtin of the version (Python). `false` asks only that no
    /// file-local binding shadows it, as without options.
    pub(crate) declared: bool,
    /// `scriptDeclarations`: in a script, a `global` name bound at the top level is the global
    /// variable itself (eslint-scope's global scope holds a script's top-level declarations), so
    /// it matches whether or not it is declared; parameters and block bindings still shadow.
    pub(crate) script_declarations: bool,
    /// `this`: the node is a `this` of the form below.
    pub(crate) this_global: bool,
    /// `this: "globalScope"` (else `"global"`, whose value is the global object): a `this` that
    /// eslint-scope's global scope holds, so no nested scope does (ESLint's no-alert).
    pub(crate) this_scope: bool,
    /// `sloppyFunctions` (with `this: "global"`): also a `this` whose owner is an ordinary function
    /// (not an arrow, method, accessor or class member) that is not strict, whose default binding
    /// is the global object (ESLint's no-eval).
    pub(crate) sloppy_functions: bool,
    /// `this` expressions, which only `this: "global"` accepts.
    pub(crate) this_kinds: BTreeSet<u16>,
    /// `imported` (Python): the node is a name an import statement imports, and the qualified
    /// name it imports is one of these.
    pub(crate) imported: BTreeSet<String>,
    /// `prefix`: an `import` or `imported` name also matches every name under it.
    pub(crate) prefix: bool,
    /// `relative: "text"`: `imported` names a relative import by its module text, ignoring the
    /// dots (Ruff's T100, F404, F407 read `module` without `level`): `from .a import b` imports
    /// `a.b`, `from . import b` imports `b`. Otherwise (`"never"`) relative imports name nothing.
    pub(crate) relative_text: bool,
    /// `dotted_name`, the kind of the names an import statement holds.
    pub(crate) dotted_kinds: BTreeSet<u16>,
    /// `global: true`: any unshadowed declared global (TypeScript) or builtin (Python), under the
    /// query's `languageOptions` or their defaults.
    pub(crate) global_any: bool,
    /// `import: true` (Python): the name or attribute chain has a qualified name (Ruff's
    /// `resolve_qualified_name(..).is_some()`): its head is an import, a definition the module
    /// path qualifies, or an unshadowed builtin.
    pub(crate) import_any: bool,
    /// `unresolved`: the reference has no binding, is no builtin or declared global, and (Python)
    /// a visible star import may (`Star`) or may not (`Unbound`) bind it.
    pub(crate) unresolved: Option<Unresolved>,
    /// Python string kinds, which `unresolved` reads as a module `__all__` entry's name.
    pub(crate) string_kinds: BTreeSet<u16>,
    /// `beforeLocal` (Python): the reference is its function's first read of the binding that the
    /// function's own later binding of the name shadows (Ruff's F823).
    pub(crate) before_local: bool,
    /// `seenBefore` (Python, with `imported` top-level modules): any node that Ruff visits after an
    /// import of one of them (Ruff's `seen_module`).
    pub(crate) seen_before: bool,
    /// Inside an `eslintDirective` tree: the node's text is a name, which holds where it is listed
    /// (or `global: true`) and a declared global of the query's `languageOptions` (the builtins of
    /// its ecmaVersion and environment, and its configured globals, `off` aside). `/* global */`
    /// comments, which the tree is one of, are not read.
    pub(crate) item: bool,
}

/// Which unresolved references `resolves.unresolved` matches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Unresolved {
    /// `true`: Ruff's F821, ESLint's no-undef.
    Unbound,
    /// `"star"`: Ruff's F405, a name only a star import can bind.
    Star,
}

/// The dotted text of a `dotted_name`, without the spaces or comments Python allows inside it.
fn dotted<D: Doc>(node: &Node<'_, D>) -> String {
    let parts = node.children().filter(|part| part.kind() == "identifier");
    parts
        .map(|part| python_identifier(&part).into_owned())
        .collect::<Vec<_>>()
        .join(".")
}

/// The module an import-from statement names; a relative one only by its text (`relative_text`),
/// which is empty for `from . import x`.
fn from_module<D: Doc>(statement: &Node<'_, D>, relative_text: bool) -> Option<String> {
    if statement.kind() == "future_import_statement" {
        return Some("__future__".to_owned());
    }
    let module = statement.field("module_name")?;
    match module.kind().as_ref() {
        "dotted_name" => Some(dotted(&module)),
        "relative_import" if relative_text => Some(
            module
                .children()
                .find(|part| part.kind() == "dotted_name")
                .map(|name| dotted(&name))
                .unwrap_or_default(),
        ),
        _ => None,
    }
}

/// What `this` means below a node of this kind (TypeScript grammar names).
fn this_boundary(kind: &str) -> Option<ThisBoundary> {
    match kind {
        "function_declaration"
        | "function_expression"
        | "function"
        | "generator_function"
        | "generator_function_declaration"
        | "class_static_block"
        | "internal_module"
        | "module" => Some(ThisBoundary::Own),
        "method_definition" => Some(ThisBoundary::OwnExcept("name")),
        "public_field_definition" | "field_definition" => Some(ThisBoundary::OnlyIn("value")),
        _ => None,
    }
}

/// A function whose `this` a plain call binds by default: not an arrow, method, accessor, static
/// block or field initializer.
fn is_ordinary_function(kind: &str) -> bool {
    matches!(
        kind,
        "function_declaration"
            | "function_expression"
            | "function"
            | "generator_function"
            | "generator_function_declaration"
    )
}

/// Whether code at `node` (a function, or the program) is strict, as eslint-scope's `isStrict`:
/// from ES5, in a module or under `impliedStrict`, inside a class, or where `node` or an enclosing
/// function or the program opens with a 'use strict' directive.
fn is_strict<D: Doc>(options: &LanguageOptions, node: &Node<'_, D>) -> bool {
    if !options.strict_mode() {
        return false;
    }
    if options.all_strict(file_source_type_for(options)) {
        return true;
    }
    std::iter::once(node.clone())
        .chain(node.ancestors())
        .any(|scope| match scope.kind().as_ref() {
            "class_declaration" | "abstract_class_declaration" | "class" => true,
            "program" => has_use_strict(&scope),
            "function_declaration"
            | "function_expression"
            | "function"
            | "generator_function"
            | "generator_function_declaration"
            | "arrow_function"
            | "method_definition" => scope
                .field("body")
                .is_some_and(|body| body.kind() == "statement_block" && has_use_strict(&body)),
            _ => false,
        })
}

/// Whether a body's directive prologue (its leading string-literal statements) holds
/// `'use strict'` or `"use strict"`, spelled exactly so.
fn has_use_strict<D: Doc>(body: &Node<'_, D>) -> bool {
    for statement in body.children().filter(|child| child.is_named()) {
        match statement.kind().as_ref() {
            "comment" | "hash_bang_line" => continue,
            "expression_statement" => {}
            _ => return false,
        }
        let mut parts = statement
            .children()
            .filter(|part| part.is_named() && part.kind() != "comment");
        let (Some(literal), None) = (parts.next(), parts.next()) else {
            return false;
        };
        if literal.kind() != "string" {
            return false;
        }
        let text = literal.text();
        if matches!(text.as_ref(), "'use strict'" | "\"use strict\"") {
            return true;
        }
    }
    false
}

enum ThisBoundary {
    /// Every `this` inside has its own value.
    Own,
    /// Every `this` inside except in this field (a computed method name) has its own value.
    OwnExcept(&'static str),
    /// Only a `this` in this field (a field initializer) has its own value.
    OnlyIn(&'static str),
}

impl Resolves {
    pub(super) fn holds<D: Doc>(&self, node: &Node<'_, D>) -> bool {
        with_adapter!(Family::of(self.language), A => self.holds_in::<<A as Adapter>::Scope, D>(node))
    }

    /// `holds`, where `S` reads the family's scope.
    fn holds_in<S: ScopeRules, D: Doc>(&self, node: &Node<'_, D>) -> bool {
        if S::FLOW {
            return self.php_holds(node);
        }
        if self.item {
            let text = node.text();
            return (self.global_any || self.globals.contains(text.as_ref()))
                && self.options.as_ref().is_none_or(|options| {
                    options.declares_in(&text, VALUE, None, file_source_type_for(options))
                });
        }
        if self.seen_before {
            return self.is_seen_before(node);
        }
        let node = &self.unparenthesized(node.clone());
        let kind = node.kind_id();
        if self.this_global && self.this_kinds.contains(&kind) {
            return if self.this_scope {
                self.is_global_scope_this(node)
            } else {
                self.is_global_this(node)
            };
        }
        if self.dotted_kinds.contains(&kind) {
            return imported_name(node, self.relative_text)
                .is_some_and(|name| self.listed_typing(&self.imported, &name));
        }
        if self.before_local {
            return self.reference_kinds.contains(&kind)
                && !self.attribute_kinds.contains(&kind)
                && self.is_before_local(node);
        }
        if let Some(mode) = self.unresolved
            && self.string_kinds.contains(&kind)
        {
            return self.is_unresolved_export(node, mode);
        }
        if !self.reference_kinds.contains(&kind) {
            return false;
        }
        if let Some(mode) = self.unresolved {
            return !self.attribute_kinds.contains(&kind) && self.is_unresolved::<S, D>(node, mode);
        }
        let chain = (!self.imports.is_empty() || self.import_any)
            && (S::IMPORT_PATHS || self.import_chain(node).is_some());
        let text = name_in(self.language, node.text());
        // Without language options only a comment can make another name global; without the
        // opt-in no table is needed.
        let unlisted = self.directives && self.options.is_none();
        let global = (!self.globals.is_empty() || unlisted || self.global_any)
            && !self.attribute_kinds.contains(&kind)
            && (unlisted || self.global_any || self.globals.contains(text.as_ref()))
            && (!self.declared
                || self
                    .python
                    .as_ref()
                    .is_none_or(|python| python.declares(text.as_ref())));
        if !global && !chain {
            return false;
        }
        let Some(table) = file_table(node, self.language, self.reading()) else {
            return false;
        };
        (global && self.is_global(&table, node, text.as_ref()))
            || (chain && S::IMPORT_PATHS && self.is_import_path::<S, D>(&table, node))
            || (chain && !S::IMPORT_PATHS && self.is_import(&table, node))
    }

    /// Rust: the path the read at `node` names through the file's imports is listed (or with
    /// `import: true`, there is one).
    fn is_import_path<S: ScopeRules, D: Doc>(
        &self,
        table: &ScopeTable,
        node: &Node<'_, D>,
    ) -> bool {
        let file = file_path();
        S::import_path(table, node, file.as_deref()).is_some_and(|path| {
            self.import_any
                || self.imports.contains(&path)
                || (self.prefix
                    && path
                        .match_indices("::")
                        .any(|(at, _)| self.imports.contains(&path[..at])))
        })
    }

    /// Ruff's F821 and F405, ESLint's no-undef: `node` reads a name with no binding (or one an
    /// unconditional `del` unbound) that is no builtin, declared global or implicit name.
    fn is_unresolved<S: ScopeRules, D: Doc>(&self, node: &Node<'_, D>, mode: Unresolved) -> bool {
        let Some(table) = file_table(node, self.language, self.reading()) else {
            return false;
        };
        let Some(reference) = table.reference(&node.range()) else {
            return false;
        };
        let name = name_in(self.language, node.text());
        // A function's bare annotation ends a load: an unbound local, whatever an outer scope,
        // builtin or star import binds, unless a nested `nonlocal` may initialize it (Ruff's
        // `annotation_binding_id` with rebinding scopes).
        if let Some(annotation) = S::unbound_annotation(&table, reference, &name) {
            return mode == Unresolved::Unbound && !table.binding_at(annotation, &name).rebound;
        }
        if let Some(binding) = S::resolve(&table, reference, &name) {
            // Ruff stops at a deletion: the name is unbound, whatever a star import binds.
            return binding.is_deletion() && mode == Unresolved::Unbound;
        }
        S::unresolved(
            &table,
            reference,
            &name,
            mode,
            &self.python_options(),
            self.options.as_deref(),
            self.directives.then_some(&table.directives),
        )
    }

    /// Ruff's `seen_module`: an import of a listed module is visited before `node` in Ruff's order.
    /// A node lies at the depth of the function bodies around it (its parameters, decorators and
    /// defaults are its enclosing body's); a lambda body, a type parameter and a `type` alias's
    /// value are visited after every function body.
    fn is_seen_before<D: Doc>(&self, node: &Node<'_, D>) -> bool {
        let Some(table) = file_table(node, self.language, self.reading()) else {
            return false;
        };
        let mut depth = 0u32;
        let mut late = false;
        let mut child = node.clone();
        for ancestor in node.ancestors() {
            let holds = |field: &str| {
                ancestor
                    .field(field)
                    .is_some_and(|part| part.range() == child.range())
            };
            match ancestor.kind().as_ref() {
                "function_definition" if holds("body") => depth += 1,
                "lambda" if holds("body") => late = true,
                "type_alias_statement" if holds("right") => late = true,
                "type_parameter" => late = true,
                _ => {}
            }
            child = ancestor;
        }
        let depth = if late { u32::MAX } else { depth };
        table.seen_module(&self.imported, depth, node.range().start)
    }

    /// Ruff's F823 (`undefined_local`): the reference is its function's first read of the binding
    /// that the function's own binding of the name, made later, shadows.
    fn is_before_local<D: Doc>(&self, node: &Node<'_, D>) -> bool {
        let Some(table) = file_table(node, self.language, self.reading()) else {
            return false;
        };
        let Some(reference) = table.reference(&node.range()) else {
            return false;
        };
        match file_usage(node, &table).and_then(|usage| usage.before_local(reference.start)) {
            Some(builtin) => !builtin || self.python_options().declares(&python_identifier(node)),
            None => false,
        }
    }

    /// Ruff's `visit_exports`: a string entry of a module `__all__` definition names nothing the
    /// module binds (F822), or only a module star import may bind it (F405).
    fn is_unresolved_export<D: Doc>(&self, node: &Node<'_, D>, mode: Unresolved) -> bool {
        let Some(table) = file_table(node, self.language, self.reading()) else {
            return false;
        };
        let range = node.range();
        if !table.dunder_all.iter().any(|statement| {
            (statement.start as usize) <= range.start && range.end <= statement.end as usize
        }) {
            return false;
        }
        let Some(usage) = file_usage(node, &table) else {
            return false;
        };
        let Some(name) = usage.export_at(range.start, range.end) else {
            return false;
        };
        // Ruff's `global_binding` materializes a builtin of the version, so it counts as bound.
        let module = &table.scopes[0];
        !module.bindings.contains_key(name)
            && !self.python_options().declares(name)
            && module.stars.is_empty() == (mode == Unresolved::Unbound)
    }

    /// The query's Python options, or Ruff's default target.
    fn python_options(&self) -> PythonOptions {
        PythonOptions::new(
            self.python
                .as_ref()
                .map_or(DEFAULT_PYTHON_MINOR, |python| python.minor),
        )
    }

    /// Whether this predicate asks about every name, not listed ones: it has no text probe.
    pub(crate) fn name_less(&self) -> bool {
        self.global_any || self.import_any || self.unresolved.is_some() || self.before_local
    }

    /// How this predicate's table reads the file; see `table_reading`.
    fn reading(&self) -> Reading {
        table_reading(self.options.as_deref(), self.python.as_deref())
    }

    /// A `this` whose value is the global object, as ESLint's no-eval reads it. At the top level
    /// (outside functions, arrows aside, field initializers, static blocks and namespaces): in a
    /// script or CommonJS file (no-eval reads a CommonJS file's top level as a script's, though
    /// Node binds it to `module.exports`); with `globalReturn` only when the top level is not
    /// strict; never in a module, where it is `undefined`. With `sloppyFunctions`, also in an
    /// ordinary function that is not strict.
    fn is_global_this<D: Doc>(&self, node: &Node<'_, D>) -> bool {
        let Some(options) = self.options.as_ref().filter(|options| {
            options.source_type(file_source_type_for(options)) != SourceType::Module
        }) else {
            return false;
        };
        let mut child = node.clone();
        while let Some(parent) = child.parent() {
            let own = match this_boundary(&parent.kind()) {
                None => false,
                Some(ThisBoundary::Own) => true,
                Some(ThisBoundary::OwnExcept(field)) => parent
                    .field(field)
                    .is_none_or(|kept| kept.range() != child.range()),
                Some(ThisBoundary::OnlyIn(field)) => parent
                    .field(field)
                    .is_some_and(|own| own.range() == child.range()),
            };
            if own {
                return self.sloppy_functions
                    && is_ordinary_function(&parent.kind())
                    && !is_strict(options, &parent);
            }
            child = parent;
        }
        !options.global_return || !is_strict(options, &child)
    }

    /// A `this` in the global scope of a script: no ancestor owns an eslint-scope scope. Functions
    /// (arrows too), classes, static blocks, namespaces, enums, catch clauses and with statements
    /// always do; blocks, switch statements and loops that declare `let` or `const` from ES2015.
    /// As ESLint's `getScope`, a node's scope is its nearest scope-owning ancestor, whichever part
    /// of it the node is in (a switch's discriminant, a loop's head, a class's heritage).
    fn is_global_scope_this<D: Doc>(&self, node: &Node<'_, D>) -> bool {
        let Some(options) = self
            .options
            .as_ref()
            .filter(|options| options.global_top_level(file_source_type_for(options)))
        else {
            return false;
        };
        let blocks = options.block_scopes();
        !node
            .ancestors()
            .any(|ancestor| match ancestor.kind().as_ref() {
                "function_declaration"
                | "function_expression"
                | "function"
                | "generator_function"
                | "generator_function_declaration"
                | "arrow_function"
                | "method_definition"
                | "class_declaration"
                | "abstract_class_declaration"
                | "class"
                | "class_static_block"
                | "internal_module"
                | "module"
                | "enum_declaration"
                | "catch_clause"
                | "with_statement" => true,
                "statement_block" | "switch_statement" => blocks,
                "for_statement" => {
                    blocks
                        && ancestor.field("initializer").is_some_and(|initializer| {
                            matches!(
                                initializer.kind().as_ref(),
                                "lexical_declaration" | "using_declaration"
                            )
                        })
                }
                "for_in_statement" => {
                    blocks
                        && ancestor
                            .field("kind")
                            .is_some_and(|kind| matches!(kind.kind().as_ref(), "let" | "const"))
                }
                _ => false,
            })
    }

    fn is_global<D: Doc>(&self, table: &ScopeTable, node: &Node<'_, D>, name: &str) -> bool {
        if self.script_declarations
            && self
                .options
                .as_ref()
                .is_some_and(|options| options.global_top_level(file_source_type_for(options)))
            && let Some(reference) = table.reference(&node.range())
            && let Some((0, _)) = table.resolve_lexical_scope(reference, name)
        {
            return true;
        }
        if !self.declared {
            return table.reference(&node.range()).is_some_and(|reference| {
                table
                    .resolve(reference, name)
                    .is_none_or(|binding| !binding.shadows())
            });
        }
        if let Some(python) = &self.python {
            return python.declares(name)
                && table.reference(&node.range()).is_some_and(|reference| {
                    table
                        .resolve(reference, name)
                        .is_none_or(|binding| !binding.shadows())
                });
        }
        if let Some(options) = &self.options {
            let directives = self.directives.then_some(&table.directives);
            return table.reference(&node.range()).is_some_and(|reference| {
                options.declares_in(
                    name,
                    reference.spaces,
                    directives,
                    file_source_type_for(options),
                ) && table
                    .resolve(reference, name)
                    .is_none_or(|binding| !binding.shadows())
            });
        }
        let listed = if self.directives {
            (self.globals.contains(name) || table.directives.declared.contains(name))
                && !table.directives.off.contains(name)
        } else {
            true
        };
        listed
            && table.reference(&node.range()).is_some_and(|reference| {
                table
                    .resolve(reference, name)
                    .is_none_or(|binding| !binding.shadows())
            })
    }

    fn unparenthesized<'r, D: Doc>(&self, node: Node<'r, D>) -> Node<'r, D> {
        unparenthesized(&self.paren_kinds, node)
    }

    /// The head name of an identifier or attribute chain and the attribute names after it, when
    /// the last one could complete a listed qualified name.
    fn import_chain<'r, D: Doc>(&self, node: &Node<'r, D>) -> Option<(Node<'r, D>, Vec<String>)> {
        let mut tail = Vec::new();
        let mut head = node.clone();
        while self.attribute_kinds.contains(&head.kind_id()) {
            tail.push(name_in(self.language, head.field("attribute")?.text()).into_owned());
            head = self.unparenthesized(head.field("object")?);
        }
        if !self.reference_kinds.contains(&head.kind_id()) {
            return None;
        }
        tail.reverse();
        if let Some(last) = tail.last()
            && !self.prefix
            && !self.import_any
            && !self
                .imports
                .iter()
                .any(|name| name.rsplit('.').next() == Some(last.as_str()))
        {
            return None;
        }
        Some((head, tail))
    }

    fn is_import<D: Doc>(&self, table: &ScopeTable, node: &Node<'_, D>) -> bool {
        let Some((head, tail)) = self.import_chain(node) else {
            return false;
        };
        let name = name_in(self.language, head.text());
        let Some(reference) = table.reference(&head.range()) else {
            return false;
        };
        let resolved = table.resolve(reference, &name);
        // Ruff qualifies an unshadowed builtin too (`["", name]`).
        if self.import_any && resolved.is_none() {
            return self.python_options().declares(&name);
        }
        let Some(Binding {
            import: Some(import),
            ..
        }) = resolved
        else {
            return false;
        };
        let full = match import {
            Import::Qualified(qualified) => {
                let mut full = qualified.to_string();
                for part in &tail {
                    full.push('.');
                    full.push_str(part);
                }
                Some(full)
            }
            Import::Relative { level, rest } => {
                file_module().and_then(|module| module.relative(*level, rest, &tail))
            }
            Import::Definition => file_module().and_then(|module| module.definition(&name, &tail)),
        };
        full.is_some_and(|full| self.import_any || self.listed_typing(&self.imports, &full))
    }

    /// Whether the predicate reads only the import statement around its node: `imported` alone,
    /// which builds no scope table and ignores the module path (`relative: "text"` included). As
    /// sound under syntax errors as a pattern, when positive.
    pub(crate) fn witness_only(&self) -> bool {
        // `imported` reads dotted names alone (`dotted_kinds`: Python's, without `seenBefore`).
        !self.dotted_kinds.is_empty()
            && self.globals.is_empty()
            && self.imports.is_empty()
            && !self.this_global
            && !self.directives
            && !self.name_less()
            && !self.seen_before
    }

    /// Whether `name` is listed, or is a typing module's member whose `typing.<member>` is (Ruff's
    /// `match_typing_qualified_name` under `lint.typing-modules`: the member is typing's own).
    fn listed_typing(&self, names: &BTreeSet<String>, name: &str) -> bool {
        if self.listed(names, name) {
            return true;
        }
        let Some(typing) = self.python.as_ref().and_then(|python| python.typing) else {
            return false;
        };
        let parts = name.split('.').map(str::to_owned).collect::<Vec<_>>();
        parts.last().is_some_and(|member| {
            names.contains(&format!("typing.{member}")) && typing.module_member(&parts, member)
        })
    }

    /// Whether `name` is listed, or with `prefix` lies under a listed name.
    fn listed(&self, names: &BTreeSet<String>, name: &str) -> bool {
        names.contains(name)
            || (self.prefix
                && name
                    .match_indices('.')
                    .any(|(dot, _)| names.contains(&name[..dot])))
    }

    /// PHP: the node is a class, function or constant name that resolves to a listed name (any
    /// resolved name with `import: true`), or (`imported`) the name a `use` clause imports, whose
    /// fully qualified name is listed. `self`, `static` and `parent` resolve to none; an
    /// unqualified function or constant PHP resolves only at run time is unknown unless this file
    /// declares it or PHP defines it (`scope/php_names.rs`).
    fn php_holds<D: Doc>(&self, node: &Node<'_, D>) -> bool {
        let node = self.unparenthesized(node.clone());
        if node.kind() == "variable_name" {
            return self.reference_kinds.contains(&node.kind_id()) && self.php_variable(&node);
        }
        // A `name` holding an expression (`X::{expr}`) is no name.
        if !self.reference_kinds.contains(&node.kind_id())
            || (node.kind() == "name" && node.children().any(|child| child.is_named()))
        {
            return false;
        }
        let Some(names) = file_php_names(&node, self.language) else {
            return false;
        };
        if let Some((kind, name)) = names.import(&node.range()) {
            return php_names::listed(&self.imported, kind, name, self.prefix);
        }
        if self.imports.is_empty() && !self.import_any {
            return false;
        }
        let Some(reference) = names.reference(&node.range()) else {
            return false;
        };
        let listed = |kind, name: &str| {
            self.import_any || php_names::listed(&self.imports, kind, name, self.prefix)
        };
        let verdict = match (&reference.resolved, reference.kind) {
            (Resolved::Name(name), Some(kind)) => Some(listed(kind, name)),
            (Resolved::Special, _) => Some(false),
            // PHP decides at run time; both candidates listed, or neither, decide here.
            (Resolved::Either { namespaced, global }, Some(kind)) => {
                let namespaced = listed(kind, namespaced);
                (namespaced == listed(kind, global)).then_some(namespaced)
            }
            _ => None,
        };
        verdict.unwrap_or_else(|| {
            UNKNOWN.set(true);
            false
        })
    }
}

impl Resolves {
    /// A PHP variable read: `global`, a superglobal (listed, or any with `global: true`);
    /// `unresolved`, a read PHPStan reports undefined because no path defines it. A read `isset`,
    /// `empty` or `??` guards is never unresolved; one a callee may take by reference, or one some
    /// paths define, is unknown.
    fn php_variable<D: Doc>(&self, node: &Node<'_, D>) -> bool {
        use crate::code_rule_dsl::scope::php::{Defined, Quiet};
        let Some(table) = file_table(node, self.language, Reading::default()) else {
            return false;
        };
        let Some(read) = table
            .php
            .as_deref()
            .and_then(|flow| flow.read(node.range().start))
        else {
            // A binding's own name, not a read.
            return false;
        };
        if self.global_any || !self.globals.is_empty() {
            return read.superglobal
                && (self.global_any || self.globals.contains(read.name.as_ref()));
        }
        if self.unresolved.is_none() || read.superglobal {
            return false;
        }
        match (read.quiet, read.defined) {
            (Quiet::Silent, _) | (_, Defined::Yes) => false,
            (Quiet::No, Defined::No) => true,
            _ => {
                UNKNOWN.set(true);
                false
            }
        }
    }
}

/// The expression inside any parentheses (of `paren_kinds`) around `node`.
pub(super) fn unparenthesized<'r, D: Doc>(
    paren_kinds: &BTreeSet<u16>,
    mut node: Node<'r, D>,
) -> Node<'r, D> {
    while paren_kinds.contains(&node.kind_id()) {
        let mut inner = node
            .children()
            .filter(|child| child.is_named() && !super::util::is_comment_kind(&child.kind()));
        let (Some(expression), None) = (inner.next(), inner.next()) else {
            break;
        };
        drop(inner);
        node = expression;
    }
    node
}

/// The qualified name an import statement imports where `node`, a `dotted_name`, stands:
/// `import a.b` imports `a.b`, `import a.b as c` too; `from m import x as y` names `m` and imports
/// `m.x`. Relative modules name nothing, or with `relative_text` their text without the dots.
/// Nothing either when the statement has a syntax error before the name ends: tree-sitter keeps
/// `os` of `from os.$ import x` as the module and drops the stray dot of `a..b`, so the name
/// would read as one the statement does not import. It reads no other node, which keeps
/// `imported` sound under errors (see `Resolves::witness_only`).
fn imported_name<D: Doc>(node: &Node<'_, D>, relative_text: bool) -> Option<String> {
    let name = statement_name(node, relative_text)?;
    let end = node.range().end;
    let statement = node.ancestors().find(|ancestor| {
        matches!(
            ancestor.kind().as_ref(),
            "import_statement" | "import_from_statement" | "future_import_statement"
        )
    })?;
    let broken = statement
        .dfs()
        .any(|part| (part.is_error() || part.is_missing()) && part.range().start <= end);
    (!broken).then_some(name)
}

fn statement_name<D: Doc>(node: &Node<'_, D>, relative_text: bool) -> Option<String> {
    let parent = node.parent()?;
    let is = |other: Option<Node<'_, D>>| other.is_some_and(|other| other.range() == node.range());
    let (statement, member) = match parent.kind().as_ref() {
        "import_statement" => return Some(dotted(node)),
        "relative_import" if relative_text => return Some(dotted(node)),
        "aliased_import" if is(parent.field("name")) => {
            let statement = parent.parent()?;
            if statement.kind() == "import_statement" {
                return Some(dotted(node));
            }
            (statement, true)
        }
        "import_from_statement" | "future_import_statement" => {
            let member = !is(parent.field("module_name"));
            (parent, member)
        }
        _ => return None,
    };
    let module = from_module(&statement, relative_text)?;
    Some(match (member, module.is_empty()) {
        (true, true) => dotted(node),
        (true, false) => format!("{module}.{}", dotted(node)),
        (false, _) => module,
    })
}

thread_local! {
    /// The scope table of the file being matched, installed per file by `FileScopes::install`.
    static FILE_SCOPES: RefCell<Option<FileScopes>> = const { RefCell::new(None) };
    /// The clause whose root is being matched (`GrammarMatcher` address), which pays for a table
    /// once per file.
    pub(super) static CURRENT_CLAUSE: Cell<usize> = const { Cell::new(0) };
    #[cfg(test)]
    pub(crate) static TABLE_BUILDS: Cell<usize> = const { Cell::new(0) };
    /// How many times a file's module system was read off its syntax (`syntax_source_type`).
    #[cfg(test)]
    pub(crate) static SOURCE_TYPE_DERIVATIONS: Cell<usize> = const { Cell::new(0) };
    /// How many times a file's JSX runtime was read (`jsx_runtime_of`).
    #[cfg(test)]
    pub(crate) static JSX_RUNTIME_DERIVATIONS: Cell<usize> = const { Cell::new(0) };
    /// The tables whose string type definitions are being matched (`parse: annotation`): a parsed
    /// annotation's names are read in its file's table.
    static SUB_TABLES: RefCell<Vec<Arc<ScopeTable>>> = const { RefCell::new(Vec::new()) };
}

/// Keeps a file's table readable from the nodes of its parsed annotations while they are matched.
pub(super) struct SubTables;

impl SubTables {
    pub(super) fn enter(table: Arc<ScopeTable>) -> Self {
        SUB_TABLES.with_borrow_mut(|tables| tables.push(table));
        Self
    }
}

impl Drop for SubTables {
    fn drop(&mut self) {
        SUB_TABLES.with_borrow_mut(|tables| tables.pop());
    }
}

struct FileScopes {
    /// The tables built or reused so far: at most one per grammar and reading.
    tables: Vec<Arc<ScopeTable>>,
    /// The file's Python module path, which qualifies its definitions and relative imports.
    module: Option<Arc<ModulePath>>,
    /// The host's path of the file (`FileScopeSlot::path`), which places a Rust file in its crate.
    path: Option<Arc<str>>,
    /// A build that stopped at its node budget stops again at any budget up to this one.
    failed_within: usize,
    charged: Vec<usize>,
    /// The clauses that paid for a table's usage index, with the table (by address).
    usage_charged: Vec<(usize, usize)>,
    /// Whether the file has syntax errors: its bindings are then not all known, so no name is
    /// read as an unshadowed builtin for a value.
    has_errors: bool,
    /// Whether the file is a Python stub (`Reading::stub`).
    stub: bool,
    /// The ECMA file's module system for queries that leave `sourceType` to it
    /// (`SourceType::of_file`); `None` until its syntax decides it (`decide_file_source_type`).
    source_type: Option<SourceType>,
    /// The ECMA file's module system by its extension and package type alone, for queries that
    /// read it (`moduleSystem: "package-type"`, `SourceType::by_package_type`).
    package_type_source_type: SourceType,
    /// The ECMA file's nearest `package.json`, which decides its JSX runtime where its text does
    /// not (`decide_file_jsx_runtime`).
    package: Option<Arc<Manifest>>,
    /// The file's JSX runtime, once a `file.jsxRuntime` read it.
    jsx_runtime: Option<JsxRuntime>,
    /// Values decoded so far, by node, grammar and reading (`value`, `compare` over values).
    values: HashMap<ValueKey, Rc<Value>>,
    /// Structures read so far, by node and equality (`compare`'s structural modes).
    structures: HashMap<(usize, Equality), Rc<Option<Vec<Tok>>>>,
    /// Literals parsed so far, by node and grammar (`parse`); `None` is a literal whose value is
    /// not known.
    parses: HashMap<(usize, Grammar), Option<Arc<SubTree>>>,
    /// Strings parsed so far, by node, reading and grammar (`formatArgs` over a PHP format that
    /// is no literal); `None` is a parse whose value is not known.
    value_parses: HashMap<(ValueKey, Grammar), Option<Arc<SubTree>>>,
    /// The file's lines, per linter's terminators (`layout`, `suppress`), built on first use.
    lines: [Option<Rc<LineTable>>; 2],
    /// The file's ESLint directive comments (`suppress.eslint`), read on first use.
    eslint_directives: Option<Rc<EslintDirectives>>,
    /// The file's eslint-plugin-vue template comment directives (`suppress.eslint` in a Vue
    /// file), read on first use.
    vue_directives: Option<Rc<VueDirectives>>,
    /// The file's oxlint directive comments (`suppress.oxlint`), read on first use.
    oxlint_directives: Option<Rc<OxlintDirectives>>,
    /// The file's Biome suppression comments (`suppress.biome`), read on first use.
    biome_suppressions: Option<Rc<BiomeSuppressions>>,
    /// The file's `noqa` comments and line mapping (`suppress.noqa`), read on first use.
    noqa: Option<Rc<NoqaIndex>>,
    /// The file's comments, read as Bandit's `# nosec` (`suppress.nosec`), read on first use.
    nosec: Option<Rc<NosecIndex>>,
    /// The file's PHPStan ignore comments (`suppress.phpstan`), read on first use.
    phpstan: Option<Rc<PhpstanIndex>>,
    /// The file's Psalm `@psalm-suppress` tags (`suppress.psalm`), read on first use.
    psalm: Option<Rc<PsalmIndex>>,
    /// The file's PHP_CodeSniffer annotations (`suppress.phpcs`), read on first use.
    phpcs: Option<Rc<PhpcsIndex>>,
    /// The file's Rust lint-level attributes (`suppress.clippy`), read on first use.
    clippy: Option<Rc<ClippyIndex>>,
    /// The file's SQLFluff `-- noqa` directives (`suppress.sqlfluff`), read on first use.
    sqlfluff: Option<Rc<SqlfluffIndex>>,
    /// The file's `rerail-ignore` directives, read on first use.
    rerail_ignores: Option<Rc<RerailIgnores>>,
    /// The file's PHP name references (`resolves` in php, and the flow a PHP table reads), built
    /// on first use or taken from a cached table's flow; a build that stopped at its node budget
    /// stops again at any budget up to `php_names_failed_within`.
    php_names: Option<Arc<PhpNames>>,
    php_names_failed_within: usize,
    /// The clauses that paid for `php_names`.
    php_names_charged: Vec<usize>,
}

/// Values decoded so far, by node and the `Reading` instance that determines their semantics.
/// `Reading` assigns its own monotonic id so independently allocated option Arcs cannot collide or
/// be confused after an allocator reuses an address.
pub(super) type ValueKey = (usize, usize);

/// Holds the file's scope table while its roots are matched: built by the first `resolves` that
/// needs it, shared by every clause on the file, and handed back for caching by `table`.
pub(crate) struct FileScopeSlot {
    previous: Option<FileScopes>,
}

impl FileScopeSlot {
    pub(crate) fn install(
        cached: Vec<Arc<ScopeTable>>,
        module: Option<Arc<ModulePath>>,
        has_errors: bool,
    ) -> Self {
        let previous = FILE_SCOPES.replace(Some(FileScopes {
            tables: cached,
            module,
            failed_within: 0,
            path: None,
            charged: Vec::new(),
            usage_charged: Vec::new(),
            has_errors,
            stub: false,
            source_type: Some(SourceType::Module),
            package_type_source_type: SourceType::Module,
            package: None,
            jsx_runtime: None,
            values: HashMap::new(),
            structures: HashMap::new(),
            parses: HashMap::new(),
            value_parses: HashMap::new(),
            lines: [None, None],
            eslint_directives: None,
            vue_directives: None,
            oxlint_directives: None,
            biome_suppressions: None,
            noqa: None,
            nosec: None,
            phpstan: None,
            psalm: None,
            phpcs: None,
            clippy: None,
            sqlfluff: None,
            rerail_ignores: None,
            php_names: None,
            php_names_failed_within: 0,
            php_names_charged: Vec::new(),
        }));
        Self { previous }
    }

    /// Reads `file` at the host's path (the one `file` conditions read) as a Python stub where it
    /// is one (`is_stub_file`), and as the module system its path and package type give it.
    pub(crate) fn file(self, file: &FileInput) -> Self {
        let stub = is_stub_file(&file.path);
        let source_type = SourceType::of_file(&file.language, &file.path, file.package_type());
        let package_type_source_type = SourceType::by_package_type(&file.path, file.package_type());
        FILE_SCOPES.with_borrow_mut(|slot| {
            if let Some(slot) = slot {
                slot.stub = stub;
                slot.path = Some(Arc::from(file.path.as_str()));
                slot.source_type = source_type;
                slot.package_type_source_type = package_type_source_type;
                slot.package.clone_from(&file.package);
            }
        });
        self
    }

    /// The tables built or reused so far.
    pub(crate) fn tables(&self) -> Vec<Arc<ScopeTable>> {
        FILE_SCOPES.with_borrow(|slot| {
            slot.as_ref()
                .map(|slot| slot.tables.clone())
                .unwrap_or_default()
        })
    }
}

impl Drop for FileScopeSlot {
    fn drop(&mut self) {
        FILE_SCOPES.set(self.previous.take());
    }
}

/// The module path of the file being matched, if the host or its name gives one.
pub(crate) fn file_module() -> Option<Arc<ModulePath>> {
    FILE_SCOPES.with_borrow(|slot| slot.as_ref().and_then(|slot| slot.module.clone()))
}

/// The host's path of the file being matched, if it gave one.
fn file_path() -> Option<Arc<str>> {
    FILE_SCOPES.with_borrow(|slot| slot.as_ref().and_then(|slot| slot.path.clone()))
}

/// The host's path of the file being matched (workspace-relative), if it gave one: where
/// `importTarget` resolves from.
pub(crate) fn matched_file_path() -> Option<Arc<str>> {
    file_path()
}

/// The module system of the file being matched, for queries that leave `sourceType` to it: its
/// path's and package's, else Node's reading of `root`'s syntax (`has_module_syntax`), read once.
/// ESLint's `module` outside a file evaluation.
pub(super) fn decide_file_source_type<D: Doc>(root: &Node<'_, D>) -> SourceType {
    if let Some(source_type) = FILE_SCOPES.with_borrow(|slot| match slot {
        Some(slot) => slot.source_type,
        None => Some(SourceType::Module),
    }) {
        return source_type;
    }
    let source_type = syntax_source_type(root);
    FILE_SCOPES.with_borrow_mut(|slot| {
        if let Some(slot) = slot {
            slot.source_type = Some(source_type);
        }
    });
    source_type
}

/// The module system Node gives a file its path and package leave to its syntax.
fn syntax_source_type<D: Doc>(root: &Node<'_, D>) -> SourceType {
    #[cfg(test)]
    SOURCE_TYPE_DERIVATIONS.set(SOURCE_TYPE_DERIVATIONS.get() + 1);
    if has_module_syntax(root) {
        SourceType::Module
    } else {
        SourceType::CommonJs
    }
}

/// The JSX runtime of a file whose tree is `root` and whose nearest manifest is `package`: the
/// one its text names (`jsx_runtime_in_source`), else its manifest's; `Unknown` without either.
fn jsx_runtime_of<D: Doc>(root: &Node<'_, D>, package: Option<&Manifest>) -> JsxRuntime {
    #[cfg(test)]
    JSX_RUNTIME_DERIVATIONS.set(JSX_RUNTIME_DERIVATIONS.get() + 1);
    jsx_runtime_in_source(root)
        .unwrap_or_else(|| package.map_or(JsxRuntime::Unknown, Manifest::jsx_runtime))
}

/// The facts of `file`, whose tree is `root`, that queries' file gates read (its module system,
/// its JSX runtime), each derived at most once, when a gate first reads it. Needs no file
/// evaluation.
pub(crate) struct FileGates<'a, 'r, D: Doc> {
    file: &'a FileInput,
    root: Node<'r, D>,
    source_type: OnceCell<SourceType>,
    jsx_runtime: OnceCell<JsxRuntime>,
}

impl<'a, 'r, D: Doc> FileGates<'a, 'r, D> {
    pub(crate) fn new(file: &'a FileInput, root: Node<'r, D>) -> Self {
        Self {
            file,
            root,
            source_type: OnceCell::new(),
            jsx_runtime: OnceCell::new(),
        }
    }

    /// Whether `matcher`'s file gates (its `when.sourceType`, its `file.jsxRuntime`) admit the
    /// file, as `run_clause` reads them. A file they exclude matches nothing, so its syntax errors
    /// cannot leave the clause's result incomplete.
    pub(crate) fn admit(&self, matcher: &GrammarMatcher) -> bool {
        let file = self.file;
        let source_type = matcher.source_types.as_ref().map(|_| {
            if matcher.package_type_gate {
                return SourceType::by_package_type(&file.path, file.package_type());
            }
            *self.source_type.get_or_init(|| {
                SourceType::of_file(&file.language, &file.path, file.package_type())
                    .unwrap_or_else(|| syntax_source_type(&self.root))
            })
        });
        matcher.may_apply_to(source_type)
            && matcher
                .file
                .as_ref()
                .and_then(|condition| condition.jsx_runtime.as_ref())
                .is_none_or(|runtimes| {
                    runtimes.contains(
                        self.jsx_runtime
                            .get_or_init(|| jsx_runtime_of(&self.root, file.package.as_deref())),
                    )
                })
    }
}

/// The module system the file being matched has by its extension and package type
/// (`SourceType::by_package_type`); ESLint's `module` outside a file evaluation.
pub(super) fn file_package_type_source_type() -> SourceType {
    FILE_SCOPES.with_borrow(|slot| {
        slot.as_ref()
            .map_or(SourceType::Module, |slot| slot.package_type_source_type)
    })
}

/// The module system the file being matched has for a query with `options` that leaves
/// `sourceType` to it: the one its extension and package type give it where the query reads that
/// (`moduleSystem: "package-type"`), else the one `decide_file_source_type` gave it. ESLint's
/// `module` outside a file evaluation.
pub(crate) fn file_source_type_for(options: &LanguageOptions) -> SourceType {
    FILE_SCOPES.with_borrow(|slot| {
        slot.as_ref()
            .and_then(|slot| {
                if options.package_type_module_system {
                    Some(slot.package_type_source_type)
                } else {
                    slot.source_type
                }
            })
            .unwrap_or(SourceType::Module)
    })
}

/// The JSX runtime of the file being matched (`file.jsxRuntime`): the one its text names
/// (`jsx_runtime_in_source`), else its manifest's, read once. Without a manifest, or outside a
/// file evaluation, a text that names none is `Unknown`.
pub(super) fn decide_file_jsx_runtime<D: Doc>(root: &Node<'_, D>) -> JsxRuntime {
    if let Some(runtime) =
        FILE_SCOPES.with_borrow(|slot| slot.as_ref().and_then(|slot| slot.jsx_runtime))
    {
        return runtime;
    }
    let package =
        FILE_SCOPES.with_borrow(|slot| slot.as_ref().and_then(|slot| slot.package.clone()));
    let runtime = jsx_runtime_of(root, package.as_deref());
    FILE_SCOPES.with_borrow_mut(|slot| {
        if let Some(slot) = slot {
            slot.jsx_runtime = Some(runtime);
        }
    });
    runtime
}

/// A value decoded earlier in this file.
pub(super) fn memoized_value(key: &ValueKey) -> Option<Rc<Value>> {
    FILE_SCOPES.with_borrow(|slot| slot.as_ref().and_then(|slot| slot.values.get(key).cloned()))
}

pub(super) fn memoize_value(key: ValueKey, value: &Rc<Value>) {
    FILE_SCOPES.with_borrow_mut(|slot| {
        if let Some(slot) = slot {
            slot.values.insert(key, Rc::clone(value));
        }
    });
}

/// A structure read earlier in this file.
pub(super) fn memoized_structure(key: &(usize, Equality)) -> Option<Rc<Option<Vec<Tok>>>> {
    FILE_SCOPES.with_borrow(|slot| {
        slot.as_ref()
            .and_then(|slot| slot.structures.get(key).cloned())
    })
}

pub(super) fn memoize_structure(key: (usize, Equality), structure: &Rc<Option<Vec<Tok>>>) {
    FILE_SCOPES.with_borrow_mut(|slot| {
        if let Some(slot) = slot {
            slot.structures.insert(key, Rc::clone(structure));
        }
    });
}

/// The lines of the file being matched, read from `source` (its whole text) with `terminators`:
/// built once per file while a file is being matched, and fresh outside one.
pub(crate) fn file_lines(source: &str, terminators: Terminators) -> Rc<LineTable> {
    let slot = terminators.index();
    if let Some(lines) = FILE_SCOPES.with_borrow(|scopes| {
        scopes
            .as_ref()
            .and_then(|scopes| scopes.lines[slot].clone())
    }) {
        return lines;
    }
    let lines = Rc::new(LineTable::new(source, terminators));
    FILE_SCOPES.with_borrow_mut(|scopes| {
        if let Some(scopes) = scopes {
            scopes.lines[slot] = Some(Rc::clone(&lines));
        }
    });
    lines
}

/// The ESLint directives of the file being matched: read once per file while a file is being
/// matched, and fresh outside one.
pub(crate) fn file_eslint_directives(
    read: impl FnOnce() -> EslintDirectives,
) -> Rc<EslintDirectives> {
    if let Some(directives) = FILE_SCOPES.with_borrow(|scopes| {
        scopes
            .as_ref()
            .and_then(|scopes| scopes.eslint_directives.clone())
    }) {
        return directives;
    }
    let directives = Rc::new(read());
    FILE_SCOPES.with_borrow_mut(|scopes| {
        if let Some(scopes) = scopes {
            scopes.eslint_directives = Some(Rc::clone(&directives));
        }
    });
    directives
}

/// The eslint-plugin-vue template comment directives of the file being matched: read once per
/// file while a file is being matched, and fresh outside one.
pub(crate) fn file_vue_directives(read: impl FnOnce() -> VueDirectives) -> Rc<VueDirectives> {
    if let Some(directives) = FILE_SCOPES.with_borrow(|scopes| {
        scopes
            .as_ref()
            .and_then(|scopes| scopes.vue_directives.clone())
    }) {
        return directives;
    }
    let directives = Rc::new(read());
    FILE_SCOPES.with_borrow_mut(|scopes| {
        if let Some(scopes) = scopes {
            scopes.vue_directives = Some(Rc::clone(&directives));
        }
    });
    directives
}

/// The oxlint directives of the file being matched: read once per file while a file is being
/// matched, and fresh outside one.
pub(crate) fn file_oxlint_directives(
    read: impl FnOnce() -> OxlintDirectives,
) -> Rc<OxlintDirectives> {
    if let Some(directives) = FILE_SCOPES.with_borrow(|scopes| {
        scopes
            .as_ref()
            .and_then(|scopes| scopes.oxlint_directives.clone())
    }) {
        return directives;
    }
    let directives = Rc::new(read());
    FILE_SCOPES.with_borrow_mut(|scopes| {
        if let Some(scopes) = scopes {
            scopes.oxlint_directives = Some(Rc::clone(&directives));
        }
    });
    directives
}

/// The Biome suppressions of the file being matched: read once per file while a file is being
/// matched, and fresh outside one.
pub(crate) fn file_biome_suppressions(
    read: impl FnOnce() -> BiomeSuppressions,
) -> Rc<BiomeSuppressions> {
    if let Some(suppressions) = FILE_SCOPES.with_borrow(|scopes| {
        scopes
            .as_ref()
            .and_then(|scopes| scopes.biome_suppressions.clone())
    }) {
        return suppressions;
    }
    let suppressions = Rc::new(read());
    FILE_SCOPES.with_borrow_mut(|scopes| {
        if let Some(scopes) = scopes {
            scopes.biome_suppressions = Some(Rc::clone(&suppressions));
        }
    });
    suppressions
}

/// The `rerail-ignore` directives of the file being matched: read once per file while a file is
/// being matched, and fresh outside one.
pub(crate) fn file_rerail_ignores(read: impl FnOnce() -> RerailIgnores) -> Rc<RerailIgnores> {
    if let Some(ignores) = FILE_SCOPES.with_borrow(|scopes| {
        scopes
            .as_ref()
            .and_then(|scopes| scopes.rerail_ignores.clone())
    }) {
        return ignores;
    }
    let ignores = Rc::new(read());
    FILE_SCOPES.with_borrow_mut(|scopes| {
        if let Some(scopes) = scopes {
            scopes.rerail_ignores = Some(Rc::clone(&ignores));
        }
    });
    ignores
}

/// The `noqa` index of the file being matched: read once per file while a file is being matched,
/// and fresh outside one.
pub(crate) fn file_noqa(read: impl FnOnce() -> NoqaIndex) -> Rc<NoqaIndex> {
    if let Some(index) =
        FILE_SCOPES.with_borrow(|scopes| scopes.as_ref().and_then(|scopes| scopes.noqa.clone()))
    {
        return index;
    }
    let index = Rc::new(read());
    FILE_SCOPES.with_borrow_mut(|scopes| {
        if let Some(scopes) = scopes {
            scopes.noqa = Some(Rc::clone(&index));
        }
    });
    index
}

/// The `nosec` index of the file being matched: read once per file while a file is being matched,
/// and fresh outside one.
pub(crate) fn file_nosec(read: impl FnOnce() -> NosecIndex) -> Rc<NosecIndex> {
    if let Some(index) =
        FILE_SCOPES.with_borrow(|scopes| scopes.as_ref().and_then(|scopes| scopes.nosec.clone()))
    {
        return index;
    }
    let index = Rc::new(read());
    FILE_SCOPES.with_borrow_mut(|scopes| {
        if let Some(scopes) = scopes {
            scopes.nosec = Some(Rc::clone(&index));
        }
    });
    index
}

/// The PHPStan ignore index of the file being matched: read once per file while a file is being
/// matched, and fresh outside one.
pub(crate) fn file_phpstan(read: impl FnOnce() -> PhpstanIndex) -> Rc<PhpstanIndex> {
    if let Some(index) =
        FILE_SCOPES.with_borrow(|scopes| scopes.as_ref().and_then(|scopes| scopes.phpstan.clone()))
    {
        return index;
    }
    let index = Rc::new(read());
    FILE_SCOPES.with_borrow_mut(|scopes| {
        if let Some(scopes) = scopes {
            scopes.phpstan = Some(Rc::clone(&index));
        }
    });
    index
}

/// The Psalm suppression index of the file being matched: read once per file while a file is
/// being matched, and fresh outside one.
pub(crate) fn file_psalm(read: impl FnOnce() -> PsalmIndex) -> Rc<PsalmIndex> {
    if let Some(index) =
        FILE_SCOPES.with_borrow(|scopes| scopes.as_ref().and_then(|scopes| scopes.psalm.clone()))
    {
        return index;
    }
    let index = Rc::new(read());
    FILE_SCOPES.with_borrow_mut(|scopes| {
        if let Some(scopes) = scopes {
            scopes.psalm = Some(Rc::clone(&index));
        }
    });
    index
}

/// The PHP_CodeSniffer annotation index of the file being matched: read once per file while a
/// file is being matched, and fresh outside one.
pub(crate) fn file_phpcs(read: impl FnOnce() -> PhpcsIndex) -> Rc<PhpcsIndex> {
    if let Some(index) =
        FILE_SCOPES.with_borrow(|scopes| scopes.as_ref().and_then(|scopes| scopes.phpcs.clone()))
    {
        return index;
    }
    let index = Rc::new(read());
    FILE_SCOPES.with_borrow_mut(|scopes| {
        if let Some(scopes) = scopes {
            scopes.phpcs = Some(Rc::clone(&index));
        }
    });
    index
}

/// The Rust lint-level attribute index of the file being matched: read once per file while a
/// file is being matched, and fresh outside one.
pub(crate) fn file_clippy(read: impl FnOnce() -> ClippyIndex) -> Rc<ClippyIndex> {
    if let Some(index) =
        FILE_SCOPES.with_borrow(|scopes| scopes.as_ref().and_then(|scopes| scopes.clippy.clone()))
    {
        return index;
    }
    let index = Rc::new(read());
    FILE_SCOPES.with_borrow_mut(|scopes| {
        if let Some(scopes) = scopes {
            scopes.clippy = Some(Rc::clone(&index));
        }
    });
    index
}

/// The SQLFluff `noqa` index of the file being matched: read once per file while a file is being
/// matched, and fresh outside one.
pub(crate) fn file_sqlfluff(read: impl FnOnce() -> SqlfluffIndex) -> Rc<SqlfluffIndex> {
    if let Some(index) =
        FILE_SCOPES.with_borrow(|scopes| scopes.as_ref().and_then(|scopes| scopes.sqlfluff.clone()))
    {
        return index;
    }
    let index = Rc::new(read());
    FILE_SCOPES.with_borrow_mut(|scopes| {
        if let Some(scopes) = scopes {
            scopes.sqlfluff = Some(Rc::clone(&index));
        }
    });
    index
}

/// A literal parsed earlier in this file: `Some(None)` when its value is not known.
pub(super) fn memoized_parse(key: &(usize, Grammar)) -> Option<Option<Arc<SubTree>>> {
    FILE_SCOPES.with_borrow(|slot| slot.as_ref().and_then(|slot| slot.parses.get(key).cloned()))
}

pub(super) fn memoize_parse(key: (usize, Grammar), tree: Option<Arc<SubTree>>) {
    FILE_SCOPES.with_borrow_mut(|slot| {
        if let Some(slot) = slot {
            slot.parses.insert(key, tree);
        }
    });
}

/// A string value parsed earlier in this file: `Some(None)` when it is not known.
pub(super) fn memoized_value_parse(key: &(ValueKey, Grammar)) -> Option<Option<Arc<SubTree>>> {
    FILE_SCOPES.with_borrow(|slot| {
        slot.as_ref()
            .and_then(|slot| slot.value_parses.get(key).cloned())
    })
}

pub(super) fn memoize_value_parse(key: (ValueKey, Grammar), tree: Option<Arc<SubTree>>) {
    FILE_SCOPES.with_borrow_mut(|slot| {
        if let Some(slot) = slot {
            slot.value_parses.insert(key, tree);
        }
    });
}

/// Whether `name` at `range` is a reference to the unshadowed builtin or global of that name, for
/// a value: `None` outside a file evaluation, in a file with syntax errors (a binding may hide in
/// an error region), or when the table cannot be built (`UTIL_EXCEEDED` is then set). With
/// language options the name must also be a declared global, a builtin of the version or a
/// variable of a listed lib in either namespace (eslint-utils' `findVariable` reads the name only).
pub(super) fn unshadowed_global<D: Doc>(
    node: &Node<'_, D>,
    range: Range<usize>,
    name: &str,
    language: GrammarDialect,
    options: Option<&LanguageOptions>,
    python: Option<&PythonOptions>,
) -> Option<bool> {
    let has_errors = FILE_SCOPES.with_borrow(|slot| slot.as_ref().map(|slot| slot.has_errors))?;
    if has_errors {
        return None;
    }
    if python.is_some_and(|python| !python.declares(name))
        || options.is_some_and(|options| {
            !options.declares_in(name, VALUE | TYPE, None, file_source_type_for(options))
        })
    {
        return Some(false);
    }
    let table = file_table(node, language, table_reading(options, python))?;
    let Some(reference) = table.reference(&range) else {
        return Some(false);
    };
    Some(
        table
            .resolve(reference, name)
            .is_none_or(|binding| !binding.shadows()),
    )
}

/// The value node the name at `range` is bound to (`value.follow`): `None` outside a file
/// evaluation, in a file with syntax errors, for a builtin or global, or for a binding that carries
/// no value; also when the table cannot be built (`UTIL_EXCEEDED` is then set).
pub(super) fn binding_value<D: Doc>(
    node: &Node<'_, D>,
    range: Range<usize>,
    name: &str,
    language: GrammarDialect,
    options: Option<&LanguageOptions>,
    python: Option<&PythonOptions>,
) -> Option<Followed> {
    let has_errors = FILE_SCOPES.with_borrow(|slot| slot.as_ref().map(|slot| slot.has_errors))?;
    if has_errors {
        return None;
    }
    let table = file_table(node, language, table_reading(options, python))?;
    let reference = table.reference(&range)?;
    let (binding, value) = table.binding(reference, name)?;
    Some(binding.followed(value?))
}

/// The table of `node`'s file a value follows names through (`value.follow`): `None` outside a
/// file evaluation, in a file with syntax errors (a binding may hide in an error region), or when
/// the table cannot be built (`UTIL_EXCEEDED` is then set).
pub(super) fn followed_table<D: Doc>(
    node: &Node<'_, D>,
    language: GrammarDialect,
    options: Option<&LanguageOptions>,
    python: Option<&PythonOptions>,
) -> Option<Arc<ScopeTable>> {
    let has_errors = FILE_SCOPES.with_borrow(|slot| slot.as_ref().map(|slot| slot.has_errors))?;
    if has_errors {
        return None;
    }
    file_table(node, language, table_reading(options, python))
}

/// The value node of the PHP constant `target` fetches in `node`'s file (`value.follow`), where
/// the file declares it once (`PhpNames::constant_value`; a Blade template's embedded program's):
/// `None` outside a file evaluation, in a file with syntax errors (an error region may hide
/// another declaration), or when the names cannot be read (`UTIL_EXCEEDED` is then set).
pub(super) fn constant_value<D: Doc>(
    node: &Node<'_, D>,
    target: NodeRef,
    language: GrammarDialect,
) -> Option<NodeRef> {
    let has_errors = FILE_SCOPES.with_borrow(|slot| slot.as_ref().map(|slot| slot.has_errors))?;
    if has_errors {
        return None;
    }
    let names = file_php_names(node, language)?;
    names.constant_value(&target.find_near(node.root().root(), node)?)
}

/// How a table reads the file under these options: Python annotations after the module from 3.14,
/// TypeScript blocks as scopes from ES2015.
pub(crate) fn table_reading(
    options: Option<&LanguageOptions>,
    python: Option<&PythonOptions>,
) -> Reading {
    Reading {
        deferred_annotations: python.is_some_and(|python| python.defers_annotations()),
        block_scopes: options.is_none_or(|options| options.block_scopes()),
        stub: false,
        python_minor: python.map_or(DEFAULT_PYTHON_MINOR, |python| python.minor),
        typing: python.and_then(|python| python.typing),
    }
}

/// The usage index of `table`, `node`'s file's, built on first use and charged to the current
/// clause's `maxVisitedNodes` once per file. `None` means the clause failed for the file
/// (`UTIL_EXCEEDED` is set): a missing index must not read as "unused".
pub(super) fn file_usage<D: Doc>(
    node: &Node<'_, D>,
    table: &Arc<ScopeTable>,
) -> Option<Arc<Usage>> {
    if UTIL_EXCEEDED.get().is_some() {
        return None;
    }
    let steps = UTIL_STEPS_LEFT.get();
    let (usage, built) = match table.usage.get() {
        Some(usage) => (Arc::clone(usage), false),
        None => match Usage::build(
            table,
            &node.root().root(),
            steps,
            &budget::matcher_budget_exhausted,
        ) {
            Ok(usage) => (
                Arc::clone(table.usage.get_or_init(|| Arc::new(usage))),
                true,
            ),
            Err(_) => {
                return exceed();
            }
        },
    };
    let key = (CURRENT_CLAUSE.get(), Arc::as_ptr(table) as usize);
    let charged = FILE_SCOPES.with_borrow_mut(|slot| {
        let Some(slot) = slot else {
            return false;
        };
        // Charged to each clause once: what this one spends depends on whether it was.
        super::read_clause_state();
        if slot.usage_charged.contains(&key) {
            return true;
        }
        slot.usage_charged.push(key);
        false
    });
    if !charged || built {
        let Some(left) = steps.checked_sub(usage.cost) else {
            return exceed();
        };
        UTIL_STEPS_LEFT.set(left);
        super::filled(usage.cost);
    }
    Some(usage)
}

/// Fails the root's clause for the file: a missing table must not read as "not global".
fn exceed<T>() -> Option<T> {
    UTIL_EXCEEDED.set(Some(UtilLimit::Scope));
    None
}

/// The PHP name references of `node`'s file (a Blade template's: its embedded program's), built on
/// first use, or taken from a cached table's flow, and charged to the current clause's
/// `maxVisitedNodes` once per file. A PHP table's flow reads these (`file_table`), so a clause that
/// reads names alone pays for them alone. `None` means the clause failed for the file
/// (`UTIL_EXCEEDED` is set): a missing table must not read as "not listed".
fn file_php_names<D: Doc>(node: &Node<'_, D>, language: GrammarDialect) -> Option<Arc<PhpNames>> {
    if UTIL_EXCEEDED.get().is_some() {
        return None;
    }
    let root = node.root().root();
    let build = |cap| PhpNames::build(root.clone(), cap, &budget::matcher_budget_exhausted);
    let clause = CURRENT_CLAUSE.get();
    let steps = UTIL_STEPS_LEFT.get();
    let installed = FILE_SCOPES.with_borrow_mut(|slot| {
        slot.as_mut().map(|slot| {
            // A table cached from an earlier evaluation holds the names its flow read.
            if slot.php_names.is_none() {
                slot.php_names = slot
                    .tables
                    .iter()
                    .filter(|table| table.language == language)
                    .find_map(|table| table.php.as_deref())
                    .map(|flow| Arc::clone(&flow.names));
            }
            (
                slot.php_names.clone(),
                slot.php_names_failed_within,
                slot.php_names_charged.contains(&clause),
            )
        })
    });
    if installed.is_some() {
        // Charged to each clause once (`php_names_charged`).
        super::read_clause_state();
    }
    let (names, charged) = match installed {
        Some((Some(names), _, charged)) => (names, charged),
        Some((None, failed_within, _)) => {
            if steps <= failed_within {
                return exceed();
            }
            match build(steps) {
                Ok(names) => {
                    let names = Arc::new(names);
                    FILE_SCOPES.with_borrow_mut(|slot| {
                        if let Some(slot) = slot {
                            slot.php_names = Some(Arc::clone(&names));
                        }
                    });
                    (names, false)
                }
                Err(error) => {
                    // A deadline is the clause's own allowance running out: a later clause, with
                    // an allowance of its own, builds the names again.
                    let within = match error {
                        BuildError::Budget => Some(steps),
                        BuildError::Depth => Some(usize::MAX),
                        BuildError::Deadline => None,
                    };
                    FILE_SCOPES.with_borrow_mut(|slot| {
                        if let (Some(slot), Some(within)) = (slot, within) {
                            slot.php_names_failed_within = slot.php_names_failed_within.max(within);
                        }
                    });
                    return exceed();
                }
            }
        }
        // Outside a file evaluation: build for this call alone, and charge it every time.
        None => match build(steps) {
            Ok(names) => (Arc::new(names), false),
            Err(_) => return exceed(),
        },
    };
    if !charged {
        let Some(left) = steps.checked_sub(names.cost) else {
            return exceed();
        };
        UTIL_STEPS_LEFT.set(left);
        FILE_SCOPES.with_borrow_mut(|slot| {
            if let Some(slot) = slot {
                slot.php_names_charged.push(clause);
            }
        });
    }
    Some(names)
}

fn build<D: Doc>(
    node: &Node<'_, D>,
    language: GrammarDialect,
    cap: usize,
    reading: Reading,
    php_names: Option<Arc<PhpNames>>,
) -> Result<ScopeTable, BuildError> {
    #[cfg(test)]
    TABLE_BUILDS.set(TABLE_BUILDS.get() + 1);
    ScopeTable::build_with_names(
        node.root().root(),
        language,
        cap,
        &budget::matcher_budget_exhausted,
        reading,
        file_module().as_deref(),
        php_names,
    )
}

/// What `calls` reads at `node`'s file in `language` (`calls::FileTables`, `CallRules::tables`):
/// its table under the default reading (as `resolves` without options reads it), the file's
/// module path, and the PHP names a table's flow reads (Blade's: its embedded program's). `None`
/// where one could not be built.
pub(crate) fn calls_tables<D: Doc>(
    node: &Node<'_, D>,
    language: GrammarDialect,
) -> Option<crate::code_rule_dsl::calls::FileTables> {
    use crate::code_rule_dsl::language::adapter::CallRules;
    with_adapter!(Family::of(language), A => <<A as Adapter>::Calls as CallRules>::tables(
        || file_table(node, language, Reading::default()),
        file_module,
        || file_php_names(node, language),
    ))
}

/// The table of `node`'s file under `reading`, charging its build to the current clause's
/// `maxVisitedNodes` once per file; a PHP or Blade table's flow reads the file's names, which
/// `file_php_names` builds and charges apart. `None` means the clause failed for the file
/// (`UTIL_EXCEEDED` is set).
pub(super) fn file_table<D: Doc>(
    node: &Node<'_, D>,
    language: GrammarDialect,
    reading: Reading,
) -> Option<Arc<ScopeTable>> {
    if UTIL_EXCEEDED.get().is_some() {
        return None;
    }
    let (stubs, flow) = with_adapter!(Family::of(language), A => (
        <<A as Adapter>::Scope as ScopeRules>::STUBS,
        <<A as Adapter>::Scope as ScopeRules>::FLOW,
    ));
    let stub = stubs && FILE_SCOPES.with_borrow(|slot| slot.as_ref().is_some_and(|slot| slot.stub));
    let reading = Reading {
        stub,
        python_minor: if stub { reading.python_minor } else { 0 },
        ..reading
    };
    // A parsed annotation's node reads the table of the file it came from.
    if crate::code_rule_dsl::literal::sub_tree(node).is_some() {
        let table = SUB_TABLES.with_borrow(|tables| {
            tables
                .iter()
                .rev()
                .find(|table| table.language == language && table.reading == reading)
                .cloned()
        });
        return table.or_else(exceed);
    }
    let php_names = if flow {
        Some(file_php_names(node, language)?)
    } else {
        None
    };
    let clause = CURRENT_CLAUSE.get();
    let installed = FILE_SCOPES.with_borrow(|slot| {
        slot.as_ref().map(|slot| {
            (
                slot.tables
                    .iter()
                    .find(|table| table.language == language && table.reading == reading)
                    .cloned(),
                slot.failed_within,
                slot.charged.contains(&clause),
            )
        })
    });
    if installed.is_some() {
        // Charged to each clause once (`charged`).
        super::read_clause_state();
    }
    let steps = UTIL_STEPS_LEFT.get();
    let (table, charged) = match installed {
        Some((Some(table), _, charged)) => (table, charged),
        Some((None, failed_within, _)) => {
            if steps <= failed_within {
                return exceed();
            }
            match build(node, language, steps, reading, php_names) {
                Ok(table) => {
                    let table = Arc::new(table);
                    FILE_SCOPES.with_borrow_mut(|slot| {
                        if let Some(slot) = slot {
                            slot.tables.push(Arc::clone(&table));
                        }
                    });
                    (table, false)
                }
                Err(error) => {
                    // A deadline is the clause's own allowance running out: a later clause, with
                    // an allowance of its own, builds the table again.
                    let within = match error {
                        BuildError::Budget => Some(steps),
                        BuildError::Depth => Some(usize::MAX),
                        BuildError::Deadline => None,
                    };
                    FILE_SCOPES.with_borrow_mut(|slot| {
                        if let (Some(slot), Some(within)) = (slot, within) {
                            slot.failed_within = slot.failed_within.max(within);
                        }
                    });
                    return exceed();
                }
            }
        }
        // Outside a file evaluation: build for this call alone, and charge it every time.
        None => match build(node, language, steps, reading, php_names) {
            Ok(table) => (Arc::new(table), false),
            Err(_) => return exceed(),
        },
    };
    if !charged {
        let Some(left) = steps.checked_sub(table.cost) else {
            return exceed();
        };
        UTIL_STEPS_LEFT.set(left);
        super::filled(table.cost);
        FILE_SCOPES.with_borrow_mut(|slot| {
            if let Some(slot) = slot {
                slot.charged.push(clause);
            }
        });
    }
    Some(table)
}
