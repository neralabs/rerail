//! `nera_tree` over P's `file` table, built from a tape's index image: children and ancestors
//! read through incremental BLOB reads agree with the tape's own navigation, overflow columns
//! included.

use super::super::reader::{Column, Readers};
use super::{ANCESTORS, CHILDREN, install};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::prefilter::tests::document;
use rusqlite::{Connection, params};

/// P's `file` table (`matcher/anchors/sql/schema.sql`), which the reader opens BLOBs in.
const FILE: &str = "CREATE TABLE file (
    fid INTEGER PRIMARY KEY, nodes INTEGER NOT NULL, path TEXT NOT NULL UNIQUE,
    digest BLOB NOT NULL, lang INTEGER NOT NULL, srcenc TEXT NOT NULL DEFAULT 'UTF8', src BLOB NOT NULL,
    rec BLOB NOT NULL, spans BLOB NOT NULL, checkpoints BLOB NOT NULL,
    parb BLOB NOT NULL, sizb BLOB NOT NULL, flags BLOB NOT NULL
)";

#[test]
fn children_ancestors_and_spans_match_the_tape() {
    // More than 65,535 nodes below the root, so sizes and parent deltas overflow to `sizb`/`parb`.
    let source = format!("def f():\n    return [{}]\n", "x, ".repeat(40_000));
    let (doc, _) = document(&source, GrammarDialect::Python);
    let image = doc.index_image(&|| false).unwrap();
    assert!(!image.sizb.is_empty() && !image.parb.is_empty());
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch(FILE).unwrap();
    db.execute(
        "INSERT INTO file(fid,nodes,path,digest,lang,src,rec,spans,checkpoints,parb,sizb,flags) \
         VALUES(1,?1,'a.py',x'00',0,?2,?3,?4,?5,?6,?7,?8)",
        params![
            doc.transducer_len() as i64,
            source.as_bytes(),
            image.rec,
            image.spans,
            image.checkpoints,
            image.parb,
            image.sizb,
            image.flags
        ],
    )
    .unwrap();
    let readers = Readers::new(&db);
    install(&db, readers.0.clone()).unwrap();
    let mut query = db.prepare("SELECT pre FROM nera_tree(1,?1,?2)").unwrap();
    let mut walk = |node: u32, axis: i64| -> Vec<u32> {
        query.query_map(params![node, axis], |row| row.get(0)).unwrap().map(Result::unwrap).collect()
    };
    let nodes = doc.transducer_len() as u32;
    for node in (0..nodes).step_by(997).chain([0, 1, nodes - 1]) {
        assert_eq!(walk(node, CHILDREN), doc.transducer_children(node).collect::<Vec<_>>(), "{node}");
        let ancestors = std::iter::successors(doc.transducer_parent(node), |&n| doc.transducer_parent(n)).collect::<Vec<_>>();
        assert_eq!(walk(node, ANCESTORS), ancestors, "{node}");
        assert_eq!(readers.0.lock().span(1, node as usize).unwrap(), doc.transducer_span(node), "{node}");
    }
    assert!(readers.0.lock().value(1, Column::Size, 0, u16::MAX).unwrap() as usize == doc.transducer_len());
}
