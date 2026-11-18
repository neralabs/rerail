//! Which documents a rule sees (decision 0058 §1): a base atom ranges over the policy's shown documents unless the
//! rule reached it from a guest.
//!
//! A run's documents are, by key (`syntax::GUEST_KEYS`, `syntax::HIDDEN_KEYS`):
//! - files, `1..2^30`;
//! - guests of code in the run's language (Python in a Python string, embedded by a library), `2^30..2^30 + 2^28`:
//!   shown, unless the program says `.guests off`;
//! - annotations (a type expression, Python's in a Python run), `..2^30 + 2^29`: not shown to a rule that does not
//!   name them, but kept in a relation they reach once its copies for them are folded into it (`combining::FoldOwn`);
//! - every other document (a guest in another grammar, a value `@parse` or `@splice` made), up to `2^31`: never
//!   shown.
//!
//! A node's id is its document's key `<< 32` and its index, so "shown" is one bound on a location value.
//!
//! A variable is *scoped* when the rule reached it from a guest relation (`guest`, `parsed`, `doc_root`, `doc_role`,
//! `tape_of`, `host_node`, `string_annotation`): it is a location in a guest column of one, or it shares a base atom's location columns
//! with a scoped variable (they are in one document), or it is read from a derived relation's column that some rule
//! fills from a scoped variable. Every other variable in a positive base atom's location column gets the bound
//! (`n < bound << 32`, `d < bound`). The bound is a comparison on a value the join already has: no row is copied,
//! and a run that makes no guests is not changed at all.
//!
//! A scoped variable also knows its grammar when the guest relation names it (`parsed(s, "sql", r)`, `guest(g, _,
//! _, "sql")`, a tape from `@parse("sql", …)`). A kind or field constant on such a node is resolved in that grammar
//! (`"sql:select"`, `plan::lower::Constants`), so a Python policy can name SQL kinds inside its SQL guests.

/// The relations a rule enters a guest through, and their columns that are the guest's: (relation, guest location
/// columns, grammar column, tape column).
/// `string_annotation(s)` is one: Ruff's strings that are type definitions, nested ones too, whose nodes are in an
/// annotation's document (not shown, `guests::key`).
pub(super) const ENTRIES: [(&str, &[usize], Option<usize>, Option<usize>); 10] = [
    ("guest", &[0], Some(3), None),
    ("string_annotation", &[0], None, None),
    ("parsed", &[2], Some(1), None),
    ("sub_tape", &[], Some(1), Some(2)),
    ("doc_root", &[0, 1], None, None),
    ("doc_role", &[0], None, None),
    ("tape_of", &[0], None, Some(1)),
    ("host_node", &[0], None, None),
    ("parse_of", &[], Some(1), Some(2)),
    ("splice_of", &[], None, Some(3)),
];

/// Views of base relations the lowering writes as rules.
pub(super) const VIEWS: [&str; 3] = ["kind", "docof", "precedes"];

/// The grammar a guest's kinds are named in: a dialect label (`annotation` is Python's expression).
pub(super) fn dialect_label(grammar: &str) -> &str {
    if grammar == "annotation" { "python" } else { grammar }
}

/// The bound of `prog`'s shown documents: files, and the run language's guests unless `.guests off`.
pub(super) fn bound(prog: &(impl crate::code_rule_dsl::logic::program_image::fields::ProgramFields + ?Sized)) -> u32 {
    use crate::code_rule_dsl::facts::vtab::syntax::{ANNOTATION_KEYS, GUEST_KEYS, GUEST_POLICY};
    let off = prog.fact_rows(GUEST_POLICY).iter().any(|row| {
        matches!(row.as_slice(), [crate::code_rule_dsl::logic::front::Value::Str(k), crate::code_rule_dsl::logic::front::Value::Str(v)] if k == "library" && v == "off")
    });
    if off { GUEST_KEYS } else { ANNOTATION_KEYS }
}

/// What the analysis knows of a variable: scoped, and its grammar (`Some("")` when two disagree). `definite`: it is
/// a guest's node, not only maybe one (O110). A variable read from a derived column that host rows fill too is
/// scoped (no bound on it) but not definite: it is specialized to a guest grammar only in a clone of its rule.
/// For a derived relation's column, `definite` says every rule fills it with a definite variable.
#[derive(Clone, Default, PartialEq, Debug)]
pub(super) struct Fact {
    pub(super) scoped: bool,
    pub(super) grammar: Option<String>,
    pub(super) definite: bool,
}

impl Fact {
    pub(super) fn join(&mut self, other: &Fact) -> bool {
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

/// `"sql:select"` as its grammar's label and the name, when the label is a registry language's.
pub fn qualified(s: &str) -> Option<(&str, &str)> {
    let (label, name) = s.split_once(':')?;
    (!label.is_empty() && !name.is_empty() && crate::code_rule_dsl::language::dialect_by_label(label).is_some()).then_some((label, name))
}
