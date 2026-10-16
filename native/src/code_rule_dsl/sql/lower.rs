//! Lowering from the compiled `AstMatcher` tree to the frame IR (`plan`). Term order,
//! `order_by_captures`, `Never` folding and grammar gaps stay the front end's: this walks what it
//! compiled. An operator the SQL engine does not cover yet fails with `Unsupported`; the engine
//! never answers for a clause it cannot lower.

use super::emit;
use super::plan::{
    CallReturn, CallSlot, OfRule, Op, Slot, SlotId, SlotKind, Statement, Stop, Unsupported, Walk,
};
use super::terminals::{PATTERN_COLUMNS, PatternTerm, Terminal};
use crate::code_rule_dsl::language::GrammarDialect;
use crate::code_rule_dsl::matcher::{
    AstMatcher, CaptureAnalysis, GrammarMatcher, Related, Semantic, StopBy, UtilRef,
};
use ast_grep_core::matcher::Matcher;
use std::collections::{BTreeSet, HashMap};

/// A per-batch shared stage: a rule matched in a fresh environment at every node of its root kinds
/// in the files of its dialect, computed once per batch into `stage_<index>`. Stop rules and
/// context-free utilities are stages; equal ones (equal SQL) are one, keyed by digest. The last
/// of `statements` selects the stage's distinct `(file, node)` rows; its root frame reads the
/// `/*FILES*/` the driver substitutes.
pub(super) struct Stage {
    pub(super) dialect: GrammarDialect,
    pub(super) statements: Vec<String>,
}

/// The terminals and stages a set of clauses shares.
#[derive(Default)]
pub(super) struct ProgramBuilder {
    pub(super) terminals: Vec<Terminal>,
    pub(super) stages: Vec<Stage>,
    stage_index: HashMap<String, usize>,
    regexes: HashMap<(String, String), usize>,
    fresh_patterns: HashMap<usize, usize>,
}

impl ProgramBuilder {
    /// Lowers one clause's grammar matcher to its statement.
    pub(super) fn clause(&mut self, grammar: &GrammarMatcher) -> Result<Statement, Unsupported> {
        if grammar.three_valued {
            return Err(Unsupported("three-valued clause".into()));
        }
        let kinds = grammar
            .anchors
            .root_kinds()
            .map(|kinds| kinds.iter().copied().collect());
        let mut lowerer = Lowerer::new(self, grammar.language, Vec::new());
        let (op, _) = lowerer.lower(&grammar.matcher, 0, &BTreeSet::new())?;
        Ok(Statement {
            slots: lowerer.slots,
            kinds,
            op,
        })
    }
}

type Bound = BTreeSet<SlotId>;

struct Lowerer<'p> {
    program: &'p mut ProgramBuilder,
    dialect: GrammarDialect,
    slots: Vec<Slot>,
    index: HashMap<(usize, String, SlotKind), SlotId>,
    scopes: usize,
    /// The utilities being lowered, innermost last: one met again is recursive.
    utils: Vec<(usize, usize)>,
}

fn unsupported<T>(operator: &str) -> Result<T, Unsupported> {
    Err(Unsupported(operator.to_owned()))
}

/// The utility identity of a reference: its table and index.
fn util_key(util: &UtilRef) -> (usize, usize) {
    (util.table.as_ptr() as usize, util.index)
}

/// Whether a rule reads, binds or counts no capture: its verdict at a node is the node's alone.
fn capture_free(rule: &AstMatcher) -> bool {
    let mut analysis = CaptureAnalysis::default();
    analysis.mentions(rule).is_empty() && analysis.counts(rule).is_empty()
}

/// The kinds a node must have for `rule` to match it; `None` where any may.
fn root_kinds(rule: &AstMatcher, depth: usize) -> Option<BTreeSet<u16>> {
    if depth > 32 {
        return None;
    }
    match rule {
        AstMatcher::Pattern(pattern) => pattern
            .potential_kinds()
            .map(|kinds| kinds.iter().map(|kind| kind as u16).collect()),
        AstMatcher::Kind(kind) => kind
            .potential_kinds()
            .map(|kinds| kinds.iter().map(|kind| kind as u16).collect()),
        AstMatcher::Any(items) => {
            let mut kinds = BTreeSet::new();
            for item in items {
                kinds.extend(root_kinds(item, depth + 1)?);
            }
            Some(kinds)
        }
        AstMatcher::All(items) => items
            .iter()
            .filter_map(|item| root_kinds(item, depth + 1))
            .reduce(|a, b| a.intersection(&b).copied().collect()),
        AstMatcher::Constrained(core) => root_kinds(&core.rule, depth + 1),
        AstMatcher::Util(util) => util
            .with_target(|rule| root_kinds(rule, depth + 1))
            .flatten(),
        AstMatcher::Never => Some(BTreeSet::new()),
        _ => None,
    }
}

/// A relation's sub-rule the frame algebra cannot express: an `nthChild` whose `ofRule` binds,
/// directly (or as a utility's body). The matcher commits `ofRule`'s bindings at the node before
/// it tests the position (`NthChild::index`), and a relation walk tests each candidate in the
/// walk's own environment, so a candidate whose position fails leaves its binding for the later
/// candidates: they are not independent, and no argmin over them reproduces that.
fn walk_leak(sub: &Op) -> Result<(), Unsupported> {
    match sub {
        Op::NthChild {
            of_rule: Some(OfRule::Frame(rule)),
            ..
        } if rule.binds() => unsupported("nthChild with a binding ofRule as a relation's rule"),
        _ => Ok(()),
    }
}

fn kind_id(kind: &ast_grep_core::matcher::KindMatcher) -> u16 {
    kind.potential_kinds()
        .and_then(|kinds| kinds.iter().next())
        .map_or(u16::MAX, |kind| kind as u16)
}

impl<'p> Lowerer<'p> {
    fn new(
        program: &'p mut ProgramBuilder,
        dialect: GrammarDialect,
        utils: Vec<(usize, usize)>,
    ) -> Self {
        Self {
            program,
            dialect,
            slots: Vec::new(),
            index: HashMap::new(),
            scopes: 0,
            utils,
        }
    }

    fn slot(&mut self, scope: usize, name: &str, kind: SlotKind) -> SlotId {
        if let Some(&slot) = self.index.get(&(scope, name.to_owned(), kind)) {
            return slot;
        }
        let id = self.slots.len();
        self.slots.push(Slot {
            name: name.to_owned(),
            kind,
            scope,
        });
        self.index.insert((scope, name.to_owned(), kind), id);
        id
    }

    fn existing(&self, scope: usize, name: &str, kind: SlotKind) -> Option<SlotId> {
        self.index.get(&(scope, name.to_owned(), kind)).copied()
    }

    /// A stage for `rule`, matched in a fresh environment: lowered, emitted and keyed by its SQL.
    fn stage(&mut self, rule: &AstMatcher) -> Result<usize, Unsupported> {
        let kinds = root_kinds(rule, 0).map(|kinds| kinds.into_iter().collect());
        let statement = {
            let mut lowerer = Lowerer::new(self.program, self.dialect, self.utils.clone());
            let (op, _) = lowerer.lower(rule, 0, &BTreeSet::new())?;
            Statement {
                slots: lowerer.slots,
                kinds,
                op,
            }
        };
        let statements = emit::stage(&statement);
        let digest = format!("{}:{}", self.dialect, statements.join(";\n"));
        if let Some(&existing) = self.program.stage_index.get(&digest) {
            return Ok(existing);
        }
        let id = self.program.stages.len();
        self.program.stage_index.insert(digest, id);
        self.program.stages.push(Stage {
            dialect: self.dialect,
            statements,
        });
        Ok(id)
    }

    fn stop(&mut self, stop: &StopBy) -> Result<Stop, Unsupported> {
        Ok(match stop {
            StopBy::Neighbor => Stop::Neighbor,
            StopBy::End => Stop::End,
            StopBy::Rule(rule) => Stop::Rule(self.stage(rule)?),
        })
    }

    /// A relation's walk and its rule, from `inside`/`has`/`precedes`/`follows`.
    fn relation<'m>(
        &mut self,
        matcher: &'m AstMatcher,
    ) -> Result<Option<(Walk, &'m Related)>, Unsupported> {
        Ok(Some(match matcher {
            AstMatcher::Inside(related, stop) => (Walk::Inside(self.stop(stop)?), &**related),
            AstMatcher::Has(related, stop) => (Walk::Has(self.stop(stop)?), &**related),
            AstMatcher::Precedes(related, stop) => (Walk::Precedes(self.stop(stop)?), &**related),
            AstMatcher::Follows(related, stop) => (Walk::Follows(self.stop(stop)?), &**related),
            _ => return Ok(None),
        }))
    }

    fn lower(
        &mut self,
        matcher: &AstMatcher,
        scope: usize,
        bound: &Bound,
    ) -> Result<(Op, Bound), Unsupported> {
        match matcher {
            AstMatcher::Pattern(pattern) => {
                let mut term = PatternTerm::new(pattern.clone(), self.dialect);
                if term.width() > PATTERN_COLUMNS {
                    return unsupported("pattern with more than 32 capture columns");
                }
                let vars: Vec<SlotId> = term
                    .vars
                    .iter()
                    .map(|(name, kind)| self.slot(scope, name, *kind))
                    .collect();
                term.reads = (0..vars.len())
                    .filter(|&index| bound.contains(&vars[index]))
                    .collect();
                let reads: Vec<SlotId> = term.reads.iter().map(|&index| vars[index]).collect();
                let env = !reads.is_empty();
                let key = std::ptr::from_ref(pattern) as usize;
                let id = match (env, self.program.fresh_patterns.get(&key)) {
                    (false, Some(&id)) => id,
                    _ => {
                        let id = self.program.terminals.len();
                        self.program.terminals.push(Terminal::Pattern(term));
                        if !env {
                            self.program.fresh_patterns.insert(key, id);
                        }
                        id
                    }
                };
                let mut out = bound.clone();
                out.extend(vars.iter().copied());
                Ok((
                    Op::Pattern {
                        term: id,
                        env,
                        vars,
                        reads,
                    },
                    out,
                ))
            }
            AstMatcher::Kind(kind) => Ok((Op::Kinds(vec![kind_id(kind)]), bound.clone())),
            AstMatcher::Regex(regex) => {
                let key = (self.dialect.to_string(), regex.as_str().to_owned());
                let id = match self.program.regexes.get(&key) {
                    Some(&id) => id,
                    None => {
                        let id = self.program.terminals.len();
                        self.program.terminals.push(Terminal::Regex(regex.clone()));
                        self.program.regexes.insert(key, id);
                        id
                    }
                };
                Ok((Op::Regex(id), bound.clone()))
            }
            AstMatcher::All(items) => {
                let mut ops = Vec::with_capacity(items.len());
                let mut current = bound.clone();
                for item in items {
                    let (op, out) = self.lower(item, scope, &current)?;
                    ops.push(op);
                    current = out;
                }
                Ok((Op::All(ops), current))
            }
            AstMatcher::Any(items) => {
                if items.iter().all(|item| matches!(item, AstMatcher::Kind(_))) {
                    let kinds = items
                        .iter()
                        .filter_map(|item| match item {
                            AstMatcher::Kind(kind) => Some(kind_id(kind)),
                            _ => None,
                        })
                        .collect();
                    return Ok((Op::Kinds(kinds), bound.clone()));
                }
                let mut ops = Vec::with_capacity(items.len());
                let mut out = bound.clone();
                for item in items {
                    let (op, branch) = self.lower(item, scope, bound)?;
                    ops.push(op);
                    out.extend(branch);
                }
                Ok((Op::Any(ops), out))
            }
            AstMatcher::Not(inner) => {
                let (op, _) = self.lower(inner, scope, bound)?;
                Ok((Op::Not(Box::new(op)), bound.clone()))
            }
            AstMatcher::Never => Ok((Op::Never, bound.clone())),
            AstMatcher::Inside(..)
            | AstMatcher::Has(..)
            | AstMatcher::Precedes(..)
            | AstMatcher::Follows(..) => {
                let (walk, related) = self.relation(matcher)?.expect("a relation");
                let (sub, out) = self.lower(related.rule(), scope, bound)?;
                walk_leak(&sub)?;
                let binds = sub.binds();
                Ok((
                    Op::Relation {
                        walk,
                        sub: Box::new(sub),
                        binds,
                    },
                    out,
                ))
            }
            AstMatcher::Field(relation) => {
                let stop = self.stop(&relation.stop)?;
                let walk = if relation.inside {
                    Walk::FieldInside(relation.field, stop)
                } else {
                    Walk::FieldHas(relation.field, stop)
                };
                let (sub, out) = self.lower(&relation.rule, scope, bound)?;
                walk_leak(&sub)?;
                let binds = sub.binds();
                Ok((
                    Op::Relation {
                        walk,
                        sub: Box::new(sub),
                        binds,
                    },
                    out,
                ))
            }
            AstMatcher::NthChild(nth) => {
                let (of_rule, out) = match &nth.of_rule {
                    None => (None, bound.clone()),
                    Some(rule) if capture_free(rule) => {
                        (Some(OfRule::Stage(self.stage(rule)?)), bound.clone())
                    }
                    Some(rule) => {
                        let (op, out) = self.lower(rule, scope, bound)?;
                        (Some(OfRule::Frame(Box::new(op))), out)
                    }
                };
                Ok((
                    Op::NthChild {
                        step: nth.position.step,
                        offset: nth.position.offset,
                        reverse: nth.reverse,
                        of_rule,
                    },
                    out,
                ))
            }
            AstMatcher::Count(count) => {
                let Some((walk, related)) = self.relation(&count.relation)? else {
                    // A count over any other term counts nothing and holds nowhere.
                    return Ok((Op::Never, bound.clone()));
                };
                let (sub, _) = self.lower(related.rule(), scope, bound)?;
                let mut out = bound.clone();
                let bind = count.binding.as_ref().map(|binding| {
                    let slot = self.slot(scope, &binding.name, SlotKind::Count);
                    out.insert(slot);
                    (slot, binding.offset)
                });
                Ok((
                    Op::Count {
                        walk,
                        sub: Box::new(sub),
                        min: count.min,
                        max: count.max,
                        bind,
                    },
                    out,
                ))
            }
            AstMatcher::Util(util) => {
                let key = util_key(util);
                if self.utils.contains(&key) {
                    return unsupported("recursive utility");
                }
                self.utils.push(key);
                let lowered = util
                    .with_target(|rule| {
                        if capture_free(rule) {
                            self.stage(rule)
                                .map(|stage| (Op::Stage(stage), bound.clone()))
                        } else {
                            self.lower(rule, scope, bound)
                        }
                    })
                    .unwrap_or_else(|| unsupported("unfilled utility"));
                self.utils.pop();
                lowered
            }
            AstMatcher::Call(call) => {
                let key = util_key(&call.util);
                if self.utils.contains(&key) {
                    return unsupported("recursive utility");
                }
                self.scopes += 1;
                let callee = self.scopes;
                let kinds = [SlotKind::Node, SlotKind::List, SlotKind::Count];
                // The parameters the caller's bindings may fill on entry.
                let mut entry_bound = Bound::new();
                for (param, caller) in &call.args {
                    for kind in kinds {
                        if let Some(slot) = self.existing(scope, caller, kind)
                            && bound.contains(&slot)
                        {
                            entry_bound.insert(self.slot(callee, param, kind));
                        }
                    }
                }
                self.utils.push(key);
                let lowered = call
                    .util
                    .with_target(|rule| self.lower(rule, callee, &entry_bound))
                    .unwrap_or_else(|| unsupported("unfilled utility"));
                self.utils.pop();
                let (body, body_out) = lowered?;
                let mut entry = Vec::new();
                for (param, caller) in &call.args {
                    for kind in kinds {
                        if let (Some(callee_slot), Some(caller_slot)) = (
                            self.existing(callee, param, kind),
                            self.existing(scope, caller, kind),
                        ) {
                            entry.push(CallSlot {
                                kind,
                                callee: callee_slot,
                                caller: caller_slot,
                            });
                        }
                    }
                }
                let mut returns = Vec::new();
                let mut out = bound.clone();
                let mut callers: Vec<&str> = Vec::new();
                for (_, caller) in &call.args {
                    if !callers.contains(&caller.as_str()) {
                        callers.push(caller);
                    }
                }
                for caller in callers {
                    for kind in kinds {
                        let params: Vec<SlotId> = call
                            .args
                            .iter()
                            .filter(|(_, mapped)| mapped == caller)
                            .filter_map(|(param, _)| self.existing(callee, param, kind))
                            .filter(|slot| body_out.contains(slot))
                            .collect();
                        if params.is_empty() {
                            continue;
                        }
                        if params.len() > 4 {
                            return unsupported(
                                "call mapping more than 4 parameters to one capture",
                            );
                        }
                        let caller_slot = self.slot(scope, caller, kind);
                        out.insert(caller_slot);
                        returns.push(CallReturn {
                            kind,
                            caller: caller_slot,
                            params,
                        });
                    }
                }
                Ok((
                    Op::Call {
                        entry,
                        body: Box::new(body),
                        returns,
                    },
                    out,
                ))
            }
            AstMatcher::Constrained(core) => {
                let (rule, mut current) = self.lower(&core.rule, scope, bound)?;
                let mut constraints = Vec::new();
                for (name, constraint) in &core.constraints {
                    let Some(slot) = self.existing(scope, name, SlotKind::Node) else {
                        // Never bound, so never checked.
                        continue;
                    };
                    let (op, out) = self.lower(constraint, scope, &current)?;
                    constraints.push((slot, op));
                    current = out;
                }
                Ok((
                    Op::Constrained {
                        rule: Box::new(rule),
                        constraints,
                    },
                    current,
                ))
            }
            AstMatcher::Compare(_) => unsupported("compare"),
            AstMatcher::Semantic(semantic) => unsupported(match semantic {
                Semantic::Resolves(_) => "resolves",
                Semantic::Binding(_) => "resolves.binding",
                Semantic::Binds(_) => "binds",
                Semantic::Value(_) => "value",
                Semantic::Arguments(_) => "arguments",
                Semantic::Parse(_) => "parse",
                Semantic::Host(_) => "host",
                Semantic::Capture(_) => "capture",
                Semantic::FormatArgs(_) => "formatArgs",
                Semantic::Layout(_) => "layout",
                Semantic::Calls(_) => "calls",
            }),
            #[cfg(test)]
            AstMatcher::TestUnknown(_) => unsupported("testUnknown"),
        }
    }
}
