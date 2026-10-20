//! Policy-scoped immutable leaf compilation, following NER-1562's singleflight cache.
//! Deliberately separate from mutable matcher graphs and statement-local SCC numbering.

use super::measured;
use crate::code_rule_dsl::language::{GrammarDialect, PatternSource, rule_pattern};
use ast_grep_core::{AstGrep, Pattern, matcher::Matcher, meta_var::MetaVarEnv};
use std::{
    borrow::Cow,
    collections::HashMap,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Clone, Hash, PartialEq, Eq)]
struct Key {
    dialect: GrammarDialect,
    context: String,
    selector: Option<String>,
}
type Compiled = Result<(Arc<Pattern>, GrammarDialect), String>;
type Cell = Arc<OnceLock<Compiled>>;
#[derive(Default)]
struct Artifacts {
    patterns: parking_lot::Mutex<HashMap<Key, Cell>>,
    builds: AtomicUsize,
}
impl Artifacts {
    fn pattern(&self, key: &Key) -> Compiled {
        let cell = self.patterns.lock().entry(key.clone()).or_default().clone();
        cell.get_or_init(|| {
            self.builds.fetch_add(1, Ordering::Relaxed);
            compile(key)
        })
        .clone()
    }
}
fn compile(key: &Key) -> Compiled {
    let source = match &key.selector {
        Some(selector) => PatternSource::Contextual {
            context: &key.context,
            selector,
        },
        None => PatternSource::Plain(&key.context),
    };
    rule_pattern(source, key.dialect)
        .map(|(pattern, dialect)| (Arc::new(pattern), dialect))
        .map_err(|error| error.to_string())
}

#[test]
fn artifacts_build_once_and_separate_dialects_selectors_and_errors() {
    let cache = Artifacts::default();
    let key = Key {
        dialect: GrammarDialect::Python,
        context: "f($X)".into(),
        selector: None,
    };
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                for _ in 0..100 {
                    cache.pattern(&key).unwrap();
                }
            });
        }
    });
    assert_eq!(cache.builds.load(Ordering::Relaxed), 1);
    let base = cache.pattern(&key).unwrap().0;
    for key in [
        Key {
            dialect: GrammarDialect::TypeScript,
            ..key.clone()
        },
        Key {
            selector: Some("call".into()),
            ..key.clone()
        },
        Key {
            selector: Some("identifier".into()),
            ..key.clone()
        },
    ] {
        assert!(!Arc::ptr_eq(&base, &cache.pattern(&key).unwrap().0));
    }
    let invalid = Key {
        selector: Some("not_a_grammar_kind".into()),
        ..key.clone()
    };
    assert_eq!(cache.pattern(&invalid).err(), compile(&invalid).err());
    assert!(cache.pattern(&invalid).is_err());
    assert_eq!(cache.builds.load(Ordering::Relaxed), 5);
}

#[test]
#[ignore = "representation measurement; run with --test-threads=1 --nocapture"]
fn measure_immutable_artifact_reuse() {
    use crate::code_rule_dsl::sql::tests::differential::tape;
    let keys: Vec<_> = (0..16)
        .map(|index| Key {
            dialect: GrammarDialect::Python,
            context: format!("f{index}($X)"),
            selector: None,
        })
        .collect();
    let root = AstGrep::doc(tape(
        GrammarDialect::Python,
        &(0..16)
            .map(|index| format!("f{index}(a)\n"))
            .collect::<String>(),
    ));
    let rounds = 200;
    let (fresh, fresh_ms) = measured(|| {
        (0..rounds)
            .flat_map(|_| keys.iter().map(|key| compile(key).unwrap()))
            .collect::<Vec<_>>()
    });
    let cache = Artifacts::default();
    let (cached, cached_ms) = measured(|| {
        (0..rounds)
            .flat_map(|_| keys.iter().map(|key| cache.pattern(key).unwrap()))
            .collect::<Vec<_>>()
    });
    // Compare match/capture outputs from every distinct artifact, not just pointer identity.
    for ((fresh, _), (cached, _)) in fresh.iter().zip(&cached).take(keys.len()) {
        for node in root.root().dfs() {
            let run = |pattern: &Pattern| {
                let mut env = Cow::Owned(MetaVarEnv::new());
                pattern
                    .match_node_with_env(node.clone(), &mut env)
                    .map(|matched| {
                        (
                            matched.node_id(),
                            env.get_match("X").map(|value| value.node_id()),
                        )
                    })
            };
            assert_eq!(run(fresh), run(cached));
        }
    }
    eprintln!(
        "SPIKE {}",
        serde_json::json!({"name":"immutable-artifacts","requests":keys.len()*rounds,
        "freshBuilds":keys.len()*rounds,"cachedBuilds":cache.builds.load(Ordering::Relaxed),
        "freshMs":fresh_ms,"cachedMs":cached_ms,"completeLeafParity":true})
    );
}
