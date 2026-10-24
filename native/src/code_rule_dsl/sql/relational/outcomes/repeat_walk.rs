//! Adjacent identical searches can share a completed traversal and its stable cost.
//! The first call populates every reached utility answer; later calls only replay it.
use super::ordered::nested::{bridge, program};
use super::{Axis, Demand, Op, Plan, Sql};
use crate::code_rule_dsl::sql::relational::ir::Set;
use serde_json::json;

pub(super) use super::repeat_cost::Boolean;

fn signature(plan: &Plan) -> Option<(Vec<Set>, usize)> {
    if !plan.mentions.is_empty() || !plan.writes.is_empty() || plan.effects.cache_fills {
        return None;
    }
    let Op::Search(Axis::Child, true, body, None) = &plan.op else {
        return None;
    };
    fn prefix(plan: &Plan, predicates: &mut Vec<Set>, utility: &mut Option<usize>) -> bool {
        match &plan.op {
            Op::Shared(body) => prefix(body, predicates, utility),
            Op::Predicate(set) if utility.is_none() => {
                predicates.push(set.clone());
                true
            }
            Op::All(items) => items.iter().all(|p| prefix(p, predicates, utility)),
            Op::Charge(id, body) if utility.is_none() && body.reads_value() => {
                *utility = Some(*id);
                true
            }
            _ => false,
        }
    }
    let mut predicates = vec![];
    let mut utility = None;
    if !prefix(body, &mut predicates, &mut utility) {
        return None;
    }
    Some((predicates, utility?))
}

pub(super) fn length(items: &[Plan]) -> usize {
    let Some(key) = items.first().and_then(signature) else {
        return 1;
    };
    items
        .iter()
        .take_while(|p| signature(p).as_ref() == Some(&key))
        .count()
}

impl Sql {
    pub(super) fn repeat_step(
        &mut self,
        input: &Demand,
        child: &Plan,
        guard: &str,
        calls: usize,
        boolean: Boolean,
    ) -> Result<String, String> {
        debug_assert!(
            calls > 1 && self.generation.memo_clause.is_some() && signature(child).is_some()
        );
        let (query, profile) = program(
            child,
            input,
            false,
            guard,
            &child.writes,
            ("d.file", "d.node", "d.work"),
            self.generation.clone(),
        )?;
        let query = replay(&query, calls - 1, boolean);
        let name = self.relation(
            "correlated repeated traversal",
            format!(
                "SELECT d.*,r.c0 AS tested,r.c1 AS remaining FROM {} d {}",
                input.name,
                bridge(&query, 2)
            ),
            true,
            "input state,tested outcome,remaining allowance",
        );
        let item = self.profile.last_mut().unwrap();
        item["childPrograms"] = json!({"operand":profile});
        item["repeatTraversal"] = json!({"calls":calls,"compiledTraversals":1,"typedReturnColumns":2,"stopTruth":match boolean {Boolean::All=>0,Boolean::Any=>1},"proof":"adjacent identical capture-free descendant searches; pure prefix then one value-bearing utility; no stop or intervening effect","cost":"first measured cost replayed for remaining reached calls","overflow":"divide remaining allowance before multiplying cost"});
        Ok(name)
    }
}

fn replay(query: &str, repeats: usize, boolean: Boolean) -> String {
    let (truth, work) = boolean.replay("f.truth", "f.work", "(d.work-f.work)", repeats);
    format!(
        "WITH first AS MATERIALIZED ({query}) SELECT {truth} AS truth,{work} AS work FROM first f"
    )
}

#[test]
fn repeated_cost_arithmetic_stays_integer_at_allowance_limits() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    crate::code_rule_dsl::sql::relational::row::register(&db).unwrap();
    let query = format!(
        "SELECT r.c0,r.c1 FROM (SELECT ?1 AS work) d {}",
        bridge(
            &replay("SELECT ?2 AS truth,?3 AS work", 31, Boolean::All),
            2
        )
    );
    let mut statement = db.prepare(&query).unwrap();
    for (start, truth, left, expected) in [
        (64, 1, 62, (1, 0)),
        (63, 1, 61, (-2, 0)),
        (63, 2, 61, (-2, 0)),
        (i64::MAX, 1, i64::MAX / 2, (-2, 0)),
        (i64::MAX, 2, i64::MAX / 2, (-2, 0)),
        (i64::MAX, 0, i64::MAX / 2, (0, i64::MAX / 2)),
        (i64::MAX, -4, i64::MAX / 2, (-4, i64::MAX / 2)),
        (i64::MAX, 1, i64::MAX - 1, (1, i64::MAX - 32)),
        (i64::MAX, 1, i64::MAX, (1, i64::MAX)),
    ] {
        let result = statement
            .query_row([start, truth, left], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })
            .unwrap();
        assert_eq!(result, expected);
    }
}

#[test]
fn alternative_repeated_cost_arithmetic_stays_integer() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    crate::code_rule_dsl::sql::relational::row::register(&db).unwrap();
    let query = format!(
        "SELECT r.c0,r.c1 FROM (SELECT ?1 AS work) d {}",
        bridge(
            &replay("SELECT ?2 AS truth,?3 AS work", 31, Boolean::Any),
            2
        )
    );
    let mut statement = db.prepare(&query).unwrap();
    for (start, truth, left, expected) in [
        (64, 0, 62, (0, 0)),
        (63, 0, 61, (-2, 0)),
        (63, 2, 61, (-2, 0)),
        (63, 1, 61, (1, 61)),
        (i64::MAX, 0, i64::MAX / 2, (-2, 0)),
        (i64::MAX, 2, i64::MAX / 2, (-2, 0)),
        (i64::MAX, 1, i64::MAX / 2, (1, i64::MAX / 2)),
        (i64::MAX, -4, i64::MAX / 2, (-4, i64::MAX / 2)),
        (i64::MAX, 0, i64::MAX - 1, (0, i64::MAX - 32)),
        (i64::MAX, 2, i64::MAX, (2, i64::MAX)),
    ] {
        let result = statement
            .query_row([start, truth, left], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
            })
            .unwrap();
        assert_eq!(result, expected);
    }
}
