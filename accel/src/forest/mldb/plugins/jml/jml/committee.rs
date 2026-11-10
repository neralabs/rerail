//! committee.h and committee.cc's prediction, in Rust.
//!
//! Jeremy Barnes, 15 March 2006. Copyright (c) 2006 Jeremy Barnes. This file is part of MLDB. Copyright 2015 mldb.ai
//! inc. Apache-2.0 (see the LICENSE at `accel/src/forest/mldb/`).
//!
//! A committee of classifiers: the prediction is the bias plus each classifier's prediction times its weight
//! (classifiers of weight zero skipped). The random forest's committee is of decision trees, so the members here
//! are `DecisionTree`s (upstream: any `Classifier_Impl`); the optimized predict, explanation, serialization and
//! printing stay in the .cc.

use super::decision_tree::{DecisionTree, OutputEncoding};
use super::tree::Distribution;

#[derive(Clone, Debug, Default)]
pub struct Committee {
    pub classifiers: Vec<DecisionTree>,
    /// Weights of each classifier.
    pub weights: Vec<f32>,
    /// Bias to add to each label.
    pub bias: Distribution,
    /// What type of output we produce.
    pub encoding: OutputEncoding,
}

impl Committee {
    pub fn new(label_count: usize) -> Self {
        Committee { bias: vec![0.0; label_count], ..Default::default() }
    }

    pub fn add(&mut self, classifier: DecisionTree, weight: f32) {
        self.classifiers.push(classifier);
        self.weights.push(weight);
    }

    /// Predict all classes, each member predicting with `predict`.
    pub fn predict(&self, predict: &dyn Fn(&DecisionTree) -> Distribution) -> Distribution {
        let mut result: Vec<f64> = self.bias.iter().map(|&b| b as f64).collect();
        for (c, &w) in self.classifiers.iter().zip(&self.weights) {
            if w == 0.0 {
                continue;
            }
            for (r, p) in result.iter_mut().zip(predict(c)) {
                *r += w as f64 * p as f64;
            }
        }
        result.into_iter().map(|v| v as f32).collect()
    }

    /// Predict the score for a single class.
    pub fn predict_label(&self, label: usize, predict: &dyn Fn(&DecisionTree) -> Distribution) -> Result<f32, String> {
        if label >= self.bias.len() {
            return Err("Committee::predict(): invalid label".into());
        }
        Ok(self.predict(predict)[label])
    }
}
