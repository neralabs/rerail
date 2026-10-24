//! Parameter frames import typed captures and commit native-compatible channels only on success.
use super::{Demand, Output, Plan, Slots, Sql, fields, value};
use std::collections::BTreeMap;

impl Sql {
    pub(super) fn call_frame(
        &mut self,
        body: &Plan,
        mapping: &[(usize, usize)],
        input: &Demand,
    ) -> Result<Output, String> {
        let typed = mapping.iter().any(|(inner, outer)| {
            self.generation
                .memo_layout
                .as_ref()
                .is_some_and(|layout| layout.span_capable(*inner) || layout.span_capable(*outer))
        });
        if !typed {
            return self.call_frame_nodes(body, mapping, input);
        }
        let mut private: Slots = mapping.iter().map(|(inner, _)| *inner).collect();
        private.extend(&self.generation.following);
        let columns = fields(&private, |inner| {
            let outer = mapping
                .iter()
                .find(|(id, _)| *id == inner)
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
                "k,file,node,work,private typed parameter captures",
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
        let returned = groups
            .values()
            .flatten()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|inner| format!(",o.c{inner} AS e{inner}"))
            .collect::<String>();
        let returns = mapping
            .iter()
            .filter(|(inner, _)| body.writes.contains(inner))
            .map(|(inner, outer)| {
                format!(
                    "relation_call_returned(d.file,{},o.c{inner})",
                    value(input, "d", *outer)
                )
            })
            .collect::<Vec<_>>();
        let returns = if returns.is_empty() {
            "0".into()
        } else {
            returns.join(" OR ")
        };
        let mut state=self.relation("parameter frame private result",format!("SELECT d.*,o.work AS returned_work,o.truth AS t0,CASE WHEN o.truth=1 THEN ({returns}) ELSE 0 END AS outputs_returned{returned} FROM {} d JOIN {} o ON o.k=d.k",input.name,answer.name),true,"caller context,remaining work,body verdict,private candidate captures");
        let mut truth = "t0".to_string();
        let mut step = 0;
        let mut exports = BTreeMap::new();
        for (outer, inners) in &groups {
            let mut merged = "NULL".to_string();
            for inner in inners {
                step += 1;
                let next_truth = format!("t{step}");
                let next_capture = format!("g{step}");
                state=self.relation("parameter candidate channel unification",format!("SELECT d.*,CASE WHEN d.{truth}<>1 OR d.outputs_returned=0 THEN d.{truth} WHEN relation_call_compatible(d.file,{merged},d.e{inner}) THEN 1 ELSE 0 END AS {next_truth},CASE WHEN d.{truth}=1 AND d.outputs_returned THEN relation_call_merge(d.file,{merged},d.e{inner},0) END AS {next_capture} FROM {state} d"),true,"caller context,unified candidates,transaction verdict");
                truth = next_truth;
                merged = format!("d.{next_capture}");
            }
            step += 1;
            let next_truth = format!("t{step}");
            let committed = format!("g{step}");
            let before = value(input, "d", *outer);
            state=self.relation("parameter caller channel unification",format!("SELECT d.*,CASE WHEN d.{truth}<>1 OR d.outputs_returned=0 THEN d.{truth} WHEN relation_call_compatible(d.file,{before},{merged}) THEN 1 ELSE 0 END AS {next_truth},CASE WHEN d.{truth}=1 AND d.outputs_returned THEN relation_call_merge(d.file,{before},{merged},1) END AS {committed} FROM {state} d"),true,"caller context,original-node precedence,agreed text and span channels");
            truth = next_truth;
            exports.insert(*outer, committed);
        }
        let columns = fields(&groups.keys().copied().collect(), |outer| {
            format!(
                "CASE WHEN d.{truth}=1 AND d.outputs_returned THEN d.{} ELSE {} END",
                exports[&outer],
                value(input, "d", outer)
            )
        });
        Ok(Output {
            name: self.relation(
                "parameter frame transactional commit",
                format!(
                    "SELECT d.k,d.returned_work AS work,d.{truth} AS truth{columns} FROM {state} d"
                ),
                true,
                "k,work,truth,committed caller captures; private captures discarded",
            ),
        })
    }
    fn call_frame_nodes(
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
