//! Recognize an ordered positive subtree utility and lower it to typed witness relations.
//! The proof requires a single fresh node capture, a total base test, and unrestricted child
//! recursion. For this grammar, first-match order equals the smallest matching preorder ID.
use crate::code_rule_dsl::sql::{
    plan::{Op, SlotKind, Statement, Stop, Walk},
    terminals::Terminal,
};
#[derive(Clone, Copy, Debug)]
pub(super) enum Shape {
    Closure,
    Interval,
    Probe,
    Segments,
}
fn unwrap(op: &Op) -> &Op {
    match op {
        Op::Unknowable { op, .. } => unwrap(op),
        _ => op,
    }
}
fn all(op: &Op) -> Option<Vec<&Op>> {
    match unwrap(op) {
        Op::All(v) => Some(v.iter().collect()),
        Op::AllOpen(v) => Some(v.iter().map(|t| &t.op).collect()),
        _ => None,
    }
}
fn required(op: &Op) -> Option<std::collections::BTreeSet<u16>> {
    if let Op::Kinds(kinds) = unwrap(op) {
        return Some(kinds.iter().copied().collect());
    }
    let mut result: Option<std::collections::BTreeSet<u16>> = None;
    if let Some(items) = all(op) {
        for item in items {
            if let Some(kinds) = required(item) {
                result = Some(result.map_or(kinds.clone(), |old| {
                    old.intersection(&kinds).copied().collect()
                }));
            }
        }
    }
    result
}
fn predicate(
    op: &Op,
    terminals: &[Terminal],
    alias: &str,
    slot: usize,
    capture: &mut bool,
    domain: &str,
) -> Result<String, String> {
    let op = unwrap(op);
    if let Some(items) = all(op) {
        return Ok(format!(
            "({})",
            items
                .into_iter()
                .map(|o| predicate(o, terminals, alias, slot, capture, domain))
                .collect::<Result<Vec<_>, _>>()?
                .join(" AND ")
        ));
    }
    Ok(match op {
        Op::Kinds(k) => {
            if k.is_empty() {
                "0".into()
            } else {
                format!(
                    "{alias}.kind IN({})",
                    k.iter().map(u16::to_string).collect::<Vec<_>>().join(",")
                )
            }
        }
        Op::Regex(term) => format!(
            "({alias}.file*4294967296+{alias}.id) IN(SELECT ek FROM text_match WHERE term={term} AND ek IN(SELECT file*4294967296+id FROM node{}))",
            if let Terminal::Regex(_) = &terminals[*term] {
                domain
            } else {
                return Err("positive closure: regex contract".into());
            }
        ),
        Op::Pattern { term, vars, .. }
            if vars == &[slot]
                && matches!(&terminals[*term],Terminal::Pattern(p) if p.root_capture && p.kinds.is_none()) =>
        {
            *capture = true;
            let Terminal::Pattern(p) = &terminals[*term] else {
                unreachable!()
            };
            format!(
                "{alias}.named AND spike_file_dialect({alias}.file)='{}'",
                p.dialect
            )
        }
        Op::Never => "0".into(),
        Op::Not(sub) => {
            let mut inner_capture = false;
            let p = predicate(sub, terminals, alias, slot, &mut inner_capture, domain)?;
            if inner_capture {
                return Err("positive closure: negated capture".into());
            }
            format!("NOT({p})")
        }
        _ => return Err("positive closure: non-total or capture-dependent base".into()),
    })
}
fn child_recur(op: &Op) -> bool {
    matches!(unwrap(op),Op::Relation {walk:Walk::Has(Stop::Neighbor),sub,..}|Op::Search {walk:Walk::Has(Stop::Neighbor),sub,..} if matches!(unwrap(sub),Op::Recur {unit:0,slots:None,context:None}))
}
pub(super) fn compile(
    statement: &Statement,
    terminals: &[Terminal],
    shape: Shape,
) -> Result<String, String> {
    if statement.units.len() != 1
        || statement.units[0].following.is_some()
        || statement.slots.iter().filter(|s| s.scope == 0).count() != 1
        || statement
            .slots
            .first()
            .is_none_or(|s| s.kind != SlotKind::Node || s.scope != 0)
        || statement.slots.iter().skip(1).any(|s| s.name != "\0origin")
    {
        return Err("positive closure: single fresh node capture and one unit required".into());
    }
    let mut root_guard = "1".to_owned();
    let root = unwrap(&statement.op);
    if !matches!(
        root,
        Op::Recur {
            unit: 0,
            slots: None,
            context: None
        }
    ) {
        let roots = all(root).ok_or("positive closure: root contract")?;
        let mut calls = 0;
        let mut predicates = Vec::new();
        for r in roots {
            if matches!(
                unwrap(r),
                Op::Recur {
                    unit: 0,
                    slots: None,
                    context: None
                }
            ) {
                calls += 1;
            } else {
                let mut capture = false;
                predicates.push(predicate(r, terminals, "n", 0, &mut capture, "")?);
                if capture {
                    return Err("positive closure: bound root environment".into());
                }
            }
        }
        if calls != 1 {
            return Err("positive closure: exactly one root reference required".into());
        }
        if !predicates.is_empty() {
            root_guard = predicates.join(" AND ");
        }
    }
    let branches: Vec<_> = match unwrap(&statement.units[0].body) {
        Op::Any(v) => v.iter().collect(),
        Op::AnyOpen(v) => v.iter().map(|v| &v.0).collect(),
        _ => return Err("positive closure: base-first alternative required".into()),
    };
    if branches.len() != 2 || !child_recur(branches[1]) {
        return Err("positive closure: unrestricted child recurrence required".into());
    }
    let domain = required(branches[0]).map_or(String::new(), |k| {
        if k.is_empty() {
            " WHERE 0".into()
        } else {
            format!(
                " WHERE kind IN({})",
                k.iter().map(u16::to_string).collect::<Vec<_>>().join(",")
            )
        }
    });
    let mut capture = false;
    let base = predicate(branches[0], terminals, "n", 0, &mut capture, &domain)?;
    if !capture {
        return Err("positive closure: base must bind witness".into());
    }
    let kinds = statement.kinds.as_ref().map_or(String::new(), |k| {
        if k.is_empty() {
            " AND 0".into()
        } else {
            format!(
                " AND n.kind IN({})",
                k.iter().map(u16::to_string).collect::<Vec<_>>().join(",")
            )
        }
    });
    let common = format!(
        "seed(file,node) AS MATERIALIZED(SELECT n.file,n.id FROM node n WHERE {base}), roots(file,node,last) AS MATERIALIZED(SELECT n.file,n.id,n.\"last\" FROM node n WHERE {root_guard}{kinds})"
    );
    let (extra,answer)=match shape {
        Shape::Closure=>(", reach(file,node,witness) AS(SELECT file,node,node FROM seed UNION ALL SELECT a.file,n.parent,a.witness FROM reach a JOIN node n ON n.file=a.file AND n.id=a.node WHERE n.parent IS NOT NULL), grouped(file,node,witness) AS MATERIALIZED(SELECT file,node,MIN(witness) FROM reach GROUP BY file,node)".to_owned(),"SELECT r.file,r.node,g.witness FROM roots r LEFT JOIN grouped g ON g.file=r.file AND g.node=r.node".to_owned()),
        Shape::Interval=>(String::new(),"SELECT r.file,r.node,MIN(s.node) AS witness FROM roots r LEFT JOIN seed s ON s.file=r.file AND s.node>=r.node AND s.node<=r.last GROUP BY r.file,r.node".into()),
        Shape::Segments=>(format!(", segments(file,witness,previous) AS MATERIALIZED(SELECT file,node,LAG(node,1,-1) OVER(PARTITION BY file ORDER BY node) FROM seed), accepted(file,node,witness) AS MATERIALIZED(SELECT n.file,n.id,s.witness FROM segments s CROSS JOIN node n ON n.file=s.file AND n.id>s.previous AND n.id<=s.witness WHERE s.witness<=n.\"last\" AND ({root_guard}){kinds})"),"SELECT r.file,r.node,a.witness FROM roots r LEFT JOIN accepted a ON a.file=r.file AND a.node=r.node".into()),
        Shape::Probe=>(String::new(),"SELECT r.file,r.node,(SELECT s.node FROM seed s WHERE s.file=r.file AND s.node>=r.node AND s.node<=r.last ORDER BY s.node LIMIT 1) AS witness FROM roots r".into()),
    };
    Ok(format!(
        "WITH RECURSIVE {common}{extra}, chosen AS MATERIALIZED({answer}) SELECT 0 AS clause,file,node,json_object('result',witness IS NOT NULL,'fault',0,'env',json(CASE WHEN witness IS NULL THEN '{{}}' ELSE json_object('0',witness) END),'steps',0) AS captures FROM chosen ORDER BY file,node"
    ))
}
