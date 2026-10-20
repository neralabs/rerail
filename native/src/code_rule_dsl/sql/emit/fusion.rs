//! Benchmark-only selection fusion for three-valued conjunctions.

use super::{Emitter, Live, Op, Reads, list};

impl Emitter<'_> {
    /// Deterministic predicates do not alter captures or produce unknown. Their selection can
    /// carry the conjunction's accumulated verdict and mask without an intermediate frame.
    pub(super) fn fused_filter(
        &mut self,
        op: &Op,
        input: &str,
        first: bool,
        live: &Live,
    ) -> Option<String> {
        let (joined, condition) = match op {
            Op::Kinds(kinds) if kinds.is_empty() => ("", "0".into()),
            Op::Kinds(kinds) => (
                " JOIN node n ON n.file = f.file AND n.id = f.node",
                format!("n.kind IN ({})", list(kinds)),
            ),
            Op::Regex(term) => (
                "",
                format!(
                    "(f.file * 4294967296 + f.node) IN (SELECT ek FROM text_match \
                 WHERE term = {term} AND ek IN (SELECT file * 4294967296 + node FROM {input}))"
                ),
            ),
            Op::Never => ("", "0".into()),
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
                &format!("SELECT {selected} FROM {input} f{joined} WHERE {condition}"),
                Reads::join(&[input]),
            )
        }))
    }
}
