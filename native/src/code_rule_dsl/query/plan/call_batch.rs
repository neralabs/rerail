//! Project call rows from Python tape columns without materializing the whole AST.
use super::python_structure::{
    Argument, Attribute, CallSite, ContextKind, ExecutionContext, NodeRef, PythonStructure, Spread,
};
use crate::code_rule_dsl::tape::TapeDoc;
use ast_grep_core::{Doc, Node, language::Language};
use rustc_hash::FxHashSet;
use std::{cmp::Reverse, collections::BinaryHeap};

struct Kinds {
    call: u16,
    function: u16,
    class: u16,
    lambda: u16,
    asynchronous: u16,
    identifier: u16,
    attribute: u16,
    parens: u16,
    generator: u16,
    keyword: u16,
    list_splat: u16,
    parenthesized_splat: u16,
    dictionary_splat: u16,
    comment: u16,
    continuation: u16,
    function_field: u16,
}
impl Kinds {
    fn new(doc: &TapeDoc) -> Self {
        let lang = doc.get_lang();
        Self {
            call: lang.kind_to_id("call"),
            function: lang.kind_to_id("function_definition"),
            class: lang.kind_to_id("class_definition"),
            lambda: lang.kind_to_id("lambda"),
            // Keyword tokens are anonymous; the DSL kind lookup names only
            // named kinds and would return zero for this token.
            asynchronous: lang
                .kinds()
                .grammar()
                .expect("Python grammar")
                .id_for_node_kind("async", false),
            identifier: lang.kind_to_id("identifier"),
            attribute: lang.kind_to_id("attribute"),
            parens: lang.kind_to_id("parenthesized_expression"),
            generator: lang.kind_to_id("generator_expression"),
            keyword: lang.kind_to_id("keyword_argument"),
            list_splat: lang.kind_to_id("list_splat"),
            parenthesized_splat: lang.kind_to_id("parenthesized_list_splat"),
            dictionary_splat: lang.kind_to_id("dictionary_splat"),
            comment: lang.kind_to_id("comment"),
            continuation: lang.kind_to_id("line_continuation"),
            function_field: lang.field_to_id("function").unwrap(),
        }
    }
}
fn children(doc: &TapeDoc, parent: u32) -> impl Iterator<Item = u32> + '_ {
    let end = doc.transducer_descendants(parent).end;
    let mut next = parent + 1;
    std::iter::from_fn(move || {
        if next >= end {
            return None;
        }
        let current = next;
        next = doc.transducer_descendants(current).end;
        Some(current)
    })
}
fn field(doc: &TapeDoc, parent: u32, field: u16) -> Option<u32> {
    children(doc, parent).find(|&child| doc.transducer_field(child) == field)
}
fn at<'t>(root: &Node<'t, TapeDoc>, ordinal: u32) -> Node<'t, TapeDoc> {
    root.root().adopt(root.get_doc().node(ordinal))
}
fn node_ref(doc_id: u32, doc: &TapeDoc, ordinal: u32) -> NodeRef {
    let span = doc.transducer_span(ordinal);
    NodeRef {
        doc: doc_id,
        node_id: ordinal as usize,
        start: span.start,
        end: span.end,
    }
}
fn emit<'t>(
    doc: u32,
    node: &Node<'t, TapeDoc>,
    seen: &mut FxHashSet<usize>,
    observe: &mut impl FnMut(u32, &Node<'t, TapeDoc>),
) {
    if seen.insert(node.node_id()) {
        observe(doc, node);
    }
}
fn unparen<'t>(mut node: Node<'t, TapeDoc>, kinds: &Kinds) -> Node<'t, TapeDoc> {
    while node.kind_id() == kinds.parens {
        let Some(child) = node.children().find(|child| {
            child.is_named()
                && child.kind_id() != kinds.comment
                && child.kind_id() != kinds.continuation
        }) else {
            break;
        };
        node = child;
    }
    node
}
fn named(node: &Node<'_, TapeDoc>, kinds: &Kinds) -> bool {
    node.is_named() && node.kind_id() != kinds.comment
}

/// Reads every tape row once for kind/context dispatch. Only definitions and calls
/// in `wanted` create AST Nodes; the observer receives projected nodes, not all rows.
/// Each root must be a document (or subtree) of a Python TapeDoc.
pub(crate) fn collect<'t>(
    roots: &[(u32, Node<'t, TapeDoc>)],
    wanted: ContextKind,
    mut observe: impl FnMut(u32, &Node<'t, TapeDoc>),
) -> PythonStructure {
    let mut out = PythonStructure::default();
    for (doc_id, root) in roots {
        let doc = root.get_doc();
        let kinds = Kinds::new(doc);
        let first = root.node_id() as u32;
        let end = doc.transducer_descendants(first).end;
        let module = out.contexts.len() as u32;
        out.contexts.push(ExecutionContext {
            kind: ContextKind::Module,
            parent: None,
            owner: None,
            body: NodeRef::of(*doc_id, root),
        });
        let mut active = vec![(end, module)];
        // Headers can contain lambdas whose bodies precede a pending definition's
        // body. Events therefore follow body start, not definition discovery order.
        let mut starts = BinaryHeap::<Reverse<(u32, u32, u32)>>::new();
        let mut observed = FxHashSet::default();
        out.counters.documents += 1;
        for ordinal in first..end {
            out.counters.visited_nodes += 1;
            while active.last().is_some_and(|&(stop, _)| ordinal >= stop) {
                active.pop();
            }
            while starts
                .peek()
                .is_some_and(|Reverse((start, _, _))| *start == ordinal)
            {
                let Reverse((_, stop, context)) = starts.pop().unwrap();
                active.push((stop, context));
            }
            let context = active.last().expect("root interval").1;
            let kind = doc.transducer_kind(ordinal);
            if kind == kinds.function || kind == kinds.class || kind == kinds.lambda {
                let owner = at(root, ordinal);
                // Preserve Node's field selection, including tape aliases/first-field rules.
                if let Some(body_node) = owner.field("body") {
                    let body = body_node.node_id() as u32;
                    let context_kind = if kind == kinds.class {
                        ContextKind::Class
                    } else if kind == kinds.lambda {
                        ContextKind::Lambda
                    } else if children(doc, ordinal)
                        .any(|child| doc.transducer_kind(child) == kinds.asynchronous)
                    {
                        ContextKind::AsyncFunction
                    } else {
                        ContextKind::SyncFunction
                    };
                    let id = out.contexts.len() as u32;
                    out.contexts.push(ExecutionContext {
                        kind: context_kind,
                        parent: Some(context),
                        owner: Some(NodeRef::of(*doc_id, &owner)),
                        body: node_ref(*doc_id, doc, body),
                    });
                    starts.push(Reverse((body, doc.transducer_descendants(body).end, id)));
                }
            }
            out.counters.peak_pending_nodes = out
                .counters
                .peak_pending_nodes
                .max(starts.len() + active.len());
            if kind != kinds.call {
                continue;
            }
            out.counters.calls += 1;
            if field(doc, ordinal, kinds.function_field).is_none() {
                out.counters.calls_without_function += 1;
                continue;
            }
            if out.contexts[context as usize].kind != wanted {
                continue;
            }
            let call = at(root, ordinal);
            let Some(callee) = call.field("function") else {
                continue;
            };
            let report = unparen(callee.clone(), &kinds);
            let reference = (report.kind_id() == kinds.identifier
                || report.kind_id() == kinds.attribute)
                .then(|| NodeRef::of(*doc_id, &report));
            out.counters.semantic_callees += usize::from(reference.is_some());
            let index = out.calls.len();
            let start = out.arguments.len();
            emit(*doc_id, &call, &mut observed, &mut observe);
            emit(*doc_id, &callee, &mut observed, &mut observe);
            emit(*doc_id, &report, &mut observed, &mut observe);
            if report.kind_id() == kinds.attribute {
                if let (Some(object), Some(name)) =
                    (report.field("object"), report.field("attribute"))
                {
                    emit(*doc_id, &object, &mut observed, &mut observe);
                    let bare_object = unparen(object.clone(), &kinds);
                    emit(*doc_id, &bare_object, &mut observed, &mut observe);
                    emit(*doc_id, &name, &mut observed, &mut observe);
                    out.attributes.push(Attribute {
                        node: NodeRef::of(*doc_id, &report),
                        object: NodeRef::of(*doc_id, &object),
                        name_node: NodeRef::of(*doc_id, &name),
                        name: crate::code_rule_dsl::identifier::python_identifier(&name)
                            .into_owned(),
                    });
                }
            }
            if let Some(arguments) = call.field("arguments") {
                if arguments.kind_id() == kinds.generator {
                    emit(*doc_id, &arguments, &mut observed, &mut observe);
                    out.arguments.push(Argument {
                        call: index,
                        node: NodeRef::of(*doc_id, &arguments),
                        value: NodeRef::of(*doc_id, &arguments),
                        ordinal: 0,
                        keyword: None,
                        spread: Spread::None,
                        positional_index: Some(0),
                        after_positional_spread: false,
                    });
                } else {
                    let mut positional = 0;
                    let mut starred = false;
                    for (ordinal, argument) in arguments
                        .children()
                        .filter(|node| named(node, &kinds))
                        .enumerate()
                    {
                        let kind = argument.kind_id();
                        let spread =
                            if kind == kinds.list_splat || kind == kinds.parenthesized_splat {
                                Spread::Positional
                            } else if kind == kinds.dictionary_splat {
                                Spread::Keywords
                            } else {
                                Spread::None
                            };
                        let keyword = if kind == kinds.keyword {
                            argument.field("name")
                        } else {
                            None
                        };
                        let value = if kind == kinds.keyword {
                            argument.field("value").unwrap_or_else(|| argument.clone())
                        } else if spread != Spread::None {
                            argument
                                .children()
                                .find(|child| named(child, &kinds))
                                .unwrap_or_else(|| argument.clone())
                        } else {
                            argument.clone()
                        };
                        let position =
                            if kind != kinds.keyword && spread == Spread::None && !starred {
                                let at = positional;
                                positional += 1;
                                Some(at)
                            } else {
                                None
                            };
                        emit(*doc_id, &argument, &mut observed, &mut observe);
                        emit(*doc_id, &value, &mut observed, &mut observe);
                        let bare_value = unparen(value.clone(), &kinds);
                        emit(*doc_id, &bare_value, &mut observed, &mut observe);
                        if let Some(name) = &keyword {
                            emit(*doc_id, name, &mut observed, &mut observe);
                        }
                        out.arguments.push(Argument {
                            call: index,
                            node: NodeRef::of(*doc_id, &argument),
                            value: NodeRef::of(*doc_id, &value),
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
            }
            out.calls.push(CallSite {
                call: NodeRef::of(*doc_id, &call),
                callee: NodeRef::of(*doc_id, &callee),
                report_node: NodeRef::of(*doc_id, &report),
                ref_node: reference,
                context,
                arguments: start..out.arguments.len(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_rule_dsl::{
        language::GrammarDialect,
        query::plan::python_structure,
        tape::{Grammar, TapeImage},
    };
    use ast_grep_core::{AstGrep, tree_sitter::StrDoc};
    use std::sync::Arc;

    fn tape(source: &str) -> AstGrep<TapeDoc> {
        let grammar = Grammar::new(GrammarDialect::Python).unwrap();
        let parsed = StrDoc::new(source, GrammarDialect::Python);
        let image =
            TapeImage::encode(parsed.tree.root_node(), &grammar, source, &|| false).unwrap();
        AstGrep::doc(
            TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap(),
        )
    }
    fn call_rows(
        facts: &PythonStructure,
        wanted: ContextKind,
    ) -> Vec<(
        NodeRef,
        NodeRef,
        NodeRef,
        Option<NodeRef>,
        u32,
        Vec<(
            NodeRef,
            NodeRef,
            usize,
            Option<String>,
            Spread,
            Option<usize>,
            bool,
        )>,
    )> {
        facts
            .calls
            .iter()
            .filter(|call| facts.contexts[call.context as usize].kind == wanted)
            .map(|call| {
                let arguments = facts.arguments[call.arguments.clone()]
                    .iter()
                    .map(|arg| {
                        (
                            arg.node,
                            arg.value,
                            arg.ordinal,
                            arg.keyword.clone(),
                            arg.spread,
                            arg.positional_index,
                            arg.after_positional_spread,
                        )
                    })
                    .collect();
                (
                    call.call,
                    call.callee,
                    call.report_node,
                    call.ref_node,
                    call.context,
                    arguments,
                )
            })
            .collect()
    }
    #[test]
    fn column_contexts_and_selected_call_rows_match_generic_collection() {
        let source = "\noutside()\nasync def outer():\n    ((\\\n      pkg.wait\\\n    ))(1, *items, trailing(), mode=(os.P_WAIT), **kw, K=2)\n    @decorate(header())\n    def sync(x=header_default(), y: annotation()=0):\n        sync_only()\n    class C(base()):\n        class_only()\n        async def method(x=class_default()):\n            nested_async()\n    thunk = lambda x=default(): lambda_only()\n    def inner(x=lambda y=lambda_default(): lambda_body()):\n        inner_body()\n    consume(generated() for x in iterable())\n    factory()()\n";
        let root = tape(source);
        let roots = vec![(9, root.root())];
        let generic = python_structure::collect(roots.clone());
        for wanted in [
            ContextKind::Module,
            ContextKind::AsyncFunction,
            ContextKind::SyncFunction,
            ContextKind::Class,
            ContextKind::Lambda,
        ] {
            let mut seen = FxHashSet::default();
            let actual = collect(&roots, wanted, |doc, node| {
                assert_eq!(doc, 9);
                assert!(seen.insert(node.node_id()));
            });
            assert_eq!(
                call_rows(&actual, wanted),
                call_rows(&generic, wanted),
                "{wanted:?}"
            );
            assert_eq!(
                actual
                    .contexts
                    .iter()
                    .map(|c| (c.kind, c.parent, c.owner, c.body))
                    .collect::<Vec<_>>(),
                generic
                    .contexts
                    .iter()
                    .map(|c| (c.kind, c.parent, c.owner, c.body))
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                actual.counters.visited_nodes,
                root.root().get_doc().transducer_len()
            );
            assert_eq!(actual.counters.calls, generic.counters.calls);
            for (index, call) in actual.calls.iter().enumerate() {
                for node in [call.call, call.callee, call.report_node] {
                    assert!(seen.contains(&node.node_id));
                }
                for arg in &actual.arguments[call.arguments.clone()] {
                    assert_eq!(arg.call, index);
                    assert!(seen.contains(&arg.node.node_id));
                    assert!(seen.contains(&arg.value.node_id));
                }
            }
            let reports: FxHashSet<_> = actual.calls.iter().map(|c| c.report_node).collect();
            let expected: Vec<_> = generic
                .attributes
                .iter()
                .filter(|a| reports.contains(&a.node))
                .map(|a| (a.node, a.object, a.name_node, a.name.clone()))
                .collect();
            assert_eq!(
                actual
                    .attributes
                    .iter()
                    .map(|a| (a.node, a.object, a.name_node, a.name.clone()))
                    .collect::<Vec<_>>(),
                expected
            );
            assert!(
                actual.sequences.is_empty()
                    && actual.sequence_children.is_empty()
                    && actual.assignments.is_empty()
                    && actual.with_items.is_empty()
                    && actual.parentheses.is_empty()
            );
            assert!(seen.len() < actual.counters.visited_nodes);
        }
    }
    #[test]
    fn projected_nodes_keep_comment_adjacency_and_multiline_report_positions() {
        let source = "\nasync def f():\n    (\n       pkg.open # adjacent comment\n    )()\n    consume(x for x in source())\n";
        let root = tape(source);
        let mut projected = std::collections::BTreeMap::new();
        let facts = collect(
            &[(2, root.root())],
            ContextKind::AsyncFunction,
            |doc, node| {
                projected.insert(NodeRef::of(doc, node), node.clone());
            },
        );
        let call = facts
            .calls
            .iter()
            .find(|call| &source[call.report_node.start..call.report_node.end] == "pkg.open")
            .unwrap();
        assert_eq!(call.report_node.start, source.find("pkg.open").unwrap());
        assert_eq!(call.ref_node, Some(call.report_node));
        assert_eq!(
            projected[&call.report_node].next().unwrap().kind(),
            "comment"
        );
        assert_eq!(
            projected[&call.callee].next().unwrap().kind(),
            "argument_list"
        );
        let attr = facts
            .attributes
            .iter()
            .find(|attr| attr.node == call.report_node)
            .unwrap();
        assert!(projected.contains_key(&attr.name_node));
        assert!(projected.contains_key(&attr.object));
        let generator = facts
            .calls
            .iter()
            .find(|call| &source[call.report_node.start..call.report_node.end] == "consume")
            .unwrap();
        let args = &facts.arguments[generator.arguments.clone()];
        assert_eq!(args.len(), 1);
        assert_eq!(args[0].positional_index, Some(0));
        assert_eq!(projected[&args[0].value].kind(), "generator_expression");
    }
}
