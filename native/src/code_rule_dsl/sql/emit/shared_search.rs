//! Benchmark-only first-match composition through a shared, indexed eligible-node relation.

use super::{Effect, Emitter, Live, Op, Reads, Stop, Walk, Writes, list, shape_test};
use std::collections::BTreeSet;

/// These bodies never read the input environment or return unknown. Fresh patterns may bind
/// captures; a bound pattern or any richer operator retains the original lowering.
fn independent(op: &Op) -> bool {
    match op {
        Op::Kinds(_) | Op::Regex(_) | Op::Never => true,
        Op::Pattern { env: false, .. } => true,
        Op::All(items) => items.iter().all(independent),
        Op::AllOpen(terms) => terms.iter().all(|term| independent(&term.op)),
        _ => false,
    }
}

impl Emitter<'_> {
    pub(super) fn shared_search(&mut self, op: &Op, input: &str, live: &Live) -> Option<String> {
        let (sub, need, binds) = match op {
            Op::Relation {
                walk: Walk::Has(Stop::End),
                sub,
                need,
                binds,
            } => (sub, need, *binds),
            Op::Search {
                walk: Walk::Has(Stop::End),
                sub,
                need,
                ..
            } => (sub, need, sub.binds()),
            _ => return None,
        };
        if !independent(sub)
            || need.kinds.as_ref().is_some_and(Vec::is_empty)
            || super::super::mutant(1)
        {
            return None;
        }
        let back = if binds {
            self.rebound(op, live)
        } else {
            Live::default()
        };
        let held = self.live_before(sub, &back);
        let domain = self.within(&held, |this| {
            let nulls: String = this.layout.columns.iter().map(|_| ", NULL").collect();
            let kinds = need.kinds.as_ref().map_or(String::new(), |kinds| {
                if kinds.is_empty() { " AND 0".into() }
                else { format!(" AND n.kind IN ({})", list(kinds)) }
            });
            let shape = need.shape.as_ref().map_or(String::new(), |shape| {
                format!(" AND ({})", shape_test(shape, "n.file", "n.id", &mut 0))
            });
            this.frame("j", &format!(
                "SELECT NULL, n.file, n.id{nulls}{} FROM \
                 (SELECT f.file AS file, MIN(f.node) AS lo, MAX(a.\"last\") AS hi \
                  FROM {input} f CROSS JOIN node a ON a.file = f.file AND a.id = f.node GROUP BY f.file) b \
                 CROSS JOIN node n ON n.file = b.file AND n.id > b.lo AND n.id <= b.hi{kinds}{shape}",
                this.zeros()), Reads::join(&[input]))
        });
        self.keyed_by(&domain, vec![BTreeSet::new()]);
        if !self.three
            && let Some(kinds) = &need.kinds
        {
            self.kinds_of.insert(domain.clone(), kinds.clone());
        }
        let matched = self.op(sub, &domain, &back);
        let eligible = self.name("j");
        let columns = self.within(&back, |this| this.physical_list().to_owned());
        let once = if self.looping > 0 {
            "IF NOT EXISTS "
        } else {
            ""
        };
        self.push(format!(
            "CREATE TEMP TABLE {once}{eligible}(file INTEGER, node INTEGER{columns}, PRIMARY KEY(file, node)) WITHOUT ROWID"
        ), Effect::default());
        if self.looping > 0 {
            self.push(
                format!("DELETE FROM {eligible}"),
                Reads::join(&[&eligible]).writes(Writes::Clear(eligible.clone())),
            );
        }
        let values = self.within(&back, |this| this.physical_of("m"));
        self.insert(
            &eligible,
            format!("INSERT INTO {eligible} SELECT m.file, m.node{values} FROM {matched} m"),
            Reads::join(&[&matched]),
        );
        let overrides = self.all_from(&back, "w");
        let selected = self.select("f", &overrides);
        Some(self.frame("x", &format!(
            "SELECT {selected} FROM {input} f CROSS JOIN node a ON a.file = f.file AND a.id = f.node \
             CROSS JOIN {eligible} w ON w.file = f.file AND w.node = \
             (SELECT e.node FROM {eligible} e WHERE e.file = f.file AND e.node > f.node \
              AND e.node <= a.\"last\" ORDER BY e.node LIMIT 1)"
        ), Reads::join(&[input, &eligible])))
    }
}
