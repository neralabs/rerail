//! SQL-owned, demand-driven property sharing. Each stage gathers missing keys in one query.
use super::{Lower, Set};

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub(super) enum Property {
    Node,
    Span,
    Child(u16),
}
#[derive(Clone)]
pub(super) struct Demand {
    pub(super) scope: String,
    pub(super) property: Property,
    pub(super) consumers: std::collections::BTreeSet<usize>,
}
impl Property {
    fn shape(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Node => ("props", "decoded_properties", "parent,end,kind,field,flags"),
            Self::Span => ("spans", "decoded_spans", "start,len"),
            Self::Child(_) => ("edges", "decoded_children", "field,child"),
        }
    }
    fn guard(self) -> String {
        match self {
            Self::Child(field) => format!(" AND d.field={field}"),
            _ => String::new(),
        }
    }
}
impl Lower {
    pub(super) fn decoded(&mut self, scope: &Set, property: Property) -> String {
        let binding = *self.producer_bindings.entry(property).or_insert_with(|| {
            property.binding(
                &mut self.producers,
                self.lang,
                format!("candidate-result.{}", self.consumer),
            )
        });
        let key = (scope.name.clone(), binding);
        if let Some(name) = self.reuse.decoded.get(&key) {
            self.decoding
                .get_mut(name)
                .expect("decoded selector")
                .consumers
                .insert(self.consumer);
            return name.clone();
        }
        // The physical planner shares the demand source or hoists a covering universal demand.
        let (category, table, columns) = property.shape();
        let name = format!("{category}{}", self.ctes.len());
        let qualified = columns
            .split(',')
            .map(|c| format!("d.{c}"))
            .collect::<Vec<_>>()
            .join(",");
        let guard = property.guard();
        let body = if property == Property::Node && scope.name == "all_nodes" {
            format!("SELECT d.fid,d.pre,{qualified} FROM cf CROSS JOIN {table} d ON d.fid=cf.fid")
        } else {
            format!(
                "SELECT s.fid,s.pre,{qualified} FROM {} s CROSS JOIN {table} d ON d.fid=s.fid AND d.pre=s.pre{guard}",
                scope.name
            )
        };
        // Text predicates consume these spans after the demanded-key join completes.
        // Flattening this boundary lets SQLite move costly source predicates into a
        // retained property scan before other selective consumers have restricted it.
        let boundary = if property == Property::Span {
            "MATERIALIZED"
        } else {
            "NOT MATERIALIZED"
        };
        self.ctes
            .push(format!("{name}(fid,pre,{columns}) AS {boundary} ({body})"));
        self.decoding.insert(
            name.clone(),
            Demand {
                scope: scope.name.clone(),
                property,
                consumers: std::collections::BTreeSet::from([self.consumer]),
            },
        );
        self.reuse.decoded.insert(key, name.clone());
        name
    }
}
impl Demand {
    pub(super) fn span_population(
        scopes: &std::collections::BTreeSet<String>,
        inline: &mut super::Definitions,
    ) -> String {
        let (ctes, body) = super::waves::fill(scopes.iter().cloned());
        inline.with_extra(ctes, body)
    }

    /// Ordinary CTE dependencies determine execution order. These fills perform no host lookups.
    pub(super) fn population(&self, inline: &mut super::Definitions) -> String {
        let (_, table, columns) = self.property.shape();
        let guard = self.property.guard();
        let (function, selected) = match self.property {
            Property::Node => ("nera_decode(s.fid,s.pre)".to_owned(), "nera_word(packed,0)-1,nera_word(packed,1),nera_word(packed,2),nera_word(packed,3),nera_word(packed,4)".to_owned()),
            Property::Span => ("nera_span(s.fid,s.pre)".to_owned(), "nera_word(packed,0),nera_word(packed,1)".to_owned()),
            Property::Child(field) => (format!("nera_child(s.fid,s.pre,{field})"), format!("{field},packed")),
        };
        let mut dependencies = Vec::new();
        if self.property == Property::Node {
            if self.scope == "all_nodes" {
                dependencies.push("missing_files(fid) AS MATERIALIZED (SELECT cf.fid FROM cf LEFT JOIN decoded_files d ON d.fid=cf.fid WHERE d.fid IS NULL)".into());
                return inline.with_extra(
                    dependencies,
                    "INSERT INTO decoded_properties SELECT f.fid,p.pre,p.parent,p.end,p.kind,p.field,p.flags FROM missing_files f JOIN file t ON t.fid=f.fid CROSS JOIN nera_nodes(f.fid,0,t.nodes) p LEFT JOIN decoded_properties d ON d.fid=f.fid AND d.pre=p.pre WHERE d.fid IS NULL",
                );
            }
            dependencies.push(format!("needed(fid,pre) AS MATERIALIZED (SELECT DISTINCT s.fid,s.pre FROM {} s LEFT JOIN decoded_properties d ON d.fid=s.fid AND d.pre=s.pre WHERE d.fid IS NULL)",self.scope));
            return inline.with_extra(
                dependencies,
                "INSERT INTO decoded_properties SELECT s.fid,p.pre,p.parent,p.end,p.kind,p.field,p.flags FROM needed s CROSS JOIN nera_nodes(s.fid,s.pre,s.pre+1) p",
            );
        }
        if self.property == Property::Node && self.scope == "all_nodes" {
            dependencies.push("missing_files(fid) AS MATERIALIZED (SELECT cf.fid FROM cf LEFT JOIN decoded_files d ON d.fid=cf.fid WHERE d.fid IS NULL)".into());
            dependencies.push("needed(fid,pre) AS MATERIALIZED (SELECT s.fid,s.pre FROM missing_files f CROSS JOIN all_nodes s LEFT JOIN decoded_properties d ON d.fid=s.fid AND d.pre=s.pre WHERE s.fid=f.fid AND d.fid IS NULL)".into());
        } else {
            dependencies.push(format!("needed(fid,pre) AS MATERIALIZED (SELECT DISTINCT s.fid,s.pre FROM {} s LEFT JOIN {table} d ON d.fid=s.fid AND d.pre=s.pre{guard} WHERE d.fid IS NULL)", self.scope));
        }
        dependencies.push(format!(
            "decoded(fid,pre,packed) AS MATERIALIZED (SELECT s.fid,s.pre,{function} FROM needed s)"
        ));
        inline.with_extra(
            dependencies,
            &format!(
                "INSERT INTO {table}(fid,pre,{columns}) SELECT fid,pre,{selected} FROM decoded"
            ),
        )
    }
}

impl Property {
    /// One trusted declaration per algorithm. Selectors request keys; they are not binding options.
    pub(super) fn binding(
        self,
        registry: &mut crate::code_rule_dsl::producers::Registry<u32>,
        language: u32,
        consumer: String,
    ) -> usize {
        use crate::code_rule_dsl::producers::{Binding, Column, Contract, Definition, OptionValue};
        use std::collections::BTreeMap;
        let (name, function, output) = match self {
            Self::Node => (
                "syntax.node_properties.v1",
                "nera_nodes",
                "parent,end,kind,field,flags",
            ),
            Self::Span => ("syntax.source_spans.v1", "nera_span", "start,len"),
            Self::Child(_) => ("syntax.child_fields.v1", "nera_child", "field,child"),
        };
        let mut columns = "fid,pre"
            .split(',')
            .chain(output.split(','))
            .map(|name| Column {
                name: name.into(),
                kind: "integer".into(),
                nullable: false,
            })
            .collect::<Vec<_>>();
        if let Self::Child(_) = self {
            columns.last_mut().expect("child column").nullable = true;
        }
        let options: BTreeMap<String, OptionValue> = if let Self::Child(field) = self {
            BTreeMap::from([("field".into(), OptionValue::Integer(i64::from(field)))])
        } else {
            BTreeMap::new()
        };
        let contract = registry
            .register(Contract {
                name: name.into(),
                origin: concat!(file!(), ":Property::binding").into(),
                definition: Definition {
                    inputs: BTreeMap::from([("tape".into(), "syntax-tape".into())]),
                    options: options
                        .keys()
                        .map(|name| (name.clone(), "integer".into()))
                        .collect(),
                    columns,
                    key: vec!["fid".into(), "pre".into()],
                    completion: "keys".into(),
                    multiplicity: "set".into(),
                    ordering: vec![],
                    implementation: BTreeMap::from([
                        ("backend".into(), "sqlite-native-function".into()),
                        ("symbol".into(), function.into()),
                        ("revision".into(), "1".into()),
                    ]),
                },
            })
            .expect("trusted engine producer declaration");
        registry
            .bind(
                Binding {
                    contract,
                    inputs: BTreeMap::from([("tape".into(), language)]),
                    options,
                    domain: None,
                },
                consumer,
            )
            .expect("trusted engine producer binding")
    }
}
