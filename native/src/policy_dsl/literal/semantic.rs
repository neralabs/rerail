//! The semantic syntax errors Ruff 0.16.9 reports in a string type definition it parsed
//! (`ruff_python_parser::semantic_errors`, `SemanticSyntaxChecker::visit_expr`): Ruff's checker
//! visits the parsed expression in the scope the string is in and reports each error as F722 at
//! the offending node (the whole string for a complex one). An expression can raise only the
//! checks of `visit_expr`; which apply depends on the scopes around the node: the host's, then
//! the lambdas and comprehensions inside the string.

use super::PNode;
use super::annotation::kind_name;
use crate::policy_dsl::identifier::nfkc;
use std::borrow::Cow;
use std::collections::HashSet;

/// A scope around a node, as Ruff's `ScopeKind` tells them apart for these checks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Scope {
    Module,
    Class,
    Function {
        is_async: bool,
    },
    Lambda,
    /// A PEP 695 type parameters' scope.
    Type,
    Generator {
        kind: Comprehension,
        is_async: bool,
    },
}

/// Ruff's `GeneratorKind`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Comprehension {
    Generator,
    List,
    Set,
    Dict,
}

impl Comprehension {
    /// The comprehension a tree-sitter kind is.
    pub(crate) fn of(kind: &str) -> Option<Self> {
        match kind {
            "generator_expression" => Some(Self::Generator),
            "list_comprehension" => Some(Self::List),
            "set_comprehension" => Some(Self::Set),
            "dictionary_comprehension" => Some(Self::Dict),
            _ => None,
        }
    }
}

/// Where the string is: its scopes, outermost first, and its innermost scope's `global`
/// declarations (name, statement start).
pub(crate) struct Host<'h> {
    pub(crate) scopes: Vec<Scope>,
    pub(crate) globals: &'h dyn Fn(&str) -> Option<usize>,
    /// The target version's minor (asynchronous comprehensions nest from 3.11).
    pub(crate) minor: u8,
}

/// One error: a decoded range and Ruff's message.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SemanticError {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) message: String,
}

/// The errors Ruff's checker reports in the expression under `root` (an `expression_statement`),
/// in the order it reports them. `text` is the decoded text; `source_start` maps a decoded offset
/// to where Ruff places it in the source (for `global` declarations).
pub(crate) fn check(
    root: &PNode,
    text: &str,
    host: &Host<'_>,
    source_start: &dyn Fn(usize) -> usize,
) -> Vec<SemanticError> {
    let mut checker = Checker {
        text,
        host,
        source_start,
        scopes: host.scopes.clone(),
        errors: Vec::new(),
    };
    for child in named(root) {
        checker.expression(child);
    }
    checker.errors
}

fn kind(node: &PNode) -> &'static str {
    kind_name(node.kind)
}

fn named(node: &PNode) -> impl Iterator<Item = &PNode> {
    node.children
        .iter()
        .filter(|child| child.named && kind(child) != "comment")
}

fn field<'n>(node: &'n PNode, name: &str) -> Option<&'n PNode> {
    fields(node, name).next()
}

fn fields<'n>(node: &'n PNode, name: &str) -> impl Iterator<Item = &'n PNode> {
    let id = super::annotation::python()
        .field_id_for_name(name)
        .map_or(0, |id| id.get());
    node.children
        .iter()
        .filter(move |child| id != 0 && child.field == id)
}

/// One clause of a comprehension, as Ruff's `Comprehension`: a `for` clause with the `if` clauses
/// after it; its range covers them.
struct Clause<'n> {
    target: Option<&'n PNode>,
    iter: Vec<&'n PNode>,
    ifs: Vec<&'n PNode>,
    is_async: bool,
    start: usize,
    end: usize,
}

fn clauses(node: &PNode) -> Vec<Clause<'_>> {
    let mut clauses: Vec<Clause<'_>> = Vec::new();
    for child in named(node) {
        match kind(child) {
            "for_in_clause" => clauses.push(Clause {
                target: field(child, "left"),
                iter: fields(child, "right").collect(),
                ifs: Vec::new(),
                is_async: child
                    .children
                    .iter()
                    .any(|token| !token.named && kind(token) == "async"),
                start: child.start,
                end: child.end,
            }),
            "if_clause" => {
                if let Some(clause) = clauses.last_mut() {
                    clause.end = child.end;
                    clause.ifs.extend(named(child));
                }
            }
            _ => {}
        }
    }
    clauses
}

/// The comprehension's element expressions: a dict's key and value, or the element.
fn elements(node: &PNode) -> Vec<&PNode> {
    match field(node, "body") {
        Some(body) if kind(body) == "pair" => named(body).collect(),
        Some(body) => vec![body],
        None => Vec::new(),
    }
}

/// The names a comprehension target stores (Ruff's `StoredNameFinder`): through tuples, lists
/// and stars, not into attributes or subscripts.
fn stored_names<'t>(target: &PNode, text: &'t str, names: &mut HashSet<Cow<'t, str>>) {
    match kind(target) {
        "identifier" => {
            names.insert(nfkc(&text[target.start..target.end]));
        }
        "pattern_list"
        | "tuple_pattern"
        | "list_pattern"
        | "tuple"
        | "list"
        | "list_splat_pattern"
        | "list_splat"
        | "parenthesized_expression" => {
            for child in named(target) {
                stored_names(child, text, names);
            }
        }
        _ => {}
    }
}

fn is_starred(node: &PNode) -> bool {
    matches!(kind(node), "list_splat" | "list_splat_pattern")
}

struct Checker<'c> {
    text: &'c str,
    host: &'c Host<'c>,
    source_start: &'c dyn Fn(usize) -> usize,
    /// Scopes around the current node, outermost first.
    scopes: Vec<Scope>,
    errors: Vec<SemanticError>,
}

impl<'c> Checker<'c> {
    fn error(&mut self, start: usize, end: usize, message: String) {
        self.errors.push(SemanticError {
            start,
            end,
            message,
        });
    }

    /// An identifier's name, NFKC-normalized as Ruff's parser reads it.
    fn name(&self, node: &PNode) -> Cow<'c, str> {
        nfkc(&self.text[node.start..node.end])
    }

    fn innermost(&self) -> impl Iterator<Item = &Scope> {
        self.scopes.iter().rev()
    }

    fn in_function_scope(&self) -> bool {
        matches!(
            self.scopes.last(),
            Some(Scope::Function { .. } | Scope::Lambda)
        )
    }

    fn in_async_context(&self) -> bool {
        for scope in self.innermost() {
            match scope {
                Scope::Class | Scope::Lambda => return false,
                Scope::Function { is_async } => return *is_async,
                _ => {}
            }
        }
        false
    }

    fn in_await_allowed_context(&self) -> bool {
        for scope in self.innermost() {
            match scope {
                Scope::Class => return false,
                Scope::Function { .. }
                | Scope::Lambda
                | Scope::Generator {
                    kind: Comprehension::Generator,
                    ..
                } => return true,
                _ => {}
            }
        }
        false
    }

    fn in_yield_allowed_context(&self) -> bool {
        for scope in self.innermost() {
            match scope {
                Scope::Class | Scope::Generator { .. } => return false,
                Scope::Function { .. } | Scope::Lambda => return true,
                _ => {}
            }
        }
        false
    }

    fn in_generator_context(&self) -> bool {
        self.scopes.iter().any(|scope| {
            matches!(
                scope,
                Scope::Generator {
                    kind: Comprehension::Generator,
                    ..
                }
            )
        })
    }

    fn in_sync_comprehension(&self) -> bool {
        self.scopes.iter().any(|scope| {
            matches!(
                scope,
                Scope::Generator {
                    kind: Comprehension::List | Comprehension::Set | Comprehension::Dict,
                    is_async: false,
                }
            )
        })
    }

    fn in_class_body_comprehension(&self) -> bool {
        for scope in self.innermost() {
            match scope {
                Scope::Generator { .. } => {}
                Scope::Class => return true,
                _ => return false,
            }
        }
        false
    }

    /// F704's `yield_outside_function`, Ruff's first check of `await`, `yield` and `yield from`.
    fn yield_outside_function(&mut self, start: usize, end: usize, keyword: &str) {
        if self.in_function_scope() {
            return;
        }
        let allowed = if keyword == "await" {
            self.in_await_allowed_context()
        } else {
            self.in_yield_allowed_context()
        };
        if !allowed {
            self.error(
                start,
                end,
                format!("`{keyword}` statement outside of a function"),
            );
        }
    }

    fn await_outside_async_function(&mut self, start: usize, end: usize, what: &str) {
        if self.in_async_context() || self.in_generator_context() {
            return;
        }
        self.error(
            start,
            end,
            format!("{what} outside of an asynchronous function"),
        );
    }

    /// `__debug__` stored.
    fn store_name(&mut self, node: &PNode) {
        if kind(node) == "identifier" && self.name(node) == "__debug__" {
            self.error(node.start, node.end, "cannot assign to `__debug__`".into());
        }
    }

    /// A comprehension target, stored: `__debug__`, two stars in one tuple or list.
    fn target(&mut self, node: &PNode) {
        match kind(node) {
            "identifier" => self.store_name(node),
            "pattern_list" | "tuple_pattern" | "list_pattern" | "tuple" | "list" => {
                if named(node).filter(|child| is_starred(child)).count() > 1 {
                    self.error(
                        node.start,
                        node.end,
                        "Two starred expressions in assignment".into(),
                    );
                }
                for child in named(node) {
                    self.target(child);
                }
            }
            "list_splat_pattern" | "list_splat" | "parenthesized_expression" => {
                for child in named(node) {
                    self.target(child);
                }
            }
            _ => self.expression(node),
        }
    }

    /// Visits an expression: Ruff's checks on the node in the scopes around it, then its parts,
    /// each in the scope Ruff visits it in.
    fn expression(&mut self, node: &PNode) {
        match kind(node) {
            "identifier" => self.before_global(node),
            "await" => {
                let (start, end) = (node.start, await_end(node));
                self.yield_outside_function(start, end, "await");
                self.await_outside_async_function(start, end, "`await`");
                self.children(node);
            }
            "yield" => {
                let from = node
                    .children
                    .iter()
                    .any(|token| !token.named && kind(token) == "from");
                if from {
                    self.yield_outside_function(node.start, node.end, "yield from");
                    if self.in_function_scope() && self.in_async_context() {
                        self.error(
                            node.start,
                            node.end,
                            "`yield from` statement in async function; use `async for` instead"
                                .into(),
                        );
                    }
                } else {
                    if let Some(value) = named(node).next()
                        && is_starred(value)
                    {
                        self.error(
                            value.start,
                            value.end,
                            "Starred expression cannot be used here".into(),
                        );
                    }
                    self.yield_outside_function(node.start, node.end, "yield");
                }
                self.children(node);
            }
            "named_expression" => {
                if let Some(name) = field(node, "name") {
                    self.store_name(name);
                    self.before_global(name);
                }
                if let Some(value) = field(node, "value") {
                    self.expression(value);
                }
            }
            "lambda" => self.lambda(node),
            "call" => {
                if let Some(arguments) = field(node, "arguments") {
                    let mut seen = HashSet::new();
                    for keyword in named(arguments).filter(|item| kind(item) == "keyword_argument")
                    {
                        if let Some(name) = field(keyword, "name")
                            && !seen.insert(self.name(name))
                        {
                            let message =
                                format!("Duplicate keyword argument `{}`", self.name(name));
                            self.error(keyword.start, keyword.end, message);
                        }
                    }
                }
                self.children(node);
            }
            "keyword_argument" => {
                if let Some(value) = field(node, "value") {
                    self.expression(value);
                }
            }
            "attribute" => {
                if let Some(object) = field(node, "object") {
                    self.expression(object);
                }
            }
            kind if Comprehension::of(kind).is_some() => self.comprehension(node),
            _ => self.children(node),
        }
    }

    fn children(&mut self, node: &PNode) {
        for child in named(node) {
            self.expression(child);
        }
    }

    /// A name read or stored (PLE0118) in the string's own scope before that scope's `global`
    /// statement for it.
    fn before_global(&mut self, node: &PNode) {
        if self.scopes.len() != self.host.scopes.len() {
            return;
        }
        let name = self.name(node);
        if let Some(statement) = (self.host.globals)(&name)
            && (self.source_start)(node.start) < statement
        {
            let message = format!("name `{name}` is used prior to global declaration");
            self.error(node.start, node.end, message);
        }
    }

    fn lambda(&mut self, node: &PNode) {
        let mut seen = HashSet::new();
        let mut defaults = Vec::new();
        if let Some(parameters) = field(node, "parameters") {
            let names: Vec<&PNode> = named(parameters)
                .filter_map(|parameter| match kind(parameter) {
                    "identifier" => Some(parameter),
                    "default_parameter" => {
                        defaults.extend(field(parameter, "value"));
                        field(parameter, "name")
                    }
                    "list_splat_pattern" | "dictionary_splat_pattern" => named(parameter).next(),
                    _ => None,
                })
                .collect();
            for name in &names {
                self.store_name(name);
            }
            if names.len() > 1 {
                for name in names {
                    if !seen.insert(self.name(name)) {
                        let message = format!("Duplicate parameter \"{}\"", self.name(name));
                        self.error(name.start, name.end, message);
                    }
                }
            }
        }
        for default in defaults {
            self.expression(default);
        }
        self.scopes.push(Scope::Lambda);
        if let Some(body) = field(node, "body") {
            self.expression(body);
        }
        self.scopes.pop();
    }

    fn comprehension(&mut self, node: &PNode) {
        let generator = Comprehension::of(kind(node)).unwrap_or(Comprehension::Generator);
        let clauses = clauses(node);
        let elements = elements(node);
        let mut all_targets = HashSet::new();
        for clause in &clauses {
            if let Some(target) = clause.target {
                stored_names(target, self.text, &mut all_targets);
            }
        }
        // `check_generator_expr` on each element.
        for element in &elements {
            self.rebound(element, &all_targets, &HashSet::new());
            self.class_body(element);
        }
        // `check_generator_clauses`.
        for (index, clause) in clauses.iter().enumerate() {
            if let Some(target) = clause.target
                && is_starred(target)
            {
                self.error(
                    target.start,
                    target.end,
                    "Starred expression cannot be used here".into(),
                );
            }
            let mut before = HashSet::new();
            let mut after = HashSet::new();
            for (other, later) in clauses.iter().enumerate() {
                if let Some(target) = later.target {
                    let names = if other <= index {
                        &mut before
                    } else {
                        &mut after
                    };
                    stored_names(target, self.text, names);
                }
            }
            for test in &clause.ifs {
                self.rebound(test, &before, &after);
                self.class_body(test);
            }
            for iter in &clause.iter {
                self.named_in_iterable(iter);
            }
        }
        if generator != Comprehension::Generator {
            if self.host.minor < 11 && self.in_sync_comprehension() {
                for clause in clauses.iter().filter(|clause| clause.is_async) {
                    let message = format!(
                        "cannot use an asynchronous comprehension inside of a synchronous comprehension on Python 3.{} (syntax was added in 3.11)",
                        self.host.minor
                    );
                    self.error(clause.start, clause.end, message);
                }
            }
            for clause in clauses.iter().filter(|clause| clause.is_async) {
                self.await_outside_async_function(
                    clause.start,
                    clause.end,
                    "asynchronous comprehension",
                );
            }
        }
        // The first iterable in the outer scope; the rest in the comprehension's.
        let mut rest = clauses.iter();
        if let Some(first) = rest.next() {
            for iter in &first.iter {
                self.expression(iter);
            }
            self.scopes.push(Scope::Generator {
                kind: generator,
                is_async: clauses.iter().any(|clause| clause.is_async),
            });
            if let Some(target) = first.target {
                self.target(target);
            }
            for test in &first.ifs {
                self.expression(test);
            }
            for clause in rest {
                for iter in &clause.iter {
                    self.expression(iter);
                }
                if let Some(target) = clause.target {
                    self.target(target);
                }
                for test in &clause.ifs {
                    self.expression(test);
                }
            }
            for element in &elements {
                self.expression(element);
            }
            self.scopes.pop();
        }
    }

    /// Ruff's `ReboundComprehensionVisitor`: a walrus storing a comprehension's target.
    fn rebound(
        &mut self,
        node: &PNode,
        targets: &HashSet<Cow<'_, str>>,
        direct: &HashSet<Cow<'_, str>>,
    ) {
        match kind(node) {
            "lambda" => {
                if let Some(parameters) = field(node, "parameters") {
                    for default in named(parameters)
                        .filter(|parameter| kind(parameter) == "default_parameter")
                        .filter_map(|parameter| field(parameter, "value"))
                    {
                        self.rebound(default, targets, direct);
                    }
                }
                return;
            }
            kind if Comprehension::of(kind).is_some() && !direct.is_empty() => {
                for child in named(node) {
                    self.rebound(child, targets, &HashSet::new());
                }
                return;
            }
            "named_expression" => {
                if let Some(name) = field(node, "name")
                    && kind(name) == "identifier"
                {
                    let text = self.name(name);
                    if targets.contains(text.as_ref()) || direct.contains(text.as_ref()) {
                        self.error(
                            name.start,
                            name.end,
                            "assignment expression cannot rebind comprehension variable".into(),
                        );
                    }
                }
            }
            _ => {}
        }
        for child in named(node) {
            self.rebound(child, targets, direct);
        }
    }

    /// Ruff's `ClassBodyNamedExpressionVisitor`, in a comprehension in a class body.
    fn class_body(&mut self, node: &PNode) {
        if !self.in_class_body_comprehension() {
            return;
        }
        self.class_body_walrus(node);
    }

    fn class_body_walrus(&mut self, node: &PNode) {
        match kind(node) {
            "lambda" => {
                if let Some(parameters) = field(node, "parameters") {
                    for default in named(parameters)
                        .filter(|parameter| kind(parameter) == "default_parameter")
                        .filter_map(|parameter| field(parameter, "value"))
                    {
                        self.class_body_walrus(default);
                    }
                }
                return;
            }
            kind if Comprehension::of(kind).is_some() => return,
            "named_expression"
                if field(node, "name").is_some_and(|name| kind(name) == "identifier") =>
            {
                self.error(
                    node.start,
                    node.end,
                    "assignment expression within a comprehension cannot be used in a class body"
                        .into(),
                );
            }
            _ => {}
        }
        for child in named(node) {
            self.class_body_walrus(child);
        }
    }

    /// Ruff's `ComprehensionIterableNamedExpressionVisitor`: any walrus in an iterable.
    fn named_in_iterable(&mut self, node: &PNode) {
        if kind(node) == "named_expression"
            && field(node, "name").is_some_and(|name| kind(name) == "identifier")
        {
            self.error(
                node.start,
                node.end,
                "assignment expression cannot be used in a comprehension iterable expression"
                    .into(),
            );
        }
        for child in named(node) {
            self.named_in_iterable(child);
        }
    }
}

/// Where Ruff's `await` ends: tree-sitter-python binds `await` looser than `**`, Python tighter
/// (`await a ** b` is `(await a) ** b`), so the operand ends at the leftmost power's base.
fn await_end(node: &PNode) -> usize {
    let mut operand = named(node).next();
    while let Some(power) = operand {
        if kind(power) != "binary_operator"
            || !power
                .children
                .iter()
                .any(|token| !token.named && kind(token) == "**")
        {
            break;
        }
        operand = field(power, "left");
    }
    operand.map_or(node.end, |operand| operand.end)
}

#[cfg(test)]
mod tests {
    use super::super::annotation::expression;
    use super::*;

    fn errors(text: &str, scopes: Vec<Scope>) -> Vec<String> {
        let root = expression(text, false).expect("parses");
        let host = Host {
            scopes,
            globals: &|name| (name == "g").then_some(100),
            minor: 10,
        };
        check(&root, text, &host, &|at| at)
            .into_iter()
            .map(|error| format!("{}:{}", &text[error.start..error.end], error.message))
            .collect()
    }

    #[test]
    fn errors_depend_on_the_scopes_around_the_node() {
        let module = || vec![Scope::Module];
        assert_eq!(
            errors("await y", module()),
            [
                "await y:`await` statement outside of a function",
                "await y:`await` outside of an asynchronous function",
            ]
        );
        assert_eq!(
            errors(
                "await y",
                vec![Scope::Module, Scope::Function { is_async: false }]
            ),
            ["await y:`await` outside of an asynchronous function"]
        );
        assert_eq!(
            errors(
                "await y",
                vec![Scope::Module, Scope::Function { is_async: true }]
            ),
            Vec::<String>::new()
        );
        assert_eq!(
            errors("await y ** 2", module())[0],
            "await y:`await` statement outside of a function"
        );
        assert_eq!(
            errors("lambda: await y", module()),
            ["await y:`await` outside of an asynchronous function"]
        );
        assert_eq!(
            errors("(await a for a in b)", module()),
            Vec::<String>::new()
        );
        assert_eq!(
            errors("[[a async for a in b] for c in d]", module()),
            [
                "async for a in b:cannot use an asynchronous comprehension inside of a synchronous comprehension on Python 3.10 (syntax was added in 3.11)",
                "async for a in b:asynchronous comprehension outside of an asynchronous function",
            ]
        );
        assert_eq!(
            errors("[(q := 1) for a in b]", vec![Scope::Module, Scope::Class]),
            ["q := 1:assignment expression within a comprehension cannot be used in a class body"]
        );
        assert_eq!(
            errors("[x for a in b if (c := 1) for c in d]", module()),
            ["c:assignment expression cannot rebind comprehension variable"]
        );
        assert_eq!(
            errors("[a for a in (b := c)]", module()),
            ["b := c:assignment expression cannot be used in a comprehension iterable expression"]
        );
        assert_eq!(
            errors("f(a=1, a=2) or (lambda a, a: 1)", module()),
            [
                "a=2:Duplicate keyword argument `a`",
                "a:Duplicate parameter \"a\"",
            ]
        );
        assert_eq!(
            errors("[a for (*a, *c) in b] or (__debug__ := 1)", module()),
            [
                "(*a, *c):Two starred expressions in assignment",
                "__debug__:cannot assign to `__debug__`",
            ]
        );
        assert_eq!(
            errors(
                "g or [g for x in y]",
                vec![Scope::Module, Scope::Function { is_async: false }]
            ),
            ["g:name `g` is used prior to global declaration"]
        );
        // Names are read under NFKC, as Ruff's parser reads them; errors stay at their text.
        assert_eq!(
            errors(
                "f(a=1, 𝐚=2) or (lambda a, 𝐚: 1) or [x for 𝐜 in d if (c := 1)] or (__𝐝𝐞𝐛𝐮𝐠__ := 1)",
                module()
            ),
            [
                "𝐚=2:Duplicate keyword argument `a`",
                "𝐚:Duplicate parameter \"a\"",
                "c:assignment expression cannot rebind comprehension variable",
                "__𝐝𝐞𝐛𝐮𝐠__:cannot assign to `__debug__`",
            ]
        );
        assert_eq!(
            errors(
                "𝐠",
                vec![Scope::Module, Scope::Function { is_async: false }]
            ),
            ["𝐠:name `g` is used prior to global declaration"]
        );
    }
}
