use super::*;
use crate::code_rule_dsl::language::{self, GrammarDialect};
use crate::code_rule_dsl::fixtures::{document, temp};
use crate::code_rule_dsl::tape::TAPE_FORMAT_VERSION;
// What the slabs module reads through its own imports, which its facade in the engine does not re-export.
use crate::code_rule_dsl::tape::{Grammar, resident};
use crate::content_store::ContentStore;
use sha2::{Digest as _, Sha256};
use std::sync::Arc;

const SOURCES: [(&str, GrammarDialect); 3] = [
    ("value = left + right\nclass A:\n    def f(self, x=[]):\n        return (yield x)\n", GrammarDialect::Python),
    ("const f = (a, b) => { return a?.b ?? `t${b}`; };\n// c\n", GrammarDialect::JavaScript),
    ("def broken(:\n    pass\n", GrammarDialect::Python),
];

fn grammar(id: i64) -> Result<Arc<Grammar>, String> {
    let lang = u8::try_from(id).ok().and_then(language::from_tape_tag).ok_or("unknown grammar")?;
    Grammar::new(lang)
}

#[allow(dead_code)]
fn u32s(document: &Resident, offset: u32, count: usize) -> Vec<u32> {
    document.bytes_at(offset, 4 * count).chunks_exact(4).map(|word| u32::from_le_bytes(word.try_into().unwrap())).collect()
}

use std::sync::Mutex;

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
        // The device reads the stored image in place.
        let (tape, stored) = document(source, lang);
        assert!(resident::layout(&stored).is_ok(), "a stored image is read in place");
        let bytes = tape.resident_parts(None).unwrap().0;
        let source_id = store.put_source(source.as_bytes(), &[]).unwrap();
        store.put_tape(source_id, i64::from(language::tape_tag(lang).unwrap()), 1, i64::from(TAPE_FORMAT_VERSION), &bytes).unwrap();
        digests.push(crate::code_rule_dsl::tape::tape_identity(&Sha256::digest(source.as_bytes()).into(), &bytes));
        tapes.push((tape, bytes, source));
    }
    let documents = admit(&store, &digests, &grammar).unwrap();
    assert!(held().documents >= SOURCES.len());
    for (document, (tape, bytes, source)) in documents.iter().zip(&tapes) {
        // The image is the stored bytes, the source the file's.
        assert_eq!(document.image(), bytes.as_slice());
        assert_eq!(document.bytes_at(document.view.source, document.view.source_len as usize), source.as_bytes());
        for offset in document.view.columns.into_iter().chain([document.view.codes, document.view.source, document.view.lines]) {
            assert_eq!(offset % 16, 0, "every part at 16 bytes in its slab");
        }
        assert_eq!(document.view.nodes as usize, tape.transducer_len());
        let lines = u32s(document, document.view.lines, document.view.line_count as usize);
        let expected = std::iter::once(0).chain(source.bytes().enumerate().filter(|(_, byte)| *byte == b'\n').map(|(at, _)| at as u32 + 1)).collect::<Vec<_>>();
        assert_eq!(lines, expected);
    }
    // Admitted again: the same documents, read from the residency, not the store.
    let again = admit(&store, &digests, &grammar).unwrap();
    assert!(again.iter().zip(&documents).all(|(a, b)| Arc::ptr_eq(a, b)));
    // One table over them: one slab, the documents numbered one after another.
    let table = DocTable::new(again).unwrap();
    assert!(table.block.is_some());
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

/// A parser's inputs resident as sources (decision 0067): each file's bytes at a 16-byte offset
/// of a slab, at least 16 bytes of room; asked again, the same sources, not copied again; at most
/// `MAX_SLABS` slabs bound for them.
#[test]
fn sources_are_admitted_once_and_read_in_place() {
    if metal::Device::system_default().is_none() {
        return;
    }
    let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let files: Vec<Vec<u8>> = vec![b"{\"a\": [1, 2]}".to_vec(), Vec::new(), vec![b'x'; 1000], b"{\"a\": [1, 2]}".to_vec()];
    let bytes: Vec<&[u8]> = files.iter().map(Vec::as_slice).collect();
    let first = admit_sources(&bytes).expect("admits");
    for (source, file) in first.iter().zip(&files) {
        assert_eq!(source.offset % 16, 0);
        assert_eq!(source.bytes(), &file[..]);
        assert!(source.offset + source.len.max(16) <= source.slab.byte_length());
    }
    // The same bytes are one source.
    assert!(Arc::ptr_eq(&first[0], &first[3]));
    let again = admit_sources(&bytes).expect("admits");
    for (a, b) in first.iter().zip(&again) {
        assert!(Arc::ptr_eq(a, b), "a resident source is not copied again");
    }
    let (slabs, at) = source_slabs(&again).expect("within MAX_SLABS");
    assert!(slabs.len() <= MAX_SLABS);
    for (source, &k) in again.iter().zip(&at) {
        assert_eq!(slabs[k as usize].id, source.slab.id);
    }
}

/// Decision 0289 §4: a batch reads one block. Documents admitted into two slabs (a source past a slab's size between
/// them) are copied into one, each read as before; the residency finds them there, so the next batch over them copies
/// nothing.
#[test]
fn a_batch_over_several_slabs_is_gathered_into_one_block() {
    if metal::Device::system_default().is_none() {
        return;
    }
    use crate::code_rule_dsl::tape::formats::Format;
    let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    for format in [Format::F15, Format::F16, Format::F16Z] {
        drop_all();
        let docs: Vec<crate::code_rule_dsl::tape::TapeDoc> = SOURCES
            .iter()
            .map(|&(source, lang)| {
                let native = ast_grep_core::tree_sitter::StrDoc::new(source, lang);
                let grammar = Grammar::new(lang).unwrap();
                crate::code_rule_dsl::tape::formats::as_run(format, || {
                    let image = crate::code_rule_dsl::tape::TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap().primary.in_format(format, source.as_bytes());
                    crate::code_rule_dsl::tape::TapeDoc::restore(Arc::new(source.to_owned()), grammar, &image, &|| false).unwrap()
                })
            })
            .collect();
        let first = admit_docs(&docs[..1]).unwrap();
        // A source larger than a slab takes one of its own; the next document goes to another.
        let big = vec![b'x'; SLAB_BYTES + (1 << 20)];
        let held_big = admit_sources(&[&big]).unwrap();
        let rest = admit_docs(&docs[1..]).unwrap();
        assert_ne!(first[0].slab.id, rest[0].slab.id, "the documents are in two slabs");
        let documents: Vec<Arc<Resident>> = first.iter().chain(&rest).cloned().collect();
        let table = DocTable::one_block(documents.clone()).unwrap();
        let block = table.block.as_ref().expect("one block bound");
        for (document, gathered) in documents.iter().zip(&table.documents) {
            assert_eq!(gathered.slab.id, block.id);
            assert_eq!(gathered.image(), document.image());
            let v = &gathered.view;
            assert_eq!(gathered.bytes_at(v.source, v.source_len as usize), document.bytes_at(document.view.source, document.view.source_len as usize));
            // Format 15's line starts are placed beside its tape; format 16's are its tape's, compared with the image.
            if v.format == 15 {
                assert_eq!(u32s(gathered, v.lines, v.line_count as usize), u32s(document, document.view.lines, document.view.line_count as usize));
            }
            assert_eq!(gathered.codes(), document.codes());
            // The view moved with the document: its offsets count from its first byte (format 16's codes are its
            // language's table, the block's: copied there once).
            let codes = |view: &TapeView| TapeView { codes: 0, ..*view };
            assert_eq!(codes(gathered.view.view()), codes(document.view.view()));
            assert_eq!(gathered.table, document.table);
            if let Some(table) = gathered.table {
                assert_eq!(Some(gathered.at.wrapping_add(v.codes as usize) as u32 as usize), block.table(table.key), "{format:?}: the block's table");
            }
        }
        // Asked again, the residency gives the gathered documents: one slab already, nothing copied.
        let again = admit_docs(&docs).unwrap();
        assert!(again.iter().all(|document| document.slab.id == block.id));
        let twice = gather(again.clone()).unwrap();
        assert!(twice.iter().zip(&again).all(|(a, b)| Arc::ptr_eq(a, b)));
        drop((table, held_big, twice, again));
        drop_all();
    }
}

/// The device reads a resident tape as its document reads it (decision 0293), in every format the run may store:
/// format 15's columns, format 16's arrays, and a 16z tape laid out as format 16 where it is placed, with no parent
/// column, child list or index of nodes by code beside them. Every node's code (kind, field, flags, classes), its
/// parent, next and next named siblings, children, subtree and span. One document of more than a side-list part
/// (1,024 nodes), with gaps and lengths past a byte, reads the side lists and the parts' starts.
#[test]
fn the_device_reads_every_format_as_the_document_does() {
    use crate::code_rule_dsl::tape::formats::Format;
    use crate::code_rule_dsl::transducer::packed::metal_backend::{MappedBuffer, pipeline_named};
    use metal::MTLSize;
    const TAPE: &str = concat!(include_str!("../../transducer/packed/metal_tape_reads.metal"), include_str!("../../transducer/packed/metal_tape.metal"));
    const FIELDS: usize = 14;
    const KERNEL: &str = r#"
inline uint tape_global(ChunkTape d,uint i){return i==TAPE_NONE?TAPE_NONE:d.node_base+i;}
kernel void tape_conformance(device const ChunkTape* docs [[buffer(0)]],device const uint* nodeDoc [[buffer(1)]],device uint* out [[buffer(2)]],device const uchar* s [[buffer(3)]],constant uint& count [[buffer(7)]],uint g [[thread_position_in_grid]]){
 if(g>=count)return;
 ChunkTape d=docs[nodeDoc[g]];
 uint i=g-d.node_base;
 device uint* o=out+g*14u;
 o[0]=tape_kind(s,d,i);o[1]=tape_field(s,d,i);o[2]=tape_flags(s,d,i);
 o[3]=tape_global(d,tape_parent(s,d,i));o[4]=tape_global(d,tape_next(s,d,i));o[5]=tape_global(d,tape_next_named(s,d,i));
 o[6]=tape_child_count(s,d,i);o[7]=d.node_base+i+tape_size(s,d,i);o[8]=tape_start(s,d,i);o[9]=tape_end(s,d,i);
 o[10]=tape_global(d,tape_first_child(s,d,i));o[11]=tape_code(s,d,i).classCount;
 uint line=tape_line(s,d,tape_start(s,d,i));o[12]=line;o[13]=tape_line_start(s,d,line);
}
"#;
    let Some(device) = metal::Device::system_default() else { return };
    let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let long = format!("# {}\n", "x".repeat(300))
        + &(0..300).map(|i| format!("v{i} = [{i}, \"{}\", f(a, b=c)]  # {}\n", "s".repeat(i % 40), "c".repeat(260 * usize::from(i % 50 == 0)))).collect::<String>();
    let mut sources = SOURCES.iter().map(|&(source, lang)| (source.to_owned(), lang)).collect::<Vec<_>>();
    sources.push((long, GrammarDialect::Python));
    for format in [Format::F15, Format::F16, Format::F16Z] {
        let docs = sources
            .iter()
            .map(|(source, lang)| {
                let native = ast_grep_core::tree_sitter::StrDoc::new(source, *lang);
                let grammar = Grammar::new(*lang).unwrap();
                // Made as a run of `format` makes it, whatever the process's run stores.
                crate::code_rule_dsl::tape::formats::as_run(format, || {
                    let image = crate::code_rule_dsl::tape::TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap().primary.in_format(format, source.as_bytes());
                    crate::code_rule_dsl::tape::TapeDoc::restore(Arc::new(source.clone()), grammar, &image, &|| false).unwrap()
                })
            })
            .collect::<Vec<_>>();
        assert!(docs.last().unwrap().transducer_len() > 2 * 1024, "a document of several parts");
        // Stored as the run stores them, and admitted from the store: each read from its frames.
        let path = temp();
        let store = ContentStore::open(&path).unwrap();
        let mut digests = Vec::new();
        for ((source, lang), doc) in sources.iter().zip(&docs) {
            let bytes = doc.resident_parts(None).unwrap().0;
            let source_id = store.put_source(source.as_bytes(), &[]).unwrap();
            store.put_tape(source_id, i64::from(language::tape_tag(*lang).unwrap()), 1, i64::from(format.store_version()), &bytes).unwrap();
            digests.push(crate::code_rule_dsl::tape::tape_identity(&Sha256::digest(source.as_bytes()).into(), &bytes));
        }
        let table = DocTable::one_block(admit(&store, &digests, &grammar).unwrap()).unwrap();
        let expected_format = if format == Format::F15 { 15 } else { 16 };
        assert!(table.views.iter().all(|view| view.format == expected_format), "{format:?} placed as format {expected_format}");
        if format != Format::F15 {
            // One code table a language in the block (decision 0336 §7): documents of a language read the same one.
            let mut tables = std::collections::HashMap::new();
            for (view, (_, lang)) in table.views.iter().zip(&sources) {
                let at = view.at.wrapping_add(view.codes);
                assert_eq!(*tables.entry(*lang).or_insert(at), at, "{format:?}: {lang:?}'s documents share its table");
            }
            assert!(tables.len() < sources.len(), "a language of several documents");
            let distinct: std::collections::HashSet<_> = tables.values().collect();
            assert_eq!(distinct.len(), tables.len(), "a table a language");
        }
        let node_doc = table.views.iter().enumerate().flat_map(|(doc, view)| std::iter::repeat_n(doc as u32, view.nodes as usize)).collect::<Vec<_>>();
        let views = MappedBuffer::from_slice::<ChunkTape>(&device, &table.views).unwrap();
        let node_doc = MappedBuffer::from_slice(&device, &node_doc).unwrap();
        let out = MappedBuffer::zeroed::<u32>(&device, table.nodes * FIELDS).unwrap();
        let pipeline = pipeline_named(&format!("#include <metal_stdlib>\nusing namespace metal;\n{}{TAPE}{KERNEL}", msl()), "tape_conformance").unwrap();
        let queue = device.new_command_queue();
        let command = queue.new_command_buffer();
        let encoder = command.new_compute_command_encoder();
        encoder.set_compute_pipeline_state(&pipeline);
        encoder.set_buffer(0, Some(&views.buffer), 0);
        encoder.set_buffer(1, Some(&node_doc.buffer), 0);
        encoder.set_buffer(2, Some(&out.buffer), 0);
        // The batch's chunk block, as the kernels bind it (decision 0289 §4).
        encoder.set_buffer(3, Some(table.block.as_ref().expect("one block").buffer()), 0);
        let count = table.nodes as u32;
        encoder.set_bytes(7, 4, std::ptr::from_ref(&count).cast());
        encoder.dispatch_threads(MTLSize::new(table.nodes as u64, 1, 1), MTLSize::new(64, 1, 1));
        encoder.end_encoding();
        command.commit();
        command.wait_until_completed();
        // SAFETY: the kernel wrote `nodes * FIELDS` u32s, and the command finished.
        let read = unsafe { std::slice::from_raw_parts(out.buffer.contents().cast::<u32>(), table.nodes * FIELDS) };
        for (doc, (tape, view)) in docs.iter().zip(&table.views).enumerate() {
            let source = sources[doc].0.as_bytes();
            let global = |node: Option<u32>| node.map_or(u32::MAX, |node| node + view.node_base);
            let mut classes = vec![0u32; tape.transducer_len()];
            for (node, _) in tape.transducer_supertypes() {
                classes[node as usize] += 1;
            }
            for node in 0..tape.transducer_len() as u32 {
                let span = tape.transducer_span(node);
                let expected = [
                    u32::from(tape.transducer_kind(node)),
                    u32::from(tape.transducer_field(node)),
                    u32::from(tape.transducer_flags(node)),
                    global(tape.transducer_parent(node)),
                    global(tape.transducer_next_sibling(node, false)),
                    global(tape.transducer_next_sibling(node, true)),
                    tape.transducer_children(node).count() as u32,
                    tape.transducer_descendants(node).end + view.node_base,
                    span.start as u32,
                    span.end as u32,
                    global(tape.transducer_children(node).next()),
                    classes[node as usize],
                    // The line of the node's start, and that line's first byte: from the source's newlines.
                    source[..span.start].iter().filter(|&&byte| byte == b'\n').count() as u32,
                    source[..span.start].iter().rposition(|&byte| byte == b'\n').map_or(0, |at| at as u32 + 1),
                ];
                let g = (view.node_base + node) as usize;
                assert_eq!(&read[g * FIELDS..][..FIELDS], &expected, "{format:?}: document {doc} node {node}");
            }
        }
        drop(table);
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
