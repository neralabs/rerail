//! Shared Python syntax facts. Every document is walked once, independently of rules.
use ast_grep_core::{Doc, Node};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct NodeRef {
    pub doc: u32,
    pub node_id: usize,
    pub start: usize,
    pub end: usize,
}
impl NodeRef {
    pub(crate) fn of<D: Doc>(doc: u32, node: &Node<'_, D>) -> Self {
        let range = node.range();
        Self {
            doc,
            node_id: node.node_id(),
            start: range.start,
            end: range.end,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ContextKind {
    Module,
    SyncFunction,
    AsyncFunction,
    Class,
    Lambda,
}
#[derive(Debug)]
pub(crate) struct ExecutionContext {
    pub kind: ContextKind,
    pub parent: Option<u32>,
    pub owner: Option<NodeRef>,
    pub body: NodeRef,
}
#[derive(Debug)]
pub(crate) struct CallSite {
    pub call: NodeRef,
    /// The original function field, including all parentheses.
    pub callee: NodeRef,
    /// The unparenthesized function expression; the catalog reports this leaf.
    pub report_node: NodeRef,
    /// Full identifier/attribute callee for semantic projection, never just its head.
    /// Other callable expressions (for example f()()) retain a row with None here.
    pub ref_node: Option<NodeRef>,
    pub context: u32,
    pub arguments: Range<usize>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Spread {
    None,
    Positional,
    Keywords,
}
#[derive(Debug)]
pub(crate) struct Argument {
    pub call: usize,
    pub node: NodeRef,
    pub value: NodeRef,
    /// Source order among named, non-comment arguments.
    pub ordinal: usize,
    pub keyword: Option<String>,
    pub spread: Spread,
    /// Syntactic slot before the first positional spread. Keywords and ** do not
    /// consume positional slots; values after * have no definite syntactic slot.
    pub positional_index: Option<usize>,
    pub after_positional_spread: bool,
}
#[derive(Debug)]
pub(crate) struct Attribute {
    pub node: NodeRef,
    pub object: NodeRef,
    pub name_node: NodeRef,
    pub name: String,
}
#[derive(Debug)]
pub(crate) struct Parenthesis {
    pub node: NodeRef,
    pub child: NodeRef,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SequenceKind {
    Tuple,
    List,
    Set,
    Pattern,
    Expressions,
}
#[derive(Debug)]
pub(crate) struct Sequence {
    pub node: NodeRef,
    pub kind: SequenceKind,
    pub children: Range<usize>,
}
#[derive(Debug)]
pub(crate) struct SequenceChild {
    pub node: NodeRef,
    pub named_index: usize,
    pub spread: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AssignmentKind {
    Assignment,
    Augmented,
    NamedExpression,
}
#[derive(Debug)]
pub(crate) struct Assignment {
    pub node: NodeRef,
    pub left: NodeRef,
    pub right: Option<NodeRef>,
    pub annotation: Option<NodeRef>,
    pub kind: AssignmentKind,
    pub context: u32,
}
#[derive(Debug)]
pub(crate) struct WithItem {
    pub node: NodeRef,
    pub value: NodeRef,
    pub as_pattern: Option<NodeRef>,
    pub target: Option<NodeRef>,
    pub target_wrapper: Option<NodeRef>,
    pub context: u32,
}
#[derive(Default, Debug)]
pub(crate) struct Counters {
    pub documents: usize,
    /// All syntax nodes, including punctuation and comments, each visited once.
    pub visited_nodes: usize,
    pub calls: usize,
    pub calls_without_function: usize,
    pub semantic_callees: usize,
    pub peak_pending_nodes: usize,
}
#[derive(Default, Debug)]
pub(crate) struct PythonStructure {
    pub contexts: Vec<ExecutionContext>,
    pub calls: Vec<CallSite>,
    pub arguments: Vec<Argument>,
    pub attributes: Vec<Attribute>,
    pub parentheses: Vec<Parenthesis>,
    pub sequences: Vec<Sequence>,
    pub sequence_children: Vec<SequenceChild>,
    pub assignments: Vec<Assignment>,
    pub with_items: Vec<WithItem>,
    pub counters: Counters,
}
struct Pending<'t, D: Doc> {
    node: Node<'t, D>,
    context: u32,
    argument_owner: Option<usize>,
}
fn unparenthesized<'t, D: Doc>(mut node: Node<'t, D>) -> Node<'t, D> {
    while node.kind() == "parenthesized_expression" {
        let Some(child) = node.children().find(|child| {
            child.is_named()
                && !child.kind().ends_with("comment")
                && child.kind() != "line_continuation"
        }) else {
            break;
        };
        node = child;
    }
    node
}
fn named<D: Doc>(node: &Node<'_, D>) -> bool {
    node.is_named() && !node.kind().ends_with("comment")
}

/// Collects owned byte-range facts; no Node or document reference escapes this call.
/// Document IDs must be unique within the batch.
pub(crate) fn collect<'t, D: Doc>(documents: Vec<(u32, Node<'t, D>)>) -> PythonStructure {
    collect_with(documents, |_, _| {})
}

/// Observes each original Node during the same pass, allowing the caller to build
/// its NodeRef-to-Node index without a second walk. The callback may clone Nodes.
pub(crate) fn collect_with<'t, D: Doc>(
    documents: Vec<(u32, Node<'t, D>)>,
    mut observe: impl FnMut(u32, &Node<'t, D>),
) -> PythonStructure {
    let mut out = PythonStructure::default();
    let mut pending = Vec::new();
    for (doc, root) in documents {
        out.counters.documents += 1;
        let module = out.contexts.len() as u32;
        out.contexts.push(ExecutionContext {
            kind: ContextKind::Module,
            parent: None,
            owner: None,
            body: NodeRef::of(doc, &root),
        });
        pending.push(Pending {
            node: root,
            context: module,
            argument_owner: None,
        });
        out.counters.peak_pending_nodes = out.counters.peak_pending_nodes.max(pending.len());
        while let Some(Pending {
            node,
            context,
            argument_owner,
        }) = pending.pop()
        {
            observe(doc, &node);
            out.counters.visited_nodes += 1;
            let at = NodeRef::of(doc, &node);
            let kind = node.kind();
            let children: Vec<_> = node.children().collect();
            let body = match kind.as_ref() {
                "function_definition" | "class_definition" | "lambda" => node.field("body"),
                _ => None,
            };
            let body_context = body.as_ref().map(|body| {
                let kind = match kind.as_ref() {
                    "class_definition" => ContextKind::Class,
                    "lambda" => ContextKind::Lambda,
                    _ if children.iter().any(|child| child.kind() == "async") => {
                        ContextKind::AsyncFunction
                    }
                    _ => ContextKind::SyncFunction,
                };
                let id = out.contexts.len() as u32;
                out.contexts.push(ExecutionContext {
                    kind,
                    parent: Some(context),
                    owner: Some(at),
                    body: NodeRef::of(doc, body),
                });
                id
            });
            if let Some(call) = argument_owner {
                let start = out.arguments.len();
                if kind == "generator_expression" {
                    out.arguments.push(Argument {
                        call,
                        node: at,
                        value: at,
                        ordinal: 0,
                        keyword: None,
                        spread: Spread::None,
                        positional_index: Some(0),
                        after_positional_spread: false,
                    });
                } else {
                    let mut positional = 0;
                    let mut starred = false;
                    for (ordinal, child) in children.iter().filter(|child| named(child)).enumerate()
                    {
                        let spread = match child.kind().as_ref() {
                            "dictionary_splat" => Spread::Keywords,
                            "list_splat" | "parenthesized_list_splat" => Spread::Positional,
                            _ => Spread::None,
                        };
                        let keyword = (child.kind() == "keyword_argument")
                            .then(|| child.field("name"))
                            .flatten();
                        let value = if child.kind() == "keyword_argument" {
                            child.field("value").unwrap_or_else(|| child.clone())
                        } else if spread != Spread::None {
                            child
                                .children()
                                .find(named)
                                .unwrap_or_else(|| child.clone())
                        } else {
                            child.clone()
                        };
                        let position = if child.kind() != "keyword_argument"
                            && spread == Spread::None
                            && !starred
                        {
                            let index = positional;
                            positional += 1;
                            Some(index)
                        } else {
                            None
                        };
                        out.arguments.push(Argument {
                            call,
                            node: NodeRef::of(doc, child),
                            value: NodeRef::of(doc, &value),
                            ordinal,
                            keyword: keyword.map(|name| {
                                crate::code_rule_dsl::identifier::python_identifier(&name)
                                    .into_owned()
                            }),
                            spread,
                            positional_index: position,
                            after_positional_spread: starred,
                        });
                        starred |= spread == Spread::Positional;
                    }
                }
                out.calls[call].arguments = start..out.arguments.len();
            }
            let mut call_arguments = None;
            if kind == "call" {
                out.counters.calls += 1;
                if let Some(callee) = node.field("function") {
                    let leaf = unparenthesized(callee.clone());
                    let reference = matches!(leaf.kind().as_ref(), "identifier" | "attribute")
                        .then(|| NodeRef::of(doc, &leaf));
                    out.counters.semantic_callees += usize::from(reference.is_some());
                    let call = out.calls.len();
                    out.calls.push(CallSite {
                        call: at,
                        callee: NodeRef::of(doc, &callee),
                        report_node: NodeRef::of(doc, &leaf),
                        ref_node: reference,
                        context,
                        arguments: out.arguments.len()..out.arguments.len(),
                    });
                    call_arguments = node
                        .field("arguments")
                        .map(|arguments| (arguments.node_id(), call));
                } else {
                    out.counters.calls_without_function += 1;
                }
            }
            match kind.as_ref() {
                "attribute" => {
                    if let (Some(object), Some(name)) =
                        (node.field("object"), node.field("attribute"))
                    {
                        out.attributes.push(Attribute {
                            node: at,
                            object: NodeRef::of(doc, &object),
                            name_node: NodeRef::of(doc, &name),
                            name: crate::code_rule_dsl::identifier::python_identifier(&name)
                                .into_owned(),
                        });
                    }
                }
                "parenthesized_expression" => {
                    if let Some(child) = children.iter().find(|child| named(child)) {
                        out.parentheses.push(Parenthesis {
                            node: at,
                            child: NodeRef::of(doc, child),
                        });
                    }
                }
                "tuple" | "list" | "set" | "pattern_list" | "tuple_pattern" | "list_pattern"
                | "expression_list" => {
                    let start = out.sequence_children.len();
                    for (named_index, child) in
                        children.iter().filter(|child| named(child)).enumerate()
                    {
                        out.sequence_children.push(SequenceChild {
                            node: NodeRef::of(doc, child),
                            named_index,
                            spread: matches!(
                                child.kind().as_ref(),
                                "list_splat" | "list_splat_pattern" | "parenthesized_list_splat"
                            ),
                        });
                    }
                    let kind = match kind.as_ref() {
                        "tuple" => SequenceKind::Tuple,
                        "list" => SequenceKind::List,
                        "set" => SequenceKind::Set,
                        "expression_list" => SequenceKind::Expressions,
                        _ => SequenceKind::Pattern,
                    };
                    out.sequences.push(Sequence {
                        node: at,
                        kind,
                        children: start..out.sequence_children.len(),
                    });
                }
                "assignment" | "augmented_assignment" | "named_expression" => {
                    if let Some(left) = node.field("left").or_else(|| node.field("name")) {
                        let right = node.field("right").or_else(|| node.field("value"));
                        let annotation = node.field("type");
                        let kind = match kind.as_ref() {
                            "named_expression" => AssignmentKind::NamedExpression,
                            "augmented_assignment" => AssignmentKind::Augmented,
                            _ => AssignmentKind::Assignment,
                        };
                        out.assignments.push(Assignment {
                            node: at,
                            left: NodeRef::of(doc, &left),
                            right: right.as_ref().map(|right| NodeRef::of(doc, right)),
                            annotation: annotation
                                .as_ref()
                                .map(|annotation| NodeRef::of(doc, annotation)),
                            kind,
                            context,
                        });
                    }
                }
                "with_item" => {
                    if let Some(value) = node.field("value") {
                        let as_pattern = (value.kind() == "as_pattern").then_some(&value);
                        let expression = as_pattern
                            .and_then(|value| value.children().find(named))
                            .unwrap_or_else(|| value.clone());
                        let target_wrapper = as_pattern.and_then(|value| value.field("alias"));
                        let target = target_wrapper.as_ref().map(|target| {
                            if target.kind() == "as_pattern_target" {
                                target
                                    .children()
                                    .find(named)
                                    .unwrap_or_else(|| target.clone())
                            } else {
                                target.clone()
                            }
                        });
                        out.with_items.push(WithItem {
                            node: at,
                            value: NodeRef::of(doc, &expression),
                            as_pattern: as_pattern.map(|pattern| NodeRef::of(doc, pattern)),
                            target: target.as_ref().map(|target| NodeRef::of(doc, target)),
                            target_wrapper: target_wrapper
                                .as_ref()
                                .map(|target| NodeRef::of(doc, target)),
                            context,
                        });
                    }
                }
                _ => {}
            }
            for child in children.into_iter().rev() {
                let child_context = if body
                    .as_ref()
                    .is_some_and(|body| body.node_id() == child.node_id())
                {
                    body_context.unwrap()
                } else {
                    context
                };
                let argument_owner = call_arguments
                    .filter(|(arguments, _)| *arguments == child.node_id())
                    .map(|(_, call)| call);
                pending.push(Pending {
                    node: child,
                    context: child_context,
                    argument_owner,
                });
            }
            out.counters.peak_pending_nodes = out.counters.peak_pending_nodes.max(pending.len());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::language::GrammarDialect;
    use ast_grep_core::AstGrep;
    use std::collections::BTreeMap;

    fn text(source: &str, node: NodeRef) -> &str {
        &source[node.start..node.end]
    }

    #[test]
    fn execution_context_switches_only_at_definition_and_lambda_bodies() {
        let source = "module_call()\nasync def outer():\n    outer_call()\n    @decorate(decorator_arg())\n    def sync(x=default_call(), y: annotation_call()=0) -> return_annotation():\n        sync_body()\n    async def nested(x=nested_default()):\n        nested_body()\n    class C(base_call(), metaclass=meta_call()):\n        class_body()\n        def method(x=method_default()):\n            method_body()\n    thunk = lambda x=lambda_default(): lambda_body()\n    def f(x=lambda y=nested_lambda_default(): nested_lambda_body()):\n        f_body()\n    consume(generated() for _ in iterable())\n";
        let root = AstGrep::str(source, GrammarDialect::Python);
        let facts = collect(vec![(4, root.root())]);
        let actual: BTreeMap<_, _> = facts
            .calls
            .iter()
            .map(|call| {
                let context = &facts.contexts[call.context as usize];
                (text(source, call.report_node), context.kind)
            })
            .collect();
        let expected = [
            ("module_call", ContextKind::Module),
            ("outer_call", ContextKind::AsyncFunction),
            ("decorate", ContextKind::AsyncFunction),
            ("decorator_arg", ContextKind::AsyncFunction),
            ("default_call", ContextKind::AsyncFunction),
            ("annotation_call", ContextKind::AsyncFunction),
            ("return_annotation", ContextKind::AsyncFunction),
            ("sync_body", ContextKind::SyncFunction),
            ("nested_default", ContextKind::AsyncFunction),
            ("nested_body", ContextKind::AsyncFunction),
            ("base_call", ContextKind::AsyncFunction),
            ("meta_call", ContextKind::AsyncFunction),
            ("class_body", ContextKind::Class),
            ("method_default", ContextKind::Class),
            ("method_body", ContextKind::SyncFunction),
            ("lambda_default", ContextKind::AsyncFunction),
            ("lambda_body", ContextKind::Lambda),
            ("nested_lambda_default", ContextKind::AsyncFunction),
            ("nested_lambda_body", ContextKind::Lambda),
            ("f_body", ContextKind::SyncFunction),
            ("consume", ContextKind::AsyncFunction),
            ("generated", ContextKind::AsyncFunction),
            ("iterable", ContextKind::AsyncFunction),
        ]
        .into_iter()
        .collect();
        assert_eq!(actual, expected);
        assert_eq!(facts.calls.len(), 23);
        let find = |name| {
            facts
                .calls
                .iter()
                .find(|call| text(source, call.report_node) == name)
                .unwrap()
        };
        let outer_context = find("outer_call").context;
        assert_eq!(find("nested_default").context, outer_context);
        assert_ne!(find("nested_body").context, outer_context);
        let nested = &facts.contexts[find("nested_body").context as usize];
        assert_eq!(nested.parent, Some(outer_context));
        assert!(text(source, nested.owner.unwrap()).starts_with("async def nested"));
        assert!(text(source, nested.body).contains("nested_body()"));
    }

    #[test]
    fn callee_positions_arguments_and_observer_preserve_original_node_identity() {
        let source = "async def f():\n    (\n      (\n        pkg.open\n      )\n    )(1, *xs, tail(), mode=(pkg.WAIT), **kw, K=2)\n    consume(x for x in source())\n    factory()()\n";
        let root = AstGrep::str(source, GrammarDialect::Python);
        let mut nodes = BTreeMap::new();
        let facts = collect_with(vec![(12, root.root())], |doc, node| {
            assert!(nodes.insert(NodeRef::of(doc, node), node.clone()).is_none());
        });
        assert_eq!(nodes.len(), facts.counters.visited_nodes);
        assert_eq!(facts.counters.documents, 1);
        assert_eq!(facts.counters.calls, facts.calls.len());
        let (call_id, call) = facts
            .calls
            .iter()
            .enumerate()
            .find(|(_, call)| text(source, call.report_node) == "pkg.open")
            .unwrap();
        assert_eq!(
            text(source, call.callee),
            "(\n      (\n        pkg.open\n      )\n    )"
        );
        assert_eq!(call.report_node.start, source.find("pkg.open").unwrap());
        assert_eq!(call.ref_node, Some(call.report_node));
        assert_eq!(nodes[&call.report_node].kind(), "attribute");
        assert_eq!(nodes[&call.callee].kind(), "parenthesized_expression");
        let rows = &facts.arguments[call.arguments.clone()];
        assert_eq!(rows.len(), 6);
        assert!(rows.iter().all(|row| row.call == call_id));
        assert_eq!(
            rows.iter().map(|row| row.ordinal).collect::<Vec<_>>(),
            (0..6).collect::<Vec<_>>()
        );
        assert_eq!(
            rows.iter()
                .map(|row| row.positional_index)
                .collect::<Vec<_>>(),
            vec![Some(0), None, None, None, None, None]
        );
        assert_eq!(rows[1].spread, Spread::Positional);
        assert_eq!(text(source, rows[1].value), "xs");
        assert!(!rows[1].after_positional_spread);
        assert!(rows[2..].iter().all(|row| row.after_positional_spread));
        assert_eq!(rows[3].keyword.as_deref(), Some("mode"));
        assert_eq!(text(source, rows[3].value), "(pkg.WAIT)");
        assert_eq!(rows[4].spread, Spread::Keywords);
        assert_eq!(text(source, rows[4].value), "kw");
        assert_eq!(rows[5].keyword.as_deref(), Some("K"));
        let generator = facts
            .calls
            .iter()
            .find(|call| text(source, call.report_node) == "consume")
            .unwrap();
        let rows = &facts.arguments[generator.arguments.clone()];
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].positional_index, Some(0));
        assert_eq!(nodes[&rows[0].value].kind(), "generator_expression");
        let indirect = facts
            .calls
            .iter()
            .find(|call| text(source, call.callee) == "factory()")
            .unwrap();
        assert!(indirect.ref_node.is_none());
        assert_eq!(text(source, indirect.report_node), "factory()");
        assert!(facts.parentheses.len() >= 3);
    }

    #[test]
    fn sequence_assignment_and_with_rows_support_structural_binding_joins() {
        let source = "a, b = [one(), # ignored\n *xs, three()]\nx: T = init()\ny: T\n(z := choose())\nwith maker() as (p, q), other() as r:\n    use(p)\n";
        let root = AstGrep::str(source, GrammarDialect::Python);
        let other = AstGrep::str("second()\n", GrammarDialect::Python);
        let facts = collect(vec![(1, root.root()), (2, other.root())]);
        assert_eq!(facts.counters.documents, 2);
        let sequence = facts
            .sequences
            .iter()
            .find(|sequence| sequence.kind == SequenceKind::List)
            .unwrap();
        let rows = &facts.sequence_children[sequence.children.clone()];
        assert_eq!(
            rows.iter().map(|row| row.named_index).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(
            rows.iter().map(|row| row.spread).collect::<Vec<_>>(),
            vec![false, true, false]
        );
        let annotated = facts
            .assignments
            .iter()
            .find(|assignment| text(source, assignment.left) == "x")
            .unwrap();
        assert_eq!(text(source, annotated.right.unwrap()), "init()");
        assert_eq!(text(source, annotated.annotation.unwrap()), "T");
        let bare = facts
            .assignments
            .iter()
            .find(|assignment| text(source, assignment.left) == "y")
            .unwrap();
        assert!(bare.right.is_none());
        assert!(facts.assignments.iter().any(|assignment| assignment.kind
            == AssignmentKind::NamedExpression
            && text(source, assignment.left) == "z"));
        assert_eq!(facts.with_items.len(), 2);
        assert_eq!(text(source, facts.with_items[0].value), "maker()");
        assert_eq!(text(source, facts.with_items[0].target.unwrap()), "(p, q)");
        assert!(facts.with_items[0].as_pattern.is_some());
        assert!(facts.with_items[0].target_wrapper.is_some());
        assert_eq!(text(source, facts.with_items[1].target.unwrap()), "r");
        let second = facts.calls.iter().find(|call| call.call.doc == 2).unwrap();
        assert_eq!(text("second()\n", second.report_node), "second");
        assert_eq!(
            facts.contexts[second.context as usize].kind,
            ContextKind::Module
        );
    }
}
