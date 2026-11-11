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
