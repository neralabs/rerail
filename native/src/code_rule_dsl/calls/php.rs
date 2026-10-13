//! One PHP file's call summary (tree-sitter-php 0.24.2): its functions, methods, closures and
//! class-likes, what each calls, and the class hierarchy its class-likes declare. As Python's
//! summary, rule-independent and file-local: callees are names, which the graph links.
//!
//! Names are PHP's fully qualified names with a leading backslash: `\App\f` for a function,
//! `\App\C` for a class, interface, trait or enum, `\App\C.m` for a method. Closures, arrow
//! functions and anonymous classes are named per file (`\{closure}#2@<file>`,
//! `\{class}#1@<file>`), since only their own file reaches them. A definition keeps its name as
//! written; callee names are lowercased, as PHP compares function, class and method names.
//!
//! A callee is named where PHP's dispatch is statically decided: a function by name resolution
//! (an unqualified call in a namespace that neither the file declares nor PHP defines is the
//! namespaced function where the workspace declares one, else the global one, which the graph
//! decides); `self::m()`, `parent::m()` and `C::m()` by the class's own method, a trait's, its
//! parent's or an interface's, as PHP looks it up; `new C` by `C`'s constructor; a closure,
//! arrow function, first-class callable, string or array callable, or `new C` (`__invoke`) a
//! variable holds on every path that reaches the call; `call_user_func` and
//! `call_user_func_array` by their constant callback. `$this->m()`, `static::m()` and
//! `new static` are late-bound (`\App\C.m^`): the class's method and every override a class
//! the workspace derives from it declares, unless the class cannot be extended (final, an enum,
//! an anonymous class) or no override can replace the method for a call in the class (final, or
//! private as the class has it, trait adaptations included). A method called on any other
//! receiver names nothing and gives no edge (its class needs type inference), as Python's
//! `obj.m()`; inside a trait, `$this`, `self`, `static` and `parent` are the using class's and
//! unknown, as inside a closure whose value may leave where PHP calls it (passed, returned,
//! stored, or held by a variable read other than to call it), which may bind it elsewhere.
//! Other dynamic callees (`$f()` of a parameter, `$o->$m()`, `new $class`) are unknown. `f(...)`
//! makes a callable and calls nothing.

use super::names::Name;
use super::summary::{CLASS, Def, OPEN, Summary};
use crate::code_rule_dsl::language::Family;
use crate::code_rule_dsl::scope::php_names::{PhpNames, Resolved, builtin_function};
use crate::code_rule_dsl::scope::{Scope, ScopeKind, ScopeTable};
use crate::code_rule_dsl::value::{Context as ValueContext, Value, php as php_value};
use ast_grep_core::{Doc, Node};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// The node kinds a PHP `calls` term holds at.
pub(crate) const CALL_KINDS: [&str; 5] = [
    "function_call_expression",
    "member_call_expression",
    "nullsafe_member_call_expression",
    "scoped_call_expression",
    "object_creation_expression",
];

/// The definitions a seed query runs on: what a call may reach.
pub(crate) const FUNCTION_KINDS: [&str; 4] = [
    "function_definition",
    "method_declaration",
    "anonymous_function",
    "arrow_function",
];

const CLASS_KINDS: [&str; 5] = [
    "class_declaration",
    "interface_declaration",
    "trait_declaration",
    "enum_declaration",
    "anonymous_class",
];

/// A class-like's parent class: `(class, parent)`.
pub(crate) const PARENT: u8 = 0;
/// A trait the class-like uses: `(class, trait)`.
pub(crate) const TRAIT: u8 = 1;
/// An interface the class-like implements or extends: `(class, interface)`.
pub(crate) const INTERFACE: u8 = 2;
/// `T::m as n`: `(class.n, trait.m)`.
pub(crate) const ALIAS: u8 = 3;
/// `m as n`: `(class.n, m)`, the method `m` of whichever used trait has it.
pub(crate) const ALIAS_TRAITS: u8 = 4;
/// `T::m insteadof U`: `(class.m, u)`, the method `m` is not taken from `U`.
pub(crate) const EXCLUDE: u8 = 5;
/// `m as private`, `T::m as private n`: `(class.n, m)` (or `trait.m`), the method the class
/// imports as `n` is private.
pub(crate) const PRIVATE_AS: u8 = 6;
/// `m as public`, `T::m as protected n`: as `PRIVATE_AS`, the imported method is not private,
/// whatever the trait declares.
pub(crate) const VISIBLE_AS: u8 = 7;
/// `m as final` (PHP 8.3): as `PRIVATE_AS`, the imported method is final.
pub(crate) const FINAL_AS: u8 = 8;

/// A private method: a call in the class that has it runs it, whatever an override declares.
pub(crate) const PRIVATE: u8 = 8;
/// A method without a body: abstract, or an interface's.
pub(crate) const ABSTRACT: u8 = 16;
/// A final method, which no derived class declares.
pub(crate) const FINAL: u8 = 32;

/// Builtin functions whose first argument PHP calls.
const CALLBACK_CALLERS: [&str; 2] = ["call_user_func", "call_user_func_array"];

/// Variables a callable is followed through before it is unknown.
const MAX_FOLLOW: u8 = 4;

/// A callee before every definition of the file is named.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Found {
    /// A function or method by lowercased name; a namespace fallback as
    /// `\ns\f|\f` (namespaced first).
    Name(String),
    /// The closure or arrow function whose node starts here.
    Local(u32),
    /// A method, lowercased, of the class-like whose node starts here; late-bound or not.
    Member(u32, String, bool),
    Open,
}

/// A class a call's receiver or scope names.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ClassRef {
    /// By lowercased name, with its leading backslash.
    Named(String),
    /// The class-like whose node starts here.
    Local(u32),
    /// The class-like whose node starts here or one the workspace derives from it: `$this`,
    /// `static`.
    Late(u32),
}

impl ClassRef {
    fn member(self, method: &str) -> Found {
        match self {
            ClassRef::Named(class) => Found::Name(format!("{class}.{method}")),
            ClassRef::Local(start) => Found::Member(start, method.to_owned(), false),
            ClassRef::Late(start) => Found::Member(start, method.to_owned(), true),
        }
    }
}

/// What a receiver or class position decides: a class, nothing (no edge), or unknown.
enum Class {
    Is(ClassRef),
    None,
    Open,
}

/// The class-like whose body holds a node, as the node's `$this`, `self`, `static` and
/// `parent` read it.
struct Enclosing {
    start: u32,
    /// `$this`, `self`, `static` and `parent` are not the class's: a trait's (the using class's),
    /// or inside a closure whose value may reach code that binds it to another object or scope
    /// (`Closure::bind`, `bindTo`, `call`, a framework's macros; see `Reader::escapes`).
    unbound: bool,
    /// The class cannot be extended: `final`, an enum or an anonymous class.
    sealed: bool,
    parent: Option<String>,
}

impl Enclosing {
    /// The class as `$this` and `static` name it: late-bound unless it cannot be extended.
    fn late(&self) -> ClassRef {
        if self.sealed {
            ClassRef::Local(self.start)
        } else {
            ClassRef::Late(self.start)
        }
    }
}

/// What callee resolution reads of one file.
pub(crate) struct Reader<'a, 'r, D: Doc> {
    pub(crate) names: &'a PhpNames,
    pub(crate) table: &'a ScopeTable,
    pub(crate) root: Node<'r, D>,
}

impl<'r, D: Doc> Reader<'_, 'r, D> {
    /// The callee of the call `call`; `None` where it calls nothing statically named (a method
    /// of a receiver whose class is not known, a first-class callable).
    pub(crate) fn callee(&self, call: &Node<'r, D>) -> Option<Found> {
        if first_class(call) {
            return None;
        }
        self.called(call)
    }

    /// The callee of `call`, a call or a first-class callable of one.
    fn called(&self, call: &Node<'r, D>) -> Option<Found> {
        match call.kind().as_ref() {
            "function_call_expression" => {
                let function = call.field("function")?;
                if !is_name(&function) {
                    return self.callable(&function, 0);
                }
                let Some(reference) = self.names.reference(&function.range()) else {
                    return Some(Found::Open);
                };
                Some(match &reference.resolved {
                    Resolved::Name(name) => {
                        let lower = name.to_ascii_lowercase();
                        if CALLBACK_CALLERS.contains(&lower.as_str()) {
                            return self.callback(call);
                        }
                        // PHP's own function: a workspace declaration of its name is a polyfill
                        // PHP 8.5 never runs.
                        if builtin_function(&lower) {
                            return None;
                        }
                        Found::Name(format!("\\{lower}"))
                    }
                    Resolved::Either { namespaced, global } => Found::Name(format!(
                        "\\{}|\\{}",
                        namespaced.to_ascii_lowercase(),
                        global.to_ascii_lowercase()
                    )),
                    Resolved::Special | Resolved::Unknown => Found::Open,
                })
            }
            "member_call_expression" | "nullsafe_member_call_expression" => {
                let Some(method) = method_name(call) else {
                    return Some(Found::Open);
                };
                let object = unparenthesized(&call.field("object")?);
                match self.receiver(&object, 0) {
                    Class::Is(class) => Some(class.member(&method)),
                    Class::None => None,
                    Class::Open => Some(Found::Open),
                }
            }
            "scoped_call_expression" => {
                let Some(method) = method_name(call) else {
                    return Some(Found::Open);
                };
                let scope = unparenthesized(&call.field("scope")?);
                match self.scope_class(&scope, call) {
                    Class::Is(class) => Some(class.member(&method)),
                    Class::None => None,
                    Class::Open => Some(Found::Open),
                }
            }
            "object_creation_expression" => match self.created(call) {
                Class::Is(class) => Some(class.member("__construct")),
                Class::None => None,
                Class::Open => Some(Found::Open),
            },
            _ => None,
        }
    }

    /// `call_user_func(callback, ...)`, `call_user_func_array(callback, args)`: the callback.
    fn callback(&self, call: &Node<'r, D>) -> Option<Found> {
        let arguments = call.field("arguments")?;
        let mut arguments = arguments
            .children()
            .filter(|child| child.kind() == "argument");
        let first = arguments.next()?;
        let argument = match first.field("name") {
            None => first,
            Some(_) => std::iter::once(first).chain(arguments).find(|argument| {
                argument
                    .field("name")
                    .is_some_and(|name| name.text() == "callback")
            })?,
        };
        let expression = argument
            .children()
            .filter(|child| child.is_named() && !child.kind().ends_with("comment"))
            .last()?;
        if expression.kind() == "variadic_unpacking"
            || argument
                .field("name")
                .is_some_and(|name| name.range() == expression.range())
        {
            return Some(Found::Open);
        }
        self.callable(&expression, 0)
    }

    /// What calling the value of `expression` calls.
    fn callable(&self, expression: &Node<'r, D>, depth: u8) -> Option<Found> {
        let expression = unparenthesized(expression);
        match expression.kind().as_ref() {
            "anonymous_function" | "arrow_function" => {
                Some(Found::Local(expression.range().start as u32))
            }
            "function_call_expression"
            | "member_call_expression"
            | "nullsafe_member_call_expression"
            | "scoped_call_expression"
                if first_class(&expression) =>
            {
                self.called(&expression)
            }
            "object_creation_expression" => match self.created(&expression) {
                Class::Is(class) => Some(class.member("__invoke")),
                Class::None => None,
                Class::Open => Some(Found::Open),
            },
            "string" | "encapsed_string" => match string(&expression) {
                Some(text) => named_callable(&text),
                None => Some(Found::Open),
            },
            "array_creation_expression" => self.array_callable(&expression, depth),
            "variable_name" if expression.text() == "$this" => match self.enclosing(&expression) {
                Some(class) if !class.unbound => Some(class.late().member("__invoke")),
                _ => Some(Found::Open),
            },
            "variable_name" if depth < MAX_FOLLOW => {
                let Some(values) = self.values(&expression) else {
                    return Some(Found::Open);
                };
                let mut found = values.iter().map(|value| self.callable(value, depth + 1));
                let first = found.next()?;
                if found.all(|other| other == first) {
                    first
                } else {
                    Some(Found::Open)
                }
            }
            _ => Some(Found::Open),
        }
    }

    /// `[object or class, 'method']`: `None` for an object whose class is not known, as a method
    /// call on it.
    fn array_callable(&self, array: &Node<'r, D>, depth: u8) -> Option<Found> {
        let elements = array
            .children()
            .filter(|child| child.kind() == "array_element_initializer")
            .collect::<Vec<_>>();
        let [target, method] = elements.as_slice() else {
            return Some(Found::Open);
        };
        let value = |element: &Node<'r, D>| {
            let mut named = element
                .children()
                .filter(|child| child.is_named() && !child.kind().ends_with("comment"));
            match (named.next(), named.next()) {
                (Some(value), None) => Some(unparenthesized(&value)),
                _ => None,
            }
        };
        let (Some(target), Some(method)) = (value(target), value(method)) else {
            return Some(Found::Open);
        };
        let Some(method) = string(&method).filter(|method| is_label(method)) else {
            return Some(Found::Open);
        };
        let method = method.to_ascii_lowercase();
        let class = match target.kind().as_ref() {
            "string" | "encapsed_string" => match string(&target) {
                Some(class) => match class_name(&class) {
                    Some(class) => Class::Is(ClassRef::Named(class)),
                    None => Class::Open,
                },
                None => Class::Open,
            },
            "class_constant_access_expression" => self.class_constant(&target),
            _ => self.receiver(&target, depth),
        };
        match class {
            Class::Is(class) => Some(class.member(&method)),
            Class::None => None,
            Class::Open => Some(Found::Open),
        }
    }

    /// `X::class`: the class `X` names.
    fn class_constant(&self, access: &Node<'r, D>) -> Class {
        let mut named = access
            .children()
            .filter(|child| child.is_named() && !child.kind().ends_with("comment"));
        let (Some(scope), Some(constant), None) = (named.next(), named.next(), named.next()) else {
            return Class::Open;
        };
        if !constant.text().eq_ignore_ascii_case("class") {
            return Class::Open;
        }
        self.scope_class(&scope, access)
    }

    /// The class of the object `object` evaluates to, where it is `$this`, a `new` expression,
    /// or a variable that holds one on every path.
    fn receiver(&self, object: &Node<'r, D>, depth: u8) -> Class {
        match object.kind().as_ref() {
            "variable_name" if object.text() == "$this" => match self.enclosing(object) {
                Some(class) if !class.unbound => Class::Is(class.late()),
                _ => Class::Open,
            },
            "variable_name" if depth < MAX_FOLLOW => {
                let Some(values) = self.values(object) else {
                    return Class::None;
                };
                let mut classes = values.iter().map(|value| {
                    let value = unparenthesized(value);
                    if value.kind() == "object_creation_expression" {
                        self.created(&value)
                    } else {
                        Class::None
                    }
                });
                let first = match classes.next() {
                    Some(Class::Is(class)) => class,
                    Some(other) => return other,
                    None => return Class::None,
                };
                for other in classes {
                    match other {
                        Class::Is(class) if class == first => {}
                        Class::Open => return Class::Open,
                        _ => return Class::None,
                    }
                }
                Class::Is(first)
            }
            "object_creation_expression" => self.created(object),
            _ => Class::None,
        }
    }

    /// The class a `new` expression instantiates.
    fn created(&self, creation: &Node<'r, D>) -> Class {
        let Some(class) = creation
            .children()
            .find(|child| child.is_named() && !child.kind().ends_with("comment"))
        else {
            return Class::Open;
        };
        if class.kind() == "anonymous_class" {
            return Class::Is(ClassRef::Local(class.range().start as u32));
        }
        if is_name(&class) {
            return self.scope_class(&class, creation);
        }
        Class::Open
    }

    /// The class a class position names: a name, `self`, `static` or `parent`.
    fn scope_class(&self, scope: &Node<'r, D>, at: &Node<'r, D>) -> Class {
        let special = match scope.kind().as_ref() {
            "relative_scope" => scope.text().to_ascii_lowercase(),
            kind if is_name_kind(kind) => match self.names.reference(&scope.range()) {
                Some(reference) => match &reference.resolved {
                    Resolved::Name(name) => {
                        return Class::Is(ClassRef::Named(format!(
                            "\\{}",
                            name.to_ascii_lowercase()
                        )));
                    }
                    Resolved::Special => scope.text().to_ascii_lowercase(),
                    Resolved::Either { .. } | Resolved::Unknown => return Class::Open,
                },
                None => return Class::Open,
            },
            "variable_name" => {
                return match self.receiver(scope, 0) {
                    Class::Is(class) => Class::Is(class),
                    _ => Class::Open,
                };
            }
            _ => return Class::Open,
        };
        let Some(class) = self.enclosing(at) else {
            return Class::Open;
        };
        if class.unbound {
            return Class::Open;
        }
        match special.as_str() {
            "self" => Class::Is(ClassRef::Local(class.start)),
            "parent" => match class.parent {
                Some(parent) => Class::Is(ClassRef::Named(parent)),
                // A class without a parent: PHP refuses `parent` there.
                None => Class::None,
            },
            "static" => Class::Is(class.late()),
            _ => Class::Open,
        }
    }

    /// The values the variable read at `variable` holds on every path that reaches it: each
    /// binding's assigned value. `None` where a path reaches it unassigned or with a value no
    /// assignment gives (a parameter, a reference, `global`, `foreach`, a dynamic write).
    fn values(&self, variable: &Node<'r, D>) -> Option<Vec<Node<'r, D>>> {
        let flow = self.table.php.as_ref()?;
        let read = flow.read(variable.range().start)?;
        if read.defined != crate::code_rule_dsl::scope::php::Defined::Yes
            || read.aliased
            || read.superglobal
            || read.bindings.is_empty()
        {
            return None;
        }
        read.bindings
            .iter()
            .map(|(scope, index)| {
                let binding = self
                    .table
                    .scopes
                    .get(*scope as usize)?
                    .bindings
                    .get(read.name.as_ref())?
                    .get(*index)?;
                binding.site.value?.find_near(self.root.clone(), variable)
            })
            .collect()
    }

    /// The class-like whose body holds `node` and gives it `$this`, `self` and `static`: none
    /// inside a named function.
    fn enclosing(&self, node: &Node<'r, D>) -> Option<Enclosing> {
        let at = node.range().start;
        let mut closures = Vec::new();
        for ancestor in node.ancestors() {
            let kind = ancestor.kind();
            if kind == "function_definition" {
                return None;
            }
            if matches!(kind.as_ref(), "anonymous_function" | "arrow_function") {
                closures.push(ancestor.clone());
            }
            if !CLASS_KINDS.contains(&kind.as_ref())
                || !ancestor
                    .field("body")
                    .is_some_and(|body| body.range().contains(&at))
            {
                continue;
            }
            let sealed = matches!(kind.as_ref(), "enum_declaration" | "anonymous_class")
                || ancestor
                    .children()
                    .any(|child| child.kind() == "final_modifier");
            let parent = matches!(kind.as_ref(), "class_declaration" | "anonymous_class")
                .then(|| {
                    let clause = ancestor
                        .children()
                        .find(|child| child.kind() == "base_clause")?;
                    let name = clause.children().find(|child| is_name(child))?;
                    self.class_name_at(&name)
                })
                .flatten();
            return Some(Enclosing {
                start: ancestor.range().start as u32,
                unbound: kind == "trait_declaration"
                    || closures.iter().any(|closure| self.escapes(closure)),
                sealed,
                parent,
            });
        }
        None
    }

    /// Whether the value of the closure or arrow function `closure` may reach code that binds
    /// it elsewhere: unless PHP calls it where it is made (`(function () {})()`), or it is
    /// assigned, as a statement, to a variable of a function body that every read calls
    /// (`$f = function () {}; $f();`). Any other read (an argument, a return, a `use`, a store,
    /// `->bindTo()`), a reference, or a read of every variable (`compact`, `get_defined_vars`,
    /// `include`) lets it out.
    fn escapes(&self, closure: &Node<'r, D>) -> bool {
        let mut value = closure.clone();
        let mut parent = closure.parent();
        while let Some(outer) = parent
            .as_ref()
            .filter(|outer| outer.kind() == "parenthesized_expression")
        {
            value = outer.clone();
            parent = outer.parent();
        }
        let Some(parent) = parent else {
            return true;
        };
        match parent.kind().as_ref() {
            "function_call_expression" => !parent
                .field("function")
                .is_some_and(|function| function.range() == value.range()),
            "assignment_expression" => !self.only_called(&parent, &value),
            _ => true,
        }
    }

    /// Whether the statement `assignment` assigns `value` to a variable of a function body whose
    /// every read calls it.
    fn only_called(&self, assignment: &Node<'r, D>, value: &Node<'r, D>) -> bool {
        let (Some(left), Some(right)) = (assignment.field("left"), assignment.field("right"))
        else {
            return false;
        };
        if right.range() != value.range()
            || left.kind() != "variable_name"
            || assignment
                .parent()
                .is_none_or(|statement| statement.kind() != "expression_statement")
        {
            return false;
        }
        let Some(flow) = self.table.php.as_ref() else {
            return false;
        };
        let name = left.text();
        let start = left.range().start;
        // The binding the assignment makes: its scope and index among the variable's bindings.
        let made = |(scope, entry): (usize, &Scope)| {
            let bindings = entry.bindings.get(name.as_ref())?;
            let index = bindings.iter().position(|binding| {
                binding
                    .name
                    .is_some_and(|node| node.start as usize == start)
            })?;
            Some((scope as u32, index))
        };
        let Some((scope, index)) = self.table.scopes.iter().enumerate().find_map(made) else {
            return false;
        };
        // A file's variables are global: `global`, `$GLOBALS` and an included file read them.
        if self.table.scopes[scope as usize].kind == ScopeKind::Module {
            return false;
        }
        let key = (scope, Box::<str>::from(name.as_ref()), index);
        let within = |mut inner: u32| loop {
            if inner == scope {
                return true;
            }
            match self.table.scopes[inner as usize].parent {
                Some(outer) => inner = outer,
                None => return false,
            }
        };
        if flow.aliased.contains(&key)
            || flow
                .modifies
                .values()
                .flatten()
                .any(|modified| *modified == key)
            || flow.reads_all.iter().any(|all| within(*all))
            || flow
                .mentions()
                .any(|(at, mentioned, _)| *mentioned == key.1 && within(*at))
        {
            return false;
        }
        flow.reads()
            .filter(|read| read.name == key.1 && read.bindings.contains(&(scope, index)))
            .all(|read| {
                read.node.find(self.root.clone()).is_some_and(|variable| {
                    variable.kind() == "variable_name"
                        && variable.parent().is_some_and(|call| {
                            call.kind() == "function_call_expression"
                                && call
                                    .field("function")
                                    .is_some_and(|function| function.range() == variable.range())
                        })
                })
            })
    }

    /// The lowercased class a class-position name resolves to.
    fn class_name_at(&self, name: &Node<'r, D>) -> Option<String> {
        match &self.names.reference(&name.range())?.resolved {
            Resolved::Name(name) => Some(format!("\\{}", name.to_ascii_lowercase())),
            _ => None,
        }
    }
}

/// Whether `call` makes a first-class callable (`f(...)`) rather than calling.
fn first_class<D: Doc>(call: &Node<'_, D>) -> bool {
    call.field("arguments").is_some_and(|arguments| {
        arguments
            .children()
            .any(|child| child.kind() == "variadic_placeholder")
    })
}

/// A member call's method, lowercased: `None` for a dynamic one (`$o->$m()`, `C::{$m}()`).
fn method_name<D: Doc>(call: &Node<'_, D>) -> Option<String> {
    let name = call.field("name")?;
    (name.kind() == "name").then(|| name.text().to_ascii_lowercase())
}

fn is_name_kind(kind: &str) -> bool {
    matches!(kind, "name" | "qualified_name" | "relative_name")
}

fn is_name<D: Doc>(node: &Node<'_, D>) -> bool {
    is_name_kind(&node.kind())
}

fn unparenthesized<'r, D: Doc>(node: &Node<'r, D>) -> Node<'r, D> {
    let mut node = node.clone();
    while node.kind() == "parenthesized_expression" {
        let mut inner = node
            .children()
            .filter(|child| child.is_named() && !child.kind().ends_with("comment"));
        let (Some(expression), None) = (inner.next(), inner.next()) else {
            break;
        };
        drop(inner);
        node = expression;
    }
    node
}

/// A string literal's value, where it is constant UTF-8.
fn string<D: Doc>(node: &Node<'_, D>) -> Option<String> {
    let mut spend = || true;
    match php_value::decode(node, &mut ValueContext::new(&mut spend)) {
        Value::PhpStr(bytes) => String::from_utf8(bytes).ok(),
        _ => None,
    }
}

/// A PHP label: a letter, `_` or a byte from 0x80, then those or digits.
fn is_label(text: &str) -> bool {
    crate::code_rule_dsl::scope::php_names::is_label(text)
}

/// A class name a string spells (always fully qualified), lowercased with its leading
/// backslash.
fn class_name(text: &str) -> Option<String> {
    let text = text.strip_prefix('\\').unwrap_or(text);
    text.split('\\')
        .all(is_label)
        .then(|| format!("\\{}", text.to_ascii_lowercase()))
}

/// What a string callable names: `'f'`, `'ns\f'` or `'C::m'`; `None` for PHP's own function.
fn named_callable(text: &str) -> Option<Found> {
    Some(match text.split_once("::") {
        Some((class, method)) => match class_name(class) {
            Some(class) if is_label(method) => {
                Found::Name(format!("{class}.{}", method.to_ascii_lowercase()))
            }
            _ => Found::Open,
        },
        None => match class_name(text) {
            Some(function) if builtin_function(&function[1..]) => return None,
            Some(function) => Found::Name(function),
            None => Found::Open,
        },
    })
}

/// The per-file tag of local definitions' names: the file's path, hashed (no dots, which name
/// members).
fn file_tag(path: &str) -> String {
    let digest = Sha256::digest(path.as_bytes());
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The namespace name a `namespace_definition` declares, `""` for the global one.
fn namespace_of<D: Doc>(definition: &Node<'_, D>) -> String {
    definition
        .field("name")
        .map(|name| {
            name.children()
                .filter(|segment| segment.kind() == "name")
                .map(|segment| segment.text().into_owned())
                .collect::<Vec<_>>()
                .join("\\")
        })
        .unwrap_or_default()
}

fn qualify(namespace: &str, name: &str) -> String {
    if namespace.is_empty() {
        format!("\\{name}")
    } else {
        format!("\\{namespace}\\{name}")
    }
}

/// The lexical context a node is visited in.
#[derive(Clone)]
struct Context {
    /// The function-like whose body this is.
    owner: Option<u32>,
    namespace: String,
    /// In a class-like body: its name as written.
    class: Option<String>,
}

struct Extractor {
    names: Vec<Box<str>>,
    ids: HashMap<Box<str>, u32>,
    defs: Vec<Def>,
    edges: Vec<(u32, u32)>,
    targets: Vec<u32>,
    supers: Vec<(u32, u32, u8)>,
    open_sites: bool,
    tag: String,
    closures: u32,
    classes: u32,
}

impl Extractor {
    fn intern(&mut self, name: &str) -> u32 {
        if let Some(id) = self.ids.get(name) {
            return *id;
        }
        let id = self.names.len() as u32;
        self.names.push(name.into());
        self.ids.insert(name.into(), id);
        id
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

    fn define<D: Doc>(&mut self, node: &Node<'_, D>, name: &str, flags: u8) -> u32 {
        let def = self.defs.len() as u32;
        let name = self.intern(name);
        self.defs.push(Def {
            name,
            start: node.range().start as u32,
            end: node.range().end as u32,
            flags,
        });
        def
    }

    /// A definition's name by its node's start.
    fn def_name(&self, start: u32) -> Option<&str> {
        let index = self.defs.partition_point(|def| def.start < start);
        let def = self.defs.get(index).filter(|def| def.start == start)?;
        Some(&self.names[def.name as usize])
    }
}

/// Extracts the summary of the PHP file at `path` whose tree is `root`, through its variable
/// flow (`table`) and name resolution (`names`); without them (they could not be built), a
/// summary of its definitions alone, which is open.
pub(crate) fn extract<D: Doc>(
    root: &Node<'_, D>,
    path: &str,
    table: Option<&ScopeTable>,
    names: Option<&PhpNames>,
    has_errors: bool,
) -> Summary {
    let mut extractor = Extractor {
        names: Vec::new(),
        ids: HashMap::new(),
        defs: Vec::new(),
        edges: Vec::new(),
        targets: Vec::new(),
        supers: Vec::new(),
        open_sites: false,
        tag: file_tag(path),
        closures: 0,
        classes: 0,
    };
    let reader = table.zip(names).map(|(table, names)| Reader {
        names,
        table,
        root: root.clone(),
    });
    let mut contexts = vec![Context {
        owner: None,
        namespace: String::new(),
        class: None,
    }];
    // Calls, named once every definition is: `(owner, callee)`.
    let mut calls = Vec::new();
    let mut stack = vec![(root.clone(), 0usize)];
    while let Some((node, context)) = stack.pop() {
        let before = stack.len();
        let kind = node.kind();
        let mut inner = None;
        let mut body_only = false;
        match kind.as_ref() {
            "function_definition"
            | "method_declaration"
            | "anonymous_function"
            | "arrow_function" => {
                let outer = &contexts[context];
                let name = match kind.as_ref() {
                    "function_definition" => node
                        .field("name")
                        .map(|name| qualify(&outer.namespace, &name.text())),
                    "method_declaration" => outer
                        .class
                        .as_ref()
                        .zip(node.field("name"))
                        .map(|(class, name)| format!("{class}.{}", name.text())),
                    _ => {
                        extractor.closures += 1;
                        Some(format!(
                            "\\{{closure}}#{}@{}",
                            extractor.closures, extractor.tag
                        ))
                    }
                };
                if let Some(name) = name {
                    let method = kind == "method_declaration";
                    let modifier = |is: &dyn Fn(&Node<'_, D>) -> bool| {
                        method && node.children().any(|modifier| is(&modifier))
                    };
                    let private = modifier(&|modifier| {
                        modifier.kind() == "visibility_modifier"
                            && modifier.text().eq_ignore_ascii_case("private")
                    });
                    let final_method = modifier(&|modifier| modifier.kind() == "final_modifier");
                    let abstract_method = method
                        && (node.field("body").is_none()
                            || modifier(&|modifier| modifier.kind() == "abstract_modifier"));
                    let flags = if private { PRIVATE } else { 0 }
                        | if final_method { FINAL } else { 0 }
                        | if abstract_method { ABSTRACT } else { 0 };
                    let def = extractor.define(&node, &name, flags);
                    inner = Some(Context {
                        owner: Some(def),
                        namespace: outer.namespace.clone(),
                        class: None,
                    });
                }
            }
            kind if CLASS_KINDS.contains(&kind) => {
                let outer = &contexts[context];
                let name = if kind == "anonymous_class" {
                    extractor.classes += 1;
                    Some(format!(
                        "\\{{class}}#{}@{}",
                        extractor.classes, extractor.tag
                    ))
                } else {
                    node.field("name")
                        .map(|name| qualify(&outer.namespace, &name.text()))
                };
                if let Some(name) = name {
                    extractor.define(&node, &name, CLASS);
                    if let Some(names) = names {
                        supers(&mut extractor, &node, &name, names);
                    }
                    inner = Some(Context {
                        owner: outer.owner,
                        namespace: outer.namespace.clone(),
                        class: Some(name),
                    });
                    body_only = true;
                }
            }
            // `namespace X;` names the namespace of the statements after it (see `program`).
            "namespace_definition" if node.field("body").is_some() => {
                inner = Some(Context {
                    owner: None,
                    namespace: namespace_of(&node),
                    class: None,
                });
                body_only = true;
            }
            _ => {}
        }
        if CALL_KINDS.contains(&kind.as_ref())
            && let Some(reader) = &reader
            && let Some(found) = reader.callee(&node)
        {
            calls.push((contexts[context].owner, found));
        }
        match inner {
            Some(inner) => {
                // A class-like's or braced namespace's body is in its context; the rest (its
                // attributes, bases, a `new class`'s arguments) where it is. A function-like
                // runs its parameters' defaults and body in its own context, its attributes
                // where it is.
                contexts.push(inner);
                let inner = contexts.len() - 1;
                let body = node.field("body").map(|body| body.range());
                let attributes = node
                    .field("attributes")
                    .map(|attributes| attributes.range());
                for child in node.children() {
                    let range = Some(child.range());
                    let into =
                        if (body_only && range != body) || (!body_only && range == attributes) {
                            context
                        } else {
                            inner
                        };
                    stack.push((child, into));
                }
            }
            None if kind == "program" => {
                // Statement namespaces name what follows them: each top-level statement gets
                // its own context, updated by the `namespace X;` statements before it.
                for child in node.children() {
                    contexts.push(contexts[context].clone());
                    let own = contexts.len() - 1;
                    stack.push((child, own));
                }
                stack[before..].reverse();
                // A `namespace X;` statement changes the contexts after it.
                let mut namespace = String::new();
                for (child, own) in stack[before..].iter().rev() {
                    if child.kind() == "namespace_definition" && child.field("body").is_none() {
                        namespace = namespace_of(child);
                    }
                    contexts[*own].namespace.clone_from(&namespace);
                }
                continue;
            }
            None => stack.extend(node.children().map(|child| (child, context))),
        }
        // Children were pushed in order; pop them in order.
        stack[before..].reverse();
    }
    for (owner, found) in calls {
        let name = match found {
            Found::Name(name) => name,
            Found::Local(start) => match extractor.def_name(start) {
                Some(name) => name.to_ascii_lowercase(),
                None => {
                    extractor.open(owner);
                    continue;
                }
            },
            Found::Member(start, method, late) => match extractor.def_name(start) {
                Some(class) => member_name(class, &method, late),
                None => {
                    extractor.open(owner);
                    continue;
                }
            },
            Found::Open => {
                extractor.open(owner);
                continue;
            }
        };
        let name = extractor.intern(&name);
        extractor.call(owner, name);
    }
    debug_assert!(extractor.defs.is_sorted_by_key(|def| def.start));
    extractor.edges.sort_unstable();
    extractor.edges.dedup();
    extractor.targets.sort_unstable();
    extractor.targets.dedup();
    Summary {
        module: None,
        names: extractor
            .names
            .iter()
            .map(|name| Name::new(Family::Php, name))
            .collect(),
        defs: extractor.defs,
        edges: extractor.edges,
        targets: extractor.targets,
        aliases: Vec::new(),
        opaque: Vec::new(),
        stars: Vec::new(),
        supers: extractor.supers,
        open: has_errors || reader.is_none(),
        open_sites: extractor.open_sites,
        bytes: 0,
    }
}

/// Records what the class-like `node` named `class` extends, implements and uses, and its trait
/// adaptations, each by lowercased name.
fn supers<D: Doc>(extractor: &mut Extractor, node: &Node<'_, D>, class: &str, names: &PhpNames) {
    let class = class.to_ascii_lowercase();
    let resolved = |name: &Node<'_, D>| match &names.reference(&name.range())?.resolved {
        Resolved::Name(name) => Some(format!("\\{}", name.to_ascii_lowercase())),
        _ => None,
    };
    let add = |extractor: &mut Extractor, key: &str, target: &str, kind: u8| {
        let pair = (extractor.intern(key), extractor.intern(target), kind);
        extractor.supers.push(pair);
    };
    let interface = node.kind() == "interface_declaration";
    for clause in node.children() {
        let kind = match clause.kind().as_ref() {
            "base_clause" if interface => INTERFACE,
            "base_clause" => PARENT,
            "class_interface_clause" => INTERFACE,
            _ => continue,
        };
        for name in clause.children().filter(|child| is_name(child)) {
            if let Some(target) = resolved(&name) {
                add(extractor, &class, &target, kind);
            }
        }
    }
    let Some(body) = node.field("body") else {
        return;
    };
    for used in body
        .children()
        .filter(|member| member.kind() == "use_declaration")
    {
        for name in used.children().filter(|child| is_name(child)) {
            if let Some(target) = resolved(&name) {
                add(extractor, &class, &target, TRAIT);
            }
        }
        let Some(list) = used.children().find(|child| child.kind() == "use_list") else {
            continue;
        };
        for clause in list.children() {
            let parts = clause
                .children()
                .filter(|child| child.is_named() && !child.kind().ends_with("comment"))
                .collect::<Vec<_>>();
            // `T::m`: the trait and the method.
            let method_of = |part: &Node<'_, D>| -> Option<(Option<String>, String)> {
                match part.kind().as_ref() {
                    "class_constant_access_expression" => {
                        let mut named = part.children().filter(|child| child.is_named());
                        let (trait_name, method) = (named.next()?, named.next()?);
                        Some((resolved(&trait_name), method.text().to_ascii_lowercase()))
                    }
                    "name" => Some((None, part.text().to_ascii_lowercase())),
                    _ => None,
                }
            };
            match clause.kind().as_ref() {
                "use_instead_of_clause" => {
                    let Some((Some(_), method)) = parts.first().and_then(method_of) else {
                        continue;
                    };
                    for excluded in &parts[1..] {
                        if let Some(excluded) = resolved(excluded) {
                            add(extractor, &format!("{class}.{method}"), &excluded, EXCLUDE);
                        }
                    }
                }
                "use_as_clause" => {
                    let Some(first) = parts.first() else {
                        continue;
                    };
                    let Some((trait_name, method)) = method_of(first) else {
                        continue;
                    };
                    // `T::m`, or the `m` of whichever used trait has it.
                    let (target, alias_kind) = match trait_name {
                        Some(trait_name) => (format!("{trait_name}.{method}"), ALIAS),
                        None if first.kind() == "name" => (method.clone(), ALIAS_TRAITS),
                        None => continue,
                    };
                    let visibility = |modifier: &Node<'_, D>| {
                        if modifier.text().eq_ignore_ascii_case("private") {
                            PRIVATE_AS
                        } else {
                            VISIBLE_AS
                        }
                    };
                    let (modifier, alias) = match &parts[1..] {
                        [modifier] if modifier.kind() == "visibility_modifier" => {
                            (Some(visibility(modifier)), None)
                        }
                        // tree-sitter-php 0.24.2 reads PHP 8.3's `m as final` as an alias named
                        // `final`, a name PHP never lets an alias take.
                        [name]
                            if name.kind() == "name"
                                && name.text().eq_ignore_ascii_case("final") =>
                        {
                            (Some(FINAL_AS), None)
                        }
                        [name] if name.kind() == "name" => (None, Some(name)),
                        [modifier, name]
                            if modifier.kind() == "visibility_modifier"
                                && name.kind() == "name" =>
                        {
                            (Some(visibility(modifier)), Some(name))
                        }
                        _ => continue,
                    };
                    let imported = alias.map_or(method, |alias| alias.text().to_ascii_lowercase());
                    let key = format!("{class}.{imported}");
                    if alias.is_some() {
                        add(extractor, &key, &target, alias_kind);
                    }
                    if let Some(modifier) = modifier {
                        add(extractor, &key, &target, modifier);
                    }
                }
                _ => {}
            }
        }
    }
}

/// What the call `call` in `summary`'s file names: the same answer extraction recorded for it.
/// `None` where it names nothing.
pub(crate) fn callee_at<'r, D: Doc>(
    reader: &Reader<'_, 'r, D>,
    summary: &Summary,
    call: &Node<'r, D>,
) -> Option<super::summary::Callee> {
    use super::summary::Callee;
    let def_name = |start: u32| summary.def_at(start).map(|def| summary.name(def.name));
    Some(match reader.callee(call)? {
        Found::Name(name) => Callee::Name(name),
        Found::Local(start) => match def_name(start) {
            Some(name) => Callee::Name(name.to_ascii_lowercase()),
            None => Callee::Open,
        },
        Found::Member(start, method, late) => match def_name(start) {
            Some(class) => Callee::Name(member_name(class, &method, late)),
            None => Callee::Open,
        },
        Found::Open => Callee::Open,
    })
}

/// The callee name of `class`'s `method` (lowercased): `\c.m`, `\c.m^` late-bound.
fn member_name(class: &str, method: &str, late: bool) -> String {
    let mut name = format!("{}.{method}", class.to_ascii_lowercase());
    if late {
        name.push('^');
    }
    name
}

/// The display form of a PHP definition's name: `App\C::m`, `App\f`, `{closure}`,
/// `class@anonymous::m`.
pub(crate) fn label(name: &str) -> String {
    let name = name.strip_prefix('\\').unwrap_or(name);
    let local = |name: &str, tag: &str, shown: &str| {
        name.strip_prefix(tag)
            .map(|rest| match rest.split_once('.') {
                Some((_, member)) => format!("{shown}::{member}"),
                None => shown.to_owned(),
            })
    };
    if let Some(label) = local(name, "{closure}#", "{closure}") {
        return label;
    }
    if let Some(label) = local(name, "{class}#", "class@anonymous") {
        return label;
    }
    match name.rsplit_once('.') {
        Some((class, member)) => format!("{class}::{member}"),
        None => name.to_owned(),
    }
}
