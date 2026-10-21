//! Explicit, allocation-free payload traversal, except for the set of shared row identities.
//! These are ownership estimates, not allocator statistics or RSS.
use crate::code_rule_dsl::violations::Witness;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::mem::size_of;

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct JsonMemory {
    pub values: usize,
    pub object_entries: usize,
    pub string_bytes: usize,
    pub key_bytes: usize,
    pub heap_bytes_estimate: usize,
}
impl JsonMemory {
    pub(crate) fn visit(&mut self, value: &Value) {
        self.values += 1;
        match value {
            Value::String(text) => {
                self.string_bytes += text.len();
                self.heap_bytes_estimate += text.capacity();
            }
            Value::Array(items) => {
                self.heap_bytes_estimate += items.capacity() * size_of::<Value>();
                for item in items {
                    self.visit(item);
                }
            }
            Value::Object(items) => {
                self.object_entries += items.len();
                // BTreeMap does not expose its node capacity. Include an explicit estimate
                // for slots and links, rather than confuse serialized bytes with heap bytes.
                self.heap_bytes_estimate += items.len()
                    * (size_of::<String>() + size_of::<Value>() + 3 * size_of::<usize>());
                for (key, value) in items {
                    self.key_bytes += key.len();
                    self.heap_bytes_estimate += key.capacity();
                    self.visit(value);
                }
            }
            _ => {}
        }
    }
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WitnessMemory {
    pub witnesses: usize,
    pub owned_bytes_estimate: usize,
    pub projection_references: usize,
    pub unique_projections: usize,
    pub projection: JsonMemory,
    #[serde(skip)]
    seen: HashSet<usize>,
}
impl WitnessMemory {
    pub(crate) fn visit(&mut self, witness: &Witness) {
        self.witnesses += 1;
        self.owned_bytes_estimate += size_of::<Witness>()
            + witness.path.capacity()
            + witness.captures.owned_heap_bytes()
            + witness.message.as_ref().map_or(0, String::capacity);
        if let Some(fix) = &witness.fix {
            let mut memory = JsonMemory::default();
            memory.visit(fix);
            self.owned_bytes_estimate += memory.heap_bytes_estimate;
        }
        if let Some(value) = witness.captures.projection() {
            self.projection_references += 1;
            if self.seen.insert(std::ptr::from_ref(value) as usize) {
                self.unique_projections += 1;
                self.owned_bytes_estimate += size_of::<Value>() + 2 * size_of::<usize>();
                self.projection.visit(value);
            }
        }
    }
}
