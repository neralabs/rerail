//! The device's tape accessors (`device_tape.h`) over resident documents, against the packed
//! batch's node records they replace (decision 0032): every node of a batch of stored fixtures. The kernel is UKL,
//! launched through the device layer (decision 0440 step C): it runs on the process's device, and its source
//! compiles for every backend this machine has.
use crate::code_rule_dsl::language::{self, GrammarDialect};
use crate::code_rule_dsl::fixtures::{document, temp};
use crate::code_rule_dsl::residency::slabs::{self, DocTable, DocView};
use crate::code_rule_dsl::tape::{Grammar, TapeDoc};
use crate::content_store::ContentStore;
use sha2::{Digest as _, Sha256};
use rerail_gpu::device::{ComputeFunctionLibrary, ComputeQueue};
use rerail_gpu::kernels::{buffer_of, launch, read};

const TAPE: &str = concat!("#include <ukl/reads/tape_z.h>\n", include_str!("device_tape.h"));
const FIELDS: usize = 12;

const KERNEL: &str = r#"
UKL_INLINE uint32_t tape_global(DocView d,uint32_t i){return i==TAPE_NONE?TAPE_NONE:d.node_base+i;}
typedef struct Conformance { uint32_t count; uint32_t pad; } Conformance;
UKL_REFLECT_STRUCT(Conformance, count, pad)
DEFINE_KERNEL2(tape_tests, tape_conformance,
               ROBUFFER, DocView,  docs,    "[documents]",
               ROBUFFER, uint32_t, nodeDoc, "[count]",
               RWBUFFER, uint32_t, out,     "[count * 12]",
               ROBUFFER, uint8_t,  chunk,   "[slab]",
               ARGS,     Conformance, args, ,
               GID0,     uint32_t, g,       )
{
 if(g>=args.count)return;
 UKL_GLOBAL const uint8_t* s=chunk;
 DocView d=docs[nodeDoc[g]];
 uint32_t i=g-d.node_base;
 UKL_GLOBAL uint32_t* o=out+g*12u;
 o[0]=tape_kind(s,d,i);o[1]=tape_field(s,d,i);o[2]=tape_flags(s,d,i);
 o[3]=tape_global(d,tape_parent(s,d,i));o[4]=tape_global(d,tape_next(s,d,i));o[5]=tape_global(d,tape_next_named(s,d,i));
 o[6]=tape_child_count(s,d,i);o[7]=d.node_base+i+tape_size(s,d,i);o[8]=tape_start(s,d,i);o[9]=tape_end(s,d,i);
 o[10]=tape_global(d,tape_first_child(s,d,i));o[11]=tape_code(s,d,i)->classCount;
}
"#;

const SOURCES: [(&str, GrammarDialect); 5] = [
    ("value = left + right\nclass A:\n    def f(self, x=[]):\n        return (yield x)\n", GrammarDialect::Python),
    ("const f = (a, b) => { return a?.b ?? `t${b}`; };\n// c\n", GrammarDialect::JavaScript),
    ("def broken(:\n    pass\n", GrammarDialect::Python),
    ("fn main() { let x: Vec<u8> = vec![1, 2]; if let Some(y) = x.first() { println!(\"{y}\"); } }\n", GrammarDialect::Rust),
    ("import os\n\n\ndef g(*args, **kw):\n    '''doc'''\n    return [a for a in args if a]  # noqa\n", GrammarDialect::Python),
];

/// The kernel's arguments.
#[repr(C)]
#[derive(Clone, Copy)]
struct Conformance {
    count: u32,
    pad: u32,
}
unsafe impl rerail_gpu::Pod for Conformance {}

/// The views' words (`DocView` is 30 u32s, `#[repr(C)]`).
fn bytemuck_words(views: &[DocView]) -> &[u32] {
    // SAFETY: a `DocView` is u32s alone, with no padding.
    unsafe { std::slice::from_raw_parts(views.as_ptr().cast::<u32>(), std::mem::size_of_val(views) / 4) }
}

#[test]
fn the_device_reads_resident_tapes_as_the_packed_batch_records_them() {
    if rerail_gpu::shared::get().is_none() {
        return;
    }
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
    let shared = rerail_gpu::shared::get().expect("the process's device");
    let source = format!("{}{TAPE}{KERNEL}", slabs::ukl());
    // The same source for every backend this machine has: the views are portable UKL.
    let others = [
        rerail_gpu::compute_kernel_cpu::CpuDevice::system_default().map(rerail_gpu::device::Device::Cpu),
        rerail_gpu::compute_kernel_opencl::ClDevice::open().ok().map(rerail_gpu::device::Device::OpenCl),
    ];
    for device in others.into_iter().flatten() {
        let which = if device.is_cpu() { "the CPU" } else { "OpenCL" };
        let text = rerail_gpu::Preprocessing::preprocess(&device).file("/<tests>/tape_conformance.uglykl", source.as_str()).run("/<tests>/tape_conformance.uglykl").unwrap_or_else(|e| panic!("{which}: {e}"));
        let lib = ComputeFunctionLibrary::compile_from_source(&device, &text, "tape_conformance.uglykl").unwrap_or_else(|e| {
            let kept = std::env::temp_dir().join(format!("tape_conformance.{}.cl", std::process::id()));
            let _ = std::fs::write(&kept, &text);
            panic!("{which}: {e} (the text compiled: {})", kept.display())
        });
        lib.get_function(&device, "tape_conformance").unwrap_or_else(|e| panic!("{which}: {e}"));
    }
    let program = rerail_gpu::kernels::program("tape_conformance.uglykl", Box::leak(source.into_boxed_str())).unwrap_or_else(|e| panic!("{e}"));
    let docs = buffer_of::<u32>(bytemuck_words(&table.views)).unwrap();
    let node_doc = buffer_of(&node_doc).unwrap();
    let out = buffer_of(&vec![0u32; table.nodes * FIELDS]).unwrap();
    let mut q = ComputeQueue::new(&shared.queue);
    launch(&q, program.function("tape_conformance").unwrap())
        .buffer("docs", &docs)
        .buffer("nodeDoc", &node_doc)
        .buffer("out", &out)
        // The batch's one block (decision 0289 §4), read where it lies.
        .buffer("chunk", table.block.as_ref().unwrap().device_buffer())
        .value("args", &Conformance { count: table.nodes as u32, pad: 0 })
        .run(&[table.nodes.div_ceil(64) as u64], &[64])
        .unwrap_or_else(|e| panic!("{e}"));
    q.flush().await_().unwrap();
    let read: Vec<u32> = read(&out, table.nodes * FIELDS);
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

/// The diagnostics' kernels (UKL): `touch` writes each thread's index; `deep` keeps a private array of
/// `DEEP_WORDS` words (a define), so the driver gives it a stack of that size.
const DIAG: &str = r#"
#include <ukl/backend.h>
DEFINE_KERNEL2(diag, touch, RWBUFFER, uint32_t, o, "[threads]", GID0, uint32_t, g, ) { o[g] = g; }
DEFINE_KERNEL2(diag, deep, RWBUFFER, uint32_t, o, "[threads]", GID0, uint32_t, g, )
{
    uint32_t s[DEEP_WORDS];
    for (uint32_t i = 0u; i < DEEP_WORDS; i++) s[(i * 7u + g) % DEEP_WORDS] = i ^ g;
    uint32_t a = 0u;
    for (uint32_t i = 0u; i < DEEP_WORDS; i += 13u) a += s[(i + o[g]) % DEEP_WORDS];
    o[g] = a;
}
"#;

/// A diagnostic's kernel, with its stack's words.
fn diag_kernel(name: &str, words: usize) -> rerail_metal::lane::Kernel {
    rerail_metal::lane::kernel(&format!("#define DEEP_WORDS {words}u\n{DIAG}"), name, None)
}

/// One launch of `kernel` over `threads` threads of 64, waited for.
fn diag_submit(kernel: &rerail_metal::lane::Kernel, out: &rerail_gpu::device::Buffer, threads: u64) {
    let _scope = rerail_metal::launch::LaunchScope::enter();
    let mut command = rerail_metal::lane::Command::new("diagnostic");
    command.launch(kernel, "diagnostic").buffer("o", out).threads(threads, 64);
    command.run().expect("the diagnostic's launch");
}

/// Diagnostic (O39): the process footprint a command queue and a submission add, run alone
/// (`--ignored --nocapture`). The kernel is UKL, on the process's device; the queues made apart
/// from the device's own are Metal's (a device layer's queue is the device's).
#[test]
#[ignore = "diagnostic: prints the footprint of queues and submissions"]
fn a_submission_costs_footprint() {
    let Ok(device) = rerail_metal::lane::device() else { return };
    let mb = crate::code_rule_dsl::prof::footprint_mb;
    let touch = diag_kernel("touch", 1);
    eprintln!("start {} MB", mb());
    let threads = std::env::var("RERAIL_DIAG_THREADS").ok().and_then(|s| s.parse().ok()).unwrap_or(1_u64 << 20);
    let out = rerail_metal::lane::buffer_of(device, &vec![0u32; threads as usize]);
    diag_submit(&touch, &out, threads);
    eprintln!("first submission {} MB", mb());
    for _ in 0..10 {
        diag_submit(&touch, &out, threads);
    }
    eprintln!("ten more on one queue {} MB", mb());
    if let Some(metal) = device.metal() {
        let submit = |queue: metal::CommandQueue| {
            let queue = rerail_gpu::device::QueueRoot::Native(queue);
            let mut q = ComputeQueue::new(&queue);
            launch(&q, &touch).buffer("o", &out).run(&[threads.div_ceil(64)], &[64]).expect("the launch");
            q.flush().await_().expect("the queue");
        };
        for _ in 0..10 {
            metal::objc::rc::autoreleasepool(|| submit(metal.new_command_queue()));
        }
        eprintln!("ten new queues, pooled {} MB", mb());
        for _ in 0..10 {
            submit(metal.new_command_queue());
        }
        eprintln!("ten new queues, unpooled {} MB", mb());
    }
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
    let Ok(device) = rerail_metal::lane::device() else { return };
    let mb = crate::code_rule_dsl::prof::footprint_mb;
    let words: usize = std::env::var("RERAIL_DIAG_WORDS").ok().and_then(|s| s.parse().ok()).unwrap_or(1024);
    let out = rerail_metal::lane::buffer_of(device, &vec![0u32; 1 << 20]);
    diag_submit(&diag_kernel("touch", words), &out, 1 << 20);
    eprintln!("after a shallow kernel {} MB", mb());
    let deep = diag_kernel("deep", words);
    eprintln!("compiled, {words} words {} MB", mb());
    diag_submit(&deep, &out, 1 << 20);
    eprintln!("after a {words}-word stack {} MB", mb());
}

/// Diagnostic (O39): the footprint one label kernel's dispatch adds (`RERAIL_DIAG_KERNEL`), with
/// no items, in a fresh process: its private memory, which the driver sizes for every thread in
/// flight and keeps. A source of one's own (`RERAIL_DIAG_SOURCE`) is UKL too.
#[test]
#[ignore = "diagnostic: prints the footprint of one label kernel's dispatch"]
fn a_label_kernel_costs_footprint() {
    let Ok(device) = rerail_metal::lane::device() else { return };
    let Ok(name) = std::env::var("RERAIL_DIAG_KERNEL") else { return };
    let mb = crate::code_rule_dsl::prof::footprint_mb;
    let source = match std::env::var_os("RERAIL_DIAG_SOURCE") {
        Some(path) => std::fs::read_to_string(path).unwrap(),
        None => String::new(),
    };
    // The label kernels through the kernel store, for a mixed batch.
    let kernel = if source.is_empty() {
        rerail_metal::lane::kernel_fixed(super::label_passes::label_source().0, &name, Some(0))
    } else {
        rerail_metal::lane::kernel(&source, &name, None)
    };
    let zero = rerail_metal::lane::buffer_of(device, &vec![0u32; 1 << 18]);
    diag_submit(&diag_kernel("touch", 1), &zero, 1 << 18);
    let before = mb();
    {
        // Every row zero: each buffer the zero buffer, each value zero bytes of its type's width.
        let _scope = rerail_metal::launch::LaunchScope::enter();
        let mut command = rerail_metal::lane::Command::new("diagnostic");
        let mut launch = command.launch(&kernel, "diagnostic");
        for a in kernel.get_argument_info() {
            launch = if a.ty.dims > 0 { launch.buffer(&a.name, &zero) } else { launch.bytes(&a.name, &vec![0u8; a.ty.base_type.width() as usize]) };
        }
        launch.groups([1, 1, 1], [64, 1, 1]);
        command.run().expect("the label kernel's launch");
    }
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
    // `RERAIL_PROBE_NOINLINE=read,read`: those generated reads (`clif_tape_<read>`) compiled out of line;
    // `RERAIL_PROBE_INLINE=read,read`: those inlined.
    let mut source = source;
    for (var, from, to) in [("RERAIL_PROBE_NOINLINE", "inline", "__attribute__((noinline))"), ("RERAIL_PROBE_INLINE", "__attribute__((noinline))", "inline")] {
        for read in std::env::var(var).unwrap_or_default().split(',').filter(|r| !r.is_empty()) {
            let before = source.clone();
            for ty in ["uint", "ulong"] {
                source = source.replace(&format!("{from} {ty} clif_tape_{read}("), &format!("{to} {ty} clif_tape_{read}("));
            }
            assert_ne!(source, before, "clif_tape_{read} is a read marked {from}");
        }
    }
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
            let pipeline = device.new_compute_pipeline_state(&descriptor).expect("the pipeline");
            let pipeline_s = clock.elapsed().as_secs_f64();
            // The thread limit: under 1,024, the kernel's registers cap its occupancy (t41).
            let threads = pipeline.max_total_threads_per_threadgroup();
            eprintln!("  format {format:>2} {name:<18} function {function_s:>6.2} s  pipeline {pipeline_s:>6.2} s  max threads {threads}");
            functions += function_s;
            pipelines += pipeline_s;
        }
        eprintln!("format {format}: functions {functions:.2} s, pipelines {pipelines:.2} s");
    }
}

#[test]
fn the_label_kernels_compile_for_every_format() {
    if rerail_gpu::shared::get().is_none() {
        return;
    }
    for format in [0, 15, 16] {
        let compiled = super::label_passes::compile_label_kernels(format).unwrap_or_else(|_| panic!("the label kernels for format {format}"));
        // 9: each walk's stack tiers are one kernel (work queue Q181), with decision 0365's `label_leaves` and
        // 0475's `label_phase` and `label_parents`.
        assert_eq!(compiled, (9, 1), "format {format}: the label kernels, then the pattern kernel");
    }
}
