//! TypeScript and TSX bindings and references, after eslint-scope and typescript-eslint's scope
//! manager: `var` hoists to the enclosing function (or module or namespace); `let`, `const`, class
//! and function declarations bind in their block; values and types are separate namespaces.

use super::{
    Binding, BuildError, Builder, Form, Grammar, NodeRef, ScopeKind, ScopeTable, Site, TYPE, VALUE,
};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::literal::eslint_directive::Directive;
use ast_grep_core::{Doc, Node};
use std::sync::OnceLock;

#[derive(Clone, Copy, Default, PartialEq)]
enum Tag {
    #[default]
    Other,
    Identifier,
    TypeIdentifier,
    ShorthandProperty,
    Undefined,
    Comment,
    /// Nodes whose subtree never holds a reference: literal types, JSX namespace names, `export *
    /// as`.
    Opaque,
    StatementBlock,
    FunctionDeclaration,
    FunctionExpression,
    ArrowFunction,
    MethodDefinition,
    FunctionSignature,
    Signature,
    FormalParameters,
    Parameter,
    ClassDeclaration,
    ClassExpression,
    ClassStaticBlock,
    LexicalDeclaration,
    VariableDeclaration,
    VariableDeclarator,
    ForStatement,
    ForInStatement,
    CatchClause,
    SwitchBody,
    EnumDeclaration,
    EnumAssignment,
    PropertyIdentifier,
    Module,
    AmbientDeclaration,
    TypeAlias,
    Interface,
    TypeParameter,
    ConditionalType,
    InferType,
    IndexSignature,
    MappedTypeClause,
    ImportStatement,
    ImportClause,
    NamespaceImport,
    NamedImports,
    ImportSpecifier,
    ImportRequireClause,
    ImportAlias,
    ExportStatement,
    ExportSpecifier,
    NestedIdentifier,
    NestedTypeIdentifier,
    JsxElementName,
    ObjectPattern,
    ArrayPattern,
    PairPattern,
    ObjectAssignmentPattern,
    AssignmentPattern,
    RestPattern,
    ComputedPropertyName,
    /// `x = v` and `x += v`: the target is written.
    Assignment,
    /// `x++`: the argument is written.
    Update,
    Call,
    /// Member access and subscripts, whose head a write or mutating call mutates.
    MemberExpression,
    /// Expressions around one expression that leave it the same reference.
    Wrapper,
    With,
}

fn tag(name: &str) -> Tag {
    match name {
        "identifier" => Tag::Identifier,
        "type_identifier" => Tag::TypeIdentifier,
        "shorthand_property_identifier" | "shorthand_property_identifier_pattern" => {
            Tag::ShorthandProperty
        }
        "undefined" => Tag::Undefined,
        "comment" | "html_comment" => Tag::Comment,
        "literal_type" | "jsx_namespace_name" | "namespace_export" => Tag::Opaque,
        "statement_block" => Tag::StatementBlock,
        "function_declaration" | "generator_function_declaration" => Tag::FunctionDeclaration,
        "function_expression" | "function" | "generator_function" => Tag::FunctionExpression,
        "arrow_function" => Tag::ArrowFunction,
        "method_definition" => Tag::MethodDefinition,
        "function_signature" => Tag::FunctionSignature,
        "method_signature"
        | "abstract_method_signature"
        | "call_signature"
        | "construct_signature"
        | "function_type"
        | "constructor_type" => Tag::Signature,
        "formal_parameters" => Tag::FormalParameters,
        "required_parameter" | "optional_parameter" => Tag::Parameter,
        "class_declaration" | "abstract_class_declaration" => Tag::ClassDeclaration,
        "class" => Tag::ClassExpression,
        "class_static_block" => Tag::ClassStaticBlock,
        "lexical_declaration" => Tag::LexicalDeclaration,
        "variable_declaration" => Tag::VariableDeclaration,
        "variable_declarator" => Tag::VariableDeclarator,
        "for_statement" => Tag::ForStatement,
        "for_in_statement" => Tag::ForInStatement,
        "catch_clause" => Tag::CatchClause,
        "switch_body" => Tag::SwitchBody,
        "enum_declaration" => Tag::EnumDeclaration,
        "enum_assignment" => Tag::EnumAssignment,
        "property_identifier" => Tag::PropertyIdentifier,
        "internal_module" | "module" => Tag::Module,
        "ambient_declaration" => Tag::AmbientDeclaration,
        "type_alias_declaration" => Tag::TypeAlias,
        "interface_declaration" => Tag::Interface,
        "type_parameter" => Tag::TypeParameter,
        "conditional_type" => Tag::ConditionalType,
        "infer_type" => Tag::InferType,
        "index_signature" => Tag::IndexSignature,
        "mapped_type_clause" => Tag::MappedTypeClause,
        "import_statement" => Tag::ImportStatement,
        "import_clause" => Tag::ImportClause,
        "namespace_import" => Tag::NamespaceImport,
        "named_imports" => Tag::NamedImports,
        "import_specifier" => Tag::ImportSpecifier,
        "import_require_clause" => Tag::ImportRequireClause,
        "import_alias" => Tag::ImportAlias,
        "export_statement" => Tag::ExportStatement,
        "export_specifier" => Tag::ExportSpecifier,
        "nested_identifier" => Tag::NestedIdentifier,
        "nested_type_identifier" => Tag::NestedTypeIdentifier,
        "jsx_opening_element" | "jsx_self_closing_element" | "jsx_closing_element" => {
            Tag::JsxElementName
        }
        "object_pattern" => Tag::ObjectPattern,
        "array_pattern" => Tag::ArrayPattern,
        "pair_pattern" => Tag::PairPattern,
        "object_assignment_pattern" => Tag::ObjectAssignmentPattern,
        "assignment_pattern" => Tag::AssignmentPattern,
        "rest_pattern" => Tag::RestPattern,
        "computed_property_name" => Tag::ComputedPropertyName,
        "assignment_expression" | "augmented_assignment_expression" => Tag::Assignment,
        "update_expression" => Tag::Update,
        "call_expression" => Tag::Call,
        "member_expression" | "subscript_expression" => Tag::MemberExpression,
        "parenthesized_expression"
        | "non_null_expression"
        | "as_expression"
        | "satisfies_expression"
        | "type_assertion" => Tag::Wrapper,
        "with_statement" => Tag::With,
        _ => Tag::Other,
    }
}

struct Fields {
    name: u16,
    alias: u16,
    value: u16,
    body: u16,
    parameter: u16,
    pattern: u16,
    left: u16,
    key: u16,
    source: u16,
    object: u16,
    module: u16,
    alternative: u16,
    ty: u16,
    argument: u16,
    function: u16,
    property: u16,
    index: u16,
}

struct Script {
    grammar: Grammar<Tag>,
    fields: Fields,
}

fn fields_of(grammar: &Grammar<Tag>) -> Fields {
    Fields {
        name: grammar.field("name"),
        alias: grammar.field("alias"),
        value: grammar.field("value"),
        body: grammar.field("body"),
        parameter: grammar.field("parameter"),
        pattern: grammar.field("pattern"),
        left: grammar.field("left"),
        key: grammar.field("key"),
        source: grammar.field("source"),
        alternative: grammar.field("alternative"),
        object: grammar.field("object"),
        module: grammar.field("module"),
        ty: grammar.field("type"),
        argument: grammar.field("argument"),
        function: grammar.field("function"),
        property: grammar.field("property"),
        index: grammar.field("index"),
    }
}

/// A dialect's grammar, as the visit reads it: each ECMAScript dialect parses with its own; Vue's
/// tapes hold its script blocks' program in TypeScript, TSX or JavaScript, as Vue numbers their
/// kinds (`KindCatalog`).
fn script(language: GrammarDialect) -> &'static Script {
    static TYPESCRIPT: OnceLock<Script> = OnceLock::new();
    static TSX: OnceLock<Script> = OnceLock::new();
    static JAVASCRIPT: OnceLock<Script> = OnceLock::new();
    static VUE: OnceLock<Script> = OnceLock::new();
    let slot = match language {
        GrammarDialect::TypeScript => &TYPESCRIPT,
        GrammarDialect::Tsx => &TSX,
        GrammarDialect::JavaScript => &JAVASCRIPT,
        GrammarDialect::Vue => &VUE,
        GrammarDialect::Python | GrammarDialect::Php | GrammarDialect::Blade => {
            unreachable!("only ECMAScript dialects build ECMAScript scopes")
        }
        #[cfg(test)]
        _ => unreachable!("only policy dialects build ECMAScript scopes"),
    };
    slot.get_or_init(|| {
        let grammar = Grammar::new(language, tag);
        let fields = fields_of(&grammar);
        Script { grammar, fields }
    })
}

type Visit = Result<(), BuildError>;

fn named<'a, 'r, D: Doc>(node: &'a Node<'r, D>) -> impl Iterator<Item = Node<'r, D>> + 'a {
    node.children().filter(|child| child.is_named())
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

pub(crate) fn visit_program<D: Doc>(builder: &mut Builder, root: &Node<'_, D>) -> Visit {
    let script = script(builder.table.language);
    let mut visitor = Visitor {
        builder,
        script,
        conditionals: Vec::new(),
        destructured: Vec::new(),
    };
    visitor.children(root)?;
    collect_directives(visitor.builder, root, script);
    Ok(())
}

/// Charges each write and property mutation to the binding it resolves to.
pub(crate) fn finish<D: Doc>(table: &mut ScopeTable, root: &Node<'_, D>) {
    table.count_writes(&root.text(), root.range().start);
}

/// ESLint's `/* global a, b: off */` block comments, as its `ConfigCommentParser` reads them
/// (`literal::eslint_directive`, which `parse` with `eslintDirective` reads too). Found by scanning
/// the source and confirmed against the tree, so a lookalike in a string never counts.
fn collect_directives<D: Doc>(builder: &mut Builder, root: &Node<'_, D>, script: &Script) {
    let text = root.text();
    let base = root.range().start;
    let mut from = 0;
    while let Some(found) = text[from..].find("/*") {
        let start = from + found;
        let end = text[start + 2..]
            .find("*/")
            .map_or(text.len(), |close| start + 4 + close);
        from = end.min(text.len());
        let comment = &text[start..end];
        let Some(directive) = Directive::parse(comment)
            .filter(|directive| matches!(directive.label.as_str(), "global" | "globals"))
        else {
            continue;
        };
        if !is_comment_at(root, base + start, script) {
            continue;
        }
        for item in directive.items(comment) {
            let directives = &mut builder.table.directives;
            if item
                .setting
                .as_ref()
                .is_some_and(|(setting, _)| setting == "off")
            {
                directives.declared.remove(&item.name);
                directives.off.insert(item.name);
            } else {
                directives.off.remove(&item.name);
                directives.declared.insert(item.name);
            }
        }
    }
}

fn is_comment_at<D: Doc>(root: &Node<'_, D>, offset: usize, script: &Script) -> bool {
    let mut node = root.clone();
    loop {
        let Some(child) = node
            .children()
            .find(|child| child.range().start <= offset && offset < child.range().end)
        else {
            return false;
        };
        if child.range().start == offset && script.grammar.tag(&child) == Tag::Comment {
            return true;
        }
        node = child;
    }
}

struct Visitor<'b, 'd> {
    builder: &'b mut Builder<'d>,
    script: &'static Script,
    /// The scopes of the conditional types being visited: `infer X` binds in the innermost one,
    /// through any function type or signature scope between.
    conditionals: Vec<u32>,
    /// The literal elements the names of the declarator being bound take, by name start.
    destructured: Vec<(usize, NodeRef)>,
}

/// Where a declaration binds.
#[derive(Clone, Copy)]
enum Target {
    /// The current scope (`let`, `const`, parameters, classes, types).
    Block,
    /// The nearest function, module or namespace scope (`var`).
    Var,
    /// A given scope: a conditional type's, for `infer`.
    At(u32),
}

impl Visitor<'_, '_> {
    fn tag<D: Doc>(&self, node: &Node<'_, D>) -> Tag {
        self.script.grammar.tag(node)
    }

    fn fields(&self) -> &'static Fields {
        &self.script.fields
    }

    fn children<D: Doc>(&mut self, node: &Node<'_, D>) -> Visit {
        for child in named(node) {
            self.visit(&child)?;
        }
        Ok(())
    }

    fn children_except<D: Doc>(
        &mut self,
        node: &Node<'_, D>,
        skip: &[Option<Node<'_, D>>],
    ) -> Visit {
        for child in named(node) {
            if !skip.iter().any(|skip| same(&child, skip.as_ref())) {
                self.visit(&child)?;
            }
        }
        Ok(())
    }

    fn visit<D: Doc>(&mut self, node: &Node<'_, D>) -> Visit {
        self.builder.enter()?;
        let fields = self.fields();
        match self.tag(node) {
            Tag::Identifier | Tag::ShorthandProperty | Tag::Undefined => {
                self.builder.reference(node.range(), VALUE)
            }
            Tag::TypeIdentifier => self.builder.reference(node.range(), TYPE),
            Tag::Comment | Tag::Opaque | Tag::PropertyIdentifier => {}
            Tag::StatementBlock | Tag::SwitchBody if self.builder.table.reading.block_scopes => {
                self.builder.push(ScopeKind::Block, node.range());
                self.children(node)?;
                self.builder.pop();
            }
            Tag::FunctionDeclaration => {
                if let Some(name) = field(node, fields.name) {
                    self.bind(Target::Block, &name, VALUE, site(Form::Definition, node));
                }
                self.function(node, true, false)?;
            }
            Tag::FunctionExpression => self.function(node, true, true)?,
            Tag::ArrowFunction => self.function(node, false, false)?,
            Tag::MethodDefinition => {
                if let Some(name) = field(node, fields.name) {
                    self.visit(&name)?;
                }
                self.function(node, true, false)?;
            }
            Tag::FunctionSignature => {
                if let Some(name) = field(node, fields.name) {
                    self.bind(Target::Block, &name, VALUE, site(Form::Definition, node));
                }
                self.signature(node)?;
            }
            Tag::Signature | Tag::IndexSignature => self.signature(node)?,
            Tag::FormalParameters => self.parameters(node)?,
            // A labelled tuple member (`[name: string]`): the label names nothing.
            Tag::Parameter => {
                let label = field(node, fields.name);
                self.children_except(node, &[label])?;
            }
            Tag::ClassDeclaration => {
                let name = field(node, fields.name);
                if let Some(name) = &name {
                    let site = site(Form::Definition, node);
                    self.bind(Target::Block, name, VALUE | TYPE, site);
                }
                // As eslint-scope, the class scope binds the name again, for the body's reads. The
                // name node stays the outer binding's.
                self.class(node, name, false)?;
            }
            Tag::ClassExpression => self.class(node, field(node, fields.name), true)?,
            Tag::ClassStaticBlock => {
                self.builder.push(ScopeKind::Function, node.range());
                match field(node, fields.body) {
                    Some(body) => self.children(&body)?,
                    None => self.children(node)?,
                }
                self.builder.pop();
            }
            Tag::LexicalDeclaration => self.declaration(node, Target::Block)?,
            Tag::VariableDeclaration => self.declaration(node, Target::Var)?,
            Tag::ForStatement => {
                self.builder.push(ScopeKind::Block, node.range());
                self.children(node)?;
                self.builder.pop();
            }
            Tag::ForInStatement => self.for_in(node)?,
            Tag::CatchClause => {
                self.builder.push(ScopeKind::Block, node.range());
                let parameter = field(node, fields.parameter);
                if let Some(parameter) = &parameter {
                    let site = Site {
                        annotation: self.annotation(node),
                        ..site(Form::Except, node)
                    };
                    self.pattern(parameter, Target::Block, site)?;
                }
                self.children_except(node, &[parameter])?;
                self.builder.pop();
            }
            Tag::EnumDeclaration => {
                if let Some(name) = field(node, fields.name) {
                    let site = site(Form::Definition, node);
                    self.bind(Target::Block, &name, VALUE | TYPE, site);
                }
                self.builder.push(ScopeKind::Block, node.range());
                if let Some(body) = field(node, fields.body) {
                    for member in named(&body) {
                        let site = site(Form::TypeParameter, &member);
                        match self.tag(&member) {
                            Tag::PropertyIdentifier => {
                                self.bind(Target::Block, &member, VALUE, site)
                            }
                            Tag::EnumAssignment => {
                                if let Some(name) = field(&member, fields.name)
                                    && self.tag(&name) == Tag::PropertyIdentifier
                                {
                                    self.bind(Target::Block, &name, VALUE, site);
                                }
                                if let Some(value) = field(&member, fields.value) {
                                    self.visit(&value)?;
                                }
                            }
                            _ => self.visit(&member)?,
                        }
                    }
                }
                self.builder.pop();
            }
            Tag::Module => self.module(node)?,
            Tag::AmbientDeclaration => {
                // `declare global { ... }` is the only form with a bare block.
                match named(node).find(|child| self.tag(child) == Tag::StatementBlock) {
                    Some(body) => {
                        self.builder
                            .push(ScopeKind::GlobalAugmentation, node.range());
                        self.children(&body)?;
                        self.builder.pop();
                    }
                    None => self.children(node)?,
                }
            }
            Tag::TypeAlias | Tag::Interface => {
                let name = field(node, fields.name);
                if let Some(name) = &name {
                    self.bind(Target::Block, name, TYPE, site(Form::Definition, node));
                }
                self.builder.push(ScopeKind::TypeParams, node.range());
                self.children_except(node, &[name])?;
                self.builder.pop();
            }
            Tag::TypeParameter => {
                let name = field(node, fields.name);
                if let Some(name) = &name {
                    self.bind(Target::Block, name, TYPE, site(Form::TypeParameter, node));
                }
                self.children_except(node, &[name])?;
            }
            Tag::ConditionalType => {
                // `infer X` binds in the check, extends and true types; the false branch is outside
                // (typescript-eslint's conditional type scope).
                let alternative = field(node, fields.alternative);
                let scope = self.builder.push(ScopeKind::TypeParams, node.range());
                self.conditionals.push(scope);
                let visited = self.children_except(node, std::slice::from_ref(&alternative));
                self.conditionals.pop();
                visited?;
                self.builder.pop();
                if let Some(alternative) = &alternative {
                    self.visit(alternative)?;
                }
            }
            Tag::InferType => {
                let mut parts = named(node);
                if let Some(name) = parts.next() {
                    if self.tag(&name) == Tag::TypeIdentifier {
                        let target = self
                            .conditionals
                            .last()
                            .map_or(Target::Block, |scope| Target::At(*scope));
                        self.bind(target, &name, TYPE, site(Form::TypeParameter, node));
                    } else {
                        self.visit(&name)?;
                    }
                }
                for part in parts {
                    self.visit(&part)?;
                }
            }
            Tag::MappedTypeClause => {
                let name = field(node, fields.name);
                if let Some(name) = &name {
                    self.bind(Target::Block, name, TYPE, site(Form::TypeParameter, node));
                }
                self.children_except(node, &[name])?;
            }
            Tag::ImportStatement => self.import(node)?,
            Tag::ImportAlias => {
                let mut parts = named(node);
                if let Some(name) = parts.next() {
                    self.bind(Target::Block, &name, VALUE | TYPE, site(Form::Import, node));
                }
                for part in parts {
                    self.visit(&part)?;
                }
            }
            Tag::ExportStatement => {
                // `export { a } from "m"` re-exports: its names are not local references.
                // `export = x` and `export default x` may name a type or a value (typescript-eslint's
                // `referenceDualValueType`).
                if field(node, fields.source).is_none() {
                    for child in named(node) {
                        if self.tag(&child) == Tag::Identifier {
                            self.builder.reference(child.range(), VALUE | TYPE);
                        } else {
                            self.visit(&child)?;
                        }
                    }
                }
            }
            Tag::ExportSpecifier => {
                if let Some(name) = field(node, fields.name)
                    && self.tag(&name) == Tag::Identifier
                {
                    self.builder.reference(name.range(), VALUE | TYPE);
                }
            }
            Tag::NestedIdentifier => {
                if let Some(object) = field(node, fields.object) {
                    self.visit(&object)?;
                }
            }
            Tag::NestedTypeIdentifier => {
                if let Some(module) = field(node, fields.module) {
                    if self.tag(&module) == Tag::Identifier {
                        self.builder.reference(module.range(), VALUE | TYPE);
                    } else {
                        self.visit(&module)?;
                    }
                }
            }
            Tag::JsxElementName => {
                // A lower-case tag is an intrinsic element, not a reference (typescript-eslint).
                let name = field(node, fields.name);
                if node.kind() != "jsx_closing_element" {
                    let current = self.builder.current;
                    self.builder.table.jsx.push((current, name.is_none()));
                }
                if let Some(name) = &name {
                    if self.tag(name) == Tag::Identifier {
                        let text = name.text();
                        let first = text.chars().next().unwrap_or('a');
                        if first.to_uppercase().next() == Some(first) {
                            self.builder.reference(name.range(), VALUE);
                        }
                    } else {
                        self.visit(name)?;
                    }
                }
                self.children_except(node, &[name])?;
            }
            Tag::Assignment => {
                let left = field(node, fields.left);
                if let Some(left) = &left {
                    self.write_target(left)?;
                }
                self.children_except(node, &[left])?;
            }
            Tag::Update => {
                let argument = field(node, fields.argument);
                if let Some(argument) = &argument {
                    self.write_target(argument)?;
                }
                self.children_except(node, &[argument])?;
            }
            Tag::Call => {
                if field(node, fields.function).is_some_and(|callee| self.is_eval(&callee)) {
                    let current = self.builder.current;
                    self.builder.table.dynamic.push(current);
                }
                let function =
                    field(node, fields.function).filter(|function| self.mutating_method(function));
                if let Some(function) = &function {
                    self.member_chain(function)?;
                }
                self.children_except(node, &[function])?;
            }
            Tag::With => {
                let current = self.builder.current;
                self.builder.table.dynamic.push(current);
                self.children(node)?;
            }
            _ => self.children(node)?,
        }
        self.builder.leave();
        Ok(())
    }

    /// A callee that is the name `eval`, parentheses aside: eslint-scope's direct eval.
    fn is_eval<D: Doc>(&self, callee: &Node<'_, D>) -> bool {
        let mut callee = callee.clone();
        while callee.kind() == "parenthesized_expression" {
            let inner = named(&callee).find(|child| self.tag(child) != Tag::Comment);
            match inner {
                Some(inner) => callee = inner,
                None => return false,
            }
        }
        self.tag(&callee) == Tag::Identifier && callee.text() == "eval"
    }

    /// A callee that mutates the array it is called on (eslint-utils' mutation methods), or a
    /// computed member whose name is not read here.
    fn mutating_method<D: Doc>(&self, callee: &Node<'_, D>) -> bool {
        if self.tag(callee) != Tag::MemberExpression {
            return false;
        }
        match field(callee, self.fields().property) {
            Some(property) if property.kind() == "property_identifier" => matches!(
                property.text().as_ref(),
                "copyWithin"
                    | "fill"
                    | "pop"
                    | "push"
                    | "reverse"
                    | "shift"
                    | "sort"
                    | "splice"
                    | "unshift"
            ),
            Some(_) => false,
            // A subscript: `x["push"](1)`.
            None => true,
        }
    }

    fn bind<D: Doc>(&mut self, target: Target, name: &Node<'_, D>, spaces: u8, site: Site) {
        let text = name.text();
        self.bind_name(target, &text, spaces, site, Some(NodeRef::of(name)));
    }

    /// The type a `type` field's annotation holds, without its `:`.
    fn annotation<D: Doc>(&self, node: &Node<'_, D>) -> Option<NodeRef> {
        let annotation = field(node, self.fields().ty)?;
        let mut inner = named(&annotation).filter(|child| self.tag(child) != Tag::Comment);
        Some(match (inner.next(), inner.next()) {
            (Some(ty), None) => NodeRef::of(&ty),
            _ => NodeRef::of(&annotation),
        })
    }

    fn bind_name(
        &mut self,
        target: Target,
        text: &str,
        spaces: u8,
        site: Site,
        name: Option<NodeRef>,
    ) {
        let mut scope = match target {
            Target::At(scope) => scope,
            _ => self.builder.current,
        };
        if let Target::Var = target {
            loop {
                let current = self.builder.scope(scope);
                if matches!(
                    current.kind,
                    ScopeKind::Function
                        | ScopeKind::Module
                        | ScopeKind::Namespace
                        | ScopeKind::GlobalAugmentation
                ) {
                    break;
                }
                scope = current.parent.unwrap_or(0);
            }
        }
        if self.builder.scope(scope).kind == ScopeKind::GlobalAugmentation {
            return;
        }
        let mut binding = Binding::new(0, spaces, site);
        binding.name = name;
        self.builder.bind(scope, text, binding);
    }

    /// A declaration pattern: its names bind at `target`, defaults and computed keys are read.
    fn pattern<D: Doc>(&mut self, node: &Node<'_, D>, target: Target, site: Site) -> Visit {
        self.builder.enter()?;
        let fields = self.fields();
        match self.tag(node) {
            // tree-sitter gives `undefined` its own kind, but it is an ordinary bindable name.
            Tag::Identifier | Tag::ShorthandProperty | Tag::Undefined => {
                let start = node.range().start;
                let value = self
                    .destructured
                    .iter()
                    .find(|(name, _)| *name == start)
                    .map(|(_, value)| *value);
                let site = value.map_or(site, |value| Site {
                    value: Some(value),
                    ..site
                });
                self.bind(target, node, VALUE, site)
            }
            Tag::ObjectPattern | Tag::ArrayPattern | Tag::RestPattern => {
                for child in named(node) {
                    self.pattern(&child, target, site)?;
                }
            }
            Tag::PairPattern => {
                let key = field(node, fields.key);
                if let Some(key) = &key
                    && self.tag(key) == Tag::ComputedPropertyName
                {
                    self.visit(key)?;
                }
                if let Some(value) = field(node, fields.value) {
                    self.pattern(&value, target, site)?;
                }
            }
            Tag::ObjectAssignmentPattern | Tag::AssignmentPattern => {
                let left = field(node, fields.left);
                if let Some(left) = &left {
                    self.pattern(left, target, site)?;
                }
                self.children_except(node, &[left])?;
            }
            Tag::Comment => {}
            _ => self.visit(node)?,
        }
        self.builder.leave();
        Ok(())
    }

    /// A `var`, `let` or `const` declaration. A declarator of a plain name carries its
    /// initializer as the value; a name destructured from an object or array literal the element
    /// it takes (which `value.follow` does not read, as eslint-utils reads no destructured one).
    fn declaration<D: Doc>(&mut self, node: &Node<'_, D>, target: Target) -> Visit {
        let fields = self.fields();
        for declarator in named(node) {
            if self.tag(&declarator) != Tag::VariableDeclarator {
                self.visit(&declarator)?;
                continue;
            }
            let name = field(&declarator, fields.name);
            if let Some(name) = &name {
                let plain = matches!(self.tag(name), Tag::Identifier | Tag::Undefined);
                let value = field(&declarator, fields.value);
                let site = Site {
                    value: value
                        .as_ref()
                        .filter(|_| plain)
                        .map(|value| NodeRef::of(value)),
                    annotation: self.annotation(&declarator),
                    ..site(Form::Assignment, &declarator)
                };
                if let Some(value) = value.as_ref().filter(|_| !plain) {
                    crate::code_rule_dsl::value::ecma::destructured(
                        name,
                        value,
                        &mut self.destructured,
                    );
                }
                self.pattern(name, target, site)?;
                self.destructured.clear();
            }
            self.children_except(&declarator, &[name])?;
        }
        Ok(())
    }

    fn for_in<D: Doc>(&mut self, node: &Node<'_, D>) -> Visit {
        let fields = self.fields();
        self.builder.push(ScopeKind::Block, node.range());
        let left = field(node, fields.left);
        let declared = node.children().find(|child| {
            !child.is_named() && matches!(child.kind().as_ref(), "var" | "let" | "const")
        });
        if let Some(left) = &left {
            let site = site(Form::For, node);
            match declared.map(|keyword| keyword.kind() == "var") {
                Some(true) => self.pattern(left, Target::Var, site)?,
                Some(false) => self.pattern(left, Target::Block, site)?,
                None => self.write_target(left)?,
            }
        }
        self.children_except(node, &[left])?;
        self.builder.pop();
        Ok(())
    }

    /// Parameters of the current function or signature scope: names bind, types, defaults and
    /// decorators are read in that scope.
    fn parameters<D: Doc>(&mut self, node: &Node<'_, D>) -> Visit {
        let fields = self.fields();
        for parameter in named(node) {
            let site = site(Form::Parameter, &parameter);
            match self.tag(&parameter) {
                Tag::Parameter => {
                    let pattern = field(&parameter, fields.pattern);
                    if let Some(pattern) = &pattern {
                        let site = Site {
                            annotation: self.annotation(&parameter),
                            ..site
                        };
                        self.pattern(pattern, Target::Block, site)?;
                    }
                    for child in named(&parameter) {
                        if !same(&child, pattern.as_ref()) && child.kind() != "decorator" {
                            self.visit(&child)?;
                        }
                    }
                }
                Tag::Identifier | Tag::Undefined => {
                    self.bind(Target::Block, &parameter, VALUE, site)
                }
                _ => self.pattern(&parameter, Target::Block, site)?,
            }
        }
        Ok(())
    }

    /// A function's own scope: type parameters, parameters and body share it (eslint-scope does
    /// not open a block for the body). `named` binds a function expression's name inside it.
    fn function<D: Doc>(
        &mut self,
        node: &Node<'_, D>,
        arguments: bool,
        named_inside: bool,
    ) -> Visit {
        let fields = self.fields();
        let name = field(node, fields.name);
        let decorators = node.field_children("decorator").collect::<Vec<_>>();
        for decorator in &decorators {
            self.visit(decorator)?;
        }
        // Parameter decorators run in the scope around the method (typescript-eslint's class
        // visitor), before its own.
        let parameter_decorators = named(node)
            .filter(|child| child.kind() == "formal_parameters")
            .flat_map(|parameters| named(&parameters).collect::<Vec<_>>())
            .flat_map(|parameter| {
                named(&parameter)
                    .filter(|child| child.kind() == "decorator")
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        for decorator in &parameter_decorators {
            self.visit(decorator)?;
        }
        // A named function expression binds its name in a scope of its own around the function's
        // (eslint-scope's `function-expression-name`), so a `var` of that name inside is another
        // variable.
        let name_scope = named_inside && name.is_some();
        if name_scope {
            self.builder.push(ScopeKind::FunctionName, node.range());
            if let Some(name) = &name {
                self.bind(Target::Block, name, VALUE, site(Form::Definition, node));
            }
        }
        self.builder.push(ScopeKind::Function, node.range());
        if arguments {
            self.bind_name(Target::Block, "arguments", VALUE, Site::IMPLICIT, None);
        }
        if let Some(parameter) = field(node, fields.parameter) {
            let site = site(Form::Parameter, &parameter);
            self.pattern(&parameter, Target::Block, site)?;
        }
        let body = field(node, fields.body);
        for child in named(node) {
            if same(&child, name.as_ref())
                || same(&child, body.as_ref())
                || same(&child, field(node, fields.parameter).as_ref())
                || decorators
                    .iter()
                    .any(|decorator| decorator.node_id() == child.node_id())
            {
                continue;
            }
            self.visit(&child)?;
        }
        if let Some(body) = &body {
            if self.tag(body) == Tag::StatementBlock {
                self.children(body)?;
            } else {
                self.visit(body)?;
            }
        }
        self.builder.pop();
        if name_scope {
            self.builder.pop();
        }
        Ok(())
    }

    /// A call, construct, method or index signature, or a function or constructor type: its
    /// parameters and type parameters are local to it.
    fn signature<D: Doc>(&mut self, node: &Node<'_, D>) -> Visit {
        let fields = self.fields();
        let name = field(node, fields.name);
        if let Some(name) = &name
            && self.tag(name) == Tag::ComputedPropertyName
        {
            self.visit(name)?;
        }
        self.builder.push(ScopeKind::TypeParams, node.range());
        if self.tag(node) == Tag::IndexSignature
            && let Some(name) = &name
            && self.tag(name) == Tag::Identifier
        {
            self.bind(Target::Block, name, VALUE, site(Form::Parameter, node));
        }
        self.children_except(node, &[name])?;
        self.builder.pop();
        Ok(())
    }

    /// A class: its scope binds its name (`inner_name`), which is the name node's own binding
    /// only for a class expression (`own_name`).
    fn class<D: Doc>(
        &mut self,
        node: &Node<'_, D>,
        inner_name: Option<Node<'_, D>>,
        own_name: bool,
    ) -> Visit {
        let fields = self.fields();
        let name = field(node, fields.name);
        let decorators = node.field_children("decorator").collect::<Vec<_>>();
        for decorator in &decorators {
            self.visit(decorator)?;
        }
        self.builder.push(ScopeKind::Class, node.range());
        if let Some(inner) = &inner_name {
            let text = inner.text();
            let name = own_name.then(|| NodeRef::of(inner));
            let site = site(Form::Definition, node);
            self.bind_name(Target::Block, &text, VALUE | TYPE, site, name);
        }
        for child in named(node) {
            if same(&child, name.as_ref())
                || decorators
                    .iter()
                    .any(|decorator| decorator.node_id() == child.node_id())
            {
                continue;
            }
            self.visit(&child)?;
        }
        self.builder.pop();
        Ok(())
    }

    /// `namespace A.B { }` binds `A`; `declare module "m" { }` binds nothing. The body is a scope.
    fn module<D: Doc>(&mut self, node: &Node<'_, D>) -> Visit {
        let fields = self.fields();
        let name = field(node, fields.name);
        if let Some(name) = &name {
            let mut head = name.clone();
            while self.tag(&head) == Tag::NestedIdentifier {
                match field(&head, fields.object) {
                    Some(object) => head = object,
                    None => break,
                }
            }
            if self.tag(&head) == Tag::Identifier {
                self.bind(
                    Target::Block,
                    &head,
                    VALUE | TYPE,
                    site(Form::Definition, node),
                );
            }
        }
        self.builder.push(ScopeKind::Namespace, node.range());
        if let Some(body) = field(node, fields.body) {
            self.children(&body)?;
        }
        self.builder.pop();
        Ok(())
    }

    /// Default, namespace and named imports, and `import x = require()`: each binds both spaces.
    fn import<D: Doc>(&mut self, node: &Node<'_, D>) -> Visit {
        let fields = self.fields();
        for child in named(node) {
            match self.tag(&child) {
                Tag::ImportClause => {
                    for part in named(&child) {
                        match self.tag(&part) {
                            Tag::Identifier => {
                                let site = site(Form::Import, &child);
                                self.bind(Target::Block, &part, VALUE | TYPE, site)
                            }
                            Tag::NamespaceImport => {
                                if let Some(name) =
                                    named(&part).find(|name| self.tag(name) == Tag::Identifier)
                                {
                                    let site = site(Form::Import, &part);
                                    self.bind(Target::Block, &name, VALUE | TYPE, site);
                                }
                            }
                            Tag::NamedImports => {
                                for specifier in named(&part) {
                                    if self.tag(&specifier) != Tag::ImportSpecifier {
                                        continue;
                                    }
                                    let local = field(&specifier, fields.alias)
                                        .or_else(|| field(&specifier, fields.name));
                                    if let Some(local) = local
                                        && self.tag(&local) == Tag::Identifier
                                    {
                                        let site = site(Form::Import, &specifier);
                                        self.bind(Target::Block, &local, VALUE | TYPE, site);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Tag::ImportRequireClause => {
                    if let Some(name) = named(&child).find(|name| self.tag(name) == Tag::Identifier)
                    {
                        let site = site(Form::Import, &child);
                        self.bind(Target::Block, &name, VALUE | TYPE, site);
                    }
                }
                Tag::ImportAlias => self.visit(&child)?,
                _ => {}
            }
        }
        Ok(())
    }

    /// The target of an assignment, update or `for ... in/of` without a declaration: its names
    /// are written; a member chain's head is read and its value mutated; defaults, computed keys
    /// and subscripts are read. TypeScript's wrappers (`x!`, `x as T`) are transparent.
    fn write_target<D: Doc>(&mut self, node: &Node<'_, D>) -> Visit {
        self.builder.enter()?;
        let fields = self.fields();
        match self.tag(node) {
            Tag::Identifier | Tag::ShorthandProperty | Tag::Undefined => self
                .builder
                .push_reference(node.range(), VALUE, true, false),
            Tag::ObjectPattern | Tag::ArrayPattern | Tag::RestPattern => {
                for child in named(node) {
                    self.write_target(&child)?;
                }
            }
            Tag::Wrapper => {
                let inner = self.wrapped(node);
                for child in named(node) {
                    if same(&child, inner.as_ref()) {
                        self.write_target(&child)?;
                    } else {
                        self.visit(&child)?;
                    }
                }
            }
            Tag::Comment => {}
            Tag::PairPattern => {
                let key = field(node, fields.key);
                if let Some(key) = &key
                    && self.tag(key) == Tag::ComputedPropertyName
                {
                    self.visit(key)?;
                }
                if let Some(value) = field(node, fields.value) {
                    self.write_target(&value)?;
                }
            }
            Tag::ObjectAssignmentPattern | Tag::AssignmentPattern => {
                let left = field(node, fields.left);
                if let Some(left) = &left {
                    self.write_target(left)?;
                }
                self.children_except(node, &[left])?;
            }
            Tag::MemberExpression => self.member_chain(node)?,
            _ => self.visit(node)?,
        }
        self.builder.leave();
        Ok(())
    }

    /// A member chain whose value is mutated (`x.a.b = 1`, `x[0]++`, `x.push(1)`): its head name
    /// is read, and marked as mutating its value; keys and subscripts are read.
    fn member_chain<D: Doc>(&mut self, node: &Node<'_, D>) -> Visit {
        let mut head = node.clone();
        let mut parts = Vec::new();
        loop {
            match self.tag(&head) {
                Tag::MemberExpression => match field(&head, self.fields().object) {
                    Some(object) => {
                        parts.push(head.clone());
                        head = object;
                    }
                    None => break,
                },
                Tag::Wrapper => {
                    let inner = self.wrapped(&head);
                    match inner {
                        Some(inner) => {
                            parts.push(head.clone());
                            head = inner;
                        }
                        None => break,
                    }
                }
                _ => break,
            }
        }
        match self.tag(&head) {
            Tag::Identifier | Tag::Undefined => {
                self.builder
                    .push_reference(head.range(), VALUE, false, true);
            }
            _ => self.visit(&head)?,
        }
        // Everything in the chain besides its head: computed keys, type arguments. eslint-utils
        // walks up from any name through member expressions, so a computed key's name (or the
        // head of a computed key's chain) mutates its value too.
        for part in &parts {
            let index = (part.kind() == "subscript_expression")
                .then(|| field(part, self.fields().index))
                .flatten();
            for child in named(part) {
                let inner = parts.iter().any(|other| other.node_id() == child.node_id())
                    || child.node_id() == head.node_id();
                if inner {
                    continue;
                }
                if same(&child, index.as_ref()) {
                    match self.tag(&child) {
                        Tag::Identifier | Tag::Undefined => {
                            self.builder
                                .push_reference(child.range(), VALUE, false, true);
                        }
                        Tag::MemberExpression => self.member_chain(&child)?,
                        _ => self.visit(&child)?,
                    }
                } else {
                    self.visit(&child)?;
                }
            }
        }
        Ok(())
    }

    /// The expression inside a wrapper: `(x)`, `x!`, `x as T`, `x satisfies T`, `<T>x`.
    fn wrapped<'r, D: Doc>(&self, node: &Node<'r, D>) -> Option<Node<'r, D>> {
        let mut children = named(node).filter(|child| self.tag(child) != Tag::Comment);
        if node.kind() == "type_assertion" {
            children.last()
        } else {
            children.next()
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
