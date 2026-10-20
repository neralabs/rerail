//! Read existing lexical inference, without implementing a second resolver.
use super::TapeSource;
use crate::code_rule_dsl::{
    language::GrammarDialect,
    scope::{Reading, ScopeTable},
};
use ast_grep_core::{AstGrep, Doc};
use rerail_reachability_spike::rows::{OwnedRows, Source};
use rusqlite::{Connection, Result, functions::FunctionFlags, types::Value, vtab::Context};
use std::sync::{Arc, OnceLock};

struct Bindings {
    source: Arc<TapeSource>,
    rows: OnceLock<std::result::Result<OwnedRows, String>>,
}
impl Bindings {
    fn get(&self) -> Result<&OwnedRows> {
        self.rows
            .get_or_init(|| self.build().map_err(|e| e.to_string()))
            .as_ref()
            .map_err(|e| rusqlite::Error::UserFunctionError(e.clone().into()))
    }
    fn build(&self) -> Result<OwnedRows> {
        let root = AstGrep::doc(self.source.doc.clone());
        let table = ScopeTable::build_in(
            root.root(),
            GrammarDialect::Python,
            200_000_000,
            &|| self.source.charge(1).is_err(),
            Reading::default(),
            None,
        )
        .map_err(|e| rusqlite::Error::UserFunctionError(format!("scope build: {e:?}").into()))?;
        self.source.charge(table.cost)?;
        let mut rows = Vec::new();
        for reference in &table.references {
            self.source.charge(1)?;
            let Some(name) = table.reference_name(reference, self.source.doc.get_source(), 0)
            else {
                continue;
            };
            let binding = table.resolve(reference, &name);
            let target = binding.and_then(|b| b.site.node);
            rows.push(vec![
                Value::Text(self.source.path.clone()),
                Value::Integer(reference.start as i64),
                Value::Integer(reference.end as i64),
                Value::Text(name.into_owned()),
                target.map_or(Value::Null, |n| Value::Integer(n.start as i64)),
                target.map_or(Value::Null, |n| Value::Integer(n.end as i64)),
                binding.map_or(Value::Null, |b| Value::Text(format!("{:?}", b.site.form))),
            ]);
        }
        OwnedRows::new(
            self.columns(),
            rows,
            vec![1, 3, 4],
            Arc::clone(&self.source.charge),
        )
    }
}
impl Source for Bindings {
    fn columns(&self) -> Vec<String> {
        [
            "__path",
            "reference_start",
            "reference_end",
            "name",
            "target_start",
            "target_end",
            "form",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect()
    }
    fn len(&self) -> usize {
        self.rows
            .get()
            .and_then(|r| r.as_ref().ok())
            .map_or(0, |r| r.rows.len())
    }
    fn estimated_len(&self) -> usize {
        self.source.len() / 5
    }
    fn initialize(&self) -> Result<()> {
        self.get().map(|_| ())
    }
    fn indexed(&self, c: usize) -> bool {
        matches!(c, 1 | 3 | 4)
    }
    fn lookup(&self, c: usize, v: &[Value]) -> Result<Option<Vec<usize>>> {
        self.get()?.lookup(c, v)
    }
    fn column(&self, r: usize, c: usize, ctx: &mut Context) -> Result<()> {
        self.get()?.column(r, c, ctx)
    }
    fn charge(&self, n: usize) -> Result<()> {
        self.source.charge(n)
    }
}
pub(super) fn register(db: &Connection, source: Arc<TapeSource>) -> Result<()> {
    let bindings = Arc::new(Bindings {
        source,
        rows: OnceLock::new(),
    });
    rerail_reachability_spike::rows::register(db, "native_bindings", bindings.clone())?;
    db.create_scalar_function(
        c"binding_start",
        2,
        FunctionFlags::SQLITE_UTF8,
        move |ctx| {
            let start = ctx.get::<i64>(0)?;
            let end = ctx.get::<i64>(1)?;
            let rows = bindings.get()?;
            let candidates = rows
                .lookup(1, &[Value::Integer(start)])?
                .unwrap_or_default();
            for i in candidates {
                if rows.rows[i][2] == Value::Integer(end) {
                    return Ok(rows.rows[i][4].clone());
                }
            }
            Ok(Value::Null)
        },
    )?;
    Ok(())
}
