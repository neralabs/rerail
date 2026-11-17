//! The planner's join estimates, corrected by a learned model (decision 0354). The planner prices each step of a
//! join's order by a rule of thumb (its `basis`: postings, field statistics, containment, membership…), and some are
//! wrong by hundreds of times. Each step the engine ran is a sample of what its rule estimated and what the step
//! made ([`StepSample`]); a regression forest (`rerail-forest`, as the cost model's) learns the step's miss from what
//! the planner knew when it priced it, and corrects the estimate by it.
//!
//! **Step 1, the shadow.** The model is trained and compared, not used: [`compare`] holds out the newest fifth of the
//! samples, trains on the rest, and measures both the planner's estimates and the corrected ones against what the
//! held-out steps made ([`JoinComparison`]). No plan changes.
//!
//! **The target.** A step's miss in rows: ln((made + 1) / (estimated fanout × rows in + 1)). The estimated fanout is
//! the planner's rows out over its rows in, for the step; it is applied to the rows the step actually read, so that
//! the miss is the step's own and not the steps' before it.

use crate::cost_model::Errors;

/// A step a join ran: what the planner knew when it priced it, and what it made.
#[derive(Clone, Debug, PartialEq)]
pub struct StepSample {
    /// The rule that priced the step's rows (`StepPlan::basis`, its first words): "postings", "observed"…; "none" for
    /// a filter.
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
    /// Where the sample came from, not a feature: "ran" (the order a line ran throughout), "replaced" (an order of a
    /// line that ran more than one, decision 0206's handoffs: the orders the planner did not keep).
    pub origin: String,
}

impl StepSample {
    /// The planner's rows out for the rows the step read.
    pub fn est_out(&self) -> f64 {
        self.est_fan.max(0.0) * self.actual_in
    }

    /// The step's miss in rows, on the log scale (the model's target).
    pub fn target(&self) -> f64 {
        ((self.actual_out + 1.0) / (self.est_out() + 1.0)).ln().clamp(-30.0, 30.0)
    }

    /// The rows of work the step stands for: what it read, and the more of what it made and what it was planned to
    /// make. Weighed by what it made alone, an overestimate weighed little, though it misleads a plan as much as an
    /// underestimate (`up_fn_of` on hogql, planned at 1,000 times its work, priced the GPU cheaper).
    pub fn work(&self) -> f64 {
        self.actual_in + self.actual_out.max(self.est_out())
    }
}

/// The features, in the feature space's column order.
pub const STEP_FEATURES: [&str; 12] = ["basis", "access", "source", "bound", "step", "steps", "est_in", "est_fan", "card", "solving", "seed", "key_ndv"];

/// The planner's estimates and the model's corrected ones, on the held-out steps.
#[derive(Clone, Copy, Debug, Default)]
pub struct JoinComparison {
    pub trained_on: usize,
    pub held_out: usize,
    /// Per step, unweighted: the planner's rows out against what the step made, and the model's.
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

/// The comparison of corrections `corrections` (the predicted target of each of `held`) with the planner's.
pub fn compared(held: &[StepSample], corrections: &[f64], trained_on: usize) -> JoinComparison {
    let planner: Vec<(f64, f64)> = held.iter().map(|s| (s.est_out() + 1.0, s.actual_out + 1.0)).collect();
    let model: Vec<(f64, f64)> = held.iter().zip(corrections).map(|(s, c)| ((s.est_out() + 1.0) * c.exp(), s.actual_out + 1.0)).collect();
    let w: Vec<f64> = held.iter().map(StepSample::work).collect();
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

    /// A trained correction: a regression forest over [`STEP_FEATURES`], predicting a step's miss.
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
            c[0].push(CellValue::String(s.basis.clone()));
            c[1].push(CellValue::String(s.access.clone()));
            c[2].push(CellValue::String(s.source.clone()));
            c[3].push(CellValue::Integer(s.bound as i64));
            c[4].push(CellValue::Integer(s.step as i64));
            c[5].push(CellValue::Integer(s.steps as i64));
            c[6].push(CellValue::Float(ln1(s.est_in)));
            c[7].push(CellValue::Float(s.est_fan.max(1e-9).ln()));
            c[8].push(CellValue::Float(ln1(s.card)));
            c[9].push(CellValue::Integer(s.solving as i64));
            c[10].push(CellValue::Integer(s.seed as i64));
            c[11].push(CellValue::Float(ln1(s.key_ndv)));
        }
        c
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

        /// The predicted miss of a step (its target: ln of made over estimated), from its features alone.
        pub fn correction(&self, s: &StepSample) -> Result<f64, String> {
            let r = &self.rules;
            let b = [
                r.bucket_of_name(0, Some(&s.basis)),
                r.bucket_of_name(1, Some(&s.access)),
                r.bucket_of_name(2, Some(&s.source)),
                r.bucket_of_number(3, s.bound as f64),
                r.bucket_of_number(4, s.step as f64),
                r.bucket_of_number(5, s.steps as f64),
                r.bucket_of_number(6, ln1(s.est_in)),
                r.bucket_of_number(7, s.est_fan.max(1e-9).ln()),
                r.bucket_of_number(8, ln1(s.card)),
                r.bucket_of_number(9, s.solving as u8 as f64),
                r.bucket_of_number(10, s.seed as u8 as f64),
                r.bucket_of_number(11, ln1(s.key_ndv)),
            ];
            Ok(self.scale.from_unit(self.forest.predict_row(&b)?[1]))
        }
    }

    /// Step 1 (the shadow): trains on all but the newest fifth of `samples` (oldest first), and compares the planner's
    /// estimates of the held-out steps with the corrected ones. The model is handed back; nothing is put in place.
    pub fn compare(samples: &[StepSample], config: &RandomForestProcedureConfig) -> Result<(JoinComparison, JoinModel), String> {
        let cut = samples.len() - samples.len() / 5;
        let (train, held) = samples.split_at(cut);
        compare_on(train, held, config)
    }

    /// Trains on `train` and compares on `held`: another run's steps, another corpus's.
    pub fn compare_on(train: &[StepSample], held: &[StepSample], config: &RandomForestProcedureConfig) -> Result<(JoinComparison, JoinModel), String> {
        let model = JoinModel::train(train, config)?;
        let corrections = held.iter().map(|s| model.correction(s)).collect::<Result<Vec<f64>, String>>()?;
        Ok((super::compared(held, &corrections, train.len()), model))
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
            origin: "ran".into(),
        }
    }

    /// The target is the step's own miss: the planner's fanout over the rows it read, against what it made.
    #[test]
    fn a_steps_miss_is_its_fanout_over_the_rows_it_read() {
        let s = step(10.0, 100.0, 999.0);
        assert!((s.target() - (1000.0f64 / 1001.0).ln()).abs() < 1e-12);
        let none = step(4877.0, 10.0, 0.0);
        assert!((none.target() + 48_771f64.ln()).abs() < 1e-9, "made nothing, planned the relation a key");
    }

    /// A correction that predicts each miss makes the comparison's model exact; none leaves the planner's.
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

    /// The forest learns a miss that follows the step's basis: "postings" 30× over, "observed" right.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_model_learns_a_basis_that_overestimates() {
        let samples: Vec<StepSample> = (0..600)
            .map(|i| {
                let observed = i % 2 == 0;
                let n = 100.0 + (i % 17) as f64 * 50.0;
                let mut s = step(30.0, n, if observed { 30.0 * n } else { n });
                s.basis = if observed { "observed" } else { "postings" }.into();
                s
            })
            .collect();
        let config = crate::cost_model::CostConfig::default().forest;
        let (c, _) = compare(&samples, &config).unwrap();
        assert!(c.planner_weighted > 4.0 && c.model_weighted < 1.5, "{c}");
        assert!(c.model_wins());
    }
}
