//! The lexer of strata from the middle (decision 0698, on 0715's passes): a chunk of blocks lexed from its entry, the
//! state of every stratum where it starts (`Entry`), and the chunks lexed one after another, each from the one before's
//! exit (`lex_chunked`), which is the serial lexer's records.
//!
//! Stratum 2 is variant B's: its carries are a few bits, and an f-string is a plain string to it, its interpolations
//! read by the walk inside the string's token. So what crosses a chunk's start is stratum 1's three escape bits,
//! variant B's machines' states and carries, the bracket depth, and the walk's state between two items (where the next
//! starts, whether the line has a token, whether it is a block's header, the indents open). A text B reads wrongly is
//! read serially by variant A (`Chunked::NotB`), as `lex_with` reads it.

use super::{B, Masks, RB, S1, Text, Walk, classes_in};
use super::{Variant, lex_with};
use crate::lex::{Extra, Lexed, Refusal, Tok};

/// The state of the strata where a chunk starts at block `from`: stratum 1's carries into block `from - 2`, variant
/// B's and the depth's into block `from - 1` (the walk reads the depth below its first item, and stratum 2 a string's
/// prefix in the block below its quote), and the walk's at its first item from byte `64 * from` on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    s1: S1,
    rb: RB,
    depth: i8,
    p: usize,
    open: bool,
    header: bool,
    indents: Vec<u32>,
    done: bool,
}

impl Entry {
    /// The text's start.
    pub fn start() -> Entry {
        Entry { s1: S1::default(), rb: RB::default(), depth: 0, p: 0, open: false, header: false, indents: vec![0], done: false }
    }
}

/// The block stratum 1 starts a chunk at `from` from, and the one strata 2 and 3 start it from.
fn firsts(from: usize) -> (usize, usize) {
    let r0 = from.saturating_sub(1);
    (r0.saturating_sub(1), r0)
}

/// A chunk lexed.
#[derive(Debug, PartialEq, Eq)]
pub enum Chunked {
    /// Its records written, and the entry of the chunk after it.
    Lexed(Entry),
    /// Variant B's reading of the text is wrong: it is read by variant A, from its start.
    NotB,
    /// The text is refused.
    Refused(Refusal),
}

/// Blocks `from..to` of `src` lexed from `e`: their items' records after `toks` and `extras` (an item that starts in
/// them read to its end, past them where it runs past), and the entry of the chunk at `to`. The strata are made over
/// a window of the blocks from `from - 2` to one past `to`, and over a longer one where the walk reads past it.
pub fn lex_chunk(src: &[u8], from: usize, to: usize, e: &Entry, toks: &mut Vec<Tok>, extras: &mut Vec<Extra>) -> Chunked {
    let blocks = src.len().div_ceil(64);
    let to = to.min(blocks);
    let (c0, r0) = firsts(from);
    let (c0_next, r0_next) = firsts(to);
    let mut r1 = (to + 1).min(blocks);
    loop {
        // Stratum 1 over `c0..r1 + 1` (stratum 2 reads the block after each of its own), its carries into the next
        // chunk's first kept.
        let mut b: Vec<B> = Vec::with_capacity(r1 + 2 - c0);
        let (mut s1, mut s1_next) = (e.s1, S1::default());
        classes_in(src, c0, (r1 + 1).min(blocks), &mut s1, &mut b, c0_next, &mut s1_next);
        if c0_next >= blocks {
            s1_next = s1;
        }
        if r1 == blocks {
            b.push(B::default());
        }
        let mut t = Text { src, b, first: c0 };
        // Strata 2 and 3 over `r0..r1`, their carries into the next chunk's first kept.
        let (mut rb, mut depth) = (e.rb, e.depth);
        let split = r0_next.clamp(r0, r1);
        if !t.regions_b_in(r0, split, &mut rb) {
            return Chunked::NotB;
        }
        let rb_next = rb;
        if !t.regions_b_in(split, r1, &mut rb) {
            return Chunked::NotB;
        }
        if r1 == blocks && (rb.t0 != 0 || rb.s0 != 0) {
            return Chunked::NotB;
        }
        if let Err(r) = t.depth_in(r0, split, &mut depth) {
            return Chunked::Refused(r);
        }
        let depth_next = depth;
        if let Err(r) = t.depth_in(split, r1, &mut depth) {
            return Chunked::Refused(r);
        }
        // The walk, over the blocks strata 2 and 3 made (and the one past the text's end, where the window reaches it).
        let short = core::cell::Cell::new(false);
        let upto = if r1 == blocks { r1 + 1 } else { r1 };
        let masks = Masks { src, b: &t.b[r0 - c0..upto - c0], first: r0, short: &short };
        let stop = if to >= blocks { usize::MAX } else { 64 * to };
        let mut w = Walk { t: masks, toks: Vec::new(), extras: Vec::new(), p: e.p, depth: 0, indents: e.indents.clone(), inside_interpolated_string: false, outer: None, b: true, b_wrong: false, stop, done: e.done };
        let r = if e.done { Ok((e.open, e.header)) } else { w.module(e.open, e.header) };
        if short.get() {
            assert!(r1 < blocks, "a read below the chunk's window: block {r0} on, from byte {}", e.p);
            r1 = (r0 + 2 * (r1 - r0)).min(blocks);
            continue;
        }
        if w.b_wrong {
            return Chunked::NotB;
        }
        let (open, header) = match r {
            Ok(x) => x,
            Err(r) => return Chunked::Refused(r),
        };
        toks.append(&mut w.toks);
        extras.append(&mut w.extras);
        return Chunked::Lexed(Entry { s1: s1_next, rb: rb_next, depth: depth_next, p: w.p, open, header, indents: w.indents, done: w.done });
    }
}

/// `src` lexed in chunks of `blocks` blocks, each from the entry the one before leaves: the serial lexer's records
/// (variant B's; none where B's reading is wrong), and the chunks' entries.
pub fn lex_chunked(src: &[u8], blocks: usize) -> Option<(Result<Lexed, Refusal>, Vec<Entry>)> {
    let n = src.len().div_ceil(64);
    let (mut toks, mut extras) = (Vec::new(), Vec::new());
    let mut e = Entry::start();
    let mut entries = Vec::new();
    let mut from = 0;
    loop {
        entries.push(e.clone());
        let to = from + blocks.max(1);
        match lex_chunk(src, from, to, &e, &mut toks, &mut extras) {
            Chunked::Lexed(x) => e = x,
            Chunked::NotB => return None,
            Chunked::Refused(r) => return Some((Err(r), entries)),
        }
        if to >= n {
            return Some((Ok(Lexed { toks, extras }), entries));
        }
        from = to;
    }
}

/// The chunked lexer's records against the serial one's (variant B's), at each chunk size: none where B's reading
/// is wrong (then the serial lexer reads it by A). Whether the chunks gave the serial records, as text where not.
pub fn chunked_is_serial(src: &[u8], blocks: usize) -> Result<bool, String> {
    let Some((got, _)) = lex_chunked(src, blocks) else { return Ok(false) };
    let want = lex_with(src, Variant::B);
    let same = match (&got, &want) {
        (Ok(a), Ok(b)) => a.toks.iter().map(|t| (t.kind, t.soft, t.start, t.end)).eq(b.toks.iter().map(|t| (t.kind, t.soft, t.start, t.end))) && a.extras.iter().map(|e| (e.start, e.end, e.comment)).eq(b.extras.iter().map(|e| (e.start, e.end, e.comment))),
        // Where each refuses is the first refusal of its own order of reading; either way the text goes to tree-sitter.
        (Err(_), Err(_)) => true,
        _ => false,
    };
    if same {
        return Ok(true);
    }
    let show = |r: &Result<Lexed, Refusal>| match r {
        Ok(l) => l.text(src),
        Err(r) => format!("refused: {r:?}"),
    };
    let (a, b) = (show(&got), show(&want));
    let k = a.lines().zip(b.lines()).position(|(x, y)| x != y).unwrap_or(0);
    let around = |t: &str| t.lines().skip(k.saturating_sub(2)).take(5).collect::<Vec<_>>().join("\n");
    Err(format!("{blocks} blocks a chunk, {:?}:\n  chunked:\n{}\n  serial:\n{}", String::from_utf8_lossy(src), around(&a), around(&b)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_lexed_from_their_entries_are_the_serial_lexer() {
        let mut texts: Vec<String> = [
            "if a:\n    b\nc\n",
            "x = (1,\n 2)\n",
            "if a:\n    b\n    # c\nd\n",
            "f'a{b!r:>{w}}c'\n",
            "x = 'a # b' # c ( \ny = [1, # ]\n 2]\n",
            "t = '''a\n'' b ( '''\nu = \"\"\"a\\\"\"\" b\"\"\"\n",
            "def f():\n    return [\n        1,\n    ]\n\nclass C:\n    pass\n",
            "x = 1 \\\n  + 2\n",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        // A text across many blocks: each region, bracket, indent and long token crossing a block's edge somewhere.
        let mut long = String::new();
        for k in 0..60 {
            long.push_str(&format!("x{k} = f(a, '{}', \"\"\"q\n{}\"\"\", # c ( {k}\n  [{}])  # e\n", "s".repeat(k), "t".repeat(k * 3), "1, ".repeat(k)));
            long.push_str(&format!("if x{k}:\n{}y = rf'{}{{x{k}}}'\n{}name_{} = 0\n", " ".repeat(k % 9 + 1), "\\\\".repeat(k), " ".repeat(k % 9 + 1), "n".repeat(k * 2)));
        }
        texts.push(long);
        let (mut same, mut not_b) = (0, 0);
        for t in &texts {
            for blocks in [1, 2, 3, 16] {
                match chunked_is_serial(t.as_bytes(), blocks) {
                    Ok(true) => same += 1,
                    Ok(false) => not_b += 1,
                    Err(e) => panic!("{e}"),
                }
            }
        }
        assert_eq!(not_b, 0, "{same} the same");
    }

    #[test]
    fn random_texts_chunked_are_the_serial_lexer() {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut checked = 0;
        for len in [63usize, 64, 65, 130, 400, 1000] {
            for _ in 0..200 {
                let src: Vec<u8> = (0..len)
                    .map(|_| {
                        x ^= x << 13;
                        x ^= x >> 7;
                        x ^= x << 17;
                        b"''''\\\"\"\"\"#\n\n  ab(rf{}x)\n    :"[(x >> 8) as usize % 29]
                    })
                    .collect();
                for blocks in [1, 2, 3] {
                    checked += usize::from(chunked_is_serial(&src, blocks).unwrap_or_else(|e| panic!("{e}")));
                }
            }
        }
        assert!(checked > 0);
    }
}
