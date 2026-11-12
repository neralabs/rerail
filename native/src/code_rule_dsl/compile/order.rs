//! The order of a rule object's terms by capture dependency: a term that reads a capture a later
//! sibling binds runs after that sibling, so a comparison inside one relation (or a `not`, an
//! `any`) sees what another relation of the same rule object bound.

use crate::code_rule_dsl::json::invalid;
use crate::code_rule_dsl::matcher::{AstMatcher, CaptureAnalysis};
use napi::Result;
use std::collections::BTreeSet;
/// An explicit `all` keeps its written order (ast-grep's, and the sequencing tool capture ordering
/// leaves to authors): an item that reads a capture which only a later item binds would read it
/// unbound, so it is refused, naming the reorder. `context` holds what the rule object's other terms
/// bind, which runs before the `all` (atomic terms first, siblings by capture ordering).
pub fn all_order_conflicts(
    items: &[AstMatcher],
    context: &BTreeSet<String>,
    path: &str,
    captures: &mut CaptureAnalysis,
) -> Vec<AllOrderConflict> {
    let binds = items
        .iter()
        .map(|item| captures.binds(item))
        .collect::<Vec<_>>();
    let mut conflicts = Vec::new();
    let mut bound = context.clone();
    for (index, item) in items.iter().enumerate() {
        for name in captures.reads(item).difference(&bound) {
            let later = binds[index + 1..]
                .iter()
                .position(|later| later.contains(name));
            if let Some(later) = later {
                conflicts.push(AllOrderConflict {
                    path: path.to_owned(),
                    item: index,
                    later: index + 1 + later,
                    name: name.clone(),
                });
            }
        }
        bound.extend(binds[index].iter().cloned());
    }
    conflicts
}

/// An `all` item reading `name`, which a later item binds: refused unless something outside those
/// items (an enclosing rule, a caller) binds it too, which the query checks once it is compiled.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct AllOrderConflict {
    /// The rule object holding the `all`.
    pub(in crate::code_rule_dsl) path: String,
    pub(in crate::code_rule_dsl) item: usize,
    pub(in crate::code_rule_dsl) later: usize,
    pub(in crate::code_rule_dsl) name: String,
}

impl AllOrderConflict {
    pub(in crate::code_rule_dsl) fn error(&self) -> napi::Error {
        let (path, item, later, name) = (&self.path, self.item, self.later, &self.name);
        invalid(format!(
            "{path}/all/{item} reads `${name}`, which only {path}/all/{later} binds, after it: \
             an `all` runs its items in the order written; move all/{later} before all/{item}"
        ))
    }
}

/// Orders `terms[from..]`, each with its path, so that a term reading a capture runs after every
/// sibling that binds it, unless a term before `from` binds it already. Terms keep their order
/// otherwise. A cycle (two terms each reading what the other binds) is a compile-time error.
pub fn order_by_captures(
    terms: &mut Vec<(AstMatcher, String)>,
    from: usize,
    path: &str,
    captures: &mut CaptureAnalysis,
) -> Result<()> {
    let early = terms[..from]
        .iter()
        .flat_map(|(term, _)| captures.binds(term))
        .collect::<BTreeSet<_>>();
    let group = &terms[from..];
    let bound = group
        .iter()
        .map(|(term, _)| captures.binds(term))
        .collect::<Vec<_>>();
    // `reads` already leaves out what a term binds for itself (an `any` binds only in the branch
    // that matches, so its other branches still read).
    let needs = group
        .iter()
        .map(|(term, _)| {
            captures
                .reads(term)
                .into_iter()
                .filter(|name| !early.contains(name))
                .collect::<BTreeSet<_>>()
        })
        .collect::<Vec<_>>();
    // `after[i]`: the siblings that must run before term `i`, with a name each binds for it.
    let after = needs
        .iter()
        .enumerate()
        .map(|(index, need)| {
            bound
                .iter()
                .enumerate()
                .filter(|(other, _)| *other != index)
                .filter_map(|(other, binds)| {
                    need.iter()
                        .find(|name| binds.contains(*name))
                        .map(|name| (other, name.clone()))
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    if after.iter().all(Vec::is_empty) {
        return Ok(());
    }
    let mut placed = vec![false; group.len()];
    let mut order = Vec::with_capacity(group.len());
    while order.len() < group.len() {
        let Some(next) = (0..group.len()).find(|index| {
            !placed[*index] && after[*index].iter().all(|(before, _)| placed[*before])
        }) else {
            let stuck = (0..group.len())
                .find(|index| !placed[*index])
                .expect("an unplaced term");
            let (other, name) = after[stuck]
                .iter()
                .find(|(before, _)| !placed[*before])
                .expect("an unplaced dependency");
            let back = after[*other]
                .iter()
                .find(|(before, _)| !placed[*before])
                .map_or_else(String::new, |(_, name)| format!("`${name}`"));
            return Err(invalid(format!(
                "{path}: {} reads `${name}`, which {} binds, and {} reads {back}, which {} binds; \
                 no order binds both first. Bind one of them in an earlier term (a pattern, or an \
                 `all` whose first item binds it)",
                group[stuck].1, group[*other].1, group[*other].1, group[stuck].1
            )));
        };
        placed[next] = true;
        order.push(next);
    }
    let mut taken = terms
        .split_off(from)
        .into_iter()
        .map(Some)
        .collect::<Vec<_>>();
    for index in order {
        terms.push(taken[index].take().expect("each term once"));
    }
    Ok(())
}

// tests: a test the engine hosts (it reads above this crate; tools/cratify/test-hosts.tsv)
