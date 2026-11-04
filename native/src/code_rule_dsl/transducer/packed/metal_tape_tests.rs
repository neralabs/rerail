//! The device's tape accessors (`metal_tape.metal`) over resident documents, against the packed
//! batch's node records they replace (decision 0032): every node of a batch of stored fixtures.
use super::*;
use crate::code_rule_dsl::language::{self, GrammarDialect};
use crate::code_rule_dsl::prefilter::tests::{document, temp};
use crate::code_rule_dsl::residency::slabs::{self, DocTable, DocView};
use crate::code_rule_dsl::tape::{Grammar, TAPE_FORMAT_VERSION};
use crate::content_store::ContentStore;
use sha2::{Digest as _, Sha256};

const TAPE: &str = include_str!("metal_tape.metal");
const FIELDS: usize = 12;

const KERNEL: &str = r#"
inline uint tape_global(DocView d,uint i){return i==TAPE_NONE?TAPE_NONE:d.nodeBase+i;}
kernel void tape_conformance(device const DocView* docs [[buffer(0)]],device const uint* nodeDoc [[buffer(1)]],device uint* out [[buffer(2)]],device const uchar* s0 [[buffer(3)]],device const uchar* s1 [[buffer(4)]],device const uchar* s2 [[buffer(5)]],device const uchar* s3 [[buffer(6)]],constant uint& count [[buffer(7)]],uint g [[thread_position_in_grid]]){
 if(g>=count)return;
 Slabs s={s0,s1,s2,s3};
 DocView d=docs[nodeDoc[g]];
 uint i=g-d.nodeBase;
 device uint* o=out+g*12u;
 o[0]=tape_kind(s,d,i);o[1]=tape_field(s,d,i);o[2]=tape_flags(s,d,i);
 o[3]=tape_global(d,tape_parent(s,d,i));o[4]=tape_global(d,tape_next(s,d,i));o[5]=tape_global(d,tape_next_named(s,d,i));
 o[6]=tape_child_count(s,d,i);o[7]=d.nodeBase+i+tape_size(s,d,i);o[8]=tape_start(s,d,i);o[9]=tape_end(s,d,i);
 o[10]=tape_global(d,tape_first_child(s,d,i));o[11]=tape_code(s,d,i).classCount;
}
"#;

const SOURCES: [(&str, GrammarDialect); 5] = [
    ("value = left + right\nclass A:\n    def f(self, x=[]):\n        return (yield x)\n", GrammarDialect::Python),
    ("const f = (a, b) => { return a?.b ?? `t${b}`; };\n// c\n", GrammarDialect::JavaScript),
    ("def broken(:\n    pass\n", GrammarDialect::Python),
    ("fn main() { let x: Vec<u8> = vec![1, 2]; if let Some(y) = x.first() { println!(\"{y}\"); } }\n", GrammarDialect::Rust),
    ("import os\n\n\ndef g(*args, **kw):\n    '''doc'''\n    return [a for a in args if a]  # noqa\n", GrammarDialect::Python),
];

#[test]
fn the_device_reads_resident_tapes_as_the_packed_batch_records_them() {
    let Some(device) = METAL_DEVICE.as_ref() else { return };
    let path = temp();
    let store = ContentStore::open(&path).unwrap();
    let (mut digests, mut tapes) = (Vec::new(), Vec::new());
    for (source, lang) in SOURCES {
        let (tape, bytes) = document(source, lang);
        let source_id = store.put_source(source.as_bytes(), &[]).unwrap();
        store.put_tape(source_id, i64::from(language::tape_tag(lang).unwrap()), 1, i64::from(TAPE_FORMAT_VERSION), &bytes).unwrap();
        digests.push(<[u8; 32]>::from(Sha256::digest(&bytes)));
        tapes.push(tape);
    }
    let grammar = |id: i64| Grammar::new(u8::try_from(id).ok().and_then(language::from_tape_tag).ok_or("unknown grammar")?);
    let table = DocTable::new(slabs::admit(&store, &digests, &grammar).unwrap()).unwrap();
    // The records the device read until now, for the same files.
    let program = Program::new(vec![super::super::word(super::super::EMIT, 0, 0)], vec![b"tape".to_vec()], Vec::new(), vec![0], Vec::new()).unwrap();
    let anchors = vec![vec![Vec::<u32>::new()]; tapes.len()];
    let batch = PackedBatch::new(std::slice::from_ref(&program), &tapes, &anchors).unwrap();
    assert_eq!(batch.nodes.len(), table.nodes);
    let node_doc = table.views.iter().enumerate().flat_map(|(doc, view)| std::iter::repeat_n(doc as u32, view.nodes as usize)).collect::<Vec<_>>();
    let docs = MappedBuffer::from_slice::<DocView>(device, &table.views).unwrap();
    let node_doc = MappedBuffer::from_slice(device, &node_doc).unwrap();
    let out = MappedBuffer::zeroed::<u32>(device, table.nodes * FIELDS).unwrap();
    let source = format!("#include <metal_stdlib>\nusing namespace metal;\n{TAPE}{KERNEL}");
    let pipeline = pipeline_named(&source, "tape_conformance").unwrap();
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let encoder = command.new_compute_command_encoder();
    encoder.set_compute_pipeline_state(&pipeline);
    encoder.set_buffer(0, Some(&docs.buffer), 0);
    encoder.set_buffer(1, Some(&node_doc.buffer), 0);
    encoder.set_buffer(2, Some(&out.buffer), 0);
    for slot in 0..4 {
        let slab = &table.slabs[slot.min(table.slabs.len() - 1)];
        encoder.set_buffer(3 + slot as u64, Some(&slab.buffer.buffer), 0);
    }
    let count = table.nodes as u32;
    encoder.set_bytes(7, 4, std::ptr::from_ref(&count).cast());
    encoder.dispatch_threads(MTLSize::new(table.nodes as u64, 1, 1), MTLSize::new(64, 1, 1));
    encoder.end_encoding();
    command.commit();
    command.wait_until_completed();
    let read = unsafe { std::slice::from_raw_parts(out.buffer.contents().cast::<u32>(), table.nodes * FIELDS) };
    let classes = |node: u32| batch.supertypes.iter().filter(|pair| pair.node == node).count() as u32;
    for (file, record) in batch.files.iter().enumerate() {
        for local in 0..record.node_count {
            let g = record.node_start + local;
            let node = &batch.nodes[g as usize];
            let first = if node.child_count > 0 { batch.children[node.child_start as usize] } else { u32::MAX };
            let expected = [
                node.kind,
                node.field,
                node.flags,
                node.parent,
                node.next_any,
                node.next_named,
                node.child_count,
                node.descendant_end,
                node.source_start - record.source_start,
                node.source_end - record.source_start,
                first,
                classes(g),
            ];
            assert_eq!(&read[g as usize * FIELDS..][..FIELDS], &expected, "file {file} node {local}");
        }
    }
    drop(table);
    drop(store);
    let _ = std::fs::remove_file(path);
}
