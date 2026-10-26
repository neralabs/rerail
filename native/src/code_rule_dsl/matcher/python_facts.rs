//! Pure Python semantic projections over a scope table supplied by the caller.
//!
//! This module neither obtains/charges scope or usage tables nor evaluates subrules.
//! The caller must supply the table built for these nodes and the same reading/module
//! configuration used by its semantic consumer. Rows own their facts, not AST nodes.
use crate::code_rule_dsl::{
    identifier::python_identifier,
    language::{
        Family, GrammarDialect,
        adapter::{Python, ScopeRules},
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

pub(crate) struct Config<'a> {
    pub module: Option<&'a ModulePath>,
    pub python: &'a PythonOptions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProjectionError {
    NotPython,
    UnsupportedPythonVersion,
    ReadingMismatch,
    MixedDocuments,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reason {
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
pub(crate) enum Fact<T> {
    Known(T),
    /// A real semantic unknown, currently an undecided reference.
    Unknown(Reason),
    /// This expression has no projected resolution; other structural branches may match.
    NotApplicable(Reason),
    /// The producer cannot safely project this input; consumers must fall back.
    Unsupported(Reason),
}

fn unavailable<T>(reason: Reason) -> Fact<T> {
    match reason {
        Reason::UndecidedReference => Fact::Unknown(reason),
        Reason::MissingReference
        | Reason::UnsupportedExpression
        | Reason::MissingModule
        | Reason::UnqualifiableModule => Fact::NotApplicable(reason),
        Reason::SyntaxError | Reason::OutOfFileTarget | Reason::LookupNotProjected => {
            Fact::Unsupported(reason)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Target {
    pub range: NodeRange,
    pub kind: u16,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BindingId {
    pub scope: u32,
    // An index is relative to this row's normalized head name, not scope alone.
    pub index: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BindingProjection {
    pub id: BindingId,
    pub form: Form,
    pub site: Option<Target>,
    pub value: Option<Target>,
    pub annotation: Option<Target>,
    pub name: Option<Target>,
    pub is_global_declaration: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Bindings {
    pub default: Fact<Option<BindingProjection>>,
    pub stop_at_global: Fact<Option<BindingProjection>>,
}
impl Bindings {
    /// `only` needs a separate native lookup and is deliberately not guessed here.
    pub(crate) fn get(&self, lookup: Lookup) -> Fact<Option<BindingProjection>> {
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
pub(crate) enum QualifiedOrigin {
    Import,
    Definition,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Qualification {
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
pub(crate) struct NodeFacts {
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
pub(crate) struct Stats {
    pub requested: usize,
    pub distinct: usize,
    pub qualified_lookups: usize,
    pub binding_lookups: usize,
}
#[derive(Default, Debug)]
pub(crate) struct Batch {
    // Distinct node kinds can occupy the same range; retain every requested ID.
    pub by_range: BTreeMap<NodeRange, Vec<NodeFacts>>,
    pub stats: Stats,
}

/// Project each distinct requested node once. The batch must belong to one file;
/// parenthesized identifier/attribute expressions are supported. Missing/unsupported
/// facts distinguish not-applicable, unsupported and semantic unknown states.
pub(crate) fn project<'t, D: Doc>(
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
            _ => return Err(Reason::UnsupportedExpression),
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
        return Err(Reason::UnsupportedExpression);
    }
    if head.ancestors().any(|n| n.is_error() || n.is_missing()) {
        return Err(Reason::SyntaxError);
    }
    attributes.reverse();
    Ok((expression, head, attributes))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::scope::options::DEFAULT_PYTHON_MINOR;
    use ast_grep_language::LanguageExt;

    fn batch(source: &str, module: Option<&ModulePath>, texts: &[&str]) -> Batch {
        let doc = GrammarDialect::Python.ast_grep(source);
        let table =
            ScopeTable::build(doc.root(), GrammarDialect::Python, usize::MAX, &|| false).unwrap();
        let python = PythonOptions::new(DEFAULT_PYTHON_MINOR);
        let nodes: Vec<_> = doc
            .root()
            .dfs()
            .filter(|n| {
                matches!(
                    n.kind().as_ref(),
                    "identifier" | "attribute" | "parenthesized_expression" | "call"
                ) && texts.contains(&n.text().as_ref())
            })
            .collect();
        project(
            &table,
            Config {
                module,
                python: &python,
            },
            nodes,
        )
        .unwrap()
    }
    fn at<'a>(facts: &'a Batch, start: usize, text: &str) -> &'a NodeFacts {
        &facts.by_range[&(start, start + text.len())][0]
    }
    fn qualified(row: &NodeFacts) -> &str {
        let Fact::Known(Qualification::Qualified { name, .. }) = &row.qualification else {
            panic!("expected qualified name: {:?}", row.qualification)
        };
        name
    }
    fn bound(row: &NodeFacts) -> &BindingProjection {
        let Fact::Known(Some(binding)) = &row.head_bindings.default else {
            panic!("expected binding: {:?}", row.head_bindings.default)
        };
        binding
    }
    fn text<'a>(source: &'a str, target: Option<Target>) -> &'a str {
        let (start, end) = target.expect("binding target").range;
        &source[start..end]
    }

    #[test]
    fn aliases_shadowing_builtins_and_normalized_attributes() {
        let source = "import os as platform\nfrom pathlib import Path as P\nplatform.path.join('a', 'b')\nP('x')\nlen([])\ndef f(platform):\n    platform.path.join('a', 'b')\n";
        let facts = batch(source, None, &["platform.path.join", "P", "len"]);
        assert_eq!(
            qualified(at(
                &facts,
                source.find("platform.path.join").unwrap(),
                "platform.path.join"
            )),
            "os.path.join"
        );
        assert_eq!(
            qualified(at(&facts, source.rfind("P(").unwrap(), "P")),
            "pathlib.Path"
        );
        let last = at(
            &facts,
            source.rfind("platform.path.join").unwrap(),
            "platform.path.join",
        );
        assert!(matches!(
            last.qualification,
            Fact::Known(Qualification::Local(_))
        ));
        assert!(!last.binding_eligible);
        let len = at(&facts, source.find("len([])").unwrap(), "len");
        assert!(
            matches!(&len.qualification, Fact::Known(Qualification::Unshadowed { name, builtin: true, declared_placeholder: false }) if name == "len")
        );
        let source = "import pkg\n(pkg.K)\n";
        let facts = batch(source, None, &["(pkg.K)"]);
        assert_eq!(
            qualified(at(&facts, source.find("(pkg.K)").unwrap(), "(pkg.K)")),
            "pkg.K"
        );
    }

    #[test]
    fn relative_imports_and_definitions_use_explicit_module_only() {
        let source = "from .helpers import wait as w\ndef run():\n    pass\nw()\nrun()\n";
        let module = ModulePath::dotted("pkg.worker").unwrap();
        let facts = batch(source, Some(&module), &["w", "run"]);
        assert_eq!(
            qualified(at(&facts, source.rfind("w()").unwrap(), "w")),
            "pkg.helpers.wait"
        );
        let run = at(&facts, source.rfind("run()").unwrap(), "run");
        assert_eq!(qualified(run), "pkg.worker.run");
        assert!(matches!(
            run.qualification,
            Fact::Known(Qualification::Qualified {
                origin: QualifiedOrigin::Definition,
                ..
            })
        ));
        let without = batch(source, None, &["w"]);
        assert!(matches!(
            at(&without, source.rfind("w()").unwrap(), "w").qualification,
            Fact::NotApplicable(Reason::MissingModule)
        ));
    }

    #[test]
    fn binding_targets_preserve_assignment_annotation_unpacking_and_with_site() {
        let source = "from pathlib import Path\np: Path = Path('first')\na, b = (Path('a'), Path('b'))\nwith open('f') as handle:\n    handle.read()\np\na\nb\n";
        let facts = batch(source, None, &["p", "a", "b", "handle"]);
        let p = bound(at(&facts, source.rfind("\np\n").unwrap() + 1, "p"));
        assert_eq!(text(source, p.value), "Path('first')");
        assert_eq!(text(source, p.annotation), "Path");
        assert_eq!(text(source, p.site), "p: Path = Path('first')");
        for (name, expected) in [("a", "Path('a')"), ("b", "Path('b')")] {
            let start = source.rfind(&format!("\n{name}\n")).unwrap() + 1;
            assert_eq!(text(source, bound(at(&facts, start, name)).value), expected);
        }
        let handle = bound(at(&facts, source.find("handle.read").unwrap(), "handle"));
        assert_eq!(handle.form, Form::With);
        assert_eq!(text(source, handle.site), "open('f') as handle");
        assert!(handle.value.is_none());
    }

    #[test]
    fn default_and_stop_global_remain_distinct_and_duplicates_are_deduplicated() {
        let source = "x = 1\ndef f():\n    global x\n    return x\n";
        let doc = GrammarDialect::Python.ast_grep(source);
        let table =
            ScopeTable::build(doc.root(), GrammarDialect::Python, usize::MAX, &|| false).unwrap();
        let node = doc
            .root()
            .dfs()
            .find(|n| n.kind() == "identifier" && n.range().start == source.rfind('x').unwrap())
            .unwrap();
        let python = PythonOptions::new(DEFAULT_PYTHON_MINOR);
        let facts = project(
            &table,
            Config {
                module: None,
                python: &python,
            },
            [node.clone(), node],
        )
        .unwrap();
        assert_eq!(
            (
                facts.stats.requested,
                facts.stats.distinct,
                facts.stats.qualified_lookups,
                facts.stats.binding_lookups
            ),
            (2, 1, 1, 1)
        );
        let row = at(&facts, source.rfind('x').unwrap(), "x");
        assert_eq!(text(source, bound(row).value), "1");
        let Fact::Known(Some(stopped)) = &row.head_bindings.stop_at_global else {
            panic!("global placeholder")
        };
        assert!(stopped.is_global_declaration);
        assert!(stopped.value.is_none());
        assert_ne!(stopped.id, bound(row).id);
        assert!(matches!(
            row.head_bindings.get(Lookup {
                only: true,
                stop_at_global: false
            }),
            Fact::Unsupported(Reason::LookupNotProjected)
        ));
    }

    #[test]
    fn unsupported_callee_is_not_a_semantic_unknown_and_bad_configuration_is_rejected() {
        let source = "from pathlib import Path\nPath('f').open()\n";
        let facts = batch(source, None, &["Path('f').open", "Path"]);
        assert!(matches!(
            at(
                &facts,
                source.find("Path('f').open").unwrap(),
                "Path('f').open"
            )
            .qualification,
            Fact::NotApplicable(Reason::UnsupportedExpression)
        ));
        assert_eq!(
            qualified(at(&facts, source.find("Path('f')").unwrap(), "Path")),
            "pathlib.Path"
        );
        let doc = GrammarDialect::Python.ast_grep("x\n");
        let table =
            ScopeTable::build(doc.root(), GrammarDialect::Python, usize::MAX, &|| false).unwrap();
        let bad = PythonOptions::new(u8::MAX);
        assert_eq!(
            project(
                &table,
                Config {
                    module: None,
                    python: &bad
                },
                [doc.root()]
            )
            .unwrap_err(),
            ProjectionError::UnsupportedPythonVersion
        );
        let incompatible = PythonOptions::new(14);
        assert_eq!(
            project(
                &table,
                Config {
                    module: None,
                    python: &incompatible
                },
                [doc.root()]
            )
            .unwrap_err(),
            ProjectionError::ReadingMismatch
        );
        let other = GrammarDialect::Python.ast_grep("y\n");
        let python = PythonOptions::new(DEFAULT_PYTHON_MINOR);
        assert_eq!(
            project(
                &table,
                Config {
                    module: None,
                    python: &python
                },
                [doc.root(), other.root()]
            )
            .unwrap_err(),
            ProjectionError::MixedDocuments
        );
    }
}
