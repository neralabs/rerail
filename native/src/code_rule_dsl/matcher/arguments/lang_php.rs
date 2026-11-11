//! PHP's argument binding (moved from `arguments.rs` by the crate split).

use super::*;

/// PHP's argument binding (`Arguments`), its crate's.
pub(crate) trait PhpArguments {
    /// PHP 8.5's call binding: positional arguments fill the slots in order; `name: value` the
    /// slot of that name (case-sensitive; a name outside the signature binds a parameter it does
    /// not list); `...[...]` unpacks an array literal, its integer keys by position and its string
    /// keys by name; `...$x` may pass any slot not otherwise passed. A call PHP refuses — a
    /// positional argument after a named one or after unpacking, unpacking after a named argument,
    /// a name passed twice (listed or not), a slot passed by position and by name, a by-reference
    /// argument — passes nothing known: every slot is unknown. `f(...)` is a first-class
    /// callable, not a call: `None`.
    fn php<'r, D: Doc>(&self, call: &Node<'r, D>) -> Option<Vec<Passed<'r, D>>>;
    /// PHP's counterpart of Ruff's `find_argument`, over nikic/PHP-Parser's `args`: the first
    /// named argument of the slot's name (case-sensitive), else the positional argument at the
    /// slot's position among those before the first unpacking (`...`), parentheses unwrapped as
    /// PHP-Parser has none. Nothing is bound as PHP binds it: a call PHP refuses (a positional
    /// argument after a named one, a name passed twice) is read as written. An argument list PHP
    /// cannot parse (a syntax error, `&$x`) passes nothing known: every slot is unknown.
    /// `f(...)` is a first-class callable, not a call: `None`.
    fn php_syntactic<'r, D: Doc>(
        &self,
        call: &Node<'r, D>,
    ) -> Option<Vec<Passed<'r, D>>>;
}

impl PhpArguments for Arguments {
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
        for argument in arguments.named_code().collect::<Vec<_>>() {
            match argument.kind().as_ref() {
                "variadic_placeholder" => return None,
                "argument" => {}
                _ => return refused(),
            }
            if argument.field("reference_modifier").is_some() {
                return refused();
            }
            let name = argument.field("name");
            let parts: Vec<_> = argument.named_code().collect::<Vec<_>>()
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
                let Some(inner) = value.named_code().collect::<Vec<_>>().into_iter().next() else {
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
    fn php_syntactic<'r, D: Doc>(
        &self,
        call: &Node<'r, D>,
    ) -> Option<Vec<Passed<'r, D>>> {
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
        for argument in arguments.named_code().collect::<Vec<_>>() {
            match argument.kind().as_ref() {
                "variadic_placeholder" => return None,
                "argument" => {}
                _ => return refused(),
            }
            if argument.field("reference_modifier").is_some() {
                return refused();
            }
            let name = argument.field("name");
            let parts: Vec<_> = argument.named_code().collect::<Vec<_>>()
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
}

/// A PHP call's argument list: a function or method call's `arguments`, or the `arguments` of
/// `new C(...)` or of the anonymous class `new class(...) {...}` owns. `None` where `new` has
/// none (`new C`, `new class {}`).
pub(super) fn php_arguments<'r, D: Doc>(call: &Node<'r, D>) -> Option<Node<'r, D>> {
    if let Some(arguments) = call.field("arguments") {
        return Some(arguments);
    }
    let owner = call
        .children()
        .find(|child| child.kind() == "anonymous_class")
        .unwrap_or_else(|| call.clone());
    owner.children().find(|child| child.kind() == "arguments")
}
