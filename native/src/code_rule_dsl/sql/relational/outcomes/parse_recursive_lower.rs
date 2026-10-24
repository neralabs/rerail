//! Recursive utility components whose every cycle crosses a fresh parsed document.
use super::{CaptureKind, Lower, Op, Plan, Rc, Slots};
use crate::code_rule_dsl::matcher::{self, AstMatcher, Semantic, StopBy, UtilRef};
use crate::code_rule_dsl::sql::relational::{ir::Set, recursive_chain::pure};
use std::collections::{BTreeMap, BTreeSet};
type Key = (usize, usize);
pub(super) type Components =
    std::collections::HashMap<(usize, usize, String), (usize, Rc<Program>)>;
pub(in crate::code_rule_dsl::sql::relational::outcomes) enum Instruction {
    Return,
    Eval(usize),
    Call(usize),
    Boolean {
        all: bool,
        children: Vec<(usize, bool)>,
    },
    Not(usize),
    Search {
        end: bool,
        body: usize,
        strict: bool,
        filter: Option<Set>,
    },
    Parse {
        term: usize,
        body: usize,
        exports: Vec<usize>,
    },
}
pub(in crate::code_rule_dsl::sql::relational::outcomes) struct Program {
    pub code: Vec<Instruction>,
    pub functions: Vec<usize>,
    pub outputs: Vec<Slots>,
    pub regions: Vec<Plan>,
    pub mentions: Slots,
    pub writes: Slots,
    pub three: bool,
}
struct Source {
    keys: BTreeMap<Key, usize>,
    refs: Vec<UtilRef>,
    edges: Vec<(usize, usize, i8)>,
    operators: usize,
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
                self.edges.push((owner, id, progress));
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
                    self.walk(owner, s, progress)?;
                }
                self.walk(owner, c.rule(), progress)?;
            }
            AstMatcher::Field(f) => {
                if let StopBy::Rule(s) = &f.stop {
                    self.walk(owner, s, progress)?;
                }
                self.walk(owner, &f.rule, progress)?;
            }
            AstMatcher::Constrained(c) => {
                self.walk(owner, &c.rule, progress)?;
                for (_, r) in &c.constraints {
                    self.walk(owner, r, progress)?;
                }
            }
            AstMatcher::Count(c) => self.walk(owner, &c.relation, progress)?,
            AstMatcher::NthChild(c) => {
                if let Some(r) = &c.of_rule {
                    self.walk(owner, r, progress)?;
                }
            }
            AstMatcher::Semantic(s) => match s {
                Semantic::Binding(t) => {
                    for c in t.sub_rules() {
                        self.walk(owner, c, progress)?;
                    }
                }
                Semantic::Binds(t) => {
                    for c in t.sub_rules() {
                        self.walk(owner, c, progress)?;
                    }
                }
                Semantic::Parse(t) => self.walk(owner, &t.rule, 2)?,
                Semantic::Host(_) => return Err(()),
                Semantic::FormatArgs(t) => {
                    for c in [t.unused.as_ref(), t.missing.as_ref()]
                        .into_iter()
                        .flatten()
                    {
                        self.walk(owner, c, progress)?;
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

        let mut zero = vec![vec![false; n]; n];
        for &(a, b, d) in &self.edges {
            if !members.contains(&a) || !members.contains(&b) {
                continue;
            }
            if d == 0 {
                zero[a][b] = true;
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
            let plan = lower.plan(m)?;
            lower.admit_parsed_child(&plan)?;
            if plan
                .writes
                .iter()
                .any(|slot| lower.slots[*slot].kind != CaptureKind::Node)
            {
                return Err("parsed recursion: finite regions require single captures; list/count exports need a separate contract".into());
            }
            self.program.mentions.extend(&plan.mentions);
            self.program.writes.extend(&plan.writes);
            let id = self.program.regions.len();
            self.program.regions.push(plan);
            return self.push(Instruction::Eval(id));
        }
        let instruction = match m {
            AstMatcher::Util(u) => Instruction::Call(
                self.dense[&self.source.keys[&(u.table.as_ptr() as usize, u.index)]],
            ),
            AstMatcher::All(items) | AstMatcher::Any(items) => {
                let all = matches!(m, AstMatcher::All(_));
                // Only one recursive term may occur in a conjunction. Every other
                // term is total and capture-free, so unknown continuation needs no
                // withheld-name set and still evaluates all reached pure suffixes.
                if all {
                    let mut recursive = 0;
                    for item in items {
                        if self.source.recursive(item, self.members) {
                            recursive += 1;
                        } else {
                            pure(item).map_err(|_| "parsed recursion: conjunction siblings must be total capture-free metadata")?;
                        }
                    }
                    if recursive > 1 {
                        return Err(
                            "parsed recursion: conjunction has multiple recursive continuations"
                                .into(),
                        );
                    }
                }
                let children = items
                    .iter()
                    .map(|item| {
                        let strict = !matcher::CaptureAnalysis::default().binds(item).is_empty();
                        self.compile(item, lower).map(|pc| (pc, strict))
                    })
                    .collect::<Result<_, _>>()?;
                Instruction::Boolean { all, children }
            }
            AstMatcher::Not(child) => Instruction::Not(self.compile(child, lower)?),
            AstMatcher::Has(child, stop) if matches!(stop, StopBy::Neighbor | StopBy::End) => {
                let filter = if self.program.three {
                    None
                } else {
                    matcher::walk_candidate_filter(child.rule())
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
                                terms.push(lower.pure.set(&AstMatcher::Regex(regex))?);
                            }
                            Ok::<_, String>(Set::Intersect(terms))
                        })
                        .transpose()?
                };
                let strict = !matcher::CaptureAnalysis::default()
                    .binds(child.rule())
                    .is_empty();
                Instruction::Search {
                    end: matches!(stop, StopBy::End),
                    body: self.compile(child.rule(), lower)?,
                    strict,
                    filter,
                }
            }
            AstMatcher::Semantic(Semantic::Parse(test)) if test.host_sound.is_none() => {
                let term = lower.parse_atomic(test);
                let exports = test
                    .exports
                    .iter()
                    .map(|name| lower.slot(name, CaptureKind::Node))
                    .collect::<Result<Vec<_>, _>>()?;
                self.program.writes.extend(&exports);
                for slot in &exports {
                    lower.span_slots.insert(*slot);
                }
                Instruction::Parse {
                    term,
                    body: self.compile(&test.rule, lower)?,
                    exports,
                }
            }
            _ => {
                return Err(
                    "parsed recursion: recursive edge crosses an unsupported control boundary"
                        .into(),
                );
            }
        };
        self.push(instruction)
    }
    fn push(&mut self, instruction: Instruction) -> Result<usize, String> {
        if self.program.code.len() >= 256 {
            return Err("parsed recursion: control exceeds 256 instructions".into());
        }
        let pc = self.program.code.len();
        self.program.code.push(instruction);
        Ok(pc)
    }
}
impl Lower {
    pub(super) fn parsed_recursive(&mut self, util: &UtilRef) -> Result<Option<Op>, String> {
        if self.parsed_depth == 0 {
            return Ok(None);
        }
        let key = (
            util.table.as_ptr() as usize,
            util.index,
            self.namespace.clone(),
        );
        if let Some((entry, program)) = self.parsed_components.get(&key) {
            return Ok(Some(Op::ParsedRecursive {
                program: Rc::clone(program),
                entry: *entry,
            }));
        }
        let mut source = Source {
            keys: BTreeMap::new(),
            refs: vec![],
            edges: vec![],
            operators: 0,
        };
        if source.utility(util).is_err() {
            return Ok(None);
        }
        let Some(members) = source.component() else {
            return Ok(None);
        };
        let mut builder = Builder {
            source: &source,
            members: &members,
            dense: members
                .iter()
                .enumerate()
                .map(|(dense, id)| (*id, dense))
                .collect(),
            program: Program {
                code: vec![Instruction::Return],
                functions: vec![0; members.len()],
                outputs: vec![Slots::new(); members.len()],
                regions: vec![],
                mentions: Slots::new(),
                writes: Slots::new(),
                three: self.three_valued,
            },
        };
        // Members are emitted directly from their original ASTs. They never re-enter
        // normal utility lowering, and finite regions cannot contain SCC backedges.
        for &id in &members {
            let entry = source.refs[id]
                .with_target(|body| builder.compile(body, self))
                .ok_or("parsed recursion: missing utility")??;
            let outputs = source.refs[id]
                .with_target(|body| {
                    matcher::CaptureAnalysis::default()
                        .binds(body)
                        .iter()
                        .map(|name| self.slot(name, CaptureKind::Node))
                        .collect::<Result<Slots, String>>()
                })
                .ok_or("parsed recursion: missing output contract")??;
            builder.program.functions[builder.dense[&id]] = entry;
            builder.program.outputs[builder.dense[&id]] = outputs;
        }
        let program = Rc::new(builder.program);
        for &id in &members {
            let r = &source.refs[id];
            self.parsed_components.insert(
                (r.table.as_ptr() as usize, r.index, self.namespace.clone()),
                (builder.dense[&id], Rc::clone(&program)),
            );
        }
        Ok(Some(Op::ParsedRecursive { program, entry: 0 }))
    }
}
