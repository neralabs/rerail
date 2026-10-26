//! Finding identity and coverage are independent of evaluation order and budgets.
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FindingKey {
    pub rule: String,
    pub code: String,
    pub doc: u32,
    pub line: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Coverage {
    Complete,
    Unknown(String),
}

#[derive(Default, Debug)]
pub(crate) struct Findings {
    pub keys: BTreeSet<FindingKey>,
    pub coverage: BTreeMap<(String, u32), Coverage>,
}

impl Findings {
    /// Publish a whole rule/document domain atomically. Failed domains cannot
    /// leak provisional findings, including findings from an earlier stage.
    pub(crate) fn publish(
        &mut self,
        rule: &str,
        doc: u32,
        coverage: Coverage,
        keys: impl IntoIterator<Item = FindingKey>,
    ) {
        if self.coverage.contains_key(&(rule.to_owned(), doc)) {
            self.keys.retain(|key| key.rule != rule || key.doc != doc);
        }
        if coverage == Coverage::Complete {
            self.keys.extend(
                keys.into_iter()
                    .filter(|key| key.rule == rule && key.doc == doc),
            );
        }
        self.coverage.insert((rule.to_owned(), doc), coverage);
    }
}

/// Logical uncertainty is an expression value, not exhausted execution. A
/// resource failure invalidates its domain through Coverage, never this enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Truth {
    No,
    Yes,
    Unknown,
}
impl Truth {
    pub(crate) fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::No, _) | (_, Self::No) => Self::No,
            (Self::Yes, Self::Yes) => Self::Yes,
            _ => Self::Unknown,
        }
    }
    pub(crate) fn or(self, other: Self) -> Self {
        match (self, other) {
            (Self::Yes, _) | (_, Self::Yes) => Self::Yes,
            (Self::No, Self::No) => Self::No,
            _ => Self::Unknown,
        }
    }
    pub(crate) fn not(self) -> Self {
        match self {
            Self::No => Self::Yes,
            Self::Yes => Self::No,
            Self::Unknown => Self::Unknown,
        }
    }
}

#[test]
fn logical_unknown_is_not_resource_failure() {
    use Truth::*;
    assert_eq!(No.and(Unknown), No);
    assert_eq!(Yes.or(Unknown), Yes);
    assert_eq!(Unknown.not(), Unknown);
    for a in [No, Yes, Unknown] {
        for b in [No, Yes, Unknown] {
            assert_eq!(a.and(b), b.and(a));
            assert_eq!(a.or(b), b.or(a));
            assert_eq!(a.and(b).not(), a.not().or(b.not()));
        }
    }
}

#[test]
fn failed_domain_discards_only_its_provisional_findings() {
    let key = |doc| FindingKey {
        rule: "r".into(),
        code: "C".into(),
        doc,
        line: 1,
    };
    let mut result = Findings::default();
    result.publish("r", 1, Coverage::Complete, [key(1), key(1)]);
    result.publish("r", 2, Coverage::Complete, [key(2)]);
    result.publish("r", 1, Coverage::Unknown("budget".into()), [key(1)]);
    assert_eq!(result.keys, BTreeSet::from([key(2)]));
}
