//! The canonical pattern: a core with its whole-text anchors split out, and the rewrites that bring
//! a pattern to it.
//!
//! A rewrite is a sound simplification of a pattern over a domain's text: the rewritten pattern
//! matches exactly the same strings of that domain. Rewrites are applied in list order to a fixpoint
//! (`normalize`). A new rewrite is a `Rewrite` added to `REWRITES`; one that holds only for some
//! texts reads `Domain` and declines elsewhere.

use regex_syntax::hir::{Capture, Class, Hir, HirKind, Look, Repetition};

/// What is known of the text a test reads. A field is a fact rewrites may rely on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Domain {
    /// The text never holds a line break (identifiers, names, paths).
    pub(in crate::code_rule_dsl) single_line: bool,
}

impl Domain {
    /// Any text: a node's, a file's, a string literal's.
    pub const ANY: Self = Self { single_line: false };
    /// One line at most: an identifier, a dotted name, a path.
    #[allow(dead_code, reason = "the framework's API: no caller knows its text is one line yet")]
    pub(in crate::code_rule_dsl) const LINE: Self = Self { single_line: true };
}

/// `\A`? `core` `\z`?: the whole text's anchors, and what lies between them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    pub(in crate::code_rule_dsl) start: bool,
    pub(in crate::code_rule_dsl) core: Hir,
    pub(in crate::code_rule_dsl) end: bool,
}

impl Pattern {
    /// The pattern of a parsed regex: unanchored where its anchors are inside it.
    pub(in crate::code_rule_dsl) fn of(hir: Hir) -> Self {
        Self { start: false, core: hir, end: false }
    }

    /// Whether the core asserts anything about positions (a word boundary, a line's start).
    pub(in crate::code_rule_dsl) fn looks(&self) -> bool {
        !self.core.properties().look_set().is_empty()
    }
}

/// A sound simplification of a pattern over `domain`'s text; `None` where it does not apply.
pub(in crate::code_rule_dsl) trait Rewrite: Sync {
    fn name(&self) -> &'static str;
    fn apply(&self, pattern: &Pattern, domain: Domain) -> Option<Pattern>;
}

/// The rewrites, in the order they are tried.
pub(in crate::code_rule_dsl) static REWRITES: &[&dyn Rewrite] = &[&Captures, &Anchors, &EdgeAnything, &Contradiction];

/// `pattern` rewritten to a fixpoint, and the rewrites that changed it.
pub(in crate::code_rule_dsl) fn normalize(mut pattern: Pattern, domain: Domain) -> (Pattern, Vec<&'static str>) {
    let mut applied = Vec::new();
    // Each rewrite shrinks the pattern or moves an anchor out of it; the bound is a guard.
    for _ in 0..64 {
        let Some((rewrite, next)) = REWRITES.iter().find_map(|rewrite| rewrite.apply(&pattern, domain).filter(|next| *next != pattern).map(|next| (rewrite.name(), next))) else {
            break;
        };
        applied.push(rewrite);
        pattern = next;
    }
    (pattern, applied)
}

/// Groups capture nothing a text test reads: `(a|b)` is `(?:a|b)`.
struct Captures;

impl Rewrite for Captures {
    fn name(&self) -> &'static str {
        "captures"
    }

    fn apply(&self, pattern: &Pattern, _: Domain) -> Option<Pattern> {
        fn strip(hir: &Hir) -> Hir {
            match hir.kind() {
                HirKind::Capture(Capture { sub, .. }) => strip(sub),
                HirKind::Concat(parts) => Hir::concat(parts.iter().map(strip).collect()),
                HirKind::Alternation(parts) => Hir::alternation(parts.iter().map(strip).collect()),
                HirKind::Repetition(r) => Hir::repetition(Repetition { min: r.min, max: r.max, greedy: r.greedy, sub: Box::new(strip(&r.sub)) }),
                _ => hir.clone(),
            }
        }
        let has = |hir: &Hir| hir.properties().explicit_captures_len() > 0;
        has(&pattern.core).then(|| Pattern { core: strip(&pattern.core), ..pattern.clone() })
    }
}

/// `\A` and `\z` at the core's edges, or at the edges of every branch of a top-level alternation,
/// move out of the core.
struct Anchors;

impl Rewrite for Anchors {
    fn name(&self) -> &'static str {
        "anchors"
    }

    fn apply(&self, pattern: &Pattern, _: Domain) -> Option<Pattern> {
        let (start, core, end) = split(&pattern.core);
        (start || end).then(|| Pattern { start: pattern.start || start, core, end: pattern.end || end })
    }
}

/// The anchors at `hir`'s edges (in every branch), and `hir` without them.
fn split(hir: &Hir) -> (bool, Hir, bool) {
    match hir.kind() {
        HirKind::Look(Look::Start) => (true, Hir::empty(), false),
        HirKind::Look(Look::End) => (false, Hir::empty(), true),
        HirKind::Concat(parts) => {
            let mut parts = parts.clone();
            let start = matches!(parts.first().map(Hir::kind), Some(HirKind::Look(Look::Start)));
            if start {
                parts.remove(0);
            }
            let end = matches!(parts.last().map(Hir::kind), Some(HirKind::Look(Look::End)));
            if end {
                parts.pop();
            }
            (start, Hir::concat(parts), end)
        }
        HirKind::Alternation(branches) => {
            let split = branches.iter().map(split).collect::<Vec<_>>();
            let start = split.iter().all(|(start, _, _)| *start);
            let end = split.iter().all(|(_, _, end)| *end);
            if !start && !end {
                return (false, hir.clone(), false);
            }
            // Only the anchors every branch has move out; the others stay in their branch.
            let branches = branches
                .iter()
                .zip(split)
                .map(|(branch, (s, core, e))| {
                    let mut parts = Vec::new();
                    if s && !start {
                        parts.push(Hir::look(Look::Start));
                    }
                    parts.push(core);
                    if e && !end {
                        parts.push(Hir::look(Look::End));
                    }
                    let _ = branch;
                    Hir::concat(parts)
                })
                .collect();
            (start, Hir::alternation(branches), end)
        }
        _ => (false, hir.clone(), false),
    }
}

/// A run of any characters at an anchored edge says nothing: `\A(?s).*x` is "contains x" (and so
/// `\A(?s).*\z` is "anything"). Without `(?s)`, `.` stops at a line break, so the rewrite holds
/// only for single-line text.
struct EdgeAnything;

impl Rewrite for EdgeAnything {
    fn name(&self) -> &'static str {
        "edge-anything"
    }

    fn apply(&self, pattern: &Pattern, domain: Domain) -> Option<Pattern> {
        let anything = |hir: &Hir| match hir.kind() {
            HirKind::Repetition(r) => r.min == 0 && r.max.is_none() && covers(&r.sub, domain),
            _ => false,
        };
        let parts = match pattern.core.kind() {
            HirKind::Concat(parts) => parts.clone(),
            _ => vec![pattern.core.clone()],
        };
        let mut next = pattern.clone();
        let mut parts = parts;
        if next.start && parts.first().is_some_and(anything) {
            parts.remove(0);
            next.start = false;
        }
        if next.end && parts.last().is_some_and(anything) {
            parts.pop();
            next.end = false;
        }
        next.core = Hir::concat(parts);
        (next != *pattern).then_some(next)
    }
}

/// Whether `class` matches every character `domain`'s text can hold.
fn covers(hir: &Hir, domain: Domain) -> bool {
    match hir.kind() {
        HirKind::Class(Class::Unicode(class)) => {
            let ranges = class.ranges();
            let all = ranges.len() == 1 && ranges[0].start() == '\0' && ranges[0].end() == '\u{10FFFF}';
            let all_but_newline = ranges.len() == 2
                && ranges[0].start() == '\0'
                && ranges[0].end() == '\t'
                && ranges[1].start() == '\u{0B}'
                && ranges[1].end() == '\u{10FFFF}';
            all || (domain.single_line && all_but_newline)
        }
        HirKind::Class(Class::Bytes(class)) => {
            let ranges = class.ranges();
            let all = ranges.len() == 1 && ranges[0].start() == 0 && ranges[0].end() == 255;
            let all_but_newline = ranges.len() == 2 && ranges[0].start() == 0 && ranges[0].end() == 9 && ranges[1].start() == 11 && ranges[1].end() == 255;
            all || (domain.single_line && all_but_newline)
        }
        _ => false,
    }
}

/// A word boundary and its negation at one position match nothing (`\b\B`, the source rules'
/// "never"): the core becomes the empty class.
struct Contradiction;

impl Rewrite for Contradiction {
    fn name(&self) -> &'static str {
        "contradiction"
    }

    fn apply(&self, pattern: &Pattern, _: Domain) -> Option<Pattern> {
        let HirKind::Concat(parts) = pattern.core.kind() else { return None };
        let contradicts = parts.windows(2).any(|pair| {
            matches!(
                (pair[0].kind(), pair[1].kind()),
                (HirKind::Look(Look::WordAscii), HirKind::Look(Look::WordAsciiNegate))
                    | (HirKind::Look(Look::WordAsciiNegate), HirKind::Look(Look::WordAscii))
                    | (HirKind::Look(Look::WordUnicode), HirKind::Look(Look::WordUnicodeNegate))
                    | (HirKind::Look(Look::WordUnicodeNegate), HirKind::Look(Look::WordUnicode))
            )
        });
        contradicts.then(|| Pattern { start: false, core: Hir::fail(), end: false })
    }
}
