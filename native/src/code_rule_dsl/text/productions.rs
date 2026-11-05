//! Lowering productions: a canonical pattern to a plan. Each production either accepts a pattern
//! (giving a plan exact for every text of the domain) or declines. They are tried in order, the
//! cheapest plan first, and the first that accepts wins. `Automaton` accepts every regular
//! pattern, so the list always ends with a plan.
//!
//! A new production is a `Production` placed in `PRODUCTIONS` before the ones it is cheaper than.
//! It must be exact, and a production test (`text::tests`) states what it accepts and declines.

use super::canon::{Domain, Pattern};
use super::plan::{Anchor, Plan};
use regex_syntax::hir::HirKind;
use regex_syntax::hir::literal::{ExtractKind, Extractor};

pub(in crate::code_rule_dsl) trait Production: Sync {
    fn name(&self) -> &'static str;
    fn lower(&self, pattern: &Pattern, domain: Domain) -> Option<Plan>;
}

/// The productions, cheapest plan first.
pub(in crate::code_rule_dsl) static PRODUCTIONS: &[&dyn Production] = &[&Never, &Always, &Literals, &Automaton];

/// The first production that accepts `pattern`, and its plan.
pub(in crate::code_rule_dsl) fn lower(pattern: &Pattern, domain: Domain) -> (Plan, &'static str) {
    PRODUCTIONS
        .iter()
        .find_map(|production| production.lower(pattern, domain).map(|plan| (plan, production.name())))
        .expect("Automaton accepts every regular pattern")
}

/// A pattern of an empty language (an empty class, `Hir::fail`).
struct Never;

impl Production for Never {
    fn name(&self) -> &'static str {
        "never"
    }

    fn lower(&self, pattern: &Pattern, _: Domain) -> Option<Plan> {
        pattern.core.properties().minimum_len().is_none().then_some(Plan::Never)
    }
}

/// An unanchored pattern that matches the empty string, with no assertion: every text holds a match.
struct Always;

impl Production for Always {
    fn name(&self) -> &'static str {
        "always"
    }

    fn lower(&self, pattern: &Pattern, _: Domain) -> Option<Plan> {
        let free = !pattern.start && !pattern.end && !pattern.looks();
        (free && pattern.core.properties().minimum_len() == Some(0)).then_some(Plan::Always)
    }
}

/// The literals the extractor can list without approximation.
const LITERAL_LIMIT: usize = 4096;

/// A core whose language is a finite set of strings (with no assertion inside it): a literal set
/// at the pattern's anchor. A set holding the empty string matches every text unless the whole text
/// must match.
struct Literals;

impl Production for Literals {
    fn name(&self) -> &'static str {
        "literals"
    }

    fn lower(&self, pattern: &Pattern, _: Domain) -> Option<Plan> {
        if pattern.looks() {
            return None;
        }
        // An empty core is the empty string: `\A\z` is the empty text, `\A` alone every text.
        if matches!(pattern.core.kind(), HirKind::Empty) {
            return Some(match Anchor::of(pattern.start, pattern.end) {
                Anchor::Whole => Plan::Literals { anchor: Anchor::Whole, literals: vec![Vec::new()] },
                _ => Plan::Always,
            });
        }
        let mut extractor = Extractor::new();
        extractor.kind(ExtractKind::Prefix).limit_total(LITERAL_LIMIT).limit_class(64).limit_repeat(16).limit_literal_len(512);
        let sequence = extractor.extract(&pattern.core);
        let literals = sequence.literals()?;
        if literals.is_empty() || !literals.iter().all(|literal| literal.is_exact()) {
            return None;
        }
        let mut literals = literals.iter().map(|literal| literal.as_bytes().to_vec()).collect::<Vec<_>>();
        literals.sort();
        literals.dedup();
        let anchor = Anchor::of(pattern.start, pattern.end);
        if anchor != Anchor::Whole && literals.iter().any(Vec::is_empty) {
            return Some(Plan::Always);
        }
        Some(Plan::Literals { anchor, literals: minimal(anchor, literals) })
    }
}

/// The literals that decide the set at `anchor`: for a prefix set, a literal that extends another
/// adds nothing (`ab` after `a`); likewise for suffixes and substrings.
fn minimal(anchor: Anchor, literals: Vec<Vec<u8>>) -> Vec<Vec<u8>> {
    let implied = |longer: &[u8], shorter: &[u8]| {
        longer.len() > shorter.len()
            && match anchor {
                Anchor::Whole => false,
                Anchor::Start => longer.starts_with(shorter),
                Anchor::End => longer.ends_with(shorter),
                Anchor::Within => longer.windows(shorter.len()).any(|window| window == shorter),
            }
    };
    if anchor == Anchor::Whole || literals.len() > 512 {
        return literals;
    }
    literals.iter().filter(|literal| !literals.iter().any(|other| implied(literal, other))).cloned().collect()
}

/// Any regular pattern: an automaton at the pattern's anchor.
struct Automaton;

impl Production for Automaton {
    fn name(&self) -> &'static str {
        "automaton"
    }

    fn lower(&self, pattern: &Pattern, _: Domain) -> Option<Plan> {
        Some(Plan::Automaton { anchor: Anchor::of(pattern.start, pattern.end), core: pattern.core.clone() })
    }
}
