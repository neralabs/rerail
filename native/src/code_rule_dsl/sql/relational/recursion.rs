//! Dependency components and termination proofs for total structural relations.
use super::{
    ir::{self, Edge, Program},
    physical::{self, Arm, Predicate},
};

/// Iterative Kosaraju traversal, including negative and domain dependencies.
pub(super) fn components(program: &Program) -> Vec<Vec<usize>> {
    let count = program.relations.len();
    let mut edges = vec![Vec::new(); count];
    let mut reverse = vec![Vec::new(); count];
    for (id, relation) in program.relations.iter().enumerate() {
        ir::references(&relation.expression, &mut edges[id]);
        edges[id].extend(relation.domain);
        for &next in &edges[id] {
            reverse[next].push(id);
        }
    }
    let mut seen = vec![false; count];
    let mut order = Vec::new();
    for root in 0..count {
        let mut stack = vec![(root, false)];
        while let Some((id, exiting)) = stack.pop() {
            if exiting {
                order.push(id);
            } else if !std::mem::replace(&mut seen[id], true) {
                stack.push((id, true));
                stack.extend(edges[id].iter().rev().map(|&next| (next, false)));
            }
        }
    }
    seen.fill(false);
    let mut components = Vec::new();
    for root in order.into_iter().rev() {
        if seen[root] {
            continue;
        }
        let mut members = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if !std::mem::replace(&mut seen[id], true) {
                members.push(id);
                stack.extend(&reverse[id]);
            }
        }
        members.sort_unstable();
        components.push(members);
    }
    components
}

pub(super) fn mutual(program: &Program, id: usize) -> bool {
    components(program)
        .iter()
        .any(|members| members.len() > 1 && members.contains(&id))
}

/// Every recursive path strictly changes preorder in the same direction. Thus native
/// evaluation cannot revisit a (utility, file, node) key, even at a utility graph cycle.
/// Capture-free total predicates observe existence only, so positive least-fixed-point
/// answers and depth-first native answers agree without storing path identity.
pub(super) fn prove(program: &Program, members: &[usize]) -> Result<Vec<Vec<Arm>>, String> {
    let mut direction = None;
    let mut planned = Vec::new();
    for &id in members {
        let relation = &program.relations[id];
        if members.len() > 1 && (relation.domain.is_some() || relation.keys.is_some()) {
            return Err("recursive component needs demand for all members together".into());
        }
        let arms = physical::expand(&relation.expression)
            .map_err(|e| format!("recursive component: {e}"))?;
        for arm in &arms {
            for predicate in &arm.predicates {
                let mut dependencies = Vec::new();
                match predicate {
                    Predicate::Absent(_, set) => ir::references(set, &mut dependencies),
                    Predicate::Region(_, id) | Predicate::Keys(_, id) => dependencies.push(*id),
                    _ => {}
                }
                if dependencies.iter().any(|r| members.contains(r)) {
                    return Err(
                        "recursive negation or region dependency needs an outcome contract".into(),
                    );
                }
            }
            let recursive: Vec<_> = arm
                .references
                .iter()
                .filter(|(_, r)| members.contains(r))
                .collect();
            if recursive.len() > 1 {
                return Err("nonlinear recursion needs a component outcome contract".into());
            }
            if let Some(&&(mut node, _)) = recursive.first() {
                if node == 0 {
                    return Err("recursive component must advance through a structural edge".into());
                }
                while node != 0 {
                    let &(parent, edge, _) = arm
                        .edges
                        .iter()
                        .find(|(_, _, to)| *to == node)
                        .ok_or("recursive component has a disconnected dependency")?;
                    let ascending = matches!(edge, Edge::Parent | Edge::Previous);
                    if direction.is_some_and(|old| old != ascending) {
                        return Err(
                            "recursive component needs one strict preorder direction".into()
                        );
                    }
                    direction = Some(ascending);
                    node = parent;
                }
            }
        }
        planned.push(arms);
    }
    if planned.iter().map(Vec::len).sum::<usize>() > 128 {
        return Err("relational compiler: component expansion exceeds 128 arms".into());
    }
    Ok(planned)
}
