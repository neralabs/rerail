//! TypeScript and JavaScript usedness, after ESLint 10.11.0's no-unused-vars (`isUsedVariable`)
//! and typescript-eslint 8.70.1's (`collectVariables`): a variable is one scope's bindings of one
//! name, and it is used when one of its references reads it for something other than itself.
//!
//! The walk pairs each reference and binding name of the table with its node, to read what
//! ESLint reads from the ESTree around it. tree-sitter's parentheses are transparent, as ESTree
//! has none; TypeScript's wrappers (`x!`, `x as T`) are not.

use super::super::{
    Binding, BuildError, Form, MAX_BUILD_DEPTH, MAX_BUILD_STACK_BYTES, NodeRef, ScopeKind,
    ScopeTable, stack_address,
};
use super::{Facts, PrivateDeclaration, PrivateMember, Steps, known};
use crate::code_rule_dsl::language::kind_ids;
use ast_grep_core::{Doc, Node};
use foldhash::{HashMap, HashMapExt};
use std::collections::BTreeSet;

/// A variable: a scope and one of its names.
type Key<'t> = (u32, &'t str);

/// What the usage index keeps of a TypeScript table.
pub(super) struct Built {
    pub(super) facts: Vec<HashMap<Box<str>, Vec<Facts>>>,
    /// Names `/* exported */` block comments list (ESLint marks the global scope's variables of
    /// those names used).
    pub(super) comment_exports: BTreeSet<Box<str>>,
    pub(super) occurrences: HashMap<(u32, Box<str>), Vec<NodeRef>>,
    pub(super) privates: Vec<PrivateMember>,
}

/// One reference of a variable, as `isUsedVariable` reads it.
struct Ref<'r, D: Doc> {
    start: usize,
    end: usize,
    read: bool,
    write: bool,
    /// The scope the reference is made from.
    from: u32,
    /// `a += 1` or `a++` as a statement (or a non-final sequence item): a read for itself.
    self_update: bool,
    /// The left side of `a = ...` (any operator) as a statement: the right side's range.
    assign_rhs: Option<(usize, usize)>,
    in_loop: bool,
    /// The reference's function scope (eslint-scope's `variableScope`): a table scope, or `None`
    /// for a class field initializer, which is one of its own.
    variable_scope: Option<u32>,
    /// `for (x in o) return;`: ESLint counts the loop variable as used.
    for_in_of: bool,
    /// Inside a `typeof` type query or a type predicate.
    type_use: bool,
    /// The identifier, for the rare storable-function test.
    node: Option<Node<'r, D>>,
}

struct Variable<'r, D: Doc> {
    refs: Vec<Ref<'r, D>>,
    /// typescript-eslint's `markVariableAsUsed` (and base ESLint's skips): enum members, mapped
    /// type keys, signature and setter parameters, parameter properties, a class expression's own
    /// name, `arguments`, declarations of a `declare module` body.
    marked: bool,
    exported: bool,
    /// Function, namespace and enum nodes whose own scopes' reads are self-references.
    self_scopes: Vec<(usize, usize)>,
    /// Interface and type alias nodes whose reads are self-references.
    self_inside: Vec<(usize, usize)>,
    /// Some definition is an import: a reference in a type query reads it (typescript-eslint
    /// does not report an import used only as a type).
    imports: bool,
    /// Its definition names and references.
    occurrences: Vec<NodeRef>,
}

impl<D: Doc> Default for Variable<'_, D> {
    fn default() -> Self {
        Self {
            refs: Vec::new(),
            marked: false,
            exported: false,
            self_scopes: Vec::new(),
            self_inside: Vec::new(),
            imports: false,
            occurrences: Vec::new(),
        }
    }
}

/// A binding's name node, with the binding.
struct NameEntry<'t> {
    start: usize,
    end: usize,
    kind: u16,
    key: Key<'t>,
    index: usize,
}

const FUNCTIONS: &[&str] = &[
    "function_declaration",
    "generator_function_declaration",
    "function_expression",
    "function",
    "generator_function",
    "arrow_function",
    "method_definition",
];
const LOOPS: &[&str] = &[
    "for_statement",
    "for_in_statement",
    "while_statement",
    "do_statement",
];
const WRAPPERS: &[&str] = &[
    "parenthesized_expression",
    "non_null_expression",
    "as_expression",
    "satisfies_expression",
    "type_assertion",
];

fn named_children<'a, 'r, D: Doc>(node: &'a Node<'r, D>) -> impl Iterator<Item = Node<'r, D>> + 'a {
    node.children()
        .filter(|child| child.is_named() && child.kind() != "comment")
}

fn same<D: Doc>(left: &Node<'_, D>, right: &Node<'_, D>) -> bool {
    left.range() == right.range() && left.kind_id() == right.kind_id()
}

/// The expression inside any parentheses around `node`.
fn unparenthesized<'r, D: Doc>(mut node: Node<'r, D>) -> Node<'r, D> {
    while node.kind() == "parenthesized_expression" {
        let inner = named_children(&node).next();
        match inner {
            Some(inner) => node = inner,
            None => break,
        }
    }
    node
}

fn has_token<D: Doc>(node: &Node<'_, D>, tokens: &[&str]) -> bool {
    node.children()
        .any(|child| !child.is_named() && tokens.contains(&child.kind().as_ref()))
}

/// A class member's key: tree-sitter-javascript's `field_definition` names it `property`;
/// tree-sitter-typescript's `public_field_definition` and both grammars' `method_definition`,
/// `name`.
fn member_key<'r, D: Doc>(member: &Node<'r, D>) -> Option<Node<'r, D>> {
    member.field(if member.kind() == "field_definition" {
        "property"
    } else {
        "name"
    })
}

struct Walk<'t, 'r, 'a, 'd, D: Doc> {
    table: &'t ScopeTable,
    text: &'a str,
    base: usize,
    next_reference: usize,
    names: Vec<NameEntry<'t>>,
    next_name: usize,
    ancestors: Vec<Node<'r, D>>,
    variables: HashMap<Key<'t>, Variable<'r, D>>,
    comment_exports: BTreeSet<Box<str>>,
    steps: &'a mut Steps<'d>,
    stack_base: usize,
    /// Class bodies and private names, when the file has a `#`.
    private_kinds: Option<(BTreeSet<u16>, BTreeSet<u16>)>,
    /// The private names of each class body the walk is in, innermost last.
    classes: Vec<HashMap<Box<str>, usize>>,
    privates: Vec<PrivateMember>,
}

impl<'t, 'r, D: Doc> Walk<'t, 'r, '_, '_, D> {
    fn visit(&mut self, node: Node<'r, D>) -> Result<(), BuildError> {
        self.steps.take()?;
        if self.ancestors.len() as u32 > MAX_BUILD_DEPTH
            || self.stack_base.abs_diff(stack_address()) > MAX_BUILD_STACK_BYTES
        {
            return Err(BuildError::Depth);
        }
        let range = node.range();
        if node.kind() == "comment" {
            self.comment(&node);
            return Ok(());
        }
        while self
            .table
            .references
            .get(self.next_reference)
            .is_some_and(|reference| reference.start < range.start)
        {
            self.next_reference += 1;
        }
        if let Some(reference) = self.table.references.get(self.next_reference)
            && reference.start == range.start
            && reference.end == range.end
            && node.children().next().is_none()
        {
            self.next_reference += 1;
            self.reference(*reference, &node);
        }
        while self
            .names
            .get(self.next_name)
            .is_some_and(|name| name.start < range.start)
        {
            self.next_name += 1;
        }
        while let Some(entry) = self.names.get(self.next_name)
            && entry.start == range.start
            && entry.end == range.end
            && entry.kind == node.kind_id()
        {
            let (key, index) = (entry.key, entry.index);
            self.next_name += 1;
            self.name(key, index, &node);
        }
        let mut class_body = false;
        if let Some((bodies, names)) = &self.private_kinds {
            let kind = node.kind_id();
            if bodies.contains(&kind) {
                class_body = true;
            } else if names.contains(&kind) {
                self.private_name(&node);
            }
        }
        if class_body {
            self.enter_class_body(&node);
        }
        self.ancestors.push(node.clone());
        for child in node.children().filter(|child| child.is_named()) {
            self.visit(child)?;
        }
        self.ancestors.pop();
        if class_body {
            self.classes.pop();
        }
        Ok(())
    }

    /// ESLint's `ClassBody` visit: every field, method and accessor with a private name declares
    /// it, a getter and setter one member together.
    fn enter_class_body(&mut self, body: &Node<'r, D>) {
        let mut members: HashMap<Box<str>, usize> = HashMap::new();
        for member in named_children(body) {
            let definition = match member.kind().as_ref() {
                "public_field_definition" | "field_definition" => Form::Field,
                "method_definition" if has_token(&member, &["get", "set"]) => Form::Accessor,
                "method_definition" => Form::Method,
                _ => continue,
            };
            let Some(name) =
                member_key(&member).filter(|name| name.kind() == "private_property_identifier")
            else {
                continue;
            };
            let field = definition == Form::Field;
            let annotation = member
                .field("type")
                .filter(|_| field)
                .and_then(|annotation| named_children(&annotation).next());
            let declaration = PrivateDeclaration {
                name: NodeRef::of(&name),
                form: definition,
                site: NodeRef::of(&member),
                value: member
                    .field("value")
                    .filter(|_| field)
                    .map(|value| NodeRef::of(&value)),
                annotation: annotation.map(|annotation| NodeRef::of(&annotation)),
            };
            let index = *members.entry(name.text().into()).or_insert_with(|| {
                self.privates.push(PrivateMember::default());
                self.privates.len() - 1
            });
            let private = &mut self.privates[index];
            private.occurrences.push(declaration.name);
            private.declarations.push(declaration);
        }
        self.classes.push(members);
    }

    /// ESLint's `PrivateIdentifier` visit: a reference names the innermost enclosing class body's
    /// member of its name, and uses it unless it only writes it (`isWriteOnlyAssignment` and the
    /// rule's update and destructuring cases); any reference uses an accessor.
    fn private_name(&mut self, node: &Node<'r, D>) {
        let Some(parent) = self.ancestors.last() else {
            return;
        };
        let declaration = matches!(
            parent.kind().as_ref(),
            "public_field_definition" | "field_definition" | "method_definition"
        ) && member_key(parent).is_some_and(|name| same(&name, node));
        if declaration {
            return;
        }
        let name = node.text();
        let Some(index) = self
            .classes
            .iter()
            .rev()
            .find_map(|members| members.get(name.as_ref()).copied())
        else {
            return;
        };
        let write_only = self.private_write_only(node);
        let private = &mut self.privates[index];
        private.occurrences.push(NodeRef::of(node));
        private.referenced = true;
        let accessor = private
            .declarations
            .last()
            .is_some_and(|declaration| declaration.form == Form::Accessor);
        if accessor || !write_only {
            private.used = true;
        }
    }

    /// Whether the private name `node` is the property of a member expression that is only
    /// written: the left side of `=`, of a compound assignment whose value is a statement's, or of
    /// a `for ... in/of` or pattern default; a statement's update; a destructuring target. ESTree
    /// has no parentheses; TypeScript's wrappers (`!`, `as`) make it a read.
    fn private_write_only(&self, node: &Node<'r, D>) -> bool {
        let Some(member) = self.ancestors.last() else {
            return false;
        };
        if member.kind() != "member_expression"
            || !member
                .field("property")
                .is_some_and(|property| same(&property, node))
        {
            return false;
        }
        let parens = &["parenthesized_expression"];
        let Some((index, child)) = self.parent_below(self.ancestors.len() - 1, member, parens)
        else {
            return false;
        };
        let wrapping = &self.ancestors[index];
        let left = || {
            wrapping
                .field("left")
                .is_some_and(|left| same(&left, &child))
        };
        let statement = || {
            self.parent_below(index, wrapping, parens)
                .is_some_and(|(parent, _)| self.ancestors[parent].kind() == "expression_statement")
        };
        match wrapping.kind().as_ref() {
            "assignment_expression" | "for_in_statement" | "assignment_pattern" => left(),
            "augmented_assignment_expression" => left() && statement(),
            "update_expression" => statement(),
            "pair_pattern" => wrapping
                .field("value")
                .is_some_and(|value| same(&value, &child)),
            "rest_pattern" | "array_pattern" => true,
            _ => false,
        }
    }

    /// ESLint's `/* exported a, b */` block comments.
    fn comment(&mut self, node: &Node<'r, D>) {
        let text = node.text();
        let Some(body) = text
            .strip_prefix("/*")
            .and_then(|body| body.strip_suffix("*/"))
        else {
            return;
        };
        let directive = directive_part(body).trim();
        let Some(rest) = directive.strip_prefix("exported") else {
            return;
        };
        if !(rest.is_empty() || rest.starts_with(char::is_whitespace)) {
            return;
        }
        for item in rest.split(',') {
            let item = item.trim();
            let item = ['"', '\'']
                .iter()
                .find_map(|quote| {
                    item.strip_prefix(*quote)
                        .and_then(|inner| inner.strip_suffix(*quote))
                })
                .unwrap_or(item);
            if !item.is_empty() {
                self.comment_exports.insert(item.into());
            }
        }
    }

    /// The node's nearest ancestor that is not one of `skip`, with the child of it on the path.
    fn parent_skipping(&self, node: &Node<'r, D>, skip: &[&str]) -> Option<(usize, Node<'r, D>)> {
        self.parent_below(self.ancestors.len(), node, skip)
    }

    /// The same for the ancestor at `below`, `node`: the nearest one above it.
    fn parent_below(
        &self,
        below: usize,
        node: &Node<'r, D>,
        skip: &[&str],
    ) -> Option<(usize, Node<'r, D>)> {
        let mut child = node.clone();
        for index in (0..below).rev() {
            let ancestor = &self.ancestors[index];
            if !skip.contains(&ancestor.kind().as_ref()) {
                return Some((index, child));
            }
            child = ancestor.clone();
        }
        None
    }

    /// ESLint's `isUnusedExpression`: the value of the expression at `index` is discarded.
    fn unused_expression(&self, index: usize) -> bool {
        let expression = &self.ancestors[index];
        let Some((parent, child)) =
            self.parent_below(index, expression, &["parenthesized_expression"])
        else {
            return false;
        };
        let parent_node = &self.ancestors[parent];
        match parent_node.kind().as_ref() {
            "expression_statement" => true,
            "sequence_expression" => {
                let last = named_children(parent_node).last();
                match last {
                    Some(last) if same(&last, &child) => self.unused_expression(parent),
                    _ => true,
                }
            }
            _ => false,
        }
    }

    fn reference(&mut self, reference: super::super::Reference, node: &Node<'r, D>) {
        let Some(name) = self
            .text
            .get(reference.start - self.base..reference.end - self.base)
        else {
            return;
        };
        let Some((scope, _)) = self.table.resolve_lexical_index(&reference, name) else {
            return;
        };
        let Some((key, _)) = self.table.scopes[scope as usize]
            .bindings
            .get_key_value(name)
        else {
            return;
        };
        let key = (scope, key.as_ref());
        let parent = self.parent_skipping(node, &["parenthesized_expression"]);
        // ESTree has no parentheses: `(a) += 1` assigns to `a` itself.
        let assignment_left = |(index, _): &(usize, Node<'r, D>), kinds: &[&str]| {
            let parent = &self.ancestors[*index];
            kinds.contains(&parent.kind().as_ref())
                && parent
                    .field("left")
                    .is_some_and(|left| same(&unparenthesized(left), node))
        };
        let compound = self
            .parent_skipping(node, WRAPPERS)
            .is_some_and(|(index, child)| {
                let parent = &self.ancestors[index];
                match parent.kind().as_ref() {
                    "augmented_assignment_expression" => parent
                        .field("left")
                        .is_some_and(|left| left.range() == child.range()),
                    "update_expression" => true,
                    _ => false,
                }
            });
        let write = reference.write;
        let read = !write || compound;
        let mut self_update = false;
        let mut assign_rhs = None;
        let mut for_in_of = false;
        if let Some(parent) = &parent {
            let parent_node = &self.ancestors[parent.0];
            let logical = has_token(parent_node, &["||=", "&&=", "??="]);
            if assignment_left(parent, &["augmented_assignment_expression"])
                && !logical
                && self.unused_expression(parent.0)
            {
                self_update = true;
            }
            if parent_node.kind() == "update_expression" && self.unused_expression(parent.0) {
                self_update = true;
            }
            if assignment_left(
                parent,
                &["assignment_expression", "augmented_assignment_expression"],
            ) && self.unused_expression(parent.0)
            {
                assign_rhs = parent_node
                    .field("right")
                    .map(|right| (right.range().start, right.range().end));
            }
            for_in_of = self.for_in_of(parent, node);
        }
        let in_loop = assign_rhs.is_some() && self.in_loop();
        let variable_scope = self.variable_scope(reference.scope);
        let type_use = self.type_use(node);
        let keep_node = read
            && self
                .ancestors
                .iter()
                .any(|ancestor| FUNCTIONS.contains(&ancestor.kind().as_ref()));
        // A destructuring assignment's defaults write again (`[a = 1] = b`), as eslint-scope's
        // `referencingDefaultValue`.
        let defaults = if write { self.defaults(node) } else { 0 };
        let variable = self.variables.entry(key).or_default();
        variable.occurrences.push(NodeRef::of(node));
        for _ in 0..defaults {
            variable.refs.push(Ref {
                start: reference.start,
                end: reference.end,
                read: false,
                write: true,
                from: reference.scope,
                self_update: false,
                assign_rhs: None,
                in_loop: false,
                variable_scope,
                for_in_of: false,
                type_use: false,
                node: None,
            });
        }
        variable.refs.push(Ref {
            start: reference.start,
            end: reference.end,
            read,
            write,
            from: reference.scope,
            self_update,
            assign_rhs,
            in_loop,
            variable_scope,
            for_in_of,
            type_use,
            node: keep_node.then(|| node.clone()),
        });
    }

    /// ESLint's `isForInOfRef`: the node is a `for ... in/of` loop's variable, and the loop's body
    /// is `return` or a block whose first statement is.
    fn for_in_of(&self, (index, child): &(usize, Node<'r, D>), node: &Node<'r, D>) -> bool {
        let parent = &self.ancestors[*index];
        if parent.kind() != "for_in_statement"
            || !same(child, node)
            || !parent.field("left").is_some_and(|left| same(&left, node))
        {
            return false;
        }
        let Some(body) = parent.field("body") else {
            return false;
        };
        let first = if body.kind() == "statement_block" {
            named_children(&body).next()
        } else {
            Some(body)
        };
        first.is_some_and(|first| first.kind() == "return_statement")
    }

    /// ESLint's `isInLoop`: a loop encloses the node within its function.
    fn in_loop(&self) -> bool {
        for ancestor in self.ancestors.iter().rev() {
            let kind = ancestor.kind();
            if FUNCTIONS.contains(&kind.as_ref()) {
                return false;
            }
            if LOOPS.contains(&kind.as_ref()) {
                return true;
            }
        }
        false
    }

    /// The function scope of a reference made from `scope`: `None` inside a class field
    /// initializer (outside any function in it), which eslint-scope gives a variable scope of its
    /// own.
    fn variable_scope(&self, scope: u32) -> Option<u32> {
        for ancestor in self.ancestors.iter().rev() {
            match ancestor.kind().as_ref() {
                "public_field_definition" | "field_definition" => return None,
                kind if FUNCTIONS.contains(&kind) => break,
                "class_static_block" | "internal_module" | "module" | "program" => break,
                _ => {}
            }
        }
        Some(function_scope(self.table, scope))
    }

    /// typescript-eslint's `referenceContainsTypeQuery` and `referenceContainsTypePredicate`.
    fn type_use(&self, node: &Node<'r, D>) -> bool {
        let mut child = node.clone();
        for ancestor in self.ancestors.iter().rev() {
            match ancestor.kind().as_ref() {
                "member_expression" | "nested_identifier" | "nested_type_identifier"
                    if ancestor.child(0).is_some_and(|first| same(&first, &child)) =>
                {
                    child = ancestor.clone();
                }
                // The queried expression or the predicate's parameter, not its type.
                "type_query" | "type_predicate" | "asserts" => {
                    return named_children(ancestor)
                        .next()
                        .is_some_and(|first| same(&first, &child));
                }
                _ => return false,
            }
        }
        false
    }

    fn name(&mut self, key: Key<'t>, index: usize, node: &Node<'r, D>) {
        let binding = &self.table.scopes[key.0 as usize].bindings[key.1][index];
        let scope_kind = self.table.scopes[key.0 as usize].kind;
        let site = binding.site.node.and_then(|site| {
            self.ancestors.iter().rposition(|ancestor| {
                let range = ancestor.range();
                range.start == site.start as usize
                    && range.end == site.end as usize
                    && ancestor.kind_id() == site.kind
            })
        });
        // A name that is its own site: an enum member, an arrow function's lone parameter.
        let own_site = binding.site.node.is_some_and(|site| {
            let range = node.range();
            site.start as usize == range.start
                && site.end as usize == range.end
                && site.kind == node.kind_id()
        });
        let site_kind = match site {
            Some(index) => Some(self.ancestors[index].kind().into_owned()),
            None if own_site => Some(node.kind().into_owned()),
            None => None,
        };
        let marked = self.marked(binding, scope_kind, site_kind.as_deref());
        let exported = site.is_some_and(|index| self.exported(binding, index));
        let writes = self.definition_writes(binding, node);
        let mut self_scopes = Vec::new();
        let mut self_inside = Vec::new();
        if let (Some(index), Some(kind)) = (site, site_kind.as_deref()) {
            let range = self.ancestors[index].range();
            let range = (range.start, range.end);
            match kind {
                "function_declaration"
                | "generator_function_declaration"
                | "function_expression"
                | "function"
                | "generator_function"
                | "internal_module"
                | "module"
                | "enum_declaration"
                    if binding.site.form == Form::Definition =>
                {
                    self_scopes.push(range)
                }
                "interface_declaration" | "type_alias_declaration" => self_inside.push(range),
                "variable_declarator" => {
                    if let Some(value) = self.ancestors[index].field("value").map(unparenthesized)
                        && matches!(
                            value.kind().as_ref(),
                            "function_expression"
                                | "function"
                                | "generator_function"
                                | "arrow_function"
                        )
                    {
                        self_scopes.push((value.range().start, value.range().end));
                    }
                }
                _ => {}
            }
        }
        let variable_scope = Some(function_scope(self.table, key.0));
        let for_in_of = self
            .ancestors
            .len()
            .checked_sub(1)
            .is_some_and(|parent| self.for_in_of(&(parent, node.clone()), node));
        let range = node.range();
        let variable = self.variables.entry(key).or_default();
        variable.marked |= marked;
        variable.exported |= exported;
        variable.self_scopes.extend(self_scopes);
        variable.self_inside.extend(self_inside);
        variable.imports |= binding.site.form == Form::Import;
        variable.occurrences.push(NodeRef::of(node));
        for _ in 0..writes {
            variable.refs.push(Ref {
                start: range.start,
                end: range.end,
                read: false,
                write: true,
                from: key.0,
                self_update: false,
                assign_rhs: None,
                in_loop: false,
                variable_scope,
                for_in_of,
                type_use: false,
                node: None,
            });
        }
    }

    /// How many defaults a pattern puts around `node` (`a = 1` in `[a = 1]`, `{ a = 1 }`).
    fn defaults(&self, node: &Node<'r, D>) -> usize {
        let mut defaults = 0;
        let mut child = node.clone();
        for ancestor in self.ancestors.iter().rev() {
            match ancestor.kind().as_ref() {
                "object_assignment_pattern" | "assignment_pattern" => {
                    defaults += usize::from(
                        ancestor
                            .field("left")
                            .is_some_and(|left| same(&left, &child)),
                    );
                }
                "pair_pattern"
                | "object_pattern"
                | "array_pattern"
                | "rest_pattern"
                | "parenthesized_expression" => {}
                _ => break,
            }
            child = ancestor.clone();
        }
        defaults
    }

    /// The write references eslint-scope makes at a definition's name: one per default around it
    /// (`a = 1` in a pattern or parameter), and one for a declarator's initializer or a `for ...
    /// in/of` loop's declaration.
    fn definition_writes(&self, binding: &Binding, node: &Node<'r, D>) -> usize {
        if !matches!(
            binding.site.form,
            Form::Assignment | Form::Parameter | Form::Except | Form::For
        ) {
            return 0;
        }
        let mut writes = 0;
        let mut child = node.clone();
        for ancestor in self.ancestors.iter().rev() {
            let is_field = |field: &str| {
                ancestor
                    .field(field)
                    .is_some_and(|value| same(&value, &child))
            };
            match ancestor.kind().as_ref() {
                "object_assignment_pattern" | "assignment_pattern" => {
                    writes += usize::from(is_field("left"));
                }
                "pair_pattern" | "object_pattern" | "array_pattern" | "rest_pattern" => {}
                "required_parameter" | "optional_parameter" => {
                    writes += usize::from(is_field("pattern") && ancestor.field("value").is_some());
                    break;
                }
                "variable_declarator" => {
                    writes += usize::from(is_field("name") && ancestor.field("value").is_some());
                    break;
                }
                "for_in_statement" => {
                    let declared = has_token(ancestor, &["var", "let", "const"]);
                    writes += usize::from(is_field("left") && declared);
                    break;
                }
                _ => break,
            }
            child = ancestor.clone();
        }
        writes
    }

    /// Whether the definition is exported: its declaration (a declarator's statement) stands in an
    /// `export` statement, `declare` aside (ESLint's `isExported`, over every definition as
    /// typescript-eslint reads it).
    fn exported(&self, binding: &Binding, site: usize) -> bool {
        if matches!(
            binding.site.form,
            Form::Parameter | Form::Except | Form::For | Form::TypeParameter | Form::Implicit
        ) {
            return false;
        }
        let site_node = &self.ancestors[site];
        if binding.site.form == Form::Import && site_node.kind() != "import_alias" {
            return false;
        }
        let mut index = site;
        if site_node.kind() == "variable_declarator" {
            let Some(statement) = index.checked_sub(1) else {
                return false;
            };
            index = statement;
        }
        while let Some(parent) = index.checked_sub(1) {
            match self.ancestors[parent].kind().as_ref() {
                "ambient_declaration" => index = parent,
                "export_statement" => return true,
                _ => return false,
            }
        }
        false
    }

    /// typescript-eslint's `markVariableAsUsed` calls, which base ESLint mirrors by skipping the
    /// same variables.
    fn marked(&self, binding: &Binding, scope: ScopeKind, site: Option<&str>) -> bool {
        let parent = self.ancestors.last();
        match binding.site.form {
            // Signature parameters (`(a: T) => U`, overloads, `declare function`, methods of
            // interfaces and abstract classes) and index signature keys.
            Form::Parameter if scope == ScopeKind::TypeParams => return true,
            Form::Parameter => {
                // `constructor(private x)`.
                if let Some(parent) = parent
                    && matches!(
                        parent.kind().as_ref(),
                        "required_parameter" | "optional_parameter"
                    )
                    && (parent.children().any(|child| {
                        child.kind() == "accessibility_modifier"
                            || child.kind() == "override_modifier"
                    }) || has_token(parent, &["readonly"]))
                {
                    return true;
                }
                // A setter's parameters.
                if let Some(function) = self
                    .ancestors
                    .iter()
                    .rev()
                    .find(|ancestor| FUNCTIONS.contains(&ancestor.kind().as_ref()))
                    && function.kind() == "method_definition"
                    && has_token(function, &["set"])
                {
                    return true;
                }
            }
            // Enum members and mapped type keys.
            Form::TypeParameter
                if matches!(
                    site,
                    Some("property_identifier" | "enum_assignment" | "mapped_type_clause")
                ) =>
            {
                return true;
            }
            // A class expression's own name, in its class scope.
            Form::Definition if scope == ScopeKind::Class => return true,
            // `namespace A.B {}`: typescript-eslint declares nothing for a qualified name.
            Form::Definition
                if site == Some("internal_module")
                    && binding.site.node.is_some_and(|site| {
                        self.ancestors.iter().rev().any(|ancestor| {
                            ancestor.range().start == site.start as usize
                                && ancestor.kind_id() == site.kind
                                && ancestor
                                    .field("name")
                                    .is_some_and(|name| name.kind() == "nested_identifier")
                        })
                    }) =>
            {
                return true;
            }
            Form::Implicit => return true,
            _ => {}
        }
        self.declared_module_member(binding)
    }

    /// A declaration directly in the body of a `declare module` or `declare namespace` (or of a
    /// module nested in one) whose body re-exports nothing (typescript-eslint's ambient
    /// declaration selectors outside `.d.ts` files).
    fn declared_module_member(&self, binding: &Binding) -> bool {
        let Some(site) = binding.site.node else {
            return false;
        };
        let Some(mut index) = self.ancestors.iter().rposition(|ancestor| {
            ancestor.range().start == site.start as usize && ancestor.kind_id() == site.kind
        }) else {
            return false;
        };
        let kind = self.ancestors[index].kind();
        let statement = match kind.as_ref() {
            "variable_declarator" => {
                let Some(statement) = index.checked_sub(1) else {
                    return false;
                };
                statement
            }
            "interface_declaration"
            | "type_alias_declaration"
            | "class_declaration"
            | "abstract_class_declaration"
            | "function_signature"
            | "enum_declaration"
            | "internal_module"
            | "module" => index,
            _ => return false,
        };
        index = statement;
        // A namespace statement stands in an expression statement.
        if let Some(parent) = index.checked_sub(1)
            && self.ancestors[parent].kind() == "expression_statement"
        {
            index = parent;
        }
        let (Some(block), Some(module)) = (index.checked_sub(1), index.checked_sub(2)) else {
            return false;
        };
        if self.ancestors[block].kind() != "statement_block"
            || !matches!(
                self.ancestors[module].kind().as_ref(),
                "internal_module" | "module"
            )
        {
            return false;
        }
        // `declare module`, `declare namespace` or `declare global`, around the module or it.
        let declared = self.ancestors[..module]
            .iter()
            .any(|ancestor| ancestor.kind() == "ambient_declaration");
        declared && !overriding_exports(&self.ancestors[block])
    }
}

/// typescript-eslint's `hasOverridingExportStatement`: an `export { }` list, `export *`,
/// `export =` or `export default` of a name.
fn overriding_exports<D: Doc>(block: &Node<'_, D>) -> bool {
    named_children(block).any(|statement| {
        if statement.kind() != "export_statement" || statement.field("declaration").is_some() {
            return false;
        }
        if has_token(&statement, &["default"]) {
            return statement
                .field("value")
                .is_some_and(|value| value.kind() == "identifier");
        }
        true
    })
}

/// ESLint's directive text: before a ` -- ` description.
fn directive_part(body: &str) -> &str {
    let bytes = body.as_bytes();
    for index in 0..bytes.len() {
        if bytes[index].is_ascii_whitespace() && bytes.get(index + 1) == Some(&b'-') {
            let mut end = index + 1;
            while bytes.get(end) == Some(&b'-') {
                end += 1;
            }
            if end - index > 2 && bytes.get(end).is_some_and(u8::is_ascii_whitespace) {
                return &body[..index];
            }
        }
    }
    body
}

/// The function, namespace or top-level scope that `scope` belongs to (eslint-scope's
/// `variableScope`).
pub(crate) fn function_scope(table: &ScopeTable, mut scope: u32) -> u32 {
    loop {
        let current = &table.scopes[scope as usize];
        if matches!(
            current.kind,
            ScopeKind::Function | ScopeKind::Module | ScopeKind::Namespace
        ) {
            return scope;
        }
        match current.parent {
            Some(parent) => scope = parent,
            None => return scope,
        }
    }
}

/// ESLint's `isStorableFunction` from the function around `node`, within `rhs`: a function the
/// right side stores (assigns, passes, yields, tags) can run later, so its reads are real.
fn inside_storable_function<D: Doc>(node: &Node<'_, D>, rhs: (usize, usize)) -> bool {
    let inside = |node: &Node<'_, D>| rhs.0 <= node.range().start && node.range().end <= rhs.1;
    let Some(function) = node
        .ancestors()
        .find(|ancestor| FUNCTIONS.contains(&ancestor.kind().as_ref()))
    else {
        return false;
    };
    if !inside(&function) {
        return false;
    }
    let mut child = function.clone();
    let mut parent = function.parent();
    while let Some(current) = parent.filter(|current| inside(current)) {
        let kind = current.kind();
        match kind.as_ref() {
            "parenthesized_expression" | "arguments" | "template_substitution" => {}
            "sequence_expression"
                if !named_children(&current)
                    .last()
                    .is_some_and(|last| same(&last, &child)) =>
            {
                return false;
            }
            "call_expression" => {
                if current
                    .field("arguments")
                    .is_some_and(|arguments| arguments.kind() == "template_string")
                {
                    return true;
                }
                return !current
                    .field("function")
                    .is_some_and(|callee| same(&callee, &child));
            }
            "new_expression" => {
                return !current
                    .field("constructor")
                    .is_some_and(|callee| same(&callee, &child));
            }
            "assignment_expression" | "augmented_assignment_expression" | "yield_expression" => {
                return true;
            }
            kind if kind.ends_with("_statement")
                || kind.ends_with("_declaration")
                || kind == "statement_block" =>
            {
                return true;
            }
            _ => {}
        }
        child = current.clone();
        parent = current.parent();
    }
    false
}

impl<D: Doc> Variable<'_, D> {
    fn self_reference(&self, table: &ScopeTable, reference: &Ref<'_, D>) -> bool {
        if self
            .self_inside
            .iter()
            .any(|(start, end)| *start <= reference.start && reference.end <= *end)
        {
            return true;
        }
        if self.self_scopes.is_empty() {
            return false;
        }
        let mut current = Some(reference.from);
        while let Some(index) = current {
            let scope = &table.scopes[index as usize];
            if self
                .self_scopes
                .iter()
                .any(|(start, end)| scope.range.start == *start && scope.range.end == *end)
            {
                return true;
            }
            current = scope.parent;
        }
        false
    }

    /// ESLint's `isUsedVariable` over the references in order, and whether a counted read comes
    /// from another scope than `scope`.
    fn reads(&self, table: &ScopeTable, scope: u32) -> (u32, u32) {
        let variable_scope = Some(function_scope(table, scope));
        let mut rhs: Option<(usize, usize)> = None;
        let (mut reads, mut nested) = (0, 0);
        for reference in &self.refs {
            if reference.for_in_of {
                reads += 1;
                continue;
            }
            let inside =
                rhs.is_some_and(|(start, end)| start <= reference.start && reference.end <= end);
            let storable = inside
                && reference
                    .node
                    .as_ref()
                    .zip(rhs)
                    .is_some_and(|(node, rhs)| inside_storable_function(node, rhs));
            let for_itself = reference.read && (reference.self_update || (inside && !storable));
            rhs = if inside {
                rhs
            } else {
                reference
                    .assign_rhs
                    .filter(|_| reference.variable_scope == variable_scope && !reference.in_loop)
            };
            let type_only = reference.type_use && !self.imports;
            if reference.read && !for_itself && !type_only && !self.self_reference(table, reference)
            {
                reads += 1;
                nested += u32::from(reference.from != scope);
            }
        }
        (reads, nested)
    }
}

/// Where ESLint reports a variable: its last write from its own function scope (eslint-scope's
/// `variableScope`), else its first definition's name.
fn anchor<D: Doc>(
    table: &ScopeTable,
    scope: u32,
    bindings: &[Binding],
    variable: &Variable<'_, D>,
) -> Option<(u32, u32)> {
    let own = Some(function_scope(table, scope));
    variable
        .refs
        .iter()
        .rev()
        .find(|reference| reference.write && reference.variable_scope == own)
        .map(|reference| (reference.start as u32, reference.end as u32))
        .or_else(|| {
            bindings
                .iter()
                .find_map(|binding| binding.name)
                .map(|name| (name.start, name.end))
        })
}

/// Builds the TypeScript facts of `table`, whose file's root is `root`.
pub(super) fn build<'r, D: Doc>(
    table: &ScopeTable,
    root: &Node<'r, D>,
    steps: &mut Steps<'_>,
) -> Result<Built, BuildError> {
    let text = root.text();
    let mut names = Vec::new();
    for (scope_index, scope) in table.scopes.iter().enumerate() {
        for (name, bindings) in &scope.bindings {
            for (index, binding) in bindings.iter().enumerate() {
                steps.take()?;
                if let Some(node) = binding.name {
                    names.push(NameEntry {
                        start: node.start as usize,
                        end: node.end as usize,
                        kind: node.kind,
                        key: (scope_index as u32, name.as_ref()),
                        index,
                    });
                }
            }
        }
    }
    names.sort_by_key(|entry| (entry.start, entry.end));
    let mut walk = Walk {
        table,
        text: text.as_ref(),
        base: root.range().start,
        next_reference: 0,
        names,
        next_name: 0,
        ancestors: Vec::new(),
        variables: HashMap::new(),
        comment_exports: BTreeSet::new(),
        steps,
        stack_base: stack_address(),
        private_kinds: text.contains('#').then(|| {
            (
                kind_ids(table.language, &["class_body"]),
                kind_ids(table.language, &["private_property_identifier"]),
            )
        }),
        classes: Vec::new(),
        privates: Vec::new(),
    };
    walk.visit(root.clone())?;
    let Walk {
        mut variables,
        comment_exports,
        steps,
        mut privates,
        ..
    } = walk;
    for private in &mut privates {
        private
            .occurrences
            .sort_by_key(|node| (node.start, node.end));
    }
    // Every scope an `eval` or `with` runs in, and those around it: a read there may be unseen.
    let mut dynamic = vec![false; table.scopes.len()];
    for scope in &table.dynamic {
        let mut current = Some(*scope);
        while let Some(index) = current {
            dynamic[index as usize] = true;
            current = table.scopes[index as usize].parent;
        }
    }
    let mut facts = Vec::with_capacity(table.scopes.len());
    let mut occurrences = HashMap::new();
    for (scope_index, scope) in table.scopes.iter().enumerate() {
        let mut per_name = HashMap::with_capacity(scope.bindings.len());
        for (name, bindings) in &scope.bindings {
            steps.take()?;
            let key = (scope_index as u32, name.as_ref());
            let variable = variables.remove(&key).unwrap_or_default();
            let mut variable = variable;
            variable.refs.sort_by_key(|reference| reference.start);
            let (reads, nested) = variable.reads(table, scope_index as u32);
            let unknown = dynamic[scope_index];
            let referenced = !variable.refs.is_empty();
            // `arguments` with no declaration of its own.
            let marked = variable.marked
                || bindings
                    .iter()
                    .all(|binding| binding.site.form == Form::Implicit);
            let fact = Facts {
                reads,
                nested,
                used: known(marked || reads > 0, unknown),
                nested_use: known(nested > 0, unknown),
                exported: Some(variable.exported),
                referenced: known(referenced, unknown),
                anchor: anchor(table, scope_index as u32, bindings, &variable),
                references: variable.refs.len() as u32,
            };
            per_name.insert(name.clone(), vec![fact; bindings.len()]);
            let mut found = std::mem::take(&mut variable.occurrences);
            found.sort_by_key(|node| (node.start, node.end));
            occurrences.insert((scope_index as u32, name.clone()), found);
        }
        facts.push(per_name);
    }
    Ok(Built {
        facts,
        comment_exports,
        occurrences,
        privates,
    })
}
