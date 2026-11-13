//! Immutable named ordering for sibling and descendant range positions.
use super::Error;
use super::packed_batch::PackedBatch;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct NamedOrderRecord {
    pub sibling: u32,
    pub preorder: u32,
}

pub(super) fn build(batch: &PackedBatch) -> Result<Vec<NamedOrderRecord>, Error> {
    let size = batch
        .nodes
        .len()
        .checked_add(1)
        .ok_or(Error::BackendCapacity)?;
    let mut rows = vec![
        NamedOrderRecord {
            sibling: u32::MAX,
            preorder: 0
        };
        size
    ];
    let mut named = 0u32;
    for (id, node) in batch.nodes.iter().enumerate() {
        rows[id].preorder = named;
        named = named
            .checked_add(u32::from(node.flags & 1 != 0))
            .ok_or(Error::BackendCapacity)?;
        if node.parent == u32::MAX {
            rows[id].sibling = 0;
        }
        let start = node.child_start as usize;
        let end = start
            .checked_add(node.child_count as usize)
            .ok_or(Error::InvalidAnchor)?;
        let mut rank = 0u32;
        for &child in batch.children.get(start..end).ok_or(Error::InvalidAnchor)? {
            let record = batch
                .nodes
                .get(child as usize)
                .ok_or(Error::InvalidAnchor)?;
            if record.parent as usize != id || rows[child as usize].sibling != u32::MAX {
                return Err(Error::InvalidAnchor);
            }
            rows[child as usize].sibling = rank;
            rank = rank
                .checked_add(u32::from(record.flags & 1 != 0))
                .ok_or(Error::BackendCapacity)?;
        }
    }
    if rows[..batch.nodes.len()]
        .iter()
        .any(|row| row.sibling == u32::MAX)
    {
        return Err(Error::InvalidAnchor);
    }
    rows[batch.nodes.len()] = NamedOrderRecord {
        sibling: 0,
        preorder: named,
    };
    Ok(rows)
}
