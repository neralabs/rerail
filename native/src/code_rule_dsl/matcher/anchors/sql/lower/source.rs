//! One typed source-confirmation frame per dependency wave; consumer scopes remain explicit.
use super::{Definitions, Stage};
use std::collections::{BTreeMap, HashMap};

pub(super) struct Demand {
    pub keys: String,
    pub expression: String,
    pub literal: bool,
    pub producer: usize,
    #[cfg(feature = "code-rule-profile")]
    pub consumers: std::collections::BTreeSet<usize>,
    #[cfg(feature = "code-rule-profile")]
    pub identity: String,
}

#[cfg(feature = "code-rule-profile")]
pub(super) fn register(
    demands: &HashMap<String, Demand>,
    registry: &mut crate::code_rule_dsl::producers::Registry<u32>,
) {
    use crate::code_rule_dsl::producers::{Binding, Column, Contract, Definition};
    let contract = registry
        .register(Contract {
            name: "syntax.source_confirmation.v1".into(),
            origin: concat!(file!(), ":register").into(),
            definition: Definition {
                inputs: BTreeMap::from([
                    ("tape".into(), "syntax-tape".into()),
                    ("requests".into(), "source-predicate-requests".into()),
                ]),
                options: BTreeMap::new(),
                columns: ["producer", "fid", "pre"]
                    .into_iter()
                    .map(|name| Column {
                        name: name.into(),
                        kind: "integer".into(),
                        nullable: false,
                    })
                    .collect(),
                key: vec!["producer".into(), "fid".into(), "pre".into()],
                completion: "domain".into(),
                multiplicity: "set".into(),
                ordering: vec![],
                implementation: BTreeMap::from([
                    ("backend".into(), "sqlite-native-table".into()),
                    ("symbol".into(), "nera_source_batch".into()),
                    ("revision".into(), "1".into()),
                ]),
            },
        })
        .expect("trusted source confirmation contract");
    for (name, demand) in demands {
        for consumer in &demand.consumers {
            registry
                .bind(
                    Binding {
                        contract,
                        inputs: BTreeMap::from([
                            ("tape".into(), 0),
                            (
                                "requests".into(),
                                u32::try_from(demand.producer).expect("producer ordinal"),
                            ),
                        ]),
                        options: BTreeMap::new(),
                        domain: Some(demand.identity.clone()),
                    },
                    format!("candidate-result.{consumer} selector {name}"),
                )
                .expect("trusted source confirmation binding");
        }
    }
}

pub(super) fn stages(
    demands: &HashMap<String, Demand>,
    levels: &HashMap<String, usize>,
    inline: &Definitions,
) -> Vec<(usize, Stage)> {
    let stage = |name: String, sql: String| Stage {
        name,
        sql,
        index: None,
        temporary: false,
        covering: false,
    };
    let mut out = vec![
        (
            0,
            stage(
                "source_reset_matches".into(),
                "DELETE FROM source_batch_match".into(),
            ),
        ),
        (
            0,
            stage(
                "source_reset_definitions".into(),
                "DELETE FROM source_batch_definition".into(),
            ),
        ),
    ];
    let mut definitions = demands.values().collect::<Vec<_>>();
    definitions.sort_by_key(|d| d.producer);
    for (i, chunk) in definitions.chunks(128).enumerate() {
        let values = chunk
            .iter()
            .map(|d| format!("({},{},{})", d.producer, d.expression, i32::from(d.literal)))
            .collect::<Vec<_>>()
            .join(",");
        out.push((
            0,
            stage(
                format!("source_definitions_{i}"),
                format!("INSERT INTO source_batch_definition VALUES {values}"),
            ),
        ));
    }
    let mut waves = BTreeMap::<usize, Vec<&Demand>>::new();
    for (name, demand) in demands {
        waves.entry(levels[name]).or_default().push(demand);
    }
    for (level, mut demands) in waves {
        demands.sort_by_key(|d| d.producer);
        let arms = demands
            .iter()
            .map(|d| format!("SELECT {} AS producer,fid,pre FROM {}", d.producer, d.keys))
            .collect();
        let input = inline.render(&format!(
            "INSERT INTO source_batch_demand(producer,fid,pre) {}",
            super::waves::compound("producer,fid,pre", arms)
        ));
        out.extend([
            (level,stage(format!("source_clear_wave_{level}"),"DELETE FROM source_batch_demand".into())),
            (level,stage(format!("source_input_wave_{level}"),input)),
            // This is frame-publication metadata, not execution-time posting pricing.
            (level,stage(format!("source_count_wave_{level}"),"UPDATE source_batch_stats SET n=(SELECT count(*) FROM source_batch_demand)".into())),
            (level,stage(format!("source_match_wave_{level}"),"INSERT OR IGNORE INTO source_batch_match SELECT m.producer,m.fid,m.pre FROM source_batch_stats stats CROSS JOIN (SELECT 1 FROM source_batch_demand LIMIT 1) demand_gate CROSS JOIN (SELECT 1 FROM source_batch_definition LIMIT 1) definition_gate CROSS JOIN nera_source_batch(1) m WHERE stats.n>0".into())),
        ]);
    }
    out
}
