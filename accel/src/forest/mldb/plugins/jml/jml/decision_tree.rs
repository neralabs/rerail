//! decision_tree.h and decision_tree.cc's prediction, in Rust.
//!
//! Jeremy Barnes, 22 March 2004. Copyright (c) 2004 Jeremy Barnes. This file is part of MLDB. Copyright 2015 mldb.ai
//! inc. Apache-2.0 (see the LICENSE at `accel/src/forest/mldb/`).
//!
//! Decision tree classifier: a tree of arbitrary depth (CART, without pruning). Prediction walks the tree from the
//! root; at a node, the split says how much of the example goes each way (true, false, missing), and each leaf
//! reached adds its distribution, weighted (`predict_recursive_impl`).
//!
//! Two ways to say where an example goes: by its values (`predict_values`, upstream's: `Split::apply` on the
//! feature's value), or by its buckets (`predict_buckets`: the bucket the trainer split at, `Node::split_bucket`,
//! against the example's bucket in that feature), which routes every example as training did. The training
//! (`Decision_Tree_Generator`), printing, explanation, serialization and the optimized (dense) predict stay in the
//! .cc; the optimized predict differs only in how it reads the features.
//!
//! Departure: a child is reached with the weight it is reached with times the split's weight for it (upstream
//! passed only the split's weight, dropping the weight already reached; the two agree for the 0/1 weights of a
//! single value).

use super::feature::Feature;
use super::split::{Split, MISSING};
use super::tree::{Distribution, Ptr, Tree};

/// JML's `Output_Encoding` (classifier.h).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputEncoding {
    /// A probability between 0 and 1.
    #[default]
    Prob,
    /// A value between -inf and inf.
    PmInf,
    /// A value between -1 and 1.
    PmOne,
}

/// A decision tree as a classifier.
#[derive(Clone, Debug, Default)]
pub struct DecisionTree {
    /// The tree we have learned.
    pub tree: Tree,
    /// How the outputs are represented.
    pub encoding: OutputEncoding,
    /// The number of labels (the length of each leaf's distribution).
    pub label_count: usize,
}

impl DecisionTree {
    pub fn new(tree: Tree, label_count: usize) -> Self {
        DecisionTree { tree, encoding: OutputEncoding::Prob, label_count }
    }

    /// The prediction, where `weights(split)` gives how much of the example goes true, false and missing
    /// (`Split::Weights`).
    pub fn predict_with(&self, weights: &dyn Fn(&Split, &super::tree::Node) -> [f32; 3]) -> Distribution {
        let mut accum = vec![0f64; self.label_count];
        self.predict_recursive_impl(weights, &mut accum, self.tree.root, 1.0);
        accum.into_iter().map(|v| v as f32).collect()
    }

    /// The prediction with `value(feature)` the example's values (NaN for missing), as upstream (`Split::apply`).
    pub fn predict_values(&self, value: &dyn Fn(&Feature) -> f32) -> Distribution {
        self.predict_with(&|split, _| {
            let mut w = [0f32; 3];
            w[split.apply_value(value(split.feature())) as usize] = 1.0;
            w
        })
    }

    /// The prediction with `bucket(feature)` the example's buckets: true where training put the bucket on the left
    /// of the node's split (an ordinal split's lower buckets, a categorical split's own bucket).
    pub fn predict_buckets(&self, bucket: &dyn Fn(&Feature) -> u32) -> Distribution {
        self.predict_with(&|split, node| {
            let b = bucket(split.feature()) as i64;
            let s = node.split_bucket as i64;
            let left = if split.op() == super::split::Op::Less { b < s } else { b == s };
            if left { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] }
        })
    }

    fn predict_recursive_impl(&self, weights: &dyn Fn(&Split, &super::tree::Node) -> [f32; 3], accum: &mut [f64], ptr: Ptr, weight: f64) {
        match ptr {
            Ptr::Null => {}
            Ptr::Leaf(_) => {
                for (a, &p) in accum.iter_mut().zip(self.tree.pred(ptr)) {
                    *a += weight * p as f64;
                }
            }
            Ptr::Node(_) => {
                let node = self.tree.node(ptr).unwrap();
                let w = weights(&node.split, node);
                // Go down all of the edges that we need to for this example.
                for (child, w) in [(node.child_true, w[1]), (node.child_false, w[0]), (node.child_missing, w[MISSING as usize])] {
                    if w > 0.0 {
                        self.predict_recursive_impl(weights, accum, child, weight * w as f64);
                    }
                }
            }
        }
    }
}
