//! One Python file's call summary: its definitions, what each calls by qualified name, the
//! module- and class-level names it binds, and its classes' bases. Rule-independent and
//! file-local: callees are names, not definitions, so a summary changes only when its own file
//! does.
//!
//! A callee is named where its binding is statically decided, else it is open (unknown), never
//! absent. `f()`, `mod.f()`, `a.b.C.m()` are named through the file's scope table, a name bound to
//! another (`run = helper`) through that name; a method's `self.m()`, `cls.m()` and `cls()` are
//! late bound (`pkg.C.m^`: the class's method and every override a workspace subclass declares).
//! A builtin, and a literal's method, name nothing and give no edge. Open: an unbound name (a star
//! import may bind it), a parameter, a value no name denotes (`obj.m()` of a local or a call's
//! result, `f()()`, `a[0]()`, a lambda, `super().m()`), a name bound on several paths unless all
//! are definitions, and a definition read before a later binding replaces it.
//!
//! A definition a decorator may replace is `DECORATED` (its call runs what the decorator
//! returned: open, though that usually calls it); one a later unconditional binding of its scope
//! replaces is `SHADOWED` (the name, read once the scope ran, is no longer it).

use super::names::Name;
use crate::code_rule_dsl::identifier::{python_identifier, python_name};
use crate::code_rule_dsl::module_path::ModulePath;
use crate::code_rule_dsl::scope::options::{DEFAULT_PYTHON_MINOR, PythonOptions};
use crate::code_rule_dsl::scope::{Binding, Form, Import, ScopeKind, ScopeTable};
use ast_grep_core::{Doc, Node};
use foldhash::{HashMap, HashMapExt};

/// Bumped whenever extraction changes what a summary holds for the same file.
pub(crate) const EXTRACTOR_VERSION: u32 = 9;

pub(crate) const ASYNC: u8 = 1;
pub(crate) const CLASS: u8 = 2;
/// The definition's body calls something unknown.
pub(crate) const OPEN: u8 = 4;
/// Python: a decorator that is not known to return the definition itself binds its name.
pub(crate) const DECORATED: u8 = 32;
/// Python: a later binding of the name that runs whenever its scope's body runs to its end
/// replaces the definition.
pub(crate) const SHADOWED: u8 = 64;

/// Python `supers` kinds, `(class, target, kind)`: a base, in order (`php::PARENT`'s value: the
/// graph indexes subclasses alike); a base or class keyword no static name denotes (`(class,
/// class)`); the metaclass.
pub(crate) const BASE: u8 = 0;
pub(crate) const UNKNOWN_BASE: u8 = 6;
pub(crate) const METACLASS: u8 = 7;

/// Decorators that return the definition they decorate (`functools.wraps(f)` when called).
const TRANSPARENT: [&str; 6] = [
    "abc.abstractmethod",
    "typing.final",
    "typing.override",
    "typing.no_type_check",
    "typing_extensions.final",
    "typing_extensions.override",
];

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
    /// Module- and class-level names other code may call through: `(this module's name for it,
    /// the name it denotes)`, from imports and from assignments of a name.
    pub(crate) aliases: Vec<(u32, u32)>,
    /// Module- and class-level names bound, on some path that may be the last, to a value no name
    /// denotes (a call's result, a lambda, a loop target): a call through one is unknown. Sorted,
    /// distinct.
    pub(crate) opaque: Vec<u32>,
    /// Modules `from m import *` reads at module level.
    pub(crate) stars: Vec<u32>,
    /// PHP: what each class-like extends, implements and uses, and its trait adaptations:
    /// `(class or class.method, target, kind)` (`php::PARENT` and the other kinds), in
    /// declaration order. Python: each class's bases and metaclass (`BASE` and the other kinds).
    pub(crate) supers: Vec<(u32, u32, u8)>,
    /// The file has syntax errors, or its scope table could not be built: what its definitions
    /// call is not all known.
    pub(crate) open: bool,
    /// A call of the file names something unknown: its `calls` verdict is unknown there.
    pub(crate) open_sites: bool,
    /// The source's length, which a code rule's `maxSourceBytes` reads.
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
            extractor.members(root, table);
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
                + 4 * (self.targets.len() + self.opaque.len() + self.stars.len())
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
        for ids in [&self.targets, &self.opaque, &self.stars] {
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
        let opaque = ids()?;
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
            opaque,
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

/// A method's receiver: its first parameter, the class whose body holds the method, and whether
/// the parameter is the class (a classmethod's `cls`) rather than an instance.
#[derive(Clone)]
struct Receiver {
    parameter: String,
    class: String,
    class_bound: bool,
}

impl Receiver {
    /// A call through the receiver, late bound (`pkg.C.m^`): `self.m()` and `cls.m()` the method,
    /// `cls()` the class (`pkg.C.^`), `self()` its `__call__`. A deeper attribute is an
    /// attribute's value: unknown.
    fn late(&self, tail: &[String]) -> Found {
        match tail {
            [] if self.class_bound => Found::Name(format!("{}.^", self.class)),
            [] => Found::Name(format!("{}.__call__^", self.class)),
            [method] => Found::Name(format!("{}.{method}^", self.class)),
            _ => Found::Open,
        }
    }
}

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

/// Names followed through bindings (`run = helper`) before a callee is unknown.
const MAX_FOLLOW: u8 = 4;

/// Expression kinds whose value is a builtin type's: their methods name nothing.
const LITERALS: [&str; 16] = [
    "string",
    "concatenated_string",
    "integer",
    "float",
    "true",
    "false",
    "none",
    "ellipsis",
    "list",
    "dictionary",
    "set",
    "tuple",
    "list_comprehension",
    "dictionary_comprehension",
    "set_comprehension",
    "generator_expression",
];

/// Constructs whose parts may not run whenever the statement around them does.
const CONDITIONAL: [&str; 18] = [
    "if_statement",
    "elif_clause",
    "else_clause",
    "for_statement",
    "while_statement",
    "try_statement",
    "except_clause",
    "except_group_clause",
    "finally_clause",
    "with_statement",
    "match_statement",
    "case_clause",
    "conditional_expression",
    "boolean_operator",
    "list_comprehension",
    "dictionary_comprehension",
    "set_comprehension",
    "generator_expression",
];

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
    opaque: Vec<u32>,
    stars: Vec<u32>,
    supers: Vec<(u32, u32, u8)>,
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
            opaque: Vec::new(),
            stars: Vec::new(),
            supers: Vec::new(),
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

    /// The definition whose node starts at `start`. Definitions are made in pre-order, which is
    /// by node start.
    fn def_index(&self, start: u32) -> Option<usize> {
        let index = self.defs.partition_point(|def| def.start < start);
        (self.defs.get(index)?.start == start).then_some(index)
    }

    /// `def`'s qualified name followed by `tail`.
    fn def_name(&self, def: usize, tail: &[String]) -> String {
        qualify(&self.names[self.defs[def].name as usize], tail)
    }

    fn finish(mut self) -> Summary {
        debug_assert!(self.defs.is_sorted_by_key(|def| def.start));
        let module = self.module.clone().map(|module| self.intern(&module));
        for list in [&mut self.edges, &mut self.aliases] {
            list.sort_unstable();
            list.dedup();
        }
        for list in [&mut self.targets, &mut self.opaque, &mut self.stars] {
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
            opaque: self.opaque,
            stars: self.stars,
            supers: self.supers,
            open: false,
            open_sites: self.open_sites,
            bytes: 0,
        }
    }

    /// Visits nodes in document order (pre-order), so definitions are made by node start. A
    /// lambda's calls are its enclosing definition's: it runs them when it runs the lambda, or
    /// passes the lambda on.
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
                    self.definition(root, &node, &contexts[context])
                }
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
                    contexts.push(inner);
                    let inner = contexts.len() - 1;
                    for child in node.children() {
                        let into = if Some(child.range()) == body {
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
            let Some(def) = self.def_index(site) else {
                // The binding's site is no definition this walk made: no callee is known.
                self.open(owner);
                continue;
            };
            let name = self.def_name(def, &tail);
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
    fn definition<D: Doc>(
        &mut self,
        root: &Node<'_, D>,
        node: &Node<'_, D>,
        outer: &Context,
    ) -> Option<Context> {
        let class = node.kind() == "class_definition";
        let qualified = format!(
            "{}{}",
            outer.prefix,
            python_identifier(&node.field("name")?)
        );
        let mut flags = if class {
            CLASS
        } else if is_async(node) {
            ASYNC
        } else {
            0
        };
        if self.replaced(root, node, outer) {
            flags |= DECORATED;
        }
        let def = self.defs.len() as u32;
        let name = self.intern(&qualified);
        self.defs.push(Def {
            name,
            start: node.range().start as u32,
            end: node.range().end as u32,
            flags,
        });
        if class {
            self.bases(node, name, outer);
        }
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
                    receiver_parameter(node).map(|(parameter, class_bound)| Receiver {
                        parameter,
                        class: class.clone(),
                        class_bound,
                    })
                }),
            }
        })
    }

    /// Whether a decorator of `definition` may bind its name to another value than the
    /// definition: any but `staticmethod`, `classmethod`, the `TRANSPARENT` ones,
    /// `functools.wraps(f)`, and a function of this file that returns its argument.
    fn replaced<D: Doc>(
        &self,
        root: &Node<'_, D>,
        definition: &Node<'_, D>,
        outer: &Context,
    ) -> bool {
        let Some(decorated) = definition
            .parent()
            .filter(|parent| parent.kind() == "decorated_definition")
        else {
            return false;
        };
        let Some(table) = self.table else {
            return true;
        };
        decorated
            .children()
            .filter(|child| child.kind() == "decorator")
            .any(|decorator| {
                let Some(expression) = decorator
                    .children()
                    .find(|child| child.is_named() && child.kind() != "comment")
                else {
                    return true;
                };
                let expression = unparenthesized(&expression);
                let called = expression.kind() == "call";
                let Some(callee) = (if called {
                    expression.field("function")
                } else {
                    Some(expression)
                }) else {
                    return true;
                };
                let receiver = outer.receiver.as_ref();
                let transparent = match follow(table, self.path, &callee, receiver, Vec::new(), 0) {
                    Some(Found::Name(name)) if called => name == "functools.wraps",
                    Some(Found::Name(name)) => TRANSPARENT.contains(&name.as_str()),
                    None => {
                        !called
                            && callee.kind() == "identifier"
                            && matches!(
                                python_identifier(&callee).as_ref(),
                                "staticmethod" | "classmethod"
                            )
                    }
                    Some(Found::Local(site, tail)) => {
                        !called && tail.is_empty() && self.identity(root, site)
                    }
                    Some(Found::Open) => false,
                };
                !transparent
            })
    }

    /// Whether this file's function at `site` returns its first argument, as it got it, on every
    /// path.
    fn identity<D: Doc>(&self, root: &Node<'_, D>, site: u32) -> bool {
        let Some(def) = self.def_index(site).map(|def| self.defs[def]) else {
            return false;
        };
        def.flags & (CLASS | ASYNC | DECORATED) == 0
            && node_at(
                root,
                def.start as usize..def.end as usize,
                "function_definition",
            )
            .is_some_and(|function| returns_its_argument(&function))
    }

    /// `class`'s bases, in order, and its metaclass, as `supers` entries: a base by the name it
    /// denotes (`builtins.X` for a builtin; `object` is every class's), and a splat or a base no
    /// name denotes as `UNKNOWN_BASE`. Other class keywords go to `__init_subclass__`.
    fn bases<D: Doc>(&mut self, class: &Node<'_, D>, key: u32, outer: &Context) {
        let Some(arguments) = class.field("superclasses") else {
            return;
        };
        for argument in arguments
            .children()
            .filter(|child| child.is_named() && child.kind() != "comment")
        {
            let (expression, kind) = match argument.kind().as_ref() {
                "keyword_argument" => match (argument.field("name"), argument.field("value")) {
                    (Some(name), Some(value)) if python_identifier(&name) == "metaclass" => {
                        (value, METACLASS)
                    }
                    _ => continue,
                },
                "list_splat" | "dictionary_splat" => {
                    self.supers.push((key, key, UNKNOWN_BASE));
                    continue;
                }
                _ => (argument, BASE),
            };
            let mut head = unparenthesized(&expression);
            // `Generic[T]`, `Base[int]`: the class the alias gives as its MRO entry.
            if head.kind() == "subscript"
                && let Some(value) = head.field("value")
            {
                head = unparenthesized(&value);
            }
            let found = match self.table {
                Some(table) => follow(
                    table,
                    self.path,
                    &head,
                    outer.receiver.as_ref(),
                    Vec::new(),
                    0,
                ),
                None => Some(Found::Open),
            };
            let target = match found {
                Some(Found::Name(name)) => Some(name),
                Some(Found::Local(site, tail)) => {
                    self.def_index(site).map(|def| self.def_name(def, &tail))
                }
                Some(Found::Open) => None,
                None if head.kind() == "identifier" => {
                    let builtin = python_identifier(&head);
                    if builtin == "object" || (kind == METACLASS && builtin == "type") {
                        continue;
                    }
                    Some(format!("builtins.{builtin}"))
                }
                None => None,
            };
            match target {
                Some(target) => {
                    let target = self.intern(&target);
                    self.supers.push((key, target, kind));
                }
                None => self.supers.push((key, key, UNKNOWN_BASE)),
            }
        }
    }

    /// The module's and its classes' names other code may call through (`aliases`, `opaque`),
    /// from the bindings that may be their last; and the definitions no longer bound once their
    /// scope's body has run (`SHADOWED`).
    fn members<D: Doc>(&mut self, root: &Node<'_, D>, table: &ScopeTable) {
        let mut aliases = Vec::new();
        let mut opaque = Vec::new();
        for (index, scope) in table.scopes.iter().enumerate() {
            let prefix = if index == 0 {
                self.module.clone()
            } else if scope.kind == ScopeKind::Class {
                self.def_index(scope.range.start as u32)
                    .map(|def| self.def_name(def, &[]))
            } else {
                None
            };
            for (name, bindings) in &scope.bindings {
                let last = last_bindings(root, bindings);
                for (at, binding) in bindings.iter().enumerate() {
                    if matches!(binding.import, Some(Import::Definition))
                        && !last.contains(&at)
                        && let Some(def) = binding
                            .site
                            .node
                            .and_then(|site| self.def_index(site.start))
                    {
                        self.defs[def].flags |= SHADOWED;
                    }
                }
                let Some(prefix) = &prefix else {
                    continue;
                };
                for at in last {
                    match self.member(root, table, &bindings[at]) {
                        Some(Some(target)) => aliases.push((format!("{prefix}.{name}"), target)),
                        Some(None) => opaque.push(format!("{prefix}.{name}")),
                        None => {}
                    }
                }
            }
        }
        // Interned in a stable order: a summary is the same for the same file.
        aliases.sort();
        for (alias, target) in aliases {
            let pair = (self.intern(&alias), self.intern(&target));
            self.aliases.push(pair);
        }
        opaque.sort();
        for name in opaque {
            let id = self.intern(&name);
            self.opaque.push(id);
        }
    }

    /// What a module or class member's `binding` makes it: an alias of a name (`Some(Some)`), a
    /// value no name denotes (`Some(None)`), or nothing a call follows (`None`: a definition,
    /// which the graph links by name, a literal, a builtin, a deletion).
    fn member<D: Doc>(
        &self,
        root: &Node<'_, D>,
        table: &ScopeTable,
        binding: &Binding,
    ) -> Option<Option<String>> {
        match (&binding.import, binding.site.form) {
            (Some(Import::Definition), _) | (None, Form::Deletion) => None,
            (Some(Import::Qualified(qualified)), _) => Some(Some(qualified.to_string())),
            (Some(Import::Relative { level, rest }), _) => {
                Some(self.path.and_then(|path| path.relative(*level, rest, &[])))
            }
            (None, Form::Assignment | Form::Annotated | Form::Named) => {
                let Some(value) = binding
                    .site
                    .value
                    .and_then(|value| value.find(root.clone()))
                else {
                    return Some(None);
                };
                match follow(table, self.path, &value, None, Vec::new(), 0)? {
                    Found::Name(target) => Some(Some(target)),
                    Found::Local(site, tail) => {
                        Some(self.def_index(site).map(|def| self.def_name(def, &tail)))
                    }
                    Found::Open => Some(None),
                }
            }
            _ => Some(None),
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
            let text = python_name(from.text());
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
/// `None` where the call can run no workspace definition.
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
    let (parameter, class_bound) = receiver_parameter(&function)?;
    Some(Receiver {
        parameter,
        class: summary.name(class.name).to_owned(),
        class_bound,
    })
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
    follow(
        table,
        path,
        &call.field("function")?,
        receiver,
        Vec::new(),
        0,
    )
}

/// What a callee expression denotes, with the attribute names `after` read from it: `f`,
/// `mod.f`, `a.b.C.m`, `self.m` in a method, `run` where `run = helper`. `None` where it can be
/// no workspace definition (a builtin, a literal's method, a deleted name).
///
/// A name's binding decides it where it is the one live at the read: the read sees the last
/// bindings of the name's scope (from a function nested in it, as Ruff reads it, or past all of
/// them), and they are one binding or definitions alone; or the read comes before a later binding,
/// outside any loop, and its binding runs unconditionally. Several last bindings of a module's
/// name are what the graph links for the module's member.
fn follow<D: Doc>(
    table: &ScopeTable,
    path: Option<&ModulePath>,
    expression: &Node<'_, D>,
    receiver: Option<&Receiver>,
    after: Vec<String>,
    depth: u8,
) -> Option<Found> {
    let mut head = unparenthesized(expression);
    let mut tail = Vec::new();
    while head.kind() == "attribute" {
        let (Some(attribute), Some(object)) = (head.field("attribute"), head.field("object"))
        else {
            return Some(Found::Open);
        };
        tail.push(python_identifier(&attribute).into_owned());
        head = unparenthesized(&object);
    }
    tail.reverse();
    tail.extend(after);
    if head.kind() != "identifier" {
        // A literal's methods are its builtin type's; any other value (a call's result, a
        // subscript, a lambda, `super()`) is known only at run time.
        return (!LITERALS.contains(&head.kind().as_ref())).then_some(Found::Open);
    }
    let name = python_identifier(&head);
    let Some(reference) = table.reference(&head.range()) else {
        return Some(Found::Open);
    };
    let Some((scope, index)) = table
        .resolve_python_index(reference, &name)
        .filter(|found| table.binding_at(*found, &name).shadows())
    else {
        // A builtin is outside every workspace; an unbound name a star import may bind.
        return (!PythonOptions::new(DEFAULT_PYTHON_MINOR).declares(&name)).then_some(Found::Open);
    };
    let bindings = &table.scopes[scope as usize].bindings[name.as_ref()];
    let binding = &bindings[index];
    if binding.is_deletion() {
        return None;
    }
    if binding.site.form == Form::Parameter {
        // A method's receiver is its class's or a subclass's; any other parameter, the caller's.
        return Some(
            match receiver.filter(|receiver| receiver.parameter == name.as_ref()) {
                Some(receiver) => receiver.late(&tail),
                None => Found::Open,
            },
        );
    }
    let root = expression.root().root();
    if index + 1 < bindings.len() && !crosses_function(table, reference.scope, scope) {
        return if !matches!(binding.import, Some(Import::Definition))
            && unconditional(&root, binding)
            && !in_loop(&head)
        {
            bound(table, path, &root, binding, tail, receiver, depth)
        } else {
            Some(Found::Open)
        };
    }
    let last = last_bindings(&root, bindings);
    let live = last
        .iter()
        .map(|at| &bindings[*at])
        .filter(|binding| !binding.is_deletion())
        .collect::<Vec<_>>();
    match live.as_slice() {
        [] => None,
        [only] => bound(table, path, &root, only, tail, receiver, depth),
        several
            if several
                .iter()
                .all(|binding| matches!(binding.import, Some(Import::Definition))) =>
        {
            bound(table, path, &root, binding, tail, receiver, depth)
        }
        _ if scope == 0 => Some(match path.and_then(ModulePath::import_name) {
            Some(module) => Found::Name(qualify(&format!("{module}.{name}"), &tail)),
            None => Found::Open,
        }),
        _ => Some(Found::Open),
    }
}

/// What one binding makes a callee with attribute names `tail`.
fn bound<D: Doc>(
    table: &ScopeTable,
    path: Option<&ModulePath>,
    root: &Node<'_, D>,
    binding: &Binding,
    tail: Vec<String>,
    receiver: Option<&Receiver>,
    depth: u8,
) -> Option<Found> {
    Some(match (&binding.import, binding.site.form) {
        (Some(Import::Qualified(qualified)), _) => Found::Name(qualify(qualified, &tail)),
        (Some(Import::Relative { level, rest }), _) => {
            match path.and_then(|path| path.relative(*level, rest, &[])) {
                Some(name) => Found::Name(qualify(&name, &tail)),
                None => Found::Open,
            }
        }
        (Some(Import::Definition), _) => match binding.site.node {
            Some(node) => Found::Local(node.start, tail),
            None => Found::Open,
        },
        (None, Form::Assignment | Form::Annotated | Form::Named) if depth < MAX_FOLLOW => {
            match binding
                .site
                .value
                .and_then(|value| value.find(root.clone()))
            {
                Some(value) => return follow(table, path, &value, receiver, tail, depth + 1),
                None => Found::Open,
            }
        }
        _ => Found::Open,
    })
}

/// Whether a read in scope `from` of a binding of scope `to` lies in a function or lambda nested
/// in `to`, which runs after `to`'s body (Ruff's deferred reading).
fn crosses_function(table: &ScopeTable, from: u32, to: u32) -> bool {
    let mut current = Some(from);
    while let Some(index) = current.filter(|index| *index != to) {
        let scope = &table.scopes[index as usize];
        if matches!(scope.kind, ScopeKind::Function | ScopeKind::Lambda) {
            return true;
        }
        current = scope.parent;
    }
    false
}

/// The bindings of a name that may be its last once the scope's body has run: those from the
/// last that runs whenever the body does (`unconditional`). A bare annotation binds nothing; a
/// binding made from a function body (`global`, `nonlocal`) may run at any time.
fn last_bindings<D: Doc>(root: &Node<'_, D>, bindings: &[Binding]) -> Vec<usize> {
    let mut last = Vec::new();
    if let [only] = bindings {
        if !only.is_annotation() {
            last.push(0);
        }
        return last;
    }
    for (at, binding) in bindings.iter().enumerate() {
        if binding.is_annotation() {
            continue;
        }
        if !binding.deferred && !binding.outer && unconditional(root, binding) {
            last.clear();
        }
        last.push(at);
    }
    last
}

/// Whether `binding`'s statement runs whenever its scope's body runs to its end: no branch, loop,
/// `try`, `with` or `match` lies between it and the scope.
fn unconditional<D: Doc>(root: &Node<'_, D>, binding: &Binding) -> bool {
    let Some(site) = binding.site.node.and_then(|site| site.find(root.clone())) else {
        return false;
    };
    for ancestor in site.ancestors() {
        let kind = ancestor.kind();
        if matches!(
            kind.as_ref(),
            "module" | "function_definition" | "class_definition" | "lambda"
        ) {
            return true;
        }
        if CONDITIONAL.contains(&kind.as_ref()) {
            return false;
        }
    }
    true
}

/// Whether `node` lies in a loop of the function, class or module whose body holds it.
fn in_loop<D: Doc>(node: &Node<'_, D>) -> bool {
    for ancestor in node.ancestors() {
        match ancestor.kind().as_ref() {
            "for_statement" | "while_statement" => return true,
            "function_definition" | "class_definition" | "lambda" | "module" => return false,
            _ => {}
        }
    }
    false
}

/// The node of `kind` spanning exactly `range`.
fn node_at<'r, D: Doc>(
    root: &Node<'r, D>,
    range: std::ops::Range<usize>,
    kind: &str,
) -> Option<Node<'r, D>> {
    let mut node = root.clone();
    loop {
        if node.range() == range && node.kind() == kind {
            return Some(node);
        }
        let inner = node.children().find(|child| {
            let span = child.range();
            span.start <= range.start && range.end <= span.end
        })?;
        node = inner;
    }
}

/// Whether `function` returns its first parameter, unrebound, on every path: its body ends in a
/// `return`, every `return` returns the parameter, and it neither yields nor binds the name.
fn returns_its_argument<'r, D: Doc>(function: &Node<'r, D>) -> bool {
    let (Some(parameter), Some(body)) = (first_parameter(function), function.field("body")) else {
        return false;
    };
    let named = |node: &Node<'r, D>| -> Vec<Node<'r, D>> {
        node.children()
            .filter(|child| child.is_named() && child.kind() != "comment")
            .collect::<Vec<_>>()
    };
    if named(&body)
        .last()
        .is_none_or(|last| last.kind() != "return_statement")
    {
        return false;
    }
    let is_parameter = |node: Option<Node<'r, D>>| {
        node.is_some_and(|node| {
            node.kind() == "identifier" && python_identifier(&node) == parameter.as_str()
        })
    };
    let mut stack = vec![body];
    while let Some(node) = stack.pop() {
        match node.kind().as_ref() {
            "function_definition" | "class_definition" | "lambda" => {}
            "yield" | "global_statement" | "nonlocal_statement" | "delete_statement" => {
                return false;
            }
            "return_statement" => match named(&node).as_slice() {
                [value] if is_parameter(Some(value.clone())) => {}
                _ => return false,
            },
            "assignment" | "augmented_assignment" | "for_statement"
                if is_parameter(node.field("left")) =>
            {
                return false;
            }
            "named_expression" if is_parameter(node.field("name")) => return false,
            _ => stack.extend(node.children()),
        }
    }
    true
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

/// A method's receiver parameter, where the method is not a `@staticmethod`, and whether it is
/// the class: a `@classmethod`'s, or `__new__`'s, `__init_subclass__`'s and
/// `__class_getitem__`'s, which Python passes the class.
fn receiver_parameter<D: Doc>(function: &Node<'_, D>) -> Option<(String, bool)> {
    let decorators = function
        .parent()
        .filter(|parent| parent.kind() == "decorated_definition")
        .map(|parent| {
            parent
                .children()
                .filter(|child| child.kind() == "decorator")
                .map(|child| {
                    python_name(child.text())
                        .trim_start_matches('@')
                        .trim()
                        .to_owned()
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if decorators
        .iter()
        .any(|decorator| decorator == "staticmethod")
    {
        return None;
    }
    let class_bound = decorators
        .iter()
        .any(|decorator| decorator == "classmethod")
        || function.field("name").is_some_and(|name| {
            matches!(
                python_identifier(&name).as_ref(),
                "__new__" | "__init_subclass__" | "__class_getitem__"
            )
        });
    Some((first_parameter(function)?, class_bound))
}

/// A function's first parameter, where it is a plain or typed name.
fn first_parameter<D: Doc>(function: &Node<'_, D>) -> Option<String> {
    let parameters = function.field("parameters")?;
    let first = parameters.children().find(|child| child.is_named())?;
    let name = match first.kind().as_ref() {
        "identifier" => first,
        "typed_parameter" => first.children().find(|c| c.kind() == "identifier")?,
        "default_parameter" | "typed_default_parameter" => first.field("name")?,
        _ => return None,
    };
    Some(python_identifier(&name).into_owned())
}
