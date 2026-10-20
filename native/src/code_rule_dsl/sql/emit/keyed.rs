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
//! recorded under, but where the module's last section takes it out), `violation.each`'s root
//! and enumerated slots. That is the same key the native matcher's per-file memo keys a
//! utility's answer by: the node and the input bindings restricted to the utility's scope, with
//! the exclusion state (the each root).
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
//!
//! A slot unbound in every row of the input (the root frame binds none, and an operator leaves
//! the slots it does not name as they were: `Emitter::nulls`) is one value, NULL, across it. So
//! the key leaves it out, and a frame unique on a set of slots is unique on the set without it.
//!
//! # Recursive reads from a key: pseudo-nodes
//!
//! In a unit's body every row carries the origin slot, the demand row it is evaluated for, which
//! names the key a recursive read is recorded from (`Emitter::recur`). It decides no verdict, so
//! a key may leave it out: the key then stands for every row sharing it, whatever key each is
//! evaluated for, and is evaluated once. Its frame's origin column numbers the key afresh
//! instead, a pseudo-node (`emit::PSEUDO`) no pass met before (`emit::PSEUDO_KEY_FUNCTION`),
//! which the reads under the key are recorded from; and each input row's key reads the
//! pseudo-node of its key, `(origin, key) → (PSEUDO, pseudo-node)`, from the rows `lineage`
//! needs.
//!
//! That pays only where many of the unit's keys reach one row: the key frame, the input rows'
//! reads into it and the back join cost a row each, and save the rows the keys share. So the
//! origin leaves the key only where both hold, decided from the plan alone:
//! - rows of distinct keys can meet (`Emitter::shared`): a walk between the unit's keys and the
//!   input leads distinct nodes to one (up the tree, along siblings, to a terminal's related
//!   node). Without one, a row's node descends from its own key's node alone and the rows that
//!   share a key are one key's;
//! - the operator, two-valued, reaches two recursive references or more, each demanded, answered
//!   and recorded once per key rather than per row (`serverplan_state_tripwires`' `rtauth` walk:
//!   491k candidates of 1.7k keys, the descendants of an ancestor many keys share). Such an
//!   operator is keyed even where it does not walk.
//!
//! Elsewhere the origin is part of the key, as any other slot the operator names.
//!
//! The check is exact (`exec::acyclic`, `lineage`'s argument). A key's pseudo-node is needed
//! where one of its rows is (`lineage::Derive::Keyed`), and its evaluation is the same for every
//! row: a path `s → p → d` holds exactly where a needed row evaluated for `s` reads `d` through
//! its key, and a pseudo-node is no hop of the depth limit. A pass's pseudo-nodes are new, so a
//! key's reads replaced by a later pass drop its reads into them, and the reads out of a
//! pseudo-node no kept read reaches decide nothing.

use super::{Effect, Emitter, Live, PSEUDO, PSEUDO_KEY_FUNCTION, Reads};
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
        Op::Kinds(_) | Op::Regex(_) | Op::Program(_) | Op::Never => {}
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
        Op::Recur {
            unit,
            slots,
            context,
        } => {
            match slots {
                Some(slots) => out.extend(slots.iter().copied()),
                None => out.extend(names.units[*unit].key.iter().copied()),
            }
            out.extend(*context);
            out.extend(names.origin);
        }
        Op::Follow {
            base,
            out: added,
            sub: rule,
            ..
        } => {
            out.extend([*base, *added]);
            sub(rule, out);
        }
        Op::EachRoot { root } => {
            out.insert(*root);
        }
        Op::Excluded { binding, root } => {
            out.extend(binding.slots());
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
        Op::Call { body, .. } | Op::Parse { body, .. } | Op::Follow { sub: body, .. } => {
            walks(body)
        }
        Op::Host { rule, .. } => walks(rule),
        Op::Constrained { rule, constraints } => {
            walks(rule) || constraints.iter().any(|(_, op)| walks(op))
        }
        Op::AllOpen(terms) => terms.iter().any(|term| walks(&term.op)),
        Op::AnyOpen(branches) => branches.iter().any(|(branch, _)| walks(branch)),
        _ => false,
    }
}

/// How many recursive references `op`'s tree holds.
fn references(op: &Op) -> usize {
    let mut referred = Vec::new();
    crate::code_rule_dsl::sql::lower::referred(op, &mut referred);
    referred.len()
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

    /// `op` on `input` per distinct key (the module doc), where it walks (or, two-valued, reaches
    /// two recursive references or more over rows several keys of its unit share) and `input`
    /// may hold several rows of one key, its output holding `live`; `None` where it is evaluated
    /// directly.
    pub(super) fn keyed(&mut self, op: &Op, input: &str, live: &Live) -> Option<String> {
        // Rows several keys of the unit share (`Emitter::shared`), read by two references or
        // more: keyed apart from the keys they are evaluated for (the module doc).
        let shared = !self.three && references(op) >= 2 && self.shared(input);
        if !keyed_operator(op) && !shared {
            return None;
        }
        let touched = self.touched(op);
        // There the origin decides no verdict: it names the key a read is recorded from. So no
        // key holds it, and each key stands for every row sharing it, whatever key the row is
        // evaluated for: a pseudo-node of the recursion's reads (the module doc).
        let origin = self
            .origin
            .filter(|origin| shared && touched.contains(origin));
        let named_slots: BTreeSet<SlotId> = touched
            .iter()
            .copied()
            .filter(|&slot| Some(slot) != origin)
            .collect();
        // A slot unbound in every input row is one value: no key needs it, and a frame unique on
        // a set of slots is unique on the set without it.
        let nulls = self.nulls_of(input);
        if self.unique.get(input).is_some_and(|sets| {
            sets.iter().any(|set| {
                set.difference(&nulls)
                    .all(|slot| named_slots.contains(slot))
            })
        }) {
            return None;
        }
        // Mutant 13: the key omits its first slot.
        let named: BTreeSet<SlotId> = if crate::code_rule_dsl::sql::mutant(13) {
            named_slots.iter().skip(1).copied().collect()
        } else {
            named_slots
        };
        let key: BTreeSet<SlotId> = named.difference(&nulls).copied().collect();
        let columns: Vec<String> = key.iter().flat_map(|&slot| self.columns(slot)).collect();
        let pseudo = origin.filter(|origin| !self.evaluating.is_empty() && !nulls.contains(origin));
        // The key frame: one row per distinct key, the other slots the operator names unbound
        // but the origin, which numbers its pseudo-node. Its rows return the key's slots of
        // `live`.
        let keys = self.name("q");
        let back = self.rebound(op, live);
        self.within(&touched, |this| {
            let frame = format!(
                "k INTEGER PRIMARY KEY, file, node{}{}",
                this.physical_list(),
                this.extra_list()
            );
            this.create(&keys, &frame);
            let indexed: String = columns.iter().map(|column| format!(", {column}")).collect();
            this.push(
                format!("CREATE INDEX IF NOT EXISTS {keys}_key ON {keys}(file, node{indexed})"),
                Effect::default(),
            );
            let mut values = String::new();
            for column in this.layout.columns.iter() {
                if columns.contains(column) {
                    write!(values, ", {column}")
                } else if pseudo.is_some_and(|origin| *column == format!("s{origin}")) {
                    write!(values, ", {PSEUDO_KEY_FUNCTION}()")
                } else {
                    values.write_str(", NULL")
                }
                .expect("write to a string");
            }
            let distinct: String = columns
                .iter()
                .map(|column| format!(", f.{column} AS {column}"))
                .collect();
            this.insert(
                &keys,
                format!(
                    "INSERT INTO {keys} SELECT NULL, file, node{values}{} FROM \
                     (SELECT DISTINCT f.file AS file, f.node AS node{distinct} FROM {input} f)",
                    this.zeros()
                ),
                Reads::join(&[input]),
            );
        });
        self.keyed_by(&keys, vec![key.clone()]);
        self.inherit_kinds(input, &keys);
        let unbound = touched
            .difference(&key)
            .copied()
            .filter(|&slot| Some(slot) != pseudo)
            .collect();
        self.nulls.insert(keys.clone(), Rc::new(unbound));
        // A key's rows meet no other key's: under a pseudo-node a walk must converge again.
        let reach = if pseudo.is_some() {
            super::lineage::Reach::Apart
        } else {
            super::lineage::Reach::Kept
        };
        self.open_space(
            &keys,
            input,
            super::lineage::Derive::Keyed {
                input: input.to_owned(),
                columns: columns.clone(),
            },
            reach,
        );
        if let (Some(origin), Some(&from)) = (pseudo, self.evaluating.last()) {
            // Each input row's key reads its pseudo-node: recorded from the needed rows, as the
            // reads under the key are from its own (`lineage`).
            let reads = super::reads_table(self.units[from].scc);
            let source = self.read_source(from, origin);
            let matched: String = columns
                .iter()
                .map(|column| format!(" AND q.{column} IS f.{column}"))
                .collect();
            self.record_read(
                &reads,
                input,
                &keys,
                format!(
                    "INSERT OR IGNORE INTO {reads} SELECT {source}, {PSEUDO}, q.s{origin} FROM {input} f"
                ),
                format!(" CROSS JOIN {keys} q ON q.file = f.file AND q.node = f.node{matched}"),
            );
            self.pseudo += 1;
            self.pseudo_levels = self.pseudo_levels.max(self.pseudo);
        }
        let out = self.op(op, &keys, &back);
        if pseudo.is_some() {
            self.pseudo -= 1;
        }
        // Each input row takes its key's verdict: the slots the operator names as it left them;
        // the input's unbound slots are not read.
        let mut overrides: Vec<(String, String)> = named
            .iter()
            .flat_map(|&slot| self.columns(slot))
            .map(|column| (column.clone(), format!("o.{column}")))
            .collect();
        for column in self.null_columns(input) {
            if !overrides.iter().any(|(name, _)| *name == column) {
                overrides.push((column, "NULL".to_owned()));
            }
        }
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
