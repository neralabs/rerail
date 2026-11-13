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
    imports: [u16; 3],
}
impl Kinds {
    fn new(doc: &TapeDoc) -> Self {
        let lang = doc.get_lang();
        Self {
            call: lang.kind_to_id("call"),
            imports: [
                "import_statement",
                "import_from_statement",
                "future_import_statement",
            ]
            .map(|k| lang.kind_to_id(k)),
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
pub fn at<'t>(root: &Node<'t, TapeDoc>, ordinal: u32) -> Node<'t, TapeDoc> {
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
/// in `wanted` create AST Nodes for the observer. Import declarations are decoded
/// only for documents with selected calls, and sent to a separate batch observer.
/// Calls have empty argument ranges by contract; collect_arguments is a separate
/// producer for the calls selected by relational target composition.
/// Each root must be a document (or subtree) of a Python TapeDoc.
pub fn collect<'t>(
    roots: &[(u32, Node<'t, TapeDoc>)],
    wanted: ContextKind,
    mut observe: impl FnMut(u32, &Node<'t, TapeDoc>),
    mut observe_import: impl FnMut(u32, &Node<'t, TapeDoc>),
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
        let mut imports = Vec::new();
        let first_call = out.calls.len();
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
            if kinds.imports.contains(&kind) {
                imports.push(ordinal);
            }
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
            out.calls.push(CallSite {
                call: NodeRef::of(*doc_id, &call),
                callee: NodeRef::of(*doc_id, &callee),
                report_node: NodeRef::of(*doc_id, &report),
                ref_node: reference,
                context,
                // Arguments are a separate demanded producer after target selection.
                arguments: 0..0,
            });
        }
        if out.calls.len() != first_call {
            for ordinal in imports {
                observe_import(*doc_id, &at(root, ordinal));
            }
        }
    }
    out
}

#[derive(Default, Debug)]
pub struct ArgumentCounters {
    pub requested_calls: usize,
    pub distinct_calls: usize,
    pub argument_rows: usize,
}
#[derive(Debug)]
pub struct CallArguments {
    pub call: NodeRef,
    pub arguments: std::ops::Range<usize>,
}
#[derive(Default, Debug)]
pub struct ArgumentBatch {
    pub calls: Vec<CallArguments>,
    /// Argument.call is an index into this batch's calls, not collect().calls.
    pub arguments: Vec<Argument>,
    pub counters: ArgumentCounters,
}
/// Produces argument facts only for the explicitly requested call nodes. Repeated
/// (document, node ID) demands execute once; no demand performs no AST work.
pub fn collect_arguments<'t>(calls: &[(u32, Node<'t, TapeDoc>)]) -> ArgumentBatch {
    let mut out = ArgumentBatch::default();
    out.counters.requested_calls = calls.len();
    let mut seen = FxHashSet::default();
    let mut by_doc = std::collections::BTreeMap::new();
    for (doc_id, call) in calls {
        let identity = call.root() as *const _ as usize;
        let (prior, kinds) = by_doc
            .entry(*doc_id)
            .or_insert_with(|| (identity, Kinds::new(call.get_doc())));
        assert_eq!(
            *prior, identity,
            "argument demands reused a document ID for different roots"
        );
        // Identity validation precedes deduplication: different tapes can assign
        // the same ordinal to unrelated nodes.
        if !seen.insert((*doc_id, call.node_id())) {
            continue;
        }
        let kinds: &Kinds = kinds;
        assert_eq!(
            call.kind_id(),
            kinds.call,
            "argument demand must name a call"
        );
        out.counters.distinct_calls += 1;
        let index = out.calls.len();
        let start = out.arguments.len();
        if let Some(arguments) = call.field("arguments") {
            if arguments.kind_id() == kinds.generator {
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
                    let spread = if kind == kinds.list_splat || kind == kinds.parenthesized_splat {
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
                    let position = if kind != kinds.keyword && spread == Spread::None && !starred {
                        let at = positional;
                        positional += 1;
                        Some(at)
                    } else {
                        None
                    };
                    out.arguments.push(Argument {
                        call: index,
                        node: NodeRef::of(*doc_id, &argument),
                        value: NodeRef::of(*doc_id, &value),
                        ordinal,
                        keyword: keyword.map(|name| {
                            crate::code_rule_dsl::identifier::python_identifier(&name).into_owned()
                        }),
                        spread,
                        positional_index: position,
                        after_positional_spread: starred,
                    });
                    starred |= spread == Spread::Positional;
                }
            }
        }

        out.calls.push(CallArguments {
            call: NodeRef::of(*doc_id, call),
            arguments: start..out.arguments.len(),
        });
    }
    out.counters.argument_rows = out.arguments.len();
    out
}

// tests: a test the engine hosts (it reads above this crate; tools/cratify/test-hosts.tsv)
