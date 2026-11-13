//! Tests of this module that read what the engine registers as it loads (the languages, their `.rr` libraries,
//! the ECMA crate's functions): the engine's facade of the module mounts them.

#[test]
fn pruning_dead_native_producers_removes_their_dependency_edges() {
    let p = super::super::super::rr::parse(
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
    assert!(!p.natives.is_empty());
    let pruned = super::super::super::front::prune(p);
    assert!(
        pruned.natives.is_empty(),
        "dead native edges must not create cross-file exports"
    );
    assert!(
        !super::super::super::front::needed(&pruned, pruned.outputs.clone()).contains("wanted")
    );
}
