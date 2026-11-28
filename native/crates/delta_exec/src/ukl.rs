//! The operations' UKL versions (decision 0686; the runner's step 4): each operation's kernels, written in
//! `delta_exec/ukl/<module>.uglykl` over the delta library (`<ukl/delta/*.h>`), checked on a device against its twin
//! and registered there, so a runner on that device runs them (`Runner::on`).
//!
//! A version's source is its module's file after its params' declarations (`ParamsLayout::ukl`). The block layouts
//! its kernels read are the twins' own constants, written into `<ukl/delta/layout.h>` (`layout_h`, generated and held
//! equal by a test), so each layout keeps one definition.

use crate::cache::{CACHE_COMMIT, CACHE_OPEN, ORDER, OpCache};
use crate::contract::push_layer;
use crate::dir::Directory;
use crate::drop::{DROP_PARTS, Parts};
use crate::k0::{DOCS_OF, K0_PASS, NAMED_BLOCKS, ROOM_OF};
use crate::intern::INTERN;
use crate::semi::{CHANGE_ROWS, REPLACE_ROWS, SEMI_CHANGE};
use rerail_blocks::block::kind;
use rerail_blocks::gens::set_word;
use rerail_blocks::region::pin_block;
use crate::trace::{RollUp, Trace};
use core::mem::offset_of;
use rerail_blocks::gens::BODY_AT;
use rerail_blocks::layer::{ColType, LayerHead, MAX_LAYERS, NONE, RunsHead, Shape, ShapeW};
use rerail_blocks::relation::{FAR_BASE, Relation};
use crate::op::{Case, Devices, Impl, Kernels, Registry, Runner, Signature, Status, Tiling};
use rerail_delta_twins::Params;
use rerail_delta_twins::cache::{self as cache_twin, CacheP};
use rerail_delta_twins::drop::{self as drop_twin, DropP};
use rerail_delta_twins::k0::{self as k0_twin, DocAt, DocsOfP, NamedP};
use rerail_delta_twins::semi::{ReplaceP, SemiP};
use rerail_delta_twins::intern as intern_twin;
use rerail_blocks::atoms::AtomsRoot;
use rerail_rr_syntax::tape::Kind;

/// Every backend a UKL version runs on.
const EVERY_BACKEND: Devices = Devices(Devices::CPU.0 | Devices::METAL.0 | Devices::CUDA.0 | Devices::HIP.0 | Devices::OPENCL.0);

/// The tilings a version is checked at: whole, and in tiles run out of order.
const TILINGS: [Tiling; 2] = [Tiling::WHOLE, Tiling { tile: 3, seed: 7 }];

/// One operation's UKL version: its signature, its module's kernels, and the case it is checked on.
struct Ukl {
    sig: &'static Signature,
    module: &'static str,
    cases: &'static [fn(&mut Directory) -> Case],
}

/// The modules' kernels, by name.
const MODULES: &[(&str, &str)] = &[
    ("cache", include_str!("../ukl/cache.uglykl")),
    ("drop", include_str!("../ukl/drop.uglykl")),
    ("semi", include_str!("../ukl/semi.uglykl")),
    ("k0", include_str!("../ukl/k0.uglykl")),
    ("intern", include_str!("../ukl/intern.uglykl")),
];

/// The operations with a UKL version.
const VERSIONS: &[Ukl] = &[
    Ukl { sig: &CACHE_OPEN, module: "cache", cases: &[cache_open_case] },
    Ukl { sig: &CACHE_COMMIT, module: "cache", cases: &[cache_commit_case] },
    Ukl { sig: &ORDER, module: "cache", cases: &[order_case] },
    Ukl { sig: &DROP_PARTS, module: "drop", cases: &[drop_parts_case] },
    Ukl { sig: &SEMI_CHANGE, module: "semi", cases: &[semi_change_case] },
    Ukl { sig: &CHANGE_ROWS, module: "semi", cases: &[change_rows_case] },
    Ukl { sig: &REPLACE_ROWS, module: "semi", cases: &[replace_rows_case] },
    Ukl { sig: &DOCS_OF, module: "k0", cases: &[docs_of_case] },
    Ukl { sig: &ROOM_OF, module: "k0", cases: &[room_of_case] },
    Ukl { sig: &NAMED_BLOCKS, module: "k0", cases: &[named_blocks_case] },
    Ukl { sig: &INTERN, module: "intern", cases: &[intern_case] },
    Ukl { sig: &K0_PASS, module: "k0", cases: &[k0_pass_case, k0_refused_case] },
];

/// Each operation's UKL version checked on `device` against its twin, at `TILINGS`, and registered as the device's
/// default.
pub(crate) fn register_on(reg: &mut Registry, device: &dyn Kernels) -> Result<(), String> {
    for &(module, text) in MODULES {
        let params: Vec<String> = VERSIONS.iter().filter(|u| u.module == module && u.sig.params.size > 0).map(|u| u.sig.params.ukl()).collect();
        let mut decls: Vec<String> = Vec::new();
        for p in params {
            if !decls.contains(&p) {
                decls.push(p);
            }
        }
        let source: &'static str = Box::leak(format!("{}\n{text}", decls.join("\n")).into_boxed_str());
        for u in VERSIONS.iter().filter(|u| u.module == module) {
            let op = reg.find(u.sig).ok_or_else(|| format!("{}: not declared", u.sig.name))?;
            let twin = reg.reference(op).ok_or_else(|| format!("{}: no twin", u.sig.name))?;
            let name: &'static str = Box::leak(format!("{} ukl", u.sig.name).into_boxed_str());
            let v = crate::op::Version { name, imp: Impl::Ukl { source, entry: u.sig.name }, devices: EVERY_BACKEND, status: Status::Default, ..*twin };
            let mut checked = None;
            for case in u.cases {
                checked = Some(reg.check_on(&v, device, case, &TILINGS)?);
            }
            let checked = checked.ok_or_else(|| format!("{}: no case", u.sig.name))?;
            reg.register(v, checked)?;
        }
    }
    Ok(())
}

/// A case's params' bytes.
fn params<P: Params>(p: P) -> Vec<u8> {
    let mut b = vec![0; P::SIZE];
    p.write(&mut b);
    b
}

/// A cache committed at state `0xd1`, opened for another (`0xd2`): reset to ∅, then marked open.
fn cache_open_case(dir: &mut Directory) -> Case {
    let cpu = Runner::new().expect("the CPU's runner");
    let c = OpCache::new(dir);
    c.open(&cpu, dir, 0).and_then(|_| c.commit(&cpu, dir, 0xd1)).expect("a committed cache");
    Case { slots: vec![c.0], params: params(CacheP { state: 0xd2 }) }
}

/// A cache opened by a run, committed at the state it made (`0xd1`).
fn cache_commit_case(dir: &mut Directory) -> Case {
    let cpu = Runner::new().expect("the CPU's runner");
    let c = OpCache::new(dir);
    c.open(&cpu, dir, 0).expect("an open cache");
    Case { slots: vec![c.0], params: params(CacheP { state: 0xd1 }) }
}

/// A layer of 150 rows (three fences) ordered by its second column then its first, into a cache holding another
/// layer's order and too small for this one: it grows, and the order is written once.
fn order_case(dir: &mut Directory) -> Case {
    let cpu = Runner::new().expect("the CPU's runner");
    let mut t = Trace::new(&Shape::run(vec![ColType::I32, ColType::I64], vec![0, 1], Some(0))).expect("a trace").with_roll_up(RollUp::Never);
    let rows: Vec<(Vec<i64>, i32)> = (0..150).map(|i| (vec![i, (i * 37) % 11], 1)).collect();
    let layer = push_layer(&cpu, dir, &mut t, &rows).expect("a layer");
    let c = OpCache(dir.make_capped(kind::CACHE, cache_twin::CACHE_BYTES, cache_twin::ENTRIES_AT + 512, 1 << 20));
    c.open(&cpu, dir, 0).expect("an open cache");
    c.order(&cpu, dir, layer, &[0]).expect("another order");
    Case { slots: vec![layer, c.0], params: params(cache_twin::order_params(layer.0, &[1, 0])) }
}

/// A trace of two layers partitioned on its first column, the second killing a row of the first, its rows in
/// partitions 3 and 1 (listed out of order) dropped with sign -1: three rows, partition 1's then 3's, the kill skipped.
fn drop_parts_case(dir: &mut Directory) -> Case {
    let cpu = Runner::new().expect("the CPU's runner");
    let mut t = Trace::new(&Shape::run(vec![ColType::I32, ColType::I64], vec![0, 1], Some(0))).expect("a trace").with_roll_up(RollUp::Never);
    push_layer(&cpu, dir, &mut t, &[(vec![1, 10], 1), (vec![1, 11], 1), (vec![2, 20], 1), (vec![3, 30], 1)]).expect("a layer");
    let head = push_layer(&cpu, dir, &mut t, &[(vec![1, 11], -1), (vec![3, 31], 1), (vec![4, 40], 1)]).expect("a layer");
    let runs = t.open(dir, 16);
    let parts = Parts::write(dir, 2, |put| [3, 1].into_iter().for_each(put)).expect("a parts block");
    Case { slots: vec![head, parts.id(), runs], params: params(DropP { head: u64::from(head.0), diff: -1 }) }
}

/// A trace of `(doc, value)` rows partitioned by doc and one of wants `(port, doc)`, three layers each: their heads.
fn semi_traces(cpu: &Runner, dir: &mut Directory) -> (Trace, Vec<crate::dir::BlockId>, Vec<crate::dir::BlockId>) {
    let mut a = Trace::new(&Shape::run(vec![ColType::I32, ColType::I64], vec![0, 1], Some(0))).expect("a trace").with_roll_up(RollUp::Never);
    let mut b = Trace::new(&crate::contract::want_shape()).expect("a trace").with_roll_up(RollUp::Never);
    let r = |d: i64, v: i64| (vec![d, v], 1);
    let k = |d: i64, v: i64| (vec![d, v], -1);
    let al = [
        vec![r(1, 10), r(2, 20), r(3, 30), r(4, 40)],
        // Doc 1 edited, doc 3 gone, doc 5 new.
        vec![k(1, 10), r(1, 11), k(3, 30), r(5, 50)],
        // Doc 2 edited, doc 3 back.
        vec![k(2, 20), r(2, 21), r(3, 31)],
    ];
    let bl = [
        vec![r(0, 1), r(0, 2), r(0, 3)],
        // 2 no longer wanted, 4 and 5 newly.
        vec![k(0, 2), r(0, 4), r(0, 5)],
        // 2 wanted again, 1 not.
        vec![r(0, 2), k(0, 1)],
    ];
    let (mut ah, mut bh) = (vec![], vec![]);
    for (x, y) in al.iter().zip(&bl) {
        ah.push(push_layer(cpu, dir, &mut a, x).expect("a layer"));
        bh.push(push_layer(cpu, dir, &mut b, y).expect("a layer"));
    }
    (a, ah, bh)
}

/// `semi_change`'s params: `A` and `B` each from their first layer's state to their last's.
fn semi_p(ah: &[crate::dir::BlockId], bh: Option<&[crate::dir::BlockId]>) -> Vec<u8> {
    let h = |b: crate::dir::BlockId| u64::from(b.0);
    let none = rerail_delta_twins::semi::NONE;
    params(SemiP {
        a_before: h(ah[0]),
        a_after: h(ah[2]),
        b_before: bh.map_or(none, |b| h(b[0])),
        b_after: bh.map_or(none, |b| h(b[2])),
        a_arity: 2,
        b_arity: bh.map_or(0, |_| 2),
        a_col: 0,
        b_col: bh.map_or(0, |_| 1),
        neg: 0,
        parts_sign: -1,
    })
}

/// `A ⋉ B` between their first and last states: rows gone and added, a kept row whose doc came to be wanted (4)
/// added; the docs of the rows gone; the wants' change (4, 5 held, 1 not).
fn semi_change_case(dir: &mut Directory) -> Case {
    let cpu = Runner::new().expect("the CPU's runner");
    let (a, ah, bh) = semi_traces(&cpu, dir);
    let asks = Trace::new(&crate::contract::asks_shape()).expect("a trace");
    let (rows, parts, keys) = (a.open(dir, 64), Parts::open(dir, 16).expect("a parts block"), asks.open(dir, 16));
    Case { slots: vec![ah[2], bh[2], rows, parts, keys], params: semi_p(&ah, Some(&bh)) }
}

/// `A`'s change alone between its first and last states.
fn change_rows_case(dir: &mut Directory) -> Case {
    let cpu = Runner::new().expect("the CPU's runner");
    let (a, ah, _) = semi_traces(&cpu, dir);
    let (rows, parts) = (a.open(dir, 64), Parts::open(dir, 16).expect("a parts block"));
    Case { slots: vec![ah[2], rows, parts], params: semi_p(&ah, None) }
}

/// A state of two keys' rows given three: one as it is (nothing), one changed (its old row goes, it comes), one new.
fn replace_rows_case(dir: &mut Directory) -> Case {
    let cpu = Runner::new().expect("the CPU's runner");
    let mut t = Trace::new(&Shape::run(vec![ColType::I32, ColType::I64, ColType::I64], vec![0, 1, 2], Some(0))).expect("a trace").with_roll_up(RollUp::Never);
    let head = push_layer(&cpu, dir, &mut t, &[(vec![1, 10, 100], 1), (vec![2, 20, 200], 1)]).expect("a layer");
    let given = t.open(dir, 3);
    let b = dir.block(given).expect("just made");
    let pin = rerail_blocks::region::pin_block(&b, "given rows", true).expect("a write pin");
    for r in [[1, 10, 100], [2, 21, 200], [3, 30, 300]] {
        rerail_blocks::layer::RunsW::new(&pin).push(&r, 1).expect("room for the given rows");
    }
    drop(pin);
    let runs = t.open(dir, 8);
    Case { slots: vec![head, given, runs], params: params(ReplaceP { head: u64::from(head.0), arity: 3, key: 1 }) }
}

/// A document's rows for the K0 cases: its module, then its source's, tape's and symbols' blob names, four words
/// each (`(d, 1, 0, 0)`, `(d, 2, 0, 0)`, `(d, 3, 0, 0)`).
const DOC_ROW: crate::k0::RowCols = crate::k0::RowCols { module: 0, source: 1, tape: 5, syms: 9 };

fn doc_row(d: i64) -> Vec<i64> {
    let mut r = vec![d];
    for k in 1..=3 {
        r.extend([d, k, 0, 0]);
    }
    r
}

/// A trace of document rows (`doc_row`'s thirteen columns).
fn doc_trace() -> Trace {
    Trace::new(&Shape::run(vec![ColType::I64; 13], (0..13).collect(), None)).expect("a trace").with_roll_up(RollUp::Never)
}

/// The blobs' rows: every blob of documents 1, 2 and 4, document 3's source alone.
fn blob_rows(cpu: &Runner, dir: &mut Directory) -> crate::dir::BlockId {
    let mut t = Trace::new(&crate::files::blobs_shape()).expect("a trace");
    let mut rows = vec![];
    for (d, kinds) in [(1, 1..=3), (2, 1..=3), (3, 1..=1), (4, 1..=3)] {
        for k in kinds {
            rows.push((vec![d, k, 0, 0, 100 + 10 * d + k, 8 * k, 64 + d], 1));
        }
    }
    push_layer(cpu, dir, &mut t, &rows).expect("the blobs' rows")
}

/// A docs block opened, with room for `n` documents, `fill` writing some.
fn docs_block(dir: &mut Directory, n: usize, fill: &[DocAt]) -> crate::dir::BlockId {
    let id = dir.make(kind::CONTROL, k0_twin::docs_bytes(n));
    let b = dir.block(id).expect("just made");
    let mut w = pin_block(&b, "a docs block", true).expect("a write pin");
    let bytes = w.bytes_mut().expect("a write pin");
    k0_twin::open_docs(bytes);
    for (i, d) in fill.iter().enumerate() {
        k0_twin::put_doc(bytes, i, d);
    }
    k0_twin::seal_docs(bytes, fill.len());
    id
}

/// A selection reading documents 1, 2 and 3 (3's tape in no blobs' row: missing) and retracting 4: the docs of 1 and
/// 2, and the blocks they name.
fn docs_of_case(dir: &mut Directory) -> Case {
    let cpu = Runner::new().expect("the CPU's runner");
    let blobs = blob_rows(&cpu, dir);
    let rows = doc_trace().open(dir, 4);
    let b = dir.block(rows).expect("just made");
    let pin = pin_block(&b, "the selection", true).expect("a write pin");
    for (d, sign) in [(1, 1), (2, 1), (3, 1), (4, -1)] {
        rerail_blocks::layer::RunsW::new(&pin).push(&doc_row(d), sign).expect("room for the selection");
    }
    drop(pin);
    let (docs, list) = (docs_block(dir, 4, &[]), Parts::open(dir, 12).expect("a parts block"));
    let c = DOC_ROW;
    let p = DocsOfP { blobs: u64::from(blobs.0), module: c.module as u32, source: c.source as u32, tape: c.tape as u32, syms: c.syms as u32 };
    Case { slots: vec![rows, blobs, docs, list], params: params(p) }
}

/// Two documents interned in turn into one atoms block, the second's launch the case: names the first made (found),
/// new ones (made in node order), an operator, strings with escapes (a surrogate pair; one read as written), a comment.
fn intern_case(dir: &mut Directory) -> Case {
    let cpu = Runner::new().expect("the CPU's runner");
    let mut blobs = crate::files::BlobRows::new();
    let atoms = crate::sym::Atoms::new(dir);
    let doc = |dir: &mut Directory, blobs: &mut crate::files::BlobRows, src: &str| {
        let ids = crate::files::doc_of(&cpu, dir, blobs, src).expect("a document");
        let place = |id| blobs.get(dir, id).unwrap().expect("a blob housed");
        (place(&ids.source), place(&ids.tape))
    };
    let (s, t) = doc(dir, &mut blobs, "p(x) :- e(x, \"s\").\n");
    crate::intern::intern(&cpu, dir, &mut blobs, atoms, s, t, "a.rerail", "a").expect("the first interned");
    let (s, t) = doc(dir, &mut blobs, "// a comment\nq(y, \"t\\u{e9}\\n\\ud83d\\ude00\", \"\\q\") :- p(y), e(y, \"s\"), y != 3.\n");
    let (_, slots, p, _) = crate::intern::launch(dir, atoms, s, t, "b.rerail", "b").expect("the second's launch");
    Case { slots: slots.to_vec(), params: p.to_vec() }
}

/// Documents `srcs` read, interned and listed for `k0_pass` (module `k + 1` each), its launch over them with control
/// at room for two statements (`count` grows it) and a cache holding document 2's sizes and a gone document's.
fn k0_launch(dir: &mut Directory, srcs: &[&str]) -> Case {
    let cpu = Runner::new().expect("the CPU's runner");
    let mut blobs = crate::files::BlobRows::new();
    let atoms = crate::sym::Atoms::new(dir);
    let mut docs = Vec::new();
    for (k, src) in srcs.iter().enumerate() {
        let ids = crate::files::doc_of(&cpu, dir, &mut blobs, src).expect("a document");
        let place = |dir: &Directory, blobs: &crate::files::BlobRows, id| blobs.get(dir, id).unwrap().expect("a blob housed");
        let (s, t) = (place(dir, &blobs, &ids.source), place(dir, &blobs, &ids.tape));
        let path = format!("m{k}.rerail");
        let syms = crate::intern::intern(&cpu, dir, &mut blobs, atoms, s, t, &path, &path[..2]).expect("interned");
        docs.push(crate::k0::K0Doc { module: k as i64 + 1, src: s, tape: t, syms: place(dir, &blobs, &syms).block });
    }
    let d = crate::k0::DocsIn::of(dir, &docs).expect("the docs");
    let control = dir.make_capped(kind::CONTROL, k0_twin::control_bytes(2), 0, 1 << 20);
    {
        let b = dir.block(control).expect("just made");
        k0_twin::init_control(pin_block(&b, "control", true).expect("a write pin").bytes_mut().expect("a write pin"));
    }
    let cache = OpCache::new(dir);
    {
        let b = dir.block(cache.0).expect("just made");
        let mut w = pin_block(&b, "the cache", true).expect("a write pin");
        let bytes = w.bytes_mut().expect("a write pin");
        set_word::<u64>(bytes, cache_twin::USED_AT, 0);
        for d in [2u32, 9] {
            let at = cache_twin::ENTRIES_AT + cache_twin::used(bytes);
            cache_twin::put_sizes(bytes, at, d, &[1; k0_twin::R], true);
        }
    }
    let gone = Parts::write(dir, 1, |put| put(9)).expect("the gone");
    let mut slots = vec![control; k0_twin::LANG + 1];
    (slots[k0_twin::DOCS], slots[k0_twin::ATOMS], slots[k0_twin::GONE]) = (d.first, atoms.0, gone.id());
    for k in 0..k0_twin::R {
        slots[k0_twin::RUNS + k] = Trace::new(&crate::kform::shape(k)).expect("a trace").open(dir, 64);
    }
    (slots[k0_twin::CACHE], slots[k0_twin::LANG]) = (cache.0, crate::intern::language(dir).expect("the language"));
    Case { slots, params: params(rerail_delta_twins::k0::K0P { docs: u64::from(d.first.0), diff: -1 }) }
}

/// Three documents of every K0 form: rules and facts, atoms, negations, comparisons, and
/// terms of each kind (strings with escapes, numbers to an i64's end, wildcards), a comment among them.
fn k0_pass_case(dir: &mut Directory) -> Case {
    k0_launch(dir, &[
        "e(1, 2).\np(x) :- e(x, _), !q(x).\n",
        "// a comment\nq(y, \"t\\u{e9}\") :- e(y, 0), y != 9223372036854775807, r(\"a\\\"b\").\nr(\"s\").\n",
        "g(z) :- e(z, _).\nf(a, b) :- e(a, b), a != b.\n",
    ])
}

/// A run with a document past K0 between two that are: refused, the first refusal by document and node order named.
fn k0_refused_case(dir: &mut Directory) -> Case {
    k0_launch(dir, &["e(1, 2).\n", "p(x) :- e(x, _).\nn(c) :- c = count : { e(_, _) }.\n", "q(1).\n"])
}

/// A cache holding the sizes of documents 1, 2 and 5; 5 and 7 gone, 1 and 3 read: the room 1's and 5's take.
fn room_of_case(dir: &mut Directory) -> Case {
    let cpu = Runner::new().expect("the CPU's runner");
    let c = OpCache(dir.make(kind::CACHE, cache_twin::CACHE_BYTES + 256));
    c.open(&cpu, dir, 0).and_then(|_| c.commit(&cpu, dir, 0xd1)).expect("a committed cache");
    {
        let b = dir.block(c.0).expect("just made");
        let mut w = pin_block(&b, "the cache", true).expect("a write pin");
        let bytes = w.bytes_mut().expect("a write pin");
        for d in [1u32, 2, 5] {
            let at = cache_twin::ENTRIES_AT + cache_twin::used(bytes);
            cache_twin::put_sizes(bytes, at, d, &core::array::from_fn::<u32, { k0_twin::R }, _>(|r| d * 10 + r as u32), true);
        }
        set_word::<u64>(bytes, BODY_AT, (cache_twin::CACHE_BYTES + cache_twin::used(bytes)) as u64);
    }
    let gone = Parts::write(dir, 2, |put| [5, 7].into_iter().for_each(put)).expect("a parts block");
    let doc = |module| DocAt { module, source: 0, tape: 0, syms: 0, src: (0, 0), at: (0, 0) };
    let docs = docs_block(dir, 2, &[doc(1), doc(3)]);
    let room = dir.make(kind::CONTROL, 8 * k0_twin::R);
    Case { slots: vec![c.0, gone.id(), docs, room], params: vec![] }
}

/// A document set of 1, 2, 3 and 4, 2 gone since: the blocks of 1's and 4's blobs and 3's source; 3 missing its tape.
fn named_blocks_case(dir: &mut Directory) -> Case {
    let cpu = Runner::new().expect("the CPU's runner");
    let blobs = blob_rows(&cpu, dir);
    let mut t = doc_trace();
    push_layer(&cpu, dir, &mut t, &[(doc_row(1), 1), (doc_row(2), 1), (doc_row(3), 1)]).expect("a layer");
    let set = push_layer(&cpu, dir, &mut t, &[(doc_row(2), -1), (doc_row(4), 1)]).expect("a layer");
    let (blocks, missing) = (Parts::open(dir, 12).expect("a parts block"), Parts::open(dir, 4).expect("a parts block"));
    let c = DOC_ROW;
    let p = NamedP { set: u64::from(set.0), blobs: u64::from(blobs.0), arity: 13, module: c.module as u32, source: c.source as u32, tape: c.tape as u32, syms: c.syms as u32, pad: 0 };
    Case { slots: vec![set, blobs, blocks, missing], params: params(p) }
}

/// The block layouts the kernels read, by name: the twins' own constants.
pub fn layout() -> Vec<(&'static str, usize)> {
    vec![
        ("BODY_AT", BODY_AT),
        ("MAX_SLOTS", rerail_delta_twins::MAX_SLOTS),
        ("CACHE_BYTES", cache_twin::CACHE_BYTES),
        ("CACHE_STATE_AT", cache_twin::STATE_AT),
        ("CACHE_OPEN_AT", cache_twin::OPEN_AT),
        ("CACHE_HELD_AT", cache_twin::HELD_AT),
        ("CACHE_USED_AT", cache_twin::USED_AT),
        // `rerail_delta_core::runs`, `::layer`, `::relation` and `::trace`.
        ("NONE", NONE as usize),
        ("MAX_LAYERS", MAX_LAYERS),
        ("SHAPE_ARITY_AT", offset_of!(ShapeW, arity)),
        ("SHAPE_PARTITION_AT", offset_of!(ShapeW, partition)),
        ("SHAPE_ORDER_AT", offset_of!(ShapeW, order)),
        ("RUNS_CAP_AT", BODY_AT + offset_of!(RunsHead, cap)),
        ("RUNS_CLAIMED_AT", BODY_AT + offset_of!(RunsHead, claimed)),
        ("RUNS_NEEDED_AT", BODY_AT + offset_of!(RunsHead, needed)),
        ("RUNS_TYPES_AT", BODY_AT + offset_of!(RunsHead, shape) + offset_of!(ShapeW, types)),
        ("RUNS_COL_AT", BODY_AT + offset_of!(RunsHead, col_at)),
        ("RUNS_DIFF_AT", BODY_AT + offset_of!(RunsHead, diff_at)),
        ("LAYER_ROWS_AT", BODY_AT + offset_of!(LayerHead, rows)),
        ("LAYER_PARTS_AT", BODY_AT + offset_of!(LayerHead, parts)),
        ("LAYER_GONE_AT", BODY_AT + offset_of!(LayerHead, gone)),
        ("LAYER_SHAPE_AT", BODY_AT + offset_of!(LayerHead, shape)),
        ("REL_FLAGS_AT", offset_of!(Relation, flags)),
        ("REL_BASE_ROWS_AT", offset_of!(Relation, base_rows)),
        ("REL_BASE_BLOCK_AT", offset_of!(Relation, base_block)),
        ("REL_FAR_BASE", FAR_BASE as usize),
        // `rerail_delta_twins::drop`'s parts block.
        ("PARTS_N_AT", drop_twin::N_AT),
        ("PARTS_AT", drop_twin::PARTS_AT),
        // `rerail_delta_twins::cache`'s entries.
        ("CACHE_ENTRIES_AT", cache_twin::ENTRIES_AT),
        ("CACHE_ENTRY_BYTES", cache_twin::ENTRY_BYTES),
        ("FENCE", cache_twin::FENCE),
        // `rerail_delta_twins::k0`'s docs block and the relations a pass writes.
        ("DOCS_LEN_AT", k0_twin::DOCS_LEN_AT),
        ("DOCS_N_AT", k0_twin::DOCS_N_AT),
        ("DOCS_MISSING_AT", k0_twin::DOCS_MISSING_AT),
        ("DOCS_AT", k0_twin::DOCS_AT),
        ("DOC_WORDS", k0_twin::DOC_WORDS),
        ("K0_RELATIONS", k0_twin::R),
        // An atoms block's root (`rerail_blocks::atoms`), at the block header's root word.
        ("ATOMS_ROOT_WORD", offset_of!(rerail_blocks::block::Header, root)),
        ("ATOMS_FIRST_AT", offset_of!(AtomsRoot, first)),
        ("ATOMS_CAP_AT", offset_of!(AtomsRoot, cap)),
        ("ATOMS_BYTES_CAP_AT", offset_of!(AtomsRoot, bytes_cap)),
        ("ATOMS_COUNT_AT", offset_of!(AtomsRoot, count)),
        ("ATOMS_BYTES_LEN_AT", offset_of!(AtomsRoot, bytes_len)),
        ("ATOMS_MASK_AT", offset_of!(AtomsRoot, mask)),
        ("ATOMS_PLACED_AT", offset_of!(AtomsRoot, placed)),
        ("ATOMS_ENDS_AT", offset_of!(AtomsRoot, ends)),
        ("ATOMS_HASHES_AT", offset_of!(AtomsRoot, hashes)),
        ("ATOMS_TABLE_AT", offset_of!(AtomsRoot, table)),
        ("ATOMS_STRINGS_AT", offset_of!(AtomsRoot, strings)),
        // `rerail_delta_twins::intern`'s symbols block and the language's table, and the kinds it reads names by.
        ("SYMS_N_AT", intern_twin::N_AT),
        ("SYMS_PATH_SYM_AT", intern_twin::PATH_SYM_AT),
        ("SYMS_MODULE_SYM_AT", intern_twin::MODULE_SYM_AT),
        ("SYMS_PATH_LEN_AT", intern_twin::PATH_LEN_AT),
        ("SYMS_MODULE_LEN_AT", intern_twin::MODULE_LEN_AT),
        ("SYMS_NAMES_AT", intern_twin::NAMES_AT),
        ("SYM_NEW", intern_twin::NEW as usize),
        ("SYM_FIND", intern_twin::FIND as usize),
        ("LANG_CODES_N_AT", intern_twin::LANG_CODES_N_AT),
        ("LANG_NAMES_N_AT", intern_twin::LANG_NAMES_N_AT),
        ("LANG_CODES_AT", intern_twin::LANG_CODES_AT),
        ("LANG_EXTRA", intern_twin::LANG_EXTRA as usize),
        ("KIND_IDENTIFIER", Kind::Identifier as usize),
        ("KIND_VARIABLE", Kind::Variable as usize),
        ("KIND_STRING", Kind::String as usize),
        ("FIRST_ANONYMOUS", rerail_rr_syntax::tape::FIRST_ANONYMOUS as usize),
        ("VIEW_WORDS", intern_twin::VIEW_WORDS),
        ("SYMS_VIEW_AT", intern_twin::VIEW_AT),
        ("LANG_FIELD", intern_twin::LANG_FIELD as usize),
        ("KIND_NUMBER", Kind::Number as usize),
        ("KIND_WILDCARD", Kind::Wildcard as usize),
        ("KIND_CLAUSE", Kind::Clause as usize),
        ("KIND_CONJUNCTION", Kind::Conjunction as usize),
        ("KIND_ATOM", Kind::Atom as usize),
        ("KIND_NEGATION", Kind::Negation as usize),
        ("KIND_COMPARISON", Kind::Comparison as usize),
        ("FIELD_RELATION", field("relation")),
        ("FIELD_HEAD", field("head")),
        ("FIELD_MODAL", field("modal")),
        ("FIELD_EDIT", field("edit")),
        ("FIELD_ARGUMENTS", field("arguments")),
        ("FIELD_BODY", field("body")),
        ("FIELD_OPERAND", field("operand")),
        ("FIELD_OPERATOR", field("operator")),
        ("FIELD_LEFT", field("left")),
        ("FIELD_RIGHT", field("right")),
        // `rerail_delta_twins::k0`'s slots and control.
        ("K0_DOCS", k0_twin::DOCS),
        ("K0_ATOMS", k0_twin::ATOMS),
        ("K0_CONTROL", k0_twin::CONTROL),
        ("K0_GONE", k0_twin::GONE),
        ("K0_RUNS", k0_twin::RUNS),
        ("K0_CACHE", k0_twin::CACHE),
        ("K0_LANG", k0_twin::LANG),
        ("K0_REFUSED_NODE_AT", k0_twin::REFUSED_NODE),
        ("K0_REFUSED_WHY_AT", k0_twin::REFUSED_WHY),
        ("K0_REFUSED_DOC_AT", k0_twin::REFUSED_DOC),
        ("K0_PLACED_AT", k0_twin::PLACED),
        ("K0_REFUSED_MODULE_AT", k0_twin::REFUSED_MODULE),
        ("K0_ENDS_AT", k0_twin::ENDS),
        ("K0_PLACES_AT", k0_twin::PLACES),
        ("K0_UNCLAIMED", k0_twin::UNCLAIMED as usize),
        ("CACHE_DEAD", cache_twin::DEAD as usize),
    ]
}

/// The `.rerail` field named `f`'s number.
fn field(f: &str) -> usize {
    rerail_rr_syntax::tape::FIELD_NAMES.iter().position(|n| *n == f).expect("an rr field")
}

/// `<ukl/delta/layout.h>`'s text: `layout`'s constants.
pub fn layout_h() -> String {
    let mut s = String::from(
        "// Generated by native/crates/delta_exec/src/ukl.rs (`layout_h`); do not edit. The block layouts the delta operations'\n\
         // kernels read, each the twins' own constant (decision 0686). `RERAIL_WRITE_LAYOUT=1 cargo test -p rerail-delta-exec\n\
         // --lib ukl::tests::the_layout_header_is_the_twins` writes it again.\n#pragma once\n",
    );
    for (name, v) in layout() {
        s += &format!("#define {name} {v}u\n");
    }
    // The signed ones, as their bits: `k0`'s no document.
    for (name, v) in [("NO_DOC", k0_twin::NO_DOC)] {
        s += &format!("#define {name} ((int64_t){:#x}ull)\n", v as u64);
    }
    s
}

#[cfg(test)]
#[path = "ukl_tests.rs"]
mod tests;
