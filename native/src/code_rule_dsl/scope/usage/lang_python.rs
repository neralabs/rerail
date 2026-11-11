//! Python's usage index (moved from `usage.rs` by the crate split).

use super::*;

/// Python's usage index (`Usage`), its crate's.
pub(crate) trait PythonUsage: Sized {
    /// Python's index: Ruff's semantic model, with what a binding inherits from the one it
    /// shadows.
    fn python<D: Doc>(
        table: &ScopeTable,
        root: &Node<'_, D>,
        steps: &mut Steps<'_>,
    ) -> Result<Self, BuildError>;
}

impl PythonUsage for Usage {
    /// Python's index: Ruff's semantic model, with what a binding inherits from the one it
    /// shadows.
    fn python<D: Doc>(
        table: &ScopeTable,
        root: &Node<'_, D>,
        steps: &mut Steps<'_>,
    ) -> Result<Self, BuildError> {
        let text = root.text();
        let base = root.range().start;
        let mut facts: Vec<HashMap<Box<str>, Vec<Facts>>> = table
            .scopes
            .iter()
            .map(|scope| {
                scope
                    .bindings
                    .iter()
                    .map(|(name, bindings)| (name.clone(), vec![Facts::default(); bindings.len()]))
                    .collect()
            })
            .collect();
        // Ruff honours `locals()` only in the function that calls it, not in a lambda or a
        // comprehension, and reads no other builtin (`vars`, `eval`, `exec`, `globals`) so.
        let mut uses_locals = vec![false; table.scopes.len()];
        for &scope in &table.locals_scopes {
            uses_locals[scope as usize] = table.scopes[scope as usize].kind == ScopeKind::Function;
        }
        // Per function and name, its first read (by where it is read) of the binding its own
        // binding of the name shadows, and whether that is the builtin.
        let mut first_reads: HashMap<(u32, Cow<'_, str>), (usize, usize, bool)> = HashMap::new();
        // Where each read of a binding that an `except` name later shadows is made: `None` for one
        // Ruff makes after the whole scope (from a function body), and whether it is nested.
        #[allow(clippy::type_complexity)]
        let mut positions: HashMap<(u32, Cow<'_, str>, usize), Vec<(Option<usize>, bool)>> =
            HashMap::new();
        for reference in &table.references {
            steps.take()?;
            let Some(name) = table.reference_name(reference, &text, base) else {
                continue;
            };
            let reads = table.reads(reference, &name);
            let before = match table.shadowed_by_local(reference, &name) {
                Some(Shadowed::Binding(shadowed)) => reads.contains(&shadowed).then_some(false),
                // A read that resolves to nothing reads the builtin, if the name is one.
                Some(Shadowed::Builtin) => reads.is_empty().then_some(true),
                None => None,
            };
            if let Some(builtin) = before {
                let key = (reference.scope, name.clone());
                let at = (reference.at, reference.start, builtin);
                first_reads
                    .entry(key)
                    .and_modify(|first| *first = (*first).min(at))
                    .or_insert(at);
            }
            for (scope, index) in reads {
                let read = &mut facts[scope as usize]
                    .get_mut(name.as_ref())
                    .expect("a resolved name has facts")[index];
                read.reads += 1;
                read.nested += u32::from(reference.scope != scope);
                // An `except` name inherits only the reads made before it (below); a read after
                // its handler may resolve to the binding before it.
                let chain = &table.scopes[scope as usize].bindings[name.as_ref()];
                if chain[index + 1..]
                    .iter()
                    .any(|binding| binding.until.is_some())
                {
                    let late = reference.deferred || table.crosses_function(reference.scope, scope);
                    positions
                        .entry((scope, name.clone(), index))
                        .or_default()
                        .push(((!late).then_some(reference.at), reference.scope != scope));
                }
            }
            if let Some((scope, submodule, index)) = table.submodule_alias(reference, &name) {
                let read = &mut facts[scope as usize]
                    .get_mut(submodule)
                    .expect("a submodule import has facts")[index];
                read.reads += 1;
                read.nested += u32::from(reference.scope != scope);
            }
        }
        let exports = dunder_all(table, root, steps)?;
        for (_, _, name) in &exports.entries {
            if let Some(bindings) = table.scopes[0].bindings.get(name.as_str())
                && let Some(last) = facts[0]
                    .get_mut(name.as_str())
                    .and_then(|facts| facts.get_mut(bindings.len() - 1))
            {
                last.reads += 1;
            }
        }
        for (scope_index, scope) in table.scopes.iter().enumerate() {
            let module = scope_index == 0;
            for (name, bindings) in &scope.bindings {
                steps.take()?;
                let chain = facts[scope_index]
                    .get_mut(name)
                    .expect("every binding has facts");
                let unknown = module && !exports.complete;
                let mut previous: Option<(bool, bool)> = None;
                let mut previous_inherited = (false, false);
                for (index, binding) in bindings.iter().enumerate() {
                    let inherited = if index > 0 && inherits(&bindings[index - 1], binding) {
                        if binding.until.is_some() {
                            // Ruff copies the references when it adds the binding: an `except`
                            // name gets the reads made before its handler, not those after it,
                            // which resolve to the binding before it.
                            let early = positions
                                .get(&(scope_index as u32, Cow::Borrowed(name.as_ref()), index - 1))
                                .into_iter()
                                .flatten()
                                .filter(|(at, _)| at.is_some_and(|at| at < binding.offset));
                            let (mut used, mut nested) = previous_inherited;
                            for (_, read_nested) in early {
                                used = true;
                                nested |= *read_nested;
                            }
                            Some((used, nested))
                        } else {
                            previous
                        }
                    } else {
                        None
                    };
                    previous_inherited = inherited.unwrap_or_default();
                    let unknown = unknown
                        || (uses_locals[scope_index] && LOCALS_FORMS.contains(&binding.site.form));
                    let own = &mut chain[index];
                    let used = own.reads > 0 || inherited.is_some_and(|(used, _)| used);
                    let nested = own.nested > 0 || inherited.is_some_and(|(_, nested)| nested);
                    own.used = known(used, unknown);
                    own.nested_use = known(nested, unknown);
                    own.referenced = known(own.reads > 0, unknown);
                    own.exported = if binding.explicit_export
                        || (module && exports.names.contains(name.as_ref()))
                    {
                        Some(true)
                    } else if module && !exports.complete {
                        None
                    } else {
                        Some(false)
                    };
                    previous = Some((used, nested));
                }
            }
        }
        let sites = name_sites(table, steps)?;
        let mut entries = exports
            .entries
            .into_iter()
            .map(|(start, end, name)| (start as u32, end as u32, name.into_boxed_str()))
            .collect::<Vec<_>>();
        entries.sort_by_key(|(start, end, _)| (*start, *end));
        let mut before_local = first_reads
            .into_values()
            .map(|(_, start, builtin)| (start, builtin))
            .collect::<Vec<_>>();
        before_local.sort_unstable();
        Ok(Self {
            facts,
            sites,
            exports: entries,
            comment_exports: BTreeSet::new(),
            occurrences: HashMap::new(),
            privates: Vec::new(),
            private_names: Vec::new(),
            before_local,
            cost: steps.cost,
        })
    }
}

/// The names the module's `__all__` definitions list, and whether every definition is static.
pub(super) struct Exports {
    pub(super) names: BTreeSet<String>,
    /// Each string entry: its range and the name.
    pub(super) entries: Vec<(usize, usize, String)>,
    pub(super) complete: bool,
}

impl Exports {
    /// Records a display's element: a string, in any parentheses (Ruff's AST has none), at the
    /// string's own range; anything else makes the exports incomplete.
    fn add<D: Doc>(&mut self, element: &Node<'_, D>) {
        let element = unparenthesized(element);
        match string_name(&element) {
            Some(name) => {
                let range = element.range();
                self.names.insert(name.clone());
                self.entries.push((range.start, range.end, name));
            }
            None => self.complete = false,
        }
    }
}

/// Ruff's `extract_dunder_all_names` over each module-level `__all__ =`, `+=` or `: T =`: string
/// entries of lists and tuples (a bare tuple on the right side too), joined by `+`, and
/// `list(...)`/`tuple(...)` of a display. Anything else, a non-string entry included, makes the
/// exports incomplete (Ruff ignores it).
pub(super) fn dunder_all<D: Doc>(
    table: &ScopeTable,
    root: &Node<'_, D>,
    steps: &mut Steps<'_>,
) -> Result<Exports, BuildError> {
    let mut exports = Exports {
        names: BTreeSet::new(),
        entries: Vec::new(),
        complete: true,
    };
    for statement in &table.dunder_all {
        steps.take()?;
        let Some(statement) = statement.find(root.clone()) else {
            continue;
        };
        let mut value = statement.field("right");
        while let Some(inner) = value.clone().filter(|value| value.kind() == "assignment") {
            value = inner.field("right");
        }
        match value {
            Some(value) => extract_value(table, &value, &mut exports, steps)?,
            None => exports.complete = false,
        }
    }
    Ok(exports)
}

/// An `__all__` statement's value: a chain of binary operations, any operator, reads the right
/// side of each and the leftmost operand (Ruff's loop in `extract_dunder_all_names`); a binary
/// operation on a right side is not read.
fn extract_value<D: Doc>(
    table: &ScopeTable,
    value: &Node<'_, D>,
    exports: &mut Exports,
    steps: &mut Steps<'_>,
) -> Result<(), BuildError> {
    let mut current = unparenthesized(value);
    while current.kind() == "binary_operator" {
        steps.take()?;
        let (Some(left), Some(right)) = (current.field("left"), current.field("right")) else {
            exports.complete = false;
            return Ok(());
        };
        extract(table, &unparenthesized(&right), exports, steps)?;
        current = unparenthesized(&left);
    }
    extract(table, &current, exports, steps)
}

/// The expression inside any parentheses around `node` (Ruff's AST has none).
fn unparenthesized<'r, D: Doc>(node: &Node<'r, D>) -> Node<'r, D> {
    let mut node = node.clone();
    while node.kind() == "parenthesized_expression" {
        let mut inner = node
            .named_code();
        let (Some(expression), None) = (inner.next(), inner.next()) else {
            break;
        };
        drop(inner);
        node = expression;
    }
    node
}

/// Whether a call's callee is the builtin `list` or `tuple`, as Ruff's `resolve_builtin_symbol`
/// reads it through `map_subscript`: the name, unshadowed where the call is (or imported from
/// `builtins`), or `builtins.list` through an import of `builtins`; `list[str]` too.
fn builtin_sequence<D: Doc>(table: &ScopeTable, function: &Node<'_, D>) -> bool {
    let mut callee = unparenthesized(function);
    if callee.kind() == "subscript"
        && let Some(value) = callee.field("value")
    {
        callee = unparenthesized(&value);
    }
    let sequence = |name: &str| matches!(name, "list" | "tuple");
    let imports = |binding: &Binding, qualified: &dyn Fn(&str) -> bool| match &binding.import {
        Some(super::super::Import::Qualified(name)) => qualified(name),
        _ => false,
    };
    match callee.kind().as_ref() {
        "identifier" => {
            let name = python_identifier(&callee);
            let Some(reference) = table.reference(&callee.range()) else {
                return false;
            };
            match table.resolve(reference, &name) {
                None => sequence(&name),
                Some(binding) if !binding.shadows() => sequence(&name),
                Some(binding) => imports(binding, &|qualified| {
                    qualified.strip_prefix("builtins.").is_some_and(sequence)
                }),
            }
        }
        "attribute" => {
            let (Some(object), Some(attribute)) =
                (callee.field("object"), callee.field("attribute"))
            else {
                return false;
            };
            let object = unparenthesized(&object);
            object.kind() == "identifier"
                && sequence(&python_identifier(&attribute))
                && table.reference(&object.range()).is_some_and(|reference| {
                    table
                        .resolve(reference, &python_identifier(&object))
                        .is_some_and(|binding| {
                            imports(binding, &|qualified| qualified == "builtins")
                        })
                })
        }
        _ => false,
    }
}

fn extract<D: Doc>(
    table: &ScopeTable,
    node: &Node<'_, D>,
    exports: &mut Exports,
    steps: &mut Steps<'_>,
) -> Result<(), BuildError> {
    steps.take()?;
    let named = || {
        node.named_code()
    };
    match node.kind().as_ref() {
        "parenthesized_expression" => match named().collect::<Vec<_>>().as_slice() {
            [inner] => extract(table, inner, exports, steps)?,
            _ => exports.complete = false,
        },
        "named_expression" => match node.field("value") {
            Some(value) => extract(table, &value, exports, steps)?,
            None => exports.complete = false,
        },
        // A bare tuple (`__all__ = "a", "b"`) is an `expression_list`: Ruff's `Expr::Tuple`.
        "list" | "tuple" | "expression_list" => {
            for element in named() {
                steps.take()?;
                exports.add(&element);
            }
        }
        // Another `__all__` adds nothing this module binds.
        "identifier" if python_identifier(node) == "__all__" => {}
        // A value that holds no names (Ruff's `INVALID_FORMAT`, which lists none either).
        "none" | "true" | "false" | "integer" | "float" | "ellipsis" => {}
        "unary_operator"
            if node.field("argument").is_some_and(|argument| {
                matches!(argument.kind().as_ref(), "integer" | "float")
            }) => {}
        "attribute"
            if node
                .field("attribute")
                .is_some_and(|attribute| python_identifier(&attribute) == "__all__") => {}
        "call"
            if node
                .field("function")
                .is_some_and(|function| builtin_sequence(table, &function)) =>
        {
            let arguments = node
                .field("arguments")
                .map(|arguments| {
                    arguments
                        .named_code()
                        .map(|argument| unparenthesized(&argument))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            match arguments.as_slice() {
                [] => {}
                [display] if matches!(display.kind().as_ref(), "list" | "tuple" | "set") => {
                    let set = display.kind() == "set";
                    if set {
                        for element in display
                            .named_code()
                        {
                            steps.take()?;
                            exports.add(&element);
                        }
                    } else {
                        extract(table, display, exports, steps)?;
                    }
                }
                _ => exports.complete = false,
            }
        }
        _ => exports.complete = false,
    }
    Ok(())
}

/// The name a string entry spells: Ruff's `ExprStringLiteral` value, escapes decoded and an
/// implicit concatenation joined; none for an f-, t- or bytes literal.
fn string_name<D: Doc>(node: &Node<'_, D>) -> Option<String> {
    crate::code_rule_dsl::value::python::string_literal(node)
}
