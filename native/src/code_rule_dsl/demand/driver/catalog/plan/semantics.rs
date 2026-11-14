use super::identity::LogicalFillId;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DslBehavior {
    AtomicPredicate,
    CaptureFreeRelation,
    FirstWitnessRelation,
    IndependentAll,
    DependentAll,
    IndependentAny,
    EffectfulAny,
    Each,
    AcyclicUtility,
    RecursiveUtility,
    LogicalBudget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DeclarativeContract {
    ClosedRelation,
    SemiJoin,
    RelationalJoin,
    OrderedChoice,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PushedOperator {
    OrderedNavigation,
    OrderedRegion,
    OrderedEnumeration,
    UtilityAnswers,
    RecursiveUtilityAnswers,
    EffectLedger,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OperationalOwner {
    Declarative(DeclarativeContract),
    Pushed(PushedOperator),
}

impl DslBehavior {
    pub(super) fn owner(self) -> OperationalOwner {
        match self {
            Self::AtomicPredicate => {
                OperationalOwner::Declarative(DeclarativeContract::ClosedRelation)
            }
            Self::CaptureFreeRelation => {
                OperationalOwner::Declarative(DeclarativeContract::SemiJoin)
            }
            Self::FirstWitnessRelation => {
                OperationalOwner::Pushed(PushedOperator::OrderedNavigation)
            }
            Self::IndependentAll => {
                OperationalOwner::Declarative(DeclarativeContract::RelationalJoin)
            }
            Self::DependentAll | Self::EffectfulAny => {
                OperationalOwner::Pushed(PushedOperator::OrderedRegion)
            }
            Self::IndependentAny => {
                OperationalOwner::Declarative(DeclarativeContract::OrderedChoice)
            }
            Self::Each => OperationalOwner::Pushed(PushedOperator::OrderedEnumeration),
            Self::AcyclicUtility => OperationalOwner::Pushed(PushedOperator::UtilityAnswers),
            Self::RecursiveUtility => {
                OperationalOwner::Pushed(PushedOperator::RecursiveUtilityAnswers)
            }
            Self::LogicalBudget => OperationalOwner::Pushed(PushedOperator::EffectLedger),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) enum FillOwner {
    File,
    Clause,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct LogicalFill {
    key: LogicalFillId,
    work: u64,
    owner: FillOwner,
}

impl LogicalFill {
    pub(super) fn new(key: LogicalFillId, work: u64, owner: FillOwner) -> Self {
        Self { key, work, owner }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum EffectSummaryError {
    ConflictingFill(LogicalFillId, FillOwner),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct EffectSummary {
    replay_work: u64,
    relative_peak_depth: u16,
    fills: Vec<LogicalFill>,
}

impl EffectSummary {
    pub(super) fn new(
        replay_work: u64,
        relative_peak_depth: u16,
        mut fills: Vec<LogicalFill>,
    ) -> Result<Self, EffectSummaryError> {
        fills.sort_unstable_by_key(|fill| (fill.owner, fill.key));
        let mut canonical = Vec::<LogicalFill>::with_capacity(fills.len());
        for fill in fills {
            if let Some(previous) = canonical.last()
                && previous.owner == fill.owner
                && previous.key == fill.key
            {
                if previous.work != fill.work {
                    return Err(EffectSummaryError::ConflictingFill(fill.key, fill.owner));
                }
                continue;
            }
            canonical.push(fill);
        }
        Ok(Self {
            replay_work,
            relative_peak_depth,
            fills: canonical,
        })
    }
}

#[derive(Default)]
pub(super) struct FileEffectState {
    fills: BTreeSet<LogicalFillId>,
}

impl FileEffectState {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn is_empty(&self) -> bool {
        self.fills.is_empty()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum EffectFailure {
    DepthLimit { required: u32, maximum: u16 },
    WorkLimit { required: u64, remaining: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct EffectApplication {
    work_charged: u64,
}

impl EffectApplication {
    pub(super) fn work_charged(self) -> u64 {
        self.work_charged
    }
}

pub(super) struct EffectLedger {
    maximum_work: u64,
    used_work: u64,
    maximum_depth: u16,
    clause_fills: BTreeSet<LogicalFillId>,
}

impl EffectLedger {
    pub(super) fn new(maximum_work: u64, maximum_depth: u16) -> Self {
        Self {
            maximum_work,
            used_work: 0,
            maximum_depth,
            clause_fills: BTreeSet::new(),
        }
    }

    pub(super) fn apply(
        &mut self,
        summary: &EffectSummary,
        file: &mut FileEffectState,
    ) -> Result<EffectApplication, EffectFailure> {
        self.apply_at_depth(summary, 0, file)
    }

    pub(super) fn apply_at_depth(
        &mut self,
        summary: &EffectSummary,
        current_depth: u16,
        file: &mut FileEffectState,
    ) -> Result<EffectApplication, EffectFailure> {
        let required_depth = u32::from(current_depth) + u32::from(summary.relative_peak_depth);
        if required_depth > u32::from(self.maximum_depth) {
            return Err(EffectFailure::DepthLimit {
                required: required_depth,
                maximum: self.maximum_depth,
            });
        }

        let mut required_work = summary.replay_work;
        for fill in &summary.fills {
            let present = match fill.owner {
                FillOwner::File => file.fills.contains(&fill.key),
                FillOwner::Clause => self.clause_fills.contains(&fill.key),
            };
            if !present {
                required_work =
                    required_work
                        .checked_add(fill.work)
                        .ok_or(EffectFailure::WorkLimit {
                            required: u64::MAX,
                            remaining: self.maximum_work.saturating_sub(self.used_work),
                        })?;
            }
        }
        let remaining = self.maximum_work.saturating_sub(self.used_work);
        if required_work > remaining {
            return Err(EffectFailure::WorkLimit {
                required: required_work,
                remaining,
            });
        }

        for fill in &summary.fills {
            match fill.owner {
                FillOwner::File => {
                    file.fills.insert(fill.key);
                }
                FillOwner::Clause => {
                    self.clause_fills.insert(fill.key);
                }
            }
        }
        self.used_work += required_work;
        Ok(EffectApplication {
            work_charged: required_work,
        })
    }

    pub(super) fn used_work(&self) -> u64 {
        self.used_work
    }
}
