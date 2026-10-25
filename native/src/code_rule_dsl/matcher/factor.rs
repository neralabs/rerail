//! Matcher adapter for reusable producer prefix planning. Only closed syntactic predicates
//! enter the plan. Capture reads/writes, semantic context, enumeration and calls stay residual.
use super::{AstMatcher, StopBy};
use ast_grep_core::matcher::Matcher;
use std::{collections::BTreeSet, sync::Arc};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::code_rule_dsl) enum Key {
    Never,
    Kinds(Vec<u16>),
    Regex(String),
    All(Vec<Key>),
    Any(Vec<Key>),
    Not(Box<Key>),
    Relation(u8, Box<Key>, Box<Key>),
    Stop(u8),
    Utility(Arc<[u8]>),
    Count(Box<Key>, usize, Option<usize>),
}
fn closed(m: &AstMatcher, visited: &mut BTreeSet<(usize, usize)>) -> bool {
    match m {
        AstMatcher::Never | AstMatcher::Kind(_) | AstMatcher::Regex(_) => true,
        AstMatcher::All(v) | AstMatcher::Any(v) => v.iter().all(|m| closed(m, visited)),
        AstMatcher::Not(m) => closed(m, visited),
        AstMatcher::Has(m, s)
        | AstMatcher::Inside(m, s)
        | AstMatcher::Precedes(m, s)
        | AstMatcher::Follows(m, s) => {
            closed(m, visited)
                && match s {
                    StopBy::Rule(m) => closed(m, visited),
                    _ => true,
                }
        }
        AstMatcher::Count(c) => c.binding.is_none() && closed(&c.relation, visited),
        AstMatcher::Util(u) => {
            let id = (u.table.as_ptr() as usize, u.index);
            if !visited.insert(id) {
                return true;
            }
            u.with_target(|m| closed(m, visited)).unwrap_or(false)
        }
        _ => false,
    }
}
pub(in crate::code_rule_dsl) fn key(m: &AstMatcher) -> Option<Key> {
    Some(match m {
        AstMatcher::Never => Key::Never,
        AstMatcher::Kind(k) => Key::Kinds(k.potential_kinds()?.iter().map(|k| k as u16).collect()),
        AstMatcher::Regex(r) => Key::Regex(r.as_str().into()),
        AstMatcher::All(v) => Key::All(v.iter().map(key).collect::<Option<_>>()?),
        AstMatcher::Any(v) => Key::Any(v.iter().map(key).collect::<Option<_>>()?),
        AstMatcher::Not(m) => Key::Not(Box::new(key(m)?)),
        AstMatcher::Has(r, s)
        | AstMatcher::Inside(r, s)
        | AstMatcher::Precedes(r, s)
        | AstMatcher::Follows(r, s) => {
            let stop = match s {
                StopBy::Neighbor => Key::Stop(0),
                StopBy::End => Key::Stop(1),
                StopBy::Rule(m) => key(m)?,
            };
            let direction = match m {
                AstMatcher::Has(..) => 0,
                AstMatcher::Inside(..) => 1,
                AstMatcher::Precedes(..) => 2,
                _ => 3,
            };
            Key::Relation(direction, Box::new(key(r)?), Box::new(stop))
        }
        AstMatcher::Count(c) if c.binding.is_none() => {
            Key::Count(Box::new(key(&c.relation)?), c.min, c.max)
        }
        AstMatcher::Util(u) if closed(m, &mut BTreeSet::new()) => {
            let table = u.table.upgrade()?;
            Key::Utility(table.origins.get(u.index)?.as_ref()?.definition.clone())
        }
        _ => return None,
    })
}

/// The catalog describes the operation; prefix planning and backend execution remain separate.
/// Consumers must bind a closed domain and an execution budget, including empty completion.
pub(in crate::code_rule_dsl) fn contract() -> crate::code_rule_dsl::producers::Contract {
    use crate::code_rule_dsl::producers::{Column, Contract, Definition, Order};
    use std::collections::BTreeMap;
    Contract {
        name: "matcher.closed_predicate.v1".into(),
        origin: "matcher/factor.rs".into(),
        definition: Definition {
            inputs: BTreeMap::from([
                ("tape".into(), "syntax-tape".into()),
                ("predicate".into(), "closed-matcher-definition".into()),
                ("domain".into(), "ordered-root-domain".into()),
                ("budget".into(), "execution-budget".into()),
            ]),
            options: BTreeMap::new(),
            columns: [
                ("request", "request-id"),
                ("file", "document"),
                ("node", "node"),
                ("truth", "outcome"),
            ]
            .into_iter()
            .map(|(name, kind)| Column {
                name: name.into(),
                kind: kind.into(),
                nullable: false,
            })
            .collect(),
            key: vec!["file".into(), "request".into()],
            completion: "domain".into(),
            multiplicity: "bag".into(),
            ordering: vec![Order {
                column: "node".into(),
                direction: "asc".into(),
                nulls: "first".into(),
            }],
            implementation: BTreeMap::from([
                ("native".into(), "closed AstMatcher predicate".into()),
                ("revision".into(), "1".into()),
            ]),
        },
    }
}

#[derive(Clone, Default)]
pub(in crate::code_rule_dsl) struct Prefix {
    pub keys: Vec<Key>,
    pub terms: Vec<AstMatcher>,
    pub rest: Vec<AstMatcher>,
}
impl Prefix {
    pub fn of(m: &AstMatcher, enumerating: bool, kinds: Option<&[u16]>) -> Self {
        let mut terms = match m {
            AstMatcher::All(v) => v.clone(),
            _ => vec![m.clone()],
        };
        // Admission already proves these direct kind tests. Materializing their positive
        // outputs would copy the entire input domain without reducing any native work.
        if !enumerating && let Some(kinds) = kinds {
            terms.retain(|m| match m {
                AstMatcher::Kind(k) => k.potential_kinds().is_none_or(|allowed| {
                    !kinds.iter().all(|kind| allowed.contains(*kind as usize))
                }),
                _ => true,
            });
        }
        let keys = if enumerating {
            vec![]
        } else {
            terms.iter().map_while(key).collect::<Vec<_>>()
        };
        let count = keys.len();
        Self {
            keys,
            terms: terms[..count].to_vec(),
            rest: terms[count..].to_vec(),
        }
    }
    pub fn residual(&self, consumed: usize) -> AstMatcher {
        AstMatcher::All(
            self.terms[consumed..]
                .iter()
                .chain(&self.rest)
                .cloned()
                .collect(),
        )
    }
}
