//! Python bindings and references, after Ruff's `Checker` visitor.

use super::options::{DEFAULT_PYTHON_MINOR, python_builtins};
use super::{
    Access, Binding, BuildError, Builder, Form, Grammar, Import, ImportKind, NodeRef, Origin,
    Reading, Reference, ScopeKind, ScopeTable, Site, VALUE,
};
use crate::code_rule_dsl::identifier::python_identifier as identifier;
use crate::code_rule_dsl::language::GrammarDialect;
use ast_grep_core::{Doc, Node};
use std::sync::OnceLock;

mod typedefs;

use super::options::PythonTyping;
use typedefs::{
    RootKind, SUBSCRIPT_MEMBERS, SubscriptKind, TYPING_CALLABLES, last_segment, subscript_kind,
    typing as typing_member,
};
pub(super) use typedefs::{TypeRoot, string_type_definitions};

#[derive(Clone, Copy, Default, PartialEq)]
enum Tag {
    #[default]
    Other,
    Identifier,
    Comment,
    FunctionDefinition,
    ClassDefinition,
    Lambda,
    Parameters,
    DefaultParameter,
    TypedParameter,
    TypedDefaultParameter,
    ListSplatPattern,
    DictionarySplatPattern,
    TypeParameter,
    TypeDefault,
    Type,
    ConstrainedType,
    SplatType,
    GenericType,
    TypeAliasStatement,
    Assignment,
    AugmentedAssignment,
    ForStatement,
    NamedExpression,
    Comprehension,
    ForInClause,
    AsPattern,
    AsPatternTarget,
    ImportStatement,
    ImportFromStatement,
    FutureImportStatement,
    DottedName,
    AliasedImport,
    RelativeImport,
    GlobalStatement,
    NonlocalStatement,
    Attribute,
    KeywordArgument,
    CaseClause,
    CasePattern,
    ClassPattern,
    KeywordPattern,
    SplatPattern,
    DictPattern,
    /// Target shapes that only group other targets.
    TargetGroup,
    ConditionalExpression,
    /// Statements and clauses whose bodies may hold a `global` statement of the same function.
    Compound,
    /// Compound statements and clauses that open Ruff branches, or decide whether a `del` is
    /// conditional; their bodies may hold `global` statements too.
    If,
    Elif,
    Else,
    Try,
    Except,
    Finally,
    With,
    While,
    Match,
    Delete,
    WildcardImport,
    /// String literals, whose scope a string type definition is read in.
    String,
    Call,
    Subscript,
}

fn tag(name: &str) -> Tag {
    match name {
        "identifier" => Tag::Identifier,
        "comment" => Tag::Comment,
        "function_definition" => Tag::FunctionDefinition,
        "class_definition" => Tag::ClassDefinition,
        "lambda" => Tag::Lambda,
        "parameters" | "lambda_parameters" => Tag::Parameters,
        "default_parameter" => Tag::DefaultParameter,
        "typed_parameter" => Tag::TypedParameter,
        "typed_default_parameter" => Tag::TypedDefaultParameter,
        "list_splat_pattern" | "list_splat" => Tag::ListSplatPattern,
        "dictionary_splat_pattern" => Tag::DictionarySplatPattern,
        "type_parameter" => Tag::TypeParameter,
        "type_default" => Tag::TypeDefault,
        "type" => Tag::Type,
        "constrained_type" => Tag::ConstrainedType,
        "splat_type" => Tag::SplatType,
        "generic_type" => Tag::GenericType,
        "type_alias_statement" => Tag::TypeAliasStatement,
        "assignment" => Tag::Assignment,
        "augmented_assignment" => Tag::AugmentedAssignment,
        "for_statement" => Tag::ForStatement,
        "named_expression" => Tag::NamedExpression,
        "list_comprehension"
        | "set_comprehension"
        | "dictionary_comprehension"
        | "generator_expression" => Tag::Comprehension,
        "for_in_clause" => Tag::ForInClause,
        "as_pattern" => Tag::AsPattern,
        "as_pattern_target" => Tag::AsPatternTarget,
        "import_statement" => Tag::ImportStatement,
        "import_from_statement" => Tag::ImportFromStatement,
        "future_import_statement" => Tag::FutureImportStatement,
        "dotted_name" => Tag::DottedName,
        "aliased_import" => Tag::AliasedImport,
        "relative_import" => Tag::RelativeImport,
        "global_statement" => Tag::GlobalStatement,
        "nonlocal_statement" => Tag::NonlocalStatement,
        "attribute" => Tag::Attribute,
        "keyword_argument" => Tag::KeywordArgument,
        "case_clause" => Tag::CaseClause,
        "case_pattern" | "union_pattern" | "list_pattern" | "tuple_pattern" => Tag::CasePattern,
        "class_pattern" => Tag::ClassPattern,
        "keyword_pattern" => Tag::KeywordPattern,
        "splat_pattern" => Tag::SplatPattern,
        "dict_pattern" => Tag::DictPattern,
        "pattern_list" | "tuple" | "list" | "expression_list" | "parenthesized_expression" => {
            Tag::TargetGroup
        }
        "conditional_expression" => Tag::ConditionalExpression,
        "block" | "decorated_definition" => Tag::Compound,
        "if_statement" => Tag::If,
        "elif_clause" => Tag::Elif,
        "else_clause" => Tag::Else,
        "try_statement" => Tag::Try,
        "except_clause" | "except_group_clause" => Tag::Except,
        "finally_clause" => Tag::Finally,
        "with_statement" => Tag::With,
        "while_statement" => Tag::While,
        "match_statement" => Tag::Match,
        "delete_statement" => Tag::Delete,
        "wildcard_import" => Tag::WildcardImport,
        "string" | "concatenated_string" => Tag::String,
        "call" => Tag::Call,
        "subscript" => Tag::Subscript,
        _ => Tag::Other,
    }
}

struct Fields {
    name: u16,
    alias: u16,
    left: u16,
    right: u16,
    value: u16,
    body: u16,
    parameters: u16,
    type_parameters: u16,
    return_type: u16,
    superclasses: u16,
    ty: u16,
    object: u16,
    alternative: u16,
    module_name: u16,
    condition: u16,
    consequence: u16,
}

struct Python {
    grammar: Grammar<Tag>,
    fields: Fields,
}

fn python() -> &'static Python {
    static PYTHON: OnceLock<Python> = OnceLock::new();
    PYTHON.get_or_init(|| {
        let grammar = Grammar::new(GrammarDialect::Python, GrammarDialect::Python, tag);
        let fields = Fields {
            name: grammar.field("name"),
            alias: grammar.field("alias"),
            left: grammar.field("left"),
            right: grammar.field("right"),
            value: grammar.field("value"),
            body: grammar.field("body"),
            parameters: grammar.field("parameters"),
            type_parameters: grammar.field("type_parameters"),
            return_type: grammar.field("return_type"),
            superclasses: grammar.field("superclasses"),
            ty: grammar.field("type"),
            object: grammar.field("object"),
            alternative: grammar.field("alternative"),
            module_name: grammar.field("module_name"),
            condition: grammar.field("condition"),
            consequence: grammar.field("consequence"),
        };
        Python { grammar, fields }
    })
}

type Visit = Result<(), BuildError>;

fn named<'a, 'r, D: Doc>(node: &'a Node<'r, D>) -> impl Iterator<Item = Node<'r, D>> + 'a {
    node.children().filter(|child| child.is_named())
}

fn fields() -> &'static Fields {
    &python().fields
}

fn field<'r, D: Doc>(node: &Node<'r, D>, id: u16) -> Option<Node<'r, D>> {
    if id == 0 {
        None
    } else {
        node.child_by_field_id(id)
    }
}

fn same<D: Doc>(left: &Node<'_, D>, right: Option<&Node<'_, D>>) -> bool {
    right.is_some_and(|right| right.node_id() == left.node_id())
}

/// Builds a Python file's table: Ruff's visit of the module, then its string type definitions.
pub(crate) fn visit<D: Doc>(builder: &mut Builder, root: &Node<'_, D>, reading: Reading) -> Visit {
    visit_module(builder, root, reading)?;
    builder.table.sort_references();
    string_type_definitions(builder, root, reading)
}

/// Orders each name's bindings as Ruff adds them: the body in order, then what function bodies
/// bind into the scope (`nonlocal`, placeholders). Marks what a nested `nonlocal` rebinds.
pub(crate) fn finish<D: Doc>(table: &mut ScopeTable, root: &Node<'_, D>) {
    for scope in &mut table.scopes {
        for bindings in scope.bindings.values_mut() {
            bindings.sort_by_key(|binding| (binding.deferred, binding.offset));
        }
    }
    if table
        .references
        .iter()
        .any(|reference| reference.access == Access::Nonlocal)
    {
        table.mark_rebound(&root.text(), root.range().start);
    }
}

/// Visits a module; `deferred_annotations` reads every annotation after it, as
/// `from __future__ import annotations` does; a stub also reads its other type definitions and
/// its class bases after it.
pub(super) fn visit_module<D: Doc>(
    builder: &mut Builder,
    root: &Node<'_, D>,
    reading: Reading,
) -> Visit {
    let mut visitor = Visitor::new(builder, reading);
    visitor.children(root)?;
    // Declared in function bodies, which Ruff visits after the module: any module binding by then
    // (a bare annotation binds no value) keeps the placeholder out.
    for name in std::mem::take(&mut visitor.deferred_globals) {
        if !visitor
            .builder
            .scope(0)
            .bindings
            .get(name.as_str())
            .is_some_and(|bindings| {
                bindings
                    .iter()
                    .any(|binding| binding.site.form != Form::Annotation)
            })
        {
            visitor.declare(&name, root.range().end, true);
        }
    }
    Ok(())
}

struct Visitor<'b, 'd> {
    builder: &'b mut Builder<'d>,
    py: &'static Python,
    future_annotations: bool,
    /// A `.pyi` stub, which Ruff reads typing-aware: see `typing_aware`.
    stub: bool,
    /// The target version's minor, for a stub's builtins and TypedDict's keyword fields.
    minor: u8,
    /// Names nested functions and classes declare `global`, settled after the module.
    deferred_globals: Vec<String>,
    /// The end of the `except` handler whose name is being bound.
    until: Option<usize>,
    /// The values unpacking pairs with the targets being bound, by the target name's start.
    unpacked: Vec<(usize, NodeRef)>,
    /// Inside a `Literal[...]` slice, where Ruff defers nothing (`in_typing_literal`).
    in_literal: bool,
    /// Inside a subscript that is assigned or deleted, which Ruff walks without reading it as a
    /// typing subscript (`in_subscript`).
    store_subscript: bool,
}

impl<'b, 'd> Visitor<'b, 'd> {
    fn new(builder: &'b mut Builder<'d>, reading: Reading) -> Self {
        Self {
            builder,
            py: python(),
            future_annotations: reading.deferred_annotations || reading.stub,
            stub: reading.stub,
            minor: match reading.python_minor {
                0 => DEFAULT_PYTHON_MINOR,
                minor => minor,
            },
            deferred_globals: Vec::new(),
            until: None,
            unpacked: Vec::new(),
            in_literal: false,
            store_subscript: false,
        }
    }
}

impl Visitor<'_, '_> {
    fn tag<D: Doc>(&self, node: &Node<'_, D>) -> Tag {
        self.py.grammar.tag(node)
    }

    fn children<D: Doc>(&mut self, node: &Node<'_, D>) -> Visit {
        for child in named(node) {
            self.visit(&child)?;
        }
        Ok(())
    }

    fn visit_field<D: Doc>(&mut self, node: &Node<'_, D>, id: u16) -> Visit {
        match field(node, id) {
            Some(child) => self.visit(&child),
            None => Ok(()),
        }
    }

    /// An expression or statement evaluated in the current scope.
    fn visit<D: Doc>(&mut self, node: &Node<'_, D>) -> Visit {
        self.builder.enter()?;
        let fields = fields();
        match self.tag(node) {
            Tag::Identifier => self.builder.reference(node.range(), VALUE),
            Tag::Comment => {}
            Tag::FunctionDefinition => self.function(node)?,
            Tag::ClassDefinition => self.class(node)?,
            Tag::Lambda => self.lambda(node)?,
            Tag::TypeAliasStatement => self.type_alias(node)?,
            Tag::Assignment => {
                self.dunder_all(node);
                // `X: TypeAlias = value`: the value is a type definition, and a stub reads it
                // after the module.
                if let (Some(annotation), Some(value)) =
                    (field(node, fields.ty), field(node, fields.right))
                    && identifier(&annotation).ends_with("TypeAlias")
                {
                    self.builder.type_roots.push(TypeRoot {
                        node: NodeRef::of(&value),
                        kind: RootKind::AliasValue(annotation_ref(&annotation)),
                    });
                }
                if self.typing_aware()
                    && let Some(annotation) = field(node, fields.ty)
                    && self
                        .qualified(&annotation)
                        .is_some_and(|name| typing_member(&name, "TypeAlias", self.settings()))
                {
                    self.lazily(|visitor| visitor.visit_field(node, fields.right))?;
                } else {
                    self.visit_field(node, fields.right)?;
                }
                self.annotation(node, fields.ty)?;
                if let Some(left) = field(node, fields.left) {
                    if field(node, fields.right).is_some() {
                        let site = self.assignment_site(node, &left);
                        if site.value.is_none()
                            && let Some(value) = innermost_value(node)
                        {
                            unpack(&left, &value, &mut self.unpacked);
                        }
                        let bound = self.targets(&left, node.range().end, site);
                        self.unpacked.clear();
                        bound?;
                    } else {
                        // `x: int` and `(x): int` bind an annotation; `a.b: int` loads `a`.
                        let target =
                            unparenthesized(&left, &["parenthesized_expression", "tuple_pattern"]);
                        if self.tag(&target) == Tag::Identifier {
                            self.annotate(node, &target);
                        } else {
                            self.visit(&left)?;
                        }
                    }
                }
            }
            Tag::AugmentedAssignment => {
                self.dunder_all(node);
                self.visit_field(node, fields.right)?;
                if let Some(left) = field(node, fields.left) {
                    self.store(&left)?;
                    if self.tag(&left) == Tag::Identifier {
                        let site = site(Form::Augmented, node);
                        self.bind_here(&left, node.range().end, None, site);
                    }
                }
            }
            Tag::ForStatement => {
                let right = field(node, fields.right);
                if let Some(right) = &right {
                    self.visit(right)?;
                }
                if let Some(left) = field(node, fields.left) {
                    let offset = right.map_or(left.range().end, |right| right.range().end);
                    self.targets(&left, offset, site(Form::For, node))?;
                }
                self.visit_field(node, fields.body)?;
                self.visit_field(node, fields.alternative)?;
            }
            Tag::NamedExpression => {
                self.visit_field(node, fields.value)?;
                if let Some(name) = field(node, fields.name) {
                    let mut scope = self.builder.current;
                    while self.builder.scope(scope).kind == ScopeKind::Comprehension {
                        scope = self.builder.scope(scope).parent.unwrap_or(0);
                    }
                    let site = Site {
                        value: field(node, fields.value).map(|value| NodeRef::of(&value)),
                        ..site(Form::Named, node)
                    };
                    // In code read after the module (an annotation under `from __future__ import
                    // annotations` or 3.14, a stub's type definition), the walrus binds only
                    // then: later code in the body does not see it.
                    let binding = Binding {
                        deferred: self.builder.deferring,
                        ..self.binding(node.range().end, None, site, NodeRef::of(&name))
                    };
                    self.bind_name(scope, &identifier(&name), binding);
                }
            }
            Tag::Comprehension => self.comprehension(node)?,
            Tag::AsPattern => {
                // `with a as b` and `except E as e`; case patterns have their own walk. Python
                // deletes an `except` name when its handler ends.
                let alias = field(node, fields.alias);
                let handler = node.parent().filter(|parent| {
                    matches!(
                        parent.kind().as_ref(),
                        "except_clause" | "except_group_clause"
                    )
                });
                let form = if handler.is_some() {
                    Form::Except
                } else {
                    Form::With
                };
                for child in named(node) {
                    if same(&child, alias.as_ref()) {
                        self.until = handler.as_ref().map(|handler| handler.range().end);
                        let bound = self.targets(&child, node.range().end, site(form, node));
                        self.until = None;
                        bound?;
                    } else {
                        self.visit(&child)?;
                    }
                }
            }
            Tag::ImportStatement => self.import(node),
            Tag::ImportFromStatement | Tag::FutureImportStatement => self.import_from(node),
            Tag::GlobalStatement => {
                // Ruff binds the name in the declaring scope only; the module is not rebound.
                let current = self.builder.current;
                if self.builder.scope(current).kind != ScopeKind::Module {
                    let names = named(node).filter(|child| self.tag(child) == Tag::Identifier);
                    for child in names.collect::<Vec<_>>() {
                        let binding = Binding {
                            origin: Origin::Global,
                            name: Some(NodeRef::of(&child)),
                            branch: self.builder.branch,
                            outer: true,
                            ..Binding::new(node.range().start, VALUE, site(Form::Global, node))
                        };
                        self.builder.bind(current, &identifier(&child), binding);
                    }
                }
            }
            Tag::NonlocalStatement => {
                // Each name also reads the enclosing binding, which Ruff marks used.
                for child in named(node) {
                    if self.tag(&child) != Tag::Identifier {
                        continue;
                    }
                    let scope = &mut self.builder.table.scopes[self.builder.current as usize];
                    scope.nonlocals.insert(identifier(&child).as_ref().into());
                    self.builder.access_reference(
                        child.range(),
                        VALUE,
                        false,
                        false,
                        Access::Nonlocal,
                    );
                }
            }
            Tag::If => {
                let condition = field(node, fields.condition);
                for child in named(node) {
                    if same(&child, condition.as_ref()) || self.tag(&child) == Tag::Comment {
                        self.visit(&child)?;
                    } else {
                        self.in_branch(None, |visitor| visitor.visit(&child))?;
                    }
                }
            }
            Tag::Try => {
                // The body and the `else` share a branch; each handler and the `finally` get one.
                let body = field(node, fields.body);
                let mut shared = None;
                for child in named(node) {
                    match self.tag(&child) {
                        Tag::Comment => self.visit(&child)?,
                        Tag::Else => {
                            let branch = shared;
                            shared = Some(self.in_branch(branch, |visitor| visitor.visit(&child))?);
                        }
                        _ if same(&child, body.as_ref()) => {
                            let branch = shared;
                            shared = Some(self.in_branch(branch, |visitor| visitor.visit(&child))?);
                        }
                        _ => {
                            self.in_branch(None, |visitor| visitor.visit(&child))?;
                        }
                    }
                }
            }
            Tag::With => {
                let body = field(node, fields.body);
                for child in named(node) {
                    if same(&child, body.as_ref()) {
                        self.in_branch(None, |visitor| visitor.visit(&child))?;
                    } else {
                        self.visit(&child)?;
                    }
                }
            }
            Tag::Delete => {
                for child in named(node) {
                    self.delete_target(node, &child)?;
                }
            }
            Tag::WildcardImport => {}
            Tag::String => {
                let range = node.range();
                self.builder
                    .strings
                    .insert((range.start, range.end), self.builder.current);
                self.children(node)?;
            }
            // A typing callable's or generic's arguments may be type definitions.
            Tag::Call | Tag::Subscript | Tag::GenericType => {
                let head = match self.tag(node) {
                    Tag::Call => node.field("function"),
                    Tag::Subscript => node.field("value"),
                    _ => named(node).next(),
                };
                let call = self.tag(node) == Tag::Call;
                let settings = self.settings();
                let member = |name: &str| {
                    if call {
                        TYPING_CALLABLES.contains(&name)
                    } else {
                        SUBSCRIPT_MEMBERS.contains(&name)
                            || settings.is_some_and(|settings| settings.generic_member(name))
                    }
                };
                if head.is_some_and(|head| {
                    member(last_segment(&head).as_str()) || self.imports_member(&head, &member)
                }) {
                    self.builder.type_roots.push(TypeRoot {
                        node: NodeRef::of(node),
                        kind: RootKind::Candidate,
                    });
                }
                match self.tag(node) {
                    Tag::Call => {
                        // `locals()` marks its own scope (Ruff's `set_uses_locals`), whatever
                        // `locals` is.
                        if node
                            .field("function")
                            .map(|function| {
                                unparenthesized(&function, &["parenthesized_expression"])
                            })
                            .is_some_and(|function| {
                                self.tag(&function) == Tag::Identifier
                                    && identifier(&function) == "locals"
                            })
                        {
                            let current = self.builder.current;
                            self.builder.table.locals_scopes.push(current);
                        }
                        if self.typing_aware() {
                            self.typing_call(node)?;
                        } else {
                            self.children(node)?;
                        }
                    }
                    Tag::Subscript if self.typing_aware() && !self.store_subscript => {
                        self.typing_subscript(node)?;
                    }
                    _ => self.children(node)?,
                }
            }
            Tag::ConditionalExpression => {
                // `body if test else orelse` runs the test first (Ruff visits it first too).
                let parts = named(node)
                    .filter(|part| self.tag(part) != Tag::Comment)
                    .collect::<Vec<_>>();
                if let [body, test, rest @ ..] = parts.as_slice() {
                    self.visit(test)?;
                    self.read_after(test.range().end, |visitor| visitor.visit(body))?;
                    for part in rest {
                        self.visit(part)?;
                    }
                } else {
                    self.children(node)?;
                }
            }
            Tag::Attribute => self.visit_field(node, fields.object)?,
            Tag::KeywordArgument => self.visit_field(node, fields.value)?,
            Tag::CaseClause => {
                let offset = node.range().start;
                let consequence = field(node, fields.consequence);
                for child in named(node) {
                    if self.tag(&child) == Tag::CasePattern {
                        self.case_pattern(&child, offset)?;
                    } else if same(&child, consequence.as_ref()) {
                        self.in_branch(None, |visitor| visitor.visit(&child))?;
                    } else {
                        self.visit(&child)?;
                    }
                }
            }
            Tag::DottedName => {
                // Outside imports and patterns: only the head names a binding.
                if let Some(head) = named(node).next() {
                    self.visit(&head)?;
                }
            }
            _ => self.children(node)?,
        }
        self.builder.leave();
        Ok(())
    }

    /// Binds `name` where an assignment in the current scope lands: the nearest enclosing function
    /// for a `nonlocal` name, else the current scope (a `global` name too, as in Ruff).
    fn bind_here<D: Doc>(
        &mut self,
        name: &Node<'_, D>,
        offset: usize,
        import: Option<Import>,
        site: Site,
    ) {
        self.bind_in(self.builder.current, name, offset, import, site);
    }

    fn bind_in<D: Doc>(
        &mut self,
        scope: u32,
        name: &Node<'_, D>,
        offset: usize,
        import: Option<Import>,
        site: Site,
    ) {
        let text = identifier(name);
        let binding = self.binding(offset, import, site, NodeRef::of(name));
        self.bind_name(scope, &text, binding);
    }

    /// A binding made here, named by the node `name`.
    fn binding(&self, offset: usize, import: Option<Import>, site: Site, name: NodeRef) -> Binding {
        Binding {
            import,
            until: self.until,
            name: Some(name),
            branch: self.builder.branch,
            ..Binding::new(offset, VALUE, site)
        }
    }

    /// Runs `visit` in a new Ruff branch, or in `branch` (a `try` body's and its `else`'s);
    /// returns the branch it ran in.
    fn in_branch(
        &mut self,
        branch: Option<u32>,
        visit: impl FnOnce(&mut Self) -> Visit,
    ) -> Result<u32, BuildError> {
        let outer = self.builder.branch;
        let inner = branch.unwrap_or_else(|| {
            self.builder.branches += 1;
            self.builder.branches
        });
        self.builder.branch = inner;
        let visited = visit(self);
        self.builder.branch = outer;
        visited.map(|()| inner)
    }

    /// A bare annotation `x: T` (Ruff's `Annotation` binding, which does not count as assigned):
    /// added only where its scope binds no such name yet, a `nonlocal` declaration included
    /// (Ruff then keeps it as a delayed annotation, outside the scope).
    fn annotate<D: Doc>(&mut self, statement: &Node<'_, D>, name: &Node<'_, D>) {
        let scope = self.builder.current;
        let text = identifier(name);
        let offset = statement.range().end;
        let declared = self.builder.scope(scope);
        if declared.nonlocals.contains(text.as_ref())
            || declared
                .bindings
                .get(text.as_ref())
                .is_some_and(|bindings| {
                    bindings
                        .iter()
                        .any(|earlier| !earlier.deferred && earlier.offset <= offset)
                })
        {
            return;
        }
        let site = Site {
            annotation: field(statement, fields().ty).as_ref().map(annotation_ref),
            ..site(Form::Annotation, statement)
        };
        let binding = self.binding(offset, None, site, NodeRef::of(name));
        self.bind_name(scope, &text, binding);
    }

    /// Records a module-level `__all__ =`, `+=` or `: T =` (Ruff's `in_dunder_all_assignment`).
    /// Of a chained `a = b = v`, only the first target counts.
    fn dunder_all<D: Doc>(&mut self, node: &Node<'_, D>) {
        if self.builder.current == 0
            && field(node, fields().right).is_some()
            && node
                .parent()
                .is_none_or(|parent| parent.kind() != "assignment")
            && field(node, fields().left).is_some_and(|left| {
                self.tag(&left) == Tag::Identifier && identifier(&left) == "__all__"
            })
        {
            self.builder.table.dunder_all.push(NodeRef::of(node));
        }
    }

    /// A `del` target: a name is read in the current scope only and, unless the `del` is
    /// conditional, unbound (Ruff's `handle_node_delete`); attributes and subscripts are loads.
    fn delete_target<D: Doc>(&mut self, statement: &Node<'_, D>, target: &Node<'_, D>) -> Visit {
        match self.tag(target) {
            Tag::Identifier => {
                // Ruff resolves a `del` in place, stubs included.
                let deferring = std::mem::replace(&mut self.builder.deferring, false);
                self.builder
                    .access_reference(target.range(), VALUE, false, false, Access::Del);
                self.builder.deferring = deferring;
                if !conditional_delete(statement) {
                    let binding = Binding {
                        name: Some(NodeRef::of(target)),
                        branch: self.builder.branch,
                        ..Binding::new(target.range().end, VALUE, site(Form::Deletion, statement))
                    };
                    let current = self.builder.current;
                    self.builder.bind(current, &identifier(target), binding);
                }
                Ok(())
            }
            Tag::TargetGroup => {
                self.builder.enter()?;
                for child in named(target) {
                    self.delete_target(statement, &child)?;
                }
                self.builder.leave();
                Ok(())
            }
            _ => self.store(target),
        }
    }

    /// Where `x = v` (or `x: T = v`) binds `left`: the value is the innermost right-hand side
    /// (`a = b = v` gives both `v`), for a plain name (Ruff's `find_binding_value`; unpacked
    /// targets take their element, see `unpack`).
    fn assignment_site<D: Doc>(&self, node: &Node<'_, D>, left: &Node<'_, D>) -> Site {
        let fields = fields();
        let value = innermost_value(node);
        // `(x) = v` is a plain name; tree-sitter reads the parentheses as a tuple pattern.
        let mut target = left.clone();
        while matches!(
            target.kind().as_ref(),
            "parenthesized_expression" | "tuple_pattern"
        ) && !target.children().any(|child| child.kind() == ",")
        {
            match named(&target)
                .filter(|child| self.tag(child) != Tag::Comment)
                .collect::<Vec<_>>()
                .as_slice()
            {
                [inner] => target = inner.clone(),
                _ => break,
            }
        }
        let plain = self.tag(&target) == Tag::Identifier;
        let annotation = field(node, fields.ty);
        Site {
            form: if annotation.is_some() {
                Form::Annotated
            } else {
                Form::Assignment
            },
            node: Some(NodeRef::of(node)),
            value: value.filter(|_| plain).map(|value| NodeRef::of(&value)),
            annotation: annotation.as_ref().map(annotation_ref),
        }
    }

    fn bind_name(&mut self, scope: u32, text: &str, binding: Binding) {
        let declared = self.builder.scope(scope);
        let (target, redirected) = if declared.nonlocals.contains(text) {
            let mut outer = declared.parent;
            while let Some(index) = outer {
                let kind = self.builder.scope(index).kind;
                if matches!(kind, ScopeKind::Function | ScopeKind::Lambda) {
                    break;
                }
                outer = self.builder.scope(index).parent;
            }
            (outer.unwrap_or(scope), outer.is_some())
        } else {
            (scope, false)
        };
        // A name its scope declares `global` stays marked (Ruff copies the flag down the chain).
        let global = declared.bindings.get(text).is_some_and(|bindings| {
            bindings
                .iter()
                .any(|earlier| earlier.origin == Origin::Global && earlier.offset <= binding.offset)
        });
        // A method's `nonlocal __class__` names the class cell (Ruff's `DunderClassCell` scope
        // binds it): the binding is `nonlocal`, though no enclosing function holds it.
        let cell = !redirected
            && text == "__class__"
            && declared.nonlocals.contains(text)
            && self.encloses_cell(scope);
        let outer = binding.outer || redirected || global || cell;
        self.builder.bind(
            target,
            text,
            Binding {
                deferred: binding.deferred || redirected,
                outer,
                ..binding
            },
        );
    }

    /// Whether `scope` is a method: a function whose scope is a class body's, type parameters
    /// aside, so Python gives it a `__class__` cell.
    fn encloses_cell(&self, scope: u32) -> bool {
        if self.builder.scope(scope).kind != ScopeKind::Function {
            return false;
        }
        let mut parent = self.builder.scope(scope).parent;
        while let Some(index) =
            parent.filter(|index| self.builder.scope(*index).kind == ScopeKind::TypeParams)
        {
            parent = self.builder.scope(index).parent;
        }
        parent.is_some_and(|index| self.builder.scope(index).kind == ScopeKind::Class)
    }

    /// Whether a function or lambda body lies between `inner` and its ancestor `outer`: bindings
    /// made there are visited after the enclosing body.
    fn crosses_function(&self, inner: u32, outer: u32) -> bool {
        let mut current = Some(inner);
        while let Some(index) = current.filter(|index| *index != outer) {
            let scope = self.builder.scope(index);
            if matches!(scope.kind, ScopeKind::Function | ScopeKind::Lambda) {
                return true;
            }
            current = scope.parent;
        }
        false
    }

    /// Assignment targets: names bind, attributes and subscripts are loads.
    fn targets<D: Doc>(&mut self, node: &Node<'_, D>, offset: usize, site: Site) -> Visit {
        match self.tag(node) {
            Tag::Identifier => {
                let start = node.range().start;
                let value = self
                    .unpacked
                    .iter()
                    .find(|(at, _)| *at == start)
                    .map(|(_, value)| *value);
                let site = Site {
                    value: site.value.or(value),
                    ..site
                };
                self.bind_here(node, offset, None, site);
                Ok(())
            }
            Tag::TargetGroup | Tag::ListSplatPattern | Tag::AsPatternTarget | Tag::CasePattern => {
                self.builder.enter()?;
                for child in named(node) {
                    self.targets(&child, offset, site)?;
                }
                self.builder.leave();
                Ok(())
            }
            _ => self.store(node),
        }
    }

    /// An assigned or deleted target that is no name: a subscript's parts are read as Ruff walks
    /// them, not as a typing subscript.
    fn store<D: Doc>(&mut self, node: &Node<'_, D>) -> Visit {
        if self.tag(node) != Tag::Subscript {
            return self.visit(node);
        }
        let store = std::mem::replace(&mut self.store_subscript, true);
        let visited = self.visit(node);
        self.store_subscript = store;
        visited
    }

    /// Parameter names of a `parameters` or `lambda_parameters` node, bound at `offset`.
    fn bind_parameters<D: Doc>(&mut self, parameters: &Node<'_, D>, offset: usize) {
        let fields = fields();
        for parameter in named(parameters) {
            let name = match self.tag(&parameter) {
                Tag::Identifier => Some(parameter.clone()),
                Tag::DefaultParameter | Tag::TypedDefaultParameter => {
                    field(&parameter, fields.name)
                }
                Tag::TypedParameter => {
                    let ty = field(&parameter, fields.ty);
                    named(&parameter).find(|child| !same(child, ty.as_ref()))
                }
                Tag::ListSplatPattern | Tag::DictionarySplatPattern => Some(parameter.clone()),
                _ => None,
            };
            let Some(name) = name else { continue };
            let name = if matches!(
                self.tag(&name),
                Tag::ListSplatPattern | Tag::DictionarySplatPattern
            ) {
                named(&name).next()
            } else {
                Some(name)
            };
            let Some(name) = name else { continue };
            if self.tag(&name) == Tag::Identifier {
                let annotation = match self.tag(&parameter) {
                    Tag::TypedParameter | Tag::TypedDefaultParameter => {
                        field(&parameter, fields.ty)
                    }
                    _ => None,
                };
                let site = Site {
                    annotation: annotation.as_ref().map(annotation_ref),
                    ..site(Form::Parameter, &parameter)
                };
                self.bind_here(&name, offset, None, site);
            }
        }
    }

    /// Defaults, which run in the enclosing scope when the function is defined.
    fn visit_defaults<D: Doc>(&mut self, parameters: &Node<'_, D>) -> Visit {
        for parameter in named(parameters) {
            if matches!(
                self.tag(&parameter),
                Tag::DefaultParameter | Tag::TypedDefaultParameter
            ) {
                self.visit_field(&parameter, fields().value)?;
            }
        }
        Ok(())
    }

    /// Annotations, which run in the enclosing (or type-parameter) scope.
    fn visit_annotations<D: Doc>(&mut self, parameters: &Node<'_, D>) -> Visit {
        for parameter in named(parameters) {
            if matches!(
                self.tag(&parameter),
                Tag::TypedParameter | Tag::TypedDefaultParameter
            ) {
                self.annotation(&parameter, fields().ty)?;
            }
        }
        Ok(())
    }

    /// Visits what Ruff reads in the type scope it opens around a class or function definition.
    fn in_type_scope(&mut self, visit: impl FnOnce(&mut Self) -> Visit) -> Visit {
        let typing = std::mem::replace(&mut self.builder.typing, true);
        let visited = visit(self);
        self.builder.typing = typing;
        visited
    }

    /// Visits code Ruff reads after the module: a type parameter's bound or default, a `type`
    /// alias's value (`visit_deferred_type_param_definitions`), and in a stub its class bases and
    /// type definitions (`visit_deferred_class_bases`, `visit_deferred_future_type_definitions`).
    fn lazily(&mut self, visit: impl FnOnce(&mut Self) -> Visit) -> Visit {
        let deferring = self.builder.deferring;
        self.builder.deferring = true;
        let visited = visit(self);
        self.builder.deferring = deferring;
        visited
    }

    /// Ruff's typing settings the table is read under.
    fn settings(&self) -> Option<&'static PythonTyping> {
        self.builder.table.reading.typing
    }

    /// Whether this node is read typing-aware, as Ruff reads a stub (`future_annotations_or_stub`
    /// with `is_stub`): a type definition outside an annotation is read after the module. Not
    /// inside what is read after the module already, nor inside a `Literal[...]` slice.
    fn typing_aware(&self) -> bool {
        self.stub && !self.builder.deferring && !self.in_literal
    }

    /// Visits `node` as a type definition (Ruff's `visit_type_definition`), or in place.
    fn type_definition<D: Doc>(&mut self, node: &Node<'_, D>, type_definition: bool) -> Visit {
        if type_definition {
            self.lazily(|visitor| visitor.visit(node))
        } else {
            self.visit(node)
        }
    }

    /// The qualified name `node` denotes where it is read (Ruff's `resolve_qualified_name`): a name
    /// or an attribute chain on one whose head resolves to an import, or an unshadowed builtin of
    /// the target version (`["", name, ...]`).
    fn qualified<D: Doc>(&self, node: &Node<'_, D>) -> Option<Vec<String>> {
        const WRAPPERS: &[&str] = &["parenthesized_expression", "type"];
        let mut tail = Vec::new();
        let mut head = unparenthesized(node, WRAPPERS);
        while self.tag(&head) == Tag::Attribute {
            tail.push(identifier(&head.field("attribute")?).into_owned());
            head = unparenthesized(&head.field("object")?, WRAPPERS);
        }
        if self.tag(&head) != Tag::Identifier {
            return None;
        }
        tail.reverse();
        let name = identifier(&head);
        let range = head.range();
        let reference = Reference {
            start: range.start,
            end: range.end,
            scope: self.builder.current,
            spaces: VALUE,
            at: self.builder.position.unwrap_or(range.start),
            deferred: self.builder.deferring,
            forward: self.builder.forwarding,
            type_scope: self.builder.typing,
            write: false,
            mutates: false,
            access: Access::Load,
        };
        match self.builder.table.resolve(&reference, &name) {
            Some(binding) if binding.shadows() => {
                let module = self.builder.module.as_ref();
                let qualified = match binding.import.as_ref()? {
                    Import::Qualified(qualified) => qualified.to_string(),
                    Import::Relative { level, rest } => module?.relative(*level, rest, &[])?,
                    Import::Definition => module?.definition(&name, &[])?,
                };
                Some(
                    qualified
                        .split('.')
                        .map(str::to_owned)
                        .chain(tail)
                        .collect(),
                )
            }
            _ => python_builtins()
                .get(&self.minor)
                .is_some_and(|builtins| builtins.contains(name.as_ref()))
                .then(|| {
                    std::iter::once(String::new())
                        .chain(std::iter::once(name.into_owned()))
                        .chain(tail)
                        .collect()
                }),
        }
    }

    /// A call a stub reads typing-aware: the callee first, then each argument, a type definition
    /// where Ruff's `Checker::visit_expr` reads one for `typing`'s special callables.
    fn typing_call<D: Doc>(&mut self, node: &Node<'_, D>) -> Visit {
        let Some(function) = node.field("function") else {
            return self.children(node);
        };
        self.visit(&function)?;
        let Some(arguments) = node.field("arguments") else {
            return Ok(());
        };
        let callable = self
            .qualified(&function)
            .and_then(|name| typing_callable(&name, self.settings()))
            .filter(|_| arguments.kind() == "argument_list");
        let Some(callable) = callable else {
            return self.visit(&arguments);
        };
        let items = named(&arguments)
            .filter(|item| self.tag(item) != Tag::Comment)
            .collect::<Vec<_>>();
        // Ruff's `Arguments::args` (positional and starred) and `keywords` (named and `**`).
        let keywords = items.iter().map(keyword_argument).collect::<Vec<_>>();
        let positional = items
            .iter()
            .filter(|item| {
                !matches!(
                    item.kind().as_ref(),
                    "keyword_argument" | "dictionary_splat"
                )
            })
            .collect::<Vec<_>>();
        let legacy_fields = self.minor < 13
            && match positional.as_slice() {
                [_] => true,
                [_, second] => {
                    unparenthesized(second, &["parenthesized_expression"]).kind() == "none"
                }
                _ => false,
            };
        let mut first_positional = true;
        for (index, (item, named_keyword)) in items.iter().zip(keywords).enumerate() {
            match (callable, named_keyword) {
                (Callable::Cast | Callable::NewType | Callable::TypeAliasType, Some((None, _))) => {
                    // Ruff does not visit a `**` argument of these.
                }
                (Callable::Cast, Some((Some(name), value))) => {
                    self.type_definition(&value, name == "typ")?;
                }
                (Callable::Cast, None) => self.type_definition(item, index == 0)?,
                (Callable::TypeForm, Some((_, value))) => self.type_definition(&value, false)?,
                (Callable::TypeForm, None) => self.type_definition(item, first_positional)?,
                (Callable::NewType, Some((Some(name), value))) => {
                    self.type_definition(&value, name == "tp")?;
                }
                (Callable::NewType, None) => self.type_definition(item, index == 1)?,
                (Callable::TypeVar, Some((name, value))) => {
                    let bound =
                        name.is_some_and(|name| matches!(name.as_str(), "bound" | "default"));
                    self.type_definition(&value, bound)?;
                }
                (Callable::TypeVar, None) => self.type_definition(item, !first_positional)?,
                (Callable::TypeAliasType, Some((Some(name), value))) => {
                    let value_or_parameters = matches!(name.as_str(), "value" | "type_params");
                    self.type_definition(&value, value_or_parameters)?;
                }
                (Callable::TypeAliasType, None) => self.type_definition(item, index == 1)?,
                (Callable::NamedTuple, Some((None, value))) => {
                    if value.kind() == "dictionary" {
                        self.dictionary_values(&value, false)?;
                    } else {
                        self.visit(&value)?;
                    }
                }
                (Callable::NamedTuple, Some((Some(_), value))) => {
                    self.type_definition(&value, true)?
                }
                (Callable::NamedTuple, None) if first_positional => self.visit(item)?,
                (Callable::NamedTuple, None) => {
                    if matches!(item.kind().as_ref(), "list" | "tuple") {
                        self.named_tuple_fields(item)?;
                    } else {
                        self.visit(item)?;
                    }
                }
                (Callable::TypedDict, Some((name, value))) => {
                    let is_type = match name.as_deref() {
                        Some("total") => false,
                        Some("extra_items") | None => true,
                        Some(_) => legacy_fields,
                    };
                    self.type_definition(&value, is_type)?;
                }
                (Callable::TypedDict, None) if first_positional => self.visit(item)?,
                (Callable::TypedDict, None) => {
                    if item.kind() == "dictionary" {
                        self.dictionary_values(item, true)?;
                    } else {
                        self.visit(item)?;
                    }
                }
                (Callable::MypyExtension, Some((name, value))) => {
                    let typed = positional.is_empty() && name.as_deref() == Some("type");
                    self.type_definition(&value, typed)?;
                }
                (Callable::MypyExtension, None) => self.type_definition(item, first_positional)?,
            }
            if !matches!(
                item.kind().as_ref(),
                "keyword_argument" | "dictionary_splat"
            ) {
                first_positional = false;
            }
        }
        Ok(())
    }

    /// `NamedTuple("N", [("a", int), ...])`: each two-element display's second element is a type.
    fn named_tuple_fields<D: Doc>(&mut self, display: &Node<'_, D>) -> Visit {
        self.builder.enter()?;
        for element in named(display)
            .filter(|element| element.kind() != "comment")
            .collect::<Vec<_>>()
        {
            let pair = matches!(element.kind().as_ref(), "list" | "tuple")
                .then(|| {
                    named(&element)
                        .filter(|part| self.tag(part) != Tag::Comment)
                        .collect::<Vec<_>>()
                })
                .filter(|parts| parts.len() == 2);
            match pair {
                Some(parts) => {
                    self.visit(&parts[0])?;
                    self.type_definition(&parts[1], true)?;
                }
                None => self.visit(&element)?,
            }
        }
        self.builder.leave();
        Ok(())
    }

    /// A dictionary display whose values are types: `TypedDict("T", {"a": int})` (where a `**`
    /// entry's value is one too) or `NamedTuple("N", **{"a": int})` (where it is not).
    fn dictionary_values<D: Doc>(&mut self, display: &Node<'_, D>, splat_is_type: bool) -> Visit {
        self.builder.enter()?;
        for entry in named(display)
            .filter(|entry| entry.kind() != "comment")
            .collect::<Vec<_>>()
        {
            if entry.kind() == "pair" {
                if let Some(key) = entry.field("key") {
                    self.visit(&key)?;
                }
                if let Some(value) = entry.field("value") {
                    self.type_definition(&value, true)?;
                }
            } else {
                self.type_definition(&entry, splat_is_type && entry.kind() == "dictionary_splat")?;
            }
        }
        self.builder.leave();
        Ok(())
    }

    /// A subscript a stub reads typing-aware (Ruff's `match_annotated_subscript`): the value first;
    /// a generic's slice is a type definition, `Annotated`'s first element, a `TypedDict`
    /// display's values; a `Literal`'s slice defers nothing.
    fn typing_subscript<D: Doc>(&mut self, node: &Node<'_, D>) -> Visit {
        let Some(value) = node.field("value") else {
            return self.children(node);
        };
        self.visit(&value)?;
        let slices = named(node)
            .filter(|child| child.node_id() != value.node_id() && self.tag(child) != Tag::Comment)
            .collect::<Vec<_>>();
        let kind = self
            .qualified(&value)
            .and_then(|name| subscript_kind(&name, self.settings()));
        match kind {
            Some(SubscriptKind::Literal) => {
                let literal = std::mem::replace(&mut self.in_literal, true);
                let visited = slices.iter().try_for_each(|slice| self.visit(slice));
                self.in_literal = literal;
                visited
            }
            Some(SubscriptKind::Generic) => slices
                .iter()
                .try_for_each(|slice| self.type_definition(slice, true)),
            Some(SubscriptKind::Annotated) => {
                // Ruff's slice is a tuple when there are several, or one written in parentheses.
                let elements = match slices.as_slice() {
                    [single] if single.kind() == "tuple" => named(single)
                        .filter(|element| self.tag(element) != Tag::Comment)
                        .collect::<Vec<_>>(),
                    [_] => return self.visit(&slices[0]),
                    _ => slices,
                };
                for (index, element) in elements.iter().enumerate() {
                    self.type_definition(element, index == 0)?;
                }
                Ok(())
            }
            Some(SubscriptKind::TypedDict) => match slices.as_slice() {
                [single] if single.kind() == "dictionary" => self.dictionary_values(single, false),
                _ => slices.iter().try_for_each(|slice| self.visit(slice)),
            },
            None => slices.iter().try_for_each(|slice| self.visit(slice)),
        }
    }

    /// Visits code that runs after the source at `offset` although it is written before it.
    fn read_after(&mut self, offset: usize, visit: impl FnOnce(&mut Self) -> Visit) -> Visit {
        let position = self.builder.position;
        self.builder.position = Some(position.map_or(offset, |current| current.max(offset)));
        let visited = visit(self);
        self.builder.position = position;
        visited
    }

    /// An annotation: read in place, or after the module under `from __future__ import annotations`
    /// (Ruff's deferred type definitions), where it is a forward reference too: typing-only, so
    /// its names resolve in the module first.
    fn annotation<D: Doc>(&mut self, node: &Node<'_, D>, id: u16) -> Visit {
        if let Some(annotation) = field(node, id) {
            self.definition(&annotation);
        }
        let (deferring, forwarding) = (self.builder.deferring, self.builder.forwarding);
        self.builder.deferring |= self.future_annotations;
        self.builder.forwarding |= self.future_annotations;
        let visited = self.visit_field(node, id);
        self.builder.deferring = deferring;
        self.builder.forwarding = forwarding;
        visited
    }

    /// Whether `head` is a name an import bound so far to a member `members` holds under another
    /// name (`from typing import List as L`).
    fn imports_member<D: Doc>(&self, head: &Node<'_, D>, members: &dyn Fn(&str) -> bool) -> bool {
        if self.tag(head) != Tag::Identifier {
            return false;
        }
        let text = identifier(head);
        [self.builder.current, 0].iter().any(|scope| {
            self.builder
                .scope(*scope)
                .bindings
                .get(text.as_ref())
                .is_some_and(|bindings| {
                    bindings.iter().any(|binding| match &binding.import {
                        Some(Import::Qualified(qualified)) => {
                            qualified.rsplit('.').next().is_some_and(members)
                        }
                        _ => false,
                    })
                })
        })
    }

    /// Records a type definition: an annotation, a type alias's value, a type parameter's bound.
    fn definition<D: Doc>(&mut self, node: &Node<'_, D>) {
        self.builder.type_roots.push(TypeRoot {
            node: NodeRef::of(node),
            kind: RootKind::Definition,
        });
    }

    /// Opens the annotation scope of PEP 695 type parameters and binds them; false without any.
    /// A bound and a PEP 696 default (`T: B = D`, `type_default`) are read lazily in that scope.
    fn type_parameters<D: Doc>(
        &mut self,
        owner: &Node<'_, D>,
        list: Option<Node<'_, D>>,
    ) -> Result<bool, BuildError> {
        let Some(list) = list else { return Ok(false) };
        self.builder.push(ScopeKind::TypeParams, owner.range());
        let offset = owner.range().start;
        let fields = fields();
        for item in named(&list) {
            let (parameter, default) = if self.tag(&item) == Tag::TypeDefault {
                (field(&item, fields.name), field(&item, fields.value))
            } else {
                (Some(item.clone()), None)
            };
            let inner = parameter.and_then(|parameter| {
                if self.tag(&parameter) == Tag::Type {
                    named(&parameter).next()
                } else {
                    Some(parameter)
                }
            });
            let site = site(Form::TypeParameter, &item);
            match inner.as_ref().map(|inner| (inner, self.tag(inner))) {
                None => {}
                Some((inner, Tag::Identifier)) => self.bind_here(inner, offset, None, site),
                Some((inner, Tag::SplatType)) => {
                    if let Some(name) =
                        named(inner).find(|child| self.tag(child) == Tag::Identifier)
                    {
                        self.bind_here(&name, offset, None, site);
                    }
                }
                Some((inner, Tag::ConstrainedType)) => {
                    let mut parts = named(inner);
                    if let Some(name) = parts.next() {
                        let name = if self.tag(&name) == Tag::Type {
                            named(&name).next()
                        } else {
                            Some(name)
                        };
                        if let Some(name) = name.filter(|name| self.tag(name) == Tag::Identifier) {
                            self.bind_here(&name, offset, None, site);
                        }
                    }
                    for bound in parts {
                        self.definition(&bound);
                        self.lazily(|visitor| visitor.visit(&bound))?;
                    }
                }
                Some((inner, _)) => self.lazily(|visitor| visitor.visit(inner))?,
            }
            if let Some(default) = default {
                self.definition(&default);
                self.lazily(|visitor| visitor.visit(&default))?;
            }
        }
        Ok(true)
    }

    fn function<D: Doc>(&mut self, node: &Node<'_, D>) -> Visit {
        let fields = fields();
        let outer = self.builder.current;
        let parameters = field(node, fields.parameters);
        if let Some(parameters) = &parameters {
            self.visit_defaults(parameters)?;
        }
        let typed = self.type_parameters(node, field(node, fields.type_parameters))?;
        self.in_type_scope(|visitor| {
            if let Some(parameters) = &parameters {
                visitor.visit_annotations(parameters)?;
            }
            visitor.annotation(node, fields.return_type)
        })?;
        self.builder.push(ScopeKind::Function, node.range());
        if let Some(parameters) = &parameters {
            self.bind_parameters(parameters, node.range().start);
        }
        self.visit_field(node, fields.body)?;
        self.builder.pop();
        if typed {
            self.builder.pop();
        }
        if let Some(name) = field(node, fields.name) {
            let site = site(Form::Definition, node);
            self.bind_in(
                outer,
                &name,
                node.range().end,
                Some(Import::Definition),
                site,
            );
        }
        self.declare_globals(node, outer)
    }

    /// Ruff's `set_globals`: where a function or class is defined, each name its body declares
    /// `global` that the module does not bind yet gets a placeholder module binding, so later
    /// module code has no F821. Inside a function body that happens after the whole module.
    fn declare_globals<D: Doc>(&mut self, node: &Node<'_, D>, outer: u32) -> Visit {
        let Some(body) = field(node, fields().body) else {
            return Ok(());
        };
        let mut declared = Vec::new();
        self.declared_globals(&body, &mut declared)?;
        let offset = node.range().end;
        if self.crosses_function(outer, 0) {
            self.deferred_globals.extend(declared);
            return Ok(());
        }
        for name in declared {
            let bound = self
                .builder
                .scope(0)
                .bindings
                .get(name.as_str())
                .is_some_and(|bindings| {
                    bindings.iter().any(|binding| {
                        !binding.deferred
                            && binding.offset <= offset
                            && binding.site.form != Form::Annotation
                    })
                });
            if !bound {
                self.declare(&name, offset, false);
            }
        }
        Ok(())
    }

    fn declare(&mut self, name: &str, offset: usize, deferred: bool) {
        let binding = Binding {
            deferred,
            origin: Origin::Declared,
            ..Binding::new(offset, VALUE, Site::IMPLICIT)
        };
        self.builder.bind(0, name, binding);
    }

    /// Names a function body declares `global`, outside nested functions and classes.
    fn declared_globals<D: Doc>(&mut self, node: &Node<'_, D>, names: &mut Vec<String>) -> Visit {
        for child in named(node) {
            match self.tag(&child) {
                Tag::GlobalStatement => {
                    self.builder.enter()?;
                    names.extend(
                        named(&child)
                            .filter(|name| self.tag(name) == Tag::Identifier)
                            .map(|name| identifier(&name).into_owned()),
                    );
                    self.builder.leave();
                }
                Tag::Compound
                | Tag::ForStatement
                | Tag::CaseClause
                | Tag::If
                | Tag::Elif
                | Tag::Else
                | Tag::Try
                | Tag::Except
                | Tag::Finally
                | Tag::With
                | Tag::While
                | Tag::Match => {
                    self.builder.enter()?;
                    self.declared_globals(&child, names)?;
                    self.builder.leave();
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn lambda<D: Doc>(&mut self, node: &Node<'_, D>) -> Visit {
        let fields = fields();
        let parameters = field(node, fields.parameters);
        if let Some(parameters) = &parameters {
            self.visit_defaults(parameters)?;
        }
        self.builder.push(ScopeKind::Lambda, node.range());
        if let Some(parameters) = &parameters {
            self.bind_parameters(parameters, node.range().start);
        }
        self.visit_field(node, fields.body)?;
        self.builder.pop();
        Ok(())
    }

    fn class<D: Doc>(&mut self, node: &Node<'_, D>) -> Visit {
        let fields = fields();
        let outer = self.builder.current;
        let typed = self.type_parameters(node, field(node, fields.type_parameters))?;
        // A stub reads its class bases and keywords after the module (Ruff's deferred class
        // bases), where a forward name is legal.
        self.in_type_scope(|visitor| {
            if visitor.stub {
                visitor.lazily(|visitor| visitor.visit_field(node, fields.superclasses))
            } else {
                visitor.visit_field(node, fields.superclasses)
            }
        })?;
        self.builder.push(ScopeKind::Class, node.range());
        self.visit_field(node, fields.body)?;
        self.builder.pop();
        if typed {
            self.builder.pop();
        }
        if let Some(name) = field(node, fields.name) {
            let site = site(Form::Definition, node);
            self.bind_in(
                outer,
                &name,
                node.range().end,
                Some(Import::Definition),
                site,
            );
        }
        self.declare_globals(node, outer)
    }

    /// `type X[T] = value`: the name binds where the statement is, the value is lazy and sees `T`.
    fn type_alias<D: Doc>(&mut self, node: &Node<'_, D>) -> Visit {
        let fields = fields();
        let mut name = None;
        let mut parameters = None;
        if let Some(left) = field(node, fields.left) {
            let inner = if self.tag(&left) == Tag::Type {
                named(&left).next()
            } else {
                Some(left)
            };
            match inner.as_ref().map(|inner| self.tag(inner)) {
                Some(Tag::Identifier) => name = inner,
                Some(Tag::GenericType) => {
                    let inner = inner.expect("tagged node");
                    for child in named(&inner) {
                        match self.tag(&child) {
                            Tag::Identifier if name.is_none() => name = Some(child),
                            Tag::TypeParameter => parameters = Some(child),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(name) = &name {
            self.bind_here(name, node.range().start, None, site(Form::TypeAlias, node));
        }
        let typed = self.type_parameters(node, parameters)?;
        if let Some(value) = field(node, fields.right) {
            self.definition(&value);
        }
        self.lazily(|visitor| visitor.visit_field(node, fields.right))?;
        if typed {
            self.builder.pop();
        }
        Ok(())
    }

    /// The first iterable runs in the enclosing scope; targets, conditions, later iterables and the
    /// element run in the comprehension's own scope.
    fn comprehension<D: Doc>(&mut self, node: &Node<'_, D>) -> Visit {
        let fields = fields();
        let clauses = named(node)
            .filter(|child| self.tag(child) != Tag::Comment)
            .collect::<Vec<_>>();
        let body = field(node, fields.body);
        let first = clauses
            .iter()
            .find(|clause| self.tag(clause) == Tag::ForInClause)
            .and_then(|clause| field(clause, fields.right));
        if let Some(first) = &first {
            self.visit(first)?;
        }
        self.builder.push(ScopeKind::Comprehension, node.range());
        for clause in &clauses {
            if same(clause, body.as_ref()) {
                continue;
            }
            if self.tag(clause) == Tag::ForInClause {
                // Each clause's iterable runs before its target binds (Ruff's
                // `visit_generators`): `for x in x` reads the enclosing `x`.
                let right = field(clause, fields.right);
                if let Some(right) = &right
                    && !same(right, first.as_ref())
                {
                    self.visit(right)?;
                }
                if let Some(left) = field(clause, fields.left) {
                    let offset = right.map_or(left.range().end, |right| right.range().end);
                    self.targets(&left, offset, site(Form::For, clause))?;
                }
            } else {
                self.visit(clause)?;
            }
        }
        if let Some(body) = &body {
            self.read_after(node.range().end, |visitor| visitor.visit(body))?;
        }
        self.builder.pop();
        Ok(())
    }

    fn dotted<D: Doc>(&self, node: &Node<'_, D>) -> String {
        named(node)
            .map(|part| identifier(&part).into_owned())
            .collect::<Vec<_>>()
            .join(".")
    }

    /// Records that Ruff's traversal sees `module` imported at `statement` (`add_module`): at the
    /// depth of the function bodies around it.
    fn seen<D: Doc>(&mut self, statement: &Node<'_, D>, module: &str) {
        let mut depth = 0;
        let mut current = Some(self.builder.current);
        while let Some(index) = current {
            let scope = self.builder.scope(index);
            depth += u32::from(scope.kind == ScopeKind::Function);
            current = scope.parent;
        }
        self.builder.table.module_imports.push(super::ModuleImport {
            module: module.into(),
            depth,
            offset: statement.range().start,
        });
    }

    /// `import a.b` binds `a` (to `a`); `import a.b as c` binds `c` to `a.b`.
    fn import<D: Doc>(&mut self, node: &Node<'_, D>) {
        let offset = node.range().end;
        let fields = fields();
        for item in named(node) {
            let module = match self.tag(&item) {
                Tag::DottedName => Some(item.clone()),
                Tag::AliasedImport => field(&item, fields.name),
                _ => None,
            };
            if let Some(head) = module.as_ref().and_then(|module| named(module).next()) {
                self.seen(node, &identifier(&head));
            }
            match self.tag(&item) {
                Tag::DottedName => {
                    if let Some(head) = named(&item).next() {
                        let text = identifier(&head);
                        let import = Import::Qualified(text.as_ref().into());
                        let site = site(Form::Import, &item);
                        let dotted = self.dotted(&item);
                        let submodule = dotted != text;
                        let binding = Binding {
                            submodule: submodule.then(|| dotted.into()),
                            import_kind: Some(if submodule {
                                ImportKind::Submodule
                            } else {
                                ImportKind::Module
                            }),
                            ..self.binding(offset, Some(import), site, NodeRef::of(&item))
                        };
                        self.bind_name(self.builder.current, &text, binding);
                    }
                }
                Tag::AliasedImport => {
                    if let (Some(name), Some(alias)) =
                        (field(&item, fields.name), field(&item, fields.alias))
                    {
                        let dotted = self.dotted(&name);
                        let binding = Binding {
                            explicit_export: dotted == identifier(&alias),
                            import_kind: Some(ImportKind::Module),
                            ..self.binding(
                                offset,
                                Some(Import::Qualified(dotted.into())),
                                site(Form::Import, &item),
                                NodeRef::of(&alias),
                            )
                        };
                        self.bind_name(self.builder.current, &identifier(&alias), binding);
                    }
                }
                _ => {}
            }
        }
    }

    /// `from m import x as y` binds `y` to `m.x`. A relative module binds without a qualified name;
    /// `*` binds nothing (Ruff: star imports never shadow).
    fn import_from<D: Doc>(&mut self, node: &Node<'_, D>) {
        let offset = node.range().end;
        let fields = fields();
        let module_node = field(node, fields.module_name);
        let module = match &module_node {
            Some(module) if self.tag(module) == Tag::DottedName => Some(self.dotted(module)),
            Some(_) => None,
            None => Some("__future__".to_owned()),
        };
        // Ruff marks only an absolute import's module seen.
        if let Some(head) = module
            .as_deref()
            .and_then(|module| module.split('.').next())
        {
            self.seen(node, head);
        }
        // `from ..a.b import c`: the level is the prefix's dots, `a.b` the module after them.
        let relative = module_node
            .as_ref()
            .filter(|_| module.is_none())
            .map(|node| {
                let text = node.text();
                let prefix = text.chars().take_while(|c| *c == '.' || c.is_whitespace());
                let level = prefix.filter(|c| *c == '.').count();
                let after = named(node)
                    .find(|part| self.tag(part) == Tag::DottedName)
                    .map(|part| self.dotted(&part));
                (level, after)
            });
        let qualified = |name: &str| match (&module, &relative) {
            (Some(module), _) => Import::Qualified(format!("{module}.{name}").into()),
            (None, Some((level, Some(after)))) => Import::Relative {
                level: *level,
                rest: format!("{after}.{name}").into(),
            },
            (None, Some((level, None))) => Import::Relative {
                level: *level,
                rest: name.into(),
            },
            (None, None) => Import::Relative {
                level: usize::MAX,
                rest: name.into(),
            },
        };
        let member = if module.as_deref() == Some("__future__") {
            ImportKind::Future
        } else {
            ImportKind::Member
        };
        for item in named(node) {
            if same(&item, module_node.as_ref()) {
                continue;
            }
            match self.tag(&item) {
                Tag::DottedName => {
                    let name = self.dotted(&item);
                    if module.as_deref() == Some("__future__") && name == "annotations" {
                        self.future_annotations = true;
                    }
                    let import = qualified(&name);
                    let site = site(Form::Import, &item);
                    let binding = Binding {
                        import_kind: Some(member),
                        ..self.binding(offset, Some(import), site, NodeRef::of(&item))
                    };
                    self.bind_name(self.builder.current, &name, binding);
                }
                Tag::AliasedImport => {
                    if let (Some(name), Some(alias)) =
                        (field(&item, fields.name), field(&item, fields.alias))
                    {
                        let dotted = self.dotted(&name);
                        let binding = Binding {
                            explicit_export: dotted == identifier(&alias),
                            import_kind: Some(member),
                            ..self.binding(
                                offset,
                                Some(qualified(&dotted)),
                                site(Form::Import, &item),
                                NodeRef::of(&alias),
                            )
                        };
                        self.bind_name(self.builder.current, &identifier(&alias), binding);
                    }
                }
                // Ruff: star imports bind nothing, but a later unresolved name may come from one.
                Tag::WildcardImport => {
                    let current = self.builder.current as usize;
                    self.builder.table.scopes[current].stars.push(offset);
                }
                _ => {}
            }
        }
    }

    /// A `case` pattern: capture names bind for the guard and body; value patterns are loads.
    fn case_pattern<D: Doc>(&mut self, node: &Node<'_, D>, offset: usize) -> Visit {
        self.builder.enter()?;
        match self.tag(node) {
            Tag::CasePattern | Tag::AsPattern => {
                for child in named(node) {
                    if self.tag(&child) == Tag::Identifier {
                        self.capture(&child, offset);
                    } else {
                        self.case_pattern(&child, offset)?;
                    }
                }
            }
            Tag::DottedName => {
                let parts = named(node).collect::<Vec<_>>();
                match parts.as_slice() {
                    [name] => self.capture(name, offset),
                    [head, ..] => self.visit(head)?,
                    [] => {}
                }
            }
            Tag::Identifier | Tag::SplatPattern => {
                let name = if self.tag(node) == Tag::Identifier {
                    Some(node.clone())
                } else {
                    named(node).find(|child| self.tag(child) == Tag::Identifier)
                };
                if let Some(name) = name {
                    self.capture(&name, offset);
                }
            }
            Tag::ClassPattern => {
                let mut parts = named(node);
                if let Some(class) = parts.next() {
                    if self.tag(&class) == Tag::DottedName {
                        if let Some(head) = named(&class).next() {
                            self.visit(&head)?;
                        }
                    } else {
                        self.visit(&class)?;
                    }
                }
                for part in parts {
                    self.case_pattern(&part, offset)?;
                }
            }
            Tag::KeywordPattern => {
                // The first identifier names an attribute.
                for (index, part) in named(node).enumerate() {
                    if index > 0 || self.tag(&part) != Tag::Identifier {
                        self.case_pattern(&part, offset)?;
                    }
                }
            }
            Tag::DictPattern => {
                let keys = node
                    .field_children("key")
                    .map(|key| key.node_id())
                    .collect::<Vec<_>>();
                for part in named(node) {
                    if keys.contains(&part.node_id()) {
                        self.visit(&part)?;
                    } else {
                        self.case_pattern(&part, offset)?;
                    }
                }
            }
            _ => self.visit(node)?,
        }
        self.builder.leave();
        Ok(())
    }

    /// A capture binds at its pattern: the case, `as`, splat or keyword pattern around the name.
    fn capture<D: Doc>(&mut self, name: &Node<'_, D>, offset: usize) {
        if identifier(name) != "_" {
            let pattern = name.parent().unwrap_or_else(|| name.clone());
            self.bind_here(name, offset, None, site(Form::Match, &pattern));
        }
    }
}

/// The innermost right-hand side of an assignment (`v` in `a = b = v`).
fn innermost_value<'r, D: Doc>(node: &Node<'r, D>) -> Option<Node<'r, D>> {
    let right = fields().right;
    let mut value = field(node, right)?;
    while value.kind() == "assignment" {
        value = field(&value, right)?;
    }
    Some(value)
}

/// The named children of a node, comments aside.
fn elements<'r, D: Doc>(node: &Node<'r, D>) -> Vec<Node<'r, D>> {
    named(node)
        .filter(|child| child.kind() != "comment")
        .collect()
}

/// What a pair of parentheses around one element holds: `(x)` as a target (tree-sitter reads it
/// as a tuple pattern without a comma) or an expression.
pub(super) fn unparenthesized<'r, D: Doc>(node: &Node<'r, D>, kinds: &[&str]) -> Node<'r, D> {
    let mut node = node.clone();
    while kinds.contains(&node.kind().as_ref()) && !node.children().any(|child| child.kind() == ",")
    {
        match elements(&node).as_slice() {
            [inner] => node = inner.clone(),
            _ => break,
        }
    }
    node
}

/// Pairs the names of an unpacking target with the elements of a tuple or list display, as
/// Ruff's `find_binding_value` does (nested displays too), where Python binds that element: the
/// arities agree, or the element comes before a starred target (and, with arities that agree,
/// after it). A starred value hides every position from its own on; a set's order is unknown.
fn unpack<D: Doc>(target: &Node<'_, D>, value: &Node<'_, D>, out: &mut Vec<(usize, NodeRef)>) {
    let target = unparenthesized(target, &["parenthesized_expression", "tuple_pattern"]);
    let value = unparenthesized(value, &["parenthesized_expression"]);
    if target.kind() == "identifier" {
        out.push((target.range().start, NodeRef::of(&value)));
        return;
    }
    let groups = [
        "pattern_list",
        "tuple_pattern",
        "list_pattern",
        "tuple",
        "list",
    ];
    let displays = ["expression_list", "tuple", "list"];
    if !groups.contains(&target.kind().as_ref()) || !displays.contains(&value.kind().as_ref()) {
        return;
    }
    let (targets, values) = (elements(&target), elements(&value));
    let starred = |node: &Node<'_, D>, kinds: &[&str]| kinds.contains(&node.kind().as_ref());
    let star_target = targets
        .iter()
        .position(|node| starred(node, &["list_splat_pattern", "list_splat"]));
    let star_value = values
        .iter()
        .position(|node| starred(node, &["list_splat", "parenthesized_list_splat"]));
    let aligned = targets.len() == values.len();
    if star_target.is_none() && star_value.is_none() && !aligned {
        return;
    }
    for (index, target) in targets.iter().enumerate() {
        let hidden = star_value.is_some_and(|star| index >= star)
            || star_target.is_some_and(|star| index == star || (index > star && !aligned));
        if let (false, Some(value)) = (hidden, values.get(index)) {
            unpack(target, value, out);
        }
    }
}

/// A binding site with no value or annotation.
fn site<D: Doc>(form: Form, node: &Node<'_, D>) -> Site {
    Site {
        form,
        node: Some(NodeRef::of(node)),
        value: None,
        annotation: None,
    }
}

/// An annotation's expression, without the `type` node around it.
fn annotation_ref<D: Doc>(annotation: &Node<'_, D>) -> NodeRef {
    let mut inner = named(annotation).filter(|child| child.kind() != "comment");
    match (inner.next(), inner.next()) {
        (Some(expression), None) if annotation.kind() == "type" => NodeRef::of(&expression),
        _ => NodeRef::of(annotation),
    }
}

/// Whether a `del` is conditional, so Ruff adds no `Deletion` for it: inside an `if`, `while` or
/// `match` anywhere above it, an `except` handler of its own function or class, or the `else` of
/// a loop or `try` (Ruff's `on_conditional_branch`, `in_exception_handler`, `in_orelse`).
fn conditional_delete<D: Doc>(statement: &Node<'_, D>) -> bool {
    let mut own_body = true;
    for ancestor in statement.ancestors() {
        match ancestor.kind().as_ref() {
            "if_statement" | "while_statement" | "match_statement" => return true,
            "except_clause" | "except_group_clause" if own_body => return true,
            "else_clause"
                if ancestor.parent().is_some_and(|parent| {
                    matches!(
                        parent.kind().as_ref(),
                        "for_statement" | "while_statement" | "try_statement"
                    )
                }) =>
            {
                return true;
            }
            "function_definition" | "class_definition" => own_body = false,
            _ => {}
        }
    }
    false
}

/// `typing`'s callables whose arguments Ruff reads as type definitions (`typing::Callable`).
#[derive(Clone, Copy)]
enum Callable {
    Cast,
    TypeForm,
    NewType,
    TypeVar,
    TypeAliasType,
    NamedTuple,
    TypedDict,
    MypyExtension,
}

fn typing_callable(name: &[String], settings: Option<&PythonTyping>) -> Option<Callable> {
    let callables = [
        ("cast", Callable::Cast),
        ("TypeForm", Callable::TypeForm),
        ("NewType", Callable::NewType),
        ("TypeVar", Callable::TypeVar),
        ("TypeAliasType", Callable::TypeAliasType),
        ("NamedTuple", Callable::NamedTuple),
        ("TypedDict", Callable::TypedDict),
    ];
    if let Some((_, callable)) = callables
        .iter()
        .find(|(member, _)| typing_member(name, member, settings))
    {
        return Some(*callable);
    }
    match name {
        [module, member]
            if module == "mypy_extensions"
                && matches!(
                    member.as_str(),
                    "Arg" | "DefaultArg" | "NamedArg" | "DefaultNamedArg" | "VarArg" | "KwArg"
                ) =>
        {
            Some(Callable::MypyExtension)
        }
        _ => None,
    }
}

/// A call argument's keyword: `name=value` or, with no name, `**value`. `None` for a positional or
/// starred argument.
fn keyword_argument<'r, D: Doc>(item: &Node<'r, D>) -> Option<(Option<String>, Node<'r, D>)> {
    match item.kind().as_ref() {
        "keyword_argument" => Some((
            item.field("name")
                .map(|name| identifier(&name).into_owned()),
            item.field("value")?,
        )),
        "dictionary_splat" => Some((None, named(item).find(|inner| inner.kind() != "comment")?)),
        _ => None,
    }
}
