//! Python's argument binding (moved from `arguments.rs` by the crate split).

use super::*;

/// Python's argument binding (`Arguments`), its crate's.
pub(crate) trait PythonArguments {
    fn python<'r, D: Doc>(&self, call: &Node<'r, D>) -> Vec<Passed<'r, D>>;
}

impl PythonArguments for Arguments {
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
        for argument in arguments.named_code().collect::<Vec<_>>() {
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
}

/// The pairs of `**{'a': 1}`: a dict display whose keys are all string literals, without `**`
/// of its own.
pub(super) fn literal_keywords<'r, D: Doc>(splat: &Node<'r, D>) -> Option<Vec<(String, Node<'r, D>)>> {
    let [dictionary] = splat.named_code().collect::<Vec<_>>().try_into().ok()?;
    if dictionary.kind() != "dictionary" {
        return None;
    }
    let mut pairs = Vec::new();
    for pair in dictionary.named_code().collect::<Vec<_>>() {
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
