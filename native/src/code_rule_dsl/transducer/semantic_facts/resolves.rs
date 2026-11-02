//! `resolves` (Python) as a fact producer (O24; decision 0011: a semantic leaf is a node label
//! from its producer's rows, not an operation inside a program).
//!
//! One pass per document and table reading emits every row any form of the predicate reads,
//! whatever clauses read them: the reads (a name, or an attribute chain whose head is a name)
//! with how their head is bound and the qualified name an import gives them; the names import
//! statements import; the module `__all__` entries; the module's top-level imports in Ruff's
//! visiting order. A clause's form is then a filter over the rows (`label`), what its SQL puts in
//! a `WHERE`: the listed names, `prefix`, the target version's builtins. W's matcher
//! (`Resolves::holds`) is the oracle (`gate`).

// Read only by the gates until a lowering reads the labels (O41).
#![allow(dead_code)]

use super::scopes::Scopes;
use crate::code_rule_dsl::identifier::name_in;
use crate::code_rule_dsl::language::adapter::{Python, ScopeRules};
use crate::code_rule_dsl::language::{GrammarDialect, kind_ids};
use crate::code_rule_dsl::matcher::{Resolves, Unresolved, imported_name, node_at};
use crate::code_rule_dsl::module_path::ModulePath;
use crate::code_rule_dsl::scope::Import;
use crate::code_rule_dsl::scope::options::{DEFAULT_PYTHON_MINOR, PythonOptions};
use crate::code_rule_dsl::tape::TapeDoc;
use ast_grep_core::{AstGrep as Root, Node};
use std::collections::BTreeSet;

/// A read: a name, or an attribute chain whose head is a name, that the table holds a reference
/// for. A parenthesized expression around one carries its row too, with its own node.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Read {
    /// The node (tape ordinal).
    pub(crate) node: u32,
    /// The kind of the expression inside any parentheses: `identifier` or `attribute`.
    pub(crate) kind: u16,
    /// The head's name, as Python compares names (NFKC).
    pub(crate) head: Box<str>,
    /// Some binding resolves the head.
    pub(crate) bound: bool,
    /// ... and it shadows (it is no `global` declaration's placeholder).
    pub(crate) shadows: bool,
    /// The head's binding is an import (or a definition the module path qualifies), and this is
    /// its qualified name joined with the chain's attributes.
    pub(crate) qualified: Option<Box<str>>,
    /// A name's facts for Ruff's F821 and F405 (`unresolved`); `None` for an attribute chain.
    pub(crate) unresolved: Option<UnresolvedFacts>,
    /// A name that is its function's first read of the binding the function's later binding of
    /// it shadows (Ruff's F823): whether that binding is the builtin.
    pub(crate) before_local: Option<bool>,
}

/// What decides whether a name no binding resolves is unresolved (`ScopeRules::unresolved`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct UnresolvedFacts {
    /// A function's bare annotation the read stops at, and whether a nested `nonlocal` may bind it.
    pub(crate) annotation: Option<bool>,
    /// The binding that resolves it is a deletion.
    pub(crate) deleted: bool,
    /// An `except` name its handler unbound.
    pub(crate) unbound_exception: bool,
    /// A name the interpreter defines implicitly there (`__module__`, `__file__`, ...).
    pub(crate) implicit: bool,
    /// A star import is visible from the read.
    pub(crate) star_visible: bool,
}

/// A `dotted_name` of an import statement and the qualified name it imports, read without
/// (`strict`) and with (`text`) a relative module's text (`relative: "text"`).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Imported {
    pub(crate) node: u32,
    pub(crate) strict: Option<Box<str>>,
    pub(crate) text: Option<Box<str>>,
}

/// A string entry of a module `__all__` definition (or parentheses around one).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Export {
    pub(crate) node: u32,
    /// The name it exports.
    pub(crate) name: Box<str>,
    /// The module binds it.
    pub(crate) bound: bool,
}

/// One document's rows.
#[derive(Debug, Default)]
pub(crate) struct ResolvesRows {
    pub(crate) reads: Vec<Read>,
    pub(crate) imported: Vec<Imported>,
    pub(crate) exports: Vec<Export>,
    /// The module holds star imports.
    pub(crate) module_stars: bool,
    /// Each absolute import's top-level module, at the function depth and offset Ruff visits it.
    pub(crate) module_imports: Vec<(Box<str>, u32, usize)>,
}

impl ResolvesRows {
    pub(crate) fn len(&self) -> usize {
        self.reads.len() + self.imported.len() + self.exports.len() + self.module_imports.len()
    }
}

/// Python's kinds the rows read.
pub(crate) struct Kinds {
    identifier: BTreeSet<u16>,
    attribute: BTreeSet<u16>,
    paren: BTreeSet<u16>,
    dotted: BTreeSet<u16>,
    strings: BTreeSet<u16>,
}

impl Kinds {
    pub(crate) fn python() -> Self {
        let python = GrammarDialect::Python;
        Self {
            identifier: kind_ids(python, Python::REFERENCE_KINDS),
            attribute: kind_ids(python, &["attribute"]),
            paren: kind_ids(python, &["parenthesized_expression"]),
            dotted: kind_ids(python, &["dotted_name"]),
            strings: kind_ids(python, Python::ANNOTATION_STRING_KINDS),
        }
    }
}

/// The expression inside any parentheses around `node` (W's `unparenthesized`).
fn unparenthesized<'r>(kinds: &Kinds, mut node: Node<'r, TapeDoc>) -> Node<'r, TapeDoc> {
    while kinds.paren.contains(&node.kind_id()) {
        let mut inner = node
            .children()
            .filter(|child| child.is_named() && child.kind() != "comment");
        let (Some(expression), None) = (inner.next(), inner.next()) else {
            break;
        };
        drop(inner);
        node = expression;
    }
    node
}

/// The rows of `root`'s document, whose table and usage index are `scopes`. `module` is the
/// module path the table was built with.
pub(crate) fn rows(
    root: &Root<TapeDoc>,
    scopes: &Scopes,
    module: Option<&ModulePath>,
    kinds: &Kinds,
) -> ResolvesRows {
    let (table, usage) = (&scopes.table, &scopes.usage);
    let tape = root.root().get_doc();
    let init = module.is_some_and(|module| module.last() == "__init__");
    let mut rows = ResolvesRows {
        module_stars: !table.scopes[0].stars.is_empty(),
        module_imports: table
            .module_imports
            .iter()
            .map(|import| (import.module.clone(), import.depth, import.offset))
            .collect(),
        ..ResolvesRows::default()
    };
    for ordinal in 0..tape.transducer_len() as u32 {
        let kind = tape.transducer_kind(ordinal);
        if kinds.dotted.contains(&kind) {
            let node = node_at(root, ordinal);
            let strict = imported_name(&node, false).map(Box::from);
            let text = imported_name(&node, true).map(Box::from);
            if strict.is_some() || text.is_some() {
                rows.imported.push(Imported {
                    node: ordinal,
                    strict,
                    text,
                });
            }
            continue;
        }
        let candidate = kinds.identifier.contains(&kind)
            || kinds.attribute.contains(&kind)
            || kinds.paren.contains(&kind)
            || kinds.strings.contains(&kind);
        if !candidate {
            continue;
        }
        let expression = unparenthesized(kinds, node_at(root, ordinal));
        let kind = expression.kind_id();
        if kinds.strings.contains(&kind) {
            let range = expression.range();
            let listed = table.dunder_all.iter().any(|statement| {
                (statement.start as usize) <= range.start && range.end <= statement.end as usize
            });
            if let Some(name) = usage.export_at(range.start, range.end).filter(|_| listed) {
                rows.exports.push(Export {
                    node: ordinal,
                    name: name.into(),
                    bound: table.scopes[0].bindings.contains_key(name),
                });
            }
            continue;
        }
        let attribute = kinds.attribute.contains(&kind);
        if !attribute && !kinds.identifier.contains(&kind) {
            continue;
        }
        // The chain's head and its attributes, as W's `import_chain` reads them.
        let mut tail = Vec::new();
        let mut head = expression.clone();
        let mut broken = false;
        while kinds.attribute.contains(&head.kind_id()) {
            let (Some(name), Some(object)) = (head.field("attribute"), head.field("object")) else {
                broken = true;
                break;
            };
            tail.push(name_in(GrammarDialect::Python, name.text()).into_owned());
            head = unparenthesized(kinds, object);
        }
        if broken || !kinds.identifier.contains(&head.kind_id()) {
            continue;
        }
        tail.reverse();
        let Some(reference) = table.reference(&head.range()) else {
            continue;
        };
        let name = name_in(GrammarDialect::Python, head.text());
        let resolved = <Python as ScopeRules>::resolve(table, reference, &name);
        let qualified = resolved
            .and_then(|binding| binding.import.as_ref())
            .and_then(|import| match import {
                Import::Qualified(qualified) => {
                    let mut full = qualified.to_string();
                    for part in &tail {
                        full.push('.');
                        full.push_str(part);
                    }
                    Some(full)
                }
                Import::Relative { level, rest } => module?.relative(*level, rest, &tail),
                Import::Definition => module?.definition(&name, &tail),
            })
            .map(Box::from);
        let unresolved = (!attribute).then(|| UnresolvedFacts {
            annotation: <Python as ScopeRules>::unbound_annotation(table, reference, &name)
                .map(|annotation| table.binding_at(annotation, &name).rebound),
            deleted: resolved.is_some_and(|binding| binding.is_deletion()),
            unbound_exception: table.unbound_exception(reference, &name),
            implicit: table.implicit(reference, &name, init),
            star_visible: table.star_visible(reference),
        });
        rows.reads.push(Read {
            node: ordinal,
            kind,
            head: name.as_ref().into(),
            bound: resolved.is_some(),
            shadows: resolved.is_some_and(|binding| binding.shadows()),
            qualified,
            unresolved,
            before_local: (!attribute)
                .then(|| usage.before_local(reference.start))
                .flatten(),
        });
    }
    rows
}

/// A form this producer's rows do not answer, named.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Refused(pub(crate) &'static str);

/// Whether the rows answer `form`: Python's, without the ECMAScript and PHP options.
pub(crate) fn admits(form: &Resolves) -> Result<(), Refused> {
    if form.language != GrammarDialect::Python {
        return Err(Refused("not Python"));
    }
    if form.item {
        return Err(Refused("eslintDirective item"));
    }
    if form.this_global {
        return Err(Refused("this"));
    }
    if form.directives || form.options.is_some() || form.script_declarations {
        return Err(Refused("languageOptions"));
    }
    Ok(())
}

/// The query's Python options, or Ruff's default target (W's `python_options`).
fn python_options(form: &Resolves) -> PythonOptions {
    PythonOptions::new(
        form.python
            .as_ref()
            .map_or(DEFAULT_PYTHON_MINOR, |python| python.minor),
    )
}

/// Whether `name` is listed, or with `prefix` lies under a listed name, or is a typing module's
/// member whose `typing.<member>` is (W's `listed_typing`).
fn listed(form: &Resolves, names: &BTreeSet<String>, name: &str) -> bool {
    if names.contains(name)
        || (form.prefix
            && name
                .match_indices('.')
                .any(|(dot, _)| names.contains(&name[..dot])))
    {
        return true;
    }
    let Some(typing) = form.python.as_ref().and_then(|python| python.typing) else {
        return false;
    };
    let parts = name.split('.').map(str::to_owned).collect::<Vec<_>>();
    parts.last().is_some_and(|member| {
        names.contains(&format!("typing.{member}")) && typing.module_member(&parts, member)
    })
}

/// A read's verdict under `form` (W's `holds`, from the reference kinds on).
fn read_holds(form: &Resolves, read: &Read, python: &PythonOptions) -> bool {
    let attribute = form.attribute_kinds.contains(&read.kind);
    if form.before_local {
        return form.reference_kinds.contains(&read.kind)
            && !attribute
            && read
                .before_local
                .is_some_and(|builtin| !builtin || python.declares(&read.head));
    }
    if !form.reference_kinds.contains(&read.kind) {
        return false;
    }
    if let Some(mode) = form.unresolved {
        let Some(facts) = read.unresolved.filter(|_| !attribute) else {
            return false;
        };
        let unbound = mode == Unresolved::Unbound;
        if let Some(rebound) = facts.annotation {
            return unbound && !rebound;
        }
        if read.bound {
            return facts.deleted && unbound;
        }
        if facts.unbound_exception && !python.declares(&read.head) {
            return unbound;
        }
        return !python.declares(&read.head)
            && !facts.implicit
            && facts.star_visible == (mode == Unresolved::Star);
    }
    let chain = !form.imports.is_empty() || form.import_any;
    let name = read.head.as_ref();
    let global = (!form.globals.is_empty() || form.global_any)
        && !attribute
        && (form.global_any || form.globals.contains(name))
        && (!form.declared
            || form
                .python
                .as_ref()
                .is_none_or(|python| python.declares(name)));
    let unshadowed = !read.bound || !read.shadows;
    let is_global = unshadowed
        && (!form.declared
            || form
                .python
                .as_ref()
                .is_none_or(|python| python.declares(name)));
    let is_import = if form.import_any && !read.bound {
        python.declares(name)
    } else {
        read.qualified
            .as_deref()
            .is_some_and(|full| form.import_any || listed(form, &form.imports, full))
    };
    (global && is_global) || (chain && is_import)
}

/// Ruff's visiting depth of each node (W's `is_seen_before`): the function bodies around it, or
/// `u32::MAX` inside a lambda body, a `type` alias's value or a type parameter, which Ruff visits
/// after every function body. In node order, each from its parent's.
fn visiting_depths(root: &Root<TapeDoc>) -> Vec<u32> {
    let tape = root.root().get_doc();
    let mut depth = vec![0u32; tape.transducer_len()];
    let mut late = vec![false; tape.transducer_len()];
    for ordinal in 1..tape.transducer_len() as u32 {
        let node = node_at(root, ordinal);
        let Some(parent) = node.parent() else {
            continue;
        };
        let at = parent.node_id();
        let holds = |field: &str| {
            parent
                .field(field)
                .is_some_and(|part| part.range() == node.range())
        };
        let (mut own, mut own_late) = (depth[at], late[at]);
        match parent.kind().as_ref() {
            "function_definition" if holds("body") => own += 1,
            "lambda" if holds("body") => own_late = true,
            "type_alias_statement" if holds("right") => own_late = true,
            "type_parameter" => own_late = true,
            _ => {}
        }
        depth[ordinal as usize] = own;
        late[ordinal as usize] = own_late;
    }
    depth
        .into_iter()
        .zip(late)
        .map(|(depth, late)| if late { u32::MAX } else { depth })
        .collect()
}

/// The nodes `form` holds at in `rows`' document: its label. `root` is the document's tape,
/// which `seenBefore` reads every node of.
pub(crate) fn label(
    form: &Resolves,
    rows: &ResolvesRows,
    root: &Root<TapeDoc>,
) -> Result<BTreeSet<u32>, Refused> {
    admits(form)?;
    if form.seen_before {
        let first = rows
            .module_imports
            .iter()
            .filter(|(module, _, _)| form.imported.contains(module.as_ref()))
            .map(|(_, depth, offset)| (*depth, *offset))
            .min();
        let Some(first) = first else {
            return Ok(BTreeSet::new());
        };
        let tape = root.root().get_doc();
        return Ok(visiting_depths(root)
            .into_iter()
            .enumerate()
            .filter(|&(ordinal, depth)| first < (depth, tape.transducer_span(ordinal as u32).start))
            .map(|(ordinal, _)| ordinal as u32)
            .collect());
    }
    let python = python_options(form);
    let mut label = BTreeSet::new();
    if !form.dotted_kinds.is_empty() {
        label.extend(
            rows.imported
                .iter()
                .filter(|row| {
                    let name = if form.relative_text {
                        &row.text
                    } else {
                        &row.strict
                    };
                    name.as_deref()
                        .is_some_and(|name| listed(form, &form.imported, name))
                })
                .map(|row| row.node),
        );
    }
    if let Some(mode) = form.unresolved.filter(|_| !form.before_local) {
        label.extend(
            rows.exports
                .iter()
                .filter(|export| {
                    !export.bound
                        && !python.declares(&export.name)
                        && rows.module_stars != (mode == Unresolved::Unbound)
                })
                .map(|export| export.node),
        );
    }
    label.extend(
        rows.reads
            .iter()
            .filter(|read| read_holds(form, read, &python))
            .map(|read| read.node),
    );
    Ok(label)
}
