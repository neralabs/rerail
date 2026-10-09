//! What `compare`'s `key`, `member`, `name` and `reference` modes read from PHP (tree-sitter-php
//! 0.24.2), as nikic/PHP-Parser 5.9.0 parses the code and PHP 8.5 evaluates and looks up what it
//! names (`fixtures/php/value/compare/generate_php.php`).
//!
//! - A key is the name of a property, method, class constant, array element or named argument,
//!   or of the access, declaration or element holding one: an identifier as written, or what PHP
//!   converts a computed name to (`$o->{'a' . 'b'}` is `ab`, `[1.5 => x]` is `1`).
//! - A name is what PHP looks a symbol up by: functions, classes, methods and namespaces
//!   case-insensitively (folded to ASCII lower case, as PHP's symbol tables are), variables,
//!   properties and constants as written; a string literal's value.
//! - A reference is a variable, constant or chain of accesses (`$a->b['k']`, `C::$p`, `C::K`), the
//!   same where PHP reaches the same storage by the same path.

use super::php::{self, Key};
use super::{Context, Value};
use ast_grep_core::{Doc, Node};

/// What a `use` clause imports.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ImportKind {
    Class,
    Function,
    Constant,
}

/// What a `use` clause imports: classes, unless its own keyword or its statement's says
/// `function` or `const`. `use function a\f, b\g;` types every clause with the first one's
/// keyword, which the grammar gives that clause; a group statement carries its keyword itself.
pub(crate) fn import_kind<D: Doc>(clause: &Node<'_, D>) -> ImportKind {
    let keyword = clause.field("type").or_else(|| {
        let parent = clause.parent()?;
        if parent.kind() == "namespace_use_group" {
            parent.parent()?.field("type")
        } else {
            parent
                .children()
                .find(|child| child.kind() == "namespace_use_clause")?
                .field("type")
        }
    });
    match keyword
        .map(|keyword| keyword.text().to_ascii_lowercase())
        .as_deref()
    {
        Some("function") => ImportKind::Function,
        Some("const") => ImportKind::Constant,
        _ => ImportKind::Class,
    }
}

/// What a key is converted with, where an expression computes it.
#[derive(Clone, Copy, PartialEq)]
enum Role {
    /// `$o->{e}`, `C::${e}`, and a key node by itself outside a key position: PHP's string
    /// conversion of a scalar (an array converts only with a warning, which is no name here).
    String,
    /// `$o->{e}()`, `C::{e}()`, `C::{e}`: a string only.
    StringOnly,
    /// `[e => v]`, `$a[e]`: PHP's array key coercion.
    ArrayKey,
}

/// The key a node holds.
enum Keyed<'r, D: Doc> {
    /// An identifier, as written.
    Written(Vec<u8>),
    /// An expression whose value, converted, is the key.
    Computed(Node<'r, D>, Role),
    /// No key: a positional argument or element, `$a[]`, any other node.
    Nameless,
}

/// A key's name: `Some(name)`, `None` where the key is not known (its value is not, or PHP
/// refuses it), and `Some(None)` where the node holds no key.
pub(crate) type KeyName = Option<Option<Vec<u16>>>;

fn named_children<'r, D: Doc>(node: &Node<'r, D>) -> impl Iterator<Item = Node<'r, D>> {
    node.children()
        .filter(|child| child.is_named() && child.kind() != "comment")
}

fn has_token<D: Doc>(node: &Node<'_, D>, token: &str) -> bool {
    node.children()
        .any(|child| !child.is_named() && child.kind() == token)
}

/// The name of an access whose `name` field may be computed: `->foo`, `->$x`, `->{e}`, `::{e}`.
fn member_key<'r, D: Doc>(
    holder: &Node<'r, D>,
    name: Option<Node<'r, D>>,
    role: Role,
) -> Keyed<'r, D> {
    let Some(name) = name else {
        return Keyed::Nameless;
    };
    // `C::{e}` reads as a `name` holding the expression.
    let inner = named_children(&name).next();
    match inner {
        Some(inner) if name.kind() == "name" => Keyed::Computed(inner, role),
        // `->{FOO}` is the constant's value, not the name `FOO`.
        _ if name.kind() == "name" && !has_token(holder, "{") => {
            Keyed::Written(name.text().as_bytes().to_vec())
        }
        _ => Keyed::Computed(name, role),
    }
}

/// A static property's name: `$p` in `C::$p`, or what `$$x` and `${e}` compute.
fn static_property_key<'r, D: Doc>(name: Option<Node<'r, D>>) -> Keyed<'r, D> {
    let Some(name) = name else {
        return Keyed::Nameless;
    };
    match name.kind().as_ref() {
        "variable_name" => variable(&name).map_or(Keyed::Nameless, Keyed::Written),
        "dynamic_variable_name" => match named_children(&name).next() {
            Some(inner) => Keyed::Computed(inner, Role::String),
            None => Keyed::Nameless,
        },
        _ => Keyed::Nameless,
    }
}

/// A variable's name without its `$`.
fn variable<D: Doc>(node: &Node<'_, D>) -> Option<Vec<u8>> {
    let name = named_children(node).find(|child| child.kind() == "name")?;
    Some(name.text().as_bytes().to_vec())
}

/// The key a node holds, or the key a key node is.
fn keyed<'r, D: Doc>(node: &Node<'r, D>) -> Keyed<'r, D> {
    match node.kind().as_ref() {
        "member_access_expression" | "nullsafe_member_access_expression" => {
            member_key(node, node.field("name"), Role::String)
        }
        "member_call_expression" | "nullsafe_member_call_expression" | "scoped_call_expression" => {
            member_key(node, node.field("name"), Role::StringOnly)
        }
        "scoped_property_access_expression" => static_property_key(node.field("name")),
        "class_constant_access_expression" => {
            member_key(node, named_children(node).nth(1), Role::StringOnly)
        }
        "array_element_initializer" if has_token(node, "=>") => match named_children(node).next() {
            Some(key) => Keyed::Computed(key, Role::ArrayKey),
            None => Keyed::Nameless,
        },
        "subscript_expression" if has_token(node, "[") || has_token(node, "{") => {
            match named_children(node).nth(1) {
                Some(index) => Keyed::Computed(index, Role::ArrayKey),
                None => Keyed::Nameless,
            }
        }
        "argument" | "method_declaration" | "enum_case" => match node.field("name") {
            Some(name) => Keyed::Written(name.text().as_bytes().to_vec()),
            None => Keyed::Nameless,
        },
        "property_element" | "property_promotion_parameter" => {
            let Some(mut name) = node.field("name") else {
                return Keyed::Nameless;
            };
            if name.kind() == "by_ref" {
                let inner = named_children(&name).next();
                if let Some(inner) = inner {
                    name = inner;
                }
            }
            variable(&name).map_or(Keyed::Nameless, Keyed::Written)
        }
        "const_element" => match named_children(node).find(|child| child.kind() == "name") {
            Some(name) => Keyed::Written(name.text().as_bytes().to_vec()),
            None => Keyed::Nameless,
        },
        _ => key_node(node),
    }
}

/// A key node by itself: the key of what holds it where it is that holder's key, else a name as
/// written or an expression's value.
fn key_node<'r, D: Doc>(node: &Node<'r, D>) -> Keyed<'r, D> {
    if let Some(parent) = node.parent() {
        let holds = match parent.kind().as_ref() {
            "member_access_expression"
            | "nullsafe_member_access_expression"
            | "member_call_expression"
            | "nullsafe_member_call_expression"
            | "scoped_call_expression"
            | "scoped_property_access_expression"
            | "argument"
            | "method_declaration"
            | "enum_case"
            | "property_element"
            | "property_promotion_parameter" => parent
                .field("name")
                .is_some_and(|name| name.range() == node.range()),
            "class_constant_access_expression" => named_children(&parent)
                .nth(1)
                .is_some_and(|name| name.range() == node.range()),
            "array_element_initializer" if has_token(&parent, "=>") => named_children(&parent)
                .next()
                .is_some_and(|key| key.range() == node.range()),
            "subscript_expression" => named_children(&parent)
                .nth(1)
                .is_some_and(|index| index.range() == node.range()),
            "const_element" => node.kind() == "name",
            _ => false,
        };
        if holds {
            return keyed(&parent);
        }
    }
    match node.kind().as_ref() {
        "name" if named_children(node).next().is_none() => {
            Keyed::Written(node.text().as_bytes().to_vec())
        }
        _ => Keyed::Computed(node.clone(), Role::String),
    }
}

/// A computed key's value converted as its role converts it; `None` where PHP refuses it or the
/// value is not known.
fn converted(value: &Value, role: Role) -> Option<Vec<u8>> {
    match role {
        Role::ArrayKey => match php::key_of(value)? {
            Key::Int(index) => Some(index.to_string().into_bytes()),
            Key::Str(text) => Some(text),
        },
        Role::StringOnly => match value {
            Value::PhpStr(text) => Some(text.clone()),
            _ => None,
        },
        Role::String => scalar_string(value),
    }
}

/// A scalar as PHP converts it to a string; `None` for an array or a value not known.
fn scalar_string(value: &Value) -> Option<Vec<u8>> {
    match value {
        Value::PhpArray(_) => None,
        value => php::to_bytes(value),
    }
}

fn utf16(bytes: &[u8]) -> Option<Vec<u16>> {
    Some(std::str::from_utf8(bytes).ok()?.encode_utf16().collect())
}

/// The key a node holds or is, as UTF-16 (`compare`'s `key` and `member`): `value` reads a
/// computed key's expression. `Some(None)` where the node holds no key; `None` where the key's
/// value is not known, PHP refuses it, or it is not UTF-8.
pub(crate) fn key_name<'r, D: Doc>(
    node: &Node<'r, D>,
    value: &mut dyn FnMut(&Node<'r, D>) -> Value,
) -> KeyName {
    let bytes = match keyed(node) {
        Keyed::Nameless => return Some(None),
        Keyed::Written(bytes) => bytes,
        Keyed::Computed(expression, role) => converted(&value(&expression), role)?,
    };
    utf16(&bytes).map(Some)
}

/// How PHP compares a name it looks up.
#[derive(Clone, Copy, PartialEq)]
enum Case {
    /// Functions, classes, interfaces, traits, enums, methods and namespaces.
    Insensitive,
    /// Constants: their namespace case-insensitively, their own name as written.
    Constant,
    /// Variables, properties, class constants, enum cases, parameters, labels: as written.
    Sensitive,
}

/// How PHP looks up the name a `name`, `qualified_name` or `relative_name` node writes, by where
/// it stands.
fn case_of<D: Doc>(node: &Node<'_, D>) -> Case {
    let Some(parent) = node.parent() else {
        return Case::Sensitive;
    };
    let is_field = |field: &str| {
        parent
            .field(field)
            .is_some_and(|child| child.range() == node.range())
    };
    match parent.kind().as_ref() {
        // A segment of a qualified name is looked up as the qualified name is.
        "qualified_name" | "relative_name" => match case_of(&parent) {
            Case::Insensitive => Case::Insensitive,
            _ => Case::Sensitive,
        },
        "namespace_name" | "namespace_definition" => Case::Insensitive,
        "function_definition"
        | "class_declaration"
        | "interface_declaration"
        | "trait_declaration"
        | "enum_declaration"
        | "method_declaration" => {
            if is_field("name") {
                Case::Insensitive
            } else {
                Case::Sensitive
            }
        }
        "function_call_expression" => {
            if is_field("function") {
                Case::Insensitive
            } else {
                Case::Sensitive
            }
        }
        "member_call_expression" | "nullsafe_member_call_expression" => {
            if is_field("name") {
                Case::Insensitive
            } else {
                Case::Sensitive
            }
        }
        "scoped_call_expression" => {
            if is_field("name") || is_field("scope") {
                Case::Insensitive
            } else {
                Case::Sensitive
            }
        }
        "scoped_property_access_expression" => {
            if is_field("scope") {
                Case::Insensitive
            } else {
                Case::Sensitive
            }
        }
        "class_constant_access_expression" => {
            if named_children(&parent)
                .next()
                .is_some_and(|scope| scope.range() == node.range())
            {
                Case::Insensitive
            } else {
                Case::Sensitive
            }
        }
        "named_type"
        | "object_creation_expression"
        | "base_clause"
        | "class_interface_clause"
        | "use_declaration"
        | "type_list"
        | "attribute"
        | "use_as_clause"
        | "use_instead_of_clause" => Case::Insensitive,
        "binary_expression" => {
            if is_field("right")
                && parent
                    .field("operator")
                    .is_some_and(|operator| operator.kind() == "instanceof")
            {
                Case::Insensitive
            } else {
                Case::Constant
            }
        }
        "namespace_use_clause" => {
            let constant = import_kind(&parent) == ImportKind::Constant;
            match (is_field("alias"), constant) {
                // An alias names what it imports: a constant's alias as written.
                (true, true) => Case::Sensitive,
                (false, true) => Case::Constant,
                (_, false) => Case::Insensitive,
            }
        }
        // Anywhere else a name stands as an expression: a constant.
        _ => Case::Constant,
    }
}

/// A written name as PHP looks it up: folded to ASCII lower case where PHP compares it so, a
/// constant's namespace folded and its own name as written.
fn folded(text: &str, case: Case) -> Vec<u8> {
    match case {
        Case::Insensitive => text.to_ascii_lowercase().into_bytes(),
        Case::Sensitive => text.as_bytes().to_vec(),
        Case::Constant => match text.rfind('\\') {
            Some(at) => {
                let mut out = text[..at].to_ascii_lowercase().into_bytes();
                out.extend_from_slice(&text.as_bytes()[at..]);
                out
            }
            None => text.as_bytes().to_vec(),
        },
    }
}

/// The name a node writes, as PHP looks it up (`compare`'s `name`), as UTF-16: a `name`,
/// qualified or relative name or namespace name folded as its position looks it up, a variable's
/// name without `$` (`$this` is `this`), `self`, `static` and `parent` in lower case, and a
/// string literal's value. `None` for any other node, or a name that is not UTF-8.
pub(crate) fn node_name<D: Doc>(node: &Node<'_, D>) -> Option<Vec<u16>> {
    let bytes = match node.kind().as_ref() {
        "name" if named_children(node).next().is_none() => {
            // A variable's own name is as written.
            let case = match node.parent() {
                Some(parent) if parent.kind() == "variable_name" => Case::Sensitive,
                _ => case_of(node),
            };
            folded(&node.text(), case)
        }
        "qualified_name" | "relative_name" => folded(&node.text(), case_of(node)),
        "namespace_name" => node.text().to_ascii_lowercase().into_bytes(),
        "relative_scope" => node.text().to_ascii_lowercase().into_bytes(),
        "variable_name" => variable(node)?,
        "string" | "encapsed_string" | "heredoc" | "nowdoc" => {
            let mut spend = || true;
            let mut context = Context::new(&mut spend);
            match php::decode(node, &mut context) {
                Value::PhpStr(text) => text,
                _ => return None,
            }
        }
        _ => return None,
    };
    utf16(&bytes)
}

/// The expression inside any parentheses.
fn unparenthesized<'r, D: Doc>(node: &Node<'r, D>) -> Node<'r, D> {
    let mut node = node.clone();
    while node.kind() == "parenthesized_expression" {
        let children: Vec<_> = named_children(&node).collect();
        match children.as_slice() {
            [inner] => node = inner.clone(),
            _ => break,
        }
    }
    node
}

/// What a reference is.
#[derive(PartialEq)]
enum Shape {
    Variable,
    DynamicVariable,
    Constant,
    Literal,
    Property,
    StaticProperty,
    ClassConstant,
    Element,
    Other,
}

fn shape<D: Doc>(node: &Node<'_, D>) -> Shape {
    match node.kind().as_ref() {
        "variable_name" => Shape::Variable,
        "dynamic_variable_name" => Shape::DynamicVariable,
        "name" if named_children(node).next().is_none() => Shape::Constant,
        "qualified_name" | "relative_name" => Shape::Constant,
        "string" | "nowdoc" | "integer" | "float" | "boolean" | "null" => Shape::Literal,
        "encapsed_string" | "heredoc" if literal_string(node) => Shape::Literal,
        "member_access_expression" | "nullsafe_member_access_expression" => Shape::Property,
        "scoped_property_access_expression" => Shape::StaticProperty,
        "class_constant_access_expression" => Shape::ClassConstant,
        "subscript_expression" => Shape::Element,
        _ => Shape::Other,
    }
}

/// Whether a double-quoted string or heredoc holds no interpolation.
fn literal_string<D: Doc>(node: &Node<'_, D>) -> bool {
    let body = match node.field("value") {
        Some(body) => body,
        None => node.clone(),
    };
    named_children(&body).all(|child| {
        matches!(
            child.kind().as_ref(),
            "string_content" | "escape_sequence" | "heredoc_start" | "heredoc_end"
        )
    })
}

/// A literal's value; `None` where it does not decode.
fn literal<D: Doc>(node: &Node<'_, D>) -> Option<Value> {
    let mut spend = || true;
    let mut context = Context::new(&mut spend);
    match php::decode(node, &mut context) {
        Value::Unknown => None,
        value => Some(value),
    }
}

/// A key written or computed by a literal, converted: what `static_keys` compares.
fn static_key<D: Doc>(keyed: &Keyed<'_, D>) -> Option<Vec<u8>> {
    match keyed {
        Keyed::Written(bytes) => Some(bytes.clone()),
        Keyed::Computed(expression, role) => {
            let expression = unparenthesized(expression);
            if shape(&expression) != Shape::Literal {
                return None;
            }
            converted(&literal(&expression)?, *role)
        }
        Keyed::Nameless => None,
    }
}

/// Two keys of the same kind of access the same: with `static_keys`, where either is written or
/// a literal, by the key PHP converts it to; else written keys by their text, and computed ones
/// as references (literals by `===`). A missing key is never the same.
fn same_key<D: Doc>(left: &Keyed<'_, D>, right: &Keyed<'_, D>, static_keys: bool) -> Option<bool> {
    if static_keys && let Some(name) = static_key(left) {
        return Some(static_key(right).is_some_and(|other| other == name));
    }
    match (left, right) {
        (Keyed::Written(left), Keyed::Written(right)) => Some(left == right),
        (Keyed::Computed(left, _), Keyed::Computed(right, _)) => {
            same_reference(left, right, static_keys)
        }
        _ => Some(false),
    }
}

/// Two class scopes (`C::`, `\A\C::`, `self::`, `$o::`) are the same: class names as PHP looks
/// them up, as written otherwise (`self` is not `static`), expressions as references.
fn same_scope<D: Doc>(left: &Node<'_, D>, right: &Node<'_, D>, static_keys: bool) -> Option<bool> {
    let class = |node: &Node<'_, D>| -> Option<Vec<u8>> {
        match node.kind().as_ref() {
            "name" | "qualified_name" | "relative_name" | "relative_scope" => {
                Some(node.text().to_ascii_lowercase().into_bytes())
            }
            _ => None,
        }
    };
    match (class(left), class(right)) {
        (Some(left), Some(right)) => Some(left == right),
        (None, None) => same_reference(left, right, static_keys),
        _ => Some(false),
    }
}

/// A static access's class: its `scope` field, or the first child of a class constant access.
fn scope<'r, D: Doc>(node: &Node<'r, D>) -> Option<Node<'r, D>> {
    match node.field("scope") {
        Some(scope) => Some(scope),
        None => named_children(node).next(),
    }
}

/// Whether two expressions are the same reference (`compare`'s `reference`): parentheses aside,
/// and `?->` the same as `->`, the same variable by name; the same constant by name as PHP looks
/// it up; literals by `===`; and accesses of the same object, class or array by the same key.
/// With `static_keys`, a written name and a literal key are the same where PHP converts them to
/// the same name (`$o->a` and `$o->{'a'}`, `$a[1]` and `$a['1']`); without, a written name is
/// not a computed one, and literal keys compare by `===`. A variable variable is the same as
/// another of the same reference; anything else (a call, `$a[]`, an interpolated string) is
/// never. `None` where a literal does not decode.
pub(crate) fn same_reference<D: Doc>(
    left: &Node<'_, D>,
    right: &Node<'_, D>,
    static_keys: bool,
) -> Option<bool> {
    let (left, right) = (unparenthesized(left), unparenthesized(right));
    let kind = shape(&left);
    if kind != shape(&right) {
        return Some(false);
    }
    match kind {
        Shape::Variable => Some(variable(&left)? == variable(&right)?),
        Shape::DynamicVariable => {
            let (Some(left), Some(right)) =
                (named_children(&left).next(), named_children(&right).next())
            else {
                return Some(false);
            };
            if left.kind() == "name" || right.kind() == "name" {
                return Some(left.kind() == right.kind() && left.text() == right.text());
            }
            same_reference(&left, &right, static_keys)
        }
        Shape::Constant => Some(node_name(&left)? == node_name(&right)?),
        Shape::Literal => php::identical(&literal(&left)?, &literal(&right)?),
        Shape::Property => {
            let (Some(left_object), Some(right_object)) =
                (left.field("object"), right.field("object"))
            else {
                return Some(false);
            };
            Some(
                same_reference(&left_object, &right_object, static_keys)?
                    && same_key(&keyed(&left), &keyed(&right), static_keys)?,
            )
        }
        Shape::StaticProperty | Shape::ClassConstant => {
            let (Some(left_scope), Some(right_scope)) = (scope(&left), scope(&right)) else {
                return Some(false);
            };
            Some(
                same_scope(&left_scope, &right_scope, static_keys)?
                    && same_key(&keyed(&left), &keyed(&right), static_keys)?,
            )
        }
        Shape::Element => {
            let (Some(left_object), Some(right_object)) =
                (named_children(&left).next(), named_children(&right).next())
            else {
                return Some(false);
            };
            let (left_key, right_key) = (keyed(&left), keyed(&right));
            if matches!(left_key, Keyed::Nameless) || matches!(right_key, Keyed::Nameless) {
                return Some(false);
            }
            Some(
                same_reference(&left_object, &right_object, static_keys)?
                    && same_key(&left_key, &right_key, static_keys)?,
            )
        }
        Shape::Other => Some(false),
    }
}

#[cfg(test)]
mod tests;
