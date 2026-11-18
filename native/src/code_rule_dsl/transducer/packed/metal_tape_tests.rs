//! The device's tape accessors (`metal_tape.metal`) over resident documents, against the packed
//! batch's node records they replace (decision 0032): every node of a batch of stored fixtures.
use super::*;
use crate::code_rule_dsl::language::{self, GrammarDialect};
use crate::code_rule_dsl::fixtures::{document, temp};
use crate::code_rule_dsl::residency::slabs::{self, DocTable, DocView};
use crate::code_rule_dsl::tape::{Grammar, TapeDoc};
use crate::content_store::ContentStore;
use sha2::{Digest as _, Sha256};
// What the backend reads through its own imports, which its facade in the engine does not re-export.
use crate::code_rule_dsl::transducer::packed::metal_backend::{MappedBuffer, pipeline_named};
use metal::MTLSize;

const TAPE: &str = concat!(include_str!("metal_tape_reads.metal"), include_str!("metal_tape.metal"));
const FIELDS: usize = 12;

const KERNEL: &str = r#"
inline uint tape_global(DocView d,uint i){return i==TAPE_NONE?TAPE_NONE:d.node_base+i;}
kernel void tape_conformance(device const DocView* docs [[buffer(0)]],device const uint* nodeDoc [[buffer(1)]],device uint* out [[buffer(2)]],device const uchar* chunk [[buffer(3)]],constant uint& count [[buffer(7)]],uint g [[thread_position_in_grid]]){
 if(g>=count)return;
 device const uchar* s=chunk;
 DocView d=docs[nodeDoc[g]];
 uint i=g-d.node_base;
 device uint* o=out+g*12u;
 o[0]=tape_kind(s,d,i);o[1]=tape_field(s,d,i);o[2]=tape_flags(s,d,i);
 o[3]=tape_global(d,tape_parent(s,d,i));o[4]=tape_global(d,tape_next(s,d,i));o[5]=tape_global(d,tape_next_named(s,d,i));
 o[6]=tape_child_count(s,d,i);o[7]=d.node_base+i+tape_size(s,d,i);o[8]=tape_start(s,d,i);o[9]=tape_end(s,d,i);
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
        // The device reads full images in place; a tree-sitter tape is stored named-only
        // (RER-131), so the store holds the document's restored image here.
        let (tape, _stored) = document(source, lang);
        let bytes = tape.resident_parts(None).unwrap().0;
        let source_id = store.put_source(source.as_bytes(), &[]).unwrap();
        store.put_tape(source_id, i64::from(language::tape_tag(lang).unwrap()), 1, i64::from(crate::code_rule_dsl::tape::formats::stored_version()), &bytes).unwrap();
        digests.push(crate::code_rule_dsl::tape::tape_identity(&Sha256::digest(source.as_bytes()).into(), &bytes));
        tapes.push(tape);
    }
    let grammar = |id: i64| Grammar::new(u8::try_from(id).ok().and_then(language::from_tape_tag).ok_or("unknown grammar")?);
    let table = DocTable::new(slabs::admit(&store, &digests, &grammar).unwrap()).unwrap();
    assert_eq!(tapes.iter().map(TapeDoc::transducer_len).sum::<usize>(), table.nodes);
    let node_doc = table.views.iter().enumerate().flat_map(|(doc, view)| std::iter::repeat_n(doc as u32, view.nodes as usize)).collect::<Vec<_>>();
    let docs = MappedBuffer::from_slice::<DocView>(device, &table.views).unwrap();
    let node_doc = MappedBuffer::from_slice(device, &node_doc).unwrap();
    let out = MappedBuffer::zeroed::<u32>(device, table.nodes * FIELDS).unwrap();
    let source = format!("#include <metal_stdlib>\nusing namespace metal;\n{}{TAPE}{KERNEL}", slabs::msl());
    let pipeline = pipeline_named(&source, "tape_conformance").unwrap();
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let encoder = command.new_compute_command_encoder();
    encoder.set_compute_pipeline_state(&pipeline);
    encoder.set_buffer(0, Some(&docs.buffer), 0);
    encoder.set_buffer(1, Some(&node_doc.buffer), 0);
    encoder.set_buffer(2, Some(&out.buffer), 0);
    // The batch's one block (decision 0289 §4).
    encoder.set_buffer(3, Some(table.block.as_ref().unwrap().buffer()), 0);
    let count = table.nodes as u32;
    encoder.set_bytes(7, 4, std::ptr::from_ref(&count).cast());
    encoder.dispatch_threads(MTLSize::new(table.nodes as u64, 1, 1), MTLSize::new(64, 1, 1));
    encoder.end_encoding();
    command.commit();
    command.wait_until_completed();
    let read = unsafe { std::slice::from_raw_parts(out.buffer.contents().cast::<u32>(), table.nodes * FIELDS) };
    // The tape's own answers, by its accessors, in the batch's numbering (each file's nodes past the files before).
    let mut base = 0u32;
    for (file, tape) in tapes.iter().enumerate() {
        let count = tape.transducer_len();
        let (mut parent, mut next_any, mut next_named) = (vec![u32::MAX; count], vec![u32::MAX; count], vec![u32::MAX; count]);
        for node in 0..count as u32 {
            let children = tape.transducer_children(node).collect::<Vec<_>>();
            for (k, &child) in children.iter().enumerate() {
                parent[child as usize] = base + node;
                next_any[child as usize] = children.get(k + 1).map_or(u32::MAX, |&next| base + next);
                next_named[child as usize] =
                    children[k + 1..].iter().find(|&&next| tape.transducer_is_named(next)).map_or(u32::MAX, |&next| base + next);
            }
        }
        for local in 0..count as u32 {
            let g = base + local;
            let children = tape.transducer_children(local).collect::<Vec<_>>();
            let span = tape.transducer_span(local);
            let expected = [
                u32::from(tape.transducer_kind(local)),
                u32::from(tape.transducer_field(local)),
                u32::from(tape.transducer_flags(local)),
                parent[local as usize],
                next_any[local as usize],
                next_named[local as usize],
                children.len() as u32,
                base + tape.transducer_descendants(local).end,
                span.start as u32,
                span.end as u32,
                children.first().map_or(u32::MAX, |&child| base + child),
                tape.transducer_supertypes().filter(|&(node, _)| node == local).count() as u32,
            ];
            assert_eq!(&read[g as usize * FIELDS..][..FIELDS], &expected, "file {file} node {local}");
        }
        base += count as u32;
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
        None => format!("#include <metal_stdlib>\nusing namespace metal;\n{}{TAPE}{}", slabs::msl(), include_str!("metal_label_passes.metal")),
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

/// Every label kernel and the fact-only pattern kernel compile, for a mixed batch (0) and for each format's own
/// (`tape_format`). A kernel that does not compile is fatal (decision 0382; before it, a run fell back to the CPU and
/// gave the right rows, so the label tests alone passed with MSL that did not compile: t41, Q62); this compiles every
/// format's kernels whether or not a run reaches them.
/// Q181 probe: what the label kernels' cold compile spends. The source is made new to Metal's shader cache by a
/// nonce, in a comment (`RERAIL_PROBE_NONCE=comment`: the front end's cache misses) or in code each kernel reads (the
/// default: both miss); then the library (the front end), and per kernel and format its function (the constant's
/// specialization) and pipeline (the back end), each timed.
#[test]
#[ignore = "a probe of the label kernels' compile"]
fn label_kernels_compile_cost() {
    use std::time::Instant;
    let Ok(device) = rerail_metal::device() else { return };
    let (source, kernels) = super::label_passes::label_source();
    let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u32 | 1 << 20;
    let source = match std::env::var("RERAIL_PROBE_NONCE").as_deref() {
        Ok("comment") => format!("{source}\n// nonce {nonce}\n"),
        _ => source.replace("#define clif_view(v,a) ((tape_format!=0u&&", &format!("#define clif_view(v,a) ((tape_format!=0u&&tape_format!={nonce}u&&")),
    };
    assert!(source.contains(&nonce.to_string()), "the nonce is in the source");
    eprintln!("source: {} bytes, {} lines", source.len(), source.lines().count());
    let clock = Instant::now();
    let library = device.new_library_with_source(&source, &metal::CompileOptions::new()).expect("the library");
    eprintln!("library (front end): {:.2} s", clock.elapsed().as_secs_f64());
    if std::env::var_os("RERAIL_PROBE_PARALLEL").is_some() {
        // Every kernel of format 16 at once, a thread each: whether the compiler service runs them side by side.
        let format = 16_u32;
        let clock = Instant::now();
        let times = std::thread::scope(|scope| {
            let handles = kernels
                .iter()
                .map(|name| {
                    let (library, device) = (&library, &device);
                    scope.spawn(move || {
                        let constants = metal::FunctionConstantValues::new();
                        constants.set_constant_value_at_index((&format as *const u32).cast(), metal::MTLDataType::UInt, 0);
                        let clock = Instant::now();
                        let function = library.get_function(name, Some(constants)).expect("the function");
                        let descriptor = metal::ComputePipelineDescriptor::new();
                        descriptor.set_compute_function(Some(&function));
                        let _pipeline = device.new_compute_pipeline_state(&descriptor).expect("the pipeline");
                        clock.elapsed().as_secs_f64()
                    })
                })
                .collect::<Vec<_>>();
            handles.into_iter().map(|handle| handle.join().unwrap()).collect::<Vec<_>>()
        });
        eprintln!("format 16 in parallel: {:.2} s wall, {:.2} s summed over the kernels' threads", clock.elapsed().as_secs_f64(), times.iter().sum::<f64>());
        return;
    }
    let formats: Vec<u32> = std::env::var("RERAIL_PROBE_FORMATS").ok().map_or(vec![16, 15, 0], |f| f.split(',').map(|f| f.parse().unwrap()).collect());
    for format in formats {
        let (mut functions, mut pipelines) = (0.0, 0.0);
        for name in kernels {
            let constants = metal::FunctionConstantValues::new();
            constants.set_constant_value_at_index((&format as *const u32).cast(), metal::MTLDataType::UInt, 0);
            let clock = Instant::now();
            let function = library.get_function(name, Some(constants)).expect("the function");
            let function_s = clock.elapsed().as_secs_f64();
            let descriptor = metal::ComputePipelineDescriptor::new();
            descriptor.set_compute_function(Some(&function));
            let clock = Instant::now();
            let _pipeline = device.new_compute_pipeline_state(&descriptor).expect("the pipeline");
            let pipeline_s = clock.elapsed().as_secs_f64();
            eprintln!("  format {format:>2} {name:<18} function {function_s:>6.2} s  pipeline {pipeline_s:>6.2} s");
            functions += function_s;
            pipelines += pipeline_s;
        }
        eprintln!("format {format}: functions {functions:.2} s, pipelines {pipelines:.2} s");
    }
}

#[test]
fn the_label_kernels_compile_for_every_format() {
    if METAL_DEVICE.is_none() {
        return;
    }
    for format in [0, 15, 16] {
        let compiled = super::label_passes::compile_label_kernels(format).unwrap_or_else(|_| panic!("the label kernels for format {format}"));
        // 8: each walk's stack tiers are one kernel (work queue Q181), with decision 0365's `label_formula` and
        // `label_leaves`.
        assert_eq!(compiled, (8, 1), "format {format}: the label kernels, then the pattern kernel");
    }
}
