//! Suppression kinds: what a linter's suppressions are, as its language crate declares them (`SuppressionKind`), the
//! registry the engine fills as it loads, and what a violation's `suppress` asks of them (`Suppress`).

use super::super::json::{invalid, object, reject_unknown};
use napi::Result;
use serde_json::Value;

/// Names a `rules` or `codes` list may hold.
pub(in crate::code_rule_dsl) const MAX_NAMES: usize = 256;

/// A linter's suppressions, as the language crate that reads them declares them. The engine registers each kind
/// as it loads (`install`), in the order `suppress` is parsed and checked in. Nothing here assumes comments:
/// Clippy's are attributes. The host's twin is `suppressionKinds` in
/// `src/engine/code-rule/code-rule-schema-semantic.ts`.
pub(crate) trait SuppressionKind: 'static {
    /// The `suppress` key, which is also the kind's name in the registry's `suppressions`
    /// (`languages/*.json`).
    const KEY: &'static str;
    /// Why only a family that honours it may ask for it: `key: detail`.
    const DETAIL: &'static str;
    /// What `suppress.<KEY>` parses to: what the family adapter's `Adapter::suppresses` reads
    /// (`Suppress::get`).
    type Spec: Clone + std::fmt::Debug + Eq + Send + Sync + 'static;
    /// Parses `suppress.<KEY>`, at `at`.
    fn spec(value: &Value, at: &str) -> Result<Self::Spec>;
}

/// A registered kind, as the core reads it.
pub(crate) struct Linter {
    pub(in crate::code_rule_dsl) key: &'static str,
    pub(in crate::code_rule_dsl) detail: &'static str,
    pub(in crate::code_rule_dsl) parse: fn(&Value, &str) -> Result<Spec>,
}

pub(in crate::code_rule_dsl) fn parse_spec<K: SuppressionKind>(value: &Value, at: &str) -> Result<Spec> {
    K::spec(value, at).map(|spec| Spec(std::sync::Arc::new(spec)))
}

/// A linter's suppressions a family may honour: a registered kind (`kind::<K>()`).
#[derive(Clone, Copy)]
pub(crate) struct Suppression(pub(in crate::code_rule_dsl) &'static Linter);

/// Kind `K`, as the core reads it.
pub(crate) fn kind<K: SuppressionKind>() -> Suppression {
    Suppression(const { &Linter { key: K::KEY, detail: K::DETAIL, parse: parse_spec::<K> } })
}

pub(in crate::code_rule_dsl) static KINDS: std::sync::OnceLock<Vec<Suppression>> = std::sync::OnceLock::new();

/// The engine's suppression kinds, in the order `suppress` is parsed and checked in; the first call holds.
pub(crate) fn install(kinds: Vec<Suppression>) {
    let _ = KINDS.set(kinds);
}

impl Suppression {
    /// Every registered kind, in order.
    pub(crate) fn all() -> &'static [Self] {
        KINDS.get().map_or(&[], Vec::as_slice)
    }

    /// Its `suppress` key, which is also its name in the registry's `suppressions`.
    pub(crate) fn key(self) -> &'static str {
        self.0.key
    }

    /// Why only a family that honours it may ask for it: `key: detail`.
    pub(crate) fn detail(self) -> &'static str {
        self.0.detail
    }
}

impl PartialEq for Suppression {
    fn eq(&self, other: &Self) -> bool {
        self.0.key == other.0.key
    }
}

impl Eq for Suppression {}

impl std::hash::Hash for Suppression {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.0.key.hash(state);
    }
}

impl std::fmt::Debug for Suppression {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.key)
    }
}

/// A parsed `suppress.<key>`: its kind's `Spec`, which only that kind's reader reads.
#[derive(Clone)]
pub(crate) struct Spec(pub(in crate::code_rule_dsl) std::sync::Arc<dyn SpecValue>);

pub(in crate::code_rule_dsl) trait SpecValue: std::any::Any + std::fmt::Debug + Send + Sync {
    fn same(&self, other: &dyn SpecValue) -> bool;
    fn any(&self) -> &dyn std::any::Any;
}

impl<T: std::any::Any + std::fmt::Debug + Eq + Send + Sync> SpecValue for T {
    fn same(&self, other: &dyn SpecValue) -> bool {
        other.any().downcast_ref::<T>() == Some(self)
    }
    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

impl PartialEq for Spec {
    fn eq(&self, other: &Self) -> bool {
        self.0.same(&*other.0)
    }
}

impl Eq for Spec {}

impl std::fmt::Debug for Spec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// `suppress` on a violation spec: per kind, what the violation asks for, in the kinds' order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::code_rule_dsl) struct Suppress {
    pub(in crate::code_rule_dsl) specs: Vec<(Suppression, Spec)>,
}

impl Suppress {
    /// The kinds the violation asks for, in the kinds' order.
    pub(in crate::code_rule_dsl) fn requested(&self) -> impl Iterator<Item = Suppression> + '_ {
        self.specs.iter().map(|(kind, _)| *kind)
    }

    /// What the violation asks of kind `K`, if anything.
    pub(in crate::code_rule_dsl) fn get<K: SuppressionKind>(&self) -> Option<&K::Spec> {
        self.specs.iter().find(|(kind, _)| kind.key() == K::KEY).and_then(|(_, spec)| spec.0.any().downcast_ref())
    }
}

pub(in crate::code_rule_dsl) fn parse(value: &Value, path: &str) -> Result<Suppress> {
    let suppress = object(value, path)?;
    let kinds = Suppression::all();
    reject_unknown(suppress, &kinds.iter().map(|kind| kind.key()).collect::<Vec<_>>(), path)?;
    // Kinds are parsed in their order.
    let mut specs = Vec::new();
    for &kind in kinds {
        if let Some(spec) = suppress.get(kind.key()) {
            specs.push((kind, (kind.0.parse)(spec, &format!("{path}/{}", kind.key()))?));
        }
    }
    if specs.is_empty() {
        return Err(invalid(format!("{path} must hold {}", one_of(kinds))));
    }
    Ok(Suppress { specs })
}

/// `a, b, or c`: the keys a `suppress` may hold.
pub(in crate::code_rule_dsl) fn one_of(kinds: &[Suppression]) -> String {
    let keys = kinds.iter().map(|kind| kind.key()).collect::<Vec<_>>();
    match keys.split_last() {
        Some((last, [])) => (*last).to_owned(),
        Some((last, rest)) => format!("{}, or {last}", rest.join(", ")),
        None => String::new(),
    }
}

/// A non-empty list of at most `MAX_NAMES` strings, each passing `valid`: `what` names the list's
/// items, `each` one item.
pub(in crate::code_rule_dsl) fn names(
    value: Option<&Value>,
    path: &str,
    valid: fn(&str) -> bool,
    what: &str,
    each: &str,
) -> Result<Vec<String>> {
    let items = value
        .and_then(Value::as_array)
        .filter(|items| !items.is_empty() && items.len() <= MAX_NAMES)
        .ok_or_else(|| invalid(format!("{path} must be a list of 1 to {MAX_NAMES} {what}")))?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            item.as_str()
                .filter(|name| valid(name))
                .map(str::to_owned)
                .ok_or_else(|| invalid(format!("{path}/{index} must be {each}")))
        })
        .collect()
}

