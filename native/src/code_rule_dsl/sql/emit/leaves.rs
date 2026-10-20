//! Emission of the semantic leaves' operators: the walks over a terminal's related nodes
//! (`Walk::Slot`, `Walk::Related`), the sequences and gates their sub-rules run in (`Op::Seq`,
//! `Op::Gate`, `Op::Every`), and parsed trees (`Op::Parse`, `Op::Host`, `Op::Exports`).

use super::{Emitter, Reads};
use crate::code_rule_dsl::sql::plan::{Export, Op, SlotId, SlotKind, Walk};
use std::fmt::Write;

impl Emitter<'_> {
    /// A key value of a slot of `alias`, as the `fact` table decodes it: a node id, a list's
    /// `lo:hi`, a text's hexadecimal and its range, empty where unbound.
    pub(super) fn key_value(&self, alias: &str, slot: SlotId) -> String {
        let value = match self.slots[slot].kind {
            SlotKind::List => format!("{alias}.s{slot}a || ':' || {alias}.s{slot}b"),
            SlotKind::Text => format!("hex({alias}.s{slot}) || ':' || {alias}.s{slot}r"),
            SlotKind::Node | SlotKind::Count => format!("{alias}.s{slot}"),
        };
        format!("COALESCE({value}, '')")
    }

    /// The key of a fact term over `alias`'s rows: `file,node` (node 0 where the term does not
    /// read it) and its reads' values.
    fn fact_key(&self, alias: &str, reads: &[SlotId], at_node: bool) -> String {
        let node = if at_node {
            format!("{alias}.node")
        } else {
            "0".to_owned()
        };
        let mut key = format!("{alias}.file || ',' || {node}");
        for &slot in reads {
            write!(key, " || ',' || {}", self.key_value(alias, slot)).expect("write to a string");
        }
        key
    }

    /// The candidates of a terminal's walk, as `candidates` lays them out: the node a slot holds
    /// (`Walk::Slot`), or the related rows the fact terminal returns for each row's key, written
    /// into the slots the source sets (`Walk::Related`). `None` for a walk over the tree.
    pub(super) fn leaf_candidates(&mut self, walk: Walk, input: &str) -> Option<String> {
        let (with, select) = match walk {
            Walk::Slot(slot) => (
                String::new(),
                format!(
                    "SELECT NULL, f.file, f.s{slot}{}, f.k, 0{} FROM {input} f WHERE f.s{slot} IS NOT NULL",
                    self.physical_of("f"),
                    self.zeros()
                ),
            ),
            Walk::Related(source) => {
                let source = &self.sources[source];
                let key = self.fact_key("f", &source.reads, source.at_node);
                let mut values = String::from("v0, v1, v2");
                let mut overrides = Vec::new();
                let mut at = 3;
                for &slot in &source.sets {
                    for column in self.columns(slot) {
                        write!(values, ", v{at}").expect("write to a string");
                        overrides.push((column, format!("t.v{at}")));
                        at += 1;
                    }
                }
                let with = format!(
                    "WITH e AS MATERIALIZED (SELECT f.k AS k, {key} AS ek FROM {input} f), \
                     t AS MATERIALIZED (SELECT ek, {values} FROM fact WHERE term = {} AND ek IN (SELECT ek FROM e)) ",
                    source.term
                );
                let mut slots = String::new();
                for column in &self.physical {
                    match overrides.iter().find(|(name, _)| name == column) {
                        Some((_, value)) => write!(slots, ", {value}"),
                        None => write!(slots, ", f.{column}"),
                    }
                    .expect("write to a string");
                }
                (
                    with,
                    format!(
                        "SELECT NULL, COALESCE(t.v0, f.file), t.v1{slots}, f.k, t.v2{} \
                         FROM {input} f JOIN e ON e.k = f.k JOIN t ON t.ek = e.ek",
                        self.zeros()
                    ),
                )
            }
            _ => return None,
        };
        let name = self.name("g");
        let columns = format!(
            "k INTEGER PRIMARY KEY, file, node{}, pk, ord{}",
            self.physical_list(),
            self.extra_list()
        );
        self.create(&name, &columns);
        self.insert(
            &name,
            format!("{with}INSERT INTO {name} {select}"),
            Reads::join(&[input]),
        );
        Some(name)
    }

    /// `Op::Seq`: each item runs on the rows every item before it was true at; a row an item is
    /// unknown at ends unknown, with its input's bindings, and one it is false at ends.
    pub(super) fn seq(&mut self, items: &[Op], input: &str) -> String {
        if !self.three {
            let mut current = input.to_owned();
            for item in items {
                current = self.op(item, &current);
            }
            return current;
        }
        let mut current = input.to_owned();
        let mut unknown = Vec::new();
        let mut outs = Vec::new();
        for (index, item) in items.iter().enumerate() {
            let out = self.op(item, &current);
            unknown.push(format!("SELECT k FROM {out} WHERE unk = 1"));
            current = if index + 1 < items.len() {
                self.frame(
                    "y",
                    &format!("SELECT {} FROM {out} f WHERE f.unk = 0", self.cols("f")),
                    Reads::join(&[&out]),
                )
            } else {
                out.clone()
            };
            outs.push(out);
        }
        if unknown.len() == 1 {
            return current;
        }
        unknown.pop();
        outs.pop();
        // The last item's rows, and the input's rows an earlier item was unknown at.
        let mut branches = vec![current.as_str()];
        branches.extend(outs.iter().map(String::as_str));
        let reads = Reads::union(&branches);
        self.frame(
            "x",
            &format!(
                "SELECT {} FROM {current} f UNION ALL SELECT {} FROM {input} f WHERE f.k IN ({})",
                self.verdict("f", &[], "f.unk"),
                self.verdict("f", &[], "1"),
                unknown.join(" UNION ALL ")
            ),
            reads,
        )
    }

    /// `Op::Gate`: true where the slot holds `PASSES` (or nothing), unknown where it holds
    /// `OPEN`; a two-valued clause keeps the true rows.
    pub(super) fn gate(&mut self, slot: SlotId, input: &str) -> String {
        let state = format!("COALESCE(f.s{slot}, 0)");
        if !self.three {
            return self.frame(
                "x",
                &format!("SELECT {} FROM {input} f WHERE {state} = 0", self.cols("f")),
                Reads::join(&[input]),
            );
        }
        self.frame(
            "x",
            &format!(
                "SELECT {} FROM {input} f WHERE {state} IN (0, 1)",
                self.verdict("f", &[], &state.to_string())
            ),
            Reads::join(&[input]),
        )
    }

    /// `Op::Every`: the first candidate in walk order where the sub-rule is not true decides:
    /// false where it is false there, unknown where it is unknown; true where there is none.
    pub(super) fn every(&mut self, walk: Walk, sub: &Op, input: &str) -> String {
        let candidates = self.candidates(walk, None, input);
        let matched = self.op(sub, &candidates);
        let unk = if self.three { "s.unk" } else { "0" };
        let first = self.grouped(
            "st",
            &format!(
                "WITH v AS (SELECT g.pk AS pk, g.ord AS ord, CASE WHEN s.k IS NULL THEN 2 WHEN {unk} = 1 THEN 1 ELSE 0 END AS st \
                 FROM {candidates} g LEFT JOIN {matched} s ON s.k = g.k), \
                 w AS (SELECT pk, MIN(ord) AS o FROM v WHERE st > 0 GROUP BY pk) \
                 SELECT v.pk, MIN(v.st) FROM v JOIN w ON w.pk = v.pk AND w.o = v.ord GROUP BY v.pk"
            ),
            Reads::join(&[&candidates]),
        );
        let select = if self.three {
            format!(
                "SELECT {} FROM {input} f LEFT JOIN {first} x ON x.pk = f.k WHERE x.pk IS NULL OR x.st = 1",
                self.verdict("f", &[], "COALESCE(x.st, 0)")
            )
        } else {
            format!(
                "SELECT {} FROM {input} f LEFT JOIN {first} x ON x.pk = f.k WHERE x.pk IS NULL",
                self.cols("f")
            )
        };
        // The input's rows without a deciding candidate: every one where none decides.
        self.frame("x", &select, Reads::anti(input, &[&first]))
    }

    /// The CTE and joins that read `exports` of `alias`'s rows (the tree `tree` holds) from the
    /// `Exported` terminal, and the overrides of the host text slots they fill: an export the
    /// tree maps to no range leaves the slot as `keep`'s row holds it.
    fn export_reads(
        &self,
        alias: &str,
        keep: &str,
        tree: SlotId,
        exports: &[Export],
    ) -> (String, String, Vec<(String, String)>) {
        let Some(term) = exports.first().map(|export| export.term) else {
            return (String::new(), String::new(), Vec::new());
        };
        let keys: Vec<String> = exports
            .iter()
            .map(|export| {
                format!(
                    "SELECT {alias}.s{tree} || ',' || {alias}.s{} AS ek FROM {{INPUT}} {alias} WHERE {alias}.s{} IS NOT NULL",
                    export.from, export.from
                )
            })
            .collect();
        let with = format!(
            "x AS MATERIALIZED (SELECT ek, v0, v1 FROM fact WHERE term = {term} AND ek IN ({})) ",
            keys.join(" UNION ALL ")
        );
        let mut joins = String::new();
        let mut overrides = Vec::new();
        for (index, export) in exports.iter().enumerate() {
            let x = format!("x{index}");
            write!(
                joins,
                " LEFT JOIN x {x} ON {x}.ek = {alias}.s{tree} || ',' || {alias}.s{}",
                export.from
            )
            .expect("write to a string");
            let to = export.to;
            overrides.push((
                format!("s{to}"),
                format!("CASE WHEN {x}.ek IS NOT NULL THEN {x}.v0 ELSE {keep}.s{to} END"),
            ));
            overrides.push((
                format!("s{to}r"),
                format!("CASE WHEN {x}.ek IS NOT NULL THEN {x}.v1 ELSE {keep}.s{to}r END"),
            ));
        }
        (with, joins, overrides)
    }

    /// `Op::Exports`: the host text slots of the parsed captures, where they map to a range.
    pub(super) fn exports(&mut self, tree: SlotId, exports: &[Export], input: &str) -> String {
        let (with, joins, overrides) = self.export_reads("f", "f", tree, exports);
        if with.is_empty() {
            return input.to_owned();
        }
        let with = format!("WITH {}", with.replace("{INPUT}", input));
        let select = format!(
            "SELECT {} FROM {input} f{joins}",
            self.verdict_or_select("f", &overrides, "f.unk")
        );
        self.frame_with("x", &with, &select, Reads::join(&[input]))
    }

    /// A row of `alias` with `overrides`: its verdict `unk` kept where three-valued.
    fn verdict_or_select(&self, alias: &str, overrides: &[(String, String)], unk: &str) -> String {
        if self.three {
            self.verdict(alias, overrides, unk)
        } else {
            self.select(alias, overrides)
        }
    }

    /// `Op::Parse`: the tree's rows (`term`'s), the body at each tree's root in a frame of the
    /// row's own `k` (each row has one tree), and its matches joined back with the exports.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn parse(
        &mut self,
        term: usize,
        inner: &[SlotId],
        host: (SlotId, SlotId),
        tree: SlotId,
        body: &Op,
        exports: &[Export],
        input: &str,
    ) -> String {
        let key = self.fact_key("f", &[], true);
        let with = format!(
            "WITH e AS MATERIALIZED (SELECT f.k AS k, {key} AS ek FROM {input} f), \
             t AS MATERIALIZED (SELECT ek, unk, v0 FROM fact WHERE term = {term} AND ek IN (SELECT ek FROM e)) "
        );
        let parsed = self.name("p");
        self.create(&parsed, "k INTEGER PRIMARY KEY, unk, tree");
        self.insert(
            &parsed,
            format!(
                "{with}INSERT INTO {parsed} SELECT e.k, t.unk, t.v0 FROM e JOIN t ON t.ek = e.ek"
            ),
            Reads::join(&[input]),
        );
        let mut overrides: Vec<(String, String)> = Vec::new();
        for &slot in inner {
            for column in self.columns(slot) {
                overrides.push((column, "NULL".to_owned()));
            }
        }
        overrides.push((format!("s{}", host.0), "f.file".to_owned()));
        overrides.push((format!("s{}", host.1), "f.node".to_owned()));
        overrides.push((format!("s{tree}"), "p.tree".to_owned()));
        let mut entry = "p.k, p.tree, 0".to_owned();
        for column in &self.physical {
            match overrides.iter().find(|(name, _)| name == column) {
                Some((_, value)) => write!(entry, ", {value}"),
                None => write!(entry, ", f.{column}"),
            }
            .expect("write to a string");
        }
        entry.push_str(&self.zeros());
        let roots = self.frame(
            "i",
            &format!(
                "SELECT {entry} FROM {input} f JOIN {parsed} p ON p.k = f.k WHERE p.unk = 0 AND p.tree >= 0"
            ),
            Reads::join(&[input, &parsed]),
        );
        let matched = self.op(body, &roots);
        let (with, joins, exported) = self.export_reads("b", "f", tree, exports);
        let with = if with.is_empty() {
            String::new()
        } else {
            format!("WITH {}", with.replace("{INPUT}", &matched))
        };
        // The host row, with the exports: the body binds the tree's own slots only, which no
        // host operator reads.
        let (select, reads) = if self.three {
            (
                format!(
                    "SELECT {} FROM {matched} b JOIN {input} f ON f.k = b.k{joins} UNION ALL \
                     SELECT {} FROM {input} f JOIN {parsed} p ON p.k = f.k WHERE p.unk = 1",
                    self.verdict("f", &exported, "b.unk"),
                    self.verdict("f", &[], "1")
                ),
                Reads::union(&[&matched, &parsed]),
            )
        } else {
            (
                format!(
                    "SELECT {} FROM {matched} b JOIN {input} f ON f.k = b.k{joins}",
                    self.select("f", &exported)
                ),
                Reads::join(&[&matched, input]),
            )
        };
        self.frame_with("x", &with, &select, reads)
    }

    /// `Op::Host`: `rule` at the host node, in a frame of the row's own `k` holding the host's
    /// bindings and the tree's captures so far exported into its text slots; the row passes
    /// where the rule is true (unknown where it is unknown), binding nothing.
    pub(super) fn host(
        &mut self,
        host: (SlotId, SlotId),
        tree: SlotId,
        rule: &Op,
        exports: &[Export],
        input: &str,
    ) -> String {
        let (with, joins, overrides) = self.export_reads("f", "f", tree, exports);
        let with = if with.is_empty() {
            String::new()
        } else {
            format!("WITH {}", with.replace("{INPUT}", input))
        };
        let mut select = format!("f.k, f.s{}, f.s{}", host.0, host.1);
        for column in &self.physical {
            match overrides.iter().find(|(name, _)| name == column) {
                Some((_, value)) => write!(select, ", {value}"),
                None => write!(select, ", f.{column}"),
            }
            .expect("write to a string");
        }
        select.push_str(&self.zeros());
        let at_host = self.frame_with(
            "h",
            &with,
            &format!("SELECT {select} FROM {input} f{joins}"),
            Reads::join(&[input]),
        );
        let held = self.op(rule, &at_host);
        let unk = if self.three { "o.unk" } else { "0" };
        self.frame(
            "x",
            &format!(
                "SELECT {} FROM {input} f JOIN {held} o ON o.k = f.k",
                self.verdict_or_select("f", &[], unk)
            ),
            Reads::join(&[input, &held]),
        )
    }
}
