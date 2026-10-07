//! PHP's compile-time name resolution for classes, functions and constants, as nikic/PHP-Parser's
//! `NameResolver` (which PHPStan runs) performs it for PHP 8.5.
//!
//! A reference is a `name`, `qualified_name` or `relative_name` in a class position (`new X`,
//! `X::m()`, `X::C`, `X::$p`, `instanceof X`, `extends` and `implements` lists, types, `catch`
//! types, attributes, trait `use` and its adaptations), a function position (a call's callee) or a
//! constant position (any other expression; `true`, `false` and `null` too). Declared names,
//! member names, namespace names and the names inside `use` statements are not references.
//!
//! Fully qualified names are themselves; `namespace\X` is the current namespace's `X`; a qualified
//! name resolves its first segment through the class aliases; an unqualified class name resolves
//! through the class aliases or the current namespace (`self`, `static` and `parent` are the
//! enclosing class's, never resolved). An unqualified function or constant resolves through its own
//! aliases (constants case-sensitively; `true`, `false` and `null` are always global); otherwise it
//! is the global name in the global namespace, and in a namespace PHP tries the namespaced name
//! first and falls back to the global one at run time. That fallback is read as the namespaced
//! name where this file declares it (a function, a `const`, or the global `define` of a constant
//! expression), as the global name where PHP 8.5.11 defines it (`data/php_builtins.json`), and as
//! unknown otherwise: another file may declare it.

use super::{BuildError, NodeRef};
use crate::policy_dsl::value::php_compare::{ImportKind, import_kind};
use crate::policy_dsl::value::{Context, Value, php as php_value};
use ast_grep_core::{Doc, Node};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::LazyLock;

/// Nodes between two reads of the evaluation deadline.
const DEADLINE_INTERVAL: usize = 1_024;

/// The node kinds a reference has: names, and the `true`, `false` and `null` literals, which
/// PHP-Parser reads as constant fetches.
pub(crate) const REFERENCE_KINDS: &[&str] =
    &["name", "qualified_name", "relative_name", "boolean", "null"];

/// Unqualified type names PHP reads as builtin types, not classes (PHP-Parser's
/// `handleBuiltinTypes`, and the `array` and `callable` keywords).
const BUILTIN_TYPES: &[&str] = &[
    "array", "bool", "callable", "false", "float", "int", "iterable", "mixed", "never", "null",
    "object", "string", "true", "void",
];

/// What a reference names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NameKind {
    Class,
    Function,
    Constant,
}

/// What a reference resolves to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Resolved {
    /// A fully qualified name, without the leading backslash, spelled as the source and the `use`
    /// statement that aliased it spell it.
    Name(Box<str>),
    /// `self`, `static` or `parent`: the enclosing class, which name resolution does not name.
    Special,
    /// An unqualified function or constant in a namespace that neither this file declares nor PHP
    /// defines: at run time the namespaced name if another file declares it, else the global one.
    Either {
        namespaced: Box<str>,
        global: Box<str>,
    },
    /// Not known: a name PHP would reject, or a name in an error region whose position is not
    /// known.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Reference {
    /// `None` for a name whose position an error region hides.
    pub(crate) kind: Option<NameKind>,
    pub(crate) resolved: Resolved,
}

/// The name references of one PHP file, by byte range, and the constants and classes it declares.
#[derive(Debug, Default)]
pub(crate) struct PhpNames {
    references: HashMap<(usize, usize), Reference>,
    /// The names `use` statements import, by the byte range of the name a clause writes (its
    /// alias aside): what each imports, fully qualified without the leading backslash.
    imports: HashMap<(usize, usize), (NameKind, Box<str>)>,
    /// The value of each constant this file declares, by `constant_key`: a `const`, or a `define`
    /// statement at the top level. `None` for one declared more than once, or by a `define`
    /// elsewhere, which may run or not.
    constants: Declared,
    /// Each class, interface, enum and trait this file declares, by lowercased name; `None` for
    /// one declared more than once.
    classes: Declared,
    /// Nodes visited to build it, charged to a check's `maxVisitedNodes`.
    pub(crate) cost: usize,
}

/// Declarations by name: the node of the one declaration, or `None` for a name declared more than
/// once.
type Declared = HashMap<String, Option<NodeRef>>;

impl PhpNames {
    /// Reads the references of `root`'s file, visiting at most `cap` nodes.
    pub(crate) fn build<D: Doc>(
        root: Node<'_, D>,
        cap: usize,
        deadline: &dyn Fn() -> bool,
    ) -> Result<Self, BuildError> {
        let mut builder = Builder::new();
        let mut stack = vec![root];
        let mut cost = 0usize;
        while let Some(node) = stack.pop() {
            cost += 1;
            if cost > cap {
                return Err(BuildError::Budget);
            }
            if cost.is_multiple_of(DEADLINE_INTERVAL) && deadline() {
                return Err(BuildError::Deadline);
            }
            let descend = builder.visit(&node);
            if descend {
                let start = stack.len();
                stack.extend(node.children().filter(|child| child.is_named()));
                stack[start..].reverse();
            }
        }
        let imports = std::mem::take(&mut builder.imports);
        let (references, constants, classes) = builder.finish();
        Ok(Self {
            references,
            imports,
            constants,
            classes,
            cost,
        })
    }

    /// The reference at `range`, if a name there is one.
    pub(crate) fn reference(&self, range: &Range<usize>) -> Option<&Reference> {
        self.references.get(&(range.start, range.end))
    }

    /// What the `use` clause name at `range` imports, if a name there is one.
    pub(crate) fn import(&self, range: &Range<usize>) -> Option<(NameKind, &str)> {
        self.imports
            .get(&(range.start, range.end))
            .map(|(kind, name)| (*kind, &**name))
    }

    #[cfg(test)]
    pub(crate) fn imports(&self) -> impl Iterator<Item = (Range<usize>, NameKind, &str)> {
        self.imports
            .iter()
            .map(|(&(start, end), (kind, name))| (start..end, *kind, &**name))
    }

    /// The value node of the constant `node` fetches, where this file declares it once: a
    /// constant name a `const` or a top-level `define` declares, or a class constant (`C::X`,
    /// `self::X`, `parent::X`, and `static::X` where the class or the constant is final) a class,
    /// interface or enum this file declares holds or inherits from those this file declares.
    /// `None` where another declaration may win: an ancestor or a trait this file does not
    /// declare, a name PHP resolves at run time (`Resolved::Either`), a non-public constant read
    /// from where PHP may refuse it.
    pub(crate) fn constant_value<D: Doc>(&self, node: &Node<'_, D>) -> Option<NodeRef> {
        if node.kind() == "class_constant_access_expression" {
            return self.class_constant(node);
        }
        let reference = self.reference(&node.range())?;
        match (reference.kind, &reference.resolved) {
            (Some(NameKind::Constant), Resolved::Name(name)) => {
                *self.constants.get(&constant_key(name))?
            }
            _ => None,
        }
    }

    fn class_constant<D: Doc>(&self, access: &Node<'_, D>) -> Option<NodeRef> {
        let mut parts = access
            .children()
            .filter(|child| child.is_named() && !child.kind().ends_with("comment"));
        let (Some(scope), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
            return None;
        };
        // `C::class` is the class's name; `C::{expr}` fetches a constant named at run time.
        if name.kind() != "name"
            || name.children().any(|child| child.is_named())
            || name.text().eq_ignore_ascii_case("class")
        {
            return None;
        }
        let enclosing = enclosing_class(access);
        let (class, late, relative) = match scope.kind().as_ref() {
            "relative_scope" => {
                let enclosing = enclosing.clone()?;
                match scope.text().to_ascii_lowercase().as_str() {
                    "self" => (enclosing, false, true),
                    // Late static binding: a subclass may redeclare the constant unless the
                    // class or the constant is final.
                    "static" => {
                        let late = !is_final(&enclosing);
                        (enclosing, late, true)
                    }
                    "parent" => (self.parent_class(&enclosing)?, false, true),
                    _ => return None,
                }
            }
            "name" | "qualified_name" | "relative_name" => {
                (self.class_named(&scope)?, false, false)
            }
            _ => return None,
        };
        let Lookup::Found(found) = self.lookup(&class, &name.text(), MAX_ANCESTORS) else {
            return None;
        };
        if late && !found.is_final {
            return None;
        }
        // PHP refuses a private constant outside its class and a protected one outside its
        // class's hierarchy, which the relative scopes stay in.
        let inside_owner =
            enclosing.is_some_and(|enclosing| NodeRef::of(&enclosing) == found.owner);
        let readable = match found.visibility {
            Visibility::Public => true,
            Visibility::Protected => relative || inside_owner,
            Visibility::Private => inside_owner && found.owner == NodeRef::of(&class),
        };
        readable.then_some(found.value)
    }

    /// The class-like a class name in this file names.
    fn class_named<'r, D: Doc>(&self, name: &Node<'r, D>) -> Option<Node<'r, D>> {
        let reference = self.reference(&name.range())?;
        let (Some(NameKind::Class), Resolved::Name(resolved)) =
            (reference.kind, &reference.resolved)
        else {
            return None;
        };
        let declared = (*self.classes.get(&resolved.to_ascii_lowercase())?)?;
        declared.find(name.root().root())
    }

    /// The class a class extends, where this file declares it.
    fn parent_class<'r, D: Doc>(&self, class: &Node<'r, D>) -> Option<Node<'r, D>> {
        if !matches!(
            class.kind().as_ref(),
            "class_declaration" | "anonymous_class"
        ) {
            return None;
        }
        let base = class
            .children()
            .find(|child| child.kind() == "base_clause")?;
        self.class_named(&first_named(&base)?)
    }

    /// The constant `name` of `class`: its own declaration, else the one the traits it uses, its
    /// parent class or its interfaces give it, `depth` ancestors deep at most.
    fn lookup<D: Doc>(&self, class: &Node<'_, D>, name: &str, depth: usize) -> Lookup {
        if depth == 0 {
            return Lookup::Unknown;
        }
        let Some(body) = class.field("body") else {
            return Lookup::Unknown;
        };
        let mut ancestors = Vec::new();
        for member in body.children() {
            match member.kind().as_ref() {
                "const_declaration" => {
                    for element in member
                        .children()
                        .filter(|child| child.kind() == "const_element")
                    {
                        if first_named(&element).is_none_or(|own| own.text() != name) {
                            continue;
                        }
                        let Some(value) = const_value(&element) else {
                            return Lookup::Unknown;
                        };
                        let visibility = member
                            .children()
                            .find(|child| child.kind() == "visibility_modifier")
                            .map_or(Visibility::Public, |modifier| {
                                match modifier.text().to_ascii_lowercase().as_str() {
                                    "private" => Visibility::Private,
                                    "protected" => Visibility::Protected,
                                    _ => Visibility::Public,
                                }
                            });
                        return Lookup::Found(Found {
                            value: NodeRef::of(&value),
                            owner: NodeRef::of(class),
                            is_final: member
                                .children()
                                .any(|child| child.kind() == "final_modifier"),
                            visibility,
                        });
                    }
                }
                // The traits a class uses compose their constants into it.
                "use_declaration" => ancestors.extend(
                    member
                        .children()
                        .filter(|child| is_name_kind(&child.kind())),
                ),
                _ => {}
            }
        }
        for clause in class.children().filter(|child| {
            matches!(
                child.kind().as_ref(),
                "base_clause" | "class_interface_clause"
            )
        }) {
            ancestors.extend(
                clause
                    .children()
                    .filter(|child| is_name_kind(&child.kind())),
            );
        }
        let mut found = Lookup::Absent;
        for ancestor in ancestors {
            let Some(ancestor) = self.class_named(&ancestor) else {
                return Lookup::Unknown;
            };
            match self.lookup(&ancestor, name, depth - 1) {
                Lookup::Unknown => return Lookup::Unknown,
                Lookup::Absent => {}
                // A parent's private constant is not inherited, and a trait's becomes the
                // class's own: either is read where PHP may refuse it.
                Lookup::Found(inherited) if inherited.visibility == Visibility::Private => {
                    return Lookup::Unknown;
                }
                Lookup::Found(inherited) => match &found {
                    Lookup::Found(earlier) if earlier.value != inherited.value => {
                        return Lookup::Unknown;
                    }
                    _ => found = Lookup::Found(inherited),
                },
            }
        }
        found
    }

    #[cfg(test)]
    pub(crate) fn references(&self) -> impl Iterator<Item = (Range<usize>, &Reference)> {
        self.references
            .iter()
            .map(|(&(start, end), reference)| (start..end, reference))
    }
}

/// Ancestors an inherited class constant is looked up through, at most (a cycle is unknown).
const MAX_ANCESTORS: usize = 64;

/// What a class-like gives a constant name.
enum Lookup {
    Found(Found),
    /// Neither it nor an ancestor declares the name.
    Absent,
    /// Not known: an ancestor or a trait this file does not declare may.
    Unknown,
}

struct Found {
    value: NodeRef,
    /// The class-like declaring it.
    owner: NodeRef,
    is_final: bool,
    visibility: Visibility,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Visibility {
    Public,
    Protected,
    Private,
}

/// The class-like whose body `node` is in, which `self` and `static` name: `None` in a trait
/// (the class using it) and in a function declared inside a method (which has no class).
fn enclosing_class<'r, D: Doc>(node: &Node<'r, D>) -> Option<Node<'r, D>> {
    for ancestor in node.ancestors() {
        match ancestor.kind().as_ref() {
            "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "anonymous_class" => return Some(ancestor),
            "trait_declaration" | "function_definition" => return None,
            _ => {}
        }
    }
    None
}

/// Whether no class can extend `class`: a `final` class, an enum or an anonymous class.
fn is_final<D: Doc>(class: &Node<'_, D>) -> bool {
    match class.kind().as_ref() {
        "enum_declaration" | "anonymous_class" => true,
        "class_declaration" => class
            .children()
            .any(|child| child.kind() == "final_modifier"),
        _ => false,
    }
}

/// A name as written: its form and its segments.
#[derive(Debug)]
enum Form {
    FullyQualified,
    Relative,
    Qualified,
    Unqualified,
}

/// A resolution before the fallback of an unqualified function or constant is decided.
enum Pending {
    Done(Resolved),
    Fallback {
        kind: NameKind,
        namespaced: String,
        global: String,
    },
}

struct Builder<'r, D: Doc> {
    /// The current namespace, `""` for the global one.
    namespace: String,
    /// `use` aliases: classes and functions keyed lowercased, constants as written.
    classes: HashMap<String, String>,
    functions: HashMap<String, String>,
    constants: HashMap<String, String>,
    references: Vec<((usize, usize), Option<NameKind>, Pending)>,
    /// Functions this file declares, lowercased.
    declared_functions: HashSet<String>,
    /// Constants this file declares, by `constant_key`.
    declared_constants: HashSet<String>,
    /// The values of the constants this file declares (`PhpNames::constants`).
    constant_values: Declared,
    /// The class-likes this file declares (`PhpNames::classes`).
    declared_classes: Declared,
    /// Calls that may be the global `define`: the callee's resolution and the call.
    defines: Vec<(Pending, Node<'r, D>)>,
    /// What each `use` clause imports, by its name's range.
    imports: HashMap<(usize, usize), (NameKind, Box<str>)>,
}

impl<'r, D: Doc> Builder<'r, D> {
    fn new() -> Self {
        Self {
            namespace: String::new(),
            classes: HashMap::new(),
            functions: HashMap::new(),
            constants: HashMap::new(),
            references: Vec::new(),
            declared_functions: HashSet::new(),
            declared_constants: HashSet::new(),
            constant_values: HashMap::new(),
            declared_classes: HashMap::new(),
            defines: Vec::new(),
            imports: HashMap::new(),
        }
    }

    /// Reads one node; whether its children are read too.
    fn visit(&mut self, node: &Node<'r, D>) -> bool {
        match node.kind().as_ref() {
            "namespace_definition" => {
                self.namespace = node
                    .field("name")
                    .map(|name| segments_text(&name))
                    .unwrap_or_default();
                self.classes.clear();
                self.functions.clear();
                self.constants.clear();
                // Its name is no reference (`position`); the walk reads its body, inside it.
                true
            }
            "namespace_use_declaration" => {
                self.add_uses(node);
                false
            }
            "function_definition" => {
                if let Some(name) = node.field("name") {
                    let name = qualify(&self.namespace, &name.text());
                    self.declared_functions.insert(name.to_ascii_lowercase());
                }
                true
            }
            "class_declaration"
            | "interface_declaration"
            | "enum_declaration"
            | "trait_declaration" => {
                if let Some(name) = node.field("name") {
                    let name = qualify(&self.namespace, &name.text()).to_ascii_lowercase();
                    declare(&mut self.declared_classes, name, NodeRef::of(node));
                }
                true
            }
            "const_declaration"
                if !node.parent().is_some_and(|parent| {
                    matches!(
                        parent.kind().as_ref(),
                        "declaration_list" | "enum_declaration_list"
                    )
                }) =>
            {
                for element in node
                    .children()
                    .filter(|child| child.kind() == "const_element")
                {
                    if let Some(name) = first_named(&element) {
                        let key = constant_key(&qualify(&self.namespace, &name.text()));
                        self.declared_constants.insert(key.clone());
                        if let Some(value) = const_value(&element) {
                            declare(&mut self.constant_values, key, NodeRef::of(&value));
                        }
                    }
                }
                true
            }
            "function_call_expression" => {
                if let Some(callee) = node.field("function")
                    && is_name_kind(&callee.kind())
                    && let Some(pending) = self.resolve(&callee, NameKind::Function)
                {
                    let named_define = match &pending {
                        Pending::Done(Resolved::Name(name)) => name.eq_ignore_ascii_case("define"),
                        Pending::Fallback { global, .. } => global.eq_ignore_ascii_case("define"),
                        Pending::Done(_) => false,
                    };
                    if named_define {
                        self.defines.push((pending, node.clone()));
                    }
                }
                true
            }
            kind if is_name_kind(kind) => {
                // A `name` holding an expression (`X::{expr}`) is no name: read what it holds.
                if kind == "name" && node.children().any(|child| child.is_named()) {
                    return true;
                }
                if let Some(kind) = position(node) {
                    let pending = match kind {
                        Some(kind) => self
                            .resolve(node, kind)
                            .unwrap_or(Pending::Done(Resolved::Unknown)),
                        None => Pending::Done(Resolved::Unknown),
                    };
                    let range = node.range();
                    self.references
                        .push(((range.start, range.end), kind, pending));
                }
                false
            }
            _ => true,
        }
    }

    /// Adds the aliases of a `use` statement: single, multiple, grouped and mixed groups, each of
    /// classes (the default), functions or constants.
    fn add_uses(&mut self, node: &Node<'r, D>) {
        let prefix = node
            .children()
            .find(|child| child.kind() == "namespace_name")
            .map(|prefix| segments_text(&prefix));
        let clauses: Vec<_> = match node.field("body") {
            Some(group) => group
                .children()
                .filter(|child| child.kind() == "namespace_use_clause")
                .collect(),
            None => node
                .children()
                .filter(|child| child.kind() == "namespace_use_clause")
                .collect(),
        };
        for clause in &clauses {
            let Some(name) = clause
                .children()
                .find(|child| matches!(child.kind().as_ref(), "name" | "qualified_name"))
            else {
                continue;
            };
            let written = name.text();
            let written = written.trim_start_matches('\\');
            if !written.split('\\').all(is_label) {
                continue;
            }
            let target = match &prefix {
                Some(prefix) if !prefix.is_empty() => format!("{prefix}\\{written}"),
                _ => written.to_owned(),
            };
            let alias = match clause.field("alias") {
                Some(alias) => alias.text().into_owned(),
                None => last_segment(&target).to_owned(),
            };
            let range = name.range();
            let kind = match import_kind(clause) {
                ImportKind::Class => NameKind::Class,
                ImportKind::Function => NameKind::Function,
                ImportKind::Constant => NameKind::Constant,
            };
            let aliases = match kind {
                NameKind::Function => &mut self.functions,
                NameKind::Constant => &mut self.constants,
                NameKind::Class => &mut self.classes,
            };
            // Constants are aliased as written, classes and functions lowercased.
            let alias = match kind {
                NameKind::Constant => alias,
                _ => alias.to_ascii_lowercase(),
            };
            aliases.insert(alias, target.clone());
            self.imports
                .insert((range.start, range.end), (kind, target.into()));
        }
    }

    /// PHP-Parser's `NameContext::getResolvedName`, for the name `node` in a `kind` position:
    /// `None` for a name PHP would reject.
    fn resolve(&self, node: &Node<'r, D>, kind: NameKind) -> Option<Pending> {
        let (form, parts) = parse(node)?;
        let done = |name: String| Some(Pending::Done(Resolved::Name(name.into())));
        match form {
            Form::FullyQualified => done(parts.join("\\")),
            Form::Relative => done(qualify(&self.namespace, &parts.join("\\"))),
            Form::Qualified => {
                // A qualified name's first segment resolves through the class aliases, whatever
                // the name's kind.
                let rest = parts[1..].join("\\");
                match self.classes.get(&parts[0].to_ascii_lowercase()) {
                    Some(alias) => done(format!("{alias}\\{rest}")),
                    None => done(qualify(&self.namespace, &parts.join("\\"))),
                }
            }
            Form::Unqualified => {
                let name = parts[0].as_str();
                let alias = match kind {
                    NameKind::Class => {
                        if is_special_class(name) {
                            return Some(Pending::Done(Resolved::Special));
                        }
                        return done(match self.classes.get(&name.to_ascii_lowercase()) {
                            Some(alias) => alias.clone(),
                            None => qualify(&self.namespace, name),
                        });
                    }
                    NameKind::Function => self.functions.get(&name.to_ascii_lowercase()),
                    // PHP reads these three as its own constants in any namespace.
                    NameKind::Constant if is_special_constant(name) => {
                        return done(name.to_owned());
                    }
                    NameKind::Constant => self.constants.get(name),
                };
                match alias {
                    Some(alias) => done(alias.clone()),
                    None if self.namespace.is_empty() => done(name.to_owned()),
                    None => Some(Pending::Fallback {
                        kind,
                        namespaced: qualify(&self.namespace, name),
                        global: name.to_owned(),
                    }),
                }
            }
        }
    }

    /// Decides every fallback once the file's declarations are known: the references, and the
    /// values of the constants and the class-likes the file declares.
    fn finish(mut self) -> (HashMap<(usize, usize), Reference>, Declared, Declared) {
        let defines = std::mem::take(&mut self.defines);
        for (callee, call) in defines {
            if !matches!(self.settle(callee), Resolved::Name(name) if name.eq_ignore_ascii_case("define"))
            {
                continue;
            }
            if let Some(name) = defined_name(&call) {
                let key = constant_key(&name);
                self.declared_constants.insert(key.clone());
                // A `define` the top level runs as a statement holds its value; one elsewhere
                // may run or not, before or after another.
                match argument_for(&call, 1, "value") {
                    Some(value) if is_top_level_statement(&call) => {
                        declare(&mut self.constant_values, key, NodeRef::of(&value));
                    }
                    _ => {
                        self.constant_values.insert(key, None);
                    }
                }
            }
        }
        let references = std::mem::take(&mut self.references);
        let references = references
            .into_iter()
            .map(|(range, kind, pending)| {
                let resolved = self.settle(pending);
                (range, Reference { kind, resolved })
            })
            .collect();
        (references, self.constant_values, self.declared_classes)
    }

    fn settle(&self, pending: Pending) -> Resolved {
        match pending {
            Pending::Done(resolved) => resolved,
            Pending::Fallback {
                kind,
                namespaced,
                global,
            } => {
                let (declared, builtin) = match kind {
                    NameKind::Function => (
                        self.declared_functions
                            .contains(&namespaced.to_ascii_lowercase()),
                        builtin_function(&global.to_ascii_lowercase()),
                    ),
                    _ => (
                        self.declared_constants.contains(&constant_key(&namespaced)),
                        builtin_constant(&global),
                    ),
                };
                if declared {
                    Resolved::Name(namespaced.into())
                } else if builtin {
                    Resolved::Name(global.into())
                } else {
                    Resolved::Either {
                        namespaced: namespaced.into(),
                        global: global.into(),
                    }
                }
            }
        }
    }
}

/// The name a call of the global `define` declares: its `constant_name` argument (the first
/// positional one, or the one so named) when it is a constant expression of a string. A leading
/// backslash declares a constant no name reaches.
fn defined_name<D: Doc>(call: &Node<'_, D>) -> Option<String> {
    let expression = argument_for(call, 0, "constant_name")?;
    let mut spend = || true;
    let value = php_value::decode(&expression, &mut Context::new(&mut spend));
    let Value::PhpStr(bytes) = value else {
        return None;
    };
    let name = String::from_utf8(bytes).ok()?;
    (!name.starts_with('\\') && !name.is_empty()).then_some(name)
}

/// The expression `call` passes for its parameter `name` at `position`: the positional argument
/// there, or the one so named; `None` for an `...` unpacking.
fn argument_for<'r, D: Doc>(
    call: &Node<'r, D>,
    position: usize,
    name: &str,
) -> Option<Node<'r, D>> {
    let arguments = call.field("arguments")?;
    let (_, argument) = arguments
        .children()
        .filter(|child| child.kind() == "argument")
        .enumerate()
        .find(|(index, argument)| match argument.field("name") {
            None => *index == position,
            Some(label) => label.text() == name,
        })?;
    let expression = argument
        .children()
        .filter(|child| child.is_named() && !child.kind().ends_with("comment"))
        .last()?;
    if expression.kind() == "variadic_unpacking"
        || argument
            .field("name")
            .is_some_and(|label| label.range() == expression.range())
    {
        return None;
    }
    Some(expression)
}

/// Whether `call` is a statement of the file's top level, or of a namespace's block there.
fn is_top_level_statement<D: Doc>(call: &Node<'_, D>) -> bool {
    let Some(statement) = call.parent() else {
        return false;
    };
    statement.kind() == "expression_statement"
        && statement
            .parent()
            .is_some_and(|block| match block.kind().as_ref() {
                "program" => true,
                "compound_statement" => block
                    .parent()
                    .is_some_and(|parent| parent.kind() == "namespace_definition"),
                _ => false,
            })
}

/// The value a `const_element` gives its name.
fn const_value<'r, D: Doc>(element: &Node<'r, D>) -> Option<Node<'r, D>> {
    let mut named = element
        .children()
        .filter(|child| child.is_named() && !child.kind().ends_with("comment"));
    let _name = named.next()?;
    let value = named.next()?;
    named.next().is_none().then_some(value)
}

/// Declares `key` once more: a second declaration makes it unknown.
fn declare(declared: &mut Declared, key: String, node: NodeRef) {
    declared
        .entry(key)
        .and_modify(|known| *known = None)
        .or_insert(Some(node));
}

/// The position of the reference kind `node` stands in: `None` where it is no reference,
/// `Some(None)` inside an error region.
fn position<D: Doc>(node: &Node<'_, D>) -> Option<Option<NameKind>> {
    let parent = node.parent()?;
    let range = node.range();
    let is = |field: &str| {
        parent
            .field(field)
            .is_some_and(|child| child.range() == range)
    };
    let kind = match parent.kind().as_ref() {
        "qualified_name"
        | "relative_name"
        | "namespace_name"
        | "variable_name"
        | "namespace_use_clause"
        | "namespace_use_group"
        | "namespace_definition"
        | "use_as_clause"
        | "goto_statement"
        | "named_label_statement"
        | "visibility_modifier" => return None,
        // A hook's name (`get`, `set`) is no reference; a short hook's body (`get => X;`) is an
        // expression.
        "property_hook" if !is("body") => return None,
        "ERROR" => return Some(None),
        "object_creation_expression"
        | "base_clause"
        | "class_interface_clause"
        | "attribute"
        | "use_declaration"
        | "use_instead_of_clause" => NameKind::Class,
        "named_type" if is_builtin_type(node) => return None,
        "named_type" => NameKind::Class,
        "class_constant_access_expression" => {
            if !first_named(&parent).is_some_and(|first| first.range() == range) {
                return None;
            }
            NameKind::Class
        }
        "scoped_call_expression" | "scoped_property_access_expression" => {
            if !is("scope") {
                return None;
            }
            NameKind::Class
        }
        "binary_expression"
            if is("right")
                && parent
                    .field("operator")
                    .is_some_and(|operator| operator.kind().eq_ignore_ascii_case("instanceof")) =>
        {
            NameKind::Class
        }
        // `isset()`, `empty()`, `eval()`, `die()`, `exit()` are language constructs.
        "function_call_expression" if is("function") && is_construct(node) => return None,
        "function_call_expression" if is("function") => NameKind::Function,
        // `"${name}"` names the variable `$name`.
        "dynamic_variable_name" if is_string_part(&parent) => return None,
        "function_definition"
        | "method_declaration"
        | "class_declaration"
        | "interface_declaration"
        | "trait_declaration"
        | "enum_declaration"
        | "enum_case"
        | "member_access_expression"
        | "nullsafe_member_access_expression"
        | "member_call_expression"
        | "nullsafe_member_call_expression"
        | "argument"
            if is("name") =>
        {
            return None;
        }
        "const_element" if first_named(&parent).is_some_and(|first| first.range() == range) => {
            return None;
        }
        "subscript_expression" if is_string_part(&parent) && !follows_brace(&parent) => {
            return None;
        }
        // Magic constants are compile-time values, not constants.
        _ if is_magic_constant(node) => return None,
        _ => NameKind::Constant,
    };
    // `true`, `false` and `null` are only ever constants.
    if kind != NameKind::Constant && matches!(node.kind().as_ref(), "boolean" | "null") {
        return None;
    }
    Some(Some(kind))
}

/// Whether `part` is an interpolation directly inside a string.
fn is_string_part<D: Doc>(part: &Node<'_, D>) -> bool {
    part.parent().is_some_and(|parent| {
        matches!(
            parent.kind().as_ref(),
            "encapsed_string" | "heredoc_body" | "shell_command_expression"
        )
    })
}

/// Whether an interpolation is the complex (`{$a[KEY]}`) form, whose key is an expression; in
/// the simple form (`$a[KEY]`) a bare key is a string.
fn follows_brace<D: Doc>(part: &Node<'_, D>) -> bool {
    part.prev().is_some_and(|previous| previous.kind() == "{")
}

fn is_construct<D: Doc>(node: &Node<'_, D>) -> bool {
    node.kind() == "name"
        && ["isset", "empty", "eval", "die", "exit", "__halt_compiler"]
            .iter()
            .any(|construct| node.text().eq_ignore_ascii_case(construct))
}

fn is_magic_constant<D: Doc>(node: &Node<'_, D>) -> bool {
    node.kind() == "name"
        && [
            "__LINE__",
            "__FILE__",
            "__DIR__",
            "__FUNCTION__",
            "__CLASS__",
            "__TRAIT__",
            "__METHOD__",
            "__NAMESPACE__",
            "__PROPERTY__",
        ]
        .iter()
        .any(|magic| node.text().eq_ignore_ascii_case(magic))
}

fn is_builtin_type<D: Doc>(node: &Node<'_, D>) -> bool {
    node.kind() == "name"
        && BUILTIN_TYPES
            .iter()
            .any(|builtin| node.text().eq_ignore_ascii_case(builtin))
}

fn is_name_kind(kind: &str) -> bool {
    REFERENCE_KINDS.contains(&kind)
}

fn first_named<'r, D: Doc>(node: &Node<'r, D>) -> Option<Node<'r, D>> {
    node.children()
        .find(|child| child.is_named() && !child.kind().ends_with("comment"))
}

/// A name's form and segments; `None` for a name PHP would not read as one token.
fn parse<D: Doc>(node: &Node<'_, D>) -> Option<(Form, Vec<String>)> {
    let text = node.text();
    let (form, rest) = match node.kind().as_ref() {
        "relative_name" => {
            let (keyword, rest) = text.split_once('\\')?;
            if !keyword.eq_ignore_ascii_case("namespace") {
                return None;
            }
            (Form::Relative, rest)
        }
        _ => match text.strip_prefix('\\') {
            Some(rest) => (Form::FullyQualified, rest),
            None if text.contains('\\') => (Form::Qualified, text.as_ref()),
            None => (Form::Unqualified, text.as_ref()),
        },
    };
    let parts: Vec<String> = rest.split('\\').map(str::to_owned).collect();
    parts
        .iter()
        .all(|part| is_label(part))
        .then_some((form, parts))
}

/// A PHP label: a letter, `_` or a byte from 0x80, then those or digits.
pub(crate) fn is_label(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_' || !first.is_ascii())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || !c.is_ascii())
}

/// A namespace name's segments joined by `\`, without whitespace or comments between them.
fn segments_text<D: Doc>(node: &Node<'_, D>) -> String {
    node.children()
        .filter(|child| child.kind() == "name")
        .map(|child| child.text().into_owned())
        .collect::<Vec<_>>()
        .join("\\")
}

fn qualify(namespace: &str, name: &str) -> String {
    if namespace.is_empty() {
        name.to_owned()
    } else {
        format!("{namespace}\\{name}")
    }
}

fn last_segment(name: &str) -> &str {
    name.rsplit('\\').next().unwrap_or(name)
}

/// A constant's name as PHP looks it up: namespace segments lowercased, its own name as written.
fn constant_key(name: &str) -> String {
    match name.rsplit_once('\\') {
        Some((namespace, own)) => format!("{}\\{own}", namespace.to_ascii_lowercase()),
        None => name.to_owned(),
    }
}

fn is_special_class(name: &str) -> bool {
    ["self", "static", "parent"]
        .iter()
        .any(|special| name.eq_ignore_ascii_case(special))
}

pub(crate) fn is_special_constant(name: &str) -> bool {
    ["true", "false", "null"]
        .iter()
        .any(|special| name.eq_ignore_ascii_case(special))
}

/// Whether a resolved name is one of `listed`, or with `prefix` lies under one: classes and
/// functions compared case-insensitively, constants case-sensitively in their own name (`true`,
/// `false` and `null` aside). Listed names may carry a leading backslash.
pub(crate) fn listed<'a>(
    listed: impl IntoIterator<Item = &'a String>,
    kind: NameKind,
    name: &str,
    prefix: bool,
) -> bool {
    listed.into_iter().any(|entry| {
        let entry = entry.strip_prefix('\\').unwrap_or(entry);
        same_name(entry, name, kind)
            || (prefix
                && name.len() > entry.len()
                && name.as_bytes()[entry.len()] == b'\\'
                && name[..entry.len()].eq_ignore_ascii_case(entry))
    })
}

fn same_name(entry: &str, name: &str, kind: NameKind) -> bool {
    match kind {
        NameKind::Class | NameKind::Function => entry.eq_ignore_ascii_case(name),
        NameKind::Constant if is_special_constant(name) => entry.eq_ignore_ascii_case(name),
        NameKind::Constant => match (entry.rsplit_once('\\'), name.rsplit_once('\\')) {
            (Some((entry_space, entry_own)), Some((space, own))) => {
                entry_own == own && entry_space.eq_ignore_ascii_case(space)
            }
            (None, None) => entry == name,
            _ => false,
        },
    }
}

/// PHP 8.5.11's builtin names (`data/generate_php_builtins.php`).
#[derive(Deserialize)]
struct BuiltinsFile {
    functions: HashSet<String>,
    constants: HashSet<String>,
}

static BUILTINS: LazyLock<BuiltinsFile> = LazyLock::new(|| {
    serde_json::from_str(include_str!("data/php_builtins.json"))
        .expect("php_builtins.json is valid")
});

/// Whether PHP 8.5.11 defines the function, by its lowercased name.
pub(crate) fn builtin_function(lowercased: &str) -> bool {
    BUILTINS.functions.contains(lowercased)
}

/// Whether PHP 8.5.11 defines the constant, spelled exactly.
fn builtin_constant(name: &str) -> bool {
    is_special_constant(name) || BUILTINS.constants.contains(name)
}

#[cfg(test)]
mod tests;
