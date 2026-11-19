    use super::super::python_structure;
    use super::*;
    use crate::code_rule_dsl::{
        language::GrammarDialect,
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
    #[allow(clippy::type_complexity)]
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
            let mut actual = collect(
                &roots,
                wanted,
                |doc, node| {
                    assert_eq!(doc, 9);
                    assert!(seen.insert(node.node_id()));
                },
                |_, _| {},
            );
            assert!(actual.arguments.is_empty());
            assert!(actual.calls.iter().all(|call| call.arguments.is_empty()));
            let demand: Vec<_> = actual
                .calls
                .iter()
                .map(|call| (call.call.doc, at(&roots[0].1, call.call.node_id as u32)))
                .collect();
            let batch = collect_arguments(&demand);
            assert_eq!(batch.counters.distinct_calls, actual.calls.len());
            actual.arguments = batch.arguments;
            for (call, rows) in actual.calls.iter_mut().zip(batch.calls) {
                assert_eq!(call.call, rows.call);
                call.arguments = rows.arguments;
            }
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
    fn imports_are_decoded_only_for_documents_with_selected_calls() {
        let inactive = tape("import requests\ndef f():\n requests.get('x')\n");
        let active = tape("async def f():\n requests.get('x')\nimport requests\n");
        let mut imports = Vec::new();
        let facts = collect(
            &[(0, inactive.root()), (1, active.root())],
            ContextKind::AsyncFunction,
            |_, _| {},
            |doc, node| {
                imports.push((doc, node.text().into_owned()));
            },
        );
        assert_eq!(facts.calls.len(), 1);
        assert_eq!(imports, vec![(1, "import requests".into())]);
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
            |_, _| {},
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
        assert!(facts.arguments.is_empty());
        assert!(generator.arguments.is_empty());
        let batch = collect_arguments(&[(2, at(&root.root(), generator.call.node_id as u32))]);
        let args = &batch.arguments[batch.calls[0].arguments.clone()];
        assert_eq!(args.len(), 1);
        assert_eq!(args[0].positional_index, Some(0));
        assert_eq!(
            at(&root.root(), args[0].value.node_id as u32).kind(),
            "generator_expression"
        );
    }
    #[test]
    fn argument_demands_are_empty_or_deduplicated_without_eager_observation() {
        let empty = collect_arguments(&[]);
        assert!(empty.calls.is_empty() && empty.arguments.is_empty());
        assert_eq!(
            (
                empty.counters.requested_calls,
                empty.counters.distinct_calls,
                empty.counters.argument_rows
            ),
            (0, 0, 0)
        );
        let root = tape("async def f():\n target(123, mode=(value), *xs)\n");
        let mut observed = FxHashSet::default();
        let facts = collect(
            &[(0, root.root())],
            ContextKind::AsyncFunction,
            |_, node| {
                observed.insert(node.node_id());
            },
            |_, _| {},
        );
        assert_eq!(facts.calls.len(), 1);
        assert!(facts.arguments.is_empty() && facts.calls[0].arguments.is_empty());
        let call = at(&root.root(), facts.calls[0].call.node_id as u32);
        let batch = collect_arguments(&[(0, call.clone()), (0, call.clone()), (1, call)]);
        assert_eq!(
            (
                batch.counters.requested_calls,
                batch.counters.distinct_calls,
                batch.counters.argument_rows
            ),
            (3, 2, 6)
        );
        assert_eq!(
            batch
                .calls
                .iter()
                .map(|call| call.call.doc)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
        assert!(
            !observed.contains(&batch.arguments[0].node.node_id),
            "call collection must not observe argument literals"
        );
        for (index, call) in batch.calls.iter().enumerate() {
            let args = &batch.arguments[call.arguments.clone()];
            assert_eq!(args.len(), 3);
            assert!(
                args.iter()
                    .all(|arg| arg.call == index && arg.node.doc == call.call.doc)
            );
            assert_eq!(args[0].positional_index, Some(0));
            assert_eq!(args[1].keyword.as_deref(), Some("mode"));
            assert_eq!(args[2].spread, Spread::Positional);
        }
    }
    #[test]
    #[should_panic(expected = "argument demands reused a document ID for different roots")]
    fn argument_demand_identity_is_checked_before_duplicate_ordinals() {
        let first = tape("f()\n");
        let second = tape("f()\n");
        let first_call = first
            .root()
            .dfs()
            .find(|node| node.kind() == "call")
            .unwrap();
        let second_call = second
            .root()
            .dfs()
            .find(|node| node.kind() == "call")
            .unwrap();
        assert_eq!(first_call.node_id(), second_call.node_id());
        collect_arguments(&[(7, first_call), (7, second_call)]);
    }
