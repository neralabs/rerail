//! Differential tests of `sql::select_when` and `sql::crossfile` against their native oracles,
//! `query::select_when::answer` and `query::crossfile::{require_each, budget}`: random plans and
//! leaf rows, random relations and predicates; every answer must be identical.
//!
//! The generator is deterministic: `RERAIL_SQL_SETOPS_SEED` (default `0x5e7_0b5`) seeds it and
//! `RERAIL_SQL_SETOPS_CASES` (default 500) sets how many cases each of `select_when`,
//! `require_each` and `budget` compares.

use crate::code_rule_dsl::model::{Query, RegexSource};
use crate::code_rule_dsl::query::crossfile::{self, Group, Interrupted, Relation, Requirement};
use crate::code_rule_dsl::query::select_when::{self, Answer, LeafRow, Leaves, Plan, Unknown};
use crate::code_rule_dsl::sql::{self, SqlError};
use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

/// SplitMix64: small, fast and fixed, so a seed names the same cases everywhere.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound.max(1) as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    fn pick<T: Clone>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())].clone()
    }
}

fn env_number(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|text| match text.strip_prefix("0x") {
            Some(hex) => u64::from_str_radix(hex, 16).ok(),
            None => text.parse().ok(),
        })
        .unwrap_or(default)
}

fn settings() -> (u64, usize) {
    (
        env_number("RERAIL_SQL_SETOPS_SEED", 0x5e7_0b5),
        env_number("RERAIL_SQL_SETOPS_CASES", 500) as usize,
    )
}

fn case_rng(seed: u64, salt: u64, index: usize) -> Rng {
    Rng(seed ^ salt ^ (index as u64).wrapping_mul(0x2545_f491_4f6c_dd1d))
}

/// Differences found, the first few kept for the failure message.
#[derive(Default)]
struct Diffs {
    count: usize,
    shown: Vec<String>,
}

impl Diffs {
    fn note(&mut self, message: String) {
        self.count += 1;
        if self.shown.len() < 5 {
            self.shown.push(message);
        }
    }

    fn assert_none(&self, what: &str, seed: u64) {
        assert!(
            self.count == 0,
            "{what}: {} differences (seed {seed:#x}):\n{}",
            self.count,
            self.shown.join("\n\n")
        );
    }
}

// --- select_when ------------------------------------------------------------------------------

fn regex_leaf() -> Query {
    Query::Regex {
        regex: regex::Regex::new("x").expect("regex"),
        source: RegexSource::Content,
        options: Default::default(),
    }
}

/// A random query: leaves and `select_when`s nested on both sides.
fn random_query(rng: &mut Rng, depth: usize) -> Query {
    if depth == 0 || rng.chance(35) {
        return regex_leaf();
    }
    Query::SelectWhen {
        select: Box::new(random_query(rng, depth - 1)),
        exists: Box::new(random_query(rng, depth - 1)),
    }
}

fn shape(query: &Query) -> String {
    match query {
        Query::SelectWhen { select, exists } => {
            format!("when({}, {})", shape(exists), shape(select))
        }
        _ => "leaf".to_owned(),
    }
}

fn random_row(rng: &mut Rng) -> LeafRow {
    LeafRow {
        failed: rng.chance(15),
        partial: rng.chance(25),
        witnesses: match rng.below(10) {
            0..=4 => 0,
            5..=8 => 1 + rng.below(4),
            _ => rng.pick(&[17, 1000, 1 << 40]),
        },
    }
}

/// Fixed rows per leaf, recording every `row` call and failing at `error`.
struct Fixed<'r> {
    rows: &'r [Vec<LeafRow>],
    error: Option<(usize, usize)>,
    calls: RefCell<Vec<(usize, usize)>>,
}

impl<'r> Fixed<'r> {
    fn new(rows: &'r [Vec<LeafRow>], error: Option<(usize, usize)>) -> Self {
        Self {
            rows,
            error,
            calls: RefCell::default(),
        }
    }
}

impl Leaves for Fixed<'_> {
    fn row(&self, leaf: usize, position: usize) -> Result<Option<LeafRow>, String> {
        self.calls.borrow_mut().push((leaf, position));
        if self.error == Some((leaf, position)) {
            return Err(format!("row {leaf}/{position} failed"));
        }
        Ok(self.rows[leaf].get(position).copied())
    }
}

type Answered = Result<Result<(Vec<(usize, usize, usize)>, Vec<(usize, usize)>), Unknown>, String>;

fn answered(answer: Result<Result<Answer, Unknown>, String>) -> Answered {
    answer.map(|answer| answer.map(|answer| (answer.witnesses, answer.partial)))
}

/// What one case's SQL answers did: `(unknown, refused)`.
fn select_when_case(rng: &mut Rng, diffs: &mut Diffs, case: usize) -> (bool, bool) {
    let depth = rng.below(5);
    let query = random_query(rng, depth);
    let decision = rng.chance(25);
    let plan = Plan::new(&query, decision, |_, reported| reported);
    let rows: Vec<Vec<LeafRow>> = plan
        .leaves
        .iter()
        .map(|_| {
            let length = rng.below(7);
            (0..length).map(|_| random_row(rng)).collect()
        })
        .collect();
    let cap = rng.pick(&[0, 1, 2, 3, 4, 7, usize::MAX]);
    let error = rng.chance(10).then(|| {
        let leaf = rng.below(rows.len());
        (leaf, rng.below(rows[leaf].len() + 1))
    });
    let describe = || {
        format!(
            "case {case}: {} decision={decision} cap={cap} error={error:?} rows={rows:?}",
            shape(&query)
        )
    };
    let native_leaves = Fixed::new(&rows, error);
    let native = answered(select_when::answer(&plan, cap, &native_leaves));
    // Through the `Leaves` reader: the same calls, the same answer.
    let sql_leaves = Fixed::new(&rows, error);
    let through = answered(sql::select_when::answer_leaves(&plan, cap, &sql_leaves));
    let refused = matches!(&through, Err(message) if message.contains("Unsupported"));
    if !refused {
        if through != native {
            diffs.note(format!(
                "{}\n native {native:?}\n sql    {through:?}",
                describe()
            ));
        }
        if *sql_leaves.calls.borrow() != *native_leaves.calls.borrow() {
            diffs.note(format!(
                "{}\n native calls {:?}\n sql calls    {:?}",
                describe(),
                native_leaves.calls.borrow(),
                sql_leaves.calls.borrow()
            ));
        }
    }
    // Over every row of every leaf, read or not: the statement ends each leaf itself.
    if let Ok(expected) = &native {
        match sql::select_when::answer(&plan, cap, &rows) {
            Ok(whole) => {
                let whole = whole.map(|answer| (answer.witnesses, answer.partial));
                if &whole != expected {
                    diffs.note(format!(
                        "{} (whole tables)\n native {expected:?}\n sql    {whole:?}",
                        describe()
                    ));
                }
            }
            Err(SqlError::Unsupported(_)) => return (false, true),
            Err(error) => diffs.note(format!("{}: {error}", describe())),
        }
    }
    (matches!(native, Ok(Err(_))), refused)
}

#[test]
fn select_when_answers_alike_in_sql() {
    let (seed, cases) = settings();
    let started = Instant::now();
    let mut diffs = Diffs::default();
    let (mut unknown, mut refused) = (0, 0);
    for case in 0..cases {
        let mut rng = case_rng(seed, 0x005e_1ec7, case);
        let (was_unknown, was_refused) = select_when_case(&mut rng, &mut diffs, case);
        unknown += usize::from(was_unknown);
        refused += usize::from(was_refused);
    }
    eprintln!(
        "setops select_when: seed {seed:#x}, {cases} cases compared, {} differences, {refused} refused, {unknown} unknown answers, {:.1}s",
        diffs.count,
        started.elapsed().as_secs_f64()
    );
    diffs.assert_none("select_when", seed);
}

/// A leaf whose rows hold more than `i64::MAX` witnesses in all is refused, never answered from
/// an overflowed running sum.
#[test]
fn select_when_refuses_witness_totals_past_i64() {
    let query = regex_leaf();
    let plan = Plan::new(&query, false, |_, reported| reported);
    let huge = LeafRow {
        failed: false,
        partial: false,
        witnesses: usize::MAX / 2,
    };
    assert!(matches!(
        sql::select_when::answer(&plan, 10, &[vec![huge; 3]]),
        Err(SqlError::Unsupported(_))
    ));
    let answer = sql::select_when::answer(&plan, usize::MAX, &[vec![huge]])
        .expect("answers")
        .expect("known");
    assert_eq!(answer.witnesses, [(0, 0, usize::MAX / 2)]);
}

/// A lazy reader like the workspace's (`workspace::global::EvidenceLeaves`): each leaf walks its
/// partitions in path order as rows are asked for, a decision leaf ends at its first partition
/// holding a witness, and once a shared allowance of pulls is spent every leaf ends with a failed
/// row. Native and SQL answers through it agree, and pull the same partitions.
struct Partitions<'p> {
    decisions: Vec<bool>,
    reads: Vec<RefCell<PartitionRead<'p>>>,
    allowance: RefCell<usize>,
}

/// A partition: `(failed, syntax, witnesses)`.
type Partition = (bool, bool, usize);

struct PartitionRead<'p> {
    partitions: Option<std::collections::btree_map::Iter<'p, String, Partition>>,
    rows: Vec<Option<&'p Partition>>,
}

impl<'p> Partitions<'p> {
    fn new(
        plan: &Plan<bool>,
        partitions: &'p [BTreeMap<String, Partition>],
        allowance: usize,
    ) -> Self {
        Self {
            decisions: plan.leaves.iter().map(|leaf| leaf.decision).collect(),
            reads: partitions
                .iter()
                .map(|partitions| {
                    RefCell::new(PartitionRead {
                        partitions: Some(partitions.iter()),
                        rows: Vec::new(),
                    })
                })
                .collect(),
            allowance: RefCell::new(allowance),
        }
    }

    fn pulled(&self) -> Vec<usize> {
        self.reads
            .iter()
            .map(|read| read.borrow().rows.len())
            .collect()
    }
}

impl Leaves for Partitions<'_> {
    fn row(&self, leaf: usize, position: usize) -> Result<Option<LeafRow>, String> {
        let mut read = self.reads[leaf].borrow_mut();
        while read.rows.len() <= position {
            let Some(partitions) = &mut read.partitions else {
                break;
            };
            let next = partitions.next();
            let spent = {
                let mut allowance = self.allowance.borrow_mut();
                let spent = *allowance == 0;
                *allowance = allowance.saturating_sub(1);
                spent
            };
            let ends = match next {
                None => true,
                Some(_) if spent => {
                    read.rows.push(None);
                    true
                }
                Some((_, partition)) => {
                    read.rows.push(Some(partition));
                    self.decisions[leaf] && !partition.0 && partition.2 > 0
                }
            };
            if ends {
                read.partitions = None;
            }
        }
        Ok(read.rows.get(position).map(|row| match row {
            None => LeafRow {
                failed: true,
                partial: false,
                witnesses: 0,
            },
            Some((failed, syntax, witnesses)) => LeafRow {
                failed: *failed,
                partial: !failed && *syntax,
                witnesses: if *failed { 0 } else { *witnesses },
            },
        }))
    }
}

#[test]
fn a_lazy_workspace_reader_answers_alike_through_sql() {
    let (seed, cases) = settings();
    let mut diffs = Diffs::default();
    for case in 0..cases {
        let mut rng = case_rng(seed, 0x0e71_de4c, case);
        let depth = rng.below(4);
        let query = random_query(&mut rng, depth);
        let decision = rng.chance(25);
        let plan = Plan::new(&query, decision, |_, reported| reported);
        let partitions: Vec<BTreeMap<String, Partition>> = plan
            .leaves
            .iter()
            .map(|_| {
                (0..rng.below(6))
                    .map(|_| {
                        (
                            format!("f{}.ts", rng.below(9)),
                            (rng.chance(10), rng.chance(20), rng.pick(&[0, 0, 1, 2, 5])),
                        )
                    })
                    .collect()
            })
            .collect();
        let allowance = rng.pick(&[0, 1, 3, 8, usize::MAX]);
        let cap = rng.pick(&[0, 1, 2, 5, usize::MAX]);
        let native_leaves = Partitions::new(&plan, &partitions, allowance);
        let native = answered(select_when::answer(&plan, cap, &native_leaves));
        let sql_leaves = Partitions::new(&plan, &partitions, allowance);
        let through = answered(sql::select_when::answer_leaves(&plan, cap, &sql_leaves));
        if through != native || sql_leaves.pulled() != native_leaves.pulled() {
            diffs.note(format!(
                "case {case}: {} decision={decision} cap={cap} allowance={allowance} {partitions:?}\n native {native:?} {:?}\n sql    {through:?} {:?}",
                shape(&query),
                native_leaves.pulled(),
                sql_leaves.pulled()
            ));
        }
    }
    diffs.assert_none("select_when through a lazy reader", seed);
}

// --- require_each and budget ------------------------------------------------------------------

const FIELDS: &[&str] = &["k", "n", "x", "", "a.b", "missing"];

/// A value from a small pool (so equalities happen) or a random one of any type.
fn random_value(rng: &mut Rng, depth: usize) -> Value {
    if rng.chance(55) {
        return rng.pick(&[
            json!(1),
            json!("1"),
            json!(1.0),
            Value::Null,
            json!("a"),
            json!([1]),
            json!(true),
        ]);
    }
    match rng.below(if depth > 0 { 16 } else { 13 }) {
        0 => Value::Null,
        1 => json!(true),
        2 => json!(false),
        3 => json!(0),
        4 => json!(-1),
        5 => json!(0.0),
        6 => json!(-0.0),
        7 => json!(u64::MAX),
        8 => json!(i64::MIN),
        9 => json!(2.5),
        10 => json!(1e21),
        11 | 12 => json!(rng.pick(&[
            "",
            "1",
            "a",
            "A",
            "1.0",
            "s.k",
            "e.k",
            "u.k",
            "null",
            "\u{1f}",
            "a\"b",
            "é",
            "all",
            // Ordered apart by UTF-16 code units, not by code points or UTF-8 bytes.
            "\u{ffff}",
            "\u{1f600}",
            "2050",
            "3000",
        ])),
        13 | 14 => Value::Array(
            (0..rng.below(3))
                .map(|_| random_value(rng, depth - 1))
                .collect(),
        ),
        _ => {
            // Keys in a random insertion order.
            let mut object = Map::new();
            for _ in 0..rng.below(3) {
                object.insert(
                    rng.pick(&["a", "b", "k"]).to_owned(),
                    random_value(rng, depth - 1),
                );
            }
            Value::Object(object)
        }
    }
}

fn random_relation(rng: &mut Rng) -> Relation {
    Relation {
        rows: (0..rng.below(9))
            .map(|_| {
                let mut row = Map::new();
                for field in FIELDS {
                    if *field != "missing" && rng.chance(75) {
                        row.insert((*field).to_owned(), random_value(rng, 2));
                    }
                }
                row
            })
            .collect(),
        incomplete: if rng.chance(25) {
            BTreeSet::from(["broken.ts".to_owned()])
        } else {
            BTreeSet::new()
        },
    }
}

/// An operand: a field of one of `aliases` (or of an unbound alias), or a literal of any type.
fn random_operand(rng: &mut Rng, aliases: &[&str]) -> Value {
    if rng.chance(65) {
        let alias = if rng.chance(90) {
            rng.pick(aliases)
        } else {
            "q"
        };
        json!(format!("{alias}.{}", rng.pick(FIELDS)))
    } else {
        random_value(rng, 1)
    }
}

/// A random `where` predicate over every operator, malformed shapes included.
fn random_predicate(rng: &mut Rng, aliases: &[&str], depth: usize) -> Value {
    let items = |rng: &mut Rng| -> Vec<Value> {
        (0..rng.below(4))
            .map(|_| random_predicate(rng, aliases, depth.saturating_sub(1)))
            .collect()
    };
    let leaf = depth == 0 || rng.chance(40);
    match if leaf { rng.below(5) } else { 5 + rng.below(9) } {
        0 | 1 => json!({ "eq": [random_operand(rng, aliases), random_operand(rng, aliases)] }),
        2 => json!({ "ne": [random_operand(rng, aliases), random_operand(rng, aliases)] }),
        3 => {
            let operator = rng.pick(&ORDERED);
            json!({ operator: [random_operand(rng, aliases), random_operand(rng, aliases)] })
        }
        4 => rng.pick(&[
            json!(5),
            json!("x"),
            json!([]),
            Value::Null,
            json!({ "foo": 1 }),
            json!({ "eq": ["s.k"] }),
            json!({ "eq": "s.k" }),
            json!({ "eq": ["s.k", "e.k", "e.n"] }),
            json!({ "lt": ["s.k"] }),
            json!({ "ge": "s.k" }),
            json!({ "all": 5 }),
            json!({ "any": "x" }),
            json!({}),
        ]),
        5 | 6 => json!({ "all": items(rng) }),
        7 | 8 => json!({ "any": items(rng) }),
        9 | 10 => json!({ "not": random_predicate(rng, aliases, depth - 1) }),
        11 => {
            // Several operators in one object: the first of all, any, not, eq, ne, lt, le, gt, ge
            // decides.
            let mut object = Map::new();
            for key in ["ge", "ne", "lt", "eq", "not", "any", "gt", "all", "le"] {
                if rng.chance(50) {
                    let value = match key {
                        "eq" | "ne" | "lt" | "le" | "gt" | "ge" if rng.chance(70) => {
                            json!([random_operand(rng, aliases), random_operand(rng, aliases)])
                        }
                        "eq" | "ne" | "lt" | "le" | "gt" | "ge" => {
                            json!([random_operand(rng, aliases)])
                        }
                        "not" => random_predicate(rng, aliases, depth - 1),
                        _ => Value::Array(items(rng)),
                    };
                    object.insert(key.to_owned(), value);
                }
            }
            Value::Object(object)
        }
        _ => json!({ "all": [
            { "eq": [random_operand(rng, aliases), random_operand(rng, aliases)] },
            random_predicate(rng, aliases, depth - 1),
        ] }),
    }
}

/// The ordering operators of a `where` predicate.
const ORDERED: [&str; 4] = ["lt", "le", "gt", "ge"];

fn running() -> bool {
    false
}

fn stopped() -> bool {
    true
}

fn random_interrupt(rng: &mut Rng) -> fn() -> bool {
    if rng.chance(5) { stopped } else { running }
}

fn optional_predicate(rng: &mut Rng, aliases: &[&str]) -> Option<Value> {
    (!rng.chance(15)).then(|| random_predicate(rng, aliases, 3))
}

#[test]
fn require_each_answers_alike_in_sql() {
    let (seed, cases) = settings();
    let started = Instant::now();
    let mut diffs = Diffs::default();
    let (mut refused, mut found) = (0, 0);
    for case in 0..cases {
        let mut rng = case_rng(seed, 0x4e9_ea5c, case);
        let (subject_alias, evidence_alias) = match rng.below(20) {
            0 => ("s", "s"),
            1 => ("s.x", "e"),
            _ => ("s", "e"),
        };
        let subject = random_relation(&mut rng);
        let evidence = random_relation(&mut rng);
        let subject_predicate = optional_predicate(&mut rng, &[subject_alias]);
        let predicate = optional_predicate(&mut rng, &[subject_alias, evidence_alias]);
        let interrupted = random_interrupt(&mut rng);
        let native = crossfile::require_each(
            (subject_alias, &subject),
            subject_predicate.as_ref(),
            (evidence_alias, &evidence),
            predicate.as_ref(),
            interrupted,
        );
        let sql = sql::crossfile::require_each(
            (subject_alias, &subject),
            subject_predicate.as_ref(),
            (evidence_alias, &evidence),
            predicate.as_ref(),
            interrupted,
        );
        match sql {
            Err(SqlError::Unsupported(_)) => refused += 1,
            Ok(sql) if sql == native => {
                found += native.map_or(0, |requirements| requirements.len());
            }
            sql => diffs.note(format!(
                "case {case}: aliases ({subject_alias}, {evidence_alias}), subject where {subject_predicate:?}, where {predicate:?}\n subjects {:?} incomplete {:?}\n evidence {:?} incomplete {:?}\n native {native:?}\n sql    {sql:?}",
                subject.rows, subject.incomplete, evidence.rows, evidence.incomplete
            )),
        }
    }
    eprintln!(
        "setops require_each: seed {seed:#x}, {cases} cases compared, {} differences, {refused} refused, {found} requirements, {:.1}s",
        diffs.count,
        started.elapsed().as_secs_f64()
    );
    diffs.assert_none("require_each", seed);
}

#[test]
fn budget_answers_alike_in_sql() {
    const NAMES: &[&str] = &[
        "u.k",
        "u.n",
        "u.x",
        "u.missing",
        "u.",
        "u.a.b",
        "literal",
        "x.k",
    ];
    let (seed, cases) = settings();
    let started = Instant::now();
    let mut diffs = Diffs::default();
    let (mut refused, mut groups) = (0, 0);
    for case in 0..cases {
        let mut rng = case_rng(seed, 0x00b0_d6e7, case);
        let relation = random_relation(&mut rng);
        let predicate = optional_predicate(&mut rng, &["u"]);
        let names = |rng: &mut Rng, most: usize| -> Vec<String> {
            (0..rng.below(most + 1))
                .map(|_| rng.pick(NAMES).to_owned())
                .collect()
        };
        let group_by = names(&mut rng, 3);
        let distinct = if rng.chance(40) {
            names(&mut rng, 2)
        } else {
            Vec::new()
        };
        let maximum = rng.pick(&[0, 1, 2, 3, usize::MAX]);
        let interrupted = random_interrupt(&mut rng);
        let native = crossfile::budget(
            ("u", &relation),
            predicate.as_ref(),
            &group_by,
            &distinct,
            maximum,
            interrupted,
        );
        let sql = sql::crossfile::budget(
            ("u", &relation),
            predicate.as_ref(),
            &group_by,
            &distinct,
            maximum,
            interrupted,
        );
        match sql {
            Err(SqlError::Unsupported(_)) => refused += 1,
            Ok(sql) if sql == native => {
                groups += native.map_or(0, |found: Vec<Group>| found.len());
            }
            sql => diffs.note(format!(
                "case {case}: where {predicate:?} group_by {group_by:?} distinct {distinct:?} maximum {maximum}\n rows {:?} incomplete {:?}\n native {native:?}\n sql    {sql:?}",
                relation.rows, relation.incomplete
            )),
        }
    }
    eprintln!(
        "setops budget: seed {seed:#x}, {cases} cases compared, {} differences, {refused} refused, {groups} groups, {:.1}s",
        diffs.count,
        started.elapsed().as_secs_f64()
    );
    diffs.assert_none("budget", seed);
}

/// An interrupted clause fails in SQL where it does natively, and answers where native visits no
/// row (and so never polls).
#[test]
fn interrupted_clauses_fail_alike() {
    let empty = Relation::default();
    let one = Relation {
        rows: vec![json!({ "k": 1 }).as_object().expect("object").clone()],
        incomplete: BTreeSet::new(),
    };
    let join = json!({ "eq": ["s.k", "e.k"] });
    for (subject, evidence, expected) in [
        (&empty, &empty, Ok(Vec::new())),
        (&empty, &one, Err(Interrupted)),
        (&one, &empty, Err(Interrupted)),
    ] {
        let native =
            crossfile::require_each(("s", subject), None, ("e", evidence), Some(&join), stopped);
        let sql = sql::crossfile::require_each(
            ("s", subject),
            None,
            ("e", evidence),
            Some(&join),
            stopped,
        )
        .expect("no SQL error");
        assert_eq!(native, expected);
        assert_eq!(sql, native);
    }
    let unseeking = crossfile::require_each(("s", &empty), None, ("e", &one), None, stopped);
    assert_eq!(unseeking, Ok(Vec::<Requirement>::new()));
    assert_eq!(
        sql::crossfile::require_each(("s", &empty), None, ("e", &one), None, stopped)
            .expect("no SQL error"),
        unseeking
    );
    for relation in [&empty, &one] {
        assert_eq!(
            sql::crossfile::budget(("u", relation), None, &[], &[], 0, stopped)
                .expect("no SQL error"),
            crossfile::budget(("u", relation), None, &[], &[], 0, stopped)
        );
    }
}

/// `lt`/`le`/`gt`/`ge` hold in SQL exactly where natively: of two strings by UTF-16 code units,
/// never of a missing or non-string operand, under `not` as well. The first predicate is
/// `data-layer-layer-violations`' `layer-0` budget, whose layer edges SQL once counted none of.
#[test]
fn ordered_predicates_answer_alike_in_sql() {
    let rows = [
        json!({ "path": "api/l2050_transaction/query.py", "start": 0, "from": "2050", "to": "3000" }),
        json!({ "path": "api/l2100_sql_grammar/a.py", "start": 0, "from": "2100", "to": "2200", "grammar": "l2100_sql_grammar" }),
        json!({ "path": "api/l2000_connection/connection_factory.py", "start": 0, "from": "2000", "to": "2200" }),
        json!({ "path": "api/l3000_high/a.py", "start": 0, "from": "3000", "to": "1000" }),
        json!({ "path": "api/l2050_transaction/b.py", "start": 0, "from": "2050" }),
        json!({ "path": "api/l2050_transaction/c.py", "start": 0, "from": 2050, "to": "3000" }),
        // U+FFFF is one code unit above the surrogate U+1F600 starts with, below it by code point.
        json!({ "path": "api/x.py", "start": 0, "from": "\u{ffff}", "to": "\u{1f600}" }),
    ];
    let relation = Relation {
        rows: rows
            .iter()
            .map(|row| row.as_object().expect("object").clone())
            .collect(),
        incomplete: BTreeSet::new(),
    };
    let group_by = ["edge.path".to_owned(), "edge.start".to_owned()];
    for (predicate, firsts) in [
        (
            json!({ "all": [
                { "lt": ["edge.from", "edge.to"] },
                { "not": { "all": [
                    { "eq": ["edge.grammar", "l2100_sql_grammar"] },
                    { "eq": ["edge.to", "2200"] },
                ] } },
            ] }),
            vec![0, 2],
        ),
        (
            json!({ "not": { "lt": ["edge.from", "edge.to"] } }),
            vec![3, 4, 5, 6],
        ),
        (json!({ "gt": ["edge.from", "edge.to"] }), vec![3, 6]),
        (json!({ "le": ["edge.from", "2050"] }), vec![0, 2, 4]),
        (json!({ "ge": ["3000", "edge.to"] }), vec![0, 1, 2, 3, 5]),
    ] {
        let native = crossfile::budget(
            ("edge", &relation),
            Some(&predicate),
            &group_by,
            &[],
            0,
            running,
        );
        let found: Vec<usize> = native
            .as_ref()
            .expect("not interrupted")
            .iter()
            .map(|group| group.first)
            .collect();
        assert_eq!(found, firsts, "native groups under {predicate}");
        let sql = sql::crossfile::budget(
            ("edge", &relation),
            Some(&predicate),
            &group_by,
            &[],
            0,
            running,
        )
        .expect("no SQL error");
        assert_eq!(sql, native, "SQL groups under {predicate}");
    }
}

// --- whole evaluations ------------------------------------------------------------------------

/// The stateless evaluator answers `select_when`, `require_each` and `budget` clauses alike under
/// both engines: regex and AST leaves, nested gates, a broken file read around its errors.
#[test]
fn set_operation_clauses_evaluate_alike_under_both_engines() {
    use crate::code_rule_dsl::{CodeRuleDslRuntime, test_support::CAPABILITIES};
    let violation = json!({ "anchor": "witness.span", "code": "E", "message": "set" });
    let ast = |kind: &str| json!({ "ast": { "language": "typescript", "rule": { "kind": kind } } });
    let regex =
        |pattern: &str| json!({ "regex": { "source": "file.content", "pattern": pattern } });
    let when = |exists: Value, select: Value| json!({ "select_when": { "exists": exists, "select": select } });
    let evidence =
        |alias: &str, include: &str, kind: &str, project: Value, filter: Option<Value>| {
            let mut relation = json!({
                "as": alias,
                "paths": { "include": [include] },
                "query": ast(kind),
                "project": project,
            });
            if let Some(filter) = filter {
                relation["where"] = filter;
            }
            json!({ "evidence": relation })
        };
    let clauses = json!([
        { "id": "gate", "violation": violation, "forbid": when(regex("danger\\("), ast("debugger_statement")) },
        { "id": "ast-gate", "violation": violation, "forbid": when(ast("debugger_statement"), regex("debugger")) },
        { "id": "nested", "violation": violation, "forbid": when(
            when(regex("danger"), ast("call_expression")),
            when(ast("debugger_statement"), ast("call_expression")),
        ) },
        { "id": "closed", "violation": violation, "forbid": when(regex("never-present"), ast("call_expression")) },
        { "id": "stories", "violation": violation, "require_each": {
            "subject": evidence("c", "**/*.ts", "function_declaration", json!({ "name": "capture.NAME", "dir": "file.directory" }), None),
            "evidence": evidence("s", "**/*.ts", "comment", json!({ "dir": "file.directory" }), Some(json!({ "all": [
                { "eq": ["c.dir", "s.dir"] },
                { "not": { "eq": ["c.name", "s.dir"] } },
            ] }))),
        } },
        { "id": "per-dir", "violation": violation, "budget": {
            "query": evidence("u", "**/*.ts", "call_expression", json!({ "dir": "file.directory", "path": "file.path" }), None),
            "group_by": ["u.dir"],
            "count": { "lte": 0, "distinct": ["u.path"] },
        } },
        // `data-layer-layer-violations`' shape: projected strings, an edge kept where one orders
        // below the other (`ordered_predicates_answer_alike_in_sql`).
        { "id": "ordered", "violation": violation, "budget": {
            "query": evidence("u", "**/*.ts", "call_expression", json!({
                "path": "file.path",
                "from": "file.directory",
                "to": { "source": "file.name", "regex": "^([a-z])", "group": 1 },
            }), Some(json!({ "lt": ["u.from", "u.to"] }))),
            "group_by": ["u.path"],
            "count": { "lte": 0 },
        } },
    ]);
    let code_rule = json!({
        "format": "rerail.policy/v1",
        "id": "set-operations",
        "revision": 1,
        "mode": "steer",
        "clauses": clauses,
    });
    let compiled = CodeRuleDslRuntime::compile(
        code_rule.to_string(),
        "json".to_owned(),
        CAPABILITIES.to_owned(),
    )
    .expect("policy compiles");
    for files in [
        vec![
            (
                "typescript",
                "a/one.ts",
                "danger();\ndebugger;\nfunction f() {}\n// note\n",
            ),
            ("typescript", "a/two.ts", "g();\ndebugger;\n"),
            ("typescript", "b/three.ts", "function h() { h(); }\n"),
        ],
        vec![
            (
                "typescript",
                "a/broken.ts",
                "debugger; // keeps the error region small\nfunction f( {\n",
            ),
            ("typescript", "b/danger.ts", "danger();\n"),
            ("typescript", "c/plain.ts", "x();\n// c\n"),
        ],
        vec![("typescript", "a/quiet.ts", "let x = 1;\n")],
    ] {
        let result = super::same(&compiled, &super::snapshot(&files));
        let listed = |key: &str, field: &str| -> Vec<String> {
            result[key]
                .as_array()
                .expect("a list")
                .iter()
                .map(|entry| format!("{}:{}", entry["clauseId"], entry[field]))
                .collect()
        };
        eprintln!(
            "set operations over {:?}: violations {:?}, failures {:?}",
            files.iter().map(|(_, path, _)| *path).collect::<Vec<_>>(),
            listed("violations", "path"),
            listed("failures", "code"),
        );
    }
}
