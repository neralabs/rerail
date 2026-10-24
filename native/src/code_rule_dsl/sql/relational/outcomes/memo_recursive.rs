//! Reached recursive utility calls with postorder memo publication.
//! One correlated schedule owns one numeric continuation chain. Capture inputs
//! stay in its outer demand; only successful exports cross return boundaries.
use super::super::{
    Demand, Output, Sql, fields,
    ordered::nested::{bridge, program},
    value,
};
use super::RecursiveSearch;
use crate::code_rule_dsl::sql::relational::emit::condition;
use crate::code_rule_dsl::sql::relational::outcomes::lower::recursive_search_lower::Arm;
use crate::code_rule_dsl::sql::relational::outcomes::{Axis, Op};
use serde_json::json;

impl Sql {
    pub(in crate::code_rule_dsl::sql::relational::outcomes) fn memo_recursive(
        &mut self,
        id: usize,
        component: &RecursiveSearch,
        input: &Demand,
    ) -> Result<Output, String> {
        let exports = &component.base.writes;
        let alias = format!("recursive_entry_{id}");
        let layout = self.generation.memo_layout.clone();
        let contract = layout
            .as_ref()
            .and_then(|l| l.contracts.get(&(2_000_000 + id)));
        let identity = contract.map_or(2_000_000 + id, |c| c.identity);
        let column = |s| layout.as_ref().map_or(s, |l| l.column(s));
        let mut keys = input.slots.clone();
        if let Some(layout) = &layout {
            keys.retain(|s| layout.capture(*s, contract.and_then(|c| c.scope.as_ref())));
        }
        keys.extend(&self.generation.following);
        let keys = keys
            .iter()
            .map(|s| {
                let raw = value(input, "d", *s);
                let key = layout
                    .as_ref()
                    .map_or_else(|| raw.clone(), |l| l.normalize_key(*s, raw.clone()));
                format!(" AND m.k{} IS {key}", column(*s))
            })
            .collect::<String>();
        let clause = self
            .generation
            .memo_clause
            .ok_or("recursive memo needs clause scope")?;
        let identity = format!(
            "m.clause IS {clause} AND m.utility IS {identity} AND m.file IS d.file AND m.node IS s.node"
        );
        let bypass = component.bypass || self.generation.memo_bypass;
        let hit_exports = exports
            .iter()
            .map(|s| {
                let stored=format!("m.o{}",column(*s));
                let replay=if layout.as_ref().is_some_and(|layout|layout.span_capable(*s)){format!("CASE WHEN m.truth IS NOT NULL THEN relation_capture_replay(s.file,{},{stored}) END",value(input,"d",*s))}else{stored};
                format!(",{replay} AS e{s}")
            })
            .collect::<String>();
        let seed_exports = fields(exports, |_| "NULL".into());
        let carry_exports = exports
            .iter()
            .map(|s| format!(",s.c{s}"))
            .collect::<String>();
        let null_exports = exports.iter().map(|_| ",NULL").collect::<String>();
        let hit_return = exports
            .iter()
            .map(|s| format!(",s.e{s}"))
            .collect::<String>();
        let supplied = exports
            .iter()
            .map(|s| {
                format!(
                    " AND m.v{} IS CASE WHEN s.truth=1 THEN s.c{s} END",
                    column(*s)
                )
            })
            .collect::<String>();
        let strict = !exports.is_empty();
        let continues = if strict {
            "s.truth=0"
        } else {
            "s.truth IN (0,2)"
        };
        let width = 11 + exports.len();
        if width > 130 {
            return Err("recursive memo exceeds typed continuation return width".into());
        }
        let mut generation = self.generation.clone();
        // Both the outer result and transition tuple remain active while the
        // reached base executes. Budget the actual simultaneous typed cells.
        generation.return_cells = generation
            .return_cells
            .checked_add(width + 2 + exports.len())
            .filter(|cells| *cells <= 128)
            .ok_or("recursive memo return payload exceeds 128 active cells")?;
        generation
            .return_peak
            .set(generation.return_peak.get().max(generation.return_cells));
        generation.dynamic_depth = Some(format!("{alias}.level+1"));
        generation.utility_depth = 0;
        let base_plans = match &component.base.op {
            Op::Any(arms) if component.ordered_bases => arms.iter().collect::<Vec<_>>(),
            _ => vec![&component.base],
        };
        let mut bases = Vec::new();
        let mut profiles = Vec::new();
        for (index, arm) in component.arms.iter().enumerate() {
            let Arm::Base(base_index) = arm else { continue };
            let arm = base_plans[*base_index];
            let phase = 10 + index;
            let (query, profile) = program(
                arm,
                input,
                false,
                &format!("{alias}.phase={phase}"),
                &arm.writes,
                ("d.file", &format!("{alias}.node"), &format!("{alias}.work")),
                generation.clone(),
            )?;
            bases.push((phase, *base_index, query));
            profiles.push(profile);
        }
        let first_phase = 10;
        let next_phase = |phase: usize| {
            if phase + 1 < 10 + component.arms.len() {
                phase + 1
            } else {
                3
            }
        };
        let depth = self.generation.depth_expression();
        // Every side-effecting transition starts from a materialized reached row.
        // In particular a recursive UNION arm cannot open/close an unrelated frame.
        let state_columns = [
            "node",
            "level",
            "phase",
            "parent",
            "work",
            "start",
            "fill",
            "token",
            "truth",
            "uncertain",
            "cursor",
        ]
        .into_iter()
        .map(str::to_string)
        .chain(exports.iter().map(|s| format!("c{s}")))
        .collect::<Vec<_>>();
        let seed_columns = state_columns
            .iter()
            .map(|c| format!("{alias}.{c} AS {c}"))
            .collect::<Vec<_>>()
            .join(",");
        let seed = |phase| format!("SELECT {seed_columns} WHERE {alias}.phase={phase}");
        let enter = format!(
            r#"WITH s AS MATERIALIZED ({}),
lookup AS MATERIALIZED (SELECT s.*,m.truth AS hit,m.cost,m.peak{hit_exports} FROM s LEFT JOIN relation_memo m ON m.mode IS CASE WHEN {} THEN 0 END AND {identity}{keys} AND m.peak<=256-s.level),
opened AS MATERIALIZED (SELECT s.*,CASE WHEN s.hit IS NULL AND s.level<256 THEN relation_peak_open(s.level) END AS opened_token FROM lookup s),
noted AS MATERIALIZED (SELECT s.*,CASE WHEN s.hit IS NOT NULL THEN relation_peak_note(s.level+s.peak) WHEN s.opened_token IS NOT NULL THEN relation_peak_note(s.level+1) ELSE 1 END AS ready FROM opened s)
SELECT s.node,s.level,CASE WHEN s.hit IS NOT NULL THEN 4 WHEN s.level>=256 OR s.work=0 THEN 3 ELSE {first_phase} END,s.parent,
CASE WHEN s.hit IS NOT NULL THEN MAX(0,s.work-s.cost) WHEN s.level<256 AND s.work>0 THEN s.work-1 ELSE s.work END,
s.work,relation_cache_fill_steps(),s.opened_token,
CASE WHEN s.hit IS NOT NULL THEN CASE WHEN s.work<s.cost THEN -2 ELSE s.hit END WHEN s.level>=256 THEN -1 WHEN s.work=0 THEN -2 ELSE 0 END,0,0{hit_return} FROM noted s WHERE s.ready=1"#,
            seed(0),
            if bypass { "0" } else { "1" }
        );
        let mut base_steps = Vec::new();
        for (phase, base_index, query) in &bases {
            let arm = base_plans[*base_index];
            let next = next_phase(*phase);
            let can_continue = if arm.writes.is_empty() {
                "r.c0 IN (0,2)"
            } else {
                "r.c0=0"
            };
            let returned = exports
                .iter()
                .map(|s| {
                    arm.writes.iter().position(|a| a == s).map_or_else(
                        || format!(",{}", value(input, "d", *s)),
                        |i| format!(",r.c{}", i + 2),
                    )
                })
                .collect::<String>();
            let step = format!(
                "WITH s AS MATERIALIZED ({}) SELECT s.node,s.level,CASE WHEN {can_continue} THEN {next} ELSE 3 END,s.parent,r.c1,s.start,s.fill,s.token,CASE WHEN r.c0=0 AND s.uncertain THEN 2 ELSE r.c0 END,s.uncertain OR r.c0=2,0{returned} FROM s {}",
                seed(*phase),
                bridge(query, 2 + arm.writes.len())
            );
            base_steps.push((*phase, step));
        }
        let mut child_steps = Vec::new();
        for (index, arm) in component.arms.iter().enumerate() {
            let Arm::Search {
                axis,
                guard,
                child_guard,
                child_kinds,
                field,
            } = arm
            else {
                continue;
            };
            let phase = 10 + index;
            let exhausted_phase = next_phase(phase);
            let guard = condition(guard, "n")?;
            let mut child_filter = child_kinds.as_ref().map_or("1".into(), |kinds| {
                format!(
                    "child.kind IN ({})",
                    kinds
                        .iter()
                        .map(u16::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                )
            });
            if let Some(child_guard) = child_guard {
                child_filter = format!(
                    "({child_filter}) AND ({})",
                    condition(child_guard, "child")?
                );
            }
            let candidate_domain = match axis {
                Axis::Child => "child.parent=s.node AND child.id>s.cursor AND child.id<=n.\"last\"",
                Axis::Parent => "s.cursor=0 AND child.id=n.parent",
                Axis::Previous => "s.cursor=0 AND child.id=n.prev",
                Axis::Next => "s.cursor=0 AND child.id=n.next",
            };
            if let Some(field) = field {
                let target = if matches!(axis, Axis::Child) {
                    "child"
                } else {
                    "n"
                };
                child_filter = format!(
                    "({child_filter}) AND s.cursor=0 AND {target}.field={field} AND {target}.ffirst=1"
                );
            }
            let multiple = matches!(axis, Axis::Child) && field.is_none();
            let saved_cursor = if multiple { "chosen.\"last\"" } else { "1" };
            let chosen = if multiple {
                "LEFT JOIN node chosen ON chosen.file=d.file AND chosen.id=s.child"
            } else {
                ""
            };
            let child_step = format!(
                r#"WITH reached AS MATERIALIZED ({}),
s AS MATERIALIZED (SELECT s.*,(SELECT child.id FROM node child WHERE child.file=d.file AND ({candidate_domain}) AND ({child_filter}) AND ({guard}) ORDER BY child.id LIMIT 1) AS child FROM reached s CROSS JOIN node n ON n.file=d.file AND n.id=s.node)
SELECT COALESCE(s.child,s.node),CASE WHEN s.child IS NULL THEN s.level ELSE s.level+1 END,CASE WHEN s.child IS NULL THEN {exhausted_phase} ELSE 0 END,COALESCE(p.handle,s.parent),s.work,s.start,s.fill,CASE WHEN s.child IS NULL THEN s.token END,CASE WHEN s.uncertain THEN 2 ELSE 0 END,s.uncertain,0{null_exports}
FROM s {chosen} LEFT JOIN relation_continuation p ON p.mode IS CASE WHEN s.child IS NOT NULL THEN 1 END AND p.parent_input IS s.parent AND p.v0 IS s.node AND p.v1 IS s.level AND p.v2 IS s.start AND p.v3 IS s.fill AND p.v4 IS s.token AND p.v5 IS s.uncertain AND p.v6 IS {saved_cursor} AND p.v7 IS {phase}"#,
                seed(phase)
            );
            child_steps.push((phase, child_step));
        }
        let close_step = format!(
            r#"WITH reached AS MATERIALIZED ({}),
s AS MATERIALIZED (SELECT s.*,CASE WHEN s.token IS NOT NULL THEN relation_peak_close(s.token) END AS measured FROM reached s)
SELECT s.node,s.level,4,s.parent,s.work,s.start,s.fill,NULL,COALESCE(m.truth,s.truth),s.uncertain,s.cursor{carry_exports} FROM s LEFT JOIN relation_memo m ON m.mode IS CASE WHEN {} AND s.truth>=0 AND s.token IS NOT NULL THEN 1 END AND {identity}{keys} AND m.vtruth IS s.truth AND m.vcost IS MAX(0,s.start-s.work-relation_cache_fill_delta(s.fill)) AND m.vpeak IS s.measured{supplied} WHERE COALESCE(s.measured,0)>=0"#,
            seed(3),
            if bypass { "0" } else { "1" }
        );
        let return_step = format!(
            r#"WITH s AS MATERIALIZED ({})
SELECT COALESCE(p.c0,s.node),COALESCE(p.c1,s.level),CASE WHEN s.parent IS NULL THEN 5 WHEN {continues} THEN p.c7 ELSE 3 END,p.parent,s.work,p.c2,p.c3,p.c4,s.truth,COALESCE(p.c5,0) OR s.truth=2,p.c6{carry_exports}
FROM s LEFT JOIN relation_continuation p ON p.mode IS CASE WHEN s.parent IS NOT NULL THEN 2 END AND p.h IS s.parent"#,
            seed(4)
        );
        let mut arms = vec![format!(
            "SELECT d.node AS node,({depth}) AS level,0 AS phase,NULL AS parent,d.work AS work,d.work AS start,0 AS fill,NULL AS token,0 AS truth,0 AS uncertain,0 AS cursor{seed_exports}"
        )];
        for (phase, query) in [(0, enter), (3, close_step), (4, return_step)]
            .into_iter()
            .chain(base_steps)
            .chain(child_steps)
        {
            let columns = (0..width)
                .map(|i| format!("r.c{i}"))
                .collect::<Vec<_>>()
                .join(",");
            arms.push(format!(
                "SELECT {columns} FROM schedule {alias} {} WHERE {alias}.phase={phase}",
                bridge(&query, width)
            ));
        }
        let payload = exports.iter().map(|s| format!(",c{s}")).collect::<String>();
        let query = format!(
            "WITH RECURSIVE schedule AS ({}) SELECT truth,work{payload} FROM schedule WHERE phase=5",
            arms.join("\nUNION ALL\n")
        );
        let returned = exports
            .iter()
            .enumerate()
            .map(|(i, s)| format!(",r.c{} AS c{s}", i + 2))
            .collect::<String>();
        let name = self.relation(
            "memoized recursive utility schedule",
            format!(
                "SELECT d.k,r.c1 AS work,r.c0 AS truth{returned} FROM {} d {}",
                input.name,
                bridge(&query, 2 + exports.len())
            ),
            true,
            "one reached invocation; indexed children; postorder completed memo answers",
        );
        self.profile.last_mut().unwrap()["recursiveMemo"] = json!({"bases":profiles,"recursiveColumns":width,"continuationCells":8,"captureStackCopies":false,"childAccess":"indexed immediate-child seek","failurePublication":false,"bypass":bypass});
        Ok(Output { name })
    }
}
