//! End-only fields: inclusive field subtree, or all ancestor edges with a gated
//! body. Geometry never invokes native composition; child programs own outcomes.
use super::{Demand, Output, Plan, Sql, fields, ordered, value};
use serde_json::json;

impl Sql {
    pub(super) fn field_end(
        &mut self,
        inside: bool,
        field: u16,
        child: &Plan,
        input: &Demand,
    ) -> Result<Output, String> {
        if child.effects.walk_depth + 1 > 4 {
            return Err("nested ordered traversal depth exceeds 4".into());
        }
        let selected = if inside {
            self.relation("field end ancestor seed", format!(
                "SELECT d.k,d.file,n.parent AS node,n.id AS prior,NULL AS bound,d.work FROM {} d CROSS JOIN node n ON n.file=d.file AND n.id=d.node", input.name
            ), true, "caller,file,ancestor,previous ascent node,bound,work")
        } else {
            // A parent-constrained node cursor walks siblings. LIMIT 1 must end
            // that cursor at the first field child, rather than inspect every
            // later sibling merely to finish a LEFT JOIN.
            let first = self.relation("first field end child lookup", format!(
                "SELECT d.k,d.file,(SELECT n.id FROM node n WHERE n.file=d.file AND n.parent=d.node AND n.field={field} AND n.ffirst=1 LIMIT 1) AS selected,d.work FROM {} d", input.name
            ), true, "caller,file,optional first field child,work");
            self.relation("field end inclusive subtree seed", format!(
                "SELECT d.k,d.file,n.id AS node,NULL AS prior,n.\"last\" AS bound,d.work FROM {first} d LEFT JOIN node n ON n.file=d.file AND n.id=d.selected"
            ), true, "caller,file,first field child,prior,inclusive subtree bound,work")
        };
        let filter = ordered::candidate_filter(child, "n");
        let eligible = if inside {
            format!("edge.field={field} AND edge.ffirst=1")
        } else {
            "1".into()
        };
        let (body, profile) = ordered::nested::program(
            child,
            input,
            false,
            &format!("({eligible}) AND ({filter})"),
            &child.writes,
            ("w.file", "n.id", "w.work"),
            self.generation.clone(),
        )?;
        let walk = format!("outcome{}", self.definitions.len());
        let seed_values = child
            .writes
            .iter()
            .map(|slot| format!(",NULL AS v{slot}"))
            .collect::<String>();
        let write_values = child
            .writes
            .iter()
            .enumerate()
            .map(|(i, _)| format!(",r.c{}", i + 2))
            .collect::<String>();
        let null_values = child.writes.iter().map(|_| ",NULL").collect::<String>();
        let keep_values = child
            .writes
            .iter()
            .map(|slot| format!(",w.v{slot}"))
            .collect::<String>();
        let next = if inside {
            "n.parent"
        } else {
            "CASE WHEN n.id<w.bound THEN n.id+1 END"
        };
        let prior = if inside { "w.node" } else { "NULL" };
        let edge_join = if inside {
            "CROSS JOIN node edge ON edge.file=w.file AND edge.id=w.prior"
        } else {
            ""
        };
        let decisive = if child.writes.is_empty() {
            "w.own<0 OR w.own=1"
        } else {
            "w.own<0 OR w.own=1 OR w.own=2"
        };
        let bridge = ordered::nested::bridge(&body, child.writes.len() + 2);
        self.relation("ordered field end walk", format!(
            "SELECT s.k,s.file,s.node,s.prior,s.bound,0 AS phase,0 AS own,0 AS open,s.work{seed_values} FROM {selected} s WHERE s.node IS NOT NULL \
             UNION ALL SELECT w.k,w.file,w.node,w.prior,w.bound,1,COALESCE(r.c0,0),w.open,COALESCE(r.c1,w.work){write_values} \
             FROM {walk} w CROSS JOIN node n ON n.file=w.file AND n.id=w.node {edge_join} CROSS JOIN {} d ON d.k=w.k {bridge} WHERE w.phase=0 \
             UNION ALL SELECT w.k,w.file,{next},{prior},w.bound,0,0,(w.open OR w.own=2),w.work{null_values} \
             FROM {walk} w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=1 AND NOT ({decisive}) AND ({next}) IS NOT NULL",input.name
        ), true, "caller,file,candidate,prior,bound,phase,truth,open,work,typed exports");
        self.profile.last_mut().unwrap()["childPrograms"] = json!({"fieldEnd":profile});
        self.profile.last_mut().unwrap()["geometry"] = json!(if inside {
            "all ancestor edges; body eligible on first requested field child"
        } else {
            "inclusive first field child subtree in preorder"
        });
        let terminal = self.relation("completed field end walk", format!(
            "SELECT w.k,w.work,CASE WHEN w.own<0 THEN w.own WHEN w.own=1 THEN 1 WHEN w.open OR w.own=2 THEN 2 ELSE 0 END AS truth{keep_values} \
             FROM {walk} w CROSS JOIN node n ON n.file=w.file AND n.id=w.node WHERE w.phase=1 AND (({decisive}) OR ({next}) IS NULL)"
        ), true, "caller,remaining work,truth,typed exports");
        let columns = fields(&child.writes, |slot| {
            format!(
                "CASE WHEN t.truth=1 THEN t.v{slot} ELSE {} END",
                value(input, "d", slot)
            )
        });
        Ok(Output { name:self.relation("field end outcome",format!(
            "SELECT d.k,COALESCE(t.work,d.work) AS work,COALESCE(t.truth,0) AS truth{columns} FROM {} d LEFT JOIN {terminal} t ON t.k=d.k",input.name
        ),true,"caller,remaining work,truth,committed child exports") })
    }
}
