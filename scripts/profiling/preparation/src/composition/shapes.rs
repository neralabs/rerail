use super::{Meter, code};
use rusqlite::{Connection, Result};
use std::collections::BTreeMap;

pub(super) fn ranges(
    db: &Connection,
    v: &BTreeMap<String, i64>,
    variant: &str,
    m: &mut Meter,
) -> Result<Vec<Vec<i64>>> {
    let roots = format!(
        "{},{},{},{}",
        code(v, "kind:function_definition"),
        code(v, "kind:assignment"),
        code(v, "kind:block"),
        code(v, "kind:module")
    );
    let blocked = [
        "module",
        "block",
        "expression_statement",
        "argument_list",
        "decorated_definition",
        "type",
    ]
    .iter()
    .filter_map(|name| v.get(&format!("kind:{name}")))
    .map(i64::to_string)
    .collect::<Vec<_>>()
    .join(",");
    let call = code(v, "kind:call");
    if variant == "closure" || variant == "demanded-closure" {
        let seed = if variant == "demanded-closure" {
            format!(" WHERE kind IN ({roots})")
        } else {
            String::new()
        };
        m.statement(db,&format!("CREATE TEMP TABLE closure AS WITH RECURSIVE w(fid,root,node,expand) AS (SELECT fid,pre,pre,kind NOT IN ({blocked}) FROM n{seed} UNION SELECT w.fid,w.root,c.pre,w.expand FROM w JOIN n c ON c.fid=w.fid AND c.parent=w.node WHERE w.expand) SELECT fid,root,node FROM w"))?;
        m.statement(
            db,
            "CREATE UNIQUE INDEX temp.closure_root ON closure(fid,root,node)",
        )?;
        m.statement(
            db,
            "CREATE INDEX temp.closure_node ON closure(fid,node,root)",
        )?;
        m.statement(db,&format!("SELECT r.fid,r.pre,count(c.pre),coalesce(min(c.pre),-1) FROM n r LEFT JOIN closure w ON w.fid=r.fid AND w.root=r.pre LEFT JOIN n c ON c.fid=w.fid AND c.pre=w.node AND c.kind={call} WHERE r.kind IN ({roots}) GROUP BY r.fid,r.pre"))
    } else if variant == "interval" {
        m.statement(db,&format!("SELECT r.fid,r.pre,count(c.pre),coalesce(min(c.pre),-1) FROM n r LEFT JOIN n c ON c.fid=r.fid AND c.pre>=r.pre AND c.pre<CASE WHEN r.kind IN ({blocked}) THEN r.pre+1 ELSE r.end END AND c.kind={call} WHERE r.kind IN ({roots}) GROUP BY r.fid,r.pre"))
    } else {
        Err(rusqlite::Error::InvalidParameterName(variant.into()))
    }
}

pub(super) const REMOVED: &str = "^(?:_execute_grammar|_execute_sql_or_schema_statement|_execute_sql_or_schema_statement_async|_execute_template_with_params|_execute_template_with_params_async|execute_grammar|execute_sql|fetchall_grammar|fetchall_raw|fetchall_raw_async|fetchone_grammar|fetchone_raw|fetchone_raw_async|nonportably_execute_sql|raw_execute|raw_execute_async|raw_execute_fetch|raw_execute_fetch_async)$";

pub(super) fn demand(
    db: &Connection,
    v: &BTreeMap<String, i64>,
    variant: &str,
    m: &mut Meter,
) -> Result<Vec<Vec<i64>>> {
    let identifier = code(v, "kind:identifier");
    let call = code(v, "kind:call");
    let attribute = code(v, "kind:attribute");
    let function_field = code(v, "field:function");
    let attribute_field = code(v, "field:attribute");
    let predicate = if variant == "eager" {
        m.statement(db,"CREATE TEMP TABLE text_nodes(fid INTEGER,pre INTEGER,text TEXT,PRIMARY KEY(fid,pre)) WITHOUT ROWID")?;
        m.statement(db,&format!("CREATE TEMP TABLE spans AS SELECT fid,pre,nera_span(fid,pre) AS span FROM n WHERE kind={identifier}"))?;
        m.statement(db,"INSERT INTO text_nodes SELECT fid,pre,nera_slice(fid,nera_word(span,0),nera_word(span,1)) FROM spans")?;
        format!(
            "EXISTS (SELECT 1 FROM text_nodes t WHERE t.fid=i.fid AND t.pre=i.pre AND nera_term_matches('{REMOVED}',t.text))"
        )
    } else if variant == "selective" || variant == "cut" {
        format!("nera_matches('{REMOVED}',i.fid,i.pre)")
    } else {
        return Err(rusqlite::Error::InvalidParameterName(variant.into()));
    };
    let keys = format!(
        "SELECT c.fid,c.pre AS root,a.pre AS attr,i.pre AS witness FROM n c JOIN n a ON a.fid=c.fid AND a.parent=c.pre AND a.kind={attribute} AND a.field={function_field} JOIN n i ON i.fid=a.fid AND i.parent=a.pre AND i.kind={identifier} AND i.field={attribute_field} WHERE c.kind={call}"
    );
    if variant == "cut" {
        m.statement(db,&format!("WITH demanded AS MATERIALIZED ({keys}) SELECT fid,root,attr,witness FROM demanded WHERE nera_matches('{REMOVED}',fid,witness)"))
    } else {
        m.statement(db, &format!("{keys} AND {predicate}"))
    }
}

fn inline(depth: usize, leaf: &str) -> String {
    if depth == 0 {
        return leaf.into();
    }
    let child = inline(depth - 1, leaf);
    branch(&child)
}
fn branch(child: &str) -> String {
    // Distinct consumers share the same child. This is not the removable `U OR U` case.
    format!(
        "SELECT fid,pre FROM n WHERE ((fid,pre) IN ({child}) AND pre%2=0) OR ((fid,pre) IN ({child}) AND pre%3=0)"
    )
}
pub(super) fn utility(
    db: &Connection,
    v: &BTreeMap<String, i64>,
    variant: &str,
    depth: usize,
    m: &mut Meter,
) -> Result<Vec<Vec<i64>>> {
    let leaf = format!("SELECT fid,pre FROM n WHERE kind={}", code(v, "kind:call"));
    match variant {
        "inline" => m.statement(db, &inline(depth, &leaf)),
        "cte" => {
            let mut definitions = vec![format!("u0 AS MATERIALIZED ({leaf})")];
            for d in 1..=depth {
                let p = d - 1;
                definitions.push(format!(
                    "u{d} AS MATERIALIZED ({})",
                    branch(&format!("SELECT fid,pre FROM u{p}"))
                ));
            }
            m.statement(
                db,
                &format!(
                    "WITH {} SELECT fid,pre FROM u{depth}",
                    definitions.join(",")
                ),
            )
        }
        "cuts" => {
            m.statement(db, &format!("CREATE TEMP TABLE u0 AS {leaf}"))?;
            m.statement(db, "CREATE UNIQUE INDEX temp.u0_key ON u0(fid,pre)")?;
            for d in 1..=depth {
                let p = d - 1;
                m.statement(
                    db,
                    &format!(
                        "CREATE TEMP TABLE u{d} AS {}",
                        branch(&format!("SELECT fid,pre FROM u{p}"))
                    ),
                )?;
                m.statement(
                    db,
                    &format!("CREATE UNIQUE INDEX temp.u{d}_key ON u{d}(fid,pre)"),
                )?;
                m.statement(db, &format!("DROP TABLE u{p}"))?;
            }
            m.statement(db, &format!("SELECT fid,pre FROM u{depth}"))
        }
        _ => Err(rusqlite::Error::InvalidParameterName(variant.into())),
    }
}
