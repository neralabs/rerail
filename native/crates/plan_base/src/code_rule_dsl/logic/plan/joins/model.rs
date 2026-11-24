//! The learned join model as the planner's batch pricer (decision 0354): the cost model trains it on the stored steps
//! (`costs::join_steps::install_model`), and this puts it in place, registered as the engine loads (`boot`).

use crate::code_rule_dsl::logic::costs::join_steps::{StepSample, model_price};

/// Puts a trained model in place as the batch pricer.
pub fn install(m: rerail_accel::join_model::JoinModel) {
    super::set_pricer(Some(std::sync::Arc::new(ModelPricer(m))));
}

/// The learned join model as the batch pricer (decision 0354).
struct ModelPricer(rerail_accel::join_model::JoinModel);

impl super::Pricer for ModelPricer {
    /// Every step of every candidate of the round in one prediction (on the device, past a few steps), then each
    /// candidate's price from its steps' targets.
    fn price(&self, round: &[super::Choice]) -> Option<Vec<rerail_blocks::vector::Vector<f64>>> {
        let steps: Vec<&StepSample> = round.iter().flat_map(|c| c.candidates.iter().flat_map(|k| &k.steps)).collect();
        if steps.is_empty() {
            return None;
        }
        // Queued with the rounds of strata planned at once (`JoinModel::fanouts`): one dispatch for all of them.
        let targets = self.0.fanouts(&steps).ok()?;
        let [asked, dispatched, walked] = self.0.counts();
        if asked.is_power_of_two() {
            let [rows, waiting, predicting] = self.0.cpu();
            super::log(|| format!("join model: {asked} rounds, {dispatched} dispatches, {walked} walked on the host; CPU {rows:.3} s bucketing, {waiting:.3} s waiting, {predicting:.3} s predicting"));
        }
        let mut at = 0;
        Some(
            round
                .iter()
                .map(|c| {
                    c.candidates
                        .iter()
                        .map(|k| {
                            let t = &targets[at..at + k.steps.len()];
                            at += k.steps.len();
                            model_price(k.est, &k.steps, t).unwrap_or(k.est)
                        })
                        .collect()
                })
                .collect(),
        )
    }
}
