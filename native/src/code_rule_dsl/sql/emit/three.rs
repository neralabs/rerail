//! Three-valued emission (`plan`'s contract): the operators whose verdict can be unknown, over
//! frames that carry `unk` and the withheld-capture mask, and the input-set terminals' verdicts
//! (`Op::Fact`), which two-valued clauses read too.

use super::{Emitter, Reads, holds, stage_table};
use crate::code_rule_dsl::sql::nearest;
use crate::code_rule_dsl::sql::plan::{AllTerm, OfRule, Op, Stop, Walk};
use std::collections::BTreeSet;
use std::fmt::Write;

/// The order key of an event after every candidate of a walk: an unknown stop that ends it.
const LAST: &str = "4611686018427387904";

/// Word `word` of a mask; a mask holds no word past its highest bit.
fn word_of(mask: &[i64], word: usize) -> i64 {
    mask.get(word).copied().unwrap_or(0)
}

impl Emitter<'_> {
    /// The three-valued lowering of the operators that read a sub-rule's verdict; `None` for the
    /// others, whose rows are true wherever they emit one (leaves), or which read `unk`
    /// themselves (calls, terminals).
    pub(super) fn op3(&mut self, op: &Op, input: &str) -> Option<String> {
        Some(match op {
            Op::AllOpen(terms) => self.all_open(terms, input),
            Op::AnyOpen(branches) => self.any_open(branches, input),
            Op::Not(inner) => {
                if let Op::Not(rule) = &**inner {
                    return Some(self.holds_at(rule, input));
                }
                let out = self.op(inner, input);
                self.frame(
                    "x",
                    &format!(
                        "SELECT {} FROM {input} f LEFT JOIN {out} o ON o.k = f.k WHERE o.k IS NULL OR o.unk = 1",
                        self.verdict("f", &[], "COALESCE(o.unk, 0)")
                    ),
                    Reads::anti(input, &[&out]),
                )
            }
            Op::Search {
                walk,
                sub,
                binds,
                strict,
                kinds,
            } => self.search(*walk, kinds.as_deref(), sub, *binds, *strict, input),
            Op::Count {
                walk,
                sub,
                min,
                max,
                bind,
                kinds,
            } => self.count_open(*walk, kinds.as_deref(), sub, *min, *max, *bind, input),
            Op::NthChild {
                step,
                offset,
                reverse,
                of_rule: Some(of_rule),
            } => self.nth_child_open(*step, *offset, *reverse, of_rule, input),
            Op::Constrained { rule, constraints } => {
                self.constrained_open(rule, constraints, input)
            }
            Op::All(items) => self.each_open(items, input),
            Op::NthCommit { nth, commit } => self.nth_commit_open(nth, commit, input),
            Op::Any(_) | Op::Relation { .. } => {
                unreachable!("a three-valued clause lowers them as AnyOpen and Search")
            }
            _ => return None,
        })
    }

    /// `match_all`: each term runs on the rows no unknown term withholds it from; its verdict
    /// joins the row's accumulated `unk` and, where it is unknown and may bind, its bind mask
    /// joins `wh`. A withheld row passes the term untouched, its bind mask joining `wh`. The
    /// `all` is unknown, with its input's bindings, where any term was.
    fn all_open(&mut self, terms: &[AllTerm], input: &str) -> String {
        let mut current = input.to_owned();
        for (index, term) in terms.iter().enumerate() {
            let first = index == 0;
            // The rows that reach the term with a capture it mentions withheld.
            let held = (!first && term.checked)
                .then(|| {
                    term.mentions
                        .iter()
                        .enumerate()
                        .filter(|(_, bits)| **bits != 0)
                        .map(|(word, bits)| format!("(p.wh{word} & {bits}) <> 0"))
                        .collect::<Vec<_>>()
                        .join(" OR ")
                })
                // Mutant 4: no term is withheld.
                .filter(|held| !held.is_empty() && !crate::code_rule_dsl::sql::mutant(4));
            let run = match &held {
                Some(held) => self.frame(
                    "y",
                    &format!(
                        "SELECT {} FROM {current} p WHERE NOT ({held})",
                        self.cols("p")
                    ),
                    Reads::join(&[&current]),
                ),
                None => current.clone(),
            };
            self.inherit(&current, &run, &BTreeSet::new());
            let out = self.op(&term.op, &run);
            let mut select = if first {
                format!("SELECT o.k, o.file, o.node{}, o.unk", self.physical_of("o"))
            } else {
                format!(
                    "SELECT o.k, o.file, o.node{}, p.unk | o.unk",
                    self.physical_of("o")
                )
            };
            for word in 0..self.masks {
                let before = if first {
                    "0".to_owned()
                } else {
                    format!("p.wh{word}")
                };
                let bits = if term.withholds {
                    word_of(&term.binds, word)
                } else {
                    0
                };
                if bits == 0 {
                    write!(select, ", {before}")
                } else {
                    write!(
                        select,
                        ", {before} | CASE WHEN o.unk = 1 THEN {bits} ELSE 0 END"
                    )
                }
                .expect("write to a string");
            }
            write!(select, " FROM {out} o").expect("write to a string");
            if !first {
                write!(select, " JOIN {current} p ON p.k = o.k").expect("write to a string");
            }
            if let Some(held) = &held {
                write!(
                    select,
                    " UNION ALL SELECT p.k, p.file, p.node{}, p.unk",
                    self.physical_of("p")
                )
                .expect("write to a string");
                for word in 0..self.masks {
                    write!(select, ", p.wh{word} | {}", word_of(&term.binds, word))
                        .expect("write to a string");
                }
                write!(select, " FROM {current} p WHERE {held}").expect("write to a string");
            }
            // `v`, not `a`: `a{unit}` is a recursive unit's answer table (`emit::answer_table`).
            // The term's rows, and with `held` the current rows it withholds from.
            let reads = if held.is_some() {
                Reads::join(&[&current])
            } else {
                Reads::join(&[&out])
            };
            let next = self.frame("v", &select, reads);
            let changed = self.touched(&term.op);
            self.inherit(&current, &next, &changed);
            current = next;
        }
        if !terms.iter().any(|term| term.op.binds()) {
            return current;
        }
        // An unknown `all` commits nothing: its row keeps the input's bindings.
        let overrides: Vec<(String, String)> = self
            .physical
            .iter()
            .map(|column| {
                (
                    column.clone(),
                    format!("CASE WHEN c.unk = 1 THEN f.{column} ELSE c.{column} END"),
                )
            })
            .collect();
        self.frame(
            "x",
            &format!(
                "SELECT {} FROM {current} c JOIN {input} f ON f.k = c.k",
                self.verdict("c", &overrides, "c.unk")
            ),
            Reads::join(&[&current, input]),
        )
    }

    /// `match_any`: branch by branch from the input bindings, over the rows no earlier branch
    /// decided. A true branch decides with its bindings; an unknown one decides unknown where it
    /// may bind, and otherwise lets later branches decide, the row remembering it (`unk` of the
    /// rows still open). Rows no branch matched are unknown where one was.
    fn any_open(&mut self, branches: &[(Op, bool)], input: &str) -> String {
        let mut rest = input.to_owned();
        let mut parts = Vec::new();
        // The frames the parts read: each branch's output, and the last remainder.
        let mut frames = Vec::new();
        for (index, (branch, may_bind)) in branches.iter().enumerate() {
            let before = if index == 0 { "0" } else { "r.unk" };
            let out = self.op(branch, &rest);
            parts.push(format!(
                "SELECT {} FROM {out} o WHERE o.unk = 0",
                self.verdict("o", &[], "0")
            ));
            if *may_bind {
                parts.push(format!(
                    "SELECT {} FROM {out} o JOIN {rest} r ON r.k = o.k WHERE o.unk = 1",
                    self.verdict("r", &[], "1")
                ));
            }
            // The rows an unknown verdict of this branch leaves to the next.
            let open = if *may_bind { "0" } else { "o.unk = 1" };
            if index + 1 == branches.len() {
                parts.push(format!(
                    "SELECT {} FROM {rest} r LEFT JOIN {out} o ON o.k = r.k WHERE (o.k IS NULL AND {before} = 1) OR ({open})",
                    self.verdict("r", &[], "1")
                ));
            } else {
                let next = self.frame(
                    "y",
                    &format!(
                        "SELECT {} FROM {rest} r LEFT JOIN {out} o ON o.k = r.k WHERE o.k IS NULL OR ({open})",
                        self.verdict("r", &[], &format!("({before} | (o.k IS NOT NULL))"))
                    ),
                    Reads::anti(&rest, &[&out]),
                );
                self.inherit(&rest, &next, &BTreeSet::new());
                rest = next;
            }
            frames.push(out);
        }
        frames.push(rest);
        let reads: Vec<&str> = frames.iter().map(String::as_str).collect();
        let reads = Reads::union(&reads);
        self.frame("x", &parts.join(" UNION ALL "), reads)
    }

    /// `relation::Search` over a walk: a strict search ends at its first candidate that is true
    /// or unknown, or at an unknown stop; any other is true where a candidate is (the first one
    /// binds) and otherwise unknown where a candidate or a stop was.
    fn search(
        &mut self,
        walk: Walk,
        kinds: Option<&[u16]>,
        sub: &Op,
        binds: bool,
        strict: bool,
        input: &str,
    ) -> String {
        let candidates = self.candidates(walk, self.narrowed(walk, kinds), input);
        let matched = self.op(sub, &candidates);
        let opened = self.opened(walk, input, &candidates);
        // Every event is a matched candidate's or an unknown stop's.
        let reads = match &opened {
            Some(opened) => Reads::union(&[&matched, opened]),
            None => Reads::join(&[input, &matched]),
        };
        let physical = self.physical.clone();
        let from = |alias: &str, test: &str| -> Vec<(String, String)> {
            if !binds {
                return Vec::new();
            }
            physical
                .iter()
                .map(|column| {
                    (
                        column.clone(),
                        format!("CASE WHEN {test} THEN {alias}.{column} ELSE f.{column} END"),
                    )
                })
                .collect()
        };
        // Mutant 5: a strict search runs on past an unknown candidate.
        if strict && !crate::code_rule_dsl::sql::mutant(5) {
            // Events in walk order: each candidate's test, then an unknown stop's (`opened`).
            let stops = opened.map_or(String::new(), |opened| {
                let nulls: String = self.physical.iter().map(|_| ", NULL").collect();
                format!(" UNION ALL SELECT pk, e, 1{nulls} FROM {opened}")
            });
            let with = format!(
                "WITH v AS (SELECT g.pk AS pk, 2 * g.ord AS e, s.unk AS uk{} FROM {matched} s JOIN {candidates} g ON g.k = s.k{stops}), \
                 w AS (SELECT pk, MIN(e) AS e, uk{} FROM v GROUP BY pk) ",
                self.physical_of("s"),
                self.physical_list()
            );
            let select = format!(
                "SELECT {} FROM {input} f JOIN w ON w.pk = f.k",
                self.verdict("f", &from("w", "w.uk = 0"), "w.uk")
            );
            return self.frame_with("x", &with, &select, reads);
        }
        let stops = opened.map_or(String::new(), |opened| {
            format!(" UNION SELECT pk FROM {opened}")
        });
        let first = self.grouped(
            &format!("o{}", self.physical_list()),
            &format!(
                "SELECT g.pk, MIN(g.ord){} FROM {matched} s JOIN {candidates} g ON g.k = s.k WHERE s.unk = 0 GROUP BY g.pk",
                self.physical_of("s")
            ),
            Reads::join(&[&matched, &candidates]),
        );
        let with = format!(
            "WITH u AS (SELECT g.pk AS pk FROM {matched} s JOIN {candidates} g ON g.k = s.k WHERE s.unk = 1{stops}) "
        );
        let select = format!(
            "SELECT {} FROM {input} f LEFT JOIN {first} t ON t.pk = f.k WHERE t.pk IS NOT NULL OR f.k IN (SELECT pk FROM u)",
            self.verdict("f", &from("t", "t.pk IS NOT NULL"), "(t.pk IS NULL)")
        );
        self.frame_with("x", &with, &select, reads)
    }

    /// The kinds a three-valued walk's candidates may be narrowed to: none where the walk has an
    /// unknown stop (`opened`), whose events are placed among every candidate the walk visits.
    fn narrowed<'k>(&self, walk: Walk, kinds: Option<&'k [u16]>) -> Option<&'k [u16]> {
        kinds.filter(|_| self.open_stop(walk).is_none())
    }

    /// The stop-rule stage of `walk` whose verdict may be unknown, where it has one.
    fn open_stop(&self, walk: Walk) -> Option<usize> {
        let (Walk::Has(Stop::Rule(stage))
        | Walk::Inside(Stop::Rule(stage))
        | Walk::Precedes(Stop::Rule(stage))
        | Walk::Follows(Stop::Rule(stage))
        | Walk::FieldHas(_, Stop::Rule(stage))
        | Walk::FieldInside(_, Stop::Rule(stage))) = walk
        else {
            return None;
        };
        self.open.contains(&stage).then_some(stage)
    }

    /// The unknown stops of a walk whose stop rule may be unknown: `(pk, e)`, `e` the order key
    /// of the event among the candidates' (`2 * ord`). A walk along a link ends at its unknown
    /// stop, after every candidate; `has` passes over the stop's subtree and reads the unknown
    /// after testing the next node it visits (`has_until`); `has` with a field reads the field
    /// child's stop only; `inside` with a field ends at its unknown stop whether or not it tests
    /// it. `None` where the walk has no such stop.
    fn opened(&mut self, walk: Walk, input: &str, candidates: &str) -> Option<String> {
        let stage = self.open_stop(walk)?;
        let table = stage_table(stage);
        let unknown = |alias: &str| {
            format!(
                "JOIN {table} st ON st.file = {alias}.file AND st.node = {alias}.node AND st.unk = 1"
            )
        };
        let (with, select) = match walk {
            Walk::Has(_) => (
                format!(
                    "WITH q AS (SELECT pk, file, node, LEAD(ord) OVER (PARTITION BY pk ORDER BY ord) AS nx FROM {candidates}) "
                ),
                format!(
                    "SELECT q.pk, COALESCE(2 * q.nx + 1, {LAST}) FROM q {}",
                    unknown("q")
                ),
            ),
            Walk::Inside(_) | Walk::Precedes(_) | Walk::Follows(_) => (
                String::new(),
                format!("SELECT g.pk, {LAST} FROM {candidates} g {}", unknown("g")),
            ),
            Walk::FieldHas(..) => (
                String::new(),
                format!(
                    "SELECT g.pk, {LAST} FROM {candidates} g JOIN {input} f ON f.k = g.pk \
                     JOIN node c ON c.file = g.file AND c.id = g.node {} WHERE c.parent = f.node",
                    unknown("g")
                ),
            ),
            // Its walk ends at `f`'s nearest proper ancestor in `S`.
            Walk::FieldInside(..) => (
                String::new(),
                format!(
                    "SELECT f.k, {LAST} FROM {input} f CROSS JOIN nearest nf ON {} AND nf.id = f.node \
                     JOIN {table} st ON st.file = f.file AND st.node = nf.anc AND st.unk = 1",
                    nearest::set_of("nf", stage, "f.file")
                ),
            ),
            Walk::Slot(_) | Walk::Related(_) => return None,
        };
        // The stops are candidates' (or, `inside` with a field, the walk's from the input).
        let reads = match walk {
            Walk::FieldInside(..) => Reads::join(&[input, &table]),
            _ => Reads::join(&[candidates, &table]),
        };
        let name = self.name("o");
        self.create(&name, "pk, e");
        self.insert(&name, format!("{with}INSERT INTO {name} {select}"), reads);
        Some(name)
    }

    /// `Count::holds`: the true count lies in `t..=t + u` (`t` true, `u` unknown candidates),
    /// unbounded above where an unknown stop ended the walk. True where every value in it holds
    /// (and, with `as`, where it is exact), false where none does, unknown otherwise.
    #[allow(clippy::too_many_arguments)]
    fn count_open(
        &mut self,
        walk: Walk,
        kinds: Option<&[u16]>,
        sub: &Op,
        min: usize,
        max: Option<usize>,
        bind: Option<(usize, i64)>,
        input: &str,
    ) -> String {
        let candidates = self.candidates(walk, self.narrowed(walk, kinds), input);
        let matched = self.op(sub, &candidates);
        let opened = self.opened(walk, input, &candidates);
        let open = opened.map_or("0".to_owned(), |opened| {
            format!("(f.k IN (SELECT pk FROM {opened}))")
        });
        // Mutant 6: an unknown candidate counts as false.
        let unknown = if crate::code_rule_dsl::sql::mutant(6) {
            "0"
        } else {
            "SUM(s.unk = 1)"
        };
        let counts = self.grouped(
            "t, u",
            &format!(
                "SELECT g.pk, SUM(s.unk = 0), {unknown} FROM {matched} s JOIN {candidates} g ON g.k = s.k GROUP BY g.pk"
            ),
            Reads::join(&[&matched, &candidates]),
        );
        let (t, u) = ("COALESCE(c.t, 0)", "COALESCE(c.u, 0)");
        let exact = if bind.is_some() {
            format!("(NOT {open} AND {u} = 0)")
        } else {
            "1".to_owned()
        };
        let upper = max.map_or("1".to_owned(), |max| {
            format!("(NOT {open} AND {t} + {u} <= {max})")
        });
        let holds = format!("({exact} AND {t} >= {min} AND {upper})");
        let above = max.map_or(String::new(), |max| format!(" OR {t} > {max}"));
        let fails = format!("((NOT {open} AND {t} + {u} < {min}){above})");
        let overrides: Vec<(String, String)> = bind
            .iter()
            .map(|(slot, offset)| {
                (
                    format!("s{slot}"),
                    format!("CASE WHEN {holds} THEN {t} + ({offset}) ELSE f.s{slot} END"),
                )
            })
            .collect();
        let select = format!(
            "SELECT {} FROM {input} f LEFT JOIN {counts} c ON c.pk = f.k WHERE {holds} OR NOT {fails}",
            self.verdict("f", &overrides, &format!("(NOT {holds})"))
        );
        // A row with no candidate counts `t = u = 0`, which may hold.
        self.frame("x", &select, Reads::join(&[input]))
    }

    /// `NthChild::index` with `ofRule`: unknown where a named sibling's probe is (before the
    /// position is read); otherwise the node's rank among the siblings `ofRule` is true at, its
    /// own bindings under `ofRule` committed.
    fn nth_child_open(
        &mut self,
        step: i32,
        offset: i32,
        reverse: bool,
        of_rule: &OfRule,
        input: &str,
    ) -> String {
        let index = if reverse { "(r.cnt - r.r + 1)" } else { "r.r" };
        let position = holds(index, step, offset);
        match of_rule {
            OfRule::Stage(stage) => {
                // Over the input's parents' children alone (`Emitter::nth_child`).
                let table = stage_table(*stage);
                // Mutant 19: ranks over the whole stage, not the input's parents' children.
                let ranked = if crate::code_rule_dsl::sql::mutant(19) {
                    format!(
                        "r AS (SELECT n.file AS file, n.id AS node, \
                         ROW_NUMBER() OVER (PARTITION BY n.file ORDER BY n.id) AS r, \
                         COUNT(*) OVER (PARTITION BY n.file) AS cnt FROM {table} st \
                         JOIN node n ON n.file = st.file AND n.id = st.node \
                         WHERE n.named = 1 AND n.parent IS NOT NULL AND st.unk = 0)"
                    )
                } else {
                    format!(
                        "r AS (SELECT c.file AS file, c.id AS node, \
                         ROW_NUMBER() OVER (PARTITION BY c.file, c.parent ORDER BY c.id) AS r, \
                         COUNT(*) OVER (PARTITION BY c.file, c.parent) AS cnt FROM p \
                         JOIN node c ON c.file = p.file AND c.parent = p.parent \
                         JOIN {table} st ON st.file = c.file AND st.node = c.id \
                         WHERE c.named = 1 AND st.unk = 0)"
                    )
                };
                let with = format!(
                    "WITH p AS MATERIALIZED (SELECT DISTINCT n.file AS file, n.parent AS parent \
                     FROM {input} f JOIN node n ON n.file = f.file AND n.id = f.node \
                     WHERE n.parent IS NOT NULL), \
                     {ranked}, \
                     u AS (SELECT DISTINCT c.file AS file, c.parent AS parent FROM p \
                     JOIN node c ON c.file = p.file AND c.parent = p.parent \
                     JOIN {table} st ON st.file = c.file AND st.node = c.id \
                     WHERE st.unk = 1 AND c.named = 1) "
                );
                let select = format!(
                    "SELECT {} FROM {input} f JOIN r ON r.file = f.file AND r.node = f.node \
                     JOIN node fn ON fn.file = f.file AND fn.id = f.node \
                     LEFT JOIN u ON u.file = f.file AND u.parent = fn.parent \
                     WHERE u.parent IS NULL AND {position} \
                     UNION ALL SELECT {} FROM {input} f JOIN node fn ON fn.file = f.file AND fn.id = f.node \
                     JOIN u ON u.file = f.file AND u.parent = fn.parent",
                    self.verdict("f", &[], "0"),
                    self.verdict("f", &[], "1")
                );
                self.frame_with("x", &with, &select, Reads::join(&[input, &table]))
            }
            OfRule::Frame(rule) => {
                let siblings = self.name("g");
                let columns = format!(
                    "k INTEGER PRIMARY KEY, file, node{}, pk, ord{}",
                    self.physical_list(),
                    self.extra_list()
                );
                self.create(&siblings, &columns);
                self.insert(
                    &siblings,
                    format!(
                        "INSERT INTO {siblings} SELECT NULL, c.file, c.id{}, f.k, c.id{} FROM {input} f \
                         JOIN node fn ON fn.file = f.file AND fn.id = f.node \
                         JOIN node c ON c.file = f.file AND c.parent = fn.parent WHERE c.named = 1",
                        self.physical_of("f"),
                        self.zeros()
                    ),
                    Reads::join(&[input]),
                );
                let matched = self.op(rule, &siblings);
                let with = format!(
                    "WITH r AS (SELECT g.pk AS pk, g.node AS node, \
                     ROW_NUMBER() OVER (PARTITION BY g.pk ORDER BY g.node) AS r, \
                     COUNT(*) OVER (PARTITION BY g.pk) AS cnt{} FROM {matched} s JOIN {siblings} g ON g.k = s.k \
                     WHERE s.unk = 0), \
                     u AS (SELECT DISTINCT g.pk AS pk FROM {matched} s JOIN {siblings} g ON g.k = s.k WHERE s.unk = 1) ",
                    self.physical_of("s")
                );
                let select = format!(
                    "SELECT {} FROM {input} f JOIN r ON r.pk = f.k AND r.node = f.node \
                     WHERE f.k NOT IN (SELECT pk FROM u) AND {position} \
                     UNION ALL SELECT {} FROM {input} f WHERE f.k IN (SELECT pk FROM u)",
                    self.verdict("f", &self.all_from("r"), "0"),
                    self.verdict("f", &[], "1")
                );
                // Both parts read the siblings `ofRule` matched (`r`, `u`).
                self.frame_with("x", &with, &select, Reads::join(&[input, &matched]))
            }
        }
    }

    /// `Constrained::match_node`: an unknown rule ends unknown before any constraint; each
    /// constraint then runs at its bound capture's node, an unknown one ending the root unknown.
    fn constrained_open(&mut self, rule: &Op, constraints: &[(usize, Op)], input: &str) -> String {
        let mut current = self.op(rule, input);
        for (slot, constraint) in constraints {
            let column = format!("s{slot}");
            let passed = self.frame(
                "y",
                &format!(
                    "SELECT {} FROM {current} f WHERE f.unk = 1 OR f.{column} IS NULL",
                    self.verdict("f", &[], "f.unk")
                ),
                Reads::join(&[&current]),
            );
            let captured = self.frame(
                "g",
                &format!(
                    "SELECT f.k, f.file, f.{column}{}{} FROM {current} f WHERE f.unk = 0 AND f.{column} IS NOT NULL",
                    self.physical_of("f"),
                    self.zeros()
                ),
                Reads::join(&[&current]),
            );
            let checked = self.op(constraint, &captured);
            current = self.frame(
                "x",
                &format!(
                    "SELECT {} FROM {checked} c JOIN {current} f ON f.k = c.k UNION ALL SELECT {} FROM {passed} p",
                    self.verdict("f", &self.all_from("c"), "c.unk"),
                    self.verdict("p", &[], "p.unk")
                ),
                Reads::union(&[&checked, &passed]),
            );
        }
        current
    }

    /// An input-set terminal's verdicts on the frame: its distinct keys, `file,node` (the node
    /// `0` where the terminal does not read it) and the bindings of `reads` (`lo:hi` for a list,
    /// empty where unbound), go to the `fact` table in one `xFilter`; it returns the keys that are
    /// true or unknown, with the values of `binds`. A two-valued clause keeps the true ones.
    pub(super) fn fact(
        &mut self,
        term: usize,
        reads: &[usize],
        at_node: bool,
        binds: &[usize],
        input: &str,
    ) -> String {
        let node = if at_node { "f.node" } else { "0" };
        let mut key = format!("f.file || ',' || {node}");
        for &slot in reads {
            write!(key, " || ',' || {}", self.key_value("f", slot)).expect("write to a string");
        }
        let mut overrides = Vec::new();
        let mut values = String::new();
        for &slot in binds {
            for column in self.columns(slot) {
                write!(values, ", v{}", overrides.len()).expect("write to a string");
                overrides.push((column, format!("t.v{}", overrides.len())));
            }
        }
        let with = format!(
            "WITH e AS MATERIALIZED (SELECT f.k AS k, {key} AS ek FROM {input} f), \
             t AS MATERIALIZED (SELECT ek, unk{values} FROM fact WHERE term = {term} AND ek IN (SELECT ek FROM e)) "
        );
        let (select, known) = if self.three {
            (self.verdict("f", &overrides, "t.unk"), "")
        } else {
            (self.select("f", &overrides), " WHERE t.unk = 0")
        };
        self.frame_with(
            "x",
            &with,
            &format!(
                "SELECT {select} FROM {input} f JOIN e ON e.k = f.k JOIN t ON t.ek = e.ek{known}"
            ),
            Reads::join(&[input]),
        )
    }
}
