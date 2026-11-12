//! The device's tape accessors (`metal_tape.metal`) over resident documents, against the packed
//! batch's node records they replace (decision 0032): every node of a batch of stored fixtures.
use super::*;
use crate::code_rule_dsl::language::{self, GrammarDialect};
use crate::code_rule_dsl::fixtures::{document, temp};
// cratify: a re-export of rerail-engine's, which its facades of this module make
use crate::code_rule_dsl::residency::slabs::{self, DocTable, DocView};
use crate::code_rule_dsl::tape::{Grammar, TAPE_FORMAT_VERSION};
use crate::content_store::ContentStore;
use sha2::Sha256;

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

#[ignore = "open (decision 0128): parses files whose language only the engine registers; to be hosted by rerail-engine"]
fn the_device_reads_resident_tapes_as_the_packed_batch_records_them() {
    let Some(device) = METAL_DEVICE.as_ref() else { return };
    let path = temp();
    let store = ContentStore::open(&path).unwrap();
    let (mut digests, mut tapes) = (Vec::new(), Vec::new());
    for (source, lang) in SOURCES {
        // The device reads full images in place; a tree-sitter tape is stored named-only
        // (RER-131), so the store holds the document's restored image here.
        let (tape, _stored) = document(source, lang);
        let bytes = tape.resident_parts().unwrap().0;
        let source_id = store.put_source(source.as_bytes(), &[]).unwrap();
        store.put_tape(source_id, i64::from(language::tape_tag(lang).unwrap()), 1, i64::from(TAPE_FORMAT_VERSION), &bytes).unwrap();
        digests.push(crate::code_rule_dsl::tape::tape_identity(&Sha256::digest(source.as_bytes()).into(), &bytes));
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

/// Diagnostic (O39): the process footprint a command queue and a submission add, run alone
/// (`--ignored --nocapture`).
#[test]
#[ignore = "diagnostic: prints the footprint of queues and submissions"]
fn a_submission_costs_footprint() {
    let Some(device) = METAL_DEVICE.as_ref() else { return };
    let mb = crate::code_rule_dsl::prof::footprint_mb;
    let source = "#include <metal_stdlib>\nusing namespace metal;\nkernel void touch(device uint* o [[buffer(0)]],uint g [[thread_position_in_grid]]){o[g]=g;}";
    let pipeline = pipeline_named(source, "touch").unwrap();
    eprintln!("start {} MB", mb());
    let out = MappedBuffer::zeroed::<u32>(device, 1 << 20).unwrap();
    let submit = |queue: &metal::CommandQueueRef| {
        let command = queue.new_command_buffer();
        let encoder = command.new_compute_command_encoder();
        encoder.set_compute_pipeline_state(&pipeline);
        encoder.set_buffer(0, Some(&out.buffer), 0);
        let threads = std::env::var("RERAIL_DIAG_THREADS").ok().and_then(|s| s.parse().ok()).unwrap_or(1_u64 << 20);
        encoder.dispatch_threads(MTLSize::new(threads, 1, 1), MTLSize::new(64, 1, 1));
        encoder.end_encoding();
        command.commit();
        command.wait_until_completed();
    };
    let queue = device.new_command_queue();
    eprintln!("queue {} MB", mb());
    submit(&queue);
    eprintln!("first submission {} MB", mb());
    for _ in 0..10 {
        submit(&queue);
    }
    eprintln!("ten more on one queue {} MB", mb());
    for _ in 0..10 {
        metal::objc::rc::autoreleasepool(|| submit(&device.new_command_queue()));
    }
    eprintln!("ten new queues, pooled {} MB", mb());
    for _ in 0..10 {
        submit(&device.new_command_queue());
    }
    eprintln!("ten new queues, unpooled {} MB", mb());
    // `RERAIL_DIAG_PAUSE`: seconds to wait here, for `footprint`/`vmmap` on the process.
    if let Some(seconds) = std::env::var("RERAIL_DIAG_PAUSE").ok().and_then(|s| s.parse().ok()) {
        eprintln!("pid {}", std::process::id());
        std::thread::sleep(std::time::Duration::from_secs(seconds));
    }
}

/// Diagnostic (O39): the footprint a kernel's private stack adds (the driver's scratch, sized for
/// every thread in flight and kept by the process), by stack words (`RERAIL_DIAG_WORDS`).
#[test]
#[ignore = "diagnostic: prints the footprint of a kernel's private stack"]
fn a_private_stack_costs_footprint() {
    let Some(device) = METAL_DEVICE.as_ref() else { return };
    let mb = crate::code_rule_dsl::prof::footprint_mb;
    let words: usize = std::env::var("RERAIL_DIAG_WORDS").ok().and_then(|s| s.parse().ok()).unwrap_or(1024);
    let source = format!(
        "#include <metal_stdlib>\nusing namespace metal;\nkernel void deep(device uint* o [[buffer(0)]],uint g [[thread_position_in_grid]]){{uint s[{words}];for(uint i=0;i<{words}u;i++)s[(i*7u+g)%{words}u]=i^g;uint a=0;for(uint i=0;i<{words}u;i+=13u)a+=s[(i+o[g])%{words}u];o[g]=a;}}"
    );
    let shallow = "#include <metal_stdlib>\nusing namespace metal;\nkernel void touch(device uint* o [[buffer(0)]],uint g [[thread_position_in_grid]]){o[g]=g;}";
    let out = MappedBuffer::zeroed::<u32>(device, 1 << 20).unwrap();
    let queue = device.new_command_queue();
    let submit = |pipeline: &metal::ComputePipelineState| {
        let command = queue.new_command_buffer();
        let encoder = command.new_compute_command_encoder();
        encoder.set_compute_pipeline_state(pipeline);
        encoder.set_buffer(0, Some(&out.buffer), 0);
        encoder.dispatch_threads(MTLSize::new(1 << 20, 1, 1), MTLSize::new(64, 1, 1));
        encoder.end_encoding();
        command.commit();
        command.wait_until_completed();
    };
    submit(&pipeline_named(shallow, "touch").unwrap());
    eprintln!("after a shallow kernel {} MB", mb());
    let deep = pipeline_named(&source, "deep").unwrap();
    eprintln!("compiled, {words} words {} MB", mb());
    submit(&deep);
    eprintln!("after a {words}-word stack {} MB", mb());
}

/// Diagnostic (O39): the footprint one label kernel's dispatch adds (`RERAIL_DIAG_KERNEL`), with
/// no items, in a fresh process: its private memory, which the driver sizes for every thread in
/// flight and keeps.
#[test]
#[ignore = "diagnostic: prints the footprint of one label kernel's dispatch"]
fn a_label_kernel_costs_footprint() {
    let Some(device) = METAL_DEVICE.as_ref() else { return };
    let Ok(name) = std::env::var("RERAIL_DIAG_KERNEL") else { return };
    let mb = crate::code_rule_dsl::prof::footprint_mb;
    let source = match std::env::var_os("RERAIL_DIAG_SOURCE") {
        Some(path) => std::fs::read_to_string(path).unwrap(),
        None => format!("#include <metal_stdlib>\nusing namespace metal;\n{TAPE}{}", include_str!("metal_label_passes.metal")),
    };
    let pipeline = pipeline_named(&source, &name).unwrap();
    let zero = MappedBuffer::zeroed::<u32>(device, 1 << 18).unwrap();
    let queue = device.new_command_queue();
    let submit = |pipeline: &metal::ComputePipelineState| {
        let command = queue.new_command_buffer();
        let encoder = command.new_compute_command_encoder();
        encoder.set_compute_pipeline_state(pipeline);
        for slot in 0..31 {
            encoder.set_buffer(slot, Some(&zero.buffer), 0);
        }
        encoder.dispatch_thread_groups(MTLSize::new(1, 1, 1), MTLSize::new(64, 1, 1));
        encoder.end_encoding();
        command.commit();
        command.wait_until_completed();
    };
    let touch = pipeline_named("#include <metal_stdlib>\nusing namespace metal;\nkernel void touch(device uint* o [[buffer(0)]],uint g [[thread_position_in_grid]]){o[g]=0;}", "touch").unwrap();
    submit(&touch);
    let before = mb();
    submit(&pipeline);
    eprintln!("KERNEL {name}: +{} MB", mb() as i64 - before as i64);
}
