//! Distinct-key evaluation: a sub-rule runs once per distinct key of its input frame, not once
//! per row, and its verdicts are joined back to the rows on the key.
//!
//! # The key, and why it is exact
//!
//! An operator's output row for an input row (`plan`'s contract) is a function of the row's
//! `file`, `node` and the bindings of the slots the operator reads or may bind, and of nothing
//! else of the row: its other slots pass through unchanged, and a three-valued operator ignores
//! the row's `unk` and `wh` (they belong to the `all` whose pipeline the row is in). The slots it
//! reads or may bind are among the slots it names anywhere in its tree (`touched`): a pattern's
//! variables, a terminal's reads and binds, a terminal walk's slot or its source's reads and the
//! slots it sets (`Walk::Slot`, `Walk::Related`), a gate's slot, a parsed tree's inner, host,
//! tree and export slots, a count's `as`, a call's caller and parameter slots and every slot of
//! its body, a recursive reference's key (and, in a unit's loop, the origin slot its read is
//! recorded under), `violation.each`'s root and enumerated slots. That is the
//! same key the native matcher's per-file memo keys a utility's answer by: the node and the input
//! bindings restricted to the utility's scope, with the exclusion state (the each root).
//!
//! So `keyed` takes the input's `DISTINCT (file, node, columns of touched)`, gives each key its
//! own `k`, runs the operator on that key frame (each key evaluated once, whatever the number of
//! input rows sharing it), and joins each input row to its key's output row on `(file, node,
//! key columns)`, comparing slots with `IS` (an unbound slot is NULL). The key columns of the
//! joined row come from the output (what the operator bound), the others from the input row.
//! An input row whose key has no output row is false and drops, as the operator would drop it.
//!
//! A per-batch stage (a stop rule, a capture-free `ofRule`: readers of the whole set) is the
//! zero-read case of the same mechanism: its rule names no slot, so its key is `(file, node)`
//! alone, and its domain is every node of its root kinds in the batch rather than the keys of one
//! input frame. Its table is keyed by `(file, node)`, and a frame reads it with the same equality
//! join. A `matches` is never a stage: its utility runs on the frame's keys (inlined, or a unit).
//!
//! # When a frame is already keyed
//!
//! Every operator maps an input row to at most one output row with the same `file` and `node`,
//! and leaves the slots it does not name as they were. So a frame unique on `(file, node, S)`
//! stays unique on it through an operator that names no slot of `S`. The root frame is unique on
//! `(file, node)`, a key frame on its key, a unit's demanded keys on the unit's key; a relation's
//! candidates are not. Where the input is unique on a subset of the key, it already holds one row
//! per key and is evaluated directly.

use super::{Effect, Emitter, Reads};
use crate::code_rule_dsl::sql::plan::{Export, OfRule, Op, SlotId, Source, Unit, Walk};
use std::collections::BTreeSet;
use std::fmt::Write;
use std::rc::Rc;

/// The slots and recursive units an operator's tree resolves against: a terminal walk's sources
/// (`Walk::Related`), the statement's units, and a unit loop's origin slot.
struct Names<'a> {
    sources: &'a [Source],
    units: &'a [Unit],
    origin: Option<SlotId>,
}

/// The slots a walk reads or writes into its candidates: a terminal walk's slot, or its source's
/// reads and sets.
fn walk_slots(walk: Walk, sources: &[Source], out: &mut BTreeSet<SlotId>) {
    match walk {
        Walk::Slot(slot) => {
            out.insert(slot);
        }
        Walk::Related(source) => {
            let source = &sources[source];
            out.extend(source.reads.iter().copied());
            out.extend(source.sets.iter().copied());
        }
        _ => {}
    }
}

/// A parsed tree's exports' inner and host slots.
fn export_slots(exports: &[Export], out: &mut BTreeSet<SlotId>) {
    for export in exports {
        out.insert(export.from);
        out.insert(export.to);
    }
}

/// Every slot `op` names in its tree (the module doc's key): a terminal walk's slots, and
/// `origin` too where it reaches a recursive unit.
fn touched(op: &Op, names: &Names, out: &mut BTreeSet<SlotId>) {
    let sub = |op: &Op, out: &mut BTreeSet<SlotId>| touched(op, names, out);
    match op {
        Op::Kinds(_) | Op::Regex(_) | Op::Never => {}
        Op::Pattern { vars, reads, .. } => {
            out.extend(vars.iter().copied());
            out.extend(reads.iter().copied());
        }
        Op::All(items) | Op::Any(items) | Op::Seq(items) => {
            items.iter().for_each(|item| sub(item, out));
        }
        Op::Not(inner) => sub(inner, out),
        Op::Relation {
            walk, sub: rule, ..
        }
        | Op::Search {
            walk, sub: rule, ..
        }
        | Op::Every { walk, sub: rule } => {
            walk_slots(*walk, names.sources, out);
            sub(rule, out);
        }
        Op::Count {
            walk,
            sub: rule,
            bind,
            ..
        } => {
            walk_slots(*walk, names.sources, out);
            sub(rule, out);
            out.extend(bind.iter().map(|(slot, _)| *slot));
        }
        Op::NthChild { of_rule, .. } => {
            if let Some(OfRule::Frame(rule)) = of_rule {
                sub(rule, out);
            }
        }
        Op::Call {
            entry,
            body,
            returns,
        } => {
            for copy in entry {
                out.insert(copy.callee);
                out.insert(copy.caller);
            }
            for ret in returns {
                out.insert(ret.caller);
                out.extend(ret.params.iter().copied());
            }
            sub(body, out);
        }
        Op::Constrained { rule, constraints } => {
            sub(rule, out);
            for (slot, constraint) in constraints {
                out.insert(*slot);
                sub(constraint, out);
            }
        }
        Op::Unknowable { op, .. } => sub(op, out),
        Op::AllOpen(terms) => terms.iter().for_each(|term| sub(&term.op, out)),
        Op::AnyOpen(branches) => branches.iter().for_each(|(branch, _)| sub(branch, out)),
        Op::Fact { reads, binds, .. } => {
            out.extend(reads.iter().copied());
            out.extend(binds.iter().copied());
        }
        Op::Recur { unit, slots } => {
            match slots {
                Some(slots) => out.extend(slots.iter().copied()),
                None => out.extend(names.units[*unit].key.iter().copied()),
            }
            out.extend(names.origin);
        }
        Op::EachRoot { root } => {
            out.insert(*root);
        }
        Op::Excluded { name, root } => {
            out.insert(*name);
            out.insert(*root);
        }
        Op::NthCommit { nth, commit } => {
            sub(nth, out);
            sub(commit, out);
        }
        Op::Gate(slot) => {
            out.insert(*slot);
        }
        Op::Parse {
            inner,
            host,
            tree,
            body,
            exports,
            ..
        } => {
            out.extend(inner.iter().copied());
            out.extend([host.0, host.1, *tree]);
            export_slots(exports, out);
            sub(body, out);
        }
        Op::Host {
            host,
            tree,
            rule,
            exports,
        } => {
            out.extend([host.0, host.1, *tree]);
            export_slots(exports, out);
            sub(rule, out);
        }
        Op::Exports { tree, exports } => {
            out.insert(*tree);
            export_slots(exports, out);
        }
    }
}

/// Whether `op` walks: it runs a sub-rule on candidates other than the row's node (a relation,
/// a count, an `ofRule` probe, `redefines.every`, a parsed tree's root, the host node), itself
/// or below a negation, call or sequence.
fn walks(op: &Op) -> bool {
    match op {
        Op::Relation { .. }
        | Op::Search { .. }
        | Op::Count { .. }
        | Op::Every { .. }
        | Op::NthCommit { .. }
        | Op::NthChild {
            of_rule: Some(OfRule::Frame(_)),
            ..
        } => true,
        Op::All(items) | Op::Any(items) | Op::Seq(items) => items.iter().any(walks),
        Op::Not(inner) | Op::Unknowable { op: inner, .. } => walks(inner),
        Op::Call { body, .. } | Op::Parse { body, .. } => walks(body),
        Op::Host { rule, .. } => walks(rule),
        Op::Constrained { rule, constraints } => {
            walks(rule) || constraints.iter().any(|(_, op)| walks(op))
        }
        Op::AllOpen(terms) => terms.iter().any(|term| walks(&term.op)),
        Op::AnyOpen(branches) => branches.iter().any(|(branch, _)| walks(branch)),
        _ => false,
    }
}

/// Whether `op` is evaluated per distinct key: an operator that walks, or a negation, call,
/// parsed tree or host rule over one. A pipeline (`all`, `any`, a sequence) is not: its terms
/// key themselves.
fn keyed_operator(op: &Op) -> bool {
    match op {
        Op::Relation { .. }
        | Op::Search { .. }
        | Op::Count { .. }
        | Op::Every { .. }
        | Op::NthCommit { .. }
        | Op::NthChild {
            of_rule: Some(OfRule::Frame(_)),
            ..
        } => true,
        Op::Not(inner) => walks(inner),
        Op::Call { body, .. } | Op::Parse { body, .. } => walks(body),
        Op::Host { rule, .. } => walks(rule),
        _ => false,
    }
}

impl Emitter<'_> {
    /// The slots `op` names (`touched`), computed once per operator.
    pub(super) fn touched(&mut self, op: &Op) -> Rc<BTreeSet<SlotId>> {
        let address = std::ptr::from_ref(op) as usize;
        if let Some(slots) = self.touched_slots.get(&address) {
            return Rc::clone(slots);
        }
        let mut slots = BTreeSet::new();
        let names = Names {
            sources: self.sources,
            units: self.units,
            origin: self.origin,
        };
        touched(op, &names, &mut slots);
        let slots = Rc::new(slots);
        self.touched_slots.insert(address, Rc::clone(&slots));
        slots
    }

    /// Records that the frame `name` holds one row per `(file, node, slots of a set)`, for each
    /// of `sets`.
    pub(super) fn keyed_by(&mut self, name: &str, sets: Vec<BTreeSet<SlotId>>) {
        self.unique.insert(name.to_owned(), sets);
    }

    /// Records that `to` holds a subset of `from`'s rows, one per row at most, with the same
    /// `file` and `node` and only the slots of `changed` rebound: it keeps `from`'s keys that do
    /// not meet `changed`. An operator that returns its input frame itself changes nothing.
    pub(super) fn inherit(&mut self, from: &str, to: &str, changed: &BTreeSet<SlotId>) {
        let Some(sets) = self.unique.get(from).filter(|_| from != to) else {
            return;
        };
        let kept: Vec<BTreeSet<SlotId>> = sets
            .iter()
            .filter(|set| set.is_disjoint(changed))
            .cloned()
            .collect();
        if !kept.is_empty() {
            self.unique.insert(to.to_owned(), kept);
        }
    }

    /// `op` on `input` per distinct key (the module doc), where it walks and `input` may hold
    /// several rows of one key; `None` where it is evaluated directly.
    pub(super) fn keyed(&mut self, op: &Op, input: &str) -> Option<String> {
        if !keyed_operator(op) {
            return None;
        }
        let key = self.touched(op);
        if self
            .unique
            .get(input)
            .is_some_and(|sets| sets.iter().any(|set| set.is_subset(&key)))
        {
            return None;
        }
        // Mutant 13: the key omits its first slot.
        let key = if crate::code_rule_dsl::sql::mutant(13) {
            Rc::new(key.iter().skip(1).copied().collect())
        } else {
            key
        };
        let columns: Vec<String> = key.iter().flat_map(|&slot| self.columns(slot)).collect();
        // The key frame: one row per distinct key, the other slots unbound.
        let keys = self.name("q");
        let frame = format!(
            "k INTEGER PRIMARY KEY, file, node{}{}",
            self.physical_list(),
            self.extra_list()
        );
        self.create(&keys, &frame);
        let indexed: String = columns.iter().map(|column| format!(", {column}")).collect();
        self.push(
            format!("CREATE INDEX IF NOT EXISTS {keys}_key ON {keys}(file, node{indexed})"),
            Effect::default(),
        );
        let mut values = String::new();
        for column in &self.physical {
            if columns.contains(column) {
                write!(values, ", {column}")
            } else {
                values.write_str(", NULL")
            }
            .expect("write to a string");
        }
        let distinct: String = columns
            .iter()
            .map(|column| format!(", f.{column} AS {column}"))
            .collect();
        self.insert(
            &keys,
            format!(
                "INSERT INTO {keys} SELECT NULL, file, node{values}{} FROM \
                 (SELECT DISTINCT f.file AS file, f.node AS node{distinct} FROM {input} f)",
                self.zeros()
            ),
            Reads::join(&[input]),
        );
        self.keyed_by(&keys, vec![(*key).clone()]);
        self.open_space(
            &keys,
            input,
            super::lineage::Derive::Keyed {
                input: input.to_owned(),
                columns: columns.clone(),
            },
        );
        let out = self.op(op, &keys);
        // Each input row takes its key's verdict: the key's slots as the operator left them.
        let overrides: Vec<(String, String)> = columns
            .iter()
            .map(|column| (column.clone(), format!("o.{column}")))
            .collect();
        // A keyed operator emits no withheld capture (its rows are verdicts, `plan`'s contract).
        let select = if self.three {
            self.verdict("f", &overrides, "o.unk")
        } else {
            self.select("f", &overrides)
        };
        let matched: String = columns
            .iter()
            .map(|column| format!(" AND q.{column} IS f.{column}"))
            .collect();
        Some(self.frame(
            "x",
            &format!(
                "SELECT {select} FROM {input} f CROSS JOIN {keys} q ON q.file = f.file AND q.node = f.node{matched} \
                 CROSS JOIN {out} o ON o.k = q.k"
            ),
            Reads::join(&[input, &keys, &out]),
        ))
    }
}
