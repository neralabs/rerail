//! An `ast` atom's captures read from the tape (decision 0107 §2, as weakened: a capture may be bound by any means
//! that gives W's rows exactly). At a root the transducer matched, each capture is reached from the root by a path
//! derived once from the rule W compiled: through an `all` to the one term that binds it, an `any`'s first arm that
//! holds (as `match_any` takes it), an `inside`, `has`, `precedes` or `follows` to its first candidate the relation's
//! rule holds at (as `relation.rs` walks it: `stopBy`, `field`; siblings by the tape node's own walks), and a util's rule in place of `matches`. The rules tested on the way
//! are read from the tape by node: kinds, named nodes, supertypes, regexes, the composites and relations of them.
//!
//! A rule outside that set leaves the capture to W (`Binder::of` is `None`): a pattern but a lone metavariable,
//! `nthChild`, constraints, counts, comparisons, semantic predicates, a parameterized util, a
//! multi-capture, and a name two terms of one `all` mention (W unifies them; arms of one `any` may share it).

use super::AstMatcher;
use super::relation::StopBy;
use crate::code_rule_dsl::regexes::Regex;
use crate::code_rule_dsl::tape::TapeDoc;
use ast_grep_core::matcher::{Matcher, PatternNode};
use ast_grep_core::source::SgNode;
use ast_grep_core::meta_var::MetaVariable;
use std::collections::BTreeSet;

/// A rule as the tape answers it at a node, with no environment.
#[derive(Clone)]
enum Rule {
    True,
    Never,
    Named,
    Kind(u16),
    Supertype(u16),
    Regex(Regex),
    All(Vec<Rule>),
    Any(Vec<Rule>),
    Not(Box<Rule>),
    Rel(Box<Rel>),
}

/// How far a relation walks (`StopBy`).
#[derive(Clone)]
enum Walk {
    Neighbor,
    End,
    Until(Rule),
}

/// Which way a relation walks from its node.
#[derive(Clone, Copy, PartialEq)]
enum Dir {
    /// `inside`: to ancestors.
    Up,
    /// `has`: to descendants.
    Down,
    /// `precedes`: to later siblings.
    Next,
    /// `follows`: to earlier siblings.
    Prev,
}

/// `inside`, `has`, `precedes` or `follows`, `inside` and `has` perhaps through a field: its first candidate its rule
/// holds at.
#[derive(Clone)]
struct Rel {
    dir: Dir,
    field: Option<u16>,
    walk: Walk,
    rule: Rule,
}

/// Where a capture is from the node a rule matched: the node itself, an `any`'s first arm that holds (the arms that
/// do not bind it: `None`), or a relation's candidate.
enum Path {
    Here,
    Any(Vec<(Rule, Option<Path>)>),
    Rel(Rel, Box<Path>),
}

/// Each capture's path from the root (`of`), in the atom's order.
pub struct Binder {
    paths: Vec<Path>,
}

/// Utils inlined at most this deep: a deeper or recursive one is W's.
const DEPTH: usize = 32;

impl Binder {
    /// The paths of `names` in `matcher` (W's rule of the atom), where every rule it reads is the tape's and every
    /// name is bound once; `None` leaves the captures to W.
    pub fn of(matcher: &AstMatcher, names: &[String]) -> Option<Self> {
        if names.is_empty() {
            return None;
        }
        rule(matcher, &mut BTreeSet::new(), 0)?;
        let paths = names.iter().map(|name| path(matcher, name, 0)?).collect::<Option<Vec<_>>>()?;
        Some(Self { paths })
    }

    /// Each capture's node at `root`, a root W's rule matches, into `out`; false where one is not bound there (W
    /// gives that root no row).
    pub fn bind(&self, tape: &TapeDoc, root: u32, out: &mut Vec<u32>) -> bool {
        out.clear();
        for path in &self.paths {
            match path.bind(tape, root) {
                Some(n) => out.push(n),
                None => return false,
            }
        }
        true
    }
}

/// `matcher` as the tape's rule, its names (captures it may bind) added to `names`; `None` where it is not one, or
/// two terms of an `all` mention one name.
fn rule(matcher: &AstMatcher, names: &mut BTreeSet<String>, depth: usize) -> Option<Rule> {
    Some(match matcher {
        AstMatcher::Pattern(pattern) => match &pattern.node {
            PatternNode::MetaVar { meta_var: MetaVariable::Capture(name, named) } => {
                names.insert(name.clone());
                if *named { Rule::Named } else { Rule::True }
            }
            PatternNode::MetaVar { meta_var: MetaVariable::Dropped(named) } => {
                if *named {
                    Rule::Named
                } else {
                    Rule::True
                }
            }
            _ => return None,
        },
        AstMatcher::Kind(kind) => Rule::Kind(kind.potential_kinds()?.iter().next()? as u16),
        AstMatcher::Supertype(supertype) => Rule::Supertype(supertype.class),
        AstMatcher::Regex(regex) => Rule::Regex(regex.clone()),
        AstMatcher::Never => Rule::Never,
        AstMatcher::All(items) => {
            // A name two of its items bind is one W unifies; arms of an `any` around it may bind it too.
            let mut all = Vec::with_capacity(items.len());
            let mut seen = BTreeSet::new();
            for item in items {
                let mut its = BTreeSet::new();
                all.push(rule(item, &mut its, depth)?);
                if !its.is_disjoint(&seen) {
                    return None;
                }
                seen.extend(its);
            }
            names.extend(seen);
            Rule::All(all)
        }
        AstMatcher::Any(arms) => Rule::Any(arms.iter().map(|arm| rule(arm, names, depth)).collect::<Option<_>>()?),
        AstMatcher::Not(inner) => Rule::Not(Box::new(rule(inner, names, depth)?)),
        AstMatcher::Inside(related, stop) | AstMatcher::Has(related, stop) | AstMatcher::Precedes(related, stop) | AstMatcher::Follows(related, stop) => {
            Rule::Rel(Box::new(Rel { dir: dir(matcher), field: None, walk: walk(stop, depth)?, rule: rule(related.rule(), names, depth)? }))
        }
        AstMatcher::Field(relation) => Rule::Rel(Box::new(Rel {
            dir: if relation.inside { Dir::Up } else { Dir::Down },
            field: Some(relation.field),
            walk: walk(&relation.stop, depth)?,
            rule: rule(&relation.rule, names, depth)?,
        })),
        AstMatcher::Util(util) if depth < DEPTH => util.with_target(|target| rule(target, names, depth + 1)).flatten()?,
        _ => return None,
    })
}

/// The way relation `matcher` walks.
fn dir(matcher: &AstMatcher) -> Dir {
    match matcher {
        AstMatcher::Inside(..) => Dir::Up,
        AstMatcher::Has(..) => Dir::Down,
        AstMatcher::Precedes(..) => Dir::Next,
        _ => Dir::Prev,
    }
}

/// A relation's walk; a stop rule matches in an environment of its own, so its names are its own.
fn walk(stop: &StopBy, depth: usize) -> Option<Walk> {
    Some(match stop {
        StopBy::Neighbor => Walk::Neighbor,
        StopBy::End => Walk::End,
        StopBy::Rule(stop) => Walk::Until(rule(stop, &mut BTreeSet::new(), depth)?),
    })
}

/// Where `name` is bound by `matcher` at a node it matched: `Some(None)` where it binds none, `None` where the path
/// is not the tape's (`rule` has accepted the whole of `matcher` before).
fn path(matcher: &AstMatcher, name: &str, depth: usize) -> Option<Option<Path>> {
    Some(match matcher {
        AstMatcher::Pattern(pattern) => match &pattern.node {
            PatternNode::MetaVar { meta_var: MetaVariable::Capture(bound, _) } if bound == name => Some(Path::Here),
            _ => None,
        },
        AstMatcher::All(items) => {
            let mut found = None;
            for item in items {
                if let Some(p) = path(item, name, depth)? {
                    if found.is_some() {
                        return None;
                    }
                    found = Some(p);
                }
            }
            found
        }
        AstMatcher::Any(arms) => {
            let arms: Vec<(Rule, Option<Path>)> = arms.iter().map(|arm| Some((rule(arm, &mut BTreeSet::new(), depth)?, path(arm, name, depth)?))).collect::<Option<_>>()?;
            arms.iter().any(|(_, p)| p.is_some()).then_some(Path::Any(arms))
        }
        AstMatcher::Inside(related, stop) | AstMatcher::Has(related, stop) | AstMatcher::Precedes(related, stop) | AstMatcher::Follows(related, stop) => {
            match path(related.rule(), name, depth)? {
                Some(p) => Some(Path::Rel(Rel { dir: dir(matcher), field: None, walk: walk(stop, depth)?, rule: rule(related.rule(), &mut BTreeSet::new(), depth)? }, Box::new(p))),
                None => None,
            }
        }
        AstMatcher::Field(relation) => match path(&relation.rule, name, depth)? {
            Some(p) => Some(Path::Rel(
                Rel { dir: if relation.inside { Dir::Up } else { Dir::Down }, field: Some(relation.field), walk: walk(&relation.stop, depth)?, rule: rule(&relation.rule, &mut BTreeSet::new(), depth)? },
                Box::new(p),
            )),
            None => None,
        },
        AstMatcher::Util(util) if depth < DEPTH => util.with_target(|target| path(target, name, depth + 1)).flatten()?,
        // `not` keeps what it binds to itself; the rest bind nothing.
        AstMatcher::Not(_) | AstMatcher::Kind(_) | AstMatcher::Supertype(_) | AstMatcher::Regex(_) | AstMatcher::Never => None,
        _ => return None,
    })
}

impl Path {
    fn bind(&self, tape: &TapeDoc, n: u32) -> Option<u32> {
        match self {
            Path::Here => Some(n),
            Path::Any(arms) => {
                let (_, then) = arms.iter().find(|(arm, _)| arm.holds(tape, n))?;
                then.as_ref()?.bind(tape, n)
            }
            Path::Rel(rel, then) => then.bind(tape, rel.find(tape, n)?),
        }
    }
}

impl Rule {
    fn holds(&self, tape: &TapeDoc, n: u32) -> bool {
        match self {
            Rule::True => true,
            Rule::Never => false,
            Rule::Named => tape.transducer_is_named(n),
            Rule::Kind(kind) => tape.transducer_kind(n) == *kind,
            Rule::Supertype(class) => tape.transducer_has_supertype(n, *class),
            Rule::Regex(regex) => regex.is_match(tape.transducer_text_str(n)),
            Rule::All(items) => items.iter().all(|item| item.holds(tape, n)),
            Rule::Any(arms) => arms.iter().any(|arm| arm.holds(tape, n)),
            Rule::Not(inner) => !inner.holds(tape, n),
            Rule::Rel(rel) => rel.find(tape, n).is_some(),
        }
    }
}

/// The first child of `n` in `field`, as `child_by_field_id` takes it.
fn field_child(tape: &TapeDoc, n: u32, field: u16) -> Option<u32> {
    tape.transducer_children(n).find(|&c| tape.transducer_field(c) == field)
}

impl Rel {
    /// The first candidate from `n` the rule holds at, in the walk's order (`relation.rs`: `StopBy::find`,
    /// `FieldRelation::find_inside`, `has`, `has_until`, `FieldRelation::find_has`).
    fn find(&self, tape: &TapeDoc, n: u32) -> Option<u32> {
        let holds = |c: u32| self.rule.holds(tape, c);
        let stops = |c: u32| match &self.walk {
            Walk::Until(stop) => stop.holds(tape, c),
            _ => false,
        };
        let descendants = |c: u32| (c + 1)..tape.transducer_descendants(c).end;
        match (self.dir, self.field) {
            // The siblings as W walks them (`StopBy::find_sibling`): the tape node's own `next`/`prev` and
            // `next_all`/`prev_all` (tree-sitter's byte-positioned neighbor, the walk from the anchor).
            (Dir::Next | Dir::Prev, _) => {
                let node = tape.node(n);
                let forward = self.dir == Dir::Next;
                if let Walk::Neighbor = self.walk {
                    let next = if forward { node.next() } else { node.prev() };
                    return next.map(|s| s.node_id() as u32).filter(|&s| holds(s));
                }
                let all: Box<dyn Iterator<Item = u32>> = if forward {
                    Box::new(node.next_all().map(|s| s.node_id() as u32))
                } else {
                    Box::new(node.prev_all().map(|s| s.node_id() as u32))
                };
                for s in all {
                    if holds(s) {
                        return Some(s);
                    }
                    if stops(s) {
                        return None;
                    }
                }
                None
            }
            (Dir::Up, field) => {
                // An ancestor is a candidate only where the node the walk came from is its first child in the field.
                let mut last = n;
                let mut at = tape.transducer_parent(n);
                while let Some(a) = at {
                    let entered = field.is_none_or(|f| field_child(tape, a, f) == Some(last));
                    if entered && holds(a) {
                        return Some(a);
                    }
                    if matches!(self.walk, Walk::Neighbor) || stops(a) {
                        return None;
                    }
                    last = a;
                    at = tape.transducer_parent(a);
                }
                None
            }
            (Dir::Down, None) => match &self.walk {
                Walk::Neighbor => tape.transducer_children(n).find(|&c| holds(c)),
                Walk::End => descendants(n).find(|&c| holds(c)),
                // Pre-order; a descendant the stop rule holds at is tested, not entered (`descend_until`).
                Walk::Until(_) => {
                    let mut stack: Vec<Vec<u32>> = vec![tape.transducer_children(n).collect::<Vec<_>>().into_iter().rev().collect()];
                    while let Some(level) = stack.last_mut() {
                        let Some(c) = level.pop() else {
                            stack.pop();
                            continue;
                        };
                        if holds(c) {
                            return Some(c);
                        }
                        if !stops(c) {
                            stack.push(tape.transducer_children(c).collect::<Vec<_>>().into_iter().rev().collect());
                        }
                    }
                    None
                }
            },
            (Dir::Down, Some(field)) => {
                let child = field_child(tape, n, field)?;
                match &self.walk {
                    Walk::Neighbor => holds(child).then_some(child),
                    Walk::End => std::iter::once(child).chain(descendants(child)).find(|&c| holds(c)),
                    // The field child, else (where it does not stop the walk) its children only.
                    Walk::Until(_) if holds(child) => Some(child),
                    Walk::Until(_) if stops(child) => None,
                    Walk::Until(_) => tape.transducer_children(child).find(|&c| holds(c)),
                }
            }
        }
    }
}
