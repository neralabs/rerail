//! Rust call resolution over a crate's files, from syntax and lexical scopes alone: which function a
//! `call_expression` names where no type or runtime value is needed. A bare name resolves through
//! the file's scope table (`scope::rust`), a path through its first segment, `Self::f` and
//! `Type::f` through the `impl` blocks of the crate, `self.m()` through the `impl` it sits in. What
//! needs types (other method calls, trait dispatch, closures and function pointers) is `Open`.
//! A macro's tokens are not parsed: a macro invocation whose expansion may call a function of the
//! crate is `Open` (`macro_may_call`), so a call there is left open, never absent.

use super::names::Name;
use super::summary::{self, Def, OPEN, Summary};
use crate::code_rule_dsl::language::Family;
use crate::code_rule_dsl::scope::rust::{UseHead, UsePath, glob_visible, resolve_index, use_path};
use crate::code_rule_dsl::scope::{Binding, Form, Reference, ScopeTable};
use ast_grep_core::{Doc, Node};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

#[cfg(test)]
mod tests;

/// The crates whose paths name library code, not the crate itself.
pub(crate) const LIBRARY_ROOTS: &[&str] = &["std", "core", "alloc"];
/// Tuple variants of the prelude: calling one builds a value, it calls no function.
const PRELUDE_CONSTRUCTORS: &[&str] = &["Some", "Ok", "Err"];

/// What a call expression calls.
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
/// dispatches). A deferred one knows none of them: a file's summary names every crate path a
/// call denotes, and the call graph confirms it against the crate's definitions (`link`).
pub(crate) struct Known {
    counts: HashMap<String, usize>,
    traits: HashSet<String>,
    deferred: bool,
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
            deferred: false,
        }
    }

    /// Names whose confirmation the call graph makes.
    fn deferred() -> Self {
        Self {
            counts: HashMap::new(),
            traits: HashSet::new(),
            deferred: true,
        }
    }

    /// Whether exactly one function of the crate has this name: a name two definitions share
    /// (an inherent and a trait method of one type, one function behind two `cfg`s) names neither.
    fn names_one(&self, name: &str) -> bool {
        self.deferred || self.counts.get(name) == Some(&1)
    }

    /// Whether `name` is a method or function of a trait.
    fn in_trait(&self, name: &str) -> bool {
        !self.deferred
            && name
                .rsplit_once("::")
                .is_some_and(|(owner, _)| self.traits.contains(owner))
    }
}

/// A file's module path from its crate-relative path: `src/lib.rs` and `src/main.rs` are the
/// crate root (`[]`), `src/a.rs` and `src/a/mod.rs` are `["a"]`, `src/a/b.rs` is `["a", "b"]`.
/// `None` for a file outside `src/`, outside Rust, or under `src/bin/` (each file there is a
/// crate root of its own). A module declared with `#[path]` is named by its declaration, not its
/// file: `summary` names such a crate opaque, so no call into it is linked by these names.
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
    /// body or block holds items too). Without a table no `impl`'s type is resolved, so its
    /// functions are not read.
    fn collect<D: Doc>(root: &Node<'_, D>, module: &[String], table: Option<&ScopeTable>) -> Self {
        let mut items = Items {
            functions: HashMap::new(),
            types: HashMap::new(),
            variants: HashSet::new(),
            externs: HashSet::new(),
        };
        let mut pending = Vec::new();
        items.scan(root, &qualify(module), &mut pending);
        let mut next = 0;
        while let Some(table) = table
            && next < pending.len()
        {
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

    /// Records the items among `node`'s descendants under `prefix`; impls wait in `pending`. The
    /// walk keeps its own stack: a file's nesting is not bounded.
    fn scan<'r, D: Doc>(
        &mut self,
        node: &Node<'r, D>,
        prefix: &str,
        pending: &mut Vec<Node<'r, D>>,
    ) {
        let mut stack: Vec<(Node<'r, D>, Rc<str>)> = Vec::new();
        let push_children =
            |stack: &mut Vec<(Node<'r, D>, Rc<str>)>, node: &Node<'r, D>, prefix: Rc<str>| {
                let children: Vec<_> = node.children().filter(|child| child.is_named()).collect();
                stack.extend(
                    children
                        .into_iter()
                        .rev()
                        .map(|child| (child, Rc::clone(&prefix))),
                );
            };
        push_children(&mut stack, node, Rc::from(prefix));
        while let Some((child, prefix)) = stack.pop() {
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
                        push_children(&mut stack, &body, Rc::from(qualified));
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
                        push_children(&mut stack, &body, Rc::from(qualified));
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
                        push_children(&mut stack, &body, Rc::from(qualified));
                    }
                }
                "extern_crate_declaration" => {
                    if let Some(local) = child.field("alias").or_else(|| child.field("name")) {
                        self.externs.insert(local.range().start);
                    }
                }
                _ => push_children(&mut stack, &child, prefix),
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
    let mut found: Vec<(String, usize)> = Items::collect(root, module, Some(table))
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
    let mut names: Vec<String> = Items::collect(root, module, Some(table))
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
/// Calls that build a value (a tuple struct or variant) have no entry; a macro invocation that may
/// call a function of the crate (`macro_may_call`) is `Open` at its macro's path. `module` is the
/// file's `module_of`; `table` its scope table; `known` the crate's definitions.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn callees<D: Doc>(
    root: &Node<'_, D>,
    module: &[String],
    table: &ScopeTable,
    known: &Known,
) -> Vec<(usize, Callee)> {
    let items = Items::collect(root, module, Some(table));
    let mut found = Vec::new();
    visit_calls(root, module, table, known, &items, &mut found);
    found
}

/// A definition of a trait (its default method): a call of its name dispatches, so it links to
/// nothing (`Known::in_trait`).
pub(crate) const TRAIT_METHOD: u8 = 8;

/// The crate a workspace file belongs to and its module there: the directory holding the crate's
/// `src/` (empty for the workspace root's) and `module_of` the path under it. A file outside a
/// crate's `src/` (a test, an example, a build script) and a binary under `src/bin/` is a crate
/// root of its own, named by its path.
pub(crate) fn crate_module(path: &str) -> (String, Vec<String>) {
    let split = match path.strip_prefix("src/") {
        Some(rest) => Some(("", rest)),
        None => path.find("/src/").map(|at| (&path[..at], &path[at + 5..])),
    };
    split
        .and_then(|(dir, rest)| Some((dir.to_owned(), module_of(&format!("src/{rest}"))?)))
        .unwrap_or_else(|| (path.to_owned(), Vec::new()))
}

/// A crate path's name in the call graph: qualified by the crate's directory, so two crates of a
/// workspace never share one.
fn graph_name(dir: &str, qualified: &str) -> String {
    if dir.is_empty() {
        qualified.to_owned()
    } else {
        format!("{dir}/{qualified}")
    }
}

/// The summary of the Rust file at `path` (`CallRules::summary`): its functions (`definitions`),
/// what each body calls by crate path, and whether a call needs types to tell. A library call names
/// nothing; a call that builds a value is none. Without `table` (it could not be built) the
/// summary holds nothing and is open. The module is the crate's directory and the file's module,
/// which `callee_at` reads back. A file that declares a module with a `#[path]` attribute names
/// its crate opaque (`crate_root`): the crate's module paths are not its files' (`module_of`), so
/// no call into it is linked.
pub(crate) fn summary<D: Doc>(
    root: &Node<'_, D>,
    path: &str,
    table: Option<&ScopeTable>,
    has_errors: bool,
) -> Summary {
    let (dir, module) = crate_module(path);
    let mut names = Interner::default();
    let module_name = names.intern(&graph_name(&dir, &qualify(&module)));
    let items = Items::collect(root, &module, table);
    let traits: HashSet<&str> = items
        .types
        .values()
        .filter(|(_, kind)| *kind == ItemKind::Trait)
        .map(|(name, _)| name.as_str())
        .collect();
    let mut defs = Vec::new();
    for function in root.dfs().filter(|node| node.kind() == "function_item") {
        let Some(qualified) = function
            .field("name")
            .and_then(|name| items.functions.get(&name.range().start))
        else {
            continue;
        };
        let in_trait = qualified
            .rsplit_once("::")
            .is_some_and(|(owner, _)| traits.contains(owner));
        let range = function.range();
        defs.push(Def {
            name: names.intern(&graph_name(&dir, qualified)),
            start: range.start as u32,
            end: range.end as u32,
            flags: if in_trait { TRAIT_METHOD } else { 0 },
        });
    }
    defs.sort_unstable_by_key(|def| def.start);
    let mut summary = Summary {
        module: Some(module_name),
        defs,
        open: has_errors || table.is_none(),
        ..Summary::default()
    };
    if declares_path_module(root) {
        summary
            .opaque
            .push(names.intern(&graph_name(&dir, "crate")));
    }
    let Some(table) = table else {
        return names.finish(summary);
    };
    let mut found = Vec::new();
    visit_calls(root, &module, table, &Known::deferred(), &items, &mut found);
    for (at, callee) in found {
        let owner = summary
            .functions_around(at as u32)
            .next()
            .map(|def| def.start);
        let owner = owner.map(|start| summary.defs.partition_point(|def| def.start < start));
        match callee {
            Callee::Path(path) => {
                let target = names.intern(&graph_name(&dir, &path));
                summary.targets.push(target);
                if let Some(owner) = owner {
                    summary.edges.push((owner as u32, target));
                }
            }
            Callee::External(_) => {}
            Callee::Open => {
                summary.open_sites = true;
                if let Some(owner) = owner {
                    summary.defs[owner].flags |= OPEN;
                }
            }
        }
    }
    names.finish(summary)
}

/// The crate root's graph name of a crate path's graph name (`dir/crate` of `dir/crate::a::f`),
/// which a summary names opaque where the crate's module paths are not its files'.
pub(crate) fn crate_root(name: &str) -> &str {
    name.split_once("::").map_or(name, |(root, _)| root)
}

/// Whether a `mod` item of the file carries a `path` attribute (`#[path = "x.rs"]`, or one a
/// `cfg_attr` may apply): the module's file, and those of the modules inside it, are not where
/// its path would put them.
fn declares_path_module<D: Doc>(root: &Node<'_, D>) -> bool {
    root.dfs()
        .filter(|node| node.kind() == "mod_item")
        .any(|module| {
            std::iter::successors(module.prev(), Node::prev)
                .take_while(|sibling| {
                    matches!(
                        sibling.kind().as_ref(),
                        "attribute_item" | "line_comment" | "block_comment"
                    )
                })
                .filter(|sibling| sibling.kind() == "attribute_item")
                .any(|attribute| {
                    let text: String = attribute
                        .text()
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .collect();
                    let inner = text.strip_prefix("#[").unwrap_or(&text);
                    inner.starts_with("path=")
                        || (inner.starts_with("cfg_attr(") && inner.contains("path="))
                })
        })
}

/// Interns a summary's names.
#[derive(Default)]
struct Interner {
    names: Vec<String>,
    ids: HashMap<String, u32>,
}

impl Interner {
    fn intern(&mut self, name: &str) -> u32 {
        if let Some(id) = self.ids.get(name) {
            return *id;
        }
        let id = self.names.len() as u32;
        self.names.push(name.to_owned());
        self.ids.insert(name.to_owned(), id);
        id
    }

    /// `summary` with these names, its edges and targets sorted and distinct.
    fn finish(self, mut summary: Summary) -> Summary {
        summary.edges.sort_unstable();
        summary.edges.dedup();
        summary.targets.sort_unstable();
        summary.targets.dedup();
        summary.names = self
            .names
            .iter()
            .map(|name| Name::new(Family::Rust, name))
            .collect();
        summary
    }
}

thread_local! {
    /// The items of the file whose calls `callee_at` last read, by its table (held, so its address
    /// is not another table's).
    static ITEMS: RefCell<Option<(Arc<ScopeTable>, Rc<Items>)>> = const { RefCell::new(None) };
}

/// What the call `node` calls (`CallRules::callee_at`), in the file whose summary is `summary`
/// and whose table is `table`: a crate path's graph name, `Open` where it needs types, `None` for
/// a library call and a call that builds a value.
pub(crate) fn callee_at<D: Doc>(
    node: &Node<'_, D>,
    summary: &Summary,
    table: &Arc<ScopeTable>,
) -> Option<summary::Callee> {
    let module_name = summary.name(summary.module?);
    let (dir, qualified) = module_name.rsplit_once('/').unwrap_or(("", module_name));
    let module: Vec<String> = qualified.split("::").skip(1).map(str::to_owned).collect();
    let items = ITEMS.with_borrow_mut(|cached| match cached {
        Some((held, items)) if Arc::ptr_eq(held, table) => Rc::clone(items),
        _ => {
            let items = Rc::new(Items::collect(&node.root().root(), &module, Some(table)));
            *cached = Some((Arc::clone(table), Rc::clone(&items)));
            items
        }
    });
    match read_call(node, &module, table, &Known::deferred(), &items)?.1 {
        Callee::Path(path) => Some(summary::Callee::Name(graph_name(dir, &path))),
        Callee::External(_) => None,
        Callee::Open => Some(summary::Callee::Open),
    }
}

/// Reads the calls among `node`'s descendants, skipping token trees: a macro invocation that may
/// call a function of the crate is `Open` at its macro's path. The walk keeps its own stack: a
/// file's nesting is not bounded.
fn visit_calls<D: Doc>(
    node: &Node<'_, D>,
    module: &[String],
    table: &ScopeTable,
    known: &Known,
    items: &Items,
    found: &mut Vec<(usize, Callee)>,
) {
    let read = |child: &Node<'_, D>| {
        child.is_named()
            && !matches!(
                child.kind().as_ref(),
                "token_tree" | "attribute_item" | "inner_attribute_item" | "macro_definition"
            )
    };
    let mut stack: Vec<_> = node.children().filter(read).collect();
    stack.reverse();
    while let Some(node) = stack.pop() {
        match node.kind().as_ref() {
            "call_expression" => {
                if let Some(entry) = read_call(&node, module, table, known, items) {
                    found.push(entry);
                }
            }
            "macro_invocation" if macro_may_call(&node) => {
                let at = node.field("macro").unwrap_or_else(|| node.clone());
                found.push((at.range().start, Callee::Open));
            }
            _ => {}
        }
        let first = stack.len();
        stack.extend(node.children().filter(read));
        stack[first..].reverse();
    }
}

/// The standard library's macros whose expansion calls no function of the crate but those its
/// arguments' tokens call: the formatting, assertion and constructor macros. `write!` and
/// `writeln!` are not among them: they call a method of their destination (`dst.write_fmt(..)`),
/// which needs its type.
const STD_MACROS: &[&str] = &[
    "assert",
    "assert_eq",
    "assert_ne",
    "cfg",
    "column",
    "compile_error",
    "concat",
    "dbg",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "env",
    "eprint",
    "eprintln",
    "file",
    "format",
    "format_args",
    "include_bytes",
    "include_str",
    "line",
    "matches",
    "module_path",
    "option_env",
    "panic",
    "print",
    "println",
    "stringify",
    "thread_local",
    "todo",
    "unimplemented",
    "unreachable",
    "vec",
];

/// Whether the expansion of the macro invocation `invocation` may call a function of the crate:
/// any macro but `STD_MACROS` may (its expansion is not read), and one of those may where its
/// tokens hold a call (`call_in_tokens`).
fn macro_may_call<D: Doc>(invocation: &Node<'_, D>) -> bool {
    let Some(path) = invocation.field("macro") else {
        return true;
    };
    let path: String = path.text().chars().filter(|c| !c.is_whitespace()).collect();
    let path = path.strip_prefix("::").unwrap_or(&path);
    let name = LIBRARY_ROOTS
        .iter()
        .find_map(|root| path.strip_prefix(root)?.strip_prefix("::"))
        .unwrap_or(path);
    if !STD_MACROS.contains(&name) {
        return true;
    }
    invocation
        .children()
        .filter(|child| child.kind() == "token_tree")
        .any(|tree| call_in_tokens(&tree))
}

/// Whether the tokens of `tree` (and of the token trees inside it) may hold a call: a
/// parenthesized token tree after a name, a `>` or `>>` (a turbofish), a `?` or another token
/// tree (`f(x)`, `a.m::<T>(x)`, `f(x)(y)`, `a[0](x)`), or a macro invocation of a macro that may
/// call one (`m!(..)`, `std::format!(..)` whose tokens hold a call). A tuple struct's or
/// variant's constructor (`Some(x)`) reads as a call: tokens are not resolved.
fn call_in_tokens<'r, D: Doc>(tree: &Node<'r, D>) -> bool {
    let mut trees = vec![tree.clone()];
    while let Some(tree) = trees.pop() {
        // The last four tokens before this one, nearest first.
        let mut previous: [Option<Node<'r, D>>; 4] = [None, None, None, None];
        for token in tree.children() {
            let kind = token.kind();
            if kind == "token_tree" {
                let kind_of = |at: usize| previous[at].as_ref().map(|token| token.kind());
                let is = |at: usize, want: &str| kind_of(at).is_some_and(|kind| kind == want);
                if is(0, "!") && is(1, "identifier") {
                    // Another macro's call: by its bare name, or the standard library's path.
                    let library = !is(2, "::")
                        || (is(3, "identifier")
                            && previous[3]
                                .as_ref()
                                .is_some_and(|root| LIBRARY_ROOTS.contains(&root.text().as_ref())));
                    let named = previous[1]
                        .as_ref()
                        .is_some_and(|name| STD_MACROS.contains(&name.text().as_ref()));
                    if !library || !named {
                        return true;
                    }
                } else if token.child(0).is_some_and(|open| open.kind() == "(")
                    && kind_of(0).is_some_and(|kind| {
                        matches!(
                            kind.as_ref(),
                            "identifier"
                                | "self"
                                | "super"
                                | "crate"
                                | ">"
                                | ">>"
                                | "?"
                                | "token_tree"
                        )
                    })
                {
                    return true;
                }
                trees.push(token.clone());
            }
            if !matches!(kind.as_ref(), "line_comment" | "block_comment") {
                previous.rotate_right(1);
                previous[0] = Some(token);
            }
        }
    }
    false
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
            // A glob import may bind the name to a function (`use m::*` of a `pub fn Some()`).
            Denoted::Unbound
                if PRELUDE_CONSTRUCTORS.contains(&text.as_ref())
                    && !self
                        .reference_at(name)
                        .is_some_and(|reference| glob_visible(self.table, reference)) =>
            {
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

    /// The reference the scope table records at `name`.
    fn reference_at<D: Doc>(&self, name: &Node<'_, D>) -> Option<&'a Reference> {
        let start = name.range().start;
        let position = self
            .table
            .references
            .partition_point(|reference| reference.start < start);
        self.table
            .references
            .get(position)
            .filter(|reference| reference.start == start)
    }

    /// The scope binding the reference at `name` resolves to, with its scope and index.
    fn binding_of<D: Doc>(
        &self,
        name: &Node<'_, D>,
        text: &str,
    ) -> Option<((u32, usize), &'a Binding)> {
        let reference = self.reference_at(name)?;
        let (scope, index) = resolve_index(self.table, reference, text)?;
        let binding = self.table.scopes[scope as usize]
            .bindings
            .get(text)?
            .get(index)?;
        Some(((scope, index), binding))
    }

    /// What the name at `name` denotes, by its lexical binding.
    fn denote<D: Doc>(&self, name: &Node<'_, D>, text: &str) -> Denoted {
        let Some((at, binding)) = self.binding_of(name, text) else {
            return Denoted::Unbound;
        };
        match binding.site.form {
            Form::Definition => self.item(binding),
            Form::Import => {
                let is_extern = |binding: &Binding| {
                    binding
                        .name
                        .is_some_and(|name| self.items.externs.contains(&(name.start as usize)))
                };
                match use_path(self.table, at, text, is_extern) {
                    Some(used) => self.imported(used),
                    None => Denoted::Opaque,
                }
            }
            _ => Denoted::Opaque,
        }
    }

    /// What a definition binding of the file denotes: a function, a module or type, else nothing
    /// a path reaches.
    fn item(&self, binding: &Binding) -> Denoted {
        let offset = binding.name.map(|name| name.start as usize);
        if let Some(name) = offset.and_then(|offset| self.items.functions.get(&offset)) {
            return Denoted::Crate(name.clone());
        }
        match offset.and_then(|offset| self.items.types.get(&offset)) {
            Some((name, kind)) => Denoted::Item(name.clone(), *kind),
            None => Denoted::Opaque,
        }
    }

    /// What a `use` path (`use_path`) denotes: `crate::`, `self::` and `super::` paths against the
    /// current module, a library or extern crate's path as is, a path through an item of the
    /// file from that item, and any other path relative to the module (a name of the module the
    /// crate's definitions tell; no definition gives one where it is an extern crate's).
    fn imported(&self, used: UsePath<'_>) -> Denoted {
        let mut segments = used.segments.iter();
        let Some(first) = segments.next() else {
            return Denoted::Opaque;
        };
        let mut denoted = match used.head {
            UseHead::Extern => return Denoted::External(used.segments.join("::")),
            UseHead::Item(binding) => match self.item(binding) {
                // A function heads no path.
                Denoted::Crate(_) => Denoted::Opaque,
                item => item,
            },
            UseHead::Keyword => match first.as_str() {
                "crate" => Denoted::Crate(String::from("crate")),
                "self" => Denoted::Crate(qualify(self.module)),
                _ => self.parent_module(),
            },
            UseHead::Unbound if LIBRARY_ROOTS.contains(&first.as_str()) => {
                return Denoted::External(used.segments.join("::"));
            }
            UseHead::Unbound => Denoted::Crate(format!("{}::{first}", qualify(self.module))),
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
