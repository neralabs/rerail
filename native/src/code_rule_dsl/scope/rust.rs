//! Rust's scopes, bindings and references, as rustc resolves names lexically (the Rust
//! Reference, "Names"): items are visible in their whole block or module, `let` and pattern
//! bindings from where they are made, a nested `fn` sees no local of the function around it, and a
//! module sees nothing of its parent but macros. Types, values, macros, lifetimes and labels are
//! separate namespaces. Names in macro token trees and paths past their first segment are not read:
//! what they denote needs macro expansion or type information.

use super::{
    Binding, BuildError, Builder, Form, Import, NodeRef, Reference, ScopeKind, ScopeTable, Site,
    TYPE, VALUE,
};
use crate::code_rule_dsl::calls::rust::{LIBRARY_ROOTS, crate_module};
use ast_grep_core::{Doc, Node};

/// The macro namespace: `macro_rules!` definitions and imported macros.
pub(crate) const MACRO: u8 = 4;
/// Lifetime parameters (`'a`).
pub(crate) const LIFETIME: u8 = 8;
/// Loop and block labels (`'outer:`).
pub(crate) const LABEL: u8 = 16;

/// Builds the scopes, bindings and references of a Rust file (`ScopeRules::visit`); the caller
/// has pushed the file's module scope.
pub(crate) fn visit_source_file<D: Doc>(
    builder: &mut Builder<'_>,
    root: &Node<'_, D>,
) -> Result<(), BuildError> {
    let mut visitor = Visitor { builder };
    visitor.items(root);
    visitor.children(root)
}

/// Orders each name's bindings by offset (`ScopeRules::finish`).
pub(crate) fn finish<D: Doc>(table: &mut ScopeTable, _: &Node<'_, D>) {
    for scope in &mut table.scopes {
        for chain in scope.bindings.values_mut() {
            chain.sort_by_key(|binding| binding.offset);
        }
    }
}

/// The scope and index of the binding `name` resolves to at `reference`: in each scope from the
/// reference's outwards, the latest binding of the reference's namespace made before it. Past a
/// function only items stay visible; past a module, only macros (`macro_rules!` is textual).
pub(crate) fn resolve_index(
    table: &ScopeTable,
    reference: &Reference,
    name: &str,
) -> Option<(u32, usize)> {
    resolve_except(table, reference, name, None)
}

/// `resolve_index`, passing over the binding `except` (its scope and index): a `use` path's head
/// does not name the binding its own `use` makes (`use log::log;`).
fn resolve_except(
    table: &ScopeTable,
    reference: &Reference,
    name: &str,
    except: Option<(u32, usize)>,
) -> Option<(u32, usize)> {
    let mut current = Some(reference.scope);
    let mut items_only = false;
    let mut macros_only = false;
    // Whether the walk just left a function into its impl's or trait's (or item's) generic
    // parameters: an associated function reads them and `Self`, a nested `fn` does not (E0401).
    let mut associated = false;
    while let Some(index) = current {
        let scope = &table.scopes[index as usize];
        let generics = associated && scope.kind == ScopeKind::TypeParams;
        if let Some(chain) = scope.bindings.get(name) {
            let found = chain
                .iter()
                .enumerate()
                .filter(|(position, binding)| {
                    except != Some((index, *position))
                        && binding.spaces & reference.spaces != 0
                        && binding.offset <= reference.start
                        && (!items_only
                            || is_item(binding, scope.range.start)
                            || (generics
                                && matches!(
                                    binding.site.form,
                                    Form::TypeParameter | Form::Implicit
                                )))
                        && (!macros_only || binding.spaces & MACRO != 0)
                })
                .max_by_key(|(_, binding)| binding.offset)
                .map(|(position, _)| position);
            if let Some(position) = found {
                return Some((index, position));
            }
        }
        associated = false;
        match scope.kind {
            ScopeKind::Function => {
                items_only = true;
                associated = true;
            }
            ScopeKind::Module if scope.parent.is_some() => macros_only = true,
            _ => {}
        }
        current = scope.parent;
    }
    None
}

/// Whether a binding is an item, visible from a nested function: not a local (`let`, a parameter
/// or pattern, a generic parameter or `Self`), which rustc refuses there (E0434, E0401).
fn is_item(binding: &Binding, scope_start: usize) -> bool {
    match binding.site.form {
        Form::Definition | Form::Static | Form::Import | Form::TypeAlias => true,
        // A `const` item is bound at its scope's start, a `let` after its declaration.
        Form::Assignment => binding.offset == scope_start,
        _ => false,
    }
}

/// Whether a glob import (`use m::*`) of the reference's module or a block within it may bind a
/// name: the module it reads is another file's or crate's.
pub(crate) fn glob_visible(table: &ScopeTable, reference: &Reference) -> bool {
    let mut current = Some(reference.scope);
    while let Some(index) = current {
        let scope = &table.scopes[index as usize];
        if !scope.stars.is_empty() {
            return true;
        }
        if scope.kind == ScopeKind::Module {
            return false;
        }
        current = scope.parent;
    }
    false
}

/// What the first segment of a `use` path names, once `use_path` has followed it.
pub(crate) enum UseHead<'t> {
    /// `crate`, `self` or `super`.
    Keyword,
    /// An extern crate: the path is global (`::serde`) or its head an `extern crate` item's name,
    /// which the path's first segment now spells as the crate's.
    Extern,
    /// A binding of the file that imports nothing: a module, a type, an enum.
    Item(&'t Binding),
    /// No binding of the file: an extern crate's name, or a glob import's.
    Unbound,
}

/// What a `use` path leads to: its segments, the head followed through the `use` bindings it names
/// where the `use` stands (`use crate::b as a; use a::f;` imports `crate::b::f`), and what that
/// head then names.
pub(crate) struct UsePath<'t> {
    pub(crate) segments: Vec<String>,
    pub(crate) head: UseHead<'t>,
}

/// The longest chain of `use` bindings `use_path` follows.
const MAX_USE_HOPS: usize = 64;

/// The path the `use` or `extern crate` binding `name` (`at`, its scope and index) imports, its
/// head resolved in the type namespace from the scope the binding is made in, as rustc resolves a
/// `use` path's first segment: each head bound by another `use` is replaced with that one's path,
/// read from its own scope. `is_extern` tells an `extern crate` item's binding. `None` for a
/// binding that imports no path, and where the `use` bindings name each other in a cycle.
pub(crate) fn use_path<'t>(
    table: &'t ScopeTable,
    at: (u32, usize),
    name: &str,
    is_extern: impl Fn(&Binding) -> bool,
) -> Option<UsePath<'t>> {
    let mut current = at;
    let mut current_name = name.to_owned();
    let binding = table.scopes[at.0 as usize].bindings.get(name)?.get(at.1)?;
    let Some(Import::Qualified(path)) = &binding.import else {
        return None;
    };
    if let Some(global) = path.strip_prefix("::") {
        return Some(UsePath {
            segments: global.split("::").map(str::to_owned).collect(),
            head: UseHead::Extern,
        });
    }
    let mut segments: Vec<String> = path.split("::").map(str::to_owned).collect();
    if is_extern(binding) {
        return Some(UsePath {
            segments,
            head: UseHead::Extern,
        });
    }
    let mut followed = vec![(at, current_name.clone())];
    loop {
        let first = segments.first()?.clone();
        if matches!(first.as_str(), "crate" | "self" | "super") {
            return Some(UsePath {
                segments,
                head: UseHead::Keyword,
            });
        }
        let scope = &table.scopes[current.0 as usize];
        let probe = Reference {
            start: scope.range.start,
            end: scope.range.start,
            scope: current.0,
            spaces: TYPE,
            at: scope.range.start,
            deferred: false,
            forward: false,
            type_scope: false,
            write: false,
            mutates: false,
            access: super::Access::Load,
        };
        let except = (first == current_name).then_some(current);
        let Some(found) = resolve_except(table, &probe, &first, except) else {
            return Some(UsePath {
                segments,
                head: UseHead::Unbound,
            });
        };
        let binding = &table.scopes[found.0 as usize].bindings[first.as_str()][found.1];
        let path = match &binding.import {
            Some(Import::Qualified(path)) if binding.site.form == Form::Import => path,
            _ => {
                return Some(UsePath {
                    segments,
                    head: UseHead::Item(binding),
                });
            }
        };
        let rest = segments.split_off(1);
        if let Some(global) = path.strip_prefix("::") {
            segments = global.split("::").map(str::to_owned).collect();
            segments.extend(rest);
            return Some(UsePath {
                segments,
                head: UseHead::Extern,
            });
        }
        segments = path.split("::").map(str::to_owned).collect();
        segments.extend(rest);
        if is_extern(binding) {
            return Some(UsePath {
                segments,
                head: UseHead::Extern,
            });
        }
        if followed.len() >= MAX_USE_HOPS
            || followed
                .iter()
                .any(|(seen, seen_name)| *seen == found && *seen_name == first)
        {
            return None;
        }
        followed.push((found, first.clone()));
        current = found;
        current_name = first;
    }
}

/// The path a read at `node` names (`resolves.import`), as rustc resolves its first segment: an
/// identifier or a path, its head bound by a `use` declaration or `extern crate` item replaced
/// with the path that imports it (that path's own head followed the same way, `use_path`), joined
/// with the rest by `::`. `crate::`, `self::` and `super::`
/// are normalised to `crate::` paths of the module the read sits in (the file's module, which
/// `file` gives under a crate's `src/`, and the inline `mod` blocks around the read); an unbound
/// head of a longer path is an extern crate's name (`std::process::exit`), as a path that starts
/// with `::` is. `None` where the read names no path so: a local or an item of the file, `Self`,
/// a qualified-type or generic head (`<T as Trait>::f`), a bare unbound name (the prelude's), a
/// head a glob import may bind, a path no read holds (a `use` path, a visibility), or `self::`
/// and `super::` where the module is not known. A `use` path that names neither is read as
/// written (an extern crate's, or a module of the crate the definitions would tell).
pub(crate) fn import_path<D: Doc>(
    table: &ScopeTable,
    node: &Node<'_, D>,
    file: Option<&str>,
) -> Option<String> {
    let mut rest = Vec::new();
    let mut head = node.clone();
    let absolute = loop {
        match head.kind().as_ref() {
            "scoped_identifier" | "scoped_type_identifier" => {
                rest.push(head.field("name")?.text().to_string());
                match head.field("path") {
                    Some(path) => head = path,
                    None => break true,
                }
            }
            "identifier" | "type_identifier" | "crate" | "self" | "super" => break false,
            _ => return None,
        }
    };
    rest.reverse();
    if node.ancestors().any(|ancestor| {
        matches!(
            ancestor.kind().as_ref(),
            "use_declaration" | "visibility_modifier" | "attribute_item" | "inner_attribute_item"
        )
    }) {
        return None;
    }
    let module = || {
        let module = file.map(crate_module)?.1;
        let mut inline: Vec<String> = node
            .ancestors()
            .filter(|ancestor| ancestor.kind() == "mod_item")
            .filter_map(|ancestor| ancestor.field("name").map(|name| name.text().to_string()))
            .collect();
        inline.reverse();
        Some([module, inline].concat())
    };
    let mut path = if absolute {
        Vec::new()
    } else {
        let name = head.text();
        match name.as_ref() {
            "crate" | "self" | "super" if rest.is_empty() => return None,
            "crate" | "self" | "super" => vec![name.to_string()],
            _ => {
                let reference = table.reference(&head.range())?;
                match resolve_index(table, reference, &name) {
                    Some(at) => {
                        let root = node.root().root();
                        let is_extern = |binding: &Binding| {
                            binding.site.node.is_some_and(|site| {
                                site.find(root.clone())
                                    .is_some_and(|site| site.kind() == "extern_crate_declaration")
                            })
                        };
                        use_path(table, at, &name, is_extern)?.segments
                    }
                    None if rest.is_empty() => return None,
                    None if glob_visible(table, reference)
                        && !LIBRARY_ROOTS.contains(&name.as_ref()) =>
                    {
                        return None;
                    }
                    None => vec![name.to_string()],
                }
            }
        }
    };
    path.extend(rest);
    normalise(&path, module)
}

/// `path` with a leading `self` or `super` (and any `super` after it) read against the module
/// `module` gives, as a `crate::` path; any other path as written.
fn normalise(path: &[String], module: impl FnOnce() -> Option<Vec<String>>) -> Option<String> {
    let (first, rest) = path.split_first()?;
    let mut joined = match first.as_str() {
        "self" | "super" => {
            let mut module = module()?;
            if first == "super" {
                module.pop()?;
            }
            let mut joined = vec![String::from("crate")];
            joined.extend(module);
            joined
        }
        _ => vec![first.clone()],
    };
    for segment in rest {
        if segment == "super" {
            if joined.first().is_some_and(|first| first != "crate") || joined.len() < 2 {
                return None;
            }
            joined.pop();
        } else {
            joined.push(segment.clone());
        }
    }
    Some(joined.join("::"))
}

/// The lexical names a pattern binds rather than reads: rustc reads an identifier pattern as a
/// path when it names a constant or a unit struct or variant (`None`).
const PRELUDE_PATTERN_PATHS: &[&str] = &["None"];

struct Visitor<'a, 'd> {
    builder: &'a mut Builder<'d>,
}

impl Visitor<'_, '_> {
    fn visit<D: Doc>(&mut self, node: &Node<'_, D>) -> Result<(), BuildError> {
        self.builder.enter()?;
        let result = self.dispatch(node);
        self.builder.leave();
        result
    }

    fn children<D: Doc>(&mut self, node: &Node<'_, D>) -> Result<(), BuildError> {
        for child in node.children().filter(|child| child.is_named()) {
            self.visit(&child)?;
        }
        Ok(())
    }

    fn field<D: Doc>(&mut self, node: &Node<'_, D>, name: &str) -> Result<(), BuildError> {
        match node.field(name) {
            Some(child) => self.visit(&child),
            None => Ok(()),
        }
    }

    fn dispatch<D: Doc>(&mut self, node: &Node<'_, D>) -> Result<(), BuildError> {
        match node.kind().as_ref() {
            "mod_item" => self.module(node),
            "function_item" | "function_signature_item" => self.function(node),
            "closure_expression" => self.closure(node),
            "block" => self.block(node, None),
            "let_declaration" => self.let_declaration(node),
            "if_expression" => self.if_expression(node),
            "while_expression" => self.while_expression(node),
            "loop_expression" => self.loop_expression(node),
            "for_expression" => self.for_expression(node),
            "match_arm" => self.match_arm(node),
            "struct_item" | "enum_item" | "union_item" | "type_item" | "trait_item"
            | "impl_item" => self.generic_item(node),
            "const_item" | "static_item" => {
                self.field(node, "type")?;
                self.field(node, "value")
            }
            "use_declaration" => {
                self.use_declaration(node);
                Ok(())
            }
            "extern_crate_declaration" | "macro_definition" => Ok(()),
            "macro_invocation" => {
                if let Some(name) = node
                    .field("macro")
                    .filter(|name| name.kind() == "identifier")
                {
                    self.builder.reference(name.range(), MACRO);
                }
                Ok(())
            }
            "enum_variant" => {
                self.field(node, "body")?;
                self.field(node, "value")
            }
            "field_declaration" => self.field(node, "type"),
            "scoped_identifier" | "scoped_type_identifier" => self.path(node),
            "generic_function" => {
                self.field(node, "function")?;
                self.field(node, "type_arguments")
            }
            "field_expression" => self.field(node, "value"),
            "field_initializer" => self.field(node, "value"),
            "struct_expression" => {
                self.type_name(node.field("name"))?;
                self.field(node, "body")
            }
            "type_binding" => {
                self.field(node, "type_arguments")?;
                self.field(node, "type")
            }
            "assignment_expression" | "compound_assignment_expr" => {
                match node.field("left") {
                    Some(left) if left.kind() == "identifier" => {
                        self.builder
                            .push_reference(left.range(), VALUE, true, false);
                    }
                    Some(left) => self.visit(&left)?,
                    None => {}
                }
                self.field(node, "right")
            }
            "break_expression" | "continue_expression" => {
                for child in node.children().filter(|child| child.is_named()) {
                    if child.kind() == "label" {
                        self.builder.reference(child.range(), LABEL);
                    } else {
                        self.visit(&child)?;
                    }
                }
                Ok(())
            }
            "identifier" | "self" => {
                self.builder.reference(node.range(), VALUE);
                Ok(())
            }
            "type_identifier" => {
                self.builder.reference(node.range(), TYPE);
                Ok(())
            }
            "lifetime" => {
                self.builder.reference(node.range(), LIFETIME);
                Ok(())
            }
            // Opaque to a syntax-level reader: macro arguments and attribute contents; names that
            // are no lexical reference: fields, labels where they are made, `pub(in path)`.
            "token_tree"
            | "attribute_item"
            | "inner_attribute_item"
            | "field_identifier"
            | "shorthand_field_identifier"
            | "label"
            | "visibility_modifier"
            | "crate"
            | "super"
            | "metavariable"
            | "line_comment"
            | "block_comment" => Ok(()),
            _ => self.children(node),
        }
    }

    // Items.

    /// Binds the items among `node`'s direct children at the start of the current scope: an item
    /// is visible in its whole block or module, before it too.
    fn items<D: Doc>(&mut self, node: &Node<'_, D>) {
        let start = self.scope_start();
        for child in node.children().filter(|child| child.is_named()) {
            let (spaces, form) = match child.kind().as_ref() {
                "function_item" | "function_signature_item" => (VALUE, Form::Definition),
                "struct_item" => {
                    // A tuple or unit struct is also its constructor, in the value namespace.
                    let braced = child
                        .field("body")
                        .is_some_and(|body| body.kind() == "field_declaration_list");
                    (if braced { TYPE } else { TYPE | VALUE }, Form::Definition)
                }
                "enum_item" | "union_item" | "trait_item" | "mod_item" => (TYPE, Form::Definition),
                "type_item" => (TYPE, Form::TypeAlias),
                "const_item" => (VALUE, Form::Assignment),
                "static_item" => (VALUE, Form::Static),
                "extern_crate_declaration" => {
                    self.extern_crate(&child, start);
                    continue;
                }
                // `macro_rules!` is textual: visible after its definition only.
                "macro_definition" => {
                    if let Some(name) = child.field("name") {
                        self.bind(&name, &child, MACRO, Form::Definition, child.range().end);
                    }
                    continue;
                }
                _ => continue,
            };
            let Some(name) = child.field("name") else {
                continue;
            };
            let mut binding = self.binding(&name, &child, spaces, form, start);
            if matches!(child.kind().as_ref(), "const_item" | "static_item") {
                binding.site.value = child.field("value").map(|value| NodeRef::of(&value));
                binding.site.annotation = child.field("type").map(|ty| NodeRef::of(&ty));
            }
            self.builder
                .bind(self.builder.current, &name.text(), binding);
        }
    }

    fn extern_crate<D: Doc>(&mut self, node: &Node<'_, D>, start: usize) {
        let Some(crate_name) = node.field("name") else {
            return;
        };
        let local = node.field("alias").unwrap_or_else(|| crate_name.clone());
        if local.text() == "_" {
            return;
        }
        let mut binding = self.binding(&local, node, TYPE, Form::Import, start);
        binding.import = Some(Import::Qualified(crate_name.text().as_ref().into()));
        self.builder
            .bind(self.builder.current, &local.text(), binding);
    }

    fn module<D: Doc>(&mut self, node: &Node<'_, D>) -> Result<(), BuildError> {
        if let Some(body) = node.field("body") {
            self.builder.push(ScopeKind::Module, body.range());
            self.items(&body);
            let result = self.children(&body);
            self.builder.pop();
            result?;
        }
        Ok(())
    }

    /// A struct, enum, union, type alias, trait or impl: its generic parameters (and `Self`) are
    /// visible in its whole declaration. The functions of an impl or trait body are no lexical
    /// names: they are reached through a path (`Self::f`).
    fn generic_item<D: Doc>(&mut self, node: &Node<'_, D>) -> Result<(), BuildError> {
        self.builder.push(ScopeKind::TypeParams, node.range());
        let start = node.range().start;
        if let Some(parameters) = node.field("type_parameters") {
            self.type_parameters(&parameters, start)?;
        }
        if matches!(node.kind().as_ref(), "impl_item" | "trait_item") {
            let name = node.field(if node.kind() == "impl_item" {
                "type"
            } else {
                "name"
            });
            let mut binding = Binding::new(
                start,
                TYPE,
                Site {
                    node: Some(NodeRef::of(node)),
                    ..Site::IMPLICIT
                },
            );
            binding.name = name.map(|name| NodeRef::of(&name));
            self.builder.bind(self.builder.current, "Self", binding);
        }
        for child in node.children().filter(|child| child.is_named()) {
            let is_name = node
                .field("name")
                .is_some_and(|name| name.range() == child.range());
            if is_name || child.kind() == "type_parameters" {
                continue;
            }
            if child.kind() == "declaration_list" {
                // The associated items' own signatures and bodies.
                self.children(&child)?;
            } else {
                self.visit(&child)?;
            }
        }
        self.builder.pop();
        Ok(())
    }

    /// Binds generic parameters at `start` (their item's) and reads their bounds and defaults.
    fn type_parameters<D: Doc>(
        &mut self,
        node: &Node<'_, D>,
        start: usize,
    ) -> Result<(), BuildError> {
        for parameter in node.children().filter(|child| child.is_named()) {
            let spaces = match parameter.kind().as_ref() {
                "type_parameter" => TYPE,
                // A const parameter is read in value and in type-argument position (`[u8; N]`,
                // `Holder::<N>`), which the grammar cannot tell from a type there.
                "const_parameter" => VALUE | TYPE,
                "lifetime_parameter" => LIFETIME,
                _ => continue,
            };
            if let Some(name) = parameter.field("name") {
                self.bind(&name, &parameter, spaces, Form::TypeParameter, start);
            }
            for child in parameter.children().filter(|child| child.is_named()) {
                let is_name = parameter
                    .field("name")
                    .is_some_and(|name| name.range() == child.range());
                if !is_name {
                    self.visit(&child)?;
                }
            }
        }
        Ok(())
    }

    fn function<D: Doc>(&mut self, node: &Node<'_, D>) -> Result<(), BuildError> {
        let start = node.range().start;
        self.builder.push(ScopeKind::Function, node.range());
        if let Some(parameters) = node.field("type_parameters") {
            self.type_parameters(&parameters, start)?;
        }
        if let Some(parameters) = node.field("parameters") {
            self.parameters(&parameters, start)?;
        }
        self.field(node, "return_type")?;
        for child in node
            .children()
            .filter(|child| child.kind() == "where_clause")
        {
            self.visit(&child)?;
        }
        if let Some(body) = node.field("body") {
            self.items(&body);
            self.children(&body)?;
        }
        self.builder.pop();
        Ok(())
    }

    /// A function's or closure's parameters: their types are read, their patterns bound at
    /// `start`.
    fn parameters<D: Doc>(&mut self, node: &Node<'_, D>, start: usize) -> Result<(), BuildError> {
        for parameter in node.children().filter(|child| child.is_named()) {
            match parameter.kind().as_ref() {
                "self_parameter" => {
                    if let Some(name) = parameter.children().find(|child| child.kind() == "self") {
                        self.bind(&name, &parameter, VALUE, Form::Parameter, start);
                    }
                    for lifetime in parameter
                        .children()
                        .filter(|child| child.kind() == "lifetime")
                    {
                        self.visit(&lifetime)?;
                    }
                }
                "parameter" => {
                    self.field(&parameter, "type")?;
                    if let Some(pattern) = parameter.field("pattern") {
                        if pattern.kind() == "self" {
                            self.bind(&pattern, &parameter, VALUE, Form::Parameter, start);
                        } else {
                            self.pattern(&pattern, &parameter, Form::Parameter, start, None)?;
                        }
                    }
                }
                "attribute_item" | "line_comment" | "block_comment" => {}
                // A closure's untyped parameter is a bare pattern.
                _ => self.pattern(&parameter, &parameter, Form::Parameter, start, None)?,
            }
        }
        Ok(())
    }

    fn closure<D: Doc>(&mut self, node: &Node<'_, D>) -> Result<(), BuildError> {
        self.builder.push(ScopeKind::Lambda, node.range());
        if let Some(parameters) = node.field("parameters") {
            self.parameters(&parameters, node.range().start)?;
        }
        self.field(node, "return_type")?;
        self.field(node, "body")?;
        self.builder.pop();
        Ok(())
    }

    /// A block: its items are bound first; `label` is the label of the expression it bodies.
    fn block<D: Doc>(
        &mut self,
        node: &Node<'_, D>,
        label: Option<&Node<'_, D>>,
    ) -> Result<(), BuildError> {
        self.builder.push(ScopeKind::Block, node.range());
        if let Some(label) = label {
            self.bind(label, label, LABEL, Form::Definition, node.range().start);
        }
        for child in node.children().filter(|child| child.kind() == "label") {
            self.bind(&child, node, LABEL, Form::Definition, node.range().start);
        }
        self.items(node);
        self.children(node)?;
        self.builder.pop();
        Ok(())
    }

    fn label<'r, D: Doc>(node: &Node<'r, D>) -> Option<Node<'r, D>> {
        node.children().find(|child| child.kind() == "label")
    }

    fn let_declaration<D: Doc>(&mut self, node: &Node<'_, D>) -> Result<(), BuildError> {
        // The initializer and the `else` block run before the pattern binds.
        self.field(node, "type")?;
        self.field(node, "value")?;
        self.field(node, "alternative")?;
        if let Some(pattern) = node.field("pattern") {
            let value = node
                .field("value")
                .filter(|_| pattern.kind() == "identifier");
            let offset = node.range().end;
            self.pattern(&pattern, node, Form::Assignment, offset, value.as_ref())?;
            if let Some(annotation) = node.field("type") {
                self.annotate(&pattern, &annotation);
            }
        }
        Ok(())
    }

    /// `if`: the bindings of its `let` conditions are visible in later conditions and the
    /// consequence, not in the `else`.
    fn if_expression<D: Doc>(&mut self, node: &Node<'_, D>) -> Result<(), BuildError> {
        self.builder.push(ScopeKind::Block, node.range());
        if let Some(condition) = node.field("condition") {
            self.condition(&condition, Form::Match)?;
        }
        self.field(node, "consequence")?;
        self.builder.pop();
        self.field(node, "alternative")
    }

    fn while_expression<D: Doc>(&mut self, node: &Node<'_, D>) -> Result<(), BuildError> {
        self.builder.push(ScopeKind::Block, node.range());
        if let Some(label) = Self::label(node) {
            self.bind(&label, node, LABEL, Form::Definition, node.range().start);
        }
        if let Some(condition) = node.field("condition") {
            self.condition(&condition, Form::Match)?;
        }
        self.field(node, "body")?;
        self.builder.pop();
        Ok(())
    }

    /// A condition: each `let` binds its pattern after its value, visible from its end on.
    fn condition<D: Doc>(&mut self, node: &Node<'_, D>, form: Form) -> Result<(), BuildError> {
        match node.kind().as_ref() {
            "let_condition" => {
                self.field(node, "value")?;
                if let Some(pattern) = node.field("pattern") {
                    self.pattern(&pattern, node, form, node.range().end, None)?;
                }
                Ok(())
            }
            "let_chain" => {
                for child in node.children().filter(|child| child.is_named()) {
                    self.condition(&child, form)?;
                }
                Ok(())
            }
            _ => self.visit(node),
        }
    }

    fn loop_expression<D: Doc>(&mut self, node: &Node<'_, D>) -> Result<(), BuildError> {
        let label = Self::label(node);
        match node.field("body") {
            Some(body) => self.block(&body, label.as_ref()),
            None => Ok(()),
        }
    }

    fn for_expression<D: Doc>(&mut self, node: &Node<'_, D>) -> Result<(), BuildError> {
        self.field(node, "value")?;
        self.builder.push(ScopeKind::Block, node.range());
        if let Some(label) = Self::label(node) {
            self.bind(&label, node, LABEL, Form::Definition, node.range().start);
        }
        if let Some(pattern) = node.field("pattern") {
            self.pattern(&pattern, node, Form::For, node.range().start, None)?;
        }
        self.field(node, "body")?;
        self.builder.pop();
        Ok(())
    }

    /// A match arm: its pattern binds for its guard and its value.
    fn match_arm<D: Doc>(&mut self, node: &Node<'_, D>) -> Result<(), BuildError> {
        self.builder.push(ScopeKind::Block, node.range());
        let start = node.range().start;
        if let Some(pattern) = node.field("pattern") {
            for child in pattern.children().filter(|child| child.is_named()) {
                let is_condition = pattern
                    .field("condition")
                    .is_some_and(|condition| condition.range() == child.range());
                if !is_condition {
                    self.pattern(&child, node, Form::Match, start, None)?;
                }
            }
            if let Some(condition) = pattern.field("condition") {
                self.condition(&condition, Form::Match)?;
            }
        }
        self.field(node, "value")?;
        self.builder.pop();
        Ok(())
    }

    /// Binds the names `pattern` makes at `offset`, for the construct `site`, and reads the paths
    /// and expressions in it. An `or_pattern` binds each name once (its first alternative).
    fn pattern<D: Doc>(
        &mut self,
        pattern: &Node<'_, D>,
        site: &Node<'_, D>,
        form: Form,
        offset: usize,
        value: Option<&Node<'_, D>>,
    ) -> Result<(), BuildError> {
        match pattern.kind().as_ref() {
            "identifier" => {
                let name = pattern.text();
                if self.pattern_path(&name, pattern.range().start) {
                    self.builder.reference(pattern.range(), VALUE);
                } else if self.bound_here(&name, offset) {
                    // A later alternative of an or-pattern names the binding the first made: an
                    // occurrence of it, which reads nothing.
                    self.builder
                        .push_reference(pattern.range(), VALUE, true, false);
                } else {
                    let mut binding = self.binding(pattern, site, VALUE, form, offset);
                    binding.site.value = value.map(NodeRef::of);
                    self.builder.bind(self.builder.current, &name, binding);
                }
                Ok(())
            }
            "shorthand_field_identifier" => {
                if self.bound_here(&pattern.text(), offset) {
                    self.builder
                        .push_reference(pattern.range(), VALUE, true, false);
                } else {
                    self.bind(pattern, site, VALUE, form, offset);
                }
                Ok(())
            }
            "tuple_struct_pattern" => {
                if let Some(path) = pattern.field("type") {
                    self.pattern_constructor(&path)?;
                }
                let constructor = pattern.field("type");
                for child in pattern.children().filter(|child| child.is_named()) {
                    if constructor
                        .as_ref()
                        .is_none_or(|path| path.range() != child.range())
                    {
                        self.pattern(&child, site, form, offset, None)?;
                    }
                }
                Ok(())
            }
            "struct_pattern" => {
                self.type_name(pattern.field("type"))?;
                for child in pattern.children().filter(|child| child.is_named()) {
                    if child.kind() == "field_pattern" {
                        if let Some(inner) = child.field("pattern") {
                            self.pattern(&inner, site, form, offset, None)?;
                        } else if let Some(name) = child.field("name") {
                            self.pattern(&name, site, form, offset, None)?;
                        }
                    }
                }
                Ok(())
            }
            "scoped_identifier" | "scoped_type_identifier" => self.path(pattern),
            "range_pattern" | "_literal_pattern" | "negative_literal" | "string_literal"
            | "integer_literal" | "float_literal" | "char_literal" | "boolean_literal"
            | "raw_string_literal" => self.children(pattern),
            "closure_expression" | "macro_invocation" => self.visit(pattern),
            "token_tree" | "attribute_item" | "line_comment" | "block_comment" => Ok(()),
            _ => {
                for child in pattern.children().filter(|child| child.is_named()) {
                    self.pattern(&child, site, form, offset, None)?;
                }
                Ok(())
            }
        }
    }

    /// A tuple struct or variant pattern's path: a value read.
    fn pattern_constructor<D: Doc>(&mut self, path: &Node<'_, D>) -> Result<(), BuildError> {
        match path.kind().as_ref() {
            "identifier" => {
                self.builder.reference(path.range(), VALUE);
                Ok(())
            }
            _ => self.visit(path),
        }
    }

    /// Whether an identifier pattern is a path rustc reads: it names a constant, a static, a unit
    /// or tuple struct of this file visible here, or a prelude unit variant. A function's name is
    /// no such path: a pattern binds over it.
    fn pattern_path(&self, name: &str, at: usize) -> bool {
        if PRELUDE_PATTERN_PATHS.contains(&name) {
            return true;
        }
        let probe = Reference {
            start: at,
            end: at,
            scope: self.builder.current,
            spaces: VALUE,
            at,
            deferred: false,
            forward: false,
            type_scope: false,
            write: false,
            mutates: false,
            access: super::Access::Load,
        };
        resolve_index(&self.builder.table, &probe, name).is_some_and(|(scope, index)| {
            let binding = &self.builder.table.scopes[scope as usize].bindings[name][index];
            let start = self.builder.table.scopes[scope as usize].range.start;
            match binding.site.form {
                Form::Static => true,
                // A unit or tuple struct is bound in both namespaces, a function in values only.
                Form::Definition => binding.spaces & TYPE != 0,
                // A `const` item is bound at its scope's start, a `let` after its declaration.
                Form::Assignment => binding.offset == start,
                _ => false,
            }
        })
    }

    /// Whether the current scope already binds `name` at `offset` (an or-pattern's later
    /// alternative).
    fn bound_here(&self, name: &str, offset: usize) -> bool {
        self.builder
            .scope(self.builder.current)
            .bindings
            .get(name)
            .is_some_and(|chain| {
                chain.iter().any(|binding| {
                    binding.offset == offset && binding.site.form != Form::Definition
                })
            })
    }

    /// Sets the annotation of a plain identifier pattern's binding.
    fn annotate<D: Doc>(&mut self, pattern: &Node<'_, D>, annotation: &Node<'_, D>) {
        if pattern.kind() != "identifier" {
            return;
        }
        let name = pattern.text();
        let current = self.builder.current;
        if let Some(binding) = self.builder.table.scopes[current as usize]
            .bindings
            .get_mut(name.as_ref())
            .and_then(|chain| chain.last_mut())
        {
            binding.site.annotation = Some(NodeRef::of(annotation));
        }
    }

    // Paths.

    /// A path `a::b::c`: its first segment is read (a module, type, enum, import or crate name,
    /// in the type namespace); `crate`, `self`, `super` and `Self` resolve no lexical name but
    /// `Self`; later segments need the first one's members.
    fn path<D: Doc>(&mut self, node: &Node<'_, D>) -> Result<(), BuildError> {
        let mut head = node.clone();
        while let Some(path) = head.field("path") {
            head = path;
        }
        match head.kind().as_ref() {
            "identifier" | "type_identifier" => {
                if head.range() == node.range() {
                    // A bare name: no path at all.
                    self.builder.reference(head.range(), VALUE);
                } else {
                    self.builder.reference(head.range(), TYPE);
                }
            }
            "generic_type" | "bracketed_type" => self.visit(&head)?,
            _ => {}
        }
        // Generic arguments on any segment (`Vec::<T>::new`) are types read here.
        let mut segment = Some(node.clone());
        while let Some(current) = segment {
            if current.kind() == "generic_type" {
                self.field(&current, "type_arguments")?;
            }
            segment = current.field("path");
        }
        Ok(())
    }

    /// A struct expression's or pattern's type name: a type read.
    fn type_name<D: Doc>(&mut self, name: Option<Node<'_, D>>) -> Result<(), BuildError> {
        match name {
            Some(name) if name.kind() == "type_identifier" => {
                self.builder.reference(name.range(), TYPE);
                Ok(())
            }
            Some(name) => self.visit(&name),
            None => Ok(()),
        }
    }

    // `use`.

    /// Binds what a `use` imports: each leaf under its local name (its alias, else its last
    /// segment; `self` in a list names the list's path), with the path as written. A glob binds
    /// nothing a reader can name: it is recorded as the scope's star. `as _` binds nothing.
    fn use_declaration<D: Doc>(&mut self, node: &Node<'_, D>) {
        let start = self.scope_start();
        if let Some(argument) = node.field("argument") {
            self.use_tree(&argument, "", node, start);
        }
    }

    fn use_tree<D: Doc>(
        &mut self,
        tree: &Node<'_, D>,
        prefix: &str,
        site: &Node<'_, D>,
        start: usize,
    ) {
        let joined = |text: &str| {
            if prefix.is_empty() {
                text.to_owned()
            } else {
                format!("{prefix}::{text}")
            }
        };
        match tree.kind().as_ref() {
            "identifier" | "crate" | "super" => {
                self.use_leaf(tree, &joined(&path_text(tree)), site, start);
            }
            "scoped_identifier" => {
                if let Some(name) = tree.field("name") {
                    self.use_leaf(&name, &joined(&path_text(tree)), site, start);
                }
            }
            "self" => {
                // `use a::{self}` imports `a` itself.
                if let Some(last) = prefix.rsplit("::").next().filter(|last| !last.is_empty()) {
                    let mut binding = self.binding(tree, site, TYPE, Form::Import, start);
                    binding.import = Some(Import::Qualified(prefix.into()));
                    self.builder.bind(self.builder.current, last, binding);
                }
            }
            "use_as_clause" => {
                let (Some(path), Some(alias)) = (tree.field("path"), tree.field("alias")) else {
                    return;
                };
                if alias.text() == "_" {
                    return;
                }
                let target = if path.kind() == "self" {
                    prefix.to_owned()
                } else {
                    joined(&path_text(&path))
                };
                let mut binding =
                    self.binding(&alias, site, TYPE | VALUE | MACRO, Form::Import, start);
                binding.import = Some(Import::Qualified(target.into()));
                self.builder
                    .bind(self.builder.current, &alias.text(), binding);
            }
            "scoped_use_list" => {
                let base = tree
                    .field("path")
                    .map(|path| joined(&path_text(&path)))
                    .unwrap_or_else(|| prefix.to_owned());
                if let Some(list) = tree.field("list") {
                    self.use_tree(&list, &base, site, start);
                }
            }
            "use_list" => {
                for child in tree.children().filter(|child| child.is_named()) {
                    self.use_tree(&child, prefix, site, start);
                }
            }
            "use_wildcard" => {
                let current = self.builder.current;
                self.builder.table.scopes[current as usize]
                    .stars
                    .push(tree.range().start);
            }
            _ => {}
        }
    }

    fn use_leaf<D: Doc>(
        &mut self,
        name: &Node<'_, D>,
        path: &str,
        site: &Node<'_, D>,
        start: usize,
    ) {
        if matches!(name.kind().as_ref(), "crate" | "super" | "self") {
            return;
        }
        let mut binding = self.binding(name, site, TYPE | VALUE | MACRO, Form::Import, start);
        binding.import = Some(Import::Qualified(path.into()));
        self.builder
            .bind(self.builder.current, &name.text(), binding);
    }

    // Bindings.

    fn scope_start(&self) -> usize {
        self.builder.scope(self.builder.current).range.start
    }

    fn binding<D: Doc>(
        &self,
        name: &Node<'_, D>,
        site: &Node<'_, D>,
        spaces: u8,
        form: Form,
        offset: usize,
    ) -> Binding {
        let mut binding = Binding::new(
            offset,
            spaces,
            Site {
                form,
                node: Some(NodeRef::of(site)),
                value: None,
                annotation: None,
            },
        );
        binding.name = Some(NodeRef::of(name));
        binding
    }

    fn bind<D: Doc>(
        &mut self,
        name: &Node<'_, D>,
        site: &Node<'_, D>,
        spaces: u8,
        form: Form,
        offset: usize,
    ) {
        let binding = self.binding(name, site, spaces, form, offset);
        self.builder
            .bind(self.builder.current, &name.text(), binding);
    }
}

/// A path's text with its whitespace removed (`std :: io` is `std::io`).
fn path_text<D: Doc>(node: &Node<'_, D>) -> String {
    node.text().chars().filter(|c| !c.is_whitespace()).collect()
}

/// A Rust file's table, as `ScopeTable::build` builds it.
#[cfg(test)]
pub(crate) fn build_for_test(source: &str) -> ScopeTable {
    use crate::code_rule_dsl::language::GrammarDialect;
    use ast_grep_core::tree_sitter::LanguageExt;
    let root = GrammarDialect::Rust.ast_grep(source);
    ScopeTable::build(root.root(), GrammarDialect::Rust, usize::MAX, &|| false)
        .expect("Rust scope build")
}

#[cfg(test)]
mod tests;
