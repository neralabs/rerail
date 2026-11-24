//! A batch of tapes' auxiliary indexes built on the device (decision 0656 step 2): the plan of launches over `aux_index.uglykl`'s
//! kernels and the library's scans, compactions and stable radix sorts. Its bytes are `rerail_tape_format::aux_index::
//! build`'s, every section, document after document (`the_device_builds_each_aux_index_as_the_crate_does`).
//!
//! The launches are fixed for any batch: none per document, none per node. The one wait is for the output's size,
//! which the device computes (each document's sections' lengths, scanned) before the output is made.
//!
//! The work: a pass over the nodes (leaves, codes, families), one over the leaves (each text's hash), a sort of the
//! leaves by (document, hash) for the terms, a sort of the terms by (document, count, first) for their numbers, a sort
//! of the nodes by (document, code) for the counts and the rare codes; then each section written where it lies.

use crate::device::{Buffer, BufferRef, ComputeQueue};
use crate::kernels::{alloc, buffer_of, launch, program, read, Program};
use crate::library::{compact_u32, radix_sort_pairs, scan_exclusive_u32, Sorted};
use crate::Pod;

const SOURCE: &str = include_str!("aux_index.uglykl");

fn kernels() -> Result<&'static Program, String> {
    program("aux_index.uglykl", SOURCE)
}

/// A document of a batch: its tape at byte `tape` of the tapes' buffer (8-byte aligned), its source at byte `src` of
/// the sources', its tape's digest (the auxiliary index's header), its nodes, and its view's words (its offsets from `tape`).
#[derive(Clone, Copy)]
pub struct Doc {
    pub tape: u32,
    pub src: u32,
    pub digest: u64,
    pub nodes: u32,
    pub view: [u32; 32],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ScDoc {
    tape: u32,
    src: u32,
    digest_lo: u32,
    digest_hi: u32,
}
unsafe impl Pod for ScDoc {}

#[repr(C)]
#[derive(Clone, Copy)]
struct ScArgs {
    docs: u32,
    nodes: u32,
    blocks: u32,
    pad: u32,
}
unsafe impl Pod for ScArgs {}

/// The words of a document's record (`I_WORDS`).
const INFO: usize = 28;

/// The auxiliary indexes of a batch: one buffer, each document's at its offset (`at[d]..at[d + 1]`).
pub struct Built {
    pub out: Buffer,
    pub at: Vec<u32>,
}

/// Why a batch's auxiliary indexes were not built on the device, where the host builds them (`aux_index::build`).
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// Two texts of a document have one hash.
    Collision,
    /// 65535 documents or more, or a code of 65536 or more: past the sorts' keys.
    Limits,
}

/// The auxiliary indexes of `docs`, their tapes in `tapes` and sources in `sources`, every section. `Ok(Err(_))` where the
/// batch is one the device does not build (`Refused`).
pub fn build(queue: &mut ComputeQueue, tapes: &BufferRef, sources: &BufferRef, docs: &[Doc]) -> Result<Result<Built, Refused>, String> {
    let k = kernels()?;
    let d = docs.len();
    if d == 0 {
        return Ok(Ok(Built { out: alloc::<u32>(4)?, at: vec![0] }));
    }
    if d >= 0xffff {
        return Ok(Err(Refused::Limits));
    }
    let mut node_at = Vec::with_capacity(d + 1);
    let mut block_at = Vec::with_capacity(d + 1);
    let (mut n, mut b) = (0u32, 0u32);
    for x in docs {
        node_at.push(n);
        block_at.push(b);
        n += x.nodes;
        b += x.nodes.div_ceil(256);
    }
    node_at.push(n);
    block_at.push(b);
    let args = ScArgs { docs: d as u32, nodes: n, blocks: b, pad: 0 };
    let nn = n as usize;
    let aux_docs: Vec<ScDoc> = docs.iter().map(|x| ScDoc { tape: x.tape, src: x.src, digest_lo: x.digest as u32, digest_hi: (x.digest >> 32) as u32 }).collect();
    let views: Vec<u32> = docs.iter().flat_map(|x| x.view).collect();
    let (docs_b, node_at_b, block_at_b, views_b) = (buffer_of(&aux_docs)?, buffer_of(&node_at)?, buffer_of(&block_at)?, buffer_of(&views)?);
    let bad = buffer_of(&[0u32; 4])?;
    let u32s = |len: usize| alloc::<u32>(len);
    let u64s = |len: usize| alloc::<u64>(len);
    let grid = |count: u32| [u64::from(count.div_ceil(256)).max(1)];
    const BLOCK: [u64; 1] = [256];
    let q: &ComputeQueue = queue;

    // The nodes.
    let (leaf, code_key, iota, sib_n) = (u32s(nn)?, u32s(nn)?, u32s(nn)?, u32s(nn)?);
    launch(q, k.function("aux_nodes")?)
        .value("args", &args)
        .buffer("docs", &docs_b)
        .buffer("nodeAt", &node_at_b)
        .buffer("tapes", tapes)
        .buffer("views", &views_b)
        .buffer("leaf", &leaf)
        .buffer("codeKey", &code_key)
        .buffer("iota", &iota)
        .buffer("sibN", &sib_n)
        .buffer("bad", &bad)
        .run(&grid(n), &BLOCK)?;
    let (leaf_at, leaf_node) = (u32s(nn + 1)?, u32s(nn)?);
    compact_u32(q, &iota, &leaf, &leaf_node, &leaf_at, n)?;
    let sib_at = u32s(nn + 1)?;
    scan_exclusive_u32(q, &sib_n, &sib_at, n)?;

    // The leaves, sorted by (document, hash, leaf).
    let (hash_key, leaf_hash, leaf_doc, l_iota) = (u64s(nn)?, u64s(nn)?, u32s(nn)?, u32s(nn)?);
    launch(q, k.function("aux_leaf_hash")?)
        .value("args", &args)
        .buffer("docs", &docs_b)
        .buffer("nodeAt", &node_at_b)
        .buffer("tapes", tapes)
        .buffer("views", &views_b)
        .buffer("sources", sources)
        .buffer("leafAt", &leaf_at)
        .buffer("leafNode", &leaf_node)
        .buffer("hashKey", &hash_key)
        .buffer("leafHash", &leaf_hash)
        .buffer("leafDoc", &leaf_doc)
        .buffer("iota", &l_iota)
        .run(&grid(n), &BLOCK)?;
    let o = sort_by_doc::<u64>(q, k, &args, n, &hash_key, l_iota, 64, &leaf_doc)?;
    let (head, p_iota, run_of, run_start) = (u32s(nn)?, u32s(nn)?, u32s(nn + 1)?, u32s(nn + 1)?);
    launch(q, k.function("aux_term_heads")?)
        .value("args", &args)
        .buffer("leafAt", &leaf_at)
        .buffer("o", &o)
        .buffer("leafHash", &leaf_hash)
        .buffer("leafDoc", &leaf_doc)
        .buffer("head", &head)
        .buffer("iota", &p_iota)
        .run(&grid(n), &BLOCK)?;
    compact_u32(q, &p_iota, &head, &run_start, &run_of, n)?;

    // The terms, sorted by (document, count descending, first leaf): their numbers.
    let (freq_key, run_doc, r_iota) = (u64s(nn)?, u32s(nn)?, u32s(nn)?);
    launch(q, k.function("aux_term_runs")?)
        .value("args", &args)
        .buffer("nodeAt", &node_at_b)
        .buffer("leafAt", &leaf_at)
        .buffer("o", &o)
        .buffer("leafDoc", &leaf_doc)
        .buffer("runOf", &run_of)
        .buffer("runStart", &run_start)
        .buffer("freqKey", &freq_key)
        .buffer("runDoc", &run_doc)
        .buffer("iota", &r_iota)
        .run(&grid(n + 1), &BLOCK)?;
    let freq = sort_by_doc::<u64>(q, k, &args, n, &freq_key, r_iota, 64, &run_doc)?;
    let number = u32s(nn)?;
    launch(q, k.function("aux_numbers")?)
        .value("args", &args)
        .buffer("runOf", &run_of)
        .buffer("freq", &freq)
        .buffer("runDoc", &run_doc)
        .buffer("number", &number)
        .run(&grid(n), &BLOCK)?;
    let (leaf_term, esc) = (u32s(nn)?, u32s(nn)?);
    launch(q, k.function("aux_leaf_terms")?)
        .value("args", &args)
        .buffer("docs", &docs_b)
        .buffer("nodeAt", &node_at_b)
        .buffer("tapes", tapes)
        .buffer("views", &views_b)
        .buffer("sources", sources)
        .buffer("leafAt", &leaf_at)
        .buffer("leafNode", &leaf_node)
        .buffer("leafDoc", &leaf_doc)
        .buffer("o", &o)
        .buffer("runOf", &run_of)
        .buffer("runStart", &run_start)
        .buffer("number", &number)
        .buffer("leafTerm", &leaf_term)
        .buffer("esc", &esc)
        .buffer("bad", &bad)
        .run(&grid(n), &BLOCK)?;
    let (esc_at, esc_leaf) = (u32s(nn + 1)?, u32s(nn)?);
    let e_iota = identity(q, k, &args, n)?;
    compact_u32(q, &e_iota, &esc, &esc_leaf, &esc_at, n)?;

    // The codes, sorted by (document, code).
    let (co_tmp_k, co_tmp_v) = (u32s(nn)?, u32s(nn)?);
    let (ck, co) = match radix_sort_pairs::<u32>(q, &code_key, &iota, &co_tmp_k, &co_tmp_v, n, 32)? {
        Sorted::InPlace => (code_key, iota),
        Sorted::InTemp => (co_tmp_k, co_tmp_v),
    };
    let (c_head, c_iota, code_of, code_start) = (u32s(nn)?, u32s(nn)?, u32s(nn + 1)?, u32s(nn + 1)?);
    launch(q, k.function("aux_code_heads")?)
        .value("args", &args)
        .buffer("key", &ck)
        .buffer("head", &c_head)
        .buffer("iota", &c_iota)
        .run(&grid(n), &BLOCK)?;
    compact_u32(q, &c_iota, &c_head, &code_start, &code_of, n)?;
    let (code_doc, rare, rare_n) = (u32s(nn)?, u32s(nn)?, u32s(nn)?);
    launch(q, k.function("aux_code_runs")?)
        .value("args", &args)
        .buffer("nodeAt", &node_at_b)
        .buffer("key", &ck)
        .buffer("codeOf", &code_of)
        .buffer("codeStart", &code_start)
        .buffer("codeDoc", &code_doc)
        .buffer("rare", &rare)
        .buffer("rareN", &rare_n)
        .run(&grid(n + 1), &BLOCK)?;
    let (rare_at, rare_n_at) = (u32s(nn + 1)?, u32s(nn + 1)?);
    scan_exclusive_u32(q, &rare, &rare_at, n)?;
    scan_exclusive_u32(q, &rare_n, &rare_n_at, n)?;

    // The layout, and the one wait: the output's size.
    let (info, size, out_at) = (u32s(d * INFO)?, u32s(d + 1)?, u32s(d + 1)?);
    launch(q, k.function("aux_layout")?)
        .value("args", &args)
        .buffer("nodeAt", &node_at_b)
        .buffer("leafAt", &leaf_at)
        .buffer("runOf", &run_of)
        .buffer("runDoc", &run_doc)
        .buffer("escAt", &esc_at)
        .buffer("sibAt", &sib_at)
        .buffer("codeOf", &code_of)
        .buffer("codeDoc", &code_doc)
        .buffer("rareAt", &rare_at)
        .buffer("rareNAt", &rare_n_at)
        .buffer("info", &info)
        .buffer("size", &size)
        .run(&grid(d as u32), &BLOCK)?;
    scan_exclusive_u32(q, &size, &out_at, d as u32)?;
    queue.flush().await_()?;
    let flags: Vec<u32> = read(&bad, 2);
    if flags[1] != 0 {
        return Ok(Err(Refused::Limits));
    }
    if flags[0] != 0 {
        return Ok(Err(Refused::Collision));
    }
    let at: Vec<u32> = read(&out_at, d + 1);
    let words = (at[d] / 4) as usize;
    let out = u32s(words)?;
    let q: &ComputeQueue = queue;
    launch(q, k.function("aux_zero")?).value("args", &args).buffer("outAt", &out_at).buffer("out", &out).run(&grid(words as u32), &BLOCK)?;

    // The sections, each where it lies.
    launch(q, k.function("aux_header")?)
        .value("args", &args)
        .buffer("docs", &docs_b)
        .buffer("info", &info)
        .buffer("outAt", &out_at)
        .buffer("escAt", &esc_at)
        .buffer("escLeaf", &esc_leaf)
        .buffer("out", &out)
        .run(&grid(d as u32), &BLOCK)?;
    launch(q, k.function("aux_leaves")?)
        .value("args", &args)
        .buffer("nodeAt", &node_at_b)
        .buffer("blockAt", &block_at_b)
        .buffer("leafAt", &leaf_at)
        .buffer("info", &info)
        .buffer("outAt", &out_at)
        .buffer("out", &out)
        .run(&grid(b), &BLOCK)?;
    launch(q, k.function("aux_names")?)
        .value("args", &args)
        .buffer("leafAt", &leaf_at)
        .buffer("leafDoc", &leaf_doc)
        .buffer("leafTerm", &leaf_term)
        .buffer("escAt", &esc_at)
        .buffer("info", &info)
        .buffer("outAt", &out_at)
        .buffer("out", &out)
        .run(&grid(n), &BLOCK)?;
    launch(q, k.function("aux_vocab")?)
        .value("args", &args)
        .buffer("docs", &docs_b)
        .buffer("nodeAt", &node_at_b)
        .buffer("tapes", tapes)
        .buffer("views", &views_b)
        .buffer("leafNode", &leaf_node)
        .buffer("leafHash", &leaf_hash)
        .buffer("o", &o)
        .buffer("runOf", &run_of)
        .buffer("runStart", &run_start)
        .buffer("runDoc", &run_doc)
        .buffer("number", &number)
        .buffer("info", &info)
        .buffer("outAt", &out_at)
        .buffer("out", &out)
        .run(&grid(n), &BLOCK)?;
    launch(q, k.function("aux_siblings")?)
        .value("args", &args)
        .buffer("docs", &docs_b)
        .buffer("nodeAt", &node_at_b)
        .buffer("tapes", tapes)
        .buffer("views", &views_b)
        .buffer("sibAt", &sib_at)
        .buffer("info", &info)
        .buffer("outAt", &out_at)
        .buffer("out", &out)
        .run(&grid(n), &BLOCK)?;
    launch(q, k.function("aux_counts")?)
        .value("args", &args)
        .buffer("key", &ck)
        .buffer("codeOf", &code_of)
        .buffer("codeStart", &code_start)
        .buffer("codeDoc", &code_doc)
        .buffer("rareAt", &rare_at)
        .buffer("rareNAt", &rare_n_at)
        .buffer("info", &info)
        .buffer("outAt", &out_at)
        .buffer("out", &out)
        .run(&grid(n), &BLOCK)?;
    launch(q, k.function("aux_rare_nodes")?)
        .value("args", &args)
        .buffer("nodeAt", &node_at_b)
        .buffer("co", &co)
        .buffer("codeOf", &code_of)
        .buffer("codeStart", &code_start)
        .buffer("codeDoc", &code_doc)
        .buffer("rareAt", &rare_at)
        .buffer("rareNAt", &rare_n_at)
        .buffer("info", &info)
        .buffer("outAt", &out_at)
        .buffer("out", &out)
        .run(&grid(n), &BLOCK)?;
    queue.flush().await_()?;
    Ok(Ok(Built { out, at }))
}

/// `values` sorted by (`of[value]`, `keys`), stably: a sort by `keys` (`bits` of them), then by each row's document
/// (`of` its value's, below 65536). The rows' values, in their order.
#[allow(clippy::too_many_arguments)]
fn sort_by_doc<K: crate::library::RadixKey>(
    q: &ComputeQueue,
    k: &Program,
    args: &ScArgs,
    n: u32,
    keys: &BufferRef,
    values: Buffer,
    bits: u32,
    of: &BufferRef,
) -> Result<Buffer, String> {
    let nn = n as usize;
    let (tk, tv) = (alloc::<K>(nn)?, alloc::<u32>(nn)?);
    let order = match radix_sort_pairs::<K>(q, keys, &values, &tk, &tv, n, bits)? {
        Sorted::InPlace => values,
        Sorted::InTemp => tv,
    };
    let (doc_key, tk2, tv2) = (alloc::<u32>(nn)?, alloc::<u32>(nn)?, alloc::<u32>(nn)?);
    launch(q, k.function("aux_gather_doc")?)
        .value("args", args)
        .buffer("order", &order)
        .buffer("of", of)
        .buffer("key", &doc_key)
        .run(&[u64::from(n.div_ceil(256)).max(1)], &[256])?;
    Ok(match radix_sort_pairs::<u32>(q, &doc_key, &order, &tk2, &tv2, n, 16)? {
        Sorted::InPlace => order,
        Sorted::InTemp => tv2,
    })
}

/// `0..n`, as a buffer.
fn identity(q: &ComputeQueue, k: &Program, args: &ScArgs, n: u32) -> Result<Buffer, String> {
    let iota = alloc::<u32>(n as usize)?;
    launch(q, k.function("aux_iota")?).value("args", args).buffer("iota", &iota).run(&[u64::from(n.div_ceil(256)).max(1)], &[256])?;
    Ok(iota)
}

#[cfg(test)]
mod tests {
    use rerail_tape_format::aux_index;
    use rerail_tape_format::tape::Tape;

    /// Decision 0656 step 2: the device builds each document's auxiliary index as `aux_index::build` does, byte for byte: a
    /// batch of the sample document and smaller cuts of it (one node; a few; one leaf family), format 16 tapes, on the
    /// process's device. Without a device it passes, saying so.
    #[test]
    fn the_device_builds_each_aux_index_as_the_crate_does() {
        let Some(shared) = crate::shared::get() else {
            eprintln!("no device: the auxiliary index's builder did not run");
            return;
        };
        let mut queue = crate::device::ComputeQueue::new(&shared.queue);
        let sample = aux_index::sample::Doc::new();
        let lang = rerail_tape_format::f16::Lang::new(sample.table.clone(), [1, 1, 1], 1).unwrap();
        let number = rerail_tape_format::f16::numbers(&lang, &sample.table).unwrap();
        // Documents: the sample, and its first subtrees as documents of their own.
        let mut docs: Vec<(Vec<(u32, u64, u64, usize)>, Vec<u8>)> = vec![(sample.nodes.clone(), sample.source.clone())];
        let mut c = 1;
        while c < sample.nodes.len() && docs.len() < 6 {
            let end = sample.nodes[c].3;
            let (s0, s1) = (sample.nodes[c].1, sample.nodes[c].2);
            let nodes = sample.nodes[c..end].iter().map(|&(k, s, e, se)| (k, s - s0, e - s0, se - c)).collect();
            docs.push((nodes, sample.source[s0 as usize..s1 as usize].to_vec()));
            c = end;
        }
        let (mut tapes, mut sources, mut want, mut ins) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (k, (nodes, source)) in docs.iter().enumerate() {
            let doc = aux_index::sample::Doc { nodes: nodes.clone(), source: source.clone(), table: sample.table.clone() };
            let Ok(rerail_tape_format::f16::Written::F16(t)) =
                rerail_tape_format::f16::write_from(&lang, &doc, &number, source, rerail_tape_format::newlines::Terminators::Lf, 0, false)
            else {
                panic!("format 16")
            };
            let r = t.reader(&lang).unwrap();
            want.push(aux_index::build(&r, source, 100 + k as u64, aux_index::Sections::ALL).unwrap());
            let view = rerail_tape_format::f16::placement(t.as_bytes()).unwrap().view(&lang, 0).unwrap();
            // SAFETY: the view is plain u32s (`repr(C)`, 128 bytes).
            let words: [u32; 32] = unsafe { std::ptr::read(std::ptr::from_ref(&view).cast::<[u32; 32]>()) };
            tapes.resize(tapes.len().next_multiple_of(8), 0);
            ins.push(super::Doc { tape: tapes.len() as u32, src: sources.len() as u32, digest: 100 + k as u64, nodes: r.len() as u32, view: words });
            tapes.extend_from_slice(t.as_bytes());
            sources.extend_from_slice(source);
        }
        tapes.resize(tapes.len().next_multiple_of(8) + 8, 0);
        sources.resize(sources.len().next_multiple_of(8) + 8, 0);
        let (tb, sb) = (crate::kernels::buffer_of(&tapes).unwrap(), crate::kernels::buffer_of(&sources).unwrap());
        let built = super::build(&mut queue, &tb, &sb, &ins).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend())).expect("built on the device");
        let words: Vec<u32> = crate::kernels::read(&built.out, (built.at[ins.len()] / 4) as usize);
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        for (k, w) in want.iter().enumerate() {
            let got = &bytes[built.at[k] as usize..built.at[k + 1] as usize];
            if got != &w[..] {
                let at = got.iter().zip(w.iter()).position(|(a, b)| a != b).unwrap_or(got.len().min(w.len()));
                panic!("{}: document {k} ({} nodes): {} bytes, want {}; first difference at byte {at}", crate::device::backend(), ins[k].nodes, got.len(), w.len());
            }
            assert!(aux_index::AuxIndex::read(got).is_ok());
        }
        eprintln!("{}: {} auxiliary indexes built, {} bytes", crate::device::backend(), ins.len(), bytes.len());
    }

    /// A text's hash on the device (`ukl_aux_text_hash`) is `aux_index::text_hash`: texts of every length to 40 bytes,
    /// at every alignment of their start and of the base they are read through.
    #[test]
    fn the_device_hashes_texts_as_the_crate_does() {
        let Some(shared) = crate::shared::get() else {
            eprintln!("no device: the hash did not run");
            return;
        };
        let mut queue = crate::device::ComputeQueue::new(&shared.queue);
        const K: &str = "#include <ukl/reads/aux_index.h>\n\
            DEFINE_KERNEL2(t, t_hash, ROBUFFER, uint8_t, src, \"[bytes]\", ROBUFFER, uint32_t, at, \"[n * 3]\", RWBUFFER, uint64_t, out, \"[n]\", GID0, uint32_t, i, )\n\
            { if (i < at[0]) out[i] = ukl_aux_text_hash(src + at[3u * i + 1u], at[3u * i + 2u], at[3u * i + 3u]); }\n";
        let p = crate::kernels::program("aux_index_hash", K).unwrap_or_else(|e| panic!("{}: {e}", crate::device::backend()));
        let src: Vec<u8> = (0..256u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        let mut at = vec![0u32];
        let mut want = Vec::new();
        for base in 0..8u32 {
            for start in 0..9u32 {
                for len in 0..41u32 {
                    at.extend([base, start, len]);
                    want.push(aux_index::text_hash(&src[(base + start) as usize..][..len as usize]));
                }
            }
        }
        at[0] = want.len() as u32;
        let (sb, ab, ob) = (crate::kernels::buffer_of(&src).unwrap(), crate::kernels::buffer_of(&at).unwrap(), crate::kernels::alloc::<u64>(want.len()).unwrap());
        crate::kernels::launch(&queue, p.function("t_hash").unwrap())
            .buffer("src", &sb)
            .buffer("at", &ab)
            .buffer("out", &ob)
            .run(&[want.len().div_ceil(64) as u64], &[64])
            .unwrap();
        queue.flush().await_().unwrap();
        let got: Vec<u64> = crate::kernels::read(&ob, want.len());
        for (k, (g, w)) in got.iter().zip(&want).enumerate() {
            let (base, start, len) = (at[1 + 3 * k], at[2 + 3 * k], at[3 + 3 * k]);
            assert_eq!(g, w, "{}: base {base}, start {start}, length {len}", crate::device::backend());
        }
    }
}
