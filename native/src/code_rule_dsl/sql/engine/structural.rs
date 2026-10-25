//! Pure structural relations. These predicates have no captures, utility calls, or unknown effects.
use crate::code_rule_dsl::matcher::{AstMatcher, StopBy};
use ast_grep_core::matcher::Matcher;
#[derive(Clone)]
pub(super) enum Predicate {
    False,
    Kinds(Vec<u16>),
    All(Vec<Self>),
    Any(Vec<Self>),
    Not(Box<Self>),
    Related {
        direction: Direction,
        immediate: bool,
        child: Box<Self>,
    },
    Count {
        relation: Box<Self>,
        min: usize,
        max: Option<usize>,
    },
}
#[derive(Clone, Copy)]
pub(super) enum Direction {
    Down,
    Up,
    Before,
    After,
}
impl Predicate {
    pub fn lower(m: &AstMatcher) -> Option<Self> {
        Some(match m {
            AstMatcher::Never => Self::False,
            AstMatcher::Kind(k) => {
                Self::Kinds(k.potential_kinds()?.iter().map(|k| k as u16).collect())
            }
            AstMatcher::All(v) => Self::All(v.iter().map(Self::lower).collect::<Option<_>>()?),
            AstMatcher::Any(v) => Self::Any(v.iter().map(Self::lower).collect::<Option<_>>()?),
            AstMatcher::Not(m) => Self::Not(Box::new(Self::lower(m)?)),
            AstMatcher::Count(c) if c.binding.is_none() => Self::Count {
                relation: Box::new(Self::lower(&c.relation)?),
                min: c.min,
                max: c.max,
            },
            AstMatcher::Has(r, s)
            | AstMatcher::Inside(r, s)
            | AstMatcher::Precedes(r, s)
            | AstMatcher::Follows(r, s) => {
                let immediate = match s {
                    StopBy::Neighbor => true,
                    StopBy::End => false,
                    _ => return None,
                };
                Self::Related {
                    direction: match m {
                        AstMatcher::Has(..) => Direction::Down,
                        AstMatcher::Inside(..) => Direction::Up,
                        AstMatcher::Precedes(..) => Direction::After,
                        _ => Direction::Before,
                    },
                    immediate,
                    child: Box::new(Self::lower(r)?),
                }
            }
            _ => return None,
        })
    }
    pub fn sql(&self, n: &str, depth: usize) -> String {
        match self {
            Self::False => "0".into(),
            Self::Kinds(k) => format!(
                "{n}.kind IN ({})",
                k.iter().map(u16::to_string).collect::<Vec<_>>().join(",")
            ),
            Self::All(v) | Self::Any(v) => {
                if v.is_empty() {
                    return if matches!(self, Self::All(_)) {
                        "1"
                    } else {
                        "0"
                    }
                    .into();
                }
                format!(
                    "({})",
                    v.iter().map(|p| p.sql(n, depth)).collect::<Vec<_>>().join(
                        if matches!(self, Self::All(_)) {
                            " AND "
                        } else {
                            " OR "
                        }
                    )
                )
            }
            Self::Not(p) => format!("NOT ({})", p.sql(n, depth)),
            Self::Related { .. } => format!("EXISTS (SELECT 1 {})", self.walk(n, depth).unwrap()),
            Self::Count { relation, min, max } => {
                let count = format!(
                    "(SELECT count(*) {})",
                    relation.walk(n, depth).expect("count relation")
                );
                format!(
                    "{count} BETWEEN {min} AND {}",
                    max.map_or(i64::MAX as usize, |m| m)
                )
            }
        }
    }
    fn walk(&self, n: &str, depth: usize) -> Option<String> {
        let Self::Related {
            direction,
            immediate,
            child,
        } = self
        else {
            return None;
        };
        let c = format!("s{depth}");
        let edge = match (direction, immediate) {
            (Direction::Down, true) => format!("{c}.parent={n}.id"),
            (Direction::Down, false) => format!("{c}.id>{n}.id AND {c}.id<={n}.last"),
            (Direction::Up, true) => format!("{c}.id={n}.parent"),
            (Direction::Up, false) => format!("{c}.id<{n}.id AND {c}.last>={n}.id"),
            (Direction::After, true) => format!("{c}.id={n}.next"),
            (Direction::Before, true) => format!("{c}.id={n}.prev"),
            (Direction::After, false) => format!("{c}.parent={n}.parent AND {c}.id>{n}.id"),
            (Direction::Before, false) => format!("{c}.parent={n}.parent AND {c}.id<{n}.id"),
        };
        Some(format!(
            "FROM node {c} WHERE {c}.file={n}.file AND {edge} AND {}",
            child.sql(&c, depth + 1)
        ))
    }
}
impl Predicate {
    /// The alternate anchor is valid only for one conjunctive descendant predicate.
    pub fn rare_descendant(&self) -> Option<&[u16]> {
        match self {
            Self::Related {
                direction: Direction::Down,
                immediate: false,
                child,
            } => {
                if let Self::Kinds(k) = child.as_ref() {
                    Some(k)
                } else {
                    None
                }
            }
            Self::All(v) => {
                let mut relation = None;
                for p in v {
                    match p {
                        Self::Kinds(_) => {}
                        _ => {
                            if relation.is_some() {
                                return None;
                            }
                            relation = Some(p.rare_descendant()?);
                        }
                    }
                }
                relation
            }
            _ => None,
        }
    }
}
