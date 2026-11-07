//! Compile the same pinned plan objects in fixed permutations. No alternate executor or SQL runs.
use crate::code_rule_dsl::{json::hex_digest, model::CompiledCodeRule};
use serde_json::json;

pub(in crate::code_rule_dsl) fn candidate_frontier_audit(
    rule: &CompiledCodeRule,
) -> serde_json::Value {
    let mut languages = Vec::new();
    for (language, plans) in super::plans::plans(std::iter::once(rule)) {
        let forward: Vec<_> = (0..plans.len()).collect();
        let mut reverse = forward.clone();
        reverse.reverse();
        let mut rotated = forward.clone();
        rotated.rotate_left(plans.len() / 2);
        let interleaved = (0..plans.len())
            .step_by(2)
            .chain((1..plans.len()).step_by(2))
            .collect();
        let mut variants = Vec::new();
        for (name, permutation) in [
            ("forward", forward),
            ("reverse", reverse),
            ("rotated", rotated),
            ("interleaved", interleaved),
        ] {
            let refs: Vec<_> = permutation.iter().map(|&i| plans[i].as_ref()).collect();
            let statement = super::lower::batch_files(&refs, language, &[]);
            let stages: Vec<_> = statement
                .stages
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    json!([
                        s.name,
                        s.sql,
                        s.index,
                        s.temporary,
                        s.covering,
                        statement.ownership.slot(i),
                        statement.ownership.members(i)
                    ])
                })
                .collect();
            let parameters: Vec<_> = statement
                .parameters
                .iter()
                .map(|v| format!("{v:?}"))
                .collect();
            let mut admissions: Vec<_> = statement
                .admissions
                .iter()
                .map(|a| (permutation[a.consumer], a.generated, a.broadened))
                .collect();
            admissions.sort_unstable();
            variants.push(json!({"order":name,"plans":plans.len(),"stages":stages.len(),"physicalBuilds":statement.stages.iter().filter(|s|s.temporary).count(),"producerSha256":hex_digest(&serde_json::to_vec(&statement.graph).expect("producer graph")),"storageSha256":hex_digest(&serde_json::to_vec(&statement.ownership.setup).expect("storage setup")),"stageSha256":hex_digest(&serde_json::to_vec(&stages).expect("stage metadata")),"parametersSha256":hex_digest(&serde_json::to_vec(&parameters).expect("bound values")),"admissions":admissions,"resultQueries":statement.queries.len(),"peakLiveRelations":statement.schedule.peak_relations}));
        }
        let stable = variants.iter().all(|v| {
            [
                "stageSha256",
                "storageSha256",
                "producerSha256",
                "parametersSha256",
                "admissions",
                "resultQueries",
                "peakLiveRelations",
            ]
            .iter()
            .all(|k| v[k] == variants[0][k])
        });
        languages.push(json!({"language":language,"deterministic":stable,"variants":variants}));
    }
    json!({"format":"rerail.frontier-audit/v1","rule":rule.id,"policyDigest":rule.digest,"deterministic":languages.iter().all(|v|v["deterministic"]==true),"languages":languages,"sqlExecution":"none; plan compilation only","scope":"one compiled policy; identical plan objects, capabilities and tape input slot"})
}
