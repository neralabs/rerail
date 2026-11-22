//! Which documents a rule sees (decision 0058 §1): a base atom ranges over the policy's shown documents unless the
//! rule reached it from a guest.
//!
//! A run's documents are, by key (`syntax::GUEST_KEYS`, `syntax::HIDDEN_KEYS`):
//! - files, `1..2^30`, a file read for its programs among them (its text, decision 0512);
//! - programs in the run's language (a `<script>` in an HTML file, a JSON object in a rule file), `2^30..2^30 + 2^29`:
//!   shown (decision 0523: always);
//! - every other document (a guest in another grammar, a file's own tree in another grammar, a value `@parse` or
//!   `@splice` made, Python's string annotations among them), up to `2^31`: never shown. A context's rules read a
//!   value's nodes (decision 0597), from its roots.
//!
//! A node's id is its document's key `<< 32` and its index, so "shown" is one bound on a location value.
//!
//! A variable is *scoped* when the rule reached it from a guest relation (`parsed`, `tape_of`, `host_node`,
//! `guest_of`, `string_annotation`; the lowering's `rr_program`): it is a location in a guest column of one, or it shares a base atom's location columns
//! with a scoped variable (they are in one document), or it is read from a derived relation's column that some rule
//! fills from a scoped variable. Every other variable in a positive base atom's location column gets the bound
//! (`n < bound << 32`, `d < bound`). The bound is a comparison on a value the join already has: no row is copied,
//! and a run that makes no guests is not changed at all.
//!
//! A scoped variable also knows its grammar when the guest relation names it (`parsed(s, "sql", r)`, a tape from
//! `@parse("sql", …)`). A kind or field constant on such a node is resolved in that grammar
//! (`"sql:select"`, `plan::lower::Constants`), so a Python policy can name SQL kinds inside its SQL guests.

/// The relations a rule enters a guest through, and their columns that are the guest's: (relation, guest location
/// columns, grammar column, tape column).
/// `string_annotation(s)` is one: Ruff's strings that are type definitions, nested ones too, whose nodes are in an
/// annotation's document (not shown, `guests::key`).
pub const ENTRIES: [(&str, &[usize], Option<usize>, Option<usize>); 11] = [
    ("rr_program", &[0], Some(2), None),
    ("string_annotation", &[0], None, None),
    ("parsed", &[2], Some(1), None),
    ("sub_tape", &[], Some(1), Some(2)),
    ("doc_grammar", &[0], Some(1), None),
    ("tape_of", &[0], None, Some(1)),
    ("host_node", &[0], None, None),
    ("guest_of", &[0], None, None),
    ("guest_place", &[0], None, None),
    ("parse_of", &[], Some(1), Some(2)),
    ("splice_of", &[], None, Some(3)),
];

/// Views of base relations the lowering writes as rules.
pub const VIEWS: [&str; 3] = ["kind", "docof", "precedes"];

/// The grammar a guest's kinds are named in: a dialect label (a fragment's is its dialect's, decision 0466).
pub fn dialect_label(grammar: &str) -> &str {
    crate::code_rule_dsl::language::kinds_label(grammar)
}

/// The bound of a program's shown documents: files, and the run language's code guests (decision 0523: `.guests` is
/// retired; what a run reads of other languages' files is `.embedded`'s).
pub const BOUND: u32 = crate::code_rule_dsl::facts::vtab::syntax::HIDDEN_KEYS;

/// What the analysis knows of a variable: scoped, and its grammar (`Some("")` when two disagree). `definite`: it is
/// a guest's node, not only maybe one (O110). A variable read from a derived column that host rows fill too is
/// scoped (no bound on it) but not definite: it is specialized to a guest grammar only in a clone of its rule.
/// For a derived relation's column, `definite` says every rule fills it with a definite variable.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct Fact {
    pub scoped: bool,
    pub grammar: Option<String>,
    pub definite: bool,
}

impl Fact {
    pub fn join(&mut self, other: &Fact) -> bool {
        let mut changed = (other.scoped && !self.scoped) || (other.definite && !self.definite);
        self.scoped |= other.scoped;
        self.definite |= other.definite;
        // Two grammars join to none known (`""`).
        match (&self.grammar, &other.grammar) {
            (_, None) => {}
            (None, Some(b)) => {
                self.grammar = Some(b.clone());
                changed = true;
            }
            (Some(a), Some(b)) if a == b || a.is_empty() => {}
            (Some(_), Some(_)) => {
                self.grammar = Some(String::new());
                changed = true;
            }
        }
        changed
    }
}

pub use crate::code_rule_dsl::logic::plan::lower::qualified;
