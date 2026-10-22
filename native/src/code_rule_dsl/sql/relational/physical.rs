//! Bounded union-of-joins expansion. Recursive arms reference their CTE once, directly.
use super::ir::{Edge, Program, Set};

const MAX_ARMS: usize = 128;
const MAX_NODES: usize = 32;

#[derive(Clone, Debug)]
pub(super) enum Predicate {
    Kinds(usize, Vec<u16>),
    Regex(usize, usize),
    Absent(usize, Set),
}

#[derive(Clone, Debug)]
pub(super) struct Arm {
    pub nodes: usize,
    pub edges: Vec<(usize, Edge, usize)>,
    pub predicates: Vec<Predicate>,
    pub references: Vec<(usize, usize)>,
}

impl Default for Arm {
    fn default() -> Self {
        Self {
            nodes: 1,
            edges: vec![],
            predicates: vec![],
            references: vec![],
        }
    }
}

pub(super) fn expand(set: &Set) -> Result<Vec<Arm>, String> {
    extend(set, 0, vec![Arm::default()])
}

fn extend(set: &Set, node: usize, mut arms: Vec<Arm>) -> Result<Vec<Arm>, String> {
    match set {
        Set::Empty => arms.clear(),
        Set::AllNodes => {}
        Set::Kinds(kinds) => {
            for arm in &mut arms {
                arm.predicates.push(Predicate::Kinds(node, kinds.clone()));
            }
        }
        Set::Regex(term) => {
            for arm in &mut arms {
                arm.predicates.push(Predicate::Regex(node, *term));
            }
        }
        Set::Reference(id) => {
            for arm in &mut arms {
                arm.references.push((node, *id));
            }
        }
        Set::Complement(inner) => {
            for arm in &mut arms {
                arm.predicates.push(Predicate::Absent(node, *inner.clone()));
            }
        }
        Set::Intersect(sets) => {
            for set in sets {
                arms = extend(set, node, arms)?;
            }
        }
        Set::Union(sets) => {
            let mut combined = Vec::new();
            for set in sets {
                combined.extend(extend(set, node, arms.clone())?);
                if combined.len() > MAX_ARMS {
                    return Err("relational compiler: join-arm expansion exceeds 128".into());
                }
            }
            arms = combined;
        }
        Set::Preimage(edge, inner) => {
            let mut combined = Vec::new();
            for mut arm in arms {
                let child = arm.nodes;
                if child >= MAX_NODES {
                    return Err("relational compiler: join width exceeds 32 nodes".into());
                }
                arm.nodes += 1;
                arm.edges.push((node, *edge, child));
                combined.extend(extend(inner, child, vec![arm])?);
                if combined.len() > MAX_ARMS {
                    return Err("relational compiler: join-arm expansion exceeds 128".into());
                }
            }
            arms = combined;
        }
    }
    Ok(arms)
}

fn refers(set: &Set, id: usize) -> bool {
    match set {
        Set::Reference(r) => *r == id,
        Set::Intersect(s) | Set::Union(s) => s.iter().any(|s| refers(s, id)),
        Set::Complement(s) | Set::Preimage(_, s) => refers(s, id),
        _ => false,
    }
}

pub(super) fn plan(program: &Program) -> Result<Vec<Vec<Arm>>, String> {
    program
        .relations
        .iter()
        .enumerate()
        .map(|(id, relation)| {
            let mut arms = expand(&relation.expression)?;
            for arm in &arms {
                if arm
                    .predicates
                    .iter()
                    .any(|p| matches!(p,Predicate::Absent(_,s) if refers(s,id)))
                {
                    return Err(format!(
                        "{}: recursive negation needs an outcome contract",
                        relation.source
                    ));
                }
                let recursive: Vec<_> = arm.references.iter().filter(|(_, r)| *r == id).collect();
                if recursive.len() > 1 {
                    return Err(format!(
                        "{}: nonlinear recursion needs a component plan",
                        relation.source
                    ));
                }
                if let Some((mut node, _)) = recursive.first().copied().copied() {
                    if node == 0 {
                        return Err(format!(
                            "{}: recursion must advance through a parent edge",
                            relation.source
                        ));
                    }
                    while node != 0 {
                        let Some(&(parent, Edge::Parent, _)) =
                            arm.edges.iter().find(|(_, _, to)| *to == node)
                        else {
                            return Err(format!(
                                "{}: only strictly ascending recursive dependencies are admitted",
                                relation.source
                            ));
                        };
                        node = parent;
                    }
                }
            }
            // SQLite requires all nonrecursive SELECTs before recursive SELECTs.
            arms.sort_by_key(|arm| arm.references.iter().any(|(_, r)| *r == id));
            if !arms.is_empty()
                && arms
                    .iter()
                    .all(|arm| arm.references.iter().any(|(_, r)| *r == id))
            {
                return Err(format!(
                    "{}: recursive relation has no seed",
                    relation.source
                ));
            }
            Ok(arms)
        })
        .collect()
}
