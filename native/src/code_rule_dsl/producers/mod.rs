//! Trusted producer definitions and full typed binding identities. This catalog does not execute work.
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) mod prefix;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Column {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub nullable: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Definition {
    pub inputs: BTreeMap<String, String>,
    pub options: BTreeMap<String, String>,
    pub columns: Vec<Column>,
    pub key: Vec<String>,
    pub completion: String,
    pub multiplicity: String,
    pub ordering: Vec<Order>,
    pub implementation: BTreeMap<String, String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Order {
    pub column: String,
    pub direction: String,
    pub nulls: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Contract {
    pub name: String,
    #[serde(flatten)]
    pub definition: Definition,
    pub origin: String,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
pub(crate) enum OptionValue {
    Integer(i64),
    Text(String),
}
impl OptionValue {
    fn kind(&self) -> &str {
        match self {
            Self::Integer(_) => "integer",
            Self::Text(_) => "text",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Binding<I> {
    pub contract: usize,
    pub inputs: BTreeMap<String, I>,
    pub options: BTreeMap<String, OptionValue>,
    pub domain: Option<String>,
}
#[derive(Default)]
pub(crate) struct Registry<I> {
    contracts: Vec<Contract>,
    names: BTreeMap<String, usize>,
    bindings: Vec<(Binding<I>, BTreeSet<String>)>,
    // Ordered maps compare complete definitions. No digest can establish sharing.
    identities: BTreeMap<Binding<I>, usize>,
}
#[cfg(any(test, feature = "code-rule-profile"))]
#[derive(Serialize)]
pub(crate) struct External {
    pub name: String,
    pub generation: String,
}
impl<I: Ord + Clone> Registry<I> {
    pub fn register(&mut self, contract: Contract) -> Result<usize, String> {
        if let Some(&id) = self.names.get(&contract.name) {
            let first = &self.contracts[id];
            if first.definition != contract.definition {
                let a = serde_json::to_value(&first.definition).expect("contract metadata");
                let b = serde_json::to_value(&contract.definition).expect("contract metadata");
                let mut fields = Vec::new();
                differences("", &a, &b, &mut fields);
                return Err(format!(
                    "producer contract clash {}: {} versus {}: {}",
                    contract.name,
                    first.origin,
                    contract.origin,
                    fields.join(", ")
                ));
            }
            return Ok(id);
        }
        let d = &contract.definition;
        let columns: BTreeSet<_> = d.columns.iter().map(|c| &c.name).collect();
        if d.columns.is_empty()
            || columns.len() != d.columns.len()
            || d.key.iter().any(|key| !columns.contains(key))
            || d.key.iter().collect::<BTreeSet<_>>().len() != d.key.len()
            || !matches!(d.completion.as_str(), "keys" | "domain")
            || (d.completion == "keys" && d.key.is_empty())
            || !matches!(d.multiplicity.as_str(), "set" | "bag")
            || d.ordering.iter().any(|o| {
                !columns.contains(&o.column)
                    || !matches!(o.direction.as_str(), "asc" | "desc")
                    || !matches!(o.nulls.as_str(), "first" | "last")
            })
        {
            return Err(format!(
                "invalid producer contract {} at {}",
                contract.name, contract.origin
            ));
        }
        let id = self.contracts.len();
        self.names.insert(contract.name.clone(), id);
        self.contracts.push(contract);
        Ok(id)
    }
    pub fn bind(&mut self, binding: Binding<I>, consumer: String) -> Result<usize, String> {
        let contract = self
            .contracts
            .get(binding.contract)
            .ok_or("unknown producer contract")?;
        let d = &contract.definition;
        if binding.inputs.keys().ne(d.inputs.keys())
            || binding.options.keys().ne(d.options.keys())
            || binding
                .options
                .iter()
                .any(|(name, value)| value.kind() != d.options[name])
            || (d.completion == "domain") != binding.domain.is_some()
            || binding.domain.as_ref().is_some_and(String::is_empty)
            || consumer.is_empty()
        {
            return Err(format!(
                "invalid binding for {} at {}",
                contract.name, contract.origin
            ));
        }
        if let Some(&id) = self.identities.get(&binding) {
            self.bindings[id].1.insert(consumer);
            return Ok(id);
        }
        let id = self.bindings.len();
        self.identities.insert(binding.clone(), id);
        self.bindings.push((binding, BTreeSet::from([consumer])));
        Ok(id)
    }
    #[cfg(any(test, feature = "code-rule-profile"))]
    pub fn manifest(&self, snapshot: &str, resolve: impl Fn(&I) -> External) -> serde_json::Value {
        let bindings: Vec<_> = self.bindings.iter().enumerate().map(|(id, (binding, consumers))| {
            let inputs: BTreeMap<_, _> = binding.inputs.iter().map(|(name, input)| (name, serde_json::json!({"external":resolve(input)}))).collect();
            let mut value = serde_json::json!({"id":format!("producer.{id}"),"contract":self.contracts[binding.contract].name,"inputs":inputs,"options":binding.options,"consumers":consumers,"origin":consumers.iter().cloned().collect::<Vec<_>>().join(", ")});
            if let Some(domain) = &binding.domain { value["domain"] = domain.clone().into(); }
            value
        }).collect();
        serde_json::json!({"format":"rerail.shared-work-plan/v1","snapshot":snapshot,"contracts":self.contracts,"bindings":bindings})
    }
}
fn differences(path: &str, a: &serde_json::Value, b: &serde_json::Value, fields: &mut Vec<String>) {
    if let (Some(a), Some(b)) = (a.as_object(), b.as_object()) {
        for key in a.keys().chain(b.keys()).collect::<BTreeSet<_>>() {
            differences(
                &format!("{path}.{key}"),
                &a.get(key).cloned().unwrap_or_default(),
                &b.get(key).cloned().unwrap_or_default(),
                fields,
            );
        }
    } else if let (Some(a), Some(b)) = (a.as_array(), b.as_array()) {
        if a.len() != b.len() {
            fields.push(format!("{path}: {a:?} versus {b:?}"));
        } else {
            for (i, (a, b)) in a.iter().zip(b).enumerate() {
                differences(&format!("{path}[{i}]"), a, b, fields);
            }
        }
    } else if a != b {
        fields.push(format!("{path}: {a} versus {b}"));
    }
}

#[cfg(test)]
mod tests;
