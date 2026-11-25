//! A chunk's size, from measures alone, and the same on every run (decision 0435 §4, R4 and R6).
//!
//! The first chunk is `FIRST_NODES`. Each next is up to `GROWTH` times the last, held to what the latest chunk's
//! blocks wrote a node (`rows::Written`), against three quarters of what the limit leaves past the run's start and
//! the rows the chunks filed hold:
//!
//! ```text
//! a chunk of 16,384 nodes wrote 2 MB, from 300 MB of 1,536 MB  ->  131,072 nodes (8 times it; the room holds 7 M)
//! a chunk of 131,072 nodes wrote 30 MB                         ->  262,144 nodes (the caller's most), 8 at once
//! a chunk of 16,384 nodes wrote 160 MB, from 400 MB            ->  ~72,000 nodes (what the room holds)
//! ```
//!
//! **Spread over the chunks at once** (decision 0690). No chunk is more than the corpus's nodes over `SPREAD` times as
//! many chunks as evaluate at once, so a corpus is cut into at least that many: 300 files of 456 K nodes, with a most
//! of 1 M, were 2 chunks (the first's 65,536 nodes, then the rest), one evaluating while the other waited, where 8
//! could have run; they are 7 now, each at the first's size. The corpus's nodes are estimated
//! from the chunks filed (their nodes a file) times the corpus's files, so the bound too is the same on every run.
//!
//! **The same cut on every run** (Q314). Chunk `n`'s size is decided from chunks filed before it, in their order,
//! never from what is filed when the builder reaches it: the builder cut the documents by the size last stored while
//! it worked ahead, so two runs cut different chunks, and the plans made from them (Q88) differed (1 relation inlined
//! in one hogql run, 11 in the next; 83-92 B). While chunks grow, chunk `n + 1` is sized by chunk `n` (one evaluates
//! at a time). Once the size holds, chunk `n + window` is sized by chunk `n`: no more evaluate at once than the
//! plans' window (`plan::PLAN_WINDOW`), so the chunk a size waits for is filed before the builder reaches it.
//!
//! The measure is the chunk's own blocks' bytes, not the process's count, which other chunks' and threads' timing
//! moves. The run's start is the count when the chunks begin, in steps of `BASE_STEP`.

/// The first chunk's nodes (at most the caller's most). At 4,096 nodes hogql's CPU runs retired 92.5-93.7 B against
/// 88.8-92.0 B at 65,536, measured before the cut was the same on every run (Q314).
pub const FIRST_NODES: usize = 65_536;

/// The most a chunk grows past the last, when the room holds it: from the first to 262,144 nodes in one or two steps.
/// Each chunk runs every file-local stratum once, so a climb of small chunks pays that fixed work per step, in every
/// phase's run. Doubling from 4,096 nodes ran 36 chunks on hogql; growth up to 8 times, 20-23 (2-4 B fewer
/// instructions, measured together with another change, so not apart from it).
pub const GROWTH: usize = 8;

/// The run's start is counted in steps of this: a few MB more or less of heap at the start moves no chunk's size.
pub const BASE_STEP: usize = 64 << 20;

/// The chunks a corpus is cut into at least, for each that evaluates at once (decision 0690): two, so the last chunks'
/// tail is half a chunk's work, not one whole.
pub const SPREAD: usize = 2;

/// What a chunk filed measured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Measured {
    /// Its nodes, the bytes its blocks wrote, and of those the bytes held past it (rows it handed on).
    pub nodes: usize,
    pub bytes: usize,
    pub held: usize,
    /// Its files (a host and its guests are one).
    pub files: usize,
}

/// What set a size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum By {
    /// The first chunk's.
    First,
    /// `GROWTH` times the last.
    Grown,
    /// The caller's most.
    Most,
    /// What the room holds.
    Room,
    /// The corpus spread over the chunks at once (`SPREAD`).
    Spread,
}

/// A size decided, and why.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decided {
    /// The chunk it is for, its nodes, and as many as evaluate at once from it.
    pub chunk: usize,
    pub nodes: usize,
    pub at_once: usize,
    pub by: By,
    /// Bytes a node the chunk deciding it wrote, the room left (bytes), and the nodes the room holds at that rate.
    pub per_node: f64,
    pub room: usize,
    pub fit: usize,
    /// The most a chunk is for the corpus to spread over the chunks at once (`SPREAD`), when estimated.
    pub spread: Option<usize>,
}

/// The run's chunk sizes, by the chunks' order.
#[derive(Debug)]
pub struct Schedule {
    first: usize,
    most: usize,
    window: usize,
    at_most: usize,
    /// Three quarters of the limit, less the run's start.
    room: usize,
    /// The sizes decided, by chunk; and from which chunk on growth has stopped.
    sizes: Vec<Option<(usize, By)>>,
    stopped: Option<usize>,
    /// Chunks cut, chunks filed, and the bytes the filed chunks hold.
    cut: usize,
    filed: usize,
    held: usize,
    /// The corpus's files, and the nodes and files of the chunks filed: the corpus's nodes estimated (`SPREAD`).
    files: usize,
    filed_nodes: usize,
    filed_files: usize,
    at_once: usize,
    /// Every chunk after the first of this many nodes, whatever was measured (`RERAIL_CHUNK_NODES`, Q88).
    fixed: Option<usize>,
}

impl Schedule {
    /// Chunks of `first` to `most` nodes, `window` the plans' (`plan::PLAN_WINDOW`), at most `at_most` at once,
    /// against `limit` bytes from `start` (the run's count when the chunks begin), of a corpus of `files` files (none:
    /// 0); `fixed`, every chunk after the first of that many nodes.
    #[allow(clippy::too_many_arguments)]
    pub fn new(first: usize, most: usize, window: usize, at_most: usize, limit: usize, start: usize, files: usize, fixed: Option<usize>) -> Schedule {
        let first = first.min(most).max(1);
        let base = start.div_ceil(BASE_STEP) * BASE_STEP;
        Schedule {
            first,
            most,
            window: window.max(at_most).max(1),
            at_most: at_most.max(1),
            room: (limit / 4 * 3).saturating_sub(base),
            sizes: vec![Some((first, By::First))],
            stopped: None,
            cut: 0,
            filed: 0,
            held: 0,
            files,
            filed_nodes: 0,
            filed_files: 0,
            at_once: 1,
            fixed,
        }
    }

    /// The next chunk's size, if it is decided yet (the builder holds until it is).
    pub fn next_size(&self) -> Option<usize> {
        self.size(self.cut)
    }

    /// Chunk `n`'s size, if decided.
    pub fn size(&self, n: usize) -> Option<usize> {
        if let Some(f) = self.fixed.filter(|_| n > 0) {
            return Some(f);
        }
        if let Some(&Some((nodes, _))) = self.sizes.get(n) {
            return Some(nodes);
        }
        // Past the chunk growth stopped at, and before the window's first decision, the size it stopped at.
        let s = self.stopped?;
        (n >= s && n < s + self.window).then(|| self.sizes[s].expect("the size growth stopped at").0)
    }

    /// The next chunk is cut: its number.
    pub fn cut(&mut self) -> usize {
        self.cut += 1;
        self.cut - 1
    }

    /// Chunks cut, and filed.
    pub fn counts(&self) -> (usize, usize) {
        (self.cut, self.filed)
    }

    /// As many chunks as may evaluate at once now.
    pub fn at_once(&self) -> usize {
        self.at_once
    }

    /// Chunk `n` (the next to be filed, in order) is filed with what it measured: the size it decides.
    pub fn file(&mut self, n: usize, m: Measured) -> Decided {
        assert_eq!(n, self.filed, "chunks are filed in their order");
        self.filed += 1;
        let target = self.size(n).unwrap_or(self.first);
        let room = self.room.saturating_sub(self.held);
        self.held += m.held;
        let per_node = (m.bytes as f64 / m.nodes.max(1) as f64).max(1.0);
        let fit = (room as f64 / per_node) as usize;
        // The corpus's nodes, at the filed chunks' nodes a file, over `SPREAD` times the chunks at once.
        (self.filed_nodes, self.filed_files) = (self.filed_nodes + m.nodes, self.filed_files + m.files);
        let spread = (self.files > 0 && self.filed_files > 0)
            .then(|| (self.filed_nodes as f64 / self.filed_files as f64 * self.files as f64) as usize / (SPREAD * self.at_most));
        // The least of the most, the room and the spread, and what set it: `None` when growth may go past it.
        let bound = |grown: usize| {
            let mut least = (self.most, By::Most);
            for b in [(fit, By::Room), (spread.unwrap_or(usize::MAX), By::Spread)] {
                if b.0 < least.0 {
                    least = b;
                }
            }
            if grown < least.0 { (grown, By::Grown) } else { least }
        };
        let (chunk, (nodes, by)) = match self.stopped {
            None => (n + 1, bound(m.nodes.max(target).saturating_mul(GROWTH))),
            // Growth stopped: the least of the most, what the room holds and the spread.
            Some(_) => (n + self.window, bound(usize::MAX)),
        };
        let nodes = nodes.clamp(self.first, self.most);
        if self.sizes.len() <= chunk {
            self.sizes.resize(chunk + 1, None);
        }
        self.sizes[chunk] = Some((nodes, by));
        if self.stopped.is_none() && by != By::Grown {
            self.stopped = Some(chunk);
        }
        self.at_once = if by == By::Grown { 1 } else { (fit / nodes.max(1)).clamp(1, self.at_most) };
        Decided { chunk, nodes, at_once: self.at_once, by, per_node, room, fit, spread }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: usize = 1 << 20;
    const LIMIT: usize = 1536 * MB;
    const MOST: usize = 262_144;

    fn schedule(start_mb: usize) -> Schedule {
        Schedule::new(16_384, MOST, 8, 8, LIMIT, start_mb * MB, 0, None)
    }

    fn wrote(nodes: usize, mb: usize) -> Measured {
        Measured { nodes, bytes: mb * MB, held: 0, files: 0 }
    }

    /// A cheap chunk is followed by one `GROWTH` times its size, alone (decision 0435 R4).
    #[test]
    fn a_cheap_chunk_grows() {
        let mut s = schedule(300);
        assert_eq!(s.size(0), Some(16_384));
        assert_eq!(s.size(1), None, "undecided until chunk 0 is filed");
        let d = s.file(0, wrote(16_384, 2));
        assert_eq!((d.chunk, d.nodes, d.at_once, d.by), (1, 16_384 * GROWTH, 1, By::Grown));
        assert_eq!(s.size(1), Some(16_384 * GROWTH));
    }

    /// Growth stops at the caller's most, and then as many chunks evaluate at once as the room holds, each sized by
    /// the chunk a window before it.
    #[test]
    fn growth_stops_at_the_most_and_then_chunks_run_beside_each_other() {
        let mut s = schedule(300);
        let d = s.file(0, wrote(16_384, 1));
        assert_eq!(d.by, By::Grown);
        let d = s.file(1, wrote(131_072, 30));
        assert_eq!((d.chunk, d.nodes, d.by), (2, MOST, By::Most));
        assert!(d.at_once > 1 && d.at_once <= 8, "{d:?}");
        for n in 2..10 {
            assert_eq!(s.size(n), Some(MOST), "chunk {n} is sized by growth's stop");
        }
        assert_eq!(s.size(10), None, "chunk 10 is sized by chunk 2");
        let d = s.file(2, wrote(MOST, 60));
        assert_eq!((d.chunk, d.nodes), (10, MOST));
    }

    /// A chunk whose growth a node would not fit `GROWTH` times its size is held to what the room holds, alone.
    #[test]
    fn a_costly_chunk_is_held_to_the_room() {
        let mut s = schedule(400);
        // 10 KB a node; 1,152 MB less 448 MB (the start in 64 MB steps) holds about 72,000 nodes.
        let d = s.file(0, wrote(16_384, 160));
        assert_eq!(d.by, By::Room);
        assert!(d.nodes > 65_000 && d.nodes < 80_000, "{d:?}");
        assert_eq!(d.at_once, 1);
    }

    /// With no room left, the next chunk is the first's size, never smaller; rows the chunks hand on leave less.
    #[test]
    fn no_room_is_the_first_size_and_held_rows_leave_less() {
        let mut s = schedule(1_200);
        assert_eq!(s.file(0, wrote(16_384, 10)).nodes, 16_384);
        let mut s = schedule(300);
        let d = s.file(0, Measured { nodes: 16_384, bytes: 800 * MB, held: 800 * MB, files: 0 });
        let after = s.file(1, wrote(16_384, 1));
        assert_eq!(after.room + 800 * MB, d.room, "{d:?} {after:?}");
    }

    /// A corpus is cut into at least `SPREAD` times as many chunks as evaluate at once (decision 0690, R1): growth
    /// stops at the corpus's nodes, at the filed chunks' nodes a file, over that many, and as many evaluate at once as
    /// the room holds; a corpus whose spread is past the most is cut at the most, as before (R2).
    #[test]
    fn a_corpus_is_spread_over_the_chunks_at_once() {
        // 300 files, the first chunk's 16,384 nodes of 10: about 491,000 nodes, spread over 16 chunks.
        let mut s = Schedule::new(16_384, 1 << 20, 8, 8, LIMIT, 300 * MB, 300, None);
        let d = s.file(0, Measured { nodes: 16_384, bytes: MB, held: 0, files: 10 });
        assert_eq!((d.by, d.nodes, d.spread), (By::Spread, 30_720, Some(30_720)), "{d:?}");
        assert_eq!(d.at_once, 8, "the room holds 8 at once: {d:?}");
        for n in 1..9 {
            assert_eq!(s.size(n), Some(30_720), "chunk {n} is sized by the spread");
        }
        // PostHog's 29,885 files at the same nodes a file: about 49 M nodes, past the most.
        let mut s = Schedule::new(16_384, MOST, 8, 8, LIMIT, 300 * MB, 29_885, None);
        let d = s.file(0, Measured { nodes: 16_384, bytes: MB, held: 0, files: 10 });
        assert_eq!((d.by, d.nodes), (By::Grown, 16_384 * GROWTH), "{d:?}");
        let d = s.file(1, Measured { nodes: 16_384 * GROWTH, bytes: 8 * MB, held: 0, files: 80 });
        assert_eq!((d.by, d.nodes), (By::Most, MOST), "{d:?}");
    }

    /// The spread is never below the first chunk's nodes: a corpus smaller than `SPREAD` times the chunks at once of
    /// them is cut into chunks of the first's size.
    #[test]
    fn the_spread_is_never_below_the_first_size() {
        let mut s = Schedule::new(16_384, MOST, 8, 8, LIMIT, 300 * MB, 20, None);
        let d = s.file(0, Measured { nodes: 16_384, bytes: MB, held: 0, files: 10 });
        assert_eq!((d.by, d.nodes), (By::Spread, 16_384), "{d:?}");
    }

    /// The sizes are a function of the chunks' measures in their order (R6): filed at whatever time, interleaved
    /// with the builder's cuts however the threads ran, the same chunks get the same sizes.
    #[test]
    fn the_sizes_are_the_same_whatever_the_timing() {
        let measures = [wrote(16_384, 1), wrote(131_072, 20), wrote(MOST, 50), wrote(MOST, 400), wrote(MOST, 40), wrote(MOST, 40)];
        let run = |cut_ahead: usize| {
            let mut s = schedule(300);
            let mut sizes = Vec::new();
            for (n, m) in measures.iter().enumerate() {
                // The builder cuts as far as the decided sizes let it, up to `cut_ahead` beyond what is filed.
                while s.counts().0 < n + 1 + cut_ahead && s.next_size().is_some() {
                    sizes.push(s.next_size().unwrap());
                    s.cut();
                }
                s.file(n, *m);
            }
            sizes.truncate(measures.len());
            sizes
        };
        assert_eq!(run(0), run(3));
        assert_eq!(run(0), run(7));
        assert_eq!(run(0)[..3], [16_384, 131_072, MOST]);
    }
}
