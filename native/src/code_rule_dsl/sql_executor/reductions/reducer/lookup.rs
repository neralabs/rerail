//! Share indexed summary probes while keeping range checks at their original uses.
use super::*;
impl Reducer {
    fn summary_field(&self, name: &str) -> String {
        format!(
            "__rr_summary_{}_{}",
            self.combined.as_ref().unwrap().members[0],
            name
        )
    }
    pub(in crate::code_rule_dsl::sql_executor::reductions) fn joined_lookup(
        &self,
        rule: &Rule,
    ) -> E {
        let mut expression = self.lookup(rule);
        if self.combined.as_ref().is_none_or(|s|!s.joined) || self.dynamic() {
            return expression;
        }
        expression.visit_mut(&mut |e| {
            if let E::Scalar(query) = e {
                let mut value = query.columns[0].0.clone();
                value.visit_mut(&mut |field| {
                    if let E::Qualified {
                        alias,
                        column: name,
                    } = field
                        && alias == "rr_reducer"
                    {
                        let projected = column("rr_context", &self.summary_field(name));
                        *field = if matches!(self.kind, Kind::Sum) {
                            // A missing left-join row represents an empty bag. Do not
                            // pass NULL to the exact integer range-check function.
                            E::Call {
                                name: "coalesce".into(),
                                arguments: vec![projected, integer(0)],
                            }
                        } else {
                            projected
                        };
                    }
                });
                *e = value;
            }
        });
        expression
    }
    pub(in crate::code_rule_dsl::sql_executor::reductions) fn append_join(
        &self,
        rule: &Rule,
        projections: &mut Vec<String>,
        from: &mut String,
    ) {
        let Some(shared) = &self.combined else {
            return;
        };
        if !shared.owner || !shared.joined || self.dynamic() {
            return;
        }
        let alias = format!("rr_summary_{}", shared.members[0]);
        for name in ["n", "value", "high", "minimum", "maximum"] {
            projections.push(format!("{alias}.{name} AS {}", self.summary_field(name)));
        }
        let keys = if self.outer_keys.is_empty() {
            vec![integer(1)]
        } else {
            self.context_keys(rule, "rr_input")
        };
        let on = keys
            .iter()
            .enumerate()
            .map(|(i, e)| format!("{alias}.c{i}={}", e.render()))
            .collect::<Vec<_>>()
            .join(" AND ");
        *from += &format!(" LEFT JOIN {} {alias} ON {on}", quote(&self.groups));
    }
    pub(in crate::code_rule_dsl::sql_executor::reductions) fn storage_explain(&self) -> Value {
        json!({"group":self.groups,"value_index":self.values,"context_consumers":self.pool_consumers,"cross_gate_projection_proven":self.transition_key.is_some(),"lookup_members":self.combined.as_ref().map(|s|&s.members),"joined_lookup":self.combined.as_ref().is_some_and(|s|s.joined && !self.dynamic()),"summary_protocol":if self.combined.is_some(){"Grouped SQL publication for one or more consumers"}else{"Specialized ordered or opaque reducer"}})
    }
    pub(in crate::code_rule_dsl::sql_executor::reductions) fn shared_lookup_owner(&self) -> bool {
        self.combined.as_ref().is_some_and(|s| s.owner && s.joined && !self.dynamic())
    }
}
