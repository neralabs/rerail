//! Atomic binding resolution for SQL-owned composition. This never evaluates a child rule.
use super::binding::Resolution;
use super::*;

pub(in crate::code_rule_dsl) enum BindingFacts {
    No,
    Unknown,
    Found {
        targets: [Option<u32>; 3],
        occurrences: Vec<Option<u32>>,
    },
}

pub(in crate::code_rule_dsl) fn binding_facts_budget<'tree, D: Doc>(
    test: &BindingTest,
    node: Node<'tree, D>,
    env: &MetaVarEnv<'tree, D>,
    work: usize,
) -> (Result<BindingFacts, UtilLimit>, usize) {
    struct Reset(bool, bool, usize, Option<UtilLimit>);
    impl Drop for Reset {
        fn drop(&mut self) {
            THREE_VALUED.set(self.0);
            UNKNOWN.set(self.1);
            UTIL_STEPS_LEFT.set(self.2);
            UTIL_EXCEEDED.set(self.3);
        }
    }
    let _reset = Reset(
        THREE_VALUED.replace(true),
        UNKNOWN.replace(false),
        UTIL_STEPS_LEFT.replace(work),
        UTIL_EXCEEDED.replace(None),
    );
    let facts = match test.resolve(&node, env) {
        Resolution::No => BindingFacts::No,
        Resolution::Unknown => BindingFacts::Unknown,
        Resolution::Found {
            anchor,
            site,
            value,
            annotation,
            occurrences,
        } => {
            if [
                (&test.site, site),
                (&test.value, value),
                (&test.annotation, annotation),
            ]
            .iter()
            .any(|(rule, target)| rule.is_some() && target.is_none())
            {
                return (Ok(BindingFacts::No), UTIL_STEPS_LEFT.get());
            }
            // Missing references stay in the ordered occurrence stream: native matching spends
            // its visit before find_near, whereas site/value/annotation spend only after it.
            let near = |target: crate::code_rule_dsl::value::NodeRef| {
                target
                    .find_near(anchor.root().root(), &anchor)
                    .map(|node| node.node_id() as u32)
            };
            let targets = [site, value, annotation].map(|target| target.and_then(near));
            BindingFacts::Found {
                targets,
                occurrences: occurrences.into_iter().map(near).collect(),
            }
        }
    };
    (
        match UTIL_EXCEEDED.get() {
            Some(limit) => Err(limit),
            None => Ok(facts),
        },
        UTIL_STEPS_LEFT.get(),
    )
}
