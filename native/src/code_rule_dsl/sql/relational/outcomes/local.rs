//! Capture-free truth tests for indexed existence probes. Unknown is neither true nor false.
use super::{Op, Plan};

enum Test {
    Constant(bool),
    Sql(String),
}
impl Test {
    fn sql(self) -> String {
        match self {
            Self::Constant(value) => usize::from(value).to_string(),
            Self::Sql(sql) => sql,
        }
    }
    fn not(self) -> Self {
        match self {
            Self::Constant(value) => Self::Constant(!value),
            Self::Sql(sql) => Self::Sql(format!("NOT ({sql})")),
        }
    }
    fn combine(tests: Vec<Self>, all: bool) -> Self {
        let mut terms = Vec::new();
        for test in tests {
            match test {
                Self::Constant(value) if value != all => return Self::Constant(!all),
                Self::Constant(_) => {}
                Self::Sql(sql) => terms.push(format!("({sql})")),
            }
        }
        if terms.is_empty() {
            Self::Constant(all)
        } else {
            Self::Sql(terms.join(if all { " AND " } else { " OR " }))
        }
    }
}

fn tests(plan: &Plan) -> Result<Option<(Test, Test)>, String> {
    Ok(Some(match &plan.op {
        Op::Predicate(set) => {
            let sql = super::super::emit::condition(set, "n")?;
            (Test::Sql(sql.clone()), Test::Sql(sql).not())
        }
        Op::Unknown(term) => (
            Test::Sql(format!("NOT node_regex({term},n.file,n.id)")),
            Test::Constant(false),
        ),
        Op::Not(child) => {
            let Some((yes, no)) = tests(child)? else {
                return Ok(None);
            };
            (no, yes)
        }
        Op::All(children) | Op::Any(children) => {
            let mut yes = Vec::new();
            let mut no = Vec::new();
            for child in children {
                let Some((a, b)) = tests(child)? else {
                    return Ok(None);
                };
                yes.push(a);
                no.push(b);
            }
            let all = matches!(plan.op, Op::All(_));
            (Test::combine(yes, all), Test::combine(no, !all))
        }
        _ => return Ok(None),
    }))
}

pub(super) fn predicates(plan: &Plan) -> Result<Option<(String, String)>, String> {
    if !plan.mentions.is_empty() || !plan.writes.is_empty() {
        return Ok(None);
    }
    Ok(tests(plan)?.map(|(yes, no)| {
        let holds = yes.sql();
        let unknown = format!("NOT (({holds}) OR ({}))", no.sql());
        (holds, unknown)
    }))
}
