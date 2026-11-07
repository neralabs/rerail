//! Reuse admitted fragments across different frontiers. Physical composition keeps exact scopes.
use super::{Admission, AnchorPlan, Definitions, decode, scope};
use crate::code_rule_dsl::matcher::d_anchors::sql::{cache::Cache, statistics};
use rusqlite::types::Value;
use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
};

const SQL_BUDGET: usize = 32 * 1024 * 1024;
type Key = (u32, usize, bool, Option<String>);
struct Cached {
    decisions: Vec<usize>,
    fragment: Arc<Fragment>,
}
pub(super) struct Fragment {
    pub live: Vec<String>,
    pub parameters: Vec<Value>,
    pub selected: String,
    pub indexed: BTreeSet<String>,
    pub preferred: BTreeSet<String>,
    pub decoding: HashMap<String, decode::Demand>,
    pub predicates: HashMap<String, (String, String)>,
    pub source_predicates: HashMap<String, (String, String, bool)>,
    pub admission: Admission,
    pub anchors: Vec<statistics::Decision>,
}
pub(in crate::code_rule_dsl::matcher::d_anchors::sql) struct Fragments {
    cache: Cache<Key, Cached>,
    hits: usize,
    compilations: usize,
}
impl Default for Fragments {
    fn default() -> Self {
        Self {
            cache: Cache::new(SQL_BUDGET),
            hits: 0,
            compilations: 0,
        }
    }
}
impl Fragments {
    pub(super) fn get(
        &mut self,
        plan: &AnchorPlan,
        lang: u32,
        fids: Option<&[i64]>,
        path_glob: Option<&str>,
        statistics: &statistics::Statistics,
    ) -> (Arc<Fragment>, bool) {
        let key = (
            lang,
            plan as *const AnchorPlan as usize,
            fids.is_some(),
            path_glob.map(str::to_owned),
        );
        let decisions = statistics.decisions(lang, &[plan]);
        if let Some(cached) = self.cache.get(&key)
            && cached.decisions == decisions
        {
            self.hits += 1;
            return (Arc::clone(&cached.fragment), true);
        }
        #[cfg(test)]
        super::FRAGMENT_COMPILES.set(super::FRAGMENT_COMPILES.get() + 1);
        self.compilations += 1;
        let mut lower = scope(lang, fids, path_glob, statistics);
        let checkpoint = lower.ctes.len();
        lower.limit = checkpoint + 256;
        let attempted = lower.plan(plan, lower.all());
        let anchors = std::mem::take(&mut lower.anchors);
        let admission = Admission {
            consumer: 0,
            generated: lower.ctes.len() - checkpoint,
            broadened: attempted.is_err() || lower.partial,
            partial: lower.partial && attempted.is_ok(),
        };
        let selected = match attempted {
            Ok(selected) => selected,
            Err(()) => {
                lower = scope(lang, fids, path_glob, statistics);
                lower.limit = lower.ctes.len() + 256;
                match &plan.root_kinds {
                    Some(kinds) => lower.kinds(kinds).unwrap_or_else(|_| lower.all()),
                    None => lower.all(),
                }
            }
        };
        let definitions = Definitions::new(std::mem::take(&mut lower.ctes));
        let live = definitions
            .live(&format!("SELECT fid,pre FROM {}", selected.name))
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        let fragment = Arc::new(Fragment {
            live,
            parameters: lower.parameters,
            selected: selected.name,
            indexed: lower.indexed,
            preferred: lower.preferred,
            decoding: lower.decoding,
            predicates: lower.predicates,
            source_predicates: lower.source_predicates,
            admission,
            anchors,
        });
        let bytes = fragment.live.iter().map(String::capacity).sum::<usize>()
            + fragment
                .parameters
                .iter()
                .map(|v| match v {
                    Value::Text(s) => s.capacity(),
                    Value::Blob(b) => b.capacity(),
                    _ => 0,
                })
                .sum::<usize>();
        self.cache.insert(
            key,
            Cached {
                decisions,
                fragment: Arc::clone(&fragment),
            },
            bytes,
        );
        (fragment, false)
    }
    pub fn memory_profile(&self) -> serde_json::Value {
        serde_json::json!({"entries":self.cache.len(),"retainedSqlBytes":self.cache.bytes(),"sqlBudgetBytes":SQL_BUDGET,"hits":self.hits,"compilations":self.compilations})
    }
}
