//! Integer semantic-cost replay shared by traversal and direct utility groups.
#[derive(Clone, Copy)]
pub(super) enum Boolean {
    All,
    Any,
}
impl Boolean {
    pub(super) fn replay(
        self,
        truth: &str,
        work: &str,
        cost: &str,
        repeats: usize,
    ) -> (String, String) {
        assert!(repeats > 0);
        let (continues, stops) = match self {
            Self::All => (format!("{truth}>0"), format!("{truth}<=0")),
            Self::Any => (format!("{truth} IN (0,2)"), format!("{truth} NOT IN (0,2)")),
        };
        // Division proves the multiplication fits the remaining signed allowance.
        let exhausted = format!("{cost}>{work}/{repeats}");
        (
            format!("CASE WHEN {continues} AND ({exhausted}) THEN -2 ELSE {truth} END"),
            format!(
                "CASE WHEN {stops} THEN {work} WHEN {exhausted} THEN 0 ELSE {work}-{cost}*{repeats} END"
            ),
        )
    }
}

#[test]
fn direct_replay_keeps_null_failures_and_integer_limits() {
    let db = rusqlite::Connection::open_in_memory().unwrap();
    for boolean in [Boolean::All, Boolean::Any] {
        let (truth, remaining) = boolean.replay("d.truth", "d.work", "d.cost", 31);
        let query = format!(
            "SELECT {truth},CASE WHEN d.truth IS NULL THEN d.work ELSE {remaining} END FROM (SELECT ?1 AS truth,?2 AS work,?3 AS cost) d"
        );
        let mut stmt = db.prepare(&query).unwrap();
        let continuing = if matches!(boolean, Boolean::All) {
            1
        } else {
            0
        };
        let stopping = 1 - continuing;
        for (answer, left, cost, expected) in [
            (Some(continuing), 62, Some(2), (Some(continuing), 0)),
            (Some(continuing), 61, Some(2), (Some(-2), 0)),
            (Some(2), 61, Some(2), (Some(-2), 0)),
            (
                Some(stopping),
                i64::MAX,
                Some(i64::MAX),
                (Some(stopping), i64::MAX),
            ),
            (Some(-4), 17, None, (Some(-4), 17)),
            (Some(-2), 0, None, (Some(-2), 0)),
            (None, 17, None, (None, 17)),
            (Some(continuing), i64::MAX, Some(i64::MAX), (Some(-2), 0)),
            (
                Some(continuing),
                i64::MAX,
                Some(1),
                (Some(continuing), i64::MAX - 31),
            ),
            (Some(2), i64::MAX, Some(0), (Some(2), i64::MAX)),
        ] {
            let result = stmt
                .query_row(rusqlite::params![answer, left, cost], |row| {
                    Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, i64>(1)?))
                })
                .unwrap();
            assert_eq!(result, expected);
        }
    }
}
