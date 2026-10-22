//! Bounded union-of-joins expansion. Recursive arms reference their CTE once, directly.
use super::ir::{Edge, Program, Set};

const MAX_ARMS: usize = 128;
const MAX_NODES: usize = 32;

#[derive(Clone, Debug)]
pub(super) enum Predicate {
    Kinds(usize, Vec<u16>),
    Regex(usize, usize),
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
        Set::Ancestors { .. } => return Err("ancestor closure is a relation boundary".into()),
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
        Set::Reference(r) | Set::Within(r) => *r == id,
        Set::Ancestors { seeds, stop } => *seeds == id || *stop == Some(id),
        Set::Intersect(s) | Set::Union(s) => s.iter().any(|s| refers(s, id)),
        Set::Complement(s) | Set::Preimage(_, s) => refers(s, id),
        _ => false,
    }
}

pub(super) enum RelationPlan {
    Joins(Vec<Arm>),
    Ancestors { seeds: usize, stop: Option<usize> },
}

fn dependencies(program: &Program) -> Result<(), String> {
    fn visit(id: usize, program: &Program, state: &mut [u8]) -> Result<(), String> {
        if state[id] == 2 {
            return Ok(());
        }
        if state[id] == 1 {
            return Err("relational compiler: recursive traversal needs a component plan".into());
        }
        state[id] = 1;
        let mut edges = Vec::new();
        super::ir::references(&program.relations[id].expression, &mut edges);
        edges.extend(program.relations[id].domain);
        for next in edges {
            if next != id {
                visit(next, program, state)?;
            }
        }
        state[id] = 2;
        Ok(())
    }
    visit(0, program, &mut vec![0; program.relations.len()])
}

pub(super) fn plan(program: &Program) -> Result<Vec<RelationPlan>, String> {
    dependencies(program)?;
    program
        .relations
        .iter()
        .enumerate()
        .map(|(id, relation)| {
            if let Set::Ancestors { seeds, stop } = relation.expression {
                if seeds == id || stop == Some(id) {
                    return Err("recursive traversal needs a component plan".into());
                }
                return Ok(RelationPlan::Ancestors { seeds, stop });
            }
            let mut arms = expand(&relation.expression)?;
            for arm in &arms {
                if arm
                    .predicates
                    .iter()
                    .any(|p| matches!(p,Predicate::Region(_,r) if *r==id))
                {
                    return Err(
                        "relational compiler: recursive traversal needs a component plan".into(),
                    );
                }
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
            Ok(RelationPlan::Joins(arms))
        })
        .collect()
}
