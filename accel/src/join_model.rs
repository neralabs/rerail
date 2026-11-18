//! Join steps' fanouts, learned from what runs measured (decision 0354). The planner prices each step of a join's
//! order by a rule of thumb (its `basis`: postings, field statistics, containment, membership…), and some are wrong by
//! hundreds of times. Each step the engine ran is a sample ([`StepSample`]) of the data it read (the relation, the
//! columns bound, their distinct values, the relation's rows) and of what it did: its rows in and out. A regression
//! forest (`rerail-forest`, as the cost model's) learns how far the statistics' estimate of it is from what it made.
//!
//! **The target is the measured rows out over the statistics' estimate of them**: ln((rows out + 1) / (estimated
//! fanout × rows in + 1)). The label is what the run measured (the user, 2026-11-17: "the training data should be
//! from model runs, not from the planner"); the estimate is the statistics' (a constant's frequency, a key's
//! postings), a feature (the prior) and what the target is a ratio to. A forest predicts piecewise constants: asked
//! the measured fanout itself, with the estimate as a feature, it could not follow the estimate across six orders
//! of magnitude, and missed kind-frequency steps by ×14 where the statistics missed by ×4.5 (Q82). Each step is
//! also described by the data it reads: the relation, the columns bound, their distinct values, the relation's rows.
//!
//! **In shadow.** The model is trained and compared, not used: no plan changes.

use crate::cost_model::Errors;

/// A step a join ran: what the planner knew when it priced it, and what it made.
#[derive(Clone, Debug, PartialEq)]
pub struct StepSample {
    /// The rule that priced the step's rows (`StepPlan::basis`, its first words): "postings", "observed"…; "none" for
    /// a filter. With `est_fan`, the statistics' estimate (the prior): features, never the target. Without them
    /// (Q82, across corpora) the model knew no constant's frequency, and missed kind-frequency steps by ×37 to ×45
    /// where the planner's statistics missed by ×3.6.
    pub basis: String,
    /// How the step reads (`StepPlan::access`, its first word): "lookup", "scan", "membership", "filter"…
    pub access: String,
    /// What it reads (`StepPlan::source`): "input", "derived", "text test"…; "none" for a filter.
    pub source: String,
    /// The columns bound when it reads (constants and bound variables).
    pub bound: u32,
    /// Its place in the order (from 1: the first step, the delta or a seed's anchor, is not a sample), and the
    /// order's steps.
    pub step: u32,
    pub steps: u32,
    /// The planner's rows into the step, and its rows out over them.
    pub est_in: f64,
    pub est_fan: f64,
    /// The rows of the relation it reads (0 for a filter), and whether that relation is of the component being
    /// solved (its rows still coming).
    pub card: f64,
    pub solving: bool,
    /// Whether the line is a seed (else a probe, led by its delta).
    pub seed: bool,
    /// Which literal of which rule: the rule's digest and the literal's place in its body. Kept, not a feature:
    /// across corpora (corpus0 and hogql, Q82) the model asked a new corpus missed as much with it as without
    /// (×1.83 against ×1.81; ×1.64 against ×1.59), and a new rule's literals are unseen.
    pub literal: String,
    /// The most distinct values of a bound column of the relation it reads (0: none bound, or not measured): how many
    /// keys a probe can hit, of which a membership's or a lookup's share follows.
    pub key_ndv: f64,
    /// The rows the step read and made.
    pub actual_in: f64,
    pub actual_out: f64,
    /// The relation read (its name, from the literal; empty for a filter), the columns bound when it reads (bit `c`:
    /// column `c`, constants and bound variables), and its columns.
    pub relation: String,
    pub bound_mask: u64,
    pub cols: u32,
    /// Where the sample came from, not a feature: "ran" (the order a line ran throughout), "replaced" (an order of a
    /// line that ran more than one, decision 0206's handoffs: the orders the planner did not keep).
    pub origin: String,
}

impl StepSample {
    /// The planner's rows out for the rows the step read.
    pub fn est_out(&self) -> f64 {
        self.est_fan.max(0.0) * self.actual_in
    }

    /// The model's target: the measured rows out over the statistics' estimate for the rows the step read, on the
    /// log scale.
    pub fn target(&self) -> f64 {
        ((self.actual_out + 1.0) / (self.est_out() + 1.0)).ln().clamp(-30.0, 30.0)
    }

    /// The rows out a predicted target `t` gives for the rows the step read.
    pub fn out_of(&self, t: f64) -> f64 {
        ((self.est_out() + 1.0) * t.exp() - 1.0).max(0.0)
    }

    /// The rows of work the step stands for, measured: what it read and made (training's weight).
    pub fn work(&self) -> f64 {
        self.actual_in + self.actual_out
    }
}

/// The features, in the feature space's column order.
/// Facts of the data the step reads and of how it reads it: the relation, which columns are bound and how many there
/// are, the relation's rows, the bound key's distinct values and the rows per key they leave, whether the relation
/// is still being solved, whether the line is a seed; and the statistics' estimate of its fanout with the rule that
/// made it (the prior: a constant's frequency, a key's postings, which no other feature carries); and the step's
/// place in its order and the rows estimated into it (without them, across corpora, ×1.87 and ×1.83 against ×1.83
/// and ×1.64 with: Q82).
pub const STEP_FEATURES: [&str; 16] = ["relation", "access", "source", "bound", "bound_mask", "cols", "card", "key_ndv", "rows_per_key", "solving", "seed", "basis", "prior", "est_in", "step", "steps"];

/// The planner's estimates and the model's corrected ones, on the held-out steps.
#[derive(Clone, Copy, Debug, Default)]
pub struct JoinComparison {
    pub trained_on: usize,
    pub held_out: usize,
    /// Per step, unweighted: the planner's rows out against what the step made, and the model's.
    /// Weighted, each step by its rows read and the most of what it made and what either predicted: an overestimate
    /// misleads a plan as much as an underestimate (`up_fn_of` on hogql, planned at 1,000 times its work).
    pub planner: Errors,
    pub model: Errors,
    /// Weighted by each step's rows of work: the geometric mean miss, as a factor (1.0: none).
    pub planner_weighted: f64,
    pub model_weighted: f64,
}

impl JoinComparison {
    /// Whether the corrected estimates miss less, weighted by work and per step at the median.
    pub fn model_wins(&self) -> bool {
        self.held_out > 0 && self.model_weighted < self.planner_weighted && self.model.median_log <= self.planner.median_log
    }
}

impl std::fmt::Display for JoinComparison {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "trained on {} steps, {} held out", self.trained_on, self.held_out)?;
        writeln!(f, "  planner: {}; weighted by work ×{:.2}", self.planner, self.planner_weighted)?;
        write!(f, "  model:   {}; weighted by work ×{:.2}", self.model, self.model_weighted)
    }
}

/// The miss of `pairs` (predicted, made) weighted by `w`, as a factor.
fn weighted_miss(pairs: &[(f64, f64)], w: &[f64]) -> f64 {
    let (mut s, mut t) = (0.0, 0.0);
    for (&(p, a), &w) in pairs.iter().zip(w) {
        let e = (p.max(1e-9) / a.max(1e-9)).ln();
        if e.is_finite() {
            s += w * e.abs();
            t += w;
        }
    }
    if t > 0.0 { (s / t).exp() } else { 1.0 }
}

/// The comparison of the model's fanouts `predicted` (on the target's scale, one per step of `held`) with the
/// planner's estimates.
pub fn compared(held: &[StepSample], predicted: &[f64], trained_on: usize) -> JoinComparison {
    let planner: Vec<(f64, f64)> = held.iter().map(|s| (s.est_out() + 1.0, s.actual_out + 1.0)).collect();
    let model: Vec<(f64, f64)> = held.iter().zip(predicted).map(|(s, &t)| (s.out_of(t) + 1.0, s.actual_out + 1.0)).collect();
    let w: Vec<f64> = held.iter().zip(predicted).map(|(s, &t)| s.actual_in + s.actual_out.max(s.est_out()).max(s.out_of(t))).collect();
    JoinComparison {
        trained_on,
        held_out: held.len(),
        planner: Errors::of(&planner),
        model: Errors::of(&model),
        planner_weighted: weighted_miss(&planner, &w),
        model_weighted: weighted_miss(&model, &w),
    }
}

#[cfg(target_os = "macos")]
pub use on::{JoinModel, compare, compare_on};

#[cfg(target_os = "macos")]
mod on {
    use super::{JoinComparison, STEP_FEATURES, StepSample};
    use rerail_forest::bucket::CellValue;
    use rerail_forest::dataset_feature_space::{DatasetFeatureSpace, FeatureType};
    use rerail_forest::randomforest_predict::{BucketRules, FlatForest};
    use rerail_forest::randomforest_procedure::{RandomForestProcedureConfig, TargetScale, train_regression_flat};
    use std::sync::Arc;

    /// A trained model: a regression forest over [`STEP_FEATURES`], predicting a step's rows out over the statistics' estimate.
    pub struct JoinModel {
        rules: BucketRules,
        forest: FlatForest,
        scale: TargetScale,
        pub trained_on: usize,
    }

    impl std::fmt::Debug for JoinModel {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "JoinModel {{ trained on {}, {} nodes }}", self.trained_on, self.forest.nodes.len())
        }
    }

    fn ln1(v: f64) -> f64 {
        (v.max(0.0) + 1.0).ln()
    }

    fn cells(samples: &[StepSample]) -> Vec<Vec<CellValue>> {
        let mut c: Vec<Vec<CellValue>> = vec![Vec::with_capacity(samples.len()); STEP_FEATURES.len()];
        for s in samples {
            c[0].push(CellValue::String(s.relation.clone()));
            c[1].push(CellValue::String(s.access.clone()));
            c[2].push(CellValue::String(s.source.clone()));
            c[3].push(CellValue::Integer(s.bound as i64));
            c[4].push(CellValue::Integer(s.bound_mask as i64));
            c[5].push(CellValue::Integer(s.cols as i64));
            c[6].push(CellValue::Float(ln1(s.card)));
            c[7].push(CellValue::Float(ln1(s.key_ndv)));
            c[8].push(CellValue::Float(rows_per_key(s)));
            c[9].push(CellValue::Integer(s.solving as i64));
            c[10].push(CellValue::Integer(s.seed as i64));
            c[11].push(CellValue::String(s.basis.clone()));
            c[12].push(CellValue::Float(prior(s)));
            c[13].push(CellValue::Float(ln1(s.est_in)));
            c[14].push(CellValue::Integer(s.step as i64));
            c[15].push(CellValue::Integer(s.steps as i64));
        }
        c
    }

    /// The statistics' fanout, on the target's scale (0, a fanout of one, where it has none: an empty basis, as a
    /// comparison that leaves the prior out makes it; a filter's basis is "none").
    fn prior(s: &StepSample) -> f64 {
        if !s.basis.is_empty() && s.est_fan.is_finite() && s.est_fan > 0.0 { s.est_fan.ln().clamp(-30.0, 30.0) } else { 0.0 }
    }

    /// The relation's rows per value of the bound key, on the log scale (0 where no key is measured).
    fn rows_per_key(s: &StepSample) -> f64 {
        if s.key_ndv > 0.0 { ln1(s.card / s.key_ndv) } else { 0.0 }
    }

    impl JoinModel {
        /// Trains on `samples`, each weighted by the log of its rows of work: a step of many rows counts more, and
        /// one huge step does not make the rest noise.
        pub fn train(samples: &[StepSample], config: &RandomForestProcedureConfig) -> Result<JoinModel, String> {
            if samples.is_empty() {
                return Err("no steps to train on".into());
            }
            let named: Vec<(String, Vec<CellValue>)> = STEP_FEATURES.iter().map(|n| n.to_string()).zip(cells(samples)).collect();
            let fs = Arc::new(DatasetFeatureSpace::from_columns(&named, FeatureType::Real)?);
            let targets: Vec<f32> = samples.iter().map(|s| s.target() as f32).collect();
            let weights: Vec<f32> = samples.iter().map(|s| ln1(s.work()).max(1.0) as f32).collect();
            let (forest, scale) = train_regression_flat(fs.clone(), &targets, None, Some(&weights), config)?;
            Ok(JoinModel { rules: BucketRules::new(&fs)?, forest, scale, trained_on: samples.len() })
        }

        /// A step's predicted target (its rows out over the statistics' estimate, on the log scale), from its features.
        pub fn fanout(&self, s: &StepSample) -> Result<f64, String> {
            let r = &self.rules;
            let b = [
                r.bucket_of_name(0, Some(&s.relation)),
                r.bucket_of_name(1, Some(&s.access)),
                r.bucket_of_name(2, Some(&s.source)),
                r.bucket_of_number(3, s.bound as f64),
                r.bucket_of_number(4, s.bound_mask as f64),
                r.bucket_of_number(5, s.cols as f64),
                r.bucket_of_number(6, ln1(s.card)),
                r.bucket_of_number(7, ln1(s.key_ndv)),
                r.bucket_of_number(8, rows_per_key(s)),
                r.bucket_of_number(9, s.solving as u8 as f64),
                r.bucket_of_number(10, s.seed as u8 as f64),
                r.bucket_of_name(11, Some(&s.basis)),
                r.bucket_of_number(12, prior(s)),
                r.bucket_of_number(13, ln1(s.est_in)),
                r.bucket_of_number(14, s.step as f64),
                r.bucket_of_number(15, s.steps as f64),
            ];
            Ok(self.scale.from_unit(self.forest.predict_row(&b)?[1]))
        }
    }

    /// The shadow: trains on all but the newest fifth of `samples` (oldest first), and compares the planner's
    /// estimates of the held-out steps with the model's. The model is handed back; nothing is put in place.
    pub fn compare(samples: &[StepSample], config: &RandomForestProcedureConfig) -> Result<(JoinComparison, JoinModel), String> {
        let cut = samples.len() - samples.len() / 5;
        let (train, held) = samples.split_at(cut);
        compare_on(train, held, config)
    }

    /// Trains on `train` and compares on `held`: another run's steps, another corpus's.
    pub fn compare_on(train: &[StepSample], held: &[StepSample], config: &RandomForestProcedureConfig) -> Result<(JoinComparison, JoinModel), String> {
        let model = JoinModel::train(train, config)?;
        let predicted = held.iter().map(|s| model.fanout(s)).collect::<Result<Vec<f64>, String>>()?;
        Ok((super::compared(held, &predicted, train.len()), model))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(est_fan: f64, actual_in: f64, actual_out: f64) -> StepSample {
        StepSample {
            basis: "postings".into(),
            access: "lookup".into(),
            source: "input".into(),
            bound: 1,
            step: 1,
            steps: 2,
            est_in: actual_in,
            est_fan,
            card: 1000.0,
            solving: false,
            seed: true,
            literal: "r:1".into(),
            key_ndv: 100.0,
            actual_in,
            actual_out,
            relation: "r".into(),
            bound_mask: 1,
            cols: 2,
            origin: "ran".into(),
        }
    }

    /// The target is the measured rows out over the statistics' estimate for the rows read; a predicted target
    /// gives back the rows out.
    #[test]
    fn a_steps_target_is_its_measured_rows_over_the_estimate() {
        let s = step(10.0, 100.0, 999.0);
        assert!((s.target() - (1000.0f64 / 1001.0).ln()).abs() < 1e-12);
        let none = step(4877.0, 10.0, 0.0);
        assert!((none.target() + 48_771f64.ln()).abs() < 1e-9, "made nothing, estimated the relation a key");
        assert!((s.out_of(s.target()) - 999.0).abs() < 1e-9);
        assert_eq!(none.out_of(-30.0), 0.0);
    }

    /// Predicting each step's target makes the comparison's model exact; predicting none leaves the statistics' misses.
    #[test]
    fn the_comparison_weighs_misses_by_work() {
        let held = [step(1.0, 1000.0, 99_999.0), step(1.0, 10.0, 10.0)];
        let exact: Vec<f64> = held.iter().map(StepSample::target).collect();
        let c = compared(&held, &exact, 8);
        assert!((c.model_weighted - 1.0).abs() < 1e-9 && c.model.median_log.abs() < 1e-9);
        assert!(c.planner_weighted > 50.0, "the big step's 100× miss dominates: {c}");
        assert!(c.model_wins());
        assert!(!compared(&held, &[0.0, 0.0], 8).model_wins());
    }

    /// The forest learns a fanout that follows the relation read, where the planner priced both alike.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_model_learns_a_relations_fanout() {
        let samples: Vec<StepSample> = (0..600)
            .map(|i| {
                let wide = i % 2 == 0;
                let n = 100.0 + (i % 17) as f64 * 50.0;
                let mut s = step(1.0, n, if wide { 30.0 * n } else { n / 30.0 });
                s.relation = if wide { "calls" } else { "defines" }.into();
                s
            })
            .collect();
        let config = crate::cost_model::CostConfig::default().forest;
        let (c, _) = compare(&samples, &config).unwrap();
        assert!(c.planner_weighted > 4.0 && c.model_weighted < 1.5, "{c}");
        assert!(c.model_wins());
    }
}
