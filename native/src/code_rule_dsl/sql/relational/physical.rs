//! Bounded union-of-joins expansion. Recursive arms reference their CTE once, directly.
use super::ir::{Edge, Program, Set};

const MAX_ARMS: usize = 128;
const MAX_NODES: usize = 32;

#[derive(Clone, Debug)]
pub(super) enum Predicate {
    Kinds(usize, Vec<u16>),
    Regex(usize, usize),
    Nonempty(usize),
    Pattern(usize, usize),
    Region(usize, usize),
    Keys(usize, usize),
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
        Set::Nonempty => {
            for arm in &mut arms {
                arm.predicates.push(Predicate::Nonempty(node));
            }
        }
        Set::Regex(term) => {
            for arm in &mut arms {
                arm.predicates.push(Predicate::Regex(node, *term));
            }
        }
        Set::Pattern(term) => {
            for arm in &mut arms {
                arm.predicates.push(Predicate::Pattern(node, *term));
            }
        }
        Set::Within(roots) => {
            for arm in &mut arms {
                arm.predicates.push(Predicate::Region(node, *roots));
            }
        }
        Set::Ancestors { .. } | Set::SearchRoots { .. } => {
            return Err("ancestor closure is a relation boundary".into());
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

pub(super) enum RelationPlan {
    Component {
        leader: usize,
        members: Vec<usize>,
        arms: Vec<Arm>,
    },
    Joins(Vec<Arm>),
    Ancestors {
        seeds: usize,
        stop: Option<usize>,
    },
    SearchRoots {
        origins: usize,
        filter: usize,
        stop: Option<usize>,
    },
}

pub(super) fn plan(program: &Program) -> Result<Vec<RelationPlan>, String> {
    let mut components = std::collections::HashMap::new();
    for members in super::recursion::components(program) {
        if members.len() > 1 {
            let plans = super::recursion::prove(program, &members)?;
            for (&id, arms) in members.iter().zip(plans) {
                components.insert(
                    id,
                    RelationPlan::Component {
                        leader: members[0],
                        members: members.clone(),
                        arms,
                    },
                );
            }
        }
    }
    program
        .relations
        .iter()
        .enumerate()
        .map(|(id, relation)| {
            if let Some(component) = components.remove(&id) {
                return Ok(component);
            }
            if let Set::SearchRoots {
                origins,
                filter,
                stop,
            } = relation.expression
            {
                if origins == id || filter == id || stop == Some(id) {
                    return Err("origin search cannot depend on itself".into());
                }
                return Ok(RelationPlan::SearchRoots {
                    origins,
                    filter,
                    stop,
                });
            }
            if let Set::Ancestors { seeds, stop } = relation.expression {
                if seeds == id || stop == Some(id) {
                    return Err("recursive traversal needs a component plan".into());
                }
                return Ok(RelationPlan::Ancestors { seeds, stop });
            }
            let mut arms = super::recursion::prove(program, &[id])?.pop().unwrap();
            // SQLite requires all nonrecursive SELECTs before recursive SELECTs.
            arms.sort_by_key(|arm| arm.references.iter().any(|(_, r)| *r == id));
            Ok(RelationPlan::Joins(arms))
        })
        .collect()
}
