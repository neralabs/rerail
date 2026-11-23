//! A min or max bounded by its probe (decision 0511): lowered as one aggregate of the group's values with the bound
//! on its op, and folded natively with the bound as a filter, nothing made per probe. W's side of each check is the
//! plain search `ScopeTable::visible` does: the latest of a group's values before the read.
use rerail_blocks::vector::Vector;
use super::spec::hex;
use super::*;
use crate::code_rule_dsl::logic::front::AggOp;
use crate::code_rule_dsl::logic::program_image::{AggRef, Lit};

fn connection() -> Connection {
    let path = crate::code_rule_dsl::fixtures::temp();
    drop(crate::code_rule_dsl::fixtures::open(&path));
    let db = Connection::open(path).unwrap();
    crate::code_rule_dsl::logic::native::register(&db).unwrap();
    db
}

fn rel_of(rows: &[Vector<i64>], types: &[u8]) -> InRel {
    let mut r = Rel::new("x", types);
    for row in rows {
        r.push(row);
    }
    InRel::moved(r)
}

const SRC: &str = ".decl b(k: number, t: number)\n.decl p(i: number, k: number, r: number)\n.input b, p\n.decl o(i: number, u: number)\n.output o\n";

fn aggregates(src: &str) -> (Vec<AggOp>, usize) {
    let p = crate::code_rule_dsl::logic::rr::parse_version(src).unwrap_or_else(|e| panic!("{e}"));
    let head = |r: crate::code_rule_dsl::logic::program_image::RuleRef<'_>| p.rel_name(r.head().rel()).to_owned();
    let mut ops = Vec::new();
    for r in p.rules().filter(|&r| head(r) == "o") {
        for l in r.body() {
            if let Lit::Agg { op, .. } = l.view() {
                ops.push(match op {
                    AggRef::Count => AggOp::Count,
                    AggRef::Min => AggOp::Min,
                    AggRef::Max => AggOp::Max,
                    AggRef::Sum => AggOp::Sum,
                    AggRef::Product => AggOp::Product,
                    AggRef::Join { .. } => AggOp::Join { by: Vec::new(), sep: String::new() },
                    AggRef::Bounded { max, bound, strict } => AggOp::Bounded { max, bound: p.text(r.var(bound)).to_owned(), strict },
                });
            }
        }
    }
    (ops, p.rules().filter(|&r| head(r).starts_with("rr_ctx")).count())
}

/// Each side of a bound is recognized, either way round, and none needs a context of the probes.
#[test]
fn a_min_or_max_below_a_probes_bound_lowers_to_a_bounded_aggregate() {
    for (body, max, strict) in [("t < r", true, true), ("t <= r", true, false), ("r > t", true, true), ("t > r", false, true), ("r <= t", false, false)] {
        let op = if max { "max" } else { "min" };
        let (ops, contexts) = aggregates(&format!("{SRC}o(i, u) :- p(i, k, r), u = {op} t : {{ b(k, t), {body} }}.\n"));
        assert_eq!(ops, vec![AggOp::Bounded { max, bound: "r".into(), strict }], "{op} over {body}");
        assert_eq!(contexts, 0, "{op} over {body}: no context of the probes");
    }
}

/// A bound the body reads elsewhere, a bound on the wrong side, and two bounds are lowered as before: a context of
/// the probes and a plain min or max.
#[test]
fn a_bound_the_body_reads_or_the_wrong_side_lowers_as_before() {
    for (op, body) in [("max", "b(k, t), t < r, r != k"), ("max", "b(k, t), t > r"), ("min", "b(k, t), t < r"), ("max", "b(k, t), t < r, t < i")] {
        let (ops, contexts) = aggregates(&format!("{SRC}o(i, u) :- p(i, k, r), u = {op} t : {{ {body} }}.\n"));
        assert!(ops.iter().all(|o| matches!(o, AggOp::Min | AggOp::Max)), "{op} over {body}: {ops:?}");
        assert!(contexts > 0, "{op} over {body}: a context of the probes");
    }
}

/// The bounded step's value is the plain search's on 10⁴ values in 100 groups (ties, and groups with no value) and
/// 10⁴ probes, each side of the bound, strict or not; a probe whose group has no value on its side has no row.
#[test]
fn the_bounded_step_finds_the_plain_searchs_value_for_every_probe() {
    let db = connection();
    // Group k holds values when k % 7 != 0, each value twice in some groups (ties).
    let values: Vec<Vector<i64>> = (0..10_000i64).filter(|i| (i % 100) % 7 != 0).map(|i| rerail_blocks::vector![i % 100, (i * 7919) % 1000 / (1 + i % 3)]).collect();
    let probes: Vec<Vector<i64>> = (0..10_000i64).map(|i| rerail_blocks::vector![i, (i * 31) % 100, (i * 104_729) % 1100 - 50]).collect();
    adopt(&db, "bnd_t", Arc::new(rel_of(&values, b"ii")));
    adopt(&db, "bnd_p", Arc::new(rel_of(&probes, b"iii")));
    let r = |t: &str| format!("r:{}", hex(t));
    for (step, max, strict) in [("maxlt", true, true), ("maxle", true, false), ("mingt", false, true), ("minge", false, false)] {
        let name = format!("bnd_{step}");
        let body = format!("pos {} v0 v1 v2\n{step} {} v1 _ 1 v2 v3\nhead v0 v3", r("bnd_p"), r("bnd_t"));
        open(&db, &format!("name {name}\nrel {name} ii empty\nnowrite\nseed 0 {}", hex(&body))).unwrap();
        run(&db, &name, 0).unwrap();
        close(&db, &name).unwrap();
        let hit = held(&db, &name).expect("the derived relation");
        let mut got: Vec<(i64, i64)> = (0..hit.len()).filter(|&i| !is_dead(&hit.dead, i as u32)).map(|i| (hit.at(i).at(0), hit.at(i).at(1))).collect();
        got.sort_unstable();
        let mut want: Vec<(i64, i64)> = probes
            .iter()
            .filter_map(|p| {
                let side = |t: i64| match (max, strict) {
                    (true, true) => t < p[2],
                    (true, false) => t <= p[2],
                    (false, true) => t > p[2],
                    (false, false) => t >= p[2],
                };
                let group = values.iter().filter(|v| v[0] == p[1] && side(v[1])).map(|v| v[1]);
                let best = if max { group.max() } else { group.min() };
                best.map(|b| (p[0], b))
            })
            .collect();
        want.sort_unstable();
        assert!(want.len() > 5_000 && want.len() < 10_000, "{step}: probes with and without a value ({})", want.len());
        assert_eq!(got.len(), want.len(), "{step}: one row a probe with a value");
        assert_eq!(got, want, "{step}");
    }
}
