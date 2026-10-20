//! The `node` table against the tape's own walks, node by node: every axis the lowering joins on
//! (children, descendants, ancestors, the byte-positioned `next`, `next_all`/`prev_all` from the
//! walk anchor, kind postings, named positions, first field children) must list exactly the nodes
//! the matcher's walks visit, in their order.

use super::differential::tape;
use crate::code_rule_dsl::language::dialect_by_label;
use crate::code_rule_dsl::matcher::node_at;
use crate::code_rule_dsl::sql::exec::with_nodes;
use rusqlite::params;

const SOURCES: &[(&str, &str)] = &[
    (
        "typescript",
        "g(x);\nh(y, z,);\nclass A { m() { return [1, 2]; } }\n",
    ),
    (
        "typescript",
        "let q = [1, 2;\nlet r = 3;\nif (a { b(); }\nc(\n",
    ),
    (
        "python",
        "def f(a, b=1):\n    return [a for a in b]\nprint(a\nx = {1: 2,\n",
    ),
    (
        "php",
        "<?php\nfunction f($a) { return $a + ; }\n$b = [1, 2\n",
    ),
    (
        "blade",
        "<div>{{ $a }}</div>\n@if ($b)\n  <p>{{ $c }}</p>\n@endif\n<x-y :a=\"$d\" />\n",
    ),
];

fn ids<'t>(
    nodes: impl Iterator<Item = ast_grep_core::Node<'t, crate::code_rule_dsl::tape::TapeDoc>>,
) -> Vec<i64> {
    nodes.map(|node| node.node_id() as i64).collect()
}

#[test]
fn every_axis_lists_the_nodes_the_tape_walks_visit() {
    let docs: Vec<_> = SOURCES
        .iter()
        .map(|(label, source)| tape(dialect_by_label(label).expect("dialect"), source))
        .collect();
    let zero_width = with_nodes(docs, |connection, batch| {
        let query = |sql: &str, file: usize, node: u32| -> Vec<i64> {
            let mut statement = connection.prepare_cached(sql).expect("prepares");
            statement
                .query_map(params![file as i64, i64::from(node)], |row| {
                    row.get::<_, Option<i64>>(0)
                })
                .expect("runs")
                .filter_map(|row| row.expect("row"))
                .collect()
        };
        let mut zero_width = 0;
        for (file, entry) in batch.files.iter().enumerate() {
            let root = &entry.root;
            let tape = root.root().get_doc();
            for id in 0..tape.transducer_len() as u32 {
                let node = node_at(root, id);
                let at = |sql: &str| query(sql, file, id);
                if node.range().is_empty() {
                    zero_width += 1;
                }
                assert_eq!(
                    at("SELECT c.id FROM node c WHERE c.file = ?1 AND c.parent = ?2 ORDER BY c.id"),
                    ids(node.children()),
                    "children of {id} in {file}"
                );
                assert_eq!(
                    at(
                        "SELECT c.id FROM node fn JOIN node c ON c.file = fn.file AND c.id > fn.id AND c.id <= fn.\"last\" \
                        WHERE fn.file = ?1 AND fn.id = ?2 ORDER BY c.id"
                    ),
                    ids(node.dfs().skip(1)),
                    "descendants of {id} in {file}"
                );
                assert_eq!(
                    at(
                        "SELECT c.id FROM node c WHERE c.file = ?1 AND c.id < ?2 AND c.\"last\" >= ?2 ORDER BY c.id DESC"
                    ),
                    ids(node.ancestors()),
                    "ancestors of {id} in {file}"
                );
                assert_eq!(
                    at("SELECT next FROM node WHERE file = ?1 AND id = ?2"),
                    ids(node.next().into_iter()),
                    "next of {id} in {file}"
                );
                assert_eq!(
                    at("SELECT prev FROM node WHERE file = ?1 AND id = ?2"),
                    ids(node.prev().into_iter()),
                    "prev of {id} in {file}"
                );
                assert_eq!(
                    at(
                        "SELECT c.id FROM node fn JOIN node c ON c.file = fn.file AND c.parent = fn.wpar AND c.id >= fn.anext \
                        WHERE fn.file = ?1 AND fn.id = ?2 ORDER BY c.id"
                    ),
                    ids(node.next_all()),
                    "next_all of {id} in {file}"
                );
                assert_eq!(
                    at(
                        "SELECT c.id FROM node fn JOIN node c ON c.file = fn.file AND c.parent = fn.wpar AND c.id <= fn.aprev \
                        WHERE fn.file = ?1 AND fn.id = ?2 ORDER BY c.id DESC"
                    ),
                    ids(node.prev_all()),
                    "prev_all of {id} in {file}"
                );
                let named: Vec<_> = node
                    .parent()
                    .map(|parent| ids(parent.children().filter(|child| child.is_named())))
                    .unwrap_or_default();
                let expected = named
                    .iter()
                    .position(|&child| child == i64::from(id))
                    .map(|index| vec![index as i64 + 1, named.len() as i64]);
                let position = at(
                    "SELECT named_ix FROM node WHERE file = ?1 AND id = ?2 AND named_ix IS NOT NULL \
                                   UNION ALL SELECT named_n FROM node WHERE file = ?1 AND id = ?2 AND named_ix IS NOT NULL",
                );
                assert_eq!(
                    position,
                    expected.unwrap_or_default(),
                    "named position of {id} in {file}"
                );
                let field = tape.transducer_field(id);
                let first = node
                    .parent()
                    .and_then(|parent| parent.child_by_field_id(field))
                    .is_some_and(|child| field != 0 && child.node_id() == id as usize);
                assert_eq!(
                    at("SELECT ffirst FROM node WHERE file = ?1 AND id = ?2"),
                    vec![i64::from(first)],
                    "first field child {id} in {file}"
                );
            }
            // Kind postings, one kind and several (`IN`, passed whole).
            let kinds: Vec<u16> = (0..tape.transducer_len() as u32)
                .map(|id| tape.transducer_kind(id))
                .take(3)
                .collect();
            let expected: Vec<i64> = (0..tape.transducer_len() as u32)
                .filter(|&id| kinds.contains(&tape.transducer_kind(id)))
                .map(i64::from)
                .collect();
            let listed = kinds
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            assert_eq!(
                query(
                    &format!(
                        "SELECT id FROM node WHERE file = ?1 AND ?2 = ?2 AND kind IN ({listed}) ORDER BY id"
                    ),
                    file,
                    0
                ),
                expected,
                "kinds {kinds:?} in {file}"
            );
        }
        zero_width
    });
    assert!(zero_width > 0, "the sources hold zero-width nodes");
}

#[test]
fn value_postings_intersect_kind_and_structural_ranges() {
    let dialect = dialect_by_label("typescript").unwrap();
    with_nodes(
        vec![tape(dialect, "f(a, b, a); g(a); a;\n")],
        |connection, batch| {
            let facts = &batch.files[0].facts;
            for needle in 0..facts.len() {
                let kind = facts.kind(needle);
                let hash = facts.thash(needle);
                for lo in 0..facts.len() {
                    let hi = (lo + 7).min(facts.len());
                    let expected: Vec<u32> = (lo..hi)
                        .filter(|&id| facts.kind(id) == kind && facts.thash(id) == hash)
                        .collect();
                    let mut statement = connection.prepare_cached(
                    "SELECT id FROM node WHERE file=0 AND kind=?1 AND thash=?2 AND id>=?3 AND id<?4 ORDER BY id"
                ).unwrap();
                    let got: Vec<u32> = statement
                        .query_map(params![kind, hash, lo, hi], |row| row.get(0))
                        .unwrap()
                        .map(Result::unwrap)
                        .collect();
                    assert_eq!(got, expected, "needle {needle}, range {lo}..{hi}");
                }
            }
        },
    );
}
