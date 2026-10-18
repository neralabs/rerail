//! Where a rule reads `calls`: a rule that reads it only positively (never under `not`, a stop
//! rule, a count or a position) matches nothing in a file where every `calls` is false.

use super::super::{AstMatcher, Semantic, stop_rule};
use std::collections::BTreeSet;

/// Whether every `calls` term of `matcher` is read positively: false everywhere, the rule matches
/// nowhere a `calls` term decides.
pub(crate) fn calls_positive_only(matcher: &AstMatcher) -> bool {
    let mut polarity = Polarity::default();
    walk(matcher, false, &mut BTreeSet::new(), &mut polarity);
    polarity.calls && !polarity.negative
}

#[derive(Default)]
struct Polarity {
    /// A `calls` term was found.
    calls: bool,
    /// One is read negatively, or where both readings matter.
    negative: bool,
}

/// Walks `matcher`'s terms. `negated` flips at `not`; a stop rule, a count, a position among
/// siblings and a binding's sub-rules read both ways (their false answers can make a match).
fn walk(matcher: &AstMatcher, negated: bool, seen: &mut BTreeSet<usize>, out: &mut Polarity) {
    let both = |item: &AstMatcher, seen: &mut BTreeSet<usize>, out: &mut Polarity| {
        walk(item, false, seen, out);
        walk(item, true, seen, out);
    };
    match matcher {
        AstMatcher::Semantic(Semantic::Calls(_)) => {
            out.calls = true;
            out.negative |= negated;
        }
        AstMatcher::All(items) | AstMatcher::Any(items) => {
            for item in items {
                walk(item, negated, seen, out);
            }
        }
        AstMatcher::Not(item) => walk(item, !negated, seen, out),
        AstMatcher::Inside(item, stop)
        | AstMatcher::Has(item, stop)
        | AstMatcher::Precedes(item, stop)
        | AstMatcher::Follows(item, stop) => {
            walk(item, negated, seen, out);
            if let Some(stop) = stop_rule(stop) {
                both(stop, seen, out);
            }
        }
        AstMatcher::Field(relation) => {
            walk(&relation.rule, negated, seen, out);
            if let Some(stop) = stop_rule(&relation.stop) {
                both(stop, seen, out);
            }
        }
        AstMatcher::Count(count) => both(&count.relation, seen, out),
        AstMatcher::NthChild(nth) => {
            if let Some(rule) = &nth.of_rule {
                both(rule, seen, out);
            }
        }
        AstMatcher::Constrained(core) => {
            walk(&core.rule, negated, seen, out);
            for (_, constraint) in &core.constraints {
                walk(constraint, negated, seen, out);
            }
        }
        AstMatcher::Util(util) => {
            if seen.insert(util.key() * 2 + usize::from(negated)) {
                util.with_target(|rule| walk(rule, negated, seen, out));
            }
        }
        AstMatcher::Call(call) => {
            if seen.insert(call.util.key() * 2 + usize::from(negated)) {
                call.util.with_target(|rule| walk(rule, negated, seen, out));
            }
        }
        AstMatcher::Semantic(Semantic::Binding(test)) => {
            for rule in test.sub_rules() {
                both(rule, seen, out);
            }
        }
        AstMatcher::Semantic(Semantic::Binds(test)) => {
            for rule in test.sub_rules() {
                both(rule, seen, out);
            }
        }
        AstMatcher::Semantic(Semantic::FormatArgs(test)) => {
            if let Some(unused) = &test.unused {
                both(unused, seen, out);
            }
        }
        AstMatcher::Semantic(Semantic::Parse(test)) => both(&test.rule, seen, out),
        AstMatcher::Semantic(Semantic::Host(test)) => both(&test.rule, seen, out),
        AstMatcher::Pattern(_)
        | AstMatcher::Kind(_)
        | AstMatcher::Regex(_)
        | AstMatcher::Compare(_)
        | AstMatcher::Semantic(Semantic::Resolves(_))
        | AstMatcher::Semantic(Semantic::Value(_))
        | AstMatcher::Semantic(Semantic::Arguments(_))
        | AstMatcher::Semantic(Semantic::Capture(_))
        | AstMatcher::Semantic(Semantic::Layout(_))
        | AstMatcher::Never => {}
        #[cfg(test)]
        AstMatcher::TestUnknown(_) => {}
    }
}
