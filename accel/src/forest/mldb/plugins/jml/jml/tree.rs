//! tree.h and tree.cc, in Rust.
//!
//! Jeremy Barnes, 24 April 2006. Copyright (c) 2006 Jeremy Barnes. This file is part of MLDB. Copyright 2015 mldb.ai
//! inc. Apache-2.0 (see the LICENSE at `accel/src/forest/mldb/`).
//!
//! Class to hold and manage a tree. The tree owns the memory of its nodes and leaves (boost object pools upstream,
//! two vectors here), which keeps everything close together in memory. A `Ptr` is an index into them.
//!
//! `serialize`/`reconstitute` (JML's `DB::Store` and `Feature_Space`) are not brought over.

use super::split::Split;

/// `distribution<float>`: probabilities for each class, or means for regression.
pub type Distribution = Vec<f32>;

/// Pointer that points to either a leaf or a node (or to nothing).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Ptr {
    #[default]
    Null,
    Node(u32),
    Leaf(u32),
}

impl Ptr {
    pub fn is_node(&self) -> bool {
        matches!(self, Ptr::Node(_))
    }
    pub fn is_leaf(&self) -> bool {
        matches!(self, Ptr::Leaf(_))
    }
    pub fn is_some(&self) -> bool {
        !matches!(self, Ptr::Null)
    }
}

/// What nodes and leaves have in common.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Base {
    /// Probs for class, means for regress.
    pub pred: Distribution,
    /// Num examples at this point.
    pub examples: f32,
}

impl Base {
    /// Estimate of the amount of allocated memory.
    pub fn memusage(&self) -> usize {
        std::mem::size_of::<Self>() + self.pred.capacity() * 4
    }
}

/// Leaf node. Contains a prediction for each class.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Leaf {
    pub base: Base,
}

/// Node. Contains child pointers and a test.
#[derive(Clone, Debug)]
pub struct Node {
    pub base: Base,
    /// Value to split on.
    pub split: Split,
    /// Z score for this split.
    pub z: f32,
    /// Result if true.
    pub child_true: Ptr,
    /// Result if false.
    pub child_false: Ptr,
    /// Result if missing.
    pub child_missing: Ptr,
    /// The bucket the split is at, among its feature's buckets (`split_bucket < 0` for none): the random forest
    /// trainer splits on buckets, and its prediction routes rows by them (an addition to upstream's node, whose
    /// split is only the value).
    pub split_bucket: i32,
    /// A categorical split's buckets on the true side (CART's subset split, a bit per bucket; empty for none): the
    /// trainer's categorical splits are subsets of their feature's buckets.
    pub split_subset: Vec<u32>,
}

impl Default for Node {
    fn default() -> Self {
        Node { base: Base::default(), split: Split::default(), z: 0.0, child_true: Ptr::Null, child_false: Ptr::Null, child_missing: Ptr::Null, split_bucket: -1, split_subset: Vec::new() }
    }
}

impl Node {
    pub fn memusage(&self) -> usize {
        self.base.memusage() + std::mem::size_of::<Self>() - std::mem::size_of::<Base>()
    }
}

/// Tree structure. Contains the entire representation of a tree, as well as memory management machinery.
#[derive(Clone, Debug, Default)]
pub struct Tree {
    /// The root node of the tree.
    pub root: Ptr,
    nodes: Vec<Node>,
    leafs: Vec<Leaf>,
}

impl Tree {
    pub fn new() -> Self {
        Tree::default()
    }

    pub fn clear(&mut self) {
        self.root = Ptr::Null;
        self.nodes.clear();
        self.leafs.clear();
    }

    /// Allocate a new tree node.
    pub fn new_node(&mut self) -> Ptr {
        self.nodes.push(Node::default());
        Ptr::Node(self.nodes.len() as u32 - 1)
    }

    /// Allocate a new tree leaf.
    pub fn new_leaf_with(&mut self, dist: Distribution, examples: f32) -> Ptr {
        self.leafs.push(Leaf { base: Base { pred: dist, examples } });
        Ptr::Leaf(self.leafs.len() as u32 - 1)
    }

    /// Allocate a new tree leaf.
    pub fn new_leaf(&mut self) -> Ptr {
        self.new_leaf_with(Distribution::new(), 0.0)
    }

    pub fn node(&self, p: Ptr) -> Option<&Node> {
        match p {
            Ptr::Node(i) => self.nodes.get(i as usize),
            _ => None,
        }
    }

    pub fn node_mut(&mut self, p: Ptr) -> Option<&mut Node> {
        match p {
            Ptr::Node(i) => self.nodes.get_mut(i as usize),
            _ => None,
        }
    }

    pub fn leaf(&self, p: Ptr) -> Option<&Leaf> {
        match p {
            Ptr::Leaf(i) => self.leafs.get(i as usize),
            _ => None,
        }
    }

    pub fn leaf_mut(&mut self, p: Ptr) -> Option<&mut Leaf> {
        match p {
            Ptr::Leaf(i) => self.leafs.get_mut(i as usize),
            _ => None,
        }
    }

    /// The node's or leaf's base.
    pub fn base(&self, p: Ptr) -> Option<&Base> {
        match p {
            Ptr::Node(_) => self.node(p).map(|n| &n.base),
            Ptr::Leaf(_) => self.leaf(p).map(|l| &l.base),
            Ptr::Null => None,
        }
    }

    pub fn base_mut(&mut self, p: Ptr) -> Option<&mut Base> {
        match p {
            Ptr::Node(i) => self.nodes.get_mut(i as usize).map(|n| &mut n.base),
            Ptr::Leaf(i) => self.leafs.get_mut(i as usize).map(|l| &mut l.base),
            Ptr::Null => None,
        }
    }

    /// `Ptr::examples()`.
    pub fn examples(&self, p: Ptr) -> f32 {
        self.base(p).map_or(0.0, |b| b.examples)
    }

    /// `Ptr::pred()`.
    pub fn pred(&self, p: Ptr) -> &[f32] {
        self.base(p).map_or(&[], |b| b.pred.as_slice())
    }

    /// Estimate of the amount of allocated memory.
    pub fn memusage(&self) -> usize {
        std::mem::size_of::<Self>() + self.nodes.iter().map(Node::memusage).sum::<usize>() + self.leafs.iter().map(|l| l.base.memusage()).sum::<usize>()
    }

    /// `tree_copy_recursive`: copies the subtree at `from` in `src` into this tree; its new pointer.
    pub fn copy_recursive(&mut self, src: &Tree, from: Ptr) -> Ptr {
        match from {
            Ptr::Node(_) => {
                let n = src.node(from).expect("node").clone();
                let t = self.copy_recursive(src, n.child_true);
                let f = self.copy_recursive(src, n.child_false);
                let m = self.copy_recursive(src, n.child_missing);
                let p = self.new_node();
                *self.node_mut(p).unwrap() = Node { child_true: t, child_false: f, child_missing: m, ..n };
                p
            }
            Ptr::Leaf(_) => {
                let l = src.leaf(from).expect("leaf");
                self.new_leaf_with(l.base.pred.clone(), l.base.examples)
            }
            Ptr::Null => Ptr::Null,
        }
    }
}

/// `print_outcome`.
pub fn print_outcome(outcome: &Leaf) -> String {
    let labels: Vec<String> = outcome.base.pred.iter().map(|p| format!("{p:.3}")).collect();
    format!(" (weight = {:.2}) {}", outcome.base.examples, labels.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_build_and_copy() {
        let mut t = Tree::new();
        let l = t.new_leaf_with(vec![0.25, 0.75], 4.0);
        let r = t.new_leaf_with(vec![1.0, 0.0], 2.0);
        let n = t.new_node();
        {
            let node = t.node_mut(n).unwrap();
            node.child_true = l;
            node.child_false = r;
            node.base.examples = 6.0;
        }
        t.root = n;
        let mut u = Tree::new();
        u.root = u.copy_recursive(&t, t.root);
        let un = u.node(u.root).unwrap();
        assert_eq!(u.examples(un.child_true), 4.0);
        assert_eq!(u.pred(un.child_false), &[1.0, 0.0]);
        assert_eq!(u.examples(Ptr::Null), 0.0);
        assert!(print_outcome(u.leaf(un.child_true).unwrap()).contains("weight = 4.00"));
    }
}
