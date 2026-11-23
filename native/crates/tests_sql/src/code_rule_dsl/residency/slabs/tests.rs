use super::*;
use crate::code_rule_dsl::language::{self, GrammarDialect};
use crate::code_rule_dsl::fixtures::{document, temp};
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

use std::sync::Mutex;

/// The residency is process-wide: the tests that admit and drop take turns. Other tests' label
/// runs admit their batches alongside, so counts are at least this test's.
static SERIAL: Mutex<()> = Mutex::new(());

#[test]
fn stored_tapes_are_admitted_into_slabs_and_read_in_place() {
    if rerail_metal::lane::device().is_err() {
        return;
    }
    let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // The run's format; every format's admission is `the_device_reads_every_format_as_the_document_does`.
    drop_all();
    let path = temp();
    let store = ContentStore::open(&path).unwrap();
    let mut digests = Vec::new();
    let mut tapes = Vec::new();
    for (source, lang) in SOURCES {
        // The device reads the stored image in place.
        let (tape, stored) = document(source, lang);
        assert!(resident::stored(&stored).is_ok(), "a stored image is read in place");
        let bytes = tape.resident_parts(None).unwrap().0;
        let source_id = store.put_source(source.as_bytes(), &[]).unwrap();
        store.put_tape(source_id, i64::from(language::tape_tag(lang).unwrap()), 1, i64::from(crate::code_rule_dsl::tape::formats::stored_version()), &bytes).unwrap();
        digests.push(crate::code_rule_dsl::tape::tape_identity(&Sha256::digest(source.as_bytes()).into(), &bytes));
        tapes.push((tape, bytes, source));
    }
    let documents = admit(&store, &digests, &grammar).unwrap();
    assert!(held().documents >= SOURCES.len());
    for (document, (tape, bytes, source)) in documents.iter().zip(&tapes) {
        // The image is the stored bytes, the source the file's.
        assert_eq!(document.image(), bytes.as_slice());
        assert_eq!(document.bytes_at(document.view.source, document.view.source_len as usize), source.as_bytes());
        for offset in [document.view.codes, document.view.source] {
            assert_eq!(offset % 16, 0, "the code table and the source at 16 bytes in their slab");
        }
        assert_eq!(document.view.nodes as usize, tape.transducer_len());
        // Its line index, its language's (decision 0612), where its tape holds it: each line's start read off it is
        // the document's.
        let head = document.bytes_at(document.view.lines, rerail_tape_format::newlines::HEAD);
        let index = document.bytes_at(document.view.lines, rerail_tape_format::newlines::len_of(head).unwrap() + 8);
        let view = rerail_clif::view::TapeView::default();
        let starts: Vec<u32> = (0..document.view.line_count).map(|k| rerail_tape_format::reads::f16::line_start(index, &view, k)).collect();
        assert_eq!(starts, tape.line_table().starts().collect::<Vec<_>>());
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
    if rerail_metal::lane::device().is_err() {
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

/// Decision 0353 step 4, 0471: a batch reads its documents where they lie. Documents admitted into two slabs (a source
/// past a slab's size between them) stay there: the batch binds the first one's slab, names the other as one to make
/// resident, and each view counts from the bound slab by the slabs' GPU addresses. Nothing is copied (the device reads
/// them so in `the_device_reads_every_format_as_the_document_does`).
#[test]
fn a_batch_over_several_slabs_reads_each_document_where_it_lies() {
    if rerail_metal::lane::device().is_err() {
        return;
    }
    use crate::code_rule_dsl::tape::formats::Format;
    let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    for format in Format::ALL {
        drop_all();
        let docs: Vec<crate::code_rule_dsl::tape::TapeDoc> = SOURCES
            .iter()
            .map(|&(source, lang)| {
                let native = ast_grep_core::tree_sitter::StrDoc::new(source, lang);
                let grammar = Grammar::new(lang).unwrap();
                crate::code_rule_dsl::tape::formats::as_run(format, || {
                    let image = crate::code_rule_dsl::tape::TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap().primary;
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
        // A batch over two slabs reaches the second by the slabs' device addresses (and a 16z batch its cache's):
        // refused where buffers have none (OpenCL without host USM: Apple's, AMD's), and its caller splits it by slab.
        if !rerail_metal::lane::device().unwrap().has_addresses() {
            assert!(DocTable::new(documents).is_none(), "{format:?}: a batch over two slabs refused without device addresses");
            drop(held_big);
            continue;
        }
        let table = DocTable::new(documents.clone()).unwrap();
        let block = table.block.as_ref().expect("one block bound");
        assert_eq!(block.id, first[0].slab.id, "{format:?}: the first document's slab is bound");
        assert_eq!(table.slabs.iter().map(|slab| slab.id).collect::<Vec<_>>(), vec![rest[0].slab.id], "{format:?}: the other slab, made resident");
        for (document, (held, view)) in documents.iter().zip(table.documents.iter().zip(&table.views)) {
            assert!(Arc::ptr_eq(document, held), "{format:?}: the document itself, not a copy");
            let address = |slab: &Slab| slab.device_buffer().device_address().expect("a buffer with an address") as i64;
            let apart = address(&document.slab).wrapping_sub(address(block));
            assert_eq!(view.offset(), apart + i64::from(document.view.at), "{format:?}: the view counts from the bound slab");
            // Past the batch's numbering, and a 16z view's cache (its address, its entry size), set for the batch.
            let rest_of = |view: &DocView| {
                let mut view = DocView { at: 0, at_high: 0, node_base: 0, source_base: 0, line_base: 0, ..*view };
                if view.format == rerail_clif::view::F16Z_FORMAT {
                    (view.side[0], view.side[1], view.columns[3], view.columns[4]) = (0, 0, 0, 0);
                }
                view
            };
            assert_eq!(rest_of(view), rest_of(&document.view), "{format:?}: its other offsets are its own");
        }
        // Asked again, the residency gives the same documents, where they were.
        let again = admit_docs(&docs).unwrap();
        assert!(again.iter().zip(&documents).all(|(a, b)| Arc::ptr_eq(a, b)), "{format:?}: nothing moved");
        drop((table, held_big, again));
        drop_all();
    }
}

/// The device reads a resident tape as its document reads it (decision 0293), in every format the run may store:
/// format 16's arrays, and a 16z tape as stored, read through its table's cache of decoded blocks
/// (decision 0353 step 4b) with no failures, with no parent
/// column, child list or index of nodes by code beside them. Every node's code (kind, field, flags, classes), its
/// parent, next and next named siblings, children, subtree and span. One document of more than a side-list part
/// (1,024 nodes), with gaps and lengths past a byte, reads the side lists and the parts' starts.
#[test]
fn the_device_reads_every_format_as_the_document_does() {
    use crate::code_rule_dsl::tape::formats::Format;
    use rerail_metal::lane;
    let tape = format!("{}{}{}", ukl(), "#include <ukl/reads/tape_z.h>\n", include_str!("../../../../../../src/code_rule_dsl/transducer/packed/device_tape.h"));
    const FIELDS: usize = 14;
    // UKL, on the process's device through the lane (decision 0440 step C).
    const KERNEL: &str = r#"
UKL_INLINE uint32_t tape_global(DocView d,uint32_t i){return i==TAPE_NONE?TAPE_NONE:d.node_base+i;}
typedef struct Conformance { uint32_t count; uint32_t pad; } Conformance;
UKL_REFLECT_STRUCT(Conformance, count, pad)
DEFINE_KERNEL2(slab_tests, tape_conformance,
               ROBUFFER, DocView,  docs,    "[documents]",
               ROBUFFER, uint32_t, nodeDoc, "[count]",
               RWBUFFER, uint32_t, out,     "[count * 14]",
               ROBUFFER, uint8_t,  chunk,   "[slab]",
               ARGS,     Conformance, args, ,
               GID0,     uint32_t, g,       )
{
 if(g>=args.count)return;
 UKL_GLOBAL const uint8_t* s=chunk;
 DocView d=docs[nodeDoc[g]];
 uint32_t i=g-d.node_base;
 UKL_GLOBAL uint32_t* o=out+g*14u;
 o[0]=tape_kind(s,d,i);o[1]=tape_field(s,d,i);o[2]=tape_flags(s,d,i);
 o[3]=tape_global(d,tape_parent(s,d,i));o[4]=tape_global(d,tape_next(s,d,i));o[5]=tape_global(d,tape_next_named(s,d,i));
 o[6]=tape_child_count(s,d,i);o[7]=d.node_base+i+tape_size(s,d,i);o[8]=tape_start(s,d,i);o[9]=tape_end(s,d,i);
 o[10]=tape_global(d,tape_first_child(s,d,i));o[11]=tape_code(s,d,i)->classCount;
 uint32_t line=tape_line(s,d,tape_start(s,d,i));o[12]=line;o[13]=tape_line_start(s,d,line);
}
"#;
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Conformance {
        count: u32,
        pad: u32,
    }
    // SAFETY: two u32s, no padding.
    unsafe impl lane::Pod for Conformance {}
    let Ok(device) = lane::device() else { return };
    // A command is submitted within an evaluation's launch budget.
    let _evaluation = crate::code_rule_dsl::transducer::packed::metal_backend::evaluation_scope();
    let _serial = SERIAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let long = format!("# {}\n", "x".repeat(300))
        + &(0..300).map(|i| format!("v{i} = [{i}, \"{}\", f(a, b=c)]  # {}\n", "s".repeat(i % 40), "c".repeat(260 * usize::from(i % 50 == 0)))).collect::<String>();
    let mut sources = SOURCES.iter().map(|&(source, lang)| (source.to_owned(), lang)).collect::<Vec<_>>();
    sources.push((long, GrammarDialect::Python));
    for format in Format::ALL {
        let docs = sources
            .iter()
            .map(|(source, lang)| {
                let native = ast_grep_core::tree_sitter::StrDoc::new(source, *lang);
                let grammar = Grammar::new(*lang).unwrap();
                // Made as a run of `format` makes it, whatever the process's run stores.
                crate::code_rule_dsl::tape::formats::as_run(format, || {
                    let image = crate::code_rule_dsl::tape::TapeImage::encode(native.tree.root_node(), &grammar, source, &|| false).unwrap().primary;
                    crate::code_rule_dsl::tape::TapeDoc::restore(Arc::new(source.clone()), grammar, &image, &|| false).unwrap()
                })
            })
            .collect::<Vec<_>>();
        assert!(docs.last().unwrap().transducer_len() > 2 * 1024, "a document of several parts");
        // Stored as the run stores them, and admitted from the store: each read from its frames.
        crate::install_store_functions();
        let path = temp();
        let store = ContentStore::open(&path).unwrap();
        let mut digests = Vec::new();
        for ((source, lang), doc) in sources.iter().zip(&docs) {
            let bytes = doc.resident_parts(None).unwrap().0;
            let source_id = store.put_source(source.as_bytes(), &[]).unwrap();
            store.put_tape(source_id, i64::from(language::tape_tag(*lang).unwrap()), 1, i64::from(format.store_version()), &bytes).unwrap();
            digests.push(crate::code_rule_dsl::tape::tape_identity(&Sha256::digest(source.as_bytes()).into(), &bytes));
        }
        // Every tape is stored as it is, in a segment (a load is its bytes, read in place; 16z decodes none of its
        // blocks), its presence with it: no row holds its bytes, whatever its format.
        let rows: Vec<(i64, i64, i64)> = rusqlite::Connection::open(&path).unwrap().prepare("SELECT codec_id, seg, length(frame) FROM tape_objects").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap().map(Result::unwrap).collect();
        assert!(!rows.is_empty() && rows.iter().all(|&(codec, seg, frame)| codec == 0 && seg > 0 && frame == 0), "{format:?} stored as {rows:?}");
        // As admitted (one slab), then over two slabs, a source past a slab's size admitted between the first document
        // and the rest: the batch reads each where it lies, the second slab made resident (decision 0471).
        for split in [false, true] {
        let held_big = if split {
            drop_all();
            let first = admit(&store, &digests[..1], &grammar).unwrap();
            let big = admit_sources(&[&vec![b'x'; SLAB_BYTES + (1 << 20)]]).unwrap();
            let rest = admit(&store, &digests[1..], &grammar).unwrap();
            assert_ne!(first[0].slab.id, rest[0].slab.id, "{format:?}: the documents are in two slabs");
            Some((big, first.into_iter().chain(rest).collect::<Vec<_>>()))
        } else {
            None
        };
        let documents = match &held_big {
            Some((_, documents)) => documents.clone(),
            None => admit(&store, &digests, &grammar).unwrap(),
        };
        // A 16z batch needs its cache's address in each view, and a batch over two slabs their addresses: refused
        // where buffers have none (OpenCL 1.2).
        if (format == Format::F16Z || split) && !rerail_metal::lane::device().unwrap().has_addresses() {
            assert!(DocTable::new(documents).is_none(), "{format:?}: refused without device addresses");
            continue;
        }
        let table = DocTable::new(documents).unwrap();
        assert_eq!(table.slabs.len(), usize::from(split), "{format:?}: the batch's other slabs");
        let expected_format = match format {
            Format::F16 => 16,
            Format::F16Z => rerail_clif::view::F16Z_FORMAT,
        };
        assert!(table.views.iter().all(|view| view.format == expected_format), "{format:?} placed as format {expected_format}");
        // The device reads the presence where the tape's home holds it (decision 0434 §4, step 6), 16 and 16z alike.
        assert!(table.views.iter().all(|view| view.presence != 0), "{format:?}: the views name the presence");
        assert_eq!(table.cache.is_some(), format == Format::F16Z, "a cache for 16z tapes alone");
        if !split {
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
        let views = lane::HostMapped::from_slice(&table.views).unwrap();
        let node_doc = lane::buffer_of(device, &node_doc);
        let out = lane::buffer_of(device, &vec![0_u32; table.nodes * FIELDS]);
        let kernel = lane::kernel(&format!("{tape}{KERNEL}"), "tape_conformance", None);
        let mut command = lane::Command::new("slab tape conformance");
        let mut launch = command
            .launch(&kernel, "tape_conformance")
            .buffer("docs", views.buffer())
            .buffer("nodeDoc", &node_doc)
            .buffer("out", &out)
            .buffer("chunk", table.block.as_ref().expect("one block bound").device_buffer())
            .value("args", &Conformance { count: table.nodes as u32, pad: 0 });
        for slab in &table.slabs {
            launch = launch.uses_read(slab.device_buffer());
        }
        if let Some(cache) = &table.cache {
            launch = launch.uses(cache);
        }
        launch.threads(table.nodes as u64, 64);
        command.run().unwrap();
        if let Some(cache) = &table.cache {
            assert_eq!(rerail_metal::slabs::zcache_failed(cache), 0, "every 16z block found room");
        }
        // SAFETY: the kernel wrote `nodes * FIELDS` u32s, and the command finished.
        let read = unsafe { std::slice::from_raw_parts(out.contents().cast::<u32>(), table.nodes * FIELDS) };
        for (doc, (tape, view)) in docs.iter().zip(&table.views).enumerate() {
            let lines = crate::code_rule_dsl::lines::LineTable::new(&sources[doc].0, crate::code_rule_dsl::lines::terminators(sources[doc].1));
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
                    // The line of the node's start, and that line's first byte: the language's lines.
                    lines.line(span.start) as u32,
                    lines.start(lines.line(span.start)) as u32,
                ];
                let g = (view.node_base + node) as usize;
                assert_eq!(&read[g * FIELDS..][..FIELDS], &expected, "{format:?}, over two slabs {split}: document {doc} node {node}");
            }
        }
        drop((table, held_big));
        }
        drop(store);
        let _ = std::fs::remove_file(path);
    }
}
