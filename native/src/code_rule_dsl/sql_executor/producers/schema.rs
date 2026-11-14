//! Preparation-only native schemas. Explicit temp qualification uses rolled-back views.
use super::*;
use crate::code_rule_dsl::facts::vtab::Modules;

pub fn with_schema<T>(
    db: &mut Database,
    emitted: &plan::emit::Emitted,
    program: &front::Program,
    producers: &[Producer],
    prepare: impl FnOnce(&mut Database, &str) -> Result<T>,
) -> Result<T> {
    let modules = Modules::default();
    let mut registered = Vec::new();
    let mut qualified_scope = false;
    let result = (|| {
        let named = producers
            .iter()
            .flat_map(|p| words(&p.query))
            .collect::<BTreeSet<_>>();
        // Explicit temp qualification bypasses both eponymous modules and CTE aliases.
        // Keep that SQL interface with a small, rolled-back facade only for named reads.
        if named.contains("temp") {
            db.execute_batch("SAVEPOINT rr_producer_qualified_schema")
                .map_err(error)?;
            qualified_scope = true;
        }
        let mut aliases = Vec::new();
        for (rel, width) in emitted.program.relations.iter().cloned().chain(
            emitted
                .views
                .iter()
                .map(|(rel, q)| (rel.clone(), q.body.columns.len())),
        ) {
            let typed = format!("{}rel_{rel}", catalog::PREFIX);
            if !named.contains(&rel.to_ascii_lowercase())
                && !named.contains(&typed.to_ascii_lowercase())
            {
                continue;
            }
            // Real inputs, previous results, and extension views retain their owner.
            if db
                .prepare(&format!("SELECT * FROM {} LIMIT 0", quote(&rel)))
                .is_err()
            {
                modules
                    .bind(
                        db,
                        &rel,
                        Arc::new(live::Live::from_rows(
                            (0..width).map(|c| format!("c{c}")).collect(),
                            [],
                        )),
                    )
                    .map_err(error)?;
                registered.push(rel.clone());
            }
            if qualified_scope
                && db
                    .prepare(&format!("SELECT * FROM temp.{} LIMIT 0", quote(&rel)))
                    .is_err()
            {
                db.execute_batch(&format!(
                    "CREATE TEMP VIEW {} AS SELECT * FROM main.{}",
                    quote(&rel),
                    quote(&rel)
                ))
                .map_err(error)?;
            }
            if named.contains(&typed.to_ascii_lowercase())
                && let Some(decl) = program.decls.get(&rel)
            {
                let names = catalog::view_columns(decl)
                    .iter()
                    .map(|c| quote(c))
                    .collect::<Vec<_>>()
                    .join(",");
                // A CTE shadows stale catalog views without changing the session schema.
                aliases.push(format!(
                    "{}({names}) AS (SELECT * FROM {})",
                    quote(&typed),
                    quote(&rel)
                ));
                if qualified_scope {
                    db.execute_batch(&format!("DROP VIEW IF EXISTS temp.{}; CREATE TEMP VIEW {}({names}) AS SELECT * FROM {}",quote(&typed),quote(&typed),quote(&rel))).map_err(error)?;
                }
            }
        }
        if emitted
            .setup
            .iter()
            .any(|s| s.starts_with("CREATE TEMP TABLE rdl_int("))
        {
            constants::integer_domain(db)?;
        }
        let prefix = if aliases.is_empty() {
            String::new()
        } else {
            format!("WITH {} ", aliases.join(","))
        };
        prepare(db, &prefix)
    })();
    let qualified_cleanup = if qualified_scope {
        db.execute_batch(
            "ROLLBACK TO rr_producer_qualified_schema; RELEASE rr_producer_qualified_schema",
        )
        .map_err(error)
    } else {
        Ok(())
    };
    // Unregister every temporary module, including after a failed prepare or partial setup.
    let mut cleanup = qualified_cleanup;
    for name in registered.into_iter().rev() {
        if let Err(err) = modules.unregister(db, &name) {
            cleanup = Err(error(err));
        }
    }
    match result {
        Ok(value) => cleanup.map(|_| value),
        Err(err) => Err(err),
    }
}
