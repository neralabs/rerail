//! A chunk's size, from measures alone (decision 0435 §4).
//!
//! The first chunk is small. Each next is up to `GROWTH` times the last, held to what the latest chunk added a node to
//! what the run wrote at its peak (`rerail_accel::budget::take_most`), against three quarters of what the limit leaves.
//! Nothing comes from a prior or from the run's worst measure:
//!
//! ```text
//! a chunk of 16,384 nodes added 2 MB, from 300 MB of 1,536 MB  ->  131,072 nodes (8 times it; the room holds 7 M)
//! a chunk of 65,536 nodes added 60 MB                          ->  262,144 nodes (the caller's most), 3 at once
//! a chunk of 100,000 nodes added 1 GB, from 400 MB            ->  ~75,000 nodes (what the room holds)
//! ```
//!
//! While chunks grow they evaluate one at a time, so a chunk's growth is its own. Once the size stops growing (at
//! the caller's most, or held by the room), as many evaluate at once as the room holds at that size.

/// The first chunk's nodes (at most the caller's most). Its strata's plans are made from it: at 4,096 nodes they were
/// planned from a sample unlike the run's, with twice the stratum variants and 2-3 B more of hogql's instructions on
/// the CPU than at 65,536.
pub const FIRST_NODES: usize = 65_536;

/// The most a chunk grows past the last, when the room holds it: from the first to 262,144 nodes in two steps. At twice,
/// six chunks climbed there in every phase's run, and their per-chunk work cost 8 B of hogql's instructions.
pub const GROWTH: usize = 8;

/// What the chunk just evaluated measured.
#[derive(Clone, Copy, Debug)]
pub struct Measured {
    /// Its nodes, and the chunks evaluating beside it, itself included (the count's growth was theirs together).
    pub nodes: usize,
    pub at_once: usize,
    /// What the run wrote when it began, and when it ended (bytes).
    pub start: usize,
    pub end: usize,
}

/// The next chunks' size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Next {
    pub nodes: usize,
    pub at_once: usize,
    /// Bytes a node the chunk added, and the nodes the room holds at that rate.
    pub per_node: f64,
    pub fit: usize,
    /// What set the size.
    pub by: By,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum By {
    /// `GROWTH` times the last.
    Grown,
    /// The caller's most.
    Most,
    /// What the room holds.
    Room,
}

/// The next chunks' size after `m`, against `limit` bytes, between `first` and `most` nodes, at most `at_most` at once.
pub fn next(m: &Measured, limit: usize, first: usize, most: usize, at_most: usize) -> Next {
    let added = m.end.saturating_sub(m.start);
    // A chunk that added under a byte a node (its pages reused) is counted at a byte a node.
    let per_node = (added as f64 / (m.nodes.max(1) * m.at_once.max(1)) as f64).max(1.0);
    let room = (limit / 4 * 3).saturating_sub(m.start);
    let fit = (room as f64 / per_node) as usize;
    let grown = m.nodes.saturating_mul(GROWTH);
    let (nodes, by) = if most <= grown && most <= fit {
        (most, By::Most)
    } else if grown <= fit {
        (grown, By::Grown)
    } else {
        (fit, By::Room)
    };
    let nodes = nodes.clamp(first.min(most), most);
    let at_once = if by == By::Grown { 1 } else { (fit / nodes.max(1)).clamp(1, at_most.max(1)) };
    Next { nodes, at_once, per_node, fit, by }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: usize = 1 << 20;
    const LIMIT: usize = 1536 * MB;

    fn after(nodes: usize, start_mb: usize, added_mb: usize) -> Next {
        next(&Measured { nodes, at_once: 1, start: start_mb * MB, end: (start_mb + added_mb) * MB }, LIMIT, FIRST_NODES, 262_144, 8)
    }

    /// A cheap chunk is followed by one `GROWTH` times its size, alone (decision 0435 R4).
    #[test]
    fn a_cheap_chunk_grows() {
        let n = after(16_384, 300, 2);
        assert_eq!((n.nodes, n.at_once, n.by), (16_384 * GROWTH, 1, By::Grown));
    }

    /// Growth stops at the caller's most, and then as many chunks evaluate at once as the room holds.
    #[test]
    fn growth_stops_at_the_most_and_then_chunks_run_beside_each_other() {
        let n = after(65_536, 300, 15);
        assert_eq!((n.nodes, n.by), (262_144, By::Most));
        assert!(n.at_once > 1 && n.at_once <= 8, "{n:?}");
        assert_eq!(after(262_144, 300, 60).nodes, 262_144, "never past the most");
    }

    /// A chunk whose growth a node would not fit twice its size is held to what the room holds, alone.
    #[test]
    fn a_costly_chunk_is_held_to_the_room() {
        // 10 KB a node; 1,152 MB less 400 MB of room holds about 78,000 nodes.
        let n = after(100_000, 400, 977);
        assert_eq!(n.by, By::Room);
        assert!(n.nodes > 70_000 && n.nodes < 85_000, "{n:?}");
        assert_eq!(n.at_once, 1);
    }

    /// With no room left, the next chunk is the first's size, never smaller; growth shared by chunks at once is
    /// divided among them.
    #[test]
    fn no_room_is_the_first_size_and_growth_is_shared() {
        assert_eq!(after(50_000, 1_200, 10).nodes, FIRST_NODES);
        let shared = next(&Measured { nodes: 10_000, at_once: 4, start: 300 * MB, end: 340 * MB }, LIMIT, FIRST_NODES, 262_144, 8);
        assert!((shared.per_node - 1048.576).abs() < 1.0, "{shared:?}");
    }
}
