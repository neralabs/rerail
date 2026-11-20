//! Tests of this module that read what the engine registers as it loads (the languages, their `.rr` libraries,
//! the ECMA crate's functions): the engine's facade of the module mounts them.

#[test]
fn pruning_dead_native_producers_removes_their_dependency_edges() {
    let p = super::super::super::rr::parse_version(
        r#"
#include "core.rr"
.decl wanted(n:node)
wanted(n) :- node(_,n,_,_,_).
.decl targets(q:symbol)
targets("requests.get").
.decl unused(n:node,q:symbol)
unused(n,q) :- python_qualified(n,q,wanted,targets).
.decl result(n:number)
result(1).
.output result
"#,
    )
    .unwrap();
    assert!(!p.native_map().unwrap().is_empty());
    let mut pipeline = crate::code_rule_dsl::logic::program_image::manager::Pipeline::new("test", p);
    pipeline.program(&mut crate::code_rule_dsl::logic::plan::corpus::preparation::prune::Prune::default());
    let (pruned, _) = pipeline.finish().unwrap();
    assert!(pruned.native_map().unwrap().is_empty(), "dead native edges must not create cross-file exports");
    assert!(pruned.rel("wanted").is_none_or(|r| pruned.heads(r).next().is_none()), "`wanted` is needed by no output: {}", pruned.show());
}
