    use super::super::super::program_image::{Builder, T, Version};
    use super::super::super::rr::parse_version;
    use super::{Freeze as _, Placed};

    /// 0276 §3: a writer frozen in an order of its own reads each rule where the pass put it, the edit folded or not.
    /// A fold numbers the rules afresh (`Builder::finish_placing`), so the order the pass wrote is read back through it.
    #[test]
    fn a_frozen_edit_keeps_its_order_through_a_fold() {
        let rels = ["a", "b", "c", "f", "g", "h", "i", "j"];
        let text: String = rels.iter().map(|r| format!(".decl {r}(x: number)\n{r}(x) :- e(x).\n")).collect();
        let v = parse_version(&format!(".decl e(x: number)\n.input e\n.decl d(x: number)\n{text}")).unwrap();
        let shown = |p: &Placed| (0..p.len()).map(|k| p.v.show_rule(p.rule(k))).collect::<Vec<_>>();
        let ids: Vec<_> = v.rules().map(|r| r.id()).collect();
        // One rule removed and two added: an overlay of the eight. Four removed: half the program or more, folded.
        for (remove, folds) in [(1, false), (4, true)] {
            let mut b = Builder::on(&v);
            for &id in &ids[..remove] {
                b.remove(id);
            }
            let x = b.rule().head("d", &[T::Var("x")]).atom("a", &[T::Var("x")]).finish().unwrap();
            let y = b.rule().head("d", &[T::Var("x")]).atom("b", &[T::Var("x")]).finish().unwrap();
            let p = b.freeze_placed(vec![y, ids[7], x], true).unwrap();
            assert_eq!(!matches!(&*p.v, Version::Overlay { .. }), folds, "{remove} removed: folded");
            assert_eq!(shown(&p), ["d($x) :- b($x)", "j($x) :- e($x)", "d($x) :- a($x)"], "{remove} removed");
        }
    }
