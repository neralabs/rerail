//! Compile an ordered capture-demand frontier from the original matcher.
//! This is an intermediate relation, not a clause result. Later filters stay outside its choices.
use super::{
    demand, emit,
    ir::{CaptureDemand, Program, Set},
    lower::Lowerer,
    physical,
};
use crate::code_rule_dsl::matcher::{AstMatcher, CaptureAnalysis, GrammarMatcher, StopBy};
use ast_grep_core::{matcher::PatternNode, meta_var::MetaVariable};
use std::collections::{BTreeSet, HashSet};

#[derive(Clone)]
pub(super) struct Search {
    pub site: usize,
    pub origins: AstMatcher,
    pub ancestor_filter: AstMatcher,
    pub ancestor_stop: AstMatcher,
    pub leaf_filter: AstMatcher,
    pub leaf_stop: Option<AstMatcher>,
    pub capture: String,
    pub named: bool,
    /// Native oracle only. Never called by SQL execution.
    pub matcher: AstMatcher,
}

pub(super) struct CompiledDemand {
    pub sql: String,
    pub program: Program,
    pub profile: serde_json::Value,
}

pub(super) fn pure(rule: &AstMatcher) -> bool {
    CaptureAnalysis::default().mentions(rule).is_empty()
        && CaptureAnalysis::default().counts(rule).is_empty()
}
fn all(mut items: Vec<AstMatcher>) -> AstMatcher {
    if items.len() == 1 {
        items.pop().unwrap()
    } else {
        AstMatcher::All(items)
    }
}

/// Flatten only conjunctions and utility aliases. Never cross an ordered alternative or walk.
fn terms(
    rule: &AstMatcher,
    active: &mut HashSet<(usize, usize)>,
) -> Result<Vec<AstMatcher>, String> {
    match rule {
        AstMatcher::All(items) => {
            let mut out = Vec::new();
            for item in items {
                out.extend(terms(item, active)?);
            }
            Ok(out)
        }
        AstMatcher::Util(u) if !pure(rule) => {
            let key = (u.table.as_ptr() as usize, u.index);
            if !active.insert(key) {
                return Err(
                    "capture demand: recursive capture producer needs a component plan".into(),
                );
            }
            let result = u
                .with_target(|r| terms(r, active))
                .ok_or("capture demand: missing utility")?;
            active.remove(&key);
            result
        }
        _ => Ok(vec![rule.clone()]),
    }
}

/// Necessary same-node predicates only. Capture-bearing negatives cannot restrict this domain.
fn guard(rule: &AstMatcher) -> AstMatcher {
    if pure(rule) {
        return rule.clone();
    }
    match rule {
        AstMatcher::All(items) => all(items.iter().map(guard).collect()),
        AstMatcher::Any(items) => AstMatcher::Any(items.iter().map(guard).collect()),
        _ => AstMatcher::All(vec![]),
    }
}

fn producer(
    rule: &AstMatcher,
) -> Result<(AstMatcher, AstMatcher, Option<AstMatcher>, String, bool), String> {
    let mut predicates = Vec::new();
    let mut search = None;
    for term in terms(rule, &mut HashSet::new())? {
        if pure(&term) {
            predicates.push(term);
            continue;
        }
        match term {
            AstMatcher::Has(body, stop) if search.is_none() => search = Some((body, stop)),
            _ => return Err(
                "capture demand: producer needs one descendant binding search and pure conjuncts"
                    .into(),
            ),
        }
    }
    let (body, stop) = search.ok_or("capture demand: no descendant binding search")?;
    let leaf_stop = match stop {
        StopBy::End => None,
        StopBy::Rule(r) if pure(&r) => Some(*r),
        _ => {
            return Err(
                "capture demand: descendant search needs end or a pure inclusive stop".into(),
            );
        }
    };
    let mut leaf = Vec::new();
    let mut capture = None;
    for term in terms(&body, &mut HashSet::new())? {
        if pure(&term) {
            leaf.push(term);
            continue;
        }
        match term {
            AstMatcher::Pattern(p) => match &p.node {
                PatternNode::MetaVar {
                    meta_var: MetaVariable::Capture(name, named),
                } if capture.is_none() => capture = Some((name.clone(), *named)),
                _ => {
                    return Err(
                        "capture demand: only a single whole-node capture is admitted".into(),
                    );
                }
            },
            _ => {
                return Err(
                    "capture demand: leaf binding has an unsupported capture effect".into(),
                );
            }
        }
    }
    let (capture, named) = capture.ok_or("capture demand: missing whole-node capture")?;
    Ok((all(predicates), all(leaf), leaf_stop, capture, named))
}

/// Names present after every successful path. A union of possible bindings is not sufficient.
fn definite(rule: &AstMatcher, active: &mut HashSet<(usize, usize)>) -> BTreeSet<String> {
    match rule {
        AstMatcher::Pattern(p) => p.defined_vars().into_iter().map(str::to_owned).collect(),
        AstMatcher::All(items) => items.iter().flat_map(|r| definite(r, active)).collect(),
        AstMatcher::Any(items) => items
            .iter()
            .map(|r| definite(r, active))
            .reduce(|a, b| a.intersection(&b).cloned().collect())
            .unwrap_or_default(),
        AstMatcher::Inside(r, _)
        | AstMatcher::Has(r, _)
        | AstMatcher::Follows(r, _)
        | AstMatcher::Precedes(r, _) => definite(r, active),
        AstMatcher::Util(u) => {
            let key = (u.table.as_ptr() as usize, u.index);
            if !active.insert(key) {
                return BTreeSet::new();
            }
            let result = u.with_target(|r| definite(r, active)).unwrap_or_default();
            active.remove(&key);
            result
        }
        _ => BTreeSet::new(),
    }
}

struct Discovery {
    searches: Vec<Search>,
    active: HashSet<(usize, usize)>,
    seen: HashSet<(usize, usize, Vec<String>)>,
}
impl Discovery {
    fn visit(
        &mut self,
        rule: &AstMatcher,
        context: AstMatcher,
        bound: &BTreeSet<String>,
    ) -> Result<(), String> {
        if pure(rule) {
            return Ok(());
        }
        match rule {
            AstMatcher::All(items) => {
                let context = all(vec![context, guard(rule)]);
                let mut bound = bound.clone();
                for item in items {
                    self.visit(item, context.clone(), &bound)?;
                    bound.extend(definite(item, &mut HashSet::new()));
                }
            }
            AstMatcher::Any(items) => {
                for item in items {
                    self.visit(item, context.clone(), bound)?;
                }
            }
            AstMatcher::Not(_) => {} // No outward demand binding leaves negation.
            AstMatcher::Util(u) => {
                let key = (u.table.as_ptr() as usize, u.index);
                if !self.active.insert(key) {
                    return Ok(());
                }
                // A frontier's origins use its local necessary guard, not a consumer-specific subset.
                // Visiting a utility once therefore covers every consumer; witness choice stays keyed by origin.
                if self
                    .seen
                    .insert((key.0, key.1, bound.iter().cloned().collect()))
                {
                    u.with_target(|r| self.visit(r, AstMatcher::All(vec![]), bound))
                        .ok_or("capture demand: missing utility")??;
                }
                self.active.remove(&key);
            }
            AstMatcher::Inside(body, StopBy::Rule(stop)) if !pure(body) => {
                if !pure(stop) {
                    return Err(
                        "capture demand: capture-bearing stop needs an outcome contract".into(),
                    );
                }
                let (ancestor_filter, leaf_filter, leaf_stop, capture, named) = producer(body)?;
                if !bound.contains(&capture) {
                    return Err(
                        "capture demand: ordered search requires an already bound capture key"
                            .into(),
                    );
                }
                self.searches.push(Search {
                    site: rule as *const AstMatcher as usize,
                    origins: context,
                    ancestor_filter,
                    ancestor_stop: *stop.clone(),
                    leaf_filter,
                    leaf_stop,
                    capture,
                    named,
                    matcher: rule.clone(),
                });
            }
            AstMatcher::Inside(body, _)
            | AstMatcher::Has(body, _)
            | AstMatcher::Follows(body, _)
            | AstMatcher::Precedes(body, _) => self.visit(body, AstMatcher::All(vec![]), bound)?,
            AstMatcher::Pattern(_) => {}
            _ => return Err("capture demand: unsupported effect in the discovery path".into()),
        }
        Ok(())
    }
}

pub(super) fn discover(grammar: &GrammarMatcher) -> Result<Vec<Search>, String> {
    if grammar.three_valued || grammar.transform.is_some() {
        return Err("capture demand: only total untransformed matchers are admitted".into());
    }
    let mut discovery = Discovery {
        searches: vec![],
        active: HashSet::new(),
        seen: HashSet::new(),
    };
    if grammar.applies {
        discovery.visit(&grammar.matcher, AstMatcher::All(vec![]), &BTreeSet::new())?;
    }
    if discovery.searches.is_empty() {
        return Err("capture demand: no admitted ordered frontier".into());
    }
    Ok(discovery.searches)
}

/// Compile one independently inspectable frontier. This does not claim complete clause coverage.
pub(super) fn lower_frontier(
    grammar: &GrammarMatcher,
    search: &Search,
) -> Result<(Lowerer, CaptureDemand, usize), String> {
    let mut lower = Lowerer::new(grammar.language);
    lower.relation(Set::Empty, "frontier outputs");
    let mut add = |rule: &AstMatcher, source: &str| -> Result<usize, String> {
        let set = lower.set(rule)?;
        Ok(lower.relation(set, source))
    };
    let origins = add(&search.origins, "origin candidates")?;
    let filter = add(&search.ancestor_filter, "ancestor predicate")?;
    let stop = add(&search.ancestor_stop, "origin search stop")?;
    let leaf_stop = search
        .leaf_stop
        .as_ref()
        .map(|s| add(s, "binding search stop"))
        .transpose()?;
    let roots = lower.relation(
        Set::SearchRoots {
            origins,
            filter,
            stop: Some(stop),
        },
        "ordered search roots",
    );
    let leaf = lower.set(&search.leaf_filter)?;
    let targets = lower.relation(
        Set::Intersect(vec![leaf, Set::Within(roots)]),
        "demanded binding candidates",
    );
    let mut outputs = vec![Set::Reference(roots), Set::Reference(targets)];
    outputs.extend(leaf_stop.map(Set::Reference));
    lower.program.relations[0].expression = Set::Union(outputs);
    Ok((
        lower,
        CaptureDemand {
            roots,
            targets,
            leaf_stop,
            named: search.named,
        },
        origins,
    ))
}

pub(super) fn compile(grammar: &GrammarMatcher, search: Search) -> Result<CompiledDemand, String> {
    let (mut lower, demand, origins) = lower_frontier(grammar, &search)?;
    let (roots, targets) = (demand.roots, demand.targets);
    physical::plan(&lower.program)?;
    demand::apply(&mut lower.program);
    let planned = physical::plan(&lower.program)?;
    let sql = emit::capture_demand(&lower.program, &planned, &demand)?;
    let profile = serde_json::json!({
        "scope":"Intermediate ordered capture demand extracted from the original GrammarMatcher; not complete clause findings.",
        "capture":search.capture,"origins":origins,"roots":roots,"targets":targets,
        "originKey":["file","origin"],"valueKey":["file","atom"],"witnessKey":["file","witness"],
        "choices":["First successful descendant in preorder per ancestor/value, before later predicates.","Nearest successful ancestor per origin/value, before later predicates."],
        "demandDecisions":lower.program.demand_decisions,
        "limitations":["Only already-bound leaf capture keys are admitted; composite binding candidates cause an execution error.","Native resource budgets and complete clause coverage remain pending."]
    });
    Ok(CompiledDemand {
        sql,
        program: lower.program,
        profile,
    })
}
