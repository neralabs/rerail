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
        for block in [20, 32, 36, 64, 256] {
            let mut state = f.start.clone();
            let most = unsafe { crate::nearest_run(crate::nearest_jump_w8::nearest_jump_w8, &mut state, args(&f), block) };
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
                let got = unsafe { crate::mix_run(crate::mix_lanes_w8::mix_lanes_w8, input, rounds, block) };
                assert_eq!(got, want, "{} values, {rounds} rounds, groups of {block}", input.len());
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
        for block in [8, 20, 32, 36, 256] {
            let got = unsafe { crate::parent::run(crate::parent_all_w8::parent_all_w8, &d, block) };
            if let Some(i) = (0..want.len()).find(|&i| got[i] != want[i]) {
                panic!("{} ({} nodes), groups of {block}: node {i}'s parent {} (the host's {})", d.name, want.len(), got[i], want[i]);
            }
        }
    }
}
