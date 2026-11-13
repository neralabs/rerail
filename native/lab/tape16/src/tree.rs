//! A tree as neutral nodes in preorder: what every encoding must give back exactly.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub kind: u16,
    pub field: u8,
    /// Extra 1, error 2, missing 4 (format 15's code flags).
    pub flags: u8,
    /// Supertype memberships, ascending.
    pub classes: Vec<u16>,
    pub start: u64,
    pub end: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tree {
    pub nodes: Vec<Node>,
    /// Each node's subtree, itself counted.
    pub size: Vec<u32>,
    /// Each node's parent (`u32::MAX` for the root).
    pub parent: Vec<u32>,
    pub source_len: u64,
}

pub const NONE: u32 = u32::MAX;

impl Tree {
    /// Parents from sizes, with a stack.
    pub fn parents_from_sizes(size: &[u32]) -> Vec<u32> {
        let mut parent = vec![NONE; size.len()];
        let mut open: Vec<(u32, usize)> = Vec::new();
        for i in 0..size.len() {
            while open.last().is_some_and(|&(_, end)| end <= i) {
                open.pop();
            }
            if let Some(&(p, _)) = open.last() {
                parent[i] = p;
            }
            open.push((i as u32, i + size[i] as usize));
        }
        parent
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn depth(&self) -> Vec<u32> {
        let mut d = vec![0u32; self.len()];
        for i in 1..self.len() {
            d[i] = d[self.parent[i] as usize] + 1;
        }
        d
    }
}
