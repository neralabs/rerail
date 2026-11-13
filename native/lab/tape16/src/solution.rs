//! A candidate encoding (`Solution`) and its evaluation on a corpus: one row of numbers across T1–T5.

use crate::blocks::{self, Block, Cut};
use crate::capture::File;
use crate::codec::{self, Codec, Dictionary};
use crate::events::{self, Candidates, Events};
use crate::layout::{self, Extras, Layout};
use crate::measure::{Cost, cost};
use crate::tree::{NONE, Tree};
use crate::{huff, v15};
use std::collections::{BTreeMap, HashMap, HashSet};

/// A captured file, read and encoded once: what every solution starts from. Only the events and the source
/// are kept (the tree is decoded again where a step needs it), so a corpus of millions of nodes fits the
/// lab's 1 GB.
pub struct Prepared {
    pub path: String,
    pub tag: u8,
    pub test: bool,
    pub source: Vec<u8>,
    pub events: Events,
    pub nodes: usize,
    pub v15_raw: usize,
    pub v15_stored: usize,
    /// Format 15 under zstd with a dictionary trained on the training half (test files only; else 0).
    pub v15_trained: usize,
    pub source_stored: usize,
}

impl Prepared {
    pub fn tree(&self, corpus: &Corpus) -> Tree {
        events::decode(&self.events, self.tag, &corpus.candidates, self.source.len() as u64)
    }
}

pub struct Corpus {
    pub files: Vec<Prepared>,
    pub candidates: Candidates,
    /// Whether format 15 had a dictionary trained for the language (zstd's trainer needs 8 samples).
    pub v15_trained: BTreeMap<u8, bool>,
}

/// Training samples: 4 KiB pieces, at most 16 MiB per language (zstd's trainer wants many small samples).
fn add_samples(samples: &mut Vec<Vec<u8>>, total: &mut usize, bytes: &[u8]) {
    for piece in bytes.chunks(4096) {
        if *total > 16 << 20 {
            return;
        }
        *total += piece.len();
        samples.push(piece.to_vec());
    }
}

pub fn prepare(files: Vec<File>) -> Result<Corpus, String> {
    prepare_with(files, None)
}

/// As `prepare`, with the supertype candidates already learned (a streamed corpus learns them from every
/// file first, so every shard numbers masks alike).
pub fn prepare_with(files: Vec<File>, known: Option<Candidates>) -> Result<Corpus, String> {
    // Pass 1: every tree's supertypes (a mask indexes the full candidate list), and format 15's training half.
    let learn = known.is_none();
    let mut candidates = known.unwrap_or_default();
    let mut v15_train: BTreeMap<u8, (Vec<Vec<u8>>, usize)> = BTreeMap::new();
    for f in &files {
        if learn {
            let tree = v15::read(&f.image).map_err(|e| format!("{}: {e}", f.path))?;
            candidates.learn(f.tag, &tree);
        }
        if !f.is_test() {
            let (v, total) = v15_train.entry(f.tag).or_default();
            add_samples(v, total, &f.image);
        }
    }
    let trained: BTreeMap<u8, Dictionary> = v15_train.iter().map(|(t, (v, _))| (*t, Dictionary::train(v, 64 * 1024))).collect();
    // Pass 2: events, the baselines' sizes; the image and the tree are dropped.
    let mut out = Vec::with_capacity(files.len());
    for f in files {
        let tree = v15::read(&f.image).map_err(|e| format!("{}: {e}", f.path))?;
        let events = events::encode(&tree, f.tag, &candidates);
        let store_dict = v15::store_tape_dictionary(f.tag);
        let source_dict = v15::store_source_dictionary(f.tag);
        let test = f.is_test();
        let v15_trained = match trained.get(&f.tag) {
            Some(d) if test && !d.bytes.is_empty() => codec::compress(Codec::Zstd, &f.image, d, 64).len(),
            _ => 0,
        };
        out.push(Prepared {
            tag: f.tag,
            test,
            nodes: tree.len(),
            v15_raw: f.image.len(),
            v15_stored: v15::stored_size(&f.image, store_dict.as_deref()),
            v15_trained,
            source_stored: v15::stored_size(&f.source, source_dict.as_deref()),
            path: f.path,
            source: f.source,
            events,
        });
    }
    let v15_trained = trained.into_iter().map(|(t, d)| (t, !d.bytes.is_empty())).collect();
    Ok(Corpus { files: out, candidates, v15_trained })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Solution {
    pub layout: Layout,
    pub codec: Codec,
    pub cut: Cut,
    pub dict: usize,
    pub k: usize,
    pub extras: Extras,
    /// The entropy stage (`huff`) over an LZ codec's sequences and literals.
    pub entropy: bool,
}

impl Solution {
    pub fn name(&self) -> String {
        let mut s = format!("{}/{}/{}/d{}k", self.layout.name(), self.codec.name(), self.cut.name(), self.dict / 1024);
        if self.codec.has_k() {
            s += &format!("/k{}", self.k);
        }
        if self.extras.parent {
            s += "/+parent";
        }
        if self.extras.size {
            s += "/+size";
        }
        if self.entropy {
            s += "/+huff";
        }
        s
    }

    /// Whether the entropy stage applies (only LZ codecs have sequences and literals to code).
    pub fn huff(&self) -> bool {
        self.entropy && self.codec.has_k()
    }
}

/// Per node, `i − parent` and the subtree size, for a block's extras.
fn extras_for(tree: &Tree) -> (Vec<u32>, Vec<u32>) {
    let up = (0..tree.len()).map(|i| if tree.parent[i] == NONE { 0 } else { i as u32 - tree.parent[i] }).collect();
    (up, tree.size.clone())
}

/// The blocks, their placement and laid-out bytes; `tree` only for extras.
fn lay_blocks(p: &Prepared, tree: Option<&Tree>, s: &Solution) -> (Vec<Block>, Vec<blocks::Placement>, Vec<Vec<u8>>) {
    let (bs, places) = blocks::split(&p.events, s.cut);
    let (up, size) = match tree {
        Some(t) if s.extras.parent || s.extras.size => extras_for(t),
        _ => (Vec::new(), Vec::new()),
    };
    let mut bytes = Vec::with_capacity(bs.len());
    for (b, pl) in bs.iter().zip(&places) {
        let a = pl.first_node as usize;
        let n = b.opens.len();
        let (u, z) = if up.is_empty() { (&[][..], &[][..]) } else { (&up[a..a + n], &size[a..a + n]) };
        bytes.push(layout::lay(b, s.layout, s.extras, u, z));
    }
    (bs, places, bytes)
}

/// The dictionaries a solution needs, by language, trained on the training half's laid-out blocks.
#[derive(Default)]
pub struct Dicts {
    cache: HashMap<(u8, Layout, Cut, usize, bool, bool), std::sync::Arc<Dictionary>>,
    tables: HashMap<(u8, Solution), std::sync::Arc<(huff::Table, huff::Table)>>,
}

impl Dicts {
    pub fn get(&mut self, corpus: &Corpus, tag: u8, s: &Solution) -> std::sync::Arc<Dictionary> {
        let key = (tag, s.layout, s.cut, s.dict, s.extras.parent, s.extras.size);
        if let Some(d) = self.cache.get(&key) {
            return d.clone();
        }
        let mut samples = Vec::new();
        let mut total = 0usize;
        if s.dict > 0 {
            for p in corpus.files.iter().filter(|p| !p.test && p.tag == tag) {
                let tree = (s.extras.parent || s.extras.size).then(|| p.tree(corpus));
                for b in lay_blocks(p, tree.as_ref(), s).2 {
                    add_samples(&mut samples, &mut total, &b);
                }
                if total > 16 << 20 {
                    break;
                }
            }
        }
        let d = std::sync::Arc::new(Dictionary::train(&samples, s.dict));
        self.cache.insert(key, d.clone());
        d
    }

    /// The language's Huffman tables for the solution's sequences and literals, from the training half's
    /// compressed blocks (at most 8 MiB of them).
    pub fn tables(&mut self, corpus: &Corpus, tag: u8, s: &Solution) -> std::sync::Arc<(huff::Table, huff::Table)> {
        let key = (tag, Solution { entropy: false, ..*s });
        if let Some(t) = self.tables.get(&key) {
            return t.clone();
        }
        let dict = self.get(corpus, tag, s);
        let (mut seq, mut lit) = ([0u64; 256], [0u64; 256]);
        let mut total = 0usize;
        for p in corpus.files.iter().filter(|p| !p.test && p.tag == tag) {
            let tree = (s.extras.parent || s.extras.size).then(|| p.tree(corpus));
            let laid = lay_blocks(p, tree.as_ref(), s).2;
            let c = codec::compress_file(s.codec, &laid, &dict, s.k);
            total += c.iter().map(Vec::len).sum::<usize>();
            huff::count(&c, &mut seq, &mut lit);
            if total > 8 << 20 {
                break;
            }
        }
        let t = std::sync::Arc::new((huff::Table::train(&seq), huff::Table::train(&lit)));
        self.tables.insert(key, t.clone());
        t
    }
}

#[derive(Clone, Debug, Default)]
pub struct Lang {
    pub files: usize,
    pub nodes: u64,
    pub source_bytes: u64,
    pub source_stored: u64,
    pub v15_raw: u64,
    pub v15_stored: u64,
    pub v15_trained: u64,
    pub laid: u64,
    pub stored: u64,
    pub placement: u64,
    pub entropy: f64,
    pub dict_bytes: u64,
    pub blocks: u64,
    pub dup_bytes: u64,
    pub trained: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Row {
    pub solution: String,
    pub by_lang: BTreeMap<String, Lang>,
    pub encode: Cost,
    pub decode: Cost,
    pub positions: Cost,
    pub random: Cost,
    pub random_reads: u64,
    pub errors: usize,
}

impl Lang {
    pub fn merge(&mut self, o: &Lang) {
        self.files += o.files;
        self.nodes += o.nodes;
        self.source_bytes += o.source_bytes;
        self.source_stored += o.source_stored;
        self.v15_raw += o.v15_raw;
        self.v15_stored += o.v15_stored;
        self.v15_trained += o.v15_trained;
        self.laid += o.laid;
        self.stored += o.stored;
        self.placement += o.placement;
        self.entropy += o.entropy;
        self.dict_bytes = self.dict_bytes.max(o.dict_bytes);
        self.blocks += o.blocks;
        self.dup_bytes += o.dup_bytes;
        self.trained |= o.trained;
    }
}

impl Row {
    /// Another shard's row of the same solution added in.
    pub fn merge(&mut self, o: &Row) {
        for (k, l) in &o.by_lang {
            self.by_lang.entry(k.clone()).or_default().merge(l);
        }
        self.encode += o.encode;
        self.decode += o.decode;
        self.positions += o.positions;
        self.random += o.random;
        self.random_reads += o.random_reads;
        self.errors += o.errors;
        if self.solution.is_empty() {
            self.solution = o.solution.clone();
        }
    }

    pub fn total(&self) -> Lang {
        let mut t = Lang::default();
        for l in self.by_lang.values() {
            t.files += l.files;
            t.nodes += l.nodes;
            t.source_bytes += l.source_bytes;
            t.source_stored += l.source_stored;
            t.v15_raw += l.v15_raw;
            t.v15_stored += l.v15_stored;
            t.v15_trained += l.v15_trained;
            t.laid += l.laid;
            t.stored += l.stored;
            t.placement += l.placement;
            t.entropy += l.entropy;
            t.dict_bytes += l.dict_bytes;
            t.blocks += l.blocks;
            t.dup_bytes += l.dup_bytes;
        }
        t
    }
}

/// Where node `i`'s kind (2 bytes) sits in its block's laid-out bytes, as reads of (offset, len).
fn kind_reads(layout: Layout, n: usize, i: usize, x: Extras) -> Vec<(usize, usize)> {
    match layout {
        Layout::BytePlanes => vec![(16 + i, 1), (16 + n + i, 1)],
        Layout::FieldPlanes => vec![(16 + 2 * i, 2)],
        Layout::Records => vec![(16 + i * (14 + 4 * (x.parent as usize + x.size as usize)), 2)],
        Layout::Varint => vec![],
    }
}

pub fn evaluate(corpus: &Corpus, s: &Solution, dicts: &mut Dicts, check: bool) -> Row {
    let mut row = Row { solution: s.name(), ..Default::default() };
    let mut seen: HashSet<[u8; 32]> = HashSet::new();
    let mut tags_seen: HashSet<u8> = HashSet::new();
    let mut rng = 0x2545f4914f6cdd1du64;
    for p in corpus.files.iter().filter(|p| p.test) {
        let dict = dicts.get(corpus, p.tag, s);
        let lang = row.by_lang.entry(crate::capture::language(p.tag).to_owned()).or_default();
        if tags_seen.insert(p.tag) {
            lang.dict_bytes = dict.bytes.len() as u64;
            lang.trained = corpus.v15_trained.get(&p.tag).copied().unwrap_or(false);
        }
        lang.files += 1;
        let tree = p.tree(corpus);
        lang.nodes += tree.len() as u64;
        lang.source_bytes += p.source.len() as u64;
        lang.source_stored += p.source_stored as u64;
        lang.v15_raw += p.v15_raw as u64;
        lang.v15_stored += p.v15_stored as u64;
        // With too few training files to train on (zstd needs 8), the store's dictionary stands in.
        lang.v15_trained += if lang.trained { p.v15_trained } else { p.v15_stored } as u64;
        // Encode (E6, the writer from a tree): events, blocks, laid out, compressed.
        let tables = s.huff().then(|| dicts.tables(corpus, p.tag, s));
        let ((bs, places, compressed, laid, coded), c) = cost(|| {
            std::hint::black_box(events::encode(&tree, p.tag, &corpus.candidates));
            let (bs, places, laid) = lay_blocks(p, Some(&tree), s);
            let compressed = codec::compress_file(s.codec, &laid, &dict, s.k);
            let coded: Option<Vec<Vec<u8>>> = tables.as_ref().map(|t| compressed.iter().map(|c| huff::encode(c, &t.0, &t.1)).collect());
            (bs, places, compressed, laid, coded)
        });
        row.encode += c;
        lang.blocks += bs.len() as u64;
        lang.laid += laid.iter().map(|b| b.len() as u64).sum::<u64>();
        lang.stored += coded.as_ref().unwrap_or(&compressed).iter().map(|b| b.len() as u64).sum::<u64>();
        if bs.len() > 1 {
            lang.placement += (places.len() * blocks::PLACEMENT_BYTES) as u64;
        }
        if s.huff() {
            lang.entropy += coded.as_ref().unwrap().iter().map(|b| b.len() as f64).sum::<f64>();
        } else if s.codec.has_k() {
            lang.entropy += compressed.iter().map(|b| crate::lp::entropy_bytes(b)).sum::<f64>();
        } else {
            lang.entropy += compressed.iter().map(|b| b.len() as f64).sum::<f64>();
        }
        for (c, b) in compressed.iter().zip(&laid) {
            let h: [u8; 32] = blake3::hash(b).into();
            if !seen.insert(h) {
                lang.dup_bytes += c.len() as u64;
            }
        }
        // Decode: every block decompressed, read back, joined, and the tree derived.
        let (back, c) = cost(|| {
            // With the entropy stage, every block's streams are decoded first (the GPU's first pass).
            let plain: Vec<Vec<u8>>;
            let compressed = match (&coded, &tables) {
                (Some(cd), Some(t)) => {
                    plain = cd.iter().map(|c| huff::decode_block(c, &t.0, &t.1)).collect();
                    &plain
                }
                _ => &compressed,
            };
            let lens: Vec<usize> = laid.iter().map(Vec::len).collect();
            let back: Vec<Block> = codec::decompress_file(s.codec, compressed, &dict, &lens)
                .iter()
                .map(|b| layout::unlay(b, s.layout, s.extras))
                .collect();
            events::decode(&blocks::join(&back), p.tag, &corpus.candidates, tree.source_len)
        });
        row.decode += c;
        if check && back != tree {
            row.errors += 1;
        }
        let ((), c) = cost(|| {
            let (mut a, mut b) = (Vec::new(), Vec::new());
            events::positions(&p.events, &mut a, &mut b);
            std::hint::black_box((&a, &b));
        });
        row.positions += c;
        // Random reads: 8 nodes' kinds per file, each from its compressed block alone.
        let mut out = Vec::new();
        let ((), c) = cost(|| {
            for _ in 0..8 {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                let k = (rng % bs.len() as u64) as usize;
                let n = bs[k].opens.len();
                if n == 0 {
                    continue;
                }
                let i = ((rng >> 20) % n as u64) as usize;
                let reads = kind_reads(s.layout, n, i, s.extras);
                // With the entropy stage, span LZ decodes only the checkpoint segments the read reaches (in
                // this block, and in any block a span names). The other LZ codecs decode the blocks the read
                // may touch first.
                match (&coded, &tables, s.codec) {
                    (Some(cd), Some(t), Codec::LpSpan { ptr, cross, .. }) => {
                        let first = if cross { 0 } else { k };
                        let mut srcs = crate::span::coded_sources(&cd[first..=k], t);
                        for &(off, len) in &reads {
                            crate::span::read(&mut srcs, k - first, &dict.pool, ptr, off, len, &mut out);
                            std::hint::black_box(&out);
                        }
                        if reads.is_empty() {
                            let all = crate::span::decode(&mut srcs, k - first, &dict.pool, ptr);
                            std::hint::black_box(layout::unlay(&all, s.layout, s.extras).opens[i].kind);
                        }
                        continue;
                    }
                    _ => {}
                }
                let plain: Vec<Vec<u8>>;
                let (compressed, kb) = match (&coded, &tables) {
                    (Some(cd), Some(t)) => {
                        plain = cd[k..=k].iter().map(|c| huff::decode_block(c, &t.0, &t.1)).collect();
                        (&plain, 0)
                    }
                    _ => (&compressed, k),
                };
                if reads.is_empty() {
                    let all = codec::decompress_in(s.codec, compressed, kb, &dict, laid[k].len());
                    std::hint::black_box(layout::unlay(&all, s.layout, s.extras).opens[i].kind);
                } else {
                    for (off, len) in reads {
                        codec::read_at_in(s.codec, compressed, kb, &dict, laid[k].len(), off, len, &mut out);
                        std::hint::black_box(&out);
                    }
                }
            }
        });
        row.random += c;
        row.random_reads += 8;
    }
    row
}
