//! Capture-free recursive control with finite SQL regions and monotone SCC calls.
use super::{Lower, Op, Plan, Rc, Slots};
use crate::code_rule_dsl::matcher::{self, AstMatcher, Semantic, StopBy, UtilRef};
use crate::code_rule_dsl::sql::relational::{ir::Set, recursive_chain::pure};
use std::collections::{BTreeMap, BTreeSet};
type Key = (usize, usize);
#[derive(Clone, Copy)]
pub(in crate::code_rule_dsl::sql::relational::outcomes) enum Direction {
    Child,
    Parent,
    Previous,
    Next,
}
pub(in crate::code_rule_dsl::sql::relational::outcomes) struct Walk {
    pub direction: Direction,
    pub end: bool,
    pub field: Option<u16>,
    pub stop: Option<Set>,
    pub filter: Option<Set>,
}
pub(in crate::code_rule_dsl::sql::relational::outcomes) enum Instruction {
    Dispatch,
    Failure,
    Test(Set),
    Eval(usize),
    Call(usize),
    Boolean { all: bool, children: Vec<usize> },
    Not(usize),
    Search { walk: Walk, body: usize },
}
pub(in crate::code_rule_dsl::sql::relational::outcomes) struct Function {
    pub entry: usize,
    pub memo: usize,
}
pub(in crate::code_rule_dsl::sql::relational::outcomes) struct Program {
    pub code: Vec<Instruction>,
    pub functions: Vec<Function>,
    pub regions: Vec<Plan>,
    pub mentions: Slots,
    pub bypass: bool,
    pub three: bool,
}
struct Source {
    keys: BTreeMap<Key, usize>,
    refs: Vec<UtilRef>,
    bodies: Vec<AstMatcher>,
    edges: Vec<(usize, usize, i8)>,
    operators: usize,
}
fn movement(a: i8, b: i8) -> i8 {
    if a == 0 {
        b
    } else if a == b {
        a
    } else {
        2
    }
}
impl Source {
    fn utility(&mut self, u: &UtilRef) -> Result<usize, ()> {
        let key = (u.table.as_ptr() as usize, u.index);
        if let Some(id) = self.keys.get(&key) {
            return Ok(*id);
        }
        if self.refs.len() >= 128 {
            return Err(());
        }
        let id = self.refs.len();
        self.keys.insert(key, id);
        self.refs.push(u.clone());
        let body = u.with_target(Clone::clone).ok_or(())?;
        self.bodies.push(body.clone());
        self.walk(id, &body, 0)?;
        Ok(id)
    }
    fn walk(&mut self, owner: usize, m: &AstMatcher, progress: i8) -> Result<(), ()> {
        self.operators += 1;
        if self.operators > 256 {
            return Err(());
        }
        match m {
            AstMatcher::Util(u) => {
                let id = self.utility(u)?;
                self.edges.push((owner, id, progress));
            }
            AstMatcher::Call(c) => {
                let id = self.utility(&c.util)?;
                self.edges.push((owner, id, 2));
            }
            AstMatcher::All(v) | AstMatcher::Any(v) => {
                for c in v {
                    self.walk(owner, c, progress)?;
                }
            }
            AstMatcher::Not(c) => self.walk(owner, c, progress)?,
            AstMatcher::Inside(c, s)
            | AstMatcher::Has(c, s)
            | AstMatcher::Follows(c, s)
            | AstMatcher::Precedes(c, s) => {
                if let StopBy::Rule(s) = s {
                    self.walk(owner, s, 2)?;
                }
                let delta = if matches!(m, AstMatcher::Has(..) | AstMatcher::Precedes(..)) {
                    1
                } else {
                    -1
                };
                self.walk(owner, c.rule(), movement(progress, delta))?;
            }
            AstMatcher::Field(f) => {
                if let StopBy::Rule(s) = &f.stop {
                    self.walk(owner, s, 2)?;
                }
                self.walk(
                    owner,
                    &f.rule,
                    if matches!(f.stop, StopBy::Neighbor) {
                        movement(progress, if f.inside { -1 } else { 1 })
                    } else {
                        2
                    },
                )?;
            }
            AstMatcher::Constrained(c) => {
                self.walk(owner, &c.rule, progress)?;
                for (_, r) in &c.constraints {
                    self.walk(owner, r, 2)?;
                }
            }
            AstMatcher::Count(c) => self.walk(owner, &c.relation, 2)?,
            AstMatcher::NthChild(c) => {
                if let Some(r) = &c.of_rule {
                    self.walk(owner, r, 2)?;
                }
            }
            AstMatcher::Semantic(s) => match s {
                Semantic::Binding(t) => {
                    for c in t.sub_rules() {
                        self.walk(owner, c, 2)?;
                    }
                }
                Semantic::Binds(t) => {
                    for c in t.sub_rules() {
                        self.walk(owner, c, 2)?;
                    }
                }
                Semantic::Parse(t) => self.walk(owner, &t.rule, 2)?,
                Semantic::Host(t) => self.walk(owner, &t.rule, 2)?,
                Semantic::FormatArgs(t) => {
                    for c in [t.unused.as_ref(), t.missing.as_ref()]
                        .into_iter()
                        .flatten()
                    {
                        self.walk(owner, c, 2)?;
                    }
                }
                Semantic::Calls(_) => return Err(()),
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }
    fn component(&self) -> Option<BTreeSet<usize>> {
        let n = self.refs.len();
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
        if !reach[0][0] {
            return None;
        }
        let members: BTreeSet<_> = (0..n).filter(|i| reach[0][*i] && reach[*i][0]).collect();
        let mut direction = 0;
        let mut zero = vec![vec![false; n]; n];
        for &(a, b, d) in &self.edges {
            if !members.contains(&a) || !members.contains(&b) {
                continue;
            }
            if d == 2 {
                return None;
            }
            if d == 0 {
                zero[a][b] = true;
            } else if direction == 0 {
                direction = d;
            } else if direction != d {
                return None;
            }
        }
        for &k in &members {
            for &i in &members {
                for &j in &members {
                    let joined = zero[i][k] && zero[k][j];
                    zero[i][j] |= joined;
                }
            }
        }
        if members.iter().any(|i| zero[*i][*i]) {
            None
        } else {
            Some(members)
        }
    }
    fn recursive(&self, m: &AstMatcher, members: &BTreeSet<usize>) -> bool {
        let mut found = false;
        let mut pending = vec![m];
        while let Some(m) = pending.pop() {
            match m {
                AstMatcher::Util(u) => {
                    found |= members.contains(&self.keys[&(u.table.as_ptr() as usize, u.index)]);
                }
                AstMatcher::Call(c) => {
                    found |= members
                        .contains(&self.keys[&(c.util.table.as_ptr() as usize, c.util.index)]);
                }
                AstMatcher::All(v) | AstMatcher::Any(v) => pending.extend(v),
                AstMatcher::Not(c) => pending.push(c),
                AstMatcher::Inside(c, s)
                | AstMatcher::Has(c, s)
                | AstMatcher::Follows(c, s)
                | AstMatcher::Precedes(c, s) => {
                    pending.push(c.rule());
                    if let StopBy::Rule(s) = s {
                        pending.push(s);
                    }
                }
                AstMatcher::Field(f) => {
                    pending.push(&f.rule);
                    if let StopBy::Rule(s) = &f.stop {
                        pending.push(s);
                    }
                }
                AstMatcher::Constrained(c) => {
                    pending.push(&c.rule);
                    pending.extend(c.constraints.iter().map(|(_, r)| r));
                }
                AstMatcher::Count(c) => pending.push(&c.relation),
                AstMatcher::NthChild(c) => pending.extend(c.of_rule.iter()),
                AstMatcher::Semantic(s) => match s {
                    Semantic::Binding(t) => pending.extend(t.sub_rules()),
                    Semantic::Binds(t) => pending.extend(t.sub_rules()),
                    Semantic::Parse(t) => pending.push(&t.rule),
                    Semantic::Host(t) => pending.push(&t.rule),
                    Semantic::FormatArgs(t) => pending.extend(
                        [t.unused.as_ref(), t.missing.as_ref()]
                            .into_iter()
                            .flatten(),
                    ),
                    _ => {}
                },
                _ => {}
            }
        }
        found
    }
}

struct Builder<'a> {
    source: &'a Source,
    members: &'a BTreeSet<usize>,
    dense: BTreeMap<usize, usize>,
    program: Program,
}
impl Builder<'_> {
    fn compile(&mut self, m: &AstMatcher, lower: &mut Lower) -> Result<usize, String> {
        if !self.source.recursive(m, self.members) {
            let instruction = if pure(m).is_ok() {
                Instruction::Test(lower.pure.set(m)?)
            } else {
                let p = lower.plan(m)?;
                if !p.writes.is_empty() {
                    return Err(
                        "mixed recursion: finite regions must not write captures or counts".into(),
                    );
                }
                self.program.mentions.extend(&p.mentions);
                let id = self.program.regions.len();
                self.program.regions.push(p);
                Instruction::Eval(id)
            };
            return self.push(instruction);
        }
        let instruction = match m {
            AstMatcher::Util(u) => Instruction::Call(
                self.dense[&self.source.keys[&(u.table.as_ptr() as usize, u.index)]],
            ),
            AstMatcher::All(v) | AstMatcher::Any(v) => {
                let mut children = vec![];
                for c in v {
                    children.push(self.compile(c, lower)?);
                }
                Instruction::Boolean {
                    all: matches!(m, AstMatcher::All(_)),
                    children,
                }
            }
            AstMatcher::Not(c) => Instruction::Not(self.compile(c, lower)?),
            AstMatcher::Inside(c, s)
            | AstMatcher::Has(c, s)
            | AstMatcher::Follows(c, s)
            | AstMatcher::Precedes(c, s) => {
                let direction = match m {
                    AstMatcher::Inside(..) => Direction::Parent,
                    AstMatcher::Has(..) => Direction::Child,
                    AstMatcher::Follows(..) => Direction::Previous,
                    _ => Direction::Next,
                };
                let stop = match s {
                    StopBy::Rule(r) => {
                        pure(r).map_err(
                            |_| "mixed recursion: recursive control stops must be total pure rules",
                        )?;
                        Some(lower.pure.set(r)?)
                    }
                    _ => None,
                };
                let filter = self.filter(c.rule(), lower)?;
                Instruction::Search {
                    walk: Walk {
                        direction,
                        end: !matches!(s, StopBy::Neighbor),
                        field: None,
                        stop,
                        filter,
                    },
                    body: self.compile(c.rule(), lower)?,
                }
            }
            AstMatcher::Field(f) if matches!(f.stop, StopBy::Neighbor) => Instruction::Search {
                walk: Walk {
                    direction: if f.inside {
                        Direction::Parent
                    } else {
                        Direction::Child
                    },
                    end: false,
                    field: Some(f.field),
                    stop: None,
                    filter: self.filter(&f.rule, lower)?,
                },
                body: self.compile(&f.rule, lower)?,
            },
            _ => {
                return Err(
                    "mixed recursion: recursive call crosses an unproved control boundary".into(),
                );
            }
        };
        self.push(instruction)
    }
    fn push(&mut self, i: Instruction) -> Result<usize, String> {
        if self.program.code.len() >= 255 {
            return Err("mixed recursion: emitted control exceeds 256 instructions".into());
        }
        let pc = self.program.code.len();
        self.program.code.push(i);
        Ok(pc)
    }
    fn filter(&mut self, m: &AstMatcher, lower: &mut Lower) -> Result<Option<Set>, String> {
        if self.program.three {
            return Ok(None);
        }
        matcher::walk_candidate_filter(m)
            .map(|(kinds, clauses)| {
                let mut tests = vec![Set::Kinds(kinds)];
                for c in clauses {
                    let regex = regex::Regex::new(
                        &c.iter()
                            .map(|s| regex::escape(s))
                            .collect::<Vec<_>>()
                            .join("|"),
                    )
                    .map_err(|e| e.to_string())?;
                    tests.push(lower.pure.set(&AstMatcher::Regex(regex))?);
                }
                Ok(Set::Intersect(tests))
            })
            .transpose()
    }
}
impl Lower {
    pub(in crate::code_rule_dsl::sql::relational::outcomes) fn mixed_recursive(
        &mut self,
        u: &UtilRef,
    ) -> Result<Option<Op>, String> {
        if !self.three_valued {
            return Ok(None);
        }
        let key = (u.table.as_ptr() as usize, u.index);
        let context = (
            key.0,
            key.1,
            self.namespace.clone(),
            self.parsed_depth > 0,
            self.hosted_depth > 0,
        );
        if let Some((entry, program)) = self.mixed_components.get(&context) {
            return Ok(Some(Op::MixedRecursive {
                program: Rc::clone(program),
                entry: *entry,
            }));
        }
        let mut source = Source {
            keys: BTreeMap::new(),
            refs: vec![],
            bodies: vec![],
            edges: vec![],
            operators: 0,
        };
        if source.utility(u).is_err() {
            return Ok(None);
        }
        let Some(members) = source.component() else {
            return Ok(None);
        };
        for &id in &members {
            let mut a = matcher::CaptureAnalysis::default();
            if !a.binds(&source.bodies[id]).is_empty() || !a.counts(&source.bodies[id]).is_empty() {
                return Ok(None);
            }
        }
        // Prefer the indexed ordered-arm scheduler for its complete singleton
        // domain. A false flag is a finite base, which must not hide SCC edges.
        if members.len() == 1
            && let AstMatcher::Any(items) = &source.bodies[0]
            && let Some(recursive) =
                super::recursive_search_lower::singleton_arms(&source.bodies[0], key)
            && items
                .iter()
                .zip(recursive)
                .all(|(item, recursive)| recursive || !source.recursive(item, &members))
        {
            return Ok(None);
        }
        let active: Vec<_> = members
            .iter()
            .map(|id| {
                let r = &source.refs[*id];
                (r.table.as_ptr() as usize, r.index)
            })
            .collect();
        if active.iter().any(|key| self.active.contains(key)) {
            return Ok(None);
        }
        let dense: BTreeMap<_, _> = members
            .iter()
            .enumerate()
            .map(|(dense, id)| (*id, dense))
            .collect();
        let functions = members
            .iter()
            .map(|_| {
                let memo = 4_000_000 + self.mixed_functions;
                self.mixed_functions += 1;
                Function { entry: 0, memo }
            })
            .collect();
        let mut builder = Builder {
            source: &source,
            members: &members,
            dense,
            program: Program {
                code: vec![Instruction::Dispatch, Instruction::Failure],
                functions,
                regions: vec![],
                mentions: Slots::new(),
                bypass: self.parsed_depth > 0 || self.hosted_depth > 0,
                three: self.three_valued,
            },
        };
        for key in &active {
            self.active.insert(*key);
        }
        let built = (|| {
            for &id in &members {
                let dense = builder.dense[&id];
                let pc = source.refs[id]
                    .with_target(|body| builder.compile(body, self))
                    .ok_or("mixed recursion: missing body")??;
                builder.program.functions[dense].entry = pc;
            }
            Ok::<_, String>(())
        })();
        for key in &active {
            self.active.remove(key);
        }
        built?;
        let program = Rc::new(builder.program);
        for &id in &members {
            let dense = members
                .iter()
                .position(|candidate| *candidate == id)
                .unwrap();
            let r = &source.refs[id];
            let key = (r.table.as_ptr() as usize, r.index);
            let scope = r
                .scope()
                .flatten()
                .map(|names| names.iter().map(|name| name.to_string()).collect());
            self.utility_origins
                .insert(program.functions[dense].memo, (key, scope));
            self.mixed_components.insert(
                (
                    key.0,
                    key.1,
                    self.namespace.clone(),
                    self.parsed_depth > 0,
                    self.hosted_depth > 0,
                ),
                (dense, Rc::clone(&program)),
            );
        }
        Ok(Some(Op::MixedRecursive { program, entry: 0 }))
    }
}
