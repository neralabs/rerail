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
//! tape16 gpu   <cap> [--dict KB] [--k K] [--cut N]      E3: the LZ codecs decoded on Metal
//! ```

use rerail_tape_lab::blocks::{self, Cut};
use rerail_tape_lab::codec::{self, Codec};
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
        push(Solution { codec: Codec::LpSpan { depth: 4, cross: true }, cut: Cut::Target(1024), dict: 16 * 1024, entropy: true, ..b }, &mut v);
        push(Solution { codec: Codec::LpSpan { depth: 4, cross: true }, cut: Cut::Target(256), dict: 16 * 1024, entropy: true, ..b }, &mut v);
    }
    if axis == "dictcut" {
        for cut in [Cut::Target(256), Cut::Target(1024)] {
            for dict in [0, 16 * 1024, 64 * 1024] {
                push(Solution { codec: Codec::Zstd, cut, dict, ..b }, &mut v);
                for codec in [Codec::LzOutput, Codec::LpSpan { depth: 4, cross: true }] {
                    push(Solution { codec, cut, dict, entropy: true, ..b }, &mut v);
                }
            }
        }
    }
    if axis == "huff" {
        for cut in [Cut::Whole, Cut::Target(1024)] {
            for codec in [Codec::LzOutput, Codec::LpLz, Codec::LpReplay, Codec::LpSpan { depth: 4, cross: false }, Codec::LpSpan { depth: 4, cross: true }] {
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
fn cmd_gpu(path: &str, dict_kb: usize, k: usize, cut: Cut) {
    use rerail_tape_lab::lp::Refs;
    let corpus = load(path);
    let mut dicts = Dicts::default();
    println!("Blocks: {}, dictionary {dict_kb} KB, K {k}\n", cut.name());
    println!("| language | codec | threads | MB out | GPU µs | GB/s | CPU decode µs | correct |");
    println!("|---|---|---|---|---|---|---|---|");
    let tags: std::collections::BTreeSet<u8> = corpus.files.iter().map(|p| p.tag).collect();
    let codecs = [
        Codec::LpLz,
        Codec::LpReplay,
        Codec::LpSpan { depth: 1, cross: false },
        Codec::LpSpan { depth: 4, cross: false },
        Codec::LpSpan { depth: 4, cross: true },
        Codec::LzOutput,
    ];
    for tag in tags {
        for c in codecs {
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
                    for b in 0..cf.len() {
                        std::hint::black_box(codec::decompress_in(c, cf, b, &dict, lf[b].len()));
                    }
                }
            });
            drop(files);
            // Each run in its own pool: Metal's command buffers are autoreleased, and hold their buffers.
            let run = metal::objc::rc::autoreleasepool(|| match c {
                Codec::LpSpan { .. } => rerail_tape_lab::gpu::decode_span(&comp, &dict.bytes),
                Codec::LpLz => rerail_tape_lab::gpu::decode(&comp.concat(), &dict.bytes, Refs::Literals),
                Codec::LpReplay => rerail_tape_lab::gpu::decode(&comp.concat(), &dict.bytes, Refs::Replay),
                _ => rerail_tape_lab::gpu::decode(&comp.concat(), &dict.bytes, Refs::Output),
            });
            if let Codec::LpSpan { .. } = c {
                let t = dicts.tables(&corpus, tag, &Solution { entropy: true, ..s });
                let coded: Vec<Vec<Vec<u8>>> = comp.iter().map(|f| f.iter().map(|b| rerail_tape_lab::huff::encode(b, &t.0, &t.1)).collect()).collect();
                match metal::objc::rc::autoreleasepool(|| rerail_tape_lab::gpu::decode_span_huff(&coded, &dict.bytes, &t.0, &t.1)) {
                    Ok(g) => println!(
                        "| {} | {}+huff | {} | {:.2} | {:.0} + {:.0} | {:.2} | | {} |",
                        capture::language(tag),
                        c.name(),
                        g.threads,
                        g.bytes_out as f64 / 1e6,
                        g.huff_seconds * 1e6,
                        g.lz_seconds * 1e6,
                        g.bytes_out as f64 / (g.huff_seconds + g.lz_seconds).max(1e-9) / 1e9,
                        g.correct
                    ),
                    Err(e) => println!("| {} | {}+huff | error: {e} |", capture::language(tag), c.name()),
                }
            }
            match run {
                Ok(g) => println!(
                    "| {} | {} | {} | {:.2} | {:.0} | {:.2} | {:.0} | {} |",
                    capture::language(tag),
                    c.name(),
                    g.threads,
                    g.bytes_out as f64 / 1e6,
                    g.gpu_seconds * 1e6,
                    g.bytes_out as f64 / g.gpu_seconds.max(1e-9) / 1e9,
                    cpu.nanos as f64 / 1000.0,
                    g.correct
                ),
                Err(e) => println!("| {} | {} | error: {e} |", capture::language(tag), c.name()),
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
                        if d.start[g] != t.nodes[i].start || d.end[g] != t.nodes[i].end || d.depth[g] != depth[i] || d.parent[g] != parent {
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
        #[cfg(target_os = "macos")]
        "gpu" => cmd_gpu(path, opt("--dict", 64), opt("--k", 64), match opt("--cut", 0) {
            0 => Cut::Whole,
            n => Cut::Target(n),
        }),
        _ => die(&format!("unknown command {cmd}")),
    }
}
