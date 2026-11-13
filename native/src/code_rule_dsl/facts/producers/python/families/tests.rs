use super::*;
use crate::code_rule_dsl::facts::core;
use crate::code_rule_dsl::facts::producers::python::call_batch::{collect, collect_arguments};
use crate::code_rule_dsl::facts::producers::python::python_structure::{ContextKind, Spread};
use crate::code_rule_dsl::language::{GrammarDialect, tape_tag};
use crate::code_rule_dsl::lines::{LineTable, Terminators};
use crate::code_rule_dsl::tape::{Grammar as TapeGrammar, TapeDoc, TapeImage};
use ast_grep_core::{AstGrep, tree_sitter::StrDoc};
use std::path::Path;
use std::sync::Arc;

fn tape(source: &str) -> AstGrep<TapeDoc> {
    let grammar = TapeGrammar::new(GrammarDialect::Python).unwrap();
    let parsed = StrDoc::new(source, GrammarDialect::Python);
    let image = TapeImage::encode(parsed.tree.root_node(), &grammar, source, &|| false).unwrap();
    AstGrep::doc(TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap())
}

type Arg = (u32, u32, u32, Option<String>, u32, u32);

/// S's producers' rows for `source`, in the families' layout (keywords as their names).
fn oracle(root: &AstGrep<TapeDoc>) -> (Rows, Vec<Arg>) {
    let roots = vec![(0, root.root())];
    let mut out = Rows::default();
    let mut args = Vec::new();
    let order = [ContextKind::Module, ContextKind::SyncFunction, ContextKind::AsyncFunction, ContextKind::Class, ContextKind::Lambda];
    for (kind, wanted) in order.into_iter().enumerate() {
        let found = collect(&roots, wanted, |_, _| {}, |_, _| {});
        let id = |n: usize| n as u32;
        for call in &found.calls {
            let owner = found.contexts[call.context as usize].owner.map_or(NONE, |o| id(o.node_id));
            out.call.push([id(call.call.node_id), id(call.callee.node_id)]);
            out.site.push([id(call.report_node.node_id), id(call.call.node_id), call.ref_node.map_or(NONE, |r| id(r.node_id))]);
            out.context.push([id(call.call.node_id), owner, kind as u32]);
        }
        out.attribute.extend(found.attributes.iter().map(|a| [id(a.node.node_id), id(a.object.node_id), id(a.name_node.node_id)]));
        let top = root.root();
        let demand: Vec<_> = found.calls.iter().map(|c| (0, top.root().adopt(top.get_doc().node(c.call.node_id as u32)))).collect();
        let batch = collect_arguments(&demand);
        for a in &batch.arguments {
            let spread = match a.spread {
                Spread::None => 0,
                Spread::Positional => 1,
                Spread::Keywords => 2,
            };
            args.push((id(batch.calls[a.call].call.node_id), id(a.node.node_id), a.ordinal as u32, a.keyword.clone(), spread, id(a.value.node_id)));
        }
    }
    (out, args)
}

fn sorted<T: Ord + Clone>(rows: &[T]) -> Vec<T> {
    let mut rows = rows.to_vec();
    rows.sort();
    rows
}

/// The families' rows equal S's producers' on `source`.
fn check(source: &str, what: &str) -> usize {
    let root = tape(source);
    let facts = core::from_tape(root.root().get_doc(), Arc::new(LineTable::new(source, Terminators::of(GrammarDialect::Python)))).unwrap();
    let grammar = Grammar::new(tape_tag(GrammarDialect::Python).unwrap(), &GrammarDialect::Python.kinds());
    let rows = rows(&grammar, &facts);
    let basic=structure(&grammar,&facts);
    assert!(basic.arg.is_empty(),"call structure must not extract arguments");
    assert_eq!(basic.call,rows.call);
    let selected:Vec<_>=rows.call.iter().take(2).map(|r|r[0]).collect();
    assert_eq!(arguments(&grammar,&facts,&selected),rows.arg.iter().copied().filter(|r|selected.contains(&r[0])).collect::<Vec<_>>());
    assert!(arguments(&grammar,&facts,&[]).is_empty());
    let (expected, args) = oracle(&root);
    assert_eq!(sorted(&rows.call), sorted(&expected.call), "{what}: py_call");
    assert_eq!(sorted(&rows.site), sorted(&expected.site), "{what}: py_call_site");
    assert_eq!(sorted(&rows.context), sorted(&expected.context), "{what}: py_exec_context");
    assert_eq!(sorted(&rows.attribute), sorted(&expected.attribute), "{what}: py_attribute");
    let name = |n: u32| (n != NONE).then(|| crate::code_rule_dsl::identifier::python_name(facts.text(n).into()).into_owned());
    let actual: Vec<Arg> = rows.arg.iter().map(|&[c, a, o, k, s, v]| (c, a, o, name(k), s, v)).collect();
    assert_eq!(sorted(&actual), sorted(&args), "{what}: py_call_arg");
    rows.call.len()
}

#[test]
fn py_families_equal_s_producers() {
    let sources = [
        "\noutside()\nasync def outer():\n    ((\\\n      pkg.wait\\\n    ))(1, *items, trailing(), mode=(os.P_WAIT), **kw, K=2)\n    @decorate(header())\n    def sync(x=header_default(), y: annotation()=0):\n        sync_only()\n    class C(base()):\n        class_only()\n        async def method(x=class_default()):\n            nested_async()\n    thunk = lambda x=default(): lambda_only()\n    def inner(x=lambda y=lambda_default(): lambda_body()):\n        inner_body()\n    consume(generated() for x in iterable())\n    factory()()\n",
        "\nasync def f():\n    (\n       pkg.open # adjacent comment\n    )()\n    consume(x for x in source())\n    g(a, # note\n      *b, **c)\n",
    ];
    let mut calls = 0;
    for (at, source) in sources.iter().enumerate() {
        calls += check(source, &format!("source {at}"));
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures/python");
    let mut stack = vec![root];
    let mut files = 0;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "py") {
                if let Ok(source) = std::fs::read_to_string(&path) {
                    calls += check(&source, &path.display().to_string());
                    files += 1;
                }
            }
        }
    }
    assert!(files > 100 && calls > 1000, "{files} files, {calls} calls");
}
