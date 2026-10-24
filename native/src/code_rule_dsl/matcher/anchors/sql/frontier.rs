//! Keep physical plan order separate from the identity of a demand set.
use std::sync::Arc;

pub(super) fn frontier<T>(plans: &[Arc<T>]) -> (Vec<Arc<T>>, Vec<usize>) {
    let mut plans = plans.to_vec();
    plans.sort_unstable_by_key(|plan| Arc::as_ptr(plan) as usize);
    plans.dedup_by_key(|plan| Arc::as_ptr(plan) as usize);
    let ids = plans
        .iter()
        .map(|plan| Arc::as_ptr(plan) as usize)
        .collect();
    (plans, ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allocation_addresses_do_not_reorder_consumers_or_duplicate_shared_plans() {
        let mut allocated = [Arc::new(0), Arc::new(0)];
        allocated.sort_unstable_by_key(|p| Arc::as_ptr(p) as usize);
        *Arc::get_mut(&mut allocated[0]).unwrap() = 2;
        *Arc::get_mut(&mut allocated[1]).unwrap() = 1;
        let (ordered, _) = frontier(&[
            allocated[1].clone(),
            allocated[0].clone(),
            allocated[1].clone(),
        ]);
        assert_eq!(
            ordered.iter().map(|p| **p).collect::<Vec<_>>(),
            [1, 2],
            "authored consumer order must survive opposite allocation order"
        );
    }

    #[test]
    fn demand_identity_is_an_unordered_set_while_execution_preserves_consumer_order() {
        let a = Arc::new(1);
        let b = Arc::new(2);
        let (ab, key_ab) = frontier(&[a.clone(), b.clone()]);
        let (ba, key_ba) = frontier(&[b, a]);
        assert_eq!(
            key_ab, key_ba,
            "compatible file demands must still coalesce"
        );
        assert_eq!(ab.iter().map(|p| **p).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(ba.iter().map(|p| **p).collect::<Vec<_>>(), [2, 1]);
    }
}
