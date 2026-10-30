//! String type definitions, after Ruff 0.16.9's `Checker` (`checkers/ast/mod.rs`): which string
//! literals Ruff reads as type definitions (`string_type_definitions`), parsed and read in the
//! scope they occur in, their names forward references (`in_forward_reference`), nested strings up
//! to eight levels deep.
//!
//! A string is a type definition where Ruff's `TYPE_DEFINITION` flag holds and `TYPING_LITERAL`
//! does not: in an annotation, a type alias's value or a type parameter's bound; in the slice of a
//! typing generic's subscript (`List["T"]`, the first element of `Annotated[...]`, a `TypedDict`
//! display's values), and in the arguments typing callables read as types (`cast`, `TypeVar`,
//! `NewType`, `NamedTuple`, `TypedDict`, `TypeAliasType`, `TypeForm`, mypy_extensions' `Arg`s).
//! Other calls' arguments are not; `Literal[...]` never is. Whether a generic or callable is the
//! typing one is a qualified name, read from the finished table.

use super::unparenthesized;

use super::super::options::{DEFAULT_PYTHON_MINOR, PythonTyping};
use super::super::{
    Annotation, BuildError, Builder, DEADLINE_INTERVAL, Import, MAX_BUILD_DEPTH, NodeRef, Reading,
    ScopeTable, annotation_offset,
};
use super::super::{Origin, ScopeKind};
use super::{Visit, Visitor, named};
use crate::code_rule_dsl::identifier::python_identifier;
use crate::code_rule_dsl::literal::{SubDoc, annotation, semantic};
use crate::code_rule_dsl::module_path::ModulePath;
use ast_grep_core::{AstGrep, Doc, Node};
use std::sync::Arc;

/// How deep strings nest in string type definitions before Rerail stops reading them.
const MAX_NESTING: usize = 8;

/// Callees whose arguments a typing callable may read as types (the last segment of the name).
pub(super) const TYPING_CALLABLES: &[&str] = &[
    "cast",
    "TypeForm",
    "NewType",
    "TypeVar",
    "TypeAliasType",
    "NamedTuple",
    "TypedDict",
    "Arg",
    "DefaultArg",
    "NamedArg",
    "DefaultNamedArg",
    "VarArg",
    "KwArg",
];

/// Subscripted names whose slice may be a type definition or a literal: Ruff's
/// `is_standard_library_generic_member`, `Literal`, `Annotated` and `TypedDict`.
pub(super) const SUBSCRIPT_MEMBERS: &[&str] = &[
    "Literal",
    "Annotated",
    "TypedDict",
    "dict",
    "frozendict",
    "frozenset",
    "list",
    "set",
    "slice",
    "tuple",
    "type",
    "ChainMap",
    "Counter",
    "OrderedDict",
    "defaultdict",
    "deque",
    "AsyncGenerator",
    "AsyncIterable",
    "AsyncIterator",
    "Awaitable",
    "ByteString",
    "Callable",
    "Collection",
    "Container",
    "Coroutine",
    "Generator",
    "ItemsView",
    "Iterable",
    "Iterator",
    "KeysView",
    "Mapping",
    "MappingView",
    "MutableMapping",
    "MutableSequence",
    "MutableSet",
    "Reversible",
    "Sequence",
    "Set",
    "ValuesView",
    "AbstractAsyncContextManager",
    "AbstractContextManager",
    "Match",
    "Pattern",
    "AbstractSet",
    "AsyncContextManager",
    "BinaryIO",
    "ClassVar",
    "Concatenate",
    "ContextManager",
    "DefaultDict",
    "Deque",
    "Dict",
    "Final",
    "FrozenSet",
    "Generic",
    "IO",
    "List",
    "Optional",
    "TextIO",
    "Tuple",
    "Type",
    "TypeForm",
    "TypeGuard",
    "TypeIs",
    "Union",
    "Unpack",
    "Required",
    "NotRequired",
    "ReadOnly",
    "WeakKeyDictionary",
    "WeakSet",
    "WeakValueDictionary",
];

/// Where a type definition may start: recorded while the table is built, classified after.
pub(crate) struct TypeRoot {
    pub(super) node: NodeRef,
    pub(super) kind: RootKind,
}

pub(super) enum RootKind {
    /// An annotation, a `type` statement's value, a type parameter's bound.
    Definition,
    /// A subscript or call whose name may be a typing generic or callable.
    Candidate,
    /// The value of `x: A = value`, a type definition where `A` is `TypeAlias`.
    AliasValue(NodeRef),
}

/// The name a subscript's value or a call's callee ends with.
pub(super) fn last_segment<D: Doc>(node: &Node<'_, D>) -> String {
    match node.kind().as_ref() {
        "attribute" => node
            .field("attribute")
            .map(|attribute| python_identifier(&attribute).into_owned())
            .unwrap_or_default(),
        _ => python_identifier(node).into_owned(),
    }
}

/// Ruff's flags: `TYPE_DEFINITION` and `TYPING_LITERAL`.
#[derive(Clone, Copy)]
struct Flags {
    definition: bool,
    literal: bool,
}

impl Flags {
    fn with(self, definition: bool) -> Self {
        Self { definition, ..self }
    }
}

/// Ruff's `resolve_qualified_name` over the finished table: an import's qualified name, a
/// builtin's (`["", name]`), then the attribute names after the head; a relative import or a
/// definition by the module path, where there is one. `None` for a name bound otherwise, a
/// deleted one, or an expression that is no name or attribute chain.
fn qualified_name<D: Doc>(
    table: &ScopeTable,
    module: Option<&ModulePath>,
    node: &Node<'_, D>,
) -> Option<Vec<String>> {
    let mut tail = Vec::new();
    let mut head = node.clone();
    loop {
        match head.kind().as_ref() {
            "attribute" => {
                tail.push(python_identifier(&head.field("attribute")?).into_owned());
                head = head.field("object")?;
            }
            "parenthesized_expression" => {
                let inner = named(&head)
                    .filter(|child| child.kind() != "comment")
                    .collect::<Vec<_>>();
                let [inner] = inner.as_slice() else {
                    return None;
                };
                head = inner.clone();
            }
            "identifier" => break,
            _ => return None,
        }
    }
    tail.reverse();
    let name = python_identifier(&head);
    let reference = table.reference(&head.range())?;
    let mut segments = match table.resolve(reference, &name) {
        None => {
            if !matches!(
                name.as_ref(),
                "dict" | "frozenset" | "list" | "set" | "slice" | "tuple" | "type" | "bool"
            ) {
                return None;
            }
            vec![String::new(), name.into_owned()]
        }
        Some(binding) if binding.is_deletion() => return None,
        Some(binding) => {
            let resolved = match binding.import.as_ref()? {
                Import::Qualified(qualified) => Some(qualified.to_string()),
                Import::Relative { level, rest } => {
                    module.and_then(|module| module.relative(*level, rest, &[]))
                }
                Import::Definition => module.and_then(|module| module.definition(&name, &[])),
            };
            match resolved {
                Some(qualified) => qualified.split('.').map(str::to_owned).collect(),
                // Unqualified, it names no standard library or configured typing name.
                None => vec![".".to_owned(), name.into_owned()],
            }
        }
    };
    segments.extend(tail);
    Some(segments)
}

/// Ruff's `match_typing_qualified_name`: `member` of `typing`, `_typeshed`, `typing_extensions` or
/// a configured typing module.
pub(super) fn typing(segments: &[String], member: &str, settings: Option<&PythonTyping>) -> bool {
    matches!(
        segments,
        [module, name] if matches!(module.as_str(), "typing" | "_typeshed" | "typing_extensions") && name == member
    ) || settings.is_some_and(|settings| settings.module_member(segments, member))
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum SubscriptKind {
    Literal,
    Generic,
    Annotated,
    TypedDict,
}

/// Ruff's `match_annotated_subscript`: the standard library's `Literal`, generics (and the
/// configured ones), `Annotated` and `TypedDict`, then a configured typing module's member read by
/// its last name.
pub(super) fn subscript_kind(
    segments: &[String],
    settings: Option<&PythonTyping>,
) -> Option<SubscriptKind> {
    let parts: Vec<&str> = segments.iter().map(String::as_str).collect();
    let pair = |module: &[&str], member: &[&str]| {
        parts.len() == 2 && module.contains(&parts[0]) && member.contains(&parts[1])
    };
    if pair(&["typing", "typing_extensions"], &["Literal"]) {
        return Some(SubscriptKind::Literal);
    }
    if generic(&parts) || settings.is_some_and(|settings| settings.generic(segments)) {
        return Some(SubscriptKind::Generic);
    }
    if pair(&["typing", "typing_extensions"], &["Annotated"]) {
        return Some(SubscriptKind::Annotated);
    }
    if pair(&["typing", "typing_extensions"], &["TypedDict"]) {
        return Some(SubscriptKind::TypedDict);
    }
    match settings.and_then(|settings| settings.module_tail(segments))? {
        "Literal" => Some(SubscriptKind::Literal),
        "Annotated" => Some(SubscriptKind::Annotated),
        "TypedDict" => Some(SubscriptKind::TypedDict),
        member if generic_member(member) => Some(SubscriptKind::Generic),
        _ => None,
    }
}

/// Ruff's `is_standard_library_generic_member`: the last names of `generic`'s patterns.
fn generic_member(member: &str) -> bool {
    !matches!(
        member,
        "Literal" | "Annotated" | "TypedDict" | "Required" | "NotRequired" | "ReadOnly"
    ) && SUBSCRIPT_MEMBERS.contains(&member)
}

/// Ruff's `is_standard_library_generic`.
fn generic(parts: &[&str]) -> bool {
    match parts {
        ["" | "builtins", name] => matches!(
            *name,
            "dict" | "frozendict" | "frozenset" | "list" | "set" | "slice" | "tuple" | "type"
        ),
        [
            "collections" | "typing" | "typing_extensions",
            "ChainMap" | "Counter",
        ] => true,
        ["collections" | "typing", "OrderedDict"] => true,
        ["collections", "defaultdict" | "deque"] => true,
        ["collections", "abc", name] => matches!(
            *name,
            "AsyncGenerator"
                | "AsyncIterable"
                | "AsyncIterator"
                | "Awaitable"
                | "ByteString"
                | "Callable"
                | "Collection"
                | "Container"
                | "Coroutine"
                | "Generator"
                | "ItemsView"
                | "Iterable"
                | "Iterator"
                | "KeysView"
                | "Mapping"
                | "MappingView"
                | "MutableMapping"
                | "MutableSequence"
                | "MutableSet"
                | "Reversible"
                | "Sequence"
                | "Set"
                | "ValuesView"
        ),
        [
            "contextlib",
            "AbstractAsyncContextManager" | "AbstractContextManager",
        ] => true,
        ["re" | "typing", "Match" | "Pattern"] => true,
        ["typing", name] => matches!(
            *name,
            "AbstractSet"
                | "AsyncContextManager"
                | "AsyncGenerator"
                | "AsyncIterator"
                | "Awaitable"
                | "BinaryIO"
                | "ByteString"
                | "Callable"
                | "ClassVar"
                | "Collection"
                | "Concatenate"
                | "Container"
                | "ContextManager"
                | "Coroutine"
                | "DefaultDict"
                | "Deque"
                | "Dict"
                | "Final"
                | "FrozenSet"
                | "Generator"
                | "Generic"
                | "IO"
                | "ItemsView"
                | "Iterable"
                | "Iterator"
                | "KeysView"
                | "List"
                | "Mapping"
                | "MutableMapping"
                | "MutableSequence"
                | "MutableSet"
                | "Optional"
                | "Reversible"
                | "Sequence"
                | "Set"
                | "TextIO"
                | "Tuple"
                | "Type"
                | "TypeForm"
                | "TypeGuard"
                | "TypeIs"
                | "Union"
                | "Unpack"
                | "ValuesView"
        ),
        ["typing", "io", "BinaryIO" | "IO" | "TextIO"] => true,
        ["typing", "re", "Match" | "Pattern"] => true,
        ["typing_extensions", name] => matches!(
            *name,
            "AsyncContextManager"
                | "AsyncGenerator"
                | "AsyncIterable"
                | "AsyncIterator"
                | "Awaitable"
                | "ClassVar"
                | "Concatenate"
                | "ContextManager"
                | "Coroutine"
                | "DefaultDict"
                | "Deque"
                | "Type"
                | "TypeForm"
                | "TypeGuard"
                | "TypeIs"
        ),
        [
            "weakref",
            "WeakKeyDictionary" | "WeakSet" | "WeakValueDictionary",
        ] => true,
        _ => false,
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Callable {
    Cast,
    TypeForm,
    NewType,
    TypeVar,
    TypeAliasType,
    NamedTuple,
    TypedDict,
    Mypy,
}

fn callable(segments: &[String], settings: Option<&PythonTyping>) -> Option<Callable> {
    let member = |name| typing(segments, name, settings);
    Some(if member("cast") {
        Callable::Cast
    } else if member("TypeForm") {
        Callable::TypeForm
    } else if member("NewType") {
        Callable::NewType
    } else if member("TypeVar") {
        Callable::TypeVar
    } else if member("TypeAliasType") {
        Callable::TypeAliasType
    } else if member("NamedTuple") {
        Callable::NamedTuple
    } else if member("TypedDict") {
        Callable::TypedDict
    } else if matches!(
        segments,
        [module, name] if module == "mypy_extensions"
            && matches!(name.as_str(), "Arg" | "DefaultArg" | "NamedArg" | "DefaultNamedArg" | "VarArg" | "KwArg")
    ) {
        Callable::Mypy
    } else {
        return None;
    })
}

/// A string Ruff's `Expr::StringLiteral` is: no `f`, `b` or `t` prefix in any part.
fn plain_string<D: Doc>(node: &Node<'_, D>) -> bool {
    let parts: Vec<Node<'_, D>> = if node.kind() == "string" {
        vec![node.clone()]
    } else {
        named(node).filter(|part| part.kind() == "string").collect()
    };
    !parts.is_empty()
        && parts.iter().all(|part| {
            let text = part.text();
            let prefix = &text[..text.find(['\'', '"']).unwrap_or(0)];
            !prefix
                .chars()
                .any(|c| matches!(c, 'f' | 'F' | 'b' | 'B' | 't' | 'T'))
        })
}

/// The expression a `type` node wraps.
fn unwrap_type<'r, D: Doc>(node: &Node<'r, D>) -> Node<'r, D> {
    if node.kind() == "type" {
        let mut inner = named(node).filter(|child| child.kind() != "comment");
        if let (Some(expression), None) = (inner.next(), inner.next()) {
            return expression;
        }
    }
    node.clone()
}

/// The elements of a list or tuple display (parentheses aside).
fn display_elements<'r, D: Doc>(node: &Node<'r, D>) -> Option<Vec<Node<'r, D>>> {
    matches!(node.kind().as_ref(), "list" | "tuple").then(|| {
        named(node)
            .filter(|child| child.kind() != "comment")
            .collect()
    })
}

struct Classifier<'t> {
    table: &'t ScopeTable,
    module: Option<&'t ModulePath>,
    /// Nodes walked, charged to the table's cost after.
    steps: usize,
    cap: usize,
    depth: u32,
    deadline: &'t dyn Fn() -> bool,
}

impl Classifier<'_> {
    fn enter(&mut self) -> Visit {
        self.steps += 1;
        if self.table.cost + self.steps > self.cap {
            return Err(BuildError::Budget);
        }
        if self.steps.is_multiple_of(DEADLINE_INTERVAL) && (self.deadline)() {
            return Err(BuildError::Deadline);
        }
        self.depth += 1;
        if self.depth > MAX_BUILD_DEPTH {
            return Err(BuildError::Depth);
        }
        Ok(())
    }

    /// Walks `node` with Ruff's flags, collecting the string type definitions in `found`.
    fn walk<'r, D: Doc>(
        &mut self,
        node: &Node<'r, D>,
        flags: Flags,
        found: &mut Vec<Node<'r, D>>,
    ) -> Visit {
        self.enter()?;
        let kind = node.kind();
        match kind.as_ref() {
            "string" | "concatenated_string" => {
                if flags.definition && !flags.literal && plain_string(node) {
                    found.push(node.clone());
                }
            }
            "subscript" => self.subscript(node, flags, found)?,
            "generic_type" => self.generic_type(node, flags, found)?,
            "call" => self.call(node, flags, found)?,
            "comment" => {}
            _ => {
                for child in named(node) {
                    self.walk(&child, flags, found)?;
                }
            }
        }
        self.depth -= 1;
        Ok(())
    }

    fn walk_all<'r, D: Doc>(
        &mut self,
        nodes: &[Node<'r, D>],
        flags: Flags,
        found: &mut Vec<Node<'r, D>>,
    ) -> Visit {
        for node in nodes {
            self.walk(node, flags, found)?;
        }
        Ok(())
    }

    fn subscript<'r, D: Doc>(
        &mut self,
        node: &Node<'r, D>,
        flags: Flags,
        found: &mut Vec<Node<'r, D>>,
    ) -> Visit {
        let value = node.field("value");
        let slices: Vec<Node<'r, D>> = named(node)
            .filter(|child| {
                child.kind() != "comment"
                    && value
                        .as_ref()
                        .is_none_or(|value| value.node_id() != child.node_id())
            })
            .collect();
        if let Some(value) = &value {
            self.walk(value, flags, found)?;
        }
        // A subscript that is stored or deleted reads no type (Ruff's `ExprContext::Store`).
        let stored = node.parent().is_some_and(|parent| {
            matches!(
                parent.kind().as_ref(),
                "assignment"
                    | "augmented_assignment"
                    | "delete_statement"
                    | "pattern_list"
                    | "for_statement"
            ) && parent
                .field("left")
                .is_none_or(|left| left.node_id() == node.node_id())
        });
        let kind = if stored {
            None
        } else {
            value
                .as_ref()
                .and_then(|value| qualified_name(self.table, self.module, value))
                .and_then(|segments| subscript_kind(&segments, self.table.reading.typing))
        };
        // A slice of several elements is Ruff's tuple.
        let elements: Vec<Node<'r, D>> = match slices.as_slice() {
            [single] if single.kind() == "tuple" => named(single)
                .filter(|child| child.kind() != "comment")
                .collect(),
            _ => slices.clone(),
        };
        match kind {
            Some(SubscriptKind::Literal) => self.walk_all(
                &slices,
                Flags {
                    literal: true,
                    ..flags
                },
                found,
            ),
            Some(SubscriptKind::Generic) => self.walk_all(&slices, flags.with(true), found),
            Some(SubscriptKind::Annotated) => {
                if slices.len() > 1 || slices.first().is_some_and(|slice| slice.kind() == "tuple") {
                    for (index, element) in elements.iter().enumerate() {
                        self.walk(element, flags.with(index == 0), found)?;
                    }
                    Ok(())
                } else {
                    self.walk_all(&slices, flags, found)
                }
            }
            Some(SubscriptKind::TypedDict) => match slices.as_slice() {
                [dict] if dict.kind() == "dictionary" => self.dict(dict, flags, found),
                _ => self.walk_all(&slices, flags.with(false), found),
            },
            None => self.walk_all(&slices, flags, found),
        }
    }

    /// tree-sitter-python's annotation subscript, `Name[T, ...]`: the name, then a `type_parameter`
    /// holding each element as a `type`.
    fn generic_type<'r, D: Doc>(
        &mut self,
        node: &Node<'r, D>,
        flags: Flags,
        found: &mut Vec<Node<'r, D>>,
    ) -> Visit {
        let mut parts = named(node).filter(|child| child.kind() != "comment");
        let Some(value) = parts.next() else {
            return Ok(());
        };
        self.walk(&value, flags, found)?;
        let elements: Vec<Node<'r, D>> = parts
            .filter(|part| part.kind() == "type_parameter")
            .flat_map(|parameter| {
                named(&parameter)
                    .filter(|child| child.kind() != "comment")
                    .collect::<Vec<_>>()
            })
            .map(|element| unwrap_type(&element))
            .collect();
        let kind = qualified_name(self.table, self.module, &value)
            .and_then(|segments| subscript_kind(&segments, self.table.reading.typing));
        match kind {
            Some(SubscriptKind::Literal) => self.walk_all(
                &elements,
                Flags {
                    literal: true,
                    ..flags
                },
                found,
            ),
            Some(SubscriptKind::Generic) => self.walk_all(&elements, flags.with(true), found),
            Some(SubscriptKind::Annotated) if elements.len() > 1 => {
                for (index, element) in elements.iter().enumerate() {
                    self.walk(element, flags.with(index == 0), found)?;
                }
                Ok(())
            }
            Some(SubscriptKind::TypedDict) => match elements.as_slice() {
                [dict] if dict.kind() == "dictionary" => self.dict(dict, flags, found),
                _ => self.walk_all(&elements, flags.with(false), found),
            },
            _ => self.walk_all(&elements, flags, found),
        }
    }

    /// A dict display whose keys are not types and whose values are.
    fn dict<'r, D: Doc>(
        &mut self,
        dict: &Node<'r, D>,
        flags: Flags,
        found: &mut Vec<Node<'r, D>>,
    ) -> Visit {
        for entry in named(dict) {
            match entry.kind().as_ref() {
                "pair" => {
                    if let Some(key) = entry.field("key") {
                        self.walk(&key, flags.with(false), found)?;
                    }
                    if let Some(value) = entry.field("value") {
                        self.walk(&value, flags.with(true), found)?;
                    }
                }
                "comment" => {}
                _ => self.walk(&entry, flags.with(false), found)?,
            }
        }
        Ok(())
    }

    fn call<'r, D: Doc>(
        &mut self,
        node: &Node<'r, D>,
        flags: Flags,
        found: &mut Vec<Node<'r, D>>,
    ) -> Visit {
        let function = node.field("function");
        if let Some(function) = &function {
            self.walk(function, flags, found)?;
        }
        let callable = function
            .as_ref()
            .and_then(|function| qualified_name(self.table, self.module, function))
            .and_then(|segments| callable(&segments, self.table.reading.typing));
        let Some(arguments) = node.field("arguments") else {
            return Ok(());
        };
        // Arguments in source order: positional (a starred one too) and keyword.
        let items: Vec<Node<'r, D>> = named(&arguments)
            .filter(|child| child.kind() != "comment")
            .collect();
        let positional: Vec<&Node<'r, D>> = items
            .iter()
            .filter(|item| {
                !matches!(
                    item.kind().as_ref(),
                    "keyword_argument" | "dictionary_splat"
                )
            })
            .collect();
        let keyword = |item: &Node<'r, D>| -> Option<String> {
            (item.kind() == "keyword_argument")
                .then(|| {
                    item.field("name")
                        .map(|name| python_identifier(&name).into_owned())
                })
                .flatten()
        };
        let value = |item: &Node<'r, D>| -> Node<'r, D> {
            match item.kind().as_ref() {
                "keyword_argument" => item.field("value").unwrap_or_else(|| item.clone()),
                _ => item.clone(),
            }
        };
        let non = flags.with(false);
        let def = flags.with(true);
        match callable {
            None => {
                for item in &items {
                    if item.kind() == "dictionary_splat" {
                        self.walk(item, non, found)?;
                    } else {
                        self.walk(&value(item), non, found)?;
                    }
                }
            }
            Some(Callable::Cast | Callable::NewType | Callable::TypeAliasType) => {
                let (position, names): (usize, &[&str]) = match callable {
                    Some(Callable::Cast) => (0, &["typ"]),
                    Some(Callable::NewType) => (1, &["tp"]),
                    _ => (1, &["value", "type_params"]),
                };
                for (index, item) in items.iter().enumerate() {
                    match item.kind().as_ref() {
                        // Ruff visits no `**` argument of these.
                        "dictionary_splat" => {}
                        "keyword_argument" => {
                            let typed =
                                keyword(item).is_some_and(|name| names.contains(&name.as_str()));
                            self.walk(&value(item), flags.with(typed), found)?;
                        }
                        _ => self.walk(item, flags.with(index == position), found)?,
                    }
                }
            }
            Some(Callable::TypeForm) => {
                for (index, item) in positional.iter().enumerate() {
                    self.walk(item, flags.with(index == 0), found)?;
                }
                for item in items
                    .iter()
                    .filter(|item| item.kind() == "keyword_argument")
                {
                    self.walk(&value(item), non, found)?;
                }
            }
            Some(Callable::TypeVar) => {
                for (index, item) in positional.iter().enumerate() {
                    self.walk(item, flags.with(index > 0), found)?;
                }
                for item in &items {
                    match item.kind().as_ref() {
                        "keyword_argument" => {
                            let typed = keyword(item)
                                .is_some_and(|name| matches!(name.as_str(), "bound" | "default"));
                            self.walk(&value(item), flags.with(typed), found)?;
                        }
                        "dictionary_splat" => self.walk(item, non, found)?,
                        _ => {}
                    }
                }
            }
            Some(Callable::NamedTuple) => {
                for (index, item) in positional.iter().enumerate() {
                    if index == 0 {
                        self.walk(item, non, found)?;
                        continue;
                    }
                    match display_elements(item) {
                        Some(elements) => {
                            for element in elements {
                                match display_elements(&element).filter(|pair| pair.len() == 2) {
                                    Some(pair) => {
                                        self.walk(&pair[0], non, found)?;
                                        self.walk(&pair[1], def, found)?;
                                    }
                                    None => self.walk(&element, non, found)?,
                                }
                            }
                        }
                        None => self.walk(item, non, found)?,
                    }
                }
                for item in &items {
                    match item.kind().as_ref() {
                        "keyword_argument" => self.walk(&value(item), def, found)?,
                        "dictionary_splat" => {
                            let inner = named(item).find(|child| child.kind() != "comment");
                            match inner {
                                Some(dict) if dict.kind() == "dictionary" => {
                                    self.dict(&dict, flags, found)?
                                }
                                _ => self.walk(item, non, found)?,
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some(Callable::TypedDict) => {
                for (index, item) in positional.iter().enumerate() {
                    if index > 0 && item.kind() == "dictionary" {
                        self.dict(item, flags, found)?;
                    } else {
                        self.walk(item, non, found)?;
                    }
                }
                // Stub tables carry their target minor; outside stubs, tables are shared across
                // versions and read the default target, as the scope builder does.
                let minor = match self.table.reading.python_minor {
                    0 => DEFAULT_PYTHON_MINOR,
                    minor => minor,
                };
                let legacy = minor < 13
                    && match positional.as_slice() {
                        [_] => true,
                        [_, second] => {
                            unparenthesized(second, &["parenthesized_expression"]).kind() == "none"
                        }
                        _ => false,
                    };
                for item in &items {
                    match item.kind().as_ref() {
                        "keyword_argument" => {
                            let typed = match keyword(item).as_deref() {
                                Some("total") => false,
                                Some("extra_items") => true,
                                _ => legacy,
                            };
                            self.walk(&value(item), flags.with(typed), found)?;
                        }
                        "dictionary_splat" => self.walk(item, def, found)?,
                        _ => {}
                    }
                }
            }
            Some(Callable::Mypy) => {
                if positional.is_empty() {
                    for item in items
                        .iter()
                        .filter(|item| item.kind() == "keyword_argument")
                    {
                        let typed = keyword(item).as_deref() == Some("type");
                        self.walk(&value(item), flags.with(typed), found)?;
                    }
                } else {
                    for (index, item) in positional.iter().enumerate() {
                        self.walk(item, flags.with(index == 0), found)?;
                    }
                    for item in items
                        .iter()
                        .filter(|item| item.kind() == "keyword_argument")
                    {
                        self.walk(&value(item), non, found)?;
                    }
                }
            }
        }
        Ok(())
    }
}

/// Classifies the string type definitions under the recorded roots, parses them, reads their
/// names in the scopes they occur in, and repeats for the strings nested in them. Their names are
/// deferred forward references: they see every binding, the module's first.
pub(crate) fn string_type_definitions<D: Doc>(
    builder: &mut Builder,
    root: &Node<'_, D>,
    reading: Reading,
) -> Visit {
    let mut roots = std::mem::take(&mut builder.type_roots);
    roots.sort_by_key(|root| (root.node.start, std::cmp::Reverse(root.node.end)));
    let mut found = Vec::new();
    let mut walked_to = 0u32;
    for type_root in &roots {
        if type_root.node.end <= walked_to {
            continue;
        }
        let Some(node) = type_root.node.find(root.clone()) else {
            continue;
        };
        let definition = match &type_root.kind {
            RootKind::Definition => true,
            RootKind::Candidate => false,
            RootKind::AliasValue(annotation) => {
                let Some(annotation) = annotation.find(root.clone()) else {
                    continue;
                };
                qualified_name(&builder.table, builder.module.as_ref(), &annotation).is_some_and(
                    |segments| typing(&segments, "TypeAlias", builder.table.reading.typing),
                )
            }
        };
        classify(builder, &node, definition, &mut found)?;
        walked_to = walked_to.max(type_root.node.end);
    }
    let mut pending = register(builder, root, &found)?;
    for _ in 1..MAX_NESTING {
        if pending.is_empty() {
            break;
        }
        read(builder, &pending, reading)?;
        builder.table.sort_references();
        let mut nested = Vec::new();
        for index in &pending {
            let tree = Arc::clone(&builder.table.annotations[*index].tree);
            if tree.is_error() {
                continue;
            }
            let document = AstGrep::doc(SubDoc { tree });
            let mut inner = Vec::new();
            classify(builder, &document.root(), true, &mut inner)?;
            nested.extend(register(builder, root, &inner)?);
        }
        pending = nested;
    }
    if !pending.is_empty() {
        read(builder, &pending, reading)?;
    }
    builder
        .table
        .annotations
        .sort_by_key(|annotation| annotation.start);
    Ok(())
}

/// Walks one root with Ruff's flags, `definition` set or not.
fn classify<'r, D: Doc>(
    builder: &mut Builder,
    node: &Node<'r, D>,
    definition: bool,
    found: &mut Vec<Node<'r, D>>,
) -> Visit {
    let flags = Flags {
        definition,
        literal: false,
    };
    let mut classifier = Classifier {
        table: &builder.table,
        module: builder.module.as_ref(),
        steps: 0,
        cap: builder.cap,
        depth: 0,
        deadline: builder.deadline,
    };
    let result = classifier.walk(node, flags, found);
    let steps = classifier.steps;
    builder.table.cost += steps;
    result
}

/// Parses each string type definition found, once, and records it; returns the new ones' indices.
fn register<D: Doc, R: Doc>(
    builder: &mut Builder,
    root: &Node<'_, R>,
    found: &[Node<'_, D>],
) -> Result<Vec<usize>, BuildError> {
    let mut added = Vec::new();
    for node in found {
        let range = node.range();
        if builder
            .table
            .annotations
            .iter()
            .any(|annotation| annotation.start == range.start && annotation.end == range.end)
        {
            continue;
        }
        let index = builder.table.annotations.len();
        let scope = builder
            .strings
            .get(&(range.start, range.end))
            .copied()
            .unwrap_or(0);
        let scopes = host_scopes(builder, root, scope);
        let globals = |name: &str| {
            let current = &builder.table.scopes[scope as usize];
            (current.kind != ScopeKind::Module)
                .then(|| current.bindings.get(name))
                .flatten()
                .and_then(|bindings| {
                    bindings
                        .iter()
                        .filter(|binding| binding.origin == Origin::Global)
                        .map(|binding| binding.offset)
                        .min()
                })
        };
        let host = semantic::Host {
            scopes,
            globals: &globals,
            // Tables outside stubs are shared across target versions; Ruff's semantic checks read
            // the latest version where none is set (`parser_version`).
            minor: match builder.table.reading.python_minor {
                0 => u8::MAX,
                minor => minor,
            },
        };
        let Some(tree) = annotation::parse(node, annotation_offset(index), &host) else {
            continue;
        };
        builder.table.cost += tree.len();
        if builder.table.cost > builder.cap {
            return Err(BuildError::Budget);
        }
        builder.table.annotations.push(Annotation {
            start: range.start,
            end: range.end,
            tree: Arc::new(tree),
            scope,
        });
        added.push(index);
    }
    Ok(added)
}

/// The scopes around a string in the table's scope `scope`, outermost first, as Ruff's semantic
/// checks tell them apart: a function's `async` and a comprehension's kind are read from the
/// source (a comprehension inside a string type definition reads as a list's).
fn host_scopes<R: Doc>(builder: &Builder, root: &Node<'_, R>, scope: u32) -> Vec<semantic::Scope> {
    let mut scopes = Vec::new();
    let mut current = Some(scope);
    while let Some(index) = current {
        let found = &builder.table.scopes[index as usize];
        let node = || node_at(root, &found.range);
        scopes.push(match found.kind {
            ScopeKind::Class => semantic::Scope::Class,
            ScopeKind::Function => semantic::Scope::Function {
                is_async: node().is_some_and(|node| node.text().starts_with("async")),
            },
            ScopeKind::Lambda => semantic::Scope::Lambda,
            ScopeKind::TypeParams => semantic::Scope::Type,
            ScopeKind::Comprehension => {
                let node = node();
                semantic::Scope::Generator {
                    kind: node
                        .as_ref()
                        .and_then(|node| semantic::Comprehension::of(&node.kind()))
                        .unwrap_or(semantic::Comprehension::List),
                    is_async: node.is_some_and(|node| {
                        named(&node)
                            .filter(|clause| clause.kind() == "for_in_clause")
                            .any(|clause| clause.children().any(|token| token.kind() == "async"))
                    }),
                }
            }
            _ => semantic::Scope::Module,
        });
        current = found.parent;
    }
    scopes.reverse();
    scopes
}

/// The node of `root`'s document whose range is `range`.
fn node_at<'r, D: Doc>(root: &Node<'r, D>, range: &std::ops::Range<usize>) -> Option<Node<'r, D>> {
    let mut node = root.clone();
    loop {
        if node.range() == *range {
            return Some(node);
        }
        let next = node.children().find(|child| {
            let found = child.range();
            found.start <= range.start && range.end <= found.end
        })?;
        node = next;
    }
}

/// Reads the names of the given annotations in their scopes, as deferred forward references.
fn read(builder: &mut Builder, indices: &[usize], reading: Reading) -> Visit {
    for index in indices {
        let (tree, scope) = {
            let annotation = &builder.table.annotations[*index];
            (Arc::clone(&annotation.tree), annotation.scope)
        };
        if tree.is_error() {
            continue;
        }
        let document = AstGrep::doc(SubDoc { tree });
        let saved = (
            builder.current,
            builder.deferring,
            builder.forwarding,
            builder.position,
            builder.branch,
        );
        builder.current = scope;
        builder.deferring = true;
        builder.forwarding = true;
        builder.position = None;
        let mut visitor = Visitor::new(builder, reading);
        let visited = visitor.visit(&document.root());
        (
            builder.current,
            builder.deferring,
            builder.forwarding,
            builder.position,
            builder.branch,
        ) = saved;
        visited?;
    }
    Ok(())
}
