use super::super::matcher::{Collation, CompareOp, CompareText, SemanticText};
use super::super::value::structure::Equality;
use num_traits::FromPrimitive as _;
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum TerminalKind {
    Universe,
    Pattern,
    Kind,
    Regex,
    Text,
    Value,
    Navigation,
    Equality,
    Fact,
    Parse,
    CallSeed,
    Transform,
    File,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct TerminalSpec {
    kind: TerminalKind,
    semantic_key: String,
}

impl TerminalSpec {
    pub(super) fn new(kind: TerminalKind, semantic_key: impl Into<String>) -> Self {
        Self {
            kind,
            semantic_key: semantic_key.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum SqlOperation {
    PackedMaterialized,
    StatusJoin,
    ScanTerminal,
    ScanCapture,
    ScanNavigation,
    ScanFact,
    InnerJoin,
    LeftJoin,
    OrderedUnion,
    AntiJoin,
    Exists,
    RecursiveCte,
    GroupBy,
    Having,
    WindowRank,
    MinPosition,
    Distinct,
    CaseTruth,
    Bindings,
    MapDocument,
    Projection,
    OrderBy,
    WhereFilter,
    FileFilter,
    CrossFileJoin,
}

const ALL_TERMINAL_KINDS: [TerminalKind; 13] = [
    TerminalKind::Universe,
    TerminalKind::Pattern,
    TerminalKind::Kind,
    TerminalKind::Regex,
    TerminalKind::Text,
    TerminalKind::Value,
    TerminalKind::Navigation,
    TerminalKind::Equality,
    TerminalKind::Fact,
    TerminalKind::Parse,
    TerminalKind::CallSeed,
    TerminalKind::Transform,
    TerminalKind::File,
];

const ALL_SQL_OPERATIONS: [SqlOperation; 25] = [
    SqlOperation::PackedMaterialized,
    SqlOperation::StatusJoin,
    SqlOperation::ScanTerminal,
    SqlOperation::ScanCapture,
    SqlOperation::ScanNavigation,
    SqlOperation::ScanFact,
    SqlOperation::InnerJoin,
    SqlOperation::LeftJoin,
    SqlOperation::OrderedUnion,
    SqlOperation::AntiJoin,
    SqlOperation::Exists,
    SqlOperation::RecursiveCte,
    SqlOperation::GroupBy,
    SqlOperation::Having,
    SqlOperation::WindowRank,
    SqlOperation::MinPosition,
    SqlOperation::Distinct,
    SqlOperation::CaseTruth,
    SqlOperation::Bindings,
    SqlOperation::MapDocument,
    SqlOperation::Projection,
    SqlOperation::OrderBy,
    SqlOperation::WhereFilter,
    SqlOperation::FileFilter,
    SqlOperation::CrossFileJoin,
];

impl SqlOperation {
    pub(super) const fn fragment(self) -> &'static str {
        match self {
            Self::PackedMaterialized => "WITH packed AS MATERIALIZED",
            Self::StatusJoin => "JOIN producer_status",
            Self::ScanTerminal => "FROM terminal_match",
            Self::ScanCapture => "JOIN capture",
            Self::ScanNavigation => "JOIN navigation",
            Self::ScanFact => "JOIN fact",
            Self::InnerJoin => "INNER JOIN",
            Self::LeftJoin => "LEFT JOIN",
            Self::OrderedUnion => "UNION ALL /* branch_order */",
            Self::AntiJoin => "NOT EXISTS",
            Self::Exists => "EXISTS",
            Self::RecursiveCte => "WITH RECURSIVE",
            Self::GroupBy => "GROUP BY",
            Self::Having => "HAVING",
            Self::WindowRank => "row_number() OVER (ORDER BY",
            Self::MinPosition => "min(axis_order)",
            Self::Distinct => "DISTINCT",
            Self::CaseTruth => "CASE verdict",
            Self::Bindings => "JOIN environment_binding",
            Self::MapDocument => "JOIN document_map",
            Self::Projection => "SELECT /* violation projection */",
            Self::OrderBy => "ORDER BY",
            Self::WhereFilter => "WHERE",
            Self::FileFilter => "JOIN batch_file",
            Self::CrossFileJoin => "JOIN /* cross-file relation */",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NavigationAxis {
    Ancestor,
    Descendant,
    FollowingSibling,
    PrecedingSibling,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum NavigationStop {
    Neighbor,
    End,
    Rule,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NavigationSpec {
    pub(super) axis: NavigationAxis,
    pub(super) stop: NavigationStop,
    pub(super) field: Option<u16>,
    pub(super) field_stop_children_only: bool,
    pub(super) producer: usize,
    pub(super) enumerate: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct NthChildSpec {
    pub(super) step: i32,
    pub(super) offset: i32,
    pub(super) reverse: bool,
    pub(super) producer: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CountSpec {
    pub(super) navigation: NavigationSpec,
    pub(super) minimum: usize,
    pub(super) maximum: Option<usize>,
    pub(super) binding: Option<(String, i64)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ComparisonSpec {
    pub(super) left: String,
    pub(super) left_source: String,
    pub(super) left_transform: Option<usize>,
    pub(super) op: CompareOp,
    pub(super) right: String,
    pub(super) right_source: String,
    pub(super) right_transform: Option<usize>,
    pub(super) collation: Collation,
    pub(super) text: CompareText,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ValueLiteral {
    Null,
    Bool(bool),
    Integer(String),
    Float(String),
    String(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ValuePredicateSpec {
    pub(super) producer: usize,
    pub(super) language: super::super::language::GrammarDialect,
    pub(super) eq: Option<ValueLiteral>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SemanticRuleSpec {
    pub(super) slot: u16,
    pub(super) target_kind: Option<u16>,
    pub(super) relation: Option<Box<RelationalIr>>,
    pub(super) incomplete_position: Option<u32>,
    pub(super) deduplicate_source: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ParseFactSpec {
    pub(super) producer: usize,
    pub(super) host_producer: usize,
    pub(super) required_host_producer: Option<usize>,
    pub(super) target_kind: u16,
    pub(super) export: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SemanticFactSpec {
    pub(super) producer: usize,
    pub(super) argument_labels: Vec<String>,
    pub(super) rules: Vec<SemanticRuleSpec>,
    pub(super) require_all_candidates: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct UtilitySpec {
    pub(super) definition: super::plan::UtilityDefinitionId,
    pub(super) component: String,
    pub(super) universe_producer: usize,
    pub(super) depth: u32,
    pub(super) max_depth: u32,
    pub(super) max_work: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct UtilityFrameSpec {
    pub(super) utility: UtilitySpec,
    pub(super) arguments: Vec<(String, String)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RecursiveUtilityTransition {
    pub(super) caller: usize,
    pub(super) target: usize,
    pub(super) arguments: Vec<(String, String)>,
    pub(super) steps: Vec<NavigationSpec>,
    pub(super) stops: Vec<(usize, RelationalIr)>,
    pub(super) guards: Vec<(usize, RelationalIr)>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RecursiveUtilitySpec {
    pub(super) definition: super::plan::UtilityDefinitionId,
    pub(super) component: String,
    pub(super) root: usize,
    pub(super) complement: bool,
    pub(super) universe_producer: usize,
    pub(super) navigation_producer: usize,
    pub(super) max_depth: u32,
    pub(super) max_work: usize,
    pub(super) transitions: Vec<RecursiveUtilityTransition>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RelationNode {
    Seed,
    Empty,
    Producer(TerminalSpec),
    Apply {
        operation: SqlOperation,
        inputs: Vec<RelationNode>,
    },
    Navigate {
        spec: NavigationSpec,
        source: Box<RelationNode>,
        related: Box<RelationNode>,
        stop: Option<Box<RelationNode>>,
    },
    NthChild {
        spec: NthChildSpec,
        source: Box<RelationNode>,
        of_rule: Option<Box<RelationNode>>,
    },
    Constrain {
        source: Box<RelationNode>,
        constraints: Vec<(String, RelationNode)>,
    },
    Count {
        spec: CountSpec,
        source: Box<RelationNode>,
        related: Box<RelationNode>,
        stop: Option<Box<RelationNode>>,
    },
    Compare {
        spec: ComparisonSpec,
        source: Box<RelationNode>,
    },
    Value {
        spec: ValuePredicateSpec,
        source: Box<RelationNode>,
    },
    Semantic {
        spec: SemanticFactSpec,
    },
    Parsed {
        spec: ParseFactSpec,
    },
    SelectWhen {
        select: Box<RelationNode>,
        exists: Box<RelationNode>,
    },
    RequireEach {
        subject: Box<RelationNode>,
        evidence: Box<RelationNode>,
        keys: Vec<(String, String)>,
    },
    Budget {
        source: Box<RelationNode>,
        group_by: Vec<String>,
        distinct: Vec<String>,
        maximum: usize,
    },
    Utility {
        spec: UtilitySpec,
        source: Box<RelationNode>,
        body: Box<RelationNode>,
    },
    UtilityFrame {
        spec: UtilityFrameSpec,
        source: Box<RelationNode>,
        body: Box<RelationNode>,
    },
    RecursiveUtility {
        spec: RecursiveUtilitySpec,
        source: Box<RelationNode>,
        bases: Vec<(usize, RelationNode)>,
    },
    PrefixChoice {
        source: Box<RelationNode>,
        branch: usize,
    },
    MergeChoices(Vec<RelationNode>),
    Shared(Arc<RelationNode>),
    Rendered(String),
}

impl TerminalSpec {
    fn producer_id(&self) -> Result<usize, String> {
        self.semantic_key
            .strip_prefix("terminal:")
            .ok_or_else(|| "production terminal has no producer identity".to_owned())?
            .parse()
            .map_err(|_| "production terminal has an invalid producer identity".to_owned())
    }
}

impl RelationNode {
    fn utility_body_clone(&self) -> Self {
        match self {
            Self::Shared(body) => body.as_ref().clone(),
            body => body.clone(),
        }
    }

    fn independent_of_input(&self) -> bool {
        match self {
            Self::Producer(_) | Self::Parsed { .. } | Self::Semantic { .. } | Self::Empty => true,
            Self::Apply {
                operation:
                    SqlOperation::ScanTerminal
                    | SqlOperation::ScanCapture
                    | SqlOperation::CaseTruth
                    | SqlOperation::WindowRank
                    | SqlOperation::Projection
                    | SqlOperation::OrderBy,
                inputs,
            } => inputs.iter().all(Self::independent_of_input),
            Self::Shared(node) => node.independent_of_input(),
            _ => false,
        }
    }

    fn apply_after(self, input: Self) -> Self {
        let join = |left, right| Self::Apply {
            operation: SqlOperation::InnerJoin,
            inputs: vec![left, right],
        };
        match self {
            Self::Compare { spec, source } => Self::Compare {
                spec,
                source: Box::new(match *source {
                    Self::Seed => input,
                    nested => nested.apply_after(input),
                }),
            },
            Self::Value { spec, source } => Self::Value {
                spec,
                source: Box::new(match *source {
                    Self::Seed => input,
                    nested => nested.apply_after(input),
                }),
            },
            Self::Utility { spec, source, body } => Self::Utility {
                spec,
                source: Box::new(join(input, *source)),
                body,
            },
            Self::UtilityFrame { spec, source, body } => Self::UtilityFrame {
                spec,
                source: Box::new(join(input, *source)),
                body,
            },
            Self::RecursiveUtility {
                spec,
                source,
                bases,
            } => Self::RecursiveUtility {
                spec,
                source: Box::new(match *source {
                    Self::Seed => input,
                    nested => nested.apply_after(input),
                }),
                bases,
            },
            Self::Count {
                spec,
                source,
                related,
                stop,
            } => Self::Count {
                spec,
                source: Box::new(join(input, *source)),
                related,
                stop,
            },
            Self::Navigate {
                spec,
                source,
                related,
                stop,
            } => Self::Navigate {
                spec,
                source: Box::new(join(input, *source)),
                related,
                stop,
            },
            Self::NthChild {
                spec,
                source,
                of_rule,
            } => Self::NthChild {
                spec,
                source: Box::new(join(input, *source)),
                of_rule,
            },
            Self::Apply {
                operation: SqlOperation::InnerJoin,
                inputs,
            } => inputs.into_iter().fold(input, |current, nested| {
                if !nested.independent_of_input() {
                    return nested.apply_after(current);
                }
                match current {
                    Self::Apply {
                        operation: SqlOperation::InnerJoin,
                        mut inputs,
                    } => {
                        inputs.push(nested);
                        Self::Apply {
                            operation: SqlOperation::InnerJoin,
                            inputs,
                        }
                    }
                    current => join(current, nested),
                }
            }),
            Self::Apply {
                operation: SqlOperation::OrderedUnion,
                inputs,
            } => {
                let input = Arc::new(input);
                Self::MergeChoices(
                    inputs
                        .into_iter()
                        .enumerate()
                        .map(|(branch, nested)| {
                            nested.apply_after(Self::PrefixChoice {
                                source: Box::new(Self::Shared(Arc::clone(&input))),
                                branch,
                            })
                        })
                        .collect(),
                )
            }
            Self::Apply {
                operation: SqlOperation::AntiJoin,
                inputs,
            } => {
                let [universe, excluded] = <Vec<Self> as TryInto<[Self; 2]>>::try_into(inputs)
                    .expect("a verified negation has universe and excluded operands");
                let input = Arc::new(input);
                Self::Apply {
                    operation: SqlOperation::AntiJoin,
                    inputs: vec![
                        join(Self::Shared(Arc::clone(&input)), universe),
                        excluded.apply_after(Self::Shared(input)),
                    ],
                }
            }
            Self::Apply { operation, inputs }
                if matches!(
                    operation,
                    SqlOperation::CaseTruth
                        | SqlOperation::WindowRank
                        | SqlOperation::Projection
                        | SqlOperation::OrderBy
                ) =>
            {
                let input = Arc::new(input);
                Self::Apply {
                    operation,
                    inputs: inputs
                        .into_iter()
                        .map(|nested| nested.apply_after(Self::Shared(Arc::clone(&input))))
                        .collect(),
                }
            }
            Self::Rendered(_) => input,
            operation => join(input, operation),
        }
    }

    fn expanded_size_exceeds(&self, limit: usize) -> bool {
        fn visit(node: &RelationNode, remaining: &mut usize) -> bool {
            if *remaining == 0 {
                return true;
            }
            *remaining -= 1;
            match node {
                RelationNode::Apply { inputs, .. } | RelationNode::MergeChoices(inputs) => {
                    inputs.iter().any(|input| visit(input, remaining))
                }
                RelationNode::Navigate {
                    source,
                    related,
                    stop,
                    ..
                }
                | RelationNode::Count {
                    source,
                    related,
                    stop,
                    ..
                } => {
                    visit(source, remaining)
                        || visit(related, remaining)
                        || stop.as_deref().is_some_and(|stop| visit(stop, remaining))
                }
                RelationNode::NthChild {
                    source, of_rule, ..
                } => {
                    visit(source, remaining)
                        || of_rule
                            .as_deref()
                            .is_some_and(|of_rule| visit(of_rule, remaining))
                }
                RelationNode::Constrain {
                    source,
                    constraints,
                } => {
                    visit(source, remaining)
                        || constraints
                            .iter()
                            .any(|(_, constraint)| visit(constraint, remaining))
                }
                RelationNode::Compare { source, .. }
                | RelationNode::Value { source, .. }
                | RelationNode::Budget { source, .. }
                | RelationNode::PrefixChoice { source, .. } => visit(source, remaining),
                RelationNode::SelectWhen { select, exists } => {
                    visit(select, remaining) || visit(exists, remaining)
                }
                RelationNode::RequireEach {
                    subject, evidence, ..
                } => visit(subject, remaining) || visit(evidence, remaining),
                RelationNode::Utility { source, body, .. }
                | RelationNode::UtilityFrame { source, body, .. } => {
                    visit(source, remaining) || visit(body, remaining)
                }
                RelationNode::RecursiveUtility {
                    spec,
                    source,
                    bases,
                } => {
                    visit(source, remaining)
                        || bases.iter().any(|(_, base)| visit(base, remaining))
                        || spec.transitions.iter().any(|transition| {
                            transition.stops.iter().chain(&transition.guards).any(
                                |(_, relation)| {
                                    relation.roots.iter().any(|root| visit(root, remaining))
                                },
                            )
                        })
                }
                RelationNode::Shared(shared) => visit(shared, remaining),
                RelationNode::Seed
                | RelationNode::Empty
                | RelationNode::Producer(_)
                | RelationNode::Semantic { .. }
                | RelationNode::Parsed { .. }
                | RelationNode::Rendered(_) => false,
            }
        }
        visit(self, &mut limit.saturating_add(1))
    }

    fn can_bind(&self) -> bool {
        match self {
            Self::Producer(spec) => spec.kind == TerminalKind::Pattern,
            Self::Apply { inputs, .. } => inputs.iter().any(Self::can_bind),
            Self::Navigate {
                source, related, ..
            } => source.can_bind() || related.can_bind(),
            Self::NthChild {
                source, of_rule, ..
            } => source.can_bind() || of_rule.as_deref().is_some_and(Self::can_bind),
            Self::Constrain { .. } | Self::Count { .. } => true,
            Self::Parsed { spec } => spec.export.is_some(),
            Self::Semantic { spec } => {
                !spec.argument_labels.is_empty()
                    || spec.rules.iter().any(|rule| {
                        rule.relation
                            .as_deref()
                            .is_some_and(|relation| relation.roots.iter().any(Self::can_bind))
                    })
            }
            Self::Compare { source, .. }
            | Self::Value { source, .. }
            | Self::Budget { source, .. } => source.can_bind(),
            Self::SelectWhen { select, exists } => select.can_bind() || exists.can_bind(),
            Self::RequireEach {
                subject, evidence, ..
            } => subject.can_bind() || evidence.can_bind(),
            Self::Utility { source, body, .. } => source.can_bind() || body.can_bind(),
            Self::UtilityFrame { source, body, .. } => source.can_bind() || body.can_bind(),
            Self::RecursiveUtility {
                spec,
                source,
                bases,
            } => {
                source.can_bind()
                    || bases.iter().any(|(_, base)| base.can_bind())
                    || spec.transitions.iter().any(|transition| {
                        transition
                            .stops
                            .iter()
                            .chain(&transition.guards)
                            .any(|(_, relation)| relation.roots.iter().any(Self::can_bind))
                    })
            }
            Self::PrefixChoice { source, .. } => source.can_bind(),
            Self::MergeChoices(inputs) => inputs.iter().any(Self::can_bind),
            Self::Shared(node) => node.can_bind(),
            Self::Seed | Self::Empty | Self::Rendered(_) => false,
        }
    }

    fn collect_utility_definitions(
        &self,
        definitions: &mut BTreeSet<super::plan::UtilityDefinitionId>,
    ) {
        match self {
            Self::Utility { spec, source, body } => {
                definitions.insert(spec.definition);
                source.collect_utility_definitions(definitions);
                body.collect_utility_definitions(definitions);
            }
            Self::UtilityFrame { spec, source, body } => {
                definitions.insert(spec.utility.definition);
                source.collect_utility_definitions(definitions);
                body.collect_utility_definitions(definitions);
            }
            Self::RecursiveUtility {
                spec,
                source,
                bases,
            } => {
                definitions.insert(spec.definition);
                source.collect_utility_definitions(definitions);
                for (_, base) in bases {
                    base.collect_utility_definitions(definitions);
                }
                for transition in &spec.transitions {
                    for (_, relation) in transition.stops.iter().chain(&transition.guards) {
                        for root in &relation.roots {
                            root.collect_utility_definitions(definitions);
                        }
                    }
                }
            }
            Self::Apply { inputs, .. } | Self::MergeChoices(inputs) => {
                for input in inputs {
                    input.collect_utility_definitions(definitions);
                }
            }
            Self::Navigate {
                source,
                related,
                stop,
                ..
            }
            | Self::Count {
                source,
                related,
                stop,
                ..
            } => {
                source.collect_utility_definitions(definitions);
                related.collect_utility_definitions(definitions);
                if let Some(stop) = stop {
                    stop.collect_utility_definitions(definitions);
                }
            }
            Self::NthChild {
                source, of_rule, ..
            } => {
                source.collect_utility_definitions(definitions);
                if let Some(of_rule) = of_rule {
                    of_rule.collect_utility_definitions(definitions);
                }
            }
            Self::Constrain {
                source,
                constraints,
            } => {
                source.collect_utility_definitions(definitions);
                for (_, constraint) in constraints {
                    constraint.collect_utility_definitions(definitions);
                }
            }
            Self::SelectWhen { select, exists } => {
                select.collect_utility_definitions(definitions);
                exists.collect_utility_definitions(definitions);
            }
            Self::RequireEach {
                subject, evidence, ..
            } => {
                subject.collect_utility_definitions(definitions);
                evidence.collect_utility_definitions(definitions);
            }
            Self::Compare { source, .. }
            | Self::Value { source, .. }
            | Self::Budget { source, .. }
            | Self::PrefixChoice { source, .. } => {
                source.collect_utility_definitions(definitions);
            }
            Self::Semantic { spec } => {
                for rule in &spec.rules {
                    if let Some(relation) = &rule.relation {
                        for root in &relation.roots {
                            root.collect_utility_definitions(definitions);
                        }
                    }
                }
            }
            Self::Shared(node) => node.collect_utility_definitions(definitions),
            Self::Seed
            | Self::Empty
            | Self::Producer(_)
            | Self::Parsed { .. }
            | Self::Rendered(_) => {}
        }
    }

    fn collect_terminals(&self, terminals: &mut Vec<TerminalKind>) {
        self.collect_terminals_inner(terminals, &mut Vec::new());
    }

    fn collect_terminals_inner(
        &self,
        terminals: &mut Vec<TerminalKind>,
        shared: &mut Vec<Arc<RelationNode>>,
    ) {
        match self {
            Self::Seed | Self::Empty | Self::Rendered(_) => {}
            Self::Shared(node) => {
                if !shared.iter().any(|known| known.as_ref() == node.as_ref()) {
                    shared.push(Arc::clone(node));
                    node.collect_terminals_inner(terminals, shared);
                }
            }
            Self::Producer(spec) => {
                if !terminals.contains(&spec.kind) {
                    terminals.push(spec.kind);
                }
            }
            Self::Apply { inputs, .. } | Self::MergeChoices(inputs) => {
                for input in inputs {
                    input.collect_terminals_inner(terminals, shared);
                }
            }
            Self::PrefixChoice { source, .. } => source.collect_terminals_inner(terminals, shared),
            Self::Utility { source, body, .. } => {
                source.collect_terminals_inner(terminals, shared);
                body.collect_terminals_inner(terminals, shared);
            }
            Self::UtilityFrame { source, body, .. } => {
                source.collect_terminals_inner(terminals, shared);
                body.collect_terminals_inner(terminals, shared);
            }
            Self::RecursiveUtility {
                spec,
                source,
                bases,
            } => {
                source.collect_terminals_inner(terminals, shared);
                for (_, base) in bases {
                    base.collect_terminals_inner(terminals, shared);
                }
                for transition in &spec.transitions {
                    for (_, relation) in transition.stops.iter().chain(&transition.guards) {
                        for root in &relation.roots {
                            root.collect_terminals_inner(terminals, shared);
                        }
                    }
                }
                for kind in [TerminalKind::Universe, TerminalKind::Navigation] {
                    if !terminals.contains(&kind) {
                        terminals.push(kind);
                    }
                }
            }
            Self::Navigate {
                spec,
                source,
                related,
                stop,
            } => {
                if !terminals.contains(&TerminalKind::Navigation) {
                    terminals.push(TerminalKind::Navigation);
                }
                let _ = spec;
                source.collect_terminals_inner(terminals, shared);
                related.collect_terminals_inner(terminals, shared);
                if let Some(stop) = stop {
                    stop.collect_terminals_inner(terminals, shared);
                }
            }
            Self::NthChild {
                source, of_rule, ..
            } => {
                if !terminals.contains(&TerminalKind::Navigation) {
                    terminals.push(TerminalKind::Navigation);
                }
                source.collect_terminals_inner(terminals, shared);
                if let Some(of_rule) = of_rule {
                    of_rule.collect_terminals_inner(terminals, shared);
                }
            }
            Self::Constrain {
                source,
                constraints,
            } => {
                source.collect_terminals_inner(terminals, shared);
                for (_, constraint) in constraints {
                    constraint.collect_terminals_inner(terminals, shared);
                }
            }
            Self::Compare { spec, source } => {
                source.collect_terminals_inner(terminals, shared);
                if (spec.left_transform.is_some() || spec.right_transform.is_some())
                    && !terminals.contains(&TerminalKind::Transform)
                {
                    terminals.push(TerminalKind::Transform);
                }
            }
            Self::Budget { source, .. } => source.collect_terminals_inner(terminals, shared),
            Self::SelectWhen { select, exists } => {
                select.collect_terminals_inner(terminals, shared);
                exists.collect_terminals_inner(terminals, shared);
            }
            Self::RequireEach {
                subject, evidence, ..
            } => {
                subject.collect_terminals_inner(terminals, shared);
                evidence.collect_terminals_inner(terminals, shared);
            }
            Self::Parsed { .. } => {
                if !terminals.contains(&TerminalKind::Parse) {
                    terminals.push(TerminalKind::Parse);
                }
            }
            Self::Semantic { spec } => {
                if !terminals.contains(&TerminalKind::Fact) {
                    terminals.push(TerminalKind::Fact);
                }
                for rule in &spec.rules {
                    if let Some(relation) = &rule.relation {
                        for root in &relation.roots {
                            root.collect_terminals_inner(terminals, shared);
                        }
                    }
                }
            }
            Self::Value { source, .. } => {
                source.collect_terminals_inner(terminals, shared);
                if !terminals.contains(&TerminalKind::Value) {
                    terminals.push(TerminalKind::Value);
                }
            }
            Self::Count {
                source,
                related,
                stop,
                ..
            } => {
                if !terminals.contains(&TerminalKind::Navigation) {
                    terminals.push(TerminalKind::Navigation);
                }
                source.collect_terminals_inner(terminals, shared);
                related.collect_terminals_inner(terminals, shared);
                if let Some(stop) = stop {
                    stop.collect_terminals_inner(terminals, shared);
                }
            }
        }
    }

    fn collect_operations(&self, operations: &mut Vec<SqlOperation>) {
        self.collect_operations_inner(operations, &mut Vec::new());
    }

    fn collect_operations_inner(
        &self,
        operations: &mut Vec<SqlOperation>,
        shared: &mut Vec<Arc<RelationNode>>,
    ) {
        match self {
            Self::Apply { operation, inputs } => {
                for input in inputs {
                    input.collect_operations_inner(operations, shared);
                }
                if !operations.contains(operation) {
                    operations.push(*operation);
                }
            }
            Self::Navigate {
                source,
                related,
                stop,
                ..
            } => {
                source.collect_operations_inner(operations, shared);
                related.collect_operations_inner(operations, shared);
                if let Some(stop) = stop {
                    stop.collect_operations_inner(operations, shared);
                }
                for operation in [SqlOperation::ScanNavigation, SqlOperation::InnerJoin] {
                    if !operations.contains(&operation) {
                        operations.push(operation);
                    }
                }
            }
            Self::NthChild {
                source, of_rule, ..
            } => {
                source.collect_operations_inner(operations, shared);
                if let Some(of_rule) = of_rule {
                    of_rule.collect_operations_inner(operations, shared);
                }
                for operation in [SqlOperation::ScanNavigation, SqlOperation::WindowRank] {
                    if !operations.contains(&operation) {
                        operations.push(operation);
                    }
                }
            }
            Self::Constrain {
                source,
                constraints,
            } => {
                source.collect_operations_inner(operations, shared);
                for (_, constraint) in constraints {
                    constraint.collect_operations_inner(operations, shared);
                }
                for operation in [SqlOperation::Bindings, SqlOperation::InnerJoin] {
                    if !operations.contains(&operation) {
                        operations.push(operation);
                    }
                }
            }
            Self::PrefixChoice { source, .. } => {
                source.collect_operations_inner(operations, shared)
            }
            Self::MergeChoices(inputs) => {
                for input in inputs {
                    input.collect_operations_inner(operations, shared);
                }
                if !operations.contains(&SqlOperation::OrderedUnion) {
                    operations.push(SqlOperation::OrderedUnion);
                }
            }
            Self::Compare { source, .. } => {
                source.collect_operations_inner(operations, shared);
                for operation in [SqlOperation::Bindings, SqlOperation::WhereFilter] {
                    if !operations.contains(&operation) {
                        operations.push(operation);
                    }
                }
            }
            Self::Parsed { .. } => {
                for operation in [
                    SqlOperation::ScanTerminal,
                    SqlOperation::ScanFact,
                    SqlOperation::InnerJoin,
                    SqlOperation::Bindings,
                ] {
                    if !operations.contains(&operation) {
                        operations.push(operation);
                    }
                }
            }
            Self::Semantic { spec } => {
                for rule in &spec.rules {
                    if let Some(relation) = &rule.relation {
                        for root in &relation.roots {
                            root.collect_operations_inner(operations, shared);
                        }
                    }
                }
                for operation in [
                    SqlOperation::ScanFact,
                    SqlOperation::Bindings,
                    SqlOperation::InnerJoin,
                    SqlOperation::CaseTruth,
                ] {
                    if !operations.contains(&operation) {
                        operations.push(operation);
                    }
                }
            }
            Self::Value { source, .. } => {
                source.collect_operations_inner(operations, shared);
                for operation in [SqlOperation::ScanFact, SqlOperation::CaseTruth] {
                    if !operations.contains(&operation) {
                        operations.push(operation);
                    }
                }
            }
            Self::Utility { source, body, .. } => {
                source.collect_operations_inner(operations, shared);
                body.collect_operations_inner(operations, shared);
                if !operations.contains(&SqlOperation::Bindings) {
                    operations.push(SqlOperation::Bindings);
                }
            }
            Self::UtilityFrame { source, body, .. } => {
                source.collect_operations_inner(operations, shared);
                body.collect_operations_inner(operations, shared);
                if !operations.contains(&SqlOperation::Bindings) {
                    operations.push(SqlOperation::Bindings);
                }
            }
            Self::RecursiveUtility {
                spec,
                source,
                bases,
            } => {
                source.collect_operations_inner(operations, shared);
                for (_, base) in bases {
                    base.collect_operations_inner(operations, shared);
                }
                for transition in &spec.transitions {
                    for (_, relation) in transition.stops.iter().chain(&transition.guards) {
                        for root in &relation.roots {
                            root.collect_operations_inner(operations, shared);
                        }
                    }
                }
                for operation in [
                    SqlOperation::ScanTerminal,
                    SqlOperation::ScanNavigation,
                    SqlOperation::RecursiveCte,
                    SqlOperation::Bindings,
                ] {
                    if !operations.contains(&operation) {
                        operations.push(operation);
                    }
                }
            }
            Self::Count {
                source,
                related,
                stop,
                ..
            } => {
                source.collect_operations_inner(operations, shared);
                related.collect_operations_inner(operations, shared);
                if let Some(stop) = stop {
                    stop.collect_operations_inner(operations, shared);
                }
                for operation in [
                    SqlOperation::ScanNavigation,
                    SqlOperation::GroupBy,
                    SqlOperation::Having,
                ] {
                    if !operations.contains(&operation) {
                        operations.push(operation);
                    }
                }
            }
            Self::SelectWhen { select, exists } => {
                select.collect_operations_inner(operations, shared);
                exists.collect_operations_inner(operations, shared);
                operations.push(SqlOperation::Exists);
            }
            Self::RequireEach {
                subject, evidence, ..
            } => {
                subject.collect_operations_inner(operations, shared);
                evidence.collect_operations_inner(operations, shared);
                operations.push(SqlOperation::AntiJoin);
                operations.push(SqlOperation::CrossFileJoin);
            }
            Self::Budget { source, .. } => {
                source.collect_operations_inner(operations, shared);
                operations.push(SqlOperation::GroupBy);
                operations.push(SqlOperation::Having);
            }
            Self::Shared(node) => {
                if !shared.iter().any(|known| known.as_ref() == node.as_ref()) {
                    shared.push(Arc::clone(node));
                    node.collect_operations_inner(operations, shared);
                }
            }
            Self::Seed | Self::Empty | Self::Producer(_) | Self::Rendered(_) => {}
        }
    }
}

const UTILITY_ROW_ALIASES: [&str; 31] = [
    "m",
    "s",
    "li",
    "ri",
    "g",
    "br",
    "sl",
    "uv",
    "h",
    "ar",
    "t",
    "mt",
    "p",
    "rt",
    "rs",
    "fr",
    "c",
    "match",
    "source",
    "left_input",
    "right_input",
    "generated",
    "branch",
    "selected",
    "universe",
    "host",
    "argument",
    "target",
    "matched",
    "parent",
    "result",
];

const UTILITY_DEAD_COLUMNS: [&str; 25] = [
    "batch",
    "row_kind",
    "task_index",
    "capture_index",
    "capture_occurrence",
    "capture_sequence_start",
    "capture_sequence_count",
    "axis_kind",
    "axis_order",
    "fact_kind",
    "start_ordinal",
    "output_label",
    "capture_label",
    "text_value",
    "complete",
    "work_used",
    "error_code",
    "field_id",
    "related_named",
    "source_text",
    "structural_key",
    "named_leaf",
    "capture_kind",
    "capture_named",
    "equality_occurrence",
];

fn compact_utility_binding_predicates(sql: &str) -> String {
    const PREFIX: &str = "NOT EXISTS (SELECT 1 FROM json_each(";
    const RIGHT: &str = " JOIN json_each(";
    let mut output = String::with_capacity(sql.len());
    let mut remaining = sql;
    while let Some(start) = remaining.find(PREFIX) {
        output.push_str(&remaining[..start]);
        let predicate = &remaining[start..];
        let Some(left_end) = predicate[PREFIX.len()..].find(".bindings)") else {
            output.push_str(predicate);
            return output;
        };
        let left = &predicate[PREFIX.len()..PREFIX.len() + left_end];
        let right_start = PREFIX.len() + left_end + ".bindings)".len();
        let Some(right_start) = predicate[right_start..]
            .find(RIGHT)
            .map(|offset| right_start + offset + RIGHT.len())
        else {
            output.push_str(&predicate[..1]);
            remaining = &predicate[1..];
            continue;
        };
        let Some(right_end) = predicate[right_start..].find(".bindings)") else {
            output.push_str(predicate);
            return output;
        };
        let right = &predicate[right_start..right_start + right_end];
        let authored = SqlRenderer::binding_compatibility(left, right);
        if predicate.starts_with(&authored) {
            output.push_str(&SqlRenderer::compact_binding_compatibility(left, right));
            remaining = &predicate[authored.len()..];
        } else {
            output.push_str(&predicate[..1]);
            remaining = &predicate[1..];
        }
    }
    output.push_str(remaining);
    output
}

fn compact_large_utility_base_names(sql: &str) -> String {
    let mut output = String::with_capacity(sql.len());
    let mut remaining = sql;
    let mut quoted = false;
    while !remaining.is_empty() {
        let character = remaining
            .chars()
            .next()
            .expect("a nonempty SQL suffix has one character");
        if character == '\'' {
            output.push(character);
            remaining = &remaining[character.len_utf8()..];
            if quoted && remaining.starts_with('\'') {
                output.push('\'');
                remaining = &remaining[1..];
            } else {
                quoted = !quoted;
            }
            continue;
        }
        if !quoted && (character.is_ascii_alphabetic() || character == '_') {
            let length = remaining
                .chars()
                .take_while(|next| next.is_ascii_alphanumeric() || *next == '_')
                .map(char::len_utf8)
                .sum::<usize>();
            let identifier = &remaining[..length];
            output.push_str(match identifier {
                "producer_status" => "ps",
                "terminal_match" => "tm",
                "navigation" => "nv",
                "capture" => "cp",
                "fact" => "ft",
                _ => identifier,
            });
            remaining = &remaining[length..];
            continue;
        }
        output.push(character);
        remaining = &remaining[character.len_utf8()..];
    }
    output
}

fn cte_declaration(sql: &str) -> Option<(usize, &str)> {
    sql.match_indices("AS").find_map(|(index, keyword)| {
        let before = sql[..index].chars().next_back();
        let after = sql[index + keyword.len()..].trim_start();
        let boundary =
            before.is_none_or(|character| !character.is_ascii_alphanumeric() && character != '_');
        let declaration = after.starts_with('(')
            || after
                .strip_prefix("MATERIALIZED")
                .is_some_and(|body| body.trim_start().starts_with('('));
        (boundary && declaration).then_some((index + keyword.len(), &sql[..index]))
    })
}

fn recursive_materialization_fence(sql: String) -> String {
    let Some((body_start, declaration)) = cte_declaration(&sql) else {
        return sql;
    };
    let relation = declaration.split('(').next().unwrap_or(declaration).trim();
    let body = &sql[body_start..];
    if body
        .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .any(|identifier| identifier == relation)
    {
        return sql;
    }
    let Some(close) = sql.rfind(')') else {
        return sql;
    };
    format!(
        "{} UNION ALL SELECT*FROM {relation} WHERE 0{}",
        &sql[..close],
        &sql[close..]
    )
}

fn compact_utility_cte_for_catalog(sql: &str, compact_base_relations: bool) -> String {
    let compact = compact_utility_cte(sql);
    if !compact_base_relations {
        return compact;
    }
    let compact = compact_large_utility_base_names(&compact);
    let materialized = match cte_declaration(&compact) {
        Some((body_start, _))
            if !compact[body_start..]
                .trim_start()
                .starts_with("MATERIALIZED") =>
        {
            format!(
                "{} MATERIALIZED{}",
                &compact[..body_start],
                &compact[body_start..]
            )
        }
        _ => compact,
    };
    recursive_materialization_fence(materialized)
}

fn compact_utility_cte(sql: &str) -> String {
    compact_utility_row_layout(&compact_sql_layout(&compact_utility_binding_predicates(
        sql,
    )))
    .replace(
        "CASE WHEN li.verdict=0 OR ri.verdict=0 THEN 0 WHEN li.verdict IS NULL OR ri.verdict IS NULL THEN NULL ELSE 1 END",
        "CASE WHEN li.verdict=0 OR ri.verdict=0 THEN 0 ELSE li.verdict*ri.verdict END",
    )
}

fn compact_utility_row_layout(sql: &str) -> String {
    let name = sql.split_once(" AS").map_or("", |(name, _)| name);
    if name.contains("_errors_")
        || name.ends_with("_errors")
        || sql.contains("/* utility-interface */")
        || sql.contains("/*utility-interface*/")
    {
        return sql.to_owned();
    }

    let bytes = sql.as_bytes();
    let mut output = String::with_capacity(sql.len());
    let mut copied = 0;
    let mut index = 0;
    let mut quoted = false;
    while index < bytes.len() {
        if bytes[index] == b'\'' {
            if quoted && bytes.get(index + 1) == Some(&b'\'') {
                index += 2;
                continue;
            }
            quoted = !quoted;
            index += 1;
            continue;
        }
        if quoted || !(bytes[index].is_ascii_alphabetic() || bytes[index] == b'_') {
            index += 1;
            continue;
        }
        let token_start = index;
        while bytes
            .get(index)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            index += 1;
        }
        if bytes.get(index) != Some(&b'.')
            || !UTILITY_ROW_ALIASES.contains(&&sql[token_start..index])
        {
            continue;
        }
        index += 1;
        let column_start = index;
        while bytes
            .get(index)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        {
            index += 1;
        }
        let column = &sql[column_start..index];
        if !UTILITY_DEAD_COLUMNS.contains(&column) {
            continue;
        }
        let mut end = index;
        while bytes.get(end).is_some_and(u8::is_ascii_whitespace) {
            end += 1;
        }
        if bytes.get(end) != Some(&b',') {
            continue;
        }
        end += 1;
        while bytes.get(end).is_some_and(u8::is_ascii_whitespace) {
            end += 1;
        }
        output.push_str(&sql[copied..token_start]);
        copied = end;
        index = end;
    }
    output.push_str(&sql[copied..]);
    output
}

fn compact_sql_layout(sql: &str) -> String {
    fn compact_empty_json(sql: &str) -> String {
        let mut output = String::with_capacity(sql.len());
        let mut remaining = sql;
        let mut quoted = false;
        while !remaining.is_empty() {
            if !quoted && remaining.starts_with("json('[]')") {
                output.push_str("'[]'");
                remaining = &remaining["json('[]')".len()..];
                continue;
            }
            let character = remaining
                .chars()
                .next()
                .expect("a nonempty SQL suffix has one character");
            output.push(character);
            remaining = &remaining[character.len_utf8()..];
            if character == '\'' {
                if quoted && remaining.starts_with('\'') {
                    output.push('\'');
                    remaining = &remaining[1..];
                } else {
                    quoted = !quoted;
                }
            }
        }
        output
    }

    fn compact_inner_joins(sql: &str) -> String {
        let mut output = String::with_capacity(sql.len());
        let mut remaining = sql;
        let mut quoted = false;
        while !remaining.is_empty() {
            if !quoted && remaining.starts_with("INNER JOIN ") {
                output.push_str("JOIN ");
                remaining = &remaining["INNER JOIN ".len()..];
                continue;
            }
            if !quoted && remaining.starts_with("SELECT 1 FROM ") {
                output.push_str("SELECT*FROM ");
                remaining = &remaining["SELECT 1 FROM ".len()..];
                continue;
            }
            let character = remaining
                .chars()
                .next()
                .expect("a nonempty SQL suffix has one character");
            output.push(character);
            remaining = &remaining[character.len_utf8()..];
            if character == '\'' {
                if quoted && remaining.starts_with('\'') {
                    output.push('\'');
                    remaining = &remaining[1..];
                } else {
                    quoted = !quoted;
                }
            }
        }
        output
    }

    fn compact_punctuation(character: char) -> bool {
        matches!(
            character,
            ',' | '(' | ')' | ';' | '=' | '<' | '>' | '+' | '-' | '*' | '/' | '|'
        )
    }

    fn compact_column_aliases(sql: &str) -> String {
        const ALIASES: [&str; 15] = [
            "accepted_order",
            "axis_order",
            "bindings",
            "branch_order",
            "choice_order",
            "depth",
            "error_code",
            "file_index",
            "node",
            "position",
            "related_node",
            "truth_order",
            "unknown_blocks",
            "verdict",
            "work_used",
        ];
        let bytes = sql.as_bytes();
        let mut output = String::with_capacity(sql.len());
        let mut quoted = false;
        let mut copied = 0;
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'\'' {
                if quoted && bytes.get(index + 1) == Some(&b'\'') {
                    index += 2;
                    continue;
                }
                quoted = !quoted;
                index += 1;
                continue;
            }
            if !quoted && bytes[index..].starts_with(b" AS ") {
                let alias_start = index + 4;
                let mut alias_end = alias_start;
                while bytes
                    .get(alias_end)
                    .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                {
                    alias_end += 1;
                }
                let alias = &sql[alias_start..alias_end];
                if ALIASES.contains(&alias) {
                    output.push_str(&sql[copied..=index]);
                    output.push_str(alias);
                    copied = alias_end;
                    index = alias_end;
                    continue;
                }
            }
            index += 1;
        }
        output.push_str(&sql[copied..]);
        output
    }

    fn short_identifier(identifier: &str) -> Cow<'_, str> {
        fn base36(mut value: usize) -> String {
            const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
            let mut reversed = Vec::new();
            loop {
                reversed.push(DIGITS[value % 36] as char);
                value /= 36;
                if value == 0 {
                    break;
                }
            }
            reversed.into_iter().rev().collect()
        }

        if let Some(ordinal) = identifier.strip_prefix('u')
            && !ordinal.is_empty()
            && ordinal.bytes().all(|byte| byte.is_ascii_digit())
            && let Ok(ordinal) = ordinal.parse::<usize>()
        {
            return Cow::Owned(format!("u{}", base36(ordinal)));
        }
        let short = match identifier {
            "source" => "s",
            "candidate" => "c",
            "match" => "m",
            "captured" => "cp",
            "first" => "f",
            "repeated" => "r",
            "generated" => "g",
            "mapping" => "mp",
            "narrower" => "nw",
            "related" => "rl",
            "stopped" => "st",
            "earlier" => "e",
            "child" => "ch",
            "parent" => "p",
            "root" => "rt",
            "status" => "ps",
            "target" => "t",
            "gate" => "gt",
            "result" => "rs",
            "usage" => "us",
            "limited" => "lm",
            "caller" => "cr",
            "walk" => "w",
            "left_binding" => "lb",
            "right_binding" => "rb",
            "item" => "i",
            "same" => "sm",
            "failed" => "fd",
            "failed_match" => "fm",
            "undecided" => "ud",
            "origin" => "o",
            "required" => "rq",
            "checked" => "ck",
            "input_order" => "io",
            "capture_order" => "co",
            "base" => "b",
            "path" => "pa",
            "branch" => "br",
            "counted" => "ct",
            "counted_rule" => "ctr",
            "source_rule" => "sr",
            "evidence" => "ev",
            "excluded" => "ex",
            "argument" => "ar",
            "frame" => "fr",
            "matched" => "mt",
            "selected" => "sl",
            "host" => "h",
            "subject" => "sj",
            "universe" => "uv",
            "accepted" => "ac",
            "accepted_order" => "ao",
            "branch_order" => "bo",
            "unknown_blocks" => "ub",
            "truth_order" => "tr",
            "actual" => "a",
            "source_node" => "sn",
            "source_bindings" => "sb",
            "semantic" => "se",
            "nested" => "nt",
            "bound" | "binding" => "bd",
            "left_value" => "lv",
            "right_value" => "rv",
            "left_input" => "li",
            "right_input" => "ri",
            _ => {
                for (prefix, compact) in [
                    ("input_", "i"),
                    ("guard", "g"),
                    ("edge", "e"),
                    ("relation_", "r"),
                    ("compare_", "q"),
                    ("term_", "t"),
                ] {
                    let Some(suffix) = identifier.strip_prefix(prefix) else {
                        continue;
                    };
                    let mut parts = suffix.split('_');
                    let Some(ordinal) = parts.next() else {
                        continue;
                    };
                    if !ordinal.bytes().all(|byte| byte.is_ascii_digit()) {
                        continue;
                    }
                    let mut shortened = format!("{compact}{ordinal}");
                    for role in parts {
                        let role = match role {
                            "accepted" => "a",
                            "candidates" | "context" => "c",
                            "walk" => "w",
                            _ if role.bytes().all(|byte| byte.is_ascii_digit()) => role,
                            _ => {
                                shortened.clear();
                                break;
                            }
                        };
                        shortened.push('o');
                        shortened.push_str(role);
                    }
                    if !shortened.is_empty() {
                        return Cow::Owned(shortened);
                    }
                }
                identifier
            }
        };
        Cow::Borrowed(short)
    }

    let mut layout = String::with_capacity(sql.len());
    let mut characters = sql.chars().peekable();
    let mut quoted = false;
    let mut pending_space = false;
    while let Some(character) = characters.next() {
        if quoted {
            layout.push(character);
            if character == '\'' {
                if characters.peek() == Some(&'\'') {
                    layout.push(characters.next().expect("escaped SQL quote"));
                } else {
                    quoted = false;
                }
            }
        } else if character == '\'' {
            if pending_space
                && !layout.is_empty()
                && !layout.ends_with('(')
                && !layout.ends_with('.')
                && !layout.chars().last().is_some_and(compact_punctuation)
            {
                layout.push(' ');
            }
            pending_space = false;
            quoted = true;
            layout.push(character);
        } else if character.is_whitespace() {
            pending_space = true;
        } else {
            if pending_space
                && !layout.is_empty()
                && (!compact_punctuation(character) || (character == '(' && layout.ends_with("AS")))
                && !layout.ends_with('(')
                && !layout.ends_with('.')
                && !layout.chars().last().is_some_and(compact_punctuation)
            {
                layout.push(' ');
            }
            pending_space = false;
            layout.push(character);
        }
    }

    let mut output = String::with_capacity(layout.len());
    let mut characters = layout.chars().peekable();
    let mut quoted = false;
    while let Some(character) = characters.next() {
        if character == '\'' {
            output.push(character);
            if quoted && characters.peek() == Some(&'\'') {
                output.push(characters.next().expect("escaped SQL quote"));
            } else {
                quoted = !quoted;
            }
        } else if !quoted && (character.is_ascii_alphabetic() || character == '_') {
            let mut identifier = String::from(character);
            while characters
                .peek()
                .is_some_and(|next| next.is_ascii_alphanumeric() || *next == '_')
            {
                identifier.push(characters.next().expect("identifier character"));
            }
            output.push_str(short_identifier(&identifier).as_ref());
        } else {
            output.push(character);
        }
    }
    const JSON_ALIASES: [&str; 18] = [
        "lb", "rb", "i", "b", "f", "c", "rq", "e", "uv", "ex", "bd", "lv", "rv", "cp", "r", "fr",
        "ar", "s",
    ];
    const JSON_PATHS: [&str; 10] = [
        "label",
        "kind",
        "named",
        "equality_occurrence",
        "node",
        "text",
        "leaf",
        "structure",
        "semantic_structure",
        "occurrence",
    ];
    let mut compact_json = String::with_capacity(output.len());
    let mut remaining = output.as_str();
    while let Some(start) = remaining.find("json_extract(") {
        compact_json.push_str(&remaining[..start]);
        let call = &remaining[start..];
        let Some(close) = call.find(')') else {
            compact_json.push_str(call);
            remaining = "";
            break;
        };
        let expression = &call[13..close];
        let replacement = (|| {
            let (value, path) = expression.split_once(",'$.")?;
            let path = path.strip_suffix('\'')?;
            let alias = value.strip_suffix(".value");
            if !JSON_PATHS.contains(&path)
                || alias.is_some_and(|alias| !JSON_ALIASES.contains(&alias))
                || (alias.is_none() && value != "value")
            {
                return None;
            }
            Some(format!("{value}->>'{path}'"))
        })();
        if let Some(replacement) = replacement {
            compact_json.push_str(&replacement);
        } else {
            compact_json.push_str(&call[..=close]);
        }
        remaining = &call[close + 1..];
    }
    compact_json.push_str(remaining);
    compact_column_aliases(&compact_empty_json(&compact_inner_joins(&compact_json)))
}

const ROW_COLUMNS: &str = "batch, row_kind, producer_id, file_index, task_index, match_index, capture_index, capture_occurrence, capture_sequence_start, capture_sequence_count, node, related_node, axis_kind, axis_order, fact_kind, document_id, mapped_document_id, start_ordinal, output_label, capture_label, text_value, start_byte, end_byte, verdict, complete, work_used, error_code, field_id, related_named, source_text, structural_key, named_leaf, capture_kind, capture_named, equality_occurrence";

struct UtilityAttempt {
    source: String,
    depth: u32,
    inline: bool,
    max_depth: u32,
    max_work: usize,
    universe_producer: usize,
}

struct SharedUtility {
    component: String,
    body: RelationNode,
    universe_producer: usize,
    output: String,
    published_output: Option<String>,
    demand: Option<String>,
    inputs: Vec<String>,
    dependencies: BTreeSet<super::plan::UtilityDefinitionId>,
    rendered: bool,
}

#[derive(Default)]
pub(super) struct UtilityCatalog {
    utilities: BTreeMap<super::plan::UtilityDefinitionId, SharedUtility>,
    recursive: HashMap<String, (String, String)>,
    call_counts: BTreeMap<super::plan::UtilityDefinitionId, usize>,
}

impl UtilityCatalog {
    pub(super) fn new(call_counts: BTreeMap<super::plan::UtilityDefinitionId, usize>) -> Self {
        Self {
            utilities: BTreeMap::new(),
            recursive: HashMap::new(),
            call_counts,
        }
    }

    fn must_share(&self, definition: super::plan::UtilityDefinitionId) -> bool {
        self.call_counts
            .get(&definition)
            .copied()
            .unwrap_or_default()
            > 1
    }

    fn compact_names(&self) -> bool {
        self.call_counts.len() > 64
    }

    fn compact_component_name(&self, prefix: &str, component: &str) -> String {
        let suffix = component.rsplit('_').next().unwrap_or(component);
        if self.compact_names()
            && suffix.len() == 16
            && suffix.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            format!("{prefix}{suffix}")
        } else {
            format!("{prefix}{component}")
        }
    }

    fn compact_family(&self, family: &str) -> String {
        if !self.compact_names() {
            return family.to_owned();
        }
        let mut parts = family.split('_').collect::<Vec<_>>();
        let Some(hash_index) = parts
            .iter()
            .position(|part| part.len() == 16 && part.bytes().all(|byte| byte.is_ascii_hexdigit()))
        else {
            return family.to_owned();
        };
        let hash = parts[hash_index];
        let role = parts.drain(hash_index + 1..).collect::<Vec<_>>().join("_");
        if role.is_empty() {
            format!("x{hash}")
        } else {
            format!("x{hash}_{role}")
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct SqlMaterialization {
    pub(super) table: String,
    pub(super) ctes: Vec<String>,
    pub(super) root: String,
}

struct SqlRenderer<'a> {
    clause: usize,
    next_relation: usize,
    ctes: Vec<String>,
    errors: Vec<String>,
    utility_attempts: Vec<UtilityAttempt>,
    utility_stack: Vec<super::plan::UtilityDefinitionId>,
    utilities: &'a mut UtilityCatalog,
    ordered_unions: Vec<(Vec<RelationNode>, String)>,
    joined_relations: HashMap<Vec<String>, String>,
    // Structural equality shares equivalent regions. The Arc retains the canonical graph while
    // this statement is rendered; allocation identity is not part of the comparison.
    shared: Vec<(Arc<RelationNode>, String)>,
    rendered_terminals: HashMap<usize, String>,
    utility_terminal_source: Option<String>,
    utility_row_metadata: Option<String>,
    utility_seed: Option<String>,
    capacity_checked_ctes: usize,
    compact_rendered_bytes: usize,
    compact_base_relations: bool,
    relation_families: BTreeMap<String, usize>,
    share_all: bool,
    packetize: bool,
    materializations: Vec<SqlMaterialization>,
}

impl<'a> SqlRenderer<'a> {
    fn new(clause: usize, share_all: bool, utilities: &'a mut UtilityCatalog) -> Self {
        let compact_base_relations = utilities.call_counts.len() > 8
            || utilities.utilities.len() + utilities.recursive.len() > 16;
        Self {
            clause,
            next_relation: 0,
            ctes: Vec::new(),
            errors: Vec::new(),
            utility_attempts: Vec::new(),
            utility_stack: Vec::new(),
            utilities,
            ordered_unions: Vec::new(),
            joined_relations: HashMap::new(),
            shared: Vec::new(),
            rendered_terminals: HashMap::new(),
            utility_terminal_source: None,
            utility_row_metadata: None,
            utility_seed: None,
            capacity_checked_ctes: 0,
            compact_rendered_bytes: 0,
            compact_base_relations,
            relation_families: BTreeMap::new(),
            share_all,
            packetize: false,
            materializations: Vec::new(),
        }
    }

    fn render(&mut self, node: &RelationNode) -> Result<String, String> {
        let boundary_start = self.ctes.len();
        let attempt_start = self.utility_attempts.len();
        let name = self.render_inner(node)?;
        if self.packetize
            && self.clause != usize::MAX
            && self.utility_stack.is_empty()
            && self.utility_attempts.len() == attempt_start
            && self.ctes.len().saturating_sub(boundary_start) >= 32
            && !name.starts_with('(')
            && !name.starts_with("rerail_packet_")
        {
            return Ok(self.materialize_relation(boundary_start, name));
        }
        Ok(name)
    }

    fn materialize_relation(&mut self, _boundary_start: usize, root: String) -> String {
        let table = format!(
            "rerail_packet_{}_{}",
            self.clause,
            self.materializations.len()
        );
        self.materializations.push(SqlMaterialization {
            table: table.clone(),
            ctes: self.ctes.clone(),
            root,
        });
        self.ctes.clear();
        self.capacity_checked_ctes = 0;
        self.compact_rendered_bytes = 0;
        self.shared.clear();
        self.joined_relations.clear();
        self.ordered_unions.clear();
        self.rendered_terminals.clear();
        self.utility_terminal_source = None;
        self.utility_row_metadata = None;
        if self.clause != usize::MAX {
            self.utility_seed = None;
        }
        table
    }

    fn render_inner(&mut self, node: &RelationNode) -> Result<String, String> {
        const MAX_RENDERED_SQL_BYTES: usize = 8 * 1024 * 1024;
        const CAPACITY_CHECK_INTERVAL: usize = 128;

        if self.next_relation.is_multiple_of(CAPACITY_CHECK_INTERVAL) {
            let rendered_bytes = if self.clause == usize::MAX {
                self.compact_rendered_bytes += self.ctes[self.capacity_checked_ctes..]
                    .iter()
                    .map(|cte| {
                        compact_utility_cte_for_catalog(cte, self.compact_base_relations).len()
                    })
                    .sum::<usize>();
                self.capacity_checked_ctes = self.ctes.len();
                self.compact_rendered_bytes
            } else {
                self.ctes.iter().map(String::len).sum::<usize>()
            };
            let limit = MAX_RENDERED_SQL_BYTES;
            if rendered_bytes > limit {
                let largest_cte = self
                    .ctes
                    .iter()
                    .map(|cte| {
                        compact_utility_cte_for_catalog(cte, self.compact_base_relations).len()
                    })
                    .max()
                    .unwrap_or(0);
                let families = self
                    .relation_families
                    .iter()
                    .map(|(family, count)| format!("{family}={count}"))
                    .collect::<Vec<_>>()
                    .join(",");
                return Err(format!(
                    "relational SQL exceeds bounded renderer capacity: {rendered_bytes} bytes > {limit} bytes across {} CTEs (largest CTE: {largest_cte} bytes; families: {families})",
                    self.ctes.len()
                ));
            }
        }
        let RelationNode::Apply { operation, inputs } = node else {
            return match node {
                RelationNode::Navigate {
                    spec,
                    source,
                    related,
                    stop,
                } => self.render_navigation(*spec, source, related, stop.as_deref()),
                RelationNode::NthChild {
                    spec,
                    source,
                    of_rule,
                } => self.render_nth_child(*spec, source, of_rule.as_deref()),
                RelationNode::Constrain {
                    source,
                    constraints,
                } => self.render_constraints(source, constraints),
                RelationNode::Count {
                    spec,
                    source,
                    related,
                    stop,
                } => self.render_count(spec, source, related, stop.as_deref()),
                RelationNode::Compare { spec, source } => self.render_comparison(spec, source),
                RelationNode::Value { spec, source } => self.render_value(spec, source),
                RelationNode::Semantic { spec } => self.render_semantic(spec),
                RelationNode::Parsed { spec } => self.render_parsed(spec),
                RelationNode::SelectWhen { select, exists } => {
                    let select = self.render(select)?;
                    let exists = self.render(exists)?;
                    let name = self.next_name("select_when");
                    self.ctes.push(format!("{name} AS (\n  SELECT selected.* FROM {select} selected WHERE EXISTS (SELECT 1 FROM {exists} required WHERE required.file_index = selected.file_index)\n)"));
                    Ok(name)
                }
                RelationNode::RequireEach {
                    subject,
                    evidence,
                    keys,
                } => {
                    let subject = self.render(subject)?;
                    let evidence = self.render(evidence)?;
                    let equal = keys
                        .iter()
                        .map(|(left, right)| format!("{left} = {right}"))
                        .collect::<Vec<_>>()
                        .join(" AND ");
                    let name = self.next_name("require_each");
                    self.ctes.push(format!("{name} AS (\n  SELECT subject.* FROM {subject} subject WHERE NOT EXISTS (SELECT 1 FROM {evidence} evidence WHERE {equal})\n)"));
                    Ok(name)
                }
                RelationNode::Budget {
                    source,
                    group_by,
                    distinct,
                    maximum,
                } => {
                    let source = self.render(source)?;
                    let groups = if group_by.is_empty() {
                        "'all'".to_owned()
                    } else {
                        group_by.join(", ")
                    };
                    let count = if distinct.is_empty() {
                        "count(*)".to_owned()
                    } else {
                        format!("count(DISTINCT json_array({}))", distinct.join(", "))
                    };
                    let name = self.next_name("budget");
                    self.ctes.push(format!("{name} AS (\n  SELECT usage.* FROM {source} usage GROUP BY {groups} HAVING {count} > {maximum}\n)"));
                    Ok(name)
                }
                RelationNode::Utility { spec, source, body } => {
                    self.render_utility(spec, source, body)
                }
                RelationNode::UtilityFrame { spec, source, body } => {
                    self.render_utility_frame(spec, source, body)
                }
                RelationNode::RecursiveUtility {
                    spec,
                    source,
                    bases,
                } => self.render_recursive_utility(spec, source, bases),
                RelationNode::Empty => {
                    let name = self.next_name("empty");
                    self.ctes.push(format!(
                        "{name} AS (\n  SELECT terminal_match.*, json('[]') AS bindings, '' AS choice_order FROM terminal_match WHERE 0\n)"
                    ));
                    Ok(name)
                }
                RelationNode::PrefixChoice { source, branch } => {
                    let source = self.render(source)?;
                    let columns = qualified_row_columns("source");
                    let query = format!(
                        "SELECT {columns}, source.bindings, source.choice_order || printf('%08d/', {branch}) AS choice_order FROM {source} source"
                    );
                    if self.clause == usize::MAX {
                        Ok(format!("({query})"))
                    } else {
                        let name = self.next_name("choice");
                        self.ctes.push(format!("{name} AS (\n  {query}\n)"));
                        Ok(name)
                    }
                }
                RelationNode::Shared(shared) => {
                    if !self.share_all {
                        return self.render(shared);
                    }
                    if let Some((_, name)) = self
                        .shared
                        .iter()
                        .find(|(known, _)| known.as_ref() == shared.as_ref())
                    {
                        return Ok(name.clone());
                    }
                    let name = self.render(shared)?;
                    self.shared.push((Arc::clone(shared), name.clone()));
                    Ok(name)
                }
                RelationNode::Rendered(name) => Ok(name.clone()),
                RelationNode::MergeChoices(inputs) => {
                    let inputs = inputs
                        .iter()
                        .map(|input| self.render(input))
                        .collect::<Result<Vec<_>, _>>()?;
                    let query = inputs
                        .iter()
                        .map(|input| format!("SELECT * FROM {input}"))
                        .collect::<Vec<_>>()
                        .join("\n  UNION ALL\n  ");
                    if self.clause == usize::MAX {
                        Ok(format!("({query})"))
                    } else {
                        let name = self.next_name("relation");
                        self.ctes.push(format!("{name} AS (\n  {query}\n)"));
                        Ok(name)
                    }
                }
                RelationNode::Seed => self.utility_seed.clone().ok_or_else(|| {
                    "a clause root must be an executable relation (bare seed)".to_owned()
                }),
                RelationNode::Producer(spec) => Err(format!(
                    "a clause root must be an executable relation (bare {:?} producer)",
                    spec.kind
                )),
                _ => Err("a clause root must be an executable relation".to_owned()),
            };
        };
        match operation {
            SqlOperation::ScanTerminal => self.render_terminal(inputs),
            SqlOperation::ScanCapture
            | SqlOperation::CaseTruth
            | SqlOperation::WindowRank
            | SqlOperation::Projection
            | SqlOperation::OrderBy => self.render_transparent(inputs),
            SqlOperation::InnerJoin => self.render_all(inputs),
            SqlOperation::MapDocument => self.render_document_map(inputs),
            SqlOperation::OrderedUnion => self.render_any(inputs),
            SqlOperation::AntiJoin => self.render_not(inputs),
            unsupported => Err(format!(
                "SQL rendering is not implemented for {unsupported:?}"
            )),
        }
    }

    fn sql_text(value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }

    fn capture_equality(left: &str, right: &str) -> String {
        format!(
            "({left}.node = {right}.node OR (({left}.named_leaf = 1 OR {right}.named_leaf = 1) AND {left}.source_text = {right}.source_text) OR ({left}.named_leaf = 0 AND {right}.named_leaf = 0 AND CASE WHEN json_valid({left}.structural_key) THEN json_extract({left}.structural_key, '$.syntax') ELSE {left}.structural_key END = CASE WHEN json_valid({right}.structural_key) THEN json_extract({right}.structural_key, '$.syntax') ELSE {right}.structural_key END))"
        )
    }

    fn json_capture_equality(left: &str, right: &str) -> String {
        format!(
            "(json_extract({left}.value, '$.node') = json_extract({right}.value, '$.node') OR ((json_extract({left}.value, '$.leaf') = 1 OR json_extract({right}.value, '$.leaf') = 1) AND json_extract({left}.value, '$.text') = json_extract({right}.value, '$.text')) OR (json_extract({left}.value, '$.leaf') = 0 AND json_extract({right}.value, '$.leaf') = 0 AND CASE WHEN json_valid(json_extract({left}.value, '$.structure')) THEN json_extract(json_extract({left}.value, '$.structure'), '$.syntax') ELSE json_extract({left}.value, '$.structure') END = CASE WHEN json_valid(json_extract({right}.value, '$.structure')) THEN json_extract(json_extract({right}.value, '$.structure'), '$.syntax') ELSE json_extract({right}.value, '$.structure') END))"
        )
    }

    fn compact_json_capture_equality(left: &str, right: &str) -> String {
        format!(
            "({left}.value->>'n' = {right}.value->>'n' OR CASE WHEN {left}.value->>'f' = 1 OR {right}.value->>'f' = 1 THEN {left}.value->>'t' = {right}.value->>'t' ELSE {left}.value->>'s' = {right}.value->>'s' END)"
        )
    }

    fn compact_binding_compatibility(left: &str, right: &str) -> String {
        format!(
            "NOT EXISTS (SELECT 1 FROM json_each({left}.bindings) left_binding JOIN json_each({right}.bindings) right_binding ON right_binding.value->>'l' = left_binding.value->>'l' WHERE left_binding.value->>'k' <> right_binding.value->>'k' OR ((left_binding.value->>'k' = 1 OR (left_binding.value->>'k' = 2 AND left_binding.value->>'m' = 1 AND right_binding.value->>'m' = 1 AND left_binding.value->>'e' = right_binding.value->>'e')) AND NOT {equal}) OR (left_binding.value->>'k' = 2 AND left_binding.value->>'c' <> right_binding.value->>'c'))",
            equal = Self::compact_json_capture_equality("left_binding", "right_binding")
        )
    }

    fn json_semantic_structure(left: &str, right: &str) -> String {
        format!(
            "json_extract({left}.value, '$.semantic_structure') = json_extract({right}.value, '$.semantic_structure')"
        )
    }

    fn binding_compatibility(left: &str, right: &str) -> String {
        format!(
            "NOT EXISTS (SELECT 1 FROM json_each({left}.bindings) left_binding JOIN json_each({right}.bindings) right_binding ON json_extract(right_binding.value, '$.label') = json_extract(left_binding.value, '$.label') WHERE json_extract(left_binding.value, '$.kind') <> json_extract(right_binding.value, '$.kind') OR (json_extract(left_binding.value, '$.kind') = 1 AND NOT {equal}) OR (json_extract(left_binding.value, '$.kind') = 2 AND json_extract(left_binding.value, '$.named') = 1 AND json_extract(right_binding.value, '$.named') = 1 AND json_extract(left_binding.value, '$.equality_occurrence') = json_extract(right_binding.value, '$.equality_occurrence') AND NOT {equal}) OR (json_extract(left_binding.value, '$.kind') = 2 AND (SELECT count(*) FROM json_each({left}.bindings) item WHERE json_extract(item.value, '$.label') = json_extract(left_binding.value, '$.label') AND json_extract(item.value, '$.named') = 1) <> (SELECT count(*) FROM json_each({right}.bindings) item WHERE json_extract(item.value, '$.label') = json_extract(right_binding.value, '$.label') AND json_extract(item.value, '$.named') = 1)))",
            equal = Self::json_capture_equality("left_binding", "right_binding")
        )
    }

    fn merged_bindings(inputs: &[String]) -> String {
        let values = inputs
            .iter()
            .enumerate()
            .map(|(input_order, input)| {
                format!(
                    "SELECT {input_order} AS input_order, CAST(key AS INTEGER) AS capture_order, value FROM json_each({input}.bindings)"
                )
            })
            .collect::<Vec<_>>()
            .join(" UNION ALL ");
        format!(
            "COALESCE((SELECT json_group_array(json(value)) FROM (SELECT value FROM ({values}) GROUP BY value ORDER BY min(input_order), min(capture_order))), json('[]'))"
        )
    }

    fn render_terminal(&mut self, inputs: &[RelationNode]) -> Result<String, String> {
        let [RelationNode::Producer(spec)] = inputs else {
            return Err("a terminal scan must have one producer operand".to_owned());
        };
        self.render_terminal_producer(spec.producer_id()?, "term")
    }

    fn render_terminal_producer(
        &mut self,
        producer: usize,
        family: &str,
    ) -> Result<String, String> {
        if self.clause == usize::MAX
            && let Some(rendered) = self.rendered_terminals.get(&producer)
        {
            return Ok(rendered.clone());
        }
        let name = self.next_name(family);
        if self.clause == usize::MAX {
            let terminal_source = if let Some(source) = &self.utility_terminal_source {
                source.clone()
            } else {
                let source = self.next_name("terminal_source");
                let columns = qualified_row_columns_with_verdict("match", "1");
                self.ctes.push(format!(
                    "{source} AS (\n  SELECT {columns}, COALESCE((\n    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'syntax', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.syntax') ELSE captured.structural_key END, 'l', captured.capture_label, 'n', captured.node, 't', captured.source_text, 's', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.syntax') ELSE captured.structural_key END, 'f', captured.named_leaf, 'k', captured.capture_kind, 'm', captured.capture_named, 'e', captured.equality_occurrence, 'c', (SELECT count(*) FROM capture same_label WHERE same_label.producer_id = captured.producer_id AND same_label.file_index = captured.file_index AND same_label.match_index = captured.match_index AND same_label.capture_label = captured.capture_label AND same_label.capture_named = 1), 'semantic_structure', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.semanticPlain') END, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))\n    FROM capture captured\n    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index\n      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'\n    ORDER BY captured.capture_index\n  ), json('[]')) AS bindings, '' AS choice_order\n  FROM terminal_match match\n  INNER JOIN producer_status status USING (producer_id, file_index)\n  WHERE status.complete = 1\n    AND NOT EXISTS (\n      SELECT 1 FROM capture first\n      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index\n        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label\n        AND repeated.capture_index > first.capture_index\n      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index\n        AND first.match_index = match.match_index AND first.capture_label <> 'match'\n        AND first.capture_kind = 1 AND repeated.capture_kind = 1\n        AND NOT {}\n    )\n)",
                    Self::capture_equality("first", "repeated")
                ));
                self.utility_terminal_source = Some(source.clone());
                source
            };
            self.ctes.push(format!(
                "{name} AS (\n  SELECT * FROM {terminal_source} WHERE producer_id = {producer}\n)"
            ));
            self.rendered_terminals.insert(producer, name.clone());
            return Ok(name);
        }
        let columns = qualified_row_columns_with_verdict("match", "1");
        self.ctes.push(format!(
            "{name} AS (\n  SELECT {columns}, COALESCE((\n    SELECT json_group_array(json_object('label', captured.capture_label, 'occurrence', captured.capture_occurrence, 'node', captured.node, 'text', captured.source_text, 'structure', captured.structural_key, 'semantic_structure', CASE WHEN json_valid(captured.structural_key) THEN json_extract(captured.structural_key, '$.semanticPlain') END, 'leaf', captured.named_leaf, 'kind', captured.capture_kind, 'named', captured.capture_named, 'equality_occurrence', captured.equality_occurrence))\n    FROM capture captured\n    WHERE captured.producer_id = match.producer_id AND captured.file_index = match.file_index\n      AND captured.match_index = match.match_index AND captured.capture_label <> 'match'\n    ORDER BY captured.capture_index\n  ), json('[]')) AS bindings, '' AS choice_order\n  FROM terminal_match match\n  INNER JOIN producer_status status USING (producer_id, file_index)\n  WHERE match.producer_id = {producer} AND status.complete = 1\n    AND NOT EXISTS (\n      SELECT 1 FROM capture first\n      JOIN capture repeated ON repeated.producer_id = first.producer_id AND repeated.file_index = first.file_index\n        AND repeated.match_index = first.match_index AND repeated.capture_label = first.capture_label\n        AND repeated.capture_index > first.capture_index\n      WHERE first.producer_id = match.producer_id AND first.file_index = match.file_index\n        AND first.match_index = match.match_index AND first.capture_label <> 'match'\n        AND first.capture_kind = 1 AND repeated.capture_kind = 1\n        AND NOT {}\n    )\n)",
            Self::capture_equality("first", "repeated")
        ));
        Ok(name)
    }

    fn render_document_map(&mut self, inputs: &[RelationNode]) -> Result<String, String> {
        let [input] = inputs else {
            return Err("document mapping must have one generated relation".to_owned());
        };
        let input = self.render(input)?;
        let name = self.next_name("mapped");
        let columns = qualified_row_columns("generated")
            .replace(
                "generated.file_index",
                "mapping.mapped_document_id AS file_index",
            )
            .replace(
                "generated.document_id",
                "mapping.mapped_document_id AS document_id",
            )
            .replace(
                "generated.mapped_document_id",
                "mapping.document_id AS mapped_document_id",
            )
            .replace("generated.start_byte", "mapping.node AS start_byte")
            .replace("generated.end_byte", "mapping.related_node AS end_byte");
        self.ctes.push(format!(
            "{name} AS (\n  SELECT {columns}, generated.bindings, generated.choice_order\n  FROM {input} generated\n  INNER JOIN document_map mapping ON mapping.document_id = COALESCE(generated.document_id, generated.file_index)\n    AND generated.start_byte >= mapping.start_byte AND generated.end_byte <= mapping.end_byte\n  WHERE NOT EXISTS (\n    SELECT 1 FROM document_map narrower\n    WHERE narrower.document_id = mapping.document_id\n      AND generated.start_byte >= narrower.start_byte AND generated.end_byte <= narrower.end_byte\n      AND (narrower.end_byte - narrower.start_byte < mapping.end_byte - mapping.start_byte\n        OR (narrower.end_byte - narrower.start_byte = mapping.end_byte - mapping.start_byte AND narrower.match_index < mapping.match_index))\n  )\n)"
        ));
        Ok(name)
    }

    fn render_transparent(&mut self, inputs: &[RelationNode]) -> Result<String, String> {
        let [input] = inputs else {
            return Err("a unary relational operation must have one operand".to_owned());
        };
        self.render(input)
    }

    fn render_all(&mut self, inputs: &[RelationNode]) -> Result<String, String> {
        if inputs.len() < 2 {
            return Err("ordered conjunction requires at least two operands".to_owned());
        }
        if let [source, RelationNode::Parsed { spec }] = inputs {
            let source = self.render(source)?;
            return self.render_parsed_source(spec, &source);
        }
        if let [source, RelationNode::Semantic { spec }] = inputs
            && let Some(source_producer) = Self::terminal_producer(source)?
        {
            if spec.rules.is_empty() {
                return self.render_simple_semantic_gate(
                    source_producer,
                    spec.producer,
                    &spec.argument_labels,
                    spec.require_all_candidates,
                );
            }
            if !spec.argument_labels.is_empty()
                && let [rule] = spec.rules.as_slice()
                && rule.target_kind.is_none()
                && let Some(relation) = &rule.relation
                && let [root] = relation.roots.as_slice()
                && let Some(target_producer) = Self::terminal_producer(root)?
            {
                return self.render_argument_target(
                    source_producer,
                    spec.producer,
                    target_producer,
                    rule.slot,
                    &spec.argument_labels,
                );
            }
            if spec.argument_labels.is_empty()
                && let [rule] = spec.rules.as_slice()
            {
                if let Some(target_kind) = rule.target_kind {
                    return self.render_semantic_kind_target(
                        source_producer,
                        spec.producer,
                        rule.slot,
                        target_kind,
                        rule.incomplete_position,
                    );
                }
                if let Some(relation) = &rule.relation
                    && let [root] = relation.roots.as_slice()
                    && let Some(target_producer) = Self::terminal_producer(root)?
                {
                    return self.render_semantic_target(
                        source_producer,
                        spec.producer,
                        target_producer,
                        rule.slot,
                        rule.incomplete_position,
                        rule.deduplicate_source,
                    );
                }
            }
        }
        let relations = inputs
            .iter()
            .map(|input| self.render(input))
            .collect::<Result<Vec<_>, _>>()?;
        if self.clause == usize::MAX
            && let Some(relation) = self.joined_relations.get(&relations)
        {
            return Ok(relation.clone());
        }
        let name = self.next_name("relation");
        let binding_inputs = inputs
            .iter()
            .enumerate()
            .filter(|(_, input)| input.can_bind())
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if binding_inputs.is_empty() || (self.clause == usize::MAX && binding_inputs.len() == 1) {
            let aliases = (0..relations.len())
                .map(|index| match index {
                    0 => "left_input".to_owned(),
                    1 => "right_input".to_owned(),
                    _ => format!("input_{index}"),
                })
                .collect::<Vec<_>>();
            let verdict = format!(
                "CASE WHEN {} THEN 0 WHEN {} THEN NULL ELSE 1 END",
                aliases
                    .iter()
                    .map(|alias| format!("{alias}.verdict = 0"))
                    .collect::<Vec<_>>()
                    .join(" OR "),
                aliases
                    .iter()
                    .map(|alias| format!("{alias}.verdict IS NULL"))
                    .collect::<Vec<_>>()
                    .join(" OR ")
            );
            let columns = qualified_row_columns_with_verdict("left_input", &verdict);
            let binding_alias = binding_inputs
                .first()
                .map_or("left_input", |index| aliases[*index].as_str());
            let choices = if binding_inputs.is_empty() {
                "left_input.choice_order".to_owned()
            } else {
                format!(
                    "{} AS choice_order",
                    aliases
                        .iter()
                        .map(|alias| format!("{alias}.choice_order"))
                        .collect::<Vec<_>>()
                        .join(" || ")
                )
            };
            let materialized = if self.compact_base_relations {
                " MATERIALIZED"
            } else {
                ""
            };
            let mut sql = format!(
                "{name} AS{materialized} (\n  SELECT {columns}, {binding_alias}.bindings, {choices} FROM {} left_input",
                relations[0]
            );
            for (index, relation) in relations.iter().enumerate().skip(1) {
                let alias = &aliases[index];
                if self.clause == usize::MAX {
                    sql.push_str(&format!(
                        "\n  INNER JOIN {relation} {alias} USING (file_index, node)"
                    ));
                } else {
                    sql.push_str(&format!(
                        "\n  INNER JOIN {relation} {alias}\n    ON {alias}.file_index = left_input.file_index AND {alias}.node = left_input.node"
                    ));
                }
            }
            sql.push_str("\n)");
            self.ctes.push(sql);
            if self.clause == usize::MAX {
                self.joined_relations.insert(relations, name.clone());
            }
            return Ok(name);
        }
        let aliases = relations
            .iter()
            .enumerate()
            .map(|(index, _)| {
                if index == 0 {
                    "left_input".to_owned()
                } else if index == 1 {
                    "right_input".to_owned()
                } else {
                    format!("input_{index}")
                }
            })
            .collect::<Vec<_>>();
        let verdict = format!(
            "CASE WHEN {} THEN 0 WHEN {} THEN NULL ELSE 1 END",
            aliases
                .iter()
                .map(|alias| format!("{alias}.verdict = 0"))
                .collect::<Vec<_>>()
                .join(" OR "),
            aliases
                .iter()
                .map(|alias| format!("{alias}.verdict IS NULL"))
                .collect::<Vec<_>>()
                .join(" OR ")
        );
        let columns = qualified_row_columns_with_verdict("left_input", &verdict);
        let compatibility_inputs = if self.clause == usize::MAX {
            binding_inputs
        } else {
            (0..inputs.len()).collect()
        };
        let merged = Self::merged_bindings(
            &compatibility_inputs
                .iter()
                .map(|index| aliases[*index].clone())
                .collect::<Vec<_>>(),
        );
        let choices = relations
            .iter()
            .enumerate()
            .map(|(index, _)| {
                if index == 0 {
                    "left_input.choice_order".to_owned()
                } else if index == 1 {
                    "right_input.choice_order".to_owned()
                } else {
                    format!("input_{index}.choice_order")
                }
            })
            .collect::<Vec<_>>()
            .join(" || ");
        let materialized = if self.compact_base_relations {
            " MATERIALIZED"
        } else {
            ""
        };
        let mut sql = format!(
            "{name} AS{materialized} (\n  SELECT {columns}, {merged} AS bindings, {choices} AS choice_order FROM {} left_input",
            relations[0]
        );
        for (index, relation) in relations.iter().enumerate().skip(1) {
            let alias = if index == 1 {
                "right_input".to_owned()
            } else {
                format!("input_{index}")
            };
            let compatible = (self.clause != usize::MAX || inputs[index].can_bind()).then(|| {
                compatibility_inputs
                    .iter()
                    .copied()
                    .take_while(|prior| *prior < index)
                    .map(|prior| Self::binding_compatibility(&aliases[prior], &alias))
                    .collect::<Vec<_>>()
                    .join(" AND ")
            });
            let compatible = compatible
                .filter(|predicate| !predicate.is_empty())
                .map_or_else(String::new, |predicate| format!(" AND {predicate}"));
            if self.clause == usize::MAX {
                sql.push_str(&format!(
                    "\n  INNER JOIN {relation} {alias}\n    ON ({alias}.file_index, {alias}.node) = (left_input.file_index, left_input.node){compatible}"
                ));
            } else {
                sql.push_str(&format!(
                    "\n  INNER JOIN {relation} {alias}\n    ON {alias}.file_index = left_input.file_index AND {alias}.node = left_input.node{compatible}"
                ));
            }
        }
        sql.push_str("\n)");
        self.ctes.push(sql);
        if self.clause == usize::MAX {
            self.joined_relations.insert(relations, name.clone());
        }
        Ok(name)
    }

    fn semantic_join(node: &RelationNode) -> Option<&[RelationNode]> {
        let RelationNode::Apply { operation, inputs } = node else {
            return None;
        };
        if *operation == SqlOperation::InnerJoin {
            return Some(inputs);
        }
        if matches!(
            operation,
            SqlOperation::ScanCapture
                | SqlOperation::CaseTruth
                | SqlOperation::WindowRank
                | SqlOperation::Projection
                | SqlOperation::OrderBy
        ) && let [input] = inputs.as_slice()
        {
            return Self::semantic_join(input);
        }
        None
    }

    fn terminal_producer(node: &RelationNode) -> Result<Option<usize>, String> {
        let RelationNode::Apply { operation, inputs } = node else {
            return Ok(None);
        };
        if *operation == SqlOperation::InnerJoin {
            let producers = inputs
                .iter()
                .map(Self::terminal_producer)
                .collect::<Result<Vec<_>, _>>()?;
            if let Some(producer) = producers.first().copied().flatten()
                && producers
                    .iter()
                    .all(|candidate| *candidate == Some(producer))
            {
                return Ok(Some(producer));
            }
            return Ok(None);
        }
        if *operation == SqlOperation::ScanTerminal {
            let [RelationNode::Producer(producer)] = inputs.as_slice() else {
                return Ok(None);
            };
            return producer.producer_id().map(Some);
        }
        if matches!(
            operation,
            SqlOperation::ScanCapture
                | SqlOperation::CaseTruth
                | SqlOperation::WindowRank
                | SqlOperation::Projection
                | SqlOperation::OrderBy
        ) && let [input] = inputs.as_slice()
        {
            return Self::terminal_producer(input);
        }
        Ok(None)
    }

    fn semantic_bindings(argument_labels: &[String], compact: bool) -> String {
        let mut rows = argument_labels
            .iter()
            .enumerate()
            .map(|(key, label)| {
                let label = Self::sql_text(label);
                let compact = if compact {
                    format!(", 'l', {label}, 'n', argument.related_node, 't', argument.source_text, 's', CASE WHEN json_valid(argument.structural_key) THEN json_extract(argument.structural_key, '$.syntax') ELSE argument.structural_key END, 'f', argument.named_leaf, 'k', 1, 'm', argument.capture_named, 'e', 0, 'c', 1")
                } else {
                    String::new()
                };
                format!(
                    "SELECT json_object('label', {label}, 'occurrence', 0, 'node', argument.related_node, 'text', argument.source_text, 'structure', argument.structural_key, 'leaf', argument.named_leaf, 'kind', 1, 'named', argument.capture_named, 'equality_occurrence', 0{compact}) AS value FROM fact argument WHERE argument.producer_id = gate.producer_id AND argument.file_index = gate.file_index AND argument.node = gate.node AND argument.fact_kind = 15 AND argument.match_index = {key}"
                )
            })
            .collect::<Vec<_>>();
        let compact = if compact {
            ", 'l', input.output_label, 'n', input.related_node, 't', input.source_text, 's', CASE WHEN json_valid(input.structural_key) THEN json_extract(input.structural_key, '$.syntax') ELSE input.structural_key END, 'f', input.named_leaf, 'k', input.capture_kind, 'm', input.capture_named, 'e', input.equality_occurrence, 'c', (SELECT count(*) FROM fact same_label WHERE same_label.producer_id = input.producer_id AND same_label.file_index = input.file_index AND same_label.node = input.node AND same_label.fact_kind = input.fact_kind AND same_label.match_index = input.match_index AND same_label.output_label = input.output_label AND same_label.capture_named = 1)"
        } else {
            ""
        };
        rows.push(format!(
            "SELECT json_object('label', input.output_label, 'occurrence', input.capture_occurrence, 'node', input.related_node, 'text', input.source_text, 'structure', input.structural_key, 'leaf', input.named_leaf, 'kind', input.capture_kind, 'named', input.capture_named, 'equality_occurrence', input.equality_occurrence{compact}) AS value FROM fact input WHERE input.producer_id = gate.producer_id AND input.file_index = gate.file_index AND input.node = gate.node AND input.fact_kind = 17 AND input.match_index = gate.match_index"
        ));
        format!(
            "COALESCE((SELECT json_group_array(json(value)) FROM ({})), json('[]'))",
            rows.join(" UNION ALL ")
        )
    }

    fn render_simple_semantic_gate(
        &mut self,
        source_producer: usize,
        fact_producer: usize,
        argument_labels: &[String],
        require_all_candidates: bool,
    ) -> Result<String, String> {
        let source = self.render_terminal_producer(source_producer, "source")?;
        let gate = self.next_name("semantic_gate");
        let gate_columns = qualified_row_columns_with_verdict(
            "gate",
            "CASE WHEN gate.verdict < 0 THEN NULL ELSE gate.verdict END",
        );
        let bindings = Self::semantic_bindings(argument_labels, self.clause == usize::MAX);
        let candidate_guard = if require_all_candidates {
            "\n    AND NOT EXISTS (SELECT 1 FROM fact candidate WHERE candidate.producer_id = gate.producer_id AND candidate.file_index = gate.file_index AND candidate.node = gate.node AND candidate.match_index = gate.match_index AND candidate.fact_kind = 8 AND candidate.verdict IS NOT 1)"
        } else {
            ""
        };
        self.ctes.push(format!(
            "{gate} AS (\n  SELECT {gate_columns}, {bindings} AS bindings, '' AS choice_order FROM fact gate\n  JOIN producer_status status USING (producer_id, file_index)\n  WHERE gate.producer_id = {fact_producer} AND gate.fact_kind = 0 AND status.complete = 1 AND gate.verdict <> 0{candidate_guard}\n)"
        ));
        let resolved = self.next_name("resolved");
        let verdict = "CASE WHEN source.verdict = 0 OR semantic.verdict = 0 THEN 0 WHEN source.verdict IS NULL OR semantic.verdict IS NULL THEN NULL ELSE 1 END";
        let columns = qualified_row_columns_with_verdict("source", verdict);
        let merged = Self::merged_bindings(&["source".to_owned(), "semantic".to_owned()]);
        let compatible = Self::binding_compatibility("source", "semantic");
        self.ctes.push(format!(
            "{resolved} AS (\n  SELECT {columns}, {merged} AS bindings, source.choice_order || semantic.choice_order AS choice_order FROM {source} source\n  INNER JOIN {gate} semantic\n    ON semantic.file_index = source.file_index AND semantic.node = source.node\n    AND {compatible}\n)"
        ));
        Ok(resolved)
    }

    fn render_argument_target(
        &mut self,
        source_producer: usize,
        fact_producer: usize,
        target_producer: usize,
        slot: u16,
        argument_labels: &[String],
    ) -> Result<String, String> {
        let source = self.render_terminal_producer(source_producer, "source")?;
        let gate = self.next_name("semantic_argument");
        let gate_columns = qualified_row_columns_with_verdict(
            "gate",
            "CASE WHEN gate.verdict < 0 THEN NULL ELSE gate.verdict END",
        );
        let bindings = Self::semantic_bindings(argument_labels, self.clause == usize::MAX);
        self.ctes.push(format!(
            "{gate} AS (\n  SELECT {gate_columns}, {bindings} AS bindings, '' AS choice_order FROM fact gate\n  JOIN producer_status status USING (producer_id, file_index)\n  WHERE gate.producer_id = {fact_producer} AND gate.fact_kind = 0 AND status.complete = 1 AND gate.verdict <> 0\n)"
        ));
        let nested = self.render_terminal_producer(target_producer, "semantic_argument_rule")?;
        let checked = self.next_name("semantic_argument_checked");
        let checked_columns = qualified_row_columns("argument");
        let checked_bindings = Self::merged_bindings(&["argument".to_owned(), "nested".to_owned()]);
        let checked_compatible = Self::binding_compatibility("argument", "nested");
        self.ctes.push(format!(
            "{checked} AS (\n  SELECT DISTINCT {checked_columns}, {checked_bindings} AS bindings, argument.choice_order || nested.choice_order AS choice_order FROM {gate} argument\n  JOIN fact target ON target.producer_id = {fact_producer} AND target.file_index = argument.file_index AND target.node = argument.node AND target.match_index = argument.match_index AND target.fact_kind = {slot} AND target.verdict = 1\n  JOIN {nested} nested ON nested.file_index = argument.file_index AND nested.node = target.related_node AND {checked_compatible}\n)"
        ));
        let resolved = self.next_name("resolved");
        let verdict = "CASE WHEN source.verdict = 0 OR checked.verdict = 0 THEN 0 WHEN source.verdict IS NULL OR checked.verdict IS NULL THEN NULL ELSE 1 END";
        let columns = qualified_row_columns_with_verdict("source", verdict);
        let merged = Self::merged_bindings(&["source".to_owned(), "checked".to_owned()]);
        let compatible = Self::binding_compatibility("source", "checked");
        self.ctes.push(format!(
            "{resolved} AS (\n  SELECT {columns}, {merged} AS bindings, source.choice_order || checked.choice_order AS choice_order FROM {source} source\n  INNER JOIN {checked} checked\n    ON checked.file_index = source.file_index AND checked.node = source.node\n    AND {compatible}\n)"
        ));
        Ok(resolved)
    }

    fn render_semantic_kind_target(
        &mut self,
        source_producer: usize,
        fact_producer: usize,
        slot: u16,
        target_kind: u16,
        incomplete_position: Option<u32>,
    ) -> Result<String, String> {
        let source = self.render_terminal_producer(source_producer, "source")?;
        let target = self.next_name("semantic_site");
        let verdict = "CASE WHEN source.verdict = 0 OR gate.verdict = 0 THEN 0 WHEN source.verdict IS NULL OR gate.verdict < 0 OR gate.verdict IS NULL THEN NULL ELSE 1 END";
        let columns = qualified_row_columns_with_verdict("source", verdict);
        let complete = incomplete_position.map_or_else(String::new, |position| {
            format!(
                " AND NOT EXISTS (SELECT 1 FROM fact incomplete WHERE incomplete.producer_id = gate.producer_id AND incomplete.file_index = gate.file_index AND incomplete.node = gate.node AND incomplete.match_index = gate.match_index AND incomplete.fact_kind = 14 AND incomplete.capture_index = {position} AND incomplete.verdict <> 0)"
            )
        });
        self.ctes.push(format!(
            "{target} AS (\n  SELECT DISTINCT {columns}, source.bindings, source.choice_order FROM {source} source\n  JOIN fact gate ON gate.producer_id = {fact_producer} AND gate.file_index = source.file_index AND gate.node = source.node AND gate.fact_kind = 0\n  JOIN fact target ON target.producer_id = gate.producer_id AND target.file_index = gate.file_index AND target.node = gate.node AND target.match_index = gate.match_index AND target.fact_kind = {slot}\n  JOIN producer_status status ON status.producer_id = gate.producer_id AND status.file_index = gate.file_index\n  WHERE status.complete = 1 AND gate.verdict <> 0 AND target.verdict = 1 AND target.capture_kind = {target_kind}{complete}\n)"
        ));
        Ok(target)
    }

    fn render_semantic_target(
        &mut self,
        source_producer: usize,
        fact_producer: usize,
        target_producer: usize,
        slot: u16,
        incomplete_position: Option<u32>,
        deduplicate_source: bool,
    ) -> Result<String, String> {
        let source = self.render_terminal_producer(source_producer, "source")?;
        let targets = self.next_name("semantic_target");
        let target_verdict = "CASE WHEN source.verdict = 0 OR gate.verdict = 0 THEN 0 WHEN source.verdict IS NULL OR gate.verdict < 0 OR gate.verdict IS NULL THEN NULL ELSE 1 END";
        let target_columns = qualified_row_columns_with_verdict("source", target_verdict);
        let complete = incomplete_position.map_or_else(String::new, |position| {
            format!(
                " AND NOT EXISTS (SELECT 1 FROM fact incomplete WHERE incomplete.producer_id = gate.producer_id AND incomplete.file_index = gate.file_index AND incomplete.node = gate.node AND incomplete.match_index = gate.match_index AND incomplete.fact_kind = 14 AND incomplete.capture_index = {position} AND incomplete.verdict <> 0)"
            )
        });
        self.ctes.push(format!(
            "{targets} AS (\n  SELECT {target_columns}, source.bindings, source.choice_order, target.related_node AS target_node FROM {source} source\n  JOIN fact gate ON gate.producer_id = {fact_producer} AND gate.file_index = source.file_index AND gate.node = source.node AND gate.fact_kind = 0\n  JOIN fact target ON target.producer_id = gate.producer_id AND target.file_index = gate.file_index AND target.node = gate.node AND target.match_index = gate.match_index AND target.fact_kind = {slot}\n  JOIN producer_status status ON status.producer_id = gate.producer_id AND status.file_index = gate.file_index\n  WHERE status.complete = 1 AND gate.verdict <> 0 AND target.verdict = 1{complete}\n)"
        ));
        let nested = self.render_terminal_producer(target_producer, "semantic_target_rule")?;
        let checked = self.next_name("semantic_target_result");
        let checked_verdict = "CASE WHEN target.verdict = 0 OR nested.verdict = 0 THEN 0 WHEN target.verdict IS NULL OR nested.verdict IS NULL THEN NULL ELSE 1 END";
        let checked_columns = qualified_row_columns_with_verdict("target", checked_verdict);
        let merged = Self::merged_bindings(&["target".to_owned(), "nested".to_owned()]);
        let compatible = Self::binding_compatibility("target", "nested");
        let distinct = if deduplicate_source { "DISTINCT " } else { "" };
        self.ctes.push(format!(
            "{checked} AS (\n  SELECT {distinct}{checked_columns}, {merged} AS bindings, target.choice_order || nested.choice_order AS choice_order FROM {targets} target\n  INNER JOIN {nested} nested\n    ON nested.file_index = target.file_index AND nested.node = target.target_node\n    AND {compatible}\n)"
        ));
        Ok(checked)
    }

    fn render_any(&mut self, inputs: &[RelationNode]) -> Result<String, String> {
        if inputs.len() < 2 {
            return Err("ordered alternatives require at least two operands".to_owned());
        }
        if let Some((_, relation)) = self
            .ordered_unions
            .iter()
            .find(|(rendered_inputs, _)| rendered_inputs == inputs)
        {
            return Ok(relation.clone());
        }
        let relations = inputs
            .iter()
            .map(|input| self.render(input))
            .collect::<Result<Vec<_>, _>>()?;
        let name = self.next_name("relation");
        let branches = relations
            .iter()
            .enumerate()
            .map(|(branch, relation)| {
                let columns = qualified_row_columns("branch");
                let blocks = i64::from(inputs[branch].can_bind());
                format!(
                    "SELECT {columns}, branch.bindings, printf('%08d/', {branch}) || branch.choice_order AS choice_order, {branch} AS branch_order, {blocks} AS unknown_blocks FROM {relation} branch"
                )
            })
            .collect::<Vec<_>>();
        let columns = qualified_row_columns("chosen");
        if self.clause == usize::MAX {
            let branches = branches.join("\n      UNION ALL\n      ");
            self.ctes.push(format!(
                "{name} AS (\n  SELECT {columns}, chosen.bindings, chosen.choice_order FROM (\n    SELECT branch.*, row_number() OVER (PARTITION BY file_index, node ORDER BY CASE WHEN verdict = 1 OR (verdict IS NULL AND unknown_blocks = 1) THEN 0 WHEN verdict IS NULL THEN 1 ELSE 2 END, branch_order, choice_order, producer_id, match_index) AS truth_order FROM (\n      {branches}\n    ) branch\n  ) chosen\n  WHERE chosen.truth_order = 1 AND chosen.verdict IS NOT 0\n)"
            ));
        } else {
            let union = format!("{name}_branches");
            self.ctes.push(format!(
                "{union} AS (\n  {}\n)",
                branches.join("\n  UNION ALL\n  ")
            ));
            self.ctes.push(format!(
                "{name} AS (\n  SELECT {columns}, chosen.bindings, chosen.choice_order FROM (\n    SELECT branch.*, row_number() OVER (PARTITION BY file_index, node ORDER BY CASE WHEN verdict = 1 OR (verdict IS NULL AND unknown_blocks = 1) THEN 0 WHEN verdict IS NULL THEN 1 ELSE 2 END, branch_order, choice_order, producer_id, match_index) AS truth_order FROM {union} branch\n  ) chosen\n  WHERE chosen.truth_order = 1 AND chosen.verdict IS NOT 0\n)"
            ));
        }
        self.ordered_unions.push((inputs.to_vec(), name.clone()));
        Ok(name)
    }

    fn render_not(&mut self, inputs: &[RelationNode]) -> Result<String, String> {
        let [universe, excluded] = inputs else {
            return Err("negation requires universe and excluded operands".to_owned());
        };
        let excluded_can_bind = excluded.can_bind();
        let universe = self.render(universe)?;
        let excluded = self.render(excluded)?;
        let name = self.next_name("relation");
        if excluded_can_bind && self.clause == usize::MAX {
            let candidates = self.next_name("relation");
            let compatible = Self::binding_compatibility("universe", "excluded");
            self.ctes.push(format!(
                "{candidates} AS (\n  SELECT DISTINCT universe.file_index, universe.node, universe.bindings AS source_bindings, excluded.verdict FROM {universe} universe JOIN {excluded} excluded ON excluded.file_index = universe.file_index AND excluded.node = universe.node AND {compatible}\n)"
            ));
            let summary = self.next_name("relation");
            self.ctes.push(format!(
                "{summary} AS (\n  SELECT candidate.file_index, candidate.node, candidate.source_bindings, MAX(CASE WHEN candidate.verdict = 1 THEN 1 ELSE 0 END) AS has_true, MAX(CASE WHEN candidate.verdict IS NULL THEN 1 ELSE 0 END) AS has_unknown FROM {candidates} candidate GROUP BY candidate.file_index, candidate.node, candidate.source_bindings\n)"
            ));
            let verdict = "CASE WHEN excluded.has_unknown = 1 THEN NULL ELSE 1 END".to_owned();
            let columns = qualified_row_columns_with_verdict("universe", &verdict);
            self.ctes.push(format!(
                "{name} AS (\n  SELECT {columns}, universe.bindings, universe.choice_order FROM {universe} universe LEFT JOIN {summary} excluded ON excluded.file_index = universe.file_index AND excluded.node = universe.node AND excluded.source_bindings = universe.bindings\n  WHERE universe.verdict = 1 AND COALESCE(excluded.has_true, 0) = 0\n)"
            ));
            return Ok(name);
        }
        let compatible = if excluded_can_bind {
            Self::binding_compatibility("universe", "excluded")
        } else {
            "1 = 1".to_owned()
        };
        let verdict = format!(
            "CASE WHEN EXISTS (SELECT 1 FROM {excluded} excluded WHERE excluded.file_index = universe.file_index AND excluded.node = universe.node AND excluded.verdict IS NULL AND {compatible}) THEN NULL ELSE 1 END"
        );
        let columns = qualified_row_columns_with_verdict("universe", &verdict);
        self.ctes.push(format!(
            "{name} AS (\n  SELECT {columns}, universe.bindings, universe.choice_order FROM {universe} universe\n  WHERE universe.verdict = 1 AND NOT EXISTS (\n    SELECT 1 FROM {excluded} excluded\n    WHERE excluded.file_index = universe.file_index AND excluded.node = universe.node AND excluded.verdict = 1\n      AND {compatible}\n  )\n)"
        ));
        Ok(name)
    }

    fn render_comparison(
        &mut self,
        spec: &ComparisonSpec,
        source: &RelationNode,
    ) -> Result<String, String> {
        let source = self.render(source)?;
        let name = self.next_name("compare");
        let left_name = Self::sql_text(&spec.left_source);
        let right_name = Self::sql_text(&spec.right_source);
        let operand = |alias: &str, transformed: Option<&str>| {
            if let Some(transformed) = transformed {
                return format!("{transformed}.text_value");
            }
            match spec.text {
                CompareText::Source => format!("json_extract({alias}.value, '$.text')"),
                CompareText::Unquoted => format!(
                    "CASE WHEN substr(json_extract({alias}.value, '$.text'), 1, 1) = substr(json_extract({alias}.value, '$.text'), -1, 1) AND substr(json_extract({alias}.value, '$.text'), 1, 1) IN ('\"', '''', '`') THEN substr(json_extract({alias}.value, '$.text'), 2, length(json_extract({alias}.value, '$.text')) - 2) ELSE json_extract({alias}.value, '$.text') END"
                ),
                CompareText::Node => format!(
                    "printf('%d:%s', json_extract({alias}.value, '$.node'), json_extract({alias}.value, '$.structure'))"
                ),
                CompareText::Tokens | CompareText::Index | CompareText::Semantic(_) => {
                    String::new()
                }
            }
        };
        if spec.collation != Collation::Ordinal {
            return Err(
                "non-ordinal comparison collation requires a bounded collation fact".to_owned(),
            );
        }
        let operator = match spec.op {
            CompareOp::Lt => "<",
            CompareOp::Le => "<=",
            CompareOp::Eq => "=",
            CompareOp::Ne => "<>",
            CompareOp::Gt => ">",
            CompareOp::Ge => ">=",
        };
        if spec.text == CompareText::Node && !matches!(spec.op, CompareOp::Eq | CompareOp::Ne) {
            return Err("node identity supports equality comparisons only".to_owned());
        }
        let test = match (spec.text, spec.op) {
            (CompareText::Semantic(SemanticText::Structure(Equality::Plain)), CompareOp::Eq) => {
                Self::json_semantic_structure("left_value", "right_value")
            }
            (CompareText::Semantic(SemanticText::Structure(Equality::Plain)), CompareOp::Ne) => {
                format!(
                    "NOT ({})",
                    Self::json_semantic_structure("left_value", "right_value")
                )
            }
            (CompareText::Tokens | CompareText::Index | CompareText::Semantic(_), _) => {
                return Err("comparison text mode is not lowered in phase two".to_owned());
            }
            _ => format!(
                "{} {operator} {}",
                operand("left_value", spec.left_transform.map(|_| "left_transform")),
                operand(
                    "right_value",
                    spec.right_transform.map(|_| "right_transform")
                )
            ),
        };
        let unavailable_structure = matches!(
            spec.text,
            CompareText::Semantic(SemanticText::Structure(Equality::Plain))
        )
        .then(|| {
            " OR json_extract(left_value.value, '$.semantic_structure') IS NULL OR json_extract(right_value.value, '$.semantic_structure') IS NULL"
        })
        .unwrap_or_default();
        let unavailable_transform = [
            spec.left_transform
                .map(|_| " OR left_transform.text_value IS NULL"),
            spec.right_transform
                .map(|_| " OR right_transform.text_value IS NULL"),
        ]
        .into_iter()
        .flatten()
        .collect::<String>();
        let verdict = format!(
            "CASE WHEN source.verdict = 0 THEN 0 WHEN source.verdict IS NULL OR left_value.value IS NULL OR right_value.value IS NULL{unavailable_structure}{unavailable_transform} THEN NULL WHEN {test} THEN 1 ELSE 0 END"
        );
        let transform_join = |side: &str, producer: usize| {
            format!(
                "\n  LEFT JOIN fact {side}_transform ON {side}_transform.producer_id = {producer} AND {side}_transform.file_index = source.file_index AND {side}_transform.fact_kind = 19 AND {side}_transform.related_node = json_extract({side}_value.value, '$.node') AND {side}_transform.capture_label = {} AND {side}_transform.source_text = json_extract({side}_value.value, '$.text') AND {side}_transform.output_label = {}",
                Self::sql_text(if side == "left" {
                    &spec.left_source
                } else {
                    &spec.right_source
                }),
                Self::sql_text(if side == "left" {
                    &spec.left
                } else {
                    &spec.right
                })
            )
        };
        let mut transform_joins = String::new();
        if let Some(producer) = spec.left_transform {
            transform_joins.push_str(&transform_join("left", producer));
        }
        if let Some(producer) = spec.right_transform {
            transform_joins.push_str(&transform_join("right", producer));
        }
        let columns = qualified_row_columns_with_verdict("source", &verdict);
        self.ctes.push(format!(
            "{name} AS (\n  SELECT {columns}, source.bindings, source.choice_order FROM {source} source\n  LEFT JOIN json_each(source.bindings) left_value ON json_extract(left_value.value, '$.label') = {left_name} AND json_extract(left_value.value, '$.kind') = 1 AND json_extract(left_value.value, '$.occurrence') = 0\n  LEFT JOIN json_each(source.bindings) right_value ON json_extract(right_value.value, '$.label') = {right_name} AND json_extract(right_value.value, '$.kind') = 1 AND json_extract(right_value.value, '$.occurrence') = 0{transform_joins}\n)"
        ));
        Ok(name)
    }

    fn render_value(
        &mut self,
        spec: &ValuePredicateSpec,
        source: &RelationNode,
    ) -> Result<String, String> {
        let source = if matches!(source, RelationNode::Seed) {
            let name = self.next_name("value_source");
            self.ctes.push(format!(
                "{name} AS (\n  SELECT fact.*, json('[]') AS bindings, '' AS choice_order FROM fact WHERE fact.producer_id = {} AND fact.fact_kind = 1\n)",
                spec.producer
            ));
            name
        } else if let Some(producer) = Self::terminal_producer(source)? {
            self.render_terminal_producer(producer, "source")?
        } else {
            self.render(source)?
        };
        let name = self.next_name("value");
        let expected = match &spec.eq {
            Some(ValueLiteral::Null) => "null".to_owned(),
            Some(ValueLiteral::Bool(value))
                if spec.language == super::super::language::GrammarDialect::Python =>
            {
                format!("int:{}", i64::from(*value))
            }
            Some(ValueLiteral::Bool(value)) => format!("bool:{}", i64::from(*value)),
            Some(ValueLiteral::Integer(value)) => format!("int:{value}"),
            Some(ValueLiteral::Float(value)) => {
                let value = value
                    .parse::<f64>()
                    .map_err(|_| "value comparison has an invalid float literal")?;
                if spec.language == super::super::language::GrammarDialect::Python
                    && value.is_finite()
                    && value.fract() == 0.0
                {
                    format!(
                        "int:{}",
                        num_bigint::BigInt::from_f64(value)
                            .ok_or("integral value comparison exceeds its numeric fact")?
                    )
                } else {
                    format!("float:{:016x}", value.to_bits())
                }
            }
            Some(ValueLiteral::String(value)) => format!("string:{value}"),
            None => String::new(),
        };
        let verdict = if spec.eq.is_some() {
            let expected = Self::sql_text(&expected);
            format!(
                "CASE WHEN source.verdict = 0 OR value_fact.verdict = 0 THEN 0 WHEN source.verdict IS NULL OR value_fact.verdict IS NULL THEN NULL WHEN value_fact.text_value = {expected} THEN 1 ELSE 0 END"
            )
        } else {
            "CASE WHEN source.verdict = 0 OR value_fact.verdict = 0 THEN 0 WHEN source.verdict IS NULL OR value_fact.verdict IS NULL THEN NULL ELSE 1 END".to_owned()
        };
        let columns = qualified_row_columns_with_verdict("source", &verdict);
        self.ctes.push(format!(
            "{name} AS (\n  SELECT {columns}, source.bindings, source.choice_order FROM {source} source\n  JOIN fact value_fact ON value_fact.producer_id = {} AND value_fact.file_index = source.file_index AND value_fact.node = source.node AND value_fact.fact_kind = 1\n)",
            spec.producer
        ));
        Ok(name)
    }

    fn render_parsed(&mut self, spec: &ParseFactSpec) -> Result<String, String> {
        let host = self.render_terminal_producer(spec.host_producer, "parse_host")?;
        self.render_parsed_source(spec, &host)
    }

    fn render_parsed_source(
        &mut self,
        spec: &ParseFactSpec,
        source: &str,
    ) -> Result<String, String> {
        let name = self.next_name("parse");
        let mut columns = qualified_row_columns("host");
        let bindings = if let Some(export) = &spec.export {
            columns = columns
                .replace("host.start_byte", "candidate.start_byte AS start_byte")
                .replace("host.end_byte", "candidate.end_byte AS end_byte");
            format!(
                "COALESCE((SELECT json_group_array(json(value)) FROM (SELECT value FROM json_each(host.bindings) UNION ALL SELECT json_object('label', {}, 'occurrence', 0, 'node', NULL, 'text', candidate.source_text, 'structure', NULL, 'leaf', candidate.named_leaf, 'kind', 1, 'named', candidate.capture_named, 'equality_occurrence', 0, 'start_byte', candidate.start_byte, 'end_byte', candidate.end_byte))), json('[]'))",
                Self::sql_text(export)
            )
        } else {
            "host.bindings".to_owned()
        };
        let required_host = spec.required_host_producer.map_or_else(String::new, |producer| {
            format!(
                "\n    AND EXISTS (SELECT 1 FROM terminal_match required JOIN producer_status required_status USING (producer_id, file_index) WHERE required.producer_id = {producer} AND required.file_index = host.file_index AND required.node = host.node AND required_status.complete = 1)"
            )
        });
        self.ctes.push(format!(
            "{name} AS (\n  SELECT {columns}, {bindings} AS bindings, '' AS choice_order\n  FROM {source} host\n  JOIN fact candidate ON candidate.producer_id = {producer} AND candidate.file_index = host.file_index AND candidate.node = host.node AND candidate.fact_kind = 18 AND candidate.capture_kind = {target_kind} AND candidate.axis_order > 0\n  JOIN producer_status parse_status ON parse_status.producer_id = candidate.producer_id AND parse_status.file_index = candidate.file_index AND parse_status.complete = 1\n  WHERE 1 = 1{required_host}\n    AND NOT EXISTS (SELECT 1 FROM fact earlier WHERE earlier.producer_id = candidate.producer_id AND earlier.file_index = candidate.file_index AND earlier.node = candidate.node AND earlier.fact_kind = 18 AND earlier.capture_kind = candidate.capture_kind AND earlier.axis_order > 0 AND earlier.axis_order < candidate.axis_order)\n)",
            producer = spec.producer,
            target_kind = spec.target_kind,
            source = source,
        ));
        Ok(name)
    }

    fn render_semantic(&mut self, spec: &SemanticFactSpec) -> Result<String, String> {
        let mut current = self.next_name("semantic_gate");
        let bindings = Self::semantic_bindings(&spec.argument_labels, self.clause == usize::MAX);
        let gate_columns = qualified_row_columns_with_verdict(
            "gate",
            "CASE WHEN gate.verdict < 0 THEN NULL ELSE gate.verdict END",
        );
        self.ctes.push(format!(
            "{current} AS (\n  SELECT {gate_columns}, {bindings} AS bindings, '' AS choice_order FROM fact gate\n  JOIN producer_status status USING (producer_id, file_index)\n  WHERE gate.producer_id = {} AND gate.fact_kind = 0 AND status.complete = 1 AND gate.verdict <> 0\n)",
            spec.producer
        ));
        for rule in &spec.rules {
            let name = self.next_name("semantic_rule");
            if let Some(kind) = rule.target_kind {
                let columns = qualified_row_columns("source");
                self.ctes.push(format!(
                    "{name} AS (\n  SELECT DISTINCT {columns}, source.bindings, source.choice_order FROM {current} source\n  JOIN fact target ON target.producer_id = {} AND target.file_index = source.file_index AND target.node = source.node AND target.match_index = source.match_index AND target.fact_kind = {} AND target.verdict = 1 AND target.capture_kind = {kind}\n)",
                    spec.producer, rule.slot
                ));
            } else {
                let relation = rule
                    .relation
                    .as_ref()
                    .ok_or("semantic target rule has no relational operand")?;
                let [root] = relation.roots.as_slice() else {
                    return Err("semantic target rule must have one relational root".to_owned());
                };
                let checked = self.render(root)?;
                let columns = qualified_row_columns("source");
                let merged = Self::merged_bindings(&["source".to_owned(), "checked".to_owned()]);
                let compatible = Self::binding_compatibility("source", "checked");
                self.ctes.push(format!(
                    "{name} AS (\n  SELECT DISTINCT {columns}, {merged} AS bindings, source.choice_order || checked.choice_order AS choice_order FROM {current} source\n  JOIN fact target ON target.producer_id = {} AND target.file_index = source.file_index AND target.node = source.node AND target.match_index = source.match_index AND target.fact_kind = {} AND target.verdict = 1\n  JOIN {checked} checked ON checked.file_index = source.file_index AND checked.node = target.related_node AND {compatible}\n)",
                    spec.producer, rule.slot
                ));
            }
            current = name;
        }
        Ok(current)
    }

    fn record_utility_attempts(&mut self, spec: &UtilitySpec, source: &str) {
        let (source, inline) = if self.clause == usize::MAX {
            (source.to_owned(), true)
        } else {
            let attempts = self.next_name(&format!("utility_{}_attempts", spec.component));
            self.ctes.push(format!(
                "{attempts} AS (\n  SELECT source.file_index, source.node, {} AS depth FROM {source} source\n)",
                spec.depth
            ));
            (attempts, false)
        };
        self.utility_attempts.push(UtilityAttempt {
            source,
            depth: spec.depth,
            inline,
            max_depth: spec.max_depth,
            max_work: spec.max_work,
            universe_producer: spec.universe_producer,
        });
    }

    fn record_utility_dependency(&mut self, definition: super::plan::UtilityDefinitionId) {
        let Some(parent) = self.utility_stack.last() else {
            return;
        };
        if *parent == definition {
            return;
        }
        if let Some(utility) = self.utilities.utilities.get_mut(parent) {
            utility.dependencies.insert(definition);
        }
    }

    fn shared_utility(
        &mut self,
        definition: super::plan::UtilityDefinitionId,
        component: &str,
        universe_producer: usize,
        body: &RelationNode,
    ) -> String {
        self.record_utility_dependency(definition);
        let published = format!("utility_{component}_shared");
        let candidate = self.utilities.compact_component_name("su", component);
        let published_output = (candidate != published).then_some(published);
        let utility = self
            .utilities
            .utilities
            .entry(definition)
            .or_insert_with(|| SharedUtility {
                component: component.to_owned(),
                body: body.clone(),
                universe_producer,
                output: candidate,
                published_output,
                demand: None,
                inputs: Vec::new(),
                dependencies: BTreeSet::new(),
                rendered: false,
            });
        utility.output.clone()
    }

    fn shared_framed_utility(
        &mut self,
        definition: super::plan::UtilityDefinitionId,
        component: &str,
        universe_producer: usize,
        body: &RelationNode,
        input: String,
    ) -> (String, String) {
        self.record_utility_dependency(definition);
        let published = format!("utility_{component}_shared");
        let candidate = self.utilities.compact_component_name("su", component);
        let published_output = (candidate != published).then_some(published);
        let demand = format!("utility_{component}_demand");
        let utility = self
            .utilities
            .utilities
            .entry(definition)
            .or_insert_with(|| SharedUtility {
                component: component.to_owned(),
                body: body.clone(),
                universe_producer,
                output: candidate,
                published_output,
                demand: Some(demand),
                inputs: Vec::new(),
                dependencies: BTreeSet::new(),
                rendered: false,
            });
        let frame = utility.inputs.len().to_string();
        utility.inputs.push(input);
        (utility.output.clone(), frame)
    }

    fn join_utility_call(&mut self, source: String, shared: String) -> Result<String, String> {
        self.render_all(&[
            RelationNode::Rendered(source),
            RelationNode::Rendered(shared),
        ])
    }

    fn render_utility(
        &mut self,
        spec: &UtilitySpec,
        source: &RelationNode,
        body: &RelationNode,
    ) -> Result<String, String> {
        let source = self.render(source)?;
        self.record_utility_attempts(spec, &source);
        if !self.utilities.must_share(spec.definition) {
            let body = body
                .utility_body_clone()
                .apply_after(RelationNode::Rendered(source));
            let body = self.render(&body)?;
            let name = self.next_name(&format!("utility_{}", spec.component));
            self.ctes
                .push(format!("{name} AS (\n  SELECT * FROM {body}\n)"));
            return Ok(name);
        }
        let shared = self.shared_utility(
            spec.definition,
            &spec.component,
            spec.universe_producer,
            body,
        );
        self.join_utility_call(source, shared)
    }

    fn render_utility_frame(
        &mut self,
        spec: &UtilityFrameSpec,
        source: &RelationNode,
        body: &RelationNode,
    ) -> Result<String, String> {
        let source = self.render(source)?;
        self.record_utility_attempts(&spec.utility, &source);
        let share = self.utilities.must_share(spec.utility.definition);
        let input = self.next_name(&format!("utility_{}_frame", spec.utility.component));
        let mut input_values = vec![
            "SELECT json_set(value, '$.label', '@caller:' || json_extract(value, '$.label')) AS value FROM json_each(source.bindings)".to_owned(),
        ];
        for (parameter, caller) in &spec.arguments {
            input_values.push(format!(
                "SELECT json_set(value, '$.label', {}) AS value FROM json_each(source.bindings) WHERE json_extract(value, '$.label') = {}",
                Self::sql_text(parameter),
                Self::sql_text(caller)
            ));
        }
        let columns = qualified_row_columns("source");
        self.ctes.push(format!(
            "{input} AS (\n  SELECT {columns}, COALESCE((SELECT json_group_array(json(value)) FROM ({})), json('[]')) AS bindings, source.choice_order FROM {source} source\n)",
            input_values.join(" UNION ALL ")
        ));
        let mut output_values = vec![
            "SELECT json_set(value, '$.label', substr(json_extract(value, '$.label'), 9)) AS value FROM json_each(result.bindings) WHERE json_extract(value, '$.label') LIKE '@caller:%'".to_owned(),
        ];
        for (parameter, caller) in &spec.arguments {
            output_values.push(format!(
                "SELECT json_set(value, '$.label', {}) AS value FROM json_each(result.bindings) WHERE json_extract(value, '$.label') = {}",
                Self::sql_text(caller),
                Self::sql_text(parameter)
            ));
        }
        if !share {
            let body = body
                .utility_body_clone()
                .apply_after(RelationNode::Rendered(input));
            let body = self.render(&body)?;
            let output = self.next_name(&format!("utility_{}", spec.utility.component));
            let columns = qualified_row_columns("result");
            self.ctes.push(format!(
                "{output} AS (\n  SELECT {columns}, COALESCE((SELECT json_group_array(json(value)) FROM (SELECT value FROM ({}) GROUP BY value)), json('[]')) AS bindings, result.choice_order FROM {body} result\n)",
                output_values.join(" UNION ALL ")
            ));
            return Ok(output);
        }
        let frame_input =
            self.next_name(&format!("utility_{}_frame_input", spec.utility.component));
        let (shared, frame) = self.shared_framed_utility(
            spec.utility.definition,
            &spec.utility.component,
            spec.utility.universe_producer,
            body,
            frame_input.clone(),
        );
        let frame_columns = qualified_row_columns("frame");
        let frame_node = -(frame.parse::<i64>().expect("a numeric utility frame") + 1);
        let frame_text = Self::sql_text(&frame);
        self.ctes.push(format!(
            "{frame_input} AS (\n  SELECT {frame_columns}, COALESCE((SELECT json_group_array(json(value)) FROM (SELECT value FROM json_each(frame.bindings) UNION ALL SELECT json_object('label', '@utility-frame', 'occurrence', 0, 'node', {frame_node}, 'text', {frame_text}, 'structure', NULL, 'leaf', 1, 'kind', 1, 'named', 0, 'equality_occurrence', 0) AS value)), json('[]')) AS bindings, frame.choice_order FROM {input} frame\n)"
        ));
        let output = self.next_name(&format!("utility_{}", spec.utility.component));
        let columns = qualified_row_columns("result");
        self.ctes.push(format!(
            "{output} AS (\n  SELECT {columns}, COALESCE((SELECT json_group_array(json(value)) FROM (SELECT value FROM ({}) GROUP BY value)), json('[]')) AS bindings, result.choice_order FROM {shared} result WHERE EXISTS (SELECT 1 FROM json_each(result.bindings) frame WHERE json_extract(frame.value, '$.label') = '@utility-frame' AND json_extract(frame.value, '$.kind') = 1 AND json_extract(frame.value, '$.node') = {frame_node})\n)",
            output_values.join(" UNION ALL ")
        ));
        Ok(output)
    }

    fn rename_owned_cte(&mut self, current: &str, owner: &str, first_owned_cte: usize) -> bool {
        let prefix = format!("{current} AS (");
        let Some(cte) = self.ctes[first_owned_cte..]
            .iter_mut()
            .find(|cte| cte.starts_with(&prefix))
        else {
            return false;
        };
        cte.replace_range(..current.len(), owner);
        for rendered in self.rendered_terminals.values_mut() {
            if rendered == current {
                owner.clone_into(rendered);
            }
        }
        for (_, rendered) in &mut self.shared {
            if rendered == current {
                owner.clone_into(rendered);
            }
        }
        for (_, rendered) in &mut self.ordered_unions {
            if rendered == current {
                owner.clone_into(rendered);
            }
        }
        for rendered in self.joined_relations.values_mut() {
            if rendered == current {
                owner.clone_into(rendered);
            }
        }
        true
    }

    fn utility_row_metadata(&mut self) -> String {
        if let Some(metadata) = &self.utility_row_metadata {
            return metadata.clone();
        }
        let metadata = "utility_row_metadata".to_owned();
        let dead = UTILITY_DEAD_COLUMNS.join(", ");
        self.ctes.push(format!(
            "{metadata} AS (\n  SELECT producer_id, file_index, match_index, {dead} FROM terminal_match\n)"
        ));
        self.utility_row_metadata = Some(metadata.clone());
        metadata
    }

    fn finish_shared_utilities(&mut self) -> Result<(), String> {
        while let Some(component) = self
            .utilities
            .utilities
            .iter()
            .find_map(|(component, utility)| (!utility.rendered).then_some(*component))
        {
            let component_start = self.ctes.len();
            let (provenance, body, universe_producer, output, published_output, demand, inputs) = {
                let utility = self
                    .utilities
                    .utilities
                    .get_mut(&component)
                    .expect("the pending utility exists");
                utility.rendered = true;
                (
                    utility.component.clone(),
                    utility.body.clone(),
                    utility.universe_producer,
                    utility.output.clone(),
                    utility.published_output.clone(),
                    utility.demand.clone(),
                    utility.inputs.clone(),
                )
            };
            let has_external_demand = demand.is_some();
            let (body, seed) = if let Some(demand) = demand {
                let union = inputs
                    .iter()
                    .map(|input| format!("SELECT * FROM {input}"))
                    .collect::<Vec<_>>()
                    .join("\n  UNION ALL\n  ");
                self.ctes.push(format!("{demand} AS (\n  {union}\n)"));
                (
                    body.utility_body_clone()
                        .apply_after(RelationNode::Rendered(demand.clone())),
                    demand,
                )
            } else {
                let universe = RelationNode::Apply {
                    operation: SqlOperation::ScanTerminal,
                    inputs: vec![RelationNode::Producer(TerminalSpec::new(
                        TerminalKind::Universe,
                        format!("terminal:{universe_producer}"),
                    ))],
                };
                let seed = self.render(&universe)?;
                (body, seed)
            };
            self.utility_stack.push(component);
            let previous_seed = self.utility_seed.replace(seed);
            let body_start = self.ctes.len();
            let rendered = self
                .render(&body)
                .map_err(|error| format!("utility component `{provenance}`: {error}"));
            self.utility_seed = previous_seed;
            let rendered = rendered?;
            let popped = self.utility_stack.pop();
            debug_assert_eq!(popped, Some(component));
            let first_owned_cte = if self.packetize {
                let owned = self.ctes.split_off(component_start);
                self.capacity_checked_ctes = self.ctes.len();
                self.compact_rendered_bytes = self
                    .ctes
                    .iter()
                    .map(|cte| {
                        compact_utility_cte_for_catalog(cte, self.compact_base_relations).len()
                    })
                    .sum();
                self.shared.clear();
                self.joined_relations.clear();
                self.ordered_unions.clear();
                self.rendered_terminals.clear();
                self.utility_terminal_source = None;
                self.utility_row_metadata = None;
                self.finish_shared_utilities()?;
                let first_owned_cte = self.ctes.len() + body_start.saturating_sub(component_start);
                self.ctes.extend(owned);
                first_owned_cte
            } else {
                self.finish_shared_utilities()?;
                body_start
            };
            let dependencies = &self
                .utilities
                .utilities
                .get(&component)
                .expect("the rendered utility exists")
                .dependencies;
            debug_assert!(dependencies.iter().all(|dependency| {
                self.utilities
                    .utilities
                    .get(dependency)
                    .is_some_and(|utility| utility.rendered)
            }));
            let dependencies_materialized = dependencies.iter().all(|dependency| {
                self.utilities
                    .utilities
                    .get(dependency)
                    .is_some_and(|utility| {
                        let output = utility.published_output.as_ref().unwrap_or(&utility.output);
                        self.materializations
                            .iter()
                            .any(|materialization| materialization.table == *output)
                    })
            });
            let internal = format!("{output}_body");
            if !self.rename_owned_cte(&rendered, &internal, first_owned_cte) {
                self.ctes
                    .push(format!("{internal} AS (\n  SELECT * FROM {rendered}\n)"));
            }
            let metadata = self.utility_row_metadata();
            self.ctes.push(format!(
                "{output} AS (\n  SELECT /* utility-interface */ packed.*, result.* FROM {internal} result JOIN {metadata} packed USING (producer_id, file_index, match_index)\n)"
            ));
            let public_output = if let Some(published_output) = published_output {
                self.ctes.push(format!(
                    "{published_output} AS (\n  SELECT * FROM {output}\n)"
                ));
                published_output
            } else {
                output
            };
            if self.packetize && !has_external_demand && dependencies_materialized {
                let retained = first_owned_cte;
                self.materializations.push(SqlMaterialization {
                    table: public_output.clone(),
                    ctes: self.ctes.clone(),
                    root: public_output,
                });
                self.ctes.truncate(retained);
                self.capacity_checked_ctes = self.ctes.len();
                self.compact_rendered_bytes = self
                    .ctes
                    .iter()
                    .map(|cte| {
                        compact_utility_cte_for_catalog(cte, self.compact_base_relations).len()
                    })
                    .sum();
                self.shared.clear();
                self.joined_relations.clear();
                self.ordered_unions.clear();
                self.rendered_terminals.clear();
                self.utility_terminal_source = None;
                self.utility_row_metadata = None;
            }
        }
        Ok(())
    }

    fn finish_utility_limits(&mut self) -> Result<(), String> {
        let Some(first) = self.utility_attempts.first() else {
            return Ok(());
        };
        let (max_depth, max_work, universe_producer) =
            (first.max_depth, first.max_work, first.universe_producer);
        if self.utility_attempts.iter().any(|attempt| {
            attempt.max_depth != max_depth
                || attempt.max_work != max_work
                || attempt.universe_producer != universe_producer
        }) {
            return Err("one clause has inconsistent utility limits or universes".to_owned());
        }
        let attempts = self.next_name("utility_attempts");
        let union = self
            .utility_attempts
            .iter()
            .map(|attempt| {
                if attempt.inline {
                    format!(
                        "SELECT file_index, node, {} AS depth FROM {}",
                        attempt.depth, attempt.source
                    )
                } else {
                    format!("SELECT * FROM {}", attempt.source)
                }
            })
            .collect::<Vec<_>>()
            .join("\n  UNION ALL\n  ");
        self.ctes.push(format!("{attempts} AS (\n  {union}\n)"));
        let summary = self.next_name("utility_limits");
        self.ctes.push(format!(
            "{summary} AS (\n  SELECT file_index, CASE WHEN max(depth) > {} THEN 'utility-depth-limit' WHEN count(*) > {} THEN 'utility-work-limit' END AS error_code, count(*) AS work_used FROM {attempts} GROUP BY file_index HAVING max(depth) > {} OR count(*) > {}\n)",
            max_depth, max_work, max_depth, max_work
        ));
        let errors = self.next_name("utility_errors");
        let error_columns = qualified_row_columns_with_overrides(
            "root",
            &[
                ("row_kind", "8"),
                ("verdict", "NULL"),
                ("complete", "0"),
                ("work_used", "limited.work_used"),
                ("error_code", "limited.error_code"),
            ],
        );
        self.ctes.push(format!(
            "{errors} AS (\n  SELECT {error_columns}, json('[]') AS bindings, '' AS choice_order FROM {summary} limited JOIN terminal_match root ON root.producer_id = {} AND root.file_index = limited.file_index AND root.node = (SELECT min(candidate.node) FROM terminal_match candidate WHERE candidate.producer_id = {} AND candidate.file_index = limited.file_index)\n)",
            universe_producer, universe_producer
        ));
        self.errors.push(errors);
        Ok(())
    }

    fn apply_recursive_utility(
        &mut self,
        source: &RelationNode,
        result: String,
        family: &str,
    ) -> Result<String, String> {
        if matches!(source, RelationNode::Seed) {
            return Ok(result);
        }
        let source = self.render(source)?;
        let applied = self.next_name(family);
        let columns = qualified_row_columns("source");
        let merged = Self::merged_bindings(&["source".to_owned(), "matched".to_owned()]);
        let compatible = Self::binding_compatibility("source", "matched");
        self.ctes.push(format!(
            "{applied} AS (\n  SELECT {columns}, {merged} AS bindings, source.choice_order || matched.choice_order AS choice_order FROM {source} source JOIN {result} matched ON matched.file_index = source.file_index AND matched.node = source.node AND {compatible}\n)"
        ));
        Ok(applied)
    }

    fn recursive_child_bindings(transition: &RecursiveUtilityTransition) -> String {
        if transition.arguments.is_empty()
            || transition
                .arguments
                .iter()
                .all(|(parameter, caller)| parameter == caller)
        {
            return "child.bindings".to_owned();
        }
        let cases = transition
            .arguments
            .iter()
            .map(|(parameter, caller)| {
                format!(
                    "WHEN {} THEN json_set(value, '$.label', {})",
                    Self::sql_text(parameter),
                    Self::sql_text(caller)
                )
            })
            .collect::<Vec<_>>()
            .join(" ");
        format!(
            "COALESCE((SELECT json_group_array(json(value)) FROM (SELECT CASE json_extract(value, '$.label') {cases} ELSE value END AS value FROM json_each(child.bindings) ORDER BY key)), json('[]'))"
        )
    }

    fn render_recursive_utility(
        &mut self,
        spec: &RecursiveUtilitySpec,
        source: &RelationNode,
        bases: &[(usize, RelationNode)],
    ) -> Result<String, String> {
        if let Some((result, errors)) = self.utilities.recursive.get(&spec.component).cloned() {
            self.errors.push(errors);
            return self.apply_recursive_utility(
                source,
                result,
                &format!("utility_{}", spec.component),
            );
        }
        let rendered_bases = bases
            .iter()
            .map(|(component, base)| Ok((*component, self.render(base)?)))
            .collect::<Result<Vec<_>, String>>()?;
        let family = format!("utility_{}", spec.component);
        let ancestors = self.next_name(&format!("{family}_ancestors"));
        self.ctes.push(format!(
            "{ancestors}(file_index, ancestor, descendant) AS (\n  SELECT edge.file_index, edge.node, edge.related_node FROM navigation edge WHERE edge.producer_id = {}\n  UNION\n  SELECT edge.file_index, edge.node, path.descendant FROM {ancestors} path JOIN navigation edge ON edge.producer_id = {} AND edge.file_index = path.file_index AND edge.related_node = path.ancestor\n)",
            spec.navigation_producer, spec.navigation_producer
        ));
        let relation = self.next_name(&format!("{family}_fix"));
        let empty_columns = qualified_row_columns_with_verdict("seed", "1");
        let mut terms = vec![format!(
            "SELECT 0 AS utility_component, {empty_columns}, json('[]') AS bindings, '' AS choice_order FROM terminal_match seed WHERE 0"
        )];
        for (component, base) in rendered_bases {
            terms.push(format!(
                "SELECT {component} AS utility_component, base.* FROM {base} base"
            ));
        }
        for transition in &spec.transitions {
            if transition.guards.is_empty()
                && transition
                    .arguments
                    .iter()
                    .all(|(parameter, caller)| parameter == caller)
                && transition.steps.len() == 1
                && transition.steps[0].axis == NavigationAxis::Descendant
                && transition.steps[0].field.is_none()
            {
                let step = transition.steps[0];
                let parent_relation = if step.stop == NavigationStop::Neighbor {
                    format!(
                        "navigation edge ON edge.producer_id = {} AND edge.file_index = child.file_index AND edge.related_node = child.node",
                        step.producer
                    )
                } else {
                    format!(
                        "{ancestors} edge ON edge.file_index = child.file_index AND edge.descendant = child.node"
                    )
                };
                let parent_node = if step.stop == NavigationStop::Neighbor {
                    "edge.node"
                } else {
                    "edge.ancestor"
                };
                let columns = qualified_row_columns_with_verdict("parent", "1");
                terms.push(format!(
                    "SELECT {} AS utility_component, {columns}, child.bindings, child.choice_order FROM {relation} child JOIN {parent_relation} JOIN terminal_match parent ON parent.producer_id = {} AND parent.file_index = child.file_index AND parent.node = {parent_node} JOIN producer_status parent_status ON parent_status.producer_id = parent.producer_id AND parent_status.file_index = parent.file_index AND parent_status.complete = 1 WHERE child.utility_component = {}",
                    transition.caller,
                    spec.universe_producer,
                    transition.target
                ));
                continue;
            }
            let stops = transition
                .stops
                .iter()
                .enumerate()
                .map(|(ordinal, (level, stop))| {
                    let [root] = stop.roots.as_slice() else {
                        return Err(
                            "a recursive utility stop must have exactly one root".to_owned()
                        );
                    };
                    Ok((*level, format!("stop{ordinal}"), self.render(root)?))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let child_bindings = Self::recursive_child_bindings(transition);
            let child_compatibility = |alias: &str| {
                Self::binding_compatibility("child", alias)
                    .replace("child.bindings", &child_bindings)
            };
            let guards = transition
                .guards
                .iter()
                .enumerate()
                .map(|(ordinal, (level, guard))| {
                    let [root] = guard.roots.as_slice() else {
                        return Err(
                            "a recursive utility guard must have exactly one root".to_owned()
                        );
                    };
                    Ok((*level, format!("guard{ordinal}"), self.render(root)?))
                })
                .collect::<Result<Vec<_>, String>>()?;
            let mut joins = String::new();
            let mut stop_predicates = String::new();
            let mut node = "child.node".to_owned();
            let mut binding_aliases = vec!["child".to_owned()];
            for (level, alias, guard) in guards.iter().filter(|(level, _, _)| *level == 0) {
                let _ = level;
                joins.push_str(&format!(
                    " JOIN {guard} {alias} ON {alias}.file_index = child.file_index AND {alias}.node = {node} AND {}",
                    child_compatibility(alias)
                ));
                binding_aliases.push(alias.clone());
            }
            for (ordinal, step) in transition.steps.iter().enumerate() {
                let edge = format!("edge{ordinal}");
                match (step.axis, step.stop) {
                    (NavigationAxis::Descendant, NavigationStop::Neighbor) => {
                        let field = step
                            .field
                            .map_or("1".to_owned(), |field| format!("{edge}.field_id = {field}"));
                        joins.push_str(&format!(
                            " JOIN navigation {edge} ON {edge}.producer_id = {} AND {edge}.file_index = child.file_index AND {edge}.related_node = {node} AND {field}",
                            step.producer
                        ));
                        node = format!("{edge}.node");
                    }
                    (NavigationAxis::Descendant, NavigationStop::End | NavigationStop::Rule)
                        if step.field.is_none() =>
                    {
                        joins.push_str(&format!(
                            " JOIN {ancestors} {edge} ON {edge}.file_index = child.file_index AND {edge}.descendant = {node}"
                        ));
                        node = format!("{edge}.ancestor");
                    }
                    (NavigationAxis::Ancestor, NavigationStop::Neighbor) => {
                        let field = step
                            .field
                            .map_or("1".to_owned(), |field| format!("{edge}.field_id = {field}"));
                        joins.push_str(&format!(
                            " JOIN navigation {edge} ON {edge}.producer_id = {} AND {edge}.file_index = child.file_index AND {edge}.node = {node} AND {field}",
                            step.producer
                        ));
                        node = format!("{edge}.related_node");
                    }
                    (NavigationAxis::Ancestor, NavigationStop::End | NavigationStop::Rule)
                        if step.field.is_none() =>
                    {
                        joins.push_str(&format!(
                            " JOIN {ancestors} {edge} ON {edge}.file_index = child.file_index AND {edge}.ancestor = {node}"
                        ));
                        node = format!("{edge}.descendant");
                    }
                    (NavigationAxis::FollowingSibling | NavigationAxis::PrecedingSibling, stop) => {
                        let origin = format!("{edge}_origin");
                        let comparison = if step.axis == NavigationAxis::FollowingSibling {
                            "<"
                        } else {
                            ">"
                        };
                        let neighbor = if stop == NavigationStop::Neighbor {
                            " AND abs(candidate.axis_order - origin.axis_order) = 1"
                        } else {
                            ""
                        };
                        joins.push_str(&format!(
                            " JOIN navigation {origin} ON {origin}.producer_id = {} AND {origin}.file_index = child.file_index AND {origin}.related_node = {node} JOIN navigation {edge} ON {edge}.producer_id = {} AND {edge}.file_index = {origin}.file_index AND {edge}.node = {origin}.node AND {edge}.axis_order {comparison} {origin}.axis_order{neighbor}",
                            step.producer, step.producer
                        ));
                        node = format!("{edge}.related_node");
                    }
                    _ => {
                        return Err(
                            "a recursive utility path has an unsupported field or stop".to_owned()
                        );
                    }
                }
                for (_, alias, stop) in stops.iter().filter(|(level, _, _)| *level == ordinal) {
                    match step.axis {
                        NavigationAxis::Descendant | NavigationAxis::Ancestor => {
                            stop_predicates.push_str(&format!(
                                " AND NOT EXISTS (SELECT 1 FROM {ancestors} from_parent JOIN {ancestors} to_child ON to_child.file_index = from_parent.file_index AND to_child.ancestor = from_parent.descendant JOIN {stop} {alias} ON {alias}.file_index = from_parent.file_index AND {alias}.node = from_parent.descendant WHERE from_parent.file_index = child.file_index AND from_parent.ancestor = {edge}.ancestor AND to_child.descendant = {edge}.descendant)"
                            ));
                        }
                        NavigationAxis::FollowingSibling | NavigationAxis::PrecedingSibling => {
                            stop_predicates.push_str(&format!(
                                " AND NOT EXISTS (SELECT 1 FROM navigation between_edge JOIN {stop} {alias} ON {alias}.file_index = between_edge.file_index AND {alias}.node = between_edge.related_node WHERE between_edge.producer_id = {} AND between_edge.file_index = child.file_index AND between_edge.node = {edge}.node AND between_edge.axis_order > min({edge}.axis_order, {edge}_origin.axis_order) AND between_edge.axis_order < max({edge}.axis_order, {edge}_origin.axis_order))",
                                step.producer
                            ));
                        }
                    }
                }
                for (_, alias, guard) in guards.iter().filter(|(level, _, _)| *level == ordinal + 1)
                {
                    let compatibility = binding_aliases
                        .iter()
                        .map(|bound| {
                            let compatibility = Self::binding_compatibility(bound, alias);
                            if bound == "child" {
                                compatibility.replace("child.bindings", &child_bindings)
                            } else {
                                compatibility
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(" AND ");
                    joins.push_str(&format!(
                        " JOIN {guard} {alias} ON {alias}.file_index = child.file_index AND {alias}.node = {node} AND {compatibility}"
                    ));
                    binding_aliases.push(alias.clone());
                }
            }
            let columns = qualified_row_columns_with_overrides(
                "parent",
                &[
                    ("node", &node),
                    ("related_node", "parent.node"),
                    ("verdict", "1"),
                ],
            );
            let bindings =
                Self::merged_bindings(&binding_aliases).replace("child.bindings", &child_bindings);
            terms.push(format!(
                "SELECT {} AS utility_component, {columns}, {bindings} AS bindings, child.choice_order FROM {relation} child{joins} JOIN terminal_match parent ON parent.producer_id = {} AND parent.file_index = child.file_index AND parent.node = {node} JOIN producer_status parent_status ON parent_status.producer_id = parent.producer_id AND parent_status.file_index = parent.file_index AND parent_status.complete = 1 WHERE child.utility_component = {}{stop_predicates}",
                transition.caller,
                spec.universe_producer,
                transition.target
            ));
        }
        self.ctes.push(format!(
            "{relation} AS (\n  {}\n)",
            terms.join("\n  UNION\n  ")
        ));

        let walk = self.next_name(&format!("{family}_walk"));
        let mut walk_terms = vec![format!(
            "SELECT {} AS utility_component, root.file_index, root.node, 1 AS depth FROM terminal_match root JOIN producer_status status ON status.producer_id = root.producer_id AND status.file_index = root.file_index AND status.complete = 1 WHERE root.producer_id = {}",
            spec.root, spec.universe_producer
        )];
        for transition in &spec.transitions {
            walk_terms.push(format!(
                "SELECT {} AS utility_component, caller.file_index, edge.related_node AS node, caller.depth + 1 AS depth FROM {walk} caller JOIN navigation edge ON edge.producer_id = {} AND edge.file_index = caller.file_index AND edge.node = caller.node WHERE caller.utility_component = {} AND caller.depth <= {}",
                transition.target,
                spec.navigation_producer,
                transition.caller,
                spec.max_depth
            ));
        }
        self.ctes.push(format!(
            "{walk}(utility_component, file_index, node, depth) AS (\n  {}\n)",
            walk_terms.join("\n  UNION ALL\n  ")
        ));
        let summary = self.next_name(&format!("{family}_limits"));
        self.ctes.push(format!(
            "{summary} AS (\n  SELECT file_index, CASE WHEN max(depth) > {} THEN 'utility-depth-limit' WHEN count(*) > {} THEN 'utility-work-limit' END AS error_code, count(*) AS work_used FROM {walk} GROUP BY file_index HAVING max(depth) > {} OR count(*) > {}\n)",
            spec.max_depth, spec.max_work, spec.max_depth, spec.max_work
        ));
        let errors = self.next_name(&format!("{family}_errors"));
        let error_columns = qualified_row_columns_with_overrides(
            "root",
            &[
                ("row_kind", "8"),
                ("verdict", "NULL"),
                ("complete", "0"),
                ("work_used", "limited.work_used"),
                ("error_code", "limited.error_code"),
            ],
        );
        self.ctes.push(format!(
            "{errors} AS (\n  SELECT {error_columns}, json('[]') AS bindings, '' AS choice_order FROM {summary} limited JOIN terminal_match root ON root.producer_id = {} AND root.file_index = limited.file_index AND root.node = (SELECT min(candidate.node) FROM terminal_match candidate WHERE candidate.producer_id = {} AND candidate.file_index = limited.file_index)\n)",
            spec.universe_producer, spec.universe_producer
        ));
        self.errors.push(errors.clone());
        let result = self.next_name(&family);
        if spec.complement {
            let root_columns = qualified_row_columns("root");
            self.ctes.push(format!(
                "{result} AS (\n  SELECT {root_columns}, json('[]') AS bindings, '' AS choice_order FROM terminal_match root JOIN producer_status status ON status.producer_id = root.producer_id AND status.file_index = root.file_index AND status.complete = 1 WHERE root.producer_id = {} AND NOT EXISTS (SELECT 1 FROM {relation} failed_match WHERE failed_match.utility_component = {} AND failed_match.file_index = root.file_index AND failed_match.node = root.node) AND NOT EXISTS (SELECT 1 FROM {errors} failed WHERE failed.file_index = root.file_index)\n)",
                spec.universe_producer, spec.root
            ));
        } else {
            let root_columns = qualified_row_columns("matched");
            self.ctes.push(format!(
                "{result} AS (\n  SELECT {root_columns}, matched.bindings, matched.choice_order FROM {relation} matched WHERE matched.utility_component = {} AND NOT EXISTS (SELECT 1 FROM {errors} failed WHERE failed.file_index = matched.file_index)\n)",
                spec.root
            ));
        }
        self.utilities
            .recursive
            .insert(spec.component.clone(), (result.clone(), errors));
        self.apply_recursive_utility(source, result, &family)
    }

    fn render_neighbor_navigation(
        &mut self,
        spec: NavigationSpec,
        source: String,
        related: &RelationNode,
    ) -> Result<String, String> {
        let name = self.next_name("relation");
        let context = format!("{name}_context");
        let (candidate, joins, eligible) = match spec.axis {
            NavigationAxis::Descendant => (
                "edge.related_node",
                format!(
                    "JOIN navigation edge ON edge.producer_id = {} AND edge.file_index = source.file_index AND edge.node = source.node",
                    spec.producer
                ),
                spec.field
                    .map_or("1".to_owned(), |field| format!("edge.field_id = {field}")),
            ),
            NavigationAxis::Ancestor => (
                "edge.node",
                format!(
                    "JOIN navigation edge ON edge.producer_id = {} AND edge.file_index = source.file_index AND edge.related_node = source.node",
                    spec.producer
                ),
                spec.field
                    .map_or("1".to_owned(), |field| format!("edge.field_id = {field}")),
            ),
            NavigationAxis::FollowingSibling | NavigationAxis::PrecedingSibling => {
                let forward = spec.axis == NavigationAxis::FollowingSibling;
                let offset = if forward { "+ 1" } else { "- 1" };
                (
                    "edge.related_node",
                    format!(
                        "JOIN navigation origin ON origin.producer_id = {producer} AND origin.file_index = source.file_index AND origin.related_node = source.node\n  JOIN navigation edge ON edge.producer_id = {producer} AND edge.file_index = origin.file_index AND edge.node = origin.node AND edge.axis_order = origin.axis_order {offset}",
                        producer = spec.producer
                    ),
                    "1".to_owned(),
                )
            }
        };
        let columns = qualified_row_columns_with_overrides(
            "source",
            &[
                ("node", candidate),
                ("related_node", "source.node"),
                ("axis_order", "edge.axis_order"),
            ],
        );
        self.ctes.push(format!(
            "{context} AS (\n  SELECT {columns}, source.bindings, source.choice_order FROM {source} source\n  {joins}\n  WHERE {eligible}\n)"
        ));
        let related = related
            .clone()
            .apply_after(RelationNode::Rendered(context.clone()));
        let related = self.render(&related)?;
        let accepted_query = format!(
            "SELECT walk.file_index, walk.related_node AS source_node, related.bindings, related.choice_order, row_number() OVER (PARTITION BY walk.file_index, walk.related_node, walk.bindings ORDER BY walk.axis_order, walk.node, related.choice_order, related.producer_id, related.match_index) AS accepted_order\n  FROM {context} walk\n  JOIN {related} related ON related.file_index = walk.file_index AND related.node = walk.node AND related.related_node = walk.related_node\n    AND NOT EXISTS (SELECT 1 FROM json_each(walk.bindings) required WHERE NOT EXISTS (SELECT 1 FROM json_each(related.bindings) actual WHERE actual.value = required.value))"
        );
        let columns = qualified_row_columns("source");
        let first = if spec.enumerate {
            String::new()
        } else {
            " AND accepted.accepted_order = 1".to_owned()
        };
        self.ctes.push(format!(
            "{name} AS (\n  SELECT {columns}, accepted.bindings, accepted.choice_order FROM {source} source\n  JOIN (\n  {accepted_query}\n  ) accepted ON accepted.file_index = source.file_index AND accepted.source_node = source.node{first}\n)"
        ));
        Ok(name)
    }

    fn render_navigation(
        &mut self,
        spec: NavigationSpec,
        source: &RelationNode,
        related: &RelationNode,
        stop: Option<&RelationNode>,
    ) -> Result<String, String> {
        let source = self.render(source)?;
        if self.clause == usize::MAX && spec.stop == NavigationStop::Neighbor {
            return self.render_neighbor_navigation(spec, source, related);
        }
        let name = self.next_name("relation");
        let walk = format!("{name}_walk");
        let carry_source = self.clause != usize::MAX;
        let carried_names = ROW_COLUMNS
            .split(", ")
            .map(|column| format!("source_{column}"))
            .chain(["source_choice_order".to_owned()])
            .collect::<Vec<_>>();
        let carry_declaration = if carry_source {
            format!(", {}", carried_names.join(", "))
        } else {
            String::new()
        };
        let carry_seed = if carry_source {
            format!(
                ", {}",
                ROW_COLUMNS
                    .split(", ")
                    .map(|column| format!("source.{column}"))
                    .chain(["source.choice_order".to_owned()])
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            String::new()
        };
        let carry_step = if carry_source {
            format!(
                ", {}",
                carried_names
                    .iter()
                    .map(|column| format!("walk.{column}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            String::new()
        };
        let field = spec
            .field
            .map_or("1".to_owned(), |field| format!("edge.field_id = {field}"));
        match spec.axis {
            NavigationAxis::Descendant | NavigationAxis::Ancestor => {
                let (seed_join, step_join, candidate) = if spec.axis == NavigationAxis::Descendant {
                    (
                        "edge.node = source.node",
                        "edge.node = walk.candidate",
                        "edge.related_node",
                    )
                } else {
                    (
                        "edge.related_node = source.node",
                        "edge.related_node = walk.candidate",
                        "edge.node",
                    )
                };
                let recursive_field =
                    if spec.axis == NavigationAxis::Descendant && spec.field.is_some() {
                        "1".to_owned()
                    } else {
                        field.clone()
                    };
                let recurse = if spec.stop == NavigationStop::Neighbor {
                    String::new()
                } else {
                    let guard =
                        if spec.field_stop_children_only && spec.stop == NavigationStop::Rule {
                            "\n  WHERE walk.axis_order = 1"
                        } else {
                            ""
                        };
                    format!(
                        "\n  UNION ALL\n  SELECT walk.file_index, walk.source_node, {candidate}, walk.axis_order + 1, {recursive_field}, walk.path || printf('/%d', {candidate}), walk.source_bindings{carry_step}\n  FROM {walk} walk\n  JOIN navigation edge ON edge.producer_id = {} AND edge.file_index = walk.file_index AND {step_join}{guard}",
                        spec.producer
                    )
                };
                self.ctes.push(format!(
                    "{walk}(file_index, source_node, candidate, axis_order, eligible, path, source_bindings{carry_declaration}) AS (\n  SELECT source.file_index, source.node, {candidate}, 1, {field}, printf('/%d', {candidate}), source.bindings{carry_seed}\n  FROM {source} source\n  JOIN navigation edge ON edge.producer_id = {} AND edge.file_index = source.file_index AND {seed_join}{recurse}\n)",
                    spec.producer
                ));
            }
            NavigationAxis::FollowingSibling | NavigationAxis::PrecedingSibling => {
                let forward = spec.axis == NavigationAxis::FollowingSibling;
                let comparison = if forward { ">" } else { "<" };
                let neighbor = if spec.stop == NavigationStop::Neighbor {
                    if forward {
                        "AND candidate.axis_order = origin.axis_order + 1"
                    } else {
                        "AND candidate.axis_order = origin.axis_order - 1"
                    }
                } else {
                    ""
                };
                self.ctes.push(format!(
                    "{walk}(file_index, source_node, candidate, axis_order, eligible, path, source_bindings{carry_declaration}) AS (\n  SELECT source.file_index, source.node, candidate.related_node, abs(candidate.axis_order - origin.axis_order), 1, printf('/%d', candidate.related_node), source.bindings{carry_seed}\n  FROM {source} source\n  JOIN navigation origin ON origin.producer_id = {} AND origin.file_index = source.file_index AND origin.related_node = source.node\n  JOIN navigation candidate ON candidate.producer_id = {} AND candidate.file_index = origin.file_index AND candidate.node = origin.node\n  WHERE candidate.axis_order {comparison} origin.axis_order {neighbor}\n)",
                    spec.producer, spec.producer
                ));
            }
        }
        let context_name = format!("{name}_context");
        let context_query = if carry_source {
            let context_columns = ROW_COLUMNS
                .split(", ")
                .map(|column| match column {
                    "node" => "walk.candidate AS node".to_owned(),
                    "related_node" => "walk.source_node AS related_node".to_owned(),
                    _ => format!("walk.source_{column} AS {column}"),
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "SELECT {context_columns}, walk.source_bindings AS bindings, walk.source_choice_order AS choice_order\n  FROM {walk} walk"
            )
        } else {
            let context_columns = qualified_row_columns_with_node("source", "walk.candidate");
            format!(
                "SELECT {context_columns}, source.bindings, source.choice_order\n  FROM {walk} walk\n  JOIN {source} source ON source.file_index = walk.file_index AND source.node = walk.source_node"
            )
        };
        let context = if self.clause == usize::MAX && stop.is_none() {
            format!("({context_query})")
        } else {
            self.ctes
                .push(format!("{context_name} AS (\n  {context_query}\n)"));
            context_name
        };
        let related = related
            .clone()
            .apply_after(RelationNode::Rendered(context.clone()));
        let related = self.render(&related)?;
        let stop = stop
            .map(|stop| {
                self.render(
                    &stop
                        .clone()
                        .apply_after(RelationNode::Rendered(context.clone())),
                )
            })
            .transpose()?;
        let candidate_stop_limit = stop.as_ref().map_or(String::new(), |stop| match spec.axis {
            NavigationAxis::Descendant => format!(
                "\n    AND NOT EXISTS (SELECT 1 FROM {walk} earlier JOIN {stop} stopped ON stopped.file_index = earlier.file_index AND stopped.node = earlier.candidate AND stopped.related_node = earlier.source_node WHERE earlier.file_index = walk.file_index AND earlier.source_node = walk.source_node AND earlier.path <> walk.path AND substr(walk.path, 1, length(earlier.path) + 1) = earlier.path || '/' AND {})",
                Self::binding_compatibility("related", "stopped")
            ),
            NavigationAxis::Ancestor => format!(
                "\n    AND NOT EXISTS (SELECT 1 FROM {walk} earlier JOIN {stop} stopped ON stopped.file_index = earlier.file_index AND stopped.node = earlier.candidate AND stopped.related_node = earlier.source_node WHERE earlier.file_index = walk.file_index AND earlier.source_node = walk.source_node AND earlier.axis_order < walk.axis_order AND {})",
                Self::binding_compatibility("related", "stopped")
            ),
            NavigationAxis::FollowingSibling | NavigationAxis::PrecedingSibling => {
                let between = "earlier.axis_order < walk.axis_order";
                format!(
                    "\n    AND NOT EXISTS (SELECT 1 FROM {walk} earlier JOIN {stop} stopped ON stopped.file_index = earlier.file_index AND stopped.node = earlier.candidate AND stopped.related_node = earlier.source_node WHERE earlier.file_index = walk.file_index AND earlier.source_node = walk.source_node AND {between} AND {})",
                    Self::binding_compatibility("related", "stopped")
                )
            }
        });
        let order = if spec.axis == NavigationAxis::Descendant {
            "walk.candidate ASC"
        } else {
            "walk.axis_order ASC"
        };
        let accepted_carry = if carry_source {
            format!(
                ", {}",
                carried_names
                    .iter()
                    .map(|column| format!("walk.{column}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else {
            String::new()
        };
        let accepted_query = format!(
            "SELECT walk.file_index, walk.source_node, related.bindings, related.choice_order, row_number() OVER (PARTITION BY walk.file_index, walk.source_node, walk.source_bindings ORDER BY {order}, walk.candidate, related.choice_order, related.producer_id, related.match_index) AS accepted_order{accepted_carry}\n  FROM {walk} walk\n  JOIN {related} related ON related.file_index = walk.file_index AND related.node = walk.candidate AND related.related_node = walk.source_node\n  WHERE walk.eligible{candidate_stop_limit}"
        );
        let columns = qualified_row_columns("source");
        let first = if spec.enumerate {
            String::new()
        } else {
            " AND accepted.accepted_order = 1".to_owned()
        };
        if self.clause == usize::MAX {
            self.ctes.push(format!(
                "{name} AS (\n  SELECT {columns}, accepted.bindings, accepted.choice_order FROM {source} source\n  JOIN (\n  {accepted_query}\n  ) accepted ON accepted.file_index = source.file_index AND accepted.source_node = source.node{first}\n)"
            ));
        } else {
            let accepted = format!("{name}_accepted");
            self.ctes
                .push(format!("{accepted} AS (\n  {accepted_query}\n)"));
            let carried_columns = ROW_COLUMNS
                .split(", ")
                .map(|column| format!("accepted.source_{column} AS {column}"))
                .collect::<Vec<_>>()
                .join(", ");
            self.ctes.push(format!(
                "{name} AS (\n  SELECT {carried_columns}, accepted.bindings, accepted.choice_order FROM {accepted} accepted WHERE 1 = 1{first}\n)"
            ));
        }
        Ok(name)
    }

    fn render_constraints(
        &mut self,
        source: &RelationNode,
        constraints: &[(String, RelationNode)],
    ) -> Result<String, String> {
        if let Some(inputs) = Self::semantic_join(source)
            && let [terminal, RelationNode::Semantic { spec }] = inputs
            && let Some(source_producer) = Self::terminal_producer(terminal)?
            && spec.rules.is_empty()
            && let [(capture, constraint)] = constraints
            && spec.argument_labels.len() == 1
            && spec.argument_labels[0] == *capture
            && let Some(target_producer) = Self::terminal_producer(constraint)?
        {
            return self.render_argument_target(
                source_producer,
                spec.producer,
                target_producer,
                15,
                &spec.argument_labels,
            );
        }
        let mut current = self.render(source)?;
        for (capture, constraint) in constraints {
            let constraint = self.render(constraint)?;
            let capture = Self::sql_text(capture);
            let name = self.next_name("constraint");
            let columns = qualified_row_columns("source");
            let merged = Self::merged_bindings(&["source".to_owned(), "checked".to_owned()]);
            let compatible = Self::binding_compatibility("source", "checked");
            self.ctes.push(format!(
                "{name} AS (\n  SELECT {columns}, {merged} AS bindings, source.choice_order || checked.choice_order AS choice_order\n  FROM {current} source\n  JOIN json_each(source.bindings) bound\n    ON json_extract(bound.value, '$.label') = {capture} AND json_extract(bound.value, '$.kind') = 1 AND json_extract(bound.value, '$.occurrence') = 0\n  JOIN {constraint} checked\n    ON checked.file_index = source.file_index AND checked.node = json_extract(bound.value, '$.node')\n    AND {compatible}\n  UNION ALL\n  SELECT {columns}, source.bindings, source.choice_order\n  FROM {current} source\n  WHERE NOT EXISTS (SELECT 1 FROM json_each(source.bindings) bound WHERE json_extract(bound.value, '$.label') = {capture} AND json_extract(bound.value, '$.kind') = 1)\n)"
            ));
            current = name;
        }
        Ok(current)
    }

    fn render_count(
        &mut self,
        spec: &CountSpec,
        source: &RelationNode,
        related: &RelationNode,
        stop: Option<&RelationNode>,
    ) -> Result<String, String> {
        let source = self.render(source)?;
        let name = self.next_name("count");
        let candidates = format!("{name}_candidates");
        let producer = spec.navigation.producer;
        match spec.navigation.axis {
            NavigationAxis::Descendant | NavigationAxis::Ancestor => {
                let descendant = spec.navigation.axis == NavigationAxis::Descendant;
                let seed_join = if descendant {
                    "edge.node = source.node"
                } else {
                    "edge.related_node = source.node"
                };
                let step_join = if descendant {
                    "edge.node = walk.candidate"
                } else {
                    "edge.related_node = walk.candidate"
                };
                let candidate = if descendant {
                    "edge.related_node"
                } else {
                    "edge.node"
                };
                let recurse = if spec.navigation.stop == NavigationStop::Neighbor {
                    String::new()
                } else {
                    format!(
                        "\n  UNION ALL\n  SELECT walk.file_index, walk.source_node, {candidate}, walk.axis_order + 1, walk.path || printf('/%d', {candidate})\n  FROM {candidates} walk\n  JOIN navigation edge ON edge.producer_id = {producer} AND edge.file_index = walk.file_index AND {step_join}"
                    )
                };
                self.ctes.push(format!(
                    "{candidates}(file_index, source_node, candidate, axis_order, path) AS (\n  SELECT source.file_index, source.node, {candidate}, 1, printf('/%d', {candidate})\n  FROM {source} source\n  JOIN navigation edge ON edge.producer_id = {producer} AND edge.file_index = source.file_index AND {seed_join}{recurse}\n)"
                ));
            }
            NavigationAxis::FollowingSibling | NavigationAxis::PrecedingSibling => {
                let forward = spec.navigation.axis == NavigationAxis::FollowingSibling;
                let comparison = if forward { ">" } else { "<" };
                let neighbor = if spec.navigation.stop == NavigationStop::Neighbor {
                    if forward {
                        "AND candidate.axis_order = origin.axis_order + 1"
                    } else {
                        "AND candidate.axis_order = origin.axis_order - 1"
                    }
                } else {
                    ""
                };
                self.ctes.push(format!(
                    "{candidates} AS (\n  SELECT source.file_index, source.node AS source_node, candidate.related_node AS candidate, abs(candidate.axis_order - origin.axis_order) AS axis_order, printf('/%d', candidate.related_node) AS path\n  FROM {source} source\n  JOIN navigation origin ON origin.producer_id = {producer} AND origin.file_index = source.file_index AND origin.related_node = source.node\n  JOIN navigation candidate ON candidate.producer_id = {producer} AND candidate.file_index = origin.file_index AND candidate.node = origin.node\n  WHERE candidate.axis_order {comparison} origin.axis_order {neighbor}\n)"
                ));
            }
        }
        let context = format!("{name}_context");
        let context_columns = qualified_row_columns_with_node("source", "candidate.candidate");
        self.ctes.push(format!(
            "{context} AS (\n  SELECT {context_columns}, source.bindings, source.choice_order\n  FROM {candidates} candidate\n  JOIN {source} source ON source.file_index = candidate.file_index AND source.node = candidate.source_node\n)"
        ));
        let related = related
            .clone()
            .apply_after(RelationNode::Rendered(context.clone()));
        let related = self.render(&related)?;
        let stop = stop
            .map(|stop| {
                self.render(
                    &stop
                        .clone()
                        .apply_after(RelationNode::Rendered(context.clone())),
                )
            })
            .transpose()?;
        let stop_limit = stop.as_ref().map_or(String::new(), |stop| match spec.navigation.axis {
            NavigationAxis::Descendant => format!(
                " AND NOT EXISTS (SELECT 1 FROM {candidates} earlier JOIN {stop} stopped ON stopped.file_index = earlier.file_index AND stopped.node = earlier.candidate AND stopped.related_node = earlier.source_node WHERE earlier.file_index = candidate.file_index AND earlier.source_node = candidate.source_node AND earlier.path <> candidate.path AND substr(candidate.path, 1, length(earlier.path) + 1) = earlier.path || '/' AND {})",
                Self::binding_compatibility("source", "stopped")
            ),
            NavigationAxis::FollowingSibling | NavigationAxis::PrecedingSibling => {
                let comparison = if spec.navigation.axis == NavigationAxis::FollowingSibling {
                    "<"
                } else {
                    ">"
                };
                format!(
                    " AND NOT EXISTS (SELECT 1 FROM {candidates} earlier JOIN {stop} stopped ON stopped.file_index = earlier.file_index AND stopped.node = earlier.candidate AND stopped.related_node = earlier.source_node WHERE earlier.file_index = candidate.file_index AND earlier.source_node = candidate.source_node AND earlier.candidate {comparison} candidate.candidate AND {})",
                    Self::binding_compatibility("source", "stopped")
                )
            }
            NavigationAxis::Ancestor => format!(
                " AND NOT EXISTS (SELECT 1 FROM {candidates} earlier JOIN {stop} stopped ON stopped.file_index = earlier.file_index AND stopped.node = earlier.candidate AND stopped.related_node = earlier.source_node WHERE earlier.file_index = candidate.file_index AND earlier.source_node = candidate.source_node AND earlier.axis_order < candidate.axis_order AND {})",
                Self::binding_compatibility("source", "stopped")
            ),
        });
        let counts = format!("{name}_values");
        self.ctes.push(format!(
            "{counts} AS (\n  SELECT source.*, (\n    SELECT count(DISTINCT candidate.candidate) FROM {candidates} candidate\n    JOIN {related} related ON related.file_index = candidate.file_index AND related.node = candidate.candidate AND related.related_node = source.node AND {}\n    WHERE candidate.file_index = source.file_index AND candidate.source_node = source.node{stop_limit}\n  ) AS found\n  FROM {source} source\n)",
            Self::binding_compatibility("source", "related")
        ));
        let maximum = spec.maximum.map_or(String::new(), |maximum| {
            format!(" AND counted.found <= {maximum}")
        });
        let columns = qualified_row_columns("counted");
        let bindings = spec.binding.as_ref().map_or_else(
            || "counted.bindings".to_owned(),
            |(binding, offset)| {
                let binding = Self::sql_text(binding);
                format!(
                    "COALESCE((SELECT json_group_array(json(value)) FROM (SELECT value FROM json_each(counted.bindings) UNION ALL SELECT json_object('label', {binding}, 'occurrence', 0, 'node', NULL, 'text', CAST(counted.found + ({offset}) AS TEXT), 'structure', NULL, 'leaf', 0, 'kind', 1, 'named', 0, 'equality_occurrence', 0))), json('[]'))"
                )
            },
        );
        self.ctes.push(format!(
            "{name} AS (\n  SELECT {columns}, {bindings} AS bindings, counted.choice_order FROM {counts} counted\n  WHERE counted.found >= {}{maximum}\n)",
            spec.minimum
        ));
        Ok(name)
    }

    fn render_nth_child(
        &mut self,
        spec: NthChildSpec,
        source: &RelationNode,
        of_rule: Option<&RelationNode>,
    ) -> Result<String, String> {
        let source = self.render(source)?;
        let name = self.next_name("relation");
        let direction = if spec.reverse { ">=" } else { "<=" };
        let position = if spec.step == 0 {
            format!("position = {}", spec.offset)
        } else if spec.step > 0 {
            format!(
                "position - {} >= 0 AND (position - {}) % {} = 0",
                spec.offset, spec.offset, spec.step
            )
        } else {
            format!(
                "position - {} <= 0 AND (position - {}) % {} = 0",
                spec.offset, spec.offset, spec.step
            )
        };
        let (counted_join, source_join, output_bindings, output_choice) = if let Some(of_rule) =
            of_rule
        {
            let siblings = format!("{name}_siblings");
            let columns = qualified_row_columns_with_node("source", "candidate.related_node");
            self.ctes.push(format!(
                "{siblings} AS (\n  SELECT {columns}, source.bindings, source.choice_order\n  FROM {source} source\n  JOIN navigation edge ON edge.producer_id = {} AND edge.file_index = source.file_index AND edge.related_node = source.node\n  JOIN navigation candidate ON candidate.producer_id = {} AND candidate.file_index = edge.file_index AND candidate.node = edge.node AND candidate.related_named = 1\n)",
                spec.producer, spec.producer
            ));
            let matches = of_rule
                .clone()
                .apply_after(RelationNode::Rendered(siblings));
            let matches = self.render(&matches)?;
            (
                format!(
                    "\n      JOIN {matches} counted_rule ON counted_rule.file_index = counted.file_index AND counted_rule.node = counted.related_node AND counted_rule.related_node = source.node AND {}",
                    Self::binding_compatibility("source", "counted_rule")
                ),
                format!(
                    "\n  JOIN {matches} source_rule ON source_rule.file_index = source.file_index AND source_rule.node = source.node AND source_rule.related_node = source.node AND {}",
                    Self::binding_compatibility("source", "source_rule")
                ),
                Self::merged_bindings(&["source".to_owned(), "source_rule".to_owned()]),
                "source.choice_order || source_rule.choice_order".to_owned(),
            )
        } else {
            (
                String::new(),
                String::new(),
                "source.bindings".to_owned(),
                "source.choice_order".to_owned(),
            )
        };
        self.ctes.push(format!(
            "{name}_positions AS (\n  SELECT source.file_index, source.node, source.bindings, {output_bindings} AS output_bindings, {output_choice} AS output_choice, (\n    SELECT count(DISTINCT counted.related_node) FROM navigation counted{counted_join}\n    WHERE counted.producer_id = {} AND counted.file_index = edge.file_index AND counted.node = edge.node\n      AND counted.related_named = 1 AND counted.axis_order {direction} edge.axis_order\n  ) AS position\n  FROM {source} source{source_join}\n  JOIN navigation edge ON edge.producer_id = {} AND edge.file_index = source.file_index AND edge.related_node = source.node\n  WHERE edge.related_named = 1\n),\n{name} AS (\n  SELECT {} , position.output_bindings AS bindings, position.output_choice AS choice_order FROM {source} source\n  JOIN {name}_positions position ON position.file_index = source.file_index AND position.node = source.node AND position.bindings = source.bindings\n  WHERE {position}\n)",
            spec.producer,
            spec.producer,
            qualified_row_columns("source")
        ));
        Ok(name)
    }

    fn peek_name(&self, family: &str) -> String {
        if self.clause == usize::MAX {
            format!("u{}", self.next_relation)
        } else {
            let family = self.utilities.compact_family(family);
            format!("clause_{}_{family}_{}", self.clause, self.next_relation)
        }
    }

    fn next_name(&mut self, family: &str) -> String {
        let name = self.peek_name(family);
        self.next_relation += 1;
        let family = self.utilities.compact_family(family);
        *self.relation_families.entry(family).or_default() += 1;
        name
    }
}

fn qualified_row_columns_with_verdict(alias: &str, verdict: &str) -> String {
    ROW_COLUMNS
        .split(", ")
        .map(|column| {
            if column == "verdict" {
                format!("{verdict} AS verdict")
            } else {
                format!("{alias}.{column}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn qualified_row_columns_with_node(alias: &str, node: &str) -> String {
    ROW_COLUMNS
        .split(", ")
        .map(|column| {
            if column == "node" {
                format!("{node} AS node")
            } else if column == "related_node" {
                format!("{alias}.node AS related_node")
            } else {
                format!("{alias}.{column}")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn qualified_row_columns(alias: &str) -> String {
    qualified_row_columns_with_overrides(alias, &[])
}

pub(super) fn qualified_failure_columns(alias: &str) -> String {
    qualified_row_columns_with_overrides(alias, &[("row_kind", "8")])
}

fn qualified_row_columns_with_overrides(alias: &str, overrides: &[(&str, &str)]) -> String {
    ROW_COLUMNS
        .split(", ")
        .map(|column| {
            overrides
                .iter()
                .find(|(name, _)| *name == column)
                .map_or_else(
                    || format!("{alias}.{column}"),
                    |(_, value)| format!("{value} AS {column}"),
                )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SharedRelationalIr {
    roots: Vec<Arc<RelationNode>>,
    terminal_kinds: Vec<TerminalKind>,
    operations: Vec<SqlOperation>,
}

impl SharedRelationalIr {
    pub(super) fn instantiate(&self) -> RelationalIr {
        RelationalIr {
            roots: self
                .roots
                .iter()
                .map(|root| RelationNode::Shared(Arc::clone(root)))
                .collect(),
            terminal_kinds: self.terminal_kinds.clone(),
            operations: self.operations.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct RelationalIr {
    roots: Vec<RelationNode>,
    terminal_kinds: Vec<TerminalKind>,
    operations: Vec<SqlOperation>,
}

impl Default for RelationalIr {
    fn default() -> Self {
        Self::empty()
    }
}

impl RelationalIr {
    pub(super) fn into_shared(self) -> SharedRelationalIr {
        SharedRelationalIr {
            roots: self.roots.into_iter().map(Arc::new).collect(),
            terminal_kinds: self.terminal_kinds,
            operations: self.operations,
        }
    }

    pub(super) fn empty() -> Self {
        Self {
            roots: Vec::new(),
            terminal_kinds: Vec::new(),
            operations: vec![SqlOperation::PackedMaterialized, SqlOperation::StatusJoin],
        }
    }

    pub(super) fn empty_relation() -> Self {
        let mut relation = Self {
            roots: vec![RelationNode::Empty],
            terminal_kinds: Vec::new(),
            operations: Vec::new(),
        };
        relation.refresh_explanation();
        relation
    }

    pub(super) fn comparison(spec: ComparisonSpec) -> Self {
        let mut relation = Self {
            roots: vec![RelationNode::Compare {
                spec,
                source: Box::new(RelationNode::Seed),
            }],
            terminal_kinds: Vec::new(),
            operations: Vec::new(),
        };
        relation.refresh_explanation();
        relation
    }

    pub(super) fn value(spec: ValuePredicateSpec) -> Self {
        let mut relation = Self {
            roots: vec![RelationNode::Value {
                spec,
                source: Box::new(RelationNode::Seed),
            }],
            terminal_kinds: Vec::new(),
            operations: Vec::new(),
        };
        relation.refresh_explanation();
        relation
    }

    pub(super) fn parsed(spec: ParseFactSpec) -> Self {
        Self {
            roots: vec![RelationNode::Parsed { spec }],
            terminal_kinds: vec![TerminalKind::Parse],
            operations: vec![
                SqlOperation::ScanTerminal,
                SqlOperation::ScanFact,
                SqlOperation::InnerJoin,
                SqlOperation::Bindings,
            ],
        }
    }

    pub(super) fn semantic(spec: SemanticFactSpec) -> Self {
        let mut relation = Self {
            roots: vec![RelationNode::Semantic { spec }],
            terminal_kinds: Vec::new(),
            operations: Vec::new(),
        };
        relation.refresh_explanation();
        relation
    }

    pub(super) fn utility(spec: UtilitySpec, body: Self) -> Result<Self, String> {
        let source = RelationNode::Apply {
            operation: SqlOperation::ScanTerminal,
            inputs: vec![RelationNode::Producer(TerminalSpec::new(
                TerminalKind::Universe,
                format!("terminal:{}", spec.universe_producer),
            ))],
        };
        let node = RelationNode::Utility {
            spec,
            source: Box::new(source),
            body: Box::new(body.one_root()?),
        };
        let mut relation = Self {
            roots: vec![node],
            terminal_kinds: Vec::new(),
            operations: Vec::new(),
        };
        relation.refresh_explanation();
        Ok(relation)
    }

    pub(super) fn utility_frame(spec: UtilityFrameSpec, body: Self) -> Result<Self, String> {
        let source = RelationNode::Apply {
            operation: SqlOperation::ScanTerminal,
            inputs: vec![RelationNode::Producer(TerminalSpec::new(
                TerminalKind::Universe,
                format!("terminal:{}", spec.utility.universe_producer),
            ))],
        };
        let node = RelationNode::UtilityFrame {
            spec,
            source: Box::new(source),
            body: Box::new(body.one_root()?),
        };
        let mut relation = Self {
            roots: vec![node],
            terminal_kinds: Vec::new(),
            operations: Vec::new(),
        };
        relation.refresh_explanation();
        Ok(relation)
    }

    pub(super) fn recursive_utility(
        spec: RecursiveUtilitySpec,
        bases: Vec<(usize, Self)>,
    ) -> Result<Self, String> {
        let bases = bases
            .into_iter()
            .map(|(component, base)| Ok((component, base.one_root()?)))
            .collect::<Result<Vec<_>, String>>()?;
        let node = RelationNode::RecursiveUtility {
            spec,
            source: Box::new(RelationNode::Seed),
            bases,
        };
        let mut relation = Self {
            roots: vec![node],
            terminal_kinds: Vec::new(),
            operations: Vec::new(),
        };
        relation.refresh_explanation();
        Ok(relation)
    }

    pub(super) fn union_all(relations: Vec<Self>) -> Result<Self, String> {
        let roots = relations
            .into_iter()
            .map(Self::one_root)
            .collect::<Result<Vec<_>, _>>()?;
        let mut relation = Self {
            roots: vec![RelationNode::MergeChoices(roots)],
            terminal_kinds: Vec::new(),
            operations: Vec::new(),
        };
        relation.refresh_explanation();
        Ok(relation)
    }

    pub(super) fn select_when(select: Self, exists: Self) -> Result<Self, String> {
        let mut relation = Self {
            roots: vec![RelationNode::SelectWhen {
                select: Box::new(select.one_root()?),
                exists: Box::new(exists.one_root()?),
            }],
            terminal_kinds: Vec::new(),
            operations: Vec::new(),
        };
        relation.refresh_explanation();
        Ok(relation)
    }

    pub(super) fn require_each(
        subject: Self,
        evidence: Self,
        keys: Vec<(String, String)>,
    ) -> Result<Self, String> {
        let mut relation = Self {
            roots: vec![RelationNode::RequireEach {
                subject: Box::new(subject.one_root()?),
                evidence: Box::new(evidence.one_root()?),
                keys,
            }],
            terminal_kinds: Vec::new(),
            operations: Vec::new(),
        };
        relation.refresh_explanation();
        Ok(relation)
    }

    pub(super) fn budget(
        source: Self,
        group_by: Vec<String>,
        distinct: Vec<String>,
        maximum: usize,
    ) -> Result<Self, String> {
        let mut relation = Self {
            roots: vec![RelationNode::Budget {
                source: Box::new(source.one_root()?),
                group_by,
                distinct,
                maximum,
            }],
            terminal_kinds: Vec::new(),
            operations: Vec::new(),
        };
        relation.refresh_explanation();
        Ok(relation)
    }

    pub(super) fn terminal_spec(
        &mut self,
        terminal: TerminalKind,
        semantic_key: impl Into<String>,
    ) {
        self.roots.push(RelationNode::Producer(TerminalSpec::new(
            terminal,
            semantic_key,
        )));
        if !self.terminal_kinds.contains(&terminal) {
            self.terminal_kinds.push(terminal);
        }
    }

    fn one_root(self) -> Result<RelationNode, String> {
        let [root] = <Vec<RelationNode> as TryInto<[RelationNode; 1]>>::try_into(self.roots)
            .map_err(|_| "a relational operand must have exactly one root".to_owned())?;
        Ok(root)
    }

    pub(super) fn navigate(
        spec: NavigationSpec,
        source: Self,
        related: Self,
        stop: Option<Self>,
    ) -> Result<Self, String> {
        let node = RelationNode::Navigate {
            spec,
            source: Box::new(source.one_root()?),
            related: Box::new(related.one_root()?),
            stop: stop.map(Self::one_root).transpose()?.map(Box::new),
        };
        let mut ir = Self {
            roots: vec![node],
            terminal_kinds: Vec::new(),
            operations: Vec::new(),
        };
        ir.refresh_explanation();
        Ok(ir)
    }

    pub(super) fn nth_child(
        spec: NthChildSpec,
        source: Self,
        of_rule: Option<Self>,
    ) -> Result<Self, String> {
        let node = RelationNode::NthChild {
            spec,
            source: Box::new(source.one_root()?),
            of_rule: of_rule.map(Self::one_root).transpose()?.map(Box::new),
        };
        let mut ir = Self {
            roots: vec![node],
            terminal_kinds: Vec::new(),
            operations: Vec::new(),
        };
        ir.refresh_explanation();
        Ok(ir)
    }

    pub(super) fn constrain(
        source: Self,
        constraints: Vec<(String, Self)>,
    ) -> Result<Self, String> {
        let constraints = constraints
            .into_iter()
            .map(|(name, constraint)| Ok((name, constraint.one_root()?)))
            .collect::<Result<Vec<_>, String>>()?;
        let node = RelationNode::Constrain {
            source: Box::new(source.one_root()?),
            constraints,
        };
        let mut ir = Self {
            roots: vec![node],
            terminal_kinds: Vec::new(),
            operations: Vec::new(),
        };
        ir.refresh_explanation();
        Ok(ir)
    }

    pub(super) fn count(
        spec: CountSpec,
        source: Self,
        related: Self,
        stop: Option<Self>,
    ) -> Result<Self, String> {
        let node = RelationNode::Count {
            spec,
            source: Box::new(source.one_root()?),
            related: Box::new(related.one_root()?),
            stop: stop.map(Self::one_root).transpose()?.map(Box::new),
        };
        let mut ir = Self {
            roots: vec![node],
            terminal_kinds: Vec::new(),
            operations: Vec::new(),
        };
        ir.refresh_explanation();
        Ok(ir)
    }

    pub(super) fn combine(operation: SqlOperation, parts: Vec<Self>) -> Result<Self, String> {
        if parts.is_empty() {
            return Err(format!(
                "{operation:?} requires at least one relational operand"
            ));
        }
        let mut inputs = Vec::with_capacity(parts.len());
        let mut terminal_kinds = Vec::new();
        let mut operations = Vec::new();
        for part in parts {
            if part.roots.len() != 1 {
                return Err("a relational operand must have exactly one root".to_owned());
            }
            for terminal in part.terminal_kinds {
                if !terminal_kinds.contains(&terminal) {
                    terminal_kinds.push(terminal);
                }
            }
            for nested in part.operations {
                if !operations.contains(&nested) {
                    operations.push(nested);
                }
            }
            inputs.extend(part.roots);
        }
        if !operations.contains(&operation) {
            operations.push(operation);
        }
        let root = if operation == SqlOperation::InnerJoin && inputs.len() == 1 {
            inputs
                .into_iter()
                .next()
                .expect("the nonempty operand check supplies one relation")
        } else if operation == SqlOperation::InnerJoin
            && inputs.iter().all(RelationNode::independent_of_input)
        {
            RelationNode::Apply { operation, inputs }
        } else if operation == SqlOperation::InnerJoin {
            let mut inputs = inputs.into_iter();
            let mut accumulated = inputs
                .next()
                .expect("the nonempty operand check supplies the first relation");
            for input in inputs {
                accumulated = input.apply_after(accumulated);
            }
            accumulated
        } else {
            RelationNode::Apply { operation, inputs }
        };
        Ok(Self {
            roots: vec![root],
            terminal_kinds,
            operations,
        })
    }

    pub(super) fn operation(&mut self, operation: SqlOperation) {
        assert!(
            !matches!(
                operation,
                SqlOperation::PackedMaterialized | SqlOperation::StatusJoin
            ),
            "packed scan and status join are statement prelude operations"
        );
        let inputs = if self.roots.is_empty() {
            vec![RelationNode::Seed]
        } else {
            std::mem::take(&mut self.roots)
        };
        self.roots.push(RelationNode::Apply { operation, inputs });
        if !self.operations.contains(&operation) {
            self.operations.push(operation);
        }
    }

    fn terminal_specs_with_shared_visits(&self) -> (BTreeSet<TerminalSpec>, usize) {
        fn insert(specs: &mut BTreeSet<TerminalSpec>, kind: TerminalKind, producer: usize) {
            specs.insert(TerminalSpec::new(kind, format!("terminal:{producer}")));
        }

        fn collect(
            node: &RelationNode,
            specs: &mut BTreeSet<TerminalSpec>,
            shared: &mut Vec<Arc<RelationNode>>,
        ) {
            match node {
                RelationNode::Shared(node) => {
                    if !shared.iter().any(|known| known.as_ref() == node.as_ref()) {
                        shared.push(Arc::clone(node));
                        collect(node, specs, shared);
                    }
                }
                RelationNode::Seed | RelationNode::Empty | RelationNode::Rendered(_) => {}
                RelationNode::Producer(spec) => {
                    specs.insert(spec.clone());
                }
                RelationNode::Apply { inputs, .. } | RelationNode::MergeChoices(inputs) => {
                    for input in inputs {
                        collect(input, specs, shared);
                    }
                }
                RelationNode::PrefixChoice { source, .. } | RelationNode::Budget { source, .. } => {
                    collect(source, specs, shared)
                }
                RelationNode::Compare { spec, source } => {
                    collect(source, specs, shared);
                    for producer in [spec.left_transform, spec.right_transform]
                        .into_iter()
                        .flatten()
                    {
                        insert(specs, TerminalKind::Transform, producer);
                    }
                }
                RelationNode::SelectWhen { select, exists } => {
                    collect(select, specs, shared);
                    collect(exists, specs, shared);
                }
                RelationNode::RequireEach {
                    subject, evidence, ..
                } => {
                    collect(subject, specs, shared);
                    collect(evidence, specs, shared);
                }
                RelationNode::Value { spec, source } => {
                    insert(specs, TerminalKind::Value, spec.producer);
                    collect(source, specs, shared);
                }
                RelationNode::Parsed { spec } => {
                    insert(specs, TerminalKind::Parse, spec.producer);
                    insert(specs, TerminalKind::Universe, spec.host_producer);
                    if let Some(producer) = spec.required_host_producer {
                        insert(specs, TerminalKind::Kind, producer);
                    }
                }
                RelationNode::Semantic { spec } => {
                    insert(specs, TerminalKind::Fact, spec.producer);
                    for relation in spec.rules.iter().filter_map(|rule| rule.relation.as_ref()) {
                        for root in &relation.roots {
                            collect(root, specs, shared);
                        }
                    }
                }
                RelationNode::Utility { source, body, .. }
                | RelationNode::UtilityFrame { source, body, .. } => {
                    collect(source, specs, shared);
                    collect(body, specs, shared);
                }
                RelationNode::RecursiveUtility {
                    spec,
                    source,
                    bases,
                } => {
                    insert(specs, TerminalKind::Universe, spec.universe_producer);
                    insert(specs, TerminalKind::Navigation, spec.navigation_producer);
                    collect(source, specs, shared);
                    for (_, base) in bases {
                        collect(base, specs, shared);
                    }
                    for transition in &spec.transitions {
                        for (_, relation) in transition.stops.iter().chain(&transition.guards) {
                            for root in &relation.roots {
                                collect(root, specs, shared);
                            }
                        }
                    }
                }
                RelationNode::Navigate {
                    spec,
                    source,
                    related,
                    stop,
                } => {
                    insert(specs, TerminalKind::Navigation, spec.producer);
                    collect(source, specs, shared);
                    collect(related, specs, shared);
                    if let Some(stop) = stop {
                        collect(stop, specs, shared);
                    }
                }
                RelationNode::NthChild {
                    spec,
                    source,
                    of_rule,
                } => {
                    insert(specs, TerminalKind::Navigation, spec.producer);
                    collect(source, specs, shared);
                    if let Some(rule) = of_rule {
                        collect(rule, specs, shared);
                    }
                }
                RelationNode::Constrain {
                    source,
                    constraints,
                } => {
                    collect(source, specs, shared);
                    for (_, rule) in constraints {
                        collect(rule, specs, shared);
                    }
                }
                RelationNode::Count {
                    spec,
                    source,
                    related,
                    stop,
                } => {
                    insert(specs, TerminalKind::Navigation, spec.navigation.producer);
                    collect(source, specs, shared);
                    collect(related, specs, shared);
                    if let Some(rule) = stop {
                        collect(rule, specs, shared);
                    }
                }
            }
        }

        let mut specs = BTreeSet::new();
        let mut shared = Vec::new();
        for root in &self.roots {
            collect(root, &mut specs, &mut shared);
        }
        (specs, shared.len())
    }

    pub(super) fn terminal_specs(&self) -> BTreeSet<TerminalSpec> {
        self.terminal_specs_with_shared_visits().0
    }

    pub(super) fn operations(&self) -> &[SqlOperation] {
        &self.operations
    }

    pub(super) fn utility_definitions(&self) -> BTreeSet<super::plan::UtilityDefinitionId> {
        let mut definitions = BTreeSet::new();
        for root in &self.roots {
            root.collect_utility_definitions(&mut definitions);
        }
        definitions
    }

    pub(super) fn requires_recursive_cte(&self) -> bool {
        self.operations.contains(&SqlOperation::RecursiveCte)
    }

    pub(super) fn render_clause_relations(
        &self,
        clause_ordinal: usize,
        each: Option<&str>,
        utilities: &mut UtilityCatalog,
        packetize: bool,
    ) -> Result<(Vec<String>, String, Vec<SqlMaterialization>), String> {
        if self.roots.len() != 1 {
            return Err("a clause must have exactly one relational root".to_owned());
        }
        let share_all = self.roots[0].expanded_size_exceeds(500);
        let mut renderer = SqlRenderer::new(clause_ordinal, share_all, utilities);
        renderer.packetize = packetize;
        let result = renderer.render(&self.roots[0])?;
        renderer.finish_utility_limits()?;
        let selected = renderer.next_name("selected");
        let errors = renderer
            .errors
            .iter()
            .map(|error| format!("SELECT * FROM {error}"))
            .collect::<Vec<_>>();
        let no_error = renderer
            .errors
            .iter()
            .map(|error| {
                format!(
                    "NOT EXISTS (SELECT 1 FROM {error} failed WHERE failed.file_index = candidate.file_index)"
                )
            })
            .collect::<Vec<_>>();
        let no_error = if no_error.is_empty() {
            String::new()
        } else {
            format!(" AND {}", no_error.join(" AND "))
        };
        let errors = if errors.is_empty() {
            String::new()
        } else {
            format!("\n  UNION ALL\n  {}", errors.join("\n  UNION ALL\n  "))
        };
        let same_output = each.map_or(String::new(), |name| {
            let name = SqlRenderer::sql_text(name);
            format!(
                " AND COALESCE((SELECT printf('%s:%s', json_extract(binding.value, '$.node'), json_extract(binding.value, '$.text')) FROM json_each(earlier.bindings) binding WHERE json_extract(binding.value, '$.label') = {name} AND json_extract(binding.value, '$.occurrence') = 0), '') = COALESCE((SELECT printf('%s:%s', json_extract(binding.value, '$.node'), json_extract(binding.value, '$.text')) FROM json_each(candidate.bindings) binding WHERE json_extract(binding.value, '$.label') = {name} AND json_extract(binding.value, '$.occurrence') = 0), '')"
            )
        });
        renderer.ctes.push(format!(
            "{selected} AS (\n  SELECT candidate.* FROM {result} candidate\n  WHERE candidate.verdict = 1{no_error} AND NOT EXISTS (\n    SELECT 1 FROM {result} earlier\n    WHERE earlier.file_index = candidate.file_index AND earlier.node = candidate.node{same_output}\n      AND (earlier.choice_order < candidate.choice_order\n        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id < candidate.producer_id)\n        OR (earlier.choice_order = candidate.choice_order AND earlier.producer_id = candidate.producer_id AND earlier.match_index < candidate.match_index))\n  ){errors}\n)"
        ));
        Ok((renderer.ctes, selected, renderer.materializations))
    }

    pub(super) fn render_utility_catalog(
        utilities: &mut UtilityCatalog,
        packetize: bool,
    ) -> Result<(Vec<String>, Vec<SqlMaterialization>), String> {
        const MAX_RENDERED_SQL_BYTES: usize = 8 * 1024 * 1024;
        let mut renderer = SqlRenderer::new(usize::MAX, true, utilities);
        renderer.packetize = packetize;
        renderer.finish_shared_utilities()?;
        let large_catalog = renderer.compact_base_relations;
        let base_relations = [
            "ps AS MATERIALIZED(SELECT*FROM producer_status)",
            "tm AS MATERIALIZED(SELECT*FROM terminal_match)",
            "nv AS MATERIALIZED(SELECT*FROM navigation)",
            "cp AS MATERIALIZED(SELECT*FROM capture)",
            "ft AS MATERIALIZED(SELECT*FROM fact)",
        ]
        .map(|cte| recursive_materialization_fence(cte.to_owned()));
        let mut materializations = renderer.materializations;
        for materialization in &mut materializations {
            materialization.ctes = materialization
                .ctes
                .iter()
                .map(|cte| compact_utility_cte_for_catalog(cte, large_catalog))
                .collect();
            let mut declared = BTreeSet::new();
            materialization.ctes.retain(|cte| {
                cte.split_once(" AS ")
                    .is_none_or(|(name, _)| declared.insert(name.to_owned()))
            });
            if large_catalog {
                for base in base_relations.iter().rev() {
                    let name = base.split_once(" AS ").expect("base relation alias").0;
                    if !materialization
                        .ctes
                        .iter()
                        .any(|cte| cte.starts_with(&format!("{name} AS ")))
                    {
                        materialization.ctes.insert(0, base.clone());
                    }
                }
            }
        }
        let mut ctes = renderer
            .ctes
            .into_iter()
            .map(|cte| compact_utility_cte_for_catalog(&cte, large_catalog))
            .collect::<Vec<_>>();
        let mut declared = BTreeSet::new();
        ctes.retain(|cte| {
            cte.split_once(" AS ")
                .is_none_or(|(name, _)| declared.insert(name.to_owned()))
        });
        if large_catalog {
            for base in base_relations.into_iter().rev() {
                let name = base.split_once(" AS ").expect("base relation alias").0;
                if !ctes
                    .iter()
                    .any(|cte| cte.starts_with(&format!("{name} AS ")))
                {
                    ctes.insert(0, base);
                }
            }
        }
        let rendered_bytes = ctes.iter().map(String::len).sum::<usize>();
        if rendered_bytes > MAX_RENDERED_SQL_BYTES {
            return Err(format!(
                "relational SQL exceeds bounded renderer capacity after layout compaction: {rendered_bytes} bytes > {MAX_RENDERED_SQL_BYTES} bytes"
            ));
        }
        Ok((ctes, materializations))
    }

    pub(super) fn normalized_sql(&self) -> String {
        self.operations
            .iter()
            .map(|operation| operation.fragment())
            .collect::<Vec<_>>()
            .join(" -> ")
    }

    pub(super) fn validate(&self) -> Result<(), String> {
        if self.operations.first() != Some(&SqlOperation::PackedMaterialized) {
            return Err("relational IR must start with a materialized packed scan".to_owned());
        }
        if self.operations.get(1) != Some(&SqlOperation::StatusJoin) {
            return Err("relational IR must join producer status before evaluation".to_owned());
        }
        if ALL_TERMINAL_KINDS.len() != ALL_TERMINAL_KINDS.iter().collect::<BTreeSet<_>>().len() {
            return Err("relational IR terminal catalog contains duplicate kinds".to_owned());
        }
        if ALL_SQL_OPERATIONS.len() != ALL_SQL_OPERATIONS.iter().collect::<BTreeSet<_>>().len() {
            return Err("relational IR SQL catalog contains duplicate operations".to_owned());
        }
        let mut terminals = Vec::new();
        let mut operations = vec![SqlOperation::PackedMaterialized, SqlOperation::StatusJoin];
        for root in &self.roots {
            root.collect_terminals(&mut terminals);
            root.collect_operations(&mut operations);
        }
        if terminals != self.terminal_kinds || operations != self.operations {
            return Err("relational IR explanation does not match its expression graph".to_owned());
        }
        Ok(())
    }

    pub(super) fn refresh_explanation(&mut self) {
        let mut terminals = Vec::new();
        let mut operations = vec![SqlOperation::PackedMaterialized, SqlOperation::StatusJoin];
        for root in &self.roots {
            root.collect_terminals(&mut terminals);
            root.collect_operations(&mut operations);
        }
        self.terminal_kinds = terminals;
        self.operations = operations;
    }
}

#[cfg(test)]
mod layout_tests {
    use super::{
        NavigationAxis, NavigationSpec, NavigationStop, RelationNode, RelationalIr, SqlOperation,
        SqlRenderer, TerminalKind, TerminalSpec, UtilityCatalog, compact_large_utility_base_names,
        compact_sql_layout, compact_utility_cte, compact_utility_cte_for_catalog,
        compact_utility_row_layout,
    };
    use std::sync::Arc;

    #[test]
    fn column_declared_utility_ctes_get_structural_materialization_fences() {
        let compact = compact_utility_cte_for_catalog(
            "u_walk(file_index,node) AS (SELECT file_index,node FROM capture)",
            true,
        );

        assert_eq!(
            compact,
            "u_walk(file_index,node)AS MATERIALIZED (SELECT file_index,node FROM cp UNION ALL SELECT*FROM u_walk WHERE 0)"
        );
    }

    #[test]
    fn compact_utility_rows_remove_dead_packed_columns() {
        let sql = "u AS (SELECT m.batch,m.row_kind,m.producer_id,m.file_index,m.task_index,m.match_index,m.capture_sequence_start,m.capture_sequence_count,m.node,m.axis_kind,m.start_ordinal,m.verdict FROM terminal_match m)";
        let compact = compact_utility_row_layout(sql);
        assert_eq!(
            compact,
            "u AS (SELECT m.producer_id,m.file_index,m.match_index,m.node,m.verdict FROM terminal_match m)"
        );
    }

    #[test]
    fn physical_region_memo_retains_its_logical_relation_owner() {
        let shared = Arc::new(RelationNode::Empty);
        let mut utilities = UtilityCatalog::default();
        let mut renderer = SqlRenderer::new(0, true, &mut utilities);

        renderer
            .render(&RelationNode::Shared(Arc::clone(&shared)))
            .unwrap();

        assert_eq!(Arc::strong_count(&shared), 2);
    }

    #[test]
    fn independent_conjunction_operands_share_one_physical_region() {
        let terminal = |producer| RelationNode::Apply {
            operation: SqlOperation::ScanTerminal,
            inputs: vec![RelationNode::Producer(TerminalSpec::new(
                TerminalKind::Kind,
                format!("terminal:{producer}"),
            ))],
        };
        let parts = (0..4)
            .map(|producer| RelationalIr {
                roots: vec![terminal(producer)],
                terminal_kinds: vec![TerminalKind::Kind],
                operations: vec![SqlOperation::ScanTerminal],
            })
            .collect();

        let combined = RelationalIr::combine(SqlOperation::InnerJoin, parts).unwrap();

        let [RelationNode::Apply { operation, inputs }] = combined.roots.as_slice() else {
            panic!("conjunction must own one physical relation region");
        };
        assert_eq!(*operation, SqlOperation::InnerJoin);
        assert_eq!(inputs.len(), 4);
    }

    #[test]
    fn independent_capture_conjunction_operands_share_one_physical_region() {
        let terminal = |producer| RelationNode::Apply {
            operation: SqlOperation::ScanTerminal,
            inputs: vec![RelationNode::Producer(TerminalSpec::new(
                TerminalKind::Pattern,
                format!("terminal:{producer}"),
            ))],
        };
        let parts = (0..4)
            .map(|producer| RelationalIr {
                roots: vec![terminal(producer)],
                terminal_kinds: vec![TerminalKind::Pattern],
                operations: vec![SqlOperation::ScanTerminal],
            })
            .collect();

        let combined = RelationalIr::combine(SqlOperation::InnerJoin, parts).unwrap();

        let [RelationNode::Apply { operation, inputs }] = combined.roots.as_slice() else {
            panic!("capture conjunction must own one physical relation region");
        };
        assert_eq!(*operation, SqlOperation::InnerJoin);
        assert_eq!(inputs.len(), 4);
    }

    #[test]
    fn conjunction_with_one_binding_input_carries_bindings_without_remerging_them() {
        let terminal = |producer, kind| RelationNode::Apply {
            operation: SqlOperation::ScanTerminal,
            inputs: vec![RelationNode::Producer(TerminalSpec::new(
                kind,
                format!("terminal:{producer}"),
            ))],
        };
        let relation = RelationNode::Apply {
            operation: SqlOperation::InnerJoin,
            inputs: vec![
                terminal(0, TerminalKind::Pattern),
                terminal(1, TerminalKind::Kind),
                terminal(2, TerminalKind::Regex),
            ],
        };
        let mut utilities = UtilityCatalog::default();
        let mut renderer = SqlRenderer::new(usize::MAX, true, &mut utilities);

        renderer.render(&relation).unwrap();

        let joined = renderer.ctes.last().unwrap();
        assert!(joined.contains("left_input.bindings"));
        assert!(!joined.contains("json_each("));
    }

    #[test]
    fn conjunction_compatibility_ignores_capture_free_inputs() {
        let terminal = |producer, kind| RelationNode::Apply {
            operation: SqlOperation::ScanTerminal,
            inputs: vec![RelationNode::Producer(TerminalSpec::new(
                kind,
                format!("terminal:{producer}"),
            ))],
        };
        let relation = RelationNode::Apply {
            operation: SqlOperation::InnerJoin,
            inputs: vec![
                terminal(0, TerminalKind::Pattern),
                terminal(1, TerminalKind::Kind),
                terminal(2, TerminalKind::Pattern),
            ],
        };
        let mut utilities = UtilityCatalog::default();
        let mut renderer = SqlRenderer::new(usize::MAX, true, &mut utilities);

        renderer.render(&relation).unwrap();

        let joined = renderer.ctes.last().unwrap();
        assert!(joined.contains("json_each(left_input.bindings)"));
        assert!(joined.contains("json_each(input_2.bindings)"));
        assert!(!joined.contains("json_each(right_input.bindings)"));
    }

    #[test]
    fn utility_navigation_owns_acceptance_and_result_in_one_region() {
        let terminal = |producer, kind| RelationNode::Apply {
            operation: SqlOperation::ScanTerminal,
            inputs: vec![RelationNode::Producer(TerminalSpec::new(
                kind,
                format!("terminal:{producer}"),
            ))],
        };
        let relation = RelationNode::Navigate {
            spec: NavigationSpec {
                axis: NavigationAxis::Descendant,
                stop: NavigationStop::Neighbor,
                field: None,
                field_stop_children_only: false,
                producer: 2,
                enumerate: false,
            },
            source: Box::new(terminal(0, TerminalKind::Universe)),
            related: Box::new(terminal(1, TerminalKind::Kind)),
            stop: None,
        };
        let mut utilities = UtilityCatalog::default();
        let mut renderer = SqlRenderer::new(usize::MAX, true, &mut utilities);

        renderer.render(&relation).unwrap();

        assert!(
            renderer
                .ctes
                .iter()
                .all(|cte| !cte.contains("_accepted AS"))
        );
        assert!(renderer.ctes.iter().all(|cte| !cte.contains("_walk(")));
    }

    #[test]
    fn single_consumer_utility_navigation_inlines_its_context() {
        let terminal = |producer, kind| RelationNode::Apply {
            operation: SqlOperation::ScanTerminal,
            inputs: vec![RelationNode::Producer(TerminalSpec::new(
                kind,
                format!("terminal:{producer}"),
            ))],
        };
        let relation = RelationNode::Navigate {
            spec: NavigationSpec {
                axis: NavigationAxis::Descendant,
                stop: NavigationStop::End,
                field: None,
                field_stop_children_only: false,
                producer: 2,
                enumerate: false,
            },
            source: Box::new(terminal(0, TerminalKind::Universe)),
            related: Box::new(terminal(1, TerminalKind::Kind)),
            stop: None,
        };
        let mut utilities = UtilityCatalog::default();
        let mut renderer = SqlRenderer::new(usize::MAX, true, &mut utilities);

        renderer.render(&relation).unwrap();

        assert!(renderer.ctes.iter().all(|cte| !cte.contains("_context AS")));
    }

    #[test]
    fn utility_choice_prefixes_stay_inside_their_consumer_regions() {
        let terminal = |producer| RelationNode::Apply {
            operation: SqlOperation::ScanTerminal,
            inputs: vec![RelationNode::Producer(TerminalSpec::new(
                TerminalKind::Kind,
                format!("terminal:{producer}"),
            ))],
        };
        let alternatives = RelationNode::Apply {
            operation: SqlOperation::OrderedUnion,
            inputs: vec![terminal(1), terminal(2)],
        }
        .apply_after(terminal(0));
        let mut utilities = UtilityCatalog::default();
        let mut renderer = SqlRenderer::new(usize::MAX, true, &mut utilities);

        renderer.render(&alternatives).unwrap();

        assert!(!renderer.relation_families.contains_key("choice"));
        assert!(renderer.ctes.iter().all(|cte| !cte.contains("UNION ALL")));
    }

    #[test]
    fn utility_capacity_uses_the_final_compact_predicate_shape() {
        let authored = SqlRenderer::binding_compatibility("left", "right");
        let compact = compact_utility_cte(&format!("u AS (SELECT 1 WHERE {authored})"));

        assert_eq!(compact.matches("lb.value->>'n'=rb.value->>'n'").count(), 1);
        assert!(compact.len() < authored.len());
    }

    #[test]
    fn compact_utility_compatibility_emits_capture_equality_once() {
        let predicate = SqlRenderer::compact_binding_compatibility("left", "right");

        assert_eq!(
            predicate
                .matches("left_binding.value->>'n' = right_binding.value->>'n'")
                .count(),
            1
        );
        assert!(predicate.contains("CASE WHEN left_binding.value->>'f' = 1"));
        assert!(predicate.contains("left_binding.value->>'s'"));
        assert!(predicate.contains("right_binding.value->>'l'"));
        assert!(predicate.contains("left_binding.value->>'c'"));
        assert!(!predicate.contains("SELECT count(*)"));
        assert!(!predicate.contains("left_binding.value->>'syntax'"));
        assert!(!predicate.contains("json_valid"));
    }

    #[test]
    fn compact_layout_shortens_navigation_region_identifiers() {
        assert_eq!(
            compact_sql_layout(
                "SELECT json('[]'), walk.source_node, walk.source_bindings, accepted.accepted_order, branch.branch_order, branch.unknown_blocks, chosen.truth_order FROM relation walk INNER JOIN relation actual ON actual.source_node = walk.source_node"
            ),
            "SELECT '[]',w.sn,w.sb,ac.ao,br.bo,br.ub,chosen.tr FROM relation w JOIN relation a ON a.sn=w.sn"
        );
        assert_eq!(
            compact_sql_layout(
                "SELECT 'SELECT 1 FROM text' FROM left INNER JOIN right WHERE EXISTS (SELECT 1 FROM nested)"
            ),
            "SELECT 'SELECT 1 FROM text' FROM left JOIN right WHERE EXISTS(SELECT*FROM nt)"
        );
        assert_eq!(
            compact_sql_layout(
                "SELECT json_extract(left_value.value, '$.semantic_structure') FROM relation"
            ),
            "SELECT lv.value->>'semantic_structure'FROM relation"
        );
        assert_eq!(
            compact_sql_layout("walk(file_index, node) AS (SELECT 1)"),
            "w(file_index,node)AS (SELECT 1)"
        );
        assert_eq!(
            compact_sql_layout(
                "u12345 AS (SELECT relation_42.node FROM relation_42 JOIN compare_3 USING (file_index, node))"
            ),
            "u9ix AS (SELECT r42.node FROM r42 JOIN q3 USING(file_index,node))"
        );
    }

    #[test]
    fn large_utility_catalogs_use_short_base_relation_names() {
        assert_eq!(
            compact_large_utility_base_names(
                "u AS(SELECT * FROM terminal_match JOIN producer_status USING(producer_id,file_index) JOIN navigation USING(file_index) JOIN capture USING(file_index) JOIN fact USING(file_index))"
            ),
            "u AS(SELECT * FROM tm JOIN ps USING(producer_id,file_index) JOIN nv USING(file_index) JOIN cp USING(file_index) JOIN ft USING(file_index))"
        );
    }

    #[test]
    fn terminal_verification_visits_each_shared_relation_once() {
        let mut root = Arc::new(RelationNode::Producer(TerminalSpec::new(
            TerminalKind::Kind,
            "terminal:7".to_owned(),
        )));
        for _ in 0..32 {
            root = Arc::new(RelationNode::Apply {
                operation: SqlOperation::InnerJoin,
                inputs: vec![
                    RelationNode::Shared(Arc::clone(&root)),
                    RelationNode::Shared(Arc::clone(&root)),
                ],
            });
        }
        let ir = RelationalIr {
            roots: vec![RelationNode::Shared(root)],
            terminal_kinds: vec![TerminalKind::Kind],
            operations: vec![SqlOperation::InnerJoin],
        };

        let (specs, shared_visits) = ir.terminal_specs_with_shared_visits();

        assert_eq!(
            specs,
            [TerminalSpec::new(
                TerminalKind::Kind,
                "terminal:7".to_owned()
            )]
            .into_iter()
            .collect()
        );
        assert_eq!(shared_visits, 33);
    }

    #[test]
    fn compact_layout_shortens_numbered_relational_aliases() {
        assert_eq!(
            compact_sql_layout(
                "SELECT input_2.node <> edge0.node AS verdict FROM relation input_2 JOIN edge0 WHERE json_extract(left_binding.value, '$.label') = 'x'"
            ),
            "SELECT i2.node<>e0.node verdict FROM relation i2 JOIN e0 WHERE lb.value->>'label'='x'"
        );
    }
}
