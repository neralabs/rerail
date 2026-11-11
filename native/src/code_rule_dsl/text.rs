//! The text layer (decision 0042): every text test, however a rule spells it, as one canonical plan
//! run by one of two engines.
//!
//! A rule tests text as a regex, a glob, a prefix, suffix or substring, an exact string or a set of
//! them. Badly written tests are common (`(?s).*`, a regex on every node's text), and the engine
//! must not be slower for them: a regex that is a set or a glob in disguise runs as that set or
//! glob, under the same key, as one shared job.
//!
//! ```text
//!  Source ──parse──▶ Pattern ──rewrites──▶ Pattern ──productions──▶ Plan ──▶ vocabulary engine
//!  (spelling)        (\A? core \z?)        (canonical)                (key)   (once per distinct
//!                                                                               string)
//!                                                                       └──▶ scan engine
//!                                                                             (once per file, placed
//!                                                                              on nodes by span)
//! ```
//!
//! The extension points:
//! - **Rewrites** (`canon::REWRITES`): sound, domain-aware simplifications of a pattern
//!   (`(?s).*` at an edge drops that edge's anchor). Each is a `canon::Rewrite`, applied to a
//!   fixpoint in list order.
//! - **Productions** (`productions::PRODUCTIONS`): each a `productions::Production` turning a
//!   canonical pattern into a plan, or declining. They are tried in order, cheapest plan first;
//!   the first that accepts wins, and the last (`Automaton`) always accepts a regular pattern.
//! - **Plan families** (`plan::Plan`): what the engines know how to run. A new family is a variant
//!   with its case in `vocabulary::evaluate`, `scan::how` and `Compiled`.
//!
//! A lowering records which rewrites applied and which production fired (`Lowered`), for
//! diagnostics and counters (`text.production.<name>`).

pub mod canon;
pub mod plan;
pub mod productions;
pub mod scan;
pub mod source;
pub mod vocabulary;
#[cfg(test)]
pub mod tests;

pub use canon::{Domain, Pattern};
#[allow(unused_imports)]
pub use plan::{Anchor, Compiled, Key, Plan};
#[allow(unused_imports)]
pub use source::{Source, Spelling};

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

/// Installs the text layer's SQL functions on a store connection: `str_test(op, s, pattern)`, the
/// rules' text tests (`logic::strings`), so every statement a store runs (a wave's included) has them.
pub fn install(db: &rusqlite::Connection) -> rusqlite::Result<()> {
    crate::code_rule_dsl::logic::strings::register(db)
}

/// Why a test could not be lowered: its spelling does not parse.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(pub(in crate::code_rule_dsl) String);

/// A test lowered: its plan and key, and how it got there.
pub struct Lowered {
    pub(in crate::code_rule_dsl) plan: Plan,
    pub key: Key,
    /// The production that made the plan (`Production::name`).
    pub(in crate::code_rule_dsl) production: &'static str,
    /// The rewrites that changed the pattern, in the order they applied.
    pub(in crate::code_rule_dsl) rewrites: Vec<&'static str>,
    compiled: std::sync::OnceLock<Arc<Compiled>>,
    prepared: std::sync::OnceLock<Arc<scan::Prepared>>,
}

impl std::fmt::Debug for Lowered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lowered").field("plan", &self.plan).field("key", &self.key).field("production", &self.production).field("rewrites", &self.rewrites).finish()
    }
}

impl Lowered {
    /// The plan as a test of one string (a vocabulary term, a node's text, a Datalog value).
    pub fn compiled(&self) -> Arc<Compiled> {
        Arc::clone(self.compiled.get_or_init(|| Arc::new(Compiled::of(&self.plan))))
    }

    /// A plan made by the engines themselves (a part of another plan), not from a source.
    pub(in crate::code_rule_dsl) fn of_plan(plan: Plan, production: &'static str) -> Arc<Self> {
        Arc::new(Lowered { key: plan.key(), plan, production, rewrites: Vec::new(), compiled: Default::default(), prepared: Default::default() })
    }

    /// The plan's file-scan machinery (`scan`), built once.
    pub(in crate::code_rule_dsl) fn prepared(&self) -> Arc<scan::Prepared> {
        Arc::clone(self.prepared.get_or_init(|| Arc::new(scan::Prepared::of(&self.plan))))
    }
}

/// `source` over text of `domain`, lowered once per process and shared.
pub fn lower(source: &Source, domain: Domain) -> Result<Arc<Lowered>, Error> {
    static LOWERED: LazyLock<Mutex<HashMap<(Source, Domain), Result<Arc<Lowered>, Error>>>> = LazyLock::new(Default::default);
    if let Some(found) = LOWERED.lock().expect("text lowerings").get(&(source.clone(), domain)) {
        return found.clone();
    }
    let lowered = lower_uncached(source, domain).map(Arc::new);
    if let Ok(lowered) = &lowered {
        crate::code_rule_dsl::prof::count(counter(lowered.production), 1);
    }
    LOWERED.lock().expect("text lowerings").insert((source.clone(), domain), lowered.clone());
    lowered
}

/// The counter of a production's lowerings (`text.production.<name>`), made once per name.
fn counter(production: &'static str) -> &'static str {
    static NAMES: LazyLock<Mutex<HashMap<&'static str, &'static str>>> = LazyLock::new(Default::default);
    NAMES.lock().expect("counter names").entry(production).or_insert_with(|| Box::leak(format!("text.production.{production}").into_boxed_str()))
}

fn lower_uncached(source: &Source, domain: Domain) -> Result<Lowered, Error> {
    let lowered = |plan: Plan, production, rewrites| Lowered { key: plan.key(), plan, production, rewrites, compiled: Default::default(), prepared: Default::default() };
    match source.parse()? {
        source::Parsed::Regular(hir) => {
            let (pattern, rewrites) = canon::normalize(Pattern::of(hir), domain);
            let (plan, production) = productions::lower(&pattern, domain);
            Ok(lowered(plan, production, rewrites))
        }
        // Not a regular language (backreferences): no rewrite or production applies, and the plan
        // is the pattern itself, still run once per distinct string.
        source::Parsed::Backtracking(pattern) => Ok(lowered(Plan::Backtracking { pattern }, "backtracking", Vec::new())),
    }
}
