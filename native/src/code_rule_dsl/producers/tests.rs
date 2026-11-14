use super::*;

fn contract() -> Contract {
    Contract {
        name: "syntax.example.v1".into(),
        origin: "engine/first.rs:7".into(),
        definition: Definition {
            inputs: BTreeMap::from([("tape".into(), "syntax-tape".into())]),
            options: BTreeMap::from([("field".into(), "integer".into())]),
            columns: vec![Column {
                name: "id".into(),
                kind: "integer".into(),
                nullable: false,
            }],
            key: vec!["id".into()],
            completion: "keys".into(),
            multiplicity: "set".into(),
            ordering: vec![],
            implementation: BTreeMap::from([
                ("symbol".into(), "engine::example".into()),
                ("revision".into(), "1".into()),
            ]),
        },
    }
}
fn binding(contract: usize, generation: &str, field: i64) -> Binding<String> {
    Binding {
        contract,
        inputs: BTreeMap::from([("tape".into(), generation.into())]),
        options: BTreeMap::from([("field".into(), OptionValue::Integer(field))]),
        domain: None,
    }
}

#[test]
fn complete_bindings_coalesce_different_consumers_but_separate_sources_options_and_names() {
    let mut registry = Registry::default();
    let id = registry.register(contract()).unwrap();
    let mut same = contract();
    same.origin = "engine/second.rs:19".into();
    assert_eq!(registry.register(same).unwrap(), id);
    let shared = registry
        .bind(
            binding(id, "generation-1", 4),
            "rule.a selector.a packet.0".into(),
        )
        .unwrap();
    assert_eq!(
        registry
            .bind(
                binding(id, "generation-1", 4),
                "rule.b selector.b packet.1".into()
            )
            .unwrap(),
        shared
    );
    assert_ne!(
        registry
            .bind(binding(id, "generation-2", 4), "rule.a".into())
            .unwrap(),
        shared
    );
    assert_ne!(
        registry
            .bind(binding(id, "generation-1", 5), "rule.a".into())
            .unwrap(),
        shared
    );
    let mut other = contract();
    other.name = "other.example.v1".into();
    let other = registry.register(other).unwrap();
    assert_ne!(
        registry
            .bind(binding(other, "generation-1", 4), "rule.a".into())
            .unwrap(),
        shared
    );
    let manifest = registry.manifest("snapshot.1", |generation| External {
        name: "tape".into(),
        generation: generation.clone(),
    });
    assert_eq!(
        manifest["bindings"][shared]["consumers"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        manifest["bindings"][shared]["options"]["field"],
        serde_json::json!({"kind":"integer","value":4})
    );
}

#[test]
fn name_clashes_report_both_origins_and_all_differing_definition_fields() {
    let mut registry: Registry<u32> = Registry::default();
    registry.register(contract()).unwrap();
    let mut other = contract();
    other.origin = "pack/other.sql:21".into();
    other.definition.columns[0].nullable = true;
    other
        .definition
        .implementation
        .insert("revision".into(), "2".into());
    let error = registry.register(other).unwrap_err();
    for expected in [
        "syntax.example.v1",
        "engine/first.rs:7",
        "pack/other.sql:21",
        "columns[0].nullable",
        "implementation.revision",
    ] {
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn malformed_bindings_fail_without_entering_the_registry() {
    let mut registry = Registry::default();
    let id = registry.register(contract()).unwrap();
    let good = binding(id, "generation-1", 3);
    let mut missing = good.clone();
    missing.inputs.clear();
    let mut unknown = good.clone();
    unknown
        .options
        .insert("unexpected".into(), OptionValue::Integer(3));
    let mut domain = good.clone();
    domain.domain = Some("sealed-membership".into());
    let mut invalid = good.clone();
    invalid.contract = 999;
    for bad in [missing, unknown, domain, invalid] {
        assert!(registry.bind(bad, "rule.a".into()).is_err());
    }
    assert!(registry.bind(good.clone(), String::new()).is_err());
    assert!(registry.bindings.is_empty());
    let mut d = contract();
    d.name = "syntax.domain.v1".into();
    d.definition.completion = "domain".into();
    let domain_id = registry.register(d).unwrap();
    let mut domain = binding(domain_id, "generation-1", 3);
    assert!(registry.bind(domain.clone(), "rule.a".into()).is_err());
    domain.domain = Some("file-a@generation-1".into());
    let first = registry.bind(domain.clone(), "rule.a".into()).unwrap();
    domain.domain = Some("file-b@generation-1".into());
    assert_ne!(first, registry.bind(domain, "rule.a".into()).unwrap());
}

#[test]
fn schema_key_multiplicity_and_ordering_changes_are_contract_clashes() {
    let mut registry: Registry<u32> = Registry::default();
    registry.register(contract()).unwrap();
    let mut schema = contract();
    schema.definition.columns[0].kind = "text".into();
    let mut key = contract();
    key.definition.key.clear();
    let mut bag = contract();
    bag.definition.multiplicity = "bag".into();
    let mut order = contract();
    order.definition.ordering.push(Order {
        column: "id".into(),
        direction: "asc".into(),
        nulls: "last".into(),
    });
    for changed in [schema, key, bag, order] {
        assert!(registry.register(changed).is_err());
    }
    assert_eq!(registry.contracts.len(), 1);
}
