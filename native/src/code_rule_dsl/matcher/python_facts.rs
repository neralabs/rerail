//! Pure Python semantic projections over a scope table supplied by the caller.
//!
//! This module neither obtains/charges scope or usage tables nor evaluates subrules.
//! The caller must supply the table built for these nodes and the same reading/module
//! configuration used by its semantic consumer. Rows own their facts, not AST nodes.
use crate::code_rule_dsl::{
    identifier::python_identifier,
    language::{
        Family, GrammarDialect,
        adapter::{python::Python, ScopeRules},
    },
    module_path::ModulePath,
    scope::{
        Binding, Form, Import, Lookup, Origin, ScopeTable,
        options::{PythonOptions, python_builtins},
    },
    value::NodeRef,
};
use ast_grep_core::{Doc, Node};
use std::collections::{BTreeMap, HashSet};

pub(crate) type NodeRange = (usize, usize);

pub struct Config<'a> {
    pub module: Option<&'a ModulePath>,
    pub python: &'a PythonOptions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectionError {
    NotPython,
    UnsupportedPythonVersion,
    ReadingMismatch,
    MixedDocuments,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    SyntaxError,
    UnsupportedExpression,
    MissingReference,
    UndecidedReference,
    MissingModule,
    UnqualifiableModule,
    OutOfFileTarget,
    LookupNotProjected,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Fact<T> {
    Known(T),
    /// A real semantic unknown or an input whose syntax is incomplete.
    Unknown(Reason),
    /// This expression has no projected resolution; other structural branches may match.
    NotApplicable(Reason),
    /// The producer cannot project this input; coverage is incomplete.
    Unsupported(Reason),
}

fn unavailable<T>(reason: Reason) -> Fact<T> {
    match reason {
        Reason::UndecidedReference | Reason::SyntaxError => Fact::Unknown(reason),
        Reason::MissingReference
        | Reason::UnsupportedExpression
        | Reason::MissingModule
        | Reason::UnqualifiableModule => Fact::NotApplicable(reason),
        Reason::OutOfFileTarget | Reason::LookupNotProjected => Fact::Unsupported(reason),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub range: NodeRange,
    pub kind: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BindingId {
    pub scope: u32,
    // An index is relative to this row's normalized head name, not scope alone.
    pub index: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BindingProjection {
    pub id: BindingId,
    pub form: Form,
    pub site: Option<Target>,
    pub value: Option<Target>,
    pub annotation: Option<Target>,
    pub name: Option<Target>,
    pub is_global_declaration: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Bindings {
    pub default: Fact<Option<BindingProjection>>,
    pub stop_at_global: Fact<Option<BindingProjection>>,
}
impl Bindings {
    /// `only` needs a separate native lookup and is deliberately not guessed here.
    pub fn get(&self, lookup: Lookup) -> Fact<Option<BindingProjection>> {
        if lookup.only {
            unavailable(Reason::LookupNotProjected)
        } else if lookup.stop_at_global {
            self.stop_at_global.clone()
        } else {
            self.default.clone()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QualifiedOrigin {
    Import,
    Definition,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Qualification {
    Qualified {
        name: String,
        origin: QualifiedOrigin,
    },
    /// No binding shadows this head. Builtin is target-version dependent. A
    /// declared placeholder still differs from no binding for `import: true`.
    Unshadowed {
        name: String,
        builtin: bool,
        declared_placeholder: bool,
    },
    Local(BindingId),
    Deleted(BindingId),
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeFacts {
    pub node_id: usize,
    pub range: NodeRange,
    pub expression_range: Option<NodeRange>,
    pub head_range: Option<NodeRange>,
    pub head_name: Option<String>,
    pub attributes: Vec<String>,
    /// Native `resolves.binding` accepts the identifier, not an attribute chain.
    pub binding_eligible: bool,
    pub qualification: Fact<Qualification>,
    pub head_bindings: Bindings,
}

#[derive(Default, Debug)]
pub struct Stats {
    pub requested: usize,
    pub distinct: usize,
    pub qualified_lookups: usize,
    pub binding_lookups: usize,
}
#[derive(Default, Debug)]
pub struct Batch {
    // Distinct node kinds can occupy the same range; retain every requested ID.
    pub by_range: BTreeMap<NodeRange, Vec<NodeFacts>>,
    pub stats: Stats,
}

/// Project each distinct requested node once. The batch must belong to one file;
/// parenthesized identifier/attribute expressions are supported. Missing/unsupported
/// facts distinguish not-applicable, unsupported and semantic unknown states.
pub fn project<'t, D: Doc>(
    table: &ScopeTable,
    config: Config<'_>,
    nodes: impl IntoIterator<Item = Node<'t, D>>,
) -> Result<Batch, ProjectionError> {
    if table.language != GrammarDialect::Python || table.family != Family::Python {
        return Err(ProjectionError::NotPython);
    }
    if !python_builtins().contains_key(&config.python.minor) {
        return Err(ProjectionError::UnsupportedPythonVersion);
    }
    if table.reading.deferred_annotations != config.python.defers_annotations()
        || (table.reading.stub && table.reading.python_minor != config.python.minor)
        || table.reading.typing != config.python.typing
    {
        return Err(ProjectionError::ReadingMismatch);
    }
    let mut batch = Batch::default();
    let mut document = None;
    let mut seen = HashSet::new();
    for node in nodes {
        batch.stats.requested += 1;
        let identity = node.root() as *const _ as usize;
        if document
            .replace(identity)
            .is_some_and(|prior| prior != identity)
        {
            return Err(ProjectionError::MixedDocuments);
        }
        if !seen.insert(node.node_id()) {
            continue;
        }
        batch.stats.distinct += 1;
        let facts = project_node(table, &config, &node, &mut batch.stats);
        batch.by_range.entry(facts.range).or_default().push(facts);
    }
    Ok(batch)
}

fn range<D: Doc>(node: &Node<'_, D>) -> NodeRange {
    let r = node.range();
    (r.start, r.end)
}
fn unparenthesized<'t, D: Doc>(mut node: Node<'t, D>) -> Result<Node<'t, D>, Reason> {
    loop {
        if node.is_error() || node.is_missing() {
            return Err(Reason::SyntaxError);
        }
        if node.kind() != "parenthesized_expression" {
            return Ok(node);
        }
        let parts: Vec<_> = node
            .children()
            .filter(|n| n.is_named() && n.kind() != "comment")
            .collect();
        match parts.as_slice() {
            [inner] => node = inner.clone(),
            _ => {
                return Err(
                    if node.dfs().any(|n| n.is_error() || n.is_missing())
                        || node.ancestors().any(|n| n.is_error() || n.is_missing())
                    {
                        Reason::SyntaxError
                    } else {
                        Reason::UnsupportedExpression
                    },
                );
            }
        }
    }
}
fn chain<'t, D: Doc>(
    node: &Node<'t, D>,
) -> Result<(Node<'t, D>, Node<'t, D>, Vec<String>), Reason> {
    let expression = unparenthesized(node.clone())?;
    let mut head = expression.clone();
    let mut attributes = Vec::new();
    while head.kind() == "attribute" {
        let attribute = head.field("attribute").ok_or(Reason::SyntaxError)?;
        if attribute.kind() != "identifier" || attribute.is_error() || attribute.is_missing() {
            return Err(Reason::SyntaxError);
        }
        attributes.push(python_identifier(&attribute).into_owned());
        head = unparenthesized(head.field("object").ok_or(Reason::SyntaxError)?)?;
    }
    if head.kind() != "identifier" {
        // Only a well-formed unsupported expression is a definite non-qualification.
        // Inspect this requested expression, not its enclosing document.
        if node.dfs().any(|n| n.is_error() || n.is_missing())
            || head.ancestors().any(|n| n.is_error() || n.is_missing())
        {
            return Err(Reason::SyntaxError);
        }
        return Err(Reason::UnsupportedExpression);
    }
    if head.ancestors().any(|n| n.is_error() || n.is_missing()) {
        return Err(Reason::SyntaxError);
    }
    attributes.reverse();
    Ok((expression, head, attributes))
}
/// Normalized, scope-free shape of an identifier/attribute qualification demand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QualificationChain {
    pub head: String,
    pub attributes: Vec<String>,
}

/// Parse and normalize once for callers that also need coarse import screening.
/// This acquires no scopes and evaluates no semantic predicate.
pub fn qualification_chain<D: Doc>(node: &Node<'_, D>) -> Fact<QualificationChain> {
    match chain(node) {
        Ok((_, head, attributes)) => Fact::Known(QualificationChain {
            head: python_identifier(&head).into_owned(),
            attributes,
        }),
        Err(reason) => unavailable(reason),
    }
}

/// Well-formed call/subscript/other non-name heads cannot satisfy explicit native
/// import/global qualification. Incomplete syntax stays unknown.
pub fn qualification_eligibility<D: Doc>(node: &Node<'_, D>) -> Fact<()> {
    match qualification_chain(node) {
        Fact::Known(_) => Fact::Known(()),
        Fact::NotApplicable(reason) => Fact::NotApplicable(reason),
        Fact::Unknown(reason) => Fact::Unknown(reason),
        Fact::Unsupported(reason) => Fact::Unsupported(reason),
    }
}

fn unknown_node<D: Doc>(node: &Node<'_, D>, reason: Reason) -> NodeFacts {
    NodeFacts {
        node_id: node.node_id(),
        range: range(node),
        expression_range: None,
        head_range: None,
        head_name: None,
        attributes: Vec::new(),
        binding_eligible: false,
        qualification: unavailable(reason),
        head_bindings: Bindings {
            default: unavailable(reason),
            stop_at_global: unavailable(reason),
        },
    }
}
fn project_node<D: Doc>(
    table: &ScopeTable,
    config: &Config<'_>,
    node: &Node<'_, D>,
    stats: &mut Stats,
) -> NodeFacts {
    let (expression, head, attributes) = match chain(node) {
        Ok(chain) => chain,
        Err(reason) => return unknown_node(node, reason),
    };
    let name = python_identifier(&head).into_owned();
    let mut output = unknown_node(node, Reason::MissingReference);
    output.expression_range = Some(range(&expression));
    output.head_range = Some(range(&head));
    output.head_name = Some(name.clone());
    output.binding_eligible = attributes.is_empty();
    output.attributes = attributes;
    let Some(reference) = table.reference(&head.range()) else {
        return output;
    };
    if <Python as ScopeRules>::undecided(table, reference) {
        output.qualification = unavailable(Reason::UndecidedReference);
        output.head_bindings = Bindings {
            default: unavailable(Reason::UndecidedReference),
            stop_at_global: unavailable(Reason::UndecidedReference),
        };
        return output;
    }
    stats.qualified_lookups += 1;
    let resolved = table.resolve_python_index(reference, &name);
    output.qualification = qualification(table, config, &name, &output.attributes, resolved);
    let root = range(&node.root().root());
    // The default binding reuses the qualification lookup; only this variant looks up again.
    stats.binding_lookups += 1;
    output.head_bindings.default = binding(table, resolved, &name, root);
    let stopped = table.python_index_by(
        reference,
        &name,
        Lookup {
            stop_at_global: true,
            only: false,
        },
    );
    output.head_bindings.stop_at_global = binding(table, stopped, &name, root);
    output
}
fn id((scope, index): (u32, usize)) -> BindingId {
    BindingId { scope, index }
}
fn qualification(
    table: &ScopeTable,
    config: &Config<'_>,
    name: &str,
    tail: &[String],
    resolved: Option<(u32, usize)>,
) -> Fact<Qualification> {
    let Some(found) = resolved else {
        return Fact::Known(Qualification::Unshadowed {
            name: name.into(),
            builtin: config.python.declares(name),
            declared_placeholder: false,
        });
    };
    let binding = table.binding_at(found, name);
    if !binding.shadows() {
        return Fact::Known(Qualification::Unshadowed {
            name: name.into(),
            builtin: config.python.declares(name),
            declared_placeholder: true,
        });
    }
    if binding.is_deletion() {
        return Fact::Known(Qualification::Deleted(id(found)));
    }
    let (qualified, origin) = match &binding.import {
        Some(Import::Qualified(import)) => {
            let mut qualified = import.to_string();
            for segment in tail {
                qualified.push('.');
                qualified.push_str(segment);
            }
            (Some(qualified), QualifiedOrigin::Import)
        }
        Some(Import::Relative { level, rest }) => {
            let Some(module) = config.module else {
                return unavailable(Reason::MissingModule);
            };
            (module.relative(*level, rest, tail), QualifiedOrigin::Import)
        }
        Some(Import::Definition) => {
            let Some(module) = config.module else {
                return unavailable(Reason::MissingModule);
            };
            (module.definition(name, tail), QualifiedOrigin::Definition)
        }
        None => return Fact::Known(Qualification::Local(id(found))),
    };
    match qualified {
        Some(name) => Fact::Known(Qualification::Qualified { name, origin }),
        None => unavailable(Reason::UnqualifiableModule),
    }
}
fn target(node: Option<NodeRef>, root: NodeRange) -> Result<Option<Target>, Reason> {
    node.map(|node| {
        let range = (node.start as usize, node.end as usize);
        if range.0 < root.0 || range.1 > root.1 || range.0 > range.1 {
            return Err(Reason::OutOfFileTarget);
        }
        Ok(Target {
            range,
            kind: node.kind,
        })
    })
    .transpose()
}
fn binding(
    table: &ScopeTable,
    found: Option<(u32, usize)>,
    name: &str,
    root: NodeRange,
) -> Fact<Option<BindingProjection>> {
    let Some(found) = found else {
        return Fact::Known(None);
    };
    let bound: &Binding = table.binding_at(found, name);
    if !bound.shadows() || bound.is_deletion() {
        return Fact::Known(None);
    }
    let targets = (|| {
        Ok::<_, Reason>((
            target(bound.site.node, root)?,
            target(bound.site.value, root)?,
            target(bound.site.annotation, root)?,
            target(bound.name, root)?,
        ))
    })();
    match targets {
        Ok((site, value, annotation, name)) => Fact::Known(Some(BindingProjection {
            id: id(found),
            form: bound.site.form,
            site,
            value,
            annotation,
            name,
            is_global_declaration: bound.origin == Origin::Global,
        })),
        Err(reason) => unavailable(reason),
    }
}

// tests: a test the engine hosts (it reads above this crate; tools/cratify/test-hosts.tsv)
