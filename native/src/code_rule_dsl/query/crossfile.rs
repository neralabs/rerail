//! Cross-file clauses over relation rows.
//!
//! A cross-file clause reads relations: rows an AST query projects from every file of its domain
//! (`require_each` subjects and evidence, `budget` usages). Each file contributes its rows, or is
//! incomplete (its query failed or could not finish), which no absence or upper bound may ignore.
//! `require_each` is an anti-join: evidence rows are bucketed by the hash of the fields the join
//! predicate's top-level `eq`s compare with subject fields or literals, and each subject tests the
//! predicate against its bucket alone (every evidence row when no `eq` seeks), stopping at the
//! first match. `budget` counts the passing rows per group key, groups in first-row order.
//!
//! A `where` predicate (`all`, `any`, `not`, `eq`, `ne`) compares operands by their JSON text: a
//! field reads its row's value (missing equals only missing), anything else is a JSON literal; so
//! `"1"` and `1` differ, `1` and `1.0` differ, and objects compare key by key in order.

use indexmap::IndexMap;
use serde_json::{Map, Number, Value};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hash, Hasher};

/// One relation's rows and the files whose rows are unknown.
#[derive(Default)]
pub(in crate::code_rule_dsl) struct Relation {
    /// Projected fields per row, in the order the caller reports them.
    pub(in crate::code_rule_dsl) rows: Vec<Map<String, Value>>,
    /// Files of the relation's domain whose rows are incomplete.
    pub(in crate::code_rule_dsl) incomplete: BTreeSet<String>,
}

/// What `require_each` found for one subject row.
#[derive(Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) enum Requirement {
    /// No evidence row satisfies the predicate, and the evidence is complete.
    Missing(usize),
    /// No complete evidence row satisfies it, but some evidence file is incomplete: unknown.
    Unknown(usize),
}

/// One `budget` group over its limit, or one that could be.
#[derive(Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct Group {
    /// The group's first row (its anchor).
    pub(in crate::code_rule_dsl) first: usize,
    /// The group's key: its `group_by` values' JSON joined by U+001F, or `all`.
    pub(in crate::code_rule_dsl) key: String,
    pub(in crate::code_rule_dsl) count: usize,
    /// The count is a lower bound: some file of the domain is incomplete.
    pub(in crate::code_rule_dsl) partial: bool,
}

/// A cross-file clause `interrupted` stopped before it answered.
#[derive(Debug, Eq, PartialEq)]
pub(in crate::code_rule_dsl) struct Interrupted;

/// Rows visited between polls of `interrupted`.
const ROWS_PER_POLL: usize = 1024;

/// Counts visited rows: polls `interrupted` at the first and then every `ROWS_PER_POLL`.
struct Poll {
    interrupted: fn() -> bool,
    visited: usize,
}

impl Poll {
    fn new(interrupted: fn() -> bool) -> Self {
        Self {
            interrupted,
            visited: 0,
        }
    }

    fn visit(&mut self) -> Result<(), Interrupted> {
        let due = self.visited.is_multiple_of(ROWS_PER_POLL);
        self.visited += 1;
        if due && (self.interrupted)() {
            Err(Interrupted)
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Evidence rows the last `require_each` on this thread tested the join predicate against.
    static PROBED_ROWS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The bound row `require_each`'s evidence fields read; its subject fields read row 0.
const EVIDENCE: usize = 1;

/// A predicate operand: `alias.field` of a bound alias reads that row's field (the field is
/// everything after the first dot); anything else is a literal.
enum Operand<'v> {
    /// The field of the row bound at this position of the aliases.
    Field(usize, &'v str),
    Literal(Value),
}

impl<'v> Operand<'v> {
    fn compile(value: &'v Value, aliases: &[&str]) -> Self {
        value
            .as_str()
            .and_then(|text| Self::field(text, aliases))
            .unwrap_or_else(|| Self::Literal(value.clone()))
    }

    /// `group_by`/`distinct` names: each a bound field, or else the name as a JSON string.
    fn named(names: &'v [String], aliases: &[&str]) -> Vec<Self> {
        names
            .iter()
            .map(|name| {
                Self::field(name, aliases)
                    .unwrap_or_else(|| Self::Literal(Value::String(name.clone())))
            })
            .collect()
    }

    fn field(text: &'v str, aliases: &[&str]) -> Option<Self> {
        let (alias, field) = text.split_once('.')?;
        let bound = aliases.iter().position(|name| *name == alias)?;
        Some(Self::Field(bound, field))
    }

    /// The operand's value over `rows` (indexed as the aliases were); `None` for a missing field.
    fn read<'r>(&'r self, rows: &[&'r Map<String, Value>]) -> Option<&'r Value> {
        match self {
            Self::Field(bound, field) => rows[*bound].get(*field),
            Self::Literal(value) => Some(value),
        }
    }
}

enum Predicate<'v> {
    Constant(bool),
    All(Vec<Self>),
    Any(Vec<Self>),
    Not(Box<Self>),
    /// `eq` (`true`) or `ne` (`false`) of two operands' JSON text.
    Equal(Operand<'v>, Operand<'v>, bool),
    /// UTF-16 ordinal order of two string operands; other JSON types do not compare.
    Ordered(Operand<'v>, Operand<'v>, &'static str),
}

impl<'v> Predicate<'v> {
    /// A `where` predicate with `aliases` bound to rows by position. No predicate holds; anything
    /// unreadable (not an object, an unknown operator, `eq`/`ne` without two operands) does not.
    /// An object's first of `all`, `any`, `not`, `eq`, `ne` decides; `all`/`any` of no items (or
    /// of a non-array) hold and do not.
    fn compile(predicate: Option<&'v Value>, aliases: &[&str]) -> Self {
        let Some(predicate) = predicate else {
            return Self::Constant(true);
        };
        let Some(object) = predicate.as_object() else {
            return Self::Constant(false);
        };
        let items = |items: &'v Value| {
            items
                .as_array()
                .into_iter()
                .flatten()
                .map(|item| Self::compile(Some(item), aliases))
                .collect()
        };
        if let Some(all) = object.get("all") {
            return Self::All(items(all));
        }
        if let Some(any) = object.get("any") {
            return Self::Any(items(any));
        }
        if let Some(inner) = object.get("not") {
            return Self::Not(Box::new(Self::compile(Some(inner), aliases)));
        }
        for (key, equal) in [("eq", true), ("ne", false)] {
            if let Some([left, right]) =
                object.get(key).and_then(Value::as_array).map(Vec::as_slice)
            {
                return Self::Equal(
                    Operand::compile(left, aliases),
                    Operand::compile(right, aliases),
                    equal,
                );
            }
        }
        for key in ["lt", "le", "gt", "ge"] {
            if let Some([left, right]) =
                object.get(key).and_then(Value::as_array).map(Vec::as_slice)
            {
                return Self::Ordered(
                    Operand::compile(left, aliases),
                    Operand::compile(right, aliases),
                    key,
                );
            }
        }
        Self::Constant(false)
    }

    fn holds(&self, rows: &[&Map<String, Value>]) -> bool {
        match self {
            Self::Constant(holds) => *holds,
            Self::All(items) => items.iter().all(|item| item.holds(rows)),
            Self::Any(items) => items.iter().any(|item| item.holds(rows)),
            Self::Not(inner) => !inner.holds(rows),
            Self::Equal(left, right, equal) => {
                let same = match (left.read(rows), right.read(rows)) {
                    (Some(left), Some(right)) => same_text(left, right),
                    (left, right) => left.is_none() && right.is_none(),
                };
                same == *equal
            }
            Self::Ordered(left, right, operation) => {
                let (Some(left), Some(right)) = (
                    left.read(rows).and_then(Value::as_str),
                    right.read(rows).and_then(Value::as_str),
                ) else {
                    return false;
                };
                let order = left.encode_utf16().cmp(right.encode_utf16());
                match *operation {
                    "lt" => order.is_lt(),
                    "le" => order.is_le(),
                    "gt" => order.is_gt(),
                    "ge" => order.is_ge(),
                    _ => false,
                }
            }
        }
    }
}

/// Whether two values serialize to the same compact JSON text.
fn same_text(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => same_number(left, right),
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len() && left.iter().zip(right).all(|(l, r)| same_text(l, r))
        }
        (Value::Object(left), Value::Object(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|((lk, lv), (rk, rv))| lk == rk && same_text(lv, rv))
        }
        _ => left == right,
    }
}

/// Integers print their value; floats print their shortest round-trip form, which always has a
/// `.` or exponent and differs per bit pattern (`-0.0` is not `0.0`).
fn same_number(left: &Number, right: &Number) -> bool {
    if left.is_f64() || right.is_f64() {
        left.is_f64()
            && right.is_f64()
            && left.as_f64().map(f64::to_bits) == right.as_f64().map(f64::to_bits)
    } else {
        left.as_i64() == right.as_i64() && left.as_u64() == right.as_u64()
    }
}

/// Hashes a value consistently with `same_text`.
fn hash_text(value: &Value, hasher: &mut impl Hasher) {
    match value {
        Value::Null => hasher.write_u8(0),
        Value::Bool(value) => {
            hasher.write_u8(1);
            value.hash(hasher);
        }
        Value::Number(number) => {
            hasher.write_u8(2);
            if number.is_f64() {
                number.as_f64().map(f64::to_bits).hash(hasher);
            } else {
                (number.as_i64(), number.as_u64()).hash(hasher);
            }
        }
        Value::String(text) => {
            hasher.write_u8(3);
            text.hash(hasher);
        }
        Value::Array(items) => {
            hasher.write_u8(4);
            hasher.write_usize(items.len());
            items.iter().for_each(|item| hash_text(item, hasher));
        }
        Value::Object(fields) => {
            hasher.write_u8(5);
            hasher.write_usize(fields.len());
            for (key, value) in fields {
                key.hash(hasher);
                hash_text(value, hasher);
            }
        }
    }
}

/// The hash of a probe key: the values its parts read, missing ones included.
fn key_hash<'r>(parts: impl Iterator<Item = Option<&'r Value>>) -> u64 {
    let mut hasher = DefaultHasher::new();
    for part in parts {
        match part {
            None => hasher.write_u8(0),
            Some(value) => {
                hasher.write_u8(1);
                hash_text(value, &mut hasher);
            }
        }
    }
    hasher.finish()
}

/// The `eq`s every evidence row satisfying `predicate` meets, as (evidence field, operand reading
/// no evidence). Only conjunctions are followed: past `any`, `not` or `ne` none is necessary.
fn seeks<'p, 'v>(predicate: &'p Predicate<'v>, found: &mut Vec<(&'v str, &'p Operand<'v>)>) {
    match predicate {
        Predicate::All(items) => items.iter().for_each(|item| seeks(item, found)),
        Predicate::Equal(left, right, true) => {
            for (sought, other) in [(left, right), (right, left)] {
                if let Operand::Field(EVIDENCE, field) = sought
                    && !matches!(other, Operand::Field(EVIDENCE, _))
                {
                    found.push((field, other));
                }
            }
        }
        _ => {}
    }
}

/// `require_each`: the subject rows (passing `subject_predicate`) no evidence row satisfies
/// `predicate` for, in row order. Where evidence is incomplete, such a subject is unknown, never
/// missing. Fails once `interrupted` says so.
pub(in crate::code_rule_dsl) fn require_each(
    (subject_alias, subject): (&str, &Relation),
    subject_predicate: Option<&Value>,
    (evidence_alias, evidence): (&str, &Relation),
    predicate: Option<&Value>,
    interrupted: fn() -> bool,
) -> Result<Vec<Requirement>, Interrupted> {
    let subject_predicate = Predicate::compile(subject_predicate, &[subject_alias]);
    let predicate = Predicate::compile(predicate, &[subject_alias, evidence_alias]);
    let mut sought = Vec::new();
    seeks(&predicate, &mut sought);
    let mut poll = Poll::new(interrupted);
    let mut buckets = HashMap::<u64, Vec<usize>>::new();
    if !sought.is_empty() {
        for (index, row) in evidence.rows.iter().enumerate() {
            poll.visit()?;
            let hash = key_hash(sought.iter().map(|(field, _)| row.get(*field)));
            buckets.entry(hash).or_default().push(index);
        }
    }
    #[cfg(test)]
    PROBED_ROWS.with(|probed| probed.set(0));
    let unknown = !evidence.incomplete.is_empty();
    let mut requirements = Vec::new();
    for (index, row) in subject.rows.iter().enumerate() {
        poll.visit()?;
        if !subject_predicate.holds(&[row]) {
            continue;
        }
        let mut satisfies = |candidate: usize| -> Result<bool, Interrupted> {
            poll.visit()?;
            #[cfg(test)]
            PROBED_ROWS.with(|probed| probed.set(probed.get() + 1));
            Ok(predicate.holds(&[row, &evidence.rows[candidate]]))
        };
        let satisfied = if sought.is_empty() {
            any_candidate(0..evidence.rows.len(), &mut satisfies)?
        } else {
            let hash = key_hash(sought.iter().map(|(_, operand)| operand.read(&[row])));
            let bucket = buckets.get(&hash).map_or(&[][..], Vec::as_slice);
            any_candidate(bucket.iter().copied(), &mut satisfies)?
        };
        if !satisfied {
            requirements.push(if unknown {
                Requirement::Unknown(index)
            } else {
                Requirement::Missing(index)
            });
        }
    }
    Ok(requirements)
}

/// Whether some candidate satisfies the predicate; stops at the first that does.
fn any_candidate(
    candidates: impl Iterator<Item = usize>,
    satisfies: &mut impl FnMut(usize) -> Result<bool, Interrupted>,
) -> Result<bool, Interrupted> {
    for candidate in candidates {
        if satisfies(candidate)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// One group's count so far.
struct Tally {
    first: usize,
    rows: usize,
    distinct: HashSet<String>,
}

/// Appends the JSON text of `operands` over `row` joined by U+001F; a missing field reads `null`.
fn append_key(key: &mut String, operands: &[Operand<'_>], row: &Map<String, Value>) {
    for (index, operand) in operands.iter().enumerate() {
        if index > 0 {
            key.push('\u{1f}');
        }
        match operand.read(&[row]) {
            Some(value) => {
                let _ = write!(key, "{value}");
            }
            None => key.push_str("null"),
        }
    }
}

/// `budget`: groups of the rows passing `predicate`, by `group_by` operands, whose count (of rows,
/// or of distinct `distinct` values) is over `maximum`; with incomplete files, every group, marked
/// partial, since any count may be a lower bound. Rows keep their order; a group's anchor is its
/// first row, and groups come in anchor order. Fails once `interrupted` says so.
pub(in crate::code_rule_dsl) fn budget(
    (alias, relation): (&str, &Relation),
    predicate: Option<&Value>,
    group_by: &[String],
    distinct: &[String],
    maximum: usize,
    interrupted: fn() -> bool,
) -> Result<Vec<Group>, Interrupted> {
    let aliases = [alias];
    let predicate = Predicate::compile(predicate, &aliases);
    let (group_by, distinct) = (
        Operand::named(group_by, &aliases),
        Operand::named(distinct, &aliases),
    );
    let mut poll = Poll::new(interrupted);
    let mut tallies = IndexMap::<String, Tally>::new();
    let (mut key, mut value) = (String::new(), String::new());
    for (index, row) in relation.rows.iter().enumerate() {
        poll.visit()?;
        if !predicate.holds(&[row]) {
            continue;
        }
        key.clear();
        if group_by.is_empty() {
            key.push_str("all");
        } else {
            append_key(&mut key, &group_by, row);
        }
        let slot = match tallies.get_index_of(key.as_str()) {
            Some(slot) => slot,
            None => {
                let tally = Tally {
                    first: index,
                    rows: 0,
                    distinct: HashSet::new(),
                };
                tallies.insert_full(key.clone(), tally).0
            }
        };
        let tally = &mut tallies[slot];
        tally.rows += 1;
        if !distinct.is_empty() {
            value.clear();
            append_key(&mut value, &distinct, row);
            if !tally.distinct.contains(value.as_str()) {
                tally.distinct.insert(value.clone());
            }
        }
    }
    let partial = !relation.incomplete.is_empty();
    let mut groups = tallies
        .into_iter()
        .map(|(key, tally)| Group {
            first: tally.first,
            key,
            count: if distinct.is_empty() {
                tally.rows
            } else {
                tally.distinct.len()
            },
            partial,
        })
        .filter(|group| partial || group.count > maximum)
        .collect::<Vec<_>>();
    if partial && groups.is_empty() {
        // Nothing counted and some file unknown: the whole domain is one open group.
        groups.push(Group {
            first: usize::MAX,
            key: String::new(),
            count: 0,
            partial: true,
        });
    }
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn relation(rows: Vec<Value>, incomplete: &[&str]) -> Relation {
        Relation {
            rows: rows
                .into_iter()
                .map(|row| row.as_object().unwrap().clone())
                .collect(),
            incomplete: incomplete.iter().map(|path| (*path).to_owned()).collect(),
        }
    }

    fn running() -> bool {
        false
    }

    #[test]
    fn require_each_is_an_anti_join_and_incomplete_evidence_proves_no_absence() {
        let components = relation(
            vec![
                json!({"name": "Button", "dir": "ui"}),
                json!({"name": "Card", "dir": "ui"}),
            ],
            &[],
        );
        let stories = relation(vec![json!({"name": "Button", "dir": "ui"})], &[]);
        let join = json!({"all": [{"eq": ["c.name", "s.name"]}, {"eq": ["c.dir", "s.dir"]}]});
        assert_eq!(
            require_each(
                ("c", &components),
                None,
                ("s", &stories),
                Some(&join),
                running
            )
            .unwrap(),
            [Requirement::Missing(1)]
        );
        let partial = relation(
            vec![json!({"name": "Button", "dir": "ui"})],
            &["ui/Broken.stories.tsx"],
        );
        assert_eq!(
            require_each(
                ("c", &components),
                None,
                ("s", &partial),
                Some(&join),
                running
            )
            .unwrap(),
            [Requirement::Unknown(1)]
        );
    }

    #[test]
    fn fields_named_with_path_syntax_are_read_as_their_own_values() {
        // A double quote, a dot, a dollar, a single quote or a backslash in a field name must
        // still address that field: were both sides NULL, `IS` would join every subject to
        // unrelated evidence, and every row would group under `null`.
        for field in ["a\"b", "a.b", "$x", "it's", "c\\d"] {
            let subjects = relation(vec![json!({ field: "Button" })], &[]);
            let evidence = relation(vec![json!({ field: "Card" })], &[]);
            let join = json!({"eq": [format!("c.{field}"), format!("s.{field}")]});
            assert_eq!(
                require_each(
                    ("c", &subjects),
                    None,
                    ("s", &evidence),
                    Some(&join),
                    running
                )
                .unwrap(),
                [Requirement::Missing(0)],
                "{field}"
            );
            let groups = [format!("c.{field}")];
            assert_eq!(
                budget(("c", &subjects), None, &groups, &[], 0, running).unwrap(),
                [Group {
                    first: 0,
                    key: "\"Button\"".to_owned(),
                    count: 1,
                    partial: false
                }],
                "{field}"
            );
        }
    }

    /// Once `interrupted` says so, a cross-file clause fails instead of reading every row and
    /// answering late.
    #[test]
    fn an_interrupted_clause_fails_instead_of_answering() {
        fn stopped() -> bool {
            true
        }
        let subjects = relation(vec![json!({"name": "Button"})], &[]);
        let evidence = relation(vec![json!({"name": "Card"})], &[]);
        let join = json!({"eq": ["c.name", "s.name"]});
        assert_eq!(
            require_each(
                ("c", &subjects),
                None,
                ("s", &evidence),
                Some(&join),
                stopped
            ),
            Err(Interrupted)
        );
        assert_eq!(
            budget(("c", &subjects), None, &[], &[], 0, stopped),
            Err(Interrupted)
        );
    }

    #[test]
    fn require_each_reads_projected_fields_with_quotes() {
        let subjects = relation(vec![json!({"na\"me": "Button"})], &[]);
        let evidence = relation(vec![json!({"na\"me": "Card"})], &[]);
        let join = json!({"eq": ["subject.na\"me", "evidence.na\"me"]});
        assert_eq!(
            require_each(
                ("subject", &subjects),
                None,
                ("evidence", &evidence),
                Some(&join),
                running
            )
            .unwrap(),
            vec![Requirement::Missing(0)]
        );
    }

    #[test]
    fn require_each_probes_only_the_evidence_rows_with_the_joined_value() {
        let subjects = relation(
            (0..32)
                .map(|index| json!({"name": format!("subject-{index}"), "note": index}))
                .collect(),
            &[],
        );
        let evidence = relation(
            (0..2_000)
                .map(|index| json!({"name": format!("evidence-{index}"), "extra": index}))
                .chain([json!({"name": "subject-5"})])
                .collect(),
            &[],
        );
        let join = json!({"eq": ["subject.name", "evidence.name"]});
        assert_eq!(
            require_each(
                ("subject", &subjects),
                None,
                ("evidence", &evidence),
                Some(&join),
                running
            )
            .unwrap()
            .len(),
            31
        );
        assert_eq!(
            PROBED_ROWS.with(std::cell::Cell::get),
            1,
            "the evidence probe reads only rows holding the joined value"
        );
    }

    /// A predicate with no `eq` to seek by tests the evidence rows in order, stopping at the
    /// first that satisfies it.
    #[test]
    fn require_each_without_an_eq_probes_evidence_until_the_first_match() {
        let subjects = relation(
            ["a", "b", "c"]
                .into_iter()
                .map(|owner| json!({"owner": owner}))
                .collect(),
            &[],
        );
        let evidence = relation(vec![json!({"owner": "a"}), json!({"owner": "a"})], &[]);
        let differ = json!({"ne": ["subject.owner", "evidence.owner"]});
        let same = json!({"not": differ.clone()});
        for (predicate, missing, probed) in [(differ, vec![0], 4), (same, vec![1, 2], 5)] {
            assert_eq!(
                require_each(
                    ("subject", &subjects),
                    None,
                    ("evidence", &evidence),
                    Some(&predicate),
                    running
                )
                .unwrap(),
                missing
                    .into_iter()
                    .map(Requirement::Missing)
                    .collect::<Vec<_>>(),
                "{predicate}"
            );
            assert_eq!(
                PROBED_ROWS.with(std::cell::Cell::get),
                probed,
                "{predicate}"
            );
        }
    }

    #[test]
    fn require_each_preserves_json_type_and_null_equality() {
        let subjects = relation(
            vec![json!({"key": 1}), json!({"key": null}), json!({"key": "1"})],
            &[],
        );
        let evidence = relation(vec![json!({"key": "1"}), json!({"key": null})], &[]);
        let join = json!({"eq": ["subject.key", "evidence.key"]});
        assert_eq!(
            require_each(
                ("subject", &subjects),
                None,
                ("evidence", &evidence),
                Some(&join),
                running
            )
            .unwrap(),
            [Requirement::Missing(0)]
        );
    }

    #[test]
    fn budget_counts_groups_and_never_calls_a_lower_bound_clean() {
        let usages = relation(
            vec![
                json!({"package": "a"}),
                json!({"package": "b"}),
                json!({"package": "a"}),
            ],
            &[],
        );
        let groups = ["u.package".to_owned()];
        assert_eq!(
            budget(("u", &usages), None, &groups, &[], 1, running).unwrap(),
            [Group {
                first: 0,
                key: "\"a\"".to_owned(),
                count: 2,
                partial: false
            }]
        );
        assert_eq!(
            budget(
                ("u", &usages),
                None,
                &groups,
                &["u.package".to_owned()],
                1,
                running
            )
            .unwrap(),
            [],
            "two rows of one distinct value count once"
        );
        let partial = relation(vec![json!({"package": "a"})], &["b/x.py"]);
        assert_eq!(
            budget(("u", &partial), None, &groups, &[], 1, running).unwrap(),
            [Group {
                first: 0,
                key: "\"a\"".to_owned(),
                count: 1,
                partial: true
            }]
        );
    }
}
