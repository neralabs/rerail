//! Phase-one generated shape. The relational plan is constructed here by hand;
//! lowering arbitrary catalog grammars to this plan is a separate compiler gate.
use super::super::sql_shape::{Binding, Column, Emission, Expr, Join, Query, Select, Shape};

fn column(name: &str) -> Column {
    let (alias, field) = name.split_once('.').unwrap();
    Column::new(alias, field)
}
fn field(name: &str) -> Expr {
    Expr::Column(column(name))
}
fn join(relation: &str, alias: &str, pairs: &[(&str, &str)]) -> Join {
    Join {
        source: Binding::new(relation, alias),
        on: pairs.iter().map(|(a, b)| (column(a), column(b))).collect(),
    }
}
fn select(relation: &str, alias: &str, output: &[&str]) -> Select {
    Select {
        from: Binding::new(relation, alias),
        joins: Vec::new(),
        projection: output.iter().map(|name| field(name)).collect(),
        predicates: Vec::new(),
    }
}
fn emit(mut shape: Shape, output: &[&str], branches: Vec<Select>) -> Emission {
    shape
        .define(
            "result",
            Query {
                columns: output.iter().map(|c| (*c).to_owned()).collect(),
                branches,
            },
        )
        .unwrap();
    shape.emit("result").unwrap()
}
pub(super) fn queries() -> [Emission; 2] {
    let mut shape = Shape::default();
    for (name, columns) in [
        ("call_sites", &["doc", "report", "call", "ref"][..]),
        ("qualified", &["doc", "node", "symbol"][..]),
        ("wanted_symbol", &["label", "symbol", "requirement"][..]),
        ("receiver", &["doc", "callee", "object"][..]),
    ] {
        shape.source(name, columns).unwrap();
    }
    let mut direct = select(
        "wanted_symbol",
        "w",
        &["w.label", "s.doc", "s.report", "s.call", "w.requirement"],
    );
    direct.joins = vec![
        join("qualified", "r", &[("r.symbol", "w.symbol")]),
        join(
            "call_sites",
            "s",
            &[("s.doc", "r.doc"), ("s.ref", "r.node")],
        ),
    ];
    let mut receiver = select("call_sites", "s", &[]);
    receiver.joins = vec![join(
        "receiver",
        "x",
        &[("x.doc", "s.doc"), ("x.callee", "s.report")],
    )];
    receiver.projection = vec![
        Expr::Integer(3),
        field("s.doc"),
        field("s.report"),
        field("s.call"),
        Expr::Integer(2),
    ];
    let targets = emit(
        shape,
        &["label", "doc", "report", "call", "requirement"],
        vec![direct, receiver],
    );

    let mut shape = Shape::default();
    for (name, columns) in [
        (
            "candidates",
            &["label", "doc", "report", "call", "requirement"][..],
        ),
        ("qualified_values", &["doc", "node", "symbol"][..]),
        ("value_origin", &["doc", "owner", "reading", "value"][..]),
        ("call_slot", &["doc", "call", "value"][..]),
        ("receiver", &["doc", "callee", "object"][..]),
        ("constructed", &["doc", "value", "callee"][..]),
    ] {
        shape.source(name, columns).unwrap();
    }
    let branch = |requirement| {
        let mut branch = select("candidates", "c", &["c.label", "c.doc", "c.report"]);
        branch
            .predicates
            .push((field("c.requirement"), Expr::Integer(requirement)));
        branch
    };
    let direct = branch(0);
    let slot = || {
        join(
            "call_slot",
            "a",
            &[("a.doc", "c.doc"), ("a.call", "c.call")],
        )
    };
    let mut absent = branch(1);
    absent.joins = vec![slot()];
    absent
        .predicates
        .push((field("a.value"), Expr::Integer(-1)));
    let mut mode = branch(1);
    mode.joins = vec![
        slot(),
        join(
            "value_origin",
            "o",
            &[("o.doc", "a.doc"), ("o.owner", "a.value")],
        ),
        join(
            "qualified_values",
            "q",
            &[("q.doc", "o.doc"), ("q.node", "o.value")],
        ),
    ];
    mode.predicates.extend([
        (field("o.reading"), Expr::Integer(1)),
        (field("q.symbol"), Expr::Parameter(1)),
    ]);
    let mut constructed = branch(2);
    constructed.joins = vec![
        join(
            "receiver",
            "x",
            &[("x.doc", "c.doc"), ("x.callee", "c.report")],
        ),
        join(
            "value_origin",
            "o",
            &[("o.doc", "x.doc"), ("o.owner", "x.object")],
        ),
        join(
            "constructed",
            "k",
            &[("k.doc", "o.doc"), ("k.value", "o.value")],
        ),
        join(
            "qualified_values",
            "q",
            &[("q.doc", "k.doc"), ("q.node", "k.callee")],
        ),
    ];
    constructed.predicates.extend([
        (field("o.reading"), Expr::Integer(0)),
        (field("q.symbol"), Expr::Parameter(2)),
    ]);
    [
        targets,
        emit(
            shape,
            &["label", "doc", "report"],
            vec![direct, absent, mode, constructed],
        ),
    ]
}
