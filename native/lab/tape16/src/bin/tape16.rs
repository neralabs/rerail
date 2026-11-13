//! The lab's CLI: each command measures one group of 0156's tradeoffs on a capture (`rerail-engine capture-tapes`).
//!
//! ```text
//! tape16 grid  <cap> [--full] [--langs] [--no-check]   T1–T5, E1/E2/E6: bytes, codes, speed, random reads
//! tape16 text  <cap>                                    T6, E5: text levels against the stored source
//! tape16 edits <pairs.cap>                              T7, E4: blocks and rows an edit changes, per identity
//! tape16 keys  <cap>                                    E8: widths every key and count needs
//! tape16 gpu   <cap> [--dict KB] [--k K]                E3: literal-pool LZ decoded on Metal
//! ```

use rerail_tape_lab::blocks::{self, Cut};
use rerail_tape_lab::codec::Codec;
use rerail_tape_lab::layout::{Extras, Layout};
use rerail_tape_lab::solution::{self, Corpus, Dicts, Solution};
use rerail_tape_lab::{capture, edits, events, layout, report, text};
use std::collections::{BTreeMap, HashMap};

fn base() -> Solution {
    Solution { layout: Layout::BytePlanes, codec: Codec::LpLz, cut: Cut::Whole, dict: 64 * 1024, k: 64, extras: Extras::default() }
}

/// One axis at a time from the base, or (`full`) every combination of the main axes.
fn grid(full: bool) -> Vec<Solution> {
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
    for layout in Layout::ALL {
        for codec in Codec::ALL {
            push(Solution { layout, codec, ..b }, &mut v);
        }
    }
    for cut in cuts {
        push(Solution { cut, ..b }, &mut v);
        push(Solution { cut, codec: Codec::Zstd, ..b }, &mut v);
    }
    for dict in dicts {
        push(Solution { dict, ..b }, &mut v);
        push(Solution { dict, codec: Codec::Zstd, ..b }, &mut v);
    }
    for k in [16, 64, 256] {
        push(Solution { k, ..b }, &mut v);
    }
    for extras in [Extras { parent: true, size: false }, Extras { parent: false, size: true }, Extras { parent: true, size: true }] {
        push(Solution { extras, ..b }, &mut v);
        push(Solution { extras, codec: Codec::Zstd, ..b }, &mut v);
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

fn cmd_grid(path: &str, full: bool, langs: bool, check: bool) {
    let corpus = load(path);
    let test = corpus.files.iter().filter(|p| p.test).count();
    let nodes: usize = corpus.files.iter().filter(|p| p.test).map(|p| p.nodes).sum();
    println!("{path}: {} files, {test} in the test half ({nodes} nodes)\n", corpus.files.len());
    let mut dicts = Dicts::default();
    let mut rows = Vec::new();
    for s in grid(full) {
        let r = solution::evaluate(&corpus, &s, &mut dicts, check);
        eprintln!("  {}", s.name());
        rows.push(r);
    }
    report::print_grid(&rows);
    if langs {
        report::print_langs(&rows[0]);
    }
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
            edits::compare(&old.tree(&corpus), &new.tree(&corpus), &old.events, &new.events, cut, &lay, &mut st);
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
            println!("|  | (D: {} shift ops covering {} rows) |", st.shift_ops, st.shifted_rows);
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
fn cmd_gpu(path: &str, dict_kb: usize, k: usize) {
    use rerail_tape_lab::codec::{self, Dictionary};
    use rerail_tape_lab::lp::Refs;
    let corpus = load(path);
    let mut dicts = Dicts::default();
    println!("| language | refs | threads | MB out | GPU µs | GB/s | CPU decode µs | correct |");
    println!("|---|---|---|---|---|---|---|---|");
    let tags: std::collections::BTreeSet<u8> = corpus.files.iter().map(|p| p.tag).collect();
    for tag in tags {
        for (refs, c) in [(Refs::Literals, Codec::LpLz), (Refs::Replay, Codec::LpReplay), (Refs::Output, Codec::LzOutput)] {
            let s = Solution { codec: c, dict: dict_kb * 1024, k, ..base() };
            let dict = dicts.get(&corpus, tag, &s);
            let laid: Vec<Vec<u8>> = corpus
                .files
                .iter()
                .filter(|p| p.test && p.tag == tag)
                .flat_map(|p| blocks::split(&p.events, s.cut).0.into_iter().map(|b| layout::lay(&b, s.layout, s.extras, &[], &[])))
                .collect();
            let comp: Vec<Vec<u8>> = laid.iter().map(|b| codec::compress(c, b, &dict, k)).collect();
            let (_, cpu) = rerail_tape_lab::measure::cost(|| {
                for (b, l) in comp.iter().zip(&laid) {
                    std::hint::black_box(codec::decompress(c, b, &dict, l.len()));
                }
            });
            let _: &Dictionary = &dict;
            match rerail_tape_lab::gpu::decode(&comp, &dict.bytes, refs) {
                Ok(g) => println!(
                    "| {} | {:?} | {} | {:.2} | {:.0} | {:.2} | {:.0} | {} |",
                    capture::language(tag),
                    refs,
                    g.threads,
                    g.bytes_out as f64 / 1e6,
                    g.gpu_seconds * 1e6,
                    g.bytes_out as f64 / g.gpu_seconds.max(1e-9) / 1e9,
                    cpu.nanos as f64 / 1000.0,
                    g.correct
                ),
                Err(e) => println!("| {} | {refs:?} | error: {e} |", capture::language(tag)),
            }
        }
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
        "grid" => cmd_grid(path, flag("--full"), flag("--langs"), !flag("--no-check")),
        "text" => cmd_text(path),
        "edits" => cmd_edits(path),
        "keys" => cmd_keys(path),
        "dicts" => cmd_dicts(path),
        #[cfg(target_os = "macos")]
        "gpu" => cmd_gpu(path, opt("--dict", 64), opt("--k", 64)),
        _ => die(&format!("unknown command {cmd}")),
    }
}
