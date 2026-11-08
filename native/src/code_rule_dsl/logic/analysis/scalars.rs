//! Compatibility of retained scalar summaries, independent of their storage and evaluator.
use super::Decision;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
pub(in crate::code_rule_dsl) enum Expression {
    Column(usize),
    Integer(i64),
    Text(String),
    /// Intrinsic operators only. Function callbacks must use Opaque.
    Apply(String, Vec<Expression>),
    Opaque(String),
}
impl Expression {
    pub fn stable(&self) -> bool {
        match self {
            Self::Opaque(_) => false,
            Self::Apply(operator, args) => {
                matches!(
                    operator.as_str(),
                    "=" | "Equal"
                        | "NotEqual"
                        | "Less"
                        | "LessOrEqual"
                        | "Greater"
                        | "GreaterOrEqual"
                        | "And"
                        | "Or"
                        | "Not"
                        | "IsNull"
                        | "IsNotNull"
                ) && args.iter().all(Self::stable)
            }
            _ => true,
        }
    }
    fn value(&self) -> bool {
        matches!(self, Self::Column(_) | Self::Integer(_) | Self::Text(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub(in crate::code_rule_dsl) enum Kind {
    Count,
    Sum,
    Min,
    Max,
    OrderedJoin,
    Exists,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub(in crate::code_rule_dsl) struct Scalar {
    pub kind: Kind,
    pub source: String,
    pub keys: Vec<Expression>,
    pub predicates: Vec<Expression>,
    pub lookup: Vec<Expression>,
    pub value: Expression,
    pub dependent: bool,
    pub non_null: bool,
}
impl Scalar {
    pub fn compatible(&self, other: &Self) -> Decision {
        let reason = if matches!(self.kind, Kind::OrderedJoin | Kind::Exists)
            || matches!(other.kind, Kind::OrderedJoin | Kind::Exists)
        {
            Some("Ordering, string, or existence semantics require a separate summary")
        } else if self.dependent || other.dependent {
            Some(
                "Aggregate-dependent or computed correlation keys require separate dependency maintenance",
            )
        } else if !self.non_null || !other.non_null {
            Some("Nullable values require a separate null-count contract")
        } else if self.source != other.source
            || self.keys != other.keys
            || self.lookup != other.lookup
            || self.predicates != other.predicates
        {
            Some("Input, grouping keys, correlation mapping, or filters differ")
        } else if self
            .keys
            .iter()
            .chain(&self.predicates)
            .chain(&self.lookup)
            .any(|e| !e.stable())
            || other
                .keys
                .iter()
                .chain(&other.predicates)
                .chain(&other.lookup)
                .any(|e| !e.stable())
        {
            Some("An opaque expression has no evaluation-sharing proof")
        } else if self.kind != Kind::Count && !self.value.value()
            || other.kind != Kind::Count && !other.value.value()
        {
            Some("Computed values require an explicit evaluation and error-timing contract")
        } else if self.kind != Kind::Count && other.kind != Kind::Count && self.value != other.value
        {
            Some("Value expressions differ")
        } else {
            None
        };
        Decision::new(reason.is_none(), reason.unwrap_or("Same non-null bag, keys, filters, and value; preserve empty results and exact sums"))
    }
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub(in crate::code_rule_dsl) struct Layout {
    pub count: bool,
    pub sum: bool,
    pub min: bool,
    pub max: bool,
}
#[derive(Clone, Debug, serde::Serialize)]
pub(in crate::code_rule_dsl) struct Family {
    pub members: Vec<usize>,
    pub layout: Layout,
}
#[derive(Clone, Debug, serde::Serialize)]
pub(in crate::code_rule_dsl) struct Check {
    pub left: usize,
    pub right: usize,
    pub decision: Decision,
}
#[derive(Clone, Debug, Default, serde::Serialize)]
pub(in crate::code_rule_dsl) struct Analysis {
    pub inputs: Vec<Scalar>,
    pub families: Vec<Family>,
    pub checks: Vec<Check>,
}
pub(in crate::code_rule_dsl) fn analyze(inputs: Vec<Scalar>) -> Analysis {
    let mut result = Analysis {
        inputs,
        ..Default::default()
    };
    for i in 0..result.inputs.len() {
        let eligible = result.inputs[i].compatible(&result.inputs[i]);
        let can_share = eligible.proven;
        result.checks.push(Check {
            left: i,
            right: i,
            decision: eligible,
        });
        let mut selected = None;
        for (f, family) in result.families.iter().enumerate() {
            if !can_share {
                break;
            }
            let mut accepted = true;
            // Check every member: count must not bridge two different value expressions.
            for &j in &family.members {
                let decision = result.inputs[i].compatible(&result.inputs[j]);
                accepted &= decision.proven;
                result.checks.push(Check {
                    left: j,
                    right: i,
                    decision,
                });
                if !accepted {
                    break;
                }
            }
            if accepted {
                selected = Some(f);
                break;
            }
        }
        let f = selected.unwrap_or_else(|| {
            result.families.push(Family {
                members: vec![],
                layout: Layout::default(),
            });
            result.families.len() - 1
        });
        let family = &mut result.families[f];
        family.members.push(i);
        match result.inputs[i].kind {
            Kind::Count => family.layout.count = true,
            Kind::Sum => family.layout.sum = true,
            Kind::Min => family.layout.min = true,
            Kind::Max => family.layout.max = true,
            _ => {}
        }
    }
    result
}
