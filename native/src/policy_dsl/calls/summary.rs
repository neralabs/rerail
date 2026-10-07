//! One Python file's call summary: its definitions, what each calls by qualified name, and the
//! module-level names it re-exports. Rule-independent and file-local: callees are names, not
//! definitions, so a summary changes only when its own file does.
//!
//! A callee is named statically or not at all. `f()`, `mod.f()`, `a.b.C.m()` and a method's
//! `self.m()` are named through the file's scope table; `obj.m()`, `f()()` and `a[0]()` name
//! nothing and give no edge (type inference would be needed). A builtin names nothing either. An
//! unbound name, which a star import may bind, is open: its callee is unknown.

use super::names::Name;
use crate::policy_dsl::module_path::ModulePath;
use crate::policy_dsl::scope::options::{DEFAULT_PYTHON_MINOR, PythonOptions};
use crate::policy_dsl::scope::{Binding, Form, Import, ScopeTable};
use crate::policy_dsl::value::NodeRef;
use ast_grep_core::{Doc, Node};
use std::collections::HashMap;

/// Bumped whenever extraction changes what a summary holds for the same file.
pub(crate) const EXTRACTOR_VERSION: u32 = 6;

pub(crate) const ASYNC: u8 = 1;
pub(crate) const CLASS: u8 = 2;
/// The definition's body calls something unknown.
pub(crate) const OPEN: u8 = 4;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Summary {
    /// The name others import the module by (`pkg.mod`; `pkg` for `pkg/__init__.py`).
    pub(crate) module: Option<u32>,
    /// Names used by this summary, each once.
    pub(crate) names: Vec<Name>,
    /// Functions and classes, by node start.
    pub(crate) defs: Vec<Def>,
    /// `(definition, callee name)`: what each definition's own body calls. Sorted, distinct.
    pub(crate) edges: Vec<(u32, u32)>,
    /// Every callee name any call of the file names, in a definition or not. Sorted, distinct.
    pub(crate) targets: Vec<u32>,
    /// Module-level names other modules may call through: `(this module's name for it, the name
    /// it denotes)`, from imports and from assignments of a name.
    pub(crate) aliases: Vec<(u32, u32)>,
    /// Modules `from m import *` reads at module level.
    pub(crate) stars: Vec<u32>,
    /// PHP: what each class-like extends, implements and uses, and its trait adaptations:
    /// `(class or class.method, target, kind)` (`php::PARENT` and the other kinds), in
    /// declaration order.
    pub(crate) supers: Vec<(u32, u32, u8)>,
    /// The file has syntax errors, or its scope table could not be built: what its definitions
    /// call is not all known.
    pub(crate) open: bool,
    /// A call of the file names something unknown: its `calls` verdict is unknown there.
    pub(crate) open_sites: bool,
    /// The source's length, which a policy's `maxSourceBytes` reads.
    pub(crate) bytes: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Def {
    /// The qualified name (`pkg.mod.Class.method`, `pkg.mod.f.<locals>.g`).
    pub(crate) name: u32,
    /// The `function_definition` or `class_definition` node's start.
    pub(crate) start: u32,
    /// And its end.
    pub(crate) end: u32,
    pub(crate) flags: u8,
}

impl Def {
    #[cfg(test)]
    pub(crate) fn is_async(&self) -> bool {
        self.flags & ASYNC != 0
    }
}

/// What a call's callee denotes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Callee {
    Name(String),
    /// Unknown: the name is unbound, or a relative import outside a package.
    Open,
}

impl Summary {
    pub(crate) fn name(&self, id: u32) -> &str {
        self.names[id as usize].text()
    }

    /// Name `id` as interned: what the call graph keys by.
    pub(crate) fn interned(&self, id: u32) -> &Name {
        &self.names[id as usize]
    }

    /// The functions (not classes) whose node holds `at`, innermost first.
    pub(crate) fn functions_around(&self, at: u32) -> impl Iterator<Item = &Def> {
        let before = self.defs.partition_point(|def| def.start <= at);
        self.defs[..before]
            .iter()
            .rev()
            .filter(move |def| at < def.end && def.flags & CLASS == 0)
    }

    /// The definition whose node starts at `start`.
    pub(crate) fn def_at(&self, start: u32) -> Option<&Def> {
        let index = self.defs.partition_point(|def| def.start < start);
        self.defs.get(index).filter(|def| def.start == start)
    }

    /// Extracts `root`'s summary through its scope table; without one (it could not be built), a
    /// summary of its definitions alone, which is open.
    pub(crate) fn extract<D: Doc>(
        root: &Node<'_, D>,
        table: Option<&ScopeTable>,
        module: Option<&ModulePath>,
        has_errors: bool,
    ) -> Self {
        let mut extractor = Extractor::new(table, module);
        extractor.walk(root);
        if let Some(table) = table {
            extractor.aliases(root, table);
        }
        extractor.stars(root);
        let mut summary = extractor.finish();
        summary.open |= has_errors || table.is_none();
        summary
    }

    /// A stable serialization for the content store.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            32 + self
                .names
                .iter()
                .map(|name| name.text().len() + 4)
                .sum::<usize>()
                + 13 * self.defs.len()
                + 8 * (self.edges.len() + self.aliases.len())
                + 4 * (self.targets.len() + self.stars.len())
                + 9 * self.supers.len(),
        );
        let put = |out: &mut Vec<u8>, value: u32| out.extend_from_slice(&value.to_le_bytes());
        put(&mut out, EXTRACTOR_VERSION);
        out.push(u8::from(self.open));
        put(&mut out, self.module.unwrap_or(u32::MAX));
        out.push(u8::from(self.open_sites));
        put(&mut out, self.bytes);
        put(&mut out, self.names.len() as u32);
        for name in &self.names {
            put(&mut out, name.text().len() as u32);
            out.extend_from_slice(name.text().as_bytes());
        }
        put(&mut out, self.defs.len() as u32);
        for def in &self.defs {
            put(&mut out, def.name);
            put(&mut out, def.start);
            put(&mut out, def.end);
            out.push(def.flags);
        }
        for pairs in [&self.edges, &self.aliases] {
            put(&mut out, pairs.len() as u32);
            for (a, b) in pairs {
                put(&mut out, *a);
                put(&mut out, *b);
            }
        }
        for ids in [&self.targets, &self.stars] {
            put(&mut out, ids.len() as u32);
            for id in ids {
                put(&mut out, *id);
            }
        }
        put(&mut out, self.supers.len() as u32);
        for (key, target, kind) in &self.supers {
            put(&mut out, *key);
            put(&mut out, *target);
            out.push(*kind);
        }
        out
    }

    /// The inverse of `encode`; `None` for another extractor version or corrupt bytes.
    pub(crate) fn decode(bytes: &[u8]) -> Option<Self> {
        let mut reader = Reader { bytes, at: 0 };
        if reader.u32()? != EXTRACTOR_VERSION {
            return None;
        }
        let open = reader.u8()? != 0;
        let module = Some(reader.u32()?).filter(|id| *id != u32::MAX);
        let open_sites = reader.u8()? != 0;
        let source_bytes = reader.u32()?;
        let names = (0..reader.count(4)?)
            .map(|_| {
                let len = reader.u32()? as usize;
                let text = std::str::from_utf8(reader.take(len)?).ok()?;
                Some(Name::new(text))
            })
            .collect::<Option<Vec<_>>>()?;
        let known = |id: u32| (id as usize) < names.len();
        if module.is_some_and(|id| !known(id)) {
            return None;
        }
        let defs = (0..reader.count(13)?)
            .map(|_| {
                let def = Def {
                    name: reader.u32()?,
                    start: reader.u32()?,
                    end: reader.u32()?,
                    flags: reader.u8()?,
                };
                (known(def.name) && def.start <= def.end).then_some(def)
            })
            .collect::<Option<Vec<_>>>()?;
        let mut pairs = |first: &dyn Fn(u32) -> bool| {
            (0..reader.count(8)?)
                .map(|_| {
                    let pair = (reader.u32()?, reader.u32()?);
                    (first(pair.0) && known(pair.1)).then_some(pair)
                })
                .collect::<Option<Vec<_>>>()
        };
        let edges = pairs(&|def| (def as usize) < defs.len())?;
        let aliases = pairs(&known)?;
        let mut ids = || {
            (0..reader.count(4)?)
                .map(|_| reader.u32().filter(|id| known(*id)))
                .collect::<Option<Vec<_>>>()
        };
        let targets = ids()?;
        let stars = ids()?;
        let supers = (0..reader.count(9)?)
            .map(|_| {
                let entry = (reader.u32()?, reader.u32()?, reader.u8()?);
                (known(entry.0) && known(entry.1)).then_some(entry)
            })
            .collect::<Option<Vec<_>>>()?;
        (reader.at == bytes.len()).then_some(Self {
            module,
            names,
            defs,
            edges,
            targets,
            aliases,
            stars,
            supers,
            open,
            open_sites,
            bytes: source_bytes,
        })
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let slice = self.bytes.get(self.at..self.at.checked_add(len)?)?;
        self.at += len;
        Some(slice)
    }
    fn u8(&mut self) -> Option<u8> {
        self.take(1).map(|bytes| bytes[0])
    }
    fn u32(&mut self) -> Option<u32> {
        self.take(4)
            .map(|bytes| u32::from_le_bytes(bytes.try_into().expect("four bytes")))
    }
    /// A count of items of at least `size` bytes each, bounded by the bytes left so a corrupt
    /// count cannot allocate.
    fn count(&mut self, size: usize) -> Option<usize> {
        let count = self.u32()? as usize;
        (count.checked_mul(size)? <= self.bytes.len() - self.at).then_some(count)
    }
}

/// What a method's body calls through its first parameter: that parameter's name and the class.
type Receiver = (String, String);

/// The lexical context a node is visited in.
#[derive(Clone)]
struct Context {
    /// The definition whose body this is.
    owner: Option<u32>,
    /// Prepended to a definition's name here.
    prefix: String,
    /// In a class body: the class's qualified name.
    class: Option<String>,
    receiver: Option<Receiver>,
}

/// A callee before the file's definitions are all named.
enum Found {
    Name(String),
    /// A definition of this file, by its node's start, followed by attribute names.
    Local(u32, Vec<String>),
    Open,
}

struct Extractor<'t> {
    table: Option<&'t ScopeTable>,
    /// The file's module path, against which relative imports resolve.
    path: Option<&'t ModulePath>,
    /// The name others import the module by, which qualifies its definitions.
    module: Option<String>,
    names: Vec<Box<str>>,
    ids: HashMap<Box<str>, u32>,
    defs: Vec<Def>,
    edges: Vec<(u32, u32)>,
    targets: Vec<u32>,
    aliases: Vec<(u32, u32)>,
    stars: Vec<u32>,
    open_sites: bool,
}

impl<'t> Extractor<'t> {
    fn new(table: Option<&'t ScopeTable>, module: Option<&'t ModulePath>) -> Self {
        Self {
            table,
            path: module,
            module: module.and_then(ModulePath::import_name),
            names: Vec::new(),
            ids: HashMap::new(),
            defs: Vec::new(),
            edges: Vec::new(),
            targets: Vec::new(),
            aliases: Vec::new(),
            stars: Vec::new(),
            open_sites: false,
        }
    }

    fn intern(&mut self, name: &str) -> u32 {
        if let Some(id) = self.ids.get(name) {
            return *id;
        }
        let id = self.names.len() as u32;
        self.names.push(name.into());
        self.ids.insert(name.into(), id);
        id
    }

    fn finish(mut self) -> Summary {
        // Definitions were made in pre-order, which is by node start.
        debug_assert!(self.defs.is_sorted_by_key(|def| def.start));
        let module = self.module.clone().map(|module| self.intern(&module));
        for list in [&mut self.edges, &mut self.aliases] {
            list.sort_unstable();
            list.dedup();
        }
        for list in [&mut self.targets, &mut self.stars] {
            list.sort_unstable();
            list.dedup();
        }
        Summary {
            module,
            names: self.names.iter().map(|name| Name::new(name)).collect(),
            defs: self.defs,
            edges: self.edges,
            targets: self.targets,
            aliases: self.aliases,
            stars: self.stars,
            supers: Vec::new(),
            open: false,
            open_sites: self.open_sites,
            bytes: 0,
        }
    }

    /// Visits nodes in document order (pre-order), so definitions are made by node start.
    fn walk<D: Doc>(&mut self, root: &Node<'_, D>) {
        let top = Context {
            owner: None,
            prefix: self
                .module
                .as_ref()
                .map_or_else(String::new, |m| format!("{m}.")),
            class: None,
            receiver: None,
        };
        let mut contexts = vec![top];
        // Calls of a local definition, named once every definition is: `(owner, site, tail)`.
        let mut local = Vec::new();
        let mut stack = vec![(root.clone(), 0usize)];
        while let Some((node, context)) = stack.pop() {
            let before = stack.len();
            let kind = node.kind();
            let inner = match kind.as_ref() {
                "function_definition" | "class_definition" => {
                    self.definition(&node, &contexts[context])
                }
                "lambda" => Some(Context {
                    owner: None,
                    ..contexts[context].clone()
                }),
                _ => None,
            };
            if kind == "call"
                && let Some(table) = self.table
            {
                let owner = contexts[context].owner;
                let receiver = contexts[context].receiver.as_ref();
                match callee(table, self.path, &node, receiver) {
                    Some(Found::Name(name)) => {
                        let name = self.intern(&name);
                        self.call(owner, name);
                    }
                    Some(Found::Local(site, tail)) => local.push((owner, site, tail)),
                    Some(Found::Open) => self.open(owner),
                    None => {}
                }
            }
            match inner {
                Some(inner) => {
                    // A function's body runs in its own context, a class body in the class's; the
                    // rest (decorators, defaults, bases) where the definition is.
                    let body = node.field("body").map(|body| body.range());
                    let lambda = kind == "lambda";
                    contexts.push(inner);
                    let inner = contexts.len() - 1;
                    for child in node.children() {
                        let into = if lambda || Some(child.range()) == body {
                            inner
                        } else {
                            context
                        };
                        stack.push((child, into));
                    }
                }
                None => stack.extend(node.children().map(|child| (child, context))),
            }
            // Children were pushed in order; pop them in order.
            stack[before..].reverse();
        }
        for (owner, site, tail) in local {
            let Some(def) = self.defs.iter().find(|def| def.start == site).copied() else {
                // The binding's site is no definition this walk made: no callee is known.
                self.open(owner);
                continue;
            };
            let name = qualify(&self.names[def.name as usize], &tail);
            let name = self.intern(&name);
            self.call(owner, name);
        }
    }

    fn call(&mut self, owner: Option<u32>, name: u32) {
        self.targets.push(name);
        if let Some(owner) = owner {
            self.edges.push((owner, name));
        }
    }

    fn open(&mut self, owner: Option<u32>) {
        self.open_sites = true;
        if let Some(owner) = owner {
            self.defs[owner as usize].flags |= OPEN;
        }
    }

    /// Makes the definition `node` is and returns the context of its body.
    fn definition<D: Doc>(&mut self, node: &Node<'_, D>, outer: &Context) -> Option<Context> {
        let class = node.kind() == "class_definition";
        let qualified = format!("{}{}", outer.prefix, node.field("name")?.text());
        let flags = if class {
            CLASS
        } else if is_async(node) {
            ASYNC
        } else {
            0
        };
        let def = self.defs.len() as u32;
        let name = self.intern(&qualified);
        self.defs.push(Def {
            name,
            start: node.range().start as u32,
            end: node.range().end as u32,
            flags,
        });
        Some(if class {
            Context {
                owner: outer.owner,
                prefix: format!("{qualified}."),
                class: Some(qualified),
                receiver: None,
            }
        } else {
            Context {
                owner: Some(def),
                prefix: format!("{qualified}.<locals>."),
                class: None,
                receiver: outer.class.as_ref().and_then(|class| {
                    receiver_parameter(node).map(|parameter| (parameter, class.clone()))
                }),
            }
        })
    }

    /// Module-level names others may call through: imports, and assignments of a name.
    fn aliases<D: Doc>(&mut self, root: &Node<'_, D>, table: &ScopeTable) {
        let Some(module) = self.module.clone() else {
            return;
        };
        let Some(scope) = table.scopes.first() else {
            return;
        };
        let mut entries = Vec::new();
        for (name, bindings) in &scope.bindings {
            for binding in bindings {
                let target = match (&binding.import, binding.site.form) {
                    (Some(Import::Qualified(qualified)), _) => {
                        // `import a.b` binds `a` to `a`, which aliases nothing.
                        if binding.submodule.is_some() || qualified.as_ref() == name.as_ref() {
                            continue;
                        }
                        qualified.to_string()
                    }
                    (Some(Import::Relative { level, rest }), _) => {
                        match self.path.and_then(|m| m.relative(*level, rest, &[])) {
                            Some(target) => target,
                            None => continue,
                        }
                    }
                    (None, Form::Assignment | Form::Annotated) => {
                        let Some(value) = binding
                            .site
                            .value
                            .and_then(|value| value.find(root.clone()))
                        else {
                            continue;
                        };
                        match reference(table, self.path, &value, None) {
                            Some(Found::Name(target)) => target,
                            Some(Found::Local(site, tail)) => {
                                match self.defs.iter().find(|def| def.start == site) {
                                    Some(def) => qualify(&self.names[def.name as usize], &tail),
                                    None => continue,
                                }
                            }
                            _ => continue,
                        }
                    }
                    _ => continue,
                };
                entries.push((format!("{module}.{name}"), target));
            }
        }
        entries.sort();
        for (alias, target) in entries {
            let pair = (self.intern(&alias), self.intern(&target));
            self.aliases.push(pair);
        }
    }

    /// `from m import *` at module level.
    fn stars<D: Doc>(&mut self, root: &Node<'_, D>) {
        for statement in root.children() {
            if statement.kind() != "import_from_statement"
                || !statement.children().any(|c| c.kind() == "wildcard_import")
            {
                continue;
            }
            let Some(from) = statement.field("module_name") else {
                continue;
            };
            let text = from.text();
            let target = if from.kind() == "relative_import" {
                let level = text.chars().take_while(|c| *c == '.').count();
                let rest = text[level..].trim();
                // `relative` appends `rest`'s segments; `from . import *` has none.
                self.path
                    .and_then(|m| m.relative(level, rest, &[]))
                    .map(|name| name.trim_end_matches('.').to_owned())
            } else {
                Some(text.into_owned())
            };
            if let Some(target) = target.filter(|target| !target.is_empty()) {
                let id = self.intern(&target);
                self.stars.push(id);
            }
        }
    }
}

/// What the call `call` in `summary`'s file denotes: the same answer extraction recorded for it.
/// `None` where no static name denotes the callee.
pub(crate) fn callee_at<D: Doc>(
    table: &ScopeTable,
    path: Option<&ModulePath>,
    summary: &Summary,
    call: &Node<'_, D>,
) -> Option<Callee> {
    let receiver = enclosing_receiver(summary, call);
    match callee(table, path, call, receiver.as_ref())? {
        Found::Name(name) => Some(Callee::Name(name)),
        Found::Open => Some(Callee::Open),
        Found::Local(site, tail) => Some(match summary.def_at(site) {
            Some(def) => Callee::Name(qualify(summary.name(def.name), &tail)),
            None => Callee::Open,
        }),
    }
}

/// The receiver a call inside a method reads: the method's first parameter, and its class. As in
/// extraction's contexts, the nearest function or class whose body holds the node decides (its
/// decorators, defaults and bases are in the context around it); a lambda passes.
fn enclosing_receiver<D: Doc>(summary: &Summary, node: &Node<'_, D>) -> Option<Receiver> {
    let function = enclosing_scope(node)?;
    if function.kind() != "function_definition" {
        return None;
    }
    let class = enclosing_scope(&function)?;
    if class.kind() != "class_definition" {
        return None;
    }
    let class = summary.def_at(class.range().start as u32)?;
    Some((
        receiver_parameter(&function)?,
        summary.name(class.name).to_owned(),
    ))
}

/// The nearest function or class definition whose body holds `node`.
fn enclosing_scope<'r, D: Doc>(node: &Node<'r, D>) -> Option<Node<'r, D>> {
    let at = node.range().start;
    node.ancestors().find(|scope| {
        matches!(
            scope.kind().as_ref(),
            "function_definition" | "class_definition"
        ) && scope
            .field("body")
            .is_some_and(|body| body.range().contains(&at))
    })
}

/// A call's callee.
fn callee<D: Doc>(
    table: &ScopeTable,
    path: Option<&ModulePath>,
    call: &Node<'_, D>,
    receiver: Option<&Receiver>,
) -> Option<Found> {
    reference(table, path, &call.field("function")?, receiver)
}

/// What a name or attribute chain denotes: `f`, `mod.f`, `a.b.C.m`, `self.m` in a method.
fn reference<D: Doc>(
    table: &ScopeTable,
    path: Option<&ModulePath>,
    expression: &Node<'_, D>,
    receiver: Option<&Receiver>,
) -> Option<Found> {
    let mut head = unparenthesized(expression);
    let mut tail = Vec::new();
    while head.kind() == "attribute" {
        tail.push(head.field("attribute")?.text().into_owned());
        head = unparenthesized(&head.field("object")?);
    }
    if head.kind() != "identifier" {
        return None;
    }
    tail.reverse();
    let name = head.text();
    let found = table.reference(&head.range())?;
    let base = match table.resolve(found, &name).filter(|b| b.shadows()) {
        // A builtin is outside every workspace.
        None if PythonOptions::new(DEFAULT_PYTHON_MINOR).declares(&name) => return None,
        None => return Some(Found::Open),
        Some(binding) if binding.is_deletion() => return None,
        Some(Binding {
            import: Some(import),
            site,
            ..
        }) => match import {
            Import::Qualified(qualified) => qualified.to_string(),
            Import::Relative { level, rest } => {
                match path.and_then(|path| path.relative(*level, rest, &[])) {
                    Some(name) => name,
                    None => return Some(Found::Open),
                }
            }
            Import::Definition => {
                return Some(match site.node {
                    Some(NodeRef { start, .. }) => Found::Local(start, tail),
                    None => Found::Open,
                });
            }
        },
        Some(binding) => {
            let (parameter, class) = receiver?;
            if binding.site.form != Form::Parameter || name != parameter.as_str() {
                return None;
            }
            class.clone()
        }
    };
    Some(Found::Name(qualify(&base, &tail)))
}

fn qualify(base: &str, tail: &[String]) -> String {
    let mut name =
        String::with_capacity(base.len() + tail.iter().map(|t| t.len() + 1).sum::<usize>());
    name.push_str(base);
    for part in tail {
        name.push('.');
        name.push_str(part);
    }
    name
}

fn unparenthesized<'r, D: Doc>(node: &Node<'r, D>) -> Node<'r, D> {
    let mut node = node.clone();
    while node.kind() == "parenthesized_expression" {
        let mut inner = node
            .children()
            .filter(|child| child.is_named() && child.kind() != "comment");
        let (Some(expression), None) = (inner.next(), inner.next()) else {
            break;
        };
        drop(inner);
        node = expression;
    }
    node
}

fn is_async<D: Doc>(function: &Node<'_, D>) -> bool {
    function
        .children()
        .next()
        .is_some_and(|first| first.kind() == "async")
}

/// A method's receiver parameter: its first, where it is a plain or typed name and the method is
/// not a `@staticmethod`.
fn receiver_parameter<D: Doc>(function: &Node<'_, D>) -> Option<String> {
    let decorated = function
        .parent()
        .filter(|p| p.kind() == "decorated_definition");
    if decorated.is_some_and(|parent| {
        parent.children().any(|child| {
            child.kind() == "decorator"
                && child.text().trim_start_matches('@').trim() == "staticmethod"
        })
    }) {
        return None;
    }
    let parameters = function.field("parameters")?;
    let first = parameters.children().find(|child| child.is_named())?;
    let name = match first.kind().as_ref() {
        "identifier" => first,
        "typed_parameter" => first.children().find(|c| c.kind() == "identifier")?,
        "default_parameter" | "typed_default_parameter" => first.field("name")?,
        _ => return None,
    };
    Some(name.text().into_owned())
}
