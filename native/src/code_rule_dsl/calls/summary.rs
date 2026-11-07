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
use crate::code_rule_dsl::language::Family;
use crate::code_rule_dsl::module_path::ModulePath;
use crate::code_rule_dsl::scope::options::{DEFAULT_PYTHON_MINOR, PythonOptions};
use crate::code_rule_dsl::scope::{Binding, Form, Import, ScopeKind, ScopeTable};
use ast_grep_core::{Doc, Node};
use foldhash::{HashMap, HashMapExt};

/// Bumped whenever extraction changes what a summary holds for the same file.
pub(crate) const EXTRACTOR_VERSION: u32 = 18;

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
    /// Resolved value references, including callbacks and returned function values.
    pub(crate) references: Vec<(u32, u32)>,
    pub(crate) reference_targets: Vec<u32>,
    /// Imported declarations, without run-time assignment aliases.
    pub(crate) reference_aliases: Vec<(u32, u32)>,
    /// Decorated methods and their syntactic decorator names.
    pub(crate) reference_decorators: Vec<(u32, u32)>,
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
    pub(crate) fn as_references(&self, links: super::contracts::Key) -> Self {
        let mut summary = self.clone();
        summary.edges = self.references.clone();
        summary.targets = self.reference_targets.clone();
        summary.aliases.clear();
        summary.supers.clear();
        summary.stars.clear();
        let latest = self
            .defs
            .iter()
            .enumerate()
            .map(|(index, def)| (def.name, index))
            .collect::<HashMap<_, _>>();
        let class_starts = self
            .defs
            .iter()
            .filter(|def| def.flags & CLASS != 0)
            .map(|def| (self.name(def.name), def.start))
            .collect::<HashMap<_, _>>();
        let mut names = self
            .names
            .iter()
            .cloned()
            .enumerate()
            .map(|(index, name)| (name, index as u32))
            .collect::<HashMap<_, _>>();
        let family = self.names.first().map_or(Family::Python, Name::family);
        let mut intern = |text: &str| {
            let name = Name::new(family, text);
            *names.entry(name.clone()).or_insert_with(|| {
                let id = summary.names.len() as u32;
                summary.names.push(name);
                id
            })
        };
        for &(alias, target) in &self.reference_aliases {
            let name = self.name(alias);
            if let Some((class, rest)) = name.split_once(".<field-declaration>.") {
                if let Some((start, field)) = rest.split_once(':')
                    && start.parse::<u32>().ok().as_ref() == class_starts.get(class)
                {
                    summary
                        .aliases
                        .push((intern(&format!("{class}.{field}")), target));
                }
            } else if let Some((class, rest)) = name.split_once(".<base-slot>.") {
                if let Some((start, _)) = rest.split_once(':')
                    && start.parse::<u32>().ok().as_ref() == class_starts.get(class)
                {
                    summary.supers.push((intern(class), target, BASE));
                }
            } else {
                summary.aliases.push((alias, target));
            }
        }
        let contracts = super::contracts::get(links);
        for group in self
            .reference_decorators
            .chunk_by(|left, right| left.0 == right.0)
        {
            let owner = group[0].0;
            let def = &self.defs[owner as usize];
            let decorators = group
                .iter()
                .map(|(_, name)| self.name(*name))
                .collect::<Vec<_>>();
            if let Some(target) = contracts.sibling(self.name(def.name), &decorators) {
                let id = intern(&target);
                summary.edges.push((owner, id));
                summary.targets.push(id);
            }
        }
        summary.edges.sort_unstable();
        summary.edges.dedup();
        summary.targets.sort_unstable();
        summary.targets.dedup();
        summary.open_sites = false;
        for (index, def) in summary.defs.iter_mut().enumerate() {
            def.flags &= !OPEN;
            if latest.get(&def.name) == Some(&index) {
                def.flags &= !SHADOWED;
            }
        }
        summary
    }
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
        let mut summary = extractor.finish(Family::Python);
        summary.open |= has_errors || table.is_none();
        summary
    }

    /// A stable serialization for the content store.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(
            62 + self
                .names
                .iter()
                .map(|name| name.text().len() + 4)
                .sum::<usize>()
                + 13 * self.defs.len()
                + 8 * (self.edges.len()
                    + self.references.len()
                    + self.aliases.len()
                    + self.reference_aliases.len()
                    + self.reference_decorators.len())
                + 4 * (self.targets.len()
                    + self.reference_targets.len()
                    + self.opaque.len()
                    + self.stars.len())
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
        for pairs in [
            &self.edges,
            &self.references,
            &self.aliases,
            &self.reference_aliases,
            &self.reference_decorators,
        ] {
            put(&mut out, pairs.len() as u32);
            for (a, b) in pairs {
                put(&mut out, *a);
                put(&mut out, *b);
            }
        }
        for ids in [
            &self.targets,
            &self.reference_targets,
            &self.opaque,
            &self.stars,
        ] {
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
    pub(crate) fn decode(bytes: &[u8], family: Family) -> Option<Self> {
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
                Some(Name::new(family, text))
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
        let references = pairs(&|def| (def as usize) < defs.len())?;
        let aliases = pairs(&known)?;
        let reference_aliases = pairs(&known)?;
        let reference_decorators = pairs(&|def| (def as usize) < defs.len())?;
        let mut ids = || {
            (0..reader.count(4)?)
                .map(|_| reader.u32().filter(|id| known(*id)))
                .collect::<Option<Vec<_>>>()
        };
        let targets = ids()?;
        let reference_targets = ids()?;
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
            references,
            reference_targets,
            reference_aliases,
            reference_decorators,
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
    reference_owner: Option<u32>,
    reference_receiver: Option<Receiver>,
    reference_class: Option<String>,
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
    references: Vec<(u32, u32)>,
    reference_targets: Vec<u32>,
    reference_aliases: Vec<(u32, u32)>,
    reference_decorators: Vec<(u32, u32)>,
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
            references: Vec::new(),
            reference_targets: Vec::new(),
            reference_aliases: Vec::new(),
            reference_decorators: Vec::new(),
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

    fn finish(mut self, family: Family) -> Summary {
        debug_assert!(self.defs.is_sorted_by_key(|def| def.start));
        let module = self.module.clone().map(|module| self.intern(&module));
        for list in [
            &mut self.edges,
            &mut self.references,
            &mut self.aliases,
            &mut self.reference_aliases,
            &mut self.reference_decorators,
        ] {
            list.sort_unstable();
            list.dedup();
        }
        for list in [
            &mut self.targets,
            &mut self.reference_targets,
            &mut self.opaque,
            &mut self.stars,
        ] {
            list.sort_unstable();
            list.dedup();
        }
        Summary {
            module,
            names: self
                .names
                .iter()
                .map(|name| Name::new(family, name))
                .collect(),
            defs: self.defs,
            edges: self.edges,
            references: self.references,
            reference_targets: self.reference_targets,
            reference_aliases: self.reference_aliases,
            reference_decorators: self.reference_decorators,
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
            reference_owner: None,
            reference_receiver: None,
            reference_class: None,
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
        let mut local_references = Vec::new();
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
            if let Some(table) = self.table {
                let head_of_attribute = node.parent().is_some_and(|parent| {
                    parent.kind() == "attribute"
                        && parent
                            .field("object")
                            .is_some_and(|object| object.range() == node.range())
                });
                let call_target = node.parent().is_some_and(|parent| {
                    parent.kind() == "call"
                        && parent
                            .field("function")
                            .is_some_and(|function| function.range() == node.range())
                });
                let parameter_value = kind == "identifier"
                    && table.reference(&node.range()).is_some_and(|reference| {
                        table
                            .resolve(reference, &python_identifier(&node))
                            .is_some_and(|binding| {
                                binding.site.form == Form::Parameter
                                    && parameter_owner(binding, &node).is_some_and(|start| {
                                        contexts[context].reference_owner.is_some_and(|owner| {
                                            self.defs[owner as usize].start == start
                                        })
                                    })
                            })
                    });
                let expression = if kind == "call" {
                    node.field("function")
                } else if (kind == "attribute"
                    && !call_target
                    && node
                        .field("object")
                        .is_some_and(|object| object.kind() == "identifier"))
                    || (kind == "identifier"
                        && !head_of_attribute
                        && !call_target
                        && !parameter_value
                        && table.reference(&node.range()).is_some_and(|reference| {
                            matches!(reference.access, crate::code_rule_dsl::scope::Access::Load)
                        }))
                {
                    Some(node.clone())
                } else {
                    None
                };
                if let Some(expression) = expression {
                    let owner = contexts[context].reference_owner;
                    let local_value = kind != "call"
                        && expression.kind() == "identifier"
                        && table
                            .reference(&expression.range())
                            .is_some_and(|reference| {
                                let name = python_identifier(&expression);
                                table.resolve_python_index(reference, &name).is_some_and(
                                    |(scope, index)| {
                                        let binding = &table.scopes[scope as usize].bindings
                                            [name.as_ref()][index];
                                        (scope != 0 || binding.deferred || binding.outer)
                                            && !matches!(binding.site.form, Form::Definition)
                                            && !(binding.site.form == Form::Parameter
                                                && parameter_owner(binding, &expression)
                                                    .is_some_and(|start| {
                                                        owner.is_some_and(|owner| {
                                                            self.defs[owner as usize].start != start
                                                        })
                                                    }))
                                    },
                                )
                            });
                    let builtin_value = kind != "call"
                        && expression.kind() == "identifier"
                        && PythonOptions::new(DEFAULT_PYTHON_MINOR)
                            .declares(&python_identifier(&expression));
                    let unqualified =
                        (expression.kind() == "identifier" && !local_value && !builtin_value)
                            .then(|| {
                                let short = python_identifier(&expression);
                                let bare = self
                                    .module
                                    .as_ref()
                                    .map(|module| format!("{module}.<bare>.{short}=>"))
                                    .unwrap_or_default();
                                contexts[context]
                                    .reference_class
                                    .as_ref()
                                    .map(|class| format!("{class}.<unqualified>.{short}=>{bare}"))
                                    .unwrap_or(bare)
                            })
                            .filter(|prefix| !prefix.is_empty());
                    if !local_value && !builtin_value {
                        match follow_reference(
                            table,
                            self.path,
                            &expression,
                            contexts[context].reference_receiver.as_ref(),
                            contexts[context].reference_class.as_deref(),
                            owner.map(|owner| self.defs[owner as usize].start),
                        ) {
                            Some(Found::Name(name)) => {
                                let name = unqualified.as_ref().map_or_else(
                                    || name.clone(),
                                    |prefix| format!("{prefix}{name}"),
                                );
                                self.reference_named(owner, &name);
                            }
                            Some(Found::Local(site, tail)) => {
                                local_references.push((owner, site, tail, unqualified))
                            }
                            _ => {
                                if let Some(name) = unqualified {
                                    self.reference_named(owner, &name);
                                }
                            }
                        }
                    }
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
        for (owner, site, tail, unqualified) in local_references {
            if let Some(def) = self.def_index(site) {
                let name = self.def_name(def, &tail);
                let name = unqualified
                    .as_ref()
                    .map_or_else(|| name.clone(), |prefix| format!("{prefix}{name}"));
                self.reference_named(owner, &name);
            }
        }
    }

    fn reference_named(&mut self, owner: Option<u32>, name: &str) {
        let name = self.intern(name);
        self.reference_targets.push(name);
        if let Some(owner) = owner {
            self.references.push((owner, name));
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
        if !class
            && outer.class.is_some()
            && let Some(decorated) = node
                .parent()
                .filter(|parent| parent.kind() == "decorated_definition")
        {
            for decorator in decorated
                .children()
                .filter(|child| child.kind() == "decorator")
            {
                let Some(expression) = decorator
                    .children()
                    .find(|child| child.is_named() && child.kind() != "comment")
                else {
                    continue;
                };
                let mut expression = unparenthesized(&expression);
                if expression.kind() == "call" {
                    let Some(function) = expression.field("function") else {
                        continue;
                    };
                    expression = unparenthesized(&function);
                }
                let terminal = if expression.kind() == "attribute" {
                    expression.field("attribute")
                } else {
                    Some(expression)
                };
                if let Some(terminal) = terminal.filter(|node| node.kind() == "identifier") {
                    let name = self.intern(&python_identifier(&terminal));
                    self.reference_decorators.push((def, name));
                }
            }
        }
        if class {
            self.bases(node, name, outer);
        }
        Some(if class {
            Context {
                owner: outer.owner,
                reference_owner: outer.reference_owner,
                reference_receiver: outer.reference_receiver.clone(),
                reference_class: outer.reference_class.clone(),
                prefix: format!("{qualified}."),
                class: Some(qualified),
                receiver: None,
            }
        } else {
            Context {
                owner: Some(def),
                reference_owner: outer.reference_owner.or(Some(def)),
                reference_class: if outer.reference_owner.is_some() {
                    outer.reference_class.clone()
                } else {
                    outer.class.clone()
                },
                reference_receiver: if outer.reference_owner.is_some() {
                    outer.reference_receiver.clone()
                } else {
                    outer.class.as_ref().and_then(|class| {
                        receiver_parameter(node).map(|(parameter, class_bound)| Receiver {
                            parameter,
                            class: class.clone(),
                            class_bound,
                        })
                    })
                },
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
            if kind == BASE {
                let short = head.text();
                let short = short
                    .split('[')
                    .next()
                    .unwrap_or(&short)
                    .rsplit('.')
                    .next()
                    .unwrap_or("")
                    .trim();
                let alias = format!(
                    "{}.<base-slot>.{}:{:010}",
                    self.names[key as usize],
                    class.range().start,
                    self.reference_aliases.len()
                );
                let target = format!("{}.<base>.{short}", self.module.as_deref().unwrap_or(""));
                let pair = (self.intern(&alias), self.intern(&target));
                self.reference_aliases.push(pair);
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
        let mut reference_aliases = Vec::new();
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
                if scope.kind == ScopeKind::Class
                    && let Some(prefix) = &prefix
                    && let Some(annotation) = bindings
                        .iter()
                        .rev()
                        .find_map(|binding| {
                            (matches!(binding.site.form, Form::Annotated | Form::Annotation))
                                .then_some(binding.site.annotation)
                                .flatten()
                                .filter(|site| {
                                    site.find(root.clone()).is_some_and(|annotation| {
                                        annotation
                                            .ancestors()
                                            .find(|node| node.kind() == "expression_statement")
                                            .and_then(|statement| statement.parent())
                                            .is_some_and(|block| {
                                                block.kind() == "block"
                                                    && block.parent().is_some_and(|owner| {
                                                        owner.kind() == "class_definition"
                                                    })
                                            })
                                    })
                                })
                        })
                        .and_then(|annotation| annotation.find(root.clone()))
                {
                    reference_aliases.push((
                        format!("{prefix}.<field-declaration>.{}:{name}", scope.range.start),
                        annotation_reference(&annotation.text()),
                    ));
                }
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
                        Some(Some(target)) => {
                            if matches!(
                                bindings[at].import,
                                Some(Import::Qualified(_) | Import::Relative { .. })
                            ) {
                                reference_aliases
                                    .push((format!("{prefix}.{name}"), target.clone()));
                            }
                            aliases.push((format!("{prefix}.{name}"), target));
                        }
                        Some(None) => opaque.push(format!("{prefix}.{name}")),
                        None => {}
                    }
                }
            }
        }
        // The declared inventory ignores assignments and includes top-level class
        // methods in the module's short-name namespace, preferring functions.
        if let Some(module) = &self.module {
            let mut selected = std::collections::BTreeMap::<String, (bool, String)>::new();
            for def in &self.defs {
                let qualified = self.names[def.name as usize].as_ref();
                if qualified.contains(".<locals>.") {
                    continue;
                }
                let short = qualified.rsplit('.').next().unwrap_or(qualified);
                let function = def.flags & CLASS == 0;
                let entry = selected
                    .entry(short.to_owned())
                    .or_insert((function, qualified.to_owned()));
                if function && !entry.0 {
                    *entry = (true, qualified.to_owned());
                }
            }
            for (short, (_, target)) in selected {
                reference_aliases.push((format!("{module}.<declared>.{short}"), target));
            }
            let mut classes = std::collections::BTreeMap::new();
            for def in &self.defs {
                if def.flags & CLASS != 0 {
                    let qualified = self.names[def.name as usize].as_ref();
                    let short = qualified.rsplit('.').next().unwrap_or(qualified);
                    classes
                        .entry(short.to_owned())
                        .or_insert_with(|| qualified.to_owned());
                }
            }
            for (short, target) in classes {
                reference_aliases.push((format!("{module}.<class>.{short}"), target));
            }
            let mut imports =
                std::collections::BTreeMap::<(String, String), (usize, String)>::new();
            for scope in &table.scopes {
                for (name, bindings) in &scope.bindings {
                    for binding in bindings {
                        if !matches!(
                            binding.import,
                            Some(Import::Qualified(_) | Import::Relative { .. })
                        ) {
                            continue;
                        }
                        let Some(node) = binding.site.node.and_then(|site| site.find(root.clone()))
                        else {
                            continue;
                        };
                        let Some(statement) = node.ancestors().find(|node| {
                            matches!(
                                node.kind().as_ref(),
                                "import_statement" | "import_from_statement"
                            )
                        }) else {
                            continue;
                        };
                        let (kind, target) = if statement.kind() == "import_from_statement" {
                            let Some(Some(target)) = self.member(root, table, binding) else {
                                continue;
                            };
                            ("imported", target)
                        } else {
                            (
                                "module-imported",
                                node.field("name")
                                    .unwrap_or(node.clone())
                                    .text()
                                    .into_owned(),
                            )
                        };
                        let entry = imports
                            .entry((kind.to_owned(), name.to_string()))
                            .or_insert((0, String::new()));
                        if node.range().start >= entry.0 {
                            *entry = (node.range().start, target);
                        }
                    }
                }
            }
            for ((kind, name), (_, target)) in imports {
                reference_aliases.push((format!("{module}.<{kind}>.{name}"), target));
            }
        }
        reference_aliases.extend(self.constructor_parameters(root, table));
        // Interned in a stable order: a summary is the same for the same file.
        aliases.sort();
        for (alias, target) in aliases {
            let pair = (self.intern(&alias), self.intern(&target));
            self.aliases.push(pair);
        }
        reference_aliases.sort();
        for (alias, target) in reference_aliases {
            let pair = (self.intern(&alias), self.intern(&target));
            self.reference_aliases.push(pair);
        }
        opaque.sort();
        for name in opaque {
            let id = self.intern(&name);
            self.opaque.push(id);
        }
    }

    /// Constructor parameter declarations are metadata. A graph opts into a field
    /// naming convention before using them; no assignment inference is implicit.
    fn constructor_parameters<D: Doc>(
        &self,
        root: &Node<'_, D>,
        table: &ScopeTable,
    ) -> Vec<(String, String)> {
        let latest = self
            .defs
            .iter()
            .enumerate()
            .map(|(index, def)| (def.name, index))
            .collect::<HashMap<_, _>>();
        let mut aliases = Vec::new();
        for scope in &table.scopes {
            if scope.kind != ScopeKind::Function {
                continue;
            }
            let Some(index) = self.def_index(scope.range.start as u32) else {
                continue;
            };
            let def = &self.defs[index];
            if latest.get(&def.name) != Some(&index) {
                continue;
            }
            let qualified = self.names[def.name as usize].as_ref();
            let Some(class) = qualified.strip_suffix(".__init__") else {
                continue;
            };
            let mut parent = scope.parent;
            let mut method = false;
            while let Some(index) = parent {
                let parent_scope = &table.scopes[index as usize];
                if parent_scope.kind == ScopeKind::Class {
                    method = true;
                    break;
                }
                if matches!(
                    parent_scope.kind,
                    ScopeKind::Module | ScopeKind::Function | ScopeKind::Lambda
                ) {
                    break;
                }
                parent = parent_scope.parent;
            }
            if !method {
                continue;
            }
            for (name, bindings) in &scope.bindings {
                let Some(binding) = bindings
                    .iter()
                    .find(|binding| binding.site.form == Form::Parameter)
                else {
                    continue;
                };
                let Some(parameter) = binding.site.node.and_then(|site| site.find(root.clone()))
                else {
                    continue;
                };
                let splat = |node: &Node<'_, D>| {
                    matches!(
                        node.kind().as_ref(),
                        "list_splat_pattern" | "dictionary_splat_pattern"
                    ) || node.children().any(|child| {
                        matches!(
                            child.kind().as_ref(),
                            "list_splat_pattern" | "dictionary_splat_pattern"
                        )
                    })
                };
                if splat(&parameter)
                    || parameter.parent().is_some_and(|parameters| {
                        parameters.children().any(|child| {
                            child.kind() == "positional_separator"
                                && child.range().start > parameter.range().end
                        })
                    })
                {
                    continue;
                }
                let keyword = parameter.parent().is_some_and(|parameters| {
                    parameters.children().any(|child| {
                        child.range().end <= parameter.range().start
                            && (child.kind() == "keyword_separator" || splat(&child))
                    })
                });
                if !keyword && matches!(name.as_ref(), "self" | "cls") {
                    continue;
                }
                let target = match binding.site.annotation {
                    None => super::contracts::UNANNOTATED.to_owned(),
                    Some(annotation) => annotation
                        .find(root.clone())
                        .map(|annotation| annotation_reference(&annotation.text()))
                        .unwrap_or_else(|| super::contracts::UNRESOLVED_TYPE.to_owned()),
                };
                aliases.push((super::contracts::parameter_key(class, name), target));
            }
        }
        aliases
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

/// A declared parameter type can name the receiver of a reference. This does not
/// change the conservative, run-time dispatch used by the ordinary call graph.
fn follow_reference<D: Doc>(
    table: &ScopeTable,
    path: Option<&ModulePath>,
    expression: &Node<'_, D>,
    receiver: Option<&Receiver>,
    class: Option<&str>,
    parameter_owner: Option<u32>,
) -> Option<Found> {
    if let (Some(object), Some(attribute)) =
        (expression.field("object"), expression.field("attribute"))
    {
        if object.kind() == "identifier"
            && matches!(python_identifier(&object).as_ref(), "self" | "cls")
        {
            return class.map(|class| {
                Found::Name(format!("{class}.<self>.{}", python_identifier(&attribute)))
            });
        }
        if object.kind() == "attribute"
            && object.field("object").is_some_and(|base| {
                base.kind() == "identifier"
                    && matches!(python_identifier(&base).as_ref(), "self" | "cls")
            })
            && let Some(field) = object.field("attribute")
        {
            return class.map(|class| {
                Found::Name(format!(
                    "{class}.<field>.{}.{}",
                    python_identifier(&field),
                    python_identifier(&attribute)
                ))
            });
        }
        if object.kind() == "call"
            && object.field("function").is_some_and(|function| {
                function.kind() == "identifier" && python_identifier(&function) == "super"
            })
        {
            return class.map(|class| {
                Found::Name(format!("{class}.<super>.{}", python_identifier(&attribute)))
            });
        }
    }
    if let Some(object) = expression.field("object") {
        if object.kind() != "identifier" {
            return None;
        }
        if let Some(reference) = table.reference(&object.range())
            && let Some((scope, _)) =
                table.resolve_python_index(reference, &python_identifier(&object))
            && let Some(binding) = table.scopes[scope as usize]
                .bindings
                .get(python_identifier(&object).as_ref())
                .and_then(|chain| {
                    chain
                        .iter()
                        .find(|binding| binding.site.form == Form::Parameter)
                })
        {
            return declared_parameter_reference(table, path, expression, binding, parameter_owner);
        }
    }
    let mut head = unparenthesized(expression);
    while head.kind() == "attribute" {
        head = unparenthesized(&head.field("object")?);
    }
    if head.kind() == "identifier"
        && let Some(reference) = table.reference(&head.range())
        && let Some(binding) = table.resolve(reference, &python_identifier(&head))
        && binding.import.is_none()
        && matches!(
            binding.site.form,
            Form::Assignment | Form::Annotated | Form::Named
        )
    {
        return None;
    }
    let receiver = receiver.filter(|receiver| {
        expression.kind() != "identifier" && matches!(receiver.parameter.as_str(), "self" | "cls")
    });
    follow(table, path, expression, receiver, Vec::new(), 0)
}

fn declared_parameter_reference<D: Doc>(
    table: &ScopeTable,
    path: Option<&ModulePath>,
    expression: &Node<'_, D>,
    binding: &Binding,
    parameter_owner: Option<u32>,
) -> Option<Found> {
    let attribute = expression.field("attribute")?;
    let parameter = binding.site.node?.find(expression.root().root())?;
    if parameter_owner.is_some_and(|owner| {
        parameter
            .ancestors()
            .find(|node| node.kind() == "function_definition")
            .is_none_or(|node| node.range().start != owner as usize)
    }) {
        return None;
    }
    if parameter.children().any(|child| {
        matches!(
            child.kind().as_ref(),
            "list_splat_pattern" | "dictionary_splat_pattern"
        )
    }) || parameter.parent().is_some_and(|parameters| {
        parameters.children().any(|child| {
            child.kind() == "positional_separator" && child.range().start > parameter.range().end
        })
    }) {
        return None;
    }
    let annotation = binding.site.annotation?.find(expression.root().root())?;
    if let Some(module) = path.and_then(ModulePath::import_name) {
        let annotation = annotation_reference(&annotation.text());
        let name = annotation.strip_prefix("<annotation>.")?;
        return Some(Found::Name(format!(
            "{module}.<type>.{name}.{}",
            python_identifier(&attribute)
        )));
    }
    let ty = declared_type(table, path, &expression.root().root(), &annotation)?;
    Some(match ty {
        Found::Name(name) => Found::Name(qualify(
            &name,
            &[python_identifier(&attribute).into_owned()],
        )),
        Found::Local(site, mut tail) => {
            tail.push(python_identifier(&attribute).into_owned());
            Found::Local(site, tail)
        }
        Found::Open => Found::Open,
    })
}

fn parameter_owner<D: Doc>(binding: &Binding, node: &Node<'_, D>) -> Option<u32> {
    binding
        .site
        .node?
        .find(node.root().root())?
        .ancestors()
        .find(|node| node.kind() == "function_definition")
        .map(|node| node.range().start as u32)
}

fn annotation_reference(text: &str) -> String {
    let name = text.split(['[', '|']).next().unwrap_or("").trim();
    if name.contains('.') {
        return super::contracts::UNRESOLVED_TYPE.to_owned();
    }
    format!("<annotation>.{name}")
}

fn declared_type<D: Doc>(
    table: &ScopeTable,
    path: Option<&ModulePath>,
    root: &Node<'_, D>,
    annotation: &Node<'_, D>,
) -> Option<Found> {
    let text = annotation.text();
    let name = text.split(['[', '|']).next()?.trim();
    // A forward string or qualified expression does not declare a module binding.
    let chain = table.scopes[0].bindings.get(name)?;
    let ty = chain.last()?;
    if !matches!(
        ty.import,
        Some(Import::Definition | Import::Qualified(_) | Import::Relative { .. })
    ) {
        return None;
    }
    bound(table, path, root, ty, Vec::new(), None, 0)
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
