//! Every encoding the lab measures gives back exactly what it was given: codecs (whole and by `read_at`),
//! layouts, blocks, and the tree through events, including a source past 4 GiB (U5).

use rerail_tape_lab::blocks::{self, Cut};
use rerail_tape_lab::codec::{self, Codec, Dictionary};
use rerail_tape_lab::events::{self, Candidates};
use rerail_tape_lab::layout::{self, Extras, Layout};
use rerail_tape_lab::tree::{Node, Tree};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Repetitive bytes, as tapes are: a few motifs repeated with small changes, runs, and noise.
fn data(rng: &mut Rng, n: usize) -> Vec<u8> {
    let motifs: Vec<Vec<u8>> = (0..6).map(|_| (0..4 + rng.below(40) as usize).map(|_| rng.below(256) as u8).collect()).collect();
    let mut v = Vec::with_capacity(n);
    while v.len() < n {
        match rng.below(4) {
            0 => v.extend(std::iter::repeat_n(rng.below(3) as u8, 8 + rng.below(30) as usize)),
            1 => v.push(rng.below(256) as u8),
            _ => {
                let m = &motifs[rng.below(6) as usize];
                v.extend_from_slice(m);
            }
        }
    }
    v.truncate(n);
    v
}

#[test]
fn every_codec_roundtrips_and_reads_any_range() {
    let mut rng = Rng(0x9e3779b97f4a7c15);
    let samples: Vec<Vec<u8>> = (0..40).map(|_| data(&mut rng, 2000)).collect();
    for dict in [Dictionary::none(), Dictionary::train(&samples, 4096)] {
        for &n in &[0usize, 1, 3, 17, 300, 5000, 70_000] {
            let d = data(&mut rng, n);
            for codec in Codec::ALL {
                for k in [1, 16, 64] {
                    let c = codec::compress(codec, &d, &dict, k);
                    assert_eq!(codec::decompress(codec, &c, &dict, d.len()), d, "{} k{k} n{n}", codec.name());
                    if n == 0 {
                        continue;
                    }
                    let mut out = Vec::new();
                    for _ in 0..50 {
                        let off = rng.below(n as u64) as usize;
                        let len = 1 + rng.below((n - off).min(40) as u64) as usize;
                        codec::read_at(codec, &c, &dict, d.len(), off, len, &mut out);
                        assert_eq!(out, &d[off..off + len], "{} k{k} n{n} at {off}+{len}", codec.name());
                    }
                }
            }
        }
    }
}

#[test]
fn replay_finds_repeated_sequences() {
    let mut rng = Rng(7);
    let unit = data(&mut rng, 3000);
    let d: Vec<u8> = unit.iter().chain(&unit).chain(&unit).copied().collect();
    let dict = Dictionary::none();
    let lp = codec::compress(Codec::LpLz, &d, &dict, 64).len();
    let rp = codec::compress(Codec::LpReplay, &d, &dict, 64).len();
    assert!(rp < lp, "replay {rp} not under literal-pool {lp}");
}

/// A random tree in preorder with spans inside the source and gaps up to `gap`.
fn tree(rng: &mut Rng, n: usize, gap: u64) -> Tree {
    // Sizes first (a random preorder shape), then spans by a walk.
    let mut size = vec![1u32; n];
    let mut parent = vec![u32::MAX; n];
    let mut stack: Vec<usize> = Vec::new();
    for i in 0..n {
        while stack.len() > 1 && rng.below(3) == 0 {
            stack.pop();
        }
        if i > 0 {
            if stack.is_empty() {
                stack.push(0);
            }
            parent[i] = *stack.last().unwrap() as u32;
        }
        stack.push(i);
    }
    for i in (1..n).rev() {
        size[parent[i] as usize] += size[i];
    }
    let mut nodes: Vec<Node> = (0..n)
        .map(|_| Node { kind: (rng.below(40) + 1) as u16, field: rng.below(5) as u8, flags: (rng.below(8) == 0) as u8, classes: Vec::new(), start: 0, end: 0 })
        .collect();
    let mut pos = 0u64;
    let mut open: Vec<usize> = Vec::new();
    for i in 0..n {
        while let Some(&j) = open.last() {
            if j + size[j] as usize > i {
                break;
            }
            pos += rng.below(gap);
            nodes[j].end = pos;
            open.pop();
        }
        pos += rng.below(gap);
        nodes[i].start = pos;
        open.push(i);
    }
    while let Some(j) = open.pop() {
        pos += rng.below(gap);
        nodes[j].end = pos;
    }
    let source_len = pos + rng.below(gap);
    // Kinds 1–3 carry supertypes, as grammars' do for some kinds.
    for nd in &mut nodes {
        if nd.kind <= 3 {
            nd.classes = vec![100 + nd.kind, 200];
        }
    }
    Tree { nodes, size, parent, source_len }
}

#[test]
fn every_solution_roundtrips_a_tree() {
    let mut rng = Rng(42);
    for n in [1usize, 2, 50, 3000] {
        let t = tree(&mut rng, n, 20);
        let mut cand = Candidates::default();
        cand.learn(1, &t);
        let ev = events::encode(&t, 1, &cand);
        assert_eq!(events::decode(&ev, 1, &cand, t.source_len), t);
        for cut in [Cut::Whole, Cut::Target(64), Cut::Target(256)] {
            let (bs, places) = blocks::split(&ev, cut);
            assert_eq!(bs.len(), places.len());
            for layout in Layout::ALL {
                for extras in [Extras::default(), Extras { parent: true, size: true }] {
                    let up: Vec<u32> = (0..n).map(|i| if t.parent[i] == u32::MAX { 0 } else { i as u32 - t.parent[i] }).collect();
                    let back: Vec<_> = bs
                        .iter()
                        .zip(&places)
                        .map(|(b, pl)| {
                            let a = pl.first_node as usize;
                            let m = b.opens.len();
                            let bytes = layout::lay(b, layout, extras, &up[a..a + m], &t.size[a..a + m]);
                            let c = codec::compress(Codec::LpReplay, &bytes, &Dictionary::none(), 16);
                            layout::unlay(&codec::decompress(Codec::LpReplay, &c, &Dictionary::none(), bytes.len()), layout, extras)
                        })
                        .collect();
                    assert_eq!(&back, &bs, "{n} nodes, {} {}", cut.name(), layout.name());
                    assert_eq!(events::decode(&blocks::join(&back), 1, &cand, t.source_len), t);
                }
            }
        }
    }
}

#[test]
fn a_source_past_4_gib_keeps_exact_positions() {
    // Gaps up to 1.5 GiB (under format 16's 2 GiB limit per gap or leaf): 40 nodes reach well past 4 GiB.
    let mut rng = Rng(3);
    let t = tree(&mut rng, 40, 3 << 29);
    assert!(t.source_len > 1 << 33, "source {} not past 8 GiB", t.source_len);
    let cand = Candidates::default();
    let mut t = t;
    for n in &mut t.nodes {
        n.classes.clear();
    }
    let ev = events::encode(&t, 1, &cand);
    assert_eq!(events::decode(&ev, 1, &cand, t.source_len), t);
    let (bs, places) = blocks::split(&ev, Cut::Target(8));
    assert!(places.iter().any(|p| p.base > u64::from(u32::MAX)), "no block placed past 4 GiB");
    assert_eq!(events::decode(&blocks::join(&bs), 1, &cand, t.source_len), t);
}

#[test]
fn span_codecs_roundtrip_files_of_blocks() {
    let mut rng = Rng(11);
    let shared = data(&mut rng, 4000);
    let samples: Vec<Vec<u8>> = (0..40).map(|_| data(&mut rng, 2000)).collect();
    for dict in [Dictionary::none(), Dictionary::train(&samples, 4096)] {
        // Blocks that repeat each other's content, as a file's blocks do.
        let blocks: Vec<Vec<u8>> = (0..6)
            .map(|i| {
                let mut b = data(&mut rng, 500 + 700 * i);
                b.extend_from_slice(&shared[..(1000 * i).min(shared.len())]);
                b
            })
            .collect();
        for codec in Codec::ALL {
            for k in [1, 16, 64] {
                let c = codec::compress_file(codec, &blocks, &dict, k);
                for (b, d) in blocks.iter().enumerate() {
                    assert_eq!(&codec::decompress_in(codec, &c, b, &dict, d.len()), d, "{} k{k} block {b}", codec.name());
                    let mut out = Vec::new();
                    for _ in 0..40 {
                        let off = rng.below(d.len() as u64) as usize;
                        let len = 1 + rng.below((d.len() - off).min(40) as u64) as usize;
                        codec::read_at_in(codec, &c, b, &dict, d.len(), off, len, &mut out);
                        assert_eq!(out, &d[off..off + len], "{} k{k} block {b} at {off}+{len}", codec.name());
                    }
                }
            }
        }
        // Crossing blocks makes the later blocks smaller.
        let alone: usize = codec::compress_file(Codec::span(4, false), &blocks, &dict, 64).iter().map(Vec::len).sum();
        let cross: usize = codec::compress_file(Codec::span(4, true), &blocks, &dict, 64).iter().map(Vec::len).sum();
        assert!(cross < alone, "cross {cross} not under alone {alone}");
    }
}

#[test]
fn the_entropy_stage_gives_back_the_lz_block() {
    use rerail_tape_lab::huff;
    let mut rng = Rng(5);
    let dict = Dictionary::none();
    let train: Vec<Vec<u8>> = (0..20).map(|_| codec::compress(Codec::LpLz, &data(&mut rng, 3000), &dict, 16)).collect();
    let (mut sc, mut lc) = ([0u64; 256], [0u64; 256]);
    huff::count(&train, &mut sc, &mut lc);
    let (st, lt) = (huff::Table::train(&sc), huff::Table::train(&lc));
    // A skewed table still codes every byte (unseen bytes get the longest codes).
    let mut skew = [0u64; 256];
    skew[0] = 1 << 40;
    let sk = huff::Table::train(&skew);
    for &n in &[1usize, 100, 5000, 40_000] {
        let d = data(&mut rng, n);
        for codec in [Codec::LpLz, Codec::LpReplay, Codec::LzOutput, Codec::span(4, false)] {
            for k in [1, 16, 64] {
                let c = codec::compress(codec, &d, &dict, k);
                for (s, l) in [(&st, &lt), (&sk, &sk)] {
                    let e = huff::encode(&c, s, l);
                    assert_eq!(huff::decode_block(&e, s, l), c, "{} k{k} n{n}", codec.name());
                }
            }
        }
    }
}

#[test]
fn every_span_pointer_form_round_trips_plain_and_entropy_coded() {
    use rerail_tape_lab::huff;
    use rerail_tape_lab::span::{self, Ptr};
    let mut rng = Rng(11);
    let dict = Dictionary::new(data(&mut rng, 2000));
    // A file of blocks sharing content, so spans reach into earlier blocks.
    let base = data(&mut rng, 6000);
    let blocks: Vec<Vec<u8>> = (0..6)
        .map(|_| {
            let mut b = base.clone();
            for _ in 0..30 {
                let i = rng.below(b.len() as u64) as usize;
                b[i] = rng.below(256) as u8;
            }
            b.truncate(1000 + rng.below(5000) as usize);
            b
        })
        .collect();
    for ptr in Ptr::ALL {
        for cross in [false, true] {
            for k in [1, 16, 64] {
                let codec = Codec::LpSpan { depth: 4, cross, ptr };
                let c = codec::compress_file(codec, &blocks, &dict, k);
                let (mut sc, mut lc) = ([0u64; 256], [0u64; 256]);
                huff::count(&c, &mut sc, &mut lc);
                let t = (huff::Table::train(&sc), huff::Table::train(&lc));
                let coded: Vec<Vec<u8>> = c.iter().map(|b| huff::encode(b, &t.0, &t.1)).collect();
                let name = codec.name();
                for (b, d) in blocks.iter().enumerate() {
                    assert_eq!(&codec::decompress_in(codec, &c, b, &dict, d.len()), d, "{name} k{k} block {b}");
                    assert_eq!(&span::decode(&mut span::coded_sources(&coded, &t), b, &dict.pool, ptr), d, "{name}+huff k{k} block {b}");
                    let mut out = Vec::new();
                    for _ in 0..30 {
                        let off = rng.below(d.len() as u64) as usize;
                        let len = 1 + rng.below((d.len() - off).min(40) as u64) as usize;
                        codec::read_at_in(codec, &c, b, &dict, d.len(), off, len, &mut out);
                        assert_eq!(out, &d[off..off + len], "{name} k{k} block {b} at {off}+{len}");
                        // Entropy-coded, decoding only the segments the read reaches.
                        let mut srcs = span::coded_sources(&coded, &t);
                        span::read(&mut srcs, b, &dict.pool, ptr, off, len, &mut out);
                        assert_eq!(out, &d[off..off + len], "{name}+huff k{k} block {b} at {off}+{len}");
                    }
                }
            }
        }
    }
}
