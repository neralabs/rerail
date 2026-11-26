//! Each kernel's three versions on the same inputs: the same answers.

use crate::{Forest, NearestArgs, scalar};

fn args(f: &Forest) -> NearestArgs {
    NearestArgs { nodes: f.nodes, cells: f.cells(), pad0: 0, pad1: 0 }
}

/// The forests the versions run on: rerail-ukl-vec's test's (4 sets of a binary tree of 16 nodes), and random ones
/// of sizes that leave chunks and groups part full, shallow and deep.
fn forests() -> Vec<Forest> {
    let mut fs = vec![Forest::new(16, 4, |i| (i > 0).then(|| (i - 1) / 2), |s, i| i % (s + 2) == 0)];
    for (k, &(nodes, sets, sparse, deep)) in [(1, 3, 2, false), (7, 3, 3, false), (33, 5, 4, true), (100, 3, 9, false), (1000, 4, 50, true), (4099, 2, 7, false)].iter().enumerate() {
        fs.push(Forest::random(nodes, sets, sparse, deep, k as u64 + 1));
    }
    fs
}

/// The scalar reference's answers are the forests' (the kernel's meaning, before any version is compared with it).
#[test]
fn the_scalar_nearest_is_each_cells_nearest_ancestor_in_its_set() {
    for f in forests() {
        let mut state = f.start.clone();
        let most = scalar::nearest_jump(&mut state, f.nodes, f.cells());
        assert_eq!(state, f.want, "{} nodes, {} sets", f.nodes, f.sets);
        assert!(most <= f.nodes);
    }
}

/// The hand-vectorized kernel's answers are the scalar one's.
#[cfg(target_arch = "aarch64")]
#[test]
fn the_hand_vectorized_nearest_is_the_scalar_one() {
    for f in forests() {
        let mut state = f.start.clone();
        let most = unsafe { crate::hand::nearest_jump(state.as_mut_ptr(), f.nodes, f.cells()) };
        assert_eq!(state, f.want, "{} nodes, {} sets", f.nodes, f.sets);
        assert!(most <= f.nodes);
    }
}

/// The hand-vectorized kernel with its threads in strided order: the scalar answers, at groups that leave cells
/// past the last whole one.
#[cfg(target_arch = "aarch64")]
#[test]
fn the_strided_nearest_is_the_scalar_one() {
    for f in forests() {
        for block in [8, 64, 256] {
            let mut state = f.start.clone();
            let most = unsafe { crate::hand::nearest_jump_strided(state.as_mut_ptr(), f.nodes, f.cells(), block) };
            assert_eq!(state, f.want, "{} nodes, {} sets, groups of {block}", f.nodes, f.sets);
            assert!(most <= f.nodes);
        }
    }
}

/// rerail-ukl-vec's Rust of `nearest_jump` at 8 lanes: the scalar kernel's answers, at groups of 20, 32, 36, 64 and
/// 256 threads (the last groups part full), and a thread's steps counted (`stats`) when any took one.
#[cfg(target_arch = "aarch64")]
#[test]
fn the_kernels_rust_nearest_is_the_scalar_one() {
    for f in forests() {
        // Groups of 20 and 36 threads end in part of a chunk; the cells' end is a chunk's middle in most forests.
        for (block, k) in [20, 32, 36, 64, 256].into_iter().flat_map(|b| [(b, crate::nearest_jump_w8::nearest_jump_w8 as crate::Kernel), (b, crate::nearest_jump_w32::nearest_jump_w32)]) {
            let mut state = f.start.clone();
            let most = unsafe { crate::nearest_run(k, &mut state, args(&f), block) };
            assert_eq!(state, f.want, "{} nodes, {} sets, groups of {block}", f.nodes, f.sets);
            let moved = f.start.iter().zip(&f.want).any(|(a, b)| a != b);
            assert!(most <= f.nodes && (most > 0) == moved, "{} nodes, {} sets: {most} steps", f.nodes, f.sets);
        }
    }
}

/// rerail-ukl-vec's Rust of `mix_lanes` at 8 lanes: the scalar answers, on values whose lanes agree (chunks of
/// only odd, only small, none zero, one loop bound) and part (random, some zero, some past the likely bound), at
/// groups that end in part of a chunk.
#[cfg(target_arch = "aarch64")]
#[test]
fn the_kernels_rust_mix_is_the_scalar_one() {
    let mut r = 0x9E37_79B9u32;
    let mut next = move || {
        r ^= r << 13;
        r ^= r >> 17;
        r ^= r << 5;
        r
    };
    let inputs: Vec<Vec<u32>> = vec![
        vec![],
        vec![0],
        (0..37).map(|i| 2 * i + 1).collect(),
        (0..64).map(|_| 0x13).collect(),
        (0..100).map(|i| i % 3).collect(),
        (0..1000).map(|_| next()).collect(),
        (0..333).map(|i| if i % 7 == 0 { 0 } else if i % 5 == 0 { 0xF000_0000 + i } else { next() & 0xFF }).collect(),
    ];
    for input in &inputs {
        for rounds in [0, 1, 5] {
            let want: Vec<u32> = input.iter().map(|&x| scalar::mix(x, rounds)).collect();
            for block in [8, 20, 32, 36, 256] {
                for (w, k) in [(8, crate::mix_lanes_w8::mix_lanes_w8 as crate::Kernel), (32, crate::mix_lanes_w32::mix_lanes_w32)] {
                    let got = unsafe { crate::mix_run(k, input, rounds, block) };
                    assert_eq!(got, want, "{} values, {rounds} rounds, groups of {block}, {w} lanes", input.len());
                }
            }
        }
    }
}

/// Format 16's parent read, the host's (the reference): on every test document, a node's parent is before it and
/// holds it in its subtree, and only the root has none.
#[test]
fn the_hosts_parent_read_is_each_nodes_parent() {
    let lang = crate::parent::json_lang();
    // The paths the read takes: a slot of its array's width, an escaped one (its side list's), and a walk back over
    // a leaf's siblings (a parent not the node before).
    let (mut escaped, mut walked) = (0, 0);
    for d in crate::parent::documents(&lang) {
        let want = crate::parent::scalar(&d);
        assert!(!want.is_empty(), "{}: no nodes", d.name);
        let w = d.view.widths & 15;
        let ones = if w >= 8 { u64::MAX } else { (1u64 << (8 * w)) - 1 };
        escaped += (0..d.nodes()).filter(|&i| rerail_tape_format::reads::f16::slot(&d.bytes, &d.view, i) >= ones).count();
        walked += want.iter().enumerate().filter(|&(i, &p)| i > 0 && p as usize + 1 != i).count();
        for (i, &p) in want.iter().enumerate() {
            if i == 0 {
                assert_eq!(p, crate::parent::NONE, "{}: the root's parent", d.name);
                continue;
            }
            let size = rerail_tape_format::reads::f16::size(&d.bytes, &d.view, p);
            assert!((p as usize) < i && i < p as usize + size as usize, "{}: node {i}'s parent {p} (size {size})", d.name);
        }
    }
    assert!(escaped > 0 && walked > 1000, "{escaped} escaped slots, {walked} walks over siblings");
}

/// rerail-ukl-vec's Rust of `f16_parent` at 8 lanes: the host's parents, on every node of every test document, at
/// groups that end in part of a chunk.
#[cfg(target_arch = "aarch64")]
#[test]
fn the_kernels_rust_parent_is_the_hosts() {
    let lang = crate::parent::json_lang();
    for d in crate::parent::documents(&lang) {
        let want = crate::parent::scalar(&d);
        let steps = crate::parent::scalar_steps(&d);
        let widths: [(u32, [crate::Kernel; 4]); 2] = [
            (8, [crate::parent_group_w8::parent_group_w8, crate::parent_back_w8::parent_back_w8, crate::parent_step_w8::parent_step_w8, crate::parent_link_w8::parent_link_w8]),
            (32, [crate::parent_group_w32::parent_group_w32, crate::parent_back_w32::parent_back_w32, crate::parent_step_w32::parent_step_w32, crate::parent_link_w32::parent_link_w32]),
        ];
        for block in [8, 20, 32, 36, 256] {
            for (w, [group, back, step, link]) in widths {
                let group = unsafe { crate::parent::run(group, &d, block) };
                let back = unsafe { crate::parent::run_back(back, &d, block) };
                let made = unsafe { crate::parent::run_step(step, &d, block) };
                assert!(made == steps, "step: {}, groups of {block}, {w} lanes: not the host's steps", d.name);
                let two = unsafe { crate::parent::run_two(step, link, &d, block) }.unwrap_or_else(|e| panic!("two: {}, groups of {block}, {w} lanes: {e}", d.name));
                for (name, got) in [("two", two), ("group", group), ("back", back)] {
                    if let Some(i) = (0..want.len()).find(|&i| got[i] != want[i]) {
                        panic!("{name}: {} ({} nodes), groups of {block}, {w} lanes: node {i}'s parent {} (the host's {})", d.name, want.len(), got[i], want[i]);
                    }
                }
            }
        }
    }
}

/// A chain of links past 7 (no tape format 16's writer made) is the tape's corruption, reported, not a parent: a
/// chain of 7 is a parent, one of 8 the error naming its node, at groups that end in part of a chunk.
#[cfg(target_arch = "aarch64")]
#[test]
fn a_chain_past_seven_links_is_a_corrupt_tape() {
    use crate::parent::{Corrupt, NONE, run_link};
    const LINKED: u32 = 1 << 31;
    // Node 0 the root, 1 its child, 2 to 9 each linked to the node before: node 8 is 7 links from 1, node 9 is 8.
    let steps: Vec<u32> = [NONE, 0].into_iter().chain((2..10).map(|i| LINKED | (i - 1))).collect();
    for block in [8, 20, 256] {
        for link in [crate::parent_link_w8::parent_link_w8 as crate::Kernel, crate::parent_link_w32::parent_link_w32] {
            let ok = unsafe { run_link(link, &steps[..9], block) };
            assert_eq!(ok, Ok(vec![NONE, 0, 0, 0, 0, 0, 0, 0, 0]), "groups of {block}");
            let bad = unsafe { run_link(link, &steps, block) };
            assert_eq!(bad, Err(Corrupt { node: 9 }), "groups of {block}");
        }
    }
}

/// The hand-vectorized NEON reads (`hand::parent_all`, the kernel's walks; `hand::parent_all_scan`, from the answers
/// before) on every node of every document: the host's parents.
#[test]
fn the_hand_vectorized_parent_is_the_hosts() {
    let lang = crate::parent::json_lang();
    for d in crate::parent::documents(&lang) {
        let want = crate::parent::scalar(&d);
        for (name, f) in [("walks", crate::hand::parent_all as unsafe fn(&[u8], &rerail_clif::view::TapeView, &mut [u32])), ("scan", crate::hand::parent_all_scan)] {
            let mut got = vec![0u32; want.len()];
            unsafe { f(&d.bytes, &d.view, &mut got) };
            if let Some(i) = (0..want.len()).find(|&i| got[i] != want[i]) {
                panic!("{name}, {} ({} nodes): node {i}'s parent {} (the host's {})", d.name, want.len(), got[i], want[i]);
            }
        }
    }
}

/// The 8-lane slot reader (`hand::slots_all`: its runs, gathers, and side lists' count search) on every node of every
/// document: the host's slots, every escaped one among them.
#[test]
fn the_hand_vectorized_slot_read_is_the_hosts() {
    let lang = crate::parent::json_lang();
    let mut escaped = 0;
    for d in crate::parent::documents(&lang) {
        let want: Vec<u64> = (0..d.nodes()).map(|i| rerail_tape_format::reads::f16::slot(&d.bytes, &d.view, i)).collect();
        let w = d.view.widths & 15;
        escaped += want.iter().filter(|&&s| w < 8 && s >= (1u64 << (8 * w)) - 1).count();
        let mut got = vec![0u64; want.len()];
        unsafe { crate::hand::slots_all(&d.bytes, &d.view, &mut got) };
        if let Some(i) = (0..want.len()).find(|&i| got[i] != want[i]) {
            panic!("{} ({} nodes): node {i}'s slot {} (the host's {})", d.name, want.len(), got[i], want[i]);
        }
    }
    assert!(escaped > 100, "the documents escape {escaped} slots: too few to test the side lists' search");
}

/// A node's first step (`reads::f16::parent_step`) links it to a node before it at most 7 times in a row (every K-th
/// sibling names the parent): `parent_group`'s 7 hops resolve every node whose links stay in its group.
#[test]
fn every_chain_of_links_is_at_most_seven() {
    use rerail_tape_format::reads::f16::{LINK, parent_step};
    let lang = crate::parent::json_lang();
    let mut longest = 0;
    for d in crate::parent::documents(&lang) {
        for i in 0..d.nodes() {
            let (mut r, mut links) = (parent_step(&d.bytes, &d.view, i), 0);
            while r & LINK != 0 {
                links += 1;
                r = parent_step(&d.bytes, &d.view, r as u32);
            }
            assert!(links <= 7, "{}: node {i}: {links} links", d.name);
            longest = longest.max(links);
        }
    }
    assert_eq!(longest, 7, "the documents hold a chain of the most links");
}

/// rerail-ukl-vec's Rust of `wide_lanes` at 8 lanes (64-bit lanes as their halves): the scalar answers, on values
/// that carry and borrow between the halves, of each sign, equal in either half, zero, and shifted by every count
/// that ends below, at and past 32; at groups that end in part of a chunk.
#[cfg(target_arch = "aarch64")]
#[test]
fn the_kernels_rust_wide_is_the_scalar_one() {
    let mut r = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        r ^= r << 13;
        r ^= r >> 7;
        r ^= r << 17;
        r
    };
    let special = [0u64, 1, 2, 3, 0xFFFF_FFFF, 0x1_0000_0000, 0xFFFF_FFFF_FFFF_FFFF, 0x8000_0000_0000_0000, 0x7FFF_FFFF_FFFF_FFFF, 0x8000_0000, 0x1_FFFF_FFFF, 13, 0x0000_0001_0000_0001];
    let mut sets: Vec<(Vec<u64>, Vec<u64>, Vec<u32>)> = vec![(vec![], vec![], vec![])];
    let n = special.len();
    sets.push(((0..n * n).map(|i| special[i / n]).collect(), (0..n * n).map(|i| special[i % n]).collect(), (0..n * n).map(|i| i as u32 * 5).collect()));
    sets.push(((0..300).map(|_| next()).collect(), (0..300).map(|_| next()).collect(), (0..300).map(|_| next() as u32).collect()));
    sets.push(((0..77).map(|i| next() & !(i & 3)).collect(), (0..77).map(|_| next() >> 20).collect(), (0..77).map(|i| i as u32).collect()));
    sets.push(((0..64).map(|_| 0x1234_5678_9ABC_DEF1).collect(), (0..64).map(|_| 7).collect(), (0..64).map(|_| 33).collect()));
    for (a, b, c) in &sets {
        for k in [0u64, 1, 2, 3, 4, 5, 6, 7, 0xFFFF_FFFF_0000_0001] {
            let len = a.len();
            let mut want = vec![0u64; len * crate::WIDE_ROWS];
            for i in 0..len {
                for (row, v) in crate::scalar::wide(a, b, c, k, i).into_iter().enumerate() {
                    want[row * len + i] = v;
                }
            }
            for block in [8, 20, 32, 36, 256] {
                for (w, kernel) in [(8, crate::wide_lanes_w8::wide_lanes_w8 as crate::Kernel), (32, crate::wide_lanes_w32::wide_lanes_w32)] {
                    let got = unsafe { crate::wide_run(kernel, a, b, c, k, block) };
                    for row in 0..crate::WIDE_ROWS {
                        assert_eq!(got[row * len..(row + 1) * len], want[row * len..(row + 1) * len], "row {row}, {len} values, k {k:#x}, groups of {block}, {w} lanes");
                    }
                }
            }
        }
    }
}

/// rerail-ukl-vec's Rust of the label passes' `label_parents` (the engine's batch parents, decision 0475) at 8
/// lanes: the host's parents, on every node of every test document, each alone and all in one batch (a link out
/// of a group's block, a head, and files whose regions follow each other).
#[cfg(target_arch = "aarch64")]
#[test]
fn the_label_passes_parents_are_the_hosts() {
    let lang = crate::parent::json_lang();
    let docs = crate::parent::documents(&lang);
    let all: Vec<&crate::parent::Doc> = docs.iter().collect();
    for batch in docs.iter().map(|d| vec![d]).chain([all]) {
        let b = crate::label::Batch::new(&batch);
        for (w, k) in [(8, crate::label_parents_w8::label_parents_w8 as crate::Kernel), (32, crate::label_parents_w32::label_parents_w32)] {
            let got = unsafe { crate::label::run_parents(k, &b) };
            for (d, got) in batch.iter().zip(got) {
                assert!(got == crate::parent::scalar(d), "{}: not the host's parents ({} files, {w} lanes)", d.name, batch.len());
            }
        }
    }
}


/// The label passes' parents as rerail-ukl-vec's Rust do not grow slower (decision 0567 step 3): the thread's
/// instructions a node, the fewest of 5 runs, at most a tenth over what it took when its lowering was last made
/// faster (2026-11-23: 98.1, 117.3, 103.3; the CPU device's c++ of the same UKL took 111.7, 157.0, 118.7), and
/// under the host's read of each node's parent (`parent::scalar`). A lowering that makes it faster lowers these.
/// Where the thread's counter is not read (`counts::instructions`), only its parents are checked.
#[cfg(target_arch = "aarch64")]
#[test]
fn the_label_passes_parents_take_no_more_instructions() {
    let lang = crate::parent::json_lang();
    for (kind, most) in [("records", 108.0), ("numbers", 129.0), ("random", 114.0)] {
        let text = match kind {
            "numbers" => format!("[{}]", (0..100_000).map(|k| k.to_string()).collect::<Vec<_>>().join(",")),
            "random" => {
                let mut g = crate::parent::Gen::new(100_000);
                let mut s = String::from("[");
                for k in 0..100_000 / 64 {
                    if k > 0 {
                        s.push(',');
                    }
                    g.value(&mut s, 6, 6);
                }
                s.push(']');
                s
            }
            _ => crate::parent::records(3000),
        };
        let d = crate::parent::json_doc(&lang, kind, text.as_bytes());
        let b = crate::label::Batch::new(&[&d]);
        let got = unsafe { crate::label::run_parents(crate::label_parents_w8::label_parents_w8, &b) };
        assert!(got[0] == crate::parent::scalar(&d), "{kind}: not the host's parents");
        let n = f64::from(d.nodes());
        let Some(rust) = crate::counts::fewest(5, || drop(std::hint::black_box(unsafe { crate::label::run_parents(crate::label_parents_w8::label_parents_w8, &b) }))) else {
            continue;
        };
        let host = crate::counts::fewest(5, || drop(std::hint::black_box(crate::parent::scalar(&d)))).unwrap_or(u64::MAX);
        let (rust, host) = (rust as f64 / n, host as f64 / n);
        assert!(rust <= most && rust < host, "{kind}: {rust:.1} instructions a node (at most {most}, and under the host read's {host:.1})");
    }
}

/// The label passes' leaf tests as rerail-ukl-vec's Rust are the CPU device's c++ build's of the same UKL (its text
/// with a line more, so that the device has no Rust of it), over each document alone.
#[cfg(target_arch = "aarch64")]
#[test]
fn the_label_passes_leaves_are_the_cpu_devices() {
    use rerail_gpu::kernels::{buffer_of, launch, program_with, read};
    // SAFETY: no other test of this crate reads the environment or a device.
    unsafe { std::env::set_var("RERAIL_GPU", "cpu") };
    let Some(shared) = rerail_gpu::shared::get() else {
        eprintln!("no CPU device: skipped");
        return;
    };
    assert!(rerail_gpu::device::cpu_chosen(), "not the CPU device");
    let p = program_with("label_passes_cpp", &format!("{}\n// The compiler's build.\n", crate::label::source()), &[("UKL_CONSTANT_0", "16")]).unwrap_or_else(|e| panic!("label_passes: {e}"));
    let k = p.function("label_leaves").unwrap();
    let tests = crate::label::leaf_tests();
    let lang = crate::parent::json_lang();
    let mut q = rerail_gpu::device::ComputeQueue::new(&shared.queue);
    let docs = crate::parent::documents(&lang);
    let mut alone = Vec::new();
    for d in &docs {
        let b = crate::label::Batch::new(&[d]);
        let got = unsafe { crate::label::run_leaves(rerail_ukl_rust::label_leaves_w8::label_leaves_w8, &b, &tests) };
        let words = b.words();
        let shape = crate::label::Shape { recursive: words, file_count: 1, target_count: tests.len() as u32, ..crate::label::Shape::default() };
        // SAFETY: the views and the shape are plain u32s.
        let (views, shape) = unsafe {
            (std::slice::from_raw_parts(b.views.as_ptr().cast::<u32>(), size_of_val(&b.views[..]) / 4), std::slice::from_raw_parts((&raw const shape).cast::<u8>(), size_of::<crate::label::Shape>()))
        };
        let (vb, tb, cb, lb) = (buffer_of(views).unwrap(), buffer_of(&tests).unwrap(), buffer_of(&d.bytes).unwrap(), buffer_of(&vec![0u32; got.len()]).unwrap());
        launch(&q, k).buffer("docs", &vb).buffer("tests", &tb).buffer("leaves", &lb).bytes("shape", shape).buffer("chunk", &cb).run(&[u64::from(words.div_ceil(crate::label::GROUP))], &[u64::from(crate::label::GROUP)]).unwrap();
        q.flush().await_().unwrap();
        let want = read::<u32>(&lb, got.len());
        assert!(want.iter().any(|&w| w != 0), "{}: no test held", d.name);
        let again = unsafe { crate::label::run_leaves(rerail_ukl_rust::label_leaves_w8::label_leaves_w8, &b, &tests) };
        let apart: Vec<(usize, u32, u32, u32)> = (0..got.len()).filter(|&i| got[i] != want[i] || again[i] != got[i]).map(|i| (i, got[i], again[i], want[i])).take(8).collect();
        assert!(apart.is_empty(), "{}: not the CPU device's leaves ({} nodes, {words} words; at, Rust, Rust again, c++: {apart:?})", d.name, d.nodes());
        let w32 = unsafe { crate::label::run_leaves(crate::label_leaves_w32::label_leaves_w32, &b, &tests) };
        let apart: Vec<(usize, u32, u32)> = (0..got.len()).filter(|&i| w32[i] != want[i]).map(|i| (i, w32[i], want[i])).take(8).collect();
        assert!(apart.is_empty(), "{}: the Rust of 32 lanes is not the CPU device's leaves (at, Rust, c++: {apart:?})", d.name);
        alone.push((got, words));
    }
    // All in one batch, each where it lies: each file's words its own leaves.
    let all: Vec<&crate::parent::Doc> = docs.iter().collect();
    let b = crate::label::Batch::new(&all);
    let got = unsafe { crate::label::run_leaves(rerail_ukl_rust::label_leaves_w8::label_leaves_w8, &b, &tests) };
    for (f, (one, words)) in alone.iter().enumerate() {
        for t in 0..tests.len() {
            let (at, n) = (t * b.words() as usize + b.region(f) as usize, b.views[f].nodes.div_ceil(32) as usize);
            assert!(got[at..at + n] == one[t * *words as usize..][..n], "{}: its leaves in a batch are not its own", docs[f].name);
        }
    }
}
