//! Selection fusion for three-valued conjunctions.

use super::{Emitter, Live, Op, Reads, list};

impl Emitter<'_> {
    /// Compose conjunction metadata in the selection itself. A nonbinding fact needs its
    /// batched leaf answers, but no full-width temporary verdict frame before the merge.
    pub(super) fn fused_filter(
        &mut self,
        op: &Op,
        input: &str,
        first: bool,
        live: &Live,
    ) -> Option<String> {
        if let Some(region) = self.predicate_region(op, input, first, live) {
            return Some(region);
        }
        if let Op::Fact {
            term,
            reads,
            at_node,
            binds,
        } = op
            && binds.is_empty()
        {
            let node = if *at_node { "f.node" } else { "0" };
            let (with, join) = if reads.is_empty() {
                let key = format!("f.file*4294967296+{node}");
                (
                    format!(
                        "WITH t AS MATERIALIZED (SELECT ek,unk FROM fact WHERE term = {term} AND nk IN (SELECT {key} FROM {input} f)) "
                    ),
                    format!("JOIN t ON t.ek={key}"),
                )
            } else {
                let mut key = format!("f.file || ',' || {node}");
                for &slot in reads {
                    key.push_str(&format!(" || ',' || {}", self.key_value("f", slot)));
                }
                (
                    format!(
                        "WITH e AS MATERIALIZED (SELECT f.k AS k,{key} AS ek FROM {input} f), t AS MATERIALIZED (SELECT ek,unk FROM fact WHERE term = {term} AND ek IN (SELECT ek FROM e)) "
                    ),
                    "JOIN e ON e.k=f.k JOIN t ON t.ek=e.ek".into(),
                )
            };
            return Some(self.within(live, |this| {
                let mut selected = this.overridden("f", &[]);
                selected.push_str(if first {
                    ", t.unk"
                } else {
                    ", (f.unk | t.unk)"
                });
                for word in 0..this.masks {
                    selected.push_str(&if first {
                        ", 0".into()
                    } else {
                        format!(", f.wh{word}")
                    });
                }
                this.frame_with(
                    "v",
                    &with,
                    &format!("SELECT {selected} FROM {input} f {join}"),
                    Reads::join(&[input]),
                )
            }));
        }
        let (with, joined, condition) = match op {
            Op::Kinds(kinds) if kinds.is_empty() => (String::new(), "", "0".into()),
            Op::Kinds(kinds) => (
                String::new(),
                " JOIN node n ON n.file = f.file AND n.id = f.node",
                format!("n.kind IN ({})", list(kinds)),
            ),
            Op::Regex(term) => (
                format!(
                    "WITH t AS MATERIALIZED (SELECT ek FROM text_match \
                 WHERE term = {term} AND ek IN (SELECT file * 4294967296 + node FROM {input})) "
                ),
                " CROSS JOIN t ON t.ek = f.file * 4294967296 + f.node",
                "1".into(),
            ),
            Op::Never => (String::new(), "", "0".into()),
            _ => return None,
        };
        Some(self.within(live, |this| {
            let selected = if first {
                this.cols("f")
            } else {
                let mut selected = this.overridden("f", &[]);
                selected.push_str(", f.unk");
                for word in 0..this.masks {
                    selected.push_str(&format!(", f.wh{word}"));
                }
                selected
            };
            this.frame(
                "v",
                &format!("{with}SELECT {selected} FROM {input} f{joined} WHERE {condition}"),
                Reads::join(&[input]),
            )
        }))
    }
}
