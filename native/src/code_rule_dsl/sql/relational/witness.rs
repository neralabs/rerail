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

/// A semantic-shape mismatch is a planning decision, not a failed compilation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ShapeReason {
    RecursiveProducer,
    ProducerShape,
    MissingDescendant,
    DescendantStop,
    WholeCapture,
    BindingEffect,
    MissingCapture,
    CaptureStop,
    BoundFrontierInput,
    DiscoveryEffect,
    TruthContract,
    NoFrontier,
    SameCapture,
    RecursiveAnswer,
    UnnamedCapture,
    MultipleProducers,
    MissingProducer,
    OutwardStop,
    OutwardOperator,
    MultipleProbeBindings,
    MissingProbePath,
    ProbeStop,
    ProbeOperator,
    BoundProbeInput,
    RecursiveInput,
    InputEffect,
    FrontierCount,
    PredicateContract,
    CountContract,
}
impl std::fmt::Display for ShapeReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::RecursiveProducer => {
                "capture demand: recursive capture producer needs a component plan"
            }
            Self::ProducerShape => {
                "capture demand: producer needs one descendant binding search and pure conjuncts"
            }
            Self::MissingDescendant => "capture demand: no descendant binding search",
            Self::DescendantStop => {
                "capture demand: descendant search needs end or a pure inclusive stop"
            }
            Self::WholeCapture => "capture demand: only a single whole-node capture is admitted",
            Self::BindingEffect => "capture demand: leaf binding has an unsupported capture effect",
            Self::MissingCapture => "capture demand: missing whole-node capture",
            Self::CaptureStop => "capture demand: capture-bearing stop needs an outcome contract",
            Self::BoundFrontierInput => {
                "capture demand: ordered search requires an already bound capture key"
            }
            Self::DiscoveryEffect => "capture demand: unsupported effect in the discovery path",
            Self::TruthContract => "capture demand: only total untransformed matchers are admitted",
            Self::NoFrontier => "capture demand: no admitted ordered frontier",
            Self::SameCapture => "ordered answers: expected the same whole-node capture",
            Self::RecursiveAnswer => {
                "ordered answers: positive recursive capture needs an ordered component plan"
            }
            Self::UnnamedCapture => "ordered answers: unnamed capture keys are not admitted",
            Self::MultipleProducers => {
                "ordered answers: conjunction needs one outward capture producer"
            }
            Self::MissingProducer => "ordered answers: binding without a demand producer",
            Self::OutwardStop => {
                "ordered answers: unsupported ancestor choice outside the frontier"
            }
            Self::OutwardOperator => "ordered answers: unsupported outward capture operator",
            Self::MultipleProbeBindings => {
                "negative probe: multiple capture operations at one path node"
            }
            Self::MissingProbePath => "negative probe: missing capture path",
            Self::ProbeStop => "negative probe: unsupported descendant stop",
            Self::ProbeOperator => "negative probe: unsupported value-dependent operator",
            Self::BoundProbeInput => {
                "ordered answers: negative value probe requires a bound input capture"
            }
            Self::RecursiveInput => "ordered answers: recursive outward capture is not admitted",
            Self::InputEffect => "ordered answers: unsupported input capture effect",
            Self::FrontierCount => {
                "ordered answers: exactly one independently bound frontier is currently admitted"
            }
            Self::PredicateContract => {
                "ordered answers: semantic or effectful operators require outcome relations"
            }
            Self::CountContract => "ordered answers: count effects require outcome relations",
        })
    }
}

/// Only explicit shape rejections permit choosing another physical representation.
/// Missing definitions, invalid graphs, limits, and emission errors remain fatal.
#[derive(Debug)]
pub(super) enum AdmissionError {
    NotApplicable(ShapeReason),
    Invalid(String),
}
impl From<ShapeReason> for AdmissionError {
    fn from(reason: ShapeReason) -> Self {
        Self::NotApplicable(reason)
    }
}
impl From<String> for AdmissionError {
    fn from(reason: String) -> Self {
        Self::Invalid(reason)
    }
}
impl From<&str> for AdmissionError {
    fn from(reason: &str) -> Self {
        Self::Invalid(reason.into())
    }
}
impl std::fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotApplicable(reason) => reason.fmt(f),
            Self::Invalid(reason) => f.write_str(reason),
        }
    }
}

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
) -> Result<Vec<AstMatcher>, AdmissionError> {
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
                return Err(ShapeReason::RecursiveProducer.into());
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
) -> Result<(AstMatcher, AstMatcher, Option<AstMatcher>, String, bool), AdmissionError> {
    let mut predicates = Vec::new();
    let mut search = None;
    for term in terms(rule, &mut HashSet::new())? {
        if pure(&term) {
            predicates.push(term);
            continue;
        }
        match term {
            AstMatcher::Has(body, stop) if search.is_none() => search = Some((body, stop)),
            _ => return Err(ShapeReason::ProducerShape.into()),
        }
    }
    let (body, stop) = search.ok_or(ShapeReason::MissingDescendant)?;
    let leaf_stop = match stop {
        StopBy::End => None,
        StopBy::Rule(r) if pure(&r) => Some(*r),
        _ => {
            return Err(ShapeReason::DescendantStop.into());
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
                    return Err(ShapeReason::WholeCapture.into());
                }
            },
            _ => {
                return Err(ShapeReason::BindingEffect.into());
            }
        }
    }
    let (capture, named) = capture.ok_or(ShapeReason::MissingCapture)?;
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
    ) -> Result<(), AdmissionError> {
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
                    return Err(ShapeReason::CaptureStop.into());
                }
                let (ancestor_filter, leaf_filter, leaf_stop, capture, named) = producer(body)?;
                if !bound.contains(&capture) {
                    return Err(ShapeReason::BoundFrontierInput.into());
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
            _ => return Err(ShapeReason::DiscoveryEffect.into()),
        }
        Ok(())
    }
}

pub(super) fn discover_typed(grammar: &GrammarMatcher) -> Result<Vec<Search>, AdmissionError> {
    if grammar.three_valued || grammar.transform.is_some() {
        return Err(ShapeReason::TruthContract.into());
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
        return Err(ShapeReason::NoFrontier.into());
    }
    Ok(discovery.searches)
}

pub(super) fn discover(grammar: &GrammarMatcher) -> Result<Vec<Search>, String> {
    discover_typed(grammar).map_err(|error| error.to_string())
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
