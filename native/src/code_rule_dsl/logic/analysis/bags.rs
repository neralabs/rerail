//! Summary storage compatibility across rules. Outer lookup identities remain local.
use super::scalars::{self, Expression, Kind, Scalar};
use std::collections::BTreeMap;
#[derive(Clone, Debug, serde::Serialize)]
pub struct Site {
    pub rule: usize,
    pub head: String,
    pub scalar: usize,
}
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Analysis {
    pub sites: Vec<Site>,
    pub bags: scalars::Analysis,
    pub partitions: Vec<Partition>,
}
#[derive(Clone, Debug, serde::Serialize)]
pub(in crate::code_rule_dsl) struct Partition {
    pub members: Vec<usize>,
    pub reason: &'static str,
}
pub fn analyze(
    inputs: impl IntoIterator<Item = (Site, Scalar)>,
) -> Analysis {
    let (sites, inputs): (Vec<_>, Vec<_>) = inputs.into_iter().unzip();
    let mut result = Analysis {
        sites,
        bags: scalars::Analysis {
            inputs,
            ..Default::default()
        },
        partitions: vec![],
    };
    let mut sources = BTreeMap::new();
    for (i, s) in result.bags.inputs.iter().enumerate() {
        sources
            .entry((s.source.clone(), s.keys.clone(), s.predicates.clone()))
            .or_insert_with(Vec::new)
            .push(i);
    }
    for members in sources.into_values() {
        let first_value = members
            .iter()
            .filter_map(|&i| {
                let s = &result.bags.inputs[i];
                (s.kind != Kind::Count && s.same_bag(s).proven).then(|| s.value.clone())
            })
            .next();
        let mut preferred = BTreeMap::new();
        for &i in &members {
            let input = &result.bags.inputs[i];
            if input.kind != Kind::Count && input.same_bag(input).proven {
                preferred
                    .entry((result.sites[i].rule, input.lookup.clone()))
                    .or_insert_with(|| input.value.clone());
            }
        }
        let mut values = BTreeMap::<Option<Expression>, Vec<usize>>::new();
        for i in members {
            let s = &result.bags.inputs[i];
            if !s.same_bag(s).proven {
                append(&mut result, vec![i]);
                continue;
            }
            let value = if s.kind == Kind::Count {
                preferred
                    .get(&(result.sites[i].rule, s.lookup.clone()))
                    .cloned()
                    .or_else(|| first_value.clone())
            } else {
                Some(s.value.clone())
            };
            values.entry(value).or_default().push(i);
        }
        for members in values.into_values() {
            append(&mut result, members);
        }
    }
    result.bags.families.sort_by_key(|f| f.members[0]);
    result
}
fn append(result: &mut Analysis, members: Vec<usize>) {
    let mut analysis = scalars::analyze_bags(
        members
            .iter()
            .map(|&i| result.bags.inputs[i].clone())
            .collect(),
    );
    for f in &mut analysis.families {
        f.witness = members[f.witness];
        for i in &mut f.members {
            *i = members[*i];
        }
    }
    for c in &mut analysis.checks {
        c.left = members[c.left];
        c.right = members[c.right];
    }
    result.bags.families.extend(analysis.families);
    result.bags.checks.extend(analysis.checks);
    result.partitions.push(Partition {members,reason:"Different sources, grouping keys, filters, values, or effect contracts cannot share storage; count prefers a compatible local lookup before adopting another value projection"});
}
