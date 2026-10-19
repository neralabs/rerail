//! `violation.each` in a three-valued clause: the exclusion wrappers over the operators that read
//! verdicts (`plan`'s contract).
//!
//! Natively, a round excludes at its choice points only, and only a *true* candidate: the trial
//! that matched is tested with `each::excluded` and passed over, bindings and all, as if it had
//! not matched; a candidate that did not match is never tested, so an *unknown* one still reads
//! `UNKNOWN` and counts as unknown. The SQL shape is therefore one wrapper, `Op::All([op,
//! Op::Excluded])` (`Lowerer::excluding`), emitted by `excluding_open`: of `op`'s rows, the true
//! ones whose binding of the enumerated capture has a range reported for the row's root are
//! dropped (no row is false), and every unknown one is kept with its verdict. The operator around
//! the wrapper then reads exactly the events the native search reads. Operator by operator:
//!
//! - **Relations** (`relation.rs` `Search::test` → `test_excluding`, `Search::open`;
//!   `Op::Search`): an excluded true candidate returns `Continue` before the `UNKNOWN` check, an
//!   unknown one reaches `open`. The relation's rule is wrapped; `search` then takes, strict
//!   (`may_bind`), the first event in walk order among the kept rows and the unknown stops, so a
//!   true candidate passed over before an unknown one leaves the unknown first and the search
//!   unknown; non-strict, true where a kept row is true, else unknown where one is unknown.
//! - **`any`** (`mod.rs` `match_any`; `Op::AnyOpen`): a true branch whose trial is excluded falls
//!   through to `UNKNOWN.replace(false)`, which a true match left as it found it (false), so the
//!   branch reads as false and the next branch runs; an unknown branch is never tested, and ends
//!   the `any` unknown where it may bind. Each branch is wrapped; `any_open` sees no row of an
//!   excluded branch and passes the row on with what earlier branches left (`r.unk`).
//! - **`all`** (`mod.rs` `match_all`; `Op::AllOpen`): excludes nothing itself; its terms'
//!   verdicts are their wrapped choice points', and withholding reads them as without `each`.
//! - **`not`**, **`count`**, **stop rules**, **`nthChild` probes** (`match_not` and
//!   `Count::holds` under `each::suspended`, `relation::stops_at`, `NthChild::index`'s probes):
//!   exclude nothing, three-valued or not; they lower with `exclude = None`. `count`'s interval
//!   (`count_open`) and an unknown sibling (`nth_child_open`) are read as without `each`.
//! - **`nthChild` with a binding `ofRule`** (`nth_child.rs` `NthChild::index`; `Op::NthCommit`):
//!   the position comes from the suspended probes, unknown where a sibling's is; then `ofRule` is
//!   re-matched at the node excluding and its verdict dropped. A false re-match leaves the
//!   position true with the input's bindings (`nth_commit_open`). An unknown one leaves `UNKNOWN`
//!   set past a true answer (the enclosing `all` then reads its next false term as unknown) or
//!   turns a failed position unknown: no frame verdict says that, so a three-valued `ofRule` that
//!   reads values is refused under `each` (`Unsupported(three-valued nthChild whose excluding
//!   commit may be unknown)`); without one the re-match cannot be unknown.
//! - **Calls** (`call.rs` `each::framed`): the capture is excluded in the frame under the
//!   parameter it is passed as; a frame it is not passed to excludes nothing. An unknown body is
//!   an unknown call (`Emitter::call`), as without `each`.
//! - **Recursion units** (`memo.rs` keys verdicts by `each::exclusion_key` and, through
//!   `memo::kept`, by the excluded capture): a unit under `each` is keyed by the row's root
//!   (bound only where a range is reported for it, `Op::EachRoot`, so a root that excludes
//!   nothing shares its keys) and, three-valued, by the excluded capture too, also where the
//!   utility does not hold it, and its body holds the wrappers; its answer carries the body's
//!   `unk`, and an unknown answer keeps the key's bindings, so a reference reads it as the
//!   matcher's `matches` does. Units are emptied between rounds, as the memo's generation
//!   changes with each recorded range.
//! - **Stages** (stop rules and capture-free `ofRule`s, matched once per batch excluding
//!   nothing; a `matches` is never staged, and a relation's rule is keyed, `emit/keyed.rs`): a
//!   capture-free rule binds nothing, but its choice points test the trial it runs on, so where
//!   the excluded capture may be bound on entry (`Lowerer::excluding_bound`) it is lowered in
//!   place, with its wrappers; elsewhere no row it runs on binds the capture and its stage is
//!   exact.
//! - **Rounds** (`run.rs` `enumerate`, `match_rule`): a root whose verdict is unknown does not
//!   match, so it is not visited and its enumeration ends. `clause_each` reads the matches with
//!   `unk = 0` only: an unknown root is neither reported nor kept for the next round.
//!
//! Why every sub-rule must read the exclusion in a three-valued clause, where a two-valued one
//! may read some without it: a row's bindings only grow (an unknown verdict keeps its input's),
//! so a row that carries a reported range out of every choice point ends its root unreported,
//! whatever the verdicts it met; but inside a choice point's trial that binds a reported range,
//! the trial's verdict still decides the search: true is passed over, and unknown is an event
//! (it stops a strict search, or leaves the answer open). Two-valued, true and false are both
//! passed over; three-valued, a sub-rule of the trial that read true or false where the matcher
//! reads unknown (or the reverse) would change the search.

use super::{EACH, Emitter, Reads};
use crate::code_rule_dsl::sql::plan::Op;

impl Emitter<'_> {
    /// `Op::All` in a three-valued statement, which lowers only `violation.each`'s two sequences
    /// to it: the root filter before the clause's operator, and an operator then its exclusion.
    pub(super) fn each_open(&mut self, items: &[Op], input: &str) -> String {
        match items {
            [root @ Op::EachRoot { .. }, op] => {
                let roots = self.op(root, input);
                self.op(op, &roots)
            }
            [op, Op::Excluded { name, root }] => self.excluding_open(op, *name, *root, input),
            _ => unreachable!("a three-valued clause lowers `all` as AllOpen"),
        }
    }

    /// `op`'s verdicts with the true rows that bind the enumerated capture (`name`) to a range
    /// reported for their root dropped: an excluded true candidate is passed over, an unknown one
    /// stays unknown.
    fn excluding_open(&mut self, op: &Op, name: usize, root: usize, input: &str) -> String {
        let out = self.op(op, input);
        self.frame(
            "x",
            &format!(
                "SELECT {} FROM {out} f LEFT JOIN node b ON b.file = f.file AND b.id = f.s{name} \
                 LEFT JOIN {EACH}x e ON e.file = f.file AND e.root = f.s{root} AND e.start = b.start AND e.\"end\" = b.\"end\" \
                 WHERE f.unk = 1 OR e.root IS NULL",
                self.verdict("f", &[], "f.unk")
            ),
            Reads::anti(&out, &[&format!("{EACH}x")]),
        )
    }

    /// `NthChild::index` under `each`: unknown where the position is (a sibling's probe is), with
    /// the input's bindings; elsewhere true where the position holds, with the bindings of
    /// `ofRule` re-matched at the node excluding (`commit`), or the input's where it does not
    /// match. `commit` is never unknown: the lowering refuses an `ofRule` that could be.
    pub(super) fn nth_commit_open(&mut self, nth: &Op, commit: &Op, input: &str) -> String {
        let positioned = self.op(nth, input);
        let passed = self.frame(
            "y",
            &format!(
                "SELECT {} FROM {input} f WHERE f.k IN (SELECT k FROM {positioned} WHERE unk = 0)",
                self.cols("f")
            ),
            Reads::join(&[input, &positioned]),
        );
        let committed = self.op(commit, &passed);
        let overrides: Vec<(String, String)> = self
            .physical
            .iter()
            .map(|column| {
                (
                    column.clone(),
                    format!("CASE WHEN c.k IS NOT NULL THEN c.{column} ELSE f.{column} END"),
                )
            })
            .collect();
        self.frame(
            "x",
            &format!(
                "SELECT {} FROM {passed} f LEFT JOIN {committed} c ON c.k = f.k AND c.unk = 0 \
                 UNION ALL SELECT {} FROM {input} f WHERE f.k IN (SELECT k FROM {positioned} WHERE unk = 1)",
                self.verdict("f", &overrides, "0"),
                self.verdict("f", &[], "1")
            ),
            Reads::join(&[input, &positioned]),
        )
    }
}
