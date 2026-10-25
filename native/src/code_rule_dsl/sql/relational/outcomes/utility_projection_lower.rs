//! Immutable, capture-free utility projection IR. Utility and traversal
//! boundaries stay explicit; Boolean composition is a scalar projection.
use super::super::{ir::Set, lower::Lowerer};
use super::{Axis, Op, Plan, WalkFilter};
use crate::code_rule_dsl::matcher::{AstMatcher, StopBy, UtilRef};
use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};
pub(super) type Id = usize;
#[derive(Clone)]
pub(super) enum Expr {
    Atom(Set),
    All(Vec<Id>),
    Any(Vec<Id>),
    Not(Id),
    Call(usize),
    Search {
        axis: Axis,
        field: Option<u16>,
        body: Id,
        filter: Option<WalkFilter>,
    },
}
pub(super) struct Function {
    pub utility: usize,
    pub body: Id,
}
pub(super) struct Program {
    pub expressions: Vec<Expr>,
    pub functions: Vec<Function>,
    pub entry: Id,
    pub original: Option<Plan>,
    pub source_operators: usize,
}
impl Program {
    pub fn has_calls(&self) -> bool {
        !self.functions.is_empty()
    }
    /// Scalar completion is NOT permission to omit native memo publication.
    /// Consumers must use reached call recipes, or keep the original frontier.
    pub fn requires_completion_recipe(&self) -> bool {
        self.has_calls()
    }
}
fn set_supported(s: &Set) -> bool {
    match s {
        Set::AllNodes | Set::Empty | Set::Kinds(_) | Set::Regex(_) | Set::Nonempty => true,
        Set::Intersect(v) | Set::Union(v) => v.iter().all(set_supported),
        Set::Complement(s) => set_supported(s),
        _ => false,
    }
}
#[derive(Default)]
struct Builder {
    expressions: Vec<Expr>,
    functions: Vec<Function>,
    functions_by_key: HashMap<(usize, usize), usize>,
    active: HashSet<(usize, usize)>,
    shared: HashMap<usize, Id>,
    source: usize,
}
impl Builder {
    fn push(&mut self, e: Expr) -> Result<Id, String> {
        self.source += 1;
        if self.source > 256 {
            return Err("pure projection source operators exceed256".into());
        }
        let id = self.expressions.len();
        self.expressions.push(e);
        Ok(id)
    }
    fn call(
        &mut self,
        key: (usize, usize),
        utility: usize,
        body: impl FnOnce(&mut Self) -> Result<Id, String>,
    ) -> Result<Id, String> {
        if self.active.contains(&key) {
            return Err("pure projection requires acyclic utility bodies".into());
        }
        let function = if let Some(&f) = self.functions_by_key.get(&key) {
            f
        } else {
            let f = self.functions.len();
            self.functions.push(Function {
                utility,
                body: usize::MAX,
            });
            self.functions_by_key.insert(key, f);
            self.active.insert(key);
            let entry = body(self)?;
            self.active.remove(&key);
            self.functions[f].body = entry;
            f
        };
        self.push(Expr::Call(function))
    }
    fn plan(&mut self, p: &Plan) -> Result<Id, String> {
        if !p.mentions.is_empty()
            || !p.writes.is_empty()
            || p.effects.reads_value
            || p.effects.cache_fills
            || p.effects.sibling_cache
        {
            return Err("projection cannot cross capture/value/cache-fill effects".into());
        }
        let e = match &p.op {
            Op::Predicate(s) if set_supported(s) => Expr::Atom(s.clone()),
            Op::All(v) | Op::Any(v) => {
                let children = v
                    .iter()
                    .map(|p| self.plan(p))
                    .collect::<Result<Vec<_>, _>>()?;
                if matches!(p.op, Op::All(_)) {
                    Expr::All(children)
                } else {
                    Expr::Any(children)
                }
            }
            Op::Not(p) => Expr::Not(self.plan(p)?),
            Op::Shared(p) => {
                let key = Rc::as_ptr(p) as usize;
                if let Some(&id) = self.shared.get(&key) {
                    return Ok(id);
                }
                let id = self.plan(p)?;
                self.shared.insert(key, id);
                return Ok(id);
            }
            Op::Charge(id, body) => {
                return self.call((Rc::as_ptr(body) as usize, *id), *id, |b| b.plan(body));
            }
            Op::Search(axis, false, body, None) => Expr::Search {
                axis: *axis,
                field: None,
                body: self.plan(body)?,
                filter: body.walk_filter.clone(),
            },
            Op::FieldNeighbor {
                inside,
                field,
                child,
            } => Expr::Search {
                axis: if *inside { Axis::Parent } else { Axis::Child },
                field: Some(*field),
                body: self.plan(child)?,
                filter: child.walk_filter.clone(),
            },
            _ => return Err("projection operator requires original effect frontier".into()),
        };
        self.push(e)
    }
    fn ast(
        &mut self,
        m: &AstMatcher,
        lower: &mut Lowerer,
        three: bool,
        identity: &mut impl FnMut(&UtilRef) -> usize,
    ) -> Result<Id, String> {
        let e = match m {
            AstMatcher::Kind(_) | AstMatcher::Regex(_) | AstMatcher::Never => {
                Expr::Atom(lower.set(m)?)
            }
            AstMatcher::All(v) | AstMatcher::Any(v) => {
                let c = v
                    .iter()
                    .map(|m| self.ast(m, lower, three, identity))
                    .collect::<Result<Vec<_>, _>>()?;
                if matches!(m, AstMatcher::All(_)) {
                    Expr::All(c)
                } else {
                    Expr::Any(c)
                }
            }
            AstMatcher::Not(m) => Expr::Not(self.ast(m, lower, three, identity)?),
            AstMatcher::Util(u) => {
                let key = (u.table.as_ptr() as usize, u.index);
                let uid = identity(u);
                return self.call(key, uid, |b| {
                    u.with_target(|m| b.ast(m, lower, three, identity))
                        .ok_or("utility source unavailable")?
                });
            }
            AstMatcher::Has(r, StopBy::Neighbor) => {
                let filter = if three {
                    None
                } else {
                    crate::code_rule_dsl::matcher::walk_candidate_filter(r.rule()).map(
                        |(kinds, _)| WalkFilter {
                            kinds,
                            text: vec![],
                        },
                    )
                };
                Expr::Search {
                    axis: Axis::Child,
                    field: None,
                    body: self.ast(r.rule(), lower, three, identity)?,
                    filter,
                }
            }
            _ => return Err("projection source outside immutable family".into()),
        };
        self.push(e)
    }
    fn finish(self, entry: Id, original: Option<Plan>) -> Program {
        Program {
            expressions: self.expressions,
            functions: self.functions,
            entry,
            original,
            source_operators: self.source,
        }
    }
}
pub(super) fn lower_plan(plan: &Plan) -> Option<Program> {
    let mut b = Builder::default();
    let entry = b.plan(plan).ok()?;
    Some(b.finish(entry, Some(plan.clone())))
}
/// Admission is a read-only first pass. Unsupported sources do not allocate
/// terminals or utility identities in the caller's normal Lower context.
pub(super) fn lower_ast(
    m: &AstMatcher,
    lower: &mut Lowerer,
    three: bool,
    mut identity: impl FnMut(&UtilRef) -> usize,
) -> Result<Option<Program>, String> {
    fn proof(
        m: &AstMatcher,
        active: &mut HashSet<(usize, usize)>,
        done: &mut HashSet<(usize, usize)>,
        count: &mut usize,
    ) -> bool {
        *count += 1;
        if *count > 256 {
            return false;
        }
        match m {
            AstMatcher::Kind(_) | AstMatcher::Never => true,
            AstMatcher::Regex(r) => r.capture_names().all(|n| n.is_none()),
            AstMatcher::All(v) | AstMatcher::Any(v) => {
                v.iter().all(|m| proof(m, active, done, count))
            }
            AstMatcher::Not(m) => proof(m, active, done, count),
            AstMatcher::Has(r, StopBy::Neighbor) => proof(r.rule(), active, done, count),
            AstMatcher::Util(u) => {
                let key = (u.table.as_ptr() as usize, u.index);
                if active.contains(&key) {
                    return false;
                }
                if done.contains(&key) {
                    return true;
                }
                active.insert(key);
                let ok = u
                    .with_target(|m| proof(m, active, done, count))
                    .unwrap_or(false);
                active.remove(&key);
                done.insert(key);
                ok
            }
            _ => false,
        }
    }
    if !proof(m, &mut HashSet::new(), &mut HashSet::new(), &mut 0) {
        return Ok(None);
    }
    let mut b = Builder::default();
    let entry = b.ast(m, lower, three, &mut identity)?;
    Ok(Some(b.finish(entry, None)))
}

/// Merge eligible immutable bodies using the same utility/source identity table.
/// The caller retains each body's ordinary entry/memo frame.
pub(super) fn lower_forest(plans: &[&Plan]) -> Option<(Program, Vec<Id>)> {
    let mut b = Builder::default();
    let mut entries = Vec::new();
    for plan in plans {
        entries.push(b.plan(plan).ok()?);
    }
    let entry = *entries.first()?;
    Some((b.finish(entry, None), entries))
}
