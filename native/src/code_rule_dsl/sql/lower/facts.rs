//! A capture-free, two-valued sub-rule whose regexes are holes of a shared literal fact
//! (`sql::facts`), lowered as its member: `Op::InFact`, the rows at whose node the fact's member
//! holds, instead of the sub-rule's own operators on the frame.
//!
//! A sub-rule is a member where it is a *spine* down to its regexes: `all`, `any` (of terms with
//! regexes only), `has`/`inside` with a neighbor or end stop, and their `field` forms with a
//! neighbor stop, each walk with a sub-rule holding regexes; everything else on the way is a
//! literal-free test, a kind or the stage of its rule (`Lowerer::stage`). It must have root kinds
//! (the fact starts from every node of them) and every regex must lie below a walk
//! (`Draft::anchored`), and nothing may read the row's bindings or its exclusions: no capture, no
//! unknown verdict, not in a parsed tree, and not where `violation.each` excludes the enumerated
//! capture's binding (relations and `any` branches then pass over candidates by the row's
//! binding, `Lowerer::excluding`).

use super::{Lowerer, kind_id};
use crate::code_rule_dsl::matcher::{AstMatcher, StopBy};
use crate::code_rule_dsl::sql::facts::{Axis, Filter, Spine};
use crate::code_rule_dsl::sql::plan::{Op, SlotKind};
use crate::code_rule_dsl::sql::recursion;
use regex::Regex;

/// What a sub-rule is to a fact.
enum Class<'m> {
    /// It holds no hole: a literal-free test.
    Free,
    /// A spine down to holes.
    Spine(Draft<'m>),
    /// It holds holes where no spine reaches them.
    Refused,
}

/// A spine whose literal-free tests are not lowered yet (`Lowerer::build`).
enum Draft<'m> {
    Hole(usize),
    All {
        filters: Vec<&'m AstMatcher>,
        items: Vec<Draft<'m>>,
    },
    Any(Vec<Draft<'m>>),
    Walk {
        axis: Axis,
        kinds: Option<Vec<u16>>,
        sub: Box<Draft<'m>>,
    },
}

impl Draft<'_> {
    /// Whether every hole lies below a walk from the rule's node. A fact starts from every node
    /// of its rule's root kinds: a hole at the node itself (a kind and a regex, an `any` of the
    /// names of an identifier or an attribute) would test the text of every such node of the
    /// batch, where a clause's own frame tests the few its walks reach.
    fn anchored(&self) -> bool {
        match self {
            Self::Hole(_) => false,
            Self::All { items, .. } | Self::Any(items) => items.iter().all(Self::anchored),
            Self::Walk { .. } => true,
        }
    }

    /// Whether its literal-free tests are all node-local (`node_local`): then the whole rule is
    /// one transducer pushdown compiles.
    fn pushable(&self) -> bool {
        match self {
            Self::Hole(_) => true,
            Self::All { filters, items } => {
                filters.iter().all(|filter| node_local(filter, 0)) && items.iter().all(Self::pushable)
            }
            Self::Any(items) => items.iter().all(Self::pushable),
            Self::Walk { sub, .. } => sub.pushable(),
        }
    }
}

/// Whether `rule` is what `pushdown::rule` takes once lowered: kinds, regexes, `all`/`any`/`not`
/// of such rules, walks with a neighbor or end stop (`field` forms too) and counts without `as`
/// over them, through the utilities it reaches (lowered inline, as one reference lowers them;
/// `depth` bounds a recursive one, which is a unit, never a program).
fn node_local(rule: &AstMatcher, depth: usize) -> bool {
    if depth > 64 {
        return false;
    }
    let local = |rule: &AstMatcher| node_local(rule, depth + 1);
    match rule {
        AstMatcher::Kind(_) | AstMatcher::Regex(_) | AstMatcher::Never => true,
        AstMatcher::All(items) | AstMatcher::Any(items) => items.iter().all(local),
        AstMatcher::Not(inner) => local(inner),
        AstMatcher::Has(related, stop) | AstMatcher::Inside(related, stop) => {
            matches!(stop, StopBy::Neighbor | StopBy::End) && local(related.rule())
        }
        AstMatcher::Field(relation) => {
            matches!(relation.stop, StopBy::Neighbor | StopBy::End) && local(&relation.rule)
        }
        AstMatcher::Count(count) => {
            count.binding.is_none()
                && matches!(
                    &count.relation,
                    AstMatcher::Has(_, StopBy::Neighbor | StopBy::End)
                        | AstMatcher::Inside(_, StopBy::Neighbor | StopBy::End)
                )
                && local(&count.relation)
        }
        AstMatcher::Util(util) => util.with_target(local).unwrap_or(false),
        _ => false,
    }
}

impl Lowerer<'_> {
    /// `rule` as a member of its shared literal fact (`Op::InFact`), where it is one here.
    pub(super) fn shared_fact(&mut self, rule: &AstMatcher, scope: usize) -> Option<Op> {
        if self.tree.is_some()
            || self.program.census.is_some()
            || !matches!(
                rule,
                AstMatcher::All(_)
                    | AstMatcher::Any(_)
                    | AstMatcher::Has(..)
                    | AstMatcher::Inside(..)
                    | AstMatcher::Field(_)
            )
            || self.excludes_here(scope)
        {
            return None;
        }
        let key = (std::ptr::from_ref(rule) as usize, self.dialect.to_string());
        let known = self.program.fact_rules.get(&key).copied();
        if known == Some(None) {
            return None;
        }
        // A stage cannot stand for a rule reaching back into the recursion being lowered.
        if self
            .following
            .as_ref()
            .is_some_and(|following| recursion::reaches(rule, &following.keys))
            || recursion::reaches(rule, &self.program.in_progress)
        {
            return None;
        }
        let member = match known {
            Some(member) => member,
            None => {
                let member = self.member(rule);
                self.program.fact_rules.insert(key, member);
                member
            }
        };
        member.map(|(stage, member)| Op::InFact { stage, member })
    }

    /// Whether `violation.each` excludes the enumerated capture's binding here: some slot of it
    /// exists in `scope` (`excluding`).
    fn excludes_here(&self, scope: usize) -> bool {
        self.root.is_some()
            && self.exclude.as_ref().is_some_and(|name| {
                [SlotKind::Node, SlotKind::Text]
                    .into_iter()
                    .any(|kind| self.existing(scope, name, kind).is_some())
            })
    }

    /// `rule`'s fact stage and member number, where it is a member.
    fn member(&mut self, rule: &AstMatcher) -> Option<(usize, usize)> {
        if !self.program.analysis.capture_free(rule) || self.program.analysis.reads_values(rule) {
            return None;
        }
        let roots: Vec<u16> = self
            .root_kinds(rule)?
            .into_iter()
            .collect::<Vec<u16>>();
        if roots.is_empty() {
            return None;
        }
        let mut holes = Vec::new();
        let Class::Spine(draft) = self.class(rule, &mut holes) else {
            return None;
        };
        // A spine all of whose tests are node-local is a transducer program already (`pushdown`,
        // which takes every capture-free rule of a regex under a walk): it is pushdown's.
        if !draft.anchored() || (super::super::pushdown::enabled() && draft.pushable()) {
            return None;
        }
        let spine = self.build(draft)?;
        let program = &mut *self.program;
        Some(
            program
                .facts
                .member(&mut program.stages, self.dialect, roots, spine, holes),
        )
    }

    fn root_kinds(&mut self, rule: &AstMatcher) -> Option<std::collections::BTreeSet<u16>> {
        let three = self.three;
        self.kinds
            .get_or_insert_with(|| super::RootKinds::new(three))
            .of(rule, &mut self.program.analysis)
    }

    /// What `rule` is to a fact, its regexes on the spine appended to `holes`.
    fn class<'m>(&mut self, rule: &'m AstMatcher, holes: &mut Vec<Regex>) -> Class<'m> {
        match rule {
            AstMatcher::Regex(regex) => {
                holes.push(regex.clone());
                Class::Spine(Draft::Hole(holes.len() - 1))
            }
            AstMatcher::All(items) => {
                let (mut filters, mut spines) = (Vec::new(), Vec::new());
                for item in items {
                    match self.class(item, holes) {
                        Class::Free => filters.push(item),
                        Class::Spine(spine) => spines.push(spine),
                        Class::Refused => return Class::Refused,
                    }
                }
                if spines.is_empty() {
                    Class::Free
                } else {
                    Class::Spine(Draft::All {
                        filters,
                        items: spines,
                    })
                }
            }
            AstMatcher::Any(items) => {
                let mut spines = Vec::with_capacity(items.len());
                let mut free = 0;
                for item in items {
                    match self.class(item, holes) {
                        Class::Free => free += 1,
                        Class::Spine(spine) => spines.push(spine),
                        Class::Refused => return Class::Refused,
                    }
                }
                match (free, spines.is_empty()) {
                    (_, true) => Class::Free,
                    // A literal-free branch holds for every member: refused, not a wildcard.
                    (0, false) => Class::Spine(Draft::Any(spines)),
                    _ => Class::Refused,
                }
            }
            AstMatcher::Has(related, stop) | AstMatcher::Inside(related, stop) => {
                let has = matches!(rule, AstMatcher::Has(..));
                let axis = match (has, stop) {
                    (true, StopBy::Neighbor) => Axis::Child,
                    (true, StopBy::End) => Axis::Descendant,
                    (false, StopBy::Neighbor) => Axis::Parent,
                    (false, StopBy::End) => Axis::Ancestor,
                    (_, StopBy::Rule(_)) => return self.free_or_refused(related.rule(), holes),
                };
                self.walk(axis, related.rule(), holes)
            }
            AstMatcher::Field(relation) => match relation.stop {
                StopBy::Neighbor if relation.inside => {
                    self.walk(Axis::FieldParent(relation.field), &relation.rule, holes)
                }
                StopBy::Neighbor => {
                    self.walk(Axis::FieldChild(relation.field), &relation.rule, holes)
                }
                _ => self.free_or_refused(&relation.rule, holes),
            },
            _ => Class::Free,
        }
    }

    /// A walk on `axis` to `sub`: a spine where `sub` is one. An end walk needs its sub-rule's
    /// kinds, or it would list every node below or above.
    fn walk<'m>(&mut self, axis: Axis, sub: &'m AstMatcher, holes: &mut Vec<Regex>) -> Class<'m> {
        match self.class(sub, holes) {
            Class::Spine(spine) => {
                let kinds: Option<Vec<u16>> = self
                    .root_kinds(sub)
                    .map(|kinds| kinds.into_iter().collect());
                let bounded = kinds.as_ref().is_some_and(|kinds| !kinds.is_empty());
                if matches!(axis, Axis::Descendant | Axis::Ancestor) && !bounded {
                    return Class::Refused;
                }
                Class::Spine(Draft::Walk {
                    axis,
                    kinds,
                    sub: Box::new(spine),
                })
            }
            other => other,
        }
    }

    /// A relation no spine takes: literal-free where `sub` is, else refused.
    fn free_or_refused<'m>(&mut self, sub: &'m AstMatcher, holes: &mut Vec<Regex>) -> Class<'m> {
        let before = holes.len();
        let class = self.class(sub, holes);
        holes.truncate(before);
        match class {
            Class::Free => Class::Free,
            _ => Class::Refused,
        }
    }

    /// The spine of `draft`, its literal-free tests lowered: a kind test, or the stage of the
    /// test's rule where that stage is bounded by kinds; `None` where one is not.
    fn build(&mut self, draft: Draft<'_>) -> Option<Spine> {
        Some(match draft {
            Draft::Hole(hole) => Spine::Hole(hole),
            Draft::All { filters, items } => {
                let mut tests = Vec::with_capacity(filters.len());
                for filter in filters {
                    tests.push(self.filter(filter)?);
                }
                let mut spines = Vec::with_capacity(items.len());
                for item in items {
                    spines.push(self.build(item)?);
                }
                Spine::All {
                    filters: tests,
                    items: spines,
                }
            }
            Draft::Any(items) => {
                let mut spines = Vec::with_capacity(items.len());
                for item in items {
                    spines.push(self.build(item)?);
                }
                Spine::Any(spines)
            }
            Draft::Walk { axis, kinds, sub } => Spine::Walk {
                axis,
                kinds,
                sub: Box::new(self.build(*sub)?),
            },
        })
    }

    fn filter(&mut self, rule: &AstMatcher) -> Option<Filter> {
        match rule {
            AstMatcher::Kind(kind) => Some(Filter::Kinds(vec![kind_id(kind)])),
            AstMatcher::Any(items) if items.iter().all(|item| matches!(item, AstMatcher::Kind(_))) => {
                Some(Filter::Kinds(
                    items
                        .iter()
                        .filter_map(|item| match item {
                            AstMatcher::Kind(kind) => Some(kind_id(kind)),
                            _ => None,
                        })
                        .collect(),
                ))
            }
            _ if self.stage_bounded(rule) => self.stage(rule).ok().map(Filter::Set),
            _ => None,
        }
    }
}
