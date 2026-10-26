//! Atomic candidate indexes for operation-cohort relation walks.
use super::{Context, NativeHost};
use crate::code_rule_dsl::matcher::{
    Semantic,
    cohort::{NavigationRequest, NavigationResult, SemanticRequest},
    memo,
};
use ast_grep_core::{Doc, source::Content};
use std::{collections::BTreeMap, sync::Arc};

pub(super) fn evaluate<'t, D: Doc<Source: Content<Underlying = u8>>, C: Context>(
    host: &mut NativeHost<C>,
    requests: Vec<NavigationRequest<'t, D>>,
) -> Vec<NavigationResult<'t, D>> {
    // Use addresses rather than program IDs: one walk program can appear at several roots
    // with different captures and directions in the same projection cohort.
    let semantic = Arc::new(Semantic::Capture(String::new()));
    let leaves: Vec<_> = requests
        .iter()
        .map(|r| SemanticRequest {
            semantic: semantic.clone(),
            request: r.request.clone(),
            utility_depth: r.utility_depth,
        })
        .collect();
    let positions: BTreeMap<_, _> = leaves
        .iter()
        .enumerate()
        .map(|(i, leaf)| (leaf as *const _ as usize, i))
        .collect();
    let hosted = host.hosted.get() != 0;
    let projected = host.project(&leaves, |leaf| {
        let r = &requests[positions[&(leaf as *const _ as usize)]];
        if r.descendants && !r.request.three {
            r.filter.as_ref().and_then(|filter| {
                memo::bounded(filter, &r.request.node, |tape, ordinal| {
                    tape.transducer_descendants(ordinal)
                })
            })
        } else if let Some(forward) = r.forward.filter(|_| !hosted) {
            r.related.as_ref().and_then(|related| {
                related.same_key_siblings(&r.request.node, &r.request.env, forward)
            })
        } else {
            None
        }
    });
    {
        let mut metrics = host.metrics.borrow_mut();
        metrics.navigation_queries += projected.len() as u64;
        for p in &projected {
            if let Some(Some(nodes)) = &p.value {
                metrics.navigation_index_hits += 1;
                metrics.navigation_candidates += nodes.len() as u64;
            }
        }
    }
    projected
        .into_iter()
        .map(|p| NavigationResult {
            nodes: p.value.flatten(),
            work: p.cost.work,
            replay_work: p.cost.replay_work,
            levels: p.cost.levels,
            failure: p.cost.failure,
            clause_state_read: p.cost.clause_state_read,
        })
        .collect()
}
