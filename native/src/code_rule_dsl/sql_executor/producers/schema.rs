//! Preparation-only schemas. No SQL tables, views, or execution roots are created.
use super::*;
use crate::code_rule_dsl::facts::vtab::Modules;

pub(super) fn with_schema<T>(
    db: &mut Database,
    emitted: &plan::emit::Emitted,
    program: &front::Program,
    producers: &[Producer],
    prepare: impl FnOnce(&mut Database, &str) -> Result<T>,
) -> Result<T> {
    let modules = Modules::default();
    let mut registered = Vec::new();
    let result = (|| {
        let named = producers
            .iter()
            .flat_map(|p| words(&p.query))
            .collect::<BTreeSet<_>>();
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
    // Unregister every temporary module, including after a failed prepare or partial setup.
    let mut cleanup = Ok(());
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
