//! Typed producer sharing and reverse owner postings. Only the leaf-input `$X` terminal's
//! semantic equivalence permits atom keys here. Arbitrary capture tuples must retain identity.

use super::{
    measured,
    packed::{PackedCapture, native},
};
use crate::code_rule_dsl::sql::tests::differential::tape;
use crate::code_rule_dsl::{language::GrammarDialect, matcher::node_at};
use ast_grep_core::{AstGrep, Pattern};
use std::collections::HashMap;

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
struct Key {
    term: u32,
    file: u32,
    node: u32,
    atom: Option<u32>,
}

#[test]
fn atom_producers_preserve_the_candidate_occurrence_and_file_identity() {
    let doc = tape(GrammarDialect::Python, "f(a, a, b)\ng(a)\n");
    let root = AstGrep::doc(doc.clone());
    let pattern = Pattern::try_new("$X", GrammarDialect::Python).unwrap();
    let mut atoms = HashMap::new();
    let mut cache = HashMap::new();
    let bounds: Vec<_> = (0..doc.transducer_len() as u32)
        .filter(|&id| node_at(&root, id).is_named_leaf())
        .collect();
    let program = PackedCapture;
    for file in [0, 1] {
        for &bound in &bounds {
            let next = atoms.len() as u32;
            let atom = *atoms.entry(doc.transducer_text_str(bound)).or_insert(next);
            for node in 0..doc.transducer_len() as u32 {
                let key = Key {
                    term: 0,
                    file,
                    node,
                    atom: Some(atom),
                };
                let actual = *cache.entry(key).or_insert_with(|| {
                    program.run(&doc, node, Some(doc.transducer_text_str(bound)))
                });
                assert_eq!(actual, native(&root, &pattern, node, Some(bound)));
            }
        }
    }
    assert_eq!(cache.len(), 2 * doc.transducer_len() * atoms.len());
}

/// One preorder pass; each node belongs to its nearest owner of the requested kind.
fn owner_postings(
    doc: &crate::code_rule_dsl::tape::TapeDoc,
    owner_kind: u16,
) -> (Vec<Option<u32>>, HashMap<u32, Vec<u32>>) {
    let mut owners = vec![None; doc.transducer_len()];
    let mut postings: HashMap<u32, Vec<u32>> = HashMap::new();
    for node in 0..doc.transducer_len() as u32 {
        let parent = doc.transducer_links(node)[0];
        let owner = if doc.transducer_kind(node) == owner_kind {
            Some(node)
        } else if parent != u32::MAX {
            owners[parent as usize]
        } else {
            None
        };
        owners[node as usize] = owner;
        if let Some(owner) = owner {
            postings.entry(owner).or_default().push(node);
        }
    }
    (owners, postings)
}

#[test]
fn owner_reverse_index_matches_a_full_owner_scan_with_nested_scopes() {
    let doc = tape(
        GrammarDialect::Python,
        "x = 1\ndef outer():\n    def inner():\n        return x\n    return x\ndef other():\n    pass\n",
    );
    let root = AstGrep::doc(doc.clone());
    let kind = root
        .root()
        .dfs()
        .find(|n| n.kind() == "function_definition")
        .unwrap()
        .kind_id();
    let (owners, postings) = owner_postings(&doc, kind);
    for node in root.root().dfs() {
        let expected = std::iter::once(node.clone())
            .chain(node.ancestors())
            .find(|n| n.kind_id() == kind)
            .map(|n| n.node_id() as u32);
        assert_eq!(owners[node.node_id()], expected);
    }
    for (owner, actual) in postings {
        let expected: Vec<_> = owners
            .iter()
            .enumerate()
            .filter(|(_, value)| **value == Some(owner))
            .map(|(node, _)| node as u32)
            .collect();
        assert_eq!(actual, expected);
    }
}

#[test]
#[ignore = "representation measurement; run with --test-threads=1 --nocapture"]
fn measure_owner_reverse_postings() {
    let source = (0..2000)
        .map(|index| format!("def f{index}():\n    return x + y\n"))
        .collect::<String>();
    let doc = tape(GrammarDialect::Python, &source);
    let root = AstGrep::doc(doc.clone());
    let kind = root
        .root()
        .dfs()
        .find(|n| n.kind() == "function_definition")
        .unwrap()
        .kind_id();
    let ((owners, postings), build_ms) = measured(|| owner_postings(&doc, kind));
    let keys: Vec<_> = postings.keys().copied().collect();
    let (scanned, scan_ms) = measured(|| {
        keys.iter()
            .map(|owner| {
                owners
                    .iter()
                    .filter(|value| **value == Some(*owner))
                    .count()
            })
            .collect::<Vec<_>>()
    });
    let (indexed, index_ms) = measured(|| {
        keys.iter()
            .map(|owner| postings[owner].len())
            .collect::<Vec<_>>()
    });
    assert_eq!(scanned, indexed);
    eprintln!(
        "SPIKE {}",
        serde_json::json!({"name":"owner-reverse-postings","nodes":owners.len(),"owners":keys.len(),
        "buildMs":build_ms,"scanMs":scan_ms,"indexMs":index_ms,"scanVisits":owners.len()*keys.len(),
        "postingsEntries":indexed.iter().sum::<usize>(),"completeOwnerParity":true})
    );
}

#[test]
#[ignore = "representation measurement; run with --test-threads=1 --nocapture"]
fn measure_capture_aware_producer_sharing() {
    let doc = tape(GrammarDialect::Python, &"f(a, b, a)\n".repeat(100));
    let root = AstGrep::doc(doc.clone());
    let pattern = Pattern::try_new("$X", GrammarDialect::Python).unwrap();
    let bounds: Vec<_> = (0..doc.transducer_len() as u32)
        .filter(|&id| node_at(&root, id).is_named_leaf() && doc.transducer_text_str(id) == "a")
        .collect();
    let requests: Vec<_> = (0..100_000)
        .map(|index| {
            let node = (index % doc.transducer_len()) as u32;
            (
                Key {
                    term: 0,
                    file: 0,
                    node,
                    atom: Some(0),
                },
                bounds[index % bounds.len()],
            )
        })
        .collect();
    let (oracle, fresh_ms) = measured(|| {
        requests
            .iter()
            .map(|(key, bound)| native(&root, &pattern, key.node, Some(*bound)))
            .collect::<Vec<_>>()
    });
    let mut producers = HashMap::new();
    let (shared, shared_ms) = measured(|| {
        requests
            .iter()
            .map(|(key, bound)| {
                *producers
                    .entry(*key)
                    .or_insert_with(|| native(&root, &pattern, key.node, Some(*bound)))
            })
            .collect::<Vec<_>>()
    });
    assert_eq!(oracle, shared);
    eprintln!(
        "SPIKE {}",
        serde_json::json!({"name":"capture-producer-sharing","requests":requests.len(),
        "producerEvaluations":producers.len(),"freshMs":fresh_ms,"sharedMs":shared_ms,"completeCaptureParity":true})
    );
}
