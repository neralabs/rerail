//! Compile a relational clause and validate every SQL statement against its
//! finite table schema before any source is read.
use super::{define::Defines, evidence::compile_relation};
use crate::code_rule_dsl::json::{invalid, non_empty_str, object, reject_unknown};
use crate::code_rule_dsl::model::Capabilities;
use crate::code_rule_dsl::relational::{self, Plan, Stage};
use napi::Result;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, atomic::AtomicUsize};
#[path = "relational/inference.rs"]
mod inference;

fn names(value: &Value, path: &str) -> Result<Vec<String>> {
    let array = value
        .as_array()
        .filter(|items| !items.is_empty() && items.len() <= 16)
        .ok_or_else(|| invalid(format!("{path} must contain 1 to 16 field names")))?;
    array
        .iter()
        .map(|value| non_empty_str(Some(value), path).map(str::to_owned))
        .collect()
}
fn text(value: Option<&Value>, path: &str, limit: usize) -> Result<String> {
    let text = non_empty_str(value, path)?;
    if text.len() > limit {
        return Err(invalid(format!("{path} exceeds maxAstPatternBytes")));
    }
    Ok(text.to_owned())
}

pub(super) fn compile(
    value: &Value,
    path: &str,
    capabilities: &Capabilities,
    defines: &Defines,
    required_ast: &mut BTreeSet<(String, String)>,
) -> Result<Plan> {
    let spec = object(value, path)?;
    reject_unknown(
        spec,
        &[
            "inputs",
            "stages",
            "query",
            "key",
            "code",
            "references",
            "locations",
            "partitionBy",
            "syntax",
            "requireNonemptyInput",
        ],
        path,
    )?;
    let eager_types = spec
        .get("stages")
        .and_then(Value::as_array)
        .is_some_and(|stages| stages.iter().any(|stage| stage.get("deferred").is_some()));
    let per_file = match spec.get("partitionBy") {
        None => false,
        Some(Value::String(value)) if value == "file" => true,
        Some(Value::String(value)) if value == "repository" => false,
        _ => {
            return Err(invalid(format!(
                "{path}/partitionBy must be `file` or `repository`"
            )));
        }
    };
    if per_file && (spec.contains_key("references") || spec.contains_key("locations")) {
        return Err(invalid(format!(
            "{path}: file partitions cannot declare references or locations"
        )));
    }
    let input_spec = object(
        spec.get("inputs")
            .ok_or_else(|| invalid(format!("{path}/inputs is required")))?,
        &format!("{path}/inputs"),
    )?;
    if input_spec.is_empty() || input_spec.len() > 32 {
        return Err(invalid(format!(
            "{path}/inputs must contain 1 to 32 relations"
        )));
    }
    let mut locations = BTreeSet::new();
    if let Some(value) = spec.get("locations") {
        let items = value
            .as_array()
            .filter(|values| !values.is_empty() && values.len() <= 4096)
            .ok_or_else(|| invalid(format!("{path}/locations must contain 1 to 4096 paths")))?;
        for value in items {
            let location = non_empty_str(Some(value), &format!("{path}/locations"))?;
            if location.len() > 4096
                || location.contains(['\\', ':', '\0'])
                || location
                    .split('/')
                    .any(|part| matches!(part, "" | "." | ".."))
                || !locations.insert(location.to_owned())
            {
                return Err(invalid(format!(
                    "{path}/locations must contain distinct normalized relative paths"
                )));
            }
        }
    }
    let work = Arc::new(AtomicUsize::new(usize::MAX));
    let db = relational::validation_connection(
        capabilities.limits.max_regex_pattern_bytes,
        Arc::clone(&work),
    )?;
    #[cfg(feature = "code-rule-profile")]
    db.context(serde_json::json!({"scope":"compile", "producer":"relational-schema-validation", "path":path}));
    let mut tables = BTreeSet::new();
    let mut schemas = relational::schema::Registry::new();
    let mut inference = relational::schema::inference::Inference::default();
    let mut inputs = Vec::new();
    for (name, value) in input_spec {
        if !relational::identifier(name) {
            return Err(invalid(format!(
                "{path}/inputs/{name} is not a relation identifier"
            )));
        }
        if tables
            .iter()
            .any(|table: &String| table.eq_ignore_ascii_case(name))
        {
            return Err(invalid("input relations must have distinct names"));
        }
        let input = compile_relation(
            value,
            &format!("{path}/inputs/{name}"),
            capabilities,
            defines,
            required_ast,
        )?;
        if input.predicate.is_some() || !input.closure.is_empty() {
            return Err(invalid(format!(
                "{path}/inputs/{name}: use SQL WHERE and recursive stages instead of evidence where/closure"
            )));
        }
        let fields = input
            .project
            .iter()
            .map(|project| project.field.clone())
            .collect::<Vec<_>>();
        let mut unique = BTreeSet::new();
        if fields
            .iter()
            .any(|name| !relational::identifier(name) || !unique.insert(name.to_ascii_lowercase()))
        {
            return Err(invalid(format!(
                "{path}/inputs/{name}: projected fields must be distinct relation identifiers"
            )));
        }
        schemas.insert(
            name.clone(),
            Arc::new(
                relational::schema::register(&db, name, relational::input_schema(name, &fields))
                    .map_err(|error| invalid(format!("relational query: {error}")))?,
            ),
        );
        tables.insert(name.clone());
        inputs.push((name.clone(), input));
    }
    let require_nonempty = spec
        .get("requireNonemptyInput")
        .map(|value| -> Result<usize> {
            let name = non_empty_str(Some(value), &format!("{path}/requireNonemptyInput"))?;
            if !relational::identifier(name) {
                return Err(invalid(
                    "requireNonemptyInput must be a relation identifier",
                ));
            }
            inputs
                .iter()
                .position(|(input, _)| input == name)
                .ok_or_else(|| invalid("requireNonemptyInput must name a declared input"))
        })
        .transpose()?;
    let syntax = spec
        .get("syntax")
        .map(|value| -> Result<relational::syntax::Spec> {
            let mut syntax: relational::syntax::Spec =
                serde_json::from_value(value.clone()).map_err(|e| invalid(e.to_string()))?;
            if !inputs.iter().any(|(name, _)| name == &syntax.gate) {
                return Err(invalid("syntax gate must name an input"));
            }
            if syntax.fields.len() > 32
                || syntax.exclude_kinds.len() > 32
                || syntax.empty_text_kinds.len() > 32
                || syntax.columns.len() > 256
                || syntax.owners.len() > 8
            {
                return Err(invalid(
                    "syntax declaration exceeds its field or owner bound",
                ));
            }
            let gate = inputs
                .iter()
                .find(|(name, _)| name == &syntax.gate)
                .unwrap();
            let crate::code_rule_dsl::model::Query::Ast { matchers, .. } = &gate.1.ast.query else {
                return Err(invalid("syntax gate requires one AST grammar"));
            };
            let language = matchers
                .first()
                .ok_or_else(|| invalid("syntax gate has no grammar"))?
                .language;
            if matchers.iter().any(|m| m.language != language) {
                return Err(invalid("syntax gate must select one grammar"));
            }
            let grammar = crate::code_rule_dsl::tape::Grammar::new(language).map_err(invalid)?;
            for kind in syntax.exclude_kinds.iter().chain(&syntax.empty_text_kinds) {
                if kind.is_empty() || kind.len() > 128 {
                    return Err(invalid("invalid syntax kind"));
                }
            }
            let mut fields = BTreeSet::new();
            for field in &syntax.fields {
                if !fields.insert(field) || !grammar.has_field(field) {
                    return Err(invalid("syntax field is unknown or repeated"));
                }
            }
            let mut declared = vec![&syntax.name];
            declared.extend(syntax.edges.iter());
            declared.extend(syntax.ownership.iter());
            for name in declared {
                if !relational::identifier(name)
                    || tables.iter().any(|n| n.eq_ignore_ascii_case(name))
                {
                    return Err(invalid("syntax relations must have distinct names"));
                }
                tables.insert(name.clone());
            }
            for (name, owner) in &syntax.owners {
                if !relational::identifier(name)
                    || matches!(name.as_str(), "id" | "__path")
                    || owner.kind.is_empty()
                    || owner.kind.len() > 128
                {
                    return Err(invalid("invalid syntax owner declaration"));
                }
            }
            for field in &syntax.fields {
                syntax
                    .field_codes
                    .insert(field.clone(), grammar.field_code(field).unwrap_or(0));
            }
            schemas.extend(relational::syntax::compiler_schema(&db, &syntax)?);
            Ok(syntax)
        })
        .transpose()?;
    let references = spec.get("references").map(|value| -> Result<_> {
        let at = format!("{path}/references");
        let spec = object(value,&at)?;
        reject_unknown(spec,&["language","siblingLinks","constructorFields"],&at)?;
        if spec.get("language").and_then(Value::as_str) != Some("python") { return Err(invalid(format!("{at}/language must be python"))); }
        if tables.iter().any(|name|name.eq_ignore_ascii_case("reference_edges")) { return Err(invalid("reference_edges is reserved for the native graph")); }
        let mut calls = spec.clone(); calls.remove("language");
        calls.insert("graph".to_owned(),Value::String("references".to_owned()));
        calls.insert("at".to_owned(),Value::String("definition".to_owned()));
        calls.insert("kind".to_owned(),Value::String("function_definition".to_owned()));
        calls.insert("not".to_owned(),serde_json::json!({"kind":"function_definition"}));
        let query = super::compile_query(&serde_json::json!({"ast":{"language":"python","rule":{"kind":"function_definition","calls":calls}}}),&at,capabilities,defines,required_ast)?;
        let key = query.calls().first().ok_or_else(||invalid("native graph term was not compiled"))?.through.links;
        schemas.insert("reference_edges".to_owned(),Arc::new(relational::schema::register(&db,"reference_edges",relational::input_schema("reference_edges",&relational::REFERENCE_FIELDS.iter().map(|field|(*field).to_owned()).collect::<Vec<_>>())).map_err(|error|invalid(format!("relational query: {error}")))?));
        tables.insert("reference_edges".to_owned());
        Ok(relational::References { query,key })
    }).transpose()?;
    let mut stages = Vec::new();
    let mut last_uses = BTreeMap::new();
    let mut input_fields = relational::FieldReads::new();
    let mut reads_source = syntax.is_some();
    if let Some(value) = spec.get("stages") {
        let values = value
            .as_array()
            .filter(|items| items.len() <= 64)
            .ok_or_else(|| invalid(format!("{path}/stages must contain at most 64 stages")))?;
        for (index, value) in values.iter().enumerate() {
            let at = format!("{path}/stages/{index}");
            let spec = object(value, &at)?;
            reject_unknown(
                spec,
                &["name", "query", "indexes", "reachability", "deferred"],
                &at,
            )?;
            let name = non_empty_str(spec.get("name"), &format!("{at}/name"))?.to_owned();
            if !relational::identifier(&name)
                || tables.iter().any(|table| table.eq_ignore_ascii_case(&name))
            {
                return Err(invalid(format!(
                    "{at}/name must be a new relation identifier"
                )));
            }
            let mut stage_reads_source = false;
            let mut dependencies = BTreeSet::new();
            let reachability = spec
                .get("reachability")
                .map(|value| -> Result<_> {
                    if spec.contains_key("query")
                        || spec.contains_key("indexes")
                        || spec.contains_key("deferred")
                    {
                        return Err(invalid(format!(
                            "{at}: reachability cannot declare query or indexes"
                        )));
                    }
                    let shape = object(value, &format!("{at}/reachability"))?;
                    reject_unknown(shape, &["roots", "edges", "batchSize", "graph", "syntax"], &at)?;
                    let syntax_source=shape.get("syntax").map(|v|non_empty_str(Some(v),&at).map(str::to_owned)).transpose()?;
                    if let Some(name)=&syntax_source && (syntax.as_ref().is_none_or(|s| &s.name!=name) || shape.contains_key("edges") || shape.contains_key("graph")) {return Err(invalid("syntax reachability must name the declared syntax source and cannot declare edges or graph"));}
                    let forest = match shape.get("graph").and_then(Value::as_str) {
                        None if !shape.contains_key("graph") => false,
                        Some("directed") => false,
                        Some("forest") => true,
                        _ => {
                            return Err(invalid(format!(
                                "{at}/reachability/graph must be directed or forest"
                            )));
                        }
                    };
                    let mut queries = Vec::new();
                    let mut expand = false;
                    for (field, required) in [
                        ("roots", vec!["__path", "id"]),
                        ("edges", vec!["__path", "src", "dst"]),
                    ] {
                        if field=="edges" && syntax_source.is_some() { queries.push(String::new());continue; }
                        let query = text(
                            shape.get(field),
                            &format!("{at}/reachability/{field}"),
                            capabilities.limits.max_ast_pattern_bytes,
                        )?;
                        let (selected, reads, source) = relational::inspect_fields(&db, &query, &tables)?;
                        reads_source |= source;
                        stage_reads_source |= source;
                        for (name,fields) in reads {
                            dependencies.insert(name.clone());
                            input_fields.entry(name.clone()).or_default().extend(fields);
                            last_uses.insert(name, index);
                        }
                        if field=="roots" && selected==["__path","id","expand"] {
                            expand=true;
                        } else if selected != required {
                            return Err(invalid(format!(
                                "{at}/reachability/{field} must select {} in order",
                                required.join(",")
                            )));
                        }
                        queries.push(query);
                    }
                    let batch_size = match shape.get("batchSize") {
                        None => 256,
                        Some(value) => value
                            .as_u64()
                            .filter(|v| (1..=4096).contains(v))
                            .ok_or_else(|| {
                                invalid(format!("{at}/reachability/batchSize must be 1 to 4096"))
                            })? as usize,
                    };
                    Ok(relational::Reachability {
                        roots: queries.remove(0),
                        edges: queries.remove(0),
                        batch_size,
                        forest,
                        syntax: syntax_source,
                        expand,
                    })
                })
                .transpose()?;
            let query = if reachability.is_some() {
                "SELECT CAST(NULL AS TEXT) AS __path,NULL AS root,NULL AS node WHERE 0".to_owned()
            } else {
                text(
                    spec.get("query"),
                    &format!("{at}/query"),
                    capabilities.limits.max_ast_pattern_bytes,
                )?
            };
            let (mut columns, reads, source, recursive) =
                relational::inspect_fields_recursive(&db, &query, &tables)?;
            let schema_reads = reads.keys().cloned().collect::<Vec<_>>();
            dependencies.extend(schema_reads.iter().cloned());
            reads_source |= source;
            stage_reads_source |= source;
            for (name, fields) in reads {
                input_fields.entry(name.clone()).or_default().extend(fields);
                last_uses.insert(name, index);
            }
            let mut deferred = BTreeMap::<String, String>::new();
            if let Some(value) = spec.get("deferred") {
                let fields = object(value, &format!("{at}/deferred"))?;
                if fields.is_empty() || fields.len() > 32 {
                    return Err(invalid(format!(
                        "{at}/deferred must contain 1 to 32 columns"
                    )));
                }
                for (field, expression) in fields {
                    if !relational::identifier(field)
                        || deferred.keys().any(|name| name.eq_ignore_ascii_case(field))
                        || columns
                            .iter()
                            .any(|name| name.eq_ignore_ascii_case(field) && name != field)
                    {
                        return Err(invalid(format!(
                            "{at}/deferred must name distinct relation columns"
                        )));
                    }
                    deferred.insert(
                        field.clone(),
                        text(
                            Some(expression),
                            &format!("{at}/deferred/{field}"),
                            capabilities.limits.max_ast_pattern_bytes,
                        )?,
                    );
                }
            }
            let stored_columns = columns
                .iter()
                .filter(|c| !deferred.contains_key(*c))
                .cloned()
                .collect::<Vec<_>>();
            if stored_columns.is_empty() {
                return Err(invalid(format!(
                    "{at}: a stage must retain a physical column"
                )));
            }
            for field in deferred.keys() {
                if !columns.contains(field) {
                    columns.push(field.clone());
                }
            }
            let indexes = spec
                .get("indexes")
                .map(|value| {
                    value
                        .as_array()
                        .filter(|items| items.len() <= 8)
                        .ok_or_else(|| {
                            invalid(format!("{at}/indexes must contain at most 8 indexes"))
                        })?
                        .iter()
                        .map(|value| {
                            let fields = names(value, &format!("{at}/indexes"))?;
                            if fields.iter().any(|field| !stored_columns.contains(field)) {
                                return Err(invalid(format!(
                                    "{at}/indexes names an unknown column"
                                )));
                            }
                            Ok(fields)
                        })
                        .collect::<Result<Vec<_>>>()
                })
                .transpose()?
                .unwrap_or_default();
            let mut stage = Stage {
                inputs: dependencies,
                reads_source: stage_reads_source,
                recursive,
                name,
                query,
                indexes,
                reachability,
                deferred,
                stored_columns,
                output_columns: columns,
                declaration: Arc::from(""),
            };
            inference
                .before(
                    &db,
                    &mut schemas,
                    &schema_reads,
                    &stage.query,
                    &stage.output_columns,
                )
                .map_err(|error| invalid(format!("relational query: {error}")))?;
            if !stage.deferred.is_empty() {
                inference
                    .flush(&db, &mut schemas)
                    .map_err(|error| invalid(format!("relational query: {error}")))?;
            }
            let schema_key = relational::schema::cache::Key::new(
                &stage.query,
                &schemas,
                schema_reads.iter(),
                stage.deferred.keys(),
            );
            let cached = if eager_types {
                relational::schema::cache::shared()
                    .get(&schema_key)
                    .cloned()
            } else {
                None
            };
            let (columns, created) = if !eager_types {
                let columns = Arc::new(
                    stage
                        .output_columns
                        .iter()
                        .map(|name| relational::schema::Column {
                            name: name.clone(),
                            declared_type: String::new(),
                        })
                        .collect::<Vec<_>>(),
                );
                relational::schema::bind(
                    &db,
                    &stage.name,
                    relational::schema::declaration(&stage.name, &columns),
                )
                .map_err(relational::sql_error)?;
                (columns, false)
            } else if let Some(columns) = cached {
                relational::schema::bind(
                    &db,
                    &stage.name,
                    relational::schema::declaration(&stage.name, &columns),
                )
                .map_err(|error| invalid(format!("relational query: {error}")))?;
                (columns, false)
            } else if stage.deferred.is_empty() {
                let columns = inference
                    .queue(
                        &db,
                        &mut schemas,
                        &stage.name,
                        &stage.query,
                        stage.output_columns.clone(),
                        schema_key,
                    )
                    .map_err(|error| invalid(format!("relational query: {error}")))?;
                (columns, false)
            } else {
                relational::create_stage(&db, &stage, false, &work)?;
                let columns = Arc::new(
                    relational::schema::describe(&db, &stage.name)
                        .map_err(|error| invalid(format!("relational query: {error}")))?,
                );
                relational::schema::cache::shared().insert(schema_key, columns.clone());
                (columns, true)
            };
            reads_source |= relational::deferred::validate(&db, &stage, &columns)?;
            let physical_columns = columns
                .iter()
                .filter(|c| !stage.deferred.contains_key(&c.name))
                .cloned()
                .collect::<Vec<_>>();
            stage.declaration = relational::schema::declaration(
                &relational::deferred::physical_name(&stage),
                &physical_columns,
            )
            .replacen("CREATE TABLE", "CREATE TEMP TABLE", 1)
            .into();
            let columns = relational::deferred::expose(&db, &stage, columns, created)?;
            schemas.insert(stage.name.clone(), columns);
            tables.insert(stage.name.clone());
            stages.push(stage);
        }
    }
    inference
        .flush(&db, &mut schemas)
        .map_err(|error| invalid(format!("relational query: {error}")))?;
    for stage in &mut stages {
        let physical = schemas[&stage.name]
            .iter()
            .filter(|column| !stage.deferred.contains_key(&column.name))
            .cloned()
            .collect::<Vec<_>>();
        stage.declaration =
            relational::schema::declaration(&relational::deferred::physical_name(stage), &physical)
                .replacen("CREATE TABLE", "CREATE TEMP TABLE", 1)
                .into();
    }
    let query = text(
        spec.get("query"),
        &format!("{path}/query"),
        capabilities.limits.max_ast_pattern_bytes,
    )?;
    let (columns, reads, source) = relational::inspect_fields(&db, &query, &tables)?;
    let final_reads = reads.keys().cloned().collect();
    reads_source |= source;
    for (name, fields) in reads {
        input_fields.entry(name.clone()).or_default().extend(fields);
        last_uses.insert(name, stages.len());
    }
    for required in ["path", "start_byte", "end_byte"] {
        if !columns.iter().any(|column| column == required) {
            return Err(invalid(format!("{path}/query must select {required}")));
        }
    }
    let key = spec
        .get("key")
        .map(|value| names(value, &format!("{path}/key")))
        .transpose()?
        .unwrap_or_default();
    let code = spec
        .get("code")
        .map(|value| non_empty_str(Some(value), &format!("{path}/code")).map(str::to_owned))
        .transpose()?;
    for field in key.iter().chain(code.iter()) {
        if !columns.contains(field) {
            return Err(invalid(format!("{path}: unknown output column {field}")));
        }
    }
    // SQLite supplies actual column dependencies, including predicates and nested queries.
    // Keep the original matching/anchor semantics while omitting unconsumed projection work.
    for (name, input) in &mut inputs {
        let fields = input_fields.get(name);
        input.retain_projected_fields(fields);
    }
    let mut plan = Plan {
        require_nonempty,
        syntax: syntax.map(Arc::new),
        per_file,
        references,
        locations,
        inputs,
        stages,
        query,
        key,
        code,
        last_uses,
        reads_source,
    };
    relational::pipeline::lower(&mut plan, final_reads);
    if !eager_types {
        inference::finish(&mut plan, &db, &mut schemas)?;
    }
    Ok(plan)
}
