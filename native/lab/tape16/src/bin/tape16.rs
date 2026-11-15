//! The lab's CLI: each command measures one group of 0156's tradeoffs on a capture (`rerail-engine capture-tapes`).
//!
//! ```text
//! tape16 grid  <cap> [--axis A] [--full] [--langs] [--no-check]   T1–T5, E1/E2/E6: bytes, codes, speed, random reads
//!              (A: codec, layout, cut, dict, k, extras, dictcut; all = every one but codec and dictcut)
//! tape16 text  <cap>                                    T6, E5: text levels against the stored source
//! tape16 edits <pairs.cap>                              T7, E4: blocks and rows an edit changes, per identity
//! tape16 writer <cap> (--features lean for A)           E6: format 16's writer against format 15's, on .rr files
//! tape16 keys  <cap>                                    E8: widths every key and count needs
//! tape16 scan  <cap>                                    T5 on Metal: positions, depth, parents, ends from events
//! tape16 dicts <cap>                                    T4: the dictionary size training gives per size asked
//! tape16 base  <cap> [--dump <dir>]                   E9: base relations as rows against the tape
//! tape16 textrel <cap>                                  E9: text and token as rows with atoms, against the tape
//! tape16 inplace <cap>                                E10: in-memory tapes read in place, every question costed
//! tape16 disk  <cap>                                   E18: the forms on disk, zstd and LZ4: bytes, decode, matches
//! tape16 ralz  <cap>                                   E19: LZ4 in fixed blocks, stored as repeats, raw or LZ4
//! tape16 code8 <cap>                                   E20: codes in 8 bits, overloaded by the adjacent code; E21: numbered by frequency, one cutoff
//! tape16 f16     <cap>                                 E24: format 16's tape as one form: memory, disk, load
//! tape16 nofield <cap>                                 E22: the field taken out of the code
//! tape16 gpu   <cap> [--dict KB] [--k K] [--cut N]      E3: the LZ codecs decoded on Metal
//! ```

use rerail_tape_lab::blocks::{self, Cut};
use rerail_tape_lab::codec::{self, Codec};
use rerail_tape_lab::span::Ptr;
use rerail_tape_lab::layout::{Extras, Layout};
use rerail_tape_lab::solution::{self, Corpus, Dicts, Solution};
use rerail_tape_lab::{capture, edits, events, layout, report, text};
use std::collections::{BTreeMap, HashMap};


fn base() -> Solution {
    Solution { layout: Layout::BytePlanes, codec: Codec::LpLz, cut: Cut::Whole, dict: 64 * 1024, k: 64, extras: Extras::default(), entropy: false }
}

/// One axis at a time from the base (`axis`: layout, cut, dict, k, extras, or all), or (`full`) every
/// combination of layout, codec, cut and dictionary size.
fn grid(full: bool, axis: &str) -> Vec<Solution> {
    let b = base();
    let mut v = vec![b];
    let push = |s: Solution, v: &mut Vec<Solution>| {
        if !v.contains(&s) {
            v.push(s);
        }
    };
    let cuts = [Cut::Whole, Cut::Target(256), Cut::Target(1024), Cut::Target(4096)];
    let dicts = [0, 16 * 1024, 64 * 1024, 256 * 1024];
    if full {
        for layout in Layout::ALL {
            for codec in Codec::ALL {
                for cut in cuts {
                    for dict in dicts {
                        push(Solution { layout, codec, cut, dict, ..b }, &mut v);
                    }
                }
            }
        }
        return v;
    }
    let on = |a: &str| axis == "all" || axis == a;
    if axis == "codec" {
        for cut in [Cut::Whole, Cut::Target(256), Cut::Target(1024)] {
            for codec in Codec::ALL.into_iter().filter(|c| *c != Codec::Raw) {
                push(Solution { codec, cut, ..b }, &mut v);
            }
        }
    }
    if on("layout") {
        for layout in Layout::ALL {
            for codec in Codec::ALL {
                push(Solution { layout, codec, ..b }, &mut v);
            }
        }
    }
    let codecs = [Codec::Zstd, Codec::LpLz, Codec::LpReplay];
    if on("cut") {
        for cut in cuts {
            for codec in codecs {
                push(Solution { cut, codec, ..b }, &mut v);
            }
        }
    }
    if on("dict") {
        for dict in dicts {
            for codec in codecs {
                push(Solution { dict, codec, ..b }, &mut v);
            }
        }
    }
    if on("k") {
        for k in [16, 64, 256] {
            for codec in [Codec::LpLz, Codec::LpReplay] {
                push(Solution { k, codec, ..b }, &mut v);
            }
        }
    }
    if axis == "final" {
        // The candidates the measurements so far point at, for a whole-repository run.
        v.clear();
        push(Solution { codec: Codec::Zstd, ..b }, &mut v);
        push(Solution { codec: Codec::Zstd, cut: Cut::Target(1024), dict: 16 * 1024, ..b }, &mut v);
        push(Solution { codec: Codec::LzOutput, cut: Cut::Target(1024), dict: 16 * 1024, entropy: true, ..b }, &mut v);
        push(Solution { codec: Codec::span(4, true), cut: Cut::Target(1024), dict: 16 * 1024, entropy: true, ..b }, &mut v);
        push(Solution { codec: Codec::span(4, true), cut: Cut::Target(256), dict: 16 * 1024, entropy: true, ..b }, &mut v);
        let seq = Codec::LpSpan { depth: 4, cross: true, ptr: Ptr::Seq };
        push(Solution { codec: seq, cut: Cut::Target(1024), dict: 16 * 1024, entropy: true, ..b }, &mut v);
        push(Solution { codec: seq, cut: Cut::Target(256), dict: 16 * 1024, entropy: true, ..b }, &mut v);
    }
    if axis == "dictcut" {
        for cut in [Cut::Target(256), Cut::Target(1024)] {
            for dict in [0, 16 * 1024, 64 * 1024] {
                push(Solution { codec: Codec::Zstd, cut, dict, ..b }, &mut v);
                for codec in [Codec::LzOutput, Codec::span(4, true)] {
                    push(Solution { codec, cut, dict, entropy: true, ..b }, &mut v);
                }
            }
        }
    }
    if axis == "span" {
        // How a span names its target: the pointer forms, with and without the entropy stage.
        for cut in [Cut::Whole, Cut::Target(1024), Cut::Target(256)] {
            for ptr in Ptr::ALL {
                for entropy in [false, true] {
                    push(Solution { codec: Codec::LpSpan { depth: 4, cross: true, ptr }, cut, dict: 16 * 1024, entropy, ..b }, &mut v);
                }
            }
            push(Solution { codec: Codec::LzOutput, cut, dict: 16 * 1024, entropy: true, ..b }, &mut v);
        }
    }
    if axis == "huff" {
        for cut in [Cut::Whole, Cut::Target(1024)] {
            for codec in [Codec::LzOutput, Codec::LpLz, Codec::LpReplay, Codec::span(4, false), Codec::span(4, true)] {
                for entropy in [false, true] {
                    push(Solution { codec, cut, entropy, ..b }, &mut v);
                }
            }
            push(Solution { codec: Codec::Zstd, cut, ..b }, &mut v);
        }
    }
    if on("extras") {
        for extras in [Extras { parent: true, size: false }, Extras { parent: false, size: true }, Extras { parent: true, size: true }] {
            for codec in codecs {
                push(Solution { extras, codec, ..b }, &mut v);
            }
        }
    }
    v
}

fn load(path: &str) -> Corpus {
    let files = capture::read(std::path::Path::new(path)).unwrap_or_else(|e| die(&e));
    solution::prepare(files).unwrap_or_else(|e| die(&e))
}

fn die(e: &str) -> ! {
    eprintln!("tape16: {e}");
    std::process::exit(1)
}

/// The grid over a capture too big to hold: candidates from every file, a fixed training subsample (about
/// `TRAIN_NODES` per language) for dictionaries and tables, then the test half in `shards` shards, rows added.
fn cmd_grid_streamed(path: &str, full: bool, axis: &str, check: bool, shards: u64, only: Option<usize>) {
    const TRAIN_NODES: u64 = 1_500_000;
    let p = std::path::Path::new(path);
    let mut cand = rerail_tape_lab::events::Candidates::default();
    let mut train_nodes: BTreeMap<u8, u64> = BTreeMap::new();
    let (mut files, mut nodes) = (0usize, 0u64);
    capture::stream(p, |f| {
        let t = rerail_tape_lab::v15::read(&f.image)?;
        cand.learn(f.tag, &t);
        files += 1;
        if !f.is_test() {
            *train_nodes.entry(f.tag).or_default() += t.len() as u64;
        } else {
            nodes += t.len() as u64;
        }
        Ok(())
    })
    .unwrap_or_else(|e| die(&e));
    let rate: BTreeMap<u8, u64> = train_nodes.iter().map(|(t, n)| (*t, n.div_ceil(TRAIN_NODES).max(1))).collect();
    let mut train = Vec::new();
    capture::stream(p, |f| {
        if !f.is_test() && capture::path_hash(&f.path, 1) % rate[&f.tag] == 0 {
            train.push(f);
        }
        Ok(())
    })
    .unwrap_or_else(|e| die(&e));
    println!("{path}: {files} files, {nodes} test nodes in {shards} shards; training on {} files (1 in {:?} by language)\n", train.len(), rate.values().collect::<Vec<_>>());
    let mut sols = grid(full, axis);
    if let Some(i) = only {
        sols = vec![sols[i]];
    }
    let mut rows: Vec<solution::Row> = sols.iter().map(|_| solution::Row::default()).collect();
    let mut dicts = Dicts::default();
    for shard in 0..shards {
        let mut part: Vec<capture::File> = train.iter().map(|f| capture::File { path: f.path.clone(), tag: f.tag, source: f.source.clone(), image: f.image.clone() }).collect();
        capture::stream(p, |f| {
            if f.is_test() && capture::path_hash(&f.path, 2) % shards == shard {
                part.push(f);
            }
            Ok(())
        })
        .unwrap_or_else(|e| die(&e));
        let corpus = solution::prepare_with(part, Some(cand.clone())).unwrap_or_else(|e| die(&e));
        for (s, row) in sols.iter().zip(&mut rows) {
            row.merge(&solution::evaluate(&corpus, s, &mut dicts, check));
        }
        eprintln!("  shard {shard} done");
    }
    report::print_grid(&rows);
    report::print_lang_matrix(&rows);
}

fn cmd_grid(path: &str, full: bool, axis: &str, langs: bool, check: bool) {
    let corpus = load(path);
    let test = corpus.files.iter().filter(|p| p.test).count();
    let nodes: usize = corpus.files.iter().filter(|p| p.test).map(|p| p.nodes).sum();
    println!("{path}: {} files, {test} in the test half ({nodes} nodes)\n", corpus.files.len());
    let mut dicts = Dicts::default();
    let mut rows = Vec::new();
    for s in grid(full, axis) {
        let r = solution::evaluate(&corpus, &s, &mut dicts, check);
        eprintln!("  {}", s.name());
        rows.push(r);
    }
    report::print_grid(&rows);
    if langs {
        report::print_langs(&rows[0]);
    }
    report::print_lang_matrix(&rows);
    println!("\nWidest supertype masks (language: kind with the most candidates):");
    for (tag, (n, kind)) in corpus.candidates.widest() {
        println!("  {}: {n} candidates (kind {kind})", capture::language(tag));
    }
}

/// T4: what the trainer gives for each asked size, per language (zstd's trainer may return less).
fn cmd_dicts(path: &str) {
    let corpus = load(path);
    let mut dicts = Dicts::default();
    println!("| language | asked KB | got KB |");
    println!("|---|---|---|");
    let tags: std::collections::BTreeSet<u8> = corpus.files.iter().map(|p| p.tag).collect();
    for tag in tags {
        for kb in [16, 64, 256] {
            let d = dicts.get(&corpus, tag, &Solution { dict: kb * 1024, ..base() });
            println!("| {} | {kb} | {:.1} |", capture::language(tag), d.bytes.len() as f64 / 1024.0);
        }
    }
}

fn zstd_len(b: &[u8]) -> usize {
    zstd::bulk::compress(b, 3).map_or(b.len(), |c| c.len())
}

fn cmd_text(path: &str) {
    let corpus = load(path);
    println!("| language | level | table entries | table KB | ids+pool zstd B/node | pool KB | ×source stored | gaps in table % | leaves in table % | exact |");
    println!("|---|---|---|---|---|---|---|---|---|---|");
    let mut by: BTreeMap<u8, Vec<&solution::Prepared>> = BTreeMap::new();
    for p in &corpus.files {
        by.entry(p.tag).or_default().push(p);
    }
    for (tag, files) in by {
        let mut counts: HashMap<Vec<u8>, u64> = HashMap::new();
        for p in files.iter().filter(|p| !p.test) {
            for (_, t) in text::pieces(&p.events, &p.source) {
                *counts.entry(t.to_vec()).or_insert(0) += 1;
            }
        }
        for entries in [0usize, 4096, 65536] {
            let table = text::Table::train(&counts, entries);
            for (level, leaves) in [("names", true), ("source", false)] {
                let (mut bytes, mut pool, mut nodes, mut src) = (0usize, 0usize, 0usize, 0usize);
                let (mut g, mut gt, mut l, mut lt) = (0, 0, 0, 0);
                let mut exact = true;
                for p in files.iter().filter(|p| p.test) {
                    let pieces = text::pieces(&p.events, &p.source);
                    let s = text::stream(&pieces, &table, leaves);
                    bytes += zstd_len(&text::id_planes(&s.ids)) + zstd_len(&s.pool);
                    pool += s.pool.len();
                    nodes += p.nodes;
                    src += p.source_stored;
                    (g, gt, l, lt) = (g + s.gaps, gt + s.gaps_in_table, l + s.leaves, lt + s.leaves_in_table);
                    if !leaves && text::rebuild(&s, &table) != p.source {
                        exact = false;
                    }
                }
                println!(
                    "| {} | {level} | {} | {} | {:.2} | {} | {:.2} | {:.1} | {:.1} | {} |",
                    capture::language(tag),
                    table.ids.len(),
                    table.bytes / 1024,
                    bytes as f64 / nodes.max(1) as f64,
                    pool / 1024,
                    bytes as f64 / src.max(1) as f64,
                    100.0 * gt as f64 / g.max(1) as f64,
                    100.0 * lt as f64 / l.max(1) as f64,
                    if leaves { "-" } else if exact { "yes" } else { "NO" },
                );
            }
        }
    }
}

fn cmd_edits(path: &str) {
    let files = capture::read(std::path::Path::new(path)).unwrap_or_else(|e| die(&e));
    let corpus = solution::prepare(files).unwrap_or_else(|e| die(&e));
    let by_path: HashMap<&str, &solution::Prepared> = corpus.files.iter().map(|p| (p.path.as_str(), p)).collect();
    let pairs: Vec<_> = corpus
        .files
        .iter()
        .filter(|p| p.path.contains("/old/"))
        .filter_map(|old| by_path.get(old.path.replacen("/old/", "/new/", 1).as_str()).map(|new| (old, *new)))
        .collect();
    println!("{path}: {} pairs\n", pairs.len());
    let lay = |b: &blocks::Block| layout::lay(b, Layout::BytePlanes, Extras::default(), &[], &[]);
    println!("| cut | pairs | new nodes | replaced % | ancestors % | blocks new % | block bytes new % | span moved % | {} |", edits::IDENTITIES.iter().map(|i| format!("{i}: keys / links changed %")).collect::<Vec<_>>().join(" | "));
    println!("|{}", "---|".repeat(8 + 4));
    for cut in [Cut::Whole, Cut::Target(256), Cut::Target(1024), Cut::Target(4096)] {
        let mut st = edits::EditStats::default();
        for (old, new) in &pairs {
            edits::compare(&old.tree(&corpus), &new.tree(&corpus), &old.source, &new.source, &old.events, &new.events, cut, &lay, &mut st);
        }
        let pct = |a: u64, b: u64| 100.0 * a as f64 / b.max(1) as f64;
        let ids: Vec<String> = (0..4).map(|i| format!("{:.2} / {:.2}", pct(st.key_changed[i], st.new_nodes), pct(st.link_changed[i], st.new_nodes))).collect();
        println!(
            "| {} | {} | {} | {:.2} | {:.2} | {:.1} | {:.1} | {:.1} | {} |",
            cut.name(),
            st.pairs,
            st.new_nodes,
            pct(st.replaced, st.new_nodes),
            pct(st.ancestors, st.new_nodes),
            pct(st.blocks_new, st.blocks_total),
            pct(st.block_bytes_new, st.tape_bytes),
            pct(st.span_moved, st.new_nodes),
            ids.join(" | "),
        );
        if cut == Cut::Whole {
            println!("|  | (D: {} shift ops over {} pairs, covering {} rows) |", st.shift_ops, st.pairs, st.shifted_rows);
        }
    }
}

fn bits(v: u64) -> u32 {
    64 - v.leading_zeros()
}

fn cmd_keys(path: &str) {
    let corpus = load(path);
    let (mut max_nodes, mut max_src, mut total) = (0u64, 0u64, 0u64);
    let (mut max_lead, mut max_kind, mut max_field, mut max_pops) = (0u32, 0u16, 0u8, 0u32);
    for p in &corpus.files {
        max_nodes = max_nodes.max(p.nodes as u64);
        max_src = max_src.max(p.source.len() as u64);
        total += p.nodes as u64;
        for o in &p.events.opens {
            max_lead = max_lead.max(o.lead & !events::BACK);
            max_kind = max_kind.max(o.kind);
            max_field = max_field.max(o.field);
            max_pops = max_pops.max(o.pops);
        }
        for &t in &p.events.trails {
            max_lead = max_lead.max(t & !events::BACK);
        }
    }
    println!("| quantity | max | bits |");
    println!("|---|---|---|");
    for (name, v) in [
        ("nodes in a file (A: ordinal)", max_nodes),
        ("nodes in the corpus", total),
        ("source bytes in a file", max_src),
        ("event delta (lead/trail)", u64::from(max_lead)),
        ("kind", u64::from(max_kind)),
        ("field", u64::from(max_field)),
        ("closes before one open (pops)", u64::from(max_pops)),
    ] {
        println!("| {name} | {v} | {} |", bits(v));
    }
    for cut in [Cut::Target(256), Cut::Target(1024), Cut::Target(4096)] {
        let (mut blocks, mut max_local) = (0u64, 0u64);
        for p in &corpus.files {
            let (bs, _) = blocks::split(&p.events, cut);
            blocks = blocks.max(bs.len() as u64);
            for b in &bs {
                max_local = max_local.max(b.opens.len() as u64);
            }
        }
        println!("| B at {}: blocks in a file / nodes in a block | {blocks} / {max_local} | {} + {} |", cut.name(), bits(blocks), bits(max_local));
    }
}

#[cfg(target_os = "macos")]
/// `lang`: one language only (its name, as the tables print it); `codecs`: only codecs whose name holds it.
fn cmd_gpu(path: &str, dict_kb: usize, k: usize, cut: Cut, lang: Option<&str>, only: Option<&str>) {
    use rerail_tape_lab::lp::Refs;
    let corpus = load(path);
    let mut dicts = Dicts::default();
    println!("Blocks: {}, dictionary {dict_kb} KB, K {k}\n", cut.name());
    println!("| language | codec | threads | MB out | GPU µs | GB/s | CPU decode µs | correct |");
    println!("|---|---|---|---|---|---|---|---|");
    let tags: std::collections::BTreeSet<u8> =
        corpus.files.iter().map(|p| p.tag).filter(|&t| lang.is_none_or(|l| capture::language(t) == l)).collect();
    let codecs = [
        Codec::LpLz,
        Codec::LpReplay,
        Codec::span(1, false),
        Codec::span(4, false),
        Codec::span(4, true),
        Codec::LpSpan { depth: 4, cross: true, ptr: Ptr::Split },
        Codec::LpSpan { depth: 4, cross: true, ptr: Ptr::SplitNoLen },
        Codec::LpSpan { depth: 4, cross: true, ptr: Ptr::Seq },
        Codec::LzOutput,
    ];
    for tag in tags {
        for c in codecs.into_iter().filter(|c| only.is_none_or(|o| c.name().contains(o))) {
            let s = Solution { codec: c, dict: dict_kb * 1024, k, cut, ..base() };
            let dict = dicts.get(&corpus, tag, &s);
            // Per file: its blocks laid out, and compressed as one file (a span may reach an earlier block).
            let files: Vec<Vec<Vec<u8>>> = corpus
                .files
                .iter()
                .filter(|p| p.test && p.tag == tag)
                .map(|p| blocks::split(&p.events, s.cut).0.iter().map(|b| layout::lay(b, s.layout, s.extras, &[], &[])).collect())
                .collect();
            let comp: Vec<Vec<Vec<u8>>> = files.iter().map(|f| codec::compress_file(c, f, &dict, k)).collect();
            let (_, cpu) = rerail_tape_lab::measure::cost(|| {
                for (cf, lf) in comp.iter().zip(&files) {
                    let lens: Vec<usize> = lf.iter().map(Vec::len).collect();
                    std::hint::black_box(codec::decompress_file(c, cf, &dict, &lens));
                }
            });
            drop(files);
            // Each run in its own pool: Metal's command buffers are autoreleased, and hold their buffers.
            // The sequence-index form runs twice: walking from checkpoints, and with the index pass.
            let modes: &[bool] = if matches!(c, Codec::LpSpan { ptr: Ptr::Seq, .. }) { &[false, true] } else { &[false] };
            for &index in modes {
                let label = if index { format!("{}+index", c.name()) } else { c.name() };
                let run = metal::objc::rc::autoreleasepool(|| match c {
                    Codec::LpSpan { ptr, .. } => rerail_tape_lab::gpu::decode_span(&comp, &dict.bytes, ptr, index),
                    Codec::LpLz => rerail_tape_lab::gpu::decode(&comp.concat(), &dict.bytes, Refs::Literals),
                    Codec::LpReplay => rerail_tape_lab::gpu::decode(&comp.concat(), &dict.bytes, Refs::Replay),
                    _ => rerail_tape_lab::gpu::decode(&comp.concat(), &dict.bytes, Refs::Output),
                });
                if let Codec::LpSpan { ptr, .. } = c {
                    let t = dicts.tables(&corpus, tag, &Solution { entropy: true, ..s });
                    let coded: Vec<Vec<Vec<u8>>> = comp.iter().map(|f| f.iter().map(|b| rerail_tape_lab::huff::encode(b, &t.0, &t.1)).collect()).collect();
                    match metal::objc::rc::autoreleasepool(|| rerail_tape_lab::gpu::decode_span_huff(&coded, &dict.bytes, &t.0, &t.1, ptr, index)) {
                        Ok(g) => println!(
                            "| {} | {label}+huff | {} | {:.2} | {:.0} + {:.0}{} | {:.2} | | {} |",
                            capture::language(tag),
                            g.threads,
                            g.bytes_out as f64 / 1e6,
                            g.huff_seconds * 1e6,
                            g.lz_seconds * 1e6,
                            if index { format!(" (index {:.0})", g.index_seconds * 1e6) } else { String::new() },
                            g.bytes_out as f64 / (g.huff_seconds + g.lz_seconds).max(1e-9) / 1e9,
                            g.correct
                        ),
                        Err(e) => println!("| {} | {label}+huff | error: {e} |", capture::language(tag)),
                    }
                }
                match run {
                    Ok(g) => println!(
                        "| {} | {label} | {} | {:.2} | {:.0}{} | {:.2} | {:.0} | {} |",
                        capture::language(tag),
                        g.threads,
                        g.bytes_out as f64 / 1e6,
                        g.gpu_seconds * 1e6,
                        if index { format!(" (index {:.0})", g.index_seconds * 1e6) } else { String::new() },
                        g.bytes_out as f64 / g.gpu_seconds.max(1e-9) / 1e9,
                        cpu.nanos as f64 / 1000.0,
                        g.correct
                    ),
                    Err(e) => println!("| {} | {label} | error: {e} |", capture::language(tag)),
                }
            }
        }
    }
}

/// T5 on the GPU: positions, depth, parents and ends derived from events and placements, checked against the
/// CPU's decode, per block size.
#[cfg(target_os = "macos")]
fn cmd_scan(path: &str) {
    use rerail_tape_lab::scan;
    let corpus = load(path);
    let test: Vec<&solution::Prepared> = corpus.files.iter().filter(|p| p.test).collect();
    // Trees are decoded one at a time (all at once would not fit the lab's 1 GB).
    let nodes: usize = test.iter().map(|p| p.nodes).sum();
    let max_depth = test.iter().map(|p| p.tree(&corpus).depth().into_iter().max().unwrap_or(0)).max().unwrap_or(0) as usize + 1;
    // The CPU's form: one stack pass per file (`events::decode`).
    let (_, cpu) = rerail_tape_lab::measure::cost(|| {
        for p in &test {
            std::hint::black_box(p.tree(&corpus));
        }
    });
    println!("{path}: {} test files, {nodes} nodes, depth up to {max_depth}; the CPU's decode (one thread) {:.1} ms\n", test.len(), cpu.nanos as f64 / 1e6);
    println!("| blocks | count | unresolved closes | pass A µs | pass B µs | pass C µs | closes µs | total µs | ns/node | correct |");
    println!("|---|---|---|---|---|---|---|---|---|---|");
    for cut in [Cut::Whole, Cut::Target(4096), Cut::Target(1024), Cut::Target(256), Cut::Target(64)] {
        let files: Vec<scan::FileBlocks> = test
            .iter()
            .map(|p| {
                let (bs, places) = blocks::split(&p.events, cut);
                scan::FileBlocks { events: &p.events, places, counts: bs.iter().map(|b| (b.opens.len() as u32, b.trails.len() as u32)).collect() }
            })
            .collect();
        let r = metal::objc::rc::autoreleasepool(|| scan::derive(&files, max_depth));
        match r {
            Ok(d) => {
                let mut ok = true;
                let mut base = 0usize;
                for p in &test {
                    let t = &p.tree(&corpus);
                    let depth = t.depth();
                    for i in 0..t.len() {
                        let g = base + i;
                        let parent = if t.parent[i] == u32::MAX { u32::MAX } else { t.parent[i] + base as u32 };
                        let sub = (g + t.size[i] as usize) as u32;
                        if d.start[g] != t.nodes[i].start || d.end[g] != t.nodes[i].end || d.depth[g] != depth[i] || d.parent[g] != parent || d.sub[g] != sub {
                            ok = false;
                        }
                    }
                    base += t.len();
                }
                let total: f64 = d.seconds.iter().sum();
                println!(
                    "| {} | {} | {} | {:.0} | {:.0} | {:.0} | {:.0} | {:.0} | {:.2} | {} |",
                    cut.name(),
                    d.blocks,
                    d.unresolved_closes,
                    d.seconds[0] * 1e6,
                    d.seconds[1] * 1e6,
                    d.seconds[2] * 1e6,
                    d.seconds[3] * 1e6,
                    total * 1e6,
                    total * 1e9 / nodes as f64,
                    ok
                );
            }
            Err(e) => println!("| {} | error: {e} |", cut.name()),
        }
    }
}

/// E7's reader form on the GPU: format 16 (span LZ, sequence-index pointers, Huffman, byte planes) decoded
/// straight into columns (kind, field, flags, membership, start, end, parent, subtree end), per language and
/// then every language in one set of dispatches, checked against the tree; against the same decode on one CPU
/// thread into a tree.
#[cfg(target_os = "macos")]
fn cmd_columns(path: &str, cut: Cut, lang: Option<&str>) {
    use rerail_tape_lab::columns;
    let corpus = load(path);
    let mut dicts = Dicts::default();
    let s = Solution { codec: Codec::LpSpan { depth: 4, cross: true, ptr: Ptr::Seq }, cut, dict: 16 * 1024, entropy: true, ..base() };
    println!("{path}: {}\n", s.name());
    print!("| language | nodes | blocks | threads | MB in | MB laid |");
    for p in columns::PASSES {
        print!(" {p} µs |");
    }
    println!(" GPU total µs | GPU ns/node | CPU ms (one thread, into a tree) | CPU ns/node | correct |");
    println!("|---|---|---|---|---|---|{}---|---|---|---|---|", "---|".repeat(columns::PASSES.len()));
    let tags: Vec<u8> = corpus
        .files
        .iter()
        .filter(|p| p.test)
        .map(|p| p.tag)
        .filter(|&t| lang.is_none_or(|l| capture::language(t) == l))
        .collect::<std::collections::BTreeSet<u8>>()
        .into_iter()
        .collect();
    struct F {
        lang: usize,
        coded: Vec<Vec<u8>>,
        places: Vec<rerail_tape_lab::blocks::Placement>,
        counts: Vec<(u32, u32)>,
        lens: Vec<usize>,
    }
    let mut per: Vec<(Vec<u8>, std::sync::Arc<(rerail_tape_lab::huff::Table, rerail_tape_lab::huff::Table)>)> = Vec::new();
    let (mut all, mut all_test, mut all_cpu, mut max_all) = (Vec::<F>::new(), Vec::<&solution::Prepared>::new(), 0u64, 0usize);
    for (li, &tag) in tags.iter().enumerate() {
        let dict = dicts.get(&corpus, tag, &s);
        let t = dicts.tables(&corpus, tag, &s);
        let test: Vec<&solution::Prepared> = corpus.files.iter().filter(|p| p.test && p.tag == tag).collect();
        let files: Vec<F> = test
            .iter()
            .map(|p| {
                let (bs, places) = blocks::split(&p.events, s.cut);
                let laid: Vec<Vec<u8>> = bs.iter().map(|b| layout::lay(b, s.layout, s.extras, &[], &[])).collect();
                let comp = codec::compress_file(s.codec, &laid, &dict, s.k);
                let coded = comp.iter().map(|b| rerail_tape_lab::huff::encode(b, &t.0, &t.1)).collect();
                F { lang: li, coded, places, counts: bs.iter().map(|b| (b.opens.len() as u32, b.trails.len() as u32)).collect(), lens: laid.iter().map(Vec::len).collect() }
            })
            .collect();
        let max_depth = test.iter().map(|p| p.tree(&corpus).depth().into_iter().max().unwrap_or(0)).max().unwrap_or(0) as usize + 1;
        // The CPU: the same bytes into a tree, one thread (Huffman, span LZ, unlay, events).
        let (_, cpu) = rerail_tape_lab::measure::cost(|| {
            for (f, p) in files.iter().zip(&test) {
                let plain: Vec<Vec<u8>> = f.coded.iter().map(|c| rerail_tape_lab::huff::decode_block(c, &t.0, &t.1)).collect();
                let back: Vec<rerail_tape_lab::blocks::Block> =
                    codec::decompress_file(s.codec, &plain, &dict, &f.lens).iter().map(|b| layout::unlay(b, s.layout, s.extras)).collect();
                std::hint::black_box(events::decode(&blocks::join(&back), p.tag, &corpus.candidates, p.source.len() as u64));
            }
        });
        per.push((dict.bytes.clone(), t.clone()));
        let langs = [columns::Lang { dict: &per[li].0, tables: &per[li].1 }];
        let ins: Vec<columns::FileIn> = files.iter().map(|f| columns::FileIn { lang: 0, coded: &f.coded, places: &f.places, counts: &f.counts }).collect();
        columns_row(capture::language(tag), &corpus, &test, &ins, &langs, max_depth, cpu.nanos);
        all_cpu += cpu.nanos;
        max_all = max_all.max(max_depth);
        all.extend(files);
        all_test.extend(test);
    }
    if tags.len() > 1 {
        let langs: Vec<columns::Lang> = per.iter().map(|(d, t)| columns::Lang { dict: d, tables: t }).collect();
        let ins: Vec<columns::FileIn> = all.iter().map(|f| columns::FileIn { lang: f.lang, coded: &f.coded, places: &f.places, counts: &f.counts }).collect();
        columns_row("all, one dispatch", &corpus, &all_test, &ins, &langs, max_all, all_cpu);
    }
}

/// One row of `columns`: the pipeline over `ins`, checked against `test`'s events and trees.
#[cfg(target_os = "macos")]
fn columns_row(name: &str, corpus: &Corpus, test: &[&solution::Prepared], ins: &[rerail_tape_lab::columns::FileIn], langs: &[rerail_tape_lab::columns::Lang], max_depth: usize, cpu_nanos: u64) {
    use rerail_tape_lab::columns;
    let r = metal::objc::rc::autoreleasepool(|| columns::decode(ins, langs, Ptr::Seq, max_depth));
    let c = match r {
        Ok(c) => c,
        Err(e) => {
            println!("| {name} | error: {e} |");
            return;
        }
    };
    let (mut ok, mut base) = (true, 0usize);
    for p in test {
        let tr = p.tree(corpus);
        for (i, o) in p.events.opens.iter().enumerate() {
            let g = base + i;
            let parent = if tr.parent[i] == u32::MAX { u32::MAX } else { tr.parent[i] + base as u32 };
            ok &= c.kind[g] == o.kind && c.field[g] == o.field && c.flags[g] == o.flags && c.memb[g] == o.memb;
            ok &= c.derived.start[g] == tr.nodes[i].start && c.derived.end[g] == tr.nodes[i].end && c.derived.parent[g] == parent;
            ok &= c.derived.sub[g] == (g + tr.size[i] as usize) as u32;
        }
        base += tr.len();
    }
    let nodes = base.max(1);
    let total: f64 = c.seconds.iter().sum();
    print!("| {name} | {nodes} | {} | {} | {:.2} | {:.2} |", c.derived.blocks, c.threads, c.bytes_in as f64 / 1e6, c.laid_bytes as f64 / 1e6);
    for x in c.seconds {
        print!(" {:.0} |", x * 1e6);
    }
    println!(" {:.0} | {:.2} | {:.1} | {:.1} | {ok} |", total * 1e6, total * 1e9 / nodes as f64, cpu_nanos as f64 / 1e6, cpu_nanos as f64 / nodes as f64);
}

/// E6: format 16's writer against format 15's on the captured `.rr` files (tag 13); with `--features lean`,
/// the lean parse too (A), else A is left out.
fn cmd_writer(path: &str) {
    use rerail_tape_lab::measure::cost;
    use rerail_tape_lab::writer;
    let files: Vec<capture::File> = capture::read(std::path::Path::new(path)).unwrap_or_else(|e| die(&e)).into_iter().filter(|f| f.tag == 13).collect();
    #[allow(unused_mut)]
    let (mut a, mut b, mut c) = (rerail_tape_lab::measure::Cost::default(), rerail_tape_lab::measure::Cost::default(), rerail_tape_lab::measure::Cost::default());
    let (mut nodes, mut bytes, mut same) = (0usize, 0usize, 0usize);
    for f in &files {
        let recs = writer::records(&f.image).unwrap_or_else(|e| die(&format!("{}: {e}", f.path)));
        nodes += recs.len();
        bytes += f.source.len();
        #[cfg(feature = "lean")]
        {
            let src = std::str::from_utf8(&f.source).unwrap_or("");
            let (img, x) = cost(|| rerail_rr_syntax::lean::tape(src));
            a += x;
            same += (img == f.image) as usize;
        }
        let (img, x) = cost(|| rerail_tape_format::preorder_image(f.tag, &f.source, &recs));
        b += x;
        if !cfg!(feature = "lean") {
            same += (img == f.image) as usize;
        }
        let (planes, x) = cost(|| std::hint::black_box(writer::events_writer(f.source.len() as u64, &recs)));
        c += x;
        if planes != writer::events_writer_reference(f.source.len() as u64, &recs) {
            die(&format!("{}: the one-pass writer differs from the reference", f.path));
        }
    }
    let m = |x: &rerail_tape_lab::measure::Cost| x.instructions as f64 / 1e6;
    println!("{path}: {} .rr files, {nodes} nodes, {} KB of source; images equal to the capture: {same}\n", files.len(), bytes / 1024);
    println!("| part | M instructions | instructions/node | ms |");
    println!("|---|---|---|---|");
    if cfg!(feature = "lean") {
        println!("| A: lean parse + format-15 write | {:.1} | {:.0} | {:.1} |", m(&a), a.instructions as f64 / nodes as f64, a.nanos as f64 / 1e6);
    }
    println!("| B: format-15 writer alone | {:.1} | {:.0} | {:.1} |", m(&b), b.instructions as f64 / nodes as f64, b.nanos as f64 / 1e6);
    println!("| C: format-16 writer alone (events + byte planes) | {:.1} | {:.0} | {:.1} |", m(&c), c.instructions as f64 / nodes as f64, c.nanos as f64 / 1e6);
    if cfg!(feature = "lean") {
        let e = a.instructions as f64 - b.instructions as f64 + c.instructions as f64;
        println!("| A − B + C: lean parse + format-16 write | {:.1} | {:.0} | |", e / 1e6, e / nodes as f64);
    }
}

/// E9 (T8): the engine's base relations as copied rows against the tape kept and rebuilt (`base`).
fn cmd_base(path: &str, dump: Option<&str>) {
    use rerail_tape_lab::base::{self, NAMES, WIDTHS};
    use rerail_tape_lab::measure::Cost;
    use rerail_tape_lab::measure::cost;
    use rerail_tape_lab::tree::NONE;
    let corpus = load(path);
    let nodes: usize = corpus.files.iter().map(|p| p.nodes).sum();
    // Bytes: rows per relation; a block per document (a memo entry's) and per corpus (a kept frame's, sized as one
    // block: the encoding is a column at a time, so its size streams, each column's last value carried on).
    let mut count = [0usize; 5];
    let mut per_doc = [0usize; 5];
    let mut corpus_bytes = [0usize; 5];
    let mut last: Vec<Vec<i64>> = WIDTHS.iter().map(|&w| vec![0i64; w]).collect();
    let (mut v15, mut ev_zstd) = (0usize, 0usize);
    // Rebuild costs, one thread: rows decoded (+ span_within sorted by outer, the other probe's order); the tape's
    // events decoded to a tree (`events::decode`) and its reader columns derived.
    let (mut decode_rows, mut sort_outer, mut decode_tape, mut derive) = (Cost::default(), Cost::default(), Cost::default(), Cost::default());
    // Probes, each node once a pattern: ns summed.
    let mut probe = [[0u128; 2]; 5];
    let mut probes = [0usize; 5];
    let mut sink = 0i64;
    let mut got = [[0i64; 2]; 5];
    // `--dump <dir>`: every relation's rows over the corpus, for the engine's own probes (`base_probe`).
    let mut dumped: Option<Vec<std::io::BufWriter<std::fs::File>>> = dump.map(|d| {
        std::fs::create_dir_all(d).expect("the dump directory");
        NAMES.iter().map(|n| std::io::BufWriter::new(std::fs::File::create(std::path::Path::new(d).join(format!("{n}.rows"))).expect("a dump file"))).collect()
    });
    let mut base_id = 0i64;
    let (mut line_search, mut line_table) = (0u128, 0usize);
    for (d, p) in corpus.files.iter().enumerate() {
        v15 += p.v15_stored;
        let (bs, _) = blocks::split(&p.events, Cut::Whole);
        for b in &bs {
            let laid = layout::lay(b, Layout::BytePlanes, Extras { parent: false, size: false }, &[], &[]);
            ev_zstd += zstd::bulk::compress(&laid, 3).map(|c| c.len()).unwrap_or(0);
        }
        let (tree, c) = cost(|| p.tree(&corpus));
        decode_tape += c;
        let (cols, c) = cost(|| base::columns(&tree, &p.source));
        derive += c;
        let rels = base::rows(&tree, d as i64, base_id, &cols.line_starts);
        if let Some(w) = dumped.as_mut() {
            use std::io::Write as _;
            for r in 0..5 {
                for v in &rels[r] {
                    w[r].write_all(&v.to_le_bytes()).expect("a dump write");
                }
            }
        }
        let mut blocks: Vec<Vec<u8>> = Vec::new();
        for r in 0..5 {
            let w = WIDTHS[r];
            count[r] += rels[r].len() / w;
            let mut b = Vec::new();
            base::encode(w, &rels[r], &mut b);
            per_doc[r] += b.len();
            for c in 0..w {
                for row in rels[r].chunks_exact(w) {
                    let dlt = row[c].wrapping_sub(last[r][c]);
                    last[r][c] = row[c];
                    let z = ((dlt << 1) ^ (dlt >> 63)) as u64;
                    corpus_bytes[r] += (64 - z.max(1).leading_zeros() as usize).div_ceil(7);
                }
            }
            blocks.push(b);
        }
        // The rows' way back: each block decoded to columns.
        let (decoded, c) = cost(|| {
            (0..5).map(|r| {
                let mut cols: Vec<Vec<i64>> = vec![Vec::new(); WIDTHS[r]];
                base::decode(WIDTHS[r], rels[r].len() / WIDTHS[r], &blocks[r], &mut cols);
                cols
            }).collect::<Vec<_>>()
        });
        decode_rows += c;
        let (by_outer, c) = cost(|| {
            let mut pairs: Vec<(i64, i64)> = decoded[4][1].iter().copied().zip(decoded[4][0].iter().copied()).collect();
            pairs.sort_unstable();
            pairs
        });
        sort_outer += c;
        let outer_col: Vec<i64> = by_outer.iter().map(|x| x.0).collect();
        let n = tree.len();
        // child by parent.
        let t = std::time::Instant::now();
        for i in 0..n { let r = base::range(&decoded[1][0], base_id + i as i64); got[1][0] += r.len() as i64; }
        probe[1][0] += t.elapsed().as_nanos();
        let t = std::time::Instant::now();
        for i in 0..n { got[1][1] += i64::from(cols.child_offsets[i + 1] - cols.child_offsets[i]); }
        probe[1][1] += t.elapsed().as_nanos();
        // field by parent: the rows' range; the columns' children filtered by field.
        let t = std::time::Instant::now();
        for i in 0..n { let r = base::range(&decoded[2][0], base_id + i as i64); got[2][0] += r.len() as i64; }
        probe[2][0] += t.elapsed().as_nanos();
        let t = std::time::Instant::now();
        for i in 0..n {
            let (a, b) = (cols.child_offsets[i] as usize, cols.child_offsets[i + 1] as usize);
            got[2][1] += cols.children[a..b].iter().filter(|&&c| tree.nodes[c as usize].field != 0).count() as i64;
        }
        probe[2][1] += t.elapsed().as_nanos();
        // line by node: the rows are dense by node (an index); the columns' line table searched by start.
        let t = std::time::Instant::now();
        for i in 0..n { got[3][0] += decoded[3][1][i]; }
        probe[3][0] += t.elapsed().as_nanos();
        let t = std::time::Instant::now();
        for i in 0..n { got[3][1] += i64::from(cols.line[i]); }
        probe[3][1] += t.elapsed().as_nanos();
        let t = std::time::Instant::now();
        for i in 0..n { sink += base::line_of(&cols.line_starts, tree.nodes[i].start) as i64; }
        line_search += t.elapsed().as_nanos();
        line_table += base::line_table_bytes(&cols.line_starts);
        // span_within by inner (its containers): the rows' range; a climb and the same-span descendants.
        let t = std::time::Instant::now();
        for i in 0..n { let r = base::range(&decoded[4][0], base_id + i as i64); got[4][0] += r.len() as i64; }
        probe[4][0] += t.elapsed().as_nanos();
        let t = std::time::Instant::now();
        for i in 0..n {
            let mut k = 0i64;
            let mut a = i as u32;
            while a != NONE { k += 1; a = tree.parent[a as usize]; }
            let x = &tree.nodes[i];
            let mut dd = i + 1;
            while dd < i + tree.size[i] as usize && tree.nodes[dd].start == x.start && tree.nodes[dd].end == x.end { k += 1; dd += 1; }
            got[4][1] += k;
        }
        probe[4][1] += t.elapsed().as_nanos();
        // span_within by outer (what it holds): the rows re-sorted by outer; the subtree, an interval, and the
        // ancestors of the same span.
        let t = std::time::Instant::now();
        for i in 0..n { let r = base::range(&outer_col, base_id + i as i64); got[0][0] += r.len() as i64; }
        probe[0][0] += t.elapsed().as_nanos();
        let t = std::time::Instant::now();
        for i in 0..n {
            let mut k = i64::from(tree.size[i]);
            let x = &tree.nodes[i];
            let mut a = tree.parent[i];
            while a != NONE && tree.nodes[a as usize].start == x.start && tree.nodes[a as usize].end == x.end { k += 1; a = tree.parent[a as usize]; }
            got[0][1] += k;
        }
        probe[0][1] += t.elapsed().as_nanos();
        for r in 0..5 { probes[r] += n; }
        base_id += n as i64;
    }
    std::hint::black_box(sink);
    let same = got.iter().all(|g| g[0] == g[1]);
    if let Some(d) = dump {
        drop(dumped);
        // Each relation's width and the rows a probe of each node finds, summed (what the engine's probes must find).
        let found = [got[1][0], got[2][0], got[3][0], got[4][0], got[0][0]];
        // A line: name, width, rows, the probed column, the rows the probes of every node find.
        let meta = format!("node 5 {} 1 {}\nchild 3 {} 0 {}\nfield 3 {} 0 {}\nline 2 {} 0 {}\nspan_within 2 {} 0 {}\nspan_within 2 {} 1 {}\n", count[0], count[0], count[1], found[0], count[2], found[1], count[3], count[3], count[4], found[3], count[4], found[4]);
        std::fs::write(std::path::Path::new(d).join("meta"), meta).expect("the dump's meta");
    }
    let nf = nodes as f64;
    println!("{path}: {} files, {nodes} nodes\n", corpus.files.len());
    println!("| relation | rows | rows/node | B/row, a block per document | B/node | B/row, one block | B/node |");
    println!("|---|---|---|---|---|---|---|");
    for r in 0..5 {
        println!("| {} | {} | {:.2} | {:.2} | {:.2} | {:.2} | {:.2} |", NAMES[r], count[r], count[r] as f64 / nf,
            per_doc[r] as f64 / count[r].max(1) as f64, per_doc[r] as f64 / nf, corpus_bytes[r] as f64 / count[r].max(1) as f64, corpus_bytes[r] as f64 / nf);
    }
    let (pd, cb): (usize, usize) = (per_doc.iter().sum(), corpus_bytes.iter().sum());
    println!("| **all five** | {} | | | **{:.2}** | | **{:.2}** |", count.iter().sum::<usize>(), pd as f64 / nf, cb as f64 / nf);
    println!("| the tape: format 15 as stored | | | | {:.2} | | |", v15 as f64 / nf);
    println!("| the tape: format 16 events, byte planes, zstd 3 (no dictionary) | | | | {:.2} | | |", ev_zstd as f64 / nf);
    println!("| the line table, stored (a varint a line) | | | | {:.2} | | |\n", line_table as f64 / nf);
    let per = |c: Cost| format!("{:.1} ns, {:.0} instr", c.nanos as f64 / nf, c.instructions as f64 / nf);
    println!("| rebuild, per node, one thread | rows | the tape |");
    println!("|---|---|---|");
    println!("| decode | {} (the five relations' blocks) | {} (events to a tree) |", per(decode_rows), per(decode_tape));
    println!("| index | {} (span_within by outer, sorted) | {} (children CSR, line table, line column) |", per(sort_outer), per(derive));
    let mut sum_rows = decode_rows; sum_rows += sort_outer;
    let mut sum_tape = decode_tape; sum_tape += derive;
    println!("| **total** | **{}** | **{}** |\n", per(sum_rows), per(sum_tape));
    println!("Probes checked: rows found on each side {}.\n", if same { "equal for every pattern" } else { "DIFFER" });
    if !same { println!("{got:?}"); }
    println!("| probe, each node once | rows: ns/probe | the tape's columns: ns/probe |");
    println!("|---|---|---|");
    let label = ["span_within by outer", "child by parent", "field by parent", "line by node", "span_within by inner"];
    for r in [1, 2, 3, 4, 0] {
        println!("| {} | {:.1} | {:.1} |", label[r], probe[r][0] as f64 / probes[r] as f64, probe[r][1] as f64 / probes[r] as f64);
    }
    println!("| line by node, the line table searched instead | | {:.1} |", line_search as f64 / probes[3] as f64);
}

/// E9's text part (T6, T8): `text` and `token` as copied rows with their atoms, against the tape's text.
fn cmd_textrel(path: &str) {
    use rerail_tape_lab::base;
    use rerail_tape_lab::tree::NONE;
    use rerail_tape_lab::measure::{Cost, cost};
    let corpus = load(path);
    let nodes: usize = corpus.files.iter().map(|p| p.nodes).sum();
    // The store's cost of an atom beyond its bytes: its end (4 B) and its `atom_key` row (18.6 B on impl/sqlite).
    const ATOM: f64 = 22.6;
    let empty = text::Table::train(&HashMap::new(), 0);
    // Atoms as the store numbers them (first seen), for the eager form (every node within the bound) and for
    // leaves only; token rows name the leaf children's atoms.
    let (mut eager, mut leaves): (HashMap<Vec<u8>, i64>, HashMap<Vec<u8>, i64>) = (HashMap::new(), HashMap::new());
    let (mut eager_bytes, mut leaf_bytes) = (0usize, 0usize);
    let (mut rows_eager, mut rows_leaf, mut rows_token) = (0usize, 0usize, 0usize);
    let (mut b_eager, mut b_leaf, mut b_token) = (0usize, 0usize, 0usize);
    let (mut names, mut src_stored, mut leaf_count) = (0usize, 0usize, 0usize);
    let (mut unzstd, mut names_decode, mut intern) = (Cost::default(), Cost::default(), Cost::default());
    let (mut eq_rows, mut eq_tape) = (0u128, 0u128);
    let mut same = 0i64;
    let mut base_id = 0i64;
    for p in &corpus.files {
        let tree = p.tree(&corpus);
        let src = &p.source;
        let text = |i: usize| &src[tree.nodes[i].start as usize..tree.nodes[i].end as usize];
        let (mut te, mut tl, mut tk) = (Vec::new(), Vec::new(), Vec::new());
        let mut index = vec![0i64; tree.len()];
        let mut leaf_ids: Vec<(usize, i64)> = Vec::new();
        for i in 0..tree.len() {
            let t = text(i);
            let id = base_id + i as i64;
            if t.len() <= 256 {
                let next = eager.len() as i64;
                let a = *eager.entry(t.to_vec()).or_insert_with(|| { eager_bytes += t.len(); next });
                te.extend([id, a]);
            }
            let pa = tree.parent[i];
            if pa != NONE {
                index[pa as usize] += 1;
            }
            if base::is_leaf(&tree, i) {
                leaf_count += 1;
                let next = leaves.len() as i64;
                let a = *leaves.entry(t.to_vec()).or_insert_with(|| { leaf_bytes += t.len(); next });
                tl.extend([id, a]);
                leaf_ids.push((i, a));
                if pa != NONE {
                    tk.push([base_id + pa as i64, index[pa as usize] - 1, a, tree.nodes[i].start as i64, tree.nodes[i].end as i64]);
                }
            }
        }
        tk.sort_unstable();
        let tk = tk.concat();
        for (rows, w, b, v) in [(&mut rows_eager, 2, &mut b_eager, &te), (&mut rows_leaf, 2, &mut b_leaf, &tl), (&mut rows_token, 5, &mut b_token, &tk)] {
            *rows += v.len() / w;
            let mut out = Vec::new();
            base::encode(w, v, &mut out);
            *b += out.len();
        }
        // The tape's text: the names stream (no table), and the source as the store keeps it.
        let s = text::stream(&text::pieces(&p.events, src), &empty, true);
        names += zstd_len(&text::id_planes(&s.ids)) + zstd_len(&s.pool);
        src_stored += p.source_stored;
        // Text back from the tape: the source decompressed (zstd 3 here, as stored), or the names stream decoded.
        let packed = zstd::bulk::compress(src, 3).unwrap();
        let (_, c) = cost(|| std::hint::black_box(zstd::bulk::decompress(&packed, src.len() + 16).unwrap()));
        unzstd += c;
        let (_, c) = cost(|| { let texts = base::pool_texts(&s.pool); std::hint::black_box(s.ids.iter().map(|&id| texts[id as usize].len()).sum::<usize>()) });
        names_decode += c;
        // Interning on demand: each leaf's text hashed and found among the atoms (what a rows-less read pays where a
        // rule needs an atom, not the bytes).
        let (_, c) = cost(|| { for &(i, _) in &leaf_ids { std::hint::black_box(leaves.get(text(i))); std::hint::black_box(base::fx(text(i))); } });
        intern += c;
        // `text(n, "self")`-like tests: an atom id compared, against the bytes compared.
        let k: &[u8] = b"self";
        let ka = leaves.get(k).copied().unwrap_or(-1);
        let t0 = std::time::Instant::now();
        let a = tl.chunks_exact(2).filter(|r| r[1] == ka).count();
        eq_rows += t0.elapsed().as_nanos();
        let t0 = std::time::Instant::now();
        let b = leaf_ids.iter().filter(|&&(i, _)| text(i) == k).count();
        eq_tape += t0.elapsed().as_nanos();
        same += (a == b) as i64 - 1;
        base_id += tree.len() as i64;
    }
    let nf = nodes as f64;
    let lf = leaf_count as f64;
    println!("{path}: {} files, {nodes} nodes, {leaf_count} leaves; `text(n, \"self\")` found alike on both sides: {}\n", corpus.files.len(), if same == 0 { "yes" } else { "NO" });
    println!("| copied | rows | rows/node | rows B/node | atoms | atoms B/node (bytes + {ATOM} B each) | total B/node |");
    println!("|---|---|---|---|---|---|---|");
    let at = |n: usize, b: usize| (n as f64 * ATOM + b as f64) / nf;
    println!("| text, every node within 256 B (eager) | {rows_eager} | {:.2} | {:.2} | {} | {:.2} | **{:.2}** |", rows_eager as f64 / nf, b_eager as f64 / nf, eager.len(), at(eager.len(), eager_bytes), b_eager as f64 / nf + at(eager.len(), eager_bytes));
    println!("| text, leaves | {rows_leaf} | {:.2} | {:.2} | {} | {:.2} | **{:.2}** |", rows_leaf as f64 / nf, b_leaf as f64 / nf, leaves.len(), at(leaves.len(), leaf_bytes), b_leaf as f64 / nf + at(leaves.len(), leaf_bytes));
    println!("| token, leaf children (their atoms are the leaves') | {rows_token} | {:.2} | {:.2} | | | **{:.2}** |", rows_token as f64 / nf, b_token as f64 / nf, b_token as f64 / nf);
    println!("| the tape: names stream, ids + pool, zstd, no table | | | | | | **{:.2}** |", names as f64 / nf);
    println!("| the source as stored (kept today in any case) | | | | | | {:.2} |\n", src_stored as f64 / nf);
    let per = |c: Cost, d: f64| format!("{:.1} ns, {:.0} instr", c.nanos as f64 / d, c.instructions as f64 / d);
    println!("| text from the tape, one thread | per node | per leaf |");
    println!("|---|---|---|");
    println!("| the source decompressed (zstd 3) | {} | {} |", per(unzstd, nf), per(unzstd, lf));
    println!("| the names stream decoded | {} | {} |", per(names_decode, nf), per(names_decode, lf));
    println!("| each leaf's text interned on demand (hash + lookup) | {} | {} |\n", per(intern, nf), per(intern, lf));
    println!("| `text(n, \"self\")` over every leaf | ns/leaf |");
    println!("|---|---|");
    println!("| rows: the atom compared | {:.2} |", eq_rows as f64 / lf);
    println!("| the tape: the bytes compared | {:.2} |", eq_tape as f64 / lf);
}

/// The cheapest of three runs (the machine is shared: a run can be slowed 2–4× by other work).
fn best3<T>(mut f: impl FnMut() -> T) -> (T, rerail_tape_lab::measure::Cost) {
    let (mut v, mut best) = rerail_tape_lab::measure::cost(&mut f);
    for _ in 0..2 {
        let (w, c) = rerail_tape_lab::measure::cost(&mut f);
        if c.nanos < best.nanos {
            (v, best) = (w, c);
        }
    }
    (v, best)
}

/// E10's questions on one reader, monomorphized so each accessor inlines as a real reader's would.
#[allow(clippy::too_many_arguments)]
fn bench<R: rerail_tape_lab::inplace::Reader>(r: &R, tree: &rerail_tape_lab::tree::Tree, want: &[u32], order: &[usize], set: &[bool],
    costs: &mut [rerail_tape_lab::measure::Cost; 8], counts: &mut [u64; 8], wrong: &mut usize, bytes: &mut usize) {
    use rerail_tape_lab::measure::cost;
    use rerail_tape_lab::tree::NONE;
    let n = tree.len();
        *bytes += r.bytes();
        let ids: Vec<usize> = (0..n).map(|p| r.id_of(p)).collect();
        let rand: Vec<usize> = order.iter().map(|&p| ids[p]).collect();
        // Every answer against the tree.
        let ((), k) = cost(|| {
            for p in 0..n {
                let id = ids[p];
                let ok = r.code(id) == want[p]
                    && r.start(id) == tree.nodes[p].start
                    && r.end(id) == tree.nodes[p].end
                    && (tree.parent[p] == NONE || {
                        let q = tree.parent[p] as usize;
                        let next = p + tree.size[p] as usize;
                        let want = (next < q + tree.size[q] as usize).then(|| ids[next]);
                        r.next_sibling(id, r.subtree_end(ids[q])) == want
                    })
                    && r.parent(id) == (tree.parent[p] != NONE).then(|| ids[tree.parent[p] as usize])
                    && r.first_child(id) == (tree.size[p] > 1).then(|| ids[p + 1]);
                *wrong += usize::from(!ok);
            }
        });
        costs[6] += k;
        counts[6] += n as u64;
        let (hits, k) = best3(|| ids.iter().filter(|&&id| set[r.code(id) as usize]).count());
        std::hint::black_box(hits);
        costs[0] += k;
        counts[0] += n as u64;
        let (s, k) = best3(|| rand.iter().map(|&id| u64::from(r.code(id))).sum::<u64>());
        std::hint::black_box(s);
        costs[1] += k;
        counts[1] += n as u64;
        let (s, k) = best3(|| rand.iter().map(|&id| r.start(id)).fold(0, u64::wrapping_add));
        std::hint::black_box(s);
        costs[2] += k;
        counts[2] += n as u64;
        let (s, k) = best3(|| rand.iter().map(|&id| r.end(id)).fold(0, u64::wrapping_add));
        std::hint::black_box(s);
        costs[7] += k;
        counts[7] += n as u64;
        let (s, k) = best3(|| rand.iter().map(|&id| r.subtree_end(id)).sum::<usize>());
        std::hint::black_box(s);
        costs[3] += k;
        counts[3] += n as u64;
        let (s, k) = best3(|| {
            let mut kids = 0usize;
            for &id in &ids {
                let pe = r.subtree_end(id);
                let mut ch = r.first_child(id);
                while let Some(c) = ch {
                    kids += 1;
                    ch = r.next_sibling(c, pe);
                }
            }
            kids
        });
        std::hint::black_box(s);
        costs[4] += k;
        counts[4] += n as u64;
        let (s, k) = best3(|| rand.iter().step_by(16).map(|&id| r.parent(id).unwrap_or(0)).sum::<usize>());
        std::hint::black_box(s);
        costs[5] += k;
        counts[5] += n.div_ceil(16) as u64;
}

/// E10 (0156 T9, t38): candidate in-memory tapes, every question answered in place on each, checked against
/// the tree, and costed one thread at a time.
fn cmd_inplace(path: &str, f11: bool) {
    use rerail_tape_lab::inplace::{B, CodeMap, Codes, Cols, EfEf, EfGap, F15, ForLen, PredGap, PredLen, X, XF, XR};
    use rerail_tape_lab::measure::{Cost, cost};
    let cap = std::path::Path::new(path);
    // Pass 1: each language's codes, as the grammar would number them, and its commonest kind.
    let mut codes: BTreeMap<u8, Codes> = BTreeMap::new();
    let mut kinds: BTreeMap<u8, HashMap<u16, usize>> = BTreeMap::new();
    let mut cands = events::Candidates::default();
    capture::stream(cap, |f| {
        let tree = rerail_tape_lab::v15::read(&f.image)?;
        codes.entry(f.tag).or_default().learn(&tree);
        cands.learn(f.tag, &tree);
        let k = kinds.entry(f.tag).or_default();
        for n in &tree.nodes {
            *k.entry(n.kind).or_default() += 1;
        }
        Ok(())
    }).unwrap_or_else(|e| die(&e));
    // Shape 2: the codes as the events carry them.
    let maps: BTreeMap<u8, CodeMap> = codes.iter().map(|(&tag, c)| (tag, CodeMap::new(c, tag, &cands))).collect();
    // The sweep's target: the codes of each language's commonest kind.
    let target: BTreeMap<u8, Vec<bool>> = codes.iter().map(|(&tag, c)| {
        let top = kinds[&tag].iter().max_by_key(|e| e.1).map(|e| *e.0).unwrap_or(0);
        (tag, c.table.iter().map(|t| t.0 == top).collect())
    }).collect();
    const Q: [&str; 8] = ["sweep: code in a set", "code at random", "start at random", "subtree end at random", "children of every node", "parent at random (1 node in 16)", "check (every answer)", "end at random"];
    // `--f11`: a capture with every token (`capture-tapes --all-tokens`) read as format 11: format 15's columns
    // over every node, plus the 32-byte source digest format 11 stored (format 12 dropped it).
    let names = [if f11 { "F11" } else { "F15" }, "X", "M", "XE", "B", "BM", "BE", "BH", "XF", "MF", "XR", "F16"];
    const C: usize = 12;
    // F16 (E24): each language's numbering and widths, over every file (as XF's widths are).
    let f16langs: BTreeMap<u8, rerail_tape_lab::f16::F16Lang> = {
        let mut learn: BTreeMap<u8, rerail_tape_lab::f16::Learner> = BTreeMap::new();
        capture::stream(cap, |f| {
            let tree = rerail_tape_lab::v15::read(&f.image)?;
            learn.entry(f.tag).or_default().add(&Cols::of_tree(&tree, &codes[&f.tag]));
            Ok(())
        }).unwrap_or_else(|e| die(&e));
        learn.into_iter().map(|(t, l)| (t, l.finish(&codes[&t], rerail_tape_lab::f16::grammar_codes(t)).marked(t))).collect()
    };
    let mut costs = vec![[Cost::default(); 8]; C];
    let mut counts = [[0u64; 8]; C];
    let mut bytes = [0usize; C];
    let mut parts = [[0usize; 4]; C];
    // Loading. Shape 1: zstd of the in-memory bytes on disk, loading is zstd alone. Shape 2: zstd of format 16's
    // events on disk, loading is zstd, then a build pass (here: through a tree, an upper bound).
    let (mut disk1, mut load1, mut build) = ([0usize; C], [Cost::default(); C], [Cost::default(); C]);
    let (mut disk2, mut load2, mut to_cols) = (0usize, Cost::default(), Cost::default());
    let (mut planes_cols, mut zstd2) = (Cost::default(), Cost::default());
    let (mut nodes, mut files, mut wrong) = (0usize, 0usize, [0usize; C]);
    let mut rng = 0x9e37_79b9_7f4a_7c15u64;
    capture::stream(cap, |f| {
        let tree = rerail_tape_lab::v15::read(&f.image)?;
        let n = tree.len();
        if n == 0 {
            return Ok(());
        }
        let lang = &codes[&f.tag];
        let set = &target[&f.tag];
        let want: Vec<u32> = tree.nodes.iter().map(|x| lang.ids[&(x.kind, x.field, x.flags, x.classes.clone())]).collect();
        // The same random order of nodes for every candidate.
        let mut order: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            order.swap(i, (rng % (i as u64 + 1)) as usize);
        }
        let f15 = F15::new(&f.image, lang);
        let x = X::<ForLen>::new(&tree, lang);
        let m = X::<PredGap>::new(&tree, lang);
        let xe = X::<EfGap>::new(&tree, lang);
        let b = B::<ForLen>::new(&tree, lang);
        let bm = B::<PredLen>::new(&tree, lang);
        let be = B::<EfEf>::new(&tree, lang);
        let bh = B::<PredLen>::with_big(&tree, lang);
        let xf = XF::<ForLen>::new(&tree, lang);
        let mf = XF::<PredGap>::new(&tree, lang);
        let xr = XR::new(&tree, lang);
        let f16 = rerail_tape_lab::f16::F16::new(&Cols::of_tree(&tree, lang), &f16langs[&f.tag]);
        for (c, p) in [x.parts(), m.parts(), xe.parts(), b.parts(), bm.parts(), be.parts(), bh.parts(), xf.parts(), mf.parts(), xr.parts(), f16.parts()].iter().enumerate() {
            for k in 0..4 {
                parts[c + 1][k] += p[k];
            }
        }
        // Shape 2's common part: the events zstd'd as one block (byte planes); loading inflates them to the
        // events, then one stack pass gives the builders' columns (no tree).
        let cols = {
            let ev = events::encode(&tree, f.tag, &cands);
            let (bs, _) = blocks::split(&ev, Cut::Whole);
            let laid: Vec<Vec<u8>> = bs.iter().map(|b| layout::lay(b, Layout::BytePlanes, Extras::default(), &[], &[])).collect();
            let packed: Vec<Vec<u8>> = laid.iter().map(|l| zstd::bulk::compress(l, 3).unwrap()).collect();
            disk2 += packed.iter().map(Vec::len).sum::<usize>();
            let (back, k) = best3(|| {
                let blocks: Vec<_> = packed.iter().zip(&laid).map(|(z, l)| layout::unlay(&zstd::bulk::decompress(z, l.len()).unwrap(), Layout::BytePlanes, Extras::default())).collect();
                blocks::join(&blocks)
            });
            load2 += k;
            if back != ev {
                return Err(format!("{}: the byte planes did not give the events back", f.path));
            }
            let (cols, k) = best3(|| Cols::of_events(&back, &maps[&f.tag], tree.source_len));
            to_cols += k;
            if cols != Cols::of_tree(&tree, lang) {
                return Err(format!("{}: the events did not give the columns back", f.path));
            }
            // The writer's own path: zstd, then the columns read straight off the byte planes.
            assert_eq!(packed.len(), 1, "one block (Cut::Whole)");
            let (direct, k) = best3(|| Cols::of_planes(&zstd::bulk::decompress(&packed[0], laid[0].len()).unwrap(), &maps[&f.tag], tree.source_len));
            planes_cols += k;
            let (_, k) = best3(|| zstd::bulk::decompress(&packed[0], laid[0].len()).unwrap().len());
            zstd2 += k;
            if direct != cols {
                return Err(format!("{}: the byte planes did not give the columns back", f.path));
            }
            cols
        };
        // Shape 1: each candidate's bytes zstd'd, and decompressed; and each candidate's build from the tree.
        // F16's image: its arrays' bytes as stored, side lists after each.
        let f16_image: Vec<u8> = f16.streams().iter().flat_map(|(a, b)| a.iter().chain(b.iter()).copied()).collect();
        let images: [Vec<u8>; C] = [f.image.clone(), x.image(), m.image(), xe.image(), b.image(), bm.image(), be.image(), bh.image(), xf.image(), mf.image(), xr.image(), f16_image];
        for (c, im) in images.iter().enumerate() {
            let z = zstd::bulk::compress(im, 3).unwrap();
            disk1[c] += z.len();
            let (back, k) = cost(|| zstd::bulk::decompress(&z, im.len()).unwrap());
            load1[c] += k;
            assert_eq!(&back, im);
        }
        // Shape 2's build: each candidate from the columns, its image the tree-built one byte for byte.
        let built: [(Vec<u8>, Cost); C - 1] = [
            { let (v, k) = best3(|| X::<ForLen>::of(&cols)); (v.image(), k) },
            { let (v, k) = best3(|| X::<PredGap>::of(&cols)); (v.image(), k) },
            { let (v, k) = best3(|| X::<EfGap>::of(&cols)); (v.image(), k) },
            { let (v, k) = best3(|| B::<ForLen>::of(&cols)); (v.image(), k) },
            { let (v, k) = best3(|| B::<PredLen>::of(&cols)); (v.image(), k) },
            { let (v, k) = best3(|| B::<EfEf>::of(&cols)); (v.image(), k) },
            { let (v, k) = best3(|| B::<PredLen>::with_big_of(&cols)); (v.image(), k) },
            { let (v, k) = best3(|| XF::<ForLen>::of(&cols)); (v.image(), k) },
            { let (v, k) = best3(|| XF::<PredGap>::of(&cols)); (v.image(), k) },
            { let (v, k) = best3(|| XR::of(&cols)); (v.image(), k) },
            { let (v, k) = best3(|| rerail_tape_lab::f16::F16::new(&cols, &f16langs[&f.tag])); (v.streams().iter().flat_map(|(a, b)| a.iter().chain(b.iter()).copied()).collect(), k) },
        ];
        for (c, (im, k)) in built.iter().enumerate() {
            if im != &images[c + 1] {
                return Err(format!("{}: {} built from the events differs from the tree's", f.path, names[c + 1]));
            }
            build[c + 1] += *k;
        }
        bench(&f15, &tree, &want, &order, set, &mut costs[0], &mut counts[0], &mut wrong[0], &mut bytes[0]);
        bench(&x, &tree, &want, &order, set, &mut costs[1], &mut counts[1], &mut wrong[1], &mut bytes[1]);
        bench(&m, &tree, &want, &order, set, &mut costs[2], &mut counts[2], &mut wrong[2], &mut bytes[2]);
        bench(&xe, &tree, &want, &order, set, &mut costs[3], &mut counts[3], &mut wrong[3], &mut bytes[3]);
        bench(&b, &tree, &want, &order, set, &mut costs[4], &mut counts[4], &mut wrong[4], &mut bytes[4]);
        bench(&bm, &tree, &want, &order, set, &mut costs[5], &mut counts[5], &mut wrong[5], &mut bytes[5]);
        bench(&be, &tree, &want, &order, set, &mut costs[6], &mut counts[6], &mut wrong[6], &mut bytes[6]);
        bench(&bh, &tree, &want, &order, set, &mut costs[7], &mut counts[7], &mut wrong[7], &mut bytes[7]);
        bench(&xf, &tree, &want, &order, set, &mut costs[8], &mut counts[8], &mut wrong[8], &mut bytes[8]);
        bench(&mf, &tree, &want, &order, set, &mut costs[9], &mut counts[9], &mut wrong[9], &mut bytes[9]);
        bench(&xr, &tree, &want, &order, set, &mut costs[10], &mut counts[10], &mut wrong[10], &mut bytes[10]);
        bench(&f16, &tree, &want, &order, set, &mut costs[11], &mut counts[11], &mut wrong[11], &mut bytes[11]);
        nodes += n;
        files += 1;
        if f11 {
            bytes[0] += 32;
        }
        Ok(())
    }).unwrap_or_else(|e| die(&e));
    println!("{}\n", rerail_tape_lab::measure::machine());
    println!("{path}: {files} files, {nodes} nodes; codes a language: {}\n", codes.iter().map(|(t, c)| format!("{} {} ({} bits)", capture::language(*t), c.table.len(), c.width())).collect::<Vec<_>>().join(", "));
    println!("| | {} |", names.join(" | "));
    println!("|---|{}", "---|".repeat(names.len()));
    println!("| bytes a node | {} |", bytes.iter().map(|&b| format!("**{:.2}**", b as f64 / nodes as f64)).collect::<Vec<_>>().join(" | "));
    println!("| answers wrong | {} |", wrong.iter().map(|w| w.to_string()).collect::<Vec<_>>().join(" | "));
    for (q, name) in Q.iter().enumerate() {
        println!("| {name}: ns, instr a call | {} |", (0..names.len()).map(|c| {
            let d = counts[c][q].max(1) as f64;
            format!("{:.1}, {:.0}", costs[c][q].nanos as f64 / d, costs[c][q].instructions as f64 / d)
        }).collect::<Vec<_>>().join(" | "));
    }
    println!();
    println!("| B/node by part | {} |", names[1..].join(" | "));
    println!("|---|{}", "---|".repeat(names.len() - 1));
    for (k, part) in ["code", "structure", "start", "end"].iter().enumerate() {
        println!("| {part} | {} |", parts[1..].iter().map(|p| format!("{:.2}", p[k] as f64 / nodes as f64)).collect::<Vec<_>>().join(" | "));
    }
    println!();
    let nf = nodes as f64;
    println!("| loading | {} |", names.join(" | "));
    println!("|---|{}", "---|".repeat(names.len()));
    println!("| shape 1: disk, zstd 3 of the in-memory bytes, B/node | {} |", disk1.iter().map(|&d| format!("{:.2}", d as f64 / nf)).collect::<Vec<_>>().join(" | "));
    println!("| shape 1: load = zstd, ns/node | {} |", load1.iter().map(|l| format!("{:.1}", l.nanos as f64 / nf)).collect::<Vec<_>>().join(" | "));
    println!("| shape 2: disk, zstd 3 of the events, B/node | {} |", names.iter().map(|_| format!("{:.2}", disk2 as f64 / nf)).collect::<Vec<_>>().join(" | "));
    println!("| shape 2: zstd + byte planes to events, ns/node | | {} |", names[1..].iter().map(|_| format!("{:.1}", load2.nanos as f64 / nf)).collect::<Vec<_>>().join(" | "));
    println!("| shape 2: events to columns (one stack pass), ns/node | | {} |", names[1..].iter().map(|_| format!("{:.1}", to_cols.nanos as f64 / nf)).collect::<Vec<_>>().join(" | "));
    println!("| shape 2: columns to the tape (the build), ns/node | | {} |", build[1..].iter().map(|l| format!("{:.1}", l.nanos as f64 / nf)).collect::<Vec<_>>().join(" | "));
    println!("| shape 2: zstd alone, ns/node | | {} |", names[1..].iter().map(|_| format!("{:.1}", zstd2.nanos as f64 / nf)).collect::<Vec<_>>().join(" | "));
    println!("| shape 2: zstd + columns read off the byte planes (no events), ns/node | | {} |", names[1..].iter().map(|_| format!("{:.1}", planes_cols.nanos as f64 / nf)).collect::<Vec<_>>().join(" | "));
    println!("| shape 2: load, all (zstd, planes to columns, build), ns/node | | {} |", build[1..].iter().map(|l| format!("**{:.1}**", (planes_cols.nanos + l.nanos) as f64 / nf)).collect::<Vec<_>>().join(" | "));
    // E7's mixes (reads a node): kind, flags and named come with the two sweeps; the rest at random.
    // (sweeps, random code reads, spans, children walks a node, parents)
    let mixes: [(&str, [f64; 5]); 4] = [
        ("median rule", [2.0, 0.05 + 0.04 + 0.01, 0.01, 0.05, 0.0]),
        ("121 rules, all", [2.0, 0.49 + 0.33 + 0.01, 0.96, 1.17, 0.0]),
        ("schema-introspection (tree walk)", [2.0, 1.65 + 0.73 + 0.04, 3.16, 3.52, 5.0]),
        ("heaviest rule", [2.0, 9.47 + 2.68 + 0.12, 11.12, 13.18, 0.01]),
    ];
    println!();
    println!("| E7 mix, ns a node | {} |", names.join(" | "));
    println!("|---|{}", "---|".repeat(names.len()));
    for (name, m) in mixes {
        println!("| {name} | {} |", (0..names.len()).map(|c| {
            let per = |q: usize| costs[c][q].nanos as f64 / counts[c][q].max(1) as f64;
            // A children walk a node: the children question's cost over the mean children a node (about 1).
            let v = m[0] * per(0) + m[1] * per(1) + m[2] * (per(2) + per(7)) + m[3] * per(4) + m[4] * per(5);
            format!("{v:.1}")
        }).collect::<Vec<_>>().join(" | "));
    }
}

/// E10 on Metal: the corpus as one forest, each candidate's image in one buffer, every question a kernel.
#[cfg(target_os = "macos")]
fn cmd_prim_gpu(path: &str) {
    use rerail_tape_lab::inplace::{Codes, forest};
    use rerail_tape_lab::prim::{Names, SETS};
    let files = capture::read(std::path::Path::new(path)).unwrap_or_else(|e| die(&e));
    // One language (the one with the most nodes), its files as one forest.
    let trees: Vec<_> = files.iter().map(|f| (f.tag, rerail_tape_lab::v15::read(&f.image).unwrap_or_else(|e| die(&e)))).collect();
    let mut by_tag: BTreeMap<u8, usize> = BTreeMap::new();
    for (tag, t) in &trees {
        *by_tag.entry(*tag).or_default() += t.len();
    }
    // The largest language that has functions (a data format has none of the sets but strings).
    let names = Names::load();
    let tag = *by_tag.iter().filter(|e| !names.kind_ids(*e.0, SETS[0].1).is_empty()).max_by_key(|e| e.1).unwrap().0;
    let mine: Vec<_> = trees.into_iter().filter(|e| e.0 == tag).map(|e| e.1).collect();
    let nfiles = mine.len();
    let tree = forest(&mine);
    drop(mine);
    let mut codes = Codes::default();
    codes.learn(&tree);
    let n = tree.len();
    println!("{}\n", rerail_tape_lab::measure::machine());
    println!("{path}: language {tag}, {nfiles} files as one forest, {n} nodes; GPU: one thread a group of 64 nodes, best of 8 warm dispatches; CPU: one thread, from the S-nodes, best of 3\n");
    println!("| ns a node | set | share | wrong | GPU match | GPU nearest (with match) | GPU outermost | GPU has, climbing | GPU has, from counts | GPU span | BE nearest, carry by climbing | GPU nearest, bounded climb | GPU nearest, back over the S-nodes | GPU outermost, prefix max | CPU nearest | CPU outermost | CPU has |");
    println!("|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|");
    for (s, kinds) in SETS {
        let set = names.set(tag, &codes, kinds);
        let share = tree.nodes.iter().filter(|x| set[codes.ids[&(x.kind, x.field, x.flags, x.classes.clone())] as usize]).count() as f64 / n as f64;
        let rows = rerail_tape_lab::prim_gpu::all(&tree, &codes, &set, rerail_tape_lab::f16::grammar_codes(tag)).unwrap_or_else(|e| die(&e));
        for (name, r, c) in rows {
            let (n2, o2) = r.scan.map_or((String::new(), String::new()), |(a, b)| (format!("{a:.3}"), format!("{b:.3}")));
            println!("| {name} | {s} | {:.2}% | {} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {:.3} | {} | {n2} | {} | {o2} | {:.2} | {:.2} | {:.2} |", 100.0 * share, r.wrong, r.ns[0], r.ns[1], r.ns[2], r.ns[3], r.has_counts, r.span,
                r.no_carry.map_or(String::new(), |v| format!("{v:.3}")), r.back.map_or(String::new(), |v| format!("{v:.3}")), c[0], c[1], c[2]);
        }
    }
}

/// t39 step 23: parent reads and climbs under each leaf-slot rule, and the slots' memory.
fn cmd_climb(path: &str) {
    use rerail_tape_lab::climb::{RULES, climbs, memory};
    use rerail_tape_lab::inplace::{Codes, Cols, forest};
    use rerail_tape_lab::prim::{Names, SETS};
    let files = capture::read(std::path::Path::new(path)).unwrap_or_else(|e| die(&e));
    let trees: Vec<_> = files.iter().map(|f| (f.tag, rerail_tape_lab::v15::read(&f.image).unwrap_or_else(|e| die(&e)))).collect();
    let mut by_tag: BTreeMap<u8, Vec<&rerail_tape_lab::tree::Tree>> = BTreeMap::new();
    for (tag, t) in &trees {
        by_tag.entry(*tag).or_default().push(t);
    }
    println!("{path}: slots by rule, per language (in memory B a node, escaped %, slots a point parent reads: mean / most, zstd 3 B a node a file at a time)\n");
    println!("| language | nodes | {} |", RULES.iter().map(|r| r.0).collect::<Vec<_>>().join(" | "));
    println!("|---|---|{}", "---|".repeat(RULES.len()));
    for (tag, ts) in &by_tag {
        let mut codes = Codes::default();
        for t in ts {
            codes.learn(t);
        }
        let cols: Vec<Cols> = ts.iter().map(|t| Cols::of_tree(t, &codes)).collect();
        let m = memory(&cols);
        println!("| {tag} | {} | {} |", m.nodes, m.rows.iter().map(|r| format!("{:.3} B, {:.2}%, {:.2} / {}, {:.3}", r.0, r.1, r.2, r.3, r.4)).collect::<Vec<_>>().join(" | "));
    }
    // The climbs, on prim-gpu's forest: the largest language with functions.
    let names = Names::load();
    let tag = *by_tag.iter().filter(|e| !names.kind_ids(*e.0, SETS[0].1).is_empty()).max_by_key(|e| e.1.iter().map(|t| t.len()).sum::<usize>()).unwrap().0;
    let mine: Vec<_> = trees.into_iter().filter(|e| e.0 == tag).map(|e| e.1).collect();
    let tree = forest(&mine);
    drop(mine);
    let mut codes = Codes::default();
    codes.learn(&tree);
    println!("\nlanguage {tag} as one forest, {} nodes: slot reads a node for each 64-node group's carry climb, full (to the root) / bounded; XF reads one slot a parent\n", tree.len());
    println!("| set | parents a node, full / bounded | {} |", RULES.iter().map(|r| r.0).collect::<Vec<_>>().join(" | "));
    println!("|---|---|{}", "---|".repeat(RULES.len()));
    for (s, kinds) in SETS {
        let set = names.set(tag, &codes, kinds);
        let c = climbs(&tree, &codes, &set);
        println!("| {s} | {:.3} / {:.3} | {} |", c.steps.0, c.steps.1, c.full.iter().zip(&c.bounded).map(|(f, b)| format!("{f:.3} / {b:.3}")).collect::<Vec<_>>().join(" | "));
    }
}

#[cfg(target_os = "macos")]
fn cmd_inplace_gpu(path: &str) {
    use rerail_tape_lab::inplace::{Codes, forest};
    let files = capture::read(std::path::Path::new(path)).unwrap_or_else(|e| die(&e));
    let trees: Vec<_> = files.iter().map(|f| rerail_tape_lab::v15::read(&f.image).unwrap_or_else(|e| die(&e))).collect();
    let tree = forest(&trees);
    drop(trees);
    let mut codes = Codes::default();
    codes.learn(&tree);
    let n = tree.len();
    let mut rng = 0x9e37_79b9_7f4a_7c15u64;
    let mut order: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        order.swap(i, (rng % (i as u64 + 1)) as usize);
    }
    let rows = rerail_tape_lab::inplace_gpu::all(&tree, &codes, &order).unwrap_or_else(|e| die(&e));
    println!("{path}: {} files as one forest, {n} nodes, one GPU thread a question, best of 4 warm dispatches\n", files.len());
    println!("| GPU ns a question | {} |", rows.iter().map(|r| r.0).collect::<Vec<_>>().join(" | "));
    println!("|---|{}", "---|".repeat(rows.len()));
    println!("| image bytes a node | {} |", rows.iter().map(|r| format!("{:.2}", r.1.bytes as f64 / n as f64)).collect::<Vec<_>>().join(" | "));
    for (q, name) in rerail_tape_lab::inplace_gpu::QUESTIONS.iter().enumerate() {
        println!("| {name} | {} |", rows.iter().map(|r| format!("{:.3}{}", r.1.ns[q], if r.1.correct[q] { "" } else { " WRONG" })).collect::<Vec<_>>().join(" | "));
    }
}

/// E12 (t39): the tree primitives rules ask for, bulk (every node, one pass) and point (one node at random),
/// on each credible candidate, every answer checked against the tree.
fn cmd_prim(path: &str) {
    use rerail_tape_lab::inplace::{B, Codes, EfEf, F15, ForLen, PredGap, Reader, XF, XR};
    use rerail_tape_lab::measure::Cost;
    use rerail_tape_lab::prim::{self, Names, SETS, WRAP};
    use rerail_tape_lab::tree::NONE;
    let cap = std::path::Path::new(path);
    let names = Names::load();
    let mut codes: BTreeMap<u8, Codes> = BTreeMap::new();
    capture::stream(cap, |f| {
        codes.entry(f.tag).or_default().learn(&rerail_tape_lab::v15::read(&f.image)?);
        Ok(())
    }).unwrap_or_else(|e| die(&e));
    // Per language: each set's code mask, the wrapper mask, each code's field, the name and body fields.
    struct Lang { sets: Vec<Vec<bool>>, kinds: Vec<Vec<u16>>, kind_of: Vec<u16>, wrap: Vec<bool>, field_of: Vec<u8>, fields: [u8; 2] }
    let langs: BTreeMap<u8, Lang> = codes.iter().map(|(&tag, c)| (tag, Lang {
        sets: SETS.iter().map(|(_, k)| names.set(tag, c, k)).collect(),
        kinds: SETS.iter().map(|(_, k)| names.kind_ids(tag, k)).collect(),
        kind_of: c.table.iter().map(|t| t.0).collect(),
        wrap: names.set(tag, c, WRAP),
        field_of: c.table.iter().map(|t| t.1).collect(),
        fields: [names.field(tag, "name").unwrap_or(255), names.field(tag, "body").unwrap_or(255)],
    })).collect();
    const C: [&str; 6] = ["F15", "XF", "MF", "BE", "XR", "F16"];
    let f16langs: BTreeMap<u8, rerail_tape_lab::f16::F16Lang> = {
        let mut learn: BTreeMap<u8, rerail_tape_lab::f16::Learner> = BTreeMap::new();
        capture::stream(cap, |f| {
            let tree = rerail_tape_lab::v15::read(&f.image)?;
            learn.entry(f.tag).or_default().add(&rerail_tape_lab::inplace::Cols::of_tree(&tree, &codes[&f.tag]));
            Ok(())
        }).unwrap_or_else(|e| die(&e));
        learn.into_iter().map(|(t, l)| (t, l.finish(&codes[&t], rerail_tape_lab::f16::grammar_codes(t)).marked(t))).collect()
    };
    // Rows: (primitive, set) → per candidate cost and count.
    let mut rows: Vec<String> = Vec::new();
    for (s, _) in SETS {
        for q in ["nearest, every node", "outermost, every node", "has, every node", "nearest, one node", "has, one node",
            "has, every node, backwards", "has, one node, skipping (exact summary)", "has, one node, by preorder steps",
            "has, one node, position lists", "every node of the set, a code sweep (a node)", "every node of the set, position lists (a node)",
            "every node of the set, a sweep skipping by the exact summary (a node)", "every node of the set, 64 codes at a time (a node)",
            "nearest, every node, from the S-nodes (64 codes at a time)", "outermost, every node, from the S-nodes", "has, every node, from the S-nodes (climbing)", "span of every node of the set (a node of the set)"] {
            rows.push(format!("{q} ({s})"));
        }
    }
    rows.extend(["field `name`, one node".into(), "field `body`, one node".into(), "unparen, one node".into(), "line, every node".into(), "line, one node".into(), "span, every node in order".into(), "span, every node, ends closed by the sweep".into(), "span, every node, a block at a time".into()]);
    let nr = rows.len();
    let mut cost = vec![vec![Cost::default(); C.len()]; nr];
    let mut count = vec![vec![0u64; C.len()]; nr];
    let mut wrong = vec![0usize; C.len()];
    let mut sum_bytes = vec![[0usize; 3]; C.len()];
    let mut line_bytes = 0usize;
    // Codes' bits: the file's one width, and a table of each block's distinct codes (blocks of 64, 256, 1,024).
    let mut code_bits = [0usize; 4];
    // BE's close searches: nodes whose close lies 2 or more second-level groups (2,048 positions) past the
    // open, the groups so crossed, the most in one search, and the largest file.
    let (mut far, mut far_groups, mut far_max, mut biggest) = (0usize, 0usize, 0usize, 0usize);
    let (mut nodes, mut files) = (0usize, 0usize);
    // Sets' share of nodes, for the table.
    let mut share = vec![0usize; SETS.len()];
    let mut rng = 0x2545_f491_4f6c_dd1du64;
    #[allow(clippy::too_many_arguments)]
    fn one<R: Reader>(r: &R, c: usize, tree: &rerail_tape_lab::tree::Tree, code: &[u32], l: &Lang, lines: &prim::Lines, line_truth: &[u32], sample: &[usize], truths: &[(Vec<u32>, Vec<u32>, Vec<bool>)],
        cost: &mut [Vec<Cost>], count: &mut [Vec<u64>], wrong: &mut [usize], sum_bytes: &mut [[usize; 3]]) {
        let n = tree.len();
        let ids: Vec<usize> = (0..n).map(|p| r.id_of(p)).collect();
        let pre: HashMap<usize, usize> = ids.iter().enumerate().map(|(p, &id)| (id, p)).collect();
        let to_pre = |x: Option<usize>| x.map_or(NONE, |id| pre[&id] as u32);
        let (mut near, mut flag) = (Vec::with_capacity(n), Vec::with_capacity(n));
        let sums = [prim::Summary::new(r, l.field_of.len(), true)];
        sum_bytes[c][0] += sums[0].bytes();
        let post = prim::Postings::new(r, &l.kind_of);
        sum_bytes[c][2] += post.bytes();
        for (k, set) in l.sets.iter().enumerate() {
            let row = k * 17;
            let (tn, to, th) = &truths[k];
            let (_, x) = best3(|| prim::nearest_all(r, set, &mut near));
            wrong[c] += near.iter().zip(tn).filter(|(a, b)| a != b).count();
            cost[row][c] += x;
            count[row][c] += n as u64;
            let (_, x) = best3(|| prim::outermost_all(r, set, &mut near));
            wrong[c] += near.iter().zip(to).filter(|(a, b)| a != b).count();
            cost[row + 1][c] += x;
            count[row + 1][c] += n as u64;
            let (_, x) = best3(|| prim::has_all(r, set, &mut flag));
            wrong[c] += flag.iter().zip(th).filter(|(a, b)| a != b).count();
            cost[row + 2][c] += x;
            count[row + 2][c] += n as u64;
            let (got, x) = best3(|| sample.iter().map(|&p| prim::nearest_one(r, set, ids[p])).collect::<Vec<_>>());
            wrong[c] += got.iter().zip(sample).filter(|&(g, &p)| to_pre(*g) != tn[p]).count();
            cost[row + 3][c] += x;
            count[row + 3][c] += sample.len() as u64;
            let (got, x) = best3(|| sample.iter().map(|&p| prim::has_one(r, set, ids[p])).collect::<Vec<_>>());
            wrong[c] += got.iter().zip(sample).filter(|&(g, &p)| *g != th[p]).count();
            cost[row + 4][c] += x;
            count[row + 4][c] += sample.len() as u64;
            let (_, x) = best3(|| prim::has_all_back(r, set, &mut flag));
            wrong[c] += flag.iter().zip(th).filter(|(a, b)| a != b).count();
            cost[row + 5][c] += x;
            count[row + 5][c] += n as u64;
            let mask = sums[0].mask(set);
            let (got, x) = best3(|| sample.iter().map(|&p| prim::has_one_skip(r, &sums[0], &mask, set, ids[p])).collect::<Vec<_>>());
            wrong[c] += got.iter().zip(sample).filter(|&(g, &p)| *g != th[p]).count();
            cost[row + 6][c] += x;
            count[row + 6][c] += sample.len() as u64;
            let (got, x) = best3(|| sample.iter().map(|&p| prim::has_one_steps(r, set, ids[p])).collect::<Vec<_>>());
            wrong[c] += got.iter().zip(sample).filter(|&(g, &p)| *g != th[p]).count();
            cost[row + 7][c] += x;
            count[row + 7][c] += sample.len() as u64;
            let lists = post.of_set(&l.kinds[k]);
            let (got, x) = best3(|| sample.iter().map(|&p| prim::has_one_post(r, &lists, ids[p])).collect::<Vec<_>>());
            wrong[c] += got.iter().zip(sample).filter(|&(g, &p)| *g != th[p]).count();
            cost[row + 8][c] += x;
            count[row + 8][c] += sample.len() as u64;
            let want: Vec<u32> = (0..n as u32).filter(|&p| set[code[p as usize] as usize]).collect();
            let mut every = Vec::new();
            let (_, x) = best3(|| prim::every_sweep(r, set, &mut every));
            wrong[c] += usize::from(every != want);
            cost[row + 9][c] += x;
            count[row + 9][c] += n as u64;
            let (_, x) = best3(|| prim::every_post(&lists, &mut every));
            wrong[c] += usize::from(every != want);
            cost[row + 10][c] += x;
            count[row + 10][c] += n as u64;
            let mask = sums[0].mask(set);
            let (_, x) = best3(|| prim::every_sweep_skip(r, &sums[0], &mask, set, &mut every));
            wrong[c] += usize::from(every != want);
            cost[row + 11][c] += x;
            count[row + 11][c] += n as u64;
            let (_, x) = best3(|| prim::every_groups(r, set, &mut every));
            wrong[c] += usize::from(every != want);
            cost[row + 12][c] += x;
            count[row + 12][c] += n as u64;
            let (_, x) = best3(|| prim::axis_groups(r, set, false, &mut near));
            wrong[c] += near.iter().zip(tn).filter(|(a, b)| a != b).count();
            cost[row + 13][c] += x;
            count[row + 13][c] += n as u64;
            let (_, x) = best3(|| prim::axis_groups(r, set, true, &mut near));
            wrong[c] += near.iter().zip(to).filter(|(a, b)| a != b).count();
            cost[row + 14][c] += x;
            count[row + 14][c] += n as u64;
            let (_, x) = best3(|| prim::has_groups(r, set, &mut flag));
            wrong[c] += flag.iter().zip(th).filter(|(a, b)| a != b).count();
            cost[row + 15][c] += x;
            count[row + 15][c] += n as u64;
            let mut sp = Vec::new();
            let (_, x) = best3(|| prim::spans_groups(r, set, &mut sp));
            wrong[c] += usize::from(sp.len() != want.len()) + sp.iter().zip(&want).filter(|&(g, &p)| *g != (tree.nodes[p as usize].start, tree.nodes[p as usize].end)).count();
            cost[row + 16][c] += x;
            count[row + 16][c] += want.len().max(1) as u64;
        }
        let base = l.sets.len() * 17;
        for (j, &f) in l.fields.iter().enumerate() {
            let (got, x) = best3(|| sample.iter().map(|&p| prim::field_one(r, &l.field_of, f, ids[p])).collect::<Vec<_>>());
            for (g, &p) in got.iter().zip(sample) {
                let want = (p + 1..p + tree.size[p] as usize).find(|&q| tree.parent[q] as usize == p && tree.nodes[q].field == f);
                wrong[c] += usize::from(to_pre(*g) != want.map_or(NONE, |q| q as u32));
            }
            cost[base + j][c] += x;
            count[base + j][c] += sample.len() as u64;
        }
        let (got, x) = best3(|| sample.iter().map(|&p| prim::unparen_one(r, &l.wrap, ids[p])).collect::<Vec<_>>());
        cost[base + 2][c] += x;
        count[base + 2][c] += sample.len() as u64;
        let mut lv = Vec::with_capacity(n);
        let (_, x) = best3(|| prim::lines_all(r, lines, &mut lv));
        wrong[c] += lv.iter().zip(line_truth).filter(|(a, b)| a != b).count();
        cost[base + 3][c] += x;
        count[base + 3][c] += n as u64;
        let (got_l, x) = best3(|| sample.iter().map(|&p| lines.line(r.start(ids[p])) as u32).collect::<Vec<_>>());
        wrong[c] += got_l.iter().zip(sample).filter(|&(g, &p)| *g != line_truth[p]).count();
        cost[base + 4][c] += x;
        count[base + 4][c] += sample.len() as u64;
        let mut sp = Vec::new();
        let (_, x) = best3(|| prim::spans_all(r, &mut sp));
        wrong[c] += sp.iter().zip(&tree.nodes).filter(|(g, t)| **g != (t.start, t.end)).count();
        cost[base + 5][c] += x;
        count[base + 5][c] += n as u64;
        let (_, x) = best3(|| prim::spans_stream(r, &mut sp));
        wrong[c] += sp.iter().zip(&tree.nodes).filter(|(g, t)| **g != (t.start, t.end)).count();
        cost[base + 6][c] += x;
        count[base + 6][c] += n as u64;
        let (_, x) = best3(|| prim::spans_bulk(r, &mut sp));
        wrong[c] += sp.iter().zip(&tree.nodes).filter(|(g, t)| **g != (t.start, t.end)).count();
        cost[base + 7][c] += x;
        count[base + 7][c] += n as u64;
        for (g, &p) in got.iter().zip(sample) {
            let mut q = p;
            while l.wrap[code[q] as usize] && tree.size[q] > 1 {
                q += 1;
            }
            wrong[c] += usize::from(pre[g] != q);
        }
    }
    capture::stream(cap, |f| {
        let tree = rerail_tape_lab::v15::read(&f.image)?;
        let n = tree.len();
        if n == 0 {
            return Ok(());
        }
        let lang = &codes[&f.tag];
        let l = &langs[&f.tag];
        let code: Vec<u32> = tree.nodes.iter().map(|x| lang.ids[&(x.kind, x.field, x.flags, x.classes.clone())]).collect();
        let truths: Vec<_> = l.sets.iter().map(|set| {
            let inset: Vec<bool> = code.iter().map(|&c| set[c as usize]).collect();
            (prim::truth::nearest(&tree, &inset), prim::truth::outermost(&tree, &inset), prim::truth::has(&tree, &inset))
        }).collect();
        for (k, set) in l.sets.iter().enumerate() {
            share[k] += code.iter().filter(|&&c| set[c as usize]).count();
        }
        let lines = prim::Lines::new(&f.source);
        line_bytes += lines.bytes();
        {
            let mut depth = vec![0usize; n];
            for i in 1..n {
                depth[i] = depth[tree.parent[i] as usize] + 1;
            }
            for i in 0..n {
                let open = 2 * i - depth[i];
                let close = open + 2 * tree.size[i] as usize - 1;
                let g = (close >> 11) - (open >> 11);
                if g >= 2 {
                    far += 1;
                    far_groups += g;
                    far_max = far_max.max(g);
                }
            }
            biggest = biggest.max(n);
        }
        let wf = (32 - code.iter().max().unwrap().leading_zeros()).max(1) as usize;
        code_bits[0] += n * wf + 8;
        for (q, b) in [64usize, 256, 1024].into_iter().enumerate() {
            for blk in code.chunks(b) {
                let mut d = blk.to_vec();
                d.sort_unstable();
                d.dedup();
                let wl = (usize::BITS - (d.len() - 1).leading_zeros()) as usize;
                // A header byte (the local width), the table at the file's width, the codes at the local width.
                code_bits[q + 1] += 8 + d.len() * wf + blk.len() * wl;
            }
        }
        let nl: Vec<u64> = f.source.iter().enumerate().filter(|e| *e.1 == b'\n').map(|e| e.0 as u64).collect();
        let line_truth: Vec<u32> = tree.nodes.iter().map(|x| nl.partition_point(|&v| v < x.start) as u32).collect();
        // One node in 16, at random.
        let sample: Vec<usize> = (0..n.div_ceil(16)).map(|_| {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            (rng % n as u64) as usize
        }).collect();
        // Format 15 has no parent (each step is a search): its point questions on 1 node in 1,024.
        one(&F15::new(&f.image, lang), 0, &tree, &code, l, &lines, &line_truth, &sample[..sample.len().div_ceil(64)], &truths, &mut cost, &mut count, &mut wrong, &mut sum_bytes);
        one(&XF::<ForLen>::new(&tree, lang), 1, &tree, &code, l, &lines, &line_truth, &sample, &truths, &mut cost, &mut count, &mut wrong, &mut sum_bytes);
        one(&XF::<PredGap>::new(&tree, lang), 2, &tree, &code, l, &lines, &line_truth, &sample, &truths, &mut cost, &mut count, &mut wrong, &mut sum_bytes);
        one(&B::<EfEf>::new(&tree, lang), 3, &tree, &code, l, &lines, &line_truth, &sample, &truths, &mut cost, &mut count, &mut wrong, &mut sum_bytes);
        one(&XR::new(&tree, lang), 4, &tree, &code, l, &lines, &line_truth, &sample, &truths, &mut cost, &mut count, &mut wrong, &mut sum_bytes);
        one(&rerail_tape_lab::f16::F16::new(&rerail_tape_lab::inplace::Cols::of_tree(&tree, lang), &f16langs[&f.tag]), 5, &tree, &code, l, &lines, &line_truth, &sample, &truths, &mut cost, &mut count, &mut wrong, &mut sum_bytes);
        nodes += n;
        files += 1;
        Ok(())
    }).unwrap_or_else(|e| die(&e));
    println!("{}\n", rerail_tape_lab::measure::machine());
    println!("{path}: {files} files, {nodes} nodes; sets' share of nodes: {}\n", SETS.iter().zip(&share).map(|((s, _), &k)| format!("{s} {:.2}%", 100.0 * k as f64 / nodes as f64)).collect::<Vec<_>>().join(", "));
    println!("\ncodes, bytes a node: one width a file {:.3}; a table a block of 64 {:.3}, of 256 {:.3}, of 1,024 {:.3}.\n",
        code_bits[0] as f64 / 8.0 / nodes as f64, code_bits[1] as f64 / 8.0 / nodes as f64, code_bits[2] as f64 / 8.0 / nodes as f64, code_bits[3] as f64 / 8.0 / nodes as f64);
    println!("BE close searches crossing 2 or more second-level groups: {:.3}% of nodes, {:.1} groups each on average, at most {far_max}; the largest file {biggest} nodes.\n",
        100.0 * far as f64 / nodes as f64, far_groups as f64 / far.max(1) as f64);
    println!("| ns a node (every node) or a call (one node, 1 in 16 at random; F15 1 in 1,024); instructions | {} |", C.join(" | "));
    println!("|---|{}", "---|".repeat(C.len()));
    println!("| answers wrong | {} |", wrong.iter().map(|w| w.to_string()).collect::<Vec<_>>().join(" | "));
    println!("| line index (Elias–Fano of the newlines) bytes a node | {} |", C.iter().map(|_| format!("{:.3}", line_bytes as f64 / nodes as f64)).collect::<Vec<_>>().join(" | "));
    println!("| exact block summary (codes in each 256 nodes), bytes a node | {} |", sum_bytes.iter().map(|b| format!("{:.3}", b[0] as f64 / nodes as f64)).collect::<Vec<_>>().join(" | "));
    println!("| per-kind position lists, bytes a node | {} |", sum_bytes.iter().map(|b| format!("{:.3}", b[2] as f64 / nodes as f64)).collect::<Vec<_>>().join(" | "));
    for (i, row) in rows.iter().enumerate() {
        println!("| {row} | {} |", (0..C.len()).map(|c| {
            let d = count[i][c].max(1) as f64;
            format!("{:.1}, {:.0}", cost[i][c].nanos as f64 / d, cost[i][c].instructions as f64 / d)
        }).collect::<Vec<_>>().join(" | "));
    }
}

/// E16 (t39 step 8): the leaf slot by sibling, and positions by gap with a start every K (`relative.rs`).
fn cmd_relative(path: &str) {
    use rerail_tape_lab::inplace::Codes;
    let files = capture::read(std::path::Path::new(path)).unwrap_or_else(|e| die(&e));
    let mut by_tag: BTreeMap<u8, Vec<rerail_tape_lab::tree::Tree>> = BTreeMap::new();
    for f in &files {
        by_tag.entry(f.tag).or_default().push(rerail_tape_lab::v15::read(&f.image).unwrap_or_else(|e| die(&e)));
    }
    println!("{}\n", rerail_tape_lab::measure::machine());
    for (tag, trees) in &by_tag {
        let mut codes = Codes::default();
        for t in trees {
            codes.learn(t);
        }
        let r = rerail_tape_lab::relative::run(trees, &codes);
        rerail_tape_lab::relative::print(&format!("{path}, {} ({tag})", capture::language(*tag)), &r);
        println!();
    }
}

/// E18 (t39 step 10): the forms on disk under zstd and LZ4: bytes, decode time, match coverage (`disk.rs`).
fn cmd_disk(path: &str) {
    let files = capture::read(std::path::Path::new(path)).unwrap_or_else(|e| die(&e));
    let mut by_tag: BTreeMap<u8, Vec<&capture::File>> = BTreeMap::new();
    for f in &files {
        by_tag.entry(f.tag).or_default().push(f);
    }
    println!("{}\n", rerail_tape_lab::measure::machine());
    for (tag, fs) in &by_tag {
        let r = rerail_tape_lab::disk::run(fs);
        rerail_tape_lab::disk::print(&format!("{path}, {} ({tag})", capture::language(*tag)), &r);
    }
}

/// E19 (t39 step 11): LZ4 with random access, fixed blocks stored as repeats, raw or LZ4 (`ralz.rs`).
fn cmd_ralz(path: &str) {
    let files = capture::read(std::path::Path::new(path)).unwrap_or_else(|e| die(&e));
    let mut by_tag: BTreeMap<u8, Vec<&capture::File>> = BTreeMap::new();
    for f in &files {
        by_tag.entry(f.tag).or_default().push(f);
    }
    println!("{}\n", rerail_tape_lab::measure::machine());
    for (tag, fs) in &by_tag {
        let r = rerail_tape_lab::ralz::run(fs);
        rerail_tape_lab::ralz::print(&format!("{path}, {} ({tag})", capture::language(*tag)), &r);
    }
}

/// E20 (t39 step 12): codes in 8 bits, overloaded by the adjacent code (`code8.rs`).
fn cmd_code8(path: &str) {
    let files = capture::read(std::path::Path::new(path)).unwrap_or_else(|e| die(&e));
    let mut by_tag: BTreeMap<u8, Vec<&capture::File>> = BTreeMap::new();
    for f in &files {
        by_tag.entry(f.tag).or_default().push(f);
    }
    println!("{}\n", rerail_tape_lab::measure::machine());
    for (tag, fs) in &by_tag {
        let r = rerail_tape_lab::code8::run(fs);
        rerail_tape_lab::code8::print(&format!("{path}, {} ({tag})", capture::language(*tag)), &r);
    }
}

/// E24 (t39 step 16): format 16's tape as one form, at rest (`f16.rs`); its questions are in `inplace` and `prim`.
fn cmd_f16(path: &str) {
    let files = capture::read(std::path::Path::new(path)).unwrap_or_else(|e| die(&e));
    let mut by_tag: BTreeMap<u8, Vec<&capture::File>> = BTreeMap::new();
    for f in &files {
        by_tag.entry(f.tag).or_default().push(f);
    }
    println!("{}\n", rerail_tape_lab::measure::machine());
    for (tag, fs) in &by_tag {
        let r = rerail_tape_lab::f16::run(fs);
        rerail_tape_lab::f16::print(&format!("{path}, {} ({tag})", capture::language(*tag)), &r);
    }
}

/// E25 (t39 step 17): every code the grammar allows, checked against every node of the capture (`gcodes.rs`).
#[cfg(feature = "grammar")]
fn cmd_gcodes(path: &str) {
    let files = capture::read(std::path::Path::new(path)).unwrap_or_else(|e| die(&e));
    let mut by_tag: BTreeMap<u8, Vec<&capture::File>> = BTreeMap::new();
    for f in &files {
        by_tag.entry(f.tag).or_default().push(f);
    }
    let scratch = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tmp/e25");
    for (tag, fs) in &by_tag {
        let title = format!("{path}, {} ({tag})", capture::language(*tag));
        match rerail_tape_lab::gcodes::run(*tag, fs, &scratch) {
            Ok(r) => rerail_tape_lab::gcodes::print(&title, &r),
            Err(e) => println!("### {title}\n\n{e}\n"),
        }
    }
}

/// E26 (t39 step 18): format 16's fixed blocks decoded on Metal (`f16_gpu.rs`).
#[cfg(target_os = "macos")]
fn cmd_f16_gpu(path: &str) {
    let files = capture::read(std::path::Path::new(path)).unwrap_or_else(|e| die(&e));
    let mut by_tag: BTreeMap<u8, Vec<&capture::File>> = BTreeMap::new();
    for f in &files {
        by_tag.entry(f.tag).or_default().push(f);
    }
    println!("{}\n", rerail_tape_lab::measure::machine());
    println!("{path}\n");
    rerail_tape_lab::f16_gpu::print_header();
    let mut parts = Vec::new();
    for (tag, fs) in &by_tag {
        let a = rerail_tape_lab::f16::at_rest(fs);
        match rerail_tape_lab::f16_gpu::run(&a) {
            Ok(r) => rerail_tape_lab::f16_gpu::print(capture::language(*tag), &a, &r),
            Err(e) => println!("| {} | {e} |", capture::language(*tag)),
        }
        parts.push(a);
    }
    // Every language's blocks in one dispatch, as a load of the whole capture would decode them.
    let all = rerail_tape_lab::f16::AtRest::merge(&parts);
    drop(parts);
    match rerail_tape_lab::f16_gpu::run(&all) {
        Ok(r) => rerail_tape_lab::f16_gpu::print("**all, one dispatch**", &all, &r),
        Err(e) => println!("| all | {e} |"),
    }
}

/// E28 (t39 step 20): format 16's writer against format 15's (`f16w.rs`).
fn cmd_f16w(path: &str) {
    let files = capture::read(std::path::Path::new(path)).unwrap_or_else(|e| die(&e));
    let mut by_tag: BTreeMap<u8, Vec<&capture::File>> = BTreeMap::new();
    for f in &files {
        by_tag.entry(f.tag).or_default().push(f);
    }
    println!("{}\n\n{path}\n", rerail_tape_lab::measure::machine());
    rerail_tape_lab::f16w::print_header();
    for (tag, fs) in &by_tag {
        let r = rerail_tape_lab::f16w::run(fs);
        rerail_tape_lab::f16w::print(capture::language(*tag), &r);
    }
}

/// E22 (t39 step 14): the field taken out of the code (`nofield.rs`).
fn cmd_nofield(path: &str) {
    let files = capture::read(std::path::Path::new(path)).unwrap_or_else(|e| die(&e));
    let mut by_tag: BTreeMap<u8, Vec<&capture::File>> = BTreeMap::new();
    for f in &files {
        by_tag.entry(f.tag).or_default().push(f);
    }
    println!("{}\n", rerail_tape_lab::measure::machine());
    for (tag, fs) in &by_tag {
        let r = rerail_tape_lab::nofield::run(fs);
        rerail_tape_lab::nofield::print(&format!("{path}, {} ({tag})", capture::language(*tag)), &r);
    }
}

/// E15 (t39 step 7): repeated structure as repeated bits, per slot encoding (`repeat.rs`).
fn cmd_repeat(path: &str) {
    use rerail_tape_lab::inplace::Codes;
    let files = capture::read(std::path::Path::new(path)).unwrap_or_else(|e| die(&e));
    let mut by_tag: BTreeMap<u8, Vec<rerail_tape_lab::tree::Tree>> = BTreeMap::new();
    for f in &files {
        by_tag.entry(f.tag).or_default().push(rerail_tape_lab::v15::read(&f.image).unwrap_or_else(|e| die(&e)));
    }
    println!("{}\n", rerail_tape_lab::measure::machine());
    for (tag, trees) in &by_tag {
        let mut codes = Codes::default();
        for t in trees {
            codes.learn(t);
        }
        let r = rerail_tape_lab::repeat::run(trees, &codes);
        rerail_tape_lab::repeat::print(&format!("{path}, language {tag}"), &r);
        println!();
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let opt = |f: &str, d: usize| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(d);
    let (Some(cmd), Some(path)) = (args.first(), args.get(1)) else {
        die("usage: tape16 grid|text|edits|keys|gpu <capture> [options]")
    };
    match cmd.as_str() {
        "grid" => {
            let axis = args.iter().position(|a| a == "--axis").and_then(|i| args.get(i + 1)).map_or("all", String::as_str);
            match opt("--shards", 1) {
                1 => cmd_grid(path, flag("--full"), axis, flag("--langs"), !flag("--no-check")),
                n => {
                    let only = args.iter().position(|a| a == "--only").and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok());
                    cmd_grid_streamed(path, flag("--full"), axis, !flag("--no-check"), n as u64, only)
                }
            }
        }
        "text" => cmd_text(path),
        "edits" => cmd_edits(path),
        "keys" => cmd_keys(path),
        "writer" => cmd_writer(path),
        "dicts" => cmd_dicts(path),
        #[cfg(target_os = "macos")]
        "scan" => cmd_scan(path),
        "textrel" => cmd_textrel(path),
        "inplace" => cmd_inplace(path, flag("--f11")),
        "climb" => cmd_climb(path),
        "prim" => cmd_prim(path),
        "repeat" => cmd_repeat(path),
        "relative" => cmd_relative(path),
        "disk" => cmd_disk(path),
        "ralz" => cmd_ralz(path),
        "code8" => cmd_code8(path),
        "nofield" => cmd_nofield(path),
        "f16" => cmd_f16(path),
        "f16w" => cmd_f16w(path),
        #[cfg(target_os = "macos")]
        "f16-gpu" => cmd_f16_gpu(path),
        #[cfg(feature = "grammar")]
        "gcodes" => cmd_gcodes(path),
        #[cfg(target_os = "macos")]
        "inplace-gpu" => cmd_inplace_gpu(path),
        #[cfg(target_os = "macos")]
        "prim-gpu" => cmd_prim_gpu(path),
        "base" => cmd_base(path, args.iter().position(|a| a == "--dump").map(|i| args[i + 1].as_str())),
        #[cfg(target_os = "macos")]
        "columns" => {
            let lang = args.iter().position(|a| a == "--lang").and_then(|i| args.get(i + 1)).map(String::as_str);
            cmd_columns(path, match opt("--cut", 1024) {
                0 => Cut::Whole,
                n => Cut::Target(n),
            }, lang)
        }
        #[cfg(target_os = "macos")]
        "gpu" => {
            let text = |f: &str| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).map(String::as_str);
            let cut = match opt("--cut", 0) {
                0 => Cut::Whole,
                n => Cut::Target(n),
            };
            cmd_gpu(path, opt("--dict", 64), opt("--k", 64), cut, text("--lang"), text("--codecs"))
        }
        _ => die(&format!("unknown command {cmd}")),
    }
}
