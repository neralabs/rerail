//! Forte's directive registry (`Parser\Directives\Directives`, `DiscoveredDirective`,
//! `ArgumentRequirement`, `StructureRole`) and the directive metadata it loads
//! (`resources/directives/*.json`), ported method by method. What reads the token stream for
//! directives is in `token_index` (`DirectiveTokenIndex`), `helper` (`DirectiveHelper`) and
//! `arguments` (`ArgumentScanner`).
//!
//! The registry is shared by value: the loaded table sits behind an `Arc`, so a parse's copy
//! (`Directives::withDefaults()` clones its template) costs no allocation until a parse registers
//! a directive. Names are bytes, lowercased as `StringInterner::lower` (`strtolower`, ASCII) does.

pub(crate) mod arguments;
pub(crate) mod helper;
pub(crate) mod token_index;

use super::node::StructureRole;
use indexmap::{IndexMap, IndexSet};
use serde_json::Value as Json;
use std::borrow::Cow;
use std::sync::{Arc, LazyLock, OnceLock};

/// Whether a directive takes arguments (`ArgumentRequirement`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArgumentRequirement {
    Required,
    NotAllowed,
    Optional,
}

/// A directive the registry knows (`DiscoveredDirective`). `name` keeps the metadata's case
/// (`endSwitch`); the registry's keys are lowercase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiscoveredDirective {
    pub(crate) name: Box<[u8]>,
    pub(crate) args: ArgumentRequirement,
    pub(crate) role: StructureRole,
    pub(crate) is_condition: bool,
    pub(crate) has_condition_like_branches: bool,
    pub(crate) terminators: Vec<Box<[u8]>>,
    pub(crate) condition_like_branches: Vec<Box<[u8]>>,
    pub(crate) terminator: Option<Box<[u8]>>,
    pub(crate) is_switch: bool,
    pub(crate) switch_parent: Option<Box<[u8]>>,
    pub(crate) is_switch_branch: bool,
    pub(crate) is_switch_terminator: bool,
    pub(crate) is_conditional_pair: bool,
    pub(crate) pairing_strategy: Option<Box<str>>,
    pub(crate) is_conditional_close: bool,
}

impl DiscoveredDirective {
    /// A directive with `DiscoveredDirective`'s defaults.
    fn named(name: &[u8], args: ArgumentRequirement, role: StructureRole) -> Self {
        Self {
            name: name.into(),
            args,
            role,
            is_condition: false,
            has_condition_like_branches: false,
            terminators: Vec::new(),
            condition_like_branches: Vec::new(),
            terminator: None,
            is_switch: false,
            switch_parent: None,
            is_switch_branch: false,
            is_switch_terminator: false,
            is_conditional_pair: false,
            pairing_strategy: None,
            is_conditional_close: false,
        }
    }
}

/// The registry's directives and the lists computed from them (Forte's caches, which
/// `invalidateDirectiveCaches` clears on every change).
#[derive(Clone, Debug, Default)]
pub(crate) struct Table {
    pub(crate) conditions: IndexSet<Box<[u8]>>,
    pub(crate) final_terminators: IndexSet<Box<[u8]>>,
    pub(crate) directives: IndexMap<Box<[u8]>, DiscoveredDirective>,
    condition_terminators: OnceLock<Vec<Box<[u8]>>>,
    condition_branches: OnceLock<Vec<Box<[u8]>>>,
}

impl Table {
    fn invalidate(&mut self) {
        self.condition_terminators = OnceLock::new();
        self.condition_branches = OnceLock::new();
    }
}

/// A directive registry (`Directives`).
#[derive(Clone, Debug)]
pub(crate) struct Directives {
    table: Arc<Table>,
    seen: IndexSet<Box<[u8]>>,
    advisory_pairs: IndexSet<Box<[u8]>>,
    advisory_conditions: IndexSet<Box<[u8]>>,
    accept_all: bool,
}

/// `StringInterner::lower`: ASCII lowercase.
pub(crate) fn lower(name: &[u8]) -> Box<[u8]> {
    name.to_ascii_lowercase().into_boxed_slice()
}

/// `lower` for a lookup: borrows a name that is lowercase already, as directive names mostly are.
fn lowered(name: &[u8]) -> Cow<'_, [u8]> {
    if name.iter().any(u8::is_ascii_uppercase) {
        Cow::Owned(name.to_ascii_lowercase())
    } else {
        Cow::Borrowed(name)
    }
}

/// Forte's directive metadata, in the order `DirectoryIterator` lists `resources/directives` in
/// the installed package (macOS, APFS), which `loadDirectory` loads them in: a name two files
/// define (`elseif`, `endif`) keeps its first position and its last definition.
const RESOURCES: [&str; 21] = [
    include_str!("resources/for.json"),
    include_str!("resources/authorizations.json"),
    include_str!("resources/switch.json"),
    include_str!("resources/foreach.json"),
    include_str!("resources/error.json"),
    include_str!("resources/translations.json"),
    include_str!("resources/verbatim.json"),
    include_str!("resources/sessions.json"),
    include_str!("resources/stacks.json"),
    include_str!("resources/forelse.json"),
    include_str!("resources/conditions.json"),
    include_str!("resources/includes.json"),
    include_str!("resources/env.json"),
    include_str!("resources/sections.json"),
    include_str!("resources/fragments.json"),
    include_str!("resources/guest.json"),
    include_str!("resources/components.json"),
    include_str!("resources/auth.json"),
    include_str!("resources/while.json"),
    include_str!("resources/layouts.json"),
    include_str!("resources/basic.json"),
];

impl Directives {
    /// An empty registry (`new Directives`).
    pub(crate) fn empty() -> Self {
        Self {
            table: Arc::new(Table::default()),
            seen: IndexSet::new(),
            advisory_pairs: IndexSet::new(),
            advisory_conditions: IndexSet::new(),
            accept_all: false,
        }
    }

    /// Forte's directives (`Directives::withDefaults()`).
    pub(crate) fn with_defaults() -> Self {
        static DEFAULTS: LazyLock<Directives> = LazyLock::new(|| {
            let mut directives = Directives::empty();
            for json in RESOURCES {
                directives.load_json(json);
            }
            directives
        });
        DEFAULTS.clone()
    }

    /// A registry from its parts (the oracle's recorded registries).
    #[cfg(test)]
    pub(crate) fn from_parts(
        table: Table,
        seen: IndexSet<Box<[u8]>>,
        advisory_pairs: IndexSet<Box<[u8]>>,
        advisory_conditions: IndexSet<Box<[u8]>>,
        accept_all: bool,
    ) -> Self {
        Self {
            table: Arc::new(table),
            seen,
            advisory_pairs,
            advisory_conditions,
            accept_all,
        }
    }

    /// The registry's directive table.
    #[cfg(test)]
    pub(crate) fn table(&self) -> &Table {
        &self.table
    }

    pub(crate) fn set_accept_all(&mut self, accept_all: bool) {
        self.accept_all = accept_all;
    }

    pub(crate) fn accepts_all_directives(&self) -> bool {
        self.accept_all
    }

    pub(crate) fn is_directive(&self, name: &[u8]) -> bool {
        self.accept_all || self.get_directive(name).is_some()
    }

    pub(crate) fn has_explicit_directive(&self, name: &[u8]) -> bool {
        self.table.directives.contains_key(&*lowered(name))
    }

    pub(crate) fn has_seen_directive(&self, name: &[u8]) -> bool {
        self.seen.contains(&*lowered(name))
    }

    pub(crate) fn has_advisory_pair(&self, name: &[u8]) -> bool {
        self.advisory_pairs.contains(&*lowered(name))
    }

    pub(crate) fn has_advisory_condition(&self, name: &[u8]) -> bool {
        self.advisory_conditions.contains(&*lowered(name))
    }

    /// `trainCompact`: learns which unknown directives pair (`@x`/`@endx`) or branch
    /// (`@x`/`@elsex`) from the names of the template's directive tokens.
    pub(crate) fn train(&mut self, names: impl Iterator<Item = Box<[u8]>>) {
        let mut names_lower = Vec::new();
        for name in names {
            let mut name = lower(&name);
            if name.first() == Some(&b'@') {
                name = name[1..].into();
            }
            if !name.is_empty() {
                names_lower.push(name);
            }
        }
        self.train_from_directive_names(names_lower);
    }

    /// `trainFromDirectiveNames`.
    fn train_from_directive_names(&mut self, names: Vec<Box<[u8]>>) {
        let mut set: IndexSet<Box<[u8]>> = IndexSet::new();
        self.seen.clear();
        self.advisory_pairs.clear();
        self.advisory_conditions.clear();
        for name in names {
            if name.is_empty() {
                continue;
            }
            let name = lower(&name);
            self.seen.insert(name.clone());
            set.insert(name);
        }
        for name in &set {
            if self.table.directives.contains_key(name) {
                continue;
            }
            let end = [b"end".as_slice(), name].concat();
            let else_ = [b"else".as_slice(), name].concat();
            if set.contains(end.as_slice()) {
                self.advisory_pairs.insert(name.clone());
            }
            if set.contains(else_.as_slice()) {
                self.advisory_conditions.insert(name.clone());
            }
        }
    }

    pub(crate) fn is_condition(&self, directive: &[u8]) -> bool {
        self.table.conditions.contains(&*lowered(directive))
    }

    /// `isPaired`: whether a directive opens a block a terminator closes.
    pub(crate) fn is_paired(&self, directive: &[u8]) -> bool {
        self.table
            .directives
            .get(&*lowered(directive))
            .is_some_and(|instance| {
                instance.role == StructureRole::Opening
                    && !instance.terminators.is_empty()
                    && instance.terminator.is_some()
            })
    }

    /// `getConditionTerminators`: the lowercase names of the closing condition directives.
    pub(crate) fn get_condition_terminators(&self) -> &[Box<[u8]>] {
        self.table.condition_terminators.get_or_init(|| {
            self.table
                .directives
                .values()
                .filter(|directive| {
                    directive.is_condition && directive.role == StructureRole::Closing
                })
                .map(|directive| lower(&directive.name))
                .collect()
        })
    }

    /// `getBranches`: every condition branch, for a condition directive.
    pub(crate) fn get_branches(&self, directive_name: &[u8]) -> &[Box<[u8]>] {
        match self.table.directives.get(&*lowered(directive_name)) {
            Some(directive) if directive.is_condition => self.get_all_condition_branches(),
            _ => &[],
        }
    }

    pub(crate) fn get_directive(&self, directive: &[u8]) -> Option<&DiscoveredDirective> {
        self.table.directives.get(&*lowered(directive))
    }

    /// `getAllConditionBranches`: `else` and every opening condition's terminators.
    fn get_all_condition_branches(&self) -> &[Box<[u8]>] {
        self.table.condition_branches.get_or_init(|| {
            let mut branches: Vec<Box<[u8]>> = vec![b"else".as_slice().into()];
            for directive in self.table.directives.values() {
                if !directive.is_condition || directive.role != StructureRole::Opening {
                    continue;
                }
                branches.extend(directive.terminators.iter().cloned());
            }
            branches
        })
    }

    pub(crate) fn is_switch(&self, name: &[u8]) -> bool {
        self.get_directive(name)
            .is_some_and(|directive| directive.is_switch)
    }

    /// `getSwitchBranches`: the names (as the metadata spells them) of the directives whose
    /// switch parent is `switch_name`.
    pub(crate) fn get_switch_branches(&self, switch_name: &[u8]) -> Vec<&[u8]> {
        let switch_name = lower(switch_name);
        if !self.table.directives.contains_key(&switch_name) {
            return Vec::new();
        }
        self.table
            .directives
            .values()
            .filter(|dir| dir.switch_parent.as_deref() == Some(&*switch_name))
            .map(|dir| &*dir.name)
            .collect()
    }

    pub(crate) fn is_switch_branch(&self, name: &[u8]) -> bool {
        self.get_directive(name)
            .is_some_and(|directive| directive.is_switch_branch)
    }

    pub(crate) fn is_switch_terminator(&self, name: &[u8]) -> bool {
        self.get_directive(name)
            .is_some_and(|directive| directive.is_switch_terminator)
    }

    pub(crate) fn is_conditional_pair(&self, name: &[u8]) -> bool {
        self.get_directive(name)
            .is_some_and(|directive| directive.is_conditional_pair)
    }

    pub(crate) fn get_pairing_strategy(&self, name: &[u8]) -> Option<&str> {
        self.get_directive(name)?.pairing_strategy.as_deref()
    }

    /// `addDirective`.
    fn add_directive(&mut self, directive: DiscoveredDirective) {
        let name = lower(&directive.name);
        let table = Arc::make_mut(&mut self.table);
        if directive.role == StructureRole::Closing && directive.terminators.is_empty() {
            table.final_terminators.insert(name.clone());
        }
        if directive.is_condition {
            table.conditions.insert(name.clone());
        }
        table.directives.insert(name, directive);
        table.invalidate();
    }

    /// `loadJson`: one metadata file.
    fn load_json(&mut self, contents: &str) {
        let Ok(Json::Array(data)) = serde_json::from_str::<Json>(contents) else {
            return;
        };
        for meta in data {
            let Json::Object(meta) = meta else {
                continue;
            };
            let args = argument_requirement(meta.get("args"));
            let names_raw = meta.get("name").and_then(Json::as_str).unwrap_or("");
            // `array_filter` drops "" and "0".
            let names: Vec<&str> = names_raw
                .split(',')
                .map(php_trim)
                .filter(|name| !name.is_empty() && *name != "0")
                .collect();

            let mut role = StructureRole::None;
            let mut is_condition = false;
            let mut terminators = Vec::new();
            let mut condition_like_branches = Vec::new();
            let mut terminator = None;
            let mut is_switch = false;
            let mut switch_parent: Option<Box<[u8]>> = None;
            let mut is_switch_branch = false;
            let mut is_switch_terminator = false;
            let mut is_conditional_pair = false;
            let mut pairing_strategy = None;
            let mut is_conditional_close = false;

            if let Some(Json::Object(structure)) = meta.get("structure") {
                let role_val = structure.get("role").and_then(Json::as_str).unwrap_or("");
                role = structure_role(role_val);
                is_condition = structure.get("condition").is_some_and(php_truthy);
                let string = |key: &str| structure.get(key).and_then(Json::as_str).unwrap_or("");
                terminators = parse_terminators(string("terminators"));
                condition_like_branches = parse_terminators(string("branches"));
                if !terminators.is_empty() {
                    terminator = Some(resolve_primary_terminator(&terminators));
                }
                if structure.get("type").and_then(Json::as_str) == Some("switch") {
                    is_switch = true;
                }
                if let Some(parent) = structure.get("parent").and_then(Json::as_str) {
                    switch_parent = Some(lower(parent.as_bytes()));
                }
                let parent_is_switch = switch_parent.as_deref() == Some(b"switch".as_slice());
                if role_val == "branch" && parent_is_switch {
                    is_switch_branch = true;
                }
                if role_val == "branch_terminator" && parent_is_switch {
                    is_switch_terminator = true;
                }
                if role_val == "conditional_pair" {
                    is_conditional_pair = true;
                    pairing_strategy = structure
                        .get("pairing_strategy")
                        .and_then(Json::as_str)
                        .map(Box::from);
                }
                if role_val == "conditional_close" {
                    is_conditional_close = true;
                }
            }

            for name in names {
                let mut directive = DiscoveredDirective::named(name.as_bytes(), args, role);
                directive.is_condition = is_condition;
                directive.has_condition_like_branches = !condition_like_branches.is_empty();
                directive.terminators = terminators.clone();
                directive.condition_like_branches = condition_like_branches.clone();
                directive.terminator = terminator.clone();
                directive.is_switch = is_switch;
                directive.switch_parent = switch_parent.clone();
                directive.is_switch_branch = is_switch_branch;
                directive.is_switch_terminator = is_switch_terminator;
                directive.is_conditional_pair = is_conditional_pair;
                directive.pairing_strategy = pairing_strategy.clone();
                directive.is_conditional_close = is_conditional_close;
                self.add_directive(directive);
            }
        }
    }
}

/// `getArgumentRequirement`.
fn argument_requirement(args: Option<&Json>) -> ArgumentRequirement {
    match args {
        None | Some(Json::Null) => ArgumentRequirement::Optional,
        Some(Json::Bool(true)) => ArgumentRequirement::Required,
        Some(Json::Bool(false)) => ArgumentRequirement::NotAllowed,
        Some(Json::Object(args)) => {
            let allowed = args.get("allowed").is_some_and(php_truthy);
            let required = args.get("required").is_some_and(php_truthy);
            if !allowed {
                ArgumentRequirement::NotAllowed
            } else if required {
                ArgumentRequirement::Required
            } else {
                ArgumentRequirement::Optional
            }
        }
        // PHP's `is_array` holds for a JSON list too, whose `allowed` is unset.
        Some(Json::Array(_)) => ArgumentRequirement::NotAllowed,
        Some(_) => ArgumentRequirement::Optional,
    }
}

/// `getStructureRole`.
fn structure_role(role: &str) -> StructureRole {
    match role {
        "open" | "conditional_pair" => StructureRole::Opening,
        "close" | "conditional_close" => StructureRole::Closing,
        "mixed" => StructureRole::Mixed,
        _ => StructureRole::None,
    }
}

/// `parseTerminators`: a comma-separated list, trimmed and lowercased, empties dropped.
fn parse_terminators(terminators: &str) -> Vec<Box<[u8]>> {
    terminators
        .split(',')
        .map(|terminator| lower(php_trim(terminator).as_bytes()))
        .filter(|terminator| !terminator.is_empty())
        .collect()
}

/// `resolvePrimaryTerminator`: the first `end…` terminator, else the last.
fn resolve_primary_terminator(terminators: &[Box<[u8]>]) -> Box<[u8]> {
    terminators
        .iter()
        .find(|terminator| terminator.starts_with(b"end"))
        .or(terminators.last())
        .cloned()
        .unwrap_or_default()
}

/// PHP's `trim` with its default characters (" \t\n\r\0\x0B").
fn php_trim(text: &str) -> &str {
    text.trim_matches([' ', '\t', '\n', '\r', '\0', '\x0B'])
}

/// PHP's `(bool)` of a decoded JSON value.
fn php_truthy(value: &Json) -> bool {
    match value {
        Json::Null => false,
        Json::Bool(value) => *value,
        Json::Number(number) => number.as_f64().is_some_and(|number| number != 0.0),
        Json::String(text) => !text.is_empty() && text != "0",
        Json::Array(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
    }
}
