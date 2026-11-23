//! Format 16's parent read (`clif_f16_parent`, `kernels/f16_parent.uglykl`) over real tapes: JSON documents parsed by
//! JSON's parser as Rust (`rerail-json-aot`), which writes format 16 through the engine's writer, each read in place
//! through its view. The scalar reference is the host's own read (`rerail_tape_format::reads::f16::parent`, which
//! the tape crate's tests hold to the CLIF reads' JIT).

use rerail_clif::view::TapeView;
use rerail_tape_format::f16::{Lang, Writer, Written};

/// No parent: the root's.
pub const NONE: u32 = u32::MAX;

/// A document's format 16 tape and the view its reads take (its offsets from the tape's first byte).
pub struct Doc {
    pub name: String,
    pub bytes: Vec<u8>,
    pub view: TapeView,
}

impl Doc {
    pub fn nodes(&self) -> u32 {
        self.view.nodes
    }
}

/// JSON's language, its table the one its parser's generator made.
pub fn json_lang() -> Lang {
    let entries = rerail_json_aot::ENTRIES.iter().map(|&(kind, field, classes)| rerail_tape_format::tape::Entry { kind, field, flags: 0, alias: None, classes: classes.to_vec() }).collect();
    Lang::new(rerail_tape_format::tape::CodeTable::new(entries), rerail_json_aot::WIDTHS, 1).expect("JSON's language")
}

/// `src`'s format 16 tape, by JSON's parser and the engine's writer.
pub fn json_doc(lang: &Lang, name: &str, src: &[u8]) -> Doc {
    let mut w = Writer::new(lang, src, 0, false, false, src.len() / 2 + 16);
    rerail_json_aot::parse(&rerail_json_aot::padded(src), src.len(), &mut w).unwrap_or_else(|b| panic!("{name}: JSON's parser refused it at byte {b}"));
    let Written::F16(t) = w.finish().unwrap_or_else(|e| panic!("{name}: {e}")).tape else { panic!("{name}: not format 16") };
    let view = t.placement().view(lang, 0).unwrap_or_else(|e| panic!("{name}: {e}"));
    Doc { name: name.to_owned(), bytes: t.as_bytes().to_vec(), view }
}

/// Each node's parent by the host's read: the reference.
pub fn scalar(d: &Doc) -> Vec<u32> {
    (0..d.nodes()).map(|i| rerail_tape_format::reads::f16::parent(&d.bytes, &d.view, i)).collect()
}

/// `f16_parent`'s arguments' struct (`ParentArgs`).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ParentArgs {
    pub nodes: u32,
    pub pad0: u32,
    pub pad1: u32,
    pub pad2: u32,
}

/// A corrupt tape: a node whose chain of links to previous siblings goes past 7 (format 16 names the parent on every
/// 8th sibling, so no tape its writer made holds one). `node` is one such node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Corrupt {
    pub node: u32,
}

impl std::fmt::Display for Corrupt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "corrupt format 16 tape: node {}'s parent is more than 7 links of siblings back (every 8th names it)", self.node)
    }
}

impl std::error::Error for Corrupt {}

/// `parent_link`'s fault word (`out`'s past the nodes', 0 before the launch): the corrupt tape it names, if any.
pub fn fault(word: u32) -> Result<(), Corrupt> {
    if word == 0 { Ok(()) } else { Err(Corrupt { node: word - 1 }) }
}

/// `f16_parent`'s parent kernel of two launches as Rust (`step`: `parent_step`, `link`: `parent_link`) over `d`'s
/// nodes in groups of `block` threads: each node's step into `steps`, then each linked node's chase of them. Each
/// node's parent, or the tape's corruption (a chain past 7).
///
/// # Safety
///
/// `step` and `link` are kernels of `parent_step`'s and `parent_link`'s arguments.
pub unsafe fn run_two(step: crate::Kernel, link: crate::Kernel, d: &Doc, block: u32) -> Result<Vec<u32>, Corrupt> {
    let mut steps = vec![0u32; d.nodes() as usize];
    let (tp, vp) = (d.bytes.as_ptr(), (&raw const d.view).cast::<u8>());
    let sp = steps.as_mut_ptr().cast::<u8>().cast_const();
    let (counts, first): (*const u32, u32) = (std::ptr::null(), 0);
    let args = ParentArgs { nodes: d.nodes(), pad0: 0, pad1: 0, pad2: 0 };
    let p = |x: &*const u8| (&raw const *x).cast::<u8>();
    let (a, c, f) = ((&raw const args).cast::<u8>(), (&raw const counts).cast::<u8>(), (&raw const first).cast::<u8>());
    unsafe { crate::launch(step, &[p(&tp), p(&vp), p(&sp), a, c, f], d.nodes().div_ceil(block), block) };
    unsafe { run_link(link, &steps, block) }
}

/// `parent_link` alone, as Rust, over `steps` (each node's step: its parent, or `LINKED | j`) in groups of `block`
/// threads: each node's parent, or the corruption of a chain past 7.
///
/// # Safety
///
/// `link` is a kernel of `parent_link`'s arguments.
pub unsafe fn run_link(link: crate::Kernel, steps: &[u32], block: u32) -> Result<Vec<u32>, Corrupt> {
    let n = steps.len() as u32;
    // Each node's word, then the fault word, 0.
    let mut out = vec![0u32; steps.len() + 1];
    let sp = steps.as_ptr().cast::<u8>();
    let op = out.as_mut_ptr().cast::<u8>().cast_const();
    let (counts, first): (*const u32, u32) = (std::ptr::null(), 0);
    let args = ParentArgs { nodes: n, pad0: 0, pad1: 0, pad2: 0 };
    let p = |x: &*const u8| (&raw const *x).cast::<u8>();
    let (a, c, f) = ((&raw const args).cast::<u8>(), (&raw const counts).cast::<u8>(), (&raw const first).cast::<u8>());
    unsafe { crate::launch(link, &[p(&sp), p(&op), a, c, f], n.div_ceil(block), block) };
    let word = out.pop().unwrap_or(0);
    fault(word).map(|()| out)
}

/// `f16_parent`'s kernel of one launch as Rust (`parent_group`) over `d`'s nodes in groups of `block` threads (at
/// most 256, its threadgroup memory's steps): each group's steps, then each linked node's chase of them. Each
/// node's parent.
///
/// # Safety
///
/// `group` is a kernel of `parent_group`'s arguments.
pub unsafe fn run(group: crate::Kernel, d: &Doc, block: u32) -> Vec<u32> {
    assert!(block <= 256, "a group of {block} threads: parent_group's steps are 256");
    let n = d.nodes() as usize;
    let mut out = vec![0u32; n];
    let (tp, vp) = (d.bytes.as_ptr(), (&raw const d.view).cast::<u8>());
    let op = out.as_mut_ptr().cast::<u8>().cast_const();
    let (counts, first): (*const u32, u32) = (std::ptr::null(), 0);
    let args = ParentArgs { nodes: d.nodes(), pad0: 0, pad1: 0, pad2: 0 };
    let p = |x: &*const u8| (&raw const *x).cast::<u8>();
    let (a, c, f) = ((&raw const args).cast::<u8>(), (&raw const counts).cast::<u8>(), (&raw const first).cast::<u8>());
    unsafe { crate::launch(group, &[p(&tp), p(&vp), p(&op), a, c, f], d.nodes().div_ceil(block), block) };
    out
}

/// Spike: `parent_back` (a look-back, no barrier, one launch) as Rust over `d`'s nodes in groups of `block` threads:
/// `out` as it comes (zeros, wrong parents), `seen` all ones (every group published), its counts 0. Each node's parent.
///
/// # Safety
///
/// `back` is a kernel of `parent_back`'s arguments.
pub unsafe fn run_back(back: crate::Kernel, d: &Doc, block: u32) -> Vec<u32> {
    let mut out = vec![0u32; d.nodes() as usize];
    let mut counts4 = [0u32; 128];
    // Every group's bit set, as if published: only the launch's own fill clears them.
    let mut seen = vec![u32::MAX; (d.nodes() as usize).div_ceil(1024) + 1];
    let (tp, vp) = (d.bytes.as_ptr(), (&raw const d.view).cast::<u8>());
    let (op, kp) = (out.as_mut_ptr().cast::<u8>().cast_const(), counts4.as_mut_ptr().cast::<u8>().cast_const());
    let sp = seen.as_mut_ptr().cast::<u8>().cast_const();
    let (counts, first): (*const u32, u32) = (std::ptr::null(), 0);
    let args = ParentArgs { nodes: d.nodes(), pad0: 0, pad1: 0, pad2: 0 };
    let p = |x: &*const u8| (&raw const *x).cast::<u8>();
    let (a, c, f) = ((&raw const args).cast::<u8>(), (&raw const counts).cast::<u8>(), (&raw const first).cast::<u8>());
    unsafe { crate::launch(back, &[p(&tp), p(&vp), p(&op), p(&sp), p(&kp), a, c, f], d.nodes().div_ceil(block), block) };
    out
}


/// A generator of JSON texts, the same from the same seed.
pub struct Gen(u64);

impl Gen {
    pub fn new(seed: u64) -> Gen {
        Gen(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    /// A value of at most `depth` more levels: a scalar, or an array or object of up to `wide` members.
    pub fn value(&mut self, out: &mut String, depth: u32, wide: u64) {
        match if depth == 0 { self.below(4) } else { self.below(6) } {
            0 => out.push_str(&self.below(100_000).to_string()),
            1 => out.push_str(&format!("\"s{}\"", self.below(1000))),
            2 => out.push_str(["true", "false", "null"][self.below(3) as usize]),
            3 => out.push_str(&format!("{}.{}", self.below(1000), self.below(100))),
            4 => {
                out.push('[');
                for k in 0..self.below(wide + 1) {
                    if k > 0 {
                        out.push(',');
                    }
                    self.value(out, depth - 1, wide);
                }
                out.push(']');
            }
            _ => {
                out.push('{');
                for k in 0..self.below(wide + 1) {
                    if k > 0 {
                        out.push(',');
                    }
                    out.push_str(&format!("\"k{k}\":"));
                    self.value(out, depth - 1, wide);
                }
                out.push('}');
            }
        }
    }
}

/// `n` records: objects of fields, a nested object and an array each (a document of many small subtrees).
pub fn records(n: u32) -> String {
    let mut s = String::from("[");
    for k in 0..n {
        if k > 0 {
            s.push(',');
        }
        s.push_str(&format!("{{\"id\":{k},\"name\":\"n{k}\",\"on\":{},\"pos\":{{\"x\":{},\"y\":{}}},\"tags\":[\"a\",\"b{}\",{}]}}", k % 2 == 0, k * 3, k * 7, k % 13, k % 5));
    }
    s.push(']');
    s
}

/// The test documents: tiny ones (fewer nodes than a chunk), a long array (leaves whose walk steps back over their
/// siblings), records, deep nests, and random values; the larger ones' big subtrees escape their slots' width.
pub fn documents(lang: &Lang) -> Vec<Doc> {
    let mut texts: Vec<(String, String)> = vec![
        ("empty array".into(), "[]".into()),
        ("small object".into(), r#"{"a":[1,{"b":null}],"c":"d"}"#.into()),
        ("numbers".into(), format!("[{}]", (0..5000).map(|k| k.to_string()).collect::<Vec<_>>().join(","))),
        ("records".into(), records(700)),
        ("deep".into(), format!("{}1{}", "[0,".repeat(300), "]".repeat(300))),
        ("deep objects".into(), format!("{}{}", "{\"a\":[1,2,{\"b\":".repeat(120), format!("3{}", "}]}".repeat(120)))),
    ];
    for seed in 1..4 {
        let mut g = Gen::new(seed);
        let mut s = String::new();
        g.value(&mut s, 7, 9);
        texts.push((format!("random {seed}"), format!("[{s},{}]", records(20))));
    }
    texts.into_iter().map(|(name, text)| json_doc(lang, &name, text.as_bytes())).collect()
}
