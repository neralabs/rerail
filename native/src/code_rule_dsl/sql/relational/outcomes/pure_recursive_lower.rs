//! Capture-free structural recursion: finite control and a strict preorder progress proof.
use crate::code_rule_dsl::{
    matcher::{AstMatcher, StopBy, UtilRef, walk_candidate_filter},
    sql::{
        relational::{ir::Set, lower::Lowerer, recursive_chain::pure},
        terminals::pattern_vars,
    },
};
use ast_grep_core::matcher::Matcher;
use std::collections::{BTreeMap, BTreeSet};

type Key = (usize, usize);
#[derive(Clone, Copy)]
pub(in crate::code_rule_dsl::sql::relational::outcomes) enum Direction {
    Child,
    Parent,
    Previous,
    Next,
}
#[derive(Clone)]
pub(in crate::code_rule_dsl::sql::relational::outcomes) struct Walk {
    pub direction: Direction,
    pub end: bool,
    pub field: Option<u16>,
    pub stop: Option<Set>,
    pub filter: Option<Set>,
}
pub(in crate::code_rule_dsl::sql::relational::outcomes) enum Instruction {
    Return(bool),
    Dispatch,
    Failure,
    Test {
        predicate: Set,
        yes: usize,
        no: usize,
    },
    Call {
        utility: usize,
        yes: usize,
        no: usize,
    },
    Search {
        walk: Walk,
        body: usize,
        yes: usize,
        no: usize,
    },
}
pub(in crate::code_rule_dsl::sql::relational::outcomes) struct Program {
    pub projection: Option<super::super::projected_tail::Projection>,
    pub instructions: Vec<Instruction>,
    pub functions: Vec<usize>,
    pub entry: usize,
    pub source_operators: usize,
    pub unfused_instructions: usize,
}
struct Source {
    keys: BTreeMap<Key, usize>,
    bodies: Vec<AstMatcher>,
    edges: Vec<(usize, usize, i8)>,
    operators: usize,
}
impl Source {
    fn utility(&mut self, util: &UtilRef) -> Result<usize, ()> {
        let key = (util.table.as_ptr() as usize, util.index);
        if let Some(id) = self.keys.get(&key) {
            return Ok(*id);
        }
        if self.keys.len() == 128 {
            return Err(());
        }
        let id = self.bodies.len();
        self.keys.insert(key, id);
        let body = util.with_target(Clone::clone).ok_or(())?;
        self.bodies.push(body.clone());
        self.rule(id, &body, 0)?;
        Ok(id)
    }
    fn rule(&mut self, owner: usize, rule: &AstMatcher, progress: i8) -> Result<(), ()> {
        self.operators += 1;
        if self.operators > 256 {
            return Err(());
        }
        match rule {
            AstMatcher::Kind(_) | AstMatcher::Regex(_) | AstMatcher::Never => Ok(()),
            AstMatcher::Pattern(pattern) if pattern_vars(pattern).is_empty() => Ok(()),
            AstMatcher::All(items) | AstMatcher::Any(items) => {
                for item in items {
                    self.rule(owner, item, progress)?;
                }
                Ok(())
            }
            AstMatcher::Not(child) => self.rule(owner, child, progress),
            AstMatcher::Util(util) => {
                let target = self.utility(util)?;
                self.edges.push((owner, target, progress));
                Ok(())
            }
            AstMatcher::Inside(child, stop)
            | AstMatcher::Has(child, stop)
            | AstMatcher::Follows(child, stop)
            | AstMatcher::Precedes(child, stop) => {
                if let StopBy::Rule(stop) = stop {
                    pure(stop).map_err(|_| ())?;
                }
                let delta = if matches!(rule, AstMatcher::Has(..) | AstMatcher::Precedes(..)) {
                    1
                } else {
                    -1
                };
                self.rule(owner, child.rule(), movement(progress, delta))
            }
            AstMatcher::Field(field) if matches!(field.stop, StopBy::Neighbor) => self.rule(
                owner,
                &field.rule,
                movement(progress, if field.inside { -1 } else { 1 }),
            ),
            _ => Err(()),
        }
    }
    fn reach(&self) -> Vec<Vec<bool>> {
        let n = self.bodies.len();
        let mut reach = vec![vec![false; n]; n];
        for &(a, b, _) in &self.edges {
            reach[a][b] = true;
        }
        for k in 0..n {
            for i in 0..n {
                for j in 0..n {
                    let joined = reach[i][k] && reach[k][j];
                    reach[i][j] |= joined;
                }
            }
        }
        reach
    }
    fn legacy_root(&self, root: &UtilRef) -> bool {
        let AstMatcher::Any(items) = &self.bodies[0] else {
            return false;
        };
        let key = (root.table.as_ptr() as usize, root.index);
        let Some(flags) = super::recursive_search_lower::singleton_arms(&self.bodies[0], key)
        else {
            return false;
        };
        if flags.iter().filter(|v| **v).count() != 1 || !flags.last().copied().unwrap_or(false) {
            return false;
        }
        let reach = self.reach();
        // Acyclic helper DAGs are profitably fused into this program. Separate
        // cyclic components, or a standalone simple self loop, retain forests.
        if reach.len() > 1 && !(1..reach.len()).any(|id| reach[id][id]) {
            return false;
        }
        if (0..reach.len())
            .filter(|id| reach[0][*id] && reach[*id][0])
            .count()
            != 1
        {
            return false;
        }
        let tail = items.last().unwrap();
        let (guards, walk) = match tail {
            AstMatcher::All(v) if !v.is_empty() => (&v[..v.len() - 1], v.last().unwrap()),
            other => (&[][..], other),
        };
        let AstMatcher::Has(child, StopBy::Neighbor) = walk else {
            return false;
        };
        if !matches!(child.rule(), AstMatcher::Util(_)) || !guards.iter().any(finite_kind_guard) {
            return false;
        }
        items[..items.len() - 1]
            .iter()
            .all(|m| !references_utility(m, key))
    }
    fn progress(&self) -> bool {
        let count = self.bodies.len();
        let reach = self.reach();
        // A single guarded neighbor delegation is cheaper around the existing
        // specialized child. Rich acyclic controls still benefit from fusion.
        if !reach[0][0] && thin_neighbor_wrapper(&self.bodies[0]) {
            return false;
        }
        let mut recursive = false;
        let mut checked: BTreeSet<usize> = BTreeSet::new();
        for (origin, row) in reach.iter().enumerate() {
            if checked.contains(&origin) || !row[origin] {
                continue;
            }
            recursive = true;
            let component: BTreeSet<_> =
                (0..count).filter(|&n| row[n] && reach[n][origin]).collect();
            checked.extend(&component);
            let mut direction = 0;
            let mut zero = vec![vec![false; count]; count];
            for &(from, to, delta) in &self.edges {
                if !component.contains(&from) || !component.contains(&to) {
                    continue;
                }
                if delta == 2 {
                    return false;
                }
                if delta == 0 {
                    zero[from][to] = true;
                } else if direction == 0 {
                    direction = delta;
                } else if direction != delta {
                    return false;
                }
            }
            for &k in &component {
                for &i in &component {
                    for &j in &component {
                        let joined = zero[i][k] && zero[k][j];
                        zero[i][j] |= joined;
                    }
                }
            }
            if component.iter().any(|&n| zero[n][n]) {
                return false;
            }
        }
        recursive
    }
}
fn finite_kind_guard(m: &AstMatcher) -> bool {
    match m {
        AstMatcher::Kind(k) => k.potential_kinds().is_some(),
        AstMatcher::All(v) => v.iter().any(finite_kind_guard),
        AstMatcher::Any(v) => !v.is_empty() && v.iter().all(finite_kind_guard),
        _ => false,
    }
}
fn references_utility(m: &AstMatcher, key: Key) -> bool {
    match m {
        AstMatcher::Util(u) => (u.table.as_ptr() as usize, u.index) == key,
        AstMatcher::All(v) | AstMatcher::Any(v) => v.iter().any(|m| references_utility(m, key)),
        AstMatcher::Not(m) => references_utility(m, key),
        AstMatcher::Has(c, _)
        | AstMatcher::Inside(c, _)
        | AstMatcher::Follows(c, _)
        | AstMatcher::Precedes(c, _) => references_utility(c.rule(), key),
        AstMatcher::Field(f) => references_utility(&f.rule, key),
        _ => false,
    }
}

fn local_guard(m: &AstMatcher) -> bool {
    match m {
        AstMatcher::Kind(_) | AstMatcher::Regex(_) | AstMatcher::Never => true,
        AstMatcher::Pattern(p) => pattern_vars(p).is_empty(),
        AstMatcher::All(items) | AstMatcher::Any(items) => items.iter().all(local_guard),
        AstMatcher::Not(m) => local_guard(m),
        _ => false,
    }
}
fn thin_neighbor_wrapper(m: &AstMatcher) -> bool {
    let (guards, tail) = match m {
        AstMatcher::All(items) if !items.is_empty() => {
            (&items[..items.len() - 1], items.last().unwrap())
        }
        other => (&[][..], other),
    };
    if !guards.iter().all(local_guard) {
        return false;
    }
    match tail {
        AstMatcher::Has(child, StopBy::Neighbor)
        | AstMatcher::Inside(child, StopBy::Neighbor)
        | AstMatcher::Follows(child, StopBy::Neighbor)
        | AstMatcher::Precedes(child, StopBy::Neighbor) => {
            matches!(child.rule(), AstMatcher::Util(_))
        }
        AstMatcher::Field(field) if matches!(field.stop, StopBy::Neighbor) => {
            matches!(field.rule, AstMatcher::Util(_))
        }
        _ => false,
    }
}

fn movement(progress: i8, delta: i8) -> i8 {
    if progress == 0 || progress == delta {
        delta
    } else {
        2
    }
}

/// Prove the complete reachable utility graph before adding any atomic terminals.
/// A declined probe leaves the caller's lowerer untouched.
pub(super) fn try_lower(
    util: &UtilRef,
    lower: &mut Lowerer,
    three: bool,
) -> Result<Option<Program>, String> {
    let mut source = Source {
        keys: BTreeMap::new(),
        bodies: vec![],
        edges: vec![],
        operators: 0,
    };
    if source.utility(util).is_err() || !source.progress() || source.legacy_root(util) {
        return Ok(None);
    }
    let filter = candidate_filter(&AstMatcher::Util(util.clone()), lower, three)?;
    let projection = super::super::projected_tail::admit(util, lower, three, filter);
    let mut program = Program {
        projection,
        instructions: vec![
            Instruction::Return(true),
            Instruction::Return(false),
            Instruction::Dispatch,
            Instruction::Failure,
        ],
        functions: vec![0; source.bodies.len()],
        entry: 0,
        source_operators: source.operators,
        unfused_instructions: 5 + source.bodies.iter().map(unfused_count).sum::<usize>(),
    };
    for (id, body) in source.bodies.iter().enumerate() {
        program.functions[id] = compile(
            body,
            0,
            1,
            &source.keys,
            lower,
            three,
            &mut program.instructions,
        )?;
    }
    program.entry = program.instructions.len();
    program.instructions.push(Instruction::Call {
        utility: 0,
        yes: 0,
        no: 1,
    });
    if program.instructions.len() > 256 {
        return Err("pure recursion: emitted control exceeds 256 instructions".into());
    }
    Ok(Some(program))
}
fn compile(
    rule: &AstMatcher,
    yes: usize,
    no: usize,
    keys: &BTreeMap<Key, usize>,
    lower: &mut Lowerer,
    three: bool,
    code: &mut Vec<Instruction>,
) -> Result<usize, String> {
    // Fuse complete total Boolean/neighbor regions. The machine never steps a
    // separate Kind, Regex, All or Not opcode when SQL can evaluate the region.
    if pure(rule).is_ok() {
        let pc = code.len();
        code.push(Instruction::Test {
            predicate: lower.set(rule)?,
            yes,
            no,
        });
        return Ok(pc);
    }
    let instruction = match rule {
        AstMatcher::All(items) | AstMatcher::Any(items) => {
            let all = matches!(rule, AstMatcher::All(_));
            let mut next = if all { yes } else { no };
            let mut end = items.len();
            while end > 0 {
                let mut start = end - 1;
                if pure(&items[start]).is_ok() {
                    while start > 0 && pure(&items[start - 1]).is_ok() {
                        start -= 1;
                    }
                }
                if end - start > 1 {
                    // Only a contiguous utility-free total region is fused. A
                    // call/search boundary keeps native short-circuit order.
                    let terms = items[start..end]
                        .iter()
                        .map(|child| lower.set(child))
                        .collect::<Result<Vec<_>, _>>()?;
                    let pc = code.len();
                    code.push(Instruction::Test {
                        predicate: if all {
                            Set::Intersect(terms)
                        } else {
                            Set::Union(terms)
                        },
                        yes: if all { next } else { yes },
                        no: if all { no } else { next },
                    });
                    next = pc;
                } else {
                    next = compile(
                        &items[start],
                        if all { next } else { yes },
                        if all { no } else { next },
                        keys,
                        lower,
                        three,
                        code,
                    )?;
                }
                end = start;
            }
            return Ok(next);
        }
        AstMatcher::Not(child) => return compile(child, no, yes, keys, lower, three, code),
        AstMatcher::Util(util) => Instruction::Call {
            utility: keys[&(util.table.as_ptr() as usize, util.index)],
            yes,
            no,
        },
        AstMatcher::Inside(child, stop)
        | AstMatcher::Has(child, stop)
        | AstMatcher::Follows(child, stop)
        | AstMatcher::Precedes(child, stop) => {
            let direction = match rule {
                AstMatcher::Inside(..) => Direction::Parent,
                AstMatcher::Has(..) => Direction::Child,
                AstMatcher::Follows(..) => Direction::Previous,
                _ => Direction::Next,
            };
            let body = compile(child.rule(), 0, 1, keys, lower, three, code)?;
            let filter = candidate_filter(child.rule(), lower, three)?;
            Instruction::Search {
                walk: Walk {
                    direction,
                    end: !matches!(stop, StopBy::Neighbor),
                    field: None,
                    stop: match stop {
                        StopBy::Rule(rule) => Some(lower.set(rule)?),
                        _ => None,
                    },
                    filter,
                },
                body,
                yes,
                no,
            }
        }
        AstMatcher::Field(field) => {
            let body = compile(&field.rule, 0, 1, keys, lower, three, code)?;
            Instruction::Search {
                walk: Walk {
                    direction: if field.inside {
                        Direction::Parent
                    } else {
                        Direction::Child
                    },
                    end: false,
                    field: Some(field.field),
                    stop: None,
                    filter: candidate_filter(&field.rule, lower, three)?,
                },
                body,
                yes,
                no,
            }
        }
        _ => return Err("pure recursion: proof and lowering disagree".into()),
    };
    let pc = code.len();
    code.push(instruction);
    Ok(pc)
}

fn unfused_count(rule: &AstMatcher) -> usize {
    if pure(rule).is_ok() {
        return 1;
    }
    match rule {
        AstMatcher::All(items) | AstMatcher::Any(items) => items.iter().map(unfused_count).sum(),
        AstMatcher::Not(child) => unfused_count(child),
        AstMatcher::Inside(child, _)
        | AstMatcher::Has(child, _)
        | AstMatcher::Follows(child, _)
        | AstMatcher::Precedes(child, _) => 1 + unfused_count(child.rule()),
        AstMatcher::Field(field) => 1 + unfused_count(&field.rule),
        AstMatcher::Util(_) => 1,
        _ => unreachable!("pure recursion proof precedes control counting"),
    }
}

fn candidate_filter(
    rule: &AstMatcher,
    lower: &mut Lowerer,
    three: bool,
) -> Result<Option<Set>, String> {
    if three {
        return Ok(None);
    }
    walk_candidate_filter(rule)
        .map(|(kinds, clauses)| {
            let mut terms = vec![Set::Kinds(kinds)];
            for clause in clauses {
                let regex = regex::Regex::new(
                    &clause
                        .iter()
                        .map(|s| regex::escape(s))
                        .collect::<Vec<_>>()
                        .join("|"),
                )
                .map_err(|e| e.to_string())?;
                terms.push(lower.set(&AstMatcher::Regex(regex))?);
            }
            Ok(Set::Intersect(terms))
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn graph(edges: Vec<(usize, usize, i8)>) -> Source {
        let count = edges.iter().flat_map(|(a, b, _)| [*a, *b]).max().unwrap() + 1;
        Source {
            keys: BTreeMap::new(),
            bodies: vec![AstMatcher::Never; count],
            edges,
            operators: count,
        }
    }
    #[test]
    fn structural_progress_rejects_zero_cycles_and_opposite_directions() {
        assert!(!graph(vec![(0, 0, 0)]).progress());
        assert!(!graph(vec![(0, 1, 0), (1, 0, 0)]).progress());
        assert!(!graph(vec![(0, 1, 1), (1, 0, -1)]).progress());
        assert!(!graph(vec![(0, 0, 2)]).progress());
        assert!(graph(vec![(0, 1, 0), (1, 0, 1)]).progress());
        assert!(graph(vec![(0, 1, 0), (1, 0, -1)]).progress());
    }
}
