//! A node's id inside its batch (decision 0724 §1): 32 bits, meaningful only while one document's batch is active,
//! so it never names its document. Since 0677 and 0727 no node id leaves its file's part: a node that leaves is a
//! `Located` (its document's key and its low bits), made by `NodeId::located` alone.
//!
//! The type is branded: it is no integer, so nothing widens it into a 64-bit cell or ors a document's key above it by
//! accident. A shift of one is a shift of its 32 bits, and a shift by 32 or more panics (`<< 32` is how a key was
//! put above an id; it has no meaning for a batch's id).

use core::ops::{Shl, Shr};

/// A node's id in its batch: its tape position (`NodeIds::Tape`), 32 bits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct NodeId(u32);

impl NodeId {
    /// The id of tape position `position`.
    #[inline]
    pub const fn new(position: u32) -> NodeId {
        NodeId(position)
    }
    /// Its tape position.
    #[inline]
    pub const fn position(self) -> u32 {
        self.0
    }
    /// The id a cell holds (a relation's column of nodes): its low 32 bits, which must be all it holds. A cell with
    /// a document's key above them is a `Located`, and is refused here.
    #[inline]
    pub fn of_cell(cell: i64) -> NodeId {
        match u32::try_from(cell) {
            Ok(p) => NodeId(p),
            Err(_) => panic!("a node id is 32 bits: cell {cell:#x} holds more (a located id is `Located`, not a node id)"),
        }
    }
    /// The cell a relation's column of nodes holds: the id, zero-extended.
    #[inline]
    pub const fn cell(self) -> i64 {
        self.0 as i64
    }
    /// The node as it leaves its batch: its document's key and its id (decision 0724 §6).
    #[inline]
    pub const fn located(self, key: crate::located::DocKey) -> crate::located::Located {
        crate::located::Located::new(key.0, self.0)
    }
}

impl Shl<u32> for NodeId {
    type Output = NodeId;
    /// A shift of the id's 32 bits: by 32 or more it panics, never widens.
    #[inline]
    #[track_caller]
    fn shl(self, by: u32) -> NodeId {
        assert!(by < 32, "a node id is 32 bits: `<< {by}` would put it past them (a document's key goes in `Located`, never above a node id)");
        NodeId(self.0 << by)
    }
}

impl Shr<u32> for NodeId {
    type Output = NodeId;
    #[inline]
    #[track_caller]
    fn shr(self, by: u32) -> NodeId {
        assert!(by < 32, "a node id is 32 bits: `>> {by}` would read past them");
        NodeId(self.0 >> by)
    }
}

impl From<NodeId> for u32 {
    #[inline]
    fn from(n: NodeId) -> u32 {
        n.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_node_id_is_32_bits_and_shifts_within_them() {
        let n = NodeId::new(5);
        assert_eq!((n << 3).position(), 40);
        assert_eq!((n << 31).position(), 1 << 31);
        assert_eq!((NodeId::new(40) >> 3).position(), 5);
        assert_eq!(NodeId::of_cell(n.cell()), n);
        assert_eq!(n.located(crate::located::DocKey(7)), crate::located::Located::new(7, 5));
    }

    #[test]
    #[should_panic(expected = "a node id is 32 bits")]
    fn a_shift_by_32_panics() {
        let _ = NodeId::new(1) << 32;
    }

    #[test]
    #[should_panic(expected = "a node id is 32 bits")]
    fn a_located_cell_is_no_node_id() {
        let _ = NodeId::of_cell(crate::located::Located::new(3, 1).0);
    }
}
