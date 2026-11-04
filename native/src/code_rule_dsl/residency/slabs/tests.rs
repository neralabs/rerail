use super::*;
use crate::code_rule_dsl::language::{self, GrammarDialect};
use crate::code_rule_dsl::prefilter::tests::{document, temp};
use crate::code_rule_dsl::tape::TAPE_FORMAT_VERSION;

const SOURCES: [(&str, GrammarDialect); 3] = [
    ("value = left + right\nclass A:\n    def f(self, x=[]):\n        return (yield x)\n", GrammarDialect::Python),
    ("const f = (a, b) => { return a?.b ?? `t${b}`; };\n// c\n", GrammarDialect::JavaScript),
    ("def broken(:\n    pass\n", GrammarDialect::Python),
];

fn grammar(id: i64) -> Result<Arc<Grammar>, String> {
    let lang = u8::try_from(id).ok().and_then(language::from_tape_tag).ok_or("unknown grammar")?;
    Grammar::new(lang)
}

fn u32s(document: &Resident, offset: u32, count: usize) -> Vec<u32> {
    document.bytes_at(offset, 4 * count).chunks_exact(4).map(|word| u32::from_le_bytes(word.try_into().unwrap())).collect()
}

/// The residency is process-wide: the tests that admit and drop take turns. Other tests' label
/// runs admit their batches alongside, so counts are at least this test's.
static SERIAL: Mutex<()> = Mutex::new(());

#[test]
fn stored_tapes_are_admitted_into_slabs_and_read_in_place() {
    if metal::Device::system_default().is_none() {
        return;
    }
    let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    drop_all();
    let path = temp();
    let store = ContentStore::open(&path).unwrap();
    let mut digests = Vec::new();
    let mut tapes = Vec::new();
    for (source, lang) in SOURCES {
        let (tape, bytes) = document(source, lang);
        let source_id = store.put_source(source.as_bytes(), &[]).unwrap();
        store.put_tape(source_id, i64::from(language::tape_tag(lang).unwrap()), 1, i64::from(TAPE_FORMAT_VERSION), &bytes).unwrap();
        digests.push(<[u8; 32]>::from(Sha256::digest(&bytes)));
        tapes.push((tape, bytes, source));
    }
    let documents = admit(&store, &digests, &grammar).unwrap();
    assert!(held().documents >= SOURCES.len());
    for (document, (tape, bytes, source)) in documents.iter().zip(&tapes) {
        // The image is the stored bytes, the source the file's.
        assert_eq!(document.image(), bytes.as_slice());
        assert_eq!(document.bytes_at(document.view.source, document.view.source_len as usize), source.as_bytes());
        for offset in [document.view.code, document.view.size, document.view.base, document.view.off, document.view.tail, document.view.parent, document.view.codes, document.view.source, document.view.lines] {
            assert_eq!(offset % 16, 0, "every part at 16 bytes in its slab");
        }
        let nodes = document.view.nodes as usize;
        assert_eq!(nodes, tape.transducer_len());
        let parents = u32s(document, document.view.parent, nodes);
        for node in 0..nodes as u32 {
            assert_eq!((parents[node as usize] != u32::MAX).then_some(parents[node as usize]), tape.transducer_parent(node));
        }
        let lines = u32s(document, document.view.lines, document.view.line_count as usize);
        let expected = std::iter::once(0).chain(source.bytes().enumerate().filter(|(_, byte)| *byte == b'\n').map(|(at, _)| at as u32 + 1)).collect::<Vec<_>>();
        assert_eq!(lines, expected);
    }
    // Admitted again: the same documents, read from the residency, not the store.
    let again = admit(&store, &digests, &grammar).unwrap();
    assert!(again.iter().zip(&documents).all(|(a, b)| Arc::ptr_eq(a, b)));
    // One table over them: one slab, the documents numbered one after another.
    let table = DocTable::new(again).unwrap();
    assert_eq!(table.slabs.len(), 1);
    assert_eq!(table.views.iter().map(|view| view.node_base).collect::<Vec<_>>(), {
        let mut base = 0;
        tapes
            .iter()
            .map(|(tape, _, _)| {
                let at = base;
                base += tape.transducer_len() as u32;
                at
            })
            .collect::<Vec<_>>()
    });
    // A tape the store does not hold is refused, not an empty document.
    assert!(admit(&store, &[[7; 32]], &grammar).is_err());
    // The drop releases every document (other tests may admit the same tapes again at once, so
    // that is not asserted here); a batch's leases keep its slab's bytes readable.
    drop_all();
    assert_eq!(documents[0].image(), tapes[0].1.as_slice());
    drop(documents);
    drop(table);
    drop(store);
    let _ = std::fs::remove_file(path);
}
