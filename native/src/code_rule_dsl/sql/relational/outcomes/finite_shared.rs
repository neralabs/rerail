//! Exact immutable source DAG definitions. Sharing code never shares an answer:
//! every parent edge still evaluates the definition with its current typed state.
use super::super::{Op, Plan};
use super::{Lower, Rc};
use crate::code_rule_dsl::matcher::{AstMatcher, BindingTest, Semantic, StopBy};
use ast_grep_core::{Matcher, matcher::PatternNode};
use std::sync::Arc;
fn pointer<T>(value: &Option<Arc<T>>) -> Option<usize> {
    value.as_ref().map(|v| Arc::as_ptr(v) as usize)
}
fn pattern(node: &PatternNode, out: &mut Vec<String>) {
    match node {
        PatternNode::MetaVar { meta_var } => out.push(format!("meta:{meta_var:?}")),
        PatternNode::Terminal {
            text,
            is_named,
            kind_id,
        } => out.push(format!("terminal:{kind_id}:{is_named}:{text:?}")),
        PatternNode::Internal { kind_id, children } => {
            out.push(format!("internal:{kind_id}:{}", children.len()));
            for child in children {
                pattern(child, out);
            }
        }
    }
}
/// FOLLOWING identities are unobservable only in utility-free, capture-free,
/// semantic-free child predicates. Do not merge a binding callback identity.
fn context_free(m: &AstMatcher) -> bool {
    match m {
        AstMatcher::Kind(_) | AstMatcher::Regex(_) | AstMatcher::Never => true,
        AstMatcher::Pattern(p) => crate::code_rule_dsl::sql::terminals::pattern_vars(p).is_empty(),
        AstMatcher::All(v) | AstMatcher::Any(v) => v.iter().all(context_free),
        AstMatcher::Not(m) => context_free(m),
        AstMatcher::Has(c, s)
        | AstMatcher::Inside(c, s)
        | AstMatcher::Follows(c, s)
        | AstMatcher::Precedes(c, s) => {
            context_free(c.rule())
                && match s {
                    StopBy::Rule(m) => context_free(m),
                    _ => true,
                }
        }
        AstMatcher::Field(f) => {
            context_free(&f.rule)
                && match &f.stop {
                    StopBy::Rule(m) => context_free(m),
                    _ => true,
                }
        }
        _ => false,
    }
}
fn binding(t: &BindingTest) -> Option<String> {
    if !t.sub_rules().all(context_free) {
        return None;
    }
    let f = &t.filter;
    let data = vec![
        format!(
            "{:?}",
            (
                t.language,
                t.lookup,
                t.destructured_value,
                t.write_last,
                &t.name_kinds,
                &t.reference_kinds,
                &t.paren_kinds,
                pointer(&t.options),
                pointer(&t.python)
            )
        ),
        format!(
            "{:?}",
            (
                &f.forms,
                &f.scopes,
                f.used,
                f.referenced,
                f.nested_use,
                f.exported,
                f.latest,
                f.outer
            )
        ),
        format!(
            "{:?}",
            (
                &f.bindings,
                f.builtin,
                f.directives,
                f.language,
                pointer(&f.options)
            )
        ),
    ];
    Some(format!(
        "binding:{data:?}:{:?}",
        [
            t.site.as_ref(),
            t.value.as_ref(),
            t.annotation.as_ref(),
            t.occurrence.as_ref()
        ]
        .into_iter()
        .map(|m| match m {
            None => Some(None),
            Some(m) => key(m).map(Some),
        })
        .collect::<Option<Vec<_>>>()?
    ))
}
fn append_stop(by: &StopBy, out: &mut Vec<String>) -> Option<()> {
    match by {
        StopBy::Neighbor => out.push("neighbor".into()),
        StopBy::End => out.push("end".into()),
        StopBy::Rule(m) => {
            out.push("stop-rule".into());
            append(m, out)?;
        }
    }
    Some(())
}
fn append(m: &AstMatcher, out: &mut Vec<String>) -> Option<()> {
    match m {
        AstMatcher::Kind(k) => out.push(format!(
            "kind:{:?}",
            k.potential_kinds()?.iter().collect::<Vec<_>>()
        )),
        AstMatcher::Regex(r) => out.push(format!("regex:{:?}", r.as_str())),
        AstMatcher::Util(u) => {
            out.push(format!("utility:{}:{}", u.table.as_ptr() as usize, u.index))
        }
        AstMatcher::Pattern(p)
            if matches!(p.strictness, ast_grep_core::MatchStrictness::Smart) && !p.has_error() =>
        {
            out.push(format!(
                "pattern-smart:{:?}",
                p.potential_kinds().map(|k| k.iter().collect::<Vec<_>>())
            ));
            pattern(&p.node, out);
        }
        AstMatcher::Never => out.push("never".into()),
        AstMatcher::All(v) | AstMatcher::Any(v) => {
            out.push(format!(
                "{}:{}",
                if matches!(m, AstMatcher::All(_)) {
                    "all"
                } else {
                    "any"
                },
                v.len()
            ));
            for child in v {
                append(child, out)?;
            }
        }
        AstMatcher::Not(m) => {
            out.push("not".into());
            append(m, out)?;
        }
        AstMatcher::Has(c, s)
        | AstMatcher::Inside(c, s)
        | AstMatcher::Follows(c, s)
        | AstMatcher::Precedes(c, s) => {
            out.push(
                match m {
                    AstMatcher::Has(..) => "has",
                    AstMatcher::Inside(..) => "inside",
                    AstMatcher::Follows(..) => "follows",
                    _ => "precedes",
                }
                .into(),
            );
            append_stop(s, out)?;
            append(c.rule(), out)?;
        }
        AstMatcher::Field(f) => {
            out.push(format!("field:{}:{}", f.inside, f.field));
            append_stop(&f.stop, out)?;
            append(&f.rule, out)?;
        }
        AstMatcher::NthChild(n) if n.of_rule.is_none() => out.push(format!(
            "nth:{}:{}:{}",
            n.position.step, n.position.offset, n.reverse
        )),
        AstMatcher::Semantic(Semantic::Binding(t)) => out.push(binding(t)?),
        _ => return None,
    }
    Some(())
}
/// Flat tagged tokens escape user strings once, so depth never multiplies key size.
fn key(m: &AstMatcher) -> Option<String> {
    let mut tokens = vec![];
    append(m, &mut tokens)?;
    Some(format!("{tokens:?}"))
}
impl Lower {
    pub(super) fn finite_signature(&self, matcher: &AstMatcher) -> Option<String> {
        if !self.finite_mode {
            return None;
        }
        Some(format!(
            "{:?}",
            (
                self.language,
                &self.namespace,
                self.parsed_depth,
                self.hosted_depth,
                self.scope_owner,
                key(matcher)?
            )
        ))
    }
    pub(super) fn finite_shared(&self, key: &Option<String>) -> Option<Plan> {
        let body = self.finite_shared.get(key.as_ref()?)?;
        Some(Plan {
            op: Op::Shared(Rc::clone(body)),
            effects: body.effects,
            mentions: body.mentions.clone(),
            writes: body.writes.clone(),
            node_writes: body.node_writes.clone(),
            walk_filter: body.walk_filter.clone(),
        })
    }
    pub(super) fn finite_define(&mut self, key: Option<String>, plan: Plan) -> Plan {
        let Some(key) = key else {
            return plan;
        };
        let body = Rc::new(plan);
        let result = Plan {
            op: Op::Shared(Rc::clone(&body)),
            effects: body.effects,
            mentions: body.mentions.clone(),
            writes: body.writes.clone(),
            node_writes: body.node_writes.clone(),
            walk_filter: body.walk_filter.clone(),
        };
        self.finite_shared.insert(key, body);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_source_key_size_is_linear_in_depth() {
        let mut sizes = vec![];
        let compiled = regex::Regex::new(r#"^\"quoted\.text$"#).unwrap();
        for depth in [8, 32, 128] {
            let mut rule = AstMatcher::Regex(compiled.clone());
            for _ in 0..depth {
                rule = AstMatcher::Not(Box::new(rule));
            }
            sizes.push(key(&rule).unwrap().len());
        }
        assert!(sizes[1] < sizes[0] * 4);
        assert!(sizes[2] < sizes[1] * 4);
        assert!(sizes[2] < 4096);
    }
}
