//! The auxiliary index's reads in UKL (`<ukl/reads/aux_index.h>`, `<ukl/reads/aux_index_tape.h>`) read as its reads in Rust
//! (`rerail_tape_format::aux_index`), on the CPU's vector path: interpreted and compiled (decision 0656).

use rerail_tape_format::aux_index::{self, NONE, Sections, AuxIndex};
use rerail_tape_format::tape::Tape;

/// Words a thread writes.
const W: usize = 25;

const KERNEL: &str = r#"
#define CLIF_VIEW_SPACE UKL_GLOBAL
#include <ukl/reads/aux_index_tape.h>

typedef struct ScArgs {
    uint32_t n;
    uint32_t threads;
    uint32_t rare;
    uint32_t pad;
} ScArgs;
UKL_REFLECT_STRUCT(ScArgs, n, threads, rare, pad)

DEFINE_KERNEL2(tests, t_aux_index,
               ARGS,     ScArgs,   args, ,
               ROBUFFER, uint8_t,  tape, "[bytes]",
               ROBUFFER, uint32_t, view, "[32]",
               ROBUFFER, uint8_t,  src,  "[bytes]",
               RWBUFFER, uint32_t, out,  "[threads * 25]",
               GID0,     uint32_t, i,    )
{
    if (i >= args.threads)
        return;
    // The index where the tape's home holds it, after its presence (decision 0656 step 3).
    UKL_GLOBAL const uint8_t* aux = tape + ukl_aux_in_home(tape, view);
    RWBUFFER(uint32_t) o = out + i * 25u;
    o[0] = ukl_aux_leaf_rank(aux, i);
    o[1] = ukl_aux_leaf_node(aux, i);
    o[2] = ukl_aux_term_of_leaf(aux, i);
    o[3] = ukl_aux_term_of_node(aux, i);
    o[4] = ukl_aux_term_start(aux, i);
    o[5] = ukl_aux_term_len(aux, i);
    uint64_t h = ukl_aux_term_hash(aux, i);
    o[6] = (uint32_t)h;
    o[7] = (uint32_t)(h >> 32);
    o[8] = ukl_aux_by_hash(aux, i);
    o[9] = ukl_aux_term_by_hash(aux, h);
    uint64_t th = o[4] == UKL_AUX_NONE ? 0u : ukl_aux_text_hash(src, o[4], o[5]);
    o[10] = (uint32_t)th;
    o[11] = (uint32_t)(th >> 32);
    if (i < args.n) {
        uint32_t p = clif_tape_parent(tape, view, i);
        uint32_t k = ukl_aux_place(tape, view, aux, i);
        o[12] = k;
        o[13] = p == CLIF_T_NONE ? CLIF_T_NONE : ukl_aux_child(tape, view, aux, p, k);
        o[14] = ukl_aux_child(tape, view, aux, i, i % 37u);
        o[15] = ukl_aux_eighth(aux, i, 1u);
        o[16] = ukl_aux_eighth(aux, i, 2u);
        uint64_t e = p == CLIF_T_NONE ? 0u : ukl_aux_eighth_at_or_before(aux, p, i);
        o[17] = (uint32_t)e;
        o[18] = (uint32_t)(e >> 32);
    }
    o[19] = ukl_aux_count(aux, i);
    uint64_t r = ukl_aux_rare_range(aux, i);
    o[20] = (uint32_t)r;
    o[21] = (uint32_t)(r >> 32);
    o[22] = i < args.rare ? ukl_aux_rare_node(aux, i) : 0u;
    o[23] = ukl_aux_section(aux, i % 8u);
    o[24] = ukl_aux_named(aux, i);
}
"#;

/// Bytes readable through the 8-byte word that holds the last one, 16-aligned.
fn padded(b: &[u8]) -> Vec<u128> {
    let mut v = vec![0u128; b.len().div_ceil(16) + 1];
    // SAFETY: u128s are 16 bytes each, as many as `b` needs and one more.
    unsafe { std::slice::from_raw_parts_mut(v.as_mut_ptr().cast::<u8>(), b.len()) }.copy_from_slice(b);
    v
}

/// Every read of the auxiliary index in UKL answers as the Rust read: each node's leaf rank, term, place, children and
/// every-8th siblings, each leaf's node and term, each term's entry, hash order and lookup by hash, each text's hash,
/// each code's count and rare nodes, each section's offset. An auxiliary index of a format 16 tape, read with the tape's UKL
/// reads, on the interpreter and compiled.
#[test]
fn the_aux_indexes_ukl_reads_are_its_rust_reads() {
    let d = aux_index::sample::Doc::new();
    let lang = rerail_tape_format::f16::Lang::new(d.table.clone(), [1, 1, 1], 1).unwrap();
    let number = rerail_tape_format::f16::numbers(&lang, &d.table).unwrap();
    // A format 16 tape's home: the tape, its presence, and its auxiliary index laid after it, as the engine stores it.
    let mut w = rerail_tape_format::f16::write_from_with_presence(&lang, &d, &number, &d.source, rerail_tape_format::newlines::Terminators::Lf, 0, false).unwrap().tape;
    let rerail_tape_format::f16::Written::F16(t16) = &w else { panic!("format 16") };
    let bytes = aux_index::build(&t16.reader(&lang).unwrap(), &d.source, 3, &aux_index::sample::named(), Sections::ALL).unwrap();
    w.lay_aux(&bytes).unwrap();
    let rerail_tape_format::f16::Written::F16(t16) = &w else { panic!("format 16") };
    let t = t16.reader(&lang).unwrap();
    assert_eq!(t16.aux_bytes(), Some(&bytes[..]));
    let aux = AuxIndex::read(&bytes).unwrap();
    let n = t.len() as u32;
    let rare_total = (0..64).filter_map(|c| aux.rare_range(c)).map(|r| r.1).max().unwrap_or(0);
    let threads = n.max(aux.leaves()).max(aux.terms()).max(64) + 40;
    // Rust's answers, word for word.
    let mut want = vec![0u32; threads as usize * W];
    for i in 0..threads {
        let o = &mut want[i as usize * W..][..W];
        o[0] = aux.leaf_rank(i);
        o[1] = aux.leaf_node(i);
        o[2] = aux.term_of_leaf(i);
        o[3] = aux.term_of_node(i);
        let (h, start, len) = aux.term(i).unwrap_or((0, NONE, 0));
        (o[4], o[5], o[6], o[7]) = (start, len, h as u32, (h >> 32) as u32);
        o[8] = aux.by_hash(i);
        o[9] = aux.term_by_hash(h);
        let th = if start == NONE { 0 } else { aux_index::text_hash(&d.source[start as usize..][..len as usize]) };
        (o[10], o[11]) = (th as u32, (th >> 32) as u32);
        if i < n {
            let p = t.parent(i as usize).map(|p| p as u32);
            let k = aux_index::place(&t, &aux, i);
            o[12] = k;
            o[13] = p.map_or(NONE, |p| aux_index::child(&t, &aux, p, k).unwrap_or(NONE));
            o[14] = aux_index::child(&t, &aux, i, i % 37).unwrap_or(NONE);
            o[15] = aux.eighth(i, 1);
            o[16] = aux.eighth(i, 2);
            let (k8, e) = p.map_or((0, 0), |p| aux.eighth_at_or_before(p, i));
            (o[17], o[18]) = (e, k8);
        }
        o[19] = aux.count(i);
        let r = aux.rare_range(i).unwrap_or((NONE, NONE));
        (o[20], o[21]) = r;
        o[22] = if i < rare_total { aux.rare_node(i) } else { 0 };
        let kind = aux_index::Kind::ALL.iter().find(|k| **k as u32 == i % 8);
        o[23] = kind.map_or(0, |k| if aux.has(*k) { section_at(&bytes, *k as u16) } else { 0 });
        o[24] = u32::from(aux.named(i));
    }
    // The answers mean something: the reads met leaves, escaped terms, wide families and rare codes.
    assert!(want.chunks(W).any(|o| o[2] != NONE && o[2] >= 255), "an escaped term");
    assert!(want.chunks(W).any(|o| o[15] != NONE) && want.chunks(W).any(|o| o[12] > 100), "wide families");
    assert!(rare_total > 0);
    // UKL's.
    let src = super::source("/<kernels>/aux_index.uglykl", KERNEL, &[]).unwrap_or_else(|e| panic!("{e}"));
    let tape = super::c_tape(&src).unwrap();
    let p = super::lower::lower(&src, &tape).unwrap_or_else(|e| panic!("{e}"));
    let k = super::vector::vectorize(&p, p.kernel("t_aux_index").unwrap()).unwrap_or_else(|e| panic!("{}", e.0));
    let view = rerail_tape_format::f16::placement(t16.extent()).unwrap().view(&lang, 0).unwrap();
    // SAFETY: the view is plain u32s (`repr(C)`).
    let view_words = unsafe { std::slice::from_raw_parts(std::ptr::from_ref(&view).cast::<u8>(), size_of_val(&view)) };
    let (tb, vb, sb) = (padded(t16.extent()), padded(view_words), padded(&d.source));
    for exec in [super::run::Exec::Interpret, super::run::Exec::Jit] {
        let mut out = vec![0u32; threads as usize * W];
        let ptr = |p: *const u128| super::run::Arg::Bytes((p as u64).to_le_bytes().to_vec());
        let args = [
            super::run::Arg::Bytes([n, threads, rare_total, 0].iter().flat_map(|x| x.to_le_bytes()).collect()),
            ptr(tb.as_ptr()),
            ptr(vb.as_ptr()),
            ptr(sb.as_ptr()),
            super::run::Arg::Bytes((out.as_mut_ptr() as u64).to_le_bytes().to_vec()),
            super::run::Arg::Bytes(vec![0; 8]),
            super::run::Arg::Bytes(vec![0; 4]),
        ];
        let launch = super::run::Launch { grid: [threads.div_ceil(32), 1, 1], block: [32, 1, 1], local: 0, resident: 1, exec };
        super::run::run(&p, &k, &args, launch).unwrap_or_else(|e| panic!("{exec:?}: {}", e.0));
        const NAMES: [&str; W] = [
            "leaf rank", "leaf node", "term of leaf", "term of node", "term start", "term len", "hash lo", "hash hi", "by hash",
            "term by hash", "text hash lo", "text hash hi", "place", "child at place", "child i%37", "eighth 1", "eighth 2",
            "eighth at or before: node", "eighth at or before: k", "count", "rare from", "rare to", "rare node", "section", "named",
        ];
        for i in 0..threads as usize {
            for w in 0..W {
                assert_eq!(out[i * W + w], want[i * W + w], "{exec:?}: thread {i}: {}", NAMES[w]);
            }
        }
    }
}

/// Section `kind`'s offset, from the directory.
fn section_at(b: &[u8], kind: u16) -> u32 {
    let count = u16::from_le_bytes([b[6], b[7]]) as usize;
    (0..count)
        .map(|e| aux_index::HEADER + aux_index::ENTRY * e)
        .find(|&d| u16::from_le_bytes([b[d], b[d + 1]]) == kind)
        .map_or(0, |d| u32::from_le_bytes(b[d + 8..d + 12].try_into().unwrap()))
}
