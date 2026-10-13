//! Rust call resolution over a crate's files, from syntax and lexical scopes alone: which function a
//! `call_expression` names where no type or runtime value is needed. A bare name resolves through
//! the file's scope table (`scope::rust`), a path through its first segment, `Self::f` and
//! `Type::f` through the `impl` blocks of the crate, `self.m()` through the `impl` it sits in. What
//! needs types (other method calls, trait dispatch, closures and function pointers) is `Open`;
//! calls inside macro token trees are not read.

use crate::code_rule_dsl::scope::rust::resolve_index;
use crate::code_rule_dsl::scope::{Binding, Form, Import, ScopeTable};
use ast_grep_core::{Doc, Node};
use std::collections::{HashMap, HashSet};

#[cfg(test)]
mod tests;

/// The crates whose paths name library code, not the crate itself.
const LIBRARY_ROOTS: &[&str] = &["std", "core", "alloc"];
/// Tuple variants of the prelude: calling one builds a value, it calls no function.
const PRELUDE_CONSTRUCTORS: &[&str] = &["Some", "Ok", "Err"];

/// What a call expression calls.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Callee {
    /// A function of the crate, by the qualified name `definitions` gives it.
    Path(String),
    /// A function of `std`, `core`, `alloc` or an extern crate, by its path as imported or written.
    External(String),
    /// Needs types or runtime values to tell.
    Open,
}

/// The qualified names of the whole crate, which a path may resolve to: every function the
/// crate's files define, and the traits among the types it names (a call through a trait
/// dispatches).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Known {
    counts: HashMap<String, usize>,
    traits: HashSet<String>,
}

impl Known {
    /// The crate's `definitions` of every file, and its `trait_names`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn new(
        definitions: impl IntoIterator<Item = String>,
        traits: impl IntoIterator<Item = String>,
    ) -> Self {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for name in definitions {
            *counts.entry(name).or_default() += 1;
        }
        Self {
            counts,
            traits: traits.into_iter().collect(),
        }
    }

    /// Whether exactly one function of the crate has this name: a name two definitions share
    /// (an inherent and a trait method of one type, one function behind two `cfg`s) names neither.
    fn names_one(&self, name: &str) -> bool {
        self.counts.get(name) == Some(&1)
    }

    /// Whether `name` is a method or function of a trait.
    fn in_trait(&self, name: &str) -> bool {
        name.rsplit_once("::")
            .is_some_and(|(owner, _)| self.traits.contains(owner))
    }
}

/// A file's module path from its crate-relative path: `src/lib.rs` and `src/main.rs` are the
/// crate root (`[]`), `src/a.rs` and `src/a/mod.rs` are `["a"]`, `src/a/b.rs` is `["a", "b"]`.
/// `None` for a file outside `src/`, outside Rust, or under `src/bin/` (each file there is a
/// crate root of its own). A module declared with `#[path]` is named by its declaration, not its
/// file: the caller must not ask for it.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn module_of(path: &str) -> Option<Vec<String>> {
    let rest = path.strip_prefix("src/")?;
    let stem = rest.strip_suffix(".rs")?;
    let mut segments: Vec<&str> = stem.split('/').collect();
    if segments.iter().any(|segment| segment.is_empty()) {
        return None;
    }
    if segments.first() == Some(&"bin") && segments.len() > 1 {
        return None;
    }
    match segments.as_slice() {
        ["lib"] | ["main"] => return Some(Vec::new()),
        _ => {}
    }
    if segments.last() == Some(&"mod") {
        segments.pop();
    }
    Some(segments.into_iter().map(str::to_owned).collect())
}

/// What a file declares: its functions and the items a path may head.
struct Items {
    /// Each function's qualified name by the offset of its name node.
    functions: HashMap<usize, String>,
    /// Each module, struct, enum, union and trait by the offset of its name node.
    types: HashMap<usize, (String, ItemKind)>,
    /// `crate::a::Enum::Variant` of each enum variant.
    variants: HashSet<String>,
    /// The offsets of the local names `extern crate` binds.
    externs: HashSet<usize>,
}

/// What a name in `Items::types` is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ItemKind {
    Module,
    Type,
    Trait,
}

impl Items {
    /// Scans a file's items: modules, functions, types and impls, wherever they nest (a function
    /// body or block holds items too).
    fn collect<D: Doc>(root: &Node<'_, D>, module: &[String], table: &ScopeTable) -> Self {
        let mut items = Items {
            functions: HashMap::new(),
            types: HashMap::new(),
            variants: HashSet::new(),
            externs: HashSet::new(),
        };
        let mut pending = Vec::new();
        items.scan(root, &qualify(module), &mut pending);
        let mut next = 0;
        while next < pending.len() {
            let node = pending[next].clone();
            next += 1;
            let module = enclosing_module(&node, module);
            let reader = Reader::new(table, &module, &items, None);
            let Some(owner) = node.field("type").and_then(|ty| reader.type_name(&ty)) else {
                continue;
            };
            let Some(body) = node.field("body") else {
                continue;
            };
            let mut inner = Vec::new();
            items.scan(&body, &owner, &mut inner);
            pending.extend(inner);
        }
        items
    }

    /// Records the items among `node`'s descendants under `prefix`; impls wait in `pending`.
    fn scan<'r, D: Doc>(
        &mut self,
        node: &Node<'r, D>,
        prefix: &str,
        pending: &mut Vec<Node<'r, D>>,
    ) {
        for child in node.children().filter(|child| child.is_named()) {
            match child.kind().as_ref() {
                "token_tree"
                | "attribute_item"
                | "inner_attribute_item"
                | "macro_invocation"
                | "macro_definition" => {}
                "mod_item" => {
                    let Some(name) = child.field("name") else {
                        continue;
                    };
                    let qualified = format!("{prefix}::{}", name.text());
                    self.types
                        .insert(name.range().start, (qualified.clone(), ItemKind::Module));
                    if let Some(body) = child.field("body") {
                        self.scan(&body, &qualified, pending);
                    }
                }
                "struct_item" | "union_item" | "enum_item" => {
                    let Some(name) = child.field("name") else {
                        continue;
                    };
                    let qualified = format!("{prefix}::{}", name.text());
                    if child.kind() == "enum_item" {
                        self.variants_of(&child, &qualified);
                    }
                    self.types
                        .insert(name.range().start, (qualified, ItemKind::Type));
                }
                "trait_item" => {
                    let Some(name) = child.field("name") else {
                        continue;
                    };
                    let qualified = format!("{prefix}::{}", name.text());
                    self.types
                        .insert(name.range().start, (qualified.clone(), ItemKind::Trait));
                    if let Some(body) = child.field("body") {
                        self.scan(&body, &qualified, pending);
                    }
                }
                "impl_item" => pending.push(child),
                "function_item" => {
                    let Some(name) = child.field("name") else {
                        continue;
                    };
                    let qualified = format!("{prefix}::{}", name.text());
                    self.functions.insert(name.range().start, qualified.clone());
                    if let Some(body) = child.field("body") {
                        self.scan(&body, &qualified, pending);
                    }
                }
                "extern_crate_declaration" => {
                    if let Some(local) = child.field("alias").or_else(|| child.field("name")) {
                        self.externs.insert(local.range().start);
                    }
                }
                _ => self.scan(&child, prefix, pending),
            }
        }
    }

    /// Records the variants of an enum.
    fn variants_of<D: Doc>(&mut self, node: &Node<'_, D>, qualified: &str) {
        let Some(body) = node.field("body") else {
            return;
        };
        for variant in body
            .children()
            .filter(|child| child.kind() == "enum_variant")
        {
            if let Some(name) = variant.field("name") {
                self.variants
                    .insert(format!("{qualified}::{}", name.text()));
            }
        }
    }
}

/// `crate::a::b` of the module path `["a", "b"]`.
fn qualify(module: &[String]) -> String {
    let mut name = String::from("crate");
    for segment in module {
        name.push_str("::");
        name.push_str(segment);
    }
    name
}

/// The module `node` sits in: the file's `module` and the inline `mod` blocks around the node.
fn enclosing_module<D: Doc>(node: &Node<'_, D>, module: &[String]) -> Vec<String> {
    let mut inline: Vec<String> = node
        .ancestors()
        .filter(|ancestor| ancestor.kind() == "mod_item")
        .filter_map(|ancestor| ancestor.field("name").map(|name| name.text().to_string()))
        .collect();
    inline.reverse();
    let mut path = module.to_vec();
    path.extend(inline);
    path
}

/// Every function a file defines, by qualified name, with the byte offset of its name node:
/// free functions `crate::a::f`, functions of inline modules, associated functions and methods of
/// `impl Type` and `impl Trait for Type` as `crate::a::Type::f`, and a trait's default methods as
/// `crate::a::Trait::f`. Functions nested in a function body are `crate::a::outer::inner`; an
/// `impl` whose type is no struct, enum or union path of the crate (a generic parameter, a
/// reference, a library type) defines nothing here. `module` is the file's `module_of`; `table` its
/// scope table.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn definitions<D: Doc>(
    root: &Node<'_, D>,
    module: &[String],
    table: &ScopeTable,
) -> Vec<(String, usize)> {
    let mut found: Vec<(String, usize)> = Items::collect(root, module, table)
        .functions
        .into_iter()
        .map(|(offset, name)| (name, offset))
        .collect();
    found.sort_by_key(|(_, offset)| *offset);
    found
}

/// The qualified names of the traits a file declares (`Known::new`'s second argument).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn trait_names<D: Doc>(
    root: &Node<'_, D>,
    module: &[String],
    table: &ScopeTable,
) -> Vec<String> {
    let mut names: Vec<String> = Items::collect(root, module, table)
        .types
        .into_values()
        .filter(|(_, kind)| *kind == ItemKind::Trait)
        .map(|(name, _)| name)
        .collect();
    names.sort();
    names
}

/// What each `call_expression` of a file calls, by the byte offset of the callee's name (the last
/// path segment, the method name, or the callee expression where it has no name), in source order.
/// Calls that build a value (a tuple struct or variant) have no entry, nor do calls in macro
/// token trees. `module` is the file's `module_of`; `table` its scope table; `known` the crate's
/// definitions.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn callees<D: Doc>(
    root: &Node<'_, D>,
    module: &[String],
    table: &ScopeTable,
    known: &Known,
) -> Vec<(usize, Callee)> {
    let items = Items::collect(root, module, table);
    let mut found = Vec::new();
    visit_calls(root, module, table, known, &items, &mut found);
    found
}

/// Reads the calls among `node`'s descendants, skipping token trees.
fn visit_calls<D: Doc>(
    node: &Node<'_, D>,
    module: &[String],
    table: &ScopeTable,
    known: &Known,
    items: &Items,
    found: &mut Vec<(usize, Callee)>,
) {
    for child in node.children().filter(|child| child.is_named()) {
        if matches!(
            child.kind().as_ref(),
            "token_tree" | "attribute_item" | "inner_attribute_item" | "macro_definition"
        ) {
            continue;
        }
        if child.kind() == "call_expression" {
            if let Some(entry) = read_call(&child, module, table, known, items) {
                found.push(entry);
            }
        }
        visit_calls(&child, module, table, known, items, found);
    }
}

/// The entry of one call expression, or `None` for a call that builds a value.
fn read_call<D: Doc>(
    call: &Node<'_, D>,
    module: &[String],
    table: &ScopeTable,
    known: &Known,
    items: &Items,
) -> Option<(usize, Callee)> {
    let mut function = call.field("function")?;
    while function.kind() == "generic_function" {
        function = function.field("function")?;
    }
    let module = enclosing_module(call, module);
    let reader = Reader::new(table, &module, items, Some(known));
    let offset = reader.callee_offset(&function);
    let outcome = match function.kind().as_ref() {
        "identifier" => reader.bare_call(&function),
        "scoped_identifier" => reader.path_call(&function),
        "field_expression" => reader.method_call(&function),
        _ => Outcome::Callee(Callee::Open),
    };
    match outcome {
        Outcome::Constructor => None,
        Outcome::Callee(callee) => Some((offset, callee)),
    }
}

/// How a call reads.
enum Outcome {
    /// The call builds a value, calls no function.
    Constructor,
    Callee(Callee),
}

/// What a name or path denotes, before the crate's definitions confirm it.
enum Denoted {
    /// A path in the crate (`crate::a::f`): a function only if the crate defines it so.
    Crate(String),
    /// A path through `std`, `core`, `alloc` or an extern crate.
    External(String),
    /// A module, struct, enum, union or trait of this file.
    Item(String, ItemKind),
    /// A function or a name that needs values (a local, a parameter, a static).
    Opaque,
    /// No lexical binding: a prelude item or a glob import's.
    Unbound,
}

/// Resolves names and paths at a place in a file.
struct Reader<'a> {
    table: &'a ScopeTable,
    /// The module the place sits in, inline modules included.
    module: &'a [String],
    items: &'a Items,
    known: Option<&'a Known>,
}

impl<'a> Reader<'a> {
    fn new(
        table: &'a ScopeTable,
        module: &'a [String],
        items: &'a Items,
        known: Option<&'a Known>,
    ) -> Self {
        Self {
            table,
            module,
            items,
            known,
        }
    }

    /// The offset a callee is reported at: its name, else the expression.
    fn callee_offset<D: Doc>(&self, function: &Node<'_, D>) -> usize {
        let name = match function.kind().as_ref() {
            "scoped_identifier" => function.field("name"),
            "field_expression" => function.field("field"),
            _ => None,
        };
        name.unwrap_or_else(|| function.clone()).range().start
    }

    /// `f()`.
    fn bare_call<D: Doc>(&self, name: &Node<'_, D>) -> Outcome {
        let text = name.text();
        match self.denote(name, &text) {
            Denoted::Unbound if PRELUDE_CONSTRUCTORS.contains(&text.as_ref()) => {
                Outcome::Constructor
            }
            Denoted::Unbound => Outcome::Callee(Callee::Open),
            // A tuple struct's name is its constructor.
            Denoted::Item(_, ItemKind::Type) => Outcome::Constructor,
            denoted => self.confirm(denoted),
        }
    }

    /// `a::b::f()`, `Self::f()`, `<T as Trait>::f()`.
    fn path_call<D: Doc>(&self, path: &Node<'_, D>) -> Outcome {
        let (Some(segments), Some(head)) = (path_segments(path), leftmost(path)) else {
            return Outcome::Callee(Callee::Open);
        };
        let Some((first, rest)) = segments.split_first() else {
            return Outcome::Callee(Callee::Open);
        };
        let mut denoted = match first.as_str() {
            "crate" => Denoted::Crate(String::from("crate")),
            "self" => Denoted::Crate(qualify(self.module)),
            "super" => self.parent_module(),
            "Self" => self.self_type(path),
            text => self.denote_head(&head, text),
        };
        for segment in rest {
            denoted = descend(denoted, segment);
        }
        self.confirm(denoted)
    }

    /// `x.m()`: only `self.m()` in an inherent `impl` that defines `m` is read.
    fn method_call<D: Doc>(&self, access: &Node<'_, D>) -> Outcome {
        let open = Outcome::Callee(Callee::Open);
        let (Some(value), Some(method)) = (access.field("value"), access.field("field")) else {
            return open;
        };
        if value.kind() != "self" {
            return open;
        }
        let Some(known) = self.known else {
            return open;
        };
        let Some(function) = access
            .ancestors()
            .find(|ancestor| ancestor.kind() == "function_item")
        else {
            return open;
        };
        let Some(implementation) = function
            .parent()
            .filter(|parent| parent.kind() == "declaration_list")
            .and_then(|list| list.parent())
            .filter(|parent| parent.kind() == "impl_item" && parent.field("trait").is_none())
        else {
            return open;
        };
        let Some(receiver) = receiver_of(&function) else {
            return open;
        };
        let method_name = method.text();
        let defines = implementation.field("body").is_some_and(|body| {
            body.children().any(|child| {
                child.kind() == "function_item"
                    && child
                        .field("name")
                        .is_some_and(|name| name.text() == method_name)
                    && receiver_of(&child).is_some_and(|other| other == receiver)
            })
        });
        if !defines {
            return open;
        }
        let Some(owner) = implementation
            .field("type")
            .and_then(|ty| self.type_name(&ty))
        else {
            return open;
        };
        let qualified = format!("{owner}::{method_name}");
        if known.names_one(&qualified) && !known.in_trait(&qualified) {
            Outcome::Callee(Callee::Path(qualified))
        } else {
            open
        }
    }

    /// The type of the `impl` around `at`, for `Self::f`; `Opaque` in a trait or where the type
    /// is not a path of the crate.
    fn self_type<D: Doc>(&self, at: &Node<'_, D>) -> Denoted {
        let Some(implementation) = at
            .ancestors()
            .find(|ancestor| matches!(ancestor.kind().as_ref(), "impl_item" | "trait_item"))
        else {
            return Denoted::Opaque;
        };
        if implementation.kind() != "impl_item" {
            return Denoted::Opaque;
        }
        match implementation
            .field("type")
            .and_then(|ty| self.type_name(&ty))
        {
            Some(name) => Denoted::Item(name, ItemKind::Type),
            None => Denoted::Opaque,
        }
    }

    /// The qualified name of the struct, enum or union an `impl`'s type names, in the crate.
    fn type_name<D: Doc>(&self, ty: &Node<'_, D>) -> Option<String> {
        let mut ty = ty.clone();
        while ty.kind() == "generic_type" {
            ty = ty.field("type")?;
        }
        let denoted = match ty.kind().as_ref() {
            "type_identifier" => self.denote(&ty, &ty.text()),
            "scoped_type_identifier" => {
                let segments = path_segments(&ty)?;
                let head = leftmost(&ty)?;
                let (first, rest) = segments.split_first()?;
                let mut denoted = match first.as_str() {
                    "crate" => Denoted::Crate(String::from("crate")),
                    "self" => Denoted::Crate(qualify(self.module)),
                    "super" => self.parent_module(),
                    text => self.denote_head(&head, text),
                };
                for segment in rest {
                    denoted = descend(denoted, segment);
                }
                denoted
            }
            _ => return None,
        };
        match denoted {
            Denoted::Item(name, ItemKind::Type) | Denoted::Crate(name) => Some(name),
            _ => None,
        }
    }

    /// The module above the current one, as `super` names it.
    fn parent_module(&self) -> Denoted {
        match self.module.split_last() {
            Some((_, parent)) => Denoted::Crate(qualify(parent)),
            None => Denoted::Opaque,
        }
    }

    /// What the head segment of a path denotes (type namespace).
    fn denote_head<D: Doc>(&self, head: &Node<'_, D>, text: &str) -> Denoted {
        let denoted = self.denote(head, text);
        match denoted {
            Denoted::Unbound if LIBRARY_ROOTS.contains(&text) => Denoted::External(text.to_owned()),
            Denoted::Unbound => Denoted::Opaque,
            other => other,
        }
    }

    /// The scope binding the reference at `name` resolves to.
    fn binding_of<D: Doc>(&self, name: &Node<'_, D>, text: &str) -> Option<&'a Binding> {
        let start = name.range().start;
        let position = self
            .table
            .references
            .partition_point(|reference| reference.start < start);
        let reference = self.table.references.get(position)?;
        if reference.start != start {
            return None;
        }
        let (scope, index) = resolve_index(self.table, reference, text)?;
        self.table.scopes[scope as usize]
            .bindings
            .get(text)?
            .get(index)
    }

    /// What the name at `name` denotes, by its lexical binding.
    fn denote<D: Doc>(&self, name: &Node<'_, D>, text: &str) -> Denoted {
        let Some(binding) = self.binding_of(name, text) else {
            return Denoted::Unbound;
        };
        match binding.site.form {
            Form::Definition => {
                let offset = binding.name.map(|name| name.start as usize);
                if let Some(name) = offset.and_then(|offset| self.items.functions.get(&offset)) {
                    return Denoted::Crate(name.clone());
                }
                match offset.and_then(|offset| self.items.types.get(&offset)) {
                    Some((name, kind)) => Denoted::Item(name.clone(), *kind),
                    None => Denoted::Opaque,
                }
            }
            Form::Import => {
                let offset = binding.name.map(|name| name.start as usize);
                match &binding.import {
                    Some(Import::Qualified(path)) => {
                        if offset.is_some_and(|offset| self.items.externs.contains(&offset)) {
                            Denoted::External(path.to_string())
                        } else {
                            self.normalise(path)
                        }
                    }
                    _ => Denoted::Opaque,
                }
            }
            _ => Denoted::Opaque,
        }
    }

    /// A `use` path against the current module: `crate::`, `self::` and `super::` as written, a
    /// library path as is, any other path relative to the module (a name of the module or an
    /// extern crate: the crate's definitions tell).
    fn normalise(&self, path: &str) -> Denoted {
        let mut segments = path.split("::");
        let Some(first) = segments.next() else {
            return Denoted::Opaque;
        };
        let mut denoted = match first {
            "crate" => Denoted::Crate(String::from("crate")),
            "self" => Denoted::Crate(qualify(self.module)),
            "super" => self.parent_module(),
            library if LIBRARY_ROOTS.contains(&library) => {
                return Denoted::External(path.to_owned());
            }
            _ => Denoted::Crate(format!("{}::{first}", qualify(self.module))),
        };
        for segment in segments {
            denoted = descend(denoted, segment);
        }
        denoted
    }

    /// Confirms a denoted path against the crate's definitions.
    fn confirm(&self, denoted: Denoted) -> Outcome {
        match denoted {
            Denoted::External(path) => Outcome::Callee(Callee::External(path)),
            Denoted::Crate(name) => {
                if self.items.variants.contains(&name) {
                    return Outcome::Constructor;
                }
                match self.known {
                    Some(known) if known.names_one(&name) && !known.in_trait(&name) => {
                        Outcome::Callee(Callee::Path(name))
                    }
                    _ => Outcome::Callee(Callee::Open),
                }
            }
            _ => Outcome::Callee(Callee::Open),
        }
    }
}

/// The segments of `a::b::f` (with `Vec::<T>` read as `Vec`); `None` where the path has a
/// qualified-type head (`<T as Trait>::f`) or is global (`::f`).
fn path_segments<D: Doc>(node: &Node<'_, D>) -> Option<Vec<String>> {
    match node.kind().as_ref() {
        "identifier" | "type_identifier" | "crate" | "self" | "super" => {
            Some(vec![node.text().to_string()])
        }
        "scoped_identifier" | "scoped_type_identifier" => {
            let mut segments = path_segments(&node.field("path")?)?;
            segments.push(node.field("name")?.text().to_string());
            Some(segments)
        }
        "generic_type" => path_segments(&node.field("type")?),
        _ => None,
    }
}

/// The node of a path's first segment.
fn leftmost<'r, D: Doc>(node: &Node<'r, D>) -> Option<Node<'r, D>> {
    let mut head = node.clone();
    loop {
        head = match head.kind().as_ref() {
            "scoped_identifier" | "scoped_type_identifier" => head.field("path")?,
            "generic_type" => head.field("type")?,
            _ => return Some(head),
        };
    }
}

/// What the path `denoted` followed by `segment` denotes.
fn descend(denoted: Denoted, segment: &str) -> Denoted {
    match denoted {
        Denoted::Crate(name) if segment == "super" => pop_segment(&name),
        Denoted::Crate(name) | Denoted::Item(name, ItemKind::Type | ItemKind::Module) => {
            Denoted::Crate(format!("{name}::{segment}"))
        }
        Denoted::External(name) => Denoted::External(format!("{name}::{segment}")),
        _ => Denoted::Opaque,
    }
}

/// The `self` parameter of a method as written, without whitespace; `None` for an associated
/// function.
fn receiver_of<D: Doc>(function: &Node<'_, D>) -> Option<String> {
    let parameters = function.field("parameters")?;
    let receiver = parameters
        .children()
        .find(|child| child.kind() == "self_parameter")?;
    Some(
        receiver
            .text()
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect(),
    )
}

/// The name without its last segment, as a module path; `crate` has none.
fn pop_segment(name: &str) -> Denoted {
    match name.rsplit_once("::") {
        Some((parent, _)) => Denoted::Crate(parent.to_owned()),
        None => Denoted::Opaque,
    }
}
