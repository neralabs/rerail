//! Bounded compile-time attribution; this never runs matcher composition.
use super::Lower;
use crate::code_rule_dsl::matcher::{AstMatcher, Semantic, StopBy, UtilRef};
fn children(m: &AstMatcher) -> Vec<&AstMatcher> {
    match m {
        AstMatcher::All(v) | AstMatcher::Any(v) => v.iter().collect(),
        AstMatcher::Not(m) => vec![m],
        AstMatcher::Inside(c, s)
        | AstMatcher::Has(c, s)
        | AstMatcher::Follows(c, s)
        | AstMatcher::Precedes(c, s) => std::iter::once(c.rule())
            .chain(if let StopBy::Rule(m) = s {
                Some(m.as_ref())
            } else {
                None
            })
            .collect(),
        AstMatcher::Field(f) => std::iter::once(&f.rule)
            .chain(if let StopBy::Rule(m) = &f.stop {
                Some(m.as_ref())
            } else {
                None
            })
            .collect(),
        AstMatcher::Constrained(c) => std::iter::once(&c.rule)
            .chain(c.constraints.iter().map(|(_, m)| m))
            .collect(),
        AstMatcher::Count(c) => vec![&c.relation],
        AstMatcher::NthChild(c) => c.of_rule.iter().collect(),
        AstMatcher::Semantic(s) => match s {
            Semantic::Binding(t) => t.sub_rules().collect(),
            Semantic::Binds(t) => t.sub_rules().collect(),
            Semantic::Parse(t) => vec![&t.rule],
            Semantic::Host(t) => vec![&t.rule],
            Semantic::FormatArgs(t) => [t.unused.as_ref(), t.missing.as_ref()]
                .into_iter()
                .flatten()
                .collect(),
            _ => vec![],
        },
        _ => vec![],
    }
}
fn tag(m: &AstMatcher) -> &'static str {
    match m {
        AstMatcher::All(_) => "all",
        AstMatcher::Any(_) => "any",
        AstMatcher::Not(_) => "not",
        AstMatcher::Util(_) => "utility",
        AstMatcher::Call(_) => "call",
        AstMatcher::Kind(_) => "kind",
        AstMatcher::Regex(_) => "regex",
        AstMatcher::Pattern(_) => "pattern",
        AstMatcher::Inside(..) => "inside",
        AstMatcher::Has(..) => "has",
        AstMatcher::Follows(..) => "follows",
        AstMatcher::Precedes(..) => "precedes",
        AstMatcher::Field(_) => "field",
        AstMatcher::Count(_) => "count",
        AstMatcher::NthChild(_) => "nth",
        AstMatcher::Constrained(_) => "constrained",
        AstMatcher::Semantic(Semantic::Binding(_)) => "binding",
        AstMatcher::Semantic(Semantic::Value(_)) => "value",
        AstMatcher::Semantic(Semantic::Resolves(_)) => "resolves",
        AstMatcher::Semantic(_) => "semantic",
        _ => "other",
    }
}
fn inventory(
    m: &AstMatcher,
    owner: &str,
    seen: &mut std::collections::HashSet<(usize, usize)>,
    result: &mut std::collections::BTreeMap<String, usize>,
    remaining: &mut usize,
) {
    if *remaining == 0 {
        return;
    }
    *remaining -= 1;
    *result.entry(format!("{owner}/{}", tag(m))).or_default() += 1;
    let reference = match m {
        AstMatcher::Util(u) => Some(u),
        AstMatcher::Call(c) => Some(&c.util),
        _ => None,
    };
    if let Some(u) = reference {
        let key = (u.table.as_ptr() as usize, u.index);
        if seen.len() < 128 && seen.insert(key) {
            let name = u
                .table
                .upgrade()
                .and_then(|t| t.names.get(u.index).cloned())
                .unwrap_or_else(|| format!("utility{}", u.index));
            let _ = u.with_target(|body| inventory(body, &name, seen, result, remaining));
        }
    }
    for child in children(m) {
        inventory(child, owner, seen, result, remaining);
    }
}
impl Lower {
    fn finite_own(&mut self, m: &AstMatcher, name: &str) {
        self.finite_owners
            .insert(std::ptr::from_ref(m) as usize, name.to_owned());
        for child in children(m) {
            self.finite_own(child, name);
        }
    }
    pub(super) fn finite_register_source(&mut self, util: &UtilRef) {
        if !self.finite_mode {
            return;
        }
        let name = util
            .table
            .upgrade()
            .and_then(|t| t.names.get(util.index).cloned())
            .unwrap_or_else(|| format!("utility{}", util.index));
        self.finite_names
            .insert((util.table.as_ptr() as usize, util.index), name.clone());
        let _ = util.with_target(|m| self.finite_own(m, &name));
    }
    pub(super) fn finite_visit(&mut self, m: &AstMatcher) {
        if !self.finite_mode {
            return;
        }
        if self.finite_owners.is_empty() {
            self.finite_own(m, "root");
            inventory(
                m,
                "root",
                &mut Default::default(),
                &mut self.finite_inventory,
                &mut 8192,
            );
        }
        let address = std::ptr::from_ref(m) as usize;
        if !self.finite_seen.insert(address) {
            self.finite_repeated += 1;
        }
        let owner = self
            .finite_owners
            .get(&address)
            .map(String::as_str)
            .unwrap_or("speculative-clone");
        *self
            .finite_visits
            .entry(format!("{owner}/{}", tag(m)))
            .or_default() += 1;
    }
}
