//! Restricted transforms of known emitted shapes. Each generated query keeps the original
//! projection and is compared with its complete SELECT-body multiset on live engine inputs.

use regex::Regex;
use std::sync::LazyLock;

pub(super) struct Case {
    pub(super) family: &'static str,
    pub(super) input: String,
    pub(super) select: String,
    writer: String,
}

impl Case {
    /// Insert after top-level WITH definitions, never inside a CTE or quoted expression.
    pub(super) fn insert(&self, select: &str) -> String {
        let bytes = select.as_bytes();
        let mut depth = 0;
        let mut quote = None;
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            if let Some(q) = quote {
                if b == q {
                    if bytes.get(i + 1) == Some(&q) {
                        i += 2;
                        continue;
                    }
                    quote = None;
                }
            } else if b == b'\'' || b == b'"' {
                quote = Some(b);
            } else if b == b'(' {
                depth += 1;
            } else if b == b')' {
                depth -= 1;
            } else if depth == 0 && bytes[i..].starts_with(b"SELECT ") {
                return format!("{}{}{}", &select[..i], self.writer, &select[i..]);
            }
            i += 1;
        }
        panic!("no top-level SELECT in spike variant");
    }
}

pub(super) struct Variant {
    pub(super) name: &'static str,
    pub(super) sql: String,
}

static INPUT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bFROM ((?:s\d+_)?f\d+_\d+) f\b").unwrap());
static INSERT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"INSERT INTO [a-zA-Z0-9_]+(?:\([^)]*\))? ").unwrap());
static ATOM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"COALESCE\(\(SELECT atom FROM node WHERE file = f.file AND id = f\.(s\d+)\), f\.(s\d+)\)",
    )
    .unwrap()
});

pub(super) fn classify(sql: &str) -> Option<Case> {
    let family = if sql.contains("c.parent = fn.wpar AND c.id <= fn.aprev")
        && sql.contains(" f JOIN node fn ON fn.file = f.file AND fn.id = f.node JOIN node c ON ")
        && sql.starts_with("INSERT INTO ")
    {
        "sibling-walk"
    } else if sql.starts_with("INSERT INTO c")
        && sql.contains("SELECT DISTINCT f.file, f.node, COALESCE((SELECT atom")
        && sql.contains("WHERE NOT EXISTS (SELECT 1 FROM c")
    {
        "capture-demand"
    } else if sql.starts_with("WITH p AS MATERIALIZED") && sql.contains(" FROM pattern WHERE ") {
        "pattern-terminal"
    } else if (sql.starts_with("WITH t AS MATERIALIZED")
        || sql.starts_with("WITH e AS MATERIALIZED"))
        && sql.contains(" FROM fact WHERE ")
    {
        "fact-terminal"
    } else if sql.starts_with("INSERT INTO ")
        && sql.contains("SELECT NULL, c.file, c.node")
        && sql.contains(" JOIN node fn ON fn.file = f.file AND fn.id = f.node")
        && sql.contains(" JOIN node c ON ")
    {
        "relation-walk"
    } else {
        return None;
    };
    let input = INPUT.captures(sql)?.get(1)?.as_str().to_owned();
    let writer = INSERT.find(sql)?;
    let select = format!("{}{}", &sql[..writer.start()], &sql[writer.end()..]);
    Some(Case {
        family,
        input,
        select,
        writer: sql[writer.start()..writer.end()].to_owned(),
    })
}

pub(super) fn variants(case: &Case) -> Vec<Variant> {
    let mut variants = vec![Variant {
        name: "baseline",
        sql: case.select.clone(),
    }];
    let mut add = |name, sql| variants.push(Variant { name, sql });
    match case.family {
        "relation-walk" => {
            add(
                "cross-join",
                case.select
                    .replace(" JOIN node fn ON ", " CROSS JOIN node fn ON ")
                    .replace(" JOIN node c ON ", " CROSS JOIN node c ON "),
            );
        }
        "sibling-walk" => {
            let from = format!(
                "FROM {} f JOIN node fn ON fn.file = f.file AND fn.id = f.node JOIN node c ON ",
                case.input
            );
            if !case.select.contains(&from) {
                return variants;
            }
            let anchor_join = " JOIN node fn ON fn.file = f.file AND fn.id = f.node";
            let pinned = case
                .select
                .replace(anchor_join, &format!(" CROSS{anchor_join}"))
                .replace(" JOIN node c ON ", " CROSS JOIN node c ON ");
            add("cross-join", pinned.clone());
            if !case.select.contains(" UNION ALL ") {
                add(
                "comma-where",
                case.select.replace(
                    &from,
                    &format!(
                        "FROM {} f, node fn, node c WHERE fn.file = f.file AND fn.id = f.node AND ",
                        case.input
                    ),
                ),
            );
            }
            add(
                "derived-fence",
                case.select.replace(
                    &format!("FROM {} f", case.input),
                    &format!("FROM (SELECT * FROM {} LIMIT -1) f", case.input),
                ),
            );
            for (name, hint, join) in [
                ("anchor-cte-inline", "NOT MATERIALIZED", "JOIN"),
                ("anchor-cte-materialized", "MATERIALIZED", "JOIN"),
                ("anchor-cte-cross", "MATERIALIZED", "CROSS JOIN"),
                ("anchor-inline-cross", "NOT MATERIALIZED", "CROSS JOIN"),
            ] {
                let body = case
                    .select
                    .replace(&format!("FROM {} f", case.input), "FROM anchors f")
                    .replace(anchor_join, "")
                    .replace("fn.wpar", "f.qpar")
                    .replace("fn.aprev", "f.qhi");
                let body = if join == "CROSS JOIN" {
                    body.replace(" JOIN node c ON ", " CROSS JOIN node c ON ")
                } else {
                    body
                };
                add(
                    name,
                    format!(
                        "WITH anchors AS {hint} (SELECT f.*, fn.wpar AS qpar, fn.aprev AS qhi FROM {} f CROSS JOIN node fn ON fn.file = f.file AND fn.id = f.node) {body}",
                        case.input
                    ),
                );
            }
            let scalar = case
                .select
                .replace(anchor_join, "")
                .replace(" JOIN node c ON ", " CROSS JOIN node c ON ")
                .replace(
                    "fn.wpar",
                    "(SELECT wpar FROM node WHERE file = f.file AND id = f.node)",
                )
                .replace(
                    "fn.aprev",
                    "(SELECT aprev FROM node WHERE file = f.file AND id = f.node)",
                );
            add("scalar-anchor", scalar);
            let relation = "c.file = f.file AND c.parent = fn.wpar AND c.id <= fn.aprev";
            let candidate = case
                .select
                .replace(anchor_join, "")
                .replace(" JOIN node c ON ", " CROSS JOIN node c ON ");
            add("candidate-exists",candidate.replace(relation,"c.file = f.file AND EXISTS (SELECT 1 FROM node owner WHERE owner.file = f.file AND owner.id = f.node AND c.parent = owner.wpar AND c.id <= owner.aprev)"));
            add("candidate-in",candidate.replace(relation,"c.file = f.file AND c.id IN (SELECT neighbor.id FROM node owner CROSS JOIN node neighbor ON neighbor.file = owner.file AND neighbor.parent = owner.wpar AND neighbor.id <= owner.aprev WHERE owner.file = f.file AND owner.id = f.node)"));
            // A unary + prevents SQLite from choosing kind postings as the candidate access
            // path. All values are integer kind codes, so this preserves the predicate.
            add(
                "cross-parent-access",
                pinned.replace("c.kind IN (", "+c.kind IN ("),
            );
        }
        "capture-demand" => {
            let Some(key) = ATOM.captures(&case.select) else {
                return variants;
            };
            if key[1] != key[2] {
                return variants;
            }
            let key_expr = key.get(0).unwrap().as_str();
            let from = format!("FROM {} f", case.input);
            for (name, hint) in [
                ("key-cte-inline", "NOT MATERIALIZED"),
                ("key-cte-materialized", "MATERIALIZED"),
            ] {
                let body = case
                    .select
                    .replace(key_expr, "f.qkey")
                    .replace(&from, "FROM keys f");
                add(
                    name,
                    format!(
                        "WITH keys AS {hint} (SELECT f.*, {key_expr} AS qkey FROM {} f) {body}",
                        case.input
                    ),
                );
            }
            add("key-left-join", case.select.replace(key_expr,&format!("COALESCE(keynode.atom, f.{})",&key[1]))
                .replace(&from,&format!("{from} LEFT JOIN node keynode ON keynode.file = f.file AND keynode.id = f.{}",&key[1])));
            let anti = " WHERE NOT EXISTS (SELECT 1 FROM ";
            if let Some((project, _)) = case.select.split_once(anti) {
                let tail = case.select.split_once(anti).unwrap().1;
                let demand = tail.split_whitespace().next().unwrap();
                let distinct = project
                    .replace(key_expr, "f.qkey")
                    .replace(&from, "FROM keys f");
                let body = case
                    .select
                    .replace(key_expr, "f.qkey")
                    .replace(&from, "FROM unique_keys f");
                add(
                    "dedup-before-anti",
                    format!(
                        "WITH keys AS MATERIALIZED (SELECT f.*, {key_expr} AS qkey FROM {} f), unique_keys AS MATERIALIZED ({distinct}) {}",
                        case.input,
                        body.replace(
                            "SELECT DISTINCT f.file, f.node, f.qkey",
                            "SELECT f.file, f.node, f.qkey"
                        )
                    ),
                );
                // Only this exact three-column form is supported. EXCEPT treats NULL keys as
                // equal, like the emitter's IS predicate. Output order is checked separately.
                if project == format!("SELECT DISTINCT f.file, f.node, {key_expr} {from}") {
                    add(
                        "key-except",
                        format!("{project} EXCEPT SELECT file, node, c0 FROM {demand}"),
                    );
                }
            }
        }
        _ => {
            add(
                "terminal-inline",
                case.select
                    .replace("AS MATERIALIZED (", "AS NOT MATERIALIZED ("),
            );
            add(
                "terminal-default-cte",
                case.select.replace("AS MATERIALIZED (", "AS ("),
            );
            add(
                "frame-first-cross",
                case.select
                    .replace(" f JOIN p ON ", " f CROSS JOIN p ON ")
                    .replace(" f JOIN t ON ", " f CROSS JOIN t ON "),
            );
            for (name, hint) in [
                ("input-cte-materialized", "MATERIALIZED"),
                ("input-cte-inline", "NOT MATERIALIZED"),
            ] {
                let body = case
                    .select
                    .strip_prefix("WITH ")
                    .unwrap()
                    .replace(&format!("FROM {}", case.input), "FROM input_rows");
                add(
                    name,
                    format!(
                        "WITH input_rows AS {hint} (SELECT * FROM {}), {body}",
                        case.input
                    ),
                );
            }
        }
    }
    variants
}

#[test]
fn classification_excludes_control_sql_and_preserves_projected_captures() {
    assert!(classify("DELETE FROM f0_0").is_none());
    let case = classify("INSERT INTO f1_0 SELECT NULL, c.file, c.node, c.s0, c.pk, -c.node FROM (SELECT f.k AS pk, c.file AS file, c.id AS node, f.s0 FROM f0_0 f JOIN node fn ON fn.file = f.file AND fn.id = f.node JOIN node c ON c.file = f.file AND c.parent = fn.wpar AND c.id <= fn.aprev AND c.kind IN (215)) c").unwrap();
    for variant in variants(&case) {
        assert!(
            variant
                .sql
                .contains("SELECT NULL, c.file, c.node, c.s0, c.pk, -c.node")
        );
        assert!(!variant.sql.contains("INSERT INTO"));
        let insert = case.insert(&variant.sql);
        assert_eq!(INSERT.find_iter(&insert).count(), 1);
    }
    let select = "WITH keys AS MATERIALIZED (SELECT 'SELECT (' AS x) SELECT 'é',x FROM keys";
    assert_eq!(
        case.insert(select),
        "WITH keys AS MATERIALIZED (SELECT 'SELECT (' AS x) INSERT INTO f1_0 SELECT 'é',x FROM keys"
    );
}
