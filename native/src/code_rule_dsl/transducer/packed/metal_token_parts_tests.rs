//! `token_parts` recognizes on the device exactly the tokens the host does (RER-131): for every
//! record of a named-only batch, the same tokens at the same places among its parts as
//! `TapeDoc::pattern_children`. `RERAIL_TOKEN_PARTS=<dialect>=<file list>[:<max>];...` adds corpora.

use super::*;
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::tape::RECOGNIZED_TOKEN;

const SAMPLES: [(GrammarDialect, &str); 6] = [
    (GrammarDialect::Python, "async def f(a, b=1):  # c\n    return a if a is not b else [x for x in b]\nimport os, sys\nfrom . import (x, y,)\nx = \\\n  1\n"),
    (GrammarDialect::JavaScript, "const f = async (a, b) => { let x = [1,,2]; return a?.b ?? 0; };\nfor (const k of o) if (!k) break;\n"),
    (GrammarDialect::TypeScript, "const f = async (a, b?) => { let x = [1,,2]; return a?.b ?? 0; };\ninterface I { readonly a: string; }\n"),
    (GrammarDialect::Rust, "fn f(x: &[u8]) -> Option<u8> { if let Some(y) = x.first() { return Some(*y + 1); } None }\n"),
    (GrammarDialect::Php, "<?php\nFUNCTION f($a, $b = [1, 2,]) { return $a ?? $b; }\n"),
    (GrammarDialect::Sql, "SELECT a, b FROM t WHERE a = 1 AND b IN (1, 2);\n"),
];

fn dialect(label: &str) -> GrammarDialect {
    match label {
        "python" => GrammarDialect::Python,
        "javascript" => GrammarDialect::JavaScript,
        "typescript" => GrammarDialect::TypeScript,
        "tsx" => GrammarDialect::Tsx,
        "sql" => GrammarDialect::Sql,
        "rust" => GrammarDialect::Rust,
        "php" => GrammarDialect::Php,
        other => panic!("unknown dialect {other}"),
    }
}

/// Every record's tokens, on the device: per document, per node (slot, kind).
fn device_tokens(files: &[TapeDoc]) -> Vec<Vec<Vec<(u32, u32)>>> {
    let device = METAL_DEVICE.as_ref().expect("a Metal device");
    let table = crate::code_rule_dsl::residency::slabs::admit_docs(files).expect("admitted");
    let table = crate::code_rule_dsl::residency::slabs::DocTable::new(table).expect("within the slabs");
    let nodes = table.nodes;
    let parts = TokenParts::new(device, files, &table.views, &[], true, nodes).expect("buffers").expect("named-only documents");
    let docs = MappedBuffer::from_slice(device, &table.views).expect("docs");
    let flags = MappedBuffer::zeroed::<u32>(device, 6).expect("flags");
    let state = pipelines().expect("pipelines")[TOKEN_PARTS].clone();
    let queue = device.new_command_queue();
    let command = queue.new_command_buffer();
    let encoder = command.new_compute_command_encoder();
    encoder.set_compute_pipeline_state(&state);
    let buffers = parts.buffers(&docs.buffer, &flags.buffer);
    for (index, (buffer, offset)) in buffers.iter().enumerate() {
        encoder.set_buffer(index as u64, Some(buffer), *offset);
    }
    encoder.set_bytes(buffers.len() as u64, std::mem::size_of::<Shape>() as u64, std::ptr::from_ref(&parts.shape).cast());
    for slot in 0..crate::code_rule_dsl::residency::slabs::MAX_SLABS {
        let slabs = &table.slabs;
        let buffer = slabs.get(slot).or(slabs.last()).map_or(&flags.buffer, |slab| &slab.buffer.buffer);
        encoder.set_buffer(SLAB_SLOT + slot as u64, Some(buffer), 0);
    }
    let threads = parts.shape.item_count as u64;
    encoder.dispatch_threads(MTLSize::new(threads, 1, 1), MTLSize::new(state.thread_execution_width().min(threads), 1, 1));
    encoder.end_encoding();
    command.commit();
    command.wait_until_completed();
    assert_eq!(unsafe { *flags.buffer.contents().cast::<u32>() }, 0, "token_parts failed");
    let index = unsafe { std::slice::from_raw_parts(parts.index.buffer.contents().cast::<[u32; 2]>(), nodes) };
    let tokens = unsafe { std::slice::from_raw_parts(parts.tokens.buffer.contents().cast::<u32>(), parts.shape.result_arena as usize) };
    let mut base = 0;
    files
        .iter()
        .map(|file| {
            let per = (0..file.transducer_len())
                .map(|node| {
                    let [first, count] = index[base + node];
                    tokens[first as usize..(first + count) as usize].iter().map(|&token| (token >> 10, token & 1023)).collect()
                })
                .collect();
            base += file.transducer_len();
            per
        })
        .collect()
}

/// The host's: each record's parts that are recognized tokens, (place, kind).
fn host_tokens(file: &TapeDoc) -> Vec<Vec<(u32, u32)>> {
    (0..file.transducer_len() as u32)
        .map(|node| {
            file.pattern_children(node)
                .into_iter()
                .enumerate()
                .filter(|(_, part)| part & RECOGNIZED_TOKEN != 0)
                .map(|(slot, part)| (slot as u32, part & 0xffff))
                .collect()
        })
        .collect()
}

fn check(files: &[TapeDoc], names: &[String]) -> usize {
    let device = device_tokens(files);
    let mut tokens = 0;
    for ((file, name), device) in files.iter().zip(names).zip(device) {
        for (node, (host, device)) in host_tokens(file).into_iter().zip(device).enumerate() {
            assert_eq!(device, host, "{name}: node {node} ({})", file.transducer_kind(node as u32));
            tokens += host.len();
        }
    }
    tokens
}

#[test]
fn the_device_recognizes_the_tokens_the_host_does() {
    let (names, files): (Vec<String>, Vec<TapeDoc>) = SAMPLES
        .iter()
        .filter_map(|(lang, source)| Some((format!("{lang:?}"), TapeDoc::named_only_for_test(*lang, source)?)))
        .unzip();
    assert!(files.len() >= 5, "named-only samples");
    assert!(check(&files, &names) > 50);
}

#[test]
#[ignore = "corpora: RERAIL_TOKEN_PARTS"]
fn the_device_recognizes_the_tokens_the_host_does_on_corpora() {
    let spec = std::env::var("RERAIL_TOKEN_PARTS").expect("RERAIL_TOKEN_PARTS");
    for entry in spec.split(';').filter(|entry| !entry.trim().is_empty()) {
        let (label, list) = entry.split_once('=').expect("<dialect>=<list>");
        let (list, most) = list.split_once(':').map_or((list, usize::MAX), |(list, most)| (list, most.parse().expect("a count")));
        let lang = dialect(label.trim());
        let (mut names, mut files) = (Vec::new(), Vec::new());
        for path in std::fs::read_to_string(list).expect("a file list").lines().take(most) {
            let Ok(source) = std::fs::read_to_string(path) else { continue };
            if let Some(file) = TapeDoc::named_only_for_test(lang, &source) {
                names.push(path.to_owned());
                files.push(file);
            }
        }
        // A batch at a time, as the wave admits them.
        let mut tokens = 0;
        for (names, files) in names.chunks(32).zip(files.chunks(32)) {
            tokens += check(files, names);
        }
        eprintln!("{label}: {} files, {tokens} tokens alike", files.len());
    }
}
