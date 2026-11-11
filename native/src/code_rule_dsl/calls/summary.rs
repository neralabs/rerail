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

use crate::code_rule_dsl::tree::Tree;
use super::names::Name;
use crate::code_rule_dsl::language::Family;
use crate::code_rule_dsl::module_path::ModulePath;
use crate::code_rule_dsl::scope::{Binding, Form, Import, ScopeKind, ScopeTable};
use ast_grep_core::{Doc, Node};
use foldhash::{HashMap, HashMapExt};

/// Bumped whenever extraction changes what a summary holds for the same file.
pub(crate) const EXTRACTOR_VERSION: u32 = 18;

pub const ASYNC: u8 = 1;
pub const CLASS: u8 = 2;
/// The definition's body calls something unknown.
pub const OPEN: u8 = 4;
/// Python: a decorator that is not known to return the definition itself binds its name.
pub const DECORATED: u8 = 32;
/// Python: a later binding of the name that runs whenever its scope's body runs to its end
/// replaces the definition.
pub const SHADOWED: u8 = 64;

/// Python `supers` kinds, `(class, target, kind)`: a base, in order (`php::PARENT`'s value: the
/// graph indexes subclasses alike); a base or class keyword no static name denotes (`(class,
/// class)`); the metaclass.
pub const BASE: u8 = 0;
pub const UNKNOWN_BASE: u8 = 6;
pub const METACLASS: u8 = 7;

/// Decorators that return the definition they decorate (`functools.wraps(f)` when called).
pub const TRANSPARENT: [&str; 6] = [
    "abc.abstractmethod",
    "typing.final",
    "typing.override",
    "typing.no_type_check",
    "typing_extensions.final",
    "typing_extensions.override",
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// The name others import the module by (`pkg.mod`; `pkg` for `pkg/__init__.py`).
    pub module: Option<u32>,
    /// Names used by this summary, each once.
    pub names: Vec<Name>,
    /// Functions and classes, by node start.
    pub defs: Vec<Def>,
    /// `(definition, callee name)`: what each definition's own body calls. Sorted, distinct.
    pub edges: Vec<(u32, u32)>,
    /// Resolved value references, including callbacks and returned function values.
    pub references: Vec<(u32, u32)>,
    pub reference_targets: Vec<u32>,
    /// Imported declarations, without run-time assignment aliases.
    pub reference_aliases: Vec<(u32, u32)>,
    /// Decorated methods and their syntactic decorator names.
    pub reference_decorators: Vec<(u32, u32)>,
    /// Every callee name any call of the file names, in a definition or not. Sorted, distinct.
    pub targets: Vec<u32>,
    /// Module- and class-level names other code may call through: `(this module's name for it,
    /// the name it denotes)`, from imports and from assignments of a name.
    pub aliases: Vec<(u32, u32)>,
    /// Module- and class-level names bound, on some path that may be the last, to a value no name
    /// denotes (a call's result, a lambda, a loop target): a call through one is unknown. Sorted,
    /// distinct.
    pub opaque: Vec<u32>,
    /// Modules `from m import *` reads at module level.
    pub stars: Vec<u32>,
    /// PHP: what each class-like extends, implements and uses, and its trait adaptations:
    /// `(class or class.method, target, kind)` (`php::PARENT` and the other kinds), in
    /// declaration order. Python: each class's bases and metaclass (`BASE` and the other kinds).
    pub supers: Vec<(u32, u32, u8)>,
    /// The file has syntax errors, or its scope table could not be built: what its definitions
    /// call is not all known.
    pub open: bool,
    /// A call of the file names something unknown: its `calls` verdict is unknown there.
    pub open_sites: bool,
    /// The source's length, which a code rule's `maxSourceBytes` reads.
    pub bytes: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Def {
    /// The qualified name (`pkg.mod.Class.method`, `pkg.mod.f.<locals>.g`).
    pub name: u32,
    /// The `function_definition` or `class_definition` node's start.
    pub start: u32,
    /// And its end.
    pub end: u32,
    pub flags: u8,
}

impl Def {
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn is_async(&self) -> bool {
        self.flags & ASYNC != 0
    }
}

/// What a call's callee denotes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Callee {
    Name(String),
    /// Unknown: the name is unbound, or a relative import outside a package.
    Open,
}

impl Summary {
    pub fn as_references(&self, links: super::contracts::Key) -> Self {
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
    pub fn name(&self, id: u32) -> &str {
        self.names[id as usize].text()
    }

    /// Name `id` as interned: what the call graph keys by.
    pub fn interned(&self, id: u32) -> &Name {
        &self.names[id as usize]
    }

    /// The functions (not classes) whose node holds `at`, innermost first.
    pub fn functions_around(&self, at: u32) -> impl Iterator<Item = &Def> {
        let before = self.defs.partition_point(|def| def.start <= at);
        self.defs[..before]
            .iter()
            .rev()
            .filter(move |def| at < def.end && def.flags & CLASS == 0)
    }

    /// The definition whose node starts at `start`.
    pub fn def_at(&self, start: u32) -> Option<&Def> {
        let index = self.defs.partition_point(|def| def.start < start);
        self.defs.get(index).filter(|def| def.start == start)
    }

    /// A stable serialization for the content store.
    pub fn encode(&self) -> Vec<u8> {
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
    pub fn decode(bytes: &[u8], family: Family) -> Option<Self> {
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

// Python's extraction of a file's summary (`python`, the Python crate's).
// cratify: `python` is rerail-lang-python's (it mounts it)
