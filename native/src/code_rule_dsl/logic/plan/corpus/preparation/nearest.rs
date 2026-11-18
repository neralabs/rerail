//! `nearest($n, $a, set)` read from the transducer's file-local FIX (O12, option a), on the version (decision 0281's
//! path, step 6a): each literal becomes the input atom `nearest_<set>($n, $a)`, which the run binds beside the base
//! relations (`facts::vtab::syntax::with_nearest`). What the pass finds is the relations the run must make
//! (`nearest_<set>`, and the tree axes' `rr_inside_<set>` and `rr_has_<set>`), each with its set's kind atoms. A pass
//! of preparation, before the program's run.

use crate::code_rule_dsl::logic::front::{AXIS, DESCENDANTS, NEAREST, is_axis};
use crate::code_rule_dsl::logic::plan::fields::{Fields, FieldsData};
use crate::code_rule_dsl::logic::program_image::Builder;
use crate::code_rule_dsl::logic::program_image::manager::{Cx, RuleOut, RulePass, Select, Step};
use crate::code_rule_dsl::logic::program_image::{Lit, RuleRef, T};
use std::collections::BTreeSet;

#[derive(Default)]
pub struct Nearest {
    /// The relations the run makes, with their sets' kind atoms: the axes first, in the inputs' order, then each
    /// `nearest_<set>` by its set's name.
    pub sets: Vec<(String, Vec<i64>)>,
    used: BTreeSet<String>,
}

impl RulePass for Nearest {
    const NAME: &'static str = "nearest sets made by the transducer";
    fn begin(&mut self, cx: &Cx<'_>) -> Result<Select, String> {
        let v = cx.v();
        let ids = v.rules().filter(|r| r.body().any(|l| matches!(l.view(), Lit::Nearest { .. }))).map(|r| r.id());
        let mut set = crate::code_rule_dsl::logic::program_image::manager::RuleSet::new();
        for id in ids {
            set.insert(id);
        }
        Ok(Select::Rules(set))
    }
    fn rule(&mut self, cx: &Cx<'_>, r: RuleRef<'_>, out: &mut RuleOut<'_, '_>) -> Result<Step, String> {
        let v = cx.v();
        // Each `nearest` literal's relation, named before the rule is written.
        let mut rels = Vec::new();
        for l in r.body() {
            if let Lit::Nearest { kindset, .. } = l.view() {
                let set = v.text(kindset);
                self.used.insert(set.to_owned());
                rels.push(out.builder().rel(&format!("{NEAREST}{set}")));
            }
        }
        let mut rels = rels.into_iter();
        let mut w = out.rule().head_from(v, r, r.head());
        for l in r.body() {
            match l.view() {
                Lit::Nearest { n, a, .. } => w = w.atom(rels.next().expect("named above"), &[T::Var(v.text(r.var(n))), T::Var(v.text(r.var(a)))]),
                _ => w = w.lit(v, r, l),
            }
        }
        w.finish()?;
        Ok(Step::Written)
    }
    fn finish(&mut self, cx: &Cx<'_>, b: &mut Builder<'_>) -> Result<(), String> {
        let v = cx.v();
        let d = cx.get::<FieldsData>()?;
        let f = Fields::over(v, &d);
        let kinds = |set: &str| -> Vec<i64> { f.kindset(set).into_iter().flatten().filter_map(|&k| v.text(k).parse::<i64>().ok()).collect() };
        let mut inputs: Vec<String> = v.inputs().map(|a| v.text(a).to_owned()).collect();
        for rel in inputs.iter().filter(|r| is_axis(r)) {
            let set = rel.strip_prefix(AXIS).or_else(|| rel.strip_prefix(DESCENDANTS)).expect("an axis");
            self.sets.push((rel.clone(), kinds(set)));
        }
        let before = inputs.len();
        for set in std::mem::take(&mut self.used) {
            let rel = format!("{NEAREST}{set}");
            b.declare(&rel, &[("n", "node"), ("a", "node")]);
            if !inputs.contains(&rel) {
                inputs.push(rel.clone());
            }
            self.sets.push((rel, kinds(&set)));
        }
        if inputs.len() != before {
            b.set_inputs(&inputs);
        }
        Ok(())
    }
}
