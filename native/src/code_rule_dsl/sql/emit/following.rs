//! Emission of `Op::Follow`, the matcher's `FOLLOWING` guard (`binding::match_binding_nodes`):
//! a binding's site, value or annotation sub-rule fails where the binding sub-rules being matched
//! already hold it at its node, and otherwise runs with it added.
//!
//! # The interned contexts
//!
//! A following context is a set of `(node, rule)` pairs, stored as one element `x = node *
//! 65536 + rule` (`ELEMENT`; a statement numbers far fewer guarded sub-rules). The statement
//! interns the contexts it builds in two tables that live across its operators and passes:
//! `fs(id, key)`, each context once under its key, its elements in ascending order joined by
//! commas, and `fm(id, x)`, each context's elements. NULL is the empty context, which neither
//! holds. A context is the same set whatever the order its pairs were added in, so a unit's key
//! is the matcher's memo key less only that order, which no verdict reads.
//!
//! Each step is a join over the frame's rows: the guard is an anti-join of `(context, element)`
//! against `fm`; adding is a `GROUP BY` of each distinct `(context, element)` over the context's
//! elements and the new one, which gives the new key; the keys not interned yet are added with
//! their elements, and each row joins its `(context, element)` to its key's id.
//!
//! # Why the recursion terminates, and what the cycle check still refuses
//!
//! A unit of a following cycle is keyed by its context too (`plan::Unit::following`), and a
//! reference in the cycle passes the context of the guarded sub-rule it lies in. A recursive
//! reference under a guarded sub-rule therefore demands a key whose context strictly holds the
//! context of the key that reads it: the read graph's keys along such a path never repeat, as the
//! matcher's stack of binding nodes never does. A loop that passes no guarded sub-rule keeps its
//! context, so its keys repeat as before, and the cycle check (`exec::acyclic`) refuses it: the
//! matcher recurses there until its depth limit.

use super::{Effect, Emitter, Live, NAMESPACE, Reads};
use crate::code_rule_dsl::sql::plan::{Op, SlotId};
use std::collections::BTreeSet;

/// An element's node factor: the element of `(node, rule)` is `node * ELEMENT + rule`.
const ELEMENT: usize = 65536;

impl Emitter<'_> {
    /// `Op::Follow` (`follow`) on `input`: `sub` on the rows whose context (`base`) does not hold
    /// the node's `(node, rule)`, with `out` holding that context with it added; its output holds
    /// `live`.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn follow(
        &mut self,
        follow: &Op,
        rule: usize,
        base: SlotId,
        out: SlotId,
        sub: &Op,
        input: &str,
        live: &Live,
    ) -> String {
        // The rows `sub` runs on hold what it reads, and `base` and `out`.
        let held = self.live_before(follow, live);
        assert!(rule < ELEMENT, "a statement numbers its guarded sub-rules below {ELEMENT}");
        let sets = format!("{NAMESPACE}fs");
        let members = format!("{NAMESPACE}fm");
        for sql in [
            format!("CREATE TEMP TABLE IF NOT EXISTS {sets}(id INTEGER PRIMARY KEY, key)"),
            format!("CREATE INDEX IF NOT EXISTS {sets}_key ON {sets}(key)"),
            format!("CREATE TEMP TABLE IF NOT EXISTS {members}(id, x)"),
            format!("CREATE INDEX IF NOT EXISTS {members}_at ON {members}(id, x)"),
        ] {
            self.push(sql, Effect::default());
        }
        let element = format!("f.node * {ELEMENT} + {rule}");
        // Mutant 20: the guard is ignored, so a binding node is entered again under its rule.
        let guard = if crate::code_rule_dsl::sql::mutant(20) {
            "1".to_owned()
        } else {
            format!(
                "NOT EXISTS (SELECT 1 FROM {members} m WHERE m.id = f.s{base} AND m.x = {element})"
            )
        };
        let entered = self.within(&held, |this| {
            this.frame(
                "y",
                &format!("SELECT {} FROM {input} f WHERE {guard}", this.cols("f")),
                Reads::anti(input, &[&members]),
            )
        });
        self.inherit(input, &entered, &BTreeSet::new());
        self.same_space(input, &entered);
        // Each distinct (context, element) entered, and the key of the context with it added.
        let added = self.name("w");
        self.create(&added, "b, x, key");
        self.insert(
            &added,
            format!(
                "WITH q AS (SELECT DISTINCT f.s{base} AS b, {element} AS x FROM {entered} f) \
                 INSERT INTO {added} SELECT b, x, group_concat(e, ',' ORDER BY e) FROM \
                 (SELECT q.b AS b, q.x AS x, m.x AS e FROM q JOIN {members} m ON m.id = q.b \
                 UNION ALL SELECT b, x, x FROM q) GROUP BY b, x"
            ),
            Reads::join(&[&entered]),
        );
        // The elements of the keys not interned yet, then those keys and their elements.
        let fresh = self.name("w");
        self.create(&fresh, "key, e");
        self.insert(
            &fresh,
            format!(
                "INSERT INTO {fresh} SELECT a.key, m.x FROM {added} a JOIN {members} m ON m.id = a.b \
                 WHERE a.key NOT IN (SELECT key FROM {sets}) \
                 UNION SELECT a.key, a.x FROM {added} a WHERE a.key NOT IN (SELECT key FROM {sets})"
            ),
            Reads::join(&[&added]),
        );
        self.insert(
            &sets,
            format!("INSERT INTO {sets} SELECT NULL, key FROM {fresh} GROUP BY key"),
            Reads::join(&[&fresh]),
        );
        self.insert(
            &members,
            format!(
                "INSERT INTO {members} SELECT s.id, w.e FROM {fresh} w JOIN {sets} s ON s.key = w.key"
            ),
            Reads::join(&[&fresh, &sets]),
        );
        let following = self.within(&held, |this| {
            this.frame(
                "y",
                &format!(
                    "SELECT {} FROM {entered} f JOIN {added} a ON a.b IS f.s{base} AND a.x = {element} \
                     JOIN {sets} s ON s.key = a.key",
                    this.select("f", &[(format!("s{out}"), "s.id".to_owned())])
                ),
                Reads::join(&[&entered, &added, &sets]),
            )
        });
        self.inherit(&entered, &following, &BTreeSet::from([out]));
        self.same_space(&entered, &following);
        self.op(sub, &following, live)
    }
}
