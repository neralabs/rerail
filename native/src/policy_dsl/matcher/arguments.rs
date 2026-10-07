use super::util::UNKNOWN;
use super::value::spend_step;
use crate::policy_dsl::identifier::python_identifier;
use crate::policy_dsl::language::{Family, GrammarDialect};
use crate::policy_dsl::value::{Context, Value};
use crate::policy_dsl::value::{php, python};
use ast_grep_core::meta_var::MetaVarEnv;
use ast_grep_core::{Doc, Node};
use std::borrow::Cow;
use std::collections::BTreeSet;

/// How a signature slot may be passed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SlotKind {
    /// Before `/`: by position only.
    PositionalOnly,
    /// By position or by keyword.
    Either,
    /// After `*`: by keyword only.
    KeywordOnly,
}

/// `arguments` (a Rerail extension): a call's arguments bound to a signature's named slots, as the
/// language binds them. Positional arguments fill the positional slots in order and keywords
/// (Python) their slot by name. A starred argument (`*a`, `...a`) makes every positional slot from
/// its position on unknown; `**kw` every keyword-passable slot not passed otherwise, unless it
/// spreads a dict display with string keys, whose pairs are keywords. `bind` captures the argument
/// in a slot and `absent` asserts a slot was not passed; where a slot is not known either way the
/// verdict is unknown.
#[derive(Clone)]
pub(crate) struct Arguments {
    pub(crate) language: GrammarDialect,
    pub(crate) slots: Vec<(String, SlotKind)>,
    /// Capture names and the slot each binds, by slot index.
    pub(crate) bind: Vec<(String, usize)>,
    /// Slots that must not be passed, by index.
    pub(crate) absent: Vec<usize>,
    /// Call kinds: Python `call`; TypeScript `call_expression` and `new_expression`; PHP function,
    /// method, nullsafe method and static calls and `new`.
    pub(crate) call_kinds: BTreeSet<u16>,
    /// Python and PHP: find each slot as Ruff's `find_argument` does, by the argument list's
    /// syntax alone (`mode: "syntactic"`), rather than as the call binds it.
    pub(crate) syntactic: bool,
    /// TypeScript: each slot is ESLint's `node.arguments[i]`, a spread element included
    /// (`mode: "estree"`), and a tagged template is no call.
    pub(crate) estree: bool,
}

/// What a call passes for a slot.
enum Passed<'r, D: Doc> {
    Absent,
    Value(Node<'r, D>),
    /// A starred argument or a spread keyword dict may pass it, or a TypeError may be raised.
    Unknown,
}

fn every<'r, D: Doc>(count: usize, make: impl Fn() -> Passed<'r, D>) -> Vec<Passed<'r, D>> {
    (0..count).map(|_| make()).collect()
}

fn named_children<'r, D: Doc>(node: &Node<'r, D>) -> Vec<Node<'r, D>> {
    node.children()
        .filter(|child| child.is_named() && !child.kind().ends_with("comment"))
        .collect()
}

impl Arguments {
    pub(super) fn match_node<'tree, D: Doc>(
        &self,
        node: Node<'tree, D>,
        env: &mut Cow<MetaVarEnv<'tree, D>>,
    ) -> Option<Node<'tree, D>> {
        if !self.call_kinds.contains(&node.kind_id()) {
            return None;
        }
        // ESTree's TaggedTemplateExpression has no `arguments`.
        if self.estree
            && node
                .field("arguments")
                .is_some_and(|arguments| arguments.kind() != "arguments")
        {
            return None;
        }
        let passed = match Family::of(self.language) {
            Family::Python if self.syntactic => self.syntactic(&node),
            Family::Python => self.python(&node),
            Family::Ecma => self.typescript(&node),
            Family::Php | Family::Blade if self.syntactic => self.php_syntactic(&node)?,
            Family::Php | Family::Blade => self.php(&node)?,
        };
        let mut unknown = false;
        for &slot in &self.absent {
            match passed[slot] {
                Passed::Absent => {}
                Passed::Value(_) => return None,
                Passed::Unknown => unknown = true,
            }
        }
        let mut trial = Cow::Borrowed(env.as_ref());
        for (name, slot) in &self.bind {
            match &passed[*slot] {
                Passed::Absent => return None,
                Passed::Unknown => unknown = true,
                Passed::Value(value) => {
                    trial.to_mut().insert(name, value.clone())?;
                }
            }
        }
        if unknown {
            UNKNOWN.set(true);
            return None;
        }
        if let Cow::Owned(bound) = trial {
            *env = Cow::Owned(bound);
        }
        Some(node)
    }

    /// Positional slots, in order.
    fn positional(&self) -> impl Iterator<Item = usize> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, (_, kind))| *kind != SlotKind::KeywordOnly)
            .map(|(index, _)| index)
    }

    fn keyword_slot(&self, name: &str) -> Option<usize> {
        self.slots
            .iter()
            .position(|(slot, kind)| slot == name && *kind != SlotKind::PositionalOnly)
    }

    fn python<'r, D: Doc>(&self, call: &Node<'r, D>) -> Vec<Passed<'r, D>> {
        let mut passed = every(self.slots.len(), || Passed::Absent);
        let Some(arguments) = call.field("arguments") else {
            return every(self.slots.len(), || Passed::Unknown);
        };
        // `f(x for x in y)` passes the generator as its one positional argument.
        if arguments.kind() == "generator_expression" {
            if let Some(first) = self.positional().next() {
                passed[first] = Passed::Value(arguments);
            }
            return passed;
        }
        let mut positions = self.positional();
        let mut starred = false;
        let mut keywords = Vec::new();
        let mut spread_keywords = false;
        for argument in named_children(&arguments) {
            match argument.kind().as_ref() {
                "keyword_argument" => {
                    if let (Some(name), Some(value)) =
                        (argument.field("name"), argument.field("value"))
                    {
                        keywords.push((python_identifier(&name).into_owned(), value));
                    }
                }
                "dictionary_splat" => match literal_keywords(&argument) {
                    Some(pairs) => keywords.extend(pairs),
                    None => spread_keywords = true,
                },
                "list_splat" | "parenthesized_list_splat" => starred = true,
                _ if starred => {}
                _ => {
                    if let Some(slot) = positions.next() {
                        passed[slot] = Passed::Value(argument);
                    }
                }
            }
        }
        if starred {
            for slot in positions {
                passed[slot] = Passed::Unknown;
            }
        }
        for (name, value) in keywords {
            if let Some(slot) = self.keyword_slot(&name) {
                // The same slot twice is a TypeError; the call binds nothing.
                passed[slot] = match passed[slot] {
                    Passed::Absent => Passed::Value(value),
                    Passed::Value(_) => Passed::Unknown,
                    Passed::Unknown => Passed::Value(value),
                };
            }
        }
        if spread_keywords {
            for (slot, (_, kind)) in self.slots.iter().enumerate() {
                if *kind != SlotKind::PositionalOnly && matches!(passed[slot], Passed::Absent) {
                    passed[slot] = Passed::Unknown;
                }
            }
        }
        passed
    }

    /// Ruff's `Arguments::find_argument`: a keyword of the slot's name (the first one), else the
    /// positional argument at the slot's position among the arguments before the first starred
    /// one. `**kw` and `**{...}` name no keyword. Positional-only slots take no keyword and
    /// keyword-only slots no position. Nothing is unknown.
    fn syntactic<'r, D: Doc>(&self, call: &Node<'r, D>) -> Vec<Passed<'r, D>> {
        let mut passed = every(self.slots.len(), || Passed::Absent);
        let Some(arguments) = call.field("arguments") else {
            return passed;
        };
        let mut positional = Vec::new();
        let mut keywords = Vec::new();
        if arguments.kind() == "generator_expression" {
            positional.push(arguments);
        } else {
            let mut starred = false;
            for argument in named_children(&arguments) {
                match argument.kind().as_ref() {
                    "keyword_argument" => {
                        if let (Some(name), Some(value)) =
                            (argument.field("name"), argument.field("value"))
                        {
                            keywords.push((python_identifier(&name).into_owned(), value));
                        }
                    }
                    "dictionary_splat" => {}
                    "list_splat" | "parenthesized_list_splat" => starred = true,
                    _ if starred => {}
                    _ => positional.push(argument),
                }
            }
        }
        for (position, slot) in self.positional().enumerate() {
            if let Some(argument) = positional.get(position) {
                passed[slot] = Passed::Value(argument.clone());
            }
        }
        for (slot, (name, kind)) in self.slots.iter().enumerate() {
            if *kind == SlotKind::PositionalOnly {
                continue;
            }
            if let Some((_, value)) = keywords.iter().find(|(keyword, _)| keyword == name) {
                passed[slot] = Passed::Value(value.clone());
            }
        }
        passed
    }

    /// PHP 8.5's call binding: positional arguments fill the slots in order; `name: value` the
    /// slot of that name (case-sensitive; a name outside the signature binds a parameter it does
    /// not list); `...[...]` unpacks an array literal, its integer keys by position and its string
    /// keys by name; `...$x` may pass any slot not otherwise passed. A call PHP refuses — a
    /// positional argument after a named one or after unpacking, unpacking after a named argument,
    /// a name passed twice (listed or not), a slot passed by position and by name, a by-reference
    /// argument — passes nothing known: every slot is unknown. `f(...)` is a first-class
    /// callable, not a call: `None`.
    fn php<'r, D: Doc>(&self, call: &Node<'r, D>) -> Option<Vec<Passed<'r, D>>> {
        let refused = || Some(every(self.slots.len(), || Passed::Unknown));
        let mut passed = every(self.slots.len(), || Passed::Absent);
        let Some(arguments) = php_arguments(call) else {
            return Some(passed);
        };
        // An argument list PHP cannot parse passes nothing known.
        if arguments
            .dfs()
            .any(|node| node.is_error() || node.is_missing())
        {
            return refused();
        }
        let mut positions = self.positional();
        let (mut named, mut unpacked, mut unpacked_named) = (false, false, false);
        // Every name passed so far, literal or unpacked, whether or not the signature lists it.
        let mut names = BTreeSet::new();
        // After `...$x`, no slot's position is known, and a slot `$x` may pass by name is
        // unknown whatever else passes it.
        let mut spread = false;
        for argument in named_children(&arguments) {
            match argument.kind().as_ref() {
                "variadic_placeholder" => return None,
                "argument" => {}
                _ => return refused(),
            }
            if argument.field("reference_modifier").is_some() {
                return refused();
            }
            let name = argument.field("name");
            let parts: Vec<_> = named_children(&argument)
                .into_iter()
                .filter(|part| {
                    name.as_ref()
                        .is_none_or(|name| part.range() != name.range())
                })
                .collect();
            let [value] = parts.as_slice() else {
                return refused();
            };
            if value.kind() == "variadic_unpacking" {
                if named {
                    return refused();
                }
                unpacked = true;
                let Some(inner) = named_children(value).into_iter().next() else {
                    return refused();
                };
                let mut spend = || spend_step();
                let mut context = Context::new(&mut spend);
                let Some(entries) = php::array_entries(&inner, &mut context) else {
                    spread = true;
                    continue;
                };
                for (key, element) in entries {
                    match key {
                        php::Key::Int(_) if unpacked_named => return refused(),
                        // A position after `...$x` is not known.
                        php::Key::Int(_) if spread => {}
                        php::Key::Int(_) => {
                            if let Some(slot) = positions.next() {
                                passed[slot] = Passed::Value(element);
                            }
                        }
                        php::Key::Str(name) => {
                            unpacked_named = true;
                            let slot = std::str::from_utf8(&name)
                                .ok()
                                .and_then(|name| self.keyword_slot(name));
                            if !pass_by_name(&mut passed, &mut names, &name, slot, element, spread)
                            {
                                return refused();
                            }
                        }
                    }
                }
            } else if let Some(name) = name {
                named = true;
                let name = name.text();
                let slot = self.keyword_slot(&name);
                if !pass_by_name(
                    &mut passed,
                    &mut names,
                    name.as_bytes(),
                    slot,
                    value.clone(),
                    spread,
                ) {
                    return refused();
                }
            } else {
                if named || unpacked {
                    return refused();
                }
                if let Some(slot) = positions.next() {
                    passed[slot] = Passed::Value(value.clone());
                }
            }
        }
        if spread {
            for item in &mut passed {
                if matches!(item, Passed::Absent) {
                    *item = Passed::Unknown;
                }
            }
        }
        Some(passed)
    }

    /// PHP's counterpart of Ruff's `find_argument`, over nikic/PHP-Parser's `args`: the first
    /// named argument of the slot's name (case-sensitive), else the positional argument at the
    /// slot's position among those before the first unpacking (`...`), parentheses unwrapped as
    /// PHP-Parser has none. Nothing is bound as PHP binds it: a call PHP refuses (a positional
    /// argument after a named one, a name passed twice) is read as written. An argument list PHP
    /// cannot parse (a syntax error, `&$x`) passes nothing known: every slot is unknown.
    /// `f(...)` is a first-class callable, not a call: `None`.
    fn php_syntactic<'r, D: Doc>(&self, call: &Node<'r, D>) -> Option<Vec<Passed<'r, D>>> {
        let mut passed = every(self.slots.len(), || Passed::Absent);
        let Some(arguments) = php_arguments(call) else {
            return Some(passed);
        };
        let refused = || Some(every(self.slots.len(), || Passed::Unknown));
        if arguments
            .dfs()
            .any(|node| node.is_error() || node.is_missing())
        {
            return refused();
        }
        let mut positional = Vec::new();
        let mut named = Vec::new();
        let mut unpacked = false;
        for argument in named_children(&arguments) {
            match argument.kind().as_ref() {
                "variadic_placeholder" => return None,
                "argument" => {}
                _ => return refused(),
            }
            if argument.field("reference_modifier").is_some() {
                return refused();
            }
            let name = argument.field("name");
            let parts: Vec<_> = named_children(&argument)
                .into_iter()
                .filter(|part| {
                    name.as_ref()
                        .is_none_or(|name| part.range() != name.range())
                })
                .collect();
            let [value] = parts.as_slice() else {
                return refused();
            };
            if value.kind() == "variadic_unpacking" {
                unpacked = true;
            } else if let Some(name) = name {
                named.push((name, unparenthesized(value.clone())));
            } else if !unpacked {
                positional.push(unparenthesized(value.clone()));
            }
        }
        for (position, slot) in self.positional().enumerate() {
            if let Some(argument) = positional.get(position) {
                passed[slot] = Passed::Value(argument.clone());
            }
        }
        for (slot, (slot_name, _)) in self.slots.iter().enumerate() {
            if let Some((_, value)) = named.iter().find(|(name, _)| name.text() == *slot_name) {
                passed[slot] = Passed::Value(value.clone());
            }
        }
        Some(passed)
    }

    fn typescript<'r, D: Doc>(&self, call: &Node<'r, D>) -> Vec<Passed<'r, D>> {
        let mut passed = every(self.slots.len(), || Passed::Absent);
        let Some(arguments) = call.field("arguments") else {
            return passed;
        };
        if arguments.kind() != "arguments" {
            return every(self.slots.len(), || Passed::Unknown);
        }
        let mut positions = self.positional();
        for argument in named_children(&arguments) {
            if argument.kind() == "spread_element" && !self.estree {
                for slot in positions.by_ref() {
                    passed[slot] = Passed::Unknown;
                }
                break;
            }
            if let Some(slot) = positions.next() {
                passed[slot] = Passed::Value(if self.estree {
                    unparenthesized(argument)
                } else {
                    argument
                });
            }
        }
        passed
    }
}

/// A PHP call's argument list: a function or method call's `arguments`, or the `arguments` of
/// `new C(...)` or of the anonymous class `new class(...) {...}` owns. `None` where `new` has
/// none (`new C`, `new class {}`).
fn php_arguments<'r, D: Doc>(call: &Node<'r, D>) -> Option<Node<'r, D>> {
    if let Some(arguments) = call.field("arguments") {
        return Some(arguments);
    }
    let owner = call
        .children()
        .find(|child| child.kind() == "anonymous_class")
        .unwrap_or_else(|| call.clone());
    owner.children().find(|child| child.kind() == "arguments")
}

/// Passes `value` by `name`, to `slot` where the signature lists the name; a name outside it
/// binds a parameter the signature does not list. `false` where PHP refuses the call: the name
/// is in `names` already, or the slot was passed by position. After `...$x`, which may pass the
/// slot too, it is unknown.
fn pass_by_name<'r, D: Doc>(
    passed: &mut [Passed<'r, D>],
    names: &mut BTreeSet<Vec<u8>>,
    name: &[u8],
    slot: Option<usize>,
    value: Node<'r, D>,
    spread: bool,
) -> bool {
    if !names.insert(name.to_vec()) {
        return false;
    }
    let Some(slot) = slot else {
        return true;
    };
    if !matches!(passed[slot], Passed::Absent) {
        return false;
    }
    passed[slot] = if spread {
        Passed::Unknown
    } else {
        Passed::Value(value)
    };
    true
}

/// The expression inside any parentheses: neither ESTree nor PHP-Parser has a parenthesized
/// expression, so `super((a))`'s `arguments[0]` is `a`, as `f((a))`'s PHP argument is.
fn unparenthesized<'r, D: Doc>(mut node: Node<'r, D>) -> Node<'r, D> {
    while node.kind() == "parenthesized_expression" {
        match named_children(&node).as_slice() {
            [inner] => node = inner.clone(),
            _ => break,
        }
    }
    node
}

/// The pairs of `**{'a': 1}`: a dict display whose keys are all string literals, without `**`
/// of its own.
fn literal_keywords<'r, D: Doc>(splat: &Node<'r, D>) -> Option<Vec<(String, Node<'r, D>)>> {
    let [dictionary] = named_children(splat).try_into().ok()?;
    if dictionary.kind() != "dictionary" {
        return None;
    }
    let mut pairs = Vec::new();
    for pair in named_children(&dictionary) {
        if pair.kind() != "pair" {
            return None;
        }
        let (key, value) = (pair.field("key")?, pair.field("value")?);
        if !matches!(key.kind().as_ref(), "string" | "concatenated_string") {
            return None;
        }
        let mut spend = spend_step;
        let mut global = |_: std::ops::Range<usize>, _: &str| None;
        let mut context = Context::with_globals(&mut spend, &mut global);
        let Value::Str(name) = python::decode(&key, &mut context) else {
            return None;
        };
        pairs.push((name, value));
    }
    Some(pairs)
}

#[cfg(test)]
mod tests {
    use super::literal_keywords;
    use crate::policy_dsl::language::GrammarDialect;
    use crate::policy_dsl::matcher::util::{UTIL_EXCEEDED, UTIL_STEPS_LEFT, UtilLimit};
    use ast_grep_core::tree_sitter::LanguageExt;

    /// Every case in the PHP oracle (`rerail-fixtures/php/arguments/generate_php.php`): a slot Rerail
    /// finds passed holds the argument PHP binds to it, a slot Rerail finds absent keeps PHP's
    /// default, and a call PHP refuses passes nothing known. Rerail does not know a slot only where
    /// the case's `unknown` names that slot, with why (a spread whose value it does not know).
    /// `callee: "new class"` passes the arguments to an anonymous class's constructor.
    #[test]
    fn php_calls_bind_as_php_binds_them() {
        use super::{Arguments, Passed, SlotKind};
        use crate::policy_dsl::oracle;
        use crate::policy_dsl::value::{Context, php};
        use serde_json::Value as Json;

        let oracle: Json =
            serde_json::from_str(oracle!("php/arguments/php.expected.json")).expect("oracle JSON");
        assert_eq!(oracle["php"].as_str().map(|v| &v[..3]), Some("8.5"));
        let mut failures = Vec::new();
        for case in oracle["cases"].as_array().expect("cases") {
            let call = case["call"].as_str().expect("call");
            let signature = case["signature"].as_array().expect("signature");
            let slots: Vec<_> = signature
                .iter()
                .map(|name| (name.as_str().expect("slot").to_owned(), SlotKind::Either))
                .collect();
            let arguments = Arguments {
                language: GrammarDialect::Php,
                slots,
                bind: Vec::new(),
                absent: Vec::new(),
                call_kinds: Default::default(),
                syntactic: false,
                estree: false,
            };
            let (source, kind) = match case.get("callee").and_then(Json::as_str) {
                None => (format!("f({call});"), "function_call_expression"),
                Some("new class") => (
                    format!("new class({call}) {{}};"),
                    "object_creation_expression",
                ),
                Some(other) => panic!("{call:?}: callee {other:?}"),
            };
            let root = GrammarDialect::Php.ast_grep(&source);
            let node = root
                .root()
                .dfs()
                .find(|node| node.kind() == kind)
                .expect("a call");
            let passed = arguments.php(&node).expect("a call, not a callable");
            if let Some(error) = case.get("error") {
                if !passed.iter().all(|slot| matches!(slot, Passed::Unknown)) {
                    failures.push(format!(
                        "{call:?}: PHP refuses it ({error}), Rerail binds slots"
                    ));
                }
                continue;
            }
            for (index, (slot, expected)) in passed
                .iter()
                .zip(case["slots"].as_array().expect("slots"))
                .enumerate()
            {
                let (agrees, rerail) = match slot {
                    Passed::Absent => (expected[0].as_str() == Some("absent"), "absent"),
                    Passed::Unknown => {
                        let name = signature[index].as_str().expect("slot");
                        (case["unknown"].get(name).is_some(), "unknown")
                    }
                    Passed::Value(node) => {
                        let mut spend = || true;
                        let mut context = Context::new(&mut spend);
                        let value = php::decode(node, &mut context);
                        (
                            expected[0].as_str() != Some("absent")
                                && php::tests::agrees(&value, expected),
                            "passed",
                        )
                    }
                };
                if !agrees {
                    failures.push(format!(
                        "{call:?}: slot {index}: PHP binds {expected}, Rerail finds it {rerail}"
                    ));
                }
            }
        }
        assert!(
            failures.is_empty(),
            "{} disagreements:\n{}",
            failures.len(),
            failures.join("\n")
        );
        // `f(...)` is a first-class callable, not a call.
        let root = GrammarDialect::Php.ast_grep("f(...);");
        let node = root
            .root()
            .dfs()
            .find(|node| node.kind() == "function_call_expression")
            .expect("a call");
        let arguments = Arguments {
            language: GrammarDialect::Php,
            slots: vec![("a".into(), SlotKind::Either)],
            bind: Vec::new(),
            absent: Vec::new(),
            call_kinds: Default::default(),
            syntactic: false,
            estree: false,
        };
        assert!(arguments.php(&node).is_none());
    }

    #[test]
    fn dictionary_splat_key_decoding_uses_the_value_budget() {
        let root = GrammarDialect::Python.ast_grep("f(**{'a': 1})");
        let splat = root
            .root()
            .dfs()
            .find(|node| node.kind() == "dictionary_splat")
            .expect("dictionary splat");
        let saved_steps = UTIL_STEPS_LEFT.replace(0);
        let saved_exceeded = UTIL_EXCEEDED.replace(None);
        assert!(literal_keywords(&splat).is_none());
        assert_eq!(UTIL_EXCEEDED.get(), Some(UtilLimit::Value));
        UTIL_STEPS_LEFT.set(saved_steps);
        UTIL_EXCEEDED.set(saved_exceeded);
    }
}

#[cfg(test)]
mod php_syntactic_tests;
