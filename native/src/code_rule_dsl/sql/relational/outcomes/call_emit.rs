//! A call frame imports only parameters and commits exports only after unification.
use super::{Demand, Output, Plan, Slots, Sql, fields, value};
use std::collections::BTreeMap;

impl Sql {
    pub(super) fn call_frame(
        &mut self,
        body: &Plan,
        mapping: &[(usize, usize)],
        input: &Demand,
    ) -> Result<Output, String> {
        let mut private: Slots = mapping.iter().map(|(inner, _)| *inner).collect();
        // Following metadata is a memo context, not a visible capture input.
        private.extend(&self.generation.following);
        let columns = fields(&private, |inner| {
            let outer = mapping
                .iter()
                .find(|(i, _)| *i == inner)
                .map_or(inner, |(_, outer)| *outer);
            value(input, "d", outer)
        });
        let frame = Demand {
            name: self.relation(
                "parameter frame imports",
                format!(
                    "SELECT d.k,d.file,d.node,d.work{columns} FROM {} d",
                    input.name
                ),
                true,
                "k,file,node,work,private parameter captures",
            ),
            slots: private,
        };
        let answer = self.emit(body, &frame)?;
        let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (inner, outer) in mapping {
            if body.writes.contains(inner) {
                groups.entry(*outer).or_default().push(*inner);
            }
        }
        let mut equalities = Vec::new();
        let exports: BTreeMap<_, _> = groups.iter().map(|(outer, inners)| {
            // Native unification chooses the lexicographically earliest source span.
            // Equal spans retain parameter order, even for nested equal-text nodes.
            let candidates = inners.iter().enumerate().map(|(ordinal, inner)| format!("SELECT o.c{inner} AS capture,{ordinal} AS ordinal")).collect::<Vec<_>>().join(" UNION ALL ");
            let first = format!("(SELECT candidates.capture FROM ({candidates}) candidates JOIN node chosen ON chosen.file=d.file AND chosen.id=candidates.capture WHERE candidates.capture IS NOT NULL ORDER BY chosen.start,chosen.\"end\",candidates.ordinal LIMIT 1)");
            for inner in inners {
                equalities.push(format!("(o.c{inner} IS NULL OR {first} IS NULL OR relation_capture_equal(d.file,o.c{inner},{first}))"));
            }
            (*outer, format!("COALESCE({},{first})", value(input, "d", *outer)))
        }).collect();
        let valid = if equalities.is_empty() {
            "1".into()
        } else {
            equalities.join(" AND ")
        };
        let tested = format!("CASE WHEN o.truth=1 AND NOT ({valid}) THEN 0 ELSE o.truth END");
        let columns = groups
            .keys()
            .map(|outer| {
                format!(
                    ",CASE WHEN ({tested})=1 THEN {} ELSE {} END AS c{outer}",
                    exports[outer],
                    value(input, "d", *outer)
                )
            })
            .collect::<String>();
        Ok(Output {
            name: self.relation(
                "parameter frame unification and commit",
                format!(
                    "SELECT d.k,o.work,{tested} AS truth{columns} FROM {} d JOIN {} o ON o.k=d.k",
                    input.name, answer.name
                ),
                true,
                "k,work,truth,caller exports; private captures discarded",
            ),
        })
    }
}
