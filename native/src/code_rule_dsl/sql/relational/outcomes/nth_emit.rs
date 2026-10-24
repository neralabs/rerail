//! Only reached classifications spend work; completed typed lists are cached by parent.
use super::{Demand, Output, Plan, Slots, Sql, ordered};
use serde_json::json;

pub(super) fn position(step: i32, offset: i32, index: &str) -> String {
    if step == 0 {
        format!("({index})={offset}")
    } else {
        format!("((({index})-({offset}))/({step}))>=0 AND ((({index})-({offset}))%({step}))=0")
    }
}
pub(super) fn plain(step: i32, offset: i32, reverse: bool, alias: &str) -> String {
    let index = if reverse {
        format!("{alias}.named_n-{alias}.named_ix+1")
    } else {
        format!("{alias}.named_ix")
    };
    format!(
        "({alias}.named_ix IS NOT NULL AND ({}))",
        position(step, offset, &index)
    )
}
impl Sql {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn nth_child(
        &mut self,
        step: i32,
        offset: i32,
        reverse: bool,
        rule: &Plan,
        term: usize,
        memoize: bool,
        input: &Demand,
    ) -> Result<Output, String> {
        let width = 2;
        let mut generation = self.generation.clone();
        generation.return_cells = generation
            .return_cells
            .checked_add(width)
            .filter(|cells| *cells <= 128)
            .ok_or("sibling index return payload exceeds 128 active cells")?;
        generation
            .return_peak
            .set(generation.return_peak.get().max(generation.return_cells));
        let mut nested = Sql {
            definitions: vec![],
            profile: vec![],
            correlated: true,
            generation,
        };
        let reached = Demand {
            name: nested.relation(
                "single reached sibling index",
                "SELECT 1 AS k,d.file,d.node,d.work".into(),
                true,
                "one capture-free reached invocation",
            ),
            slots: Slots::new(),
        };
        let answer =
            nested.nth_child_single(step, offset, reverse, rule, term, memoize, &reached)?;
        let query = format!(
            "WITH RECURSIVE {} SELECT o.truth,o.work FROM {} o",
            nested.definitions.join(",\n"),
            answer.name
        );
        let name = self.relation(
            "serialized sibling index transaction",
            format!(
                "SELECT d.k,r.c0 AS truth,r.c1 AS work FROM {} d {}",
                input.name,
                ordered::nested::bridge(&query, width)
            ),
            true,
            "one complete lookup,classify,fill,store transaction per demand",
        );
        self.profile.last_mut().unwrap()["childPrograms"] = json!({"siblingIndex":{"relations":nested.profile,"singleReachedDemand":true,"typedReturnColumns":width}});
        Ok(Output { name })
    }

    #[allow(clippy::too_many_arguments)]
    fn nth_child_single(
        &mut self,
        step: i32,
        offset: i32,
        reverse: bool,
        rule: &Plan,
        term: usize,
        memoize: bool,
        input: &Demand,
    ) -> Result<Output, String> {
        self.generation.tree(rule)?;
        let predicate = ordered::boolean(rule, "s")?
            .ok_or("sibling index: total metadata predicate required")?;
        let clause = self.generation.components.borrow().clause;
        let reverse = usize::from(reverse);
        let lookup_mode = if memoize { "0" } else { "-1" };
        let looked=self.relation("reached sibling classification lookup",format!("SELECT d.*,n.parent AS sibling_parent,n.named_n AS sibling_count,c.found AS sibling_hit,c.position AS sibling_position FROM {} d CROSS JOIN node n ON n.file=d.file AND n.id=d.node CROSS JOIN relation_sibling_state({lookup_mode},{clause},{term},d.file,COALESCE(n.parent,0),d.node,{reverse},'') c",input.name),true,"caller context,parent,named sibling count,completed-list lookup");
        let reached = "d.sibling_parent IS NOT NULL AND NOT d.sibling_hit";
        let classified = format!(
            "(WITH RECURSIVE sibling_prefix(id,next,named_count) AS (SELECT s.id,s.nsib,s.named FROM node s WHERE s.file=d.file AND s.id=(SELECT first.id FROM node first WHERE first.file=d.file AND first.parent=d.sibling_parent ORDER BY first.id LIMIT 1) AND d.work>0 UNION ALL SELECT s.id,s.nsib,p.named_count+s.named FROM sibling_prefix p CROSS JOIN node s ON s.file=d.file AND s.id=p.next WHERE p.named_count<d.work) SELECT json_group_array(id) FROM (SELECT s.id FROM sibling_prefix p CROSS JOIN node s ON s.file=d.file AND s.id=p.id WHERE CASE WHEN s.named THEN ({predicate}) ELSE 0 END ORDER BY s.id))"
        );
        let store_mode = if memoize {
            "CASE WHEN d.work>=d.sibling_count THEN 1 ELSE 2 END"
        } else {
            "2"
        };
        let fill = if memoize {
            "CASE WHEN d.sibling_parent IS NOT NULL AND NOT d.sibling_hit THEN relation_sibling_fill(MIN(d.work,d.sibling_count)) ELSE 1 END"
        } else {
            "1"
        };
        let tested=self.relation("reached sibling classification and cache fill",format!("SELECT d.*,c.position AS computed_position,{fill} AS fill_ready FROM {looked} d CROSS JOIN relation_sibling_state(CASE WHEN {reached} THEN {store_mode} ELSE -1 END,{clause},{term},d.file,COALESCE(d.sibling_parent,0),d.node,{reverse},CASE WHEN {reached} THEN {classified} ELSE '' END) c"),true,"caller context,complete classification rank,measured cache fill");
        let index = "CASE WHEN d.sibling_hit THEN d.sibling_position ELSE d.computed_position END";
        let matches = position(step, offset, index);
        Ok(Output{name:self.relation("sibling index outcome",format!("SELECT d.k,CASE WHEN d.sibling_parent IS NULL OR d.sibling_hit THEN d.work ELSE MAX(0,d.work-d.sibling_count) END AS work,CASE WHEN d.sibling_parent IS NULL THEN 0 WHEN NOT d.sibling_hit AND d.work<d.sibling_count THEN -2 WHEN ({index}) IS NOT NULL THEN CASE WHEN ({predicate}) AND ({matches}) THEN 1 ELSE 0 END ELSE 0 END AS truth FROM {tested} d CROSS JOIN node s ON s.file=d.file AND s.id=d.node WHERE d.fill_ready=1"),true,"caller key,remaining work,native An+B truth or work limit")})
    }
}
